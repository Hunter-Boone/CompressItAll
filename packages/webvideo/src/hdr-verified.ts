/**
 * Browsers whose OffscreenCanvas tone-mapping passed the HDR fixture check
 * (DESIGN.md 3.5.10 step 4, 7.4). Maintained by the Playwright HDR run, not
 * by hand: the test extracts frames from the web output and requires
 * SSIMULACRA2 >= 60 against the desktop FFmpeg reference. Empty until that
 * run exists, so every browser refuses HDR sources with `UseDesktopApp`.
 */
export const HDR_VERIFIED_BROWSERS: ReadonlyArray<{ browser: string; minVersion: number }> = [];

export function isHdrVerified(browser: string, version = 0): boolean {
  return HDR_VERIFIED_BROWSERS.some((b) => b.browser === browser && version >= b.minVersion);
}
