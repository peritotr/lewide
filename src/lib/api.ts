// The interface's view of the Rust core. The shapes mirror
// crates/core/src/{model,value,error}.rs; the commands are in src-tauri/src/lib.rs.
import { invoke as tauriInvoke } from "@tauri-apps/api/core";

/** How commands reach the core. In the application this is Tauri's own call. */
let invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T> = tauriInvoke;

/**
 * Development only: a build of the core started as `bridge` (see
 * src-tauri/src/bin/bridge.rs) answers the same commands over HTTP, so the
 * interface in a browser can be tried against real databases.
 */
function bridgeInvoke(port: string): typeof invoke {
  return async (cmd, args = {}) => {
    const response = await fetch(`http://127.0.0.1:${port}/invoke`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ cmd, args }) });
    const answer = await response.json();
    if ("err" in answer) throw answer.err;
    return answer.ok;
  };
}

export type DataModel =
  | "relational"
  | "document"
  | "key_value"
  | "vector"
  | "graph"
  | "time_series"
  | "search";

export interface Capabilities {
  sql: boolean;
  schemas: boolean;
  transactions: boolean;
  cancel: boolean;
  vector_search: boolean;
  file_based: boolean;
  tls: boolean;
}

export interface DriverInfo {
  id: string;
  name: string;
  model: DataModel;
  capabilities: Capabilities;
  default_port: number | null;
  desktop_only: boolean;
}

/** "verify" checks the certificate and host name; "insecure" only encrypts. */
export type TlsMode = "off" | "verify" | "insecure";

export interface TlsConfig {
  mode: TlsMode;
  ca_file?: string;
}

export type SshAuth =
  | { method: "password"; password: string }
  | { method: "key_file"; path: string; passphrase?: string };

export interface SshConfig {
  host: string;
  port?: number;
  user: string;
  auth: SshAuth;
  known_hosts_file?: string;
  /** Fingerprint the user approved after an "unknown_host_key" error. */
  accept_fingerprint?: string;
  /** Write the approved key to known_hosts. */
  remember_host?: boolean;
}

export interface ConnectionConfig {
  driver: string;
  host?: string;
  port?: number;
  user?: string;
  password?: string;
  database?: string;
  tls?: TlsConfig;
  ssh?: SshConfig;
  params?: Record<string, string>;
}

/** 64-bit integers and decimals arrive as strings to keep full precision. */
export type Value =
  | { type: "null" }
  | { type: "bool"; value: boolean }
  | { type: "int"; value: string }
  | { type: "float"; value: number }
  | { type: "decimal"; value: string }
  | { type: "text"; value: string }
  | { type: "bytes"; value: string }
  | { type: "json"; value: unknown }
  | { type: "temporal"; value: string };

export interface Column {
  name: string;
  type_name: string | null;
}

export interface QueryResult {
  columns: Column[];
  rows: Value[][];
  rows_affected: number | null;
  truncated: boolean;
  elapsed_ms: number;
}

export interface DbObject {
  name: string;
  kind: "table" | "view" | "collection" | "index";
  /** The database's own estimate of the row count; null when it keeps none. */
  row_estimate: number | null;
}

export interface ColumnDescription {
  name: string;
  type_name: string | null;
  nullable: boolean;
  default_value: string | null;
  primary_key: boolean;
  /** The comment stored for the column, where the database keeps one. */
  comment?: string | null;
  /** The allowed values, in order, when the column's type is an enumeration. */
  enum_values?: string[] | null;
}

export interface IndexDescription {
  name: string;
  /** Indexed columns in order; an expression is given as its text. */
  columns: string[];
  unique: boolean;
  primary: boolean;
  /** The access method (btree, gin, …), where the database reports it. */
  method?: string | null;
  /** The size as the server words it. */
  size?: string | null;
  /** The predicate of a partial index. */
  condition?: string | null;
}

/** A foreign key in another table that points at this one. */
export interface Reference {
  name: string | null;
  /** Null when the referencing table is in the same namespace. */
  namespace: string | null;
  table: string;
  columns: string[];
  referenced_columns: string[];
  on_update: string | null;
  on_delete: string | null;
}

