// Navigateur par défaut (enregistrement HKCU), URL ou fichier reçu en argument, raccourcis et instance unique par profil.
use std::{
    fs::File,
    io::Read,
    os::windows::io::{AsRawHandle, FromRawHandle},
    path::Path,
};

use windows::{
    Win32::{
        Foundation::{ERROR_NO_DATA, ERROR_PIPE_CONNECTED, ERROR_SUCCESS, HANDLE, HWND},
        Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND},
        System::{
            Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree, IPersistFile},
            Pipes::{ConnectNamedPipe, CreateNamedPipeW, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, WaitNamedPipeW},
            Registry::{HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegGetValueW, RegSetKeyValueW},
        },
        UI::{
            Shell::{FOLDERID_Desktop, FOLDERID_Downloads, FOLDERID_Programs, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellLinkW, SIGDN_FILESYSPATH, KF_FLAG_DEFAULT, SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify, SHGetKnownFolderPath, ShellExecuteW, ShellLink},
            WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow, SW_SHOWNORMAL},
        },
    },
    core::{HSTRING, Interface},
};

pub const APP: &str = "Volt";

/// Seules les adresses http(s) avec un hôte passent : n'importe quel programme peut lancer le navigateur par défaut.
pub fn web_url(s: &str) -> Option<String> {
    let t = s.trim();
    let l = t.to_ascii_lowercase();
    let rest = l.strip_prefix("https://").or_else(|| l.strip_prefix("http://"))?;
    let host_ok = !rest.split(['/', '?', '#']).next()?.is_empty();
    (host_ok && !t.chars().any(|c| c.is_control() || c.is_whitespace())).then(|| t.to_string())
}

/// Fichier local existant (PDF, page enregistrée, image… : ce que Chrome ouvre aussi) -> adresse file:///.
pub fn file_url(s: &str) -> Option<String> {
    let p = std::path::absolute(s.trim().trim_matches('"')).ok().filter(|p| p.is_file())?;
    let mut u = String::from("file:///");
    for b in p.to_string_lossy().replace('\\', "/").bytes() {
        // tout le reste est encodé : « # », « ? » et « % » d'un nom de fichier couperaient l'adresse
        if b.is_ascii_alphanumeric() || b"/:-_.~".contains(&b) { u.push(b as char) } else { u.push_str(&format!("%{b:02X}")) }
    }
    Some(u)
}

/// Premier argument ouvrable : adresse http(s) ou fichier existant ; options et autres schémas ignorés.
pub fn arg_url(args: impl Iterator<Item = String>) -> Option<String> {
    args.filter_map(|a| web_url(&a).or_else(|| file_url(&a))).next()
}

/// Message reçu par le tube : ce qu'`arg_url` produit (adresse http(s) ou file:///), rien d'autre.
pub fn msg_url(m: &str) -> Option<String> {
    web_url(m).or_else(|| (m.starts_with("file:///") && !m.chars().any(|c| c.is_control() || c.is_whitespace())).then(|| m.to_string()))
}

// Types de fichiers que Volt sait afficher (ceux que Chrome déclare) : proposé dans « Ouvrir avec », jamais imposé.
const FILE_TYPES: &[&str] = &[".pdf", ".htm", ".html", ".xhtml", ".svg", ".webp"];

