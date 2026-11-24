// Téléchargements lancés par les pages, pour la page « Téléchargements » de Volt : le panneau du moteur s'ouvre mais
// ne liste rien dans une vue intégrée (mesuré), on tient donc la liste nous-mêmes. Tout vit sur le fil de l'interface.
// Les téléchargements terminés sont gardés d'un lancement à l'autre (fichier downloads.json du profil, 50 au plus).
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};
use webview2_com::{
    DownloadStartingEventHandler,
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_DOWNLOAD_STATE, COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED, COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS, ICoreWebView2_4, ICoreWebView2Deferral, ICoreWebView2DownloadOperation,
        ICoreWebView2DownloadStartingEventArgs,
    },
    StateChangedEventHandler, take_pwstr,
};
use windows::core::{BOOL, HSTRING, Interface, PWSTR};
use wry::{WebView, WebViewExtWindows};

thread_local! {
    // le plus récent en tête ; l'objet du moteur donne à tout moment l'avancement et permet pause, reprise, annulation
    static LIST: RefCell<Vec<(u32, ICoreWebView2DownloadOperation)>> = const { RefCell::new(vec![]) };
    static NEXT: Cell<u32> = const { Cell::new(1) };
    // téléchargements retenus par le moteur le temps de demander où les enregistrer (voir `ask_waiting`)
    static WAITING: RefCell<Vec<Waiting>> = const { RefCell::new(vec![]) };
    // téléchargements terminés des lancements précédents (et de celui-ci), le plus récent en tête, et leur fichier
    static PAST: RefCell<Vec<Past>> = const { RefCell::new(vec![]) };
    static PAST_FILE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct Past {
    #[serde(skip)]
    id: u32,
    name: String,
    path: String,
    site: String,
    size: i64,
}
const PAST_MAX: usize = 50;

fn save_past() {
    PAST_FILE.with_borrow(|f| {
        if let Some(f) = f {
            let _ = PAST.with_borrow(|l| std::fs::write(f, serde_json::to_vec(l).unwrap_or_default()));
        }
    });
}

/// Au démarrage : relit l'historique ; les fichiers qui ne sont plus sur le disque sont oubliés.
pub fn load_history(file: PathBuf) {
    let mut l: Vec<Past> = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    l.retain(|p| std::path::Path::new(&p.path).is_file());
    l.truncate(PAST_MAX);
    for p in &mut l {
        p.id = NEXT.replace(NEXT.get() + 1);
    }
    PAST.with_borrow_mut(|x| *x = l);
    PAST_FILE.with_borrow_mut(|f| *f = Some(file));
}

/// Un téléchargement vient de finir : gardé dans l'historique (une seule fois par fichier).
fn remember(op: &ICoreWebView2DownloadOperation) {
    let p = path(op);
    if p.is_empty() || PAST.with_borrow(|l| l.iter().any(|x| x.path == p)) {
        return;
    }
    let (mut got, url) = (0i64, text(|u| unsafe { op.Uri(u) }));
    let _ = unsafe { op.BytesReceived(&mut got) };
    let site = crate::cmp::site(&url).filter(|_| url.starts_with("http")).unwrap_or_default();
    let id = NEXT.replace(NEXT.get() + 1);
    PAST.with_borrow_mut(|l| {
        l.insert(0, Past { id, name: name(&p), path: p, site, size: got });
        l.truncate(PAST_MAX);
    });
    save_past();
}
type Waiting = (ICoreWebView2DownloadStartingEventArgs, ICoreWebView2Deferral, Box<dyn Fn(Option<u32>)>);

/// Réglage « demander où enregistrer chaque fichier ».
pub static ASK: AtomicBool = AtomicBool::new(false);

const RUNNING: u8 = 0;
const STOPPED: u8 = 1; // en pause (reprise possible) ou interrompu
const DONE: u8 = 2;

fn text(f: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>) -> String {
    let mut p = PWSTR::null();
    if f(&mut p).is_ok() { take_pwstr(p) } else { String::new() }
}

fn path(op: &ICoreWebView2DownloadOperation) -> String {
    text(|p| unsafe { op.ResultFilePath(p) })
}

/// Nom du fichier, sans son dossier.
fn name(path: &str) -> String {
    name_ref(path).to_string()
}

fn state(op: &ICoreWebView2DownloadOperation) -> u8 {
    let mut s = COREWEBVIEW2_DOWNLOAD_STATE::default();
    let _ = unsafe { op.State(&mut s) };
    if s == COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED {
        DONE
    } else if s == COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS {
        // certains téléchargements (mesuré : un .exe servi par GitHub) restent « en cours » alors que tous les octets
        // sont reçus et le fichier écrit : quand tout est là, on les considère terminés (sinon l'animation tournerait sans fin).
        let (mut got, mut total) = (0i64, 0i64);
        let _ = unsafe { (op.BytesReceived(&mut got), op.TotalBytesToReceive(&mut total)) };
        if total > 0 && got >= total { DONE } else { RUNNING }
    } else {
        STOPPED
    }
}

/// Suit les téléchargements de cette vue. `changed(Some(id))` : ce téléchargement vient de commencer, ou son fichier
/// vient d'arriver (à signaler par la petite notif) ; `changed(None)` : la liste ou un état a changé.
/// `ask()` : un téléchargement attend qu'on demande où l'enregistrer ; Volt appelle alors `ask_waiting`.
pub fn watch(wv: &WebView, changed: impl Fn(Option<u32>) + Clone + 'static, ask: impl Fn() + 'static) {
    let Ok(w) = wv.webview().cast::<ICoreWebView2_4>() else { return };
    let h = DownloadStartingEventHandler::create(Box::new(move |_, args| {
        let Some(a) = args else { return Ok(()) };
        unsafe { a.SetHandled(true)? }; // pas le panneau du moteur : la page de Volt
        if ASK.load(Ordering::Relaxed) {
            // la fenêtre « Enregistrer sous » ne s'ouvre pas ici, dans le rappel du moteur : il patiente, elle vient juste après
            let d = unsafe { a.GetDeferral()? };
            WAITING.with_borrow_mut(|l| l.push((a, d, Box::new(changed.clone()))));
            ask();
            return Ok(());
        }
        track(unsafe { a.DownloadOperation()? }, changed.clone());
        Ok(())
    }));
    let mut token = 0i64;
    let _ = unsafe { w.add_DownloadStarting(&h, &mut token) };
}

/// Ajoute le téléchargement à la liste et suit son état.
fn track(op: ICoreWebView2DownloadOperation, changed: impl Fn(Option<u32>) + 'static) {
    let id = NEXT.replace(NEXT.get() + 1);
    let c = std::rc::Rc::new(changed);
    let c2 = c.clone();
    let st = StateChangedEventHandler::create(Box::new(move |op, _| {
        if let Some(o) = op.as_ref().filter(|o| state(o) == DONE) {
            remember(o);
        }
        c2(op.as_ref().filter(|o| state(o) == DONE).map(|_| id));
        Ok(())
    }));
    let mut token = 0i64;
    let _ = unsafe { op.add_StateChanged(&st, &mut token) };
    LIST.with_borrow_mut(|l| l.insert(0, (id, op)));
    c(Some(id));
}

/// Extension d'un nom de fichier, sans le point ("" s'il n'en a pas).
fn ext(file: &str) -> &str {
    name_ref(file).rsplit_once('.').map_or("", |x| x.1)
}

fn name_ref(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// Fenêtre « Enregistrer sous » pour chaque téléchargement qui attend, au nom et dans le dossier que le moteur proposait ;
/// fenêtre annulée : le fichier n'est pas téléchargé.
pub fn ask_waiting(hwnd: isize) {
    while let Some((a, d, changed)) = WAITING.with_borrow_mut(|l| (!l.is_empty()).then(|| l.remove(0))) {
        let proposed = text(|p| unsafe { a.ResultFilePath(p) });
        let e = ext(&proposed);
        unsafe {
            match crate::vault::pick_file(hwnd, true, &format!("*.{}", if e.is_empty() { "*" } else { e }), e, &proposed) {
                Some(p) => {
                    let _ = a.SetResultFilePath(&HSTRING::from(p.to_string_lossy().as_ref()));
                    if let Ok(op) = a.DownloadOperation() {
                        track(op, changed);
                    }
                }
                None => {
                    let _ = a.SetCancel(true);
                }
            }
            let _ = d.Complete();
        }
    }
}

/// [{id, name, path, site, got, total, state (0 en cours, 1 arrêté, 2 terminé), resume}] ; `total` vaut -1 quand le
/// site n'annonce pas la taille.
pub fn list() -> Value {
    let live: Vec<Value> = LIST.with_borrow(|l| {
        l.iter()
            .map(|(id, op)| unsafe {
                let (mut got, mut total, mut resume) = (0i64, 0i64, BOOL(0));
                let _ = (op.BytesReceived(&mut got), op.TotalBytesToReceive(&mut total), op.CanResume(&mut resume));
                let (p, url) = (path(op), text(|p| op.Uri(p)));
                // adresse en data: (fichier fabriqué par la page) : pas de site, et surtout pas le contenu dans la liste
                let site = crate::cmp::site(&url).filter(|_| url.starts_with("http")).unwrap_or_default();
                json!({"id": id, "name": name(&p), "path": p, "site": site, "got": got, "total": total, "state": state(op), "resume": resume.as_bool()})
            })
            .collect()
    });
    // puis l'historique (lancements précédents), sans doubler un fichier encore dans la liste de cette session
    let mut out = live.clone();
    PAST.with_borrow(|l| {
        for p in l.iter().filter(|p| !live.iter().any(|v| v["path"] == p.path.as_str())) {
            out.push(json!({"id": p.id, "name": p.name, "path": p.path, "site": p.site, "got": p.size, "total": p.size, "state": DONE, "resume": false}));
        }
    });
    Value::Array(out)
}

pub fn path_of(id: u32) -> Option<String> {
    LIST.with_borrow(|l| l.iter().find(|x| x.0 == id).map(|x| path(&x.1))).or_else(|| PAST.with_borrow(|l| l.iter().find(|x| x.id == id).map(|x| x.path.clone())))
}

/// "pause", "resume", "cancel", ou "forget" (retiré de la liste ; un téléchargement en cours est d'abord annulé).
pub fn act(id: u32, what: &str) {
    LIST.with_borrow_mut(|l| {
        let Some(i) = l.iter().position(|x| x.0 == id) else { return };
        let op = &l[i].1;
        let _ = unsafe {
            match what {
                "pause" => op.Pause(),
                "resume" => op.Resume(),
                "cancel" => op.Cancel(),
                _ if state(op) != DONE => op.Cancel(),
                _ => Ok(()),
            }
        };
        if what == "forget" {
            let p = path(&l[i].1);
            l.remove(i);
            PAST.with_borrow_mut(|h| h.retain(|x| x.path != p));
            save_past();
        }
    });
    // élément de l'historique (lancement précédent) : seul « retirer » le concerne
    if what == "forget" && PAST.with_borrow(|l| l.iter().any(|x| x.id == id)) {
        PAST.with_borrow_mut(|l| l.retain(|x| x.id != id));
        save_past();
    }
}

/// Vide la liste, sauf ce qui est encore en cours.
pub fn clear() {
    LIST.with_borrow_mut(|l| l.retain(|x| state(&x.1) == RUNNING));
    PAST.with_borrow_mut(|l| l.clear());
    save_past();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names() {
        assert_eq!(name(r"C:\Users\x\Downloads\rapport final.pdf"), "rapport final.pdf");
        assert_eq!(name("C:/a/b.zip"), "b.zip");
        assert_eq!(name("seul.txt"), "seul.txt");
        assert_eq!(name(""), "");
        assert_eq!(ext(r"C:\a.b\rapport final.PDF"), "PDF");
        assert_eq!(ext(r"C:\a.b\sans_extension"), "");
        assert_eq!(ext("archive.tar.gz"), "gz");
    }
}
