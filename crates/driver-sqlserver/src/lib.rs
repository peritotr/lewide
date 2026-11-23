//! Microsoft SQL Server, through the TDS protocol (tiberius).
//!
//! Text from the editor is sent as SQL batches, split on `GO` lines as SQL
//! Server's own tools do. The result stream does not carry row counts, so a
//! batch that may change rows is followed by `SELECT @@ROWCOUNT`, read back
//! under a reserved column name and not shown.
//!
//! Stop: tiberius cannot send TDS's attention signal, so a running request is
//! stopped with `KILL <session id>` from a second, short connection. That ends
//! the server session (an open transaction is rolled back); the driver then
//! opens a new one, so the connection stays usable.

mod convert;

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dbclient_core::{
    Capabilities, Column, ColumnDescription, Connection, ConnectionConfig, DataModel, DbObject,
    Driver, DriverInfo, Error, ForeignKey, IndexDescription, Namespace, ObjectDescription,
    ObjectKind, QueryRequest, QueryResult, Reference, Result, SqlDialect, TlsMode,
    TriggerDescription, Value,
};
use futures_util::TryStreamExt;
use tiberius::{AuthMethod, Client, EncryptionLevel, QueryItem, ToSql};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

use convert::{batches, column_type, stands_alone, to_value, type_name, unwrap_default};

const ROWCOUNT: &str = "__lewide_rowcount";
/// Separates names inside one aggregated catalog value; no identifier contains it.
const SEP: char = '\u{1f}';
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

pub struct SqlServerDriver;

#[async_trait]
impl Driver for SqlServerDriver {
    fn info(&self) -> DriverInfo {
        DriverInfo {
            id: "sqlserver",
            name: "SQL Server",
            model: DataModel::Relational,
            capabilities: Capabilities {
                sql: true,
                schemas: true,
                transactions: true,
                cancel: true,
                tls: true,
                ..Capabilities::default()
            },
            default_port: Some(1433),
            desktop_only: false,
        }
    }

    async fn connect(&self, config: &ConnectionConfig) -> Result<Box<dyn Connection>> {
        let mut client = open(config).await?;
        let spid = session_id(&mut client).await?;
        Ok(Box::new(SqlServerConnection {
            client: Mutex::new(Some(client)),
            config: config.clone(),
            spid: AtomicI32::new(spid),
            running: AtomicBool::new(false),
            killed: AtomicBool::new(false),
        }))
    }
}

/// A logged-in session for `config`.
async fn open(config: &ConnectionConfig) -> Result<Session> {
    {
        let host = config
            .host
            .clone()
            .filter(|h| !h.trim().is_empty())
            .unwrap_or_else(|| "localhost".into());
        let port = config.port.unwrap_or(1433);
        let user = config
            .user
            .clone()
            .filter(|u| !u.is_empty())
            .ok_or_else(|| Error::InvalidConfig("SQL Server needs a user name".into()))?;

        let mut settings = tiberius::Config::new();
        settings.host(&host);
        settings.port(port);
        if let Some(database) = config.database.as_deref().filter(|d| !d.is_empty()) {
            settings.database(database);
        }
        settings.application_name("LewIDE");
        settings.authentication(AuthMethod::sql_server(
            user,
            config.password.clone().unwrap_or_default(),
        ));
        match config.tls.mode {
            // SQL Server always encrypts the login; with TLS off only the
            // login is encrypted, and its self-signed certificate is accepted.
            TlsMode::Off => {
                settings.encryption(EncryptionLevel::Off);
                settings.trust_cert();
            }
            TlsMode::Insecure => {
                settings.encryption(EncryptionLevel::Required);
                settings.trust_cert();
            }
            TlsMode::Verify => {
                settings.encryption(EncryptionLevel::Required);
                if let Some(ca) = config.tls.ca_file.as_deref().filter(|c| !c.is_empty()) {
                    if !std::path::Path::new(ca).is_file() {
                        return Err(Error::InvalidConfig(format!(
                            "the certificate authority file does not exist: {ca}"
                        )));
                    }
                    settings.trust_cert_ca(ca);
                }
            }
        }

        // Through a tunnel the socket goes to the tunnel's end; the name above
        // is still the one the login and the certificate check use.
        let address = config
            .route
            .map_or_else(|| format!("{host}:{port}"), |route| route.to_string());
        let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&address))
            .await
            .map_err(|_| Error::Connect(format!("no answer from {address}")))?
            .map_err(|e| Error::Connect(format!("{address}: {e}")))?;
        tcp.set_nodelay(true)
            .map_err(|e| Error::Connect(e.to_string()))?;
        let client = tokio::time::timeout(
            CONNECT_TIMEOUT,
            Client::connect(settings, tcp.compat_write()),
        )
        .await
        .map_err(|_| Error::Connect("the server did not finish the login".into()))?
        .map_err(connect_err)?;
        Ok(client)
    }
}