/// (clé sous HKCU, nom de valeur, donnée) de l'enregistrement comme navigateur ; "" = valeur par défaut.
pub fn entries(exe: &str, app: &str) -> Vec<(String, String, String)> {
    let (class, client) = (format!(r"Software\Classes\{app}HTML"), format!(r"Software\Clients\StartMenuInternet\{app}"));
    let (icon, caps) = (format!("{exe},0"), format!(r"{client}\Capabilities"));
    let pdf = format!(r"Software\Classes\{app}PDF"); // classe à part : un PDF ne s'appelle pas « HTML Document » dans l'Explorateur
    let mut e: Vec<(String, String, String)> = vec![
        (class.clone(), "", format!("{app} HTML Document")),
        (class.clone(), "URL Protocol", String::new()),
        (format!(r"{class}\DefaultIcon"), "", icon.clone()),
        (format!(r"{class}\shell\open\command"), "", format!("\"{exe}\" \"%1\"")),
        (pdf.clone(), "", format!("{app} PDF Document")),
        (format!(r"{pdf}\DefaultIcon"), "", icon.clone()),
        (format!(r"{pdf}\shell\open\command"), "", format!("\"{exe}\" \"%1\"")),
        (client.clone(), "", app.into()),
        (format!(r"{client}\DefaultIcon"), "", icon.clone()),
        (format!(r"{client}\shell\open\command"), "", format!("\"{exe}\"")),
        (caps.clone(), "ApplicationName", app.into()),
        (caps.clone(), "ApplicationDescription", crate::lang::t("Navigateur web léger pour joueurs")),
        (caps.clone(), "ApplicationIcon", icon),
        (format!(r"{caps}\URLAssociations"), "http", format!("{app}HTML")),
        (format!(r"{caps}\URLAssociations"), "https", format!("{app}HTML")),
        (format!(r"{caps}\StartMenu"), "StartMenuInternet", app.into()),
        (r"Software\RegisteredApplications".into(), app, caps.clone()),
    ]
    .into_iter()
    .map(|(k, n, d)| (k, n.to_string(), d))
    .collect();
    for ext in FILE_TYPES {
        let id = format!("{app}{}", if *ext == ".pdf" { "PDF" } else { "HTML" });
        e.push((format!(r"{caps}\FileAssociations"), ext.to_string(), id.clone()));
        // liste « Ouvrir avec » de l'extension : on s'y ajoute, la valeur par défaut de la clé n'est jamais écrite
        e.push((format!(r"Software\Classes\{ext}\OpenWithProgids"), id, String::new()));
    }
    e.extend(uninstall_entries(exe, app));
    e
}

fn uninstall_key(app: &str) -> String {
    format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{app}")
}

/// Ce qui fait apparaître Volt dans « Applications installées » de Windows, avec son bouton Désinstaller.
fn uninstall_entries(exe: &str, app: &str) -> Vec<(String, String, String)> {
    let dir = Path::new(exe).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    [
        ("DisplayName", app.to_string()),
        ("DisplayVersion", env!("CARGO_PKG_VERSION").to_string()),
        ("Publisher", "Manasy9999".to_string()),
        ("DisplayIcon", format!("{exe},0")),
        ("InstallLocation", dir),
        ("UninstallString", format!("\"{exe}\" --uninstall")),
    ]
    .into_iter()
    .map(|(n, v)| (uninstall_key(app), n.to_string(), v))
    .collect()
}

fn set(key: &str, name: &str, data: &str) -> Result<(), String> {
    let w: Vec<u16> = data.encode_utf16().chain([0]).collect();
    let r = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(name), REG_SZ.0, Some(w.as_ptr().cast()), (w.len() * 2) as u32) };
    if r == ERROR_SUCCESS { Ok(()) } else { Err(crate::lang::te("registre {e}", format!("{key} : {}", r.0))) }
}

fn get(key: &str, name: &str) -> Option<String> {
    let mut buf = [0u16; 512];
    let mut n = (buf.len() * 2) as u32;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(name), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut n)) };
    (r == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..(n as usize / 2).saturating_sub(1)]))
}

/// Écrit (ou réécrit, idempotent) l'enregistrement de `app` pour l'utilisateur courant, sans droits administrateur.
pub fn register(exe: &str, app: &str) -> Result<(), String> {
    for (k, n, d) in entries(exe, app) {
        set(&k, &n, &d)?;
    }
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) }; // l'Explorateur relit les associations
    Ok(())
}

/// Volt déjà enregistré avec cet exe : complète sans rien demander ce qui manque (types de fichiers ajoutés depuis).
/// Un autre exe (copie d'essai) ne touche à rien.
pub fn refresh() {
    let Ok(exe) = std::env::current_exe() else { return };
    let e = entries(&exe.to_string_lossy(), APP);
    let cmd = format!(r"Software\Classes\{APP}HTML\shell\open\command");
    let has = |x: &(String, String, String)| get(&x.0, &x.1).as_deref() == Some(x.2.as_str());
    if e.iter().any(|x| x.0 == cmd && has(x)) && !e.iter().all(has) {
        let _ = register(&exe.to_string_lossy(), APP);
    }
}

