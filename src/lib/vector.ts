// Pieces of the vector search view that are worth testing on their own.

/** Reads a JSON array or numbers separated by commas or spaces; null when it is not a list of numbers. */
export function parseVector(text: string): number[] | null {
  const trimmed = text.trim().replace(/^\[/, "").replace(/\]$/, "").trim();
  if (!trimmed) return [];
  const parts = trimmed.split(/[\s,]+/).filter(Boolean);
  const values = parts.map(Number);
  return parts.every((part, i) => /^[-+]?(\d+\.?\d*|\.\d+)(e[-+]?\d+)?$/i.test(part) && Number.isFinite(values[i])) ? values : null;
}

/** A payload condition's value as typed: JSON when it reads as JSON, text otherwise. */
export function parseConditionValue(text: string, list: boolean): unknown {
  const trimmed = text.trim();
  if (list) {
    try {
      const parsed: unknown = JSON.parse(trimmed);
      if (Array.isArray(parsed)) return parsed;
    } catch {
      // Not JSON: a comma-separated list.
    }
    return trimmed.split(",").map((part) => parseConditionValue(part, false));
  }
  if (/^(-?\d+(\.\d+)?|true|false)$/.test(trimmed)) return JSON.parse(trimmed);
  return /^".*"$/.test(trimmed) ? trimmed.slice(1, -1) : trimmed;
}

export interface PayloadSummary {
  /** The first text field. */
  title: string | null;
  /** Short text values of list fields. */
  tags: string[];
  /** The longest text field, when it is not the title. */
  text: string | null;
}

export function summarize(payload: Record<string, unknown> | null): PayloadSummary {
  const entries = Object.entries(payload ?? {}).filter((e): e is [string, string] => typeof e[1] === "string" && e[1] !== "");
  const texts = entries.map(([, v]) => v);
  // Stores may return fields in any order, so a field that is named like a title wins.
  const title = entries.find(([k]) => /^(title|name|label|subject|headline)$/i.test(k))?.[1] ?? texts[0] ?? null;
  const longest = texts.reduce<string | null>((best, v) => (best === null || v.length > best.length ? v : best), null);
  const tags = Object.values(payload ?? {})
    .filter(Array.isArray)
    .flat()
    .filter((v): v is string => typeof v === "string" && v.length <= 24)
    .slice(0, 6);
  return { title, tags, text: longest !== null && longest !== title ? longest : null };
}
