//! PostgreSQL driver, also used for wire-compatible databases
//! (CockroachDB, YugabyteDB, TimescaleDB and similar).
//!
//! Statements run over the simple query protocol, so the server formats every
//! value as text. That makes any type displayable, including extension and
//! user-defined ones, and lets several statements run in one request.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use dbclient_core::{
    Capabilities, Column, ColumnDescription, Connection, ConnectionConfig, DataModel, DbObject,
    Driver, DriverInfo, Error, ForeignKey, IndexDescription, Namespace, ObjectDescription,
    ObjectKind, QueryRequest, QueryResult, Reference, Result, TlsMode, TriggerDescription, Value,
};
use futures_util::{StreamExt, pin_mut};
use tokio::sync::Mutex;
use tokio_postgres::config::SslMode;
use tokio_postgres::types::Type;
use tokio_postgres::{CancelToken, Client, NoTls, SimpleQueryMessage};
use tokio_postgres_rustls::MakeRustlsConnect;

pub struct PostgresDriver;

#[async_trait]
impl Driver for PostgresDriver {
    fn info(&self) -> DriverInfo {
        DriverInfo {
            id: "postgres",
            name: "PostgreSQL",
            model: DataModel::Relational,
            capabilities: Capabilities {
                sql: true,
                schemas: true,
                transactions: true,
                cancel: true,
                tls: true,
                ..Capabilities::default()
            },
            default_port: Some(5432),
            desktop_only: false,
        }
    }

    async fn connect(&self, config: &ConnectionConfig) -> Result<Box<dyn Connection>> {
        let user = config
            .user
            .as_deref()
            .filter(|u| !u.is_empty())
            .ok_or_else(|| Error::InvalidConfig("a user name is required".into()))?;

        let mut pg = tokio_postgres::Config::new();
        pg.host(
            config
                .host
                .as_deref()
                .filter(|h| !h.is_empty())
                .unwrap_or("localhost"),
        )
        // Through a route the socket goes to the route's port instead.
        .port(
            config
                .route
                .map_or(config.port.unwrap_or(5432), |r| r.port()),
        )
        .user(user)
        .application_name("dbclient")
        .connect_timeout(Duration::from_secs(10));
        if let Some(password) = &config.password {
            pg.password(password);
        }
        if let Some(database) = config.database.as_deref().filter(|d| !d.is_empty()) {
            pg.dbname(database);
        }

        if let Some(route) = config.route {
            // `host` is still sent for certificate checks; the socket goes to the route.
            pg.hostaddr(route.ip());
        }

        let connect_err = |e: tokio_postgres::Error| Error::Connect(describe(&e));
        // The connection object drives the socket; it ends when the client is dropped.
        let (client, tls) = match config.tls.mode {
            TlsMode::Off => {
                pg.ssl_mode(SslMode::Disable);
                let (client, connection) = pg.connect(NoTls).await.map_err(connect_err)?;
                tokio::spawn(connection);
                (client, None)
            }
            TlsMode::Verify | TlsMode::Insecure => {
                pg.ssl_mode(SslMode::Require);
                let tls = MakeRustlsConnect::new(dbclient_tls::client_config(&config.tls, None)?);
                let (client, connection) = pg.connect(tls.clone()).await.map_err(connect_err)?;
                tokio::spawn(connection);
                (client, Some(tls))
            }
        };

        // CockroachDB speaks this protocol but differs in a few places below.
        let cockroach = match client.query_opt("SELECT version()", &[]).await {
            Ok(Some(row)) => cockroach_version(&row.get::<_, String>(0)),
            _ => None,
        };

        Ok(Box::new(PostgresConnection {
            cancel_token: client.cancel_token(),
            client,
            tls,
            cockroach,
            running: Mutex::new(()),
        }))
    }
}

struct PostgresConnection {
    client: Client,
    cancel_token: CancelToken,
    /// The cancel request opens its own socket and needs the same TLS setup.
    tls: Option<MakeRustlsConnect>,
    /// The CockroachDB version when the server is one.
    cockroach: Option<String>,
    /// One statement at a time per session. `cancel` does not take this lock.
    running: Mutex<()>,
}

impl PostgresConnection {
    async fn catalog_query(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> Result<Vec<tokio_postgres::Row>> {
        let _running = self.running.lock().await;
        self.client.query(sql, params).await.map_err(query_err)
    }
}

#[async_trait]
impl Connection for PostgresConnection {
    async fn ping(&self) -> Result<()> {
        let _running = self.running.lock().await;
        self.client
            .simple_query("SELECT 1")
            .await
            .map(|_| ())
            .map_err(query_err)
    }

