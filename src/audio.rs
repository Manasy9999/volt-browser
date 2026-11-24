// Son par onglet (détection « joue de l'audio », sourdine), plein écran et icône du site, via l'API WebView2.
use webview2_com::{
    ContainsFullScreenElementChangedEventHandler, FaviconChangedEventHandler, FindStartCompletedHandler, IsDocumentPlayingAudioChangedEventHandler,
    GetNonDefaultPermissionSettingsCompletedHandler, NewWindowRequestedEventHandler, SetPermissionStateCompletedHandler, WebMessageReceivedEventHandler,
    ZoomFactorChangedEventHandler,
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_STATE, COREWEBVIEW2_PERMISSION_STATE_ALLOW, COREWEBVIEW2_PERMISSION_STATE_DEFAULT, ICoreWebView2Profile4,
    },
    Microsoft::Web::WebView2::Win32::{ICoreWebView2_8, ICoreWebView2_13, ICoreWebView2_15, ICoreWebView2_28, ICoreWebView2Environment15},
    take_pwstr,
};
use windows::core::{BOOL, HSTRING, Interface, PWSTR};
use wry::{WebView, WebViewExtWindows};

fn wv8(wv: &WebView) -> windows::core::Result<ICoreWebView2_8> {
    wv.webview().cast()
}

/// Appelle `f(joue_du_son)` à chaque changement.
pub fn watch(wv: &WebView, f: impl Fn(bool) + 'static) {
    let Ok(w) = wv8(wv) else { return };
    let h = IsDocumentPlayingAudioChangedEventHandler::create(Box::new(move |sender, _| {
        let mut on = BOOL(0);
        if let Some(s) = sender.and_then(|s| s.cast::<ICoreWebView2_8>().ok()) {
            unsafe { s.IsDocumentPlayingAudio(&mut on)? };
        }
        f(on.as_bool());
        Ok(())
    }));
    let mut token = 0i64;
    let _ = unsafe { w.add_IsDocumentPlayingAudioChanged(&h, &mut token) };
}

/// Appelle `f(plein_écran)` quand la page entre ou sort du plein écran (vidéo YouTube, etc.).
pub fn watch_fullscreen(wv: &WebView, f: impl Fn(bool) + 'static) {
    let h = ContainsFullScreenElementChangedEventHandler::create(Box::new(move |sender, _| {
        let mut on = BOOL(0);
        if let Some(s) = sender {
            unsafe { s.ContainsFullScreenElement(&mut on)? };
        }
        f(on.as_bool());
        Ok(())
    }));
    let mut token = 0i64;
    let _ = unsafe { wv.webview().add_ContainsFullScreenElementChanged(&h, &mut token) };
}

/// Appelle `f(adresse)` quand la page annonce son icône (balise <link rel="icon">, ou /favicon.ico trouvé par le moteur).
pub fn watch_icon(wv: &WebView, f: impl Fn(String) + 'static) {
    let Ok(w) = wv.webview().cast::<ICoreWebView2_15>() else { return };
    let h = FaviconChangedEventHandler::create(Box::new(move |sender, _| {
        if let Some(s) = sender.and_then(|s| s.cast::<ICoreWebView2_15>().ok()) {
            let mut p = PWSTR::null();
            unsafe { s.FaviconUri(&mut p)? };
            f(take_pwstr(p));
        }
        Ok(())
    }));
    let mut token = 0i64;
    let _ = unsafe { w.add_FaviconChanged(&h, &mut token) };
}

/// La page demande une nouvelle fenêtre : rien ne s'ouvre tout seul, `f(adresse, voulu)` décide. `voulu` : la demande
/// vient d'un geste de l'utilisateur (clic sur un lien) et non d'un script lancé tout seul (fenêtre surgissante).
pub fn watch_new_window(wv: &WebView, f: impl Fn(String, bool) + 'static) {
    let h = NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
        if let Some(a) = args {
            let (mut uri, mut user) = (PWSTR::null(), BOOL(0));
            unsafe {
                a.Uri(&mut uri)?;
                a.IsUserInitiated(&mut user)?;
                a.SetHandled(true)?;
            }
            f(take_pwstr(uri), user.as_bool());
        }
        Ok(())
    }));
    let mut token = 0i64;
    let _ = unsafe { wv.webview().add_NewWindowRequested(&h, &mut token) };
}

/// Appelle `f(facteur)` quand le zoom de la page change (Ctrl + molette, Ctrl + / −) ; 1.0 = 100 %.
pub fn watch_zoom(wv: &WebView, f: impl Fn(f64) + 'static) {
    let h = ZoomFactorChangedEventHandler::create(Box::new(move |sender, _| {
        if let Some(c) = sender {
            let mut z = 1.0f64;
            unsafe { c.ZoomFactor(&mut z)? };
            f(z);
        }
        Ok(())
    }));
    let mut token = 0i64;
    let _ = unsafe { wv.controller().add_ZoomFactorChanged(&h, &mut token) };
}

