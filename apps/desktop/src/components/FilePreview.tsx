import { useEffect, useState } from "react";
import { api, type JobSnapshot } from "../lib/api";
import { FileIcon } from "./FileIcon";

// Virtualized rows can mount more than once. Share pending reads and cache
// failures so files without artwork are not decoded repeatedly.
const previews = new Map<string, Promise<string | null>>();
export function FilePreview({ job }: { job: JobSnapshot }) {
  const [preview, setPreview] = useState<{ key: string; url: string } | null>(null);
  const key = `${job.id}:${job.completedAt}`;
  useEffect(() => {
    if (job.state !== "completed" || !["images", "videos", "audio"].includes(job.category)) return;
    let disposed = false;
    let pending = previews.get(key);
    if (!pending) {
      pending = api.jobThumbnail(job.id).catch(() => null);
      if (previews.size > 500) previews.clear();
      previews.set(key, pending);
    }
    pending.then((url) => { if (!disposed && url) setPreview({ key, url }); });
    return () => { disposed = true; };
  }, [key, job.id, job.state, job.category]);
  return preview?.key === key ? <img className="file-preview" src={preview.url} alt="" /> : <FileIcon category={job.category} />;
}
