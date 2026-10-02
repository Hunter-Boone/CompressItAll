import { CheckoutCreateRequestSchema, type CheckoutCreateResponse } from "@cia/api-types";

import { route } from "@/lib/api";
import { CHECKOUT_COOKIE, CHECKOUT_TTL_S, hashSecret, newSecret } from "@/lib/claims";
import { secretBytes } from "@/lib/env";
import { ApiError, clientIp, json, readJson, serializeCookie } from "@/lib/http";
import { newCheckoutId } from "@/lib/ids";
import { paddle } from "@/lib/paddle";
import { HOUR, enforceLimits, ipKey } from "@/lib/rate-limit";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export const POST = route({ browserOnly: true }, async ({ req, rt }) => {
  await enforceLimits(rt.db, [{ bucket: "checkout", id: ipKey(secretBytes(rt.env, "IP_HASH_SALT"), clientIp(req)), max: 30, windowSeconds: HOUR }]);
  const body = await readJson(req, CheckoutCreateRequestSchema);
  const now = rt.now();

  let claimId: string | null = null;
  if (body.claim_id) {
    const claim = await rt.db.findClaim(body.claim_id);
    if (!claim) throw new ApiError(404, "not_found", "That purchase link isn't known. Start again from the app.");
    if (claim.status !== "pending" || new Date(claim.expires_at).getTime() <= now.getTime()) {
      throw new ApiError(410, "claim_expired", "That purchase link has expired. Start again from the app.");
    }
    claimId = claim.id;
  }

  const priceId = body.plan === "lifetime" ? rt.env.PADDLE_PRICE_ID_LIFETIME : rt.env.PADDLE_PRICE_ID_YEARLY;
  const secret = newSecret();
  const checkout = await rt.db.insertCheckout({
    id: newCheckoutId(now.getTime()),
    secret_hash: hashSecret(secret),
    plan: body.plan,
    paddle_price_id: priceId,
    paddle_transaction_id: null,
    claim_id: claimId,
    entitlement_id: null,
  });
  const txn = await paddle.createTransaction(rt, { priceId, checkoutId: checkout.id, claimId });
  await rt.db.updateCheckout(checkout.id, { paddle_transaction_id: txn.id });

  // checkout_id is an extra field (the zod schema strips unknown keys); the success URL needs it.
  const res: CheckoutCreateResponse & { checkout_id: string } = { transaction_id: txn.id, checkout_id: checkout.id };
  // DESIGN.md 5.8.3 says Path=/success; the cookie must reach GET /api/v1/checkout/{id}, so the
  // path is the API prefix instead (docs/DECISIONS.md).
  const cookie = serializeCookie(CHECKOUT_COOKIE, secret, { maxAge: CHECKOUT_TTL_S, path: "/api/v1/checkout", secure: rt.env.NEXT_PUBLIC_SITE_URL.startsWith("https://") });
  return json(res, { status: 201, headers: { "Set-Cookie": cookie } });
});
