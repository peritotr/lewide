//! Elasticsearch and OpenSearch, through their REST API.
//!
//! The two share the endpoints used here, so one driver serves both; the
//! server says which it is. They are shown with the document view:
//!
//! - the cluster is the one database, its indices are the collections;
//! - a document is its `_source` with `_id` in front, as MongoDB has it;
//! - the filter field takes Query DSL, a `{"field": value}` shorthand, or
//!   Lucene query string text; projection and sort take Elasticsearch's own
//!   forms and MongoDB's `{field: 1}`.
//!
//! Writes wait for a refresh (`refresh=wait_for`), so the view shows them at
//! once. Query tabs take requests as Kibana's Dev Tools console writes them.

mod query;

use std::error::Error as _;
use std::net::IpAddr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HOST, HeaderMap, HeaderValue};
use reqwest::{Method, StatusCode, Url};
use serde_json::{Value as Json, json};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// For everything but the query tab, whose requests may take long on purpose.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Searches cannot page past this many hits with `from` and `size`.
const RESULT_WINDOW: u64 = 10_000;

pub struct ElasticsearchDriver;

#[async_trait]
impl Driver for ElasticsearchDriver {
    fn info(&self) -> DriverInfo {
        DriverInfo {
            id: "elasticsearch",
            name: "Elasticsearch",
            model: DataModel::Document,
            capabilities: Capabilities {
                tls: true,
                cancel: true,
                ..Capabilities::default()
            },
            default_port: Some(9200),
            desktop_only: false,
        }
    }

    async fn connect(&self, config: &ConnectionConfig) -> Result<Box<dyn Connection>> {
        let mut connection = SearchConnection::new(config)?;
        // Checks the address and the credentials, and learns what the server is.
        let root = connection
            .call(Method::GET, "/", None, REQUEST_TIMEOUT)
            .await
            .map_err(connect_error)?;
        connection.cluster = root["cluster_name"]
            .as_str()
            .unwrap_or("cluster")
            .to_owned();
        let number = root["version"]["number"].as_str().unwrap_or("?");
        let product = match root["version"]["distribution"].as_str() {
            Some("opensearch") => "OpenSearch",
            _ => "Elasticsearch",
        };
        connection.version = format!("{number} ({product})");
        Ok(Box::new(connection))
    }
}

struct SearchConnection {
    http: reqwest::Client,
    /// Scheme and address, without a path.
    base: Url,
    cluster: String,
    version: String,
    closed: AtomicBool,
    /// The `X-Opaque-Id` of the console request now running, for `cancel`.
    running: Mutex<Option<String>>,
    /// Numbers the console requests of this connection.
    requests: AtomicU64,
}

impl SearchConnection {
    fn new(config: &ConnectionConfig) -> Result<Self> {
        let host = config
            .host
            .as_deref()
            .map(|h| h.trim().trim_start_matches('[').trim_end_matches(']'))
            .filter(|h| !h.is_empty())
            .unwrap_or("localhost");
        let encrypted = config.tls.mode != TlsMode::Off;
        let port = config.port.unwrap_or(9200);
        let scheme = if encrypted { "https" } else { "http" };

        let mut headers = HeaderMap::new();
        let header = |text: String, what: &str| {
            let mut value = HeaderValue::from_str(&text).map_err(|_| {
                Error::InvalidConfig(format!(
                    "the {what} contains characters a header cannot carry"
                ))
            })?;
            // Keeps credentials out of debug output of requests.
            value.set_sensitive(true);
            Ok::<_, Error>(value)
        };
        let api_key = config
            .params
            .get("apiKey")
            .map(|k| k.trim())
            .filter(|k| !k.is_empty());
        if let Some(key) = api_key {
            headers.insert(AUTHORIZATION, header(format!("ApiKey {key}"), "API key")?);
        } else if let Some(user) = config.user.as_deref().filter(|u| !u.is_empty()) {
            let pair = format!("{user}:{}", config.password.as_deref().unwrap_or(""));
            headers.insert(
                AUTHORIZATION,
                header(
                    format!("Basic {}", base64(pair.as_bytes())),
                    "user name or password",
                )?,
            );
        }

        let mut builder = reqwest::Client::builder()
            // A database address is reached directly. A proxy from the
            // environment would also break the route of a tunnel.
            .no_proxy()
            // A redirect would carry the credentials to wherever it points.
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
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
        let base = Url::parse(&base)
            .map_err(|_| Error::InvalidConfig(format!("not a valid host name: {host}")))?;
        let http = builder
            .default_headers(headers)
            .build()
            .map_err(|e| Error::Internal(format!("HTTP client: {}", describe(&e))))?;
        Ok(Self {
            http,
            base,
            cluster: String::new(),
            version: String::new(),
            closed: AtomicBool::new(false),
            running: Mutex::new(None),
            requests: AtomicU64::new(0),
        })
    }

