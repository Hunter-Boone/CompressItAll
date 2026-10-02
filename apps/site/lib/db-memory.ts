// In-memory Db for tests and `SMG_DB=memory` local development. Same semantics as the SQL
// functions in supabase/migrations/0002_functions.sql. `clock` lets tests control "now".

import { randomUUID } from "node:crypto";

import type {
  ActivateResult,
  AuditEntry,
  Checkout,
  Claim,
  Customer,
  Db,
  DeactivatedBy,
  Device,
  Entitlement,
  OtpRow,
  PaddleEvent,
  Patch,
  Session,
} from "./db";
import { UniqueViolation } from "./db";

export class MemoryDb implements Db {
  customers: Customer[] = [];
  entitlements: Entitlement[] = [];
  devices: Device[] = [];
  claims: Claim[] = [];
  checkouts: Checkout[] = [];
  events: PaddleEvent[] = [];
  appliedAdjustments: { adjustment_id: string; entitlement_id: string; action: string; applied_at: string }[] = [];
  otps: OtpRow[] = [];
  sessions: Session[] = [];
  emailLog: { id: string; kind: string; ref: string; to_email: string; resend_id: string | null; sent_at: string }[] = [];
  auditLog: (AuditEntry & { at: string })[] = [];
  rateLimits = new Map<string, { window_start: number; count: number }[]>();
  /** Set to true to make rateLimitHit throw, for fail-closed tests. */
  rateLimitBroken = false;

  constructor(public clock: () => Date = () => new Date()) {}

  private now(): string {
    return this.clock().toISOString();
  }

  // ---- customers ------------------------------------------------------------
  async findCustomerById(id: string) {
    return this.customers.find((c) => c.id === id) ?? null;
  }
  async findCustomerByPaddleId(paddleCustomerId: string) {
    return this.customers.find((c) => c.paddle_customer_id === paddleCustomerId) ?? null;
  }
  async findCustomersByEmail(email: string) {
    return this.customers.filter((c) => c.email === email);
  }
  async upsertCustomer(paddleCustomerId: string, email: string) {
    const existing = await this.findCustomerByPaddleId(paddleCustomerId);
    if (existing) {
      existing.email = email;
      existing.updated_at = this.now();
      return existing;
    }
    const c: Customer = { id: randomUUID(), paddle_customer_id: paddleCustomerId, email, created_at: this.now(), updated_at: this.now() };
    this.customers.push(c);
    return c;
  }
  async updateCustomerEmail(paddleCustomerId: string, email: string) {
    const c = await this.findCustomerByPaddleId(paddleCustomerId);
    if (c) {
      c.email = email;
      c.updated_at = this.now();
    }
  }

  // ---- entitlements ---------------------------------------------------------
  async insertEntitlement(ent: Omit<Entitlement, "created_at" | "updated_at">) {
    if (this.entitlements.some((e) => e.id === ent.id)) throw new UniqueViolation("entitlements_pkey");
    if (this.entitlements.some((e) => e.product_key_hash === ent.product_key_hash)) throw new UniqueViolation("entitlements_product_key_hash_key");
    if (ent.paddle_transaction_id && this.entitlements.some((e) => e.paddle_transaction_id === ent.paddle_transaction_id)) {
      throw new UniqueViolation("entitlements_paddle_transaction_id_key");
    }
    if (ent.paddle_subscription_id && this.entitlements.some((e) => e.paddle_subscription_id === ent.paddle_subscription_id)) {
      throw new UniqueViolation("entitlements_paddle_subscription_id_key");
    }
    if ((ent.plan === "yearly") !== (ent.paddle_subscription_id !== null)) throw new Error("check constraint yearly_has_sub");
    const row: Entitlement = { ...ent, created_at: this.now(), updated_at: this.now() };
    this.entitlements.push(row);
    return row;
  }
  async findEntitlementById(id: string) {
    return this.entitlements.find((e) => e.id === id) ?? null;
  }
  async findEntitlementByKeyHash(hash: string) {
    return this.entitlements.find((e) => e.product_key_hash === hash) ?? null;
  }
  async findEntitlementByTransactionId(txnId: string) {
    return this.entitlements.find((e) => e.paddle_transaction_id === txnId) ?? null;
  }
  async findEntitlementBySubscriptionId(subId: string) {
    return this.entitlements.find((e) => e.paddle_subscription_id === subId) ?? null;
  }
  async listEntitlementsForCustomers(customerIds: string[]) {
    return this.entitlements.filter((e) => customerIds.includes(e.customer_id)).sort((a, b) => a.created_at.localeCompare(b.created_at));
  }
  async updateEntitlement(id: string, patch: Patch<Entitlement>) {
    const e = await this.findEntitlementById(id);
    if (!e) throw new Error(`entitlement ${id} not found`);
    Object.assign(e, patch, { updated_at: this.now() });
    return e;
  }

