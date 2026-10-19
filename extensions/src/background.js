// CrowDM browser extension: background script shared by Chrome, Edge and
// Firefox. It catches browser downloads, hands them to the CrowDM app through
// native messaging, and keeps the browser download as a fallback until the
// app confirms it took over.

const B = globalThis.browser ?? globalThis.chrome;
const HOST = "com.crowdm.host";
const IS_FIREFOX = typeof globalThis.browser !== "undefined" && navigator.userAgent.includes("Firefox");
const BROWSER = IS_FIREFOX ? "firefox" : navigator.userAgent.includes("Edg/") ? "edge" : "chrome";
const VERSION = B.runtime.getManifest().version;

// How long to wait for the app to acknowledge a capture (covers a cold start).
const RECEIVE_TIMEOUT_MS = 15000;

const DEFAULT_SETTINGS = {
  theme: "system",
  appearance: null,
  animations: true,
  respectReducedMotion: true,
  enabled: true,
  excludedSites: [],
  excludedExtensions: [],
  minSize: 0,
  includePrivate: false,
  videoButton: true,
  skipWebPages: true,
};

let settings = { ...DEFAULT_SETTINGS };
/** Catching switched off from this browser's toolbar button. */
let pausedHere = false;
let port = null;
let appReady = false;
let lastError = null;
let nextId = 1;
/** Messages waiting for the hello round-trip. */
let outbox = [];

/**
 * Captures waiting on the app, by request id:
 * { downloadId, received, timer, suggest, sent }
 */
const pending = new Map();
const manualReceipts = new Map();
const videoQueries = new Map();
const batchReceipts = new Map();
/** Download ids we are handling (to ignore duplicate events). */
const handled = new Map();

const ready = B.storage.local.get(["settings", "pausedHere"]).then((r) => {
  if (r && r.settings) settings = { ...DEFAULT_SETTINGS, ...r.settings };
  pausedHere = !!(r && r.pausedHere);
});

function log(...args) {
  console.debug("[CrowDM]", ...args);
}

// ----- native messaging ------------------------------------------------

function connect() {
  if (port) return port;
  appReady = false;
  try {
    port = B.runtime.connectNative(HOST);
  } catch (e) {
    lastError = String(e && e.message ? e.message : e);
    port = null;
    return null;
  }
  port.onMessage.addListener(onAppMessage);
  port.onDisconnect.addListener(() => {
    const err = B.runtime.lastError || port?.error;
    lastError = err ? String(err.message || err) : "CrowDM closed the connection.";
    log("disconnected:", lastError);
    port = null;
    appReady = false;
    outbox = [];
    for (const finish of manualReceipts.values()) finish(false);
    manualReceipts.clear();
    for (const finish of videoQueries.values()) finish({ ok: false, error: "CrowDM disconnected. Open the app and try again." });
    videoQueries.clear();
    for (const finish of batchReceipts.values()) finish({ok:false,error:"CrowDM disconnected. Open the app and try again."});
    batchReceipts.clear();
    // Anything still waiting goes back to the browser, so nothing is lost.
    for (const [id, p] of pending) {
      giveBack(p);
      pending.delete(id);
    }
    broadcastStatus();
  });
  port.postMessage({ type: "hello", id: String(nextId++), browser: BROWSER, extensionVersion: VERSION, protocol: 1 });
  return port;
}

function send(msg) {
  if (!connect()) return false;
  if (!appReady && msg.type !== "hello") {
    outbox.push(msg);
    return true;
  }
  try {
    port.postMessage(msg);
    return true;
  } catch (e) {
    lastError = String(e);
    return false;
  }
}

function onAppMessage(msg) {
  switch (msg.type) {
    case "hello":
      appReady = true;
      lastError = null;
      applySettings(msg.settings);
      for (const m of outbox.splice(0)) port.postMessage(m);
      broadcastStatus();
      break;
    case "settings":
      applySettings(msg.settings);
      break;
    case "received": {
      batchReceipts.get(msg.replyTo)?.({ok:true});
      manualReceipts.get(msg.replyTo)?.(true);
      const p = pending.get(msg.replyTo);
      if (p) {
        p.received = true;
        clearTimeout(p.timer);
      }
      break;
    }
    case "decision":
      onDecision(msg.replyTo, msg.decision);
      break;
    case "error":
      batchReceipts.get(msg.replyTo)?.({ok:false,error:msg.message});
      videoQueries.get(msg.replyTo)?.({ ok: false, error: msg.message });
      lastError = msg.message;
      log("app error:", msg.code, msg.message);
      if (msg.code === "appUnavailable" || msg.code === "appClosed") {
        for (const [id, p] of pending) {
          giveBack(p);
          pending.delete(id);
        }
      }
      broadcastStatus();
      break;
    case "videoOptions":
      videoQueries.get(msg.replyTo)?.({ ok: true, title: msg.title, formats: msg.formats });
      break;
  }
}

