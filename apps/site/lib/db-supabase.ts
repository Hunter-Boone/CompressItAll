// Supabase implementation of Db. Uses the service role key (server only; RLS has no policies so
// nothing else can read). Device activation, OTP attempts and rate limits go through the SQL
// functions in supabase/migrations/0002_functions.sql; there is no non-atomic fallback.

import { createClient, type SupabaseClient } from "@supabase/supabase-js";

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
import { DbUnavailable, UniqueViolation } from "./db";

type Row = Record<string, unknown>;

/** PostgREST represents bytea as `\x<hex>`; the app works in plain hex. */
const toBytea = (hex: string) => `\\x${hex}`;
const fromBytea = (v: unknown) => (typeof v === "string" && v.startsWith("\\x") ? v.slice(2) : String(v));

interface PgError {
  code?: string;
  message: string;
  details?: string;
}

function fail(op: string, error: PgError): never {
  if (error.code === "23505") throw new UniqueViolation(error.details ?? error.message);
  throw new DbUnavailable(`${op}: ${error.message}`);
}

export class SupabaseDb implements Db {
  private readonly sb: SupabaseClient;

  constructor(url: string, serviceRoleKey: string, client?: SupabaseClient) {
    this.sb = client ?? createClient(url, serviceRoleKey, { auth: { persistSession: false, autoRefreshToken: false } });
  }

  // ---- customers ------------------------------------------------------------
  async findCustomerById(id: string) {
    const { data, error } = await this.sb.from("customers").select("*").eq("id", id).maybeSingle();
    if (error) fail("findCustomerById", error);
    return (data as Customer | null) ?? null;
  }
  async findCustomerByPaddleId(paddleCustomerId: string) {
    const { data, error } = await this.sb.from("customers").select("*").eq("paddle_customer_id", paddleCustomerId).maybeSingle();
    if (error) fail("findCustomerByPaddleId", error);
    return (data as Customer | null) ?? null;
  }
  async findCustomersByEmail(email: string) {
    const { data, error } = await this.sb.from("customers").select("*").eq("email", email);
    if (error) fail("findCustomersByEmail", error);
    return (data ?? []) as Customer[];
  }
  async upsertCustomer(paddleCustomerId: string, email: string) {
    const { data, error } = await this.sb
      .from("customers")
      .upsert({ paddle_customer_id: paddleCustomerId, email, updated_at: new Date().toISOString() }, { onConflict: "paddle_customer_id" })
      .select("*")
      .single();
    if (error) fail("upsertCustomer", error);
    return data as Customer;
  }
  async updateCustomerEmail(paddleCustomerId: string, email: string) {
    const { error } = await this.sb.from("customers").update({ email, updated_at: new Date().toISOString() }).eq("paddle_customer_id", paddleCustomerId);
    if (error) fail("updateCustomerEmail", error);
  }

  // ---- entitlements ---------------------------------------------------------
  private entFromRow(r: Row): Entitlement {
    return { ...(r as unknown as Entitlement), product_key_hash: fromBytea(r.product_key_hash) };
  }
  async insertEntitlement(ent: Omit<Entitlement, "created_at" | "updated_at">) {
    const { data, error } = await this.sb
      .from("entitlements")
      .insert({ ...ent, product_key_hash: toBytea(ent.product_key_hash) })
      .select("*")
      .single();
    if (error) fail("insertEntitlement", error);
    return this.entFromRow(data as Row);
  }
  private async entWhere(col: string, value: string) {
    const { data, error } = await this.sb.from("entitlements").select("*").eq(col, value).maybeSingle();
    if (error) fail(`findEntitlement(${col})`, error);
    return data ? this.entFromRow(data as Row) : null;
  }
  findEntitlementById(id: string) {
    return this.entWhere("id", id);
  }
  findEntitlementByKeyHash(hash: string) {
    return this.entWhere("product_key_hash", toBytea(hash));
  }
  findEntitlementByTransactionId(txnId: string) {
    return this.entWhere("paddle_transaction_id", txnId);
  }
  findEntitlementBySubscriptionId(subId: string) {
    return this.entWhere("paddle_subscription_id", subId);
  }
  async listEntitlementsForCustomers(customerIds: string[]) {
    if (customerIds.length === 0) return [];
    const { data, error } = await this.sb.from("entitlements").select("*").in("customer_id", customerIds).order("created_at");
    if (error) fail("listEntitlementsForCustomers", error);
    return ((data ?? []) as Row[]).map((r) => this.entFromRow(r));
  }
  async updateEntitlement(id: string, patch: Patch<Entitlement>) {
    const body: Row = { ...patch, updated_at: new Date().toISOString() };
    if (typeof patch.product_key_hash === "string") body.product_key_hash = toBytea(patch.product_key_hash);
    const { data, error } = await this.sb.from("entitlements").update(body).eq("id", id).select("*").single();
    if (error) fail("updateEntitlement", error);
    return this.entFromRow(data as Row);
  }

