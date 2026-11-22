// Traduction de la page par le relais de Google Traduction (le moteur n'en a pas) : la page traduite est servie
// depuis <hôte encodé>.translate.goog ; détection de la langue à la fin du chargement.
use crate::{cmp, defaults};

/// Adresse du relais qui sert `url` traduite en français.
pub fn relay(url: &str) -> String {
    format!("https://translate.google.com/translate?sl=auto&tl=fr&u={}", super::enc(url))
}

pub fn is_relay(url: &str) -> bool {
    cmp::site(url).is_some_and(|h| h.ends_with(".translate.goog"))
}

/// Pages que le relais ne sait pas servir (mesuré le 22/11/2026) : Google refuse ses propres pages (une recherche
/// donne une page anti-robot sans issue), YouTube reste vide. L'icône n'y est pas proposée.
pub fn can_relay(url: &str) -> bool {
    !is_relay(url) && defaults::web_url(url).is_some() && cmp::site(url).is_some_and(|h| !h.split('.').any(|l| l == "google" || l == "youtube"))
}

/// Le relais a répondu par la page anti-robot de Google (`/sorry/`), dont la vérification ne marche pas sur ce domaine.
pub fn is_refused(url: &str) -> bool {
    is_relay(url) && url.to_ascii_lowercase().split_once(".translate.goog").is_some_and(|(_, t)| t.starts_with("/sorry/"))
}

/// Adresse d'origine portée par la page de refus (paramètre `continue`) ; None si absente ou pas http(s).
pub fn refused_origin(url: &str) -> Option<String> {
    let c = url.split_once('?')?.1.split('&').find_map(|p| p.strip_prefix("continue="))?;
    let (b, mut out, mut i) = (c.as_bytes(), vec![], 0);
    while i < b.len() {
        // %XX -> octet ; tout le reste tel quel
        let hex = (b[i] == b'%').then(|| c.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())).flatten();
        out.push(hex.unwrap_or(b[i]));
        i += if hex.is_some() { 3 } else { 1 };
    }
    defaults::web_url(&String::from_utf8(out).ok()?)
}

/// Adresse d'origine d'une page du relais ; None si ce n'en est pas une ou si le résultat n'est pas http(s).
/// Encodage constaté (novembre 2026) : « - » -> « -- », « . » -> « - » ; hôte trop long : son début passe dans
/// `_x_tr_hp` ; hôte en « xn-- » : « xn---- » devient « 0- » (`_x_tr_enc=0`) ; `_x_tr_sch=http`, `_x_tr_port`.
pub fn original(url: &str) -> Option<String> {
    let rest = url.get(..8).filter(|p| p.eq_ignore_ascii_case("https://")).map(|_| &url[8..])?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (host, tail) = rest.split_at(end);
    let enc = host.to_ascii_lowercase().strip_suffix(".translate.goog")?.to_string();
    let (tail, frag) = tail.split_once('#').map_or((tail, ""), |(a, _)| (a, &tail[a.len()..]));
    let (path, query) = tail.split_once('?').unwrap_or((tail, ""));
    let (mut sch, mut port, mut hp, mut idn, mut keep) = ("https", "", "", false, vec![]);
    for p in query.split('&').filter(|p| !p.is_empty()) {
        match p.split_once('=').unwrap_or((p, "")) {
            ("_x_tr_sch", "http") => sch = "http",
            ("_x_tr_port", v) => port = v,
            ("_x_tr_hp", v) => hp = v,
            ("_x_tr_enc", "0") => idn = true,
            (k, _) if k.starts_with("_x_tr_") => {}
            _ => keep.push(p),
        }
    }
    let mut e = format!("{hp}{enc}");
    if idn {
        e = format!("xn----{}", e.strip_prefix("0-")?);
    }
    // seuls lettres, chiffres et tirets : `_x_tr_hp` vient de la requête, rien d'autre ne doit entrer dans l'hôte
    if e.is_empty() || !e.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') || !port.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (mut h, mut it) = (String::new(), e.chars().peekable());
    while let Some(c) = it.next() {
        h.push(match c {
            '-' if it.next_if_eq(&'-').is_some() => '-',
            '-' => '.',
            c => c,
        });
    }
    let port = if port.is_empty() { String::new() } else { format!(":{port}") };
    let q = if keep.is_empty() { String::new() } else { format!("?{}", keep.join("&")) };
    defaults::web_url(&format!("{sch}://{h}{port}{path}{q}{frag}"))
}

