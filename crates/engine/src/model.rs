//! Persistent and UI-facing data types.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type JobId = Uuid;

/// Lifecycle state of a download job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobState {
    /// Waiting in the queue; started automatically while the queue runs.
    Queued,
    /// Actively transferring (includes connecting, verifying and finalizing).
    Downloading,
    /// Stopped by the user or by a scheduler stop time.
    Paused,
    Completed,
    /// Stopped because of an error; see [`Job::error`].
    Failed,
}

impl JobState {
    pub fn is_active(self) -> bool {
        matches!(self, JobState::Downloading)
    }
    pub fn is_finished(self) -> bool {
        matches!(self, JobState::Completed)
    }
}

/// Where a job came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SourceKind {
    /// Automatically captured from a browser download.
    BrowserCapture,
    /// "Download with CrowDM" context-menu action.
    BrowserMenu,
    /// "Download this video" overlay.
    Video,
    /// URL typed or pasted into the app.
    #[default]
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobSource {
    pub kind: SourceKind,
    /// Page the download was started from, when known.
    pub page_url: Option<String>,
    /// Browser name ("chrome", "edge", "firefox"), when captured.
    pub browser: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChecksumAlgo {
    Md5,
    Sha1,
    Sha256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Checksum {
    pub algo: ChecksumAlgo,
    /// Expected lowercase hex digest.
    pub expected: String,
}

/// Machine-readable error category, used by the UI and by retry logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    /// Connection problems, timeouts, truncated bodies. Retryable.
    Network,
    /// A server error (5xx, 429...). Retryable.
    ServerBusy,
    /// 401/403/407: expired or authenticated link.
    AccessDenied,
    /// 404/410.
    NotFound,
    /// Another unexpected HTTP status.
    Http,
    /// The server's file no longer matches the partially downloaded one.
    ResourceChanged,
    /// Not enough free space on the destination drive.
    DiskFull,
    /// Local file-system failure.
    Io,
    /// The completed file did not match the expected checksum.
    ChecksumMismatch,
    /// The URL is malformed or uses an unsupported scheme.
    InvalidUrl,
}

impl ErrorKind {
    pub fn is_retryable(self) -> bool {
        matches!(self, ErrorKind::Network | ErrorKind::ServerBusy)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobError {
    pub kind: ErrorKind,
    pub message: String,
}

/// A byte range of the target file. `end` is exclusive; `u64::MAX` means
/// "until the end of an unknown-length body". `pos` is the next byte to be
/// written, so `pos - start` bytes are durably owned by this segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub start: u64,
    pub end: u64,
    pub pos: u64,
}

impl Segment {
    pub fn done(&self) -> u64 {
        self.pos.min(self.end).saturating_sub(self.start)
    }
    pub fn remaining(&self) -> u64 {
        self.end.saturating_sub(self.pos)
    }
    pub fn is_complete(&self) -> bool {
        self.pos >= self.end
    }
}

/// A download job as persisted in the database.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    /// yt-dlp media job. Defaults to false for existing direct-file videos.
    #[serde(default)]
    pub media_download: bool,
    #[serde(default)]
    pub media_format: Option<String>,
    pub id: JobId,
    /// URL as supplied by the user or browser.
    pub url: String,
    /// URL after redirects, from the most recent response.
    pub final_url: Option<String>,
    pub referrer: Option<String>,
    /// Extra request headers (cookies, user agent, ...). Cleared on completion.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    pub file_name: String,
    /// Directory the finished file is placed in.
    pub dir: PathBuf,
    pub category: String,
    /// Total size in bytes, when the server reports it.
    pub size: Option<u64>,
    /// Whether the server accepts byte ranges (multi-connection and resume).
    pub resumable: Option<bool>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub mime: Option<String>,
    pub state: JobState,
    pub error: Option<JobError>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    /// When true the queue starts this job automatically.
    pub queued: bool,
    /// Ordering key inside the queue (lower runs first).
    pub queue_order: i64,
    /// Per-job connection cap; `None` uses the global setting.
    pub max_connections: Option<u8>,
    /// Per-job speed limit in bytes/second; `None` means unlimited.
    pub speed_limit: Option<u64>,
    pub checksum: Option<Checksum>,
    /// Automatic job-level retries used so far.
    #[serde(default)]
    pub auto_retries: u32,
    #[serde(default)]
    pub source: JobSource,
    /// Bytes done, cached for the UI and list sorting.
    #[serde(default)]
    pub downloaded: u64,
    /// Accumulated active transfer time in milliseconds.
    #[serde(default)]
    pub active_ms: u64,
    /// The name was guessed from the URL and may be replaced by the
    /// server's `Content-Disposition` once the first response arrives.
    #[serde(default)]
    pub auto_name: bool,
    /// The folder was derived from the category (not chosen by the user).
    #[serde(default)]
    pub auto_dir: bool,
}

impl Job {
    pub fn target_path(&self) -> PathBuf {
        self.dir.join(&self.file_name)
    }
    pub fn part_path(&self) -> PathBuf {
        self.dir.join(format!("{}.crowdm-part", self.file_name))
    }
}

/// Everything the UI needs to render one row.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSnapshot {
    #[serde(flatten)]
    pub job: Job,
    /// Bytes per second, smoothed. Zero when not downloading.
    pub speed: u64,
    /// Seconds remaining when size and speed are known.
    pub eta: Option<u64>,
    pub connections: u32,
    /// Short phase text while downloading ("Connecting", "Verifying" ...).
    pub phase: Option<String>,
    /// Per-segment progress for the detail view: (start, end, done).
    pub segments: Vec<(u64, u64, u64)>,
}