    /// One request; the reply's JSON, or the server's error.
    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<Body>,
        timeout: Duration,
    ) -> Result<Json> {
        let (status, json) = self.send(method, path, body, timeout).await?;
        if status.is_success() {
            Ok(json)
        } else {
            Err(server_error(status, &json))
        }
    }

    /// Like [`call`](Self::call), but hands back any status for callers that
    /// treat some (a missing document) as an answer.
    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Body>,
        timeout: Duration,
    ) -> Result<(StatusCode, Json)> {
        self.send_tagged(method, path, body, timeout, None).await
    }

    /// Like [`send`](Self::send), with an `X-Opaque-Id` the server keeps on
    /// the task it starts, so the task can be found and cancelled.
    async fn send_tagged(
        &self,
        method: Method,
        path: &str,
        body: Option<Body>,
        timeout: Duration,
        opaque: Option<&str>,
    ) -> Result<(StatusCode, Json)> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(Error::ConnectionLost("the connection was closed".into()));
        }
        let url = self
            .base
            .join(path)
            .map_err(|_| Error::Query(format!("not a valid path: {path}")))?;
        let mut request = self.http.request(method, url);
        if let Some(opaque) = opaque {
            request = request.header("X-Opaque-Id", opaque);
        }
        if timeout > Duration::ZERO {
            request = request.timeout(timeout);
        }
        request =
            match body {
                None => request,
                Some(Body::Json(json)) => request
                    .header(CONTENT_TYPE, "application/json")
                    .body(json.to_string()),
                Some(Body::Ndjson(text)) => request
                    .header(CONTENT_TYPE, "application/x-ndjson")
                    .body(if text.ends_with('\n') {
                        text
                    } else {
                        format!("{text}\n")
                    }),
            };
        let response = request.send().await.map_err(network_error)?;
        let status = response.status();
        let text = response.text().await.map_err(network_error)?;
        let json = if text.trim().is_empty() {
            Json::Null
        } else {
            serde_json::from_str(&text).unwrap_or(Json::String(text))
        };
        Ok((status, json))
    }

    fn index_path(index: &str, rest: &str) -> String {
        format!("/{}{rest}", encode(index))
    }

    fn check_database(&self, database: &str) -> Result<()> {
        if database == self.cluster {
            Ok(())
        } else {
            Err(Error::Query(format!(
                "this cluster is {}, not {database}",
                self.cluster
            )))
        }
    }

    async fn indices(&self, pattern: Option<&str>) -> Result<Vec<CollectionInfo>> {
        let path = match pattern {
            Some(index) => format!(
                "/_cat/indices/{}?format=json&bytes=b&h=index,docs.count,store.size",
                encode(index)
            ),
            None => "/_cat/indices?format=json&bytes=b&h=index,docs.count,store.size".to_owned(),
        };
        let reply = self.call(Method::GET, &path, None, REQUEST_TIMEOUT).await?;
        let number = |v: &Json| {
            v.as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .or_else(|| v.as_u64())
        };
        let mut list: Vec<CollectionInfo> = reply
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| {
                let name = row["index"].as_str()?.to_owned();
                let count = number(&row["docs.count"]);
                let size = number(&row["store.size"]);
                Some(CollectionInfo {
                    name,
                    kind: CollectionKind::Collection,
                    document_count: count,
                    avg_document_bytes: match (size, count) {
                        (Some(s), Some(c)) if c > 0 => Some(s / c),
                        _ => None,
                    },
                    size_bytes: size,
                })
            })
            // Indices starting with a dot belong to the system and to plugins.
            .filter(|c| pattern.is_some() || !c.name.starts_with('.'))
            .collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(list)
    }

    /// The stored document, or `None` when there is none with that id.
    async fn get_source(&self, index: &str, id: &str) -> Result<Option<(Json, Json, Json)>> {
        let (status, reply) = self
            .send(
                Method::GET,
                &Self::index_path(index, &format!("/_doc/{}", encode(id))),
                None,
                REQUEST_TIMEOUT,
            )
            .await?;
        if status == StatusCode::NOT_FOUND && reply.get("found") == Some(&Json::Bool(false)) {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(server_error(status, &reply));
        }
        Ok(Some((
            reply["_source"].clone(),
            reply["_seq_no"].clone(),
            reply["_primary_term"].clone(),
        )))
    }

    /// Mapped fields of an index as columns: `address.city` for nested objects.
    async fn mapped_fields(&self, index: &str) -> Result<Vec<(String, String)>> {
        let reply = self
            .call(
                Method::GET,
                &Self::index_path(index, "/_mapping"),
                None,
                REQUEST_TIMEOUT,
            )
            .await?;
        let mut fields = Vec::new();
        fn walk(prefix: &str, properties: &Json, out: &mut Vec<(String, String)>) {
            let Some(map) = properties.as_object() else {
                return;
            };
            for (name, spec) in map {
                let path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}.{name}")
                };
                match spec.get("properties") {
                    Some(nested) => walk(&path, nested, out),
                    None => out.push((path, spec["type"].as_str().unwrap_or("object").to_owned())),
                }
            }
        }
        if let Some(map) = reply.as_object() {
            for mapping in map.values() {
                walk("", &mapping["mappings"]["properties"], &mut fields);
            }
        }
        Ok(fields)
    }
}

