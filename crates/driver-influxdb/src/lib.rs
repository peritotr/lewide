//! InfluxDB 3 (Core and Enterprise), through its HTTP API.
//!
//! SQL goes to `POST /api/v3/query_sql` with the database named in each
//! request: HTTP keeps no session, so `USE` is remembered here. The server
//! takes one statement per request, so the editor's text is cut into
//! statements first. Replies are JSON: one object per row, in the order of
//! the SELECT list, where a null value is left out of its row.
//!
//! The driver only reads. InfluxDB is written with line protocol, not SQL,
//! and a row has no key that identifies it, so the grid is read-only and
//! columns never claim to be a primary key. Databases are the namespaces;
//! a database's tables live in its `iox` schema.

mod split;
mod tls;

use std::error::Error as _;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dbclient_core::{
    ApplyResult, Capabilities, ChangeSet, Column, ColumnDescription, Connection, ConnectionConfig,
    DataModel, DbObject, Driver, DriverInfo, Error, Namespace, ObjectDescription, ObjectKind,
    QueryRequest, QueryResult, Result, SqlDialect, TablePage, TableRequest, TlsMode, Value,
};
use reqwest::header::{AUTHORIZATION, HOST, HeaderMap, HeaderValue};
use reqwest::{Method, Url};
use serde_json::{Value as Json, json};

const DEFAULT_PORT: u16 = 8181;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// For one whole request. A query over a long time range can take a while; a
/// server that says nothing for this long is not coming back.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
/// The schema that holds a database's own tables; `system` and
/// `information_schema` are the server's.
const TABLE_SCHEMA: &str = "iox";
/// The server's own database, hidden from the tree.
const INTERNAL_DATABASE: &str = "_internal";

pub struct InfluxDbDriver;

#[async_trait]
impl Driver for InfluxDbDriver {
    fn info(&self) -> DriverInfo {
        DriverInfo {
            id: "influxdb",
            name: "InfluxDB",
            model: DataModel::Relational,
            capabilities: Capabilities {
                sql: true,
                tls: true,
                ..Capabilities::default()
            },
            default_port: Some(DEFAULT_PORT),
            desktop_only: false,
        }
    }

    async fn connect(&self, config: &ConnectionConfig) -> Result<Box<dyn Connection>> {
        let connection = InfluxDbConnection::new(config)?;
        // Needs a valid token, and proves this is InfluxDB 3.
        let databases = connection.databases().await.map_err(connect_error)?;
        let chosen = connection.database();
        if chosen.is_empty() {
            // Without a database in the form, the first one is used.
            if let Some(first) = databases.first()
                && let Ok(mut database) = connection.database.lock()
            {
                *database = first.clone();
            }
        } else if !databases.contains(&chosen) {
            return Err(Error::Connect(format!(
                "there is no database named {chosen}"
            )));
        }
        Ok(Box::new(connection))
    }
}

struct InfluxDbConnection {
    http: reqwest::Client,
    /// Scheme and address, without a path.
    base: Url,
    /// The database unqualified names resolve to; empty when the server has none yet.
    database: Mutex<String>,
}

/// The text of a failed request and where it went wrong.
struct Failure {
    status: u16,
    body: String,
}

impl InfluxDbConnection {
    fn new(config: &ConnectionConfig) -> Result<Self> {
        let host = config
            .host
            .as_deref()
            .map(|h| h.trim().trim_start_matches('[').trim_end_matches(']'))
            .filter(|h| !h.is_empty())
            .unwrap_or("localhost");
        let port = config.port.unwrap_or(DEFAULT_PORT);
        let encrypted = config.tls.mode != TlsMode::Off;
        let scheme = if encrypted { "https" } else { "http" };

        let mut headers = HeaderMap::new();
        if let Some(token) = config.password.as_deref().filter(|t| !t.is_empty()) {
            let mut value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| {
                Error::InvalidConfig("the token contains characters a header cannot carry".into())
            })?;
            // Keeps the token out of debug output of requests.
            value.set_sensitive(true);
            headers.insert(AUTHORIZATION, value);
        }

