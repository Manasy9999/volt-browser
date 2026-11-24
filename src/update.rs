// Mise à jour intégrée : dernière version publiée sur GitHub, volt.exe téléchargé, vérifié, puis échangé avec l'exe
// en cours (Windows permet de renommer un exe qui tourne). Tout ce qui vient du réseau est hostile : rien ne sert à
// construire un chemin, les noms de fichiers locaux sont fixes.
use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

use serde_json::Value;

use crate::lang::{t, te};
use windows::{
    Win32::{Security::Cryptography::{BCRYPT_SHA256_ALG_HANDLE, BCryptHash}, System::Com::Urlmon::URLDownloadToFileW},
    core::HSTRING,
};

pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
// Constantes du binaire de production : aucun réglage ni message de l'interface ne les change.
const API: &str = "https://api.github.com/repos/Manasy9999/volt-browser/releases/latest";
const PREFIX: &str = "https://github.com/Manasy9999/volt-browser/releases/download/";
const ASSET: &str = "volt.exe";
const NEW: &str = "volt.update.tmp"; // téléchargé à côté de l'exe : le renommage reste sur le même disque
pub const OLD: &str = "volt.old.exe";
const MAX: u64 = 100 << 20;

static BUSY: AtomicBool = AtomicBool::new(false);

/// (adresse interrogée, préfixe de téléchargement). Substitution pour les essais en debug seulement : absente du release.
fn urls() -> (String, String) {
    #[cfg(debug_assertions)]
    if let Ok(b) = std::env::var("VOLT_UPDATE_TEST") {
        return (format!("{b}latest"), format!("{b}download/"));
    }
    (API.into(), PREFIX.into())
}

/// « v0.10.2 » ou « 0.10.2 » -> [0, 10, 2] ; tout le reste (pré-version, lettres, 4 nombres) est refusé.
pub fn parse_ver(s: &str) -> Option<[u64; 3]> {
    let p: Vec<&str> = s.strip_prefix(['v', 'V']).unwrap_or(s).split('.').collect();
    if p.len() != 3 || p.iter().any(|x| x.is_empty() || x.len() > 9 || !x.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    Some([p[0].parse().ok()?, p[1].parse().ok()?, p[2].parse().ok()?])
}

/// Strictement plus récente que la version installée (nombres comparés un à un : 0.10.0 > 0.9.0).
pub fn newer(tag: &str, cur: &str) -> Option<bool> {
    Some(parse_ver(tag)? > parse_ver(cur)?)
}

/// Adresse de téléchargement : le préfixe autorisé puis un chemin simple (pas de « .. », d'échappement ni de requête).
pub fn valid_url(u: &str, prefix: &str) -> bool {
    u.strip_prefix(prefix).is_some_and(|r| !r.is_empty() && !r.contains("..") && r.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b)))
}

#[derive(Debug, PartialEq)]
pub struct Asset {
    pub url: String,
    pub size: u64,
    pub sha: [u8; 32],
}

/// Étiquette de la dernière version, telle que publiée (vérifiée par `parse_ver` avant tout usage).
pub fn tag(json: &Value) -> Result<&str, String> {
    json["tag_name"].as_str().ok_or_else(|| t("réponse de GitHub sans numéro de version"))
}

/// Fichier joint `volt.exe` : adresse sous le préfixe, taille et empreinte SHA-256 publiées (champ `digest`).
pub fn asset(json: &Value, prefix: &str) -> Result<Asset, String> {
    let a = json["assets"].as_array().and_then(|l| l.iter().find(|a| a["name"] == ASSET)).ok_or_else(|| t("aucun fichier volt.exe dans la dernière version"))?;
    let url = a["browser_download_url"].as_str().unwrap_or_default();
    if !valid_url(url, prefix) {
        return Err(t("adresse de téléchargement hors de la page du projet, mise à jour refusée"));
    }
    let size = a["size"].as_u64().filter(|&n| n > 0 && n <= MAX).ok_or_else(|| t("taille du fichier absente ou invalide"))?;
    let hex = a["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).filter(|h| h.len() == 64).ok_or_else(|| t("empreinte SHA-256 absente, mise à jour refusée"))?;
    let mut sha = [0u8; 32];
    for (i, b) in sha.iter_mut().enumerate() {
        *b = u8::from_str_radix(hex.get(2 * i..2 * i + 2).ok_or_else(|| t("empreinte SHA-256 illisible"))?, 16).map_err(|_| t("empreinte SHA-256 illisible"))?;
    }
    Ok(Asset { url: url.into(), size, sha })
}

/// En-tête d'exécutable Windows : « MZ » puis la signature « PE\0\0 » à l'adresse qu'il indique.
pub fn is_exe(b: &[u8]) -> bool {
    let pe = b.get(0x3c..0x40).map(|x| u32::from_le_bytes(x.try_into().unwrap()) as usize);
    b.starts_with(b"MZ") && pe.and_then(|o| b.get(o..o.checked_add(4)?)) == Some(b"PE\0\0")
}

/// SHA-256 par le fournisseur de chiffrement de Windows (pas de crate en plus).
pub fn sha256(b: &[u8]) -> Result<[u8; 32], String> {
    let mut out = [0u8; 32];
    unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, b, &mut out) }.ok().map_err(|e| e.message())?;
    Ok(out)
}

