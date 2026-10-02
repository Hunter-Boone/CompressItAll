import { beforeEach, describe, expect, it } from "vitest";

import { verifyToken } from "@cia/api-types";

import { POST as activate } from "../app/api/v1/license/activate/route";
import { POST as deactivate } from "../app/api/v1/license/deactivate/route";
import { POST as refresh } from "../app/api/v1/license/refresh/route";
import { POST as resend } from "../app/api/v1/license/resend/route";
import { decryptDisplayKey } from "../lib/webhook";
import { APP_URL, DEVICE_A, DEVICE_B, DEVICE_C, DEVICE_D, WEB_1, WEB_2, WEB_3, WEB_4, call, deliver, event, makeRequest, makeRuntime, seedPaddle, type TestRuntime } from "./helpers";
import { publicKeyFromSeed } from "@cia/api-types";

let rt: TestRuntime;
let key: string;
let entId: string;

beforeEach(async () => {
  rt = makeRuntime();
  seedPaddle(rt.net);
  await deliver(rt, event("transaction.completed.lifetime"));
  const ent = rt.db.entitlements[0]!;
  entId = ent.id;
  key = decryptDisplayKey(rt, ent);
});

function activateReq(device: string, extra: Record<string, unknown> = {}, o: { ip?: string; origin?: string | null } = {}) {
  return makeRequest("POST", "/api/v1/license/activate", { product_key: key, kind: "desktop", device_hash: device, device_name: `PC ${device.slice(2, 3)}`, platform: "windows", app_version: "1.0.0", ...extra }, { origin: o.origin === undefined ? null : o.origin, ip: o.ip ?? "203.0.113.10" });
}

