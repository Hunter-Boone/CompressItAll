import { describe, expect, it } from "vitest";

import { describeStatus, grantsAccess, projectSubscription } from "../lib/entitlements";

const now = new Date("2026-10-02T12:00:00Z");
const past = "2026-10-01T00:00:00Z";
const future = "2027-10-02T00:00:00Z";

describe("grantsAccess", () => {
  const table: [plan: "lifetime" | "yearly", status: "active" | "past_due" | "ended" | "revoked", access_until: string | null, expected: boolean][] = [
    ["lifetime", "active", null, true],
    ["lifetime", "past_due", null, false],
    ["lifetime", "ended", null, false],
    ["lifetime", "revoked", null, false],
    ["lifetime", "active", past, true],
    ["yearly", "active", future, true],
    ["yearly", "active", past, false],
    ["yearly", "active", null, false],
    ["yearly", "past_due", future, true],
    ["yearly", "past_due", past, false],
    ["yearly", "ended", future, true],
    ["yearly", "ended", past, false],
    ["yearly", "revoked", future, false],
    ["yearly", "revoked", past, false],
  ];
  for (const [plan, status, access_until, expected] of table) {
    it(`${plan} ${status} access_until=${access_until} -> ${expected}`, () => {
      expect(grantsAccess({ plan, status, access_until }, now)).toBe(expected);
    });
  }
  it("access_until exactly now is not access", () => {
    expect(grantsAccess({ plan: "yearly", status: "active", access_until: now.toISOString() }, now)).toBe(false);
  });
});

describe("projectSubscription", () => {
  const period = { starts_at: "2026-10-02T11:58:00Z", ends_at: "2027-10-02T11:58:00Z" };
  it("active", () => {
    expect(projectSubscription({ id: "s", status: "active", current_billing_period: period })).toEqual({ status: "active", access_until: period.ends_at, cancel_at: null });
  });
  it("active with scheduled cancel", () => {
    expect(projectSubscription({ id: "s", status: "active", current_billing_period: period, scheduled_change: { action: "cancel", effective_at: period.ends_at } })).toEqual({ status: "active", access_until: period.ends_at, cancel_at: period.ends_at });
  });
  it("scheduled pause is not a cancel", () => {
    expect(projectSubscription({ id: "s", status: "active", current_billing_period: period, scheduled_change: { action: "pause", effective_at: period.ends_at } }).cancel_at).toBeNull();
  });
  it("past_due adds 30 days of dunning", () => {
    expect(projectSubscription({ id: "s", status: "past_due", current_billing_period: period })).toEqual({ status: "past_due", access_until: "2027-11-01T11:58:00.000Z", cancel_at: null });
  });
  it("canceled and paused end at their timestamps", () => {
    expect(projectSubscription({ id: "s", status: "canceled", canceled_at: "2027-01-01T00:00:00Z", current_billing_period: null })).toEqual({ status: "ended", access_until: "2027-01-01T00:00:00Z", cancel_at: null });
    expect(projectSubscription({ id: "s", status: "paused", paused_at: "2027-02-01T00:00:00Z", current_billing_period: period })).toEqual({ status: "ended", access_until: "2027-02-01T00:00:00Z", cancel_at: null });
  });
});

describe("describeStatus", () => {
  const fmt = (iso: string) => iso.slice(0, 10);
  it("covers the dashboard lines", () => {
    expect(describeStatus({ plan: "lifetime", status: "active", access_until: null, cancel_at: null, revoked_at: null, created_at: "2026-10-02T12:00:00Z" }, fmt)).toBe("bought 2026-10-02");
    expect(describeStatus({ plan: "yearly", status: "active", access_until: future, cancel_at: null, revoked_at: null, created_at: past }, fmt)).toBe("renews 2027-10-02");
    expect(describeStatus({ plan: "yearly", status: "active", access_until: future, cancel_at: future, revoked_at: null, created_at: past }, fmt)).toBe("ends 2027-10-02");
    expect(describeStatus({ plan: "yearly", status: "past_due", access_until: future, cancel_at: null, revoked_at: null, created_at: past }, fmt)).toContain("Payment problem");
    expect(describeStatus({ plan: "lifetime", status: "revoked", access_until: null, cancel_at: null, revoked_at: "2026-10-05T00:00:00Z", created_at: past }, fmt)).toBe("Refunded on 2026-10-05");
  });
});