// [1 français, 0 autre langue, -1 inconnu ; champ de mot de passe]. Attribut `lang` d'abord, sinon part des petits mots
// français dans le texte (pages françaises 26 à 35 %, autres 0 à 1 % : seuil à 8 %).
pub const LANG_JS: &str = r#"(()=>{const l=(document.documentElement.lang||'').trim().toLowerCase();let f=l?+/^fr\b/.test(l):-1;if(f<0){const w=((document.body&&document.body.innerText)||'').slice(0,30000).toLowerCase().match(/[a-zàâäçéèêëîïôöûùüÿœ]+/g)||[];if(w.length>=40){const F=new Set(['le','les','des','et','du','une','est','dans','pour','qui','pas','sur','au','aux','avec','ce','cette','il','elle','nous','vous','sont','mais','ou','par','plus','son','ses','leur','été']);f=+(w.filter(x=>F.has(x)).length/w.length>=.08)}}return[f,!!document.querySelector('input[type=password]')]})()"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_and_back() {
        assert_eq!(relay("https://en.wikipedia.org/wiki/A?b=1&c=é#d"), "https://translate.google.com/translate?sl=auto&tl=fr&u=https%3A%2F%2Fen.wikipedia.org%2Fwiki%2FA%3Fb%3D1%26c%3D%C3%A9%23d");
        // adresses servies par le relais (relevées le 22/11/2026) -> adresse d'origine exacte
        for (r, o) in [
            ("https://en-wikipedia-org.translate.goog/wiki/Rust_(programming_language)?a=1&_x_tr_sl=auto&_x_tr_tl=fr&_x_tr_hl=fr#History", "https://en.wikipedia.org/wiki/Rust_(programming_language)?a=1#History"),
            ("https://www-my--site-co-uk.translate.goog/x?_x_tr_sl=auto&_x_tr_tl=fr&_x_tr_hl=fr", "https://www.my-site.co.uk/x"),
            ("https://neverssl-com.translate.goog/?_x_tr_sch=http&_x_tr_sl=auto&_x_tr_tl=fr&_x_tr_hl=fr", "http://neverssl.com/"),
            ("https://news-ycombinator-com.translate.goog/item?id=1&_x_tr_sl=auto&_x_tr_tl=fr&_x_tr_hl=fr&_x_tr_pto=wapp", "https://news.ycombinator.com/item?id=1"),
            ("https://a--b--c-example--site-com.translate.goog/p?q=1&r=2&_x_tr_sl=auto&_x_tr_tl=fr&_x_tr_hl=fr", "https://a-b-c.example-site.com/p?q=1&r=2"),
            ("https://example-com.translate.goog/x?_x_tr_port=8443&_x_tr_sl=auto&_x_tr_tl=fr&_x_tr_hl=fr", "https://example.com:8443/x"),
            ("https://0-caf--dma-fr.translate.goog/?_x_tr_enc=0&_x_tr_sl=auto&_x_tr_tl=fr&_x_tr_hl=fr", "https://xn--caf-dma.fr/"),
            ("https://www-xn----caf--dma-fr.translate.goog/a?_x_tr_sl=auto&_x_tr_tl=fr&_x_tr_hl=fr", "https://www.xn--caf-dma.fr/a"),
            ("https://de-xn----mnchen--3ya-de.translate.goog/?_x_tr_sl=auto&_x_tr_tl=fr&_x_tr_hl=fr", "https://de.xn--mnchen-3ya.de/"),
            ("https://1--2-3-com.translate.goog/?_x_tr_sl=auto&_x_tr_tl=fr&_x_tr_hl=fr", "https://1-2.3.com/"),
            (
                "https://klmnopqrstuvwxyzabcdefghij-klmnopqrstuvwxyzabcdefghijklmnop-com.translate.goog/?_x_tr_hp=www-abcdefghij&_x_tr_sl=auto&_x_tr_tl=fr&_x_tr_hl=fr",
                "https://www.abcdefghijklmnopqrstuvwxyzabcdefghij.klmnopqrstuvwxyzabcdefghijklmnop.com/",
            ),
            ("https://www-spiegel-de.translate.goog/?_x_tr_sl=auto&_x_tr_tl=fr&_x_tr_hl=fr", "https://www.spiegel.de/"),
            ("https://WWW-Spiegel-DE.translate.goog", "https://www.spiegel.de"),
        ] {
            assert_eq!(original(r).as_deref(), Some(o), "{r}");
            assert!(is_relay(r));
        }
        for bad in [
            "https://en.wikipedia.org/wiki/A",
            "http://en-wikipedia-org.translate.goog/",
            "https://translate.goog/",
            "https://evil-com.translate.goog.evil.com/",
            "https://x-com.translate.goog/?_x_tr_hp=evil.com%2F",
            "https://x-com.translate.goog/?_x_tr_hp=a@b",
            "https://x-com.translate.goog/?_x_tr_port=80/evil",
            "https://x-com.translate.goog/?_x_tr_enc=0",
            "javascript:alert(1)//x-com.translate.goog/",
            "",
        ] {
            assert!(original(bad).is_none(), "{bad}");
        }
        assert!(!is_relay("https://lemonde.fr/translate.goog"));
    }

    #[test]
    fn pages_the_relay_refuses() {
        for ok in ["https://en.wikipedia.org/wiki/A", "http://neverssl.com/", "https://google-fan.fr/x", "https://notgoogle.com/"] {
            assert!(can_relay(ok), "{ok}");
        }
        for no in ["https://www.google.com/search?q=a", "https://google.fr/", "https://mail.google.com/x", "https://www.google.co.uk/", "https://www.youtube.com/watch?v=1", "https://m.youtube.com/", "https://en-wikipedia-org.translate.goog/", "javascript:alert(1)", "volt://newtab", ""] {
            assert!(!can_relay(no), "{no}");
        }
        // adresse relevée en traduisant une recherche Google
        let r = "https://www-google-com.translate.goog/sorry/index?continue=https://www.google.com/search?q%3Dorion%2Bnebula%26udm%3D2&q=EgRC";
        assert!(is_refused(r) && is_refused("https://WWW-Google-COM.translate.goog/sorry/index"));
        assert_eq!(refused_origin(r).as_deref(), Some("https://www.google.com/search?q=orion+nebula&udm=2"));
        assert!(!is_refused("https://en-wikipedia-org.translate.goog/wiki/Sorry") && !is_refused("https://www.google.com/sorry/index"));
        for bad in ["https://x-com.translate.goog/sorry/index", "https://x-com.translate.goog/sorry/index?continue=javascript%3Aalert(1)", "https://x-com.translate.goog/sorry/index?continue=%ZZ%"] {
            assert!(refused_origin(bad).is_none(), "{bad}");
        }
    }
}
