# Performance

Measured on the development machine: Windows 11 Pro (build 26200), AMD Ryzen 5
7600X, 32 GB RAM, NVMe SSD, WebView2 153, home internet connection of roughly
150-250 Mbit/s. Release builds. Numbers are what was observed, not targets;
re-run the tools below on your machine before quoting them.

## Dialog latency (browser download to CrowDM dialog)

`tools/bench/popup-latency.mjs` drives the real native host with a fake
browser; the dialog reports when it has painted each capture
(`capture dialog painted N ms after the capture arrived` in the app log).
`tools/e2e/browser-capture.mjs` measures from a real click in Chromium.

| Situation | Result |
|---|---|
| App acknowledges a capture to the browser | 0.5-1 ms |
| Dialog painted, dialog already loaded | 18-28 ms (median 19) |
| Dialog painted, first capture after a quiet period (window created) | 480-530 ms |
| Real Chromium: click to dialog painted, warm | 46 ms |
| Real Chromium: click to dialog, CrowDM not running (host starts it) | 754 ms |
| Real Firefox: click to capture arriving in the app, host not yet running / running | 178 ms / 17 ms |
| Real Firefox: capture to dialog painted, first / second capture | 561-687 ms / 25-27 ms |
| Native host connects to an already running app | ~110 ms after the browser starts it |

Default behaviour: the dialog is created on the first capture and kept loaded
for 10 minutes after use, so bursts of downloads get the fast path while an
idle CrowDM does not hold a WebView. "Keep the download dialog loaded at all
times" in Settings makes every dialog take the fast path.

## Memory (all CrowDM processes, private bytes)

Measured with `tools/bench/memory.ps1`, which sums CrowDM.exe and every child
process (the WebView2 browser, renderer, GPU and utility processes).

| State | Processes | Private | Working set |
|---|---|---|---|
| Tray only, no window loaded | 1 | 3.4 MB | 17.6 MB |
| Dialog loaded (hidden or shown) | 7 | 155 MB | 350 MB |
| Dialog loaded, WebView2 started with `--disable-gpu` (not adopted) | 7 | 114 MB | 330 MB |

WebView2 accounts for almost all memory; the Rust core is a few MB. This is
why the main window is destroyed when closed to the tray and the dialog is
released when idle.

## Transfer engine

`cargo run --release -p crowdm-engine --example get -- <url> --connections N`
downloads with the engine alone and prints time, speed and the process's peak
working set.

**Local server, 4 GiB file** (loopback, measures the engine and disk path):

| Connections | Time | Speed | Peak working set |
|---|---|---|---|
| 1 | 3.14 s | 1306 MB/s | 13.7 MB |
| 8 | 2.33 s | 1757 MB/s | 21.8 MB |

Memory stays flat with file size: the 100 GiB sparse test in the test suite
writes segments across the whole file with the same bounded buffers.

**Local server that caps each connection at 2 MiB/s** (simulates servers and
long-distance links where one connection cannot fill the line), 64 MiB file:

| Connections | Time | Speed |
|---|---|---|
| 1 (same as a browser or curl) | 32.0 s | 2.0 MB/s |
| 4 | 8.1 s | 7.9 MB/s |
| 8 | 4.2 s | 15.4 MB/s |
| 16 | 2.2 s | 28.7 MB/s |

**Public servers over the internet** (100 MiB files). The home connection
tops out around 20-30 MB/s, and one connection already fills it, so extra
connections cannot add much here. Both test servers also limit parallel
connections: Hetzner accepts about 2 and OVH about 3, answering `429` to more.

| Server | curl, 1 connection | CrowDM, 1 connection | CrowDM, 8 connections |
|---|---|---|---|
| fsn1-speed.hetzner.com | 17.4-18.8 MB/s | 17.6-20.2 MB/s | 22.4-23.5 MB/s (best runs) |
| proof.ovh.net | 27.6-29.6 MB/s | 18.3-29.9 MB/s | 26.7-29.1 MB/s (best runs) |

Repeated runs were noisy: after a few downloads both servers started to
throttle or refuse connections for every tool, including curl. The raw runs
are in `bench-results/network-2026-09-28.json`. The honest summary: on this
connection CrowDM matches a single-connection download and sometimes beats
it; the large multi-connection gains appear when a single connection is
limited (distant or throttling servers), as the local simulation shows.

### A problem these measurements found

The first internet runs showed 8 connections taking 47-66 s against OVH where
1 connection took 3-5 s. Tracing showed the servers answering `429` to extra
connections, and CrowDM retrying those with backoff and new connections while
segments waited. Fixes, all covered by tests:

- a `429`/`503` on a connection while others are working lowers the
  connection target at once, and the limit is remembered for that host for an
  hour;
- ranged requests are open-ended, so a connection that finishes its segment
  continues into the next idle segment on the same connection instead of
  opening a new one;
- a connection that receives nothing for 5 s has the rest of its segment
  taken over by an idle connection, and leftover connections are dropped as
  soon as every byte has arrived.

After the fix the same servers gave 22.4 MB/s (8 connections) vs 17.6 MB/s
(1) on Hetzner and 29.1 vs 29.9 MB/s on OVH.

## Comparison with IDM

Internet Download Manager is installed on the development machine, but its
command line (`IDMan.exe /d <url> /n`) hands the job to the running IDM
window, which did not complete the files within 10 minutes during automated
runs (it likely waited on its own dialog), so no IDM numbers are claimed. To
compare by hand under the same conditions: download the same file with IDM and
with CrowDM set to the same number of connections, one after the other, three
times each, and compare the times shown by each app. No speed multiplier over
IDM or browsers is claimed.

## UI responsiveness

Progress for all running downloads arrives as one event every 500 ms; the list
is virtualized and re-sorted only when downloads are added, removed or change
state. This was checked by reasoning and code review, not measured under load
with thousands of rows.
