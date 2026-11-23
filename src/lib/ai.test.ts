import { describe, expect, it, vi } from "vitest";

// The module talks to the desktop shell on import; these tests only use its pure helpers.
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), Channel: class {} }));

const { clarification, findings, sqlBlocks, verdict } = await import("./ai");

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

describe("review findings", () => {
  it("reads one finding per line, most severe first, and keeps the rest of the text", () => {
    const reply = [
      "Verdict: Errors",
      "- WARNING line 4: `SELECT *` returns every column.",
      "- ERROR line 2: `o.user_id` is compared with `u.email`; the join matches nothing.",
      "- NOTE: consider a LIMIT.",
      "",
      "Suggested query:",
      "```sql",
      "SELECT 1",
      "```",
    ].join("\n");
    const { findings: list, rest } = findings(reply.replace(/^Verdict:[^\n]*\n/, ""));
    expect(list.map((f) => [f.severity, f.line, f.token])).toEqual([
      ["error", 2, "o.user_id"],
      ["warning", 4, "SELECT *"],
      ["note", null, null],
    ]);
    expect(rest.startsWith("Suggested query:")).toBe(true);
    expect(findings("Plain text\n- a list item").findings).toEqual([]);
    expect(findings("- **Error** (line 3) — wrong").findings[0]).toMatchObject({ severity: "error", line: 3, text: "wrong" });
  });
});

describe("clarifying questions", () => {
  it("reads the question and its quick replies", () => {
    const reply = "Question: Which date should decide the month?\n- Option: `orders.created_at` — when the order was placed\n- Option: orders.paid_at - when it was paid\n\nOr say another.";
    expect(clarification(reply)).toEqual({
      question: "Which date should decide the month?",
      options: [
        { value: "orders.created_at", gloss: "when the order was placed" },
        { value: "orders.paid_at", gloss: "when it was paid" },
      ],
      rest: "Or say another.",
    });
    expect(clarification("No question here.")).toBeNull();
  });
});
