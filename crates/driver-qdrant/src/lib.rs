//! Qdrant driver.
//!
//! Talks to the REST API (JSON over HTTP, port 6333) rather than gRPC: it
//! needs no generated client, and every reply is plain JSON that can be handed
//! to the interface as it is.
//!
//! The driver only reads. Searches go through the Query API
//! (`POST /collections/{name}/points/query`, Qdrant 1.10 and later), which
//! takes a vector or a point id in the same request.

mod filter;
mod parse;
mod tls;

use std::error::Error as _;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dbclient_core::vector::{
    CollectionInfo, Point, PointId, PointPage, ScoredPoint, ScrollRequest, SearchQuery,
    SearchRequest, SearchResult, VectorStore,
};
use dbclient_core::{
    ApplyResult, Capabilities, ChangeSet, Connection, ConnectionConfig, DataModel, DbObject,
    Driver, DriverInfo, Error, Namespace, ObjectDescription, ObjectKind, QueryRequest, QueryResult,
    Result, SqlDialect, TlsMode,
};
use futures_util::StreamExt;
use reqwest::header::{HOST, HeaderMap, HeaderValue};
use reqwest::{Method, Url};
use serde_json::{Value, json};

const DEFAULT_PORT: u16 = 6333;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// For one whole request. A search over a large collection without an index
/// can take a while; a server that says nothing for this long is not coming back.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
/// How many collections are asked for their point count at once when listing.
const INFO_REQUESTS_AT_ONCE: usize = 8;

pub struct QdrantDriver;

#[async_trait]
impl Driver for QdrantDriver {
    fn info(&self) -> DriverInfo {
        DriverInfo {
            id: "qdrant",
            name: "Qdrant",
            model: DataModel::Vector,
            capabilities: Capabilities {
                vector_search: true,
                tls: true,
                ..Capabilities::default()
            },
            default_port: Some(DEFAULT_PORT),
            desktop_only: false,
        }
    }

    async fn connect(&self, config: &ConnectionConfig) -> Result<Box<dyn Connection>> {
        let connection = QdrantConnection::new(config)?;

        // The root page answers without a key, so it says the server is there
        // and which version it is, but not that the key is right.
        let root = connection
            .send(Method::GET, &[], None)
            .await
            .map_err(connect_error)?;
        let version = serde_json::from_str::<Value>(&root)
            .ok()
            .and_then(|v| v["version"].as_str().map(str::to_owned));

        // Listing collections needs a valid key and proves this is the Qdrant API.
        connection.list_collections().await.map_err(connect_error)?;

        Ok(Box::new(QdrantConnection {
            version,
            ..connection
        }))
    }
}

struct QdrantConnection {
    http: reqwest::Client,
    /// Scheme and address, without a path.
    base: Url,
    version: Option<String>,
}

impl QdrantConnection {
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
        if let Some(key) = config.password.as_deref().filter(|k| !k.is_empty()) {
            let mut key = HeaderValue::from_str(key).map_err(|_| {
                Error::InvalidConfig("the API key contains characters a header cannot carry".into())
            })?;
            // Keeps the key out of debug output of requests.
            key.set_sensitive(true);
            headers.insert("api-key", key);
        }

        let mut builder = reqwest::Client::builder()
            // A database address is reached directly. A proxy from the
            // environment would also break the route of a tunnel.
            .no_proxy()
            // A redirect would carry the request, and with it the key, to
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
                    // No name to resolve, so the route goes into the URL. The
                    // certificate is still checked against `host`, see `tls`.
                    format!("{scheme}://{route}")
                } else {
                    // Resolving the name to the route keeps the name in the TLS
                    // handshake. The URL has no port so that the route's is used:
                    // a port written in the URL would win over it.
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
            version: None,
        })
    }

    /// Sends one request and returns the body of a successful reply.
    async fn send(&self, method: Method, path: &[&str], body: Option<&Value>) -> Result<String> {
        let mut url = self.base.clone();
        // Segments are escaped one by one, so a collection name cannot reach another path.
        url.path_segments_mut()
            .map_err(|_| Error::Internal("the server address cannot carry a path".into()))?
            .extend(path);
        let mut request = self.http.request(method, url);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.map_err(network_error)?;
        let status = response.status();
        let text = response.text().await.map_err(network_error)?;
        if !status.is_success() {
            return Err(status_error(status.as_u16(), &text));
        }
        Ok(text)
    }

    /// Sends one API request and returns the `result` part of the reply.
    async fn call(&self, method: Method, path: &[&str], body: Option<&Value>) -> Result<Value> {
        let text = self.send(method, path, body).await?;
        let mut reply: Value = serde_json::from_str(&text).map_err(|_| parse::unexpected(&text))?;
        match reply.get_mut("result") {
            Some(result) => Ok(result.take()),
            None => Err(parse::unexpected(&text)),
        }
    }

    /// Whether a higher score is a closer match for the vector a search uses.
    async fn higher_is_better(&self, collection: &str, using: Option<&str>) -> Result<bool> {
        let info = self.collection_info(collection).await?;
        Ok(info
            .vectors
            .iter()
            .find(|v| v.name.as_deref() == using)
            .is_none_or(|v| v.higher_is_better))
    }
}

