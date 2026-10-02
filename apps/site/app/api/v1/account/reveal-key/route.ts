import { RevealKeyRequestSchema, type RevealKeyResponse } from "@cia/api-types";

import { ownedEntitlement } from "@/lib/account";
import { route } from "@/lib/api";
import { json, readJson } from "@/lib/http";
import { requireSession } from "@/lib/session";
import { decryptDisplayKey } from "@/lib/webhook";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export const POST = route({ browserOnly: true }, async ({ req, rt }) => {
  const session = await requireSession(rt, req);
  const body = await readJson(req, RevealKeyRequestSchema);
  const ent = await ownedEntitlement(rt, session, body.entitlement_id);
  await rt.db.audit({ actor: "dashboard", action: "key.revealed", entitlement_id: ent.id, detail: { email: session.email } });
  const res: RevealKeyResponse = { product_key: decryptDisplayKey(rt, ent) };
  return json(res);
});
