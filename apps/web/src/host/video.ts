/**
 * Client for the video worker (apps/web/src/workers/video.worker.ts). One
 * worker, created on first use and replaced if it dies. Planner RPCs the
 * worker makes are answered by `rpcHandler`, which the WebHost points at the
 * engine worker's wasm instance.
 */
import type { EngineEvent, VideoProbe } from "@cia/engine-client";
import type { VideoRpcName, VideoWork } from "../workers/engine.worker";
import type { ProbeReply, RunItemResult, VideoReply, VideoRequest } from "../workers/video.worker";

type DistributiveOmit<T, K extends PropertyKey> = T extends unknown ? Omit<T, K> : never;
type Req = DistributiveOmit<VideoRequest, "id">;
type Pending = { resolve: (v: unknown) => void; reject: (e: Error) => void; onEvent?: (e: EngineEvent) => void };

export class VideoClient {
  private worker: Worker | null = null;
  private pending = new Map<number, Pending>();
  private seq = 1;

  constructor(private rpcHandler: (name: VideoRpcName, args: unknown[]) => Promise<unknown>) {}

  private ensure(): Worker {
    if (this.worker) return this.worker;
    const w = new Worker(new URL("../workers/video.worker.ts", import.meta.url), { type: "module", name: "cia-video" });
    w.onmessage = (e: MessageEvent<VideoReply>) => this.onReply(e.data);
    w.onerror = (e) => this.die(new Error(e.message || "video worker error"));
    this.worker = w;
    return w;
  }

  private onReply(r: VideoReply) {
    if ("type" in r) {
      if (r.type === "event") this.pending.get(r.id)?.onEvent?.(r.event);
      else if (r.type === "rpc") {
        this.rpcHandler(r.name, r.args).then(
          (result) => this.worker?.postMessage({ id: 0, type: "rpc_reply", rpcId: r.rpcId, ok: true, result } satisfies VideoRequest),
          (err: unknown) => this.worker?.postMessage({ id: 0, type: "rpc_reply", rpcId: r.rpcId, ok: false, error: err instanceof Error ? err.message : String(err) } satisfies VideoRequest),
        );
      }
      return;
    }
    const p = this.pending.get(r.id);
    if (!p) return;
    this.pending.delete(r.id);
    if (r.ok) p.resolve(r.result);
    else p.reject(new Error(r.error));
  }

  private die(err: Error) {
    for (const p of this.pending.values()) p.reject(err);
    this.pending.clear();
    this.worker?.terminate();
    this.worker = null;
  }

  call<T>(req: Req, onEvent?: (e: EngineEvent) => void, transfer: Transferable[] = []): Promise<T> {
    const w = this.ensure();
    const id = this.seq++;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject, onEvent });
      w.postMessage({ id, ...req }, transfer);
    });
  }

  /** mediabunny probe; null when the file is damaged or has no video track. */
  probe(file: Blob): Promise<ProbeReply> {
    return this.call<ProbeReply>({ type: "probe", file });
  }

  runItem(args: { jobId: string; itemId: string; file: Blob; probe: VideoProbe; work: VideoWork; sourceBytes: number }, onEvent: (e: EngineEvent) => void): Promise<RunItemResult> {
    return this.call<RunItemResult>({ type: "run_item", ...args }, onEvent);
  }

  frame(key: string, file: Blob, ms: number): Promise<string | null> {
    return this.call<string | null>({ type: "frame", key, file, ms });
  }

  release(key: string): void {
    if (this.worker) void this.call({ type: "release", key }).catch(() => {});
  }

  cancel(): Promise<void> {
    if (!this.worker) return Promise.resolve();
    return this.call<null>({ type: "cancel" }).then(() => undefined);
  }
}
