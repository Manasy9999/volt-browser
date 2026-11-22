// Coffre de mots de passe de Volt. Chiffré au repos par DPAPI (CryptProtectData, lié à la session Windows) ; le JSON
// déchiffré ne vit qu'en mémoire. Les valeurs en clair ne quittent le Rust que pour remplir un champ (script injecté)
// ou pour l'export CSV (derrière Windows Hello). L'interface et les journaux ne reçoivent jamais que des nombres.
use std::path::Path;

use serde::{Deserialize, Serialize};
use windows::{
    Security::Credentials::UI::{UserConsentVerifier, UserConsentVerifierAvailability},
    Win32::{
        Foundation::{HLOCAL, HWND, LocalFree},
        Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData},
        System::Com::{COINIT_MULTITHREADED, CoInitializeEx},
        UI::Controls::Dialogs::{GetOpenFileNameW, GetSaveFileNameW, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW},
    },
    core::{PCWSTR, PWSTR},
};

/// Une entrée du coffre. `origin` = schéma + hôte (ex. "https://github.com"), clé exacte du remplissage.
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct Entry {
    pub origin: String,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub note: String,
}

/// Origine exacte d'une URL (schéma http(s) + autorité en minuscules), ou None (schéma interdit, pas d'hôte).
/// Jamais de sous-domaine replié : l'origine sert de clé stricte au remplissage.
pub fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.trim().split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?.rsplit('@').next()?; // sans chemin ni userinfo
    (!authority.is_empty()).then(|| format!("{scheme}://{}", authority.to_ascii_lowercase()))
}

/// Entrées d'une origine EXACTE (schéma + hôte identiques).
pub fn for_origin<'a>(entries: &'a [Entry], origin: &str) -> Vec<&'a Entry> {
    entries.iter().filter(|e| e.origin == origin).collect()
}

/// Ajoute les entrées absentes (clé : origine + identifiant) ; une ré-importation identique n'ajoute rien. Renvoie le nombre ajouté.
pub fn merge(into: &mut Vec<Entry>, new: Vec<Entry>) -> usize {
    let before = into.len();
    for e in new {
        if !into.iter().any(|x| x.origin == e.origin && x.username == e.username) {
            into.push(e);
        }
    }
    into.len() - before
}

