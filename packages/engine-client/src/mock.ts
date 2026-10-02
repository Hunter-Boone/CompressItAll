/**
 * MockHost: a fake EngineHost for browser development and Playwright UI tests.
 * It inspects real File objects (dimensions, duration) but invents results with
 * deterministic math so every UI state can be reached without the engine.
 */
import type {
  Capabilities,
  EngineEvent,
  InputItem,
  ItemOutcome,
  ItemPlan,
  JobSummary,
  Kind,
  Plan,
  PlanVerdict,
  Refusal,
} from "./generated";
import type { AllowanceView, EngineHost, InputSource, JobHandle, LicenseInfo, PlanRequest } from "./host";
import { duration, mb } from "./format";

const IMAGE_EXT = ["jpg", "jpeg", "png", "webp", "bmp", "tiff", "tif", "avif", "heic", "heif", "jxl"];
const VIDEO_EXT = ["mp4", "mov", "m4v", "mkv", "webm", "avi", "wmv", "flv", "ts", "mts", "3gp"];
const AUDIO_EXT = ["mp3", "wav", "flac", "ogg", "opus", "m4a", "aac", "aiff", "wma", "caf"];
const OFFICE_EXT = ["docx", "docm", "pptx", "pptm", "xlsx", "xlsm", "odt", "odp", "ods", "epub"];
const ARCHIVE_EXT = ["zip", "7z", "tar", "gz", "tgz", "xz", "zst", "bz2", "rar"];
const TEXT_EXT = ["txt", "csv", "json", "xml", "svg", "log", "md"];

function kindOf(name: string): Kind {
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  if (ext === "gif") return "animated_image";
  if (IMAGE_EXT.includes(ext)) return "image";
  if (VIDEO_EXT.includes(ext)) return "video";
  if (AUDIO_EXT.includes(ext)) return "audio";
  if (ext === "pdf") return "pdf";
  if (OFFICE_EXT.includes(ext)) return "office_doc";
  if (ARCHIVE_EXT.includes(ext)) return "archive";
  if (TEXT_EXT.includes(ext)) return "text";
  return "other";
}

let counter = 0;
const nextId = () => `mock_${Date.now().toString(36)}_${(counter++).toString(36)}`;

async function probeImage(file: File): Promise<{ w: number; h: number } | null> {
  try {
    const bmp = await createImageBitmap(file);
    const r = { w: bmp.width, h: bmp.height };
    bmp.close();
    return r;
  } catch {
    return null;
  }
}

async function probeMedia(file: File, video: boolean): Promise<{ ms: number; w?: number; h?: number } | null> {
  return new Promise((resolve) => {
    const el = document.createElement(video ? "video" : "audio") as HTMLVideoElement;
    const url = URL.createObjectURL(file);
    const done = (v: { ms: number; w?: number; h?: number } | null) => {
      URL.revokeObjectURL(url);
      resolve(v);
    };
    el.preload = "metadata";
    el.onloadedmetadata = () =>
      done({ ms: Math.round(el.duration * 1000), w: el.videoWidth || undefined, h: el.videoHeight || undefined });
    el.onerror = () => done(null);
    setTimeout(() => done(null), 3000);
    el.src = url;
  });
}

export interface MockOptions {
  /** Speed multiplier for fake progress (1 = realistic-ish). */
  speed?: number;
  kind?: "desktop" | "web";
  ffmpegInstalled?: boolean;
  pro?: boolean;
}

export class MockHost implements EngineHost {
  readonly kind: "desktop" | "web";
  private files = new Map<string, File>();
  private items = new Map<string, InputItem>();
  private cancelled = new Set<string>();
  private opts: Required<MockOptions>;
  private settings: Record<string, unknown>;
  private lic: LicenseInfo;
  private uses: number[] = [];

  constructor(opts: MockOptions = {}) {
    this.opts = { speed: 1, kind: "web", ffmpegInstalled: false, pro: false, ...opts };
    this.kind = this.opts.kind;
    this.settings = JSON.parse(localStorage.getItem("cia.mock.settings") ?? "{}");
    this.lic = this.opts.pro ? { status: "pro", plan: "lifetime", key4: "7KQ2P", keyMasked: "ABCDE-…-7KQ2P", devicesUsed: 1, devicesMax: 3 } : { status: "free" };
    (window as unknown as { __mockHost: MockHost }).__mockHost = this;
    this.ffmpegSetup = this.buildFfmpegSetup();
    this.updates = this.kind === "desktop" ? { check: async () => ({ available: false }), install: async () => {} } : undefined;
  }

