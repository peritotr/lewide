import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";
import { preferredLanguage } from "./i18n";
import { initBackend } from "./lib/api";
import { chooseLanguage, loadDrivers, loadSaved } from "./store";

// Before anything is rendered or named: tab titles and messages are created in this language.
chooseLanguage(preferredLanguage());

initBackend().then((seedDemo) => {
  if (seedDemo) seedDemo();
  else void loadDrivers().then(loadSaved);

  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
});
