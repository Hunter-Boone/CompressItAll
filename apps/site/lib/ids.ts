// ULID (Crockford base32, 26 chars: 48-bit ms timestamp + 80 random bits) and the prefixed ids
// used in the database. The alphabet matches the regexes in @cia/api-types.

import { randomBytes, randomUUID } from "node:crypto";

const ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

export function ulid(now: number = Date.now()): string {
  let time = "";
  let t = now;
  for (let i = 0; i < 10; i++) {
    time = ALPHABET[t % 32]! + time;
    t = Math.floor(t / 32);
  }
  const rnd = randomBytes(16);
  let rand = "";
  for (let i = 0; i < 16; i++) rand += ALPHABET[rnd[i]! % 32]!;
  return time + rand;
}

export const newEntitlementId = (now?: number) => `ent_${ulid(now)}`;
export const newClaimId = (now?: number) => `clm_${ulid(now)}`;
export const newCheckoutId = (now?: number) => `chk_${ulid(now)}`;
export const newUuid = () => randomUUID();
