# Download and install CrowDM

CrowDM 0.2.1 is an early Windows preview for Windows 10/11, 64-bit.

## Get the app

1. Open [CrowDM Releases](https://github.com/Syst4n/CrowDM/releases).
   For this preview, use the [0.2.1 release](https://github.com/Syst4n/CrowDM/releases/tag/v0.2.1).
2. Download `CrowDM_0.2.1_x64-setup.exe`. Run it to install for your Windows
   user. Administrator rights are not required. WebView2 is installed if needed.
3. Open CrowDM from the Start menu.

Preview builds are currently unsigned. Download from this repository and
compare the SHA-256 hash with the release's `SHA256SUMS.txt`:

```powershell
Get-FileHash .\CrowDM_0.2.1_x64-setup.exe -Algorithm SHA256
```

The installer includes video tools and the browser extensions. They are
installed under `%LOCALAPPDATA%\CrowDM`. No Python installation is needed.

## Chrome and Edge

1. In CrowDM, open **Settings → Browsers → Open extension folder**.
2. Open `chrome://extensions` in Chrome, or `edge://extensions` in Edge.
3. Enable **Developer mode**, choose **Load unpacked**, and select the
   `chromium` folder from the extension directory.
4. Pin CrowDM to the browser toolbar if desired. Its popup reports the
   connection state and can pause capture for that browser.

Alternatively, download the Chromium extension ZIP from the release,
extract it into a permanent folder, and choose that folder in step 3.
The desktop app must also be installed and started once to register the
native messaging connection. Keep the extracted folder in place.

**Use CrowDM as your download handler:** disable the browser extensions of
Free Download Manager, Internet Download Manager or other download managers,
or turn off their browser integration. Keep CrowDM's capture enabled.
Browsers do not provide an exclusive priority order for competing download
manager extensions. CrowDM's app setting cannot override another extension.

## Firefox

Open `about:debugging#/runtime/this-firefox`, choose **Load Temporary Add-on**,
then select `manifest.json` in CrowDM's `firefox` folder. The Firefox ZIP in
the release contains the same files. This preview is not store-signed, so
Firefox removes the temporary add-on after a browser restart. A permanent
Firefox store release is planned.

## Download files, videos and audio

- Click a normal file download link. Choose a filename and folder in CrowDM,
  then **Start download** or **Add to queue**. **Download in browser** hands
  the download back to the browser.
- Add a URL directly with **Ctrl+N**. Paste a supported video page to resolve
  available formats, rather than downloading the page as HTML.
- Hover over a video. The faint Nightwing icon at the top right expands to
  **Download video**. Click it and choose an available format/quality.
- Choose **MP4** for an MP4 video with audio, or **MKV** for the available
  source codecs in a flexible container. Choices depend on the source.
- Choose **MP3** to download and convert the audio. CrowDM automatically
  selects Audio and its folder; you can still choose a custom destination.
  Available artwork is embedded. Conversion does not improve source quality.
- Sizes marked **About** or **≈** are estimates. Streaming bitrate and audio
  processing can change the final size. Finite HLS streams are checked for
  an ending; live, unfinished and protected streams are excluded.
- Pause and resume from the list. Use the folder button beside a row to
  reveal its file. Drag the divider above the details panel to resize it.
- Sort with the toolbar menu. Adjust themes and motion under
  **Settings → Appearance**; startup and sleep options are under **General**.
- In Appearance, choose **Light**, **Dark** or **Match Windows**, then pick
  a palette. Use **Preview motion** to check animation. Windows reduced-motion
  preferences are followed unless you turn that preference off explicitly.
- In **Settings → Folders and categories**, enable **Sort files into category
  folders** to save files under `Videos`, `Audio`, `Archives`, and the other
  category folders inside your chosen download folder. With it off, files go
  directly into the download folder. Changing the root does not move old files.

## Update an existing installation

Pause active downloads, close CrowDM, and run the newer installer. Download
history, settings and partial files are retained. Then open the browser's
extension page and click **Reload** on CrowDM. Refresh video tabs that were
already open so they receive the updated content script. After that, moving
between YouTube videos and playlist items does not require a page refresh.

## Portable version

Extract the portable ZIP into a writable folder and run `CrowDM.exe`. Keep
the native host, `tools`, `extensions`, and the empty `portable` marker
together. History and settings live in its `data` folder. Run the app once
after moving it to register its new browser connection path.

## Troubleshooting

- **Another manager opens first:** disable its browser extension/integration.
- **No dialog:** open CrowDM, check its toolbar popup, and use
  **Settings → Browsers → Repair browser connection**. Check that capture is
  enabled and the site/file type is not excluded.
- **Wrong or stale video:** reload the updated extension and refresh that
  tab once after upgrading. Reopen the download menu on the current video.
- **Video unavailable:** it may be private, protected, live, expired,
  region-limited or unsupported. Universal downloading is not possible.
- **Large stream:** an unknown size does not mean a small file. High-bitrate
  films can be several gigabytes. CrowDM estimates finite playlists, stops
  unavailable fragments and unusually oversized transfers, and verifies
  the completed media duration when the source provides it.
- **Missing thumbnail:** previews use local completed files. Images display
  their contents, videos display a frame, and audio displays embedded cover
  art. Files without a decodable preview keep the category icon.

Remove CrowDM through Windows **Settings → Apps**. Downloaded files are not
deleted. Keep a copy of its database and download folders when backing up.