/// The server's id of this session, which `KILL` takes.
async fn session_id(client: &mut Session) -> Result<i32> {
    let row = client
        .simple_query("SELECT CAST(@@SPID AS int)")
        .await
        .map_err(|e| query_err(e, 0))?
        .into_row()
        .await
        .map_err(|e| query_err(e, 0))?;
    row.and_then(|r| r.try_get::<i32, _>(0).ok().flatten())
        .ok_or_else(|| Error::Connect("the server did not say its session id".into()))
}

type Session = Client<Compat<TcpStream>>;

struct SqlServerConnection {
    /// `None` once closed. Requests take turns: a TDS connection carries one at a time.
    client: Mutex<Option<Session>>,
    /// For the second connection that stops a request, and for opening a new session after it.
    config: ConnectionConfig,
    spid: AtomicI32,
    /// Statements from the editor are running.
    running: AtomicBool,
    /// Stop ended the session; the next failure is that, not the server's error.
    killed: AtomicBool,
}

impl SqlServerConnection {
    /// A catalog query with parameters (`@P1`, …), its first result as values.
    async fn catalog(&self, sql: &str, params: &[&dyn ToSql]) -> Result<Vec<Vec<Value>>> {
        let mut guard = self.client.lock().await;
        let client = guard.as_mut().ok_or_else(closed)?;
        let rows = client
            .query(sql, params)
            .await
            .map_err(|e| query_err(e, 0))?
            .into_first_result()
            .await
            .map_err(|e| query_err(e, 0))?;
        Ok(rows
            .iter()
            .map(|row| {
                row.cells()
                    .map(|(column, data)| to_value(data, column.column_type()))
                    .collect()
            })
            .collect())
    }

    /// `[schema].[name]` for OBJECT_ID, in the connection's default schema when none is given.
    fn object(&self, namespace: Option<&str>, name: &str) -> String {
        self.qualified(namespace, name)
    }

    async fn schema_or_default(&self, namespace: Option<&str>) -> Result<String> {
        match namespace.filter(|n| !n.is_empty()) {
            Some(n) => Ok(n.to_owned()),
            None => self
                .current_namespace()
                .await?
                .ok_or_else(|| Error::Query("no default schema".into())),
        }
    }

