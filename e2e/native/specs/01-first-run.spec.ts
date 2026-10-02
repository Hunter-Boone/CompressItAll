import { browser, $, expect } from "@wdio/globals";
import { bodyText, clickTestId, jsClick, screenshot, textOf, waitForApp } from "../lib/app";

/** DESIGN.md 4.6, 4.7: welcome card, then the seven-step tour, on a fresh app-data dir. */
describe("first run", () => {
  it("shows the welcome card and runs the tour", async () => {
    await waitForApp();
    const title = await browser.getTitle();
    expect(title).toContain("Smidge");

    const welcome = await $('[data-testid="welcome"]');
    await welcome.waitForExist({ timeout: 10_000, timeoutMsg: "welcome card did not show on first run" });
    expect(await textOf(welcome)).toContain("Make any file small enough to send.");
    await screenshot("01-welcome");

    await clickTestId("welcome-tour");
    const tour = await $('[data-testid="tour"]');
    await tour.waitForExist({ timeout: 10_000 });
    expect(await textOf(tour)).toContain("Welcome to Smidge");
    expect(await textOf(tour)).toContain("This tour takes about 20 seconds.");
    await screenshot("01-tour-step-1");

    const expected = ["1. Add your files", "2. Pick where it's going", "No limit in mind?", "3. Check, then compress", "Extra options", "You're ready"];
    for (const [i, heading] of expected.entries()) {
      await jsClick(tour.$(i === 0 ? "button=Show me around" : "button=Next"));
      await browser.waitUntil(async () => (await textOf(tour)).includes(heading), { timeoutMsg: `tour step "${heading}" never showed` });
    }
    await jsClick(tour.$("button=Finish"));
    await tour.waitForExist({ reverse: true, timeout: 10_000 });

    const text = await bodyText();
    expect(text).toContain("Add your files");
    expect(text).toContain("Where are you sending it?");
    expect(text).toContain("Your files stay on this computer.");
    await screenshot("01-empty");
  });
});
