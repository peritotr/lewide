//! Milvus driver.
//!
//! Talks to the RESTful API v2 (`POST /v2/vectordb/...`, port 19530, the same
//! port as gRPC) rather than gRPC: it needs no generated client, and the
//! replies are JSON. Every reply comes with HTTP status 200; success or failure
//! is the `code` inside it (0 for success).
//!
//! The driver only reads. A point is an entity: its primary key is the point
//! id, each vector field is a named vector, and the other fields (with the
//! dynamic ones kept in `$meta`) are the payload. Listing pages by the key
//! (`key > last`, ordered by the key), so it is not held to Milvus's limit of
//! 16,384 for `offset + limit`.

mod expr;

use std::error::Error as _;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dbclient_core::vector::{
    CollectionInfo, PayloadField, Point, PointId, PointPage, PointVector, ScoredPoint,
    ScrollRequest, SearchQuery, SearchRequest, SearchResult, VectorData, VectorKind, VectorSpec,
    VectorStore,
};
use dbclient_core::{
    ApplyResult, Capabilities, ChangeSet, Connection, ConnectionConfig, DataModel, DbObject,
    Driver, DriverInfo, Error, Namespace, ObjectDescription, ObjectKind, QueryRequest, QueryResult,
    Result, SqlDialect, TlsMode,
};
use futures_util::StreamExt;
use reqwest::Url;
use reqwest::header::{AUTHORIZATION, HOST, HeaderMap, HeaderValue};
use serde_json::{Map, Value, json};

use expr::{Field, Schema};

const DEFAULT_PORT: u16 = 19530;
const DEFAULT_DATABASE: &str = "default";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// For one whole request. A search over a large collection can take a while;
/// a server that says nothing for this long is not coming back.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
/// How many collections are asked for their row count at once when listing.
const INFO_REQUESTS_AT_ONCE: usize = 8;
/// Milvus's code for a request without valid credentials.
const NOT_AUTHENTICATED: i64 = 1800;

pub struct MilvusDriver;

#[async_trait]
impl Driver for MilvusDriver {
    fn info(&self) -> DriverInfo {
        DriverInfo {
            id: "milvus",
            name: "Milvus",
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
        let connection = MilvusConnection::new(config)?;
        // Needs valid credentials and an existing database, and proves this is the Milvus API.
        connection.list_collections().await.map_err(connect_error)?;
        Ok(Box::new(connection))
    }
}

struct MilvusConnection {
    http: reqwest::Client,
    /// Scheme and address, without a path.
    base: Url,
    /// Sent with every request: the REST API keeps no session to remember it.
    database: String,
}

/// What the driver reads from a collection's description.
struct Described {
    key: Field,
    /// Vector fields in schema order: name, Milvus type, dimensions.
    vectors: Vec<(String, String, Option<u64>)>,
    /// Every field that is neither the key nor a vector.
    scalars: Vec<Field>,
    dynamic: bool,
    /// Metric of each indexed vector field, as Milvus names it (`COSINE`, `L2`, `IP`...).
    metrics: Vec<(String, String)>,
    /// `LoadStateLoaded`, `LoadStateLoading`, `LoadStateNotLoad`.
    load: Option<String>,
}

impl Described {
    fn schema(&self) -> Schema<'_> {
        Schema {
            fields: &self.scalars,
            dynamic: self.dynamic,
        }
    }

    fn metric(&self, field: &str) -> Option<&str> {
        self.metrics
            .iter()
            .find(|(name, _)| name == field)
            .map(|(_, metric)| metric.as_str())
    }

    /// The fields to ask for: the payload, the vectors, or both. The key is always returned.
    fn output_fields(&self, payload: bool, vectors: bool) -> Vec<String> {
        let mut fields = Vec::new();
        if payload {
            fields.extend(self.scalars.iter().map(|f| f.name.clone()));
            if self.dynamic {
                fields.push("$meta".to_owned());
            }
        }
        if vectors {
            fields.extend(self.vectors.iter().map(|(name, _, _)| name.clone()));
        }
        fields
    }