  async capabilities(): Promise<Capabilities> {
    const desktop = this.kind === "desktop";
    return {
      host: desktop ? "desktop-mock" : "web-mock",
      ffmpeg: desktop && this.opts.ffmpegInstalled ? { version: "7.1.2", path: "/mock/ffmpeg", user_supplied: false, working_encoders: ["h264_nvenc", "libvpx-vp9"], has_libx264: false, has_libvpx_vp9: true, has_aac: true, has_libmp3lame: true, has_libopus: true } : null,
      web: desktop ? null : { cross_origin_isolated: true, webcodecs: true, h264_encode: true, vp9_encode: true, aac_encode: true, opus_encode: true, hdr_verified: false, directory_picker: "showDirectoryPicker" in window, opfs: true, browser: navigator.userAgent.includes("Firefox") ? "Firefox" : "Chromium" },
      can_copy_files: desktop,
      can_drag_out: desktop,
      can_reveal: desktop,
      can_choose_folder: desktop || "showDirectoryPicker" in window,
      heic_input: false,
      avif_input: true,
      opus_encode: true,
      mp3_encode: desktop && this.opts.ffmpegInstalled,
      aac_encode: !desktop || this.opts.ffmpegInstalled,
      video: !desktop || this.opts.ffmpegInstalled,
    };
  }

  async addInputs(inputs: InputSource[]): Promise<InputItem[]> {
    const out: InputItem[] = [];
    for (const src of inputs) {
      if (src.kind !== "file") continue;
      const file = src.file;
      const id = nextId();
      const kind = kindOf(file.name);
      const detail: InputItem["detail"] = { format: file.name.split(".").pop()?.toLowerCase() ?? "", width: null, height: null, has_alpha: null, frame_count: null, duration_ms: null, fps: null, video_codec: null, audio_streams: [], is_hdr: null, rotation_degrees: null, page_count: null, entry_count: null, encrypted: null };
      if (kind === "image" || kind === "animated_image") {
        const p = await probeImage(file);
        if (p) { detail.width = p.w; detail.height = p.h; }
        else if (file.size < 100) { detail.format = "corrupt"; }
      } else if (kind === "video" || kind === "audio") {
        const p = await probeMedia(file, kind === "video");
        if (p) { detail.duration_ms = BigInt(p.ms); detail.width = p.w ?? null; detail.height = p.h ?? null; detail.fps = 30; }
      } else if (kind === "pdf") {
        detail.page_count = Math.max(1, Math.round(file.size / 150_000));
      }
      const item: InputItem = { id, source: { type: "handle", handle_id: id }, rel_path: src.relPath ?? file.name, bytes: BigInt(file.size), kind, detail, folder: src.relPath?.includes("/") ? src.relPath.split("/")[0]! : null };
      this.files.set(id, file);
      this.items.set(id, item);
      out.push(item);
    }
    return out;
  }

  removeInput(itemId: string): void {
    this.files.delete(itemId);
    this.items.delete(itemId);
  }

