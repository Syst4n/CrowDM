import { useState } from "react";
import { api, errorText, type JobSnapshot } from "../lib/api";
import { useStore } from "../lib/store";
import * as fmt from "../lib/format";

const SOURCE: Record<string, string> = {
  browserCapture: "Caught from the browser",
  browserMenu: "Browser menu",
  video: "Video button",
  manual: "Added in CrowDM",
};

export function Details({ job: j, onRemove }: { job: JobSnapshot; onRemove: (ids: string[]) => void }) {
  const showToast = useStore((s) => s.showToast);
  const [editingLimit, setEditingLimit] = useState(false);
  const [limitText, setLimitText] = useState("");
  const [refreshing, setRefreshing] = useState(false);
  const [newUrl, setNewUrl] = useState("");
  const run = (p: Promise<unknown>) => p.catch((e) => showToast(errorText(e), "error"));
  const pct = fmt.percent(j.downloaded, j.size);

  const saveLimit = () => {
    const v = fmt.parseRate(limitText);
    if (v == null) {
      showToast("Enter a speed like 500 KB or 2 MB.", "error");
      return;
    }
    run(api.updateJob(j.id, { speedLimit: v || null }).then(() => setEditingLimit(false)));
  };

  return (
    <div className="details">
      <div>
        <h3 className="selectable">{j.fileName}</h3>
        <div className="actions">
          {j.state === "completed" && (
            <button className="btn primary" onClick={() => run(api.openFile(j.id))}>
              Open
            </button>
          )}
          {(j.state === "paused" || j.state === "failed" || j.state === "queued") && (
            <button className="btn primary" onClick={() => run(api.resume([j.id]))}>
              {j.state === "failed" ? "Retry" : "Resume"}
            </button>
          )}
          {j.state === "downloading" && (
            <button className="btn" onClick={() => run(api.pause([j.id]))}>
              Pause
            </button>
          )}
          <button className="btn" onClick={() => run(api.reveal(j.id))}>
            Show in folder
          </button>
          {j.state !== "completed" && j.state !== "downloading" && !refreshing && (
            <button className="btn ghost" onClick={() => (setNewUrl(j.url), setRefreshing(true))}>
              Update address
            </button>
          )}
          <button className="btn ghost danger" onClick={() => onRemove([j.id])}>
            Remove
          </button>
        </div>
        <dl className="kv">
          <dt>Status</dt>
          <dd>
            {j.state === "downloading"
              ? `${j.phase ?? "Downloading"}${pct != null ? `, ${pct.toFixed(1)}%` : ""}`
              : j.state[0].toUpperCase() + j.state.slice(1)}
            {j.state === "downloading" && j.speed > 0 && ` at ${fmt.speed(j.speed)}`}
          </dd>
          <dt>Size</dt>
          <dd className="num">
            {fmt.bytes(j.downloaded)} of {j.size != null ? fmt.bytes(j.size) : "unknown size"}
          </dd>
          <dt>Saved to</dt>
          <dd className="selectable">{j.dir}</dd>
          <dt>Address</dt>
          <dd className="selectable" title={j.finalUrl ?? j.url}>
            {j.url}
          </dd>
          {j.source.pageUrl && (
            <>
              <dt>From page</dt>
              <dd className="selectable">{j.source.pageUrl}</dd>
            </>
          )}
          <dt>Source</dt>
          <dd>
            {SOURCE[j.source.kind]}
            {j.source.browser ? ` (${j.source.browser})` : ""}
          </dd>
          <dt>Resume</dt>
          <dd>{j.resumable == null ? "Not checked yet" : j.resumable ? "Supported" : "Not supported by the server"}</dd>
          <dt>Added</dt>
          <dd title={new Date(j.createdAt).toLocaleString()}>{fmt.when(j.createdAt)}</dd>
          {j.checksum && (
            <>
              <dt>Checksum</dt>
              <dd className="selectable">
                {j.checksum.algo.toUpperCase()} {j.checksum.expected}
              </dd>
            </>
          )}
        </dl>
        {j.error && j.state !== "completed" && <div className="error-box">{j.error.message}</div>}
        {refreshing && (
          <div className="field" style={{ marginTop: 10 }}>
            <label htmlFor="newurl">New address for the same file</label>
            <div className="input-row">
              <input id="newurl" className="input" value={newUrl} onChange={(e) => setNewUrl(e.target.value)} placeholder="https://" autoFocus />
              <button
                className="btn primary"
                onClick={() =>
                  run(
                    api.updateJob(j.id, { url: newUrl.trim() }).then(() => {
                      setRefreshing(false);
                      return api.resume([j.id]);
                    }),
                  )
                }
              >
                Save and resume
              </button>
            </div>
            <span className="hint">CrowDM checks that the size and version still match before continuing.</span>
          </div>
        )}
      </div>
      <div className="segments">
        <span className="label">Transfer</span>
        <SegmentMap job={j} />
        <dl className="kv" style={{ gridTemplateColumns: "100px 1fr", marginTop: 4 }}>
          <dt>Connections</dt>
          <dd className="num">{j.state === "downloading" ? j.connections : "None"}</dd>
          <dt>Time left</dt>
          <dd className="num">{j.state === "downloading" ? fmt.duration(j.eta) || "Calculating" : "None"}</dd>
          <dt>Speed limit</dt>
          <dd>
            {editingLimit ? (
              <span className="input-row">
                <input
                  className="input"
                  style={{ width: 110, height: 28 }}
                  value={limitText}
                  placeholder="No limit"
                  onChange={(e) => setLimitText(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && saveLimit()}
                  autoFocus
                />
                <button className="btn" style={{ height: 28 }} onClick={saveLimit}>
                  Set
                </button>
              </span>
            ) : (
              <button className="disclosure" onClick={() => (setLimitText(fmt.rateText(j.speedLimit)), setEditingLimit(true))}>
                {j.speedLimit ? `${fmt.speed(j.speedLimit)}` : "No limit"} (change)
              </button>
            )}
          </dd>
        </dl>
      </div>
    </div>
  );
}

function SegmentMap({ job: j }: { job: JobSnapshot }) {
  const size = j.size;
  if (j.state === "completed") {
    return (
      <div className="segmap" aria-label="Download complete">
        <span style={{ left: 0, width: "100%" }} />
      </div>
    );
  }
  if (!size) {
    return <div className="segmap" aria-label="Size unknown" />;
  }
  return (
    <div className="segmap" aria-label="Downloaded parts of the file">
      {j.segments.map(([start, , done], i) =>
        done > 0 ? <span key={i} className="edge" style={{ left: `${(start / size) * 100}%`, width: `${Math.max(0.2, (done / size) * 100)}%` }} /> : null,
      )}
    </div>
  );
}
