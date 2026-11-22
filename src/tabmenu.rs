// Clic droit sur un onglet, clic sur l'icône d'une extension : entrées des menus (pures), menu natif Windows
// (la page web cacherait un menu HTML), et logique d'ordre des onglets (épinglés en tête, fermetures, pile « Rouvrir »).
use windows::{
    Win32::{
        Foundation::{HWND, POINT},
        System::LibraryLoader::{GetProcAddress, LoadLibraryW},
        UI::WindowsAndMessaging::*,
    },
    core::{HSTRING, PCSTR, w},
};

use crate::Tab;

/// (action, libellé, grisé) ; None = séparateur.
pub type Item = Option<(&'static str, &'static str, bool)>;
const CLOSED_MAX: usize = 25;

/// Entrées du menu de l'onglet `id`, dans l'ordre de Chrome.
pub fn items(tabs: &[Tab], id: u32, fav: bool, can_reopen: bool) -> Vec<Item> {
    let (pinned, muted) = tabs.iter().find(|t| t.id == id).map_or((false, false), |t| (t.pinned, t.muted));
    vec![
        Some(("new_right", "Nouvel onglet à droite", false)),
        None,
        Some(("reload", "Actualiser", false)),
        Some(("dup", "Dupliquer", false)),
        Some(("pin", if pinned { "Retirer l'épingle" } else { "Épingler" }, false)),
        Some(("mute", if muted { "Réactiver le son de l'onglet" } else { "Couper le son de l'onglet" }, false)),
        Some(("fav", if fav { "Retirer des favoris" } else { "Ajouter aux favoris" }, false)),
        None,
        Some(("close", "Fermer", false)),
        Some(("close_others", "Fermer les autres onglets", others(tabs, id).is_empty())),
        Some(("close_right", "Fermer les onglets à droite", right(tabs, id).is_empty())),
        None,
        Some(("reopen", "Rouvrir l'onglet fermé", !can_reopen)),
    ]
}

/// Menu de l'icône d'une extension (sa petite fenêtre ne peut pas s'afficher dans WebView2) ; Options seulement si
/// elle a une page d'options.
pub fn ext_items(on: bool, opts: bool) -> Vec<Item> {
    let mut m = vec![Some(("toggle", if on { "Désactiver" } else { "Activer" }, false))];
    if opts {
        m.push(Some(("opts", "Options", false)));
    }
    m.extend([None, Some(("manage", "Gérer les extensions", false)), None, Some(("remove", "Désinstaller", false))]);
    m
}

/// Affiche le menu au curseur et renvoie l'action choisie (None : menu fermé sans choix).
pub fn show(hwnd: isize, items: &[Item]) -> Option<&'static str> {
    unsafe {
        force_dark();
        let m = CreatePopupMenu().ok()?;
        for (n, it) in items.iter().enumerate() {
            let _ = match it {
                None => AppendMenuW(m, MF_SEPARATOR, 0, None),
                Some((_, l, g)) => AppendMenuW(m, if *g { MF_STRING | MF_GRAYED } else { MF_STRING }, n + 1, &HSTRING::from(*l)),
            };
        }
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        let h = HWND(hwnd as _);
        let _ = SetForegroundWindow(h); // sans ça, un clic ailleurs ne referme pas le menu
        let r = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, p.x, p.y, None, h, None);
        let _ = DestroyMenu(m);
        (*items.get((r.0 as usize).checked_sub(1)?)?).map(|i| i.0)
    }
}

/// Menu sombre même si Windows est en thème clair : tao ne fait qu'« autoriser » le sombre (suit le système).
/// Même entrée non documentée d'uxtheme (ordinal 135) que tao ; 2 = ForceDark.
unsafe fn force_dark() {
    unsafe {
        let Ok(ux) = LoadLibraryW(w!("uxtheme.dll")) else { return };
        if let Some(f) = GetProcAddress(ux, PCSTR(135 as _)) {
            let set: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(f);
            set(2);
        }
    }
}

/// Onglets fermés par « Fermer les autres onglets » : tous sauf `id` et les épinglés.
pub fn others(tabs: &[Tab], id: u32) -> Vec<u32> {
    tabs.iter().filter(|t| t.id != id && !t.pinned).map(|t| t.id).collect()
}

/// Onglets fermés par « Fermer les onglets à droite » : non épinglés après `id`.
pub fn right(tabs: &[Tab], id: u32) -> Vec<u32> {
    let i = tabs.iter().position(|t| t.id == id).map_or(tabs.len(), |i| i + 1);
    tabs[i..].iter().filter(|t| !t.pinned).map(|t| t.id).collect()
}

fn pinned(tabs: &[Tab]) -> usize {
    tabs.iter().filter(|t| t.pinned).count()
}

/// Place d'un nouvel onglet non épinglé « à droite » de l'onglet n° `i` : jamais au milieu des épinglés.
pub fn new_at(tabs: &[Tab], i: usize) -> usize {
    (i + 1).max(pinned(tabs))
}

/// Épingle / désépingle l'onglet n° `i` : il rejoint la fin des épinglés, ou le début des autres.
pub fn toggle_pin(tabs: &mut Vec<Tab>, i: usize) {
    let mut t = tabs.remove(i);
    t.pinned = !t.pinned;
    let at = pinned(tabs);
    tabs.insert(at, t);
}

