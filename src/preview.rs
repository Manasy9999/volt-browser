// Aperçu d'onglet : capture JPEG de la page, affichée au retour sur un onglet déchargé le temps qu'il recharge ;
// sans capture, une page d'attente faite de ce que Volt sait de l'onglet.
use std::path::Path;

use webview2_com::{CapturePreviewCompletedHandler, Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_JPEG};
use windows::{
    Win32::{
        System::Com::{STGM_CREATE, STGM_WRITE},
        UI::Shell::SHCreateStreamOnFileEx,
    },
    core::HSTRING,
};
use wry::{WebView, WebViewExtWindows};

// NavigateToString est limité à 2 Mo ; le base64 grossit d'un tiers.
const MAX_JPEG: usize = 1_400_000;

/// Écrit la capture de `wv` dans `path` (asynchrone) ; fichier supprimé si la capture échoue.
pub fn capture(wv: &WebView, path: &Path) {
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    // 0x80 = FILE_ATTRIBUTE_NORMAL
    let Ok(stream) = (unsafe { SHCreateStreamOnFileEx(&HSTRING::from(path), (STGM_CREATE | STGM_WRITE).0, 0x80, true, None) }) else { return };
    let (keep, p) = (stream.clone(), path.to_path_buf());
    let h = CapturePreviewCompletedHandler::create(Box::new(move |r| {
        drop(keep); // ferme le fichier : le flux doit vivre jusqu'à la fin de l'écriture
        if r.is_err() {
            let _ = std::fs::remove_file(&p);
        }
        Ok(())
    }));
    if unsafe { wv.webview().CapturePreview(COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_JPEG, &stream, &h) }.is_err() {
        drop(stream);
        let _ = std::fs::remove_file(path);
    }
    // couleur de fond de la page, à côté de la capture : fond du moteur au retour, à la place du gris entre l'aperçu et la page
    let bg = path.with_extension("bg");
    let _ = wv.evaluate_script_with_callback(BG_JS, move |c| {
        let _ = std::fs::write(&bg, c);
    });
}

const BG_JS: &str = "[document.body,document.documentElement].map(e=>e&&getComputedStyle(e).backgroundColor).find(c=>c&&!/^rgba\\(.*, 0\\)$|transparent/.test(c))||''";

/// Couleur relevée par `capture` (« rgb(r, g, b) », en JSON), opaque seulement.
pub fn bg(path: &Path) -> Option<(u8, u8, u8, u8)> {
    let s: String = serde_json::from_slice(&std::fs::read(path.with_extension("bg")).ok()?).ok()?;
    let v: Vec<u8> = s.strip_prefix("rgb(")?.strip_suffix(')')?.split(',').map(|x| x.trim().parse().ok()).collect::<Option<_>>()?;
    (v.len() == 3).then(|| (v[0], v[1], v[2], 255))
}

/// Page qui montre la capture puis part vers la vraie URL ; None si pas d'aperçu utilisable.
pub fn page(path: &Path, url: &str) -> Option<String> {
    let img = std::fs::read(path).ok().filter(|b| !b.is_empty() && b.len() <= MAX_JPEG)?;
    // URL venue du web : chaîne JSON, et `<` échappé pour qu'un `</script>` ne ferme pas la balise.
    let u = serde_json::to_string(url).ok()?.replace('<', "\\u003c");
    let (r, g, b, _) = bg(path).unwrap_or((46, 46, 51, 255));
    // replace : pas d'entrée d'historique ; après une image peinte, sinon l'aperçu n'apparaît jamais
    Some(format!(
        r#"<!doctype html><body style="margin:0;background:rgb({r},{g},{b})"><img src="data:image/jpeg;base64,{}" style="display:block;width:100%"><script>onload=()=>requestAnimationFrame(()=>setTimeout(()=>location.replace({u})))</script>"#,
        base64(&img)
    ))
}

fn base64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            s.push(if i <= c.len() { A[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_rfc4648() {
        for (i, o) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64(i.as_bytes()), o);
        }
        assert_eq!(base64(&[0xff, 0xfe, 0x00]), "//4A");
    }

    #[test]
    fn hostile_url_stays_in_string() {
        let d = std::env::temp_dir().join(format!("volt_preview_test_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("1.jpg");
        std::fs::write(&f, b"jpg").unwrap();
        let p = page(&f, "https://x.fr/</script><script>alert(1)</script>\"'").unwrap();
        assert_eq!(p.matches("</script>").count(), 1); // seulement la nôtre
        assert!(p.contains(&format!("location.replace(\"https://x.fr/{l}/script>{l}script>alert(1){l}/script>\\\"'\")", l = "\\u003c")));
        std::fs::write(&f, vec![0u8; MAX_JPEG + 1]).unwrap();
        assert!(page(&f, "https://x.fr").is_none()); // trop gros pour NavigateToString
        assert!(page(&d.join("absent.jpg"), "https://x.fr").is_none());
        // couleur de fond relevée : opaque seulement, sinon le gris habituel
        assert!(bg(&f).is_none());
        for (c, want) in [(r#""rgb(255, 255, 255)""#, Some((255, 255, 255, 255))), (r#""rgba(0, 0, 0, 0)""#, None), (r#""""#, None), ("{cass", None), (r#""rgb(1, 2)""#, None)] {
            std::fs::write(d.join("1.bg"), c).unwrap();
            assert_eq!(bg(&f), want, "{c}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
