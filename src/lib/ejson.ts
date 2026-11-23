// MongoDB documents arrive as canonical Extended JSON: strings, booleans, null,
// objects and arrays are plain, every other type is a one-key `$` wrapper.
// These helpers read that shape for display and write it back for an edit.

type Json = unknown;
const isObject = (v: Json): v is Record<string, Json> => typeof v === "object" && v !== null && !Array.isArray(v);

const WRAPPERS: Record<string, string> = {
  $oid: "objectId",
  $numberInt: "int32",
  $numberLong: "int64",
  $numberDouble: "double",
  $numberDecimal: "decimal128",
  $date: "date",
  $binary: "binData",
  $regularExpression: "regex",
  $timestamp: "timestamp",
  $code: "javascript",
  $symbol: "symbol",
  $undefined: "undefined",
  $minKey: "minKey",
  $maxKey: "maxKey",
  $dbPointer: "dbPointer",
};

/** The BSON type of a value, as the server names it. */
export function bsonType(value: Json): string {
  if (value === null) return "null";
  if (typeof value === "string") return "string";
  if (typeof value === "boolean") return "bool";
  if (typeof value === "number") return "double";
  if (Array.isArray(value)) return "array";
  if (isObject(value)) {
    const keys = Object.keys(value);
    if (keys[0] === "$code" && keys.includes("$scope")) return "javascriptWithScope";
    if (keys.length >= 1 && keys.length <= 2 && keys[0] in WRAPPERS) return WRAPPERS[keys[0]];
    return "object";
  }
  return "string";
}

export const isContainer = (value: Json) => ["object", "array"].includes(bsonType(value));

/** `object {2}`, `array [3]`, or the plain type name. */
export function typeLabel(value: Json): string {
  const type = bsonType(value);
  if (type === "array") return `array [${(value as Json[]).length}]`;
  if (type === "object") return `object {${Object.keys(value as object).length}}`;
  return type;
}

const dateText = (wrapped: Json): string => {
  const inner = (wrapped as { $date: Json }).$date;
  const ms = isObject(inner) ? Number(inner.$numberLong) : typeof inner === "string" ? Date.parse(inner) : Number(inner);
  const date = new Date(ms);
  return Number.isNaN(date.getTime()) ? String(ms) : date.toISOString();
};

/** A scalar as the user reads it: no wrapper, no quotes. */
export function scalarText(value: Json): string {
  const type = bsonType(value);
  const v = value as Record<string, Json>;
  switch (type) {
    case "null":
      return "null";
    case "string":
    case "bool":
      return String(value);
    case "double":
      return typeof value === "number" ? String(value) : String(v.$numberDouble);
    case "int32":
      return String(v.$numberInt);
    case "int64":
      return String(v.$numberLong);
    case "decimal128":
      return String(v.$numberDecimal);
    case "objectId":
      return String(v.$oid);
    case "date":
      return dateText(value);
    case "binData":
      return `Binary(${String((v.$binary as Record<string, Json>).base64).length * 0.75 | 0} B)`;
    case "regex": {
      const r = v.$regularExpression as Record<string, Json>;
      return `/${String(r.pattern)}/${String(r.options)}`;
    }
    default:
      return JSON.stringify(value);
  }
}

/** Which syntax colour a scalar takes. */
export function scalarTone(value: Json): "str" | "num" | "kw" | "fn" {
  const type = bsonType(value);
  if (type === "string") return "str";
  if (["double", "int32", "int64", "decimal128"].includes(type)) return "num";
  if (["bool", "null", "undefined"].includes(type)) return "kw";
  return "fn";
}

/** Text typed over a value, as a value of the same type; an error when it does not read as one. */
export function parseAs(text: string, original: Json): { value: Json } | { error: string } {
  const type = bsonType(original);
  const trimmed = text.trim();
  switch (type) {
    case "string":
      return { value: text };
    case "bool":
      return trimmed === "true" || trimmed === "false" ? { value: trimmed === "true" } : { error: "true or false" };
    case "int32":
      return /^-?\d+$/.test(trimmed) && Math.abs(Number(trimmed)) <= 2147483647 ? { value: { $numberInt: trimmed } } : { error: "a whole number (int32)" };
    case "int64":
      return /^-?\d+$/.test(trimmed) ? { value: { $numberLong: trimmed } } : { error: "a whole number (int64)" };
    case "double":
      return trimmed !== "" && Number.isFinite(Number(trimmed)) ? { value: { $numberDouble: trimmed } } : { error: "a number" };
    case "decimal128":
      return /^-?\d+(\.\d+)?$/.test(trimmed) ? { value: { $numberDecimal: trimmed } } : { error: "a decimal number" };
    case "objectId":
      return /^[0-9a-f]{24}$/i.test(trimmed) ? { value: { $oid: trimmed.toLowerCase() } } : { error: "24 hexadecimal characters" };
    case "date": {
      const ms = Date.parse(trimmed);
      return Number.isNaN(ms) ? { error: "a date such as 2026-01-31T10:00:00Z" } : { value: { $date: { $numberLong: String(ms) } } };
    }
    default:
      return { error: "this type cannot be edited here" };
  }
}

/** The first fields of a document on one line, for a collapsed block. */
export function preview(document: Record<string, Json>, max = 4): string {
  return Object.entries(document)
    .filter(([key]) => key !== "_id")
    .slice(0, max)
    .map(([key, value]) => `${key}: ${isContainer(value) ? typeLabel(value) : bsonType(value) === "string" ? JSON.stringify(value) : scalarText(value)}`)
    .join(", ");
}

/**
 * The table's columns: every top-level field in the page, with the scalars of
 * a nested object one level down as dotted paths.
 */
export function tableColumns(documents: Record<string, Json>[]): string[] {
  const columns: string[] = [];
  const add = (name: string) => void (columns.includes(name) || columns.push(name));
  for (const document of documents) {
    for (const [key, value] of Object.entries(document)) {
      if (bsonType(value) !== "object") add(key);
      else {
        const inner = Object.entries(value as Record<string, Json>);
        if (inner.length === 0 || inner.every(([, v]) => isContainer(v))) add(key);
        for (const [k, v] of inner) if (!isContainer(v)) add(`${key}.${k}`);
        if (inner.some(([, v]) => isContainer(v)) && !inner.every(([, v]) => isContainer(v))) add(key);
      }
    }
  }
  return columns;
}

/** A document's value for a table column; undefined when the field is missing. */
export function cellAt(document: Record<string, Json>, column: string): Json | undefined {
  if (column in document) return document[column];
  const dot = column.indexOf(".");
  if (dot < 0) return undefined;
  const outer = document[column.slice(0, dot)];
  return isObject(outer) && bsonType(outer) === "object" ? outer[column.slice(dot + 1)] : undefined;
}
