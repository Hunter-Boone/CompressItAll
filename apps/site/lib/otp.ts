// Email sign-in codes (DESIGN.md 5.6.5). The code is never stored: only HMAC(OTP_PEPPER,
// email:code). Attempts are charged atomically by consume_otp_attempt (max 5).

import { createHmac, randomInt, timingSafeEqual } from "node:crypto";

import { secretBytes } from "./env";
import type { Runtime } from "./runtime";

export const OTP_TTL_MS = 10 * 60 * 1000;
export const OTP_RESEND_COOLDOWN_MS = 60 * 1000;
export const OTP_MAX_ATTEMPTS = 5;

export function generateCode(): string {
  return randomInt(0, 1_000_000).toString().padStart(6, "0");
}

export function codeHmac(rt: Runtime, email: string, code: string): string {
  return createHmac("sha256", secretBytes(rt.env, "OTP_PEPPER")).update(`${email}:${code}`, "utf8").digest("hex");
}

/** True if a code was sent to this email within the resend cooldown. */
export async function withinCooldown(rt: Runtime, email: string): Promise<boolean> {
  const row = await rt.db.getOtp(email);
  if (!row) return false;
  return rt.now().getTime() - new Date(row.sent_at).getTime() < OTP_RESEND_COOLDOWN_MS;
}

export async function storeCode(rt: Runtime, email: string, code: string): Promise<void> {
  const now = rt.now();
  await rt.db.upsertOtp({
    email,
    code_hmac: codeHmac(rt, email, code),
    sent_at: now.toISOString(),
    expires_at: new Date(now.getTime() + OTP_TTL_MS).toISOString(),
    attempts: 0,
    used_at: null,
  });
}

export type OtpVerify = "ok" | "invalid";

/**
 * Charge one attempt and compare in constant time. Wrong, expired, missing, used and over-cap
 * all answer "invalid" so the response is not an oracle.
 */
export async function verifyCode(rt: Runtime, email: string, code: string): Promise<OtpVerify> {
  const row = await rt.db.consumeOtpAttempt(email, OTP_MAX_ATTEMPTS);
  if (!row) return "invalid";
  if (new Date(row.expires_at).getTime() <= rt.now().getTime()) return "invalid";
  const expected = Buffer.from(row.code_hmac, "hex");
  const given = Buffer.from(codeHmac(rt, email, code), "hex");
  if (expected.length !== given.length || !timingSafeEqual(expected, given)) return "invalid";
  await rt.db.markOtpUsed(email, rt.now().toISOString());
  return "ok";
}
