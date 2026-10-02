/**
 * Web license client (DESIGN.md 5.5, 5.6.3, 5.6.4): talks to the site's
 * /api/v1 with the @cia/api-types schemas, keeps the SMG1 token in IndexedDB
 * and validates it offline with the public keys from
 * crates/cia-license/src/keys.rs.
 */
import { get, set, del } from "idb-keyval";
import brand from "@cia/brand";
import {
  ClaimsCreateResponseSchema,
  ClaimStatusResponseSchema,
  ErrorResponseSchema,
  LicenseActivateResponseSchema,
  LicenseRefreshResponseSchema,
  OkResponseSchema,
  hexToBytes,
  verifyToken,
  type LicensePayload,
  type LicensePublicKeys,
} from "@cia/api-types";
import type { LicenseInfo } from "@cia/engine-client";

/** Twin of `PUBLIC_KEYS` in crates/cia-license/src/keys.rs (kid, Ed25519 public key). */
export const PUBLIC_KEYS: LicensePublicKeys = [
  ["2026-10", hexToBytes("43989ddd3c48fb7399f1938617565f09f1dd629800f4e6d75dff3a87baa6a664")],
];

const API = (import.meta.env.VITE_API_BASE as string | undefined) ?? brand.urls.api;
const KEY_TOKEN = "cia.license.token";
const KEY_INSTALL = "cia.install_id";
const KEY_REFRESHED = "cia.license.refreshed_at";
const REFRESH_EVERY_MS = 24 * 60 * 60 * 1000;

export class ApiError extends Error {
  constructor(readonly code: string, message: string, readonly extra?: unknown) {
    super(message);
  }
}

function base32(bytes: Uint8Array): string {
  const alphabet = "abcdefghijklmnopqrstuvwxyz234567";
  let bits = 0, value = 0, out = "";
  for (const b of bytes) {
    value = (value << 8) | b;
    bits += 8;
    while (bits >= 5) {
      out += alphabet[(value >>> (bits - 5)) & 31];
      bits -= 5;
    }
  }
  if (bits > 0) out += alphabet[(value << (5 - bits)) & 31];
  return out;
}

/** `w_` + base32(16 random bytes), created on first load (DESIGN.md 5.4). */
export async function installId(): Promise<string> {
  let id = await get<string>(KEY_INSTALL);
  if (!id) {
    id = "w_" + base32(crypto.getRandomValues(new Uint8Array(16))).slice(0, 26);
    await set(KEY_INSTALL, id);
  }
  return id;
}

async function post<T>(path: string, body: unknown, parse: (v: unknown) => T, headers: Record<string, string> = {}): Promise<T> {
  let res: Response;
  try {
    res = await fetch(`${API}${path}`, { method: "POST", headers: { "content-type": "application/json", ...headers }, body: JSON.stringify(body) });
  } catch {
    throw new ApiError("offline", "Smidge can't reach the license server. Check your connection and try again.");
  }
  const json: unknown = await res.json().catch(() => ({}));
  if (!res.ok) {
    const err = ErrorResponseSchema.safeParse(json);
    if (err.success) throw new ApiError(err.data.error, err.data.message, json);
    throw new ApiError("http_" + res.status, `The license server answered ${res.status}.`);
  }
  return parse(json);
}

export class LicenseClient {
  private claims = new Map<string, string>();

  async token(): Promise<string | null> {
    return (await get<string>(KEY_TOKEN)) ?? null;
  }

  private async payload(token: string): Promise<LicensePayload | null> {
    const r = await verifyToken(token, PUBLIC_KEYS);
    return r.ok ? r.payload : null;
  }