export interface ForeignKey {
  /** Null where the database keeps no name for it (SQLite). */
  name: string | null;
  columns: string[];
  /** Null when the referenced table is in the same namespace. */
  referenced_namespace: string | null;
  referenced_table: string;
  /** Empty when the key points at the primary key without naming it. */
  referenced_columns: string[];
  on_update: string | null;
  on_delete: string | null;
}

export interface TriggerDescription {
  name: string;
  timing: string | null;
  events: string[];
  definition: string | null;
}

export interface TableStructure {
  name: string;
  columns: ColumnDescription[];
  indexes: IndexDescription[];
  foreign_keys: ForeignKey[];
  /** Foreign keys in other tables that reference this one. */
  referenced_by?: Reference[];
  triggers: TriggerDescription[];
  /** The statements that would create the object again. */
  ddl: string | null;
}

/** A column to write: which one, and what it is called in the file. */
export interface ExportColumn {
  source: string;
  name: string;
}

/** A column as the user wants it; `type_name` and `default_value` are SQL as written. */
export interface ColumnDef {
  name: string;
  type_name: string;
  nullable: boolean;
  default_value?: string | null;
}

export type ColumnChange = { kind: "add"; column: ColumnDef } | { kind: "modify"; from: ColumnDef; to: ColumnDef } | { kind: "drop"; column: string };

export interface StructureChangeSet {
  namespace?: string;
  table: string;
  changes: ColumnChange[];
}

/** One key as the key list shows it. `kind` is the store's own type name. */
export interface KeyEntry {
  key: string;
  kind: string;
  /** Milliseconds until the key expires; null when it does not. */
  ttl_ms: number | null;
}

export interface KeyspaceInfo {
  keys: number;
  memory_bytes: number | null;
  /** Database numbers that hold keys, with their key counts. */
  databases: [number, number][];
}

export type KeyData =
  | { kind: "string"; value: Value }
  | { kind: "hash"; fields: [string, Value][]; total: number }
  | { kind: "list"; items: Value[]; start: number; total: number }
  | { kind: "set"; members: Value[]; cursor: string; total: number }
  | { kind: "zset"; members: [Value, number][]; start: number; total: number }
  | { kind: "other" };

export interface KeyDetails {
  key: string;
  kind: string;
  ttl_ms: number | null;
  size_bytes: number | null;
  encoding: string | null;
  data: KeyData;
}

export interface KeyRange {
  offset?: number;
  limit?: number;
  descending?: boolean;
  cursor?: string;
}

/** One change to one key; each is a single command, applied at once. */
export type KeyCommand =
  | { op: "set_string"; key: string; value: string }
  | { op: "hash_set"; key: string; field: string; value: string }
  | { op: "hash_delete"; key: string; field: string }
  | { op: "list_push"; key: string; value: string; left: boolean }
  | { op: "list_set"; key: string; index: number; value: string }
  | { op: "list_remove"; key: string; index: number }
  | { op: "set_add"; key: string; member: string }
  | { op: "set_remove"; key: string; member: string }
  | { op: "zset_add"; key: string; member: string; score: number }
  | { op: "zset_remove"; key: string; member: string }
  | { op: "delete"; key: string }
  | { op: "rename"; key: string; to: string }
  | { op: "expire"; key: string; seconds: number }
  | { op: "persist"; key: string };

export interface DocCollectionInfo {
  name: string;
  kind: "collection" | "view" | "timeseries";
  /** Estimates from the collection's metadata; null for views. */
  document_count: number | null;
  avg_document_bytes: number | null;
  size_bytes: number | null;
}

/** Filter, projection and sort are MongoDB query documents as the user wrote them. */
export interface FindRequest {
  database: string;
  collection: string;
  filter?: string | null;
  projection?: string | null;
  sort?: string | null;
  skip: number;
  limit: number;
}

export interface FoundDocument {
  /** Canonical Extended JSON: every number and special type is a one-key `$` wrapper. */
  document: Record<string, unknown>;
  size_bytes: number;
}

export interface FindPage {
  documents: FoundDocument[];
  skip: number;
  has_more: boolean;
  elapsed_ms: number;
}

/** `id` is the `_id` exactly as it was read; `document` and `update` are Extended JSON text. */
export type DocumentChange =
  | { op: "insert"; document: string }
  | { op: "replace"; id: unknown; document: string }
  | { op: "update"; id: unknown; update: string }
  | { op: "delete"; id: unknown };

