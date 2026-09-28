//! Protocol between the browser extensions, the native messaging host and
//! the CrowDM desktop app.
//!
//! ```text
//! extension ⇄ (stdio, 4-byte length + JSON) ⇄ crowdm-host.exe ⇄ (named pipe, JSON lines) ⇄ CrowDM.exe
//! ```
//!
//! The host is a relay: it forwards each JSON message unchanged in both
//! directions, starting the app when it is not running. All message types
//! are defined here and documented in `docs/BROWSER_INTEGRATION.md`.

use serde::{Deserialize, Serialize};

/// Native messaging host name registered with the browsers.
pub const HOST_NAME: &str = "com.crowdm.host";

/// Chromium extension id, fixed by the public key in the extension manifest.
pub const CHROMIUM_EXTENSION_ID: &str = "pomhhnfijfeecghaflldjibhaofbiabm";

/// Firefox add-on id (`browser_specific_settings.gecko.id`).
pub const FIREFOX_EXTENSION_ID: &str = "crowdm@syst4n.github.io";

/// Protocol version; bumped on incompatible changes.
pub const PROTOCOL_VERSION: u32 = 1;

/// Per-user pipe name (the pipe namespace is machine-wide).
pub fn pipe_name() -> String {
    let user: String = std::env::var("USERNAME")
        .unwrap_or_else(|_| "user".into())
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!(r"\\.\pipe\CrowDM-{user}")
}

/// A cookie or header forwarded by the extension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Header {
    pub name: String,
    pub value: String,
}

/// Why the extension is handing a download to the app.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CaptureKind {
    /// A browser download that was intercepted (and is paused in the browser).
    Intercepted,
    /// "Download with CrowDM" from the context menu.
    Menu,
    /// "Download this video" button.
    Video,
    /// A URL entered in the app itself.
    Manual,
}

/// Details of a download offered to the app.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Capture {
    pub kind: CaptureKind,
    /// Browser download id for intercepted downloads.
    pub browser_download_id: Option<i64>,
    pub url: String,
    /// URL after redirects as seen by the browser.
    pub final_url: Option<String>,
    pub referrer: Option<String>,
    pub page_url: Option<String>,
    pub page_title: Option<String>,
    /// File name suggested by the browser (base name only).
    pub file_name: Option<String>,
    pub mime: Option<String>,
    /// Total size reported by the browser, if known.
    pub size: Option<u64>,
    #[serde(default)]
    pub incognito: bool,
    /// Request headers to replay (Cookie, User-Agent, Authorization...).
    #[serde(default)]
    pub headers: Vec<Header>,
}

/// Messages from the extension (relayed by the host) to the app.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum FromBrowser {
    Hello {
        id: String,
        browser: String,
        extension_version: String,
        #[serde(default)]
        protocol: u32,
    },
    Capture {
        id: String,
        browser: String,
        capture: Capture,
    },
    /// The browser download we were deciding about went away (the user
    /// cancelled it in the browser, or it finished before we answered).
    CaptureGone { id: String },
    Ping { id: String },
    /// Sent by a second copy of the app to the running one (single instance).
    Activate { args: Vec<String> },
}

/// What the extension should do with the paused browser download.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    /// CrowDM took the download: cancel and erase it in the browser.
    TakeOver,
    /// Let the browser continue the download.
    ResumeInBrowser,
    /// The user dismissed the download: cancel it in the browser too.
    Cancel,
}

/// Messages from the app to the extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ToBrowser {
    Hello {
        reply_to: String,
        app_version: String,
        protocol: u32,
        settings: BrowserSettings,
    },
    /// The app received the capture and is showing it to the user.
    Received { reply_to: String },
    Decision { reply_to: String, decision: Decision },
    Settings { settings: BrowserSettings },
    Pong { reply_to: String },
    Error { reply_to: Option<String>, code: String, message: String },
}

/// Interception settings the extension enforces.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSettings {
    pub enabled: bool,
    pub excluded_sites: Vec<String>,
    pub excluded_extensions: Vec<String>,
    pub min_size: u64,
    pub include_private: bool,
    pub video_button: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_roundtrip_uses_camel_case() {
        let json = r#"{"type":"capture","id":"1","browser":"chrome","capture":{"kind":"intercepted","browserDownloadId":5,
            "url":"https://x/a.zip","finalUrl":null,"referrer":null,"pageUrl":null,"pageTitle":null,"fileName":"a.zip",
            "mime":"application/zip","size":10,"incognito":false,"headers":[{"name":"Cookie","value":"a=b"}]}}"#;
        let m: FromBrowser = serde_json::from_str(json).unwrap();
        match m {
            FromBrowser::Capture { capture, .. } => {
                assert_eq!(capture.browser_download_id, Some(5));
                assert_eq!(capture.headers[0].name, "Cookie");
            }
            _ => panic!(),
        }
        let out = serde_json::to_string(&ToBrowser::Decision { reply_to: "1".into(), decision: Decision::TakeOver }).unwrap();
        assert_eq!(out, r#"{"type":"decision","replyTo":"1","decision":"takeOver"}"#);
    }

    #[test]
    fn pipe_name_is_per_user() {
        assert!(pipe_name().starts_with(r"\\.\pipe\CrowDM-"));
    }
}