  // ---- devices --------------------------------------------------------------
  async activateDevice(args: { entitlementId: string; kind: "desktop" | "web"; deviceHash: string; deviceName: string | null; platform: Device["platform"]; appVersion: string | null }): Promise<ActivateResult> {
    const { data, error } = await this.sb.rpc("activate_device", {
      p_ent: args.entitlementId,
      p_kind: args.kind,
      p_hash: args.deviceHash,
      p_name: args.deviceName,
      p_platform: args.platform,
      p_version: args.appVersion,
    });
    if (error) fail("activate_device", error);
    const r = data as {
      ok: boolean;
      existing?: boolean;
      used?: number;
      max?: number;
      device?: Device;
      reason?: string;
      devices?: { name: string | null; platform: Device["platform"]; last_seen_at: string }[];
    };
    if (r.ok) {
      return { ok: true, existing: Boolean(r.existing), used: Number(r.used ?? 0), max: Number(r.max ?? 0), device: r.device as Device };
    }
    return { ok: false, reason: "limit", devices: r.devices ?? [] };
  }
  async findActiveDevice(entitlementId: string, deviceHash: string) {
    const { data, error } = await this.sb.from("devices").select("*").eq("entitlement_id", entitlementId).eq("device_hash", deviceHash).is("deactivated_at", null).maybeSingle();
    if (error) fail("findActiveDevice", error);
    return (data as Device | null) ?? null;
  }
  async findDeviceById(id: string) {
    const { data, error } = await this.sb.from("devices").select("*").eq("id", id).maybeSingle();
    if (error) fail("findDeviceById", error);
    return (data as Device | null) ?? null;
  }
  async listActiveDevices(entitlementId: string) {
    const { data, error } = await this.sb.from("devices").select("*").eq("entitlement_id", entitlementId).is("deactivated_at", null).order("activated_at");
    if (error) fail("listActiveDevices", error);
    return (data ?? []) as Device[];
  }
  async touchDevice(id: string, appVersion: string | null) {
    const patch: Row = { last_seen_at: new Date().toISOString() };
    if (appVersion !== null) patch.app_version = appVersion;
    const { error } = await this.sb.from("devices").update(patch).eq("id", id);
    if (error) fail("touchDevice", error);
  }
  async deactivateDevice(id: string, by: DeactivatedBy) {
    const { error } = await this.sb.from("devices").update({ deactivated_at: new Date().toISOString(), deactivated_by: by }).eq("id", id).is("deactivated_at", null);
    if (error) fail("deactivateDevice", error);
  }
  async deactivateWebDevices(entitlementId: string, by: DeactivatedBy) {
    const { data, error } = await this.sb
      .from("devices")
      .update({ deactivated_at: new Date().toISOString(), deactivated_by: by })
      .eq("entitlement_id", entitlementId)
      .eq("kind", "web")
      .is("deactivated_at", null)
      .select("id");
    if (error) fail("deactivateWebDevices", error);
    return (data ?? []).length;
  }
  async countDashboardDeactivationsSince(entitlementId: string, since: string) {
    const { count, error } = await this.sb
      .from("devices")
      .select("id", { count: "exact", head: true })
      .eq("entitlement_id", entitlementId)
      .eq("deactivated_by", "dashboard")
      .gte("deactivated_at", since);
    if (error) fail("countDashboardDeactivationsSince", error);
    return count ?? 0;
  }
  async markIdleDesktopDevices(unseenSince: string) {
    const { data, error } = await this.sb
      .from("devices")
      .update({ deactivated_at: new Date().toISOString(), deactivated_by: "idle" })
      .eq("kind", "desktop")
      .is("deactivated_at", null)
      .lt("last_seen_at", unseenSince)
      .select("id");
    if (error) fail("markIdleDesktopDevices", error);
    return (data ?? []).length;
  }

  // ---- claims ---------------------------------------------------------------
  private claimFromRow(r: Row): Claim {
    return { ...(r as unknown as Claim), secret_hash: fromBytea(r.secret_hash) };
  }
  async insertClaim(claim: Omit<Claim, "created_at">) {
    const { data, error } = await this.sb.from("claims").insert({ ...claim, secret_hash: toBytea(claim.secret_hash) }).select("*").single();
    if (error) fail("insertClaim", error);
    return this.claimFromRow(data as Row);
  }
  async findClaim(id: string) {
    const { data, error } = await this.sb.from("claims").select("*").eq("id", id).maybeSingle();
    if (error) fail("findClaim", error);
    return data ? this.claimFromRow(data as Row) : null;
  }
  async updateClaim(id: string, patch: Patch<Claim>) {
    const body: Row = { ...patch };
    if (typeof patch.secret_hash === "string") body.secret_hash = toBytea(patch.secret_hash);
    const { error } = await this.sb.from("claims").update(body).eq("id", id);
    if (error) fail("updateClaim", error);
  }
  async expirePendingClaims(before: string) {
    const { data, error } = await this.sb.from("claims").update({ status: "expired" }).eq("status", "pending").lt("expires_at", before).select("id");
    if (error) fail("expirePendingClaims", error);
    return (data ?? []).length;
  }

