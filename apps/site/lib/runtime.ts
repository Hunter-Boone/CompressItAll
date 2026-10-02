// The per-process runtime: validated env, the Db, fetch (Paddle, Resend, GitHub) and a clock.
// Built lazily on first request so `next build` needs no secrets. Tests replace it with
// `setRuntime` (MemoryDb, fetch stub, fixed clock).

import type { Db } from "./db";
import { MemoryDb } from "./db-memory";
import { getEnv, type Env } from "./env";

export interface Runtime {
  env: Env;
  db: Db;
  fetch: typeof fetch;
  now: () => Date;
}

let current: Runtime | null = null;
let memory: MemoryDb | null = null;

async function build(): Promise<Runtime> {
  const env = getEnv();
  let db: Db;
  if (env.SMG_DB === "memory") {
    memory ??= new MemoryDb();
    db = memory;
  } else {
    const { SupabaseDb } = await import("./db-supabase");
    db = new SupabaseDb(env.SUPABASE_URL!, env.SUPABASE_SERVICE_ROLE_KEY!);
  }
  return { env, db, fetch: (input, init) => fetch(input, init), now: () => new Date() };
}

export async function getRuntime(): Promise<Runtime> {
  if (current === null) current = await build();
  return current;
}

/** Tests only: install a runtime, or pass null to rebuild from the environment. */
export function setRuntime(rt: Runtime | null): void {
  current = rt;
}
