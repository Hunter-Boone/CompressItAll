import React from "react";
import { createRoot } from "react-dom/client";
import "@cia/ui/styles.css";
import { App } from "@cia/ui";
import type { EngineHost } from "@cia/engine-client";

async function pickHost(): Promise<EngineHost> {
  const params = new URLSearchParams(location.search);
  // `?mock=1` (or VITE_MOCK_HOST=1) runs the UI against the fake engine for development and UI tests.
  if (params.get("mock") === "1" || import.meta.env.VITE_MOCK_HOST === "1") {
    const { MockHost } = await import("@cia/engine-client/mock");
    return new MockHost({ kind: params.get("desktop") === "1" ? "desktop" : "web", ffmpegInstalled: params.get("ffmpeg") === "1", pro: params.get("pro") === "1", speed: Number(params.get("speed") ?? 1) });
  }
  const { WebHost } = await import("./host/WebHost");
  return new WebHost();
}

// Offline after the first visit (DESIGN.md 2.7). Production only: the dev server has no sw.js.
if (import.meta.env.PROD && "serviceWorker" in navigator && new URLSearchParams(location.search).get("nosw") !== "1") {
  window.addEventListener("load", () => {
    navigator.serviceWorker.register("/sw.js").catch((e) => console.warn("service worker", e));
  });
}

pickHost().then((host) => {
  createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <App host={host} />
    </React.StrictMode>,
  );
});
