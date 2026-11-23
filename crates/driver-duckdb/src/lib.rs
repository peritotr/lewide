//! DuckDB, embedded: the library is built into the application and opens a
//! file (or `:memory:`) directly.
//!
//! Extensions are never installed or loaded behind the user's back: the
//! automatic download DuckDB does when a query needs one is turned off, so
//! the application fetches no code. The Parquet and JSON readers are built in.
//!
//! The library is synchronous; every call runs on a blocking thread, one at a
//! time per connection, and `cancel` interrupts the one running.

mod convert;
mod split;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use dbclient_core::{
    Capabilities, Column, ColumnDescription, Connection, ConnectionConfig, DataModel, DbObject,
    Driver, DriverInfo, Error, ForeignKey, IndexDescription, Namespace, ObjectDescription,
    ObjectKind, QueryRequest, QueryResult, Reference, Result, SqlDialect, Value,
};
use duckdb::types::Value as Duck;
use duckdb::{Config, InterruptHandle, ToSql};

use convert::{database_error, has_zone, to_value, type_name};

pub struct DuckDbDriver;

#[async_trait]
impl Driver for DuckDbDriver {
    fn info(&self) -> DriverInfo {
        DriverInfo {
            id: "duckdb",
            name: "DuckDB",
            model: DataModel::Relational,
            capabilities: Capabilities {
                sql: true,
                schemas: true,
                transactions: true,
                cancel: true,
                file_based: true,
                ..Capabilities::default()
            },
            default_port: None,
            // The library is native code built for the desktop.
            desktop_only: true,
        }
    }

    async fn connect(&self, config: &ConnectionConfig) -> Result<Box<dyn Connection>> {
        let path = config
            .database
            .clone()
            .filter(|p| !p.trim().is_empty())
            .ok_or_else(|| Error::InvalidConfig("a database file path is required".into()))?;
        // Without "create", a mistyped path fails instead of leaving an empty file behind.
        let create = path == ":memory:" || config.params.get("create").is_some_and(|v| v == "true");
        if !create && !std::path::Path::new(&path).is_file() {
            return Err(Error::Connect(format!("{path}: no such file")));
        }
        let conn = blocking(move || {
            let settings = Config::default()
                .enable_autoload_extension(false)
                .map_err(|e| Error::Internal(e.to_string()))?;
            duckdb::Connection::open_with_flags(&path, settings)
                .map_err(|e| Error::Connect(format!("{path}: {e}")))
        })
        .await?;
        let interrupt = conn.interrupt_handle();
        Ok(Box::new(DuckDbConnection {
            conn: Arc::new(Mutex::new(Some(conn))),
            interrupt,
            in_transaction: Arc::new(AtomicBool::new(false)),
        }))
    }
}

struct DuckDbConnection {
    /// `None` once closed.
    conn: Arc<Mutex<Option<duckdb::Connection>>>,
    interrupt: Arc<InterruptHandle>,
    /// Whether a transaction is open. The library's own `is_autocommit` does
    /// not follow BEGIN and COMMIT, so the statements that change it are watched.
    in_transaction: Arc<AtomicBool>,
}

impl DuckDbConnection {
    async fn with_conn<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&duckdb::Connection) -> Result<T> + Send + 'static,
    {
        let conn = self.conn.clone();
        blocking(move || {
            let guard = conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let conn = guard
                .as_ref()
                .ok_or_else(|| Error::ConnectionLost("the connection is closed".into()))?;
            f(conn)
        })
        .await
    }

    /// A catalog query with `?` parameters, its rows as values.
    async fn catalog(&self, sql: &'static str, params: Vec<String>) -> Result<Vec<Vec<Value>>> {
        self.with_conn(move |conn| {
            let params: Vec<&dyn ToSql> = params.iter().map(|p| p as &dyn ToSql).collect();
            let result = run(conn, sql, sql, 0, None, &params)?;
            Ok(result.rows)
        })
        .await
    }

    async fn schema(&self, namespace: Option<&str>) -> Result<String> {
        match namespace.filter(|n| !n.is_empty()) {
            Some(n) => Ok(n.to_owned()),
            None => self
                .current_namespace()
                .await?
                .ok_or_else(|| Error::Query("no current schema".into())),
        }
    }

    async fn constraints(
        &self,
        schema: &str,
        table: &str,
        kind: &'static str,
    ) -> Result<Vec<Vec<Value>>> {
        self.catalog(
            "SELECT constraint_name, constraint_column_names, referenced_table, referenced_column_names \
             FROM duckdb_constraints() \
             WHERE database_name = current_database() AND schema_name = ? AND table_name = ? AND constraint_type = ? \
             ORDER BY constraint_index",
            vec![schema.into(), table.into(), kind.into()],
        )
        .await
    }
}

