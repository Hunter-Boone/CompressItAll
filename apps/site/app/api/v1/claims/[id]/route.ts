import { ClaimIdSchema, type ClaimStatusResponse } from "@cia/api-types";

import { preflight, route } from "@/lib/api";
import { secretMatches } from "@/lib/claims";
import { ApiError, bearer, json } from "@/lib/http";
import { enforceLimits } from "@/lib/rate-limit";
import { decryptDisplayKey, tokenForClaimDevice } from "@/lib/webhook";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export const OPTIONS = preflight();

export const GET = route<{ id: string }>({ cors: true }, async ({ req, rt, params }) => {
  const id = ClaimIdSchema.safeParse(params.id);
  if (!id.success) throw new ApiError(404, "not_found", "No such claim.");
  const secret = bearer(req, "Claim");
  if (!secret) throw new ApiError(401, "unauthorized", "Missing claim secret.");
  // 1,500 per claim: 2 h at one per 3 s with margin.
  await enforceLimits(rt.db, [{ bucket: "claim_poll", id: id.data, max: 1500, windowSeconds: 7200 }]);
  const claim = await rt.db.findClaim(id.data);
  if (!claim || !secretMatches(secret, claim.secret_hash)) throw new ApiError(404, "not_found", "No such claim.");

  let body: ClaimStatusResponse;
  if (claim.status === "fulfilled" && claim.entitlement_id) {
    const ent = await rt.db.findEntitlementById(claim.entitlement_id);
    if (!ent) throw new ApiError(500, "internal", "The claim points at a missing entitlement.");
    const token = await tokenForClaimDevice(rt, ent, claim.kind, claim.device_hash);
    body = { status: "fulfilled", product_key: decryptDisplayKey(rt, ent), token, plan: ent.plan };
  } else if (claim.status === "expired" || new Date(claim.expires_at).getTime() <= rt.now().getTime()) {
    body = { status: "expired" };
  } else {
    body = { status: "pending" };
  }
  return json(body);
});
