import { describe, expect, it } from "vitest";
import { destructiveStatements, splitStatements, statementAt } from "./sql";

const texts = (sql: string, dialect: Parameters<typeof splitStatements>[1] = "postgres") =>
  splitStatements(sql, dialect).map((s) => s.text);

describe("splitStatements", () => {
  it("splits on semicolons and keeps the terminator", () => {
    expect(texts("SELECT 1; SELECT 2;")).toEqual(["SELECT 1;", "SELECT 2;"]);
  });

  it("keeps a last statement without a semicolon", () => {
    expect(texts("SELECT 1;\n\nSELECT 2  \n")).toEqual(["SELECT 1;", "SELECT 2"]);
  });

  it("attaches a leading comment to its statement and drops trailing comments", () => {
    const sql = "-- first\nSELECT 1;\n\n-- second\nSELECT 2;\n-- trailing note\n";
    expect(texts(sql)).toEqual(["-- first\nSELECT 1;", "-- second\nSELECT 2;"]);
  });

  it("ignores semicolons inside strings, identifiers and comments", () => {
    expect(texts(`SELECT 'a;b', "c;d" -- e;f\n, 1 /* g;h */;`)).toHaveLength(1);
    expect(texts("SELECT 'it''s; fine'; SELECT 2")).toEqual(["SELECT 'it''s; fine';", "SELECT 2"]);
  });

  it("understands PostgreSQL dollar quoting and nested comments", () => {
    const fn = "CREATE FUNCTION f() RETURNS int AS $body$ BEGIN RETURN 1; END; $body$ LANGUAGE plpgsql;";
    expect(texts(`${fn} SELECT 1;`)).toEqual([fn, "SELECT 1;"]);
    expect(texts("SELECT $$a;b$$; SELECT 2;")).toHaveLength(2);
    expect(texts("SELECT 1 /* a /* b; */ c; */; SELECT 2")).toHaveLength(2);
  });

  it("does not treat a positional parameter as a dollar quote", () => {
    expect(texts("SELECT $1; SELECT $2;")).toHaveLength(2);
  });

  it("understands MySQL backticks, backslash escapes and # comments", () => {
    expect(texts("SELECT `a;b`, 'c\\';d' # e;f\n; SELECT 2", "mysql")).toHaveLength(2);
  });

  it("understands T-SQL brackets, GO lines and blocks", () => {
    const tsql = (sql: string) => texts(sql, "sqlserver");
    expect(tsql("SELECT [a;b]]c] FROM t; SELECT 2")).toEqual(["SELECT [a;b]]c] FROM t;", "SELECT 2"]);
    expect(tsql("SELECT 1\nGO\nSELECT 2\n  go 3\nSELECT 'go'")).toEqual(["SELECT 1", "SELECT 2", "SELECT 'go'"]);
    // A procedure body runs to GO, whatever semicolons it holds.
    const proc = "CREATE OR ALTER PROCEDURE p AS\nBEGIN\n  SELECT 1;\n  SELECT 2;\nEND";
    expect(tsql(`${proc}\nGO\nEXEC p;`)).toEqual([proc, "EXEC p;"]);
    // BEGIN ... END and CASE ... END keep their semicolons; BEGIN TRANSACTION opens no block.
    const block = "IF 1 = 1 BEGIN SELECT CASE WHEN 1 = 1 THEN 'a' END; DELETE FROM t WHERE x = 1; END;";
    expect(tsql(`${block} SELECT 3;`)).toEqual([block, "SELECT 3;"]);
    expect(tsql("BEGIN TRANSACTION; UPDATE t SET a = 1 WHERE b = 2; COMMIT;")).toHaveLength(3);
    expect(tsql("SELECT gogo FROM go_table; SELECT 1")).toHaveLength(2);
  });

  it("understands ClickHouse escapes, # comments and nested comments", () => {
    const ch = (sql: string) => texts(sql, "clickhouse");
    expect(ch("SELECT 'a\\';b', `c\\`;d` # e;f\n; SELECT 2")).toHaveLength(2);
    expect(ch("SELECT 1 /* a /* b; */ c; */; SELECT 2")).toHaveLength(2);
  });

  it("understands DuckDB dollar quoting and plain backslashes", () => {
    const duck = (sql: string) => texts(sql, "duckdb");
    expect(duck("SELECT $$a;b$$, $t$c;$t$; SELECT 'd\\'; SELECT 2")).toEqual(["SELECT $$a;b$$, $t$c;$t$;", "SELECT 'd\\';", "SELECT 2"]);
  });

  it("returns nothing for blank or comment-only text", () => {
    expect(texts("  \n -- nothing here\n")).toEqual([]);
  });
});

