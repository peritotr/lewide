// The filter builder: conditions on columns, joined with AND, written as the
// text that follows WHERE.
import type { ColumnDescription } from "./api";
import { quoteIdent, type SqlDialect } from "./sql";

export type Operator = "=" | "<>" | ">" | ">=" | "<" | "<=" | "between" | "in" | "like" | "starts" | "null" | "not_null";

export interface Condition {
  column: string;
  operator: Operator;
  value: string;
  /** The upper bound of BETWEEN. */
  value2: string;
}

/** Symbol shown in chips and lists; the label for each is given by the interface. */
export const OPERATOR_SYMBOL: Record<Operator, string> = {
  "=": "=",
  "<>": "≠",
  ">": ">",
  ">=": "≥",
  "<": "<",
  "<=": "≤",
  between: "BETWEEN",
  in: "IN",
  like: "LIKE",
  starts: "^=",
  null: "IS NULL",
  not_null: "IS NOT NULL",
};

const NUMERIC = /^(tinyint|smallint|mediumint|int|integer|bigint|int2|int4|int8|serial|smallserial|bigserial|numeric|decimal|real|float|float4|float8|double|money|number)\b/i;
const TEXTUAL = /char|text|citext|string|clob|enum|uuid|name/i;
/** ClickHouse spells its numbers Int32, UInt64, Float64, Decimal(…), possibly wrapped in Nullable(…). */
const CLICKHOUSE_NUMERIC = /^(Nullable\(|LowCardinality\()*(U?Int\d+|Float\d+|BFloat16|Decimal)/;

/** The operators that make sense for a column: text adds LIKE and "starts with". */
export function operatorsFor(column: ColumnDescription): Operator[] {
  const base: Operator[] = ["=", "<>", ">", ">=", "<", "<=", "between", "in"];
  const text: Operator[] = !column.type_name || TEXTUAL.test(column.type_name) ? ["like", "starts"] : [];
  return [...base, ...text, "null", "not_null"];
}

export const valueCount = (operator: Operator) => (operator === "null" || operator === "not_null" ? 0 : operator === "between" ? 2 : 1);

const isNumber = (text: string) => /^[-+]?(\d+\.?\d*|\.\d+)([eE][-+]?\d+)?$/.test(text.trim());

/**
 * A quoted string. MySQL reads a backslash inside quotes as an escape, so it is
 * doubled there; SQL Server needs N'' to compare characters outside its code page.
 */
function quote(text: string, dialect: SqlDialect): string {
  // ClickHouse, like MySQL, reads a backslash in quotes as an escape.
  const escaped = dialect === "mysql" || dialect === "clickhouse" ? text.replace(/\\/g, "\\\\") : text;
  return `${dialect === "sqlserver" ? "N" : ""}'${escaped.replace(/'/g, "''")}'`;
}

/** A value as a literal: a bare number for a numeric column, a quoted string otherwise. */
function literal(text: string, column: ColumnDescription, dialect: SqlDialect): string {
  if ((NUMERIC.test(column.type_name ?? "") || CLICKHOUSE_NUMERIC.test(column.type_name ?? "")) && isNumber(text)) return text.trim();
  if (/^bool/i.test(column.type_name ?? "") && /^(true|false)$/i.test(text.trim())) return text.trim().toUpperCase();
  return quote(text, dialect);
}

/** One condition as SQL, or null while it is incomplete. */
export function conditionSql(condition: Condition, columns: ColumnDescription[], dialect: SqlDialect): string | null {
  const column = columns.find((c) => c.name === condition.column);
  if (!column) return null;
  const name = quoteIdent(column.name, dialect);
  const needs = valueCount(condition.operator);
  if (needs >= 1 && condition.value === "" && condition.operator !== "=" && condition.operator !== "<>") return null;
  if (needs === 2 && condition.value2 === "") return null;
  switch (condition.operator) {
    case "null":
      return `${name} IS NULL`;
    case "not_null":
      return `${name} IS NOT NULL`;
    case "between":
      return `${name} BETWEEN ${literal(condition.value, column, dialect)} AND ${literal(condition.value2, column, dialect)}`;
    case "in": {
      const items = condition.value.split(",").map((item) => item.trim()).filter((item) => item !== "");
      return items.length > 0 ? `${name} IN (${items.map((item) => literal(item, column, dialect)).join(", ")})` : null;
    }
    case "like":
      return `${name} LIKE ${literal(condition.value, column, dialect)}`;
    case "starts":
      // ClickHouse has no LIKE ... ESCAPE, and startsWith says it exactly.
      if (dialect === "clickhouse") return `startsWith(${name}, ${quote(condition.value, dialect)})`;
      // The typed text is matched as it is: LIKE's own wildcards are escaped.
      // In T-SQL "[" also starts a wildcard.
      return `${name} LIKE ${quote(`${condition.value.replace(dialect === "sqlserver" ? /[!%_[]/g : /[!%_]/g, "!$&")}%`, dialect)} ESCAPE '!'`;
    default:
      return `${name} ${condition.operator} ${literal(condition.value, column, dialect)}`;
  }
}

/** All conditions joined with AND. Incomplete ones are left out. */
export function toWhere(conditions: Condition[], columns: ColumnDescription[], dialect: SqlDialect): string {
  return conditions
    .map((condition) => conditionSql(condition, columns, dialect))
    .filter((sql): sql is string => sql !== null)
    .join(" AND ");
}

/** What a chip shows after the operator. */
export function conditionValueText(condition: Condition): string {
  switch (valueCount(condition.operator)) {
    case 0:
      return "";
    case 2:
      return `${condition.value} … ${condition.value2}`;
    default:
      return condition.operator === "in" ? `(${condition.value})` : condition.value;
  }
}
