// CrowDM "Download video" button.
//
// Shows a small button over a web video player when the pointer is over it
// Direct files, manifests and watch pages are handed to the app's video
// extractor, which downloads and merges non-DRM video and audio tracks.
//
// The button lives in a closed shadow root in a fixed-position host, so it
// never changes the page's own DOM layout or styles.

(() => {
  if (window.__crowdmVideo) return;
  window.__crowdmVideo = true;
  const B = globalThis.browser ?? globalThis.chrome;

  let enabled = true;
  let pausedHere = false;
  let settings = {};
  const apply = () => { enabled = settings.enabled !== false && settings.videoButton !== false && !pausedHere
    && (!B.extension?.inIncognitoContext || settings.includePrivate === true)
    && !settings.excludedSites?.some((s) => location.hostname === s || location.hostname.endsWith(`.${s}`)); if (host) { host.dataset.theme = settings.theme || "system"; globalThis.CrowDMTheme?.apply(host, settings); } if (!enabled) hide(); };
  B.storage.local.get(["settings", "pausedHere"]).then((r) => {
    settings = r.settings || {}; pausedHere = !!r.pausedHere; apply();
  });
  B.storage.onChanged.addListener((changes) => {
    if (changes.settings) settings = changes.settings.newValue || {};
    if (changes.pausedHere) pausedHere = !!changes.pausedHere.newValue;
    apply();
  });

  const MIN_W = 200;
  const MIN_H = 110;

  let host = null;
  let button = null;
  let current = null;
  let hideTimer = 0;
  let raf = 0;
  let lastPoint = null;
  let menu = null;
  let menuOpen = false;
  let menuGeneration = 0;
  let pageIdentity = location.href;
  function changedPage() {
    if (pageIdentity === location.href) return;
    pageIdentity = location.href;
    hide();
  }
  // YouTube reuses the same document and <video> between playlist items.
  for (const event of ["yt-navigate-start", "yt-navigate-finish", "yt-page-data-updated", "popstate", "hashchange"]) {
    addEventListener(event, () => { changedPage(); closeMenu(); }, true);
  }
  setInterval(changedPage, 250);
  document.addEventListener("loadstart", (e) => { if (e.target === current) closeMenu(); }, true);
  document.addEventListener("emptied", (e) => { if (e.target === current) hide(); }, true);

  function videoSource(v) {
    const candidates = [v.currentSrc, v.src, ...Array.from(v.querySelectorAll("source")).map((s) => s.src)];
    return candidates.find((u) => typeof u === "string" && /^https?:/i.test(u)) || null;
  }

  function build() {
    host = document.createElement("crowdm-video-button");
    host.dataset.theme = settings.theme || "system";
    globalThis.CrowDMTheme?.apply(host, settings);
    host.style.cssText = "all: initial; position: fixed; z-index: 2147483647; top: 0; left: 0; display: none;";
    const root = host.attachShadow({ mode: "closed" });
    root.innerHTML = `
      <style>
        :host { --panel: #161a20; --ink: #f3f5f7; --muted: #b6c0cb; --edge: #39424a; --hover: #263e40; --accent: #38c1b0; }
        :host([data-theme="nightwing"]) { --panel: #151d29; --ink: #e3eaf3; --muted: #abb9cc; --edge: #3a4b61; --hover: #25384f; --accent: #94bde8; }
        :host([data-theme="ocean"]) { --panel: #162731; --ink: #e0ecef; --muted: #aac1cb; --edge: #3a5663; --hover: #28424e; --accent: #77c7cd; }
        :host([data-theme="forest"]) { --panel: #1b2921; --ink: #e5ece4; --muted: #b2c2b3; --edge: #435b4b; --hover: #304438; --accent: #a7c99b; }
        :host([data-theme="light"]) { --panel: #fff; --ink: #15181d; --muted: #545c67; --edge: #cdd2d9; --hover: #dfedea; --accent: #0e8177; }
        :host([data-theme="sand"]) { --panel: #fcf8f1; --ink: #342c25; --muted: #685c4e; --edge: #cbbca5; --hover: #eadfcd; --accent: #956125; }
        .wrap { display: flex; flex-direction: column; align-items: flex-end; gap: 6px; }
        button {
          all: initial;
          display: inline-flex; align-items: center; justify-content: center;
          height: 34px; padding: 0 9px; border-radius: 8px;
          background: var(--panel); color: var(--ink);
          font: 600 13px/1 "Segoe UI", system-ui, sans-serif;
          box-shadow: 0 2px 10px rgba(0,0,0,0.35);
          cursor: pointer; backdrop-filter: blur(6px);
          transition: background .15s ease, opacity .18s ease;
        }
        .trigger { opacity: .3; }
        .trigger:hover, .trigger:focus-visible, .trigger.is-open { opacity: 1; background: var(--hover); }
        .trigger .label { max-width: 0; opacity: 0; overflow: hidden; white-space: nowrap; transition: max-width .18s ease, opacity .18s ease, margin .18s ease; }
        .trigger:hover .label, .trigger:focus-visible .label, .trigger.is-open .label { max-width: 190px; opacity: 1; margin-left: 7px; }
        button:active { transform: translateY(1px); }
        button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
        svg { width: 20px; height: 20px; flex: none; }
        .menu { width: 252px; padding: 7px; border: 1px solid var(--edge); border-radius: 9px; background: var(--panel); color: var(--ink); font: 13px/1.4 'Segoe UI',system-ui,sans-serif; box-shadow: 0 6px 24px #0006; overflow-y: auto; }
        .menu[hidden] { display: none; }
        .menu p { margin: 5px 7px 8px; font-size: 12px; color: var(--muted); overflow-wrap: anywhere; }
        .menu button { display: flex; width: 100%; box-sizing: border-box; justify-content: space-between; font-weight: 500; box-shadow: none; background: transparent; border-radius: 5px; }
        .menu button:hover, .menu button:focus-visible { background: var(--hover); }
        .menu button small { font: 11px 'Segoe UI',system-ui,sans-serif; color: var(--muted); margin-left: 8px; }
        @media (prefers-reduced-motion: reduce) { :host(:not([data-reduced-motion="ignore"])) * { transition: none !important; } }
        :host([data-motion="off"]) * { transition: none !important; }
      </style>
      <div class="wrap">
      <button class="trigger" type="button" title="Download this video with CrowDM" aria-label="Download video" aria-haspopup="menu" aria-expanded="false">
        <svg viewBox="0 0 256 256" aria-hidden="true"><g fill="currentColor"><path d="M28 68 100 104V124L48 98ZM42 107 100 136V156L56 134Z"/><path d="M28 68 100 104V124L48 98ZM42 107 100 136V156L56 134Z" transform="translate(256 0) scale(-1 1)"/><path fill-rule="evenodd" d="M112 96V72L102 78 108 59C112 40 124 30 140 30H153L147 38 162 45C179 47 188 57 186 76L177 68 165 65H156L144 80V96ZM135 47 155 53 142 59Z"/><path fill="var(--accent)" d="M112 104H144V164H168L128 204 88 164H112Z"/><path d="M76 204H88V216H168V204H180V228H76Z"/></g></svg>
        <span class="label">Download video</span>
      </button>
      <div class="menu" role="menu" aria-label="Available video downloads" hidden></div>
      </div>`;
    button = root.querySelector(".trigger");
    menu = root.querySelector(".menu");
    const label = root.querySelector(".label");
    button.addEventListener("click", async (e) => {
      e.preventDefault();
      e.stopPropagation();
      if (!current) return;
      if (menuOpen) { closeMenu(); return; }
      menuOpen = true;
      const generation = ++menuGeneration;
      button.classList.add("is-open");
      button.setAttribute("aria-expanded", "true");
      menu.hidden = false;
      menu.replaceChildren();
      const message = document.createElement("p");
      message.textContent = "Finding available video qualities…";
      message.setAttribute("role", "status");
      menu.appendChild(message);
      place();
      const url = videoSource(current);
      const pageUrl = location.href;
      const source = current;
      try {
        const result = await B.runtime.sendMessage({ type: "videoOptions", url, pageUrl, title: document.title });
        if (generation !== menuGeneration || !menuOpen || pageUrl !== location.href || source !== current) { closeMenu(); return; }
        if (!result?.ok) { message.textContent = result?.error || "Open CrowDM and try again."; return; }
        message.textContent = "Choose video or audio";
        for (const option of result.formats || []) {
          const choice = document.createElement("button");
          choice.type = "button";
          choice.setAttribute("role", "menuitem");
          const name = document.createElement("span"); name.textContent = option.label;
          const format = document.createElement("small"); format.textContent = (option.container || "mkv").toUpperCase();
          choice.append(name, format);
          choice.addEventListener("click", async (event) => {
            event.preventDefault(); event.stopPropagation();
            if (pageUrl !== location.href || source !== current || videoSource(current) !== url) { hide(); return; }
            for (const item of menu.querySelectorAll("button")) item.disabled = true;
            message.textContent = "Sending to CrowDM…";
            try {
              const sent = await B.runtime.sendMessage({ type: "video", url: videoSource(current), pageUrl, title: document.title, mediaFormat: option.format });
              if (generation !== menuGeneration) return;
              if (sent?.ok) { closeMenu(); label.textContent = "Sent to CrowDM"; setTimeout(() => { label.textContent = "Download video"; }, 1800); }
              else { message.textContent = "Could not send to CrowDM. Open the app and try again."; }
            } catch { message.textContent = "Could not connect to CrowDM."; }
            for (const item of menu.querySelectorAll("button")) item.disabled = false;
          });
          menu.appendChild(choice);
        }
        if (!result.formats?.length) message.textContent = "No downloadable video formats were found.";
        place();
      } catch { if (generation === menuGeneration) message.textContent = "Could not connect to CrowDM."; }
    });
    host.addEventListener("pointerenter", () => clearTimeout(hideTimer));
    host.addEventListener("pointerleave", () => scheduleHide());
    new ResizeObserver(place).observe(host);
  }

  function closeMenu() {
    menuOpen = false;
    menuGeneration++;
    if (menu) menu.hidden = true;
    button?.classList.remove("is-open");
    button?.setAttribute("aria-expanded", "false");
    place();
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
    if (menu) menu.style.maxHeight = `${Math.max(80, Math.min(320, innerHeight - top - 52))}px`;
  }

  function show(v) {
    if (!host) build();
    clearTimeout(hideTimer);
    current = v;
    place();
  }

  function hide() {
    current = null;
    closeMenu();
    if (host) host.style.display = "none";
  }

  function scheduleHide() {
    if (menuOpen) return;
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
    if (!enabled || !lastPoint || menuOpen) return;
    const v = videoAt(lastPoint.x, lastPoint.y);
    if (v && (videoSource(v) || v.currentSrc || v.src || v.srcObject)) {
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
  document.addEventListener("pointerdown", (e) => { if (menuOpen && !e.composedPath().includes(host)) closeMenu(); }, true);
  document.addEventListener("keydown", (e) => { if (menuOpen && e.key === "Escape") { closeMenu(); button?.focus(); } }, true);

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
