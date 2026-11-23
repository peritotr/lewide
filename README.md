<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/banner-dark.svg">
  <img src="docs/assets/banner-light.svg" width="100%" alt="LewIDE — the open-source desktop client for every database: PostgreSQL, MySQL, SQLite, SQL Server, ClickHouse, DuckDB, Redis, MongoDB, Qdrant, pgvector, Elasticsearch and Neo4j">
</picture>

<h3>One free, open-source app for every database you work with.</h3>

[![CI](https://github.com/peritotr/lewide/actions/workflows/ci.yml/badge.svg)](https://github.com/peritotr/lewide/actions/workflows/ci.yml)
[![License: MIT or Apache-2.0](https://img.shields.io/badge/license-MIT%20%7C%20Apache--2.0-416180)](#license)
[![Platforms](https://img.shields.io/badge/macOS%20%C2%B7%20Windows%20%C2%B7%20Linux-desktop-5980a6)](#install)
[![Built with Tauri](https://img.shields.io/badge/Tauri%202-Rust%20%2B%20React-24C8DB?logo=tauri&logoColor=white)](https://tauri.app)
[![GitHub stars](https://img.shields.io/github/stars/peritotr/lewide?style=flat&color=e0b46a)](https://github.com/peritotr/lewide/stargazers)

[Features](#features) · [Databases](#supported-databases) · [Screenshots](#a-closer-look) · [Install](#install) · [Roadmap](ROADMAP.md) · [Contributing](CONTRIBUTING.md) · [Türkçe](README.tr.md)

</div>

**LewIDE** is a free, open-source **database client and SQL editor** for
macOS, Windows and Linux. It opens relational databases, document stores,
key-value stores, vector databases and graph databases in one fast desktop
app, with an **AI assistant that reads your schema but never your rows**. If
you have been switching between DBeaver, TablePlus, Navicat, DataGrip,
MongoDB Compass, RedisInsight, Neo4j Browser and Kibana Dev Tools, LewIDE puts them in one place.

<div align="center">
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/demo-dark.webp">
  <img src="docs/assets/demo-light.webp" width="100%" alt="A tour of LewIDE: SQL editor with results, AI assistant, table data, table structure, Redis keys, MongoDB documents, Qdrant vector search and a Neo4j graph">
</picture>
</div>

## Features

| | |
|---|---|
| 🗂️ **Every kind of database** | SQL, documents, key-value, vectors and graphs, each in a view made for it: a data grid, a document tree, a key browser, a vector search, a graph canvas. |
| ✍️ **A real SQL editor** | Highlighting and completion for your tables, run the statement under the cursor or all of them, errors pointed at the exact place, query plans, formatting, saved queries. |
| 🛡️ **Edits you can trust** | Changes stay pending until you apply them in one transaction, with the SQL shown first. A Production connection is marked everywhere and asks before every write. |
| 🧱 **Structure at a glance** | Columns, indexes, foreign keys (both ways), triggers and the DDL; change columns without writing ALTER TABLE by hand. |
| 🤖 **An AI that respects your data** | Explain, review and optimize queries with Anthropic, OpenAI, Gemini or any OpenAI-compatible server, Ollama included. Only table and column names are sent, and you can see exactly what. |
| 🔐 **Secure connections** | Passwords in the system keychain, never in a file. SSH tunnels and TLS for every network database. |
| 📦 **Import and export** | Import CSV, JSON and Excel with a column mapping, a preview and a report of the rows that did not fit; export to CSV, JSON, Excel or SQL. |
| 🌍 **Yours to use** | English and Turkish, light and dark themes, MIT / Apache-2.0. No account, no telemetry. |

## Supported databases

| | Database | What you get |
|---|---|---|
| ![PostgreSQL](https://img.shields.io/badge/-PostgreSQL-4169E1?logo=postgresql&logoColor=white) | PostgreSQL, CockroachDB, YugabyteDB | Data grid with editing, filters, structure and column changes, import, export, query plans |
| ![TimescaleDB](https://img.shields.io/badge/-TimescaleDB-FDB515?logo=timescale&logoColor=black) | TimescaleDB | The same; hypertables open like tables |
| ![MySQL](https://img.shields.io/badge/-MySQL-4479A1?logo=mysql&logoColor=white) | MySQL, MariaDB, TiDB | The same |
| ![SQLite](https://img.shields.io/badge/-SQLite-003B57?logo=sqlite&logoColor=white) | SQLite | The same, on a local file |
| ![SQL Server](https://img.shields.io/badge/-SQL%20Server-CC2927) | Microsoft SQL Server | Data grid with editing, structure and column changes, query plans |
| ![ClickHouse](https://img.shields.io/badge/-ClickHouse-FFCC01?logo=clickhouse&logoColor=black) | ClickHouse | Fast queries with cancel, structure, import |
| ![DuckDB](https://img.shields.io/badge/-DuckDB-FFF000?logo=duckdb&logoColor=black) | DuckDB | Built in, no server needed: data grid with editing, structure, import, export |
| ![Redis](https://img.shields.io/badge/-Redis-DC382D?logo=redis&logoColor=white) | Redis, Valkey | Key browser with editors for every core type |
| ![MongoDB](https://img.shields.io/badge/-MongoDB-47A248?logo=mongodb&logoColor=white) | MongoDB | Documents as a tree, a table or JSON, edited in place |
| ![Qdrant](https://img.shields.io/badge/-Qdrant-DC244C) | Qdrant | Vector similarity search with payload filters |
| ![Elasticsearch](https://img.shields.io/badge/-Elasticsearch-005571?logo=elasticsearch&logoColor=white) | Elasticsearch, OpenSearch | Search indices with Query DSL or query strings, edit documents, Dev Tools requests |
| ![pgvector](https://img.shields.io/badge/-pgvector-4169E1?logo=postgresql&logoColor=white) | pgvector | Vector search on PostgreSQL tables, with SQL beside it |
| ![Milvus](https://img.shields.io/badge/-Milvus-00A1EA?logo=milvus&logoColor=white) | Milvus | Vector similarity search with filters, named vectors |
| ![InfluxDB](https://img.shields.io/badge/-InfluxDB-22ADF6?logo=influxdb&logoColor=white) | InfluxDB 3 | SQL over time series: read-only grid, tags and fields in the structure |
| ![Couchbase](https://img.shields.io/badge/-Couchbase-EA2328?logo=couchbase&logoColor=white) | Couchbase | Documents by bucket and collection, edited in place; SQL++ in query tabs |
| ![Neo4j](https://img.shields.io/badge/-Neo4j-4581C3?logo=neo4j&logoColor=white) | Neo4j | Cypher with a graph canvas, a table and text |

More are on the way: Oracle, Snowflake, BigQuery and others. See the [roadmap](ROADMAP.md).

## A closer look

<table>
<tr>
<td width="50%"><b>SQL editor and results</b><br><picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screenshots/query-dark.webp"><img src="docs/assets/screenshots/query-light.webp" alt="SQL editor with two result sets in LewIDE"></picture></td>
<td width="50%"><b>AI assistant</b><br><picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screenshots/ai-dark.webp"><img src="docs/assets/screenshots/ai-light.webp" alt="The AI assistant explaining a query, next to the editor"></picture></td>
</tr>
<tr>
<td><b>Table data</b><br><picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screenshots/table-dark.webp"><img src="docs/assets/screenshots/table-light.webp" alt="Browsing and editing a table in the data grid"></picture></td>
<td><b>Table structure</b><br><picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screenshots/structure-dark.webp"><img src="docs/assets/screenshots/structure-light.webp" alt="Columns, keys and constraints of a table"></picture></td>
</tr>
<tr>
<td><b>Redis keys</b><br><picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screenshots/keys-dark.webp"><img src="docs/assets/screenshots/keys-light.webp" alt="Redis key browser with a JSON string value"></picture></td>
<td><b>MongoDB documents</b><br><picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screenshots/documents-dark.webp"><img src="docs/assets/screenshots/documents-light.webp" alt="MongoDB documents shown as a tree with BSON types"></picture></td>
</tr>
<tr>
<td><b>Vector search</b><br><picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screenshots/vector-dark.webp"><img src="docs/assets/screenshots/vector-light.webp" alt="Qdrant vector search with scores and payloads"></picture></td>
<td><b>Graph</b><br><picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screenshots/graph-dark.webp"><img src="docs/assets/screenshots/graph-light.webp" alt="A Neo4j graph drawn on a canvas with labels and relationships"></picture></td>
</tr>
</table>

## Install

Download the installer for your system from the
**[latest release](https://github.com/peritotr/lewide/releases/latest)**:

| System | File |
|---|---|
| macOS, Apple silicon (M1 and later) | `LewIDE_…_aarch64.dmg` |
| macOS, Intel | `LewIDE_…_x64.dmg` |
| Windows 10 and 11 | `LewIDE_…_x64-setup.exe` or `.msi` |
| Linux | `.AppImage` (any distribution), `.deb` (Debian, Ubuntu) or `.rpm` (Fedora, openSUSE) |

The installers are not signed with a paid developer certificate yet, so the
first launch shows a warning:

- **macOS:** open the app once, then go to System Settings → Privacy & Security and choose **Open Anyway**.
- **Windows:** SmartScreen says "Windows protected your PC"; choose **More info → Run anyway**.

⭐ Star or watch the repository to hear about new releases.

### Build from source

```bash
git clone https://github.com/peritotr/lewide.git
cd lewide
pnpm install
pnpm tauri build      # or `pnpm tauri dev` to run it straight away
```

You need [Rust](https://rustup.rs), [Node.js](https://nodejs.org) 22,
[pnpm](https://pnpm.io) and the [Tauri prerequisites](https://tauri.app/start/prerequisites/)
for your system. [CONTRIBUTING.md](CONTRIBUTING.md) has the details.

## Questions

**Is it free?** Yes, completely, under the MIT or Apache-2.0 licence. There is
no paid edition and no account.

**Does the AI assistant see my data?** No. It is sent the names and types of
your tables and columns, never the rows, and you can see what is sent before
it goes. You bring your own API key, or a local model through Ollama.

**Where are my passwords kept?** In your operating system's keychain
(Keychain on macOS, Credential Manager on Windows, the system keyring on Linux).

**Can it connect through SSH?** Yes, to every network database, with a
password or a key, and it checks the server's host key.

**Is it safe on a production database?** Mark the connection as Production:
the window shows it, and every change asks for confirmation first.

## Contributing

LewIDE is built in the open and contributions of every size are welcome: a
bug report, a translation, a new database driver. Start with
[CONTRIBUTING.md](CONTRIBUTING.md) and the [roadmap](ROADMAP.md).

## License

LewIDE is licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT license](LICENSE-MIT)

at your option. Unless you state otherwise, any contribution you submit for
inclusion in LewIDE is licensed as above, without any additional terms.
