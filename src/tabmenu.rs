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

/// Couleurs proposées pour un groupe d'onglets : (nom en français, traduit à l'affichage, couleur).
pub const GROUP_COLORS: [(&str, &str); 8] =
    [("Gris", "#9aa0a6"), ("Bleu", "#8ab4f8"), ("Rouge", "#f28b82"), ("Jaune", "#fdd663"), ("Vert", "#81c995"), ("Rose", "#ff8bcb"), ("Violet", "#c58af9"), ("Cyan", "#78d9ec")];

/// (action, libellé en français, traduit à l'affichage par `show`, grisé) ; None = séparateur.
pub type Item = Option<(&'static str, &'static str, bool)>;
const CLOSED_MAX: usize = 25;

/// Entrées du menu de l'onglet `id`, dans l'ordre de Chrome.
pub fn items(tabs: &[Tab], id: u32, fav: bool, can_reopen: bool) -> Vec<Item> {
    let (pinned, muted, grouped) = tabs.iter().find(|t| t.id == id).map_or((false, false, false), |t| (t.pinned, t.muted, t.group != 0));
    let mut m = vec![Some(("new_right", "Nouvel onglet à droite", false)), Some(("group_new", "Ajouter à un nouveau groupe", pinned))];
    if grouped {
        m.push(Some(("group_out", "Retirer du groupe", false)));
    }
    m.extend([
        None,
        Some(("reload", "Actualiser", false)),
        Some(("dup", "Dupliquer", false)),
        Some(("to_window", "Déplacer dans une nouvelle fenêtre", tabs.len() < 2)),
        Some(("pin", if pinned { "Retirer l'épingle" } else { "Épingler" }, false)),
        Some(("mute", if muted { "Réactiver le son de l'onglet" } else { "Couper le son de l'onglet" }, false)),
        Some(("fav", if fav { "Retirer des favoris" } else { "Ajouter aux favoris" }, false)),
        None,
        Some(("close", "Fermer", false)),
        Some(("close_others", "Fermer les autres onglets", others(tabs, id).is_empty())),
        Some(("close_right", "Fermer les onglets à droite", right(tabs, id).is_empty())),
        None,
        Some(("reopen", "Rouvrir l'onglet fermé", !can_reopen)),
        None,
        Some(("new_window", "Nouvelle fenêtre", false)),
        Some(("new_private", "Nouvelle fenêtre de navigation privée", false)),
    ]);
    m
}

/// Menu d'une pastille de groupe : renommer, les couleurs (la sienne cochée), dissocier, fermer. Renvoie les lignes et,
/// pour chacune, son action (« color:N » pour la couleur n° N).
pub fn group_rows(color: &str) -> (Vec<Option<(String, bool)>>, Vec<String>) {
    let t = crate::lang::t;
    let mut rows = vec![Some((t("Renommer le groupe"), false)), None];
    let mut acts = vec!["rename".to_string(), String::new()];
    for (n, (name, c)) in GROUP_COLORS.iter().enumerate() {
        rows.push(Some((format!("{}{}", if *c == color { "✓ " } else { "    " }, t(name)), false)));
        acts.push(format!("color:{n}"));
    }
    rows.extend([None, Some((t("Dissocier le groupe"), false)), Some((t("Fermer le groupe"), false))]);
    acts.extend([String::new(), "ungroup".into(), "close".into()]);
    (rows, acts)
}

/// Choix fait dans un menu aux lignes ajoutées : une action du menu, ou le rang de la ligne ajoutée.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Picked {
    Action(&'static str),
    Extra(usize),
}

/// Lignes de `items` (traduites) avec les lignes `extra` (libellés déjà prêts) glissées après l'entrée `after`, et ce que
/// vaut chaque ligne.
fn rows_with(items: &[Item], after: &str, extra: &[String]) -> (Vec<Option<(String, bool)>>, Vec<Option<Picked>>) {
    let (mut rows, mut map) = (vec![], vec![]);
    for it in items {
        rows.push(it.map(|(_, l, g)| (crate::lang::t(l), g)));
        map.push(it.map(|i| Picked::Action(i.0)));
        if it.is_some_and(|i| i.0 == after) {
            for (n, l) in extra.iter().enumerate() {
                rows.push(Some((l.clone(), false)));
                map.push(Some(Picked::Extra(n)));
            }
        }
    }
    (rows, map)
}