export interface WriteOutcome {
  matched: number;
  modified: number;
  deleted: number;
  inserted_id: unknown;
}

export interface GraphNode {
  id: string;
  labels: string[];
  properties: Record<string, unknown>;
}

export interface GraphRelationship {
  id: string;
  type: string;
  start: string;
  end: string;
  properties: Record<string, unknown>;
}

export type GraphCell =
  | { kind: "value"; value: Value }
  | ({ kind: "node" } & GraphNode)
  | ({ kind: "relationship" } & GraphRelationship)
  | { kind: "path"; nodes: GraphNode[]; relationships: GraphRelationship[] };

export interface CypherResult {
  columns: string[];
  rows: GraphCell[][];
  truncated: boolean;
  graph: { nodes: GraphNode[]; relationships: GraphRelationship[] };
  /** Distinct nodes in the result, drawn or not. */
  node_count: number;
  relationship_count: number;
  nodes_omitted: number;
  relationships_omitted: number;
  counters: Record<string, number> | null;
  elapsed_ms: number;
}

/** A node label or relationship type; `count` is null when counting failed. */
export interface GraphLabel {
  name: string;
  count: number | null;
}

export interface Neighbours {
  nodes: GraphNode[];
  relationships: GraphRelationship[];
  /** Every relationship of the node that matches, shown or not. */
  total: number;
  groups: { type: string; outgoing: boolean; count: number }[];
}

export interface VectorSpec {
  /** Null for a collection's single unnamed vector. */
  name: string | null;
  kind: "dense" | "multi" | "sparse";
  size: number | null;
  distance: string | null;
  /** False where the score is a distance: the lowest is the nearest. */
  higher_is_better: boolean;
}

export interface CollectionInfo {
  name: string;
  status: string | null;
  points: number | null;
  vectors: VectorSpec[];
  payload_fields: { name: string; kind: string; points: number | null }[];
}

export type PointVector = { name: string | null } & (
  | { kind: "dense"; values: number[] }
  | { kind: "multi"; vectors: number[][] }
  | { kind: "sparse"; indices: number[]; values: number[] }
  | { kind: "other"; raw: unknown }
);

/** Ids are always text: integer ids use all 64 bits. */
export interface VectorPoint {
  id: string;
  payload: Record<string, unknown> | null;
  vectors: PointVector[];
}

export interface ScoredPoint extends VectorPoint {
  rank: number;
  score: number;
}

export interface VectorSearchResult {
  hits: ScoredPoint[];
  below_min_score: number;
  elapsed_ms: number;
}

export type VectorOp = "eq" | "ne" | "gt" | "gte" | "lt" | "lte" | "in" | "is_null" | "is_not_null";

export interface VectorCondition {
  field: string;
  op: VectorOp;
  value?: unknown;
}

export interface VectorSearchRequest {
  collection: string;
  query: { by: "vector"; values: number[] } | { by: "point"; id: string };
  using: string | null;
  limit: number;
  min_score: number | null;
  filter: VectorCondition[];
  with_payload: boolean;
  with_vector: boolean;
}

export type ImportFormat = { format: "csv"; delimiter: string; header: boolean; trim: boolean; null_tokens: string[] } | { format: "json" } | { format: "json_lines" };

export interface ImportInspection {
  columns: string[];
  /** The first rows, as read; null is a missing value. */
  sample: (string | null)[][];
  total_rows: number;
  bytes: number;
}

/** Which file column (by index) feeds which table column. */
export interface ImportMapping {
  source: number;
  column: string;
}

export interface ImportRowError {
  /** The line the row starts on in the file. */
  line: number;
  column: string | null;
  value: string | null;
  reason: string;
}

export interface ImportCheck {
  total_rows: number;
  /** How many rows the preview looked at. */
  checked: number;
  errors: number;
  rows: { line: number; values: Value[]; texts: (string | null)[]; error: ImportRowError | null }[];
  /** Table columns no file column feeds. */
  unfilled: string[];
}

export interface ImportSummary {
  inserted: number;
  skipped_count: number;
  skipped: ImportRowError[];
  elapsed_ms: number;
}

export interface ImportTarget {
  namespace?: string;
  table: string;
}

