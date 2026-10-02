// Paddle webhook processing (DESIGN.md 5.8.2). `processEvent` is called by the route (within a
// 3.5 s budget) and by the reprocess cron. Every branch is idempotent: entity-level unique keys,
// applied_adjustments, email_log (kind, ref) and subscription projection from a refetch.

import { PlatformSchema, productKey, type Plan, type Platform } from "@cia/api-types";

import { accessEndedEmail } from "../emails/access-ended";
import { cancelScheduledEmail } from "../emails/cancel-scheduled";
import { purchaseKeyEmail } from "../emails/purchase-key";
import { refundDoneEmail } from "../emails/refund-done";
import type { Entitlement, PaddleEvent } from "./db";
import { UniqueViolation } from "./db";
import { sendOnce } from "./email";
import { projectSubscription } from "./entitlements";
import { newEntitlementId } from "./ids";
import { decryptKey, generateStoredKey } from "./keys";
import { paddle, type PaddleSubscription } from "./paddle";
import { secretBytes } from "./env";
import { mintToken } from "./tokens";
import type { Runtime } from "./runtime";

export type Outcome = "processed" | "ignored";

interface EventEnvelope {
  event_id: string;
  event_type: string;
  occurred_at: string;
  data: Record<string, unknown>;
}

export function parseEnvelope(raw: unknown): EventEnvelope | null {
  if (typeof raw !== "object" || raw === null) return null;
  const r = raw as Record<string, unknown>;
  if (typeof r.event_id !== "string" || typeof r.event_type !== "string" || typeof r.occurred_at !== "string") return null;
  if (typeof r.data !== "object" || r.data === null) return null;
  return { event_id: r.event_id, event_type: r.event_type, occurred_at: r.occurred_at, data: r.data as Record<string, unknown> };
}

export async function processEvent(rt: Runtime, ev: PaddleEvent): Promise<Outcome> {
  const env = parseEnvelope(ev.payload);
  if (!env) throw new Error("payload is not a Paddle event envelope");
  const type = env.event_type;
  if (type === "transaction.completed") return onTransactionCompleted(rt, env);
  if (type.startsWith("subscription.")) return onSubscriptionEvent(rt, env);
  if (type === "adjustment.created" || type === "adjustment.updated") return onAdjustment(rt, env);
  if (type === "customer.updated") return onCustomerUpdated(rt, env);
  return "ignored";
}

// ---- transaction.completed --------------------------------------------------

interface TxnData {
  id: string;
  origin: string;
  customer_id: string | null;
  subscription_id: string | null;
  custom_data: Record<string, unknown> | null;
  items: { price?: { id?: string } }[];
}

