// Application state. Components read it with `useApp(selector)`; the functions
// below are the only places that change it.
import { create } from "zustand";
import { currentLanguage, setLanguage, t, tn, type Language } from "./i18n";
import { typeOf } from "./lib/types";
import { usePrefs } from "./prefs";
import type { Condition } from "./lib/filter";
import { addRow, editCell, editInsert, emptyPending, pendingCount, removeInsert, toChangeSet, toggleDelete, type Pending } from "./lib/edit";
import { parsePlan, type Plan } from "./lib/plan";
import { destructiveStatements, dialectOf, quoteIdent, type Destructive } from "./lib/sql";
import {
  api,
  dbError,
  displayValue,
  errorMessage,
  type DbError,
  type Value,
  type ColumnChange,
  type SavedQuery,
  type TableStructure,
  type ConnectionConfig,
  type DbObject,
  type DriverInfo,
  type Environment,
  type QueryResult,
  errorKind,
  hostKeyChanged,
  unknownHostKey,
  type ConnectAnswers,
  type HostKeyChanged,
  type SavedConnection,
  type SaveOptions,
  type SortColumn,
  type TablePage,
  type UnknownHostKey,
} from "./lib/api";

export type { Environment };
export type ThemePref = "system" | "light" | "dark";
export type ResultView = "results" | "messages" | "plan";
/** "lost": the connection was open and broke; its tabs stay, and it can be reopened. */
export type ConnStatus = "disconnected" | "connecting" | "connected" | "failed" | "lost";

export interface Conn {
  /** Id of the saved connection. */
  id: string;
  name: string;
  /** Two-letter code of the database type (lib/types.ts); several types can share a driver. */
  engine: string;
  driver: DriverInfo;
  /** Settings without passwords; those stay in the core. */
  config: ConnectionConfig;
  environment: Environment;
  /** Index into the connection marker palette, `--c-1` to `--c-4`. */
  colour: 1 | 2 | 3 | 4;
  requires: SavedConnection["requires"];
  secretPolicy: SavedConnection["secrets"];
  status: ConnStatus;
  lastError: { text: string; kind: string; time: number } | null;
  session: number | null;
  version: string | null;
  namespaces: string[];
  /** The namespace unqualified names resolve to. */
  namespace: string | null;
  /** Loaded object lists, keyed by namespace ("" when the driver has none). */
  objects: Record<string, DbObject[]>;
  /** Column names per table of the current namespace, for the editor's completion. */
  columns: Record<string, string[]>;
  /** "manual": statements run in one transaction until the user commits or rolls back. */
  txMode: "auto" | "manual";
  /** The open transaction on this connection's session, if any. */
  tx: { openedAt: number; statements: number } | null;
  savedQueries: SavedQuery[];
}

export type RunState =
  | { state: "idle" }
  /** `startedAt` is when the whole run began; `index` is the statement in progress. */
  | { state: "running"; startedAt: number; index: number; total: number; preview: string; stopping: boolean }
  | { state: "cancelled"; elapsedMs: number; notRun: number }
  | { state: "done" }
  /** `from` is where the failed statement starts in the editor text, for pointing at the error. */
  | { state: "error"; error: DbError; index: number; total: number; elapsedMs: number; from: number | null; statement: string };

/** One result of a run, with the statement that produced it. */
export interface ResultSet {
  result: QueryResult;
  statement: string;
  /** The row limit the statement ran with. */
  limit: number;
}

/** One line of the Messages view: what a statement did. */
export interface Message {
  time: number;
  /** Position of the statement in its run, from 0. */
  index: number;
  preview: string;
  result: string;
  durationMs: number;
  error: DbError | null;
  /** Where the statement starts in the editor text, when it came from there. */
  from: number | null;
}

export interface PlanState {
  statement: string;
  /** The server's EXPLAIN output as text. */
  raw: string;
  /** The plan as a tree, for engines whose output can be read into one. */
  plan: Plan | null;
  error: string | null;
}

/** A statement to run and where it starts in the editor text. */
export interface RunItem {
  text: string;
  from: number | null;
}

export interface Tab {
  id: string;
  kind: "query" | "table-data" | "ai-chat" | "key-value" | "vector" | "document" | "graph";
  title: string;
  connectionId: string | null;
  sql: string;
  run: RunState;
  sets: ResultSet[];
  /** Index into `sets` of the result on screen. */
  activeSet: number;
  view: ResultView;
  messages: Message[];
  plan: PlanState | null;
  /** When the results on screen were fetched. */
  finishedAt: number | null;
  /** The text was edited since the last run: marks from that run no longer apply. */
  edited: boolean;
  /** A run finished while another tab was in front. */
  unseen: boolean;
  /** The text differs from the saved query, or was never saved. */
  dirty: boolean;
  savedQueryId: string | null;
  limit: number;
  caret: { line: number; col: number };
  /** Set for a table-data tab: the table being browsed, one page at a time. */
  table: TableView | null;
  /** Set for a key-value tab: where the user is in the keyspace. */
  keys: KeysView | null;
  /** Set for a vector search tab: the collection being searched. */
  vector: { collection: string | null } | null;
  /** Set for a document tab: the collection being browsed. */
  doc: { database: string; collection: string } | null;
  /** Set for a graph tab: the Cypher text, kept while the tab is in the background. */
  graph: { text: string } | null;
}

export interface KeysView {
  db: number;
  pattern: string;
  /** The store's type name to list, or null for every type. */
  kind: string | null;
  selected: string | null;
}

export interface TableView {
  namespace: string;
  name: string;
  page: TablePage | null;
  offset: number;
  pageSize: number;
  sort: SortColumn | null;
  /** Exact row count once the user asked for it. */
  total: number | "counting" | null;
  loading: boolean;
  error: string | null;
  /** Which of the tab's two views is showing. */
  mode: "data" | "structure";
  /** The condition rows are filtered by, as it follows WHERE; empty for none. */
  filter: string;
  /** How the filter is written: as conditions, or as the text after WHERE. */
  filterMode: "builder" | "where";
  /** The builder's conditions. `filter` is their SQL while the mode is "builder". */
  conditions: Condition[];
  /** Columns the grid leaves out, by name. */
  hidden: string[];
  subTab: StructureTab;
  /** Loaded the first time the structure view opens. */
  /** Row changes made in the grid and not yet sent to the database. */
  pending: Pending;
  /** Earlier states of `pending`, for undo. */
  undo: Pending[];
  apply: ApplyState;
  /** Column changes made in the structure view and not yet applied. */
  columnEdits: ColumnChange[];
  structureApply: ApplyState;
  structure: { state: "loading" } | { state: "ready"; data: TableStructure } | { state: "error"; error: DbError } | null;
}

