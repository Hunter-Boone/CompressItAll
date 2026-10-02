/**
 * Video worker (DESIGN.md 2.5, 3.5.10): runs the WebCodecs pipeline off the
 * main thread. It probes files (mediabunny), transcodes one item at a time
 * (decide → plan → remux or encode → verify → retry, 3.5.8), posts the same
 * `EngineEvent`s the engine would, and extracts preview frames for the Trim
 * panel. Planning math lives in the wasm engine: this worker asks the host
 * for it over a small RPC (`plan_video`, `retry_video`, `decide_video`), so
 * the 11 MB module is not instantiated a second time.
 */
import type { Attempt, Decision, EngineEvent, ItemState, VideoPlan, VideoProbe } from "@cia/engine-client";
import { canEncodePlan, probeVideo, ProbeError, remux, retryLoop, transcode, TranscodeError, verifyOutput, FrameExtractor, type ExpectedOutput, type ProbeResult, type TranscodeHandle } from "@cia/webvideo";
import type { PlanOut, VideoResultIn, VideoRpcName, VideoWork } from "./engine.worker";

export type VideoRequest =
  | { id: number; type: "probe"; file: Blob }
  | { id: number; type: "run_item"; jobId: string; itemId: string; file: Blob; probe: VideoProbe; work: VideoWork; sourceBytes: number }
  | { id: number; type: "frame"; key: string; file: Blob; ms: number }
  | { id: number; type: "release"; key: string }
  | { id: number; type: "cancel" }
  | { id: number; type: "rpc_reply"; rpcId: number; ok: boolean; result?: unknown; error?: string };

export type VideoReply =
  | { id: number; ok: true; result: unknown }
  | { id: number; ok: false; error: string }
  | { type: "event"; id: number; event: EngineEvent }
  | { type: "rpc"; id: number; rpcId: number; name: VideoRpcName; args: unknown[] };

/** `probe` reply: null when the file is damaged or has no video track. */
export type ProbeReply = ProbeResult | null;
export interface RunItemResult { result: VideoResultIn; bytes: Uint8Array<ArrayBuffer> | null }

const ctx = self as unknown as { postMessage(msg: unknown, transfer?: Transferable[]): void; onmessage: ((e: MessageEvent<VideoRequest>) => void) | null };

// ---- RPC to the host (which forwards to the engine worker) --------------------
let rpcSeq = 1;
const rpcPending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();
function rpc<T>(reqId: number, name: VideoRpcName, args: unknown[]): Promise<T> {
  const rpcId = rpcSeq++;
  return new Promise<T>((resolve, reject) => {
    rpcPending.set(rpcId, { resolve: resolve as (v: unknown) => void, reject });
    ctx.postMessage({ type: "rpc", id: reqId, rpcId, name, args } satisfies VideoReply);
  });
}

// ---- state ---------------------------------------------------------------------
let current: { handle: TranscodeHandle | null; cancelled: boolean } | null = null;
const extractors = new Map<string, FrameExtractor>();

function expectedFor(plan: VideoPlan, work: VideoWork): ExpectedOutput {
  return {
    container: plan.container,
    video_codec: plan.video_codec,
    audio_codec: plan.audio_codec,
    duration_ms: Number(plan.duration_ms),
    fps: plan.fps,
    width: plan.width,
    height: plan.height,
    has_audio: plan.audio_bps > 0n,
    hard_bytes: Number(work.budget.hard_bytes),
    allowed: work.allowed,
  };
}

