// Request/response helpers for the route handlers. Plain web Request/Response so the handlers
// run unchanged under Vitest.

import type { ErrorCode } from "@cia/api-types";
import type { z } from "zod";

import type { Env } from "./env";

export class ApiError extends Error {
  constructor(
    public readonly status: number,
    public readonly code: ErrorCode | string,
    message: string,
    options?: { cause?: unknown; extra?: Record<string, unknown> },
  ) {
    super(message, options?.cause !== undefined ? { cause: options.cause } : undefined);
    this.name = "ApiError";
    this.extra = options?.extra;
  }
  readonly extra: Record<string, unknown> | undefined;
}

export function json(data: unknown, init: ResponseInit = {}): Response {
  const headers = new Headers(init.headers);
  headers.set("Content-Type", "application/json; charset=utf-8");
  headers.set("Cache-Control", "no-store");
  return new Response(JSON.stringify(data), { ...init, headers });
}

export function errorResponse(err: ApiError): Response {
  return json({ error: err.code, message: err.message, ...(err.extra ?? {}) }, { status: err.status });
}

/** Parse a JSON body with a zod schema; 400 bad_request on any problem. */
export async function readJson<S extends z.ZodTypeAny>(req: Request, schema: S): Promise<z.infer<S>> {
  let raw: unknown;
  try {
    raw = await req.json();
  } catch {
    throw new ApiError(400, "bad_request", "The request body isn't valid JSON.");
  }
  const parsed = schema.safeParse(raw);
  if (!parsed.success) {
    const first = parsed.error.issues[0];
    const where = first?.path.join(".") || "body";
    throw new ApiError(400, "bad_request", `Check ${where}: ${first?.message ?? "invalid"}.`);
  }
  return parsed.data;
}

/** Client IP from Vercel's proxy headers. */
export function clientIp(req: Request): string {
  const xff = req.headers.get("x-forwarded-for");
  if (xff) return xff.split(",")[0]!.trim();
  return req.headers.get("x-real-ip") ?? "unknown";
}

/** Tauri's webview origins, accepted next to the site and app origins. */
const NATIVE_ORIGINS = ["tauri://localhost", "http://tauri.localhost", "https://tauri.localhost"];

export function allowedOrigins(env: Env): string[] {
  return [env.NEXT_PUBLIC_SITE_URL, env.NEXT_PUBLIC_APP_URL, ...NATIVE_ORIGINS];
}

/**
 * CSRF check for POSTs (DESIGN.md 5.10): an Origin header, when present, must be one of ours.
 * Non-browser clients (the desktop app's Rust HTTP client, Paddle, cron) send none.
 */
export function checkOrigin(req: Request, env: Env, options: { required?: boolean } = {}): void {
  const origin = req.headers.get("origin");
  if (origin === null) {
    if (options.required) throw new ApiError(403, "forbidden", "This request must come from the Smidge site.");
    return;
  }
  if (!allowedOrigins(env).includes(origin)) {
    throw new ApiError(403, "forbidden", "This request came from a site Smidge doesn't trust.");
  }
}

/** CORS headers for the routes the web app calls (marked W in DESIGN.md 5.10). */
export function corsHeaders(env: Env): Record<string, string> {
  return {
    "Access-Control-Allow-Origin": env.NEXT_PUBLIC_APP_URL,
    "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
    "Access-Control-Allow-Headers": "Content-Type, Authorization, X-Smidge-Install",
    "Access-Control-Max-Age": "600",
    Vary: "Origin",
  };
}

export function withHeaders(res: Response, headers: Record<string, string>): Response {
  for (const [k, v] of Object.entries(headers)) res.headers.set(k, v);
  return res;
}

export function parseCookies(req: Request): Map<string, string> {
  const out = new Map<string, string>();
  const header = req.headers.get("cookie");
  if (!header) return out;
  for (const part of header.split(";")) {
    const i = part.indexOf("=");
    if (i < 0) continue;
    const name = part.slice(0, i).trim();
    const value = part.slice(i + 1).trim();
    if (name) out.set(name, decodeURIComponent(value));
  }
  return out;
}

export interface CookieOptions {
  maxAge: number;
  path: string;
  secure: boolean;
}

export function serializeCookie(name: string, value: string, o: CookieOptions): string {
  const parts = [`${name}=${encodeURIComponent(value)}`, "HttpOnly", "SameSite=Lax", `Path=${o.path}`, `Max-Age=${o.maxAge}`];
  if (o.secure) parts.push("Secure");
  return parts.join("; ");
}

export function bearer(req: Request, scheme: string): string | null {
  const h = req.headers.get("authorization");
  if (!h) return null;
  const [s, ...rest] = h.split(" ");
  if (!s || s.toLowerCase() !== scheme.toLowerCase()) return null;
  const v = rest.join(" ").trim();
  return v.length > 0 ? v : null;
}

export function noContent(headers: Record<string, string> = {}): Response {
  return new Response(null, { status: 204, headers });
}
