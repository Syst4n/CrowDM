//! Non-DRM video extraction and merging through yt-dlp and FFmpeg. Output
//! stays in a job-owned staging directory until normal engine finalization.
use crate::{
    error::TransferError,
    file::PartFile,
    model::{ErrorKind, Job, ProbeInfo},
    names,
    settings::Settings,
    transfer::{Completed, Shared, Stop},
    ProbeRequest,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::{Child, Command},
};
use tokio_util::sync::CancellationToken;

fn invalid(message: impl Into<String>) -> TransferError {
    TransferError::new(ErrorKind::Http, message)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaOption {
    pub format: String,
    pub label: String,
    pub height: Option<u64>,
    pub container: String,
    pub size: Option<u64>,
}

pub fn output_format(format: Option<&str>) -> (&str, &str) {
    match format.unwrap_or("bv*+ba/b").split_once(':') {
        Some((container @ ("mp4" | "mp3" | "mkv"), selector)) => (container, selector),
        _ => ("mkv", format.unwrap_or("bv*+ba/b")),
    }
}
fn estimated_size(f: &Value, duration: Option<f64>) -> Option<u64> {
    f["filesize"].as_u64().or_else(|| f["filesize_approx"].as_u64()).or_else(|| {
        let bytes = f["tbr"].as_f64()? * 1000.0 / 8.0 * duration?;
        (bytes.is_finite() && bytes > 0.0).then_some(bytes as u64)
    })
}

fn quality_options(info: &Value) -> Vec<MediaOption> {
    let mut heights: Vec<u64> = info["formats"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f["vcodec"].as_str() != Some("none") && f["has_drm"].as_bool() != Some(true))
        .filter_map(|f| f["height"].as_u64())
        .filter(|h| *h > 0)
        .collect();
    if let Some(height) = info["height"].as_u64() {
        heights.push(height);
    }
    heights.sort_unstable_by(|a, b| b.cmp(a));
    heights.dedup();
    let duration = info["duration"].as_f64();
    let formats: Vec<&Value> = info["formats"].as_array().map(|f| f.iter().collect()).unwrap_or_else(|| vec![info]);
    let audio_size = formats.iter().filter(|f| f["vcodec"] == "none").filter_map(|f| estimated_size(f, duration)).max();
    let estimate = |height: Option<u64>, mp4: bool| {
        formats
            .iter()
            .filter(|f| {
                f["vcodec"] != "none"
                    && f["has_drm"] != true
                    && height.is_none_or(|h| f["height"].as_u64() == Some(h))
                    && (!mp4 || f["ext"] == "mp4")
            })
            .filter_map(|f| estimated_size(f, duration).map(|size| size + if f["acodec"] == "none" { audio_size.unwrap_or(0) } else { 0 }))
            .max()
    };
    let mut options = vec![MediaOption {
        format: "bv*+ba/b".into(),
        label: "Best video + audio".into(),
        height: None,
        container: "mkv".into(),
        size: estimate(None, false),
    }];
    options.extend(heights.iter().map(|h| MediaOption {
        format: format!("bv*[height={h}]+ba/b[height={h}]"),
        label: format!("{h}p video + audio"),
        height: Some(*h),
        container: "mkv".into(),
        size: estimate(Some(*h), false),
    }));
    for height in std::iter::once(None).chain(heights.into_iter().map(Some)) {
        if !formats.iter().any(|f| f["ext"] == "mp4" && f["vcodec"] != "none" && height.is_none_or(|h| f["height"].as_u64() == Some(h))) {
            continue;
        }
        let filter = height.map(|h| format!("[height={h}]")).unwrap_or_default();
        options.push(MediaOption {
            format: format!("mp4:bv*[ext=mp4]{filter}+ba[ext=m4a]/b[ext=mp4]{filter}"),
            label: height.map(|h| format!("{h}p video + audio")).unwrap_or_else(|| "Best video + audio".into()),
            height,
            container: "mp4".into(),
            size: estimate(height, true),
        });
    }
    if formats.iter().any(|f| f["acodec"].as_str() != Some("none")) {
        options.push(MediaOption {
            format: "mp3:ba/b".into(),
            label: "Audio only · MP3".into(),
            height: None,
            container: "mp3".into(),
            size: duration.map(|d| (d * 320_000.0 / 8.0) as u64),
        });
    }
    options
}

/// Recognize watch links pasted into Add URL as well as manifest URLs.
pub fn is_media_url(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else { return false };
    let host = url.host_str().unwrap_or_default();
    host == "youtu.be"
        || (host == "youtube.com" || host.ends_with(".youtube.com"))
            && (url.path() == "/watch"
                || url.path().starts_with("/shorts/")
                || url.path().starts_with("/embed/")
                || url.path().starts_with("/live/"))
        || [".m3u8", ".mpd"].iter().any(|ext| url.path().to_ascii_lowercase().ends_with(ext))
}

pub fn tool(name: &str) -> PathBuf {
    let filename = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    if let Some(dir) = std::env::var_os("CROWDM_MEDIA_TOOLS") {
        let p = PathBuf::from(dir).join(&filename);
        if p.is_file() {
            return p;
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        for dir in exe.ancestors().skip(1).take(5) {
            for p in [dir.join("tools").join("media").join(&filename), dir.join("media").join(&filename), dir.join(&filename)] {
                if p.is_file() {
                    return p;
                }
            }
        }
    }
    filename.into() // Also support tools installed on PATH.
}

fn hidden(command: &mut Command) {
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    command.kill_on_drop(true);
}

// The Cookie header is scoped to its original host in a Netscape cookie
// file, rather than being forwarded as a header to arbitrary media CDNs.
struct SessionFile(PathBuf);
impl Drop for SessionFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn session_file(url: &str, headers: &[(String, String)]) -> Result<SessionFile, TransferError> {
    let path = std::env::temp_dir().join(format!("crowdm-session-{}.txt", uuid::Uuid::new_v4()));
    let parsed = url::Url::parse(url).map_err(|_| invalid("Invalid media URL."))?;
    let host = parsed.host_str().ok_or_else(|| invalid("Invalid media host."))?;
    let mut text = String::from("# Netscape HTTP Cookie File\n");
    for (_, value) in headers.iter().filter(|(name, _)| name.eq_ignore_ascii_case("cookie")) {
        for pair in value.split(';') {
            if let Some((key, value)) = pair.trim().split_once('=') {
                if !key.is_empty() && !pair.contains(['\r', '\n', '\t']) {
                    text.push_str(&format!(
                        "{host}\tFALSE\t/\t{}\t0\t{key}\t{value}\n",
                        if parsed.scheme() == "https" { "TRUE" } else { "FALSE" }
                    ));
                }
            }
        }
    }
    std::fs::write(&path, text).map_err(|e| TransferError::io("Could not prepare the video session", e))?;
    Ok(SessionFile(path))
}

fn command(req: &ProbeRequest, session: &SessionFile) -> Command {
    let mut cmd = Command::new(tool("yt-dlp"));
    hidden(&mut cmd);
    cmd.args([
        "--ignore-config",
        "--no-plugin-dirs",
        "--no-cache-dir",
        "--no-playlist",
        "--no-warnings",
        "--socket-timeout",
        "25",
        "--retries",
        "3",
        "--no-colors",
    ])
    .arg("--cookies")
    .arg(&session.0)
    .arg("--ffmpeg-location")
    .arg(tool("ffmpeg"))
    .arg("--js-runtimes")
    .arg(format!("node:{}", tool("node").display()));
    for (name, value) in &req.headers {
        if name.eq_ignore_ascii_case("user-agent") {
            cmd.arg("--user-agent").arg(value);
        }
    }
    if let Some(referrer) = &req.referrer {
        cmd.arg("--referer").arg(referrer);
    }
    cmd
}

fn process_error(stderr: &[u8]) -> TransferError {
    // Keep errors actionable without persisting cookies or signed URLs.
    let raw = String::from_utf8_lossy(stderr);
    let line = raw
        .lines()
        .rev()
        .find(|l| l.contains("ERROR:"))
        .unwrap_or("Video extraction failed. Check that the video is playable and update the media tools if needed.");
    let safe = line.split_whitespace().map(|s| if s.contains("://") { "[media address]" } else { s }).collect::<Vec<_>>().join(" ");
    invalid(safe.chars().take(700).collect::<String>())
}

async fn metadata(req: &ProbeRequest) -> Result<Value, TransferError> {
    let session = session_file(&req.url, &req.headers)?;
    let mut cmd = command(req, &session);
    cmd.args(["--skip-download", "--dump-single-json", "--format", output_format(req.media_format.as_deref()).1]).arg("--").arg(&req.url);
    let output = tokio::time::timeout(Duration::from_secs(100), cmd.output())
        .await
        .map_err(|_| invalid("Video lookup timed out. Try the video button again."))?
        .map_err(|e| TransferError::io("Could not start yt-dlp. Run scripts/prepare-media.ps1 to install the video tools", e))?;
    if !output.status.success() {
        return Err(process_error(&output.stderr));
    }
    let info: Value = serde_json::from_slice(&output.stdout).map_err(|_| invalid("The video extractor returned invalid metadata."))?;
    if info["has_drm"].as_bool() == Some(true) {
        return Err(invalid("This video is DRM-protected and cannot be downloaded."));
    }
    if info["is_live"].as_bool() == Some(true) || matches!(info["live_status"].as_str(), Some("is_live" | "is_upcoming" | "post_live")) {
        return Err(invalid("This is a live broadcast. Download it after the broadcast ends."));
    }
    if output_format(req.media_format.as_deref()).0 != "mp3" && info["vcodec"].as_str() == Some("none") {
        return Err(invalid("No downloadable video track was found."));
    }
    Ok(info)
}

// Inspect the selected media playlist, not the watch page. ENDLIST is the
// authoritative VOD boundary; a duration alone does not make a stream finite.
async fn stream_size(req: &ProbeRequest, info: &Value) -> Result<Option<u64>, TransferError> {
    let formats: Vec<&Value> = info["requested_formats"].as_array().map(|f| f.iter().collect()).unwrap_or_else(|| vec![info]);
    let client =
        reqwest::Client::builder().timeout(Duration::from_secs(15)).build().map_err(|_| invalid("Could not inspect the media stream."))?;
    let mut total = 0;
    let mut known = true;
    for f in formats {
        let protocol = f["protocol"].as_str().unwrap_or_default();
        let mut size = estimated_size(f, info["duration"].as_f64());
        if !protocol.starts_with("m3u8") {
            if protocol.contains("dash") && info["duration"].as_f64().is_none() && f["fragments"].as_array().is_none_or(|a| a.is_empty()) {
                return Err(invalid("This stream has no finite duration or segment list. Live recording is not supported."));
            }
            if size.is_none() && matches!(protocol, "http" | "https") {
                if let Some(address) = f["url"].as_str() {
                    if let Ok(response) = client.head(address).headers(scoped_headers(req, address)).send().await {
                        if response.status().is_success() {
                            size = response
                                .headers()
                                .get(reqwest::header::CONTENT_LENGTH)
                                .and_then(|h| h.to_str().ok())
                                .and_then(|s| s.parse().ok());
                        }
                    }
                }
            }
            if let Some(size) = size {
                total += size;
            } else {
                known = false;
            }
            continue;
        }
        let address = f["url"].as_str().ok_or_else(|| invalid("The media playlist address is missing."))?;
        let response = client
            .get(address)
            .headers(scoped_headers(req, address))
            .send()
            .await
            .map_err(|_| invalid("Could not inspect the video playlist. Try again."))?;
        let base = response.url().clone();
        if !response.status().is_success() {
            return Err(invalid("The video playlist is no longer available. Try the video button again."));
        }
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| invalid("Could not read the video playlist."))? {
            if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
                return Err(invalid("The video playlist is too large to inspect safely."));
            }
            bytes.extend_from_slice(&chunk);
        }
        let playlist = String::from_utf8_lossy(&bytes);
        if !playlist.contains("#EXT-X-ENDLIST") || playlist.contains("#EXT-X-STREAM-INF") {
            return Err(invalid("This media playlist has no fixed ending. Live or unfinished streams cannot be downloaded."));
        }
        let segments: Vec<_> = playlist.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
        if segments.is_empty() {
            return Err(invalid("The video playlist contains no media segments."));
        }
        if let Some(size) = hls_range_size(&playlist, segments.len()) {
            total += size;
            continue;
        }
        if let Some(size) = size {
            total += size;
            continue;
        }
        let mut samples = Vec::new();
        for index in [0, segments.len() / 2, segments.len() - 1] {
            if let Ok(url) = base.join(segments[index]) {
                if let Ok(response) = client.head(url.as_str()).headers(scoped_headers(req, url.as_str())).send().await {
                    if response.status().is_success() {
                        if let Some(size) = response
                            .headers()
                            .get(reqwest::header::CONTENT_LENGTH)
                            .and_then(|h| h.to_str().ok())
                            .and_then(|s| s.parse::<u64>().ok())
                            .filter(|s| *s > 0)
                        {
                            samples.push(size);
                        }
                    }
                }
            }
        }
        if samples.is_empty() {
            known = false;
        } else {
            total += samples.iter().sum::<u64>() / samples.len() as u64 * segments.len() as u64;
        }
    }
    Ok((known && total > 0).then_some(total))
}

