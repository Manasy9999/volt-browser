// Extensions Chrome : téléchargement depuis le Chrome Web Store + gestion via l'API WebView2.
use std::{
    fs,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::{json, Value};
use webview2_com::{
    BrowserExtensionEnableCompletedHandler, BrowserExtensionRemoveCompletedHandler,
    Microsoft::Web::WebView2::Win32::*, ProfileAddBrowserExtensionCompletedHandler,
    ProfileGetBrowserExtensionsCompletedHandler, take_pwstr,
};
use windows::{
    Win32::System::Com::Urlmon::URLDownloadToFileW,
    core::{BOOL, HSTRING, Interface, PWSTR},
};
use wry::{WebView, WebViewExtWindows};

use crate::lang::t;

/// Extrait l'ID (32 lettres a-p) d'un ID brut ou d'une URL du Chrome Web Store.
pub fn parse_id(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut run = 0;
    for (i, c) in b.iter().enumerate() {
        run = if (b'a'..=b'p').contains(c) { run + 1 } else { 0 };
        if run == 32 && b.get(i + 1).is_none_or(|n| !(b'a'..=b'p').contains(n)) {
            return Some(s[i + 1 - 32..=i].to_string());
        }
    }
    None
}

/// Renvoie la partie ZIP d'un fichier CRX (v2 ou v3).
pub fn crx_zip(b: &[u8]) -> Result<&[u8], String> {
    let u = |o: usize| b.get(o..o + 4).map(|x| u32::from_le_bytes(x.try_into().unwrap()) as usize);
    if b.get(..4) != Some(b"Cr24") {
        return Err(t("fichier CRX invalide"));
    }
    let start = match u(4) {
        Some(3) => 12 + u(8).ok_or_else(|| t("CRX tronqué"))?,
        Some(2) => 16 + u(8).ok_or_else(|| t("CRX tronqué"))? + u(12).ok_or_else(|| t("CRX tronqué"))?,
        _ => return Err(t("version CRX inconnue")),
    };
    b.get(start..).ok_or_else(|| t("CRX tronqué"))
}

/// Télécharge et décompresse l'extension dans `dir/<id>`. Bloquant : à lancer hors du thread UI.
/// Utilise le téléchargeur (urlmon) et `tar.exe` de Windows : aucune pile HTTPS/zip dans l'exe.
pub fn download(id: &str, dir: &Path) -> Result<PathBuf, String> {
    let url = format!(
        "https://clients2.google.com/service/update2/crx?response=redirect&prodversion=131.0&acceptformat=crx2,crx3&x=id%3D{id}%26uc"
    );
    // Fichiers temporaires hors du dossier extensions (chaque entrée y est chargée comme extension).
    let tmp = std::env::temp_dir();
    let (crx, zip) = (tmp.join(format!("volt-{id}.crx")), tmp.join(format!("volt-{id}.zip")));
    let r = (|| {
        unsafe { URLDownloadToFileW(None, &HSTRING::from(url), &HSTRING::from(crx.as_path()), 0, None) }
            .map_err(|e| e.message())?;
        fs::write(&zip, crx_zip(&fs::read(&crx).map_err(|e| e.to_string())?)?).map_err(|e| e.to_string())?;
        let out = dir.join(id);
        let _ = fs::remove_dir_all(&out);
        fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        // bsdtar refuse par défaut les chemins absolus et les « .. » : pas d'écriture hors de `out`.
        let tar = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join(r"System32\tar.exe");
        let ok = Command::new(tar)
            .arg("-xf")
            .arg(&zip)
            .arg("-C")
            .arg(&out)
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            let _ = fs::remove_dir_all(&out);
            return Err(t("archive de l'extension illisible"));
        }
        // Chromium refuse de charger un dossier non empaqueté contenant `_metadata`.
        let _ = fs::remove_dir_all(out.join("_metadata"));
        Ok(out)
    })();
    let _ = fs::remove_file(&crx);
    let _ = fs::remove_file(&zip);
    r
}

fn profile(wv: &WebView) -> windows::core::Result<ICoreWebView2Profile7> {
    unsafe { wv.webview().cast::<ICoreWebView2_13>()?.Profile()?.cast() }
}

fn ext_id(e: &ICoreWebView2BrowserExtension) -> String {
    let mut p = PWSTR::null();
    unsafe { e.Id(&mut p).map(|_| take_pwstr(p)).unwrap_or_default() }
}

