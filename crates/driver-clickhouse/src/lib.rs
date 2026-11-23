//! ClickHouse, through its HTTP interface.
//!
//! HTTP keeps no session: every request names the database, and `USE` is
//! remembered here instead of on the server. The editor's text is cut into
//! statements (the interface takes one per request). Results come as
//! JSONCompact with 64-bit integers and decimals as strings, so no digit is
//! lost on the way.
//!
//! Rows are not edited from the grid: a ClickHouse key does not identify one
//! row, and UPDATE and DELETE run later as background mutations. Columns
//! therefore never claim to be a primary key, and `apply_changes` takes new
//! rows only (for imports).

mod convert;
mod split;

use std::error::Error as _;
use std::net::IpAddr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use dbclient_core::sql::literal;
use dbclient_core::{
    ApplyResult, Capabilities, ChangeSet, Column, ColumnDescription, Connection, ConnectionConfig,
    DataModel, DbObject, Driver, DriverInfo, Error, IndexDescription, Namespace, ObjectDescription,
    ObjectKind, QueryRequest, QueryResult, Result, RowChange, SqlDialect, TablePage, TableRequest,
    TlsMode, Value,
};
use reqwest::header::{HOST, HeaderMap, HeaderValue};
use reqwest::{RequestBuilder, Url};
use serde_json::Value as Json;

use convert::{key_parts, server_error, size_text, to_value};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// For the catalog. Statements from the editor have no limit: they can be cancelled.
const CATALOG_TIMEOUT: Duration = Duration::from_secs(60);

pub struct ClickHouseDriver;

#[async_trait]
impl Driver for ClickHouseDriver {
    fn info(&self) -> DriverInfo {
        DriverInfo {
            id: "clickhouse",
            name: "ClickHouse",
            model: DataModel::Relational,
            capabilities: Capabilities {
                sql: true,
                cancel: true,
                tls: true,
                ..Capabilities::default()
            },
            default_port: Some(8123),
            desktop_only: false,
        }
    }

    async fn connect(&self, config: &ConnectionConfig) -> Result<Box<dyn Connection>> {
        let connection = ClickHouseConnection::new(config)?;
        // Checks the address, the credentials and the database in one go.
        connection
            .catalog("SELECT version()", &[])
            .await
            .map_err(connect_error)?;
        Ok(Box::new(connection))
    }
}

struct ClickHouseConnection {
    http: reqwest::Client,
    /// Scheme and address, without a path.
    base: Url,
    /// The database unqualified names resolve to; `USE` changes it.
    database: Mutex<String>,
    /// The id of the statement running now, for `cancel`.
    running: Mutex<Option<String>>,
    closed: AtomicBool,
}

static NEXT_QUERY: AtomicU64 = AtomicU64::new(1);

/// A query id that no other client is likely to use.
fn query_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(
        "lewide-{}-{nanos}-{}",
        std::process::id(),
        NEXT_QUERY.fetch_add(1, Ordering::Relaxed)
    )
}

/// What a reply carries: its body and how many rows it wrote.
struct Reply {
    body: String,
    written: Option<u64>,
}

