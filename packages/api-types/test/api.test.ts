import { describe, expect, it } from "vitest";

import {
  AccountResponseSchema,
  ClaimStatusResponseSchema,
  ClaimsCreateRequestSchema,
  DeviceHashSchema,
  DeviceLimitErrorSchema,
  ErrorResponseSchema,
  LicenseActivateRequestSchema,
  LicenseRefreshResponseSchema,
  LicenseTokenSchema,
  OtpVerifyRequestSchema,
} from "../src/api.js";

const token =
  "SMG1.eyJ2IjoxfQ.cQHds0QQhI1vtPJ2QkZwpO8JE_gk1-IXX3iawJT7akS-Bz46r6ADQWbFNBWMPTime_5_cAjOm0iMEyUIkTmKAA";

describe("api schemas", () => {
  it("accepts well-formed requests and rejects junk", () => {
    expect(
      ClaimsCreateRequestSchema.safeParse({
        kind: "desktop",
        device_hash: "d_atdigb22hssonfwhpkhnk3koie",
        device_name: "HUNTERS-AMD",
        platform: "windows",
      }).success,
    ).toBe(true);
    expect(
      ClaimsCreateRequestSchema.safeParse({ kind: "phone", device_hash: "x", platform: "ios" })
        .success,
    ).toBe(false);
    expect(DeviceHashSchema.safeParse("w_ljnfuws2ljnfuws2ljnfuws2li").success).toBe(true);
    expect(DeviceHashSchema.safeParse("d_UPPER").success).toBe(false);
    expect(LicenseTokenSchema.safeParse(token).success).toBe(true);
    expect(LicenseTokenSchema.safeParse("SMG1.abc.def").success).toBe(false);
    expect(
      OtpVerifyRequestSchema.parse({ email: " Hunter@Example.com ", code: "123456" }).email,
    ).toBe("hunter@example.com");
    expect(OtpVerifyRequestSchema.safeParse({ email: "a@b.co", code: "12345" }).success).toBe(
      false,
    );
    expect(
      LicenseActivateRequestSchema.safeParse({
        product_key: "5aqqz-6ap8b-dh5bg-58tgn",
        kind: "web",
        device_hash: "w_ljnfuws2ljnfuws2ljnfuws2li",
        platform: "web",
        app_version: "0.1.0",
      }).success,
    ).toBe(true);
  });

  it("discriminates status unions", () => {
    expect(ClaimStatusResponseSchema.parse({ status: "pending" })).toEqual({ status: "pending" });
    expect(
      ClaimStatusResponseSchema.safeParse({
        status: "fulfilled",
        product_key: "X",
        token,
        plan: "lifetime",
      }).success,
    ).toBe(true);
    expect(ClaimStatusResponseSchema.safeParse({ status: "fulfilled" }).success).toBe(false);
    expect(LicenseRefreshResponseSchema.safeParse({ status: "ok", token }).success).toBe(true);
    expect(
      LicenseRefreshResponseSchema.safeParse({
        status: "revoked",
        reason: "refund",
        at: "2026-10-05T12:00:00Z",
      }).success,
    ).toBe(true);
    expect(LicenseRefreshResponseSchema.safeParse({ status: "revoked" }).success).toBe(false);
  });

  it("models error bodies", () => {
    expect(
      ErrorResponseSchema.safeParse({
        error: "key_not_found",
        message: "We couldn't find that key.",
      }).success,
    ).toBe(true);
    expect(ErrorResponseSchema.safeParse({ error: "KeyNotFound", message: "x" }).success).toBe(
      false,
    );
    expect(
      DeviceLimitErrorSchema.safeParse({
        error: "device_limit",
        message: "This key is already used on 3 computers.",
        devices: [{ name: "Laptop", platform: "macos", last_seen_at: "2026-10-01T08:00:00.000Z" }],
      }).success,
    ).toBe(true);
  });

  it("parses an account response", () => {
    const r = AccountResponseSchema.safeParse({
      email: "hunter@example.com",
      entitlements: [
        {
          id: "ent_01K6QW3Z8B4N7P2R9S5T6V7W8X",
          plan: "yearly",
          status: "active",
          key_masked: "5AQQZ-…-58TGN",
          key4: "58TGN",
          created_at: "2026-10-02T10:00:00Z",
          access_until: "2027-10-02T10:00:00Z",
          cancel_at: null,
          devices_max: 3,
          devices: [
            {
              id: "3b241101-e2bb-4255-8caf-4136c566a962",
              kind: "desktop",
              name: "HUNTERS-AMD",
              platform: "windows",
              app_version: "0.1.0",
              activated_at: "2026-10-02T10:00:00Z",
              last_seen_at: "2026-10-02T10:00:00Z",
            },
          ],
          web_count: 1,
        },
      ],
    });
    expect(r.success, JSON.stringify(r.error)).toBe(true);
  });
});
