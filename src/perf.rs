// Réglages moteur adaptés à la machine + mise en veille des onglets.
use webview2_com::{Microsoft::Web::WebView2::Win32::ICoreWebView2_3, TrySuspendCompletedHandler};
use windows::{
    Win32::{
        Foundation::{CloseHandle, FILETIME},
        System::{
            Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS},
            SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
            Threading::{
                BELOW_NORMAL_PRIORITY_CLASS, GetSystemTimes, NORMAL_PRIORITY_CLASS, OpenProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
                PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE, PROCESS_SET_INFORMATION,
                ProcessPowerThrottling, SetPriorityClass, SetProcessInformation,
            },
        },
    },
    core::Interface,
};
use wry::{WebView, WebViewExtWindows};

pub fn machine() -> (usize, u64) {
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut m = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    let ram_gb = if unsafe { GlobalMemoryStatusEx(&mut m) }.is_ok() { m.ullTotalPhys >> 30 } else { 8 };
    (cores, ram_gb)
}

/// Arguments Chromium : identiques pour tous les webviews (même environnement).
pub fn browser_args(cores: usize, ram_gb: u64, night: bool, game: bool, extra: &str) -> String {
    let mut a = vec![
        // valeurs par défaut de wry, à conserver
        "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection".to_string(),
        // rendu des pages sur la carte graphique, découpé en tuiles traitées en parallèle
        "--enable-gpu-rasterization".into(),
        "--enable-zero-copy".into(),
        format!("--num-raster-threads={}", (cores / 2).clamp(1, 4)),
        // onglets d'un même site dans un seul processus (tas V8 et caches communs) :
        // mesuré -14 % de RAM commitée et -13 processus sur 50 onglets / 15 sites.
        // Revers : un plantage du processus emporte tous les onglets de ce site.
        "--process-per-site".into(),
    ];
    let mut features = vec!["ParallelDownloading"];
    if night {
        features.push("WebContentsForceDark");
    }
    // extra (VOLT_ARGS, tests A/B) : ses --enable-features rejoignent la liste, sinon l'un écraserait l'autre.
    for x in extra.split_whitespace() {
        match x.strip_prefix("--enable-features=") {
            Some(f) => features.push(f),
            None => a.push(x.into()),
        }
    }
    a.push(format!("--enable-features={}", features.join(",")));
    // Moins de processus de pages = moins de RAM (chaque processus coûte ~30-80 Mo fixes).
    // Petites machines : toujours ; Mode Jeu : plafond selon la RAM ; sinon Chromium décide.
    let limit = if ram_gb <= 8 {
        Some((ram_gb as usize).max(2))
    } else if game {
        Some((ram_gb as usize / 2).clamp(4, 12))
    } else {
        None
    };
    if let Some(l) = limit {
        a.push(format!("--renderer-process-limit={l}"));
    }
    a.join(" ")
}

/// Arbre de processus (racine incluse), via un instantané Toolhelp.
fn process_tree(root: u32) -> Vec<u32> {
    let mut pairs = vec![]; // (pid, parent)
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return vec![root] };
        let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            pairs.push((e.th32ProcessID, e.th32ParentProcessID));
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    let mut tree = vec![root];
    let mut i = 0;
    while i < tree.len() {
        let p = tree[i];
        tree.extend(pairs.iter().filter(|&&(c, par)| par == p && c != p && !tree.contains(&c)).map(|&(c, _)| c).collect::<Vec<_>>());
        i += 1;
    }
    tree
}

/// Fenêtre réduite (on joue) : Volt et tout le moteur passent en priorité basse + EcoQoS,
/// pour que le planificateur Windows serve le jeu d'abord. `false` rétablit la normale.
pub fn background(wv: &WebView, on: bool) {
    let mut browser = 0u32;
    let _ = unsafe { wv.webview().BrowserProcessId(&mut browser) };
    let mut pids = process_tree(browser);
    pids.push(std::process::id());
    let class = if on { BELOW_NORMAL_PRIORITY_CLASS } else { NORMAL_PRIORITY_CLASS };
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: if on { PROCESS_POWER_THROTTLING_EXECUTION_SPEED } else { 0 },
    };
    for pid in pids {
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_SET_INFORMATION, false, pid) {
                let _ = SetPriorityClass(h, class);
                let _ = SetProcessInformation(h, ProcessPowerThrottling, &state as *const _ as _, size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32);
                let _ = CloseHandle(h);
            }
        }
    }
}

