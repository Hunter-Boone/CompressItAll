import { z } from "zod";

import type { OkResponse } from "@cia/api-types";

import { ownedEntitlement } from "@/lib/account";
import { route } from "@/lib/api";
import { ApiError, json } from "@/lib/http";
import { requireSession } from "@/lib/session";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export const DEACTIVATIONS_PER_30_DAYS = 10;

export const POST = route<{ id: string }>({ browserOnly: true }, async ({ req, rt, params }) => {
  const session = await requireSession(rt, req);
  const id = z.string().uuid().safeParse(params.id);
  if (!id.success) throw new ApiError(404, "not_found", "That computer isn't on this account.");
  const device = await rt.db.findDeviceById(id.data);
  if (!device) throw new ApiError(404, "not_found", "That computer isn't on this account.");
  const ent = await ownedEntitlement(rt, session, device.entitlement_id);
  const now = rt.now();
  const since = new Date(now.getTime() - 30 * 24 * 3600 * 1000).toISOString();
  const recent = await rt.db.countDashboardDeactivationsSince(ent.id, since);
  if (recent >= DEACTIVATIONS_PER_30_DAYS) {
    throw new ApiError(429, "rate_limited", "You've removed 10 computers in the last 30 days. Contact support to remove more.");
  }
  if (device.deactivated_at === null) {
    await rt.db.deactivateDevice(device.id, "dashboard");
    await rt.db.audit({ actor: "dashboard", action: "device.deactivated", entitlement_id: ent.id, detail: { device_id: device.id, email: session.email } });
  }
  const res: OkResponse = { ok: true };
  return json(res);
});
