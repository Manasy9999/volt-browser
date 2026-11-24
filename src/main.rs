#![cfg_attr(not(test), windows_subsystem = "windows")]
mod audio;
mod cards;
mod chromium;
mod migrate;
mod cmp;
mod cookies;
mod defaults;
mod dl;
mod ext;
mod import;
mod lang;
mod perf;
mod preview;
mod sign;
mod sort;
mod tabmenu;
mod translate;
mod update;
mod vault;
mod vicon;

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use webview2_com::{
    Microsoft::Web::WebView2::Win32::{ICoreWebView2_20, ICoreWebView2Settings4},
    SourceChangedEventHandler, take_pwstr,
};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL};
use windows::Win32::UI::WindowsAndMessaging::{IDCANCEL, IDYES, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK, MB_YESNOCANCEL, MessageBoxW};
use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, GetCursorPos, HWND_BOTTOM, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos, WindowFromPoint};
use windows::core::{BOOL, Interface, PWSTR};
use tao::{
    dpi::PhysicalPosition,
    event::{ElementState, Event, MouseButton, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget},
    platform::windows::{WindowBuilderExtWindows, WindowExtWindows},
    window::{CursorIcon, Fullscreen, Icon, ResizeDirection, Theme as WinTheme, Window, WindowBuilder, WindowId},
};
use wry::{
    MemoryUsageLevel, PageLoadEvent, Rect, Theme, WebContext, WebView, WebViewBuilder,
    WebViewBuilderExtWindows, WebViewExtWindows,
    dpi::{LogicalPosition, LogicalSize},
};

const TOP: f64 = 84.0; // hauteur barre d'onglets + barre d'adresse
const FAVBAR_H: u32 = 30; // une rangée, en attendant la vraie hauteur envoyée par l'interface
// Hauteur de la barre de favoris (px logiques, 0 = masquée) : l'interface l'envoie (« favh »), la mise en page la lit.
static FAVH: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

// Hauteur que l'interface demande au-dessus de la page (px logiques, 0 = rien) pour sa liste de suggestions : elle passe
// alors devant la page au lieu de la pousser, la page ne bouge pas.
static OVER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

// Une page est en plein écran (vidéo) : la page prend toute la fenêtre.
static FULL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn full() -> bool {
    FULL.load(std::sync::atomic::Ordering::Relaxed)
}

// Réglage « remplir les formulaires » : le moteur retient et propose nom, adresse, e-mail, téléphone (jamais les mots de
// passe ni les cartes, tenus par Volt).
static AUTOFILL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

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
// raccourcis pris par Volt avant la page : Ctrl (c), Maj (s) puis la touche
const KEYS: [&str; 24] = ["ct", "cw", "cl", "ctab", "cstab", "cd", "ch", "cst", "cn", "csn", "cf", "cj", "csa", "sescape", "f11", "c1", "c2", "c3", "c4", "c5", "c6", "c7", "c8", "c9"];
// Ctrl+clic ou clic molette sur un lien : signalé juste avant que le moteur demande la nouvelle fenêtre, pour l'ouvrir
// en arrière-plan comme les autres navigateurs (le moteur, lui, ne dit pas où l'utilisateur la veut).
const BG_JS: &str = r#"(()=>{const a=e=>e.target&&e.target.closest&&e.target.closest('a[href]');addEventListener('click',e=>{if((e.ctrlKey||e.metaKey)&&!e.shiftKey&&a(e))window.ipc.postMessage('bg')},true);addEventListener('auxclick',e=>{if(e.button===1&&a(e))window.ipc.postMessage('bg')},true)})()"#;
const KEY_JS: &str = r#"addEventListener('keydown',e=>{const k=(e.ctrlKey?'c':'')+(e.shiftKey?'s':'')+e.key.toLowerCase();if(['ct','cw','cl','ctab','cstab','cd','ch','cst','cn','csn','cf','cj','csa','sescape','f11','c1','c2','c3','c4','c5','c6','c7','c8','c9'].includes(k)){e.preventDefault();window.ipc.postMessage(k)}},true)"#;
// Mémorise les champs où l'utilisateur a tapé (jamais leur contenu) ; composedPath : champs dans un shadow DOM.
const TYPED_JS: &str = r#"addEventListener('input',e=>{const t=e.composedPath()[0];if(t.tagName=='TEXTAREA'||t.isContentEditable||(t.tagName=='INPUT'&&/^(text|search|email|url|tel|number|password)$/.test(t.type)))(window.__voltTyped||=new Set).add(t)},true)"#;
// Gestionnaire de mots de passe : repère un formulaire de connexion (champ `password` + identifiant associé) et le signale
// (`pw:form`, sans donnée) ; à l'envoi, transmet identifiant+mot de passe au Rust seul (`pw:save:`, jamais journalisé) ;
// `__voltFill` remplit sur demande de Volt (valeurs insérées en JSON par le Rust). shortcut: détection au chargement et au
// focus seulement (un formulaire d'appli monopage apparu plus tard n'est vu qu'au premier focus) ; à revoir si des sites manquent.
const PW_JS: &str = r#"(()=>{const isPw=e=>e.tagName==='INPUT'&&e.type==='password';
const userIn=root=>{const ins=[...root.querySelectorAll('input')];const pi=ins.findIndex(isPw);for(let i=pi-1;i>=0;i--){const t=ins[i];if(/^(text|email|tel)$/.test(t.type)||/user|email|login|mail/i.test((t.autocomplete||'')+' '+(t.name||'')+' '+(t.id||'')))return t}return ins.find(t=>/^(text|email|tel)$/.test(t.type))};
let sent=false;const scan=()=>{if(sent)return;if([...document.querySelectorAll('input')].some(isPw)){sent=true;window.ipc.postMessage('pw:form')}};
addEventListener('DOMContentLoaded',scan,true);addEventListener('load',scan,true);addEventListener('focusin',scan,true);
addEventListener('submit',e=>{const f=e.target;if(!f.querySelector)return;const pw=f.querySelector('input[type=password]');if(!pw||!pw.value)return;const u=userIn(f);window.ipc.postMessage('pw:save:'+JSON.stringify({u:u?u.value:'',p:pw.value}))},true);
window.__voltFill=(user,pass)=>{const pw=document.querySelector('input[type=password]');if(!pw)return;const u=userIn(pw.form||document);if(u&&user){u.focus();u.value=user;u.dispatchEvent(new Event('input',{bubbles:true}));u.dispatchEvent(new Event('change',{bubbles:true}))}pw.focus();pw.value=pass;pw.dispatchEvent(new Event('input',{bubbles:true}));pw.dispatchEvent(new Event('change',{bubbles:true}))}})()"#;
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

// Carte « Préférés » de la page de favoris, modifiable par l'utilisateur.
#[derive(Serialize, Deserialize, Clone)]
struct Vignette {
    name: String,
    url: String,
    // image perso : vide, ou data:image/...;base64,... (stockée telle quelle, comme les icônes perso de dossiers)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    image: String,
    // couleur de la carte choisie par l'utilisateur ; vide = « Auto » (dérivée du logo, sinon teinte du site)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    color: String,
}

/// Cartes « Préférés » par défaut (sites connus, sans image) ; l'utilisateur les modifie ensuite.
fn default_vignettes() -> Vec<Vignette> {
    // Wikipédia et Amazon : l'édition de la langue de l'interface (adresses dans le dictionnaire)
    let (wiki, wiki_url, amazon) = (lang::t("Wikipédia"), lang::t("https://fr.wikipedia.org"), lang::t("https://www.amazon.fr"));
    [
        ("YouTube", "https://www.youtube.com"),
        ("Netflix", "https://www.netflix.com"),
        ("Twitch", "https://www.twitch.tv"),
        ("Spotify", "https://open.spotify.com"),
        ("GitHub", "https://github.com"),
        ("Reddit", "https://www.reddit.com"),
        ("Steam", "https://store.steampowered.com"),
        ("Discord", "https://discord.com"),
        (wiki.as_str(), wiki_url.as_str()),
        ("Amazon", amazon.as_str()),
        ("X", "https://x.com"),
        ("Instagram", "https://www.instagram.com"),
    ]
    .iter()
    .map(|(n, u)| Vignette { name: (*n).into(), url: (*u).into(), image: String::new(), color: String::new() })
    .collect()
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
    safe_browsing: bool, // alerte sur les sites dangereux (SmartScreen du moteur) ; lu au démarrage seulement
    sleep_min: u32, // 0 = jamais décharger
    keep_loaded: u32, // onglets chargés au plus (~108 Mo chacun) ; 0 = illimité
    volty_anim: bool, // lu par l'interface seulement
    session: Vec<String>,
    pinned: u32, // les `pinned` premiers onglets de la session sont épinglés (toujours en tête)
    session_groups: Vec<u32>, // groupe de chaque onglet de la session (0 : aucun), dans le même ordre
    groups: Vec<Group>,       // les groupes de cette session
    favs: Vec<Page>,
    folders: Vec<Folder>,
    vignettes: Vec<Vignette>, // cartes « Préférés » en haut de la page de favoris
    favbar: bool,
    favbar_style: String, // "etageres" (défaut), "ruban" ou "icones" ; lu par l'interface seulement
    fav_open: String,     // dossier de favoris : "barre" (défaut, il se déplie dans la barre) ou "menu" (liste, comme Chrome)
    save_passwords: bool, // le moteur propose d'enregistrer les identifiants à la connexion
    autofill: bool,       // le moteur retient et propose ce qu'on tape dans les formulaires (nom, adresse, e-mail…)
    welcomed: bool, // « Commencer » cliqué sur la page de bienvenue : elle ne revient plus
    exts_open: bool, // icônes des extensions dépliées dans la barre d'adresse ; lu par l'interface seulement
    translate_auto: bool, // page détectée en langue étrangère : rechargée traduite par le relais de Google
    no_translate: Vec<String>, // sites jamais traduits automatiquement (« Voir l'original » les ajoute)
    popup_allow: Vec<String>,  // sites autorisés à ouvrir des fenêtres surgissantes
    download_dir: String,      // dossier des téléchargements ; vide : celui de Windows
    download_ask: bool,        // fenêtre « Enregistrer sous » à chaque téléchargement
    auto_update: bool,         // cherche et installe les mises à jour en fond ; actives au prochain démarrage (lu au démarrage)
    cmp_auto: bool, // réponse automatique aux bandeaux de cookies (cmp.rs)
    cmp_mode: String, // "refuser" (défaut) ou "accepter"
    cmp_skip: Vec<String>, // sites exclus de la réponse automatique
    lang: String, // langue de l'interface : "auto" (celle de Windows), "fr" ou le code d'un dictionnaire (lang.rs)
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
            safe_browsing: true,
            sleep_min: 20,
            keep_loaded: 7,
            volty_anim: false,
            session: vec![],
            pinned: 0,
            session_groups: vec![],
            groups: vec![],
            favs: vec![],
            folders: vec![],
            vignettes: default_vignettes(),
            favbar: true,
            favbar_style: "etageres".into(),
            fav_open: "barre".into(),
            save_passwords: true,
            autofill: true,
            welcomed: false,
            exts_open: true,
            translate_auto: false,
            no_translate: vec![],
            popup_allow: vec![],
            download_dir: String::new(),
            download_ask: false,
            auto_update: true,
            cmp_auto: false,
            cmp_mode: "refuser".into(),
            cmp_skip: vec![],
            lang: "auto".into(),
        }
    }
}

/// D'où vient un raccourci clavier : de l'interface d'une fenêtre, ou de la page d'un onglet.
#[derive(Clone, Copy)]
enum Src {
    Win(WindowId),
    Tab(u32),
}

/// À quelle fenêtre revient un évènement.
enum Dest {
    Win(WindowId),
    Tab(u32), // celle qui porte cet onglet
    Here,     // n'importe laquelle : la courante
}

/// Fenêtre qui n'est pas en cours de traitement. La boucle ne travaille que sur UNE fenêtre à la fois, la « courante »,
/// tenue dans ses variables (win, ui, tabs, active…) ; les autres attendent ici, et `swap_win!` échange les états.
/// Ordre des champs = ordre de destruction : les vues avant leur fenêtre.
struct Parked {
    ui: WebView,
    tabs: Vec<Tab>,
    win: Window,
    active: u32,
    panel: bool,
    minimized_at: Option<Instant>,
    cursor: (f64, f64),
    offer: Pending,
    first: bool,
    fresh: bool,
    private: bool, // navigation privée : ni historique, ni session, ni aperçus sur le disque ; vues hors du profil
    // mise en page de cette fenêtre (les globales FAVH, DRAWER, DRAWER_W et FULL sont celles de la courante)
    favh: u32,
    over: u32,
    drawer: bool,
    drawer_w: u32,
    full: bool,
}

/// Opération sur les fenêtres, demandée pendant un évènement et faite à la fin de son traitement.
enum WinOp {
    /// Nouvelle fenêtre : vide (`tab` None) ou autour d'un onglet sorti de la courante ; `at` : position à l'écran.
    New { private: bool, tab: Option<u32>, at: Option<(i32, i32)> },
    /// Onglet de la courante déposé dans une autre fenêtre.
    Move { tab: u32, to: WindowId },
    /// Fermeture de la courante alors qu'il en reste d'autres.
    Close,
}

enum Ev {
    Ui(WindowId, String),
    Cmd(String), // commande d'interface envoyée par Volt lui-même (fil à part) : pour la fenêtre qui a parlé en dernier
    Key(Src, String),
    Title(u32, String),
    Load(u32, String, bool),
    Url(u32, String), // adresse changée sans rechargement
    Open(Option<u32>, String), // adresse à ouvrir : demandée par la page de cet onglet, ou (None) venue du dehors
    Exts(Value),
    ExtChanged,
    ExtFolder(PathBuf),
    Toast(String),
    Cpu(u8),
    Audio(u32, bool),
    Icon(u32, String), // icône du site de l'onglet
    Blocked(u32, String), // fenêtre demandée par la page sans geste de l'utilisateur
    Zoom(u32, f64),
    Bg(u32), // Ctrl+clic ou clic molette sur un lien de cet onglet
    Dls,     // la liste des téléchargements a changé (dl.rs)
    DlAsk(u32), // un téléchargement de cet onglet attend qu'on demande où l'enregistrer
    DlNote(u32, u32), // onglet, téléchargement : il vient de commencer ou de finir, à signaler par la petite notif
    Perms(Value), // autorisations données aux sites, pour la page Confidentialité
    Tasks(Value), // relevé du gestionnaire de tâches
    Sign(Result<Option<(String, String)>, String>), // « Remplir et signer » prêt : (nom, contenu en base64) du PDF à y ouvrir
    SignReady(u32),        // la page de signature de cet onglet attend son document
    SignSigs(u32, String), // elle envoie la liste des signatures à garder
    Fullscreen(u32, bool),
    State(u32, String),
    Raise, // un second Volt a été lancé : premier plan
    Cookies(Value),
    Lang(u32, String, String), // onglet, adresse, réponse de LANG_JS
    Update(Value), // état de la mise à jour pour l'interface (`upd`)
    PwForm(u32), // un formulaire de connexion a été repéré sur l'onglet (signal, sans donnée)
    Card(u32, String, bool), // champs de carte annoncés dans cet onglet (cc:form, cc:part, cc:touch) ; vrai : par la page elle-même, faux : par un cadre
    CardTarget(u32, Option<String>), // origine où remplir la carte dans cet onglet (None : aucun champ désigné par l'utilisateur)
    PwSave(u32, String), // formulaire envoyé : {u,p} en JSON ; le mot de passe ne va qu'ici, jamais journalisé
    Hello(Value, bool), // Windows Hello terminé pour l'action `cmd` : vrai = validé
    Vicon(String, String, String), // logo résolu pour une vignette : (url de la vignette, url de l'icône, couleur dominante)
}