  // ---- checkouts ------------------------------------------------------------
  private checkoutFromRow(r: Row): Checkout {
    return { ...(r as unknown as Checkout), secret_hash: fromBytea(r.secret_hash) };
  }
  async insertCheckout(checkout: Omit<Checkout, "created_at">) {
    const { data, error } = await this.sb.from("checkouts").insert({ ...checkout, secret_hash: toBytea(checkout.secret_hash) }).select("*").single();
    if (error) fail("insertCheckout", error);
    return this.checkoutFromRow(data as Row);
  }
  async findCheckout(id: string) {
    const { data, error } = await this.sb.from("checkouts").select("*").eq("id", id).maybeSingle();
    if (error) fail("findCheckout", error);
    return data ? this.checkoutFromRow(data as Row) : null;
  }
  async updateCheckout(id: string, patch: Patch<Checkout>) {
    const body: Row = { ...patch };
    if (typeof patch.secret_hash === "string") body.secret_hash = toBytea(patch.secret_hash);
    const { error } = await this.sb.from("checkouts").update(body).eq("id", id);
    if (error) fail("updateCheckout", error);
  }

  // ---- paddle events --------------------------------------------------------
  async insertEventIfAbsent(ev: Omit<PaddleEvent, "received_at" | "status" | "attempts" | "last_error" | "processed_at">) {
    // upsert with ignoreDuplicates = INSERT ... ON CONFLICT DO NOTHING; it returns no row when
    // the event already existed, so we read it back in that case.
    const { data, error } = await this.sb
      .from("paddle_events")
      .upsert({ event_id: ev.event_id, event_type: ev.event_type, occurred_at: ev.occurred_at, payload: ev.payload }, { onConflict: "event_id", ignoreDuplicates: true })
      .select("*");
    if (error) fail("insertEventIfAbsent", error);
    const inserted = (data ?? [])[0] as PaddleEvent | undefined;
    if (inserted) return { inserted: true, row: inserted };
    const existing = await this.findEvent(ev.event_id);
    if (!existing) throw new DbUnavailable("insertEventIfAbsent: row vanished");
    return { inserted: false, row: existing };
  }
  async findEvent(eventId: string) {
    const { data, error } = await this.sb.from("paddle_events").select("*").eq("event_id", eventId).maybeSingle();
    if (error) fail("findEvent", error);
    return (data as PaddleEvent | null) ?? null;
  }
  async updateEvent(eventId: string, patch: Partial<Omit<PaddleEvent, "event_id">>) {
    const { error } = await this.sb.from("paddle_events").update(patch).eq("event_id", eventId);
    if (error) fail("updateEvent", error);
  }
  async listEventsToReprocess(receivedBefore: string, maxAttempts: number, limit: number) {
    const { data, error } = await this.sb
      .from("paddle_events")
      .select("*")
      .or(`and(status.eq.error,attempts.lt.${maxAttempts}),and(status.eq.received,received_at.lt.${receivedBefore})`)
      .order("occurred_at")
      .limit(limit);
    if (error) fail("listEventsToReprocess", error);
    return (data ?? []) as PaddleEvent[];
  }
  async listDeadLetterEvents(maxAttempts: number, limit: number) {
    const { data, error } = await this.sb.from("paddle_events").select("*").eq("status", "error").gte("attempts", maxAttempts).limit(limit);
    if (error) fail("listDeadLetterEvents", error);
    return (data ?? []) as PaddleEvent[];
  }

  // ---- adjustments ----------------------------------------------------------
  async insertAppliedAdjustment(adjustmentId: string, entitlementId: string, action: string) {
    const { data, error } = await this.sb
      .from("applied_adjustments")
      .upsert({ adjustment_id: adjustmentId, entitlement_id: entitlementId, action }, { onConflict: "adjustment_id", ignoreDuplicates: true })
      .select("adjustment_id");
    if (error) fail("insertAppliedAdjustment", error);
    return (data ?? []).length > 0;
  }