    /// An entity of a reply as a point.
    fn point(&self, row: &Value, with_payload: bool) -> Result<Point> {
        let Value::Object(row) = row else {
            return Err(unexpected(&row.to_string()));
        };
        let id = point_id(row.get(&self.key.name).unwrap_or(&Value::Null))?;
        let mut vectors = Vec::new();
        let mut payload = Map::new();
        for (name, value) in row {
            if name == &self.key.name || name == "distance" {
                continue;
            }
            if let Some((_, kind, _)) = self.vectors.iter().find(|(n, _, _)| n == name) {
                vectors.push(PointVector {
                    name: Some(name.clone()),
                    data: vector_data(kind, value),
                });
            } else if name == "$meta" {
                // Older servers return the dynamic fields as one object.
                if let Value::Object(fields) = value {
                    for (key, value) in fields {
                        payload.insert(key.clone(), plain(value));
                    }
                }
            } else {
                payload.insert(name.clone(), plain(value));
            }
        }
        Ok(Point {
            id,
            payload: with_payload.then_some(Value::Object(payload)),
            vectors,
        })
    }

    /// The id as a literal of the key's type, for a filter expression.
    fn id_literal(&self, id: &PointId) -> Result<String> {
        let text = id.to_string();
        if self.key.kind == "Int64" {
            let number: i64 = text.parse().map_err(|_| {
                Error::Query(format!("{text} is not a valid id: the key is a number"))
            })?;
            Ok(number.to_string())
        } else {
            Ok(expr::string(&text))
        }
    }

    /// The id as JSON of the key's type, for a request body.
    fn id_json(&self, id: &PointId) -> Result<Value> {
        let literal = self.id_literal(id)?;
        if self.key.kind == "Int64" {
            serde_json::from_str(&literal).map_err(|e| Error::Internal(e.to_string()))
        } else {
            Ok(Value::String(id.to_string()))
        }
    }
}

impl MilvusConnection {
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
        // Milvus takes `user:password`; a cloud API key is sent alone.
        let user = config
            .user
            .as_deref()
            .map(str::trim)
            .filter(|u| !u.is_empty());
        let password = config.password.as_deref().filter(|p| !p.is_empty());
        let token = match (user, password) {
            (Some(user), password) => Some(format!("{user}:{}", password.unwrap_or_default())),
            (None, Some(key)) => Some(key.to_owned()),
            (None, None) => None,
        };
        if let Some(token) = token {
            let mut value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| {
                Error::InvalidConfig(
                    "the password contains characters a header cannot carry".into(),
                )
            })?;
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
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .unwrap_or(DEFAULT_DATABASE)
            .to_owned();
        Ok(Self {
            http,
            base,
            database,
        })
    }

    /// Sends one API request (`collections/list`, `entities/search`...) and
    /// returns the `data` of a successful reply.
    async fn call(&self, path: &str, mut body: Value) -> Result<Value> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| Error::Internal("the server address cannot carry a path".into()))?
            .extend(["v2", "vectordb"])
            .extend(path.split('/'));
        body["dbName"] = json!(self.database);
        let response = self
            .http
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(network_error)?;
        let status = response.status();
        let text = response.text().await.map_err(network_error)?;
        let Ok(mut reply) = serde_json::from_str::<Value>(&text) else {
            if !status.is_success() {
                return Err(api_error(status.as_u16() as i64, &text));
            }
            return Err(unexpected(&text));
        };
        match reply["code"].as_i64() {
            Some(0) => Ok(reply
                .get_mut("data")
                .map(Value::take)
                .unwrap_or(Value::Null)),
            Some(code) => Err(api_error(
                code,
                reply["message"].as_str().unwrap_or_default(),
            )),
            None => Err(unexpected(&text)),
        }
    }

    async fn describe(&self, collection: &str) -> Result<Described> {
        let data = self
            .call(
                "collections/describe",
                json!({ "collectionName": collection }),
            )
            .await?;
        let fields = data["fields"]
            .as_array()
            .ok_or_else(|| unexpected(&data.to_string()))?;
        let mut key = None;
        let mut vectors = Vec::new();
        let mut scalars = Vec::new();
        for field in fields {
            let name = field["name"].as_str().unwrap_or_default().to_owned();
            let kind = field["type"].as_str().unwrap_or_default().to_owned();
            if field["primaryKey"].as_bool() == Some(true) {
                key = Some(Field {
                    name,
                    kind,
                    nullable: false,
                });
            } else if kind.ends_with("Vector") {
                let dim = field["params"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|p| p["key"] == "dim")
                    .and_then(|p| p["value"].as_str()?.parse().ok());
                vectors.push((name, kind, dim));
            } else {
                scalars.push(Field {
                    name,
                    nullable: field["nullable"].as_bool() == Some(true),
                    kind,
                });
            }
        }
        let metrics = data["indexes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| {
                Some((
                    i["fieldName"].as_str()?.to_owned(),
                    i["metricType"].as_str()?.to_owned(),
                ))
            })
            .collect();
        Ok(Described {
            key: key.ok_or_else(|| unexpected(&data.to_string()))?,
            vectors,
            scalars,
            dynamic: data["enableDynamicField"].as_bool() == Some(true),
            metrics,
            load: data["load"].as_str().map(str::to_owned),
        })
    }

    async fn row_count(&self, collection: &str) -> Result<u64> {
        let data = self
            .call(
                "collections/get_stats",
                json!({ "collectionName": collection }),
            )
            .await?;
        data["rowCount"]
            .as_u64()
            .ok_or_else(|| unexpected(&data.to_string()))
    }
}

