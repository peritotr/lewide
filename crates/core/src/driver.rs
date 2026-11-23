use async_trait::async_trait;

use std::time::Instant;

use crate::edit::{self, ApplyResult, ChangeSet};
use crate::sql::SqlDialect;
use crate::{
    ConnectionConfig, DbObject, DriverInfo, Error, ForeignKey, IndexDescription, Namespace,
    ObjectDescription, QueryRequest, QueryResult, Reference, Result, TablePage, TableRequest,
    TableStructure, TriggerDescription, Value,
};

/// A database type. One instance is registered per driver and shared.
#[async_trait]
pub trait Driver: Send + Sync {
    fn info(&self) -> DriverInfo;
    async fn connect(&self, config: &ConnectionConfig) -> Result<Box<dyn Connection>>;
}

/// An open session with one database.
#[async_trait]
pub trait Connection: Send + Sync {
    async fn ping(&self) -> Result<()>;

    /// Runs every statement in the text and returns one result per statement, in order.
    async fn execute_all(&self, request: QueryRequest) -> Result<Vec<QueryResult>>;

    /// Like [`execute_all`](Self::execute_all), keeping only the last statement's result.
    async fn execute(&self, request: QueryRequest) -> Result<QueryResult> {
        self.execute_all(request)
            .await?
            .pop()
            .ok_or_else(|| Error::Query("no statement to run".into()))
    }

    /// Version text as the server reports it, for display.
    async fn server_version(&self) -> Result<Option<String>> {
        Ok(None)
    }

    /// The namespace unqualified names resolve to, if the session has one.
    async fn current_namespace(&self) -> Result<Option<String>>;

    /// Top-level containers: schemas, databases or keyspaces.
    async fn list_namespaces(&self) -> Result<Vec<Namespace>>;

    /// Tables, views or collections inside `namespace` (the default one if `None`).
    async fn list_objects(&self, namespace: Option<&str>) -> Result<Vec<DbObject>>;

