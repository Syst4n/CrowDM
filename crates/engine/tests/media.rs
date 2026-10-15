//! Real yt-dlp + FFmpeg integration against locally generated video. Opt in
//! with CROWDM_MEDIA_TESTS=1 after running scripts/prepare-media.ps1.
use crowdm_engine::*;
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn tool(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/media").join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}
async fn fixture_server(root: PathBuf) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let root = root.clone();
            tokio::spawn(async move {
                let mut data = Vec::new();
                loop {
                    let mut buf = [0; 4096];
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    data.extend_from_slice(&buf[..n]);
                    if data.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                    if data.len() > 65536 {
                        return;
                    }
                }
                let request = String::from_utf8_lossy(&data);
                let path = request.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap().trim_start_matches('/');
                if path.contains("..") {
                    return;
                }
                // Every manifest, media track and fragment needs the captured cookie.
                let authorized = request.to_ascii_lowercase().contains("cookie: session=fixture");
                let body = if authorized { std::fs::read(root.join(path)).ok() } else { None };
                if body.is_none() {
                    eprintln!("fixture missing: {path}, authorized={authorized}");
                }
                let mime = if path.ends_with(".m3u8") {
                    "application/vnd.apple.mpegurl"
                } else if path.ends_with(".mpd") {
                    "application/dash+xml"
                } else if path.ends_with(".ts") {
                    "video/mp2t"
                } else {
                    "video/mp4"
                };
                let bytes = body.as_deref().unwrap_or(b"denied");
                let headers = format!(
                    "HTTP/1.1 {}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    if body.is_some() { "200 OK" } else { "403 Forbidden" },
                    bytes.len()
                );
                let _ = socket.write_all(headers.as_bytes()).await;
                let _ = socket.write_all(bytes).await;
            });
        }
    });
    (url, task)
}
async fn wait(engine: &Engine, id: JobId, states: &[JobState]) -> JobSnapshot {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(100);
    loop {
        let snap = engine.job(id).unwrap();
        if states.contains(&snap.job.state) {
            return snap;
        }
        assert!(tokio::time::Instant::now() < deadline, "timed out: {:?} {:?}", snap.job.state, snap.job.error);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
fn assert_tracks(path: &Path) {
    let output = Command::new(tool("ffprobe"))
        .args(["-v", "error", "-show_entries", "stream=codec_type", "-of", "json"])
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let streams = json["streams"].as_array().unwrap();
    assert!(streams.iter().any(|s| s["codec_type"] == "video"));
    assert!(streams.iter().any(|s| s["codec_type"] == "audio"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_media_files_hls_dash_merge_pause_resume_and_queue() {
    if std::env::var("CROWDM_MEDIA_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIP: set CROWDM_MEDIA_TESTS=1 for real media tools");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let fixture = dir.path().join("fixture");
    std::fs::create_dir(&fixture).unwrap();
    let generate = Command::new(tool("ffmpeg"))
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=20",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440",
            "-t",
            "8",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-g",
            "20",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-y",
        ])
        .arg(fixture.join("clip.mp4"))
        .output()
        .unwrap();
    assert!(generate.status.success(), "{}", String::from_utf8_lossy(&generate.stderr));
    for (format, filename) in [("hls", "index.m3u8"), ("dash", "index.mpd")] {
        let output = Command::new(tool("ffmpeg"))
            .current_dir(&fixture)
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(fixture.join("clip.mp4"))
            .args(["-c", "copy", "-f", format, "-y"])
            .arg(fixture.join(filename))
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }
    let low = Command::new(tool("ffmpeg"))
        .current_dir(&fixture)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(fixture.join("clip.mp4"))
        .args(["-vf", "scale=160:90", "-c:v", "libx264", "-c:a", "aac", "-f", "hls", "-y"])
        .arg(fixture.join("low.m3u8"))
        .output()
        .unwrap();
    assert!(low.status.success());
    std::fs::write(fixture.join("master.m3u8"), "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=500000,RESOLUTION=320x180\nindex.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=100000,RESOLUTION=160x90\nlow.m3u8\n").unwrap();
    let live = std::fs::read_to_string(fixture.join("index.m3u8")).unwrap().replace("#EXT-X-ENDLIST", "");
    std::fs::write(fixture.join("live.m3u8"), live).unwrap();
    let (url, server) = fixture_server(fixture).await;
    let db = dir.path().join("test.sqlite");
    let mut engine = Engine::open(&db).unwrap();
    let mut settings = engine.settings();
    settings.download_root = dir.path().join("downloads");
    settings.organize_by_category = false;
    settings.max_active_downloads = 1;
    engine.update_settings(settings).unwrap();
    let unbounded = engine
        .probe_media(ProbeRequest {
            url: format!("{url}/live.m3u8"),
            headers: vec![("Cookie".into(), "session=fixture".into())],
            ..Default::default()
        })
        .await;
    assert!(unbounded.is_err(), "Live HLS must be rejected before adding a download");
    for filename in ["clip.mp4", "index.m3u8", "index.mpd"] {
        let headers = vec![("Cookie".into(), "session=fixture".into())];
        let info = engine
            .probe_media(ProbeRequest { url: format!("{url}/{filename}"), headers: headers.clone(), ..Default::default() })
            .await
            .unwrap();
        assert!(!info.looks_like_html);
        assert!(info.file_name.ends_with(".mkv"));
        let id = engine
            .add(NewJob {
                media_download: true,
                url: info.final_url.clone(),
                headers,
                file_name: Some(info.file_name),
                category: Some("videos".into()),
                source: JobSource { kind: SourceKind::Video, ..Default::default() },
                speed_limit: if filename == "clip.mp4" { Some(50_000) } else { None },
                ..Default::default()
            })
            .unwrap();
        if filename == "clip.mp4" {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            while engine.job(id).unwrap().job.downloaded == 0 {
                assert!(tokio::time::Instant::now() < deadline);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            engine.pause(id).unwrap();
            let paused = wait(&engine, id, &[JobState::Paused]).await;
            assert!(media::staging_dir(&paused.job).exists());
            // Restore the paused media job from disk, then remove a live
            // rate cap (the extractor must stop and resume its child process).
            engine.shutdown().await;
            drop(engine);
            engine = Engine::open(&db).unwrap();
            assert_eq!(engine.job(id).unwrap().job.state, JobState::Paused);
            engine.start(id).unwrap();
            tokio::time::sleep(Duration::from_secs(2)).await;
            engine.update_job(id, JobPatch { speed_limit: Some(None), ..Default::default() }).unwrap();
        }
        let completed = wait(&engine, id, &[JobState::Completed, JobState::Failed]).await;
        assert_eq!(completed.job.state, JobState::Completed, "{filename}: {:?}", completed.job.error);
        assert!(completed.job.headers.is_empty());
        assert!(!media::staging_dir(&completed.job).exists());
        assert_tracks(&completed.job.target_path());
    }
    engine.stop_queue();
    let info = engine
        .probe_media(ProbeRequest {
            url: format!("{url}/master.m3u8"),
            headers: vec![("Cookie".into(), "session=fixture".into())],
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(info.media_options.iter().any(|f| f.height == Some(180)));
    let selected = info.media_options.iter().find(|f| f.height == Some(90)).unwrap().format.clone();
    let quality = engine
        .add(NewJob {
            url: info.final_url,
            media_download: true,
            media_format: Some(selected),
            file_name: Some("quality90.mkv".into()),
            start: true,
            headers: vec![("Cookie".into(), "session=fixture".into())],
            source: JobSource { kind: SourceKind::Video, ..Default::default() },
            ..Default::default()
        })
        .unwrap();
    let quality_done = wait(&engine, quality, &[JobState::Completed, JobState::Failed]).await;
    assert_eq!(quality_done.job.state, JobState::Completed, "{:?}", quality_done.job.error);
    let dimensions = Command::new(tool("ffprobe"))
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=height", "-of", "csv=p=0"])
        .arg(quality_done.job.target_path())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&dimensions.stdout).trim(), "90");
    let queued = engine
        .add(NewJob {
            media_download: true,
            url: format!("{url}/index.m3u8"),
            file_name: Some("queued.mkv".into()),
            headers: vec![("Cookie".into(), "session=fixture".into())],
            start: false,
            source: JobSource { kind: SourceKind::Video, ..Default::default() },
            ..Default::default()
        })
        .unwrap();
    assert_eq!(engine.job(queued).unwrap().job.state, JobState::Queued);
    engine.start_queue();
    assert_eq!(wait(&engine, queued, &[JobState::Completed, JobState::Failed]).await.job.state, JobState::Completed);
    for (selection, extension, category) in [("mp4:b[ext=mp4]", "mp4", "videos"), ("mp3:ba/b", "mp3", "audio")] {
        let info = engine
            .probe_media(ProbeRequest {
                url: format!("{url}/clip.mp4"),
                media_format: Some(selection.into()),
                headers: vec![("Cookie".into(), "session=fixture".into())],
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(info.file_name.ends_with(extension));
        assert_eq!(info.category, category);
        assert!(info.size.is_some());
        let id = engine
            .add(NewJob {
                media_download: true,
                media_format: Some(selection.into()),
                url: info.final_url,
                file_name: Some(format!("output.{extension}")),
                headers: vec![("Cookie".into(), "session=fixture".into())],
                ..Default::default()
            })
            .unwrap();
        let done = wait(&engine, id, &[JobState::Completed, JobState::Failed]).await;
        assert_eq!(done.job.state, JobState::Completed, "{extension}: {:?}", done.job.error);
        assert_eq!(done.job.category, category);
        if extension == "mp4" {
            assert_tracks(&done.job.target_path());
        } else {
            let output = Command::new(tool("ffprobe"))
                .args(["-v", "error", "-show_entries", "stream=codec_type,codec_name", "-of", "json"])
                .arg(done.job.target_path())
                .output()
                .unwrap();
            let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            let streams = value["streams"].as_array().unwrap();
            assert!(streams.iter().any(|s| s["codec_name"] == "mp3"));
            assert!(!streams.iter().any(|s| s["codec_type"] == "video"));
        }
    }
    engine.shutdown().await;
    let restored = Engine::open(&db).unwrap();
    assert_eq!(restored.jobs().len(), 7);
    assert!(restored.jobs().iter().all(|j| j.job.state == JobState::Completed));
    restored.shutdown().await;
    server.abort();
}
