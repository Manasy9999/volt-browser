// Import des favoris des navigateurs Chromium (fichier JSON `Bookmarks` de chaque profil, lu sans y toucher).
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::{Folder, Page, defaults::web_url};

// (identifiant, nom affiché, variable d'environnement, dossier des données)
const BROWSERS: [(&str, &str, &str, &str); 7] = [
    ("chrome", "Google Chrome", "LOCALAPPDATA", r"Google\Chrome\User Data"),
    ("edge", "Microsoft Edge", "LOCALAPPDATA", r"Microsoft\Edge\User Data"),
    ("brave", "Brave", "LOCALAPPDATA", r"BraveSoftware\Brave-Browser\User Data"),
    ("vivaldi", "Vivaldi", "LOCALAPPDATA", r"Vivaldi\User Data"),
    ("chromium", "Chromium", "LOCALAPPDATA", r"Chromium\User Data"),
    ("opera", "Opera", "APPDATA", r"Opera Software\Opera Stable"),
    ("operagx", "Opera GX", "APPDATA", r"Opera Software\Opera GX Stable"),
];

/// (identifiant stable, navigateur, profil, fichier Bookmarks) de chaque profil trouvé.
fn profiles() -> Vec<(String, &'static str, String, PathBuf)> {
    let mut out = vec![];
    for (key, name, env, sub) in BROWSERS {
        let Some(root) = std::env::var_os(env).map(|d| PathBuf::from(d).join(sub)) else { continue };
        // noms affichés des profils (Local State) ; à défaut, le nom du dossier
        let state: Value = std::fs::read(root.join("Local State")).ok().and_then(|b| crate::parse_json(&b)).unwrap_or_default();
        let mut dirs: Vec<String> = std::fs::read_dir(&root).into_iter().flatten().flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|d| d == "Default" || d.starts_with("Profile ")).collect();
        dirs.sort();
        dirs.insert(0, String::new()); // anciens Opera : favoris à la racine
        for d in dirs {
            let f = root.join(&d).join("Bookmarks");
            if f.is_file() {
                let shown = state["profile"]["info_cache"][&d]["name"].as_str().map_or(if d.is_empty() { "Default".into() } else { d.clone() }, String::from);
                out.push((format!("{key}/{d}"), name, shown, f));
            }
        }
    }
    out
}

fn read(f: &PathBuf) -> Option<Value> {
    crate::parse_json(&std::fs::read(f).ok()?)
}

/// Sources contenant au moins un favori importable, pour `renderBrowsers`.
pub fn sources() -> Value {
    let list: Vec<Value> = profiles()
        .into_iter()
        .filter_map(|(id, browser, profile, f)| {
            let (mut favs, mut dirs) = (vec![], vec![]);
            let n = merge(&read(&f)?, &mut favs, &mut dirs, "x");
            (n > 0).then(|| json!({"id": id, "browser": browser, "profile": profile, "count": n}))
        })
        .collect();
    json!(list)
}

/// Importe la source `id` (seulement une de la liste : pas de chemin venu de l'interface) ; None si introuvable.
pub fn import(id: &str, favs: &mut Vec<Page>, folders: &mut Vec<Folder>) -> Option<usize> {
    let (.., f) = profiles().into_iter().find(|p| p.0 == id)?;
    // identifiants de dossier comme newdir dans l'interface : horodatage en base 36
    let mut ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
    let mut stamp = vec![];
    while ms > 0 {
        stamp.push(std::char::from_digit((ms % 36) as u32, 36).unwrap());
        ms /= 36;
    }
    Some(merge(&read(&f)?, favs, folders, &stamp.iter().rev().collect::<String>()))
}