impl ClickHouseConnection {
    fn new(config: &ConnectionConfig) -> Result<Self> {
        let host = config
            .host
            .as_deref()
            .map(|h| h.trim().trim_start_matches('[').trim_end_matches(']'))
            .filter(|h| !h.is_empty())
            .unwrap_or("localhost");
        let encrypted = config.tls.mode != TlsMode::Off;
        let port = config.port.unwrap_or(if encrypted { 8443 } else { 8123 });
        let scheme = if encrypted { "https" } else { "http" };

        let mut headers = HeaderMap::new();
        let user = config
            .user
            .as_deref()
            .filter(|u| !u.is_empty())
            .unwrap_or("default");
        headers.insert(
            "X-ClickHouse-User",
            HeaderValue::from_str(user).map_err(|_| {
                Error::InvalidConfig(
                    "the user name contains characters a header cannot carry".into(),
                )
            })?,
        );
        if let Some(password) = config.password.as_deref().filter(|p| !p.is_empty()) {
            let mut key = HeaderValue::from_str(password).map_err(|_| {
                Error::InvalidConfig(
                    "the password contains characters a header cannot carry".into(),
                )
            })?;
            // Keeps the password out of debug output of requests.
            key.set_sensitive(true);
            headers.insert("X-ClickHouse-Key", key);
        }

        let mut builder = reqwest::Client::builder()
            // A database address is reached directly. A proxy from the
            // environment would also break the route of a tunnel.
            .no_proxy()
            // A redirect would carry the password to wherever it points.
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            // Given even when the connection is not encrypted: left to itself,
            // reqwest builds a TLS setup from a process-wide rustls provider
            // that nothing guarantees has been chosen.
            .tls_backend_preconfigured(dbclient_tls::client_config(&config.tls, Some(host))?);

        let base = match config.route {
            None => format!("{scheme}://{}", authority(host, Some(port))),
            Some(route) => {
                // The socket goes to the route; the server is still told its real name.
                let default_port = if encrypted { 443 } else { 80 };
                let name = authority(host, Some(port).filter(|p| *p != default_port));
                let name = HeaderValue::from_str(&name)
                    .map_err(|_| Error::InvalidConfig(format!("not a valid host name: {host}")))?;
                headers.insert(HOST, name);
                if host.parse::<IpAddr>().is_ok() {
                    // No name to resolve, so the route goes into the URL. The
                    // certificate is still checked against `host`, see `tls`.
                    format!("{scheme}://{route}")
                } else {
                    // Resolving the name to the route keeps the name in the TLS
                    // handshake. The URL has no port so that the route's is used.
                    builder = builder.resolve(host, route);
                    format!("{scheme}://{}", authority(host, None))
                }
            }
        };
        let base = Url::parse(&base)
            .map_err(|_| Error::InvalidConfig(format!("not a valid host name: {host}")))?;
        let http = builder
            .default_headers(headers)
            .build()
            .map_err(|e| Error::Internal(format!("HTTP client: {}", describe(&e))))?;
        let database = config
            .database
            .clone()
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| "default".into());
        Ok(Self {
            http,
            base,
            database: Mutex::new(database),
            running: Mutex::new(None),
            closed: AtomicBool::new(false),
        })
    }

    fn database(&self) -> String {
        self.database.lock().map(|d| d.clone()).unwrap_or_default()
    }

    /// One request with the statement in the body. `settings` are added to
    /// the URL: ClickHouse settings, and `param_x` values for `{x:Type}`.
    fn request(
        &self,
        statement: &str,
        settings: &[(&str, String)],
        id: &str,
    ) -> Result<RequestBuilder> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(Error::ConnectionLost("the connection is closed".into()));
        }
        let mut url = self.base.clone();
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("database", &self.database())
                .append_pair("query_id", id)
                .append_pair("default_format", "JSONCompact")
                .append_pair("output_format_json_quote_64bit_integers", "1")
                .append_pair("output_format_json_quote_decimals", "1");
            for (name, value) in settings {
                query.append_pair(name, value);
            }
        }
        Ok(self.http.post(url).body(statement.to_owned()))
    }

    async fn send(&self, request: RequestBuilder, whole: &str, offset: usize) -> Result<Reply> {
        let response = request.send().await.map_err(network_error)?;
        let status = response.status();
        let code = response
            .headers()
            .get("X-ClickHouse-Exception-Code")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let written = response
            .headers()
            .get("X-ClickHouse-Summary")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| serde_json::from_str::<Json>(s).ok())
            .and_then(|s| s["written_rows"].as_str().and_then(|n| n.parse().ok()));
        let body = response.text().await.map_err(network_error)?;
        // An error after the reply has begun arrives at the end of a 200 body.
        if !status.is_success() || code.is_some() {
            if status.as_u16() == 401 || status.as_u16() == 403 {
                return Err(Error::Connect(server_message(&body)));
            }
            return Err(server_error(code.as_deref(), &body, whole, offset));
        }
        Ok(Reply { body, written })
    }

    /// A catalog query, its rows as values.
    async fn catalog(&self, statement: &str, params: &[(&str, &str)]) -> Result<Vec<Vec<Value>>> {
        let settings: Vec<(String, String)> = params
            .iter()
            .map(|(name, value)| (format!("param_{name}"), (*value).to_owned()))
            .collect();
        let settings: Vec<(&str, String)> = settings
            .iter()
            .map(|(n, v)| (n.as_str(), v.clone()))
            .collect();
        let request = self
            .request(statement, &settings, &query_id())?
            .timeout(CATALOG_TIMEOUT);
        let reply = self.send(request, statement, 0).await?;
        Ok(parse_result(&reply.body, None, statement, 0)?
            .map(|r| r.rows)
            .unwrap_or_default())
    }

    async fn run(
        &self,
        whole: &str,
        piece: &split::Piece<'_>,
        max_rows: Option<usize>,
    ) -> Result<QueryResult> {
        let started = Instant::now();
        // HTTP forgets USE between requests, so it is remembered here, once
        // the server has said the database exists.
        if let Some(name) = use_target(piece.text) {
            let check = self.request(
                &format!("USE {}", self.quote_ident(&name)),
                &[],
                &query_id(),
            )?;
            self.send(check, whole, piece.offset).await?;
            if let Ok(mut database) = self.database.lock() {
                *database = name;
            }
            return Ok(QueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
                rows_affected: None,
                truncated: false,
                elapsed_ms: started.elapsed().as_millis() as u64,
            });
        }

        let id = query_id();
        let mut settings = Vec::new();
        if let Some(max) = max_rows {
            // The server stops near the limit; one row more says whether there was more.
            settings.push(("max_result_rows", (max + 1).to_string()));
            settings.push(("result_overflow_mode", "break".to_owned()));
        }
        let request = self.request(piece.text, &settings, &id)?;
        if let Ok(mut running) = self.running.lock() {
            *running = Some(id.clone());
        }
        let reply = self.send(request, whole, piece.offset).await;
        if let Ok(mut running) = self.running.lock()
            && running.as_deref() == Some(id.as_str())
        {
            *running = None;
        }
        let reply = reply?;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match parse_result(&reply.body, max_rows, whole, piece.offset)? {
            Some(result) => Ok(QueryResult {
                elapsed_ms,
                ..result
            }),
            None => Ok(QueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
                rows_affected: reply.written,
                truncated: false,
                elapsed_ms,
            }),
        }
    }

    async fn schema(&self, namespace: Option<&str>) -> String {
        namespace
            .filter(|n| !n.is_empty())
            .map_or_else(|| self.database(), str::to_owned)
    }

    /// `sorting_key`, `primary_key`, `partition_key` and `engine` of a table.
    async fn keys(&self, database: &str, table: &str) -> Result<Option<[String; 4]>> {
        let rows = self
            .catalog(
                "SELECT sorting_key, primary_key, partition_key, engine FROM system.tables \
                 WHERE database = {db:String} AND name = {t:String}",
                &[("db", database), ("t", table)],
            )
            .await?;
        Ok(rows
            .first()
            .map(|r| [text(&r[0]), text(&r[1]), text(&r[2]), text(&r[3])]))
    }
}

