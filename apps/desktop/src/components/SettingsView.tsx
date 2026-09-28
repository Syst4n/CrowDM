import { useEffect, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Plus, X } from "@phosphor-icons/react";
import { api, errorText, type BrowserStatus, type Settings, type AppInfo } from "../lib/api";
import { useStore } from "../lib/store";
import * as fmt from "../lib/format";

type Section = "general" | "folders" | "connections" | "browsers" | "queue" | "about";
const SECTIONS: [Section, string][] = [
  ["general", "General"],
  ["folders", "Folders and categories"],
  ["connections", "Connections"],
  ["browsers", "Browsers"],
  ["queue", "Queue and schedule"],
  ["about", "About"],
];

export function SettingsView() {
  const settings = useStore((s) => s.settings);
  const setSettings = useStore((s) => s.setSettings);
  const showToast = useStore((s) => s.showToast);
  const [section, setSection] = useState<Section>("general");

  if (!settings) return null;
  const save = (patch: Partial<Settings>) => {
    const next = { ...settings, ...patch };
    setSettings(next);
    api.saveSettings(next).then(setSettings).catch((e) => showToast(errorText(e), "error"));
  };

  return (
    <div className="settings">
      <nav className="settings-nav" aria-label="Settings sections">
        {SECTIONS.map(([id, label]) => (
          <button key={id} className={`nav-item ${section === id ? "active" : ""}`} onClick={() => setSection(id)}>
            {label}
          </button>
        ))}
      </nav>
      <div className="settings-body">
        {section === "general" && <General s={settings} save={save} />}
        {section === "folders" && <Folders s={settings} save={save} />}
        {section === "connections" && <Connections s={settings} save={save} />}
        {section === "browsers" && <Browsers s={settings} save={save} />}
        {section === "queue" && <QueueSettings s={settings} save={save} />}
        {section === "about" && <About />}
      </div>
    </div>
  );
}

type P = { s: Settings; save: (p: Partial<Settings>) => void };

function Check({ checked, onChange, title, hint }: { checked: boolean; onChange: (v: boolean) => void; title: string; hint?: string }) {
  return (
    <label className="check">
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      <span>
        <div className="title">{title}</div>
        {hint && <div className="hint">{hint}</div>}
      </span>
    </label>
  );
}

function General({ s, save }: P) {
  return (
    <section>
      <h2>General</h2>
      <div className="field" style={{ maxWidth: 240 }}>
        <label htmlFor="theme">Appearance</label>
        <select id="theme" className="select" value={s.theme} onChange={(e) => save({ theme: e.target.value as Settings["theme"] })}>
          <option value="system">Match Windows</option>
          <option value="light">Light</option>
          <option value="dark">Dark</option>
        </select>
      </div>
      <Check checked={s.startWithWindows} onChange={(v) => save({ startWithWindows: v })} title="Start CrowDM when I sign in to Windows" hint="It starts quietly in the notification area." />
      <Check checked={s.closeToTray} onChange={(v) => save({ closeToTray: v })} title="Keep running in the notification area when the window is closed" hint="Needed to catch browser downloads while the window is closed." />
      <Check checked={s.startMinimized} onChange={(v) => save({ startMinimized: v })} title="Start without opening the window" />
      <Check checked={s.confirmCaptured} onChange={(v) => save({ confirmCaptured: v })} title="Ask before starting a download from the browser" hint="Turn off to start caught downloads right away with the default folder." />
      <Check checked={s.notifyOnComplete} onChange={(v) => save({ notifyOnComplete: v })} title="Show a notification when a download finishes" />
      <Check checked={s.resumeOnStartup} onChange={(v) => save({ resumeOnStartup: v })} title="Continue unfinished downloads when CrowDM starts" />
      <Check
        checked={s.prewarmDialog}
        onChange={(v) => save({ prewarmDialog: v })}
        title="Keep the download dialog ready"
        hint="Opens the dialog instantly for browser downloads, at the cost of some memory."
      />
    </section>
  );
}

