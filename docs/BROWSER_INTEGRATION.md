# Browser integration

## Pieces

| Piece | Where |
|---|---|
| Extension source | `extensions/src` (`background.js`, `video.js`, popup) |
| Built extensions | `extensions/build/chromium` (Chrome, Edge, Brave, Chromium) and `extensions/build/firefox`, produced by `node extensions/build.mjs` |
| Native messaging host | `crates/host` → `crowdm-host.exe`, installed next to `CrowDM.exe` |
| Host registration | Written by CrowDM at every start (`apps/desktop/src-tauri/src/platform.rs`), removed by the uninstaller |

## Extension identity

- Chromium extension id: `pomhhnfijfeecghaflldjibhaofbiabm`. It is fixed by
  the public key in the built `manifest.json` (`key` field), so an unpacked
  install always gets this id and matches the host manifest's
  `allowed_origins`. The matching private key is not in the repository; it is
  only needed to upload the same id to the Chrome Web Store.
- Firefox add-on id: `crowdm@syst4n.github.io`
  (`browser_specific_settings.gecko.id`), listed in the host manifest's
  `allowed_extensions`.

If the extension is published to the Chrome Web Store or Edge Add-ons under a
different id, add that id to `allowed_origins` in `platform.rs`.

## Registration

At start-up CrowDM writes two host manifests to
`%LOCALAPPDATA%\CrowDM\native-messaging\` (one Chromium-style with
`allowed_origins`, one Firefox-style with `allowed_extensions`), each pointing
at the absolute path of `crowdm-host.exe`, and sets the default value of these
keys (current user only) to the manifest path:

```
HKCU\Software\Google\Chrome\NativeMessagingHosts\com.crowdm.host
HKCU\Software\Microsoft\Edge\NativeMessagingHosts\com.crowdm.host
HKCU\Software\Chromium\NativeMessagingHosts\com.crowdm.host
HKCU\Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.crowdm.host
HKCU\Software\Google\Chrome for Testing\NativeMessagingHosts\com.crowdm.host
HKCU\Software\Mozilla\NativeMessagingHosts\com.crowdm.host
```

It also stores its own path in `HKCU\Software\CrowDM\AppPath` so the host can
find the app. Settings > Browsers > "Repair browser connection" rewrites all
of this. A portable copy registers itself the same way the first time it
runs; moving the folder afterwards needs one start (or Repair) to update the
paths.

## Installing the extensions

The installer copies both extension folders to
`<install folder>\extensions\`. Settings > Browsers > "Open extension folder"
opens it.

- **Chrome / Edge / Brave:** open `chrome://extensions` (or
  `edge://extensions`), turn on Developer mode, choose **Load unpacked**, and
  select the `chromium` folder. This install is permanent. Chrome shows a
  "developer mode extensions" notice; publishing to the Chrome Web Store and
  Edge Add-ons removes it.
- **Firefox:** release Firefox only installs signed add-ons permanently.
  Without signing, open `about:debugging#/runtime/this-firefox`, choose
  **Load Temporary Add-on**, and select `firefox\manifest.json`; it stays until
  Firefox restarts. For a permanent install the add-on must be signed by
  Mozilla (free, through addons.mozilla.org, listed or unlisted).

## Permissions and why

| Permission | Used for |
|---|---|
| `downloads` | See new browser downloads; pause, resume, cancel and erase them. |
| `nativeMessaging` | Talk to `crowdm-host.exe`. |
| `cookies` + host permission `<all_urls>` | Send the site's cookies with the hand-off, so links that need a signed-in session work in CrowDM. |
| `contextMenus` | "Download with CrowDM" on links, images, audio and video. |
| `storage` | Remember interception settings and the per-browser on/off switch. |
| content script on http/https pages | The "Download video" button. |

Nothing is sent anywhere except to the local CrowDM app.

## Messages

Native messaging frames are a 32-bit little-endian length plus UTF-8 JSON.
On the pipe, each message is one JSON line. The host forwards messages
unchanged. Types live in `crates/ipc/src/lib.rs`.

Extension to app:

```json
{"type":"hello","id":"1","browser":"chrome","extensionVersion":"0.1.0","protocol":1}
{"type":"capture","id":"dl-7-3","browser":"chrome","capture":{
  "kind":"intercepted","browserDownloadId":7,"url":"https://...","finalUrl":null,
  "referrer":"https://page","pageUrl":"https://page","pageTitle":null,
  "fileName":null,"mime":"application/zip","size":123,"incognito":false,
  "headers":[{"name":"User-Agent","value":"..."},{"name":"Cookie","value":"a=b"}]}}
{"type":"captureGone","id":"dl-7-3"}
{"type":"ping","id":"9"}
{"type":"activate","args":[]}
```

`kind` is `intercepted` (a browser download waiting on a decision), `menu`
(context menu), `video` (video button) or `manual` (typed in the app).

App to extension:

```json
{"type":"hello","replyTo":"1","appVersion":"0.1.0","protocol":1,"settings":{...}}
{"type":"received","replyTo":"dl-7-3"}
{"type":"decision","replyTo":"dl-7-3","decision":"takeOver"}
{"type":"settings","settings":{"enabled":true,"excludedSites":[],"excludedExtensions":[],"minSize":0,"includePrivate":false,"videoButton":true}}
{"type":"error","replyTo":null,"code":"appUnavailable","message":"..."}
```

`decision` is `takeOver` (cancel and erase the browser download), `resumeInBrowser`
or `cancel`.

## Failure behaviour

| What goes wrong | What happens |
|---|---|
| CrowDM is not running | The host starts it and waits up to 20 s for its pipe. |
| CrowDM cannot be started or never answers | No `received` within 15 s: the browser download is resumed. |
| The pipe or the port closes during a decision | The extension resumes every waiting download; the app removes them from its dialog. |
| The server gives CrowDM a web page or a different size than the browser saw | The dialog recommends "Download in <browser>". |
| The server gives CrowDM an error | The dialog shows the error and offers "Download in <browser>". |
| The server's file name matches an exclusion | The download goes back to the browser without asking. |
| The user closes the dialog or presses Esc | The download goes back to the browser. |
| The same link is clicked twice within 5 s | The second browser download is cancelled. |
| `blob:`/`data:` downloads (made by the page's script) | Never intercepted; they only exist inside the browser. |

## Video button

`video.js` shows "Download video" over a `<video>` element when the pointer is
over it, the element is at least 200 x 110 px, and its source is an
`http(s)` URL (a plain file). Videos whose source is a `blob:` URL are assembled
by the page from pieces (MediaSource streaming, as used by most streaming
sites); they get no button, because they are not one downloadable file.
DRM-protected media is never offered. The button lives in a closed shadow root
inside a fixed-position element so it cannot change the page's layout or be
styled by the page, and it follows the video into fullscreen.

## Known limits

- Chrome and Edge give extensions the download only after the browser has
  started it. The pause happens within milliseconds, but the first bytes may
  already have been fetched once by the browser.
- Downloads started by a form `POST` cannot be repeated by CrowDM (the request
  body is not available to extensions). CrowDM's own request then usually gets
  an error or a page, and the dialog recommends letting the browser finish.
- Firefox is not paused while the dialog is open (see ARCHITECTURE.md), so a
  large file downloads in Firefox in parallel until you decide.
- The Firefox build has not been exercised end to end yet (see STATUS.md).
