// Shared claim/checkout helpers (DESIGN.md 5.6.1, 5.8.3).

import { createHash, randomBytes, timingSafeEqual } from "node:crypto";

export const CLAIM_TTL_MS = 2 * 3600 * 1000;
export const CHECKOUT_COOKIE = "smg_chk";
export const CHECKOUT_TTL_S = 7200;
/** The success page shows the key for 2 hours after purchase. */
export const KEY_VISIBLE_MS = 2 * 3600 * 1000;

export function newSecret(): string {
  return randomBytes(32).toString("base64url");
}

export function hashSecret(secret: string): string {
  return createHash("sha256").update(secret, "utf8").digest("hex");
}

export function secretMatches(secret: string, hashHex: string): boolean {
  const a = Buffer.from(hashSecret(secret), "hex");
  const b = Buffer.from(hashHex, "hex");
  return a.length === b.length && timingSafeEqual(a, b);
}
