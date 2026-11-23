import { describe, expect, it } from "vitest";
import { diffCounts, diffLines } from "./diff";

describe("line diff", () => {
  it("keeps common lines and marks the rest", () => {
    const lines = diffLines("SELECT *\nFROM users\nWHERE id = 1", "SELECT id, email\nFROM users\nWHERE id = 1\nLIMIT 1");
    expect(lines).toEqual([
      { kind: "removed", text: "SELECT *" },
      { kind: "added", text: "SELECT id, email" },
      { kind: "same", text: "FROM users" },
      { kind: "same", text: "WHERE id = 1" },
      { kind: "added", text: "LIMIT 1" },
    ]);
    expect(diffCounts(lines)).toEqual({ added: 2, removed: 1 });
  });

  it("handles identical and wholly different texts", () => {
    expect(diffCounts(diffLines("a\nb", "a\nb"))).toEqual({ added: 0, removed: 0 });
    expect(diffCounts(diffLines("a", "b\nc"))).toEqual({ added: 2, removed: 1 });
  });
});
