// Download links for the marketing pages: read channels/stable.json from the Downloads repo at
// request time (revalidated every 5 minutes) and fall back to the releases page.

import { brand, downloadsReleasesUrl } from "./brand";
import { ChannelFileSchema, channelUrl } from "./updates";

export interface DownloadLinks {
  version: string | null;
  notes: string | null;
  windows: string;
  macos: string;
  linux: string;
  releases: string;
  /** False when the channel file could not be read and every link points at the releases page. */
  live: boolean;
}

export async function getDownloads(): Promise<DownloadLinks> {
  const releases = downloadsReleasesUrl();
  const fallback: DownloadLinks = { version: null, notes: null, windows: releases, macos: releases, linux: releases, releases, live: false };
  try {
    const res = await fetch(channelUrl("stable"), { next: { revalidate: 300 }, headers: { Accept: "application/json" } });
    if (!res.ok) return fallback;
    const parsed = ChannelFileSchema.safeParse(await res.json());
    if (!parsed.success) return fallback;
    const p = parsed.data.platforms;
    return {
      version: parsed.data.version,
      notes: parsed.data.notes ?? null,
      windows: p["windows-x86_64"]?.url ?? releases,
      macos: p["darwin-aarch64"]?.url ?? p["darwin-x86_64"]?.url ?? releases,
      linux: p["linux-x86_64"]?.url ?? releases,
      releases,
      live: true,
    };
  } catch {
    return fallback;
  }
}

export const PLATFORMS: { key: "windows" | "macos" | "linux"; label: string; note: string }[] = [
  { key: "windows", label: "Windows", note: "Windows 10 or later, 64-bit" },
  { key: "macos", label: "macOS", note: "macOS 12 or later, Intel and Apple silicon" },
  { key: "linux", label: "Linux", note: "AppImage, glibc 2.35 or later" },
];

export const appName = brand.name;
