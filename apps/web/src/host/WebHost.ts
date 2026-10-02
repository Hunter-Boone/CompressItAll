/**
 * WebHost: the EngineHost for the browser (DESIGN.md 2.2, 2.5, 3.10).
 * Inputs are File objects; the engine runs in a pool of Web Workers (pool.ts);
 * outputs live in OPFS under /jobs/<job_id>/ and leave as downloads or, on
 * Chromium, through the File System Access API. Settings, the license token
 * and the free allowance are in IndexedDB.
 *
 * Video (DESIGN.md 3.5.10, docs/DECISIONS.md "web video split"): WebCodecs is
 * asynchronous, the engine's runner is not. Video items are probed in the
 * video worker after the engine has detected them, the probe is handed to the
 * engine so `preview` plans them like any other item, and `run` transcodes
 * them in the video worker first, registers the results with the engine, and
 * only then runs the engine, which names, verifies, packages and summarises
 * everything in one `JobSummary`. The UI sees one Plan, one job, one stream of
 * events.
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
import type { VideoProbe } from "@cia/engine-client";
import { encoderSupport, isHdrVerified } from "@cia/webvideo";
import engineVersion from "../engine-version.json";
import { EngineWorkerError, WorkerPool, type EngineWorker } from "./pool";
import { LicenseClient, browserName } from "./license";
import { VideoClient } from "./video";
import type { FileIn, RunResult, VideoRpcName, VideoWork, ZipEntryIn } from "../workers/engine.worker";

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
  const w = window as Window & { showDirectoryPicker?: unknown };
  const enc = await encoderSupport();
  const nav = navigator as Navigator & { storage?: { getDirectory?: unknown } };
  const browser = browserName();
  return {
    cross_origin_isolated: window.crossOriginIsolated === true,
    webcodecs: enc.webcodecs,
    h264_encode: enc.h264,
    vp9_encode: enc.vp9,
    aac_encode: enc.aac,
    opus_encode: enc.opus,
    hdr_verified: isHdrVerified(browser, browserMajorVersion()),
    directory_picker: typeof w.showDirectoryPicker === "function",
    opfs: typeof nav.storage?.getDirectory === "function",
    browser,
  };
}

function browserMajorVersion(): number {
  const m = /(?:Edg|OPR|Chrome|Chromium|Firefox|Version)\/(\d+)/.exec(navigator.userAgent);
  return m ? Number(m[1]) : 0;
}

/** Fill the `KindDetail` fields `cia_engine::inspect` fills from a backend probe. */
function applyProbe(item: InputItem, probe: VideoProbe) {
  item.detail.duration_ms = probe.duration_ms;
  item.detail.width = probe.display_w;
  item.detail.height = probe.display_h;
  item.detail.fps = probe.avg_fps;
  item.detail.video_codec = probe.video_codec;
  item.detail.is_hdr = probe.is_hdr;
  item.detail.rotation_degrees = probe.rotation_degrees;
  item.detail.audio_streams = probe.audio.map((a) => ({ index: a.index, codec: a.codec, channels: a.channels, sample_rate: a.sample_rate, bitrate_bps: a.bitrate_bps, title: a.title }));
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
  /** item id -> mediabunny probe (null: damaged or no video track) */
  private probes = new Map<string, { probe: VideoProbe; canDecode: boolean } | null>();
  private video = new VideoClient((name, args) => this.videoRpc(name, args));
  /** The job in flight and which half of it is running, for cancel. */
  private active: { jobId: string; phase: "video" | "engine"; cancelled: boolean } | null = null;

  constructor() {
    this.webCaps = probeWebCaps();
    this.pool = this.webCaps.then((web) => new WorkerPool(web));
    void this.cleanOldJobs();
  }

  private async videoRpc(name: VideoRpcName, args: unknown[]): Promise<unknown> {
    const w = (await this.pool).job();
    await w.ready;
    return w.call({ type: "video_rpc", name, args });
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
    // Videos: the engine detected them; mediabunny reads what the planner needs (3.5.1).
    for (const item of items) {
      if (item.kind !== "video" || item.detail.format === "corrupt" || item.detail.format === "unreadable") continue;
      const file = this.heldFor(item.id)?.file;
      if (!file) continue;
      const r = await this.video.probe(file).catch(() => null);
      if (r) {
        this.probes.set(item.id, { probe: r.probe, canDecode: r.canDecode });
        applyProbe(item, r.probe);
      } else {
        this.probes.set(item.id, null);
        item.detail.format = "corrupt";
      }
    }
    return items;
  }

  /** Hand a worker the probes of the video items it is about to plan or run. */
  private async ensureProbes(w: EngineWorker, ids: string[]) {
    const probes: { id: string; probe: VideoProbe | null; canDecode: boolean }[] = [];
    for (const id of ids) {
      const handle = this.handleId(id);
      if (w.probes.has(handle) || !this.probes.has(id)) continue;
      const p = this.probes.get(id);
      probes.push({ id: handle, probe: p?.probe ?? null, canDecode: p?.canDecode ?? false });
    }
    if (!probes.length) return;
    await w.call({ type: "set_video_probes", probes });
    for (const p of probes) w.probes.add(p.id);
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
    this.probes.delete(itemId);
    this.video.release(itemId);
    void this.pool.then((p) => { for (const w of p.all()) { w.files.delete(handle); w.files.delete(itemId); w.probes.delete(handle); void w.call({ type: "remove_files", ids: [handle] }).catch(() => {}); } });
  }

  // ---- plan and run ------------------------------------------------------------

  async preview(req: PlanRequest): Promise<Plan> {
    const pool = await this.pool;
    const w = pool.job();
    await w.ready;
    const ids = req.items.map((i) => i.id);
    await this.ensureFiles(w, ids);
    await this.ensureProbes(w, ids);
    return w.call<Plan>({ type: "preview", req });
  }

  async run(req: PlanRequest, onEvent: (e: EngineEvent) => void): Promise<JobHandle> {
    const pool = await this.pool;
    const w = pool.job();
    await w.ready;
    const ids = req.items.map((i) => i.id);
    await this.ensureFiles(w, ids);
    await this.ensureProbes(w, ids);
    const shared = typeof SharedArrayBuffer === "function" && window.crossOriginIsolated ? new Int32Array(new SharedArrayBuffer(4)) : null;
    const jobId = `job_${newId().slice(2)}`;
    if (shared) this.cancelFlags.set(jobId, shared);
    const active = { jobId, phase: "video" as "video" | "engine", cancelled: false };
    this.active = active;
    const started = Date.now();
    const done = (async () => {
      // 1. Video items first, in the video worker (WebCodecs), results registered with the engine.
      const work = await w.call<VideoWork[]>({ type: "video_work", req });
      if (work.length) onEvent({ type: "job_state", job_id: jobId, state: "running" });
      for (const item of work) {
        if (active.cancelled) break;
        const file = this.heldFor(item.item_id)?.file;
        const probe = this.probes.get(item.item_id)?.probe;
        if (!file || !probe) continue;
        const r = await this.video.runItem({ jobId, itemId: item.item_id, file, probe, work: item, sourceBytes: file.size }, onEvent);
        await w.call({ type: "set_video_result", handleId: this.handleId(item.item_id), result: r.result, bytes: r.bytes }, undefined, r.bytes ? [r.bytes.buffer] : []);
      }
      if (active.cancelled && !shared) return this.cancelledSummary(req, jobId, started);
      // 2. Everything else, plus naming, verification bookkeeping, packaging and the summary.
      active.phase = "engine";
      const r = await w.call<RunResult>({ type: "run", req, cancel: shared, jobId }, onEvent);
      for (const [path, blob] of Object.entries(r.blobs)) this.blobs.set(path, blob);
      this.remember(r.summary);
      await this.recordJob(r.summary.job_id);
      if ((await this.lic.info()).status !== "pro") await this.consumeAllowance(r.summary);
      return r.summary;
    })().then((summary) => {
      if (summary.verdict === "cancelled" && !this.active?.cancelled) return summary;
      return summary;
    }, (err: unknown) => {
      const crashed = err instanceof EngineWorkerError && err.fatal;
      const message = crashed ? "Smidge ran out of memory on this file. Try the desktop app." : `Something went wrong: ${err instanceof Error ? err.message : String(err)}`;
      const summary = active.cancelled ? this.cancelledSummary(req, jobId, started) : this.failedSummary(req, jobId, message, started);
      onEvent({ type: "job_done", job_id: summary.job_id, summary });
      return summary;
    }).finally(() => {
      this.cancelFlags.delete(jobId);
      if (this.active === active) this.active = null;
      void w.call({ type: "set_video_result", handleId: "", result: {}, bytes: null }).catch(() => {});
    });
    this.lastJobId = jobId;
    return { jobId, done };
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

  private cancelledSummary(req: PlanRequest, jobId: string, started: number): JobSummary {
    const summary: JobSummary = {
      job_id: jobId,
      outcomes: req.items.map((i) => [i.id, { type: "cancelled" }]),
      packaged: null,
      input_bytes: req.items.reduce((a, i) => a + i.bytes, 0n),
      total_bytes: 0n,
      verdict: "cancelled",
      headline: "Stopped. Nothing was saved.",
      elapsed_ms: BigInt(Date.now() - started),
    };
    return summary;
  }

  private remember(summary: JobSummary) {
    this.artifacts.clear();
    for (const [, o] of summary.outcomes) if (o.type === "fitted" || o.type === "kept_original") this.artifacts.set(o.artifact.id, o.artifact);
    if (summary.packaged) this.artifacts.set(summary.packaged.id, summary.packaged);
  }

  async cancel(jobId: string): Promise<void> {
    const flag = this.cancelFlags.get(jobId) ?? [...this.cancelFlags.values()][0];
    if (flag) Atomics.store(flag, 0, 1);
    const active = this.active;
    if (active && active.jobId === jobId) {
      active.cancelled = true;
      await this.video.cancel().catch(() => {});
      if (!flag && active.phase === "engine") (await this.pool).restartJobWorker(); // no SharedArrayBuffer: stop the worker outright
    } else if (!flag) {
      (await this.pool).restartJobWorker();
    }
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
    previewFrame: async (itemId: string, ms: number) => {
      const file = this.heldFor(itemId)?.file;
      if (!file || this.items.get(itemId)?.kind !== "video") return null;
      return this.video.frame(itemId, file, Math.max(0, Math.round(ms))).catch(() => null);
    },
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
