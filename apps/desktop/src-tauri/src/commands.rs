//! Commands invoked by the web UI.

use std::path::PathBuf;

use crowdm_engine::{JobId, JobPatch, JobSnapshot, JobState, Settings};
use crowdm_ipc::ToBrowser;
use serde::Serialize;
use tauri::{Emitter, State};
use uuid::Uuid;

use crate::captures::{self, AcceptOptions};
use crate::platform;
use crate::state::{browser_settings, AppState, CaptureView};
use crate::windows;

type R<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub data_dir: String,
    pub portable: bool,
    pub queue_running: bool,
}

#[tauri::command]
pub fn app_info(state: State<AppState>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        data_dir: state.data_dir.to_string_lossy().into_owned(),
        portable: state.data_dir.starts_with(platform::exe_dir()),
        queue_running: state.engine.queue_running(),
    }
}

#[tauri::command]
pub fn list_jobs(state: State<AppState>) -> Vec<JobSnapshot> {
    state.engine.jobs()
}

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Settings {
    state.engine.settings()
}

#[tauri::command]
pub fn save_settings(state: State<AppState>, settings: Settings) -> R<Settings> {
    let before = state.engine.settings();
    let saved = state.engine.update_settings(settings).map_err(err)?;
    if before.start_with_windows != saved.start_with_windows {
        #[cfg(windows)]
        platform::set_autostart(saved.start_with_windows).map_err(err)?;
    }
    if before.interception != saved.interception {
        state.broadcast(&ToBrowser::Settings { settings: browser_settings(&saved) });
    }
    Ok(saved)
}

// ----- capture dialog ---------------------------------------------------

#[tauri::command]
pub fn capture_list(state: State<AppState>) -> Vec<CaptureView> {
    state.capture_views()
}

#[tauri::command]
pub fn capture_new(state: State<AppState>, url: Option<String>) -> Uuid {
    captures::add_manual(&state, url)
}

#[tauri::command]
pub fn capture_set_url(state: State<AppState>, key: Uuid, url: String) -> R<()> {
    captures::set_url(&state, key, url)
}

#[tauri::command]
pub fn capture_accept(state: State<AppState>, key: Uuid, options: AcceptOptions) -> R<JobId> {
    captures::accept(&state, key, options)
}

#[tauri::command]
pub fn capture_to_browser(state: State<AppState>, key: Uuid) {
    captures::to_browser(&state, key)
}

/// Called by the dialog after it has painted a capture; logs the latency
/// from receiving the capture to showing it (see docs/PERFORMANCE.md).
#[tauri::command]
pub fn capture_rendered(state: State<AppState>, key: Uuid) {
    let caps = state.captures.lock();
    if let Some(c) = caps.iter().find(|c| c.key == key) {
        tracing::info!(target: "latency", "capture dialog painted {} ms after the capture arrived", crowdm_engine::now_ms() - c.received_at);
    }
}

#[tauri::command]
pub fn capture_dismiss(state: State<AppState>, key: Uuid) {
    captures::dismiss(&state, key)
}

/// Where a file would be saved for a category (dialog preview).
#[tauri::command]
pub fn category_dir(state: State<AppState>, category: String) -> String {
    state.engine.settings().dir_for_category(&category).to_string_lossy().into_owned()
}

#[tauri::command]
pub fn show_main_window(app: tauri::AppHandle) {
    windows::show_main(&app);
}

// ----- job actions ------------------------------------------------------

#[tauri::command]
pub fn pause_jobs(state: State<AppState>, ids: Vec<JobId>) {
    for id in ids {
        let _ = state.engine.pause(id);
    }
}

#[tauri::command]
pub fn resume_jobs(state: State<AppState>, ids: Vec<JobId>) {
    for id in ids {
        let _ = state.engine.start(id);
    }
}

