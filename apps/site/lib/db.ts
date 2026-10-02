// Repository interface over the schema in supabase/migrations (DESIGN.md 5.9). Two
// implementations: SupabaseDb (service role, production) and MemoryDb (tests and local dev with
// SMG_DB=memory). Both honour the same semantics, including activate_device's locked device
// limit and consume_otp_attempt's atomic attempt charge.

import type { EntitlementStatus, Kind, Plan, Platform } from "@cia/api-types";

export interface Customer {
  id: string;
  paddle_customer_id: string | null;
  email: string;
  created_at: string;
  updated_at: string;
}

export type RevokedReason = "refund" | "chargeback" | "manual";

export interface Entitlement {
  id: string;
  customer_id: string;
  plan: Plan;
  status: EntitlementStatus;
  /** sha256 hex. */
  product_key_hash: string;
  product_key_enc: string;
  key4: string;
  max_devices: number;
  max_web: number;
  paddle_transaction_id: string | null;
  paddle_subscription_id: string | null;
  paddle_price_id: string;
  access_until: string | null;
  cancel_at: string | null;
  revoked_at: string | null;
  revoked_reason: RevokedReason | null;
  last_event_at: string | null;
  created_at: string;
  updated_at: string;
}

export type DeactivatedBy = "device" | "dashboard" | "idle" | "evicted" | "support";

export interface Device {
  id: string;
  entitlement_id: string;
  kind: Kind;
  device_hash: string;
  device_name: string | null;
  platform: Platform | null;
  app_version: string | null;
  activated_at: string;
  last_seen_at: string;
  deactivated_at: string | null;
  deactivated_by: DeactivatedBy | null;
}

export interface Claim {
  id: string;
  secret_hash: string;
  kind: Kind;
  device_hash: string;
  device_name: string | null;
  platform: string | null;
  entitlement_id: string | null;
  status: "pending" | "fulfilled" | "expired";
  created_at: string;
  expires_at: string;
}

export interface Checkout {
  id: string;
  secret_hash: string;
  plan: Plan;
  paddle_price_id: string;
  paddle_transaction_id: string | null;
  claim_id: string | null;
  entitlement_id: string | null;
  created_at: string;
}

export type EventStatus = "received" | "processed" | "ignored" | "error";

export interface PaddleEvent {
  event_id: string;
  event_type: string;
  occurred_at: string;
  payload: unknown;
  received_at: string;
  status: EventStatus;
  attempts: number;
  last_error: string | null;
  processed_at: string | null;
}

export interface OtpRow {
  email: string;
  /** hex */
  code_hmac: string;
  sent_at: string;
  expires_at: string;
  attempts: number;
  used_at: string | null;
}

export interface Session {
  id_hash: string;
  email: string;
  created_at: string;
  last_used_at: string;
  expires_at: string;
  revoked_at: string | null;
  user_agent: string | null;
}

export interface AuditEntry {
  actor: "webhook" | "api" | "dashboard" | "support" | "cron";
  action: string;
  entitlement_id?: string | null;
  detail?: unknown;
}

export type ActivateResult =
  | { ok: true; existing: boolean; used: number; max: number; device: Device }
  | { ok: false; reason: "limit"; devices: { name: string | null; platform: Platform | null; last_seen_at: string }[] };

export class UniqueViolation extends Error {
  constructor(public readonly constraint: string) {
    super(`unique violation: ${constraint}`);
    this.name = "UniqueViolation";
  }
}

export class DbUnavailable extends Error {
  constructor(message: string) {
    super(message);
    this.name = "DbUnavailable";
  }
}

export type Patch<T> = Partial<Omit<T, "id">>;

export interface Db {
  // customers
  findCustomerById(id: string): Promise<Customer | null>;
  findCustomerByPaddleId(paddleCustomerId: string): Promise<Customer | null>;
  findCustomersByEmail(email: string): Promise<Customer[]>;
  /** Insert or update the email for a Paddle customer id. */
  upsertCustomer(paddleCustomerId: string, email: string): Promise<Customer>;
  updateCustomerEmail(paddleCustomerId: string, email: string): Promise<void>;

