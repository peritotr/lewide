import { ArrowRight, Bookmark, ChevronDown, ChevronRight, Circle, Database, Layers, Plus, Search, Table, Terminal, TriangleAlert, PanelLeftOpen } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { api, compactCount, type GraphLabel } from "../lib/api";
import { typeOf } from "../lib/types";
import {
  connect,
  disconnect,
  loadObjects,
  newQueryTab,
  nodeKey,
  openDialog,
  openSavedQuery,
  openTable,
  saveConnection,
  selectConnection,
  toggleNode,
  openKeys,
  openVector,
  openCollection,
  openGraph,
  useApp,
  type Conn, setSidebarCollapsed } from "../store";
import { isMac } from "./TitleBar";
import { colourVar, EnvTag, IconButton, RegMarks, Splitter } from "./ui";
import { t } from "../i18n";

const Chevron = ({ open }: { open: boolean }) => (open ? <ChevronDown size={12} /> : <ChevronRight size={12} />);

function TreeRow({
  indent,
  children,
  selected = false,
  muted = false,
  ...handlers
}: {
  indent: number;
  children: ReactNode;
  selected?: boolean;
  muted?: boolean;
  onClick?: () => void;
  onDoubleClick?: () => void;
}) {
  return (
    <div
      role="row"
      className={`relative flex h-tree cursor-pointer items-center gap-[6px] pr-s2 hover:bg-hover ${muted ? "text-text-2" : ""}`}
      style={{ paddingLeft: indent }}
      {...handlers}
    >
      {selected && (
        <i className="pointer-events-none absolute inset-0 bg-accent-tint" style={{ boxShadow: "inset 2px 0 0 var(--accent)" }} />
      )}
      {children}
    </div>
  );
}

/** Status at the row's right edge: each state has its own shape, not only its own colour. */
function StatusSlot({ conn, onDetails }: { conn: Conn; onDetails(): void }) {
  const slot = "grid w-[12px] flex-none place-items-center";
  switch (conn.status) {
    case "connected":
      return <span className={slot} title={t("Connected")}><i className="size-[7px] bg-ok" /></span>;
    case "connecting":
      return <span className={slot} title={t("Connecting…")}><i className="status-ring" /></span>;
    case "failed":
      return (
        <button
          type="button"
          className={slot}
          title={t("Connection failed — click for details")}
          aria-label={t("Connection failed — details")}
          onClick={(e) => {
            e.stopPropagation();
            onDetails();
          }}
        >
          <TriangleAlert size={12} className="text-danger" />
        </button>
      );
    case "lost":
      return (
        <button
          type="button"
          className={slot}
          title={t("Connection lost — click to reconnect")}
          aria-label={t("Connection lost — reconnect")}
          onClick={(e) => {
            e.stopPropagation();
            void connect(conn.id);
          }}
        >
          {/* A square with a slash through it. */}
          <i className="size-[9px] border border-warn-text" style={{ background: "linear-gradient(to top right, transparent calc(50% - 0.6px), var(--warn-text) calc(50% - 0.6px), var(--warn-text) calc(50% + 0.6px), transparent calc(50% + 0.6px))" }} />
        </button>
      );
    default:
      return <span className={slot} title={t("Disconnected")}><i className="size-[7px] border border-text-3" /></span>;
  }
}

type Floating = { connId: string; x: number; y: number };

/** Closes a menu or popover on Escape or a click outside it. */
function useDismiss(active: boolean, close: () => void) {
  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    const onDown = (e: MouseEvent) => !(e.target as Element).closest("[data-floating]") && close();
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousedown", onDown);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onDown);
    };
  }, [active, close]);
}

async function duplicate(conn: Conn) {
  // The copy has the same settings and no passwords; it opens in the edit form.
  const id = await saveConnection(
    "",
    { name: t("{name} copy", { name: conn.name }), engine: conn.engine, environment: conn.environment, colour: conn.colour, config: conn.config },
    { keepPassword: true, keepSshSecret: true, unchangedIfEmpty: false },
  );
  openDialog({ kind: "form", driverId: conn.driver.id, engine: conn.engine, editId: id });
}

