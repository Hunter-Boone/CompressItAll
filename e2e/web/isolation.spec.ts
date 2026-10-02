import { test, expect } from "@playwright/test";
import { compressAndWait, fixture, openApp, pickCustomMb } from "./helpers";

test("the app is cross-origin isolated and no request leaves the origin", async ({ page }) => {
  const foreign: string[] = [];
  await page.route("**/*", (route) => {
    const url = new URL(route.request().url());
    if (url.hostname !== "localhost" && url.hostname !== "127.0.0.1") {
      foreign.push(url.href);
      return route.abort();
    }
    return route.continue();
  });
  await openApp(page);
  expect(await page.evaluate(() => crossOriginIsolated)).toBe(true);
  expect(await page.evaluate(() => typeof SharedArrayBuffer)).toBe("function");

  // Exercise the engine so the worker, wasm and OPFS paths all run.
  await page.getByTestId("file-input").setInputFiles(fixture("photo_1mp.jpg"));
  await pickCustomMb(page, 0.1);
  await compressAndWait(page);
  expect(foreign).toEqual([]);
  expect(await page.evaluate(() => crossOriginIsolated)).toBe(true);
});

test("the service worker registers and caches the engine", async ({ page }) => {
  await openApp(page);
  const cached = await page.evaluate(async () => {
    await navigator.serviceWorker.ready;
    for (let i = 0; i < 100; i++) {
      const keys = await caches.keys();
      for (const k of keys) {
        const c = await caches.open(k);
        if (await c.match("/engine/cia_wasm_bg.wasm")) return true;
      }
      await new Promise((r) => setTimeout(r, 200));
    }
    return false;
  });
  expect(cached).toBe(true);
});
