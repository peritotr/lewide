import { describe, expect, it } from "vitest";
import type { ColumnDescription } from "./api";
import { conditionSql, operatorsFor, toWhere, type Condition } from "./filter";

const column = (name: string, type: string): ColumnDescription => ({ name, type_name: type, nullable: true, default_value: null, primary_key: false });
const columns = [column("id", "int8"), column("email", "text"), column("total", "numeric(10,2)"), column("created at", "timestamptz"), column("gift", "bool")];
const cond = (c: string, operator: Condition["operator"], value = "", value2 = ""): Condition => ({ column: c, operator, value, value2 });
const sql = (c: Condition, dialect: "postgres" | "mysql" | "sqlserver" = "postgres") => conditionSql(c, columns, dialect);

describe("filter conditions on ClickHouse", () => {
  const ch = [column("id", "UInt64"), column("name", "Nullable(String)"), column("total", "Decimal(10, 2)")];
  const sql = (c: Condition) => conditionSql(c, ch, "clickhouse");
  it("quotes names with backticks, escapes backslashes and uses startsWith", () => {
    expect(sql(cond("id", "=", "42"))).toBe("`id` = 42");
    expect(sql(cond("total", ">", "1.5"))).toBe("`total` > 1.5");
    expect(sql(cond("name", "=", "a\\'b"))).toBe("`name` = 'a\\\\''b'");
    expect(sql(cond("name", "starts", "50%_"))).toBe("startsWith(`name`, '50%_')");
  });
});

describe("filter conditions on SQL Server", () => {
  it("brackets names, marks text as Unicode and escapes every LIKE wildcard", () => {
    expect(sql(cond("email", "=", "ğ'x"), "sqlserver")).toBe(`[email] = N'ğ''x'`);
    expect(sql(cond("id", "=", "42"), "sqlserver")).toBe("[id] = 42");
    expect(sql(cond("email", "starts", "a[b]_%"), "sqlserver")).toBe(`[email] LIKE N'a![b]!_!%%' ESCAPE '!'`);
  });
});

describe("filter conditions", () => {
  it("writes numbers bare and everything else quoted", () => {
    expect(sql(cond("id", "=", "42"))).toBe('"id" = 42');
    expect(sql(cond("total", ">=", "10.50"))).toBe('"total" >= 10.50');
    expect(sql(cond("email", "=", "o'brien@x.io"))).toBe(`"email" = 'o''brien@x.io'`);
    expect(sql(cond("created at", "<", "2026-01-01"))).toBe(`"created at" < '2026-01-01'`);
    expect(sql(cond("gift", "=", "true"))).toBe('"gift" = TRUE');
    expect(sql(cond("email", "=", "x"), "mysql")).toBe("`email` = 'x'");
  });

  it("never lets typed text out of its quotes", () => {
    // Not a number, so it is a string even in a numeric column: the server rejects it, nothing runs.
    expect(sql(cond("id", "=", "1 OR 1=1"))).toBe(`"id" = '1 OR 1=1'`);
    expect(sql(cond("email", "=", "'; DROP TABLE users; --"))).toBe(`"email" = '''; DROP TABLE users; --'`);
  });

  it("handles the operators that take no value, two values or a list", () => {
    expect(sql(cond("email", "null"))).toBe('"email" IS NULL');
    expect(sql(cond("email", "not_null"))).toBe('"email" IS NOT NULL');
    expect(sql(cond("total", "between", "1", "5"))).toBe('"total" BETWEEN 1 AND 5');
    expect(sql(cond("total", "between", "1", ""))).toBeNull();
    expect(sql(cond("id", "in", "1, 2 ,3,"))).toBe('"id" IN (1, 2, 3)');
    expect(sql(cond("email", "in", "a, b"))).toBe(`"email" IN ('a', 'b')`);
    expect(sql(cond("email", "starts", "50%_a"))).toBe(`"email" LIKE '50!%!_a%' ESCAPE '!'`);
    // MySQL would read \' as an escaped quote and let the rest out.
    expect(sql(cond("email", "=", "\\' OR 1=1 -- "), "mysql")).toBe("`email` = '\\\\'' OR 1=1 -- '");
    expect(sql(cond("email", "like", "%@x.io"))).toBe(`"email" LIKE '%@x.io'`);
    expect(sql(cond("email", "=", ""))).toBe(`"email" = ''`);
    expect(sql(cond("id", ">", ""))).toBeNull();
    expect(sql(cond("missing", "=", "1"))).toBeNull();
  });

  it("joins complete conditions with AND", () => {
    expect(toWhere([cond("id", ">", "5"), cond("total", "between", "1", ""), cond("email", "null")], columns, "postgres")).toBe('"id" > 5 AND "email" IS NULL');
    expect(toWhere([], columns, "postgres")).toBe("");
  });

  it("offers LIKE only for text", () => {
    expect(operatorsFor(columns[1])).toContain("like");
    expect(operatorsFor(columns[0])).not.toContain("like");
  });
});
