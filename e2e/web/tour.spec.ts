import { test, expect } from "@playwright/test";

test("welcome, then the 7-step tour, and no tour after a reload", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("welcome")).toBeVisible();
  await page.getByTestId("welcome-tour").click();
  const tour = page.getByTestId("tour");
  await expect(tour).toBeVisible();
  await expect(tour).toContainText("Welcome to Smidge");
  await tour.getByRole("button", { name: "Show me around" }).click();
  for (let step = 2; step <= 6; step++) await tour.getByRole("button", { name: "Next" }).click();
  await expect(tour).toContainText("You're ready");
  await tour.getByRole("button", { name: "Finish" }).click();
  await expect(tour).toHaveCount(0);
  // Settings are saved 150 ms after the change.
  await page.waitForTimeout(500);
  await page.reload();
  await expect(page.getByTestId("dropzone")).toBeVisible();
  await expect(page.getByTestId("welcome")).toHaveCount(0);
  await expect(page.getByTestId("tour")).toHaveCount(0);
});
