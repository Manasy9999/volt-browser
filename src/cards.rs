// Cartes de paiement enregistrées par l'utilisateur, pour remplir les formulaires de paiement. Le moteur ne sait pas le
// faire dans une vue intégrée : Volt tient donc son propre coffre, comme pour les mots de passe (fichier chiffré par
// DPAPI, lié à la session Windows). Le cryptogramme (CVC) n'est jamais demandé ni enregistré.
//
// Les champs de carte sont le plus souvent dans un cadre (iframe) d'un prestataire de paiement, d'une autre origine que
// la page : on suit donc les cadres. Règle de sûreté : le numéro ne part que vers UNE origine, celle du document où
// l'utilisateur a, de sa main, cliqué ou tapé dans un champ de carte en dernier (à défaut, la page affichée si elle
// porte elle-même le champ du numéro). Jamais vers les autres cadres, qu'une publicité pourrait garnir de faux champs.
// Ce « de sa main » ne se déclare pas : les messages qu'envoie une page peuvent être fabriqués par elle. Volt le
// relit donc lui-même dans chaque document (`PROBE`), par une fonction posée avant tout script de la page et que la
// page ne peut ni remplacer ni tromper (seuls les événements réels du navigateur portent `isTrusted`).
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use serde::{Deserialize, Serialize};
use webview2_com::{
    ExecuteScriptCompletedHandler, FrameChildFrameCreatedEventHandler, FrameCreatedEventHandler, FrameDestroyedEventHandler, FrameNavigationStartingEventHandler,
    FrameWebMessageReceivedEventHandler,
    Microsoft::Web::WebView2::Win32::{ICoreWebView2, ICoreWebView2_4, ICoreWebView2Frame, ICoreWebView2Frame2, ICoreWebView2Frame7},
    take_pwstr,
};
use windows::core::{BOOL, HSTRING, Interface, PWSTR};
use wry::{WebView, WebViewExtWindows};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Card {
    pub name: String,   // nom écrit sur la carte
    pub number: String, // chiffres seuls
    pub month: u8,
    pub year: u16, // quatre chiffres
}

pub fn load(path: &Path) -> Vec<Card> {
    std::fs::read(path).ok().and_then(|b| crate::vault::unprotect(&b)).and_then(|p| serde_json::from_slice(&p).ok()).unwrap_or_default()
}

pub fn save(path: &Path, cards: &[Card]) -> bool {
    serde_json::to_vec(cards).ok().and_then(|j| crate::vault::protect(&j)).is_some_and(|enc| std::fs::write(path, enc).is_ok())
}

/// Numéro saisi (espaces et tirets admis) -> chiffres seuls, s'il a la forme d'un numéro de carte : 12 à 19 chiffres et
/// clé de Luhn juste (une faute de frappe est refusée).
pub fn clean(number: &str) -> Option<String> {
    let d: String = number.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    if !(12..=19).contains(&d.len()) || !d.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let digit = |(i, b): (usize, u8)| {
        let n = (b - b'0') as u32;
        if i % 2 == 0 { n } else if n > 4 { n * 2 - 9 } else { n * 2 }
    };
    (d.bytes().rev().enumerate().map(digit).sum::<u32>() % 10 == 0).then_some(d)
}

/// « MM/AA » ou « MM/AAAA » (séparateur libre) -> (mois, année sur quatre chiffres).
pub fn expiry(text: &str) -> Option<(u8, u16)> {
    let mut parts = text.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty());
    let (m, y) = (parts.next()?.parse::<u8>().ok()?, parts.next()?);
    let year = match y.len() {
        2 => 2000 + y.parse::<u16>().ok()?,
        4 => y.parse().ok()?,
        _ => return None,
    };
    ((1..=12).contains(&m) && parts.next().is_none()).then_some((m, year))
}

/// Ce qu'on montre d'une carte : son réseau et ses quatre derniers chiffres, jamais le numéro.
pub fn label(c: &Card) -> String {
    let n = &c.number;
    let two = n.get(..2).and_then(|x| x.parse::<u32>().ok()).unwrap_or(0);
    let brand = if n.starts_with('4') {
        "Visa"
    } else if (51..=55).contains(&two) || (22..=27).contains(&two) {
        "Mastercard"
    } else if two == 34 || two == 37 {
        "American Express"
    } else {
        ""
    };
    format!("{brand} •••• {}", &n[n.len().saturating_sub(4)..]).trim().to_string()
}

