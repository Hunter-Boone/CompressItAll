import { defineConfig, devices } from "@playwright/test";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

/**
 * Web end-to-end tests against the production build with the real WASM engine
 * (DESIGN.md 7.4). `vite preview` applies the same COOP/COEP headers as
 * production (apps/web/vite.config.ts), so cross-origin isolation is tested
 * for real. Run `cargo xtask wasm` first; the build fails without
 * apps/web/public/engine.
 */
const here = dirname(fileURLToPath(import.meta.url));
export const ROOT = resolve(here, "../..");
export const BASE = "http://localhost:5182";

export default defineConfig({
  testDir: here,
  outputDir: resolve(here, "test-results"),
  timeout: 240_000,
  expect: { timeout: 30_000 },
  fullyParallel: false,
  workers: 1,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never", outputFolder: resolve(here, "playwright-report") }]] : "list",
  use: {
    baseURL: BASE,
    trace: "retain-on-failure",
    acceptDownloads: true,
  },
  webServer: {
    command: process.env.E2E_SKIP_BUILD ? "npm run preview:e2e -w apps/web" : "npm run build -w apps/web && npm run preview:e2e -w apps/web",
    cwd: ROOT,
    url: BASE,
    reuseExistingServer: !process.env.CI,
    timeout: 300_000,
    stdout: "ignore",
    stderr: "pipe",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
});
