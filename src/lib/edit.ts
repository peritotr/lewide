// Pending row changes of a table tab: what was edited, added or marked for
// removal, kept apart from the loaded page until the user applies them.
import { t } from "../i18n";
import type { Cell, ChangeSet, ColumnDescription, RowChange, Value } from "./api";

export interface PendingInsert {
  id: number;
  /** Columns left out get the database's default. */
  values: Record<string, Value>;
}

export interface PendingUpdate {
  /** The row's whole primary key as it was loaded. */
  key: Cell[];
  set: Record<string, Value>;
}

export interface Pending {
  inserts: PendingInsert[];
  /** By row id (see `rowId`). */
  updates: Record<string, PendingUpdate>;
  deletes: Record<string, Cell[]>;
}

export const emptyPending: Pending = { inserts: [], updates: {}, deletes: {} };

export const pendingCounts = (p: Pending) => ({ added: p.inserts.length, modified: Object.keys(p.updates).length, deleted: Object.keys(p.deletes).length });
export const pendingCount = (p: Pending) => p.inserts.length + Object.keys(p.updates).length + Object.keys(p.deletes).length;

const same = (a: Value, b: Value) => JSON.stringify(a) === JSON.stringify(b);

/** The row's primary key, or null when the table has none or a key value is NULL. */
export function rowKey(columns: ColumnDescription[], row: Value[]): Cell[] | null {
  const key = columns.flatMap((c, i) => (c.primary_key ? [{ column: c.name, value: row[i] }] : []));
  return key.length > 0 && key.every((cell) => cell.value.type !== "null") ? key : null;
}

export const rowId = (key: Cell[]) => JSON.stringify(key.map((cell) => cell.value));

/** Sets one cell of a loaded row. Setting it back to what was loaded removes the change. */
export function editCell(pending: Pending, columns: ColumnDescription[], row: Value[], column: number, value: Value): Pending {
  const key = rowKey(columns, row);
  if (!key) return pending;
  const id = rowId(key);
  if (pending.deletes[id]) return pending;
  const name = columns[column].name;
  const set = { ...(pending.updates[id]?.set ?? {}) };
  if (same(value, row[column])) delete set[name];
  else set[name] = value;
  const updates = { ...pending.updates };
  if (Object.keys(set).length === 0) delete updates[id];
  else updates[id] = { key, set };
  return { ...pending, updates };
}

export function editInsert(pending: Pending, id: number, column: string, value: Value | null): Pending {
  return {
    ...pending,
    inserts: pending.inserts.map((insert) => {
      if (insert.id !== id) return insert;
      const values = { ...insert.values };
      if (value === null) delete values[column];
      else values[column] = value;
      return { ...insert, values };
    }),
  };
}

export function addRow(pending: Pending): Pending {
  const id = pending.inserts.reduce((max, insert) => Math.max(max, insert.id), 0) + 1;
  return { ...pending, inserts: [{ id, values: {} }, ...pending.inserts] };
}

export const removeInsert = (pending: Pending, id: number): Pending => ({ ...pending, inserts: pending.inserts.filter((insert) => insert.id !== id) });

/** Marks a loaded row for deletion, or takes the mark back. Edits to the row are dropped. */
export function toggleDelete(pending: Pending, columns: ColumnDescription[], row: Value[]): Pending {
  const key = rowKey(columns, row);
  if (!key) return pending;
  const id = rowId(key);
  const deletes = { ...pending.deletes };
  const updates = { ...pending.updates };
  if (deletes[id]) delete deletes[id];
  else {
    deletes[id] = key;
    delete updates[id];
  }
  return { ...pending, updates, deletes };
}

/** Inserts first, then updates, then deletes: the order they are applied in. */
export function toChangeSet(namespace: string, table: string, pending: Pending): ChangeSet {
  const changes: RowChange[] = [
    ...[...pending.inserts].reverse().map((insert): RowChange => ({ kind: "insert", values: Object.entries(insert.values).map(([column, value]) => ({ column, value })) })),
    ...Object.values(pending.updates).map((update): RowChange => ({ kind: "update", key: update.key, set: Object.entries(update.set).map(([column, value]) => ({ column, value })) })),
    ...Object.values(pending.deletes).map((key): RowChange => ({ kind: "delete", key })),
  ];
  return { namespace: namespace || undefined, table, changes };
}

