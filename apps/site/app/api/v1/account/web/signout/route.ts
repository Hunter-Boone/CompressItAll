import { WebSignoutRequestSchema, type OkResponse } from "@cia/api-types";

import { ownedEntitlement } from "@/lib/account";
import { route } from "@/lib/api";
import { json, readJson } from "@/lib/http";
import { requireSession } from "@/lib/session";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export const POST = route({ browserOnly: true }, async ({ req, rt }) => {
  const session = await requireSession(rt, req);
  const body = await readJson(req, WebSignoutRequestSchema);
  const ent = await ownedEntitlement(rt, session, body.entitlement_id);
  const n = await rt.db.deactivateWebDevices(ent.id, "dashboard");
  await rt.db.audit({ actor: "dashboard", action: "web.signed_out", entitlement_id: ent.id, detail: { count: n, email: session.email } });
  const res: OkResponse = { ok: true };
  return json(res);
});
