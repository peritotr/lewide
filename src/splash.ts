// The loading screen (00-brand, "Loading screen"): a small window of its own,
// shown by the shell only when startup takes longer than 300 ms, and closed
// when the main window is ready. It follows the system theme and shows the
// status texts the main window sends while it starts.
import { listen } from "@tauri-apps/api/event";
import "@fontsource/barlow-condensed/600.css";
import "@fontsource/ibm-plex-mono/400.css";
import "./tokens.css";
import "./splash.css";
import { version } from "../package.json";

const dark = window.matchMedia("(prefers-color-scheme: dark)");
const theme = () => document.documentElement.setAttribute("data-theme", dark.matches ? "dark" : "light");
theme();
dark.addEventListener("change", theme);

// Status texts are not translated: the language setting is read later, by the main window.
const corner = (place: string) => `<i class="corner ${place}" aria-hidden="true"></i>`;
document.getElementById("splash")!.innerHTML = `
  <div class="frame">
    ${corner("tl")}${corner("tr")}${corner("bl")}${corner("br")}
    <div class="brand">
      <svg viewBox="0 0 32 32" width="52" height="52" fill="none" aria-hidden="true">
        <path d="M8 2v27M5 26h25" stroke="var(--text)" stroke-width="3.5" />
        <rect x="14" y="12" width="8" height="8" fill="var(--accent)" />
      </svg>
      <div>
        <div class="word">Lew<span>IDE</span></div>
        <div class="version">${version}</div>
      </div>
    </div>
    <div class="status">
      <span id="status">Starting…</span>
      <div class="track" role="progressbar" aria-label="Loading"><div class="bar"></div></div>
    </div>
  </div>`;

void listen<string>("splash-status", (event) => {
  document.getElementById("status")!.textContent = event.payload;
});
