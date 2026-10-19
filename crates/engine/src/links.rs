//! Bounded, non-executing link extraction and multipart package naming.
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;
use url::Url;

static URLS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)(?:https?://|magnet:\?)[^\s<>"']+"#).unwrap());
static PART: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(.*?)(?:[._ -]part\d+(?:[._ -]of[._ -]?\d+)?\.(?:rar|7z|zip)|\.(?:7z|zip)\.\d{3}|\.r\d{2})$").unwrap()
});
static ATTR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)(?:href|src)\s*=\s*["']([^"']+)["']|window\s*\.\s*open\s*\(\s*["']([^"']+)["']"#).unwrap());

pub fn decode_html(s: &str) -> String {
    s.replace("&amp;", "&").replace("&#38;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">")
}

pub fn canonical(input: &str) -> Option<String> {
    let mut u = Url::parse(input).ok()?;
    if !matches!(u.scheme(), "http" | "https" | "magnet") || !u.username().is_empty() || u.password().is_some() {
        return None;
    }
    if u.scheme() != "magnet" {
        u.set_fragment(None);
    }
    Some(u.to_string())
}

pub fn extract(text: &str, limit: usize) -> Vec<String> {
    let text = decode_html(text);
    let mut seen = HashSet::new();
    URLS.find_iter(&text)
        .filter_map(|m| {
            // Preserve URL-significant punctuation. Only unwrap matching copied delimiters.
            let s = m.as_str().trim_end_matches([',', ';']);
            let s = if s.ends_with(')') && s.matches(')').count() > s.matches('(').count() { &s[..s.len() - 1] } else { s };
            let key = canonical(s)?;
            seen.insert(key).then(|| s.to_string())
        })
        .take(limit)
        .collect()
}

pub fn hint(url: &str) -> Option<String> {
    let u = Url::parse(url).ok()?;
    let raw = u.fragment().filter(|f| f.contains('.')).unwrap_or_else(|| u.path().rsplit('/').next().unwrap_or(""));
    let raw = percent_encoding::percent_decode_str(raw).decode_utf8_lossy();
    (!raw.is_empty()).then(|| crate::names::sanitize(&raw))
}

pub fn package(name: &str) -> String {
    let stem = PART
        .captures(name)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())
        .unwrap_or_else(|| std::path::Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or(name));
    crate::names::sanitize(stem.trim_end_matches(['.', '_', '-', ' ']))
}

/// Explicit downloadable resources only. No scripts, challenge solving or recursive crawl.
pub fn page_links(html: &str, base: &str, limit: usize) -> Vec<String> {
    let Ok(base) = Url::parse(base) else { return vec![] };
    let mut seen = HashSet::new();
    ATTR.captures_iter(html)
        .filter_map(|c| {
            let raw = decode_html(c.get(1).or_else(|| c.get(2))?.as_str());
            let u = base.join(&raw).ok()?;
            canonical(u.as_str())?;
            let is_explicit = c.get(2).is_some();
            let host_link = matches!(u.host_str(), Some("fuckingfast.co" | "www.fuckingfast.co"));
            let ext = std::path::Path::new(u.path()).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            let file_link = matches!(
                ext.as_str(),
                "rar" | "zip" | "7z" | "001" | "bin" | "iso" | "exe" | "msi" | "pdf" | "torrent" | "mp4" | "mkv" | "mp3" | "flac"
            );
            ((is_explicit || file_link || host_link) && seen.insert(u.to_string())).then(|| u.to_string())
        })
        .take(limit)
        .collect()
}

/// Read at most 2 MiB, even when Content-Length is absent or deceptive.
pub async fn fetch_page(url: &str) -> Result<(String, String), String> {
    let client = crate::http::build_client(None).map_err(|_| "Could not create the link checker.")?;
    let mut r = client.get(url).timeout(std::time::Duration::from_secs(20)).send().await.map_err(|_| "The page could not be reached.")?;
    if !r.status().is_success() {
        return Err(match r.status().as_u16() {
            403 => "Host blocked the link checker (HTTP 403). Open in browser to check verification or access requirements. A verified direct file link may be needed.".into(),
            404 | 410 => "The host reports that this link is unavailable (HTTP 404/410).".into(),
            code => format!("Host returned HTTP {code}. Retry later or open in browser."),
        });
    }
    let content_type = r.headers().get(reqwest::header::CONTENT_TYPE).and_then(|h| h.to_str().ok()).unwrap_or("");
    if !content_type.contains("text/html") && !content_type.contains("application/xhtml") {
        return Err("The server rejected the file check. Try its direct download in your browser.".into());
    }
    let final_url = r.url().to_string();
    let mut bytes = vec![];
    while let Some(chunk) = r.chunk().await.map_err(|_| "Could not read the link page.")? {
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err("The link page exceeds the 2 MiB analysis limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((String::from_utf8_lossy(&bytes).into_owned(), final_url))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_links_deduplicate_without_dropping_signed_queries() {
        let x = extract("<a href=\"https://example.org/a?key=1&amp;x=2#part01.rar\">x</a> https://example.org/a?key=1&x=2#other.rar https://example.org/a?key=2 magnet:?xt=urn:btih:0123", 50);
        assert_eq!(x.len(), 3);
        assert!(x[0].contains("&x=2#part01"));
        assert!(canonical("https://user:secret@example.org/a").is_none());
        assert_eq!(extract("https://example.org/a https://example.org/b", 1).len(), 1);
    }
    #[test]
    fn related_archive_parts_share_a_safe_folder() {
        for n in ["Example.part01.rar", "Example.part002.rar", "Example.7z.001", "Example.r00"] {
            assert_eq!(package(n), "Example");
        }
        assert_eq!(hint("https://host.org/id#Example.part01.rar").unwrap(), "Example.part01.rar");
        assert!(!package("../../danger.part01.rar").contains('/'));
    }
    #[test]
    fn only_explicit_resources_are_collected() {
        let x = page_links(
            r#"<a href="/about">About</a><a href="/a.part01.rar">Part</a><script>window.open("https://cdn.example.org/file?id=1&amp;token=2")</script><a href="javascript:alert(1)">bad</a>"#,
            "https://example.org/",
            10,
        );
        assert_eq!(x, vec!["https://example.org/a.part01.rar", "https://cdn.example.org/file?id=1&token=2"]);
    }
}