export type ApplyState =
  | { state: "idle" }
  | { state: "running" }
  | { state: "ok"; count: number; ms: number }
  | { state: "failed"; error: DbError; count: number };

export type StructureTab = "columns" | "indexes" | "foreign_keys" | "triggers" | "ddl";

export type SettingsSection = "appearance" | "editor" | "data" | "shortcuts" | "ai" | "security" | "about";

/** The dialog on screen, if any. One at a time. */
export type Dialog =
  | { kind: "picker"; draft?: unknown }
  | { kind: "form"; driverId: string; engine?: string; editId?: string; draft?: unknown }
  | { kind: "password"; connId: string; answers: ConnectAnswers; error?: string }
  | { kind: "unknown-host"; connId: string; key: UnknownHostKey; answers: ConnectAnswers }
  | { kind: "host-changed"; connId: string; change: HostKeyChanged }
  | { kind: "remove"; connId: string }
  /** The statements pending row changes would run, for review before applying. */
  | { kind: "apply-changes"; tabId: string; statements: string[] }
  /** S07: the ALTER TABLE statements pending column changes would run. */
  | { kind: "alter"; tabId: string; statements: string[] }
  /** Asked before something that would throw pending row changes away. */
  | { kind: "discard-changes"; tabId: string; then: (() => void) | null }
  | { kind: "settings"; section: SettingsSection; aiTab?: "providers" | "privacy" }
  | { kind: "palette" }
  /** What an AI request carries: the statement and the structure text. */
  | { kind: "ai-sent"; connId: string | null; sql: string | null; lines: string | null }
  /** The import wizard, for the table of a table tab. */
  | { kind: "import"; connId: string; tabId: string }
  /** The export wizard: a whole table, or the rows of a result that is on screen. */
  | { kind: "export"; connId: string; source: { kind: "table"; tabId: string } | { kind: "result"; title: string; result: QueryResult; selection?: boolean } }
  /** Statements held back until the user confirms running them on Production. */
  | { kind: "confirm-run"; tabId: string; items: RunItem[]; found: { kind: Destructive; text: string }[] };

interface AppState {
  drivers: DriverInfo[];
  connections: Conn[];
  activeConnectionId: string | null;
  /** Tree nodes that are open, by key (see `nodeKey`). */
  expanded: Record<string, boolean>;
  tabs: Tab[];
  activeTabId: string | null;
  dialog: Dialog | null;
  theme: ThemePref;
  recents: Recent[];
  /** The interface language. Changing it re-renders the whole interface. */
  language: Language;
  sidebarWidth: number;
  /** The sidebar reduced to a rail of connection badges (N02). */
  sidebarCollapsed: boolean;
  resultsHeight: number;
}

