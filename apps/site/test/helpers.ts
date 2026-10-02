// Test harness: MemoryDb runtime, a fetch stub for Paddle, Resend and GitHub, request builders,
// and in-test webhook signing. No network anywhere.

import { readFileSync } from "node:fs";

import { MemoryDb } from "../lib/db-memory";
import { parseEnv, resetEnvCache, type Env } from "../lib/env";
import { signForTest } from "../lib/paddle-signature";
import { setRuntime, type Runtime } from "../lib/runtime";
import { POST as webhookPost } from "../app/api/v1/webhooks/paddle/route";

const testKeys: { keys: { kid: string; seed_b64: string }[] } = JSON.parse(
  readFileSync(new URL("../../../crates/cia-license/test-keys.json", import.meta.url), "utf8"),
);
export const TEST_SEED_B64 = testKeys.keys.find((k) => k.kid === "test")!.seed_b64;

export const SITE_URL = "http://localhost:3031";
export const APP_URL = "http://localhost:5181";
export const PRICE_LIFETIME = "pri_01test_lifetime";
export const PRICE_YEARLY = "pri_01test_yearly";
export const WEBHOOK_SECRET = "pdl_ntfset_01testsecret_abcdefghijklmnop";

export function testEnvRecord(overrides: Record<string, string | undefined> = {}): Record<string, string | undefined> {
  return {
    NEXT_PUBLIC_SITE_URL: SITE_URL,
    NEXT_PUBLIC_APP_URL: APP_URL,
    SMG_DB: "memory",
    PADDLE_ENV: "sandbox",
    PADDLE_API_KEY: "pdl_sdbx_apikey_01testkey_abcdefghijklmnop",
    PADDLE_WEBHOOK_SECRET: WEBHOOK_SECRET,
    PADDLE_PRICE_ID_LIFETIME: PRICE_LIFETIME,
    PADDLE_PRICE_ID_YEARLY: PRICE_YEARLY,
    NEXT_PUBLIC_PADDLE_ENV: "sandbox",
    NEXT_PUBLIC_PADDLE_CLIENT_TOKEN: "test_01clienttoken",
    RESEND_API_KEY: "re_01testresendkey",
    EMAIL_FROM: "Smidge <hello@mail.smidge.test>",
    EMAIL_REPLY_TO: "support@smidge.test",
    LICENSE_SIGNING_KEY: TEST_SEED_B64,
    LICENSE_SIGNING_KID: "test",
    KEY_ENCRYPTION_KEY: Buffer.alloc(32, 7).toString("base64"),
    OTP_PEPPER: Buffer.alloc(32, 9).toString("base64"),
    IP_HASH_SALT: Buffer.alloc(16, 3).toString("base64"),
    CRON_SECRET: "cron-01-secret-value-for-tests",
    ADMIN_ALERT_EMAIL: "alerts@smidge.test",
    ...overrides,
  };
}

export function testEnv(overrides: Record<string, string | undefined> = {}): Env {
  resetEnvCache();
  return parseEnv(testEnvRecord(overrides));
}

export interface SentEmail {
  to: string;
  subject: string;
  html: string;
  text: string;
}

/** Fake Paddle + Resend + GitHub reachable only through `fetch`. */
export class FakeNet {
  customers = new Map<string, { id: string; email: string }>();
  subscriptions = new Map<string, Record<string, unknown>>();
  transactions = new Map<string, Record<string, unknown>>();
  channels = new Map<string, unknown>();
  emails: SentEmail[] = [];
  createdTransactions: Record<string, unknown>[] = [];
  portalSessions: { customerId: string; body: unknown }[] = [];
  calls: { method: string; url: string }[] = [];
  private txnCounter = 0;
  /** Set to make every Paddle call fail. */
  paddleDown = false;
  resendDown = false;

  readonly fetch: typeof fetch = async (input, init) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
    const method = (init?.method ?? "GET").toUpperCase();
    this.calls.push({ method, url });
    const u = new URL(url);
    const body = init?.body ? JSON.parse(String(init.body)) : null;

