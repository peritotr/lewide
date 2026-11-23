//! MongoDB driver.
//!
//! The work is done through [`DocumentStore`]: collections are read with query
//! documents and changed one document at a time. The SQL-shaped methods of
//! [`Connection`] are filled in as far as they have a meaning here (databases
//! as namespaces, collections as objects); the rest report `Unsupported`.
//!
//! The connection talks to the one host it was given and does not go looking
//! for the other members of a replica set, unless the `replicaSet` parameter
//! names one. A desktop client is usually pointed at a single address, often
//! through a tunnel or a container port, where the addresses the members
//! advertise to each other cannot be reached.

mod route;
mod text;

use std::time::{Duration, Instant};

use async_trait::async_trait;
use dbclient_core::document::{
    self, CollectionInfo, CollectionKind, DatabaseInfo, DocumentChange, DocumentStore, FindPage,
    FindRequest, FoundDocument, MAX_PAGE, TextPart, WriteOutcome,
};
use dbclient_core::{
    ApplyResult, Capabilities, ChangeSet, Column, ColumnDescription, Connection, ConnectionConfig,
    DataModel, DbObject, Driver, DriverInfo, Error, Namespace, ObjectDescription, ObjectKind,
    QueryRequest, QueryResult, Result, SqlDialect, TablePage, TableRequest, TlsMode, Value,
};
use futures_util::{StreamExt, TryStreamExt, stream};
use mongodb::bson::{Bson, Document, RawDocumentBuf, doc};
use mongodb::error::{ErrorKind, WriteFailure};
use mongodb::options::{ClientOptions, Credential, ServerAddress, Socks5Proxy, Tls, TlsOptions};
use mongodb::results::CollectionType;
use mongodb::{Client, Collection};

const DEFAULT_PORT: u16 = 27017;
/// How long to wait for the server before giving up, on connect and on every
/// later operation that finds the server gone.
const WAIT: Duration = Duration::from_secs(10);
/// Documents read to guess a collection's fields in [`Connection::describe_object`].
const SAMPLE: i64 = 100;

pub struct MongoDriver;

#[async_trait]
impl Driver for MongoDriver {
    fn info(&self) -> DriverInfo {
        DriverInfo {
            id: "mongodb",
            name: "MongoDB",
            model: DataModel::Document,
            capabilities: Capabilities {
                tls: true,
                ..Capabilities::default()
            },
            default_port: Some(DEFAULT_PORT),
            desktop_only: false,
        }
    }

    async fn connect(&self, config: &ConnectionConfig) -> Result<Box<dyn Connection>> {
        let host = config
            .host
            .as_deref()
            .filter(|h| !h.is_empty())
            .unwrap_or("localhost");
        let database = config.database.clone().filter(|d| !d.is_empty());
        let param = |name: &str| config.params.get(name).filter(|v| !v.is_empty());

        let mut options = ClientOptions::builder()
            .hosts(vec![ServerAddress::Tcp {
                host: host.to_owned(),
                port: Some(config.port.unwrap_or(DEFAULT_PORT)),
            }])
            .build();
        options.app_name = Some("dbclient".into());
        options.connect_timeout = Some(WAIT);
        options.server_selection_timeout = Some(WAIT);
        options.default_database = database.clone();

        // See the module description. Through a route only the given host can
        // be reached at all, so discovery stays off there whatever was asked.
        let replica_set = param("replicaSet").filter(|_| config.route.is_none());
        options.direct_connection = Some(match param("directConnection") {
            Some(value) if config.route.is_none() => value == "true",
            _ => replica_set.is_none(),
        });
        options.repl_set_name = replica_set.cloned();

        if let Some(user) = config.user.as_deref().filter(|u| !u.is_empty()) {
            // The database a user is defined in. As in a MongoDB connection
            // string: `authSource` if given, else the database, else `admin`.
            let source = param("authSource")
                .cloned()
                .or_else(|| database.clone())
                .unwrap_or_else(|| "admin".into());
            let mut credential = Credential::builder()
                .username(user.to_owned())
                .source(source)
                .build();
            credential.password = config.password.clone();
            options.credential = Some(credential);
        }

        options.tls = Some(match config.tls.mode {
            TlsMode::Off => Tls::Disabled,
            mode => {
                let mut tls = TlsOptions::builder().build();
                tls.allow_invalid_certificates = Some(mode == TlsMode::Insecure);
                // With a CA file the driver trusts only that file, not the
                // public authorities as well.
                tls.ca_file_path = config
                    .tls
                    .ca_file
                    .as_deref()
                    .filter(|p| !p.is_empty())
                    .map(Into::into);
                Tls::Enabled(tls)
            }
        });

        let proxy = match config.route {
            Some(route) => {
                let proxy = route::start(route)
                    .await
                    .map_err(|e| Error::Connect(format!("could not open the route: {e}")))?;
                let mut socks = Socks5Proxy::builder().host("127.0.0.1").build();
                socks.port = Some(proxy.port);
                options.socks5_proxy = Some(socks);
                Some(proxy)
            }
            None => None,
        };

        let client = Client::with_options(options).map_err(connect_err)?;
        // Creating the client opens nothing. The first command connects and
        // signs in, so a wrong address or password is reported here.
        let build = client
            .database("admin")
            .run_command(doc! { "buildInfo": 1 })
            .await
            .map_err(connect_err)?;

        Ok(Box::new(MongoConnection {
            client,
            database,
            version: build.get_str("version").ok().map(str::to_owned),
            _proxy: proxy,
        }))
    }
}

