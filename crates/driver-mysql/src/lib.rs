//! MySQL driver, also used for wire-compatible databases
//! (MariaDB, TiDB, PlanetScale and similar).
//!
//! Statements run over the text protocol, so several statements can be sent in
//! one request and every value arrives in its text form with column metadata.

use std::path::PathBuf;
use std::time::Instant;

use async_trait::async_trait;
use dbclient_core::{
    Capabilities, Column, ColumnDescription, Connection, ConnectionConfig, DataModel, DbObject,
    Driver, DriverInfo, Error, ForeignKey, IndexDescription, Namespace, ObjectDescription,
    ObjectKind, QueryRequest, QueryResult, Reference, Result, TlsMode, TriggerDescription, Value,
};
use mysql_async::consts::{ColumnFlags, ColumnType};
use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Opts, OptsBuilder, SslOpts};
use tokio::sync::Mutex;

/// Character set id the server reports for binary data.
const BINARY_CHARSET: u16 = 63;

/// The values of `enum('a','b')` as the column type spells them; `None` for any other type.
fn enum_values(column_type: &str) -> Option<Vec<String>> {
    let list = column_type.strip_prefix("enum(")?.strip_suffix(')')?;
    let mut values = Vec::new();
    let mut current = String::new();
    let mut chars = list.chars().peekable();
    let mut inside = false;
    while let Some(c) = chars.next() {
        match c {
            // A quote inside a value is written twice.
            '\'' if inside && chars.peek() == Some(&'\'') => {
                chars.next();
                current.push('\'');
            }
            '\'' if inside => {
                inside = false;
                values.push(std::mem::take(&mut current));
            }
            '\'' => inside = true,
            _ if inside => current.push(c),
            _ => {}
        }
    }
    Some(values)
}

pub struct MysqlDriver;

#[async_trait]
impl Driver for MysqlDriver {
    fn info(&self) -> DriverInfo {
        DriverInfo {
            id: "mysql",
            name: "MySQL",
            model: DataModel::Relational,
            capabilities: Capabilities {
                sql: true,
                transactions: true,
                cancel: true,
                tls: true,
                ..Capabilities::default()
            },
            default_port: Some(3306),
            desktop_only: false,
        }
    }

    async fn connect(&self, config: &ConnectionConfig) -> Result<Box<dyn Connection>> {
        let user = config
            .user
            .as_deref()
            .filter(|u| !u.is_empty())
            .ok_or_else(|| Error::InvalidConfig("a user name is required".into()))?;

        let host = config
            .host
            .as_deref()
            .filter(|h| !h.is_empty())
            .unwrap_or("localhost");
        let mut builder = OptsBuilder::default()
            .ip_or_hostname(host)
            .tcp_port(config.port.unwrap_or(3306))
            .user(Some(user))
            .pass(config.password.clone())
            .db_name(config.database.clone().filter(|d| !d.is_empty()))
            // Stay on TCP: the socket path the server reports is not reachable
            // through tunnels or containers.
            .prefer_socket(false);
        if let Some(route) = config.route {
            builder = builder
                .ip_or_hostname(route.ip().to_string())
                .tcp_port(route.port());
        }

        if config.tls.mode != TlsMode::Off {
            // The client library takes the process-wide rustls provider and fails
            // if none was chosen. An earlier choice by another crate is kept.
            let _ = rustls::crypto::ring::default_provider().install_default();

            let mut ssl = SslOpts::default();
            if let Some(path) = config.tls.ca_file.as_deref().filter(|p| !p.is_empty()) {
                std::fs::metadata(path).map_err(|e| {
                    Error::InvalidConfig(format!("CA certificate file {path}: {e}"))
                })?;
                ssl = ssl.with_root_certs(vec![PathBuf::from(path).into()]);
            }
            if config.tls.mode == TlsMode::Insecure {
                ssl = ssl
                    .with_danger_accept_invalid_certs(true)
                    .with_danger_skip_domain_validation(true);
            }
            if config.route.is_some() {
                // The socket goes to the route; the certificate is still checked
                // against the real server name.
                ssl = ssl.with_danger_tls_hostname_override(Some(host.to_owned()));
            }
            builder = builder.ssl_opts(ssl);
        }
        let opts: Opts = builder.into();

        let conn = Conn::new(opts.clone())
            .await
            .map_err(|e| Error::Connect(e.to_string()))?;
        Ok(Box::new(MysqlConnection {
            id: conn.id(),
            opts,
            conn: Mutex::new(Some(conn)),
        }))
    }
}

