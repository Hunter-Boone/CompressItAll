// zod schemas for every /api/v1 request and response (DESIGN.md 5.10).
// Server routes parse requests with these and type their responses with them; the apps
// and the site parse responses with them. Timestamps in JSON bodies are ISO 8601 strings
// unless a field name ends in `_unix`.

import { z } from "zod";

import { KindSchema, PlanSchema } from "./license-token.js";

export { KindSchema, PlanSchema };

export const PlatformSchema = z.enum(["windows", "macos", "linux", "web"]);
export type Platform = z.infer<typeof PlatformSchema>;

export const EntitlementStatusSchema = z.enum(["active", "past_due", "ended", "revoked"]);
export type EntitlementStatus = z.infer<typeof EntitlementStatusSchema>;

const isoDate = z.iso.datetime({ offset: true });
const idWithPrefix = (prefix: string) =>
  z.string().regex(new RegExp(`^${prefix}_[0-9A-HJKMNP-TV-Z]{26}$`), `${prefix}_ + ULID`);

export const EntitlementIdSchema = idWithPrefix("ent");
export const ClaimIdSchema = idWithPrefix("clm");
export const CheckoutIdSchema = idWithPrefix("chk");

/** `d_` + 26 base32 chars (desktop) or `w_` + 26 base32 chars (web). */
export const DeviceHashSchema = z.string().regex(/^[dw]_[a-z2-7]{26}$/);
/** Display form or anything `normalise` accepts; the server normalises. */
export const ProductKeyInputSchema = z.string().min(20).max(64);
export const LicenseTokenSchema = z.string().regex(/^SMG1\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]{86}$/);
export const EmailSchema = z.string().trim().toLowerCase().pipe(z.email().max(254));
export const OkResponseSchema = z.object({ ok: z.literal(true) });
export type OkResponse = z.infer<typeof OkResponseSchema>;

// ---- errors ------------------------------------------------------------------

export const ErrorCodeSchema = z.enum([
  "bad_request",
  "unauthorized",
  "forbidden",
  "not_found",
  "rate_limited",
  "key_not_found",
  "revoked",
  "ended",
  "device_limit",
  "invalid_code",
  "claim_expired",
  "internal",
  "unavailable",
]);
export type ErrorCode = z.infer<typeof ErrorCodeSchema>;

export const ErrorResponseSchema = z.object({
  error: z.string().regex(/^[a-z][a-z0-9_]*$/),
  message: z.string(),
});
export type ErrorResponse = z.infer<typeof ErrorResponseSchema>;

/** 409 from POST /license/activate: the computers currently holding the key. */
export const DeviceLimitErrorSchema = ErrorResponseSchema.extend({
  error: z.literal("device_limit"),
  devices: z.array(
    z.object({
      name: z.string().nullable(),
      platform: PlatformSchema.nullable(),
      last_seen_at: isoDate,
    }),
  ),
});
export type DeviceLimitError = z.infer<typeof DeviceLimitErrorSchema>;

/** 403 from POST /license/activate. */
export const RevokedErrorSchema = ErrorResponseSchema.extend({
  error: z.enum(["revoked", "ended"]),
  reason: z.string().optional(),
  at: isoDate.optional(),
});

// ---- claims: app -> buy -> unlock by itself (5.6.1) --------------------------

export const ClaimsCreateRequestSchema = z.object({
  kind: KindSchema,
  device_hash: DeviceHashSchema,
  device_name: z.string().max(100).nullable().optional(),
  platform: PlatformSchema,
});
export type ClaimsCreateRequest = z.infer<typeof ClaimsCreateRequestSchema>;

export const ClaimsCreateResponseSchema = z.object({
  claim_id: ClaimIdSchema,
  claim_secret: z.string().min(16),
  buy_url: z.url(),
  expires_at: isoDate,
});
export type ClaimsCreateResponse = z.infer<typeof ClaimsCreateResponseSchema>;