    if (u.hostname === "api.resend.com") {
      if (this.resendDown) return Response.json({ message: "down" }, { status: 500 });
      this.emails.push({ to: body.to[0], subject: body.subject, html: body.html, text: body.text });
      return Response.json({ id: `email_${this.emails.length}` });
    }
    if (u.hostname === "raw.githubusercontent.com") {
      const m = /\/channels\/([a-z]+)\.json$/.exec(u.pathname);
      const file = m ? this.channels.get(m[1]!) : undefined;
      if (file === undefined) return new Response("not found", { status: 404 });
      return Response.json(file);
    }
    if (u.hostname === "sandbox-api.paddle.com") {
      if (this.paddleDown) return Response.json({ error: { code: "down", detail: "simulated outage" } }, { status: 503 });
      const auth = init?.headers && (init.headers as Record<string, string>).Authorization;
      if (!auth?.startsWith("Bearer pdl_sdbx_")) return Response.json({ error: { code: "forbidden" } }, { status: 403 });
      const parts = u.pathname.split("/").filter(Boolean);
      if (parts[0] === "customers" && parts.length === 2 && method === "GET") {
        const c = this.customers.get(parts[1]!);
        return c ? Response.json({ data: c }) : Response.json({ error: { code: "not_found" } }, { status: 404 });
      }
      if (parts[0] === "customers" && parts[2] === "portal-sessions" && method === "POST") {
        this.portalSessions.push({ customerId: parts[1]!, body });
        return Response.json({ data: { urls: { general: { overview: `https://sandbox-customer-portal.paddle.com/cpl_${parts[1]}` } } } });
      }
      if (parts[0] === "subscriptions" && parts.length === 2 && method === "GET") {
        const s = this.subscriptions.get(parts[1]!);
        return s ? Response.json({ data: s }) : Response.json({ error: { code: "not_found" } }, { status: 404 });
      }
      if (parts[0] === "transactions" && parts.length === 2 && method === "GET") {
        const t = this.transactions.get(parts[1]!);
        return t ? Response.json({ data: t }) : Response.json({ error: { code: "not_found" } }, { status: 404 });
      }
      if (parts[0] === "transactions" && parts.length === 1 && method === "POST") {
        this.createdTransactions.push(body);
        const id = `txn_01test_created_${++this.txnCounter}`;
        return Response.json({ data: { id, status: "ready", custom_data: body.custom_data } });
      }
      return Response.json({ error: { code: "not_found" } }, { status: 404 });
    }
    throw new Error(`unexpected fetch to ${url}`);
  };
}

export interface TestRuntime extends Runtime {
  db: MemoryDb;
  net: FakeNet;
  /** Move the clock. */
  advance(ms: number): void;
  setNow(d: Date): void;
}

export function makeRuntime(options: { now?: Date; env?: Record<string, string | undefined> } = {}): TestRuntime {
  let now = options.now ?? new Date("2026-10-02T12:00:00.000Z");
  const db = new MemoryDb(() => new Date(now));
  const net = new FakeNet();
  const rt: TestRuntime = {
    env: testEnv(options.env),
    db,
    net,
    fetch: net.fetch,
    now: () => new Date(now),
    advance: (ms) => {
      now = new Date(now.getTime() + ms);
    },
    setNow: (d) => {
      now = d;
    },
  };
  setRuntime(rt);
  return rt;
}

export interface ReqOptions {
  origin?: string | null;
  ip?: string;
  cookie?: string;
  headers?: Record<string, string>;
}

export function makeRequest(method: string, path: string, body?: unknown, o: ReqOptions = {}): Request {
  const headers = new Headers(o.headers ?? {});
  if (body !== undefined) headers.set("content-type", "application/json");
  if (o.origin !== null) headers.set("origin", o.origin ?? SITE_URL);
  headers.set("x-forwarded-for", o.ip ?? "203.0.113.10");
  if (o.cookie) headers.set("cookie", o.cookie);
  return new Request(`${SITE_URL}${path}`, { method, headers, body: body === undefined ? undefined : typeof body === "string" ? body : JSON.stringify(body) });
}