async function runItem(req: Extract<VideoRequest, { type: "run_item" }>): Promise<RunItemResult> {
  const { jobId, itemId, file, probe, work } = req;
  const emit = (event: EngineEvent) => ctx.postMessage({ type: "event", id: req.id, event } satisfies VideoReply);
  const state = (s: ItemState) => emit({ type: "item_state", job_id: jobId, item_id: itemId, state: s });
  let lastProgress = 0;
  const progress = (fraction: number, label: string, force = false) => {
    const now = Date.now();
    if (!force && now - lastProgress < 100) return;
    lastProgress = now;
    emit({ type: "progress", job_id: jobId, item_id: itemId, fraction: Math.max(0, Math.min(0.99, fraction)), eta_ms: null, label });
  };
  const me = { handle: null as TranscodeHandle | null, cancelled: false };
  current = me;
  const cancelled = () => me.cancelled;
  const trim = work.options.trim;
  const audio = work.options.audio;
  const hard = Number(work.budget.hard_bytes);
  try {
    state({ type: "planning" });
    const decision = await rpc<Decision>(req.id, "decide_video", [probe, work.hard_bytes === null ? 0 : Number(work.hard_bytes), req.sourceBytes, work.allowed]);
    if (decision.type === "keep_original") return { result: { kept_original: true, attempts: [] }, bytes: null };
    const planned = await rpc<PlanOut>(req.id, "plan_video", [probe, work.budget, work.target, work.options]);
    if (planned.type === "refusal") {
      const code = planned.refusal.code;
      return { result: { error: "refused", max_duration_ms: code.type === "too_long_for_limit" ? code.max_duration_ms : 0n, attempts: [] }, bytes: null };
    }
    const plan = planned.plan;
    const attempts: Attempt[] = [];
    const attempt = (a: Attempt) => { attempts.push(a); emit({ type: "attempt", job_id: jobId, attempt: a }); };

    // Streams allowed, container not: copy them over first (3.5.4). Fall through to an encode when that does not fit.
    if (decision.type === "remux" && !cancelled()) {
      state({ type: "encoding", attempt: 1 });
      progress(0, "Repacking", true);
      const started = Date.now();
      const h = remux({ file, probe, container: decision.container, trim, audio, onProgress: (f) => progress(f, "Repacking") });
      me.handle = h;
      const a: Attempt = { n: 1, item_id: itemId, encoder: "remux", params: { container: decision.container }, output_bytes: null, score: null, elapsed_ms: 0n, verdict: { type: "cancelled" } };
      try {
        const bytes = await h.done;
        me.handle = null;
        a.output_bytes = BigInt(bytes.byteLength);
        state({ type: "verifying", attempt: 1 });
        const sourcePlan: VideoPlan = { ...plan, container: decision.container, width: probe.display_w, height: probe.display_h, fps: probe.avg_fps, video_codec: probe.video_codec as VideoPlan["video_codec"], audio_codec: (probe.audio[0]?.codec === "opus" ? "opus" : "aac"), audio_bps: audio.type === "remove" || probe.audio.length === 0 ? 0n : plan.audio_bps || 1n, duration_ms: BigInt(trim ? Math.max(0, Math.min(Number(probe.duration_ms), trim[1]) - trim[0]) : Number(probe.duration_ms)) };
        const report = await verifyOutput(bytes, expectedFor(sourcePlan, work));
        a.elapsed_ms = BigInt(Date.now() - started);
        if (bytes.byteLength < hard && report.size_ok && report.decodes && report.failures.length === 0) {
          a.verdict = { type: "fits" };
          attempt(a);
          return { result: { plan: sourcePlan, attempts, verification: report }, bytes };
        }
        a.verdict = bytes.byteLength >= hard ? { type: "over", by_bytes: BigInt(bytes.byteLength - hard) } : { type: "error", message: report.failures.join("; ") };
        attempt(a);
      } catch (e) {
        me.handle = null;
        if (e instanceof TranscodeError && e.code === "cancelled") { attempt(a); return { result: { error: "cancelled", attempts }, bytes: null }; }
        a.verdict = { type: "error", message: e instanceof Error ? e.message : String(e) };
        attempt(a);
      }
    }

    const outcome = await retryLoop(plan, hard, itemId, {
      encode: async (p, n) => {
        if (n > 1) state({ type: "retry", attempt: n });
        state({ type: "encoding", attempt: n });
        const label = n > 1 ? `Trying again (${n})` : "Encoding";
        progress(0, label, true);
        if (!(await canEncodePlan(p))) throw new TranscodeError(`this browser cannot encode ${p.video_codec} at ${p.width}x${p.height}`, "browser_lacks_codec");
        const h = transcode({ file, probe, plan: p, trim, audio, onProgress: (f) => progress(f, label) });
        me.handle = h;
        try {
          return await h.done;
        } finally {
          me.handle = null;
        }
      },
      verify: async (bytes, p) => {
        state({ type: "verifying", attempt: attempts.length + 1 });
        progress(0.99, "Checking", true);
        return verifyOutput(bytes, expectedFor(p, work));
      },
      retry: (p, actual) => rpc<VideoPlan | null>(req.id, "retry_video", [p, actual, work.budget]),
      onAttempt: attempt,
      cancelled,
    });
    switch (outcome.type) {
      case "fitted":
        return { result: { plan: outcome.plan, attempts, verification: outcome.verification }, bytes: outcome.bytes };
      case "over":
        return { result: { error: "over", closest_bytes: BigInt(outcome.closest), attempts }, bytes: null };
      case "cancelled":
        return { result: { error: "cancelled", attempts }, bytes: null };
      default:
        return { result: { error: outcome.code === "damaged" ? "damaged" : outcome.code, message: outcome.message, closest_bytes: outcome.closest === null ? null : BigInt(outcome.closest), attempts }, bytes: null };
    }
  } finally {
    if (current === me) current = null;
  }
}

async function handle(req: VideoRequest): Promise<{ result: unknown; transfer?: Transferable[] }> {
  switch (req.type) {
    case "probe": {
      try {
        return { result: (await probeVideo(req.file)) satisfies ProbeReply };
      } catch (e) {
        if (e instanceof ProbeError) return { result: null satisfies ProbeReply };
        throw e;
      }
    }
    case "run_item": {
      const r = await runItem(req);
      return { result: r, transfer: r.bytes ? [r.bytes.buffer] : [] };
    }
    case "frame": {
      let x = extractors.get(req.key);
      if (!x) {
        x = new FrameExtractor(req.file);
        extractors.set(req.key, x);
      }
      return { result: await x.frame(req.ms) };
    }
    case "release":
      extractors.get(req.key)?.dispose();
      extractors.delete(req.key);
      return { result: null };
    case "cancel": {
      if (current) {
        current.cancelled = true;
        await current.handle?.cancel();
      }
      return { result: null };
    }
    case "rpc_reply": {
      const p = rpcPending.get(req.rpcId);
      rpcPending.delete(req.rpcId);
      if (p) {
        if (req.ok) p.resolve(req.result);
        else p.reject(new Error(req.error ?? "rpc failed"));
      }
      return { result: null };
    }
  }
}

ctx.onmessage = (e: MessageEvent<VideoRequest>) => {
  const req = e.data;
  if (req.type === "rpc_reply") {
    void handle(req);
    return;
  }
  void handle(req).then(
    ({ result, transfer }) => ctx.postMessage({ id: req.id, ok: true, result } satisfies VideoReply, transfer ?? []),
    (err: unknown) => ctx.postMessage({ id: req.id, ok: false, error: err instanceof Error ? err.message : String(err) } satisfies VideoReply),
  );
};
