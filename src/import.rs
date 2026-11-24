// Import des favoris des navigateurs Chromium (fichier JSON `Bookmarks` de chaque profil, lu sans y toucher), et
// import/export en fichier HTML, le format que tous les navigateurs (Firefox compris) savent écrire et relire.
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::{Folder, Page, defaults::web_url};

// (identifiant, nom affiché, variable d'environnement, dossier des données)
pub(crate) const BROWSERS: [(&str, &str, &str, &str); 7] = [
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
    Some(merge(&read(&f)?, favs, folders, &stamp()))
}

/// Base des identifiants de dossier, comme newdir dans l'interface : horodatage en base 36.
fn stamp() -> String {
    let mut ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
    let mut stamp = vec![];
    while ms > 0 {
        stamp.push(std::char::from_digit((ms % 36) as u32, 36).unwrap());
        ms /= 36;
    }
    stamp.iter().rev().collect()
}

/// Ajoute un favori s'il n'y est pas déjà ; `folder` : nom de son dossier (réutilisé s'il existe, créé au premier favori
/// qu'il reçoit : jamais de dossier vide), None = hors dossier. Renvoie vrai s'il a été ajouté.
fn add(url: String, title: &str, folder: Option<&str>, favs: &mut Vec<Page>, folders: &mut Vec<Folder>, stamp: &str) -> bool {
    if favs.iter().any(|f| f.url == url) {
        return false;
    }
    let folder = match folder {
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
    let title = if title.is_empty() { url.clone() } else { title.to_string() };
    favs.push(Page { url, title, icon: String::new(), folder });
    true
}

/// Texte d'un fichier HTML : les entités les plus courantes remises en clair.
fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
}

/// Favoris d'un fichier HTML de favoris : (adresse, titre, dossier). Le dossier est celui de premier niveau, les
/// sous-dossiers sont aplatis comme à l'import direct ; "" = hors dossier (ce qui est posé sur la barre de favoris).
pub fn parse_html(html: &str) -> Vec<(String, String, String)> {
    let low = html.to_ascii_lowercase(); // mêmes positions d'octets que `html`
    // texte entre la fin de la balise ouverte en `at` et la balise fermante `close` ; position après elle
    let inner = |at: usize, close: &str| -> (String, usize) {
        let start = low[at..].find('>').map_or(low.len(), |p| at + p + 1);
        let end = low[start..].find(close).map_or(low.len(), |p| start + p);
        (unescape(html[start..end].trim()), (end + close.len()).min(low.len()))
    };
    let mut out = vec![];
    let mut stack: Vec<String> = vec![]; // dossiers ouverts ; "" = niveau qui ne compte pas (racine, barre de favoris)
    let mut pending: Option<String> = None; // <H3> lu, sa liste <DL> pas encore ouverte
    let mut i = 0;
    while let Some(p) = low[i..].find('<') {
        let at = i + p;
        let rest = &low[at..];
        let tag_end = rest.find('>').map_or(low.len(), |p| at + p);
        if rest.starts_with("<h3") {
            let (name, end) = inner(at, "</h3>");
            pending = Some(if low[at..tag_end].contains("personal_toolbar_folder") { String::new() } else { name });
            i = end;
        } else if rest.starts_with("<dl") {
            stack.push(pending.take().unwrap_or_default());
            i = at + 3;
        } else if rest.starts_with("</dl") {
            stack.pop();
            i = at + 4;
        } else if rest.starts_with("<a") && rest[2..].starts_with(|c: char| c.is_ascii_whitespace()) {
            let href = low[at..tag_end].find("href=").map(|p| at + p + 5).and_then(|v| {
                let q = html[v..].chars().next().filter(|c| *c == '"' || *c == '\'')?;
                html[v + 1..tag_end].split(q).next()
            });
            let (title, end) = inner(at, "</a>");
            if let Some(u) = href.and_then(|h| web_url(&unescape(h))) {
                out.push((u, title, stack.iter().find(|s| !s.is_empty()).cloned().unwrap_or_default()));
            }
            i = end;
        } else {
            i = at + 1;
        }
    }
    out
}

/// Ajoute les favoris d'un fichier HTML ; renvoie le nombre ajouté (doublons et adresses non web ignorés).
pub fn import_html(html: &str, favs: &mut Vec<Page>, folders: &mut Vec<Folder>) -> usize {
    let st = stamp();
    parse_html(html).into_iter().filter(|(u, t, d)| add(u.clone(), t, (!d.is_empty()).then_some(d.as_str()), favs, folders, &st)).count()
}

