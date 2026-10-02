import { $, expect } from "@wdio/globals";
import { clickButton, dismissFirstRun, jsClick, openSettings, screenshot, textOf } from "../lib/app";

/** DESIGN.md 4.8: the exact copy of the "Add video and music support" modal. */
describe("FFmpeg setup modal", () => {
  it("shows the 4.8 copy from Settings → Video support", async () => {
    await dismissFirstRun();
    await openSettings("Video support");
    await clickButton("Add video support");

    const modal = await $('[data-testid="ffmpeg-setup"]');
    await modal.waitForExist();
    const text = (await textOf(modal)).replace(/\s+/g, " ");
    expect(text).toContain("Add video and music support");
    expect(text).toContain(
      "Smidge uses FFmpeg to work with videos and some music files. FFmpeg is a free, open-source program used by many apps. Smidge doesn't include it, so it needs to download it once.",
    );
    expect(text).toMatch(/Download: about [\d.]+ MB, from Smidge's public build page on GitHub/);
    expect(text).toContain("FFmpeg runs as a separate program on this computer");
    expect(text).toContain("Your files still never leave this computer");
    expect(text).toContain("FFmpeg is licensed under the LGPL.");
    expect(text).toContain("What does that mean?");
    expect(text).toContain("Download FFmpeg");
    expect(text).toContain("Not now");
    expect(text).toContain("Already have FFmpeg?");
    expect(text).toContain("Use my own copy…");

    await jsClick(modal.$("button=What does that mean?"));
    const expanded = (await textOf(modal)).replace(/\s+/g, " ");
    expect(expanded).toContain(
      "FFmpeg is made by the FFmpeg project, not by Smidge. The build Smidge downloads leaves out every part that isn't under the LGPL licence. Its exact build settings and source code are published at github.com/Hunter-Boone/Smidge-Libraries.",
    );
    await screenshot("04-ffmpeg-modal");

    // "Not now" closes the setup modal; the app has one modal slot, so Settings closes with it.
    await jsClick(modal.$("button=Not now"));
    await modal.waitForExist({ reverse: true, timeout: 10_000 });
    expect(await $('[role="dialog"]').isExisting()).toBe(false);
  });
});