struct MongoConnection {
    /// A pool of connections; it can be used from several tasks at once.
    client: Client,
    /// The database from the connection settings, if one was given.
    database: Option<String>,
    version: Option<String>,
    /// Kept alive for as long as the client may open connections through it.
    _proxy: Option<route::RouteProxy>,
}

impl MongoConnection {
    fn database_or_default<'a>(&'a self, namespace: Option<&'a str>) -> Result<&'a str> {
        namespace
            .filter(|n| !n.is_empty())
            .or(self.database.as_deref())
            .ok_or_else(|| Error::Query("no database is selected".into()))
    }

    fn collection(&self, database: &str, name: &str) -> Collection<RawDocumentBuf> {
        self.client.database(database).collection(name)
    }

    /// Document count, average size and total size from the collection's
    /// metadata. `None` for each when the server gives none (views) or refuses.
    async fn stats(&self, database: &str, name: &str) -> (Option<u64>, Option<u64>, Option<u64>) {
        let read = async {
            let mut cursor = self
                .client
                .database(database)
                .collection::<Document>(name)
                .aggregate([doc! { "$collStats": { "storageStats": {} } }])
                .await?;
            // One entry per shard on a sharded cluster.
            let (mut count, mut size) = (0u64, 0u64);
            let mut found = false;
            while let Some(entry) = cursor.try_next().await? {
                if let Ok(stats) = entry.get_document("storageStats") {
                    found = true;
                    count += stats.get("count").and_then(as_u64).unwrap_or(0);
                    size += stats.get("size").and_then(as_u64).unwrap_or(0);
                }
            }
            mongodb::error::Result::Ok(found.then_some((count, size)))
        };
        match read.await {
            Ok(Some((count, size))) => (Some(count), (count > 0).then(|| size / count), Some(size)),
            _ => (None, None, None),
        }
    }

    async fn info(&self, database: &str, name: String, kind: CollectionKind) -> CollectionInfo {
        let (document_count, avg_document_bytes, size_bytes) = if kind == CollectionKind::View {
            (None, None, None)
        } else {
            self.stats(database, &name).await
        };
        CollectionInfo {
            name,
            kind,
            document_count,
            avg_document_bytes,
            size_bytes,
        }
    }

