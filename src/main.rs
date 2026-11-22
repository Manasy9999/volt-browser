#![cfg_attr(not(test), windows_subsystem = "windows")]
mod audio;
mod cmp;
mod cookies;
mod defaults;
mod ext;
mod import;
mod perf;
mod preview;
mod sort;
mod tabmenu;
mod translate;
mod update;

use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use webview2_com::{Microsoft::Web::WebView2::Win32::ICoreWebView2Settings4, SourceChangedEventHandler, take_pwstr};
use windows::core::{BOOL, Interface, PWSTR};
use tao::{
    event::{ElementState, Event, MouseButton, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    platform::windows::{WindowBuilderExtWindows, WindowExtWindows},
    window::{CursorIcon, Fullscreen, Icon, ResizeDirection, Theme as WinTheme, Window, WindowBuilder},
};
use wry::{
    MemoryUsageLevel, NewWindowResponse, PageLoadEvent, Rect, Theme, WebContext, WebView, WebViewBuilder,
    WebViewBuilderExtWindows, WebViewExtWindows,
    dpi::{LogicalPosition, LogicalSize},
};

const TOP: f64 = 84.0; // hauteur barre d'onglets + barre d'adresse
const FAVBAR_H: u32 = 30; // une rangée, en attendant la vraie hauteur envoyée par l'interface
// Hauteur de la barre de favoris (px logiques, 0 = masquée) : l'interface l'envoie (« favh »), la mise en page la lit.
static FAVH: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

// Une page est en plein écran (vidéo) : la page prend toute la fenêtre.
static FULL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn full() -> bool {
    FULL.load(std::sync::atomic::Ordering::Relaxed)
}

// Tiroir Historique et cookies : ouvert (l'historique lui est envoyé) et largeur affichée, que l'interface anime
// (« drawerw ») ; la page est décalée d'autant, l'interface dessous occupe alors toute la fenêtre.
static DRAWER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static DRAWER_W: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn drawer_w() -> f64 {
    if full() { 0.0 } else { DRAWER_W.load(std::sync::atomic::Ordering::Relaxed) as f64 }
}

fn top() -> f64 {
    if full() {
        return 0.0;
    }
    TOP + FAVH.load(std::sync::atomic::Ordering::Relaxed) as f64
}
// Fenêtre sans barre de titre : bordure de redimensionnement gérée par la fenêtre principale.
const BORDER: f64 = 4.0;
const UI: &str = include_str!("ui.html");
// Raccourcis captés dans les pages web ; seule cette liste est acceptée depuis un site.
const KEYS: [&str; 8] = ["ct", "cw", "cl", "ctab", "cstab", "cd", "ch", "cst"];
const KEY_JS: &str = r#"addEventListener('keydown',e=>{const k=(e.ctrlKey?'c':'')+(e.shiftKey?'s':'')+e.key.toLowerCase();if(['ct','cw','cl','ctab','cstab','cd','ch','cst'].includes(k)){e.preventDefault();window.ipc.postMessage(k)}},true)"#;
// Mémorise les champs où l'utilisateur a tapé (jamais leur contenu) ; composedPath : champs dans un shadow DOM.
const TYPED_JS: &str = r#"addEventListener('input',e=>{const t=e.composedPath()[0];if(t.tagName=='TEXTAREA'||t.isContentEditable||(t.tagName=='INPUT'&&/^(text|search|email|url|tel|number|password)$/.test(t.type)))(window.__voltTyped||=new Set).add(t)},true)"#;
// État de l'onglet quitté : [défilement, temps de la plus grande vidéo, texte non envoyé]. Un seul champ
// input rempli = barre de recherche, ignorée ; seul le booléen sort de la page.
const STATE_JS: &str = r#"(()=>{const v=[...document.querySelectorAll('video')].sort((a,b)=>b.offsetWidth*b.offsetHeight-a.offsetWidth*a.offsetHeight)[0];let n=0,d=false;for(const e of window.__voltTyped||[]){if(!e.isConnected)continue;if(e.tagName=='INPUT'){if(e.value)n++}else if(e.tagName=='TEXTAREA'?e.value:e.textContent.trim())d=true}return [scrollY,v?v.currentTime:0,d||n>=2]})()"#;
// Restauration : les pages construites après coup (YouTube) n'ont ni la vidéo ni la hauteur tout de suite,
// on réessaie 5 s au plus.
const RESTORE_JS: &str = r#"((y,t)=>{let n=0;const f=()=>{if(y){scrollTo(0,y);if(Math.abs(scrollY-y)<2)y=0}const v=[...document.querySelectorAll('video')].sort((a,b)=>b.offsetWidth*b.offsetHeight-a.offsetWidth*a.offsetHeight)[0];if(t&&v){if(Math.abs(v.currentTime-t)<3)t=0;else v.currentTime=t}if((y||t)&&++n<20)setTimeout(f,250)};f()})"#;
const HIST_MAX: usize = 1000;
const EXT_WELCOME: Duration = Duration::from_secs(15);
// uBlock Origin Lite : filtrage déclaratif fait par le moteur (pas de JS par requête).
const UBOL: &str = "ddkjiahejlhfcafbddmgiahcphecmpfh";
// Fond affiché pendant les chargements (plus de flash blanc).
const LOADING_BG: (u8, u8, u8, u8) = (46, 46, 51, 255);
const TICK: Duration = Duration::from_secs(30);
// Onglet caché depuis 2 min (30 s en Mode Jeu) : gelé (processus en pause).
const FREEZE_AFTER: Duration = Duration::from_secs(120);
const FREEZE_GAME: Duration = Duration::from_secs(30);
// Fenêtre réduite depuis 20 s (on joue) : tout est gelé, sauf l'audio.
const FREEZE_MINIMIZED: Duration = Duration::from_secs(20);
#[derive(Serialize, Deserialize, Clone)]
struct Page {
    url: String,
    title: String,
    // favoris : icône (emoji ou image data:) et dossier ; vides pour l'historique
    #[serde(default, skip_serializing_if = "String::is_empty")]
    icon: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    folder: String,
}

// Entrée d'historique ; `at` (secondes Unix) absent des fichiers d'avant l'horodatage : 0, groupe « Plus ancien ».
#[derive(Serialize, Deserialize, Clone)]
struct Visit {
    url: String,
    title: String,
    #[serde(default)]
    at: u64,
}

#[derive(Serialize, Deserialize, Clone)]
struct Folder {
    id: String,
    name: String,
    color: String,
    icon: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct Settings {
    home: String,
    search: String,
    accent: String,
    theme: String,
    startup: String, // session au démarrage : "proposer" (défaut), "auto" ou "neuf"
    #[serde(skip_serializing)]
    restore: Option<bool>, // ancien booléen, remplacé par `startup` (voir `migrate`)
    game_mode: bool,
    dark_web: bool,
    night: bool,
    adblock: bool,
    sleep_min: u32, // 0 = jamais décharger
    keep_loaded: u32, // onglets chargés au plus (~108 Mo chacun) ; 0 = illimité
    volty_anim: bool, // lu par l'interface seulement
    session: Vec<String>,
    pinned: u32, // les `pinned` premiers onglets de la session sont épinglés (toujours en tête)
    favs: Vec<Page>,
    folders: Vec<Folder>,
    favbar: bool,
    favbar_style: String, // "etageres" (défaut), "ruban" ou "icones" ; lu par l'interface seulement
    save_passwords: bool, // le moteur propose d'enregistrer les identifiants à la connexion
    exts_open: bool, // icônes des extensions dépliées dans la barre d'adresse ; lu par l'interface seulement
    translate_auto: bool, // page détectée en langue étrangère : rechargée traduite par le relais de Google
    no_translate: Vec<String>, // sites jamais traduits automatiquement (« Voir l'original » les ajoute)
    cmp_auto: bool, // réponse automatique aux bandeaux de cookies (cmp.rs)
    cmp_mode: String, // "refuser" (défaut) ou "accepter"
    cmp_skip: Vec<String>, // sites exclus de la réponse automatique
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            home: "https://www.google.com".into(),
            search: "google".into(),
            accent: "#9b5cff".into(),
            theme: "volt".into(),
            startup: "proposer".into(),
            restore: None,
            game_mode: true,
            dark_web: true,
            night: true,
            adblock: true,
            sleep_min: 20,
            keep_loaded: 7,
            volty_anim: false,
            session: vec![],
            pinned: 0,
            favs: vec![],
            folders: vec![],
            favbar: true,
            favbar_style: "etageres".into(),
            save_passwords: true,
            exts_open: true,
            translate_auto: false,
            no_translate: vec![],
            cmp_auto: false,
            cmp_mode: "refuser".into(),
            cmp_skip: vec![],
        }
    }
}

enum Ev {
    Ui(String),
    Key(String),
    Title(u32, String),
    Load(u32, String, bool),
    Url(u32, String), // adresse changée sans rechargement
    Open(String),
    Exts(Value),
    ExtChanged,
    ExtFolder(PathBuf),
    Toast(String),
    Cpu(u8),
    Audio(u32, bool),
    Fullscreen(bool),
    State(u32, String),
    Raise, // un second Volt a été lancé : premier plan
    Cookies(Value),
    Lang(u32, String, String), // onglet, adresse, réponse de LANG_JS
    Update(Value), // état de la mise à jour pour l'interface (`upd`)
}

struct Tab {
    id: u32,
    wv: Option<WebView>, // None = onglet restauré pas encore chargé
    title: String,
    url: String,
    loading: bool,
    audio: bool,
    muted: bool,
    seen: Instant, // dernière fois actif
    frozen: bool,
    typed: bool, // texte non envoyé : jamais déchargé par le budget
    saved: Option<(String, f64, f64)>, // (url, défilement, temps vidéo) relevés en quittant l'onglet
    pinned: bool, // épinglé : en tête, jamais déchargé
    cmp: String, // identifiant du script des bandeaux de cookies dans ce webview ("" = aucun)
    foreign: bool, // page détectée en langue étrangère
}

