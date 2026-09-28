# CrowDM architecture

CrowDM is a Windows download manager made of four parts that run on the
user's machine. There is no server, account or telemetry.

```
 Browser (Chrome / Edge / Firefox)
   └─ CrowDM extension ── native messaging (stdio, length-prefixed JSON)
        └─ crowdm-host.exe ── named pipe \\.\pipe\CrowDM-<user> (JSON lines)
             └─ CrowDM.exe (Tauri 2)
                  ├─ Rust core: crowdm-engine (transfers, queue, SQLite)
                  └─ WebView2 UI (React + TypeScript): main window, capture dialog
```

## Repository layout

| Path | What it is |
|---|---|
| `crates/engine` | Transfer engine library: jobs, segmented HTTP transfers, queue, scheduler, persistence. No UI code. |
| `crates/ipc` | Message types shared by the app and the host; pipe name; extension ids. |
| `crates/host` | `crowdm-host.exe`, the native messaging host. A relay that also starts the app. |
| `crates/testserver` | Controllable local HTTP server used by tests and benchmarks. |
| `apps/desktop` | The Tauri app: `src-tauri/` (Rust shell: tray, pipe server, commands, windows) and `src/` (React UI). |
| `extensions` | One extension source (`src/`) built into `build/chromium` (Chrome, Edge) and `build/firefox`. |
| `tools/e2e` | End-to-end tests that drive the real app and a real Chromium. |
| `tools/bench` | Latency, memory and throughput measurements. |
| `docs` | Design, decisions, browser integration, testing, performance, status. |

## Transfer engine (`crates/engine`)

`Engine` owns every job. Each running job has a task that drives
`transfer::run`, which manages one or more HTTP connections ("workers").

**First connection.** A new transfer sends `GET` with `Range: bytes=0-`.
A `206` with `Content-Range: bytes 0-X/TOTAL` means the server supports
ranges and reports the size. A `200` means the body is the whole file and
the size is `Content-Length`. The first response becomes the first worker's
data stream rather than being thrown away. `Accept-Encoding: identity` is
always sent so byte offsets are offsets in the real file.

**Segments.** The file is a list of segments `[start, end)` with a write
position `pos`. Workers write with positioned writes at explicit offsets and
advance `pos` only after the write returns. When a worker finishes its
segment it takes an unfinished idle segment, or splits the active segment
with the most bytes left in half (when both halves are at least 1 MiB), or
takes over the rest of a segment whose connection has stalled. All
connections stay busy until the end without a fixed up-front split.

**Validation.** Every response is checked before any byte is written:

- `206` must carry a `Content-Range` whose start equals the requested
  offset and whose total equals the known size.
- `ETag` (or `Last-Modified` when there is no ETag) must match the first
  response. Ranged requests carry `If-Range` with a strong ETag or the date,
  so a changed file answers `200` instead of mixing old and new bytes.
- A `200` to a ranged request means the server ignored the range. If size
  and validators still match, the transfer switches to one connection
  streaming from byte 0 (or keeps an existing full-body stream). If they
  differ, the job fails with "the file changed on the server".
- `416` is only accepted as "done" when the segment is already complete.

**Errors and retries.** Connection errors and truncated bodies retry the
same segment from where it stopped, with exponential backoff (and
`Retry-After` when sent). `429`/`503` on an additional connection lowers the
job's connection target instead of failing. `401/403/404/410` fail
immediately with a plain explanation. Retryable failures of the whole job
are retried after a delay (settings: attempts and delay).

**Files.** Known-size downloads are written to `<name>.crowdm-part` in the
destination folder. The part file is marked sparse and extended to its final
size, so writing near the end of a 100 GB file does not make NTFS zero-fill
the gap. On completion the file is flushed, its length checked, an optional
checksum verified, the sparse flag cleared, and it is renamed with
`MoveFileExW` without replace; on a collision it becomes `name (1).ext`.
Existing files are never overwritten.

**Durable progress.** Every 3 seconds the segment table is snapshotted, the
file is flushed (`FlushFileBuffers`), and only then is the snapshot written
to SQLite. Stored progress therefore never claims bytes that are not on
disk. After a crash or power loss a download resumes from the last
checkpoint and re-downloads at most a few seconds of data.

**Memory.** Each connection holds a 256 KiB write buffer plus what the HTTP
stack has in flight; nothing proportional to file size is kept in memory.

