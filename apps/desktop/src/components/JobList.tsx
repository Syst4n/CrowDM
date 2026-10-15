import { useEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowCircleDown, CheckCircle, Clock, DownloadSimple, FolderOpen, PauseCircle, WarningCircle } from "@phosphor-icons/react";
import { api, errorText, type JobSnapshot } from "../lib/api";
import { useStore, type SortKey } from "../lib/store";
import * as fmt from "../lib/format";
import { FilePreview } from "./FilePreview";
import { ContextMenu, type MenuItem } from "./ContextMenu";

const ROW = 52;

export function JobList({ ids, onRemove }: { ids: string[]; onRemove: (ids: string[]) => void }) {
  const parentRef = useRef<HTMLDivElement>(null);
  const selected = useStore((s) => s.selected);
  const select = useStore((s) => s.select);
  const sort = useStore((s) => s.sort);
  const setSort = useStore((s) => s.setSort);
  const hasAnyJobs = useStore((s) => Object.keys(s.jobs).length > 0);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);

  const v = useVirtualizer({ count: ids.length, getScrollElement: () => parentRef.current, estimateSize: () => ROW, overscan: 8 });

  useEffect(() => {
    // Drop selections that are no longer visible.
    const visible = new Set(ids);
    const s = useStore.getState().selected;
    if (s.some((id) => !visible.has(id))) select(s.filter((id) => visible.has(id)));
  }, [ids, select]);

  const onRowClick = (e: React.MouseEvent, id: string, index: number) => {
    const s = useStore.getState();
    if (e.shiftKey && s.lastClicked) {
      const a = ids.indexOf(s.lastClicked);
      const [lo, hi] = a < index ? [a, index] : [index, a];
      select(ids.slice(lo, hi + 1));
    } else if (e.ctrlKey) {
      select(s.selected.includes(id) ? s.selected.filter((x) => x !== id) : [...s.selected, id], id);
    } else {
      select([id], id);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    const s = useStore.getState();
    const cur = s.lastClicked ? ids.indexOf(s.lastClicked) : -1;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = Math.max(0, Math.min(ids.length - 1, cur + (e.key === "ArrowDown" ? 1 : -1)));
      if (ids[next]) {
        select([ids[next]], ids[next]);
        v.scrollToIndex(next);
      }
    } else if (e.ctrlKey && e.key.toLowerCase() === "a") {
      e.preventDefault();
      select([...ids]);
    } else if (e.key === "Enter" && s.selected.length === 1) {
      const j = s.jobs[s.selected[0]];
      if (j?.state === "completed") api.openFile(j.id).catch((err) => s.showToast(errorText(err), "error"));
    } else if (e.key === " " && s.selected.length) {
      e.preventDefault();
      const js = s.selected.map((id) => s.jobs[id]).filter(Boolean);
      if (js.some((j) => j.state === "downloading")) api.pause(s.selected);
      else api.resume(s.selected);
    }
  };

  if (ids.length === 0) {
    return (
      <div className="empty">
        <div className="inner">
          <DownloadSimple size={40} weight="light" />
          <h2>{hasAnyJobs ? "Nothing here" : "No downloads yet"}</h2>
          <p>
            {hasAnyJobs
              ? "No downloads match this view."
              : "Click a download link in Chrome, Edge or Firefox, or paste a link with Ctrl+V."}
          </p>
          {!hasAnyJobs && (
            <button className="btn primary large" onClick={() => api.captureNew()}>
              Add URL
            </button>
          )}
        </div>
      </div>
    );
  }

  const head = (key: SortKey | null, label: string) =>
    key ? (
      <button className={sort === key ? "on" : ""} onClick={() => setSort(key)}>
        {label}
      </button>
    ) : (
      <span>{label}</span>
    );

  return (
    <>
      <div className="list-head">
        <span />
        {head("name", "Name")}
        {head("size", "Size")}
        {head("progress", "Progress")}
        {head(null, "Speed")}
        {head(null, "Time left")}
        {head("status", "Status")}
        <span />
      </div>
      <div className="list" ref={parentRef} tabIndex={0} onKeyDown={onKeyDown} role="listbox" aria-multiselectable>
        <div style={{ height: v.getTotalSize(), position: "relative" }}>
          {v.getVirtualItems().map((item) => (
            <Row
              key={ids[item.index]}
              id={ids[item.index]}
              top={item.start}
              selected={selected.includes(ids[item.index])}
              onClick={(e) => onRowClick(e, ids[item.index], item.index)}
              onContext={(e) => {
                e.preventDefault();
                if (!useStore.getState().selected.includes(ids[item.index])) select([ids[item.index]], ids[item.index]);
                setMenu({ x: e.clientX, y: e.clientY });
              }}
            />
          ))}
        </div>
      </div>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menuItems(onRemove)} onClose={() => setMenu(null)} />}
    </>
  );
}

