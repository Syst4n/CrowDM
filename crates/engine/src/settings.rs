//! User settings shared by the engine and the desktop UI.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A file category: files whose extension is listed are saved into `folder`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub id: String,
    pub name: String,
    /// Folder name relative to the download root, or an absolute path.
    pub folder: String,
    /// Lowercase extensions without the dot.
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum AfterQueue {
    #[default]
    Nothing,
    Notify,
    ExitApp,
    Sleep,
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub enabled: bool,
    /// Local time "HH:MM" at which the queue starts.
    pub start_time: String,
    /// Optional local time "HH:MM" at which the queue stops.
    pub stop_time: Option<String>,
    /// Monday..Sunday.
    pub days: [bool; 7],
}

impl Default for Schedule {
    fn default() -> Self {
        Schedule { enabled: false, start_time: "02:00".into(), stop_time: Some("07:00".into()), days: [true; 7] }
    }
}

/// Browser interception rules, pushed to the extensions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Interception {
    pub enabled: bool,
    /// Host names (suffix match) whose downloads the browser keeps.
    pub excluded_sites: Vec<String>,
    /// Extensions (without dot) the browser keeps.
    pub excluded_extensions: Vec<String>,
    /// Downloads smaller than this many bytes stay in the browser (0 = all).
    pub min_size: u64,
    /// Capture downloads from private/incognito windows.
    pub include_private: bool,
    /// Show the "Download this video" button over web video players.
    pub video_button: bool,
}

impl Default for Interception {
    fn default() -> Self {
        Interception {
            enabled: true,
            excluded_sites: vec![],
            excluded_extensions: vec![],
            min_size: 0,
            include_private: false,
            video_button: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Root folder; categories are sub-folders of it.
    pub download_root: PathBuf,
    /// Place files into category sub-folders.
    pub organize_by_category: bool,
    pub categories: Vec<Category>,
    /// Maximum connections for one download.
    pub max_connections_per_download: u8,
    /// Maximum connections across all downloads.
    pub max_total_connections: u16,
    /// Queue concurrency: queued downloads running at the same time.
    pub max_active_downloads: u8,
    /// Global speed limit in bytes/second; 0 = unlimited.
    pub speed_limit: u64,
    /// Automatic retries of a failed download (job level).
    pub retry_attempts: u32,
    /// Seconds before an automatic retry.
    pub retry_delay_secs: u32,
    /// Resume downloads that were running when the app closed.
    pub resume_on_startup: bool,
    /// Custom User-Agent for manual downloads; browser captures use the browser's.
    pub user_agent: Option<String>,
    pub after_queue: AfterQueue,
    pub schedule: Schedule,
    pub interception: Interception,
    pub theme: Theme,
    pub close_to_tray: bool,
    pub start_with_windows: bool,
    pub start_minimized: bool,
    pub notify_on_complete: bool,
    /// Show the confirmation dialog for captured downloads (off = start silently).
    pub confirm_captured: bool,
    /// Keep the capture dialog loaded at all times (instant display, ~150 MB).
    /// When off it is loaded on demand and kept for a while after each use.
    pub prewarm_dialog: bool,
    /// Play a sound on completion.
    pub sound_on_complete: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            download_root: default_download_root(),
            organize_by_category: true,
            categories: default_categories(),
            max_connections_per_download: 8,
            max_total_connections: 48,
            max_active_downloads: 3,
            speed_limit: 0,
            retry_attempts: 5,
            retry_delay_secs: 10,
            resume_on_startup: true,
            user_agent: None,
            after_queue: AfterQueue::Nothing,
            schedule: Schedule::default(),
            interception: Interception::default(),
            theme: Theme::System,
            close_to_tray: true,
            start_with_windows: false,
            start_minimized: false,
            notify_on_complete: true,
            confirm_captured: true,
            prewarm_dialog: false,
            sound_on_complete: false,
        }
    }
}

fn default_download_root() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join("Downloads").join("CrowDM")
}

fn cat(id: &str, name: &str, exts: &str) -> Category {
    Category {
        id: id.into(),
        name: name.into(),
        folder: name.into(),
        extensions: exts.split_whitespace().map(str::to_string).collect(),
    }
}

pub fn default_categories() -> Vec<Category> {
    vec![
        cat("videos", "Videos", "mp4 mkv webm avi mov wmv flv m4v mpg mpeg 3gp ts m2ts ogv"),
        cat("audio", "Audio", "mp3 m4a aac flac wav ogg opus wma aiff alac mid midi"),
        cat("images", "Images", "jpg jpeg png gif webp bmp tif tiff svg heic heif avif ico raw psd"),
        cat("documents", "Documents", "pdf doc docx xls xlsx ppt pptx odt ods odp rtf txt csv md epub mobi"),
        cat("archives", "Archives", "zip rar 7z tar gz tgz bz2 xz zst iso img cab"),
        cat("programs", "Programs", "exe msi msix msixbundle appx apk dmg pkg deb rpm bat ps1"),
        cat("torrents", "Torrents", "torrent"),
        Category { id: "other".into(), name: "Other".into(), folder: "Other".into(), extensions: vec![] },
    ]
}

impl Settings {
    /// Normalizes values that could break the engine.
    pub fn sanitized(mut self) -> Self {
        self.max_connections_per_download = self.max_connections_per_download.clamp(1, 32);
        self.max_total_connections = self.max_total_connections.clamp(1, 256);
        self.max_active_downloads = self.max_active_downloads.clamp(1, 32);
        self.retry_attempts = self.retry_attempts.min(100);
        if !self.categories.iter().any(|c| c.id == "other") {
            self.categories.push(Category { id: "other".into(), name: "Other".into(), folder: "Other".into(), extensions: vec![] });
        }
        for c in &mut self.categories {
            for e in &mut c.extensions {
                *e = e.trim().trim_start_matches('.').to_ascii_lowercase();
            }
            c.extensions.retain(|e| !e.is_empty());
        }
        self
    }

    /// Category id for a file name, falling back to "other".
    pub fn category_for(&self, file_name: &str) -> String {
        let ext = Path::new(file_name)
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if !ext.is_empty() {
            for c in &self.categories {
                if c.extensions.iter().any(|x| *x == ext) {
                    return c.id.clone();
                }
            }
        }
        "other".into()
    }

    /// Destination directory for a category.
    pub fn dir_for_category(&self, category: &str) -> PathBuf {
        if !self.organize_by_category {
            return self.download_root.clone();
        }
        match self.categories.iter().find(|c| c.id == category) {
            Some(c) => {
                let p = PathBuf::from(&c.folder);
                if p.is_absolute() {
                    p
                } else {
                    self.download_root.join(p)
                }
            }
            None => self.download_root.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categorizes_by_extension() {
        let s = Settings::default();
        assert_eq!(s.category_for("movie.MKV"), "videos");
        assert_eq!(s.category_for("a.tar.gz"), "archives");
        assert_eq!(s.category_for("thing.unknownext"), "other");
        assert_eq!(s.category_for("noext"), "other");
        assert_eq!(s.category_for("x.torrent"), "torrents");
    }

    #[test]
    fn category_dirs() {
        let mut s = Settings { download_root: PathBuf::from("D:\\dl"), ..Default::default() };
        assert_eq!(s.dir_for_category("videos"), PathBuf::from("D:\\dl").join("Videos"));
        s.organize_by_category = false;
        assert_eq!(s.dir_for_category("videos"), PathBuf::from("D:\\dl"));
    }
}
