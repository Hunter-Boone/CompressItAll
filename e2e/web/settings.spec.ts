import { test, expect } from "@playwright/test";
import { openApp } from "./helpers";

test("the theme choice survives a reload", async ({ page }) => {
  await openApp(page);
  await page.getByTestId("settings-button").click();
  const theme = page.getByTestId("settings").getByLabel("Theme");
  await theme.selectOption("dark");
  await expect(page.locator("html")).toHaveClass(/theme-dark/);
  await page.waitForTimeout(500);
  await page.reload();
  await expect(page.getByTestId("dropzone")).toBeVisible();
  await expect(page.locator("html")).toHaveClass(/theme-dark/);
  await page.getByTestId("settings-button").click();
  await expect(page.getByTestId("settings").getByLabel("Theme")).toHaveValue("dark");
});
