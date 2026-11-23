// Syntax colours for the editor's non-SQL languages, as CodeMirror stream
// parsers (no grammar package needed). Token names map to the highlight tags
// the editor styles: keyword, string, number, atom, comment, typeName,
// variableName.function, propertyName.
import { StreamLanguage, type StreamParser } from "@codemirror/language";

const CYPHER_KEYWORDS = new Set(
  "match optional where with unwind return order by skip limit create merge on set delete detach remove call yield union all foreach load csv from headers as distinct asc ascending desc descending and or xor not in is null starts ends contains case when then else end exists count use explain profile show".split(" "),
);
const CYPHER_ATOMS = new Set(["true", "false", "null"]);

interface CypherState {
  /** Inside a block comment. */
  comment: boolean;
}

const cypher: StreamParser<CypherState> = {
  name: "cypher",
  startState: () => ({ comment: false }),
  token(stream, state) {
    if (state.comment) {
      if (stream.skipTo("*/")) {
        stream.next();
        stream.next();
        state.comment = false;
      } else stream.skipToEnd();
      return "comment";
    }
    if (stream.eatSpace()) return null;
    if (stream.match("//")) {
      stream.skipToEnd();
      return "comment";
    }
    if (stream.match("/*")) {
      state.comment = true;
      return "comment";
    }
    const c = stream.peek();
    if (c === "'" || c === '"') {
      stream.next();
      let escaped = false;
      for (let next = stream.next(); next !== undefined; next = stream.next()) {
        if (next === c && !escaped) break;
        escaped = !escaped && next === "\\";
      }
      return "string";
    }
    if (c === "`") {
      stream.next();
      stream.skipTo("`") ? stream.next() : stream.skipToEnd();
      return "variableName";
    }
    // A label or relationship type: `:Person`, `[:KNOWS]`.
    if (c === ":" && stream.match(/^:\s*[A-Za-z_][\w]*/)) return "typeName";
    if (stream.match(/^-?\d+(\.\d+)?([eE][+-]?\d+)?/)) return "number";
    if (c === "$" && stream.match(/^\$\w+/)) return "variableName.special";
    if (stream.match(/^[A-Za-z_][\w]*/)) {
      const word = stream.current().toLowerCase();
      if (CYPHER_ATOMS.has(word)) return "atom";
      if (CYPHER_KEYWORDS.has(word)) return "keyword";
      if (stream.match(/^\s*\(/, false)) return "variableName.function";
      return "variableName";
    }
    stream.next();
    return null;
  },
  languageData: { commentTokens: { line: "//", block: { open: "/*", close: "*/" } } },
};

/** Kibana's console: `METHOD /path` lines, then a JSON body. */
const consoleMode: StreamParser<{ body: boolean }> = {
  name: "console",
  startState: () => ({ body: false }),
  token(stream) {
    if (stream.sol() && stream.match(/^\s*(GET|POST|PUT|DELETE|HEAD)\b/i)) return "keyword";
    if (stream.sol() && stream.match(/^\s*(#|\/\/)/)) {
      stream.skipToEnd();
      return "comment";
    }
    if (stream.eatSpace()) return null;
    const c = stream.peek();
    if (c === "/" || (c !== undefined && /[\w.*-]/.test(c) && stream.string.slice(0, stream.start).trim().match(/^(GET|POST|PUT|DELETE|HEAD)$/i))) {
      stream.match(/^\S+/);
      return "variableName.function";
    }
    if (c === '"') {
      stream.next();
      let escaped = false;
      for (let next = stream.next(); next !== undefined; next = stream.next()) {
        if (next === '"' && !escaped) break;
        escaped = !escaped && next === "\\";
      }
      // A string followed by ":" is a key.
      return stream.match(/^\s*:/, false) ? "propertyName" : "string";
    }
    if (stream.match(/^-?\d+(\.\d+)?([eE][+-]?\d+)?/)) return "number";
    if (stream.match(/^(true|false|null)\b/)) return "atom";
    stream.next();
    return null;
  },
};

export const cypherLanguage = StreamLanguage.define(cypher);
export const consoleLanguage = StreamLanguage.define(consoleMode);
