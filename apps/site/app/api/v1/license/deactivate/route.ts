import { LicenseDeactivateRequestSchema, type OkResponse } from "@cia/api-types";

import { preflight, route } from "@/lib/api";
import { ApiError, json, readJson } from "@/lib/http";
import { verifyOwnToken } from "@/lib/tokens";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export const OPTIONS = preflight();

export const POST = route({ cors: true }, async ({ req, rt }) => {
  const body = await readJson(req, LicenseDeactivateRequestSchema);
  const payload = await verifyOwnToken(rt.env, body.token);
  if (!payload) throw new ApiError(401, "unauthorized", "That license token isn't valid.");
  const device = await rt.db.findActiveDevice(payload.ent, payload.dev);
  if (device) {
    await rt.db.deactivateDevice(device.id, "device");
    await rt.db.audit({ actor: "api", action: "device.deactivated", entitlement_id: payload.ent, detail: { by: "device", kind: payload.kind } });
  }
  const res: OkResponse = { ok: true };
  return json(res);
});