    async fn collection_kinds(
        &self,
        database: &str,
        only: Option<&str>,
    ) -> Result<Vec<(String, CollectionKind)>> {
        let db = self.client.database(database);
        let mut listing = db.list_collections();
        if let Some(name) = only {
            listing = listing.filter(doc! { "name": name });
        }
        let specs: Vec<_> = listing
            .await
            .map_err(op_err)?
            .try_collect()
            .await
            .map_err(op_err)?;
        let mut kinds: Vec<_> = specs
            .into_iter()
            .map(|spec| {
                let kind = match spec.collection_type {
                    CollectionType::View => CollectionKind::View,
                    CollectionType::Timeseries => CollectionKind::Timeseries,
                    _ => CollectionKind::Collection,
                };
                (spec.name, kind)
            })
            // The server's own bookkeeping (view definitions, time-series
            // buckets), which users do not open.
            .filter(|(name, _)| !name.starts_with("system."))
            .collect();
        kinds.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(kinds)
    }
}

#[async_trait]
impl DocumentStore for MongoConnection {
    async fn list_databases(&self) -> Result<Vec<DatabaseInfo>> {
        let mut databases: Vec<_> = self
            .client
            .list_databases()
            .await
            .map_err(op_err)?
            .into_iter()
            .map(|d| DatabaseInfo {
                name: d.name,
                size_bytes: Some(d.size_on_disk),
            })
            .collect();
        databases.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(databases)
    }

    async fn list_collections(&self, database: &str) -> Result<Vec<CollectionInfo>> {
        let kinds = self.collection_kinds(database, None).await?;
        // One metadata read per collection, a few at a time, in listing order.
        Ok(stream::iter(kinds)
            .map(|(name, kind)| self.info(database, name, kind))
            .buffered(8)
            .collect()
            .await)
    }

    async fn collection_info(&self, database: &str, collection: &str) -> Result<CollectionInfo> {
        let (name, kind) = self
            .collection_kinds(database, Some(collection))
            .await?
            .pop()
            .ok_or_else(|| Error::Query(format!("no such collection: {database}.{collection}")))?;
        Ok(self.info(database, name, kind).await)
    }

