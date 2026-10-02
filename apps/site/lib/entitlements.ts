// Access rule and subscription projection (DESIGN.md 5.7). `grantsAccess` is the one function
// every route uses to decide whether an entitlement currently unlocks Pro.

import type { Entitlement } from "./db";

export function grantsAccess(ent: Pick<Entitlement, "plan" | "status" | "access_until">, now: Date): boolean {
  if (ent.plan === "lifetime") return ent.status === "active";
  // yearly
  if (ent.status === "revoked") return false;
  if (ent.access_until === null) return false;
  const until = new Date(ent.access_until).getTime();
  if (Number.isNaN(until)) return false;
  return (ent.status === "active" || ent.status === "past_due" || ent.status === "ended") && until > now.getTime();
}

export const PAST_DUE_GRACE_MS = 30 * 24 * 3600 * 1000;

/** The parts of a Paddle subscription the projection reads. */
export interface PaddleSubscriptionLike {
  id: string;
  status: string;
  current_billing_period?: { starts_at: string; ends_at: string } | null;
  scheduled_change?: { action: string; effective_at: string; resume_at?: string | null } | null;
  canceled_at?: string | null;
  paused_at?: string | null;
}

export interface Projection {
  status: "active" | "past_due" | "ended";
  access_until: string | null;
  cancel_at: string | null;
}

/** Project a subscription's current state onto entitlement columns (DESIGN.md 5.8.2). */
export function projectSubscription(sub: PaddleSubscriptionLike): Projection {
  const periodEnd = sub.current_billing_period?.ends_at ?? null;
  const cancelAt = sub.scheduled_change?.action === "cancel" ? sub.scheduled_change.effective_at : null;
  switch (sub.status) {
    case "active":
    case "trialing":
      return { status: "active", access_until: periodEnd, cancel_at: cancelAt };
    case "past_due":
      return {
        status: "past_due",
        access_until: periodEnd ? new Date(new Date(periodEnd).getTime() + PAST_DUE_GRACE_MS).toISOString() : null,
        cancel_at: cancelAt,
      };
    case "canceled":
      return { status: "ended", access_until: sub.canceled_at ?? periodEnd, cancel_at: null };
    case "paused":
      return { status: "ended", access_until: sub.paused_at ?? periodEnd, cancel_at: null };
    default:
      // Unknown statuses keep whatever the period says; never widen access.
      return { status: "ended", access_until: periodEnd, cancel_at: null };
  }
}

/** The dashboard's one-line status for an entitlement (DESIGN.md 5.12). */
export function describeStatus(ent: Pick<Entitlement, "plan" | "status" | "access_until" | "cancel_at" | "revoked_at" | "created_at">, fmt: (iso: string) => string): string {
  if (ent.status === "revoked") return `Refunded on ${ent.revoked_at ? fmt(ent.revoked_at) : "an unknown date"}`;
  if (ent.plan === "lifetime") return `bought ${fmt(ent.created_at)}`;
  if (ent.status === "past_due") return "Payment problem: update your card in Manage billing";
  if (ent.status === "ended") return ent.access_until ? `ends ${fmt(ent.access_until)}` : "ended";
  if (ent.cancel_at) return `ends ${fmt(ent.cancel_at)}`;
  return ent.access_until ? `renews ${fmt(ent.access_until)}` : "active";
}
