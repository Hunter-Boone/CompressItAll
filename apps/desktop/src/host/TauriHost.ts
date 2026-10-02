/**
 * The desktop EngineHost (DESIGN.md 2.2, 4.3): every method is a Tauri command
 * in `src-tauri/src/commands/`, engine events arrive on `engine://event`,
 * FFmpeg setup progress on `ffmpeg://progress`. Files dropped on the window
 * come through Tauri's own drag-drop event (the HTML5 drop carries no paths
 * under Tauri 2), and are handed to the UI through `onExternalInputs`.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { openPath, openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { startDrag } from "@crabnebula/tauri-plugin-drag";
import type {
  AllowanceView,
  Capabilities,
  EngineEvent,
  EngineHost,
  FfmpegSetupProgress,
  InputItem,
  InputSource,
  JobHandle,
  JobSummary,
  LicenseInfo,
  Plan,
  PlanRequest,
} from "@cia/engine-client";

/** Tauri's IPC is JSON; the generated types use bigint for u64, so convert before sending. */
function plain<T>(value: T): T {
  return JSON.parse(JSON.stringify(value, (_k, v) => (typeof v === "bigint" ? Number(v) : v))) as T;
}

/** Errors from Rust commands arrive as plain strings; give the UI an Error with that message. */
async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw e instanceof Error ? e : new Error(typeof e === "string" ? e : JSON.stringify(e));
  }
}

function newJobId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

interface JobWaiter {
  onEvent: (e: EngineEvent) => void;
  resolve: (s: JobSummary) => void;
}

export class TauriHost implements EngineHost {
  readonly kind = "desktop" as const;
  private jobs = new Map<string, JobWaiter>();
  private externalInputs: ((sources: InputSource[]) => void) | null = null;
  private ffmpegProgress: ((p: FfmpegSetupProgress) => void) | null = null;
  private ready: Promise<void>;

  constructor() {
    this.ready = this.subscribe();
  }

  private async subscribe(): Promise<void> {
    const unlisteners: UnlistenFn[] = [];
    unlisteners.push(
      await listen<EngineEvent>("engine://event", ({ payload }) => {
        const w = this.jobs.get(payload.job_id);
        if (!w) return;
        w.onEvent(payload);
        if (payload.type === "job_done") {
          this.jobs.delete(payload.job_id);
          w.resolve(payload.summary);
        }
      }),
    );
    unlisteners.push(
      await listen<FfmpegSetupProgress>("ffmpeg://progress", ({ payload }) => {
        this.ffmpegProgress?.(payload);
      }),
    );
    // Files dropped on the window (DESIGN.md 4.3). Folders are expanded in Rust by addInputs.
    unlisteners.push(
      await getCurrentWebview().onDragDropEvent((event) => {
        if (event.payload.type === "drop" && event.payload.paths.length) {
          this.externalInputs?.(event.payload.paths.map((path) => ({ kind: "path" as const, path })));
        }
      }),
    );
    // `debug_add_paths` (dev-build only) feeds the e2e suite through the same path as a real drop.
    unlisteners.push(
      await listen<string[]>("smidge://add-paths", ({ payload }) => {
        if (payload.length) this.externalInputs?.(payload.map((path) => ({ kind: "path" as const, path })));
      }),
    );
    window.addEventListener("beforeunload", () => unlisteners.forEach((u) => u()));
  }

  onExternalInputs(cb: (sources: InputSource[]) => void): () => void {
    this.externalInputs = cb;
    return () => {
      if (this.externalInputs === cb) this.externalInputs = null;
    };
  }

  capabilities(): Promise<Capabilities> {
    return call<Capabilities>("capabilities");
  }

  async addInputs(inputs: InputSource[]): Promise<InputItem[]> {
    const paths = inputs.flatMap((s) => (s.kind === "path" ? [s.path] : []));
    if (!paths.length) return [];
    return call<InputItem[]>("add_paths", { paths });
  }

  removeInput(itemId: string): void {
    void call("remove_input", { itemId });
  }

  preview(req: PlanRequest): Promise<Plan> {
    return call<Plan>("preview", { req: plain(req) });
  }