function stored(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

export function setSidebarCollapsed(sidebarCollapsed: boolean) {
  set({ sidebarCollapsed });
  try {
    localStorage.setItem("lewide.sidebar", sidebarCollapsed ? "collapsed" : "open");
  } catch {
    // Without storage the choice lasts for this run.
  }
}

function storedTheme(): ThemePref {
  try {
    const theme = localStorage.getItem("lewide.theme");
    return theme === "light" || theme === "dark" ? theme : "system";
  } catch {
    return "system";
  }
}

/** Tables, saved queries and connections opened lately, newest first, for the command palette. */
export interface Recent {
  kind: "table" | "query" | "connection";
  connId: string;
  namespace?: string;
  name: string;
  queryId?: string;
}

export const useApp = create<AppState>(() => ({
  drivers: [],
  connections: [],
  activeConnectionId: null,
  expanded: {},
  tabs: [],
  activeTabId: null,
  dialog: null,
  theme: storedTheme(),
  recents: [],
  language: currentLanguage(),
  sidebarWidth: 264,
  sidebarCollapsed: stored("lewide.sidebar") === "collapsed",
  resultsHeight: 400,
}));

const set = useApp.setState;
const get = useApp.getState;
let nextId = 1;
const newId = (prefix: string) => `${prefix}${nextId++}`;

export const nodeKey = (connectionId: string, ...path: string[]) =>
  [connectionId, ...path].join("\u0000");

function patchConn(id: string, patch: Partial<Conn>) {
  set((s) => ({ connections: s.connections.map((c) => (c.id === id ? { ...c, ...patch } : c)) }));
}

function patchTab(id: string, patch: Partial<Tab>) {
  set((s) => ({ tabs: s.tabs.map((t) => (t.id === id ? { ...t, ...patch } : t)) }));
}

export const findConn = (id: string | null) => get().connections.find((c) => c.id === id) ?? null;
export const openDialog = (dialog: Dialog | null) => set({ dialog });

export async function loadDrivers() {
  set({ drivers: await api.listDrivers() });
}

const colourOf = (n: number) => ((((n - 1) % 4) + 4) % 4 + 1) as Conn["colour"];

function toConn(saved: SavedConnection, driver: DriverInfo, previous?: Conn): Conn {
  return {
    status: "disconnected",
    lastError: null,
    session: null,
    version: null,
    namespaces: [],
    namespace: null,
    objects: {},
    columns: {},
    txMode: "auto",
    tx: null,
    savedQueries: [],
    ...previous,
    id: saved.id,
    name: saved.name,
    engine: typeOf(saved.engine, driver.id).code,
    driver,
    config: saved.config,
    environment: saved.environment,
    colour: colourOf(saved.colour),
    requires: saved.requires,
    secretPolicy: saved.secrets,
  };
}

/** Loads the saved connections into the sidebar, disconnected. */
export async function loadSaved() {
  const saved = await api.listSaved();
  const drivers = get().drivers;
  // A connection for a driver this build does not have is left out, not deleted.
  const connections = saved.flatMap((s) => {
    const driver = drivers.find((d) => d.id === s.config.driver);
    return driver ? [toConn(s, driver)] : [];
  });
  set({ connections });
}

export interface ConnectionInput {
  name: string;
  engine: string;
  environment: Environment;
  colour: Conn["colour"];
  /** Settings with the passwords as typed. */
  config: ConnectionConfig;
}

/**
 * Stores a new connection (`id` empty) or the changes to an existing one, and
 * returns its id. Rejects with the backend error.
 */
export async function saveConnection(id: string, input: ConnectionInput, options: SaveOptions): Promise<string> {
  const driver = get().drivers.find((d) => d.id === input.config.driver);
  if (!driver) throw new Error(`unknown driver: ${input.config.driver}`);
  const saved = await api.saveConnection(
    {
      id,
      name: input.name,
      engine: input.engine,
      environment: input.environment,
      colour: input.colour,
      config: input.config,
      requires: { password: false, ssh_password: false, ssh_passphrase: false },
      secrets: "none",
    },
    options,
  );
  const previous = findConn(saved.id) ?? undefined;
  const conn = toConn(saved, driver, previous);
  set((s) => ({
    connections: previous ? s.connections.map((c) => (c.id === conn.id ? conn : c)) : [...s.connections, conn],
    activeConnectionId: conn.id,
  }));
  return conn.id;
}

async function loadConnectionDetails(id: string, session: number) {
  const [version, namespace, namespaces] = await Promise.all([
    api.serverVersion(session).catch(() => null),
    api.currentNamespace(session).catch(() => null),
    api.listNamespaces(session).catch(() => []),
  ]);
  patchConn(id, { version, namespace, namespaces: namespaces.map((n) => n.name) });
  const conn = findConn(id);
  if (!conn) return;
  // Open the tree down to the tables of the current namespace.
  const open = namespace ?? "";
  set((s) => ({
    expanded: {
      ...s.expanded,
      [nodeKey(id)]: true,
      [nodeKey(id, open)]: true,
      [nodeKey(id, open, "tables")]: true,
    },
  }));
  await loadObjects(id, open);
}

export async function loadObjects(connectionId: string, namespace: string) {
  const conn = findConn(connectionId);
  if (!conn || conn.session === null) return;
  const objects = await api.listObjects(conn.session, namespace || undefined).catch(() => []);
  const current = findConn(connectionId);
  if (!current) return;
  patchConn(connectionId, { objects: { ...current.objects, [namespace]: objects } });
  if (namespace === (current.namespace ?? "")) void loadColumns(connectionId, namespace, objects);
}

/** Tables beyond this many complete by name only; describing each one takes a round trip. */
const COMPLETION_TABLES = 150;

/** Fetches column names in the background, for the editor's completion. */
async function loadColumns(connectionId: string, namespace: string, objects: DbObject[]) {
  for (const object of objects.slice(0, COMPLETION_TABLES)) {
    const conn = findConn(connectionId);
    if (!conn || conn.session === null) return;
    if (conn.columns[object.name]) continue;
    try {
      const description = await api.describeObject(conn.session, object.name, namespace || undefined);
      const latest = findConn(connectionId);
      if (latest) patchConn(connectionId, { columns: { ...latest.columns, [object.name]: description.columns.map((c) => c.name) } });
    } catch {
      // A table that cannot be described still completes by name.
    }
  }
}

/** Records that an open connection broke. Returns whether `error` was that. */
function noteIfLost(connectionId: string, error: unknown): boolean {
  if (errorKind(error) !== "connection_lost") return false;
  patchConn(connectionId, { session: null, status: "lost", lastError: { text: errorMessage(error), kind: "connection_lost", time: Date.now() } });
  return true;
}

/**
 * Opens a connection. Whatever it needs from the user first (a password, the
 * approval of an SSH host key) is asked through a dialog, which calls this
 * again with the answer. Failures end up on the connection row.
 */
export async function connect(id: string, answers: ConnectAnswers = {}): Promise<void> {
  const conn = findConn(id);
  if (!conn || conn.status === "connecting" || conn.session !== null) return;
  patchConn(id, { status: "connecting", lastError: null });
  set({ dialog: null });
  try {
    const session = await api.connectSaved(id, answers);
    if (answers.acceptFingerprint && conn.config.ssh && !answers.rememberHost) {
      patchConn(id, { config: { ...conn.config, ssh: { ...conn.config.ssh, accept_fingerprint: answers.acceptFingerprint } } });
    }
    if (answers.saveSecrets) patchConn(id, { secretPolicy: "keychain" });
    patchConn(id, { session, status: "connected", tx: null });
    await loadConnectionDetails(id, session);
    void loadSavedQueries(id);
    const model = findConn(id)?.driver.model;
    if (model === "key_value") openKeys(id);
    else if (model === "vector") openVector(id);
    else if (model === "graph") openGraph(id);
    else if (model === "document") {
      // There is no query language to start from: the first collection is the way in.
      const now = findConn(id);
      const first = now?.objects[now.namespace ?? ""]?.[0];
      if (now?.namespace && first) openCollection(id, now.namespace, first.name);
    }
    else if (get().tabs.length === 0) newQueryTab(id);
  } catch (e) {
    const unknown = unknownHostKey(e);
    const changed = hostKeyChanged(e);
    const approvedBefore = conn.config.ssh?.accept_fingerprint;
    if (errorKind(e) === "credentials_required") {
      patchConn(id, { status: "disconnected" });
      set({ dialog: { kind: "password", connId: id, answers } });
    } else if (unknown && approvedBefore && approvedBefore !== unknown.fingerprint && !answers.acceptFingerprint) {
      // The server shows another key than the one approved in this app earlier.
      const change = { host: conn.config.ssh?.host ?? "", line: null, saved: approvedBefore, received: unknown.fingerprint };
      fail(id, "host_key_changed", t("The SSH host key of {host} has changed.", { host: change.host }));
      set({ dialog: { kind: "host-changed", connId: id, change } });
    } else if (unknown) {
      patchConn(id, { status: "disconnected" });
      set({ dialog: { kind: "unknown-host", connId: id, key: unknown, answers } });
    } else if (changed) {
      fail(id, "host_key_changed", t("The SSH host key of {host} has changed.", { host: changed.host }));
      set({ dialog: { kind: "host-changed", connId: id, change: changed } });
    } else if (answers.secrets) {
      // A typed password that did not work: back to the prompt, with the reason.
      patchConn(id, { status: "disconnected" });
      set({ dialog: { kind: "password", connId: id, answers: { ...answers, secrets: undefined }, error: errorMessage(e) } });
    } else {
      fail(id, errorKind(e) ?? "error", errorMessage(e));
    }
  }
}

function fail(id: string, kind: string, text: string) {
  patchConn(id, { status: "failed", lastError: { text, kind, time: Date.now() } });
}

export async function disconnect(id: string) {
  const conn = findConn(id);
  if (!conn || conn.session === null) return;
  await api.closeConnection(conn.session).catch(() => {});
  patchConn(id, { session: null, status: "disconnected" });
}

/** Closes the connection and its tabs, and deletes it with its stored passwords. */
export async function removeConnection(id: string): Promise<void> {
  await disconnect(id);
  await api.deleteSaved(id);
  set((s) => {
    const tabs = s.tabs.filter((t) => t.connectionId !== id);
    return {
      connections: s.connections.filter((c) => c.id !== id),
      activeConnectionId: s.activeConnectionId === id ? null : s.activeConnectionId,
      tabs,
      activeTabId: tabs.some((t) => t.id === s.activeTabId) ? s.activeTabId : (tabs[0]?.id ?? null),
      dialog: null,
    };
  });
}

export function selectConnection(id: string) {
  set({ activeConnectionId: id });
}

export function toggleNode(key: string) {
  set((s) => ({ expanded: { ...s.expanded, [key]: !s.expanded[key] } }));
}

export function blankTab(connectionId: string | null, patch: Partial<Tab>): Tab {
  return {
    id: newId("t"),
    kind: "query",
    title: "Query",
    connectionId,
    sql: "",
    run: { state: "idle" },
    sets: [],
    activeSet: 0,
    view: "results",
    messages: [],
    plan: null,
    finishedAt: null,
    edited: false,
    unseen: false,
    dirty: false,
    savedQueryId: null,
    limit: usePrefs.getState().defaultLimit,
    caret: { line: 1, col: 1 },
    table: null,
    keys: null,
    vector: null,
    doc: null,
    graph: null,
    ...patch,
  };
}

export function newQueryTab(connectionId: string | null = get().activeConnectionId) {
  const count = get().tabs.filter((t) => t.kind === "query").length + 1;
  const tab = blankTab(connectionId, { title: t("Query {number}", { number: count }) });
  set((s) => ({ tabs: [...s.tabs, tab], activeTabId: tab.id }));
  return tab.id;
}

export function noteRecent(recent: Recent) {
  const same = (r: Recent) => r.kind === recent.kind && r.connId === recent.connId && r.namespace === recent.namespace && r.name === recent.name;
  set((s) => ({ recents: [recent, ...s.recents.filter((r) => !same(r))].slice(0, 20) }));
}

/** Opens a table in its own tab and loads its first page. */
export function openTable(connectionId: string, namespace: string, table: string, mode: TableView["mode"] = "data", initial?: { conditions: Condition[]; filter: string }) {
  const open = get().tabs.find((t) => t.connectionId === connectionId && t.table?.namespace === namespace && t.table.name === table);
  if (open) {
    set({ activeTabId: open.id, activeConnectionId: connectionId });
    if (initial) void loadPage(open.id, { ...initial, filterMode: "builder", offset: 0 });
    return showTableMode(open.id, initial ? "data" : mode);
  }
  noteRecent({ kind: "table", connId: connectionId, namespace, name: table });
  const tab = blankTab(connectionId, {
    kind: "table-data",
    title: table,
    table: { namespace, name: table, page: null, offset: 0, pageSize: 500, sort: null, total: null, loading: false, error: null, mode: "data", filter: "", filterMode: "builder", conditions: [], hidden: [], subTab: "columns", structure: null, pending: emptyPending, undo: [], apply: { state: "idle" }, columnEdits: [], structureApply: { state: "idle" } },
  });
  set((s) => ({ tabs: [...s.tabs, tab], activeTabId: tab.id, activeConnectionId: connectionId }));
  void loadPage(tab.id, initial ? { ...initial, filterMode: "builder" } : {});
  if (mode === "structure") showTableMode(tab.id, mode);
}

/** Loads a page of a table tab, after applying `change` (another page, sort or page size). */
export async function loadPage(tabId: string, change: Partial<Pick<TableView, "offset" | "sort" | "pageSize" | "filter" | "filterMode" | "conditions">> = {}) {
  const tab = get().tabs.find((t) => t.id === tabId);
  const conn = findConn(tab?.connectionId ?? null);
  if (!tab?.table) return;
  // Another page, order or filter would leave the pending changes without their rows.
  if (pendingCount(tab.table.pending) > 0) {
    set({ dialog: { kind: "discard-changes", tabId, then: () => void loadPage(tabId, change) } });
    return;
  }
  const view = { ...tab.table, ...change, loading: true, error: null };
  patchTab(tabId, { table: view });
  if (!conn || conn.session === null) {
    patchTab(tabId, { table: { ...view, loading: false, error: t("This tab's connection is not open. Double-click it in the sidebar.") } });
    return;
  }
  try {
    const page = await api.fetchRows(conn.session, {
      namespace: view.namespace || undefined,
      table: view.name,
      offset: view.offset,
      limit: view.pageSize,
      order_by: view.sort ? [view.sort] : [],
      filter: view.filter || undefined,
    });
    // A count made under another filter no longer describes these rows.
    updateTable(tabId, { page, loading: false, ...(change.filter !== undefined ? { total: null } : {}) });
  } catch (e) {
    noteIfLost(conn.id, e);
    updateTable(tabId, { loading: false, error: errorMessage(e) });
  }
}

/** Counts the table's rows exactly. The user asks for this: it can be slow. */
export async function countTable(tabId: string) {
  const tab = get().tabs.find((t) => t.id === tabId);
  const conn = findConn(tab?.connectionId ?? null);
  if (!tab?.table || !conn || conn.session === null) return;
  updateTable(tabId, { total: "counting" });
  try {
    const total = await api.countRows(conn.session, tab.table.name, tab.table.namespace || undefined, tab.table.filter || undefined);
    updateTable(tabId, { total });
  } catch {
    updateTable(tabId, { total: null });
  }
}

/** Switches a table tab between its data and its structure, reading the structure on first use. */
export function showTableMode(tabId: string, mode: TableView["mode"]) {
  const view = get().tabs.find((t) => t.id === tabId)?.table;
  if (!view) return;
  updateTable(tabId, { mode });
  if (mode === "structure" && !view.structure) void loadStructure(tabId);
}

export const showStructureTab = (tabId: string, subTab: StructureTab) => updateTable(tabId, { mode: "structure", subTab });

/** Reads columns, indexes, foreign keys, triggers and DDL of the tab's table. */
export async function loadStructure(tabId: string) {
  const tab = get().tabs.find((t) => t.id === tabId);
  const conn = findConn(tab?.connectionId ?? null);
  if (!tab?.table) return;
  if (!conn || conn.session === null) {
    const message = t("This tab's connection is not open. Double-click it in the sidebar.");
    updateTable(tabId, { structure: { state: "error", error: { code: null, message, position: null, line: null, hint: null, detail: null } } });
    return;
  }
  updateTable(tabId, { structure: { state: "loading" } });
  try {
    const data = await api.tableStructure(conn.session, tab.table.name, tab.table.namespace || undefined);
    updateTable(tabId, { structure: { state: "ready", data } });
  } catch (e) {
    noteIfLost(conn.id, e);
    updateTable(tabId, { structure: { state: "error", error: dbError(e) } });
  }
}

function changePending(tabId: string, change: (pending: Pending, view: TableView) => Pending) {
  const view = get().tabs.find((t) => t.id === tabId)?.table;
  if (!view?.page || view.apply.state === "running") return;
  const next = change(view.pending, view);
  if (next === view.pending) return;
  updateTable(tabId, { pending: next, undo: [...view.undo.slice(-99), view.pending], apply: { state: "idle" } });
}

/** Sets a cell of a loaded row (by its index in the page) as a pending change. */
export const setTableCell = (tabId: string, row: number, column: number, value: Value) =>
  changePending(tabId, (pending, view) => editCell(pending, view.page!.columns, view.page!.rows[row], column, value));

export const setInsertCell = (tabId: string, id: number, column: string, value: Value | null) => changePending(tabId, (pending) => editInsert(pending, id, column, value));
export const addTableRow = (tabId: string) => changePending(tabId, addRow);
export const removeTableInsert = (tabId: string, id: number) => changePending(tabId, (pending) => removeInsert(pending, id));
export const toggleTableRowDelete = (tabId: string, row: number) => changePending(tabId, (pending, view) => toggleDelete(pending, view.page!.columns, view.page!.rows[row]));

export function undoTableEdit(tabId: string) {
  const view = get().tabs.find((t) => t.id === tabId)?.table;
  if (!view || view.undo.length === 0 || view.apply.state === "running") return;
  updateTable(tabId, { pending: view.undo[view.undo.length - 1], undo: view.undo.slice(0, -1), apply: { state: "idle" } });
}

export function discardTableChanges(tabId: string) {
  updateTable(tabId, { pending: emptyPending, undo: [], apply: { state: "idle" } });
}

export function setColumnEdits(tabId: string, columnEdits: ColumnChange[]) {
  updateTable(tabId, { columnEdits, structureApply: { state: "idle" } });
}

export const dismissStructureApply = (tabId: string) => updateTable(tabId, { structureApply: { state: "idle" } });

/** Shows the statements the pending column changes would run. */
export async function previewStructureChanges(tabId: string) {
  const tab = get().tabs.find((t) => t.id === tabId);
  const conn = findConn(tab?.connectionId ?? null);
  if (!tab?.table || !conn || conn.session === null || tab.table.columnEdits.length === 0) return;
  try {
    const statements = await api.previewStructure(conn.session, { namespace: tab.table.namespace || undefined, table: tab.table.name, changes: tab.table.columnEdits });
    set({ dialog: { kind: "alter", tabId, statements } });
  } catch (e) {
    updateTable(tabId, { structureApply: { state: "failed", error: dbError(e), count: tab.table.columnEdits.length } });
  }
}

export async function applyStructureChanges(tabId: string) {
  const tab = get().tabs.find((t) => t.id === tabId);
  const conn = findConn(tab?.connectionId ?? null);
  const view = tab?.table;
  if (!view || !conn || conn.session === null || view.structureApply.state === "running" || view.columnEdits.length === 0) return;
  const count = view.columnEdits.length;
  const started = performance.now();
  updateTable(tabId, { structureApply: { state: "running" } });
  try {
    await api.applyStructure(conn.session, { namespace: view.namespace || undefined, table: view.name, changes: view.columnEdits });
    updateTable(tabId, { columnEdits: [], structureApply: { state: "ok", count, ms: Math.round(performance.now() - started) }, page: null });
    // The table is not what the loaded page and structure describe any more.
    await Promise.all([loadStructure(tabId), loadPage(tabId, { offset: 0 })]);
    void refreshTransaction(conn.id, 0);
  } catch (e) {
    noteIfLost(conn.id, e);
    updateTable(tabId, { structureApply: { state: "failed", error: dbError(e), count } });
  }
}

export const setFilterMode = (tabId: string, filterMode: TableView["filterMode"]) => updateTable(tabId, { filterMode });
export const setHiddenColumns = (tabId: string, hidden: string[]) => updateTable(tabId, { hidden });

export const dismissApply = (tabId: string) => updateTable(tabId, { apply: { state: "idle" } });

/** Shows the statements the pending changes would run. On Production this is the only way to apply. */
export async function previewTableChanges(tabId: string) {
  const tab = get().tabs.find((t) => t.id === tabId);
  const conn = findConn(tab?.connectionId ?? null);
  if (!tab?.table || !conn || conn.session === null) return;
  try {
    const statements = await api.previewChanges(conn.session, toChangeSet(tab.table.namespace, tab.table.name, tab.table.pending));
    set({ dialog: { kind: "apply-changes", tabId, statements } });
  } catch (e) {
    updateTable(tabId, { apply: { state: "failed", error: dbError(e), count: pendingCount(tab.table.pending) } });
  }
}

/** Apply from the pending bar: straight away, except on Production, where the statements are shown first. */
export function requestApply(tabId: string) {
  const tab = get().tabs.find((t) => t.id === tabId);
  const conn = findConn(tab?.connectionId ?? null);
  if (!tab?.table || pendingCount(tab.table.pending) === 0) return;
  if (conn?.environment === "production" || usePrefs.getState().previewBeforeApply) void previewTableChanges(tabId);
  else void applyTableChanges(tabId);
}

/** Sends the pending changes as one transaction: all of them, or none. */
export async function applyTableChanges(tabId: string) {
  const tab = get().tabs.find((t) => t.id === tabId);
  const conn = findConn(tab?.connectionId ?? null);
  const view = tab?.table;
  if (!view || !conn || conn.session === null || view.apply.state === "running") return;
  const count = pendingCount(view.pending);
  if (count === 0) return;
  const started = performance.now();
  updateTable(tabId, { apply: { state: "running" } });
  try {
    await api.applyChanges(conn.session, toChangeSet(view.namespace, view.name, view.pending));
    updateTable(tabId, { pending: emptyPending, undo: [], apply: { state: "ok", count, ms: Math.round(performance.now() - started) } });
    // New rows get their real keys and defaults, deleted rows disappear.
    await loadPage(tabId);
    void refreshTransaction(conn.id, 0);
  } catch (e) {
    noteIfLost(conn.id, e);
    updateTable(tabId, { apply: { state: "failed", error: dbError(e), count } });
  }
}

function updateTable(tabId: string, patch: Partial<TableView>) {
  const current = get().tabs.find((t) => t.id === tabId)?.table;
  if (current) patchTab(tabId, { table: { ...current, ...patch } });
}

export function closeTab(id: string, force = false) {
  const closing = get().tabs.find((t) => t.id === id);
  if (!force && closing?.table && pendingCount(closing.table.pending) > 0) {
    set({ activeTabId: id, dialog: { kind: "discard-changes", tabId: id, then: () => closeTab(id, true) } });
    return;
  }
  set((s) => {
    const index = s.tabs.findIndex((t) => t.id === id);
    const tabs = s.tabs.filter((t) => t.id !== id);
    const activeTabId =
      s.activeTabId === id ? (tabs[Math.min(index, tabs.length - 1)]?.id ?? null) : s.activeTabId;
    return { tabs, activeTabId };
  });
}

export function closeOtherTabs(id: string) {
  set((s) => ({ tabs: s.tabs.filter((t) => t.id === id), activeTabId: id }));
}

export function duplicateTab(id: string) {
  const tab = get().tabs.find((t) => t.id === id);
  if (!tab || tab.table) return;
  const copy = blankTab(tab.connectionId, { title: t("{name} copy", { name: tab.title }), sql: tab.sql, limit: tab.limit, dirty: tab.sql !== "" });
  set((s) => ({ tabs: [...s.tabs, copy], activeTabId: copy.id }));
}

export function activateTab(id: string) {
  const tab = get().tabs.find((t) => t.id === id);
  if (tab?.unseen) patchTab(id, { unseen: false });
  set({ activeTabId: id, ...(tab?.connectionId ? { activeConnectionId: tab.connectionId } : {}) });
}

export const updateTab = patchTab;

/** The user typed in the editor. */
export function editSql(id: string, sql: string) {
  patchTab(id, { sql, edited: true, dirty: true });
}

const oneLine = (text: string) => text.replace(/\s+/g, " ").trim();

function describeResult(result: QueryResult): string {
  if (result.rows_affected !== null) return tn(result.rows_affected, "{count} row affected", "{count} rows affected");
  return result.truncated
    ? t("{count}+ rows returned", { count: result.rows.length })
    : tn(result.rows.length, "{count} row returned", "{count} rows returned");
}

/**
 * Runs the statements one after another on the tab's connection, the way a
 * SQL shell does: each is its own request, and the first failure stops the
 * run. On Production, statements that remove or mass-change data wait for
 * the user's confirmation first.
 */
export function run(tabId: string, items: RunItem[]) {
  const tab = get().tabs.find((t) => t.id === tabId);
  const conn = findConn(tab?.connectionId ?? null);
  if (!tab || !conn || items.length === 0) return;
  const found =
    conn.environment === "production"
      ? items.flatMap((item) => destructiveStatements(item.text, dialectOf(conn.driver.id)))
      : [];
  if (found.length > 0) set({ dialog: { kind: "confirm-run", tabId, items, found } });
  else void runNow(tabId, items);
}

/** Runs without asking. Callers that take statements from the user go through `run`. */
export async function runNow(tabId: string, items: RunItem[]) {
  const tab = get().tabs.find((t) => t.id === tabId);
  const conn = findConn(tab?.connectionId ?? null);
  if (!tab || tab.run.state === "running" || !conn || conn.session === null || items.length === 0) return;
  const session = conn.session;
  const startedAt = Date.now();
  const total = items.length;
  const sets: ResultSet[] = [];
  const messages: Message[] = [];
  const running = (index: number): RunState => ({ state: "running", startedAt, index, total, preview: oneLine(items[index].text), stopping: false });
  patchTab(tabId, { run: running(0), edited: false, unseen: false });

  /** Leaves the run in `state` and refreshes what the statements may have changed. */
  const finish = async (state: RunState, ran: number) => {
    const front = get().activeTabId === tabId;
    const withRows = sets.findIndex((s) => s.result.columns.length > 0);
    patchTab(tabId, { run: state, sets, messages, activeSet: Math.max(withRows, 0), view: "results", finishedAt: Date.now(), unseen: !front });
    await refreshTransaction(conn.id, ran);
    const latest = findConn(conn.id);
    if (latest?.session != null) void loadObjects(conn.id, latest.namespace ?? "");
  };

  // In manual mode the first statement opens the transaction the rest run in.
  if (conn.txMode === "manual" && !conn.tx) {
    try {
      await api.runQuery(session, "BEGIN");
      patchConn(conn.id, { tx: { openedAt: Date.now(), statements: 0 } });
    } catch (e) {
      noteIfLost(conn.id, e);
      await finish({ state: "error", error: dbError(e), index: 0, total, elapsedMs: Date.now() - startedAt, from: null, statement: "BEGIN" }, 0);
      return;
    }
  }

  for (let index = 0; index < total; index++) {
    const item = items[index];
    const began = Date.now();
    if (index > 0) patchTab(tabId, { run: running(index) });
    try {
      const results = await api.runQuery(session, item.text, tab.limit);
      for (const result of results) sets.push({ result, statement: item.text, limit: tab.limit });
      messages.push({ time: began, index, preview: oneLine(item.text), result: results.map(describeResult).join(" · "), durationMs: Date.now() - began, error: null, from: item.from });
    } catch (e) {
      const lost = noteIfLost(conn.id, e);
      const current = get().tabs.find((t) => t.id === tabId)?.run;
      const stopped = current?.state === "running" && current.stopping && !lost;
      const durationMs = Date.now() - began;
      if (stopped) {
        messages.push({ time: began, index, preview: oneLine(item.text), result: t("Cancelled"), durationMs, error: null, from: item.from });
        await finish({ state: "cancelled", elapsedMs: Date.now() - startedAt, notRun: total - index - 1 }, index);
      } else {
        const error = dbError(e);
        messages.push({ time: began, index, preview: oneLine(item.text), result: error.code ? t("Error {code}", { code: error.code }) : t("Error"), durationMs, error, from: item.from });
        await finish({ state: "error", error, index, total, elapsedMs: durationMs, from: item.from, statement: item.text }, index);
      }
      return;
    }
  }
  await finish({ state: "done" }, total);
}

/** Asks the server to stop the statement in progress. Sent once per run. */
export async function cancelRun(tabId: string) {
  const tab = get().tabs.find((t) => t.id === tabId);
  const conn = findConn(tab?.connectionId ?? null);
  if (tab?.run.state !== "running" || tab.run.stopping || conn?.session == null) return;
  patchTab(tabId, { run: { ...tab.run, stopping: true } });
  await api.cancelQuery(conn.session).catch(() => {});
}

/** Whether a statement only reads, so running it again to fetch more rows changes nothing. */
function readsOnly(statement: string, driverId: string): boolean {
  const head = statement.replace(/^(\s|--[^\n]*\n|\/\*[\s\S]*?\*\/)+/, "");
  return /^(select|with|values|table|show)\b/i.test(head) && !/\b(insert|update|delete|merge|into)\b/i.test(head) && destructiveStatements(statement, dialectOf(driverId)).length === 0;
}

/** Whether "Load next" can be offered for the result on screen. */
export function canLoadMore(tab: Tab, conn: Conn | null): boolean {
  const current = tab.sets[tab.activeSet];
  return !!conn && !!current?.result.truncated && readsOnly(current.statement, conn.driver.id);
}

/**
 * Fetches `more` further rows of the result on screen. The statement is run
 * again with a larger limit, which is why it is offered for reading statements only.
 */
export async function loadMore(tabId: string, more: number) {
  const tab = get().tabs.find((t) => t.id === tabId);
  const conn = findConn(tab?.connectionId ?? null);
  const current = tab?.sets[tab.activeSet];
  if (!tab || !current || !canLoadMore(tab, conn) || conn?.session == null) return;
  const limit = current.limit + more;
  patchTab(tabId, { run: { state: "running", startedAt: Date.now(), index: 0, total: 1, preview: oneLine(current.statement), stopping: false } });
  try {
    const results = await api.runQuery(conn.session, current.statement, limit);
    const result = results.find((r) => r.columns.length > 0) ?? current.result;
    const latest = get().tabs.find((t) => t.id === tabId);
    if (!latest) return;
    patchTab(tabId, { run: { state: "done" }, sets: latest.sets.map((s, i) => (i === latest.activeSet ? { result, statement: current.statement, limit } : s)), finishedAt: Date.now() });
  } catch (e) {
    noteIfLost(conn.id, e);
    patchTab(tabId, { run: { state: "error", error: dbError(e), index: 0, total: 1, elapsedMs: 0, from: null, statement: current.statement } });
  }
}

/** Updates what is known about the session's transaction after `ran` statements. */
async function refreshTransaction(connId: string, ran: number) {
  const conn = findConn(connId);
  if (!conn || conn.session === null) return;
  let open: boolean;
  try {
    open = await api.inTransaction(conn.session);
  } catch {
    // The driver cannot tell: keep what this app knows from manual mode.
    open = conn.tx !== null;
  }
  const before = findConn(connId)?.tx ?? null;
  patchConn(connId, { tx: open ? { openedAt: before?.openedAt ?? Date.now(), statements: (before?.statements ?? 0) + ran } : null });
}

/** Commits or rolls back the connection's open transaction. Returns an error text on failure. */
export async function endTransaction(connId: string, how: "COMMIT" | "ROLLBACK"): Promise<string | null> {
  const conn = findConn(connId);
  if (!conn || conn.session === null) return t("The connection is not open.");
  try {
    await api.runQuery(conn.session, how);
    patchConn(connId, { tx: null });
    await refreshTransaction(connId, 0);
    return null;
  } catch (e) {
    noteIfLost(connId, e);
    return errorMessage(e);
  }
}

export function setTransactionMode(connId: string, txMode: Conn["txMode"]) {
  const conn = findConn(connId);
  // Leaving manual mode with work pending would strand it; the menu disables that.
  if (conn && !(conn.tx && txMode === "auto")) patchConn(connId, { txMode });
}

/** Makes `namespace` the one unqualified names resolve to on this connection. */
export async function useNamespace(connId: string, namespace: string): Promise<string | null> {
  const conn = findConn(connId);
  if (!conn || conn.session === null) return t("The connection is not open.");
  const dialect = dialectOf(conn.driver.id);
  if (dialect === "sqlite") return null;
  // A SQL Server session's default schema belongs to its user and cannot be switched.
  if (dialect === "sqlserver") return t("SQL Server cannot change the default schema for this connection. Write the schema in the query, for example {example}.", { example: `${namespace}.table` });
  const quoted = quoteIdent(namespace, dialect);
  try {
    await api.runQuery(conn.session, dialect === "mysql" || dialect === "clickhouse" || dialect === "duckdb" ? `USE ${quoted}` : `SET search_path TO ${quoted}`);
    patchConn(connId, { namespace, columns: {} });
    set((s) => ({ expanded: { ...s.expanded, [nodeKey(connId, namespace)]: true, [nodeKey(connId, namespace, "tables")]: true } }));
    await loadObjects(connId, namespace);
    return null;
  } catch (e) {
    noteIfLost(connId, e);
    return errorMessage(e);
  }
}

/** Asks the database for the plan of `sql` and shows it in the Query plan view. */
export async function explain(tabId: string, sql: string) {
  const tab = get().tabs.find((t) => t.id === tabId);
  const conn = findConn(tab?.connectionId ?? null);
  const statement = sql.trim().replace(/;\s*$/, "");
  if (!tab || !conn || conn.session === null || !statement) return;
  const session = conn.session;
  const text = (result: QueryResult | undefined) => (result ? result.rows.map((row) => row.map(displayValue).join("  ")).join("\n") : "");
  const state: PlanState = { statement, raw: "", plan: null, error: null };
  if (conn.driver.id === "sqlserver") {
    // SHOWPLAN_TEXT compiles without running. The driver runs the three
    // batches under one lock, so nothing else lands between ON and OFF.
    try {
      const sets = await api.runQuery(session, `SET SHOWPLAN_TEXT ON\nGO\n${statement}\nGO\nSET SHOWPLAN_TEXT OFF`);
      // Each statement gives its text, then its plan; the driver's own row count query is left out.
      state.raw = sets
        .filter((set) => set.columns[0]?.name === "StmtText")
        .map((set) => text(set))
        .filter((block) => block && !block.includes("__lewide_rowcount"))
        .join("\n\n");
    } catch (e) {
      noteIfLost(conn.id, e);
      state.error = errorMessage(e);
      // A statement that fails to compile leaves the session in plan mode.
      await api.runQuery(session, "SET SHOWPLAN_TEXT OFF").catch(() => {});
    }
    patchTab(tabId, { plan: state, view: "plan" });
    return;
  }
  try {
    const prefix = conn.driver.id === "sqlite" ? "EXPLAIN QUERY PLAN " : "EXPLAIN ";
    const raw = await api.runQuery(session, prefix + statement);
    state.raw = text(raw[raw.length - 1]);
    // PostgreSQL can also give the plan as a tree with costs.
    if (conn.driver.id === "postgres" && conn.engine !== "Cr") {
      const json = await api.runQuery(session, `EXPLAIN (FORMAT JSON) ${statement}`).catch(() => []);
      const value = json[0]?.rows[0]?.[0];
      if (value?.type === "json") state.plan = parsePlan(value.value);
      else if (value?.type === "text") state.plan = parsePlan(JSON.parse(value.value));
    }
  } catch (e) {
    noteIfLost(conn.id, e);
    state.error = errorMessage(e);
  }
  patchTab(tabId, { plan: state, view: "plan" });
}

// ---- saved queries ----

export async function loadSavedQueries(connId: string) {
  const savedQueries = await api.listSavedQueries(connId).catch(() => []);
  patchConn(connId, { savedQueries });
}

/** Saves the tab's text under `name`, replacing its saved query if it has one. */
export async function saveTabQuery(tabId: string, name: string): Promise<string | null> {
  const tab = get().tabs.find((t) => t.id === tabId);
  if (!tab?.connectionId) return t("Choose a connection for this tab first.");
  try {
    // A graph tab saves its Cypher; the store keeps it in the same field as SQL.
    const text = (tab: Tab | undefined) => (tab?.graph ? tab.graph.text : tab?.sql);
    const saved = await api.saveQuery({ id: tab.savedQueryId ?? "", connection_id: tab.connectionId, name, sql: text(tab) ?? "" });
    const latest = get().tabs.find((t) => t.id === tabId);
    // Typing during the save keeps the tab marked as changed.
    patchTab(tabId, { savedQueryId: saved.id, title: saved.name, dirty: text(latest) !== saved.sql });
    await loadSavedQueries(tab.connectionId);
    return null;
  } catch (e) {
    return errorMessage(e);
  }
}

/** Opens a saved query in a tab, or brings its tab to the front. */
/** Opens the key list of a key-value connection, or brings its tab to the front on that database. */
export function openKeys(connectionId: string, db?: number) {
  const conn = findConn(connectionId);
  const current = Number(/^db(\d+)$/.exec(conn?.namespace ?? "")?.[1] ?? 0);
  const open = get().tabs.find((t) => t.connectionId === connectionId && t.keys);
  if (open) {
    set({ activeTabId: open.id, activeConnectionId: connectionId });
    if (db !== undefined && open.keys!.db !== db) patchTab(open.id, { keys: { ...open.keys!, db, selected: null } });
    return;
  }
  const tab = blankTab(connectionId, { kind: "key-value", title: t("Keys"), keys: { db: db ?? current, pattern: "", kind: null, selected: null } });
  set((s) => ({ tabs: [...s.tabs, tab], activeTabId: tab.id, activeConnectionId: connectionId }));
}

/** Opens the search tab of a vector connection, or brings it to the front on that collection. */
export function openVector(connectionId: string, collection?: string) {
  const open = get().tabs.find((t) => t.connectionId === connectionId && t.vector);
  if (open) {
    set({ activeTabId: open.id, activeConnectionId: connectionId });
    if (collection && open.vector!.collection !== collection) patchTab(open.id, { vector: { collection } });
    return;
  }
  const tab = blankTab(connectionId, { kind: "vector", title: t("Vector search"), vector: { collection: collection ?? null } });
  set((s) => ({ tabs: [...s.tabs, tab], activeTabId: tab.id, activeConnectionId: connectionId }));
}

export function openCollection(connectionId: string, database: string, collection: string) {
  const open = get().tabs.find((t) => t.connectionId === connectionId && t.doc?.database === database && t.doc.collection === collection);
  if (open) return set({ activeTabId: open.id, activeConnectionId: connectionId });
  const tab = blankTab(connectionId, { kind: "document", title: collection, doc: { database, collection } });
  set((s) => ({ tabs: [...s.tabs, tab], activeTabId: tab.id, activeConnectionId: connectionId }));
}

export function openGraph(connectionId: string, text = "MATCH (a)-[r]->(b) RETURN a, r, b LIMIT 50") {
  const tab = blankTab(connectionId, { kind: "graph", title: t("Graph"), graph: { text } });
  set((s) => ({ tabs: [...s.tabs, tab], activeTabId: tab.id, activeConnectionId: connectionId }));
}

export const setGraphText = (tabId: string, text: string) => patchTab(tabId, { graph: { text }, dirty: true });

export const setVectorCollection = (tabId: string, collection: string) => patchTab(tabId, { vector: { collection } });

export const updateKeys = (tabId: string, patch: Partial<KeysView>) => {
  const keys = get().tabs.find((t) => t.id === tabId)?.keys;
  if (keys) patchTab(tabId, { keys: { ...keys, ...patch } });
};

/** Opens an AI chat tab bound to a connection. */
export function newChatTab(connectionId: string | null = get().activeConnectionId) {
  const tab = blankTab(connectionId, { kind: "ai-chat", title: t("AI chat") });
  set((s) => ({ tabs: [...s.tabs, tab], activeTabId: tab.id }));
  return tab.id;
}

/** Opens a new query tab on a connection with the given text in it. Nothing is run. */
export function openQueryWith(connId: string, sql: string) {
  const id = newQueryTab(connId);
  patchTab(id, { sql, dirty: true });
  set({ activeConnectionId: connId });
}

export function openSavedQuery(connId: string, query: SavedQuery) {
  const open = get().tabs.find((t) => t.savedQueryId === query.id);
  if (open) return activateTab(open.id);
  noteRecent({ kind: "query", connId, name: query.name, queryId: query.id });
  // Cypher saved from a graph tab opens in one again.
  const tab =
    findConn(connId)?.driver.id === "neo4j"
      ? blankTab(connId, { kind: "graph", title: query.name, graph: { text: query.sql }, savedQueryId: query.id })
      : blankTab(connId, { title: query.name, sql: query.sql, savedQueryId: query.id });
  set((s) => ({ tabs: [...s.tabs, tab], activeTabId: tab.id, activeConnectionId: connId }));
}

export function setTheme(theme: ThemePref) {
  set({ theme });
  try {
    localStorage.setItem("lewide.theme", theme);
  } catch {
    // Without storage the choice lasts for this run.
  }
}

export function chooseLanguage(language: Language) {
  setLanguage(language);
  set({ language });
}

export const activeTab = (s: AppState) => s.tabs.find((t) => t.id === s.activeTabId) ?? null;
export const connOf = (s: AppState, tab: Tab | null) =>
  s.connections.find((c) => c.id === tab?.connectionId) ?? null;

/** Test and demo hook: replaces the whole state. */
export function replaceState(state: Partial<AppState>) {
  set(state);
}
