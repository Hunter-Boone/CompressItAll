// Small byte helpers shared by product-key.ts and license-token.ts. No deps.

const B64URL = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const B64URL_INDEX = new Map([...B64URL].map((c, i) => [c, i] as const));

/** Base64url without padding (RFC 4648 §5), matching Rust's `URL_SAFE_NO_PAD`. */
export function base64urlEncode(bytes: Uint8Array): string {
  let out = "";
  let i = 0;
  for (; i + 2 < bytes.length; i += 3) {
    const n = (bytes[i]! << 16) | (bytes[i + 1]! << 8) | bytes[i + 2]!;
    out +=
      B64URL[(n >> 18) & 63]! + B64URL[(n >> 12) & 63]! + B64URL[(n >> 6) & 63]! + B64URL[n & 63]!;
  }
  const rest = bytes.length - i;
  if (rest === 1) {
    const n = bytes[i]! << 16;
    out += B64URL[(n >> 18) & 63]! + B64URL[(n >> 12) & 63]!;
  } else if (rest === 2) {
    const n = (bytes[i]! << 16) | (bytes[i + 1]! << 8);
    out += B64URL[(n >> 18) & 63]! + B64URL[(n >> 12) & 63]! + B64URL[(n >> 6) & 63]!;
  }
  return out;
}

/**
 * Strict base64url decode: no padding, no foreign characters, no non-zero
 * trailing bits. Returns null on any violation (the Rust side rejects the same
 * inputs, so a token that is "malformed" there is malformed here).
 */
export function base64urlDecode(text: string): Uint8Array | null {
  const rem = text.length % 4;
  if (rem === 1) return null;
  const outLen = Math.floor((text.length * 6) / 8);
  const out = new Uint8Array(outLen);
  let bits = 0;
  let acc = 0;
  let o = 0;
  for (const ch of text) {
    const v = B64URL_INDEX.get(ch);
    if (v === undefined) return null;
    acc = (acc << 6) | v;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out[o++] = (acc >> bits) & 0xff;
    }
  }
  // Leftover bits must be zero (canonical encoding).
  if (bits > 0 && (acc & ((1 << bits) - 1)) !== 0) return null;
  return out;
}

export function bytesToHex(bytes: Uint8Array): string {
  return [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");
}

export function hexToBytes(hex: string): Uint8Array {
  if (hex.length % 2 !== 0 || !/^[0-9a-fA-F]*$/.test(hex)) {
    throw new Error("invalid hex");
  }
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

export function asciiBytes(text: string): Uint8Array {
  return new TextEncoder().encode(text);
}
