/**
 * WebHost: the EngineHost for the browser (DESIGN.md 2.2, 2.5, 3.10).
 * Inputs are File objects; the engine runs in a pool of Web Workers (pool.ts);
 * outputs live in OPFS under /jobs/<job_id>/ and leave as downloads or, on
 * Chromium, through the File System Access API. Settings, the license token
 * and the free allowance are in IndexedDB.
 */
import { get, set } from "idb-keyval";
import type {
  AllowanceView,
  Artifact,
  Capabilities,
  EngineEvent,
  EngineHost,
  InputItem,
  InputSource,
  JobHandle,
  JobSummary,
  LicenseInfo,
  PlanRequest,
  Plan,
  WebCapabilities,
} from "@cia/engine-client";
import engineVersion from "../engine-version.json";
import { EngineWorkerError, WorkerPool, type EngineWorker } from "./pool";
import { LicenseClient, browserName } from "./license";
import type { FileIn, RunResult, ZipEntryIn } from "../workers/engine.worker";

const APP_VERSION = (import.meta.env.VITE_APP_VERSION as string | undefined) ?? "0.1.0";
const KEY_SETTINGS = "cia.settings";
const KEY_ALLOWANCE = "cia.allowance";
const KEY_JOBS = "cia.jobs";
const FREE_FILES_PER_DAY = 3;
const WINDOW_MS = 24 * 60 * 60 * 1000;
const JOB_TTL_MS = 24 * 60 * 60 * 1000;

interface Held { file: File; relPath: string; folder: string | null }

let counter = 0;
function newId(): string {
  return `f_${Date.now().toString(36)}_${(counter++).toString(36)}_${Math.random().toString(36).slice(2, 8)}`;
}

async function probeWebCaps(): Promise<WebCapabilities> {
  const w = window as Window & { VideoEncoder?: { isConfigSupported(c: object): Promise<{ supported?: boolean }> }; AudioEncoder?: { isConfigSupported(c: object): Promise<{ supported?: boolean }> }; showDirectoryPicker?: unknown };
  const sup = async (p: Promise<{ supported?: boolean }> | undefined) => {
    try { return !!(await p)?.supported; } catch { return false; }
  };
  const video = (codec: string) => w.VideoEncoder ? sup(w.VideoEncoder.isConfigSupported({ codec, width: 1920, height: 1080, bitrate: 5_000_000, framerate: 30 })) : Promise.resolve(false);
  const audio = (codec: string) => w.AudioEncoder ? sup(w.AudioEncoder.isConfigSupported({ codec, sampleRate: 48_000, numberOfChannels: 2, bitrate: 128_000 })) : Promise.resolve(false);
  const [h264, vp9, aac, opus] = await Promise.all([video("avc1.640028"), video("vp09.00.40.08"), audio("mp4a.40.2"), audio("opus")]);
  const nav = navigator as Navigator & { storage?: { getDirectory?: unknown } };
  return {
    cross_origin_isolated: window.crossOriginIsolated === true,
    webcodecs: !!w.VideoEncoder,
    h264_encode: h264,
    vp9_encode: vp9,
    aac_encode: aac,
    opus_encode: opus,
    hdr_verified: false,
    directory_picker: typeof w.showDirectoryPicker === "function",
    opfs: typeof nav.storage?.getDirectory === "function",
    browser: browserName(),
  };
}

async function opfsRoot(): Promise<FileSystemDirectoryHandle | null> {
  try {
    const nav = navigator as Navigator & { storage?: { getDirectory?: () => Promise<FileSystemDirectoryHandle> } };
    return nav.storage?.getDirectory ? await nav.storage.getDirectory() : null;
  } catch {
    return null;
  }
}

async function opfsFile(path: string): Promise<File | null> {
  const root = await opfsRoot();
  if (!root) return null;
  const parts = path.split("/").filter(Boolean);
  const name = parts.pop()!;
  try {
    let d = root;
    for (const s of parts) d = await d.getDirectoryHandle(s);
    return await (await d.getFileHandle(name)).getFile();
  } catch {
    return null;
  }
}