/// Comme `show`, avec des lignes en plus après l'entrée `after` (les groupes que l'onglet peut rejoindre).
pub fn show_with(hwnd: isize, items: &[Item], after: &str, extra: &[String]) -> Option<Picked> {
    let (rows, map) = rows_with(items, after, extra);
    *map.get(popup(hwnd, &rows, None)?)?
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
    let rows: Vec<_> = items.iter().map(|it| it.map(|(_, l, g)| (crate::lang::t(l), g))).collect();
    (*items.get(popup(hwnd, &rows, None)?)?).map(|i| i.0)
}

/// Menu natif aux libellés libres : (texte, grisé), None = séparateur. `at` : coin haut gauche à l'écran, None : au
/// curseur. Renvoie le rang de la ligne choisie.
pub fn popup(hwnd: isize, rows: &[Option<(String, bool)>], at: Option<(i32, i32)>) -> Option<usize> {
    unsafe {
        force_dark();
        let m = CreatePopupMenu().ok()?;
        for (n, it) in rows.iter().enumerate() {
            let _ = match it {
                None => AppendMenuW(m, MF_SEPARATOR, 0, None),
                Some((l, g)) => AppendMenuW(m, if *g { MF_STRING | MF_GRAYED } else { MF_STRING }, n + 1, &HSTRING::from(l.as_str())),
            };
        }
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        let (x, y) = at.unwrap_or((p.x, p.y));
        let h = HWND(hwnd as _);
        let _ = SetForegroundWindow(h); // sans ça, un clic ailleurs ne referme pas le menu
        let r = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, x, y, None, h, None);
        let _ = DestroyMenu(m);
        (r.0 as usize).checked_sub(1)
    }
}

/// Ligne d'un favori dans un menu natif : son titre (son adresse s'il n'en a pas), coupé à 60 caractères, « & » doublé
/// (Windows en ferait un raccourci clavier souligné).
pub fn label(title: &str, url: &str) -> String {
    let t = if title.trim().is_empty() { url } else { title.trim() };
    let mut s: String = t.chars().take(60).collect();
    if t.chars().count() > 60 {
        s.push('…');
    }
    s.replace('&', "&&")
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
    if t.pinned {
        t.group = 0; // un onglet épinglé n'est dans aucun groupe
    }
    let at = pinned(tabs);
    tabs.insert(at, t);
}

/// L'onglet n° `i` passe dans le groupe `g` (0 : aucun) sans couper un groupe en deux : il se place après le dernier
/// onglet de `g`, sinon après le dernier de son ancien groupe, sinon il ne bouge pas. Renvoie sa nouvelle position.
pub fn set_group(tabs: &mut Vec<Tab>, i: usize, g: u32) -> usize {
    let mut t = tabs.remove(i);
    let old = std::mem::replace(&mut t.group, g);
    let last = |g: u32| tabs.iter().rposition(|x| g != 0 && x.group == g);
    let at = last(g).or_else(|| last(old)).map_or(i, |p| p + 1);
    tabs.insert(at, t);
    at
}

/// Onglet déposé à la souris : il prend le groupe `g` de l'endroit où il arrive (0 : aucun). Seul dans son groupe, il le
/// garde : le groupe se déplace avec lui.
pub fn regroup(tabs: &mut [Tab], id: u32, g: u32) {
    let Some(i) = tabs.iter().position(|t| t.id == id) else { return };
    let own = tabs[i].group;
    if tabs[i].pinned {
        tabs[i].group = 0;
    } else if own == 0 || tabs.iter().any(|t| t.id != id && t.group == own) {
        tabs[i].group = g;
    }
}

/// Onglet arrivé en position `i` (rouvert) entre deux onglets d'un même groupe : il le rejoint.
pub fn adopt(tabs: &mut [Tab], i: usize) {
    if i > 0 && i + 1 < tabs.len() && tabs[i - 1].group != 0 && tabs[i - 1].group == tabs[i + 1].group && !tabs[i].pinned {
        tabs[i].group = tabs[i - 1].group;
    }
}

/// Déplace le dernier onglet (celui qu'on vient d'ouvrir) en position `at`.
pub fn move_last(tabs: &mut Vec<Tab>, at: usize) {
    if let Some(t) = tabs.pop() {
        tabs.insert(at.min(tabs.len()), t);
    }
}

