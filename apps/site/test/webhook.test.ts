import { beforeEach, describe, expect, it } from "vitest";

import { grantsAccess } from "../lib/entitlements";
import { signForTest } from "../lib/paddle-signature";
import { GET as reprocess } from "../app/api/v1/cron/reprocess-webhooks/route";
import { POST as activate } from "../app/api/v1/license/activate/route";
import { POST as refresh } from "../app/api/v1/license/refresh/route";
import { DEVICE_A, PRICE_LIFETIME, WEBHOOK_SECRET, activeSubscription, call, deliver, event, fixture, makeRequest, makeRuntime, seedPaddle, type TestRuntime } from "./helpers";

let rt: TestRuntime;

beforeEach(() => {
  rt = makeRuntime();
  seedPaddle(rt.net);
});

async function buyLifetime() {
  const res = await deliver(rt, event("transaction.completed.lifetime"));
  expect(res.status).toBe(200);
  const ent = await rt.db.findEntitlementByTransactionId("txn_01test_lifetime");
  expect(ent).not.toBeNull();
  return ent!;
}

async function buyYearly() {
  const res = await deliver(rt, event("transaction.completed.yearly"));
  expect(res.status).toBe(200);
  const ent = await rt.db.findEntitlementBySubscriptionId("sub_01test_yearly");
  expect(ent).not.toBeNull();
  return ent!;
}

describe("signature", () => {
  it("rejects a bad signature with 401 and no body", async () => {
    const res = await deliver(rt, event("transaction.completed.lifetime"), { secret: "pdl_ntfset_wrong_secret_value_xxx" });
    expect(res.status).toBe(401);
    expect(await res.text()).toBe("");
    expect(rt.db.events).toHaveLength(0);
  });
  it("rejects a missing header", async () => {
    expect((await deliver(rt, event("transaction.completed.lifetime"), { header: null })).status).toBe(401);
  });
  it("rejects a timestamp older than 30 s", async () => {
    const ts = Math.floor(rt.now().getTime() / 1000) - 31;
    expect((await deliver(rt, event("transaction.completed.lifetime"), { ts })).status).toBe(401);
  });
  it("accepts a timestamp 30 s old and rejects one 6 s in the future", async () => {
    const nowS = Math.floor(rt.now().getTime() / 1000);
    expect((await deliver(rt, event("transaction.completed.lifetime"), { ts: nowS - 30 })).status).toBe(200);
    expect((await deliver(rt, event("transaction.completed.lifetime"), { ts: nowS + 6 })).status).toBe(401);
  });
  it("accepts when any h1 matches (secret rotation)", async () => {
    const payload = event("transaction.completed.lifetime");
    const raw = JSON.stringify(payload);
    const ts = Math.floor(rt.now().getTime() / 1000);
    const header = signForTest(WEBHOOK_SECRET, raw, ts, ["0".repeat(64)]);
    expect(header.split("h1=").length).toBe(3);
    const res = await deliver(rt, payload, { header });
    expect(res.status).toBe(200);
  });
  it("verifies the raw body, so a reformatted body fails", async () => {
    const payload = event("transaction.completed.lifetime");
    const ts = Math.floor(rt.now().getTime() / 1000);
    const header = signForTest(WEBHOOK_SECRET, JSON.stringify(payload, null, 2), ts);
    expect((await deliver(rt, payload, { header })).status).toBe(401);
  });
});

