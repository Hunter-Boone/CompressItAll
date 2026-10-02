import { browser, $, expect } from "@wdio/globals";
import { clickButton, clickTestId, dismissFirstRun, screenshot, textOf } from "../lib/app";

/** DESIGN.md 4.12: a key with a bad check character is caught before any network call. */
describe("license activation", () => {
  it("reports a typo locally", async () => {
    await dismissFirstRun();
    await clickTestId("allowance-pill");
    await $('[data-testid="upgrade"]').waitForExist();
    await clickButton("I already have a key");
    const modal = await $('[data-testid="activate"]');
    await modal.waitForExist();

    const input = await $('[data-testid="key-input"]');
    await input.click();
    await input.setValue("ABCDEFGHJKMNPQRSTUVW");
    await browser.waitUntil(async () => ((await input.getValue()) ?? "").replace(/-/g, "").length === 20, {
      timeoutMsg: "key input did not take 20 characters",
    });
    expect(await input.getValue()).toMatch(/^[A-Z0-9]{5}-[A-Z0-9]{5}-[A-Z0-9]{5}-[A-Z0-9]{5}$/);

    await clickTestId("activate-button");
    const error = await $('[data-testid="key-error"]');
    await error.waitForExist({ timeout: 10_000 });
    expect((await textOf(error)).trim()).toBe("That key has a typo. Check the email we sent you.");
    await screenshot("05-license-typo");
  });
});