/// Glisser-déposer : l'onglet n° `i` va en position `to`, sans sortir de son groupe (épinglés en tête, autres après).
pub fn move_to(tabs: &mut Vec<Tab>, i: usize, to: usize) {
    let p = pinned(tabs);
    let (lo, hi) = if tabs[i].pinned { (0, p - 1) } else { (p, tabs.len() - 1) };
    let t = tabs.remove(i);
    tabs.insert(to.clamp(lo, hi), t);
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
        Tab { id, wv: None, title: String::new(), url: format!("https://{id}.fr"), loading: false, audio: false, muted: false, seen: Instant::now(), frozen: false, typed: false, saved: None, pinned, cmp: String::new(), foreign: false, icon: String::new(), blocked: vec![], zoom: 1.0, bg: None, group: 0 }
    }
    fn ids(t: &[Tab]) -> Vec<u32> {
        t.iter().map(|t| t.id).collect()
    }

    #[test]
    fn menu_entries() {
        let t = vec![tab(1, true), tab(2, false), tab(3, false)];
        let m = items(&t, 2, false, false);
        let labels: Vec<_> = m.iter().map(|i| i.map_or("—", |i| i.1)).collect();
        assert_eq!(labels, ["Nouvel onglet à droite", "Ajouter à un nouveau groupe", "—", "Actualiser", "Dupliquer", "Déplacer dans une nouvelle fenêtre", "Épingler", "Couper le son de l'onglet", "Ajouter aux favoris", "—", "Fermer", "Fermer les autres onglets", "Fermer les onglets à droite", "—", "Rouvrir l'onglet fermé", "—", "Nouvelle fenêtre", "Nouvelle fenêtre de navigation privée"]);
        let grayed = |m: &[Item], a| m.iter().flatten().find(|i| i.0 == a).unwrap().2;
        assert!(!grayed(&m, "close_others") && !grayed(&m, "close_right") && grayed(&m, "reopen"));
        // dernier onglet, seul non épinglé, son coupé, favori, pile non vide
        let mut t2 = vec![tab(1, true), tab(2, false)];
        t2[1].muted = true;
        let m = items(&t2, 2, true, true);
        assert!(grayed(&m, "close_others") && grayed(&m, "close_right") && !grayed(&m, "reopen"));
        assert!(m.iter().flatten().any(|i| i.1 == "Réactiver le son de l'onglet") && m.iter().flatten().any(|i| i.1 == "Retirer des favoris"));
        assert!(items(&t2, 1, false, false).iter().flatten().any(|i| i.1 == "Retirer l'épingle"));
        // libellés écrits en français ici et traduits à l'affichage (show) : chacun a sa traduction
        // onglet groupé : « Retirer du groupe » en plus ; épinglé : pas de groupe possible
        let mut t3 = vec![tab(1, true), tab(2, false)];
        t3[1].group = 4;
        assert!(items(&t3, 2, false, false).iter().flatten().any(|i| i.0 == "group_out") && !m.iter().flatten().any(|i| i.0 == "group_out"));
        assert!(grayed(&items(&t3, 1, false, false), "group_new") && !grayed(&items(&t3, 2, false, false), "group_new"));
        for (name, _) in GROUP_COLORS {
            assert!(crate::lang::missing(name).is_empty(), "« {name} » sans traduction");
        }
        for m in [items(&t, 2, false, false), items(&t2, 2, true, true), items(&t2, 1, false, false), items(&t3, 2, false, false), ext_items(true, true), ext_items(false, false)] {
            for i in m.iter().flatten() {
                assert!(crate::lang::missing(i.1).is_empty(), "« {} » sans traduction", i.1);
            }
        }
    }

    #[test]
    fn tab_groups() {
        let groups = |t: &[Tab]| t.iter().map(|t| t.group).collect::<Vec<_>>();
        let mut t: Vec<Tab> = (1..=6).map(|i| tab(i, false)).collect();
        // nouveau groupe : l'onglet ne bouge pas ; un second le rejoint et vient se ranger à côté
        assert_eq!(set_group(&mut t, 1, 7), 1);
        assert_eq!(set_group(&mut t, 4, 7), 2);
        assert_eq!((ids(&t), groups(&t)), (vec![1, 2, 5, 3, 4, 6], vec![0, 7, 7, 0, 0, 0]));
        set_group(&mut t, 0, 7); // un onglet placé avant le groupe arrive à sa suite
        assert_eq!((ids(&t), groups(&t)), (vec![2, 5, 1, 3, 4, 6], vec![7, 7, 7, 0, 0, 0]));
        // celui du milieu change de groupe ou le quitte : il sort par la droite, le groupe reste d'un seul tenant
        set_group(&mut t, 1, 8);
        assert_eq!((ids(&t), groups(&t)), (vec![2, 1, 5, 3, 4, 6], vec![7, 7, 8, 0, 0, 0]));
        set_group(&mut t, 0, 0);
        assert_eq!((ids(&t), groups(&t)), (vec![1, 2, 5, 3, 4, 6], vec![7, 0, 8, 0, 0, 0]));
        // dépôt à la souris : le groupe de l'endroit d'arrivée ; seul dans son groupe, l'onglet le garde
        regroup(&mut t, 2, 8);
        regroup(&mut t, 1, 0);
        regroup(&mut t, 99, 8); // onglet inconnu : rien
        assert_eq!(groups(&t), vec![7, 8, 8, 0, 0, 0]);
        regroup(&mut t, 5, 0); // deux dans le groupe 8 : celui-ci peut en sortir
        assert_eq!(groups(&t), vec![7, 8, 0, 0, 0, 0]);
        // épingler retire du groupe ; un onglet rouvert entre deux onglets d'un groupe le rejoint
        toggle_pin(&mut t, 1);
        assert_eq!((ids(&t), groups(&t)), (vec![2, 1, 5, 3, 4, 6], vec![0, 7, 0, 0, 0, 0]));
        (t[3].group, t[5].group) = (9, 9);
        adopt(&mut t, 4);
        adopt(&mut t, 2); // voisins sans groupe commun : rien
        adopt(&mut t, 0);
        adopt(&mut t, 5);
        assert_eq!(groups(&t), vec![0, 7, 0, 9, 9, 9]);
    }

    #[test]
    fn menu_with_group_rows() {
        let t = vec![tab(1, false), tab(2, false)];
        let (rows, map) = rows_with(&items(&t, 1, false, false), "group_new", &["Vers A".into(), "Vers B".into()]);
        assert_eq!(rows.len(), map.len());
        assert_eq!(map[..5], [Some(Picked::Action("new_right")), Some(Picked::Action("group_new")), Some(Picked::Extra(0)), Some(Picked::Extra(1)), None]);
        assert_eq!(rows[3], Some(("Vers B".to_string(), false)));
        let (rows, acts) = group_rows("#f28b82");
        assert_eq!(rows.len(), acts.len());
        assert_eq!((acts[0].as_str(), acts[4].as_str(), acts[acts.len() - 1].as_str()), ("rename", "color:2", "close"));
        assert!(rows[4].as_ref().unwrap().0.starts_with('✓') && !rows[3].as_ref().unwrap().0.starts_with('✓') && rows[1].is_none());
    }

    #[test]
    fn drag_stays_in_its_group() {
        let mut t = vec![tab(1, true), tab(2, true), tab(3, false), tab(4, false), tab(5, false)];
        move_to(&mut t, 4, 2); // dernier -> premier des non épinglés
        assert_eq!(ids(&t), [1, 2, 5, 3, 4]);
        move_to(&mut t, 2, 99); // au-delà de la fin -> dernier
        assert_eq!(ids(&t), [1, 2, 3, 4, 5]);
        move_to(&mut t, 3, 0); // un non épinglé ne passe pas devant les épinglés
        assert_eq!(ids(&t), [1, 2, 4, 3, 5]);
        move_to(&mut t, 0, 4); // un épinglé ne sort pas des épinglés
        assert_eq!(ids(&t), [2, 1, 4, 3, 5]);
    }

    #[test]
    fn favorite_labels() {
        assert_eq!(label("AT&T", "https://att.com"), "AT&&T");
        assert_eq!(label("  ", "https://a.fr/x"), "https://a.fr/x");
        let long = label(&"é".repeat(80), "");
        assert_eq!((long.chars().count(), long.ends_with('…')), (61, true));
        assert_eq!(label(&"a".repeat(60), ""), "a".repeat(60)); // pile 60 : pas de points de suspension
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
