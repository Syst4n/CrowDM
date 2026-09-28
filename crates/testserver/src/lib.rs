//! A small, controllable HTTP/1.1 server for CrowDM's transfer tests and
//! benchmarks.
//!
//! Every resource is generated on the fly from a deterministic function of
//! `(seed, offset)`, so the server can serve "files" of any size (including
//! 100 GB) without storing them, and tests can verify downloaded bytes with
//! [`content::verify_file`].
//!
//! Behaviour is controlled with query parameters on `/file`:
//!
//! | parameter      | effect                                                        |
//! |----------------|---------------------------------------------------------------|
//! | `size=N`       | resource length in bytes (required)                           |
//! | `seed=N`       | content seed (default 1)                                      |
//! | `id=KEY`       | per-resource state key (versions, counters, drop budgets)     |
//! | `norange`      | ignore `Range` and do not advertise `Accept-Ranges`           |
//! | `ignorerange`  | advertise `Accept-Ranges: bytes` but still answer `200`       |
//! | `etag=E`       | send `ETag: "E-v<version>"`                                   |
//! | `lm`           | send a `Last-Modified` derived from the version               |
//! | `chunked`      | omit `Content-Length` (chunked transfer, unknown size)        |
//! | `dropafter=N`  | abort the connection after N body bytes ...                   |
//! | `dropfirst=K`  | ... but only for the first K responses of this `id`           |
//! | `rate=BPS`     | throttle each response to BPS bytes per second                |
//! | `name=X`       | send `Content-Disposition: attachment; filename="X"`           |
//! | `mime=M`       | `Content-Type` (default `application/octet-stream`)           |
//! | `maxconn=C`    | answer `503` when more than C bodies of this `id` are active  |
//! | `lengthlie=N`  | advertise a `Content-Length`/total of N while serving `size`  |
//! | `stallfirst=K` | the first K mid-file ranged requests hang for 60 s            |
//!
//! Other routes: `/redirect?n=K&to=<path>` (302 chain), `/status?code=C`,
//! `/bump?id=KEY` (changes the resource version, i.e. "the file changed on the
//! server"), `/stats?id=KEY` and `/page` (an HTML page with links and a video).

pub mod content;

use std::collections::HashMap;
use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use http_body::{Body, Frame};
use hyper::body::Incoming;
use hyper::header::{self, HeaderValue};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use parking_lot::Mutex;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// Per-`id` mutable state.
#[derive(Default)]
struct ResourceState {
    version: u64,
    requests: u64,
    ranged_requests: u64,
    drops_used: u64,
    stalls_used: u64,
    active: u64,
    max_active: u64,
}

#[derive(Default)]
struct Shared {
    resources: Mutex<HashMap<String, ResourceState>>,
    total_requests: AtomicU64,
}

/// A running test server. Dropping it does not stop the server task; the
/// task ends with the Tokio runtime.
#[derive(Clone)]
pub struct TestServer {
    addr: SocketAddr,
    shared: Arc<Shared>,
}

/// Snapshot of the counters for one resource id.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stats {
    pub requests: u64,
    pub ranged_requests: u64,
    pub max_active: u64,
    pub version: u64,
}

impl TestServer {
    /// Starts a server on `127.0.0.1` with an ephemeral port.
    pub async fn start() -> io::Result<Self> {
        Self::bind("127.0.0.1:0".parse().unwrap()).await
    }

    pub async fn bind(addr: SocketAddr) -> io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared::default());
        let server = TestServer { addr, shared: shared.clone() };
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else { continue };
                let _ = stream.set_nodelay(true);
                let shared = shared.clone();
                tokio::spawn(async move {
                    let svc = service_fn(move |req| handle(req, shared.clone(), addr));
                    let _ = hyper::server::conn::http1::Builder::new().keep_alive(true).serve_connection(TokioIo::new(stream), svc).await;
                });
            }
        });
        Ok(server)
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// `http://127.0.0.1:PORT` followed by `path_and_query`.
    pub fn url(&self, path_and_query: &str) -> String {
        format!("http://{}{}", self.addr, path_and_query)
    }

    /// Simulates the file changing on the server.
    pub fn bump(&self, id: &str) {
        self.shared.resources.lock().entry(id.to_string()).or_default().version += 1;
    }

    pub fn stats(&self, id: &str) -> Stats {
        let map = self.shared.resources.lock();
        map.get(id)
            .map(|s| Stats { requests: s.requests, ranged_requests: s.ranged_requests, max_active: s.max_active, version: s.version })
            .unwrap_or_default()
    }

    pub fn total_requests(&self) -> u64 {
        self.shared.total_requests.load(Ordering::Relaxed)
    }
}

