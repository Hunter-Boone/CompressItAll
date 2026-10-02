import { beforeEach, describe, expect, it } from "vitest";

import { GET as account } from "../app/api/v1/account/route";
import { POST as deactivateDevice } from "../app/api/v1/account/devices/[id]/deactivate/route";
import { POST as portal } from "../app/api/v1/account/portal/route";
import { POST as revealKey } from "../app/api/v1/account/reveal-key/route";
import { POST as webSignout } from "../app/api/v1/account/web/signout/route";
import { POST as logout } from "../app/api/v1/auth/logout/route";
import { POST as otpSend } from "../app/api/v1/auth/otp/send/route";
import { POST as otpVerify } from "../app/api/v1/auth/otp/verify/route";
import { GET as housekeeping } from "../app/api/v1/cron/housekeeping/route";
import { POST as activate } from "../app/api/v1/license/activate/route";
import { OTP_MAX_ATTEMPTS } from "../lib/otp";
import { decryptDisplayKey } from "../lib/webhook";
import { DEVICE_A, DEVICE_B, WEB_1, WEB_2, call, cookieValue, deliver, event, makeRequest, makeRuntime, seedPaddle, type TestRuntime } from "./helpers";

let rt: TestRuntime;
const EMAIL = "margaret@example.com";

beforeEach(async () => {
  rt = makeRuntime();
  seedPaddle(rt.net);
  await deliver(rt, event("transaction.completed.lifetime"));
  rt.net.emails.length = 0;
});

function codeFromEmail(): string {
  const mail = rt.net.emails.at(-1)!;
  return /\b(\d{6})\b/.exec(mail.text)![1]!;
}

async function send(email = EMAIL, ip = "203.0.113.10") {
  return call(otpSend, makeRequest("POST", "/api/v1/auth/otp/send", { email }, { ip }));
}

async function signIn(): Promise<string> {
  await send();
  const code = codeFromEmail();
  const res = await call(otpVerify, makeRequest("POST", "/api/v1/auth/otp/verify", { email: EMAIL, code }));
  expect(res.status).toBe(200);
  const cookie = cookieValue(res, "smg_session");
  expect(cookie).toBeTruthy();
  expect(res.headers.get("set-cookie")).toContain("HttpOnly");
  expect(res.headers.get("set-cookie")).toContain("SameSite=Lax");
  expect(res.headers.get("set-cookie")).toContain("Max-Age=2592000");
  return `smg_session=${cookie}`;
}

describe("OTP sign-in", () => {
  it("send always answers {sent:true}; only known emails get a code", async () => {
    const unknown = await send("nobody@example.com");
    expect(await unknown.json()).toEqual({ sent: true });
    expect(rt.net.emails).toHaveLength(0);
    const known = await send();
    expect(await known.json()).toEqual({ sent: true });
    expect(rt.net.emails).toHaveLength(1);
    expect(rt.net.emails[0]!.subject).toContain("sign-in code");
    expect(rt.db.otps[0]!.attempts).toBe(0);
    expect(rt.db.otps[0]!.code_hmac).not.toContain(codeFromEmail());
  });
  it("requires an Origin header (browser POST)", async () => {
    const res = await call(otpSend, makeRequest("POST", "/api/v1/auth/otp/send", { email: EMAIL }, { origin: null }));
    expect(res.status).toBe(403);
  });
  it("60 s resend cooldown, then a new code", async () => {
    await send();
    await send();
    expect(rt.net.emails).toHaveLength(1);
    rt.advance(61_000);
    await send();
    expect(rt.net.emails).toHaveLength(2);
  });
  it("3 sends per 15 minutes per email", async () => {
    for (let i = 0; i < 3; i++) {
      expect((await send()).status).toBe(200);
      rt.advance(61_000);
    }
    expect((await send()).status).toBe(429);
  });
  it("verify consumes attempts atomically: 5 wrong then the right code is still invalid", async () => {
    await send();
    const code = codeFromEmail();
    const wrong = code === "000000" ? "111111" : "000000";
    for (let i = 0; i < OTP_MAX_ATTEMPTS; i++) {
      const res = await call(otpVerify, makeRequest("POST", "/api/v1/auth/otp/verify", { email: EMAIL, code: wrong }));
      expect(res.status).toBe(400);
      expect((await res.json()).error).toBe("invalid_code");
    }
    expect(rt.db.otps[0]!.attempts).toBe(OTP_MAX_ATTEMPTS);
    const res = await call(otpVerify, makeRequest("POST", "/api/v1/auth/otp/verify", { email: EMAIL, code }));
    expect(res.status).toBe(400);
  });
  it("a code works once and expires after 10 minutes", async () => {
    await send();
    const code = codeFromEmail();
    rt.advance(10 * 60 * 1000 + 1);
    expect((await call(otpVerify, makeRequest("POST", "/api/v1/auth/otp/verify", { email: EMAIL, code }))).status).toBe(400);
    rt.advance(61_000);
    await send();
    const code2 = codeFromEmail();
    expect((await call(otpVerify, makeRequest("POST", "/api/v1/auth/otp/verify", { email: EMAIL, code: code2 }))).status).toBe(200);
    expect((await call(otpVerify, makeRequest("POST", "/api/v1/auth/otp/verify", { email: EMAIL, code: code2 }))).status).toBe(400);
  });
  it("verify is rate limited 30 per hour per IP", async () => {
    await send();
    for (let i = 0; i < 30; i++) await call(otpVerify, makeRequest("POST", "/api/v1/auth/otp/verify", { email: EMAIL, code: "123456" }));
    const res = await call(otpVerify, makeRequest("POST", "/api/v1/auth/otp/verify", { email: EMAIL, code: "123456" }));
    expect(res.status).toBe(429);
  });
});