function ContextMenu({ conn, at, close }: { conn: Conn; at: Floating; close(): void }) {
  const item = "flex h-[26px] w-full items-center justify-between px-s3 text-left hover:bg-accent-tint";
  const act = (run: () => void) => () => {
    close();
    run();
  };
  const connected = conn.session !== null;
  return (
    <div
      data-floating
      role="menu"
      className="fixed z-40 w-[220px] border border-line bg-panel py-s1"
      style={{ left: at.x, top: Math.min(at.y, window.innerHeight - 180), boxShadow: "var(--shadow-md)" }}
    >
      {/* One item, never both. */}
      <button type="button" role="menuitem" className={item} onClick={act(() => (connected ? disconnect(conn.id) : connect(conn.id)))}>
        {connected ? t("Disconnect") : t("Connect")}
      </button>
      <button type="button" role="menuitem" className={item} onClick={act(() => newQueryTab(conn.id))}>
        {t("New query")} <span className="font-mono text-[11px] text-text-3">{isMac ? "⌘T" : "Ctrl T"}</span>
      </button>
      <i className="my-s1 block h-px bg-line" />
      <button type="button" role="menuitem" className={item} onClick={act(() => openDialog({ kind: "form", driverId: conn.driver.id, engine: conn.engine, editId: conn.id }))}>
        {t("Edit…")}
      </button>
      <button type="button" role="menuitem" className={item} onClick={act(() => void duplicate(conn))}>
        {t("Duplicate")}
      </button>
      <i className="my-s1 block h-px bg-line" />
      <button type="button" role="menuitem" className={`${item} text-danger hover:!bg-danger-tint`} onClick={act(() => openDialog({ kind: "remove", connId: conn.id }))}>
        {t("Remove…")}{" "}<span className="font-mono text-[11px]">⌫</span>
      </button>
    </div>
  );
}

function ErrorPopover({ conn, at, close }: { conn: Conn; at: Floating; close(): void }) {
  if (!conn.lastError) return null;
  const link = "text-accent-text hover:underline";
  const time = new Date(conn.lastError.time).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return (
    <div
      data-floating
      role="dialog"
      aria-label={t("{connection} · Connection failed", { connection: conn.name })}
      className="fixed z-40 w-[400px] border border-line bg-panel"
      style={{ left: at.x, top: Math.min(at.y, window.innerHeight - 220), boxShadow: "var(--shadow-md)" }}
    >
      <div className="flex items-center gap-s2 border-b border-line-2 px-s3 py-s2">
        <TriangleAlert size={14} className="text-danger" />
        <span className="font-medium">{t("{connection} · Connection failed", { connection: conn.name })}</span>
        <span className="ml-auto font-mono text-[11px] text-text-3">{time}</span>
      </div>
      <p className="selectable max-h-[160px] overflow-auto px-s3 py-s3 font-mono text-[12px] break-words whitespace-pre-wrap">{conn.lastError.text}</p>
      <div className="flex gap-s4 border-t border-line-2 px-s3 py-s2 text-[12.5px]">
        <button type="button" className={link} onClick={() => { close(); void connect(conn.id); }}>
          {t("Retry")}
        </button>
        <button type="button" className={link} onClick={() => { close(); openDialog({ kind: "form", driverId: conn.driver.id, engine: conn.engine, editId: conn.id }); }}>
          {t("Edit connection")}
        </button>
        <button type="button" className={link} onClick={() => void navigator.clipboard?.writeText(conn.lastError?.text ?? "")}>
          {t("Copy error")}
        </button>
      </div>
    </div>
  );
}

