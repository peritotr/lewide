import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";
import { preferredLanguage } from "./i18n";
import { inTauri, initBackend } from "./lib/api";
import { chooseLanguage, loadDrivers, loadSaved } from "./store";

// The loading screen shows what startup is doing (00-brand); outside the shell there is none.
const status = (text: string) => {
  if (inTauri) void import("@tauri-apps/api/event").then(({ emitTo }) => emitTo("splash", "splash-status", text)).catch(() => {});
};

// Before anything is rendered or named: tab titles and messages are created in this language.
chooseLanguage(preferredLanguage());
status("Loading settings…");

initBackend().then((seedDemo) => {
  if (seedDemo) seedDemo();
  else void loadDrivers().then(loadSaved);

  status("Opening workspace…");
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
  // After the first paint: the shell shows this window and closes the loading screen.
  if (inTauri) requestAnimationFrame(() => setTimeout(() => void import("@tauri-apps/api/core").then(({ invoke }) => invoke("app_ready")).catch(() => {}), 0));
});