describe("transaction.completed", () => {
  it("lifetime: creates customer, entitlement, key and sends one purchase email", async () => {
    const ent = await buyLifetime();
    expect(ent.plan).toBe("lifetime");
    expect(ent.status).toBe("active");
    expect(ent.paddle_price_id).toBe(PRICE_LIFETIME);
    expect(ent.access_until).toBeNull();
    expect(ent.key4).toHaveLength(5);
    const customer = await rt.db.findCustomerById(ent.customer_id);
    expect(customer?.email).toBe("margaret@example.com");
    expect(rt.net.emails).toHaveLength(1);
    expect(rt.net.emails[0]!.to).toBe("margaret@example.com");
    expect(rt.net.emails[0]!.subject).toContain("key");
    expect(rt.net.emails[0]!.text).toMatch(/[A-Z2-9]{5}-[A-Z2-9]{5}-[A-Z2-9]{5}-[A-Z2-9]{5}/);
    expect(rt.net.emails[0]!.text).toContain("#key=");
    expect(rt.db.events[0]!.status).toBe("processed");
  });
  it("yearly: fetches the subscription for access_until", async () => {
    const ent = await buyYearly();
    expect(ent.plan).toBe("yearly");
    expect(ent.status).toBe("active");
    expect(ent.access_until).toBe("2027-10-02T11:58:00Z");
    expect(ent.paddle_transaction_id).toBe("txn_01test_yearly");
    expect(rt.net.emails).toHaveLength(1);
  });
  it("duplicate delivery is a no-op", async () => {
    const payload = event("transaction.completed.lifetime");
    expect((await deliver(rt, payload)).status).toBe(200);
    const again = await deliver(rt, payload);
    expect(again.status).toBe(200);
    expect(await again.json()).toEqual({ ok: true, duplicate: true });
    expect(rt.db.entitlements).toHaveLength(1);
    expect(rt.net.emails).toHaveLength(1);
    expect(rt.net.calls.filter((c) => c.url.includes("/customers/"))).toHaveLength(1);
  });
  it("redelivery under a new event id is also a no-op (entity-level idempotency)", async () => {
    await buyLifetime();
    expect((await deliver(rt, event("transaction.completed.lifetime"))).status).toBe(200);
    expect(rt.db.entitlements).toHaveLength(1);
    expect(rt.net.emails).toHaveLength(1);
  });
  it("renewal never creates a second entitlement or email, and extends access", async () => {
    const ent = await buyYearly();
    rt.net.subscriptions.set("sub_01test_yearly", activeSubscription("2027-10-02T11:58:00Z", "2028-10-02T11:58:00Z"));
    const res = await deliver(rt, event("transaction.completed.renewal"));
    expect(res.status).toBe(200);
    expect(rt.db.entitlements).toHaveLength(1);
    expect(rt.net.emails).toHaveLength(1);
    const after = await rt.db.findEntitlementById(ent.id);
    expect(after!.access_until).toBe("2028-10-02T11:58:00Z");
    expect(after!.status).toBe("active");
  });
  it("a renewal that arrives before the purchase is processed without creating anything", async () => {
    const res = await deliver(rt, event("transaction.completed.renewal"));
    expect(res.status).toBe(200);
    expect(rt.db.entitlements).toHaveLength(0);
    expect(rt.db.events[0]!.status).toBe("processed");
  });
  it("ignores an unknown price and non-completed transaction events", async () => {
    expect((await deliver(rt, event("transaction.completed.unknown_price"))).status).toBe(200);
    expect((await deliver(rt, event("transaction.paid"))).status).toBe(200);
    expect(rt.db.entitlements).toHaveLength(0);
    expect(rt.db.events.map((e) => e.status)).toEqual(["ignored", "ignored"]);
  });
  it("returns 500 and records the error when Paddle is down, then the cron reprocesses it", async () => {
    rt.net.paddleDown = true;
    const res = await deliver(rt, event("transaction.completed.lifetime"));
    expect(res.status).toBe(500);
    expect(rt.db.events[0]!.status).toBe("error");
    expect(rt.db.events[0]!.attempts).toBe(1);
    expect(rt.db.events[0]!.last_error).toContain("Paddle");
    rt.net.paddleDown = false;
    const cron = await call(reprocess, makeRequest("GET", "/api/v1/cron/reprocess-webhooks", undefined, { origin: null, headers: { authorization: `Bearer ${rt.env.CRON_SECRET}` } }));
    expect(cron.status).toBe(200);
    expect((await cron.json()).counts.reprocessed).toBe(1);
    expect(rt.db.events[0]!.status).toBe("processed");
    expect(rt.db.entitlements).toHaveLength(1);
    expect(rt.net.emails).toHaveLength(1);
  });
  it("dead-letters after 20 attempts with one alert email", async () => {
    rt.net.paddleDown = true;
    await deliver(rt, event("transaction.completed.lifetime"));
    const cronReq = () => makeRequest("GET", "/api/v1/cron/reprocess-webhooks", undefined, { origin: null, headers: { authorization: `Bearer ${rt.env.CRON_SECRET}` } });
    for (let i = 0; i < 25; i++) await call(reprocess, cronReq());
    expect(rt.db.events[0]!.attempts).toBe(20);
    const alerts = rt.net.emails.filter((e) => e.to === "alerts@smidge.test");
    expect(alerts).toHaveLength(1);
  });
  it("cron rejects a wrong secret", async () => {
    const res = await call(reprocess, makeRequest("GET", "/api/v1/cron/reprocess-webhooks", undefined, { origin: null, headers: { authorization: "Bearer nope" } }));
    expect(res.status).toBe(401);
  });
});

