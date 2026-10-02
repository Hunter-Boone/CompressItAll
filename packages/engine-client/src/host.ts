/**
 * The EngineHost interface (DESIGN.md 2.4). The UI talks only to this; the
 * desktop app implements it over Tauri commands, the web app over a worker
 * pool running the WASM engine, and `mock.ts` fakes it for browser dev and tests.
 */
import type {
  Capabilities,
  EngineEvent,
  Goal,
  InputItem,
  JobOptions,
  JobSummary,
  Packaging,
  Plan,
} from "./generated";

/** What a host accepts as an input: a path (desktop) or a File / handle (web). */
export type InputSource =
  | { kind: "path"; path: string }
  | { kind: "file"; file: File; relPath?: string }
  | { kind: "directory"; handle: FileSystemDirectoryHandle }
  | { kind: "entry"; entry: FileSystemEntry };

export interface PlanRequest {
  items: InputItem[];
  goal: Goal;
  packaging: Packaging;
  options: JobOptions;
}

export interface JobHandle {
  jobId: string;
  /** Resolves with the summary when the job ends (done, refused, failed or cancelled). */
  done: Promise<JobSummary>;
}

export interface PlatformActions {
  revealInFolder?(artifactId: string): Promise<void>;
  copyFilesToClipboard?(artifactIds: string[]): Promise<void>;
  openFile?(artifactId: string): Promise<void>;
  startDragOut?(artifactIds: string[]): Promise<void>;
  download?(artifactIds: string[]): Promise<void>;
  saveToFolder?(artifactIds: string[]): Promise<void>;
  chooseFiles?(): Promise<InputSource[]>;
  chooseFolder?(): Promise<InputSource[]>;
  chooseOutputFolder?(): Promise<string | null>;
  openExternal?(url: string): Promise<void>;
  /** Desktop: extract a preview frame at `ms` as a blob URL. Web: handled in-worker. */
  previewFrame?(itemId: string, ms: number): Promise<string | null>;
}

export interface LicenseInfo {
  status: "free" | "pro" | "needs_online" | "ended" | "invalid";
  plan?: "lifetime" | "yearly";
  key4?: string;
  keyMasked?: string;
  devicesUsed?: number;
  devicesMax?: number;
  accessUntil?: number | null;
  message?: string;
}

export interface AllowanceView {
  remaining: number;
  nextFreeAtMs: number | null;
}

export interface FfmpegSetupProgress {
  phase: "downloading" | "checking" | "testing" | "ready" | "error";
  downloadedBytes?: number;
  totalBytes?: number;
  message?: string;
}

export interface EngineHost {
  readonly kind: "desktop" | "web";
  capabilities(): Promise<Capabilities>;
  addInputs(inputs: InputSource[]): Promise<InputItem[]>;
  removeInput(itemId: string): void;
  preview(req: PlanRequest): Promise<Plan>;
  run(req: PlanRequest, onEvent: (e: EngineEvent) => void): Promise<JobHandle>;
  cancel(jobId: string): Promise<void>;
  actions: PlatformActions;

  /** Settings are a versioned JSON blob; the host owns persistence. */
  loadSettings(): Promise<Record<string, unknown>>;
  saveSettings(settings: Record<string, unknown>): Promise<void>;

  license(): Promise<LicenseInfo>;
  activate(productKey: string): Promise<LicenseInfo>;
  deactivate(): Promise<void>;
  startPurchase(plan: "lifetime" | "yearly"): Promise<{ buyUrl: string; claimId: string }>;
  pollClaim(claimId: string): Promise<"pending" | "fulfilled" | "expired">;
  resendKey(email: string): Promise<void>;

  allowance(): Promise<AllowanceView>;

  /** Desktop only. */
  ffmpegSetup?: {
    manifest(): Promise<{ version: string; bytes: number; unpackedBytes: number }>;
    install(onProgress: (p: FfmpegSetupProgress) => void): Promise<void>;
    useOwnCopy(): Promise<void>;
    remove(): Promise<void>;
    retest(): Promise<void>;
    cancel(): Promise<void>;
  };

  /** Desktop only. */
  updates?: {
    check(): Promise<{ available: boolean; version?: string; notes?: string }>;
    install(): Promise<void>;
  };

  diagnostics?: {
    copyReport(): Promise<void>;
    openLogs(): Promise<void>;
    jobLog(jobId: string): Promise<string | null>;
  };

  version(): Promise<{ app: string; build: string; engine: string }>;

  /**
   * Desktop: files arriving from outside the DOM (an OS drop on the window, a
   * test hook). The UI subscribes once and feeds the sources to `addInputs`.
   * Returns the unsubscribe function.
   */
  onExternalInputs?(cb: (sources: InputSource[]) => void): () => void;
}