  async run(req: PlanRequest, onEvent: (e: EngineEvent) => void): Promise<JobHandle> {
    await this.ready;
    const jobId = newJobId();
    const done = new Promise<JobSummary>((resolve) => this.jobs.set(jobId, { onEvent, resolve }));
    try {
      await call("run", { jobId, req: plain(req) });
    } catch (e) {
      this.jobs.delete(jobId);
      throw e;
    }
    return { jobId, done };
  }

  cancel(jobId: string): Promise<void> {
    return call("cancel", { jobId });
  }

  private artifactPaths(ids: string[]): Promise<string[]> {
    return call<string[]>("artifact_paths", { ids });
  }

  actions: EngineHost["actions"] = {
    revealInFolder: async (id) => {
      const [p] = await this.artifactPaths([id]);
      if (p) await revealItemInDir(p);
    },
    openFile: async (id) => {
      const [p] = await this.artifactPaths([id]);
      if (p) await openPath(p);
    },
    copyFilesToClipboard: async (ids) => {
      const paths = await this.artifactPaths(ids);
      if (paths.length) await call("copy_files", { paths });
    },
    startDragOut: async (ids) => {
      const paths = await this.artifactPaths(ids);
      if (!paths.length) return;
      const icon = await call<string>("drag_icon_path");
      await startDrag({ item: paths, icon });
    },
    chooseFiles: async () => {
      const picked = await open({ multiple: true, directory: false, title: "Choose files" });
      const paths = picked === null ? [] : Array.isArray(picked) ? picked : [picked];
      return paths.map((path) => ({ kind: "path" as const, path }));
    },
    chooseFolder: async () => {
      const picked = await open({ multiple: false, directory: true, title: "Choose a folder" });
      return picked ? [{ kind: "path" as const, path: picked }] : [];
    },
    chooseOutputFolder: async () => {
      const picked = await open({ multiple: false, directory: true, title: "Always save to" });
      return picked ?? null;
    },
    openExternal: async (url) => {
      await openUrl(url);
    },
    previewFrame: async (itemId, ms) => call<string | null>("preview_frame", { itemId, ms: Math.max(0, Math.round(ms)) }),
  };

  loadSettings(): Promise<Record<string, unknown>> {
    return call<Record<string, unknown>>("settings_load");
  }

  saveSettings(settings: Record<string, unknown>): Promise<void> {
    return call("settings_save", { settings: plain(settings) });
  }

  license(): Promise<LicenseInfo> {
    return call<LicenseInfo>("license_status");
  }

  activate(productKey: string): Promise<LicenseInfo> {
    return call<LicenseInfo>("license_activate", { productKey });
  }

  deactivate(): Promise<void> {
    return call("license_deactivate");
  }

  startPurchase(plan: "lifetime" | "yearly"): Promise<{ buyUrl: string; claimId: string }> {
    return call("license_start_purchase", { plan });
  }

  pollClaim(claimId: string): Promise<"pending" | "fulfilled" | "expired"> {
    return call("license_poll_claim", { claimId });
  }

  resendKey(email: string): Promise<void> {
    return call("license_resend", { email });
  }

  allowance(): Promise<AllowanceView> {
    return call<AllowanceView>("allowance");
  }

  ffmpegSetup: EngineHost["ffmpegSetup"] = {
    manifest: () => call("ffmpeg_manifest"),
    install: async (onProgress) => {
      this.ffmpegProgress = onProgress;
      try {
        await call("ffmpeg_install");
      } finally {
        this.ffmpegProgress = null;
      }
    },
    useOwnCopy: async () => {
      const picked = await open({ multiple: false, directory: false, title: "Choose the ffmpeg program" });
      if (!picked) return;
      await call("ffmpeg_use_own_copy", { path: picked });
    },
    remove: () => call("ffmpeg_remove"),
    retest: () => call("ffmpeg_retest"),
    cancel: () => call("ffmpeg_cancel"),
  };

  updates: EngineHost["updates"] = {
    check: () => call("updates_check"),
    install: () => call("updates_install"),
  };

  diagnostics: EngineHost["diagnostics"] = {
    copyReport: () => call("diagnostics_report"),
    openLogs: () => call("diagnostics_open_logs"),
    jobLog: (jobId) => call<string | null>("diagnostics_job_log", { jobId }),
  };

  version(): Promise<{ app: string; build: string; engine: string }> {
    return call("version");
  }
}