  private fakeOutputBytes(item: InputItem, budget: number | null): { bytes: number; summary: string; quality: ItemPlan["prediction"]["quality"]; refusal?: Refusal; copy?: boolean } {
    const size = Number(item.bytes);
    const d = item.detail;
    switch (item.kind) {
      case "image": {
        const w = d.width ?? 2000, h = d.height ?? 1500;
        const floor = Math.round(w * h * 0.08); // q45 floor
        const nice = Math.round(w * h * 0.35);
        if (budget !== null && size < budget && d.format === "jpg") return { bytes: size, summary: `${w} × ${h} JPEG`, quality: "great", copy: true };
        if (budget === null) return { bytes: Math.min(size, nice), summary: `${w} × ${h} JPEG`, quality: "great" };
        if (budget >= nice) return { bytes: nice, summary: `${w} × ${h} JPEG`, quality: "great" };
        if (budget >= floor) return { bytes: Math.round(budget * 0.93), summary: `${w} × ${h} JPEG`, quality: budget > floor * 2 ? "good" : "okay" };
        // downscale
        const scale = Math.sqrt(budget / floor) * 0.97;
        const nw = Math.round(w * scale), nh = Math.round(h * scale);
        if (Math.max(nw, nh) < 480) return { bytes: floor, summary: "", quality: null, refusal: { code: { type: "below_quality_floor" }, message: `The smallest Smidge could make this was ${mb(floor)}. The limit is ${mb(budget)}.`, smallest_bytes: BigInt(floor), suggestions: [] } };
        return { bytes: Math.round(budget * 0.9), summary: `${nw} × ${nh} JPEG`, quality: "okay" };
      }
      case "video": {
        const ms = Number(d.duration_ms ?? 60_000);
        const secs = ms / 1000;
        if (budget === null) return { bytes: Math.round(size * 0.6), summary: `${d.height ?? 720}p, 30 fps, H.264`, quality: "great" };
        const minBps = 320 * 180 * 30 * 0.07 + 32_000;
        const maxSecs = Math.floor((budget * 8 * 0.96) / minBps);
        if (secs > maxSecs) {
          return { bytes: 0, summary: "", quality: null, refusal: { code: { type: "too_long_for_limit", max_duration_ms: BigInt(maxSecs * 1000) }, message: `Too long to fit in ${mb(budget)} at watchable quality. Trim it to under ${duration(maxSecs * 1000)}.`, smallest_bytes: null, suggestions: [{ type: "trim", max_duration_ms: BigInt(maxSecs * 1000) }, { type: "pick_preset", preset_id: "discord-nitro-basic", predicted_bytes: BigInt(Math.round(budget * 2.1)) }] } };
        }
        const bps = (budget * 8 * 0.96) / secs;
        const h = bps > 6_000_000 ? Math.min(1080, d.height ?? 1080) : bps > 2_500_000 ? 720 : bps > 1_200_000 ? 540 : 480;
        return { bytes: Math.round(budget * 0.94), summary: `${h}p, 30 fps, H.264`, quality: bps > 4_000_000 ? "great" : bps > 1_500_000 ? "good" : "okay" };
      }
      case "audio": {
        if (budget === null) return { bytes: Math.round(size * 0.5), summary: "FLAC", quality: "great" };
        return { bytes: Math.min(size, Math.round(budget * 0.9)), summary: "MP3, 128 kb/s", quality: "good" };
      }
      case "pdf":
        return { bytes: budget === null ? Math.round(size * 0.7) : Math.min(size, Math.round(budget * 0.85)), summary: `${d.page_count ?? 1} pages`, quality: "good" };
      case "office_doc":
        return { bytes: budget === null ? Math.round(size * 0.6) : Math.min(size, Math.round(budget * 0.85)), summary: "Photos re-saved at good quality", quality: "good" };
      case "archive":
      case "text":
        return { bytes: Math.round(size * 0.3), summary: "zip", quality: "great" };
      default:
        if (budget !== null && size > budget) return { bytes: size, summary: "", quality: null, refusal: { code: { type: "cannot_shrink_type" }, message: `Smidge can't make this kind of file smaller. It's ${mb(size)} and the limit is ${mb(budget)}.`, smallest_bytes: BigInt(size), suggestions: [] } };
        return { bytes: size, summary: "File", quality: "great", copy: true };
    }
  }

