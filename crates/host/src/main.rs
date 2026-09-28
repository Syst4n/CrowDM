//! CrowDM native messaging host.
//!
//! Browsers start this program when the CrowDM extension connects. It relays
//! messages between the extension (stdin/stdout, 32-bit length-prefixed JSON)
//! and the running CrowDM app (a per-user named pipe carrying JSON lines),
//! starting the app in the background first if necessary.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

/// Largest message a browser may send us (Chrome's limit is 64 MiB).
const MAX_IN: usize = 64 * 1024 * 1024;
/// Largest message we may send to the browser (1 MiB).
const MAX_OUT: usize = 1024 * 1024;

fn log(msg: &str) {
    let Some(dir) = std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("CrowDM").join("logs")) else { return };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("host.log");
    if std::fs::metadata(&path).map(|m| m.len() > 512 * 1024).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = writeln!(f, "[{secs}] [{}] {msg}", std::process::id());
    }
}

async fn read_frame<R: AsyncReadExt + Unpin>(r: &mut R) -> std::io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let n = u32::from_ne_bytes(len) as usize;
    if n > MAX_IN {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "message too large"));
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).await?;
    Ok(Some(buf))
}

async fn write_frame<W: AsyncWriteExt + Unpin>(w: &mut W, v: &Value) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(v)?;
    if bytes.len() > MAX_OUT {
        log("dropping oversized message to browser");
        return Ok(());
    }
    w.write_all(&(bytes.len() as u32).to_ne_bytes()).await?;
    w.write_all(&bytes).await?;
    w.flush().await
}

fn error_message(code: &str, message: &str) -> Value {
    serde_json::json!({ "type": "error", "replyTo": null, "code": code, "message": message })
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::time::Instant;
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

    pub fn app_path() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let dir = exe.parent()?;
        for name in ["CrowDM.exe", "crowdm.exe", "crowdm-desktop.exe"] {
            let p = dir.join(name);
            if p.exists() {
                return Some(p);
            }
        }
        // Recorded by the app when it registers the browser integration.
        let key = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).open_subkey(r"Software\CrowDM").ok()?;
        let p: String = key.get_value("AppPath").ok()?;
        let p = PathBuf::from(p);
        p.exists().then_some(p)
    }

    pub fn launch_app() -> bool {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        let Some(app) = app_path() else {
            log("CrowDM.exe not found next to the host or in the registry");
            return false;
        };
        log(&format!("starting {}", app.display()));
        let base = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
        // Browsers run hosts inside a job object that is closed with the
        // host; the app must break away from it to keep running.
        let spawn = |flags| std::process::Command::new(&app).arg("--background").creation_flags(flags).spawn();
        match spawn(base | CREATE_BREAKAWAY_FROM_JOB).or_else(|_| spawn(base)) {
            Ok(_) => true,
            Err(e) => {
                log(&format!("failed to start app: {e}"));
                false
            }
        }
    }

    pub async fn connect(deadline: Duration) -> Option<NamedPipeClient> {
        let name = crowdm_ipc::pipe_name();
        let start = Instant::now();
        let mut launched = false;
        loop {
            match ClientOptions::new().open(&name) {
                Ok(c) => return Some(c),
                Err(e) if e.raw_os_error() == Some(231) => {} // ERROR_PIPE_BUSY
                Err(_) => {
                    if !launched {
                        launched = true;
                        if !launch_app() {
                            return None;
                        }
                    }
                }
            }
            if start.elapsed() > deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }

    /// Process id of the app at the other end of the pipe.
    pub fn server_pid(pipe: &NamedPipeClient) -> u32 {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
        let mut pid = 0u32;
        unsafe {
            GetNamedPipeServerProcessId(pipe.as_raw_handle() as _, &mut pid);
        }
        pid
    }

    /// Lets the app bring its dialog to the front. Windows only lets the
    /// foreground process (the browser) or processes it started grant this.
    pub fn allow_foreground(pid: u32) {
        use windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;
        if pid != 0 {
            unsafe {
                AllowSetForegroundWindow(pid);
            }
        }
    }
}

#[cfg(windows)]
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    log(&format!("started for {:?}", args.get(1)));
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();

    let Some(pipe) = win::connect(Duration::from_secs(20)).await else {
        let _ = write_frame(&mut stdout, &error_message("appUnavailable", "CrowDM is not running and could not be started.")).await;
        // Give the browser a moment to read the error before exiting.
        tokio::time::sleep(Duration::from_millis(200)).await;
        return;
    };
    let app_pid = win::server_pid(&pipe);
    log(&format!("connected to app (pid {app_pid})"));
    let (pipe_read, mut pipe_write) = tokio::io::split(pipe);
    let mut lines = BufReader::new(pipe_read).lines();

    // Reading stdin is not cancellation-safe inside select!, so it runs in
    // its own task and hands frames over a channel.
    let (tx, mut from_browser) = tokio::sync::mpsc::channel::<Value>(64);
    let reader = tokio::spawn(async move {
        loop {
            match read_frame(&mut stdin).await {
                Ok(Some(buf)) => match serde_json::from_slice::<Value>(&buf) {
                    Ok(v) => {
                        if tx.send(v).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => log(&format!("invalid JSON from browser: {e}")),
                },
                Ok(None) => break,
                Err(e) => {
                    log(&format!("stdin error: {e}"));
                    break;
                }
            }
        }
    });

    loop {
        tokio::select! {
            msg = from_browser.recv() => {
                let Some(v) = msg else { break };
                if v.get("type").and_then(Value::as_str) == Some("capture") {
                    win::allow_foreground(app_pid);
                }
                let mut line = serde_json::to_string(&v).unwrap_or_default();
                line.push('\n');
                if pipe_write.write_all(line.as_bytes()).await.is_err() {
                    let _ = write_frame(&mut stdout, &error_message("appClosed", "CrowDM closed the connection.")).await;
                    break;
                }
            }
            line = lines.next_line() => {
                match line {
                    Ok(Some(l)) => {
                        if l.trim().is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<Value>(&l) {
                            Ok(v) => {
                                if write_frame(&mut stdout, &v).await.is_err() {
                                    break;
                                }
                            }
                            Err(e) => log(&format!("invalid JSON from app: {e}")),
                        }
                    }
                    _ => {
                        let _ = write_frame(&mut stdout, &error_message("appClosed", "CrowDM closed the connection.")).await;
                        break;
                    }
                }
            }
        }
    }
    reader.abort();
    log("exiting");
    std::process::exit(0);
}

#[cfg(not(windows))]
fn main() {
    eprintln!("crowdm-host currently supports Windows only");
    std::process::exit(1);
}
