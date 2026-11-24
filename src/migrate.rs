// Migration depuis un autre navigateur Chromium : découverte des profils et lecture de leurs données (cookies,
// historique, mots de passe) via `chromium`. Les favoris passent par `import` (déjà en place). L'application des
// données (injection des cookies, écriture du coffre, ajout à l'historique) se fait dans la boucle principale, qui a
// la vue et l'état sous la main.
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::chromium::{self, Cookie};

fn root(env: &str, sub: &str) -> Option<PathBuf> {
    std::env::var_os(env).map(|d| PathBuf::from(d).join(sub))
}

/// (clé, nom affiché, dossier racine) de chaque navigateur installé. En debug, VOLT_MIGRATE_TEST remplace tout par un
/// faux profil, pour les essais sans toucher aux vrais navigateurs (absent du binaire de production).
fn browsers() -> Vec<(&'static str, &'static str, PathBuf)> {
    #[cfg(debug_assertions)]
    if let Ok(p) = std::env::var("VOLT_MIGRATE_TEST") {
        return vec![("chrome", "Navigateur d'essai", PathBuf::from(p))];
    }
    crate::import::BROWSERS.into_iter().filter_map(|(k, n, e, s)| root(e, s).map(|r| (k, n, r))).collect()
}

/// Sous-dossiers de profil d'un navigateur : « Default », « Profile N », et "" (anciens Opera, à la racine).
fn profile_dirs(root: &Path) -> Vec<String> {
    let mut dirs: Vec<String> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|d| d == "Default" || d.starts_with("Profile "))
        .collect();
    dirs.sort();
    dirs.insert(0, String::new());
    dirs
}

/// Fichier Cookies du profil (emplacement récent « Network/Cookies », ou ancien « Cookies »).
fn cookies_file(profile: &Path) -> Option<PathBuf> {
    [profile.join("Network").join("Cookies"), profile.join("Cookies")].into_iter().find(|f| f.is_file())
}

/// (navigateur, dossier racine, sous-dossier de profil) à partir d'un identifiant « clé/profil ».
fn locate(id: &str) -> Option<(&'static str, PathBuf, String)> {
    let (key, d) = id.split_once('/')?;
    let (_, name, root) = browsers().into_iter().find(|b| b.0 == key)?;
    Some((name, root, d.to_string()))
}

/// Profils trouvés et ce qu'on peut en tirer : [{id, browser, profile, cookies, history, passwords, favorites}].
pub fn sources() -> Value {
    let mut out = vec![];
    for (key, name, root) in browsers() {
        let Some(root) = Some(root).filter(|r| r.exists()) else { continue };
        let state: Value = std::fs::read(root.join("Local State")).ok().and_then(|b| crate::parse_json(&b)).unwrap_or_default();
        let has_key = state["os_crypt"]["encrypted_key"].is_string(); // clé nécessaire pour cookies et mots de passe
        let mut seen = std::collections::HashSet::new();
        for d in profile_dirs(&root) {
            let profile = root.join(&d);
            let cookies = has_key && cookies_file(&profile).is_some();
            let history = profile.join("History").is_file();
            let passwords = has_key && profile.join("Login Data").is_file();
            let favorites = profile.join("Bookmarks").is_file();
            if !(cookies || history || passwords || favorites) {
                continue;
            }
            let id = format!("{key}/{d}");
            if !seen.insert(id.clone()) {
                continue;
            }
            let shown = state["profile"]["info_cache"][&d]["name"].as_str().map_or(if d.is_empty() { "Default".into() } else { d.clone() }, String::from);
            out.push(json!({"id": id, "browser": name, "profile": shown, "cookies": cookies, "history": history, "passwords": passwords, "favorites": favorites}));
        }
    }
    json!(out)
}

/// Données lues d'un profil, prêtes à appliquer dans la boucle principale.
#[derive(Default)]
pub struct Migrated {
    pub cookies: Vec<Cookie>,
    pub history: Vec<(String, String, i64, f64)>,  // adresse, titre, visites, heure unix
    pub logins: Vec<(String, String, String)>,     // site, utilisateur, mot de passe
    pub locked: bool,   // un fichier demandé n'a pas pu être lu : navigateur source ouvert ?
    pub app_bound: bool, // des valeurs « app-bound » (Chrome récent) ont été ignorées
}

/// Lit un fichier SQLite verrouillable (le navigateur source le tient) ; `locked` passe à vrai en cas d'échec.
fn read_db(f: &Path, m: &mut Migrated) -> Option<Vec<u8>> {
    match std::fs::read(f) {
        Ok(b) if b.starts_with(b"SQLite format 3\0") => Some(b),
        _ => {
            m.locked = true;
            None
        }
    }
}

/// Lit les données demandées d'un profil. Ne touche à rien dans Volt : l'appelant applique.
pub fn read(id: &str, cookies: bool, history: bool, passwords: bool) -> Migrated {
    let mut m = Migrated::default();
    let Some((_, root, d)) = locate(id) else { return m };
    let profile = root.join(&d);
    let key = (cookies || passwords).then(|| std::fs::read(root.join("Local State")).ok().and_then(|b| chromium::master_key(&b))).flatten();
    if cookies
        && let Some(f) = cookies_file(&profile)
        && let Some(db) = read_db(&f, &mut m)
        && let Some(k) = &key
    {
        m.app_bound |= chromium::app_bound_present(&db, "cookies", "encrypted_value");
        m.cookies = chromium::parse_cookies(&db, k);
    }
    if history
        && let Some(db) = read_db(&profile.join("History"), &mut m)
    {
        m.history = chromium::parse_history(&db);
    }
    if passwords
        && let Some(db) = read_db(&profile.join("Login Data"), &mut m)
        && let Some(k) = &key
    {
        m.app_bound |= chromium::app_bound_present(&db, "logins", "password_value");
        m.logins = chromium::parse_logins(&db, k);
    }
    m
}
