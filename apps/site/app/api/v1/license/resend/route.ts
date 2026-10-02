import { LicenseResendRequestSchema, type OkResponse } from "@cia/api-types";

import { keyResendEmail } from "@/emails/key-resend";
import { preflight, route } from "@/lib/api";
import { sendOnce } from "@/lib/email";
import { grantsAccess } from "@/lib/entitlements";
import { secretBytes } from "@/lib/env";
import { clientIp, json, readJson } from "@/lib/http";
import { HOUR, enforceLimits, ipKey } from "@/lib/rate-limit";
import { decryptDisplayKey } from "@/lib/webhook";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export const OPTIONS = preflight();

export const POST = route({ cors: true }, async ({ req, rt }) => {
  const body = await readJson(req, LicenseResendRequestSchema);
  await enforceLimits(rt.db, [
    { bucket: "resend_ip", id: ipKey(secretBytes(rt.env, "IP_HASH_SALT"), clientIp(req)), max: 10, windowSeconds: HOUR },
    { bucket: "resend_email", id: body.email, max: 3, windowSeconds: HOUR },
  ]);
  const customers = await rt.db.findCustomersByEmail(body.email);
  const ents = await rt.db.listEntitlementsForCustomers(customers.map((c) => c.id));
  if (ents.length > 0) {
    const now = rt.now();
    const hour = now.toISOString().slice(0, 13);
    await sendOnce(
      rt,
      "key_resend",
      `${body.email}:${hour}`,
      body.email,
      keyResendEmail({ keys: ents.map((e) => ({ productKey: decryptDisplayKey(rt, e), plan: e.plan, active: grantsAccess(e, now) })), siteUrl: rt.env.NEXT_PUBLIC_SITE_URL }),
    );
  }
  const res: OkResponse = { ok: true };
  return json(res);
});
