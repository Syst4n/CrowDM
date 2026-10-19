//! BitTorrent sessions, metadata and verified piece transfers.
use crate::{
    error::{EngineError, Result, TransferError},
    model::{ErrorKind, Job},
    settings::Settings,
    transfer::{Shared, Stop},
};
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, DhtSessionConfig, ListenerMode, ListenerOptions, Session, SessionOptions,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Component, Path, PathBuf},
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

pub const MAX_METAINFO: usize = 16 * 1024 * 1024;

fn invalid(message: impl Into<String>) -> EngineError {
    EngineError::Invalid(message.into())
}
fn failure(message: impl Into<String>) -> Stop {
    Stop::Failed(TransferError::new(ErrorKind::Io, message))
}

pub fn is_input(input: &str) -> bool {
    let input = input.trim();
    input.to_ascii_lowercase().starts_with("magnet:")
        || (!input.starts_with("http://")
            && !input.starts_with("https://")
            && local_path(input).is_some_and(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("torrent"))))
}

pub fn local_path(input: &str) -> Option<PathBuf> {
    if input.starts_with("file:") {
        url::Url::parse(input).ok()?.to_file_path().ok()
    } else if !input.contains("://") && !input.starts_with("magnet:") {
        Some(PathBuf::from(input))
    } else {
        None
    }
}

/// Windows allocates TCP and UDP ephemeral ports separately. A port returned
/// by TCP port 0 may already belong to a UDP/QUIC application or be UDP-reserved.
pub fn available_port() -> std::io::Result<u16> {
    for _ in 0..64 {
        let udp = std::net::UdpSocket::bind(("0.0.0.0", 0))?;
        let port = udp.local_addr()?.port();
        if std::net::TcpListener::bind(("0.0.0.0", port)).is_ok() {
            return Ok(port);
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AddrInUse, "No available TCP/UDP torrent port"))
}

pub async fn new_session(settings: &Settings) -> Result<Arc<Session>> {
    let config = &settings.torrents;
    let isolated = std::env::var_os("CROWDM_TEST_INSTANCE").is_some();
    let port = if isolated || config.listen_port == 0 { available_port()? } else { config.listen_port };
    Session::new_with_opts(
        settings.dir_for_category("torrents"),
        SessionOptions {
            // CrowDM persists jobs/metadata in SQLite. Avoid an unrelated rqbit
            // profile, restored background jobs or an HTTP control server.
            persistence: None,
            dht: if isolated { None } else { Some(DhtSessionConfig { persistence: None, ..Default::default() }) },
            disable_local_service_discovery: isolated,
            listen: Some(ListenerOptions {
                listen_addr: if isolated { ([127, 0, 0, 1], port).into() } else { ([0, 0, 0, 0, 0, 0, 0, 0], port).into() },
                mode: if config.enable_utp { ListenerMode::TcpAndUtp } else { ListenerMode::TcpOnly },
                enable_upnp_port_forwarding: !isolated && config.enable_upnp,
                ..Default::default()
            }),
            peer_limit: Some(config.peer_limit),
            concurrent_init_limit: Some(2),
            client_name_and_version: Some(format!("CrowDM/{}", env!("CARGO_PKG_VERSION"))),
            ..Default::default()
        },
    )
    .await
    .map_err(|_| invalid("Could not start the torrent listener. Check the configured port or choose 0 for an automatic port."))
}

pub async fn read_local(input: &str) -> Result<bytes::Bytes> {
    let path = local_path(input).ok_or_else(|| invalid("Choose a .torrent file or paste a magnet link."))?;
    if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("torrent")) {
        return Err(invalid("Choose a .torrent file."));
    }
    let file = tokio::fs::File::open(path).await?;
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    file.take(MAX_METAINFO as u64 + 1).read_to_end(&mut bytes).await?;
    if bytes.len() > MAX_METAINFO {
        return Err(invalid("The torrent metadata file is too large (maximum 16 MiB)."));
    }
    Ok(bytes.into())
}

