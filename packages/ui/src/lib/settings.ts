import type { JobOptions, Packaging } from "@cia/engine-client";
import type { ThemeChoice } from "./theme";
import type { UiScaleId } from "./uiScale";

/** Versioned settings blob (DESIGN.md 4.9). The host persists it. */
export interface Settings {
  v: 1;
  theme: ThemeChoice;
  textSize: UiScaleId;
  tourVersionSeen: number;
  welcomeSeen: boolean;
  tierByGroup: Record<string, string>;
  lastPresetId: string | null;
  customBytes: number;
  customPerMessage: boolean;
  packaging: Packaging;
  options: JobOptions;
  outputDir: { type: "same_as_source" } | { type: "folder"; path: string };
  updatesAuto: boolean;
  updateChannel: "stable" | "beta";
  installId: string;
}

export const TOUR_VERSION = 1;

export function defaultOptions(): JobOptions {
  return {
    keep_photo_details: false,
    keep_location: false,
    allow_format_change: true,
    modern_formats: false,
    flatten_transparency: false,
    max_long_edge: null,
    keep_document_details: true,
    video: { format: "most_compatible", faster: true, audio: { type: "mix_all" }, frame_rate: "automatic", trims: {} },
    audio: { format: "automatic" },
    optimise_inside_archives: true,
    output_dir: null,
  };
}

export function defaultSettings(): Settings {
  return {
    v: 1,
    theme: "system",
    textSize: "default",
    tourVersionSeen: 0,
    welcomeSeen: false,
    tierByGroup: {},
    lastPresetId: null,
    customBytes: 8_000_000,
    customPerMessage: false,
    packaging: "auto",
    options: defaultOptions(),
    outputDir: { type: "same_as_source" },
    updatesAuto: true,
    updateChannel: "stable",
    installId: "",
  };
}

export function mergeSettings(stored: Record<string, unknown>): Settings {
  const d = defaultSettings();
  const s = stored as Partial<Settings>;
  return {
    ...d,
    ...s,
    v: 1,
    options: { ...d.options, ...(s.options ?? {}), video: { ...d.options.video, ...(s.options?.video ?? {}), trims: {} }, audio: { ...d.options.audio, ...(s.options?.audio ?? {}) } },
    tierByGroup: { ...(s.tierByGroup ?? {}) },
  };
}