function Folders({ s, save }: P) {
  const [cats, setCats] = useState(s.categories);
  useEffect(() => setCats(s.categories), [s.categories]);
  const pick = async () => {
    const dir = await openDialog({ directory: true, defaultPath: s.downloadRoot, title: "Choose the download folder" });
    if (typeof dir === "string") save({ downloadRoot: dir });
  };
  const commit = (next = cats) => save({ categories: next });
  const update = (i: number, patch: Partial<(typeof cats)[number]>) => setCats(cats.map((c, j) => (j === i ? { ...c, ...patch } : c)));

  return (
    <section>
      <h2>Folders and categories</h2>
      <div className="field">
        <label>Download folder</label>
        <div className="input-row">
          <input className="input selectable" value={s.downloadRoot} readOnly />
          <button className="btn" onClick={pick}>
            Change
          </button>
          <button className="btn ghost" onClick={() => api.openFolder(s.downloadRoot)}>
            Open
          </button>
        </div>
      </div>
      <Check
        checked={s.organizeByCategory}
        onChange={(v) => save({ organizeByCategory: v })}
        title="Sort files into category folders"
        hint="For example, videos go to Videos inside the download folder."
      />
      <h3>Categories</h3>
      <p className="hint" style={{ margin: 0 }}>
        Files are matched by extension. Anything that matches nothing goes to Other. A folder can be a name inside the download folder or a full path.
      </p>
      <div className="cat-table">
        <div className="cat-row head">
          <span>Name</span>
          <span>Folder</span>
          <span>Extensions</span>
          <span />
        </div>
        {cats.map((c, i) => (
          <div className="cat-row" key={c.id}>
            <input className="input" value={c.name} onChange={(e) => update(i, { name: e.target.value })} onBlur={() => commit()} aria-label="Category name" />
            <input className="input" value={c.folder} onChange={(e) => update(i, { folder: e.target.value })} onBlur={() => commit()} aria-label="Folder" />
            <input
              className="input"
              value={c.extensions.join(" ")}
              disabled={c.id === "other"}
              placeholder={c.id === "other" ? "Everything else" : "zip rar 7z"}
              onChange={(e) => update(i, { extensions: e.target.value.split(/[\s,]+/).filter(Boolean) })}
              onBlur={() => commit()}
              aria-label="Extensions"
            />
            <button
              className="icon-btn"
              disabled={c.id === "other"}
              title="Remove category"
              onClick={() => {
                const next = cats.filter((_, j) => j !== i);
                setCats(next);
                commit(next);
              }}
            >
              <X size={15} />
            </button>
          </div>
        ))}
      </div>
      <div>
        <button
          className="btn"
          onClick={() => {
            const next = [...cats.filter((c) => c.id !== "other"), { id: `custom-${Date.now()}`, name: "New category", folder: "New category", extensions: [] }, ...cats.filter((c) => c.id === "other")];
            setCats(next);
            commit(next);
          }}
        >
          <Plus size={15} /> Add category
        </button>
      </div>
    </section>
  );
}

function RateInput({ value, onSave, id }: { value: number; onSave: (v: number) => void; id: string }) {
  const [text, setText] = useState(fmt.rateText(value));
  const [bad, setBad] = useState(false);
  useEffect(() => setText(fmt.rateText(value)), [value]);
  return (
    <>
      <input
        id={id}
        className="input"
        value={text}
        placeholder="No limit"
        onChange={(e) => setText(e.target.value)}
        onBlur={() => {
          const v = fmt.parseRate(text);
          setBad(v == null);
          if (v != null) onSave(v);
        }}
      />
      {bad && <span className="hint" style={{ color: "var(--danger)" }}>Use a value like 800 KB or 5 MB.</span>}
    </>
  );
}

