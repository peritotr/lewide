// Canned backend and state for reviewing the interface in a plain browser,
// loaded only by development builds outside the Tauri shell (see api.ts).
// The content is the sample scenario from the query editor design.
import type { Backend, DbObject, DriverInfo, QueryResult, Value } from "./api";
import { blankTab, loadPage, replaceState, runNow, type Conn, type Tab } from "../store";
import { splitStatements } from "./sql";

const caps = { sql: true, schemas: true, transactions: true, cancel: true, vector_search: false, file_based: false, tls: true };
const postgres: DriverInfo = { id: "postgres", name: "PostgreSQL", model: "relational", capabilities: caps, default_port: 5432, desktop_only: false };
const mysql: DriverInfo = { id: "mysql", name: "MySQL", model: "relational", capabilities: { ...caps, schemas: false }, default_port: 3306, desktop_only: false };
const sqlite: DriverInfo = { id: "sqlite", name: "SQLite", model: "relational", capabilities: { ...caps, schemas: false, file_based: true, tls: false }, default_port: null, desktop_only: false };
const store = { ...caps, sql: false, schemas: false, transactions: false, cancel: false };
const redis: DriverInfo = { id: "redis", name: "Redis", model: "key_value", capabilities: { ...store, sql: true }, default_port: 6379, desktop_only: false };
const mongodb: DriverInfo = { id: "mongodb", name: "MongoDB", model: "document", capabilities: store, default_port: 27017, desktop_only: false };
const qdrant: DriverInfo = { id: "qdrant", name: "Qdrant", model: "vector", capabilities: { ...store, vector_search: true }, default_port: 6333, desktop_only: false };
// PostgreSQL underneath, so it keeps SQL and transactions; the demo answers its vector calls like Qdrant's.
const pgvector: DriverInfo = { id: "pgvector", name: "pgvector", model: "vector", capabilities: { ...caps, vector_search: true }, default_port: 5432, desktop_only: false };
const neo4j: DriverInfo = { id: "neo4j", name: "Neo4j", model: "graph", capabilities: store, default_port: 7474, desktop_only: false };

/**
 * Each sample connection gets its own session number, so the catalog calls
 * can answer for the right kind of database. The relational ones share 1.
 */
const SESSIONS: Record<string, number> = { r1: 11, m1: 12, v1: 13, g1: 14 };
const REDIS = 11;
const MONGO = 12;
const QDRANT = 13;
const NEO4J = 14;

const TABLES = ["addresses", "carts", "categories", "coupons", "inventory", "order_items", "orders", "payments", "products", "reviews", "shipments", "users"];
const VIEWS = ["active_carts", "daily_revenue", "top_products"];
const ESTIMATES: Record<string, number> = {
  addresses: 2_900_000, carts: 412_000, categories: 86, coupons: 1_200, inventory: 48_000, order_items: 21_700_000,
  orders: 8_400_000, payments: 8_100_000, products: 12_400, reviews: 640_000, shipments: 7_900_000, users: 1_200_000,
};
const table = (name: string): DbObject => ({ name, kind: "table", row_estimate: ESTIMATES[name] ?? null });
const view = (name: string): DbObject => ({ name, kind: "view", row_estimate: null });