export type ExportFormat =
  | { format: "csv"; delimiter?: string; header?: boolean; null_text?: string; crlf?: boolean; bom?: boolean }
  | { format: "json" }
  | { format: "json_lines" }
  /** `table` is the name as it should appear, already quoted. */
  | { format: "sql"; table: string; dialect: "postgres" | "mysql" | "sqlite" | "sqlserver" | "clickhouse" | "duckdb" };

/** A column and the value it has or should get. */
export interface Cell {
  column: string;
  value: Value;
}

/** `key` is the row's whole primary key as it is now. */
export type RowChange =
  | { kind: "insert"; values: Cell[] }
  | { kind: "update"; key: Cell[]; set: Cell[] }
  | { kind: "delete"; key: Cell[] };

export interface ChangeSet {
  namespace?: string;
  table: string;
  changes: RowChange[];
}

export interface SortColumn {
  column: string;
  descending: boolean;
}

export interface TableRequest {
  namespace?: string;
  table: string;
  offset: number;
  limit: number;
  order_by?: SortColumn[];
  /** A condition as it would follow WHERE. */
  filter?: string;
}

export interface TablePage {
  columns: ColumnDescription[];
  rows: Value[][];
  offset: number;
  /** Rows exist beyond this page. */
  has_more: boolean;
  elapsed_ms: number;
}

/** What the database said about a statement it rejected. */
export interface DbError {
  /** SQLSTATE for PostgreSQL, "number (SQLSTATE)" for MySQL; null when the engine gives none. */
  code: string | null;
  message: string;
  /** 1-based character offset in the statement, when the server says. */
  position: number | null;
  /** 1-based line, for servers that give a line but no offset. */
  line: number | null;
  hint: string | null;
  detail: string | null;
}

/** The error as a database error; anything else becomes one with only a message. */
export function dbError(err: unknown): DbError {
  const structured = payload(err, "database") as DbError | null;
  return structured ?? { code: null, message: errorMessage(err), position: null, line: null, hint: null, detail: null };
}

export interface SavedQuery {
  /** Empty for a query that is being saved for the first time. */
  id: string;
  connection_id: string;
  name: string;
  sql: string;
}

export type Environment = "local" | "staging" | "production";

/** A saved connection. Its `config` never contains passwords. */
export interface SavedConnection {
  id: string;
  name: string;
  /** Two-letter code of the database type the user picked (see lib/types.ts). */
  engine?: string | null;
  environment: Environment;
  colour: number;
  config: ConnectionConfig;
  /** The secrets this connection needs in order to open. */
  requires: { password: boolean; ssh_password: boolean; ssh_passphrase: boolean };
  /** "keychain": all of them are stored. "ask": at least one has to be typed. */
  secrets: "none" | "keychain" | "ask";
}

export interface SaveOptions {
  keepPassword: boolean;
  keepSshSecret: boolean;
  /** Editing: a password left empty keeps the stored one. */
  unchangedIfEmpty: boolean;
}

export interface ConnectAnswers {
  secrets?: Secrets;
  /** Move the typed secrets into the keychain once they have worked. */
  saveSecrets?: boolean;
  acceptFingerprint?: string;
  rememberHost?: boolean;
}

export interface TestResult {
  version: string | null;
  latency_ms: number;
}

/** Passwords typed by the user for a connection that does not store them. */
export interface Secrets {
  password?: string;
  ssh_password?: string;
  ssh_passphrase?: string;
}

export interface ApiError {
  kind: string;
  message: string | number | object;
}