async function onTransactionCompleted(rt: Runtime, env: EventEnvelope): Promise<Outcome> {
  const txn = env.data as unknown as TxnData;
  if (typeof txn.id !== "string") throw new Error("transaction.completed without data.id");

  if (txn.origin === "subscription_recurring") {
    if (!txn.subscription_id) return "ignored";
    const ent = await rt.db.findEntitlementBySubscriptionId(txn.subscription_id);
    if (!ent) return "processed"; // the purchase handler will project when it runs
    const sub = await paddle.getSubscription(rt, txn.subscription_id);
    await applyProjection(rt, ent, sub, env.occurred_at);
    return "processed";
  }
  if (txn.origin !== "web" && txn.origin !== "api") return "ignored";

  const priceIds = (txn.items ?? []).map((i) => i.price?.id).filter((p): p is string => typeof p === "string");
  let plan: Plan | null = null;
  let priceId = "";
  if (priceIds.includes(rt.env.PADDLE_PRICE_ID_LIFETIME)) {
    plan = "lifetime";
    priceId = rt.env.PADDLE_PRICE_ID_LIFETIME;
  } else if (priceIds.includes(rt.env.PADDLE_PRICE_ID_YEARLY)) {
    plan = "yearly";
    priceId = rt.env.PADDLE_PRICE_ID_YEARLY;
  }
  if (plan === null) return "ignored";

  // Idempotency on the transaction id (and subscription id for yearly).
  let ent = await rt.db.findEntitlementByTransactionId(txn.id);
  if (!ent && plan === "yearly" && txn.subscription_id) ent = await rt.db.findEntitlementBySubscriptionId(txn.subscription_id);

  if (!ent) {
    if (!txn.customer_id) throw new Error(`transaction ${txn.id} has no customer_id`);
    const customer = await paddle.getCustomer(rt, txn.customer_id);
    const email = customer.email.trim().toLowerCase();
    const customerRow = await rt.db.upsertCustomer(customer.id, email);

    let projection: { status: Entitlement["status"]; access_until: string | null; cancel_at: string | null } = { status: "active", access_until: null, cancel_at: null };
    if (plan === "yearly") {
      if (!txn.subscription_id) throw new Error(`yearly transaction ${txn.id} has no subscription_id`);
      const sub = await paddle.getSubscription(rt, txn.subscription_id);
      projection = projectSubscription(sub);
    }

    const key = generateStoredKey(secretBytes(rt.env, "KEY_ENCRYPTION_KEY"));
    try {
      ent = await rt.db.insertEntitlement({
        id: newEntitlementId(rt.now().getTime()),
        customer_id: customerRow.id,
        plan,
        status: projection.status,
        product_key_hash: key.hash,
        product_key_enc: key.enc,
        key4: key.key4,
        max_devices: 3,
        max_web: 3,
        paddle_transaction_id: txn.id,
        paddle_subscription_id: plan === "yearly" ? txn.subscription_id : null,
        paddle_price_id: priceId,
        access_until: projection.access_until,
        cancel_at: projection.cancel_at,
        revoked_at: null,
        revoked_reason: null,
        last_event_at: env.occurred_at,
      });
    } catch (err) {
      if (!(err instanceof UniqueViolation)) throw err;
      // A concurrent delivery got there first; load its row and carry on with linking/email.
      ent = (await rt.db.findEntitlementByTransactionId(txn.id)) ?? (txn.subscription_id ? await rt.db.findEntitlementBySubscriptionId(txn.subscription_id) : null);
      if (!ent) throw err;
    }
    await rt.db.audit({ actor: "webhook", action: "entitlement.created", entitlement_id: ent.id, detail: { plan, transaction_id: txn.id, event_id: env.event_id } });
  }

  await linkCheckoutAndClaim(rt, ent, txn.custom_data);

  // Purchase email, once per entitlement.
  const customer = await customerEmailById(rt, ent.customer_id);
  if (customer) {
    const productKey = decryptDisplayKey(rt, ent);
    await sendOnce(rt, "purchase_key", ent.id, customer, purchaseKeyEmail({ productKey, plan: ent.plan, siteUrl: rt.env.NEXT_PUBLIC_SITE_URL, appUrl: rt.env.NEXT_PUBLIC_APP_URL }));
  }
  return "processed";
}

async function linkCheckoutAndClaim(rt: Runtime, ent: Entitlement, customData: Record<string, unknown> | null): Promise<void> {
  const checkoutId = typeof customData?.checkout_id === "string" ? customData.checkout_id : null;
  if (!checkoutId) return;
  const checkout = await rt.db.findCheckout(checkoutId);
  if (!checkout) return;
  if (checkout.entitlement_id !== ent.id) await rt.db.updateCheckout(checkout.id, { entitlement_id: ent.id });
  const claimId = checkout.claim_id ?? (typeof customData?.claim_id === "string" ? customData.claim_id : null);
  if (!claimId) return;
  const claim = await rt.db.findClaim(claimId);
  if (!claim || claim.status === "fulfilled") return;
  // The claim's device becomes the first activation (DESIGN.md 5.6.1); a slot is used only if free.
  const result = await rt.db.activateDevice({
    entitlementId: ent.id,
    kind: claim.kind,
    deviceHash: claim.device_hash,
    deviceName: claim.device_name,
    platform: claimPlatform(claim.platform),
    appVersion: null,
  });
  if (!result.ok) throw new Error(`claim ${claim.id}: device limit reached on a new entitlement`);
  await rt.db.updateClaim(claim.id, { status: "fulfilled", entitlement_id: ent.id });
  await rt.db.audit({ actor: "webhook", action: "claim.fulfilled", entitlement_id: ent.id, detail: { claim_id: claim.id } });
}

