// Paddle webhook signature verification, written by hand (DESIGN.md 5.8.1). The SDK keeps only
// the last h1 (breaks during secret rotation), compares with ===, and accepts future timestamps.
//
// Header: `Paddle-Signature: ts=<unix seconds>;h1=<hex>[;h1=<hex>...]`
// Signed payload: `${ts}:${rawBody}` with HMAC-SHA256 over the notification destination secret.

import { createHmac, timingSafeEqual } from "node:crypto";

export const MAX_AGE_S = 30;
export const MAX_FUTURE_SKEW_S = 5;

export interface ParsedSignature {
  ts: number;
  h1: string[];
}

export function parseSignatureHeader(header: string | null): ParsedSignature | null {
  if (!header) return null;
  let ts: number | null = null;
  const h1: string[] = [];
  for (const part of header.split(";")) {
    const i = part.indexOf("=");
    if (i < 0) return null;
    const k = part.slice(0, i).trim();
    const v = part.slice(i + 1).trim();
    if (k === "ts") {
      if (!/^\d{1,12}$/.test(v)) return null;
      ts = Number(v);
    } else if (k === "h1") {
      if (!/^[0-9a-f]{64}$/i.test(v)) return null;
      h1.push(v.toLowerCase());
    }
    // Unknown keys (future schemes) are ignored.
  }
  if (ts === null || h1.length === 0) return null;
  return { ts, h1 };
}

export function computeSignature(secret: string, ts: number, rawBody: string): string {
  return createHmac("sha256", secret).update(`${ts}:${rawBody}`, "utf8").digest("hex");
}

export type SignatureResult = "ok" | "missing" | "malformed" | "stale" | "future" | "mismatch";

/**
 * Verify a header against the raw body. Any `h1` may match (secret rotation sends two).
 * `nowS` is unix seconds.
 */
export function verifyPaddleSignature(header: string | null, rawBody: string, secret: string, nowS: number): SignatureResult {
  if (header === null) return "missing";
  const parsed = parseSignatureHeader(header);
  if (parsed === null) return "malformed";
  if (parsed.ts > nowS + MAX_FUTURE_SKEW_S) return "future";
  if (nowS - parsed.ts > MAX_AGE_S) return "stale";
  const expected = Buffer.from(computeSignature(secret, parsed.ts, rawBody), "hex");
  for (const h of parsed.h1) {
    const given = Buffer.from(h, "hex");
    if (given.length === expected.length && timingSafeEqual(given, expected)) return "ok";
  }
  return "mismatch";
}

/** For tests and tooling: build a header the verifier accepts. */
export function signForTest(secret: string, rawBody: string, ts: number, extraH1: string[] = []): string {
  const parts = [`ts=${ts}`, ...extraH1.map((h) => `h1=${h}`), `h1=${computeSignature(secret, ts, rawBody)}`];
  return parts.join(";");
}