export interface Backend {
  listDrivers(): Promise<DriverInfo[]>;
  openConnection(config: ConnectionConfig): Promise<number>;
  closeConnection(session: number): Promise<void>;
  /** One result per statement in the text, in order. */
  runQuery(session: number, statement: string, maxRows?: number): Promise<QueryResult[]>;
  cancelQuery(session: number): Promise<void>;
  serverVersion(session: number): Promise<string | null>;
  currentNamespace(session: number): Promise<string | null>;
  listNamespaces(session: number): Promise<{ name: string }[]>;
  listObjects(session: number, namespace?: string): Promise<DbObject[]>;
  describeObject(session: number, name: string, namespace?: string): Promise<{ name: string; columns: ColumnDescription[] }>;
  /** The statements a set of row changes would run. */
  previewChanges(session: number, changes: ChangeSet): Promise<string[]>;
  /** Applies row changes as one unit: all of them, or none. */
  applyChanges(session: number, changes: ChangeSet): Promise<{ statements: string[] }>;
  /** One page of a table. The order is stable across pages. */
  fetchRows(session: number, request: TableRequest): Promise<TablePage>;
  /** Exact row count; can be slow on large tables. */
  countRows(session: number, table: string, namespace?: string, filter?: string): Promise<number>;
  /** Columns, indexes, foreign keys, triggers and DDL of one table or view. */
  tableStructure(session: number, name: string, namespace?: string): Promise<TableStructure>;
  /** Writes a whole table, or what a filter leaves of it, to a file. Resolves to the row count. */
  exportTable(session: number, request: Omit<TableRequest, "offset" | "limit">, format: ExportFormat, path: string, columns?: ExportColumn[]): Promise<number>;
  /** The text an export of these rows would write. */
  exportPreview(columns: string[], rows: Value[][], format: ExportFormat): Promise<string>;
  /** Writes rows already loaded, such as a query result, to a file. */
  exportRows(columns: string[], rows: Value[][], format: ExportFormat, path: string): Promise<number>;
  /** The ALTER TABLE statements a set of column changes would run. */
  previewStructure(session: number, changes: StructureChangeSet): Promise<string[]>;
  /** Applies column changes; in one transaction where the engine can undo DDL. */
  applyStructure(session: number, changes: StructureChangeSet): Promise<{ statements: string[] }>;
  /** What a file holds when read with the given format. */
  importInspect(path: string, format: ImportFormat): Promise<ImportInspection>;
  /** The first rows as they would be written, and which cannot be. */
  importCheck(session: number, target: ImportTarget, path: string, format: ImportFormat, mapping: ImportMapping[]): Promise<ImportCheck>;
  /** Appends the file's rows to the table. */
  importRun(session: number, target: ImportTarget, path: string, format: ImportFormat, mapping: ImportMapping[], options: { skip_bad_rows: boolean; one_transaction: boolean }): Promise<ImportSummary>;
  kvKeyspace(session: number, db: number): Promise<KeyspaceInfo>;
  /** One step of listing keys; follow `cursor` until it is "0". */
  kvScan(session: number, db: number, pattern: string, kind: string | null, cursor: string, count: number): Promise<{ keys: KeyEntry[]; cursor: string }>;
  kvRead(session: number, db: number, key: string, range?: KeyRange): Promise<KeyDetails>;
  /** Runs one change and resolves to the command that ran. */
  kvWrite(session: number, db: number, command: KeyCommand): Promise<string>;
  docCollections(session: number, database: string): Promise<DocCollectionInfo[]>;
  docFind(session: number, request: FindRequest): Promise<FindPage>;
  docCount(session: number, database: string, collection: string, filter?: string | null): Promise<number>;
  docWrite(session: number, database: string, collection: string, change: DocumentChange): Promise<WriteOutcome>;
  /** Writes every document `request` matches; `columns` are the top-level fields a CSV file gets. */
  exportDocuments(session: number, request: FindRequest, format: ExportFormat, columns: string[], path: string): Promise<number>;
  /** One result per statement. `nodeLimit` caps the drawn graph, not the rows. */
  graphRun(session: number, statement: string, maxRows: number, nodeLimit: number): Promise<CypherResult[]>;
  graphNeighbours(session: number, nodeId: string, limit: number): Promise<Neighbours>;
  /** Node labels, then relationship types, each with its count when the server gave one. */
  graphLabels(session: number, database: string | null): Promise<[GraphLabel[], GraphLabel[]]>;
  vectorCollections(session: number): Promise<string[]>;
  vectorCollectionInfo(session: number, collection: string): Promise<CollectionInfo>;
  /** Read-only: nothing is written. */
  vectorSearch(session: number, request: VectorSearchRequest): Promise<VectorSearchResult>;
  /** One point with its payload and all its vectors. */
  vectorPoint(session: number, collection: string, id: string): Promise<VectorPoint>;
  /** Whether the session has a transaction open. Rejects when the driver cannot tell. */
  inTransaction(session: number): Promise<boolean>;
  listSavedQueries(connectionId: string): Promise<SavedQuery[]>;
  saveQuery(query: SavedQuery): Promise<SavedQuery>;
  deleteSavedQuery(id: string): Promise<void>;
  listSaved(): Promise<SavedConnection[]>;
  /** `connection.config` carries the passwords as entered; an empty id creates a new entry. */
  saveConnection(connection: SavedConnection, options: SaveOptions): Promise<SavedConnection>;
  deleteSaved(id: string): Promise<void>;
  connectSaved(id: string, answers?: ConnectAnswers): Promise<number>;
  /** Opens and closes a connection to check the settings. */
  testConnection(config: ConnectionConfig, savedId?: string): Promise<TestResult>;
}