  async info(): Promise<LicenseInfo> {
    const token = await this.token();
    if (!token) return { status: "free" };
    const p = await this.payload(token);
    if (!p) return { status: "invalid", message: "The saved license is damaged. Enter your key again." };
    const now = Math.floor(Date.now() / 1000);
    const base: LicenseInfo = { status: "pro", plan: p.plan, key4: p.key4, keyMasked: `…-${p.key4}`, accessUntil: p.acc ? p.acc * 1000 : null };
    if (p.acc !== null && p.acc < now) return { ...base, status: "ended", message: "Your yearly plan has ended." };
    if (p.exp < now) {
      // Offline validity ran out: try to refresh; otherwise the UI shows "needs online".
      const refreshed = await this.refresh().catch(() => null);
      if (refreshed) return this.info();
      return { ...base, status: "needs_online", message: "Connect to the internet once to keep Smidge Pro." };
    }
    void this.maybeRefresh();
    return base;
  }

  private async maybeRefresh() {
    const last = (await get<number>(KEY_REFRESHED)) ?? 0;
    if (Date.now() - last < REFRESH_EVERY_MS || !navigator.onLine) return;
    await this.refresh().catch(() => {});
  }

  /** POST /license/refresh; stores the new token or clears a revoked/ended/deactivated one. */
  async refresh(): Promise<boolean> {
    const token = await this.token();
    if (!token) return false;
    const r = await post("/license/refresh", { token }, (v) => LicenseRefreshResponseSchema.parse(v));
    await set(KEY_REFRESHED, Date.now());
    if (r.status === "ok") {
      await set(KEY_TOKEN, r.token);
      return true;
    }
    await del(KEY_TOKEN);
    return false;
  }

  async activate(productKey: string, appVersion: string): Promise<LicenseInfo> {
    const r = await post(
      "/license/activate",
      { product_key: productKey.trim(), kind: "web", device_hash: await installId(), device_name: browserName(), platform: "web", app_version: appVersion },
      (v) => LicenseActivateResponseSchema.parse(v),
    );
    if (!(await this.payload(r.token))) throw new ApiError("bad_token", "The license server sent a token this app doesn't trust.");
    await set(KEY_TOKEN, r.token);
    await set(KEY_REFRESHED, Date.now());
    const info = await this.info();
    return { ...info, devicesUsed: r.devices_used, devicesMax: r.devices_max };
  }

  async deactivate(): Promise<void> {
    const token = await this.token();
    await del(KEY_TOKEN);
    if (token) await post("/license/deactivate", { token }, (v) => OkResponseSchema.parse(v)).catch(() => {});
  }

  async startPurchase(): Promise<{ buyUrl: string; claimId: string }> {
    const r = await post("/claims", { kind: "web", device_hash: await installId(), device_name: browserName(), platform: "web" }, (v) => ClaimsCreateResponseSchema.parse(v));
    this.claims.set(r.claim_id, r.claim_secret);
    return { buyUrl: r.buy_url, claimId: r.claim_id };
  }

  async pollClaim(claimId: string): Promise<"pending" | "fulfilled" | "expired"> {
    const secret = this.claims.get(claimId);
    if (!secret) return "expired";
    const res = await fetch(`${API}/claims/${encodeURIComponent(claimId)}`, { headers: { authorization: `Claim ${secret}` } });
    if (!res.ok) return res.status === 404 ? "expired" : "pending";
    const r = ClaimStatusResponseSchema.parse(await res.json());
    if (r.status === "fulfilled") {
      await set(KEY_TOKEN, r.token);
      await set(KEY_REFRESHED, Date.now());
      this.claims.delete(claimId);
    }
    return r.status;
  }

  async resend(email: string): Promise<void> {
    await post("/license/resend", { email }, (v) => OkResponseSchema.parse(v));
  }
}

export function browserName(): string {
  const ua = navigator.userAgent;
  if (/Edg\//.test(ua)) return "Edge";
  if (/OPR\//.test(ua)) return "Opera";
  if (/Chrome\//.test(ua) && !/Chromium\//.test(ua)) return "Chrome";
  if (/Chromium\//.test(ua)) return "Chromium";
  if (/Firefox\//.test(ua)) return "Firefox";
  if (/Safari\//.test(ua) && /Version\//.test(ua)) return "Safari";
  return "Browser";
}
