//! The download manager: job registry, lifecycle, queue, scheduler,
//! progress reporting and persistence.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};
use reqwest::header::{self, HeaderMap};
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, Semaphore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::checksum;
use crate::error::{EngineError, Result, TransferError};
use crate::file;
use crate::http::{self, ResponseMeta};
use crate::model::*;
use crate::names;
use crate::ratelimit::RateLimiter;
use crate::settings::Settings;
use crate::store::Store;
use crate::transfer::{self, Hooks, Identity, Limits, Shared, Stop, TransferConfig, TransferInput};

/// Events for the UI. Progress is batched; everything else is per job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EngineEvent {
    JobUpdated {
        job: Box<JobSnapshot>,
    },
    JobRemoved {
        id: JobId,
    },
    Progress {
        items: Vec<Progress>,
    },
    JobCompleted {
        job: Box<JobSnapshot>,
    },
    JobFailed {
        job: Box<JobSnapshot>,
    },
    QueueChanged {
        running: bool,
    },
    /// The queue ran out of work after having run something.
    QueueFinished,
}

/// A URL to inspect before the user confirms the download.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProbeRequest {
    pub media_format: Option<String>,
    pub url: String,
    pub referrer: Option<String>,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// Name suggested by the browser.
    pub file_name_hint: Option<String>,
    pub mime_hint: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobPatch {
    pub file_name: Option<String>,
    pub dir: Option<PathBuf>,
    /// `Some(None)` removes the limit.
    pub speed_limit: Option<Option<u64>>,
    pub max_connections: Option<Option<u8>>,
    /// Refresh an expired link: new URL and headers for the same file.
    pub url: Option<String>,
    pub headers: Option<Vec<(String, String)>>,
    pub queued: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopReason {
    None,
    Pause,
    Requeue,
    Remove,
    Shutdown,
}

struct SpeedMeter {
    last_bytes: u64,
    last_at: Instant,
    speed: f64,
}

struct Run {
    cancel: CancellationToken,
    shared: Arc<Shared>,
    limiter: Arc<RateLimiter>,
    reason: Arc<Mutex<StopReason>>,
    meter: SpeedMeter,
    done: Arc<tokio::sync::Notify>,
    finished: Arc<AtomicBool>,
}

struct Entry {
    job: Job,
    segments: Vec<Segment>,
    run: Option<Run>,
    /// Phase text outside a transfer (e.g. waiting to retry).
    phase: Option<String>,
}

struct PendingProbe {
    response: Response,
    meta: ResponseMeta,
    at: Instant,
}

struct Inner {
    torrents: tokio::sync::OnceCell<Arc<librqbit::Session>>,
    torrent_peers: Mutex<HashMap<String, Vec<std::net::SocketAddr>>>,
    /// Runtime the engine was opened on; used so API calls work from any
    /// thread (e.g. the UI thread), not only from inside the runtime.
    rt: tokio::runtime::Handle,
    store: Store,
    jobs: Mutex<HashMap<JobId, Entry>>,
    settings: RwLock<Settings>,
    events: broadcast::Sender<EngineEvent>,
    client: RwLock<Client>,
    client_ua: RwLock<Option<String>>,
    global_limit: Arc<RateLimiter>,
    connections: Arc<Semaphore>,
    connection_permits: AtomicUsize,
    probes: Mutex<HashMap<Uuid, PendingProbe>>,
    queue_running: AtomicBool,
    /// Something from the queue ran since the queue last became idle.
    queue_did_work: AtomicBool,
    shutting_down: AtomicBool,
    schedule_state: Mutex<ScheduleState>,
    /// Connection limits servers enforced recently, by host name.
    host_limits: Mutex<HashMap<String, (usize, Instant)>>,
}

#[derive(Default)]
struct ScheduleState {
    last_start_fired: Option<String>,
    last_stop_fired: Option<String>,
}

/// Handle to the download engine. Cheap to clone.
#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

impl Engine {
    /// Opens (or creates) the database at `db_path` and starts background
    /// tasks. Must be called inside a Tokio runtime.
    pub fn open(db_path: &Path) -> Result<Engine> {
        Self::with_store(Store::open(db_path)?)
    }

    pub fn with_store(store: Store) -> Result<Engine> {
        let settings = store.load_settings()?.unwrap_or_default().sanitized();
        let client = http::build_client(settings.user_agent.as_deref()).map_err(|e| EngineError::Invalid(e.to_string()))?;
        let (events, _) = broadcast::channel(1024);
        let permits = settings.max_total_connections as usize;
        let inner = Arc::new(Inner {
            torrents: tokio::sync::OnceCell::new(),
            torrent_peers: Mutex::new(HashMap::new()),
            rt: tokio::runtime::Handle::current(),
            store,
            jobs: Mutex::new(HashMap::new()),
            global_limit: Arc::new(RateLimiter::new(settings.speed_limit)),
            client_ua: RwLock::new(settings.user_agent.clone()),
            settings: RwLock::new(settings),
            events,
            client: RwLock::new(client),
            connections: Arc::new(Semaphore::new(permits)),
            connection_permits: AtomicUsize::new(permits),
            probes: Mutex::new(HashMap::new()),
            queue_running: AtomicBool::new(true),
            queue_did_work: AtomicBool::new(false),
            shutting_down: AtomicBool::new(false),
            schedule_state: Mutex::new(ScheduleState::default()),
            host_limits: Mutex::new(HashMap::new()),
        });
        let engine = Engine { inner };
        engine.load()?;
        engine.spawn_ticker();
        Ok(engine)
    }

    fn load(&self) -> Result<()> {
        let rows = self.inner.store.load_jobs()?;
        let resume = self.inner.settings.read().resume_on_startup;
        let mut to_start = Vec::new();
        {
            let mut jobs = self.inner.jobs.lock();
            for (mut job, segments) in rows {
                if job.state == JobState::Downloading {
                    if resume {
                        if job.queued {
                            job.state = JobState::Queued;
                        } else {
                            to_start.push(job.id);
                        }
                    } else {
                        job.state = JobState::Paused;
                    }
                }
                job.downloaded = if job.state == JobState::Completed {
                    job.size.unwrap_or(job.downloaded)
                } else {
                    segments.iter().map(|s| s.done()).sum()
                };
                jobs.insert(job.id, Entry { job, segments, run: None, phase: None });
            }
        }
        if self.inner.settings.read().schedule.enabled {
            self.inner.queue_running.store(schedule_window_open(&self.inner.settings.read().schedule), Ordering::Relaxed);
        }
        for id in to_start {
            let _ = self.start(id);
        }
        self.pump_queue();
        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.inner.events.subscribe()
    }

    fn emit(&self, e: EngineEvent) {
        let _ = self.inner.events.send(e);
    }

    pub fn settings(&self) -> Settings {
        self.inner.settings.read().clone()
    }

    pub fn store(&self) -> &Store {
        &self.inner.store
    }

    pub fn update_settings(&self, s: Settings) -> Result<Settings> {
        let s = s.sanitized();
        self.inner.store.save_settings(&s)?;
        self.inner.global_limit.set_rate(s.speed_limit);
        // Resize the global connection pool.
        let want = s.max_total_connections as usize;
        let have = self.inner.connection_permits.swap(want, Ordering::SeqCst);
        if want > have {
            self.inner.connections.add_permits(want - have);
        } else if have > want {
            let sem = self.inner.connections.clone();
            let n = (have - want) as u32;
            self.inner.rt.spawn(async move {
                if let Ok(p) = sem.acquire_many_owned(n).await {
                    p.forget();
                }
            });
        }
        if *self.inner.client_ua.read() != s.user_agent {
            if let Ok(c) = http::build_client(s.user_agent.as_deref()) {
                *self.inner.client.write() = c;
                *self.inner.client_ua.write() = s.user_agent.clone();
            }
        }
        let schedule_changed = self.inner.settings.read().schedule != s.schedule;
        *self.inner.settings.write() = s.clone();
        if schedule_changed {
            *self.inner.schedule_state.lock() = ScheduleState::default();
        }
        self.pump_queue();
        Ok(s)
    }

    // ----- queries -------------------------------------------------------

    pub fn jobs(&self) -> Vec<JobSnapshot> {
        let jobs = self.inner.jobs.lock();
        let mut v: Vec<JobSnapshot> = jobs.values().map(snapshot).collect();
        v.sort_by(|a, b| b.job.created_at.cmp(&a.job.created_at));
        v
    }

    pub fn job(&self, id: JobId) -> Option<JobSnapshot> {
        self.inner.jobs.lock().get(&id).map(snapshot)
    }

    /// Most recent job for the same URL, if any.
    pub fn find_by_url(&self, url: &str) -> Option<JobId> {
        let jobs = self.inner.jobs.lock();
        jobs.values()
            .filter(|e| e.job.url == url || e.job.final_url.as_deref() == Some(url))
            .max_by_key(|e| e.job.created_at)
            .map(|e| e.job.id)
    }

    pub fn queue_running(&self) -> bool {
        self.inner.queue_running.load(Ordering::Relaxed)
    }

    fn taken_targets(&self) -> HashSet<String> {
        self.inner
            .jobs
            .lock()
            .values()
            .filter(|e| e.job.state != JobState::Completed)
            .map(|e| e.job.target_path().to_string_lossy().to_lowercase())
            .collect()
    }

    // ----- probing -------------------------------------------------------

    /// Supply a custom session before using torrents (also enables completely
    /// isolated loopback swarms in integration tests).
    pub fn use_torrent_session(&self, session: Arc<librqbit::Session>) -> Result<()> {
        self.inner.torrents.set(session).map_err(|_| EngineError::Invalid("Torrent session is already initialized.".into()))
    }

    async fn torrent_session(&self) -> Result<Arc<librqbit::Session>> {
        Ok(self.inner.torrents.get_or_try_init(|| async { crate::torrent::new_session(&self.settings()).await }).await?.clone())
    }

    fn torrent_probe_bytes(&self, bytes: &[u8], input: &str) -> Result<ProbeInfo> {
        let (torrent, name) = crate::torrent::inspect(bytes)?;
        self.inner.store.save_torrent(&torrent.info_hash, bytes)?;
        let duplicate = self
            .inner
            .jobs
            .lock()
            .values()
            .find(|e| e.job.torrent.as_ref().is_some_and(|t| t.info_hash == torrent.info_hash))
            .map(|e| e.job.id);
        let dir = self.settings().dir_for_category("torrents");
        Ok(ProbeInfo {
            size: Some(torrent.selected_size()),
            exists: dir.join(&name).exists(),
            torrent: Some(torrent),
            size_estimated: false,
            media_options: vec![],
            probe_id: None,
            final_url: input.into(),
            file_name: name,
            resumable: true,
            mime: Some("application/x-bittorrent".into()),
            category: "torrents".into(),
            dir,
            duplicate_of: duplicate,
            warnings: vec![
                "Torrent pieces are verified automatically. Uploading is enabled while downloading and stops at completion.".into()
            ],
            status: 200,
            looks_like_html: false,
        })
    }

    async fn probe_torrent(&self, input: &str) -> Result<ProbeInfo> {
        let normalized;
        let input = if input.get(..7).is_some_and(|s| s.eq_ignore_ascii_case("magnet:")) {
            normalized = format!("magnet:{}", &input[7..]);
            &normalized
        } else {
            input
        };
        if input.starts_with("magnet:") {
            let magnet = librqbit::Magnet::parse(input).map_err(|_| EngineError::Invalid("Invalid magnet link.".into()))?;
            if let Some(hash) = magnet.as_id20() {
                if let Ok(bytes) = self.inner.store.load_torrent(&hash.as_string()) {
                    return self.torrent_probe_bytes(&bytes, input);
                }
            }
            let session = self.torrent_session().await?;
            let (bytes, peers) = crate::torrent::resolve(&session, input).await?;
            let info = self.torrent_probe_bytes(&bytes, input)?;
            self.inner.torrent_peers.lock().insert(info.torrent.as_ref().unwrap().info_hash.clone(), peers);
            Ok(info)
        } else {
            self.torrent_probe_bytes(&crate::torrent::read_local(input).await?, input)
        }
    }

    pub async fn probe_media(&self, req: ProbeRequest) -> Result<ProbeInfo> {
        validate_url(&req.url)?;
        let mut info = crate::media::probe(req, &self.settings()).await?;
        info.duplicate_of = self.find_by_url(&info.final_url);
        Ok(info)
    }

    /// Opens the URL to learn its name, size and resumability. The open
    /// response is kept for a minute so that confirming the download reuses
    /// it instead of requesting the file twice (important for one-time links).
    pub async fn probe(&self, req: ProbeRequest) -> Result<ProbeInfo> {
        if crate::torrent::is_input(&req.url) {
            return self.probe_torrent(req.url.trim()).await;
        }
        validate_url(&req.url)?;
        let client = self.inner.client.read().clone();
        let headers = http::header_map(&req.headers, req.referrer.as_deref());
        let resp = client.get(&req.url).headers(headers).header(header::RANGE, "bytes=0-").send().await.map_err(http::request_error)?;
        if !resp.status().is_success() {
            return Err(http::status_error(&resp).into());
        }
        let meta = ResponseMeta::from_response(&resp);
        let settings = self.settings();
        let mime = meta.mime.clone().or(req.mime_hint.clone());
        let server_name = meta.content_disposition.as_deref().and_then(names::from_content_disposition);
        let file_name = match (server_name, req.file_name_hint.as_deref()) {
            (Some(n), _) => names::sanitize(&names::with_mime_extension(names::sanitize(&n), mime.as_deref())),
            (None, Some(h)) if !h.trim().is_empty() => names::sanitize(h),
            _ => names::choose(None, &meta.final_url, &req.url, mime.as_deref()),
        };
        let category = settings.category_for(&file_name);
        if !meta.looks_like_html()
            && (file_name.to_ascii_lowercase().ends_with(".torrent") || mime.as_deref() == Some("application/x-bittorrent"))
        {
            use futures_util::StreamExt;
            let mut stream = resp.bytes_stream();
            let mut bytes = Vec::new();
            let read = async {
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(http::request_error)?;
                    if bytes.len().saturating_add(chunk.len()) > crate::torrent::MAX_METAINFO {
                        return Err(EngineError::Invalid("Torrent metadata exceeds 16 MiB.".into()));
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Ok::<(), EngineError>(())
            };
            tokio::time::timeout(Duration::from_secs(30), read)
                .await
                .map_err(|_| EngineError::Invalid("Timed out reading torrent metadata.".into()))??;
            return self.torrent_probe_bytes(&bytes, &req.url);
        }
        let dir = settings.dir_for_category(&category);
        let size = meta.total_size();
        let resumable = size.is_some() && (meta.status == 206 || meta.accept_ranges) && meta.content_encoding.is_none();
        let mut warnings = Vec::new();
        if !resumable {
            warnings.push("The server does not support resuming; a paused download will restart from the beginning.".into());
        }
        if size.is_none() {
            warnings.push("The server did not report the file size.".into());
        }
        if meta.looks_like_html() && !file_name.to_ascii_lowercase().ends_with(".html") && !file_name.to_ascii_lowercase().ends_with(".htm")
        {
            warnings.push("The server answered with a web page, not a file. The link may need the browser's session.".into());
        }
        let exists = dir.join(&file_name).exists();
        let duplicate_of = self.find_by_url(&req.url).or_else(|| self.find_by_url(&meta.final_url));
        let probe_id = Uuid::new_v4();
        let info = ProbeInfo {
            torrent: None,
            size_estimated: false,
            media_options: vec![],
            probe_id: Some(probe_id),
            final_url: meta.final_url.clone(),
            file_name,
            size,
            resumable,
            mime,
            category,
            dir,
            exists,
            duplicate_of,
            warnings,
            status: meta.status,
            looks_like_html: meta.looks_like_html(),
        };
        self.inner.probes.lock().insert(probe_id, PendingProbe { response: resp, meta, at: Instant::now() });
        Ok(info)
    }

    /// Drops a pending probe connection (dialog cancelled).
    pub fn discard_probe(&self, id: Uuid) {
        self.inner.probes.lock().remove(&id);
    }

    // ----- lifecycle -----------------------------------------------------

    pub fn add(&self, mut new: NewJob) -> Result<JobId> {
        if let Some(requested) = &new.torrent {
            let bytes = self.inner.store.load_torrent(&requested.info_hash)?;
            let (mut validated, _) = crate::torrent::inspect(&bytes)?;
            if validated.info_hash != requested.info_hash {
                return Err(EngineError::Invalid("Torrent metadata identity mismatch.".into()));
            }
            validated.select(requested.only_files.clone())?;
            if self
                .inner
                .jobs
                .lock()
                .values()
                .any(|e| e.job.state != JobState::Completed && e.job.torrent.as_ref().is_some_and(|t| t.info_hash == validated.info_hash))
            {
                return Err(EngineError::Invalid("This torrent is already in CrowDM. Resume its existing download.".into()));
            }
            new.size_hint = Some(validated.selected_size());
            new.category = Some("torrents".into());
            new.media_download = false;
            new.torrent = Some(validated);
        } else {
            validate_url(&new.url)?;
        }
        let settings = self.settings();
        let pending = new.probe_id.and_then(|id| self.inner.probes.lock().remove(&id));
        let meta = pending.as_ref().map(|p| &p.meta);
        let mime = meta.and_then(|m| m.mime.clone()).or(new.mime_hint.clone());
        if settings.interception.skip_web_pages
            && (meta.is_some_and(|m| m.looks_like_html())
                || mime.as_deref().is_some_and(|m| m.starts_with("text/html") || m.starts_with("application/xhtml")))
        {
            return Err(EngineError::Invalid(
                "Web-page downloads are disabled in Settings. Use the video button to download streaming media.".into(),
            ));
        }
        let mut file_name = match &new.file_name {
            Some(n) if !n.trim().is_empty() => names::sanitize(n),
            _ => names::choose(
                meta.and_then(|m| m.content_disposition.as_deref()),
                meta.map(|m| m.final_url.as_str()).unwrap_or(&new.url),
                &new.url,
                mime.as_deref(),
            ),
        };
        if new.media_download {
            let extension = crate::media::output_format(new.media_format.as_deref()).0;
            file_name = format!("{}.{extension}", std::path::Path::new(&file_name).file_stem().unwrap_or_default().to_string_lossy());
        }
        let category = new
            .category
            .clone()
            .filter(|c| settings.categories.iter().any(|x| &x.id == c))
            .unwrap_or_else(|| settings.category_for(&file_name));
        let dir = new.dir.clone().unwrap_or_else(|| settings.dir_for_category(&category));
        let target = if new.overwrite { dir.join(&file_name) } else { names::unique_path(&dir, &file_name, &self.taken_targets()) };
        let final_name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(file_name);

        let now = now_ms();
        let size = meta.and_then(|m| m.total_size()).or(new.size_hint);
        let job = Job {
            package: new.package.clone(),
            torrent: new.torrent.clone(),
            media_download: new.media_download,
            media_format: new.media_format.clone(),
            id: Uuid::new_v4(),
            url: new.url.clone(),
            final_url: meta.map(|m| m.final_url.clone()),
            referrer: new.referrer.clone(),
            headers: new.headers.clone(),
            file_name: final_name,
            dir,
            category,
            size,
            resumable: if new.media_download || new.torrent.is_some() { Some(true) } else { None },
            etag: None,
            last_modified: None,
            mime,
            state: if new.start { JobState::Downloading } else { JobState::Queued },
            error: None,
            created_at: now,
            started_at: None,
            completed_at: None,
            queued: !new.start,
            queue_order: now,
            max_connections: new.max_connections,
            speed_limit: new.speed_limit,
            checksum: new.checksum.clone(),
            auto_retries: 0,
            source: new.source.clone(),
            downloaded: 0,
            active_ms: 0,
            auto_name: new.file_name.as_deref().is_none_or(|n| n.trim().is_empty()) && pending.is_none(),
            auto_dir: new.dir.is_none(),
        };
        let id = job.id;
        self.inner.store.save_job_and_segments(&job, &[])?;
        {
            let mut jobs = self.inner.jobs.lock();
            jobs.insert(id, Entry { job, segments: vec![], run: None, phase: None });
        }
        if new.start {
            self.start_with(id, pending.map(|p| (p.response, p.meta)))?;
        } else {
            self.emit_job(id);
            self.pump_queue();
        }
        Ok(id)
    }

    /// Starts or resumes a job now, regardless of the queue limit.
    pub fn start(&self, id: JobId) -> Result<()> {
        self.start_with(id, None)
    }

    fn start_with(&self, id: JobId, pending: Option<(Response, ResponseMeta)>) -> Result<()> {
        if self.inner.shutting_down.load(Ordering::Relaxed) {
            return Ok(());
        }
        let (input, shared, cancel, limiter, reason, done, finished, job) = {
            let mut jobs = self.inner.jobs.lock();
            let entry = jobs.get_mut(&id).ok_or(EngineError::NotFound)?;
            if entry.run.is_some() {
                return Ok(());
            }
            if entry.job.state == JobState::Completed {
                return Ok(());
            }
            let settings = self.inner.settings.read();
            let max_conns = self.connection_target(&entry.job, &settings);
            let job = &mut entry.job;
            job.state = JobState::Downloading;
            job.error = None;
            if job.started_at.is_none() {
                job.started_at = Some(now_ms());
            }
            entry.phase = None;
            let resumable = job.resumable;
            let shared = Shared::new(&entry.segments, job.size, resumable.unwrap_or(false), max_conns);
            let input = TransferInput {
                url: job.final_url.clone().filter(|_| !entry.segments.is_empty()).unwrap_or_else(|| job.url.clone()),
                headers: http::header_map(&job.headers, job.referrer.as_deref()),
                part_path: job.part_path(),
                size: job.size,
                resumable,
                identity: Identity { etag: job.etag.clone(), last_modified: job.last_modified.clone() },
                segments: entry.segments.clone(),
                pending,
            };
            let cancel = CancellationToken::new();
            let limiter = Arc::new(RateLimiter::new(job.speed_limit.unwrap_or(0)));
            let reason = Arc::new(Mutex::new(StopReason::None));
            let done = Arc::new(tokio::sync::Notify::new());
            let finished = Arc::new(AtomicBool::new(false));
            entry.run = Some(Run {
                cancel: cancel.clone(),
                shared: shared.clone(),
                limiter: limiter.clone(),
                reason: reason.clone(),
                meter: SpeedMeter { last_bytes: 0, last_at: Instant::now(), speed: 0.0 },
                done: done.clone(),
                finished: finished.clone(),
            });
            (input, shared, cancel, limiter, reason, done, finished, entry.job.clone())
        };
        let _ = self.inner.store.save_job(&job);
        self.emit_job(id);
        let engine = self.clone();
        self.inner.rt.spawn(async move {
            engine.drive(id, input, shared, cancel, limiter, reason).await;
            finished.store(true, Ordering::SeqCst);
            done.notify_waiters();
        });
        Ok(())
    }

    async fn drive(
        &self,
        id: JobId,
        mut input: TransferInput,
        mut shared: Arc<Shared>,
        cancel: CancellationToken,
        limiter: Arc<RateLimiter>,
        reason: Arc<Mutex<StopReason>>,
    ) {
        let started = Instant::now();
        let outcome: Result<(), Stop> = loop {
            let settings = self.settings();
            let cfg = TransferConfig {
                reject_html: settings.interception.skip_web_pages,
                max_connections: {
                    let jobs = self.inner.jobs.lock();
                    jobs.get(&id)
                        .and_then(|e| e.job.max_connections)
                        .map(|c| c as usize)
                        .unwrap_or(settings.max_connections_per_download as usize)
                },
                ..TransferConfig::default()
            };
            let limits =
                Limits { global: self.inner.global_limit.clone(), job: limiter.clone(), connections: self.inner.connections.clone() };
            let client = self.inner.client.read().clone();
            let hooks = self.hooks(id);
            let torrent_job = self.inner.jobs.lock().get(&id).filter(|e| e.job.torrent.is_some()).map(|e| e.job.clone());
            if let Some(job) = torrent_job {
                let outcome = async {
                    let session =
                        self.torrent_session().await.map_err(|e| Stop::Failed(TransferError::new(ErrorKind::Network, e.to_string())))?;
                    let info = job.torrent.as_ref().unwrap();
                    let bytes = self
                        .inner
                        .store
                        .load_torrent(&info.info_hash)
                        .map_err(|e| Stop::Failed(TransferError::new(ErrorKind::Io, e.to_string())))?;
                    let peers = self.inner.torrent_peers.lock().get(&info.info_hash).cloned().unwrap_or_default();
                    let size = crate::torrent::run(session, bytes, job.clone(), shared.clone(), cancel.clone(), peers).await?;
                    if cancel.is_cancelled() {
                        return Err(Stop::Canceled);
                    }
                    shared.set_phase(Some("Finishing torrent"));
                    let final_path = tokio::task::spawn_blocking(move || crate::torrent::finish(&job))
                        .await
                        .map_err(|_| Stop::Failed(TransferError::new(ErrorKind::Io, "Torrent finalization failed.")))?
                        .map_err(|e| Stop::Failed(TransferError::new(ErrorKind::Io, e.to_string())))?;
                    let mut jobs = self.inner.jobs.lock();
                    if let Some(entry) = jobs.get_mut(&id) {
                        entry.job.file_name = final_path.file_name().unwrap().to_string_lossy().into_owned();
                        entry.job.state = JobState::Completed;
                        entry.job.completed_at = Some(now_ms());
                        entry.job.size = Some(size);
                        entry.job.downloaded = size;
                        entry.job.headers.clear();
                        entry.job.error = None;
                        entry.segments.clear();
                    }
                    Ok(())
                }
                .await;
                break outcome;
            }
            let media_job = self.inner.jobs.lock().get(&id).filter(|e| e.job.media_download).map(|e| e.job.clone());
            let res = if let Some(job) = media_job {
                crate::media::run(job, shared.clone(), cancel.clone(), limiter.clone(), self.inner.global_limit.clone()).await
            } else {
                transfer::run(client, input, shared.clone(), limits, cfg, cancel.clone(), hooks).await
            };
            self.learn_host_limit(id, &shared);
            match res {
                Ok(done) => {
                    break self.finalize(id, done, &shared).await;
                }
                Err(Stop::Canceled) => break Err(Stop::Canceled),
                Err(Stop::Failed(e)) => {
                    let retry = {
                        let mut jobs = self.inner.jobs.lock();
                        match jobs.get_mut(&id) {
                            Some(entry) if e.kind.is_retryable() && entry.job.auto_retries < settings.retry_attempts => {
                                entry.job.auto_retries += 1;
                                let n = entry.job.auto_retries;
                                entry.phase = Some(format!(
                                    "Retrying in {}s (attempt {n} of {})",
                                    settings.retry_delay_secs, settings.retry_attempts
                                ));
                                entry.job.error = Some(e.to_job_error());
                                true
                            }
                            _ => false,
                        }
                    };
                    if !retry {
                        break Err(Stop::Failed(e));
                    }
                    self.emit_job(id);
                    let delay = e.retry_after.unwrap_or(Duration::from_secs(settings.retry_delay_secs as u64));
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = cancel.cancelled() => break Err(Stop::Canceled),
                    }
                    // Rebuild the input from the persisted state and go again.
                    let rebuilt = {
                        let mut jobs = self.inner.jobs.lock();
                        let Some(entry) = jobs.get_mut(&id) else { break Err(Stop::Canceled) };
                        entry.phase = None;
                        let job = &entry.job;
                        let max_conns = self.connection_target(job, &settings);
                        let new_shared = Shared::new(&entry.segments, job.size, job.resumable.unwrap_or(false), max_conns);
                        if let Some(run) = entry.run.as_mut() {
                            run.shared = new_shared.clone();
                            run.meter = SpeedMeter { last_bytes: 0, last_at: Instant::now(), speed: 0.0 };
                        }
                        (
                            TransferInput {
                                url: job.final_url.clone().unwrap_or_else(|| job.url.clone()),
                                headers: http::header_map(&job.headers, job.referrer.as_deref()),
                                part_path: job.part_path(),
                                size: job.size,
                                resumable: job.resumable,
                                identity: Identity { etag: job.etag.clone(), last_modified: job.last_modified.clone() },
                                segments: entry.segments.clone(),
                                pending: None,
                            },
                            new_shared,
                        )
                    };
                    input = rebuilt.0;
                    shared = rebuilt.1;
                    self.emit_job(id);
                }
            }
        };

        let why = *reason.lock();
        let mut event = None;
        {
            let mut jobs = self.inner.jobs.lock();
            let Some(entry) = jobs.get_mut(&id) else { return };
            if entry.job.torrent.is_some() && outcome.is_err() {
                entry.job.downloaded = shared.downloaded();
                entry.segments = shared.segments();
            }
            entry.run = None;
            entry.phase = None;
            entry.job.active_ms += started.elapsed().as_millis() as u64;
            match &outcome {
                Ok(()) => {
                    event = Some(true);
                }
                Err(Stop::Canceled) => match why {
                    StopReason::Requeue => entry.job.state = JobState::Queued,
                    StopReason::Shutdown => entry.job.state = JobState::Downloading,
                    StopReason::Remove => {}
                    _ => entry.job.state = JobState::Paused,
                },
                Err(Stop::Failed(e)) => {
                    entry.job.state = JobState::Failed;
                    entry.job.error = Some(e.to_job_error());
                    event = Some(false);
                }
            }
            if why != StopReason::Remove {
                let _ = self.inner.store.save_job_and_segments(&entry.job, &entry.segments);
            }
        }
        if why == StopReason::Remove {
            return;
        }
        if let Some(snap) = self.job(id) {
            self.emit(EngineEvent::JobUpdated { job: Box::new(snap.clone()) });
            match event {
                Some(true) => self.emit(EngineEvent::JobCompleted { job: Box::new(snap) }),
                Some(false) => self.emit(EngineEvent::JobFailed { job: Box::new(snap) }),
                None => {}
            }
        }
        if why != StopReason::Shutdown {
            self.pump_queue();
        }
    }

    fn hooks(&self, id: JobId) -> Hooks {
        let e1 = self.clone();
        let e2 = self.clone();
        Hooks {
            on_meta: Box::new(move |meta, size, ranges| {
                let taken = e1.taken_targets();
                let settings = e1.settings();
                let mut new_part = None;
                let mut jobs = e1.inner.jobs.lock();
                if let Some(entry) = jobs.get_mut(&id) {
                    let j = &mut entry.job;
                    if j.auto_name {
                        j.auto_name = false;
                        let mime = meta.mime.as_deref().or(j.mime.as_deref());
                        let name = names::choose(meta.content_disposition.as_deref(), &meta.final_url, &j.url, mime);
                        if name != j.file_name {
                            if j.auto_dir {
                                j.category = settings.category_for(&name);
                                j.dir = settings.dir_for_category(&j.category);
                            }
                            let mut taken = taken;
                            taken.remove(&j.target_path().to_string_lossy().to_lowercase());
                            let target = names::unique_path(&j.dir, &name, &taken);
                            j.file_name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(name);
                            new_part = Some(j.part_path());
                        }
                    }
                    j.final_url = Some(meta.final_url.clone());
                    j.size = size;
                    j.resumable = Some(ranges);
                    j.etag = meta.etag.clone();
                    j.last_modified = meta.last_modified.clone();
                    if meta.mime.is_some() {
                        j.mime = meta.mime.clone();
                    }
                    entry.segments.clear();
                    let _ = e1.inner.store.save_job_and_segments(j, &[]);
                }
                drop(jobs);
                e1.emit_job(id);
                new_part
            }),
            persist: Box::new(move |segs| {
                let mut jobs = e2.inner.jobs.lock();
                if let Some(entry) = jobs.get_mut(&id) {
                    entry.job.downloaded = segs.iter().map(|s| s.done()).sum();
                    if entry.job.size.is_none() {
                        // Unknown length finished: the stream end is the size.
                        if let [only] = segs.as_slice() {
                            if only.end != transfer::UNKNOWN_END && only.is_complete() {
                                entry.job.size = Some(only.end);
                            }
                        }
                    }
                    entry.segments = segs;
                    let _ = e2.inner.store.save_job_and_segments(&entry.job, &entry.segments);
                }
            }),
        }
    }

    /// Connections to use for a job: its own or the global setting, capped by
    /// a limit the same server enforced within the last hour.
    fn connection_target(&self, job: &Job, settings: &Settings) -> usize {
        let wanted = job.max_connections.map(|c| c as usize).unwrap_or(settings.max_connections_per_download as usize).clamp(1, 32);
        let host = host_of(job.final_url.as_deref().unwrap_or(&job.url));
        let limits = self.inner.host_limits.lock();
        match limits.get(&host) {
            Some((n, at)) if at.elapsed() < Duration::from_secs(3600) => wanted.min(*n).max(1),
            _ => wanted,
        }
    }

    fn learn_host_limit(&self, id: JobId, shared: &Shared) {
        let n = shared.limited_to.load(Ordering::Relaxed);
        if n == 0 {
            return;
        }
        let url = self.inner.jobs.lock().get(&id).map(|e| e.job.final_url.clone().unwrap_or_else(|| e.job.url.clone()));
        if let Some(url) = url {
            tracing::info!("{} accepts at most {n} connections; remembering that for an hour", host_of(&url));
            self.inner.host_limits.lock().insert(host_of(&url), (n, Instant::now()));
        }
    }

    async fn finalize(&self, id: JobId, done: transfer::Completed, shared: &Arc<Shared>) -> Result<(), Stop> {
        let (part, target, checksum, overwrite) = {
            let jobs = self.inner.jobs.lock();
            let Some(e) = jobs.get(&id) else { return Err(Stop::Canceled) };
            (e.job.part_path(), e.job.target_path(), e.job.checksum.clone(), false)
        };
        if checksum.is_some() {
            shared.set_phase(Some("Verifying checksum"));
        } else {
            shared.set_phase(Some("Finishing"));
        }
        let file = done.file;
        let size = done.size;
        let result = tokio::task::spawn_blocking(move || -> Result<PathBuf, TransferError> {
            file.clear_sparse();
            drop(file);
            if let Some(c) = &checksum {
                let actual =
                    checksum::file_digest(&part, c.algo).map_err(|e| TransferError::io("Could not read the file to verify it", e))?;
                if !actual.eq_ignore_ascii_case(c.expected.trim()) {
                    return Err(TransferError::new(
                        ErrorKind::ChecksumMismatch,
                        format!(
                            "Checksum mismatch: expected {}, got {actual}. The file was kept with a .crowdm-part extension.",
                            c.expected.trim()
                        ),
                    ));
                }
            }
            file::finalize(&part, &target, overwrite).map_err(|e| TransferError::io("Could not move the finished file into place", e))
        })
        .await
        .expect("finalize task");
        let final_path = result.map_err(Stop::Failed)?;
        let mut jobs = self.inner.jobs.lock();
        if let Some(e) = jobs.get_mut(&id) {
            let j = &mut e.job;
            if let Some(name) = final_path.file_name() {
                j.file_name = name.to_string_lossy().into_owned();
            }
            j.state = JobState::Completed;
            j.completed_at = Some(now_ms());
            j.size = Some(size);
            j.downloaded = size;
            j.error = None;
            j.auto_retries = 0;
            // Credentials are only needed while transferring.
            j.headers.clear();
            e.segments.clear();
        }
        Ok(())
    }

    fn stop(&self, id: JobId, why: StopReason) -> Option<(Arc<tokio::sync::Notify>, Arc<AtomicBool>)> {
        let jobs = self.inner.jobs.lock();
        let entry = jobs.get(&id)?;
        let run = entry.run.as_ref()?;
        *run.reason.lock() = why;
        run.cancel.cancel();
        Some((run.done.clone(), run.finished.clone()))
    }

    pub fn pause(&self, id: JobId) -> Result<()> {
        if self.stop(id, StopReason::Pause).is_none() {
            let mut jobs = self.inner.jobs.lock();
            let entry = jobs.get_mut(&id).ok_or(EngineError::NotFound)?;
            if matches!(entry.job.state, JobState::Queued | JobState::Downloading) {
                entry.job.state = JobState::Paused;
                let _ = self.inner.store.save_job(&entry.job);
                drop(jobs);
                self.emit_job(id);
            }
        }
        Ok(())
    }

    pub fn pause_all(&self) {
        let ids: Vec<JobId> = self
            .inner
            .jobs
            .lock()
            .values()
            .filter(|e| matches!(e.job.state, JobState::Downloading | JobState::Queued))
            .map(|e| e.job.id)
            .collect();
        for id in ids {
            let _ = self.pause(id);
        }
    }

    pub fn resume_all(&self) {
        let ids: Vec<JobId> = self
            .inner
            .jobs
            .lock()
            .values()
            .filter(|e| matches!(e.job.state, JobState::Paused | JobState::Failed))
            .map(|e| e.job.id)
            .collect();
        for id in ids {
            let _ = self.start(id);
        }
    }

    /// Waits until the job's transfer task has fully stopped.
    async fn wait_stopped(&self, waiter: Option<(Arc<tokio::sync::Notify>, Arc<AtomicBool>)>) {
        if let Some((notify, finished)) = waiter {
            loop {
                let n = notify.notified();
                if finished.load(Ordering::SeqCst) {
                    break;
                }
                let _ = tokio::time::timeout(Duration::from_millis(200), n).await;
            }
        }
    }

    /// Removes a job. Its partial file is deleted; a completed file is kept
    /// (the UI deletes it through the recycle bin when asked).
    pub async fn remove(&self, id: JobId) -> Result<()> {
        let waiter = self.stop(id, StopReason::Remove);
        self.wait_stopped(waiter).await;
        let entry = self.inner.jobs.lock().remove(&id).ok_or(EngineError::NotFound)?;
        self.inner.store.delete_job(&id)?;
        if entry.job.state != JobState::Completed {
            if entry.job.torrent.is_some() {
                crate::torrent::cleanup(&entry.job);
            }
            let _ = std::fs::remove_file(entry.job.part_path());
            if entry.job.media_download {
                crate::media::cleanup(&entry.job);
            }
        }
        self.emit(EngineEvent::JobRemoved { id });
        self.pump_queue();
        Ok(())
    }

    /// Discards progress and downloads the file again from the start.
    pub async fn restart(&self, id: JobId) -> Result<()> {
        let waiter = self.stop(id, StopReason::Pause);
        self.wait_stopped(waiter).await;
        {
            let mut jobs = self.inner.jobs.lock();
            let entry = jobs.get_mut(&id).ok_or(EngineError::NotFound)?;
            let _ = std::fs::remove_file(entry.job.part_path());
            if entry.job.torrent.is_some() {
                crate::torrent::cleanup(&entry.job);
            }
            if entry.job.media_download {
                crate::media::cleanup(&entry.job);
            }
            if entry.job.state == JobState::Completed {
                // Pick a fresh name so the finished copy is not overwritten.
                let taken: HashSet<String> = HashSet::new();
                let target = names::unique_path(&entry.job.dir, &entry.job.file_name, &taken);
                entry.job.file_name = target.file_name().unwrap().to_string_lossy().into_owned();
                entry.job.completed_at = None;
            }
            entry.segments.clear();
            entry.job.downloaded = 0;
            entry.job.etag = None;
            entry.job.last_modified = None;
            entry.job.resumable = None;
            entry.job.auto_retries = 0;
            entry.job.error = None;
            entry.job.state = JobState::Paused;
            self.inner.store.save_job_and_segments(&entry.job, &[])?;
        }
        self.start(id)
    }

    pub fn update_job(&self, id: JobId, patch: JobPatch) -> Result<()> {
        {
            let mut jobs = self.inner.jobs.lock();
            let entry = jobs.get_mut(&id).ok_or(EngineError::NotFound)?;
            let running = entry.run.is_some();
            let j = &mut entry.job;
            if j.torrent.is_some() {
                if patch.url.is_some() {
                    return Err(EngineError::Invalid("A torrent's identity cannot be changed. Add the new torrent separately.".into()));
                }
                if running && patch.speed_limit.is_some() {
                    return Err(EngineError::Invalid("Pause this torrent before changing its speed limit, then resume.".into()));
                }
                if patch.dir.is_some() && crate::torrent::staging_dir(j).exists() {
                    return Err(EngineError::Invalid(
                        "This torrent has saved pieces. Keep its destination or add a separate download in the new folder.".into(),
                    ));
                }
            }
            if let Some(limit) = patch.speed_limit {
                j.speed_limit = limit.filter(|v| *v > 0);
                if let Some(run) = &entry.run {
                    run.limiter.set_rate(j.speed_limit.unwrap_or(0));
                }
            }
            if let Some(c) = patch.max_connections {
                j.max_connections = c.map(|v| v.clamp(1, 32));
            }
            if let Some(q) = patch.queued {
                j.queued = q;
                if !q && j.state == JobState::Queued {
                    j.state = JobState::Paused;
                }
                if q && matches!(j.state, JobState::Paused | JobState::Failed) && !running {
                    j.state = JobState::Queued;
                    j.queue_order = now_ms();
                }
            }
            let changes_location = patch.file_name.is_some() || patch.dir.is_some() || patch.url.is_some();
            if changes_location && running {
                return Err(EngineError::Invalid("Pause the download before changing its name, folder or address.".into()));
            }
            if j.state == JobState::Completed && (patch.file_name.is_some() || patch.dir.is_some()) {
                return Err(EngineError::Invalid("This download has already finished.".into()));
            }
            if patch.file_name.is_some() || patch.dir.is_some() {
                let old_media_job = j.media_download.then(|| j.clone());
                let old_part = j.part_path();
                if let Some(n) = patch.file_name {
                    j.file_name = names::sanitize(&n);
                }
                if let Some(d) = patch.dir {
                    j.dir = d;
                }
                let new_part = j.part_path();
                if let Some(old) = old_media_job {
                    if old.dir != j.dir && crate::media::staging_dir(&old).exists() {
                        std::fs::create_dir_all(&j.dir)?;
                        if std::fs::rename(crate::media::staging_dir(&old), crate::media::staging_dir(j)).is_err() {
                            crate::media::cleanup(&old);
                            j.downloaded = 0;
                        }
                    }
                }
                if old_part != new_part && old_part.exists() {
                    if let Some(p) = new_part.parent() {
                        std::fs::create_dir_all(p)?;
                    }
                    if std::fs::rename(&old_part, &new_part).is_err() {
                        // Different volume: copying a huge part file is not
                        // worth it; start over at the new location.
                        let _ = std::fs::remove_file(&old_part);
                        entry.segments.clear();
                        j.downloaded = 0;
                    }
                }
            }
            if let Some(url) = patch.url {
                validate_url(&url)?;
                if j.media_download && url != j.url {
                    crate::media::cleanup(j);
                    let _ = std::fs::remove_file(j.part_path());
                    j.downloaded = 0;
                    entry.segments.clear();
                }
                // Same file, new address (e.g. a refreshed signed link). The
                // size/validator checks on resume still protect the data.
                j.url = url;
                j.final_url = None;
            }
            if let Some(h) = patch.headers {
                j.headers = h;
            }
            self.inner.store.save_job_and_segments(j, &entry.segments)?;
        }
        self.emit_job(id);
        self.pump_queue();
        Ok(())
    }

    /// Moves a job up (negative) or down (positive) in the queue order.
    pub fn reorder(&self, id: JobId, before: Option<JobId>) -> Result<()> {
        let mut jobs = self.inner.jobs.lock();
        let order = match before.and_then(|b| jobs.get(&b)).map(|e| e.job.queue_order) {
            Some(o) => o - 1,
            None => now_ms(),
        };
        let entry = jobs.get_mut(&id).ok_or(EngineError::NotFound)?;
        entry.job.queue_order = order;
        self.inner.store.save_job(&entry.job)?;
        drop(jobs);
        self.emit_job(id);
        Ok(())
    }

    // ----- queue ---------------------------------------------------------

    pub fn start_queue(&self) {
        self.inner.queue_running.store(true, Ordering::Relaxed);
        self.emit(EngineEvent::QueueChanged { running: true });
        self.pump_queue();
    }

    /// Stops the queue; queued downloads that are running go back to waiting.
    pub fn stop_queue(&self) {
        self.inner.queue_running.store(false, Ordering::Relaxed);
        self.inner.queue_did_work.store(false, Ordering::Relaxed);
        let ids: Vec<JobId> = self.inner.jobs.lock().values().filter(|e| e.job.queued && e.run.is_some()).map(|e| e.job.id).collect();
        for id in ids {
            self.stop(id, StopReason::Requeue);
        }
        self.emit(EngineEvent::QueueChanged { running: false });
    }

    fn pump_queue(&self) {
        if self.inner.shutting_down.load(Ordering::Relaxed) || !self.inner.queue_running.load(Ordering::Relaxed) {
            return;
        }
        let limit = self.inner.settings.read().max_active_downloads as usize;
        let (to_start, idle) = {
            let jobs = self.inner.jobs.lock();
            let active = jobs.values().filter(|e| e.job.queued && e.run.is_some()).count();
            let mut waiting: Vec<&Entry> =
                jobs.values().filter(|e| e.job.queued && e.job.state == JobState::Queued && e.run.is_none()).collect();
            waiting.sort_by_key(|e| (e.job.queue_order, e.job.created_at));
            let n = limit.saturating_sub(active);
            let to_start: Vec<JobId> = waiting.iter().take(n).map(|e| e.job.id).collect();
            (to_start, active == 0 && waiting.is_empty())
        };
        if !to_start.is_empty() {
            self.inner.queue_did_work.store(true, Ordering::Relaxed);
        }
        for id in to_start {
            let _ = self.start(id);
        }
        if idle && self.inner.queue_did_work.swap(false, Ordering::Relaxed) {
            self.emit(EngineEvent::QueueFinished);
        }
    }

    // ----- background ----------------------------------------------------

    fn spawn_ticker(&self) {
        let weak = Arc::downgrade(&self.inner);
        self.inner.rt.spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(500));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut n: u64 = 0;
            loop {
                tick.tick().await;
                let Some(inner) = weak.upgrade() else { break };
                let engine = Engine { inner };
                engine.emit_progress();
                n += 1;
                if n % 20 == 0 {
                    engine.inner.probes.lock().retain(|_, p| p.at.elapsed() < Duration::from_secs(90));
                    engine.check_schedule();
                }
            }
        });
    }

    fn emit_progress(&self) {
        let mut items = Vec::new();
        {
            let mut jobs = self.inner.jobs.lock();
            // All HTTP workers share one bucket. Each external media process
            // gets one equal share of the same configured global budget.
            let media = jobs.values().filter(|e| e.run.is_some() && e.job.media_download).count() as u64;
            let torrents = u64::from(jobs.values().any(|e| e.run.is_some() && e.job.torrent.is_some()));
            let http = u64::from(jobs.values().any(|e| e.run.is_some() && !e.job.media_download && e.job.torrent.is_none()));
            let total = self.inner.settings.read().speed_limit;
            let share = if total == 0 { 0 } else { (total / (media + http + torrents).max(1)).max(1) };
            self.inner.global_limit.set_rate(share);
            if let Some(session) = self.inner.torrents.get() {
                let download = crate::torrent::rate(share);
                let upload = crate::torrent::rate(self.inner.settings.read().torrents.upload_limit);
                // Updating an unchanged governor resets its token bucket and
                // would grant another burst on every progress tick.
                if session.ratelimits.get_download_bps() != download {
                    session.ratelimits.set_download_bps(download);
                }
                if session.ratelimits.get_upload_bps() != upload {
                    session.ratelimits.set_upload_bps(upload);
                }
            }
            for entry in jobs.values_mut() {
                let phase_outside = entry.phase.clone();
                let Some(run) = entry.run.as_mut() else { continue };
                let received = run.shared.received.load(Ordering::Relaxed);
                let now = Instant::now();
                let dt = now.duration_since(run.meter.last_at).as_secs_f64();
                if dt > 0.05 {
                    let inst = received.saturating_sub(run.meter.last_bytes) as f64 / dt;
                    run.meter.speed = if run.meter.speed == 0.0 { inst } else { run.meter.speed * 0.6 + inst * 0.4 };
                    if inst == 0.0 && run.meter.speed < 1024.0 {
                        run.meter.speed = 0.0;
                    }
                    run.meter.last_bytes = received;
                    run.meter.last_at = now;
                }
                let downloaded = run.shared.downloaded();
                let size = run.shared.size().or(entry.job.size);
                entry.job.downloaded = downloaded;
                let speed = run.meter.speed as u64;
                let eta = match (size, speed) {
                    (Some(s), sp) if sp > 0 && s >= downloaded => Some((s - downloaded) / sp),
                    _ => None,
                };
                items.push(Progress {
                    id: entry.job.id,
                    downloaded,
                    size,
                    speed,
                    eta,
                    connections: run.shared.connections.load(Ordering::Relaxed),
                    phase: phase_outside.or_else(|| run.shared.phase.lock().clone()),
                    segments: run.shared.segments().iter().map(|s| (s.start, s.end, s.done())).collect(),
                });
            }
        }
        if !items.is_empty() {
            self.emit(EngineEvent::Progress { items });
        }
    }

    fn check_schedule(&self) {
        let s = self.inner.settings.read().schedule.clone();
        if !s.enabled {
            return;
        }
        use chrono::{Datelike, Local, Timelike};
        let now = Local::now();
        let day_idx = now.weekday().num_days_from_monday() as usize;
        let hm = format!("{:02}:{:02}", now.hour(), now.minute());
        let today = now.format("%Y-%m-%d").to_string();
        let key = |t: &str| format!("{today} {t}");
        if s.days[day_idx] && hm == normalize_hm(&s.start_time) {
            let k = key(&s.start_time);
            let mut st = self.inner.schedule_state.lock();
            if st.last_start_fired.as_deref() != Some(&k) {
                st.last_start_fired = Some(k);
                drop(st);
                tracing::info!("schedule: starting queue");
                self.start_queue();
            }
        }
        if let Some(stop) = &s.stop_time {
            if hm == normalize_hm(stop) {
                let k = key(stop);
                let mut st = self.inner.schedule_state.lock();
                if st.last_stop_fired.as_deref() != Some(&k) {
                    st.last_stop_fired = Some(k);
                    drop(st);
                    tracing::info!("schedule: stopping queue");
                    self.stop_queue();
                }
            }
        }
    }

    fn emit_job(&self, id: JobId) {
        if let Some(s) = self.job(id) {
            self.emit(EngineEvent::JobUpdated { job: Box::new(s) });
        }
    }

    /// Stops all transfers, keeping them resumable on next start.
    pub async fn shutdown(&self) {
        self.inner.shutting_down.store(true, Ordering::Relaxed);
        let ids: Vec<JobId> = self.inner.jobs.lock().values().filter(|e| e.run.is_some()).map(|e| e.job.id).collect();
        let waiters: Vec<_> = ids.into_iter().filter_map(|id| self.stop(id, StopReason::Shutdown)).collect();
        let all = async {
            for w in waiters {
                self.wait_stopped(Some(w)).await;
            }
        };
        let _ = tokio::time::timeout(Duration::from_secs(8), all).await;
        if let Some(session) = self.inner.torrents.get() {
            session.stop().await;
        }
    }
}

