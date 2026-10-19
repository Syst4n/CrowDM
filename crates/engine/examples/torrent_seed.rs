//! Isolated, generated local swarm for desktop regression tests.
#[path = "../tests/support/torrent_fixture.rs"]
mod fixture;
#[tokio::main]
async fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("fixture directory"));
    let seed = fixture::Fixture::new(&root, false).await;
    let info = serde_json::json!({"path":seed.path,"magnet":seed.magnet});
    std::fs::write(root.join("ready.json"), serde_json::to_vec(&info).unwrap()).unwrap();
    std::future::pending::<()>().await;
}
