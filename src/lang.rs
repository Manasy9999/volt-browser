// Langues de l'interface. Le français est le texte source, écrit tel quel dans le code ; chaque autre langue est un
// dictionnaire src/lang/<code>.json (clé = texte français, valeur = traduction) embarqué par build.rs : ajouter une
// langue, c'est ajouter ce fichier, rien d'autre. Clés réservées : « @langue » (son nom, pour le réglage) et « @mots »
// (ses petits mots courants, qui font reconnaître une page écrite dans cette langue : translate.rs).
// Ajouter un texte : `lang::t("…")` dans le Rust, `T("…")` dans le script de ui.html (le HTML statique est traduit tout
// seul), et sa traduction dans chaque dictionnaire : le test `every_text_is_translated` échoue tant qu'elle manque.
use std::{collections::HashMap, sync::RwLock};

use serde_json::json;
use windows::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName, LOCALE_ALLOW_NEUTRAL_NAMES, MAX_LOCALE_NAME};

// (code, contenu du fichier) de chaque dictionnaire : liste générée par build.rs
const DICTS: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/langs.rs"));
// Ligne de ui.html que `ui` remplace ; laissée telle quelle, l'interface est en français.
const UI_LANG: &str = r#"const LANG={code:"fr",dict:{},list:[],go:""};"#;
const FR_WORDS: &str = "le les des et du une est dans pour qui pas sur au aux avec ce cette il elle nous vous sont mais ou par plus son ses leur été";

// Langue en cours : (code, dictionnaire) ; None = français.
static CUR: RwLock<Option<(&'static str, HashMap<String, String>)>> = RwLock::new(None);

/// Dictionnaire illisible : vide, donc tout reste en français (le test `every_text_is_translated` le signale).
fn parse(json: &str) -> HashMap<String, String> {
    serde_json::from_str(json).unwrap_or_default()
}

/// Langue d'affichage de Windows (« fr-FR »), celle que le moteur prend de lui-même ; VOLT_LANG la remplace (essais).
pub fn system() -> String {
    if let Ok(l) = std::env::var("VOLT_LANG") {
        return l;
    }
    let mut b = [0u16; MAX_LOCALE_NAME as usize];
    let n = unsafe { LCIDToLocaleName(GetUserDefaultUILanguage() as u32, Some(&mut b), LOCALE_ALLOW_NEUTRAL_NAMES) };
    String::from_utf16_lossy(&b[..(n.max(1) - 1) as usize])
}

fn known(code: &str) -> Option<&'static str> {
    std::iter::once("fr").chain(DICTS.iter().map(|d| d.0)).find(|k| k.eq_ignore_ascii_case(code))
}

/// Langue de l'interface pour un réglage (« auto », « fr », « en »…) : celle du réglage s'il en désigne une, sinon
/// celle de Windows (« fr-FR » -> « fr »), sinon l'anglais.
pub fn resolve(setting: &str, system: &str) -> &'static str {
    known(setting)
        .or_else(|| known(system))
        .or_else(|| known(system.split('-').next().unwrap_or_default()))
        .unwrap_or_else(|| known("en").unwrap_or("fr"))
}

/// Langue à imposer au moteur (`--lang` : menus des pages, lecteur PDF, pages d'erreur, Accept-Language), ou None pour
/// le laisser suivre Windows comme il le fait de lui-même : réglage automatique, ou langue choisie qui est déjà celle
/// de Windows (sa variante régionale reste alors : « en-GB » et non « en-US »).
pub fn engine(setting: &str, system: &str) -> Option<&'static str> {
    known(setting).filter(|c| !system.to_ascii_lowercase().starts_with(&c.to_ascii_lowercase()))
}

/// Applique le réglage ; vrai si la langue de l'interface a changé.
pub fn set(setting: &str, system: &str) -> bool {
    let code = resolve(setting, system);
    let mut cur = CUR.write().unwrap();
    if cur.as_ref().map_or("fr", |c| c.0) == code {
        return false;
    }
    *cur = DICTS.iter().find(|d| d.0 == code).map(|d| (d.0, parse(d.1)));
    true
}

pub fn code() -> &'static str {
    CUR.read().unwrap().as_ref().map_or("fr", |c| c.0)
}

