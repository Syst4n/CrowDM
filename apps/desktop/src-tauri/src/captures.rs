//! Browser hand-off: receiving captured downloads, showing the confirmation
//! dialog, and telling the extension what to do with its paused download.

use crowdm_engine::{Checksum, JobId, JobSource, NewJob, ProbeRequest, SourceKind};
use crowdm_ipc::{Capture, CaptureKind, Decision, ToBrowser};
use serde::Deserialize;
use tauri::Emitter;
use uuid::Uuid;

use crate::state::{AppState, PendingCapture};
use crate::windows;

pub const EVENT_CAPTURES: &str = "captures-changed";

/// A dialog opened before changing the root must not keep suggesting the old
/// auto-derived destination. Explicit folder edits are preserved by the UI.
pub fn refresh_folders(state: &AppState, settings: &crowdm_engine::Settings) {
    for capture in state.captures.lock().iter_mut() {
        if let Some(info) = &mut capture.probe {
            info.category = settings.category_for(&info.file_name);
            info.dir = settings.dir_for_category(&info.category);
            info.exists = info.dir.join(&info.file_name).exists();
        }
    }
    changed(state);
}

fn now_ms() -> i64 {
    crowdm_engine::now_ms()
}

/// Handles a capture message from an extension.
pub fn on_capture(state: &AppState, conn: u64, request_id: String, browser: String, mut capture: Capture) {
    if capture.kind == CaptureKind::Menu && crowdm_engine::media::is_media_url(&capture.url) {
        capture.kind = CaptureKind::Video;
    }
    // The same browser download offered twice (a retry after a reconnect):
    // only the first counts.
    {
        let caps = state.captures.lock();
        if let Some(id) = capture.browser_download_id {
            if caps.iter().any(|c| c.conn == Some(conn) && c.capture.browser_download_id == Some(id)) {
                state.send(conn, &ToBrowser::Received { reply_to: request_id });
                return;
            }
        }
        // Two browser downloads of the same URL within a few seconds are a
        // double click: keep the first, cancel the duplicate in the browser.
        let dup = caps.iter().any(|c| c.capture.url == capture.url && now_ms() - c.received_at < 5_000 && c.capture.kind == capture.kind);
        if dup && capture.kind == CaptureKind::Intercepted {
            drop(caps);
            state.send(conn, &ToBrowser::Decision { reply_to: request_id, decision: Decision::Cancel });
            return;
        }
    }

    let settings = state.engine.settings();
    if !settings.confirm_captured {
        // Silent mode: start immediately without the dialog.
        let mut pending = PendingCapture {
            key: Uuid::new_v4(),
            conn: Some(conn),
            request_id: Some(request_id.clone()),
            browser: Some(browser),
            capture,
            probe: None,
            probe_error: None,
            probing: false,
            received_at: now_ms(),
        };
        state.send(conn, &ToBrowser::Received { reply_to: request_id.clone() });
        let state = state.clone();
        tauri::async_runtime::spawn(async move {
            let req = probe_request(&pending.capture);
            let result = if pending.capture.kind == CaptureKind::Video {
                state.engine.probe_media(req).await
            } else {
                state.engine.probe(req).await
            };
            let decision = match result {
                Ok(info) if !(state.engine.settings().interception.skip_web_pages && info.looks_like_html) => {
                    pending.probe = Some(info);
                    match state.engine.add(new_job_for(&pending, AcceptOptions { start: true, ..Default::default() })) {
                        Ok(_) => Decision::TakeOver,
                        Err(_) => Decision::ResumeInBrowser,
                    }
                }
                Ok(info) => {
                    if let Some(id) = info.probe_id {
                        state.engine.discard_probe(id);
                    }
                    Decision::ResumeInBrowser
                }
                Err(_) => Decision::ResumeInBrowser,
            };
            state.send(conn, &ToBrowser::Decision { reply_to: request_id, decision });
        });
        return;
    }

    let key = Uuid::new_v4();
    state.captures.lock().push(PendingCapture {
        key,
        conn: Some(conn),
        request_id: Some(request_id.clone()),
        browser: Some(browser),
        capture,
        probe: None,
        probe_error: None,
        probing: true,
        received_at: now_ms(),
    });
    // Acknowledge first so the extension knows a person is deciding.
    state.send(conn, &ToBrowser::Received { reply_to: request_id });
    windows::show_capture(&state.app);
    changed(state);
    spawn_probe(state.clone(), key);
}