fn scoped_headers(req: &ProbeRequest, url: &str) -> reqwest::header::HeaderMap {
    let same_host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string))
        == url::Url::parse(&req.url).ok().and_then(|u| u.host_str().map(str::to_string));
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in &req.headers {
        if name.eq_ignore_ascii_case("user-agent") || same_host && name.eq_ignore_ascii_case("cookie") {
            if let (Ok(name), Ok(value)) = (name.parse::<reqwest::header::HeaderName>(), value.parse()) {
                headers.insert(name, value);
            }
        }
    }
    if let Some(referrer) = &req.referrer {
        if let Ok(value) = referrer.parse() {
            headers.insert(reqwest::header::REFERER, value);
        }
    }
    headers
}

fn hls_range_size(playlist: &str, segments: usize) -> Option<u64> {
    let ranges: Vec<u64> = playlist
        .lines()
        .filter_map(|line| line.trim().strip_prefix("#EXT-X-BYTERANGE:"))
        .map(|range| range.split('@').next()?.parse().ok())
        .collect::<Option<_>>()?;
    if segments == 0 || ranges.len() != segments {
        return None;
    }
    let mut size = ranges.into_iter().try_fold(0u64, |sum, length| sum.checked_add(length))?;
    for line in playlist.lines().filter(|line| line.starts_with("#EXT-X-MAP:")) {
        if let Some(range) = line.split("BYTERANGE=\"").nth(1).and_then(|s| s.split('"').next()) {
            size = size.checked_add(range.split('@').next()?.parse().ok()?)?;
        }
    }
    Some(size)
}

