//! One download's transfer: probing, segmented workers, dynamic splitting,
//! single-connection fallback, durable progress and completion checks.
//!
//! Invariants:
//! * Each segment owns the byte range `[start, end)` of the target file and
//!   writes at explicit offsets; `pos` only advances after bytes are written.
//! * Persisted progress never exceeds durable data: a snapshot of the
//!   segment table is taken *before* `sync`, and only then stored.
//! * Every response is validated against the segment it serves (status,
//!   `Content-Range` start and total, validators) before any byte is written.
//! * A server that ignores ranges (`200` to a ranged request) triggers a
//!   switch to one connection that streams from offset 0.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use parking_lot::Mutex;
use reqwest::header::{self, HeaderMap};
use reqwest::{Client, Response};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::error::TransferError;
use crate::file::{self, PartFile};
use crate::http::{self, ResponseMeta};
use crate::model::{ErrorKind, Segment};
use crate::ratelimit::RateLimiter;

pub const UNKNOWN_END: u64 = u64::MAX;

/// A connection that has received nothing for this long may have the rest
/// of its segment taken over by an idle connection.
const STALL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct TransferConfig {
    pub reject_html: bool,
    pub max_connections: usize,
    /// Segments are only split when both halves would be at least this big.
    pub min_split: u64,
    /// Consecutive failures without progress before a connection gives up.
    pub connection_retries: u32,
    pub buffer_size: usize,
    pub flush_interval: Duration,
}

impl Default for TransferConfig {
    fn default() -> Self {
        TransferConfig {
            reject_html: false,
            max_connections: 8,
            min_split: 1024 * 1024,
            connection_retries: 6,
            buffer_size: 256 * 1024,
            flush_interval: Duration::from_secs(3),
        }
    }
}

/// Validators of the remote resource, compared on every response.
#[derive(Debug, Clone, Default)]
pub struct Identity {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Debug)]
struct Slot {
    id: u64,
    seg: Segment,
    active: bool,
    /// The connection serving this slot is a full `200` body from offset 0.
    full_body: bool,
    /// Last time data arrived for this slot (or it was assigned).
    last_progress: std::time::Instant,
}

#[derive(Debug)]
struct Inner {
    slots: Vec<Slot>,
    next_id: u64,
    ranges: bool,
    size: Option<u64>,
    target_conns: usize,
}

impl Inner {
    fn find(&mut self, id: u64) -> Option<&mut Slot> {
        self.slots.iter_mut().find(|s| s.id == id)
    }
    fn new_slot(&mut self, seg: Segment, active: bool, full_body: bool) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.slots.push(Slot { id, seg, active, full_body, last_progress: std::time::Instant::now() });
        id
    }
    fn active_count(&self) -> usize {
        self.slots.iter().filter(|s| s.active).count()
    }
}

/// Live state of a transfer, shared with the engine for progress reporting.
pub struct Shared {
    inner: Mutex<Inner>,
    /// Bytes received from the network during this run (for speed).
    pub received: AtomicU64,
    pub connections: AtomicU32,
    pub phase: Mutex<Option<String>>,
    /// Connection count the server accepted after refusing more (0 = none).
    pub limited_to: std::sync::atomic::AtomicUsize,
}

