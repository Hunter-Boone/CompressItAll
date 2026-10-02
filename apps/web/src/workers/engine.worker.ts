/**
 * Engine worker (DESIGN.md 2.5 "Web Workers"): one WASM instance per worker.
 *
 * Module loading: v1 has every worker call the wasm-bindgen `init()` itself.
 * The browser's HTTP cache plus V8/SpiderMonkey code caching make the second
 * and later compiles cheap, and it keeps the worker self-contained (no
 * `WebAssembly.Module` hand-off, no ordering between main-thread compile and
 * worker start). Compiling once on the main thread and posting the Module is
 * the planned optimisation when the pool grows past a few workers.
 *
 * Protocol: the main thread posts `Request`s; each gets exactly one `Reply`.
 * Engine events during `run` arrive as `{ type: "event", id, event }` before
 * the reply. Outputs are written to OPFS under `/jobs/<job_id>/...` here with
 * `createSyncAccessHandle` (falling back to `createWritable`, then to in-memory
 * Blobs returned in the reply when OPFS is missing).
 */
import type { Capabilities, EngineEvent, InputItem, JobSummary, Plan, WebCapabilities } from "@cia/engine-client";
import type { PlanRequest } from "@cia/engine-client";

// ---- wasm-bindgen exports we use (the generated .d.ts is not committed) ----
interface CiaWasm {
  start(): void;
  version(): string;
  capabilities(web: WebCapabilities): Capabilities;
  add_file(id: string, name: string, bytes: Uint8Array): void;
  has_file(id: string): boolean;
  remove_file(id: string): void;
  clear_files(): void;
  held_bytes(): number;
  inspect(specs: { id: string; rel_path: string; folder: string | null }[]): InputItem[];
  preview(req: PlanRequest): Plan;
  run(req: PlanRequest, onEvent: (e: EngineEvent) => void, cancel: Int32Array | undefined): JobSummary;
  job_log(jobId: string): string | undefined;
  list_outputs(): string[];
  take_output(path: string): Uint8Array<ArrayBuffer> | undefined;
  drop_outputs(): void;
  package_zip(names: string[], bodies: Uint8Array[]): Uint8Array<ArrayBuffer>;
}
interface CiaWasmModule extends CiaWasm {
  default(input?: { module_or_path: URL | string | Request | Response | BufferSource | WebAssembly.Module }): Promise<unknown>;
}

export interface FileIn { id: string; name: string; file: File }
export interface ZipEntryIn { name: string; path: string; blob?: Blob }

export type Request =
  | { id: number; type: "init"; web: WebCapabilities }
  | { id: number; type: "add_files"; files: FileIn[] }
  | { id: number; type: "remove_files"; ids: string[] }
  | { id: number; type: "inspect"; specs: { id: string; rel_path: string; folder: string | null }[] }
  | { id: number; type: "preview"; req: PlanRequest }
  | { id: number; type: "run"; req: PlanRequest; cancel: Int32Array | null }
  | { id: number; type: "package_zip"; entries: ZipEntryIn[] }
  | { id: number; type: "job_log"; jobId: string }
  | { id: number; type: "version" }
  | { id: number; type: "held_bytes" };

export type StorageMode = "opfs-sync" | "opfs-writable" | "memory";
export interface RunResult { summary: JobSummary; storage: StorageMode; blobs: Record<string, Blob> }

export type Reply =
  | { id: number; ok: true; result: unknown }
  | { id: number; ok: false; error: string; fatal?: boolean }
  | { type: "event"; id: number; event: EngineEvent };

const ctx = self as unknown as { postMessage(msg: unknown, transfer?: Transferable[]): void; onmessage: ((e: MessageEvent<Request>) => void) | null; location: Location };

let wasm: CiaWasm | null = null;
const fileNames = new Map<string, string>();

async function loadWasm(): Promise<CiaWasm> {
  if (wasm) return wasm;
  const url = new URL("/engine/cia_wasm.js", ctx.location.origin).href;
  const mod = (await import(/* @vite-ignore */ url)) as CiaWasmModule;
  await mod.default({ module_or_path: new URL("/engine/cia_wasm_bg.wasm", ctx.location.origin) });
  mod.start();
  wasm = mod;
  return mod;
}

// ---- OPFS ---------------------------------------------------------------------
interface SyncHandle { write(data: BufferSource, opts?: { at?: number }): number; truncate(n: number): void; flush(): void; close(): void }
type FileHandleWithSync = FileSystemFileHandle & { createSyncAccessHandle?: () => Promise<SyncHandle> };

async function opfsRoot(): Promise<FileSystemDirectoryHandle | null> {
  try {
    const nav = navigator as Navigator & { storage?: { getDirectory?: () => Promise<FileSystemDirectoryHandle> } };
    if (!nav.storage?.getDirectory) return null;
    return await nav.storage.getDirectory();
  } catch {
    return null;
  }
}

