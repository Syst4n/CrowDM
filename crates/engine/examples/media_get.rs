//! Smoke-test the full video path: cargo run -p crowdm-engine --example
//! media_get -- <video-page-url> <destination-directory>
use crowdm_engine::*;
use std::{path::PathBuf, time::Duration};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let url = args.next().ok_or("Provide a video URL and destination directory")?;
    let dir = PathBuf::from(args.next().ok_or("Provide a destination directory")?);
    std::fs::create_dir_all(&dir)?;
    let engine = Engine::open(&dir.join("media-smoke.sqlite"))?;
    let mut settings = engine.settings();
    settings.download_root = dir;
    settings.organize_by_category = false;
    engine.update_settings(settings)?;
    let selection = args.next();
    let info = engine.probe_media(ProbeRequest { url, media_format: selection.clone(), ..Default::default() }).await?;
    println!("Resolved: {}", info.file_name);
    let id = engine.add(NewJob {
        media_download: true,
        media_format: selection,
        url: info.final_url,
        file_name: Some(info.file_name),
        category: Some(info.category),
        size_hint: info.size,
        source: JobSource { kind: SourceKind::Video, ..Default::default() },
        ..Default::default()
    })?;
    loop {
        let snap = engine.job(id).unwrap();
        if snap.job.state == JobState::Failed {
            engine.shutdown().await;
            return Err(snap.job.error.unwrap().message.into());
        }
        if snap.job.state == JobState::Completed {
            println!("Saved: {} ({} bytes)", snap.job.target_path().display(), snap.job.downloaded);
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    engine.shutdown().await;
    Ok(())
}