const tauriBackend: Backend = {
  listDrivers: () => invoke("list_drivers"),
  openConnection: (config) => invoke("open_connection", { config }),
  closeConnection: (session) => invoke("close_connection", { session }),
  runQuery: (session, statement, maxRows) =>
    invoke("run_query", { session, request: { statement, max_rows: maxRows ?? null } }),
  cancelQuery: (session) => invoke("cancel_query", { session }),
  serverVersion: (session) => invoke("server_version", { session }),
  currentNamespace: (session) => invoke("current_namespace", { session }),
  listNamespaces: (session) => invoke("list_namespaces", { session }),
  listObjects: (session, namespace) =>
    invoke("list_objects", { session, namespace: namespace ?? null }),
  describeObject: (session, name, namespace) => invoke("describe_object", { session, name, namespace: namespace ?? null }),
  previewChanges: (session, changes) => invoke("preview_changes", { session, changes }),
  applyChanges: (session, changes) => invoke("apply_changes", { session, changes }),
  fetchRows: (session, request) => invoke("fetch_rows", { session, request }),
  countRows: (session, table, namespace, filter) =>
    invoke("count_rows", { session, table, namespace: namespace ?? null, filter: filter ?? null }),
  tableStructure: (session, name, namespace) => invoke("table_structure", { session, name, namespace: namespace ?? null }),
  exportTable: (session, request, format, path, columns) =>
    invoke("export_table", { session, request: { ...request, offset: 0, limit: 1 }, format, path, columns: columns ?? null }),
  exportPreview: (columns, rows, format) => invoke("export_preview", { columns, rows, format }),
  exportRows: (columns, rows, format, path) => invoke("export_rows", { columns, rows, format, path }),
  previewStructure: (session, changes) => invoke("preview_structure", { session, changes }),
  applyStructure: (session, changes) => invoke("apply_structure", { session, changes }),
  importInspect: (path, format) => invoke("import_inspect", { path, format }),
  importCheck: (session, target, path, format, mapping) => invoke("import_check", { session, target, path, format, mapping }),
  importRun: (session, target, path, format, mapping, options) => invoke("import_run", { session, target, path, format, mapping, options }),
  kvKeyspace: (session, db) => invoke("kv_keyspace", { session, db }),
  kvScan: (session, db, pattern, kind, cursor, count) => invoke("kv_scan", { session, db, pattern, kind, cursor, count }),
  kvRead: (session, db, key, range) => invoke("kv_read", { session, db, key, range: range ?? null }),
  kvWrite: (session, db, command) => invoke("kv_write", { session, db, command }),
  docCollections: (session, database) => invoke("doc_collections", { session, database }),
  docFind: (session, request) => invoke("doc_find", { session, request }),
  docCount: (session, database, collection, filter) => invoke("doc_count", { session, database, collection, filter: filter ?? null }),
  docWrite: (session, database, collection, change) => invoke("doc_write", { session, database, collection, change }),
  exportDocuments: (session, request, format, columns, path) => invoke("export_documents", { session, request, format, columns, path }),
  graphRun: (session, statement, maxRows, nodeLimit) => invoke("graph_run", { session, request: { statement, max_rows: maxRows, node_limit: nodeLimit } }),
  graphNeighbours: (session, nodeId, limit) => invoke("graph_neighbours", { session, request: { node_id: nodeId, limit } }),
  graphLabels: (session, database) => invoke("graph_labels", { session, database }),
  vectorCollections: (session) => invoke("vector_collections", { session }),
  vectorCollectionInfo: (session, collection) => invoke("vector_collection_info", { session, collection }),
  vectorSearch: (session, request) => invoke("vector_search", { session, request }),
  vectorPoint: (session, collection, id) => invoke("vector_point", { session, collection, id }),
  inTransaction: (session) => invoke("in_transaction", { session }),
  listSavedQueries: (connectionId) => invoke("list_saved_queries", { connectionId }),
  saveQuery: (query) => invoke("save_query", { query }),
  deleteSavedQuery: (id) => invoke("delete_saved_query", { id }),
  listSaved: () => invoke("list_saved_connections"),
  saveConnection: (connection, options) => invoke("save_connection", { connection, ...options }),
  deleteSaved: (id) => invoke("delete_saved_connection", { id }),
  connectSaved: (id, answers = {}) =>
    invoke("connect_saved", {
      id,
      secrets: answers.secrets ?? null,
      saveSecrets: answers.saveSecrets ?? false,
      acceptFingerprint: answers.acceptFingerprint ?? null,
      rememberHost: answers.rememberHost ?? false,
    }),
  testConnection: (config, savedId) => invoke("test_connection", { config, savedId: savedId ?? null }),
};