/// Découpe un texte CSV (RFC 4180 : champs entre guillemets, virgules et sauts de ligne échappés, "" = guillemet) en lignes de champs.
fn records(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let (mut rows, mut row, mut field, mut quoted) = (vec![], vec![], String::new(), false);
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        if quoted {
            match c {
                '"' if it.peek() == Some(&'"') => {
                    field.push('"');
                    it.next();
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
        } else {
            match c {
                '"' => quoted = true,
                ',' => row.push(std::mem::take(&mut field)),
                '\r' => {}
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                _ => field.push(c),
            }
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows.retain(|r| r.iter().any(|f| !f.is_empty())); // lignes vides ignorées
    rows
}

/// Analyse un CSV de navigateur (en-têtes Chrome/Edge/Opera/Brave/Firefox, ordre et casse libres).
/// Ignore les lignes sans mot de passe ou sans origine http(s). Renvoie (entrées, lignes ignorées).
pub fn parse_csv(text: &str) -> (Vec<Entry>, usize) {
    let rows = records(text);
    let Some((head, body)) = rows.split_first() else { return (vec![], 0) };
    let col = |name: &str| head.iter().position(|h| h.trim().eq_ignore_ascii_case(name));
    let (Some(ui), Some(pi)) = (col("url"), col("password")) else { return (vec![], body.len()) };
    let (ni, noti) = (col("username"), col("note").or_else(|| col("notes")));
    fn field(r: &[String], i: Option<usize>) -> &str {
        i.and_then(|i| r.get(i)).map_or("", |s| s.as_str())
    }
    let (mut out, mut skipped) = (vec![], 0usize);
    for r in body {
        let pw = field(r, Some(pi));
        match origin_of(field(r, Some(ui))) {
            Some(origin) if !pw.is_empty() => out.push(Entry { origin, username: field(r, ni).into(), password: pw.into(), note: field(r, noti).into() }),
            _ => skipped += 1,
        }
    }
    (out, skipped)
}

fn csv_field(f: &str) -> String {
    if f.contains([',', '"', '\n', '\r']) { format!("\"{}\"", f.replace('"', "\"\"")) } else { f.into() }
}

/// Génère un CSV au format des navigateurs Chromium (name,url,username,password,note).
pub fn to_csv(entries: &[Entry]) -> String {
    let mut s = String::from("name,url,username,password,note\r\n");
    for e in entries {
        let host = e.origin.split_once("://").map_or(e.origin.as_str(), |x| x.1);
        let row: Vec<String> = [host, &e.origin, &e.username, &e.password, &e.note].iter().map(|f| csv_field(f)).collect();
        s.push_str(&row.join(","));
        s.push_str("\r\n");
    }
    s
}

// ----- DPAPI (isolé, non testable en pur) : chiffre/déchiffre lié à la session Windows, sans entropie ni UI. -----
fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
}

fn take_blob(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
    unsafe {
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
    }
    v
}

fn protect(plain: &[u8]) -> Option<Vec<u8>> {
    let (inb, mut out) = (blob(plain), CRYPT_INTEGER_BLOB::default());
    unsafe { CryptProtectData(&inb, PCWSTR::null(), None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }.ok()?;
    Some(take_blob(out))
}

fn unprotect(enc: &[u8]) -> Option<Vec<u8>> {
    let (inb, mut out) = (blob(enc), CRYPT_INTEGER_BLOB::default());
    unsafe { CryptUnprotectData(&inb, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }.ok()?;
    Some(take_blob(out))
}

/// Lit le coffre déchiffré (vide si absent). Fichier présent mais indéchiffrable : mis de côté en `.bak` au lieu
/// d'être écrasé au prochain enregistrement (comme settings.json) — pas de perte silencieuse.
pub fn load(path: &Path) -> Vec<Entry> {
    let Ok(enc) = std::fs::read(path) else { return vec![] };
    match unprotect(&enc).and_then(|j| serde_json::from_slice(&j).ok()) {
        Some(v) => v,
        None => {
            let _ = std::fs::rename(path, path.with_extension("dat.bak"));
            vec![]
        }
    }
}

/// Écrit le coffre chiffré par DPAPI ; false si le chiffrement ou l'écriture échoue.
pub fn save(path: &Path, entries: &[Entry]) -> bool {
    let Ok(json) = serde_json::to_vec(entries) else { return false };
    let Some(enc) = protect(&json) else { return false };
    if let Some(p) = path.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    std::fs::write(path, enc).is_ok()
}

// ----- Windows Hello (UserConsentVerifier, WinRT) : appelé hors du fil de l'interface (bloque jusqu'à la réponse). -----
fn mta() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

/// Hello disponible (PIN/biométrie configuré) ? Aucune UI.
pub fn hello_available() -> bool {
    mta();
    let Ok(op) = UserConsentVerifier::CheckAvailabilityAsync() else { return false };
    for _ in 0..3000 {
        match op.Status() {
            Ok(s) if s.0 != 0 => break, // 0 = Started
            Ok(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
            Err(_) => return false,
        }
    }
    op.GetResults().is_ok_and(|a| a == UserConsentVerifierAvailability::Available)
}

/// Demande la validation Hello avec un message ; true seulement sur « Vérifié ». Fail-closed : toute erreur bloque.
pub fn hello_verify(message: &str) -> bool {
    mta();
    let Ok(op) = UserConsentVerifier::RequestVerificationAsync(&windows::core::HSTRING::from(message)) else { return false };
    for _ in 0..6000 {
        match op.Status() {
            Ok(s) if s.0 != 0 => break,
            Ok(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
            Err(_) => return false,
        }
    }
    op.GetResults().is_ok_and(|r| r.0 == 0) // 0 = Verified
}

// ----- Sélecteur de fichier natif (import/export CSV) : l'utilisateur choisit où lire/écrire, jamais de dossier temporaire. -----
fn pick(hwnd: isize, save: bool) -> Option<std::path::PathBuf> {
    let mut buf = [0u16; 1024];
    let filter: Vec<u16> = "Fichiers CSV\0*.csv\0Tous les fichiers\0*.*\0\0".encode_utf16().collect();
    let csv: Vec<u16> = "csv\0".encode_utf16().collect();
    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: HWND(hwnd as _),
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(buf.as_mut_ptr()),
        nMaxFile: buf.len() as u32,
        lpstrDefExt: PCWSTR(csv.as_ptr()),
        Flags: if save { OFN_OVERWRITEPROMPT | OFN_HIDEREADONLY | OFN_PATHMUSTEXIST } else { OFN_FILEMUSTEXIST | OFN_HIDEREADONLY | OFN_PATHMUSTEXIST },
        ..Default::default()
    };
    let ok = unsafe {
        if save { GetSaveFileNameW(&mut ofn) } else { GetOpenFileNameW(&mut ofn) }
    };
    ok.as_bool().then(|| {
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        std::path::PathBuf::from(String::from_utf16_lossy(&buf[..n]))
    })
}

pub fn pick_open(hwnd: isize) -> Option<std::path::PathBuf> {
    pick(hwnd, false)
}
pub fn pick_save(hwnd: isize) -> Option<std::path::PathBuf> {
    pick(hwnd, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(o: &str, u: &str, p: &str) -> Entry {
        Entry { origin: o.into(), username: u.into(), password: p.into(), note: String::new() }
    }

    #[test]
    fn origins_exact() {
        assert_eq!(origin_of("https://github.com/login?x=1").as_deref(), Some("https://github.com"));
        assert_eq!(origin_of("HTTPS://GitHub.com").as_deref(), Some("https://github.com"));
        assert_eq!(origin_of("http://localhost:18493/a").as_deref(), Some("http://localhost:18493"));
        assert_eq!(origin_of("https://user:pw@site.fr/x").as_deref(), Some("https://site.fr"));
        for bad in ["file:///C:/x", "ftp://a.fr", "https://", "about:blank", "/relatif"] {
            assert!(origin_of(bad).is_none(), "{bad}");
        }
        // origine approchante rejetée : sous-domaine et schéma différents ne correspondent pas
        let v = vec![e("https://github.com", "a", "p1"), e("https://github.com", "b", "p2"), e("http://github.com", "c", "p3")];
        assert_eq!(for_origin(&v, "https://github.com").len(), 2);
        assert_eq!(for_origin(&v, "https://login.github.com").len(), 0); // sous-domaine
        assert_eq!(for_origin(&v, "http://github.com").len(), 1); // schéma strict
    }

    #[test]
    fn csv_quotes_commas_newlines() {
        // en-tête dans l'ordre et la casse d'Edge, champs entre guillemets avec virgule et saut de ligne échappés
        let txt = "Name,URL,Username,Password,Note\r\n\
            Site,https://a.fr,alice,\"p,a\"\"ss\",\"ligne1\nligne2\"\r\n\
            ,https://b.fr/login,bob,secret,\r\n\
            Sans mdp,https://c.fr,carol,,note\r\n\
            Mauvaise origine,ftp://d.fr,dan,xyz,\r\n";
        let (v, skipped) = parse_csv(txt);
        assert_eq!(v.len(), 2);
        assert_eq!(skipped, 2); // ligne sans mot de passe + origine ftp
        assert_eq!(v[0].origin, "https://a.fr");
        assert_eq!(v[0].password, "p,a\"ss"); // virgule et guillemet échappés conservés
        assert_eq!(v[0].note, "ligne1\nligne2"); // saut de ligne dans le champ
        assert_eq!(v[1].origin, "https://b.fr"); // origine depuis une URL avec chemin
        // ordre de colonnes différent (Firefox) : mappé par nom
        let fx = "username,password,url,label\nzoe,mdp,https://z.fr,x\n";
        let (f, _) = parse_csv(fx);
        assert_eq!((f[0].origin.as_str(), f[0].username.as_str(), f[0].password.as_str()), ("https://z.fr", "zoe", "mdp"));
        // sans colonne mot de passe : rien, tout compté ignoré
        let (empty, sk) = parse_csv("name,url\nx,https://a.fr\n");
        assert!(empty.is_empty() && sk == 1);
    }

    #[test]
    fn merge_dedup() {
        let mut vault = vec![e("https://a.fr", "u1", "p1")];
        let added = merge(&mut vault, vec![e("https://a.fr", "u1", "px"), e("https://a.fr", "u2", "p2"), e("https://b.fr", "u1", "p3")]);
        assert_eq!(added, 2); // u1@a déjà présent ignoré
        assert_eq!(vault.len(), 3);
        let same = vault.clone();
        assert_eq!(merge(&mut vault, same), 0); // ré-import identique : rien
    }

    #[test]
    fn csv_roundtrip() {
        let v = vec![
            Entry { origin: "https://a.fr".into(), username: "alice".into(), password: "p,a\"ss".into(), note: "n".into() },
            e("http://localhost:3000", "", "x"),
        ];
        let (back, skipped) = parse_csv(&to_csv(&v));
        assert_eq!(skipped, 0);
        // aller-retour fidèle (comparé champ par champ : Entry n'a pas de Debug, pour qu'un mot de passe ne soit jamais affiché)
        assert!(back.iter().zip(&v).all(|(a, b)| a == b) && back.len() == v.len());
        assert!(to_csv(&v).starts_with("name,url,username,password,note\r\n"));
    }

    #[test]
    fn dpapi_roundtrip() {
        // le chiffré n'est pas lisible en clair ; le déchiffré rend le JSON d'origine
        let plain = br#"[{"origin":"https://a.fr","username":"u","password":"motDePasseSecret42"}]"#;
        let enc = protect(plain).expect("DPAPI protect");
        assert!(!enc.windows(plain.len()).any(|w| w == plain)); // le mot de passe n'apparaît pas en clair
        assert_eq!(unprotect(&enc).as_deref(), Some(&plain[..]));
    }
}