const INT_RANGE: [RegExp, bigint, bigint][] = [
  [/^(smallint|int2|smallserial)$/i, -32768n, 32767n],
  [/^(integer|int|int4|serial|mediumint)$/i, -2147483648n, 2147483647n],
  [/^(bigint|int8|bigserial)$/i, -9223372036854775808n, 9223372036854775807n],
];
const WHOLE = /^(tinyint|smallint|mediumint|int|integer|bigint|int2|int4|int8|serial|smallserial|bigserial)\b/i;
const DECIMAL = /^(numeric|decimal|money|smallmoney|number)\b/i;
const FLOAT = /^(real|float|float4|float8|double)/i;
const BOOL = /^(bool|boolean|bit)$/i;
const JSON_TYPE = /^jsonb?$/i;
const TEMPORAL = /^(date|time|timestamp|datetime|timetz|timestamptz|interval|year|smalldatetime)/i;
const BYTES = /^(bytea|blob|tinyblob|mediumblob|longblob|binary|varbinary|image)/i;

export type EditorKind = "text" | "bool" | "enum" | "date" | "long" | "none";

/** How a cell of this column is edited. Binary values have no editor yet. */
export function editorKind(column: ColumnDescription, value: Value): EditorKind {
  const type = column.type_name ?? "";
  if (value.type === "bytes" || BYTES.test(type)) return "none";
  if (column.enum_values && column.enum_values.length > 0) return "enum";
  if (value.type === "bool" || BOOL.test(type)) return "bool";
  if (value.type === "json" || JSON_TYPE.test(type)) return "long";
  if (value.type === "text" && (value.value.includes("\n") || value.value.length > 120)) return "long";
  if (DATE_TIME.test(type) && (value.type === "null" || splitTemporal(displayText(value)) !== null)) return "date";
  return "text";
}

/** Types edited as date, time and zone parts. Intervals and years stay plain text. */
const DATE_TIME = /^(date|time|timestamp|datetime|timetz|timestamptz|datetime2|datetimeoffset|smalldatetime)\b/i;
const displayText = (value: Value): string => (value.type === "temporal" || value.type === "text" ? value.value : "");

export interface TemporalParts {
  /** `YYYY-MM-DD`, or empty for a time of day. */
  date: string;
  /** `HH:MM`, with seconds and fractions when the value had them; empty for a date. */
  time: string;
  /** `+03`, `+03:00`, `Z`, or empty when the value carries no zone. */
  zone: string;
}

/** A date, time or timestamp as the database wrote it, in its parts. Null when the text is something else. */
export function splitTemporal(text: string): TemporalParts | null {
  const match = /^(\d{4}-\d{2}-\d{2})?(?:[ T]?(\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?))?\s*(Z|[+-]\d{2}(?::?\d{2})?)?$/.exec(text.trim());
  if (!match || (!match[1] && !match[2])) return null;
  return { date: match[1] ?? "", time: match[2] ?? "", zone: match[3] ?? "" };
}

export const joinTemporal = (parts: TemporalParts): string => [parts.date, parts.time].filter(Boolean).join(" ") + parts.zone;

