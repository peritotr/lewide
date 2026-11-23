import type { ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("./tr.json", () => ({
  default: {
    "Run selection": "Seçimi çalıştır",
    "Statement {index} of {total} failed.": "{total} ifadeden {index}. başarısız oldu.",
    "{count} rows affected": "{count} satır etkilendi",
    "Started {time}. See {link}.": "{link} bölümüne bakın. Başlangıç: {time}.",
    "Untranslated yet": "",
  },
}));

const { setLanguage, t, tn, tx } = await import("./index");

afterEach(() => setLanguage("en", false));

describe("translations", () => {
  it("shows the English source until a language is chosen", () => {
    expect(t("Run selection")).toBe("Run selection");
    expect(t("{name} is disconnected.", { name: "acme" })).toBe("acme is disconnected.");
  });

  it("translates and fills placeholders wherever the translation puts them", () => {
    setLanguage("tr", false);
    expect(t("Run selection")).toBe("Seçimi çalıştır");
    expect(t("Statement {index} of {total} failed.", { index: 1, total: 2 })).toBe("2 ifadeden 1. başarısız oldu.");
  });

  it("falls back to English for a text with no translation, or an empty one", () => {
    setLanguage("tr", false);
    expect(t("Something new")).toBe("Something new");
    expect(t("Untranslated yet")).toBe("Untranslated yet");
  });

  it("picks the plural form: English from the two given, others from the translation", () => {
    expect(tn(1, "{count} row affected", "{count} rows affected")).toBe("1 row affected");
    expect(tn(3, "{count} row affected", "{count} rows affected")).toBe("3 rows affected");
    setLanguage("tr", false);
    expect(tn(1, "{count} row affected", "{count} rows affected")).toBe("1 satır etkilendi");
    expect(tn(3, "{count} row affected", "{count} rows affected")).toBe("3 satır etkilendi");
  });

  it("places elements where the translation wants them", () => {
    const element = { element: true } as unknown as ReactNode;
    const parts = { time: "09:41", link: element };
    expect(tx("Started {time}. See {link}.", parts)).toEqual(["Started ", "09:41", ". See ", element, "."]);
    setLanguage("tr", false);
    expect(tx("Started {time}. See {link}.", parts)).toEqual([element, " bölümüne bakın. Başlangıç: ", "09:41", "."]);
  });
});
