// Réglages moteur adaptés à la machine + mise en veille des onglets.
use webview2_com::{
    GetProcessExtendedInfosCompletedHandler,
    Microsoft::Web::WebView2::Win32::{COREWEBVIEW2_PROCESS_KIND, ICoreWebView2_3, ICoreWebView2Environment13, ICoreWebView2FrameInfo2},
    TrySuspendCompletedHandler,
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, FILETIME},
        System::{
            Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS},
            Performance::{
                PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData,
                PdhGetFormattedCounterArrayW, PdhOpenQueryW,
            },
            ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2},
            SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
            Threading::{
                BELOW_NORMAL_PRIORITY_CLASS, GetProcessTimes, GetSystemTimes, NORMAL_PRIORITY_CLASS, OpenProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
                PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION,
                ProcessPowerThrottling, SetPriorityClass, SetProcessInformation,
            },
        },
    },
    core::{BOOL, Interface, w},
};
use wry::{WebView, WebViewExtWindows};

pub fn machine() -> (usize, u64) {
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut m = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    let ram_gb = if unsafe { GlobalMemoryStatusEx(&mut m) }.is_ok() { m.ullTotalPhys >> 30 } else { 8 };
    (cores, ram_gb)
}

/// Arguments Chromium : identiques pour tous les webviews (même environnement).
pub fn browser_args(cores: usize, ram_gb: u64, night: bool, game: bool, safe: bool, lang: Option<&str>, extra: &str) -> String {
    let mut a = vec![
        // valeurs par défaut de wry, à conserver ; la dernière coupe l'alerte sur les sites dangereux (SmartScreen), que
        // wry désactive d'office : on ne la coupe que si l'utilisateur l'a demandé
        format!("--disable-features=msWebOOUI,msPdfOOUI{}", if safe { "" } else { ",msSmartScreenProtection" }),
        // rendu des pages sur la carte graphique, découpé en tuiles traitées en parallèle
        "--enable-gpu-rasterization".into(),
        "--enable-zero-copy".into(),
        format!("--num-raster-threads={}", (cores / 2).clamp(1, 4)),
        // onglets d'un même site dans un seul processus (tas V8 et caches communs) :
        // mesuré -14 % de RAM commitée et -13 processus sur 50 onglets / 15 sites.
        // Revers : un plantage du processus emporte tous les onglets de ce site.
        "--process-per-site".into(),
    ];
    // langue choisie qui n'est pas celle de Windows (lang::engine) : sans cet argument le moteur suit Windows. Mesuré :
    // il règle menus des pages, pages d'erreur, navigator.language et Accept-Language, et passe avant l'option de wry.
    if let Some(l) = lang {
        a.push(format!("--lang={l}"));
    }
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

/// Consommation de Volt tout entier (volt.exe, le moteur et tous ses onglets), pour la bulle de Volty.
/// `cpu`, `gpu` et `vram_mb` manquent au premier relevé (ce sont des écarts entre deux relevés) ou si Windows ne les donne pas.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Usage {
    pub ram_mb: u64,          // mémoire propre à Volt (celle du Gestionnaire des tâches), en Mo
    pub cpu: Option<f32>,     // % de toute la machine, tous cœurs confondus
    pub gpu: Option<f32>,     // % de la carte graphique
    pub vram_mb: Option<u64>, // mémoire vidéo dédiée, en Mo
}

// Relevé précédent : gardé d'un appel à l'autre, sur le fil de l'interface seulement. Rien ne tourne entre deux appels.
struct Meter {
    at: std::time::Instant,
    cpu: u64, // temps processeur cumulé de l'arbre, en pas de 100 ns
    gpu: Option<(PDH_HQUERY, PDH_HCOUNTER, PDH_HCOUNTER)>,
}
thread_local!(static METER: std::cell::RefCell<Option<Meter>> = const { std::cell::RefCell::new(None) });