export const ClaimStatusResponseSchema = z.discriminatedUnion("status", [
  z.object({ status: z.literal("pending") }),
  z.object({
    status: z.literal("fulfilled"),
    product_key: z.string(),
    token: LicenseTokenSchema,
    plan: PlanSchema,
  }),
  z.object({ status: z.literal("expired") }),
]);
export type ClaimStatusResponse = z.infer<typeof ClaimStatusResponseSchema>;

// ---- checkout: site browser (5.8.3) ------------------------------------------

export const CheckoutCreateRequestSchema = z.object({
  plan: PlanSchema,
  claim_id: ClaimIdSchema.optional(),
});
export type CheckoutCreateRequest = z.infer<typeof CheckoutCreateRequestSchema>;

/** Plus the `smg_chk` cookie. */
export const CheckoutCreateResponseSchema = z.object({ transaction_id: z.string().min(1) });
export type CheckoutCreateResponse = z.infer<typeof CheckoutCreateResponseSchema>;

export const CheckoutStatusResponseSchema = z.discriminatedUnion("status", [
  z.object({ status: z.literal("pending") }),
  z.object({ status: z.literal("fulfilled"), product_key: z.string(), plan: PlanSchema }),
]);
export type CheckoutStatusResponse = z.infer<typeof CheckoutStatusResponseSchema>;

// ---- license: activate / refresh / deactivate / resend ----------------------

export const LicenseActivateRequestSchema = z.object({
  product_key: ProductKeyInputSchema,
  kind: KindSchema,
  device_hash: DeviceHashSchema,
  device_name: z.string().max(100).nullable().optional(),
  platform: PlatformSchema,
  app_version: z.string().max(40),
});
export type LicenseActivateRequest = z.infer<typeof LicenseActivateRequestSchema>;

export const LicenseActivateResponseSchema = z.object({
  token: LicenseTokenSchema,
  plan: PlanSchema,
  devices_used: z.number().int().nonnegative(),
  devices_max: z.number().int().positive(),
});
export type LicenseActivateResponse = z.infer<typeof LicenseActivateResponseSchema>;

export const LicenseRefreshRequestSchema = z.object({ token: LicenseTokenSchema });
export type LicenseRefreshRequest = z.infer<typeof LicenseRefreshRequestSchema>;

export const LicenseRefreshResponseSchema = z.discriminatedUnion("status", [
  z.object({ status: z.literal("ok"), token: LicenseTokenSchema }),
  z.object({
    status: z.enum(["revoked", "ended", "deactivated"]),
    reason: z.string(),
    at: isoDate,
  }),
]);
export type LicenseRefreshResponse = z.infer<typeof LicenseRefreshResponseSchema>;

export const LicenseDeactivateRequestSchema = z.object({ token: LicenseTokenSchema });
export type LicenseDeactivateRequest = z.infer<typeof LicenseDeactivateRequestSchema>;
export const LicenseDeactivateResponseSchema = OkResponseSchema;

export const LicenseResendRequestSchema = z.object({ email: EmailSchema });
export type LicenseResendRequest = z.infer<typeof LicenseResendRequestSchema>;
/** Always `{ok:true}`, whether or not the address is known. */
export const LicenseResendResponseSchema = OkResponseSchema;

// ---- auth: dashboard sign-in (5.6.5) ----------------------------------------

export const OtpSendRequestSchema = z.object({ email: EmailSchema });
export type OtpSendRequest = z.infer<typeof OtpSendRequestSchema>;
/** Always `{sent:true}`. */
export const OtpSendResponseSchema = z.object({ sent: z.literal(true) });
export type OtpSendResponse = z.infer<typeof OtpSendResponseSchema>;

export const OtpVerifyRequestSchema = z.object({
  email: EmailSchema,
  code: z.string().regex(/^\d{6}$/),
});
export type OtpVerifyRequest = z.infer<typeof OtpVerifyRequestSchema>;
/** Plus the `smg_session` cookie; 400 `invalid_code` otherwise. */
export const OtpVerifyResponseSchema = OkResponseSchema;

