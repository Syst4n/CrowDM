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

export type SortKey = "added" | "name" | "size" | "progress";

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
  view: "downloads" | "settings";
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
        const structural = !prev || prev.state !== e.job.state || prev.fileName !== e.job.fileName || prev.category !== e.job.category || prev.queueOrder !== e.job.queueOrder;
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
      out.sort((a, b) => stateRank[a.state] - stateRank[b.state] || b.createdAt - a.createdAt);
      break;
    case "name":
      out.sort((a, b) => a.fileName.localeCompare(b.fileName, undefined, { numeric: true, sensitivity: "base" }));
      break;
    case "size":
      out.sort((a, b) => (b.size ?? -1) - (a.size ?? -1));
      break;
    case "progress":
      out.sort((a, b) => (b.size ? b.downloaded / b.size : 0) - (a.size ? a.downloaded / a.size : 0));
      break;
  }
  return out;
}