#[async_trait]
impl Connection for QdrantConnection {
    async fn ping(&self) -> Result<()> {
        // Answers without touching any collection.
        self.send(Method::GET, &["healthz"], None).await.map(|_| ())
    }

    async fn execute_all(&self, _request: QueryRequest) -> Result<Vec<QueryResult>> {
        Err(Error::Unsupported("statements"))
    }

    async fn server_version(&self) -> Result<Option<String>> {
        Ok(self.version.clone())
    }

    async fn current_namespace(&self) -> Result<Option<String>> {
        // Collections are the top level; there is nothing above them.
        Ok(None)
    }

    async fn list_namespaces(&self) -> Result<Vec<Namespace>> {
        Ok(Vec::new())
    }

    async fn list_objects(&self, _namespace: Option<&str>) -> Result<Vec<DbObject>> {
        let names = self.list_collections().await?;
        // The list has names only; the count takes one request per collection.
        let objects = futures_util::stream::iter(names)
            .map(|name| async move {
                // A collection that cannot be described is still listed.
                let row_estimate = self
                    .collection_info(&name)
                    .await
                    .ok()
                    .and_then(|i| i.points);
                DbObject {
                    name,
                    kind: ObjectKind::Collection,
                    row_estimate,
                }
            })
            .buffered(INFO_REQUESTS_AT_ONCE)
            .collect()
            .await;
        Ok(objects)
    }

    async fn describe_object(
        &self,
        _namespace: Option<&str>,
        _name: &str,
    ) -> Result<ObjectDescription> {
        // Points have no columns. The vector view reads `collection_info` instead.
        Err(Error::Unsupported("columns"))
    }

    fn dialect(&self) -> SqlDialect {
        // Unused: no statement is ever built for this driver. The trait needs a value.
        SqlDialect::Postgres
    }

    async fn in_transaction(&self) -> Result<bool> {
        Ok(false)
    }

    async fn apply_changes(&self, _changes: &ChangeSet) -> Result<ApplyResult> {
        Err(Error::Unsupported("editing"))
    }

    fn quote_ident(&self, name: &str) -> String {
        name.to_owned()
    }

    fn vector(&self) -> Option<&dyn VectorStore> {
        Some(self)
    }

    async fn close(&self) -> Result<()> {
        // Nothing to end: each request stands alone, and the idle sockets
        // close when the client is dropped.
        Ok(())
    }
}

#[async_trait]
impl VectorStore for QdrantConnection {
    async fn list_collections(&self) -> Result<Vec<String>> {
        let result = self.call(Method::GET, &["collections"], None).await?;
        let mut names: Vec<String> = result["collections"]
            .as_array()
            .ok_or_else(|| parse::unexpected(&result.to_string()))?
            .iter()
            .filter_map(|c| c["name"].as_str().map(str::to_owned))
            .collect();
        names.sort();
        Ok(names)
    }

    async fn collection_info(&self, collection: &str) -> Result<CollectionInfo> {
        let result = self
            .call(Method::GET, &["collections", collection], None)
            .await?;
        Ok(parse::collection_info(collection, &result))
    }