type Resp = Response<ChannelBody>;

fn query_map(req: &Request<Incoming>) -> HashMap<String, String> {
    let q = req.uri().query().unwrap_or("");
    url::form_urlencoded::parse(q.as_bytes()).into_owned().collect()
}

fn simple(status: StatusCode, text: &str) -> Resp {
    let mut r = Response::new(ChannelBody::full(Bytes::from(text.to_string())));
    *r.status_mut() = status;
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    r.headers_mut().insert(header::CONTENT_LENGTH, HeaderValue::from(text.len()));
    r
}

async fn handle(req: Request<Incoming>, shared: Arc<Shared>, addr: SocketAddr) -> Result<Resp, Infallible> {
    shared.total_requests.fetch_add(1, Ordering::Relaxed);
    let q = query_map(&req);
    let path = req.uri().path().to_string();
    let resp = match path.as_str() {
        "/file" => {
            // `stallfirst=K`: the first K ranged requests that do not start at
            // byte 0 get no answer for a minute (a frozen extra connection).
            if let Some(k) = q.get("stallfirst").and_then(|v| v.parse::<u64>().ok()) {
                let mid = req.headers().get(header::RANGE).and_then(|v| v.to_str().ok()).is_some_and(|r| !r.starts_with("bytes=0-"));
                let stall = mid && {
                    let id = q.get("id").cloned().unwrap_or_default();
                    let mut map = shared.resources.lock();
                    let st = map.entry(id).or_default();
                    st.stalls_used += 1;
                    st.stalls_used <= k
                };
                if stall {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                }
            }
            serve_file(&req, &q, &shared)
        }
        "/redirect" => {
            let n: u32 = q.get("n").and_then(|v| v.parse().ok()).unwrap_or(1);
            let to = q.get("to").cloned().unwrap_or_else(|| "/file?size=1024".into());
            let location = if n <= 1 {
                to
            } else {
                let enc: String = url::form_urlencoded::byte_serialize(to.as_bytes()).collect();
                format!("/redirect?n={}&to={}", n - 1, enc)
            };
            let mut r = simple(StatusCode::FOUND, "redirect");
            r.headers_mut().insert(header::LOCATION, HeaderValue::from_str(&location).unwrap());
            r
        }
        "/status" => {
            let code: u16 = q.get("code").and_then(|v| v.parse().ok()).unwrap_or(500);
            let mut r = simple(StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), "status");
            if let Some(ra) = q.get("retryafter") {
                r.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_str(ra).unwrap());
            }
            r
        }
        "/bump" => {
            let id = q.get("id").cloned().unwrap_or_default();
            shared.resources.lock().entry(id).or_default().version += 1;
            simple(StatusCode::OK, "bumped")
        }
        "/stats" => {
            let id = q.get("id").cloned().unwrap_or_default();
            let map = shared.resources.lock();
            let s = map.get(&id);
            let body = format!(
                "{{\"requests\":{},\"ranged\":{},\"maxActive\":{}}}",
                s.map(|s| s.requests).unwrap_or(0),
                s.map(|s| s.ranged_requests).unwrap_or(0),
                s.map(|s| s.max_active).unwrap_or(0)
            );
            simple(StatusCode::OK, &body)
        }
        "/page" => {
            let html = test_page(addr);
            let mut r = Response::new(ChannelBody::full(Bytes::from(html.clone())));
            r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
            r.headers_mut().insert(header::CONTENT_LENGTH, HeaderValue::from(html.len()));
            r
        }
        _ => simple(StatusCode::NOT_FOUND, "not found"),
    };
    Ok(resp)
}

