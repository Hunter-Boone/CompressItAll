import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import { bytesToHex } from "../src/bytes.js";
import {
  BODY_LEN,
  CHARSET,
  KEY_LEN,
  checkChar,
  formatDisplay,
  generate,
  hash,
  isValid,
  key4,
  masked,
  normalise,
} from "../src/product-key.js";

interface Vectors {
  charset: string;
  valid: { display: string; normalised: string; masked: string; key4: string; sha256: string }[];
  invalid: { display: string; from: string; position: number }[];
  normalise: ({ input: string; normalised: string } | { input: string; error: "typo" })[];
}

const vectors: Vectors = JSON.parse(
  readFileSync(new URL("../test-vectors/product-keys.json", import.meta.url), "utf8"),
);

describe("shared vectors (product-keys.json)", () => {
  it("uses the same charset as Rust", () => {
    expect(vectors.charset).toBe(CHARSET);
    expect(CHARSET).toHaveLength(31);
    for (const c of "0OI1L") expect(CHARSET.includes(c)).toBe(false);
  });

  it("accepts all 50 valid keys and reproduces display/masked/key4/sha256", async () => {
    expect(vectors.valid).toHaveLength(50);
    for (const k of vectors.valid) {
      expect(normalise(k.display)).toBe(k.normalised);
      expect(isValid(k.normalised)).toBe(true);
      expect(formatDisplay(k.normalised)).toBe(k.display);
      expect(masked(k.normalised)).toBe(k.masked);
      expect(key4(k.normalised)).toBe(k.key4);
      expect(bytesToHex(await hash(k.normalised))).toBe(k.sha256);
      expect(checkChar(k.normalised.slice(0, BODY_LEN))).toBe(k.normalised[BODY_LEN]);
    }
  });

  it("rejects all 50 keys with one changed character", () => {
    expect(vectors.invalid).toHaveLength(50);
    for (const k of vectors.invalid) {
      expect(normalise(k.display), k.display).toBeNull();
      expect(isValid(k.display.replaceAll("-", ""))).toBe(false);
    }
  });

  it("normalises the 20 raw inputs the same way as Rust", () => {
    expect(vectors.normalise).toHaveLength(20);
    for (const c of vectors.normalise) {
      if ("normalised" in c) expect(normalise(c.input), JSON.stringify(c.input)).toBe(c.normalised);
      else expect(normalise(c.input), JSON.stringify(c.input)).toBeNull();
    }
  });
});

describe("checksum", () => {
  it("catches every single-character substitution in the vector keys", () => {
    let checked = 0;
    for (const { normalised } of vectors.valid) {
      for (let pos = 0; pos < KEY_LEN; pos++) {
        for (const alt of CHARSET) {
          if (alt === normalised[pos]) continue;
          const mutated = normalised.slice(0, pos) + alt + normalised.slice(pos + 1);
          expect(isValid(mutated), mutated).toBe(false);
          checked++;
        }
      }
    }
    expect(checked).toBe(50 * KEY_LEN * 30);
  });

  it("catches adjacent transpositions", () => {
    for (const { normalised } of vectors.valid) {
      for (let pos = 0; pos < KEY_LEN - 1; pos++) {
        if (normalised[pos] === normalised[pos + 1]) continue;
        const chars = [...normalised];
        [chars[pos], chars[pos + 1]] = [chars[pos + 1]!, chars[pos]!];
        expect(isValid(chars.join("")), `${normalised} swap ${pos}`).toBe(false);
      }
    }
  });
});

describe("generate", () => {
  it("produces valid display-form keys", () => {
    for (let i = 0; i < 200; i++) {
      const display = generate();
      expect(display).toMatch(/^[A-Z2-9]{5}-[A-Z2-9]{5}-[A-Z2-9]{5}-[A-Z2-9]{5}$/);
      const n = normalise(display);
      expect(n).not.toBeNull();
      expect(formatDisplay(n!)).toBe(display);
    }
  });

  it("rejects bytes >= 248 instead of folding them", () => {
    let toggle = false;
    const display = generate((buf) => {
      for (let i = 0; i < buf.length; i++) {
        buf[i] = toggle ? 255 : 0;
        toggle = !toggle;
      }
    });
    expect(normalise(display)!.slice(0, BODY_LEN)).toBe("A".repeat(BODY_LEN));
  });

  it("spreads characters evenly (20,000 keys, 5 percent tolerance)", () => {
    const counts = new Map<string, number>();
    const keys = 20_000;
    for (let i = 0; i < keys; i++) {
      const n = normalise(generate())!;
      for (const c of n.slice(0, BODY_LEN)) counts.set(c, (counts.get(c) ?? 0) + 1);
    }
    const expected = (keys * BODY_LEN) / CHARSET.length;
    for (const c of CHARSET) {
      const observed = counts.get(c) ?? 0;
      expect(Math.abs(observed - expected) / expected, c).toBeLessThanOrEqual(0.05);
    }
  });
});

describe("normalise edge cases", () => {
  const good = vectors.valid[0]!.normalised;
  it("handles whitespace, case and dashes", () => {
    expect(normalise(`  ${formatDisplay(good).toLowerCase()}\n`)).toBe(good);
    expect(normalise(formatDisplay(good).replaceAll("-", " "))).toBe(good);
  });
  it("treats confusables, wrong length and non-ASCII as typos", () => {
    for (const bad of ["0", "O", "1", "I", "L", "o", "ß", "É"]) {
      expect(normalise(bad + good.slice(1))).toBeNull();
    }
    expect(normalise(good.slice(0, 19))).toBeNull();
    expect(normalise(good + "A")).toBeNull();
    expect(normalise("")).toBeNull();
    expect(masked("ABCDEFGHJKMNPQRSTUVW")).toBe("ABCDE-…-STUVW");
  });
});