/// Part du processeur occupée entre deux relevés (temps inactif / temps total, en %).
fn busy_pct(idle: u64, total: u64) -> u8 {
    if total == 0 { 0 } else { (100 - (idle.min(total) * 100 / total)) as u8 }
}

/// Charge processeur de la machine, relevée toutes les 2 s dans un fil à part ;
/// `send` n'est appelé que si la valeur bouge d'au moins 5 points (anime la mascotte).
pub fn cpu_watch(send: impl Fn(u8) + Send + 'static) {
    std::thread::spawn(move || {
        let ft = |f: FILETIME| (f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64;
        let read = || {
            let (mut i, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
            unsafe { GetSystemTimes(Some(&mut i), Some(&mut k), Some(&mut u)) }.ok().map(|_| (ft(i), ft(k) + ft(u))) // noyau inclut l'inactif
        };
        let (mut prev, mut last) = (read(), 255u8);
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let now = read();
            if let (Some((i0, t0)), Some((i1, t1))) = (prev, now) {
                let p = busy_pct(i1 - i0, t1 - t0);
                if p.abs_diff(last) >= 5 {
                    last = p;
                    send(p);
                }
            }
            prev = now;
        }
    });
}

/// Gèle un onglet caché (processus en pause, mémoire récupérable par Windows).
pub fn suspend(wv: &WebView) {
    if let Ok(w) = wv.webview().cast::<ICoreWebView2_3>() {
        let h = TrySuspendCompletedHandler::create(Box::new(|_, _| Ok(())));
        let _ = unsafe { w.TrySuspend(&h) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_adapt() {
        let small = browser_args(2, 4, true, false, "");
        assert!(small.contains("--num-raster-threads=1") && small.contains("--renderer-process-limit=4"));
        assert!(small.contains("WebContentsForceDark") && small.contains("msSmartScreenProtection") && small.contains("--process-per-site"));
        let big = browser_args(16, 32, false, false, "");
        assert!(big.contains("--num-raster-threads=4") && !big.contains("renderer-process-limit"));
        assert!(browser_args(16, 32, false, true, "").contains("--renderer-process-limit=12"));
        assert!(browser_args(8, 16, false, true, "").contains("--renderer-process-limit=8"));
        assert!(!big.contains("ForceDark"));
        // une seule liste de features, fusionnée avec celle de VOLT_ARGS
        let x = browser_args(16, 32, true, false, "--process-per-site --enable-features=Foo:Bar/20");
        assert!(x.contains("--enable-features=ParallelDownloading,WebContentsForceDark,Foo:Bar/20") && x.contains(" --process-per-site"));
        assert_eq!(x.matches("--enable-features").count(), 1);
        let (c, r) = machine();
        assert!(c >= 1 && r >= 1);
    }

    #[test]
    fn cpu_percent() {
        assert_eq!(busy_pct(0, 0), 0);
        assert_eq!(busy_pct(100, 100), 0);
        assert_eq!(busy_pct(25, 100), 75);
        assert_eq!(busy_pct(0, 100), 100);
        assert_eq!(busy_pct(200, 100), 0); // relevés incohérents : pas de débordement
    }

    #[test]
    fn tree_contains_root_and_children() {
        let me = std::process::id();
        let mut child = std::process::Command::new("cmd").args(["/C", "ping -n 3 127.0.0.1 >nul"]).spawn().unwrap();
        let t = process_tree(me);
        assert_eq!(t[0], me);
        assert!(t.contains(&child.id()));
        let _ = child.kill();
    }
}
