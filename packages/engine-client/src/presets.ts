import presetsJson from "@cia/presets";
import type { Preset, PresetFile, PresetLimit, ResolvedLimit } from "./generated";

export const PRESET_FILE = presetsJson as unknown as PresetFile;
export const PRESETS: Preset[] = [...PRESET_FILE.presets].sort((a, b) => a.order - b.order);

export function presetById(id: string): Preset | undefined {
  return PRESETS.find((p) => p.id === id);
}

/** Tiles: one per group, carrying its tiers. */
export interface PresetGroup {
  group: string;
  tileLabel: string;
  icon: string;
  mode: Preset["mode"];
  tiers: Preset[];
  defaultTier: Preset;
}

export function presetGroups(): PresetGroup[] {
  const groups = new Map<string, Preset[]>();
  for (const p of PRESETS) {
    const list = groups.get(p.group) ?? [];
    list.push(p);
    groups.set(p.group, list);
  }
  return [...groups.entries()].map(([group, tiers]) => {
    const first = tiers[0]!;
    return {
      group,
      tileLabel: first.tile_label,
      icon: first.icon,
      mode: first.mode,
      tiers,
      defaultTier: tiers.find((t) => t.default_tier) ?? first,
    };
  });
}

/** Mirrors cia_core::limits::raw_budget so the UI can show numbers before the engine answers. */
export function rawBudget(limitBytes: number, counts: "raw" | "mime_base64", safetyBytes: number): number {
  const raw = counts === "raw" ? limitBytes : Math.floor((Math.max(0, limitBytes - 100_000) * 3) / 4 * (76 / 78));
  return Math.max(0, raw - safetyBytes);
}

export function resolvePreset(p: Preset): ResolvedLimit {
  if (!p.limit) throw new Error(`preset ${p.id} has no limit`);
  const raw = rawBudget(Number(p.limit.bytes), p.limit.counts, Number(p.limit.safety_bytes));
  const byKind: Record<string, { hard_bytes: bigint; raw_budget_bytes: bigint; safety_bytes: bigint }> = {};
  for (const [k, l] of Object.entries(p.limit_by_kind ?? {}) as [string, PresetLimit | undefined][]) {
    if (!l) continue;
    byKind[k] = {
      hard_bytes: l.bytes,
      raw_budget_bytes: BigInt(rawBudget(Number(l.bytes), l.counts, Number(l.safety_bytes))),
      safety_bytes: l.safety_bytes,
    };
  }
  return {
    hard_bytes: p.limit.bytes,
    raw_budget_bytes: BigInt(raw),
    safety_bytes: p.limit.safety_bytes,
    scope: p.limit.scope,
    max_files_per_message: p.max_files_per_message ?? null,
    by_kind: byKind,
    output_label: p.output_label,
    stated_label: p.limit.counts === "mime_base64" ? `${mbWhole(raw)} of attachments` : mbWhole(Number(p.limit.bytes)),
  };
}

export function resolveCustom(bytes: number, perMessage: boolean): ResolvedLimit {
  return {
    hard_bytes: BigInt(bytes),
    raw_budget_bytes: BigInt(bytes),
    safety_bytes: 0n,
    scope: perMessage ? "per_message" : "per_file",
    max_files_per_message: null,
    by_kind: {},
    output_label: `Custom ${mbWhole(bytes)}`,
    stated_label: mbWhole(bytes),
  };
}

function mbWhole(bytes: number): string {
  if (bytes >= 1_000_000_000 && bytes % 100_000_000 === 0) {
    return `${(bytes / 1_000_000_000).toFixed(1).replace(/\.0$/, "")} GB`;
  }
  if (bytes >= 1_000_000) return `${Math.floor(bytes / 1_000_000)} MB`;
  return `${Math.floor(bytes / 1_000)} KB`;
}
