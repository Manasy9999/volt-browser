// Son par onglet : détection « joue de l'audio » et sourdine, via l'API WebView2.
use webview2_com::{
    ContainsFullScreenElementChangedEventHandler, IsDocumentPlayingAudioChangedEventHandler, Microsoft::Web::WebView2::Win32::ICoreWebView2_8,
};
use windows::core::{BOOL, Interface};
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

pub fn set_muted(wv: &WebView, muted: bool) {
    if let Ok(w) = wv8(wv) {
        let _ = unsafe { w.SetIsMuted(muted) };
    }
}