export const inTauri = "__TAURI_INTERNALS__" in window;

let backend = tauriBackend;

/**
 * Outside the Tauri shell there is no core to talk to. A development build
 * opened in a plain browser gets a canned backend with the design's sample
 * data, so the interface can be reviewed there. Release builds never load it.
 */
export async function initBackend(): Promise<(() => void) | undefined> {
  const bridge = !inTauri && import.meta.env.DEV ? new URLSearchParams(location.search).get("bridge") : null;
  if (bridge && /^\d+$/.test(bridge)) {
    invoke = bridgeInvoke(bridge);
    return undefined;
  }
  if (!inTauri && import.meta.env.DEV) {
    const demo = await import("./demo");
    backend = demo.demoBackend;
    const [{ setAiBackend }, { demoAi }] = await Promise.all([import("./ai"), import("./demo-ai")]);
    setAiBackend(demoAi);
    return demo.seedDemo;
  }
  return undefined;
}

export const api: Backend = {
  listDrivers: () => backend.listDrivers(),
  openConnection: (config) => backend.openConnection(config),
  closeConnection: (session) => backend.closeConnection(session),
  runQuery: (session, statement, maxRows) => backend.runQuery(session, statement, maxRows),
  cancelQuery: (session) => backend.cancelQuery(session),
  serverVersion: (session) => backend.serverVersion(session),
  currentNamespace: (session) => backend.currentNamespace(session),
  listNamespaces: (session) => backend.listNamespaces(session),
  listObjects: (session, namespace) => backend.listObjects(session, namespace),
  describeObject: (session, name, namespace) => backend.describeObject(session, name, namespace),
  previewChanges: (session, changes) => backend.previewChanges(session, changes),
  applyChanges: (session, changes) => backend.applyChanges(session, changes),
  fetchRows: (session, request) => backend.fetchRows(session, request),
  countRows: (session, table, namespace, filter) => backend.countRows(session, table, namespace, filter),
  tableStructure: (session, name, namespace) => backend.tableStructure(session, name, namespace),
  exportTable: (session, request, format, path, columns) => backend.exportTable(session, request, format, path, columns),
  exportPreview: (columns, rows, format) => backend.exportPreview(columns, rows, format),
  exportRows: (columns, rows, format, path) => backend.exportRows(columns, rows, format, path),
  previewStructure: (session, changes) => backend.previewStructure(session, changes),
  applyStructure: (session, changes) => backend.applyStructure(session, changes),
  importInspect: (path, format) => backend.importInspect(path, format),
  importCheck: (session, target, path, format, mapping) => backend.importCheck(session, target, path, format, mapping),
  importRun: (session, target, path, format, mapping, options) => backend.importRun(session, target, path, format, mapping, options),
  kvKeyspace: (session, db) => backend.kvKeyspace(session, db),
  kvScan: (session, db, pattern, kind, cursor, count) => backend.kvScan(session, db, pattern, kind, cursor, count),
  kvRead: (session, db, key, range) => backend.kvRead(session, db, key, range),
  kvWrite: (session, db, command) => backend.kvWrite(session, db, command),
  docCollections: (session, database) => backend.docCollections(session, database),
  docFind: (session, request) => backend.docFind(session, request),
  docCount: (session, database, collection, filter) => backend.docCount(session, database, collection, filter),
  docWrite: (session, database, collection, change) => backend.docWrite(session, database, collection, change),
  exportDocuments: (session, request, format, columns, path) => backend.exportDocuments(session, request, format, columns, path),
  graphRun: (session, statement, maxRows, nodeLimit) => backend.graphRun(session, statement, maxRows, nodeLimit),
  graphNeighbours: (session, nodeId, limit) => backend.graphNeighbours(session, nodeId, limit),
  graphLabels: (session, database) => backend.graphLabels(session, database),
  vectorCollections: (session) => backend.vectorCollections(session),
  vectorCollectionInfo: (session, collection) => backend.vectorCollectionInfo(session, collection),
  vectorSearch: (session, request) => backend.vectorSearch(session, request),
  vectorPoint: (session, collection, id) => backend.vectorPoint(session, collection, id),
  inTransaction: (session) => backend.inTransaction(session),
  listSavedQueries: (connectionId) => backend.listSavedQueries(connectionId),
  saveQuery: (query) => backend.saveQuery(query),
  deleteSavedQuery: (id) => backend.deleteSavedQuery(id),
  listSaved: () => backend.listSaved(),
  saveConnection: (connection, options) => backend.saveConnection(connection, options),
  deleteSaved: (id) => backend.deleteSaved(id),
  connectSaved: (id, answers) => backend.connectSaved(id, answers),
  testConnection: (config, savedId) => backend.testConnection(config, savedId),
};