// Nature d'un champ de carte : l'attribut `autocomplete` normalisé (cc-number, cc-exp…), sinon son nom, son identifiant
// et son libellé. Le cryptogramme est reconnu pour être écarté.
const KIND_JS: &str = r#"const KIND=e=>{if(!e||!/^(INPUT|SELECT)$/.test(e.tagName)||/^(hidden|password|checkbox|radio|submit|button)$/.test(e.type))return'';
const m=(e.getAttribute('autocomplete')||'').toLowerCase().match(/cc-(number|name|given-name|family-name|exp-month|exp-year|exp|csc)/);if(m)return m[1];
const s=[e.name,e.id,e.placeholder,e.getAttribute('aria-label')].join(' ').toLowerCase();
if(/cvc|cvv|csc|crypto|security.?code|code de s/.test(s))return'csc';
if(/card.?num|cardnum|cc.?num|num[eé]ro.{0,8}carte|numero.?carte/.test(s))return'number';
if(/exp.{0,12}(month|mois)|(month|mois).{0,12}exp|cc.?mm|expmonth/.test(s))return'exp-month';
if(/exp.{0,12}(year|ann)|(year|ann[eé]e).{0,12}exp|cc.?yy|expyear/.test(s))return'exp-year';
if(/exp|mm ?\/ ?(yy|aa)|validit/.test(s))return'exp';
if(/card.?holder|cardname|cc.?name|name.?on.?card|titulaire|nom sur la carte/.test(s))return'name';
return''};
const fields=()=>[...document.querySelectorAll('input,select')].map(e=>[e,KIND(e)]).filter(x=>x[1]&&x[1]!=='csc');"#;

// Dans chaque page et chaque cadre, avant tout script de la page. Signale un formulaire de carte (`cc:form` s'il a un
// champ de numéro, `cc:part` s'il n'a que d'autres champs de carte) : simple indication, pour afficher l'icône et
// savoir quels cadres interroger. Et retient l'instant du dernier geste RÉEL de l'utilisateur dans un champ de carte de
// ce document, que seul Volt relit par `__voltCC` : propriété ni modifiable ni remplaçable par la page, et seuls les
// événements du navigateur ont `isTrusted`. Aucune donnée ne sort de la page.
const WATCH_JS: &str = r#"const post=m=>{try{window.chrome.webview.postMessage(m)}catch(e){}},now=Date.now.bind(Date);let said='',last=0;
const scan=()=>{const f=fields(),k=f.some(x=>x[1]==='number')?'cc:form':f.length?'cc:part':'';if(k&&k!==said&&said!=='cc:form'){said=k;post(k)}};
addEventListener('DOMContentLoaded',scan,true);addEventListener('load',scan,true);addEventListener('focusin',scan,true);
const touch=e=>{if(e.isTrusted&&KIND(e.composedPath()[0])){last=now();scan();post('cc:touch')}};
addEventListener('pointerdown',touch,true);addEventListener('keydown',touch,true);
try{Object.defineProperty(window,'__voltCC',{value:Object.freeze(()=>last)})}catch(e){}"#;

// Instant (millisecondes Unix) du dernier geste réel dans un champ de carte du document ; 0 : aucun.
const PROBE: &str = "window.__voltCC?window.__voltCC():0";
// Un geste plus ancien ne désigne plus rien : l'utilisateur est passé à autre chose.
const FRESH_MS: f64 = 600_000.0;

// Remplit les champs de carte du document où il est lancé avec la carte `c` ; les événements préviennent la page comme
// le ferait une saisie (les formulaires React relisent la valeur par le « setter » d'origine).
const FILL_JS: &str = r#"const set=(e,v)=>{const d=Object.getOwnPropertyDescriptor(e.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype,'value');
e.focus();d&&d.set?d.set.call(e,v):e.value=v;for(const t of['input','change'])e.dispatchEvent(new Event(t,{bubbles:true}));e.blur()};
const pick=(e,a,b)=>{const o=[...e.options].find(o=>[o.value,o.text].some(t=>t.trim()!==''&&(+t.trim()===a||+t.trim()===b)));if(o)set(e,o.value)};
const mm=String(c.month).padStart(2,'0'),yy=String(c.year%100).padStart(2,'0'),w=c.name.trim().split(/\s+/);let n=0;
for(const [e,k] of fields()){if(e.disabled||e.readOnly)continue;const sel=e.tagName==='SELECT',ph=e.placeholder||'';
if(k==='number')set(e,c.number);else if(k==='name')set(e,c.name);else if(k==='given-name')set(e,w[0]||'');else if(k==='family-name')set(e,w.slice(1).join(' '));
else if(k==='exp')set(e,mm+(/\d\d \/ |mm \/ /i.test(ph)?' / ':'/')+(/yyyy|aaaa/i.test(ph)||e.maxLength===7?c.year:yy));
else if(k==='exp-month')sel?pick(e,c.month,c.month):set(e,mm);
else if(k==='exp-year')sel?pick(e,c.year,c.year%100):set(e,e.maxLength===2||/^(yy|aa)$/i.test(ph)?yy:String(c.year));
else continue;n++}"#;

