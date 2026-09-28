//! File-name handling: Content-Disposition parsing, URL-derived names,
//! Windows-safe sanitizing and collision-free naming.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;

/// Extracts the file name from a `Content-Disposition` header value.
/// Prefers the RFC 5987 `filename*` parameter over `filename`.
pub fn from_content_disposition(value: &str) -> Option<String> {
    let mut plain: Option<String> = None;
    let mut extended: Option<String> = None;
    for part in split_params(value) {
        let Some((k, v)) = part.split_once('=') else { continue };
        let key = k.trim().to_ascii_lowercase();
        let v = v.trim();
        if key == "filename*" {
            // charset'lang'percent-encoded
            let raw = unquote(v);
            let mut pieces = raw.splitn(3, '\'');
            let charset = pieces.next().unwrap_or("").to_ascii_lowercase();
            let _lang = pieces.next();
            if let Some(enc) = pieces.next() {
                let bytes: Vec<u8> = percent_decode_str(enc).collect();
                let s = if charset == "utf-8" || charset.is_empty() {
                    String::from_utf8_lossy(&bytes).into_owned()
                } else {
                    // ISO-8859-1 and friends: map bytes to code points.
                    bytes.iter().map(|&b| b as char).collect()
                };
                if !s.trim().is_empty() {
                    extended = Some(s);
                }
            }
        } else if key == "filename" {
            let s = unquote(v);
            // Some servers percent-encode plain filename values.
            let decoded = if s.contains('%') {
                percent_decode_str(&s).decode_utf8().map(|c| c.into_owned()).unwrap_or(s)
            } else {
                s
            };
            if !decoded.trim().is_empty() {
                plain = Some(decoded);
            }
        }
    }
    extended.or(plain)
}

fn split_params(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            cur.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quotes => {
                cur.push(ch);
                escaped = true;
            }
            '"' => {
                in_quotes = !in_quotes;
                cur.push(ch);
            }
            ';' if !in_quotes => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        let inner = &v[1..v.len() - 1];
        let mut s = String::with_capacity(inner.len());
        let mut esc = false;
        for c in inner.chars() {
            if esc {
                s.push(c);
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else {
                s.push(c);
            }
        }
        s
    } else {
        v.to_string()
    }
}

/// Last non-empty path segment of a URL, percent-decoded.
pub fn from_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let seg = parsed.path_segments()?.rev().find(|s| !s.is_empty())?;
    let decoded = percent_decode_str(seg).decode_utf8_lossy().into_owned();
    let decoded = decoded.trim().to_string();
    if decoded.is_empty() {
        None
    } else {
        Some(decoded)
    }
}

const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Makes a name safe to use as a single Windows file-name component.
pub fn sanitize(name: &str) -> String {
    // Strip any directory parts a server might have sent.
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let mut s: String = base
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 32 => '_',
            c => c,
        })
        .collect();
    // Windows silently drops trailing dots and spaces.
    while s.ends_with('.') || s.ends_with(' ') {
        s.pop();
    }
    let s2 = s.trim_start().to_string();
    s = s2;
    if s.is_empty() || s == "." || s == ".." {
        s = "download".into();
    }
    let stem_upper = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    if RESERVED.contains(&stem_upper.as_str()) {
        s = format!("_{s}");
    }
    // Keep room for " (999)" and ".crowdm-part" inside the 255 UTF-16 unit limit.
    truncate_utf16(&s, 220)
}

fn truncate_utf16(s: &str, max_units: usize) -> String {
    if s.encode_utf16().count() <= max_units {
        return s.to_string();
    }
    let (stem, ext) = split_ext(s);
    let ext_units = ext.encode_utf16().count();
    let budget = max_units.saturating_sub(ext_units).max(1);
    let mut out = String::new();
    let mut used = 0;
    for ch in stem.chars() {
        let n = ch.len_utf16();
        if used + n > budget {
            break;
        }
        out.push(ch);
        used += n;
    }
    out.push_str(ext);
    out
}

/// Splits "name.ext" into ("name", ".ext"). Names without an extension, and
/// dot-files, return an empty extension.
pub fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && i < name.len() - 1 && name.len() - i <= 16 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// Adds an extension guessed from the MIME type when the name has none.
pub fn with_mime_extension(name: String, mime: Option<&str>) -> String {
    if !split_ext(&name).1.is_empty() {
        return name;
    }
    let Some(mime) = mime else { return name };
    let essence = mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    if essence.is_empty() || essence == "application/octet-stream" || essence == "binary/octet-stream" {
        return name;
    }
    let preferred = match essence.as_str() {
        "video/mp4" => Some("mp4"),
        "audio/mpeg" => Some("mp3"),
        "image/jpeg" => Some("jpg"),
        "text/plain" => Some("txt"),
        "text/html" => Some("html"),
        "application/zip" | "application/x-zip-compressed" => Some("zip"),
        "application/pdf" => Some("pdf"),
        "application/x-msdownload" => Some("exe"),
        "application/x-bittorrent" => Some("torrent"),
        _ => None,
    };
    let ext = preferred
        .map(str::to_string)
        .or_else(|| mime_guess::get_mime_extensions_str(&essence).and_then(|e| e.first()).map(|e| e.to_string()));
    match ext {
        Some(e) => format!("{name}.{e}"),
        None => name,
    }
}