export function Sidebar() {
  const connections = useApp((s) => s.connections);
  const activeId = useApp((s) => s.activeConnectionId);
  const [filter, setFilter] = useState("");
  const [menu, setMenu] = useState<Floating | null>(null);
  const [details, setDetails] = useState<Floating | null>(null);
  const needle = filter.trim().toLowerCase();
  const active = connections.find((c) => c.id === activeId) ?? null;
  const find = (id?: string) => connections.find((c) => c.id === id);

  useDismiss(!!menu, () => setMenu(null));
  useDismiss(!!details, () => setDetails(null));

  /** Double-click or Enter: connect, or show why the last attempt failed. */
  function open(conn: Conn, row: HTMLElement) {
    if (conn.status === "failed") showDetails(conn, row);
    else if (conn.session === null) void connect(conn.id);
  }
  function showDetails(conn: Conn, row: HTMLElement) {
    const box = row.getBoundingClientRect();
    setDetails({ connId: conn.id, x: box.right + 6, y: box.top });
  }

  return (
    <aside className="relative flex min-h-0 flex-col border-r border-line">
      <Splitter
        orientation="vertical"
        className="-right-[3px]"
        onDrag={(dx) => {
          const next = useApp.getState().sidebarWidth + dx;
          // Dragged well past the minimum: it folds into the rail.
          if (next < 160) setSidebarCollapsed(true);
          else useApp.setState({ sidebarWidth: Math.min(520, Math.max(200, next)) });
        }}
      />
      <div className="p-s2">
        <label className="flex h-[26px] items-center gap-s2 border border-line bg-panel px-s2 text-[12px] text-text-3 focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-accent">
          <Search size={13} />
          <input
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder={t("Filter connections and objects")}
            aria-label={t("Filter connections and objects")}
            className="min-w-0 flex-1 bg-transparent text-text placeholder:text-text-3 focus:outline-none"
          />
        </label>
      </div>

      <div className="flex items-center justify-between py-s1 pr-s2 pl-s3">
        <span className="label">{t("Connections")}</span>
        <IconButton label={`${t("New connection")} (${isMac ? "⌘N" : "Ctrl+N"})`} size={22} onClick={() => openDialog({ kind: "picker" })}>
          <Plus size={14} />
        </IconButton>
      </div>
      {connections.length === 0 && (
        <p className="px-s3 py-s1 text-[12px] leading-[1.5] text-text-3">{t("No connections yet. Saved connections are listed here.")}</p>
      )}
      {connections
        .filter((c) => !needle || c.name.toLowerCase().includes(needle) || c.id === activeId)
        .map((conn) => {
          const highlighted = menu?.connId === conn.id || details?.connId === conn.id;
          return (
            <div
              key={conn.id}
              role="row"
              tabIndex={0}
              aria-selected={conn.id === activeId}
              className={`relative flex h-tree cursor-pointer items-center gap-s2 px-s2 hover:bg-hover ${conn.id === activeId ? "font-medium" : ""}`}
              onClick={() => selectConnection(conn.id)}
              onDoubleClick={(e) => open(conn, e.currentTarget)}
              onKeyDown={(e) => {
                if (e.key === "Enter") open(conn, e.currentTarget);
                if (e.key === "Backspace" || e.key === "Delete") openDialog({ kind: "remove", connId: conn.id });
                if (e.key === "F10" && e.shiftKey) {
                  const box = e.currentTarget.getBoundingClientRect();
                  setMenu({ connId: conn.id, x: box.left + 40, y: box.bottom });
                }
              }}
              onContextMenu={(e) => {
                e.preventDefault();
                selectConnection(conn.id);
                setDetails(null);
                setMenu({ connId: conn.id, x: e.clientX, y: e.clientY });
              }}
            >
              {highlighted && <i className="pointer-events-none absolute inset-0 bg-accent-tint" />}
              <Chevron open={conn.id === activeId && conn.session !== null} />
              <span
                title={typeOf(conn.engine, conn.driver.id).name}
                className="grid h-[16px] w-[24px] flex-none place-items-center border border-line font-mono text-[10px] leading-none font-medium text-text-2"
                style={{ borderLeft: `3px solid ${colourVar(conn.colour)}` }}
              >
                {conn.engine}
              </span>
              <span className="min-w-0 flex-1 truncate">{conn.name}</span>
              <EnvTag environment={conn.environment} />
              <StatusSlot conn={conn} onDetails={() => setDetails((d) => (d?.connId === conn.id ? null : { connId: conn.id, x: 0, y: 0 }))} />
            </div>
          );
        })}

      <div className="relative z-[2] mt-s2 flex min-h-0 flex-1 flex-col border-t border-line">
        <RegMarks corners={["tr"]} />
        {active && (
          <>
            <div className="label truncate px-s3 pt-s3 pb-s1">{t("Objects")} · {active.name}</div>
            <div className="min-h-0 flex-1 overflow-y-auto">
              {active.session !== null && <ObjectTree conn={active} needle={needle} />}
              {active.session === null && (
                <p className="px-s3 py-s1 text-[12px] text-text-3">
                  {active.status === "connecting"
                    ? t("Connecting…")
                    : active.status === "lost"
                      ? t("The connection was lost. Double-click it to reconnect.")
                      : t("Not connected. Double-click the connection to open it.")}
                </p>
              )}
            </div>
          </>
        )}
      </div>

      {menu && find(menu.connId) && <ContextMenu conn={find(menu.connId)!} at={menu} close={() => setMenu(null)} />}
      {details && find(details.connId) && (
        <ErrorPopover
          conn={find(details.connId)!}
          // Beside the sidebar when opened from the status mark, which has no row position.
          at={details.x ? details : { ...details, x: useApp.getState().sidebarWidth + 6, y: 120 }}
          close={() => setDetails(null)}
        />
      )}
    </aside>
  );
}

