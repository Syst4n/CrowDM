import { useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { CaretDown, CaretRight, Folder, Globe } from "@phosphor-icons/react";
import { api, errorText, onCaptures, type CaptureView, type Settings } from "../lib/api";
import { useTheme } from "../lib/theme";
import * as fmt from "../lib/format";
import { Logo } from "./Logo";
import { Select } from "./Select";

const BROWSER: Record<string, string> = { chrome: "Chrome", edge: "Edge", firefox: "Firefox", chromium: "Chromium", brave: "Brave" };

export function CaptureDialog() {
  const [captures, setCaptures] = useState<CaptureView[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  useTheme(settings);

  useEffect(() => {
    let disposed = false;
    let revision = 0;
    let un: (() => void) | undefined;
    let unFocus: (() => void) | undefined;
    const refresh = async () => {
      const before = revision;
      try {
        const [list, nextSettings] = await Promise.all([api.captureList(), api.getSettings()]);
        if (disposed) return;
        if (before === revision) setCaptures(list);
        setSettings(nextSettings);
        setLoadError(null);
      } catch (e) { if (!disposed) setLoadError(errorText(e)); }
    };
    // Subscribe before reading the snapshot; do not overwrite a newer event
    // with an older command response. Poll also recovers missed IPC events.
    onCaptures((list) => { revision++; if (!disposed) setCaptures(list); }).then((f) => {
      if (disposed) { f(); return; }
      un = f;
      void refresh();
    }).catch((e) => { if (!disposed) setLoadError(errorText(e)); });
    getCurrentWindow().onFocusChanged(({ payload }) => { if (payload) void refresh(); }).then((f) => { if (disposed) f(); else unFocus = f; });
    const timer = window.setInterval(refresh, 1500);
    return () => {
      disposed = true;
      clearInterval(timer);
      un?.();
      unFocus?.();
    };
  }, []);

  const current = captures[0];
  const hasCurrent = !!current && !!settings;
  useEffect(() => {
    // A freshly created dialog window starts hidden; show it once painted.
    if (!hasCurrent) return;
    requestAnimationFrame(async () => {
      const w = getCurrentWindow();
      if (!(await w.isVisible())) {
        await w.show();
        await w.setFocus();
      }
    });
  }, [hasCurrent]);
  if (!current || !settings) {
    return <div className="capture capture-loading" role="status">
      <strong>{loadError ? "Could not load download details" : "Waiting for download details…"}</strong>
      {loadError && <p>{loadError}</p>}
      <button className="btn" onClick={() => getCurrentWindow().hide()}>Close</button>
    </div>;
  }
  return <CaptureForm key={current.key} c={current} settings={settings} waiting={captures.length - 1} />;
}

function CaptureForm({ c, settings, waiting }: { c: CaptureView; settings: Settings; waiting: number }) {
  const p = c.probe;
  const [url, setUrl] = useState(c.url);
  const [name, setName] = useState(p?.fileName ?? c.fileName ?? "");
  const [nameTouched, setNameTouched] = useState(false);
  const [category, setCategory] = useState(p?.category ?? "other");
  const [dir, setDir] = useState(p?.dir ?? "");
  const [dirTouched, setDirTouched] = useState(false);
  const [advanced, setAdvanced] = useState(false);
  const [connections, setConnections] = useState(settings.maxConnectionsPerDownload);
  const [limit, setLimit] = useState("");
  const [checksum, setChecksum] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const startRef = useRef<HTMLButtonElement>(null);
  const formRef = useRef<HTMLDivElement>(null);
  const fieldsRef = useRef<HTMLDivElement>(null);
  const needsUrl = c.kind === "manual" && !c.url;

  useEffect(() => {
    const root = formRef.current;
    const fields = fieldsRef.current;
    if (!root || !fields) return;
    let frame = 0;
    const resize = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const head = root.querySelector(".capture-head")!.getBoundingClientRect().height;
        const foot = root.querySelector(".capture-foot")!.getBoundingClientRect().height;
        const height = Math.min(Math.max(280, Math.min(680, screen.availHeight - 70)), Math.max(280, Math.ceil(head + fields.scrollHeight + foot + 24)));
        getCurrentWindow().setSize(new LogicalSize(600, height)).catch(() => {});
      });
    };
    const observer = new ResizeObserver(resize);
    observer.observe(fields);
    observer.observe(root.querySelector(".capture-head")!);
    resize();
    return () => { observer.disconnect(); cancelAnimationFrame(frame); };
  }, []);

  // Adopt server-provided details as they arrive, unless the user edited them.
  useEffect(() => {
    if (!p) return;
    if (!nameTouched) setName(p.fileName);
    setCategory((cur) => (cur === "other" || !dirTouched ? p.category : cur));
    if (!dirTouched) setDir(p.dir);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [p?.fileName, p?.dir, p?.category]);

  useEffect(() => {
    if (!needsUrl) startRef.current?.focus();
  }, [needsUrl]);

  // Report when this capture is actually on screen (latency diagnostics).
  useEffect(() => {
    requestAnimationFrame(() => requestAnimationFrame(() => api.captureRendered(c.key).catch(() => {})));
  }, [c.key]);

  const onCategory = async (id: string) => {
    setCategory(id);
    if (!dirTouched) setDir(await api.categoryDir(id));
  };

  const browse = async () => {
    const picked = await openDialog({ directory: true, defaultPath: dir || settings.downloadRoot, title: "Choose a folder" });
    if (typeof picked === "string") {
      setDir(picked);
      setDirTouched(true);
    }
  };

  const accept = async (start: boolean) => {
    setError(null);
    const rate = fmt.parseRate(limit);
    if (rate == null) {
      setError("Enter the speed limit like 500 KB or 2 MB, or leave it empty.");
      return;
    }
    setBusy(true);
    try {
      await api.captureAccept(c.key, {
        fileName: name.trim() || undefined,
        dir: dir.trim() || undefined,
        category,
        start,
        maxConnections: connections !== settings.maxConnectionsPerDownload ? connections : undefined,
        speedLimit: rate || undefined,
        checksum: checksum.trim() || undefined,
      });
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  };

  const checkUrl = () => {
    if (!/^https?:\/\//i.test(url.trim())) {
      setError("Enter an address that starts with http:// or https://");
      return;
    }
    setError(null);
    api.captureSetUrl(c.key, url.trim()).catch((e) => setError(errorText(e)));
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        if (c.browserWaiting) api.captureToBrowser(c.key);
        else api.captureDismiss(c.key);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [c.key, c.browserWaiting]);

  // When CrowDM cannot get the real file, the browser's option leads.
  const preferBrowser = !!c.browserRecommended && c.browserWaiting;
  const size = p?.size ?? c.size;
  const source = fmt.host(c.pageUrl ?? c.finalUrl ?? c.url);
  const browserName = c.browser ? BROWSER[c.browser] ?? c.browser : null;
  const title = name || (needsUrl ? "Add a download" : "Checking the file");
  const categories = settings.categories;

  const notices = useMemo(() => {
    const out: { tone: "warn" | "info"; text: string }[] = [];
    if (c.browserRecommended) out.push({ tone: "warn", text: c.browserRecommended });
    else if (c.probeError) out.push({ tone: "warn", text: c.probeError });
    if (p?.duplicateOf) out.push({ tone: "info", text: "You've downloaded this address before. Starting again creates a new copy." });
    else if (p?.exists) out.push({ tone: "info", text: "A file with this name is already in the folder. CrowDM will add a number to the new one." });
    for (const w of p?.warnings ?? []) out.push({ tone: "info", text: w });
    return out;
  }, [c.browserRecommended, c.probeError, p]);

  return (
    <div className="capture" ref={formRef}>
      <div className="capture-head">
        <div className="big-icon">
          <Logo size={34} />
        </div>
        <div style={{ minWidth: 0 }}>
          <div className="title selectable">{title}</div>
          <div className="facts">
            <span className="num">{c.probing && size == null ? <span className="skeleton" /> : size != null ? `${p?.sizeEstimated ? "About " : ""}${fmt.bytes(size)}` : needsUrl ? "" : "Size unknown"}</span>
            {source && (
              <span style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
                <Globe size={13} />
                {source}
              </span>
            )}
            {c.browserWaiting && browserName && <span>{c.browser === "firefox" ? "Firefox keeps downloading until you choose" : `Paused in ${browserName}`}</span>}
          </div>
        </div>
      </div>

      <div className="capture-body">
        <div className="capture-fields" ref={fieldsRef}>
        {(c.kind === "manual" || c.probeError) && (
          <div className="field">
            <label htmlFor="url">Address</label>
            <div className="input-row">
              <input
                id="url"
                className="input selectable"
                value={url}
                placeholder="https://example.com/file.zip"
                onChange={(e) => setUrl(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && checkUrl()}
                autoFocus={needsUrl}
                spellCheck={false}
              />
              {(needsUrl || url.trim() !== c.url) && (
                <button className="btn" onClick={checkUrl}>
                  Check
                </button>
              )}
            </div>
          </div>
        )}

        {!needsUrl && (
          <>
            {c.kind === "video" && !!p?.mediaOptions?.length && <div className="field">
              <label htmlFor="quality">Download format and quality</label>
              <Select id="quality" className="select" value={c.mediaFormat ?? "bv*+ba/b"}
                onValueChange={(value) => { setNameTouched(false); setDirTouched(false); api.captureSetMediaFormat(c.key, value).catch((e) => setError(errorText(e))); }}>
                {p.mediaOptions.map((option) => <option key={option.format} value={option.format}>{option.label} · {(option.container || "mkv").toUpperCase()}{option.size ? ` · About ${fmt.bytes(option.size)}` : ""}</option>)}
              </Select>
            </div>}
            <div className="field">
              <label htmlFor="name">File name</label>
              <input
                id="name"
                className="input"
                value={name}
                onChange={(e) => (setName(e.target.value), setNameTouched(true))}
                spellCheck={false}
              />
            </div>
            <div className="field">
              <label htmlFor="dir">Save to</label>
              <div className="input-row">
                <Select className="select" value={category} onValueChange={onCategory} aria-label="Category" style={{ width: 140 }}>
                  {categories.map((cat) => (
                    <option key={cat.id} value={cat.id}>
                      {cat.name}
                    </option>
                  ))}
                </Select>
                <input id="dir" className="input" value={dir} onChange={(e) => (setDir(e.target.value), setDirTouched(true))} spellCheck={false} />
                <button className="btn" onClick={browse} title="Choose a folder">
                  <Folder size={16} />
                </button>
              </div>
            </div>

            {notices.map((n, i) => (
              <div key={i} className={n.tone === "warn" ? "warn-box" : "info-box"}>
                {n.text}
              </div>
            ))}

            <button
              className="disclosure"
              onClick={() => {
                const next = !advanced;
                setAdvanced(next);
              }}
              aria-expanded={advanced}
            >
              {advanced ? <CaretDown size={13} /> : <CaretRight size={13} />}
              More options
            </button>
            {advanced && (
              <div className="grid-2">
                <div className="field">
                  <label htmlFor="conns">Connections</label>
                  <Select id="conns" className="select" value={connections} onValueChange={(value) => setConnections(+value)}>
                    {[1, 2, 4, 6, 8, 12, 16, 24, 32].map((n) => (
                      <option key={n} value={n}>
                        {n}
                        {n === settings.maxConnectionsPerDownload ? " (default)" : ""}
                      </option>
                    ))}
                  </Select>
                </div>
                <div className="field">
                  <label htmlFor="limit">Speed limit</label>
                  <input id="limit" className="input" value={limit} placeholder="No limit" onChange={(e) => setLimit(e.target.value)} />
                </div>
                <div className="field" style={{ gridColumn: "1 / -1" }}>
                  <label htmlFor="sum">Checksum (MD5, SHA-1 or SHA-256)</label>
                  <input id="sum" className="input selectable" value={checksum} placeholder="Optional" onChange={(e) => setChecksum(e.target.value)} spellCheck={false} />
                  <span className="hint">CrowDM verifies the finished file against it.</span>
                </div>
              </div>
            )}
          </>
        )}
        {error && <div className="error-box" style={{ marginTop: 0 }}>{error}</div>}
        </div>
      </div>

      <div className="capture-foot">
        {c.browserWaiting ? (
          <button
            className={`btn ${preferBrowser ? "primary" : "ghost"}`}
            onClick={() => api.captureToBrowser(c.key)}
            title="Let the browser finish this download itself"
          >
            Download in {browserName ?? "browser"}
          </button>
        ) : (
          waiting > 0 && <span className="queue-note">{waiting} more waiting</span>
        )}
        <div className="spacer" />
        <button className="btn" onClick={() => api.captureDismiss(c.key)}>
          Cancel
        </button>
        {!needsUrl && (
          <>
            <button className="btn" disabled={busy || c.probing || !!c.probeError || !p || p.looksLikeHtml && settings.interception.skipWebPages} onClick={() => accept(false)} title="Start when the queue has a free slot">
              Add to queue
            </button>
            <button ref={startRef} className={`btn ${preferBrowser ? "" : "primary"}`} disabled={busy || c.probing || !!c.probeError || !p || p.looksLikeHtml && settings.interception.skipWebPages} onClick={() => accept(true)}>
              {c.probing ? "Checking…" : "Start download"}
            </button>
          </>
        )}
      </div>
    </div>
  );
}