/** Collision numbering from DESIGN.md 3.10: "<stem> (<Label>).ext" -> "<stem> (<Label> 2).ext". */
function numbered(name: string, n: number): string {
  const m = /^(.*\([^()]*)\)(\.[^.]*)?$/.exec(name);
  if (m) return `${m[1]} ${n})${m[2] ?? ""}`;
  const dot = name.lastIndexOf(".");
  return dot > 0 ? `${name.slice(0, dot)} ${n}${name.slice(dot)}` : `${name} ${n}`;
}

function triggerDownload(blob: Blob, name: string) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.rel = "noopener";
  document.body.appendChild(a);
  a.click();
  a.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 60_000);
}

export class WebHost implements EngineHost {
  readonly kind = "web" as const;
  private pool: Promise<WorkerPool>;
  private webCaps: Promise<WebCapabilities>;
  private held = new Map<string, Held>();
  private items = new Map<string, InputItem>();
  /** artifact id -> artifact, from the last finished job */
  private artifacts = new Map<string, Artifact>();
  private blobs = new Map<string, Blob>();
  private cancelFlags = new Map<string, Int32Array>();
  private lic = new LicenseClient();
  private lastJobId: string | null = null;

  constructor() {
    this.webCaps = probeWebCaps();
    this.pool = this.webCaps.then((web) => new WorkerPool(web));
    void this.cleanOldJobs();
  }

  async capabilities(): Promise<Capabilities> {
    return (await this.pool).capabilities();
  }

  // ---- inputs ----------------------------------------------------------------

  async addInputs(inputs: InputSource[]): Promise<InputItem[]> {
    const files: { id: string; held: Held }[] = [];
    for (const src of inputs) {
      for (const h of await this.expand(src)) files.push({ id: newId(), held: h });
    }
    if (!files.length) return [];
    for (const f of files) this.held.set(f.id, f.held);
    const pool = await this.pool;
    // Fan inspect out across workers, a few files per batch.
    const batchSize = Math.max(1, Math.ceil(files.length / pool.size));
    const batches: typeof files[] = [];
    for (let i = 0; i < files.length; i += batchSize) batches.push(files.slice(i, i + batchSize));
    const results = await Promise.all(batches.map(async (batch) => {
      const w = pool.next();
      await this.ensureFiles(w, batch.map((b) => b.id));
      return w.call<InputItem[]>({ type: "inspect", specs: batch.map((b) => ({ id: b.id, rel_path: b.held.relPath, folder: b.held.folder })) });
    }));
    const items = results.flat();
    // The engine mints its own item ids; key our file map by them.
    for (const [i, item] of items.entries()) {
      const f = files[i]!;
      if (item.source.type === "handle" && item.source.handle_id !== item.id) {
        // Keep the handle id (what the worker stores bytes under) and remember the item.
        this.held.set(item.id, f.held);
      }
      this.items.set(item.id, item);
    }
    return items;
  }

  private async expand(src: InputSource): Promise<Held[]> {
    switch (src.kind) {
      case "file": {
        const rel = src.relPath ?? src.file.name;
        return [{ file: src.file, relPath: rel, folder: rel.includes("/") ? rel.split("/")[0]! : null }];
      }
      case "directory": {
        const out: Held[] = [];
        const walk = async (dir: FileSystemDirectoryHandle, prefix: string) => {
          const entries = (dir as FileSystemDirectoryHandle & { entries(): AsyncIterable<[string, FileSystemHandle]> }).entries();
          for await (const [name, h] of entries) {
            if (name.startsWith(".")) continue;
            if (h.kind === "file") out.push({ file: await (h as FileSystemFileHandle).getFile(), relPath: `${prefix}/${name}`, folder: src.handle.name });
            else await walk(h as FileSystemDirectoryHandle, `${prefix}/${name}`);
          }
        };
        await walk(src.handle, src.handle.name);
        return out;
      }
      case "entry": {
        const out: Held[] = [];
        const walk = async (entry: FileSystemEntry, prefix: string, folder: string | null) => {
          if (entry.isFile) {
            const file = await new Promise<File>((res, rej) => (entry as FileSystemFileEntry).file(res, rej));
            if (!file.name.startsWith(".")) out.push({ file, relPath: prefix ? `${prefix}/${file.name}` : file.name, folder });
          } else if (entry.isDirectory) {
            const reader = (entry as FileSystemDirectoryEntry).createReader();
            for (;;) {
              const batch = await new Promise<FileSystemEntry[]>((res, rej) => reader.readEntries(res, rej));
              if (!batch.length) break;
              const p = prefix ? `${prefix}/${entry.name}` : entry.name;
              for (const e of batch) await walk(e, p, folder ?? entry.name);
            }
          }
        };
        await walk(src.entry, "", null);
        return out;
      }
      case "path":
        return [];
    }
  }