    async fn run_batch(
        client: &mut Session,
        text: &str,
        line_offset: u32,
        max_rows: Option<usize>,
    ) -> Result<Vec<QueryResult>> {
        let started = Instant::now();
        let counted = !stands_alone(text);
        let statement = if counted {
            let body = text.trim_end();
            let join = if body.ends_with(';') { "\n" } else { "\n;" };
            format!("{body}{join}SELECT @@ROWCOUNT AS [{ROWCOUNT}]")
        } else {
            text.to_owned()
        };

        let mut stream = client
            .simple_query(statement)
            .await
            .map_err(|e| query_err(e, line_offset))?;

        let mut sets: Vec<QueryResult> = Vec::new();
        let mut counting = false;
        let mut affected = None;
        while let Some(item) = stream
            .try_next()
            .await
            .map_err(|e| query_err(e, line_offset))?
        {
            match item {
                QueryItem::Metadata(meta) => {
                    let columns = meta.columns();
                    counting = counted && columns.len() == 1 && columns[0].name() == ROWCOUNT;
                    if !counting {
                        sets.push(QueryResult {
                            columns: columns
                                .iter()
                                .map(|c| Column {
                                    name: c.name().to_owned(),
                                    type_name: Some(type_name(c.column_type()).to_owned()),
                                })
                                .collect(),
                            rows: Vec::new(),
                            rows_affected: None,
                            truncated: false,
                            elapsed_ms: 0,
                        });
                    }
                }
                QueryItem::Row(row) if counting => {
                    affected = row.try_get::<i32, _>(0).ok().flatten().map(|n| n as u64);
                }
                QueryItem::Row(row) => {
                    let Some(set) = sets.last_mut() else { continue };
                    // Past the limit the rest is read and dropped, so the
                    // connection is free for the next request.
                    if max_rows.is_some_and(|max| set.rows.len() >= max) {
                        set.truncated = true;
                        continue;
                    }
                    set.rows.push(
                        row.cells()
                            .map(|(column, data)| to_value(data, column.column_type()))
                            .collect(),
                    );
                }
            }
        }
        let elapsed_ms = started.elapsed().as_millis() as u64;
        if sets.is_empty() {
            sets.push(QueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
                rows_affected: affected,
                truncated: false,
                elapsed_ms,
            });
        }
        for set in &mut sets {
            set.elapsed_ms = elapsed_ms;
        }
        Ok(sets)
    }

    async fn foreign_keys(&self, object: &str, incoming: bool) -> Result<Vec<Vec<Value>>> {
        // Outgoing: keys of this table. Incoming: keys elsewhere that point at it.
        let (side, other) = if incoming {
            ("referenced_object_id", "parent_object_id")
        } else {
            ("parent_object_id", "referenced_object_id")
        };
        let sql = format!(
            "SELECT fk.name, OBJECT_SCHEMA_NAME(fk.{other}), OBJECT_NAME(fk.{other}), \
               STRING_AGG(CAST(pc.name AS nvarchar(max)), NCHAR(31)) WITHIN GROUP (ORDER BY fkc.constraint_column_id), \
               STRING_AGG(CAST(rc.name AS nvarchar(max)), NCHAR(31)) WITHIN GROUP (ORDER BY fkc.constraint_column_id), \
               fk.update_referential_action_desc, fk.delete_referential_action_desc \
             FROM sys.foreign_keys fk \
             JOIN sys.foreign_key_columns fkc ON fkc.constraint_object_id = fk.object_id \
             JOIN sys.columns pc ON pc.object_id = fkc.parent_object_id AND pc.column_id = fkc.parent_column_id \
             JOIN sys.columns rc ON rc.object_id = fkc.referenced_object_id AND rc.column_id = fkc.referenced_column_id \
             WHERE fk.{side} = OBJECT_ID(@P1) \
             GROUP BY fk.name, fk.{other}, fk.update_referential_action_desc, fk.delete_referential_action_desc \
             ORDER BY fk.name"
        );
        self.catalog(&sql, &[&object]).await
    }
}

#[async_trait]
impl Connection for SqlServerConnection {
    async fn ping(&self) -> Result<()> {
        self.catalog("SELECT 1", &[]).await.map(|_| ())
    }

