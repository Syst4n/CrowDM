import { useMemo, type ReactNode } from "react";
import {
  ArrowCircleDown,
  CheckCircle,
  Clock,
  DownloadSimple,
  GearSix,
  PauseCircle,
  WarningCircle,
} from "@phosphor-icons/react";
import { useStore, type Filter } from "../lib/store";
import { FileIcon } from "./FileIcon";
import { Logo } from "./Logo";
import type { Category } from "../lib/api";

const NO_CATEGORIES: Category[] = [];

export function Sidebar() {
  const jobs = useStore((s) => s.jobs);
  const listVersion = useStore((s) => s.listVersion);
  const filter = useStore((s) => s.filter);
  const view = useStore((s) => s.view);
  const setFilter = useStore((s) => s.setFilter);
  const setView = useStore((s) => s.setView);
  const categories = useStore((s) => s.settings?.categories) ?? NO_CATEGORIES;

  const counts = useMemo(() => {
    const c: Record<string, number> = { all: 0, downloading: 0, queued: 0, paused: 0, failed: 0, completed: 0 };
    const cat: Record<string, number> = {};
    for (const j of Object.values(jobs)) {
      c.all++;
      c[j.state]++;
      cat[j.category] = (cat[j.category] ?? 0) + 1;
    }
    return { c, cat };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [listVersion]);

  const item = (f: Filter, label: string, icon: ReactNode, count: number) => {
    const active = view === "downloads" && JSON.stringify(f) === JSON.stringify(filter);
    return (
      <button key={label} className={`nav-item ${active ? "active" : ""}`} onClick={() => setFilter(f)}>
        {icon}
        <span>{label}</span>
        {count > 0 && <span className="count">{count}</span>}
      </button>
    );
  };

  return (
    <nav className="sidebar" aria-label="Filters">
      <div className="brand">
        <Logo size={34} />
        CrowDM
      </div>
      {item({ kind: "all" }, "All downloads", <DownloadSimple size={18} />, counts.c.all)}
      {item({ kind: "active" }, "Downloading", <ArrowCircleDown size={18} />, counts.c.downloading)}
      {item({ kind: "queued" }, "Queued", <Clock size={18} />, counts.c.queued)}
      {item({ kind: "paused" }, "Paused", <PauseCircle size={18} />, counts.c.paused)}
      {item({ kind: "failed" }, "Failed", <WarningCircle size={18} />, counts.c.failed)}
      {item({ kind: "completed" }, "Completed", <CheckCircle size={18} />, counts.c.completed)}
      <div className="nav-group">Categories</div>
      {categories.map((c) => item({ kind: "category", id: c.id }, c.name, <FileIcon category={c.id} />, counts.cat[c.id] ?? 0))}
      <div className="sidebar-foot">
        <button className={`nav-item ${view === "settings" ? "active" : ""}`} onClick={() => setView("settings")}>
          <GearSix size={18} />
          <span>Settings</span>
        </button>
      </div>
    </nav>
  );
}
