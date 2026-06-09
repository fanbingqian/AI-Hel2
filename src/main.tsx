import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { invoke } from "@tauri-apps/api/core";

function applyTheme(theme: string) {
  if (theme === "system") {
    const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
    document.documentElement.setAttribute("data-theme", prefersDark ? "dark" : "light");
  } else if (theme === "dark" || theme === "light") {
    document.documentElement.setAttribute("data-theme", theme);
  }
}

async function initTheme() {
  try {
    const config = await invoke("get_config") as any;
    const theme = config?.appearance?.theme || "system";
    applyTheme(theme);
  } catch {
    applyTheme("system");
  }
}

initTheme().then(() => {
  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>
  );
});
