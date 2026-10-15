//! Shared application state.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use crowdm_engine::{Engine, ProbeInfo, Settings};
use crowdm_ipc::{BrowserSettings, Capture, CaptureKind, ToBrowser};
use parking_lot::Mutex;
use serde::Serialize;
use tauri::AppHandle;
use tokio::sync::mpsc;
use uuid::Uuid;

/// A connected extension (through one native host process).
pub struct Conn {
    pub tx: mpsc::UnboundedSender<String>,
    pub browser: Option<String>,
    pub extension_version: Option<String>,
}

/// A download waiting for the user's decision in the capture dialog.
#[derive(Clone)]
pub struct PendingCapture {
    pub key: Uuid,
    /// Connection and request that must receive the decision.
    pub conn: Option<u64>,
    pub request_id: Option<String>,
    pub browser: Option<String>,
    pub capture: Capture,
    pub probe: Option<ProbeInfo>,
    pub probe_error: Option<String>,
    pub probing: bool,
    pub received_at: i64,
}

/// What the capture dialog renders.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureView {
    pub media_format: Option<String>,
    pub key: Uuid,
    pub kind: CaptureKind,
    pub browser: Option<String>,
    pub url: String,
    pub final_url: Option<String>,
    pub page_url: Option<String>,
    pub page_title: Option<String>,
    pub file_name: Option<String>,
    pub size: Option<u64>,
    pub mime: Option<String>,
    pub probe: Option<ProbeInfo>,
    pub probe_error: Option<String>,
    pub probing: bool,
    /// The browser should keep this download (reason shown in the dialog).
    pub browser_recommended: Option<String>,
    /// True when an intercepted browser download is waiting on this dialog.
    pub browser_waiting: bool,
}

impl PendingCapture {
    pub fn view(&self, conn_alive: bool) -> CaptureView {
        let c = &self.capture;
        let mut recommend = None;
        if let Some(p) = &self.probe {
            if let (Some(a), Some(b)) = (c.size, p.size) {
                if a != b {
                    recommend = Some("The server sent CrowDM a different file than the browser received.".to_string());
                }
            }
            let browser_html = c.mime.as_deref().is_some_and(|m| m.contains("html"));
            if p.looks_like_html && !browser_html {
                recommend = Some(
                    "The server answered CrowDM with a web page instead of the file. This link probably only works inside the browser."
                        .to_string(),
                );
            }
        }
        if self.probe_error.is_some() && c.kind == CaptureKind::Intercepted {
            recommend = Some("CrowDM could not open this link directly. The browser can still download it.".to_string());
        }
        CaptureView {
            media_format: c.media_format.clone(),
            key: self.key,
            kind: c.kind,
            browser: self.browser.clone(),
            url: c.url.clone(),
            final_url: c.final_url.clone(),
            page_url: c.page_url.clone(),
            page_title: c.page_title.clone(),
            file_name: c.file_name.clone(),
            size: c.size,
            mime: c.mime.clone(),
            probe: self.probe.clone(),
            probe_error: self.probe_error.clone(),
            probing: self.probing,
            browser_recommended: recommend,
            browser_waiting: c.kind == CaptureKind::Intercepted && conn_alive,
        }
    }
}

pub struct Inner {
    pub app: AppHandle,
    pub engine: Engine,
    pub data_dir: PathBuf,
    pub captures: Mutex<Vec<PendingCapture>>,
    pub conns: Mutex<HashMap<u64, Conn>>,
    pub next_conn: AtomicU64,
    pub quitting: AtomicBool,
}

#[derive(Clone)]
pub struct AppState(pub Arc<Inner>);

impl std::ops::Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    pub fn new(app: AppHandle, engine: Engine, data_dir: PathBuf) -> Self {
        AppState(Arc::new(Inner {
            app,
            engine,
            data_dir,
            captures: Mutex::new(Vec::new()),
            conns: Mutex::new(HashMap::new()),
            next_conn: AtomicU64::new(1),
            quitting: AtomicBool::new(false),
        }))
    }

    pub fn conn_alive(&self, id: Option<u64>) -> bool {
        id.is_some_and(|id| self.conns.lock().contains_key(&id))
    }

    pub fn send(&self, conn: u64, msg: &ToBrowser) {
        if let Some(c) = self.conns.lock().get(&conn) {
            if let Ok(s) = serde_json::to_string(msg) {
                let _ = c.tx.send(s);
            }
        }
    }

    pub fn broadcast(&self, msg: &ToBrowser) {
        let Ok(s) = serde_json::to_string(msg) else { return };
        for c in self.conns.lock().values() {
            let _ = c.tx.send(s.clone());
        }
    }

    pub fn capture_views(&self) -> Vec<CaptureView> {
        let caps = self.captures.lock().clone();
        caps.iter().map(|c| c.view(self.conn_alive(c.conn))).collect()
    }

    pub fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::Relaxed)
    }
}

pub fn browser_settings(s: &Settings) -> BrowserSettings {
    let i = &s.interception;
    BrowserSettings {
        appearance: Some(crowdm_ipc::BrowserAppearance {
            mode: serde_json::to_value(s.effective_appearance().mode).unwrap().as_str().unwrap().into(),
            palette: serde_json::to_value(s.effective_appearance().palette).unwrap().as_str().unwrap().into(),
        }),
        animations: s.animations,
        respect_reduced_motion: s.respect_reduced_motion,
        theme: serde_json::to_value(s.theme).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "system".into()),
        enabled: i.enabled,
        excluded_sites: i.excluded_sites.clone(),
        excluded_extensions: i.excluded_extensions.clone(),
        min_size: i.min_size,
        include_private: i.include_private,
        video_button: i.video_button,
        skip_web_pages: i.skip_web_pages,
    }
}
