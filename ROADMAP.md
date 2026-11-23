# Roadmap

LewIDE is a desktop client for every kind of database: relational,
document, key-value, vector and graph, in one interface, with an AI
assistant that only ever sees the structure of a database, never its rows.
It runs on macOS, Windows and Linux; mobile comes last.

This page says what works today, what comes next, and what is not planned
yet. It changes as work lands.

## Where it stands

| Database | Status | Notes |
|---|---|---|
| PostgreSQL, CockroachDB, YugabyteDB | ✅ Supported | Browse, edit rows in a transaction, filter, change columns, import, export |
| TimescaleDB | ✅ Supported | Same views as PostgreSQL; hypertables open like tables, chunk and policy views not built |
| MySQL, MariaDB, TiDB | ✅ Supported | Same views as PostgreSQL |
| SQLite | ✅ Supported | Local files |
| SQL Server | ✅ Supported | Changing columns and query plans are not built yet |
| ClickHouse | ✅ Supported | Read-only grid and structure by design: its keys do not identify one row |
| Redis, Valkey | ✅ Supported | Key browser and editors for every core type, TLS |
| MongoDB | ✅ Supported | Documents as tree, table or JSON; in-place edits |
| Elasticsearch, OpenSearch | ✅ Supported | Indices in the document view: Query DSL, field shorthand or query string; edits; Dev Tools requests in query tabs |
| pgvector | ✅ Supported | Tables with a vector column searched like a vector database; SQL as usual |
| Qdrant | ✅ Supported | Vector search with payload filters; search by text needs an embedding provider |
| Neo4j | ✅ Supported | Graph canvas, table and text views; Cypher highlighting still to come |
| DuckDB | ✅ Supported | Embedded, built into the app; editing and column changes |

Across all of them: saved connections with passwords in the system
credential store, SSH tunnels, TLS, Production safeguards (a confirmation
before every write), English and Turkish, light and dark themes.

## Next

**Connection types** (order agreed one at a time):

- Oracle, Snowflake, BigQuery, InfluxDB, Couchbase, Milvus

**Finishing what exists**

- Changing columns on SQL Server and ClickHouse; editing indexes, foreign keys and triggers everywhere
- Query plans for SQL Server; Cypher highlighting, Explain and Save for Neo4j
- Text search for vector databases through an embedding provider
- Excel import and export
- Structured AI findings (review results shown in place, not only as text)

**Release**

- Signed and notarized installers for macOS, Windows and Linux
- Automatic updates

## Later

- iOS and Android, for the drivers that build without a native library
- More languages for the interface

## Not planned yet

Cluster administration, schema migration tooling and visual query builders
are out of scope for now. Suggestions are welcome as issues.