  async preview(req: PlanRequest): Promise<Plan> {
    await sleep(120 / this.opts.speed);
    const limit = req.goal.type === "fit" ? req.goal.limit : null;
    const perMessage = limit?.scope === "per_message";
    const total = req.items.reduce((a, i) => a + Number(i.bytes), 0);
    const plans: ItemPlan[] = [];
    let verdict: PlanVerdict = { type: "will_fit", quality: "great" };
    let worst: "great" | "good" | "okay" = "great";
    for (const item of req.items) {
      const budget = limit ? (perMessage ? Math.round((Number(limit.raw_budget_bytes) * Number(item.bytes)) / Math.max(1, total)) : Number(limit.raw_budget_bytes)) : null;
      const needsFfmpeg = this.kind === "desktop" && !this.opts.ffmpegInstalled && (item.kind === "video" || (item.kind === "audio" && ["m4a", "aac"].includes(item.detail.format)));
      if (needsFfmpeg) {
        verdict = { type: "cannot_fit", refusal: { code: { type: "needs_ffmpeg" }, message: "Videos need a one-time setup.", smallest_bytes: null, suggestions: [{ type: "install_ffmpeg" }] } };
        plans.push({ item_id: item.id, budget_bytes: budget === null ? null : BigInt(budget), strategy: { type: "none" }, prediction: { predicted_bytes: item.bytes, exact: false, summary: "Needs video support", quality: null, notes: [] } });
        continue;
      }
      if (item.detail.format === "corrupt") {
        plans.push({ item_id: item.id, budget_bytes: null, strategy: { type: "none" }, prediction: { predicted_bytes: item.bytes, exact: true, summary: "Damaged", quality: null, notes: ["This file is damaged or incomplete, so Smidge can't read it."] } });
        continue;
      }
      const r = this.fakeOutputBytes(item, budget);
      if (r.refusal) {
        verdict = { type: "cannot_fit", refusal: r.refusal };
      } else if (r.quality && order(r.quality) < order(worst)) {
        worst = r.quality;
      }
      plans.push({ item_id: item.id, budget_bytes: budget === null ? null : BigInt(budget), strategy: r.copy ? { type: "copy" } : { type: "none" }, prediction: { predicted_bytes: BigInt(r.bytes), exact: item.kind === "image", summary: r.summary, quality: r.quality, notes: [] } });
    }
    if (verdict.type !== "cannot_fit") verdict = { type: req.items.some((i) => i.kind === "video") ? "uncertain" : "will_fit", quality: worst };
    const predicted = plans.reduce((a, p) => a + Number(p.prediction.predicted_bytes), 0);
    const many = req.items.length > (limit?.max_files_per_message ?? 10);
    const packaging: Plan["packaging"] = many || req.packaging === "zip" ? { type: "archive", format: "zip", file_name: `${req.items[0]?.folder ?? "Files"} (${limit?.output_label ?? "smaller"}).zip`, overhead_bytes: BigInt(100 * req.items.length) } : { type: "separate_files" };
    return { job_id: nextId(), items: plans, packaging, predicted_total_bytes: BigInt(predicted), verdict, headline: headline(req, plans, verdict) };
  }

  async run(req: PlanRequest, onEvent: (e: EngineEvent) => void): Promise<JobHandle> {
    const plan = await this.preview(req);
    const jobId = plan.job_id;
    const start = Date.now();
    const done = (async (): Promise<JobSummary> => {
      onEvent({ type: "job_state", job_id: jobId, state: "running" });
      const outcomes: [string, ItemOutcome][] = [];
      const limit = req.goal.type === "fit" ? req.goal.limit : null;
      for (const p of plan.items) {
        const item = req.items.find((i) => i.id === p.item_id)!;
        if (this.cancelled.has(jobId)) { outcomes.push([item.id, { type: "cancelled" }]); continue; }
        onEvent({ type: "item_state", job_id: jobId, item_id: item.id, state: { type: "encoding", attempt: 1 } });
        const steps = item.kind === "video" ? 40 : item.kind === "image" ? 6 : 12;
        const per = (item.kind === "video" ? 150 : 60) / this.opts.speed;
        for (let s = 1; s <= steps; s++) {
          if (this.cancelled.has(jobId)) break;
          await sleep(per);
          onEvent({ type: "progress", job_id: jobId, item_id: item.id, fraction: s / steps, eta_ms: BigInt(Math.round((steps - s) * per)), label: item.kind === "video" ? "Encoding" : "Compressing" });
        }
        if (this.cancelled.has(jobId)) { outcomes.push([item.id, { type: "cancelled" }]); continue; }
        onEvent({ type: "item_state", job_id: jobId, item_id: item.id, state: { type: "verifying", attempt: 1 } });
        await sleep(120 / this.opts.speed);
        let outcome: ItemOutcome;
        if (p.prediction.summary === "Damaged") {
          outcome = { type: "failed", failure: { code: "damaged_input", message: "This file is damaged or incomplete, so Smidge can't read it.", closest_bytes: null } };
        } else if (p.strategy.type === "copy") {
          outcome = { type: "kept_original", artifact: this.artifact(item, Number(item.bytes), p, "", true) };
        } else {
          const ext = item.kind === "video" ? "mp4" : item.kind === "image" ? "jpg" : item.detail.format;
          outcome = { type: "fitted", artifact: this.artifact(item, Number(p.prediction.predicted_bytes), p, ext, false) };
          if (this.lic.status !== "pro") this.uses.push(Date.now());
        }
        outcomes.push([item.id, outcome]);
        onEvent({ type: "item_done", job_id: jobId, item_id: item.id, outcome });
      }
      const fitted = outcomes.filter(([, o]) => o.type === "fitted" || o.type === "kept_original").length;
      const anyCancel = outcomes.some(([, o]) => o.type === "cancelled");
      const verdict: JobSummary["verdict"] = anyCancel ? "cancelled" : fitted === outcomes.length ? "all_fit" : fitted === 0 ? "none_fit" : "some_fit";
      const totalOut = outcomes.reduce((a, [, o]) => a + (o.type === "fitted" || o.type === "kept_original" ? Number(o.artifact.bytes) : 0), 0);
      const label = limit?.output_label ?? "smaller";
      const summary: JobSummary = {
        job_id: jobId, outcomes, packaged: plan.packaging.type === "archive" ? { id: nextId(), item_id: "", location: { type: "opfs", path: "/jobs/x/" + plan.packaging.file_name }, file_name: plan.packaging.file_name, bytes: BigInt(totalOut), format: "zip", summary: `${fitted} files`, quality: null, verification: { size_ok: true, decodes: true, checks: ["zip re-opened, CRCs ok"], failures: [] } } : null,
        input_bytes: BigInt(req.items.reduce((a, i) => a + Number(i.bytes), 0)), total_bytes: BigInt(totalOut), verdict,
        headline: verdict === "all_fit" ? (limit ? `Fits in ${label}` : "Done") : verdict === "some_fit" ? `${fitted} of ${outcomes.length} files fit. ${outcomes.length - fitted} couldn't.` : verdict === "cancelled" ? (fitted ? `Stopped. ${fitted} files were saved before you stopped.` : "Stopped. Nothing was saved.") : "Nothing could be made small enough.",
        elapsed_ms: BigInt(Date.now() - start),
      };
      onEvent({ type: "job_done", job_id: jobId, summary });
      return summary;
    })();
    return { jobId, done };
  }

