// Updater channel logic (DESIGN.md 6.3): read channels/<channel>.json from the public Downloads
// repo (cached 60 s), 204 when current, always update blocked versions, otherwise gate by
// sha256(install_id) mod 100 < rollout_percent.

import { createHash } from "node:crypto";

import { z } from "zod";

import { downloadsRawBase } from "./brand";

export const ChannelFileSchema = z.object({
  version: z.string().min(1),
  pub_date: z.string().optional(),
  notes: z.string().optional(),
  rollout_percent: z.number().int().min(0).max(100).default(100),
  blocked_versions: z.array(z.string()).default([]),
  platforms: z.record(z.string(), z.object({ url: z.string().url(), signature: z.string().min(1) })),
});
export type ChannelFile = z.infer<typeof ChannelFileSchema>;

export const CHANNEL_CACHE_MS = 60_000;

const cache = new Map<string, { at: number; file: ChannelFile | null }>();

export function channelUrl(channel: string): string {
  return `${downloadsRawBase()}/channels/${channel}.json`;
}

/** Fetch and cache a channel file. Returns null when it does not exist or fails to parse. */
export async function fetchChannel(fetchFn: typeof fetch, channel: string, now = Date.now()): Promise<ChannelFile | null> {
  const hit = cache.get(channel);
  if (hit && now - hit.at < CHANNEL_CACHE_MS) return hit.file;
  let file: ChannelFile | null = null;
  try {
    const res = await fetchFn(channelUrl(channel), { headers: { Accept: "application/json" }, cache: "no-store" });
    if (res.ok) {
      const parsed = ChannelFileSchema.safeParse(await res.json());
      file = parsed.success ? parsed.data : null;
    }
  } catch {
    file = null;
  }
  cache.set(channel, { at: now, file });
  return file;
}

export function clearChannelCache(): void {
  cache.clear();
}

/** Semver-ish compare: numeric dotted core, then a pre-release tag sorts before none. */
export function compareVersions(a: string, b: string): number {
  const pa = split(a);
  const pb = split(b);
  for (let i = 0; i < 3; i++) {
    const d = (pa.core[i] ?? 0) - (pb.core[i] ?? 0);
    if (d !== 0) return d < 0 ? -1 : 1;
  }
  if (pa.pre === null && pb.pre === null) return 0;
  if (pa.pre === null) return 1;
  if (pb.pre === null) return -1;
  const xa = pa.pre.split(".");
  const xb = pb.pre.split(".");
  for (let i = 0; i < Math.max(xa.length, xb.length); i++) {
    const sa = xa[i];
    const sb = xb[i];
    if (sa === undefined) return -1;
    if (sb === undefined) return 1;
    const na = Number(sa);
    const nb = Number(sb);
    const bothNumeric = !Number.isNaN(na) && !Number.isNaN(nb);
    const c = bothNumeric ? na - nb : sa.localeCompare(sb);
    if (c !== 0) return c < 0 ? -1 : 1;
  }
  return 0;
}

function split(v: string): { core: number[]; pre: string | null } {
  const clean = v.trim().replace(/^v/, "");
  const [corePart, ...preParts] = clean.split("-");
  const core = (corePart ?? "").split(".").map((n) => Number.parseInt(n, 10) || 0);
  return { core, pre: preParts.length > 0 ? preParts.join("-") : null };
}

export function rolloutBucket(installId: string): number {
  const h = createHash("sha256").update(installId, "utf8").digest();
  // Interpret the digest as a big integer modulo 100; the low bytes suffice since 256 ≡ 56 (mod 100),
  // but a straightforward big-int mod is clearer.
  let acc = 0n;
  for (const byte of h) acc = (acc << 8n) | BigInt(byte);
  return Number(acc % 100n);
}

export type UpdateDecision = { update: false } | { update: true; version: string; notes?: string; pub_date?: string; url: string; signature: string };

export function decideUpdate(file: ChannelFile, target: string, arch: string, currentVersion: string, installId: string | null): UpdateDecision {
  const platform = file.platforms[`${target}-${arch}`];
  if (!platform) return { update: false };
  if (compareVersions(currentVersion, file.version) >= 0) return { update: false };
  const blocked = file.blocked_versions.includes(currentVersion);
  if (!blocked) {
    const bucket = rolloutBucket(installId ?? "");
    if (bucket >= file.rollout_percent) return { update: false };
  }
  return { update: true, version: file.version, notes: file.notes, pub_date: file.pub_date, url: platform.url, signature: platform.signature };
}
