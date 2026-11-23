use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use dbclient_ai::context::{SchemaContext, schema_context};
use dbclient_ai::prompt::{self, Task};
use dbclient_ai::{
    AiStore, CancellationToken, ChatMessage, ChatRequest, Completion, ModelInfo, ProviderConfig,
    ProviderEntry, Role, StreamEvent,
};
use dbclient_core::edit::RowChange;
use dbclient_core::export::{ExportFormat, Exporter};
use dbclient_core::import::{
    ImportFormat, Inspection, Mapping, RowError, check_mapping, convert_row,
};
use dbclient_core::kv::{KeyCommand, KeyDetails, KeyValue, KeyspaceInfo, ReadRange, ScanPage};
use dbclient_core::structure::StructureChangeSet;
use dbclient_core::{
    ApplyResult, ChangeSet, Connection, ConnectionConfig, DbObject, DriverInfo, DriverRegistry,
    Error, Namespace, ObjectDescription, QueryRequest, QueryResult, TablePage, TableRequest,
    TableStructure, Value,
};
use dbclient_store::{
    ConnectionStore, Keep, KeyringSecrets, QueryStore, SavedConnection, SavedQuery, Secrets,
};
use tauri::{Manager, State};
use tokio::sync::RwLock;

/// Open connections, addressed from the UI by a numeric session id.
struct AppState {
    registry: DriverRegistry,
    sessions: RwLock<HashMap<u64, Arc<dyn Connection>>>,
    next_session: AtomicU64,
    /// Saved connections: settings in a file, passwords in the system credential store.
    saved: Arc<ConnectionStore>,
    /// Passwords the user typed but chose not to store, by saved connection id.
    /// They live here, not in the interface, until the application quits.
    typed_secrets: std::sync::Mutex<HashMap<String, Secrets>>,
    /// Queries the user saved, per connection.
    queries: Arc<QueryStore>,
    /// AI providers: settings in a file, API keys in the system credential store.
    ai: Arc<AiStore>,
    http: reqwest::Client,
    /// Replies being streamed, by request id, so they can be stopped.
    ai_running: std::sync::Mutex<HashMap<String, CancellationToken>>,
}

/// How long a connection attempt may take before it is reported as timed out.
const CONNECT_TIMEOUT_SECONDS: u64 = 10;

#[derive(serde::Serialize)]
struct TestResult {
    version: Option<String>,
    latency_ms: u64,
}

impl AppState {
    fn new(
        saved: ConnectionStore,
        queries: QueryStore,
        ai: AiStore,
        http: reqwest::Client,
    ) -> Self {
        let mut registry = DriverRegistry::new();
        registry.register(dbclient_driver_sqlite::SqliteDriver);
        registry.register(dbclient_driver_postgres::PostgresDriver);
        registry.register(dbclient_driver_mysql::MysqlDriver);
        registry.register(dbclient_driver_redis::RedisDriver);
        registry.register(dbclient_driver_qdrant::QdrantDriver);
        registry.register(dbclient_driver_neo4j::Neo4jDriver);
        registry.register(dbclient_driver_mongodb::MongoDriver);
        registry.register(dbclient_driver_sqlserver::SqlServerDriver);
        registry.register(dbclient_driver_clickhouse::ClickHouseDriver);
        registry.register(dbclient_driver_duckdb::DuckDbDriver);
        Self {
            registry,
            sessions: RwLock::new(HashMap::new()),
            next_session: AtomicU64::new(1),
            saved: Arc::new(saved),
            typed_secrets: std::sync::Mutex::new(HashMap::new()),
            queries: Arc::new(queries),
            ai: Arc::new(ai),
            http,
            ai_running: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// After a failed call: if the connection itself is gone, forget the
    /// session and say so, instead of passing on whatever the dead socket reported.
    async fn or_lost<T>(&self, session: u64, result: Result<T, Error>) -> Result<T, Error> {
        let Err(error) = result else { return result };
        let Ok(connection) = self.session(session).await else {
            return Err(error);
        };
        if connection.is_alive().await {
            return Err(error);
        }
        self.sessions.write().await.remove(&session);
        Err(Error::ConnectionLost(error.to_string()))
    }

    /// Clones the handle out so the map lock is not held while a query runs.
    async fn session(&self, id: u64) -> Result<Arc<dyn Connection>, Error> {
        self.sessions
            .read()
            .await
            .get(&id)
            .cloned()
            .ok_or(Error::UnknownSession(id))
    }
}

#[tauri::command]
fn list_drivers(state: State<'_, AppState>) -> Vec<DriverInfo> {
    state.registry.list()
}

impl AppState {
    /// Opens the connection, through an SSH tunnel when the settings ask for one.
    async fn open(&self, config: &ConnectionConfig) -> Result<u64, Error> {
        let connection = dbclient_tunnel::connect(&self.registry, config).await?;
        connection.ping().await?;
        let id = self.next_session.fetch_add(1, Ordering::Relaxed);
        self.sessions
            .write()
            .await
            .insert(id, Arc::from(connection));
        Ok(id)
    }

    /// Runs file and credential store work off the async threads: the system
    /// may show a permission prompt and block until the user answers.
    async fn with_store<T, F>(&self, work: F) -> Result<T, Error>
    where
        T: Send + 'static,
        F: FnOnce(&ConnectionStore) -> Result<T, Error> + Send + 'static,
    {
        let store = self.saved.clone();
        tauri::async_runtime::spawn_blocking(move || work(&store))
            .await
            .map_err(|e| Error::Internal(e.to_string()))?
    }
}

#[tauri::command]
async fn open_connection(
    state: State<'_, AppState>,
    config: ConnectionConfig,
) -> Result<u64, Error> {
    state.open(&config).await
}

#[tauri::command]
async fn list_saved_connections(state: State<'_, AppState>) -> Result<Vec<SavedConnection>, Error> {
    state.with_store(|store| store.list()).await
}

/// Stores the connection. `connection.config` carries the passwords as
/// entered; what comes back has none. With `unchanged_if_empty` (editing), a
/// password left empty keeps the stored one.
#[tauri::command]
async fn save_connection(
    state: State<'_, AppState>,
    connection: SavedConnection,
    keep_password: bool,
    keep_ssh_secret: bool,
    unchanged_if_empty: bool,
) -> Result<SavedConnection, Error> {
    let keep = Keep {
        password: keep_password,
        ssh: keep_ssh_secret,
    };
    let typed = Secrets::of(&connection.config);
    let saved = state
        .with_store(move |store| store.save(connection, keep, unchanged_if_empty))
        .await?;
    // What was entered but not stored stays usable until the application quits.
    let mut cache = state
        .typed_secrets
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if typed.is_empty() {
        cache.remove(&saved.id);
    } else {
        cache.insert(saved.id.clone(), typed);
    }
    Ok(saved)
}

#[tauri::command]
async fn delete_saved_connection(state: State<'_, AppState>, id: String) -> Result<(), Error> {
    state
        .typed_secrets
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&id);
    let queries = state.queries.clone();
    let connection = id.clone();
    state.with_store(move |store| store.delete(&id)).await?;
    // Its saved queries go with it.
    blocking(move || queries.delete_for_connection(&connection)).await
}

async fn blocking<T, F>(work: F) -> Result<T, Error>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, Error> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| Error::Internal(e.to_string()))?
}

