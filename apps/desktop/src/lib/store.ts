import { create } from "zustand";
import { api, onEngine, type EngineEvent, type JobSnapshot, type Settings } from "./api";

export type Filter =
  | { kind: "all" }
  | { kind: "active" }
  | { kind: "queued" }
  | { kind: "paused" }
  | { kind: "failed" }
  | { kind: "completed" }
  | { kind: "category"; id: string };

export type SortKey = "added" | "oldest" | "name" | "nameDesc" | "size" | "sizeAsc" | "status" | "statusDesc" | "progress";

export const SORT_OPTIONS: [SortKey, string][] = [
  ["added", "Newest"], ["oldest", "Oldest"], ["name", "A–Z"], ["nameDesc", "Z–A"],
  ["size", "By size: large"], ["sizeAsc", "By size: small"],
  ["status", "By status: ascending"], ["statusDesc", "By status: descending"],
  ["progress", "By progress"],
];

export type SettingsSection = "general" | "appearance" | "folders" | "connections" | "browsers" | "torrents" | "batch" | "updates" | "queue" | "about";

interface State {
  jobs: Record<string, JobSnapshot>;
  /** Bumped when membership, names or states change (re-sort/filter). */
  listVersion: number;
  settings: Settings | null;
  queueRunning: boolean;
  filter: Filter;
  search: string;
  sort: SortKey;
  selected: string[];
  lastClicked: string | null;
  view: "downloads" | "settings" | "grabber";
  settingsSection: SettingsSection;
  openUpdates(): void;
  grabbedCount: number;
  setGrabbedCount(n: number): void;
  toast: { text: string; tone: "info" | "error" } | null;

  setFilter(f: Filter): void;
  setSearch(s: string): void;
  setSort(s: SortKey): void;
  select(ids: string[], clicked?: string | null): void;
  setView(v: State["view"]): void;
  setSettings(s: Settings): void;
  showToast(text: string, tone?: "info" | "error"): void;
  apply(e: EngineEvent): void;
  load(): Promise<void>;
}

let toastTimer: number | undefined;

export const useStore = create<State>((set, get) => ({
  jobs: {},
  listVersion: 0,
  settings: null,
  queueRunning: true,
  filter: { kind: "all" },
  search: "",
  sort: "added",
  selected: [],
  lastClicked: null,
  view: "downloads",
  settingsSection: "general",
  openUpdates: () => set({view: "settings", settingsSection: "updates"}),
  grabbedCount: 0,
  setGrabbedCount: (grabbedCount) => set({grabbedCount}),
  toast: null,

  setFilter: (filter) => set({ filter, view: "downloads" }),
  setSearch: (search) => set({ search }),
  setSort: (sort) => set({ sort, listVersion: get().listVersion + 1 }),
  select: (selected, clicked) => set({ selected, lastClicked: clicked === undefined ? get().lastClicked : clicked }),
  setView: (view) => set({ view }),
  setSettings: (settings) => set({ settings, listVersion: get().listVersion + 1 }),
  showToast: (text, tone = "info") => {
    set({ toast: { text, tone } });
    window.clearTimeout(toastTimer);
    toastTimer = window.setTimeout(() => set({ toast: null }), tone === "error" ? 6000 : 3000);
  },

  apply: (e) => {
    const s = get();
    switch (e.type) {
      case "progress": {
        const jobs = { ...s.jobs };
        for (const p of e.items) {
          const j = jobs[p.id];
          if (!j) continue;
          jobs[p.id] = {
            ...j,
            downloaded: p.downloaded,
            size: p.size,
            speed: p.speed,
            eta: p.eta,
            connections: p.connections,
            phase: p.phase,
            segments: p.segments,
          };
        }
        set({ jobs });
        break;
      }
      case "jobUpdated":
      case "jobCompleted":
      case "jobFailed": {
        const prev = s.jobs[e.job.id];
        const structural = !prev || prev.state !== e.job.state || prev.size !== e.job.size || prev.fileName !== e.job.fileName || prev.category !== e.job.category || prev.queueOrder !== e.job.queueOrder;
        set({ jobs: { ...s.jobs, [e.job.id]: e.job }, listVersion: structural ? s.listVersion + 1 : s.listVersion });
        break;
      }
      case "jobRemoved": {
        const jobs = { ...s.jobs };
        delete jobs[e.id];
        set({ jobs, listVersion: s.listVersion + 1, selected: s.selected.filter((x) => x !== e.id) });
        break;
      }
      case "queueChanged":
        set({ queueRunning: e.running });
        break;
      case "queueFinished":
        break;
    }
  },

  load: async () => {
    const [jobs, settings, info] = await Promise.all([api.listJobs(), api.getSettings(), api.appInfo()]);
    const map: Record<string, JobSnapshot> = {};
    for (const j of jobs) map[j.id] = j;
    set({ jobs: map, settings, queueRunning: info.queueRunning, listVersion: get().listVersion + 1 });
  },
}));

export function startEngineSync() {
  return onEngine((e) => useStore.getState().apply(e));
}

export function matchesFilter(j: JobSnapshot, f: Filter): boolean {
  switch (f.kind) {
    case "all":
      return true;
    case "active":
      return j.state === "downloading";
    case "queued":
      return j.state === "queued";
    case "paused":
      return j.state === "paused";
    case "failed":
      return j.state === "failed";
    case "completed":
      return j.state === "completed";
    case "category":
      return j.category === f.id;
  }
}

const stateRank: Record<string, number> = { downloading: 0, queued: 1, paused: 2, failed: 3, completed: 4 };

export function sortJobs(list: JobSnapshot[], key: SortKey): JobSnapshot[] {
  const out = [...list];
  switch (key) {
    case "added":
      out.sort((a, b) => b.createdAt - a.createdAt);
      break;
    case "oldest":
      out.sort((a, b) => a.createdAt - b.createdAt);
      break;
    case "name":
    case "nameDesc":
      out.sort((a, b) => a.fileName.localeCompare(b.fileName, undefined, { numeric: true, sensitivity: "base" }));
      if (key === "nameDesc") out.reverse();
      break;
    case "size":
    case "sizeAsc":
      out.sort((a, b) => a.size == null ? (b.size == null ? 0 : 1) : b.size == null ? -1 : key === "size" ? b.size - a.size : a.size - b.size);
      break;
    case "status":
    case "statusDesc":
      out.sort((a, b) => (key === "status" ? 1 : -1) * (stateRank[a.state] - stateRank[b.state]) || b.createdAt - a.createdAt);
      break;
    case "progress":
      out.sort((a, b) => (b.size ? b.downloaded / b.size : 0) - (a.size ? a.downloaded / a.size : 0));
      break;
  }
  return out;
}
