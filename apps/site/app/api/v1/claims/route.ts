import { ClaimsCreateRequestSchema, type ClaimsCreateResponse } from "@cia/api-types";

import { preflight, route } from "@/lib/api";
import { CLAIM_TTL_MS, hashSecret, newSecret } from "@/lib/claims";
import { json, clientIp, readJson } from "@/lib/http";
import { newClaimId } from "@/lib/ids";
import { HOUR, enforceLimits, ipKey } from "@/lib/rate-limit";
import { secretBytes } from "@/lib/env";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export const OPTIONS = preflight();

export const POST = route({ cors: true }, async ({ req, rt }) => {
  await enforceLimits(rt.db, [{ bucket: "claims", id: ipKey(secretBytes(rt.env, "IP_HASH_SALT"), clientIp(req)), max: 20, windowSeconds: HOUR }]);
  const body = await readJson(req, ClaimsCreateRequestSchema);
  const secret = newSecret();
  const now = rt.now();
  const claim = await rt.db.insertClaim({
    id: newClaimId(now.getTime()),
    secret_hash: hashSecret(secret),
    kind: body.kind,
    device_hash: body.device_hash,
    device_name: body.device_name ?? null,
    platform: body.platform,
    entitlement_id: null,
    status: "pending",
    expires_at: new Date(now.getTime() + CLAIM_TTL_MS).toISOString(),
  });
  const res: ClaimsCreateResponse = {
    claim_id: claim.id,
    claim_secret: secret,
    buy_url: `${rt.env.NEXT_PUBLIC_SITE_URL}/buy?claim=${encodeURIComponent(claim.id)}`,
    expires_at: claim.expires_at,
  };
  return json(res, { status: 201 });
});