/// Déplace le dernier onglet (celui qu'on vient d'ouvrir) en position `at`.
pub fn move_last(tabs: &mut Vec<Tab>, at: usize) {
    if let Some(t) = tabs.pop() {
        tabs.insert(at.min(tabs.len()), t);
    }
}

/// Mémorise un onglet fermé (position, adresse) pour « Rouvrir » ; pages web seulement.
pub fn remember(closed: &mut Vec<(usize, String)>, i: usize, url: &str) {
    if url.starts_with("http://") || url.starts_with("https://") {
        closed.push((i, url.into()));
        if closed.len() > CLOSED_MAX {
            closed.remove(0);
        }
    }
}

/// Place d'un onglet rouvert : son ancienne position si elle existe encore, après les épinglés.
pub fn reopen_at(tabs: &[Tab], i: usize) -> usize {
    i.clamp(pinned(tabs), tabs.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn tab(id: u32, pinned: bool) -> Tab {
        Tab { id, wv: None, title: String::new(), url: format!("https://{id}.fr"), loading: false, audio: false, muted: false, seen: Instant::now(), frozen: false, typed: false, saved: None, pinned, cmp: String::new(), foreign: false }
    }
    fn ids(t: &[Tab]) -> Vec<u32> {
        t.iter().map(|t| t.id).collect()
    }

    #[test]
    fn menu_entries() {
        let t = vec![tab(1, true), tab(2, false), tab(3, false)];
        let m = items(&t, 2, false, false);
        let labels: Vec<_> = m.iter().map(|i| i.map_or("—", |i| i.1)).collect();
        assert_eq!(labels, ["Nouvel onglet à droite", "—", "Actualiser", "Dupliquer", "Épingler", "Couper le son de l'onglet", "Ajouter aux favoris", "—", "Fermer", "Fermer les autres onglets", "Fermer les onglets à droite", "—", "Rouvrir l'onglet fermé"]);
        let grayed = |m: &[Item], a| m.iter().flatten().find(|i| i.0 == a).unwrap().2;
        assert!(!grayed(&m, "close_others") && !grayed(&m, "close_right") && grayed(&m, "reopen"));
        // dernier onglet, seul non épinglé, son coupé, favori, pile non vide
        let mut t2 = vec![tab(1, true), tab(2, false)];
        t2[1].muted = true;
        let m = items(&t2, 2, true, true);
        assert!(grayed(&m, "close_others") && grayed(&m, "close_right") && !grayed(&m, "reopen"));
        assert!(m.iter().flatten().any(|i| i.1 == "Réactiver le son de l'onglet") && m.iter().flatten().any(|i| i.1 == "Retirer des favoris"));
        assert!(items(&t2, 1, false, false).iter().flatten().any(|i| i.1 == "Retirer l'épingle"));
    }

    #[test]
    fn ext_menu_entries() {
        let labels = |m: Vec<Item>| m.iter().map(|i| i.map_or("—", |i| i.1)).collect::<Vec<_>>();
        assert_eq!(labels(ext_items(true, true)), ["Désactiver", "Options", "—", "Gérer les extensions", "—", "Désinstaller"]);
        assert_eq!(labels(ext_items(false, false)), ["Activer", "—", "Gérer les extensions", "—", "Désinstaller"]);
        assert!(ext_items(true, true).iter().flatten().all(|i| !i.2)); // rien de grisé
    }

    #[test]
    fn pin_order_and_closing() {
        let mut t = vec![tab(1, false), tab(2, false), tab(3, false), tab(4, false)];
        toggle_pin(&mut t, 2); // 3 épinglé -> en tête
        assert_eq!(ids(&t), [3, 1, 2, 4]);
        toggle_pin(&mut t, 3); // 4 épinglé -> après 3
        assert_eq!(ids(&t), [3, 4, 1, 2]);
        assert_eq!(others(&t, 1), [2]); // les épinglés survivent
        assert_eq!(right(&t, 3), [1, 2]); // à droite d'un épinglé : seulement les non épinglés
        assert!(right(&t, 2).is_empty());
        assert_eq!(new_at(&t, 0), 2); // pas entre deux épinglés
        assert_eq!(new_at(&t, 2), 3);
        toggle_pin(&mut t, 0); // 3 désépinglé -> début des non épinglés
        assert_eq!(ids(&t), [4, 3, 1, 2]);
        assert!(t[0].pinned && !t[1].pinned);
        t.push(tab(9, false));
        move_last(&mut t, 1);
        assert_eq!(ids(&t), [4, 9, 3, 1, 2]);
    }

    #[test]
    fn closed_stack() {
        let mut c = vec![];
        remember(&mut c, 0, "about:blank");
        remember(&mut c, 0, "data:text/html,x");
        assert!(c.is_empty());
        for i in 0..30 {
            remember(&mut c, i, &format!("https://{i}.fr"));
        }
        assert_eq!(c.len(), 25);
        assert_eq!(c[0].1, "https://5.fr"); // les plus anciens partent
        assert_eq!(c.pop(), Some((29, "https://29.fr".into())));
        let t = vec![tab(1, true), tab(2, false)];
        assert_eq!(reopen_at(&t, 1), 1); // ancienne place
        assert_eq!(reopen_at(&t, 7), 2); // plus assez d'onglets : à la fin
        assert_eq!(reopen_at(&t, 0), 1); // jamais avant les épinglés
    }
}
