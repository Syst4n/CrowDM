// Typed wrappers around the Rust commands and events.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type JobState = "queued" | "downloading" | "paused" | "completed" | "failed";
export type ErrorKind =
  | "network"
  | "serverBusy"
  | "accessDenied"
  | "notFound"
  | "http"
  | "resourceChanged"
  | "diskFull"
  | "io"
  | "checksumMismatch"
  | "invalidUrl";
export type SourceKind = "browserCapture" | "browserMenu" | "video" | "manual";

export interface Job {
  package?: string | null;
  torrent?: TorrentInfo | null;
  mediaDownload: boolean;
  id: string;
  url: string;
  finalUrl: string | null;
  referrer: string | null;
  headers: [string, string][];
  fileName: string;
  dir: string;
  category: string;
  size: number | null;
  resumable: boolean | null;
  etag: string | null;
  lastModified: string | null;
  mime: string | null;
  state: JobState;
  error: { kind: ErrorKind; message: string } | null;
  createdAt: number;
  startedAt: number | null;
  completedAt: number | null;
  queued: boolean;
  queueOrder: number;
  maxConnections: number | null;
  speedLimit: number | null;
  checksum: { algo: string; expected: string } | null;
  autoRetries: number;
  source: { kind: SourceKind; pageUrl: string | null; browser: string | null };
  downloaded: number;
  activeMs: number;
}

export interface JobSnapshot extends Job {
  speed: number;
  eta: number | null;
  connections: number;
  phase: string | null;
  segments: [number, number, number][];
}

export interface Progress {
  id: string;
  downloaded: number;
  size: number | null;
  speed: number;
  eta: number | null;
  connections: number;
  phase: string | null;
  segments: [number, number, number][];
}

export type EngineEvent =
  | { type: "jobUpdated"; job: JobSnapshot }
  | { type: "jobRemoved"; id: string }
  | { type: "progress"; items: Progress[] }
  | { type: "jobCompleted"; job: JobSnapshot }
  | { type: "jobFailed"; job: JobSnapshot }
  | { type: "queueChanged"; running: boolean }
  | { type: "queueFinished" };

export interface Category {
  id: string;
  name: string;
  folder: string;
  extensions: string[];
}

export type AfterQueue = "nothing" | "notify" | "exitApp" | "sleep" | "shutdown";
export type Theme = "system" | "light" | "dark" | "nightwing" | "ocean" | "forest" | "sand";
export type ColorMode = "system" | "light" | "dark";
export type Palette = "classic" | "nightwing" | "ocean" | "forest" | "sand" | "rose" | "violet" | "amber" | "slate" | "cherry";
export interface Appearance { mode: ColorMode; palette: Palette }

export interface Settings {
  batch: { monitorClipboard: boolean; groupParts: boolean; skipDuplicates: boolean; quietNotifications: boolean; checkConcurrency: number; maxLinks: number };
  updates: { checkAutomatically: boolean; dismissedVersion: string | null };
  torrents: { peerLimit: number; listenPort: number; enableUpnp: boolean; enableUtp: boolean; uploadLimit: number };
  downloadRoot: string;
  organizeByCategory: boolean;
  categories: Category[];
  maxConnectionsPerDownload: number;
  maxTotalConnections: number;
  maxActiveDownloads: number;
  speedLimit: number;
  retryAttempts: number;
  retryDelaySecs: number;
  resumeOnStartup: boolean;
  userAgent: string | null;
  afterQueue: AfterQueue;
  schedule: { enabled: boolean; startTime: string; stopTime: string | null; days: boolean[] };
  interception: {
    enabled: boolean;
    excludedSites: string[];
    excludedExtensions: string[];
    minSize: number;
    includePrivate: boolean;
    videoButton: boolean;
    skipWebPages: boolean;
  };
  theme: Theme;
  appearance: Appearance | null;
  animations: boolean;
  respectReducedMotion: boolean;
  closeToTray: boolean;
  startWithWindows: boolean;
  startMinimized: boolean;
  notifyOnComplete: boolean;
  confirmCaptured: boolean;
  prewarmDialog: boolean;
  soundOnComplete: boolean;
  power: { preventSleepActive: boolean; preventSleepScheduled: boolean; allowSleepResumable: boolean };
}

export interface TorrentInfo {
  infoHash: string;
  files: {index:number;path:string;size:number}[];
  onlyFiles: number[] | null;
  multiFile: boolean;
  private: boolean;
}

export interface ProbeInfo {
  torrent: TorrentInfo | null;
  sizeEstimated: boolean;
  mediaOptions: { format: string; label: string; height: number | null; container: string; size: number | null }[];
  probeId: string | null;
  finalUrl: string;
  fileName: string;
  size: number | null;
  resumable: boolean;
  mime: string | null;
  category: string;
  dir: string;
  exists: boolean;
  duplicateOf: string | null;
  warnings: string[];
  status: number;
  looksLikeHtml: boolean;
}

export type CaptureKind = "intercepted" | "menu" | "video" | "manual";

export interface CaptureView {
  mediaFormat: string | null;
  key: string;
  kind: CaptureKind;
  browser: string | null;
  url: string;
  finalUrl: string | null;
  pageUrl: string | null;
  pageTitle: string | null;
  fileName: string | null;
  size: number | null;
  mime: string | null;
  probe: ProbeInfo | null;
  probeError: string | null;
  probing: boolean;
  browserRecommended: string | null;
  browserWaiting: boolean;
}

export interface AcceptOptions {
  onlyFiles?: number[];
  fileName?: string;
  dir?: string;
  category?: string;
  start: boolean;
  maxConnections?: number;
  speedLimit?: number;
  checksum?: string;
}

