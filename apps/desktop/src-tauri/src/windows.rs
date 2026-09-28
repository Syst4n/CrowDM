//! Window creation.
//!
//! The capture dialog is the latency-critical window. WebView2 needs about
//! half a second to create a window from scratch but only ~30 ms to show one
//! that is already loaded, while a loaded window costs ~110-150 MB. So the
//! dialog is kept loaded for a while after each use (downloads come in
//! bursts) and released when idle, unless the user asks to keep it loaded
//! permanently. The main window is destroyed when closed to the tray.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const MAIN: &str = "main";
pub const CAPTURE: &str = "capture";

/// How long an unused capture dialog stays loaded.
const KEEP_DIALOG: Duration = Duration::from_secs(10 * 60);

static HIDE_GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title("CrowDM")
        .inner_size(1180.0, 720.0)
        .min_inner_size(820.0, 520.0)
        .center()
        .visible(false) // the page shows it after its first paint, avoiding a blank flash
        .build();
    if let Err(e) = built {
        tracing::error!("could not create main window: {e}");
    }
}

/// Returns the capture window, creating it (hidden) if needed, and whether
/// it was just created.
fn ensure_capture_inner(app: &AppHandle) -> Option<(tauri::WebviewWindow, bool)> {
    if let Some(w) = app.get_webview_window(CAPTURE) {
        return Some((w, false));
    }
    WebviewWindowBuilder::new(app, CAPTURE, WebviewUrl::App("index.html#capture".into()))
        .title("New download")
        .inner_size(600.0, 480.0)
        .min_inner_size(520.0, 420.0)
        .resizable(true)
        .maximizable(false)
        .minimizable(true)
        .always_on_top(true)
        .center()
        .visible(false)
        .build()
        .map_err(|e| tracing::error!("could not create capture window: {e}"))
        .ok()
        .map(|w| (w, true))
}

/// Pre-loads the capture dialog without showing it.
pub fn ensure_capture(app: &AppHandle) {
    ensure_capture_inner(app);
}

/// Shows the capture dialog. A freshly created dialog shows itself once its
/// page has painted (see CaptureDialog.tsx), so there is no blank window.
pub fn show_capture(app: &AppHandle) {
    HIDE_GENERATION.fetch_add(1, Ordering::SeqCst);
    let app = app.clone();
    // Window creation waits on the UI thread; keep it off the pipe handler
    // so the browser gets its acknowledgement immediately.
    tauri::async_runtime::spawn_blocking(move || {
        if let Some((w, created)) = ensure_capture_inner(&app) {
            if !created {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
    });
}

pub fn hide_capture(app: &AppHandle) {
    let Some(w) = app.get_webview_window(CAPTURE) else { return };
    let _ = w.hide();
    let keep_forever = app.try_state::<crate::state::AppState>().map(|s| s.engine.settings().prewarm_dialog).unwrap_or(false);
    if keep_forever {
        return;
    }
    let generation = HIDE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(KEEP_DIALOG).await;
        if HIDE_GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        if let Some(w) = app.get_webview_window(CAPTURE) {
            if !w.is_visible().unwrap_or(true) {
                tracing::info!("releasing the idle capture dialog");
                let _ = w.destroy();
            }
        }
    });
}
