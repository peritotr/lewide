import { describe, expect, it } from "vitest";
import { bsonType, cellAt, parseAs, parseLiteral, preview, queryTokens, scalarText, tableColumns, typeLabel } from "./ejson";

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

describe("a value typed over a null", () => {
  it("reads JSON by its syntax and keeps other text as a string", () => {
    expect(parseLiteral("42")).toEqual({ value: { $numberInt: "42" } });
    expect(parseLiteral("9007199254740993")).toEqual({ value: { $numberLong: "9007199254740993" } });
    expect(parseLiteral("1.5")).toEqual({ value: { $numberDouble: "1.5" } });
    expect(parseLiteral("true")).toEqual({ value: true });
    expect(parseLiteral("null")).toEqual({ value: null });
    expect(parseLiteral('"42"')).toEqual({ value: "42" });
    expect(parseLiteral('{"$date": "2026-01-02T00:00:00Z"}')).toEqual({ value: { $date: "2026-01-02T00:00:00Z" } });
    expect(parseLiteral("İzmir")).toEqual({ value: "İzmir" });
    expect(parseLiteral("{ broken")).toEqual({ error: "valid JSON" });
    expect("error" in parseLiteral("  ")).toBe(true);
  });
});

describe("query field colours", () => {
  it("colours keys, operators, strings, numbers and keywords without changing the text", () => {
    const text = `{ price: { $lt: 10.5 }, "city": 'İzmir', ok: true }`;
    const tokens = queryTokens(text);
    expect(tokens.map((t) => t.text).join("")).toBe(text);
    const tone = (part: string) => tokens.find((t) => t.text === part)?.tone;
    expect(tone("price")).toBe("key");
    expect(tone("$lt")).toBe("op");
    expect(tone("10.5")).toBe("num");
    expect(tone('"city"')).toBe("key");
    expect(tone("'İzmir'")).toBe("str");
    expect(tone("true")).toBe("kw");
    // Half-typed text still splits.
    expect(queryTokens(`{ a: "open`).map((t) => t.text).join("")).toBe(`{ a: "open`);
  });
});
