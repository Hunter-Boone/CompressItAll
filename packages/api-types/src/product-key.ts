// Product keys: `XXXXX-XXXXX-XXXXX-XXXXX` over a 31-character alphabet (DESIGN.md 5.4).
// Twin of crates/cia-license/src/key.rs; test-vectors/product-keys.json keeps them identical.
//
// Checksum: the 20th character is a Luhn mod N check character with N = 31, weights
// 2,1,2,1,... from the right, sum reduced modulo 31. The textbook "digit fold"
// (a / N + a % N) is skipped because it is only a bijection for even N (with 31 it maps
// both 1 and 16 to 2). Reducing mod the prime 31 keeps both weights invertible, so every
// single-character substitution and every adjacent transposition is caught.

/** The 31 allowed characters. No 0, O, 1, I, L. */
export const CHARSET = "ABCDEFGHJKMNPQRSTUVWXYZ23456789";
/** Characters in a normalised key: 19 random + 1 check. */
export const KEY_LEN = 20;
/** Random characters before the check character. */
export const BODY_LEN = 19;
/** Bytes at or above this are rejected when sampling (248 = 8 × 31). */
export const REJECT_FROM = 248;
/** Characters `normalise` drops: ASCII whitespace, no-break space, dashes. Same list as Rust. */
export const IGNORED = new Set([" ", "\t", "\n", "\r", "\u000b", "\u000c", "\u00a0", "-"]);

const N = CHARSET.length;
const CODE = new Map([...CHARSET].map((c, i) => [c, i] as const));

function weightedSum(codes: readonly number[], firstFactor: number): number {
  let factor = firstFactor;
  let sum = 0;
  for (let i = codes.length - 1; i >= 0; i--) {
    sum = (sum + factor * codes[i]!) % N;
    factor = factor === 2 ? 1 : 2;
  }
  return sum;
}

function codesOf(s: string): number[] | null {
  const out: number[] = [];
  for (const ch of s) {
    const c = CODE.get(ch);
    if (c === undefined) return null;
    out.push(c);
  }
  return out;
}

/** Check character for a 19-character body. Throws on a malformed body. */
export function checkChar(body: string): string {
  const codes = codesOf(body);
  if (codes === null || codes.length !== BODY_LEN) {
    throw new Error(`key body must be ${BODY_LEN} charset characters`);
  }
  return CHARSET[(N - weightedSum(codes, 2)) % N]!;
}

/**
 * Generate a new key in display form. `rng` fills a buffer with random bytes;
 * the default is `crypto.getRandomValues`. Bytes >= 248 are rejected so each
 * character is uniform.
 */
export function generate(
  rng: (buf: Uint8Array) => void = (b) => crypto.getRandomValues(b),
): string {
  let body = "";
  const buf = new Uint8Array(32);
  while (body.length < BODY_LEN) {
    rng(buf);
    for (const b of buf) {
      if (body.length === BODY_LEN) break;
      if (b < REJECT_FROM) body += CHARSET[b % N]!;
    }
  }
  return formatDisplay(body + checkChar(body));
}

/**
 * Uppercase (ASCII only), drop IGNORED characters, then check charset, length
 * and checksum. Returns the 20-character normalised key, or null for a typo
 * (the only offline failure; the server decides whether a key exists).
 */
export function normalise(input: string): string | null {
  let out = "";
  for (const ch of input) {
    if (IGNORED.has(ch)) continue;
    const code = ch.charCodeAt(0);
    if (code > 0x7f) return null;
    const up = code >= 0x61 && code <= 0x7a ? String.fromCharCode(code - 0x20) : ch;
    if (!CODE.has(up)) return null;
    out += up;
  }
  return isValid(out) ? out : null;
}

/** True for exactly 20 charset characters with a correct check character. Does not normalise. */
export function isValid(normalised: string): boolean {
  if (normalised.length !== KEY_LEN) return false;
  const codes = codesOf(normalised);
  return codes !== null && codes.length === KEY_LEN && weightedSum(codes, 1) === 0;
}

/** `ABCDE-FGHJK-MNPQR-STUVW` from a normalised key. */
export function formatDisplay(normalised: string): string {
  const groups: string[] = [];
  for (let i = 0; i < normalised.length; i += 5) groups.push(normalised.slice(i, i + 5));
  return groups.join("-");
}

/** `ABCDE-…-STUVW`: first and last group visible. */
export function masked(normalised: string): string {
  if (normalised.length < 10) return "…";
  return `${normalised.slice(0, 5)}-…-${normalised.slice(-5)}`;
}

/** The last five characters, stored as `key4`. */
export function key4(normalised: string): string {
  return normalised.slice(-5);
}

/** `sha256(normalised)`, the database lookup key. */
export async function hash(normalised: string): Promise<Uint8Array> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(normalised));
  return new Uint8Array(digest);
}