        let mut builder = reqwest::Client::builder()
            // A database address is reached directly. A proxy from the
            // environment would also break the route of a tunnel.
            .no_proxy()
            // A redirect would carry the request, and with it the token, to
            // wherever the reply points. The API never redirects.
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            // Given even when the connection is not encrypted: left to itself,
            // reqwest builds a TLS setup from a process-wide rustls provider
            // that nothing guarantees has been chosen.
            .tls_backend_preconfigured(tls::client_config(&config.tls, host)?);

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
                    format!("{scheme}://{route}")
                } else {
                    // Resolving the name to the route keeps the name in the TLS handshake.
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
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_owned();
        Ok(Self {
            http,
            base,
            database: Mutex::new(database),
        })
    }

    fn database(&self) -> String {
        self.database.lock().map(|d| d.clone()).unwrap_or_default()
    }

    /// The database a request about `namespace` goes to.
    fn target(&self, namespace: Option<&str>) -> Result<String> {
        let database = namespace
            .filter(|n| !n.is_empty())
            .map_or_else(|| self.database(), str::to_owned);
        if database.is_empty() {
            return Err(Error::Query(
                "no database is chosen: the server has none yet, or write USE name first".into(),
            ));
        }
        Ok(database)
    }

    /// Sends one request and returns the body of a successful reply.
    async fn send(
        &self,
        method: Method,
        path: &[&str],
        query: &[(&str, &str)],
        body: Option<&Json>,
    ) -> Result<std::result::Result<(String, HeaderMap), Failure>> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| Error::Internal("the server address cannot carry a path".into()))?
            .extend(path);
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let mut request = self.http.request(method, url);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.map_err(network_error)?;
        let status = response.status();
        let headers = response.headers().clone();
        let text = response.text().await.map_err(network_error)?;
        Ok(if status.is_success() {
            Ok((text, headers))
        } else {
            Err(Failure {
                status: status.as_u16(),
                body: text,
            })
        })
    }

    /// The databases, without the server's own.
    async fn databases(&self) -> Result<Vec<String>> {
        let (text, _) = self
            .send(
                Method::GET,
                &["api", "v3", "configure", "database"],
                &[("format", "json")],
                None,
            )
            .await?
            .map_err(|f| request_error(&f, "", 0))?;
        let reply: Json = serde_json::from_str(&text).map_err(|_| unexpected(&text))?;
        let mut names: Vec<String> = reply
            .as_array()
            .ok_or_else(|| unexpected(&text))?
            .iter()
            .filter_map(|d| d["iox::database"].as_str().map(str::to_owned))
            .filter(|name| name != INTERNAL_DATABASE)
            .collect();
        names.sort();
        Ok(names)
    }

    /// Runs one statement on `database` and returns its rows as JSON objects.
    /// `whole` and `offset` place an error in the editor's text.
    async fn query(
        &self,
        database: &str,
        statement: &str,
        whole: &str,
        offset: usize,
    ) -> Result<Vec<Json>> {
        let body = json!({ "db": database, "q": statement, "format": "json" });
        let (text, _) = self
            .send(Method::POST, &["api", "v3", "query_sql"], &[], Some(&body))
            .await?
            .map_err(|f| request_error(&f, whole, offset))?;
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        match serde_json::from_str(&text) {
            Ok(Json::Array(rows)) => Ok(rows),
            _ => Err(unexpected(&text)),
        }
    }

    /// A catalog query whose values are all text.
    async fn catalog(&self, database: &str, statement: &str) -> Result<Vec<Json>> {
        self.query(database, statement, statement, 0).await
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
            if !self.databases().await?.contains(&name) {
                return Err(Error::Query(format!("there is no database named {name}")));
            }
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
        let database = self.target(None)?;
        let rows = self
            .query(&database, piece.text, whole, piece.offset)
            .await?;
        let mut result = result_of(&rows, None);
        result.truncated = max_rows.is_some_and(|max| result.rows.len() > max);
        if let Some(max) = max_rows {
            result.rows.truncate(max);
        }
        result.elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(result)
    }
}