  // ---- devices --------------------------------------------------------------
  private activeDevices(entitlementId: string, kind?: string) {
    return this.devices.filter((d) => d.entitlement_id === entitlementId && d.deactivated_at === null && (kind === undefined || d.kind === kind));
  }
  async activateDevice(args: { entitlementId: string; kind: "desktop" | "web"; deviceHash: string; deviceName: string | null; platform: Device["platform"]; appVersion: string | null }): Promise<ActivateResult> {
    // The SQL function holds pg_advisory_xact_lock(hashtextextended(p_ent, 0)); JS is single
    // threaded between awaits, so the whole method is one critical section here.
    const ent = await this.findEntitlementById(args.entitlementId);
    if (!ent) throw new Error("entitlement not found");
    const max = args.kind === "web" ? ent.max_web : ent.max_devices;
    const existing = this.devices.find((d) => d.entitlement_id === args.entitlementId && d.device_hash === args.deviceHash && d.deactivated_at === null);
    if (existing) {
      existing.last_seen_at = this.now();
      existing.app_version = args.appVersion;
      if (args.deviceName) existing.device_name = args.deviceName;
      return { ok: true, existing: true, used: this.activeDevices(args.entitlementId, args.kind).length, max, device: existing };
    }
    const active = this.activeDevices(args.entitlementId, args.kind);
    if (active.length >= max) {
      if (args.kind === "web") {
        const lru = [...active].sort((a, b) => a.last_seen_at.localeCompare(b.last_seen_at))[0]!;
        lru.deactivated_at = this.now();
        lru.deactivated_by = "evicted";
      } else {
        return {
          ok: false,
          reason: "limit",
          devices: active
            .sort((a, b) => b.last_seen_at.localeCompare(a.last_seen_at))
            .map((d) => ({ name: d.device_name, platform: d.platform, last_seen_at: d.last_seen_at })),
        };
      }
    }
    const device: Device = {
      id: randomUUID(),
      entitlement_id: args.entitlementId,
      kind: args.kind,
      device_hash: args.deviceHash,
      device_name: args.deviceName,
      platform: args.platform,
      app_version: args.appVersion,
      activated_at: this.now(),
      last_seen_at: this.now(),
      deactivated_at: null,
      deactivated_by: null,
    };
    this.devices.push(device);
    return { ok: true, existing: false, used: this.activeDevices(args.entitlementId, args.kind).length, max, device };
  }
  async findActiveDevice(entitlementId: string, deviceHash: string) {
    return this.devices.find((d) => d.entitlement_id === entitlementId && d.device_hash === deviceHash && d.deactivated_at === null) ?? null;
  }
  async findDeviceById(id: string) {
    return this.devices.find((d) => d.id === id) ?? null;
  }
  async listActiveDevices(entitlementId: string) {
    return this.activeDevices(entitlementId).sort((a, b) => a.activated_at.localeCompare(b.activated_at));
  }
  async touchDevice(id: string, appVersion: string | null) {
    const d = await this.findDeviceById(id);
    if (d) {
      d.last_seen_at = this.now();
      if (appVersion !== null) d.app_version = appVersion;
    }
  }
  async deactivateDevice(id: string, by: DeactivatedBy) {
    const d = await this.findDeviceById(id);
    if (d && d.deactivated_at === null) {
      d.deactivated_at = this.now();
      d.deactivated_by = by;
    }
  }
  async deactivateWebDevices(entitlementId: string, by: DeactivatedBy) {
    const rows = this.activeDevices(entitlementId, "web");
    for (const d of rows) {
      d.deactivated_at = this.now();
      d.deactivated_by = by;
    }
    return rows.length;
  }
  async countDashboardDeactivationsSince(entitlementId: string, since: string) {
    return this.devices.filter((d) => d.entitlement_id === entitlementId && d.deactivated_by === "dashboard" && d.deactivated_at !== null && d.deactivated_at >= since).length;
  }
  async markIdleDesktopDevices(unseenSince: string) {
    const rows = this.devices.filter((d) => d.kind === "desktop" && d.deactivated_at === null && d.last_seen_at < unseenSince);
    for (const d of rows) {
      d.deactivated_at = this.now();
      d.deactivated_by = "idle";
    }
    return rows.length;
  }

