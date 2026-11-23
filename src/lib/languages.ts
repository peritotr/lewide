// The languages the shared editor speaks besides SQL, and how each is cut into
// the pieces Run executes: Cypher (Neo4j) and Kibana-style console requests
// (Elasticsearch, OpenSearch). SQL keeps its own rules in sql.ts.
import { splitStatements, type SqlDialect, type Statement } from "./sql";

export type EditorLanguage = { kind: "sql"; dialect: SqlDialect } | { kind: "cypher" } | { kind: "console" };

/** The editor language of a connection's query tab. */
export function languageOf(driverId: string | undefined, dialect: SqlDialect): EditorLanguage {
  if (driverId === "neo4j") return { kind: "cypher" };
  if (driverId === "elasticsearch") return { kind: "console" };
  // SQL++ quotes names with backticks and strings with double quotes, as MySQL does.
  if (driverId === "couchbase") return { kind: "sql", dialect: "mysql" };
  return { kind: "sql", dialect };
}

export function splitFor(text: string, language: EditorLanguage): Statement[] {
  if (language.kind === "sql") return splitStatements(text, language.dialect);
  if (language.kind === "cypher") return splitCypher(text);
  return splitConsole(text);
}

export function statementAtFor(text: string, position: number, language: EditorLanguage): Statement | null {
  const statements = splitFor(text, language);
  if (statements.length === 0) return null;
  return statements.find((s) => position <= s.to) ?? statements[statements.length - 1];
}

/** Cypher: cut on semicolons outside strings ('…', "…", with backslash escapes), `names` and comments. */
export function splitCypher(text: string): Statement[] {
  const out: Statement[] = [];
  let start = -1;
  let last = 0;
  const push = () => {
    if (start >= 0) out.push({ text: text.slice(start, last), from: start, to: last });
    start = -1;
  };
  let i = 0;
  while (i < text.length) {
    const c = text[i];
    if (c === "/" && text[i + 1] === "/") {
      const end = text.indexOf("\n", i);
      i = end < 0 ? text.length : end;
      continue;
    }
    if (c === "/" && text[i + 1] === "*") {
      const end = text.indexOf("*/", i + 2);
      i = end < 0 ? text.length : end + 2;
      continue;
    }
    if (c === ";") {
      push();
      i++;
      continue;
    }
    if (/\s/.test(c)) {
      i++;
      continue;
    }
    if (start < 0) start = i;
    if (c === "'" || c === '"' || c === "`") {
      i++;
      while (i < text.length && text[i] !== c) i += text[i] === "\\" && c !== "`" ? 2 : 1;
      i = Math.min(i + 1, text.length);
    } else i++;
    last = i;
  }
  push();
  return out;
}

const REQUEST_LINE = /^(GET|POST|PUT|DELETE|HEAD)\s+\S/i;

/** Console text: a request starts at a `METHOD /path` line and runs to the next one. */
export function splitConsole(text: string): Statement[] {
  const out: Statement[] = [];
  let start = -1;
  let last = 0;
  let offset = 0;
  const push = () => {
    if (start >= 0) out.push({ text: text.slice(start, last), from: start, to: last });
    start = -1;
  };
  for (const line of text.split("\n")) {
    const trimmed = line.trim();
    if (REQUEST_LINE.test(trimmed)) {
      push();
      start = offset + (line.length - line.trimStart().length);
      last = offset + line.trimEnd().length;
    } else if (start >= 0 && trimmed !== "" && !(trimmed.startsWith("#") || trimmed.startsWith("//"))) {
      last = offset + line.trimEnd().length;
    }
    offset += line.length + 1;
  }
  push();
  return out;
}

/**
 * Lays console text out: each request's JSON body indented, NDJSON bodies
 * (`_bulk`, `_msearch`) one compact object per line. A body that is not
 * JSON is left as written.
 */
export function formatConsole(text: string, indent = 2): string {
  return splitConsole(text)
    .map((request) => {
      const [line, ...rest] = request.text.split("\n");
      const body = rest.join("\n").trim();
      if (!body) return line.trim();
      try {
        return `${line.trim()}\n${JSON.stringify(JSON.parse(body), null, indent)}`;
      } catch {
        const lines = body.split("\n").filter((l) => l.trim());
        try {
          return [line.trim(), ...lines.map((l) => JSON.stringify(JSON.parse(l)))].join("\n");
        } catch {
          return request.text;
        }
      }
    })
    .join("\n\n");
}

const CLAUSES = ["OPTIONAL MATCH", "MATCH", "WHERE", "WITH", "UNWIND", "RETURN", "ORDER BY", "SKIP", "LIMIT", "CREATE", "MERGE", "ON CREATE SET", "ON MATCH SET", "SET", "DETACH DELETE", "DELETE", "REMOVE", "CALL", "YIELD", "UNION ALL", "UNION", "FOREACH", "LOAD CSV"];
const KEYWORDS = ["AND", "OR", "XOR", "NOT", "IN", "IS", "NULL", "AS", "DISTINCT", "ASC", "DESC", "TRUE", "FALSE", "CASE", "WHEN", "THEN", "ELSE", "END", "EXISTS", "STARTS WITH", "ENDS WITH", "CONTAINS"];

/**
 * Lays Cypher out one clause per line, with keywords in capitals. Strings,
 * `names`, comments and what is between the clauses are kept as written.
 */
export function formatCypher(text: string): string {
  return splitCypher(text)
    .map((statement) => formatStatement(statement.text))
    .join(";\n\n");
}

function formatStatement(statement: string): string {
  // Words outside strings, names and comments, with their places.
  const parts: { text: string; code: boolean }[] = [];
  let i = 0;
  let code = "";
  const flush = () => {
    if (code) parts.push({ text: code, code: true });
    code = "";
  };
  while (i < statement.length) {
    const c = statement[i];
    if (c === "'" || c === '"' || c === "`") {
      flush();
      let j = i + 1;
      while (j < statement.length && statement[j] !== c) j += statement[j] === "\\" && c !== "`" ? 2 : 1;
      parts.push({ text: statement.slice(i, j + 1), code: false });
      i = j + 1;
    } else if (c === "/" && (statement[i + 1] === "/" || statement[i + 1] === "*")) {
      flush();
      const end = statement[i + 1] === "/" ? statement.indexOf("\n", i) : statement.indexOf("*/", i + 2) + 2;
      const stop = end <= 0 || end === 1 ? statement.length : end;
      parts.push({ text: statement.slice(i, stop), code: false });
      i = stop;
    } else {
      code += c;
      i++;
    }
  }
  flush();
  const words = (list: string[]) => list.map((w) => w.replace(/ /g, "\\s+")).join("|");
  const clause = new RegExp(`(^|\\s+)(${words(CLAUSES)})(?=\\s|\\(|$)`, "gi");
  const keyword = new RegExp(`\\b(${words(KEYWORDS)})\\b`, "gi");
  const text = parts
    .map((part) =>
      part.code
        ? part.text
            .replace(/\s+/g, " ")
            .replace(keyword, (w) => w.toUpperCase().replace(/\s+/g, " "))
            .replace(clause, (_m, _lead, word: string) => `\n${word.toUpperCase().replace(/\s+/g, " ")}`)
        : part.text,
    )
    .join("");
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "")
    .join("\n");
}