/// Lightweight periodic progress sample for running jobs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub id: JobId,
    pub downloaded: u64,
    pub size: Option<u64>,
    pub speed: u64,
    pub eta: Option<u64>,
    pub connections: u32,
    pub phase: Option<String>,
    /// (start, end, done) per segment, for the transfer map.
    pub segments: Vec<(u64, u64, u64)>,
}

/// Input for creating a job.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NewJob {
    #[serde(default)]
    pub media_download: bool,
    #[serde(default)]
    pub media_format: Option<String>,
    pub url: String,
    pub referrer: Option<String>,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// Desired file name; derived from the server when absent.
    pub file_name: Option<String>,
    /// Explicit directory; derived from the category when absent.
    pub dir: Option<PathBuf>,
    pub category: Option<String>,
    /// Start immediately (true) or only add to the queue (false).
    #[serde(default = "default_true")]
    pub start: bool,
    pub max_connections: Option<u8>,
    pub speed_limit: Option<u64>,
    pub checksum: Option<Checksum>,
    #[serde(default)]
    pub source: JobSource,
    /// Reuse the connection opened by an earlier [`crate::Engine::probe`].
    pub probe_id: Option<Uuid>,
    /// Size hint from the browser, used until the server answers.
    pub size_hint: Option<u64>,
    pub mime_hint: Option<String>,
    /// Overwrite an existing file instead of choosing a free name.
    #[serde(default)]
    pub overwrite: bool,
}

fn default_true() -> bool {
    true
}

/// Result of probing a URL before the user confirms a download.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeInfo {
    #[serde(default)]
    pub size_estimated: bool,
    #[serde(default)]
    pub media_options: Vec<crate::media::MediaOption>,
    pub probe_id: Option<Uuid>,
    pub final_url: String,
    pub file_name: String,
    pub size: Option<u64>,
    pub resumable: bool,
    pub mime: Option<String>,
    pub category: String,
    /// Folder the file would be saved to with the current settings.
    pub dir: PathBuf,
    /// True when a file with this name already exists in `dir`.
    pub exists: bool,
    /// An existing job for the same URL, if any.
    pub duplicate_of: Option<JobId>,
    /// Hints about the response worth showing ("Server does not support resume").
    pub warnings: Vec<String>,
    /// Status of the probe response (200/206...).
    pub status: u16,
    /// True when the body looks like an HTML page rather than a file.
    pub looks_like_html: bool,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}