/// Fichier HTML de favoris, relu par tous les navigateurs : chaque dossier, puis les favoris hors dossier.
pub fn to_html(favs: &[Page], folders: &[Folder]) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    let link = |f: &Page| format!("    <DT><A HREF=\"{}\">{}</A>\n", esc(&f.url), esc(&f.title));
    let mut o = String::from("<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">\n<TITLE>Bookmarks</TITLE>\n<H1>Bookmarks</H1>\n<DL><p>\n");
    for d in folders {
        o += &format!("    <DT><H3>{}</H3>\n    <DL><p>\n", esc(&d.name));
        favs.iter().filter(|f| f.folder == d.id).for_each(|f| o += &link(f));
        o += "    </DL><p>\n";
    }
    favs.iter().filter(|f| !folders.iter().any(|d| d.id == f.folder)).for_each(|f| o += &link(f));
    o + "</DL><p>\n"
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
                add(url, n["name"].as_str().unwrap_or_default(), name, favs, folders, stamp);
            }
        }
    }
    favs.len() - before
}

#[cfg(test)]
mod tests {
    use super::*;

    // tel que l'écrivent Chrome et Firefox : barre de favoris (qui ne compte pas comme dossier), dossier, sous-dossier
    const HTML: &str = r#"<!DOCTYPE NETSCAPE-Bookmark-file-1>
<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">
<TITLE>Bookmarks</TITLE><H1>Bookmarks</H1>
<DL><p>
    <DT><H3 ADD_DATE="1" PERSONAL_TOOLBAR_FOLDER="true">Barre de favoris</H3>
    <DL><p>
        <DT><A HREF="https://a.fr/" ADD_DATE="2" ICON="data:image/png;base64,AAAA">Site A &amp; Cie</A>
        <DT><H3>Jeux</H3>
        <DL><p>
            <DT><A HREF="https://jeu.fr/?x=1&amp;y=2">Un jeu</A>
            <DT><H3>Sous-dossier</H3>
            <DL><p>
                <DT><a href='https://profond.fr/'>Profond</a>
            </DL><p>
        </DL><p>
    </DL><p>
    <DT><H3>Autres favoris</H3>
    <DL><p>
        <DT><A HREF="javascript:alert(1)">Pas une page web</A>
        <DT><A HREF="https://b.fr/"></A>
    </DL><p>
    <DT><A HREF="https://racine.fr/">À la racine</A>
</DL><p>"#;

    #[test]
    fn bookmarks_html() {
        let got = parse_html(HTML);
        let want = [
            ("https://a.fr/", "Site A & Cie", ""),
            ("https://jeu.fr/?x=1&y=2", "Un jeu", "Jeux"),
            ("https://profond.fr/", "Profond", "Jeux"), // sous-dossier aplati dans son dossier de premier niveau
            ("https://b.fr/", "", "Autres favoris"),
            ("https://racine.fr/", "À la racine", ""),
        ];
        assert_eq!(got, want.map(|(u, t, d)| (u.to_string(), t.to_string(), d.to_string())));
        assert!(parse_html("").is_empty() && parse_html("<a href=").is_empty() && parse_html("<A HREF=\"https://x.fr\">sans fin").len() == 1);

        // import : doublon ignoré, dossier existant réutilisé, dossier créé seulement s'il reçoit un favori
        let mut favs = vec![Page { url: "https://a.fr/".into(), title: "Déjà là".into(), icon: String::new(), folder: String::new() }];
        let mut dirs = vec![Folder { id: "old".into(), name: "Jeux".into(), color: "#f00".into(), icon: "🎮".into() }];
        assert_eq!(import_html(HTML, &mut favs, &mut dirs), 4);
        assert_eq!(favs.len(), 5);
        assert_eq!(dirs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["Jeux", "Autres favoris"]);
        assert!(favs.iter().filter(|f| f.folder == "old").count() == 2 && favs.iter().find(|f| f.url == "https://b.fr/").unwrap().title == "https://b.fr/");
        assert_eq!(import_html(HTML, &mut favs, &mut dirs), 0); // relancé : rien de neuf

        // export relu à l'identique
        let back = parse_html(&to_html(&favs, &dirs));
        assert_eq!(back.len(), favs.len());
        for f in &favs {
            let dir = dirs.iter().find(|d| d.id == f.folder).map_or("", |d| d.name.as_str());
            assert!(back.contains(&(f.url.clone(), f.title.clone(), dir.to_string())), "{}", f.url);
        }
        assert!(to_html(&[Page { url: "https://x.fr/?a=1&b=<2>".into(), title: "\"T\" <i>".into(), icon: String::new(), folder: String::new() }], &[]).contains("HREF=\"https://x.fr/?a=1&amp;b=&lt;2&gt;\">&quot;T&quot; &lt;i&gt;</A>"));
    }

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