/// Raccourci `Volt.lnk` vers `exe` dans `dir` ; un raccourci déjà là n'est pas touché (il peut viser une autre copie).
fn shortcut(dir: &Path, exe: &Path) -> windows::core::Result<()> {
    let lnk = dir.join(format!("{APP}.lnk"));
    if lnk.exists() {
        return Ok(());
    }
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(&HSTRING::from(exe.as_os_str()))?;
        link.cast::<IPersistFile>()?.Save(&HSTRING::from(lnk.as_os_str()), true)
    }
}

/// Raccourcis du bureau et du menu Démarrer vers cet exe. Dossiers demandés à Windows : le bureau peut être déplacé.
pub fn shortcuts() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    for id in [&FOLDERID_Desktop, &FOLDERID_Programs] {
        let dir = unsafe {
            let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).map_err(|e| e.message())?;
            let d = String::from_utf16_lossy(p.as_wide());
            CoTaskMemFree(Some(p.0.cast()));
            d
        };
        shortcut(Path::new(&dir), &exe).map_err(|e| e.message())?;
    }
    // installé : Windows doit aussi savoir le désinstaller
    for (k, n, v) in uninstall_entries(&exe.to_string_lossy(), APP) {
        set(&k, &n, &v)?;
    }
    Ok(())
}

/// Dossier connu de Windows (bureau, menu Démarrer…), là où il se trouve vraiment.
fn known_dir(id: &windows::core::GUID) -> Option<std::path::PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let s = String::from_utf16_lossy(p.as_wide());
        CoTaskMemFree(Some(p.0.cast()));
        Some(s.into())
    }
}

/// Programme visé par un raccourci .lnk.
fn link_target(lnk: &Path) -> Option<String> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        link.cast::<IPersistFile>().ok()?.Load(&HSTRING::from(lnk.as_os_str()), windows::Win32::System::Com::STGM_READ).ok()?;
        let mut buf = [0u16; 1024];
        link.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
        Some(String::from_utf16_lossy(&buf[..buf.iter().position(|c| *c == 0)?]))
    }
}

/// Retire Volt de Windows. Ne touche qu'à ce qui vise CET exe : un autre Volt installé ailleurs garde ses raccourcis et
/// son enregistrement. `data` : dossier des données à supprimer aussi (None : gardées). Renvoie ce qui a été retiré.
pub fn uninstall(exe: &str, app: &str, dirs: &[std::path::PathBuf], data: Option<&Path>) -> Vec<String> {
    let mut done = vec![];
    let same = |a: &str| a.eq_ignore_ascii_case(exe);
    for lnk in dirs.iter().map(|d| d.join(format!("{app}.lnk"))) {
        if link_target(&lnk).is_some_and(|t| same(&t)) && std::fs::remove_file(&lnk).is_ok() {
            done.push(lnk.to_string_lossy().into_owned());
        }
    }
    // navigateur enregistré : seulement si la commande inscrite est celle de cet exe
    if get(&format!(r"Software\Classes\{app}HTML\shell\open\command"), "").is_some_and(|c| c == format!("\"{exe}\" \"%1\"")) {
        unregister(app);
        done.push("registre : navigateur".into());
    }
    if get(&uninstall_key(app), "UninstallString").is_some_and(|c| c == format!("\"{exe}\" --uninstall")) {
        let _ = unsafe { windows::Win32::System::Registry::RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(uninstall_key(app))) };
        let _ = unsafe { windows::Win32::System::Registry::RegDeleteKeyW(HKEY_CURRENT_USER, &HSTRING::from(uninstall_key(app))) };
        done.push("registre : applications installées".into());
    }
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    if let Some(d) = data
        && std::fs::remove_dir_all(d).is_ok()
    {
        done.push(d.to_string_lossy().into_owned());
    }
    done
}

/// Dossiers où `shortcuts` pose ses raccourcis.
pub fn shortcut_dirs() -> Vec<std::path::PathBuf> {
    [&FOLDERID_Desktop, &FOLDERID_Programs].into_iter().filter_map(known_dir).collect()
}

/// Fenêtre Windows de choix d'un dossier, au-dessus de la fenêtre `owner` ; None : annulé.
pub fn pick_folder(owner: isize) -> Option<String> {
    unsafe {
        let d: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        d.SetOptions(d.GetOptions().ok()? | FOS_PICKFOLDERS).ok()?;
        d.Show(Some(HWND(owner as _))).ok()?;
        let p = d.GetResult().ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = String::from_utf16_lossy(p.as_wide());
        CoTaskMemFree(Some(p.0.cast()));
        Some(s)
    }
}