#[async_trait]
impl Connection for InfluxDbConnection {
    async fn ping(&self) -> Result<()> {
        self.databases().await.map(|_| ())
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
        // `/ping` names the version and the product in its headers.
        let reply = self.send(Method::GET, &["ping"], &[], None).await?;
        let Ok((_, headers)) = reply else {
            return Ok(None);
        };
        let header = |name: &str| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        Ok(
            header("x-influxdb-version").map(|version| match header("x-influxdb-build") {
                Some(build) => format!("{version} ({build})"),
                None => version,
            }),
        )
    }

    async fn current_namespace(&self) -> Result<Option<String>> {
        Ok(Some(self.database()).filter(|d| !d.is_empty()))
    }

    async fn list_namespaces(&self) -> Result<Vec<Namespace>> {
        Ok(self
            .databases()
            .await?
            .into_iter()
            .map(|name| Namespace { name })
            .collect())
    }

    async fn list_objects(&self, namespace: Option<&str>) -> Result<Vec<DbObject>> {
        let database = self.target(namespace)?;
        let rows = self
            .catalog(
                &database,
                &format!(
                    "SELECT table_name, table_type FROM information_schema.tables \
                     WHERE table_schema = '{TABLE_SCHEMA}' ORDER BY table_name"
                ),
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| DbObject {
                name: r["table_name"].as_str().unwrap_or_default().to_owned(),
                kind: if r["table_type"] == "VIEW" {
                    ObjectKind::View
                } else {
                    ObjectKind::Table
                },
                // InfluxDB keeps no row count; counting reads every file.
                row_estimate: None,
            })
            .collect())
    }

    async fn describe_object(
        &self,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<ObjectDescription> {
        let database = self.target(namespace)?;
        let rows = self
            .catalog(
                &database,
                &format!(
                    "SELECT column_name, data_type, is_nullable FROM information_schema.columns \
                     WHERE table_schema = '{TABLE_SCHEMA}' AND table_name = {} ORDER BY ordinal_position",
                    string_literal(name)
                ),
            )
            .await?;
        if rows.is_empty() {
            return Err(Error::Query(format!("no such table: {name}")));
        }
        let columns = rows
            .iter()
            .map(|r| {
                let name = r["column_name"].as_str().unwrap_or_default().to_owned();
                let type_name = r["data_type"].as_str().unwrap_or_default().to_owned();
                ColumnDescription {
                    comment: Some(role(&name, &type_name).to_owned()),
                    nullable: r["is_nullable"] != "NO",
                    name,
                    type_name: Some(type_name),
                    default_value: None,
                    // A row has no key that identifies it: see the module notes.
                    primary_key: false,
                    enum_values: None,
                }
            })
            .collect();
        Ok(ObjectDescription {
            name: name.to_owned(),
            columns,
        })
    }

    /// Like the default, but against the table's database, with every column
    /// filled in (a null value is left out of a reply row) and in time order
    /// when none is asked for, tags breaking ties, so pages do not shift.
    async fn fetch_rows(&self, request: TableRequest) -> Result<TablePage> {
        let started = Instant::now();
        let database = self.target(request.namespace.as_deref())?;
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
        // Time first, then the tags, which together name one point.
        for wanted in ["time", "tag"] {
            for column in &description.columns {
                let is = role(&column.name, column.type_name.as_deref().unwrap_or("")) == wanted;
                if is && !request.order_by.iter().any(|s| s.column == column.name) {
                    order.push(self.quote_ident(&column.name));
                }
            }
        }
        let mut statement = format!(
            "SELECT * FROM {}{}",
            self.quote_ident(&request.table),
            where_clause(request.filter.as_deref())?
        );
        if !order.is_empty() {
            statement.push_str(&format!(" ORDER BY {}", order.join(", ")));
        }
        let limit = request.limit as usize;
        statement.push_str(&format!(" LIMIT {} OFFSET {}", limit + 1, request.offset));
        let rows = self.query(&database, &statement, &statement, 0).await?;
        let mut result = result_of(&rows, Some(&description.columns));
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

    async fn count_rows(
        &self,
        namespace: Option<&str>,
        table: &str,
        filter: Option<&str>,
    ) -> Result<u64> {
        let database = self.target(namespace)?;
        let statement = format!(
            "SELECT count(*) AS n FROM {}{}",
            self.quote_ident(table),
            where_clause(filter)?
        );
        let rows = self.query(&database, &statement, &statement, 0).await?;
        rows.first()
            .and_then(|r| r["n"].as_u64())
            .ok_or_else(|| Error::Query("unexpected count result".into()))
    }

    /// The table alone: a database is not part of a statement, it is chosen per request.
    fn qualified(&self, _namespace: Option<&str>, table: &str) -> String {
        self.quote_ident(table)
    }

    fn quote_ident(&self, name: &str) -> String {
        SqlDialect::DuckDb.quote_ident(name)
    }

    fn dialect(&self) -> SqlDialect {
        // InfluxDB's SQL (Apache DataFusion) quotes as PostgreSQL does and
        // takes a backslash in a string as plain text, as DuckDB does.
        SqlDialect::DuckDb
    }

    async fn in_transaction(&self) -> Result<bool> {
        Ok(false)
    }

    async fn apply_changes(&self, _changes: &ChangeSet) -> Result<ApplyResult> {
        Err(Error::Unsupported(
            "writing to InfluxDB (it takes line protocol, not SQL)",
        ))
    }

    async fn close(&self) -> Result<()> {
        // Each request stands alone; the idle sockets close with the client.
        Ok(())
    }
}

/// What a column is in InfluxDB's model: the `time` column, a tag (kept as a
/// dictionary of strings) or a field.
fn role(name: &str, type_name: &str) -> &'static str {
    if name == "time" && type_name.starts_with("Timestamp") {
        "time"
    } else if type_name.starts_with("Dictionary(") {
        "tag"
    } else {
        "field"
    }
}

