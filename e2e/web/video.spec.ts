import { test, expect, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { compressAndWait, downloadBytes, fixture, openApp, pickCustomMb } from "./helpers";

/**
 * Video in the browser (DESIGN.md 3.5.10, 7.4): real WebCodecs in headless
 * Chromium, mediabunny demux/mux, the Rust planner in wasm. Outputs are
 * checked twice: the MP4 box layout in Node, and codec/duration/size with
 * `ffprobe` from PATH (Debian's ffmpeg 5.1 on this VM).
 */

/** Top-level ISO BMFF box types, in file order. */
function mp4Boxes(buf: Buffer): string[] {
  const out: string[] = [];
  let i = 0;
  while (i + 8 <= buf.length) {
    let size = buf.readUInt32BE(i);
    const type = buf.toString("latin1", i + 4, i + 8);
    if (size === 1) size = Number(buf.readBigUInt64BE(i + 8));
    else if (size === 0) size = buf.length - i;
    if (size < 8) throw new Error(`bad box size ${size} at ${i}`);
    out.push(type);
    i += size;
  }
  if (i !== buf.length) throw new Error(`boxes do not cover the file (${i} of ${buf.length} bytes)`);
  return out;
}

interface Probe { container: string; videoCodec: string; audioCodec: string | null; width: number; height: number; duration: number; size: number; rotation: number }

function ffprobe(bytes: Buffer, ext: string): Probe {
  const dir = mkdtempSync(join(tmpdir(), "smidge-video-"));
  const path = join(dir, `out.${ext}`);
  writeFileSync(path, bytes);
  const json = JSON.parse(execFileSync("ffprobe", ["-v", "error", "-print_format", "json", "-show_format", "-show_streams", path], { encoding: "utf8" })) as {
    format: { format_name: string; duration: string; size: string };
    streams: { codec_type: string; codec_name: string; width?: number; height?: number; side_data_list?: { rotation?: number }[]; tags?: Record<string, string> }[];
  };
  const v = json.streams.find((s) => s.codec_type === "video")!;
  const a = json.streams.find((s) => s.codec_type === "audio");
  return {
    container: json.format.format_name,
    videoCodec: v.codec_name,
    audioCodec: a?.codec_name ?? null,
    width: v.width ?? 0,
    height: v.height ?? 0,
    duration: Number(json.format.duration),
    size: Number(json.format.size),
    rotation: v.side_data_list?.find((d) => d.rotation !== undefined)?.rotation ?? Number(v.tags?.rotate ?? 0),
  };
}

async function encoders(page: Page) {
  return page.evaluate(async () => {
    const sup = async (p: Promise<{ supported?: boolean }>) => { try { return !!(await p).supported; } catch { return false; } };
    const V = (globalThis as unknown as { VideoEncoder?: { isConfigSupported(c: object): Promise<{ supported?: boolean }> } }).VideoEncoder;
    const A = (globalThis as unknown as { AudioEncoder?: { isConfigSupported(c: object): Promise<{ supported?: boolean }> } }).AudioEncoder;
    return {
      h264: V ? await sup(V.isConfigSupported({ codec: "avc1.640028", width: 1280, height: 720, bitrate: 1_000_000, framerate: 30 })) : false,
      vp9: V ? await sup(V.isConfigSupported({ codec: "vp09.00.40.08", width: 1280, height: 720, bitrate: 1_000_000, framerate: 30 })) : false,
      aac: A ? await sup(A.isConfigSupported({ codec: "mp4a.40.2", sampleRate: 48_000, numberOfChannels: 2, bitrate: 128_000 })) : false,
      opus: A ? await sup(A.isConfigSupported({ codec: "opus", sampleRate: 48_000, numberOfChannels: 2, bitrate: 128_000 })) : false,
    };
  });
}

test("720p 30 fps 10 s clip to Custom 1.5 MB: under the limit, valid MP4, h264, 10 s", async ({ page }) => {
  await openApp(page);
  const enc = await encoders(page);
  console.log(`WebCodecs encoders in this Chromium: ${JSON.stringify(enc)}`);
  await expect(page.getByTestId("video-banner")).toHaveCount(0);
  await page.getByTestId("file-input").setInputFiles(fixture("v_720p_30fps_10s.mp4"));
  await expect(page.getByTestId("file-row")).toHaveCount(1, { timeout: 60_000 });
  await expect(page.getByTestId("file-row")).toContainText("10 s video", { timeout: 60_000 });
  await pickCustomMb(page, 1.5);
  await expect(page.getByTestId("prediction")).toBeVisible({ timeout: 120_000 });
  console.log(`prediction: ${await page.getByTestId("prediction").textContent()}`);

  const t0 = Date.now();
  await compressAndWait(page);
  const elapsed = Date.now() - t0;
  console.log(`10 s 720p clip to 1.5 MB took ${elapsed} ms in headless Chromium`);
  await expect(page.getByTestId("result-headline")).toContainText("Fits");

  const [download] = await Promise.all([page.waitForEvent("download"), page.getByTestId("download").click()]);
  const name = download.suggestedFilename();
  expect(name).toMatch(/^v_720p_30fps_10s \(Custom .*\)\.(mp4|webm)$/);
  const bytes = await downloadBytes(download);
  expect(bytes.length).toBeLessThan(1_500_000);
  expect(bytes.length).toBeGreaterThan(300_000);

  const ext = name.endsWith(".webm") ? "webm" : "mp4";
  if (ext === "mp4") {
    const boxes = mp4Boxes(bytes);
    expect(boxes[0]).toBe("ftyp");
    expect(boxes).toContain("moov");
    expect(boxes).toContain("mdat");
    expect(boxes.indexOf("moov")).toBeLessThan(boxes.indexOf("mdat")); // fast start
  }
  const info = ffprobe(bytes, ext);
  console.log(`ffprobe: ${JSON.stringify(info)}`);
  if (enc.h264) {
    // H.264 is available: the preferred MP4 target is kept (with Opus sound when, as in Chromium on Linux, there is no AAC encoder).
    expect(ext).toBe("mp4");
    expect(info.videoCodec).toBe("h264");
    expect(info.container).toContain("mp4");
    expect(info.audioCodec).toBe(enc.aac ? "aac" : "opus");
  } else {
    console.log("H.264 encoding is unavailable in this headless Chromium; the VP9/WebM path was exercised instead");
    expect(ext).toBe("webm");
    expect(info.videoCodec).toBe("vp9");
    expect(info.container).toContain("webm");
    expect(info.audioCodec).toBe("opus");
  }
  expect(info.size).toBe(bytes.length);
  expect(Math.abs(info.duration - 10)).toBeLessThanOrEqual(0.1);
  expect(info.width).toBeLessThanOrEqual(1280);
  expect(info.width / info.height).toBeCloseTo(16 / 9, 1);
});

test("portrait 1080x1920 clip stays portrait", async ({ page }) => {
  await openApp(page);
  await page.getByTestId("file-input").setInputFiles(fixture("v_portrait_1080x1920_10s.mp4"));
  await expect(page.getByTestId("file-row")).toContainText("10 s video", { timeout: 60_000 });
  await pickCustomMb(page, 3);
  await compressAndWait(page);
  await expect(page.getByTestId("result-headline")).toContainText("Fits");
  const [download] = await Promise.all([page.waitForEvent("download"), page.getByTestId("download").click()]);
  const bytes = await downloadBytes(download);
  expect(bytes.length).toBeLessThan(3_000_000);
  const info = ffprobe(bytes, download.suggestedFilename().endsWith(".webm") ? "webm" : "mp4");
  console.log(`portrait ffprobe: ${JSON.stringify(info)}`);
  expect(info.height).toBeGreaterThan(info.width);
  expect(info.rotation).toBe(0);
  expect(info.width / info.height).toBeCloseTo(1080 / 1920, 1);
});

test("a truncated MP4 shows the damaged message", async ({ page }) => {
  await openApp(page);
  await page.getByTestId("file-input").setInputFiles(fixture("v_truncated.mp4"));
  await expect(page.getByTestId("file-row")).toContainText("Damaged file", { timeout: 60_000 });
  await page.getByTestId("tile-discord").click();
  await expect(page.getByTestId("step-three")).toContainText("damaged or incomplete", { timeout: 60_000 });
});

test("a 2 min clip for a 1 MB limit is refused with a trim suggestion and no download", async ({ page }) => {
  await openApp(page);
  await page.getByTestId("file-input").setInputFiles(fixture("v_720p_30fps_2min.mp4"));
  await expect(page.getByTestId("file-row")).toContainText("2 min video", { timeout: 120_000 });
  await pickCustomMb(page, 1);
  const card = page.getByTestId("refusal-card");
  await expect(card).toBeVisible({ timeout: 120_000 });
  await expect(card).toContainText("Too long to fit in 1 MB");
  await expect(card).toContainText("Trim it to under");
  await expect(card.getByRole("button", { name: /^Trim to under/ })).toBeVisible();
  await expect(page.getByTestId("compress")).toBeDisabled();
  await expect(page.getByTestId("download")).toHaveCount(0);
  // The suggestion opens the Trim panel with the end handle preset (DESIGN.md 4.4).
  await card.getByRole("button", { name: /^Trim to under/ }).click();
  await expect(page.getByTestId("trim-panel")).toBeVisible();
});
