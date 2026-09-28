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
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
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
        let key = hkcu.open_subkey_with_flags(RUN_KEY, KEY_READ | KEY_WRITE)?;
        if enabled {
            let exe = std::env::current_exe()?;
            key.set_value("CrowDM", &format!("\"{}\" --background", exe.display()))?;
        } else {
            let _ = key.delete_value("CrowDM");
        }
        Ok(())
    }

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        s.encode_wide().chain(Some(0)).collect()
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
        let r = unsafe { ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
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