async function dirFor(root: FileSystemDirectoryHandle, segments: string[]): Promise<FileSystemDirectoryHandle> {
  let d = root;
  for (const s of segments) d = await d.getDirectoryHandle(s, { create: true });
  return d;
}

/** Write `bytes` at OPFS `path` ("/jobs/<id>/<folder>/<name>"). Returns the mode used, or null when OPFS is unavailable. */
async function writeOpfs(root: FileSystemDirectoryHandle, path: string, bytes: Uint8Array<ArrayBuffer>): Promise<StorageMode> {
  const parts = path.split("/").filter(Boolean);
  const name = parts.pop()!;
  const dir = await dirFor(root, parts);
  const fh = (await dir.getFileHandle(name, { create: true })) as FileHandleWithSync;
  if (fh.createSyncAccessHandle) {
    try {
      const h = await fh.createSyncAccessHandle();
      try {
        h.truncate(0);
        h.write(bytes, { at: 0 });
        h.flush();
      } finally {
        h.close();
      }
      return "opfs-sync";
    } catch {
      // Safari < 15.2, or a handle another context holds open: fall through to createWritable.
    }
  }
  const w = await fh.createWritable();
  await w.write(bytes);
  await w.close();
  return "opfs-writable";
}

async function readOpfs(root: FileSystemDirectoryHandle, path: string): Promise<Uint8Array<ArrayBuffer>> {
  const parts = path.split("/").filter(Boolean);
  const name = parts.pop()!;
  let d = root;
  for (const s of parts) d = await d.getDirectoryHandle(s);
  const f = await (await d.getFileHandle(name)).getFile();
  return new Uint8Array(await f.arrayBuffer());
}

// ---- handlers -------------------------------------------------------------------
async function handle(req: Request): Promise<unknown> {
  switch (req.type) {
    case "init": {
      const w = await loadWasm();
      return w.capabilities(req.web);
    }
    case "version":
      return (await loadWasm()).version();
    case "held_bytes":
      return wasm?.held_bytes() ?? 0;
    case "add_files": {
      const w = await loadWasm();
      for (const f of req.files) {
        if (w.has_file(f.id)) continue;
        const buf = new Uint8Array(await f.file.arrayBuffer());
        w.add_file(f.id, f.name, buf);
        fileNames.set(f.id, f.name);
      }
      return null;
    }
    case "remove_files": {
      const w = await loadWasm();
      for (const id of req.ids) {
        w.remove_file(id);
        fileNames.delete(id);
      }
      return null;
    }
    case "inspect":
      return (await loadWasm()).inspect(req.specs);
    case "preview":
      return (await loadWasm()).preview(req.req);
    case "run": {
      const w = await loadWasm();
      const summary = w.run(req.req, (event) => ctx.postMessage({ type: "event", id: req.id, event } satisfies Reply), req.cancel ?? undefined);
      // Move outputs out of the engine's memory.
      const root = await opfsRoot();
      let storage: StorageMode = "memory";
      const blobs: Record<string, Blob> = {};
      for (const path of w.list_outputs()) {
        const bytes = w.take_output(path);
        if (!bytes) continue;
        if (root) {
          try {
            storage = await writeOpfs(root, path, bytes);
            continue;
          } catch (e) {
            console.warn("OPFS write failed, keeping in memory", path, e);
          }
        }
        storage = "memory";
        blobs[path] = new Blob([bytes]);
      }
      return { summary, storage, blobs } satisfies RunResult;
    }
    case "package_zip": {
      const w = await loadWasm();
      const root = await opfsRoot();
      const names: string[] = [];
      const bodies: Uint8Array[] = [];
      for (const e of req.entries) {
        names.push(e.name);
        if (e.blob) bodies.push(new Uint8Array(await e.blob.arrayBuffer()));
        else if (root) bodies.push(await readOpfs(root, e.path));
        else throw new Error(`no bytes for ${e.path}`);
      }
      return w.package_zip(names, bodies);
    }
    case "job_log":
      return (await loadWasm()).job_log(req.jobId) ?? null;
  }
}

ctx.onmessage = (e: MessageEvent<Request>) => {
  const req = e.data;
  void handle(req).then(
    (result) => {
      const transfer: Transferable[] = result instanceof Uint8Array ? [result.buffer as ArrayBuffer] : [];
      ctx.postMessage({ id: req.id, ok: true, result } satisfies Reply, transfer);
    },
    (err: unknown) => {
      // A WebAssembly trap (RuntimeError) leaves the instance in an unknown state: the pool replaces this worker.
      const fatal = err instanceof WebAssembly.RuntimeError || (err instanceof Error && /unreachable|memory access out of bounds|table index/i.test(err.message));
      const error = err instanceof Error ? err.message : String(err);
      if (fatal) wasm = null;
      ctx.postMessage({ id: req.id, ok: false, error, fatal } satisfies Reply);
    },
  );
};