/// Encodage pour un paramètre d'adresse (espace -> « + »).
fn enc(q: &str) -> String {
    q.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn search_url(engine: &str, q: &str) -> String {
    let base = match engine {
        "duckduckgo" => "https://duckduckgo.com/?q=",
        "bing" => "https://www.bing.com/search?q=",
        "brave" => "https://search.brave.com/search?q=",
        _ => "https://www.google.com/search?q=",
    };
    base.to_string() + &enc(q)
}

/// Script des bandeaux de cookies selon les réglages ("" : réponse automatique coupée).
fn cmp_js(s: &Settings) -> String {
    if s.cmp_auto { cmp::script(s.cmp_mode == "accepter", &s.cmp_skip) } else { String::new() }
}

/// Réglages des bandeaux changés : script remplacé dans chaque onglet chargé, pour les pages chargées ensuite.
fn set_cmp(tabs: &mut [Tab], js: &str) {
    for t in tabs {
        if let Some(w) = &t.wv {
            t.cmp = cmp::swap(w, &t.cmp, js);
        }
    }
}

/// Texte de la barre d'adresse -> URL (adresse directe ou recherche).
fn to_url(input: &str, engine: &str) -> String {
    let t = input.trim();
    if t.contains("://") || t.starts_with("about:") {
        t.into()
    } else if !t.contains(' ') && (t.contains('.') || t.starts_with("localhost")) {
        format!("https://{t}")
    } else {
        search_url(engine, t)
    }
}

/// Pages internes `volt://…` (chrome:// et edge:// y sont redirigés) -> section du panneau.
fn internal(input: &str) -> Option<&'static str> {
    let t = input.trim().to_ascii_lowercase();
    let page = ["volt://", "chrome://", "edge://"].iter().find_map(|p| t.strip_prefix(p))?;
    Some(match page.trim_end_matches('/') {
        "history" => "hist",
        "favorites" | "bookmarks" => "favs",
        "extensions" | "plugins" | "store" => "exts",
        "settings/general" | "settings/search" | "search" | "settings/startup" => "start",
        "settings/appearance" | "appearance" => "look",
        "settings/game" | "game" => "gamer",
        "settings/privacy" | "privacy" | "settings/clearbrowserdata" => "priv",
        "settings/keys" | "shortcuts" => "keys",
        "newtab" => "board",
        "welcome" => "hello",
        _ => "home",
    })
}

/// Une visite par page vue, en tête, avec son heure ; une adresse revue remonte au lieu d'être doublée.
fn add_visit(hist: &mut Vec<Visit>, url: &str, title: &str) {
    hist.retain(|h| h.url != url);
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    hist.insert(0, Visit { url: url.into(), title: title.into(), at });
    hist.truncate(HIST_MAX);
}

fn toggle_fav(s: &mut Settings, t: &Tab) {
    if let Some(i) = s.favs.iter().position(|f| f.url == t.url) {
        s.favs.remove(i);
    } else {
        s.favs.push(Page { url: t.url.clone(), title: t.title.clone(), icon: String::new(), folder: String::new() });
    }
}

/// Lit un JSON (BOM accepté). Fichier illisible : mis de côté en `.bak` au lieu d'être écrasé.
fn load<T: serde::de::DeserializeOwned + Default>(path: &std::path::Path) -> T {
    let Ok(b) = fs::read(path) else { return T::default() };
    parse_json(&b).unwrap_or_else(|| {
        let _ = fs::rename(path, path.with_extension("json.bak"));
        T::default()
    })
}

fn parse_json<T: serde::de::DeserializeOwned>(b: &[u8]) -> Option<T> {
    serde_json::from_slice(b.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(b)).ok()
}

fn data_dir() -> PathBuf {
    // VOLT_DATA : profil isolé (mesures A/B sans toucher aux données de l'utilisateur).
    if let Some(d) = std::env::var_os("VOLT_DATA") {
        return d.into();
    }
    PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into())).join("Volt")
}

/// Captures des onglets (pages potentiellement privées) : jamais conservées après une sortie normale.
fn previews() -> PathBuf {
    data_dir().join("previews")
}

fn preview_of(id: u32) -> PathBuf {
    previews().join(format!("{id}.jpg"))
}

fn border(win: &Window) -> f64 {
    if win.is_maximized() || full() { 0.0 } else { BORDER }
}

fn content_rect(win: &Window) -> Rect {
    let (s, b) = (win.inner_size().to_logical::<f64>(win.scale_factor()), border(win));
    let l = drawer_w();
    Rect { position: LogicalPosition::new(b + l, top()).into(), size: LogicalSize::new((s.width - 2.0 * b - l).max(0.0), (s.height - top() - b).max(0.0)).into() }
}

fn ui_rect(win: &Window, panel: bool) -> Rect {
    let (s, b) = (win.inner_size().to_logical::<f64>(win.scale_factor()), border(win));
    let h = if panel || drawer_w() > 0.0 { s.height - b } else { top() };
    Rect { position: LogicalPosition::new(b, 0.0).into(), size: LogicalSize::new((s.width - 2.0 * b).max(0.0), h.max(0.0)).into() }
}

/// Bord de fenêtre sous la souris (coordonnées physiques), pour redimensionner sans barre de titre.
fn edge(win: &Window, x: f64, y: f64) -> Option<ResizeDirection> {
    if win.is_maximized() {
        return None;
    }
    let s = win.inner_size();
    let m = BORDER * win.scale_factor();
    let (l, r, top, bot) = (x < m, x > s.width as f64 - m, y < m, y > s.height as f64 - m);
    Some(match (l, r, top, bot) {
        (true, _, true, _) => ResizeDirection::NorthWest,
        (_, true, true, _) => ResizeDirection::NorthEast,
        (true, _, _, true) => ResizeDirection::SouthWest,
        (_, true, _, true) => ResizeDirection::SouthEast,
        (true, ..) => ResizeDirection::West,
        (_, true, ..) => ResizeDirection::East,
        (_, _, true, _) => ResizeDirection::North,
        (.., true) => ResizeDirection::South,
        _ => return None,
    })
}

fn cursor_for(d: Option<ResizeDirection>) -> CursorIcon {
    match d {
        Some(ResizeDirection::West | ResizeDirection::East) => CursorIcon::EwResize,
        Some(ResizeDirection::North | ResizeDirection::South) => CursorIcon::NsResize,
        Some(ResizeDirection::NorthWest | ResizeDirection::SouthEast) => CursorIcon::NwseResize,
        Some(_) => CursorIcon::NeswResize,
        None => CursorIcon::Default,
    }
}

/// Télécharge une extension du Chrome Web Store hors du thread UI.
fn install_ext(p: EventLoopProxy<Ev>, id: String, d: PathBuf) {
    std::thread::spawn(move || {
        match ext::download(&id, &d) {
            Ok(f) => {
                let _ = p.send_event(Ev::ExtFolder(f));
            }
            Err(e) => {
                let _ = p.send_event(Ev::Toast(format!("Téléchargement impossible : {e}")));
                let _ = p.send_event(Ev::ExtChanged); // le bouton « Installation… » de l'interface revient
            }
        }
    });
}

/// Recherche (ou installation) de mise à jour hors du thread UI ; installée, Volt redémarre par « restart ».
fn run_update(p: EventLoopProxy<Ev>, install: bool) {
    if !update::begin() {
        return;
    }
    let _ = p.send_event(Ev::Update(json!({"st": "busy", "install": install})));
    std::thread::spawn(move || {
        let r = if install { update::install().map(Some) } else { update::check() };
        update::end();
        let _ = p.send_event(Ev::Update(match &r {
            Ok(None) => json!({"st": "ok"}),
            Ok(Some(v)) if install => json!({"st": "done", "v": v}),
            Ok(Some(v)) => json!({"st": "new", "v": v}),
            Err(e) => json!({"st": "err", "msg": e}),
        }));
        if install && r.is_ok() {
            let _ = p.send_event(Ev::Ui(json!({"cmd": "restart"}).to_string()));
        }
    });
}

fn save(s: &Settings, cfg: &std::path::Path) {
    let _ = fs::write(cfg, serde_json::to_vec_pretty(s).unwrap());
}

/// Ancien `restore` (booléen) : faux devient « neuf » ; vrai ou absent, le défaut « proposer ».
fn migrate(s: &mut Settings) {
    if s.restore.take() == Some(false) {
        s.startup = "neuf".into();
    }
}

#[derive(Debug, PartialEq)]
enum Boot {
    Restore,
    Offer, // bande « Reprendre la navigation »
    Fresh,
}

/// Démarrage selon le mode, `--restart` (redémarrage, mise à jour) et le nombre d'onglets enregistrés.
fn boot(mode: &str, restart: bool, saved: usize) -> Boot {
    match saved {
        0 => Boot::Fresh,
        _ if restart || mode == "auto" => Boot::Restore,
        2.. if mode == "proposer" => Boot::Offer,
        _ => Boot::Fresh,
    }
}

#[derive(Debug, PartialEq)]
enum Close {
    Save,
    KeepOld, // reprise encore proposée, rien fait : l'ancienne session n'est pas écrasée
    Forget,
}

fn on_close(mode: &str, restart: bool, offered: bool, untouched: bool) -> Close {
    if mode == "neuf" && !restart {
        Close::Forget
    } else if offered && untouched {
        Close::KeepOld
    } else {
        Close::Save
    }
}

/// Même page, à la requête, à l'ancre et au « / » final près.
fn same_page(a: &str, b: &str) -> bool {
    let p = |u: &str| u.split(['?', '#']).next().unwrap_or_default().trim_end_matches('/').to_ascii_lowercase();
    p(a) == p(b)
}

