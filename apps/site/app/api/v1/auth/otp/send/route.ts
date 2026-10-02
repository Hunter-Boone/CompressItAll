import { OtpSendRequestSchema, type OtpSendResponse } from "@cia/api-types";

import { otpEmail } from "@/emails/otp";
import { route } from "@/lib/api";
import { sendOnce } from "@/lib/email";
import { secretBytes } from "@/lib/env";
import { clientIp, json, readJson } from "@/lib/http";
import { generateCode, storeCode, withinCooldown } from "@/lib/otp";
import { HOUR, enforceLimits, ipKey } from "@/lib/rate-limit";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export const POST = route({ browserOnly: true }, async ({ req, rt }) => {
  const body = await readJson(req, OtpSendRequestSchema);
  await enforceLimits(rt.db, [
    { bucket: "otp_send_ip", id: ipKey(secretBytes(rt.env, "IP_HASH_SALT"), clientIp(req)), max: 20, windowSeconds: HOUR },
    { bucket: "otp_send_email", id: body.email, max: 3, windowSeconds: 15 * 60 },
  ]);
  const res: OtpSendResponse = { sent: true };
  const customers = await rt.db.findCustomersByEmail(body.email);
  if (customers.length === 0) return json(res); // same answer either way
  if (await withinCooldown(rt, body.email)) return json(res);
  const code = generateCode();
  await storeCode(rt, body.email, code);
  const sentAt = (await rt.db.getOtp(body.email))?.sent_at ?? rt.now().toISOString();
  await sendOnce(rt, "otp", `${body.email}:${sentAt}`, body.email, otpEmail({ code }));
  return json(res);
});