const CUSTOMERS: [number, string, string | null, number, string, string][] = [
  [48213, "maria.kovacs@fastmail.com", "HU", 14, "2184.50", "2026-11-21 18:42:07+00"],
  [10977, "d.okafor@proton.me", "NG", 11, "1967.00", "2026-11-22 07:15:44+00"],
  [77120, "liam.oconnor@gmail.com", "IE", 9, "1845.25", "2026-11-19 21:03:12+00"],
  [3391, "yuki.tanaka@icloud.com", "JP", 12, "1790.80", "2026-11-20 02:48:55+00"],
  [61508, "elif.demir@outlook.com", "TR", 8, "1612.40", "2026-11-21 11:27:31+00"],
  [29844, "s.petrova@yandex.com", null, 7, "1544.00", "2026-11-17 16:09:20+00"],
  [90112, "jonas.weber@gmx.de", "DE", 10, "1498.75", "2026-11-22 05:51:02+00"],
  [15563, "ana.souza@uol.com.br", "BR", 6, "1430.10", "2026-11-18 23:34:47+00"],
  [70231, "noah.martin@gmail.com", "US", 9, "1388.90", "2026-11-20 14:12:09+00"],
  [8842, "priya.raman@zoho.in", "IN", 13, "1342.60", "2026-11-21 04:40:18+00"],
  [53390, "chloe.dubois@orange.fr", "FR", 5, "1305.00", "2026-11-15 19:22:36+00"],
  [41276, "mateo.garcia@gmail.com", "ES", 7, "1271.35", "2026-11-19 09:58:41+00"],
  [66904, "h.lindqvist@telia.se", "SE", 4, "1198.00", "2026-11-12 13:05:27+00"],
  [22018, "omar.haddad@gmail.com", "AE", 6, "1154.45", "2026-11-20 20:31:50+00"],
  [83557, "zoe.baker@hey.com", "GB", 8, "1120.20", "2026-11-21 22:16:03+00"],
  [5714, "lucas.silva@gmail.com", "PT", 5, "1087.90", "2026-11-16 08:44:15+00"],
  [37462, "anna.nowak@wp.pl", "PL", 6, "1043.15", "2026-11-18 17:29:58+00"],
  [94120, "kim.jiwoo@naver.com", "KR", 7, "1011.70", "2026-11-22 01:07:33+00"],
  [12689, "t.nguyen@gmail.com", null, 4, "986.30", "2026-11-10 06:53:21+00"],
  [58031, "emma.jansen@ziggo.nl", "NL", 5, "954.80", "2026-11-14 15:38:46+00"],
];

const text = (value: string): Value => ({ type: "text", value });
const int = (value: number): Value => ({ type: "int", value: String(value) });

function customers(): QueryResult {
  const rows: Value[][] = [];
  for (let i = 0; i < 100; i++) {
    const [id, email, country, orders, revenue, last] = CUSTOMERS[i % CUSTOMERS.length];
    const lap = Math.floor(i / CUSTOMERS.length);
    rows.push([
      int(id + lap * 100_003),
      text(lap === 0 ? email : email.replace("@", `+${lap}@`)),
      country === null ? { type: "null" } : text(country),
      int(Math.max(1, orders - lap)),
      { type: "decimal", value: (Number(revenue) * 0.6 ** lap).toFixed(2) },
      { type: "temporal", value: last },
    ]);
  }
  return {
    columns: [
      { name: "id", type_name: "int8" },
      { name: "email", type_name: "text" },
      { name: "country", type_name: "bpchar" },
      { name: "orders", type_name: "int8" },
      { name: "revenue", type_name: "numeric" },
      { name: "last_order", type_name: "timestamptz" },
    ],
    rows,
    rows_affected: null,
    truncated: false,
    elapsed_ms: 171,
  };
}

function statuses(): QueryResult {
  const data: [string, number][] = [["paid", 6912408], ["shipped", 1104377], ["pending", 212950], ["refunded", 148221], ["cancelled", 61874]];
  return {
    columns: [{ name: "status", type_name: "text" }, { name: "orders", type_name: "int8" }],
    rows: data.map(([status, count]) => [text(status), int(count)]),
    rows_affected: null,
    truncated: false,
    elapsed_ms: 13,
  };
}

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

