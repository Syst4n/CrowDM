# Testing

## 0.3.1 focused regression (2026-10-20)

23 isolated native-app checks passed: range-rejecting landing pages, HTTP 403 diagnostics, retry control availability, rejection of non-web browser targets, persisted link inbox, duplicate handling, byte-exact multipart downloads, held queues, package controls, settings and direct sidebar update navigation. Three Rust link-parser tests passed. Type checking and the optimized Windows app build passed. Screenshots were captured after entrance animations completed.

A real failing host returned a Cloudflare verification page with HTTP 403 for both range and ordinary requests. Browser verification and unsupported host resolvers remain limitations; these tests do not establish universal file-host support.

## Automated suites

Local update regression checks (2026-10-15):

The 0.2.1 preview passed 96 native UI checks, 7 background checks, 12 overlay
checks, 5 real Chromium/native-host checks and 46 Rust workspace tests with
real media fixtures enabled. A fresh portable extraction also completed MP3
conversion using its bundled tools. The installed upgrade preserved all
eight existing jobs, including the paused job and its stored destination.

- `node --test extensions/tests/background.test.mjs`: HTML exclusions,
  YouTube watch-page and playlist/SPA routing, stale-selection rejection,
  frame-scoped manifests, navigation cleanup,
  private sessions, disabled interception and site exclusions.
- `$env:CROWDM_MEDIA_TESTS='1'; cargo test -p crowdm-engine --test media`:
  generates cookie-protected direct MP4, HLS and DASH fixtures, exercises real
  yt-dlp/FFmpeg, verifies MP4 and MKV video/audio tracks, audio-only MP3,
  live-HLS rejection, selected resolutions, pause/resume, queues and persisted
  completion. Run `scripts/prepare-media.ps1` first.
- `node tools/e2e/updates.mjs`: isolated native host + actual WebView2 UI,
  sorting, timestamps, details drag, popup sizing/reuse/stale probes, HTML
  hand-back, MP4 selection, MP3 completion and Audio folder changes, local image,
  video (including a black intro) and embedded-artwork previews, twenty
  mode/palette combinations with text/button contrast, live themed dropdowns,
  motion preview and reduced-motion preferences, changed download roots and
  category-folder routing, and narrow-window layout. Requires the release
  desktop/host and `playwright-core` from `tools/package.json`. Screenshots
  and results go under `target/ui-updates-*`.
- `node tools/e2e/video-overlay.mjs`: isolated browser component check for
  icon placement/opacity, hover expansion, available-quality menu, selected
  quality messages, acknowledgement and playlist/SPA changes. Native quality enumeration is
  checked in the desktop suite, and the media suite verifies a real 90p
  selection from a 90p/180p HLS master produces a 90p output file.
- `node tools/e2e/browser-preview.mjs`: real Chromium, extension, native host
  and WebView2 in separate profiles. A normal file is byte-exact and the
  browser copy is cancelled after takeover; the video overlay queries native
  formats and completes an MP3. Only a temporary, uniquely named test host
  registry key is created and removed. It never changes the user's host.

`CROWDM_TEST_INSTANCE` isolates the pipe and suppresses browser registration;
`CROWDM_DATA_DIR` isolates data. The new regression runner does not terminate
the user's app or change the installed browser connection.

