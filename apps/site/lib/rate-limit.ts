// Fixed-window rate limits over rate_limit_hit (DESIGN.md 5.10). Fails CLOSED: if the limiter
// errors, the caller answers 503 (ConvertSave's version answered "allowed").

import { createHash } from "node:crypto";

import type { Db } from "./db";
import { ApiError } from "./http";

export class RateLimited extends ApiError {
  constructor(message = "Too many requests. Try again later.") {
    super(429, "rate_limited", message);
  }
}

/** Keys never contain the raw IP: sha256(IP_HASH_SALT + ip). */
export function ipKey(salt: Buffer, ip: string): string {
  return createHash("sha256").update(Buffer.concat([salt, Buffer.from(ip, "utf8")])).digest("hex").slice(0, 32);
}

export interface Limit {
  bucket: string;
  id: string;
  max: number;
  windowSeconds: number;
}

/** Charge every limit; throw RateLimited on the first that is over. */
export async function enforceLimits(db: Db, limits: Limit[]): Promise<void> {
  for (const l of limits) {
    let ok: boolean;
    try {
      ok = await db.rateLimitHit(`${l.bucket}:${l.id}`, l.windowSeconds, l.max);
    } catch (err) {
      throw new ApiError(503, "unavailable", "Smidge can't check that right now. Try again in a minute.", { cause: err });
    }
    if (!ok) throw new RateLimited();
  }
}

export const HOUR = 3600;
export const DAY = 86400;
