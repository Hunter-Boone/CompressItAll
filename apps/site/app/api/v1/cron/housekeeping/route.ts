import type { CronResponse } from "@cia/api-types";

import { requireCron, route } from "@/lib/api";
import { json } from "@/lib/http";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export const IDLE_DEVICE_DAYS = 180;

export const GET = route({}, async ({ req, rt }) => {
  requireCron(rt, req);
  const now = rt.now();
  const iso = now.toISOString();
  const counts = {
    claims_expired: await rt.db.expirePendingClaims(iso),
    otp_deleted: await rt.db.deleteOtpUsedOrExpired(iso),
    rate_limit_windows_deleted: await rt.db.deleteRateLimitsBefore(new Date(now.getTime() - 24 * 3600 * 1000).toISOString()),
    devices_idled: await rt.db.markIdleDesktopDevices(new Date(now.getTime() - IDLE_DEVICE_DAYS * 24 * 3600 * 1000).toISOString()),
  };
  if (counts.devices_idled > 0) await rt.db.audit({ actor: "cron", action: "devices.idled", detail: { count: counts.devices_idled } });
  const res: CronResponse = { ok: true, counts };
  return json(res);
});
