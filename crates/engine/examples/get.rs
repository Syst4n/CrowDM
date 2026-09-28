//! Command-line downloader on top of the engine, used for benchmarks.
//!
//! ```text
//! cargo run --release -p crowdm-engine --example get -- <url> [--connections N] [--out DIR]
//! ```
//! Prints elapsed time, average speed and the process's peak memory.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crowdm_engine::store::Store;
use crowdm_engine::{Engine, JobState, NewJob, Settings};

#[tokio::main]
async fn main() {
    if let Ok(filter) = std::env::var("CROWDM_LOG") {
        tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let url = args.first().cloned().expect("usage: get <url> [--connections N] [--out DIR]");
    let opt = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let conns: u8 = opt("--connections").and_then(|v| v.parse().ok()).unwrap_or(8);
    let out = PathBuf::from(opt("--out").unwrap_or_else(|| std::env::temp_dir().join("crowdm-get").to_string_lossy().into_owned()));
    std::fs::create_dir_all(&out).unwrap();

    let engine = Engine::with_store(Store::open_in_memory().unwrap()).unwrap();
    engine
        .update_settings(Settings {
            download_root: out.clone(),
            organize_by_category: false,
            max_connections_per_download: conns,
            ..engine.settings()
        })
        .unwrap();
    let start = Instant::now();
    let id = engine.add(NewJob { url, start: true, max_connections: Some(conns), ..Default::default() }).unwrap();
    let mut peak_conns = 0;
    let snap = loop {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let s = engine.job(id).unwrap();
        peak_conns = peak_conns.max(s.connections);
        if matches!(s.job.state, JobState::Completed | JobState::Failed) {
            break s;
        }
    };
    let secs = start.elapsed().as_secs_f64();
    let size = snap.job.size.unwrap_or(snap.job.downloaded);
    println!(
        "{{\"state\":\"{:?}\",\"file\":{:?},\"bytes\":{},\"seconds\":{:.3},\"mbPerSec\":{:.2},\"connections\":{},\"peakConnections\":{},\"peakWorkingSetMb\":{:.1},\"error\":{:?}}}",
        snap.job.state,
        snap.job.target_path(),
        size,
        secs,
        size as f64 / secs / 1_048_576.0,
        conns,
        peak_conns,
        peak_working_set() as f64 / 1_048_576.0,
        snap.job.error.as_ref().map(|e| e.message.clone())
    );
    if snap.job.state == JobState::Completed && std::env::var_os("KEEP").is_none() {
        let _ = std::fs::remove_file(snap.job.target_path());
    }
}

#[cfg(windows)]
fn peak_working_set() -> u64 {
    use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut c: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    c.PeakWorkingSetSize as u64
}

#[cfg(not(windows))]
fn peak_working_set() -> u64 {
    0
}
