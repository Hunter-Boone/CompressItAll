import { OtpVerifyRequestSchema, type OkResponse } from "@cia/api-types";

import { route } from "@/lib/api";
import { secretBytes } from "@/lib/env";
import { ApiError, clientIp, json, readJson } from "@/lib/http";
import { verifyCode } from "@/lib/otp";
import { HOUR, enforceLimits, ipKey } from "@/lib/rate-limit";
import { createSession } from "@/lib/session";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export const POST = route({ browserOnly: true }, async ({ req, rt }) => {
  await enforceLimits(rt.db, [{ bucket: "otp_verify_ip", id: ipKey(secretBytes(rt.env, "IP_HASH_SALT"), clientIp(req)), max: 30, windowSeconds: HOUR }]);
  const body = await readJson(req, OtpVerifyRequestSchema);
  const result = await verifyCode(rt, body.email, body.code);
  if (result !== "ok") throw new ApiError(400, "invalid_code", "That code didn't work. Check it, or ask for a new one.");
  const session = await createSession(rt, body.email, req.headers.get("user-agent"));
  await rt.db.audit({ actor: "dashboard", action: "session.created", detail: { email: body.email } });
  const res: OkResponse = { ok: true };
  return json(res, { headers: { "Set-Cookie": session.header } });
});