describe("POST /license/activate", () => {
  it("activates and returns a token signed for the device", async () => {
    const res = await call(activate, activateReq(DEVICE_A));
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body).toMatchObject({ plan: "lifetime", devices_used: 1, devices_max: 3 });
    const pub = await publicKeyFromSeed(Buffer.from(rt.env.LICENSE_SIGNING_KEY, "base64"));
    const v = await verifyToken(body.token, [["test", pub]]);
    expect(v.ok).toBe(true);
    if (v.ok) {
      expect(v.payload).toMatchObject({ ent: entId, plan: "lifetime", kind: "desktop", dev: DEVICE_A, acc: null });
      expect(v.payload.exp - v.payload.iat).toBe(45 * 86400);
      expect(v.payload.key4).toHaveLength(5);
    }
    expect(res.headers.get("access-control-allow-origin")).toBe(APP_URL);
  });
  it("accepts a lowercase, spaced key and rejects a typo with 404 key_not_found", async () => {
    const spaced = key.toLowerCase().replace(/-/g, " ");
    expect((await call(activate, activateReq(DEVICE_A, { product_key: spaced }))).status).toBe(200);
    const typo = key.slice(0, -1) + (key.endsWith("A") ? "B" : "A");
    const res = await call(activate, activateReq(DEVICE_B, { product_key: typo }));
    expect(res.status).toBe(404);
    expect((await res.json()).error).toBe("key_not_found");
    const unknown = await call(activate, activateReq(DEVICE_B, { product_key: "ABCDE-FGHJK-MNPQR-STUVW" }));
    expect(unknown.status).toBe(404);
  });
  it("returns 409 device_limit with the device list on the fourth computer", async () => {
    for (const d of [DEVICE_A, DEVICE_B, DEVICE_C]) expect((await call(activate, activateReq(d))).status).toBe(200);
    rt.advance(1000);
    const res = await call(activate, activateReq(DEVICE_D));
    expect(res.status).toBe(409);
    const body = await res.json();
    expect(body.error).toBe("device_limit");
    expect(body.devices).toHaveLength(3);
    expect(body.devices[0]).toMatchObject({ name: "PC a", platform: "windows" });
    expect(typeof body.devices[0].last_seen_at).toBe("string");
    expect(body.message).toContain("3 computers");
  });
  it("re-activating the same device uses no slot", async () => {
    for (const d of [DEVICE_A, DEVICE_B, DEVICE_C]) await call(activate, activateReq(d));
    const res = await call(activate, activateReq(DEVICE_A, { app_version: "1.0.1" }));
    expect(res.status).toBe(200);
    expect((await res.json()).devices_used).toBe(3);
    expect(rt.db.devices.filter((d) => d.deactivated_at === null)).toHaveLength(3);
    expect(rt.db.devices.find((d) => d.device_hash === DEVICE_A)!.app_version).toBe("1.0.1");
  });
  it("a removed computer frees its slot", async () => {
    for (const d of [DEVICE_A, DEVICE_B, DEVICE_C]) await call(activate, activateReq(d));
    const a = rt.db.devices.find((d) => d.device_hash === DEVICE_A)!;
    await rt.db.deactivateDevice(a.id, "dashboard");
    expect((await call(activate, activateReq(DEVICE_D))).status).toBe(200);
  });
  it("web activations evict the least recently used browser instead of refusing", async () => {
    const web = (h: string) => activateReq(h, { kind: "web", platform: "web", device_name: null });
    await call(activate, web(WEB_1));
    rt.advance(1000);
    await call(activate, web(WEB_2));
    rt.advance(1000);
    await call(activate, web(WEB_3));
    rt.advance(1000);
    // touch WEB_1 so WEB_2 becomes the LRU
    await call(activate, web(WEB_1));
    rt.advance(1000);
    const res = await call(activate, web(WEB_4));
    expect(res.status).toBe(200);
    expect((await res.json()).devices_used).toBe(3);
    const active = rt.db.devices.filter((d) => d.kind === "web" && d.deactivated_at === null).map((d) => d.device_hash).sort();
    expect(active).toEqual([WEB_1, WEB_3, WEB_4].sort());
    const evicted = rt.db.devices.find((d) => d.device_hash === WEB_2)!;
    expect(evicted.deactivated_by).toBe("evicted");
    // browsers never use computer slots
    for (const d of [DEVICE_A, DEVICE_B, DEVICE_C]) expect((await call(activate, activateReq(d))).status).toBe(200);
  });
  it("web tokens are valid 14 days", async () => {
    const res = await call(activate, activateReq(WEB_1, { kind: "web", platform: "web" }));
    const pub = await publicKeyFromSeed(Buffer.from(rt.env.LICENSE_SIGNING_KEY, "base64"));
    const v = await verifyToken((await res.json()).token, [["test", pub]]);
    expect(v.ok && v.payload.exp - v.payload.iat).toBe(14 * 86400);
  });
  it("rate limits 10 per hour per IP (and fails closed when the limiter breaks)", async () => {
    for (let i = 0; i < 10; i++) expect((await call(activate, activateReq(DEVICE_A))).status).toBe(200);
    const res = await call(activate, activateReq(DEVICE_A));
    expect(res.status).toBe(429);
    expect((await res.json()).error).toBe("rate_limited");
    rt.db.rateLimitBroken = true;
    const closed = await call(activate, activateReq(DEVICE_A, {}, { ip: "198.51.100.7" }));
    expect(closed.status).toBe(503);
    expect((await closed.json()).error).toBe("unavailable");
  });
  it("rejects a foreign Origin and accepts the app origin", async () => {
    expect((await call(activate, activateReq(DEVICE_A, {}, { origin: "https://evil.example" }))).status).toBe(403);
    expect((await call(activate, activateReq(DEVICE_A, {}, { origin: APP_URL }))).status).toBe(200);
  });
  it("rejects a bad body with 400", async () => {
    const res = await call(activate, makeRequest("POST", "/api/v1/license/activate", { product_key: key, kind: "desktop" }, { origin: null }));
    expect(res.status).toBe(400);
    expect((await res.json()).error).toBe("bad_request");
  });
});