export const LogoutRequestSchema = z.object({ everywhere: z.boolean().optional() });
export type LogoutRequest = z.infer<typeof LogoutRequestSchema>;
export const LogoutResponseSchema = OkResponseSchema;

// ---- account dashboard (5.12) -----------------------------------------------

export const AccountDeviceSchema = z.object({
  id: z.uuid(),
  kind: KindSchema,
  name: z.string().nullable(),
  platform: PlatformSchema.nullable(),
  app_version: z.string().nullable(),
  activated_at: isoDate,
  last_seen_at: isoDate,
});
export type AccountDevice = z.infer<typeof AccountDeviceSchema>;

export const AccountEntitlementSchema = z.object({
  id: EntitlementIdSchema,
  plan: PlanSchema,
  status: EntitlementStatusSchema,
  /** `XXXXX-XXXXX-XXXXX-7KQ2P` style, see product-key masked(). */
  key_masked: z.string(),
  key4: z.string().length(5),
  created_at: isoDate,
  /** null for lifetime. */
  access_until: isoDate.nullable(),
  cancel_at: isoDate.nullable(),
  devices_max: z.number().int().positive(),
  /** Active desktop devices. */
  devices: z.array(AccountDeviceSchema),
  /** Active web installs (not listed individually). */
  web_count: z.number().int().nonnegative(),
});
export type AccountEntitlement = z.infer<typeof AccountEntitlementSchema>;

export const AccountResponseSchema = z.object({
  email: z.string(),
  entitlements: z.array(AccountEntitlementSchema),
});
export type AccountResponse = z.infer<typeof AccountResponseSchema>;

export const RevealKeyRequestSchema = z.object({ entitlement_id: EntitlementIdSchema });
export type RevealKeyRequest = z.infer<typeof RevealKeyRequestSchema>;
export const RevealKeyResponseSchema = z.object({ product_key: z.string() });
export type RevealKeyResponse = z.infer<typeof RevealKeyResponseSchema>;

/** POST /account/devices/{id}/deactivate has no body; 429 after 10 per entitlement per 30 days. */
export const DeviceDeactivateResponseSchema = OkResponseSchema;

export const WebSignoutRequestSchema = z.object({ entitlement_id: EntitlementIdSchema });
export type WebSignoutRequest = z.infer<typeof WebSignoutRequestSchema>;
export const WebSignoutResponseSchema = OkResponseSchema;

export const PortalRequestSchema = z.object({ entitlement_id: EntitlementIdSchema });
export type PortalRequest = z.infer<typeof PortalRequestSchema>;
/** Temporary Paddle portal URL; never cache it. */
export const PortalResponseSchema = z.object({ url: z.url() });
export type PortalResponse = z.infer<typeof PortalResponseSchema>;

// ---- updater (6.3) ----------------------------------------------------------

export const UpdateChannelSchema = z.enum(["stable", "beta", "dev"]);
export type UpdateChannel = z.infer<typeof UpdateChannelSchema>;

/** Tauri updater response; the route answers 204 with no body when up to date. */
export const UpdateResponseSchema = z.object({
  version: z.string().min(1),
  notes: z.string().optional(),
  pub_date: isoDate.optional(),
  url: z.url(),
  signature: z.string().min(1),
});
export type UpdateResponse = z.infer<typeof UpdateResponseSchema>;

// ---- cron / health ----------------------------------------------------------

export const CronResponseSchema = z.object({
  ok: z.literal(true),
  /** Free-form counters, e.g. `{ reprocessed: 3, dead_lettered: 0 }`. */
  counts: z.record(z.string(), z.number().int().nonnegative()).optional(),
});
export type CronResponse = z.infer<typeof CronResponseSchema>;

export const HealthResponseSchema = z.object({ ok: z.literal(true), version: z.string() });
export type HealthResponse = z.infer<typeof HealthResponseSchema>;