fn test_page(addr: SocketAddr) -> String {
    format!(
        r#"<!doctype html><html><head><title>CrowDM test page</title></head><body>
<h1>CrowDM test page</h1>
<p><a id="small" href="/file?size=1048576&amp;name=small.bin&amp;etag=s">small.bin (1 MiB)</a></p>
<p><a id="big" href="/file?size=268435456&amp;name=big.bin&amp;etag=b">big.bin (256 MiB)</a></p>
<p><a id="unknown" href="/file?size=5000000&amp;chunked&amp;name=unknown-size.dat">unknown size</a></p>
<p><a id="odd" href="/file?size=123457&amp;name=report.weirdext">odd extension</a></p>
<p><a id="redirect" href="/redirect?n=3&amp;to=%2Ffile%3Fsize%3D2097152%26name%3Dredirected.zip%26etag%3Dr">redirected.zip</a></p>
<video id="v" controls width="480" src="http://{addr}/file?size=3145728&amp;mime=video%2Fmp4&amp;name=clip.mp4&amp;etag=v"></video>
</body></html>"#
    )
}

fn parse_range(v: &str, size: u64) -> Option<(u64, u64)> {
    let v = v.trim().strip_prefix("bytes=")?;
    if v.contains(',') {
        return None;
    }
    let (a, b) = v.split_once('-')?;
    if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        if n == 0 || size == 0 {
            return None;
        }
        return Some((size.saturating_sub(n), size - 1));
    }
    let start: u64 = a.parse().ok()?;
    let end = if b.is_empty() { size.saturating_sub(1) } else { b.parse::<u64>().ok()?.min(size.saturating_sub(1)) };
    if start >= size || end < start {
        return Some((u64::MAX, u64::MAX)); // unsatisfiable marker
    }
    Some((start, end))
}

fn serve_file(req: &Request<Incoming>, q: &HashMap<String, String>, shared: &Arc<Shared>) -> Resp {
    let Some(size) = q.get("size").and_then(|v| v.parse::<u64>().ok()) else {
        return simple(StatusCode::BAD_REQUEST, "size required");
    };
    let id = q.get("id").cloned().unwrap_or_else(|| format!("anon-{size}"));
    let base_seed: u64 = q.get("seed").and_then(|v| v.parse().ok()).unwrap_or(1);
    let has = |k: &str| q.contains_key(k);
    let range_hdr = req.headers().get(header::RANGE).and_then(|v| v.to_str().ok()).map(str::to_string);

    let (version, drop_this, over_limit) = {
        let mut map = shared.resources.lock();
        let st = map.entry(id.clone()).or_default();
        st.requests += 1;
        if range_hdr.is_some() {
            st.ranged_requests += 1;
        }
        let drop_budget: u64 = q.get("dropfirst").and_then(|v| v.parse().ok()).unwrap_or(u64::MAX);
        let drop_this = q.contains_key("dropafter") && st.drops_used < drop_budget;
        if drop_this {
            st.drops_used += 1;
        }
        let max_conn: u64 = q.get("maxconn").and_then(|v| v.parse().ok()).unwrap_or(u64::MAX);
        let over = req.method() == Method::GET && st.active >= max_conn;
        (st.version, drop_this, over)
    };
    if over_limit {
        return simple(StatusCode::SERVICE_UNAVAILABLE, "too many connections");
    }

    let seed = base_seed.wrapping_add(version.wrapping_mul(0x9E37_79B9));
    let etag = q.get("etag").map(|e| format!("\"{e}-v{version}\""));
    let last_modified = has("lm").then(|| httpdate_for(version));
    let advertised = q.get("lengthlie").and_then(|v| v.parse().ok()).unwrap_or(size);

    // If-Range: only honour the range when the validator still matches.
    let if_range_ok = match req.headers().get(header::IF_RANGE).and_then(|v| v.to_str().ok()) {
        None => true,
        Some(v) => Some(v) == etag.as_deref() || Some(v) == last_modified.as_deref(),
    };

    let ranges_enabled = !has("norange") && !has("chunked");
    let mut status = StatusCode::OK;
    let mut start = 0u64;
    let mut end = size.saturating_sub(1);
    if let (true, false, true, Some(r)) = (ranges_enabled, has("ignorerange"), if_range_ok, range_hdr.as_deref()) {
        match parse_range(r, size) {
            Some((u64::MAX, _)) => {
                let mut resp = simple(StatusCode::RANGE_NOT_SATISFIABLE, "");
                resp.headers_mut().insert(header::CONTENT_RANGE, HeaderValue::from_str(&format!("bytes */{size}")).unwrap());
                return resp;
            }
            Some((a, b)) => {
                status = StatusCode::PARTIAL_CONTENT;
                start = a;
                end = b;
            }
            None => {}
        }
    }
    let len = if size == 0 { 0 } else { end - start + 1 };

    let mut builder = Response::builder().status(status);
    let h = builder.headers_mut().unwrap();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_str(q.get("mime").map(String::as_str).unwrap_or("application/octet-stream")).unwrap());
    if ranges_enabled {
        h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    }
    if let Some(e) = &etag {
        h.insert(header::ETAG, HeaderValue::from_str(e).unwrap());
    }
    if let Some(lm) = &last_modified {
        h.insert(header::LAST_MODIFIED, HeaderValue::from_str(lm).unwrap());
    }
    if let Some(name) = q.get("name") {
        h.insert(header::CONTENT_DISPOSITION, HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")).unwrap());
    }
    if !has("chunked") {
        let adv_len = if status == StatusCode::PARTIAL_CONTENT { len } else { advertised };
        h.insert(header::CONTENT_LENGTH, HeaderValue::from(adv_len));
    }
    if status == StatusCode::PARTIAL_CONTENT {
        h.insert(header::CONTENT_RANGE, HeaderValue::from_str(&format!("bytes {start}-{end}/{advertised}")).unwrap());
    }
    if req.method() == Method::HEAD {
        return builder.body(ChannelBody::empty()).unwrap();
    }

    let drop_after: Option<u64> = if drop_this { q.get("dropafter").and_then(|v| v.parse().ok()) } else { None };
    let rate: Option<u64> = q.get("rate").and_then(|v| v.parse().ok());
    let (tx, rx) = mpsc::channel::<Result<Frame<Bytes>, io::Error>>(4);
    let guard = ActiveGuard::new(shared.clone(), id);
    tokio::spawn(async move {
        let _guard = guard;
        let mut pos = start;
        let stop = start + len;
        let mut sent = 0u64;
        let chunk = 64 * 1024u64;
        let started = tokio::time::Instant::now();
        while pos < stop {
            let mut n = chunk.min(stop - pos);
            if let Some(limit) = drop_after {
                if sent >= limit {
                    let _ = tx.send(Err(io::Error::new(io::ErrorKind::ConnectionAborted, "dropped"))).await;
                    return;
                }
                n = n.min(limit - sent);
            }
            let mut buf = vec![0u8; n as usize];
            content::fill(seed, pos, &mut buf);
            if tx.send(Ok(Frame::data(Bytes::from(buf)))).await.is_err() {
                return;
            }
            pos += n;
            sent += n;
            if let Some(bps) = rate {
                let due = Duration::from_secs_f64(sent as f64 / bps.max(1) as f64);
                tokio::time::sleep_until(started + due).await;
            }
        }
        if drop_after.is_some_and(|l| sent >= l) && pos < stop {
            let _ = tx.send(Err(io::Error::new(io::ErrorKind::ConnectionAborted, "dropped"))).await;
        }
    });
    builder.body(ChannelBody { rx: Some(rx), full: None }).unwrap()
}

