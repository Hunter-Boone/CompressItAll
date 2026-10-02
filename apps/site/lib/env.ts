// Environment validation (DESIGN.md 5.13). Every variable is checked with zod; placeholder
// values and sandbox/production mismatches throw. Validation is lazy (first `getEnv()` call,
// i.e. at request time) so `next build` never needs secrets.

import { z } from "zod";

/** Markers from ConvertSave's lib/jwt.ts plus the ones used in .env.example. */
export const PLACEHOLDER_MARKERS = [
  "your-secret",
  "your-random",
  "secret-key-here",
  "change-in-production",
  "change-me",
  "changeme",
  "placeholder",
  "example-secret",
  "xxxxxxxx",
  "replace_me",
  "replace-me",
  "todo",
];

export function looksLikePlaceholder(value: string): boolean {
  const lowered = value.toLowerCase();
  return PLACEHOLDER_MARKERS.some((m) => lowered.includes(m));
}

const base64Bytes = (len: number) =>
  z
    .string()
    .refine((s) => {
      try {
        const buf = Buffer.from(s, "base64");
        return buf.length === len && buf.toString("base64").replace(/=+$/, "") === s.replace(/=+$/, "");
      } catch {
        return false;
      }
    }, `must be base64 of exactly ${len} bytes`);

const httpsUrl = z
  .string()
  .url()
  .refine((u) => /^https?:\/\//.test(u), "must be an http(s) URL")
  .transform((u) => u.replace(/\/+$/, ""));

const secret = (min: number) =>
  z.string().min(min).refine((s) => !looksLikePlaceholder(s), "placeholder value");

const prefixed = (prefix: string, min = prefix.length + 8) =>
  secret(min).refine((s) => s.startsWith(prefix), `must start with ${prefix}`);

export const EnvSchema = z
  .object({
    NEXT_PUBLIC_SITE_URL: httpsUrl,
    NEXT_PUBLIC_APP_URL: httpsUrl,
    SMG_DB: z.enum(["supabase", "memory"]).default("supabase"),
    SUPABASE_URL: httpsUrl.optional(),
    SUPABASE_SERVICE_ROLE_KEY: secret(20).optional(),
    PADDLE_ENV: z.enum(["sandbox", "production"]),
    PADDLE_API_KEY: secret(20),
    PADDLE_WEBHOOK_SECRET: prefixed("pdl_ntfset_"),
    PADDLE_PRICE_ID_LIFETIME: prefixed("pri_"),
    PADDLE_PRICE_ID_YEARLY: prefixed("pri_"),
    NEXT_PUBLIC_PADDLE_ENV: z.enum(["sandbox", "production"]),
    NEXT_PUBLIC_PADDLE_CLIENT_TOKEN: secret(10),
    RESEND_API_KEY: prefixed("re_"),
    EMAIL_FROM: z.string().min(3).refine((s) => s.includes("@") && !looksLikePlaceholder(s), "placeholder value"),
    EMAIL_REPLY_TO: z.string().email().refine((s) => !looksLikePlaceholder(s), "placeholder value"),
    LICENSE_SIGNING_KEY: base64Bytes(32),
    LICENSE_SIGNING_KID: z.string().regex(/^[0-9]{4}-[0-9]{2}$|^test$/),
    KEY_ENCRYPTION_KEY: base64Bytes(32),
    OTP_PEPPER: base64Bytes(32),
    IP_HASH_SALT: base64Bytes(16),
    CRON_SECRET: secret(16),
    ADMIN_ALERT_EMAIL: z.string().email().refine((s) => !looksLikePlaceholder(s), "placeholder value"),
  })
  .superRefine((e, ctx) => {
    const live = e.PADDLE_ENV === "production";
    if (e.NEXT_PUBLIC_PADDLE_ENV !== e.PADDLE_ENV) {
      ctx.addIssue({ code: "custom", path: ["NEXT_PUBLIC_PADDLE_ENV"], message: "must equal PADDLE_ENV" });
    }
    const keyPrefix = live ? "pdl_live_" : "pdl_sdbx_";
    if (!e.PADDLE_API_KEY.startsWith(keyPrefix)) {
      ctx.addIssue({ code: "custom", path: ["PADDLE_API_KEY"], message: `PADDLE_ENV=${e.PADDLE_ENV} needs a key starting ${keyPrefix}` });
    }
    const tokenPrefix = live ? "live_" : "test_";
    if (!e.NEXT_PUBLIC_PADDLE_CLIENT_TOKEN.startsWith(tokenPrefix)) {
      ctx.addIssue({ code: "custom", path: ["NEXT_PUBLIC_PADDLE_CLIENT_TOKEN"], message: `PADDLE_ENV=${e.PADDLE_ENV} needs a client token starting ${tokenPrefix}` });
    }
    if (e.PADDLE_PRICE_ID_LIFETIME === e.PADDLE_PRICE_ID_YEARLY) {
      ctx.addIssue({ code: "custom", path: ["PADDLE_PRICE_ID_YEARLY"], message: "lifetime and yearly price ids must differ" });
    }
    if (e.SMG_DB === "supabase") {
      if (!e.SUPABASE_URL) ctx.addIssue({ code: "custom", path: ["SUPABASE_URL"], message: "required unless SMG_DB=memory" });
      if (!e.SUPABASE_SERVICE_ROLE_KEY) ctx.addIssue({ code: "custom", path: ["SUPABASE_SERVICE_ROLE_KEY"], message: "required unless SMG_DB=memory" });
    }
    if (live && e.SMG_DB === "memory") {
      ctx.addIssue({ code: "custom", path: ["SMG_DB"], message: "memory database is not allowed with PADDLE_ENV=production" });
    }
  });

export type Env = z.infer<typeof EnvSchema>;

export class EnvError extends Error {
  constructor(public readonly issues: string[]) {
    super(`Environment is not configured: ${issues.join("; ")}`);
    this.name = "EnvError";
  }
}

/** Parse a raw environment. Throws EnvError listing every problem. */
export function parseEnv(source: Record<string, string | undefined>): Env {
  const picked: Record<string, string> = {};
  for (const key of Object.keys(EnvSchema.shape)) {
    const v = source[key];
    if (v !== undefined && v !== "") picked[key] = v;
  }
  const result = EnvSchema.safeParse(picked);
  if (!result.success) {
    const issues = result.error.issues.map((i) => `${i.path.join(".") || "env"}: ${i.message}`);
    throw new EnvError(issues);
  }
  return result.data;
}

let cached: Env | null = null;

/** Validated environment, parsed on first use and cached. */
export function getEnv(): Env {
  if (cached === null) cached = parseEnv(process.env);
  return cached;
}

/** Tests only. */
export function resetEnvCache(): void {
  cached = null;
}

/** 32-byte buffers for the secrets that are stored base64. */
export function secretBytes(env: Env, name: "LICENSE_SIGNING_KEY" | "KEY_ENCRYPTION_KEY" | "OTP_PEPPER" | "IP_HASH_SALT"): Buffer {
  return Buffer.from(env[name], "base64");
}