pub fn select_output(info: &mut ProbeInfo, format: &str, settings: &Settings) {
    let (container, _) = output_format(Some(format));
    info.file_name = format!("{}.{container}", std::path::Path::new(&info.file_name).file_stem().unwrap_or_default().to_string_lossy());
    info.category = if container == "mp3" { "audio" } else { "videos" }.into();
    info.dir = settings.dir_for_category(&info.category);
    info.mime = Some(
        match container {
            "mp3" => "audio/mpeg",
            "mp4" => "video/mp4",
            _ => "video/x-matroska",
        }
        .into(),
    );
    info.size = info.media_options.iter().find(|o| o.format == format).and_then(|o| o.size);
    info.exists = info.dir.join(&info.file_name).exists();
    info.warnings = vec![if container == "mp3" {
        "Audio is converted to MP3 and saved in your Audio folder. Artwork is kept when available."
    } else {
        "Video and audio are saved together. Estimated sizes can change with variable bitrate and final processing."
    }
    .into()];
}

pub async fn probe(req: ProbeRequest, settings: &Settings) -> Result<ProbeInfo, TransferError> {
    let info = metadata(&req).await?;
    let size = stream_size(&req, &info).await?;
    let title = names::sanitize(info["title"].as_str().unwrap_or("Video"));
    let file_name = format!("{}.mkv", title.trim_end_matches('.'));
    let dir = settings.dir_for_category("videos");
    let mut probe = ProbeInfo {
        size_estimated: true,
        media_options: quality_options(&info),
        probe_id: None,
        final_url: req.url.clone(),
        exists: dir.join(&file_name).exists(),
        file_name,
        size,
        resumable: true,
        mime: Some("video/x-matroska".into()),
        category: "videos".into(),
        dir,
        duplicate_of: None,
        warnings: vec!["Video and audio are saved together as MKV. The final size is known after merging.".into()],
        status: 200,
        looks_like_html: false,
    };
    let selected = req.media_format.as_deref().unwrap_or("bv*+ba/b");
    if let Some(option) = probe.media_options.iter_mut().find(|o| o.format == selected) {
        option.size = size.or(option.size);
    }
    select_output(&mut probe, selected, settings);
    probe.size = probe.size.or(size);
    Ok(probe)
}

