// License tokens: `SMG1.<payload_b64url>.<signature_b64url>` (DESIGN.md 5.5).
// Twin of crates/cia-license/src/token.rs; test-vectors/license-tokens.json keeps them identical.
//
// The signature is Ed25519 over the ASCII bytes of "SMG1." + payload_b64url, so verifiers
// never re-serialise the payload. Base64 is URL-safe without padding.
//
// Verification uses WebCrypto Ed25519 (`crypto.subtle`), available in Node 20+, Chrome 113+,
// Firefox 130+ and Safari 17+. Safari 16 and older reject `importKey("raw", …, "Ed25519")`
// with NotSupportedError; when that happens (or subtle is absent) we fall back to
// @noble/ed25519 (MIT, pure JS).

import * as ed from "@noble/ed25519";
import { z } from "zod";

import { asciiBytes, base64urlDecode, base64urlEncode } from "./bytes.js";

export const TOKEN_PREFIX = "SMG1";
export const TOKEN_VERSION = 1;

export const PlanSchema = z.enum(["lifetime", "yearly"]);
export type Plan = z.infer<typeof PlanSchema>;

export const KindSchema = z.enum(["desktop", "web"]);
export type Kind = z.infer<typeof KindSchema>;

/** Token payload. Key order here is the wire order; `encodePayload` relies on it. */
export const LicensePayloadSchema = z.object({
  v: z.literal(1),
  /** Which public key verifies the token. */
  kid: z.string().min(1),
  /** Entitlement id (`ent_` + ULID). */
  ent: z.string().min(1),
  plan: PlanSchema,
  kind: KindSchema,
  /** Desktop device hash (`d_…`) or web install id (`w_…`). */
  dev: z.string().min(1),
  /** Issued at, unix seconds. */
  iat: z.number().int(),
  /** Offline validity end, unix seconds: iat + 45 days (desktop), + 14 days (web). */
  exp: z.number().int(),
  /** Paid-through time for yearly plans; null for lifetime. */
  acc: z.number().int().nullable(),
  /** Last five characters of the product key. */
  key4: z.string().length(5),
});
export type LicensePayload = z.infer<typeof LicensePayloadSchema>;

export type TokenErrorCode = "malformed" | "unsupported_version" | "unknown_kid" | "bad_signature";

export type VerifyResult =
  { ok: true; payload: LicensePayload } | { ok: false; error: TokenErrorCode };

/** `(kid, 32-byte Ed25519 public key)` pairs; first match wins. */
export type LicensePublicKeys = ReadonlyArray<readonly [kid: string, key: Uint8Array]>;

export interface VerifyOptions {
  /**
   * SubtleCrypto to use. Defaults to `globalThis.crypto.subtle`. Pass `null` to
   * force the @noble/ed25519 fallback (tests do this so both paths run).
   */
  subtle?: SubtleCrypto | null;
}

/** Canonical payload JSON (wire key order) as base64url. */
export function encodePayload(payload: LicensePayload): string {
  const ordered = {
    v: payload.v,
    kid: payload.kid,
    ent: payload.ent,
    plan: payload.plan,
    kind: payload.kind,
    dev: payload.dev,
    iat: payload.iat,
    exp: payload.exp,
    acc: payload.acc,
    key4: payload.key4,
  };
  return base64urlEncode(new TextEncoder().encode(JSON.stringify(ordered)));
}

/** The bytes that are signed: ASCII of "SMG1." + payload_b64url. */
export function signingInput(payloadB64: string): Uint8Array {
  return asciiBytes(`${TOKEN_PREFIX}.${payloadB64}`);
}

/**
 * Decode and check a token against `publicKeys`. Returns the payload; the caller
 * still applies the device, clock and expiry rules (the app does that in Rust).
 */
export async function verifyToken(
  token: string,
  publicKeys: LicensePublicKeys,
  options: VerifyOptions = {},
): Promise<VerifyResult> {
  const parts = token.split(".");
  if (parts.length !== 3 || parts[0] !== TOKEN_PREFIX) return { ok: false, error: "malformed" };
  const payloadB64 = parts[1]!;
  const payloadBytes = base64urlDecode(payloadB64);
  const signature = base64urlDecode(parts[2]!);
  if (payloadBytes === null || signature === null || signature.length !== 64) {
    return { ok: false, error: "malformed" };
  }

  let raw: unknown;
  try {
    raw = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(payloadBytes));
  } catch {
    return { ok: false, error: "malformed" };
  }
  if (typeof raw !== "object" || raw === null) return { ok: false, error: "malformed" };
  const v = (raw as { v?: unknown }).v;
  if (typeof v !== "number" || !Number.isInteger(v)) return { ok: false, error: "malformed" };
  if (v !== TOKEN_VERSION) return { ok: false, error: "unsupported_version" };
  const parsed = LicensePayloadSchema.safeParse(raw);
  if (!parsed.success) return { ok: false, error: "malformed" };
  const payload = parsed.data;

  const entry = publicKeys.find(([kid]) => kid === payload.kid);
  if (!entry) return { ok: false, error: "unknown_kid" };
  const publicKey = entry[1];
  if (publicKey.length !== 32) return { ok: false, error: "bad_signature" };

  const subtle =
    options.subtle === undefined ? (globalThis.crypto?.subtle ?? null) : options.subtle;
  const valid = await verifyEd25519(publicKey, signingInput(payloadB64), signature, subtle);
  return valid ? { ok: true, payload } : { ok: false, error: "bad_signature" };
}

async function verifyEd25519(
  publicKey: Uint8Array,
  message: Uint8Array,
  signature: Uint8Array,
  subtle: SubtleCrypto | null,
): Promise<boolean> {
  if (subtle !== null) {
    // WebCrypto wants views over a plain ArrayBuffer; copying 32-64 bytes is free.
    const pub = new Uint8Array(publicKey);
    const sig = new Uint8Array(signature);
    const msg = new Uint8Array(message);
    let key: CryptoKey | null = null;
    try {
      key = await subtle.importKey("raw", pub, { name: "Ed25519" }, false, ["verify"]);
    } catch {
      // Safari < 17 (and any runtime without Ed25519 in WebCrypto) rejects here. Fall back.
      key = null;
    }
    if (key !== null) {
      try {
        return await subtle.verify({ name: "Ed25519" }, key, sig, msg);
      } catch {
        return false;
      }
    }
  }
  try {
    // zip215: false matches ed25519-dalek's verify_strict more closely.
    return await ed.verifyAsync(signature, message, publicKey, { zip215: false });
  } catch {
    return false;
  }
}

/**
 * Sign a payload with a 32-byte Ed25519 seed, stamping `kid` into the payload.
 * Server-side only (`LICENSE_SIGNING_KEY`, `LICENSE_SIGNING_KID`). Deterministic:
 * the same payload and seed always give the same token, byte for byte, as Rust's
 * `cia_license::token::sign`.
 */
export async function signToken(
  payload: Omit<LicensePayload, "kid"> & { kid?: string },
  seed: Uint8Array,
  kid: string,
): Promise<string> {
  if (seed.length !== 32) throw new Error("Ed25519 seed must be 32 bytes");
  const full = LicensePayloadSchema.parse({ ...payload, kid });
  const payloadB64 = encodePayload(full);
  const message = signingInput(payloadB64);
  const signature = await ed.signAsync(message, seed);
  return `${TOKEN_PREFIX}.${payloadB64}.${base64urlEncode(signature)}`;
}

/** Public key for a seed, for tests and tooling. */
export async function publicKeyFromSeed(seed: Uint8Array): Promise<Uint8Array> {
  return ed.getPublicKeyAsync(seed);
}
