//! Neo4j driver.
//!
//! Talks to the server's HTTP Query API (`POST /db/{database}/query/v2`,
//! Neo4j 5.19 and later) rather than Bolt: it needs no protocol library, and
//! its typed JSON gives every node and relationship with its element id.
//!
//! HTTP has no session. Every statement is its own request and its own
//! transaction, so there is nothing to keep open and nothing to roll back.

mod convert;
mod split;
mod tls;

use std::net::IpAddr;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dbclient_core::graph::{
    CypherRequest, CypherResult, Direction, GraphCounters, GraphData, GraphDatabase, GraphStore,
    LabelInfo, NeighbourRequest, Neighbours, RelationshipGroup,
};
use dbclient_core::{
    ApplyResult, Capabilities, ChangeSet, Column, Connection, ConnectionConfig, DataModel,
    DbObject, Driver, DriverInfo, Error, Namespace, ObjectDescription, ObjectKind, QueryRequest,
    QueryResult, Result, SqlDialect, TlsMode,
};
use rustls::pki_types::ServerName;
use serde_json::{Map, Value as Json, json};

use convert::Collector;
use split::Piece;

/// Counting is one sub-query per label; this many go into one request.
const COUNTS_PER_REQUEST: usize = 100;

pub struct Neo4jDriver;

#[async_trait]
impl Driver for Neo4jDriver {
    fn info(&self) -> DriverInfo {
        DriverInfo {
            id: "neo4j",
            name: "Neo4j",
            model: DataModel::Graph,
            capabilities: Capabilities {
                tls: true,
                ..Capabilities::default()
            },
            // The HTTP port, not Bolt's 7687. With TLS the server listens on 7473.
            default_port: Some(7474),
            desktop_only: false,
        }
    }

    async fn connect(&self, config: &ConnectionConfig) -> Result<Box<dyn Connection>> {
        let mut connection = Neo4jConnection::new(config)?;
        connection.discover().await?;
        // The discovery page needs no login, so this is what checks the
        // credentials and that the database exists.
        connection.ping().await.map_err(|e| match e {
            Error::ConnectionLost(message) => Error::Connect(message),
            Error::Database {
                code: Some(code),
                message,
                ..
            } => Error::Connect(format!("{message} ({code})")),
            other => Error::Connect(other.to_string()),
        })?;
        Ok(Box::new(connection))
    }
}

struct Neo4jConnection {
    http: reqwest::Client,
    /// `scheme://host:port` the requests go to.
    base: String,
    /// Sent as `Host` when the URL names a tunnel's address instead of the server.
    host_header: Option<String>,
    user: Option<String>,
    password: Option<String>,
    database: String,
    /// From the discovery page, e.g. `5.26.0 community`.
    version: Option<String>,
}

/// One statement's answer as the server sent it.
struct Answer {
    fields: Vec<String>,
    /// Rows of typed values, see [`convert`].
    rows: Vec<Vec<Json>>,
    counters: Option<GraphCounters>,
}