impl Shared {
    pub fn new(segments: &[Segment], size: Option<u64>, ranges: bool, max_conns: usize) -> Arc<Self> {
        let mut inner = Inner { slots: vec![], next_id: 1, ranges, size, target_conns: max_conns.max(1) };
        for s in segments {
            inner.new_slot(*s, false, false);
        }
        Arc::new(Shared {
            inner: Mutex::new(inner),
            received: AtomicU64::new(0),
            connections: AtomicU32::new(0),
            phase: Mutex::new(Some("Connecting".into())),
            limited_to: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    pub fn segments(&self) -> Vec<Segment> {
        let g = self.inner.lock();
        let mut v: Vec<Segment> = g.slots.iter().map(|s| s.seg).collect();
        v.sort_by_key(|s| s.start);
        v
    }

    pub fn downloaded(&self) -> u64 {
        self.inner.lock().slots.iter().map(|s| s.seg.done()).sum()
    }

    pub fn size(&self) -> Option<u64> {
        self.inner.lock().size
    }

    pub fn set_phase(&self, p: Option<&str>) {
        *self.phase.lock() = p.map(str::to_string);
    }

    pub(crate) fn media_progress(&self, downloaded: u64, size: Option<u64>) {
        let mut g = self.inner.lock();
        g.size = size;
        g.slots.clear();
        g.new_slot(Segment { start: 0, end: size.unwrap_or(UNKNOWN_END), pos: downloaded }, true, false);
    }

    fn is_complete(&self) -> bool {
        let g = self.inner.lock();
        !g.slots.is_empty() && g.slots.iter().all(|s| s.seg.is_complete())
    }
}

/// Callbacks into the engine.
pub struct Hooks {
    /// Response metadata learned from the first successful response. May
    /// return a new part-file path when the server names the file.
    pub on_meta: Box<dyn Fn(&ResponseMeta, Option<u64>, bool) -> Option<PathBuf> + Send + Sync>,
    /// Durable progress checkpoint.
    pub persist: Box<dyn Fn(Vec<Segment>) + Send + Sync>,
}

pub struct TransferInput {
    pub url: String,
    pub headers: HeaderMap,
    pub part_path: PathBuf,
    pub size: Option<u64>,
    pub resumable: Option<bool>,
    pub identity: Identity,
    /// Persisted progress; empty for a fresh start.
    pub segments: Vec<Segment>,
    /// A response opened earlier (e.g. while the confirm dialog was shown).
    pub pending: Option<(Response, ResponseMeta)>,
}

pub struct Limits {
    pub global: Arc<RateLimiter>,
    pub job: Arc<RateLimiter>,
    pub connections: Arc<Semaphore>,
}

#[derive(Debug)]
pub enum Stop {
    Canceled,
    Failed(TransferError),
}

impl From<TransferError> for Stop {
    fn from(e: TransferError) -> Self {
        Stop::Failed(e)
    }
}

pub struct Completed {
    pub size: u64,
    pub file: PartFile,
}

struct Ctx {
    client: Client,
    url: Mutex<String>,
    headers: HeaderMap,
    identity: Mutex<Identity>,
    file: PartFile,
    shared: Arc<Shared>,
    limits: Limits,
    cancel: CancellationToken,
    cfg: TransferConfig,
    fatal: Mutex<Option<TransferError>>,
    last_error: Mutex<Option<TransferError>>,
}

impl Ctx {
    fn fail(&self, e: TransferError) {
        let mut f = self.fatal.lock();
        if f.is_none() {
            *f = Some(e);
        }
        self.cancel.cancel();
    }
}

/// Runs a transfer to completion, cancellation or failure.
pub async fn run(
    client: Client,
    mut input: TransferInput,
    shared: Arc<Shared>,
    limits: Limits,
    cfg: TransferConfig,
    cancel: CancellationToken,
    hooks: Hooks,
) -> Result<Completed, Stop> {
    let cancel = cancel.child_token();
    let resumable = input.resumable.unwrap_or(false);
    let resume_possible = !input.segments.is_empty()
        && resumable
        && input.size.is_some()
        && input.part_path.exists()
        && std::fs::metadata(&input.part_path).map(|m| Some(m.len()) == input.size).unwrap_or(false)
        && input.segments.iter().any(|s| s.pos > s.start);

    let (file, first) = if resume_possible {
        let f = PartFile::open_existing(&input.part_path).map_err(|e| TransferError::io("Could not open the partial file", e))?;
        {
            let mut g = shared.inner.lock();
            g.slots.clear();
            g.ranges = true;
            g.size = input.size;
            for s in &input.segments {
                g.new_slot(*s, false, false);
            }
        }
        // The pending response (if any) starts at 0 and is not useful here.
        drop(input.pending.take());
        (f, None)
    } else {
        // Fresh start: open (or reuse) the first response.
        let (resp, meta) = match input.pending.take() {
            Some(p) => p,
            None => open_first(&client, &input, &cancel).await?,
        };
        if cfg.reject_html && meta.looks_like_html() {
            return Err(TransferError::new(ErrorKind::Http, "The server returned a web page instead of a file. Use the video button for streaming media, or let the browser download this link.").into());
        }
        let size = meta.total_size();
        let full_body = meta.status == 200;
        let ranges = size.is_some() && (meta.status == 206 || meta.accept_ranges) && meta.content_encoding.is_none();
        if meta.status == 206 {
            if let Some(cr) = meta.content_range {
                if cr.start != 0 {
                    return Err(Stop::Failed(TransferError::new(ErrorKind::Http, "The server returned an unexpected byte range.")));
                }
            }
        }
        // Starting over: the server's current answer defines the resource.
        if let Some(p) = (hooks.on_meta)(&meta, size, ranges) {
            input.part_path = p;
        }
        input.identity = Identity { etag: meta.etag.clone(), last_modified: meta.last_modified.clone() };
        if let (Some(size), Some(dir)) = (size, input.part_path.parent()) {
            if let Some(free) = file::available_space(dir) {
                if free < size {
                    return Err(Stop::Failed(TransferError::new(
                        ErrorKind::DiskFull,
                        format!(
                            "Not enough free space: the file needs {} but the drive has {} available.",
                            fmt_bytes(size),
                            fmt_bytes(free)
                        ),
                    )));
                }
            }
        }
        let f = PartFile::create(&input.part_path, size).map_err(|e| TransferError::io("Could not create the download file", e))?;
        let first_id = {
            let mut g = shared.inner.lock();
            g.slots.clear();
            g.ranges = ranges;
            g.size = size;
            g.new_slot(Segment { start: 0, end: size.unwrap_or(UNKNOWN_END), pos: 0 }, true, full_body)
        };
        input.url = meta.final_url.clone();
        (f, Some((first_id, resp, meta)))
    };

    let ctx = Arc::new(Ctx {
        client,
        url: Mutex::new(input.url.clone()),
        headers: input.headers.clone(),
        identity: Mutex::new(input.identity.clone()),
        file: file.clone(),
        shared: shared.clone(),
        limits,
        cfg,
        cancel: cancel.clone(),
        fatal: Mutex::new(None),
        last_error: Mutex::new(None),
    });

    // Zero-length resources are complete as soon as they are created.
    if shared.size() == Some(0) {
        (hooks.persist)(shared.segments());
        return Ok(Completed { size: 0, file });
    }

    // Periodic durable checkpoints.
    let flush_done = CancellationToken::new();
    let flusher = {
        let ctx = ctx.clone();
        let done = flush_done.clone();
        let persist = Arc::new(hooks.persist);
        let persist2 = persist.clone();
        let handle = tokio::spawn(async move {
            let mut tick = tokio::time::interval(ctx.cfg.flush_interval);
            tick.tick().await;
            loop {
                tokio::select! {
                    _ = tick.tick() => {}
                    _ = done.cancelled() => break,
                }
                checkpoint(&ctx, &**persist).await;
            }
        });
        (handle, persist2)
    };

    let mut first = first;
    let mut rounds_without_progress = 0u32;
    loop {
        let before = shared.downloaded();
        let mut set = JoinSet::new();
        if let Some((slot, resp, meta)) = first.take() {
            let c = ctx.clone();
            set.spawn(async move { worker(c, Some((slot, resp, meta))).await });
        }
        let target = { shared.inner.lock().target_conns };
        while set.len() < target {
            if !ctx.shared.inner.lock().ranges && !set.is_empty() {
                break;
            }
            let c = ctx.clone();
            set.spawn(async move { worker(c, None).await });
            // Stagger new connections slightly so a server that caps
            // connections answers the earlier ones first.
            if set.len() < target {
                tokio::time::sleep(Duration::from_millis(15)).await;
            }
        }
        while set.join_next().await.is_some() {
            // Connections that were taken over may still be waiting on a
            // server that never answers; once every byte is in, drop them.
            if shared.is_complete() {
                set.abort_all();
            }
        }

        if ctx.cancel.is_cancelled() {
            break;
        }
        if shared.is_complete() {
            break;
        }
        // Workers gave up (server limits, errors) with work left: try again
        // with fresh connections unless nothing moved for a while.
        if shared.downloaded() > before {
            rounds_without_progress = 0;
        } else {
            rounds_without_progress += 1;
        }
        if rounds_without_progress >= 3 {
            let e = ctx
                .last_error
                .lock()
                .clone()
                .unwrap_or_else(|| TransferError::new(ErrorKind::Network, "The download stopped making progress."));
            ctx.fail(e);
            break;
        }
        {
            let mut g = shared.inner.lock();
            for s in g.slots.iter_mut() {
                s.active = false;
            }
            g.target_conns = g.target_conns.max(1);
        }
        if sleep_or_cancel(&ctx.cancel, Duration::from_secs(2)).await.is_err() {
            break;
        }
    }

    flush_done.cancel();
    let (flush_handle, persist) = flusher;
    let _ = flush_handle.await;
    shared.set_phase(Some("Saving progress"));
    checkpoint(&ctx, &**persist).await;

    if let Some(e) = ctx.fatal.lock().take() {
        return Err(Stop::Failed(e));
    }
    if !shared.is_complete() {
        return Err(Stop::Canceled);
    }

    // Unknown-length downloads: the file is exactly as long as what arrived.
    let final_size = {
        let g = shared.inner.lock();
        match g.size {
            Some(s) => s,
            None => g.slots.iter().map(|s| s.seg.pos).max().unwrap_or(0),
        }
    };
    let f = file.clone();
    let len = tokio::task::spawn_blocking(move || -> std::io::Result<u64> {
        if f.len()? != final_size {
            f.set_len(final_size)?;
        }
        f.sync()?;
        f.len()
    })
    .await
    .expect("sync task")
    .map_err(|e| TransferError::io("Could not finish writing the file", e))?;
    if len != final_size {
        return Err(Stop::Failed(TransferError::new(ErrorKind::Io, "The saved file has the wrong length.")));
    }
    Ok(Completed { size: final_size, file })
}

async fn checkpoint(ctx: &Ctx, persist: &(dyn Fn(Vec<Segment>) + Send + Sync)) {
    // Snapshot first, then sync: everything in the snapshot is on disk
    // once `sync` returns.
    let snapshot = ctx.shared.segments();
    let f = ctx.file.clone();
    let synced = tokio::task::spawn_blocking(move || f.sync()).await;
    if matches!(synced, Ok(Ok(()))) {
        persist(compact(snapshot));
    }
}

/// Merges adjacent completed segments to keep the stored table small.
fn compact(mut segs: Vec<Segment>) -> Vec<Segment> {
    segs.sort_by_key(|s| s.start);
    let mut out: Vec<Segment> = Vec::with_capacity(segs.len());
    for s in segs {
        if let Some(last) = out.last_mut() {
            if last.is_complete() && s.is_complete() && last.end == s.start {
                last.end = s.end;
                last.pos = s.end;
                continue;
            }
        }
        out.push(s);
    }
    out
}

async fn sleep_or_cancel(cancel: &CancellationToken, d: Duration) -> Result<(), Stop> {
    tokio::select! {
        _ = tokio::time::sleep(d) => Ok(()),
        _ = cancel.cancelled() => Err(Stop::Canceled),
    }
}

async fn open_first(client: &Client, input: &TransferInput, cancel: &CancellationToken) -> Result<(Response, ResponseMeta), Stop> {
    let mut attempt = 0u32;
    loop {
        let rb = client.get(&input.url).headers(input.headers.clone()).header(header::RANGE, "bytes=0-");
        let res = tokio::select! {
            r = rb.send() => r,
            _ = cancel.cancelled() => return Err(Stop::Canceled),
        };
        let err = match res {
            Ok(resp) if resp.status().is_success() => {
                let meta = ResponseMeta::from_response(&resp);
                return Ok((resp, meta));
            }
            Ok(resp) => http::status_error(&resp),
            Err(e) => http::request_error(e),
        };
        attempt += 1;
        if !err.kind.is_retryable() || attempt >= 4 {
            return Err(Stop::Failed(err));
        }
        let delay = err.retry_after.unwrap_or(Duration::from_secs(1 << attempt.min(4)));
        sleep_or_cancel(cancel, delay).await?;
    }
}

enum SegEnd {
    /// The slot is complete (or gone); look for more work.
    Done,
    /// Stop this connection; others continue.
    Released,
    Stop,
}

enum Work {
    Slot(u64),
    None,
}

fn acquire_work(ctx: &Ctx) -> Work {
    let mut g = ctx.shared.inner.lock();
    if g.active_count() >= g.target_conns {
        return Work::None;
    }
    if let Some(s) = g.slots.iter_mut().find(|s| !s.active && !s.seg.is_complete()) {
        s.active = true;
        s.last_progress = std::time::Instant::now();
        return Work::Slot(s.id);
    }
    if !g.ranges || g.size.is_none() {
        return Work::None;
    }
    // Take over the rest of a segment whose connection has stalled (a server
    // that never answers an extra connection, or a connection that froze).
    if let Some(i) = g
        .slots
        .iter()
        .enumerate()
        .filter(|(_, s)| s.active && !s.seg.is_complete() && s.last_progress.elapsed() > STALL)
        .max_by_key(|(_, s)| s.seg.remaining())
        .map(|(i, _)| i)
    {
        let seg = &mut g.slots[i].seg;
        let (from, to) = (seg.pos, seg.end);
        // The stalled connection's segment ends where it stopped; anything
        // it still delivers is discarded by `write_out`.
        seg.end = seg.pos;
        tracing::debug!("taking over a stalled segment at {from}");
        let id = g.new_slot(Segment { start: from, end: to, pos: from }, true, false);
        return Work::Slot(id);
    }
    // Split the active segment with the most work left.
    let min_split = ctx.cfg.min_split;
    let best = g
        .slots
        .iter()
        .enumerate()
        .filter(|(_, s)| s.active && !s.seg.is_complete())
        .max_by_key(|(_, s)| s.seg.remaining())
        .map(|(i, s)| (i, s.seg.remaining()));
    let Some((i, rem)) = best else { return Work::None };
    if rem < min_split * 2 {
        return Work::None;
    }
    let seg = &mut g.slots[i].seg;
    let mid = seg.pos + rem / 2;
    let old_end = seg.end;
    seg.end = mid;
    let id = g.new_slot(Segment { start: mid, end: old_end, pos: mid }, true, false);
    Work::Slot(id)
}

fn release(ctx: &Ctx, slot: u64) {
    if let Some(s) = ctx.shared.inner.lock().find(slot) {
        s.active = false;
    }
}

async fn worker(ctx: Arc<Ctx>, mut initial: Option<(u64, Response, ResponseMeta)>) {
    loop {
        if ctx.cancel.is_cancelled() {
            return;
        }
        let (slot, pending) = match initial.take() {
            Some((slot, resp, meta)) => (slot, Some((resp, meta))),
            None => match acquire_work(&ctx) {
                Work::Slot(s) => (s, None),
                Work::None => {
                    // Nothing to take right now. While other connections are
                    // still working, stay around: one of them may stall and
                    // need to be taken over.
                    let others_busy = {
                        let g = ctx.shared.inner.lock();
                        g.ranges && g.slots.iter().any(|s| s.active && !s.seg.is_complete())
                    };
                    if !others_busy || sleep_or_cancel(&ctx.cancel, Duration::from_secs(1)).await.is_err() {
                        return;
                    }
                    continue;
                }
            },
        };
        let mut slot = slot;
        match serve_slot(&ctx, &mut slot, pending).await {
            SegEnd::Done => {
                release(&ctx, slot);
                continue;
            }
            SegEnd::Released => {
                release(&ctx, slot);
                return;
            }
            SegEnd::Stop => {
                release(&ctx, slot);
                return;
            }
        }
    }
}

/// What a response means for the slot that requested it.
enum Verdict {
    /// Stream the body into the slot.
    Stream,
    /// Slot is already complete.
    Complete,
    /// Server ignores ranges; this body is a full copy from byte 0.
    FullBody,
}

fn judge(ctx: &Ctx, slot_pos: u64, ranged: bool, meta: &ResponseMeta) -> Result<Verdict, TransferError> {
    if ctx.cfg.reject_html && meta.looks_like_html() {
        return Err(TransferError::new(ErrorKind::Http, "The server returned a web page instead of the downloaded file."));
    }
    let size = ctx.shared.size();
    // Validator checks.
    {
        let id = ctx.identity.lock();
        if let (Some(a), Some(b)) = (id.etag.as_deref(), meta.etag.as_deref()) {
            if strip_weak(a) != strip_weak(b) {
                return Err(TransferError::changed("ETag differs"));
            }
        }
        if id.etag.is_none() {
            if let (Some(a), Some(b)) = (id.last_modified.as_deref(), meta.last_modified.as_deref()) {
                if a != b {
                    return Err(TransferError::changed("modification date differs"));
                }
            }
        }
    }
    match meta.status {
        206 => {
            let cr = meta
                .content_range
                .ok_or_else(|| TransferError::new(ErrorKind::Http, "The server sent a partial response without Content-Range."))?;
            if let (Some(expected), Some(total)) = (size, cr.total) {
                if expected != total {
                    return Err(TransferError::changed("size differs"));
                }
            }
            if cr.start != slot_pos {
                return Err(TransferError::new(
                    ErrorKind::Network,
                    format!("The server returned bytes from {} instead of {}.", cr.start, slot_pos),
                ));
            }
            Ok(Verdict::Stream)
        }
        200 => {
            if let (Some(expected), Some(total)) = (size, meta.total_size()) {
                if expected != total {
                    return Err(TransferError::changed("size differs"));
                }
            }
            if ranged && slot_pos > 0 {
                Ok(Verdict::FullBody)
            } else {
                Ok(Verdict::Stream)
            }
        }
        416 => {
            if size.is_some_and(|s| slot_pos >= s) {
                Ok(Verdict::Complete)
            } else {
                Err(TransferError::changed("requested range is no longer available"))
            }
        }
        _ => Err(TransferError::new(ErrorKind::Http, format!("Unexpected server response (HTTP {}).", meta.status))),
    }
}

fn strip_weak(e: &str) -> &str {
    e.strip_prefix("W/").unwrap_or(e)
}

/// Switch to a single connection streaming from byte 0. Returns the slot
/// this worker should now serve, or `None` if another connection already
/// carries a full body (this worker then stops).
fn fall_back_to_single(ctx: &Ctx, my_slot: u64) -> Option<u64> {
    let mut g = ctx.shared.inner.lock();
    g.ranges = false;
    g.target_conns = 1;
    let size_end = g.size.unwrap_or(UNKNOWN_END);
    if let Some(keep) = g.slots.iter().position(|s| s.id != my_slot && s.active && s.full_body && s.seg.start == 0) {
        let mut slot = g.slots.swap_remove(keep);
        slot.seg.end = size_end;
        g.slots.clear();
        g.slots.push(slot);
        return None;
    }
    g.slots.clear();
    let id = g.new_slot(Segment { start: 0, end: size_end, pos: 0 }, true, true);
    Some(id)
}

struct ConnGuard<'a>(&'a AtomicU32);
impl<'a> ConnGuard<'a> {
    fn new(c: &'a AtomicU32) -> Self {
        c.fetch_add(1, Ordering::Relaxed);
        ConnGuard(c)
    }
}
impl Drop for ConnGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

async fn serve_slot(ctx: &Arc<Ctx>, slot_ref: &mut u64, mut pending: Option<(Response, ResponseMeta)>) -> SegEnd {
    // The slot can change while streaming (continuing into the next
    // segment); the caller must release the one we end on.
    let mut slot = *slot_ref;
    macro_rules! ret {
        ($e:expr) => {{
            *slot_ref = slot;
            return $e;
        }};
    }
    let mut failures = 0u32;
    let mut buf: Vec<u8> = Vec::with_capacity(ctx.cfg.buffer_size);
    loop {
        if ctx.cancel.is_cancelled() {
            ret!(SegEnd::Stop);
        }
        // Current slot state.
        let (pos, end, ranges, others_active) = {
            let mut g = ctx.shared.inner.lock();
            let ranges = g.ranges;
            let size = g.size;
            let others = g.active_count().saturating_sub(1);
            let Some(s) = g.find(slot) else { ret!(SegEnd::Done) };
            if s.seg.is_complete() {
                ret!(SegEnd::Done);
            }
            // Without range support a retry must start over from byte 0.
            if !ranges && s.seg.pos > 0 && pending.is_none() {
                s.seg.pos = 0;
                s.seg.start = 0;
                if size.is_none() {
                    let _ = ctx.file.set_len(0);
                }
            }
            (s.seg.pos, s.seg.end, ranges, others)
        };

        let permit = tokio::select! {
            p = ctx.limits.connections.clone().acquire_owned() => p.ok(),
            _ = ctx.cancel.cancelled() => ret!(SegEnd::Stop),
        };
        let Some(_permit) = permit else { ret!(SegEnd::Stop) };

        let ranged;
        let (resp, meta) = match pending.take() {
            Some(p) => {
                ranged = true;
                p
            }
            None => {
                ranged = ranges && (pos > 0 || end != UNKNOWN_END);
                match request(ctx, pos, end, ranged).await {
                    Ok(Some(p)) => p,
                    Ok(None) => ret!(SegEnd::Stop),
                    Err(e) => {
                        // A connection beyond the first refused by the server:
                        // lower the connection target instead of failing.
                        let _ = others_active;
                        if e.status.is_some_and(|s| s == 429 || s == 503) {
                            let mut g = ctx.shared.inner.lock();
                            let others_now = g.active_count().saturating_sub(1);
                            if others_now > 0 {
                                // The server wants fewer connections: settle on
                                // the ones that work and remember the limit.
                                g.target_conns = others_now;
                                ctx.shared.limited_to.store(others_now, Ordering::Relaxed);
                                drop(g);
                                *ctx.last_error.lock() = Some(e);
                                ret!(SegEnd::Released);
                            }
                        }
                        if !e.kind.is_retryable() {
                            ctx.fail(e);
                            ret!(SegEnd::Stop);
                        }
                        failures += 1;
                        match retry_or_give_up(ctx, e, failures, others_active).await {
                            Some(end) => ret!(end),
                            None => continue,
                        }
                    }
                }
            }
        };

        tracing::debug!(slot, pos, end, status = meta.status, "response");
        let verdict = match judge(ctx, pos, ranged, &meta) {
            Ok(v) => v,
            Err(e) => {
                if e.kind.is_retryable() {
                    failures += 1;
                    match retry_or_give_up(ctx, e, failures, others_active).await {
                        Some(end) => ret!(end),
                        None => continue,
                    }
                }
                ctx.fail(e);
                ret!(SegEnd::Stop);
            }
        };
        match verdict {
            Verdict::Complete => {
                if let Some(s) = ctx.shared.inner.lock().find(slot) {
                    s.seg.pos = s.seg.end;
                }
                ret!(SegEnd::Done);
            }
            Verdict::FullBody => match fall_back_to_single(ctx, slot) {
                Some(new_slot) => {
                    tracing::info!("server ignored byte ranges; continuing with one connection");
                    slot = new_slot;
                }
                None => ret!(SegEnd::Released),
            },
            Verdict::Stream => {}
        }

        // Stream the body.
        let _conn = ConnGuard::new(&ctx.shared.connections);
        ctx.shared.set_phase(None);
        let mut stream = resp.bytes_stream();
        let mut got = 0u64;
        let mut last_touch = std::time::Instant::now() - Duration::from_secs(1);
        let outcome: Result<bool, TransferError> = loop {
            let next = tokio::select! {
                n = stream.next() => n,
                _ = ctx.cancel.cancelled() => {
                    // Keep what already arrived.
                    let _ = write_out(ctx, slot, &mut buf).await;
                    ret!(SegEnd::Stop);
                }
            };
            match next {
                Some(Ok(chunk)) => {
                    let n = chunk.len();
                    got += n as u64;
                    if last_touch.elapsed() > Duration::from_millis(250) {
                        last_touch = std::time::Instant::now();
                        if let Some(s) = ctx.shared.inner.lock().find(slot) {
                            s.last_progress = last_touch;
                        }
                    }
                    ctx.shared.received.fetch_add(n as u64, Ordering::Relaxed);
                    buf.extend_from_slice(&chunk);
                    if buf.len() >= ctx.cfg.buffer_size {
                        match flush(ctx, &mut slot, &mut buf).await {
                            Ok(WriteState::More) => {}
                            Ok(WriteState::SlotDone) => break Ok(true),
                            Ok(WriteState::SlotGone) => ret!(SegEnd::Done),
                            Err(e) => {
                                ctx.fail(e);
                                ret!(SegEnd::Stop);
                            }
                        }
                    }
                    ctx.limits.job.acquire(n).await;
                    ctx.limits.global.acquire(n).await;
                }
                Some(Err(e)) => {
                    let werr = flush(ctx, &mut slot, &mut buf).await;
                    if let Err(e) = werr {
                        ctx.fail(e);
                        ret!(SegEnd::Stop);
                    }
                    break Err(http::request_error(e));
                }
                None => {
                    match flush(ctx, &mut slot, &mut buf).await {
                        Ok(WriteState::SlotGone) => ret!(SegEnd::Done),
                        Ok(_) => {}
                        Err(e) => {
                            ctx.fail(e);
                            ret!(SegEnd::Stop);
                        }
                    }
                    break Ok(false);
                }
            }
        };
        drop(stream);

        tracing::debug!(slot, got, "stream ended: {:?}", outcome.as_ref().map_err(|e| &e.message));
        match outcome {
            Ok(true) => ret!(SegEnd::Done),
            Ok(false) => {
                // Clean end of body.
                let finished = {
                    let mut g = ctx.shared.inner.lock();
                    let unknown = g.size.is_none();
                    match g.find(slot) {
                        None => true,
                        Some(s) if s.seg.is_complete() => true,
                        Some(s) if unknown && s.seg.end == UNKNOWN_END => {
                            // Unknown length: the end of the body is the end of the file.
                            s.seg.end = s.seg.pos;
                            let p = s.seg.pos;
                            g.size = Some(p);
                            true
                        }
                        Some(_) => false,
                    }
                };
                if finished {
                    ret!(SegEnd::Done);
                }
                // Body ended early: resume from where it stopped.
                let e = TransferError::new(ErrorKind::Network, "The server closed the connection early.");
                if got > 0 {
                    failures = 0;
                }
                failures += 1;
                match retry_or_give_up(ctx, e, failures, others_active).await {
                    Some(end) => ret!(end),
                    None => continue,
                }
            }
            Err(e) => {
                if got > 0 {
                    failures = 0;
                }
                failures += 1;
                match retry_or_give_up(ctx, e, failures, others_active).await {
                    Some(end) => ret!(end),
                    None => continue,
                }
            }
        }
    }
}

/// Backs off before a retry. Returns `Some` when the connection should end.
async fn retry_or_give_up(ctx: &Ctx, e: TransferError, failures: u32, others_active: usize) -> Option<SegEnd> {
    tracing::debug!("connection error (attempt {failures}, status {:?}): {}", e.status, e.message);
    let delay = e.retry_after.unwrap_or_else(|| Duration::from_millis(500 * (1u64 << failures.min(6))).min(Duration::from_secs(30)));
    *ctx.last_error.lock() = Some(e.clone());
    if failures > ctx.cfg.connection_retries {
        if others_active > 0 {
            return Some(SegEnd::Released);
        }
        ctx.fail(e);
        return Some(SegEnd::Stop);
    }
    ctx.shared.set_phase(Some(&format!("Retrying ({failures}/{})", ctx.cfg.connection_retries)));
    if sleep_or_cancel(&ctx.cancel, delay).await.is_err() {
        return Some(SegEnd::Stop);
    }
    None
}

async fn request(ctx: &Ctx, pos: u64, end: u64, ranged: bool) -> Result<Option<(Response, ResponseMeta)>, TransferError> {
    let url = ctx.url.lock().clone();
    let mut rb = ctx.client.get(&url).headers(ctx.headers.clone());
    if ranged {
        // Open-ended ranges let one connection continue into the following
        // segment when that segment has no connection of its own (servers
        // that limit connections), instead of opening a new one.
        let _ = end;
        rb = rb.header(header::RANGE, format!("bytes={pos}-"));
        let validator = {
            let id = ctx.identity.lock();
            http::if_range_value(id.etag.as_deref(), id.last_modified.as_deref())
        };
        if let Some(v) = validator {
            rb = rb.header(header::IF_RANGE, v);
        }
    }
    let res = tokio::select! {
        r = rb.send() => r,
        _ = ctx.cancel.cancelled() => return Ok(None),
    };
    match res {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() || status.as_u16() == 416 {
                let meta = ResponseMeta::from_response(&resp);
                Ok(Some((resp, meta)))
            } else {
                Err(http::status_error(&resp))
            }
        }
        Err(e) => Err(http::request_error(e)),
    }
}

/// Writes the buffer; when the segment fills up and the next segment has
/// no connection, continues into it on this connection (`slot` changes).
async fn flush(ctx: &Ctx, slot: &mut u64, buf: &mut Vec<u8>) -> Result<WriteState, TransferError> {
    loop {
        match write_out(ctx, *slot, buf).await? {
            WriteState::SlotDone => match adopt_next(ctx, *slot) {
                Some(next) => {
                    *slot = next;
                    if buf.is_empty() {
                        return Ok(WriteState::More);
                    }
                }
                None => {
                    buf.clear();
                    return Ok(WriteState::SlotDone);
                }
            },
            other => {
                if !matches!(other, WriteState::More) {
                    buf.clear();
                }
                return Ok(other);
            }
        }
    }
}

/// Claims the idle, untouched segment that starts where `slot` ends.
fn adopt_next(ctx: &Ctx, slot: u64) -> Option<u64> {
    let mut g = ctx.shared.inner.lock();
    let end = g.find(slot)?.seg.end;
    let next = g.slots.iter_mut().find(|s| !s.active && s.seg.start == end && s.seg.pos == s.seg.start && !s.seg.is_complete())?;
    next.active = true;
    next.last_progress = std::time::Instant::now();
    let id = next.id;
    if let Some(cur) = g.find(slot) {
        cur.active = false;
    }
    Some(id)
}

enum WriteState {
    More,
    SlotDone,
    SlotGone,
}

/// Writes the buffered bytes at the slot's position and advances it.
async fn write_out(ctx: &Ctx, slot: u64, buf: &mut Vec<u8>) -> Result<WriteState, TransferError> {
    if buf.is_empty() {
        let mut g = ctx.shared.inner.lock();
        return Ok(match g.find(slot) {
            None => WriteState::SlotGone,
            Some(s) if s.seg.is_complete() => WriteState::SlotDone,
            Some(_) => WriteState::More,
        });
    }
    let (offset, allowed) = {
        let mut g = ctx.shared.inner.lock();
        let Some(s) = g.find(slot) else {
            buf.clear();
            return Ok(WriteState::SlotGone);
        };
        (s.seg.pos, s.seg.remaining().min(buf.len() as u64) as usize)
    };
    if allowed > 0 {
        let data = std::mem::take(buf);
        let f = ctx.file.clone();
        let data = tokio::task::spawn_blocking(move || f.write_all_at(&data[..allowed], offset).map(|_| data))
            .await
            .expect("write task")
            .map_err(|e| TransferError::io("Could not write to the download file", e))?;
        *buf = data;
    }
    // Bytes past the end of this segment stay buffered: the caller may
    // continue into the adjacent segment on the same connection.
    buf.drain(..allowed);
    let mut g = ctx.shared.inner.lock();
    let Some(s) = g.find(slot) else { return Ok(WriteState::SlotGone) };
    if s.seg.pos == offset {
        s.seg.pos = offset + allowed as u64;
    }
    Ok(if s.seg.is_complete() { WriteState::SlotDone } else { WriteState::More })
}

pub fn fmt_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_merges_completed_neighbours() {
        let segs =
            vec![Segment { start: 10, end: 20, pos: 20 }, Segment { start: 0, end: 10, pos: 10 }, Segment { start: 20, end: 30, pos: 25 }];
        let c = compact(segs);
        assert_eq!(c, vec![Segment { start: 0, end: 20, pos: 20 }, Segment { start: 20, end: 30, pos: 25 }]);
    }
}
