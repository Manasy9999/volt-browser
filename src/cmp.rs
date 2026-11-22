// Réponse automatique aux bandeaux de cookies : cmp.js (mis au point à part, intégré tel quel) ajouté à chaque onglet
// par AddScriptToExecuteOnDocumentCreated, qui le joue dans chaque document et chaque cadre, même d'un autre site.
// Aussi : saisie libre d'un site (listes « exclus » et « ne pas traduire ») -> nom d'hôte.
use std::{cell::RefCell, rc::Rc};

use webview2_com::AddScriptToExecuteOnDocumentCreatedCompletedHandler;
use windows::core::HSTRING;
use wry::{WebView, WebViewExtWindows};

const JS: &str = include_str!("cmp.js");
// les deux paramètres en tête de cmp.js, remplacés par leur valeur en JSON
const MODE: &str = r#"const MODE = "refuser";"#;
const EXCLUS: &str = "const EXCLUS = [];";

pub fn script(accept: bool, skip: &[String]) -> String {
    let mode = serde_json::json!(if accept { "accepter" } else { "refuser" });
    JS.replacen(MODE, &format!("const MODE = {mode};"), 1).replacen(EXCLUS, &format!("const EXCLUS = {};", serde_json::json!(skip)), 1)
}

/// Retire le script `old` (identifiant, "" = aucun) puis ajoute `js` ("" = rien) ; renvoie le nouvel identifiant.
/// Attendu sur place (comme les scripts de wry) : ajouté avant la navigation, il vaut dès le premier document.
pub fn swap(wv: &WebView, old: &str, js: &str) -> String {
    let w = wv.webview();
    if !old.is_empty() {
        let _ = unsafe { w.RemoveScriptToExecuteOnDocumentCreated(&HSTRING::from(old)) };
    }
    if js.is_empty() {
        return String::new();
    }
    let (id, js) = (Rc::new(RefCell::new(String::new())), HSTRING::from(js));
    let id2 = id.clone();
    let _ = AddScriptToExecuteOnDocumentCreatedCompletedHandler::wait_for_async_operation(
        Box::new(move |h| unsafe { w.AddScriptToExecuteOnDocumentCreated(&js, &h).map_err(Into::into) }),
        Box::new(move |r, i| {
            *id2.borrow_mut() = i;
            r
        }),
    );
    id.take()
}

/// Saisie libre (« https://www.Site.fr/page », « site.fr ») -> hôte en minuscules, sans « www. » ni port ;
/// None si ce n'est pas un nom d'hôte (au moins un point, lettres ASCII, chiffres et tirets).
pub fn site(input: &str) -> Option<String> {
    let t = input.trim().to_ascii_lowercase();
    let t = t.split_once("://").map_or(t.as_str(), |(_, r)| r);
    let h = t.split(['/', '?', '#']).next()?;
    let h = h.rsplit_once(':').filter(|(_, p)| p.bytes().all(|b| b.is_ascii_digit())).map_or(h, |(h, _)| h);
    let h = h.trim_end_matches('.');
    let h = h.strip_prefix("www.").unwrap_or(h);
    let label = |l: &str| !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-') && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    (h.contains('.') && h.len() <= 253 && h.split('.').all(label)).then(|| h.to_string())
}

/// Liste venue de l'interface : entrées normalisées, invalides et doublons retirés.
pub fn clean(list: &[String]) -> Vec<String> {
    let mut v: Vec<String> = vec![];
    for h in list.iter().filter_map(|x| site(x)) {
        if !v.contains(&h) {
            v.push(h);
        }
    }
    v
}

/// `host` est dans la liste, sous-domaines compris (comme EXCLUS dans cmp.js).
pub fn covers(host: &str, list: &[String]) -> bool {
    list.iter().any(|e| host == e || host.strip_suffix(e.as_str()).is_some_and(|p| p.ends_with('.')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_in_json() {
        // les deux paramètres sont bien là où on les attend (sinon le remplacement ne ferait rien)
        assert_eq!(JS.matches(MODE).count(), 1);
        assert_eq!(JS.matches(EXCLUS).count(), 1);
        let s = script(true, &["lemonde.fr".into(), "a\"b".into()]);
        assert!(s.contains(r#"const MODE = "accepter";"#) && s.contains(r#"const EXCLUS = ["lemonde.fr","a\"b"];"#));
        assert!(!s.contains(MODE) && !s.contains(EXCLUS));
        assert!(script(false, &[]).contains(r#"const MODE = "refuser";"#));
        assert_eq!(script(false, &[]).len(), JS.len()); // valeurs par défaut : le fichier tel quel
    }

    #[test]
    fn free_input_to_host() {
        for (i, o) in [
            ("lemonde.fr", "lemonde.fr"),
            ("  https://www.LeMonde.fr/politique/?x=1#a ", "lemonde.fr"),
            ("http://store.steampowered.com:8080/app", "store.steampowered.com"),
            ("WWW.Fnac.com", "fnac.com"),
            ("fnac.com.", "fnac.com"),
            ("my-site.co.uk/", "my-site.co.uk"),
            ("xn--caf-dma.fr", "xn--caf-dma.fr"),
            ("192.168.1.1", "192.168.1.1"),
        ] {
            assert_eq!(site(i).as_deref(), Some(o), "{i}");
        }
        for bad in ["", "   ", "lemonde", "javascript:alert(1)", "user@site.fr", "a b.fr", "café.fr", "-a.fr", "a-.fr", "a..fr", "https://", "site.fr:abc", "<b>.fr"] {
            assert!(site(bad).is_none(), "{bad}");
        }
        assert_eq!(clean(&["Site.fr".into(), "https://site.fr/x".into(), "nope".into(), "b.fr".into()]), ["site.fr", "b.fr"]);
        let l = ["lemonde.fr".to_string()];
        assert!(covers("lemonde.fr", &l) && covers("abo.lemonde.fr", &l));
        assert!(!covers("paslemonde.fr", &l) && !covers("lemonde.fr.evil.com", &l));
    }
}
