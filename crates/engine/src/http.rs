//! HTTP helpers: client construction, request building and response
//! classification.

use std::time::Duration;

use reqwest::header::{self, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Response, StatusCode};

use crate::error::TransferError;
use crate::model::ErrorKind;

pub const DEFAULT_USER_AGENT: &str = concat!("CrowDM/", env!("CARGO_PKG_VERSION"), " (Windows; +https://github.com/Syst4n/CrowDM)");

pub fn build_client(user_agent: Option<&str>) -> reqwest::Result<Client> {
    Client::builder()
        .user_agent(user_agent.unwrap_or(DEFAULT_USER_AGENT))
        // One TCP connection per segment is the point of segmenting; HTTP/2
        // would multiplex all segments over one connection.
        .http1_only()
        .connect_timeout(Duration::from_secs(20))
        // Idle-read timeout: a stalled connection is retried instead of
        // hanging forever. Not a total-transfer timeout.
        .read_timeout(Duration::from_secs(45))
        .pool_idle_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(32)
        .tcp_nodelay(true)
        .redirect(reqwest::redirect::Policy::limited(20))
        .build()
}

/// Headers a caller may not override because the engine manages them.
fn is_managed_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "range" | "if-range" | "accept-encoding" | "host" | "content-length" | "connection" | "transfer-encoding" | "te" | "upgrade"
    )
}

/// Builds the extra header map from a job's stored header pairs.
pub fn header_map(pairs: &[(String, String)], referrer: Option<&str>) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (k, v) in pairs {
        if is_managed_header(k) {
            continue;
        }
        if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(k.trim().as_bytes()), HeaderValue::from_str(v.trim())) {
            map.append(name, value);
        }
    }
    if let Some(r) = referrer {
        if !map.contains_key(header::REFERER) {
            if let Ok(v) = HeaderValue::from_str(r) {
                map.insert(header::REFERER, v);
            }
        }
    }
    // Ask for the raw bytes: a transparently-compressed body would not match
    // Content-Length or byte ranges.
    map.insert(header::ACCEPT_ENCODING, HeaderValue::from_static("identity"));
    map
}

/// Validator for `If-Range`: only a strong ETag or a Last-Modified date.
pub fn if_range_value(etag: Option<&str>, last_modified: Option<&str>) -> Option<String> {
    match etag {
        Some(e) if !e.starts_with("W/") => Some(e.to_string()),
        _ => last_modified.map(str::to_string),
    }
}

/// Parsed `Content-Range: bytes a-b/total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentRange {
    pub start: u64,
    pub end: u64,
    pub total: Option<u64>,
}

pub fn parse_content_range(v: &str) -> Option<ContentRange> {
    let v = v.trim().strip_prefix("bytes")?.trim_start();
    let (range, total) = v.split_once('/')?;
    let (a, b) = range.trim().split_once('-')?;
    let total = match total.trim() {
        "*" => None,
        t => Some(t.parse().ok()?),
    };
    let start: u64 = a.trim().parse().ok()?;
    let end: u64 = b.trim().parse().ok()?;
    if end < start {
        return None;
    }
    Some(ContentRange { start, end, total })
}

pub fn header_str<'a>(resp: &'a Response, name: header::HeaderName) -> Option<&'a str> {
    resp.headers().get(name).and_then(|v| v.to_str().ok())
}

/// Identity and size information from a response.
#[derive(Debug, Clone, Default)]
pub struct ResponseMeta {
    pub status: u16,
    pub final_url: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub mime: Option<String>,
    pub content_disposition: Option<String>,
    pub content_length: Option<u64>,
    pub content_range: Option<ContentRange>,
    pub accept_ranges: bool,
    pub content_encoding: Option<String>,
}

impl ResponseMeta {
    pub fn from_response(resp: &Response) -> Self {
        let content_range = header_str(resp, header::CONTENT_RANGE).and_then(parse_content_range);
        ResponseMeta {
            status: resp.status().as_u16(),
            final_url: resp.url().to_string(),
            etag: header_str(resp, header::ETAG).map(str::to_string),
            last_modified: header_str(resp, header::LAST_MODIFIED).map(str::to_string),
            mime: header_str(resp, header::CONTENT_TYPE).map(str::to_string),
            content_disposition: header_str(resp, header::CONTENT_DISPOSITION).map(str::to_string),
            content_length: header_str(resp, header::CONTENT_LENGTH).and_then(|v| v.trim().parse().ok()),
            content_range,
            accept_ranges: header_str(resp, header::ACCEPT_RANGES).is_some_and(|v| v.to_ascii_lowercase().contains("bytes")),
            content_encoding: header_str(resp, header::CONTENT_ENCODING)
                .map(|v| v.trim().to_ascii_lowercase())
                .filter(|v| !v.is_empty() && v != "identity"),
        }
    }