/// Runs one statement. `whole` and `offset` place an error in the editor text.
fn run(
    conn: &duckdb::Connection,
    sql: &str,
    whole: &str,
    offset: usize,
    max_rows: Option<usize>,
    params: &[&dyn ToSql],
) -> Result<QueryResult> {
    let started = Instant::now();
    let fail = |e: duckdb::Error| query_err(e, whole, offset);
    let mut stmt = conn.prepare(sql).map_err(fail)?;
    let mut rows = stmt.query(params).map_err(fail)?;
    let (columns, zones) = match rows.as_ref() {
        Some(stmt) => {
            let names = stmt.column_names();
            let types: Vec<_> = (0..names.len()).map(|i| stmt.column_type(i)).collect();
            let zones: Vec<bool> = types.iter().map(has_zone).collect();
            let columns: Vec<Column> = names
                .into_iter()
                .zip(&types)
                .map(|(name, ty)| Column {
                    name,
                    type_name: Some(type_name(ty)),
                })
                .collect();
            (columns, zones)
        }
        None => (Vec::new(), Vec::new()),
    };
    let mut out = Vec::new();
    let mut truncated = false;
    while let Some(row) = rows.next().map_err(fail)? {
        // Past the limit the rest is read and dropped, which ends the statement cleanly.
        if max_rows.is_some_and(|max| out.len() >= max) {
            truncated = true;
            continue;
        }
        let mut cells = Vec::with_capacity(columns.len());
        for (i, zone) in zones.iter().enumerate() {
            let value: Duck = row.get(i).map_err(fail)?;
            cells.push(to_value(value, *zone));
        }
        out.push(cells);
    }
    let elapsed_ms = started.elapsed().as_millis() as u64;
    // A statement that changes rows or structure answers with a "Count"
    // column: one row with the number of rows INSERT, UPDATE or DELETE
    // changed, none for CREATE and the like. That is not a result to show.
    let word = split::first_word(sql);
    let returns_rows = matches!(
        word.as_str(),
        "SELECT"
            | "WITH"
            | "FROM"
            | "VALUES"
            | "TABLE"
            | "SHOW"
            | "DESCRIBE"
            | "DESC"
            | "SUMMARIZE"
            | "EXPLAIN"
            | "PRAGMA"
            | "CALL"
            | "PIVOT"
            | "UNPIVOT"
    );
    let is_count = columns.len() == 1 && columns[0].name == "Count" && out.len() <= 1;
    if is_count && !returns_rows {
        let count = out.first().and_then(|row| match &row[0] {
            Value::Int(n) => u64::try_from(*n).ok(),
            _ => None,
        });
        let changes_rows = matches!(word.as_str(), "INSERT" | "UPDATE" | "DELETE");
        return Ok(QueryResult {
            columns: Vec::new(),
            rows: Vec::new(),
            rows_affected: if changes_rows { count } else { Some(0) },
            truncated: false,
            elapsed_ms,
        });
    }
    let rows_affected = columns.is_empty().then_some(0);
    Ok(QueryResult {
        columns,
        rows: out,
        rows_affected,
        truncated,
        elapsed_ms,
    })
}

#[async_trait]
impl Connection for DuckDbConnection {
    async fn ping(&self) -> Result<()> {
        self.catalog("SELECT 1", Vec::new()).await.map(|_| ())
    }

    async fn execute_all(&self, request: QueryRequest) -> Result<Vec<QueryResult>> {
        let open = self.in_transaction.clone();
        self.with_conn(move |conn| {
            let pieces = split::statements(&request.statement);
            if pieces.is_empty() {
                return Err(Error::Query("no statement to run".into()));
            }
            let mut out = Vec::with_capacity(pieces.len());
            for piece in &pieces {
                let result = run(
                    conn,
                    piece.text,
                    &request.statement,
                    piece.offset,
                    request.max_rows,
                    &[],
                )?;
                match split::first_word(piece.text).as_str() {
                    "BEGIN" | "START" => open.store(true, Ordering::Relaxed),
                    "COMMIT" | "END" | "ROLLBACK" | "ABORT" => open.store(false, Ordering::Relaxed),
                    _ => {}
                }
                out.push(result);
            }
            Ok(out)
        })
        .await
    }

