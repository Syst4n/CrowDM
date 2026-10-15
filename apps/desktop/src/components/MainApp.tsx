import { useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, errorText } from "../lib/api";
import { matchesFilter, sortJobs, startEngineSync, useStore } from "../lib/store";
import { useTheme } from "../lib/theme";
import { Sidebar } from "./Sidebar";
import { Toolbar } from "./Toolbar";
import { JobList } from "./JobList";
import { Details } from "./Details";
import { StatusBar } from "./StatusBar";
import { SettingsView } from "./SettingsView";
import { RemoveDialog } from "./RemoveDialog";

export function MainApp() {
  const load = useStore((s) => s.load);
  const settings = useStore((s) => s.settings);
  const view = useStore((s) => s.view);
  const toast = useStore((s) => s.toast);
  const [removing, setRemoving] = useState<string[] | null>(null);
  useTheme(settings);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    startEngineSync().then((u) => (unlisten = u));
    load()
      .catch((e) => useStore.getState().showToast(errorText(e), "error"))
      .finally(() => {
        // Show the window only after the first paint to avoid a white flash.
        requestAnimationFrame(() => getCurrentWindow().show());
      });
    return () => unlisten?.();
  }, [load]);

  // Global shortcuts.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      if (target.closest('input, textarea, select, [role="combobox"], [role="listbox"]')) return;
      if (e.ctrlKey && e.key.toLowerCase() === "n") {
        e.preventDefault();
        api.captureNew();
      } else if (e.ctrlKey && e.key.toLowerCase() === "v") {
        // Paste a link anywhere to add it.
        navigator.clipboard
          .readText()
          .then((t) => {
            const url = t.trim();
            if (/^https?:\/\//i.test(url)) api.captureNew(url);
          })
          .catch(() => {});
      } else if (e.key === "Delete") {
        const sel = useStore.getState().selected;
        if (sel.length) setRemoving(sel);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="app">
      <Sidebar />
      <div className="main">
        {view === "settings" ? (
          <SettingsView />
        ) : (
          <>
            <Toolbar onRemove={(ids) => setRemoving(ids)} />
            <Downloads onRemove={(ids) => setRemoving(ids)} />
          </>
        )}
        <StatusBar />
      </div>
      {removing && <RemoveDialog ids={removing} onClose={() => setRemoving(null)} />}
      {toast && <div className={`toast ${toast.tone === "error" ? "error" : ""}`}>{toast.text}</div>}
    </div>
  );
}

function Downloads({ onRemove }: { onRemove: (ids: string[]) => void }) {
  const contentRef = useRef<HTMLDivElement>(null);
  const [detailsHeight, setDetailsHeight] = useState(() => {
    const saved = Number(localStorage.getItem("crowdm.detailsHeight"));
    return Number.isFinite(saved) && saved >= 120 ? saved : 180;
  });
  const drag = useRef<{ y: number; height: number } | null>(null);
  const resizeDetails = (height: number) => {
    const max = Math.max(120, (contentRef.current?.clientHeight ?? 600) - 160);
    const next = Math.round(Math.max(120, Math.min(max, height)));
    setDetailsHeight(next);
    localStorage.setItem("crowdm.detailsHeight", String(next));
  };
  const jobs = useStore((s) => s.jobs);
  const listVersion = useStore((s) => s.listVersion);
  const filter = useStore((s) => s.filter);
  const search = useStore((s) => s.search);
  const sort = useStore((s) => s.sort);
  const selected = useStore((s) => s.selected);

  // Re-sort only on structural changes, not on every progress tick.
  const ids = useMemo(() => {
    const q = search.trim().toLowerCase();
    const list = Object.values(useStore.getState().jobs).filter(
      (j) => matchesFilter(j, filter) && (!q || j.fileName.toLowerCase().includes(q) || j.url.toLowerCase().includes(q)),
    );
    return sortJobs(list, sort).map((j) => j.id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [listVersion, filter, search, sort]);

  const single = selected.length === 1 ? jobs[selected[0]] : undefined;
  return (
    <div className="content" ref={contentRef}>
      <JobList ids={ids} onRemove={onRemove} />
      {single && <div className="details-pane" style={{ height: detailsHeight }}>
        <div className="details-resizer" role="separator" aria-label="Resize download details" aria-orientation="horizontal"
          aria-valuemin={120} aria-valuenow={detailsHeight} aria-valuemax={Math.max(120, (contentRef.current?.clientHeight ?? 600) - 160)} tabIndex={0}
          onPointerDown={(e) => { drag.current = { y: e.clientY, height: e.currentTarget.parentElement!.clientHeight }; e.currentTarget.setPointerCapture(e.pointerId); }}
          onPointerMove={(e) => { if (drag.current) resizeDetails(drag.current.height + drag.current.y - e.clientY); }}
          onPointerUp={() => { drag.current = null; }} onLostPointerCapture={() => { drag.current = null; }}
          onDoubleClick={() => resizeDetails(180)}
          onKeyDown={(e) => { if (["ArrowUp", "ArrowDown", "Home", "End"].includes(e.key)) { e.preventDefault(); resizeDetails(e.key === "Home" ? 120 : e.key === "End" ? 10000 : detailsHeight + (e.key === "ArrowUp" ? 20 : -20)); } }} />
        <Details job={single} onRemove={onRemove} />
      </div>}
    </div>
  );
}