function ObjectTree({ conn, needle }: { conn: Conn; needle: string }) {
  const expanded = useApp((s) => s.expanded);
  const [selected, setSelected] = useState<string | null>(null);
  // Drivers with schemas show them under the database; the others list their databases directly.
  const nested = conn.driver.capabilities.schemas;
  const base = nested ? 22 : 6.8;
  const namespaces = conn.namespaces.length > 0 ? conn.namespaces : [""];
  const rootKey = nodeKey(conn.id);
  const rootOpen = !nested || expanded[rootKey];

  function toggleNamespace(namespace: string) {
    const key = nodeKey(conn.id, namespace);
    if (!expanded[key] && !conn.objects[namespace]) void loadObjects(conn.id, namespace);
    toggleNode(key);
  }

  return (
    <div role="tree">
      <SavedQueries conn={conn} needle={needle} />
      {nested && (
        <TreeRow indent={6.8} onClick={() => toggleNode(rootKey)}>
          <Chevron open={!!rootOpen} />
          <Database size={13} className="text-text-3" />
          {/* A file-based database is named by its file; the whole path is in the tooltip. */}
          <span className="truncate font-mono text-[12px]" title={conn.config.database ?? undefined}>
            {conn.driver.capabilities.file_based ? (conn.config.database ?? "").split(/[\\/]/).pop() || conn.name : conn.config.database || conn.name}
          </span>
        </TreeRow>
      )}
      {rootOpen &&
        namespaces.map((namespace) => {
          const open = !!expanded[nodeKey(conn.id, namespace)];
          const objects = conn.objects[namespace];
          if (conn.driver.model === "graph") return <GraphTree key={namespace} conn={conn} database={namespace} indent={base} needle={needle} />;
          // A key-value store has no tables: its databases open the key list.
          if (conn.driver.model === "key_value") {
            return (
              <TreeRow key={namespace} indent={base} onDoubleClick={() => openKeys(conn.id, Number(/\d+/.exec(namespace)?.[0] ?? 0))}>
                <Database size={13} className="text-text-3" />
                <span className="truncate font-mono text-[12px]">{namespace}</span>
              </TreeRow>
            );
          }
          return (
            <div key={namespace}>
              <TreeRow indent={base} onClick={() => toggleNamespace(namespace)}>
                <Chevron open={open} />
                {nested ? <Layers size={13} className="text-text-3" /> : <Database size={13} className="text-text-3" />}
                <span className="truncate font-mono text-[12px]">{namespace || "main"}</span>
              </TreeRow>
              {open &&
                (["tables", "views"] as const).map((group) => {
                  const groupKey = nodeKey(conn.id, namespace, group);
                  const groupOpen = !!expanded[groupKey];
                  const items = (objects ?? []).filter((o) => (group === "views" ? o.kind === "view" : o.kind !== "view"));
                  const shown = items.filter((o) => !needle || o.name.toLowerCase().includes(needle));
                  return (
                    <div key={group}>
                      <TreeRow indent={base + 14} muted onClick={() => toggleNode(groupKey)}>
                        <Chevron open={groupOpen} />
                        <span className="flex-1">{group === "views" ? t("Views") : conn.driver.model === "vector" || conn.driver.model === "document" ? t("Collections") : t("Tables")}</span>
                        <span className="font-mono text-[11px] text-text-3">{objects ? items.length : "…"}</span>
                      </TreeRow>
                      {groupOpen &&
                        shown.map((object) => {
                          const key = nodeKey(conn.id, namespace, group, object.name);
                          return (
                            <TreeRow
                              key={object.name}
                              indent={base + 32}
                              selected={selected === key}
                              onClick={() => setSelected(key)}
                              onDoubleClick={() => (conn.driver.model === "vector" ? openVector(conn.id, object.name) : conn.driver.model === "document" ? openCollection(conn.id, namespace, object.name) : openTable(conn.id, namespace, object.name))}
                            >
                              <Table size={13} className="text-text-3" />
                              <span className="flex-1 truncate font-mono text-[12px]">{object.name}</span>
                              {object.row_estimate !== null && (
                                <span className="font-mono text-[11px] text-text-3" title={t("About {count} rows (the database's estimate)", { count: object.row_estimate.toLocaleString() })}>
                                  {compactCount(object.row_estimate)}
                                </span>
                              )}
                            </TreeRow>
                          );
                        })}
                    </div>
                  );
                })}
            </div>
          );
        })}
    </div>
  );
}

