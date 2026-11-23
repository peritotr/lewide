# LewIDE — notes for AI coding assistants

This file tells an AI assistant (and any new contributor) how the project is built, why, and where it stands. Human-facing docs are [README.md](README.md), [CONTRIBUTING.md](CONTRIBUTING.md) and [ROADMAP.md](ROADMAP.md).

## Working rules

- UI source text is English; translations live in `src/i18n/<lang>.json` (`tr.json` for Turkish).
- Never use real credentials. Test passwords (`test`, `testtest1`, `dbclient-test-key`, `Test-pass1`) are only for the throwaway local Docker containers started by `scripts/test-*.sh`.
- Screens come from design specs in `docs/design/spec/00–10*.md`. Implement them at high fidelity and say plainly when something is not drawn in the design and was added.
- For a new connector: write a short plan and get it approved before coding; ask instead of guessing when something is unclear; run the tests when it is done and update the Connectors section below.
- Do not make structural changes (new dependencies, changes to shared traits or the shared editor) without the maintainer's approval.
- Do not commit or push unless asked.

## 1. Purpose and architecture

LewIDE is a desktop database client for macOS, Windows and Linux (mobile later): every kind of database (relational, document, key-value, vector, graph), multilingual, with an AI assistant.

Stack: **Tauri 2**, Rust core, **React 19 + TypeScript + Vite + Tailwind v4**, zustand, CodeMirror 6, @tanstack/react-virtual, own i18n. Package manager pnpm.

```
Cargo.toml                 workspace: members = ["crates/*", "src-tauri"]; licence and repository in [workspace.package]
crates/core                traits and shared types (no I/O)
  driver.rs                Driver / Connection traits; optional accessors key_value(), document(), vector(), graph()
  kv.rs document.rs vector.rs graph.rs    one trait + types per non-relational model
  export.rs import.rs structure.rs edit.rs   CSV/JSON/SQL export, import, ALTER, row changes
crates/driver-sqlite | driver-postgres | driver-pgvector | driver-mysql | driver-elasticsearch | driver-sqlserver | driver-clickhouse | driver-duckdb | driver-influxdb | driver-redis | driver-mongodb | driver-couchbase | driver-qdrant | driver-milvus | driver-neo4j
crates/tunnel              SSH tunnel; wraps a Connection and delegates the accessors
crates/tls                 the rustls client configuration every TLS-speaking driver uses (roots, CA file, check against another name)
crates/store               saved connections and queries, secrets in the OS keychain
crates/ai                  AI providers (Anthropic, OpenAI, Gemini, OpenAI-compatible); no embeddings yet
src-tauri/src/lib.rs       AppState, driver registry, every #[tauri::command], handlers!() list, IPC tests
src-tauri/src/bin/bridge.rs  development HTTP bridge (see "Running")
src/store.ts               app state (connections, tabs, dialogs); src/prefs.ts, src/ai-store.ts
src/lib/api.ts             Backend interface: tauri backend, bridge, demo backend (src/lib/demo.ts)
src/lib/types.ts           the type picker: which database types exist and which driver serves each
src/lib/*.ts               pure helpers, each with a *.test.ts (edit, filter, diff, fuzzy, sql, languages, plan, kv, vector, ejson, graph-layout, ai); editor-modes.ts holds the Cypher and console colours
src/components/            TableView, StructureView, DataGrid, KeyValueView, VectorView, DocumentView, GraphView, Sidebar, TabBar, dialogs…
src/i18n/                  t(), tn(), tx(), msg(); tr.json
docs/design/spec/          design specs 00–10 (09 = non-relational views)
docs/assets/               README images (banner, tour, screenshots, social preview)
scripts/test-*.sh          throwaway Docker servers for live tests
scripts/readme-art.py      regenerates the README banners
```

A tab has a `kind` (`query | table-data | ai-chat | key-value | vector | document | graph`) and one matching state field (`table`, `keys`, `vector`, `doc`, `graph`). `App.tsx` picks the view from that field. On connect, `store.ts` opens the right tab for the driver's `model`.

### Running and checking

```bash
pnpm install
pnpm tauri dev                      # the real app
pnpm dev                            # UI only at http://localhost:1420 with the demo backend
cargo run -p dbclient --features bridge --bin bridge 1430   # then open http://localhost:1420/?bridge=1430 : UI in a browser against the real core
```

The bridge is how UI work is verified against real databases. It keeps its saved connections in a temp folder per run, so they vanish on restart.

