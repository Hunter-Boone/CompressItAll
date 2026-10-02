import { LicenseActivateRequestSchema, productKey, type LicenseActivateResponse } from "@cia/api-types";

import { preflight, route } from "@/lib/api";
import { grantsAccess } from "@/lib/entitlements";
import { secretBytes } from "@/lib/env";
import { ApiError, clientIp, json, readJson } from "@/lib/http";
import { hashKey } from "@/lib/keys";
import { DAY, HOUR, enforceLimits, ipKey } from "@/lib/rate-limit";
import { mintToken } from "@/lib/tokens";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export const OPTIONS = preflight();

export const POST = route({ cors: true }, async ({ req, rt }) => {
  await enforceLimits(rt.db, [{ bucket: "activate_ip", id: ipKey(secretBytes(rt.env, "IP_HASH_SALT"), clientIp(req)), max: 10, windowSeconds: HOUR }]);
  const body = await readJson(req, LicenseActivateRequestSchema);
  const normalised = productKey.normalise(body.product_key);
  if (normalised === null) throw new ApiError(404, "key_not_found", "That key has a typo. Check it against your email.");
  const hash = hashKey(normalised);
  await enforceLimits(rt.db, [{ bucket: "activate_key", id: hash.slice(0, 32), max: 30, windowSeconds: DAY }]);

  const ent = await rt.db.findEntitlementByKeyHash(hash);
  if (!ent) throw new ApiError(404, "key_not_found", "That key isn't known. Check it against your email.");
  const now = rt.now();
  if (!grantsAccess(ent, now)) {
    if (ent.status === "revoked") {
      throw new ApiError(403, "revoked", ent.revoked_reason === "chargeback" ? "This key was cancelled after a payment dispute." : "This key was refunded and no longer works.", {
        extra: { reason: ent.revoked_reason ?? "manual", at: ent.revoked_at ?? now.toISOString() },
      });
    }
    throw new ApiError(403, "ended", "This yearly plan has ended. Renew it to keep using Smidge Pro.", { extra: { reason: "ended", at: ent.access_until ?? now.toISOString() } });
  }
  if (body.kind === "desktop" && body.platform === "web") throw new ApiError(400, "bad_request", "A desktop activation needs a desktop platform.");

  const result = await rt.db.activateDevice({
    entitlementId: ent.id,
    kind: body.kind,
    deviceHash: body.device_hash,
    deviceName: body.device_name ?? null,
    platform: body.platform,
    appVersion: body.app_version,
  });
  if (!result.ok) {
    throw new ApiError(409, "device_limit", `This key is already used on ${result.devices.length} computers. Remove one in your account, then try again.`, { extra: { devices: result.devices } });
  }
  await rt.db.audit({ actor: "api", action: result.existing ? "device.reactivated" : "device.activated", entitlement_id: ent.id, detail: { kind: body.kind, platform: body.platform } });
  const token = await mintToken(rt.env, { entitlementId: ent.id, plan: ent.plan, kind: body.kind, deviceHash: body.device_hash, accessUntil: ent.access_until, key4: ent.key4 }, now);
  const res: LicenseActivateResponse = { token, plan: ent.plan, devices_used: result.used, devices_max: result.max };
  return json(res);
});
