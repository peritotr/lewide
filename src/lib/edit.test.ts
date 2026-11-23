import { describe, expect, it } from "vitest";
import type { ColumnDescription, Value } from "./api";
import { addRow, editCell, editInsert, emptyPending, joinTemporal, parseCell, pendingCount, rowId, rowKey, splitTemporal, stepTemporal, temporalError, toChangeSet, toggleDelete } from "./edit";

const column = (name: string, type: string, primary = false): ColumnDescription => ({ name, type_name: type, nullable: !primary, default_value: null, primary_key: primary });
const columns = [column("id", "int4", true), column("name", "text"), column("total", "numeric(10,2)")];
const int = (n: number): Value => ({ type: "int", value: String(n) });
const text = (s: string): Value => ({ type: "text", value: s });
const row: Value[] = [int(7), text("Ada"), { type: "decimal", value: "10.50" }];

describe("pending changes", () => {
  it("drops an edit that returns to the loaded value", () => {
    let pending = editCell(emptyPending, columns, row, 1, text("Ada L."));
    expect(pendingCount(pending)).toBe(1);
    pending = editCell(pending, columns, row, 2, { type: "decimal", value: "11" });
    expect(pending.updates[rowId(rowKey(columns, row)!)].set).toEqual({ name: text("Ada L."), total: { type: "decimal", value: "11" } });
    pending = editCell(pending, columns, row, 1, text("Ada"));
    pending = editCell(pending, columns, row, 2, { type: "decimal", value: "10.50" });
    expect(pending).toEqual(emptyPending);
  });

  it("does not edit a row marked for deletion, and deleting drops its edits", () => {
    let pending = editCell(emptyPending, columns, row, 1, text("x"));
    pending = toggleDelete(pending, columns, row);
    expect(pendingCount(pending)).toBe(1);
    expect(editCell(pending, columns, row, 1, text("y"))).toBe(pending);
    expect(toggleDelete(pending, columns, row)).toEqual(emptyPending);
  });

  it("refuses rows that cannot be identified", () => {
    const noKey = [column("a", "int4"), column("b", "text")];
    expect(rowKey(noKey, [int(1), text("x")])).toBeNull();
    expect(rowKey(columns, [{ type: "null" }, text("x"), { type: "null" }])).toBeNull();
    expect(editCell(emptyPending, noKey, [int(1), text("x")], 1, text("y"))).toBe(emptyPending);
  });

  it("writes inserts, updates and deletes in that order", () => {
    let pending = addRow(emptyPending);
    pending = editInsert(pending, pending.inserts[0].id, "name", text("Grace"));
    pending = editCell(pending, columns, row, 1, text("Ada L."));
    pending = toggleDelete(pending, columns, [int(9), text("Linus"), { type: "null" }]);
    const set = toChangeSet("public", "people", pending);
    expect(set.namespace).toBe("public");
    expect(set.changes).toEqual([
      { kind: "insert", values: [{ column: "name", value: text("Grace") }] },
      { kind: "update", key: [{ column: "id", value: int(7) }], set: [{ column: "name", value: text("Ada L.") }] },
      { kind: "delete", key: [{ column: "id", value: int(9) }] },
    ]);
  });
});

describe("reading typed text", () => {
  const read = (textValue: string, type: string, current: Value = { type: "null" }) => parseCell(textValue, column("c", type), current);

  it("checks whole numbers and their range", () => {
    expect(read("18450", "int4")).toEqual({ value: int(18450) });
    expect(read("-2147483648", "integer")).toEqual({ value: int(-2147483648) });
    expect(read("2147483648", "int4")).toHaveProperty("error");
    expect(read("12.5", "int4")).toHaveProperty("error");
    expect(read("", "bigint")).toHaveProperty("error");
    expect(read("9223372036854775807", "bigint")).toEqual({ value: { type: "int", value: "9223372036854775807" } });
  });

  it("keeps decimals digit for digit", () => {
    expect(read("0010.500", "numeric(10,2)")).toEqual({ value: { type: "decimal", value: "0010.500" } });
    expect(read("1e5", "numeric")).toHaveProperty("error");
    expect(read("abc", "decimal(4,1)")).toHaveProperty("error");
  });

  it("reads booleans, JSON and text", () => {
    expect(read("TRUE", "bool")).toEqual({ value: { type: "bool", value: true } });
    expect(read("maybe", "boolean")).toHaveProperty("error");
    expect(read('{"a": [1]}', "jsonb")).toEqual({ value: { type: "json", value: { a: [1] } } });
    expect(read("{a}", "json")).toHaveProperty("error");
    expect(read("  spaced ", "varchar(20)")).toEqual({ value: text("  spaced ") });
    expect(read("", "text")).toEqual({ value: text("") });
    expect(read("2026-01-02 10:00", "timestamptz")).toEqual({ value: { type: "temporal", value: "2026-01-02 10:00" } });
  });

  it("falls back to the loaded value's type when the column type is unknown", () => {
    expect(parseCell("x", { ...column("c", ""), type_name: null }, int(1))).toHaveProperty("error");
    expect(parseCell("5", { ...column("c", ""), type_name: null }, int(1))).toEqual({ value: int(5) });
  });
});

describe("dates and times in parts", () => {
  it("splits what the database wrote and joins it back unchanged", () => {
    for (const text of ["2026-11-22", "18:42:07", "2026-11-22 18:42:07+00", "2026-11-22 18:42:07.236714+03:00", "2026-11-22 18:42", "18:42:07Z"]) {
      const parts = splitTemporal(text);
      expect(parts, text).not.toBeNull();
      expect(joinTemporal(parts!)).toBe(text);
    }
    expect(splitTemporal("2026-11-22 18:42:07+00")).toEqual({ date: "2026-11-22", time: "18:42:07", zone: "+00" });
    expect(splitTemporal("3 days")).toBeNull();
    expect(splitTemporal("infinity")).toBeNull();
    expect(splitTemporal("")).toBeNull();
  });

  it("rejects dates and times that do not exist", () => {
    const check = (text: string, type: string) => temporalError(splitTemporal(text) ?? { date: text, time: "", zone: "" }, type);
    expect(check("2026-02-28 23:59:59+00", "timestamptz")).toBeNull();
    expect(check("2026-02-30 10:00", "timestamp")).toBe("date");
    expect(check("2026-13-01", "date")).toBe("date");
    expect(check("2026-11-22 24:00", "timestamp")).toBe("time");
    expect(check("2026-11-22", "timestamp")).toBe("time");
    expect(check("2026-11-22 10:00", "date")).toBe("time");
    expect(check("10:61", "time")).toBe("time");
    expect(temporalError({ date: "2026-11-22", time: "10:00", zone: "UTC" }, "timestamptz")).toBe("zone");
  });

  it("steps a day or a minute, across month ends and midnight", () => {
    const parts = { date: "2026-02-28", time: "23:59:30.5", zone: "+00" };
    expect(stepTemporal(parts, "date", 1).date).toBe("2026-03-01");
    expect(stepTemporal(parts, "date", -59).date).toBe("2025-12-31");
    expect(stepTemporal(parts, "time", 1).time).toBe("00:00:30.5");
    expect(stepTemporal({ ...parts, time: "00:00" }, "time", -1).time).toBe("23:59");
  });
});