const cypherName = (name: string) => `\`${name.replace(/`/g, "``")}\``;

/**
 * A graph database in the tree: its node labels and relationship types, read
 * when the database is first opened. Double-clicking one opens a graph tab
 * with a query for it (not drawn in the design).
 */
function GraphTree({ conn, database, indent, needle }: { conn: Conn; database: string; indent: number; needle: string }) {
  const expanded = useApp((s) => s.expanded);
  const key = nodeKey(conn.id, database);
  const open = !!expanded[key];
  const [names, setNames] = useState<[GraphLabel[], GraphLabel[]] | "error" | null>(null);

  useEffect(() => {
    if (!open || names !== null || conn.session === null) return;
    api.graphLabels(conn.session, database).then(setNames, () => setNames("error"));
  }, [open, names, conn.session, database]);

  // A database other than the connection's own is named in the query itself.
  const use = database && database !== conn.namespace ? `USE ${cypherName(database)}\n` : "";
  const groups = [
    { id: "labels", title: t("Node labels"), items: names && names !== "error" ? names[0] : [], query: (name: string) => `${use}MATCH (n:${cypherName(name)}) RETURN n LIMIT 50` },
    { id: "types", title: t("Relationship types"), items: names && names !== "error" ? names[1] : [], query: (name: string) => `${use}MATCH (a)-[r:${cypherName(name)}]->(b) RETURN a, r, b LIMIT 50` },
  ];

  return (
    <div>
      <TreeRow indent={indent} onClick={() => toggleNode(key)} onDoubleClick={() => openGraph(conn.id, `${use}MATCH (a)-[r]->(b) RETURN a, r, b LIMIT 50`)}>
        <Chevron open={open} />
        <Database size={13} className="text-text-3" />
        <span className="truncate font-mono text-[12px]">{database}</span>
      </TreeRow>
      {open && names === "error" && <p className="py-[3px] text-[12px] text-danger" style={{ paddingLeft: indent + 32 }}>{t("Could not read the labels.")}</p>}
      {open &&
        names !== "error" &&
        groups.map((group) => {
          const groupKey = nodeKey(conn.id, database, group.id);
          const groupOpen = !!expanded[groupKey];
          const shown = group.items.filter((item) => !needle || item.name.toLowerCase().includes(needle));
          return (
            <div key={group.id}>
              <TreeRow indent={indent + 14} muted onClick={() => toggleNode(groupKey)}>
                <Chevron open={groupOpen} />
                <span className="flex-1">{group.title}</span>
                <span className="font-mono text-[11px] text-text-3">{names ? group.items.length : "…"}</span>
              </TreeRow>
              {groupOpen &&
                shown.map((item) => (
                  <TreeRow key={item.name} indent={indent + 32} onDoubleClick={() => openGraph(conn.id, group.query(item.name))}>
                    {group.id === "labels" ? <Circle size={11} className="text-text-3" /> : <ArrowRight size={12} className="text-text-3" />}
                    <span className="flex-1 truncate font-mono text-[12px]">{item.name}</span>
                    {item.count !== null && <span className="font-mono text-[11px] text-text-3">{compactCount(item.count)}</span>}
                  </TreeRow>
                ))}
            </div>
          );
        })}
    </div>
  );
}