/// Adds a URL typed or pasted in the app (or passed on the command line).
pub fn add_manual(state: &AppState, url: Option<String>) -> Uuid {
    let key = Uuid::new_v4();
    let has_url = url.as_deref().is_some_and(|u| !u.trim().is_empty());
    state.captures.lock().push(PendingCapture {
        key,
        conn: None,
        request_id: None,
        browser: None,
        capture: Capture {
            media_format: None,
            kind: if url.as_deref().is_some_and(crowdm_engine::media::is_media_url) { CaptureKind::Video } else { CaptureKind::Manual },
            browser_download_id: None,
            url: url.unwrap_or_default().trim().to_string(),
            final_url: None,
            referrer: None,
            page_url: None,
            page_title: None,
            file_name: None,
            mime: None,
            size: None,
            incognito: false,
            headers: vec![],
        },
        probe: None,
        probe_error: None,
        probing: has_url,
        received_at: now_ms(),
    });
    windows::show_capture(&state.app);
    changed(state);
    if has_url {
        spawn_probe(state.clone(), key);
    }
    key
}

pub fn set_url(state: &AppState, key: Uuid, url: String) -> Result<(), String> {
    {
        let mut caps = state.captures.lock();
        let c = caps.iter_mut().find(|c| c.key == key).ok_or("This download is no longer waiting.")?;
        if let Some(p) = c.probe.take().and_then(|p| p.probe_id) {
            state.engine.discard_probe(p);
        }
        c.capture.url = url.trim().to_string();
        c.capture.final_url = None;
        c.capture.media_format = None;
        if c.capture.kind == CaptureKind::Manual && crowdm_engine::media::is_media_url(&c.capture.url) {
            c.capture.kind = CaptureKind::Video;
        }
        c.probe_error = None;
        c.probing = true;
    }
    changed(state);
    spawn_probe(state.clone(), key);
    Ok(())
}

fn probe_request(cap: &Capture) -> ProbeRequest {
    ProbeRequest {
        media_format: cap.media_format.clone(),
        url: cap.final_url.clone().filter(|u| !u.is_empty()).unwrap_or_else(|| cap.url.clone()),
        referrer: cap.referrer.clone().or_else(|| cap.page_url.clone()),
        headers: cap.headers.iter().map(|h| (h.name.clone(), h.value.clone())).collect(),
        file_name_hint: cap.file_name.clone(),
        mime_hint: cap.mime.clone(),
    }
}

pub fn set_media_format(state: &AppState, key: Uuid, format: String) -> Result<(), String> {
    {
        let mut caps = state.captures.lock();
        let c = caps.iter_mut().find(|c| c.key == key).ok_or("This download is no longer waiting.")?;
        if c.capture.kind != CaptureKind::Video || !c.probe.as_ref().is_some_and(|p| p.media_options.iter().any(|f| f.format == format)) {
            return Err("Choose one of the available video qualities.".into());
        }
        if let Some(info) = &mut c.probe {
            crowdm_engine::media::select_output(info, &format, &state.engine.settings());
        }
        c.capture.media_format = Some(format);
    }
    changed(state);
    Ok(())
}

