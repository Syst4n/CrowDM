//! Window creation. The capture dialog can be kept pre-loaded (hidden) so it
//! appears instantly; the main window is destroyed when closed to the tray
//! to give its memory back.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const MAIN: &str = "main";
pub const CAPTURE: &str = "capture";

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
        .visible(false) // shown by the page once it has painted, avoiding a white flash
        .build();
    if let Err(e) = built {
        tracing::error!("could not create main window: {e}");
    }
}

pub fn ensure_capture(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    if let Some(w) = app.get_webview_window(CAPTURE) {
        return Some(w);
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
}

pub fn show_capture(app: &AppHandle) {
    if let Some(w) = ensure_capture(app) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

pub fn hide_capture(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(CAPTURE) {
        let _ = w.hide();
        let prewarm = app.try_state::<crate::state::AppState>().map(|s| s.engine.settings().prewarm_dialog).unwrap_or(true);
        if !prewarm {
            let _ = w.destroy();
        }
    }
}
