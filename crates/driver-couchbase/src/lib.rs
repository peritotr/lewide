//! Couchbase Server, through the query service's REST API (SQL++, port 8093;
//! 18093 with TLS).
//!
//! Everything goes through `POST /query/service`, so one port is enough:
//! buckets and collections come from `system:keyspaces`, documents are read
//! and written with SQL++ statements whose values travel as positional
//! parameters (`$1`), never inside the text. In the document view a bucket is
//! a database and a collection is named `scope.collection`; the key of a
//! document is shown as `_id`.
//!
//! An edit reads the document's CAS (Couchbase's version number) and writes
//! with `WHERE META(d).cas = …`, so a change made by someone else in between
//! is refused instead of overwritten.

mod query;

use std::error::Error as _;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dbclient_core::document::{
    CollectionInfo, CollectionKind, DatabaseInfo, DocumentChange, DocumentStore, FindPage,
    FindRequest, FoundDocument, MAX_PAGE, TextPart, WriteOutcome, invalid_text,
};
use dbclient_core::{
    ApplyResult, Capabilities, ChangeSet, Column, ColumnDescription, Connection, ConnectionConfig,
    DataModel, DbObject, Driver, DriverInfo, Error, Namespace, ObjectDescription, ObjectKind,
    QueryRequest, QueryResult, Result, SqlDialect, TablePage, TableRequest, TlsMode, Value,
};
use futures_util::StreamExt;
use reqwest::Url;
use reqwest::header::{AUTHORIZATION, HOST, HeaderMap, HeaderValue};
use serde_json::{Map, Value as Json, json};

use query::{ALIAS, ident, path};

const DEFAULT_PORT: u16 = 8093;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// For one whole request. The server is told the same limit, so it stops the
/// statement instead of working on after nobody waits for the answer.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
/// How many collections are counted at once when listing.
const COUNTS_AT_ONCE: usize = 8;
/// The collection every bucket has, which `system:keyspaces` lists as the bucket itself.
const DEFAULT_COLLECTION: &str = "_default._default";

pub struct CouchbaseDriver;

#[async_trait]
impl Driver for CouchbaseDriver {
    fn info(&self) -> DriverInfo {
        DriverInfo {
            id: "couchbase",
            name: "Couchbase",
            model: DataModel::Document,
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
        let mut connection = CouchbaseConnection::new(config)?;
        // Needs valid credentials and proves this is the query service.
        let version = connection
            .run("SELECT RAW ds_version()", Vec::new())
            .await
            .map_err(connect_error)?;
        connection.version = version
            .results
            .first()
            .and_then(Json::as_str)
            .map(version_text);
        let buckets = connection.buckets().await.map_err(connect_error)?;
        match &connection.bucket {
            Some(bucket) if !buckets.contains(bucket) => {
                return Err(Error::Connect(format!("there is no bucket named {bucket}")));
            }
            Some(_) => {}
            None => connection.bucket = buckets.into_iter().next(),
        }
        Ok(Box::new(connection))
    }
}

struct CouchbaseConnection {
    http: reqwest::Client,
    /// The query service's address.
    url: Url,
    version: Option<String>,
    /// The bucket the tree opens on.
    bucket: Option<String>,
}

/// A successful reply.
struct Reply {
    results: Vec<Json>,
    mutations: Option<u64>,
}

impl CouchbaseConnection {
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
        let user = config.user.as_deref().unwrap_or_default();
        let password = config.password.as_deref().unwrap_or_default();
        if !user.is_empty() {
            let mut value = HeaderValue::from_str(&format!(
                "Basic {}",
                base64(format!("{user}:{password}").as_bytes())
            ))
            .map_err(|_| Error::InvalidConfig("the user name cannot be sent in a header".into()))?;
            // Keeps the password out of debug output of requests.
            value.set_sensitive(true);
            headers.insert(AUTHORIZATION, value);
        }