    async fn describe_object(
        &self,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<ObjectDescription>;

    /// The indexes of a table. Empty when the driver cannot list them.
    async fn list_indexes(
        &self,
        _namespace: Option<&str>,
        _table: &str,
    ) -> Result<Vec<IndexDescription>> {
        Ok(Vec::new())
    }

    /// The foreign keys of a table. Empty when the driver cannot list them.
    async fn list_foreign_keys(
        &self,
        _namespace: Option<&str>,
        _table: &str,
    ) -> Result<Vec<ForeignKey>> {
        Ok(Vec::new())
    }

    /// Foreign keys in other tables that point at this one. Empty when the driver cannot list them.
    async fn list_referencing(
        &self,
        _namespace: Option<&str>,
        _table: &str,
    ) -> Result<Vec<Reference>> {
        Ok(Vec::new())
    }

    /// The triggers on a table. Empty when the driver cannot list them.
    async fn list_triggers(
        &self,
        _namespace: Option<&str>,
        _table: &str,
    ) -> Result<Vec<TriggerDescription>> {
        Ok(Vec::new())
    }

    /// The statements that would create a table or view again, as the
    /// database gives them or as close as its catalog allows. `None` when
    /// the driver has no way to produce them.
    async fn object_ddl(&self, _namespace: Option<&str>, _name: &str) -> Result<Option<String>> {
        Ok(None)
    }

    /// Columns, indexes, foreign keys, triggers and DDL of one object.
    async fn table_structure(&self, namespace: Option<&str>, name: &str) -> Result<TableStructure> {
        let description = self.describe_object(namespace, name).await?;
        Ok(TableStructure {
            name: description.name,
            columns: description.columns,
            indexes: self.list_indexes(namespace, name).await?,
            foreign_keys: self.list_foreign_keys(namespace, name).await?,
            referenced_by: self.list_referencing(namespace, name).await?,
            triggers: self.list_triggers(namespace, name).await?,
            ddl: self.object_ddl(namespace, name).await?,
        })
    }

    /// Whether the session can still be used. Asked after a failure, to tell a
    /// broken connection from a statement the database rejected.
    async fn is_alive(&self) -> bool {
        self.ping().await.is_ok()
    }

    /// The SQL flavour, for writing values into statements.
    fn dialect(&self) -> SqlDialect;

    /// Whether the session has a transaction open that the user began.
    async fn in_transaction(&self) -> Result<bool>;

    /// The statements [`apply_changes`](Self::apply_changes) would run, for review.
    async fn preview_changes(&self, changes: &ChangeSet) -> Result<Vec<String>> {
        let namespace = changes.namespace.as_deref();
        let description = self.describe_object(namespace, &changes.table).await?;
        edit::statements(
            self.dialect(),
            &self.qualified(namespace, &changes.table),
            &description,
            &changes.changes,
            &|name| self.quote_ident(name),
        )
    }

    /// Applies row changes as one unit: all of them, or none.
    async fn apply_changes(&self, changes: &ChangeSet) -> Result<ApplyResult>;

    /// Quotes a name so it can be used as an identifier in a statement.
    fn quote_ident(&self, name: &str) -> String;

    /// Reads one page of a table. The default builds a `SELECT ... LIMIT ...
    /// OFFSET` statement; drivers whose dialect differs override it.
    async fn fetch_rows(&self, request: TableRequest) -> Result<TablePage> {
        let started = Instant::now();
        let description = self
            .describe_object(request.namespace.as_deref(), &request.table)
            .await?;

        // Only real column names reach the statement.
        let mut order = Vec::new();
        for sort in &request.order_by {
            if !description.columns.iter().any(|c| c.name == sort.column) {
                return Err(Error::Query(format!("no such column: {}", sort.column)));
            }
            let direction = if sort.descending { "DESC" } else { "ASC" };
            order.push(format!("{} {direction}", self.quote_ident(&sort.column)));
        }
        // Without a total order the database may return rows in a different
        // order for each page, repeating some and skipping others.
        for key in description.columns.iter().filter(|c| c.primary_key) {
            if !request.order_by.iter().any(|s| s.column == key.name) {
                order.push(self.quote_ident(&key.name));
            }
        }

        let mut statement = format!(
            "SELECT * FROM {}{}",
            self.qualified(request.namespace.as_deref(), &request.table),
            where_clause(request.filter.as_deref())?
        );
        // One row more than asked for tells whether another page exists.
        let limit = request.limit as usize;
        if self.dialect() == SqlDialect::SqlServer {
            // T-SQL pages with OFFSET ... FETCH, which needs an ORDER BY.
            let order = if order.is_empty() {
                "(SELECT NULL)".to_owned()
            } else {
                order.join(", ")
            };
            statement.push_str(&format!(
                " ORDER BY {order} OFFSET {} ROWS FETCH NEXT {} ROWS ONLY",
                request.offset,
                limit + 1
            ));
        } else {
            if !order.is_empty() {
                statement.push_str(&format!(" ORDER BY {}", order.join(", ")));
            }
            statement.push_str(&format!(" LIMIT {} OFFSET {}", limit + 1, request.offset));
        }

        let mut result = self
            .execute(QueryRequest {
                statement,
                max_rows: None,
            })
            .await?;
        let has_more = result.rows.len() > limit;
        result.rows.truncate(limit);
        Ok(TablePage {
            columns: description.columns,
            rows: result.rows,
            offset: request.offset,
            has_more,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }

    /// Counts the rows of a table exactly. Can be slow on large tables.
    async fn count_rows(
        &self,
        namespace: Option<&str>,
        table: &str,
        filter: Option<&str>,
    ) -> Result<u64> {
        let statement = format!(
            "SELECT count(*) FROM {}{}",
            self.qualified(namespace, table),
            where_clause(filter)?
        );
        let result = self
            .execute(QueryRequest {
                statement,
                max_rows: None,
            })
            .await?;
        match result.rows.first().and_then(|row| row.first()) {
            Some(Value::Int(count)) => Ok(*count as u64),
            other => Err(Error::Query(format!("unexpected count result: {other:?}"))),
        }
    }

    /// `namespace.table`, quoted.
    fn qualified(&self, namespace: Option<&str>, table: &str) -> String {
        match namespace.filter(|n| !n.is_empty()) {
            Some(namespace) => format!(
                "{}.{}",
                self.quote_ident(namespace),
                self.quote_ident(table)
            ),
            None => self.quote_ident(table),
        }
    }

    /// The key-value operations, for drivers of key-value stores.
    fn key_value(&self) -> Option<&dyn crate::kv::KeyValue> {
        None
    }

    /// The document operations, for drivers of document databases.
    fn document(&self) -> Option<&dyn crate::document::DocumentStore> {
        None
    }

    /// The vector operations, for drivers of vector databases.
    fn vector(&self) -> Option<&dyn crate::vector::VectorStore> {
        None
    }

    /// The graph operations, for drivers of graph databases.
    fn graph(&self) -> Option<&dyn crate::graph::GraphStore> {
        None
    }

    /// Interrupt the statement that is currently running, if any.
    async fn cancel(&self) -> Result<()> {
        Err(Error::Unsupported("cancel"))
    }

    async fn close(&self) -> Result<()>;
}

/// ` WHERE (...)` for a user-written condition, or nothing.
fn where_clause(filter: Option<&str>) -> Result<String> {
    let Some(filter) = filter.map(str::trim).filter(|f| !f.is_empty()) else {
        return Ok(String::new());
    };
    // A filter is one condition. A semicolon would let it carry further
    // statements, which a filter box should never run.
    if filter.contains(';') {
        return Err(Error::Query("a filter cannot contain ';'".into()));
    }
    // The line break keeps a trailing `--` comment from swallowing the closing bracket.
    Ok(format!(" WHERE ({filter}\n)"))
}
