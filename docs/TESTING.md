# Testing

## Automated suites

| Suite | Command | What it covers |
|---|---|---|
| Engine unit + integration | `cargo test -p crowdm-engine` | Filenames, Content-Disposition, sanitizing, collisions, ranges, sparse files, checksums, rate limiting, and 19 full transfers against the local test server (below). |
| IPC | `cargo test -p crowdm-ipc` | Message shapes (camelCase JSON), pipe naming. |
| Test server | `cargo test -p crowdm-testserver` | Deterministic content generator. |
| Frontend types | `pnpm --dir apps/desktop typecheck` | TypeScript. |
| App end to end | `node tools/e2e/app-capture.mjs` | Real CrowDM + real native host + fake browser: hello, capture, dialog, Start (download completes into the category folder), "Download in Chrome", HTML-page warning, Cancel, duplicate clicks, connection loss, cold start of the app by the host. 17 checks. |
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

- The Firefox extension end to end (the manifest and code paths are shared
  with Chromium except for pausing; not yet run in Firefox).
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