  // ---- claims ---------------------------------------------------------------
  async insertClaim(claim: Omit<Claim, "created_at">) {
    const row: Claim = { ...claim, created_at: this.now() };
    this.claims.push(row);
    return row;
  }
  async findClaim(id: string) {
    return this.claims.find((c) => c.id === id) ?? null;
  }
  async updateClaim(id: string, patch: Patch<Claim>) {
    const c = await this.findClaim(id);
    if (c) Object.assign(c, patch);
  }
  async expirePendingClaims(before: string) {
    let n = 0;
    for (const c of this.claims) {
      if (c.status === "pending" && c.expires_at < before) {
        c.status = "expired";
        n++;
      }
    }
    return n;
  }

  // ---- checkouts ------------------------------------------------------------
  async insertCheckout(checkout: Omit<Checkout, "created_at">) {
    const row: Checkout = { ...checkout, created_at: this.now() };
    this.checkouts.push(row);
    return row;
  }
  async findCheckout(id: string) {
    return this.checkouts.find((c) => c.id === id) ?? null;
  }
  async updateCheckout(id: string, patch: Patch<Checkout>) {
    const c = await this.findCheckout(id);
    if (c) Object.assign(c, patch);
  }

  // ---- paddle events --------------------------------------------------------
  async insertEventIfAbsent(ev: Omit<PaddleEvent, "received_at" | "status" | "attempts" | "last_error" | "processed_at">) {
    const existing = await this.findEvent(ev.event_id);
    if (existing) return { inserted: false, row: existing };
    const row: PaddleEvent = { ...ev, received_at: this.now(), status: "received", attempts: 0, last_error: null, processed_at: null };
    this.events.push(row);
    return { inserted: true, row };
  }
  async findEvent(eventId: string) {
    return this.events.find((e) => e.event_id === eventId) ?? null;
  }
  async updateEvent(eventId: string, patch: Partial<Omit<PaddleEvent, "event_id">>) {
    const e = await this.findEvent(eventId);
    if (e) Object.assign(e, patch);
  }
  async listEventsToReprocess(receivedBefore: string, maxAttempts: number, limit: number) {
    return this.events
      .filter((e) => (e.status === "error" && e.attempts < maxAttempts) || (e.status === "received" && e.received_at < receivedBefore))
      .sort((a, b) => a.occurred_at.localeCompare(b.occurred_at))
      .slice(0, limit);
  }
  async listDeadLetterEvents(maxAttempts: number, limit: number) {
    return this.events.filter((e) => e.status === "error" && e.attempts >= maxAttempts).slice(0, limit);
  }

  // ---- adjustments ----------------------------------------------------------
  async insertAppliedAdjustment(adjustmentId: string, entitlementId: string, action: string) {
    if (this.appliedAdjustments.some((a) => a.adjustment_id === adjustmentId)) return false;
    this.appliedAdjustments.push({ adjustment_id: adjustmentId, entitlement_id: entitlementId, action, applied_at: this.now() });
    return true;
  }

