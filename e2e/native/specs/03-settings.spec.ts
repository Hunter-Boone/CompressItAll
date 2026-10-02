import { browser, $, expect } from "@wdio/globals";
import { closeModal, dismissFirstRun, openSettings, readSettings, screenshot, selectValue } from "../lib/app";

/** DESIGN.md 4.9: settings live in <app_data>/settings.json with "v": 1 and survive a change. */
describe("settings", () => {
  it("persists the theme and the update channel to settings.json", async () => {
    await dismissFirstRun();
    // The welcome card was skipped in an earlier launch (or just now): that alone wrote the file.
    const before = await readSettings();
    expect(before.v).toBe(1);
    expect(before.welcomeSeen).toBe(true);

    await openSettings("General");
    const theme = await $('[data-testid="settings"] select');
    await selectValue(theme, "dark");
    await browser.waitUntil(async () => browser.execute(() => document.documentElement.classList.contains("theme-dark")), {
      timeoutMsg: "dark theme did not apply",
    });
    await openSettings("Updates");
    const channel = await $('[data-testid="settings"] select');
    await selectValue(channel, "beta");
    await screenshot("03-settings-updates");
    await closeModal();

    await browser.waitUntil(async () => { const s = await readSettings(); return s.theme === "dark" && s.updateChannel === "beta"; }, {
      timeout: 10_000,
      timeoutMsg: "settings.json did not pick up the change",
    });
    const after = await readSettings();
    expect(after.v).toBe(1);
    expect(after.textSize).toBe("default");

    // Back to the defaults so the other specs see the same shell.
    await openSettings("General");
    await selectValue($('[data-testid="settings"] select'), "system");
    await openSettings("Updates");
    await selectValue($('[data-testid="settings"] select'), "stable");
    await closeModal();
    await browser.waitUntil(async () => (await readSettings()).updateChannel === "stable", { timeout: 10_000 });
  });
});
