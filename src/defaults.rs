// Navigateur par défaut (enregistrement HKCU), URL reçue en argument et instance unique par profil.
use std::{
    fs::File,
    io::Read,
    os::windows::io::{AsRawHandle, FromRawHandle},
    path::Path,
};

use windows::{
    Win32::{
        Foundation::{ERROR_NO_DATA, ERROR_PIPE_CONNECTED, ERROR_SUCCESS, HANDLE},
        Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND},
        System::{
            Pipes::{ConnectNamedPipe, CreateNamedPipeW, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES},
            Registry::{HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegGetValueW, RegSetKeyValueW},
        },
        UI::{
            Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify},
            WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow},
        },
    },
    core::HSTRING,
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

/// Première URL valide parmi les arguments ; options et autres schémas ignorés.
pub fn arg_url(args: impl Iterator<Item = String>) -> Option<String> {
    args.filter_map(|a| web_url(&a)).next()
}

/// (clé sous HKCU, nom de valeur, donnée) de l'enregistrement comme navigateur ; "" = valeur par défaut.
pub fn entries(exe: &str, app: &str) -> Vec<(String, String, String)> {
    let (class, client) = (format!(r"Software\Classes\{app}HTML"), format!(r"Software\Clients\StartMenuInternet\{app}"));
    let (icon, caps) = (format!("{exe},0"), format!(r"{client}\Capabilities"));
    vec![
        (class.clone(), "", format!("{app} HTML Document")),
        (class.clone(), "URL Protocol", String::new()),
        (format!(r"{class}\DefaultIcon"), "", icon.clone()),
        (format!(r"{class}\shell\open\command"), "", format!("\"{exe}\" \"%1\"")),
        (client.clone(), "", app.into()),
        (format!(r"{client}\DefaultIcon"), "", icon.clone()),
        (format!(r"{client}\shell\open\command"), "", format!("\"{exe}\"")),
        (caps.clone(), "ApplicationName", app.into()),
        (caps.clone(), "ApplicationDescription", "Navigateur web léger pour joueurs".into()),
        (caps.clone(), "ApplicationIcon", icon),
        (format!(r"{caps}\URLAssociations"), "http", format!("{app}HTML")),
        (format!(r"{caps}\URLAssociations"), "https", format!("{app}HTML")),
        (format!(r"{caps}\StartMenu"), "StartMenuInternet", app.into()),
        (r"Software\RegisteredApplications".into(), app, caps),
    ]
    .into_iter()
    .map(|(k, n, d)| (k, n.to_string(), d))
    .collect()
}

fn set(key: &str, name: &str, data: &str) -> Result<(), String> {
    let w: Vec<u16> = data.encode_utf16().chain([0]).collect();
    let r = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(name), REG_SZ.0, Some(w.as_ptr().cast()), (w.len() * 2) as u32) };
    if r == ERROR_SUCCESS { Ok(()) } else { Err(format!("registre {key} : {}", r.0)) }
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

/// Windows ne laisse pas une application se déclarer par défaut : on s'enregistre puis l'utilisateur confirme.
pub fn ask() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    register(&exe.to_string_lossy(), APP)?;
    std::process::Command::new("explorer").arg(format!("ms-settings:defaultapps?registeredAppUser={APP}")).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn is_default() -> bool {
    get(r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice", "ProgId").is_some_and(|p| p == format!("{APP}HTML"))
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

#[cfg(test)]
pub fn unregister(app: &str) {
    use windows::Win32::System::Registry::{RegDeleteKeyValueW, RegDeleteTreeW};
    unsafe {
        for k in [format!(r"Software\Classes\{app}HTML"), format!(r"Software\Clients\StartMenuInternet\{app}")] {
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(k.as_str()));
            let _ = windows::Win32::System::Registry::RegDeleteKeyW(HKEY_CURRENT_USER, &HSTRING::from(k.as_str()));
        }
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(r"Software\RegisteredApplications"), &HSTRING::from(app));
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
        assert!(e.iter().all(|x| x.0.starts_with(r"Software\") && !x.0.contains("UserChoice") && !x.0.contains(".htm")));
        assert_eq!(find(r"Software\RegisteredApplications", "Volt"), Some(r"Software\Clients\StartMenuInternet\Volt\Capabilities"));
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
        unregister(app);
        assert!(get(&format!(r"Software\Classes\{app}HTML"), "").is_none());
        assert!(get(&format!(r"Software\Clients\StartMenuInternet\{app}"), "").is_none());
        assert!(get(r"Software\RegisteredApplications", app).is_none());
    }
}
