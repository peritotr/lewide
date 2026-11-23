import { describe, expect, it, vi } from "vitest";

// The module talks to the desktop shell on import; these tests only use its pure helpers.
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), Channel: class {} }));

const { sqlBlocks, verdict } = await import("./ai");

describe("sqlBlocks", () => {
  const reply = [
    "Verdict: Warnings",
    "",
    "1. Line 1 selects every column.",
    "",
    "Suggested query:",
    "```sql",
    "SELECT id, email",
    "FROM users",
    "WHERE country = 'TR';",
    "```",
    "",
    "An index would help the filter.",
    "",
    "Suggested index:",
    "",
    "```sql",
    "CREATE INDEX users_country ON users (country);",
    "```",
    "",
    "To see the plan, run:",
    "```SQL",
    "EXPLAIN SELECT 1;",
    "```",
  ].join("\n");

  it("finds every statement and which one replaces the query", () => {
    const blocks = sqlBlocks(reply);
    expect(blocks.all).toHaveLength(3);
    expect(blocks.suggested).toBe("SELECT id, email\nFROM users\nWHERE country = 'TR';");
    expect(blocks.indexes).toEqual(["CREATE INDEX users_country ON users (country);"]);
  });

  it("returns nothing to apply when the reply has no suggestion", () => {
    expect(sqlBlocks("It looks fine.\n```sql\nSELECT 1;\n```")).toEqual({ all: ["SELECT 1;"], suggested: null, indexes: [] });
    expect(sqlBlocks("No code here.").all).toEqual([]);
  });

  it("ignores a block that is still being streamed", () => {
    expect(sqlBlocks("Suggested query:\n```sql\nSELECT id FR").suggested).toBeNull();
  });
});

describe("verdict", () => {
  it("reads the first line of a review", () => {
    expect(verdict("Verdict: OK\nNothing to change.")).toBe("ok");
    expect(verdict("verdict: Errors\n1. ...")).toBe("errors");
    expect(verdict("The query is fine. Verdict: OK")).toBeNull();
  });
});