/** Whether the parts are a real date and time: no 13th month, no 25th hour. */
export function temporalError(parts: TemporalParts, typeName: string): "date" | "time" | "zone" | null {
  // "time" and "timetz" hold a time of day only; "timestamp…" holds both.
  const wantsDate = !/^time(tz)?(\s|\(|$)/i.test(typeName.trim());
  const wantsTime = !/^date$/i.test(typeName.trim());
  if (wantsDate) {
    const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(parts.date);
    if (!m) return "date";
    const day = new Date(Date.UTC(+m[1], +m[2] - 1, +m[3]));
    if (day.getUTCMonth() !== +m[2] - 1 || day.getUTCDate() !== +m[3]) return "date";
  } else if (parts.date !== "") return "date";
  if (wantsTime) {
    const m = /^(\d{2}):(\d{2})(?::(\d{2})(?:\.\d+)?)?$/.exec(parts.time);
    if (!m || +m[1] > 23 || +m[2] > 59 || +(m[3] ?? 0) > 59) return "time";
  } else if (parts.time !== "") return "time";
  if (parts.zone !== "" && !/^(Z|[+-]\d{2}(:?\d{2})?)$/.test(parts.zone)) return "zone";
  return null;
}

/** One step on the date (days) or the time (minutes), for the arrow keys. */
/** Which parts a column of this type holds. */
export const temporalShape = (typeName: string) => ({ date: !/^time(tz)?(\s|\(|$)/i.test(typeName.trim()), time: !/^date$/i.test(typeName.trim()), zone: /tz|with time zone|^datetimeoffset/i.test(typeName) });

export function stepTemporal(parts: TemporalParts, part: "date" | "time", by: number): TemporalParts {
  if (part === "date") {
    const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(parts.date);
    if (!m) return parts;
    const day = new Date(Date.UTC(+m[1], +m[2] - 1, +m[3] + by));
    return { ...parts, date: day.toISOString().slice(0, 10) };
  }
  const m = /^(\d{2}):(\d{2})(.*)$/.exec(parts.time);
  if (!m) return parts;
  const minutes = (((+m[1] * 60 + +m[2] + by) % 1440) + 1440) % 1440;
  return { ...parts, time: `${String(Math.floor(minutes / 60)).padStart(2, "0")}:${String(minutes % 60).padStart(2, "0")}${m[3]}` };
}

export const isJsonColumn = (column: ColumnDescription, value: Value) => value.type === "json" || JSON_TYPE.test(column.type_name ?? "");

/** The text an editor starts with. */
export function editText(value: Value): string {
  switch (value.type) {
    case "null":
      return "";
    case "json":
      return JSON.stringify(value.value, null, 2);
    case "bool":
    case "float":
      return String(value.value);
    case "bytes":
      return "";
    default:
      return value.value;
  }
}

/**
 * Reads what was typed as a value of the column's type. Checks the type only;
 * constraints are the server's to report when the changes are applied.
 */
export function parseCell(text: string, column: ColumnDescription, current: Value): { value: Value } | { error: string } {
  const type = (column.type_name ?? "").trim();
  const trimmed = text.trim();
  const sample = current.type;

  if (BOOL.test(type) || sample === "bool") {
    if (/^(true|t|1)$/i.test(trimmed)) return { value: { type: "bool", value: true } };
    if (/^(false|f|0)$/i.test(trimmed)) return { value: { type: "bool", value: false } };
    return { error: t("Not a boolean. Type true or false.") };
  }
  if (WHOLE.test(type) || sample === "int") {
    if (!/^[-+]?\d+$/.test(trimmed)) return { error: t("Not a whole number. {type} takes digits only.", { type: type || "integer" }) };
    const number = BigInt(trimmed);
    const range = INT_RANGE.find(([pattern]) => pattern.test(type));
    if (range && (number < range[1] || number > range[2])) {
      return { error: t("Out of range. {type} takes {min} to {max}.", { type, min: String(range[1]), max: String(range[2]) }) };
    }
    return { value: { type: "int", value: number.toString() } };
  }
  if (DECIMAL.test(type) || sample === "decimal") {
    if (!/^[-+]?(\d+\.?\d*|\.\d+)$/.test(trimmed)) return { error: t("Not a number. {type} takes digits and one decimal point.", { type: type || "numeric" }) };
    return { value: { type: "decimal", value: trimmed.replace(/^\+/, "") } };
  }
  if (FLOAT.test(type) || sample === "float") {
    const number = Number(trimmed);
    if (trimmed === "" || !Number.isFinite(number)) return { error: t("Not a number.") };
    return { value: { type: "float", value: number } };
  }
  if (JSON_TYPE.test(type) || sample === "json") {
    try {
      return { value: { type: "json", value: JSON.parse(text) } };
    } catch (e) {
      return { error: t("Invalid JSON: {reason}", { reason: e instanceof Error ? e.message : String(e) }) };
    }
  }
  if (TEMPORAL.test(type) || sample === "temporal") {
    if (trimmed === "") return { error: t("Empty. Type a value, or set NULL.") };
    return { value: { type: "temporal", value: trimmed } };
  }
  // Text keeps what was typed, spaces included.
  return { value: { type: "text", value: text } };
}

