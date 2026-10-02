import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import { build as esbuild } from "esbuild";
import { createHash } from "node:crypto";
import { readdirSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";

// COOP/COEP in dev too, so OPFS sync handles and (later) threads behave as in production.
const isolation = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

function listPublic(dir: string, base = dir): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir)) {
    if (name.startsWith("_")) continue; // _headers is a Cloudflare directive, not an asset
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...listPublic(p, base));
    else out.push("/" + relative(base, p).split("\\").join("/"));
  }
  return out;
}

/** Bundles src/sw.ts into dist/sw.js with the precache list of this build (DESIGN.md 2.7 offline). */
function serviceWorker(): Plugin {
  let outDir = "dist";
  let root = process.cwd();
  const emitted: string[] = [];
  return {
    name: "smidge-sw",
    apply: "build",
    configResolved(c) {
      outDir = c.build.outDir;
      root = c.root;
    },
    generateBundle(_opts, bundle) {
      for (const name of Object.keys(bundle)) emitted.push("/" + name);
    },
    async closeBundle() {
      const precache = ["/", "/index.html", ...emitted.filter((f) => !f.endsWith(".map")), ...listPublic(resolve(root, "public"))];
      const version = createHash("sha256").update(precache.join("\n")).digest("hex").slice(0, 12);
      await esbuild({
        entryPoints: [resolve(root, "src/sw.ts")],
        bundle: true,
        minify: true,
        format: "iife",
        target: "es2020",
        outfile: resolve(root, outDir, "sw.js"),
        define: { __PRECACHE__: JSON.stringify([...new Set(precache)]), __SW_VERSION__: JSON.stringify(version) },
      });
    },
  };
}

export default defineConfig({
  plugins: [react(), serviceWorker()],
  server: { headers: isolation },
  preview: { headers: isolation },
  worker: { format: "es" },
  build: { target: "es2022", sourcemap: false },
  optimizeDeps: { exclude: ["@cia/engine-client", "@cia/ui"] },
});