describe("dashboard", () => {
  it("GET /account needs a session and lists entitlements, devices and masked keys", async () => {
    expect((await call(account, makeRequest("GET", "/api/v1/account"))).status).toBe(401);
    const cookie = await signIn();
    const ent = rt.db.entitlements[0]!;
    const key = decryptDisplayKey(rt, ent);
    await call(activate, makeRequest("POST", "/api/v1/license/activate", { product_key: key, kind: "desktop", device_hash: DEVICE_A, device_name: "Margaret's PC", platform: "windows", app_version: "1.0.0" }, { origin: null }));
    await call(activate, makeRequest("POST", "/api/v1/license/activate", { product_key: key, kind: "web", device_hash: WEB_1, platform: "web", app_version: "1.0.0" }, { origin: null }));
    const res = await call(account, makeRequest("GET", "/api/v1/account", undefined, { cookie }));
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.email).toBe(EMAIL);
    expect(body.entitlements).toHaveLength(1);
    const e = body.entitlements[0];
    expect(e).toMatchObject({ id: ent.id, plan: "lifetime", status: "active", key_masked: `XXXXX-XXXXX-XXXXX-${ent.key4}`, key4: ent.key4, devices_max: 3, web_count: 1, access_until: null, cancel_at: null });
    expect(e.devices).toHaveLength(1);
    expect(e.devices[0]).toMatchObject({ name: "Margaret's PC", platform: "windows", kind: "desktop" });
    expect(JSON.stringify(body)).not.toContain(key);
  });
  it("reveal-key returns the key and audits it; other people's entitlements are 404", async () => {
    const cookie = await signIn();
    const ent = rt.db.entitlements[0]!;
    const res = await call(revealKey, makeRequest("POST", "/api/v1/account/reveal-key", { entitlement_id: ent.id }, { cookie }));
    expect(res.status).toBe(200);
    expect((await res.json()).product_key).toBe(decryptDisplayKey(rt, ent));
    expect(rt.db.auditLog.some((a) => a.action === "key.revealed")).toBe(true);
    const other = await call(revealKey, makeRequest("POST", "/api/v1/account/reveal-key", { entitlement_id: "ent_01ARZ3NDEKTSV4RRFFQ69G5FAV" }, { cookie }));
    expect(other.status).toBe(404);
  });
  it("Remove computer frees the slot, with a cap of 10 per 30 days", async () => {
    const cookie = await signIn();
    const ent = rt.db.entitlements[0]!;
    const key = decryptDisplayKey(rt, ent);
    const act = (d: string) => call(activate, makeRequest("POST", "/api/v1/license/activate", { product_key: key, kind: "desktop", device_hash: d, device_name: "PC", platform: "linux", app_version: "1.0.0" }, { origin: null }));
    for (let i = 0; i < 10; i++) {
      rt.advance(3600 * 1000); // stay under the 10/hour activation limit per IP
      await act(DEVICE_A);
      const id = rt.db.devices.find((d) => d.device_hash === DEVICE_A && d.deactivated_at === null)!.id;
      const res = await call(deactivateDevice, makeRequest("POST", `/api/v1/account/devices/${id}/deactivate`, undefined, { cookie }), { id });
      expect(res.status).toBe(200);
    }
    rt.advance(3600 * 1000);
    await act(DEVICE_B);
    const id = rt.db.devices.find((d) => d.device_hash === DEVICE_B && d.deactivated_at === null)!.id;
    const res = await call(deactivateDevice, makeRequest("POST", `/api/v1/account/devices/${id}/deactivate`, undefined, { cookie }), { id });
    expect(res.status).toBe(429);
    expect((await res.json()).message).toContain("Contact support");
    // unknown id
    expect((await call(deactivateDevice, makeRequest("POST", "/api/v1/account/devices/x/deactivate", undefined, { cookie }), { id: "not-a-uuid" })).status).toBe(404);
  });
  it("Sign out all browsers deactivates every web row", async () => {
    const cookie = await signIn();
    const ent = rt.db.entitlements[0]!;
    const key = decryptDisplayKey(rt, ent);
    for (const w of [WEB_1, WEB_2]) await call(activate, makeRequest("POST", "/api/v1/license/activate", { product_key: key, kind: "web", device_hash: w, platform: "web", app_version: "1.0.0" }, { origin: null }));
    const res = await call(webSignout, makeRequest("POST", "/api/v1/account/web/signout", { entitlement_id: ent.id }, { cookie }));
    expect(res.status).toBe(200);
    expect(rt.db.devices.filter((d) => d.kind === "web" && d.deactivated_at === null)).toHaveLength(0);
    expect(rt.db.devices.every((d) => d.deactivated_by === "dashboard")).toBe(true);
  });
  it("Manage billing creates a Paddle portal session", async () => {
    const cookie = await signIn();
    const ent = rt.db.entitlements[0]!;
    const res = await call(portal, makeRequest("POST", "/api/v1/account/portal", { entitlement_id: ent.id }, { cookie }));
    expect(res.status).toBe(200);
    expect((await res.json()).url).toContain("sandbox-customer-portal.paddle.com");
    expect(rt.net.portalSessions[0]!.customerId).toBe("ctm_01test_margaret");
  });
  it("Sign out revokes this session; Sign out everywhere revokes all", async () => {
    const c1 = await signIn();
    rt.advance(61_000);
    const c2 = await signIn();
    expect((await call(logout, makeRequest("POST", "/api/v1/auth/logout", {}, { cookie: c1 }))).status).toBe(200);
    expect((await call(account, makeRequest("GET", "/api/v1/account", undefined, { cookie: c1 }))).status).toBe(401);
    expect((await call(account, makeRequest("GET", "/api/v1/account", undefined, { cookie: c2 }))).status).toBe(200);
    rt.advance(61_000);
    const c3 = await signIn();
    expect((await call(logout, makeRequest("POST", "/api/v1/auth/logout", { everywhere: true }, { cookie: c3 }))).status).toBe(200);
    expect((await call(account, makeRequest("GET", "/api/v1/account", undefined, { cookie: c2 }))).status).toBe(401);
    expect((await call(account, makeRequest("GET", "/api/v1/account", undefined, { cookie: c3 }))).status).toBe(401);
  });
  it("sessions slide for 30 days and then expire", async () => {
    const cookie = await signIn();
    rt.advance(29 * 86400 * 1000);
    expect((await call(account, makeRequest("GET", "/api/v1/account", undefined, { cookie }))).status).toBe(200);
    rt.advance(29 * 86400 * 1000);
    expect((await call(account, makeRequest("GET", "/api/v1/account", undefined, { cookie }))).status).toBe(200);
    rt.advance(31 * 86400 * 1000);
    expect((await call(account, makeRequest("GET", "/api/v1/account", undefined, { cookie }))).status).toBe(401);
  });
});