/// Ouvre un fichier ou un dossier avec le programme que Windows lui associe.
pub fn open_path(p: &str) -> bool {
    let r = unsafe { ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(p), None, None, SW_SHOWNORMAL) };
    r.0 as usize > 32
}

/// Ouvre l'Explorateur sur le dossier du fichier, fichier sélectionné ; fichier disparu : son dossier.
/// Par le Shell et non `explorer /select,…` : Rust met entre guillemets un chemin à espaces, et l'Explorateur, qui ne
/// comprend pas « "/select,C:\a b\f" », ouvrait alors un autre dossier sans rien sélectionner.
pub fn reveal(p: &str) {
    use windows::Win32::UI::Shell::{ILFree, SHOpenFolderAndSelectItems, SHParseDisplayName};
    unsafe {
        let mut pidl: *mut windows::Win32::UI::Shell::Common::ITEMIDLIST = std::ptr::null_mut();
        if SHParseDisplayName(&HSTRING::from(p), None, &mut pidl, 0, None).is_ok() && !pidl.is_null() {
            let _ = SHOpenFolderAndSelectItems(pidl, None, 0);
            ILFree(Some(pidl));
        } else if let Some(d) = std::path::Path::new(p).parent() {
            open_path(&d.to_string_lossy());
        }
    }
}

/// Dossier « Téléchargements » de Windows (il peut avoir été déplacé).
pub fn downloads_dir() -> Option<String> {
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, None).ok()?;
        let s = String::from_utf16_lossy(p.as_wide());
        CoTaskMemFree(Some(p.0.cast()));
        Some(s)
    }
}

/// Windows ne laisse pas une application se déclarer par défaut : on s'enregistre puis l'utilisateur confirme.
pub fn ask() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    register(&exe.to_string_lossy(), APP)?;
    // ShellExecute et non `explorer <adresse>` : l'Explorateur ouvrait un dossier au lieu de la page des Paramètres
    let r = unsafe { ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(format!("ms-settings:defaultapps?registeredAppUser={APP}")), None, None, SW_SHOWNORMAL) };
    if r.0 as usize > 32 { Ok(()) } else { Err(crate::lang::t("impossible d'ouvrir les Paramètres de Windows")) }
}

pub fn is_default() -> bool {
    // Windows 11 récent note le choix sous « UserChoiceLatest\ProgId » et laisse l'ancienne clé telle quelle (périmée)
    let k = r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https";
    get(&format!(r"{k}\UserChoiceLatest\ProgId"), "ProgId").or_else(|| get(&format!(r"{k}\UserChoice"), "ProgId")).is_some_and(|p| p == format!("{APP}HTML"))
}

/// Un Volt écoute déjà ce tube (réponse immédiate s'il n'y en a pas, courte attente s'il est occupé) ; ne s'y connecte pas.
pub fn alive(name: &str) -> bool {
    unsafe { WaitNamedPipeW(&HSTRING::from(name), 2000) }.as_bool()
}

/// Tube propre au dossier de données : un profil de test (VOLT_DATA) ne parle jamais au vrai Volt.
pub fn pipe_name(dir: &Path) -> String {
    let d = std::fs::canonicalize(dir).unwrap_or(dir.into()).to_string_lossy().to_lowercase();
    // FNV-1a : stable d'une compilation à l'autre, contrairement au hacheur de std
    let h = d.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    format!(r"\\.\pipe\volt-{h:016x}")
}

/// Une instance du tube, en File pour la lire et la fermer avec std.
fn create(name: &str, first: bool) -> Option<File> {
    let flags = if first { PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE } else { PIPE_ACCESS_INBOUND };
    let h = unsafe { CreateNamedPipeW(&HSTRING::from(name), flags, PIPE_TYPE_BYTE | PIPE_REJECT_REMOTE_CLIENTS, PIPE_UNLIMITED_INSTANCES, 0, 4096, 0, None) };
    (!h.is_invalid()).then(|| unsafe { File::from_raw_handle(h.0) })
}

