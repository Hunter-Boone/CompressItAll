import { CheckoutIdSchema, type CheckoutStatusResponse } from "@cia/api-types";

import { route } from "@/lib/api";
import { CHECKOUT_COOKIE, KEY_VISIBLE_MS, secretMatches } from "@/lib/claims";
import { ApiError, json, parseCookies } from "@/lib/http";
import { decryptDisplayKey } from "@/lib/webhook";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export const GET = route<{ id: string }>({}, async ({ req, rt, params }) => {
  const id = CheckoutIdSchema.safeParse(params.id);
  if (!id.success) throw new ApiError(404, "not_found", "No such checkout.");
  const secret = parseCookies(req).get(CHECKOUT_COOKIE);
  if (!secret) throw new ApiError(401, "unauthorized", "This page only works in the browser you bought from. Your key is in your email.");
  const checkout = await rt.db.findCheckout(id.data);
  if (!checkout || !secretMatches(secret, checkout.secret_hash)) throw new ApiError(404, "not_found", "No such checkout.");

  let body: CheckoutStatusResponse = { status: "pending" };
  if (checkout.entitlement_id) {
    const ent = await rt.db.findEntitlementById(checkout.entitlement_id);
    if (!ent) throw new ApiError(500, "internal", "The checkout points at a missing entitlement.");
    if (rt.now().getTime() - new Date(ent.created_at).getTime() > KEY_VISIBLE_MS) {
      throw new ApiError(410, "not_found", "This page has expired. Your key is in your email.");
    }
    body = { status: "fulfilled", product_key: decryptDisplayKey(rt, ent), plan: ent.plan };
  }
  return json(body);
});
