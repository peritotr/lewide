import { describe, expect, it } from "vitest";
import { formatConsole, formatCypher, languageOf, splitConsole, splitCypher, statementAtFor } from "./languages";

describe("splitCypher", () => {
  it("cuts on semicolons outside strings, names and comments", () => {
    const text = "MATCH (n {name: 'a;b'}) RETURN n;\n// x;\nMATCH (`we;ird`) RETURN \"c\\\";\" /* ; */;";
    expect(splitCypher(text).map((s) => s.text)).toEqual(["MATCH (n {name: 'a;b'}) RETURN n", "MATCH (`we;ird`) RETURN \"c\\\";\""]);
    expect(splitCypher("  RETURN 1  ")[0]).toEqual({ text: "RETURN 1", from: 2, to: 10 });
    expect(splitCypher("// only a comment")).toEqual([]);
  });
});

describe("splitConsole", () => {
  it("makes one request per METHOD line, with its body", () => {
    const text = "# indices\nGET /_cat/indices\n\nPOST /shop/_search\n{\n  \"query\": {\"match_all\": {}}\n}\n\n";
    const requests = splitConsole(text);
    expect(requests.map((r) => r.text)).toEqual(["GET /_cat/indices", "POST /shop/_search\n{\n  \"query\": {\"match_all\": {}}\n}"]);
    expect(text.slice(requests[1].from, requests[1].to)).toBe(requests[1].text);
  });

  it("finds the request under the caret", () => {
    const text = "GET /a\n\nGET /b\n{}";
    expect(statementAtFor(text, text.indexOf("{}"), { kind: "console" })?.text).toBe("GET /b\n{}");
    expect(statementAtFor(text, 2, { kind: "console" })?.text).toBe("GET /a");
  });
});

describe("formatConsole", () => {
  it("indents JSON bodies and keeps NDJSON one object per line", () => {
    expect(formatConsole('GET /a/_search\n{"query":{"match_all":{}}}\nPOST /_bulk\n{ "index": {} }\n{ "n": 1 }\nGET  /b')).toBe(
      'GET /a/_search\n{\n  "query": {\n    "match_all": {}\n  }\n}\n\nPOST /_bulk\n{"index":{}}\n{"n":1}\n\nGET  /b',
    );
    expect(formatConsole("GET /a\nnot json")).toBe("GET /a\nnot json");
  });
});

describe("languageOf", () => {
  it("picks the language of each driver", () => {
    expect(languageOf("neo4j", "postgres")).toEqual({ kind: "cypher" });
    expect(languageOf("elasticsearch", "postgres")).toEqual({ kind: "console" });
    expect(languageOf("couchbase", "postgres")).toEqual({ kind: "sql", dialect: "mysql" });
    expect(languageOf("mysql", "mysql")).toEqual({ kind: "sql", dialect: "mysql" });
  });
});

describe("formatCypher", () => {
  it("puts each clause on its own line and capitalises keywords", () => {
    expect(formatCypher("match (p:Person)-[:KNOWS]->(f) where p.name = 'match me' and f.age > 30 return f.name as name order by name limit 5")).toBe(
      "MATCH (p:Person)-[:KNOWS]->(f)\nWHERE p.name = 'match me' AND f.age > 30\nRETURN f.name AS name\nORDER BY name\nLIMIT 5",
    );
  });

  it("keeps optional match, strings, names and statements apart", () => {
    expect(formatCypher("optional  match (n) return `where`; create (x {t: \"return\"})")).toBe("OPTIONAL MATCH (n)\nRETURN `where`;\n\nCREATE (x {t: \"return\"})");
  });
});