/// Vérifie le fichier téléchargé : taille annoncée, en-tête d'exe, empreinte publiée.
pub fn verify(b: &[u8], a: &Asset) -> Result<(), String> {
    if b.len() as u64 != a.size {
        return Err(t("taille du fichier différente de celle annoncée, fichier supprimé"));
    }
    if !is_exe(b) {
        return Err(t("le fichier téléchargé n'est pas un exécutable Windows, fichier supprimé"));
    }
    if sha256(b)? != a.sha {
        return Err(t("empreinte SHA-256 différente de celle publiée, fichier supprimé"));
    }
    Ok(())
}

pub fn download(url: &str, to: &Path) -> Result<(), String> {
    unsafe { URLDownloadToFileW(None, &HSTRING::from(url), &HSTRING::from(to), 0, None) }.map_err(|e| {
        let m = e.message();
        let m = m.trim().trim_end_matches('.');
        if m.is_empty() { format!("code {:#x}", e.code().0) } else { m.into() }
    })
}

/// Dernière version publiée : (réponse, version normalisée) si elle est strictement plus récente, None si Volt est à jour.
fn latest() -> Result<Option<(Value, String)>, String> {
    let f = std::env::temp_dir().join("volt-update.json");
    let r = download(&urls().0, &f).map_err(|e| te("GitHub injoignable, vérifiez la connexion Internet ({e})", e));
    let b = fs::read(&f);
    let _ = fs::remove_file(&f);
    r?;
    let json: Value = b.ok().and_then(|b| serde_json::from_slice(&b).ok()).ok_or_else(|| t("réponse de GitHub illisible"))?;
    let t = tag(&json)?;
    let v = parse_ver(t).ok_or_else(|| crate::lang::t("numéro de version publié illisible"))?;
    Ok(newer(t, CURRENT).unwrap_or(false).then(|| (json, format!("{}.{}.{}", v[0], v[1], v[2]))))
}

/// Une seule recherche ou installation à la fois ; false : déjà en cours.
pub fn begin() -> bool {
    !BUSY.swap(true, Ordering::SeqCst)
}

pub fn end() {
    BUSY.store(false, Ordering::SeqCst);
}

/// Bloquant : à lancer hors du fil de l'interface. Some(version) : plus récente disponible.
pub fn check() -> Result<Option<String>, String> {
    Ok(latest()?.map(|l| l.1))
}

/// Bloquant. Télécharge, vérifie et met le nouvel exe à la place de l'actuel ; renvoie la version installée.
/// Tout échec laisse l'ancien exe en place et supprime le fichier téléchargé.
pub fn install() -> Result<String, String> {
    let (json, v) = latest()?.ok_or_else(|| t("Volt est déjà à jour"))?;
    let a = asset(&json, &urls().1)?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or_else(|| t("dossier de Volt introuvable"))?;
    let (new, old) = (dir.join(NEW), dir.join(OLD));
    if old.exists() && fs::remove_file(&old).is_err() {
        return Err(te("{e} encore utilisé : redémarrez Volt puis réessayez", OLD));
    }
    let _ = fs::remove_file(&new);
    let r = (|| {
        download(&a.url, &new).map_err(|e| te("téléchargement impossible ({e})", e))?;
        verify(&fs::read(&new).map_err(|e| te("fichier téléchargé illisible ({e})", e))?, &a)?;
        fs::rename(&exe, &old).map_err(|e| te("remplacement impossible, dossier protégé ou fichier verrouillé ({e})", e))?;
        if let Err(e) = fs::rename(&new, &exe) {
            let _ = fs::rename(&old, &exe); // jamais sans exe : l'ancien revient
            return Err(te("remplacement impossible ({e})", e));
        }
        Ok(())
    })();
    let _ = fs::remove_file(&new);
    r.map(|_| v)
}