/// Rows of a reply as a result. With `columns` (a table's), every column is
/// there in table order; without, the columns are the keys of the rows in the
/// order they first appear, so a column that is null in every row is missing.
fn result_of(rows: &[Json], columns: Option<&[ColumnDescription]>) -> QueryResult {
    let (names, types): (Vec<String>, Vec<Option<String>>) = match columns {
        Some(columns) => columns
            .iter()
            .map(|c| (c.name.clone(), c.type_name.clone()))
            .unzip(),
        None => {
            let mut names: Vec<String> = Vec::new();
            for row in rows {
                if let Json::Object(fields) = row {
                    // A key first seen in a later row goes after the known key
                    // before it in that row, or at the end when there is none.
                    let mut at: Option<usize> = None;
                    for key in fields.keys() {
                        match names.iter().position(|n| n == key) {
                            Some(found) => at = Some(found + 1),
                            None => {
                                let place = at.unwrap_or(names.len()).min(names.len());
                                names.insert(place, key.clone());
                                at = Some(place + 1);
                            }
                        }
                    }
                }
            }
            let types = vec![None; names.len()];
            (names, types)
        }
    };
    let rows = rows
        .iter()
        .map(|row| {
            names
                .iter()
                .zip(&types)
                .map(|(name, type_name)| {
                    to_value(row.get(name).unwrap_or(&Json::Null), type_name.as_deref())
                })
                .collect()
        })
        .collect();
    QueryResult {
        columns: names
            .into_iter()
            .zip(types)
            .map(|(name, type_name)| Column { name, type_name })
            .collect(),
        rows,
        rows_affected: None,
        truncated: false,
        elapsed_ms: 0,
    }
}

/// One JSON value as a cell. The column's Arrow type, when known, says which
/// strings are times; otherwise a string shaped like a timestamp is taken as one.
fn to_value(value: &Json, type_name: Option<&str>) -> Value {
    match value {
        Json::Null => Value::Null,
        Json::Bool(flag) => Value::Bool(*flag),
        Json::Number(number) => match number.as_i64() {
            Some(int) => Value::Int(int),
            // Above i64 (a UInt64 field): kept exact as text.
            None if number.is_u64() => Value::Decimal(number.to_string()),
            None => Value::Float(number.as_f64().unwrap_or(f64::NAN)),
        },
        Json::String(text) => {
            let temporal = match type_name {
                Some(t) => {
                    t.starts_with("Timestamp") || t.starts_with("Date") || t.starts_with("Time")
                }
                None => looks_like_time(text),
            };
            if temporal {
                Value::Temporal(text.clone())
            } else {
                Value::Text(text.clone())
            }
        }
        other => Value::Json(other.clone()),
    }
}

