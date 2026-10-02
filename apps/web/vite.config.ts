import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// COOP/COEP in dev too, so OPFS sync handles and (later) threads behave as in production.
const isolation = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

export default defineConfig({
  plugins: [react()],
  server: { headers: isolation },
  preview: { headers: isolation },
  worker: { format: "es" },
  build: { target: "es2022", sourcemap: false },
  optimizeDeps: { exclude: ["@cia/engine-client", "@cia/ui"] },
});