**Limits.** A global semaphore caps connections across all downloads
(default 48); each download has its own cap (default 8). A token bucket
implements the global and per-download speed limits; limiting happens after
reading, so TCP flow control slows the server instead of buffering data.

**Queue and schedule.** "Start download" runs a job immediately. "Add to
queue" marks it queued; the queue starts queued jobs in order, up to the
"queued downloads at the same time" setting. The schedule starts and stops
the queue at set times and days. When the queue drains the app performs the
chosen action (notify, close, sleep, or shut down after 60 seconds, which
`shutdown /a` cancels).

**Persistence.** SQLite in WAL mode. Jobs are JSON documents with a separate
segment column; settings are one JSON document. Credentials (cookies,
authorization headers) are kept only while a job is unfinished, cleared on
completion, and never sent to the UI.

## Browser hand-off

The goal is that no download is lost or made twice, whatever fails.

1. The page starts a download. The extension sees `downloads.onCreated` and
   applies the rules (enabled, http/https only, not an excluded site or file
   type, not below the minimum size, private windows only if allowed).
2. **Chrome/Edge:** the extension pauses the browser download at once.
   Chrome keeps the request open while paused, so resuming loses nothing. It
   also holds `onDeterminingFilename`, so no Save As dialog appears while
   CrowDM decides. **Firefox:** pausing can restart or lose non-resumable
   downloads, so Firefox keeps downloading while the user decides, and its
   copy is cancelled if CrowDM takes over.
3. The extension collects the cookies for the URL and the browser's
   User-Agent and sends a `capture` message.
4. `crowdm-host.exe` relays it. If CrowDM is not running, the host starts it
   with `--background` (breaking away from the browser's job object) and
   waits for its pipe. The host calls `AllowSetForegroundWindow` for the app,
   since Windows only lets the foreground process (the browser) or its
   children grant focus.
5. The app answers `received` at once and shows the dialog. Without a
   `received` within 15 s the extension resumes the browser download.
6. The app probes the URL with the browser's cookies. The probe's open
   connection is reused when the user starts the download, so one-time links
   are not requested twice. If the server answers the app with an HTML page
   or a different size than the browser saw, the dialog recommends letting
   the browser download it. If the server's file name matches an exclusion
   rule, the download goes straight back to the browser.
7. The user's choice becomes a `decision`:
   - **Start download / Add to queue**: `takeOver`, the extension cancels
     and erases the browser download.
   - **Download in Chrome/Edge/Firefox**, closing the dialog, or `Esc`:
     `resumeInBrowser`.
   - **Cancel**: `cancel`, the extension cancels the browser download.
8. If the pipe or the native messaging port closes while a decision is
   pending, the extension resumes the browser download and the app drops
   the dialog entry.
9. A second browser download of the same URL within 5 seconds (a double
   click) is cancelled in the browser instead of creating a duplicate.

The message protocol is in `BROWSER_INTEGRATION.md`.

## Desktop app (`apps/desktop`)

Rust side (`src-tauri/src`):

- `main.rs`: single instance (the pipe is the lock; a second launch forwards
  its arguments), setup, engine events, notifications, after-queue actions.
- `pipe.rs`: named-pipe server, one task per host connection.
- `captures.rs`: pending captures, probing, decisions.
- `windows.rs`: window lifecycle (below).
- `platform.rs`: registry (native messaging, autostart), Explorer and shell
  actions, Recycle Bin, power actions, data folder location.
- `commands.rs`: commands called by the UI.

**Windows and memory.** A WebView2 window costs roughly 110-150 MB of
private memory; CrowDM with no window open uses about 4 MB. The main window
is destroyed when closed to the tray. The capture dialog is created for the
first capture (about 0.5 s until painted), kept loaded for ten minutes after
its last use (about 20 ms per dialog), then released. A setting keeps it
loaded permanently.

**UI.** React 19 with Zustand. Progress arrives as one batched event every
500 ms for all running jobs; the list re-sorts only when membership or state
changes, and rows are virtualized.

## Data locations

- Installed: `%LOCALAPPDATA%\CrowDM` (database `crowdm.sqlite`, `logs\`,
  `native-messaging\` manifests).
- Portable: a file named `portable` next to `CrowDM.exe` moves all data into
  `data\` beside it.
- Registry (current user only): native messaging host keys for Chrome, Edge,
  Chromium, Brave, Chrome for Testing and Firefox; `Software\CrowDM\AppPath`;
  the `Run` value when "start with Windows" is on. The uninstaller removes
  them.