  private artifact(item: InputItem, bytes: number, p: ItemPlan, ext: string, kept: boolean): import("./generated").Artifact {
    const stem = (item.rel_path.split(/[/\\]/).pop() ?? item.rel_path).replace(/\.[^.]+$/, "");
    const name = kept ? item.rel_path : `${stem} (${"Discord"}).${ext}`;
    return { id: nextId(), item_id: item.id, location: this.kind === "desktop" ? { type: "path", path: `/Users/mock/${name}` } : { type: "opfs", path: `/jobs/x/${name}` }, file_name: name, bytes: BigInt(bytes), format: ext || item.detail.format, summary: p.prediction.summary, quality: p.prediction.quality, verification: { size_ok: true, decodes: true, checks: ["size read back", "decoded with a second decoder"], failures: [] } };
  }

  async cancel(jobId: string): Promise<void> { this.cancelled.add(jobId); }

  actions = {
    download: async (ids: string[]) => { console.info("mock download", ids); },
    saveToFolder: "showDirectoryPicker" in window ? async (ids: string[]) => { console.info("mock save", ids); } : undefined,
    revealInFolder: async (id: string) => { console.info("mock reveal", id); },
    copyFilesToClipboard: async (ids: string[]) => { console.info("mock copy", ids); },
    openFile: async (id: string) => { console.info("mock open", id); },
    chooseFiles: async (): Promise<InputSource[]> => new Promise((resolve) => {
      const input = document.createElement("input");
      input.type = "file"; input.multiple = true;
      input.onchange = () => resolve([...(input.files ?? [])].map((file) => ({ kind: "file" as const, file })));
      input.click();
    }),
    openExternal: async (url: string) => { window.open(url, "_blank"); },
  };

