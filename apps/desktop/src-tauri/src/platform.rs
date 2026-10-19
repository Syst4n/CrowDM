//! Windows integration: browser registration, autostart, shell actions,
//! power actions and process metrics.

use std::path::{Path, PathBuf};

use serde::Serialize;

pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Browsers whose native-messaging registry key we manage.
pub const BROWSERS: &[(&str, &str, bool)] = &[
    // (id, registry path, uses Firefox-style manifest)
    ("chrome", r"Software\Google\Chrome\NativeMessagingHosts", false),
    ("edge", r"Software\Microsoft\Edge\NativeMessagingHosts", false),
    ("chromium", r"Software\Chromium\NativeMessagingHosts", false),
    ("brave", r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts", false),
    ("chrome-for-testing", r"Software\Google\Chrome for Testing\NativeMessagingHosts", false),
    ("firefox", r"Software\Mozilla\NativeMessagingHosts", true),
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserRegistration {
    pub browser: String,
    pub registered: bool,
    pub manifest: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationStatus {
    pub host_path: String,
    pub host_exists: bool,
    pub browsers: Vec<BrowserRegistration>,
    /// Folder holding the unpacked extensions shipped with the app.
    pub extensions_dir: Option<String>,
}

pub fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("."))
}

pub fn host_path() -> PathBuf {
    exe_dir().join("crowdm-host.exe")
}

/// Unpacked extension folders: bundled resources when installed, the build
/// output when running from the repository.
pub fn extensions_dir() -> Option<PathBuf> {
    let candidates = [
        exe_dir().join("extensions"),
        exe_dir().join("resources").join("extensions"),
        exe_dir().join("..").join("..").join("extensions").join("build"),
    ];
    candidates.into_iter().find(|p| p.join("chromium").join("manifest.json").exists())
}

/// Where application data lives: `data\` next to the executable in portable
/// mode (a `portable` marker file beside the exe), else `%LOCALAPPDATA%\CrowDM`.
pub fn data_dir() -> PathBuf {
    // Explicit override, used by the end-to-end tests.
    if let Some(d) = std::env::var_os("CROWDM_DATA_DIR") {
        return PathBuf::from(d);
    }
    let dir = exe_dir();
    if dir.join("portable").exists() || dir.join("portable.txt").exists() {
        return dir.join("data");
    }
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("CrowDM")
}

#[cfg(windows)]
mod imp {
    use super::*;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;

    fn manifest_json(firefox: bool, host: &Path) -> String {
        let mut v = serde_json::json!({
            "name": crowdm_ipc::HOST_NAME,
            "description": "CrowDM browser integration",
            "path": host.to_string_lossy(),
            "type": "stdio",
        });
        if firefox {
            v["allowed_extensions"] = serde_json::json!([crowdm_ipc::FIREFOX_EXTENSION_ID]);
        } else {
            v["allowed_origins"] = serde_json::json!([format!("chrome-extension://{}/", crowdm_ipc::CHROMIUM_EXTENSION_ID)]);
        }
        serde_json::to_string_pretty(&v).unwrap()
    }

    /// Registers the native messaging host for every supported browser
    /// (current user only, no administrator rights needed).
    pub fn register_browsers(data_dir: &Path) -> std::io::Result<()> {
        let host = host_path();
        let dir = data_dir.join("native-messaging");
        std::fs::create_dir_all(&dir)?;
        let chromium_manifest = dir.join("com.crowdm.host.chromium.json");
        let firefox_manifest = dir.join("com.crowdm.host.firefox.json");
        std::fs::write(&chromium_manifest, manifest_json(false, &host))?;
        std::fs::write(&firefox_manifest, manifest_json(true, &host))?;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        for (_, path, firefox) in BROWSERS {
            let (key, _) = hkcu.create_subkey(format!(r"{path}\{}", crowdm_ipc::HOST_NAME))?;
            let manifest = if *firefox { &firefox_manifest } else { &chromium_manifest };
            key.set_value("", &manifest.to_string_lossy().to_string())?;
        }
        let (app_key, _) = hkcu.create_subkey(r"Software\CrowDM")?;
        if let Ok(exe) = std::env::current_exe() {
            app_key.set_value("AppPath", &exe.to_string_lossy().to_string())?;
        }
        Ok(())
    }

    pub fn integration_status() -> IntegrationStatus {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let host = host_path();
        let browsers = BROWSERS
            .iter()
            .map(|(id, path, _)| {
                let manifest: Option<String> = hkcu
                    .open_subkey_with_flags(format!(r"{path}\{}", crowdm_ipc::HOST_NAME), KEY_READ)
                    .ok()
                    .and_then(|k| k.get_value("").ok());
                let registered = manifest.as_deref().is_some_and(|m| {
                    std::fs::read_to_string(m).map(|s| s.contains(&host.to_string_lossy().replace('\\', "\\\\"))).unwrap_or(false)
                });
                BrowserRegistration { browser: id.to_string(), registered, manifest }
            })
            .collect();
        IntegrationStatus {
            host_path: host.to_string_lossy().into_owned(),
            host_exists: host.exists(),
            browsers,
            extensions_dir: extensions_dir().map(|p| p.to_string_lossy().into_owned()),
        }
    }

    pub fn set_autostart(enabled: bool) -> std::io::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(RUN_KEY)?;
        if enabled {
            let exe = std::env::current_exe()?;
            key.set_value("CrowDM", &format!("\"{}\" --background", exe.display()))?;
        } else {
            let _ = key.delete_value("CrowDM");
        }
        Ok(())
    }

    /// Advertise handlers without overwriting .torrent/magnet UserChoice or
    /// another client's default. Windows owns the user's final selection.
    pub fn register_torrent_handlers() -> std::io::Result<()> {
        register_torrent_handlers_at(&RegKey::predef(HKEY_CURRENT_USER), &std::env::current_exe()?)?;
        unsafe {
            windows_sys::Win32::UI::Shell::SHChangeNotify(0x0800_0000, 0, std::ptr::null(), std::ptr::null());
        }
        Ok(())
    }

    pub fn register_notification_identity(data_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(data_dir)?;
        let icon = data_dir.join("notification-icon.png");
        let bytes = include_bytes!("../icons/128x128.png");
        if std::fs::read(&icon).ok().as_deref() != Some(bytes.as_slice()) {
            std::fs::write(&icon, bytes)?;
        }
        register_notification_at(&RegKey::predef(HKEY_CURRENT_USER), &icon)?;
        let id = wide(std::ffi::OsStr::new(crate::notifications::APP_ID));
        unsafe {
            windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(id.as_ptr());
        }
        Ok(())
    }
    fn register_notification_at(root: &RegKey, icon: &Path) -> std::io::Result<()> {
        let (key, _) = root.create_subkey(format!(r"Software\Classes\AppUserModelId\{}", crate::notifications::APP_ID))?;
        key.set_value("DisplayName", &"CrowDM")?;
        key.set_value("IconUri", &icon.to_string_lossy().to_string())?;
        key.set_value("IconBackgroundColor", &"0")?;
        Ok(())
    }

    pub(super) fn register_torrent_handlers_at(root: &RegKey, exe: &Path) -> std::io::Result<()> {
        let command = format!("\"{}\" \"%1\"", exe.display());
        let icon = format!("\"{}\",0", exe.display());
        for (id, description, protocol) in [("CrowDM.Torrent", "BitTorrent metadata file", false), ("CrowDM.Magnet", "Magnet link", true)] {
            let base = format!(r"Software\Classes\{id}");
            let (key, _) = root.create_subkey(&base)?;
            key.set_value("", &description)?;
            if protocol {
                key.set_value("URL Protocol", &"")?;
            }
            root.create_subkey(format!(r"{base}\DefaultIcon"))?.0.set_value("", &icon)?;
            root.create_subkey(format!(r"{base}\shell\open\command"))?.0.set_value("", &command)?;
            let (app, _) = root.create_subkey(format!(r"{base}\Application"))?;
            app.set_value("ApplicationName", &"CrowDM")?;
            app.set_value("ApplicationDescription", &"Download torrents with CrowDM")?;
            app.set_value("ApplicationIcon", &icon)?;
        }
        let (app, _) = root.create_subkey(r"Software\Classes\Applications\CrowDM.exe")?;
        app.set_value("FriendlyAppName", &"CrowDM")?;
        app.create_subkey(r"shell\open\command")?.0.set_value("", &command)?;
        app.create_subkey("SupportedTypes")?.0.set_value(".torrent", &"")?;
        root.create_subkey(r"Software\Classes\.torrent\OpenWithProgids")?.0.set_value("CrowDM.Torrent", &"")?;
        root.create_subkey(r"Software\Classes\magnet\OpenWithProgids")?.0.set_value("CrowDM.Magnet", &"")?;
        let (cap, _) = root.create_subkey(r"Software\CrowDM\Capabilities")?;
        cap.set_value("ApplicationName", &"CrowDM")?;
        cap.set_value("ApplicationDescription", &"Downloads, .torrent files and magnet links")?;
        cap.set_value("ApplicationIcon", &icon)?;
        cap.create_subkey("FileAssociations")?.0.set_value(".torrent", &"CrowDM.Torrent")?;
        cap.create_subkey("URLAssociations")?.0.set_value("magnet", &"CrowDM.Magnet")?;
        root.create_subkey(r"Software\RegisteredApplications")?.0.set_value("CrowDM", &r"Software\CrowDM\Capabilities")?;
        Ok(())
    }

    pub fn choose_torrent_defaults() -> Result<(), String> {
        use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
        let op = wide(std::ffi::OsStr::new("open"));
        let uri = wide(std::ffi::OsStr::new("ms-settings:defaultapps?registeredAppUser=CrowDM"));
        let result =
            unsafe { ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), uri.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
        if result as isize <= 32 {
            return Err("Windows could not open Default apps settings.".into());
        }
        Ok(())
    }

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        s.encode_wide().chain(Some(0)).collect()
    }

    pub fn open_web_page(url: &str) -> Result<(), String> {
        use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
        let op = wide(std::ffi::OsStr::new("open"));
        let uri = wide(std::ffi::OsStr::new(url));
        let result =
            unsafe { ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), uri.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
        if result as isize <= 32 {
            return Err("Windows could not open your browser.".into());
        }
        Ok(())
    }

    /// Opens a file with its default program.
    pub fn open_path(path: &Path) -> Result<(), String> {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        if !path.exists() {
            return Err(format!("{} no longer exists.", path.display()));
        }
        let op = wide(std::ffi::OsStr::new("open"));
        let file = wide(path.as_os_str());
        let r =
            unsafe { ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
        if (r as isize) <= 32 {
            return Err("Windows could not open this file.".into());
        }
        Ok(())
    }

    /// Shows a file selected in Explorer (or opens the folder).
    pub fn reveal_path(path: &Path) -> Result<(), String> {
        use std::os::windows::process::CommandExt;
        if path.exists() {
            let arg = format!("/select,\"{}\"", path.display());
            std::process::Command::new("explorer.exe").raw_arg(arg).spawn().map_err(|e| e.to_string())?;
            return Ok(());
        }
        let dir = if path.is_dir() { path.to_path_buf() } else { path.parent().map(Path::to_path_buf).unwrap_or_default() };
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::process::Command::new("explorer.exe").arg(&dir).spawn().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn trash(path: &Path) -> Result<(), String> {
        if path.exists() {
            trash::delete(path).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn sleep() {
        use windows_sys::Win32::System::Power::SetSuspendState;
        unsafe {
            SetSuspendState(false, false, false);
        }
    }

    /// Execution state belongs to its calling thread; keep all updates and
    /// the final release on this dedicated thread.
    pub fn start_power_management(state: crate::state::AppState) {
        std::thread::spawn(move || {
            use windows_sys::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED};
            while !state.is_quitting() {
                let s = state.engine.settings();
                let jobs = state.engine.jobs();
                let active: Vec<_> = jobs.iter().filter(|j| j.job.state.is_active()).collect();
                let active_blocks = s.power.prevent_sleep_active
                    && !active.is_empty()
                    && !(s.power.allow_sleep_resumable && active.iter().all(|j| j.job.resumable == Some(true)));
                let scheduled_blocks = s.power.prevent_sleep_scheduled
                    && s.schedule.enabled
                    && jobs.iter().any(|j| j.job.queued && j.job.state == crowdm_engine::JobState::Queued);
                unsafe {
                    SetThreadExecutionState(ES_CONTINUOUS | if active_blocks || scheduled_blocks { ES_SYSTEM_REQUIRED } else { 0 });
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
            unsafe {
                SetThreadExecutionState(ES_CONTINUOUS);
            }
        });
    }

    /// Schedules a shutdown in 60 seconds; `shutdown /a` cancels it.
    pub fn shutdown_soon() {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("shutdown.exe")
            .args(["/s", "/t", "60", "/c", "CrowDM finished its queue. Run \"shutdown /a\" to cancel."])
            .creation_flags(0x0800_0000)
            .spawn();
    }

    /// Private working sets (bytes) of this process and its WebView2 children.
    pub fn memory_report() -> Vec<(u32, String, u64)> {
        let me = std::process::id();
        let mut out = Vec::new();
        // WMIC is deprecated; PowerShell's CIM query is available everywhere.
        let script = format!(
            "$all = Get-CimInstance Win32_Process; $ids = @({me}); for ($i=0; $i -lt 4; $i++) {{ $ids += ($all | Where-Object {{ $ids -contains $_.ParentProcessId }} | ForEach-Object ProcessId) ; $ids = $ids | Select-Object -Unique }}; \
             Get-Process -Id $ids -ErrorAction SilentlyContinue | ForEach-Object {{ \"$($_.Id)|$($_.ProcessName)|$($_.PrivateMemorySize64)\" }}"
        );
        use std::os::windows::process::CommandExt;
        if let Ok(o) = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(0x0800_0000)
            .output()
        {
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                let parts: Vec<&str> = line.trim().split('|').collect();
                if let [pid, name, mem] = parts.as_slice() {
                    if let (Ok(pid), Ok(mem)) = (pid.parse(), mem.parse()) {
                        out.push((pid, name.to_string(), mem));
                    }
                }
            }
        }
        out
    }
    #[cfg(test)]
    mod torrent_registration_tests {
        use super::*;
        #[test]
        fn notification_identity_uses_crowdm_and_matches_the_installer() {
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            let path = format!(r"Software\CrowDMTests\{}", uuid::Uuid::new_v4());
            let (root, _) = hkcu.create_subkey(&path).unwrap();
            register_notification_at(&root, Path::new(r"C:\CrowDM\notification-icon.png")).unwrap();
            let key = root.open_subkey(format!(r"Software\Classes\AppUserModelId\{}", crate::notifications::APP_ID)).unwrap();
            assert_eq!(key.get_value::<String, _>("DisplayName").unwrap(), "CrowDM");
            assert!(key.get_value::<String, _>("IconUri").unwrap().ends_with("notification-icon.png"));
            let config: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
            assert_eq!(config["identifier"], crate::notifications::APP_ID);
            drop(key);
            drop(root);
            hkcu.delete_subkey_all(path).unwrap();
        }
        #[test]
        fn handlers_advertise_both_inputs_and_preserve_other_defaults() {
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            let test_path = format!(r"Software\CrowDMTests\{}", uuid::Uuid::new_v4());
            let (root, _) = hkcu.create_subkey(&test_path).unwrap();
            root.create_subkey(r"Software\Classes\magnet").unwrap().0.set_value("", &"Other.Client").unwrap();
            register_torrent_handlers_at(&root, Path::new(r"C:\Program Files\CrowDM\CrowDM.exe")).unwrap();
            let command: String = root.open_subkey(r"Software\Classes\CrowDM.Magnet\shell\open\command").unwrap().get_value("").unwrap();
            assert_eq!(command, r#""C:\Program Files\CrowDM\CrowDM.exe" "%1""#);
            let existing: String = root.open_subkey(r"Software\Classes\magnet").unwrap().get_value("").unwrap();
            assert_eq!(existing, "Other.Client");
            let torrent: String =
                root.open_subkey(r"Software\CrowDM\Capabilities\FileAssociations").unwrap().get_value(".torrent").unwrap();
            assert_eq!(torrent, "CrowDM.Torrent");
            let magnet: String = root.open_subkey(r"Software\CrowDM\Capabilities\URLAssociations").unwrap().get_value("magnet").unwrap();
            assert_eq!(magnet, "CrowDM.Magnet");
            drop(root);
            hkcu.delete_subkey_all(test_path).unwrap();
        }
    }
}

#[cfg(windows)]
pub use imp::*;

/// Sends the command line to an already running CrowDM. Returns true when
/// one received it (this process should then exit).
#[cfg(windows)]
pub fn forward_to_running_instance(args: &[String]) -> bool {
    use std::io::Write;
    let name = crowdm_ipc::pipe_name();
    for _ in 0..20 {
        match std::fs::OpenOptions::new().read(true).write(true).open(&name) {
            Ok(mut f) => {
                let msg = serde_json::json!({ "type": "activate", "args": args });
                let _ = writeln!(f, "{msg}");
                let _ = f.flush();
                return true;
            }
            Err(e) if e.raw_os_error() == Some(231) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(_) => return false,
        }
    }
    false
}
