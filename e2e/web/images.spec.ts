import { test, expect } from "@playwright/test";
import { compressAndWait, downloadBytes, fixture, jpegDimensions, openApp, pickCustomMb } from "./helpers";

test("12 MP photo to a custom 0.3 MB limit, downloaded file is a JPEG under the limit", async ({ page }) => {
  await openApp(page);
  await page.getByTestId("file-input").setInputFiles(fixture("photo_12mp.jpg"));
  await expect(page.getByTestId("file-row")).toHaveCount(1);
  await expect(page.getByTestId("file-row")).toContainText("4000 × 3000");
  const tPreview = Date.now();
  await pickCustomMb(page, 0.3);
  await expect(page.getByTestId("prediction")).toBeVisible({ timeout: 120_000 });
  console.log(`preview took ${Date.now() - tPreview} ms: ${await page.getByTestId("prediction").textContent()}`);
  await expect(page.getByTestId("prediction")).toContainText(/\d+ KB\. Quality/);

  const t0 = Date.now();
  await compressAndWait(page);
  const elapsed = Date.now() - t0;
  console.log(`12 MP photo to 0.3 MB took ${elapsed} ms in headless Chromium`);
  await expect(page.getByTestId("result-headline")).toContainText("Fits");

  const [download] = await Promise.all([page.waitForEvent("download"), page.getByTestId("download").click()]);
  expect(download.suggestedFilename()).toMatch(/^photo_12mp \(Custom .*\)\.jpg$/);
  const bytes = await downloadBytes(download);
  expect(bytes.length).toBeLessThan(300_000);
  expect(bytes.length).toBeGreaterThan(50_000);
  const dims = jpegDimensions(bytes);
  expect(dims.width).toBeGreaterThanOrEqual(1000);
  expect(dims.height).toBeGreaterThanOrEqual(750);
  expect(dims.width / dims.height).toBeCloseTo(4 / 3, 1);
});

test("a small photo that already fits is kept as is", async ({ page }) => {
  await openApp(page);
  await page.getByTestId("file-input").setInputFiles(fixture("tiny_already_fits.jpg"));
  await page.getByTestId("tile-discord").click();
  await expect(page.getByTestId("prediction")).toContainText("already fits", { timeout: 60_000 });
  await compressAndWait(page);
  await expect(page.getByTestId("result-headline")).toContainText("Already fits");
});