Checks that must stay green before reporting work as done:

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
pnpm tsc --noEmit && pnpm vitest run && pnpm i18n:check && pnpm build
```

Last full run (2026-11-23): 329 Rust tests, 74 frontend tests, clippy clean, translations 1045/1045, build ok. Live suites of the newest drivers: Milvus 3.0.6 (7), InfluxDB 3.12.3 Core (6), Couchbase Community 8.0.2 (6). The PostgreSQL and MySQL live suites (18 tests each) passed against PostgreSQL 17, CockroachDB, YugabyteDB 2025.2, TimescaleDB 2.25, MySQL 8.4, MariaDB 11 and TiDB 8.5.

Live driver tests are skipped (and pass) unless their environment variable is set:

| Script | Loads |
|---|---|
| `scripts/test-servers.sh up` then `. target/test-servers/env.sh` (`compat` adds MariaDB, CockroachDB, TiDB, YugabyteDB and TimescaleDB; run the PostgreSQL suite with `--test-threads=1` against CockroachDB and YugabyteDB) | PostgreSQL, MySQL, SSH |
| `eval "$(scripts/test-redis.sh up)"`, `… tls` | `DBCLIENT_TEST_REDIS_URL`, `…_TLS_URL`, `…_TLS_CA` |
| `eval "$(scripts/test-mongodb.sh up)"` | `DBCLIENT_TEST_MONGODB_URL`, `…_CA` |
| `eval "$(scripts/test-qdrant.sh up)"`, `… tls` | `DBCLIENT_TEST_QDRANT_URL`, `…_API_KEY`, `…_TLS_URL`, `…_TLS_CA` |
| `eval "$(scripts/test-neo4j.sh up)"` | `DBCLIENT_TEST_NEO4J_URL` |
| `eval "$(scripts/test-elasticsearch.sh up)"`, `… open` | `DBCLIENT_TEST_ELASTICSEARCH_URL`, `…_USER`, `…_PASSWORD` (8.19, security on), `DBCLIENT_TEST_OPENSEARCH_URL` (2.19, security plugin off); 512 MB heap each |
| `eval "$(scripts/test-pgvector.sh up)"` | `DBCLIENT_TEST_PGVECTOR_URL` (PostgreSQL 17 with pgvector 0.8.1) |
| none: DuckDB is built in; `crates/driver-duckdb/tests/files.rs` always runs on temporary files | — |
| `eval "$(scripts/test-clickhouse.sh up)"` | `DBCLIENT_TEST_CLICKHOUSE_URL` (arm64 image, starts in seconds) |
| `eval "$(scripts/test-sqlserver.sh up)"` | `DBCLIENT_TEST_SQLSERVER_URL` (x86 image through Rosetta on Apple silicon, about a minute to start; capped at 2 GB) |
| `eval "$(scripts/test-milvus.sh up)"` | `DBCLIENT_TEST_MILVUS_URL`, `…_TOKEN` (standalone with embedded etcd and local storage, authorization on; loads `articles` and `products`) |
| `eval "$(scripts/test-influxdb.sh up)"` | `DBCLIENT_TEST_INFLUXDB_URL`, `…_TOKEN` (in-memory storage; the admin token is made by the server at start; writes `sensors.home` and `garden.soil`) |
| `eval "$(scripts/test-couchbase.sh up)"` | `DBCLIENT_TEST_COUCHBASE_URL` (query service), `…_USER`, `…_PASSWORD` (one-node Community cluster, couchstore bucket `shop` without replicas; `sales.orders` loaded, `sales.drafts` empty for writes) |

Always run `… down` afterwards. CI (`.github/workflows/ci.yml`) runs the checks on three systems and the live tests in an `engines` job.

## 2. Decisions and why

- **Rust core, thin UI.** Drivers, export/import, ALTER generation and AI requests live in Rust so secrets and database traffic never pass through the webview. API keys are write-only from the UI.
- **One crate per driver, one trait per data model.** Non-relational engines do not fit `QueryResult`, so each model has its own trait (`KeyValue`, `DocumentStore`, `VectorStore`, `GraphStore`) reached through an optional accessor on `Connection`. A new driver touches only its crate, plus registration.
- **Prefer pure-Rust clients**, so drivers can later build for iOS and Android. Native code (DuckDB) is marked `desktop_only`.
- **Own protocol code where it is small.** Redis speaks RESP2 through our own encoder (no crate); Qdrant and Neo4j use their HTTP APIs with reqwest (Neo4j: Query API v2 with typed JSON, not Bolt; needs Neo4j 5.19+). MongoDB uses the official `mongodb` crate.
- **SQL Server through `tiberius` 0.13** (TDS, rustls). Editor text is sent as batches split on `GO` lines. The result stream carries no row counts, so the driver appends `SELECT @@ROWCOUNT AS [__lewide_rowcount]` to each batch and hides that result, except for batches that must stand alone (`CREATE/ALTER PROCEDURE|FUNCTION|TRIGGER|VIEW|SCHEMA`). The dialect is `SqlDialect::SqlServer` (`[name]` quoting, `N''` strings, `0x` bytes, 1/0 for bit, `BEGIN TRANSACTION`); paging uses `OFFSET … FETCH` in core's default `fetch_rows`. The test container accepts Microsoft's licence (`ACCEPT_EULA=Y`).
- **DuckDB is built into the app** (`duckdb` crate 1.20001.0 = DuckDB 2.0.1, `default-features = false`, features `bundled`, `parquet`, `json`). It compiles from the source the crate carries (about 3 minutes on the first build, nothing downloaded) and makes the app larger; ICU is left out, so TIMESTAMPTZ values are shown in UTC with `+00`. Extension auto-download is off (`enable_autoload_extension(false)`): the app never fetches code; a user can still run `INSTALL`/`LOAD`. Like SQLite: a file path, calls on a blocking thread, `cancel` interrupts, a new file only with `params.create = "true"`. Row and column editing are on (`SqlDialect::DuckDb` uses PostgreSQL's ALTER forms). Two DuckDB quirks the driver works around: `is_autocommit()` does not follow BEGIN/COMMIT, so the driver tracks transactions itself; and `duckdb_schemas()` marks `main` as internal, so system schemas are excluded by name. DDL and DML answer with a "Count" column; the driver reports them as commands unless the statement starts with a row-returning keyword.
- **ClickHouse through its HTTP interface** (port 8123, `reqwest`, the TLS module copied from driver-qdrant). HTTP keeps no session, so the driver names the database on every request and remembers `USE` itself. Text is cut into statements in the driver. Results come as JSONCompact with 64-bit integers and decimals quoted. An error after rows have started arrives inside a 200 reply as `"exception"`; the driver turns it into an error (this is also how a cancelled query shows). Cancel runs `KILL QUERY` on the request's `query_id`. The grid is read-only (a ClickHouse key does not identify one row; UPDATE/DELETE are background mutations), so columns never claim `primary_key`, and `apply_changes` accepts new rows only, sent as one INSERT so an import's retry cannot write rows twice. Structure is read-only. Imports need no column (ClickHouse fills a column left out with its type's default: `check_mapping(…, implicit_defaults)`), and the "one transaction" switch is hidden for drivers without transactions.
- **Elasticsearch and OpenSearch share one driver** (`crates/driver-elasticsearch`, REST over `reqwest`, the TLS module copied from driver-clickhouse; bulk-approved 2026-11-23). No new view: they use the document view. The cluster is the one database, indices are collections (dot-prefixed system indices hidden), a document is `_source` with `_id` in front. The filter field takes Query DSL, a `{"field": value}` shorthand (match per field, lists as `terms`, `null` as missing) or Lucene query-string text (`city:İzmir AND year:>2023`); projection and sort take Elasticsearch's forms and MongoDB's `{field: 1}`. Paging uses `from`/`size`, so it stops at 10,000 hits. Writes use `refresh=wait_for`; inserts with an `_id` go through `_create` (a taken id is refused); edits read the document, apply the view's `$set` paths (lists by index) and write it back with `if_seq_no`/`if_primary_term`, so a concurrent change is refused instead of lost. The view's editors write MongoDB Extended JSON (`{"$numberDouble": "1.5"}`); the driver turns it into plain JSON before writing (found while trying it: an edited number would otherwise have been stored as an object). Auth: user and password (Basic) or the `apiKey` driver option. Query tabs take Kibana Dev Tools console text (`GET /index/_search` + body; NDJSON for `_bulk`/`_msearch`); search replies become `_id`, `_index`, `_score`, `_source` rows, `_cat` replies one row per object.
- **pgvector is its own driver over driver-postgres** (`crates/driver-pgvector`, approved in the bulk list of 2026-11-23). It opens a PostgreSQL connection and wraps it: SQL, transactions and structure pass through; `vector()` shows tables with a `vector` column as collections, named by table, with the schema in front outside the current schema (`rag.chunks`). A point is a row identified by the single-column primary key (tables without one are refused); each `vector` column is a named vector; the payload is `to_jsonb(row)` minus the key and the vectors. pgvector keeps no metric on a column, so the metric comes from the operator class of the column's index (`vector_cosine_ops`, `_l2_`, `_ip_`, `_l1_`), else from the `metric` driver option (Advanced tab: `cosine`, `l2`, `ip`, `l1`), else cosine. Scores: cosine similarity and inner product are turned into higher-is-better; L2 and L1 stay distances. Filters are SQL over `to_jsonb(row)` with the vector view's rules (a list matches when an item does; `Ne` matches missing values); values are quoted literals, identifiers go through `quote_ident`. The data model is `vector`, so the sidebar lists vector tables only and the vector view opens on connect; other tables are reached through query tabs. Server version reads `0.8.1 (PostgreSQL 17.8)`.
- **Milvus through its RESTful API v2** (`crates/driver-milvus`, `reqwest` on port 19530, approved 2026-11-23). Every reply is HTTP 200 with a `code` (0 is success; 1800 is a refused user). A point is an entity: the primary key is the id (Int64 or VarChar), each vector field a named vector, the other fields plus the dynamic ones the payload; array fields arrive wrapped (`{"Data": {"StringData": {"data": […]}}}`) and are unwrapped. The vector view's conditions become a Milvus expression (`expr.rs`): arrays use `array_contains`/`array_contains_any`, keys outside the schema `$meta["key"]`, and `Ne` adds `not exists` so it also matches points without the field. Listing pages by the key (`key > last` with `orderByFields: ["key:asc"]`), not by offset, so it is not held to Milvus's 16,384 limit. Metric from the index; COSINE, IP and BM25 are higher-is-better, L2 (squared), HAMMING and Jaccard are distances. Auth: `user:password` as a bearer token, or the password alone as a cloud API key when the user is empty; the database comes from the form (default `default`). Found while building it: `PointId::parse` read the text key `"007"` as the number 7, so a digit string with a leading zero is now text (core change, approved with the connector).
- **InfluxDB 3 through its HTTP API** (`crates/driver-influxdb`, `/api/v3/query_sql`, port 8181, Bearer token). Databases are namespaces (`_internal` hidden), tables come from `information_schema` (schema `iox`); `USE` is remembered by the driver because HTTP keeps no session, and the server takes one statement per request, so the text is split (a copy of DuckDB's splitter). Replies are JSON objects in SELECT order where a null value is left out of its row: the grid fills every column from the table's description, and query tabs take columns in the order they first appear (a column that is null in every row is not shown). Pages are ordered by time, then tags, so they do not shift. The dialect is `SqlDialect::DuckDb` (PostgreSQL quoting, no backslash escapes). Read-only: InfluxDB is written with line protocol, and a row has no key. The structure marks each column `time`, `tag` (a dictionary column) or `field` in its comment.
- **Couchbase through the query service** (`crates/driver-couchbase`, `POST /query/service`, port 8093/18093, Basic auth). One port does everything: buckets and collections from `system:keyspaces` (the bucket entry stands for `_default._default`), documents through SQL++ with positional parameters. In the document view a bucket is a database and a collection is `scope.collection`; the key is `_id`. Filters take a `{"field": value}` shorthand (MongoDB's `$gt`, `$in`, `$exists`…; a list field matches when it holds the value; `null` matches a missing field) or SQL++ that follows `WHERE`; sort takes `{field: ±1}` or `ORDER BY` text; projection is applied in the driver. Edits read the CAS and write with `WHERE META(d).cas = …`, so a concurrent change is refused; `$set` paths are applied to the read document in Rust (SQL++'s `SET a.b` does nothing when `a` is missing). Requests use `scan_consistency: request_plus`, and an exact count always scans (`WHERE META(d).id IS NOT MISSING`): without a condition SQL++ answers COUNT(*) from the collection's statistics, which lagged behind a delete in CI. Query tabs run SQL++; object rows become columns, other values one `value` column.
- **Compatible engines reuse drivers.** CockroachDB, YugabyteDB and TimescaleDB use driver-postgres; MariaDB and TiDB use driver-mysql; Valkey uses driver-redis; the live tests run against them too. The driver recognises the engine from its version text and adjusts only where it must (the version shown, system schemas, TiDB's transaction state through `@@tidb_current_ts`, TiDB's "line N column M" error form). The picker's types carry the engine's usual port, user and database as hints (`src/lib/types.ts`).
- **Edits are explicit.** Relational edits stay pending and apply in one transaction. Key-value and document edits apply at once, one command each, and show the command. On a Production connection every write asks first.
- **Keys are listed with SCAN, never KEYS.** Counts are estimates and say so.
- **MongoDB documents travel as canonical Extended JSON**, so an edit keeps the stored type (`src/lib/ejson.ts`).
- **Vector and graph ids are strings on the wire** (64-bit integers would lose precision in JavaScript).
- **`serde_json` uses `preserve_order`** (declared in `crates/core/Cargo.toml`): documents and imported JSON keep their order.
- **Minimum score for vector search is applied in the driver**, so the UI can say how many were left out.
- **Own i18n instead of a library:** source text is the key; `scripts/i18n-check.mjs` finds missing, unused and placeholder-broken entries.
- **Demo backend:** in a plain browser the UI runs on canned data, so screens can be built and reviewed without a database. It has sample connections for every data model (Postgres, Redis, MongoDB, Qdrant, Neo4j); the README screenshots are taken from it.
- **App identifier `io.github.peritotr.lewide`.** The keychain service name is the identifier, so secrets saved under an earlier identifier are not found.
- **Licence MIT OR Apache-2.0**, copyright "LewIDE contributors".

## 3. Code standards

- **Comments and UI text in plain English**, explaining why, not what. No commented-out code.
- **Rust:** errors are `dbclient_core::Error` variants (`Connect`, `Database { code, message, position, … }`, `Query`, `Unsupported(&'static str)`, `InvalidConfig`, `ConnectionLost`, `Timeout`, `File`); never `unwrap` outside tests. A command follows the pattern `state.session(id)` → call → `state.or_lost(session, result)`. Every new command must be added to `handlers!()`. `cargo fmt` and clippy with `-D warnings` are mandatory.
- **TypeScript:** no `any`; shared types live in `src/lib/api.ts` and mirror the Rust serde shapes (snake_case fields). Every `Backend` method needs three entries: the tauri backend, the `api` wrapper, and `src/lib/demo.ts`.
- **UI text:** always through `t("…")`, `tn(count, one, other)`, `tx` or `msg`. After adding text run `node scripts/i18n-check.mjs --add tr`, fill the empty entries in `tr.json`, then `pnpm i18n:check`.
- **Styling:** design tokens only (`src/tokens.css`, exposed as Tailwind classes such as `bg-panel`, `border-line`, `text-text-2`, `h-row`). No new colours. Selected rows: `bg-accent-tint` plus `inset 2px 0 0 var(--accent)`. Flex children that hold long rows (the tab bar) need `min-w-0`, or they widen the window.
- **Logging:** there is none by design; errors reach the user as messages. Do not add `println!` or `console.log`.
- **Tests:** pure logic gets unit tests next to it; each driver has `tests/live.rs` gated on its environment variable; `src-tauri` has IPC tests on Tauri's mock runtime. UI behaviour is checked through the bridge against a real server before it is called done. Say exactly what was and was not verified.
- **Editing files:** read the file and use precise edits. Scripted search-and-replace over large files has failed repeatedly in this project.

## 4. Connectors

| Type (picker code) | Driver crate | Backend | UI | Verified in the UI against a real server |
|---|---|---|---|---|
| PostgreSQL (Pg), CockroachDB (Cr) | driver-postgres | done | done: grid, editing, structure, import/export | yes |
| YugabyteDB (Yb) | driver-postgres | done (version text, LSM index method) | done (PostgreSQL's views) | YugabyteDB 2025.2.0.0: connect with a password, version "2025.2.0.0 (PostgreSQL 15.12)", tree, table with Turkish text, cell edit applied in one transaction, structure with the `lsm` index |
| TimescaleDB (Ts) | driver-postgres | done (version text, extension schemas hidden) | done (PostgreSQL's views; a hypertable is a table) | TimescaleDB 2.25.0 on PostgreSQL 17.7: connect, version "2.25.0 (PostgreSQL 17.7)", only user schemas in the tree, a hypertable read across chunks with its two-column key, WHERE filter, cell edit applied in one transaction |
| MySQL (My), MariaDB (Ma) | driver-mysql | done | done (same views) | MySQL 8.4: connect, table, cell edit with Turkish text applied in one transaction, WHERE filter, structure, DDL (MariaDB: driver live tests only) |
| TiDB (Ti) | driver-mysql | done (version text, transaction state, error line, capitalised system schemas) | done (MySQL's views) | TiDB 8.5.5: connect, version "8.5.5 (MySQL 8.0.11)", system schemas hidden, cell edit with Turkish text applied in one transaction, WHERE filter, structure, multi-statement script with DDL, syntax error pointing at its line |
| SQLite (Sq) | driver-sqlite | done | done | SQLite 3.54: open a file, table, cell edit applied and written to the file, structure |
| DuckDB (Dk) | driver-duckdb | done | done: the relational views, editing and column changes | DuckDB 2.0.1 (bundled): open a file, tree, multi-statement run with Parser Error and line/column, messages with row counts, paging, cell edit applied in one transaction, WHERE filter, exact count, structure and DDL, renaming a column, cancel; import and SQL/CSV export through the bridge's commands; EXPLAIN |
| ClickHouse (Ch) | driver-clickhouse | done (no TLS test, no ALTER) | done: grid and structure read-only | ClickHouse 25.8: connect, tree with row estimates, multi-statement run with error code and position, cancel, table in sorting-key order, WHERE filter, exact count, structure, import and SQL/CSV export through the bridge's commands |
| SQL Server (Ms) | driver-sqlserver | done (no ALTER, no cancel) | done: structure is read-only | SQL Server 2022: connect, tree, multi-statement run with error code and line, paging, cell edit and apply, WHERE filter, exact count, structure, Explain message, Format; import and SQL/CSV export through the bridge's commands |
| Redis (Rd), Valkey (Vk) | driver-redis | done, with TLS | done: `KeyValueView.tsx` (K01–K05) | Redis 7.4: database number from the form, scan, read every type, hash field edit, save a string (KEEPTTL), set TTL, rename, add key, delete key, Production confirmation; TLS through the driver's live tests |
| Elasticsearch (Es), OpenSearch (Os) | driver-elasticsearch | done (no aggregation view) | done: `DocumentView.tsx` with Elasticsearch wording, `apiKey` option, Export | Elasticsearch 8.19.7: connect with a password, indices with counts, tree, query-string filter, number edit stored as a number, Dev Tools request in a query tab with rows. OpenSearch 2.19.3: connect without security, indices with counts, a slow console search stopped with Stop ("Query cancelled") |
| pgvector (Pv) | driver-pgvector (over driver-postgres) | done (read-only vector view; SQL as PostgreSQL) | done: `VectorView.tsx`, `metric` option in the form | PostgreSQL 17.8 + pgvector 0.8.1: connect, vector tables in the sidebar by schema, collection info (size, metric from the hnsw index, payload fields), search by point id with a payload filter, point panel |
| Qdrant (Qd) | driver-qdrant | done (read-only by design) | done: `VectorView.tsx` (V01–V02) | Qdrant 1.20: API key from the form, search by vector and by point id, payload filter, point panel, Find similar |
| MongoDB (Mg) | driver-mongodb | done | done: `DocumentView.tsx` (D01–D03), Export, Indexes | MongoDB 8.0: root user with `authSource` from the Advanced tab, tree, Table and JSON views, paging, filter, in-place edit, add document (types kept), delete document, Production confirmation; index list with sizes, JSON and CSV export (nested fields as JSON text, dates as ISO), sidebar count refreshed after an insert |
| Neo4j (N4) | driver-neo4j | done | done: `GraphView.tsx` (R01–R02), sidebar labels and relationship types | multi-statement run, canvas, selection and side panel, drag, pan, zoom, Fit, expand, hide/show, Table and Text views, syntax error with caret, write counters, sidebar labels/types |
| InfluxDB 3 (If) | driver-influxdb | done (read-only, no cancel) | done: the relational views, grid and structure read-only | InfluxDB 3.12.3 Core: connect with a token, version, two databases in the tree, table with NULL cells and Turkish text in time order, four-statement script with `USE` and a syntax error pointed at line 5 column 3 |
| Couchbase (Cb) | driver-couchbase | done | done: `DocumentView.tsx` with SQL++ wording, Bucket field | Couchbase Community 8.0.2: connect, buckets and `scope.collection` with counts, tree, SQL++ filter, in-place edit stored as a number |
| Milvus (Mv) | driver-milvus | done (read-only by design) | done: `VectorView.tsx` | Milvus 3.0.6: connect with `root` and password, collections with counts, search by point id, point panel, payload filter, named vectors (L2 shown as "nearest first"), text key `007` |
| Oracle, Snowflake, BigQuery | — | not started | listed in the picker as "not supported yet" | — |

Known limits and untried paths:

- **Neo4j:** graph tabs use the shared editor in Cypher mode (colours from a stream parser in `editor-modes.ts`, no completion). Explain runs `EXPLAIN` on the statement at the caret and shows the server's plan text (Query API `queryPlan`, typed JSON) in a Query plan view; Format lays out one clause per line (`formatCypher`, a simple word-based formatter: unusual Cypher may come out unchanged); Save keeps the Cypher as a saved query that reopens in a graph tab. Not tried: the `USE <database>` prefix for another database, relationship selection (not designed). Added beyond the design: double-clicking a label or relationship type opens a `MATCH … LIMIT 50` graph tab.
- **DuckDB:** DuckDB refuses ALTER on a table that a view or foreign key depends on (its own error is shown); no "create a new file" option in the form; `https://` files need `INSTALL httpfs`; the plan view is text only. Not tried: Production, a locked file, `ATTACH`.
- **ClickHouse:** not tried: TLS against a real certificate, the SSH tunnel in the UI, Production, ClickHouse Cloud, clusters. The editor highlights with the MySQL grammar; the filter uses `startsWith` for "starts with".
- **SQL Server:** left out on purpose: column editing, query plans, cancel, Windows/Azure AD sign-in, named instances. Not tried: TLS against a real certificate, the SSH tunnel in the UI, Production. Statement splitting in `src/lib/sql.ts` is a scanner, so unusual T-SQL can still be split wrongly.
- **MongoDB:** the sidebar's document count is the server's estimate (refreshed after an insert or delete from the view). Not built: syntax colours in query fields, change-type menu, nested-value viewer, aggregation, index editing. A `null` value cannot be edited. Document export (all document drivers): JSON keeps each document whole (MongoDB's canonical Extended JSON); CSV takes the top-level fields of the page on screen, so a field that only appears further down is left out; Elasticsearch stops at its 10,000-hit window.
- **Redis:** streams and module types are read-only. Key prefixes are not grouped.
- **Qdrant:** text search needs an embedding provider. No Export.
- **Elasticsearch / OpenSearch:** query tabs use the editor's console mode (method lines, JSON bodies; Run takes the request under the caret; Format indents bodies and keeps NDJSON one object per line; no Explain). Only unit-tested, not tried in the UI yet. Plain JSON numbers are labelled `double` in the tree (the label comes from MongoDB's types); without a sort, an edited document moves to the end of the list (Elasticsearch's own order); aggregations show as one JSON cell. Cancel: each console request carries an `X-Opaque-Id`, and Stop cancels the tasks with that id through the Tasks API; the server notices between segments, so a search stops a little after Stop, and a request whose task cannot be cancelled (index, bulk) runs to its end. The test containers run with disk watermarks off, so a nearly full Docker disk does not make them read-only. Not tried: TLS, the SSH tunnel, an API key.
- **pgvector:** only `vector` columns (not `halfvec`, `sparsevec`, `bit`); tables need a one-column primary key; SQL completion in query tabs only knows the vector tables (the sidebar's object list is what it reads); a failing vector query inside the user's open transaction aborts that transaction, as any PostgreSQL error does. Not tried: TLS and the SSH tunnel in the UI (the PostgreSQL driver's own tests cover them).
- **TiDB:** no triggers (TiDB has none); a column default is shown as written (`0`, where MySQL shows `0.00`); row estimates stay 0 until TiDB has gathered statistics. Not tried: TLS, the SSH tunnel, TiDB Cloud, a cluster with TiKV (the test server uses TiDB's built-in storage).
- **YugabyteDB:** index sizes read 0 bytes (the PostgreSQL size functions do not see YugabyteDB's storage); `pg_terminate_backend` makes the server restart every session, so that live test is skipped there. The single-node test server needs 4 GB, relaxed clock checks and free disk: on a loaded machine it restarted itself several times while being tried (killed backends, "clock skew" stop, "insufficient disk space" at 99 % full). Not tried: TLS, the SSH tunnel, a multi-node cluster, YugabyteDB Aeon.
- **TimescaleDB:** hypertables are shown as ordinary tables; chunks, compression, continuous aggregates and retention policies have no view of their own (they are reachable through SQL). The extension's schemas (`_timescaledb_*`, `timescaledb_information`, `timescaledb_experimental`) are hidden from the tree. Not tried: Timescale Cloud, TLS, the SSH tunnel.
- **Milvus:** read-only; no server version (the REST API does not report one); binary and int8 vectors are shown raw; `<`/`>` conditions on array fields are refused (Milvus cannot compare items). Not tried: TLS, Zilliz Cloud with an API key, the SSH tunnel, a collection that is not loaded (search then fails with the server's message).
- **InfluxDB:** InfluxDB 1.x/2.x (InfluxQL, Flux) are not supported; no InfluxQL in query tabs; no writes (Import fails with "writing to InfluxDB"); no row estimates in the tree (counting reads every file); a query-tab column that is null in every row is missing from the result. Not tried: Enterprise, TLS, the SSH tunnel, Cloud.
- **Couchbase:** the query service only (no KV API), so a key lookup is a SQL++ statement; a stored field named `_id` is hidden behind the key; documents that are not objects show as `{"_id", "value"}`; `$set` only (no `$unset` from the view); the editor highlights plain SQL. Not tried: Enterprise, Capella, TLS, the SSH tunnel, a multi-node cluster.
- **SQLite:** an `INTEGER PRIMARY KEY` column shows as nullable, as SQLite's own table info says, although it cannot hold NULL (it is the row id). Left as is on purpose: the import dialog treats non-nullable columns without a default as required, which would wrongly require the id.

## 5. Next steps

See [ROADMAP.md](ROADMAP.md) for the public list. In more detail:

1. **Next connectors**: Oracle (needs Oracle's client library), Snowflake and BigQuery (need accounts) — each waits for the owner's decision. For each: crate with `Driver` + `Connection` (+ model trait), `tests/live.rs`, `scripts/test-<name>.sh up|down`, registration in `src-tauri`, `driver:` in `src/lib/types.ts`, demo data, view, translations, bridge check, CI `engines` entry.
2. **Done 2026-11-23:** the shared editor takes a `language` (`sql` with a dialect, `cypher`, `console`; `src/lib/languages.ts`); Couchbase query tabs use the MySQL grammar for SQL++ colours.
3. **SQL Server follow-ups** (each needs approval): column editing in T-SQL, query plans, named instances.
4. **Housekeeping:** done on 2026-11-23: the nine copies of the rustls setup are one crate, `crates/tls` (`client_config(tls, check_as)`; `check_as` is the name to check when the socket goes to a tunnel). Fixed on 2026-11-23: at the window's minimum width the views no longer widen past the pane (their grid column is `minmax(0,1fr)`), and the table toolbar drops its button words below 720 px (not drawn in the design).
5. **Release:** v0.1.0 is published (2026-11-23). Pushing a `v*` tag runs `.github/workflows/release.yml`: tauri-action builds macOS (arm64, x64; ad-hoc signed), Windows (NSIS, MSI) and Linux (AppImage, deb, rpm) into a draft release, which is published by hand. For v0.1.0 the jobs could not create the release (`Resource not accessible by integration` although the token had `contents: write`); the draft was created with `gh release create --draft` and the jobs re-run, after which uploads worked. The permission is now on the job; if the next tag fails the same way, the repository's Settings → Actions → Workflow permissions must be set to read and write (the owner's decision). Still to do, each needing the owner's decision: paid code signing (Apple, Windows), automatic updates (needs an updater key in the repository secrets).

## 6. Known problems

- **MongoDB authentication database:** the driver uses `params["authSource"]`, else the connection's database, else `admin`. A root user with a database filled in needs `authSource = admin` in the form's Advanced tab.
- **MongoDB with a CA file trusts only that file**, not the public roots as well, which differs from the comment on `TlsConfig.ca_file`.
- **MongoDB `route`** (used by the SSH tunnel) runs a small in-process SOCKS5 listener on loopback; `mongodb+srv://` and several hosts are not supported.
- **Neo4j:** TLS never tested against a TLS server; no transactions and no cancel; `max_rows` truncates after the download.
- **Qdrant:** `ping` uses `/healthz`, which needs no key, so a revoked key shows only on the next real call.
- **JSON import order:** columns come in file order, not alphabetical (a result of `preserve_order`).
- Other gaps: index, foreign key and trigger editing; AI structured findings (AI05/AI14); Excel import/export; embeddings for vector text search; no splash screen.