pub fn staging_dir(job: &Job) -> PathBuf {
    job.dir.join(format!(".crowdm-media-{}", job.id))
}

pub fn cleanup(job: &Job) {
    let dir = staging_dir(job);
    if let (Ok(resolved), Ok(parent)) = (std::fs::canonicalize(&dir), std::fs::canonicalize(&job.dir)) {
        if resolved.parent() == Some(parent.as_path()) && resolved.file_name() == dir.file_name() {
            let _ = std::fs::remove_dir_all(resolved);
        }
    }
}

async fn stop_child(child: &mut Child) {
    #[cfg(windows)]
    if let Some(pid) = child.id() {
        let mut cmd = Command::new("taskkill.exe");
        hidden(&mut cmd);
        let _ = cmd.args(["/PID", &pid.to_string(), "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null()).status().await;
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}

pub async fn run(
    job: Job,
    shared: Arc<Shared>,
    cancel: CancellationToken,
    limit: Arc<crate::ratelimit::RateLimiter>,
    global: Arc<crate::ratelimit::RateLimiter>,
) -> Result<Completed, Stop> {
    let dir = staging_dir(&job);
    std::fs::create_dir_all(&dir).map_err(|e| TransferError::io("Could not create video staging folder", e))?;
    let resolved = std::fs::canonicalize(&dir).map_err(|e| TransferError::io("Could not resolve video staging folder", e))?;
    let parent = std::fs::canonicalize(&job.dir).map_err(|e| TransferError::io("Could not resolve video destination", e))?;
    if resolved.parent() != Some(parent.as_path()) {
        return Err(invalid("The video staging folder points outside the destination.").into());
    }
    let req = ProbeRequest {
        url: job.url.clone(),
        referrer: job.referrer.clone(),
        headers: job.headers.clone(),
        media_format: job.media_format.clone(),
        ..Default::default()
    };
    let session = session_file(&req.url, &req.headers)?;
    let effective_rate = || match (limit.rate(), global.rate()) {
        (0, g) => g,
        (j, 0) => j,
        (j, g) => j.min(g),
    };
    loop {
        if cancel.is_cancelled() {
            return Err(Stop::Canceled);
        }
        shared.set_phase(Some("Resolving video"));
        let info = tokio::select! {
            _ = cancel.cancelled() => return Err(Stop::Canceled),
            info = metadata(&req) => info?,
        };
        let estimate = tokio::select! {
            _ = cancel.cancelled() => return Err(Stop::Canceled),
            size = stream_size(&req, &info) => size?.or(job.size),
        };
        let byte_limit = estimate.map(|s| s.saturating_mul(3).saturating_add(64 * 1024 * 1024));
        let (container, selector) = output_format(job.media_format.as_deref());
        let rate = effective_rate();
        let mut cmd = command(&req, &session);
        cmd.args([
            "--format",
            selector,
            "--abort-on-unavailable-fragments",
            "--fragment-retries",
            "3",
            "--match-filters",
            "!is_live",
            "--downloader",
            "m3u8:native",
            "--continue",
            "--no-overwrites",
            "--newline",
            "--progress",
            "--no-simulate",
            "--progress-delta",
            "0.5",
            "--progress-template",
            "download:CROWDM_PROGRESS:%(progress)j",
            "--print",
            "after_move:CROWDM_FILE:%(filepath)s",
        ])
        .arg("--output")
        .arg(dir.join("video.%(ext)s"));
        if container == "mp3" {
            cmd.args([
                "--extract-audio",
                "--audio-format",
                "mp3",
                "--audio-quality",
                "0",
                "--embed-metadata",
                "--embed-thumbnail",
                "--convert-thumbnails",
                "jpg",
            ]);
        } else {
            cmd.args(["--merge-output-format", container, "--remux-video", container]);
        }
        if rate > 0 {
            cmd.arg("--limit-rate").arg(rate.to_string());
        }
        cmd.arg("--").arg(&job.url).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| TransferError::io("Could not start the video downloader", e))?;
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let stderr = child.stderr.take().unwrap();
        let error_reader = tokio::spawn(async move {
            let mut bytes = Vec::new();
            let _ = stderr.take(256 * 1024).read_to_end(&mut bytes).await;
            bytes
        });
        let mut output = None;
        let mut tracks: HashMap<String, u64> = HashMap::new();
        let mut restart = false;
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let mut activity = std::time::Instant::now();
        loop {
            tokio::select! {
                _ = cancel.cancelled() => { stop_child(&mut child).await; let _ = error_reader.await; return Err(Stop::Canceled); }
                _ = tick.tick() => {
                    if effective_rate() != rate { restart = true; stop_child(&mut child).await; break; }
                    if activity.elapsed() > Duration::from_secs(1800) {
                        stop_child(&mut child).await; let _ = error_reader.await;
                        return Err(invalid("Media processing stopped making progress. Pause and retry the download.").into());
                    }
                }
                line = lines.next_line() => match line {
                    Ok(Some(line)) => {
                        activity = std::time::Instant::now();
                        if let Some(json) = line.strip_prefix("CROWDM_PROGRESS:") {
                            if let Ok(p) = serde_json::from_str::<Value>(json) {
                                let bytes = p["downloaded_bytes"].as_u64().unwrap_or(0);
                                let key = p["filename"].as_str().unwrap_or("video").to_string();
                                let previous = tracks.insert(key, bytes).unwrap_or(bytes);
                                shared.received.fetch_add(bytes.saturating_sub(previous), Ordering::Relaxed);
                                let downloaded = tracks.values().sum();
                                if byte_limit.is_some_and(|limit| downloaded > limit) {
                                    stop_child(&mut child).await; let _ = error_reader.await;
                                    return Err(invalid("The stream exceeded three times its estimated size. It was stopped to avoid an unbounded download. Check the source before retrying.").into());
                                }
                                let total = estimate.or_else(|| p["total_bytes"].as_u64().or_else(|| p["total_bytes_estimate"].as_u64()));
                                shared.media_progress(downloaded, total.map(|s| s.max(downloaded)));
                                shared.connections.store(1, Ordering::Relaxed);
                                shared.set_phase(Some(if p["status"] == "finished" { if container == "mp3" { "Preparing MP3 audio" } else { "Merging video and audio" } } else { if container == "mp3" { "Downloading audio" } else { "Downloading video" } }));
                            }
                        } else if let Some(path) = line.strip_prefix("CROWDM_FILE:") { output = Some(PathBuf::from(path)); }
                    }
                    Ok(None) => break,
                    Err(_) => { stop_child(&mut child).await; break; }
                }
            }
        }
        let status = tokio::select! {
            _ = cancel.cancelled() => { stop_child(&mut child).await; let _ = error_reader.await; return Err(Stop::Canceled); }
            result = child.wait() => result.map_err(|e| TransferError::io("Video downloader stopped unexpectedly", e))?,
        };
        let errors = error_reader.await.unwrap_or_default();
        shared.connections.store(0, Ordering::Relaxed);
        if restart {
            continue;
        }
        if !status.success() {
            return Err(process_error(&errors).into());
        }
        let path = output.ok_or_else(|| invalid("The video downloader did not produce a file."))?;
        let actual = std::fs::canonicalize(&path).map_err(|e| TransferError::io("Could not find the downloaded video", e))?;
        let expected_dir = std::fs::canonicalize(&dir).map_err(|e| TransferError::io("Could not find video staging folder", e))?;
        if actual.parent() != Some(expected_dir.as_path()) || actual.extension().and_then(|e| e.to_str()) != Some(container) {
            return Err(invalid("The video downloader returned an unexpected output file.").into());
        }
        let mut verify = Command::new(tool("ffprobe"));
        hidden(&mut verify);
        verify
            .args(["-v", "error", "-protocol_whitelist", "file,pipe", "-show_entries", "format=duration:stream=codec_type", "-of", "json"])
            .arg(&actual);
        let check = tokio::time::timeout(Duration::from_secs(20), verify.output())
            .await
            .map_err(|_| invalid("The completed media file could not be verified in time."))?
            .map_err(|_| invalid("Could not verify the completed media file."))?;
        let checked: Value = serde_json::from_slice(&check.stdout).map_err(|_| invalid("The completed media file is invalid."))?;
        let streams = checked["streams"].as_array().ok_or_else(|| invalid("The completed file contains no media tracks."))?;
        let has = |kind| streams.iter().any(|s| s["codec_type"] == kind);
        if !check.status.success() || if container == "mp3" { !has("audio") } else { !has("video") } {
            return Err(invalid("The completed file does not contain the requested media tracks.").into());
        }
        if let (Some(expected), Some(actual)) =
            (info["duration"].as_f64(), checked["format"]["duration"].as_str().and_then(|s| s.parse::<f64>().ok()))
        {
            if actual > expected * 1.05 + 15.0 || actual + 15.0 < expected * 0.95 {
                return Err(invalid(
                    "The downloaded duration differs from the source. The stream may be incomplete or contain repeated segments.",
                )
                .into());
            }
        }
        std::fs::rename(&actual, job.part_path()).map_err(|e| TransferError::io("Could not prepare the completed video", e))?;
        let file = PartFile::open_existing(&job.part_path()).map_err(|e| TransferError::io("Could not open the completed video", e))?;
        file.sync().map_err(|e| TransferError::io("Could not save the completed video", e))?;
        let size = file.len().map_err(|e| TransferError::io("Could not measure the completed video", e))?;
        // Only this job's generated staging directory is removed.
        cleanup(&job);
        shared.media_progress(size, Some(size));
        return Ok(Completed { size, file });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hls_byte_ranges_count_only_the_requested_bytes() {
        let playlist = "#EXTM3U\n#EXT-X-MAP:URI=\"film.mp4\",BYTERANGE=\"100@0\"\n#EXT-X-BYTERANGE:200@100\nfilm.mp4\n#EXT-X-BYTERANGE:300@300\nfilm.mp4\n#EXT-X-ENDLIST";
        assert_eq!(hls_range_size(playlist, 2), Some(600));
        assert_eq!(hls_range_size("#EXTM3U\nsegment1.ts\nsegment2.ts", 2), None);
    }
    #[test]
    fn quality_menu_uses_real_video_heights_and_excludes_drm_and_audio() {
        let info = serde_json::json!({ "formats": [
            { "height": 1080, "vcodec": "avc1", "acodec": "none" }, { "height": 1080, "vcodec": "vp9", "acodec": "none" },
            { "height": 720, "acodec": "none" }, { "height": 2160, "vcodec": "avc1", "has_drm": true, "acodec": "none" },
            { "vcodec": "none", "height": 480, "acodec": "none" }
        ] });
        let choices = quality_options(&info);
        assert_eq!(
            choices.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
            vec!["Best video + audio", "1080p video + audio", "720p video + audio"]
        );
        assert_eq!(choices[1].format, "bv*[height=1080]+ba/b[height=1080]");
    }
    #[test]
    fn recognizes_watch_pages_and_manifests_without_matching_lookalike_hosts() {
        assert!(is_media_url("https://www.youtube.com/watch?v=abc"));
        assert!(is_media_url("https://youtu.be/abc"));
        assert!(is_media_url("https://example.com/stream.M3U8?token=1"));
        assert!(!is_media_url("https://youtube.com.evil.example/watch?v=abc"));
        assert!(!is_media_url("https://youtube.com/channel/abc"));
    }
    #[test]
    fn cookies_are_scoped_and_removed() {
        let path;
        {
            let session = session_file("https://example.com/video", &[("Cookie".into(), "token=abc; injected=x\tbad".into())]).unwrap();
            path = session.0.clone();
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.contains("example.com\tFALSE\t/\tTRUE\t0\ttoken\tabc"));
            assert!(!text.contains("injected"));
        }
        assert!(!path.exists());
    }
}
