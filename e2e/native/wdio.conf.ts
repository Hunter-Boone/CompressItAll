/**
 * WebdriverIO drives the BUILT Smidge desktop binary through @wdio/tauri-service
 * (external provider: tauri-driver plus WebKitWebDriver on Linux, msedgedriver on
 * Windows). DESIGN.md 7.5.
 *
 * Build the app first:
 *   npm run build:frontend -w apps/desktop
 *   cargo build -p cia-desktop --features custom-protocol,dev-build
 *
 * Environment:
 *   SMIDGE_APP           path to the binary (default: target/debug/smidge[.exe])
 *   SMIDGE_E2E_DATA      app-data root for the run (default: a fresh folder under the OS temp dir).
 *                        On Linux it is passed as XDG_DATA_HOME so the app starts from a clean slate.
 */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
export const e2eRoot = path.resolve(here, "..");
export const repoRoot = path.resolve(e2eRoot, "..");
export const resultsDir = path.join(e2eRoot, "results", "native");

const exe = process.platform === "win32" ? "smidge.exe" : "smidge";
const application = process.env.SMIDGE_APP ?? path.join(repoRoot, "target", "debug", exe);
if (!fs.existsSync(application)) {
  throw new Error(
    `Smidge binary not found at ${application}. Build it with "cargo build -p cia-desktop --features custom-protocol,dev-build" or set SMIDGE_APP.`,
  );
}

/**
 * One app-data root per run so the first spec sees a first run and the later ones
 * see what it saved. Chosen in the launcher and handed to the workers through the
 * environment (each worker re-imports this file).
 */
process.env.SMIDGE_E2E_DATA ??= path.join(os.tmpdir(), "smidge-e2e", new Date().toISOString().replace(/[:.]/g, "-"));
export const dataRoot = process.env.SMIDGE_E2E_DATA;
fs.mkdirSync(dataRoot, { recursive: true });
if (process.platform === "linux") {
  process.env.XDG_DATA_HOME = dataRoot;
  // Under Xvfb, WebKitGTK must keep its compositing path: with
  // WEBKIT_DISABLE_COMPOSITING_MODE=1 the first takeScreenshot works and every one
  // after the first few seconds hangs until the session dies. Software GL is enough.
  process.env.LIBGL_ALWAYS_SOFTWARE ??= "1";
  delete process.env.WEBKIT_DISABLE_COMPOSITING_MODE;
}

/** Where the app keeps settings.json, logs and so on for this run (DESIGN.md 4.9). */
export function appDataDir(): string {
  const id = "app.smidge.desktop";
  if (process.platform === "linux") return path.join(dataRoot, id);
  if (process.platform === "win32") return path.join(process.env.APPDATA ?? path.join(os.homedir(), "AppData", "Roaming"), id);
  return path.join(os.homedir(), "Library", "Application Support", id);
}

const safe = (s: string) => s.replace(/[^a-z0-9]+/gi, "-").replace(/^-|-$/g, "").slice(0, 80);

export const config: WebdriverIO.Config = {
  runner: "local",
  tsConfigPath: path.join(e2eRoot, "tsconfig.json"),
  specs: [path.join(here, "specs", "**", "*.spec.ts")],
  maxInstances: 1,
  logLevel: "warn",
  outputDir: path.join(resultsDir, "wdio-logs"),
  bail: 0,
  waitforTimeout: 20_000,
  connectionRetryTimeout: 180_000,
  connectionRetryCount: 1,
  framework: "mocha",
  mochaOpts: { ui: "bdd", timeout: 240_000 },
  reporters: [
    "spec",
    ["junit", { outputDir: resultsDir, outputFileFormat: (o: { cid: string }) => `junit-${o.cid}.xml` }],
  ],
  services: [
    [
      "@wdio/tauri-service",
      {
        driverProvider: "external",
        appBinaryPath: application,
        autoInstallTauriDriver: false,
        autoDownloadEdgeDriver: process.platform === "win32",
        statusPollTimeout: 60_000,
        captureBackendLogs: false,
        captureFrontendLogs: false,
        startTimeout: 90_000,
      },
    ],
  ],
  capabilities: [
    {
      browserName: "tauri",
      "tauri:options": { application },
      "wdio:enforceWebDriverClassic": true,
    } as WebdriverIO.Capabilities,
  ],
  onPrepare: function () {
    for (const sub of ["failures", "screens", "app-logs"]) {
      fs.rmSync(path.join(resultsDir, sub), { recursive: true, force: true });
      fs.mkdirSync(path.join(resultsDir, sub), { recursive: true });
    }
    console.log(`[e2e] app data: ${appDataDir()}`);
  },
  after: async function (_result, _caps, specs) {
    const src = path.join(appDataDir(), "logs", "app.log");
    if (!fs.existsSync(src)) return;
    const name = specs?.[0] ? path.basename(specs[0]).replace(/\.spec\.ts$/, "") : "session";
    fs.copyFileSync(src, path.join(resultsDir, "app-logs", `${name}.log`));
  },
  before: async function () {
    try {
      const handle = await browser.getWindowHandle();
      await browser.switchToWindow(handle);
    } catch (err) {
      console.log(`[e2e] could not pin the window handle: ${String(err)}`);
    }
  },
  afterTest: async function (test, _context, { passed }) {
    if (passed) return;
    try {
      const file = path.join(resultsDir, "failures", `${safe(test.parent)}--${safe(test.title)}.png`);
      await browser.saveScreenshot(file);
      console.log(`[e2e] failure screenshot: ${file}`);
    } catch (err) {
      console.log(`[e2e] could not capture failure screenshot: ${String(err)}`);
    }
  },
};