export const demoBackend: Backend = {
  listDrivers: async () => [mongodb, mysql, neo4j, pgvector, postgres, qdrant, redis, sqlite],
  openConnection: async () => 1,
  closeConnection: async () => {},
  runQuery: async (_session, statement) => {
    await wait(150);
    if (/pg_sleep/i.test(statement)) await wait(6000);
    if (/no_such_table/i.test(statement)) {
      throw { kind: "database", message: { code: "42P01", message: 'relation "no_such_table" does not exist', position: statement.toLowerCase().indexOf("no_such_table") + 1, line: null, hint: null, detail: null } };
    }
    if (/^\s*(update|insert|delete)\b/i.test(statement)) {
      return [{ columns: [], rows: [], rows_affected: 3, truncated: false, elapsed_ms: 6 }];
    }
    if (/where\s+false/i.test(statement)) return [{ ...customers(), rows: [], elapsed_ms: 9 }];
    if (/^\s*explain\s*\(format json\)/i.test(statement)) return [{ columns: [{ name: "QUERY PLAN", type_name: "json" }], rows: [[{ type: "json", value: DEMO_PLAN }]], rows_affected: null, truncated: false, elapsed_ms: 4 }];
    if (/^\s*explain\b/i.test(statement)) return [{ columns: [{ name: "QUERY PLAN", type_name: "text" }], rows: [[text("Limit  (cost=412600.00..412600.25 rows=100 width=72)")], [text("  ->  Sort  (cost=412600.00..413100.00 rows=200000 width=72)")]], rows_affected: null, truncated: false, elapsed_ms: 4 }];
    if (/truncated/i.test(statement)) return [{ ...customers(), truncated: true }];
    const sets: QueryResult[] = [];
    if (/users u/i.test(statement) || !/group by status/i.test(statement)) sets.push(customers());
    if (/group by status/i.test(statement)) sets.push(statuses());
    return sets;
  },
  cancelQuery: async () => {},
  serverVersion: async (session) => ({ [REDIS]: "7.4.1", [MONGO]: "8.0.4", [QDRANT]: "1.12.4", [NEO4J]: "5.26.0" })[session] ?? "16.2",
  currentNamespace: async (session) => ({ [REDIS]: "db0", [MONGO]: "catalog", [QDRANT]: null, [NEO4J]: "neo4j" } as Record<number, string | null>)[session] ?? "public",
  listNamespaces: async (session) =>
    ({ [REDIS]: ["db0", "db2"], [MONGO]: ["catalog", "analytics"], [QDRANT]: [], [NEO4J]: ["neo4j"] } as Record<number, string[]>)[session]?.map((name) => ({ name })) ?? [{ name: "audit" }, { name: "public" }],
  inTransaction: async () => false,
  listSavedQueries: async () => [
    { id: "s1", connection_id: "d1", name: "Daily revenue", sql: "SELECT 1;" },
    { id: "s2", connection_id: "d1", name: "Top customers", sql: DEMO_SQL },
  ],
  saveQuery: async (query) => ({ ...query, id: query.id || crypto.randomUUID() }),
  deleteSavedQuery: async () => {},
  listSaved: async () => [],
  saveConnection: async (connection) => ({
    ...connection,
    id: connection.id || crypto.randomUUID(),
    config: { ...connection.config, password: undefined },
    requires: { password: !!connection.config.password, ssh_password: false, ssh_passphrase: false },
    secrets: connection.config.password ? "keychain" : "none",
  }),
  deleteSaved: async () => {},
  connectSaved: async (id) => {
    await wait(600);
    return SESSIONS[id] ?? 1;
  },
  testConnection: async (config) => {
    await wait(900);
    if (/fail/i.test(config.host ?? "")) throw { kind: "connect", message: 'FATAL: password authentication failed for user "app_rw"' };
    if (/slow/i.test(config.host ?? "")) throw { kind: "timeout", message: 10 };
    return { version: "16.2", latency_ms: 38 };
  },
  listObjects: async (session, namespace): Promise<DbObject[]> => {
    if (session === MONGO) return [["products", 12408], ["orders", 84211], ["reviews", 6402]].map(([name, n]) => ({ name: String(name), kind: "collection", row_estimate: Number(n) }));
    if (session === QDRANT) return [{ name: "articles", kind: "collection", row_estimate: 12400 }];
    if (session === REDIS || session === NEO4J) return [];
    return namespace === "audit" ? [table("events")] : [...TABLES.map(table), ...VIEWS.map(view)];
  },
  describeObject: async (_session, name) => ({
    name,
    columns: ["id", "email", "country", "created_at"].map((column, i) => ({ name: column, type_name: "text", nullable: i > 0, default_value: null, primary_key: i === 0 })),
  }),
  previewChanges: async (_session, changes) => changes.changes.map((c) => (c.kind === "insert" ? `INSERT INTO public.${changes.table} (…) VALUES (…)` : c.kind === "update" ? `UPDATE public.${changes.table} SET … WHERE id = …` : `DELETE FROM public.${changes.table} WHERE id = …`)),
  applyChanges: async (_session, changes) => {
    await wait(300);
    if (changes.changes.some((c) => c.kind !== "delete" && (c.kind === "insert" ? c.values : c.set).some((cell) => cell.value.type === "text" && cell.value.value === "fail"))) {
      throw { kind: "database", message: { code: "23514", message: 'new row for relation "users" violates check constraint "users_email_check"', position: null, line: null, hint: null, detail: null } };
    }
    return { statements: [] };
  },
  fetchRows: async (_session, request) => {
    await wait(120);
    const all = customers();
    const rows = all.rows.slice(request.offset, request.offset + request.limit);
    return {
      columns: all.columns.map((c, i) => ({ ...c, nullable: i === 2, default_value: null, primary_key: i === 0 })),
      rows,
      offset: request.offset,
      has_more: request.offset + request.limit < all.rows.length,
      elapsed_ms: 12,
    };
  },
  tableStructure: async (_session, name) => ({
    name,
    columns: ["id", "email", "country", "created_at"].map((column, i) => ({ name: column, type_name: i === 0 ? "bigint" : "text", nullable: i > 0, default_value: null, primary_key: i === 0 })),
    indexes: [
      { name: `${name}_pkey`, columns: ["id"], unique: true, primary: true, method: "btree", size: "26 MB" },
      { name: `${name}_email_key`, columns: ["email"], unique: true, primary: false, method: "btree", size: "41 MB", condition: "(email IS NOT NULL)" },
    ],
    foreign_keys: [
      { name: `${name}_country_fkey`, columns: ["country"], referenced_namespace: null, referenced_table: "countries", referenced_columns: ["code"], on_update: "NO ACTION", on_delete: "RESTRICT" },
    ],
    referenced_by: [{ name: "orders_user_id_fkey", namespace: null, table: "orders", columns: ["user_id"], referenced_columns: ["id"], on_update: "NO ACTION", on_delete: "CASCADE" }],
    triggers: [{ name: `${name}_touch`, timing: "BEFORE", events: ["INSERT", "UPDATE"], definition: `CREATE TRIGGER ${name}_touch BEFORE INSERT OR UPDATE ON public.${name} FOR EACH ROW EXECUTE FUNCTION touch()` }],
    ddl: `CREATE TABLE public.${name} (\n    id bigint NOT NULL,\n    email text,\n    country text,\n    created_at text,\n    CONSTRAINT ${name}_pkey PRIMARY KEY (id)\n);`,
  }),
  previewStructure: async (_session, set) =>
    set.changes.map((c) =>
      c.kind === "add" ? `ALTER TABLE public.${set.table} ADD COLUMN ${c.column.name} ${c.column.type_name}${c.column.nullable ? "" : " NOT NULL"}` : c.kind === "drop" ? `ALTER TABLE public.${set.table} DROP COLUMN ${c.column}` : `ALTER TABLE public.${set.table} ALTER COLUMN ${c.from.name} TYPE ${c.to.type_name}`,
    ),
  applyStructure: async (_session, set) => {
    await wait(300);
    return { statements: set.changes.map(() => "…") };
  },
  exportTable: async () => {
    await wait(400);
    return 100;
  },
  exportRows: async (_columns, rows) => rows.length,
  // A plain rendering; the real text comes from the core.
  exportPreview: async (columns, rows, format) =>
    format.format === "csv"
      ? [columns.join(format.delimiter ?? ","), ...rows.map((row) => row.map((v) => (v.type === "null" ? "" : "value" in v ? String(v.value) : "")).join(format.delimiter ?? ","))].join("\n")
      : rows.map((row) => JSON.stringify(Object.fromEntries(columns.map((c, i) => [c, "value" in row[i] ? row[i].value : null])))).join("\n"),
  importInspect: async () => ({ columns: ["id", "email", "country", "signup"], sample: [["9001", "new1@example.com", "TR", "2026-11-01"], ["9002", "new2@example.com", "DE", null], ["x", "broken@example.com", "US", "2026-11-03"]], total_rows: 1240, bytes: 48213 }),
  importCheck: async (_session, _target, _path, _format, mapping) => ({
    total_rows: 1240,
    checked: 3,
    errors: 1,
    unfilled: ["created_at"],
    rows: [
      { line: 2, values: mapping.map((m, i) => (i === 0 ? { type: "int", value: "9001" } : { type: "text", value: `value ${m.source}` })), texts: mapping.map(() => "…"), error: null },
      { line: 3, values: mapping.map((m, i) => (i === 0 ? { type: "int", value: "9002" } : { type: "text", value: `value ${m.source}` })), texts: mapping.map(() => "…"), error: null },
      { line: 4, values: [], texts: mapping.map((_, i) => (i === 0 ? "x" : "…")), error: { line: 4, column: mapping[0]?.column ?? null, value: "x", reason: "is not a whole number (bigint)" } },
    ],
  }),
  importRun: async () => {
    await wait(900);
    return { inserted: 1237, skipped_count: 3, skipped: [{ line: 4, column: "id", value: "x", reason: "is not a whole number (bigint)" }], elapsed_ms: 612 };
  },
  kvKeyspace: async () => ({ keys: 48214, memory_bytes: 64277709, databases: [[0, 48214]] }),
  kvScan: async (_session, _db, pattern) => ({
    cursor: "0",
    keys: [
      { key: "session:8f2c1a", kind: "string", ttl_ms: 1394000 },
      { key: "cart:48213", kind: "hash", ttl_ms: null },
      { key: "queue:emails", kind: "list", ttl_ms: null },
      { key: "tags:popular", kind: "set", ttl_ms: 41000 },
      { key: "leaderboard:week", kind: "zset", ttl_ms: 273600000 },
    ].filter((k) => !pattern || pattern === "*" || k.key.includes(pattern.replace(/\*/g, ""))),
  }),
  kvRead: async (_session, _db, key) => {
    const head = { key, ttl_ms: key.startsWith("session") ? 1394000 : null, size_bytes: 312, encoding: "embstr" };
    if (key.startsWith("cart")) return { ...head, kind: "hash", data: { kind: "hash", total: 2, fields: [["sku-1", { type: "text", value: "2" }], ["sku-9", { type: "text", value: "1" }]] } };
    if (key.startsWith("queue")) return { ...head, kind: "list", data: { kind: "list", start: 0, total: 3, items: ["welcome:48213", "receipt:10977", "reset:77120"].map((value) => ({ type: "text", value })) } };
    if (key.startsWith("tags")) return { ...head, kind: "set", data: { kind: "set", cursor: "0", total: 3, members: ["sale", "new", "gift"].map((value) => ({ type: "text", value })) } };
    if (key.startsWith("leader")) return { ...head, kind: "zset", data: { kind: "zset", start: 0, total: 2, members: [[{ type: "text", value: "ada" }, 2184.5], [{ type: "text", value: "linus" }, 1967]] } };
    return { ...head, kind: "string", data: { kind: "string", value: { type: "text", value: '{"user_id": 48213, "roles": ["admin"], "expires": "2026-11-23T10:00:00Z"}' } } };
  },
  kvWrite: async (_session, _db, command) => {
    await wait(120);
    return command.op.toUpperCase();
  },
  docCollections: async () => [{ name: "products", kind: "collection", document_count: 12408, avg_document_bytes: 412, size_bytes: 5112096 }],
  docFind: async (_session, request) => {
    await wait(150);
    const int = (n: number) => ({ $numberInt: String(n) });
    const documents = [
      { _id: { $oid: "65f1c0a2b7e4d3a1c0ffee01" }, name: "Walnut desk", price: { amount: { $numberDouble: "429.5" }, currency: "EUR" }, stock: int(14), tags: ["office", "wood"], created: { $date: { $numberLong: "1700000000000" } }, discontinued: null },
      { _id: { $oid: "65f1c0a2b7e4d3a1c0ffee02" }, name: "Task lamp", price: { amount: { $numberDouble: "64" }, currency: "EUR" }, stock: int(230), tags: ["lighting"], created: { $date: { $numberLong: "1701000000000" } } },
      { _id: { $oid: "65f1c0a2b7e4d3a1c0ffee03" }, name: "Monitor arm", price: { amount: { $numberDouble: "119.9" }, currency: "EUR" }, stock: int(0), tags: [], dimensions: { w: int(12), h: int(48), parts: [{ name: "clamp" }, { name: "arm" }] } },
    ];
    return { skip: request.skip, has_more: false, elapsed_ms: 12, documents: documents.map((document) => ({ document, size_bytes: 412 })) };
  },
  docCount: async () => 3,
  docWrite: async () => ({ matched: 1, modified: 1, deleted: 0, inserted_id: null }),
  graphRun: async () => {
    await wait(200);
    const person = (i: number, name: string) => ({ id: `n${i}`, labels: ["Person"], properties: { name } });
    const nodes = [person(1, "Ada"), person(2, "Charles"), person(3, "Grace"), { id: "n4", labels: ["Company"], properties: { name: "Analytical Engines" } }, { id: "n5", labels: ["City"], properties: { name: "London" } }];
    const rel = (i: number, type: string, start: string, end: string) => ({ id: `r${i}`, type, start, end, properties: {} });
    const relationships = [rel(1, "KNOWS", "n1", "n2"), rel(2, "KNOWS", "n1", "n3"), rel(3, "WORKS_AT", "n1", "n4"), rel(4, "WORKS_AT", "n2", "n4"), rel(5, "LOCATED_IN", "n4", "n5")];
    return [{ columns: ["a", "r", "b"], rows: relationships.map((r) => [{ kind: "node" as const, ...nodes.find((n) => n.id === r.start)! }, { kind: "relationship" as const, ...r }, { kind: "node" as const, ...nodes.find((n) => n.id === r.end)! }]), truncated: false, graph: { nodes, relationships }, node_count: 5, relationship_count: 5, nodes_omitted: 0, relationships_omitted: 0, counters: null, elapsed_ms: 64 }];
  },
  graphNeighbours: async () => ({ nodes: [], relationships: [], total: 0, groups: [{ type: "KNOWS", outgoing: true, count: 2 }] }),
  graphLabels: async () => [
    [
      { name: "City", count: 1 },
      { name: "Company", count: 1 },
      { name: "Person", count: 3 },
    ],
    [
      { name: "KNOWS", count: 2 },
      { name: "LOCATED_IN", count: 1 },
      { name: "WORKS_AT", count: 2 },
    ],
  ],
  vectorCollections: async () => ["articles"],
  vectorCollectionInfo: async (_session, name) => ({ name, status: "green", points: 12400, vectors: [{ name: null, kind: "dense", size: 4, distance: "Cosine", higher_is_better: true }], payload_fields: [{ name: "lang", kind: "keyword", points: 12400 }] }),
  vectorSearch: async (_session, request) => {
    await wait(200);
    const titles = ["Indexing strategies for large tables", "Choosing a distance metric", "Backups that restore", "Connection pooling explained"];
    return {
      below_min_score: 2,
      elapsed_ms: 38,
      hits: titles.slice(0, request.limit).map((title, i) => ({
        rank: i + 1,
        score: 0.91 - i * 0.07,
        id: String(1042 + i * 17),
        payload: request.with_payload ? { title, lang: "en", tags: ["database", i % 2 ? "ops" : "performance"], summary: `${title}: a longer text that describes what the article covers and why it matters.` } : null,
        vectors: [],
      })),
    };
  },
  vectorPoint: async (_session, _collection, id) => ({ id, payload: { title: "Indexing strategies for large tables", lang: "en", tags: ["database", "performance"] }, vectors: [{ name: null, kind: "dense", values: [0.12, -0.48, 0.33, 0.8] }] }),
  countRows: async () => {
    await wait(400);
    return 100;
  },
};

