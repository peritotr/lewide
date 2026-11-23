//! SQLite driver. SQLite is synchronous, so every call runs on the blocking pool.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use dbclient_core::{
    Capabilities, Column, ColumnDescription, Connection, ConnectionConfig, DataModel, DbObject,
    Driver, DriverInfo, Error, ForeignKey, IndexDescription, Namespace, ObjectDescription,
    ObjectKind, QueryRequest, QueryResult, Reference, Result, TriggerDescription, Value,
};
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::types::ValueRef;
use rusqlite::{Batch, InterruptHandle, OpenFlags, Statement};

pub struct SqliteDriver;

#[async_trait]
impl Driver for SqliteDriver {
    fn info(&self) -> DriverInfo {
        DriverInfo {
            id: "sqlite",
            name: "SQLite",
            model: DataModel::Relational,
            capabilities: Capabilities {
                sql: true,
                transactions: true,
                cancel: true,
                file_based: true,
                ..Capabilities::default()
            },
            default_port: None,
            desktop_only: false,
        }
    }

    async fn connect(&self, config: &ConnectionConfig) -> Result<Box<dyn Connection>> {
        let path = config
            .database
            .clone()
            .filter(|p| !p.trim().is_empty())
            .ok_or_else(|| Error::InvalidConfig("a database file path is required".into()))?;
        let create = path == ":memory:" || config.params.get("create").is_some_and(|v| v == "true");

        let conn = blocking(move || {
            // Without CREATE a mistyped path fails instead of leaving an empty file behind.
            let mut flags = OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NO_MUTEX;
            if create {
                flags |= OpenFlags::SQLITE_OPEN_CREATE;
            }
            rusqlite::Connection::open_with_flags(&path, flags)
                .map_err(|e| Error::Connect(format!("{path}: {e}")))
        })
        .await?;

        let interrupt = conn.get_interrupt_handle();
        Ok(Box::new(SqliteConnection {
            conn: Arc::new(Mutex::new(conn)),
            interrupt,
        }))
    }
}

struct SqliteConnection {
    conn: Arc<Mutex<rusqlite::Connection>>,
    interrupt: InterruptHandle,
}

impl SqliteConnection {
    async fn with_conn<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&rusqlite::Connection) -> Result<T> + Send + 'static,
    {
        let conn = self.conn.clone();
        blocking(move || {
            let guard = conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            f(&guard)
        })
        .await
    }
}

#[async_trait]
impl Connection for SqliteConnection {
    async fn ping(&self) -> Result<()> {
        self.with_conn(|c| c.query_row("SELECT 1", [], |_| Ok(())).map_err(query_err))
            .await
    }

    async fn execute_all(&self, request: QueryRequest) -> Result<Vec<QueryResult>> {
        self.with_conn(move |c| {
            let mut batch = Batch::new(c, &request.statement);
            let mut sets = Vec::new();
            loop {
                let started = Instant::now();
                let Some(mut stmt) = batch.next().map_err(query_err)? else {
                    break;
                };
                let mut set = run_statement(&mut stmt, request.max_rows)?;
                set.elapsed_ms = started.elapsed().as_millis() as u64;
                sets.push(set);
            }
            if sets.is_empty() {
                return Err(Error::Query("no statement to run".into()));
            }
            Ok(sets)
        })
        .await
    }

    async fn server_version(&self) -> Result<Option<String>> {
        Ok(Some(rusqlite::version().to_owned()))
    }

    async fn current_namespace(&self) -> Result<Option<String>> {
        Ok(Some("main".into()))
    }

    async fn list_namespaces(&self) -> Result<Vec<Namespace>> {
        self.with_conn(|c| {
            let mut stmt = c
                .prepare("SELECT name FROM pragma_database_list")
                .map_err(query_err)?;
            let names = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(query_err)?
                .map(|name| name.map(|name| Namespace { name }))
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(query_err)?;
            Ok(names)
        })
        .await
    }