fn normalize_hm(t: &str) -> String {
    let (h, m) = t.trim().split_once(':').unwrap_or(("0", "0"));
    format!("{:02}:{:02}", h.parse::<u32>().unwrap_or(0).min(23), m.parse::<u32>().unwrap_or(0).min(59))
}

/// Whether "now" falls inside the schedule's start..stop window.
fn schedule_window_open(s: &crate::settings::Schedule) -> bool {
    use chrono::{Datelike, Local, Timelike};
    let now = Local::now();
    if !s.days[now.weekday().num_days_from_monday() as usize] {
        return false;
    }
    let cur = now.hour() * 60 + now.minute();
    let parse = |t: &str| {
        let n = normalize_hm(t);
        let (h, m) = n.split_once(':').unwrap();
        h.parse::<u32>().unwrap() * 60 + m.parse::<u32>().unwrap()
    };
    let start = parse(&s.start_time);
    match &s.stop_time {
        Some(stop) => {
            let stop = parse(stop);
            if start <= stop {
                cur >= start && cur < stop
            } else {
                cur >= start || cur < stop
            }
        }
        None => cur >= start,
    }
}

fn host_of(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_ascii_lowercase)).unwrap_or_default()
}

fn validate_url(url: &str) -> Result<()> {
    let parsed = url::Url::parse(url.trim()).map_err(|_| EngineError::Invalid("That doesn't look like a valid web address.".into()))?;
    match parsed.scheme() {
        "http" | "https" => Ok(()),
        other => Err(EngineError::Invalid(format!("CrowDM can't download \"{other}:\" addresses yet; use an http or https link."))),
    }
}

