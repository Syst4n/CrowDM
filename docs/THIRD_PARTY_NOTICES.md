# Third-party software

CrowDM runs these tools as separate processes. Their licenses remain in
effect independently of CrowDM's application source license. The installed
`tools/media` directory includes their notices and dependency license texts.

| Component in 0.3.0 | License and source |
| --- | --- |
| yt-dlp 2026.08.19 Windows executable | GPLv3+ distribution; [release and source](https://github.com/yt-dlp/yt-dlp/releases/tag/2026.08.19), [licensing details](https://github.com/yt-dlp/yt-dlp#license) |
| FFmpeg / FFprobe 7.1.1, Gyan essentials build | GPLv3+; [FFmpeg 7.1.1 source](https://ffmpeg.org/releases/ffmpeg-7.1.1.tar.xz), [upstream build provider and external libraries](https://www.gyan.dev/ffmpeg/builds/) |
| Node.js 25.8.1 | MIT and bundled third-party notices; [source](https://nodejs.org/dist/v25.8.1/node-v25.8.1.tar.xz), `Node-LICENSE.txt` |

`FFmpeg-BUILD.txt` records the exact FFmpeg configuration. `GPL-3.0.txt`
contains the complete GPL text. The FFmpeg source archive is also provided
with this preview's release assets. Binary checksums are recorded in the
release's SHA256SUMS file and media-tool inventory.

The desktop app also uses Tauri, React, Tokio, reqwest, SQLite, Zustand,
Phosphor icons, and other open-source libraries. Their individual copyright
and license files are bundled under `tools/media/licenses`; its `index.json`
lists package versions, licenses and upstream locations. Dependencies have
not been relicensed as CrowDM.

Native BitTorrent support uses librqbit 9.0.1 and its related crates under
Apache-2.0; [upstream source](https://github.com/ikatson/rqbit). Its dependency
notices are included in the same license index. Link Grabber is implemented
within CrowDM; it does not bundle JDownloader or its plugins.