function applySettings(s) {
  if (!s) return;
  settings = { ...DEFAULT_SETTINGS, ...s };
  B.storage.local.set({ settings });
}

function broadcastStatus() {
  B.runtime.sendMessage({ type: "status", status: status() }).catch(() => {});
}

function status() {
  return { connected: appReady, error: appReady ? null : lastError, settings, pausedHere, browser: BROWSER };
}

// ----- helpers ---------------------------------------------------------

function hostOf(url) {
  try {
    return new URL(url).hostname.toLowerCase();
  } catch {
    return "";
  }
}

function extOf(name) {
  const base = (name || "").split(/[\\/]/).pop().split("?")[0];
  const i = base.lastIndexOf(".");
  return i > 0 ? base.slice(i + 1).toLowerCase() : "";
}

function baseName(path) {
  return (path || "").split(/[\\/]/).pop() || null;
}

function siteExcluded(url) {
  const h = hostOf(url);
  return settings.excludedSites.some((s) => h === s || h.endsWith("." + s));
}

async function cookieHeader(url, storeId) {
  try {
    const query = { url };
    if (storeId) query.storeId = storeId;
    const cookies = await B.cookies.getAll(query);
    if (!cookies.length) return null;
    return cookies.map((c) => `${c.name}=${c.value}`).join("; ");
  } catch {
    return null;
  }
}

async function requestHeaders(url, storeId) {
  const headers = [{ name: "User-Agent", value: navigator.userAgent }];
  const cookie = await cookieHeader(url, storeId);
  if (cookie) headers.push({ name: "Cookie", value: cookie });
  return headers;
}

// ----- download interception ------------------------------------------

function shouldCatch(item) {
  if (!settings.enabled || pausedHere) return false;
  if (item.state && item.state !== "in_progress") return false;
  const url = item.finalUrl || item.url || "";
  if (!/^https?:/i.test(url)) return false; // blob:, data:, file: stay in the browser
  if (item.byExtensionId && item.byExtensionId === B.runtime.id) return false;
  if (item.incognito && !settings.includePrivate) return false;
  if (siteExcluded(url) || (item.referrer && siteExcluded(item.referrer))) return false;
  const ext = extOf(item.filename) || extOf(url);
  if (settings.skipWebPages && (/^(text\/html|application\/xhtml\+xml)(;|$)/i.test(item.mime || "") || ["html", "htm", "xhtml"].includes(ext))) return false;
  if (ext && settings.excludedExtensions.includes(ext)) return false;
  const size = item.totalBytes > 0 ? item.totalBytes : item.fileSize > 0 ? item.fileSize : 0;
  if (settings.minSize > 0 && size > 0 && size < settings.minSize) return false;
  return true;
}

async function giveBack(p) {
  if (p.given) return;
  p.given = true;
  if (p.suggest) {
    try {
      p.suggest();
    } catch {}
    p.suggest = null;
  }
  if (p.paused) {
    try {
      await B.downloads.resume(p.downloadId);
    } catch (e) {
      log("resume failed", e);
    }
  }
}

async function takeOver(p) {
  p.given = true;
  try {
    await B.downloads.cancel(p.downloadId);
  } catch {}
  try {
    await B.downloads.erase({ id: p.downloadId });
  } catch {}
  if (p.suggest) {
    try {
      p.suggest();
    } catch {}
    p.suggest = null;
  }
}

function onDecision(reqId, decision) {
  const p = pending.get(reqId);
  if (!p) return;
  pending.delete(reqId);
  clearTimeout(p.timer);
  if (decision === "takeOver" || decision === "cancel") takeOver(p);
  else giveBack(p);
}

function findPending(downloadId) {
  for (const [id, p] of pending) if (p.downloadId === downloadId) return [id, p];
  return [null, null];
}

