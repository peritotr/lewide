// The user's choices from Settings. Kept in the browser's storage of the app,
// applied at once: there is no Save button.
import { create } from "zustand";

export interface Prefs {
  density: "compact" | "comfortable";
  /** Editor text size in pixels, 11–20. */
  codeSize: number;
  tabWidth: 2 | 4 | 8;
  wordWrap: boolean;
  autocomplete: boolean;
  /** Format writes keywords in capitals. */
  uppercaseKeywords: boolean;
  highlightStatement: boolean;
  /** The row limit a new query tab starts with. */
  defaultLimit: number;
  numbers: "raw" | "grouped";
  nullStyle: "tag" | "dimmed" | "blank";
  /** Show the statements before applying row changes on every connection, not only Production. */
  previewBeforeApply: boolean;
  /** G06: look for a newer version at startup and download it, to be installed on restart. */
  updateAutomatically: boolean;
}

export const DEFAULT_PREFS: Prefs = {
  density: "compact",
  codeSize: 13,
  tabWidth: 2,
  wordWrap: false,
  autocomplete: true,
  uppercaseKeywords: true,
  highlightStatement: true,
  defaultLimit: 1000,
  numbers: "raw",
  nullStyle: "tag",
  previewBeforeApply: false,
  updateAutomatically: true,
};

const KEY = "lewide.prefs";

function stored(): Prefs {
  try {
    return { ...DEFAULT_PREFS, ...JSON.parse(localStorage.getItem(KEY) ?? "{}") };
  } catch {
    return DEFAULT_PREFS;
  }
}

export const usePrefs = create<Prefs>(() => stored());

/** What the stylesheet reads: row heights and the editor's text size. */
function applyToDocument(prefs: Prefs) {
  const root = document.documentElement;
  if (prefs.density === "comfortable") root.dataset.density = "comfortable";
  else delete root.dataset.density;
  root.style.setProperty("--code-size", `${prefs.codeSize}px`);
}

export function setPref<K extends keyof Prefs>(key: K, value: Prefs[K]) {
  usePrefs.setState({ [key]: value } as Pick<Prefs, K>);
  const prefs = usePrefs.getState();
  applyToDocument(prefs);
  try {
    localStorage.setItem(KEY, JSON.stringify(prefs));
  } catch {
    // Without storage the choice lasts for this run.
  }
}

applyToDocument(usePrefs.getState());

/** A number as the grid shows it: raw, or with separators between thousands. Copying always gives the raw text. */
export function groupDigits(text: string): string {
  const match = /^(-?)(\d+)(\.\d+)?$/.exec(text);
  if (!match) return text;
  return `${match[1]}${match[2].replace(/\B(?=(\d{3})+(?!\d))/g, " ")}${match[3] ?? ""}`;
}