describe("housekeeping cron", () => {
  it("expires claims, deletes used OTP rows and old rate-limit windows, idles old desktops", async () => {
    await signIn();
    const ent = rt.db.entitlements[0]!;
    const key = decryptDisplayKey(rt, ent);
    await call(activate, makeRequest("POST", "/api/v1/license/activate", { product_key: key, kind: "desktop", device_hash: DEVICE_A, device_name: "Old PC", platform: "macos", app_version: "1.0.0" }, { origin: null }));
    await rt.db.insertClaim({ id: "clm_01ARZ3NDEKTSV4RRFFQ69G5FAV", secret_hash: "00", kind: "desktop", device_hash: DEVICE_B, device_name: null, platform: "windows", entitlement_id: null, status: "pending", expires_at: rt.now().toISOString() });
    rt.advance(181 * 86400 * 1000);
    const res = await call(housekeeping, makeRequest("GET", "/api/v1/cron/housekeeping", undefined, { origin: null, headers: { authorization: `Bearer ${rt.env.CRON_SECRET}` } }));
    expect(res.status).toBe(200);
    const { counts } = await res.json();
    expect(counts).toMatchObject({ claims_expired: 1, otp_deleted: 1, devices_idled: 1 });
    expect(counts.rate_limit_windows_deleted).toBeGreaterThan(0);
    expect(rt.db.devices[0]!.deactivated_by).toBe("idle");
    expect(rt.db.claims[0]!.status).toBe("expired");
  });
});