function claimPlatform(p: string | null): Platform | null {
  const parsed = PlatformSchema.safeParse(p);
  return parsed.success ? parsed.data : null;
}

export function decryptDisplayKey(rt: Runtime, ent: Entitlement): string {
  return productKey.formatDisplay(decryptKey(ent.product_key_enc, secretBytes(rt.env, "KEY_ENCRYPTION_KEY")));
}

async function customerEmailById(rt: Runtime, customerId: string): Promise<string | null> {
  const row = await rt.db.findCustomerById(customerId);
  return row?.email ?? null;
}

/** Mint a token for the claim's device, used by GET /claims/{id}. */
export async function tokenForClaimDevice(rt: Runtime, ent: Entitlement, kind: "desktop" | "web", deviceHash: string): Promise<string> {
  return mintToken(rt.env, { entitlementId: ent.id, plan: ent.plan, kind, deviceHash, accessUntil: ent.access_until, key4: ent.key4 }, rt.now());
}

// ---- subscription.* ---------------------------------------------------------

async function onSubscriptionEvent(rt: Runtime, env: EventEnvelope): Promise<Outcome> {
  const subId = env.data.id;
  if (typeof subId !== "string") throw new Error(`${env.event_type} without data.id`);
  const ent = await rt.db.findEntitlementBySubscriptionId(subId);
  if (!ent) return "processed"; // transaction.completed will fetch the subscription itself
  const sub = await paddle.getSubscription(rt, subId);
  await applyProjection(rt, ent, sub, env.occurred_at);
  return "processed";
}

/** Write the subscription's current truth onto the entitlement; never trusts the event payload. */
export async function applyProjection(rt: Runtime, ent: Entitlement, sub: PaddleSubscription, occurredAt: string): Promise<Entitlement> {
  if (ent.status === "revoked") {
    // A refund or chargeback wins over anything the subscription says.
    return rt.db.updateEntitlement(ent.id, { last_event_at: newest(ent.last_event_at, occurredAt) });
  }
  const p = projectSubscription(sub);
  const updated = await rt.db.updateEntitlement(ent.id, {
    status: p.status,
    access_until: p.access_until,
    cancel_at: p.cancel_at,
    last_event_at: newest(ent.last_event_at, occurredAt),
  });
  if (p.status !== ent.status || p.access_until !== ent.access_until || p.cancel_at !== ent.cancel_at) {
    await rt.db.audit({ actor: "webhook", action: "entitlement.projected", entitlement_id: ent.id, detail: { from: ent.status, to: p.status, access_until: p.access_until, cancel_at: p.cancel_at } });
  }
  const email = await customerEmailById(rt, ent.customer_id);
  if (email && p.cancel_at) {
    await sendOnce(rt, "cancel_scheduled", `${sub.id}:${p.cancel_at}`, email, cancelScheduledEmail({ endsOn: fmtDate(p.cancel_at), siteUrl: rt.env.NEXT_PUBLIC_SITE_URL }));
  }
  if (email && p.status === "ended" && p.access_until && new Date(p.access_until).getTime() <= rt.now().getTime()) {
    await sendOnce(rt, "access_ended", `${ent.id}:${p.access_until}`, email, accessEndedEmail({ endedOn: fmtDate(p.access_until), siteUrl: rt.env.NEXT_PUBLIC_SITE_URL }));
  }
  return updated;
}

function newest(a: string | null, b: string): string {
  if (!a) return b;
  return new Date(a).getTime() >= new Date(b).getTime() ? a : b;
}

export function fmtDate(iso: string): string {
  return new Date(iso).toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" });
}

// ---- adjustment.* -----------------------------------------------------------

interface AdjustmentData {
  id: string;
  action: string;
  status: string;
  type?: string;
  transaction_id: string | null;
  subscription_id: string | null;
}