#[async_trait]
impl Connection for ClickHouseConnection {
    async fn ping(&self) -> Result<()> {
        self.catalog("SELECT 1", &[]).await.map(|_| ())
    }

    async fn execute_all(&self, request: QueryRequest) -> Result<Vec<QueryResult>> {
        let pieces = split::statements(&request.statement);
        if pieces.is_empty() {
            return Err(Error::Query("no statement to run".into()));
        }
        let mut out = Vec::with_capacity(pieces.len());
        for piece in &pieces {
            out.push(
                self.run(&request.statement, piece, request.max_rows)
                    .await?,
            );
        }
        Ok(out)
    }

    async fn server_version(&self) -> Result<Option<String>> {
        let rows = self.catalog("SELECT version()", &[]).await?;
        Ok(rows.first().map(|r| text(&r[0])))
    }

    async fn current_namespace(&self) -> Result<Option<String>> {
        Ok(Some(self.database()))
    }

    async fn list_namespaces(&self) -> Result<Vec<Namespace>> {
        let rows = self
            .catalog(
                "SELECT name FROM system.databases \
                 WHERE name NOT IN ('system', 'INFORMATION_SCHEMA', 'information_schema') ORDER BY name",
                &[],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| Namespace { name: text(&r[0]) })
            .collect())
    }

    async fn list_objects(&self, namespace: Option<&str>) -> Result<Vec<DbObject>> {
        let database = self.schema(namespace).await;
        let rows = self
            .catalog(
                "SELECT name, engine, total_rows FROM system.tables \
                 WHERE database = {db:String} AND NOT is_temporary AND NOT startsWith(name, '.inner') \
                 ORDER BY name",
                &[("db", &database)],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| DbObject {
                name: text(&r[0]),
                kind: if text(&r[1]).ends_with("View") {
                    ObjectKind::View
                } else {
                    ObjectKind::Table
                },
                row_estimate: match &r[2] {
                    Value::Int(n) => Some((*n).max(0) as u64),
                    Value::Decimal(n) => n.parse().ok(),
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
        let database = self.schema(namespace).await;
        let rows = self
            .catalog(
                "SELECT name, type, default_kind, default_expression, comment FROM system.columns \
                 WHERE database = {db:String} AND table = {t:String} ORDER BY position",
                &[("db", &database), ("t", name)],
            )
            .await?;
        if rows.is_empty() {
            return Err(Error::Query(format!("no such table or view: {name}")));
        }
        let columns = rows
            .iter()
            .map(|r| {
                let ty = text(&r[1]);
                let kind = text(&r[2]);
                let expression = text(&r[3]);
                ColumnDescription {
                    name: text(&r[0]),
                    nullable: ty.starts_with("Nullable("),
                    type_name: Some(ty),
                    // MATERIALIZED and ALIAS columns are computed: the kind says so.
                    default_value: match kind.as_str() {
                        "" => None,
                        "DEFAULT" => Some(expression),
                        other => Some(format!("{other} {expression}")),
                    },
                    // A ClickHouse key does not make a row unique: see the module notes.
                    primary_key: false,
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
        let database = self.schema(namespace).await;
        let mut out = Vec::new();
        if let Some([sorting, primary, partition, engine]) = self.keys(&database, table).await? {
            let key = |name: &str, expression: &str, method: Option<String>| IndexDescription {
                name: name.to_owned(),
                columns: key_parts(expression),
                unique: false,
                primary: false,
                method,
                size: None,
                condition: None,
            };
            if !sorting.is_empty() {
                out.push(key("ORDER BY", &sorting, Some(engine)));
            }
            if !primary.is_empty() && primary != sorting {
                out.push(key("PRIMARY KEY", &primary, None));
            }
            if !partition.is_empty() {
                out.push(key("PARTITION BY", &partition, None));
            }
        }
        let skipping = self
            .catalog(
                "SELECT name, expr, type_full, data_compressed_bytes FROM system.data_skipping_indices \
                 WHERE database = {db:String} AND table = {t:String} ORDER BY name",
                &[("db", &database), ("t", table)],
            )
            .await?;
        out.extend(skipping.iter().map(|r| IndexDescription {
            name: text(&r[0]),
            columns: vec![text(&r[1])],
            unique: false,
            primary: false,
            method: Some(text(&r[2])),
            size: text(&r[3]).parse().ok().map(size_text),
            condition: None,
        }));
        Ok(out)
    }

    async fn object_ddl(&self, namespace: Option<&str>, name: &str) -> Result<Option<String>> {
        let database = self.schema(namespace).await;
        let rows = self
            .catalog(
                &format!(
                    "SHOW CREATE TABLE {}",
                    self.qualified(Some(&database), name)
                ),
                &[],
            )
            .await?;
        Ok(rows
            .first()
            .map(|r| format!("{};", text(&r[0]).trim_end_matches(';'))))
    }

    /// Like the default, but in the table's sorting order when none is asked
    /// for: without one, ClickHouse may return pages in a different order.
    async fn fetch_rows(&self, request: TableRequest) -> Result<TablePage> {
        let started = Instant::now();
        let database = self.schema(request.namespace.as_deref()).await;
        let description = self
            .describe_object(Some(&database), &request.table)
            .await?;
        let mut order = Vec::new();
        for sort in &request.order_by {
            if !description.columns.iter().any(|c| c.name == sort.column) {
                return Err(Error::Query(format!("no such column: {}", sort.column)));
            }
            let direction = if sort.descending { "DESC" } else { "ASC" };
            order.push(format!("{} {direction}", self.quote_ident(&sort.column)));
        }
        if let Some([sorting, ..]) = self.keys(&database, &request.table).await? {
            for part in key_parts(&sorting) {
                // Only plain columns: an expression in the key is not a column name.
                if description.columns.iter().any(|c| c.name == part)
                    && !request.order_by.iter().any(|s| s.column == part)
                {
                    order.push(self.quote_ident(&part));
                }
            }
        }
        let mut statement = format!(
            "SELECT * FROM {}{}",
            self.qualified(Some(&database), &request.table),
            where_clause(request.filter.as_deref())?
        );
        if !order.is_empty() {
            statement.push_str(&format!(" ORDER BY {}", order.join(", ")));
        }
        let limit = request.limit as usize;
        statement.push_str(&format!(" LIMIT {} OFFSET {}", limit + 1, request.offset));
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

    fn quote_ident(&self, name: &str) -> String {
        SqlDialect::ClickHouse.quote_ident(name)
    }

    async fn is_alive(&self) -> bool {
        self.ping().await.is_ok()
    }

    fn dialect(&self) -> SqlDialect {
        SqlDialect::ClickHouse
    }

    async fn in_transaction(&self) -> Result<bool> {
        Ok(false)
    }

    /// New rows only, as an import writes them: changing or deleting a row is
    /// refused (see the module notes). The rows go in one INSERT, which
    /// ClickHouse writes whole or not at all; with no transactions, that is
    /// what keeps an import's retry of a rejected batch from writing rows twice.
    async fn apply_changes(&self, changes: &ChangeSet) -> Result<ApplyResult> {
        let mut rows = Vec::with_capacity(changes.changes.len());
        for change in &changes.changes {
            match change {
                RowChange::Insert { values } if !values.is_empty() => rows.push(values),
                RowChange::Insert { .. } => {
                    return Err(Error::Query("a new row needs at least one value".into()));
                }
                _ => {
                    return Err(Error::Unsupported(
                        "changing or deleting rows of a ClickHouse table from the grid",
                    ));
                }
            }
        }
        let Some(first) = rows.first() else {
            return Ok(ApplyResult {
                statements: Vec::new(),
            });
        };
        let description = self
            .describe_object(changes.namespace.as_deref(), &changes.table)
            .await?;
        let names: Vec<&str> = first.iter().map(|c| c.column.as_str()).collect();
        for name in &names {
            if !description.columns.iter().any(|c| c.name == *name) {
                return Err(Error::Query(format!("no such column: {name}")));
            }
        }
        let mut tuples = Vec::with_capacity(rows.len());
        for values in &rows {
            if values.len() != names.len() || values.iter().zip(&names).any(|(c, n)| c.column != *n)
            {
                return Err(Error::Query(
                    "every new row must give the same columns, in the same order".into(),
                ));
            }
            let literals = values
                .iter()
                .map(|c| literal(SqlDialect::ClickHouse, &c.value))
                .collect::<Result<Vec<_>>>()?;
            tuples.push(format!("({})", literals.join(", ")));
        }
        let columns: Vec<String> = names.iter().map(|n| self.quote_ident(n)).collect();
        let statement = format!(
            "INSERT INTO {} ({}) VALUES {}",
            self.qualified(changes.namespace.as_deref(), &changes.table),
            columns.join(", "),
            tuples.join(", ")
        );
        self.execute(QueryRequest {
            statement: statement.clone(),
            max_rows: None,
        })
        .await?;
        Ok(ApplyResult {
            statements: vec![statement],
        })
    }

    async fn cancel(&self) -> Result<()> {
        let id = self.running.lock().ok().and_then(|r| r.clone());
        let Some(id) = id else { return Ok(()) };
        // The id is one this driver made: letters, digits and dashes only.
        let kill = format!("KILL QUERY WHERE query_id = '{id}' SYNC");
        let request = self
            .request(&kill, &[], &query_id())?
            .timeout(CATALOG_TIMEOUT);
        self.send(request, &kill, 0).await.map(|_| ())
    }

    async fn close(&self) -> Result<()> {
        self.closed.store(true, Ordering::Relaxed);
        Ok(())
    }
}

/// A JSONCompact reply as a result; `None` for a statement that returns nothing.
/// A statement with a FORMAT of its own returns other text: that is shown as lines.
/// `whole` and `offset` place an error in the editor text, as for [`server_error`].
fn parse_result(
    body: &str,
    max_rows: Option<usize>,
    whole: &str,
    offset: usize,
) -> Result<Option<QueryResult>> {
    if body.trim().is_empty() {
        return Ok(None);
    }
    let Ok(reply) = serde_json::from_str::<Json>(body) else {
        let mut rows: Vec<Vec<Value>> = body
            .lines()
            .map(|l| vec![Value::Text(l.to_owned())])
            .collect();
        let truncated = max_rows.is_some_and(|max| rows.len() > max);
        if let Some(max) = max_rows {
            rows.truncate(max);
        }
        return Ok(Some(QueryResult {
            columns: vec![Column {
                name: "result".into(),
                type_name: None,
            }],
            rows,
            rows_affected: None,
            truncated,
            elapsed_ms: 0,
        }));
    };
    // A statement that fails, or is cancelled, after rows have started to
    // arrive still gets a 200; the error is in the body, after the rows so far.
    if let Some(exception) = reply["exception"].as_str() {
        return Err(server_error(None, exception, whole, offset));
    }
    let meta = reply["meta"].as_array().cloned().unwrap_or_default();
    let types: Vec<String> = meta
        .iter()
        .map(|m| m["type"].as_str().unwrap_or("").to_owned())
        .collect();
    let columns = meta
        .iter()
        .map(|m| Column {
            name: m["name"].as_str().unwrap_or("").to_owned(),
            type_name: m["type"].as_str().map(str::to_owned),
        })
        .collect();
    let mut rows: Vec<Vec<Value>> = reply["data"]
        .as_array()
        .map(|data| {
            data.iter()
                .map(|row| {
                    row.as_array()
                        .map(|cells| {
                            cells
                                .iter()
                                .zip(&types)
                                .map(|(v, t)| to_value(v, t))
                                .collect()
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default();
    let truncated = max_rows.is_some_and(|max| rows.len() > max);
    if let Some(max) = max_rows {
        rows.truncate(max);
    }
    Ok(Some(QueryResult {
        columns,
        rows,
        rows_affected: None,
        truncated,
        elapsed_ms: 0,
    }))
}

/// The database a `USE name` statement switches to.
fn use_target(statement: &str) -> Option<String> {
    let rest = statement.trim().trim_end_matches(';').trim();
    let (keyword, name) = rest.split_once(char::is_whitespace)?;
    if !keyword.eq_ignore_ascii_case("use") {
        return None;
    }
    let name = name.trim();
    let unquoted = ['`', '"']
        .iter()
        .find_map(|q| name.strip_prefix(*q).and_then(|n| n.strip_suffix(*q)))
        .map(|n| {
            n.replace("\\\\", "\\")
                .replace("\\`", "`")
                .replace("\\\"", "\"")
        })
        .unwrap_or_else(|| name.to_owned());
    (!unquoted.is_empty() && !unquoted.contains(char::is_whitespace)
        || name.starts_with(['`', '"']))
    .then_some(unquoted)
}

/// ` WHERE (...)` for a user-written condition, as core builds it.
fn where_clause(filter: Option<&str>) -> Result<String> {
    let Some(filter) = filter.map(str::trim).filter(|f| !f.is_empty()) else {
        return Ok(String::new());
    };
    if filter.contains(';') {
        return Err(Error::Query("a filter cannot contain ';'".into()));
    }
    Ok(format!(" WHERE ({filter}\n)"))
}

fn text(value: &Value) -> String {
    match value {
        Value::Text(s) | Value::Decimal(s) | Value::Temporal(s) => s.clone(),
        Value::Int(i) => i.to_string(),
        Value::Null => String::new(),
        other => format!("{other:?}"),
    }
}

/// The message of a refusal, without the code and version around it.
fn server_message(body: &str) -> String {
    match server_error(None, body, "", 0) {
        Error::Database { message, .. } => message,
        other => other.to_string(),
    }
}

fn authority(host: &str, port: Option<u16>) -> String {
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    match port {
        Some(port) => format!("{host}:{port}"),
        None => host,
    }
}

/// The error with its causes. reqwest's own text stops at "error sending request".
fn describe(error: &reqwest::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    text
}

fn network_error(error: reqwest::Error) -> Error {
    let error = error.without_url();
    if error.is_timeout() {
        let limit = if error.is_connect() {
            CONNECT_TIMEOUT
        } else {
            CATALOG_TIMEOUT
        };
        return Error::Timeout(limit.as_secs());
    }
    Error::ConnectionLost(describe(&error))
}

fn connect_error(error: Error) -> Error {
    match error {
        Error::ConnectionLost(message) => Error::Connect(message),
        Error::Database { code, message, .. } => Error::Connect(match code {
            Some(code) => format!("{message} ({code})"),
            None => message,
        }),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_use_statements() {
        assert_eq!(use_target("USE analytics;").as_deref(), Some("analytics"));
        assert_eq!(use_target("use `my db`").as_deref(), Some("my db"));
        assert_eq!(use_target("USER x"), None);
        assert_eq!(use_target("SELECT 1"), None);
    }

    #[test]
    fn parses_results() {
        let body = r#"{"meta":[{"name":"n","type":"UInt64"}],"data":[["1"],["2"],["3"]],"rows":3}"#;
        let result = parse_result(body, Some(2), "", 0).unwrap().unwrap();
        assert_eq!(result.rows, [vec![Value::Int(1)], vec![Value::Int(2)]]);
        assert!(result.truncated);
        assert!(parse_result("  \n", None, "", 0).unwrap().is_none());
        let csv = parse_result("1,a\n2,b\n", None, "", 0).unwrap().unwrap();
        assert_eq!(csv.columns[0].name, "result");
        assert_eq!(csv.rows.len(), 2);
        let cancelled = r#"{"meta":[],"data":[[0]],"rows":1,"exception":"Code: 394. DB::Exception: Query was cancelled. (QUERY_WAS_CANCELLED) (version 25.8)"}"#;
        match parse_result(cancelled, None, "", 0) {
            Err(Error::Database { code, message, .. }) => {
                assert_eq!(code.as_deref(), Some("394 (QUERY_WAS_CANCELLED)"));
                assert_eq!(message, "Query was cancelled");
            }
            other => panic!("{other:?}"),
        }
    }
}