describe("statementAt", () => {
  const sql = "SELECT 1;\n\nSELECT 2;\nSELECT 3";
  //           0        9    11       20 21

  it("picks the statement containing the caret", () => {
    expect(statementAt(sql, 3)?.text).toBe("SELECT 1;");
    expect(statementAt(sql, 15)?.text).toBe("SELECT 2;");
  });

  it("treats a caret right after the semicolon as still on that statement", () => {
    expect(statementAt(sql, 9)?.text).toBe("SELECT 1;");
  });

  it("picks the next statement from a blank line, and the last one past the end", () => {
    expect(statementAt(sql, 10)?.text).toBe("SELECT 2;");
    expect(statementAt(`${sql}\n\n`, sql.length + 2)?.text).toBe("SELECT 3");
  });

  it("returns null when there is nothing to run", () => {
    expect(statementAt("  ", 1)).toBeNull();
  });
});

describe("destructiveStatements", () => {
  const kinds = (sql: string) => destructiveStatements(sql).map((d) => d.kind);

  it("flags statements that remove data or structure", () => {
    expect(kinds("DROP TABLE users;")).toEqual(["drop"]);
    expect(kinds("truncate orders")).toEqual(["truncate"]);
    expect(kinds("DELETE FROM users WHERE id = 1")).toEqual(["delete"]);
    expect(kinds("ALTER TABLE users DROP COLUMN email")).toEqual(["alter"]);
  });

  it("flags changes to every row", () => {
    expect(kinds("DELETE FROM users")).toEqual(["delete without WHERE"]);
    expect(kinds("UPDATE users SET active = false")).toEqual(["update without WHERE"]);
    expect(kinds("UPDATE users SET active = false WHERE id = 3")).toEqual([]);
  });

  it("is not fooled by comments and strings", () => {
    expect(kinds("UPDATE users SET note = 'x' -- WHERE id = 1")).toEqual(["update without WHERE"]);
    expect(kinds("UPDATE users SET note = 'WHERE'")).toEqual(["update without WHERE"]);
    expect(kinds("SELECT 'DROP TABLE users', \"delete from\" FROM t /* TRUNCATE x */")).toEqual([]);
    expect(kinds("INSERT INTO log (msg) VALUES ('UPDATE t SET x = 1')")).toEqual([]);
    // A comment marker inside a string does not hide the WHERE after it.
    expect(kinds("UPDATE t SET a = '--' WHERE id = 1")).toEqual([]);
    expect(kinds("UPDATE t SET a = '/*' WHERE id = 1 /* done */")).toEqual([]);
    expect(kinds("DELETE FROM t /* ' */ WHERE note = 'it''s'")).toEqual(["delete"]);
  });

  it("finds them among several statements and inside WITH", () => {
    expect(kinds("SELECT 1; DROP TABLE a; SELECT 2; DELETE FROM b;")).toEqual(["drop", "delete without WHERE"]);
    expect(kinds("WITH gone AS (DELETE FROM users WHERE id < 5 RETURNING *) SELECT * FROM gone")).toEqual(["delete"]);
  });

  it("knows T-SQL's DELETE without FROM and its bracketed names", () => {
    const tsql = (sql: string) => destructiveStatements(sql, "sqlserver").map((d) => d.kind);
    expect(tsql("DELETE people")).toEqual(["delete without WHERE"]);
    expect(tsql("DELETE people WHERE id = 1")).toEqual(["delete"]);
    expect(tsql("UPDATE [where] SET a = 1")).toEqual(["update without WHERE"]);
    expect(tsql("SELECT [delete] FROM t")).toEqual([]);
  });

  it("leaves ordinary statements alone", () => {
    expect(kinds("SELECT * FROM users; INSERT INTO t VALUES (1); CREATE TABLE x (id int); ALTER TABLE x ADD COLUMN y int")).toEqual([]);
    expect(kinds("SELECT updated_at, deleted FROM dropbox_files WHERE truncated")).toEqual([]);
  });
});