const DEMO_PLAN = [
  {
    Plan: {
      "Node Type": "Limit", "Total Cost": 412600.25, "Plan Rows": 100,
      Plans: [
        {
          "Node Type": "Sort", "Sort Key": ["(sum(o.total_cents))"], "Total Cost": 412600.0, "Plan Rows": 200000,
          Plans: [
            {
              "Node Type": "Aggregate", Strategy: "Hashed", "Total Cost": 398200.0, "Plan Rows": 200000,
              Plans: [
                {
                  "Node Type": "Hash Join", "Hash Cond": "(o.user_id = u.id)", "Total Cost": 361400.0, "Plan Rows": 1840000,
                  Plans: [
                    { "Node Type": "Seq Scan", "Relation Name": "orders", Alias: "o", Filter: "(status = 'paid')", "Total Cost": 298400.0, "Plan Rows": 1840000 },
                    { "Node Type": "Hash", "Total Cost": 31200.0, "Plan Rows": 1200000, Plans: [{ "Node Type": "Seq Scan", "Relation Name": "users", Alias: "u", "Total Cost": 31200.0, "Plan Rows": 1200000 }] },
                  ],
                },
              ],
            },
          ],
        },
      ],
    },
  },
];

export const DEMO_SQL = `-- Top customers by paid revenue, last 30 days
SELECT u.id, u.email, u.country,
       COUNT(o.id) AS orders,
       SUM(o.total_cents) / 100.0 AS revenue,
       MAX(o.created_at) AS last_order
FROM users u
JOIN orders o ON o.user_id = u.id
WHERE o.status = 'paid'
  AND o.created_at >= now() - interval '30 days'
GROUP BY u.id
ORDER BY revenue DESC
LIMIT 100;

-- Order volume by status
SELECT status, COUNT(*) AS orders
FROM orders
GROUP BY status;
`;

