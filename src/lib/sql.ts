// Finds statement boundaries in editor text, so Run can execute the statement
// under the caret. This is a scanner, not a parser: it only needs to know where
// a semicolon is real and where it sits inside a string, identifier or comment.

export type SqlDialect = "postgres" | "mysql" | "sqlite" | "sqlserver" | "clickhouse" | "duckdb";

/** PostgreSQL and DuckDB both quote with $$ … $$ or $tag$ … $tag$. */
const dollarQuotes = (dialect: SqlDialect) => dialect === "postgres" || dialect === "duckdb";

/** MySQL and ClickHouse also start a comment with "#". */
const hashComments = (dialect: SqlDialect) => dialect === "mysql" || dialect === "clickhouse";
/** Where a backslash inside quotes escapes the next character: MySQL strings, everything quoted in ClickHouse. */
const backslashes = (dialect: SqlDialect, quote: string) => (dialect === "mysql" && quote !== "`") || dialect === "clickhouse";

// T-SQL: a GO line ends a batch; these statements take the whole batch as their body.
const GO_LINE = /^go(?:[ \t]+\d+)?[ \t]*(?:\r?\n|$)/i;
const MODULE = /^(?:create(?:\s+or\s+alter)?|alter)\s+(?:proc|procedure|function|trigger|view)\b/i;
const WORD = /^[A-Za-z_@#][A-Za-z0-9_@#$]*/;

export interface Statement {
  /** Offset of the first non-blank character; a leading comment belongs to the statement. */
  from: number;
  /** Offset just past the terminating semicolon, or past the last non-blank character. */
  to: number;
  text: string;
}

export function splitStatements(sql: string, dialect: SqlDialect = "postgres"): Statement[] {
  const statements: Statement[] = [];
  const n = sql.length;
  let start = -1; // first non-blank character of the statement being read
  let hasCode = false; // anything besides comments and blanks
  let lastCode = 0; // offset past the last non-blank character
  let i = 0;
  // T-SQL only: open BEGIN ... END and CASE ... END blocks, and whether the
  // statement is a procedure, function, trigger or view body, which runs to GO.
  let depth = 0;
  let module = false;
  const tsql = dialect === "sqlserver";

  const finish = (end: number) => {
    if (start >= 0 && hasCode) {
      statements.push({ from: start, to: end, text: sql.slice(start, end) });
    }
    start = -1;
    hasCode = false;
    depth = 0;
    module = false;
  };

  while (i < n) {
    const c = sql[i];
    const next = sql[i + 1];

    if (c === " " || c === "\t" || c === "\n" || c === "\r") {
      i++;
      continue;
    }
    // A GO line, alone on its line, ends the statement and is not part of it.
    if (tsql && (c === "g" || c === "G") && /(^|\n)[ \t]*$/.test(sql.slice(Math.max(0, i - 200), i)) && GO_LINE.test(sql.slice(i, i + 40))) {
      finish(lastCode);
      i += GO_LINE.exec(sql.slice(i, i + 40))![0].length;
      continue;
    }
    if (start < 0) start = i;

    // Comments.
    if ((c === "-" && next === "-") || (c === "#" && hashComments(dialect))) {
      const eol = sql.indexOf("\n", i);
      i = eol < 0 ? n : eol;
      lastCode = i;
      continue;
    }
    if (c === "/" && next === "*") {
      // PostgreSQL and ClickHouse nest block comments; the others end at the first "*/".
      let depth = 1;
      i += 2;
      while (i < n && depth > 0) {
        if (sql[i] === "*" && sql[i + 1] === "/") {
          depth--;
          i += 2;
        } else if ((dialect === "postgres" || dialect === "clickhouse") && sql[i] === "/" && sql[i + 1] === "*") {
          depth++;
          i += 2;
        } else {
          i++;
        }
      }
      lastCode = i;
      continue;
    }

    const firstCode = !hasCode;
    hasCode = true;

    if (tsql) {
      // [bracketed] names; "]]" is a bracket inside one.
      if (c === "[") {
        i++;
        while (i < n && !(sql[i] === "]" && sql[i + 1] !== "]")) i += sql[i] === "]" ? 2 : 1;
        i = Math.min(i + 1, n);
        lastCode = i;
        continue;
      }
      const word = /[A-Za-z0-9_@#$]/.test(sql[i - 1] ?? " ") ? null : WORD.exec(sql.slice(i, i + 128))?.[0];
      if (word) {
        if (firstCode && MODULE.test(sql.slice(i, i + 200))) module = true;
        const upper = word.toUpperCase();
        // BEGIN TRANSACTION and friends open no block.
        if (upper === "BEGIN" && !/^\s+(tran|transaction|distributed|dialog|conversation)\b/i.test(sql.slice(i + 5, i + 40))) depth++;
        else if (upper === "CASE") depth++;
        else if (upper === "END" && depth > 0 && !/^\s+conversation\b/i.test(sql.slice(i + 3, i + 30))) depth--;
        i += word.length;
        lastCode = i;
        continue;
      }
    }

    // Quoted strings and identifiers. A doubled quote is an escaped quote.
    if (c === "'" || c === '"' || c === "`") {
      const backslashEscapes = backslashes(dialect, c);
      i++;
      while (i < n) {
        if (backslashEscapes && sql[i] === "\\") {
          i += 2;
        } else if (sql[i] === c) {
          if (sql[i + 1] === c) i += 2;
          else break;
        } else {
          i++;
        }
      }
      i = Math.min(i + 1, n);
      lastCode = i;
      continue;
    }

    // PostgreSQL dollar quoting: $$ ... $$ or $tag$ ... $tag$.
    if (c === "$" && dollarQuotes(dialect)) {
      const tag = /^\$[A-Za-z_]*\$/.exec(sql.slice(i, i + 64));
      const afterIdentifier = i > 0 && /[A-Za-z0-9_]/.test(sql[i - 1]);
      if (tag && !afterIdentifier) {
        const close = sql.indexOf(tag[0], i + tag[0].length);
        i = close < 0 ? n : close + tag[0].length;
        lastCode = i;
        continue;
      }
    }

    if (c === ";" && depth === 0 && !module) {
      i++;
      finish(i);
      continue;
    }

    i++;
    lastCode = i;
  }
  finish(lastCode);
  return statements;
}

/** The statement Run executes for a caret at `position`: the one containing it, else the next one, else the last. */
export function statementAt(
  sql: string,
  position: number,
  dialect: SqlDialect = "postgres",
): Statement | null {
  const statements = splitStatements(sql, dialect);
  if (statements.length === 0) return null;
  return statements.find((s) => position <= s.to) ?? statements[statements.length - 1];
}

export function dialectOf(driverId: string | undefined): SqlDialect {
  return driverId === "mysql" ? "mysql" : driverId === "sqlite" ? "sqlite" : driverId === "sqlserver" ? "sqlserver" : driverId === "clickhouse" ? "clickhouse" : driverId === "duckdb" || driverId === "influxdb" ? "duckdb" : "postgres";
}

/** MySQL quotes identifiers with backticks, SQL Server with brackets; the others use the SQL standard form. */
export function quoteIdent(name: string, dialect: SqlDialect): string {
  if (dialect === "mysql") return `\`${name.replace(/`/g, "``")}\``;
  if (dialect === "sqlserver") return `[${name.replace(/]/g, "]]")}]`;
  if (dialect === "clickhouse") return `\`${name.replace(/[\\`]/g, "\\$&")}\``;
  return `"${name.replace(/"/g, '""')}"`;
}

