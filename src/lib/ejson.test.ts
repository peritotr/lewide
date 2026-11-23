import { describe, expect, it } from "vitest";
import { bsonType, cellAt, parseAs, preview, scalarText, tableColumns, typeLabel } from "./ejson";

const doc = {
  _id: { $oid: "65f1c0a2b7e4d3a1c0ffee01" },
  name: "Desk",
  price: { amount: { $numberDouble: "12.5" }, currency: "EUR", history: [{ $numberInt: "1" }] },
  stock: { $numberInt: "42" },
  views: { $numberLong: "9007199254740993" },
  created: { $date: { $numberLong: "1700000000000" } },
  tags: ["a", "b", "c"],
  gone: null,
};

describe("extended JSON", () => {
  it("names BSON types", () => {
    expect(bsonType(doc._id)).toBe("objectId");
    expect(bsonType(doc.price)).toBe("object");
    expect(bsonType(doc.price.amount)).toBe("double");
    expect(bsonType(doc.views)).toBe("int64");
    expect(bsonType(doc.created)).toBe("date");
    expect(bsonType(null)).toBe("null");
    expect(typeLabel(doc.tags)).toBe("array [3]");
    expect(typeLabel(doc.price)).toBe("object {3}");
  });

  it("shows scalars without their wrappers", () => {
    expect(scalarText(doc.views)).toBe("9007199254740993");
    expect(scalarText(doc.created)).toBe("2023-11-14T22:13:20.000Z");
    expect(scalarText(doc._id)).toBe("65f1c0a2b7e4d3a1c0ffee01");
    expect(preview(doc, 2)).toBe('name: "Desk", price: object {3}');
  });

  it("keeps the type of an edited value", () => {
    expect(parseAs("7", doc.stock)).toEqual({ value: { $numberInt: "7" } });
    expect(parseAs("7.5", doc.stock)).toHaveProperty("error");
    expect(parseAs("9007199254740995", doc.views)).toEqual({ value: { $numberLong: "9007199254740995" } });
    expect(parseAs("2024-01-01T00:00:00Z", doc.created)).toEqual({ value: { $date: { $numberLong: "1704067200000" } } });
    expect(parseAs(" x ", "old")).toEqual({ value: " x " });
    expect(parseAs("x", null)).toHaveProperty("error");
  });

  it("lays a page out as table columns", () => {
    const columns = tableColumns([doc, { _id: "x", extra: true }]);
    expect(columns).toEqual(["_id", "name", "price.amount", "price.currency", "price", "stock", "views", "created", "tags", "gone", "extra"]);
    expect(cellAt(doc, "price.currency")).toBe("EUR");
    expect(cellAt(doc, "extra")).toBeUndefined();
    expect(cellAt(doc, "gone")).toBeNull();
  });
});