/// À appeler sur le fil de l'interface, environ une fois par seconde tant que la bulle est affichée.
pub fn usage(wv: &WebView, cores: usize) -> Usage {
    let mut browser = 0u32;
    let _ = unsafe { wv.webview().BrowserProcessId(&mut browser) };
    let mut pids = process_tree(browser);
    pids.push(std::process::id());
    let (ram, cpu) = sample(&pids);
    let mut u = Usage { ram_mb: ram >> 20, ..Default::default() };
    METER.with_borrow_mut(|m| {
        let now = std::time::Instant::now();
        match m {
            // premier appel : on ouvre les compteurs de carte graphique, leurs valeurs viendront au relevé suivant
            None => *m = Some(Meter { at: now, cpu, gpu: unsafe { gpu_open() } }),
            Some(prev) => {
                let ticks = now.duration_since(prev.at).as_nanos() as f32 / 100.0 * cores.max(1) as f32;
                if ticks > 0.0 {
                    u.cpu = Some((cpu.saturating_sub(prev.cpu) as f32 * 100.0 / ticks).min(100.0));
                }
                if let Some((q, eng, mem)) = prev.gpu
                    && unsafe { PdhCollectQueryData(q) } == 0
                {
                    u.gpu = Some(gpu_pct(&unsafe { gpu_values(eng) }, &pids));
                    u.vram_mb = Some((unsafe { gpu_values(mem) }.iter().filter(|(n, _)| gpu_instance(n).is_some_and(|(p, _)| pids.contains(&p))).map(|x| x.1).sum::<f64>() as u64) >> 20);
                }
                (prev.at, prev.cpu) = (now, cpu);
            }
        }
    });
    u
}

/// Survol terminé : le prochain survol repart d'un relevé neuf (sinon son premier chiffre serait une moyenne depuis le dernier).
pub fn usage_stop() {
    METER.with_borrow_mut(|m| {
        if let Some((q, ..)) = m.take().and_then(|m| m.gpu) {
            let _ = unsafe { PdhCloseQuery(q) };
        }
    });
}

/// (mémoire privée en octets, temps processeur cumulé en pas de 100 ns) de ces processus ; un processus disparu compte 0.
fn sample(pids: &[u32]) -> (u64, u64) {
    let ft = |f: FILETIME| (f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64;
    let (mut ram, mut cpu) = (0u64, 0u64);
    for &pid in pids {
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { continue };
            let mut m = PROCESS_MEMORY_COUNTERS_EX2 { cb: size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32, ..Default::default() };
            if K32GetProcessMemoryInfo(h, &mut m as *mut _ as *mut PROCESS_MEMORY_COUNTERS, m.cb).as_bool() {
                ram += m.PrivateWorkingSetSize as u64;
            }
            let (mut c, mut e, mut k, mut us) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
            if GetProcessTimes(h, &mut c, &mut e, &mut k, &mut us).is_ok() {
                cpu += ft(k) + ft(us);
            }
            let _ = CloseHandle(h);
        }
    }
    (ram, cpu)
}

/// Un processus de Volt, pour le gestionnaire de tâches.
#[derive(Debug, PartialEq)]
pub struct Task {
    pub pid: u32,
    pub kind: i32,        // nature donnée par le moteur : 0 navigateur, 1 pages, 2 et 3 services, 4 carte graphique ; -1 : volt.exe
    pub frames: Vec<u32>, // pages servies : numéro, dans le moteur, de leur cadre principal
    pub ram_mb: u64,
    pub cpu: Option<f32>, // % de toute la machine depuis le relevé précédent ; None au premier
}

// Relevé précédent du gestionnaire de tâches : (instant, temps processeur cumulé de chaque processus).
type Before = Option<(std::time::Instant, std::collections::HashMap<u32, u64>)>;
thread_local!(static TASKS: std::cell::RefCell<Before> = const { std::cell::RefCell::new(None) });