/// Script posé dans chaque page et chaque cadre.
pub fn watch_js() -> String {
    format!("(()=>{{{KIND_JS}{WATCH_JS}}})()")
}

/// Script qui remplit la carte, à lancer seulement dans un document de l'origine choisie. Les valeurs sont insérées en
/// JSON (jamais par collage de chaîne), `<` échappé.
fn fill_js(c: &Card) -> String {
    let data = serde_json::to_string(c).unwrap_or_default().replace('<', "\\u003c");
    format!("(()=>{{{KIND_JS}const c={data};{FILL_JS}}})()")
}

thread_local! {
    // cadres qui ont annoncé des champs de carte : (onglet, cadre, origine du document qui l'a dit, donnée par le
    // moteur). Retirés dès que le cadre part vers une autre page ou disparaît : l'origine ne vaut que pour ce document.
    static FRAMES: RefCell<Vec<(u32, ICoreWebView2Frame2, String)>> = const { RefCell::new(vec![]) };
}

fn destroyed(f: &ICoreWebView2Frame2) -> bool {
    let mut d = BOOL(1);
    unsafe { f.IsDestroyed(&mut d) }.is_err() || d.as_bool()
}

fn forget(f: &ICoreWebView2Frame2) {
    FRAMES.with_borrow_mut(|l| l.retain(|x| x.1 != *f && !destroyed(&x.1)));
}

/// Suit les cadres de la page de l'onglet `tab`, et leurs propres cadres : `on(message)` pour chaque `cc:form`,
/// `cc:part` ou `cc:touch` qu'un cadre envoie.
pub fn watch_frames(wv: &WebView, tab: u32, on: impl Fn(&str) + Clone + 'static) {
    let Ok(w) = wv.webview().cast::<ICoreWebView2_4>() else { return };
    let h = FrameCreatedEventHandler::create(Box::new(move |_, args| {
        if let Some(f) = args.and_then(|a| unsafe { a.Frame() }.ok()) {
            hook(f, tab, on.clone());
        }
        Ok(())
    }));
    let mut token = 0i64;
    let _ = unsafe { w.add_FrameCreated(&h, &mut token) };
}

fn hook(frame: ICoreWebView2Frame, tab: u32, on: impl Fn(&str) + Clone + 'static) {
    let Ok(f2) = frame.cast::<ICoreWebView2Frame2>() else { return };
    let mut token = 0i64;
    unsafe {
        let (me, on2) = (f2.clone(), on.clone());
        let _ = f2.add_WebMessageReceived(
            &FrameWebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                let Some(a) = args else { return Ok(()) };
                let (mut msg, mut src) = (PWSTR::null(), PWSTR::null());
                if a.TryGetWebMessageAsString(&mut msg).is_err() || a.Source(&mut src).is_err() {
                    return Ok(());
                }
                let (msg, origin) = (take_pwstr(msg), crate::vault::origin_of(&take_pwstr(src)));
                if let (true, Some(o)) = (matches!(msg.as_str(), "cc:form" | "cc:part" | "cc:touch"), origin) {
                    FRAMES.with_borrow_mut(|l| {
                        l.retain(|x| x.1 != me && !destroyed(&x.1));
                        l.push((tab, me.clone(), o));
                    });
                    on2(&msg);
                }
                Ok(())
            })),
            &mut token,
        );
        // le cadre part vers une autre page : ce qu'il avait annoncé ne vaut plus
        let me = f2.clone();
        let _ = f2.add_NavigationStarting(
            &FrameNavigationStartingEventHandler::create(Box::new(move |_, _| {
                forget(&me);
                Ok(())
            })),
            &mut token,
        );
        let me = f2.clone();
        let _ = frame.add_Destroyed(
            &FrameDestroyedEventHandler::create(Box::new(move |_, _| {
                forget(&me);
                Ok(())
            })),
            &mut token,
        );
        // cadres dans ce cadre (certains prestataires en emboîtent deux)
        if let Ok(f7) = frame.cast::<ICoreWebView2Frame7>() {
            let _ = f7.add_FrameCreated(
                &FrameChildFrameCreatedEventHandler::create(Box::new(move |_, args| {
                    if let Some(f) = args.and_then(|a| a.Frame().ok()) {
                        hook(f, tab, on.clone());
                    }
                    Ok(())
                })),
                &mut token,
            );
        }
    }
}