/// Texte dans la langue en cours ; le texte source si c'est le français ou si la traduction manque.
/// Pas pour un texte accordé selon un nombre (« a|b ») : l'interface s'en charge (`toast_n` dans main.rs, `T` dans ui.html).
pub fn t(fr: &str) -> String {
    CUR.read().unwrap().as_ref().and_then(|c| c.1.get(fr)).cloned().unwrap_or_else(|| fr.into())
}

/// `t` pour un message qui porte un détail (erreur du système, nom de fichier) à la place de « {e} ».
pub fn te(fr: &str, e: impl std::fmt::Display) -> String {
    t(fr).replace("{e}", &e.to_string())
}

/// Petits mots courants de la langue en cours, séparés par des espaces ("" : le dictionnaire n'en donne pas).
pub fn words() -> String {
    CUR.read().unwrap().as_ref().map_or(FR_WORDS.into(), |c| c.1.get("@mots").cloned().unwrap_or_default())
}

/// Page de l'interface dans la langue en cours : elle reçoit le code, le dictionnaire et les langues à proposer dans le
/// réglage ; `go` : section à ouvrir à l'arrivée (rechargement après un changement de langue), "" au démarrage.
pub fn ui(html: &str, go: &str) -> String {
    let cur = CUR.read().unwrap();
    let (code, dict) = cur.as_ref().map_or(("fr", json!({})), |c| (c.0, json!(c.1)));
    let list: Vec<_> = DICTS.iter().map(|d| json!([d.0, parse(d.1).get("@langue").cloned().unwrap_or_else(|| d.0.into())])).collect();
    // « < » échappé : rien de ce bloc ne peut fermer la balise <script>
    html.replacen(UI_LANG, &format!("const LANG={};", json!({"code": code, "dict": dict, "list": list, "go": go})).replace('<', "\\u003c"), 1)
}