    async fn execute_all(&self, request: QueryRequest) -> Result<Vec<QueryResult>> {
        let _running = self.running.lock().await;
        let mut set_started = Instant::now();

        // The simple protocol reports column names only. Preparing the text first
        // yields the types. Text with several statements cannot be prepared, and
        // is not tried: a failed prepare would abort an open transaction. Its
        // values are returned as plain text.
        let types: Option<Vec<Type>> = if several_statements(&request.statement) {
            None
        } else {
            self.client
                .prepare(&request.statement)
                .await
                .ok()
                .map(|s| s.columns().iter().map(|c| c.type_().clone()).collect())
        };

        let stream = self
            .client
            .simple_query_raw(&request.statement)
            .await
            .map_err(query_err)?;
        pin_mut!(stream);

        let mut sets = Vec::new();
        // The result set being read; `None` between statements.
        let mut current: Option<QueryResult> = None;

        while let Some(message) = stream.next().await {
            match message.map_err(query_err)? {
                // A statement that returns rows starts with a description.
                SimpleQueryMessage::RowDescription(columns) => {
                    current = Some(QueryResult {
                        columns: columns
                            .iter()
                            .enumerate()
                            .map(|(i, c)| Column {
                                name: c.name().to_owned(),
                                type_name: types
                                    .as_ref()
                                    .and_then(|t| t.get(i))
                                    .map(|t| t.name().to_owned()),
                            })
                            .collect(),
                        rows: Vec::new(),
                        rows_affected: None,
                        truncated: false,
                        elapsed_ms: 0,
                    });
                }
                SimpleQueryMessage::Row(row) => {
                    let Some(set) = current.as_mut() else {
                        continue;
                    };
                    // Past the limit the rest is read and discarded. Cancelling
                    // instead would abort an open transaction and roll back a
                    // data-modifying statement.
                    if request.max_rows.is_some_and(|max| set.rows.len() >= max) {
                        set.truncated = true;
                        continue;
                    }
                    let cells = (0..row.len())
                        .map(|i| {
                            let ty = types.as_ref().and_then(|t| t.get(i));
                            row.try_get(i)
                                .map(|text| to_value(text, ty))
                                .map_err(query_err)
                        })
                        .collect::<Result<Vec<_>>>()?;
                    set.rows.push(cells);
                }
                // Ends every statement, with or without rows.
                SimpleQueryMessage::CommandComplete(count) => {
                    let mut set = current.take().unwrap_or(QueryResult {
                        columns: Vec::new(),
                        rows: Vec::new(),
                        rows_affected: Some(count),
                        truncated: false,
                        elapsed_ms: 0,
                    });
                    set.elapsed_ms = set_started.elapsed().as_millis() as u64;
                    set_started = Instant::now();
                    sets.push(set);
                }
                _ => {}
            }
        }

        if sets.is_empty() {
            return Err(Error::Query("no statement to run".into()));
        }
        Ok(sets)
    }

    async fn server_version(&self) -> Result<Option<String>> {
        // CockroachDB reports the PostgreSQL version it imitates there, not its own.
        if self.cockroach.is_some() {
            return Ok(self.cockroach.clone());
        }
        let rows = self
            .catalog_query(
                "SELECT pg_catalog.current_setting('server_version'), \
                        (SELECT extversion::text FROM pg_catalog.pg_extension \
                         WHERE extname = 'timescaledb')",
                &[],
            )
            .await?;
        Ok(rows.first().map(|r| {
            let timescaledb: Option<String> = r.get(1);
            engine_version(&r.get::<_, String>(0), timescaledb.as_deref())
        }))
    }

    async fn current_namespace(&self) -> Result<Option<String>> {
        // NULL when no schema on the search path exists.
        let rows = self
            .catalog_query("SELECT current_schema()::text", &[])
            .await?;
        Ok(rows.first().and_then(|r| r.get(0)))
    }

    async fn list_namespaces(&self) -> Result<Vec<Namespace>> {
        let rows = self
            .catalog_query(
                // TimescaleDB's own schemas (chunks, catalog, its information
                // views) are hidden like the system ones.
                "SELECT nspname FROM pg_catalog.pg_namespace \
                 WHERE nspname !~ '^pg_' AND nspname <> 'information_schema' \
                   AND nspname !~ '^_timescaledb_' \
                   AND nspname NOT IN ('timescaledb_information', 'timescaledb_experimental') \
                 ORDER BY nspname",
                &[],
            )
            .await?;
        Ok(rows.iter().map(|r| Namespace { name: r.get(0) }).collect())
    }