/// Document où un script peut être lancé : la page de l'onglet, ou un de ses cadres.
enum Doc {
    Main(ICoreWebView2),
    Frame(ICoreWebView2Frame2),
}

/// Lance `js` dans ce document ; `done(résultat en JSON)`, ou `done("")` si le document n'existe plus.
fn exec(d: &Doc, js: &str, done: impl Fn(String) + 'static) {
    let done = Rc::new(done);
    let d2 = done.clone();
    let h = ExecuteScriptCompletedHandler::create(Box::new(move |_, json| {
        d2(json);
        Ok(())
    }));
    let js = HSTRING::from(js);
    let r = unsafe {
        match d {
            Doc::Main(w) => w.ExecuteScript(&js, &h),
            Doc::Frame(f) => f.ExecuteScript(&js, &h),
        }
    };
    if r.is_err() {
        done(String::new());
    }
}

/// Documents de l'onglet et leur origine : la page (`main` : sa vue et son origine actuelle) puis ses cadres connus.
fn docs(tab: u32, main: Option<(ICoreWebView2, String)>) -> Vec<(Doc, String)> {
    let mut out: Vec<(Doc, String)> = main.into_iter().map(|(w, o)| (Doc::Main(w), o)).collect();
    FRAMES.with_borrow_mut(|l| {
        l.retain(|x| !destroyed(&x.1));
        out.extend(l.iter().filter(|x| x.0 == tab).map(|x| (Doc::Frame(x.1.clone()), x.2.clone())));
    });
    out
}

/// Origine du document au geste réel le plus récent (`last` : instant relevé dans chaque document), s'il date de moins
/// de dix minutes.
fn choose(last: &[f64], origins: &[String], now: f64) -> Option<String> {
    let best = last.iter().zip(origins).filter(|(t, _)| **t > now - FRESH_MS && **t <= now + 60_000.0).max_by(|a, b| a.0.total_cmp(b.0))?;
    Some(best.1.clone())
}

/// Vers quelle origine remplir la carte dans l'onglet `tab` ? `done(Some(origine))` : celle du document où l'utilisateur
/// a cliqué ou tapé lui-même dans un champ de carte en dernier ; à défaut, celle de la page si elle porte elle-même le
/// champ du numéro (`main_form`) ; sinon None. Rien n'est envoyé aux documents : on y lit seulement un instant.
pub fn target(tab: u32, main: Option<(ICoreWebView2, String)>, main_form: bool, done: impl Fn(Option<String>) + 'static) {
    let fallback = main.as_ref().filter(|_| main_form).map(|m| m.1.clone());
    let docs = docs(tab, main);
    if docs.is_empty() {
        return done(fallback);
    }
    let origins: Rc<Vec<String>> = Rc::new(docs.iter().map(|d| d.1.clone()).collect());
    let state = Rc::new(RefCell::new((vec![0f64; docs.len()], docs.len())));
    let done = Rc::new(done);
    for (i, (d, _)) in docs.iter().enumerate() {
        let (state, origins, fallback, done) = (state.clone(), origins.clone(), fallback.clone(), done.clone());
        exec(d, PROBE, move |json| {
            let mut s = state.borrow_mut();
            s.0[i] = json.trim().parse().unwrap_or(0.0);
            s.1 = s.1.saturating_sub(1);
            if s.1 == 0 {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64);
                done(choose(&s.0, &origins, now).or(fallback.clone()));
            }
        });
    }
}