type Handler<P> = (req: Request, ctx?: { params: Promise<P> }) => Promise<Response>;

export async function call<P extends Record<string, string>>(handler: Handler<P>, req: Request, params?: P): Promise<Response> {
  return handler(req, params ? { params: Promise.resolve(params) } : undefined);
}

export function cookieValue(res: Response, name: string): string | null {
  const header = res.headers.get("set-cookie");
  if (!header) return null;
  const m = new RegExp(`(?:^|,\\s*)${name}=([^;]*)`).exec(header);
  return m ? decodeURIComponent(m[1]!) : null;
}

// ---- Paddle fixtures and signed delivery -------------------------------------

export function fixture(name: string): Record<string, unknown> {
  return JSON.parse(readFileSync(new URL(`./fixtures/paddle/${name}.json`, import.meta.url), "utf8"));
}

let eventCounter = 0;

/** Clone a fixture with a fresh event id (and optional overrides applied to `data`). */
export function event(name: string, overrides: { eventId?: string; occurredAt?: string; data?: Record<string, unknown> } = {}): Record<string, unknown> {
  const f = fixture(name);
  const id = overrides.eventId ?? `evt_01test_${String(++eventCounter).padStart(4, "0")}`;
  return { ...f, event_id: id, occurred_at: overrides.occurredAt ?? f.occurred_at, data: { ...(f.data as Record<string, unknown>), ...(overrides.data ?? {}) } };
}

export async function deliver(rt: TestRuntime, payload: Record<string, unknown>, o: { ts?: number; secret?: string; header?: string | null } = {}): Promise<Response> {
  const raw = JSON.stringify(payload);
  const ts = o.ts ?? Math.floor(rt.now().getTime() / 1000);
  const header = o.header === undefined ? signForTest(o.secret ?? WEBHOOK_SECRET, raw, ts) : o.header;
  const headers = new Headers({ "content-type": "application/json" });
  if (header !== null) headers.set("paddle-signature", header);
  return webhookPost(new Request(`${SITE_URL}/api/v1/webhooks/paddle`, { method: "POST", headers, body: raw }));
}

/** Standard sandbox-shaped objects the fixtures refer to. */
export function seedPaddle(net: FakeNet): void {
  net.customers.set("ctm_01test_margaret", { id: "ctm_01test_margaret", email: "Margaret@Example.com" });
  net.customers.set("ctm_01test_jayden", { id: "ctm_01test_jayden", email: "jayden@example.com" });
  net.subscriptions.set("sub_01test_yearly", activeSubscription("2026-10-02T11:58:00Z", "2027-10-02T11:58:00Z"));
  net.transactions.set("txn_01test_renewal", { id: "txn_01test_renewal", subscription_id: "sub_01test_yearly", origin: "subscription_recurring", customer_id: "ctm_01test_jayden", items: [{ price: { id: PRICE_YEARLY }, quantity: 1 }] });
}

export function activeSubscription(startsAt: string, endsAt: string, extra: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    id: "sub_01test_yearly",
    status: "active",
    customer_id: "ctm_01test_jayden",
    current_billing_period: { starts_at: startsAt, ends_at: endsAt },
    scheduled_change: null,
    canceled_at: null,
    paused_at: null,
    items: [{ price: { id: PRICE_YEARLY } }],
    ...extra,
  };
}

export const DEVICE_A = "d_abcdefghijklmnopqrstuvwxyz";
export const DEVICE_B = "d_bbcdefghijklmnopqrstuvwxyz";
export const DEVICE_C = "d_cbcdefghijklmnopqrstuvwxyz";
export const DEVICE_D = "d_dbcdefghijklmnopqrstuvwxyz";
export const WEB_1 = "w_abcdefghijklmnopqrstuvwxyz";
export const WEB_2 = "w_bbcdefghijklmnopqrstuvwxyz";
export const WEB_3 = "w_cbcdefghijklmnopqrstuvwxyz";
export const WEB_4 = "w_dbcdefghijklmnopqrstuvwxyz";
