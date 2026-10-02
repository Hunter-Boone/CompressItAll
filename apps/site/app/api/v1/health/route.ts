import type { HealthResponse } from "@cia/api-types";

import pkg from "../../../../package.json";
import { json } from "@/lib/http";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export async function GET(): Promise<Response> {
  const sha = process.env.VERCEL_GIT_COMMIT_SHA?.slice(0, 7);
  const res: HealthResponse = { ok: true, version: sha ? `${pkg.version}+${sha}` : pkg.version };
  return json(res);
}