  // entitlements
  insertEntitlement(ent: Omit<Entitlement, "created_at" | "updated_at">): Promise<Entitlement>;
  findEntitlementById(id: string): Promise<Entitlement | null>;
  findEntitlementByKeyHash(hash: string): Promise<Entitlement | null>;
  findEntitlementByTransactionId(txnId: string): Promise<Entitlement | null>;
  findEntitlementBySubscriptionId(subId: string): Promise<Entitlement | null>;
  listEntitlementsForCustomers(customerIds: string[]): Promise<Entitlement[]>;
  updateEntitlement(id: string, patch: Patch<Entitlement>): Promise<Entitlement>;

  // devices (activate_device RPC semantics)
  activateDevice(args: { entitlementId: string; kind: Kind; deviceHash: string; deviceName: string | null; platform: Platform | null; appVersion: string | null }): Promise<ActivateResult>;
  findActiveDevice(entitlementId: string, deviceHash: string): Promise<Device | null>;
  findDeviceById(id: string): Promise<Device | null>;
  listActiveDevices(entitlementId: string): Promise<Device[]>;
  touchDevice(id: string, appVersion: string | null): Promise<void>;
  deactivateDevice(id: string, by: DeactivatedBy): Promise<void>;
  deactivateWebDevices(entitlementId: string, by: DeactivatedBy): Promise<number>;
  countDashboardDeactivationsSince(entitlementId: string, since: string): Promise<number>;
  markIdleDesktopDevices(unseenSince: string): Promise<number>;

  // claims
  insertClaim(claim: Omit<Claim, "created_at">): Promise<Claim>;
  findClaim(id: string): Promise<Claim | null>;
  updateClaim(id: string, patch: Patch<Claim>): Promise<void>;
  expirePendingClaims(before: string): Promise<number>;

  // checkouts
  insertCheckout(checkout: Omit<Checkout, "created_at">): Promise<Checkout>;
  findCheckout(id: string): Promise<Checkout | null>;
  updateCheckout(id: string, patch: Patch<Checkout>): Promise<void>;

  // paddle events
  /** insert ... on conflict do nothing; returns the existing row when it was already there. */
  insertEventIfAbsent(ev: Omit<PaddleEvent, "received_at" | "status" | "attempts" | "last_error" | "processed_at">): Promise<{ inserted: boolean; row: PaddleEvent }>;
  findEvent(eventId: string): Promise<PaddleEvent | null>;
  updateEvent(eventId: string, patch: Partial<Omit<PaddleEvent, "event_id">>): Promise<void>;
  /** `error` rows with attempts < maxAttempts and `received` rows older than `receivedBefore`. */
  listEventsToReprocess(receivedBefore: string, maxAttempts: number, limit: number): Promise<PaddleEvent[]>;
  listDeadLetterEvents(maxAttempts: number, limit: number): Promise<PaddleEvent[]>;

  // adjustments
  /** false if the adjustment was applied before. */
  insertAppliedAdjustment(adjustmentId: string, entitlementId: string, action: string): Promise<boolean>;

  // otp
  getOtp(email: string): Promise<OtpRow | null>;
  upsertOtp(row: OtpRow): Promise<void>;
  /** consume_otp_attempt: atomically charges one attempt; null when missing, used or over the cap. */
  consumeOtpAttempt(email: string, max: number): Promise<{ code_hmac: string; expires_at: string } | null>;
  markOtpUsed(email: string, at: string): Promise<void>;
  deleteOtpUsedOrExpired(now: string): Promise<number>;

  // sessions
  insertSession(s: Session): Promise<void>;
  findSession(idHash: string): Promise<Session | null>;
  touchSession(idHash: string, lastUsedAt: string, expiresAt: string): Promise<void>;
  revokeSession(idHash: string, at: string): Promise<void>;
  revokeSessionsForEmail(email: string, at: string): Promise<void>;

  // email log
  /** Insert (kind, ref); false if it already existed. */
  insertEmailLogIfAbsent(kind: string, ref: string, to: string): Promise<{ inserted: boolean; id: string }>;
  setEmailLogResendId(id: string, resendId: string): Promise<void>;

  // audit
  audit(entry: AuditEntry): Promise<void>;

  // rate limits (rate_limit_hit RPC semantics; throws DbUnavailable on error)
  rateLimitHit(key: string, windowSeconds: number, max: number): Promise<boolean>;
  deleteRateLimitsBefore(before: string): Promise<number>;
}