  async loadSettings() { return this.settings; }
  async saveSettings(s: Record<string, unknown>) { this.settings = s; localStorage.setItem("cia.mock.settings", JSON.stringify(s)); }
  async license() { return this.lic; }
  async activate(key: string): Promise<LicenseInfo> {
    await sleep(500);
    if (key.replace(/[^A-Z0-9]/gi, "").toUpperCase().endsWith("LIMIT")) throw Object.assign(new Error("This key is already used on 3 computers."), { code: "device_limit" });
    this.lic = { status: "pro", plan: "lifetime", key4: key.slice(-5).toUpperCase(), keyMasked: `${key.slice(0, 5).toUpperCase()}-…-${key.slice(-5).toUpperCase()}`, devicesUsed: 1, devicesMax: 3 };
    return this.lic;
  }
  async deactivate() { this.lic = { status: "free" }; }
  async startPurchase(plan: "lifetime" | "yearly") { return { buyUrl: `https://www.smidge.example/buy?plan=${plan}&claim=clm_mock`, claimId: "clm_mock" }; }
  async pollClaim(): Promise<"pending" | "fulfilled" | "expired"> { await sleep(3000); this.lic = { status: "pro", plan: "lifetime", key4: "MOCK1", keyMasked: "MOCK1-…-MOCK1", devicesUsed: 1, devicesMax: 3 }; return "fulfilled"; }
  async resendKey() { await sleep(300); }
  async allowance(): Promise<AllowanceView> {
    const now = Date.now();
    this.uses = this.uses.filter((t) => t + 86_400_000 > now);
    const remaining = Math.max(0, 3 - this.uses.length);
    return { remaining, nextFreeAtMs: remaining === 0 ? this.uses[0]! + 86_400_000 : null };
  }
  ffmpegSetup: EngineHost["ffmpegSetup"];
  updates: EngineHost["updates"];
  private buildFfmpegSetup(): EngineHost["ffmpegSetup"] { return this.kind === "desktop" ? {
    manifest: async () => ({ version: "7.1.2", bytes: 35_651_584, unpackedBytes: 94_371_840 }),
    install: async (onProgress: (p: import("./host").FfmpegSetupProgress) => void) => {
      for (let i = 0; i <= 10; i++) { await sleep(150); onProgress({ phase: "downloading", downloadedBytes: i * 3_565_158, totalBytes: 35_651_584 }); }
      onProgress({ phase: "checking" }); await sleep(400);
      onProgress({ phase: "testing" }); await sleep(600);
      this.opts.ffmpegInstalled = true;
      onProgress({ phase: "ready" });
    },
    useOwnCopy: async () => { this.opts.ffmpegInstalled = true; },
    remove: async () => { this.opts.ffmpegInstalled = false; },
    retest: async () => { await sleep(500); },
    cancel: async () => {},
  } : undefined; }
  diagnostics = { copyReport: async () => {}, openLogs: async () => {}, jobLog: async () => null };
  async version() { return { app: "0.1.0-mock", build: "dev", engine: "mock" }; }
}

function order(q: "great" | "good" | "okay"): number { return q === "great" ? 2 : q === "good" ? 1 : 0; }
function sleep(ms: number) { return new Promise((r) => setTimeout(r, ms)); }

function headline(req: PlanRequest, plans: ItemPlan[], verdict: PlanVerdict): string {
  if (verdict.type === "cannot_fit") return verdict.refusal.message;
  const total = plans.reduce((a, p) => a + Number(p.prediction.predicted_bytes), 0);
  const exact = plans.every((p) => p.prediction.exact);
  const about = exact ? "" : "about ";
  const q = verdict.type === "will_fit" || verdict.type === "uncertain" ? ` Quality: ${cap(verdict.quality)}.` : "";
  if (plans.length === 1) {
    const item = req.items[0]!, p = plans[0]!;
    const what = describe(item);
    if (p.strategy.type === "copy") return `${what} already fits. Nothing to change.`;
    return `${what} → ${p.prediction.summary}, ${about}${mb(total)}.${q}`;
  }
  const n = req.items.length;
  const what = req.items.every((i) => i.kind === "image") ? `${n} photos` : req.items.every((i) => i.kind === "video") ? `${n} videos` : `${n} files`;
  if (req.goal.type === "fit" && req.goal.limit.scope === "per_message") return `${what} → ${about}${mb(total)}, fits in one message.${q}`;
  if (req.goal.type === "fit") return `${what} → ${about}${mb(total)} in total.${q}`;
  return `${what} → ${about}${mb(total)}.`;
}

function describe(item: InputItem): string {
  const d = item.detail;
  switch (item.kind) {
    case "image": return d.width && d.height ? `${d.width} × ${d.height} photo` : "Photo";
    case "animated_image": return "Animated image";
    case "video": return d.duration_ms ? `${duration(d.duration_ms)} video` : "Video";
    case "audio": return d.duration_ms ? `${duration(d.duration_ms)} audio` : "Audio";
    case "pdf": return d.page_count ? `${d.page_count}-page PDF` : "PDF";
    case "office_doc": return ["pptx", "pptm", "odp"].includes(d.format) ? "Presentation" : "Document";
    case "archive": return "Zip file";
    case "text": return "Text file";
    default: return "File";
  }
}
function cap(s: string) { return s.charAt(0).toUpperCase() + s.slice(1); }