async function sendCapture(reqId, item) {
  const p = pending.get(reqId);
  if (!p || p.sent) return;
  p.sent = true;
  const url = item.url;
  const finalUrl = item.finalUrl && item.finalUrl !== item.url ? item.finalUrl : null;
  const headers = await requestHeaders(finalUrl || url, item.cookieStoreId);
  const ok = send({
    type: "capture",
    id: reqId,
    browser: BROWSER,
    capture: {
      kind: "intercepted",
      browserDownloadId: item.id,
      url,
      finalUrl,
      referrer: item.referrer || null,
      pageUrl: item.referrer || null,
      pageTitle: null,
      fileName: baseName(item.filename),
      mime: item.mime || null,
      size: item.totalBytes > 0 ? item.totalBytes : item.fileSize > 0 ? item.fileSize : null,
      incognito: !!item.incognito,
      headers,
    },
  });
  if (!ok) {
    pending.delete(reqId);
    giveBack(p);
    return;
  }
  p.timer = setTimeout(() => {
    if (!p.received && pending.has(reqId)) {
      log("app did not answer; leaving the download to the browser");
      pending.delete(reqId);
      giveBack(p);
    }
  }, RECEIVE_TIMEOUT_MS);
}

B.downloads.onCreated.addListener((item) => {
  // Settings load asynchronously on a cold start of the background script.
  ready.then(() => {
    if (handled.has(item.id) || !shouldCatch(item)) return;
    const reqId = `dl-${item.id}-${nextId++}`;
    const p = { downloadId: item.id, received: false, timer: null, suggest: null, sent: false, paused: false, given: false, item };
    pending.set(reqId, p);
    handled.set(item.id, Date.now());

    // Chrome/Edge keep the network request open while paused, so pausing is
    // lossless. Firefox may restart or lose non-resumable downloads when
    // paused, so there the browser keeps downloading until we decide.
    if (!IS_FIREFOX) {
      B.downloads.pause(item.id).then(
        () => { p.paused = true; if (p.given) B.downloads.resume(item.id).catch(() => {}); },
        () => {},
      );
    }
    // Send right away: the app asks the server for the name and size itself,
    // so waiting for the browser's filename step would only add delay.
    sendCapture(reqId, item);
  });
});

if (B.downloads.onDeterminingFilename) {
  B.downloads.onDeterminingFilename.addListener((item, suggest) => {
    const [reqId, p] = findPending(item.id);
    if (!p || p.given) return false;
    // Hold the filename decision (this also defers any Save As dialog)
    // until the app decides.
    p.suggest = () => suggest();
    return true;
  });
}

B.downloads.onChanged.addListener((delta) => {
  if (!delta.state) return;
  const [reqId, p] = findPending(delta.id);
  if (!p) return;
  if (delta.state.current === "complete" || delta.state.current === "interrupted") {
    // Finished or cancelled in the browser before the app answered.
    pending.delete(reqId);
    clearTimeout(p.timer);
    send({ type: "captureGone", id: reqId });
  }
});

// Forget old handled ids.
setInterval(() => {
  const cutoff = Date.now() - 10 * 60 * 1000;
  for (const [id, t] of handled) if (t < cutoff) handled.delete(id);
}, 60 * 1000);

// ----- context menu and video button ----------------------------------

async function sendUrl(kind, url, pageUrl, pageTitle, storeId, incognito = false, mediaFormat = null) {
  await ready;
  if (!settings.enabled || pausedHere || incognito && !settings.includePrivate || siteExcluded(url) || pageUrl && siteExcluded(pageUrl)) return false;
  if (!/^https?:/i.test(url || "")) return false;
  const headers = await requestHeaders(url, storeId);
  const id = `${kind}-${nextId++}`;
  const received = new Promise((resolve) => {
    const timer = setTimeout(() => finish(false), RECEIVE_TIMEOUT_MS);
    const finish = (ok) => { clearTimeout(timer); manualReceipts.delete(id); resolve(ok); };
    manualReceipts.set(id, finish);
  });
  const sent = send({
      type: "capture",
      id,
      browser: BROWSER,
      capture: {
        kind,
        mediaFormat,
        browserDownloadId: null,
        url,
        finalUrl: null,
        referrer: pageUrl || null,
        pageUrl: pageUrl || null,
        pageTitle: pageTitle || null,
        fileName: null,
        mime: null,
        size: null,
        incognito,
        headers,
      },
  });
  if (!sent) manualReceipts.get(id)?.(false);
  return received;
}

