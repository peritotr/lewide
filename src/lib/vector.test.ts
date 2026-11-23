import { describe, expect, it } from "vitest";
import { parseConditionValue, parseVector, summarize } from "./vector";

describe("vector search input", () => {
  it("reads a vector as JSON or as separated numbers", () => {
    expect(parseVector("[0.1, -2, 3e-2]")).toEqual([0.1, -2, 0.03]);
    expect(parseVector("1 2\n3,4")).toEqual([1, 2, 3, 4]);
    expect(parseVector("")).toEqual([]);
    expect(parseVector("[1, two]")).toBeNull();
    expect(parseVector("1,,2")).toEqual([1, 2]);
    expect(parseVector("0x10")).toBeNull();
  });

  it("reads condition values", () => {
    expect(parseConditionValue("5", false)).toBe(5);
    expect(parseConditionValue("true", false)).toBe(true);
    expect(parseConditionValue("en", false)).toBe("en");
    expect(parseConditionValue('"5"', false)).toBe("5");
    expect(parseConditionValue("en, de", true)).toEqual(["en", "de"]);
    expect(parseConditionValue("[1, 2]", true)).toEqual([1, 2]);
  });

  it("summarizes a payload", () => {
    expect(summarize({ n: 5, title: "Ada", tags: ["a", "b"], body: "A much longer text" })).toEqual({ title: "Ada", tags: ["a", "b"], text: "A much longer text" });
    expect(summarize({ title: "Only" })).toEqual({ title: "Only", tags: [], text: null });
    expect(summarize({ body: "Long body text", title: "T" })).toEqual({ title: "T", tags: [], text: "Long body text" });
    expect(summarize(null)).toEqual({ title: null, tags: [], text: null });
  });
});