#[tauri::command]
async fn list_saved_queries(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<Vec<SavedQuery>, Error> {
    let queries = state.queries.clone();
    blocking(move || queries.list(&connection_id)).await
}

/// Adds a query (empty id) or replaces the one with the same id.
#[tauri::command]
async fn save_query(state: State<'_, AppState>, query: SavedQuery) -> Result<SavedQuery, Error> {
    let queries = state.queries.clone();
    blocking(move || queries.save(query)).await
}

#[tauri::command]
async fn delete_saved_query(state: State<'_, AppState>, id: String) -> Result<(), Error> {
    let queries = state.queries.clone();
    blocking(move || queries.delete(&id)).await
}

/// Opens a saved connection. Passwords come from the credential store, from
/// what was typed earlier in this run, or from `secrets`; they never pass
/// through the interface otherwise. `save_secrets` moves typed ones into the
/// credential store once they have worked.
#[tauri::command]
async fn connect_saved(
    state: State<'_, AppState>,
    id: String,
    secrets: Option<Secrets>,
    save_secrets: bool,
    accept_fingerprint: Option<String>,
    remember_host: bool,
) -> Result<u64, Error> {
    let typed = secrets.clone().or_else(|| {
        let cache = state
            .typed_secrets
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        cache.get(&id).cloned()
    });
    let lookup = id.clone();
    let mut config = state
        .with_store(move |store| store.resolve(&lookup, typed.as_ref()))
        .await?;
    if let (Some(ssh), Some(fingerprint)) = (config.ssh.as_mut(), &accept_fingerprint) {
        ssh.accept_fingerprint = Some(fingerprint.clone());
        ssh.remember_host = remember_host;
    }

    let session = with_timeout(state.open(&config)).await?;

    if let Some(secrets) = secrets {
        if save_secrets {
            let id = id.clone();
            state
                .with_store(move |store| store.remember(&id, &secrets).map(|_| ()))
                .await?;
        } else {
            let mut cache = state
                .typed_secrets
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            cache.insert(id.clone(), secrets);
        }
    }
    // A key that was approved but not written to known_hosts is kept with the connection.
    if let Some(fingerprint) = accept_fingerprint.filter(|_| !remember_host) {
        state
            .with_store(move |store| store.set_fingerprint(&id, &fingerprint))
            .await?;
    }
    Ok(session)
}

/// Opens and closes a connection to check the settings. For a saved connection
/// (`saved_id`), passwords the form left empty are taken from the stored ones.
#[tauri::command]
async fn test_connection(
    state: State<'_, AppState>,
    mut config: ConnectionConfig,
    saved_id: Option<String>,
) -> Result<TestResult, Error> {
    if let Some(id) = saved_id {
        let cached = {
            let cache = state
                .typed_secrets
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            cache.get(&id).cloned().unwrap_or_default()
        };
        let stored = state.with_store(move |store| store.stored(&id)).await?;
        let entered = Secrets::of(&config);
        stored.apply_to(&mut config);
        cached.apply_to(&mut config);
        entered.apply_to(&mut config);
    }
    let started = std::time::Instant::now();
    with_timeout(async {
        let connection = dbclient_tunnel::connect(&state.registry, &config).await?;
        connection.ping().await?;
        let latency_ms = started.elapsed().as_millis() as u64;
        let version = connection.server_version().await.unwrap_or(None);
        let _ = connection.close().await;
        Ok(TestResult {
            version,
            latency_ms,
        })
    })
    .await
}

async fn with_timeout<T>(work: impl Future<Output = Result<T, Error>>) -> Result<T, Error> {
    tokio::time::timeout(
        std::time::Duration::from_secs(CONNECT_TIMEOUT_SECONDS),
        work,
    )
    .await
    .unwrap_or(Err(Error::Timeout(CONNECT_TIMEOUT_SECONDS)))
}

#[tauri::command]
async fn close_connection(state: State<'_, AppState>, session: u64) -> Result<(), Error> {
    let connection = state
        .sessions
        .write()
        .await
        .remove(&session)
        .ok_or(Error::UnknownSession(session))?;
    connection.close().await
}

#[tauri::command]
async fn run_query(
    state: State<'_, AppState>,
    session: u64,
    request: QueryRequest,
) -> Result<Vec<QueryResult>, Error> {
    let result = state.session(session).await?.execute_all(request).await;
    state.or_lost(session, result).await
}

#[tauri::command]
async fn server_version(state: State<'_, AppState>, session: u64) -> Result<Option<String>, Error> {
    state.session(session).await?.server_version().await
}

#[tauri::command]
async fn cancel_query(state: State<'_, AppState>, session: u64) -> Result<(), Error> {
    state.session(session).await?.cancel().await
}

#[tauri::command]
async fn current_namespace(
    state: State<'_, AppState>,
    session: u64,
) -> Result<Option<String>, Error> {
    state.session(session).await?.current_namespace().await
}

#[tauri::command]
async fn list_namespaces(
    state: State<'_, AppState>,
    session: u64,
) -> Result<Vec<Namespace>, Error> {
    state.session(session).await?.list_namespaces().await
}

#[tauri::command]
async fn list_objects(
    state: State<'_, AppState>,
    session: u64,
    namespace: Option<String>,
) -> Result<Vec<DbObject>, Error> {
    state
        .session(session)
        .await?
        .list_objects(namespace.as_deref())
        .await
}

/// One page of a table, in a stable order.
#[tauri::command]
async fn fetch_rows(
    state: State<'_, AppState>,
    session: u64,
    request: TableRequest,
) -> Result<TablePage, Error> {
    let result = state.session(session).await?.fetch_rows(request).await;
    state.or_lost(session, result).await
}

/// The statements a set of row changes would run, for review before applying.
#[tauri::command]
async fn preview_changes(
    state: State<'_, AppState>,
    session: u64,
    changes: ChangeSet,
) -> Result<Vec<String>, Error> {
    state
        .session(session)
        .await?
        .preview_changes(&changes)
        .await
}

/// Applies row changes as one unit: all of them, or none.
#[tauri::command]
async fn apply_changes(
    state: State<'_, AppState>,
    session: u64,
    changes: ChangeSet,
) -> Result<ApplyResult, Error> {
    let result = state.session(session).await?.apply_changes(&changes).await;
    state.or_lost(session, result).await
}

/// Whether the session has a transaction open. An error means the driver cannot tell.
#[tauri::command]
async fn in_transaction(state: State<'_, AppState>, session: u64) -> Result<bool, Error> {
    state.session(session).await?.in_transaction().await
}

/// The exact row count of a table. Can be slow on large tables.
#[tauri::command]
async fn count_rows(
    state: State<'_, AppState>,
    session: u64,
    namespace: Option<String>,
    table: String,
    filter: Option<String>,
) -> Result<u64, Error> {
    state
        .session(session)
        .await?
        .count_rows(namespace.as_deref(), &table, filter.as_deref())
        .await
}

#[tauri::command]
async fn describe_object(
    state: State<'_, AppState>,
    session: u64,
    namespace: Option<String>,
    name: String,
) -> Result<ObjectDescription, Error> {
    state
        .session(session)
        .await?
        .describe_object(namespace.as_deref(), &name)
        .await
}

/// Columns, indexes, foreign keys, triggers and DDL of one table or view.
#[tauri::command]
async fn table_structure(
    state: State<'_, AppState>,
    session: u64,
    namespace: Option<String>,
    name: String,
) -> Result<TableStructure, Error> {
    let result = state
        .session(session)
        .await?
        .table_structure(namespace.as_deref(), &name)
        .await;
    state.or_lost(session, result).await
}

/// The ALTER TABLE statements a set of column changes would run, for review.
#[tauri::command]
async fn preview_structure(
    state: State<'_, AppState>,
    session: u64,
    changes: StructureChangeSet,
) -> Result<Vec<String>, Error> {
    let connection = state.session(session).await?;
    let table = connection.qualified(changes.namespace.as_deref(), &changes.table);
    dbclient_core::structure::statements(connection.dialect(), &table, &changes.changes)
}

/// Applies column changes to a table. Refused while the user has a transaction open.
#[tauri::command]
async fn apply_structure(
    state: State<'_, AppState>,
    session: u64,
    changes: StructureChangeSet,
) -> Result<ApplyResult, Error> {
    let connection = state.session(session).await?;
    if connection.in_transaction().await.unwrap_or(false) {
        return Err(Error::Query(
            "a transaction is open on this connection; commit or roll it back before changing the table".into(),
        ));
    }
    let table = connection.qualified(changes.namespace.as_deref(), &changes.table);
    let statements =
        dbclient_core::structure::statements(connection.dialect(), &table, &changes.changes)?;
    let result = dbclient_core::structure::apply_on(connection.as_ref(), statements).await;
    state.or_lost(session, result).await
}

// ---- Export ----

/// Rows asked for at a time while a table is written out.
const EXPORT_PAGE_ROWS: u32 = 5000;

/// A file that appears under its real name only once it is complete.
struct PartFile {
    path: std::path::PathBuf,
    part: std::path::PathBuf,
}

impl PartFile {
    fn create(path: &str) -> Result<(Self, std::io::BufWriter<std::fs::File>), Error> {
        let path = std::path::PathBuf::from(path);
        let mut name = path
            .file_name()
            .ok_or_else(|| Error::File("not a file name".into()))?
            .to_owned();
        name.push(".part");
        let part = path.with_file_name(name);
        let file = std::fs::File::create(&part).map_err(|e| Error::File(e.to_string()))?;
        Ok((Self { path, part }, std::io::BufWriter::new(file)))
    }

    fn keep(self) -> Result<(), Error> {
        std::fs::rename(&self.part, &self.path).map_err(|e| Error::File(e.to_string()))
    }

    fn discard(self) {
        let _ = std::fs::remove_file(&self.part);
    }
}

/// Writes a whole table (or the rows a filter leaves) to a file, a page at a
/// time. Returns the number of rows written.
#[tauri::command]
async fn export_table(
    state: State<'_, AppState>,
    session: u64,
    request: TableRequest,
    format: ExportFormat,
    columns: Option<Vec<ExportColumn>>,
    path: String,
) -> Result<u64, Error> {
    let connection = state.session(session).await?;
    let (file, out) = PartFile::create(&path)?;
    let result = write_table(connection.as_ref(), request, format, columns, out).await;
    match result {
        Ok(rows) => file.keep().map(|()| rows),
        Err(error) => {
            file.discard();
            state.or_lost(session, Err(error)).await
        }
    }
}

/// A column to write: which one, and what it is called in the file.
#[derive(Debug, Clone, serde::Deserialize)]
struct ExportColumn {
    source: String,
    name: String,
}

async fn write_table(
    connection: &dyn Connection,
    mut request: TableRequest,
    format: ExportFormat,
    columns: Option<Vec<ExportColumn>>,
    out: impl std::io::Write,
) -> Result<u64, Error> {
    request.offset = 0;
    request.limit = EXPORT_PAGE_ROWS;
    let mut page = connection.fetch_rows(request.clone()).await?;
    // Where each written column sits in a row. Without a choice: all, as they are.
    let (picked, names): (Vec<usize>, Vec<String>) = match columns {
        None => page
            .columns
            .iter()
            .enumerate()
            .map(|(i, c)| (i, c.name.clone()))
            .unzip(),
        Some(chosen) => chosen
            .into_iter()
            .map(|column| {
                page.columns
                    .iter()
                    .position(|c| c.name == column.source)
                    .map(|i| (i, column.name))
                    .ok_or_else(|| Error::Query(format!("no such column: {}", column.source)))
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .unzip(),
    };
    let mut exporter = Exporter::new(out, format, names)?;
    loop {
        for row in &mut page.rows {
            *row = picked.iter().map(|&i| row[i].clone()).collect();
        }
        exporter.write_rows(&page.rows)?;
        if !page.has_more {
            return exporter.finish();
        }
        request.offset += u64::from(EXPORT_PAGE_ROWS);
        page = connection.fetch_rows(request.clone()).await?;
    }
}

/// Writes rows the interface already holds, such as a query result, to a file.
#[tauri::command]
async fn export_rows(
    columns: Vec<String>,
    rows: Vec<Vec<Value>>,
    format: ExportFormat,
    path: String,
) -> Result<u64, Error> {
    let (file, out) = PartFile::create(&path)?;
    let written = Exporter::new(out, format, columns).and_then(|mut exporter| {
        exporter.write_rows(&rows)?;
        exporter.finish()
    });
    match written {
        Ok(count) => file.keep().map(|()| count),
        Err(error) => {
            file.discard();
            Err(error)
        }
    }
}

/// The text an export of these rows would write, to show before a file is made.
#[tauri::command]
async fn export_preview(
    columns: Vec<String>,
    rows: Vec<Vec<Value>>,
    format: ExportFormat,
) -> Result<String, Error> {
    let mut out = Vec::new();
    let mut exporter = Exporter::new(&mut out, format, columns)?;
    exporter.write_rows(&rows)?;
    exporter.finish()?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

// ---- Key-value stores ----

/// The key-value side of a session, or an error that says the database is not one.
fn key_value(connection: &dyn Connection) -> Result<&dyn KeyValue, Error> {
    connection
        .key_value()
        .ok_or(Error::Unsupported("key-value operations"))
}

/// ClickHouse fills a column left out of an INSERT with its type's default, so an import needs none of them.
fn implicit_defaults(connection: &dyn Connection) -> bool {
    connection.dialect() == dbclient_core::SqlDialect::ClickHouse
}

fn vector(connection: &dyn Connection) -> Result<&dyn dbclient_core::vector::VectorStore, Error> {
    connection
        .vector()
        .ok_or(Error::Unsupported("vector operations"))
}

fn graph(connection: &dyn Connection) -> Result<&dyn dbclient_core::graph::GraphStore, Error> {
    connection
        .graph()
        .ok_or(Error::Unsupported("graph operations"))
}

fn document(
    connection: &dyn Connection,
) -> Result<&dyn dbclient_core::document::DocumentStore, Error> {
    connection
        .document()
        .ok_or(Error::Unsupported("document operations"))
}

#[tauri::command]
async fn doc_databases(
    state: State<'_, AppState>,
    session: u64,
) -> Result<Vec<dbclient_core::document::DatabaseInfo>, Error> {
    let connection = state.session(session).await?;
    let result = document(connection.as_ref())?.list_databases().await;
    state.or_lost(session, result).await
}

#[tauri::command]
async fn doc_collections(
    state: State<'_, AppState>,
    session: u64,
    database: String,
) -> Result<Vec<dbclient_core::document::CollectionInfo>, Error> {
    let connection = state.session(session).await?;
    let result = document(connection.as_ref())?
        .list_collections(&database)
        .await;
    state.or_lost(session, result).await
}

/// One page of a collection's documents, as canonical Extended JSON.
#[tauri::command]
async fn doc_find(
    state: State<'_, AppState>,
    session: u64,
    request: dbclient_core::document::FindRequest,
) -> Result<dbclient_core::document::FindPage, Error> {
    let connection = state.session(session).await?;
    let result = document(connection.as_ref())?.find(request).await;
    state.or_lost(session, result).await
}

/// The exact number of documents that match a filter.
#[tauri::command]
async fn doc_count(
    state: State<'_, AppState>,
    session: u64,
    database: String,
    collection: String,
    filter: Option<String>,
) -> Result<u64, Error> {
    let connection = state.session(session).await?;
    let result = document(connection.as_ref())?
        .count(&database, &collection, filter.as_deref())
        .await;
    state.or_lost(session, result).await
}

/// Inserts, replaces, updates or deletes one document by its `_id`.
#[tauri::command]
async fn doc_write(
    state: State<'_, AppState>,
    session: u64,
    database: String,
    collection: String,
    change: dbclient_core::document::DocumentChange,
) -> Result<dbclient_core::document::WriteOutcome, Error> {
    let connection = state.session(session).await?;
    let result = document(connection.as_ref())?
        .write(&database, &collection, change)
        .await;
    state.or_lost(session, result).await
}

/// The collections of a vector database, by name.
#[tauri::command]
async fn vector_collections(
    state: State<'_, AppState>,
    session: u64,
) -> Result<Vec<String>, Error> {
    let connection = state.session(session).await?;
    let result = vector(connection.as_ref())?.list_collections().await;
    state.or_lost(session, result).await
}

/// A collection's size, its vectors and its indexed payload fields.
#[tauri::command]
async fn vector_collection_info(
    state: State<'_, AppState>,
    session: u64,
    collection: String,
) -> Result<dbclient_core::vector::CollectionInfo, Error> {
    let connection = state.session(session).await?;
    let result = vector(connection.as_ref())?
        .collection_info(&collection)
        .await;
    state.or_lost(session, result).await
}

/// The points nearest to a vector or to another point. Nothing is written.
#[tauri::command]
async fn vector_search(
    state: State<'_, AppState>,
    session: u64,
    request: dbclient_core::vector::SearchRequest,
) -> Result<dbclient_core::vector::SearchResult, Error> {
    let connection = state.session(session).await?;
    let result = vector(connection.as_ref())?.search(request).await;
    state.or_lost(session, result).await
}

/// One point with its payload and vectors.
#[tauri::command]
async fn vector_point(
    state: State<'_, AppState>,
    session: u64,
    collection: String,
    id: dbclient_core::vector::PointId,
) -> Result<dbclient_core::vector::Point, Error> {
    let connection = state.session(session).await?;
    let result = vector(connection.as_ref())?
        .get_point(&collection, &id)
        .await;
    state.or_lost(session, result).await
}

/// A page of a collection's points, without searching.
#[tauri::command]
async fn vector_scroll(
    state: State<'_, AppState>,
    session: u64,
    request: dbclient_core::vector::ScrollRequest,
) -> Result<dbclient_core::vector::PointPage, Error> {
    let connection = state.session(session).await?;
    let result = vector(connection.as_ref())?.scroll(request).await;
    state.or_lost(session, result).await
}

/// Runs Cypher and returns each statement's rows and the graph they contain.
#[tauri::command]
async fn graph_run(
    state: State<'_, AppState>,
    session: u64,
    request: dbclient_core::graph::CypherRequest,
) -> Result<Vec<dbclient_core::graph::CypherResult>, Error> {
    let connection = state.session(session).await?;
    let result = graph(connection.as_ref())?.run(request).await;
    state.or_lost(session, result).await
}

#[tauri::command]
async fn graph_databases(
    state: State<'_, AppState>,
    session: u64,
) -> Result<Vec<dbclient_core::graph::GraphDatabase>, Error> {
    let connection = state.session(session).await?;
    let result = graph(connection.as_ref())?.databases().await;
    state.or_lost(session, result).await
}

/// Node labels and relationship types with their counts, for the legend and the sidebar.
#[tauri::command]
async fn graph_labels(
    state: State<'_, AppState>,
    session: u64,
    database: Option<String>,
) -> Result<
    (
        Vec<dbclient_core::graph::LabelInfo>,
        Vec<dbclient_core::graph::LabelInfo>,
    ),
    Error,
> {
    let connection = state.session(session).await?;
    let store = graph(connection.as_ref())?;
    let labels = store.labels(database.as_deref()).await;
    let labels = state.or_lost(session, labels).await?;
    let types = store.relationship_types(database.as_deref()).await;
    Ok((labels, state.or_lost(session, types).await?))
}

/// The relationships of one node and the nodes at their other ends.
#[tauri::command]
async fn graph_neighbours(
    state: State<'_, AppState>,
    session: u64,
    request: dbclient_core::graph::NeighbourRequest,
) -> Result<dbclient_core::graph::Neighbours, Error> {
    let connection = state.session(session).await?;
    let result = graph(connection.as_ref())?.neighbours(request).await;
    state.or_lost(session, result).await
}

/// How many keys a database holds, the memory in use, and which databases have keys.
#[tauri::command]
async fn kv_keyspace(
    state: State<'_, AppState>,
    session: u64,
    db: u32,
) -> Result<KeyspaceInfo, Error> {
    let connection = state.session(session).await?;
    let result = key_value(connection.as_ref())?.keyspace(db).await;
    state.or_lost(session, result).await
}

/// One step of listing keys by pattern and type.
#[tauri::command]
async fn kv_scan(
    state: State<'_, AppState>,
    session: u64,
    db: u32,
    pattern: String,
    kind: Option<String>,
    cursor: String,
    count: u32,
) -> Result<ScanPage, Error> {
    let connection = state.session(session).await?;
    let result = key_value(connection.as_ref())?
        .scan(db, &pattern, kind.as_deref(), &cursor, count)
        .await;
    state.or_lost(session, result).await
}

/// One key: its type, time to live, size, and its value or the part asked for.
#[tauri::command]
async fn kv_read(
    state: State<'_, AppState>,
    session: u64,
    db: u32,
    key: String,
    range: Option<ReadRange>,
) -> Result<KeyDetails, Error> {
    let connection = state.session(session).await?;
    let result = key_value(connection.as_ref())?
        .read(db, &key, range.unwrap_or_default())
        .await;
    state.or_lost(session, result).await
}

/// One change to one key. Returns the command that ran, to show the user.
#[tauri::command]
async fn kv_write(
    state: State<'_, AppState>,
    session: u64,
    db: u32,
    command: KeyCommand,
) -> Result<String, Error> {
    let connection = state.session(session).await?;
    let result = key_value(connection.as_ref())?.write(db, command).await;
    state.or_lost(session, result).await
}

// ---- Import ----

/// Rows checked for the preview. The whole file is checked again when it is imported.
const IMPORT_PREVIEW_ROWS: usize = 200;
/// Rows written per statement batch when the import is not one transaction.
const IMPORT_BATCH_ROWS: usize = 500;
/// Skipped rows listed in the summary.
const IMPORT_LISTED_ERRORS: usize = 50;

#[derive(Debug, Clone, serde::Deserialize)]
struct ImportTarget {
    #[serde(default)]
    namespace: Option<String>,
    table: String,
}

#[derive(serde::Serialize)]
struct ImportPreviewRow {
    line: u32,
    /// One value per mapped column; empty when the row has an error.
    values: Vec<Value>,
    /// The texts as they are in the file, per mapped column.
    texts: Vec<Option<String>>,
    error: Option<RowError>,
}

#[derive(serde::Serialize)]
struct ImportCheck {
    total_rows: usize,
    checked: usize,
    errors: usize,
    rows: Vec<ImportPreviewRow>,
    /// Table columns no file column feeds; they get their default or NULL.
    unfilled: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct ImportOptions {
    /// Leave out rows that cannot be written instead of stopping.
    skip_bad_rows: bool,
    one_transaction: bool,
}

#[derive(serde::Serialize)]
struct ImportSummary {
    inserted: u64,
    skipped_count: u64,
    /// The first skipped rows, with the reason for each.
    skipped: Vec<RowError>,
    elapsed_ms: u64,
}

/// What a file holds, read with the given format: its columns, first rows and row count.
#[tauri::command]
async fn import_inspect(path: String, format: ImportFormat) -> Result<Inspection, Error> {
    dbclient_core::import::inspect(std::path::Path::new(&path), &format)
}

/// The first rows of a file as they would be written, and which of them cannot be.
#[tauri::command]
async fn import_check(
    state: State<'_, AppState>,
    session: u64,
    target: ImportTarget,
    path: String,
    format: ImportFormat,
    mapping: Vec<Mapping>,
) -> Result<ImportCheck, Error> {
    let connection = state.session(session).await?;
    let table = connection
        .describe_object(target.namespace.as_deref(), &target.table)
        .await?;
    let unfilled = check_mapping(
        &mapping,
        &table.columns,
        implicit_defaults(connection.as_ref()),
    )
    .map_err(Error::Query)?;
    let source = dbclient_core::import::read_file(std::path::Path::new(&path), &format)?;
    let rows: Vec<ImportPreviewRow> = source
        .rows
        .iter()
        .zip(&source.lines)
        .take(IMPORT_PREVIEW_ROWS)
        .map(|(row, line)| {
            let texts = mapping
                .iter()
                .map(|m| row.get(m.source).cloned().flatten())
                .collect();
            match convert_row(row, *line, &mapping, &table.columns) {
                Ok(cells) => ImportPreviewRow {
                    line: *line,
                    values: cells.into_iter().map(|c| c.value).collect(),
                    texts,
                    error: None,
                },
                Err(error) => ImportPreviewRow {
                    line: *line,
                    values: Vec::new(),
                    texts,
                    error: Some(error),
                },
            }
        })
        .collect();
    Ok(ImportCheck {
        total_rows: source.rows.len(),
        checked: rows.len(),
        errors: rows.iter().filter(|r| r.error.is_some()).count(),
        rows,
        unfilled,
    })
}

/// Appends the rows of a file to a table. Nothing is written before every row
/// has been read and converted; rows that cannot be converted are skipped or
/// stop the import, as `options` says.
#[tauri::command]
async fn import_run(
    state: State<'_, AppState>,
    session: u64,
    target: ImportTarget,
    path: String,
    format: ImportFormat,
    mapping: Vec<Mapping>,
    options: ImportOptions,
) -> Result<ImportSummary, Error> {
    let started = std::time::Instant::now();
    let connection = state.session(session).await?;
    let table = connection
        .describe_object(target.namespace.as_deref(), &target.table)
        .await?;
    check_mapping(
        &mapping,
        &table.columns,
        implicit_defaults(connection.as_ref()),
    )
    .map_err(Error::Query)?;
    let source = dbclient_core::import::read_file(std::path::Path::new(&path), &format)?;

    let mut skipped: Vec<RowError> = Vec::new();
    let mut skipped_count = 0u64;
    let mut skip = |error: RowError| {
        skipped_count += 1;
        if skipped.len() < IMPORT_LISTED_ERRORS {
            skipped.push(error);
        }
    };
    let mut good: Vec<(u32, RowChange)> = Vec::with_capacity(source.rows.len());
    for (row, line) in source.rows.iter().zip(&source.lines) {
        match convert_row(row, *line, &mapping, &table.columns) {
            Ok(values) => good.push((*line, RowChange::Insert { values })),
            Err(error) if options.skip_bad_rows => skip(error),
            Err(error) => {
                return Err(Error::Query(format!(
                    "line {}: {} {}. Nothing was imported.",
                    error.line,
                    error.column.as_deref().unwrap_or("the row"),
                    error.reason
                )));
            }
        }
    }

    let change_set = |changes: Vec<RowChange>| ChangeSet {
        namespace: target.namespace.clone(),
        table: target.table.clone(),
        changes,
    };
    let mut inserted = 0u64;
    if options.one_transaction {
        let count = good.len() as u64;
        if count > 0 {
            let result = connection
                .apply_changes(&change_set(good.into_iter().map(|(_, c)| c).collect()))
                .await
                .map_err(|e| Error::Query(format!("{e} Nothing was imported.")));
            state.or_lost(session, result).await?;
        }
        inserted = count;
    } else {
        for batch in good.chunks(IMPORT_BATCH_ROWS) {
            // A batch the database rejects is halved until the rows at fault
            // are alone; the rest of it still goes in.
            let mut parts = vec![batch];
            while let Some(part) = parts.pop() {
                let changes: Vec<RowChange> = part.iter().map(|(_, c)| c.clone()).collect();
                match connection.apply_changes(&change_set(changes)).await {
                    Ok(_) => inserted += part.len() as u64,
                    Err(error) if !options.skip_bad_rows => {
                        let failure: Result<(), Error> = Err(Error::Query(format!(
                            "{error} {inserted} rows were imported before this and stay in the table."
                        )));
                        return Err(state.or_lost(session, failure).await.unwrap_err());
                    }
                    Err(error) if part.len() == 1 => {
                        // A lost connection would fail every remaining row the same way.
                        if !connection.is_alive().await {
                            let failure: Result<(), Error> = Err(error);
                            return Err(state.or_lost(session, failure).await.unwrap_err());
                        }
                        skip(RowError {
                            line: part[0].0,
                            column: None,
                            value: None,
                            reason: error.to_string(),
                        });
                    }
                    Err(_) => {
                        let (first, second) = part.split_at(part.len() / 2);
                        parts.push(second);
                        parts.push(first);
                    }
                }
            }
        }
    }
    Ok(ImportSummary {
        inserted,
        skipped_count,
        skipped,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

// ---- AI assistant ----
//
// Requests to providers are made here, so API keys never reach the interface.

impl AppState {
    /// Runs settings-file and credential-store work off the async threads.
    async fn with_ai<T, F>(&self, work: F) -> Result<T, Error>
    where
        T: Send + 'static,
        F: FnOnce(&AiStore) -> Result<T, Error> + Send + 'static,
    {
        let store = self.ai.clone();
        tauri::async_runtime::spawn_blocking(move || work(&store))
            .await
            .map_err(|e| Error::Internal(e.to_string()))?
    }
}

#[tauri::command]
async fn ai_list_providers(state: State<'_, AppState>) -> Result<Vec<ProviderEntry>, Error> {
    state.with_ai(|store| store.list()).await
}

/// Adds or updates a provider. `api_key`: absent leaves the stored key, an
/// empty string removes it, anything else replaces it.
#[tauri::command]
async fn ai_save_provider(
    state: State<'_, AppState>,
    provider: ProviderConfig,
    api_key: Option<String>,
) -> Result<ProviderEntry, Error> {
    state
        .with_ai(move |store| store.save(provider, api_key.as_deref()))
        .await
}

#[tauri::command]
async fn ai_delete_provider(state: State<'_, AppState>, id: String) -> Result<(), Error> {
    state.with_ai(move |store| store.delete(&id)).await
}

#[tauri::command]
async fn ai_set_default_provider(state: State<'_, AppState>, id: String) -> Result<(), Error> {
    state.with_ai(move |store| store.set_default(&id)).await
}

/// The models the provider's key can use.
#[tauri::command]
async fn ai_list_models(state: State<'_, AppState>, id: String) -> Result<Vec<ModelInfo>, Error> {
    let (provider, key) = state.with_ai(move |store| store.resolve(Some(&id))).await?;
    dbclient_ai::list_models(&state.http, &provider, key.as_deref()).await
}

/// Sends a one-line request to check the key, the address and the model.
#[tauri::command]
async fn ai_test_provider(state: State<'_, AppState>, id: String) -> Result<Completion, Error> {
    let (provider, key) = state.with_ai(move |store| store.resolve(Some(&id))).await?;
    let request = ChatRequest {
        system: "You are checking a connection. Reply with the single word OK.".into(),
        messages: vec![ChatMessage {
            role: Role::User,
            content: "Reply with the single word OK.".into(),
        }],
    };
    dbclient_ai::stream_chat(
        &state.http,
        &provider,
        key.as_deref(),
        &request,
        &CancellationToken::new(),
        |_| {},
    )
    .await
}

/// What the assistant would be told about the database: structure only, no rows.
#[tauri::command]
async fn ai_preview_context(
    state: State<'_, AppState>,
    session: u64,
    namespace: Option<String>,
    sql: Option<String>,
) -> Result<SchemaContext, Error> {
    let connection = state.session(session).await?;
    schema_context(connection.as_ref(), namespace.as_deref(), sql.as_deref()).await
}

#[derive(serde::Deserialize)]
struct AiAsk {
    /// Chosen by the interface, so the reply can be stopped with `ai_cancel`.
    request_id: String,
    /// The connection the question is about, if any.
    session: Option<u64>,
    /// Engine name as shown to the user, e.g. "PostgreSQL".
    engine: Option<String>,
    namespace: Option<String>,
    provider_id: Option<String>,
    task: Task,
    sql: Option<String>,
    #[serde(default)]
    history: Vec<ChatMessage>,
}

/// Asks the assistant and streams its reply to `on_event`. The request holds
/// the database's structure and the given SQL; nothing here reads table rows.
#[tauri::command]
async fn ai_ask(
    state: State<'_, AppState>,
    ask: AiAsk,
    on_event: tauri::ipc::Channel<StreamEvent>,
) -> Result<Completion, Error> {
    let provider_id = ask.provider_id.clone();
    let (provider, key) = state
        .with_ai(move |store| store.resolve(provider_id.as_deref()))
        .await?;

    let (schema, version) = match ask.session {
        Some(session) => {
            let connection = state.session(session).await?;
            let schema = schema_context(
                connection.as_ref(),
                ask.namespace.as_deref(),
                ask.sql.as_deref(),
            )
            .await?;
            (schema, connection.server_version().await.unwrap_or(None))
        }
        None => (
            SchemaContext {
                text: "(no database is connected; no structure is available)\n".into(),
                namespace: None,
                tables_total: 0,
                tables_detailed: 0,
            },
            None,
        ),
    };
    let database = prompt::Database {
        engine: ask.engine.as_deref().unwrap_or("SQL"),
        version: version.as_deref(),
        schema: &schema,
    };
    let request = prompt::build(&database, &ask.task, ask.sql.as_deref(), &ask.history);

    let cancel = CancellationToken::new();
    state
        .ai_running
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(ask.request_id.clone(), cancel.clone());
    let result = dbclient_ai::stream_chat(
        &state.http,
        &provider,
        key.as_deref(),
        &request,
        &cancel,
        // A closed channel means the interface stopped listening; the reply is dropped.
        |event| {
            let _ = on_event.send(event);
        },
    )
    .await;
    state
        .ai_running
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&ask.request_id);
    result
}

#[tauri::command]
fn ai_cancel(state: State<'_, AppState>, request_id: String) {
    if let Some(cancel) = state
        .ai_running
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&request_id)
    {
        cancel.cancel();
    }
}

/// Every command the interface can call. One list, used by the application and by the tests.
macro_rules! handlers {
    () => {
        tauri::generate_handler![
            list_drivers,
            open_connection,
            list_saved_connections,
            save_connection,
            delete_saved_connection,
            connect_saved,
            list_saved_queries,
            save_query,
            delete_saved_query,
            test_connection,
            close_connection,
            run_query,
            cancel_query,
            server_version,
            current_namespace,
            list_namespaces,
            list_objects,
            describe_object,
            fetch_rows,
            preview_changes,
            apply_changes,
            in_transaction,
            count_rows,
            table_structure,
            preview_structure,
            apply_structure,
            export_table,
            export_rows,
            export_preview,
            import_inspect,
            import_check,
            import_run,
            kv_keyspace,
            kv_scan,
            kv_read,
            kv_write,
            doc_databases,
            doc_collections,
            doc_find,
            doc_count,
            doc_write,
            vector_collections,
            vector_collection_info,
            vector_search,
            vector_point,
            vector_scroll,
            graph_run,
            graph_databases,
            graph_labels,
            graph_neighbours,
            ai_list_providers,
            ai_save_provider,
            ai_delete_provider,
            ai_set_default_provider,
            ai_list_models,
            ai_test_provider,
            ai_preview_context,
            ai_ask,
            ai_cancel,
        ]
    };
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            let service = app.config().identifier.clone();
            let saved = ConnectionStore::new(
                dir.join("connections.json"),
                KeyringSecrets::new(service.clone()),
            );
            let ai = AiStore::new(dir.join("ai.json"), KeyringSecrets::new(service));
            let queries = QueryStore::new(dir.join("queries.json"));
            app.manage(AppState::new(
                saved,
                queries,
                ai,
                dbclient_ai::http_client()?,
            ));
            Ok(())
        })
        .invoke_handler(handlers!())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Development only (feature `bridge`): the commands over HTTP on 127.0.0.1,
/// for the interface running in a browser (`pnpm dev`, then open it with
/// `?bridge=<port>`). Commands go through Tauri's own dispatch on a mock
/// window, so they behave as in the application. Passwords are kept in
/// memory, never in the keychain, and files go to a temporary folder.
/// Streaming (the assistant's answers) is not carried.
#[cfg(feature = "bridge")]
pub mod bridge {
    use super::*;
    use dbclient_store::MemorySecrets;
    use std::io::{BufRead, BufReader, Read, Write};
    use tauri::ipc::{CallbackFn, InvokeBody};
    use tauri::test::{
        INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets,
    };
    use tauri::webview::InvokeRequest;
    use tauri::{WebviewWindow, WebviewWindowBuilder};

    /// The one page allowed to call: the development server of the interface.
    const ORIGIN: &str = "http://localhost:1420";

    pub fn serve(port: u16) -> std::io::Result<()> {
        let dir = std::env::temp_dir().join(format!("lewide-bridge-{}", std::process::id()));
        let state = AppState::new(
            ConnectionStore::new(dir.join("connections.json"), MemorySecrets::default()),
            QueryStore::new(dir.join("queries.json")),
            AiStore::new(dir.join("ai.json"), MemorySecrets::default()),
            dbclient_ai::http_client().map_err(std::io::Error::other)?,
        );
        let app = mock_builder()
            .manage(state)
            .invoke_handler(handlers!())
            .build(mock_context(noop_assets()))
            .map_err(std::io::Error::other)?;
        let window = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .map_err(std::io::Error::other)?;

        let listener = std::net::TcpListener::bind(("127.0.0.1", port))?;
        eprintln!("bridge listening on http://127.0.0.1:{port} for {ORIGIN}");
        for stream in listener.incoming().flatten() {
            let window = window.clone();
            std::thread::spawn(move || {
                let _ = answer(stream, &window);
            });
        }
        Ok(())
    }

    fn answer(
        mut stream: std::net::TcpStream,
        window: &WebviewWindow<MockRuntime>,
    ) -> std::io::Result<()> {
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut first = String::new();
        reader.read_line(&mut first)?;
        let (mut length, mut origin) = (0usize, String::new());
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                match name.to_ascii_lowercase().as_str() {
                    "content-length" => length = value.trim().parse().unwrap_or(0),
                    "origin" => origin = value.trim().to_owned(),
                    _ => {}
                }
            }
        }
        let cors = format!(
            "Access-Control-Allow-Origin: {ORIGIN}\r\nAccess-Control-Allow-Headers: content-type\r\nAccess-Control-Allow-Methods: POST\r\n"
        );
        let reply = |stream: &mut std::net::TcpStream, status: &str, body: &str| {
            write!(
                stream,
                "HTTP/1.1 {status}\r\n{cors}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        };
        if first.starts_with("OPTIONS") {
            return reply(&mut stream, "204 No Content", "");
        }
        // A page from anywhere else gets nothing: the commands reach databases.
        if origin != ORIGIN || !first.starts_with("POST /invoke") {
            return reply(&mut stream, "403 Forbidden", "{}");
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body)?;
        let Ok(request) = serde_json::from_slice::<serde_json::Value>(&body) else {
            return reply(&mut stream, "400 Bad Request", "{}");
        };
        let result = get_ipc_response(
            window,
            InvokeRequest {
                cmd: request["cmd"].as_str().unwrap_or_default().to_owned(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: if cfg!(any(windows, target_os = "android")) {
                    "http://tauri.localhost"
                } else {
                    "tauri://localhost"
                }
                .parse()
                .expect("a fixed address"),
                body: InvokeBody::Json(request["args"].clone()),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        );
        let answer = match result {
            Ok(body) => {
                serde_json::json!({ "ok": body.deserialize::<serde_json::Value>().unwrap_or_default() })
            }
            Err(error) => serde_json::json!({ "err": error }),
        };
        reply(&mut stream, "200 OK", &answer.to_string())
    }
}

/// The commands as the interface calls them: the same names and argument
/// shapes as `src/lib/api.ts` and `src/lib/ai.ts`, through Tauri's own
/// dispatch. A renamed argument or an unregistered command fails here.
#[cfg(test)]
mod ipc_tests {
    use super::*;
    use dbclient_store::MemorySecrets;
    use serde_json::{Value, json};
    use tauri::ipc::{CallbackFn, InvokeBody};
    use tauri::test::{
        INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets,
    };
    use tauri::webview::InvokeRequest;
    use tauri::{WebviewWindow, WebviewWindowBuilder};

    fn window(test: &str) -> (tauri::App<MockRuntime>, WebviewWindow<MockRuntime>) {
        let dir = std::env::temp_dir().join(format!("dbclient-ipc-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let state = AppState::new(
            ConnectionStore::new(dir.join("connections.json"), MemorySecrets::default()),
            QueryStore::new(dir.join("queries.json")),
            AiStore::new(dir.join("ai.json"), MemorySecrets::default()),
            dbclient_ai::http_client().unwrap(),
        );
        let app = mock_builder()
            .manage(state)
            .invoke_handler(handlers!())
            .build(mock_context(noop_assets()))
            .unwrap();
        let window = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        (app, window)
    }

    fn call(
        window: &WebviewWindow<MockRuntime>,
        command: &str,
        args: Value,
    ) -> Result<Value, Value> {
        get_ipc_response(
            window,
            InvokeRequest {
                cmd: command.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                // The address the application's own pages have on each platform.
                url: if cfg!(any(windows, target_os = "android")) {
                    "http://tauri.localhost"
                } else {
                    "tauri://localhost"
                }
                .parse()
                .unwrap(),
                body: InvokeBody::Json(args),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
        .map(|body| body.deserialize::<Value>().unwrap())
    }

    fn ok(window: &WebviewWindow<MockRuntime>, command: &str, args: Value) -> Value {
        call(window, command, args).unwrap_or_else(|e| panic!("{command} failed: {e}"))
    }

    #[test]
    fn a_connection_from_saving_to_editing_rows() {
        let (_app, w) = window("flow");
        let drivers = ok(&w, "list_drivers", json!({}));
        let ids: Vec<_> = drivers
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["id"].as_str().unwrap())
            .collect();
        // More drivers join over time; these are the ones this test relies on.
        for id in [
            "clickhouse",
            "duckdb",
            "mongodb",
            "mysql",
            "neo4j",
            "postgres",
            "qdrant",
            "redis",
            "sqlite",
            "sqlserver",
        ] {
            assert!(ids.contains(&id), "{id} missing in {ids:?}");
        }

        // Saved, listed without secrets, opened by id.
        let saved = ok(
            &w,
            "save_connection",
            json!({
                "connection": {
                    "id": "", "name": "memory", "engine": "Sq", "environment": "local", "colour": 3,
                    "config": { "driver": "sqlite", "database": ":memory:" },
                    "requires": { "password": false, "ssh_password": false, "ssh_passphrase": false },
                    "secrets": "none",
                },
                "keepPassword": true, "keepSshSecret": true, "unchangedIfEmpty": false,
            }),
        );
        let id = saved["id"].as_str().unwrap().to_owned();
        assert_eq!(saved["engine"], "Sq");
        assert_eq!(
            ok(&w, "list_saved_connections", json!({}))
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let session = ok(
            &w,
            "connect_saved",
            json!({
                "id": id, "secrets": null, "saveSecrets": false, "acceptFingerprint": null, "rememberHost": false,
            }),
        );

        let run = |statement: &str| {
            call(
                &w,
                "run_query",
                json!({ "session": session, "request": { "statement": statement, "max_rows": 1000 } }),
            )
        };
        run("CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL)").unwrap();
        run("INSERT INTO people VALUES (1, 'Ada'), (2, 'Linus')").unwrap();
        let sets = run("SELECT id, name FROM people ORDER BY id").unwrap();
        // 64-bit integers cross the boundary as strings.
        assert_eq!(
            sets[0]["rows"][1],
            json!([{ "type": "int", "value": "2" }, { "type": "text", "value": "Linus" }])
        );

        // A rejected statement arrives as a structured error, not a string.
        let error = run("SELECT * FROM").unwrap_err();
        assert_eq!(error["kind"], "database");
        assert!(
            error["message"]["message"]
                .as_str()
                .unwrap()
                .contains("incomplete input"),
            "{error}"
        );

        assert!(ok(&w, "server_version", json!({ "session": session })).is_string());
        assert_eq!(
            ok(&w, "current_namespace", json!({ "session": session })),
            "main"
        );
        assert!(
            !ok(&w, "list_namespaces", json!({ "session": session }))
                .as_array()
                .unwrap()
                .is_empty()
        );
        let objects = ok(
            &w,
            "list_objects",
            json!({ "session": session, "namespace": null }),
        );
        assert_eq!(objects[0]["name"], "people");
        let described = ok(
            &w,
            "describe_object",
            json!({ "session": session, "name": "people", "namespace": null }),
        );
        assert_eq!(described["columns"][0]["primary_key"], true);

        let page = ok(
            &w,
            "fetch_rows",
            json!({
                "session": session,
                "request": { "table": "people", "offset": 0, "limit": 1, "order_by": [{ "column": "name", "descending": true }] },
            }),
        );
        assert_eq!(page["rows"][0][1]["value"], "Linus");
        assert_eq!(page["has_more"], true);
        assert_eq!(
            ok(
                &w,
                "count_rows",
                json!({ "session": session, "table": "people", "namespace": null, "filter": null })
            ),
            2
        );

        let structure = ok(
            &w,
            "table_structure",
            json!({ "session": session, "name": "people", "namespace": null }),
        );
        assert_eq!(structure["columns"][1]["name"], "name");
        assert!(
            structure["ddl"]
                .as_str()
                .unwrap()
                .starts_with("CREATE TABLE people"),
            "{structure}"
        );
        assert_eq!(structure["foreign_keys"], json!([]));

        // Structure changes: reviewed as statements, then applied together.
        let alter = json!({
            "table": "people",
            "changes": [
                { "kind": "add", "column": { "name": "age", "type_name": "INTEGER", "nullable": true } },
                { "kind": "modify", "from": { "name": "name", "type_name": "TEXT", "nullable": false }, "to": { "name": "full_name", "type_name": "TEXT", "nullable": false } },
            ],
        });
        let preview = ok(
            &w,
            "preview_structure",
            json!({ "session": session, "changes": alter }),
        );
        assert_eq!(
            preview,
            json!([
                "ALTER TABLE \"people\" RENAME COLUMN \"name\" TO \"full_name\"",
                "ALTER TABLE \"people\" ADD COLUMN \"age\" INTEGER",
            ])
        );
        ok(
            &w,
            "apply_structure",
            json!({ "session": session, "changes": alter }),
        );
        let after = ok(
            &w,
            "table_structure",
            json!({ "session": session, "name": "people", "namespace": null }),
        );
        assert_eq!(after["columns"][1]["name"], "full_name");
        assert_eq!(after["columns"][2]["name"], "age");
        // A failing statement undoes the ones before it.
        let bad = json!({
            "table": "people",
            "changes": [
                { "kind": "add", "column": { "name": "extra", "type_name": "TEXT", "nullable": true } },
                { "kind": "drop", "column": "no_such_column" },
            ],
        });
        let error = call(
            &w,
            "apply_structure",
            json!({ "session": session, "changes": bad }),
        )
        .unwrap_err();
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("statement 1 of 2 failed"),
            "{error}"
        );
        let after = ok(
            &w,
            "table_structure",
            json!({ "session": session, "name": "people", "namespace": null }),
        );
        assert_eq!(after["columns"].as_array().unwrap().len(), 3);
        // Back to the names the rest of this test uses.
        ok(
            &w,
            "apply_structure",
            json!({ "session": session, "changes": { "table": "people", "changes": [
                { "kind": "drop", "column": "age" },
                { "kind": "modify", "from": { "name": "full_name", "type_name": "TEXT", "nullable": false }, "to": { "name": "name", "type_name": "TEXT", "nullable": false } },
            ] } }),
        );

        // Import: inspect a file, preview it, and write it with bad rows skipped.
        let import_dir = std::env::temp_dir().join(format!("lewide-import-{}", std::process::id()));
        std::fs::create_dir_all(&import_dir).unwrap();
        let csv = import_dir.join("people.csv");
        std::fs::write(
            &csv,
            "id,full name\n10,Grace\nx,Broken\n11,\"Lin, Us\"\n1,Duplicate\n",
        )
        .unwrap();
        let csv_path = csv.to_str().unwrap();
        let format = json!({ "format": "csv", "delimiter": ",", "header": true, "trim": true, "null_tokens": [] });
        let inspected = ok(
            &w,
            "import_inspect",
            json!({ "path": csv_path, "format": format }),
        );
        assert_eq!(inspected["columns"], json!(["id", "full name"]));
        assert_eq!(inspected["total_rows"], 4);
        let mapping = json!([{ "source": 0, "column": "id" }, { "source": 1, "column": "name" }]);
        let target = json!({ "table": "people" });
        let check = ok(
            &w,
            "import_check",
            json!({ "session": session, "target": target, "path": csv_path, "format": format, "mapping": mapping }),
        );
        assert_eq!(check["errors"], 1);
        assert_eq!(check["rows"][1]["error"]["column"], "id");
        assert_eq!(
            check["rows"][2]["values"][1],
            json!({ "type": "text", "value": "Lin, Us" })
        );
        // Stopping on the first bad row writes nothing.
        let stopped = call(
            &w,
            "import_run",
            json!({ "session": session, "target": target, "path": csv_path, "format": format, "mapping": mapping, "options": { "skip_bad_rows": false, "one_transaction": true } }),
        )
        .unwrap_err();
        assert!(
            stopped["message"].as_str().unwrap().contains("line 3"),
            "{stopped}"
        );
        // One transaction: the duplicate key takes everything with it.
        let failed = call(
            &w,
            "import_run",
            json!({ "session": session, "target": target, "path": csv_path, "format": format, "mapping": mapping, "options": { "skip_bad_rows": true, "one_transaction": true } }),
        )
        .unwrap_err();
        assert!(
            failed["message"]
                .as_str()
                .unwrap()
                .contains("Nothing was imported"),
            "{failed}"
        );
        assert_eq!(
            ok(
                &w,
                "count_rows",
                json!({ "session": session, "table": "people", "namespace": null, "filter": null })
            ),
            2
        );
        // Row by row: the two good rows go in, the other two are reported.
        let summary = ok(
            &w,
            "import_run",
            json!({ "session": session, "target": target, "path": csv_path, "format": format, "mapping": mapping, "options": { "skip_bad_rows": true, "one_transaction": false } }),
        );
        assert_eq!(
            (
                summary["inserted"].as_u64(),
                summary["skipped_count"].as_u64()
            ),
            (Some(2), Some(2)),
            "{summary}"
        );
        assert_eq!(summary["skipped"][0]["line"], 3);
        assert_eq!(summary["skipped"][1]["line"], 5);
        run("DELETE FROM people WHERE id >= 10").unwrap();
        std::fs::remove_dir_all(&import_dir).unwrap();

        // Export: a table larger than one page, read back from the file.
        let folder = std::env::temp_dir().join(format!("lewide-export-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = |name: &str| folder.join(name).to_str().unwrap().to_owned();
        run("CREATE TABLE many (id INTEGER PRIMARY KEY, label TEXT)").unwrap();
        run(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 12001) \
             INSERT INTO many SELECT i, 'row ' || i FROM n",
        )
        .unwrap();
        let written = ok(
            &w,
            "export_table",
            json!({
                "session": session,
                "request": { "table": "many", "limit": 1 },
                "format": { "format": "json_lines" },
                "path": file("many.jsonl"),
            }),
        );
        assert_eq!(written, 12001);
        let text = std::fs::read_to_string(file("many.jsonl")).unwrap();
        let ids: Vec<i64> = text
            .lines()
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).unwrap()["id"]
                    .as_i64()
                    .unwrap()
            })
            .collect();
        // Every row once, in key order, across the page boundaries.
        assert_eq!(ids, (1..=12001).collect::<Vec<i64>>());

        let written = ok(
            &w,
            "export_table",
            json!({
                "session": session,
                "request": { "table": "people", "limit": 1, "filter": "name LIKE 'A%'" },
                "format": { "format": "csv", "delimiter": ";" },
                // Reordered and renamed.
                "columns": [{ "source": "name", "name": "full_name" }, { "source": "id", "name": "id" }],
                "path": file("people.csv"),
            }),
        );
        assert_eq!(written, 1);
        assert_eq!(
            std::fs::read_to_string(file("people.csv")).unwrap(),
            "full_name;id\nAda;1\n"
        );
        assert_eq!(
            ok(
                &w,
                "export_preview",
                json!({ "columns": ["id", "name"], "rows": sets[0]["rows"], "format": { "format": "json" } })
            ),
            "[\n  {\"id\": 1, \"name\": \"Ada\"},\n  {\"id\": 2, \"name\": \"Linus\"}\n]\n"
        );

        // A failed export leaves nothing behind, not even a partial file.
        let error = call(
            &w,
            "export_table",
            json!({
                "session": session,
                "request": { "table": "missing", "limit": 1 },
                "format": { "format": "json" },
                "path": file("missing.json"),
            }),
        )
        .unwrap_err();
        assert_eq!(error["kind"], "query", "{error}");
        assert!(!folder.join("missing.json").exists());
        assert!(!folder.join("missing.json.part").exists());

        // Rows the interface already holds, such as a query result.
        let written = ok(
            &w,
            "export_rows",
            json!({
                "columns": ["id", "name"],
                "rows": sets[0]["rows"],
                "format": { "format": "sql", "table": "\"people\"", "dialect": "sqlite" },
                "path": file("people.sql"),
            }),
        );
        assert_eq!(written, 2);
        assert_eq!(
            std::fs::read_to_string(file("people.sql")).unwrap(),
            "INSERT INTO \"people\" (\"id\", \"name\") VALUES (1, 'Ada');\n\
             INSERT INTO \"people\" (\"id\", \"name\") VALUES (2, 'Linus');\n"
        );
        std::fs::remove_dir_all(&folder).unwrap();

        let changes = json!({
            "table": "people",
            "changes": [
                { "kind": "update", "key": [{ "column": "id", "value": { "type": "int", "value": "1" } }], "set": [{ "column": "name", "value": { "type": "text", "value": "Ada L." } }] },
                { "kind": "insert", "values": [{ "column": "id", "value": { "type": "int", "value": "3" } }, { "column": "name", "value": { "type": "text", "value": "Grace" } }] },
                { "kind": "delete", "key": [{ "column": "id", "value": { "type": "int", "value": "2" } }] },
            ],
        });
        let preview = ok(
            &w,
            "preview_changes",
            json!({ "session": session, "changes": changes }),
        );
        assert_eq!(preview.as_array().unwrap().len(), 3);
        let applied = ok(
            &w,
            "apply_changes",
            json!({ "session": session, "changes": changes }),
        );
        assert_eq!(applied["statements"], preview);
        assert_eq!(
            ok(&w, "in_transaction", json!({ "session": session })),
            false
        );
        let after = run("SELECT name FROM people ORDER BY id").unwrap();
        assert_eq!(
            after[0]["rows"],
            json!([[{ "type": "text", "value": "Ada L." }], [{ "type": "text", "value": "Grace" }]])
        );

        // The assistant's view of the database: structure, and nothing from the rows.
        let context = ok(
            &w,
            "ai_preview_context",
            json!({ "session": session, "namespace": null, "sql": null }),
        );
        let text = context["text"].as_str().unwrap();
        assert!(
            text.contains("TABLE main.people") && !text.contains("Grace"),
            "{text}"
        );

        ok(&w, "cancel_query", json!({ "session": session }));
        ok(&w, "close_connection", json!({ "session": session }));
        assert_eq!(run("SELECT 1").unwrap_err()["kind"], "unknown_session");
        ok(&w, "delete_saved_connection", json!({ "id": id }));
        assert!(
            ok(&w, "list_saved_connections", json!({}))
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn testing_a_connection_and_unsaved_connections() {
        let (_app, w) = window("test");
        let tested = ok(
            &w,
            "test_connection",
            json!({ "config": { "driver": "sqlite", "database": ":memory:" }, "savedId": null }),
        );
        assert!(tested["latency_ms"].is_number() && tested["version"].is_string());

        let missing = call(&w, "test_connection", json!({ "config": { "driver": "sqlite", "database": "/no/such/file.db" }, "savedId": null })).unwrap_err();
        assert_eq!(missing["kind"], "connect");
        let unknown = call(
            &w,
            "open_connection",
            json!({ "config": { "driver": "oracle" } }),
        )
        .unwrap_err();
        assert_eq!(unknown["kind"], "unknown_driver");

        let session = ok(
            &w,
            "open_connection",
            json!({ "config": { "driver": "sqlite", "database": ":memory:" } }),
        );
        assert!(session.is_number());
    }

    #[test]
    fn saved_queries_and_ai_providers() {
        let (_app, w) = window("stores");
        let query = ok(
            &w,
            "save_query",
            json!({ "query": { "id": "", "connection_id": "c1", "name": "Top customers", "sql": "SELECT 1" } }),
        );
        let listed = ok(&w, "list_saved_queries", json!({ "connectionId": "c1" }));
        assert_eq!(listed[0]["name"], "Top customers");
        ok(&w, "delete_saved_query", json!({ "id": query["id"] }));
        assert!(
            ok(&w, "list_saved_queries", json!({ "connectionId": "c1" }))
                .as_array()
                .unwrap()
                .is_empty()
        );

        assert!(
            ok(&w, "ai_list_providers", json!({}))
                .as_array()
                .unwrap()
                .is_empty()
        );
        let provider = ok(
            &w,
            "ai_save_provider",
            json!({
                "provider": { "id": "", "kind": "open_ai_compatible", "name": "Local", "base_url": "http://127.0.0.1:1", "model": "m" },
                "apiKey": "secret-key",
            }),
        );
        assert_eq!(provider["has_key"], true);
        // The key is written, never read back.
        assert!(!provider.to_string().contains("secret-key"));
        let id = provider["id"].as_str().unwrap();
        ok(&w, "ai_set_default_provider", json!({ "id": id }));
        assert_eq!(
            ok(&w, "ai_list_providers", json!({}))[0]["is_default"],
            true
        );

        // Nothing listens on that port: the failure comes back as an AI error with a reason.
        let failed = call(&w, "ai_test_provider", json!({ "id": id })).unwrap_err();
        assert_eq!(failed["kind"], "ai");
        assert_eq!(failed["message"]["reason"], "network");
        assert_eq!(
            call(&w, "ai_list_models", json!({ "id": id })).unwrap_err()["kind"],
            "ai"
        );
        ok(&w, "ai_cancel", json!({ "requestId": "nothing-running" }));
        ok(&w, "ai_delete_provider", json!({ "id": id }));
    }

    /// Every command name the interface invokes is one this application registers.
    #[test]
    fn the_interface_only_calls_commands_that_exist() {
        let (_app, w) = window("names");
        let mut checked = 0;
        for file in ["../src/lib/api.ts", "../src/lib/ai.ts"] {
            let source = std::fs::read_to_string(file).unwrap();
            for part in source.split("invoke").skip(1) {
                // invoke("name", ...) or invoke<Type>("name", ...)
                let Some(open) = part.find("(\"") else {
                    continue;
                };
                if open > 60 || part[..open].contains(['\n', ';']) {
                    continue;
                }
                let name: String = part[open + 2..]
                    .chars()
                    .take_while(|c| c.is_ascii_lowercase() || *c == '_')
                    .collect();
                if name.is_empty() {
                    continue;
                }
                // Called with no arguments, a registered command complains about arguments; an unknown one is "not found".
                if let Err(error) = call(&w, &name, json!({})) {
                    assert!(
                        !error.to_string().contains("not found"),
                        "{file} calls `{name}`, which is not registered: {error}"
                    );
                }
                checked += 1;
            }
        }
        assert!(
            checked >= 30,
            "only {checked} commands were found in the interface sources"
        );
    }
}