export type Destructive = "drop" | "truncate" | "delete" | "delete without WHERE" | "update without WHERE" | "alter";

/**
 * The statement with comments removed and every string and quoted identifier
 * emptied, upper-cased. Read in one pass, so a comment marker inside a string
 * or a quote inside a comment is not mistaken for the real thing.
 */
function skeleton(statement: string, dialect: SqlDialect): string {
  let out = "";
  let i = 0;
  const n = statement.length;
  while (i < n) {
    const c = statement[i];
    const next = statement[i + 1];
    if ((c === "-" && next === "-") || (c === "#" && hashComments(dialect))) {
      const eol = statement.indexOf("\n", i);
      i = eol < 0 ? n : eol;
      out += " ";
    } else if (c === "/" && next === "*") {
      const end = statement.indexOf("*/", i + 2);
      i = end < 0 ? n : end + 2;
      out += " ";
    } else if (c === "'" || c === '"' || c === "`") {
      const backslashEscapes = backslashes(dialect, c);
      i++;
      while (i < n) {
        if (backslashEscapes && statement[i] === "\\") i += 2;
        else if (statement[i] === c && statement[i + 1] === c) i += 2;
        else if (statement[i] === c) break;
        else i++;
      }
      i++;
      out += c + c;
    } else if (c === "[" && dialect === "sqlserver") {
      i++;
      while (i < n && !(statement[i] === "]" && statement[i + 1] !== "]")) i += statement[i] === "]" ? 2 : 1;
      i++;
      out += "[]";
    } else if (c === "$" && dollarQuotes(dialect) && /^\$[A-Za-z_]*\$/.test(statement.slice(i, i + 64))) {
      const tag = /^\$[A-Za-z_]*\$/.exec(statement.slice(i, i + 64))![0];
      const close = statement.indexOf(tag, i + tag.length);
      i = close < 0 ? n : close + tag.length;
      out += "''";
    } else {
      out += c;
      i++;
    }
  }
  return out.replace(/\s+/g, " ").trim().toUpperCase();
}

/**
 * What in `sql` deserves a second look before it runs against Production: one
 * entry per statement that removes data or structure, or changes every row.
 * A statement inside a WITH clause is found too. This is a safety net, not a
 * parser: it can miss a statement hidden in a function call or DO block.
 */
export function destructiveStatements(sql: string, dialect: SqlDialect = "postgres"): { kind: Destructive; text: string }[] {
  const found: { kind: Destructive; text: string }[] = [];
  for (const statement of splitStatements(sql, dialect)) {
    const text = skeleton(statement.text, dialect);
    const has = (pattern: RegExp) => pattern.test(text);
    let kind: Destructive | null = null;
    if (has(/^DROP\b/)) kind = "drop";
    else if (has(/^TRUNCATE\b/)) kind = "truncate";
    else if (has(/^ALTER\b[\s\S]*\bDROP\b/)) kind = "alter";
    // T-SQL may leave out FROM: DELETE people WHERE ...
    else if (has(/\bDELETE FROM\b/) || (dialect === "sqlserver" && has(/^DELETE\b/))) kind = has(/\bWHERE\b/) ? "delete" : "delete without WHERE";
    else if (has(/(^|\(|\) )UPDATE\b[\s\S]*\bSET\b/) && !has(/\bWHERE\b/)) kind = "update without WHERE";
    if (kind) found.push({ kind, text: statement.text });
  }
  return found;
}