    async fn search(&self, request: SearchRequest) -> Result<SearchResult> {
        if request.limit == 0 {
            return Err(Error::Query("top K must be at least 1".into()));
        }
        let started = Instant::now();

        let mut body = json!({
            "query": match &request.query {
                SearchQuery::Vector { values } => json!(values),
                SearchQuery::Point { id } => parse::id_to_json(id),
            },
            "limit": request.limit,
            "with_payload": request.with_payload,
            "with_vector": request.with_vector,
        });
        if let Some(using) = &request.using {
            body["using"] = json!(using);
        }
        if let Some(filter) = filter::filter(&request.filter)? {
            body["filter"] = filter;
        }
        let path = ["collections", &request.collection, "points", "query"];
        let result = self.call(Method::POST, &path, Some(&body)).await?;
        let points = result["points"]
            .as_array()
            .ok_or_else(|| parse::unexpected(&result.to_string()))?;

        // The minimum score is applied here and not by the server, so that the
        // number of hits it removed is known. The outcome is the same: hits
        // come best first, and the limit cuts off the tail.
        let keep: Box<dyn Fn(f64) -> bool + Send> = match request.min_score {
            None => Box::new(|_| true),
            Some(min) => {
                let using = request.using.as_deref();
                if self.higher_is_better(&request.collection, using).await? {
                    Box::new(move |score| score >= min)
                } else {
                    // A distance: the limit is the farthest a hit may be.
                    Box::new(move |score| score <= min)
                }
            }
        };

        let mut hits = Vec::new();
        let mut below_min_score = 0;
        for point in points {
            let score = point["score"]
                .as_f64()
                .ok_or_else(|| parse::unexpected(&point.to_string()))?;
            if !keep(score) {
                below_min_score += 1;
                continue;
            }
            hits.push(ScoredPoint {
                rank: hits.len() as u32 + 1,
                score,
                point: parse::point(point)?,
            });
        }
        Ok(SearchResult {
            hits,
            below_min_score,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }

    async fn get_point(&self, collection: &str, id: &PointId) -> Result<Point> {
        let id = id.to_string();
        // Returns the payload and every vector without being asked.
        let path = ["collections", collection, "points", &id];
        let result = self.call(Method::GET, &path, None).await?;
        parse::point(&result)
    }

    async fn scroll(&self, request: ScrollRequest) -> Result<PointPage> {
        if request.limit == 0 {
            return Err(Error::Query("the page size must be at least 1".into()));
        }
        let started = Instant::now();
        let mut body = json!({
            "limit": request.limit,
            "with_payload": request.with_payload,
            "with_vector": request.with_vector,
        });
        if let Some(cursor) = &request.cursor {
            body["offset"] = parse::id_to_json(cursor);
        }
        if let Some(filter) = filter::filter(&request.filter)? {
            body["filter"] = filter;
        }
        let path = ["collections", &request.collection, "points", "scroll"];
        let result = self.call(Method::POST, &path, Some(&body)).await?;
        let points = result["points"]
            .as_array()
            .ok_or_else(|| parse::unexpected(&result.to_string()))?
            .iter()
            .map(parse::point)
            .collect::<Result<Vec<_>>>()?;
        let next = match &result["next_page_offset"] {
            Value::Null => None,
            id => Some(parse::id(id)?),
        };
        Ok(PointPage {
            points,
            next,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
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

/// An HTTP error reply as the error the interface shows. The code is the HTTP
/// status; the message is what Qdrant said.
fn status_error(status: u16, body: &str) -> Error {
    // API errors come as `{"status": {"error": "..."}}`; a refused key and
    // errors from anything in front of Qdrant come as plain text.
    let said = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["status"]["error"].as_str().map(str::to_owned))
        .unwrap_or_else(|| body.trim().chars().take(300).collect());
    let message = if said.is_empty() {
        format!("the server answered with HTTP status {status}")
    } else {
        said
    };
    Error::Database {
        code: Some(status.to_string()),
        message,
        position: None,
        line: None,
        hint: None,
        detail: None,
    }
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
            Error::Connect(format!("the server did not accept the API key: {message}"))
        }
        Error::Database {
            code: Some(code),
            message,
            ..
        } => Error::Connect(format!("HTTP status {code}: {message}")),
        Error::Internal(message) => Error::Connect(format!(
            "this does not look like a Qdrant server: {message}"
        )),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(authority("db.internal", Some(6333)), "db.internal:6333");
        assert_eq!(authority("::1", Some(6333)), "[::1]:6333");
        assert_eq!(authority("db.internal", None), "db.internal");
    }

    #[test]
    fn error_replies() {
        let Error::Database { code, message, .. } = status_error(
            404,
            r#"{"status":{"error":"Not found: Collection `nope` doesn't exist!"},"time":0.0}"#,
        ) else {
            panic!("expected a database error");
        };
        assert_eq!(code.as_deref(), Some("404"));
        assert_eq!(message, "Not found: Collection `nope` doesn't exist!");

        let refused = status_error(401, "Must provide an API key");
        assert!(matches!(
            connect_error(refused),
            Error::Connect(m) if m.contains("API key")
        ));
        assert!(matches!(
            status_error(502, ""),
            Error::Database { message, .. } if message.contains("502")
        ));
    }
}