/** Fills the store with the design's scenario: a Production connection and its tabs. */
export function seedDemo() {
  const objects = { public: [...TABLES.map(table), ...VIEWS.map(view)] };
  const conn = (id: string, name: string, environment: Conn["environment"], colour: Conn["colour"], connected: boolean): Conn => ({
    id,
    name,
    engine: "Pg",
    driver: postgres,
    config: { driver: "postgres", host: `${name}.internal`, user: "app", database: "shop" },
    environment,
    colour,
    requires: { password: true, ssh_password: false, ssh_passphrase: false },
    secretPolicy: "keychain",
    columns: {},
    txMode: "auto",
    tx: null,
    savedQueries: connected && id === "d1" ? [{ id: "s1", connection_id: "d1", name: "Daily revenue", sql: "SELECT 1;" }] : [],
    status: connected ? "connected" : "disconnected",
    lastError: null,
    session: connected ? 1 : null,
    version: connected ? "16.2" : null,
    namespaces: connected ? ["audit", "public"] : [],
    namespace: connected ? "public" : null,
    objects: connected ? objects : {},
  });
  const other = (id: string, name: string, driver: DriverInfo, engine: string, environment: Conn["environment"], colour: Conn["colour"], config: { host: string; database?: string }): Conn => ({
    ...conn(id, name, environment, colour, false),
    engine,
    driver,
    config: { driver: driver.id, user: "app", ...config },
  });
  const tab = (id: string, kind: Tab["kind"], title: string, connectionId: string, sql: string): Tab => ({
    ...blankTab(connectionId, { kind, title, sql }),
    id,
  });
  replaceState({
    drivers: [mongodb, mysql, neo4j, pgvector, postgres, qdrant, redis, sqlite],
    connections: [
      conn("d1", "acme-prod", "production", 1, true),
      conn("d2", "acme-staging", "staging", 2, true),
      conn("d3", "reporting", "staging", 4, true),
      conn("d4", "local-dev", "local", 3, false),
      // The sample connections of the non-relational design: double-click to open their views.
      other("r1", "sessions", redis, "Rd", "local", 1, { host: "cache.internal", database: "0" }),
      other("m1", "catalog-docs", mongodb, "Mg", "local", 2, { host: "docs.internal", database: "catalog" }),
      other("v1", "embeddings", qdrant, "Qd", "staging", 4, { host: "vectors.internal" }),
      other("g1", "shop-graph", neo4j, "N4", "staging", 3, { host: "graph.internal", database: "neo4j" }),
    ],
    activeConnectionId: "d1",
    expanded: { d1: true, "d1\u0000public": true, "d1\u0000public\u0000tables": true },
    tabs: [
      tab("q1", "query", "Top customers", "d1", DEMO_SQL),
      { ...tab("q2", "table-data", "users", "d1", ""), table: { namespace: "public", name: "users", page: null, offset: 0, pageSize: 50, sort: null, total: null, loading: false, error: null, mode: "data", filter: "", filterMode: "builder", conditions: [], hidden: [], subTab: "columns", structure: null, pending: { inserts: [], updates: {}, deletes: {} }, undo: [], apply: { state: "idle" }, columnEdits: [], structureApply: { state: "idle" } } },
      tab("q3", "query", "Query 2", "d2", "SELECT 1;"),
    ],
    activeTabId: "q1",
  });
  void runNow("q1", splitStatements(DEMO_SQL).map((s) => ({ text: s.text, from: s.from })));
  void loadPage("q2");
}