    async fn execute_all(&self, request: QueryRequest) -> Result<Vec<QueryResult>> {
        let pieces = batches(&request.statement);
        if pieces.is_empty() {
            return Err(Error::Query("no statement to run".into()));
        }
        // One lock for all batches: nothing else runs between them, which a
        // `SET SHOWPLAN_TEXT ON` … `OFF` sequence depends on.
        let mut guard = self.client.lock().await;
        let client = guard.as_mut().ok_or_else(closed)?;
        self.running.store(true, Ordering::SeqCst);
        let mut out = Vec::new();
        let mut failure = None;
        for batch in pieces {
            match Self::run_batch(client, batch.text, batch.line_offset, request.max_rows).await {
                Ok(sets) => out.extend(sets),
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        self.running.store(false, Ordering::SeqCst);
        if self.killed.swap(false, Ordering::SeqCst) {
            // Stop ended the session: open another in its place.
            let mut fresh = open(&self.config).await?;
            self.spid
                .store(session_id(&mut fresh).await?, Ordering::SeqCst);
            *guard = Some(fresh);
            return Err(Error::Query(
                "stopped: the server session was ended and a new one opened; an open transaction was rolled back".into(),
            ));
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(out),
        }
    }

    async fn server_version(&self) -> Result<Option<String>> {
        let rows = self
            .catalog(
                "SELECT CAST(SERVERPROPERTY('ProductVersion') AS nvarchar(128)), \
                        CAST(SERVERPROPERTY('EngineEdition') AS int)",
                &[],
            )
            .await?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let version = text(&row[0]).unwrap_or_default();
        let edition = int(&row[1]).unwrap_or(0);
        Ok(Some(product_name(&version, edition)))
    }

    async fn current_namespace(&self) -> Result<Option<String>> {
        let rows = self.catalog("SELECT SCHEMA_NAME()", &[]).await?;
        Ok(rows.first().and_then(|r| text(&r[0])))
    }

    async fn list_namespaces(&self) -> Result<Vec<Namespace>> {
        // Ids from 16384 up are the fixed database roles, which own no objects.
        let rows = self
            .catalog(
                "SELECT name FROM sys.schemas \
                 WHERE name NOT IN ('sys', 'INFORMATION_SCHEMA', 'guest') AND schema_id < 16384 \
                 ORDER BY name",
                &[],
            )
            .await?;
        Ok(rows
            .iter()
            .filter_map(|r| text(&r[0]))
            .map(|name| Namespace { name })
            .collect())
    }

    async fn list_objects(&self, namespace: Option<&str>) -> Result<Vec<DbObject>> {
        let schema = self.schema_or_default(namespace).await?;
        let rows = self
            .catalog(
                "SELECT o.name, RTRIM(o.type), \
                   CAST((SELECT SUM(p.rows) FROM sys.partitions p \
                         WHERE p.object_id = o.object_id AND p.index_id IN (0, 1)) AS bigint) \
                 FROM sys.objects o JOIN sys.schemas s ON s.schema_id = o.schema_id \
                 WHERE s.name = @P1 AND o.type IN ('U', 'V') AND o.is_ms_shipped = 0 \
                 ORDER BY o.name",
                &[&schema],
            )
            .await?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                Some(DbObject {
                    name: text(&r[0])?,
                    kind: if text(&r[1]).as_deref() == Some("V") {
                        ObjectKind::View
                    } else {
                        ObjectKind::Table
                    },
                    row_estimate: int(&r[2]).map(|n| n.max(0) as u64),
                })
            })
            .collect())
    }

    async fn describe_object(
        &self,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<ObjectDescription> {
        let object = self.object(namespace, name);
        let rows = self
            .catalog(
                "SELECT c.name, t.name, CAST(c.max_length AS int), CAST(c.precision AS int), \
                   CAST(c.scale AS int), c.is_nullable, \
                   COALESCE( \
                     CASE WHEN c.is_identity = 1 THEN 'IDENTITY(' + CAST(ic.seed_value AS nvarchar(40)) \
                       + ',' + CAST(ic.increment_value AS nvarchar(40)) + ')' END, \
                     'AS ' + cc.definition, \
                     OBJECT_DEFINITION(c.default_object_id)), \
                   CAST(CASE WHEN EXISTS ( \
                     SELECT 1 FROM sys.indexes i \
                     JOIN sys.index_columns x ON x.object_id = i.object_id AND x.index_id = i.index_id \
                     WHERE i.object_id = c.object_id AND i.is_primary_key = 1 AND x.column_id = c.column_id \
                   ) THEN 1 ELSE 0 END AS bit), \
                   CAST(ep.value AS nvarchar(4000)), \
                   CAST(CASE WHEN c.is_identity = 1 OR c.is_computed = 1 THEN 1 ELSE 0 END AS bit) \
                 FROM sys.columns c \
                 JOIN sys.types t ON t.user_type_id = c.user_type_id \
                 LEFT JOIN sys.identity_columns ic ON ic.object_id = c.object_id AND ic.column_id = c.column_id \
                 LEFT JOIN sys.computed_columns cc ON cc.object_id = c.object_id AND cc.column_id = c.column_id \
                 LEFT JOIN sys.extended_properties ep ON ep.class = 1 AND ep.major_id = c.object_id \
                   AND ep.minor_id = c.column_id AND ep.name = 'MS_Description' \
                 WHERE c.object_id = OBJECT_ID(@P1) \
                 ORDER BY c.column_id",
                &[&object],
            )
            .await?;
        if rows.is_empty() {
            return Err(Error::Query(format!("no such table or view: {name}")));
        }
        let columns = rows
            .iter()
            .map(|r| {
                let base = text(&r[1]).unwrap_or_default();
                let generated = flag(&r[9]);
                ColumnDescription {
                    name: text(&r[0]).unwrap_or_default(),
                    type_name: Some(column_type(
                        &base,
                        int(&r[2]).unwrap_or(0) as i16,
                        int(&r[3]).unwrap_or(0) as u8,
                        int(&r[4]).unwrap_or(0) as u8,
                    )),
                    nullable: flag(&r[5]),
                    // Identity and computed columns carry their rule here: the
                    // server fills them, so an insert or import may leave them out.
                    default_value: text(&r[6])
                        .map(|d| if generated { d } else { unwrap_default(&d) }),
                    primary_key: flag(&r[7]),
                    comment: text(&r[8]),
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
        let object = self.object(namespace, table);
        let rows = self
            .catalog(
                "SELECT i.name, i.is_unique, i.is_primary_key, LOWER(i.type_desc), i.filter_definition, \
                   STRING_AGG(CAST(c.name AS nvarchar(max)), NCHAR(31)) WITHIN GROUP (ORDER BY x.key_ordinal), \
                   CAST((SELECT SUM(au.used_pages) FROM sys.partitions p \
                         JOIN sys.allocation_units au ON au.container_id IN (p.hobt_id, p.partition_id) \
                         WHERE p.object_id = i.object_id AND p.index_id = i.index_id) AS bigint) * 8192 \
                 FROM sys.indexes i \
                 JOIN sys.index_columns x ON x.object_id = i.object_id AND x.index_id = i.index_id \
                   AND x.is_included_column = 0 \
                 JOIN sys.columns c ON c.object_id = x.object_id AND c.column_id = x.column_id \
                 WHERE i.object_id = OBJECT_ID(@P1) AND i.name IS NOT NULL \
                 GROUP BY i.object_id, i.index_id, i.name, i.is_unique, i.is_primary_key, i.type_desc, i.filter_definition \
                 ORDER BY i.is_primary_key DESC, i.name",
                &[&object],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| IndexDescription {
                name: text(&r[0]).unwrap_or_default(),
                columns: names(&r[5]),
                unique: flag(&r[1]),
                primary: flag(&r[2]),
                method: text(&r[3]),
                size: int(&r[6]).map(|bytes| size_text(bytes.max(0) as u64)),
                condition: text(&r[4]).map(|c| unwrap_default(&c)),
            })
            .collect())
    }

    async fn list_foreign_keys(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<ForeignKey>> {
        let rows = self
            .foreign_keys(&self.object(namespace, table), false)
            .await?;
        Ok(rows
            .iter()
            .map(|r| ForeignKey {
                name: text(&r[0]),
                columns: names(&r[3]),
                referenced_namespace: text(&r[1]),
                referenced_table: text(&r[2]).unwrap_or_default(),
                referenced_columns: names(&r[4]),
                on_update: text(&r[5]).map(|a| a.replace('_', " ")),
                on_delete: text(&r[6]).map(|a| a.replace('_', " ")),
            })
            .collect())
    }

    async fn list_referencing(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<Reference>> {
        let rows = self
            .foreign_keys(&self.object(namespace, table), true)
            .await?;
        Ok(rows
            .iter()
            .map(|r| Reference {
                name: text(&r[0]),
                namespace: text(&r[1]),
                table: text(&r[2]).unwrap_or_default(),
                columns: names(&r[3]),
                referenced_columns: names(&r[4]),
                on_update: text(&r[5]).map(|a| a.replace('_', " ")),
                on_delete: text(&r[6]).map(|a| a.replace('_', " ")),
            })
            .collect())
    }

    async fn list_triggers(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<TriggerDescription>> {
        let object = self.object(namespace, table);
        let rows = self
            .catalog(
                "SELECT t.name, t.is_instead_of_trigger, \
                   STRING_AGG(CAST(e.type_desc AS nvarchar(max)), NCHAR(31)), \
                   OBJECT_DEFINITION(t.object_id) \
                 FROM sys.triggers t JOIN sys.trigger_events e ON e.object_id = t.object_id \
                 WHERE t.parent_id = OBJECT_ID(@P1) \
                 GROUP BY t.object_id, t.name, t.is_instead_of_trigger \
                 ORDER BY t.name",
                &[&object],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| TriggerDescription {
                name: text(&r[0]).unwrap_or_default(),
                timing: Some(if flag(&r[1]) { "INSTEAD OF" } else { "AFTER" }.to_owned()),
                events: names(&r[2]),
                definition: text(&r[3]),
            })
            .collect())
    }

    async fn object_ddl(&self, namespace: Option<&str>, name: &str) -> Result<Option<String>> {
        let schema = self.schema_or_default(namespace).await?;
        let object = self.object(Some(&schema), name);
        let found = self
            .catalog(
                "SELECT RTRIM(type), OBJECT_DEFINITION(object_id) FROM sys.objects \
                 WHERE object_id = OBJECT_ID(@P1)",
                &[&object],
            )
            .await?;
        let Some(row) = found.first() else {
            return Ok(None);
        };
        if text(&row[0]).as_deref() == Some("V") {
            // A view is kept as the text it was created with.
            return Ok(text(&row[1]).map(|d| format!("{}\nGO", d.trim())));
        }
        // SQL Server keeps no text for tables; this is written from the catalog.
        let description = self.describe_object(Some(&schema), name).await?;
        let indexes = self.list_indexes(Some(&schema), name).await?;
        let mut lines: Vec<String> = description
            .columns
            .iter()
            .map(|c| {
                let mut line = format!(
                    "    {} {}",
                    self.quote_ident(&c.name),
                    c.type_name.as_deref().unwrap_or("sql_variant")
                );
                match c.default_value.as_deref() {
                    Some(d) if d.starts_with("AS ") => {
                        line = format!("    {} {d}", self.quote_ident(&c.name));
                        return line;
                    }
                    Some(d) if d.starts_with("IDENTITY(") => line.push_str(&format!(" {d}")),
                    _ => {}
                }
                if !c.nullable {
                    line.push_str(" NOT NULL");
                }
                if let Some(d) = c
                    .default_value
                    .as_deref()
                    .filter(|d| !d.starts_with("IDENTITY("))
                {
                    line.push_str(&format!(" DEFAULT {d}"));
                }
                line
            })
            .collect();
        if let Some(pk) = indexes.iter().find(|i| i.primary) {
            let columns: Vec<String> = pk.columns.iter().map(|c| self.quote_ident(c)).collect();
            lines.push(format!(
                "    CONSTRAINT {} PRIMARY KEY ({})",
                self.quote_ident(&pk.name),
                columns.join(", ")
            ));
        }
        let mut ddl = format!("CREATE TABLE {} (\n{}\n);", object, lines.join(",\n"));
        for index in indexes.iter().filter(|i| !i.primary) {
            let columns: Vec<String> = index.columns.iter().map(|c| self.quote_ident(c)).collect();
            ddl.push_str(&format!(
                "\nCREATE {}{}INDEX {} ON {object} ({}){};",
                if index.unique { "UNIQUE " } else { "" },
                match index.method.as_deref() {
                    Some("clustered") => "CLUSTERED ",
                    _ => "",
                },
                self.quote_ident(&index.name),
                columns.join(", "),
                index
                    .condition
                    .as_deref()
                    .map(|c| format!(" WHERE {c}"))
                    .unwrap_or_default()
            ));
        }
        Ok(Some(ddl))
    }

    fn quote_ident(&self, name: &str) -> String {
        SqlDialect::SqlServer.quote_ident(name)
    }

    async fn is_alive(&self) -> bool {
        self.ping().await.is_ok()
    }

    fn dialect(&self) -> SqlDialect {
        SqlDialect::SqlServer
    }

    async fn in_transaction(&self) -> Result<bool> {
        let rows = self.catalog("SELECT CAST(@@TRANCOUNT AS int)", &[]).await?;
        Ok(rows.first().and_then(|r| int(&r[0])).unwrap_or(0) > 0)
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

    /// Ends the session of the running request from a second connection; see
    /// the module notes. Nothing happens when no statement is running.
    async fn cancel(&self) -> Result<()> {
        if !self.running.load(Ordering::SeqCst) {
            return Ok(());
        }
        let spid = self.spid.load(Ordering::SeqCst);
        let mut other = open(&self.config).await?;
        self.killed.store(true, Ordering::SeqCst);
        let killed = match other.simple_query(format!("KILL {spid}")).await {
            // The reply has no rows; it is read to the end before the close.
            Ok(stream) => stream.into_results().await.map(|_| ()),
            Err(error) => Err(error),
        };
        let _ = other.close().await;
        if let Err(error) = killed {
            self.killed.store(false, Ordering::SeqCst);
            return Err(query_err(error, 0));
        }
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        if let Some(client) = self.client.lock().await.take() {
            client.close().await.map_err(|e| query_err(e, 0))?;
        }
        Ok(())
    }
}

fn closed() -> Error {
    Error::ConnectionLost("the connection is closed".into())
}

fn query_err(e: tiberius::error::Error, line_offset: u32) -> Error {
    match e {
        tiberius::error::Error::Server(token) => Error::Database {
            code: Some(token.code().to_string()),
            message: token.message().to_owned(),
            position: None,
            // Line 0 means the server did not name one.
            line: (token.line() > 0).then(|| token.line() + line_offset),
            hint: None,
            detail: None,
        },
        tiberius::error::Error::Io { message, .. } => Error::ConnectionLost(message),
        other => Error::Query(other.to_string()),
    }
}

fn connect_err(e: tiberius::error::Error) -> Error {
    match e {
        // 18456 is a failed login: wrong name or password, or a disabled login.
        tiberius::error::Error::Server(token) => Error::Connect(token.message().to_owned()),
        other => Error::Connect(other.to_string()),
    }
}

fn text(value: &Value) -> Option<String> {
    match value {
        Value::Text(s) | Value::Decimal(s) | Value::Temporal(s) => Some(s.clone()),
        Value::Int(i) => Some(i.to_string()),
        _ => None,
    }
}

fn int(value: &Value) -> Option<i64> {
    match value {
        Value::Int(i) => Some(*i),
        Value::Decimal(s) => s.split('.').next().and_then(|w| w.parse().ok()),
        _ => None,
    }
}

fn flag(value: &Value) -> bool {
    matches!(value, Value::Bool(true) | Value::Int(1))
}

fn names(value: &Value) -> Vec<String> {
    text(value)
        .map(|s| s.split(SEP).map(str::to_owned).collect())
        .unwrap_or_default()
}

fn size_text(bytes: u64) -> String {
    const KB: u64 = 1024;
    match bytes {
        b if b >= KB * KB * KB => format!("{:.1} GB", b as f64 / (KB * KB * KB) as f64),
        b if b >= KB * KB => format!("{:.1} MB", b as f64 / (KB * KB) as f64),
        b => format!("{} kB", b / KB),
    }
}

/// `16.0.4135.4 (SQL Server 2022)`: the number first, as the other drivers give
/// it (the status bar shows the first word), then the product.
fn product_name(version: &str, edition: i64) -> String {
    match edition {
        5 => return format!("{version} (Azure SQL Database)"),
        8 => return format!("{version} (Azure SQL Managed Instance)"),
        _ => {}
    }
    let year = match version
        .split('.')
        .next()
        .and_then(|m| m.parse::<u32>().ok())
    {
        Some(17) => " 2025",
        Some(16) => " 2022",
        Some(15) => " 2019",
        Some(14) => " 2017",
        Some(13) => " 2016",
        Some(12) => " 2014",
        _ => "",
    };
    format!("{version} (SQL Server{year})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_product() {
        assert_eq!(
            product_name("16.0.4135.4", 3),
            "16.0.4135.4 (SQL Server 2022)"
        );
        assert_eq!(
            product_name("12.0.2000.8", 5),
            "12.0.2000.8 (Azure SQL Database)"
        );
        assert_eq!(product_name("99.1", 2), "99.1 (SQL Server)");
        assert_eq!(size_text(16384), "16 kB");
        assert_eq!(size_text(3 * 1024 * 1024), "3.0 MB");
    }
}