struct MysqlConnection {
    /// `None` once closed. The lock also keeps the session to one statement at a time.
    conn: Mutex<Option<Conn>>,
    /// Kept to open the side connection that `cancel` needs.
    opts: Opts,
    id: u32,
}

fn open(slot: &mut Option<Conn>) -> Result<&mut Conn> {
    slot.as_mut()
        .ok_or_else(|| Error::Connect("the connection is closed".into()))
}

#[async_trait]
impl Connection for MysqlConnection {
    async fn ping(&self) -> Result<()> {
        let mut slot = self.conn.lock().await;
        open(&mut slot)?.ping().await.map_err(query_err)
    }

    async fn execute_all(&self, request: QueryRequest) -> Result<Vec<QueryResult>> {
        let mut slot = self.conn.lock().await;
        let conn = open(&mut slot)?;
        let mut set_started = Instant::now();

        let mut pending = conn
            .query_iter(request.statement.as_str())
            .await
            .map_err(query_err)?;
        let mut sets = Vec::new();

        // One pass per statement. A set stays pending until it is read,
        // including those without rows.
        while let Some(columns) = pending.columns() {
            let mut set = QueryResult {
                columns: columns
                    .iter()
                    .map(|c| Column {
                        name: c.name_str().into_owned(),
                        type_name: type_name(c).map(str::to_owned),
                    })
                    .collect(),
                rows: Vec::new(),
                rows_affected: columns.is_empty().then(|| pending.affected_rows()),
                truncated: false,
                elapsed_ms: 0,
            };

            // `next` returns `None` at the end of a set and moves to the following one.
            while let Some(row) = pending.next().await.map_err(query_err)? {
                // Past the limit the rest is read and discarded, as in the
                // PostgreSQL driver: killing the query would lose work.
                if request.max_rows.is_some_and(|max| set.rows.len() >= max) {
                    set.truncated = true;
                    continue;
                }
                set.rows.push(
                    row.unwrap()
                        .into_iter()
                        .enumerate()
                        .map(|(i, value)| to_value(value, columns.get(i)))
                        .collect(),
                );
            }
            set.elapsed_ms = set_started.elapsed().as_millis() as u64;
            set_started = Instant::now();
            sets.push(set);
        }
        // The error of a statement after the first is held back until the next read.
        pending.next().await.map_err(query_err)?;

        Ok(sets)
    }

    async fn server_version(&self) -> Result<Option<String>> {
        let mut slot = self.conn.lock().await;
        open(&mut slot)?
            .query_first("SELECT VERSION()")
            .await
            .map_err(query_err)
    }

    async fn current_namespace(&self) -> Result<Option<String>> {
        let mut slot = self.conn.lock().await;
        let current: Option<Option<String>> = open(&mut slot)?
            .query_first("SELECT DATABASE()")
            .await
            .map_err(query_err)?;
        Ok(current.flatten())
    }

    async fn list_namespaces(&self) -> Result<Vec<Namespace>> {
        let mut slot = self.conn.lock().await;
        let names: Vec<String> = open(&mut slot)?
            .query(
                "SELECT schema_name FROM information_schema.schemata \
                 WHERE schema_name NOT IN ('information_schema', 'performance_schema', 'mysql', 'sys') \
                 ORDER BY schema_name",
            )
            .await
            .map_err(query_err)?;
        Ok(names.into_iter().map(|name| Namespace { name }).collect())
    }

