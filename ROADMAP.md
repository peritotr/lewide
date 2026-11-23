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
| PostgreSQL, CockroachDB | ✅ Supported | Browse, edit rows in a transaction, filter, change columns, import, export |
| MySQL, MariaDB | ✅ Supported | Same views as PostgreSQL |
| SQLite | ✅ Supported | Local files |
| SQL Server | ✅ Supported | Changing columns and query plans are not built yet |
| ClickHouse | ✅ Supported | Read-only grid and structure by design: its keys do not identify one row |
| Redis, Valkey | ✅ Supported | Key browser and editors for every core type; no TLS yet |
| MongoDB | ✅ Supported | Documents as tree, table or JSON; in-place edits |
| Qdrant | ✅ Supported | Vector search with payload filters; search by text needs an embedding provider |
| Neo4j | ✅ Supported | Graph canvas, table and text views; Cypher highlighting still to come |
| DuckDB | ✅ Ready | Embedded, built into the app; editing and column changes |

Across all of them: saved connections with passwords in the system
credential store, SSH tunnels, TLS, Production safeguards (a confirmation
before every write), English and Turkish, light and dark themes.

## Next

**Connection types** (order agreed one at a time):

- pgvector — vector search on PostgreSQL, reusing the vector view
- TiDB, YugabyteDB, TimescaleDB — they speak the MySQL or PostgreSQL protocol, so mostly testing
- Elasticsearch and OpenSearch — needs a search view that is not designed yet
- Oracle, Snowflake, BigQuery, InfluxDB, Couchbase, Milvus

**Finishing what exists**

- A connection form per engine (Redis database number, Qdrant API key, MongoDB authentication database, Neo4j database)
- Changing columns on SQL Server and ClickHouse; editing indexes, foreign keys and triggers everywhere
- Query plans for SQL Server; Cypher highlighting, Explain and Save for Neo4j
- Redis over TLS
- Text search for vector databases through an embedding provider
- Excel import and export
- Structured AI findings (review results shown in place, not only as text)

**Release**

- Signed and notarized installers for macOS, Windows and Linux
- Automatic updates
- Continuous integration for every supported engine

## Later

- iOS and Android, for the drivers that build without a native library
- More languages for the interface

## Not planned yet

Cluster administration, schema migration tooling and visual query builders
are out of scope for now. Suggestions are welcome as issues.