    async fn find(&self, request: FindRequest) -> Result<FindPage> {
        let started = Instant::now();
        let filter = parse(TextPart::Filter, request.filter.as_deref())?;
        let projection = parse(TextPart::Projection, request.projection.as_deref())?;
        let sort = parse(TextPart::Sort, request.sort.as_deref())?;
        let limit = request.limit.clamp(1, MAX_PAGE) as usize;

        let collection = self.collection(&request.database, &request.collection);
        let mut find = collection
            .find(filter.unwrap_or_default())
            .skip(request.skip)
            // One document more than asked for tells whether another page exists.
            .limit(limit as i64 + 1);
        if let Some(projection) = projection {
            find = find.projection(projection);
        }
        if let Some(sort) = sort {
            find = find.sort(sort);
        }
        let mut cursor = find.await.map_err(op_err)?;

        let mut documents = Vec::new();
        let mut has_more = false;
        while let Some(raw) = cursor.try_next().await.map_err(op_err)? {
            if documents.len() == limit {
                has_more = true;
                break;
            }
            documents.push(FoundDocument {
                size_bytes: raw.as_bytes().len() as u64,
                document: to_json(&raw)?,
            });
        }
        Ok(FindPage {
            documents,
            skip: request.skip,
            has_more,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }

    async fn count(&self, database: &str, collection: &str, filter: Option<&str>) -> Result<u64> {
        let filter = parse(TextPart::Filter, filter)?;
        self.collection(database, collection)
            .count_documents(filter.unwrap_or_default())
            .await
            .map_err(op_err)
    }

    async fn write(
        &self,
        database: &str,
        collection: &str,
        change: DocumentChange,
    ) -> Result<WriteOutcome> {
        let collection = self
            .client
            .database(database)
            .collection::<Document>(collection);
        let by_id = |id: &serde_json::Value| -> Result<Document> {
            let id = text::value_from_json(id)
                .map_err(|e| Error::Query(format!("the _id is not valid: {e}")))?;
            Ok(doc! { "_id": id })
        };
        // A document to store may be `{}`, but not blank.
        let required = |part: TextPart, text: &str| {
            parse(part, Some(text))?.ok_or_else(|| {
                document::invalid_text(part, "a document is expected: { … }", Some(1))
            })
        };

        match change {
            DocumentChange::Insert { document } => {
                let document = required(TextPart::Document, &document)?;
                let result = collection.insert_one(document).await.map_err(op_err)?;
                Ok(WriteOutcome {
                    inserted_id: Some(result.inserted_id.into_canonical_extjson()),
                    ..WriteOutcome::default()
                })
            }
            DocumentChange::Replace { id, document } => {
                let document = required(TextPart::Document, &document)?;
                let result = collection
                    .replace_one(by_id(&id)?, document)
                    .await
                    .map_err(op_err)?;
                Ok(WriteOutcome {
                    matched: result.matched_count,
                    modified: result.modified_count,
                    ..WriteOutcome::default()
                })
            }
            DocumentChange::Update { id, update } => {
                let update = required(TextPart::Update, &update)?;
                let result = collection
                    .update_one(by_id(&id)?, update)
                    .await
                    .map_err(op_err)?;
                Ok(WriteOutcome {
                    matched: result.matched_count,
                    modified: result.modified_count,
                    ..WriteOutcome::default()
                })
            }
            DocumentChange::Delete { id } => {
                let result = collection.delete_one(by_id(&id)?).await.map_err(op_err)?;
                Ok(WriteOutcome {
                    deleted: result.deleted_count,
                    ..WriteOutcome::default()
                })
            }
        }
    }
}

#[async_trait]
impl Connection for MongoConnection {
    async fn ping(&self) -> Result<()> {
        self.client
            .database("admin")
            .run_command(doc! { "ping": 1 })
            .await
            .map(|_| ())
            .map_err(op_err)
    }

    /// Runs one database command written as a document, for example
    /// `{ listIndexes: "products" }` or `{ dbStats: 1 }`, on the connection's
    /// database (`admin` when none was set).
    ///
    /// The reply comes back as one column of JSON. A command that answers with
    /// a cursor (`find`, `aggregate`, `listIndexes`) gives one row per document
    /// of its first batch; later batches are not fetched, so such a result is
    /// marked truncated when the server has more. Any other reply is one row.
    async fn execute_all(&self, request: QueryRequest) -> Result<Vec<QueryResult>> {
        let started = Instant::now();
        let command = parse(TextPart::Command, Some(&request.statement))?
            .filter(|c| !c.is_empty())
            .ok_or_else(|| Error::Query("no command to run".into()))?;
        let reply = self
            .client
            .database(self.database.as_deref().unwrap_or("admin"))
            .run_command(command)
            .await
            .map_err(op_err)?;

        let json = |d: &Document| Value::Json(Bson::Document(d.clone()).into_canonical_extjson());
        let mut truncated = false;
        let mut rows: Vec<Vec<Value>> = match reply.get_document("cursor") {
            Ok(cursor) => {
                truncated = cursor.get("id").and_then(as_u64).is_some_and(|id| id != 0);
                cursor
                    .get_array("firstBatch")
                    .map(|batch| {
                        batch
                            .iter()
                            .filter_map(Bson::as_document)
                            .map(|d| vec![json(d)])
                            .collect()
                    })
                    .unwrap_or_default()
            }
            Err(_) => vec![vec![json(&reply)]],
        };
        if let Some(max) = request.max_rows.filter(|max| rows.len() > *max) {
            rows.truncate(max);
            truncated = true;
        }
        Ok(vec![QueryResult {
            columns: vec![Column {
                name: "document".into(),
                type_name: Some("object".into()),
            }],
            rows,
            rows_affected: None,
            truncated,
            elapsed_ms: started.elapsed().as_millis() as u64,
        }])
    }

    async fn server_version(&self) -> Result<Option<String>> {
        Ok(self.version.clone())
    }

    async fn current_namespace(&self) -> Result<Option<String>> {
        Ok(self.database.clone())
    }

    async fn list_namespaces(&self) -> Result<Vec<Namespace>> {
        Ok(DocumentStore::list_databases(self)
            .await?
            .into_iter()
            .map(|d| Namespace { name: d.name })
            .collect())
    }

    async fn list_objects(&self, namespace: Option<&str>) -> Result<Vec<DbObject>> {
        let database = self.database_or_default(namespace)?;
        Ok(DocumentStore::list_collections(self, database)
            .await?
            .into_iter()
            .map(|c| DbObject {
                name: c.name,
                kind: match c.kind {
                    CollectionKind::View => ObjectKind::View,
                    _ => ObjectKind::Collection,
                },
                row_estimate: c.document_count,
            })
            .collect())
    }

    /// A collection has no declared columns. This reads its first documents
    /// and lists the top-level fields found in them, in order of first
    /// appearance, with the types seen. Fields that only occur further down
    /// the collection are missing from the list.
    async fn describe_object(
        &self,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<ObjectDescription> {
        let database = self.database_or_default(namespace)?;
        let mut cursor = self
            .client
            .database(database)
            .collection::<Document>(name)
            .find(doc! {})
            .limit(SAMPLE)
            .await
            .map_err(op_err)?;

        // Field name, the types seen, and in how many documents it occurred.
        let mut fields: Vec<(String, Vec<&'static str>, usize)> = Vec::new();
        let mut sampled = 0;
        while let Some(document) = cursor.try_next().await.map_err(op_err)? {
            sampled += 1;
            for (key, value) in document {
                let kind = document::type_name(&value.into_canonical_extjson());
                match fields.iter_mut().find(|f| f.0 == key) {
                    Some(field) => {
                        field.2 += 1;
                        if !field.1.contains(&kind) {
                            field.1.push(kind);
                        }
                    }
                    None => fields.push((key, vec![kind], 1)),
                }
            }
        }
        Ok(ObjectDescription {
            name: name.to_owned(),
            columns: fields
                .into_iter()
                .map(|(name, kinds, seen)| ColumnDescription {
                    primary_key: name == "_id",
                    // "Nullable" here means: missing or null in some sampled document.
                    nullable: seen < sampled || kinds.contains(&"null"),
                    type_name: Some(kinds.join(" | ")),
                    name,
                    default_value: None,
                    comment: None,
                    enum_values: None,
                })
                .collect(),
        })
    }

    /// The collection's indexes from `listIndexes`, with their sizes from
    /// `$collStats` when the user may read them. A key part is the field,
    /// followed by its direction or kind when that is not ascending
    /// (`year desc`, `title text`, `location 2dsphere`).
    async fn list_indexes(
        &self,
        namespace: Option<&str>,
        table: &str,
    ) -> Result<Vec<dbclient_core::IndexDescription>> {
        let database = self.database_or_default(namespace)?;
        let collection = self.client.database(database).collection::<Document>(table);
        let models: Vec<_> = collection
            .list_indexes()
            .await
            .map_err(op_err)?
            .try_collect()
            .await
            .map_err(op_err)?;
        // Sizes are extra: a user without the right to read statistics still sees the list.
        let sizes = collection
            .aggregate([doc! { "$collStats": { "storageStats": {} } }])
            .await
            .ok()
            .map(|cursor| cursor.try_collect::<Vec<_>>());
        let sizes = match sizes {
            Some(collect) => collect
                .await
                .ok()
                .and_then(|stats| stats.into_iter().next())
                .and_then(|s| {
                    s.get_document("storageStats")
                        .ok()?
                        .get_document("indexSizes")
                        .ok()
                        .cloned()
                }),
            None => None,
        };
        Ok(models
            .into_iter()
            .map(|model| {
                let options = model.options.unwrap_or_default();
                let name = options.name.clone().unwrap_or_default();
                let mut method = None;
                let columns = model
                    .keys
                    .iter()
                    .map(|(field, kind)| match kind {
                        Bson::String(kind) => {
                            method = Some(kind.clone());
                            format!("{field} {kind}")
                        }
                        other
                            if other
                                .as_f64()
                                .or_else(|| other.as_i64().map(|i| i as f64))
                                .or_else(|| other.as_i32().map(f64::from))
                                .is_some_and(|v| v < 0.0) =>
                        {
                            format!("{field} desc")
                        }
                        _ => field.clone(),
                    })
                    .collect();
                let size = sizes
                    .as_ref()
                    .and_then(|s| s.get(&name))
                    .and_then(|b| {
                        b.as_i64()
                            .or_else(|| b.as_i32().map(i64::from))
                            .or_else(|| b.as_f64().map(|f| f as i64))
                    })
                    .map(size_text);
                dbclient_core::IndexDescription {
                    primary: name == "_id_",
                    unique: name == "_id_" || options.unique == Some(true),
                    columns,
                    method,
                    size,
                    condition: options
                        .partial_filter_expression
                        .map(|filter| Bson::Document(filter).into_relaxed_extjson().to_string()),
                    name,
                }
            })
            .collect())
    }

    /// Never used: no statement text is built for this driver. The trait
    /// requires a value.
    fn dialect(&self) -> SqlDialect {
        SqlDialect::Postgres
    }

    async fn in_transaction(&self) -> Result<bool> {
        Ok(false)
    }

    // The three below default to building SQL, which this server would not understand.

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

    async fn close(&self) -> Result<()> {
        // Waits for running operations, then closes the pooled connections.
        self.client.clone().shutdown().await;
        Ok(())
    }
}

/// Parses user-written text, turning a failure into the error the interface
/// shows under the field.
fn parse(part: TextPart, text: Option<&str>) -> Result<Option<Document>> {
    text::parse_document(text.unwrap_or_default())
        .map_err(|e| document::invalid_text(part, e.message, e.position))
}

fn to_json(raw: &RawDocumentBuf) -> Result<serde_json::Value> {
    let document = raw
        .to_document()
        .map_err(|e| Error::Query(format!("a document could not be decoded: {e}")))?;
    Ok(Bson::Document(document).into_canonical_extjson())
}

/// Servers report counts as int32, int64 or double depending on their size.
fn as_u64(value: &Bson) -> Option<u64> {
    match value {
        Bson::Int32(i) => u64::try_from(*i).ok(),
        Bson::Int64(i) => u64::try_from(*i).ok(),
        Bson::Double(f) if *f >= 0.0 => Some(*f as u64),
        _ => None,
    }
}

/// Failures while opening the connection.
fn connect_err(error: mongodb::error::Error) -> Error {
    match *error.kind {
        ErrorKind::InvalidArgument { message, .. }
        | ErrorKind::InvalidTlsConfig { message, .. } => Error::InvalidConfig(message),
        ErrorKind::Authentication { message, .. } => Error::Connect(message),
        _ => Error::Connect(error.to_string()),
    }
}

/// A byte count as the structure view shows sizes: `412 B`, `1.5 KB`, `12.3 MB`.
fn size_text(bytes: i64) -> String {
    let bytes = bytes.max(0) as f64;
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", units[unit])
    }
}

/// Failures of an operation on an open connection.
fn op_err(error: mongodb::error::Error) -> Error {
    let rejected = |code: i32, name: Option<&str>, message: &str| Error::Database {
        // The number and, where the server sends one, its name: `2 (BadValue)`.
        // Refused writes come with the number only: `11000`.
        code: Some(match name.filter(|n| !n.is_empty()) {
            Some(name) => format!("{code} ({name})"),
            None => code.to_string(),
        }),
        message: message.to_owned(),
        position: None,
        line: None,
        hint: None,
        detail: None,
    };
    match &*error.kind {
        ErrorKind::Command(e) => rejected(e.code, Some(&e.code_name), &e.message),
        ErrorKind::Write(WriteFailure::WriteError(e)) => {
            rejected(e.code, e.code_name.as_deref(), &e.message)
        }
        ErrorKind::Write(WriteFailure::WriteConcernError(e)) => {
            rejected(e.code, Some(&e.code_name), &e.message)
        }
        ErrorKind::Io(_)
        | ErrorKind::ServerSelection { .. }
        | ErrorKind::ConnectionPoolCleared { .. }
        | ErrorKind::Shutdown => Error::ConnectionLost(error.to_string()),
        ErrorKind::Authentication { message, .. } => Error::Connect(message.clone()),
        _ => Error::Query(error.to_string()),
    }
}
