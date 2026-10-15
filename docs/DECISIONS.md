# Decisions

Short records of the choices that shape CrowDM, newest last. Each says what
was decided, why, and what would change it.

## 1. Keep Tauri 2 + React/TypeScript + a Rust engine (not WinUI 3/C#)

**Decision.** Build the desktop app with Tauri 2 (WebView2) and React, with
the transfer engine, host and all system integration in Rust.

**Why.** The questions that mattered were popup latency, idle overhead,
installer size and maintainability.

- *Popup latency.* Measured: an already-loaded WebView2 dialog paints 18-28 ms
  after a capture arrives; creating one from scratch takes about 0.5 s. A
  WinUI 3 window would likely open faster from cold, but the loaded-dialog
  path (used for 10 minutes after each capture, or always if chosen) already
  beats what a user can perceive.
- *Idle overhead.* The Rust process alone uses about 3.4 MB private memory;
  WebView2 costs 110-150 MB only while a window is loaded. Releasing windows
  when idle keeps the resident cost low. WinUI 3 would use less memory with a
  window open (tens of MB), which is the main argument for it.
- *Installer size.* The original file-transfer installer was about 4 MB;
  the video-capable preview is about 88 MiB with bundled media tools.
  WebView2 is shared with Windows. A self-contained WinUI 3/.NET app also
  requires its runtime; a framework-dependent build needs the Windows App SDK.
- *Engine.* The engine is the product's core and benefits most from Rust
  (predictable memory, no GC pauses under many connections, direct Win32 file
  APIs). With WinUI 3 it would still be Rust (FFI) or be rewritten in C#.
- *Maintainability.* One language for the core and the host, with React for
  the UI and the same web stack as the extensions.

**Would change if** idle memory with a window open becomes the main
complaint, or a native Windows 11 look is required. The engine and host are
UI-independent crates, so a WinUI 3 front end could reuse them over the same
named pipe.

## 2. Named pipe between the native host and the app

Browsers start the native host per connection; the long-running app must be
separate. A per-user named pipe (`\\.\pipe\CrowDM-<user>`, remote clients
rejected) is local-only, needs no port, and doubles as the single-instance
lock. The host is a dumb relay so protocol changes live in the app and
extension.

## 3. Pause the browser download instead of cancelling it (Chromium)

Cancelling at `onCreated` and re-downloading loses the file if CrowDM then
fails, and one-time links can only be fetched once. Chrome keeps the request
open while paused, so pausing costs nothing and "Download in Chrome" resumes
it. The browser download is only cancelled after the app answers `takeOver`.
Firefox restarts or can lose paused non-resumable downloads, so there the
browser keeps downloading until the decision.

## 4. Reuse the probe connection

The dialog needs name and size before the user decides, which needs a
request. That request's response is kept open (up to 90 s) and becomes the
first connection of the download, so a link is fetched once, not twice.

## 5. Sparse part file next to the destination

`<name>.crowdm-part` in the destination folder guarantees the final rename is
atomic on the same volume. Sparse allocation avoids NTFS zero-filling when
segments far into a large file are written first. The name is visible so
users understand what the file is.

## 6. SQLite with JSON documents

Jobs and settings change shape as features are added; JSON columns avoid a
migration for every new field. SQLite (bundled, WAL) gives crash-safe commits
without a server. Segment progress is its own column because it is rewritten
every few seconds.

## 7. HTTP/1.1 only, native TLS

Segmented downloads want one TCP connection per segment; HTTP/2 would
multiplex all segments over one connection and remove the benefit. TLS uses
Windows SChannel through `native-tls`, which honours the Windows certificate
store and corporate roots, and needs no extra build tools.

## 8. Adaptive connections

The default is 8 connections per download. A server that answers `429`/`503`
to extra connections sets the job's connection target to what it accepted,
and that limit is remembered per host for an hour. Idle connections continue
into the next idle segment on the same connection and take over stalled
segments. Measured against real servers that allow only 2-3 connections
(PERFORMANCE.md).

## 9. The dialog window is loaded on demand

WebView2 memory (110-150 MB per loaded window) dominates CrowDM's footprint.
The dialog is created for the first capture and kept for 10 minutes, the main
window is destroyed when closed to the tray. A setting keeps the dialog loaded
permanently.

## 10. Torrents and FTP deferred

See STATUS.md. HTTP(S) capture and transfer quality came first. `librqbit`
(Rust, MIT/Apache) is the likely torrent engine; it adds DHT, peer
connections, piece storage and a separate UI model (seeding, ratio, file
selection), which is a milestone of its own. FTP is rare on today's web;
SFTP needs key management UI. Both are deferred until HTTP is polished.

## 11. No license chosen yet

The repository states "all rights reserved" by default (no LICENSE file). The
owner decides before a public release; see the recommendation in STATUS.md.