    async fn list_objects(&self, namespace: Option<&str>) -> Result<Vec<DbObject>> {
        let mut slot = self.conn.lock().await;
        // table_rows is exact for some engines and an estimate for InnoDB.
        let rows: Vec<(String, String, Option<u64>)> = open(&mut slot)?
            .exec(
                "SELECT table_name, table_type, table_rows FROM information_schema.tables \
                 WHERE table_schema = COALESCE(?, DATABASE()) ORDER BY table_name",
                (namespace,),
            )
            .await
            .map_err(query_err)?;
        Ok(rows
            .into_iter()
            .map(|(name, kind, row_estimate)| DbObject {
                name,
                row_estimate,
                kind: if kind.contains("VIEW") {
                    ObjectKind::View
                } else {
                    ObjectKind::Table
                },
            })
            .collect())
    }

    async fn describe_object(
        &self,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<ObjectDescription> {
        let mut slot = self.conn.lock().await;
        let rows: Vec<(String, String, bool, Option<String>, bool, Option<String>)> = open(&mut slot)?
            .exec(
                "SELECT column_name, column_type, is_nullable = 'YES', column_default, column_key = 'PRI', \
                        NULLIF(column_comment, '') \
                 FROM information_schema.columns \
                 WHERE table_schema = COALESCE(?, DATABASE()) AND table_name = ? \
                 ORDER BY ordinal_position",
                (namespace, name),
            )
            .await
            .map_err(query_err)?;
        if rows.is_empty() {
            return Err(Error::Query(format!("no such table or view: {name}")));
        }
        Ok(ObjectDescription {
            name: name.to_owned(),
            columns: rows
                .into_iter()
                .map(
                    |(name, type_name, nullable, default_value, primary_key, comment)| {
                        ColumnDescription {
                            name,
                            enum_values: enum_values(&type_name),
                            type_name: Some(type_name),
                            nullable,
                            default_value,
                            primary_key,
                            comment,
                        }
                    },
                )
                .collect(),
        })
    }

    async fn list_indexes(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<IndexDescription>> {
        let mut slot = self.conn.lock().await;
        // One row per indexed column; a NULL column name is an expression.
        let rows: Vec<(String, bool, Option<String>, Option<String>)> = open(&mut slot)?
            .exec(
                "SELECT index_name, non_unique = 0, column_name, index_type FROM information_schema.statistics \
                 WHERE table_schema = COALESCE(?, DATABASE()) AND table_name = ? \
                 ORDER BY index_name, seq_in_index",
                (namespace, table),
            )
            .await
            .map_err(query_err)?;
        let mut indexes: Vec<IndexDescription> = Vec::new();
        for (name, unique, column, method) in rows {
            let column = column.unwrap_or_else(|| "(expression)".to_owned());
            match indexes.last_mut().filter(|i| i.name == name) {
                Some(index) => index.columns.push(column),
                None => indexes.push(IndexDescription {
                    primary: name == "PRIMARY",
                    name,
                    columns: vec![column],
                    unique,
                    method: method.map(|m| m.to_lowercase()),
                    // MySQL keeps no size per index, and has no partial indexes.
                    size: None,
                    condition: None,
                }),
            }
        }
        Ok(indexes)
    }

    async fn list_referencing(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<Reference>> {
        let mut slot = self.conn.lock().await;
        type Row = (
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        );
        let rows: Vec<Row> = open(&mut slot)?
            .exec(
                "SELECT k.constraint_name, k.table_schema, k.table_name, k.column_name, \
                        k.referenced_column_name, r.update_rule, r.delete_rule, DATABASE() \
                 FROM information_schema.key_column_usage k \
                 JOIN information_schema.referential_constraints r \
                   ON r.constraint_schema = k.constraint_schema \
                  AND r.constraint_name = k.constraint_name \
                  AND r.table_name = k.table_name \
                 WHERE k.referenced_table_schema = COALESCE(?, DATABASE()) \
                   AND k.referenced_table_name = ? \
                 ORDER BY k.table_schema, k.table_name, k.constraint_name, k.ordinal_position",
                (namespace, table),
            )
            .await
            .map_err(query_err)?;
        let own = namespace.map(str::to_owned);
        let mut references: Vec<Reference> = Vec::new();
        for (name, schema, from, column, target, on_update, on_delete, current) in rows {
            let same = references
                .last()
                .is_some_and(|r| r.name.as_deref() == Some(&name) && r.table == from);
            if !same {
                let home = own.clone().or(current);
                references.push(Reference {
                    name: Some(name),
                    namespace: Some(schema).filter(|s| Some(s) != home.as_ref()),
                    table: from,
                    columns: Vec::new(),
                    referenced_columns: Vec::new(),
                    on_update,
                    on_delete,
                });
            }
            let reference = references.last_mut().expect("just pushed");
            reference.columns.push(column);
            reference.referenced_columns.push(target);
        }
        Ok(references)
    }

    async fn list_foreign_keys(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<ForeignKey>> {
        let mut slot = self.conn.lock().await;
        // One row per column of each key.
        type Row = (
            String,
            String,
            Option<String>,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        );
        let rows: Vec<Row> = open(&mut slot)?
            .exec(
                "SELECT k.constraint_name, k.column_name, k.referenced_table_schema, \
                        k.referenced_table_name, k.referenced_column_name, \
                        r.update_rule, r.delete_rule, DATABASE() \
                 FROM information_schema.key_column_usage k \
                 JOIN information_schema.referential_constraints r \
                   ON r.constraint_schema = k.constraint_schema \
                  AND r.constraint_name = k.constraint_name \
                  AND r.table_name = k.table_name \
                 WHERE k.table_schema = COALESCE(?, DATABASE()) AND k.table_name = ? \
                   AND k.referenced_table_name IS NOT NULL \
                 ORDER BY k.constraint_name, k.ordinal_position",
                (namespace, table),
            )
            .await
            .map_err(query_err)?;
        let own = namespace.map(str::to_owned);
        let mut keys: Vec<ForeignKey> = Vec::new();
        for (name, column, schema, target, target_column, on_update, on_delete, current) in rows {
            if keys.last().is_none_or(|k| k.name.as_deref() != Some(&name)) {
                let home = own.clone().or(current);
                keys.push(ForeignKey {
                    name: Some(name),
                    columns: Vec::new(),
                    referenced_namespace: schema.filter(|s| Some(s) != home.as_ref()),
                    referenced_table: target,
                    referenced_columns: Vec::new(),
                    on_update,
                    on_delete,
                });
            }
            let key = keys.last_mut().expect("just pushed");
            key.columns.push(column);
            key.referenced_columns.push(target_column);
        }
        Ok(keys)
    }

    async fn list_triggers(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<TriggerDescription>> {
        let mut slot = self.conn.lock().await;
        let rows: Vec<(String, String, String, String)> = open(&mut slot)?
            .exec(
                "SELECT trigger_name, action_timing, event_manipulation, action_statement \
                 FROM information_schema.triggers \
                 WHERE event_object_schema = COALESCE(?, DATABASE()) AND event_object_table = ? \
                 ORDER BY trigger_name",
                (namespace, table),
            )
            .await
            .map_err(query_err)?;
        Ok(rows
            .into_iter()
            .map(|(name, timing, event, body)| TriggerDescription {
                name,
                timing: Some(timing),
                events: vec![event],
                definition: Some(body),
            })
            .collect())
    }

    async fn object_ddl(&self, namespace: Option<&str>, name: &str) -> Result<Option<String>> {
        let mut slot = self.conn.lock().await;
        // The same statement answers for views; the text is the second column either way.
        let row: Option<mysql_async::Row> = open(&mut slot)?
            .query_first(format!(
                "SHOW CREATE TABLE {}",
                self.qualified(namespace, name)
            ))
            .await
            .map_err(query_err)?;
        Ok(row
            .and_then(|row| row.get::<Option<String>, _>(1))
            .flatten()
            .map(|statement| format!("{statement};")))
    }

    fn quote_ident(&self, name: &str) -> String {
        format!("`{}`", name.replace('`', "``"))
    }

    fn dialect(&self) -> dbclient_core::SqlDialect {
        dbclient_core::SqlDialect::MySql
    }

    async fn in_transaction(&self) -> Result<bool> {
        let mut slot = self.conn.lock().await;
        let conn = open(&mut slot)?;
        // MariaDB and TiDB answer directly.
        if let Ok(Some(open)) = conn.query_first::<i64, _>("SELECT @@in_transaction").await {
            return Ok(open != 0);
        }
        // MySQL records it in the performance schema, which an account may not be allowed to read.
        conn.query_first::<i64, _>(
            "SELECT COUNT(*) FROM performance_schema.events_transactions_current \
             WHERE THREAD_ID = PS_CURRENT_THREAD_ID() AND STATE = 'ACTIVE' AND AUTOCOMMIT = 'NO'",
        )
        .await
        .map(|count| count.unwrap_or(0) > 0)
        .map_err(|_| Error::Unsupported("transaction state"))
    }

    /// Runs on a connection of its own. Starting a transaction on the user's
    /// session would silently commit whatever they have open there, and MySQL
    /// does not reliably tell whether something is.
    async fn apply_changes(
        &self,
        changes: &dbclient_core::ChangeSet,
    ) -> Result<dbclient_core::ApplyResult> {
        // Count the rows a statement matched, not the rows it altered: saving a
        // value that is already there must still count as that one row.
        let opts: Opts = OptsBuilder::from_opts(self.opts.clone())
            .client_found_rows(true)
            .into();
        let conn = Conn::new(opts)
            .await
            .map_err(|e| Error::Connect(e.to_string()))?;
        let side = MysqlConnection {
            id: conn.id(),
            opts: self.opts.clone(),
            conn: Mutex::new(Some(conn)),
        };
        let result = dbclient_core::edit::apply_on(&side, changes).await;
        let _ = side.close().await;
        result
    }

    /// MySQL has no in-band cancel; a second connection kills the running statement.
    async fn cancel(&self) -> Result<()> {
        let mut side = Conn::new(self.opts.clone())
            .await
            .map_err(|e| Error::Connect(e.to_string()))?;
        let killed = side
            .query_drop(format!("KILL QUERY {}", self.id))
            .await
            .map_err(query_err);
        let _ = side.disconnect().await;
        killed
    }

    async fn close(&self) -> Result<()> {
        if let Some(conn) = self.conn.lock().await.take() {
            conn.disconnect().await.map_err(query_err)?;
        }
        Ok(())
    }
}

fn is_binary(column: &mysql_async::Column) -> bool {
    column.character_set() == BINARY_CHARSET
}

fn type_name(column: &mysql_async::Column) -> Option<&'static str> {
    use ColumnType::*;
    let unsigned = column.flags().contains(ColumnFlags::UNSIGNED_FLAG);
    let binary = is_binary(column);
    let pick = |signed, unsigned_name| if unsigned { unsigned_name } else { signed };
    Some(match column.column_type() {
        MYSQL_TYPE_TINY => pick("tinyint", "tinyint unsigned"),
        MYSQL_TYPE_SHORT => pick("smallint", "smallint unsigned"),
        MYSQL_TYPE_INT24 => pick("mediumint", "mediumint unsigned"),
        MYSQL_TYPE_LONG => pick("int", "int unsigned"),
        MYSQL_TYPE_LONGLONG => pick("bigint", "bigint unsigned"),
        MYSQL_TYPE_FLOAT => "float",
        MYSQL_TYPE_DOUBLE => "double",
        MYSQL_TYPE_DECIMAL | MYSQL_TYPE_NEWDECIMAL => "decimal",
        MYSQL_TYPE_YEAR => "year",
        MYSQL_TYPE_DATE | MYSQL_TYPE_NEWDATE => "date",
        MYSQL_TYPE_TIME | MYSQL_TYPE_TIME2 => "time",
        MYSQL_TYPE_DATETIME | MYSQL_TYPE_DATETIME2 => "datetime",
        MYSQL_TYPE_TIMESTAMP | MYSQL_TYPE_TIMESTAMP2 => "timestamp",
        MYSQL_TYPE_JSON => "json",
        MYSQL_TYPE_BIT => "bit",
        MYSQL_TYPE_ENUM => "enum",
        MYSQL_TYPE_SET => "set",
        MYSQL_TYPE_GEOMETRY => "geometry",
        MYSQL_TYPE_NULL => "null",
        MYSQL_TYPE_VARCHAR | MYSQL_TYPE_VAR_STRING if binary => "varbinary",
        MYSQL_TYPE_VARCHAR | MYSQL_TYPE_VAR_STRING => "varchar",
        MYSQL_TYPE_STRING if binary => "binary",
        MYSQL_TYPE_STRING => "char",
        MYSQL_TYPE_TINY_BLOB | MYSQL_TYPE_MEDIUM_BLOB | MYSQL_TYPE_LONG_BLOB | MYSQL_TYPE_BLOB
            if binary =>
        {
            "blob"
        }
        MYSQL_TYPE_TINY_BLOB | MYSQL_TYPE_MEDIUM_BLOB | MYSQL_TYPE_LONG_BLOB | MYSQL_TYPE_BLOB => {
            "text"
        }
        _ => return None,
    })
}

/// Turns the server's text form into a typed value using the column metadata.
fn to_value(value: mysql_async::Value, column: Option<&mysql_async::Column>) -> Value {
    use ColumnType::*;
    let bytes = match value {
        mysql_async::Value::NULL => return Value::Null,
        mysql_async::Value::Bytes(bytes) => bytes,
        // The text protocol only produces the two variants above.
        other => return Value::Text(other.as_sql(true)),
    };
    let text = |bytes: Vec<u8>| match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    };
    let Some(column) = column else {
        return Value::Text(text(bytes));
    };

    match column.column_type() {
        MYSQL_TYPE_TINY | MYSQL_TYPE_SHORT | MYSQL_TYPE_INT24 | MYSQL_TYPE_LONG
        | MYSQL_TYPE_LONGLONG | MYSQL_TYPE_YEAR => {
            let text = text(bytes);
            match text.parse() {
                Ok(v) => Value::Int(v),
                // Unsigned 64-bit values above i64::MAX keep full precision as decimals.
                Err(_) => Value::Decimal(text),
            }
        }
        MYSQL_TYPE_FLOAT | MYSQL_TYPE_DOUBLE => {
            let text = text(bytes);
            match text.parse::<f64>() {
                Ok(v) if v.is_finite() => Value::Float(v),
                _ => Value::Text(text),
            }
        }
        MYSQL_TYPE_DECIMAL | MYSQL_TYPE_NEWDECIMAL => Value::Decimal(text(bytes)),
        MYSQL_TYPE_DATE
        | MYSQL_TYPE_NEWDATE
        | MYSQL_TYPE_TIME
        | MYSQL_TYPE_TIME2
        | MYSQL_TYPE_DATETIME
        | MYSQL_TYPE_DATETIME2
        | MYSQL_TYPE_TIMESTAMP
        | MYSQL_TYPE_TIMESTAMP2 => Value::Temporal(text(bytes)),
        MYSQL_TYPE_JSON => match serde_json::from_slice(&bytes) {
            Ok(json) => Value::Json(json),
            Err(_) => Value::Text(text(bytes)),
        },
        MYSQL_TYPE_BIT => Value::Bytes(bytes),
        MYSQL_TYPE_GEOMETRY => Value::Bytes(bytes),
        _ if is_binary(column) => Value::Bytes(bytes),
        _ => Value::Text(text(bytes)),
    }
}

fn query_err(e: mysql_async::Error) -> Error {
    let mysql_async::Error::Server(server) = &e else {
        return Error::Query(e.to_string());
    };
    Error::Database {
        code: Some(format!("{} ({})", server.code, server.state)),
        // Syntax errors end with "... at line N"; that is all the position MySQL gives.
        line: server
            .message
            .rsplit_once(" at line ")
            .and_then(|(_, line)| line.trim().parse().ok()),
        message: server.message.clone(),
        position: None,
        hint: None,
        detail: None,
    }
}