#[async_trait]
impl Connection for MilvusConnection {
    async fn ping(&self) -> Result<()> {
        self.call("collections/list", json!({})).await.map(|_| ())
    }

    async fn execute_all(&self, _request: QueryRequest) -> Result<Vec<QueryResult>> {
        Err(Error::Unsupported("statements"))
    }

    async fn server_version(&self) -> Result<Option<String>> {
        // The REST API does not say which version answers.
        Ok(None)
    }

    async fn current_namespace(&self) -> Result<Option<String>> {
        // The database is chosen in the connection form; collections are the
        // top level of the tree.
        Ok(None)
    }

    async fn list_namespaces(&self) -> Result<Vec<Namespace>> {
        Ok(Vec::new())
    }

    async fn list_objects(&self, _namespace: Option<&str>) -> Result<Vec<DbObject>> {
        let names = self.list_collections().await?;
        let objects = futures_util::stream::iter(names)
            .map(|name| async move {
                // A collection whose count cannot be read is still listed.
                let row_estimate = self.row_count(&name).await.ok();
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
        // The vector view reads `collection_info` instead.
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
        // Each request stands alone; the idle sockets close with the client.
        Ok(())
    }
}

#[async_trait]
impl VectorStore for MilvusConnection {
    async fn list_collections(&self) -> Result<Vec<String>> {
        let data = self.call("collections/list", json!({})).await?;
        let mut names: Vec<String> = data
            .as_array()
            .ok_or_else(|| unexpected(&data.to_string()))?
            .iter()
            .filter_map(|n| n.as_str().map(str::to_owned))
            .collect();
        names.sort();
        Ok(names)
    }

    async fn collection_info(&self, collection: &str) -> Result<CollectionInfo> {
        let described = self.describe(collection).await?;
        let points = self.row_count(collection).await.ok();

        let mut vectors: Vec<VectorSpec> = described
            .vectors
            .iter()
            .map(|(name, kind, dim)| {
                let sparse = kind == "SparseFloatVector";
                let distance = described.metric(name).map(str::to_owned);
                VectorSpec {
                    name: Some(name.clone()),
                    kind: if sparse {
                        VectorKind::Sparse
                    } else {
                        VectorKind::Dense
                    },
                    size: if sparse { None } else { *dim },
                    higher_is_better: distance.as_deref().is_none_or(higher_is_better),
                    distance,
                }
            })
            .collect();
        // Dense by name, then sparse by name, as the view lists them.
        vectors.sort_by(|a, b| {
            (a.kind == VectorKind::Sparse, &a.name).cmp(&(b.kind == VectorKind::Sparse, &b.name))
        });

        let payload_fields = described
            .scalars
            .iter()
            .map(|f| PayloadField {
                name: f.name.clone(),
                kind: f.kind.to_lowercase(),
                points: None,
            })
            .collect();
        // Milvus's load state in the view's health words: a collection that is
        // not loaded cannot be searched.
        let status = described.load.as_deref().map(|state| {
            match state {
                "LoadStateLoaded" => "green",
                "LoadStateLoading" => "yellow",
                "LoadStateNotLoad" => "grey",
                _ => "red",
            }
            .to_owned()
        });
        Ok(CollectionInfo {
            name: collection.to_owned(),
            status,
            points,
            vectors,
            payload_fields,
        })
    }

    async fn search(&self, request: SearchRequest) -> Result<SearchResult> {
        if request.limit == 0 {
            return Err(Error::Query("top K must be at least 1".into()));
        }
        let started = Instant::now();
        let described = self.describe(&request.collection).await?;
        let using = match &request.using {
            Some(using) => using.clone(),
            None => described
                .vectors
                .first()
                .map(|(name, _, _)| name.clone())
                .ok_or_else(|| Error::Query("the collection has no vector field".into()))?,
        };

        let mut filter = expr::filter(&request.filter, &described.schema())?;
        let vector = match &request.query {
            SearchQuery::Vector { values } => json!(values),
            SearchQuery::Point { id } => {
                let rows = self
                    .call(
                        "entities/get",
                        json!({
                            "collectionName": request.collection,
                            "id": [described.id_json(id)?],
                            "outputFields": [using],
                        }),
                    )
                    .await?;
                let vector = rows
                    .as_array()
                    .and_then(|rows| rows.first())
                    .and_then(|row| row.get(&using))
                    .cloned()
                    .ok_or_else(|| Error::Query(format!("there is no point with id {id}")))?;
                // The point itself would be the best hit; it is left out.
                let itself = format!("{} != {}", described.key.name, described.id_literal(id)?);
                filter = Some(match filter {
                    Some(filter) => format!("({filter}) and {itself}"),
                    None => itself,
                });
                vector
            }
        };

        let mut body = json!({
            "collectionName": request.collection,
            "data": [vector],
            "annsField": using,
            "limit": request.limit,
            "outputFields": described.output_fields(request.with_payload, request.with_vector),
        });
        if let Some(filter) = filter {
            body["filter"] = json!(filter);
        }
        let rows = self.call("entities/search", body).await?;
        let rows = rows
            .as_array()
            .ok_or_else(|| unexpected(&rows.to_string()))?;

        // The minimum score is applied here, so that the number of hits it
        // removed is known. Hits come best first, so the result is the same.
        let better = described.metric(&using).is_none_or(higher_is_better);
        let mut hits = Vec::new();
        let mut below_min_score = 0;
        for row in rows {
            let score = row["distance"]
                .as_f64()
                .ok_or_else(|| unexpected(&row.to_string()))?;
            let kept = match request.min_score {
                None => true,
                Some(min) if better => score >= min,
                // A distance: the limit is the farthest a hit may be.
                Some(min) => score <= min,
            };
            if !kept {
                below_min_score += 1;
                continue;
            }
            hits.push(ScoredPoint {
                rank: hits.len() as u32 + 1,
                score,
                point: described.point(row, request.with_payload)?,
            });
        }
        Ok(SearchResult {
            hits,
            below_min_score,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }

    async fn get_point(&self, collection: &str, id: &PointId) -> Result<Point> {
        let described = self.describe(collection).await?;
        let rows = self
            .call(
                "entities/get",
                json!({
                    "collectionName": collection,
                    "id": [described.id_json(id)?],
                    "outputFields": described.output_fields(true, true),
                }),
            )
            .await?;
        let row = rows
            .as_array()
            .and_then(|rows| rows.first())
            .ok_or_else(|| Error::Query(format!("there is no point with id {id}")))?;
        described.point(row, true)
    }

    async fn scroll(&self, request: ScrollRequest) -> Result<PointPage> {
        if request.limit == 0 {
            return Err(Error::Query("the page size must be at least 1".into()));
        }
        let started = Instant::now();
        let described = self.describe(&request.collection).await?;
        let key = &described.key.name;

        let mut parts = Vec::new();
        if let Some(filter) = expr::filter(&request.filter, &described.schema())? {
            parts.push(format!("({filter})"));
        }
        if let Some(cursor) = &request.cursor {
            parts.push(format!("{key} > {}", described.id_literal(cursor)?));
        }
        let mut fields = described.output_fields(request.with_payload, request.with_vector);
        if fields.is_empty() {
            fields.push(key.clone());
        }
        // One more than asked, to know whether another page follows.
        let rows = self
            .call(
                "entities/query",
                json!({
                    "collectionName": request.collection,
                    "filter": parts.join(" and "),
                    "limit": request.limit + 1,
                    "outputFields": fields,
                    "orderByFields": [format!("{key}:asc")],
                }),
            )
            .await?;
        let rows = rows
            .as_array()
            .ok_or_else(|| unexpected(&rows.to_string()))?;
        let mut points = rows
            .iter()
            .map(|row| described.point(row, request.with_payload))
            .collect::<Result<Vec<_>>>()?;
        let next = if points.len() > request.limit as usize {
            points.truncate(request.limit as usize);
            points.last().map(|p| p.id.clone())
        } else {
            None
        };
        Ok(PointPage {
            points,
            next,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }
}

/// Whether a higher score is a closer match for a Milvus metric. `L2` (squared
/// distance), `HAMMING` and the Jaccard metrics are distances.
fn higher_is_better(metric: &str) -> bool {
    !matches!(
        metric.to_ascii_uppercase().as_str(),
        "L2" | "HAMMING" | "JACCARD" | "MHJACCARD" | "SUBSTRUCTURE" | "SUPERSTRUCTURE"
    )
}

fn point_id(value: &Value) -> Result<PointId> {
    match value {
        Value::Number(number) => Ok(match number.as_u64() {
            Some(number) => PointId::Number(number),
            // A negative key: kept as text, and read back as a number for the request.
            None => PointId::Uuid(number.to_string()),
        }),
        Value::String(text) => Ok(PointId::Uuid(text.clone())),
        _ => Err(unexpected(&value.to_string())),
    }
}

fn vector_data(kind: &str, value: &Value) -> VectorData {
    let floats = |items: &Vec<Value>| -> Option<Vec<f32>> {
        items.iter().map(|v| v.as_f64().map(|f| f as f32)).collect()
    };
    match (kind, value) {
        ("SparseFloatVector", Value::Object(entries)) => {
            let mut pairs: Vec<(u32, f32)> = entries
                .iter()
                .filter_map(|(index, v)| Some((index.parse().ok()?, v.as_f64()? as f32)))
                .collect();
            pairs.sort_by_key(|(index, _)| *index);
            VectorData::Sparse {
                indices: pairs.iter().map(|(i, _)| *i).collect(),
                values: pairs.iter().map(|(_, v)| *v).collect(),
            }
        }
        (_, Value::Array(items)) if kind != "BinaryVector" => match floats(items) {
            Some(values) => VectorData::Dense { values },
            None => VectorData::Other { raw: value.clone() },
        },
        _ => VectorData::Other { raw: value.clone() },
    }
}

/// A field value as plain JSON. Array fields come wrapped in the server's
/// internal layout (`{"Data": {"StringData": {"data": [...]}}}`); the list is taken out.
fn plain(value: &Value) -> Value {
    if let Value::Object(outer) = value
        && outer.len() == 1
        && let Some(Value::Object(inner)) = outer.get("Data")
        && inner.len() == 1
        && let Some(items) = inner.values().next().and_then(|v| v.get("data"))
    {
        return items.clone();
    }
    value.clone()
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

/// A failed request as the error the interface shows: Milvus's code (or the
/// HTTP status, when something in front of Milvus answered) and its message.
fn api_error(code: i64, message: &str) -> Error {
    let message: String = message.trim().chars().take(500).collect();
    Error::Database {
        code: Some(code.to_string()),
        message: if message.is_empty() {
            format!("the server answered with code {code}")
        } else {
            message
        },
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
        } if code == NOT_AUTHENTICATED.to_string() || code == "401" || code == "403" => {
            Error::Connect(format!(
                "the server did not accept the user name and password: {message}"
            ))
        }
        Error::Database {
            code: Some(code),
            message,
            ..
        } => Error::Connect(format!("code {code}: {message}")),
        Error::Internal(message) => Error::Connect(format!(
            "this does not look like a Milvus server: {message}"
        )),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn array_fields_are_unwrapped() {
        let wrapped = json!({"Data": {"StringData": {"data": ["t1", "all"]}}});
        assert_eq!(plain(&wrapped), json!(["t1", "all"]));
        let object = json!({"Data": 1, "other": 2});
        assert_eq!(plain(&object), object);
        assert_eq!(plain(&json!("x")), json!("x"));
    }

    #[test]
    fn metrics() {
        assert!(higher_is_better("COSINE"));
        assert!(higher_is_better("IP"));
        assert!(higher_is_better("BM25"));
        assert!(!higher_is_better("L2"));
        assert!(!higher_is_better("hamming"));
    }

    #[test]
    fn vectors_and_ids() {
        assert_eq!(
            vector_data("FloatVector", &json!([1, 0.5])),
            VectorData::Dense {
                values: vec![1.0, 0.5]
            }
        );
        assert_eq!(
            vector_data("SparseFloatVector", &json!({"7": 0.5, "2": 1})),
            VectorData::Sparse {
                indices: vec![2, 7],
                values: vec![1.0, 0.5]
            }
        );
        assert!(matches!(
            vector_data("BinaryVector", &json!([1, 2])),
            VectorData::Other { .. }
        ));
        assert_eq!(point_id(&json!(42)).unwrap(), PointId::Number(42));
        assert_eq!(point_id(&json!(-3)).unwrap(), PointId::Uuid("-3".into()));
        assert_eq!(
            point_id(&json!("007")).unwrap(),
            PointId::Uuid("007".into())
        );
    }

    #[test]
    fn errors() {
        assert!(matches!(
            connect_error(api_error(NOT_AUTHENTICATED, "user hasn't authenticated")),
            Error::Connect(m) if m.contains("password")
        ));
        assert!(matches!(
            api_error(800, ""),
            Error::Database { message, .. } if message.contains("800")
        ));
        assert_eq!(authority("::1", Some(19530)), "[::1]:19530");
    }
}
