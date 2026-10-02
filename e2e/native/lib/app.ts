/** Helpers for driving the real desktop app. Selectors are the UI's data-testid hooks only. */
import { browser, $, $$ } from "@wdio/globals";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { appDataDir, repoRoot, resultsDir } from "../wdio.conf";

export const fixturesRoot = path.join(repoRoot, "fixtures", "synth");

/** Per-run scratch directory: outputs land next to their source, so every test gets clean paths. */
export const runDir = path.join(os.tmpdir(), "smidge-e2e-files", new Date().toISOString().replace(/[:.]/g, "-"));

export function stageFixtures(names: string[], subdir = "files"): string[] {
  const dir = path.join(runDir, subdir);
  fs.mkdirSync(dir, { recursive: true });
  return names.map((name) => {
    const src = path.join(fixturesRoot, name);
    if (!fs.existsSync(src)) throw new Error(`fixture missing: ${src} (run "cargo xtask fixtures")`);
    const dest = path.join(dir, path.basename(name));
    fs.copyFileSync(src, dest);
    return dest;
  });
}

export async function textOf(el: WebdriverIO.Element | ChainablePromiseElement): Promise<string> {
  const resolved = await el;
  return browser.execute((e) => (e as HTMLElement).textContent ?? "", resolved);
}

export async function bodyText(): Promise<string> {
  return browser.execute(() => (document.body?.innerText ?? "").replace(/\s+/g, " "));
}

export async function jsClick(el: WebdriverIO.Element | ChainablePromiseElement) {
  const resolved = (await el) as WebdriverIO.Element;
  await resolved.waitForExist();
  await browser.execute((e) => (e as HTMLElement).click(), resolved);
}

export async function clickTestId(id: string) {
  await jsClick($(`[data-testid="${id}"]`));
}

/** Click a button by its visible text. */
export async function clickButton(text: string) {
  const btn = await $(`button=${text}`);
  await btn.waitForExist({ timeoutMsg: `button "${text}" not found` });
  await jsClick(btn);
}

/** Wait for the shell (header with the settings gear) to render. */
export async function waitForApp(timeout = 60_000) {
  await $('[data-testid="settings-button"]').waitForExist({ timeout, timeoutMsg: "the app shell never rendered" });
}

/** Skip the welcome card and the tour when they show (first run of this data dir). */
export async function dismissFirstRun() {
  await waitForApp();
  const welcome = await $('[data-testid="welcome"]');
  try {
    await welcome.waitForExist({ timeout: 1500 });
  } catch {
    // not a first run
  }
  if (await welcome.isExisting()) {
    await clickTestId("welcome-skip");
    await welcome.waitForExist({ reverse: true, timeout: 10_000 });
  }
  const tour = await $('[data-testid="tour"]');
  if (await tour.isExisting()) {
    await jsClick(tour.$("button=Skip"));
    await tour.waitForExist({ reverse: true, timeout: 10_000 });
  }
}

/**
 * Inject files the way an OS drop would, through the dev-build hook: the Rust side
 * emits `smidge://add-paths`, the TauriHost hands the paths to the UI, and the UI calls
 * `add_paths` exactly as for a real drop.
 */
export async function addFiles(paths: string[]) {
  const before = await rowCount();
  await browser.execute((p: string[]) => {
    const w = window as unknown as {
      __TAURI_INTERNALS__?: { invoke: (cmd: string, args?: unknown) => Promise<unknown> };
    };
    if (!w.__TAURI_INTERNALS__) throw new Error("__TAURI_INTERNALS__ missing: not running inside Tauri");
    return w.__TAURI_INTERNALS__.invoke("debug_add_paths", { paths: p });
  }, paths);
  await browser.waitUntil(async () => (await rowCount()) >= before + paths.length, {
    timeout: 30_000,
    timeoutMsg: `rows for ${paths.join(", ")} never appeared`,
  });
}

export async function rowCount(): Promise<number> {
  return browser.execute(() => document.querySelectorAll('[data-testid="file-row"], [data-testid="folder-row"]').length);
}

export function rows() {
  return $$('[data-testid="file-row"]');
}

export async function openSettings(section?: "General" | "Video support" | "License" | "Updates" | "About") {
  await clickTestId("settings-button");
  await $('[data-testid="settings"]').waitForExist();
  if (section) await jsClick($(`//*[@aria-label="Settings sections"]//button[normalize-space(.)="${section}"]`));
}

export async function closeModal() {
  await jsClick($('[role="dialog"] button[aria-label="Close"]'));
}

export function settingsPath(): string {
  return path.join(appDataDir(), "settings.json");
}

/** The app writes settings 150 ms after a change; wait for the file before reading it. */
export async function readSettings(): Promise<Record<string, unknown>> {
  await browser.waitUntil(() => fs.existsSync(settingsPath()), { timeout: 10_000, timeoutMsg: `${settingsPath()} was never written` });
  return JSON.parse(fs.readFileSync(settingsPath(), "utf8")) as Record<string, unknown>;
}

/** Set a React-controlled input through the native value setter so onChange fires once with the full value. */
export async function setInputValue(el: WebdriverIO.Element | ChainablePromiseElement, value: string) {
  const resolved = (await el) as WebdriverIO.Element;
  await browser.execute(
    (e, v) => {
      const input = e as HTMLInputElement;
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setter?.call(input, v);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    },
    resolved,
    value,
  );
}

/**
 * Pick a <select> option through the DOM instead of WebDriver's click: in WebKitGTK the
 * click opens a native GTK popup that keeps the grab, and the next takeScreenshot hangs.
 */
export async function selectValue(el: WebdriverIO.Element | ChainablePromiseElement, value: string) {
  const resolved = (await el) as WebdriverIO.Element;
  await browser.execute(
    (e, v) => {
      const select = e as HTMLSelectElement;
      const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")?.set;
      setter?.call(select, v);
      select.dispatchEvent(new Event("change", { bubbles: true }));
    },
    resolved,
    value,
  );
}

export async function screenshot(name: string) {
  fs.mkdirSync(path.join(resultsDir, "screens"), { recursive: true });
  const file = path.join(resultsDir, "screens", `${name}.png`);
  await browser.saveScreenshot(file);
  return file;
}