    async fn list_objects(&self, namespace: Option<&str>) -> Result<Vec<DbObject>> {
        let rows = self
            .catalog_query(
                // reltuples is the planner's estimate; negative means "never analyzed".
                "SELECT c.relname, c.relkind::text, \
                        CASE WHEN c.relkind IN ('r', 'p', 'm') AND c.reltuples >= 0 \
                             THEN c.reltuples::bigint END \
                 FROM pg_catalog.pg_class c \
                 JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = COALESCE($1, current_schema()) AND c.relkind IN ('r', 'p', 'f', 'v', 'm') \
                 ORDER BY c.relname",
                &[&namespace],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| {
                let kind: String = r.get(1);
                DbObject {
                    name: r.get(0),
                    kind: if kind == "v" || kind == "m" {
                        ObjectKind::View
                    } else {
                        ObjectKind::Table
                    },
                    row_estimate: r.get::<_, Option<i64>>(2).map(|n| n as u64),
                }
            })
            .collect())
    }

    async fn describe_object(
        &self,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<ObjectDescription> {
        let schema = namespace.unwrap_or("public");
        // CockroachDB gives a table without a primary key a hidden rowid
        // column, which SELECT * does not return; it is left out here too.
        let hidden = if self.cockroach.is_some() {
            "AND NOT a.attishidden"
        } else {
            ""
        };
        let rows = self
            .catalog_query(
                &format!(
                "SELECT a.attname, \
                        pg_catalog.format_type(a.atttypid, a.atttypmod), \
                        NOT a.attnotnull, \
                        pg_catalog.pg_get_expr(d.adbin, d.adrelid), \
                        COALESCE(a.attnum = ANY(i.indkey), false), \
                        pg_catalog.col_description(c.oid, a.attnum), \
                        CASE WHEN ty.typtype = 'e' THEN \
                             ARRAY(SELECT e.enumlabel::text FROM pg_catalog.pg_enum e \
                                   WHERE e.enumtypid = ty.oid ORDER BY e.enumsortorder) END \
                 FROM pg_catalog.pg_attribute a \
                 JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
                 JOIN pg_catalog.pg_type ty ON ty.oid = a.atttypid \
                 JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                 LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
                 LEFT JOIN pg_catalog.pg_index i ON i.indrelid = c.oid AND i.indisprimary \
                 WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped {hidden} \
                 ORDER BY a.attnum"
                ),
                &[&schema, &name],
            )
            .await?;
        if rows.is_empty() {
            return Err(Error::Query(format!(
                "no such table or view: {schema}.{name}"
            )));
        }
        Ok(ObjectDescription {
            name: name.to_owned(),
            columns: rows
                .iter()
                .map(|r| ColumnDescription {
                    name: r.get(0),
                    type_name: r.get(1),
                    nullable: r.get(2),
                    default_value: r.get(3),
                    primary_key: r.get(4),
                    comment: r.get(5),
                    enum_values: r.get(6),
                })
                .collect(),
        })
    }

    async fn list_indexes(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<IndexDescription>> {
        // CockroachDB does not report the size of an index this way.
        let size = if self.cockroach.is_some() {
            "NULL::text"
        } else {
            "pg_catalog.pg_size_pretty(pg_catalog.pg_relation_size(ix.indexrelid))"
        };
        let rows = self
            .catalog_query(
                // pg_get_indexdef gives each key as text, so expressions are covered too;
                // bit 0 of indoption marks a descending key.
                &format!(
                    "SELECT i.relname, ix.indisunique, ix.indisprimary, \
                        ARRAY(SELECT pg_catalog.pg_get_indexdef(ix.indexrelid, k + 1, true) \
                                     || CASE WHEN (COALESCE(ix.indoption[k], 0) & 1) = 1 THEN ' DESC' ELSE '' END \
                              FROM generate_subscripts(ix.indkey, 1) AS k ORDER BY k), \
                        am.amname::text, {size}, \
                        pg_catalog.pg_get_expr(ix.indpred, ix.indrelid) \
                 FROM pg_catalog.pg_index ix \
                 JOIN pg_catalog.pg_class i ON i.oid = ix.indexrelid \
                 JOIN pg_catalog.pg_am am ON am.oid = i.relam \
                 JOIN pg_catalog.pg_class t ON t.oid = ix.indrelid \
                 JOIN pg_catalog.pg_namespace n ON n.oid = t.relnamespace \
                 WHERE n.nspname = COALESCE($1, current_schema()) AND t.relname = $2 \
                 ORDER BY i.relname"
                ),
                &[&namespace, &table],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| IndexDescription {
                name: r.get(0),
                unique: r.get(1),
                primary: r.get(2),
                columns: r.get(3),
                method: r.get(4),
                size: r.get(5),
                condition: r.get(6),
            })
            .collect())
    }

    async fn list_referencing(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<Reference>> {
        let rows = self
            .catalog_query(
                "SELECT con.conname::text, NULLIF(fn.nspname::text, n.nspname::text), ft.relname::text, \
                        ARRAY(SELECT a.attname::text \
                              FROM unnest(con.conkey) WITH ORDINALITY AS k(num, ord) \
                              JOIN pg_catalog.pg_attribute a \
                                ON a.attrelid = con.conrelid AND a.attnum = k.num \
                              ORDER BY k.ord), \
                        ARRAY(SELECT a.attname::text \
                              FROM unnest(con.confkey) WITH ORDINALITY AS k(num, ord) \
                              JOIN pg_catalog.pg_attribute a \
                                ON a.attrelid = con.confrelid AND a.attnum = k.num \
                              ORDER BY k.ord), \
                        con.confupdtype::text, con.confdeltype::text \
                 FROM pg_catalog.pg_constraint con \
                 JOIN pg_catalog.pg_class t ON t.oid = con.confrelid \
                 JOIN pg_catalog.pg_namespace n ON n.oid = t.relnamespace \
                 JOIN pg_catalog.pg_class ft ON ft.oid = con.conrelid \
                 JOIN pg_catalog.pg_namespace fn ON fn.oid = ft.relnamespace \
                 WHERE con.contype = 'f' \
                   AND n.nspname = COALESCE($1, current_schema()) AND t.relname = $2 \
                 ORDER BY fn.nspname, ft.relname, con.conname",
                &[&namespace, &table],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| Reference {
                name: Some(r.get(0)),
                namespace: r.get(1),
                table: r.get(2),
                columns: r.get(3),
                referenced_columns: r.get(4),
                on_update: referential_action(r.get(5)),
                on_delete: referential_action(r.get(6)),
            })
            .collect())
    }

    async fn list_foreign_keys(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<ForeignKey>> {
        let rows = self
            .catalog_query(
                "SELECT con.conname::text, \
                        ARRAY(SELECT a.attname::text \
                              FROM unnest(con.conkey) WITH ORDINALITY AS k(num, ord) \
                              JOIN pg_catalog.pg_attribute a \
                                ON a.attrelid = con.conrelid AND a.attnum = k.num \
                              ORDER BY k.ord), \
                        NULLIF(fn.nspname::text, n.nspname::text), ft.relname::text, \
                        ARRAY(SELECT a.attname::text \
                              FROM unnest(con.confkey) WITH ORDINALITY AS k(num, ord) \
                              JOIN pg_catalog.pg_attribute a \
                                ON a.attrelid = con.confrelid AND a.attnum = k.num \
                              ORDER BY k.ord), \
                        con.confupdtype::text, con.confdeltype::text \
                 FROM pg_catalog.pg_constraint con \
                 JOIN pg_catalog.pg_class t ON t.oid = con.conrelid \
                 JOIN pg_catalog.pg_namespace n ON n.oid = t.relnamespace \
                 JOIN pg_catalog.pg_class ft ON ft.oid = con.confrelid \
                 JOIN pg_catalog.pg_namespace fn ON fn.oid = ft.relnamespace \
                 WHERE con.contype = 'f' \
                   AND n.nspname = COALESCE($1, current_schema()) AND t.relname = $2 \
                 ORDER BY con.conname",
                &[&namespace, &table],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| ForeignKey {
                name: Some(r.get(0)),
                columns: r.get(1),
                referenced_namespace: r.get(2),
                referenced_table: r.get(3),
                referenced_columns: r.get(4),
                on_update: referential_action(r.get(5)),
                on_delete: referential_action(r.get(6)),
            })
            .collect())
    }

    async fn list_triggers(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<TriggerDescription>> {
        let rows = self
            .catalog_query(
                "SELECT tg.tgname::text, tg.tgtype::int4, \
                        pg_catalog.pg_get_triggerdef(tg.oid, true) \
                 FROM pg_catalog.pg_trigger tg \
                 JOIN pg_catalog.pg_class t ON t.oid = tg.tgrelid \
                 JOIN pg_catalog.pg_namespace n ON n.oid = t.relnamespace \
                 WHERE NOT tg.tgisinternal \
                   AND n.nspname = COALESCE($1, current_schema()) AND t.relname = $2 \
                 ORDER BY tg.tgname",
                &[&namespace, &table],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| {
                let (timing, events) = trigger_type(r.get(1));
                TriggerDescription {
                    name: r.get(0),
                    timing: Some(timing.to_owned()),
                    events,
                    definition: r.get(2),
                }
            })
            .collect())
    }

    async fn object_ddl(&self, namespace: Option<&str>, name: &str) -> Result<Option<String>> {
        let qualified = self.qualified(namespace, name);
        if self.cockroach.is_some() {
            // CockroachDB writes the statement itself.
            let _running = self.running.lock().await;
            let rows = self
                .client
                .simple_query(&format!("SHOW CREATE {qualified}"))
                .await
                .map_err(query_err)?;
            return Ok(rows.iter().find_map(|message| match message {
                tokio_postgres::SimpleQueryMessage::Row(row) => {
                    row.get(1).map(|statement| format!("{statement};"))
                }
                _ => None,
            }));
        }

        let found = self
            .catalog_query(
                "SELECT c.oid, c.relkind::text, n.nspname::text \
                 FROM pg_catalog.pg_class c \
                 JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = COALESCE($1, current_schema()) AND c.relname = $2",
                &[&namespace, &name],
            )
            .await?;
        let Some(row) = found.first() else {
            return Ok(None);
        };
        let oid: u32 = row.get(0);
        let kind: String = row.get(1);
        // Always schema-qualified, so the statement means the same anywhere.
        let qualified = self.qualified(Some(&row.get::<_, String>(2)), name);

        if kind == "v" || kind == "m" {
            let rows = self
                .catalog_query("SELECT pg_catalog.pg_get_viewdef($1::oid, true)", &[&oid])
                .await?;
            let body: Option<String> = rows.first().and_then(|r| r.get(0));
            let keyword = if kind == "m" {
                "MATERIALIZED VIEW"
            } else {
                "VIEW"
            };
            return Ok(body.map(|body| {
                format!(
                    "CREATE {keyword} {qualified} AS\n{};",
                    body.trim_end().trim_end_matches(';')
                )
            }));
        }

        // PostgreSQL has no function that writes CREATE TABLE, so it is put
        // together from the catalog: columns, then constraints, then what was
        // created on the table afterwards.
        let columns = self
            .catalog_query(
                "SELECT a.attname::text, pg_catalog.format_type(a.atttypid, a.atttypmod), \
                        a.attnotnull, pg_catalog.pg_get_expr(d.adbin, d.adrelid), \
                        a.attidentity::text, a.attgenerated::text \
                 FROM pg_catalog.pg_attribute a \
                 LEFT JOIN pg_catalog.pg_attrdef d \
                   ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
                 WHERE a.attrelid = $1::oid AND a.attnum > 0 AND NOT a.attisdropped \
                 ORDER BY a.attnum",
                &[&oid],
            )
            .await?;
        let mut lines: Vec<String> = columns
            .iter()
            .map(|r| {
                let mut line = format!(
                    "{} {}",
                    self.quote_ident(&r.get::<_, String>(0)),
                    r.get::<_, String>(1)
                );
                let expression: Option<String> = r.get(3);
                let identity: String = r.get(4);
                let generated: String = r.get(5);
                match (identity.as_str(), generated.as_str(), expression) {
                    ("a", _, _) => line.push_str(" GENERATED ALWAYS AS IDENTITY"),
                    ("d", _, _) => line.push_str(" GENERATED BY DEFAULT AS IDENTITY"),
                    (_, "s", Some(e)) => {
                        line.push_str(&format!(" GENERATED ALWAYS AS ({e}) STORED"))
                    }
                    (_, _, Some(e)) => line.push_str(&format!(" DEFAULT {e}")),
                    _ => {}
                }
                if r.get::<_, bool>(2) {
                    line.push_str(" NOT NULL");
                }
                line
            })
            .collect();
        let constraints = self
            .catalog_query(
                "SELECT conname::text, pg_catalog.pg_get_constraintdef(oid, true) \
                 FROM pg_catalog.pg_constraint \
                 WHERE conrelid = $1::oid AND contype <> 'n' \
                 ORDER BY (contype = 'p') DESC, conname",
                &[&oid],
            )
            .await?;
        lines.extend(constraints.iter().map(|r| {
            format!(
                "CONSTRAINT {} {}",
                self.quote_ident(&r.get::<_, String>(0)),
                r.get::<_, String>(1)
            )
        }));
        let mut statements = vec![format!(
            "CREATE TABLE {qualified} (\n    {}\n);",
            lines.join(",\n    ")
        )];

        let rest = self
            .catalog_query(
                "SELECT pg_catalog.pg_get_indexdef(ix.indexrelid) AS def, 0 AS part \
                 FROM pg_catalog.pg_index ix \
                 WHERE ix.indrelid = $1::oid \
                   AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_constraint con \
                                   WHERE con.conindid = ix.indexrelid) \
                 UNION ALL \
                 SELECT pg_catalog.pg_get_triggerdef(tg.oid, true), 1 \
                 FROM pg_catalog.pg_trigger tg \
                 WHERE tg.tgrelid = $1::oid AND NOT tg.tgisinternal \
                 ORDER BY part, def",
                &[&oid],
            )
            .await?;
        statements.extend(rest.iter().map(|r| format!("{};", r.get::<_, String>(0))));
        Ok(Some(statements.join("\n\n")))
    }

    fn quote_ident(&self, name: &str) -> String {
        format!("\"{}\"", name.replace('"', "\"\""))
    }

    async fn is_alive(&self) -> bool {
        // Not a ping: inside a failed transaction every statement is rejected
        // although the connection itself is fine.
        !self.client.is_closed()
    }

    fn dialect(&self) -> dbclient_core::SqlDialect {
        dbclient_core::SqlDialect::Postgres
    }

    async fn in_transaction(&self) -> Result<bool> {
        if self.cockroach.is_some() {
            // CockroachDB says it directly; the clock comparison below does not hold there.
            let status = QueryRequest {
                statement: "SHOW TRANSACTION STATUS".into(),
                max_rows: None,
            };
            let result = self.execute(status).await?;
            return Ok(!matches!(
                result.rows.first().and_then(|r| r.first()),
                Some(Value::Text(status)) if status == "NoTxn"
            ));
        }
        // In a transaction, now() is frozen at its start while the statement
        // clock moves on; outside one the two are the same instant. A failed
        // transaction rejects every statement, which also means one is open.
        let check = QueryRequest {
            statement: "SELECT now() <> statement_timestamp()".into(),
            max_rows: None,
        };
        Ok(match self.execute(check).await {
            Ok(result) => result.rows.first().and_then(|r| r.first()) == Some(&Value::Bool(true)),
            Err(_) if !self.client.is_closed() => true,
            Err(e) => return Err(e),
        })
    }

    async fn apply_changes(
        &self,
        changes: &dbclient_core::ChangeSet,
    ) -> Result<dbclient_core::ApplyResult> {
        if self.in_transaction().await? {
            return Err(Error::Query("a transaction is open on this connection; commit or roll it back before applying changes".into()));
        }
        dbclient_core::edit::apply_on(self, changes).await
    }

    async fn cancel(&self) -> Result<()> {
        match &self.tls {
            Some(tls) => self.cancel_token.cancel_query(tls.clone()).await,
            None => self.cancel_token.cancel_query(NoTls).await,
        }
        .map_err(query_err)
    }

    async fn close(&self) -> Result<()> {
        // Dropping the client closes the socket.
        Ok(())
    }
}

/// `pg_constraint.confupdtype` and `confdeltype`, spelled out.
fn referential_action(code: String) -> Option<String> {
    let action = match code.as_str() {
        "a" => "NO ACTION",
        "r" => "RESTRICT",
        "c" => "CASCADE",
        "n" => "SET NULL",
        "d" => "SET DEFAULT",
        _ => return None,
    };
    Some(action.to_owned())
}

/// Timing and events packed into `pg_trigger.tgtype`.
fn trigger_type(bits: i32) -> (&'static str, Vec<String>) {
    let timing = if bits & 64 != 0 {
        "INSTEAD OF"
    } else if bits & 2 != 0 {
        "BEFORE"
    } else {
        "AFTER"
    };
    let events = [
        (4, "INSERT"),
        (16, "UPDATE"),
        (8, "DELETE"),
        (32, "TRUNCATE"),
    ]
    .into_iter()
    .filter(|(bit, _)| bits & bit != 0)
    .map(|(_, event)| event.to_owned())
    .collect();
    (timing, events)
}

/// "CockroachDB CCL v26.4.1 (...)" → "26.4.1"; `None` for any other server.
fn cockroach_version(version: &str) -> Option<String> {
    let rest = version.strip_prefix("CockroachDB")?;
    Some(
        rest.split_whitespace()
            .find_map(|word| word.strip_prefix('v'))
            .unwrap_or("")
            .to_owned(),
    )
}

/// The engine's own version where the server is a PostgreSQL derivative that
/// reports the PostgreSQL version it builds on: YugabyteDB writes
/// "15.12-YB-2025.2.0.0-b0", TimescaleDB is an extension with its own version.
fn engine_version(server_version: &str, timescaledb: Option<&str>) -> String {
    if let Some((postgres, yugabyte)) = server_version.split_once("-YB-") {
        let yugabyte = yugabyte.split_once("-b").map_or(yugabyte, |(v, _)| v);
        return format!("{yugabyte} (PostgreSQL {postgres})");
    }
    match timescaledb {
        Some(timescaledb) => format!("{timescaledb} (PostgreSQL {server_version})"),
        None => server_version.to_owned(),
    }
}

/// Turns the server's text form into a typed value where the type is known.
fn to_value(text: Option<&str>, ty: Option<&Type>) -> Value {
    let Some(text) = text else { return Value::Null };
    let fallback = || Value::Text(text.to_owned());
    let Some(ty) = ty else { return fallback() };

    match *ty {
        Type::BOOL => match text {
            "t" => Value::Bool(true),
            "f" => Value::Bool(false),
            _ => fallback(),
        },
        Type::INT2 | Type::INT4 | Type::INT8 | Type::OID => {
            text.parse().map(Value::Int).unwrap_or_else(|_| fallback())
        }
        // NaN and the infinities have no JSON form, so they stay text.
        Type::FLOAT4 | Type::FLOAT8 => match text.parse::<f64>() {
            Ok(v) if v.is_finite() => Value::Float(v),
            _ => fallback(),
        },
        Type::NUMERIC => Value::Decimal(text.to_owned()),
        Type::JSON | Type::JSONB => serde_json::from_str(text)
            .map(Value::Json)
            .unwrap_or_else(|_| fallback()),
        Type::DATE
        | Type::TIME
        | Type::TIMETZ
        | Type::TIMESTAMP
        | Type::TIMESTAMPTZ
        | Type::INTERVAL => Value::Temporal(text.to_owned()),
        Type::BYTEA => decode_hex_bytea(text)
            .map(Value::Bytes)
            .unwrap_or_else(fallback),
        _ => fallback(),
    }
}

/// Decodes the default `bytea_output = hex` form, `\x` followed by hex digits.
fn decode_hex_bytea(text: &str) -> Option<Vec<u8>> {
    let hex = text.strip_prefix("\\x")?;
    if hex.len() % 2 != 0 || !hex.is_ascii() {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

/// Server errors carry the useful text in the database error, not the wrapper.
fn describe(e: &tokio_postgres::Error) -> String {
    match e.as_db_error() {
        Some(db) => match db.detail() {
            Some(detail) => format!("{}: {} ({detail})", db.severity(), db.message()),
            None => format!("{}: {}", db.severity(), db.message()),
        },
        None => {
            let mut text = e.to_string();
            let mut source = std::error::Error::source(e);
            while let Some(inner) = source {
                text.push_str(&format!(": {inner}"));
                source = inner.source();
            }
            text
        }
    }
}

fn query_err(e: tokio_postgres::Error) -> Error {
    let Some(db) = e.as_db_error() else {
        return Error::Query(describe(&e));
    };
    Error::Database {
        code: Some(db.code().code().to_owned()),
        message: db.message().to_owned(),
        position: match db.position() {
            Some(tokio_postgres::error::ErrorPosition::Original(offset)) => Some(*offset),
            _ => None,
        },
        line: None,
        hint: db.hint().map(str::to_owned),
        detail: db.detail().map(str::to_owned),
    }
}

/// Whether the text holds more than one statement: a `;` outside quotes,
/// dollar quotes and comments, with more than space or comments after it.
/// It works on bytes: every character it looks for is ASCII, and UTF-8 never
/// uses an ASCII byte inside another character.
fn several_statements(text: &str) -> bool {
    let bytes = text.as_bytes();
    let find = |from: usize, needle: &[u8]| {
        bytes[from.min(bytes.len())..]
            .windows(needle.len())
            .position(|w| w == needle)
            .map(|n| from + n)
    };
    let mut i = 0;
    let mut ended = false;
    while i < bytes.len() {
        match bytes[i] {
            quote @ (b'\'' | b'"') => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == quote {
                        // A doubled quote stays inside.
                        if bytes.get(i + 1) == Some(&quote) {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                i += 1;
            }
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                i = find(i, b"\n").unwrap_or(bytes.len());
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = find(i + 2, b"*/").map_or(bytes.len(), |n| n + 2);
            }
            b'$' => {
                // $tag$ … $tag$, where the tag may be empty.
                let mut end = i + 1;
                while end < bytes.len()
                    && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_')
                {
                    end += 1;
                }
                if bytes.get(end) == Some(&b'$') {
                    let tag = &bytes[i..=end];
                    i = find(end + 1, tag).map_or(bytes.len(), |n| n + tag.len());
                } else {
                    i += 1;
                }
            }
            b';' => {
                ended = true;
                i += 1;
            }
            c if c.is_ascii_whitespace() => i += 1,
            _ if ended => return true,
            _ => i += 1,
        }
    }
    false
}

#[cfg(test)]
mod unit_tests {
    use super::several_statements;

    #[test]
    fn counts_statements_outside_quotes_and_bodies() {
        assert!(!several_statements("SELECT 1"));
        assert!(!several_statements("SELECT 1;  -- done\n"));
        assert!(!several_statements(
            "SELECT 'a;b', \"x;y\" FROM t; /* end */"
        ));
        assert!(!several_statements("SELECT 'it''s; fine'"));
        assert!(!several_statements(
            "CREATE FUNCTION f() RETURNS int AS $body$ BEGIN RETURN 1; END; $body$ LANGUAGE plpgsql;"
        ));
        assert!(!several_statements("DO $$ BEGIN PERFORM 1; END $$"));
        assert!(several_statements("SELECT 1; SELECT 2"));
        assert!(several_statements(
            "CREATE FUNCTION f() RETURNS trigger AS $$ BEGIN RETURN NEW; END; $$ LANGUAGE plpgsql;\nCREATE TRIGGER t BEFORE UPDATE ON x FOR EACH ROW EXECUTE FUNCTION f();"
        ));
        assert!(several_statements("SELECT 1 -- a;\n; SELECT 2"));
        // A dollar sign that opens no quote: a positional parameter.
        assert!(!several_statements("SELECT $1"));
        // Text outside ASCII is passed over a byte at a time.
        assert!(!several_statements("SELECT 'ğüş' AS şehir;"));
        assert!(several_statements("SELECT 'ğ'; SELECT 'İzmir'"));
    }

    use super::*;

    #[test]
    fn recognizes_cockroachdb() {
        assert_eq!(
            cockroach_version("CockroachDB CCL v26.4.1 (aarch64-unknown-linux-gnu, built 2026)")
                .as_deref(),
            Some("26.4.1")
        );
        assert_eq!(
            cockroach_version("PostgreSQL 17.2 on aarch64-unknown-linux-musl"),
            None
        );
    }

    #[test]
    fn names_derived_engines() {
        assert_eq!(
            engine_version("15.12-YB-2025.2.0.0-b0", None),
            "2025.2.0.0 (PostgreSQL 15.12)"
        );
        assert_eq!(
            engine_version("17.7 (Ubuntu 17.7-1)", Some("2.25.0")),
            "2.25.0 (PostgreSQL 17.7 (Ubuntu 17.7-1))"
        );
        assert_eq!(engine_version("17.2", None), "17.2");
    }

    #[test]
    fn typed_conversion() {
        assert_eq!(to_value(None, Some(&Type::INT4)), Value::Null);
        assert_eq!(to_value(Some("t"), Some(&Type::BOOL)), Value::Bool(true));
        assert_eq!(to_value(Some("-7"), Some(&Type::INT8)), Value::Int(-7));
        assert_eq!(
            to_value(Some("1.5"), Some(&Type::FLOAT8)),
            Value::Float(1.5)
        );
        assert_eq!(
            to_value(Some("NaN"), Some(&Type::FLOAT8)),
            Value::Text("NaN".into())
        );
        assert_eq!(
            to_value(Some("Infinity"), Some(&Type::FLOAT8)),
            Value::Text("Infinity".into())
        );
        assert_eq!(
            to_value(Some("1.10"), Some(&Type::NUMERIC)),
            Value::Decimal("1.10".into())
        );
        assert_eq!(
            to_value(Some("\\x00ff"), Some(&Type::BYTEA)),
            Value::Bytes(vec![0, 255])
        );
        assert_eq!(to_value(Some("42"), None), Value::Text("42".into()));
    }
}
