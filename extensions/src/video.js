// CrowDM "Download video" button.
//
// Shows a small button over a web video player when the pointer is over it
// and the video is a plain file (an http(s) source). Streams assembled by
// the page (blob: / MediaSource URLs, e.g. most streaming services) get no
// button: they are not a single downloadable file.
//
// The button lives in a closed shadow root in a fixed-position host, so it
// never changes the page's own DOM layout or styles.

(() => {
  if (window.__crowdmVideo) return;
  window.__crowdmVideo = true;
  const B = globalThis.browser ?? globalThis.chrome;

  let enabled = true;
  B.storage.local.get("settings").then((r) => {
    if (r && r.settings && r.settings.videoButton === false) enabled = false;
  });
  B.storage.onChanged.addListener((changes) => {
    if (changes.settings) enabled = changes.settings.newValue?.videoButton !== false;
    if (!enabled) hide();
  });

  const MIN_W = 200;
  const MIN_H = 110;

  let host = null;
  let button = null;
  let current = null;
  let hideTimer = 0;
  let raf = 0;
  let lastPoint = null;

  function videoSource(v) {
    const candidates = [v.currentSrc, v.src, ...Array.from(v.querySelectorAll("source")).map((s) => s.src)];
    return candidates.find((u) => typeof u === "string" && /^https?:/i.test(u)) || null;
  }

  function build() {
    host = document.createElement("crowdm-video-button");
    host.style.cssText = "all: initial; position: fixed; z-index: 2147483647; top: 0; left: 0; display: none;";
    const root = host.attachShadow({ mode: "closed" });
    root.innerHTML = `
      <style>
        button {
          all: initial;
          display: inline-flex; align-items: center; gap: 7px;
          height: 32px; padding: 0 13px 0 10px; border-radius: 8px;
          background: rgba(20, 23, 28, 0.88); color: #f3f5f7;
          font: 600 13px/1 "Segoe UI", system-ui, sans-serif;
          box-shadow: 0 2px 10px rgba(0,0,0,0.35);
          cursor: pointer; backdrop-filter: blur(6px);
          transition: background .12s ease, transform .06s ease;
        }
        button:hover { background: rgba(14, 140, 129, 0.95); }
        button:active { transform: translateY(1px); }
        button:focus-visible { outline: 2px solid #38c1b0; outline-offset: 2px; }
        svg { width: 16px; height: 16px; flex: none; }
      </style>
      <button type="button" title="Download this video with CrowDM">
        <svg viewBox="0 0 256 256" aria-hidden="true"><path fill="currentColor" d="M224 144v64a16 16 0 0 1-16 16H48a16 16 0 0 1-16-16v-64a8 8 0 0 1 16 0v64h160v-64a8 8 0 0 1 16 0Zm-101.66 5.66a8 8 0 0 0 11.32 0l40-40a8 8 0 0 0-11.32-11.32L136 124.69V32a8 8 0 0 0-16 0v92.69L93.66 98.34a8 8 0 0 0-11.32 11.32Z"/></svg>
        <span>Download video</span>
      </button>`;
    button = root.querySelector("button");
    const label = root.querySelector("span");
    button.addEventListener("click", (e) => {
      e.preventDefault();
      e.stopPropagation();
      if (!current) return;
      const url = videoSource(current);
      if (!url) return;
      B.runtime.sendMessage({ type: "video", url, pageUrl: location.href, title: document.title }).catch(() => {});
      label.textContent = "Sent to CrowDM";
      setTimeout(() => (label.textContent = "Download video"), 1800);
    });
    host.addEventListener("pointerenter", () => clearTimeout(hideTimer));
    host.addEventListener("pointerleave", () => scheduleHide());
  }

  function place() {
    if (!current || !host) return;
    const r = current.getBoundingClientRect();
    if (r.width < MIN_W || r.height < MIN_H || r.bottom < 0 || r.top > innerHeight) {
      hide();
      return;
    }
    const parent = document.fullscreenElement || document.documentElement;
    if (host.parentNode !== parent) parent.appendChild(host);
    const top = Math.max(8, r.top + 10);
    const right = Math.min(innerWidth - 8, r.right - 10);
    host.style.display = "block";
    const w = host.getBoundingClientRect().width || 150;
    host.style.transform = `translate(${Math.round(right - w)}px, ${Math.round(top)}px)`;
  }

  function show(v) {
    if (!host) build();
    clearTimeout(hideTimer);
    current = v;
    place();
  }

  function hide() {
    if (host) host.style.display = "none";
    current = null;
  }

  function scheduleHide() {
    clearTimeout(hideTimer);
    hideTimer = setTimeout(hide, 1200);
  }

  function videoAt(x, y) {
    for (const el of document.elementsFromPoint(x, y)) {
      if (el.tagName === "VIDEO") return el;
      if (host && el === host) return current;
    }
    // Custom players often cover the <video> with their own controls.
    for (const v of document.querySelectorAll("video")) {
      const r = v.getBoundingClientRect();
      if (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) return v;
    }
    return null;
  }

  function check() {
    raf = 0;
    if (!enabled || !lastPoint) return;
    const v = videoAt(lastPoint.x, lastPoint.y);
    if (v && videoSource(v)) {
      const r = v.getBoundingClientRect();
      if (r.width >= MIN_W && r.height >= MIN_H) {
        show(v);
        return;
      }
    }
    if (current) scheduleHide();
  }

  document.addEventListener(
    "pointermove",
    (e) => {
      lastPoint = { x: e.clientX, y: e.clientY };
      if (!raf) raf = requestAnimationFrame(check);
    },
    { passive: true, capture: true },
  );
  addEventListener("scroll", () => current && place(), { passive: true, capture: true });
  addEventListener("resize", () => current && place(), { passive: true });
  document.addEventListener("fullscreenchange", () => current && place());

  // Short notices from the background script (e.g. unsupported streams).
  B.runtime.onMessage.addListener((msg) => {
    if (msg?.type !== "notice" || window.top !== window) return;
    const n = document.createElement("crowdm-notice");
    n.style.cssText =
      "all: initial; position: fixed; z-index: 2147483647; left: 50%; bottom: 28px; transform: translateX(-50%); max-width: 420px; padding: 10px 14px; border-radius: 8px; background: rgba(20,23,28,.92); color: #f3f5f7; font: 13px/1.4 'Segoe UI', system-ui, sans-serif; box-shadow: 0 2px 12px rgba(0,0,0,.35);";
    n.textContent = `CrowDM: ${msg.text}`;
    document.documentElement.appendChild(n);
    setTimeout(() => n.remove(), 4500);
  });
})();
