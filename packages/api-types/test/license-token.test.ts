import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import { base64urlDecode, base64urlEncode, hexToBytes } from "../src/bytes.js";
import {
  LicensePayloadSchema,
  encodePayload,
  publicKeyFromSeed,
  signToken,
  verifyToken,
  type LicensePayload,
  type LicensePublicKeys,
  type TokenErrorCode,
} from "../src/license-token.js";

interface Vectors {
  kid: string;
  public_key_hex: string;
  tokens: { name: string; payload: LicensePayload; token: string }[];
  cases: {
    name: string;
    token: string;
    device_hash: string;
    now: number;
    last_seen_utc?: number;
    verify: "ok" | TokenErrorCode;
    status: Record<string, unknown>;
  }[];
}

const vectors: Vectors = JSON.parse(
  readFileSync(new URL("../test-vectors/license-tokens.json", import.meta.url), "utf8"),
);
const testKeys: { keys: { kid: string; seed_b64: string; public_key_hex: string }[] } = JSON.parse(
  readFileSync(new URL("../../../crates/cia-license/test-keys.json", import.meta.url), "utf8"),
);

const keys: LicensePublicKeys = [[vectors.kid, hexToBytes(vectors.public_key_hex)]];
const testSeed = Uint8Array.from(
  Buffer.from(testKeys.keys.find((k) => k.kid === "test")!.seed_b64, "base64"),
);

const paths = [
  { name: "WebCrypto", opts: {} },
  { name: "@noble/ed25519 fallback", opts: { subtle: null } },
] as const;

describe("test key", () => {
  it("matches the seed in crates/cia-license/test-keys.json", async () => {
    const pub = await publicKeyFromSeed(testSeed);
    expect(Buffer.from(pub).toString("hex")).toBe(vectors.public_key_hex);
  });
});

for (const path of paths) {
  describe(`verifyToken via ${path.name}`, () => {
    it("decodes the 5 signed tokens to their payloads", async () => {
      expect(vectors.tokens).toHaveLength(5);
      for (const t of vectors.tokens) {
        const r = await verifyToken(t.token, keys, path.opts);
        if (r.ok) expect(r.payload, t.name).toEqual(t.payload);
        else expect(r.error, t.name).toBe("unknown_kid");
      }
    });

    it("agrees with every shared case", async () => {
      expect(vectors.cases.length).toBeGreaterThanOrEqual(9);
      for (const c of vectors.cases) {
        const r = await verifyToken(c.token, keys, path.opts);
        expect(r.ok ? "ok" : r.error, c.name).toBe(c.verify);
      }
    });

    it("rejects a token signed under an unknown kid and accepts it once the kid is known", async () => {
      const wrong = vectors.cases.find((c) => c.name === "wrong kid")!;
      expect(await verifyToken(wrong.token, keys, path.opts)).toEqual({
        ok: false,
        error: "unknown_kid",
      });
      const widened: LicensePublicKeys = [...keys, ["2099-01", hexToBytes(vectors.public_key_hex)]];
      expect((await verifyToken(wrong.token, widened, path.opts)).ok).toBe(true);
    });

    it("rejects shape errors as malformed", async () => {
      for (const t of [
        "",
        "SMG1",
        "SMG1.abc",
        "SMG1.a.b.c",
        "SMG2.e30.AAAA",
        "SMG1.e30=.AAAA",
        "SMG1.!!.AAAA",
      ]) {
        expect(await verifyToken(t, keys, path.opts), JSON.stringify(t)).toEqual({
          ok: false,
          error: "malformed",
        });
      }
    });

    it("reports unsupported_version for v != 1", async () => {
      const good = vectors.tokens[0]!;
      const b64 = base64urlEncode(
        new TextEncoder().encode(JSON.stringify({ ...good.payload, v: 2 })),
      );
      const token = `SMG1.${b64}.${good.token.split(".")[2]}`;
      expect(await verifyToken(token, keys, path.opts)).toEqual({
        ok: false,
        error: "unsupported_version",
      });
    });
  });
}

describe("signToken", () => {
  it("reproduces the Rust-signed vector tokens byte for byte", async () => {
    for (const t of vectors.tokens) {
      const { kid, ...rest } = t.payload;
      expect(await signToken(rest, testSeed, kid), t.name).toBe(t.token);
    }
  });

  it("encodes the payload in wire order", () => {
    const p = vectors.tokens[0]!.payload;
    const json = new TextDecoder().decode(base64urlDecode(encodePayload(p))!);
    expect(Object.keys(JSON.parse(json))).toEqual([
      "v",
      "kid",
      "ent",
      "plan",
      "kind",
      "dev",
      "iat",
      "exp",
      "acc",
      "key4",
    ]);
    const fromVector = new TextDecoder().decode(
      base64urlDecode(vectors.tokens[0]!.token.split(".")[1]!)!,
    );
    expect(json).toBe(fromVector);
  });

  it("round-trips through verifyToken on both paths", async () => {
    const payload = LicensePayloadSchema.parse({
      ...vectors.tokens[1]!.payload,
      ent: "ent_01K6QWZZZZZZZZZZZZZZZZZZZZ",
    });
    const token = await signToken(payload, testSeed, "test");
    for (const path of paths) {
      const r = await verifyToken(token, keys, path.opts);
      expect(r.ok && r.payload).toEqual(payload);
    }
  });
});

describe("base64url", () => {
  it("is strict about padding, alphabet and trailing bits", () => {
    expect(base64urlDecode("")).toEqual(new Uint8Array());
    expect(Array.from(base64urlDecode("AQID")!)).toEqual([1, 2, 3]);
    expect(base64urlDecode("AQID=")).toBeNull();
    expect(base64urlDecode("A")).toBeNull();
    expect(base64urlDecode("AQ+D")).toBeNull();
    expect(base64urlDecode("AR")).toBeNull(); // 'R' leaves non-zero trailing bits
    expect(Array.from(base64urlDecode("AQ")!)).toEqual([1]);
    const bytes = Uint8Array.from({ length: 100 }, (_, i) => (i * 37) % 256);
    expect(Array.from(base64urlDecode(base64urlEncode(bytes))!)).toEqual(Array.from(bytes));
    expect(base64urlEncode(bytes)).toBe(Buffer.from(bytes).toString("base64url"));
  });
});
