# Status and roadmap

Last updated: 2026-10-19 — 0.3.1 link checking and update access preview.

## Working and tested

- Direct sidebar access to App updates, retrying saved unavailable links, browser-opening controls, and clear HTTP 403/404 host diagnostics. Range-rejecting HTML landing pages fall back to a normal bounded page check.

- Native .torrent/magnet metadata, file selection, verified transfer, pause,
  corruption repair on resume, restart, held queues, single/multi-file output,
  collision protection and staging cleanup. Local TCP and uTP-only swarms.
- Windows handler registration for files and magnets, preserving other
  defaults. Startup arguments are handled on cold and warm launches.
- Link Grabber parsing/deduplication, safe multipart package names and bounded
  explicit download-target parsing. Saved review inbox, batch queue jobs,
  package controls, optional clipboard collection and grouped notifications.
- Owner-controlled update announcements, plain-text release notes, verified
  installer jobs and extension guidance. Public download counts by version;
  active users/installations are not tracked. See UPDATES.md.
- Segmented HTTP/HTTPS engine: range validation, If-Range, ignored-range
  fallback, redirects, unknown lengths, interruptions, stalled connections,
  servers that limit connections (learned per host), speed limits, checksums,
  durable progress, resume after restart, sparse files up to 100 GiB,
  collision-safe finalization. See TESTING.md.
- Browser hand-off with Chromium: click, pause, dialog, take over or give
  back, exclusions, duplicate clicks, cold start of the app, connection loss.
- Browser hand-off with Firefox (headless, temporary add-on): dialog, take
  over (Firefox's copy cancelled), give back.
- Video extraction for direct files, supported non-DRM HLS/DASH streams and
  YouTube, with MP4/MKV output, MP3 conversion, automatic Audio folders,
  finite-playlist checks, size estimates and completed-media verification.
- YouTube SPA and playlist navigation regression checks; menus and captures
  follow the current watch identity and reject stale selections.
- Local previews for images, representative non-black video frames and
  embedded audio artwork, with cache invalidation for older previews.
- Nightwing branding, ten palettes with independent light/dark modes and Windows matching, a separate Appearance
  section, contrast checks, optional animation and reduced-motion support.
- Themed, keyboard-accessible dropdowns, motion preview, and category-folder
  routing that refreshes pending capture dialogs when the root changes.
- Compact popup with capture-event recovery; compact resizable details;
  per-file folder buttons, sorting, older weekday/date/time, HTML exclusions
  and Windows power-management settings.
- Desktop app: download list with filters, categories and search; details
  with a live segment map; pause, resume, retry, download again, update the
  address of an expired link, per-download speed limit, remove (optionally to
  the Recycle Bin); queue with concurrency, schedule, after-queue actions;
  settings for folders and categories, connections, browsers, general
  behaviour; tray icon and menu; notifications; light and dark themes;
  keyboard shortcuts (Ctrl+N add, Ctrl+V paste a link, Delete, Space,
  Enter, Ctrl+A, arrows).
- Per-user NSIS installer bundling the host, both extensions and video tools;
  uninstaller removes the registry entries.
- Portable ZIP: extracted into a fresh directory; its marker selects adjacent
  data storage, bundled tools resolve formats and produce an audio-only MP3.

## Built but not verified end to end

- Real Chrome and Edge stable (verified with Chrome for Testing).
- Schedule, sleep/shutdown after queue, start with Windows.

## Next steps (proposed order)

1. Sign the Firefox add-on on addons.mozilla.org so it installs permanently.
2. Publish the Chromium extension to the Chrome Web Store and Edge Add-ons
   (removes the developer-mode notice; add the store ids to the host
   manifest's `allowed_origins`).
3. Code-sign `CrowDM.exe`, `crowdm-host.exe` and the installer (SmartScreen
   warns on unsigned downloads).
4. Code-signed unattended updates, if desired; current announcements and
   installer downloads are owner controlled and installation is manual.
5. Broaden real-site testing of video extraction and manifest association.
6. Broaden external torrent/router testing, v2-only support and optional
   completed-torrent seeding/ratio controls.
7. FTP/FTPS; SFTP after that.
8. Per-site credentials (HTTP basic auth prompt) and proxy settings UI.
9. Broaden Link Grabber host support; optional multipart archive extraction.
10. Localization (UI strings are in English only).

## Known issues

- WebView2 costs 110-150 MB while a window is loaded (see PERFORMANCE.md).
- A download started by a form POST cannot be repeated by CrowDM; the dialog
  recommends letting the browser finish it.
- If the destination folder is on a different drive, changing a paused
  download's folder restarts it from the beginning at the new location.

## Application source license

No application source license has been selected yet; all rights are reserved
until one is added. Bundled tools and dependencies keep their own licenses.
See THIRD_PARTY_NOTICES.md for notices and upstream source locations.
