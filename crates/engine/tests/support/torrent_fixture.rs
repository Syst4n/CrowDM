//! Generated, loopback-only torrent swarm; no third-party content or public trackers.
use librqbit::{AddTorrent, AddTorrentOptions, ConnectionOptions, ListenerMode, ListenerOptions, Session, SessionOptions};
use sha1::{Digest, Sha1};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

pub fn string(v: &[u8]) -> Vec<u8> {
    [format!("{}:", v.len()).as_bytes(), v].concat()
}
pub fn integer(v: usize) -> Vec<u8> {
    format!("i{v}e").into_bytes()
}
pub fn list(items: Vec<Vec<u8>>) -> Vec<u8> {
    [vec![b'l'], items.concat(), vec![b'e']].concat()
}
pub fn dict(mut items: Vec<(&str, Vec<u8>)>) -> Vec<u8> {
    items.sort_by_key(|p| p.0);
    [vec![b'd'], items.into_iter().flat_map(|(k, v)| [string(k.as_bytes()), v].concat()).collect(), vec![b'e']].concat()
}
pub fn metainfo(name: &str, files: &[(&str, &[u8])], tracker: &str) -> Vec<u8> {
    let data = files.iter().flat_map(|(_, b)| b.iter().copied()).collect::<Vec<_>>();
    let hashes = data.chunks(16384).flat_map(|chunk| Sha1::digest(chunk).to_vec()).collect::<Vec<_>>();
    let files = list(
        files
            .iter()
            .map(|(p, b)| dict(vec![("length", integer(b.len())), ("path", list(p.split('/').map(|s| string(s.as_bytes())).collect()))]))
            .collect(),
    );
    dict(vec![
        ("announce", string(tracker.as_bytes())),
        (
            "info",
            dict(vec![("files", files), ("name", string(name.as_bytes())), ("piece length", integer(16384)), ("pieces", string(&hashes))]),
        ),
    ])
}
#[allow(dead_code)]
pub fn single_metainfo(name: &str, data: &[u8], tracker: &str) -> Vec<u8> {
    let pieces = data.chunks(16384).flat_map(|chunk| Sha1::digest(chunk).to_vec()).collect::<Vec<_>>();
    dict(vec![
        ("announce", string(tracker.as_bytes())),
        (
            "info",
            dict(vec![
                ("length", integer(data.len())),
                ("name", string(name.as_bytes())),
                ("piece length", integer(16384)),
                ("pieces", string(&pieces)),
            ]),
        ),
    ])
}

#[allow(dead_code)] // Different regression runners use different fixture fields.
pub struct Fixture {
    pub session: Arc<Session>,
    pub meta: Vec<u8>,
    pub magnet: String,
    pub path: PathBuf,
    pub payload: Vec<u8>,
    tracker: tokio::task::JoinHandle<()>,
}
impl Fixture {
    pub async fn new(root: &Path, utp: bool) -> Self {
        std::fs::create_dir_all(root).unwrap();
        let payload = (0..16 * 1024 * 1024).map(|i| ((i * 31 + i / 251) % 256) as u8).collect::<Vec<_>>();
        let readme = b"Generated CrowDM local test data.\n";
        std::fs::write(root.join("payload.bin"), &payload).unwrap();
        std::fs::write(root.join("readme.txt"), readme).unwrap();
        let session = local_session(root, utp, true).await;
        let addr = session.listen_addr().unwrap();
        let tracker = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let tracker_url = format!("http://127.0.0.1:{}/announce", tracker.local_addr().unwrap().port());
        let meta = metainfo("CrowDM fixture", &[("payload.bin", &payload), ("readme.txt", readme)], &tracker_url);
        let meta_http = meta.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = tracker.accept().await else { break };
                let meta = meta_http.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0; 16384];
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]);
                    let body = if request.contains("/fixture.torrent") {
                        meta
                    } else {
                        dict(vec![
                            ("interval", integer(1)),
                            ("peers", string(&[127, 0, 0, 1, (addr.port() >> 8) as u8, addr.port() as u8])),
                        ])
                    };
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/x-bittorrent\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(header.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                });
            }
        });
        let handle = session
            .add_torrent(
                AddTorrent::TorrentFileBytes(meta.clone().into()),
                Some(AddTorrentOptions {
                    output_folder: Some(root.to_string_lossy().into_owned()),
                    overwrite: true,
                    disable_trackers: true,
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(30), handle.wait_until_completed()).await.unwrap().unwrap();
        let hash = handle.info_hash().as_string();
        let magnet = format!(
            "magnet:?xt=urn:btih:{hash}&dn=CrowDM%20fixture&tr={}",
            url::form_urlencoded::byte_serialize(tracker_url.as_bytes()).collect::<String>()
        );
        let path = root.join("fixture with spaces.torrent");
        std::fs::write(&path, &meta).unwrap();
        Self { session, meta, magnet, path, payload, tracker: task }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.tracker.abort();
    }
}
pub async fn local_session(root: &Path, utp: bool, seeder: bool) -> Arc<Session> {
    Session::new_with_opts(
        root.into(),
        SessionOptions {
            dht: None,
            disable_local_service_discovery: true,
            disable_trackers: seeder,
            listen: Some(ListenerOptions {
                listen_addr: ([127, 0, 0, 1], crowdm_engine::torrent::available_port().unwrap()).into(),
                ipv4_only: true,
                mode: if utp { ListenerMode::TcpAndUtp } else { ListenerMode::TcpOnly },
                ..Default::default()
            }),
            connect: Some(ConnectionOptions { enable_tcp: !utp, ..Default::default() }),
            ..Default::default()
        },
    )
    .await
    .unwrap()
}
