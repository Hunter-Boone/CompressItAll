import React from "react";
import { createRoot } from "react-dom/client";
import "@cia/ui/styles.css";
import { App } from "@cia/ui";
import { TauriHost } from "./host/TauriHost";

const host = new TauriHost();
createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App host={host} />
  </React.StrictMode>,
);
