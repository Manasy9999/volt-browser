// « Remplir et signer » un PDF : page autonome (src/sign/index.html, embarquée dans l'exe) installée dans le profil au
// premier usage, à côté de ses trois bibliothèques, téléchargées une seule fois et vérifiées par leur empreinte
// (src/sign/libs.json) : elles pèsent 2 Mo, plus que Volt lui-même. Rien du document ne sort de la machine : la page n'a
// droit à aucune connexion (sa balise de sécurité), Volt lui remet le PDF et garde les signatures, chiffrées.
use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;

use crate::{
    defaults,
    lang::{t, te},
    update, vault,
};

const PAGE: &str = include_str!("sign/index.html");
const LIBS: &str = include_str!("sign/libs.json");
const MAX_PDF: usize = 80 << 20; // au-delà, le passage du document à la page serait trop lourd

fn dir(data: &Path) -> PathBuf {
    data.join("sign")
}

/// Adresse de la page dans ce profil, sans paramètres ; None tant qu'elle n'est pas installée.
pub fn page_url(data: &Path) -> Option<String> {
    defaults::file_url(&dir(data).join("index.html").to_string_lossy())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Le fichier est-il exactement celui attendu (taille et empreinte SHA-256) ?
fn intact(path: &Path, size: u64, sha: &str) -> bool {
    fs::read(path).is_ok_and(|b| b.len() as u64 == size && update::sha256(&b).is_ok_and(|h| hex(&h) == sha))
}

/// Bloquant (disque, et réseau la première fois) : à lancer hors du fil de l'interface. Installe la page et télécharge
/// les bibliothèques qui manquent ; un fichier qui n'a pas l'empreinte attendue n'est jamais mis en place.
pub fn install(data: &Path) -> Result<(), String> {
    let d = dir(data);
    fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let page = d.join("index.html");
    // réécrite si elle diffère : une mise à jour de Volt met la page à jour
    if fs::read_to_string(&page).ok().as_deref() != Some(PAGE) {
        fs::write(&page, PAGE).map_err(|e| e.to_string())?;
    }
    let libs: Value = serde_json::from_str(LIBS).map_err(|e| e.to_string())?;
    for f in libs["files"].as_array().into_iter().flatten() {
        let (name, size, sha) = (f["name"].as_str().unwrap_or_default(), f["size"].as_u64().unwrap_or(0), f["sha256"].as_str().unwrap_or_default());
        let (to, part) = (d.join(name), d.join(format!("{name}.part")));
        if name.is_empty() || name.contains(['/', '\\']) || intact(&to, size, sha) {
            continue;
        }
        // adresse principale, puis le miroir : même fichier, même empreinte
        let ok = [&f["url"], &f["mirror"]].iter().filter_map(|u| u.as_str()).any(|u| update::download(u, &part).is_ok() && intact(&part, size, sha));
        if !ok {
            let _ = fs::remove_file(&part);
            return Err(t("Outils de signature introuvables : vérifiez la connexion Internet (ils ne sont téléchargés qu'une fois)"));
        }
        let _ = fs::remove_file(&to);
        fs::rename(&part, &to).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Remet en clair les « %XX » d'un morceau d'adresse.
fn decode(s: &str) -> String {
    let (b, mut out, mut i) = (s.as_bytes(), vec![], 0);
    while i < b.len() {
        let h = (b[i] == b'%').then(|| s.get(i + 1..i + 3).and_then(|x| u8::from_str_radix(x, 16).ok())).flatten();
        out.push(h.unwrap_or(b[i]));
        i += if h.is_some() { 3 } else { 1 };
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Chemin Windows d'une adresse file:/// (l'inverse de `defaults::file_url`) ; None pour une autre adresse.
fn file_path(url: &str) -> Option<PathBuf> {
    let p = url.strip_prefix("file:///")?.split(['?', '#']).next()?;
    Some(PathBuf::from(decode(p).replace('/', "\\")))
}

/// Bloquant. Le PDF affiché à cette adresse : (nom du fichier, contenu). Un fichier du PC est lu sur le disque ; un PDF
/// du web est retéléchargé, sans les cookies du site (un PDF derrière une connexion doit d'abord être enregistré).
pub fn fetch(url: &str) -> Result<(String, Vec<u8>), String> {
    let (path, temp) = match file_path(url) {
        Some(p) => (p, false),
        None => {
            let tmp = std::env::temp_dir().join(format!("volt-sign-{}.pdf", std::process::id()));
            update::download(url, &tmp).map_err(|e| te("Téléchargement du PDF impossible ({e})", e))?;
            (tmp, true)
        }
    };
    let b = fs::read(&path).map_err(|e| te("Lecture du PDF impossible ({e})", e));
    if temp {
        let _ = fs::remove_file(&path);
    }
    let b = b?;
    if !b.starts_with(b"%PDF") {
        return Err(t("Ce document n'est pas un PDF lisible ici : enregistrez-le d'abord sur votre PC, puis ouvrez le fichier"));
    }
    if b.len() > MAX_PDF {
        return Err(t("Ce PDF est trop gros pour être signé ici (80 Mo au plus)"));
    }
    let last = url.split(['?', '#']).next().unwrap_or(url).rsplit('/').next().unwrap_or_default();
    let name = Some(decode(last)).filter(|n| !n.is_empty()).unwrap_or_else(|| "document.pdf".into());
    Ok((name, b))
}

/// Base64 du document, pour le remettre à la page.
pub fn b64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        let at = |s: u32| A[(n >> s) as usize & 63] as char;
        o.extend([at(18), at(12), if c.len() > 1 { at(6) } else { '=' }, if c.len() > 2 { at(0) } else { '=' }]);
    }
    o
}

/// Signatures gardées (tableau JSON d'images), chiffrées sur le disque par DPAPI comme le coffre ; "[]" s'il n'y en a pas.
pub fn sigs(data: &Path) -> String {
    fs::read(dir(data).join("sigs.dat"))
        .ok()
        .and_then(|e| vault::unprotect(&e))
        .and_then(|j| String::from_utf8(j).ok())
        .filter(|j| serde_json::from_str::<Vec<Value>>(j).is_ok())
        .unwrap_or_else(|| "[]".into())
}

/// Enregistre la liste envoyée par la page ; refuse ce qui n'est pas un tableau JSON de taille raisonnable.
pub fn save_sigs(data: &Path, json: &str) -> bool {
    json.len() <= 16 << 20 && serde_json::from_str::<Vec<Value>>(json).is_ok() && vault::protect(json.as_bytes()).is_some_and(|e| fs::write(dir(data).join("sigs.dat"), e).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64() {
        for (a, b) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(b64(a.as_bytes()), b, "{a}");
        }
        assert_eq!(b64(&[0xff, 0xfe, 0xfd]), "//79");
    }

    #[test]
    fn local_pdf_roundtrip() {
        // un fichier au nom difficile : son adresse file:/// revient au même chemin, et il est relu tel quel
        let f = std::env::temp_dir().join(format!("volt signé #{} 100%.pdf", std::process::id()));
        fs::write(&f, b"%PDF-1.4 essai").unwrap();
        let url = defaults::file_url(&f.to_string_lossy()).unwrap();
        assert_eq!(file_path(&url).unwrap(), f);
        let (name, bytes) = fetch(&url).unwrap();
        assert_eq!((name.as_str(), bytes.as_slice()), (f.file_name().unwrap().to_str().unwrap(), &b"%PDF-1.4 essai"[..]));
        fs::write(&f, b"<html>pas un pdf").unwrap();
        assert!(fetch(&url).is_err());
        fs::remove_file(&f).unwrap();
        assert!(fetch(&url).is_err() && file_path("https://a.fr/x.pdf").is_none());
        assert_eq!(decode("a%20b%C3%A9%2"), "a bé%2"); // « % » sans deux chiffres : laissé tel quel
    }

    #[test]
    fn signatures_and_page() {
        let d = std::env::temp_dir().join(format!("volt-sign-test-{}", std::process::id()));
        fs::create_dir_all(dir(&d)).unwrap();
        assert_eq!(sigs(&d), "[]");
        let list = r#"[{"src":"data:image/png;base64,AAAA","w":10,"h":5}]"#;
        assert!(save_sigs(&d, list) && sigs(&d) == list);
        // chiffré sur le disque : le contenu n'y apparaît pas en clair
        assert!(!String::from_utf8_lossy(&fs::read(dir(&d).join("sigs.dat")).unwrap()).contains("data:image"));
        assert!(!save_sigs(&d, "{\"pas\":\"un tableau\"}") && !save_sigs(&d, "n'importe quoi") && sigs(&d) == list);
        // manifeste embarqué : trois bibliothèques, empreintes de 64 caractères, adresses en https figées sur une version
        let libs: Value = serde_json::from_str(LIBS).unwrap();
        let files = libs["files"].as_array().unwrap();
        assert_eq!(files.len(), 3);
        for f in files {
            assert!(f["sha256"].as_str().unwrap().len() == 64 && f["size"].as_u64().unwrap() > 100_000);
            assert!(["url", "mirror"].iter().all(|k| f[k].as_str().unwrap().starts_with("https://") && f[k].as_str().unwrap().contains('@')));
            assert!(PAGE.contains(f["name"].as_str().unwrap()));
        }
        assert!(intact(&dir(&d).join("sigs.dat"), 0, "").eq(&false));
        fs::remove_dir_all(&d).unwrap();
    }

    // Télécharge vraiment les bibliothèques (2 Mo) : cargo test real_sign_install -- --ignored
    #[test]
    #[ignore]
    fn real_sign_install() {
        let d = std::env::temp_dir().join(format!("volt-sign-real-{}", std::process::id()));
        install(&d).unwrap();
        assert!(page_url(&d).is_some());
        for n in ["pdf.min.js", "pdf.worker.min.js", "pdf-lib.min.js"] {
            assert!(dir(&d).join(n).metadata().unwrap().len() > 100_000, "{n}");
        }
        install(&d).unwrap(); // seconde fois : rien à refaire
        fs::remove_dir_all(&d).unwrap();
    }
}
