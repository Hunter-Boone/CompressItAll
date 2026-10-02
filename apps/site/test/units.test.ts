import { createHmac } from "node:crypto";

import { describe, expect, it } from "vitest";

import { productKey } from "@cia/api-types";

import { decryptKey, encryptKey, generateStoredKey, hashKey, maskedKey } from "../lib/keys";
import { newCheckoutId, newClaimId, newEntitlementId, ulid } from "../lib/ids";
import { MAX_AGE_S, MAX_FUTURE_SKEW_S, computeSignature, parseSignatureHeader, signForTest, verifyPaddleSignature } from "../lib/paddle-signature";
import { ipKey } from "../lib/rate-limit";
import { ClaimIdSchema, CheckoutIdSchema, EntitlementIdSchema } from "@cia/api-types";

describe("keys", () => {
  const kek = Buffer.alloc(32, 5);
  it("generates a valid key and round-trips the ciphertext", () => {
    const k = generateStoredKey(kek);
    expect(productKey.normalise(k.display)).toBe(k.normalised);
    expect(k.hash).toBe(hashKey(k.normalised));
    expect(k.hash).toHaveLength(64);
    expect(decryptKey(k.enc, kek)).toBe(k.normalised);
    expect(k.enc.startsWith("v1.")).toBe(true);
    expect(k.key4).toBe(k.normalised.slice(-5));
    expect(maskedKey(k.key4)).toBe(`XXXXX-XXXXX-XXXXX-${k.key4}`);
  });
  it("uses a fresh IV per encryption and rejects a wrong key", () => {
    expect(encryptKey("ABCDEFGHJKMNPQRSTUVW", kek)).not.toBe(encryptKey("ABCDEFGHJKMNPQRSTUVW", kek));
    expect(() => decryptKey(encryptKey("ABCDEFGHJKMNPQRSTUVW", kek), Buffer.alloc(32, 6))).toThrow();
  });
});

describe("ids", () => {
  it("produce ULIDs the api-types schemas accept", () => {
    expect(ulid()).toMatch(/^[0-9A-HJKMNP-TV-Z]{26}$/);
    expect(EntitlementIdSchema.safeParse(newEntitlementId()).success).toBe(true);
    expect(ClaimIdSchema.safeParse(newClaimId()).success).toBe(true);
    expect(CheckoutIdSchema.safeParse(newCheckoutId()).success).toBe(true);
    const a = ulid(1000);
    const b = ulid(2000);
    expect(a.slice(0, 10) < b.slice(0, 10)).toBe(true);
  });
});

describe("paddle signature", () => {
  const secret = "pdl_ntfset_test";
  const body = '{"event_id":"evt_1"}';
  it("parses multiple h1 values and ignores unknown keys", () => {
    const p = parseSignatureHeader(`ts=100;h1=${"a".repeat(64)};h2=zz;h1=${"b".repeat(64)}`);
    expect(p).toEqual({ ts: 100, h1: ["a".repeat(64), "b".repeat(64)] });
    expect(parseSignatureHeader("ts=abc;h1=x")).toBeNull();
    expect(parseSignatureHeader("h1=" + "a".repeat(64))).toBeNull();
    expect(parseSignatureHeader("")).toBeNull();
  });
  it("verifies within the window and rejects stale, future, mismatch, missing", () => {
    const now = 1_700_000_000;
    expect(verifyPaddleSignature(signForTest(secret, body, now), body, secret, now)).toBe("ok");
    expect(verifyPaddleSignature(signForTest(secret, body, now - MAX_AGE_S), body, secret, now)).toBe("ok");
    expect(verifyPaddleSignature(signForTest(secret, body, now - MAX_AGE_S - 1), body, secret, now)).toBe("stale");
    expect(verifyPaddleSignature(signForTest(secret, body, now + MAX_FUTURE_SKEW_S), body, secret, now)).toBe("ok");
    expect(verifyPaddleSignature(signForTest(secret, body, now + MAX_FUTURE_SKEW_S + 1), body, secret, now)).toBe("future");
    expect(verifyPaddleSignature(signForTest("other", body, now), body, secret, now)).toBe("mismatch");
    expect(verifyPaddleSignature(null, body, secret, now)).toBe("missing");
    expect(verifyPaddleSignature("garbage", body, secret, now)).toBe("malformed");
  });
  it("matches Paddle's documented construction", () => {
    expect(computeSignature(secret, 1, "x")).toBe(createHmac("sha256", secret).update("1:x").digest("hex"));
  });
});

describe("ipKey", () => {
  it("never contains the IP and changes with the salt", () => {
    const a = ipKey(Buffer.alloc(16, 1), "203.0.113.10");
    const b = ipKey(Buffer.alloc(16, 2), "203.0.113.10");
    expect(a).not.toContain("203");
    expect(a).not.toBe(b);
    expect(a).toHaveLength(32);
  });
});
