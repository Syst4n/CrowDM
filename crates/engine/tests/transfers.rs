//! End-to-end transfer tests against the controllable local server.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crowdm_engine::store::Store;
use crowdm_engine::*;
use crowdm_testserver::{content, TestServer};

const MIB: u64 = 1024 * 1024;

struct Env {
    engine: Engine,
    dir: tempfile::TempDir,
    server: TestServer,
}

fn test_root() -> PathBuf {
    std::env::var_os("CROWDM_TEST_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

async fn setup_with(store: Store, dir: tempfile::TempDir, server: TestServer) -> Env {
    let engine = Engine::with_store(store).unwrap();
    let settings = Settings {
        download_root: dir.path().to_path_buf(),
        organize_by_category: false,
        retry_attempts: 2,
        retry_delay_secs: 1,
        ..engine.settings()
    };
    engine.update_settings(settings).unwrap();
    Env { engine, dir, server }
}

async fn setup() -> Env {
    std::fs::create_dir_all(test_root()).unwrap();
    let dir = tempfile::tempdir_in(test_root()).unwrap();
    let server = TestServer::start().await.unwrap();
    setup_with(Store::open_in_memory().unwrap(), dir, server).await
}

fn job(url: String) -> NewJob {
    NewJob { url, start: true, ..Default::default() }
}

/// Waits until the job reaches one of the given states.
async fn wait_for(engine: &Engine, id: JobId, states: &[JobState], timeout: Duration) -> JobSnapshot {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let snap = engine.job(id).expect("job exists");
        // "Downloading" with no running phase change is not terminal.
        if states.contains(&snap.job.state) && !(snap.job.state == JobState::Downloading && states.len() > 1) {
            return snap;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("timed out waiting for {states:?}; job is {:?} ({:?})", snap.job.state, snap.job.error);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn wait_done(engine: &Engine, id: JobId) -> JobSnapshot {
    wait_for(engine, id, &[JobState::Completed, JobState::Failed], Duration::from_secs(60)).await
}

fn assert_content(path: &Path, seed: u64, size: u64) {
    assert_eq!(content::verify_file(path, seed, size).unwrap(), None, "content mismatch in {}", path.display());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multi_connection_download_is_byte_exact() {
    let env = setup().await;
    let url = env.server.url("/file?size=20971520&seed=11&etag=a&id=multi&name=multi.bin");
    let id = env.engine.add(job(url)).unwrap();
    let snap = wait_done(&env.engine, id).await;
    assert_eq!(snap.job.state, JobState::Completed, "{:?}", snap.job.error);
    assert_eq!(snap.job.file_name, "multi.bin");
    assert_eq!(snap.job.resumable, Some(true));
    assert_content(&env.dir.path().join("multi.bin"), 11, 20 * MIB);
    assert!(!env.dir.path().join("multi.bin.crowdm-part").exists());
    let stats = env.server.stats("multi");
    assert!(stats.ranged_requests >= 2, "expected several ranged connections, got {stats:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_without_range_support_uses_one_connection() {
    let env = setup().await;
    let url = env.server.url("/file?size=5000000&seed=3&norange&id=norange&name=nr.bin");
    let id = env.engine.add(job(url)).unwrap();
    let snap = wait_done(&env.engine, id).await;
    assert_eq!(snap.job.state, JobState::Completed, "{:?}", snap.job.error);
    assert_eq!(snap.job.resumable, Some(false));
    assert_content(&env.dir.path().join("nr.bin"), 3, 5_000_000);
    assert_eq!(env.server.stats("norange").requests, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_that_ignores_ranges_falls_back_safely() {
    let env = setup().await;
    // Advertises Accept-Ranges but always answers 200 with the full body.
    let url = env.server.url("/file?size=12000000&seed=5&ignorerange&etag=x&id=ign&name=ign.bin");
    let id = env.engine.add(job(url)).unwrap();
    let snap = wait_done(&env.engine, id).await;
    assert_eq!(snap.job.state, JobState::Completed, "{:?}", snap.job.error);
    assert_content(&env.dir.path().join("ign.bin"), 5, 12_000_000);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unknown_length_download_completes() {
    let env = setup().await;
    let url = env.server.url("/file?size=3333333&seed=9&chunked&name=unknown.dat");
    let id = env.engine.add(job(url)).unwrap();
    let snap = wait_done(&env.engine, id).await;
    assert_eq!(snap.job.state, JobState::Completed, "{:?}", snap.job.error);
    assert_eq!(snap.job.size, Some(3_333_333));
    assert_content(&env.dir.path().join("unknown.dat"), 9, 3_333_333);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn interrupted_connections_are_resumed() {
    let env = setup().await;
    // The first 10 responses are cut off after 700 KB.
    let url = env.server.url("/file?size=16777216&seed=21&etag=d&dropafter=700000&dropfirst=10&id=drop&name=drop.bin");
    let id = env.engine.add(job(url)).unwrap();
    let snap = wait_done(&env.engine, id).await;
    assert_eq!(snap.job.state, JobState::Completed, "{:?}", snap.job.error);
    assert_content(&env.dir.path().join("drop.bin"), 21, 16 * MIB);
    assert!(env.server.stats("drop").requests > 10);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn redirects_are_followed_and_recorded() {
    let env = setup().await;
    let target = "/file?size=2097152&seed=2&etag=r&id=redir&name=redirected.zip";
    let enc: String = url::form_urlencoded::byte_serialize(target.as_bytes()).collect();
    let url = env.server.url(&format!("/redirect?n=3&to={enc}"));
    let id = env.engine.add(job(url)).unwrap();
    let snap = wait_done(&env.engine, id).await;
    assert_eq!(snap.job.state, JobState::Completed, "{:?}", snap.job.error);
    assert!(snap.job.final_url.as_deref().unwrap().contains("/file?"));
    assert_content(&env.dir.path().join("redirected.zip"), 2, 2 * MIB);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn name_comes_from_url_when_no_disposition() {
    let env = setup().await;
    let url = env.server.url("/file?size=1000&seed=1");
    let id = env.engine.add(job(url)).unwrap();
    let snap = wait_done(&env.engine, id).await;
    assert_eq!(snap.job.state, JobState::Completed);
    // "/file" has no extension; the MIME type is octet-stream, so no guess.
    assert_eq!(snap.job.file_name, "file");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_and_resume_continue_where_they_stopped() {
    let env = setup().await;
    let url = env.server.url("/file?size=8388608&seed=4&etag=p&rate=1500000&id=pause&name=pause.bin");
    let mut nj = job(url);
    nj.max_connections = Some(2);
    let id = env.engine.add(nj).unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    env.engine.pause(id).unwrap();
    let paused = wait_for(&env.engine, id, &[JobState::Paused], Duration::from_secs(10)).await;
    assert!(paused.job.downloaded > 500_000, "downloaded {}", paused.job.downloaded);
    assert!(paused.job.downloaded < 8 * MIB);
    assert!(env.dir.path().join("pause.bin.crowdm-part").exists());
    let before = env.server.stats("pause").requests;
    env.engine.start(id).unwrap();
    let snap = wait_done(&env.engine, id).await;
    assert_eq!(snap.job.state, JobState::Completed, "{:?}", snap.job.error);
    assert_content(&env.dir.path().join("pause.bin"), 4, 8 * MIB);
    // Resumed with ranged requests rather than starting over.
    assert!(env.server.stats("pause").ranged_requests > before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn download_survives_an_app_restart() {
    std::fs::create_dir_all(test_root()).unwrap();
    let dir = tempfile::tempdir_in(test_root()).unwrap();
    let db = dir.path().join("state.sqlite");
    let server = TestServer::start().await.unwrap();
    let url = server.url("/file?size=6291456&seed=8&etag=s&rate=2000000&id=restart&name=restart.bin");
    let dl_dir = dir.path().join("dl");
    let id;
    {
        let engine = Engine::with_store(Store::open(&db).unwrap()).unwrap();
        let s = Settings { download_root: dl_dir.clone(), organize_by_category: false, ..engine.settings() };
        engine.update_settings(s).unwrap();
        let mut nj = job(url.clone());
        nj.max_connections = Some(2);
        id = engine.add(nj).unwrap();
        tokio::time::sleep(Duration::from_millis(1200)).await;
        engine.shutdown().await;
        let snap = engine.job(id).unwrap();
        assert_eq!(snap.job.state, JobState::Downloading, "stays resumable across restarts");
        assert!(snap.job.downloaded > 0);
    }
    // A new engine on the same database resumes the transfer by itself.
    let engine = Engine::with_store(Store::open(&db).unwrap()).unwrap();
    let resumed_from = engine.job(id).unwrap().job.downloaded;
    assert!(resumed_from > 0, "progress was persisted");
    let snap = wait_done(&engine, id).await;
    assert_eq!(snap.job.state, JobState::Completed, "{:?}", snap.job.error);
    assert_content(&dl_dir.join("restart.bin"), 8, 6 * MIB);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn changed_remote_file_is_detected_on_resume() {
    let env = setup().await;
    let url = env.server.url("/file?size=8388608&seed=6&etag=c&rate=1000000&id=chg&name=chg.bin");
    let mut nj = job(url);
    nj.max_connections = Some(2);
    let id = env.engine.add(nj).unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;
    env.engine.pause(id).unwrap();
    wait_for(&env.engine, id, &[JobState::Paused], Duration::from_secs(10)).await;
    env.server.bump("chg");
    env.engine.start(id).unwrap();
    let snap = wait_done(&env.engine, id).await;
    assert_eq!(snap.job.state, JobState::Failed);
    assert_eq!(snap.job.error.unwrap().kind, ErrorKind::ResourceChanged);
    // The stale partial data is not presented as a finished file.
    assert!(!env.dir.path().join("chg.bin").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stalled_connections_are_taken_over() {
    let env = setup().await;
    // Three extra connections never get an answer; the others must finish
    // their segments instead of waiting for a timeout.
    let url = env.server.url("/file?size=33554432&seed=41&etag=st&rate=4000000&stallfirst=3&id=stall&name=stall.bin");
    let start = std::time::Instant::now();
    let id = env.engine.add(job(url)).unwrap();
    let snap = wait_done(&env.engine, id).await;
    assert_eq!(snap.job.state, JobState::Completed, "{:?}", snap.job.error);
    assert_content(&env.dir.path().join("stall.bin"), 41, 32 * MIB);
    let secs = start.elapsed().as_secs_f64();
    assert!(secs < 25.0, "took {secs:.1}s; stalled connections were waited for");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn missing_file_fails_without_retrying_forever() {
    let env = setup().await;
    let id = env.engine.add(job(env.server.url("/status?code=404"))).unwrap();
    let snap = wait_for(&env.engine, id, &[JobState::Failed], Duration::from_secs(10)).await;
    assert_eq!(snap.job.error.unwrap().kind, ErrorKind::NotFound);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connection_limit_of_server_is_respected() {
    let env = setup().await;
    let url = env.server.url("/file?size=10485760&seed=12&etag=l&maxconn=2&id=limit&name=limit.bin");
    let id = env.engine.add(job(url)).unwrap();
    let snap = wait_done(&env.engine, id).await;
    assert_eq!(snap.job.state, JobState::Completed, "{:?}", snap.job.error);
    assert_content(&env.dir.path().join("limit.bin"), 12, 10 * MIB);
    assert!(env.server.stats("limit").max_active <= 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn checksum_is_verified() {
    let env = setup().await;
    let size = 3_000_000u64;
    let mut data = vec![0u8; size as usize];
    content::fill(15, 0, &mut data);
    let good = {
        use sha2::Digest;
        hex::encode(sha2::Sha256::digest(&data))
    };
    let mut ok = job(env.server.url("/file?size=3000000&seed=15&etag=h&name=good.bin"));
    ok.checksum = Some(Checksum { algo: ChecksumAlgo::Sha256, expected: good.to_uppercase() });
    let id = env.engine.add(ok).unwrap();
    assert_eq!(wait_done(&env.engine, id).await.job.state, JobState::Completed);

    let mut bad = job(env.server.url("/file?size=3000000&seed=15&etag=h&name=bad.bin"));
    bad.checksum = Some(Checksum { algo: ChecksumAlgo::Sha256, expected: "00".repeat(32) });
    let id = env.engine.add(bad).unwrap();
    let snap = wait_done(&env.engine, id).await;
    assert_eq!(snap.job.state, JobState::Failed);
    assert_eq!(snap.job.error.unwrap().kind, ErrorKind::ChecksumMismatch);
    assert!(!env.dir.path().join("bad.bin").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn speed_limit_is_applied() {
    let env = setup().await;
    let mut nj = job(env.server.url("/file?size=6000000&seed=1&etag=sl&name=slow.bin"));
    nj.speed_limit = Some(2_000_000);
    let start = std::time::Instant::now();
    let id = env.engine.add(nj).unwrap();
    let snap = wait_done(&env.engine, id).await;
    assert_eq!(snap.job.state, JobState::Completed);
    let secs = start.elapsed().as_secs_f64();
    assert!(secs > 2.2, "6 MB at 2 MB/s finished in {secs:.2}s");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn existing_files_are_never_overwritten() {
    let env = setup().await;
    std::fs::write(env.dir.path().join("same.bin"), b"keep me").unwrap();
    let id = env.engine.add(job(env.server.url("/file?size=1000&seed=1&name=same.bin"))).unwrap();
    let snap = wait_done(&env.engine, id).await;
    assert_eq!(snap.job.file_name, "same (1).bin");
    assert_eq!(std::fs::read(env.dir.path().join("same.bin")).unwrap(), b"keep me");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn probe_connection_is_reused_by_the_download() {
    let env = setup().await;
    let url = env.server.url("/file?size=4000000&seed=31&etag=pr&id=probe&name=probed.iso");
    let info = env.engine.probe(ProbeRequest { url: url.clone(), ..Default::default() }).await.unwrap();
    assert_eq!(info.file_name, "probed.iso");
    assert_eq!(info.size, Some(4_000_000));
    assert!(info.resumable);
    assert_eq!(info.category, "archives");
    let mut nj = job(url);
    nj.probe_id = info.probe_id;
    nj.max_connections = Some(1);
    let id = env.engine.add(nj).unwrap();
    let snap = wait_done(&env.engine, id).await;
    assert_eq!(snap.job.state, JobState::Completed);
    assert_content(&env.dir.path().join("probed.iso"), 31, 4_000_000);
    assert_eq!(env.server.stats("probe").requests, 1, "the probe response carried the whole download");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queue_limits_concurrency_and_reports_completion() {
    let env = setup().await;
    let mut s = env.engine.settings();
    s.max_active_downloads = 2;
    env.engine.update_settings(s).unwrap();
    let mut events = env.engine.subscribe();
    let mut ids = vec![];
    for i in 0..4 {
        let mut nj = job(env.server.url(&format!("/file?size=1500000&seed={i}&rate=3000000&name=q{i}.bin")));
        nj.start = false;
        ids.push(env.engine.add(nj).unwrap());
    }
    let mut max_running = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let running = ids.iter().filter(|id| env.engine.job(**id).unwrap().job.state == JobState::Downloading).count();
        max_running = max_running.max(running);
        if ids.iter().all(|id| env.engine.job(*id).unwrap().job.state == JobState::Completed) {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "queue did not finish");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(max_running <= 2, "ran {max_running} at once");
    let mut finished = false;
    while let Ok(ev) = tokio::time::timeout(Duration::from_millis(1500), events.recv()).await {
        if matches!(ev, Ok(EngineEvent::QueueFinished)) {
            finished = true;
            break;
        }
    }
    assert!(finished, "QueueFinished was emitted");
}

/// Writes land at their true offsets inside a 100 GiB (or as large as the
/// disk allows) sparse file without allocating the whole length.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn huge_file_is_sparse_and_bounded() {
    let env = setup().await;
    let free = file::available_space(env.dir.path()).unwrap_or(0);
    let size = (100 * 1024 * MIB).min(free / 2);
    if size < 4 * 1024 * MIB {
        eprintln!("skipping: not enough free disk space for the sparse test");
        return;
    }
    let mut nj = job(env.server.url(&format!("/file?size={size}&seed=77&etag=huge&name=huge.bin")));
    nj.max_connections = Some(8);
    let id = env.engine.add(nj).unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    env.engine.pause(id).unwrap();
    let snap = wait_for(&env.engine, id, &[JobState::Paused], Duration::from_secs(20)).await;
    assert_eq!(snap.job.size, Some(size));
    let part = env.dir.path().join("huge.bin.crowdm-part");
    let meta = std::fs::metadata(&part).unwrap();
    assert_eq!(meta.len(), size, "part file has the final length");
    let allocated = file::allocated_size(&part).unwrap();
    assert!(allocated < size / 4, "allocated {allocated} of {size}");
    // Every segment's written prefix holds the right bytes, including the
    // ones far from the start of the file.
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(&part).unwrap();
    let mut far = 0;
    for (start, _end, done) in snap.segments.iter().copied() {
        if done == 0 {
            continue;
        }
        let n = done.min(256 * 1024) as usize;
        let mut got = vec![0u8; n];
        f.seek(SeekFrom::Start(start)).unwrap();
        f.read_exact(&mut got).unwrap();
        let mut want = vec![0u8; n];
        content::fill(77, start, &mut want);
        assert_eq!(got, want, "segment at {start}");
        if start > size / 2 {
            far += 1;
        }
    }
    assert!(far > 0, "some segments started in the second half of the file");
    env.engine.remove(id).await.unwrap();
    assert!(!part.exists());
}
