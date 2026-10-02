import { test, expect } from "@playwright/test";
import { fixture, openApp } from "./helpers";

test("a 25 MB random file for Discord shows the refusal card and no download", async ({ page }) => {
  await openApp(page);
  await page.getByTestId("file-input").setInputFiles(fixture("random.bin"));
  await expect(page.getByTestId("file-row")).toHaveCount(1, { timeout: 60_000 });
  await page.getByTestId("tile-discord").click();
  const card = page.getByTestId("refusal-card");
  await expect(card).toBeVisible({ timeout: 60_000 });
  await expect(card).toContainText("can't make this kind of file smaller");
  await expect(page.getByTestId("compress")).toBeDisabled();
  await expect(page.getByTestId("download")).toHaveCount(0);
});