fn spawn_probe(state: AppState, key: Uuid) {
    tauri::async_runtime::spawn(async move {
        let (req, video) = {
            let caps = state.captures.lock();
            let Some(c) = caps.iter().find(|c| c.key == key) else { return };
            (probe_request(&c.capture), c.capture.kind == CaptureKind::Video)
        };
        let requested_url = req.url.clone();
        let result = if video { state.engine.probe_media(req).await } else { state.engine.probe(req).await };
        if !state.captures.lock().iter().any(|c| c.key == key && probe_request(&c.capture).url == requested_url) {
            if let Ok(info) = result {
                if let Some(id) = info.probe_id {
                    state.engine.discard_probe(id);
                }
            }
            return;
        }
        // The browser often learns the real file name only from the server
        // (Content-Disposition), so exclusion rules are checked again here.
        if let Ok(info) = &result {
            let excluded = {
                let caps = state.captures.lock();
                caps.iter().find(|c| c.key == key).is_some_and(|c| {
                    c.capture.kind == CaptureKind::Intercepted
                        && (is_excluded(&state.engine.settings(), &info.file_name, info.size)
                            || state.engine.settings().interception.skip_web_pages && info.looks_like_html)
                })
            };
            if excluded {
                tracing::info!("{} matches an exclusion rule; leaving it to the browser", info.file_name);
                if let Some(id) = info.probe_id {
                    state.engine.discard_probe(id);
                }
                to_browser(&state, key);
                return;
            }
        }
        {
            let mut caps = state.captures.lock();
            match caps.iter_mut().find(|c| c.key == key) {
                Some(c) => {
                    c.probing = false;
                    match result {
                        Ok(info) => {
                            c.probe = Some(info);
                            c.probe_error = None;
                        }
                        Err(e) => c.probe_error = Some(e.to_string()),
                    }
                }
                None => {
                    // Dialog already closed: release the connection.
                    if let Ok(info) = result {
                        if let Some(id) = info.probe_id {
                            state.engine.discard_probe(id);
                        }
                    }
                }
            }
        }
        changed(&state);
    });
}

fn is_excluded(settings: &crowdm_engine::Settings, file_name: &str, size: Option<u64>) -> bool {
    let i = &settings.interception;
    let ext = std::path::Path::new(file_name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    (!ext.is_empty() && i.excluded_extensions.iter().any(|x| x.eq_ignore_ascii_case(&ext)))
        || (i.min_size > 0 && size.is_some_and(|s| s < i.min_size))
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AcceptOptions {
    pub file_name: Option<String>,
    pub dir: Option<String>,
    pub category: Option<String>,
    /// Start now (true) or add to the queue (false).
    pub start: bool,
    pub max_connections: Option<u8>,
    pub speed_limit: Option<u64>,
    pub checksum: Option<String>,
}

fn new_job_for(c: &PendingCapture, o: AcceptOptions) -> NewJob {
    let cap = &c.capture;
    let checksum = o
        .checksum
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|h| crowdm_engine::checksum::guess_algo(h).map(|algo| Checksum { algo, expected: h.to_ascii_lowercase() }));
    NewJob {
        media_download: cap.kind == CaptureKind::Video,
        media_format: cap.media_format.clone(),
        url: cap.final_url.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| cap.url.clone()),
        referrer: cap.referrer.clone().or_else(|| cap.page_url.clone()),
        headers: cap.headers.iter().map(|h| (h.name.clone(), h.value.clone())).collect(),
        file_name: o
            .file_name
            .filter(|n| !n.trim().is_empty())
            .or_else(|| c.probe.as_ref().map(|p| p.file_name.clone()))
            .or(cap.file_name.clone()),
        dir: o.dir.filter(|d| !d.trim().is_empty()).map(Into::into),
        category: o.category,
        start: o.start,
        max_connections: o.max_connections,
        speed_limit: o.speed_limit.filter(|v| *v > 0),
        checksum,
        source: JobSource {
            kind: match cap.kind {
                CaptureKind::Intercepted => SourceKind::BrowserCapture,
                CaptureKind::Menu => SourceKind::BrowserMenu,
                CaptureKind::Video => SourceKind::Video,
                CaptureKind::Manual => SourceKind::Manual,
            },
            page_url: cap.page_url.clone(),
            browser: c.browser.clone(),
        },
        probe_id: c.probe.as_ref().and_then(|p| p.probe_id),
        size_hint: c.probe.as_ref().and_then(|p| p.size).or(cap.size),
        mime_hint: c.probe.as_ref().and_then(|p| p.mime.clone()).or(cap.mime.clone()),
        overwrite: false,
    }
}