fn httpdate_for(version: u64) -> String {
    // Fixed dates are enough: only equality matters to the client.
    let day = 1 + (version % 28);
    format!("Mon, {day:02} Jan 2024 12:00:00 GMT")
}

struct ActiveGuard {
    shared: Arc<Shared>,
    id: String,
}

impl ActiveGuard {
    fn new(shared: Arc<Shared>, id: String) -> Self {
        {
            let mut map = shared.resources.lock();
            let st = map.entry(id.clone()).or_default();
            st.active += 1;
            st.max_active = st.max_active.max(st.active);
        }
        ActiveGuard { shared, id }
    }
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        if let Some(st) = self.shared.resources.lock().get_mut(&self.id) {
            st.active = st.active.saturating_sub(1);
        }
    }
}

/// Response body fed by a channel so the producer task can throttle or abort.
pub struct ChannelBody {
    rx: Option<mpsc::Receiver<Result<Frame<Bytes>, io::Error>>>,
    full: Option<Bytes>,
}

impl ChannelBody {
    fn full(b: Bytes) -> Self {
        ChannelBody { rx: None, full: Some(b) }
    }
    fn empty() -> Self {
        ChannelBody { rx: None, full: None }
    }
}

impl Body for ChannelBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        if let Some(b) = self.full.take() {
            return Poll::Ready(Some(Ok(Frame::data(b))));
        }
        match self.rx.as_mut() {
            Some(rx) => rx.poll_recv(cx),
            None => Poll::Ready(None),
        }
    }
}
