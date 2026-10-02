// License tokens (DESIGN.md 5.5): signed with LICENSE_SIGNING_KEY via @cia/api-types' signToken.
// The server also verifies tokens on refresh/deactivate with the public key derived from the
// same seed (plus any extra public keys handed in for rotation).

import { publicKeyFromSeed, signToken, verifyToken, type Kind, type LicensePayload, type LicensePublicKeys, type Plan } from "@cia/api-types";

import { secretBytes, type Env } from "./env";

export const DESKTOP_VALIDITY_S = 45 * 24 * 3600;
export const WEB_VALIDITY_S = 14 * 24 * 3600;

export interface MintInput {
  entitlementId: string;
  plan: Plan;
  kind: Kind;
  deviceHash: string;
  /** Paid-through time for yearly (ISO), null for lifetime. */
  accessUntil: string | null;
  key4: string;
}

export async function mintToken(env: Env, input: MintInput, now: Date): Promise<string> {
  const iat = Math.floor(now.getTime() / 1000);
  const payload: Omit<LicensePayload, "kid"> = {
    v: 1,
    ent: input.entitlementId,
    plan: input.plan,
    kind: input.kind,
    dev: input.deviceHash,
    iat,
    exp: iat + (input.kind === "desktop" ? DESKTOP_VALIDITY_S : WEB_VALIDITY_S),
    acc: input.plan === "yearly" && input.accessUntil ? Math.floor(new Date(input.accessUntil).getTime() / 1000) : null,
    key4: input.key4,
  };
  return signToken(payload, new Uint8Array(secretBytes(env, "LICENSE_SIGNING_KEY")), env.LICENSE_SIGNING_KID);
}

let cachedKeys: { kid: string; seed: string; keys: LicensePublicKeys } | null = null;

async function publicKeys(env: Env): Promise<LicensePublicKeys> {
  if (cachedKeys && cachedKeys.kid === env.LICENSE_SIGNING_KID && cachedKeys.seed === env.LICENSE_SIGNING_KEY) {
    return cachedKeys.keys;
  }
  const pub = await publicKeyFromSeed(new Uint8Array(secretBytes(env, "LICENSE_SIGNING_KEY")));
  const keys: LicensePublicKeys = [[env.LICENSE_SIGNING_KID, pub]];
  cachedKeys = { kid: env.LICENSE_SIGNING_KID, seed: env.LICENSE_SIGNING_KEY, keys };
  return keys;
}

/** Verify a token presented by an app. Only the signature and shape are checked here. */
export async function verifyOwnToken(env: Env, token: string): Promise<LicensePayload | null> {
  const result = await verifyToken(token, await publicKeys(env));
  return result.ok ? result.payload : null;
}