/// Fusionne un JSON Bookmarks : URL http(s) sans doublon, barre sans dossier, un dossier Volt par dossier
/// de premier niveau (sous-dossiers aplatis, dossier homonyme réutilisé, aucun dossier vide). Renvoie le nombre ajouté.
pub fn merge(bm: &Value, favs: &mut Vec<Page>, folders: &mut Vec<Folder>, stamp: &str) -> usize {
    fn urls<'a>(n: &'a Value, out: &mut Vec<&'a Value>) {
        match n["type"].as_str() {
            Some("url") => out.push(n),
            _ => n["children"].as_array().into_iter().flatten().for_each(|c| urls(c, out)),
        }
    }
    let before = favs.len();
    for (root, bar) in [("bookmark_bar", true), ("other", false), ("synced", false)] {
        let r = &bm["roots"][root];
        // favoris posés à la racine : sans dossier sur la barre, dans un dossier au nom de la racine sinon
        let mut groups: Vec<(Option<&str>, Vec<&Value>)> = vec![(if bar { None } else { r["name"].as_str() }, vec![])];
        for c in r["children"].as_array().into_iter().flatten() {
            if c["type"] == "url" {
                groups[0].1.push(c);
            } else {
                let mut v = vec![];
                urls(c, &mut v);
                groups.push((c["name"].as_str(), v));
            }
        }
        for (name, nodes) in groups {
            for n in nodes {
                let Some(url) = n["url"].as_str().and_then(web_url) else { continue };
                if favs.iter().any(|f| f.url == url) {
                    continue;
                }
                // dossier créé au premier favori qu'il reçoit : jamais de dossier vide
                let folder = match name {
                    None => String::new(),
                    Some(name) => match folders.iter().find(|d| d.name == name) {
                        Some(d) => d.id.clone(),
                        None => {
                            let id = (0..).map(|i| format!("{stamp}{i}")).find(|id| !folders.iter().any(|d| &d.id == id)).unwrap();
                            // mêmes couleur et icône qu'un dossier créé dans l'interface (newdir)
                            folders.push(Folder { id: id.clone(), name: name.into(), color: String::new(), icon: "📁".into() });
                            id
                        }
                    },
                };
                let title = n["name"].as_str().filter(|t| !t.is_empty()).unwrap_or(&url).to_string();
                favs.push(Page { url, title, icon: String::new(), folder });
            }
        }
    }
    favs.len() - before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmarks_to_volt() {
        let bm: Value = serde_json::from_str(
            r#"{"roots":{
            "bookmark_bar":{"type":"folder","name":"Barre","children":[
                {"type":"url","name":"A","url":"https://a.fr"},
                {"type":"url","name":"","url":"https://b.fr"},
                {"type":"url","name":"JS","url":"javascript:alert(1)"},
                {"type":"folder","name":"Jeux","children":[
                    {"type":"url","name":"C","url":"https://c.fr"},
                    {"type":"folder","name":"Sous","children":[{"type":"url","name":"D","url":"http://d.fr"},{"type":"url","name":"A2","url":"https://a.fr"}]}]},
                {"type":"folder","name":"Vide","children":[{"type":"folder","name":"VideAussi","children":[]}]},
                {"type":"folder","name":"Fichiers","children":[{"type":"url","name":"F","url":"file:///C:/x"}]},
                {"type":"folder","name":"Existant","children":[{"type":"url","name":"E","url":"https://e.fr"}]}]},
            "other":{"type":"folder","name":"Autres favoris","children":[{"type":"url","name":"O","url":"https://o.fr"}]},
            "synced":{"type":"folder","name":"Mobile","children":[]}}}"#,
        )
        .unwrap();
        let mut favs = vec![Page { url: "https://c.fr".into(), title: "déjà là".into(), icon: String::new(), folder: String::new() }];
        let mut dirs = vec![Folder { id: "old".into(), name: "Existant".into(), color: "#f00".into(), icon: "🎮".into() }];
        assert_eq!(merge(&bm, &mut favs, &mut dirs, "t"), 5); // a, b, d, e, o
        let f = |u: &str| favs.iter().find(|p| p.url == u).unwrap();
        assert!(f("https://a.fr").folder.is_empty() && f("https://a.fr").title == "A");
        assert_eq!(f("https://b.fr").title, "https://b.fr"); // sans nom : l'URL
        assert_eq!(f("https://c.fr").title, "déjà là"); // existant intact
        let jeux = dirs.iter().find(|d| d.name == "Jeux").unwrap();
        assert_eq!(f("http://d.fr").folder, jeux.id); // sous-dossier aplati
        assert_eq!(f("https://e.fr").folder, "old"); // dossier homonyme réutilisé
        assert_eq!(f("https://o.fr").folder, dirs.iter().find(|d| d.name == "Autres favoris").unwrap().id);
        let names: Vec<_> = dirs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["Existant", "Jeux", "Autres favoris"]); // ni Vide, ni Fichiers, ni Mobile
        assert!(dirs[1].id != dirs[2].id && dirs[1].icon == "📁");
        assert_eq!(merge(&bm, &mut favs, &mut dirs, "t"), 0); // second import : rien
        assert_eq!(dirs.len(), 3);
        assert_eq!(merge(&json!({}), &mut favs, &mut dirs, "t"), 0); // JSON sans racines
    }
}