#[tauri::command]
pub async fn remove_jobs(state: State<'_, AppState>, ids: Vec<JobId>, delete_files: bool) -> R<()> {
    for id in ids {
        let snap = state.engine.job(id);
        state.engine.remove(id).await.map_err(err)?;
        if delete_files {
            if let Some(s) = snap.filter(|s| s.job.state == JobState::Completed) {
                #[cfg(windows)]
                platform::trash(&s.job.target_path())?;
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn restart_job(state: State<'_, AppState>, id: JobId) -> R<()> {
    state.engine.restart(id).await.map_err(err)
}

#[tauri::command]
pub fn update_job(state: State<AppState>, id: JobId, patch: JobPatch) -> R<()> {
    state.engine.update_job(id, patch).map_err(err)
}

#[tauri::command]
pub fn set_queued(state: State<AppState>, ids: Vec<JobId>, queued: bool) -> R<()> {
    for id in ids {
        state.engine.update_job(id, JobPatch { queued: Some(queued), ..Default::default() }).map_err(err)?;
    }
    Ok(())
}

#[tauri::command]
pub fn reorder_job(state: State<AppState>, id: JobId, before: Option<JobId>) -> R<()> {
    state.engine.reorder(id, before).map_err(err)
}

#[tauri::command]
pub fn open_job_file(state: State<AppState>, id: JobId) -> R<()> {
    let snap = state.engine.job(id).ok_or("Download not found.")?;
    #[cfg(windows)]
    platform::open_path(&snap.job.target_path())?;
    Ok(())
}

#[tauri::command]
pub fn reveal_job(state: State<AppState>, id: JobId) -> R<()> {
    let snap = state.engine.job(id).ok_or("Download not found.")?;
    let path = if snap.job.state == JobState::Completed { snap.job.target_path() } else { snap.job.part_path() };
    #[cfg(windows)]
    platform::reveal_path(&path)?;
    Ok(())
}

#[tauri::command]
pub fn open_folder(path: String) -> R<()> {
    #[cfg(windows)]
    platform::reveal_path(&PathBuf::from(path))?;
    Ok(())
}

#[tauri::command]
pub fn pause_all(state: State<AppState>) {
    state.engine.pause_all();
}

#[tauri::command]
pub fn resume_all(state: State<AppState>) {
    state.engine.resume_all();
}

#[tauri::command]
pub fn set_queue_running(state: State<AppState>, running: bool) {
    if running {
        state.engine.start_queue();
    } else {
        state.engine.stop_queue();
    }
}

// ----- browser integration ---------------------------------------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConnectedBrowser {
    pub browser: String,
    pub extension_version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatus {
    #[cfg(windows)]
    pub integration: platform::IntegrationStatus,
    pub connected: Vec<ConnectedBrowser>,
}

fn connected(state: &AppState) -> Vec<ConnectedBrowser> {
    state
        .conns
        .lock()
        .values()
        .filter_map(|c| Some(ConnectedBrowser { browser: c.browser.clone()?, extension_version: c.extension_version.clone().unwrap_or_default() }))
        .collect()
}

pub fn emit_browser_status(state: &AppState) {
    let _ = state.app.emit("browsers-changed", connected(state));
}

#[tauri::command]
pub fn browser_status(state: State<AppState>) -> BrowserStatus {
    BrowserStatus {
        #[cfg(windows)]
        integration: platform::integration_status(),
        connected: connected(&state),
    }
}

#[tauri::command]
pub fn register_browsers(state: State<AppState>) -> R<BrowserStatus> {
    #[cfg(windows)]
    platform::register_browsers(&state.data_dir).map_err(err)?;
    Ok(browser_status(state))
}

#[tauri::command]
pub fn open_extensions_folder() -> R<()> {
    let dir = platform::extensions_dir().ok_or("The extension files were not found next to CrowDM.")?;
    #[cfg(windows)]
    platform::reveal_path(&dir.join("chromium"))?;
    Ok(())
}

#[tauri::command]
pub fn open_data_folder(state: State<AppState>) -> R<()> {
    #[cfg(windows)]
    platform::reveal_path(&state.data_dir.join("logs"))?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessMemory {
    pub pid: u32,
    pub name: String,
    pub private_bytes: u64,
}

/// Memory of CrowDM and its WebView2 processes (Settings > About).
#[tauri::command]
pub async fn memory_usage() -> Vec<ProcessMemory> {
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(|| {
            platform::memory_report().into_iter().filter(|(_, name, _)| !matches!(name.as_str(), "powershell" | "conhost")).map(|(pid, name, private_bytes)| ProcessMemory { pid, name, private_bytes }).collect()
        })
        .await
        .unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        vec![]
    }
}
