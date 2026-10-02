import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri serves the built files from `dist/`; in `tauri dev` it loads this dev server (port 5183).
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  envPrefix: ["VITE_", "TAURI_"],
  server: { port: 5183, strictPort: true },
  build: { target: ["es2022", "safari16"], sourcemap: false, minify: !process.env.TAURI_ENV_DEBUG },
  optimizeDeps: { exclude: ["@cia/engine-client", "@cia/ui"] },
});