fn snapshot(e: &Entry) -> JobSnapshot {
    let (speed, eta, connections, phase, segments, downloaded, size) = match &e.run {
        Some(run) => {
            let d = run.shared.downloaded();
            let size = run.shared.size().or(e.job.size);
            let sp = run.meter.speed as u64;
            let eta = match size {
                Some(s) if sp > 0 && s >= d => Some((s - d) / sp),
                _ => None,
            };
            let segs = run.shared.segments().iter().map(|s| (s.start, s.end, s.done())).collect();
            (
                sp,
                eta,
                run.shared.connections.load(Ordering::Relaxed),
                e.phase.clone().or_else(|| run.shared.phase.lock().clone()),
                segs,
                d,
                size,
            )
        }
        None => {
            (0, None, 0, e.phase.clone(), e.segments.iter().map(|s| (s.start, s.end, s.done())).collect(), e.job.downloaded, e.job.size)
        }
    };
    let mut job = e.job.clone();
    job.downloaded = downloaded;
    job.size = size;
    // Never hand credentials to the UI.
    job.headers.retain(|(k, _)| !k.eq_ignore_ascii_case("cookie") && !k.eq_ignore_ascii_case("authorization"));
    JobSnapshot { job, speed, eta, connections, phase, segments }
}

#[allow(unused)]
fn _assert_send() {
    fn is_send<T: Send + Sync>() {}
    is_send::<Engine>();
    let _ = HeaderMap::new();
}