/// Dictionnaires où `fr` n'a pas de traduction : pour les tests des libellés traduits à l'affichage (menus, dossiers proposés).
#[cfg(test)]
pub fn missing(fr: &str) -> Vec<&'static str> {
    DICTS.iter().filter(|d| !parse(d.1).contains_key(fr)).map(|d| d.0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_choice() {
        assert_eq!(resolve("auto", "fr-FR"), "fr");
        assert_eq!(resolve("auto", "fr-CA"), "fr");
        assert_eq!(resolve("auto", "en-GB"), "en");
        assert_eq!(resolve("auto", "zz-ZZ"), "en"); // langue sans dictionnaire : anglais
        assert_eq!(resolve("auto", ""), "en");
        assert_eq!(resolve("fr", "en-US"), "fr");
        assert_eq!(resolve("en", "fr-FR"), "en");
        assert_eq!(resolve("EN", "fr-FR"), "en");
        assert_eq!(resolve("klingon", "fr-FR"), "fr"); // réglage inconnu : comme « auto »
        // le moteur n'est forcé que si la langue choisie n'est pas déjà celle de Windows
        assert_eq!(engine("auto", "fr-FR"), None);
        assert_eq!(engine("auto", "zz-ZZ"), None);
        assert_eq!(engine("fr", "fr-FR"), None);
        assert_eq!(engine("en", "en-GB"), None);
        assert_eq!(engine("en", "fr-FR"), Some("en"));
        assert_eq!(engine("fr", "en-US"), Some("fr"));
    }

    #[test]
    fn page_gets_its_language() {
        // les tests tournent en français (aucun `set`) : texte source, dictionnaire vide, langues proposées
        assert!(crate::UI.contains(UI_LANG), "la ligne `const LANG=…` de ui.html a changé : `ui` ne la remplace plus");
        let p = ui(crate::UI, "start");
        assert!(!p.contains(UI_LANG) && p.contains(r#""code":"fr""#) && p.contains(r#""go":"start""#) && p.contains(r#"["en","English"]"#));
        assert_eq!((code(), t("Fermer"), te("Échec : {e}", 5)), ("fr", "Fermer".into(), "Échec : 5".into()));
        assert!(words().contains("les") && parse(DICTS.iter().find(|d| d.0 == "en").unwrap().1)["@mots"].contains("the"));
        assert!(missing("Fermer").is_empty() && missing("texte jamais écrit nulle part").len() == DICTS.len());
    }

    /// Textes du HTML statique, ceux que l'interface traduit seule au chargement : nœuds de texte, attributs title,
    /// placeholder et aria-label.
    fn html_texts(body: &str) -> Vec<String> {
        let mut out = vec![];
        for part in body.split('<') {
            let (tag, text) = part.split_once('>').unwrap_or(("", part));
            for a in [" title=\"", " placeholder=\"", " aria-label=\""] {
                out.extend(tag.match_indices(a).filter_map(|(i, _)| tag[i + a.len()..].split('"').next()).map(String::from));
            }
            out.push(text.trim().to_string());
        }
        out.retain(|t| t.chars().any(char::is_alphabetic));
        out
    }

    /// Chaînes du script : (contenu, argument direct de `T(` ?). Commentaires et expressions régulières sautés ; dans
    /// un gabarit, chaque `${…}` est remplacé par \u{1} et son contenu parcouru à son tour.
    fn js_strings(js: &str) -> Vec<(String, bool)> {
        fn scan(c: &[char], mut i: usize, inner: bool, out: &mut Vec<(String, bool)>) -> usize {
            let (mut prev, mut depth) = (';', 0usize); // prev : dernier caractère de code, il distingue /regex/ d'une division
            while i < c.len() {
                let ch = c[i];
                match ch {
                    '/' if c.get(i + 1) == Some(&'/') => {
                        i += c[i..].iter().position(|x| *x == '\n').unwrap_or(c.len() - i);
                        continue;
                    }
                    '/' if c.get(i + 1) == Some(&'*') => {
                        while i + 1 < c.len() && (c[i], c[i + 1]) != ('*', '/') {
                            i += 1;
                        }
                        i += 1;
                    }
                    // expression régulière : jusqu'au « / » fermant de la même ligne, hors classe [...] ; sinon une division
                    '/' if "(,=:[!&|?{};>".contains(prev) => {
                        let (mut j, mut class) = (i + 1, false);
                        while j < c.len() && c[j] != '\n' && (c[j] != '/' || class) {
                            match c[j] {
                                '\\' => j += 1,
                                '[' => class = true,
                                ']' => class = false,
                                _ => {}
                            }
                            j += 1;
                        }
                        if c.get(j) == Some(&'/') {
                            i = j;
                        }
                    }
                    '"' | '\'' | '`' => {
                        let wrapped = i >= 2 && c[i - 1] == '(' && c[i - 2] == 'T' && !(i >= 3 && (c[i - 3].is_alphanumeric() || "_$.".contains(c[i - 3])));
                        let mut s = String::new();
                        i += 1;
                        // une chaîne simple finit sur sa ligne : au pire, une erreur de découpage ne dépasse pas la ligne
                        while i < c.len() && c[i] != ch && (ch == '`' || c[i] != '\n') {
                            if c[i] == '\\' {
                                i += 1;
                                s.extend(c.get(i));
                            } else if ch == '`' && c[i] == '$' && c.get(i + 1) == Some(&'{') {
                                i = scan(c, i + 2, true, out);
                                s.push('\u{1}');
                                continue;
                            } else {
                                s.push(c[i]);
                            }
                            i += 1;
                        }
                        out.push((s, wrapped));
                    }
                    '{' if inner => depth += 1,
                    '}' if inner && depth == 0 => return i + 1,
                    '}' if inner => depth -= 1,
                    _ => {}
                }
                if !ch.is_whitespace() {
                    prev = ch;
                }
                i += 1;
            }
            i
        }
        let (c, mut out) = (js.chars().collect::<Vec<_>>(), vec![]);
        scan(&c, 0, false, &mut out);
        out
    }

    /// Textes écrits en clair dans un appel à t(), te() ou toast_n() d'une source Rust.
    fn rust_texts(src: &str) -> Vec<String> {
        let mut out = vec![];
        for f in ["t(\"", "te(\"", "toast_n(\""] {
            for (i, _) in src.match_indices(f) {
                if src[..i].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue; // get("…"), split("…"), write("…")…
                }
                let (mut s, mut it) = (String::new(), src[i + f.len()..].chars());
                while let Some(c) = it.next() {
                    match c {
                        '"' => break,
                        '\\' => s.extend(it.next()),
                        c => s.push(c),
                    }
                }
                out.push(s);
            }
        }
        out
    }

    /// Sources Rust (sauf celle-ci, dont les tests citent des textes d'exemple) : lues sur le disque, un fichier ajouté est vu.
    fn rust_sources() -> Vec<(String, String)> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut v: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "rs") && !p.ends_with("lang.rs")).collect();
        v.sort();
        v.iter().map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(p).unwrap())).collect()
    }

    fn script() -> (&'static str, &'static str) {
        crate::UI.split_once("<body").unwrap().1.split_once("<script>").unwrap()
    }

    /// Garde contre les oublis : chaque texte de l'interface (HTML statique, `T("…")` du script, `t("…")` du Rust) a sa
    /// traduction dans chaque dictionnaire. Le message d'échec donne les clés à ajouter, prêtes à copier.
    #[test]
    fn every_text_is_translated() {
        let (body, js) = script();
        let mut texts = html_texts(body);
        let strings = js_strings(js);
        assert!(strings.iter().all(|s| !s.1 || !s.0.contains('\u{1}')), "T(`…${{x}}…`) : la clé doit être un texte fixe, écris T(\"… {{x}} …\",{{x}})");
        texts.extend(strings.into_iter().filter(|s| s.1).map(|s| s.0));
        texts.extend(rust_sources().iter().flat_map(|(_, src)| rust_texts(src)));
        assert!(texts.len() > 400, "extraction des textes en panne : {} trouvés", texts.len());
        assert!(!DICTS.is_empty());
        for (code, json) in DICTS {
            let d: HashMap<String, String> = serde_json::from_str(json).unwrap_or_else(|e| panic!("src/lang/{code}.json illisible : {e}"));
            assert!(d.get("@langue").is_some_and(|n| !n.is_empty()), "src/lang/{code}.json : « @langue » (nom de la langue) manque");
            let mut missing: Vec<&str> = vec![];
            for t in texts.iter().filter(|t| !d.contains_key(*t)) {
                if !missing.contains(&t.as_str()) {
                    missing.push(t);
                }
            }
            assert!(missing.is_empty(), "textes sans traduction : à ajouter dans src/lang/{code}.json (clé = texte français)\n{}", serde_json::to_string_pretty(&missing).unwrap());
            // une traduction est aussi posée dans du HTML et dans des attributs entre guillemets droits
            for (k, v) in &d {
                assert!(!v.contains(['<', '"']), "src/lang/{code}.json, « {k} » : ni < ni guillemet droit dans une traduction (“ ” à la place)");
            }
        }
    }

    /// Texte français écrit dans le code sans passer par la traduction : il resterait en français dans les autres langues.
    /// shortcut: un texte neuf sans accent ni mot déjà traduit passe entre les mailles ; à revoir si un oubli arrive jusqu'à l'écran.
    #[test]
    fn no_french_text_outside_translation() {
        let dicts: Vec<_> = DICTS.iter().map(|d| parse(d.1)).collect();
        let visible = |s: &str| {
            // texte d'une chaîne, balises et ${…} retirés
            let mut text = String::new();
            let mut tag = false;
            for ch in s.chars() {
                match ch {
                    '<' => tag = true,
                    '>' => tag = false,
                    c if !tag && c != '\u{1}' => text.push(c),
                    _ => {}
                }
            }
            text.trim().to_string()
        };
        // script : chaîne hors de T() qui porte un accent français ou un texte déjà au dictionnaire
        let mut bad: Vec<String> = js_strings(script().1)
            .into_iter()
            .filter(|s| !s.1)
            .map(|s| visible(&s.0))
            .filter(|t| t.chars().any(|c| "àâäçéèêëîïôöùûüÿœÀÂÇÉÈÊÎÔÛŒ«»".contains(c)) || dicts.iter().any(|d| d.get(t).is_some_and(|v| v != t)))
            .map(|t| format!("ui.html : « {t} » hors de T(\"…\")"))
            .collect();
        // Rust : un toast reçoit un texte déjà traduit
        bad.extend(rust_sources().iter().filter(|(_, src)| src.contains("toast(\"") || src.contains("Toast(\"")).map(|(name, _)| format!("{name} : toast(\"…\") en clair, écris toast(&lang::t(\"…\"))")));
        assert!(bad.is_empty(), "textes visibles écrits sans passer par la traduction :\n{bad:#?}");
    }
}