/// Installe le dossier et note l'ID WebView2 dedans (sert à retrouver la popup).
pub fn add(wv: &WebView, folder: PathBuf, done: impl FnOnce(Result<(), String>) + 'static) {
    let path = HSTRING::from(folder.as_path());
    let h = ProfileAddBrowserExtensionCompletedHandler::create(Box::new(move |r, e| {
        match (r, e) {
            (Ok(()), Some(e)) => {
                let _ = fs::write(folder.join(".volt_id"), ext_id(&e));
                done(Ok(()));
            }
            (r, _) => done(Err(r.err().map(|e| e.message()).unwrap_or_default())),
        }
        Ok(())
    }));
    if let Err(e) = profile(wv).and_then(|p| unsafe { p.AddBrowserExtension(&path, &h) }) {
        eprintln!("ajout extension : {e}");
    }
}

/// Liste asynchrone : `done` reçoit un tableau JSON [{id, name, on, store, icon, opts}] ; `store` : ID du Store,
/// `icon` : image data: (l'interface ne peut pas charger chrome-extension://), `opts` : page d'options.
/// Seules les extensions installées par Volt (dossier dans `dir`) : les internes du moteur restent cachées.
pub fn list(wv: &WebView, dir: PathBuf, done: impl FnOnce(Value) + 'static) {
    each(wv, move |exts| {
        done(Value::Array(
            exts.iter()
                .filter_map(|e| {
                    let id = ext_id(e);
                    let (f, m) = manifest(&dir, &id)?;
                    let (mut name, mut on) = (PWSTR::null(), BOOL(0));
                    unsafe {
                        let _ = e.Name(&mut name);
                        let _ = e.IsEnabled(&mut on);
                    }
                    let store = f.file_name()?.to_string_lossy().into_owned();
                    Some(json!({ "id": id, "name": take_pwstr(name), "on": on.as_bool(), "store": store, "icon": icon(&f, &m), "opts": options(&m).is_some() }))
                })
                .collect(),
        ))
    });
}

/// Fichier d'icône à afficher (~16 px, net en 200 %) : la plus petite d'au moins 32 px, sinon la plus grande.
fn icon_path(m: &Value) -> Option<String> {
    let v = [&m["action"]["default_icon"], &m["browser_action"]["default_icon"], &m["icons"]].into_iter().find(|v| !v.is_null())?;
    let p = match v {
        Value::String(s) => s.clone(),
        Value::Object(o) => {
            let mut s: Vec<(u32, &str)> = o.iter().filter_map(|(k, v)| Some((k.parse().ok()?, v.as_str()?))).collect();
            s.sort();
            s.iter().find(|x| x.0 >= 32).or(s.last())?.1.to_string()
        }
        _ => return None,
    };
    // chemin écrit par l'extension : rien hors de son dossier
    (!p.contains("..")).then(|| p.trim_start_matches('/').to_string())
}

fn icon(folder: &Path, m: &Value) -> Option<String> {
    let p = icon_path(m)?;
    let b = fs::read(folder.join(&p)).ok()?;
    let mime = match p.rsplit('.').next().unwrap_or_default().to_ascii_lowercase().as_str() {
        "svg" => "image/svg+xml",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "image/png",
    };
    Some(format!("data:{mime};base64,{}", b64(&b)))
}

