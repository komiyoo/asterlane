import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@cloudflare/kumo/styles/standalone";
import { App } from "./app/app.tsx";
import "./build-label.ts";
import "./styles.css";

const colorMode = window.matchMedia("(prefers-color-scheme: dark)");

function syncColorMode() {
  document.documentElement.dataset.mode = colorMode.matches ? "dark" : "light";
}

syncColorMode();
colorMode.addEventListener("change", syncColorMode);

const root = document.getElementById("root");
if (root === null) {
  throw new Error("missing #root");
}

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
