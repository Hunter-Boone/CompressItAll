// Account dashboard projection (DESIGN.md 5.12) and ownership checks shared by /account/* routes.

import type { AccountEntitlement, AccountResponse } from "@cia/api-types";

import type { Entitlement, Session } from "./db";
import { ApiError } from "./http";
import { maskedKey } from "./keys";
import type { Runtime } from "./runtime";

export async function entitlementsForSession(rt: Runtime, session: Session): Promise<Entitlement[]> {
  const customers = await rt.db.findCustomersByEmail(session.email);
  return rt.db.listEntitlementsForCustomers(customers.map((c) => c.id));
}

/** The entitlement if it belongs to the signed-in email; 404 otherwise (never 403, to avoid an oracle). */
export async function ownedEntitlement(rt: Runtime, session: Session, entitlementId: string): Promise<Entitlement> {
  const ents = await entitlementsForSession(rt, session);
  const ent = ents.find((e) => e.id === entitlementId);
  if (!ent) throw new ApiError(404, "not_found", "That plan isn't on this account.");
  return ent;
}

export async function accountResponse(rt: Runtime, session: Session): Promise<AccountResponse> {
  const ents = await entitlementsForSession(rt, session);
  // revoked_at is an extra field for the dashboard's "Refunded on" line (the schema strips it).
  const entitlements: (AccountEntitlement & { revoked_at: string | null })[] = [];
  for (const e of ents) {
    const devices = await rt.db.listActiveDevices(e.id);
    entitlements.push({
      id: e.id,
      plan: e.plan,
      status: e.status,
      key_masked: maskedKey(e.key4),
      key4: e.key4,
      created_at: e.created_at,
      access_until: e.access_until,
      cancel_at: e.cancel_at,
      revoked_at: e.revoked_at,
      devices_max: e.max_devices,
      devices: devices
        .filter((d) => d.kind === "desktop")
        .map((d) => ({ id: d.id, kind: d.kind, name: d.device_name, platform: d.platform, app_version: d.app_version, activated_at: d.activated_at, last_seen_at: d.last_seen_at })),
      web_count: devices.filter((d) => d.kind === "web").length,
    });
  }
  return { email: session.email, entitlements };
}
