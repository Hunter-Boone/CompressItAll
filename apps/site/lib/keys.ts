// Product keys at rest (DESIGN.md 5.4): generate with @cia/api-types, hash with sha256 for
// lookup, AES-256-GCM encrypt with KEY_ENCRYPTION_KEY so the key can be emailed again, and keep
// the last five characters for display. The full key is never logged.

import { createCipheriv, createDecipheriv, createHash, randomBytes } from "node:crypto";

import { productKey } from "@cia/api-types";

export interface StoredKey {
  /** Normalised 20-character key (never store this). */
  normalised: string;
  /** Display form `XXXXX-XXXXX-XXXXX-XXXXX`. */
  display: string;
  /** sha256(normalised) as lowercase hex. */
  hash: string;
  /** `v1.<iv_b64>.<ciphertext_b64>.<tag_b64>` */
  enc: string;
  key4: string;
}

export function generateStoredKey(kek: Buffer): StoredKey {
  const display = productKey.generate();
  const normalised = productKey.normalise(display);
  if (normalised === null) throw new Error("generated key failed its own checksum");
  return {
    normalised,
    display,
    hash: hashKey(normalised),
    enc: encryptKey(normalised, kek),
    key4: productKey.key4(normalised),
  };
}

export function hashKey(normalised: string): string {
  return createHash("sha256").update(normalised, "utf8").digest("hex");
}

export function encryptKey(normalised: string, kek: Buffer): string {
  if (kek.length !== 32) throw new Error("KEY_ENCRYPTION_KEY must be 32 bytes");
  const iv = randomBytes(12);
  const cipher = createCipheriv("aes-256-gcm", kek, iv);
  const ct = Buffer.concat([cipher.update(normalised, "utf8"), cipher.final()]);
  const tag = cipher.getAuthTag();
  return `v1.${iv.toString("base64url")}.${ct.toString("base64url")}.${tag.toString("base64url")}`;
}

export function decryptKey(enc: string, kek: Buffer): string {
  const [v, ivB64, ctB64, tagB64] = enc.split(".");
  if (v !== "v1" || !ivB64 || !ctB64 || !tagB64) throw new Error("unknown key ciphertext format");
  const decipher = createDecipheriv("aes-256-gcm", kek, Buffer.from(ivB64, "base64url"));
  decipher.setAuthTag(Buffer.from(tagB64, "base64url"));
  return Buffer.concat([decipher.update(Buffer.from(ctB64, "base64url")), decipher.final()]).toString("utf8");
}

/** `XXXXX-XXXXX-XXXXX-7KQ2P`, the dashboard's masked form. */
export function maskedKey(key4: string): string {
  return `XXXXX-XXXXX-XXXXX-${key4}`;
}
