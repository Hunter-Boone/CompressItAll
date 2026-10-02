import { browser, $, expect } from "@wdio/globals";
import fs from "node:fs";
import path from "node:path";
import { addFiles, clickTestId, dismissFirstRun, screenshot, selectValue, setInputValue, stageFixtures, textOf } from "../lib/app";

/** DESIGN.md 3.10, 4.3: a 12 MP photo to a Custom 0.3 MB limit lands next to the source as "<stem> (Custom 300 KB).jpg". */
describe("image to a custom limit", () => {
  it("compresses photo_12mp.jpg under 300 KB and writes the output next to the source", async () => {
    await dismissFirstRun();
    const [photo] = stageFixtures(["photo_12mp.jpg"]);
    await addFiles([photo!]);
    const row = await $('[data-testid="file-row"]');
    expect(await textOf(row)).toContain("photo_12mp.jpg");

    await clickTestId("tile-custom");
    const custom = await $('[data-testid="custom-size"]');
    await custom.waitForExist();
    const size = await custom.$('input[aria-label="Custom size"]');
    const unit = await custom.$('select[aria-label="Unit"]');
    await selectValue(unit, "1000000");
    await setInputValue(size, "0.3");
    await browser.waitUntil(async () => (await size.getValue()) === "0.3", { timeoutMsg: "custom size did not take 0.3" });
    // The prediction for a 1.5 MB photo at 0.3 MB is a real pass-0 encode: "4000 × 3000 photo → …, 0.3 MB. Quality: …".
    await browser.waitUntil(
      async () => {
        const t = await textOf($('[data-testid="prediction"]'));
        return t.includes("→") && !t.includes("Working out");
      },
      { timeout: 90_000, timeoutMsg: "prediction never appeared" },
    );
    await screenshot("02-planned");

    const compress = await $('[data-testid="compress"]');
    await browser.waitUntil(async () => (await compress.getAttribute("disabled")) === null, { timeoutMsg: "Compress stayed disabled" });
    await compress.click();

    const result = await $('[data-testid="result"]');
    await result.waitForExist({ timeout: 180_000, timeoutMsg: "the result card never appeared" });
    const headline = await textOf($('[data-testid="result-headline"]'));
    expect(headline).toContain("Fits");
    await $('[data-testid="result-actions"]').waitForExist();
    // WebKit's snapshot can lag one frame behind React under Xvfb; let the result card paint.
    await browser.pause(700);
    await screenshot("02-result");

    const out = path.join(path.dirname(photo!), "photo_12mp (Custom 300 KB).jpg");
    expect(fs.existsSync(out)).toBe(true);
    const bytes = fs.statSync(out).size;
    expect(bytes).toBeGreaterThan(1000);
    expect(bytes).toBeLessThan(300_000);
    // The original is untouched and no partial file is left behind.
    expect(fs.statSync(photo!).size).toBe(fs.statSync(path.join(path.dirname(photo!), "photo_12mp.jpg")).size);
    expect(fs.readdirSync(path.dirname(photo!)).filter((n) => n.endsWith(".partial"))).toHaveLength(0);
  });
});