/// Pas navigué : au plus un onglet, resté sur la page d'accueil.
fn untouched(tabs: &[Tab], home: &str) -> bool {
    tabs.len() <= 1 && tabs.iter().all(|t| same_page(&t.url, home))
}

// Session proposée : (adresses, épinglés en tête, fin de la reprise une fois la bande fermée).
type Pending = Option<(Vec<String>, u32, Option<Instant>)>;
const OFFER_FOR: Duration = Duration::from_secs(180);

/// Onglets encore proposés (0 : rien, ou les 3 minutes de l'icône écoulées).
fn offer_n(o: &Pending) -> usize {
    o.as_ref().filter(|o| o.2.is_none_or(|d| Instant::now() < d)).map_or(0, |o| o.0.len())
}

/// Sauvegarde réglages, session et historique (fermeture de Volt).
fn persist(s: &mut Settings, tabs: &[Tab], hist: &[Visit], cfg: &std::path::Path, hp: &std::path::Path, restart: bool, offered: bool) {
    match on_close(&s.startup, restart, offered, untouched(tabs, &s.home)) {
        Close::Save => {
            s.session = tabs.iter().map(|t| t.url.clone()).collect();
            s.pinned = tabs.iter().filter(|t| t.pinned).count() as u32;
        }
        Close::Forget => (s.session, s.pinned) = (vec![], 0),
        Close::KeepOld => {}
    }
    save(s, cfg);
    let _ = fs::write(hp, serde_json::to_vec(hist).unwrap());
    let _ = fs::remove_dir_all(previews());
}

/// Ferme l'onglet `id` ; renvoie le nouvel onglet actif, ou None s'il n'en reste plus.
fn close_tab(tabs: &mut Vec<Tab>, active: u32, id: u32) -> Option<u32> {
    let i = tabs.iter().position(|t| t.id == id)?;
    tabs.remove(i);
    if id != active {
        return Some(active);
    }
    tabs.get(i.min(tabs.len().saturating_sub(1))).map(|t| t.id)
}

/// Seul chemin de fermeture : pile « Rouvrir », capture d'aperçu supprimée ; None s'il ne reste aucun onglet.
fn close(tabs: &mut Vec<Tab>, mut active: u32, ids: &[u32], closed: &mut Vec<(usize, String)>) -> Option<u32> {
    for &id in ids {
        let Some(i) = tabs.iter().position(|t| t.id == id) else { continue };
        tabmenu::remember(closed, i, &tabs[i].url);
        let _ = fs::remove_file(preview_of(id));
        let _ = fs::remove_file(preview_of(id).with_extension("bg"));
        active = close_tab(tabs, active, id)?;
    }
    Some(active)
}

fn toggle_mute(t: &mut Tab) {
    t.muted = !t.muted;
    if let Some(w) = &t.wv {
        audio::set_muted(w, t.muted);
    }
}

/// Le moteur propose d'enregistrer les identifiants à la connexion ; rien n'est enregistré sans le clic de l'utilisateur.
fn set_password_save(wv: &WebView, on: bool) {
    if let Ok(st) = unsafe { wv.webview().Settings() }.and_then(|s| s.cast::<ICoreWebView2Settings4>()) {
        let _ = unsafe { st.SetIsPasswordAutosaveEnabled(on) };
    }
}

/// Adresse changée sans rechargement (YouTube d'une vidéo à l'autre, fiches du Chrome Web Store) : le moteur ne
/// signale alors aucun chargement de page. Les nouveaux documents passent par le chargement (Ev::Load).
fn watch_url(wv: &WebView, f: impl Fn(String) + 'static) {
    let h = SourceChangedEventHandler::create(Box::new(move |sender, args| {
        let mut new = BOOL(1);
        if let Some(a) = args {
            unsafe { a.IsNewDocument(&mut new)? };
        }
        if let (false, Some(s)) = (new.as_bool(), sender) {
            let mut p = PWSTR::null();
            unsafe { s.Source(&mut p)? };
            f(take_pwstr(p));
        }
        Ok(())
    }));
    let mut token = 0i64;
    let _ = unsafe { wv.webview().add_SourceChanged(&h, &mut token) };
}

/// Onglets à décharger pour revenir à `keep` chargés (0 = illimité), les moins récemment vus d'abord.
/// `loaded` : (id, dernière fois vu, protégé) des onglets chargés ; les protégés comptent mais restent.
fn over_budget(loaded: &[(u32, Instant, bool)], keep: u32) -> Vec<u32> {
    let extra = if keep == 0 { 0 } else { loaded.len().saturating_sub(keep as usize) };
    let mut free: Vec<_> = loaded.iter().filter(|t| !t.2).collect();
    free.sort_by_key(|t| t.1);
    free.iter().take(extra).map(|t| t.0).collect()
}

/// Applique le budget ; renvoie true si un onglet a été déchargé.
fn unload_over_budget(tabs: &mut [Tab], active: u32, keep: u32) -> bool {
    let view: Vec<_> = tabs.iter().filter(|t| t.wv.is_some()).map(|t| (t.id, t.seen, t.id == active || t.audio || t.loading || t.typed || t.pinned)).collect();
    let out = over_budget(&view, keep);
    for t in tabs.iter_mut().filter(|t| out.contains(&t.id)) {
        t.wv = None;
        t.frozen = false;
    }
    !out.is_empty()
}

/// `html` : page d'aperçu ou d'attente à afficher d'abord (elle part elle-même vers `url`) et fond du moteur derrière ;
/// `cmp` : script des bandeaux de cookies ("" = aucun). Renvoie le webview et l'identifiant de ce script.
fn make_tab(win: &Window, ctx: &mut WebContext, proxy: &EventLoopProxy<Ev>, id: u32, url: &str, html: Option<(String, (u8, u8, u8, u8))>, dark: bool, pw: bool, args: &str, cmp: &str) -> (WebView, String) {
    let (p1, p2, p3, p4, p5) = (proxy.clone(), proxy.clone(), proxy.clone(), proxy.clone(), proxy.clone());
    // retour avec aperçu : fond du moteur à la couleur de la page, le passage de l'aperçu à la page ne montre plus de gris
    let bg = html.as_ref().map_or(LOADING_BG, |h| h.1);
    // rien à charger ici : la navigation part après l'ajout du script des bandeaux, pour qu'il vaille dès la première page
    let wv = WebViewBuilder::new_with_web_context(ctx)
        .with_additional_browser_args(args)
        .with_background_color(bg)
        .with_bounds(content_rect(win))
        .with_browser_extensions_enabled(true)
        .with_devtools(true)
        .with_theme(if dark { Theme::Dark } else { Theme::Auto })
        .with_initialization_script(KEY_JS)
        .with_initialization_script(TYPED_JS)
        .with_document_title_changed_handler(move |t| {
            let _ = p1.send_event(Ev::Title(id, t));
        })
        .with_on_page_load_handler(move |e, url| {
            let _ = p2.send_event(Ev::Load(id, url, matches!(e, PageLoadEvent::Started)));
        })
        .with_new_window_req_handler(move |url, _| {
            let _ = p3.send_event(Ev::Open(url));
            NewWindowResponse::Deny
        })
        .with_ipc_handler(move |r| {
            if KEYS.contains(&r.body().as_str()) {
                let _ = p4.send_event(Ev::Key(r.body().clone()));
            }
        })
        .build_as_child(win)
        .expect("création de l'onglet");
    set_password_save(&wv, pw);
    audio::watch(&wv, move |on| {
        let _ = p5.send_event(Ev::Audio(id, on));
    });
    let p6 = proxy.clone();
    audio::watch_fullscreen(&wv, move |on| {
        let _ = p6.send_event(Ev::Fullscreen(on));
    });
    let p7 = proxy.clone();
    watch_url(&wv, move |u| {
        let _ = p7.send_event(Ev::Url(id, u));
    });
    let cmp = cmp::swap(&wv, "", cmp);
    let _ = match html {
        Some((h, _)) => wv.load_html(&h),
        None => wv.load_url(url),
    };
    (wv, cmp)
}