/// `2023-11-14T22:13:20`, with or without fractions and a zone.
fn looks_like_time(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() >= 19
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && b[16] == b':'
}

/// The database a `USE name` statement switches to.
fn use_target(statement: &str) -> Option<String> {
    if split::first_word(statement) != "USE" {
        return None;
    }
    // Past the comments in front, which `first_word` looked through.
    let mut rest = statement.trim_start();
    loop {
        if let Some(after) = rest.strip_prefix("--") {
            rest = after.split_once('\n').map_or("", |(_, r)| r).trim_start();
        } else if let Some(after) = rest.strip_prefix("/*") {
            rest = after.split_once("*/").map_or("", |(_, r)| r).trim_start();
        } else {
            break;
        }
    }
    let rest = rest.trim().trim_end_matches(';').trim();
    let (_, name) = rest.split_once(char::is_whitespace)?;
    let name = name.trim();
    let name = match name.strip_prefix('"').and_then(|n| n.strip_suffix('"')) {
        Some(quoted) => quoted.replace("\"\"", "\""),
        None => name.to_owned(),
    };
    (!name.is_empty()).then_some(name)
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

fn string_literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// A refused request as the error the interface shows. SQL errors come as
/// plain text (`SQL error: ParserError("… at Line: 1, Column: 8")`), others as
/// `{"error": "…"}`. A line and column become a position in the whole text.
fn request_error(failure: &Failure, whole: &str, offset: usize) -> Error {
    let body = failure.body.trim();
    let said = serde_json::from_str::<Json>(body)
        .ok()
        .and_then(|v| v["error"].as_str().map(str::to_owned))
        .unwrap_or_else(|| body.chars().take(500).collect());
    let mut message = said.strip_prefix("SQL error: ").unwrap_or(&said).to_owned();
    // `ParserError("…")`: the text inside, without Rust's quoting.
    if let Some(open) = message.find("(\"")
        && message.ends_with("\")")
        && message[..open].bytes().all(|b| b.is_ascii_alphabetic())
    {
        message = message[open + 2..message.len() - 2].replace("\\\"", "\"");
    }
    let position = place(&message).map(|(line, column)| {
        let statement = &whole[offset.min(whole.len())..];
        let line_start: usize = statement
            .split_inclusive('\n')
            .take(line.saturating_sub(1) as usize)
            .map(str::len)
            .sum();
        let at = offset + line_start;
        let before = whole.get(..at.min(whole.len())).unwrap_or(whole);
        (before.chars().count() as u32) + column
    });
    let message = match message.find(" at Line: ") {
        Some(at) if position.is_some() => message[..at].to_owned(),
        _ => message,
    };
    Error::Database {
        code: Some(failure.status.to_string()),
        message: if message.is_empty() {
            format!("the server answered with HTTP status {}", failure.status)
        } else {
            message
        },
        position,
        line: None,
        hint: None,
        detail: None,
    }
}

/// `… at Line: 2, Column: 8` in a parser message.
fn place(message: &str) -> Option<(u32, u32)> {
    let (_, rest) = message.split_once(" at Line: ")?;
    let (line, rest) = rest.split_once(", Column: ")?;
    let column: String = rest.chars().take_while(char::is_ascii_digit).collect();
    Some((line.trim().parse().ok()?, column.parse().ok()?))
}

/// `host:port` for a URL or a `Host` header, with IPv6 addresses in brackets.
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

/// The error with its causes. reqwest's own text stops at "error sending
/// request"; the reason (refused, certificate, name not found) is underneath.
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
    // The address is known to the user and makes the message long.
    let error = error.without_url();
    if error.is_timeout() {
        let limit = if error.is_connect() {
            CONNECT_TIMEOUT
        } else {
            REQUEST_TIMEOUT
        };
        return Error::Timeout(limit.as_secs());
    }
    Error::ConnectionLost(describe(&error))
}

