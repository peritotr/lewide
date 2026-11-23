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
crates/driver-sqlite | driver-postgres | driver-mysql | driver-sqlserver | driver-clickhouse | driver-duckdb | driver-redis | driver-mongodb | driver-qdrant | driver-neo4j
crates/tunnel              SSH tunnel; wraps a Connection and delegates the accessors
crates/store               saved connections and queries, secrets in the OS keychain
crates/ai                  AI providers (Anthropic, OpenAI, Gemini, OpenAI-compatible); no embeddings yet
src-tauri/src/lib.rs       AppState, driver registry, every #[tauri::command], handlers!() list, IPC tests
src-tauri/src/bin/bridge.rs  development HTTP bridge (see "Running")
src/store.ts               app state (connections, tabs, dialogs); src/prefs.ts, src/ai-store.ts
src/lib/api.ts             Backend interface: tauri backend, bridge, demo backend (src/lib/demo.ts)
src/lib/types.ts           the type picker: which database types exist and which driver serves each
src/lib/*.ts               pure helpers, each with a *.test.ts (edit, filter, diff, fuzzy, sql, plan, kv, vector, ejson, graph-layout, ai)
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

Last full run: 254 Rust tests, 74 frontend tests, clippy clean, translations 1028/1028, build ok.

Live driver tests are skipped (and pass) unless their environment variable is set:

| Script | Loads |
|---|---|
| `scripts/test-servers.sh up` then `. target/test-servers/env.sh` (`compat` adds MariaDB and CockroachDB) | PostgreSQL, MySQL, SSH |
| `eval "$(scripts/test-redis.sh up)"` | `DBCLIENT_TEST_REDIS_URL` |
| `eval "$(scripts/test-mongodb.sh up)"` | `DBCLIENT_TEST_MONGODB_URL`, `…_CA` |
| `eval "$(scripts/test-qdrant.sh up)"`, `… tls` | `DBCLIENT_TEST_QDRANT_URL`, `…_API_KEY`, `…_TLS_URL`, `…_TLS_CA` |
| `eval "$(scripts/test-neo4j.sh up)"` | `DBCLIENT_TEST_NEO4J_URL` |
| none: DuckDB is built in; `crates/driver-duckdb/tests/files.rs` always runs on temporary files | — |
| `eval "$(scripts/test-clickhouse.sh up)"` | `DBCLIENT_TEST_CLICKHOUSE_URL` (arm64 image, starts in seconds) |
| `eval "$(scripts/test-sqlserver.sh up)"` | `DBCLIENT_TEST_SQLSERVER_URL` (x86 image through Rosetta on Apple silicon, about a minute to start; capped at 2 GB) |

Always run `… down` afterwards. CI (`.github/workflows/ci.yml`) runs the checks on three systems and the live tests in an `engines` job.

## 2. Decisions and why

- **Rust core, thin UI.** Drivers, export/import, ALTER generation and AI requests live in Rust so secrets and database traffic never pass through the webview. API keys are write-only from the UI.
- **One crate per driver, one trait per data model.** Non-relational engines do not fit `QueryResult`, so each model has its own trait (`KeyValue`, `DocumentStore`, `VectorStore`, `GraphStore`) reached through an optional accessor on `Connection`. A new driver touches only its crate, plus registration.
- **Prefer pure-Rust clients**, so drivers can later build for iOS and Android. Native code (DuckDB) is marked `desktop_only`.
- **Own protocol code where it is small.** Redis speaks RESP2 through our own encoder (no crate); Qdrant and Neo4j use their HTTP APIs with reqwest (Neo4j: Query API v2 with typed JSON, not Bolt; needs Neo4j 5.19+). MongoDB uses the official `mongodb` crate.
- **SQL Server through `tiberius` 0.13** (TDS, rustls). Editor text is sent as batches split on `GO` lines. The result stream carries no row counts, so the driver appends `SELECT @@ROWCOUNT AS [__lewide_rowcount]` to each batch and hides that result, except for batches that must stand alone (`CREATE/ALTER PROCEDURE|FUNCTION|TRIGGER|VIEW|SCHEMA`). The dialect is `SqlDialect::SqlServer` (`[name]` quoting, `N''` strings, `0x` bytes, 1/0 for bit, `BEGIN TRANSACTION`); paging uses `OFFSET … FETCH` in core's default `fetch_rows`. The test container accepts Microsoft's licence (`ACCEPT_EULA=Y`).
- **DuckDB is built into the app** (`duckdb` crate 1.20001.0 = DuckDB 2.0.1, `default-features = false`, features `bundled`, `parquet`, `json`). It compiles from the source the crate carries (about 3 minutes on the first build, nothing downloaded) and makes the app larger; ICU is left out, so TIMESTAMPTZ values are shown in UTC with `+00`. Extension auto-download is off (`enable_autoload_extension(false)`): the app never fetches code; a user can still run `INSTALL`/`LOAD`. Like SQLite: a file path, calls on a blocking thread, `cancel` interrupts, a new file only with `params.create = "true"`. Row and column editing are on (`SqlDialect::DuckDb` uses PostgreSQL's ALTER forms). Two DuckDB quirks the driver works around: `is_autocommit()` does not follow BEGIN/COMMIT, so the driver tracks transactions itself; and `duckdb_schemas()` marks `main` as internal, so system schemas are excluded by name. DDL and DML answer with a "Count" column; the driver reports them as commands unless the statement starts with a row-returning keyword.
- **ClickHouse through its HTTP interface** (port 8123, `reqwest`, the TLS module copied from driver-qdrant). HTTP keeps no session, so the driver names the database on every request and remembers `USE` itself. Text is cut into statements in the driver. Results come as JSONCompact with 64-bit integers and decimals quoted. An error after rows have started arrives inside a 200 reply as `"exception"`; the driver turns it into an error (this is also how a cancelled query shows). Cancel runs `KILL QUERY` on the request's `query_id`. The grid is read-only (a ClickHouse key does not identify one row; UPDATE/DELETE are background mutations), so columns never claim `primary_key`, and `apply_changes` accepts new rows only, sent as one INSERT so an import's retry cannot write rows twice. Structure is read-only. Imports need no column (ClickHouse fills a column left out with its type's default: `check_mapping(…, implicit_defaults)`), and the "one transaction" switch is hidden for drivers without transactions.
- **Compatible engines reuse drivers.** CockroachDB uses driver-postgres, MariaDB uses driver-mysql, Valkey uses driver-redis; the live tests run against them too.
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
| MySQL (My), MariaDB (Ma) | driver-mysql | done | done (same views) | not through the bridge; driver live tests only |
| SQLite (Sq) | driver-sqlite | done | done | not recorded; assume untested |
| DuckDB (Dk) | driver-duckdb | done | done: the relational views, editing and column changes | DuckDB 2.0.1 (bundled): open a file, tree, multi-statement run with Parser Error and line/column, messages with row counts, paging, cell edit applied in one transaction, WHERE filter, exact count, structure and DDL, renaming a column, cancel; import and SQL/CSV export through the bridge's commands; EXPLAIN |
| ClickHouse (Ch) | driver-clickhouse | done (no TLS test, no ALTER) | done: grid and structure read-only | ClickHouse 25.8: connect, tree with row estimates, multi-statement run with error code and position, cancel, table in sorting-key order, WHERE filter, exact count, structure, import and SQL/CSV export through the bridge's commands |
| SQL Server (Ms) | driver-sqlserver | done (no ALTER, no cancel) | done: structure is read-only | SQL Server 2022: connect, tree, multi-statement run with error code and line, paging, cell edit and apply, WHERE filter, exact count, structure, Explain message, Format; import and SQL/CSV export through the bridge's commands |
| Redis (Rd), Valkey (Vk) | driver-redis | done, no TLS | done: `KeyValueView.tsx` (K01–K05) | connect, scan by pattern, read every type, hash field edit |
| Qdrant (Qd) | driver-qdrant | done (read-only by design) | done: `VectorView.tsx` (V01–V02) | connect, search by vector, point panel |
| MongoDB (Mg) | driver-mongodb | done | done: `DocumentView.tsx` (D01–D03) | connect, tree, filter, invalid-filter line, in-place edit |
| Neo4j (N4) | driver-neo4j | done | done: `GraphView.tsx` (R01–R02), sidebar labels and relationship types | multi-statement run, canvas, selection and side panel, drag, pan, zoom, Fit, expand, hide/show, Table and Text views, syntax error with caret, write counters, sidebar labels/types |
| Oracle, Snowflake, BigQuery, TiDB, YugabyteDB, Couchbase, pgvector, Milvus, TimescaleDB, InfluxDB, Elasticsearch, OpenSearch | — | not started | listed in the picker as "not supported yet" | — |

Known limits and untried paths:

- **Neo4j:** the editor is a plain textarea (no Cypher highlighting; the shared `Editor` is SQL-only); `Explain`, `Format` and `Save` are not built. Not tried: the `USE <database>` prefix for another database, relationship selection (not designed). Added beyond the design: double-clicking a label or relationship type opens a `MATCH … LIMIT 50` graph tab.
- **DuckDB:** DuckDB refuses ALTER on a table that a view or foreign key depends on (its own error is shown); no "create a new file" option in the form; `https://` files need `INSTALL httpfs`; the plan view is text only. Not tried: Production, a locked file, `ATTACH`.
- **ClickHouse:** not tried: TLS against a real certificate, the SSH tunnel in the UI, Production, ClickHouse Cloud, clusters. The editor highlights with the MySQL grammar; the filter uses `startsWith` for "starts with".
- **SQL Server:** left out on purpose: column editing, query plans, cancel, Windows/Azure AD sign-in, named instances. Not tried: TLS against a real certificate, the SSH tunnel in the UI, Production. Statement splitting in `src/lib/sql.ts` is a scanner, so unusual T-SQL can still be split wrongly.
- **MongoDB:** not tried in the UI: Table and JSON views, add/delete document, paging, Production confirmation. Not built: syntax colours in query fields, change-type menu, nested-value viewer, Export, aggregation, index listing. A `null` value cannot be edited.
- **Redis:** not tried in the UI: save a string, rename, delete, TTL, add key, Production confirmation. Streams and module types are read-only. Key prefixes are not grouped.
- **Qdrant:** not tried in the UI: payload filter, search by point id, "Find similar". Text search needs an embedding provider. No Export.

## 5. Next steps

See [ROADMAP.md](ROADMAP.md) for the public list. In more detail:

1. **Click through the untested paths** above for MongoDB, Redis and Qdrant on real servers.
2. **Connection form per engine:** it still shows the relational fields for every type. Needed: Redis database number and TLS, Qdrant API key, MongoDB authentication database and replica set, Neo4j database (`src/components/connections/ConnectionForm.tsx`, "Driver options" tab).
3. **Next connectors**, suggested order: pgvector (driver-postgres plus `VectorStore`, reusing `VectorView`), then Elasticsearch / OpenSearch (needs a search view that is not designed yet). For each: crate with `Driver` + `Connection` (+ model trait), `tests/live.rs`, `scripts/test-<name>.sh up|down`, registration in `src-tauri`, `driver:` in `src/lib/types.ts`, demo data, view, translations, bridge check, CI `engines` entry.
4. **Cypher in the shared editor** (needs approval: makes `src/components/Editor.tsx` language-neutral), then `Explain`, `Format` and `Save` for graph tabs.
5. **SQL Server follow-ups** (each needs approval): column editing in T-SQL, query plans, named instances.
6. **Housekeeping:** the four copies of the rustls setup (postgres, qdrant, neo4j, clickhouse) could move into one shared helper.
7. **Release:** installers for the three systems through CI, code signing.

## 6. Known problems

- **MongoDB authentication database:** the driver uses `params["authSource"]`, else the connection's database, else `admin`. A root user with a database filled in fails to log in, and the form cannot set `authSource`.
- **MongoDB with a CA file trusts only that file**, not the public roots as well, which differs from the comment on `TlsConfig.ca_file`.
- **MongoDB `route`** (used by the SSH tunnel) runs a small in-process SOCKS5 listener on loopback; `mongodb+srv://` and several hosts are not supported.
- **Neo4j:** TLS never tested against a TLS server; no transactions and no cancel; `max_rows` truncates after the download.
- **Qdrant:** `ping` uses `/healthz`, which needs no key, so a revoked key shows only on the next real call.
- **Redis:** no TLS.
- **JSON import order:** columns come in file order, not alphabetical (a result of `preserve_order`).
- **The status bar and toolbar still speak SQL** ("auto-commit", "open transaction") on non-relational tabs.
- Other gaps: index, foreign key and trigger editing; AI structured findings (AI05/AI14); Excel import/export; embeddings for vector text search; no splash screen.