/** The connection's saved queries: collapsed until opened, double-click opens one in a tab. */
function SavedQueries({ conn, needle }: { conn: Conn; needle: string }) {
  const key = nodeKey(conn.id, "\u0001saved");
  const open = useApp((s) => !!s.expanded[key]);
  const queries = conn.savedQueries.filter((q) => !needle || q.name.toLowerCase().includes(needle));
  return (
    <>
      <TreeRow indent={6.8} muted onClick={() => toggleNode(key)}>
        <Chevron open={open} />
        <Bookmark size={13} className="text-text-3" />
        <span className="flex-1">{t("Saved queries")}</span>
        <span className="font-mono text-[11px] text-text-3">{conn.savedQueries.length}</span>
      </TreeRow>
      {open &&
        queries.map((query) => (
          <TreeRow key={query.id} indent={38.8} onDoubleClick={() => openSavedQuery(conn.id, query)}>
            <Terminal size={13} className="text-text-3" />
            <span className="truncate">{query.name}</span>
          </TreeRow>
        ))}
      {open && conn.savedQueries.length === 0 && <p className="py-s1 pr-s3 pl-[38.8px] text-[12px] text-text-3">{t("Save a query to list it here.")}</p>}
    </>
  );
}

/** N02: the sidebar as a 40px rail, one badge per connection. */
export function SidebarRail() {
  const connections = useApp((s) => s.connections);
  const activeId = useApp((s) => s.activeConnectionId);
  const expandKeys = isMac ? "⌘B" : "Ctrl+B";
  return (
    <aside className="flex min-h-0 flex-col items-center gap-[6px] border-r border-line py-s2" aria-label={t("Connections")}>
      <IconButton label={`${t("Show sidebar")} (${expandKeys})`} onClick={() => setSidebarCollapsed(false)}>
        <PanelLeftOpen size={14} />
      </IconButton>
      <i className="h-px w-[24px] bg-line" />
      <div className="flex min-h-0 flex-col items-center gap-[6px] overflow-y-auto">
        {connections.map((conn) => (
          <button
            key={conn.id}
            type="button"
            title={`${conn.name} · ${conn.session !== null ? t("Connected") : t("Disconnected")}`}
            aria-label={conn.name}
            aria-current={conn.id === activeId ? "true" : undefined}
            className="relative grid h-[22px] w-[28px] flex-none place-items-center border-y border-r border-l-[3px] border-line font-mono text-[10.5px] leading-none hover:bg-hover"
            style={{ borderLeftColor: colourVar(conn.colour), ...(conn.id === activeId ? { outline: "1.5px solid var(--accent)", outlineOffset: 1 } : {}) }}
            onClick={() => {
              selectConnection(conn.id);
              setSidebarCollapsed(false);
            }}
          >
            {conn.engine}
            {conn.session !== null && <i className="absolute -right-[3px] -bottom-[3px] size-[7px] bg-ok" />}
          </button>
        ))}
      </div>
      <IconButton label={`${t("New connection")} (${isMac ? "⌘N" : "Ctrl+N"})`} onClick={() => openDialog({ kind: "picker" })}>
        <Plus size={14} />
      </IconButton>
    </aside>
  );
}