/// A reply that is not shaped like the API's. The start of it is kept: it
/// usually shows what answered instead (a proxy's error page, another service).
fn unexpected(text: &str) -> Error {
    let start: String = text.trim().chars().take(200).collect();
    Error::Internal(format!("unexpected reply from the server: {start}"))
}

/// The same failures, worded for a connection attempt.
fn connect_error(error: Error) -> Error {
    match error {
        Error::ConnectionLost(message) => Error::Connect(message),
        Error::Database {
            code: Some(code),
            message,
            ..
        } if code == "401" || code == "403" => {
            Error::Connect(format!("the server did not accept the token: {message}"))
        }
        Error::Database {
            code: Some(code),
            message,
            ..
        } => Error::Connect(format!("HTTP status {code}: {message}")),
        Error::Internal(message) => Error::Connect(format!(
            "this does not look like an InfluxDB 3 server: {message}"
        )),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_use_statements() {
        assert_eq!(use_target("USE sensors;").as_deref(), Some("sensors"));
        assert_eq!(
            use_target("use \"my \"\"db\"\"\"").as_deref(),
            Some("my \"db\"")
        );
        assert_eq!(use_target("SELECT 1"), None);
        assert_eq!(use_target("-- x\nUSE a").as_deref(), Some("a"));
    }

    #[test]
    fn columns_follow_the_rows() {
        let rows: Vec<Json> = serde_json::from_str(
            r#"[{"a":1,"c":"x"},{"a":2,"b":true,"c":"2023-11-14T22:13:20"},{"d":18446744073709551615}]"#,
        )
        .unwrap();
        let result = result_of(&rows, None);
        let names: Vec<&str> = result.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c", "d"]);
        assert_eq!(
            result.rows[0],
            [
                Value::Int(1),
                Value::Null,
                Value::Text("x".into()),
                Value::Null
            ]
        );
        assert_eq!(
            result.rows[1][2],
            Value::Temporal("2023-11-14T22:13:20".into())
        );
        assert_eq!(
            result.rows[2][3],
            Value::Decimal("18446744073709551615".into())
        );
    }

    #[test]
    fn table_columns_fill_missing_values() {
        let column = |name: &str, ty: &str| ColumnDescription {
            name: name.into(),
            type_name: Some(ty.into()),
            nullable: true,
            default_value: None,
            primary_key: false,
            comment: None,
            enum_values: None,
        };
        let columns = [column("note", "Utf8"), column("time", "Timestamp(ns)")];
        let rows: Vec<Json> = serde_json::from_str(r#"[{"time":"2023-11-14T22:13:20"}]"#).unwrap();
        let result = result_of(&rows, Some(&columns));
        assert_eq!(
            result.rows[0],
            [Value::Null, Value::Temporal("2023-11-14T22:13:20".into())]
        );
        assert_eq!(role("room", "Dictionary(Int32, Utf8)"), "tag");
        assert_eq!(role("time", "Timestamp(ns)"), "time");
        assert_eq!(role("temp", "Float64"), "field");
    }

    #[test]
    fn errors_point_at_their_place() {
        let whole = "SELECT 1;\nSELECT *\n  FRM home";
        let failure = Failure {
            status: 400,
            body: r#"SQL error: ParserError("Expected: end of statement, found: home at Line: 2, Column: 7")"#.into(),
        };
        let Error::Database {
            message,
            position,
            code,
            ..
        } = request_error(&failure, whole, 10)
        else {
            panic!("expected a database error");
        };
        assert_eq!(code.as_deref(), Some("400"));
        assert_eq!(message, "Expected: end of statement, found: home");
        // Line 2 of the second statement starts after "SELECT 1;\nSELECT *\n" (19 characters).
        assert_eq!(position, Some(19 + 7));

        let json = Failure {
            status: 404,
            body: r#"{"error":"query error: database not found: nope"}"#.into(),
        };
        assert!(matches!(
            request_error(&json, "", 0),
            Error::Database { message, position: None, .. } if message == "query error: database not found: nope"
        ));
        assert!(matches!(
            connect_error(request_error(&Failure { status: 401, body: "{}".into() }, "", 0)),
            Error::Connect(m) if m.contains("token")
        ));
    }
}