async function queryVideoOptions(url, pageUrl, storeId, incognito) {
  if (!settings.enabled || pausedHere || incognito && !settings.includePrivate || siteExcluded(url) || pageUrl && siteExcluded(pageUrl)) return { ok: false, error: "Video capture is disabled for this page." };
  const headers = await requestHeaders(url, storeId);
  const id = `formats-${nextId++}`;
  const result = new Promise((resolve) => {
    const timer = setTimeout(() => finish({ ok: false, error: "Video lookup timed out. Open CrowDM and try again." }), 110000);
    const finish = (value) => { clearTimeout(timer); videoQueries.delete(id); resolve(value); };
    videoQueries.set(id, finish);
  });
  const sent = send({ type: "videoOptions", id, capture: { kind: "video", url, pageUrl, referrer: pageUrl, headers, incognito, mediaFormat: null,
    browserDownloadId: null, finalUrl: null, fileName: null, mime: null, size: null, pageTitle: null } });
  if (!sent) videoQueries.get(id)?.({ ok: false, error: "Could not connect to CrowDM." });
  return result;
}

// Keep only manifests, never individual DASH/HLS fragments. Frame scoping
// avoids selecting a stream from an advertisement or a different iframe.
const manifests = new Map();
B.webRequest.onHeadersReceived.addListener((details) => {
  if (details.tabId < 0 || !/^https?:/i.test(details.url)) return;
  const mime = details.responseHeaders?.find((h) => h.name.toLowerCase() === "content-type")?.value || "";
  if (!/\.(m3u8|mpd)([?#]|$)/i.test(details.url) && !/(mpegurl|dash\+xml)/i.test(mime)) return;
  const key = `${details.tabId}:${details.frameId}`;
  const list = (manifests.get(key) || []).filter((v) => Date.now() - v.at < 120000 && v.url !== details.url);
  list.push({ url: details.url, at: Date.now() });
  manifests.set(key, list.slice(-8));
}, { urls: ["http://*/*", "https://*/*"] }, ["responseHeaders"]);
function clearNavigation({ tabId, frameId }) {
  for (const key of manifests.keys()) if (frameId === 0 ? key.startsWith(`${tabId}:`) : key === `${tabId}:${frameId}`) manifests.delete(key);
}
B.webNavigation.onCommitted.addListener(clearNavigation);
B.webNavigation.onHistoryStateUpdated?.addListener(clearNavigation);
B.tabs.onRemoved.addListener((tabId) => {
  for (const key of manifests.keys()) if (key.startsWith(`${tabId}:`)) manifests.delete(key);
});

function mediaUrl(url, pageUrl, tabId, frameId = 0) {
  // A YouTube CDN URL may contain only audio or only video; the extractor
  // needs the watch page to choose and merge both tracks.
  if (/(^|\.)(youtube\.com|youtu\.be)$/.test(hostOf(pageUrl))) {
    const page = new URL(pageUrl);
    const id = page.searchParams.get("v") || (/^\/(shorts|embed|live)\//.test(page.pathname) ? page.pathname.split("/")[2] : page.hostname === "youtu.be" ? page.pathname.slice(1) : null);
    return id ? `https://www.youtube.com/watch?v=${encodeURIComponent(id)}` : pageUrl;
  }
  if (/^https?:/i.test(url || "")) return url;
  const list = (manifests.get(`${tabId}:${frameId}`) || []).filter((v) => Date.now() - v.at < 120000);
  return list.length === 1 ? list[0].url : pageUrl;
}

function selectedUrls(text) {
  if(typeof text!=="string"||text.length>512*1024)return [];
  const seen=new Set(),urls=[];
  for(const match of text.matchAll(/(?:https?:\/\/|magnet:\?)[^\s<>"']+/gi)){
    const raw=match[0].replace(/&amp;/g,"&");
    try{const u=new URL(raw);if(!["http:","https:","magnet:"].includes(u.protocol)||u.username||u.password)continue;
      if(u.protocol!=="magnet:")u.hash="";const key=u.href;
      if(!seen.has(key)){seen.add(key);urls.push(raw);}if(urls.length>1000)return [];
    }catch{/* Ignore malformed selected text. */}
  }return urls;
}
async function sendBatchLinks(text,pageUrl,incognito){
  await ready;
  if(!settings.enabled||pausedHere||incognito&&!settings.includePrivate||siteExcluded(pageUrl))return {ok:false,error:"Link collection is disabled for this page."};
  const urls=selectedUrls(text);
  if(!urls.length)return {ok:false,error:"Select up to 1,000 HTTP, HTTPS or magnet links."};
  const id=`batch-${nextId++}`;
  const result=new Promise(resolve=>{const timer=setTimeout(()=>finish({ok:false,error:"CrowDM did not respond. Open the app and try again."}),RECEIVE_TIMEOUT_MS);
    const finish=value=>{clearTimeout(timer);batchReceipts.delete(id);resolve(value);};batchReceipts.set(id,finish);});
  if(!send({type:"batchLinks",id,text:urls.join("\n"),incognito}))batchReceipts.get(id)?.({ok:false,error:"Could not connect to CrowDM."});
  return result;
}

function createMenus() {
  B.contextMenus.removeAll(() => {
    B.contextMenus.create({ id: "crowdm-link", title: "Download with CrowDM", contexts: ["link"] });
    B.contextMenus.create({ id: "crowdm-selection", title: "Collect selected links with CrowDM", contexts: ["selection"] });
    B.contextMenus.create({ id: "crowdm-media", title: "Download with CrowDM", contexts: ["video", "audio", "image"] });
  });
}

B.runtime.onInstalled.addListener(createMenus);
B.runtime.onStartup?.addListener(createMenus);

B.contextMenus.onClicked.addListener((info, tab) => {
  if(info.menuItemId==="crowdm-selection"){
    sendBatchLinks(info.selectionText,info.frameUrl||info.pageUrl||tab?.url,!!tab?.incognito).then(r=>{if(!r.ok)notifyTab(tab,r.error||"CrowDM could not collect these links.");});return;
  }
  const url = info.menuItemId === "crowdm-link" ? info.linkUrl : info.srcUrl;
  const pageUrl = info.frameUrl || info.pageUrl || tab?.url;
  const video = info.mediaType === "video" || /\.(m3u8|mpd)([?#]|$)/i.test(url || "") || /(^|\.)(youtube\.com|youtu\.be)$/.test(hostOf(url));
  const target = video ? mediaUrl(url, pageUrl, tab?.id, info.frameId) : url;
  sendUrl(video ? "video" : "menu", target, pageUrl, tab?.title, tab?.cookieStoreId, !!tab?.incognito)
    .then((ok) => { if (!ok) notifyTab(tab, "CrowDM could not receive this download. Check the extension connection and settings."); });
});

function notifyTab(tab, text) {
  if (tab?.id != null) B.tabs.sendMessage(tab.id, { type: "notice", text }).catch(() => {});
}

B.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  switch (msg?.type) {
    case "batchLinks":
      ready.then(async()=>{
        const page=sender.url||sender.tab?.url;
        if(!page||!sender.tab)return sendResponse({ok:false,error:"Select links on a web page first."});
        if(siteExcluded(sender.tab.url||""))return sendResponse({ok:false,error:"Link collection is disabled for this page."});
        sendResponse(await sendBatchLinks(msg.text,page,!!sender.tab.incognito));
      }).catch(()=>sendResponse({ok:false,error:"Could not collect these links."}));
      return true;
    case "video":
    case "videoOptions":
      ready.then(async () => {
        // sender.url can describe the document's initial URL after an SPA
        // navigation. The content script supplies the current location.
        const original = sender.url || sender.tab?.url;
        let pageUrl = original;
        try { if (new URL(msg.pageUrl).origin === new URL(original).origin) pageUrl = msg.pageUrl; } catch { /* Invalid page stays scoped to its sender. */ }
        if ((sender.frameId ?? 0) === 0 && B.tabs.get && sender.tab?.id != null) {
          const tab = await B.tabs.get(sender.tab.id);
          if (tab.url && mediaUrl(null, tab.url) !== mediaUrl(null, pageUrl)) return sendResponse({ ok: false, error: "The video changed. Open the download menu again." });
        }
        if (!settings.videoButton) return sendResponse({ ok: false });
        const url = mediaUrl(msg.url, pageUrl, sender.tab?.id, sender.frameId);
        if (msg.type === "videoOptions") {
          sendResponse(await queryVideoOptions(url, pageUrl, sender.tab?.cookieStoreId, !!sender.tab?.incognito));
          return;
        }
        const ok = await sendUrl("video", url, pageUrl, msg.title || sender.tab?.title, sender.tab?.cookieStoreId, !!sender.tab?.incognito, msg.mediaFormat);
        sendResponse({ ok });
      }).catch(() => sendResponse({ ok: false }));
      return true;
    case "getStatus":
      ready.then(() => sendResponse(status()));
      return true;
    case "setPausedHere":
      pausedHere = !!msg.paused;
      B.storage.local.set({ pausedHere });
      sendResponse(status());
      break;
    case "connect":
      connect();
      sendResponse(status());
      break;
    case "openApp":
      send({ type: "activate", args: [] });
      sendResponse({ ok: true });
      break;
  }
  return false;
});