fn b64(b: &[u8]) -> String {
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

/// ID du Chrome Web Store (nom du dossier) de l'extension `id` (identifiant WebView2).
pub fn store_id(dir: &Path, id: &str) -> Option<String> {
    Some(folder_of(dir, id)?.file_name()?.to_string_lossy().into_owned())
}

pub fn remove(wv: &WebView, id: String, dir: PathBuf, done: impl FnOnce() + 'static) {
    if let Some(f) = folder_of(&dir, &id) {
        let _ = fs::remove_dir_all(f);
    }
    each(wv, move |exts| {
        if let Some(e) = exts.iter().find(|e| ext_id(e) == id) {
            let h = BrowserExtensionRemoveCompletedHandler::create(Box::new(move |_| {
                done();
                Ok(())
            }));
            let _ = unsafe { e.Remove(&h) };
        }
    });
}

pub fn enable(wv: &WebView, id: String, on: bool, done: impl FnOnce() + 'static) {
    each(wv, move |exts| {
        if let Some(e) = exts.iter().find(|e| ext_id(e) == id) {
            let h = BrowserExtensionEnableCompletedHandler::create(Box::new(move |_| {
                done();
                Ok(())
            }));
            let _ = unsafe { e.Enable(on, &h) };
        }
    });
}

fn each(wv: &WebView, f: impl FnOnce(Vec<ICoreWebView2BrowserExtension>) + 'static) {
    let h = ProfileGetBrowserExtensionsCompletedHandler::create(Box::new(move |r, l| {
        r?;
        let mut v = vec![];
        if let Some(l) = l {
            let mut n = 0;
            unsafe { l.Count(&mut n)? };
            for i in 0..n {
                v.push(unsafe { l.GetValueAtIndex(i)? });
            }
        }
        f(v);
        Ok(())
    }));
    if let Err(e) = profile(wv).and_then(|p| unsafe { p.GetBrowserExtensions(&h) }) {
        eprintln!("liste extensions : {e}");
    }
}

fn folder_of(dir: &Path, id: &str) -> Option<PathBuf> {
    fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        fs::read_to_string(p.join(".volt_id")).is_ok_and(|s| s == id)
    })
}

fn manifest(dir: &Path, id: &str) -> Option<(PathBuf, Value)> {
    let f = folder_of(dir, id)?;
    let m = serde_json::from_str(&fs::read_to_string(f.join("manifest.json")).ok()?).ok()?;
    Some((f, m))
}

fn options(m: &Value) -> Option<&str> {
    m["options_ui"]["page"].as_str().or(m["options_page"].as_str())
}

/// Page d'options de l'extension, ouverte dans un onglet. Pas la popup : WebView2 ne sait pas l'afficher, et dans
/// un onglet elle agirait sur elle-même.
pub fn options_url(dir: &Path, id: &str) -> Option<String> {
    let (_, m) = manifest(dir, id)?;
    Some(format!("chrome-extension://{id}/{}", options(&m)?.trim_start_matches('/')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        let id = "cjpalhdlnbpafiamejdnhcphjbkeiagm";
        assert_eq!(parse_id(id).as_deref(), Some(id));
        assert_eq!(
            parse_id(&format!("https://chromewebstore.google.com/detail/ublock-origin/{id}?hl=fr")).as_deref(),
            Some(id)
        );
        assert_eq!(parse_id("trop-court"), None);
    }

    // Réseau requis : cargo test -- --ignored
    #[test]
    #[ignore]
    fn real_store_download() {
        let dir = std::env::temp_dir().join("volt-ext-test");
        let _ = fs::remove_dir_all(&dir);
        let out = download("ddkjiahejlhfcafbddmgiahcphecmpfh", &dir).expect("téléchargement uBlock Origin Lite");
        assert!(out.join("manifest.json").exists());
        assert!(!out.join("_metadata").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn icons() {
        let m = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        assert_eq!(icon_path(&m(r#"{"action":{"default_icon":{"16":"i16.png","64":"/i64.png","32":"i32.png"}},"icons":{"128":"x.png"}}"#)).as_deref(), Some("i32.png"));
        assert_eq!(icon_path(&m(r#"{"icons":{"16":"a.png","19":"/b.png"}}"#)).as_deref(), Some("b.png")); // rien >= 32 : la plus grande
        assert_eq!(icon_path(&m(r#"{"browser_action":{"default_icon":"ic.png"}}"#)).as_deref(), Some("ic.png"));
        assert_eq!(icon_path(&m(r#"{"icons":{"32":"../../secret.png"}}"#)), None);
        assert_eq!(icon_path(&m(r#"{"name":"x"}"#)), None);
        assert_eq!([b64(b""), b64(b"f"), b64(b"fo"), b64(b"foo"), b64(b"foob")], ["", "Zg==", "Zm8=", "Zm9v", "Zm9vYg=="]);
        assert_eq!(b64(&[0xfb, 0xff]), "+/8=");
    }

    #[test]
    fn crx() {
        let mut v3 = b"Cr24".to_vec();
        v3.extend(3u32.to_le_bytes());
        v3.extend(2u32.to_le_bytes());
        v3.extend(b"hhZIP");
        assert_eq!(crx_zip(&v3).unwrap(), b"ZIP");
        assert!(crx_zip(b"PK..").is_err());
    }
}
