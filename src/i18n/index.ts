// Translations.
//
// The English text in the source code is the key. A language is one JSON file
// in this folder that maps each English text to its translation; a text with
// no entry is shown in English. `pnpm i18n:check` lists what a file is missing
// and what it no longer needs.
//
//   t("Run selection")
//   t("Statement {index} of {total} failed after {ms} ms.", { index, total, ms })
//   tn(count, "{count} row affected", "{count} rows affected")
//   tx("Started {time}. See {link}.", { time, link: <a>…</a> })   // with elements
//   msg("Results")   // in a constant: marks the text; call t() where it is shown
import type { ReactNode } from "react";
import tr from "./tr.json";

/** A translation: one text, or one text per plural form of the language. */
type Entry = string | Partial<Record<Intl.LDMLPluralRule, string>>;
type Dictionary = Record<string, Entry>;

export const LANGUAGES = [
  { code: "en", name: "English" },
  { code: "tr", name: "Türkçe" },
] as const;
export type Language = (typeof LANGUAGES)[number]["code"];

const DICTIONARIES: Record<Language, Dictionary> = { en: {}, tr: tr as Dictionary };
const STORAGE_KEY = "language";

let language: Language = "en";
let dictionary: Dictionary = {};
let plurals = new Intl.PluralRules("en");

/** The saved choice, else the system language if there is a translation for it, else English. */
export function preferredLanguage(): Language {
  const known = (code: string | null | undefined) => LANGUAGES.find((l) => l.code === code?.toLowerCase().split("-")[0])?.code;
  let saved: string | null = null;
  try {
    saved = localStorage.getItem(STORAGE_KEY);
  } catch {
    // No storage (tests, a locked-down webview): fall through to the system language.
  }
  return known(saved) ?? known(globalThis.navigator?.language) ?? "en";
}

/** Switches the language for everything rendered from now on. The caller re-renders the interface. */
export function setLanguage(next: Language, remember = true) {
  language = next;
  dictionary = DICTIONARIES[next];
  plurals = new Intl.PluralRules(next);
  if (typeof document !== "undefined") document.documentElement.lang = next;
  if (remember) {
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      // The choice lasts for this run only.
    }
  }
}

export const currentLanguage = () => language;

type Vars = Record<string, string | number>;

const fill = (text: string, vars?: Vars) =>
  vars ? text.replace(/\{(\w+)\}/g, (whole, name: string) => (name in vars ? String(vars[name]) : whole)) : text;

function lookup(source: string, count?: number): string {
  const entry = dictionary[source];
  if (entry === undefined || entry === "") return source;
  if (typeof entry === "string") return entry;
  const form = count === undefined ? "other" : plurals.select(count);
  return entry[form] ?? entry.other ?? source;
}

/** The text in the current language, with `{name}` placeholders filled in. */
export function t(source: string, vars?: Vars): string {
  return fill(lookup(source), vars);
}

/**
 * A text whose wording depends on a number. `one` and `other` are the English
 * forms; `other` is the key. A translation gives the forms its language has.
 */
export function tn(count: number, one: string, other: string, vars?: Vars): string {
  const translated = dictionary[other] !== undefined && dictionary[other] !== "";
  const text = translated ? lookup(other, count) : count === 1 ? one : other;
  return fill(text, { count, ...vars });
}

/**
 * A text with elements inside it. The translation moves the placeholders to
 * where its grammar wants them; each is replaced by the given node.
 */
export function tx(source: string, parts: Record<string, ReactNode>): ReactNode[] {
  return lookup(source)
    .split(/(\{\w+\})/)
    .filter((piece) => piece !== "")
    .map((piece) => {
      const name = /^\{(\w+)\}$/.exec(piece)?.[1];
      return name !== undefined && name in parts ? parts[name] : piece;
    });
}

/** Marks a text kept in a constant for translation. Returns it unchanged; pass it to `t` where it is shown. */
export const msg = (source: string) => source;