fn main() {
    let dir = data_dir();
    let ext_dir = dir.join("extensions");
    let cfg_path = dir.join("settings.json");
    let _ = fs::create_dir_all(&ext_dir);
    let url_arg = defaults::arg_url(std::env::args().skip(1));
    let pipe = defaults::pipe_name(&dir);
    let first = !cfg_path.exists(); // page de bienvenue
    let mut s: Settings = load(&cfg_path);
    migrate(&mut s);
    let restarting = std::env::args().any(|a| a == "--restart");
    FAVH.store(if s.favbar { FAVBAR_H } else { 0 }, std::sync::atomic::Ordering::Relaxed);
    let hist_path = dir.join("history.json");
    let mut hist: Vec<Visit> = load(&hist_path);
    let _ = fs::remove_dir_all(previews()); // restes d'un arrêt brutal

    let ev_loop = EventLoopBuilder::<Ev>::with_user_event().build();
    let proxy = ev_loop.create_proxy();
    let on_msg = |p: EventLoopProxy<Ev>| {
        move |m: String| {
            if let Some(u) = defaults::web_url(&m) {
                let _ = p.send_event(Ev::Open(u));
            }
            let _ = p.send_event(Ev::Raise);
        }
    };
    // Instance unique par profil : un Volt tient déjà le tube, il reçoit l'URL et on s'arrête là.
    let mut listening = defaults::listen(&pipe, on_msg(proxy.clone()));
    if restarting {
        // Relancé par « restart » : l'ancien Volt se ferme. On attend (5 s au plus) qu'il lâche le tube, puis que
        // son moteur lâche le profil, sinon WebView2 refuse les nouveaux arguments (mode nuit).
        for _ in 0..50 {
            if listening.is_some() && !dir.join("profile").join("EBWebView").join("lockfile").exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
            if listening.is_none() {
                listening = defaults::listen(&pipe, on_msg(proxy.clone()));
            }
        }
    }
    if listening.is_none() && defaults::send(&pipe, url_arg.as_deref()) {
        return;
    }
    update::cleanup(); // ancien exe laissé par une mise à jour
    let win = WindowBuilder::new()
        .with_title("Volt")
        // Volty 64x64 en RGBA brut, généré par logos/make_icon.py (pas de décodeur PNG à embarquer)
        .with_window_icon(Icon::from_rgba(include_bytes!("../logos/volty64.rgba").to_vec(), 64, 64).ok())
        .with_theme(Some(WinTheme::Dark))
        .with_decorations(false)
        .with_undecorated_shadow(true)
        .with_background_color((11, 12, 16, 255))
        .with_inner_size(tao::dpi::LogicalSize::new(1280.0, 800.0))
        .build(&ev_loop)
        .expect("fenêtre");

    let mut ctx = WebContext::new(Some(dir.join("profile")));
    let (cores, ram_gb) = perf::machine();
    // VOLT_ARGS : arguments Chromium en plus, pour les mesures A/B sans recompiler (comme VOLT_EAGER).
    let args = perf::browser_args(cores, ram_gb, s.night, s.game_mode, &std::env::var("VOLT_ARGS").unwrap_or_default());
    let p = proxy.clone();
    // Le premier webview fixe l'environnement : mêmes options (extensions activées) partout.
    let ui = WebViewBuilder::new_with_web_context(&mut ctx)
        .with_html(UI)
        .with_additional_browser_args(&args)
        .with_background_color(LOADING_BG)
        .with_bounds(ui_rect(&win, false))
        .with_browser_extensions_enabled(true)
        .with_extensions_path(&ext_dir)
        .with_ipc_handler(move |r| {
            let b = r.body().clone();
            let _ = p.send_event(if KEYS.contains(&b.as_str()) { Ev::Key(b) } else { Ev::Ui(b) });
        })
        .build_as_child(&win)
        .expect("interface");
    let p = proxy.clone();
    perf::cpu_watch(move |c| {
        let _ = p.send_event(Ev::Cpu(c));
    });

    if s.adblock && !ext_dir.join(UBOL).exists() {
        install_ext(proxy.clone(), UBOL.into(), ext_dir.clone());
    }
    let plan = boot(&s.startup, restarting, s.session.len());
    let restored = plan == Boot::Restore;
    // gardée ici : l'interface ne reçoit que le nombre d'onglets, les adresses ne rouvrent que par « resume »
    let mut offer: Pending = (plan == Boot::Offer).then(|| (s.session.clone(), s.pinned, None));
    let fresh = !restored && url_arg.is_none(); // démarrage sans session ni adresse : la page de favoris s'ouvre
    let start = if restored { s.session.clone() } else { vec![s.home.clone()] };
    let mut next_id = 0u32;
    let mut tabs: Vec<Tab> = vec![];
    // Restauration paresseuse : seul le 1er onglet charge, les autres au premier clic.
    for u in &start {
        next_id += 1;
        // VOLT_EAGER : charge d'un coup les `keep_loaded` premiers (tests de charge ; tout charger a fait planter la machine).
        let eager = std::env::var_os("VOLT_EAGER").is_some() && (s.keep_loaded == 0 || next_id <= s.keep_loaded);
        // URL reçue en argument : c'est elle qu'on affiche, inutile de charger le 1er onglet restauré
        let (wv, cmp) = ((next_id == 1 && url_arg.is_none()) || eager).then(|| make_tab(&win, &mut ctx, &proxy, next_id, u, None, s.dark_web, s.save_passwords, &args, &cmp_js(&s))).unzip();
        let title = hist.iter().find(|h| &h.url == u).map_or(u.clone(), |h| h.title.clone());
        tabs.push(Tab { id: next_id, wv, title, url: u.clone(), loading: next_id == 1 && url_arg.is_none(), audio: false, muted: false, seen: Instant::now(), frozen: false, typed: false, saved: None, pinned: restored && next_id <= s.pinned, cmp: cmp.unwrap_or_default(), foreign: false });
    }
    let mut active = tabs[0].id;
    if let Some(u) = &url_arg {
        next_id += 1;
        let (wv, cmp) = make_tab(&win, &mut ctx, &proxy, next_id, u, None, s.dark_web, s.save_passwords, &args, &cmp_js(&s));
        tabs.push(Tab { id: next_id, wv: Some(wv), title: u.clone(), url: u.clone(), loading: true, audio: false, muted: false, seen: Instant::now(), frozen: false, typed: false, saved: None, pinned: false, cmp, foreign: false });
        active = next_id;
    }
    let mut panel = false;
    let mut next_tick = Instant::now() + TICK;
    let mut dirty = true;
    let mut minimized_at: Option<Instant> = None;
    let mut cursor = (0.0, 0.0);
    let mut closed: Vec<(usize, String)> = vec![]; // (position, adresse) des onglets fermés, pour « Rouvrir »
    let mut ext_at: Option<Instant> = None; // dernière installation d'extension (page d'accueil en arrière-plan)

    ev_loop.run(move |event, _, flow| {
        *flow = ControlFlow::WaitUntil(next_tick);

        // Affiche l'onglet actif, cache les autres (RAM minimale en mode jeu).
        // Fenêtre réduite : tout est caché, donc plus aucun rendu ni travail pour la carte graphique.
        let layout = |tabs: &Vec<Tab>, active: u32, panel: bool, game: bool, min: bool| {
            let _ = ui.set_visible(!full());
            let _ = ui.set_bounds(ui_rect(&win, panel));
            let _ = ui.set_memory_usage_level(if min { MemoryUsageLevel::Low } else { MemoryUsageLevel::Normal });
            for (t, wv) in tabs.iter().filter_map(|t| Some((t, t.wv.as_ref()?))) {
                let on = t.id == active && !panel && !min;
                // taille d'abord, affichage ensuite : sinon un onglet caché pendant un
                // redimensionnement apparaît une image à son ancienne taille
                if on {
                    let _ = wv.set_bounds(content_rect(&win));
                }
                let _ = wv.set_visible(on);
                let _ = wv.set_memory_usage_level(if min || (game && t.id != active) { MemoryUsageLevel::Low } else { MemoryUsageLevel::Normal });

            }
        };
        let sync = |tabs: &Vec<Tab>, active: u32, s: &Settings, offer: usize| {
            let list: Vec<Value> = tabs.iter().map(|t| json!({"id": t.id, "title": t.title, "url": t.url, "loading": t.loading, "audio": t.audio, "muted": t.muted, "asleep": t.wv.is_none(), "pinned": t.pinned, "foreign": t.foreign})).collect();
            let _ = ui.evaluate_script(&format!("render({})", json!({"tabs": list, "active": active, "s": s, "max": win.is_maximized(), "first": first, "fresh": fresh, "def": defaults::is_default(), "offer": offer, "ver": update::CURRENT})));
        };
        // Place interface et page ; la page (au-dessus de l'interface) d'abord quand elle gagne de la place,
        // sinon l'interface qui rétrécit laisserait voir une image le fond de la fenêtre.
        let place = |ui: &WebView, wv: Option<&WebView>, panel: bool, page_first: bool| {
            let page = || wv.map(|w| w.set_bounds(content_rect(&win)));
            if page_first {
                page();
            }
            let _ = ui.set_bounds(ui_rect(&win, panel));
            if !page_first {
                page();
            }
        };
        let toast = |m: &str| {
            let _ = ui.evaluate_script(&format!("toast({})", json!(m)));
        };
        let ext_list = || {
            let p = proxy.clone();
            ext::list(&ui, ext_dir.clone(), move |v| {
                let _ = p.send_event(Ev::Exts(v));
            });
        };
        let ext_changed = {
            let p = proxy.clone();
            move || {
                let _ = p.send_event(Ev::ExtChanged);
            }
        };

        let mut open = |url: &str, tabs: &mut Vec<Tab>, active: &mut u32| {
            next_id += 1;
            let (wv, cmp) = make_tab(&win, &mut ctx, &proxy, next_id, url, None, s.dark_web, s.save_passwords, &args, &cmp_js(&s));
            tabs.push(Tab { id: next_id, wv: Some(wv), title: "Nouvel onglet".into(), url: url.into(), loading: true, audio: false, muted: false, seen: Instant::now(), frozen: false, typed: false, saved: None, pinned: false, cmp, foreign: false });
            *active = next_id;
        };

        match event {
            // Veille : gèle les onglets cachés, puis les décharge de la RAM après une longue inactivité.
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                let now = Instant::now();
                next_tick = now + TICK;
                let unload = (s.sleep_min > 0).then(|| Duration::from_secs(s.sleep_min as u64 * 60));
                // VOLT_FREEZE (secondes) : délai de gel imposé, pour mesurer à chaud puis gelé (tests de charge).
                let freeze = std::env::var("VOLT_FREEZE").ok().and_then(|v| v.parse().ok()).map_or(if s.game_mode { FREEZE_GAME } else { FREEZE_AFTER }, Duration::from_secs);
                let min_long = minimized_at.is_some_and(|m| now - m >= FREEZE_MINIMIZED);
                let mut changed = false;
                for t in tabs.iter_mut().filter(|t| (t.id != active || min_long) && !t.audio && t.wv.is_some()) {
                    let idle = now - t.seen;
                    // texte non envoyé : gelé mais jamais déchargé, sinon il serait perdu
                    if t.id != active && !t.typed && !t.pinned && unload.is_some_and(|u| idle >= u) {
                        t.wv = None;
                        changed = true;
                    } else if !t.frozen && (min_long || idle >= freeze) {
                        perf::suspend(t.wv.as_ref().unwrap());
                        t.frozen = true;
                    }
                }
                changed |= unload_over_budget(&mut tabs, active, s.keep_loaded);
                // Aperçu pris sur l'onglet affiché : un onglet qu'on vient de cacher ne rend plus d'image à capturer.
                if minimized_at.is_none()
                    && !panel
                    && let Some(w) = tabs.iter().find(|t| t.id == active && !t.loading).and_then(|t| t.wv.as_ref())
                {
                    preview::capture(w, &preview_of(active));
                }
                dirty |= changed;
            }
            Event::WindowEvent { event: WindowEvent::Resized(_), .. } => {
                let m = win.is_minimized();
                if m != minimized_at.is_some() {
                    minimized_at = m.then(Instant::now);
                    perf::background(&ui, m);
                    if !m {
                        // réaffiché : WebView2 réveille l'onglet actif tout seul
                        tabs.iter_mut().filter(|t| t.id == active).for_each(|t| t.frozen = false);
                    }
                }
                // Tout de suite (pas au regroupement) : pendant un redimensionnement, Windows
                // retient la fin de vague jusqu'au relâchement de la souris.
                if !m {
                    let _ = ui.set_bounds(ui_rect(&win, panel));
                    if let Some(w) = tabs.iter().find(|t| t.id == active).and_then(|t| t.wv.as_ref()) {
                        let _ = w.set_bounds(content_rect(&win));
                    }
                }
                dirty = true;
            }
            // retour dans Volt (par exemple depuis la page Windows du navigateur par défaut) : état relu
            Event::WindowEvent { event: WindowEvent::Focused(true), .. } => {
                let _ = ui.evaluate_script(&format!("setDef({})", defaults::is_default()));
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                persist(&mut s, &tabs, &hist, &cfg_path, &hist_path, false, offer_n(&offer) > 0);
                *flow = ControlFlow::Exit;
            }
            // La souris n'atteint la fenêtre principale que sur la bordure (les webviews couvrent le reste).
            Event::WindowEvent { event: WindowEvent::CursorMoved { position, .. }, .. } => {
                cursor = (position.x, position.y);
                win.set_cursor_icon(cursor_for(edge(&win, position.x, position.y)));
            }
            Event::WindowEvent { event: WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. }, .. } => {
                if let Some(d) = edge(&win, cursor.0, cursor.1) {
                    let _ = win.drag_resize_window(d);
                }
            }
            Event::UserEvent(e) => {
                let prev = active;
                let mut shut: Vec<u32> = vec![]; // onglets à fermer, tous par `close` après le match
                match e {
                    // Page d'aperçu : ne touche ni titre, ni URL, ni historique, ni état à restaurer.
                    Ev::Title(_, t) | Ev::Load(_, t, _) if t == "about:blank" => {}
                    Ev::Title(id, t) => {
                        if let Some(tab) = tabs.iter_mut().find(|x| x.id == id) {
                            if let Some(h) = hist.first_mut().filter(|h| h.url == tab.url) {
                                h.title = t.clone();
                                if DRAWER.load(std::sync::atomic::Ordering::Relaxed) {
                                    let _ = ui.evaluate_script(&format!("histTitle({},{})", json!(h.url), json!(t)));
                                }
                            }
                            tab.title = t;
                        }
                    }
                    Ev::Load(id, url, l) => {
                        if let Some(tab) = tabs.iter_mut().find(|x| x.id == id) {
                            if !l && url.starts_with("http") {
                                add_visit(&mut hist, &url, &tab.title);
                                // tiroir ouvert seulement : l'historique entier ne part pas à chaque page chargée
                                if DRAWER.load(std::sync::atomic::Ordering::Relaxed) {
                                    let _ = ui.evaluate_script(&format!("renderHist({})", json!(hist)));
                                }
                            }
                            // Recréé après déchargement : remet défilement et vidéo, sauf si on a navigué ailleurs.
                            if !l && let Some((u, y, t)) = tab.saved.clone() {
                                if u == url
                                    && let Some(w) = &tab.wv
                                {
                                    let _ = w.evaluate_script(&format!("{RESTORE_JS}({y},{t})"));
                                }
                                // gardé tant que l'onglet est caché : une page qui se recharge seule ne doit pas le consommer
                                if id == active || u != url {
                                    tab.saved = None;
                                }
                            }
                            // langue relevée à la fin du chargement : icône de traduction, traduction automatique
                            if l {
                                tab.foreign = false;
                            } else if url.starts_with("http")
                                && !translate::is_relay(&url)
                                && let Some(w) = &tab.wv
                            {
                                let (p, u) = (proxy.clone(), url.clone());
                                let _ = w.evaluate_script_with_callback(translate::LANG_JS, move |r| {
                                    let _ = p.send_event(Ev::Lang(id, u.clone(), r));
                                });
                            }
                            tab.url = url;
                            tab.loading = l;
                            if !l && id == active && !panel && minimized_at.is_none()
                                && let Some(w) = &tab.wv
                            {
                                preview::capture(w, &preview_of(id));
                            }
                        }
                    }
                    // la vidéo affichée devient l'adresse de l'onglet (barre, étoile, session, restauration) et une visite
                    Ev::Url(id, url) => {
                        let Some(tab) = tabs.iter_mut().find(|x| x.id == id).filter(|t| t.url != url && url.starts_with("http")) else { return };
                        add_visit(&mut hist, &url, &tab.title);
                        if DRAWER.load(std::sync::atomic::Ordering::Relaxed) {
                            let _ = ui.evaluate_script(&format!("renderHist({})", json!(hist)));
                        }
                        tab.url = url;
                    }
                    // page en langue étrangère : traduite seule si demandé, sauf page de connexion ou site à ne pas traduire
                    Ev::Lang(id, url, r) => {
                        let Some(tab) = tabs.iter_mut().find(|t| t.id == id && t.url == url) else { return };
                        let Ok((fr, pw)) = serde_json::from_str::<(i8, bool)>(&r) else { return };
                        tab.foreign = fr == 0;
                        if tab.foreign
                            && s.translate_auto
                            && !pw
                            && !cmp::site(&url).is_some_and(|h| cmp::covers(&h, &s.no_translate))
                            && let Some(w) = &tab.wv
                        {
                            let _ = w.load_url(&translate::relay(&url));
                        }
                    }
                    Ev::State(id, r) => {
                        // réponse arrivée après un retour rapide sur l'onglet : périmée, un F5 remettrait l'ancien défilement
                        if id != active
                            && let Some(tab) = tabs.iter_mut().find(|x| x.id == id)
                            && let Ok((y, t, d)) = serde_json::from_str::<(f64, f64, bool)>(&r)
                        {
                            tab.saved = Some((tab.url.clone(), y, t));
                            tab.typed = d;
                        }
                    }
                    Ev::Fullscreen(on) => {
                        FULL.store(on, std::sync::atomic::Ordering::Relaxed);
                        win.set_fullscreen(on.then_some(Fullscreen::Borderless(None)));
                    }
                    Ev::Audio(id, on) => {
                        if let Some(tab) = tabs.iter_mut().find(|x| x.id == id) {
                            tab.audio = on;
                        }
                    }
                    // page ouverte par une extension (accueil à l'installation, parfois sur son site) : en arrière-plan, l'onglet regardé reste
                    // shortcut: un lien ouvert par l'utilisateur dans les 15 s d'une installation part aussi en arrière-plan ; suffisant tant que le moteur ne dit pas qui ouvre
                    Ev::Open(url) if url.starts_with("chrome-extension://") || ext_at.is_some_and(|t| t.elapsed() < EXT_WELCOME) => open(&url, &mut tabs, &mut active.clone()),
                    Ev::Open(url) => open(&url, &mut tabs, &mut active),
                    Ev::Raise => {
                        win.set_minimized(false);
                        win.set_focus();
                    }
                    Ev::Key(k) => {
                        let i = tabs.iter().position(|t| t.id == active).unwrap_or(0);
                        match k.as_str() {
                            "ct" => {
                                let h = s.home.clone();
                                open(&h, &mut tabs, &mut active)
                            }
                            "cw" => shut.push(active),
                            "cst" => {
                                let _ = proxy.send_event(Ev::Ui(json!({"cmd": "tabdo", "id": active, "action": "reopen"}).to_string()));
                                return;
                            }
                            "ctab" => active = tabs[(i + 1) % tabs.len()].id,
                            "cstab" => active = tabs[(i + tabs.len() - 1) % tabs.len()].id,
                            "cd" => {
                                toggle_fav(&mut s, &tabs[i]);
                                save(&s, &cfg_path);
                            }
                            "cl" => {
                                let _ = ui.focus();
                                let _ = ui.evaluate_script("focusUrl()");
                                return;
                            }
                            _ => {
                                let _ = ui.evaluate_script("toggleDrawer()"); // "ch"
                                return;
                            }
                        }
                    }
                    Ev::Exts(v) => {
                        let _ = ui.evaluate_script(&format!("renderExts({v})"));
                        return;
                    }
                    Ev::ExtChanged => {
                        ext_list();
                        return;
                    }
                    Ev::ExtFolder(f) => {
                        ext_at = Some(Instant::now());
                        let p = proxy.clone();
                        ext::add(&ui, f, move |r| {
                            let _ = p.send_event(Ev::Toast(match r {
                                Ok(()) => "Extension installée ✔".into(),
                                Err(e) => format!("Échec de l'installation : {e}"),
                            }));
                            let _ = p.send_event(Ev::ExtChanged);
                        });
                        return;
                    }
                    Ev::Cookies(v) => {
                        let _ = ui.evaluate_script(&format!("renderCookies({v})"));
                        return;
                    }
                    Ev::Toast(m) => {
                        toast(&m);
                        return;
                    }
                    Ev::Update(v) => {
                        let _ = ui.evaluate_script(&format!("upd({v})"));
                        return;
                    }
                    Ev::Cpu(c) => {
                        let _ = ui.evaluate_script(&format!("volty({c})"));
                        return;
                    }
                    Ev::Ui(body) => {
                        let Ok(v) = serde_json::from_str::<Value>(&body) else { return };
                        let str_of = |k: &str| v[k].as_str().unwrap_or_default().to_string();
                        let wv = tabs.iter().find(|t| t.id == active).and_then(|t| t.wv.as_ref());
                        match v["cmd"].as_str().unwrap_or_default() {
                            "ready" => ext_list(), // icônes de la barre d'adresse dès le démarrage
                            "sync" => {}
                            // l'interface reprend le focus : seul le bloc « navigateur par défaut » est relu
                            "def" => {
                                let _ = ui.evaluate_script(&format!("setDef({})", defaults::is_default()));
                                return;
                            }
                            // favori ouvert dans un nouvel onglet (Ctrl+clic, clic du milieu)
                            "tab" => match defaults::web_url(&str_of("url")) {
                                Some(u) => open(&u, &mut tabs, &mut active),
                                None => return,
                            },
                            "sort" => {
                                let names: Vec<String> = s.folders.iter().map(|d| d.name.clone()).collect();
                                // favoris sans dossier ou dans un dossier qui n'existe plus
                                let loose: Vec<(String, String)> = s.favs.iter().filter(|f| !s.folders.iter().any(|d| d.id == f.folder)).map(|f| (f.url.clone(), f.title.clone())).collect();
                                let props: Vec<Value> = sort::suggest(&loose, &names).into_iter().map(|p| json!({"folder": p.folder, "new": p.new, "icon": p.icon, "color": p.color, "urls": p.urls})).collect();
                                let _ = ui.evaluate_script(&format!("renderSort({})", json!(props)));
                                return;
                            }
                            "nav" if internal(&str_of("url")).is_some() => {
                                let sec = internal(&str_of("url")).unwrap();
                                let _ = ui.evaluate_script(&format!("openSection('{sec}')"));
                                return;
                            }
                            "nav" => {
                                if let Some(w) = wv {
                                    let _ = w.load_url(&to_url(&str_of("url"), &s.search));
                                    let _ = w.focus();
                                }
                            }
                            "back" => { wv.map(|w| w.evaluate_script("history.back()")); }
                            "fwd" => { wv.map(|w| w.evaluate_script("history.forward()")); }
                            "reload" => { wv.map(|w| w.reload()); }
                            "home" => { wv.map(|w| w.load_url(&s.home)); }
                            "new" => {
                                let h = s.home.clone();
                                open(&h, &mut tabs, &mut active)
                            }
                            "select" => active = v["id"].as_u64().unwrap_or(active as u64) as u32,
                            // menu de l'onglet ; « tabdo » applique une action sans menu (essais pilotés), « mute » et « close » sont les boutons de l'onglet
                            c @ ("tabmenu" | "tabdo" | "mute" | "close") => {
                                let id = v["id"].as_u64().unwrap_or(0) as u32;
                                let Some(i) = tabs.iter().position(|t| t.id == id) else { return };
                                let fav = s.favs.iter().any(|f| f.url == tabs[i].url);
                                let act = match c {
                                    "tabmenu" => tabmenu::show(win.hwnd(), &tabmenu::items(&tabs, id, fav, !closed.is_empty())).unwrap_or_default(),
                                    "tabdo" => v["action"].as_str().unwrap_or_default(),
                                    c => c,
                                };
                                match act {
                                    "new_right" => {
                                        let (h, at) = (s.home.clone(), tabmenu::new_at(&tabs, i));
                                        open(&h, &mut tabs, &mut active);
                                        tabmenu::move_last(&mut tabs, at);
                                    }
                                    "dup" => {
                                        let (u, p) = (tabs[i].url.clone(), tabs[i].pinned);
                                        open(&u, &mut tabs, &mut active);
                                        tabs.last_mut().unwrap().pinned = p; // comme Chrome : la copie d'un épinglé est épinglée, juste après lui
                                        tabmenu::move_last(&mut tabs, i + 1);
                                    }
                                    "reload" => { tabs[i].wv.as_ref().map(|w| w.reload()); }
                                    "pin" => tabmenu::toggle_pin(&mut tabs, i),
                                    "mute" => toggle_mute(&mut tabs[i]),
                                    "fav" => {
                                        toggle_fav(&mut s, &tabs[i]);
                                        save(&s, &cfg_path);
                                    }
                                    "close" => shut.push(id),
                                    "close_others" | "close_right" => {
                                        shut = if act == "close_others" { tabmenu::others(&tabs, id) } else { tabmenu::right(&tabs, id) };
                                        // comme Chrome : l'onglet cliqué devient actif si l'actif disparaît
                                        if shut.contains(&active) {
                                            active = id;
                                        }
                                    }
                                    "reopen" => {
                                        let Some((at, u)) = closed.pop() else { return };
                                        let at = tabmenu::reopen_at(&tabs, at);
                                        open(&u, &mut tabs, &mut active);
                                        tabmenu::move_last(&mut tabs, at);
                                    }
                                    _ => return,
                                }
                            }
                            "panel" => {
                                panel = v["open"].as_bool().unwrap_or(false);
                                if panel {
                                    ext_list();
                                }
                            }
                            "save" => {
                                if let Ok(mut n) = serde_json::from_value::<Settings>(v["s"].clone()) {
                                    n.session = std::mem::take(&mut s.session);
                                    (n.cmp_skip, n.no_translate) = (cmp::clean(&n.cmp_skip), cmp::clean(&n.no_translate));
                                    let had_adblock = s.adblock;
                                    let js = cmp_js(&n);
                                    if js != cmp_js(&s) {
                                        set_cmp(&mut tabs, &js);
                                    }
                                    s = n;
                                    if s.adblock && !had_adblock && !ext_dir.join(UBOL).exists() {
                                        install_ext(proxy.clone(), UBOL.into(), ext_dir.clone());
                                    } else if !s.adblock
                                        && had_adblock
                                        && let Ok(id) = fs::read_to_string(ext_dir.join(UBOL).join(".volt_id"))
                                    {
                                        ext::remove(&ui, id, ext_dir.clone(), ext_changed);
                                    }
                                    save(&s, &cfg_path);
                                    for w in tabs.iter().filter_map(|t| t.wv.as_ref()) {
                                        let _ = w.set_theme(if s.dark_web { Theme::Dark } else { Theme::Auto });
                                        set_password_save(w, s.save_passwords);
                                    }
                                }
                            }
                            "favh" => {
                                // bornée : l'interface ne doit jamais pouvoir pousser la page hors de la fenêtre
                                let h = v["h"].as_f64().unwrap_or(0.0).clamp(0.0, 120.0) as u32;
                                let old = FAVH.swap(h, std::sync::atomic::Ordering::Relaxed);
                                // tout de suite, à chaque image de l'animation de l'interface
                                place(&ui, wv, panel, h < old);
                                return;
                            }
                            "drawerw" => {
                                let w = v["w"].as_f64().unwrap_or(0.0).clamp(0.0, 340.0) as u32;
                                let old = DRAWER_W.swap(w, std::sync::atomic::Ordering::Relaxed);
                                place(&ui, wv, panel, w < old);
                                return;
                            }
                            // croix de la bande : l'icône de la barre d'adresse garde la reprise 3 minutes
                            "resume_later" => {
                                if let Some(o) = &mut offer {
                                    o.2.get_or_insert(Instant::now() + OFFER_FOR);
                                }
                                return;
                            }
                            // session rouverte comme au démarrage : onglets non chargés, l'onglet d'accueil de départ remplacé s'il n'a pas servi
                            "resume" => {
                                if offer_n(&offer) == 0 {
                                    offer = None;
                                    toast("La dernière session n'est plus proposée");
                                } else if let Some((urls, pinned, _)) = offer.take() {
                                    if let Some(i) = tabs.iter().position(|t| t.id == 1 && same_page(&t.url, &s.home)) {
                                        tabs.remove(i);
                                        let _ = fs::remove_file(preview_of(1));
                                    }
                                    let mut back: Vec<Tab> = urls.iter().enumerate().map(|(i, u)| {
                                        next_id += 1;
                                        let title = hist.iter().find(|h| &h.url == u).map_or(u.clone(), |h| h.title.clone());
                                        Tab { id: next_id, wv: None, title, url: u.clone(), loading: false, audio: false, muted: false, seen: Instant::now(), frozen: false, typed: false, saved: None, pinned: (i as u32) < pinned, cmp: String::new(), foreign: false }
                                    }).collect();
                                    if !tabs.iter().any(|t| t.id == active) {
                                        active = back[0].id;
                                    }
                                    back.append(&mut tabs);
                                    tabs = back;
                                    tabs.sort_by_key(|t| !t.pinned); // épinglés toujours en tête (tri stable : ordre gardé)
                                }
                            }
                            "go" => {
                                panel = false;
                                if let Some(w) = wv {
                                    let _ = w.load_url(&str_of("url"));
                                }
                            }
                            "hist" => {
                                let _ = ui.evaluate_script(&format!("renderHist({})", json!(hist)));
                                return;
                            }
                            // icône de la barre d'adresse : page traduite par le relais, ou retour à l'original (le site
                            // n'est alors plus traduit automatiquement, sinon il le serait aussitôt de nouveau)
                            "translate" => {
                                let (Some(w), Some(u)) = (wv, tabs.iter().find(|t| t.id == active).map(|t| t.url.clone())) else { return };
                                if !translate::is_relay(&u) {
                                    if u.starts_with("http") {
                                        let _ = w.load_url(&translate::relay(&u));
                                    }
                                    return;
                                }
                                let Some(o) = translate::original(&u) else {
                                    toast("Adresse d'origine introuvable");
                                    return;
                                };
                                let _ = w.load_url(&o);
                                if let Some(h) = cmp::site(&o).filter(|h| !s.no_translate.contains(h)) {
                                    s.no_translate.push(h);
                                    save(&s, &cfg_path);
                                }
                            }
                            // site exclu de la réponse aux bandeaux, saisi librement (ou le site affiché)
                            "cmp_skip" => {
                                let Some(h) = cmp::site(&str_of("site")) else {
                                    toast("Ce n'est pas une adresse de site (exemple : lemonde.fr)");
                                    return;
                                };
                                if s.cmp_skip.contains(&h) {
                                    return;
                                }
                                s.cmp_skip.push(h);
                                save(&s, &cfg_path);
                                set_cmp(&mut tabs, &cmp_js(&s));
                            }
                            "pw_clear" => {
                                let p = proxy.clone();
                                let started = cookies::clear_passwords(&ui, move |ok| {
                                    let _ = p.send_event(Ev::Toast(if ok { "Mots de passe enregistrés supprimés" } else { "Suppression des mots de passe impossible" }.into()));
                                });
                                if !started {
                                    toast("Suppression des mots de passe impossible");
                                }
                                return;
                            }
                            "default_browser" => {
                                if let Err(e) = defaults::ask() {
                                    toast(&format!("Enregistrement impossible : {e}"));
                                }
                                return;
                            }
                            "browsers" => {
                                let _ = ui.evaluate_script(&format!("renderBrowsers({})", import::sources()));
                                return;
                            }
                            "import" => {
                                let r = import::import(&str_of("id"), &mut s.favs, &mut s.folders);
                                let _ = ui.evaluate_script(&format!("imported({},{})", json!(str_of("id")), json!(r)));
                                match r {
                                    Some(n) => {
                                        save(&s, &cfg_path);
                                        toast(&format!("{n} favoris importés"));
                                    }
                                    None => {
                                        toast("Favoris introuvables");
                                        return;
                                    }
                                }
                            }
                            // tiroir Historique et cookies : la page se décale (layout), l'historique part une fois
                            "drawer" => {
                                let on = v["open"].as_bool().unwrap_or(false);
                                DRAWER.store(on, std::sync::atomic::Ordering::Relaxed);
                                if on {
                                    let _ = ui.evaluate_script(&format!("renderHist({})", json!(hist)));
                                    let _ = ui.focus(); // Ctrl+H depuis la page : le filtre reçoit la frappe, Échap ferme
                                } else if let Some(w) = wv {
                                    let _ = w.focus();
                                }
                            }
                            "hist_rm" => {
                                let u = str_of("url");
                                hist.retain(|h| h.url != u);
                                let _ = ui.evaluate_script(&format!("renderHist({})", json!(hist)));
                                return;
                            }
                            // nombres seulement : les valeurs des cookies restent dans le Rust (cookies.rs)
                            c @ ("cookies" | "cookies_site" | "cookies_all") => {
                                let u = tabs.iter().find(|t| t.id == active).map(|t| t.url.clone()).unwrap_or_default();
                                let p = proxy.clone();
                                match c {
                                    "cookies" => cookies::counts(&ui, &u, move |v| {
                                        let _ = p.send_event(Ev::Cookies(v));
                                    }),
                                    _ => {
                                        let again = move || {
                                            let _ = p.send_event(Ev::Ui(json!({"cmd": "cookies"}).to_string()));
                                        };
                                        if c == "cookies_all" { cookies::delete_all(&ui, again) } else { cookies::delete_site(&ui, &u, again) }
                                    }
                                }
                                return;
                            }
                            "hist_clear" => {
                                hist.clear();
                                let _ = fs::write(&hist_path, b"[]");
                                let _ = ui.evaluate_script("renderHist([])");
                                return;
                            }
                            "drag" => {
                                let _ = win.drag_window();
                                return;
                            }
                            "resize_top" => {
                                if !win.is_maximized() {
                                    let _ = win.drag_resize_window(ResizeDirection::North);
                                }
                                return;
                            }
                            "win_min" => {
                                win.set_minimized(true);
                                return;
                            }
                            "win_max" => {
                                win.set_maximized(!win.is_maximized());
                                return;
                            }
                            "win_close" => {
                                persist(&mut s, &tabs, &hist, &cfg_path, &hist_path, false, offer_n(&offer) > 0);
                                *flow = ControlFlow::Exit;
                                return;
                            }
                            "restart" => {
                                persist(&mut s, &tabs, &hist, &cfg_path, &hist_path, true, offer_n(&offer) > 0);
                                // Le nouveau Volt attend lui-même que celui-ci ait libéré le profil (voir --restart dans main).
                                // Après une mise à jour, ce chemin est celui du nouvel exe (l'ancien a été renommé).
                                if let Err(e) = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg("--restart").spawn()) {
                                    toast(&format!("Redémarrage impossible : {e}"));
                                    return;
                                }
                                *flow = ControlFlow::Exit;
                                return;
                            }
                            // la commande ne transporte rien : adresses et noms de fichiers sont fixés dans update.rs
                            c @ ("update_check" | "update_install") => {
                                run_update(proxy.clone(), c == "update_install");
                                return;
                            }
                            "clear" => {
                                let _ = ui.clear_all_browsing_data();
                                let _ = fs::remove_dir_all(previews());
                                toast("Données de navigation effacées");
                                return;
                            }
                            "ext_install" => {
                                // bouton « Installer dans Volt » : l'adresse réelle de la page à cet instant (le Store change de fiche sans recharger)
                                let src = if v["page"].as_bool() == Some(true) { wv.and_then(|w| w.url().ok()).unwrap_or_default() } else { str_of("id") };
                                let Some(id) = ext::parse_id(&src) else {
                                    toast("ID ou lien Chrome Web Store invalide");
                                    return;
                                };
                                if ext_dir.join(&id).exists() {
                                    toast("Cette extension est déjà installée");
                                    return;
                                }
                                toast("Téléchargement de l'extension…");
                                install_ext(proxy.clone(), id.clone(), ext_dir.clone());
                                // uBlock Origin Lite installé ici : le réglage du bloqueur suit
                                if id != UBOL || s.adblock {
                                    return;
                                }
                                s.adblock = true;
                                save(&s, &cfg_path);
                            }
                            // icône de la barre d'adresse (menu natif) ; « extdo » applique une action sans menu (liste des réglages, essais pilotés)
                            c @ ("extmenu" | "extdo") => {
                                let (id, on) = (str_of("id"), v["on"].as_bool().unwrap_or(true));
                                let act = match c {
                                    "extmenu" => tabmenu::show(win.hwnd(), &tabmenu::ext_items(on, ext::options_url(&ext_dir, &id).is_some())).unwrap_or_default(),
                                    _ => v["action"].as_str().unwrap_or_default(),
                                };
                                match (act, ext::options_url(&ext_dir, &id)) {
                                    ("opts", Some(u)) => {
                                        panel = false;
                                        open(&u, &mut tabs, &mut active);
                                    }
                                    ("opts", None) => {
                                        toast("Cette extension n'a pas de page d'options");
                                        return;
                                    }
                                    ("toggle", _) => {
                                        ext::enable(&ui, id, !on, ext_changed);
                                        return;
                                    }
                                    ("manage", _) => {
                                        let _ = ui.evaluate_script("openSection('exts')");
                                        return;
                                    }
                                    ("remove", _) => {
                                        // lu avant : `remove` efface le dossier tout de suite
                                        let ubol = ext::store_id(&ext_dir, &id).as_deref() == Some(UBOL);
                                        ext::remove(&ui, id, ext_dir.clone(), ext_changed);
                                        // uBlock Origin Lite désinstallé : bloqueur coupé, sinon il reviendrait au démarrage
                                        if !ubol || !s.adblock {
                                            return;
                                        }
                                        s.adblock = false;
                                        save(&s, &cfg_path);
                                    }
                                    _ => return,
                                }
                            }
                            _ => return,
                        }
                    }
                }
                if !shut.is_empty() {
                    match close(&mut tabs, active, &shut, &mut closed) {
                        Some(a) => active = a,
                        None => {
                            persist(&mut s, &tabs, &hist, &cfg_path, &hist_path, false, offer_n(&offer) > 0);
                            *flow = ControlFlow::Exit;
                            return;
                        }
                    }
                }
                if let Some(t) = tabs.iter_mut().find(|t| t.id == active) {
                    if t.wv.is_none() {
                        // Capture de la page laissée, affichée tout de suite pendant le rechargement derrière. Sans capture,
                        // rien : une page d'attente a été mesurée, elle ne s'affichait jamais et coûtait 50 ms.
                        let html = t.url.starts_with("http").then(|| preview::page(&preview_of(t.id), &t.url)).flatten().map(|h| (h, preview::bg(&preview_of(t.id)).unwrap_or(LOADING_BG)));
                        let (w, c) = make_tab(&win, &mut ctx, &proxy, t.id, &t.url, html, s.dark_web, s.save_passwords, &args, &cmp_js(&s));
                        (t.wv, t.cmp) = (Some(w), c);
                        t.loading = true;
                    } else if active != prev {
                        t.saved = None; // resté chargé : rien à restaurer
                    }
                    t.seen = Instant::now();
                    t.frozen = false; // WebView2 réveille tout seul un onglet gelé quand il redevient visible
                }
                // Onglet quitté : caché mais ses scripts tournent encore, on relève son état pour le rechargement.
                if active != prev
                    && let Some(w) = tabs.iter().find(|t| t.id == prev).and_then(|t| t.wv.as_ref())
                {
                    let p = proxy.clone();
                    let _ = w.evaluate_script_with_callback(STATE_JS, move |r| {
                        let _ = p.send_event(Ev::State(prev, r));
                    });
                }
                unload_over_budget(&mut tabs, active, s.keep_loaded);
                dirty = true;
            }
            // Un seul rendu de l'interface par vague d'événements (pas un par événement de page).
            Event::MainEventsCleared if dirty => {
                dirty = false;
                layout(&tabs, active, panel, s.game_mode, minimized_at.is_some());
                sync(&tabs, active, &s, offer_n(&offer));
            }
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(to_url("youtube.com", "google"), "https://youtube.com");
        assert_eq!(to_url("http://localhost:3000", "google"), "http://localhost:3000");
        assert_eq!(to_url("localhost:8080", "google"), "https://localhost:8080");
        assert_eq!(to_url("meilleur fps cs2", "duckduckgo"), "https://duckduckgo.com/?q=meilleur+fps+cs2");
        assert_eq!(to_url("é&", "google"), "https://www.google.com/search?q=%C3%A9%26");
        assert_eq!(internal("volt://history"), Some("hist"));
        assert_eq!(internal("chrome://extensions/"), Some("exts"));
        assert_eq!(internal("VOLT://settings"), Some("home"));
        assert_eq!(internal("volt://settings/search"), Some("start"));
        assert_eq!(internal("youtube.com"), None);
        assert_eq!(internal("volt://store"), Some("exts"));
        assert_eq!(internal("chrome://newtab/"), Some("board"));
        assert_eq!(internal("volt://welcome"), Some("hello"));
    }

    #[test]
    fn closing_tabs() {
        let tab = |id| Tab { id, wv: None, title: String::new(), url: String::new(), loading: false, audio: false, muted: false, seen: Instant::now(), frozen: false, typed: false, saved: None, pinned: false, cmp: String::new(), foreign: false };
        let mut t = vec![tab(1), tab(2), tab(3)];
        assert_eq!(close_tab(&mut t, 2, 2), Some(3)); // actif fermé -> voisin de droite
        assert_eq!(close_tab(&mut t, 3, 1), Some(3)); // autre onglet fermé -> l'actif reste
        assert_eq!(close_tab(&mut t, 3, 3), None); // dernier fermé
        assert_eq!(close_tab(&mut t, 3, 9), None); // inconnu
    }

    #[test]
    fn tab_budget() {
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs(s);
        // id, vu à t0+s, protégé
        let tabs = [(1, at(5), false), (2, at(1), false), (3, at(9), false), (4, at(3), false)];
        assert!(over_budget(&tabs, 4).is_empty()); // sous le budget
        assert!(over_budget(&tabs, 9).is_empty());
        assert_eq!(over_budget(&tabs, 2), vec![2, 4]); // plus anciens d'abord
        assert!(over_budget(&tabs, 0).is_empty()); // illimité
        let prot = [(1, at(5), false), (2, at(1), true), (3, at(9), false), (4, at(3), true)];
        assert_eq!(over_budget(&prot, 2), vec![1, 3]); // protégés comptés mais épargnés
        assert_eq!(over_budget(&prot, 3), vec![1]);
        let all = [(1, at(1), true), (2, at(2), true), (3, at(3), true)];
        assert!(over_budget(&all, 1).is_empty()); // tout l'excédent protégé
        assert_eq!(over_budget(&[(1, at(1), true), (2, at(2), true), (3, at(3), false)], 1), vec![3]);
    }

    #[test]
    fn old_settings_still_load() {
        let s: Settings = serde_json::from_str(r#"{"home":"https://x.fr","game_mode":false}"#).unwrap();
        assert_eq!(s.home, "https://x.fr");
        assert!(!s.game_mode && s.favs.is_empty() && s.search == "google");
        assert!(s.keep_loaded == 7 && !s.volty_anim && s.save_passwords && s.exts_open);
        // traduction et bandeaux : désactivés par défaut, réponse « refuser », listes vides ; script vide tant que c'est coupé
        assert!(!s.translate_auto && !s.cmp_auto && s.cmp_mode == "refuser" && s.cmp_skip.is_empty() && s.no_translate.is_empty());
        assert!(cmp_js(&s).is_empty());
        let on: Settings = serde_json::from_str(r#"{"cmp_auto":true,"cmp_mode":"accepter","cmp_skip":["lemonde.fr"]}"#).unwrap();
        assert!(cmp_js(&on).contains(r#"const MODE = "accepter";"#) && cmp_js(&on).contains(r#"const EXCLUS = ["lemonde.fr"];"#));
        let off: Settings = serde_json::from_str(r#"{"save_passwords":false}"#).unwrap();
        assert!(!off.save_passwords && off.startup == "proposer");
        // ancien booléen `restore` : vrai -> proposer, faux -> à neuf ; jamais réécrit
        let mut on: Settings = serde_json::from_str(r#"{"restore":true,"session":["https://a.fr","https://b.fr"],"pinned":1}"#).unwrap();
        migrate(&mut on);
        assert!(on.startup == "proposer" && on.session.len() == 2 && on.pinned == 1);
        assert!(!serde_json::to_string(&on).unwrap().contains("restore"));
        let mut no: Settings = serde_json::from_str(r#"{"restore":false}"#).unwrap();
        migrate(&mut no);
        assert_eq!(no.startup, "neuf");
        let mut auto: Settings = serde_json::from_str(r#"{"startup":"auto"}"#).unwrap();
        migrate(&mut auto);
        assert_eq!(auto.startup, "auto");
        let bom: Settings = parse_json(b"\xEF\xBB\xBF{\"home\":\"https://y.fr\"}").unwrap();
        assert_eq!(bom.home, "https://y.fr");
        assert!(parse_json::<Settings>(b"{cass").is_none());
        // anciens favoris (sans icône ni dossier) + dossiers
        let f: Settings = serde_json::from_str(
            r##"{"favs":[{"url":"https://a.fr","title":"A"}],"folders":[{"id":"g","name":"Jeux","color":"#f00","icon":"🎮"}]}"##,
        )
        .unwrap();
        assert!(f.favs[0].icon.is_empty() && f.favs[0].folder.is_empty() && f.folders[0].icon == "🎮" && f.favbar && f.favbar_style == "etageres");
        assert!(!serde_json::to_string(&f.favs[0]).unwrap().contains("icon"));
    }

    #[test]
    fn session_at_start_and_close() {
        use {Boot::*, Close::*};
        assert_eq!(boot("proposer", false, 5), Offer);
        assert_eq!(boot("proposer", false, 1), Fresh); // un seul onglet : rien à proposer
        assert_eq!(boot("proposer", false, 0), Fresh);
        assert_eq!(boot("auto", false, 1), Restore);
        assert_eq!(boot("neuf", false, 5), Fresh);
        for m in ["proposer", "auto", "neuf"] {
            assert_eq!(boot(m, true, 5), Restore); // redémarrage, mise à jour
            assert_eq!(boot(m, true, 1), Restore);
            assert_eq!(boot(m, true, 0), Fresh);
        }
        assert_eq!(on_close("proposer", false, false, true), Save);
        assert_eq!(on_close("proposer", false, true, true), KeepOld); // reprise proposée, rien fait
        assert_eq!(on_close("proposer", false, true, false), Save); // navigué : la nouvelle session
        assert_eq!(on_close("auto", false, false, false), Save);
        assert_eq!(on_close("neuf", false, true, true), Forget);
        assert_eq!(on_close("neuf", true, false, false), Save);
        assert_eq!(on_close("proposer", true, true, true), KeepOld); // redémarrer sans avoir repris ne perd rien
        let tab = |url: &str| Tab { id: 1, wv: None, title: String::new(), url: url.into(), loading: false, audio: false, muted: false, seen: Instant::now(), frozen: false, typed: false, saved: None, pinned: false, cmp: String::new(), foreign: false };
        assert!(untouched(&[tab("https://www.google.com/")], "https://www.google.com"));
        assert!(untouched(&[tab("https://www.google.com/?zx=1#a")], "https://www.google.com"));
        assert!(untouched(&[], "https://www.google.com"));
        assert!(!untouched(&[tab("https://www.google.com/search?q=a")], "https://www.google.com"));
        assert!(!untouched(&[tab("https://www.google.com"), tab("https://www.google.com")], "https://www.google.com"));
        let mut o: Pending = Some((vec!["a".into(), "b".into()], 0, None));
        assert_eq!(offer_n(&o), 2);
        o.as_mut().unwrap().2 = Some(Instant::now() - Duration::from_secs(1));
        assert_eq!(offer_n(&o), 0); // icône expirée
    }

    #[test]
    fn old_history_still_loads() {
        // ancien history.json : ni heure ni icône ; l'heure vaut 0 (groupe « Plus ancien »)
        let h: Vec<Visit> = parse_json(br#"[{"url":"https://a.fr","title":"A"},{"url":"https://b.fr","title":"B","at":1763800000}]"#).unwrap();
        assert!(h[0].at == 0 && h[0].title == "A" && h[1].at == 1763800000);
        let mut back: Vec<Visit> = serde_json::from_str(&serde_json::to_string(&h).unwrap()).unwrap();
        assert_eq!(back[1].at, 1763800000);
        // une visite par page vue : une adresse revue remonte en tête avec l'heure actuelle, sans doublon
        add_visit(&mut back, "https://b.fr", "B2");
        add_visit(&mut back, "https://v.fr/watch?v=2", "V2");
        assert_eq!(back.iter().map(|h| h.url.as_str()).collect::<Vec<_>>(), ["https://v.fr/watch?v=2", "https://b.fr", "https://a.fr"]);
        assert!(back[1].title == "B2" && back[1].at > 1763800000);
        // les favoris gardent leur format (pas d'heure)
        let f = Page { url: "https://c.fr".into(), title: "C".into(), icon: String::new(), folder: String::new() };
        assert_eq!(serde_json::to_string(&f).unwrap(), r#"{"url":"https://c.fr","title":"C"}"#);
    }
}
