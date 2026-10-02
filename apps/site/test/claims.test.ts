import { beforeEach, describe, expect, it } from "vitest";

import { publicKeyFromSeed, verifyToken } from "@cia/api-types";

import { POST as checkoutCreate } from "../app/api/v1/checkout/route";
import { GET as checkoutStatus } from "../app/api/v1/checkout/[id]/route";
import { GET as claimStatus } from "../app/api/v1/claims/[id]/route";
import { POST as claimsCreate } from "../app/api/v1/claims/route";
import { POST as activate } from "../app/api/v1/license/activate/route";
import { APP_URL, DEVICE_A, DEVICE_B, DEVICE_C, DEVICE_D, PRICE_LIFETIME, PRICE_YEARLY, SITE_URL, call, cookieValue, deliver, event, makeRequest, makeRuntime, seedPaddle, type TestRuntime } from "./helpers";

let rt: TestRuntime;

beforeEach(() => {
  rt = makeRuntime();
  seedPaddle(rt.net);
});

describe("claims flow end to end", () => {
  it("create claim -> checkout -> webhook -> claim fulfilled with a token for the claim's device", async () => {
    // 1. the desktop app creates a claim (no Origin, CORS headers present for the web app)
    const c = await call(claimsCreate, makeRequest("POST", "/api/v1/claims", { kind: "desktop", device_hash: DEVICE_A, device_name: "Margaret's PC", platform: "windows" }, { origin: null }));
    expect(c.status).toBe(201);
    expect(c.headers.get("access-control-allow-origin")).toBe(APP_URL);
    const claim = await c.json();
    expect(claim.claim_id).toMatch(/^clm_/);
    expect(claim.buy_url).toBe(`${SITE_URL}/buy?claim=${claim.claim_id}`);
    expect(new Date(claim.expires_at).getTime() - rt.now().getTime()).toBe(2 * 3600 * 1000);

    // 2. polling before purchase: pending; wrong secret: 404; missing secret: 401
    const poll = (secret: string | null) => call(claimStatus, makeRequest("GET", `/api/v1/claims/${claim.claim_id}`, undefined, { origin: null, headers: secret ? { authorization: `Claim ${secret}` } : {} }), { id: claim.claim_id });
    expect(await (await poll(claim.claim_secret)).json()).toEqual({ status: "pending" });
    expect((await poll("wrong")).status).toBe(404);
    expect((await poll(null)).status).toBe(401);

    // 3. the browser creates a checkout for the claim
    const ck = await call(checkoutCreate, makeRequest("POST", "/api/v1/checkout", { plan: "lifetime", claim_id: claim.claim_id }));
    expect(ck.status).toBe(201);
    const { transaction_id, checkout_id } = await ck.json();
    expect(transaction_id).toMatch(/^txn_/);
    expect(checkout_id).toMatch(/^chk_/);
    const chkCookie = cookieValue(ck, "smg_chk");
    expect(chkCookie).toBeTruthy();
    expect(ck.headers.get("set-cookie")).toContain("Path=/api/v1/checkout");
    expect(ck.headers.get("set-cookie")).toContain("Max-Age=7200");
    const checkout = rt.db.checkouts[0]!;
    expect(checkout.paddle_transaction_id).toBe(transaction_id);
    // custom_data was set server-side
    expect(rt.net.createdTransactions[0]).toMatchObject({ items: [{ price_id: PRICE_LIFETIME, quantity: 1 }], custom_data: { checkout_id: checkout.id, claim_id: claim.claim_id }, collection_mode: "automatic" });

    // 4. the success page polls: pending until the webhook
    const status = (cookie: string | undefined) => call(checkoutStatus, makeRequest("GET", `/api/v1/checkout/${checkout.id}`, undefined, { cookie: cookie ? `smg_chk=${cookie}` : undefined }), { id: checkout.id });
    expect(await (await status(chkCookie!)).json()).toEqual({ status: "pending" });
    expect((await status(undefined)).status).toBe(401);

    // 5. Paddle's webhook with the server-set custom data
    const res = await deliver(rt, event("transaction.completed.lifetime", { data: { id: transaction_id, custom_data: { checkout_id: checkout.id, claim_id: claim.claim_id } } }));
    expect(res.status).toBe(200);
    const ent = rt.db.entitlements[0]!;

    // 6. checkout fulfilled with the key (visible for 2 h), claim fulfilled with a token for DEVICE_A
    const st = await (await status(chkCookie!)).json();
    expect(st).toMatchObject({ status: "fulfilled", plan: "lifetime" });
    expect(st.product_key).toMatch(/^[A-Z2-9]{5}-[A-Z2-9]{5}-[A-Z2-9]{5}-[A-Z2-9]{5}$/);
    const fulfilled = await (await poll(claim.claim_secret)).json();
    expect(fulfilled).toMatchObject({ status: "fulfilled", plan: "lifetime", product_key: st.product_key });
    const pub = await publicKeyFromSeed(Buffer.from(rt.env.LICENSE_SIGNING_KEY, "base64"));
    const v = await verifyToken(fulfilled.token, [["test", pub]]);
    expect(v.ok && v.payload.dev).toBe(DEVICE_A);
    expect(v.ok && v.payload.ent).toBe(ent.id);
    // the claim's device is the first activation and took one slot
    expect(rt.db.devices).toHaveLength(1);
    expect(rt.db.devices[0]).toMatchObject({ device_hash: DEVICE_A, device_name: "Margaret's PC", platform: "windows", kind: "desktop" });
    expect(rt.db.claims[0]!.status).toBe("fulfilled");
    expect(rt.db.checkouts[0]!.entitlement_id).toBe(ent.id);

    // 7. redelivery of the webhook changes nothing
    await deliver(rt, event("transaction.completed.lifetime", { data: { id: transaction_id, custom_data: { checkout_id: checkout.id, claim_id: claim.claim_id } } }));
    expect(rt.db.devices).toHaveLength(1);
    expect(rt.net.emails).toHaveLength(1);

    // 8. three more computers: the fourth is refused
    const key = st.product_key as string;
    for (const d of [DEVICE_B, DEVICE_C]) expect((await call(activate, makeRequest("POST", "/api/v1/license/activate", { product_key: key, kind: "desktop", device_hash: d, device_name: "x", platform: "linux", app_version: "1.0.0" }, { origin: null }))).status).toBe(200);
    expect((await call(activate, makeRequest("POST", "/api/v1/license/activate", { product_key: key, kind: "desktop", device_hash: DEVICE_D, device_name: "x", platform: "linux", app_version: "1.0.0" }, { origin: null }))).status).toBe(409);

    // 9. two hours later the success page no longer shows the key
    rt.advance(2 * 3600 * 1000 + 1000);
    expect((await status(chkCookie!)).status).toBe(410);
  });

  it("yearly checkout from the web app with a web claim", async () => {
    const c = await call(claimsCreate, makeRequest("POST", "/api/v1/claims", { kind: "web", device_hash: "w_abcdefghijklmnopqrstuvwxyz", platform: "web" }, { origin: APP_URL }));
    const claim = await c.json();
    const ck = await call(checkoutCreate, makeRequest("POST", "/api/v1/checkout", { plan: "yearly", claim_id: claim.claim_id }));
    expect(ck.status).toBe(201);
    expect(rt.net.createdTransactions[0]).toMatchObject({ items: [{ price_id: PRICE_YEARLY, quantity: 1 }] });
    const checkout = rt.db.checkouts[0]!;
    await deliver(rt, event("transaction.completed.yearly", { data: { custom_data: { checkout_id: checkout.id, claim_id: claim.claim_id } } }));
    const fulfilled = await (await call(claimStatus, makeRequest("GET", `/api/v1/claims/${claim.claim_id}`, undefined, { origin: APP_URL, headers: { authorization: `Claim ${claim.claim_secret}` } }), { id: claim.claim_id })).json();
    expect(fulfilled.status).toBe("fulfilled");
    expect(fulfilled.plan).toBe("yearly");
    const pub = await publicKeyFromSeed(Buffer.from(rt.env.LICENSE_SIGNING_KEY, "base64"));
    const v = await verifyToken(fulfilled.token, [["test", pub]]);
    expect(v.ok && v.payload.kind).toBe("web");
    expect(v.ok && v.payload.acc).toBe(Math.floor(new Date("2027-10-02T11:58:00Z").getTime() / 1000));
    expect(rt.db.devices[0]!.kind).toBe("web");
  });

  it("checkout without a claim works; expired or unknown claims are refused", async () => {
    const ok = await call(checkoutCreate, makeRequest("POST", "/api/v1/checkout", { plan: "lifetime" }));
    expect(ok.status).toBe(201);
    expect(rt.net.createdTransactions[0]).toMatchObject({ custom_data: { claim_id: null } });
    const unknown = await call(checkoutCreate, makeRequest("POST", "/api/v1/checkout", { plan: "lifetime", claim_id: "clm_01ARZ3NDEKTSV4RRFFQ69G5FAV" }));
    expect(unknown.status).toBe(404);
    const c = await call(claimsCreate, makeRequest("POST", "/api/v1/claims", { kind: "desktop", device_hash: DEVICE_A, platform: "linux" }, { origin: null }));
    const claim = await c.json();
    rt.advance(2 * 3600 * 1000 + 1);
    const expired = await call(checkoutCreate, makeRequest("POST", "/api/v1/checkout", { plan: "lifetime", claim_id: claim.claim_id }));
    expect(expired.status).toBe(410);
    expect((await expired.json()).error).toBe("claim_expired");
    const poll = await call(claimStatus, makeRequest("GET", `/api/v1/claims/${claim.claim_id}`, undefined, { origin: null, headers: { authorization: `Claim ${claim.claim_secret}` } }), { id: claim.claim_id });
    expect(await poll.json()).toEqual({ status: "expired" });
  });

  it("checkout needs a browser Origin; Paddle outage answers 502 and leaves no transaction id", async () => {
    expect((await call(checkoutCreate, makeRequest("POST", "/api/v1/checkout", { plan: "lifetime" }, { origin: null }))).status).toBe(403);
    expect((await call(checkoutCreate, makeRequest("POST", "/api/v1/checkout", { plan: "lifetime" }, { origin: "https://evil.example" }))).status).toBe(403);
    rt.net.paddleDown = true;
    const res = await call(checkoutCreate, makeRequest("POST", "/api/v1/checkout", { plan: "lifetime" }));
    expect(res.status).toBe(502);
    expect(rt.db.checkouts[0]!.paddle_transaction_id).toBeNull();
  });

  it("claims are rate limited 20 per hour per IP", async () => {
    for (let i = 0; i < 20; i++) expect((await call(claimsCreate, makeRequest("POST", "/api/v1/claims", { kind: "desktop", device_hash: DEVICE_A, platform: "linux" }, { origin: null }))).status).toBe(201);
    expect((await call(claimsCreate, makeRequest("POST", "/api/v1/claims", { kind: "desktop", device_hash: DEVICE_A, platform: "linux" }, { origin: null }))).status).toBe(429);
  });
});