function Connections({ s, save }: P) {
  const num = (v: string, lo: number, hi: number) => Math.max(lo, Math.min(hi, Math.round(+v || lo)));
  return (
    <section>
      <h2>Connections</h2>
      <div className="grid-2">
        <div className="field">
          <label htmlFor="cpd">Connections per download</label>
          <select id="cpd" className="select" value={s.maxConnectionsPerDownload} onChange={(e) => save({ maxConnectionsPerDownload: +e.target.value })}>
            {[1, 2, 4, 6, 8, 12, 16, 24, 32].map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
          <span className="hint">More connections help on slow or distant servers. CrowDM backs off when a server refuses extra connections.</span>
        </div>
        <div className="field">
          <label htmlFor="tot">Connections in total</label>
          <input id="tot" className="input" type="number" min={1} max={256} defaultValue={s.maxTotalConnections} onBlur={(e) => save({ maxTotalConnections: num(e.target.value, 1, 256) })} />
        </div>
        <div className="field">
          <label htmlFor="limit">Overall speed limit</label>
          <RateInput id="limit" value={s.speedLimit} onSave={(v) => save({ speedLimit: v })} />
        </div>
        <div className="field">
          <label htmlFor="retry">Automatic retries</label>
          <input id="retry" className="input" type="number" min={0} max={100} defaultValue={s.retryAttempts} onBlur={(e) => save({ retryAttempts: num(e.target.value, 0, 100) })} />
          <span className="hint">For network errors and busy servers, {s.retryDelaySecs} seconds apart.</span>
        </div>
      </div>
      <div className="field">
        <label htmlFor="ua">Custom browser identity (User-Agent)</label>
        <input
          id="ua"
          className="input"
          defaultValue={s.userAgent ?? ""}
          placeholder="Default"
          onBlur={(e) => save({ userAgent: e.target.value.trim() || null })}
          spellCheck={false}
        />
        <span className="hint">Used for links you add in CrowDM. Downloads caught from a browser use that browser's identity.</span>
      </div>
    </section>
  );
}

function Browsers({ s, save }: P) {
  const [status, setStatus] = useState<BrowserStatus | null>(null);
  const showToast = useStore((st) => st.showToast);
  const [sitesText, setSitesText] = useState(s.interception.excludedSites.join("\n"));
  const [extText, setExtText] = useState(s.interception.excludedExtensions.join(" "));
  useEffect(() => {
    api.browserStatus().then(setStatus);
  }, []);
  const i = s.interception;
  const setI = (patch: Partial<Settings["interception"]>) => save({ interception: { ...i, ...patch } });
  const connected = new Set(status?.connected.map((c) => c.browser));
  const reg = (id: string) => status?.integration.browsers.find((b) => b.browser === id)?.registered;

  return (
    <section>
      <h2>Browsers</h2>
      <Check checked={i.enabled} onChange={(v) => setI({ enabled: v })} title="Catch downloads from my browsers" hint="When a download starts in the browser, CrowDM takes it over and asks where to save it." />
      <Check checked={i.videoButton} onChange={(v) => setI({ videoButton: v })} title='Show a "Download video" button on web videos' hint="Works for videos that are plain files. Streaming services and protected videos are not supported." />
      <Check checked={i.includePrivate} onChange={(v) => setI({ includePrivate: v })} title="Also catch downloads from private windows" />
      <div className="grid-2">
        <div className="field">
          <label htmlFor="sites">Never catch downloads from these sites</label>
          <textarea
            id="sites"
            className="textarea"
            rows={4}
            value={sitesText}
            placeholder={"example.com\nintranet.local"}
            onChange={(e) => setSitesText(e.target.value)}
            onBlur={() => setI({ excludedSites: sitesText.split(/[\s,]+/).map((x) => x.trim().toLowerCase()).filter(Boolean) })}
            spellCheck={false}
          />
        </div>
        <div className="field">
          <label htmlFor="exts">Leave these file types to the browser</label>
          <input
            id="exts"
            className="input"
            value={extText}
            placeholder="pdf jpg"
            onChange={(e) => setExtText(e.target.value)}
            onBlur={() => setI({ excludedExtensions: extText.split(/[\s,]+/).map((x) => x.replace(/^\./, "").toLowerCase()).filter(Boolean) })}
            spellCheck={false}
          />
          <label htmlFor="min" style={{ marginTop: 8 }}>
            Leave files smaller than this to the browser
          </label>
          <RateInput id="min" value={i.minSize} onSave={(v) => setI({ minSize: v })} />
        </div>
      </div>

      <h3>Extensions</h3>
      <div className="browser-list">
        {[
          ["chrome", "Google Chrome"],
          ["edge", "Microsoft Edge"],
          ["firefox", "Mozilla Firefox"],
        ].map(([id, label]) => (
          <div key={id}>
            <span style={{ fontWeight: 500 }}>{label}</span>
            <span className="state">
              {connected.has(id) ? (
                <span className="ok-text">Extension connected</span>
              ) : reg(id) ? (
                <span className="muted">Ready, extension not connected</span>
              ) : (
                <span className="muted">Not set up</span>
              )}
            </span>
          </div>
        ))}
      </div>
      <ol className="steps">
        <li>
          Chrome or Edge: open <code>chrome://extensions</code> or <code>edge://extensions</code>, turn on Developer mode, choose <b>Load unpacked</b> and pick the <code>chromium</code> folder.
        </li>
        <li>
          Firefox: open <code>about:debugging#/runtime/this-firefox</code> and choose <b>Load Temporary Add-on</b>, then pick <code>manifest.json</code> in the <code>firefox</code> folder. A signed Firefox add-on is needed for a permanent install.
        </li>
      </ol>
      <div className="input-row">
        <button className="btn" onClick={() => api.openExtensionsFolder().catch((e) => showToast(errorText(e), "error"))}>
          Open extension folder
        </button>
        <button
          className="btn ghost"
          onClick={() =>
            api
              .registerBrowsers()
              .then((st) => (setStatus(st), showToast("Browser connection repaired.")))
              .catch((e) => showToast(errorText(e), "error"))
          }
        >
          Repair browser connection
        </button>
      </div>
    </section>
  );
}

const DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

function QueueSettings({ s, save }: P) {
  const sch = s.schedule;
  const setSch = (patch: Partial<Settings["schedule"]>) => save({ schedule: { ...sch, ...patch } });
  return (
    <section>
      <h2>Queue and schedule</h2>
      <div className="grid-2">
        <div className="field">
          <label htmlFor="act">Queued downloads at the same time</label>
          <select id="act" className="select" value={s.maxActiveDownloads} onChange={(e) => save({ maxActiveDownloads: +e.target.value })}>
            {[1, 2, 3, 4, 5, 6, 8, 10, 16].map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
          <span className="hint">Downloads you start directly always run right away.</span>
        </div>
        <div className="field">
          <label htmlFor="after">When the queue finishes</label>
          <select id="after" className="select" value={s.afterQueue} onChange={(e) => save({ afterQueue: e.target.value as Settings["afterQueue"] })}>
            <option value="nothing">Do nothing</option>
            <option value="notify">Show a notification</option>
            <option value="exitApp">Close CrowDM</option>
            <option value="sleep">Put the computer to sleep</option>
            <option value="shutdown">Shut down (with 60 seconds to cancel)</option>
          </select>
        </div>
      </div>
      <h3>Schedule</h3>
      <Check checked={sch.enabled} onChange={(v) => setSch({ enabled: v })} title="Run the queue on a schedule" hint="Queued downloads only start inside this time window." />
      <div className="grid-2" style={{ opacity: sch.enabled ? 1 : 0.5 }}>
        <div className="field">
          <label htmlFor="st">Start at</label>
          <input id="st" className="input" type="time" value={sch.startTime} disabled={!sch.enabled} onChange={(e) => setSch({ startTime: e.target.value })} />
        </div>
        <div className="field">
          <label htmlFor="sp">Stop at</label>
          <input id="sp" className="input" type="time" value={sch.stopTime ?? ""} disabled={!sch.enabled} onChange={(e) => setSch({ stopTime: e.target.value || null })} />
          <span className="hint">Leave empty to run until the queue is done.</span>
        </div>
      </div>
      <div className="pill-days" style={{ opacity: sch.enabled ? 1 : 0.5 }}>
        {DAYS.map((d, i) => (
          <button
            key={d}
            className={sch.days[i] ? "on" : ""}
            disabled={!sch.enabled}
            onClick={() => setSch({ days: sch.days.map((v, j) => (j === i ? !v : v)) })}
            aria-pressed={sch.days[i]}
          >
            {d}
          </button>
        ))}
      </div>
    </section>
  );
}

function About() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [mem, setMem] = useState<{ pid: number; name: string; privateBytes: number }[] | null>(null);
  useEffect(() => {
    api.appInfo().then(setInfo);
  }, []);
  const total = mem?.reduce((a, m) => a + m.privateBytes, 0) ?? 0;
  return (
    <section>
      <h2>About CrowDM</h2>
      <p style={{ margin: 0, color: "var(--text-2)" }}>
        Version {info?.version} {info?.portable ? "(portable)" : ""}
      </p>
      <div className="field">
        <label>Data folder</label>
        <div className="input-row">
          <input className="input selectable" readOnly value={info?.dataDir ?? ""} />
          <button className="btn ghost" onClick={() => api.openDataFolder()}>
            Open logs
          </button>
        </div>
      </div>
      <div className="field">
        <label>Memory use</label>
        {mem ? (
          <div className="hint" style={{ color: "var(--text-2)" }}>
            {fmt.bytes(total)} private memory across {mem.length} processes ({mem.map((m) => `${m.name} ${fmt.bytes(m.privateBytes)}`).join(", ")})
          </div>
        ) : (
          <div>
            <button className="btn" onClick={() => api.memoryUsage().then(setMem)}>
              Measure
            </button>
          </div>
        )}
      </div>
      <p className="hint" style={{ margin: 0 }}>
        Source code and issue tracker: github.com/Syst4n/CrowDM
      </p>
    </section>
  );
}
