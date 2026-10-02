import { expect, type Download, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { ROOT } from "./playwright.config";

export const FIXTURES = resolve(ROOT, "fixtures/synth");
export const fixture = (name: string) => resolve(FIXTURES, name);

/** Open the app and dismiss the first-run welcome card so specs start on the main flow. */
export async function openApp(page: Page, path = "/") {
  await page.goto(path);
  await expect(page.getByTestId("dropzone")).toBeVisible();
  // Every test runs in a fresh browser context, so the welcome card appears once the host is
  // ready (capabilities probed, settings loaded, first worker compiled). Wait for it: it is also
  // the signal that settings changes will be persisted from here on.
  await expect(page.getByTestId("welcome")).toBeVisible({ timeout: 60_000 });
  await page.getByTestId("welcome-skip").click();
  await expect(page.getByTestId("welcome")).toHaveCount(0);
  await page.waitForTimeout(400); // settings persist 150 ms after a change
}

export async function downloadBytes(d: Download): Promise<Buffer> {
  const p = await d.path();
  if (!p) throw new Error(`download failed: ${await d.failure()}`);
  return readFileSync(p);
}

/** Width and height from a baseline/progressive JPEG's SOF marker; throws if the stream is not a JPEG. */
export function jpegDimensions(buf: Buffer): { width: number; height: number; progressive: boolean } {
  if (buf[0] !== 0xff || buf[1] !== 0xd8) throw new Error("not a JPEG (no SOI)");
  if (buf[buf.length - 2] !== 0xff || buf[buf.length - 1] !== 0xd9) throw new Error("JPEG is truncated (no EOI)");
  let i = 2;
  while (i < buf.length) {
    if (buf[i] !== 0xff) throw new Error(`bad marker at ${i}`);
    const marker = buf[i + 1]!;
    if (marker === 0xd8 || (marker >= 0xd0 && marker <= 0xd7)) { i += 2; continue; }
    const len = buf.readUInt16BE(i + 2);
    if ((marker >= 0xc0 && marker <= 0xc3) || (marker >= 0xc5 && marker <= 0xc7) || (marker >= 0xc9 && marker <= 0xcb) || (marker >= 0xcd && marker <= 0xcf)) {
      return { height: buf.readUInt16BE(i + 5), width: buf.readUInt16BE(i + 7), progressive: marker === 0xc2 || marker === 0xc6 || marker === 0xca || marker === 0xce };
    }
    if (marker === 0xda) break; // SOS before SOF: broken
    i += 2 + len;
  }
  throw new Error("no SOF marker");
}

/** Entry count from the zip end-of-central-directory record. */
export function zipEntryCount(buf: Buffer): number {
  const min = Math.max(0, buf.length - 22 - 65_535);
  for (let i = buf.length - 22; i >= min; i--) {
    if (buf.readUInt32LE(i) === 0x06054b50) return buf.readUInt16LE(i + 10);
  }
  throw new Error("no end-of-central-directory record");
}

/** Pick "Custom size" and type a limit in MB. */
export async function pickCustomMb(page: Page, mb: number) {
  await page.getByTestId("tile-custom").click();
  const input = page.getByTestId("custom-size").getByLabel("Custom size");
  await input.fill(String(mb));
  await input.blur();
}

export async function compressAndWait(page: Page) {
  const btn = page.getByTestId("compress");
  await expect(btn).toBeEnabled({ timeout: 120_000 });
  await btn.click();
  await expect(page.getByTestId("result")).toBeVisible({ timeout: 200_000 });
}
