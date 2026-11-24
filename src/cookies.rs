// Cookies du profil (gestionnaire de cookies de WebView2, commun à tous les onglets) : comptes et suppression ;
// suppression des mots de passe enregistrés (données du même profil).
// Les valeurs ne sortent jamais d'ici : l'interface ne reçoit que des nombres.
use serde_json::{Value, json};
use webview2_com::{
    ClearBrowsingDataCompletedHandler, GetCookiesCompletedHandler,
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_BROWSING_DATA_KINDS_PASSWORD_AUTOSAVE, COREWEBVIEW2_COOKIE_SAME_SITE_KIND, ICoreWebView2_2, ICoreWebView2_13, ICoreWebView2Cookie,
        ICoreWebView2CookieManager, ICoreWebView2Profile2,
    },
    take_pwstr,
};
use windows::core::{HSTRING, Interface, PWSTR};
use wry::{WebView, WebViewExtWindows};

fn manager(wv: &WebView) -> Option<ICoreWebView2CookieManager> {
    unsafe { wv.webview().cast::<ICoreWebView2_2>().ok()?.CookieManager().ok() }
}

/// Cookies envoyés à `uri`, ceux du domaine parent compris ; "" = tous les cookies du profil.
fn get(m: &ICoreWebView2CookieManager, uri: &str, done: impl FnOnce(Vec<ICoreWebView2Cookie>) + 'static) {
    let h = GetCookiesCompletedHandler::create(Box::new(move |_, list| {
        let mut v = vec![];
        if let Some(l) = list {
            let mut n = 0;
            unsafe { l.Count(&mut n)? };
            for i in 0..n {
                v.push(unsafe { l.GetValueAtIndex(i)? });
            }
        }
        done(v);
        Ok(())
    }));
    let _ = unsafe { m.GetCookies(&HSTRING::from(uri), &h) };
}

fn domain(c: &ICoreWebView2Cookie) -> String {
    let mut p = PWSTR::null();
    unsafe { c.Domain(&mut p) }.map(|_| take_pwstr(p)).unwrap_or_default()
}

/// Nombre de sites : domaines distincts, sans le point initial ni « www. ».
fn sites(domains: impl Iterator<Item = String>) -> usize {
    let mut v: Vec<String> = domains
        .map(|d| {
            let d = d.trim_start_matches('.').to_ascii_lowercase();
            d.strip_prefix("www.").map(str::to_string).unwrap_or(d)
        })
        .collect();
    v.sort();
    v.dedup();
    v.len()
}

/// `done` reçoit {all, sites, site} ; `site` (cookies envoyés à `url`) vaut null hors d'une page web.
pub fn counts(wv: &WebView, url: &str, done: impl FnOnce(Value) + 'static) {
    let Some(m) = manager(wv) else { return };
    let m2 = m.clone();
    let total = move |site: Option<usize>| get(&m2, "", move |all| done(json!({"all": all.len(), "sites": sites(all.iter().map(domain)), "site": site})));
    if url.starts_with("http") { get(&m, url, move |v| total(Some(v.len()))) } else { total(None) }
}

/// Supprime les cookies que `url` recevrait (domaine parent compris), puis appelle `done`.
pub fn delete_site(wv: &WebView, url: &str, done: impl FnOnce() + 'static) {
    let Some(m) = manager(wv).filter(|_| url.starts_with("http")) else { return };
    let m2 = m.clone();
    get(&m, url, move |v| {
        for c in &v {
            let _ = unsafe { m2.DeleteCookie(c) };
        }
        // la première lecture après les suppressions revient encore avec les cookies supprimés (constaté) : on la perd ici
        get(&m2, "", move |_| done());
    });
}

/// Efface les identifiants enregistrés par le moteur (et rien d'autre) ; `done(réussi)` ; false si rien n'a pu partir.
pub fn clear_passwords(wv: &WebView, done: impl FnOnce(bool) + 'static) -> bool {
    let Ok(p) = (unsafe { wv.webview().cast::<ICoreWebView2_13>().and_then(|w| w.Profile()).and_then(|p| p.cast::<ICoreWebView2Profile2>()) }) else { return false };
    let h = ClearBrowsingDataCompletedHandler::create(Box::new(move |r| {
        done(r.is_ok());
        Ok(())
    }));
    unsafe { p.ClearBrowsingData(COREWEBVIEW2_BROWSING_DATA_KINDS_PASSWORD_AUTOSAVE, &h) }.is_ok()
}

pub fn delete_all(wv: &WebView, done: impl FnOnce()) {
    if let Some(m) = manager(wv) {
        let _ = unsafe { m.DeleteAllCookies() };
        done()
    }
}

/// Importe des cookies venus d'un autre navigateur (voir `chromium`). Rend le nombre réellement ajouté.
pub fn import(wv: &WebView, list: &[crate::chromium::Cookie]) -> usize {
    let Some(m) = manager(wv) else { return 0 };
    let mut n = 0;
    for c in list {
        let add = || -> windows::core::Result<()> {
            unsafe {
                let cookie = m.CreateCookie(&HSTRING::from(&c.name), &HSTRING::from(&c.value), &HSTRING::from(&c.host), &HSTRING::from(&c.path))?;
                let _ = cookie.SetIsSecure(c.secure);
                let _ = cookie.SetIsHttpOnly(c.http_only);
                let _ = cookie.SetSameSite(COREWEBVIEW2_COOKIE_SAME_SITE_KIND(c.same_site));
                if c.expires > 0.0 {
                    let _ = cookie.SetExpires(c.expires); // sinon cookie de session
                }
                m.AddOrUpdateCookie(&cookie)
            }
        };
        if add().is_ok() {
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    #[test]
    fn sites() {
        let d = [".google.com", "www.google.com", "google.com", "accounts.google.com", "Example.org"].map(String::from);
        assert_eq!(super::sites(d.into_iter()), 3);
        assert_eq!(super::sites(std::iter::empty()), 0);
    }
}