        let mut builder = reqwest::Client::builder()
            // A database address is reached directly. A proxy from the
            // environment would also break the route of a tunnel.
            .no_proxy()
            // A redirect would carry the request, and with it the password, to
            // wherever the reply points. The API never redirects.
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
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
                    format!("{scheme}://{route}")
                } else {
                    // Resolving the name to the route keeps the name in the TLS handshake.
                    builder = builder.resolve(host, route);
                    format!("{scheme}://{}", authority(host, None))
                }
            }
        };
        let url = Url::parse(&format!("{base}/query/service"))
            .map_err(|_| Error::InvalidConfig(format!("not a valid host name: {host}")))?;
        let http = builder
            .default_headers(headers)
            .build()
            .map_err(|e| Error::Internal(format!("HTTP client: {}", describe(&e))))?;
        let bucket = config
            .database
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_owned);
        Ok(Self {
            http,
            url,
            version: None,
            bucket,
        })
    }

    /// Runs one statement with positional parameters.
    async fn run(&self, statement: &str, args: Vec<Json>) -> Result<Reply> {
        self.run_at(statement, args, None).await
    }

    /// Like `run`; `place` is the whole editor text and where the statement
    /// starts in it, so a syntax error can point there.
    async fn run_at(
        &self,
        statement: &str,
        args: Vec<Json>,
        place: Option<(&str, usize)>,
    ) -> Result<Reply> {
        let mut body = json!({
            "statement": statement,
            "timeout": format!("{}s", REQUEST_TIMEOUT.as_secs()),
            // Reads see every write made before them, the view's own included;
            // otherwise a count or page right after an edit can be out of date.
            "scan_consistency": "request_plus",
        });
        if !args.is_empty() {
            body["args"] = Json::Array(args);
        }
        let response = self
            .http
            .post(self.url.clone())
            .json(&body)
            .send()
            .await
            .map_err(network_error)?;
        let status = response.status();
        let text = response.text().await.map_err(network_error)?;
        let Ok(mut reply) = serde_json::from_str::<Json>(&text) else {
            if !status.is_success() {
                return Err(Error::Database {
                    code: Some(status.as_u16().to_string()),
                    message: text.trim().chars().take(300).collect(),
                    position: None,
                    line: None,
                    hint: None,
                    detail: None,
                });
            }
            return Err(unexpected(&text));
        };
        if reply["status"] != "success" {
            return Err(server_error(&reply, place));
        }
        Ok(Reply {
            results: match reply.get_mut("results").map(Json::take) {
                Some(Json::Array(results)) => results,
                _ => Vec::new(),
            },
            mutations: reply["metrics"]["mutationCount"].as_u64(),
        })
    }

    async fn buckets(&self) -> Result<Vec<String>> {
        let reply = self
            .run(
                "SELECT RAW k.name FROM system:keyspaces AS k WHERE k.`bucket` IS MISSING ORDER BY k.name",
                Vec::new(),
            )
            .await?;
        Ok(reply
            .results
            .iter()
            .filter_map(|n| n.as_str().map(str::to_owned))
            .collect())
    }

    async fn collections(&self, bucket: &str) -> Result<Vec<String>> {
        let reply = self
            .run(
                "SELECT RAW k.`scope` || \".\" || k.name FROM system:keyspaces AS k \
                 WHERE k.`bucket` = $1",
                vec![json!(bucket)],
            )
            .await?;
        let mut names: Vec<String> = reply
            .results
            .iter()
            .filter_map(|n| n.as_str().map(str::to_owned))
            .collect();
        if !names.iter().any(|n| n == DEFAULT_COLLECTION) {
            names.push(DEFAULT_COLLECTION.to_owned());
        }
        names.sort();
        Ok(names)
    }

    async fn document_count(&self, bucket: &str, collection: &str) -> Result<u64> {
        let statement = format!("SELECT RAW COUNT(*) FROM {}", keyspace(bucket, collection)?);
        let reply = self.run(&statement, Vec::new()).await?;
        reply
            .results
            .first()
            .and_then(Json::as_u64)
            .ok_or_else(|| Error::Query("the server sent no count".into()))
    }

    async fn info(&self, bucket: &str, collection: &str) -> CollectionInfo {
        CollectionInfo {
            name: collection.to_owned(),
            kind: CollectionKind::Collection,
            // A collection the user may not read is still listed.
            document_count: self.document_count(bucket, collection).await.ok(),
            avg_document_bytes: None,
            size_bytes: None,
        }
    }

    /// A document's content and CAS, or `None` when the key does not exist.
    async fn read(&self, space: &str, key: &str) -> Result<Option<(Json, Json)>> {
        let reply = self
            .run(
                &format!("SELECT {ALIAS} AS doc, META({ALIAS}).cas AS cas FROM {space} AS {ALIAS} USE KEYS $1"),
                vec![json!(key)],
            )
            .await?;
        Ok(reply
            .results
            .into_iter()
            .next()
            .map(|mut row| (row["doc"].take(), row["cas"].take())))
    }

    /// Runs a CAS-checked UPDATE built from SET and UNSET parts.
    async fn update(
        &self,
        space: &str,
        key: &str,
        cas: Json,
        set: Vec<(String, Json)>,
        unset: Vec<String>,
    ) -> Result<WriteOutcome> {
        if set.is_empty() && unset.is_empty() {
            return Ok(WriteOutcome {
                matched: 1,
                ..WriteOutcome::default()
            });
        }
        let mut args = vec![json!(key)];
        let mut statement = format!("UPDATE {space} AS {ALIAS} USE KEYS $1");
        if !set.is_empty() {
            let parts = set
                .into_iter()
                .map(|(field, value)| {
                    args.push(value);
                    Ok(format!("{} = ${}", path(&field)?, args.len()))
                })
                .collect::<Result<Vec<_>>>()?;
            statement.push_str(&format!(" SET {}", parts.join(", ")));
        }
        if !unset.is_empty() {
            let parts = unset.iter().map(|f| path(f)).collect::<Result<Vec<_>>>()?;
            statement.push_str(&format!(" UNSET {}", parts.join(", ")));
        }
        args.push(cas);
        statement.push_str(&format!(
            " WHERE META({ALIAS}).cas = ${} RETURNING META({ALIAS}).id",
            args.len()
        ));
        let reply = self.run(&statement, args).await?;
        if reply.results.is_empty() {
            return Err(Error::Query(
                "the document changed since it was read; refresh and try again".into(),
            ));
        }
        Ok(WriteOutcome {
            matched: 1,
            modified: reply.mutations.unwrap_or(1),
            ..WriteOutcome::default()
        })
    }
}