async function onAdjustment(rt: Runtime, env: EventEnvelope): Promise<Outcome> {
  const adj = env.data as unknown as AdjustmentData;
  if (typeof adj.id !== "string") throw new Error(`${env.event_type} without data.id`);
  if (adj.status !== "approved") return "ignored";
  const action = adj.action;
  if (action !== "refund" && action !== "chargeback" && action !== "chargeback_reverse") return "ignored";

  const ent = await findEntitlementForAdjustment(rt, adj);
  if (!ent) return "ignored";

  if (action === "refund") {
    const type = adj.type ?? "full";
    if (type !== "full") {
      // Partial refund: audit only, access unchanged.
      const fresh = await rt.db.insertAppliedAdjustment(adj.id, ent.id, "refund_partial");
      if (fresh) await rt.db.audit({ actor: "webhook", action: "adjustment.refund_partial", entitlement_id: ent.id, detail: { adjustment_id: adj.id } });
      return "processed";
    }
    const fresh = await rt.db.insertAppliedAdjustment(adj.id, ent.id, "refund");
    if (!fresh) return "processed";
    await rt.db.updateEntitlement(ent.id, { status: "revoked", revoked_reason: "refund", revoked_at: env.occurred_at, last_event_at: newest(ent.last_event_at, env.occurred_at) });
    await rt.db.audit({ actor: "webhook", action: "entitlement.revoked", entitlement_id: ent.id, detail: { reason: "refund", adjustment_id: adj.id } });
    const email = await customerEmailById(rt, ent.customer_id);
    if (email) await sendOnce(rt, "refund_done", adj.id, email, refundDoneEmail());
    return "processed";
  }

  if (action === "chargeback") {
    const fresh = await rt.db.insertAppliedAdjustment(adj.id, ent.id, "chargeback");
    if (!fresh) return "processed";
    await rt.db.updateEntitlement(ent.id, { status: "revoked", revoked_reason: "chargeback", revoked_at: env.occurred_at, last_event_at: newest(ent.last_event_at, env.occurred_at) });
    await rt.db.audit({ actor: "webhook", action: "entitlement.revoked", entitlement_id: ent.id, detail: { reason: "chargeback", adjustment_id: adj.id } });
    return "processed";
  }

  // chargeback_reverse: re-derive.
  const fresh = await rt.db.insertAppliedAdjustment(adj.id, ent.id, "chargeback_reverse");
  if (!fresh) return "processed";
  if (ent.plan === "lifetime") {
    await rt.db.updateEntitlement(ent.id, { status: "active", revoked_at: null, revoked_reason: null, last_event_at: newest(ent.last_event_at, env.occurred_at) });
  } else {
    const cleared = await rt.db.updateEntitlement(ent.id, { revoked_at: null, revoked_reason: null, status: "ended" });
    const sub = await paddle.getSubscription(rt, ent.paddle_subscription_id!);
    await applyProjection(rt, cleared, sub, env.occurred_at);
  }
  await rt.db.audit({ actor: "webhook", action: "entitlement.restored", entitlement_id: ent.id, detail: { adjustment_id: adj.id } });
  return "processed";
}

async function findEntitlementForAdjustment(rt: Runtime, adj: AdjustmentData): Promise<Entitlement | null> {
  if (adj.subscription_id) {
    const bySub = await rt.db.findEntitlementBySubscriptionId(adj.subscription_id);
    if (bySub) return bySub;
  }
  if (adj.transaction_id) {
    const byTxn = await rt.db.findEntitlementByTransactionId(adj.transaction_id);
    if (byTxn) return byTxn;
    // A renewal transaction: look up its subscription.
    const txn = await paddle.getTransaction(rt, adj.transaction_id);
    if (txn.subscription_id) return rt.db.findEntitlementBySubscriptionId(txn.subscription_id);
  }
  return null;
}

// ---- customer.updated -------------------------------------------------------

async function onCustomerUpdated(rt: Runtime, env: EventEnvelope): Promise<Outcome> {
  const id = env.data.id;
  const email = env.data.email;
  if (typeof id !== "string" || typeof email !== "string") throw new Error("customer.updated without id/email");
  await rt.db.updateCustomerEmail(id, email.trim().toLowerCase());
  return "processed";
}