  // ---- otp ------------------------------------------------------------------
  async getOtp(email: string) {
    const { data, error } = await this.sb.from("otp_codes").select("*").eq("email", email).maybeSingle();
    if (error) fail("getOtp", error);
    if (!data) return null;
    const r = data as Row;
    return { ...(r as unknown as OtpRow), code_hmac: fromBytea(r.code_hmac) };
  }
  async upsertOtp(row: OtpRow) {
    const { error } = await this.sb.from("otp_codes").upsert({ ...row, code_hmac: toBytea(row.code_hmac) }, { onConflict: "email" });
    if (error) fail("upsertOtp", error);
  }
  async consumeOtpAttempt(email: string, max: number) {
    const { data, error } = await this.sb.rpc("consume_otp_attempt", { p_email: email, p_max: max });
    if (error) fail("consume_otp_attempt", error);
    const row = (Array.isArray(data) ? data[0] : data) as Row | undefined;
    if (!row) return null;
    return { code_hmac: fromBytea(row.code_hmac), expires_at: String(row.expires_at) };
  }
  async markOtpUsed(email: string, at: string) {
    const { error } = await this.sb.from("otp_codes").update({ used_at: at }).eq("email", email);
    if (error) fail("markOtpUsed", error);
  }
  async deleteOtpUsedOrExpired(now: string) {
    const { data, error } = await this.sb.from("otp_codes").delete().or(`used_at.not.is.null,expires_at.lte.${now}`).select("email");
    if (error) fail("deleteOtpUsedOrExpired", error);
    return (data ?? []).length;
  }

  // ---- sessions -------------------------------------------------------------
  async insertSession(s: Session) {
    const { error } = await this.sb.from("sessions").insert({ ...s, id_hash: toBytea(s.id_hash) });
    if (error) fail("insertSession", error);
  }
  async findSession(idHash: string) {
    const { data, error } = await this.sb.from("sessions").select("*").eq("id_hash", toBytea(idHash)).maybeSingle();
    if (error) fail("findSession", error);
    if (!data) return null;
    const r = data as Row;
    return { ...(r as unknown as Session), id_hash: fromBytea(r.id_hash) };
  }
  async touchSession(idHash: string, lastUsedAt: string, expiresAt: string) {
    const { error } = await this.sb.from("sessions").update({ last_used_at: lastUsedAt, expires_at: expiresAt }).eq("id_hash", toBytea(idHash));
    if (error) fail("touchSession", error);
  }
  async revokeSession(idHash: string, at: string) {
    const { error } = await this.sb.from("sessions").update({ revoked_at: at }).eq("id_hash", toBytea(idHash)).is("revoked_at", null);
    if (error) fail("revokeSession", error);
  }
  async revokeSessionsForEmail(email: string, at: string) {
    const { error } = await this.sb.from("sessions").update({ revoked_at: at }).eq("email", email).is("revoked_at", null);
    if (error) fail("revokeSessionsForEmail", error);
  }

  // ---- email log ------------------------------------------------------------
  async insertEmailLogIfAbsent(kind: string, ref: string, to: string) {
    const { data, error } = await this.sb
      .from("email_log")
      .upsert({ kind, ref, to_email: to }, { onConflict: "kind,ref", ignoreDuplicates: true })
      .select("id");
    if (error) fail("insertEmailLogIfAbsent", error);
    const inserted = (data ?? [])[0] as { id: string } | undefined;
    if (inserted) return { inserted: true, id: inserted.id };
    const { data: existing, error: e2 } = await this.sb.from("email_log").select("id").eq("kind", kind).eq("ref", ref).single();
    if (e2) fail("insertEmailLogIfAbsent(read)", e2);
    return { inserted: false, id: (existing as { id: string }).id };
  }
  async setEmailLogResendId(id: string, resendId: string) {
    const { error } = await this.sb.from("email_log").update({ resend_id: resendId }).eq("id", id);
    if (error) fail("setEmailLogResendId", error);
  }

  // ---- audit ----------------------------------------------------------------
  async audit(entry: AuditEntry) {
    const { error } = await this.sb.from("audit_log").insert({ actor: entry.actor, action: entry.action, entitlement_id: entry.entitlement_id ?? null, detail: entry.detail ?? null });
    if (error) fail("audit", error);
  }

  // ---- rate limits ----------------------------------------------------------
  async rateLimitHit(key: string, windowSeconds: number, max: number) {
    const { data, error } = await this.sb.rpc("rate_limit_hit", { p_key: key, p_window_seconds: windowSeconds, p_max: max });
    if (error) fail("rate_limit_hit", error);
    if (typeof data !== "boolean") throw new DbUnavailable("rate_limit_hit returned no boolean");
    return data;
  }
  async deleteRateLimitsBefore(before: string) {
    const { data, error } = await this.sb.from("rate_limits").delete().lt("window_start", before).select("key");
    if (error) fail("deleteRateLimitsBefore", error);
    return (data ?? []).length;
  }
}