describe("subscription.*", () => {
  it("past_due keeps access for 30 days past the period end", async () => {
    const ent = await buyYearly();
    rt.setNow(new Date("2027-10-03T00:00:00Z"));
    rt.net.subscriptions.set("sub_01test_yearly", activeSubscription("2026-10-02T11:58:00Z", "2027-10-02T11:58:00Z", { status: "past_due" }));
    expect((await deliver(rt, event("subscription.updated.past_due"))).status).toBe(200);
    const after = (await rt.db.findEntitlementById(ent.id))!;
    expect(after.status).toBe("past_due");
    expect(after.access_until).toBe("2027-11-01T11:58:00.000Z");
    expect(grantsAccess(after, rt.now())).toBe(true);
    // activation still works
    const res = await call(activate, makeRequest("POST", "/api/v1/license/activate", { product_key: keyOf(rt, after), kind: "desktop", device_hash: DEVICE_A, device_name: "PC", platform: "windows", app_version: "1.0.0" }, { origin: null }));
    expect(res.status).toBe(200);
    rt.setNow(new Date("2027-11-02T00:00:00Z"));
    expect(grantsAccess(after, rt.now())).toBe(false);
  });
  it("canceled ends access at canceled_at; the app's refresh answers ended", async () => {
    const ent = await buyYearly();
    const act = await call(activate, makeRequest("POST", "/api/v1/license/activate", { product_key: keyOf(rt, ent), kind: "desktop", device_hash: DEVICE_A, device_name: "PC", platform: "windows", app_version: "1.0.0" }, { origin: null }));
    const { token } = await act.json();
    rt.net.subscriptions.set("sub_01test_yearly", activeSubscription("2026-10-02T11:58:00Z", "2027-10-02T11:58:00Z", { status: "canceled", canceled_at: "2027-11-05T09:00:00Z", current_billing_period: null }));
    rt.setNow(new Date("2027-11-05T09:01:00Z"));
    expect((await deliver(rt, event("subscription.updated.canceled"))).status).toBe(200);
    const after = (await rt.db.findEntitlementById(ent.id))!;
    expect(after.status).toBe("ended");
    expect(after.access_until).toBe("2027-11-05T09:00:00Z");
    expect(grantsAccess(after, rt.now())).toBe(false);
    const r = await call(refresh, makeRequest("POST", "/api/v1/license/refresh", { token }, { origin: null }));
    expect(await r.json()).toMatchObject({ status: "ended", at: "2027-11-05T09:00:00Z" });
    // access_ended email sent once
    expect(rt.net.emails.filter((e) => e.subject.includes("ended"))).toHaveLength(1);
  });
  it("scheduled cancel sets cancel_at and emails once, even on redelivery", async () => {
    const ent = await buyYearly();
    rt.net.subscriptions.set("sub_01test_yearly", activeSubscription("2026-10-02T11:58:00Z", "2027-10-02T11:58:00Z", { scheduled_change: { action: "cancel", effective_at: "2027-10-02T11:58:00Z", resume_at: null } }));
    expect((await deliver(rt, event("subscription.updated.scheduled_cancel"))).status).toBe(200);
    expect((await deliver(rt, event("subscription.updated.scheduled_cancel"))).status).toBe(200);
    const after = (await rt.db.findEntitlementById(ent.id))!;
    expect(after.status).toBe("active");
    expect(after.cancel_at).toBe("2027-10-02T11:58:00Z");
    expect(grantsAccess(after, rt.now())).toBe(true);
    const cancelMails = rt.net.emails.filter((e) => e.subject.includes("won't renew"));
    expect(cancelMails).toHaveLength(1);
    expect(cancelMails[0]!.text).toContain("2 Oct 2027");
  });
  it("out-of-order: a stale canceled event converges on the refetched (active) truth", async () => {
    const ent = await buyYearly();
    // Paddle says active now; an old `subscription.canceled` payload arrives late.
    expect((await deliver(rt, event("subscription.updated.canceled", { occurredAt: "2026-10-02T11:59:00Z" }))).status).toBe(200);
    const after = (await rt.db.findEntitlementById(ent.id))!;
    expect(after.status).toBe("active");
    expect(after.access_until).toBe("2027-10-02T11:58:00Z");
  });
  it("out-of-order: subscription event before the purchase is a no-op, purchase then projects the current state", async () => {
    rt.net.subscriptions.set("sub_01test_yearly", activeSubscription("2026-10-02T11:58:00Z", "2027-10-02T11:58:00Z", { status: "past_due" }));
    expect((await deliver(rt, event("subscription.updated.past_due"))).status).toBe(200);
    expect(rt.db.entitlements).toHaveLength(0);
    const ent = await buyYearly();
    expect(ent.status).toBe("past_due");
    expect(ent.access_until).toBe("2027-11-01T11:58:00.000Z");
  });
  it("a subscription event never un-revokes a refunded entitlement", async () => {
    const ent = await buyYearly();
    await rt.db.updateEntitlement(ent.id, { status: "revoked", revoked_reason: "refund", revoked_at: rt.now().toISOString() });
    expect((await deliver(rt, event("subscription.updated.past_due"))).status).toBe(200);
    expect((await rt.db.findEntitlementById(ent.id))!.status).toBe("revoked");
  });
});