/// Au démarrage : supprime l'ancien exe laissé par une mise à jour (il peut encore se fermer, on réessaie 10 s).
pub fn cleanup() {
    let Some(old) = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.join(OLD))).filter(|o| o.exists()) else { return };
    std::thread::spawn(move || {
        for _ in 0..50 {
            if fs::remove_file(&old).is_ok() || !old.exists() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versions() {
        assert_eq!(newer("v0.1.0", "0.1.0"), Some(false)); // égales
        assert_eq!(newer("v0.1.1", "0.1.0"), Some(true));
        assert_eq!(newer("0.2.0", "0.1.9"), Some(true)); // sans « v »
        assert_eq!(newer("v0.0.9", "0.1.0"), Some(false)); // plus ancienne
        assert_eq!(newer("v0.10.0", "0.9.0"), Some(true)); // nombres, pas chaînes
        assert_eq!(newer("v0.9.0", "0.10.0"), Some(false));
        assert_eq!(newer("V1.0.0", "0.99.99"), Some(true));
        for bad in ["", "v", "1.2", "1.2.3.4", "v1.2.x", "1.2.3-beta", " 1.2.3", "1..3", "v+1.2.3", "1.2.9999999999"] {
            assert_eq!(parse_ver(bad), None, "{bad}");
        }
        assert!(parse_ver(CURRENT).is_some());
    }

    #[test]
    fn urls_allowed() {
        let p = PREFIX;
        assert!(valid_url(&format!("{p}v0.1.1/volt.exe"), p));
        for bad in [
            "https://github.com/Autre/volt-browser/releases/download/v0.1.1/volt.exe",
            "http://github.com/Manasy9999/volt-browser/releases/download/v0.1.1/volt.exe",
            "https://github.com.evil.fr/Manasy9999/volt-browser/releases/download/v0.1.1/volt.exe",
            &format!("{p}../../../Autre/x/releases/download/v1/volt.exe"),
            &format!("{p}v0.1.1/%2e%2e/volt.exe"),
            &format!("{p}v0.1.1/volt.exe?x=1"),
            &format!("{p}v0.1.1\\volt.exe"),
            p,
            "",
        ] {
            assert!(!valid_url(bad, p), "{bad}");
        }
        // le préfixe de production est une constante (la substitution n'existe qu'en debug, par variable d'environnement)
        assert!(API.starts_with("https://api.github.com/repos/Manasy9999/volt-browser/") && PREFIX.starts_with("https://github.com/Manasy9999/volt-browser/"));
    }

    fn rel(assets: Value) -> Value {
        json!({"tag_name": "v0.1.1", "assets": assets})
    }

    #[test]
    fn response() {
        let sha = "ab".repeat(32);
        let ok = json!({"name": "volt.exe", "size": 1000, "digest": format!("sha256:{sha}"), "browser_download_url": format!("{PREFIX}v0.1.1/volt.exe")});
        let other = json!({"name": "volt.zip", "size": 5, "digest": format!("sha256:{sha}"), "browser_download_url": format!("{PREFIX}v0.1.1/volt.zip")});
        assert_eq!(tag(&rel(json!([]))), Ok("v0.1.1"));
        assert!(tag(&json!({"message": "Not Found"})).is_err());
        assert_eq!(asset(&rel(json!([other, ok])), PREFIX), Ok(Asset { url: format!("{PREFIX}v0.1.1/volt.exe"), size: 1000, sha: [0xab; 32] }));
        assert!(asset(&rel(json!([other])), PREFIX).unwrap_err().contains("aucun fichier"));
        assert!(asset(&json!({"tag_name": "v0.1.1"}), PREFIX).is_err());
        let with = |k: &str, v: Value| {
            let mut a = ok.clone();
            a[k] = v;
            asset(&rel(json!([a])), PREFIX)
        };
        assert!(with("digest", Value::Null).unwrap_err().contains("empreinte"));
        assert!(with("digest", json!(format!("sha1:{sha}"))).unwrap_err().contains("empreinte"));
        assert!(with("digest", json!(format!("sha256:{}", "zz".repeat(32)))).is_err());
        assert!(with("digest", json!("sha256:abcd")).is_err());
        assert!(with("browser_download_url", json!("https://evil.fr/volt.exe")).unwrap_err().contains("adresse"));
        assert!(with("size", json!(0)).is_err());
        assert!(with("size", json!(-1)).is_err());
        assert!(with("size", Value::Null).is_err());
    }

    #[test]
    fn checks_file() {
        // SHA-256 de « abc » (FIPS 180-2)
        let abc = sha256(b"abc").unwrap();
        assert_eq!(abc[..4], [0xba, 0x78, 0x16, 0xbf]);
        assert_eq!(abc[28..], [0xf2, 0x00, 0x15, 0xad]);
        let mut exe = vec![0u8; 0x100];
        exe[..2].copy_from_slice(b"MZ");
        exe[0x3c] = 0x80;
        exe[0x80..0x84].copy_from_slice(b"PE\0\0");
        assert!(is_exe(&exe) && !is_exe(b"MZ") && !is_exe(b"<html>"));
        let a = Asset { url: String::new(), size: exe.len() as u64, sha: sha256(&exe).unwrap() };
        assert!(verify(&exe, &a).is_ok());
        assert!(verify(&exe[..0xff], &a).unwrap_err().contains("taille"));
        assert!(verify(&[b'x'; 0x100], &a).unwrap_err().contains("exécutable"));
        assert!(verify(&exe, &Asset { sha: [0; 32], ..a }).unwrap_err().contains("empreinte"));
        // l'exe de test lui-même passe l'en-tête
        assert!(is_exe(&fs::read(std::env::current_exe().unwrap()).unwrap()));
    }
}