fn take(state: &AppState, key: Uuid) -> Option<PendingCapture> {
    let mut caps = state.captures.lock();
    let idx = caps.iter().position(|c| c.key == key)?;
    Some(caps.remove(idx))
}

fn decide(state: &AppState, c: &PendingCapture, decision: Decision) {
    if let (Some(conn), Some(req)) = (c.conn, c.request_id.clone()) {
        state.send(conn, &ToBrowser::Decision { reply_to: req, decision });
    }
}

pub fn accept(state: &AppState, key: Uuid, options: AcceptOptions) -> Result<JobId, String> {
    let c = {
        let caps = state.captures.lock();
        caps.iter().find(|c| c.key == key).cloned().ok_or("This download is no longer waiting.")?
    };
    if c.capture.url.trim().is_empty() {
        return Err("Enter a download address first.".into());
    }
    if c.probing {
        return Err("Please wait while CrowDM checks this download.".into());
    }
    if let Some(error) = &c.probe_error {
        return Err(error.clone());
    }
    let probe = c.probe.as_ref().ok_or("Check the download address before starting.")?;
    if probe.looks_like_html && state.engine.settings().interception.skip_web_pages {
        return Err("This address returned a web page. Use the video button or download it in the browser.".into());
    }
    let id = state.engine.add(new_job_for(&c, options)).map_err(|e| e.to_string())?;
    take(state, key);
    decide(state, &c, Decision::TakeOver);
    after_decision(state);
    Ok(id)
}

pub fn to_browser(state: &AppState, key: Uuid) {
    if let Some(c) = take(state, key) {
        if let Some(id) = c.probe.as_ref().and_then(|p| p.probe_id) {
            state.engine.discard_probe(id);
        }
        decide(state, &c, Decision::ResumeInBrowser);
    }
    after_decision(state);
}

pub fn dismiss(state: &AppState, key: Uuid) {
    if let Some(c) = take(state, key) {
        if let Some(id) = c.probe.as_ref().and_then(|p| p.probe_id) {
            state.engine.discard_probe(id);
        }
        decide(state, &c, Decision::Cancel);
    }
    after_decision(state);
}

/// The browser gave up on a capture (connection closed, or the download
/// disappeared). Its extension resumes such downloads itself.
pub fn forget_conn(state: &AppState, conn: u64) {
    let removed: Vec<PendingCapture> = {
        let mut caps = state.captures.lock();
        let (gone, keep): (Vec<_>, Vec<_>) =
            caps.drain(..).partition(|c| c.conn == Some(conn) && c.capture.kind == CaptureKind::Intercepted);
        *caps = keep;
        gone
    };
    for c in &removed {
        if let Some(id) = c.probe.as_ref().and_then(|p| p.probe_id) {
            state.engine.discard_probe(id);
        }
    }
    if !removed.is_empty() {
        after_decision(state);
    }
}

pub fn capture_gone(state: &AppState, conn: u64, request_id: &str) {
    let key = state.captures.lock().iter().find(|c| c.conn == Some(conn) && c.request_id.as_deref() == Some(request_id)).map(|c| c.key);
    if let Some(key) = key {
        if let Some(c) = take(state, key) {
            if let Some(id) = c.probe.as_ref().and_then(|p| p.probe_id) {
                state.engine.discard_probe(id);
            }
        }
        after_decision(state);
    }
}

fn after_decision(state: &AppState) {
    if state.captures.lock().is_empty() {
        windows::hide_capture(&state.app);
    }
    changed(state);
}

pub fn changed(state: &AppState) {
    let _ = state.app.emit(EVENT_CAPTURES, state.capture_views());
}
