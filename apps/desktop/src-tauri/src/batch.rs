//! Persistent Link Grabber inbox. Checking never starts downloads or opens popups.
use crate::state::AppState;
use crowdm_engine::{links, JobState, NewJob, ProbeInfo, ProbeRequest};
use futures_util::{stream, StreamExt};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::State;
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    id: Uuid,
    #[serde(default)]
    original_url: String,
    url: String,
    file_name: String,
    package: String,
    status: String,
    message: Option<String>,
    probe: Option<ProbeInfo>,
    referrer: Option<String>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Inbox {
    links: Vec<Link>,
    checking: bool,
}
pub struct BatchState {
    pub open_requested: AtomicBool,
    notified: Mutex<HashSet<String>>,
    inbox: Mutex<Inbox>,
    busy: AtomicBool,
    cancel: AtomicBool,
    path: PathBuf,
}
impl BatchState {
    pub fn load(dir: &Path) -> Self {
        let path = dir.join("link-grabber.json");
        let mut inbox: Inbox = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        inbox.checking = false;
        for l in &mut inbox.links {
            if l.status == "checking" {
                l.status = "unchecked".into();
            }
        }
        Self {
            open_requested: AtomicBool::new(false),
            notified: Mutex::new(HashSet::new()),
            inbox: Mutex::new(inbox),
            busy: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            path,
        }
    }
    fn save(&self, inbox: &Inbox) -> Result<(), String> {
        let b = serde_json::to_vec(inbox).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, b).and_then(|_| std::fs::rename(tmp, &self.path)).map_err(|_| "Could not save the Link Grabber inbox.".into())
    }
}
pub fn open(state: &AppState) {
    state.batch.open_requested.store(true, Ordering::SeqCst);
    crate::windows::show_main(&state.app);
    use tauri::Emitter;
    let _ = state.app.emit_to(crate::windows::MAIN, "open-link-grabber", ());
}
#[tauri::command]
pub fn batch_take_focus(state: State<AppState>) -> bool {
    state.batch.open_requested.swap(false, Ordering::SeqCst)
}
#[tauri::command]
pub fn batch_list(state: State<AppState>) -> Inbox {
    state.batch.inbox.lock().clone()
}
#[tauri::command]
pub fn batch_stop(state: State<AppState>) {
    state.batch.cancel.store(true, Ordering::Relaxed);
}
#[tauri::command]
pub fn batch_remove(state: State<AppState>, ids: Vec<Uuid>) -> Result<(), String> {
    let mut inbox = state.batch.inbox.lock();
    if inbox.checking {
        return Err("Stop link checking before removing entries.".into());
    }
    inbox.links.retain(|l| !ids.contains(&l.id));
    state.batch.save(&inbox)
}
#[tauri::command]
pub fn batch_analyze(state: State<AppState>, text: String) -> Result<(), String> {
    analyze(state.inner().clone(), text)
}
pub fn analyze(state: AppState, text: String) -> Result<(), String> {
    if text.len() > 2 * 1024 * 1024 {
        return Err("Paste up to 2 MiB of link text at a time.".into());
    }
    if state.batch.busy.swap(true, Ordering::SeqCst) {
        return Err("Link checking is already running.".into());
    }
    let settings = state.engine.settings();
    let mut incoming = links::extract(&text, settings.batch.max_links + 1);
    let mut inbox = state.batch.inbox.lock();
    if incoming.len() > settings.batch.max_links {
        state.batch.busy.store(false, Ordering::SeqCst);
        return Err(format!(
            "The inbox limit is {} links. Remove old entries or increase the limit in Batch settings.",
            settings.batch.max_links
        ));
    }
    let mut seen: HashSet<String> =
        inbox.links.iter().flat_map(|l| [&l.url, &l.original_url]).filter_map(|u| links::canonical(u)).collect();
    incoming.retain(|u| links::canonical(u).is_some_and(|u| seen.insert(u)));
    if incoming.len() + inbox.links.len() > settings.batch.max_links {
        state.batch.busy.store(false, Ordering::SeqCst);
        return Err("The Link Grabber inbox is full. Remove old entries or increase its limit in Batch settings.".into());
    }
    let existing: HashSet<String> = state.engine.jobs().iter().filter_map(|j| links::canonical(&j.job.url)).collect();
    for url in incoming {
        let name = links::hint(&url).unwrap_or_else(|| "Download".into());
        let duplicate = settings.batch.skip_duplicates && links::canonical(&url).is_some_and(|u| existing.contains(&u));
        inbox.links.push(Link {
            id: Uuid::new_v4(),
            original_url: url.clone(),
            package: links::package(&name),
            file_name: name,
            url,
            status: if duplicate { "duplicate" } else { "unchecked" }.into(),
            message: duplicate.then(|| "Already in downloads.".into()),
            probe: None,
            referrer: None,
        });
    }
    let work: Vec<Link> = inbox.links.iter().filter(|l| matches!(l.status.as_str(), "unchecked" | "error")).cloned().collect();
    inbox.checking = !work.is_empty();
    for l in &mut inbox.links {
        if work.iter().any(|w| w.id == l.id) {
            l.status = "checking".into();
        }
    }
    if let Err(e) = state.batch.save(&inbox) {
        state.batch.busy.store(false, Ordering::SeqCst);
        inbox.checking = false;
        return Err(e);
    }
    state.batch.cancel.store(false, Ordering::Relaxed);
    drop(inbox);
    tauri::async_runtime::spawn(async move {
        let mut pending = stream::iter(work.into_iter().map(|link| {
            let state = state.clone();
            async move {
                if state.batch.cancel.load(Ordering::Relaxed) {
                    return (link.id, Err("Checking stopped.".to_string()));
                }
                let result = tokio::time::timeout(std::time::Duration::from_secs(45), check(&state, &link))
                    .await
                    .unwrap_or_else(|_| Err("Link check timed out. Retry or open it in your browser.".into()));
                (link.id, result)
            }
        }))
        .buffer_unordered(settings.batch.check_concurrency);
        while let Some((id, result)) = pending.next().await {
            let mut inbox = state.batch.inbox.lock();
            if let Some(l) = inbox.links.iter_mut().find(|l| l.id == id) {
                match result {
                    Ok((url, p, referrer)) => {
                        l.url = url;
                        l.file_name = p.file_name.clone();
                        l.package = links::package(&p.file_name);
                        l.status = "ready".into();
                        l.message = None;
                        l.probe = Some(p);
                        l.referrer = referrer;
                    }
                    Err(e) => {
                        l.status = if state.batch.cancel.load(Ordering::Relaxed) { "unchecked" } else { "error" }.into();
                        l.message = Some(e);
                    }
                }
            }
        }
        let mut inbox = state.batch.inbox.lock();
        inbox.checking = false;
        if state.batch.save(&inbox).is_err() {
            tracing::warn!("could not save Link Grabber inbox");
        }
        state.batch.busy.store(false, Ordering::SeqCst);
    });
    Ok(())
}
async fn check(state: &AppState, l: &Link) -> Result<(String, ProbeInfo, Option<String>), String> {
    let mut url = if l.original_url.is_empty() { l.url.clone() } else { l.original_url.clone() };
    let mut refer = l.referrer.clone();
    let initial = state
        .engine
        .probe(ProbeRequest { url: url.clone(), file_name_hint: links::hint(&url), referrer: refer.clone(), ..Default::default() })
        .await;
    let needs_page = initial.as_ref().map(|p| p.looks_like_html).unwrap_or(true);
    let mut p = initial.ok();
    if needs_page {
        if let Some(id) = p.as_ref().and_then(|p| p.probe_id) {
            state.engine.discard_probe(id);
        }
        let (html, base) = links::fetch_page(&url).await?;
        let candidates = links::page_links(&html, &base, 1000);
        if candidates.len() != 1 {
            return Err(if candidates.is_empty() {
                "This page requires browser interaction, an account or an unsupported host resolver. Use a direct file link."
            } else {
                "This is a page of links. Use ‘Collect links from page’ to review the files separately."
            }
            .into());
        }
        refer = Some(base);
        url = candidates[0].clone();
        p = Some(
            state
                .engine
                .probe(ProbeRequest {
                    url: url.clone(),
                    referrer: refer.clone(),
                    file_name_hint: links::hint(&l.url),
                    ..Default::default()
                })
                .await
                .map_err(|_| "The file behind this page could not be checked. Open it in your browser to verify access.".to_string())?,
        );
        if p.as_ref().is_some_and(|p| p.looks_like_html) {
            if let Some(id) = p.as_ref().and_then(|p| p.probe_id) {
                state.engine.discard_probe(id);
            }
            return Err("The host still returned a web page. Open this link in your browser.".into());
        }
    }
    let mut p = p.ok_or("The link could not be checked.")?;
    if let Some(id) = p.probe_id {
        state.engine.discard_probe(id);
    }
    p.probe_id = None;
    Ok((url, p, refer))
}
#[tauri::command]
pub async fn batch_collect_page(state: State<'_, AppState>, url: String) -> Result<(), String> {
    if !links::canonical(&url).is_some_and(|u| u.starts_with("http")) {
        return Err("Enter an HTTP or HTTPS page address.".into());
    }
    let (html, base) = links::fetch_page(&url).await?;
    let found = links::page_links(&html, &base, state.engine.settings().batch.max_links + 1);
    if found.is_empty() {
        return Err("No supported downloadable links were found on this page. Paste the file links instead.".into());
    }
    analyze(state.inner().clone(), found.join("\n"))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    id: Uuid,
    package: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Accepted {
    ids: Vec<Uuid>,
    errors: Vec<String>,
}
#[tauri::command]
pub fn batch_accept(state: State<AppState>, selected: Vec<Selection>, root: String, group: bool) -> Result<Accepted, String> {
    if !Path::new(&root).is_absolute() {
        return Err("Choose an absolute download folder.".into());
    }
    let mut inbox = state.batch.inbox.lock();
    if inbox.checking {
        return Err("Wait for checking to finish or stop it first.".into());
    }
    if selected.is_empty() {
        return Err("Select one or more ready files.".into());
    }
    let mut accepted = Accepted { ids: vec![], errors: vec![] };
    let mut removed = HashSet::new();
    for item in selected {
        let Some(l) = inbox.links.iter().find(|l| l.id == item.id && l.status == "ready") else { continue };
        let Some(p) = &l.probe else { continue };
        if state.engine.settings().batch.skip_duplicates
            && state.engine.jobs().iter().any(|j| links::canonical(&j.job.url) == links::canonical(&l.url))
        {
            accepted.errors.push(format!("{} is already in downloads.", l.file_name));
            continue;
        }
        let package = crowdm_engine::names::sanitize(&item.package);
        let dir = if group { PathBuf::from(&root).join(&package) } else { PathBuf::from(&root) };
        let result = state.engine.add(NewJob {
            package: Some(package),
            url: l.url.clone(),
            referrer: l.referrer.clone(),
            file_name: Some(l.file_name.clone()),
            dir: Some(dir),
            category: Some(p.category.clone()),
            size_hint: p.size,
            mime_hint: p.mime.clone(),
            torrent: p.torrent.clone(),
            start: false,
            ..Default::default()
        });
        match result {
            Ok(id) => {
                accepted.ids.push(id);
                removed.insert(l.id);
            }
            Err(_) => accepted.errors.push(format!("{} could not be added.", l.file_name)),
        }
    }
    inbox.links.retain(|l| !removed.contains(&l.id));
    state.batch.save(&inbox)?;
    Ok(accepted)
}

pub fn start_clipboard_monitor(state: AppState) {
    #[cfg(windows)]
    tauri::async_runtime::spawn(async move {
        let mut last = clipboard_sequence();
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(900)).await;
            if state.is_quitting() {
                break;
            }
            let next = clipboard_sequence();
            if next == last {
                continue;
            }
            if !state.engine.settings().batch.monitor_clipboard {
                last = next;
                continue;
            }
            if state.batch.busy.load(Ordering::Relaxed) {
                continue;
            }
            if let Some(text) = clipboard_text() {
                last = next;
                let _ = analyze(state.clone(), text);
            }
        }
    });
}
#[cfg(windows)]
fn clipboard_sequence() -> u32 {
    unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() }
}
#[cfg(windows)]
fn clipboard_text() -> Option<String> {
    use windows_sys::Win32::System::{DataExchange::*, Memory::*};
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return None;
        }
        let handle = GetClipboardData(13);
        let size = if handle.is_null() { 0 } else { GlobalSize(handle) };
        let result = if size > 0 && size <= 2 * 1024 * 1024 {
            let p = GlobalLock(handle) as *const u16;
            if p.is_null() {
                None
            } else {
                let s = std::slice::from_raw_parts(p, size / 2);
                let end = s.iter().position(|n| *n == 0).unwrap_or(s.len());
                let t = String::from_utf16_lossy(&s[..end]);
                GlobalUnlock(handle);
                Some(t)
            }
        } else {
            None
        };
        CloseClipboard();
        result
    }
}

/// Suppress per-part notifications and emit one package result after every part settles.
pub fn package_notification(state: &AppState, job: &crowdm_engine::JobSnapshot) -> Option<(String, String)> {
    let name = job.job.package.as_ref()?;
    let jobs = state.engine.jobs();
    let group: Vec<_> = jobs.iter().filter(|j| j.job.package.as_ref() == Some(name) && j.job.dir == job.job.dir).collect();
    if group.iter().any(|j| matches!(j.job.state, JobState::Downloading | JobState::Queued | JobState::Paused)) {
        return None;
    }
    let failed = group.iter().filter(|j| j.job.state == JobState::Failed).count();
    let mut signatures: Vec<_> = group.iter().map(|j| format!("{}:{:?}", j.job.id, j.job.state)).collect();
    signatures.sort();
    if !state.batch.notified.lock().insert(signatures.join("|")) {
        return None;
    }
    Some((
        if failed == 0 { "Package complete" } else { "Package needs attention" }.into(),
        format!("{}: {} files, {} failed.", name, group.len(), failed),
    ))
}
