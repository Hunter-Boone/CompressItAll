import { LogoutRequestSchema, type OkResponse } from "@cia/api-types";

import { route } from "@/lib/api";
import { json, parseCookies, readJson } from "@/lib/http";
import { SESSION_COOKIE, hashSessionCookie, readSession, sessionCookieHeader } from "@/lib/session";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export const POST = route({ browserOnly: true }, async ({ req, rt }) => {
  const body = await readJson(req, LogoutRequestSchema);
  const session = await readSession(rt, req);
  const now = rt.now().toISOString();
  if (session) {
    if (body.everywhere) await rt.db.revokeSessionsForEmail(session.email, now);
    else {
      const value = parseCookies(req).get(SESSION_COOKIE)!;
      await rt.db.revokeSession(hashSessionCookie(value), now);
    }
  }
  const res: OkResponse = { ok: true };
  return json(res, { headers: { "Set-Cookie": sessionCookieHeader(rt, "", 0) } });
});
