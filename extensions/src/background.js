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
// Chrome: how long to wait for onDeterminingFilename before sending anyway.
const FILENAME_WAIT_MS = 1500;

const DEFAULT_SETTINGS = {
  enabled: true,
  excludedSites: [],
  excludedExtensions: [],
  minSize: 0,
  includePrivate: false,
  videoButton: true,
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
        () => (p.paused = true),
        () => {},
      );
      // Wait briefly for onDeterminingFilename, which carries the real name.
      setTimeout(() => sendCapture(reqId, p.item), FILENAME_WAIT_MS);
    } else {
      sendCapture(reqId, item);
    }
  });
});

if (B.downloads.onDeterminingFilename) {
  B.downloads.onDeterminingFilename.addListener((item, suggest) => {
    const [reqId, p] = findPending(item.id);
    if (!p || p.given) return false;
    // Hold the filename decision (this also defers any Save As dialog)
    // until the app decides.
    p.suggest = () => suggest();
    p.item = { ...p.item, ...item };
    sendCapture(reqId, p.item);
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

function sendUrl(kind, url, pageUrl, pageTitle, storeId) {
  if (!/^https?:/i.test(url || "")) return;
  requestHeaders(url, storeId).then((headers) => {
    send({
      type: "capture",
      id: `${kind}-${nextId++}`,
      browser: BROWSER,
      capture: {
        kind,
        browserDownloadId: null,
        url,
        finalUrl: null,
        referrer: pageUrl || null,
        pageUrl: pageUrl || null,
        pageTitle: pageTitle || null,
        fileName: null,
        mime: null,
        size: null,
        incognito: false,
        headers,
      },
    });
  });
}

function createMenus() {
  B.contextMenus.removeAll(() => {
    B.contextMenus.create({ id: "crowdm-link", title: "Download with CrowDM", contexts: ["link"] });
    B.contextMenus.create({ id: "crowdm-media", title: "Download with CrowDM", contexts: ["video", "audio", "image"] });
  });
}

B.runtime.onInstalled.addListener(createMenus);
B.runtime.onStartup?.addListener(createMenus);

B.contextMenus.onClicked.addListener((info, tab) => {
  const url = info.menuItemId === "crowdm-link" ? info.linkUrl : info.srcUrl;
  if (url && url.startsWith("blob:")) {
    notifyTab(tab, "This media is streamed in pieces and cannot be downloaded as one file.");
    return;
  }
  sendUrl("menu", url, info.pageUrl || tab?.url, tab?.title, tab?.cookieStoreId);
});

function notifyTab(tab, text) {
  if (tab?.id != null) B.tabs.sendMessage(tab.id, { type: "notice", text }).catch(() => {});
}

B.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  switch (msg?.type) {
    case "video":
      sendUrl("video", msg.url, msg.pageUrl || sender.tab?.url, msg.title || sender.tab?.title, sender.tab?.cookieStoreId);
      sendResponse({ ok: true });
      break;
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