describe("adjustments", () => {
  it("pending refund does nothing; approved refund revokes and emails once", async () => {
    const ent = await buyLifetime();
    const act = await call(activate, makeRequest("POST", "/api/v1/license/activate", { product_key: keyOf(rt, ent), kind: "desktop", device_hash: DEVICE_A, device_name: "PC", platform: "windows", app_version: "1.0.0" }, { origin: null }));
    const { token } = await act.json();

    expect((await deliver(rt, event("adjustment.created.refund_pending"))).status).toBe(200);
    expect((await rt.db.findEntitlementById(ent.id))!.status).toBe("active");
    expect(rt.db.events.at(-1)!.status).toBe("ignored");
    expect(rt.net.emails).toHaveLength(1);

    expect((await deliver(rt, event("adjustment.updated.refund_approved"))).status).toBe(200);
    const after = (await rt.db.findEntitlementById(ent.id))!;
    expect(after.status).toBe("revoked");
    expect(after.revoked_reason).toBe("refund");
    expect(after.revoked_at).toBe("2026-10-05T10:30:00Z");
    expect(rt.net.emails).toHaveLength(2);
    expect(rt.net.emails[1]!.subject).toContain("refund");

    // Same adjustment again under a new event id: no second email, still revoked.
    expect((await deliver(rt, event("adjustment.updated.refund_approved"))).status).toBe(200);
    expect(rt.net.emails).toHaveLength(2);
    expect(rt.db.appliedAdjustments).toHaveLength(1);

    const r = await call(refresh, makeRequest("POST", "/api/v1/license/refresh", { token }, { origin: null }));
    expect(await r.json()).toEqual({ status: "revoked", reason: "refund", at: "2026-10-05T10:30:00Z" });
    const a2 = await call(activate, makeRequest("POST", "/api/v1/license/activate", { product_key: keyOf(rt, ent), kind: "desktop", device_hash: DEVICE_A, device_name: "PC", platform: "windows", app_version: "1.0.0" }, { origin: null }));
    expect(a2.status).toBe(403);
    expect((await a2.json()).error).toBe("revoked");
  });
  it("partial refund is audit-only", async () => {
    const ent = await buyLifetime();
    expect((await deliver(rt, event("adjustment.updated.refund_partial"))).status).toBe(200);
    expect((await rt.db.findEntitlementById(ent.id))!.status).toBe("active");
    expect(rt.db.appliedAdjustments[0]!.action).toBe("refund_partial");
    expect(rt.net.emails).toHaveLength(1);
  });
  it("refund of a renewal transaction revokes the yearly entitlement via its subscription", async () => {
    const ent = await buyYearly();
    const payload = event("adjustment.updated.refund_approved", { data: { transaction_id: "txn_01test_renewal", id: "adj_01test_renewal_refund" } });
    expect((await deliver(rt, payload)).status).toBe(200);
    expect((await rt.db.findEntitlementById(ent.id))!.status).toBe("revoked");
  });
  it("chargeback revokes without email; chargeback_reverse restores a lifetime plan", async () => {
    const ent = await buyLifetime();
    expect((await deliver(rt, event("adjustment.updated.chargeback"))).status).toBe(200);
    let after = (await rt.db.findEntitlementById(ent.id))!;
    expect(after.status).toBe("revoked");
    expect(after.revoked_reason).toBe("chargeback");
    expect(rt.net.emails).toHaveLength(1);
    expect((await deliver(rt, event("adjustment.updated.chargeback_reverse"))).status).toBe(200);
    after = (await rt.db.findEntitlementById(ent.id))!;
    expect(after.status).toBe("active");
    expect(after.revoked_reason).toBeNull();
    expect(after.revoked_at).toBeNull();
  });
  it("chargeback_reverse on a yearly plan re-projects from the subscription", async () => {
    const ent = await buyYearly();
    const cb = event("adjustment.updated.chargeback", { data: { transaction_id: "txn_01test_yearly", id: "adj_01test_cb_yearly" } });
    expect((await deliver(rt, cb)).status).toBe(200);
    expect((await rt.db.findEntitlementById(ent.id))!.status).toBe("revoked");
    rt.net.subscriptions.set("sub_01test_yearly", activeSubscription("2026-10-02T11:58:00Z", "2027-10-02T11:58:00Z", { status: "past_due" }));
    const rev = event("adjustment.updated.chargeback_reverse", { data: { transaction_id: "txn_01test_yearly", id: "adj_01test_cbr_yearly" } });
    expect((await deliver(rt, rev)).status).toBe(200);
    const after = (await rt.db.findEntitlementById(ent.id))!;
    expect(after.status).toBe("past_due");
    expect(after.revoked_reason).toBeNull();
  });
  it("an adjustment for an unknown transaction is ignored", async () => {
    rt.net.transactions.set("txn_01test_lifetime", { id: "txn_01test_lifetime", subscription_id: null });
    expect((await deliver(rt, event("adjustment.updated.refund_approved"))).status).toBe(200);
    expect(rt.db.events[0]!.status).toBe("ignored");
  });
});