pub fn inspect(bytes: &[u8]) -> Result<(TorrentInfo, String)> {
    if bytes.is_empty() || bytes.len() > MAX_METAINFO {
        return Err(invalid("Invalid or oversized torrent metadata."));
    }
    let parsed = librqbit::torrent_from_bytes(bytes).map_err(|_| invalid("This is not a valid BitTorrent v1 or hybrid .torrent file."))?;
    let hash = parsed.info_hash.as_string();
    let info = parsed.info.data.validate().map_err(|_| invalid("The torrent contains invalid file sizes or piece hashes."))?;
    let mut files = Vec::new();
    let mut paths = std::collections::HashSet::new();
    let mut total = 0u64;
    for (index, file) in info.iter_file_details().enumerate() {
        let path = file.filename.to_pathbuf();
        validate_relative(&path)?;
        if file.attrs().symlink || file.symlink_path.is_some() {
            return Err(invalid("Torrent symbolic links are not supported."));
        }
        if file.attrs().padding {
            continue;
        }
        if !paths.insert(path.to_string_lossy().to_ascii_lowercase()) {
            return Err(invalid("Torrent contains duplicate Windows file paths."));
        }
        total = total.checked_add(file.len).ok_or_else(|| invalid("Invalid torrent size."))?;
        files.push(TorrentFile { index, path: path.to_string_lossy().into_owned(), size: file.len });
        if files.len() > 10000 {
            return Err(invalid("Torrents with more than 10,000 files are not supported in this preview."));
        }
    }
    if files.is_empty() || total == 0 {
        return Err(invalid("The torrent has no downloadable content."));
    }
    // Reject file/directory aliases as well as exact duplicate paths.
    for f in &files {
        for parent in Path::new(&f.path).ancestors().skip(1).filter(|p| !p.as_os_str().is_empty()) {
            if paths.contains(&parent.to_string_lossy().to_ascii_lowercase()) {
                return Err(invalid("Torrent file and directory paths conflict."));
            }
        }
    }
    let multi_file = files.len() > 1 || Path::new(&files[0].path).components().count() > 1;
    let name = if multi_file {
        crate::names::sanitize(info.name().as_deref().unwrap_or("Torrent download"))
    } else {
        Path::new(&files[0].path).file_name().unwrap().to_string_lossy().into_owned()
    };
    Ok((TorrentInfo { info_hash: hash, files, only_files: None, multi_file, private: info.info().private }, name))
}

pub fn validate_relative(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        return Err(invalid("Empty torrent path."));
    }
    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err(invalid("Unsafe torrent path: absolute paths and parent traversal are rejected."));
        };
        let text = name.to_string_lossy();
        if crate::names::sanitize(&text) != text || text.contains(['/', '\\', ':']) {
            return Err(invalid("The torrent contains a filename Windows cannot safely create."));
        }
    }
    Ok(())
}

impl TorrentInfo {
    pub fn selected_size(&self) -> u64 {
        self.files.iter().filter(|f| self.only_files.as_ref().is_none_or(|ids| ids.contains(&f.index))).map(|f| f.size).sum()
    }
    pub fn select(&mut self, indices: Option<Vec<usize>>) -> Result<()> {
        if let Some(ids) = &indices {
            if ids.is_empty() || ids.iter().any(|i| !self.files.iter().any(|f| f.index == *i)) {
                return Err(invalid("Select at least one valid torrent file."));
            }
        }
        self.only_files = indices.map(|mut v| {
            v.sort_unstable();
            v.dedup();
            v
        });
        Ok(())
    }
}

pub async fn resolve(session: &Arc<Session>, input: &str) -> Result<(bytes::Bytes, Vec<std::net::SocketAddr>)> {
    let magnet = librqbit::Magnet::parse(input).map_err(|_| invalid("Invalid magnet link."))?;
    if magnet.as_id20().is_none() {
        return Err(invalid("This preview supports BitTorrent v1 and hybrid magnets. A v2-only magnet cannot be opened yet."));
    }
    let response = tokio::time::timeout(
        Duration::from_secs(90),
        session.add_torrent(AddTorrent::from_url(input), Some(AddTorrentOptions { list_only: true, ..Default::default() })),
    )
    .await
    .map_err(|_| invalid("No peer supplied the magnet's metadata within 90 seconds. Check the swarm or try its .torrent file."))?
    .map_err(|_| invalid("Could not resolve magnet metadata. Check its trackers and available peers, or use the .torrent file."))?;
    match response {
        AddTorrentResponse::ListOnly(list) => Ok((list.torrent_bytes, list.seen_peers)),
        AddTorrentResponse::AlreadyManaged(_, handle) => {
            handle.with_metadata(|m| (m.torrent_bytes.clone(), vec![])).map_err(|_| invalid("Torrent metadata is not ready."))
        }
        _ => Err(invalid("Unexpected torrent metadata response.")),
    }
}

pub fn staging_dir(job: &Job) -> PathBuf {
    job.dir.join(format!(".crowdm-torrent-{}", job.id))
}

fn check_tree(job: &Job) -> Result<()> {
    let root = staging_dir(job);
    // An existing reparse point must never redirect resumed writes outside
    // the job-owned staging folder (including Windows junctions).
    for f in &job.torrent.as_ref().unwrap().files {
        let path = root.join(&f.path);
        for p in path.ancestors().take_while(|p| p.starts_with(&root)) {
            if let Ok(meta) = std::fs::symlink_metadata(p) {
                #[cfg(windows)]
                let reparse = {
                    use std::os::windows::fs::MetadataExt;
                    meta.file_attributes() & 0x400 != 0
                };
                #[cfg(not(windows))]
                let reparse = meta.file_type().is_symlink();
                if reparse {
                    return Err(invalid("A torrent staging path is a link. Choose a regular destination folder."));
                }
            }
        }
    }
    Ok(())
}

pub fn rate(rate: u64) -> Option<std::num::NonZeroU32> {
    std::num::NonZeroU32::new(rate.min(u32::MAX as u64) as u32)
}