#[async_trait]
impl DocumentStore for CouchbaseConnection {
    async fn list_databases(&self) -> Result<Vec<DatabaseInfo>> {
        Ok(self
            .buckets()
            .await?
            .into_iter()
            .map(|name| DatabaseInfo {
                name,
                size_bytes: None,
            })
            .collect())
    }

    async fn list_collections(&self, database: &str) -> Result<Vec<CollectionInfo>> {
        let names = self.collections(database).await?;
        Ok(futures_util::stream::iter(names)
            .map(|name| async move { self.info(database, &name).await })
            .buffered(COUNTS_AT_ONCE)
            .collect()
            .await)
    }

    async fn collection_info(&self, database: &str, collection: &str) -> Result<CollectionInfo> {
        keyspace(database, collection)?;
        Ok(self.info(database, collection).await)
    }

    async fn find(&self, request: FindRequest) -> Result<FindPage> {
        let started = Instant::now();
        let space = keyspace(&request.database, &request.collection)?;
        let limit = request.limit.clamp(1, MAX_PAGE) as u64;
        let projection = query::projection(request.projection.as_deref())?;
        let mut statement =
            format!("SELECT META({ALIAS}).id AS id, {ALIAS} AS doc FROM {space} AS {ALIAS}");
        if let Some(filter) = query::filter(request.filter.as_deref())? {
            statement.push_str(&format!(" WHERE {filter}"));
        }
        // The key last, so pages never overlap.
        let order = match query::sort(request.sort.as_deref())? {
            Some(sort) => format!("{sort}, META({ALIAS}).id"),
            None => format!("META({ALIAS}).id"),
        };
        // One more than shown says whether there is a next page.
        statement.push_str(&format!(
            " ORDER BY {order} LIMIT {} OFFSET {}",
            limit + 1,
            request.skip
        ));
        let reply = self.run(&statement, Vec::new()).await?;
        let has_more = reply.results.len() as u64 > limit;
        let documents = reply
            .results
            .into_iter()
            .take(limit as usize)
            .map(|mut row| {
                let stored = row["doc"].take();
                let size_bytes = stored.to_string().len() as u64;
                let mut document = Map::new();
                document.insert("_id".into(), row["id"].take());
                match stored {
                    Json::Object(fields) => {
                        for (name, value) in projection.apply(fields) {
                            // The key is `_id`; a stored field of that name stays hidden.
                            if name != "_id" {
                                document.insert(name, value);
                            }
                        }
                    }
                    // A document that is a bare value (or binary, read as null).
                    other => {
                        document.insert("value".into(), other);
                    }
                }
                FoundDocument {
                    document: Json::Object(document),
                    size_bytes,
                }
            })
            .collect();
        Ok(FindPage {
            documents,
            skip: request.skip,
            has_more,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }

    async fn count(&self, database: &str, collection: &str, filter: Option<&str>) -> Result<u64> {
        let space = keyspace(database, collection)?;
        // Without a condition SQL++ answers COUNT(*) from the collection's
        // statistics, which can lag behind a write just made (seen in CI). A
        // condition makes it scan, and the scan waits for every earlier write.
        let filter =
            query::filter(filter)?.unwrap_or_else(|| format!("META({ALIAS}).id IS NOT MISSING"));
        let statement = format!("SELECT RAW COUNT(*) FROM {space} AS {ALIAS} WHERE {filter}");
        let reply = self.run(&statement, Vec::new()).await?;
        reply
            .results
            .first()
            .and_then(Json::as_u64)
            .ok_or_else(|| Error::Query("the server sent no count".into()))
    }

    async fn write(
        &self,
        database: &str,
        collection: &str,
        change: DocumentChange,
    ) -> Result<WriteOutcome> {
        let space = keyspace(database, collection)?;
        match change {
            DocumentChange::Insert { document } => {
                let Json::Object(mut fields) =
                    query::plain(query::parse_json(TextPart::Document, &document)?)
                else {
                    return Err(invalid_text(
                        TextPart::Document,
                        "a document is an object",
                        Some(1),
                    ));
                };
                // INSERT refuses a key that is taken instead of overwriting it.
                let (key, mut args) = match fields.remove("_id") {
                    Some(id) => ("$1".to_owned(), vec![json!(key_text(&id))]),
                    None => ("UUID()".to_owned(), Vec::new()),
                };
                args.push(Json::Object(fields));
                let statement = format!(
                    "INSERT INTO {space} (KEY, VALUE) VALUES ({key}, ${}) RETURNING META().id AS id",
                    args.len()
                );
                let reply = self.run(&statement, args).await?;
                Ok(WriteOutcome {
                    inserted_id: reply.results.first().map(|r| r["id"].clone()),
                    ..WriteOutcome::default()
                })
            }
            DocumentChange::Replace { id, document } => {
                let key = key_text(&id);
                let Json::Object(mut fields) =
                    query::plain(query::parse_json(TextPart::Document, &document)?)
                else {
                    return Err(invalid_text(
                        TextPart::Document,
                        "a document is an object",
                        Some(1),
                    ));
                };
                if let Some(given) = fields.remove("_id")
                    && key_text(&given) != key
                {
                    return Err(invalid_text(
                        TextPart::Document,
                        "the _id is the document's key and cannot change",
                        None,
                    ));
                }
                let Some((old, cas)) = self.read(&space, &key).await? else {
                    return Ok(WriteOutcome::default());
                };
                let unset = match &old {
                    Json::Object(old) => old
                        .keys()
                        .filter(|k| !fields.contains_key(*k))
                        .cloned()
                        .collect(),
                    _ => Vec::new(),
                };
                let set = fields
                    .into_iter()
                    .filter(|(k, v)| old.get(k) != Some(v))
                    .collect();
                self.update(&space, &key, cas, set, unset).await
            }
            DocumentChange::Update { id, update } => {
                let key = key_text(&id);
                let paths = query::set_paths(&update)?;
                let Some((old, cas)) = self.read(&space, &key).await? else {
                    return Ok(WriteOutcome::default());
                };
                // Applied here, so objects missing on a path are created as
                // MongoDB does; SQL++'s SET leaves such a path out.
                let mut new = old.clone();
                query::apply_set(&mut new, paths)?;
                let set = match new {
                    Json::Object(fields) => fields
                        .into_iter()
                        .filter(|(k, v)| old.get(k) != Some(v))
                        .collect(),
                    _ => Vec::new(),
                };
                self.update(&space, &key, cas, set, Vec::new()).await
            }
            DocumentChange::Delete { id } => {
                let reply = self
                    .run(
                        &format!("DELETE FROM {space} USE KEYS $1"),
                        vec![json!(key_text(&id))],
                    )
                    .await?;
                Ok(WriteOutcome {
                    deleted: reply.mutations.unwrap_or(0),
                    ..WriteOutcome::default()
                })
            }
        }
    }
}

#[async_trait]
impl Connection for CouchbaseConnection {
    async fn ping(&self) -> Result<()> {
        self.run("SELECT RAW 1", Vec::new()).await.map(|_| ())
    }

    async fn execute_all(&self, request: QueryRequest) -> Result<Vec<QueryResult>> {
        let pieces = query::statements(&request.statement);
        if pieces.is_empty() {
            return Err(Error::Query("no statement to run".into()));
        }
        let mut out = Vec::with_capacity(pieces.len());
        for piece in &pieces {
            let started = Instant::now();
            let reply = self
                .run_at(
                    piece.text,
                    Vec::new(),
                    Some((&request.statement, piece.offset)),
                )
                .await?;
            let mut result = result_of(&reply.results);
            if result.columns.is_empty() {
                result.rows_affected = reply.mutations;
            }
            result.truncated = request.max_rows.is_some_and(|max| result.rows.len() > max);
            if let Some(max) = request.max_rows {
                result.rows.truncate(max);
            }
            result.elapsed_ms = started.elapsed().as_millis() as u64;
            out.push(result);
        }
        Ok(out)
    }

    async fn server_version(&self) -> Result<Option<String>> {
        Ok(self.version.clone())
    }

    async fn current_namespace(&self) -> Result<Option<String>> {
        Ok(self.bucket.clone())
    }

    async fn list_namespaces(&self) -> Result<Vec<Namespace>> {
        Ok(self
            .buckets()
            .await?
            .into_iter()
            .map(|name| Namespace { name })
            .collect())
    }

    async fn list_objects(&self, namespace: Option<&str>) -> Result<Vec<DbObject>> {
        let Some(bucket) = namespace.map(str::to_owned).or_else(|| self.bucket.clone()) else {
            return Ok(Vec::new());
        };
        Ok(self
            .list_collections(&bucket)
            .await?
            .into_iter()
            .map(|c| DbObject {
                name: c.name,
                kind: ObjectKind::Collection,
                row_estimate: c.document_count,
            })
            .collect())
    }

    async fn describe_object(
        &self,
        _namespace: Option<&str>,
        name: &str,
    ) -> Result<ObjectDescription> {
        // Documents have no fixed fields; the key is the one every document has.
        Ok(ObjectDescription {
            name: name.to_owned(),
            columns: vec![ColumnDescription {
                name: "_id".into(),
                type_name: Some("string".into()),
                nullable: false,
                default_value: None,
                primary_key: true,
                comment: None,
                enum_values: None,
            }],
        })
    }

    fn dialect(&self) -> SqlDialect {
        // No statement is built from the dialect: the document operations write their own.
        SqlDialect::Postgres
    }

    async fn in_transaction(&self) -> Result<bool> {
        Ok(false)
    }

    // The four below default to building SQL for tables, which a document store has not.

    async fn preview_changes(&self, _changes: &ChangeSet) -> Result<Vec<String>> {
        Err(Error::Unsupported(
            "row changes; use the document operations",
        ))
    }

    async fn apply_changes(&self, _changes: &ChangeSet) -> Result<ApplyResult> {
        Err(Error::Unsupported(
            "row changes; use the document operations",
        ))
    }

    async fn fetch_rows(&self, _request: TableRequest) -> Result<TablePage> {
        Err(Error::Unsupported(
            "table pages; use the document operations",
        ))
    }

    async fn count_rows(
        &self,
        _namespace: Option<&str>,
        _table: &str,
        _filter: Option<&str>,
    ) -> Result<u64> {
        Err(Error::Unsupported(
            "row counts; use the document operations",
        ))
    }

    fn quote_ident(&self, name: &str) -> String {
        ident(name)
    }

    fn document(&self) -> Option<&dyn DocumentStore> {
        Some(self)
    }

    async fn close(&self) -> Result<()> {
        // Each request stands alone; the idle sockets close with the client.
        Ok(())
    }
}

/// `` `bucket`.`scope`.`collection` `` for a collection named `scope.collection`.
fn keyspace(bucket: &str, collection: &str) -> Result<String> {
    let (scope, name) = collection.split_once('.').ok_or_else(|| {
        Error::Query(format!(
            "{collection}: a collection is named scope.collection"
        ))
    })?;
    Ok(format!(
        "{}.{}.{}",
        ident(bucket),
        ident(scope),
        ident(name)
    ))
}

/// A key as text: the view sends the `_id` as it showed it.
fn key_text(id: &Json) -> String {
    match id {
        Json::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `8.0.2-5503-community` as `8.0.2 (Community)`.
fn version_text(raw: &str) -> String {
    let version = raw.split('-').next().unwrap_or(raw);
    match raw.rsplit_once('-').map(|(_, edition)| edition) {
        Some(edition) if edition.chars().all(char::is_alphabetic) => {
            let mut chars = edition.chars();
            let edition: String = chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default();
            format!("{version} ({edition})")
        }
        _ => raw.to_owned(),
    }
}

/// Rows of a query-tab reply as a result. Objects become columns by their
/// keys, in the order they first appear; anything else is one `value` column.
fn result_of(rows: &[Json]) -> QueryResult {
    let objects = !rows.is_empty() && rows.iter().all(Json::is_object);
    let mut names: Vec<String> = Vec::new();
    if objects {
        for row in rows {
            if let Json::Object(fields) = row {
                for key in fields.keys() {
                    if !names.contains(key) {
                        names.push(key.clone());
                    }
                }
            }
        }
    } else if !rows.is_empty() {
        names.push("value".into());
    }
    let rows = rows
        .iter()
        .map(|row| {
            if objects {
                names
                    .iter()
                    .map(|n| cell(row.get(n).unwrap_or(&Json::Null)))
                    .collect()
            } else {
                vec![cell(row)]
            }
        })
        .collect();
    QueryResult {
        columns: names
            .into_iter()
            .map(|name| Column {
                name,
                type_name: None,
            })
            .collect(),
        rows,
        rows_affected: None,
        truncated: false,
        elapsed_ms: 0,
    }
}

fn cell(value: &Json) -> Value {
    match value {
        Json::Null => Value::Null,
        Json::Bool(flag) => Value::Bool(*flag),
        Json::Number(number) => match number.as_i64() {
            Some(int) => Value::Int(int),
            None if number.is_u64() => Value::Decimal(number.to_string()),
            None => Value::Float(number.as_f64().unwrap_or(f64::NAN)),
        },
        Json::String(text) => Value::Text(text.clone()),
        other => Value::Json(other.clone()),
    }
}

/// The first error of a failed reply. A syntax error's line and column (within
/// the statement) become a position in the editor's text when `place` is given.
fn server_error(reply: &Json, place: Option<(&str, usize)>) -> Error {
    let error = &reply["errors"][0];
    let code = error["code"].as_i64();
    let mut message = error["msg"]
        .as_str()
        .unwrap_or("the statement failed")
        .to_owned();
    if code == Some(2120) || code == Some(10000) {
        // Couchbase's codes for a refused user or password.
        message = format!("not authorized: {message}");
    }
    let position = match (place, error["line"].as_u64(), error["column"].as_u64()) {
        (Some((whole, offset)), Some(line), Some(column)) => {
            let statement = &whole[offset.min(whole.len())..];
            let line_start: usize = statement
                .split_inclusive('\n')
                .take(line.saturating_sub(1) as usize)
                .map(str::len)
                .sum();
            let before = whole
                .get(..(offset + line_start).min(whole.len()))
                .unwrap_or(whole);
            Some(before.chars().count() as u32 + column as u32)
        }
        _ => None,
    };
    Error::Database {
        code: code.map(|c| c.to_string()),
        message,
        position,
        line: None,
        hint: None,
        detail: None,
    }
}

/// The same failures, worded for a connection attempt.
fn connect_error(error: Error) -> Error {
    match error {
        Error::ConnectionLost(message) => Error::Connect(message),
        Error::Database { message, .. } => Error::Connect(message),
        Error::Internal(message) => Error::Connect(format!(
            "this does not look like Couchbase's query service: {message}"
        )),
        other => other,
    }
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

/// Standard base64 with padding, for the Basic authorization header.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(
            keyspace("shop", "sales.orders").unwrap(),
            "`shop`.`sales`.`orders`"
        );
        assert!(keyspace("shop", "orders").is_err());
        assert_eq!(version_text("8.0.2-5503-community"), "8.0.2 (Community)");
        assert_eq!(version_text("7.6.4-5146-enterprise"), "7.6.4 (Enterprise)");
        assert_eq!(key_text(&json!(7)), "7");
        assert_eq!(base64(b"Administrator:Test"), "QWRtaW5pc3RyYXRvcjpUZXN0");
        assert_eq!(base64(b"ab"), "YWI=");
    }

    #[test]
    fn rows_become_results() {
        let rows: Vec<Json> = serde_json::from_str(r#"[{"a":1,"b":"x"},{"a":2,"c":[1]}]"#).unwrap();
        let result = result_of(&rows);
        let names: Vec<&str> = result.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c"]);
        assert_eq!(
            result.rows[1],
            [Value::Int(2), Value::Null, Value::Json(json!([1]))]
        );
        let raw = result_of(&[json!(3), json!("x")]);
        assert_eq!(raw.columns[0].name, "value");
        assert_eq!(raw.rows, [[Value::Int(3)], [Value::Text("x".into())]]);
        assert!(result_of(&[]).columns.is_empty());
    }

    #[test]
    fn errors_point_at_the_statement() {
        let reply = json!({"status": "fatal", "errors": [{"code": 3000, "line": 2, "column": 3, "msg": "syntax error - line 2, column 3, near 'FRM'"}]});
        let whole = "SELECT 1;\nSELECT *\n  FRM x";
        let Error::Database { code, position, .. } = server_error(&reply, Some((whole, 10))) else {
            panic!("expected a database error");
        };
        assert_eq!(code.as_deref(), Some("3000"));
        assert_eq!(position, Some(19 + 3));
        let refused = json!({"status": "fatal", "errors": [{"code": 2120, "msg": "Failure to authenticate user"}]});
        assert!(
            matches!(connect_error(server_error(&refused, None)), Error::Connect(m) if m.contains("not authorized"))
        );
    }
}