function menuItems(onRemove: (ids: string[]) => void): MenuItem[] {
  const s = useStore.getState();
  const sel = s.selected.map((id) => s.jobs[id]).filter(Boolean);
  const one = sel.length === 1 ? sel[0] : undefined;
  const run = (p: Promise<unknown>) => p.catch((e) => s.showToast(errorText(e), "error"));
  return [
    { label: "Open", disabled: one?.state !== "completed", onClick: () => one && run(api.openFile(one.id)) },
    { label: "Show in folder", disabled: !one, onClick: () => one && run(api.reveal(one.id)) },
    { divider: true },
    { label: "Resume", disabled: !sel.some((j) => ["paused", "failed", "queued"].includes(j.state)), onClick: () => run(api.resume(s.selected)) },
    { label: "Pause", disabled: !sel.some((j) => ["downloading", "queued"].includes(j.state)), onClick: () => run(api.pause(s.selected)) },
    { label: "Download again", disabled: !one, onClick: () => one && run(api.restart(one.id)) },
    {
      label: sel.every((j) => j.queued) ? "Take out of queue" : "Add to queue",
      disabled: !sel.length || sel.every((j) => j.state === "completed"),
      onClick: () => run(api.setQueued(s.selected, !sel.every((j) => j.queued))),
    },
    { divider: true },
    { label: "Copy address", disabled: !one, onClick: () => one && navigator.clipboard.writeText(one.url).then(() => s.showToast("Address copied")) },
    { label: "Remove...", danger: true, onClick: () => onRemove(s.selected) },
  ];
}

function Row({ id, top, selected, onClick, onContext }: {
  id: string;
  top: number;
  selected: boolean;
  onClick: (e: React.MouseEvent) => void;
  onContext: (e: React.MouseEvent) => void;
}) {
  const j = useStore((s) => s.jobs[id]);
  if (!j) return null;
  const progress = fmt.percent(j.downloaded, j.size);
  const pct = j.mediaDownload && j.state !== "completed" && progress != null ? Math.min(99, progress) : progress;
  return (
    <div
      className={`row ${selected ? "selected" : ""}`}
      style={{ transform: `translateY(${top}px)` }}
      onMouseDown={onClick}
      onDoubleClick={() => (j.state === "completed" ? api.openFile(j.id) : j.state === "downloading" ? api.pause([j.id]) : api.resume([j.id])).catch((e) => useStore.getState().showToast(errorText(e), "error"))}
      onContextMenu={onContext}
      role="option"
      aria-selected={selected}
    >
      <div className="file-icon">
        <FilePreview job={j} />
      </div>
      <div className="name">
        <div className="title" title={j.fileName}>
          {j.fileName}
        </div>
        <div className="sub">{fmt.host(j.source.pageUrl ?? j.url)}</div>
      </div>
      <div className="cell">{j.size != null ? `${j.mediaDownload && j.state !== "completed" ? "≈ " : ""}${fmt.bytes(j.size)}` : j.state === "completed" ? fmt.bytes(j.downloaded) : "Unknown"}</div>
      <ProgressCell j={j} pct={pct} />
      <div className="cell">{j.state === "downloading" ? fmt.speed(j.speed) : ""}</div>
      <div className="cell">{j.state === "downloading" ? fmt.duration(j.eta) : ""}</div>
      <StatusCell j={j} />
      <button className="icon-btn row-reveal" title="Open file location" aria-label={`Open file location for ${j.fileName}`}
        onMouseDown={(e) => e.stopPropagation()} onDoubleClick={(e) => e.stopPropagation()}
        onClick={(e) => { e.stopPropagation(); api.reveal(j.id).catch((err) => useStore.getState().showToast(errorText(err), "error")); }}>
        <FolderOpen size={18} />
      </button>
    </div>
  );
}

function ProgressCell({ j, pct }: { j: JobSnapshot; pct: number | null }) {
  const cls = j.state === "paused" || j.state === "queued" ? "paused" : j.state === "failed" ? "failed" : "";
  const indeterminate = j.state === "downloading" && pct == null;
  return (
    <div className="progress">
      <div className={`bar ${cls} ${indeterminate ? "indeterminate" : ""}`}>
        <span style={{ width: `${j.state === "completed" ? 100 : pct ?? 0}%` }} />
      </div>
      <div className="meta">
        <span>{j.state === "completed" ? "Done" : pct != null ? `${pct.toFixed(pct < 10 ? 1 : 0)}%` : fmt.bytes(j.downloaded)}</span>
        {j.state === "downloading" && j.connections > 0 && <span>{j.connections} conn</span>}
      </div>
    </div>
  );
}

export function statusText(j: JobSnapshot): string {
  switch (j.state) {
    case "downloading":
      return j.phase ?? "Downloading";
    case "queued":
      return "Queued";
    case "paused":
      return "Paused";
    case "failed":
      return "Failed";
    case "completed":
      return fmt.when(j.completedAt) || "Completed";
  }
}

function StatusCell({ j }: { j: JobSnapshot }) {
  const icon = {
    downloading: <ArrowCircleDown size={15} />,
    queued: <Clock size={15} />,
    paused: <PauseCircle size={15} />,
    failed: <WarningCircle size={15} />,
    completed: <CheckCircle size={15} />,
  }[j.state];
  return (
    <div className={`status ${j.state}`} title={j.error?.message ?? statusText(j)}>
      {icon}
      <span className="cell" style={{ color: "inherit" }}>
        {statusText(j)}
      </span>
    </div>
  );
}