export function errorKind(err: unknown): string | null {
  return err && typeof err === "object" && "kind" in err ? String((err as ApiError).kind) : null;
}

export interface UnknownHostKey {
  fingerprint: string;
  /** Algorithm as SSH names it, e.g. "ssh-ed25519". */
  key_type: string;
}

export interface HostKeyChanged {
  host: string;
  /** Line in known_hosts, or null when the earlier key was approved in this app. */
  line: number | null;
  saved: string;
  received: string;
}

const payload = (err: unknown, kind: string): unknown =>
  err && typeof err === "object" && (err as ApiError).kind === kind ? (err as ApiError).message : null;

/** The key to approve when the SSH server is not in known_hosts yet. */
export const unknownHostKey = (err: unknown) => payload(err, "unknown_host_key") as UnknownHostKey | null;
export const hostKeyChanged = (err: unknown) => payload(err, "host_key_changed") as HostKeyChanged | null;

export function errorMessage(err: unknown): string {
  const database = payload(err, "database") as DbError | null;
  if (database) return database.message;
  if (errorKind(err) === "timeout") return `No response after ${(err as ApiError).message} s.`;
  if (err && typeof err === "object" && "message" in err) {
    const message = (err as ApiError).message;
    return typeof message === "object" ? JSON.stringify(message) : String(message);
  }
  return String(err);
}

/** 2_900_000 → "2.9M", 412_000 → "412K", 86 → "86". */
export function compactCount(n: number): string {
  for (const [unit, suffix] of [[1e9, "B"], [1e6, "M"], [1e3, "K"]] as const) {
    if (n >= unit) {
      const x = n / unit;
      return `${x >= 100 ? Math.round(x) : Number(x.toFixed(1))}${suffix}`;
    }
  }
  return String(n);
}

export function isNumeric(v: Value): boolean {
  return v.type === "int" || v.type === "float" || v.type === "decimal";
}

/** Size of a base64-encoded value, e.g. "14.2 KB". */
export function byteSize(base64: string): string {
  const padding = base64.endsWith("==") ? 2 : base64.endsWith("=") ? 1 : 0;
  const bytes = Math.max(0, (base64.length * 3) / 4 - padding);
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/** The exact value as text; nothing is rounded or regrouped. */
export function displayValue(v: Value): string {
  switch (v.type) {
    case "null":
      return "NULL";
    case "json":
      return JSON.stringify(v.value);
    case "bytes":
      return `\\x${base64ToHex(v.value, 24)}`;
    default:
      return String(v.value);
  }
}

export function base64ToHex(base64: string, maxBytes: number): string {
  const raw = atob(base64);
  let hex = "";
  for (let i = 0; i < Math.min(raw.length, maxBytes); i++) {
    hex += raw.charCodeAt(i).toString(16).padStart(2, "0");
  }
  return raw.length > maxBytes ? `${hex}… (${raw.length} bytes)` : hex;
}