  // ---- otp ------------------------------------------------------------------
  async getOtp(email: string) {
    return this.otps.find((o) => o.email === email) ?? null;
  }
  async upsertOtp(row: OtpRow) {
    const i = this.otps.findIndex((o) => o.email === row.email);
    if (i >= 0) this.otps[i] = { ...row };
    else this.otps.push({ ...row });
  }
  async consumeOtpAttempt(email: string, max: number) {
    // UPDATE ... SET attempts = attempts + 1 WHERE email = $1 AND used_at IS NULL AND attempts < $2 RETURNING ...
    const o = await this.getOtp(email);
    if (!o || o.used_at !== null || o.attempts >= max) return null;
    o.attempts += 1;
    return { code_hmac: o.code_hmac, expires_at: o.expires_at };
  }
  async markOtpUsed(email: string, at: string) {
    const o = await this.getOtp(email);
    if (o) o.used_at = at;
  }
  async deleteOtpUsedOrExpired(now: string) {
    const before = this.otps.length;
    this.otps = this.otps.filter((o) => o.used_at === null && o.expires_at > now);
    return before - this.otps.length;
  }

  // ---- sessions -------------------------------------------------------------
  async insertSession(s: Session) {
    this.sessions.push({ ...s });
  }
  async findSession(idHash: string) {
    return this.sessions.find((s) => s.id_hash === idHash) ?? null;
  }
  async touchSession(idHash: string, lastUsedAt: string, expiresAt: string) {
    const s = await this.findSession(idHash);
    if (s) {
      s.last_used_at = lastUsedAt;
      s.expires_at = expiresAt;
    }
  }
  async revokeSession(idHash: string, at: string) {
    const s = await this.findSession(idHash);
    if (s && s.revoked_at === null) s.revoked_at = at;
  }
  async revokeSessionsForEmail(email: string, at: string) {
    for (const s of this.sessions) if (s.email === email && s.revoked_at === null) s.revoked_at = at;
  }

  // ---- email log ------------------------------------------------------------
  async insertEmailLogIfAbsent(kind: string, ref: string, to: string) {
    const existing = this.emailLog.find((e) => e.kind === kind && e.ref === ref);
    if (existing) return { inserted: false, id: existing.id };
    const id = randomUUID();
    this.emailLog.push({ id, kind, ref, to_email: to, resend_id: null, sent_at: this.now() });
    return { inserted: true, id };
  }
  async setEmailLogResendId(id: string, resendId: string) {
    const e = this.emailLog.find((r) => r.id === id);
    if (e) e.resend_id = resendId;
  }

  // ---- audit ----------------------------------------------------------------
  async audit(entry: AuditEntry) {
    this.auditLog.push({ ...entry, at: this.now() });
  }

  // ---- rate limits ----------------------------------------------------------
  async rateLimitHit(key: string, windowSeconds: number, max: number) {
    if (this.rateLimitBroken) throw new Error("rate_limit_hit failed (simulated)");
    const nowS = Math.floor(this.clock().getTime() / 1000);
    const bucket = Math.floor(nowS / windowSeconds) * windowSeconds;
    let rows = this.rateLimits.get(key);
    if (!rows) {
      rows = [];
      this.rateLimits.set(key, rows);
    }
    let row = rows.find((r) => r.window_start === bucket);
    if (!row) {
      row = { window_start: bucket, count: 0 };
      rows.push(row);
    }
    row.count += 1;
    // DELETE old windows for this key.
    this.rateLimits.set(key, rows.filter((r) => r.window_start >= bucket - windowSeconds));
    return row.count <= max;
  }
  async deleteRateLimitsBefore(before: string) {
    const cutoff = Math.floor(new Date(before).getTime() / 1000);
    let n = 0;
    for (const [key, rows] of this.rateLimits) {
      const kept = rows.filter((r) => r.window_start >= cutoff);
      n += rows.length - kept.length;
      if (kept.length === 0) this.rateLimits.delete(key);
      else this.rateLimits.set(key, kept);
    }
    return n;
  }
}
