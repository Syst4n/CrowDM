import { useEffect, useState } from "react";
import { api, errorText } from "../lib/api";
import { useStore } from "../lib/store";

export function RemoveDialog({ ids, onClose }: { ids: string[]; onClose: () => void }) {
  const jobs = useStore((s) => s.jobs);
  const showToast = useStore((s) => s.showToast);
  const [deleteFiles, setDeleteFiles] = useState(false);
  const [busy, setBusy] = useState(false);
  const list = ids.map((id) => jobs[id]).filter(Boolean);
  const completed = list.filter((j) => j.state === "completed").length;
  const unfinished = list.length - completed;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  if (!list.length) return null;
  const title = list.length === 1 ? `Remove "${list[0].fileName}"?` : `Remove ${list.length} downloads?`;

  const confirm = async () => {
    setBusy(true);
    try {
      await api.remove(ids, deleteFiles);
      onClose();
    } catch (e) {
      showToast(errorText(e), "error");
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal" role="dialog" aria-modal="true" aria-labelledby="rm-title">
        <h2 id="rm-title" style={{ wordBreak: "break-all" }}>
          {title}
        </h2>
        <p style={{ margin: 0, color: "var(--text-2)" }}>
          {unfinished > 0 && "Unfinished downloads and their partial data are deleted. "}
          {completed > 0 && "Finished files stay on your disk unless you choose otherwise below."}
        </p>
        {completed > 0 && (
          <label className="check">
            <input type="checkbox" checked={deleteFiles} onChange={(e) => setDeleteFiles(e.target.checked)} />
            <span>Also move the downloaded file{completed > 1 ? "s" : ""} to the Recycle Bin</span>
          </label>
        )}
        <div className="buttons">
          <button className="btn" onClick={onClose} autoFocus>
            Cancel
          </button>
          <button className="btn primary" onClick={confirm} disabled={busy}>
            Remove
          </button>
        </div>
      </div>
    </div>
  );
}