export interface JobPatch {
  fileName?: string;
  dir?: string;
  speedLimit?: number | null;
  maxConnections?: number | null;
  url?: string;
  queued?: boolean;
}

export interface BrowserStatus {
  integration: {
    hostPath: string;
    hostExists: boolean;
    browsers: { browser: string; registered: boolean; manifest: string | null }[];
    extensionsDir: string | null;
  };
  connected: { browser: string; extensionVersion: string }[];
}

export interface AppInfo {
  version: string;
  dataDir: string;
  portable: boolean;
  queueRunning: boolean;
}

export const api = {
  batchList: () => invoke<LinkInbox>("batch_list"),
  batchTakeFocus: () => invoke<boolean>("batch_take_focus"),
  batchAnalyze: (text: string) => invoke<void>("batch_analyze", { text }),
  batchCollectPage: (url: string) => invoke<void>("batch_collect_page", { url }),
  batchStop: () => invoke<void>("batch_stop"),
  batchRemove: (ids: string[]) => invoke<void>("batch_remove", { ids }),
  batchAccept: (selected: {id: string; package: string}[], root: string, group: boolean) => invoke<{ids: string[]; errors: string[]}>("batch_accept", {selected, root, group}),
  checkUpdate: () => invoke<AnnouncedUpdate | null>("check_update"),
  downloadUpdate: () => invoke<string>("download_update"),
  installUpdate: (id: string) => invoke<void>("install_update", { id }),
  downloadCounts: () => invoke<DownloadCounts>("download_counts"),
  appInfo: () => invoke<AppInfo>("app_info"),
  testNotification: () => invoke<void>("test_notification"),
  listJobs: () => invoke<JobSnapshot[]>("list_jobs"),
  jobThumbnail: (id: string) => invoke<string | null>("job_thumbnail", { id }),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),

  captureList: () => invoke<CaptureView[]>("capture_list"),
  captureNew: (url?: string) => invoke<string>("capture_new", { url: url ?? null }),
  captureSetUrl: (key: string, url: string) => invoke<void>("capture_set_url", { key, url }),
  captureSetMediaFormat: (key: string, format: string) => invoke<void>("capture_set_media_format", { key, format }),
  captureAccept: (key: string, options: AcceptOptions) => invoke<string>("capture_accept", { key, options }),
  captureToBrowser: (key: string) => invoke<void>("capture_to_browser", { key }),
  captureRendered: (key: string) => invoke<void>("capture_rendered", { key }),
  captureDismiss: (key: string) => invoke<void>("capture_dismiss", { key }),
  categoryDir: (category: string) => invoke<string>("category_dir", { category }),
  showMainWindow: () => invoke<void>("show_main_window"),

  pause: (ids: string[]) => invoke<void>("pause_jobs", { ids }),
  resume: (ids: string[]) => invoke<void>("resume_jobs", { ids }),
  remove: (ids: string[], deleteFiles: boolean) => invoke<void>("remove_jobs", { ids, deleteFiles }),
  restart: (id: string) => invoke<void>("restart_job", { id }),
  updateJob: (id: string, patch: JobPatch) => invoke<void>("update_job", { id, patch }),
  setQueued: (ids: string[], queued: boolean) => invoke<void>("set_queued", { ids, queued }),
  openFile: (id: string) => invoke<void>("open_job_file", { id }),
  reveal: (id: string) => invoke<void>("reveal_job", { id }),
  openFolder: (path: string) => invoke<void>("open_folder", { path }),
  pauseAll: () => invoke<void>("pause_all"),
  resumeAll: () => invoke<void>("resume_all"),
  setQueueRunning: (running: boolean) => invoke<void>("set_queue_running", { running }),

  browserStatus: () => invoke<BrowserStatus>("browser_status"),
  registerBrowsers: () => invoke<BrowserStatus>("register_browsers"),
  registerTorrentHandlers: () => invoke<void>("register_torrent_handlers"),
  chooseTorrentDefaults: () => invoke<void>("choose_torrent_defaults"),
  openExtensionsFolder: () => invoke<void>("open_extensions_folder"),
  openDataFolder: () => invoke<void>("open_data_folder"),
  memoryUsage: () => invoke<{ pid: number; name: string; privateBytes: number }[]>("memory_usage"),
};

export interface GrabbedLink { id: string; url: string; fileName: string; package: string; status: "checking"|"unchecked"|"ready"|"error"|"duplicate"; message: string|null; probe: ProbeInfo|null }
export interface LinkInbox { links: GrabbedLink[]; checking: boolean }
export interface AnnouncedUpdate { version: string; whatsNew: string[]; installerUrl: string; sha256: string; extensionVersion: string }
export interface DownloadCounts { total: number; versions: {version: string; downloads: number}[] }

export function onEngine(cb: (e: EngineEvent) => void): Promise<UnlistenFn> {
  return listen<EngineEvent>("engine", (e) => cb(e.payload));
}

export function onCaptures(cb: (c: CaptureView[]) => void): Promise<UnlistenFn> {
  return listen<CaptureView[]>("captures-changed", (e) => cb(e.payload));
}

export function onGrabberRequested(cb: () => void): Promise<UnlistenFn> { return listen("open-link-grabber", cb); }

export function onBrowsers(cb: (c: { browser: string; extensionVersion: string }[]) => void): Promise<UnlistenFn> {
  return listen("browsers-changed", (e) => cb(e.payload as { browser: string; extensionVersion: string }[]));
}

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) return String((e as { message: unknown }).message);
  return "Something went wrong.";
}
