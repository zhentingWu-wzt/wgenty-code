import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { applyTheme, readStoredTheme } from "./lib/theme";
import "@fontsource-variable/inter";
import "@fontsource/jetbrains-mono";
import "./styles.css";
import "./styles/globals.css";

const rootEl = document.getElementById("root");
if (!rootEl) {
  throw new Error("#root element not found in index.html");
}

applyTheme(readStoredTheme());

createRoot(rootEl).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
