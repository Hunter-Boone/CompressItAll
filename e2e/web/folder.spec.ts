import { test, expect } from "@playwright/test";
import { readdirSync } from "node:fs";
import { compressAndWait, downloadBytes, fixture, openApp, zipEntryCount } from "./helpers";

test("Grandkids folder to Email downloads one zip with every photo", async ({ page }) => {
  test.setTimeout(600_000);
  await openApp(page);
  const dir = fixture("Grandkids");
  const names = readdirSync(dir).filter((n) => n.endsWith(".jpg")).sort();
  expect(names.length).toBe(23);
  // Playwright uploads a directory to a webkitdirectory input, so each File gets a
  // webkitRelativePath ("Grandkids/IMG_1000.jpg") and the UI treats it as a folder drop.
  await page.getByTestId("folder-input").setInputFiles(dir);
  const folderRow = page.getByTestId("folder-row");
  await expect(folderRow).toContainText("Grandkids", { timeout: 120_000 });
  await expect(folderRow).toContainText("23 photos");
  await folderRow.click();
  await expect(page.getByTestId("file-row")).toHaveCount(23);
  await page.getByTestId("tile-email").click();
  await expect(page.getByTestId("prediction")).toBeVisible({ timeout: 300_000 });
  await compressAndWait(page);
  await expect(page.getByTestId("result-headline")).toContainText(/Fits|fit/);

  const [download] = await Promise.all([page.waitForEvent("download"), page.getByTestId("download").click()]);
  expect(download.suggestedFilename()).toMatch(/\.zip$/);
  const bytes = await downloadBytes(download);
  expect(bytes.subarray(0, 4).toString("hex")).toBe("504b0304");
  expect(zipEntryCount(bytes)).toBe(23);
});
