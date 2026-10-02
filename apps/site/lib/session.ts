// Dashboard sessions (DESIGN.md 5.6.5): cookie `smg_session` holds 32 random bytes (base64url);
// the row is keyed by sha256(cookie). 30 days, sliding, revocable. No JWT.

import { createHash, randomBytes } from "node:crypto";

import { ApiError, parseCookies, serializeCookie } from "./http";
import type { Runtime } from "./runtime";
import type { Session } from "./db";

export const SESSION_COOKIE = "smg_session";
export const SESSION_TTL_S = 30 * 24 * 3600;

export function hashSessionCookie(value: string): string {
  return createHash("sha256").update(value, "utf8").digest("hex");
}

export async function createSession(rt: Runtime, email: string, userAgent: string | null): Promise<{ cookie: string; header: string }> {
  const value = randomBytes(32).toString("base64url");
  const now = rt.now();
  await rt.db.insertSession({
    id_hash: hashSessionCookie(value),
    email,
    created_at: now.toISOString(),
    last_used_at: now.toISOString(),
    expires_at: new Date(now.getTime() + SESSION_TTL_S * 1000).toISOString(),
    revoked_at: null,
    user_agent: userAgent ? userAgent.slice(0, 300) : null,
  });
  return { cookie: value, header: sessionCookieHeader(rt, value, SESSION_TTL_S) };
}

export function sessionCookieHeader(rt: Runtime, value: string, maxAge: number): string {
  return serializeCookie(SESSION_COOKIE, value, { maxAge, path: "/", secure: rt.env.NEXT_PUBLIC_SITE_URL.startsWith("https://") });
}

/** The live session for a request, with its expiry slid forward; null when absent/expired/revoked. */
export async function readSession(rt: Runtime, req: Request): Promise<Session | null> {
  const value = parseCookies(req).get(SESSION_COOKIE);
  if (!value || value.length < 32) return null;
  const idHash = hashSessionCookie(value);
  const s = await rt.db.findSession(idHash);
  const now = rt.now();
  if (!s || s.revoked_at !== null || new Date(s.expires_at).getTime() <= now.getTime()) return null;
  // Sliding window: refresh at most once an hour to keep writes down.
  if (now.getTime() - new Date(s.last_used_at).getTime() > 3600 * 1000) {
    await rt.db.touchSession(idHash, now.toISOString(), new Date(now.getTime() + SESSION_TTL_S * 1000).toISOString());
  }
  return s;
}

export async function requireSession(rt: Runtime, req: Request): Promise<Session> {
  const s = await readSession(rt, req);
  if (!s) throw new ApiError(401, "unauthorized", "Sign in to your account first.");
  return s;
}