pub fn set_zoom(wv: &WebView, z: f64) {
    let _ = unsafe { wv.controller().SetZoomFactor(z) };
}

/// Réglages du moteur que wry coupe d'office : adresse du lien survolé affichée en bas de la page.
pub fn engine_defaults(wv: &WebView) {
    if let Ok(st) = unsafe { wv.webview().Settings() } {
        let _ = unsafe { st.SetIsStatusBarEnabled(true) };
    }
}

/// Ctrl+F : barre de recherche du moteur sur cette page (compteur, suivant, précédent). Son propre raccourci n'ouvre
/// rien dans une vue intégrée : on la demande nous-mêmes. Lancée sans texte, elle s'affiche et attend la saisie.
pub fn find(wv: &WebView) {
    unsafe {
        let (Ok(w), Ok(env)) = (wv.webview().cast::<ICoreWebView2_28>(), wv.environment().cast::<ICoreWebView2Environment15>()) else { return };
        let (Ok(f), Ok(o)) = (w.Find(), env.CreateFindOptions()) else { return };
        let _ = f.Start(&o, &FindStartCompletedHandler::create(Box::new(|_| Ok(()))));
    }
}

/// Dossier où le profil de cette vue enregistre les téléchargements.
pub fn set_download_dir(wv: &WebView, dir: &str) {
    if let Ok(p) = unsafe { wv.webview().cast::<ICoreWebView2_13>().and_then(|w| w.Profile()) } {
        let _ = unsafe { p.SetDefaultDownloadFolderPath(&HSTRING::from(dir)) };
    }
}

/// Messages envoyés par la page (`window.ipc.postMessage`) : `f(texte)`. Reçus ici directement du moteur, parce que wry
/// jette en silence ceux d'une page locale (file://) : leur adresse ne passe pas son analyseur d'adresses web. Sans ça,
/// ni les raccourcis de Volt ni la page de signature ne fonctionnent sur un fichier du PC.
pub fn watch_messages(wv: &WebView, f: impl Fn(String) + 'static) {
    let h = WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
        if let Some(a) = args {
            let mut s = PWSTR::null();
            if unsafe { a.TryGetWebMessageAsString(&mut s) }.is_ok() {
                f(take_pwstr(s));
            }
        }
        Ok(())
    }));
    let mut token = 0i64;
    let _ = unsafe { wv.webview().add_WebMessageReceived(&h, &mut token) };
}

fn profile4(wv: &WebView) -> windows::core::Result<ICoreWebView2Profile4> {
    unsafe { wv.webview().cast::<ICoreWebView2_13>()?.Profile()?.cast() }
}

/// Ce que l'utilisateur a autorisé ou refusé aux sites (caméra, micro, position, notifications…), tel que le moteur le
/// garde dans ce profil : `f([{site, kind, allow}])`, `kind` étant le numéro de l'autorisation dans le moteur.
pub fn permissions(wv: &WebView, f: impl Fn(serde_json::Value) + 'static) {
    let Ok(p) = profile4(wv) else { return f(serde_json::json!([])) };
    let h = GetNonDefaultPermissionSettingsCompletedHandler::create(Box::new(move |_, view| {
        let mut out = vec![];
        if let Some(v) = view {
            let mut n = 0u32;
            unsafe { v.Count(&mut n)? };
            for i in 0..n {
                let (mut k, mut st, mut o) = (COREWEBVIEW2_PERMISSION_KIND::default(), COREWEBVIEW2_PERMISSION_STATE::default(), PWSTR::null());
                unsafe {
                    let s = v.GetValueAtIndex(i)?;
                    s.PermissionKind(&mut k)?;
                    s.PermissionState(&mut st)?;
                    s.PermissionOrigin(&mut o)?;
                }
                out.push(serde_json::json!({"site": take_pwstr(o), "kind": k.0, "allow": st == COREWEBVIEW2_PERMISSION_STATE_ALLOW}));
            }
        }
        f(serde_json::json!(out));
        Ok(())
    }));
    let _ = unsafe { p.GetNonDefaultPermissionSettings(&h) };
}

/// Oublie le choix fait pour ce site et cette autorisation : le site devra redemander.
pub fn forget_permission(wv: &WebView, kind: i32, site: &str) {
    if let Ok(p) = profile4(wv) {
        let h = SetPermissionStateCompletedHandler::create(Box::new(|_| Ok(())));
        let _ = unsafe { p.SetPermissionState(COREWEBVIEW2_PERMISSION_KIND(kind), &HSTRING::from(site), COREWEBVIEW2_PERMISSION_STATE_DEFAULT, &h) };
    }
}

pub fn set_muted(wv: &WebView, muted: bool) {
    if let Ok(w) = wv8(wv) {
        let _ = unsafe { w.SetIsMuted(muted) };
    }
}
