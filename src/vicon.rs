//! Logo d'une vignette, trouvé AUTOMATIQUEMENT à partir du lien de la carte (n'importe quelle URL), côté Rust car CORS empêche
//! la page de lire le HTML/manifeste cross-origin. UNIQUEMENT l'origine du site de la carte (aucun service d'icônes tiers).
//! On lit les icônes que la page DÉCLARE (`<link rel="icon"|"apple-touch-icon">`, et le tableau `icons` du `<link rel="manifest">`),
//! on prend la plus grande (viser >= 256, idéalement 512), replis `/apple-touch-icon.png` puis `/favicon.ico`, puis l'initiale
//! (côté interface). On télécharge l'icône choisie, on calcule sa couleur dominante (WIC, décodage sans CORS), et on met URL +
//! couleur + taille en cache sur disque par origine (vicons.json). Récupération paresseuse et asynchrone : la page s'ouvre tout
//! de suite avec l'initiale, le logo et la couleur arrivent après.
use std::{collections::HashMap, fs, path::Path, sync::Mutex};

use serde_json::Value;
use windows::{
    Win32::{
        Graphics::Imaging::{
            CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGRA, IWICImagingFactory, WICBitmapDitherTypeNone,
            WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
        },
        System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, Urlmon::URLDownloadToFileW},
    },
    core::HSTRING,
};

const MAX: u64 = 3_000_000; // plafond par téléchargement (~3 Mo)

static CACHE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn fnv(s: &str) -> u64 {
    let mut h = 1469598103934665603u64;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

/// Origine (schéma://hôte) d'une URL http(s).
fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty()).then(|| format!("{scheme}://{host}"))
}

/// Télécharge une URL (<= MAX octets) dans un fichier temporaire unique. Renvoie le chemin (à supprimer par l'appelant).
fn download(url: &str) -> Option<std::path::PathBuf> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return None;
    }
    let tmp = std::env::temp_dir().join(format!("volt-vi-{:016x}.tmp", fnv(url)));
    unsafe { URLDownloadToFileW(None, &HSTRING::from(url), &HSTRING::from(tmp.as_path()), 0, None) }.ok()?;
    match fs::metadata(&tmp) {
        Ok(m) if m.len() > 0 && m.len() <= MAX => Some(tmp),
        _ => {
            let _ = fs::remove_file(&tmp);
            None
        }
    }
}

fn fetch(url: &str) -> Option<Vec<u8>> {
    let p = download(url)?;
    let b = fs::read(&p).ok();
    let _ = fs::remove_file(&p);
    b
}