/// Premier Volt du profil : écoute le tube et passe chaque message reçu (URL ou vide) à `on`.
/// None : un autre Volt tient déjà ce tube.
pub fn listen(name: &str, on: impl Fn(String) + Send + 'static) -> Option<()> {
    let mut h = create(name, true)?;
    let name = name.to_string();
    std::thread::spawn(move || {
        loop {
            // client déjà passé (connecté, voire reparti après avoir écrit) : son message est lu quand même
            let ok = unsafe { ConnectNamedPipe(HANDLE(h.as_raw_handle()), None) }.map_or_else(|e| [ERROR_PIPE_CONNECTED, ERROR_NO_DATA].iter().any(|c| c.to_hresult() == e.code()), |_| true);
            // instance suivante créée avant de fermer celle-ci : le tube ne disparaît jamais, aucun second Volt ne peut le prendre
            let Some(next) = create(&name, false) else { return };
            if ok {
                let mut msg = String::new();
                let _ = (&h).take(8192).read_to_string(&mut msg);
                on(msg);
            }
            h = next;
        }
    });
    Some(())
}

/// Second Volt : transmet l'URL au premier ; false si personne n'a répondu.
pub fn send(name: &str, url: Option<&str>) -> bool {
    let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) }; // le premier Volt a le droit de passer devant
    for _ in 0..40 {
        // occupé (231) ou entre deux instances : on réessaie un court instant
        match std::fs::OpenOptions::new().write(true).open(name) {
            Ok(mut f) => return std::io::Write::write_all(&mut f, url.unwrap_or_default().as_bytes()).is_ok(),
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
    false
}

/// Efface l'enregistrement comme navigateur (classes, capacités, listes « Ouvrir avec »).
fn unregister(app: &str) {
    use windows::Win32::System::Registry::{RegDeleteKeyValueW, RegDeleteTreeW};
    unsafe {
        for k in [format!(r"Software\Classes\{app}HTML"), format!(r"Software\Classes\{app}PDF"), format!(r"Software\Clients\StartMenuInternet\{app}")] {
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(k.as_str()));
            let _ = windows::Win32::System::Registry::RegDeleteKeyW(HKEY_CURRENT_USER, &HSTRING::from(k.as_str()));
        }
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(r"Software\RegisteredApplications"), &HSTRING::from(app));
        for (ext, id) in FILE_TYPES.iter().flat_map(|e| [(e, "HTML"), (e, "PDF")]) {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(format!(r"Software\Classes\{ext}\OpenWithProgids")), &HSTRING::from(format!("{app}{id}")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_urls() {
        assert_eq!(web_url(" https://a.fr/x?y=1 ").as_deref(), Some("https://a.fr/x?y=1"));
        assert_eq!(web_url("HTTP://A.fr").as_deref(), Some("HTTP://A.fr"));
        for bad in ["javascript:alert(1)", "file:///C:/Windows/win.ini", "https://", "http:///x", "--flag", "a.fr", "https://a.fr/\nx", "https://a b.fr", "ftp://a.fr", ""] {
            assert!(web_url(bad).is_none(), "{bad}");
        }
        let args = ["--mute", "javascript:x", "https://b.fr", "https://c.fr"].map(String::from);
        assert_eq!(arg_url(args.into_iter()).as_deref(), Some("https://b.fr"));
    }

    #[test]
    fn registry_entries() {
        let e = entries(r"C:\V\volt.exe", "Volt");
        let find = |k: &str, n: &str| e.iter().find(|x| x.0 == k && x.1 == n).map(|x| x.2.as_str());
        assert_eq!(find(r"Software\Classes\VoltHTML\shell\open\command", ""), Some(r#""C:\V\volt.exe" "%1""#));
        assert_eq!(find(r"Software\Clients\StartMenuInternet\Volt\Capabilities\URLAssociations", "https"), Some("VoltHTML"));
        assert_eq!(find(r"Software\Clients\StartMenuInternet\Volt\Capabilities\URLAssociations", "http"), Some("VoltHTML"));
        assert_eq!(find(r"Software\Clients\StartMenuInternet\Volt\Capabilities\StartMenu", "StartMenuInternet"), Some("Volt"));
        // types de fichiers : Volt s'ajoute aux listes « Ouvrir avec », sans jamais prendre la place du programme par défaut
        assert_eq!(find(r"Software\Classes\VoltPDF\shell\open\command", ""), Some(r#""C:\V\volt.exe" "%1""#));
        assert_eq!(find(r"Software\Clients\StartMenuInternet\Volt\Capabilities\FileAssociations", ".pdf"), Some("VoltPDF"));
        assert_eq!(find(r"Software\Clients\StartMenuInternet\Volt\Capabilities\FileAssociations", ".html"), Some("VoltHTML"));
        assert_eq!(find(r"Software\Classes\.pdf\OpenWithProgids", "VoltPDF"), Some(""));
        assert_eq!(find(r"Software\Classes\.html\OpenWithProgids", "VoltHTML"), Some(""));
        assert!(e.iter().all(|x| x.0.starts_with(r"Software\") && !x.0.contains("UserChoice")));
        assert!(e.iter().filter(|x| x.0.starts_with(r"Software\Classes\.")).all(|x| x.0.ends_with(r"\OpenWithProgids") && !x.1.is_empty()));
        assert_eq!(find(r"Software\RegisteredApplications", "Volt"), Some(r"Software\Clients\StartMenuInternet\Volt\Capabilities"));
    }

    #[test]
    fn local_files() {
        let f = std::env::temp_dir().join(format!("volt essai #{} 100%.pdf", std::process::id()));
        std::fs::write(&f, b"%PDF-1.4").unwrap();
        let p = f.to_string_lossy().to_string();
        let u = file_url(&p).unwrap();
        assert!(u.starts_with("file:///") && u.ends_with("%20100%25.pdf") && u.contains("volt%20essai%20%23") && !u.contains('\\'), "{u}");
        assert_eq!(file_url(&format!("\"{p}\"")), Some(u.clone())); // « Copier en tant que chemin d'accès » met des guillemets
        assert_eq!(arg_url(["--restart".to_string(), p.clone()].into_iter()), Some(u.clone()));
        assert_eq!(msg_url(&u), Some(u.clone()));
        assert_eq!(msg_url("https://a.fr").as_deref(), Some("https://a.fr"));
        for bad in ["javascript:alert(1)", "file:///C:/a b.pdf", "file://serveur/x", r"C:\x.pdf", ""] {
            assert!(msg_url(bad).is_none(), "{bad}");
        }
        std::fs::remove_file(&f).unwrap();
        assert!(file_url(&p).is_none()); // fichier disparu
        assert!(file_url(&std::env::temp_dir().to_string_lossy()).is_none()); // un dossier n'est pas un fichier
        assert!(file_url("").is_none() && file_url("https://a.fr").is_none() && file_url("--mute").is_none());
    }

    #[test]
    fn shortcut_file() {
        use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, STGM_READ};
        let dir = std::env::temp_dir().join(format!("volt-lnk-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = std::env::current_exe().unwrap();
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            shortcut(&dir, &exe).unwrap();
            let lnk = dir.join("Volt.lnk");
            // relu par Windows : le raccourci vise bien l'exe
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
            link.cast::<IPersistFile>().unwrap().Load(&HSTRING::from(lnk.as_os_str()), STGM_READ).unwrap();
            let mut buf = [0u16; 1024];
            link.GetPath(&mut buf, std::ptr::null_mut(), 0).unwrap();
            let got = String::from_utf16_lossy(&buf[..buf.iter().position(|c| *c == 0).unwrap()]);
            assert_eq!(got.to_lowercase(), exe.to_string_lossy().to_lowercase());
            // déjà là : pas réécrit, même vers un autre exe
            let before = std::fs::read(&lnk).unwrap();
            shortcut(&dir, Path::new(r"C:\Windows\notepad.exe")).unwrap();
            assert_eq!(std::fs::read(&lnk).unwrap(), before);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn uninstall_only_what_is_ours() {
        use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let base = std::env::temp_dir().join(format!("volt-uninst-test-{}", std::process::id()));
        let (bureau, menu, data) = (base.join("bureau"), base.join("menu"), base.join("donnees"));
        for d in [&bureau, &menu, &data] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(data.join("settings.json"), b"{}").unwrap();
        let exe = std::env::current_exe().unwrap();
        let app = format!("VoltTestU{}", std::process::id()); // jamais inscrit au registre : rien à y retirer
        // un raccourci à nous (bureau), un autre du même nom qui vise un autre programme (menu)
        let mk = |dir: &Path, target: &Path| unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
            link.SetPath(&HSTRING::from(target.as_os_str())).unwrap();
            link.cast::<IPersistFile>().unwrap().Save(&HSTRING::from(dir.join(format!("{app}.lnk")).as_os_str()), true).unwrap();
        };
        mk(&bureau, &exe);
        mk(&menu, Path::new(r"C:\Windows\notepad.exe"));
        assert_eq!(link_target(&bureau.join(format!("{app}.lnk"))).unwrap().to_lowercase(), exe.to_string_lossy().to_lowercase());
        // données gardées
        let done = uninstall(&exe.to_string_lossy(), &app, &[bureau.clone(), menu.clone()], None);
        assert_eq!(done.len(), 1, "{done:?}");
        assert!(!bureau.join(format!("{app}.lnk")).exists() && menu.join(format!("{app}.lnk")).exists() && data.join("settings.json").exists());
        // données supprimées aussi
        let done = uninstall(&exe.to_string_lossy(), &app, &[bureau.clone(), menu.clone()], Some(&data));
        assert!(done.len() == 1 && !data.exists() && menu.join(format!("{app}.lnk")).exists());
        std::fs::remove_dir_all(&base).unwrap();
        // entrées « Applications installées » : commande de désinstallation de cet exe
        let e = uninstall_entries(r"C:\V\volt.exe", "Volt");
        let find = |n: &str| e.iter().find(|x| x.1 == n).map(|x| x.2.as_str());
        assert!(e.iter().all(|x| x.0 == r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Volt"));
        assert_eq!((find("UninstallString"), find("InstallLocation"), find("DisplayName")), (Some(r#""C:\V\volt.exe" --uninstall"#), Some(r"C:\V"), Some("Volt")));
        assert!(!shortcut_dirs().is_empty());
    }

    #[test]
    fn pipe_per_profile() {
        let t = std::env::temp_dir();
        assert_eq!(pipe_name(&t), pipe_name(&t));
        assert_ne!(pipe_name(&t), pipe_name(&t.join("autre")));
        assert!(pipe_name(&t).starts_with(r"\\.\pipe\volt-"));
    }

    #[test]
    fn pipe_roundtrip() {
        let name = pipe_name(&std::env::temp_dir().join(format!("volt-pipe-test-{}", std::process::id())));
        let (tx, rx) = std::sync::mpsc::channel();
        listen(&name, move |m| tx.send(m).unwrap()).unwrap();
        assert!(listen(&name, |_| {}).is_none()); // second Volt : tube déjà pris
        assert!(send(&name, Some("https://a.fr")));
        assert!(send(&name, None));
        let d = std::time::Duration::from_secs(5);
        assert_eq!(rx.recv_timeout(d).unwrap(), "https://a.fr");
        assert_eq!(rx.recv_timeout(d).unwrap(), "");
    }

    // Écrit dans le registre (sous un nom jetable puis supprimé) : cargo test real_registry -- --ignored
    #[test]
    #[ignore]
    fn real_registry() {
        let app = "VoltTest";
        unregister(app);
        register(r"C:\VoltTest\volt.exe", app).unwrap();
        register(r"C:\VoltTest\volt.exe", app).unwrap(); // idempotent
        for (k, n, d) in entries(r"C:\VoltTest\volt.exe", app) {
            assert_eq!(get(&k, &n).as_deref(), Some(d.as_str()), "{k} {n}");
        }
        assert_eq!(uninstall(r"C:\AutreVolt\volt.exe", app, &[], None), Vec::<String>::new()); // un autre exe ne retire rien
        assert!(get(&format!(r"Software\Classes\{app}HTML"), "").is_some());
        assert_eq!(uninstall(r"C:\VoltTest\volt.exe", app, &[], None).len(), 2);
        assert!(get(&uninstall_key(app), "UninstallString").is_none());
        assert!(get(&format!(r"Software\Classes\{app}HTML"), "").is_none());
        assert!(get(&format!(r"Software\Clients\StartMenuInternet\{app}"), "").is_none());
        assert!(get(r"Software\RegisteredApplications", app).is_none());
    }
}