impl Ev {
    /// `asked` : fenêtre dont l'interface a parlé en dernier (les réponses lui reviennent) ; `front` : dernière fenêtre
    /// normale regardée (un lien venu du dehors s'y ouvre).
    fn dest(&self, asked: WindowId, front: WindowId) -> Dest {
        match self {
            Ev::Ui(w, _) | Ev::Key(Src::Win(w), _) => Dest::Win(*w),
            Ev::Key(Src::Tab(t), _) | Ev::Title(t, _) | Ev::Load(t, ..) | Ev::Url(t, _) | Ev::Audio(t, _) | Ev::Icon(t, _) | Ev::Blocked(t, _) | Ev::Zoom(t, _) | Ev::Bg(t) | Ev::DlAsk(t) | Ev::DlNote(t, _) | Ev::SignReady(t) | Ev::SignSigs(t, _) | Ev::Fullscreen(t, _) | Ev::State(t, _) | Ev::Lang(t, ..) | Ev::PwForm(t) | Ev::PwSave(t, _) | Ev::Card(t, ..) | Ev::CardTarget(t, _) | Ev::Open(Some(t), _) => Dest::Tab(*t),
            Ev::Cmd(_) | Ev::Sign(_) | Ev::Perms(_) | Ev::Tasks(_) | Ev::Cookies(_) | Ev::Hello(..) | Ev::Vicon(..) | Ev::Toast(_) | Ev::ExtFolder(_) | Ev::Update(_) => Dest::Win(asked),
            Ev::Open(None, _) | Ev::Raise => Dest::Win(front),
            Ev::Exts(_) | Ev::ExtChanged | Ev::Cpu(_) | Ev::Dls => Dest::Here,
        }
    }
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
    icon: String,  // adresse de l'icône du site annoncée par la page ; vide : l'interface essaie /favicon.ico
    blocked: Vec<String>, // fenêtres surgissantes bloquées sur la page affichée (adresses), vidé à la page suivante
    zoom: f64,            // zoom de la page, 1.0 = 100 %
    bg: Option<Instant>,  // Ctrl+clic ou clic molette sur un lien à cet instant : la fenêtre demandée s'ouvre en arrière-plan
    group: u32,           // groupe d'onglets (voir `Group`) ; 0 : aucun
}

/// Groupe d'onglets : des onglets voisins sous une pastille nommée et colorée, qu'on replie d'un clic.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Group {
    id: u32,
    name: String,
    color: String,
    #[serde(default)]
    closed: bool, // replié : seule la pastille reste (et l'onglet affiché s'il en fait partie)
}

/// Nom d'un groupe dans un menu : le sien, sinon celui de sa couleur.
fn group_label(g: &Group) -> String {
    if !g.name.is_empty() {
        return format!("« {} »", g.name);
    }
    tabmenu::GROUP_COLORS.iter().find(|c| c.1 == g.color).map_or_else(|| lang::t("Groupe sans nom"), |c| lang::t(c.0))
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
/// Texte du presse-papiers Windows (None s'il est vide ou n'est pas du texte). Pour « Coller » et « Coller et rechercher ».
fn clipboard_text() -> Option<String> {
    use windows::Win32::{
        Foundation::{HANDLE, HGLOBAL},
        System::{
            DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard},
            Memory::{GlobalLock, GlobalUnlock},
        },
    };
    const CF_UNICODETEXT: u32 = 13;
    unsafe {
        OpenClipboard(None).ok()?;
        let r = (|| {
            let h: HANDLE = GetClipboardData(CF_UNICODETEXT).ok()?;
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g) as *const u16;
            if p.is_null() {
                return None;
            }
            let mut len = 0;
            while *p.add(len) != 0 {
                len += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
            let _ = GlobalUnlock(g);
            Some(s)
        })();
        let _ = CloseClipboard();
        r.filter(|s| !s.is_empty())
    }
}

fn to_url(input: &str, engine: &str) -> String {
    let t = input.trim();
    // chemin complet d'un fichier (collé depuis l'Explorateur, avec ou sans guillemets) : ouvert comme dans Chrome
    if let Some(u) = Some(t.trim_matches('"')).filter(|p| std::path::Path::new(p).is_absolute()).and_then(defaults::file_url) {
        return u;
    }
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
        "downloads" => "dl",
        "tasks" | "task-manager" => "tasks",
        "settings/payments" | "payments" => "cards",
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

/// Suggestions de la barre d'adresse, les meilleures d'abord : favoris et historique dont l'adresse ou le titre porte
/// chaque mot tapé. Tout est cherché ici, rien n'est envoyé à un moteur de recherche. Renvoie (adresse, titre, favori).
fn suggest(q: &str, favs: &[Page], hist: &[Visit], max: usize) -> Vec<(String, String, bool)> {
    let words: Vec<String> = q.to_lowercase().split_whitespace().map(String::from).collect();
    let Some(first) = words.first() else { return vec![] };
    // adresse comme on la tape : sans « https:// » ni « www. »
    let bare = |u: &str| {
        let l = u.to_lowercase();
        let l = l.split_once("://").map_or(l.as_str(), |x| x.1);
        l.strip_prefix("www.").unwrap_or(l).to_string()
    };
    let mut out: Vec<(i32, String, String, bool)> = vec![];
    // favoris d'abord : une page à la fois favorite et dans l'historique n'apparaît qu'une fois, comme favori
    let all = favs.iter().map(|f| (&f.url, &f.title, true, 0)).chain(hist.iter().enumerate().map(|(i, h)| (&h.url, &h.title, false, i)));
    for (url, title, fav, rank) in all {
        let (b, t) = (bare(url), title.to_lowercase());
        if out.iter().any(|o| &o.1 == url) || !words.iter().all(|w| b.contains(w.as_str()) || t.contains(w.as_str())) {
            continue;
        }
        // début de l'adresse (« you » -> youtube.com) > début d'un mot de l'adresse ou du titre > simple présence
        let begins = |s: &str| s.split(|c: char| !c.is_alphanumeric()).any(|p| p.starts_with(first.as_str()));
        let mut score = if b.starts_with(first.as_str()) { 100 } else if begins(&b) || begins(&t) { 60 } else { 30 };
        score += if fav { 15 } else { 0 };
        score -= (rank.min(200) / 10) as i32; // historique : le plus récent d'abord
        score -= (b.len().min(120) / 12) as i32; // à égalité, l'adresse la plus courte (l'accueil du site)
        out.push((score, url.clone(), title.clone(), fav));
    }
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.into_iter().take(max).map(|o| (o.1, o.2, o.3)).collect()
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

fn default_data_dir() -> PathBuf {
    PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into())).join("Volt")
}

fn data_dir() -> PathBuf {
    // VOLT_DATA : profil isolé (mesures A/B sans toucher aux données de l'utilisateur).
    if let Some(d) = std::env::var_os("VOLT_DATA") {
        return d.into();
    }
    default_data_dir()
}

/// Essais pilotés : sur un profil isolé (VOLT_DATA ailleurs que le profil normal), VOLT_NOHELLO évite la demande
/// Windows Hello, qu'un essai automatique ne peut pas valider. Sans effet sur le profil normal.
fn hello_skipped() -> bool {
    std::env::var_os("VOLT_NOHELLO").is_some() && std::env::var_os("VOLT_DATA").is_some_and(|d| PathBuf::from(d) != default_data_dir())
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
    let h = if panel || drawer_w() > 0.0 { s.height - b } else if full() { 0.0 } else { top().max(OVER.load(std::sync::atomic::Ordering::Relaxed) as f64) };
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
                let _ = p.send_event(Ev::Toast(lang::te("Téléchargement impossible : {e}", e)));
                let _ = p.send_event(Ev::ExtChanged); // le bouton « Installation… » de l'interface revient
            }
        }
    });
}

/// Au démarrage, en fond (réglage `auto_update`) : installe la dernière version sur le disque sans rien interrompre ;
/// elle sert au prochain lancement de Volt. L'interface affiche alors « prête, redémarrez » (état « ready »).
fn auto_update(p: EventLoopProxy<Ev>) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(25)); // laisser Volt démarrer avant de prendre de la bande passante
        if !update::begin() {
            return;
        }
        let r = update::install(); // Err (dont « déjà à jour ») : rien à signaler, l'ancien exe reste en place
        update::end();
        if let Ok(v) = r {
            let _ = p.send_event(Ev::Update(json!({"st": "ready", "v": v})));
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
            let _ = p.send_event(Ev::Cmd(json!({"cmd": "restart"}).to_string()));
        }
    });
}

fn save(s: &Settings, cfg: &std::path::Path) {
    let _ = fs::write(cfg, serde_json::to_vec_pretty(s).unwrap());
}

/// Liste du gestionnaire pour l'interface : origine, hôte, identifiant, note — JAMAIS le mot de passe (révélé à part, derrière Hello).
fn pw_manager_js(vault: &[vault::Entry]) -> String {
    let list: Vec<Value> = vault
        .iter()
        .enumerate()
        .map(|(i, e)| json!({"i": i, "host": e.origin.split_once("://").map_or(e.origin.as_str(), |x| x.1), "origin": e.origin, "user": e.username, "note": e.note}))
        .collect();
    format!("pwManager({})", json!(list))
}

