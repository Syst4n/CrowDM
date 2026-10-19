#[path = "support/torrent_fixture.rs"]
mod fixture;
use crowdm_engine::*;
use std::{
    io::{Seek, Write},
    time::{Duration, Instant},
};

async fn wait(engine: &Engine, id: JobId, state: JobState) -> JobSnapshot {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let job = engine.job(id).unwrap();
            assert_ne!(job.job.state, JobState::Failed, "{:?}", job.job.error);
            if job.job.state == state {
                return job;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("torrent did not reach expected state")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn torrent_magnet_selection_resume_integrity_collision_and_queue() {
    let tmp = tempfile::tempdir().unwrap();
    let seed = fixture::Fixture::new(&tmp.path().join("seed"), false).await;
    let db = tmp.path().join("client.sqlite");
    let engine = Engine::open(&db).unwrap();
    engine.use_torrent_session(fixture::local_session(&tmp.path().join("downloads"), false, false).await).unwrap();
    let mut settings = engine.settings();
    settings.download_root = tmp.path().join("downloads");
    settings.resume_on_startup = false;
    engine.update_settings(settings).unwrap();
    engine.stop_queue();
    // No cached metadata: resolve the magnet through the real local tracker/peer.
    let info = engine.probe(ProbeRequest { url: seed.magnet.clone(), ..Default::default() }).await.unwrap();
    assert_eq!(info.torrent.as_ref().unwrap().files.len(), 2);
    let local = engine.probe(ProbeRequest { url: seed.path.to_string_lossy().into_owned(), ..Default::default() }).await.unwrap();
    assert_eq!(local.torrent.as_ref().unwrap().info_hash, info.torrent.as_ref().unwrap().info_hash);
    let destination = info.dir.join(&info.file_name);
    std::fs::create_dir_all(&destination).unwrap();
    std::fs::write(destination.join("keep.txt"), b"existing file").unwrap();
    let mut torrent = info.torrent.unwrap();
    torrent.select(Some(vec![0])).unwrap();
    let id = engine
        .add(NewJob {
            url: seed.magnet.clone(),
            torrent: Some(torrent),
            file_name: Some(info.file_name),
            speed_limit: Some(512 * 1024),
            start: true,
            ..Default::default()
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        while engine.job(id).unwrap().job.downloaded < 32768 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    engine.pause(id).unwrap();
    wait(&engine, id, JobState::Paused).await;
    let paused = engine.job(id).unwrap();
    assert!(paused.job.downloaded > 0 && paused.job.downloaded < seed.payload.len() as u64);
    // Corrupt a saved piece. Reopening must recheck and repair it from the peer.
    let mut part = std::fs::OpenOptions::new().write(true).open(torrent::staging_dir(&paused.job).join("payload.bin")).unwrap();
    part.seek(std::io::SeekFrom::Start(0)).unwrap();
    part.write_all(b"corrupt").unwrap();
    drop(part);
    engine.shutdown().await;
    drop(engine);
    let engine = Engine::open(&db).unwrap();
    engine.use_torrent_session(fixture::local_session(&tmp.path().join("downloads"), false, false).await).unwrap();
    engine.update_job(id, JobPatch { speed_limit: Some(None), ..Default::default() }).unwrap();
    engine.start(id).unwrap();
    let start = Instant::now();
    let complete = wait(&engine, id, JobState::Completed).await;
    assert_eq!(std::fs::read(complete.job.target_path().join("payload.bin")).unwrap(), seed.payload);
    assert!(!complete.job.target_path().join("readme.txt").exists());
    assert_eq!(std::fs::read(destination.join("keep.txt")).unwrap(), b"existing file");
    assert_eq!(complete.job.category, "torrents");
    assert_eq!(complete.job.size, Some(seed.payload.len() as u64));
    println!("Verified torrent resume throughput: {:.1} MiB/s", 16.0 / start.elapsed().as_secs_f64());
    // A new copy respects the held queue, then starts and completes all files.
    engine.stop_queue();
    let p = engine.probe(ProbeRequest { url: seed.path.to_string_lossy().into_owned(), ..Default::default() }).await.unwrap();
    let copy = engine
        .add(NewJob { url: seed.magnet.clone(), torrent: p.torrent, file_name: Some(p.file_name), start: false, ..Default::default() })
        .unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(engine.job(copy).unwrap().job.state, JobState::Queued);
    engine.start_queue();
    let copied = wait(&engine, copy, JobState::Completed).await;
    assert_eq!(std::fs::read(copied.job.target_path().join("payload.bin")).unwrap(), seed.payload);
    assert!(copied.job.target_path().join("readme.txt").is_file());
    // Removing an unfinished copy cleans only its staging data.
    let p = engine.probe(ProbeRequest { url: seed.path.to_string_lossy().into_owned(), ..Default::default() }).await.unwrap();
    let removed = engine
        .add(NewJob {
            url: seed.magnet.clone(),
            torrent: p.torrent,
            file_name: Some(p.file_name),
            speed_limit: Some(65536),
            start: true,
            ..Default::default()
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        while engine.job(removed).unwrap().job.downloaded == 0 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    let stage = torrent::staging_dir(&engine.job(removed).unwrap().job);
    engine.remove(removed).await.unwrap();
    assert!(!stage.exists());
    assert!(copied.job.target_path().join("payload.bin").exists());
    // Single-file metainfo saves a file, rather than a folder with its name.
    let tracker = librqbit::torrent_from_bytes(&seed.meta).unwrap().announce.unwrap().as_ref().to_vec();
    let single = fixture::single_metainfo("payload.bin", &seed.payload, std::str::from_utf8(&tracker).unwrap());
    seed.session
        .add_torrent(
            librqbit::AddTorrent::TorrentFileBytes(single.clone().into()),
            Some(librqbit::AddTorrentOptions {
                output_folder: Some(tmp.path().join("seed").to_string_lossy().into_owned()),
                overwrite: true,
                disable_trackers: true,
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap()
        .wait_until_completed()
        .await
        .unwrap();
    let single_path = tmp.path().join("single.torrent");
    std::fs::write(&single_path, single).unwrap();
    let p = engine.probe(ProbeRequest { url: single_path.to_string_lossy().into_owned(), ..Default::default() }).await.unwrap();
    assert!(!p.torrent.as_ref().unwrap().multi_file);
    let single_job = engine
        .add(NewJob {
            url: single_path.to_string_lossy().into_owned(),
            torrent: p.torrent,
            file_name: Some(p.file_name),
            start: true,
            ..Default::default()
        })
        .unwrap();
    let finished = wait(&engine, single_job, JobState::Completed).await;
    assert!(finished.job.target_path().is_file());
    assert_eq!(std::fs::read(finished.job.target_path()).unwrap(), seed.payload);
    engine.shutdown().await;
    seed.session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn utp_only_peer_transfer_is_byte_exact() {
    let _ = tracing_subscriber::fmt().with_env_filter("warn").with_test_writer().try_init();
    let tmp = tempfile::tempdir().unwrap();
    let seed = fixture::Fixture::new(&tmp.path().join("seed"), true).await;
    let engine = Engine::open(&tmp.path().join("client.sqlite")).unwrap();
    engine.use_torrent_session(fixture::local_session(&tmp.path().join("downloads"), true, false).await).unwrap();
    let mut s = engine.settings();
    s.download_root = tmp.path().join("downloads");
    engine.update_settings(s).unwrap();
    let p = engine.probe(ProbeRequest { url: seed.path.to_string_lossy().into_owned(), ..Default::default() }).await.unwrap();
    let id = engine
        .add(NewJob { url: seed.magnet.clone(), torrent: p.torrent, file_name: Some(p.file_name), start: true, ..Default::default() })
        .unwrap();
    let start = Instant::now();
    let job = wait(&engine, id, JobState::Completed).await;
    assert_eq!(std::fs::read(job.job.target_path().join("payload.bin")).unwrap(), seed.payload);
    println!("Verified uTP-only torrent throughput: {:.1} MiB/s", 16.0 / start.elapsed().as_secs_f64());
    engine.shutdown().await;
    seed.session.stop().await;
}

#[cfg(windows)]
#[tokio::test]
async fn torrent_finish_waits_for_transient_windows_file_locks() {
    use std::os::windows::fs::OpenOptionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("sample.torrent");
    std::fs::write(&path, fixture::single_metainfo("data.bin", b"verified bytes", "http://127.0.0.1:1/announce")).unwrap();
    let engine = Engine::open(&tmp.path().join("db.sqlite")).unwrap();
    engine.stop_queue();
    let p = engine.probe(ProbeRequest { url: path.to_string_lossy().into_owned(), ..Default::default() }).await.unwrap();
    let id = engine
        .add(NewJob {
            url: path.to_string_lossy().into_owned(),
            torrent: p.torrent,
            file_name: Some(p.file_name),
            dir: Some(tmp.path().join("downloads")),
            start: false,
            ..Default::default()
        })
        .unwrap();
    let job = engine.job(id).unwrap().job;
    let stage = torrent::staging_dir(&job);
    std::fs::create_dir_all(&stage).unwrap();
    let data = stage.join("data.bin");
    std::fs::write(&data, b"verified bytes").unwrap();
    let held = std::fs::OpenOptions::new().read(true).share_mode(3).open(&data).unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        drop(held);
    });
    let output = tokio::task::spawn_blocking(move || torrent::finish(&job)).await.unwrap().unwrap();
    release.join().unwrap();
    assert_eq!(std::fs::read(output).unwrap(), b"verified bytes");
    engine.shutdown().await;
}

#[test]
fn unsafe_torrent_paths_and_invalid_selection_are_rejected() {
    for names in [vec!["../escape"], vec!["CON.txt"], vec!["file:stream"], vec!["A", "a"], vec!["file", "file/nested"]] {
        let data = vec![1; 32768];
        let files = names.iter().map(|name| (*name, data.as_slice())).collect::<Vec<_>>();
        let bytes = fixture::metainfo("unsafe", &files, "http://127.0.0.1/announce");
        assert!(torrent::inspect(&bytes).is_err(), "accepted {names:?}");
    }
    let bytes = fixture::metainfo("valid", &[("file", &[1; 32768])], "http://127.0.0.1/announce");
    let (mut t, _) = torrent::inspect(&bytes).unwrap();
    assert!(t.select(Some(vec![])).is_err());
    assert!(t.select(Some(vec![99])).is_err());
    assert!(torrent::inspect(b"<html>not a torrent</html>").is_err());
}