  /** Post the File objects a worker is missing (structured clone of a File is a reference, not a copy). */
  private async ensureFiles(w: EngineWorker, ids: string[]) {
    const missing: FileIn[] = [];
    for (const id of ids) {
      if (w.files.has(id)) continue;
      const h = this.heldFor(id);
      if (h) missing.push({ id: this.handleId(id), name: h.file.name, file: h.file });
    }
    if (!missing.length) return;
    await w.call({ type: "add_files", files: missing });
    for (const m of missing) w.files.add(m.id);
    for (const id of ids) w.files.add(id);
  }

  private heldFor(itemOrHandleId: string): Held | undefined {
    return this.held.get(itemOrHandleId);
  }

  private handleId(itemId: string): string {
    const item = this.items.get(itemId);
    return item?.source.type === "handle" ? item.source.handle_id : itemId;
  }

  removeInput(itemId: string): void {
    const handle = this.handleId(itemId);
    this.held.delete(itemId);
    this.held.delete(handle);
    this.items.delete(itemId);
    void this.pool.then((p) => { for (const w of p.all()) { w.files.delete(handle); w.files.delete(itemId); void w.call({ type: "remove_files", ids: [handle] }).catch(() => {}); } });
  }

  // ---- plan and run ------------------------------------------------------------

  async preview(req: PlanRequest): Promise<Plan> {
    const pool = await this.pool;
    const w = pool.job();
    await w.ready;
    await this.ensureFiles(w, req.items.map((i) => i.id));
    return w.call<Plan>({ type: "preview", req });
  }

  async run(req: PlanRequest, onEvent: (e: EngineEvent) => void): Promise<JobHandle> {
    const pool = await this.pool;
    const w = pool.job();
    await w.ready;
    await this.ensureFiles(w, req.items.map((i) => i.id));
    const shared = typeof SharedArrayBuffer === "function" && window.crossOriginIsolated ? new Int32Array(new SharedArrayBuffer(4)) : null;
    const pending = `pending_${Date.now()}`;
    let jobId = pending;
    if (shared) this.cancelFlags.set(pending, shared);
    const started = Date.now();
    const done = w.call<RunResult>({ type: "run", req, cancel: shared }, (e) => {
      if (jobId === pending) {
        jobId = e.job_id;
        if (shared) { this.cancelFlags.set(jobId, shared); this.cancelFlags.delete(pending); }
      }
      onEvent(e);
    }).then(async (r) => {
      for (const [path, blob] of Object.entries(r.blobs)) this.blobs.set(path, blob);
      this.remember(r.summary);
      await this.recordJob(r.summary.job_id);
      if ((await this.lic.info()).status !== "pro") await this.consumeAllowance(r.summary);
      return r.summary;
    }, (err: unknown) => {
      const crashed = err instanceof EngineWorkerError && err.fatal;
      const message = crashed ? "Smidge ran out of memory on this file. Try the desktop app." : `Something went wrong: ${err instanceof Error ? err.message : String(err)}`;
      const summary = this.failedSummary(req, jobId === pending ? `job_${started.toString(36)}` : jobId, message, started);
      onEvent({ type: "job_done", job_id: summary.job_id, summary });
      return summary;
    }).finally(() => { this.cancelFlags.delete(jobId); this.cancelFlags.delete(pending); });
    // Wait for the engine's job id (first event) so the handle carries it.
    const handle = await new Promise<JobHandle>((resolve) => {
      const tick = () => { if (jobId !== pending) resolve({ jobId, done }); else setTimeout(tick, 5); };
      tick();
      void done.then(() => resolve({ jobId, done }));
    });
    this.lastJobId = handle.jobId;
    return handle;
  }