describe("customer.updated", () => {
  it("updates the stored email (lowercased)", async () => {
    const ent = await buyLifetime();
    expect((await deliver(rt, event("customer.updated"))).status).toBe(200);
    expect((await rt.db.findCustomerById(ent.customer_id))!.email).toBe("margaret.new@example.com");
  });
});

describe("envelope", () => {
  it("rejects non-JSON and non-event bodies with 400 after a valid signature", async () => {
    const ts = Math.floor(rt.now().getTime() / 1000);
    const raw = "not json";
    const res = await fetchWebhook(raw, signForTest(WEBHOOK_SECRET, raw, ts));
    expect(res.status).toBe(400);
    const raw2 = JSON.stringify({ hello: "world" });
    expect((await fetchWebhook(raw2, signForTest(WEBHOOK_SECRET, raw2, ts))).status).toBe(400);
  });
  it("fixtures carry Paddle's envelope fields", () => {
    for (const name of ["transaction.completed.lifetime", "subscription.updated.past_due", "adjustment.updated.refund_approved", "customer.updated"]) {
      const f = fixture(name);
      expect(typeof f.event_id).toBe("string");
      expect(typeof f.notification_id).toBe("string");
      expect(typeof f.occurred_at).toBe("string");
    }
  });
});

async function fetchWebhook(raw: string, header: string): Promise<Response> {
  const { POST } = await import("../app/api/v1/webhooks/paddle/route");
  return POST(new Request("http://localhost:3031/api/v1/webhooks/paddle", { method: "POST", headers: { "paddle-signature": header, "content-type": "application/json" }, body: raw }));
}

/** The display key for an entitlement, straight from the purchase email. */
export function keyOf(rt: TestRuntime, ent: { id: string }): string {
  const { decryptDisplayKey } = requireWebhook();
  const row = rt.db.entitlements.find((e) => e.id === ent.id)!;
  return decryptDisplayKey(rt, row);
}

import * as webhookLib from "../lib/webhook";
function requireWebhook() {
  return webhookLib;
}