/// Relevé de chaque processus du moteur, et de volt.exe : la liste vient du moteur, `done` la reçoit mesurée.
pub fn tasks(wv: &WebView, cores: usize, done: impl Fn(Vec<Task>) + 'static) {
    let Ok(env) = wv.environment().cast::<ICoreWebView2Environment13>() else { return done(vec![]) };
    let h = GetProcessExtendedInfosCompletedHandler::create(Box::new(move |_, list| {
        let mut found = vec![(std::process::id(), -1, vec![])];
        if let Some(l) = list {
            let mut n = 0u32;
            unsafe { l.Count(&mut n)? };
            for i in 0..n {
                unsafe {
                    let x = l.GetValueAtIndex(i)?;
                    let (mut pid, mut kind) = (0i32, COREWEBVIEW2_PROCESS_KIND::default());
                    let p = x.ProcessInfo()?;
                    p.ProcessId(&mut pid)?;
                    p.Kind(&mut kind)?;
                    let mut frames = vec![];
                    // la liste reste en vie pendant le parcours : son itérateur ne la retient pas (mesuré : arrêt brutal sinon)
                    if let Ok(all) = x.AssociatedFrameInfos()
                        && let Ok(it) = all.GetIterator()
                    {
                        let mut has = BOOL(0);
                        it.HasCurrent(&mut has)?;
                        while has.as_bool() {
                            // on remonte au cadre principal : un sous-cadre (publicité, lecteur vidéo) compte pour la page qui le porte
                            let mut f = it.GetCurrent()?.cast::<ICoreWebView2FrameInfo2>()?;
                            while let Ok(up) = f.ParentFrameInfo().and_then(|p| p.cast::<ICoreWebView2FrameInfo2>()) {
                                f = up;
                            }
                            let mut id = 0u32;
                            f.FrameId(&mut id)?;
                            if !frames.contains(&id) {
                                frames.push(id);
                            }
                            it.MoveNext(&mut has)?;
                        }
                    }
                    found.push((pid as u32, kind.0, frames));
                }
            }
        }
        done(TASKS.with_borrow_mut(|b| measure(found, cores, b, std::time::Instant::now(), sample)));
        Ok(())
    }));
    let _ = unsafe { env.GetProcessExtendedInfos(&h) };
}

/// Mémoire et part de processeur de chaque processus, par écart avec le relevé précédent `before` (ignoré s'il a plus
/// de 10 s : la page a été quittée entre-temps, la moyenne ne dirait rien).
fn measure(found: Vec<(u32, i32, Vec<u32>)>, cores: usize, before: &mut Before, now: std::time::Instant, sample: impl Fn(&[u32]) -> (u64, u64)) -> Vec<Task> {
    let old = before.take().filter(|b| now.duration_since(b.0).as_secs() < 10);
    let ticks = old.as_ref().map_or(0.0, |b| now.duration_since(b.0).as_nanos() as f32 / 100.0 * cores.max(1) as f32);
    let mut seen = std::collections::HashMap::new();
    let out = found
        .into_iter()
        .map(|(pid, kind, frames)| {
            let (ram, cpu) = sample(&[pid]);
            seen.insert(pid, cpu);
            let was = old.as_ref().and_then(|b| b.1.get(&pid)).filter(|_| ticks > 0.0);
            Task { pid, kind, frames, ram_mb: ram >> 20, cpu: was.map(|w| (cpu.saturating_sub(*w) as f32 * 100.0 / ticks).min(100.0)) }
        })
        .collect();
    *before = Some((now, seen));
    out
}