  private failedSummary(req: PlanRequest, jobId: string, message: string, started: number): JobSummary {
    const failure = { code: "encoder_crash", message, closest_bytes: null };
    const input = req.items.reduce((a, i) => a + i.bytes, 0n);
    return {
      job_id: jobId,
      outcomes: req.items.map((i) => [i.id, { type: "failed", failure }]),
      packaged: null,
      input_bytes: input,
      total_bytes: 0n,
      verdict: "none_fit",
      headline: "Smidge couldn't finish this job.",
      elapsed_ms: BigInt(Date.now() - started),
    };
  }

  private remember(summary: JobSummary) {
    this.artifacts.clear();
    for (const [, o] of summary.outcomes) if (o.type === "fitted" || o.type === "kept_original") this.artifacts.set(o.artifact.id, o.artifact);
    if (summary.packaged) this.artifacts.set(summary.packaged.id, summary.packaged);
  }

  async cancel(jobId: string): Promise<void> {
    const flag = this.cancelFlags.get(jobId) ?? [...this.cancelFlags.values()][0];
    if (flag) Atomics.store(flag, 0, 1);
    else (await this.pool).restartJobWorker(); // no SharedArrayBuffer: stop the worker outright
  }

  // ---- outputs -------------------------------------------------------------------

  private async blobFor(a: Artifact): Promise<Blob | null> {
    const path = a.location.path;
    if (path.startsWith("source:")) return this.heldFor(a.item_id)?.file ?? this.held.get(path.slice(7))?.file ?? null;
    return this.blobs.get(path) ?? (await opfsFile(path));
  }

