# Contributing to LewIDE

Thank you for helping. Issues, ideas and pull requests are all welcome. For a
large change, open an issue first so we can agree on the approach.

## Building

You need [Rust](https://rustup.rs), [Node.js](https://nodejs.org) 22 and
[pnpm](https://pnpm.io), plus the [Tauri prerequisites](https://tauri.app/start/prerequisites/)
for your system.

```bash
pnpm install
pnpm tauri dev        # run the app
pnpm tauri build      # build an installer for this system
```

The first build compiles DuckDB from source and takes several minutes.

## Layout

| Path | Contents |
|---|---|
| `crates/core` | Driver and connection traits, capabilities, the value model, SQL dialects, import, export, column changes |
| `crates/driver-*` | One crate per engine |
| `crates/tunnel` | SSH tunnels; wraps any network driver |
| `crates/store` | Saved connections: settings in a JSON file, passwords in the system credential store |
| `crates/ai` | AI assistant: provider clients, the schema description sent to them, prompts |
| `src-tauri` | Desktop shell: the session store and the commands the interface calls |
| `src` | Interface (React, Tailwind, CodeMirror). `src/lib/api.ts` mirrors the Rust types; `src/tokens.css` holds the design tokens |
| `docs/design` | Design notes the interface is built from |
| `docs/assets` | README images; `scripts/readme-art.py` regenerates the banner |

## Checks

Pull requests run these on macOS, Windows and Linux:

```bash
pnpm build            # type-check and build the interface
pnpm test             # interface unit tests
pnpm i18n:check       # every text translated, none left over
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Tests against real servers

Driver tests that need a server are skipped unless their `DBCLIENT_TEST_*`
variables are set. Each script starts a throwaway Docker container and prints
the variables; `down` removes it again.

```bash
scripts/test-servers.sh up && . target/test-servers/env.sh   # PostgreSQL, MySQL (both with TLS), SSH
scripts/test-servers.sh compat                               # MariaDB, CockroachDB, TiDB, YugabyteDB, TimescaleDB as well
eval "$(scripts/test-redis.sh up)"; eval "$(scripts/test-redis.sh tls)"
eval "$(scripts/test-mongodb.sh up)"
eval "$(scripts/test-qdrant.sh up)"; eval "$(scripts/test-qdrant.sh tls)"
eval "$(scripts/test-pgvector.sh up)"
eval "$(scripts/test-elasticsearch.sh up)"; eval "$(scripts/test-elasticsearch.sh open)"   # Elasticsearch and OpenSearch
eval "$(scripts/test-neo4j.sh up)"
eval "$(scripts/test-clickhouse.sh up)"
eval "$(scripts/test-sqlserver.sh up)"                       # accepts Microsoft's licence for the container
cargo test --workspace
```

SQL Server's image is x86 only; on Apple silicon Docker runs it through
Rosetta and it takes about a minute to start. SQLite and DuckDB tests need no
server and always run.

## Trying the interface without building the app

`pnpm dev` serves the interface in a browser with sample data for every kind
of database. To use the real core instead:

```bash
cargo run -p dbclient --features bridge --bin bridge   # serves the commands on 127.0.0.1:1430
pnpm dev                                               # then open http://localhost:1420/?bridge=1430
```

The bridge is a development tool: it answers only the development server's
page, keeps passwords in memory, and is not part of the application.

## Adding a database

1. Create `crates/driver-<name>` and implement `Driver` and `Connection` from `dbclient-core`.
   Non-relational engines also implement one of the model traits (`KeyValue`,
   `DocumentStore`, `VectorStore`, `GraphStore`).
2. Describe it in `Driver::info`: data model, capabilities, `desktop_only` if it needs native code.
3. Register it in `src-tauri/src/lib.rs` and give the type its `driver` in `src/lib/types.ts`.
4. Add a `scripts/test-<name>.sh` and live tests gated on a `DBCLIENT_TEST_*` variable.

Prefer pure-Rust client libraries so the driver can also build for iOS and Android.

## Translations

The English text in the code is the source. Each other language is one file in
`src/i18n` (`tr.json` for Turkish) that maps an English text to its
translation; a text without an entry is shown in English.

To add a language: copy `tr.json` to `<code>.json`, translate the values, and
list the language in `LANGUAGES` and `DICTIONARIES` in `src/i18n/index.ts`.
Keep the `{placeholders}` as they are; they can move anywhere in the sentence.

```bash
pnpm i18n:check                          # what each file is missing or no longer needs
node scripts/i18n-check.mjs --add tr     # add new texts to a file, with empty translations
```

In code, wrap every text a person will read: `t("…")`, `tn(count, "one", "other")`
for plurals, `tx("… {link} …", { link: <a/> })` when the text has elements inside it.

## Licence

By contributing you agree that your contribution is licensed under the
project's terms: MIT or Apache-2.0, at the user's option.