    async fn server_version(&self) -> Result<Option<String>> {
        let rows = self.catalog("SELECT version()", Vec::new()).await?;
        Ok(rows
            .first()
            .map(|r| text(&r[0]).trim_start_matches('v').to_owned()))
    }

    async fn current_namespace(&self) -> Result<Option<String>> {
        let rows = self.catalog("SELECT current_schema()", Vec::new()).await?;
        Ok(rows.first().map(|r| text(&r[0])))
    }

    async fn list_namespaces(&self) -> Result<Vec<Namespace>> {
        let rows = self
            .catalog(
                // `main` is marked internal too, so the system schemas are named instead.
                "SELECT schema_name FROM duckdb_schemas() \
                 WHERE database_name = current_database() \
                   AND schema_name NOT IN ('information_schema', 'pg_catalog') \
                 ORDER BY schema_name",
                Vec::new(),
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| Namespace { name: text(&r[0]) })
            .collect())
    }

    async fn list_objects(&self, namespace: Option<&str>) -> Result<Vec<DbObject>> {
        let schema = self.schema(namespace).await?;
        let rows = self
            .catalog(
                "SELECT table_name, 'table', estimated_size FROM duckdb_tables() \
                 WHERE database_name = current_database() AND schema_name = ? \
                 UNION ALL \
                 SELECT view_name, 'view', NULL FROM duckdb_views() \
                 WHERE database_name = current_database() AND schema_name = ? AND NOT internal \
                 ORDER BY 1",
                vec![schema.clone(), schema],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| DbObject {
                name: text(&r[0]),
                kind: if text(&r[1]) == "view" {
                    ObjectKind::View
                } else {
                    ObjectKind::Table
                },
                row_estimate: match &r[2] {
                    Value::Int(n) => Some((*n).max(0) as u64),
                    _ => None,
                },
            })
            .collect())
    }

    async fn describe_object(
        &self,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<ObjectDescription> {
        let schema = self.schema(namespace).await?;
        let rows = self
            .catalog(
                "SELECT column_name, data_type, is_nullable, column_default, comment FROM duckdb_columns() \
                 WHERE database_name = current_database() AND schema_name = ? AND table_name = ? \
                 ORDER BY column_index",
                vec![schema.clone(), name.into()],
            )
            .await?;
        if rows.is_empty() {
            return Err(Error::Query(format!("no such table or view: {name}")));
        }
        let primary: Vec<String> = self
            .constraints(&schema, name, "PRIMARY KEY")
            .await?
            .first()
            .map(|r| names(&r[1]))
            .unwrap_or_default();
        let columns = rows
            .iter()
            .map(|r| {
                let column = text(&r[0]);
                ColumnDescription {
                    primary_key: primary.contains(&column),
                    name: column,
                    type_name: Some(text(&r[1])),
                    nullable: matches!(r[2], Value::Bool(true)),
                    default_value: Some(text(&r[3])).filter(|d| !d.is_empty()),
                    comment: Some(text(&r[4])).filter(|c| !c.is_empty()),
                    enum_values: None,
                }
            })
            .collect();
        Ok(ObjectDescription {
            name: name.to_owned(),
            columns,
        })
    }

    async fn list_indexes(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<IndexDescription>> {
        let schema = self.schema(namespace).await?;
        let mut out = Vec::new();
        for (kind, primary) in [("PRIMARY KEY", true), ("UNIQUE", false)] {
            for r in self.constraints(&schema, table, kind).await? {
                out.push(IndexDescription {
                    name: text(&r[0]),
                    columns: names(&r[1]),
                    unique: true,
                    primary,
                    method: None,
                    size: None,
                    condition: None,
                });
            }
        }
        let indexes = self
            .catalog(
                "SELECT index_name, is_unique, expressions FROM duckdb_indexes() \
                 WHERE database_name = current_database() AND schema_name = ? AND table_name = ? \
                 ORDER BY index_name",
                vec![schema, table.into()],
            )
            .await?;
        out.extend(indexes.iter().map(|r| IndexDescription {
            name: text(&r[0]),
            columns: names(&r[2]),
            unique: matches!(r[1], Value::Bool(true)),
            primary: false,
            method: Some("ART".into()),
            size: None,
            condition: None,
        }));
        Ok(out)
    }

    async fn list_foreign_keys(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<ForeignKey>> {
        let schema = self.schema(namespace).await?;
        Ok(self
            .constraints(&schema, table, "FOREIGN KEY")
            .await?
            .iter()
            .map(|r| ForeignKey {
                name: Some(text(&r[0])),
                columns: names(&r[1]),
                // DuckDB keeps foreign keys within one schema.
                referenced_namespace: Some(schema.clone()),
                referenced_table: text(&r[2]),
                referenced_columns: names(&r[3]),
                // It has no ON DELETE or ON UPDATE actions.
                on_update: None,
                on_delete: None,
            })
            .collect())
    }

    async fn list_referencing(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<Reference>> {
        let schema = self.schema(namespace).await?;
        let rows = self
            .catalog(
                "SELECT constraint_name, table_name, constraint_column_names, referenced_column_names \
                 FROM duckdb_constraints() \
                 WHERE database_name = current_database() AND schema_name = ? AND referenced_table = ? \
                   AND constraint_type = 'FOREIGN KEY' \
                 ORDER BY table_name, constraint_index",
                vec![schema.clone(), table.into()],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| Reference {
                name: Some(text(&r[0])),
                namespace: Some(schema.clone()),
                table: text(&r[1]),
                columns: names(&r[2]),
                referenced_columns: names(&r[3]),
                on_update: None,
                on_delete: None,
            })
            .collect())
    }

    async fn object_ddl(&self, namespace: Option<&str>, name: &str) -> Result<Option<String>> {
        let schema = self.schema(namespace).await?;
        let rows = self
            .catalog(
                "SELECT sql FROM duckdb_tables() WHERE database_name = current_database() AND schema_name = ? AND table_name = ? \
                 UNION ALL \
                 SELECT sql FROM duckdb_views() WHERE database_name = current_database() AND schema_name = ? AND view_name = ? \
                 UNION ALL \
                 SELECT sql FROM duckdb_indexes() WHERE database_name = current_database() AND schema_name = ? AND table_name = ? \
                   AND sql IS NOT NULL",
                vec![schema.clone(), name.into(), schema.clone(), name.into(), schema, name.into()],
            )
            .await?;
        if rows.is_empty() {
            return Ok(None);
        }
        Ok(Some(
            rows.iter()
                .map(|r| text(&r[0]).trim().trim_end_matches(';').to_owned() + ";")
                .collect::<Vec<_>>()
                .join("\n"),
        ))
    }

    fn quote_ident(&self, name: &str) -> String {
        SqlDialect::DuckDb.quote_ident(name)
    }

    async fn is_alive(&self) -> bool {
        self.ping().await.is_ok()
    }

    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }

    async fn in_transaction(&self) -> Result<bool> {
        Ok(self.in_transaction.load(Ordering::Relaxed))
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
        self.interrupt.interrupt();
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        // Dropping the connection releases the file for other programs.
        let conn = self.conn.clone();
        blocking(move || {
            conn.lock().unwrap_or_else(|p| p.into_inner()).take();
            Ok(())
        })
        .await
    }
}

async fn blocking<T, F>(f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::Internal(e.to_string()))?
}

fn query_err(e: duckdb::Error, whole: &str, offset: usize) -> Error {
    match e {
        duckdb::Error::DuckDBFailure(_, Some(message))
            if message.contains("INTERRUPT") || message.contains("Interrupted") =>
        {
            Error::Query("the statement was interrupted".into())
        }
        duckdb::Error::DuckDBFailure(_, Some(message)) => database_error(&message, whole, offset),
        other => Error::Query(other.to_string()),
    }
}

fn text(value: &Value) -> String {
    match value {
        Value::Text(s) | Value::Decimal(s) | Value::Temporal(s) => s.clone(),
        Value::Int(i) => i.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        Value::Json(j) => j.to_string(),
        other => format!("{other:?}"),
    }
}

/// A list of names, as DuckDB's catalog gives it (a LIST value, or text like `[a, b]`).
fn names(value: &Value) -> Vec<String> {
    match value {
        Value::Json(serde_json::Value::Array(items)) => items
            .iter()
            .map(|i| i.as_str().map_or_else(|| i.to_string(), str::to_owned))
            .collect(),
        Value::Text(s) => s
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(", ")
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}