/// Compteurs Windows de la carte graphique par processus (ceux du Gestionnaire des tâches) ; None s'ils n'existent pas.
unsafe fn gpu_open() -> Option<(PDH_HQUERY, PDH_HCOUNTER, PDH_HCOUNTER)> {
    unsafe {
        let (mut q, mut eng, mut mem) = (PDH_HQUERY::default(), PDH_HCOUNTER::default(), PDH_HCOUNTER::default());
        if PdhOpenQueryW(None, 0, &mut q) != 0 {
            return None;
        }
        if PdhAddEnglishCounterW(q, w!(r"\GPU Engine(*)\Utilization Percentage"), 0, &mut eng) != 0
            || PdhAddEnglishCounterW(q, w!(r"\GPU Process Memory(*)\Dedicated Usage"), 0, &mut mem) != 0
        {
            let _ = PdhCloseQuery(q);
            return None;
        }
        let _ = PdhCollectQueryData(q); // l'utilisation est un écart : ce relevé sert de point de départ
        Some((q, eng, mem))
    }
}

/// Valeurs d'un compteur à instances multiples : (nom de l'instance, valeur).
unsafe fn gpu_values(c: PDH_HCOUNTER) -> Vec<(String, f64)> {
    unsafe {
        let (mut size, mut n) = (0u32, 0u32);
        let _ = PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, &mut size, &mut n, None); // 1er appel : la taille seulement
        let mut buf = vec![0u64; size as usize / 8 + 1]; // u64 : alignement des éléments
        if size == 0 || PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, &mut size, &mut n, Some(buf.as_mut_ptr().cast())) != 0 {
            return vec![];
        }
        let items = std::slice::from_raw_parts(buf.as_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>(), n as usize);
        items.iter().map(|i| (i.szName.to_string().unwrap_or_default(), i.FmtValue.Anonymous.doubleValue)).collect()
    }
}

/// (processus, type de moteur) d'un nom d'instance « pid_1234_luid_0x…_phys_0_eng_0_engtype_3D » ("" sans type).
fn gpu_instance(name: &str) -> Option<(u32, &str)> {
    let rest = name.strip_prefix("pid_")?;
    let pid = rest.split('_').next()?.parse().ok()?;
    Some((pid, name.rsplit_once("engtype_").map_or("", |x| x.1)))
}

