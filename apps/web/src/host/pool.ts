/**
 * Worker pool for the WASM engine (DESIGN.md 2.5, 2.6). Size is
 * clamp(hardwareConcurrency - 1, 1, 4), or 2 when deviceMemory <= 4 GB.
 *
 * v1 scheduling: a job (preview + run) runs on one worker, the "job worker",
 * so the engine's preview cache is reused by `run`. `inspect` fans out across
 * the other workers. A worker that traps is terminated and replaced; the
 * caller sees a rejected promise.
 */
import type { Capabilities, EngineEvent, WebCapabilities } from "@cia/engine-client";
import type { Reply, Request } from "../workers/engine.worker";

type Pending = { resolve: (v: unknown) => void; reject: (e: Error) => void; onEvent?: (e: EngineEvent) => void };
type DistributiveOmit<T, K extends PropertyKey> = T extends unknown ? Omit<T, K> : never;
type Req = DistributiveOmit<Request, "id">;

export class EngineWorkerError extends Error {
  constructor(message: string, readonly fatal: boolean) {
    super(message);
  }
}

export function poolSize(): number {
  const nav = navigator as Navigator & { deviceMemory?: number };
  const n = Math.min(4, Math.max(1, (nav.hardwareConcurrency || 2) - 1));
  return nav.deviceMemory !== undefined && nav.deviceMemory <= 4 ? Math.min(n, 2) : n;
}

export class EngineWorker {
  private worker: Worker;
  private pending = new Map<number, Pending>();
  private seq = 1;
  /** ids of input files this worker holds bytes for */
  readonly files = new Set<string>();
  /** handle ids whose video probe this worker has been given */
  readonly probes = new Set<string>();
  readonly ready: Promise<Capabilities>;
  busy = 0;
  dead = false;

  constructor(web: WebCapabilities, private onDead: (w: EngineWorker) => void) {
    this.worker = new Worker(new URL("../workers/engine.worker.ts", import.meta.url), { type: "module", name: "cia-engine" });
    this.worker.onmessage = (e: MessageEvent<Reply>) => this.onReply(e.data);
    this.worker.onerror = (e) => this.die(new EngineWorkerError(e.message || "worker error", true));
    this.ready = this.call<Capabilities>({ type: "init", web });
    this.ready.catch(() => {});
  }

  private onReply(r: Reply) {
    if ("type" in r) {
      if (r.type === "event") this.pending.get(r.id)?.onEvent?.(r.event);
      return;
    }
    const p = this.pending.get(r.id);
    if (!p) return;
    this.pending.delete(r.id);
    this.busy--;
    if (r.ok) p.resolve(r.result);
    else {
      const err = new EngineWorkerError(r.error, !!r.fatal);
      p.reject(err);
      if (r.fatal) this.die(err);
    }
  }

  private die(err: EngineWorkerError) {
    if (this.dead) return;
    this.dead = true;
    for (const p of this.pending.values()) p.reject(err);
    this.pending.clear();
    this.worker.terminate();
    this.onDead(this);
  }

  call<T>(req: Req, onEvent?: (e: EngineEvent) => void, transfer: Transferable[] = []): Promise<T> {
    if (this.dead) return Promise.reject(new EngineWorkerError("worker is gone", true));
    const id = this.seq++;
    this.busy++;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject, onEvent });
      this.worker.postMessage({ id, ...req }, transfer);
    });
  }

  terminate() {
    this.die(new EngineWorkerError("terminated", true));
  }
}

export class WorkerPool {
  private workers: EngineWorker[] = [];
  private rr = 0;
  readonly size: number;

  constructor(private web: WebCapabilities, size = poolSize()) {
    this.size = size;
    for (let i = 0; i < size; i++) this.workers.push(this.spawn());
  }

  private spawn(): EngineWorker {
    return new EngineWorker(this.web, (dead) => {
      const i = this.workers.indexOf(dead);
      if (i >= 0) this.workers[i] = this.spawn();
    });
  }

  /** The worker that previews and runs jobs. */
  job(): EngineWorker {
    return this.workers[0]!;
  }

  /** Any other worker (round robin), for inspect fan-out. */
  next(): EngineWorker {
    if (this.workers.length === 1) return this.workers[0]!;
    const others = this.workers.slice(1);
    const w = others[this.rr++ % others.length]!;
    return w;
  }

  all(): EngineWorker[] {
    return [...this.workers];
  }

  /** Force the job worker to restart (cancel without a shared flag). */
  restartJobWorker() {
    this.workers[0]!.terminate();
  }

  capabilities(): Promise<Capabilities> {
    return this.workers[0]!.ready;
  }
}