describe("POST /license/refresh and /license/deactivate", () => {
  async function tokenFor(device: string) {
    const res = await call(activate, activateReq(device));
    return (await res.json()).token as string;
  }
  it("ok: touches the device and returns a fresh token", async () => {
    const token = await tokenFor(DEVICE_A);
    rt.advance(5000);
    const res = await call(refresh, makeRequest("POST", "/api/v1/license/refresh", { token }, { origin: null, headers: { "x-smidge-version": "1.1.0" } }));
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.status).toBe("ok");
    expect(body.token).not.toBe(token);
    const d = rt.db.devices.find((x) => x.device_hash === DEVICE_A)!;
    expect(d.app_version).toBe("1.1.0");
    expect(d.last_seen_at).toBe(rt.now().toISOString());
  });
  it("deactivated: after the device was removed", async () => {
    const token = await tokenFor(DEVICE_A);
    expect((await call(deactivate, makeRequest("POST", "/api/v1/license/deactivate", { token }, { origin: null }))).status).toBe(200);
    expect(rt.db.devices[0]!.deactivated_by).toBe("device");
    const res = await call(refresh, makeRequest("POST", "/api/v1/license/refresh", { token }, { origin: null }));
    expect(await res.json()).toMatchObject({ status: "deactivated" });
    // deactivating again is still ok
    expect((await call(deactivate, makeRequest("POST", "/api/v1/license/deactivate", { token }, { origin: null }))).status).toBe(200);
  });
  it("revoked / ended / unauthorized", async () => {
    const token = await tokenFor(DEVICE_A);
    await rt.db.updateEntitlement(entId, { status: "revoked", revoked_reason: "chargeback", revoked_at: "2026-10-20T10:00:00Z" });
    expect(await (await call(refresh, makeRequest("POST", "/api/v1/license/refresh", { token }, { origin: null }))).json()).toEqual({ status: "revoked", reason: "chargeback", at: "2026-10-20T10:00:00Z" });
    // ended: make it a yearly plan whose access passed
    await rt.db.updateEntitlement(entId, { status: "ended", revoked_reason: null, revoked_at: null, plan: "yearly", paddle_subscription_id: "sub_x", access_until: "2026-09-01T00:00:00Z" });
    expect(await (await call(refresh, makeRequest("POST", "/api/v1/license/refresh", { token }, { origin: null }))).json()).toMatchObject({ status: "ended", at: "2026-09-01T00:00:00Z" });
    // tampered token
    const bad = token.slice(0, -4) + (token.endsWith("AAAA") ? "BBBB" : "AAAA");
    expect((await call(refresh, makeRequest("POST", "/api/v1/license/refresh", { token: bad }, { origin: null }))).status).toBe(401);
  });
});

describe("POST /license/resend", () => {
  it("always answers ok and emails every key once per hour for a known address", async () => {
    const unknown = await call(resend, makeRequest("POST", "/api/v1/license/resend", { email: "nobody@example.com" }, { origin: null }));
    expect(await unknown.json()).toEqual({ ok: true });
    expect(rt.net.emails).toHaveLength(1); // only the purchase email
    const known = await call(resend, makeRequest("POST", "/api/v1/license/resend", { email: " Margaret@Example.com " }, { origin: null }));
    expect(await known.json()).toEqual({ ok: true });
    expect(rt.net.emails).toHaveLength(2);
    expect(rt.net.emails[1]!.text).toContain(key);
    await call(resend, makeRequest("POST", "/api/v1/license/resend", { email: "margaret@example.com" }, { origin: null }));
    expect(rt.net.emails).toHaveLength(2);
    // 3 per hour per email, then 429
    await call(resend, makeRequest("POST", "/api/v1/license/resend", { email: "margaret@example.com" }, { origin: null }));
    const limited = await call(resend, makeRequest("POST", "/api/v1/license/resend", { email: "margaret@example.com" }, { origin: null }));
    expect(limited.status).toBe(429);
  });
});
