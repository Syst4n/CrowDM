const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function bytes(n: number | null | undefined): string {
  if (n == null) return "Unknown size";
  if (n < 1024) return `${n} B`;
  let v = n;
  let u = 0;
  while (v >= 1024 && u < UNITS.length - 1) {
    v /= 1024;
    u++;
  }
  return `${v >= 100 ? v.toFixed(0) : v >= 10 ? v.toFixed(1) : v.toFixed(2)} ${UNITS[u]}`;
}

export function speed(bps: number): string {
  if (!bps) return "";
  return `${bytes(bps)}/s`;
}

export function duration(secs: number | null | undefined): string {
  if (secs == null || !isFinite(secs)) return "";
  if (secs < 1) return "less than a second";
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = Math.floor(secs % 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}

export function percent(done: number, size: number | null): number | null {
  if (!size) return null;
  return Math.min(100, (done / size) * 100);
}

export function when(ms: number | null | undefined): string {
  if (!ms) return "";
  const d = new Date(ms);
  const now = new Date();
  const sameDay = d.toDateString() === now.toDateString();
  const y = new Date(now);
  y.setDate(now.getDate() - 1);
  const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (sameDay) return time;
  if (d.toDateString() === y.toDateString()) return `Yesterday ${time}`;
  return d.toLocaleDateString([], { day: "numeric", month: "short", year: d.getFullYear() === now.getFullYear() ? undefined : "numeric" });
}

export function host(url: string | null | undefined): string {
  if (!url) return "";
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/** Parses "2 MB", "500k", "1.5 mb/s" into bytes; empty means 0. */
export function parseRate(text: string): number | null {
  const t = text.trim().toLowerCase().replace(/\/s$/, "").trim();
  if (!t) return 0;
  const m = t.match(/^(\d+(?:\.\d+)?)\s*(b|k|kb|m|mb|g|gb)?$/);
  if (!m) return null;
  const v = parseFloat(m[1]);
  const unit = m[2] ?? "kb";
  const mult = unit.startsWith("g") ? 1024 ** 3 : unit.startsWith("m") ? 1024 ** 2 : unit.startsWith("k") ? 1024 : 1;
  return Math.round(v * mult);
}

export function rateText(bps: number | null | undefined): string {
  if (!bps) return "";
  if (bps >= 1024 * 1024) return `${+(bps / 1024 / 1024).toFixed(2)} MB`;
  return `${Math.round(bps / 1024)} KB`;
}