/// Remplit la carte dans l'onglet `tab`, dans les seuls documents de l'origine `target` (la page si c'est la sienne,
/// et les cadres de cette origine). Renvoie le nombre de documents servis.
pub fn fill(tab: u32, target: &str, card: &Card, main: Option<(ICoreWebView2, String)>) -> usize {
    let js = fill_js(card);
    let to: Vec<_> = docs(tab, main).into_iter().filter(|d| d.1 == target).collect();
    for (d, _) in &to {
        exec(d, &js, |_| {});
    }
    to.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_dates() {
        // numéros d'essai publics des réseaux de cartes (aucune vraie carte)
        assert_eq!(clean("4242 4242 4242 4242").as_deref(), Some("4242424242424242"));
        assert_eq!(clean("5555-5555-5555-4444").as_deref(), Some("5555555555554444"));
        assert_eq!(clean("378282246310005").as_deref(), Some("378282246310005"));
        assert_eq!(clean("4242 4242 4242 4241"), None); // un chiffre faux
        assert_eq!(clean("4242"), None);
        assert_eq!(clean("4242 4242 4242 424x"), None);
        assert_eq!(clean(""), None);
        assert_eq!(expiry("08/27"), Some((8, 2027)));
        assert_eq!(expiry("8 / 2031"), Some((8, 2031)));
        assert_eq!(expiry("12-29"), Some((12, 2029)));
        for bad in ["13/27", "0/27", "08/7", "08", "", "08/27/01", "aa/bb"] {
            assert_eq!(expiry(bad), None, "{bad}");
        }
        let card = |n: &str| Card { name: "A B".into(), number: n.into(), month: 8, year: 2027 };
        assert_eq!(label(&card("4242424242424242")), "Visa •••• 4242");
        assert_eq!(label(&card("5555555555554444")), "Mastercard •••• 4444");
        assert_eq!(label(&card("2223003122003222")), "Mastercard •••• 3222");
        assert_eq!(label(&card("378282246310005")), "American Express •••• 0005");
        assert_eq!(label(&card("6011111111111117")), "•••• 1117"); // réseau non reconnu : les chiffres seuls
        assert!(!label(&card("4242424242424242")).contains("42424242"));
    }

    #[test]
    fn encrypted_file_roundtrip() {
        let p = std::env::temp_dir().join(format!("volt cartes {}.dat", std::process::id()));
        let cards = vec![Card { name: "Zoé <Test>".into(), number: "4242424242424242".into(), month: 1, year: 2030 }];
        assert!(save(&p, &cards));
        let raw = std::fs::read(&p).unwrap();
        // rien de lisible sur le disque
        assert!(!raw.windows(8).any(|w| w == b"42424242") && !String::from_utf8_lossy(&raw).contains("Zo"));
        assert_eq!(load(&p), cards);
        std::fs::write(&p, b"pas un coffre").unwrap();
        assert!(load(&p).is_empty()); // fichier abîmé : liste vide, pas d'arrêt
        std::fs::remove_file(&p).unwrap();
        assert!(load(&p).is_empty());
    }

    #[test]
    fn only_the_touched_origin() {
        let o = ["https://boutique.fr".to_string(), "https://pub.example".to_string(), "https://paiement.example".to_string()];
        let now = 1_800_000_000_000.0;
        // le geste réel le plus récent désigne l'origine ; un cadre sans geste (0) n'est jamais choisi
        assert_eq!(choose(&[0.0, 0.0, now - 4_000.0], &o, now).as_deref(), Some("https://paiement.example"));
        assert_eq!(choose(&[now - 9_000.0, 0.0, now - 4_000.0], &o, now).as_deref(), Some("https://paiement.example"));
        assert_eq!(choose(&[now - 1_000.0, 0.0, now - 4_000.0], &o, now).as_deref(), Some("https://boutique.fr"));
        // aucun geste, geste trop ancien, ou instant dans le futur (horloge incohérente) : personne
        assert_eq!(choose(&[0.0, 0.0, 0.0], &o, now), None);
        assert_eq!(choose(&[0.0, now - 700_000.0, 0.0], &o, now), None);
        assert_eq!(choose(&[0.0, now + 3_600_000.0, 0.0], &o, now), None);
        assert_eq!(choose(&[], &[], now), None);
    }

    #[test]
    fn scripts() {
        // la carte entre dans le script en JSON : un nom hostile ne peut ni fermer la chaîne ni ouvrir une balise
        let js = fill_js(&Card { name: "\"});alert(1)//</script>".into(), number: "4242424242424242".into(), month: 3, year: 2028 });
        assert!(js.contains(r#""name":"\"});alert(1)//\u003c/script>""#) && !js.contains("</script>"));
        assert!(js.starts_with("(()=>{const KIND=") && js.ends_with("})()") && js.contains(r#""month":3,"year":2028"#));
        // le script de veille ne lit aucune valeur de champ, et n'expose à la page qu'une fonction figée
        let w = watch_js();
        assert!(w.contains("isTrusted") && w.contains("Object.freeze") && !w.contains(".value"));
    }
}