| Suite | Command | What it covers |
|---|---|---|
| Engine unit + integration | `cargo test -p crowdm-engine` | Filenames, Content-Disposition, sanitizing, collisions, ranges, sparse files, checksums, rate limiting, and 19 full transfers against the local test server (below). |
| IPC | `cargo test -p crowdm-ipc` | Message shapes (camelCase JSON), pipe naming. |
| Test server | `cargo test -p crowdm-testserver` | Deterministic content generator. |
| Frontend types | `pnpm --dir apps/desktop typecheck` | TypeScript. |
| App end to end | `node tools/e2e/app-capture.mjs` | Real CrowDM + real native host + fake browser: hello, capture, dialog, Start (download completes into the category folder), "Download in Chrome", HTML-page warning, Cancel, duplicate clicks, connection loss, cold start of the app by the host. 17 checks. |
| Firefox end to end | `node tools/e2e/firefox-capture.mjs` | Installed Firefox, headless, throwaway profile, add-on loaded as temporary through geckodriver: click, dialog, Start (Firefox's own copy cancelled, CrowDM completes the file), "Download in Firefox" (Firefox finishes it, CrowDM does not). 8 checks. |
| Browser end to end | `node tools/e2e/browser-capture.mjs` | Real Chromium (Chrome for Testing, throwaway profile) with the real extension: click a link, browser download paused, dialog shows, Start takes over and cancels/erases the browser download, the file completes; "Download in Chrome" lets the browser finish; exclusions return to the browser; video button appears and sends the video; clicking while CrowDM is closed starts it. 12 checks. |

Set `CROWDM_TEST_DIR` to a folder on a drive with plenty of free space for the
100 GiB sparse test (it shrinks the size to what the drive allows and skips
below 4 GiB).

The end-to-end suites need a release build and the test server:

```
node scripts/prepare-sidecar.mjs --release
pnpm --dir apps/desktop tauri build --no-bundle
cargo run --release -p crowdm-testserver        # separate terminal, 127.0.0.1:8765
pnpm --dir tools install
node tools/e2e/app-capture.mjs --shots docs/screenshots
node tools/e2e/browser-capture.mjs
```

They start CrowDM with `CROWDM_DATA_DIR` pointing at `target/e2e-data`, so
your own downloads list is untouched, and they drive the WebView through
WebView2's DevTools port (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`), not by
moving the mouse. They do re-register the native messaging host (per user)
to the build being tested; start your installed CrowDM once afterwards to
point it back.

## Engine transfer scenarios

All run against `crates/testserver`, which generates file content from
`(seed, offset)` so every byte can be verified without storing files.

| Test | Scenario |
|---|---|
| `multi_connection_download_is_byte_exact` | 20 MiB over several ranged connections, byte-exact. |
| `server_without_range_support_uses_one_connection` | No `Accept-Ranges`: exactly one request. |
| `server_that_ignores_ranges_falls_back_safely` | Advertises ranges but always answers `200`. |
| `unknown_length_download_completes` | Chunked body with no length. |
| `interrupted_connections_are_resumed` | First 10 responses cut off after 700 KB. |
| `redirects_are_followed_and_recorded` | Three redirects; final URL stored. |
| `pause_and_resume_continue_where_they_stopped` | Pause mid-way, resume with ranged requests. |
| `download_survives_an_app_restart` | Engine shut down mid-transfer; a new engine on the same database resumes and completes. |
| `changed_remote_file_is_detected_on_resume` | ETag changes while paused: fails with "file changed", no bad file produced. |
| `connection_limit_of_server_is_respected` | Server allows 2 connections and answers `503` to more. |
| `stalled_connections_are_taken_over` | Three extra connections never get an answer; download still finishes quickly. |
| `checksum_is_verified` | Correct and wrong SHA-256. |
| `speed_limit_is_applied` | 2 MB/s per-download limit. |
| `existing_files_are_never_overwritten` | Name collision becomes `name (1).ext`. |
| `probe_connection_is_reused_by_the_download` | Exactly one request for probe + download. |
| `queue_limits_concurrency_and_reports_completion` | Queue concurrency and `QueueFinished`. |
| `huge_file_is_sparse_and_bounded` | 100 GiB part file: allocated size stays small, segments in the second half hold the right bytes. |
| `missing_file_fails_without_retrying_forever` | 404 fails immediately. |

## What has not been tested

- Real public torrent swarms, router-specific UPnP behavior and peak Internet
  torrent throughput. The 0.3 tests use generated local TCP/uTP swarms and a
  local tracker with all public discovery disabled.
- A future announced update installed through the in-app update screen.
  Announcement validation and the download integrity mechanism are checked;
  there is intentionally no live announcement during preview development.
- Real Google Chrome and Microsoft Edge with the extension loaded unpacked
  (Chrome for Testing was used; stable Chrome no longer accepts
  `--load-extension`, so automating it needs a different approach).
- Signed-in / cookie-protected downloads on real sites.
- The schedule firing at real clock times, sleep/shutdown after the queue,
  and start with Windows (code paths exist; not exercised).
- Proxies, captive portals, IPv6-only networks.
- Downloads larger than 4 GiB completed end to end over a real network (the
  100 GiB test covers allocation and offsets, not a full 100 GiB transfer).
- The installer on a clean Windows 10 machine (tested only as a silent
  per-user install on the development machine).

## 0.3 torrents, packages and update checks

- 56 Rust tests, including existing real-media and large sparse-file
  coverage. New native tests resolve real local magnet metadata, select files,
  transfer over TCP and uTP alone, pause, corrupt saved pieces, reopen, verify
  and repair them, hold/release a queue, protect collisions, clean staging and
  save both single-file and multi-file torrents. Unsafe Windows paths and
  invalid file selections are rejected. Registration uses a disposable HKCU
  test subtree and checks that an existing other default is preserved.
  The sparse fixture is up to 100 GiB, bounded by available disk space, and
  uses a limited transfer rate so downloaded bytes do not invalidate its
  allocated-space assertion on fast local networks.
- 18 native torrent UI checks in `tools/e2e/torrents.mjs`: cold file opening
  with spaces, warm magnet activation, selected bytes, real transfer output,
  queue behavior, settings persistence and small-window fit.
- 20 native Link Grabber checks in `tools/e2e/link-grabber.mjs`: mixed paste,
  deduplication before/after host resolution, offline/login errors, packages,
  no captures/transfers while checking, saved inbox after restart, held queue,
  exact output bytes, page collection and update/extension/counter guidance.
- Existing 96 native appearance/dialog/preview/routing checks, eight extension
  background checks and real Chromium/native-host checks passed against
  the 0.3 preview. Generated local files are used instead of private downloads.
- 14 selection-overlay checks cover alignment to selected text, faint/expanded
  appearance, one-click batches, linked labels, preserved selection, theme
  switching, private/input exclusions and stale-navigation hiding. Real browser
  native-host coverage includes selecting links, opening Link Grabber and
  completing both package parts. Transient Windows file-lock finalization has
  its own regression test. Notification identity registration is tested in a
  disposable registry subtree; app identity matches the installer identifier.
- Clipboard monitoring is off by default; opt-in persistence/polling code is
  checked, but tests do not overwrite the user's Windows clipboard.

Run the native UI scripts after building the release app and the
`torrent_seed` example. They use separate data folders and single-instance
names. Set `CROWDM_PLAYWRIGHT` to a Playwright Core module location;
`CROWDM_CHROMIUM` selects the isolated Chrome for Testing executable for the
browser suite. Do not point tests at the installed user's database.