impl Neo4jConnection {
    fn new(config: &ConnectionConfig) -> Result<Self> {
        let host = config
            .host
            .as_deref()
            .filter(|h| !h.is_empty())
            .unwrap_or("localhost");
        let encrypted = config.tls.mode != TlsMode::Off;
        let port = config.port.unwrap_or(if encrypted { 7473 } else { 7474 });

        let mut builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            // A database on the local network or behind a tunnel is not
            // reached through the system's web proxy.
            .no_proxy()
            // The Host header is set by hand for tunnels, which HTTP/2 does not allow.
            .http1_only();

        // Through a tunnel the socket goes to the tunnel's local end, while
        // the server must still see its own name.
        let mut url_host = host.to_owned();
        let mut url_port = port;
        let mut host_header = None;
        let mut check_as = None;
        if let Some(route) = config.route {
            url_port = route.port();
            host_header = Some(format!("{}:{port}", bracketed(host)));
            if host.parse::<IpAddr>().is_ok() {
                // An address in the URL is connected to as it is, so the URL
                // has to carry the tunnel's address; the certificate is then
                // checked against the real one.
                url_host = route.ip().to_string();
                check_as = ServerName::try_from(host.to_owned()).ok();
            } else {
                // A name can be pointed at the tunnel instead; only the
                // address is taken from here, the port comes from the URL.
                builder = builder.resolve(host, route);
            }
        }

        // Handed over ready-made also when TLS is off: reqwest would otherwise
        // set up its own and needs a process-wide crypto provider for that.
        let http = builder
            .tls_backend_preconfigured(tls::client_config(&config.tls, check_as)?)
            .build()
            .map_err(|e| Error::Internal(format!("HTTP client: {}", describe(&e))))?;

        Ok(Neo4jConnection {
            http,
            base: format!(
                "{}://{}:{url_port}",
                if encrypted { "https" } else { "http" },
                bracketed(&url_host)
            ),
            host_header,
            user: config.user.clone().filter(|u| !u.is_empty()),
            password: config.password.clone(),
            database: config
                .database
                .clone()
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| "neo4j".into()),
            version: None,
        })
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut request = self.http.request(method, format!("{}{path}", self.base));
        if let Some(host) = &self.host_header {
            request = request.header(reqwest::header::HOST, host);
        }
        if let Some(user) = &self.user {
            request = request.basic_auth(user, self.password.as_deref());
        }
        request
    }

    /// Reads the discovery page: it tells the version and whether the server
    /// has the Query API at all.
    async fn discover(&mut self) -> Result<()> {
        let wrong_port =
            "is this the HTTP port of a Neo4j server (7474 by default, not Bolt's 7687)?";
        let response = self
            .request(reqwest::Method::GET, "/")
            .header(reqwest::header::ACCEPT, "application/json")
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| Error::Connect(describe(&e)))?;
        let status = response.status();
        let page: Json = response.json().await.map_err(|_| {
            Error::Connect(format!("unexpected answer (HTTP {status}); {wrong_port}"))
        })?;
        let Some(version) = page.get("neo4j_version").and_then(Json::as_str) else {
            if let Some(error) = server_error(&page, None) {
                return Err(Error::Connect(error.to_string()));
            }
            return Err(Error::Connect(format!(
                "unexpected answer (HTTP {status}); {wrong_port}"
            )));
        };
        if page.get("query").is_none() {
            return Err(Error::Connect(format!(
                "Neo4j {version} has no HTTP Query API; version 5.19 or later is needed"
            )));
        }
        self.version = Some(match page.get("neo4j_edition").and_then(Json::as_str) {
            Some(edition) => format!("{version} {edition}"),
            None => version.to_owned(),
        });
        Ok(())
    }

    /// Runs one statement. `piece` says where it sits in the user's text, to
    /// place error positions.
    async fn query(
        &self,
        database: Option<&str>,
        statement: &str,
        parameters: &Map<String, Json>,
        piece: Option<&Piece<'_>>,
        timeout: Option<Duration>,
    ) -> Result<Answer> {
        let database = database.filter(|d| !d.is_empty()).unwrap_or(&self.database);
        let mut request = self
            .request(
                reqwest::Method::POST,
                &format!("/db/{}/query/v2", path_segment(database)),
            )
            // Typed JSON in the answer; plain JSON is still accepted for parameters.
            .header(reqwest::header::ACCEPT, "application/vnd.neo4j.query")
            .json(&json!({
                "statement": statement,
                "parameters": parameters,
                "includeCounters": true,
            }));
        if let Some(timeout) = timeout {
            request = request.timeout(timeout);
        }
        // Nothing reached the server or came back: for the caller that is a
        // lost connection, even though HTTP opens a new one next time.
        let lost = |e: reqwest::Error| Error::ConnectionLost(describe(&e));
        let response = request.send().await.map_err(lost)?;
        let status = response.status();
        let body = response.bytes().await.map_err(lost)?;
        let mut body: Json = serde_json::from_slice(&body).map_err(|_| {
            let start: String = String::from_utf8_lossy(&body).chars().take(200).collect();
            Error::Query(format!("unexpected answer (HTTP {status}): {start}"))
        })?;

        // An error can arrive with a success status and partial data, when
        // the statement fails after the first rows were produced.
        if let Some(error) = server_error(&body, piece) {
            return Err(error);
        }
        if !status.is_success() {
            return Err(Error::Query(format!("unexpected answer (HTTP {status})")));
        }

        let fields = body["data"]["fields"]
            .as_array()
            .map(|fields| {
                fields
                    .iter()
                    .map(|f| f.as_str().unwrap_or_default().to_owned())
                    .collect()
            })
            .unwrap_or_default();
        let rows = match body["data"]["values"].take() {
            Json::Array(rows) => rows
                .into_iter()
                .map(|row| match row {
                    Json::Array(cells) => cells,
                    other => vec![other],
                })
                .collect(),
            _ => Vec::new(),
        };
        Ok(Answer {
            fields,
            rows,
            counters: counters(&body["counters"]),
        })
    }

    /// A statement the driver itself wrote.
    async fn internal(
        &self,
        database: Option<&str>,
        statement: &str,
        parameters: Json,
    ) -> Result<Answer> {
        let parameters = match parameters {
            Json::Object(map) => map,
            _ => Map::new(),
        };
        self.query(database, statement, &parameters, None, None)
            .await
    }

    /// Turns one answer into the table and graph forms.
    async fn graph_result(
        &self,
        request: &CypherRequest,
        answer: Answer,
        started: Instant,
    ) -> Result<CypherResult> {
        let database = request.database.as_deref();
        let kept = request.max_rows.unwrap_or(usize::MAX);
        let truncated = answer.rows.len() > kept;

        let mut found = Collector::new(request.node_limit);
        let mut rows = Vec::new();
        for row in answer.rows.iter().take(kept) {
            for value in row {
                found.visit(value);
            }
            rows.push(row.iter().map(convert::cell).collect());
        }

        // A relationship returned on its own cannot be drawn without its end
        // nodes, so those are fetched, as far as the node limit leaves room.
        let mut missing: Vec<&str> = Vec::new();
        for relationship in &found.relationships {
            for end in [relationship.start.as_str(), relationship.end.as_str()] {
                if !found.has_node(end) && !missing.contains(&end) {
                    missing.push(end);
                }
            }
        }
        let mut node_count = found.nodes_seen() + missing.len();
        let fetch = &missing[..missing.len().min(found.room())];
        let mut nodes = Vec::new();
        if !fetch.is_empty() {
            let ends = self
                .internal(
                    database,
                    "MATCH (n) WHERE elementId(n) IN $ids RETURN n",
                    json!({ "ids": fetch }),
                )
                .await?;
            nodes.extend(
                ends.rows
                    .iter()
                    .filter_map(|row| convert::node(row.first()?)),
            );
            if fetch.len() == missing.len() {
                // All were asked for; those not returned no longer exist
                // (a statement can return a relationship it has just deleted).
                node_count = found.nodes_seen() + nodes.len();
            }
        }
        let relationship_count = found.relationships.len();
        let Collector {
            nodes: mut graph_nodes,
            mut relationships,
            ..
        } = found;
        graph_nodes.append(&mut nodes);

        let drawn: std::collections::HashSet<&str> =
            graph_nodes.iter().map(|n| n.id.as_str()).collect();
        relationships
            .retain(|r| drawn.contains(r.start.as_str()) && drawn.contains(r.end.as_str()));

        Ok(CypherResult {
            columns: answer.fields,
            rows,
            truncated,
            node_count: node_count as u64,
            relationship_count: relationship_count as u64,
            nodes_omitted: (node_count - graph_nodes.len()) as u64,
            relationships_omitted: (relationship_count - relationships.len()) as u64,
            graph: GraphData {
                nodes: graph_nodes,
                relationships,
            },
            counters: answer.counters,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }

    /// Lists names with `list`, then counts each with the pattern `count`
    /// builds for it.
    async fn names_with_counts(
        &self,
        database: Option<&str>,
        list: &str,
        count: impl Fn(&str) -> String,
    ) -> Result<Vec<LabelInfo>> {
        let answer = self.internal(database, list, Json::Null).await?;
        let mut names: Vec<LabelInfo> = answer
            .rows
            .iter()
            .filter_map(|row| convert::string(row.first()?))
            .map(|name| LabelInfo { name, count: None })
            .collect();
        names.sort_by(|a, b| a.name.cmp(&b.name));

        // A count of one label or one type with no other condition is read
        // from the store's own statistics, not by visiting the data. One
        // request asks for many of them.
        for chunk in names.chunks_mut(COUNTS_PER_REQUEST) {
            let statement = chunk
                .iter()
                .enumerate()
                .map(|(i, info)| format!("{} RETURN {i} AS i, count(x) AS c", count(&info.name)))
                .collect::<Vec<_>>()
                .join(" UNION ALL ");
            // Without the counts the list is still useful.
            let Ok(counted) = self.internal(database, &statement, Json::Null).await else {
                continue;
            };
            for row in &counted.rows {
                let (Some(i), Some(c)) = (
                    row.first().and_then(convert::count),
                    row.get(1).and_then(convert::count),
                ) else {
                    continue;
                };
                if let Some(info) = chunk.get_mut(i as usize) {
                    info.count = Some(c);
                }
            }
        }
        Ok(names)
    }
}

#[async_trait]
impl Connection for Neo4jConnection {
    async fn ping(&self) -> Result<()> {
        self.query(
            None,
            "RETURN 1",
            &Map::new(),
            None,
            Some(Duration::from_secs(15)),
        )
        .await
        .map(|_| ())
    }

    async fn execute_all(&self, request: QueryRequest) -> Result<Vec<QueryResult>> {
        let pieces = split::statements(&request.statement);
        if pieces.is_empty() {
            return Err(Error::Query("no statement to run".into()));
        }
        let mut results = Vec::new();
        for piece in &pieces {
            let started = Instant::now();
            let answer = self
                .query(None, piece.text, &Map::new(), Some(piece), None)
                .await?;
            // The whole answer has already been received; the limit only
            // bounds what is handed on.
            let kept = request.max_rows.unwrap_or(usize::MAX);
            let returns_rows = !answer.fields.is_empty();
            results.push(QueryResult {
                columns: answer
                    .fields
                    .into_iter()
                    .map(|name| Column {
                        name,
                        type_name: None,
                    })
                    .collect(),
                truncated: answer.rows.len() > kept,
                rows: answer
                    .rows
                    .iter()
                    .take(kept)
                    .map(|row| row.iter().map(convert::grid_value).collect())
                    .collect(),
                // Cypher has no single "rows affected"; the sum of what was
                // created, deleted and set is the closest to it.
                rows_affected: match answer.counters {
                    Some(counters) if !returns_rows => Some(counters.total()),
                    _ => None,
                },
                elapsed_ms: started.elapsed().as_millis() as u64,
            });
        }
        Ok(results)
    }

    async fn server_version(&self) -> Result<Option<String>> {
        Ok(self.version.clone())
    }

    async fn current_namespace(&self) -> Result<Option<String>> {
        Ok(Some(self.database.clone()))
    }

    async fn list_namespaces(&self) -> Result<Vec<Namespace>> {
        Ok(self
            .databases()
            .await?
            .into_iter()
            // The system database holds users and settings, not a graph to browse.
            .filter(|db| !db.is_system)
            .map(|db| Namespace { name: db.name })
            .collect())
    }

    async fn list_objects(&self, namespace: Option<&str>) -> Result<Vec<DbObject>> {
        Ok(self
            .labels(namespace)
            .await?
            .into_iter()
            .map(|label| DbObject {
                name: label.name,
                kind: ObjectKind::Collection,
                row_estimate: label.count,
            })
            .collect())
    }

    async fn describe_object(
        &self,
        _namespace: Option<&str>,
        _name: &str,
    ) -> Result<ObjectDescription> {
        // Nodes of one label need not share any properties: there are no columns.
        Err(Error::Unsupported("describing a label as a table"))
    }

    fn dialect(&self) -> SqlDialect {
        // Not used: Cypher is not SQL, and nothing here builds SQL text.
        SqlDialect::Postgres
    }

    async fn in_transaction(&self) -> Result<bool> {
        Ok(false)
    }

    async fn apply_changes(&self, _changes: &ChangeSet) -> Result<ApplyResult> {
        Err(Error::Unsupported("editing rows"))
    }

    fn quote_ident(&self, name: &str) -> String {
        backticked(name)
    }

    fn graph(&self) -> Option<&dyn GraphStore> {
        Some(self)
    }

    async fn close(&self) -> Result<()> {
        // No session is held on the server; idle sockets close with the client.
        Ok(())
    }
}

#[async_trait]
impl GraphStore for Neo4jConnection {
    async fn run(&self, request: CypherRequest) -> Result<Vec<CypherResult>> {
        let pieces = split::statements(&request.statement);
        if pieces.is_empty() {
            return Err(Error::Query("no statement to run".into()));
        }
        let mut results = Vec::new();
        for piece in &pieces {
            let started = Instant::now();
            let answer = self
                .query(
                    request.database.as_deref(),
                    piece.text,
                    &request.parameters,
                    Some(piece),
                    None,
                )
                .await?;
            results.push(self.graph_result(&request, answer, started).await?);
        }
        Ok(results)
    }

    async fn databases(&self) -> Result<Vec<GraphDatabase>> {
        let answer = self
            .internal(
                None,
                "SHOW DATABASES YIELD name, currentStatus, default, type \
                 RETURN name, currentStatus, default, type ORDER BY name",
                Json::Null,
            )
            .await?;
        let mut databases: Vec<GraphDatabase> = Vec::new();
        for row in &answer.rows {
            let Some(name) = row.first().and_then(convert::string) else {
                continue;
            };
            // A cluster lists a database once per server that hosts it.
            if databases.iter().any(|db| db.name == name) {
                continue;
            }
            databases.push(GraphDatabase {
                is_system: row.get(3).and_then(convert::string).as_deref() == Some("system"),
                is_default: row.get(2).and_then(convert::boolean).unwrap_or(false),
                status: row.get(1).and_then(convert::string),
                name,
            });
        }
        Ok(databases)
    }

    async fn labels(&self, database: Option<&str>) -> Result<Vec<LabelInfo>> {
        self.names_with_counts(
            database,
            "CALL db.labels() YIELD label RETURN label",
            |label| format!("MATCH (x:{})", backticked(label)),
        )
        .await
    }

    async fn relationship_types(&self, database: Option<&str>) -> Result<Vec<LabelInfo>> {
        self.names_with_counts(
            database,
            "CALL db.relationshipTypes() YIELD relationshipType RETURN relationshipType",
            |rel_type| format!("MATCH ()-[x:{}]->()", backticked(rel_type)),
        )
        .await
    }

    async fn neighbours(&self, request: NeighbourRequest) -> Result<Neighbours> {
        let database = request.database.as_deref();
        let rel_type = request.rel_type.as_deref().filter(|t| !t.is_empty());

        // First the node's relationships by type and direction. A
        // relationship from the node to itself both starts and ends at it.
        let counted = self
            .internal(
                database,
                "MATCH (n) WHERE elementId(n) = $id \
                 OPTIONAL MATCH (n)-[r]-() \
                 RETURN type(r), startNode(r) = n, endNode(r) = n, count(r)",
                json!({ "id": request.node_id }),
            )
            .await?;
        if counted.rows.is_empty() {
            return Err(Error::Query(format!("no node with id {}", request.node_id)));
        }
        let mut groups: Vec<RelationshipGroup> = Vec::new();
        let mut total = 0;
        for row in &counted.rows {
            // A node without relationships gives one row of nulls.
            let Some(name) = row.first().and_then(convert::string) else {
                continue;
            };
            let outgoing = row.get(1).and_then(convert::boolean).unwrap_or(false);
            let incoming = row.get(2).and_then(convert::boolean).unwrap_or(false);
            let count = row.get(3).and_then(convert::count).unwrap_or(0);

            let wanted = match request.direction {
                Direction::Both => true,
                Direction::Outgoing => outgoing,
                Direction::Incoming => incoming,
            };
            if wanted && rel_type.is_none_or(|t| t == name) {
                total += count;
            }
            match groups
                .iter_mut()
                .find(|g| g.rel_type == name && g.outgoing == outgoing)
            {
                Some(group) => group.count += count,
                None => groups.push(RelationshipGroup {
                    rel_type: name,
                    outgoing,
                    count,
                }),
            }
        }
        groups.sort_by(|a, b| (&a.rel_type, !a.outgoing).cmp(&(&b.rel_type, !b.outgoing)));

        let pattern = match request.direction {
            Direction::Both => "(n)-[r]-(m)",
            Direction::Outgoing => "(n)-[r]->(m)",
            Direction::Incoming => "(n)<-[r]-(m)",
        };
        let statement = format!(
            "MATCH (n) WHERE elementId(n) = $id \
             MATCH {pattern} WHERE $type IS NULL OR type(r) = $type \
             RETURN r, m LIMIT $limit"
        );
        let answer = self
            .internal(
                database,
                &statement,
                json!({ "id": request.node_id, "type": rel_type, "limit": request.limit }),
            )
            .await?;
        let mut found = Collector::new(None);
        for value in answer.rows.iter().flatten() {
            found.visit(value);
        }
        Ok(Neighbours {
            nodes: found.nodes,
            relationships: found.relationships,
            total,
            groups,
        })
    }
}

/// The first error in an answer, if it reports any.
fn server_error(body: &Json, piece: Option<&Piece<'_>>) -> Option<Error> {
    let error = body.get("errors")?.as_array()?.first()?;
    let code = error.get("code").and_then(Json::as_str).map(str::to_owned);
    let text = error
        .get("message")
        .and_then(Json::as_str)
        .unwrap_or("the server reported an error without a message");
    let (message, place) = split_position(text);
    Some(Error::Database {
        code,
        message,
        // The server counts from the start of the statement it was sent,
        // which may be a later piece of the text.
        position: place.map(|(_, offset)| offset + 1 + piece.map_or(0, |p| p.char_offset)),
        line: place.map(|(line, _)| line + piece.map_or(0, |p| p.line_offset)),
        hint: None,
        detail: None,
    })
}

/// Syntax errors end with ` (line 1, column 11 (offset: 10))` and a drawing
/// of the place. This takes that off and returns the line and offset; the
/// interface marks the place itself.
fn split_position(message: &str) -> (String, Option<(u32, u32)>) {
    let place = message.rfind(" (line ").and_then(|at| {
        let (line, rest) = message[at + 7..].split_once(", column ")?;
        let (_, rest) = rest.split_once(" (offset: ")?;
        let (offset, _) = rest.split_once("))")?;
        let line: u32 = line.parse().ok()?;
        let offset: u32 = offset.parse().ok()?;
        Some((at, line, offset))
    });
    match place {
        // Line 0 is what the server says when it has no real position.
        Some((at, 0, _)) => (message[..at].to_owned(), None),
        Some((at, line, offset)) => (message[..at].to_owned(), Some((line, offset))),
        None => (message.to_owned(), None),
    }
}

fn counters(value: &Json) -> Option<GraphCounters> {
    let value = value.as_object()?;
    let get = |name: &str| value.get(name).and_then(Json::as_u64).unwrap_or(0);
    Some(GraphCounters {
        nodes_created: get("nodesCreated"),
        nodes_deleted: get("nodesDeleted"),
        relationships_created: get("relationshipsCreated"),
        relationships_deleted: get("relationshipsDeleted"),
        properties_set: get("propertiesSet"),
        labels_added: get("labelsAdded"),
        labels_removed: get("labelsRemoved"),
        schema_changes: get("indexesAdded")
            + get("indexesRemoved")
            + get("constraintsAdded")
            + get("constraintsRemoved")
            + get("systemUpdates"),
    })
}

/// Quotes a label, type or property name the way Cypher does.
fn backticked(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

/// An IPv6 address needs brackets where a port follows.
fn bracketed(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}

/// Percent-encodes a database name for the URL path.
fn path_segment(name: &str) -> String {
    let mut encoded = String::new();
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// The error with its causes, without the URL: the message is shown to the user.
fn describe(e: &reqwest::Error) -> String {
    let mut text = if e.is_timeout() {
        "the server did not respond in time".to_owned()
    } else if e.is_connect() {
        "could not connect to the server".to_owned()
    } else {
        "the request failed".to_owned()
    };
    let mut source = std::error::Error::source(e);
    while let Some(inner) = source {
        text.push_str(&format!(": {inner}"));
        source = inner.source();
    }
    text
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use dbclient_core::TlsConfig;

    #[test]
    fn takes_the_position_off_a_syntax_error() {
        let (message, place) = split_position(
            "Invalid input 'RETRUN': expected 'RETURN' (line 2, column 2 (offset: 11))\n\" RETRUN n\"\n  ^",
        );
        assert_eq!(message, "Invalid input 'RETRUN': expected 'RETURN'");
        assert_eq!(place, Some((2, 11)));

        assert_eq!(split_position("/ by zero"), ("/ by zero".to_owned(), None));
        let (message, place) =
            split_position("Expected exactly one statement (line 0, column 0 (offset: 0))\n\"x\"");
        assert_eq!(message, "Expected exactly one statement");
        assert_eq!(place, None);
    }

    #[test]
    fn error_positions_are_moved_into_the_whole_text() {
        let body = json!({"errors": [{
            "code": "Neo.ClientError.Statement.SyntaxError",
            "message": "Invalid input (line 1, column 2 (offset: 1))\n\"x\"",
        }]});
        let piece = Piece {
            text: "",
            char_offset: 10,
            line_offset: 3,
        };
        match server_error(&body, Some(&piece)) {
            Some(Error::Database {
                code,
                message,
                position,
                line,
                ..
            }) => {
                assert_eq!(
                    code.as_deref(),
                    Some("Neo.ClientError.Statement.SyntaxError")
                );
                assert_eq!(message, "Invalid input");
                assert_eq!(position, Some(12));
                assert_eq!(line, Some(4));
            }
            other => panic!("{other:?}"),
        }
        assert!(server_error(&json!({"errors": []}), None).is_none());
        assert!(server_error(&json!({"data": {}}), None).is_none());
    }

    #[test]
    fn quotes_names_and_encodes_paths() {
        assert_eq!(backticked("Person"), "`Person`");
        assert_eq!(backticked("odd`name"), "`odd``name`");
        assert_eq!(path_segment("shop-graph.v2"), "shop-graph.v2");
        assert_eq!(path_segment("a/b c"), "a%2Fb%20c");
        assert_eq!(bracketed("::1"), "[::1]");
        assert_eq!(bracketed("db.internal"), "db.internal");
    }

    fn config(host: &str, mode: TlsMode) -> ConnectionConfig {
        ConnectionConfig {
            driver: "neo4j".into(),
            host: Some(host.into()),
            tls: TlsConfig {
                mode,
                ca_file: None,
            },
            ..ConnectionConfig::default()
        }
    }

    #[test]
    fn builds_the_address_from_the_settings() {
        let plain = Neo4jConnection::new(&config("db.internal", TlsMode::Off)).unwrap();
        assert_eq!(plain.base, "http://db.internal:7474");
        assert_eq!(plain.host_header, None);
        assert_eq!(plain.database, "neo4j");

        // The TLS setup is handed to reqwest ready-made; building the client
        // fails if reqwest does not recognize it.
        let verified = Neo4jConnection::new(&config("db.internal", TlsMode::Verify)).unwrap();
        assert_eq!(verified.base, "https://db.internal:7473");
        Neo4jConnection::new(&config("db.internal", TlsMode::Insecure)).unwrap();
    }

    #[test]
    fn a_route_changes_the_socket_but_not_the_host() {
        let route = "127.0.0.1:50001".parse().ok();

        let mut named = config("db.internal", TlsMode::Verify);
        named.route = route;
        let named = Neo4jConnection::new(&named).unwrap();
        // The name stays in the URL for the certificate check and is
        // resolved to the route; only the port differs.
        assert_eq!(named.base, "https://db.internal:50001");
        assert_eq!(named.host_header.as_deref(), Some("db.internal:7473"));

        let mut numbered = config("10.0.0.5", TlsMode::Verify);
        numbered.port = Some(7000);
        numbered.route = route;
        let numbered = Neo4jConnection::new(&numbered).unwrap();
        assert_eq!(numbered.base, "https://127.0.0.1:50001");
        assert_eq!(numbered.host_header.as_deref(), Some("10.0.0.5:7000"));
    }
}