/// Chooses a name for a response: Content-Disposition, then the final URL,
/// then the original URL, then "download".
pub fn choose(content_disposition: Option<&str>, final_url: &str, original_url: &str, mime: Option<&str>) -> String {
    let raw = content_disposition
        .and_then(from_content_disposition)
        .or_else(|| from_url(final_url))
        .or_else(|| from_url(original_url))
        .unwrap_or_else(|| "download".into());
    sanitize(&with_mime_extension(sanitize(&raw), mime))
}

/// Returns `dir/name` if free, else `dir/name (1).ext`, `(2)`, ...
/// Names in `taken` (other jobs' targets) count as used, compared
/// case-insensitively like NTFS.
pub fn unique_path(dir: &Path, name: &str, taken: &HashSet<String>) -> PathBuf {
    let is_free = |n: &str| {
        let p = dir.join(n);
        !p.exists() && !Path::new(&format!("{}.crowdm-part", p.display())).exists() && !taken.contains(&p.to_string_lossy().to_lowercase())
    };
    if is_free(name) {
        return dir.join(name);
    }
    let (stem, ext) = split_ext(name);
    for i in 1..10_000 {
        let candidate = format!("{stem} ({i}){ext}");
        if is_free(&candidate) {
            return dir.join(candidate);
        }
    }
    dir.join(format!("{stem} ({}){ext}", uuid::Uuid::new_v4().simple()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_disposition_variants() {
        assert_eq!(from_content_disposition("attachment; filename=\"a b.zip\"").as_deref(), Some("a b.zip"));
        assert_eq!(from_content_disposition("attachment; filename=plain.txt").as_deref(), Some("plain.txt"));
        assert_eq!(
            from_content_disposition("attachment; filename=\"fallback.txt\"; filename*=UTF-8''%E2%82%AC%20rates.txt").as_deref(),
            Some("€ rates.txt")
        );
        assert_eq!(from_content_disposition("attachment; filename*=iso-8859-1'en'%A3%20rates").as_deref(), Some("£ rates"));
        assert_eq!(from_content_disposition("inline; filename=\"semi;colon.pdf\"").as_deref(), Some("semi;colon.pdf"));
        assert_eq!(from_content_disposition("attachment").as_deref(), None);
    }

    #[test]
    fn url_names() {
        assert_eq!(from_url("https://x.com/a/b/file%20name.iso?x=1").as_deref(), Some("file name.iso"));
        assert_eq!(from_url("https://x.com/dir/").as_deref(), Some("dir"));
        assert_eq!(from_url("https://x.com/").as_deref(), None);
    }

    #[test]
    fn sanitizing() {
        assert_eq!(sanitize("a<b>c:d?.txt"), "a_b_c_d_.txt");
        assert_eq!(sanitize("..\\..\\evil.exe"), "evil.exe");
        assert_eq!(sanitize("CON.txt"), "_CON.txt");
        assert_eq!(sanitize("name.  "), "name");
        assert_eq!(sanitize(""), "download");
        assert!(sanitize(&"x".repeat(400)).len() <= 220);
        let long = format!("{}.mp4", "y".repeat(400));
        assert!(sanitize(&long).ends_with(".mp4"));
    }

    #[test]
    fn mime_extension() {
        assert_eq!(with_mime_extension("video".into(), Some("video/mp4")), "video.mp4");
        assert_eq!(with_mime_extension("file.bin".into(), Some("video/mp4")), "file.bin");
        assert_eq!(with_mime_extension("blob".into(), Some("application/octet-stream")), "blob");
    }

    #[test]
    fn unique_names() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"x").unwrap();
        let mut taken = HashSet::new();
        assert_eq!(unique_path(dir.path(), "a.txt", &taken), dir.path().join("a (1).txt"));
        taken.insert(dir.path().join("a (1).txt").to_string_lossy().to_lowercase());
        assert_eq!(unique_path(dir.path(), "a.txt", &taken), dir.path().join("a (2).txt"));
        assert_eq!(unique_path(dir.path(), "b.txt", &taken), dir.path().join("b.txt"));
    }
}