    async fn list_objects(&self, namespace: Option<&str>) -> Result<Vec<DbObject>> {
        let schema = quote_ident(namespace.unwrap_or("main"));
        self.with_conn(move |c| {
            let sql = format!(
                "SELECT name, type FROM {schema}.sqlite_schema \
                 WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY name"
            );
            let mut stmt = c.prepare(&sql).map_err(query_err)?;
            let objects = stmt
                .query_map([], |row| {
                    let kind: String = row.get(1)?;
                    Ok(DbObject {
                        name: row.get(0)?,
                        kind: if kind == "view" {
                            ObjectKind::View
                        } else {
                            ObjectKind::Table
                        },
                        // SQLite keeps no row count.
                        row_estimate: None,
                    })
                })
                .map_err(query_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(query_err)?;
            Ok(objects)
        })
        .await
    }

    async fn describe_object(
        &self,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<ObjectDescription> {
        let schema = namespace.unwrap_or("main").to_owned();
        let name = name.to_owned();
        self.with_conn(move |c| {
            let mut stmt = c
                .prepare(
                    "SELECT name, type, \"notnull\", dflt_value, pk \
                     FROM pragma_table_info(?1, ?2) ORDER BY cid",
                )
                .map_err(query_err)?;
            let columns = stmt
                .query_map([&name, &schema], |row| {
                    let type_name: String = row.get(1)?;
                    Ok(ColumnDescription {
                        name: row.get(0)?,
                        type_name: Some(type_name).filter(|t| !t.is_empty()),
                        nullable: row.get::<_, i64>(2)? == 0,
                        default_value: row.get(3)?,
                        primary_key: row.get::<_, i64>(4)? > 0,
                        // SQLite has no column comments and no enumeration types.
                        comment: None,
                        enum_values: None,
                    })
                })
                .map_err(query_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(query_err)?;
            if columns.is_empty() {
                return Err(Error::Query(format!(
                    "no such table or view: {schema}.{name}"
                )));
            }
            Ok(ObjectDescription { name, columns })
        })
        .await
    }

    async fn list_indexes(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<IndexDescription>> {
        let schema = namespace.unwrap_or("main").to_owned();
        let table = table.to_owned();
        self.with_conn(move |c| {
            let mut list = c
                .prepare(
                    "SELECT name, \"unique\", origin, partial FROM pragma_index_list(?1, ?2) ORDER BY name",
                )
                .map_err(query_err)?;
            let found = list
                .query_map([&table, &schema], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                })
                .map_err(query_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(query_err)?;

            let mut columns = c
                .prepare("SELECT name FROM pragma_index_info(?1, ?2) ORDER BY seqno")
                .map_err(query_err)?;
            // The statement that made a partial index holds its predicate.
            let mut source = c
                .prepare(&format!(
                    "SELECT sql FROM {}.sqlite_schema WHERE type = 'index' AND name = ?1",
                    quote_ident(&schema)
                ))
                .map_err(query_err)?;
            found
                .into_iter()
                .map(|(name, unique, origin, partial)| {
                    let keys = columns
                        .query_map([&name, &schema], |row| row.get::<_, Option<String>>(0))
                        .map_err(query_err)?
                        // A NULL name is an expression or the rowid.
                        .map(|key| key.map(|k| k.unwrap_or_else(|| "(expression)".to_owned())))
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .map_err(query_err)?;
                    let condition = if partial != 0 {
                        source
                            .query_row([&name], |row| row.get::<_, Option<String>>(0))
                            .ok()
                            .flatten()
                            .and_then(|sql| {
                                let at = sql.to_ascii_uppercase().rfind(" WHERE ")?;
                                Some(sql[at + 7..].trim().to_owned())
                            })
                    } else {
                        None
                    };
                    Ok(IndexDescription {
                        name,
                        columns: keys,
                        unique: unique != 0,
                        primary: origin == "pk",
                        method: None,
                        size: None,
                        condition,
                    })
                })
                .collect()
        })
        .await
    }

    async fn list_foreign_keys(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<ForeignKey>> {
        let schema = namespace.unwrap_or("main").to_owned();
        let table = table.to_owned();
        self.with_conn(move |c| {
            // One row per column; rows of one key share its id.
            let mut list = c
                .prepare(
                    "SELECT id, \"from\", \"table\", \"to\", on_update, on_delete \
                     FROM pragma_foreign_key_list(?1, ?2) ORDER BY id, seq",
                )
                .map_err(query_err)?;
            let rows = list
                .query_map([&table, &schema], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                })
                .map_err(query_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(query_err)?;

            let mut keys: Vec<(i64, ForeignKey)> = Vec::new();
            for (id, from, target, to, on_update, on_delete) in rows {
                if keys.last().is_none_or(|(last, _)| *last != id) {
                    keys.push((
                        id,
                        ForeignKey {
                            name: None,
                            columns: Vec::new(),
                            referenced_namespace: None,
                            referenced_table: target,
                            referenced_columns: Vec::new(),
                            on_update: Some(on_update),
                            on_delete: Some(on_delete),
                        },
                    ));
                }
                let key = &mut keys.last_mut().expect("just pushed").1;
                key.columns.push(from);
                // NULL: the key points at the primary key without naming it.
                key.referenced_columns.extend(to);
            }
            Ok(keys.into_iter().map(|(_, key)| key).collect())
        })
        .await
    }

    async fn list_referencing(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<Reference>> {
        let schema = namespace.unwrap_or("main").to_owned();
        let table = table.to_owned();
        self.with_conn(move |c| {
            // Every table of the schema is asked which of its keys point here.
            let mut tables = c
                .prepare(&format!(
                    "SELECT name FROM {}.sqlite_schema WHERE type = 'table' ORDER BY name",
                    quote_ident(&schema)
                ))
                .map_err(query_err)?;
            let names = tables
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(query_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(query_err)?;
            let mut keys = c
                .prepare(
                    "SELECT id, \"from\", \"to\", on_update, on_delete \
                     FROM pragma_foreign_key_list(?1, ?2) WHERE \"table\" = ?3 ORDER BY id, seq",
                )
                .map_err(query_err)?;
            let mut references: Vec<(i64, Reference)> = Vec::new();
            for from in names {
                let rows = keys
                    .query_map([&from, &schema, &table], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                        ))
                    })
                    .map_err(query_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(query_err)?;
                let mut last: Option<i64> = None;
                for (id, column, target, on_update, on_delete) in rows {
                    if last != Some(id) {
                        last = Some(id);
                        references.push((
                            id,
                            Reference {
                                name: None,
                                namespace: None,
                                table: from.clone(),
                                columns: Vec::new(),
                                referenced_columns: Vec::new(),
                                on_update: Some(on_update),
                                on_delete: Some(on_delete),
                            },
                        ));
                    }
                    let reference = &mut references.last_mut().expect("just pushed").1;
                    reference.columns.push(column);
                    reference.referenced_columns.extend(target);
                }
            }
            Ok(references.into_iter().map(|(_, r)| r).collect())
        })
        .await
    }

    async fn list_triggers(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<TriggerDescription>> {
        let schema = namespace.unwrap_or("main").to_owned();
        let table = table.to_owned();
        self.with_conn(move |c| {
            let mut list = c
                .prepare(&format!(
                    "SELECT name, sql FROM {}.sqlite_schema \
                     WHERE type = 'trigger' AND tbl_name = ?1 ORDER BY name",
                    quote_ident(&schema)
                ))
                .map_err(query_err)?;
            list.query_map([&table], |row| {
                Ok(TriggerDescription {
                    name: row.get(0)?,
                    // SQLite keeps only the statement; timing and events are in it.
                    timing: None,
                    events: Vec::new(),
                    definition: row.get(1)?,
                })
            })
            .map_err(query_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(query_err)
        })
        .await
    }

    async fn object_ddl(&self, namespace: Option<&str>, name: &str) -> Result<Option<String>> {
        let schema = namespace.unwrap_or("main").to_owned();
        let name = name.to_owned();
        self.with_conn(move |c| {
            // The object first, then what was created on it. Indexes SQLite made
            // itself for a constraint have no statement and are left out.
            let mut list = c
                .prepare(&format!(
                    "SELECT sql FROM {}.sqlite_schema \
                     WHERE (name = ?1 OR tbl_name = ?1) AND sql IS NOT NULL \
                     ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'view' THEN 0 \
                                        WHEN 'index' THEN 1 ELSE 2 END, name",
                    quote_ident(&schema)
                ))
                .map_err(query_err)?;
            let statements = list
                .query_map([&name], |row| row.get::<_, String>(0))
                .map_err(query_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(query_err)?;
            if statements.is_empty() {
                return Ok(None);
            }
            Ok(Some(
                statements
                    .iter()
                    .map(|statement| format!("{statement};"))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ))
        })
        .await
    }

    fn quote_ident(&self, name: &str) -> String {
        quote_ident(name)
    }

    fn dialect(&self) -> dbclient_core::SqlDialect {
        dbclient_core::SqlDialect::Sqlite
    }

    async fn in_transaction(&self) -> Result<bool> {
        self.with_conn(|c| Ok(!c.is_autocommit())).await
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
        // The file handle is released when the last reference is dropped.
        Ok(())
    }
}

fn run_statement(stmt: &mut Statement<'_>, max_rows: Option<usize>) -> Result<QueryResult> {
    let column_count = stmt.column_count();
    if column_count == 0 {
        let affected = stmt.execute([]).map_err(query_err)?;
        return Ok(QueryResult {
            columns: Vec::new(),
            rows: Vec::new(),
            rows_affected: Some(affected as u64),
            truncated: false,
            elapsed_ms: 0,
        });
    }

    let columns = stmt
        .columns()
        .map(|c| Column {
            name: c.name().to_owned(),
            type_name: c.decl_type().map(str::to_owned),
        })
        .collect();

    let mut out = Vec::new();
    let mut truncated = false;
    let mut rows = stmt.query([]).map_err(query_err)?;
    while let Some(row) = rows.next().map_err(query_err)? {
        if max_rows.is_some_and(|max| out.len() >= max) {
            truncated = true;
            break;
        }
        let mut cells = Vec::with_capacity(column_count);
        for i in 0..column_count {
            cells.push(match row.get_ref(i).map_err(query_err)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(v) => Value::Int(v),
                // The infinities have no JSON form, so they are shown as text.
                ValueRef::Real(v) if v.is_finite() => Value::Float(v),
                ValueRef::Real(v) => Value::Text(v.to_string()),
                ValueRef::Text(v) => Value::Text(String::from_utf8_lossy(v).into_owned()),
                ValueRef::Blob(v) => Value::Bytes(v.to_vec()),
            });
        }
        out.push(cells);
    }

    Ok(QueryResult {
        columns,
        rows: out,
        rows_affected: None,
        truncated,
        elapsed_ms: 0,
    })
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

fn query_err(e: rusqlite::Error) -> Error {
    match &e {
        rusqlite::Error::SqlInputError { msg, offset, .. } => Error::Database {
            code: None,
            message: msg.clone(),
            // SQLite counts from zero, and reports -1 when it has no position.
            position: u32::try_from(*offset).ok().map(|o| o + 1),
            line: None,
            hint: None,
            detail: None,
        },
        rusqlite::Error::SqliteFailure(failure, message) => Error::Database {
            code: Some(format!("{:?}", failure.code)),
            message: message.clone().unwrap_or_else(|| failure.to_string()),
            position: None,
            line: None,
            hint: None,
            detail: None,
        },
        _ => Error::Query(e.to_string()),
    }
}

fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn memory() -> Box<dyn Connection> {
        let config = ConnectionConfig {
            driver: "sqlite".into(),
            database: Some(":memory:".into()),
            ..ConnectionConfig::default()
        };
        SqliteDriver.connect(&config).await.unwrap()
    }

    /// Names and kinds only: row estimates vary from run to run.
    fn kinds(objects: Vec<DbObject>) -> Vec<(String, ObjectKind)> {
        objects.into_iter().map(|o| (o.name, o.kind)).collect()
    }

    fn query(statement: &str, max_rows: Option<usize>) -> QueryRequest {
        QueryRequest {
            statement: statement.into(),
            max_rows,
        }
    }

    #[tokio::test]
    async fn reads_every_value_type() {
        let conn = memory().await;
        let result = conn
            .execute(query("SELECT NULL, 42, 1.5, 'text', x'00ff'", None))
            .await
            .unwrap();
        assert_eq!(
            result.rows,
            vec![vec![
                Value::Null,
                Value::Int(42),
                Value::Float(1.5),
                Value::Text("text".into()),
                Value::Bytes(vec![0, 255]),
            ]]
        );
        assert_eq!(result.rows_affected, None);

        let infinite = conn.execute(query("SELECT 9e999", None)).await.unwrap();
        assert_eq!(infinite.rows[0][0], Value::Text("inf".into()));
    }

    #[tokio::test]
    async fn runs_all_statements_and_returns_the_last_result() {
        let conn = memory().await;
        let result = conn
            .execute(query(
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
                 INSERT INTO users (name) VALUES ('ada'), ('linus');
                 SELECT id, name FROM users ORDER BY id;",
                None,
            ))
            .await
            .unwrap();
        assert_eq!(result.columns[0].type_name.as_deref(), Some("INTEGER"));
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[1][1], Value::Text("linus".into()));
    }

    #[tokio::test]
    async fn reports_affected_rows() {
        let conn = memory().await;
        conn.execute(query("CREATE TABLE t (x)", None))
            .await
            .unwrap();
        let result = conn
            .execute(query("INSERT INTO t VALUES (1), (2), (3)", None))
            .await
            .unwrap();
        assert_eq!(result.rows_affected, Some(3));
    }

    #[tokio::test]
    async fn truncates_at_max_rows() {
        let conn = memory().await;
        let sql = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 10) \
                   SELECT i FROM n";
        let limited = conn.execute(query(sql, Some(4))).await.unwrap();
        assert_eq!(limited.rows.len(), 4);
        assert!(limited.truncated);

        let exact = conn.execute(query(sql, Some(10))).await.unwrap();
        assert_eq!(exact.rows.len(), 10);
        assert!(!exact.truncated);
    }

    #[tokio::test]
    async fn lists_and_describes_objects() {
        let conn = memory().await;
        conn.execute(query(
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, total REAL NOT NULL DEFAULT 0, note TEXT);
             CREATE VIEW big_orders AS SELECT * FROM orders WHERE total > 100;",
            None,
        ))
        .await
        .unwrap();

        assert_eq!(
            conn.current_namespace().await.unwrap().as_deref(),
            Some("main")
        );
        let namespaces = conn.list_namespaces().await.unwrap();
        assert!(namespaces.iter().any(|n| n.name == "main"));

        let objects = kinds(conn.list_objects(None).await.unwrap());
        assert_eq!(
            objects,
            vec![
                ("big_orders".to_string(), ObjectKind::View),
                ("orders".to_string(), ObjectKind::Table),
            ]
        );

        let table = conn.describe_object(None, "orders").await.unwrap();
        assert_eq!(table.columns.len(), 3);
        assert!(table.columns[0].primary_key);
        assert!(!table.columns[1].nullable);
        assert_eq!(table.columns[1].default_value.as_deref(), Some("0"));
        assert!(table.columns[2].nullable);

        assert!(conn.describe_object(None, "missing").await.is_err());
    }

    #[tokio::test]
    async fn sql_errors_are_query_errors() {
        let conn = memory().await;
        let err = conn
            .execute(query("SELECT * FROM nope", None))
            .await
            .unwrap_err();
        let Error::Database {
            message, position, ..
        } = err
        else {
            panic!("unexpected error: {err:?}")
        };
        assert!(message.contains("nope"), "{message}");
        // An unknown table has no position; a syntax error does.
        assert_eq!(position, None);
        let syntax = conn
            .execute(query("SELECT 1 FROM WHERE", None))
            .await
            .unwrap_err();
        assert!(
            matches!(
                syntax,
                Error::Database {
                    position: Some(15),
                    ..
                }
            ),
            "{syntax:?}"
        );
    }

    #[tokio::test]
    async fn missing_file_is_not_created() {
        let path = std::env::temp_dir().join("dbclient-test-does-not-exist.sqlite");
        let _ = std::fs::remove_file(&path);
        let config = ConnectionConfig {
            driver: "sqlite".into(),
            database: Some(path.to_string_lossy().into_owned()),
            ..ConnectionConfig::default()
        };
        let err = SqliteDriver.connect(&config).await.err().unwrap();
        assert!(matches!(err, Error::Connect(_)));
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn execute_all_returns_one_result_per_statement() {
        let conn = memory().await;
        let sets = conn
            .execute_all(query(
                "CREATE TABLE t (x); INSERT INTO t VALUES (1), (2), (3); SELECT x FROM t; SELECT count(*) FROM t",
                Some(2),
            ))
            .await
            .unwrap();
        assert_eq!(sets.len(), 4);
        assert_eq!(sets[1].rows_affected, Some(3));
        assert_eq!(sets[2].rows.len(), 2);
        assert!(sets[2].truncated);
        assert_eq!(sets[3].rows, vec![vec![Value::Int(3)]]);
        assert!(conn.server_version().await.unwrap().is_some());
    }
    #[tokio::test]
    async fn pages_through_a_table_in_a_stable_order() {
        use dbclient_core::{SortColumn, TableRequest};

        let conn = memory().await;
        conn.execute(query(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, grp TEXT, note TEXT);
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 25)
             INSERT INTO items SELECT i, CASE WHEN i % 2 = 0 THEN 'even' ELSE 'odd' END, 'n' || i FROM n;",
            None,
        ))
        .await
        .unwrap();
        let page = |offset, order_by: Vec<SortColumn>, filter: Option<&str>| TableRequest {
            namespace: None,
            table: "items".into(),
            offset,
            limit: 10,
            order_by,
            filter: filter.map(str::to_owned),
        };
        let ids = |rows: &[Vec<Value>]| rows.iter().map(|r| r[0].clone()).collect::<Vec<_>>();

        let first = conn.fetch_rows(page(0, vec![], None)).await.unwrap();
        assert_eq!(first.columns.len(), 3);
        assert!(first.columns[0].primary_key);
        assert_eq!(first.rows.len(), 10);
        assert!(first.has_more);
        assert_eq!(first.rows[0][0], Value::Int(1));

        let last = conn.fetch_rows(page(20, vec![], None)).await.unwrap();
        assert_eq!(
            ids(&last.rows),
            (21..=25).map(Value::Int).collect::<Vec<_>>()
        );
        assert!(!last.has_more);

        // Sorting by a column with ties still gives a total order, through the primary key.
        let sorted = vec![SortColumn {
            column: "grp".into(),
            descending: true,
        }];
        let odd = conn
            .fetch_rows(page(0, sorted.clone(), None))
            .await
            .unwrap();
        assert_eq!(
            ids(&odd.rows),
            [1, 3, 5, 7, 9, 11, 13, 15, 17, 19].map(Value::Int).to_vec()
        );
        let next = conn.fetch_rows(page(10, sorted, None)).await.unwrap();
        assert_eq!(ids(&next.rows)[..4], [21, 23, 25, 2].map(Value::Int));

        let filtered = conn
            .fetch_rows(page(0, vec![], Some("id > 20 -- trailing comment")))
            .await
            .unwrap();
        assert_eq!(filtered.rows.len(), 5);
        assert_eq!(conn.count_rows(None, "items", None).await.unwrap(), 25);
        assert_eq!(
            conn.count_rows(None, "items", Some("grp = 'even'"))
                .await
                .unwrap(),
            12
        );

        // A name that is not a column never reaches the statement, and a filter is one condition.
        let bogus = vec![SortColumn {
            column: "id; DROP TABLE items".into(),
            descending: false,
        }];
        assert!(conn.fetch_rows(page(0, bogus, None)).await.is_err());
        assert!(
            conn.fetch_rows(page(0, vec![], Some("1 = 1; DROP TABLE items")))
                .await
                .is_err()
        );
        assert_eq!(conn.count_rows(None, "items", None).await.unwrap(), 25);
        assert!(
            conn.fetch_rows(TableRequest {
                table: "missing".into(),
                ..page(0, vec![], None)
            })
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn lists_indexes() {
        let conn = memory().await;
        conn.execute(query(
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INT, email TEXT UNIQUE, total REAL);
             CREATE INDEX orders_user ON orders (user_id, total);",
            None,
        ))
        .await
        .unwrap();
        let indexes = conn.list_indexes(None, "orders").await.unwrap();
        let by_user = indexes.iter().find(|i| i.name == "orders_user").unwrap();
        assert_eq!(by_user.columns, ["user_id", "total"]);
        assert!(!by_user.unique);
        // The UNIQUE constraint is backed by an automatic index.
        assert!(indexes.iter().any(|i| i.unique && i.columns == ["email"]));
    }

    #[tokio::test]
    async fn describes_the_structure_of_a_table() {
        let conn = memory().await;
        conn.execute(query(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT);
             CREATE TABLE regions (country TEXT, code TEXT, PRIMARY KEY (country, code));
             CREATE TABLE orders (
                 id INTEGER PRIMARY KEY,
                 user_id INT REFERENCES users ON DELETE CASCADE,
                 country TEXT, region TEXT, note TEXT UNIQUE,
                 FOREIGN KEY (country, region) REFERENCES regions (country, code) ON UPDATE SET NULL
             );
             CREATE INDEX orders_user ON orders (user_id);
             CREATE TRIGGER orders_touch AFTER UPDATE ON orders BEGIN SELECT 1; END;
             CREATE VIEW big_orders AS SELECT * FROM orders WHERE id > 100;",
            None,
        ))
        .await
        .unwrap();

        let structure = conn.table_structure(None, "orders").await.unwrap();
        assert_eq!(structure.columns.len(), 5);
        assert_eq!(structure.indexes.len(), 2);

        let mut keys = structure.foreign_keys;
        keys.sort_by(|a, b| a.referenced_table.cmp(&b.referenced_table));
        assert_eq!(keys.len(), 2, "{keys:?}");
        assert_eq!(keys[0].columns, ["country", "region"]);
        assert_eq!(keys[0].referenced_table, "regions");
        assert_eq!(keys[0].referenced_columns, ["country", "code"]);
        assert_eq!(keys[0].on_update.as_deref(), Some("SET NULL"));
        assert_eq!(keys[0].on_delete.as_deref(), Some("NO ACTION"));
        assert_eq!(keys[1].columns, ["user_id"]);
        // Points at the primary key of users without naming it.
        assert!(keys[1].referenced_columns.is_empty());
        assert_eq!(keys[1].on_delete.as_deref(), Some("CASCADE"));

        assert_eq!(structure.triggers.len(), 1);
        assert_eq!(structure.triggers[0].name, "orders_touch");
        assert!(
            structure.triggers[0]
                .definition
                .as_deref()
                .unwrap()
                .starts_with("CREATE TRIGGER orders_touch")
        );

        // Table first, then its index and trigger; the automatic index has no statement.
        let ddl = structure.ddl.unwrap();
        let statements: Vec<&str> = ddl.split(";\n\n").collect();
        assert_eq!(statements.len(), 3, "{ddl}");
        assert!(statements[0].starts_with("CREATE TABLE orders"));
        assert!(statements[1].starts_with("CREATE INDEX orders_user"));
        assert!(statements[2].starts_with("CREATE TRIGGER orders_touch"));

        // The DDL builds the same table again in an empty database.
        let copy = memory().await;
        copy.execute(query("PRAGMA foreign_keys = OFF", None))
            .await
            .unwrap();
        copy.execute(query(&ddl, None)).await.unwrap();
        let again = copy.table_structure(None, "orders").await.unwrap();
        assert_eq!(again.columns, structure.columns);
        assert_eq!(again.indexes, structure.indexes);
        assert_eq!(again.ddl.as_deref(), Some(ddl.as_str()));

        let view = conn.table_structure(None, "big_orders").await.unwrap();
        assert!(view.ddl.unwrap().starts_with("CREATE VIEW big_orders"));
        assert!(view.foreign_keys.is_empty() && view.triggers.is_empty());
        assert_eq!(conn.object_ddl(None, "missing").await.unwrap(), None);
    }

    #[tokio::test]
    async fn reports_partial_indexes_and_who_references_a_table() {
        let conn = memory().await;
        conn.execute(query(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT, active INT);
             CREATE INDEX users_active_email ON users (email) WHERE active = 1;
             CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INT REFERENCES users (id) ON DELETE CASCADE);
             CREATE TABLE notes (id INTEGER PRIMARY KEY, author INT, FOREIGN KEY (author) REFERENCES users);",
            None,
        ))
        .await
        .unwrap();
        let indexes = conn.list_indexes(None, "users").await.unwrap();
        assert_eq!(indexes[0].condition.as_deref(), Some("active = 1"));

        let references = conn.list_referencing(None, "users").await.unwrap();
        assert_eq!(references.len(), 2, "{references:?}");
        assert_eq!(references[0].table, "notes");
        assert_eq!(references[0].columns, ["author"]);
        assert!(references[0].referenced_columns.is_empty());
        assert_eq!(references[1].table, "orders");
        assert_eq!(references[1].referenced_columns, ["id"]);
        assert_eq!(references[1].on_delete.as_deref(), Some("CASCADE"));
        assert_eq!(
            conn.table_structure(None, "users")
                .await
                .unwrap()
                .referenced_by,
            references
        );
        assert!(
            conn.list_referencing(None, "orders")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn applies_row_changes_as_one_unit() {
        use dbclient_core::edit::Cell;
        use dbclient_core::{ChangeSet, RowChange};

        let conn = memory().await;
        conn.execute(query(
            "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL, photo BLOB, score REAL);
             CREATE TABLE loose (x);
             INSERT INTO people VALUES (1, 'Ada', NULL, 1.5), (2, 'Linus', NULL, 2.5);",
            None,
        ))
        .await
        .unwrap();
        let cell = |column: &str, value: Value| Cell {
            column: column.into(),
            value,
        };
        let key = |id: i64| vec![cell("id", Value::Int(id))];
        let set = |changes: Vec<RowChange>| ChangeSet {
            namespace: None,
            table: "people".into(),
            changes,
        };
        let rows = || async {
            conn.execute(query(
                "SELECT id, name, photo, score FROM people ORDER BY id",
                None,
            ))
            .await
            .unwrap()
            .rows
        };

        let tricky = "O'Brien \\ ; DROP TABLE people; --";
        conn.apply_changes(&set(vec![
            RowChange::Insert {
                values: vec![
                    cell("id", Value::Int(3)),
                    cell("name", Value::Text(tricky.into())),
                    cell("photo", Value::Bytes(vec![0, 255])),
                    cell("score", Value::Float(0.25)),
                ],
            },
            RowChange::Update {
                key: key(1),
                set: vec![cell("score", Value::Null)],
            },
            RowChange::Delete { key: key(2) },
        ]))
        .await
        .unwrap();
        assert_eq!(
            rows().await,
            vec![
                vec![
                    Value::Int(1),
                    Value::Text("Ada".into()),
                    Value::Null,
                    Value::Null
                ],
                vec![
                    Value::Int(3),
                    Value::Text(tricky.into()),
                    Value::Bytes(vec![0, 255]),
                    Value::Float(0.25)
                ],
            ]
        );

        // A change that hits no row undoes the ones before it.
        let stale = set(vec![
            RowChange::Update {
                key: key(1),
                set: vec![cell("name", Value::Text("gone".into()))],
            },
            RowChange::Delete { key: key(99) },
        ]);
        assert!(
            conn.apply_changes(&stale)
                .await
                .unwrap_err()
                .to_string()
                .contains("Nothing was applied")
        );
        assert_eq!(rows().await[0][1], Value::Text("Ada".into()));
        assert!(!conn.in_transaction().await.unwrap());

        // No primary key, no edits; and an open transaction is not touched.
        let no_key = ChangeSet {
            namespace: None,
            table: "loose".into(),
            changes: vec![RowChange::Delete {
                key: vec![cell("x", Value::Int(1))],
            }],
        };
        assert!(
            conn.apply_changes(&no_key)
                .await
                .unwrap_err()
                .to_string()
                .contains("no primary key")
        );
        conn.execute(query("BEGIN", None)).await.unwrap();
        assert!(conn.in_transaction().await.unwrap());
        assert!(
            conn.apply_changes(&set(vec![RowChange::Delete { key: key(1) }]))
                .await
                .unwrap_err()
                .to_string()
                .contains("transaction is open")
        );
        conn.execute(query("ROLLBACK", None)).await.unwrap();
        assert_eq!(rows().await.len(), 2);
    }
}