pub async fn run(
    session: Arc<Session>,
    bytes: Vec<u8>,
    job: Job,
    shared: Arc<Shared>,
    cancel: tokio_util::sync::CancellationToken,
    initial_peers: Vec<std::net::SocketAddr>,
) -> std::result::Result<u64, Stop> {
    check_tree(&job).map_err(|e| failure(e.to_string()))?;
    let torrent = job.torrent.as_ref().unwrap();
    let folder = staging_dir(&job);
    std::fs::create_dir_all(&folder).map_err(|e| failure(e.to_string()))?;
    if job.speed_limit.is_some_and(|v| v > 0 && v < 16384) {
        return Err(failure("Torrent speed limits must be at least 16 KB/s, or unlimited."));
    }
    shared.set_phase(Some("Checking torrent pieces"));
    let response = session
        .add_torrent(
            AddTorrent::TorrentFileBytes(bytes.into()),
            Some(AddTorrentOptions {
                output_folder: Some(folder.to_string_lossy().into_owned()),
                overwrite: true,
                only_files: Some(torrent.only_files.clone().unwrap_or_else(|| torrent.files.iter().map(|f| f.index).collect())),
                initial_peers: Some(initial_peers),
                trackers: if job.url.starts_with("magnet:") {
                    librqbit::Magnet::parse(&job.url).ok().map(|m| m.trackers.into_iter().map(|t| t.to_string()).collect())
                } else {
                    None
                },
                ratelimits: librqbit::limits::LimitsConfig { download_bps: rate(job.speed_limit.unwrap_or(0)), ..Default::default() },
                ..Default::default()
            }),
        )
        .await
        .map_err(|_| failure("Could not open torrent storage or start its peers. Check the destination and free space."))?;
    let handle = match response {
        AddTorrentResponse::Added(_, h) => h,
        AddTorrentResponse::AlreadyManaged(_, _) => return Err(failure("This torrent is already active in CrowDM.")),
        _ => return Err(failure("Torrent could not start.")),
    };
    let id = handle.id();
    let mut received = 0;
    let result = loop {
        if cancel.is_cancelled() {
            break Err(Stop::Canceled);
        }
        if session.ratelimits.get_download_bps().is_some_and(|n| n.get() < 16384) {
            break Err(failure("The shared torrent bandwidth budget is below 16 KB/s. Raise the global limit or reduce simultaneous downloads, then resume."));
        }
        let stats = handle.stats();
        let downloaded = stats
            .file_progress
            .iter()
            .enumerate()
            .filter(|(i, _)| torrent.only_files.as_ref().is_none_or(|ids| ids.contains(i)))
            .map(|(_, n)| *n)
            .sum::<u64>()
            .min(torrent.selected_size());
        shared.media_progress(downloaded, Some(torrent.selected_size()));
        if let Some(live) = stats.live {
            received = received.max(live.snapshot.fetched_bytes);
            shared.received.store(received, Ordering::Relaxed);
            shared.connections.store(live.snapshot.peer_stats.live, Ordering::Relaxed);
            shared.set_phase(Some(if live.snapshot.peer_stats.live == 0 { "Finding torrent peers" } else { "Downloading torrent" }));
        }
        if stats.error.is_some() {
            break Err(failure("Torrent failed while checking or writing pieces. Check disk space and file access, then resume."));
        }
        if stats.finished {
            break Ok(torrent.selected_size());
        }
        tokio::select! { _=cancel.cancelled()=>{}, _=tokio::time::sleep(Duration::from_millis(250))=>{} }
    };
    // Stop uploading and close all storage before moving/removing files.
    let stopped = session.delete(id.into(), false).await;
    drop(handle);
    if stopped.is_err() && result.is_ok() {
        return Err(failure("Could not close torrent storage before finishing. Resume to retry."));
    }
    result
}

pub fn finish(job: &Job) -> Result<PathBuf> {
    check_tree(job)?;
    let torrent = job.torrent.as_ref().unwrap();
    let stage = staging_dir(job);
    if let Some(ids) = &torrent.only_files {
        for f in &torrent.files {
            if !ids.contains(&f.index) {
                let _ = std::fs::remove_file(stage.join(&f.path));
            }
        }
    }
    let source = if torrent.multi_file { stage.clone() } else { stage.join(&torrent.files[0].path) };
    let mut attempts = 0;
    let target = loop {
        check_tree(job)?;
        match crate::file::finalize(&source, &job.target_path(), false) {
            Ok(path) => break path,
            // Peer teardown, virus scanners and synced folders can retain a
            // Windows file handle briefly after the final verified piece.
            Err(e) if cfg!(windows) && matches!(e.raw_os_error(), Some(5 | 32 | 33)) && attempts < 60 => {
                attempts += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e.into()),
        }
    };
    if !torrent.multi_file {
        let _ = std::fs::remove_dir(&stage);
    }
    Ok(target)
}

pub fn cleanup(job: &Job) {
    // This UUID directory belongs only to this job. Never recurse through a
    // replacement link supplied by another process.
    if check_tree(job).is_ok() {
        let _ = std::fs::remove_dir_all(staging_dir(job));
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TorrentFile {
    pub index: usize,
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TorrentInfo {
    pub info_hash: String,
    pub files: Vec<TorrentFile>,
    pub only_files: Option<Vec<usize>>,
    pub multi_file: bool,
    pub private: bool,
}
