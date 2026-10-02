import { LicenseRefreshRequestSchema, type LicenseRefreshResponse } from "@cia/api-types";

import { preflight, route } from "@/lib/api";
import { grantsAccess } from "@/lib/entitlements";
import { ApiError, json, readJson } from "@/lib/http";
import { HOUR, enforceLimits } from "@/lib/rate-limit";
import { mintToken, verifyOwnToken } from "@/lib/tokens";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export const OPTIONS = preflight();

export const POST = route({ cors: true }, async ({ req, rt }) => {
  const body = await readJson(req, LicenseRefreshRequestSchema);
  const payload = await verifyOwnToken(rt.env, body.token);
  if (!payload) throw new ApiError(401, "unauthorized", "That license token isn't valid.");
  await enforceLimits(rt.db, [{ bucket: "refresh", id: payload.dev, max: 60, windowSeconds: HOUR }]);
  const now = rt.now();

  const device = await rt.db.findActiveDevice(payload.ent, payload.dev);
  let res: LicenseRefreshResponse;
  if (!device) {
    res = { status: "deactivated", reason: "This computer was removed from the license.", at: now.toISOString() };
    return json(res);
  }
  const ent = await rt.db.findEntitlementById(payload.ent);
  if (!ent) {
    res = { status: "deactivated", reason: "This license no longer exists.", at: now.toISOString() };
    return json(res);
  }
  if (ent.status === "revoked") {
    res = { status: "revoked", reason: ent.revoked_reason ?? "manual", at: ent.revoked_at ?? now.toISOString() };
    return json(res);
  }
  if (!grantsAccess(ent, now)) {
    res = { status: "ended", reason: "Your yearly plan ended.", at: ent.access_until ?? now.toISOString() };
    return json(res);
  }
  const appVersion = req.headers.get("x-smidge-version");
  await rt.db.touchDevice(device.id, appVersion);
  const token = await mintToken(rt.env, { entitlementId: ent.id, plan: ent.plan, kind: payload.kind, deviceHash: payload.dev, accessUntil: ent.access_until, key4: ent.key4 }, now);
  res = { status: "ok", token };
  return json(res);
});