/// Utilisation de la carte par ces processus : somme par type de moteur (3D, décodage vidéo…), puis le plus chargé,
/// comme le Gestionnaire des tâches.
fn gpu_pct(items: &[(String, f64)], pids: &[u32]) -> f32 {
    let mut by: Vec<(&str, f64)> = vec![];
    for (n, v) in items {
        let Some((pid, kind)) = gpu_instance(n) else { continue };
        if pids.contains(&pid) {
            match by.iter_mut().find(|b| b.0 == kind) {
                Some(b) => b.1 += v,
                None => by.push((kind, *v)),
            }
        }
    }
    by.iter().map(|b| b.1).fold(0.0, f64::max).min(100.0) as f32
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
        let small = browser_args(2, 4, true, false, false, None, "");
        assert!(small.contains("--num-raster-threads=1") && small.contains("--renderer-process-limit=4"));
        assert!(small.contains("WebContentsForceDark") && small.contains("msSmartScreenProtection") && small.contains("--process-per-site"));
        let big = browser_args(16, 32, false, false, false, None, "");
        assert!(big.contains("--num-raster-threads=4") && !big.contains("renderer-process-limit"));
        assert!(browser_args(16, 32, false, true, false, None, "").contains("--renderer-process-limit=12"));
        assert!(browser_args(8, 16, false, true, false, None, "").contains("--renderer-process-limit=8"));
        assert!(!big.contains("ForceDark") && !big.contains("--lang"));
        // alerte sur les sites dangereux demandée : SmartScreen n'est plus coupé, le reste de la liste ne bouge pas
        let safe = browser_args(16, 32, false, false, true, None, "");
        assert!(safe.contains("--disable-features=msWebOOUI,msPdfOOUI ") && !safe.contains("msSmartScreenProtection"));
        assert!(browser_args(16, 32, false, false, false, Some("en"), "").contains(" --lang=en "));
        // une seule liste de features, fusionnée avec celle de VOLT_ARGS
        let x = browser_args(16, 32, true, false, false, None, "--process-per-site --enable-features=Foo:Bar/20");
        assert!(x.contains("--enable-features=ParallelDownloading,WebContentsForceDark,Foo:Bar/20") && x.contains(" --process-per-site"));
        assert_eq!(x.matches("--enable-features").count(), 1);
        let (c, r) = machine();
        assert!(c >= 1 && r >= 1);
    }

    #[test]
    fn gpu_counters() {
        assert_eq!(gpu_instance("pid_1234_luid_0x00000000_0x0000D1F6_phys_0_eng_0_engtype_3D"), Some((1234, "3D")));
        assert_eq!(gpu_instance("pid_88_luid_0x00000000_0x0000D1F6_phys_0"), Some((88, ""))); // mémoire vidéo : pas de type
        assert_eq!(gpu_instance("_Total"), None);
        assert_eq!(gpu_instance("pid_x_luid"), None);
        let items: Vec<(String, f64)> = [
            ("pid_1_luid_0_phys_0_eng_0_engtype_3D", 10.0),
            ("pid_2_luid_0_phys_0_eng_0_engtype_3D", 5.0),
            ("pid_2_luid_0_phys_0_eng_3_engtype_VideoDecode", 40.0),
            ("pid_9_luid_0_phys_0_eng_0_engtype_3D", 90.0), // un autre programme : ignoré
        ]
        .map(|(n, v)| (n.to_string(), v))
        .into();
        assert_eq!(gpu_pct(&items, &[1, 2]), 40.0); // le type le plus chargé, pas la somme de tout
        assert_eq!(gpu_pct(&items, &[1]), 10.0);
        assert_eq!(gpu_pct(&items, &[3]), 0.0);
        assert_eq!(gpu_pct(&[("pid_1_luid_0_engtype_3D".into(), 70.0), ("pid_2_luid_0_engtype_3D".into(), 60.0)], &[1, 2]), 100.0);
    }

    #[test]
    fn own_usage() {
        // ce processus de test : de la mémoire, et du temps processeur qui ne recule pas
        let me = [std::process::id()];
        let (ram, cpu) = sample(&me);
        assert!(ram > 1 << 20, "{ram}");
        let mut x = 0u64;
        for i in 0..20_000_000u64 {
            x = x.wrapping_add(std::hint::black_box(i));
        }
        assert!(sample(&me).1 >= cpu && x > 0);
        assert_eq!(sample(&[0xFFFF_FFF0]), (0, 0)); // processus inexistant
    }

    #[test]
    fn task_manager_rows() {
        let t0 = std::time::Instant::now();
        let mut before: Before = None;
        // 4 cœurs ; le processus 7 a consommé 2 s de processeur en 1 s : la moitié de la machine
        let cpu = std::cell::Cell::new(0u64);
        let sample = |p: &[u32]| (if p[0] == 7 { 300 << 20 } else { 50 << 20 }, if p[0] == 7 { cpu.get() } else { 0 });
        let first = measure(vec![(7, 1, vec![3]), (8, 4, vec![])], 4, &mut before, t0, sample);
        assert_eq!(first[0], Task { pid: 7, kind: 1, frames: vec![3], ram_mb: 300, cpu: None }); // premier relevé : pas d'écart
        cpu.set(20_000_000);
        let next = measure(vec![(7, 1, vec![3]), (9, 1, vec![])], 4, &mut before, t0 + std::time::Duration::from_secs(1), sample);
        assert_eq!(next[0].cpu.map(|c| c.round()), Some(50.0));
        assert_eq!((next[1].pid, next[1].cpu, next[1].ram_mb), (9, None, 50)); // processus apparu entre-temps
        // relevé précédent trop vieux : on repart de zéro
        let late = measure(vec![(7, 1, vec![])], 4, &mut before, t0 + std::time::Duration::from_secs(60), sample);
        assert_eq!(late[0].cpu, None);
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