/// Octets d'une image connue (sinon None) ; évite de prendre une page HTML d'erreur pour un logo.
fn is_image(b: &[u8]) -> bool {
    b.starts_with(b"\x89PNG")
        || b.starts_with(b"\xFF\xD8\xFF")
        || b.starts_with(b"GIF8")
        || b.starts_with(b"\x00\x00\x01\x00")
        || b.starts_with(b"BM")
        || (b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP")
        || b.windows(4).take(400).any(|w| w == b"<svg")
}

/// « 512x512 256x256 » -> 512 (plus grande dimension déclarée) ; « any » (SVG) -> grande valeur.
fn max_dim(sizes: &str) -> u32 {
    if sizes.trim().eq_ignore_ascii_case("any") {
        return 1024;
    }
    sizes.split_whitespace().filter_map(|t| t.split(['x', 'X']).filter_map(|n| n.parse::<u32>().ok()).max()).max().unwrap_or(0)
}

/// Résout une URL relative (`base` = URL du document qui la déclare). Garde l'icône déclarée par le site lui-même.
fn abs(origin: &str, base: &str, src: &str) -> Option<String> {
    let src = src.trim();
    if src.is_empty() {
        return None;
    }
    Some(if src.starts_with("http") {
        src.to_string()
    } else if let Some(p) = src.strip_prefix("//") {
        format!("{}://{p}", origin.split("://").next()?)
    } else if let Some(p) = src.strip_prefix('/') {
        format!("{origin}/{p}")
    } else {
        let b = base.rsplit_once('/').map_or(origin, |x| x.0);
        format!("{b}/{src}")
    })
}

/// Valeur d'un attribut (`name="..."` ou `name='...'`) dans une balise.
fn attr(tag: &str, name: &str) -> Option<String> {
    let low = tag.to_ascii_lowercase();
    let (i, quote) = low
        .find(&format!("{name}=\""))
        .map(|i| (i + name.len() + 2, '"'))
        .or_else(|| low.find(&format!("{name}='")).map(|i| (i + name.len() + 2, '\'')))?;
    let rest = &tag[i..];
    rest.find(quote).map(|e| rest[..e].to_string())
}

/// Icônes déclarées par une page : balises `<link>` (rel icon / apple-touch-icon / manifest). Renvoie (candidats (url, taille),
/// url du manifeste éventuel). On ne décode pas encore : on choisira la plus grande, on téléchargera, on validera.
fn page_icons(origin: &str, page: &str, html: &str) -> (Vec<(String, u32)>, Option<String>) {
    let low = html.to_ascii_lowercase();
    let (mut icons, mut manifest) = (vec![], None);
    let mut pos = 0;
    while let Some(off) = low[pos..].find("<link") {
        let start = pos + off;
        let end = low[start..].find('>').map_or(low.len(), |e| start + e);
        let (tag, tag_low) = (&html[start..end], &low[start..end]);
        pos = end + 1;
        let Some(rel) = attr(tag_low, "rel") else { continue };
        let Some(href) = attr(tag, "href").and_then(|h| abs(origin, page, &h)) else { continue };
        if rel.contains("manifest") {
            manifest = Some(href);
        } else if rel.contains("icon") {
            // apple-touch sans `sizes` : ~180 px ; .svg : scalable (grande valeur)
            let mut sz = attr(tag_low, "sizes").map_or(0, |s| max_dim(&s));
            if sz == 0 {
                sz = if href.to_ascii_lowercase().contains(".svg") { 1024 } else if rel.contains("apple-touch") { 180 } else { 32 };
            }
            icons.push((href, sz));
        }
    }
    (icons, manifest)
}

/// Icônes d'un manifeste web (tableau `icons`).
fn manifest_icons(origin: &str, murl: &str, body: &[u8]) -> Vec<(String, u32)> {
    let Ok(v) = serde_json::from_slice::<Value>(body) else { return vec![] };
    v["icons"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|ic| {
                    let url = abs(origin, murl, ic["src"].as_str()?)?;
                    Some((url, ic["sizes"].as_str().map_or(0, max_dim)))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Couleur dominante d'un fichier image (moyenne par réduction WIC à 1x1) : "#rrggbb", ou None (format non décodé / transparent).
fn dominant_color(path: &Path) -> Option<String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED); // ignoré si déjà initialisé
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let dec = factory
            .CreateDecoderFromFilename(&HSTRING::from(path), None, windows::Win32::Foundation::GENERIC_READ, WICDecodeMetadataCacheOnDemand)
            .ok()?;
        let frame = dec.GetFrame(0).ok()?;
        // réduction à 1x1 (moyenne), puis conversion en BGRA
        let scaler = factory.CreateBitmapScaler().ok()?;
        scaler.Initialize(&frame, 1, 1, WICBitmapInterpolationModeFant).ok()?;
        let conv = factory.CreateFormatConverter().ok()?;
        conv.Initialize(&scaler, &GUID_WICPixelFormat32bppBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).ok()?;
        let mut px = [0u8; 4];
        conv.CopyPixels(std::ptr::null(), 4, &mut px).ok()?;
        let (b, g, r, a) = (px[0], px[1], px[2], px[3]);
        (a >= 16).then(|| format!("#{r:02x}{g:02x}{b:02x}"))
    }
}

/// Télécharge le premier candidat qui est bien une image ; renvoie (url, couleur dominante, taille déclarée).
fn pick(cands: &[(String, u32)]) -> Option<(String, Option<String>, u32)> {
    // la plus grande d'abord
    let mut ord: Vec<&(String, u32)> = cands.iter().collect();
    ord.sort_by(|a, b| b.1.cmp(&a.1));
    for (url, sz) in ord {
        let Some(path) = download(url) else { continue };
        let ok = fs::read(&path).ok().is_some_and(|b| is_image(&b));
        let color = if ok { dominant_color(&path) } else { None };
        let _ = fs::remove_file(&path);
        if ok {
            return Some((url.clone(), color, *sz));
        }
    }
    None
}

/// Résout le meilleur logo (URL, couleur, taille) depuis les icônes déclarées par la page, sinon les chemins usuels.
fn resolve(origin: &str) -> Option<(String, Option<String>, u32)> {
    let page = format!("{origin}/");
    let mut cands: Vec<(String, u32)> = vec![];
    if let Some(html) = fetch(&page).map(|b| String::from_utf8_lossy(&b).into_owned()) {
        let (mut icons, manifest) = page_icons(origin, &page, &html);
        cands.append(&mut icons);
        if let Some(murl) = manifest
            && let Some(body) = fetch(&murl)
        {
            cands.append(&mut manifest_icons(origin, &murl, &body));
        }
    }
    // replis : icônes aux chemins usuels
    cands.push((format!("{origin}/apple-touch-icon.png"), 180));
    cands.push((format!("{origin}/favicon.ico"), 32));
    pick(&cands)
}

/// Logo d'une vignette : (url de l'icône, couleur dominante "#rrggbb" ou ""), depuis le cache disque ou par le réseau.
/// `dim` reçoit la taille obtenue (pour le rapport). None si rien d'exploitable (l'interface garde l'initiale).
pub fn best(data_dir: &Path, url: &str, dim: &mut u32) -> Option<(String, String)> {
    let origin = origin_of(url)?;
    let path = data_dir.join("vicons.json");
    {
        let mut g = CACHE.lock().ok()?;
        if g.is_none() {
            *g = Some(fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default());
        }
        if let Some(v) = g.as_ref().unwrap().get(&origin) {
            // format "dim\tcolor\turl" ; "" = déjà cherché sans succès (pas de nouveau réseau)
            let mut it = v.split('\t');
            let d: u32 = it.next().and_then(|d| d.parse().ok()).unwrap_or(0);
            let color = it.next().unwrap_or("").to_string();
            let u = it.next().unwrap_or("");
            *dim = d;
            return (!u.is_empty()).then(|| (u.to_string(), color));
        }
    }
    let found = resolve(&origin);
    *dim = found.as_ref().map_or(0, |(_, _, d)| *d);
    let mut g = CACHE.lock().ok()?;
    let map = g.as_mut().unwrap();
    map.insert(origin, found.as_ref().map_or_else(String::new, |(u, c, d)| format!("{d}\t{}\t{u}", c.as_deref().unwrap_or(""))));
    let _ = fs::write(&path, serde_json::to_vec(map).unwrap());
    found.map(|(u, c, _)| (u, c.unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing() {
        assert_eq!(max_dim("512x512"), 512);
        assert_eq!(max_dim("48x48 192x192 96x96"), 192);
        assert_eq!(max_dim("any"), 1024);
        assert_eq!(origin_of("https://www.youtube.com/watch?v=1").as_deref(), Some("https://www.youtube.com"));
        assert_eq!(origin_of("ftp://x").as_deref(), None);
        let o = "https://site.fr";
        assert_eq!(abs(o, "https://site.fr/", "/icons/512.png").as_deref(), Some("https://site.fr/icons/512.png"));
        assert_eq!(abs(o, "https://site.fr/", "https://cdn.site.fr/i.png").as_deref(), Some("https://cdn.site.fr/i.png"));
        assert_eq!(abs(o, "https://site.fr/app/manifest.json", "i.png").as_deref(), Some("https://site.fr/app/i.png"));
        // icônes déclarées par une page : apple-touch (180 par défaut), icon avec sizes, manifest repéré
        let html = r#"<link rel="apple-touch-icon" href="/at.png"><link rel="icon" sizes="32x32" href="/f.png"><link rel="manifest" href="/m.webmanifest">"#;
        let (icons, man) = page_icons(o, "https://site.fr/", html);
        assert_eq!(man.as_deref(), Some("https://site.fr/m.webmanifest"));
        assert!(icons.contains(&("https://site.fr/at.png".into(), 180)) && icons.contains(&("https://site.fr/f.png".into(), 32)));
        let mi = manifest_icons(o, "https://site.fr/m.webmanifest", br#"{"icons":[{"src":"/i512.png","sizes":"512x512"}]}"#);
        assert_eq!(mi, vec![("https://site.fr/i512.png".into(), 512)]);
        assert!(is_image(b"\x89PNG\r\n") && is_image(b"\xff\xd8\xff\xe0") && !is_image(b"<html>"));
    }
}
