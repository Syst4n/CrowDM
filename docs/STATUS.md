# Status and roadmap

Last updated: 2026-09-28 (version 0.1.0, not yet released).

## Working and tested

- Segmented HTTP/HTTPS engine: range validation, If-Range, ignored-range
  fallback, redirects, unknown lengths, interruptions, stalled connections,
  servers that limit connections (learned per host), speed limits, checksums,
  durable progress, resume after restart, sparse files up to 100 GiB,
  collision-safe finalization. See TESTING.md.
- Browser hand-off with Chromium: click, pause, dialog, take over or give
  back, exclusions, duplicate clicks, cold start of the app, connection loss.
- Browser hand-off with Firefox (headless, temporary add-on): dialog, take
  over (Firefox's copy cancelled), give back.
- "Download this video" button for file-backed videos; context menu entry.
- Desktop app: download list with filters, categories and search; details
  with a live segment map; pause, resume, retry, download again, update the
  address of an expired link, per-download speed limit, remove (optionally to
  the Recycle Bin); queue with concurrency, schedule, after-queue actions;
  settings for folders and categories, connections, browsers, general
  behaviour; tray icon and menu; notifications; light and dark themes;
  keyboard shortcuts (Ctrl+N add, Ctrl+V paste a link, Delete, Space,
  Enter, Ctrl+A, arrows).
- Per-user NSIS installer (about 4 MB) bundling the host and both extensions;
  uninstaller removes the registry entries.

## Built but not verified end to end

- Real Chrome and Edge stable (verified with Chrome for Testing).
- Schedule, sleep/shutdown after queue, start with Windows.
- Portable mode (`portable` marker file next to `CrowDM.exe`).

## Next steps (proposed order)

1. Sign the Firefox add-on on addons.mozilla.org so it installs permanently.
2. Publish the Chromium extension to the Chrome Web Store and Edge Add-ons
   (removes the developer-mode notice; add the store ids to the host
   manifest's `allowed_origins`).
3. Code-sign `CrowDM.exe`, `crowdm-host.exe` and the installer (SmartScreen
   warns on unsigned downloads).
4. Auto-update (Tauri updater with GitHub Releases).
5. HLS/DASH "video" downloads for unencrypted streams (m3u8 playlists),
   clearly limited to content without DRM.
6. Torrent and magnet support with `librqbit` (see DECISIONS.md #10):
   estimated as a full milestone: engine integration, file selection, seeding
   policy, and UI for peers and ratio.
7. FTP/FTPS; SFTP after that.
8. Per-site credentials (HTTP basic auth prompt) and proxy settings UI.
9. Import from the browser's clipboard monitor ("download links in
   clipboard"), batch URL lists.
10. Localization (UI strings are in English only).

## Known issues

- WebView2 costs 110-150 MB while a window is loaded (see PERFORMANCE.md).
- A download started by a form POST cannot be repeated by CrowDM; the dialog
  recommends letting the browser finish it.
- If the destination folder is on a different drive, changing a paused
  download's folder restarts it from the beginning at the new location.

## License (decision needed before a public release)

Recommendation: **GPL-3.0-or-later** if you want every redistributed or
modified version of CrowDM to stay open source, or **MIT** (or MIT OR
Apache-2.0, the Rust ecosystem's usual pair) if you want the widest reuse,
including in closed products. All current dependencies (Tauri, React,
reqwest, tokio, rusqlite/SQLite, Phosphor icons) allow either. For a free
end-user app published on GitHub, GPL-3.0 is a common choice that protects the
project from closed forks; MIT is simpler for contributors and companies.
Until you choose, the repository has no license, which means all rights
reserved.
