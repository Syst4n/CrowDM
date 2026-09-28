import { FolderOpen, HandPalm, MagnifyingGlass, Pause, Play, Plus, Queue, Trash } from "@phosphor-icons/react";
import { api, errorText } from "../lib/api";
import { useStore } from "../lib/store";

export function Toolbar({ onRemove }: { onRemove: (ids: string[]) => void }) {
  const selected = useStore((s) => s.selected);
  const jobs = useStore((s) => s.jobs);
  const search = useStore((s) => s.search);
  const setSearch = useStore((s) => s.setSearch);
  const queueRunning = useStore((s) => s.queueRunning);
  const showToast = useStore((s) => s.showToast);

  const sel = selected.map((id) => jobs[id]).filter(Boolean);
  const canResume = sel.some((j) => j.state === "paused" || j.state === "failed" || j.state === "queued");
  const canPause = sel.some((j) => j.state === "downloading" || j.state === "queued");
  const run = (p: Promise<unknown>) => p.catch((e) => showToast(errorText(e), "error"));

  return (
    <div className="toolbar">
      <button className="btn primary" onClick={() => run(api.captureNew())} title="Add a download (Ctrl+N)">
        <Plus size={16} weight="bold" />
        Add URL
      </button>
      <div className="divider" />
      <button className="icon-btn" disabled={!canResume} onClick={() => run(api.resume(selected))} title="Resume">
        <Play size={18} />
      </button>
      <button className="icon-btn" disabled={!canPause} onClick={() => run(api.pause(selected))} title="Pause">
        <Pause size={18} />
      </button>
      <button className="icon-btn" disabled={!sel.length} onClick={() => onRemove(selected)} title="Remove (Delete)">
        <Trash size={18} />
      </button>
      <button className="icon-btn" disabled={sel.length !== 1} onClick={() => run(api.reveal(selected[0]))} title="Show in folder">
        <FolderOpen size={18} />
      </button>
      <div className="divider" />
      <button
        className="btn ghost"
        onClick={() => run(api.setQueueRunning(!queueRunning))}
        title={queueRunning ? "Queued downloads start automatically. Click to hold them." : "Queued downloads are on hold. Click to start them."}
      >
        {queueRunning ? <HandPalm size={16} /> : <Queue size={16} />}
        {queueRunning ? "Hold queue" : "Start queue"}
      </button>
      <div className="spacer" />
      <div className="search">
        <MagnifyingGlass size={15} />
        <input className="input" placeholder="Search downloads" value={search} onChange={(e) => setSearch(e.target.value)} aria-label="Search downloads" />
      </div>
    </div>
  );
}