/// Demande Windows Hello hors du fil de l'interface, puis renvoie le résultat par `Ev::Hello` pour exécuter `cmd`.
/// Hello indisponible (pas de PIN) : action bloquée, l'interface le dit. Toute erreur bloque (fail-closed).
fn ask_hello(proxy: EventLoopProxy<Ev>, message: String, cmd: Value) {
    std::thread::spawn(move || {
        if hello_skipped() {
            let _ = proxy.send_event(Ev::Hello(cmd, true));
            return;
        }
        if !vault::hello_available() {
            let _ = proxy.send_event(Ev::Toast(lang::t("Configurez un code PIN Windows (Windows Hello) : il protège vos mots de passe et vos cartes")));
            return;
        }
        let ok = vault::hello_verify(&message);
        let _ = proxy.send_event(Ev::Hello(cmd, ok));
    });
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

/// Les onglets de cette fenêtre deviennent la session à rouvrir (selon le mode de démarrage) : à la fermeture d'une
/// fenêtre normale. La dernière fermée l'emporte, comme dans Chrome.
fn remember_session(s: &mut Settings, tabs: &[Tab], groups: &[Group], restart: bool, offered: bool) {
    match on_close(&s.startup, restart, offered, untouched(tabs, &s.home)) {
        Close::Save => {
            s.session = tabs.iter().map(|t| t.url.clone()).collect();
            s.pinned = tabs.iter().filter(|t| t.pinned).count() as u32;
            s.session_groups = tabs.iter().map(|t| t.group).collect();
            s.groups = groups.iter().filter(|g| tabs.iter().any(|t| t.group == g.id)).cloned().collect();
        }
        Close::Forget => (s.session, s.pinned, s.session_groups, s.groups) = (vec![], 0, vec![], vec![]),
        Close::KeepOld => {}
    }
}

/// Groupe du n-ième onglet d'une session enregistrée (0 : aucun, ou groupe disparu).
fn saved_group(saved: &(Vec<u32>, Vec<Group>), n: usize) -> u32 {
    saved.0.get(n).copied().filter(|g| saved.1.iter().any(|x| x.id == *g)).unwrap_or(0)
}

/// Sauvegarde réglages, session et historique (fermeture de Volt). `tabs` None : fenêtre privée, la session n'est pas touchée.
fn persist(s: &mut Settings, tabs: Option<&[Tab]>, groups: &[Group], hist: &[Visit], cfg: &std::path::Path, hp: &std::path::Path, restart: bool, offered: bool) {
    if let Some(t) = tabs {
        remember_session(s, t, groups, restart, offered);
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

/// Mots de passe : Volt tient son propre coffre (vault.rs), l'enregistrement du moteur est coupé pour ne pas avoir deux
/// systèmes concurrents. Le reste des formulaires (nom, adresse, e-mail, téléphone) est retenu et proposé par le
/// moteur, selon le réglage `autofill`.
fn engine_forms(wv: &WebView) {
    if let Ok(st) = unsafe { wv.webview().Settings() }.and_then(|s| s.cast::<ICoreWebView2Settings4>()) {
        unsafe {
            let _ = st.SetIsPasswordAutosaveEnabled(false);
            let _ = st.SetIsGeneralAutofillEnabled(AUTOFILL.load(std::sync::atomic::Ordering::Relaxed));
        }
    }
}

/// Cartes pour l'interface : rang, réseau et quatre derniers chiffres, nom, échéance — JAMAIS le numéro.
fn cards_js(cards: &[cards::Card]) -> String {
    let list: Vec<Value> = cards.iter().enumerate().map(|(i, c)| json!({"i": i, "label": cards::label(c), "name": c.name, "exp": format!("{:02}/{:02}", c.month, c.year % 100)})).collect();
    format!("ccManager({})", json!(list))
}

/// Page de l'onglet pour le remplissage d'une carte : sa vue et son origine du moment.
fn card_main(t: &Tab) -> Option<(webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2, String)> {
    let w = t.wv.as_ref()?;
    Some((w.webview(), vault::origin_of(&w.url().ok()?)?))
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
fn make_tab(win: &Window, ctx: &mut WebContext, proxy: &EventLoopProxy<Ev>, id: u32, url: &str, html: Option<(String, (u8, u8, u8, u8))>, dark: bool, args: &str, cmp: &str, private: bool) -> (WebView, String) {
    let (p1, p2, p3, p4, p5) = (proxy.clone(), proxy.clone(), proxy.clone(), proxy.clone(), proxy.clone());
    // retour avec aperçu : fond du moteur à la couleur de la page, le passage de l'aperçu à la page ne montre plus de gris
    let bg = html.as_ref().map_or(LOADING_BG, |h| h.1);
    // rien à charger ici : la navigation part après l'ajout du script des bandeaux, pour qu'il vaille dès la première page
    let wv = WebViewBuilder::new_with_web_context(ctx)
        .with_additional_browser_args(args)
        .with_background_color(bg)
        .with_bounds(content_rect(win))
        .with_browser_extensions_enabled(true)
        .with_incognito(private)
        .with_devtools(true)
        .with_theme(if dark { Theme::Dark } else { Theme::Auto })
        .with_hotkeys_zoom(true)
        .with_initialization_script(KEY_JS)
        .with_initialization_script(BG_JS)
        .with_initialization_script(TYPED_JS)
        .with_initialization_script(PW_JS)
        .with_initialization_script(cards::watch_js())
        .with_document_title_changed_handler(move |t| {
            let _ = p1.send_event(Ev::Title(id, t));
        })
        .with_on_page_load_handler(move |e, url| {
            let _ = p2.send_event(Ev::Load(id, url, matches!(e, PageLoadEvent::Started)));
        })
        // gestionnaire vide : il fait exister `window.ipc` dans la page ; les messages sont lus par audio::watch_messages
        .with_ipc_handler(|_| {})
        .build_as_child(win)
        .expect("création de l'onglet");
    engine_forms(&wv);
    audio::engine_defaults(&wv);
    audio::watch_messages(&wv, move |b| {
        if KEYS.contains(&b.as_str()) {
            let _ = p4.send_event(Ev::Key(Src::Tab(id), b.clone()));
        } else if b == "bg" {
            let _ = p4.send_event(Ev::Bg(id));
        } else if b == "sign:ready" {
            let _ = p4.send_event(Ev::SignReady(id));
        } else if let Some(j) = b.strip_prefix("sign:sigs:") {
            let _ = p4.send_event(Ev::SignSigs(id, j.to_string()));
        } else if matches!(b.as_str(), "cc:form" | "cc:part" | "cc:touch") {
            let _ = p4.send_event(Ev::Card(id, b, true));
        } else if b == "pw:form" {
            let _ = p4.send_event(Ev::PwForm(id));
        } else if let Some(j) = b.strip_prefix("pw:save:") {
            let _ = p4.send_event(Ev::PwSave(id, j.to_string()));
        }
    });
    let p12 = proxy.clone();
    cards::watch_frames(&wv, id, move |m| {
        let _ = p12.send_event(Ev::Card(id, m.to_string(), false));
    });
    // nouvelle fenêtre demandée par la page : ouverte si elle vient d'un clic, bloquée (et signalée) sinon
    audio::watch_new_window(&wv, move |url, user| {
        let _ = p3.send_event(if user { Ev::Open(Some(id), url) } else { Ev::Blocked(id, url) });
    });
    let p9 = proxy.clone();
    audio::watch_zoom(&wv, move |z| {
        let _ = p9.send_event(Ev::Zoom(id, z));
    });
    let (p10, p11) = (proxy.clone(), proxy.clone());
    dl::watch(&wv, move |note| {
        let _ = p10.send_event(Ev::Dls); // d'abord la liste à jour, la notif y lit son fichier
        if let Some(n) = note {
            let _ = p10.send_event(Ev::DlNote(id, n));
        }
    }, move || {
        let _ = p11.send_event(Ev::DlAsk(id));
    });
    audio::watch(&wv, move |on| {
        let _ = p5.send_event(Ev::Audio(id, on));
    });
    let p6 = proxy.clone();
    audio::watch_fullscreen(&wv, move |on| {
        let _ = p6.send_event(Ev::Fullscreen(id, on));
    });
    let p8 = proxy.clone();
    audio::watch_icon(&wv, move |u| {
        let _ = p8.send_event(Ev::Icon(id, u));
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

/// Fenêtre de Volt, sans barre de titre (l'interface la remplace) ; `at` : position à l'écran, None : celle de Windows.
fn make_window(target: &EventLoopWindowTarget<Ev>, at: Option<PhysicalPosition<i32>>) -> Window {
    let b = WindowBuilder::new()
        .with_title("Volt")
        // Volty 64x64 en RGBA brut, généré par logos/make_icon.py (pas de décodeur PNG à embarquer)
        .with_window_icon(Icon::from_rgba(include_bytes!("../logos/volty64.rgba").to_vec(), 64, 64).ok())
        .with_theme(Some(WinTheme::Dark))
        .with_decorations(false)
        .with_undecorated_shadow(true)
        .with_background_color((11, 12, 16, 255))
        .with_inner_size(tao::dpi::LogicalSize::new(1280.0, 800.0));
    match at {
        Some(p) => b.with_position(p),
        None => b,
    }
    .build(target)
    .expect("fenêtre")
}

/// Interface d'une fenêtre (onglets, barre d'adresse, réglages). `private` : hors du profil comme ses onglets, pour que
/// les icônes de sites qu'elle charge ne laissent rien dans le profil normal.
fn make_ui(win: &Window, ctx: &mut WebContext, proxy: &EventLoopProxy<Ev>, args: &str, ext_dir: &std::path::Path, private: bool) -> WebView {
    let (p, pn, wid) = (proxy.clone(), proxy.clone(), win.id());
    // Le premier webview fixe l'environnement : mêmes options (extensions activées) partout.
    WebViewBuilder::new_with_web_context(ctx)
        .with_html(lang::ui(UI, ""))
        // l'interface ne part jamais ailleurs : un fichier ou un lien déposé dessus (page de favoris, barre d'onglets)
        // s'ouvre dans un onglet au lieu de la remplacer
        .with_navigation_handler(move |u| match defaults::msg_url(&u) {
            Some(u) => {
                let _ = pn.send_event(Ev::Open(None, u));
                false
            }
            None => true,
        })
        .with_additional_browser_args(args)
        .with_background_color(LOADING_BG)
        .with_bounds(ui_rect(win, false))
        .with_browser_extensions_enabled(true)
        .with_extensions_path(ext_dir)
        .with_incognito(private)
        .with_ipc_handler(move |r| {
            let b = r.body().clone();
            let _ = p.send_event(if KEYS.contains(&b.as_str()) { Ev::Key(Src::Win(wid), b) } else { Ev::Ui(wid, b) });
        })
        .build_as_child(win)
        .expect("interface")
}

/// `volt.exe --uninstall` (bouton Désinstaller de Windows) : demande confirmation, retire raccourcis et enregistrement de
/// cet exe, les données si l'utilisateur le veut, puis l'exe lui-même.
fn uninstall(dir: &std::path::Path, pipe: &str) {
    let sys = lang::system();
    lang::set(&load::<Settings>(&dir.join("settings.json")).lang, &sys);
    let say = |text: String, kind| unsafe { MessageBoxW(None, &windows::core::HSTRING::from(text), windows::core::w!("Volt"), kind) };
    if defaults::alive(pipe) {
        say(lang::t("Fermez d'abord Volt, puis relancez la désinstallation."), MB_OK | MB_ICONINFORMATION);
        return;
    }
    // une traduction par ligne : la garde des traductions lit les textes tels qu'écrits ici, sans retour à la ligne dedans
    let ask = [
        lang::t("Désinstaller Volt de ce PC ?"),
        String::new(),
        lang::t("Oui : supprimer aussi vos données (favoris, historique, mots de passe, réglages)."),
        lang::t("Non : désinstaller en gardant vos données."),
        lang::t("Annuler : ne rien faire."),
    ];
    let r = say(ask.join("
"), MB_YESNOCANCEL | MB_ICONQUESTION);
    if r == IDCANCEL || r.0 == 0 {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    defaults::uninstall(&exe.to_string_lossy(), defaults::APP, &defaults::shortcut_dirs(), (r == IDYES).then_some(dir));
    say(lang::t("Volt est désinstallé."), MB_OK | MB_ICONINFORMATION);
    // l'exe ne peut pas se supprimer pendant qu'il tourne : une commande le fait dans quelques secondes, puis retire son
    // dossier s'il est vide
    use std::os::windows::process::CommandExt;
    let folder = exe.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let _ = std::process::Command::new("cmd")
        .raw_arg(format!("/c ping -n 3 127.0.0.1 >nul & del /q \"{}\" & rmdir \"{folder}\"", exe.display()))
        .creation_flags(0x0800_0000) // sans fenêtre
        .spawn();
}

fn main() {
    let dir = data_dir();
    let ext_dir = dir.join("extensions");
    let cfg_path = dir.join("settings.json");
    if std::env::args().any(|a| a == "--uninstall") {
        return uninstall(&dir, &defaults::pipe_name(&dir));
    }
    let _ = fs::create_dir_all(&ext_dir);
    let url_arg = defaults::arg_url(std::env::args().skip(1));
    let pipe = defaults::pipe_name(&dir);
    let restarting = std::env::args().any(|a| a == "--restart");
    // Lien cliqué ailleurs alors qu'un Volt tient déjà ce profil : on lui passe l'adresse tout de suite, avant de
    // charger quoi que ce soit, et sans toucher à ses fichiers (ses aperçus d'onglets étaient effacés ici).
    if !restarting && defaults::alive(&pipe) && defaults::send(&pipe, url_arg.as_deref()) {
        return;
    }
    // Langue de Windows d'abord : les valeurs par défaut d'un profil neuf (cartes « Préférés ») sont dans cette langue ;
    // puis celle du réglage. `elang` : langue imposée au moteur à ce lancement, il n'en change qu'au redémarrage.
    let sys = lang::system();
    lang::set("auto", &sys);
    let mut s: Settings = load(&cfg_path);
    lang::set(&s.lang, &sys);
    let elang = lang::engine(&s.lang, &sys);
    // Bienvenue tant que « Commencer » n'a pas été cliqué : un profil créé avant cette page la voit aussi une fois.
    let mut first = !s.welcomed;
    migrate(&mut s);
    FAVH.store(if s.favbar { FAVBAR_H } else { 0 }, std::sync::atomic::Ordering::Relaxed);
    let hist_path = dir.join("history.json");
    let mut hist: Vec<Visit> = load(&hist_path);
    // Coffre de mots de passe : chiffré par DPAPI sur le disque, déchiffré ici en mémoire seulement.
    let vault_path = dir.join("passwords.dat");
    let mut vault = vault::load(&vault_path);
    // Cartes de paiement : même protection, autre fichier.
    let cards_path = dir.join("cards.dat");
    let mut cards = cards::load(&cards_path);
    AUTOFILL.store(s.autofill, std::sync::atomic::Ordering::Relaxed);

    let ev_loop = EventLoopBuilder::<Ev>::with_user_event().build();
    let proxy = ev_loop.create_proxy();
    let on_msg = |p: EventLoopProxy<Ev>| {
        move |m: String| {
            // la fenêtre passe devant d'abord : créer l'onglet prend un instant, pendant lequel rien ne bougeait
            let _ = p.send_event(Ev::Raise);
            if let Some(u) = defaults::msg_url(&m) {
                let _ = p.send_event(Ev::Open(None, u));
            }
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
    let _ = fs::remove_dir_all(previews()); // restes d'un arrêt brutal ; seulement ici, une fois sûr d'être le seul Volt du profil
    update::cleanup(); // ancien exe laissé par une mise à jour
    defaults::refresh(); // « Ouvrir avec » des PDF et pages : ajouté à un enregistrement fait par une version d'avant
    let mut win = make_window(&ev_loop, None);

    let mut ctx = WebContext::new(Some(dir.join("profile")));
    let (cores, ram_gb) = perf::machine();
    // VOLT_ARGS : arguments Chromium en plus, pour les mesures A/B sans recompiler (comme VOLT_EAGER).
    let args = perf::browser_args(cores, ram_gb, s.night, s.game_mode, s.safe_browsing, elang, &std::env::var("VOLT_ARGS").unwrap_or_default());
    let mut ui = make_ui(&win, &mut ctx, &proxy, &args, &ext_dir, false);
    if !s.download_dir.is_empty() {
        audio::set_download_dir(&ui, &s.download_dir);
    }
    dl::ASK.store(s.download_ask, std::sync::atomic::Ordering::Relaxed);
    if s.auto_update {
        auto_update(proxy.clone());
    }
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
    let mut fresh = !restored && url_arg.is_none(); // démarrage sans session ni adresse : la page de favoris s'ouvre
    // lancé par un lien sans session à rouvrir : le lien seul, pas la page d'accueil à côté
    let start = if restored { s.session.clone() } else if url_arg.is_some() { vec![] } else { vec![s.home.clone()] };
    let mut next_id = 0u32;
    let mut tabs: Vec<Tab> = vec![];
    // groupes d'onglets de toutes les fenêtres ; ceux de la session enregistrée reviennent avec elle (tout de suite, ou
    // à la reprise proposée) : gardés à part, car la session enregistrée peut être réécrite entre-temps
    let saved_groups = (s.session_groups.clone(), s.groups.clone());
    let mut groups: Vec<Group> = if restored { s.groups.clone() } else { vec![] };
    let mut next_group = s.groups.iter().map(|g| g.id).max().unwrap_or(0);
    let mut gedit: Option<(WindowId, u32)> = None; // groupe tout juste créé : son nom se saisit aussitôt dans cette fenêtre
    // Restauration paresseuse : seul le 1er onglet charge, les autres au premier clic.
    for u in &start {
        next_id += 1;
        // VOLT_EAGER : charge d'un coup les `keep_loaded` premiers (tests de charge ; tout charger a fait planter la machine).
        let eager = std::env::var_os("VOLT_EAGER").is_some() && (s.keep_loaded == 0 || next_id <= s.keep_loaded);
        // URL reçue en argument : c'est elle qu'on affiche, inutile de charger le 1er onglet restauré
        let (wv, cmp) = ((next_id == 1 && url_arg.is_none()) || eager).then(|| make_tab(&win, &mut ctx, &proxy, next_id, u, None, s.dark_web, &args, &cmp_js(&s), false)).unzip();
        let title = hist.iter().find(|h| &h.url == u).map_or(u.clone(), |h| h.title.clone());
        let (pinned, n) = (restored && next_id <= s.pinned, next_id as usize - 1);
        tabs.push(Tab { id: next_id, wv, title, url: u.clone(), loading: next_id == 1 && url_arg.is_none(), audio: false, muted: false, seen: Instant::now(), frozen: false, typed: false, saved: None, pinned, cmp: cmp.unwrap_or_default(), foreign: false, icon: String::new(), blocked: vec![], zoom: 1.0, bg: None, group: if restored && !pinned { saved_group(&saved_groups, n) } else { 0 } });
    }
    let mut active = tabs.first().map_or(0, |t| t.id);
    if let Some(u) = &url_arg {
        next_id += 1;
        let (wv, cmp) = make_tab(&win, &mut ctx, &proxy, next_id, u, None, s.dark_web, &args, &cmp_js(&s), false);
        tabs.push(Tab { id: next_id, wv: Some(wv), title: u.clone(), url: u.clone(), loading: true, audio: false, muted: false, seen: Instant::now(), frozen: false, typed: false, saved: None, pinned: false, cmp, foreign: false, icon: String::new(), blocked: vec![], zoom: 1.0, bg: None, group: 0 });
        active = next_id;
    }
    let mut panel = false;
    // autres fenêtres (voir Parked) ; la courante est-elle privée ; opération sur les fenêtres en attente ;
    // fenêtre dont l'interface a parlé en dernier, et dernière fenêtre normale regardée
    let mut parked: Vec<Parked> = vec![];
    let mut private = false;
    let mut winop: Option<WinOp> = None;
    let (mut asked, mut front) = (win.id(), win.id());
    let mut next_tick = Instant::now() + TICK;
    let mut dirty = true;
    let mut minimized_at: Option<Instant> = None;
    let mut cursor = (0.0, 0.0);
    let mut closed: Vec<(usize, String)> = vec![]; // (position, adresse) des onglets fermés, pour « Rouvrir »
    let mut ext_at: Option<Instant> = None; // dernière installation d'extension (page d'accueil en arrière-plan)
    let mut sign_docs: HashMap<u32, (String, String)> = HashMap::new(); // PDF à remettre à la page de signature de cet onglet
    let mut pw_forms: HashSet<u32> = HashSet::new(); // onglets où un formulaire de connexion est repéré
    // onglets où un champ de numéro de carte est annoncé : par la page ou un de ses cadres (l'icône s'affiche), et par
    // la page elle-même (elle peut alors être remplie sans cadre désigné)
    let (mut cc_forms, mut cc_main): (HashSet<u32>, HashSet<u32>) = (HashSet::new(), HashSet::new());
    let mut pending_save: Option<vault::Entry> = None; // identifiant envoyé, en attente du clic « Enregistrer » (jamais sur le disque)

    ev_loop.run(move |event, target, flow| {
        *flow = ControlFlow::WaitUntil(next_tick);

        // Charge une fenêtre qui attendait à la place de la courante (qui prend sa place dans `parked`).
        macro_rules! swap_win {
            ($p:expr) => {{
                let p: &mut Parked = $p;
                std::mem::swap(&mut win, &mut p.win);
                std::mem::swap(&mut ui, &mut p.ui);
                std::mem::swap(&mut tabs, &mut p.tabs);
                std::mem::swap(&mut active, &mut p.active);
                std::mem::swap(&mut panel, &mut p.panel);
                std::mem::swap(&mut minimized_at, &mut p.minimized_at);
                std::mem::swap(&mut cursor, &mut p.cursor);
                std::mem::swap(&mut offer, &mut p.offer);
                std::mem::swap(&mut first, &mut p.first);
                std::mem::swap(&mut fresh, &mut p.fresh);
                std::mem::swap(&mut private, &mut p.private);
                p.favh = FAVH.swap(p.favh, std::sync::atomic::Ordering::Relaxed);
                p.over = OVER.swap(p.over, std::sync::atomic::Ordering::Relaxed);
                p.drawer = DRAWER.swap(p.drawer, std::sync::atomic::Ordering::Relaxed);
                p.drawer_w = DRAWER_W.swap(p.drawer_w, std::sync::atomic::Ordering::Relaxed);
                p.full = FULL.swap(p.full, std::sync::atomic::Ordering::Relaxed);
            }};
        }
        // L'évènement vise-t-il une fenêtre qui attend ? Elle devient la courante avant tout le reste.
        let to = match &event {
            Event::WindowEvent { window_id, .. } if *window_id != win.id() => match parked.iter().position(|p| p.win.id() == *window_id) {
                Some(i) => Some(i),
                None => return, // fenêtre déjà fermée
            },
            Event::UserEvent(e) => match e.dest(asked, front) {
                Dest::Win(w) => parked.iter().position(|p| p.win.id() == w),
                Dest::Tab(t) => parked.iter().position(|p| p.tabs.iter().any(|x| x.id == t)),
                Dest::Here => None,
            },
            _ => None,
        };
        if let Some(i) = to {
            swap_win!(&mut parked[i]);
        }

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
        // message accordé selon un nombre (« un|plusieurs ») : traduit et accordé par l'interface, qui a les règles de pluriel de chaque langue
        let toast_n = |m: &str, n: usize| {
            let _ = ui.evaluate_script(&format!("toast(T({},{{n:{n}}}))", json!(m)));
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
            let (wv, cmp) = make_tab(&win, &mut ctx, &proxy, next_id, url, None, s.dark_web, &args, &cmp_js(&s), private);
            tabs.push(Tab { id: next_id, wv: Some(wv), title: lang::t("Nouvel onglet"), url: url.into(), loading: true, audio: false, muted: false, seen: Instant::now(), frozen: false, typed: false, saved: None, pinned: false, cmp, foreign: false, icon: String::new(), blocked: vec![], zoom: 1.0, bg: None, group: 0 });
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
                let mut changed = false;
                // chaque fenêtre à son tour : la courante, puis celles qui attendent
                for i in 0..=parked.len() {
                let min_long = minimized_at.is_some_and(|m| now - m >= FREEZE_MINIMIZED);
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
                    && !private
                    && let Some(w) = tabs.iter().find(|t| t.id == active && !t.loading).and_then(|t| t.wv.as_ref())
                {
                    preview::capture(w, &preview_of(active));
                }
                if i < parked.len() {
                    swap_win!(&mut parked[i]);
                }
                }
                dirty |= changed;
            }
            Event::WindowEvent { event: WindowEvent::Resized(_), .. } => {
                let m = win.is_minimized();
                if m != minimized_at.is_some() {
                    minimized_at = m.then(Instant::now);
                    // priorité basse seulement quand plus aucune fenêtre n'est affichée
                    perf::background(&ui, m && parked.iter().all(|p| p.minimized_at.is_some()));
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
                if !private {
                    front = win.id();
                }
                let _ = ui.evaluate_script(&format!("setDef({})", defaults::is_default()));
            }
            // dernière fenêtre : Volt se ferme ; sinon seule cette fenêtre part (voir WinOp::Close)
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                if parked.is_empty() {
                    persist(&mut s, (!private).then_some(&tabs[..]), &groups, &hist, &cfg_path, &hist_path, false, offer_n(&offer) > 0);
                    *flow = ControlFlow::Exit;
                } else {
                    winop = Some(WinOp::Close);
                }
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
                let e = match e {
                    Ev::Cmd(b) => Ev::Ui(win.id(), b),
                    e => e,
                };
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
                        if l {
                            pw_forms.remove(&id); // nouvelle page : le repère du formulaire est remis par PW_JS si besoin
                            cc_forms.remove(&id);
                            cc_main.remove(&id);
                            if let Some(t) = tabs.iter_mut().find(|x| x.id == id) {
                                t.blocked.clear();
                            }
                        }
                        if let Some(tab) = tabs.iter_mut().find(|x| x.id == id) {
                            if !l && !private && url.starts_with("http") {
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
                            } else if translate::is_refused(&url)
                                && let Some(w) = &tab.wv
                            {
                                // Google refuse cette page au relais (page anti-robot sans issue) : retour à l'original, et le
                                // site n'est plus traduit automatiquement, sinon il y repartirait aussitôt
                                match translate::refused_origin(&url) {
                                    Some(o) => {
                                        let _ = w.load_url(&o);
                                        if let Some(h) = cmp::site(&o).filter(|h| !s.no_translate.contains(h)) {
                                            s.no_translate.push(h);
                                            save(&s, &cfg_path);
                                        }
                                    }
                                    None => {
                                        let _ = w.evaluate_script("history.back()");
                                    }
                                }
                                toast(&lang::t("Google ne permet pas de traduire cette page"));
                            } else if url.starts_with("http")
                                && !translate::is_relay(&url)
                                && let Some(w) = &tab.wv
                            {
                                let (p, u) = (proxy.clone(), url.clone());
                                let _ = w.evaluate_script_with_callback(&translate::lang_js(), move |r| {
                                    let _ = p.send_event(Ev::Lang(id, u.clone(), r));
                                });
                            }
                            tab.url = url;
                            tab.loading = l;
                            if !l && id == active && !panel && !private && minimized_at.is_none()
                                && let Some(w) = &tab.wv
                            {
                                preview::capture(w, &preview_of(id));
                            }
                        }
                    }
                    // la vidéo affichée devient l'adresse de l'onglet (barre, étoile, session, restauration) et une visite
                    Ev::Url(id, url) => {
                        let Some(tab) = tabs.iter_mut().find(|x| x.id == id).filter(|t| t.url != url && url.starts_with("http")) else { return };
                        if !private {
                            add_visit(&mut hist, &url, &tab.title);
                            if DRAWER.load(std::sync::atomic::Ordering::Relaxed) {
                                let _ = ui.evaluate_script(&format!("renderHist({})", json!(hist)));
                            }
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
                            && translate::can_relay(&url)
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
                    Ev::Cmd(_) => {} // déjà changé en Ev::Ui ci-dessus
                    Ev::Fullscreen(_, on) => {
                        FULL.store(on, std::sync::atomic::Ordering::Relaxed);
                        win.set_fullscreen(on.then_some(Fullscreen::Borderless(None)));
                    }
                    Ev::Icon(id, u) => {
                        if let Some(tab) = tabs.iter_mut().find(|x| x.id == id) {
                            tab.icon = u;
                        }
                    }
                    Ev::Audio(id, on) => {
                        if let Some(tab) = tabs.iter_mut().find(|x| x.id == id) {
                            tab.audio = on;
                        }
                    }
                    // page ouverte par une extension (accueil à l'installation, parfois sur son site) : en arrière-plan, l'onglet regardé reste
                    // shortcut: un lien ouvert par l'utilisateur dans les 15 s d'une installation part aussi en arrière-plan ; suffisant tant que le moteur ne dit pas qui ouvre
                    Ev::Open(_, url) if url.starts_with("chrome-extension://") || ext_at.is_some_and(|t| t.elapsed() < EXT_WELCOME) => open(&url, &mut tabs, &mut active.clone()),
                    // lien ouvert par Ctrl+clic ou clic molette : en arrière-plan, on reste sur la page
                    Ev::Open(from, url) => {
                        let bg = from.and_then(|id| tabs.iter_mut().find(|t| t.id == id)).and_then(|t| t.bg.take()).is_some_and(|at| at.elapsed() < Duration::from_millis(1500));
                        if bg { open(&url, &mut tabs, &mut active.clone()) } else { open(&url, &mut tabs, &mut active) }
                    }
                    Ev::Bg(id) => {
                        if let Some(t) = tabs.iter_mut().find(|x| x.id == id) {
                            t.bg = Some(Instant::now());
                        }
                        return;
                    }
                    Ev::Zoom(id, z) => {
                        if let Some(t) = tabs.iter_mut().find(|x| x.id == id) {
                            t.zoom = z;
                        }
                    }
                    // fenêtre demandée par un script, sans geste : bloquée et signalée dans la barre d'adresse, sauf site autorisé
                    Ev::Blocked(id, url) => {
                        let Some(i) = tabs.iter().position(|t| t.id == id) else { return };
                        if cmp::site(&tabs[i].url).is_some_and(|h| cmp::covers(&h, &s.popup_allow)) {
                            open(&url, &mut tabs, &mut active.clone());
                        } else {
                            if tabs[i].blocked.is_empty() && id == active {
                                toast(&lang::t("Fenêtre surgissante bloquée"));
                            }
                            if tabs[i].blocked.len() < 20 && !tabs[i].blocked.contains(&url) {
                                tabs[i].blocked.push(url);
                            }
                        }
                    }
                    Ev::Raise => {
                        win.set_minimized(false);
                        win.set_focus();
                    }
                    Ev::Key(_, k) => {
                        let i = tabs.iter().position(|t| t.id == active).unwrap_or(0);
                        match k.as_str() {
                            // Ctrl+N : nouvelle fenêtre ; Ctrl+Maj+N : fenêtre de navigation privée
                            "cn" => winop = Some(WinOp::New { private: false, tab: None, at: None }),
                            "csn" => winop = Some(WinOp::New { private: true, tab: None, at: None }),
                            "ct" => {
                                let h = s.home.clone();
                                open(&h, &mut tabs, &mut active)
                            }
                            "cw" => shut.push(active),
                            "cst" => {
                                let _ = proxy.send_event(Ev::Ui(win.id(), json!({"cmd": "tabdo", "id": active, "action": "reopen"}).to_string()));
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
                            // recherche dans la page, panneau des téléchargements : ceux du moteur, sur l'onglet affiché
                            "cf" => {
                                if let Some(w) = tabs.get(i).and_then(|t| t.wv.as_ref()) {
                                    audio::find(w);
                                }
                                return;
                            }
                            "cj" => {
                                let _ = ui.evaluate_script("openSection('dl')");
                                return;
                            }
                            // Maj+Échap : gestionnaire de tâches
                            "sescape" => {
                                let _ = ui.evaluate_script("openSection('tasks')");
                                return;
                            }
                            // Ctrl+Maj+A : recherche dans les onglets ouverts, dans la barre d'adresse
                            "csa" => {
                                let _ = ui.focus();
                                let _ = ui.evaluate_script("tabSearch()");
                                return;
                            }
                            // F11 : la page seule, sans la barre du haut ; F11 de nouveau pour revenir
                            "f11" => {
                                let on = !full();
                                FULL.store(on, std::sync::atomic::Ordering::Relaxed);
                                win.set_fullscreen(on.then_some(Fullscreen::Borderless(None)));
                                if let Some(w) = tabs.get(i).and_then(|t| t.wv.as_ref()) {
                                    let _ = w.focus(); // la page garde le clavier : c'est elle qui reçoit le F11 de sortie
                                }
                            }
                            // Ctrl+1 à 8 : l'onglet de ce rang ; Ctrl+9 : le dernier
                            n if n.len() == 2 && matches!(n.as_bytes()[1], b'1'..=b'9') => {
                                let n = (n.as_bytes()[1] - b'1') as usize;
                                active = if n == 8 { tabs.last() } else { tabs.get(n) }.map_or(active, |t| t.id);
                            }
                            _ => {
                                let _ = ui.evaluate_script("toggleDrawer()"); // "ch"
                                return;
                            }
                        }
                    }
                    Ev::Exts(v) => {
                        // toutes les fenêtres montrent les mêmes extensions
                        for u in std::iter::once(&ui).chain(parked.iter().map(|p| &p.ui)) {
                            let _ = u.evaluate_script(&format!("renderExts({v})"));
                        }
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
                                Ok(()) => lang::t("Extension installée ✔"),
                                Err(e) => lang::te("Échec de l'installation : {e}", e),
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
                    Ev::Vicon(url, icon, color) => {
                        let _ = ui.evaluate_script(&format!("viconReady({},{},{})", json!(url), json!(icon), json!(color)));
                        return;
                    }
                    Ev::Update(v) => {
                        let _ = ui.evaluate_script(&format!("upd({v})"));
                        return;
                    }
                    // « Remplir et signer » installé (et le PDF lu) : la page s'ouvre dans un nouvel onglet
                    Ev::Sign(r) => match r {
                        Err(e) => {
                            toast(&e);
                            return;
                        }
                        Ok(doc) => {
                            let Some(page) = sign::page_url(&dir) else { return };
                            open(&format!("{page}?lang={}&a={}", lang::code(), enc(&s.accent)), &mut tabs, &mut active);
                            if let Some(d) = doc {
                                sign_docs.insert(active, d);
                            }
                        }
                    },
                    // la page est prête. Elle reçoit les signatures gardées et son document ; une autre page qui enverrait
                    // ce message n'obtient rien : seule l'adresse de la page de signature de ce profil est servie.
                    Ev::SignReady(id) | Ev::SignSigs(id, _) if !tabs.iter().any(|t| t.id == id && sign::page_url(&dir).is_some_and(|p| t.url.starts_with(&p))) => return,
                    Ev::SignReady(id) => {
                        if let Some(w) = tabs.iter().find(|t| t.id == id).and_then(|t| t.wv.as_ref()) {
                            let _ = w.evaluate_script(&format!("window.voltSigs={}", sign::sigs(&dir)));
                            if let Some((name, data)) = sign_docs.remove(&id) {
                                let _ = w.evaluate_script(&format!("voltOpen({})", json!({"name": name, "data": data})));
                            }
                        }
                        return;
                    }
                    Ev::SignSigs(_, j) => {
                        if !sign::save_sigs(&dir, &j) {
                            toast(&lang::t("Signature non enregistrée"));
                        }
                        return;
                    }
                    Ev::Perms(v) => {
                        let _ = ui.evaluate_script(&format!("perms({v})"));
                        return;
                    }
                    Ev::Tasks(v) => {
                        let _ = ui.evaluate_script(&format!("tasks({v})"));
                        return;
                    }
                    Ev::DlAsk(_) => {
                        dl::ask_waiting(win.hwnd());
                        return;
                    }
                    Ev::DlNote(_, n) => {
                        let _ = ui.evaluate_script(&format!("dlNote({n})"));
                        return;
                    }
                    Ev::Dls => {
                        let v = dl::list();
                        for u in std::iter::once(&ui).chain(parked.iter().map(|p| &p.ui)) {
                            let _ = u.evaluate_script(&format!("dls({v})"));
                        }
                        return;
                    }
                    Ev::Cpu(c) => {
                        for u in std::iter::once(&ui).chain(parked.iter().map(|p| &p.ui)) {
                            let _ = u.evaluate_script(&format!("volty({c})"));
                        }
                        return;
                    }
                    // champ de numéro de carte annoncé : l'icône de la barre d'adresse (simple indication, une page peut la
                    // fabriquer : ce qui décide du remplissage est relu par Volt dans la page, voir cards.rs)
                    Ev::Card(id, m, main) => {
                        if m != "cc:form" || !(cc_forms.insert(id) | (main && cc_main.insert(id))) {
                            return;
                        }
                    }
                    // origine désignée pour la carte : on la montre, avec le choix de la carte, puis Windows Hello
                    Ev::CardTarget(id, target) => {
                        let Some(o) = target.filter(|_| id == active) else {
                            if id == active {
                                toast(&lang::t("Cliquez d'abord dans le champ du numéro de carte, puis sur cette icône"));
                            }
                            return;
                        };
                        let host = o.split_once("://").map_or(o.as_str(), |x| x.1).to_string();
                        let mut rows = vec![Some((lang::t("Remplir sur {x} :").replace("{x}", &host), true)), None];
                        rows.extend(cards.iter().map(|c| Some((tabmenu::label(&format!("{}  {}", cards::label(c), c.name), ""), false))));
                        let Some(i) = tabmenu::popup(win.hwnd(), &rows, None).and_then(|k| k.checked_sub(2)) else { return };
                        ask_hello(proxy.clone(), lang::t("Remplir votre carte sur {x}").replace("{x}", &host), json!({"do": "cc_fill", "i": i, "tab": id, "target": o}));
                        return;
                    }
                    // formulaire de connexion repéré : propose le remplissage si le coffre a l'origine exacte (fait dans sync)
                    Ev::PwForm(id) => {
                        pw_forms.insert(id);
                        dirty = true;
                        return;
                    }
                    // formulaire envoyé : propose d'enregistrer si (origine, identifiant) est nouveau. Le mot de passe ne va qu'ici.
                    Ev::PwSave(id, j) => {
                        if !s.save_passwords {
                            return;
                        }
                        let (Some(url), Ok(cred)) = (tabs.iter().find(|t| t.id == id).map(|t| t.url.clone()), serde_json::from_str::<Value>(&j)) else { return };
                        let (Some(origin), p) = (vault::origin_of(&url), cred["p"].as_str().unwrap_or_default().to_string()) else { return };
                        let u = cred["u"].as_str().unwrap_or_default().to_string();
                        if p.is_empty() || vault.iter().any(|e| e.origin == origin && e.username == u) {
                            return; // vide, ou couple déjà enregistré
                        }
                        let _ = ui.evaluate_script(&format!("pwOffer({})", json!(u)));
                        pending_save = Some(vault::Entry { origin, username: u, password: p, note: String::new() });
                        return;
                    }
                    // Windows Hello terminé : exécute l'action protégée seulement si validé
                    Ev::Hello(cmd, ok) => {
                        if !ok {
                            toast(&lang::t("Déverrouillage annulé"));
                            return;
                        }
                        let i = cmd["i"].as_u64().map(|i| i as usize);
                        match cmd["do"].as_str().unwrap_or_default() {
                            "reveal" => {
                                if let Some(e) = i.and_then(|i| vault.get(i)) {
                                    let _ = ui.evaluate_script(&format!("pwShow({},{})", i.unwrap(), json!(e.password)));
                                }
                            }
                            "edit" => {
                                let origin = vault::origin_of(cmd["origin"].as_str().unwrap_or_default());
                                match (i.filter(|&i| i < vault.len()), origin) {
                                    (Some(i), Some(origin)) => {
                                        let pass = cmd["pass"].as_str().unwrap_or_default();
                                        // mot de passe laissé vide : on garde l'ancien (pas d'effacement accidentel)
                                        let password = if pass.is_empty() { vault[i].password.clone() } else { pass.into() };
                                        vault[i] = vault::Entry { origin, username: cmd["user"].as_str().unwrap_or_default().into(), password, note: cmd["note"].as_str().unwrap_or_default().into() };
                                        vault::save(&vault_path, &vault);
                                        let _ = ui.evaluate_script(&pw_manager_js(&vault));
                                    }
                                    _ => toast(&lang::t("Adresse invalide (exemple : https://site.fr)")),
                                }
                            }
                            "del" => {
                                if let Some(i) = i.filter(|&i| i < vault.len()) {
                                    vault.remove(i);
                                    vault::save(&vault_path, &vault);
                                    let _ = ui.evaluate_script(&pw_manager_js(&vault));
                                    toast(&lang::t("Mot de passe supprimé"));
                                }
                            }
                            "cc_del" => {
                                if let Some(i) = i.filter(|&i| i < cards.len()) {
                                    cards.remove(i);
                                    cards::save(&cards_path, &cards);
                                    let _ = ui.evaluate_script(&cards_js(&cards));
                                    toast(&lang::t("Carte supprimée"));
                                    dirty = true;
                                }
                            }
                            // la carte part vers l'origine qui a été montrée, et seulement elle
                            "cc_fill" => {
                                let tab = cmd["tab"].as_u64().unwrap_or(0) as u32;
                                let (Some(c), Some(target), Some(t)) = (i.and_then(|i| cards.get(i)), cmd["target"].as_str(), tabs.iter().find(|t| t.id == tab)) else { return };
                                toast(&if cards::fill(tab, target, c, card_main(t)) > 0 {
                                    lang::t("Carte remplie : vérifiez, puis tapez le cryptogramme")
                                } else {
                                    lang::t("Le formulaire de paiement a changé : recommencez")
                                });
                            }
                            "export" => match vault::pick_save(win.hwnd()) {
                                Some(path) => {
                                    if fs::write(&path, vault::to_csv(&vault)).is_ok() {
                                        toast_n("{n} identifiant exporté|{n} identifiants exportés", vault.len());
                                    } else {
                                        toast(&lang::t("Écriture du fichier impossible"));
                                    }
                                }
                                None => {}
                            },
                            _ => {}
                        }
                        return;
                    }
                    Ev::Ui(wid, body) => {
                        asked = wid;
                        let Ok(v) = serde_json::from_str::<Value>(&body) else { return };
                        let str_of = |k: &str| v[k].as_str().unwrap_or_default().to_string();
                        let wv = tabs.iter().find(|t| t.id == active).and_then(|t| t.wv.as_ref());
                        match v["cmd"].as_str().unwrap_or_default() {
                            "ready" => ext_list(), // icônes de la barre d'adresse dès le démarrage
                            "sync" => {}
                            // souris sur Volty : consommation de Volt entier, redemandée chaque seconde par l'interface
                            "usage" => {
                                let u = perf::usage(&ui, cores);
                                let _ = ui.evaluate_script(&format!("voltyUse({})", json!({"ram": u.ram_mb, "cpu": u.cpu, "gpu": u.gpu, "vram": u.vram_mb})));
                                return;
                            }
                            "usage_stop" => {
                                perf::usage_stop();
                                return;
                            }
                            // logo haute résolution d'une vignette : résolu hors du fil UI (réseau), renvoyé par Ev::Vicon
                            "vicon" => {
                                let (u, p, d) = (str_of("url"), proxy.clone(), dir.clone());
                                std::thread::spawn(move || {
                                    let mut dim = 0u32;
                                    if let Some((icon, color)) = vicon::best(&d, &u, &mut dim) {
                                        let _ = p.send_event(Ev::Vicon(u, icon, color));
                                    }
                                });
                                return;
                            }
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
                                // périmètre : tout (« all »), ou seulement les favoris sans dossier (ou dans un dossier disparu)
                                let all = v["all"].as_bool().unwrap_or(false);
                                let items: Vec<(String, String)> = s.favs.iter().filter(|f| all || !s.folders.iter().any(|d| d.id == f.folder)).map(|f| (f.url.clone(), f.title.clone())).collect();
                                let mode = match v["mode"].as_str().unwrap_or("theme") {
                                    "site" => sort::Mode::Site,
                                    "entity" => sort::Mode::Entity,
                                    _ => sort::Mode::Theme,
                                };
                                let props: Vec<Value> = sort::suggest(&items, &names, mode).into_iter().map(|p| json!({"folder": p.folder, "new": p.new, "icon": p.icon, "color": p.color, "urls": p.urls})).collect();
                                let _ = ui.evaluate_script(&format!("renderSort({})", json!(props)));
                                return;
                            }
                            // « Remplir et signer » : avec le PDF de l'onglet affiché, ou vide (volt://sign) pour en choisir un.
                            // Installation et lecture du document hors du fil de l'interface (réseau la première fois).
                            c @ ("sign" | "nav") if c == "sign" || str_of("url").trim().eq_ignore_ascii_case("volt://sign") => {
                                let url = (c == "sign").then(|| tabs.iter().find(|t| t.id == active).map(|t| t.url.clone())).flatten();
                                toast(&lang::t("Préparation de « Remplir et signer »…"));
                                let (p, d) = (proxy.clone(), dir.clone());
                                std::thread::spawn(move || {
                                    let r = sign::install(&d).and_then(|_| url.map(|u| sign::fetch(&u).map(|(n, b)| (n, sign::b64(&b)))).transpose());
                                    let _ = p.send_event(Ev::Sign(r));
                                });
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
                            // menu de la barre d'adresse : Coller (le texte revient dans le champ) ou Coller et rechercher (on y va)
                            "clip" => {
                                let text = clipboard_text().unwrap_or_default();
                                if v["go"].as_bool().unwrap_or(false) {
                                    if !text.trim().is_empty() {
                                        if let Some(sec) = internal(&text) {
                                            let _ = ui.evaluate_script(&format!("openSection('{sec}')"));
                                        } else if let Some(w) = wv {
                                            let _ = w.load_url(&to_url(&text, &s.search));
                                            let _ = w.focus();
                                        }
                                    }
                                } else {
                                    let _ = ui.evaluate_script(&format!("pasted({})", json!(text)));
                                }
                                return;
                            }
                            "back" => { wv.map(|w| w.evaluate_script("history.back()")); }
                            "fwd" => { wv.map(|w| w.evaluate_script("history.forward()")); }
                            "reload" => { wv.map(|w| w.reload()); }
                            "home" => { wv.map(|w| w.load_url(&s.home)); }
                            // l'indicateur suit tout de suite : le moteur ne prévient pas toujours d'un zoom qu'on lui impose
                            "find" => {
                                wv.map(audio::find);
                                return;
                            }
                            // autorisations des sites (caméra, micro, position…) : la liste, ou l'oubli d'un choix puis la liste
                            c @ ("perms" | "perm_forget") => {
                                if c == "perm_forget" {
                                    audio::forget_permission(&ui, v["kind"].as_i64().unwrap_or(0) as i32, &str_of("site"));
                                }
                                let p = proxy.clone();
                                audio::permissions(&ui, move |l| {
                                    let _ = p.send_event(Ev::Perms(l));
                                });
                                return;
                            }
                            // gestionnaire de tâches : chaque processus de Volt avec les onglets qu'il sert, dans toutes les fenêtres
                            "tasks" => {
                                let frame = |w: &WebView| {
                                    let mut f = 0u32;
                                    unsafe { w.webview().cast::<ICoreWebView2_20>().and_then(|x| x.FrameId(&mut f)) }.ok().map(|_| f)
                                };
                                // (cadre principal, onglet, titre, affiché dans sa fenêtre)
                                let mut pages: Vec<(u32, u32, String, bool)> = vec![];
                                for (ts, act) in std::iter::once((&tabs, active)).chain(parked.iter().map(|p| (&p.tabs, p.active))) {
                                    pages.extend(ts.iter().filter_map(|t| Some((frame(t.wv.as_ref()?)?, t.id, t.title.clone(), t.id == act))));
                                }
                                let uis: Vec<u32> = std::iter::once(&ui).chain(parked.iter().map(|p| &p.ui)).filter_map(frame).collect();
                                let p = proxy.clone();
                                perf::tasks(&ui, cores, move |l| {
                                    let rows: Vec<Value> = l
                                        .iter()
                                        .map(|t| {
                                            let on: Vec<Value> = pages.iter().filter(|x| t.frames.contains(&x.0)).map(|x| json!({"id": x.1, "title": x.2, "shown": x.3})).collect();
                                            json!({"pid": t.pid, "kind": t.kind, "ui": t.frames.iter().any(|f| uis.contains(f)), "tabs": on, "ram": t.ram_mb, "cpu": t.cpu})
                                        })
                                        .collect();
                                    let _ = p.send_event(Ev::Tasks(json!(rows)));
                                });
                                return;
                            }
                            // « Décharger » : ces onglets libèrent leur mémoire, restent dans la barre et se rechargent au clic. Jamais
                            // l'onglet qu'une fenêtre affiche, ni celui qui porte du texte non envoyé (il serait perdu).
                            "task_free" => {
                                let ids: Vec<u64> = v["ids"].as_array().map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
                                let mut kept = false;
                                for (ts, act) in std::iter::once((&mut tabs, active)).chain(parked.iter_mut().map(|p| (&mut p.tabs, p.active))) {
                                    for t in ts.iter_mut().filter(|t| ids.contains(&(t.id as u64)) && t.id != act) {
                                        if t.typed {
                                            kept = true;
                                        } else {
                                            (t.wv, t.frozen) = (None, false);
                                        }
                                    }
                                }
                                if kept {
                                    toast(&lang::t("Onglet gardé : il contient du texte non envoyé"));
                                }
                            }
                            // page « Téléchargements » : la liste, puis une action sur une ligne (ouvrir, montrer dans le
                            // dossier, pause, reprise, annulation, retrait) ou sur toute la liste
                            "dls" => {
                                let _ = ui.evaluate_script(&format!("dls({})", dl::list()));
                                return;
                            }
                            "dl_act" => {
                                let (id, a) = (v["id"].as_u64().unwrap_or(0) as u32, str_of("a"));
                                match (a.as_str(), dl::path_of(id)) {
                                    ("open", Some(p)) => {
                                        if !defaults::open_path(&p) {
                                            toast(&lang::t("Ce fichier n'est plus là"));
                                        }
                                    }
                                    ("show", Some(p)) => defaults::reveal(&p),
                                    ("folder", _) => {
                                        let d = if s.download_dir.is_empty() { defaults::downloads_dir().unwrap_or_default() } else { s.download_dir.clone() };
                                        defaults::open_path(&d);
                                    }
                                    ("clear", _) => dl::clear(),
                                    (a, _) => dl::act(id, a),
                                }
                                let _ = proxy.send_event(Ev::Dls);
                                return;
                            }
                            // dossier des téléchargements : choisi dans la fenêtre de Windows, ou remis à celui de Windows
                            c @ ("dl_pick" | "dl_reset") => {
                                let dir = if c == "dl_pick" { defaults::pick_folder(win.hwnd()) } else { defaults::downloads_dir() };
                                let Some(dir) = dir else { return };
                                audio::set_download_dir(&ui, &dir);
                                s.download_dir = if c == "dl_pick" { dir } else { String::new() };
                                save(&s, &cfg_path);
                            }
                            "zoom_reset" => {
                                wv.map(|w| audio::set_zoom(w, 1.0));
                                tabs.iter_mut().filter(|t| t.id == active).for_each(|t| t.zoom = 1.0);
                            }
                            // icône des fenêtres bloquées : les ouvrir, ou autoriser le site pour de bon
                            "popup_menu" => {
                                let Some(i) = tabs.iter().position(|t| t.id == active) else { return };
                                let Some(site) = cmp::site(&tabs[i].url) else { return };
                                let urls = tabs[i].blocked.clone();
                                let mut rows: Vec<_> = urls.iter().take(8).map(|u| Some((lang::t("Ouvrir {x}").replace("{x}", &tabmenu::label("", u)), false))).collect();
                                rows.extend([None, Some((lang::t("Toujours autoriser les fenêtres de {x}").replace("{x}", &site), false))]);
                                let Some(k) = tabmenu::popup(win.hwnd(), &rows, None) else { return };
                                tabs[i].blocked.clear();
                                match urls.get(k) {
                                    Some(u) => open(u, &mut tabs, &mut active),
                                    None => {
                                        for u in &urls {
                                            open(u, &mut tabs, &mut active.clone());
                                        }
                                        s.popup_allow.push(site);
                                        save(&s, &cfg_path);
                                    }
                                }
                            }
                            "new" => {
                                let h = s.home.clone();
                                open(&h, &mut tabs, &mut active)
                            }
                            "select" => active = v["id"].as_u64().unwrap_or(active as u64) as u32,
                            // onglet glissé à la souris vers une nouvelle place
                            "tabmove" => {
                                let id = v["id"].as_u64().unwrap_or(0) as u32;
                                if let Some(i) = tabs.iter().position(|t| t.id == id) {
                                    tabmenu::move_to(&mut tabs, i, v["to"].as_u64().unwrap_or(0) as usize);
                                    // groupe de l'endroit du dépôt, dit par l'interface (0 : hors de tout groupe)
                                    if let Some(g) = v["g"].as_u64() {
                                        tabmenu::regroup(&mut tabs, id, if groups.iter().any(|x| x.id as u64 == g) { g as u32 } else { 0 });
                                    }
                                }
                            }
                            // onglet lâché hors de la barre d'onglets : dans la fenêtre de Volt qui est sous la souris s'il y
                            // en a une autre (même mode, normal ou privé), sinon dans une nouvelle fenêtre à cet endroit
                            "tabout" => {
                                let id = v["id"].as_u64().unwrap_or(0) as u32;
                                let mut p = POINT::default();
                                let root = unsafe {
                                    let _ = GetCursorPos(&mut p);
                                    // essais pilotés : point de dépôt imposé à la place de la souris
                                    if let (Some(x), Some(y)) = (v["x"].as_i64(), v["y"].as_i64()) {
                                        p = POINT { x: x as i32, y: y as i32 };
                                    }
                                    GetAncestor(WindowFromPoint(p), GA_ROOT)
                                }
                                .0 as isize;
                                winop = match parked.iter().find(|x| x.win.hwnd() == root && x.private == private) {
                                    Some(x) => Some(WinOp::Move { tab: id, to: x.win.id() }),
                                    // seul onglet de sa fenêtre : la sortir reviendrait à déplacer la fenêtre
                                    None if tabs.len() > 1 => Some(WinOp::New { private, tab: Some(id), at: Some((p.x - 120, p.y - 18)) }),
                                    None => None,
                                };
                            }
                            // menu de l'onglet ; « tabdo » applique une action sans menu (essais pilotés), « mute » et « close » sont les boutons de l'onglet
                            c @ ("tabmenu" | "tabdo" | "mute" | "close") => {
                                let id = v["id"].as_u64().unwrap_or(0) as u32;
                                let Some(i) = tabs.iter().position(|t| t.id == id) else { return };
                                let fav = s.favs.iter().any(|f| f.url == tabs[i].url);
                                // groupes de cette fenêtre que l'onglet peut rejoindre : une ligne du menu chacun
                                let others: Vec<(u32, String)> = groups.iter().filter(|g| g.id != tabs[i].group && !tabs[i].pinned && tabs.iter().any(|t| t.group == g.id)).map(|g| (g.id, group_label(g))).collect();
                                let mut to_group = v["g"].as_u64().map(|g| g as u32); // « tabdo » : groupe visé par « group_add »
                                let act = match c {
                                    "tabmenu" => {
                                        let rows: Vec<String> = others.iter().map(|g| lang::t("Ajouter au groupe {x}").replace("{x}", &tabmenu::label(&g.1, ""))).collect();
                                        match tabmenu::show_with(win.hwnd(), &tabmenu::items(&tabs, id, fav, !closed.is_empty()), "group_new", &rows) {
                                            Some(tabmenu::Picked::Action(a)) => a,
                                            Some(tabmenu::Picked::Extra(k)) => {
                                                to_group = others.get(k).map(|g| g.0);
                                                "group_add"
                                            }
                                            None => "",
                                        }
                                    }
                                    "tabdo" => v["action"].as_str().unwrap_or_default(),
                                    c => c,
                                };
                                match act {
                                    // comme Chrome : le nouvel onglet et la copie restent dans le groupe de l'onglet d'origine
                                    "new_right" => {
                                        let (h, at, g) = (s.home.clone(), tabmenu::new_at(&tabs, i), tabs[i].group);
                                        open(&h, &mut tabs, &mut active);
                                        tabs.last_mut().unwrap().group = g;
                                        tabmenu::move_last(&mut tabs, at);
                                    }
                                    "dup" => {
                                        let (u, p, g) = (tabs[i].url.clone(), tabs[i].pinned, tabs[i].group);
                                        open(&u, &mut tabs, &mut active);
                                        (tabs.last_mut().unwrap().pinned, tabs.last_mut().unwrap().group) = (p, g); // la copie d'un épinglé est épinglée, juste après lui
                                        tabmenu::move_last(&mut tabs, i + 1);
                                    }
                                    "group_new" if !tabs[i].pinned => {
                                        next_group += 1;
                                        let color = tabmenu::GROUP_COLORS[next_group as usize % tabmenu::GROUP_COLORS.len()].1;
                                        groups.push(Group { id: next_group, name: String::new(), color: color.into(), closed: false });
                                        tabmenu::set_group(&mut tabs, i, next_group);
                                        gedit = Some((win.id(), next_group));
                                    }
                                    "group_add" if !tabs[i].pinned && to_group.is_some_and(|g| groups.iter().any(|x| x.id == g)) => {
                                        tabmenu::set_group(&mut tabs, i, to_group.unwrap_or(0));
                                    }
                                    "group_out" => {
                                        tabmenu::set_group(&mut tabs, i, 0);
                                    }
                                    "reload" => { tabs[i].wv.as_ref().map(|w| w.reload()); }
                                    "to_window" if tabs.len() > 1 => winop = Some(WinOp::New { private, tab: Some(id), at: None }),
                                    "new_window" => winop = Some(WinOp::New { private: false, tab: None, at: None }),
                                    "new_private" => winop = Some(WinOp::New { private: true, tab: None, at: None }),
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
                                        tabmenu::adopt(&mut tabs, at);
                                    }
                                    _ => return,
                                }
                            }
                            // pastille d'un groupe : clic (replier ou déplier), menu du clic droit, nom saisi ; « groupdo » applique une
                            // action sans menu
                            c @ ("groupmenu" | "groupdo") => {
                                let gid = v["id"].as_u64().unwrap_or(0) as u32;
                                let Some(gi) = groups.iter().position(|g| g.id == gid) else { return };
                                let act = match c {
                                    "groupmenu" => {
                                        let (rows, acts) = tabmenu::group_rows(&groups[gi].color);
                                        let Some(a) = tabmenu::popup(win.hwnd(), &rows, None).and_then(|k| acts.get(k).cloned()) else { return };
                                        a
                                    }
                                    _ => str_of("action"),
                                };
                                match act.as_str() {
                                    "toggle" => groups[gi].closed = !groups[gi].closed,
                                    "rename" => {
                                        let _ = ui.focus();
                                        let _ = ui.evaluate_script(&format!("groupRename({gid})"));
                                        return;
                                    }
                                    "name" => groups[gi].name = str_of("name").trim().chars().take(40).collect(),
                                    "ungroup" => tabs.iter_mut().filter(|t| t.group == gid).for_each(|t| t.group = 0),
                                    "close" => shut = tabs.iter().filter(|t| t.group == gid).map(|t| t.id).collect(),
                                    a => match a.strip_prefix("color:").and_then(|n| n.parse::<usize>().ok()).and_then(|n| tabmenu::GROUP_COLORS.get(n)) {
                                        Some(c) => groups[gi].color = c.1.into(),
                                        None => return,
                                    },
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
                                    (n.session_groups, n.groups) = (std::mem::take(&mut s.session_groups), std::mem::take(&mut s.groups));
                                    (n.cmp_skip, n.no_translate) = (cmp::clean(&n.cmp_skip), cmp::clean(&n.no_translate));
                                    let had_adblock = s.adblock;
                                    let js = cmp_js(&n);
                                    if js != cmp_js(&s) {
                                        set_cmp(&mut tabs, &js);
                                    }
                                    s = n;
                                    dl::ASK.store(s.download_ask, std::sync::atomic::Ordering::Relaxed);
                                    // remplissage des formulaires changé : appliqué tout de suite aux onglets chargés de toutes les fenêtres
                                    if AUTOFILL.swap(s.autofill, std::sync::atomic::Ordering::Relaxed) != s.autofill {
                                        tabs.iter().chain(parked.iter().flat_map(|p| p.tabs.iter())).filter_map(|t| t.wv.as_ref()).for_each(engine_forms);
                                    }
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
                                    }
                                    // langue de l'interface changée : sa page est rechargée dans la nouvelle langue (rien ne peut y
                                    // rester dans l'ancienne) et rouvre les réglages, tiroir fermé ; son « ready » relance le rendu
                                    if lang::set(&s.lang, &sys) {
                                        DRAWER.store(false, std::sync::atomic::Ordering::Relaxed);
                                        DRAWER_W.store(0, std::sync::atomic::Ordering::Relaxed);
                                        let _ = ui.load_html(&lang::ui(UI, "start"));
                                        return;
                                    }
                                }
                            }
                            // barre d'adresse : favoris et pages déjà vues qui correspondent à la saisie
                            "sugg" => {
                                let q = str_of("q");
                                let l = suggest(&q, &s.favs, &hist, 7);
                                let _ = ui.evaluate_script(&format!("sugg({},{})", json!(q), json!(l)));
                                return;
                            }
                            // liste de suggestions affichée (h > 0) ou refermée : l'interface s'étend par-dessus la page
                            "over" => {
                                let h = v["h"].as_f64().unwrap_or(0.0).clamp(0.0, 480.0) as u32;
                                OVER.store(h, std::sync::atomic::Ordering::Relaxed);
                                let _ = ui.set_bounds(ui_rect(&win, panel));
                                let _ = unsafe { SetWindowPos(ui.hwnd(), Some(if h > 0 { HWND_TOP } else { HWND_BOTTOM }), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
                                return;
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
                                    toast(&lang::t("La dernière session n'est plus proposée"));
                                } else if let Some((urls, pinned, _)) = offer.take() {
                                    if let Some(i) = tabs.iter().position(|t| t.id == 1 && same_page(&t.url, &s.home)) {
                                        tabs.remove(i);
                                        let _ = fs::remove_file(preview_of(1));
                                    }
                                    let mut back: Vec<Tab> = urls.iter().enumerate().map(|(i, u)| {
                                        next_id += 1;
                                        let title = hist.iter().find(|h| &h.url == u).map_or(u.clone(), |h| h.title.clone());
                                        let pin = (i as u32) < pinned;
                                        Tab { id: next_id, wv: None, title, url: u.clone(), loading: false, audio: false, muted: false, seen: Instant::now(), frozen: false, typed: false, saved: None, pinned: pin, cmp: String::new(), foreign: false, icon: String::new(), blocked: vec![], zoom: 1.0, bg: None, group: if pin { 0 } else { saved_group(&saved_groups, i) } }
                                    }).collect();
                                    for g in &saved_groups.1 {
                                        if !groups.iter().any(|x| x.id == g.id) {
                                            groups.push(g.clone());
                                        }
                                    }
                                    if !tabs.iter().any(|t| t.id == active) {
                                        active = back[0].id;
                                    }
                                    back.append(&mut tabs);
                                    tabs = back;
                                    tabs.sort_by_key(|t| !t.pinned); // épinglés toujours en tête (tri stable : ordre gardé)
                                }
                            }
                            // dossier de favoris ouvert « en menu » : liste native de ses favoris sous le dossier (un menu
                            // de l'interface serait coupé par la page)
                            "favmenu" => {
                                let id = str_of("id");
                                let list: Vec<&Page> = s.favs.iter().filter(|f| f.folder == id).collect();
                                let rows: Vec<_> = if list.is_empty() { vec![Some((lang::t("(vide)"), true))] } else { list.iter().map(|f| Some((tabmenu::label(&f.title, &f.url), false))).collect() };
                                // coin bas gauche du dossier, donné par l'interface en points de sa vue
                                let (k, o) = (win.scale_factor(), win.inner_position().map_or((0, 0), |p| (p.x, p.y)));
                                let at = (o.0 + ((border(&win) + v["x"].as_f64().unwrap_or(0.0)) * k) as i32, o.1 + (v["y"].as_f64().unwrap_or(0.0) * k) as i32);
                                let Some(u) = tabmenu::popup(win.hwnd(), &rows, Some(at)).and_then(|i| list.get(i)).map(|f| f.url.clone()) else { return };
                                // Ctrl tenu au moment du choix : nouvel onglet, comme un Ctrl+clic sur un favori
                                if unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0 {
                                    open(&u, &mut tabs, &mut active)
                                } else if let Some(w) = wv {
                                    let _ = w.load_url(&u);
                                    let _ = w.focus();
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
                                    if translate::can_relay(&u) {
                                        let _ = w.load_url(&translate::relay(&u));
                                    } else {
                                        toast(&lang::t("Google ne permet pas de traduire cette page"));
                                    }
                                    return;
                                }
                                let Some(o) = translate::original(&u) else {
                                    toast(&lang::t("Adresse d'origine introuvable"));
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
                                    toast(&lang::t("Ce n'est pas une adresse de site (exemple : lemonde.fr)"));
                                    return;
                                };
                                if s.cmp_skip.contains(&h) {
                                    return;
                                }
                                s.cmp_skip.push(h);
                                save(&s, &cfg_path);
                                set_cmp(&mut tabs, &cmp_js(&s));
                            }
                            // vide le coffre de Volt (plus ClearBrowsingData pour ce qui resterait côté moteur)
                            "pw_clear" => {
                                vault.clear();
                                vault::save(&vault_path, &vault);
                                let p = proxy.clone();
                                cookies::clear_passwords(&ui, move |_| {
                                    let _ = p.send_event(Ev::Toast(lang::t("Mots de passe supprimés")));
                                });
                                let _ = ui.evaluate_script(&pw_manager_js(&vault));
                                toast(&lang::t("Mots de passe supprimés"));
                                return;
                            }
                            // cartes de paiement : la liste (sans numéros), l'ajout (numéro vérifié), la suppression derrière Windows Hello
                            "cc_list" => {
                                let _ = ui.evaluate_script(&cards_js(&cards));
                                return;
                            }
                            "cc_add" => {
                                let (Some(number), Some((month, year))) = (cards::clean(&str_of("number")), cards::expiry(&str_of("exp"))) else {
                                    toast(&lang::t("Numéro de carte ou date d'expiration incorrects (date : MM/AA)"));
                                    return;
                                };
                                cards.retain(|c| c.number != number); // la même carte saisie de nouveau : remplacée
                                cards.push(cards::Card { name: str_of("name").trim().chars().take(60).collect(), number, month, year });
                                if !cards::save(&cards_path, &cards) {
                                    toast(&lang::t("Écriture du fichier impossible"));
                                }
                                let _ = ui.evaluate_script(&format!("ccAdded();{}", cards_js(&cards)));
                            }
                            "cc_del" => {
                                let mut cmd = v.clone();
                                cmd["do"] = json!("cc_del");
                                ask_hello(proxy.clone(), lang::t("Déverrouillez pour supprimer cette carte"), cmd);
                                return;
                            }
                            // icône de carte de la barre d'adresse : Volt cherche d'abord où l'utilisateur a désigné un champ de carte
                            "cc_fill" => {
                                let Some(t) = tabs.iter().find(|t| t.id == active) else { return };
                                let (p, id) = (proxy.clone(), active);
                                cards::target(id, card_main(t), cc_main.contains(&id), move |o| {
                                    let _ = p.send_event(Ev::CardTarget(id, o));
                                });
                                return;
                            }
                            // gestionnaire : envoie la liste (sans mots de passe)
                            "pw_list" => {
                                let _ = ui.evaluate_script(&pw_manager_js(&vault));
                                return;
                            }
                            // ajout d'une entrée (sans Hello) ; origine validée
                            "pw_add" => {
                                let Some(origin) = vault::origin_of(&str_of("origin")) else {
                                    toast(&lang::t("Adresse invalide (exemple : https://site.fr)"));
                                    return;
                                };
                                vault::merge(&mut vault, vec![vault::Entry { origin, username: str_of("user"), password: str_of("pass"), note: str_of("note") }]);
                                vault::save(&vault_path, &vault);
                                let _ = ui.evaluate_script(&pw_manager_js(&vault));
                                return;
                            }
                            // révéler / modifier / supprimer / exporter : derrière Windows Hello (fait dans Ev::Hello)
                            c @ ("pw_reveal" | "pw_edit" | "pw_del" | "pw_export") => {
                                let msg = if c == "pw_export" { lang::t("Déverrouillez pour exporter vos mots de passe") } else { lang::t("Déverrouillez pour voir vos mots de passe") };
                                let mut cmd = v.clone();
                                cmd["do"] = json!(c.trim_start_matches("pw_"));
                                ask_hello(proxy.clone(), msg, cmd);
                                return;
                            }
                            // import CSV (sans Hello) : l'utilisateur choisit le fichier
                            "pw_import" => {
                                let Some(path) = vault::pick_open(win.hwnd()) else { return };
                                let Ok(text) = fs::read_to_string(&path) else {
                                    toast(&lang::t("Lecture du fichier impossible"));
                                    return;
                                };
                                let (entries, skipped) = vault::parse_csv(&text);
                                let added = vault::merge(&mut vault, entries);
                                vault::save(&vault_path, &vault);
                                let _ = ui.evaluate_script(&pw_manager_js(&vault));
                                toast(&lang::t("{a} importés, {s} ignorés").replace("{a}", &added.to_string()).replace("{s}", &skipped.to_string()));
                                return;
                            }
                            // remplissage sur action de l'utilisateur : clic sur l'icône (ou choix d'un compte)
                            "pw_fill" => {
                                let Some((Some(origin), Some(w))) = tabs.iter().find(|t| t.id == active).map(|t| (vault::origin_of(&t.url), t.wv.as_ref())) else { return };
                                let accounts = vault::for_origin(&vault, &origin);
                                let chosen = match v["user"].as_str() {
                                    Some(u) => accounts.into_iter().find(|e| e.username == u),
                                    None if accounts.len() == 1 => Some(accounts[0]),
                                    None => {
                                        // plusieurs comptes : laisser choisir l'identifiant
                                        let users: Vec<&str> = accounts.iter().map(|e| e.username.as_str()).collect();
                                        let _ = ui.evaluate_script(&format!("pwPick({})", json!(users)));
                                        return;
                                    }
                                };
                                if let Some(e) = chosen {
                                    // valeurs insérées en JSON (jamais par collage de chaîne), `<` échappé comme ailleurs
                                    let esc = |x: &str| serde_json::to_string(x).unwrap_or_default().replace('<', "\\u003c");
                                    let _ = w.evaluate_script(&format!("window.__voltFill&&__voltFill({},{})", esc(&e.username), esc(&e.password)));
                                }
                                return;
                            }
                            // bandeau « Enregistrer » : valide le couple en attente (jamais sans ce clic)
                            "pw_save" => {
                                if let Some(e) = pending_save.take() {
                                    vault::merge(&mut vault, vec![e]);
                                    vault::save(&vault_path, &vault);
                                }
                                return;
                            }
                            "pw_dismiss" => {
                                pending_save = None;
                                return;
                            }
                            "default_browser" => {
                                if let Err(e) = defaults::ask() {
                                    toast(&lang::te("Enregistrement impossible : {e}", e));
                                }
                                return;
                            }
                            // « Commencer » de la bienvenue (interrupteur laissé coché) ou bouton « Créer » des réglages
                            "shortcuts" => {
                                toast(&match defaults::shortcuts() {
                                    Ok(()) => lang::t("Raccourcis en place : bureau et menu Démarrer"),
                                    Err(e) => lang::te("Raccourci impossible : {e}", e),
                                });
                                return;
                            }
                            "browsers" => {
                                let _ = ui.evaluate_script(&format!("renderBrowsers({})", import::sources()));
                                return;
                            }
                            // favoris en fichier HTML (format commun à tous les navigateurs) : l'utilisateur choisit le fichier
                            "fav_import_html" => {
                                let Some(path) = vault::pick_file(win.hwnd(), false, &lang::t("Fichiers de favoris (HTML)"), "html", "") else { return };
                                let Ok(text) = fs::read_to_string(&path) else {
                                    toast(&lang::t("Lecture du fichier impossible"));
                                    return;
                                };
                                let n = import::import_html(&text, &mut s.favs, &mut s.folders);
                                save(&s, &cfg_path);
                                toast_n("{n} favori importé|{n} favoris importés", n);
                            }
                            "fav_export_html" => {
                                let Some(path) = vault::pick_file(win.hwnd(), true, &lang::t("Fichiers de favoris (HTML)"), "html", "") else { return };
                                toast(&match fs::write(&path, import::to_html(&s.favs, &s.folders)) {
                                    Ok(()) => lang::t("Favoris exportés"),
                                    Err(e) => lang::te("Écriture du fichier impossible : {e}", e),
                                });
                                return;
                            }
                            "import" => {
                                let r = import::import(&str_of("id"), &mut s.favs, &mut s.folders);
                                let _ = ui.evaluate_script(&format!("imported({},{})", json!(str_of("id")), json!(r)));
                                match r {
                                    Some(n) => {
                                        save(&s, &cfg_path);
                                        toast_n("{n} favori importé|{n} favoris importés", n);
                                    }
                                    None => {
                                        toast(&lang::t("Favoris introuvables"));
                                        return;
                                    }
                                }
                            }
                            // page Migration : liste des profils d'autres navigateurs et ce qu'on peut en reprendre
                            "migrate_list" => {
                                let _ = ui.evaluate_script(&format!("migrateList({})", migrate::sources()));
                                return;
                            }
                            // reprise depuis un autre navigateur : cookies, favoris, mots de passe, historique (au choix)
                            "migrate" => {
                                let id = str_of("id");
                                let has = |k: &str| v["what"].as_array().is_some_and(|a| a.iter().any(|x| x.as_str() == Some(k)));
                                let (do_c, do_h, do_p, do_f) = (has("cookies"), has("history"), has("passwords"), has("favorites"));
                                let data = migrate::read(&id, do_c, do_h, do_p);
                                let mut counts = serde_json::Map::new();
                                if do_c {
                                    counts.insert("cookies".into(), json!(cookies::import(&ui, &data.cookies)));
                                }
                                if do_f {
                                    let n = import::import(&id, &mut s.favs, &mut s.folders).unwrap_or(0);
                                    save(&s, &cfg_path);
                                    counts.insert("favorites".into(), json!(n));
                                }
                                if do_p {
                                    let entries: Vec<_> = data
                                        .logins
                                        .into_iter()
                                        .filter_map(|(o, u, p)| vault::origin_of(&o).map(|o| vault::Entry { origin: o, username: u, password: p, note: String::new() }))
                                        .collect();
                                    let n = vault::merge(&mut vault, entries);
                                    vault::save(&vault_path, &vault);
                                    counts.insert("passwords".into(), json!(n));
                                }
                                if do_h {
                                    let mut h = data.history;
                                    h.sort_by(|a, b| b.3.total_cmp(&a.3)); // plus récentes d'abord : si le plafond coupe, il garde l'utile
                                    let mut added = 0;
                                    for (url, title, _n, at) in h {
                                        if hist.iter().any(|v| v.url == url) {
                                            continue;
                                        }
                                        hist.push(Visit { url, title, at: at.max(0.0) as u64 });
                                        added += 1;
                                    }
                                    hist.sort_by(|a, b| b.at.cmp(&a.at));
                                    hist.truncate(HIST_MAX);
                                    let _ = fs::write(&hist_path, serde_json::to_vec(&hist).unwrap_or_default());
                                    counts.insert("history".into(), json!(added));
                                }
                                dirty = true; // favoris et barre de favoris à rafraîchir
                                let _ = ui.evaluate_script(&format!("migrateDone({})", json!({"counts": counts, "locked": data.locked, "appbound": data.app_bound})));
                                return;
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
                                            let _ = p.send_event(Ev::Cmd(json!({"cmd": "cookies"}).to_string()));
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
                                if parked.is_empty() {
                                    persist(&mut s, (!private).then_some(&tabs[..]), &groups, &hist, &cfg_path, &hist_path, false, offer_n(&offer) > 0);
                                    *flow = ControlFlow::Exit;
                                    return;
                                }
                                winop = Some(WinOp::Close);
                            }
                            "restart" => {
                                persist(&mut s, (!private).then_some(&tabs[..]), &groups, &hist, &cfg_path, &hist_path, true, offer_n(&offer) > 0);
                                // Le nouveau Volt attend lui-même que celui-ci ait libéré le profil (voir --restart dans main).
                                // Après une mise à jour, ce chemin est celui du nouvel exe (l'ancien a été renommé).
                                if let Err(e) = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg("--restart").spawn()) {
                                    toast(&lang::te("Redémarrage impossible : {e}", e));
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
                                toast(&lang::t("Données de navigation effacées"));
                                return;
                            }
                            "ext_install" => {
                                // bouton « Installer dans Volt » : l'adresse réelle de la page à cet instant (le Store change de fiche sans recharger)
                                let src = if v["page"].as_bool() == Some(true) { wv.and_then(|w| w.url().ok()).unwrap_or_default() } else { str_of("id") };
                                let Some(id) = ext::parse_id(&src) else {
                                    toast(&lang::t("ID ou lien Chrome Web Store invalide"));
                                    return;
                                };
                                if ext_dir.join(&id).exists() {
                                    toast(&lang::t("Cette extension est déjà installée"));
                                    return;
                                }
                                toast(&lang::t("Téléchargement de l'extension…"));
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
                                        toast(&lang::t("Cette extension n'a pas de page d'options"));
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
                        // plus d'onglet : la fenêtre se ferme, et Volt avec elle si c'était la dernière
                        None if parked.is_empty() => {
                            persist(&mut s, (!private).then_some(&tabs[..]), &groups, &hist, &cfg_path, &hist_path, false, offer_n(&offer) > 0);
                            *flow = ControlFlow::Exit;
                            return;
                        }
                        None => winop = Some(WinOp::Close),
                    }
                }
                if let Some(t) = tabs.iter_mut().find(|t| t.id == active) {
                    if t.wv.is_none() {
                        // Capture de la page laissée, affichée tout de suite pendant le rechargement derrière. Sans capture,
                        // rien : une page d'attente a été mesurée, elle ne s'affichait jamais et coûtait 50 ms.
                        let html = t.url.starts_with("http").then(|| preview::page(&preview_of(t.id), &t.url)).flatten().map(|h| (h, preview::bg(&preview_of(t.id)).unwrap_or(LOADING_BG)));
                        let (w, c) = make_tab(&win, &mut ctx, &proxy, t.id, &t.url, html, s.dark_web, &args, &cmp_js(&s), private);
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
                // un groupe sans onglet dans aucune fenêtre n'existe plus
                groups.retain(|g| tabs.iter().chain(parked.iter().flat_map(|p| p.tabs.iter())).any(|t| t.group == g.id));
                // toutes les fenêtres, chacune à son tour : réglages, favoris et thème sont communs
                for i in 0..=parked.len() {
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
                let sync = |tabs: &Vec<Tab>, active: u32, s: &Settings, offer: usize, vault: &[vault::Entry], pw_forms: &HashSet<u32>, groups: &[Group], gedit: u32, cc: Option<&HashSet<u32>>| {
                    // pwfill : un formulaire de connexion est repéré ET le coffre a une entrée pour l'origine EXACTE de l'onglet
                    let has_fill = |t: &Tab| pw_forms.contains(&t.id) && vault::origin_of(&t.url).is_some_and(|o| !vault::for_origin(vault, &o).is_empty());
                    let list: Vec<Value> = tabs.iter().map(|t| json!({"id": t.id, "title": t.title, "url": t.url, "loading": t.loading, "audio": t.audio, "muted": t.muted, "asleep": t.wv.is_none(), "pinned": t.pinned, "foreign": t.foreign, "icon": t.icon, "blocked": t.blocked.len(), "zoom": (t.zoom * 100.0).round(), "tr": translate::can_relay(&t.url), "pwfill": has_fill(t), "ccfill": cc.is_some_and(|c| c.contains(&t.id)), "group": t.group})).collect();
                    let _ = ui.evaluate_script(&format!("render({})", json!({"tabs": list, "groups": groups, "gedit": gedit, "active": active, "s": s, "max": win.is_maximized(), "private": private, "first": first, "fresh": fresh, "def": defaults::is_default(), "offer": offer, "ver": update::CURRENT, "staged": update::staged(), "lrestart": lang::engine(&s.lang, &sys) != elang})));
                };
                    layout(&tabs, active, panel, s.game_mode, minimized_at.is_some());
                    let edit = gedit.filter(|g| g.0 == win.id()).map_or(0, |g| g.1);
                    if edit != 0 {
                        let _ = ui.focus(); // le nom du nouveau groupe se tape tout de suite
                    }
                    sync(&tabs, active, &s, offer_n(&offer), &vault, &pw_forms, &groups, edit, (!cards.is_empty()).then_some(&cc_forms));
                    if i < parked.len() {
                        swap_win!(&mut parked[i]);
                    }
                }
                gedit = None;
            }
            _ => {}
        }
        // Opérations sur les fenêtres demandées pendant l'évènement : faites ici, quand plus rien n'emprunte la courante.
        match winop.take() {
            None => {}
            Some(WinOp::Close) => {
                if !private {
                    remember_session(&mut s, &tabs, &groups, false, offer_n(&offer) > 0);
                    save(&s, &cfg_path);
                }
                for t in &tabs {
                    let _ = fs::remove_file(preview_of(t.id));
                    let _ = fs::remove_file(preview_of(t.id).with_extension("bg"));
                }
                let Some(mut gone) = parked.pop() else { return };
                swap_win!(&mut gone); // `gone` porte maintenant la fenêtre fermée : détruite ici, vues comprises
                drop(gone);
                win.set_focus();
                dirty = true;
            }
            Some(op) => {
                let (tab, to) = match &op {
                    WinOp::New { tab, .. } => (*tab, None),
                    WinOp::Move { tab, to } => (Some(*tab), Some(*to)),
                    WinOp::Close => (None, None),
                };
                // l'onglet sort de la courante : il garde sa page chargée, seule sa fenêtre change
                let moved = match tab {
                    Some(id) => {
                        let Some(i) = tabs.iter().position(|t| t.id == id) else { return };
                        let mut t = tabs.remove(i);
                        t.group = 0; // un groupe ne s'étend pas sur deux fenêtres
                        if active == id {
                            active = tabs.get(i.min(tabs.len().saturating_sub(1))).map_or(0, |t| t.id);
                        }
                        Some(t)
                    }
                    None => None,
                };
                let src_empty = moved.is_some() && tabs.is_empty();
                match op {
                    WinOp::Move { .. } => {
                        // fenêtre d'arrivée fermée entre-temps : l'onglet reste où il était
                        let Some(j) = to.and_then(|to| parked.iter().position(|p| p.win.id() == to)) else {
                            if let Some(t) = moved {
                                active = t.id;
                                tabs.push(t);
                            }
                            return;
                        };
                        swap_win!(&mut parked[j]); // la courante est maintenant la fenêtre d'arrivée
                        if src_empty {
                            drop(parked.remove(j)); // la fenêtre de départ n'a plus d'onglet : elle se ferme
                        }
                    }
                    WinOp::New { private: pv, at, .. } => {
                        let pv = if moved.is_some() { private } else { pv };
                        let w = make_window(target, at.map(|(x, y)| PhysicalPosition::new(x, y)));
                        let u = make_ui(&w, &mut ctx, &proxy, &args, &ext_dir, pv);
                        let mut p = Parked {
                            ui: u,
                            tabs: vec![],
                            win: w,
                            active: 0,
                            panel: false,
                            minimized_at: None,
                            cursor: (0.0, 0.0),
                            offer: None,
                            first: false,
                            fresh: moved.is_none() && !pv, // fenêtre vide : elle s'ouvre sur la page de favoris, comme au démarrage
                            private: pv,
                            favh: if s.favbar { FAVBAR_H } else { 0 },
                            over: 0,
                            drawer: false,
                            drawer_w: 0,
                            full: false,
                        };
                        swap_win!(&mut p); // la courante est maintenant la nouvelle fenêtre
                        if !src_empty {
                            parked.push(p);
                        }
                    }
                    WinOp::Close => {}
                }
                match moved {
                    Some(mut t) => {
                        match &t.wv {
                            Some(v) => {
                                let _ = v.reparent(win.hwnd());
                            }
                            // onglet déchargé : rechargé dans sa nouvelle fenêtre
                            None => {
                                let (v, c) = make_tab(&win, &mut ctx, &proxy, t.id, &t.url, None, s.dark_web, &args, &cmp_js(&s), private);
                                (t.wv, t.cmp, t.loading) = (Some(v), c, true);
                            }
                        }
                        (t.seen, t.frozen, active) = (Instant::now(), false, t.id);
                        tabs.push(t);
                    }
                    None => {
                        next_id += 1;
                        let (v, c) = make_tab(&win, &mut ctx, &proxy, next_id, &s.home, None, s.dark_web, &args, &cmp_js(&s), private);
                        tabs.push(Tab { id: next_id, wv: Some(v), title: lang::t("Nouvel onglet"), url: s.home.clone(), loading: true, audio: false, muted: false, seen: Instant::now(), frozen: false, typed: false, saved: None, pinned: false, cmp: c, foreign: false, icon: String::new(), blocked: vec![], zoom: 1.0, bg: None, group: 0 });
                        active = next_id;
                    }
                }
                if !private {
                    front = win.id();
                }
                win.set_focus();
                dirty = true;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_bar_suggestions() {
        let page = |u: &str, t: &str| Page { url: u.into(), title: t.into(), icon: String::new(), folder: String::new() };
        let visit = |u: &str, t: &str| Visit { url: u.into(), title: t.into(), at: 0 };
        let favs = [page("https://www.youtube.com/", "YouTube"), page("https://github.com/rust-lang/rust", "Rust")];
        let hist = [
            visit("https://exemple.org/tout-sur-youtube", "Un article"),
            visit("https://www.youtube.com/watch?v=1", "Une vidéo de cuisine - YouTube"),
            visit("https://www.youtube.com/", "YouTube"),
            visit("https://fr.wikipedia.org/wiki/Rust", "Rust (langage) — Wikipédia"),
        ];
        let urls = |q: &str| suggest(q, &favs, &hist, 7).into_iter().map(|x| x.0).collect::<Vec<_>>();
        // début de l'adresse d'abord, le favori en tête ; la page déjà favorite n'est pas répétée depuis l'historique
        assert_eq!(urls("you"), ["https://www.youtube.com/", "https://www.youtube.com/watch?v=1", "https://exemple.org/tout-sur-youtube"]);
        assert!(suggest("you", &favs, &hist, 7)[0].2 && !suggest("you", &favs, &hist, 7)[1].2);
        // tous les mots, dans l'adresse ou le titre, sans tenir compte de la casse
        assert_eq!(urls("CUISINE youtube"), ["https://www.youtube.com/watch?v=1"]);
        assert_eq!(urls("rust"), ["https://github.com/rust-lang/rust", "https://fr.wikipedia.org/wiki/Rust"]);
        assert_eq!(urls("wikip"), ["https://fr.wikipedia.org/wiki/Rust"]);
        assert!(urls("").is_empty() && urls("   ").is_empty() && urls("introuvable").is_empty());
        assert_eq!(suggest("o", &favs, &hist, 2).len(), 2); // plafond respecté
    }

    #[test]
    fn urls() {
        assert_eq!(to_url("youtube.com", "google"), "https://youtube.com");
        assert_eq!(to_url("http://localhost:3000", "google"), "http://localhost:3000");
        assert_eq!(to_url("localhost:8080", "google"), "https://localhost:8080");
        // chemin complet d'un fichier existant, guillemets de l'Explorateur compris ; inexistant : comme avant
        let f = std::env::temp_dir().join(format!("volt to_url {}.pdf", std::process::id()));
        fs::write(&f, b"%PDF").unwrap();
        let u = to_url(&format!("\"{}\"", f.display()), "google");
        assert!(u.starts_with("file:///") && u.ends_with(".pdf") && !u.contains(' '), "{u}");
        fs::remove_file(&f).unwrap();
        assert!(to_url(&f.to_string_lossy(), "google").starts_with("https://www.google.com/search?q="));
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
        let tab = |id| Tab { id, wv: None, title: String::new(), url: String::new(), loading: false, audio: false, muted: false, seen: Instant::now(), frozen: false, typed: false, saved: None, pinned: false, cmp: String::new(), foreign: false, icon: String::new(), blocked: vec![], zoom: 1.0, bg: None, group: 0 };
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
        assert!(!s.game_mode && s.favs.is_empty() && s.search == "google" && s.lang == "auto");
        assert!(s.groups.is_empty() && s.session_groups.is_empty() && !s.download_ask && s.autofill);
        assert_eq!(internal("volt://settings/payments"), Some("cards"));
        // carte montrée à l'interface : jamais son numéro
        let js = cards_js(&[cards::Card { name: "A B".into(), number: "4242424242424242".into(), month: 3, year: 2031 }]);
        assert_eq!(js, r#"ccManager([{"exp":"03/31","i":0,"label":"Visa •••• 4242","name":"A B"}])"#);
        // pas de `vignettes` dans l'ancien profil : la liste par défaut (sites connus), images et couleurs vides
        assert!(!s.vignettes.is_empty() && s.vignettes[0].name == "YouTube" && s.vignettes[0].image.is_empty() && s.vignettes[0].color.is_empty());
        // image et couleur vides : non sérialisées (settings.json propre)
        let j0 = serde_json::to_string(&s.vignettes[0]).unwrap();
        assert!(!j0.contains("image") && !j0.contains("color"));
        // une vignette d'un ancien profil sans `color` se charge (défaut vide)
        let vg: Vignette = serde_json::from_str(r#"{"name":"A","url":"https://a.fr","image":"data:x"}"#).unwrap();
        assert!(vg.color.is_empty() && vg.image == "data:x");
        assert!(s.keep_loaded == 7 && !s.volty_anim && s.save_passwords && s.exts_open);
        assert!(!s.welcomed); // un profil d'avant la page de bienvenue la voit une fois
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
        let tab = |url: &str| Tab { id: 1, wv: None, title: String::new(), url: url.into(), loading: false, audio: false, muted: false, seen: Instant::now(), frozen: false, typed: false, saved: None, pinned: false, cmp: String::new(), foreign: false, icon: String::new(), blocked: vec![], zoom: 1.0, bg: None, group: 0 };
        assert!(untouched(&[tab("https://www.google.com/")], "https://www.google.com"));
        assert!(untouched(&[tab("https://www.google.com/?zx=1#a")], "https://www.google.com"));
        assert!(untouched(&[], "https://www.google.com"));
        assert!(!untouched(&[tab("https://www.google.com/search?q=a")], "https://www.google.com"));
        assert!(!untouched(&[tab("https://www.google.com"), tab("https://www.google.com")], "https://www.google.com"));
        // groupes : enregistrés avec la session (seulement ceux qui ont encore un onglet), oubliés avec elle
        let mut s = Settings { startup: "auto".into(), ..Default::default() };
        let (mut a, mut b) = (tab("https://a.fr"), tab("https://b.fr"));
        (a.group, b.pinned) = (3, true);
        let g = |id, name: &str| Group { id, name: name.into(), color: "#8ab4f8".into(), closed: false };
        remember_session(&mut s, &[b, a], &[g(3, "Travail"), g(4, "Vide")], false, false);
        assert_eq!((s.session.len(), s.pinned, s.session_groups.clone(), s.groups.clone()), (2, 1, vec![0, 3], vec![g(3, "Travail")]));
        let saved = (s.session_groups.clone(), s.groups.clone());
        assert_eq!((saved_group(&saved, 0), saved_group(&saved, 1), saved_group(&saved, 9)), (0, 3, 0));
        assert_eq!(saved_group(&(vec![5], vec![]), 0), 0); // groupe disparu
        assert_eq!((group_label(&g(3, "Travail")), group_label(&g(4, ""))), ("« Travail »".to_string(), "Bleu".to_string()));
        s.startup = "neuf".into();
        remember_session(&mut s, &[tab("https://a.fr")], &[], false, false);
        assert!(s.session.is_empty() && s.session_groups.is_empty() && s.groups.is_empty());
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
