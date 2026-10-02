import { PortalRequestSchema, type PortalResponse } from "@cia/api-types";

import { ownedEntitlement } from "@/lib/account";
import { route } from "@/lib/api";
import { ApiError, json, readJson } from "@/lib/http";
import { paddle } from "@/lib/paddle";
import { requireSession } from "@/lib/session";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export const POST = route({ browserOnly: true }, async ({ req, rt }) => {
  const session = await requireSession(rt, req);
  const body = await readJson(req, PortalRequestSchema);
  const ent = await ownedEntitlement(rt, session, body.entitlement_id);
  const customer = await rt.db.findCustomerById(ent.customer_id);
  if (!customer?.paddle_customer_id) throw new ApiError(404, "not_found", "There's no billing account for this plan.");
  const url = await paddle.createPortalSession(rt, customer.paddle_customer_id, ent.paddle_subscription_id ? [ent.paddle_subscription_id] : []);
  const res: PortalResponse = { url };
  return json(res);
});