  actions: EngineHost["actions"] = {
    download: async (ids: string[]) => {
      const arts = ids.map((id) => this.artifacts.get(id)).filter((a): a is Artifact => !!a);
      if (arts.length === 1) {
        const blob = await this.blobFor(arts[0]!);
        if (blob) triggerDownload(blob, arts[0]!.file_name);
        return;
      }
      // Several files: one zip named "<folder> (<Label>).zip" (3.10), entries relative to the output folder.
      const entries: ZipEntryIn[] = [];
      const rels = arts.map((a) => (a.location.path.startsWith("source:") ? a.file_name : a.location.path.replace(/^\/jobs\/[^/]+\//, "")));
      const top = rels[0]?.split("/")[0];
      const shared = !!top && rels.every((r) => r.startsWith(top + "/"));
      for (const [i, a] of arts.entries()) {
        const name = shared ? rels[i]!.slice(top!.length + 1) : rels[i]!;
        const blob = this.blobs.get(a.location.path) ?? (a.location.path.startsWith("source:") ? await this.blobFor(a) : undefined) ?? undefined;
        entries.push({ name, path: a.location.path, blob: blob ?? undefined });
      }
      const w = (await this.pool).next();
      const bytes = await w.call<Uint8Array<ArrayBuffer>>({ type: "package_zip", entries });
      const first = this.items.get(arts[0]!.item_id);
      const zipName = shared ? `${top}.zip` : `${first?.folder ?? "Smidge files"}.zip`;
      triggerDownload(new Blob([bytes], { type: "application/zip" }), zipName);
    },
    saveToFolder: "showDirectoryPicker" in window ? async (ids: string[]) => {
      const picker = (window as unknown as { showDirectoryPicker(o?: object): Promise<FileSystemDirectoryHandle> }).showDirectoryPicker;
      let dir: FileSystemDirectoryHandle;
      try { dir = await picker({ mode: "readwrite", id: "smidge-save" }); } catch { return; }
      for (const id of ids) {
        const a = this.artifacts.get(id);
        if (!a) continue;
        const blob = await this.blobFor(a);
        if (!blob) continue;
        let name = a.file_name;
        for (let n = 2; ; n++) {
          try { await dir.getFileHandle(name); name = numbered(a.file_name, n); } catch { break; }
        }
        const fh = await dir.getFileHandle(name, { create: true });
        const w = await fh.createWritable();
        await w.write(blob);
        await w.close();
      }
    } : undefined,
    openExternal: async (url: string) => { window.open(url, "_blank", "noopener"); },
  };

  // ---- settings, license, allowance -----------------------------------------------

  async loadSettings(): Promise<Record<string, unknown>> {
    return (await get<Record<string, unknown>>(KEY_SETTINGS)) ?? {};
  }
  async saveSettings(settings: Record<string, unknown>): Promise<void> {
    await set(KEY_SETTINGS, settings);
  }

  license(): Promise<LicenseInfo> { return this.lic.info(); }
  activate(productKey: string): Promise<LicenseInfo> { return this.lic.activate(productKey, APP_VERSION); }
  deactivate(): Promise<void> { return this.lic.deactivate(); }
  startPurchase(): Promise<{ buyUrl: string; claimId: string }> { return this.lic.startPurchase(); }
  pollClaim(claimId: string) { return this.lic.pollClaim(claimId); }
  resendKey(email: string): Promise<void> { return this.lic.resend(email); }

  /** Free allowance: 3 fitted files per rolling 24 h (cia_core::allowance, DESIGN.md 5.2). */
  async allowance(): Promise<AllowanceView> {
    const uses = await this.pruneUses();
    const remaining = Math.max(0, FREE_FILES_PER_DAY - uses.length);
    return { remaining, nextFreeAtMs: remaining === 0 && uses[0] !== undefined ? uses[0] + WINDOW_MS : null };
  }
  private async pruneUses(): Promise<number[]> {
    const now = Date.now();
    const uses = ((await get<number[]>(KEY_ALLOWANCE)) ?? []).filter((t) => t + WINDOW_MS > now && t <= now + 60_000).sort((a, b) => a - b);
    await set(KEY_ALLOWANCE, uses);
    return uses;
  }
  private async consumeAllowance(summary: JobSummary) {
    const uses = await this.pruneUses();
    const now = Date.now();
    for (const [, o] of summary.outcomes) if (o.type === "fitted" && uses.length < FREE_FILES_PER_DAY) uses.push(now);
    await set(KEY_ALLOWANCE, uses);
  }

  // ---- housekeeping -------------------------------------------------------------------

  private async recordJob(jobId: string) {
    const jobs = (await get<Record<string, number>>(KEY_JOBS)) ?? {};
    jobs[jobId] = Date.now();
    await set(KEY_JOBS, jobs);
  }
  /** Delete OPFS job folders older than 24 h (DESIGN.md 2.5). */
  private async cleanOldJobs() {
    const root = await opfsRoot();
    if (!root) return;
    try {
      const jobs = (await get<Record<string, number>>(KEY_JOBS)) ?? {};
      const jobsDir = await root.getDirectoryHandle("jobs", { create: true });
      const now = Date.now();
      const keys = (jobsDir as FileSystemDirectoryHandle & { keys(): AsyncIterable<string> }).keys();
      for await (const name of keys) {
        const t = jobs[name];
        if (t === undefined || now - t > JOB_TTL_MS) {
          await jobsDir.removeEntry(name, { recursive: true }).catch(() => {});
          delete jobs[name];
        }
      }
      await set(KEY_JOBS, jobs);
    } catch { /* storage unavailable */ }
  }

  diagnostics = {
    copyReport: async () => {
      const caps = await this.capabilities();
      const log = this.lastJobId ? await this.diagnostics.jobLog(this.lastJobId) : null;
      await navigator.clipboard?.writeText(JSON.stringify({ app: APP_VERSION, engine: engineVersion, capabilities: caps, lastJob: log ? JSON.parse(log) : null }, null, 2));
    },
    openLogs: async () => {},
    jobLog: async (jobId: string): Promise<string | null> => (await this.pool).job().call<string | null>({ type: "job_log", jobId }).catch(() => null),
  };

  async version() {
    const engine = await (await this.pool).job().call<string>({ type: "version" }).catch(() => "unknown");
    return { app: APP_VERSION, build: engineVersion.sha, engine };
  }
}