enum Body {
    Json(Json),
    Ndjson(String),
}

fn id_text(id: &Json) -> String {
    match id {
        Json::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[async_trait]
impl DocumentStore for SearchConnection {
    async fn list_databases(&self) -> Result<Vec<DatabaseInfo>> {
        Ok(vec![DatabaseInfo {
            name: self.cluster.clone(),
            size_bytes: None,
        }])
    }

    async fn list_collections(&self, database: &str) -> Result<Vec<CollectionInfo>> {
        self.check_database(database)?;
        self.indices(None).await
    }

    async fn collection_info(&self, database: &str, collection: &str) -> Result<CollectionInfo> {
        self.check_database(database)?;
        self.indices(Some(collection))
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| Error::Query(format!("no index named {collection}")))
    }

    async fn find(&self, request: FindRequest) -> Result<FindPage> {
        let started = Instant::now();
        if request
            .pipeline
            .as_deref()
            .is_some_and(|p| !p.trim().is_empty())
        {
            return Err(Error::Unsupported("aggregation pipelines on Elasticsearch"));
        }
        self.check_database(&request.database)?;
        let limit = request.limit.clamp(1, MAX_PAGE) as u64;
        if request.skip + limit > RESULT_WINDOW {
            return Err(Error::Query(format!(
                "searches page through the first {RESULT_WINDOW} documents only; narrow the filter or change the sort"
            )));
        }
        let mut body = json!({
            "query": query::query(request.filter.as_deref())?,
            "from": request.skip,
            // One more than shown says whether there is a next page.
            "size": (limit + 1).min(RESULT_WINDOW - request.skip),
            "track_total_hits": false,
        });
        if let Some(source) = query::source(request.projection.as_deref())? {
            body["_source"] = source;
        }
        if let Some(sort) = query::sort(request.sort.as_deref())? {
            body["sort"] = sort;
        }
        let reply = self
            .call(
                Method::POST,
                &SearchConnection::index_path(&request.collection, "/_search"),
                Some(Body::Json(body)),
                REQUEST_TIMEOUT,
            )
            .await?;
        let hits = reply["hits"]["hits"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let has_more = hits.len() as u64 > limit;
        let documents = hits
            .into_iter()
            .take(limit as usize)
            .map(|hit| {
                let mut document = serde_json::Map::new();
                document.insert("_id".into(), hit["_id"].clone());
                if let Json::Object(source) = &hit["_source"] {
                    for (k, v) in source {
                        document.insert(k.clone(), v.clone());
                    }
                }
                let size_bytes = hit["_source"].to_string().len() as u64;
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
        self.check_database(database)?;
        let body = json!({ "query": query::query(filter)? });
        let reply = self
            .call(
                Method::POST,
                &SearchConnection::index_path(collection, "/_count"),
                Some(Body::Json(body)),
                REQUEST_TIMEOUT,
            )
            .await?;
        reply["count"]
            .as_u64()
            .ok_or_else(|| Error::Query("the server sent no count".into()))
    }

    async fn write(
        &self,
        database: &str,
        collection: &str,
        change: DocumentChange,
    ) -> Result<WriteOutcome> {
        self.check_database(database)?;
        let doc_path = |id: &str, rest: &str| {
            SearchConnection::index_path(
                collection,
                &format!("/{rest}/{}?refresh=wait_for", encode(id)),
            )
        };
        match change {
            DocumentChange::Insert { document } => {
                let Json::Object(mut source) =
                    query::plain(query::parse_json(TextPart::Document, &document)?)
                else {
                    return Err(invalid_text(
                        TextPart::Document,
                        "a document is an object",
                        Some(1),
                    ));
                };
                let reply = match source.remove("_id") {
                    // `_create` refuses an id that is taken instead of overwriting it.
                    Some(id) => {
                        self.call(
                            Method::PUT,
                            &doc_path(&id_text(&id), "_create"),
                            Some(Body::Json(Json::Object(source))),
                            REQUEST_TIMEOUT,
                        )
                        .await?
                    }
                    None => {
                        self.call(
                            Method::POST,
                            &SearchConnection::index_path(collection, "/_doc?refresh=wait_for"),
                            Some(Body::Json(Json::Object(source))),
                            REQUEST_TIMEOUT,
                        )
                        .await?
                    }
                };
                Ok(WriteOutcome {
                    inserted_id: Some(reply["_id"].clone()),
                    ..WriteOutcome::default()
                })
            }
            DocumentChange::Replace { id, document } => {
                let id = id_text(&id);
                let Json::Object(mut source) =
                    query::plain(query::parse_json(TextPart::Document, &document)?)
                else {
                    return Err(invalid_text(
                        TextPart::Document,
                        "a document is an object",
                        Some(1),
                    ));
                };
                source.remove("_id");
                let Some((_, seq_no, term)) = self.get_source(collection, &id).await? else {
                    return Ok(WriteOutcome::default());
                };
                self.replace(collection, &id, Json::Object(source), &seq_no, &term)
                    .await
            }
            DocumentChange::Update { id, update } => {
                let id = id_text(&id);
                let Some((mut source, seq_no, term)) = self.get_source(collection, &id).await?
                else {
                    return Ok(WriteOutcome::default());
                };
                query::apply_set(&mut source, &update)?;
                self.replace(collection, &id, source, &seq_no, &term).await
            }
            DocumentChange::Delete { id } => {
                let (status, reply) = self
                    .send(
                        Method::DELETE,
                        &doc_path(&id_text(&id), "_doc"),
                        None,
                        REQUEST_TIMEOUT,
                    )
                    .await?;
                if status == StatusCode::NOT_FOUND && reply["result"] == "not_found" {
                    return Ok(WriteOutcome::default());
                }
                if !status.is_success() {
                    return Err(server_error(status, &reply));
                }
                Ok(WriteOutcome {
                    deleted: 1,
                    ..WriteOutcome::default()
                })
            }
        }
    }
}

impl SearchConnection {
    /// Writes a whole document only if nobody changed it since it was read.
    async fn replace(
        &self,
        index: &str,
        id: &str,
        source: Json,
        seq_no: &Json,
        term: &Json,
    ) -> Result<WriteOutcome> {
        let path = SearchConnection::index_path(
            index,
            &format!(
                "/_doc/{}?refresh=wait_for&if_seq_no={seq_no}&if_primary_term={term}",
                encode(id)
            ),
        );
        let (status, reply) = self
            .send(
                Method::PUT,
                &path,
                Some(Body::Json(source)),
                REQUEST_TIMEOUT,
            )
            .await?;
        if status == StatusCode::CONFLICT {
            return Err(Error::Query(
                "the document changed since it was read; refresh and try again".into(),
            ));
        }
        if !status.is_success() {
            return Err(server_error(status, &reply));
        }
        Ok(WriteOutcome {
            matched: 1,
            modified: u64::from(reply["result"] == "updated"),
            ..WriteOutcome::default()
        })
    }

    /// One console request's reply as a result table.
    fn table(reply: Json, max_rows: Option<usize>, started: Instant) -> QueryResult {
        let column = |name: &str| Column {
            name: name.into(),
            type_name: None,
        };
        let limit = max_rows.unwrap_or(usize::MAX);
        let (columns, mut rows): (Vec<Column>, Vec<Vec<Value>>) =
            if let Some(hits) = reply["hits"]["hits"].as_array() {
                (
                    ["_id", "_index", "_score", "_source"].map(column).into(),
                    hits.iter()
                        .map(|h| {
                            vec![
                                h["_id"]
                                    .as_str()
                                    .map_or(Value::Null, |s| Value::Text(s.into())),
                                h["_index"]
                                    .as_str()
                                    .map_or(Value::Null, |s| Value::Text(s.into())),
                                h["_score"].as_f64().map_or(Value::Null, Value::Float),
                                Value::Json(h["_source"].clone()),
                            ]
                        })
                        .collect(),
                )
            } else if let Some(list) = reply
                .as_array()
                .filter(|l| l.iter().all(Json::is_object) && !l.is_empty())
            {
                // `_cat` APIs with format=json: one object per row.
                let names: Vec<String> = list[0]
                    .as_object()
                    .map(|o| o.keys().cloned().collect())
                    .unwrap_or_default();
                (
                    names.iter().map(|n| column(n)).collect(),
                    list.iter()
                        .map(|row| {
                            names
                                .iter()
                                .map(|n| match &row[n] {
                                    Json::Null => Value::Null,
                                    Json::String(s) => Value::Text(s.clone()),
                                    other => Value::Json(other.clone()),
                                })
                                .collect()
                        })
                        .collect(),
                )
            } else {
                (
                    vec![column("response")],
                    vec![vec![match reply {
                        Json::String(s) => Value::Text(s),
                        other => Value::Json(other),
                    }]],
                )
            };
        let truncated = rows.len() > limit;
        rows.truncate(limit);
        QueryResult {
            columns,
            rows,
            rows_affected: None,
            truncated,
            elapsed_ms: started.elapsed().as_millis() as u64,
        }
    }
}

#[async_trait]
impl Connection for SearchConnection {
    async fn ping(&self) -> Result<()> {
        self.call(Method::GET, "/", None, REQUEST_TIMEOUT)
            .await
            .map(|_| ())
    }

    async fn execute_all(&self, request: QueryRequest) -> Result<Vec<QueryResult>> {
        let mut results = Vec::new();
        for console in query::console(&request.statement)? {
            let started = Instant::now();
            let method = Method::from_bytes(console.method.as_bytes())
                .map_err(|_| Error::Query(format!("not a method: {}", console.method)))?;
            let body = match console.body {
                None => None,
                Some(text)
                    if console.path.contains("_bulk") || console.path.contains("_msearch") =>
                {
                    Some(Body::Ndjson(text))
                }
                Some(text) => Some(Body::Json(query::parse_json(TextPart::Command, &text)?)),
            };
            let opaque = format!(
                "lewide-{}-{}",
                std::process::id(),
                self.requests.fetch_add(1, Ordering::Relaxed)
            );
            if let Ok(mut running) = self.running.lock() {
                *running = Some(opaque.clone());
            }
            let sent = self
                .send_tagged(method, &console.path, body, Duration::ZERO, Some(&opaque))
                .await;
            if let Ok(mut running) = self.running.lock()
                && running.as_deref() == Some(opaque.as_str())
            {
                *running = None;
            }
            let (status, reply) = sent?;
            if !status.is_success() {
                return Err(server_error(status, &reply));
            }
            results.push(Self::table(reply, request.max_rows, started));
        }
        Ok(results)
    }

    async fn server_version(&self) -> Result<Option<String>> {
        Ok(Some(self.version.clone()))
    }

    async fn current_namespace(&self) -> Result<Option<String>> {
        Ok(Some(self.cluster.clone()))
    }

    async fn list_namespaces(&self) -> Result<Vec<Namespace>> {
        Ok(vec![Namespace {
            name: self.cluster.clone(),
        }])
    }

    async fn list_objects(&self, _namespace: Option<&str>) -> Result<Vec<DbObject>> {
        Ok(self
            .indices(None)
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
        let mut columns = vec![ColumnDescription {
            name: "_id".into(),
            type_name: Some("keyword".into()),
            nullable: false,
            default_value: None,
            primary_key: true,
            comment: None,
            enum_values: None,
        }];
        columns.extend(
            self.mapped_fields(name)
                .await?
                .into_iter()
                .map(|(field, kind)| ColumnDescription {
                    name: field,
                    type_name: Some(kind),
                    nullable: true,
                    default_value: None,
                    primary_key: false,
                    comment: None,
                    enum_values: None,
                }),
        );
        Ok(ObjectDescription {
            name: name.to_owned(),
            columns,
        })
    }

    fn dialect(&self) -> SqlDialect {
        // Nothing here is SQL; the dialect only matters for SQL tools, which are off.
        SqlDialect::Postgres
    }

    async fn in_transaction(&self) -> Result<bool> {
        Ok(false)
    }

    // The four below default to building SQL, which this server would not understand.

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
        name.to_owned()
    }

    fn document(&self) -> Option<&dyn DocumentStore> {
        Some(self)
    }

    /// Cancels the tasks the running console request started, found by its
    /// `X-Opaque-Id`. Searches and the `_by_query` APIs stop; a request whose
    /// task cannot be cancelled (an index or a bulk write) runs to its end.
    async fn cancel(&self) -> Result<()> {
        let Some(opaque) = self.running.lock().ok().and_then(|r| r.clone()) else {
            return Ok(());
        };
        let tasks = self
            .call(Method::GET, "/_tasks?group_by=none", None, REQUEST_TIMEOUT)
            .await?;
        let ids: Vec<String> = tasks["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|task| task["headers"]["X-Opaque-Id"] == opaque.as_str())
            .filter_map(|task| {
                Some(format!(
                    "{}:{}",
                    task["node"].as_str()?,
                    task["id"].as_u64()?
                ))
            })
            .collect();
        for id in ids {
            // A task that ended meanwhile is no longer there; that is not a failure.
            let _ = self
                .send(
                    Method::POST,
                    &format!("/_tasks/{}/_cancel", encode(&id)),
                    None,
                    REQUEST_TIMEOUT,
                )
                .await?;
        }
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        self.closed.store(true, Ordering::Relaxed);
        Ok(())
    }
}

/// The server's error: `{"error": {"type", "reason", "root_cause"}, "status"}`.
fn server_error(status: StatusCode, reply: &Json) -> Error {
    let error = &reply["error"];
    // The root cause usually says more than the wrapper (`search_phase_execution_exception`).
    let cause = error["root_cause"]
        .get(0)
        .filter(|c| c["reason"].is_string())
        .unwrap_or(error);
    let reason = cause["reason"]
        .as_str()
        .or_else(|| error.as_str())
        .map(str::to_owned);
    let kind = cause["type"].as_str().map(str::to_owned);
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Error::Connect(reason.unwrap_or_else(|| status.to_string()));
    }
    Error::Database {
        code: kind.or_else(|| Some(status.as_u16().to_string())),
        message: reason.unwrap_or_else(|| match reply {
            Json::Null => status.to_string(),
            other => other.to_string(),
        }),
        position: None,
        line: None,
        hint: None,
        detail: error["caused_by"]["reason"].as_str().map(str::to_owned),
    }
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

/// Percent-encodes a path segment: index names and ids may hold any character.
fn encode(segment: &str) -> String {
    let mut out = String::new();
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'*' | b',' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
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
            REQUEST_TIMEOUT
        };
        return Error::Timeout(limit.as_secs());
    }
    Error::ConnectionLost(describe(&error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b"elastic:test"), "ZWxhc3RpYzp0ZXN0");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64("ş".as_bytes()), "xZ8=");
    }

    #[test]
    fn path_segments_are_encoded() {
        assert_eq!(encode("logs-2026.11"), "logs-2026.11");
        assert_eq!(encode("a/b?c"), "a%2Fb%3Fc");
        assert_eq!(encode("İ"), "%C4%B0");
    }

    #[test]
    fn server_errors_use_the_root_cause() {
        let reply = json!({
            "error": {
                "type": "search_phase_execution_exception",
                "reason": "all shards failed",
                "root_cause": [{ "type": "query_shard_exception", "reason": "failed to create query" }],
            },
            "status": 400
        });
        let Error::Database { code, message, .. } = server_error(StatusCode::BAD_REQUEST, &reply)
        else {
            panic!("not a database error");
        };
        assert_eq!(code.as_deref(), Some("query_shard_exception"));
        assert_eq!(message, "failed to create query");
        assert!(matches!(
            server_error(StatusCode::UNAUTHORIZED, &json!({})),
            Error::Connect(_)
        ));
    }

    #[test]
    fn search_replies_become_rows() {
        let reply = json!({"hits": {"hits": [{"_id": "1", "_index": "i", "_score": 1.5, "_source": {"a": 1}}]}});
        let table = SearchConnection::table(reply, None, Instant::now());
        assert_eq!(table.columns.len(), 4);
        assert_eq!(table.rows[0][0], Value::Text("1".into()));
        assert_eq!(table.rows[0][3], Value::Json(json!({"a": 1})));
        let cat = json!([{"index": "a", "docs.count": "3"}, {"index": "b", "docs.count": "4"}]);
        let table = SearchConnection::table(cat, Some(1), Instant::now());
        assert_eq!(table.columns[1].name, "docs.count");
        assert!(table.truncated);
    }
}