    /// Total entity size implied by the response.
    pub fn total_size(&self) -> Option<u64> {
        if self.status == 206 {
            self.content_range.and_then(|r| r.total)
        } else if self.content_encoding.is_none() {
            self.content_length
        } else {
            None
        }
    }

    pub fn looks_like_html(&self) -> bool {
        self.mime.as_deref().is_some_and(|m| {
            let m = m.to_ascii_lowercase();
            m.starts_with("text/html") || m.starts_with("application/xhtml")
        })
    }
}

/// Maps a non-success status to an error, honouring `Retry-After`.
pub fn status_error(resp: &Response) -> TransferError {
    let status = resp.status();
    let retry_after = header_str(resp, header::RETRY_AFTER).and_then(parse_retry_after);
    classify_status(status, retry_after)
}

pub fn classify_status(status: StatusCode, retry_after: Option<Duration>) -> TransferError {
    let code = status.as_u16();
    let (kind, msg) = match code {
        401 | 407 => (
            ErrorKind::AccessDenied,
            format!(
                "The server requires authentication (HTTP {code}). Start the download from your browser so CrowDM receives its cookies."
            ),
        ),
        403 => (
            ErrorKind::AccessDenied,
            "The server refused access (HTTP 403). The link may have expired; start it again from the page.".to_string(),
        ),
        404 => (ErrorKind::NotFound, "The file was not found on the server (HTTP 404).".to_string()),
        410 => (ErrorKind::NotFound, "The file is no longer available (HTTP 410).".to_string()),
        408 | 425 | 429 | 500 | 502 | 503 | 504 => (ErrorKind::ServerBusy, format!("The server is busy or unavailable (HTTP {code}).")),
        c if c >= 500 => (ErrorKind::ServerBusy, format!("Server error (HTTP {code}).")),
        _ => (ErrorKind::Http, format!("Unexpected server response (HTTP {code}).")),
    };
    TransferError { kind, message: msg, retry_after, status: Some(code) }
}

fn parse_retry_after(v: &str) -> Option<Duration> {
    if let Ok(secs) = v.trim().parse::<u64>() {
        return Some(Duration::from_secs(secs.min(300)));
    }
    let when = httpdate::parse_http_date(v.trim()).ok()?;
    let d = when.duration_since(std::time::SystemTime::now()).unwrap_or_default();
    Some(d.min(Duration::from_secs(300)))
}

/// Converts a reqwest failure into a transfer error.
pub fn request_error(e: reqwest::Error) -> TransferError {
    let message = if e.is_timeout() {
        "The connection timed out.".to_string()
    } else if e.is_connect() {
        format!("Could not connect to the server: {}", root_cause(&e))
    } else if e.is_redirect() {
        "Too many redirects.".to_string()
    } else if e.is_builder() {
        return TransferError::new(ErrorKind::InvalidUrl, format!("Invalid request: {}", root_cause(&e)));
    } else {
        format!("Network error: {}", root_cause(&e))
    };
    TransferError::new(ErrorKind::Network, message)
}

fn root_cause(e: &dyn std::error::Error) -> String {
    let mut cur: &dyn std::error::Error = e;
    while let Some(next) = cur.source() {
        cur = next;
    }
    cur.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range_parsing() {
        assert_eq!(parse_content_range("bytes 0-99/1000"), Some(ContentRange { start: 0, end: 99, total: Some(1000) }));
        assert_eq!(parse_content_range("bytes 5-9/*"), Some(ContentRange { start: 5, end: 9, total: None }));
        assert_eq!(parse_content_range("bytes */1000"), None);
        assert_eq!(parse_content_range("bytes 9-5/10"), None);
    }

    #[test]
    fn if_range_prefers_strong_etag() {
        assert_eq!(if_range_value(Some("\"a\""), Some("date")).as_deref(), Some("\"a\""));
        assert_eq!(if_range_value(Some("W/\"a\""), Some("date")).as_deref(), Some("date"));
        assert_eq!(if_range_value(None, None), None);
    }

    #[test]
    fn managed_headers_are_dropped() {
        let m = header_map(&[("Range".into(), "bytes=0-".into()), ("Cookie".into(), "a=b".into())], Some("https://x/"));
        assert!(m.get("range").is_none());
        assert_eq!(m.get("cookie").unwrap(), "a=b");
        assert_eq!(m.get("referer").unwrap(), "https://x/");
        assert_eq!(m.get("accept-encoding").unwrap(), "identity");
    }
}
