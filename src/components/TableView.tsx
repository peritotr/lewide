// A table tab: its rows a page at a time (T01, T13–T19), or its structure.
// Cells are edited in place (T05, T08); edits, added rows and rows marked for
// deletion stay pending (T09) until they are applied in one transaction (T10–T12).
// Filters are built from conditions (T02) or written as WHERE text (T03).
import { ArrowDown, ArrowUp, Check, ChevronDown, ChevronLeft, ChevronRight, Columns3, Download, Filter, Lock, Minus, Plus, RefreshCw, TriangleAlert, Upload, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { api, compactCount, displayValue, type ColumnDescription, type ExportFormat, type QueryResult, type Value } from "../lib/api";
import { pendingCount, pendingCounts, rowId, rowKey } from "../lib/edit";
import { conditionSql, conditionValueText, OPERATOR_SYMBOL, operatorsFor, toWhere, valueCount, type Condition, type Operator } from "../lib/filter";
import { dialectOf, quoteIdent, type SqlDialect } from "../lib/sql";
import {
  openTable,
  addTableRow,
  countTable,
  dismissApply,
  loadPage,
  loadStructure,
  openDialog,
  previewTableChanges,
  removeTableInsert,
  requestApply,
  setFilterMode,
  setHiddenColumns,
  setInsertCell,
  setTableCell,
  showTableMode,
  toggleTableRowDelete,
  undoTableEdit,
  type Conn,
  type Tab,
  type TableView as View,
} from "../store";
import { DataGrid, type RowMeta } from "./DataGrid";
import { Button, Segmented, Strip } from "./dialog";
import { anchorOf, Floating, MenuItem, type Anchor } from "./menu";
import { errorText, Spinner } from "./Results";
import { StructureView } from "./StructureView";
import { TabBar } from "./TabBar";
import { IconButton, Tag } from "./ui";
import { msg, t, tn } from "../i18n";

const PAGE_SIZES = [100, 500, 1000, 5000];

const ghost = "flex h-[26px] flex-none items-center gap-[6px] px-s2 text-[12.5px] whitespace-nowrap enabled:hover:bg-hover disabled:cursor-not-allowed disabled:opacity-45";
const divider = <i className="h-[16px] w-px flex-none bg-line" />;
// Not drawn in the design: in a narrow pane the toolbar keeps its icons and drops their words.
const wide = "@max-[720px]:hidden";

export const qualifiedName = (view: View) => [view.namespace, view.name].filter(Boolean).join(".");

type RowRef = { kind: "insert"; id: number } | { kind: "row"; index: number };

export function TableView({ tab, view, conn }: { tab: Tab; view: View; conn: Conn | null }) {
  const production = conn?.environment === "production";
  const object = conn?.objects[view.namespace]?.find((o) => o.name === view.name);
  const isView = object?.kind === "view";
  const structure = view.mode === "structure";
  const keyed = !!view.page?.columns.some((c) => c.primary_key);
  const canEdit = keyed && !isView && view.apply.state !== "running";
  // The row of the active cell: an added row by its id, or a loaded row by its index in the page.
  const [active, setActive] = useState<RowRef | null>(null);
  const [columnsMenu, setColumnsMenu] = useState<Anchor | null>(null);
  const deleteActive = () => {
    if (!active) return;
    if (active.kind === "insert") removeTableInsert(tab.id, active.id);
    else toggleTableRowDelete(tab.id, active.index);
  };

  return (
    // One column no wider than the pane, so a long toolbar cannot widen the whole view.
    <main className="grid min-h-0 min-w-0 grid-cols-[minmax(0,1fr)] bg-panel" style={{ gridTemplateRows: `34px ${production ? "4px" : "0px"} 36px minmax(0,1fr)` }}>
      <TabBar />
      <div title={production ? t("Production connection") : undefined} className={production ? "production-band" : ""} />

      <div className="@container flex min-w-0 items-center gap-[10.2px] overflow-hidden border-b border-line px-[10.2px] whitespace-nowrap">
        <Segmented
          label={t("View")}
          value={view.mode}
          onChange={(mode) => showTableMode(tab.id, mode)}
          options={[
            { value: "data", label: t("Data") },
            { value: "structure", label: t("Structure") },
          ]}
        />
        <span className="selectable min-w-0 truncate font-mono text-[12.5px] font-medium">{qualifiedName(view)}</span>
        {structure ? (
          <>
            <span className="min-w-0 truncate text-[12px] text-text-2">{isView ? t("View · read-only") : object?.row_estimate != null ? t("Table · about {count} rows", { count: compactCount(object.row_estimate) }) : t("Table")}</span>
            <button type="button" className={`${ghost} ml-auto`} disabled={view.structure?.state === "loading"} onClick={() => loadStructure(tab.id)}>
              <RefreshCw size={13} />
              {t("Refresh")}
            </button>
          </>
        ) : (
          <div className="ml-auto flex items-center gap-[2px]">
            <button type="button" className={ghost} disabled={!canEdit} title={t("Add row")} onClick={() => addTableRow(tab.id)}>
              <Plus size={13} />
              <span className={wide}>{t("Add row")}</span>
            </button>
            <button type="button" className={ghost} disabled={!canEdit || !active} title={canEdit && !active ? t("Select a cell in the row first.") : t("Delete row")} onClick={deleteActive}>
              <Minus size={13} />
              <span className={wide}>{t("Delete row")}</span>
            </button>
            {divider}
            <button type="button" className={ghost} disabled={view.loading} title={t("Refresh")} onClick={() => loadPage(tab.id)}>
              <RefreshCw size={13} />
              <span className={wide}>{t("Refresh")}</span>
            </button>
            <button type="button" className={ghost} disabled={!view.page || !conn || isView} title={t("Import")} onClick={() => conn && openDialog({ kind: "import", connId: conn.id, tabId: tab.id })}>
              <Upload size={13} />
              <span className={wide}>{t("Import")}</span>
            </button>
            <button type="button" className={ghost} disabled={!view.page || !conn} title={t("Export")} onClick={() => conn && openDialog({ kind: "export", connId: conn.id, source: { kind: "table", tabId: tab.id } })}>
              <Download size={13} />
              <span className={wide}>{t("Export")}</span>
            </button>
            <button type="button" className={ghost} disabled={!view.page} title={t("Columns")} aria-haspopup="menu" onClick={(e) => setColumnsMenu(anchorOf(e.currentTarget))}>
              <Columns3 size={13} />
              <span className={wide}>{t("Columns")}</span>
              {view.hidden.length > 0 && <span className="font-mono text-[11px] text-text-3">{(view.page?.columns.length ?? 0) - view.hidden.length}/{view.page?.columns.length}</span>}
            </button>
            {columnsMenu && view.page && (
              // Not drawn in the design: a checked list, as in the other menus.
              <Floating anchor={columnsMenu} width={240} alignRight onClose={() => setColumnsMenu(null)} label={t("Columns")}>
                <div className="max-h-[320px] overflow-auto">
                  {view.page.columns.map((column) => {
                    const shown = !view.hidden.includes(column.name);
                    return (
                      <MenuItem
                        key={column.name}
                        checked={shown}
                        // The last visible column stays.
                        disabled={shown && view.hidden.length === view.page!.columns.length - 1}
                        onSelect={() => setHiddenColumns(tab.id, shown ? [...view.hidden, column.name] : view.hidden.filter((name) => name !== column.name))}
                      >
                        <span className="font-mono text-[12px]">{column.name}</span>
                      </MenuItem>
                    );
                  })}
                </div>
                {view.hidden.length > 0 && (
                  <MenuItem onSelect={() => setHiddenColumns(tab.id, [])}>{t("Show all columns")}</MenuItem>
                )}
              </Floating>
            )}
          </div>
        )}
      </div>

      {structure ? <StructureView tab={tab} view={view} conn={conn} isView={isView} /> : <DataView tab={tab} view={view} conn={conn} estimate={object?.row_estimate ?? null} isView={isView} canEdit={canEdit} onActive={setActive} />}
    </main>
  );
}

function DataView({ tab, view, conn, estimate, isView, canEdit, onActive }: { tab: Tab; view: View; conn: Conn | null; estimate: number | null; isView: boolean; canEdit: boolean; onActive(row: RowRef | null): void }) {
  const { page, pending } = view;
  const first = view.offset + 1;
  const last = view.offset + (page?.rows.length ?? 0);
  // The grid shows the page as it will be: added rows on top, edits written over the loaded values.
  const model = useMemo(() => {
    if (!page) return null;
    const columns = page.columns;
    // Hidden columns are left out of the grid only; edits and keys use the whole row.
    const visible = columns.map((_, i) => i).filter((i) => !view.hidden.includes(columns[i].name));
    const pick = <T,>(items: T[]) => visible.map((i) => items[i]);
    const rows: Value[][] = [];
    const meta: RowMeta[] = [];
    const refs: RowRef[] = [];
    for (const insert of pending.inserts) {
      rows.push(pick(columns.map((c): Value => insert.values[c.name] ?? { type: "null" })));
      meta.push({ mark: "+", changed: null, original: null, unset: pick(columns.map((c) => !(c.name in insert.values))), number: null });
      refs.push({ kind: "insert", id: insert.id });
    }
    page.rows.forEach((row, index) => {
      const key = rowKey(columns, row);
      const id = key && rowId(key);
      const update = id ? pending.updates[id] : undefined;
      const deleted = !!id && id in pending.deletes;
      rows.push(pick(update ? row.map((value, c) => update.set[columns[c].name] ?? value) : row));
      meta.push({
        mark: deleted ? "−" : update ? "~" : null,
        changed: update ? pick(columns.map((c) => c.name in update.set)) : null,
        original: update ? pick(row) : null,
        unset: null,
        number: view.offset + index + 1,
      });
      refs.push({ kind: "row", index });
    });
    const shown = pick(columns);
    const result: QueryResult = { columns: shown.map((c) => ({ name: c.name, type_name: c.type_name })), rows, rows_affected: null, truncated: false, elapsed_ms: page.elapsed_ms };
    return { result, meta, refs, shown, visible };
  }, [page, pending, view.offset, view.hidden]);
  const keys = useMemo(() => page?.columns.filter((c) => c.primary_key).map((c) => c.name) ?? [], [page]);
  // Rows picked by the gutter, as indices into the grid's rows. A new page starts with none.
  const [selected, setSelected] = useState<Set<number>>(new Set());
  useEffect(() => setSelected(new Set()), [page, pending.inserts.length]);
  // Foreign keys come with the structure, which the data view reads once for them.
  useEffect(() => {
    if (page && !view.structure) void loadStructure(tab.id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [page === null, tab.id]);
  const keysOut = view.structure?.state === "ready" ? view.structure.data.foreign_keys : [];
  /** Where a column's values point: the table and the column there. Single-column keys only. */
  const targetOf = (column: string) => {
    const key = keysOut.find((k) => k.columns.length === 1 && k.columns[0] === column && k.referenced_columns.length === 1);
    return key ? { key, namespace: key.referenced_namespace ?? view.namespace, table: key.referenced_table, column: key.referenced_columns[0] } : null;
  };
  const linkFilter = (column: string, value: Value) => {
    const target = targetOf(column);
    if (!target || !conn) return null;
    const text = value.type === "null" ? "" : "value" in value ? String(value.value) : "";
    const condition: Condition = { column: target.column, operator: "=", value: text, value2: "" };
    // The value's own kind says how it is written: a number bare, anything else quoted.
    const as: ColumnDescription = { name: target.column, type_name: value.type === "int" || value.type === "decimal" || value.type === "float" ? "numeric" : "text", nullable: true, default_value: null, primary_key: false };
    return { target, condition, filter: toWhere([condition], [as], dialectOf(conn.driver.id)) };
  };
  const [peek, setPeek] = useState<{ anchor: Anchor; column: string; value: Value } | null>(null);
  const follow = (column: string, value: Value) => {
    const link = linkFilter(column, value);
    if (link && conn) openTable(conn.id, link.target.namespace, link.target.table, "data", { conditions: [link.condition], filter: link.filter });
  };
  const filtered = view.filter !== "";
  const empty = model !== null && model.result.rows.length === 0 && view.offset === 0;
  const readOnly = isView
    ? t("Read-only. This is a view, so its rows can't be edited here.")
    : page && conn?.driver.id === "clickhouse"
      ? t("Read-only. A ClickHouse key does not identify one row; change rows with a statement in a query tab.")
      : page && keys.length === 0
        ? t("Read-only. This table has no primary key, so a row can't be identified for editing.")
        : null;

  // ⌘S applies, as the pending bar says.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && e.key === "s") {
        e.preventDefault();
        requestApply(tab.id);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [tab.id]);

  return (
    <div className="grid min-h-0 grid-cols-[minmax(0,1fr)]" style={{ gridTemplateRows: `36px auto minmax(0,1fr) auto 30px` }}>
      <FilterBar tab={tab} view={view} columns={page?.columns ?? []} dialect={dialectOf(conn?.driver.id ?? "")} />

      <div>
        {readOnly && (
          <Strip kind="neutral" icon={<Lock size={13} />} className="border-x-0 border-t-0">
            {readOnly}
          </Strip>
        )}
      </div>

      {view.error ? (
        <LoadError message={view.error} conn={conn} onRetry={() => loadPage(tab.id)} />
      ) : !model ? (
        <Loading text={t("Loading {name}", { name: qualifiedName(view) })} />
      ) : (
        <div className={`relative grid min-h-0 ${view.loading ? "opacity-60" : ""}`}>
          <DataGrid
            // A new page starts at the top; the same page keeps its column widths.
            key={`${tab.id}:${view.offset}:${view.pageSize}:${view.filter}:${view.hidden.join(",")}`}
            result={model.result}
            firstRow={first}
            keys={keys}
            serverSort={view.sort}
            onSort={(sort) => loadPage(tab.id, { sort, offset: 0 })}
            selection={{ selected, onChange: setSelected }}
            note={canEdit ? t("Editable · Enter to edit") : t("Read-only")}
            linked={model.shown.map((c) => targetOf(c.name) !== null)}
            actions={{
              onFollowLink: follow,
              onPeekLink: (column, value, anchor) => setPeek({ anchor, column, value }),
              onHideColumn: (name) => setHiddenColumns(tab.id, [...view.hidden, name]),
              onFilterColumn: (name) => window.dispatchEvent(new CustomEvent("lewide:filter", { detail: { tabId: tab.id, column: name } })),
              onFilterValue: (name, value) =>
                window.dispatchEvent(new CustomEvent("lewide:filter", { detail: { tabId: tab.id, column: name, value: value.type === "null" ? null : value.type === "json" ? JSON.stringify(value.value) : "value" in value ? String(value.value) : "" } })),
            }}
            edit={
              canEdit
                ? {
                    columns: model.shown,
                    meta: model.meta,
                    onChange: (row, column, value) => {
                      const ref = model.refs[row];
                      if (ref.kind === "insert") setInsertCell(tab.id, ref.id, model.shown[column].name, value);
                      else setTableCell(tab.id, ref.index, model.visible[column], value);
                    },
                    onUndo: () => undoTableEdit(tab.id),
                    onActiveRow: (row) => onActive(row === null ? null : (model.refs[row] ?? null)),
                  }
                : undefined
            }
          />
          {empty && (
            <div className="pointer-events-none absolute inset-x-0 top-[calc(var(--row-h)+40px)] flex flex-col items-center gap-s2 text-center">
              <h2 className="font-heading text-[20px] leading-[1.2] font-semibold">{filtered ? t("No rows match these filters") : t("This table is empty")}</h2>
              <p className="text-[12.5px] text-text-2">
                {filtered
                  ? estimate !== null
                    ? t("A filter is active. The table itself has about {count} rows.", { count: compactCount(estimate) })
                    : t("A filter is active.")
                  : t("It has columns but no rows yet.")}
              </p>
              {filtered && (
                <button type="button" className="pointer-events-auto text-[12.5px] text-accent-text hover:underline" onClick={() => loadPage(tab.id, { filter: "", offset: 0 })}>
                  {t("Clear filters")}
                </button>
              )}
            </div>
          )}
        </div>
      )}

      {peek &&
        conn &&
        (() => {
          const link = linkFilter(peek.column, peek.value);
          return link ? (
            <LinkPeek
              anchor={peek.anchor}
              conn={conn}
              namespace={link.target.namespace}
              table={link.target.table}
              column={link.target.column}
              constraint={link.target.key.name}
              filter={link.filter}
              valueText={link.condition.value}
              onOpen={() => {
                setPeek(null);
                follow(peek.column, peek.value);
              }}
              onClose={() => setPeek(null)}
            />
          ) : null;
        })()}
      {selected.size > 0 && model ? (
        <SelectionBar
          tab={tab}
          conn={conn}
          view={view}
          count={selected.size}
          range={[...selected].sort((x, y) => x - y)}
          rows={[...selected].sort((x, y) => x - y).map((i) => model.result.rows[i])}
          columns={model.result.columns}
          canEdit={canEdit}
          onDelete={() => {
            for (const index of selected) {
              const ref = model.refs[index];
              if (ref.kind === "insert") removeTableInsert(tab.id, ref.id);
              else if (model.meta[index].mark !== "−") toggleTableRowDelete(tab.id, ref.index);
            }
            setSelected(new Set());
          }}
          onClear={() => setSelected(new Set())}
        />
      ) : (
        <PendingBar tab={tab} view={view} />
      )}
      <Footer tab={tab} view={view} estimate={estimate} first={first} last={last} keys={keys} locked={readOnly !== null} />
    </div>
  );
}

/** T04: the row a foreign key value points at, shown before going there. */
function LinkPeek(props: { anchor: Anchor; conn: Conn; namespace: string; table: string; column: string; constraint: string | null; filter: string; valueText: string; onOpen(): void; onClose(): void }) {
  const [row, setRow] = useState<{ columns: string[]; values: Value[] } | "loading" | "missing" | { error: string }>("loading");
  useEffect(() => {
    if (props.conn.session === null) return;
    let current = true;
    api
      .fetchRows(props.conn.session, { namespace: props.namespace || undefined, table: props.table, offset: 0, limit: 1, filter: props.filter })
      .then((page) => current && setRow(page.rows.length > 0 ? { columns: page.columns.map((c) => c.name), values: page.rows[0] } : "missing"))
      .catch((e) => current && setRow({ error: String((e as { message?: unknown })?.message ?? e) }));
    return () => {
      current = false;
    };
  }, [props.conn.session, props.namespace, props.table, props.filter]);

  return (
    <Floating anchor={props.anchor} width={320} onClose={props.onClose} role="dialog" label={props.table} className="">
      <div className="flex flex-col gap-[2px] border-b border-line px-s3 py-s2">
        <span className="truncate font-mono text-[12px] font-medium">
          {props.table}.{props.column} = {props.valueText}
        </span>
        {props.constraint && <span className="truncate font-mono text-[11px] text-text-3">{props.constraint}</span>}
      </div>
      <div className="px-s3 py-s2 text-[12.5px]">
        {row === "loading" ? (
          <span className="flex items-center gap-s2 text-text-2">
            <Spinner />
            {t("Loading…")}
          </span>
        ) : row === "missing" ? (
          <span className="text-text-2">{t("No row with this key was found.")}</span>
        ) : "error" in row ? (
          <span className="selectable font-mono text-[12px] text-danger">{row.error}</span>
        ) : (
          <div className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-s3 gap-y-[3px] font-mono text-[12px]">
            {/* Up to three columns besides the key itself. */}
            {row.columns
              .map((name, i) => ({ name, value: row.values[i] }))
              .filter((c) => c.name !== props.column)
              .slice(0, 3)
              .map((c) => (
                <div key={c.name} className="contents">
                  <span className="text-text-3">{c.name}</span>
                  <span className="selectable truncate">{c.value.type === "null" ? <span className="text-text-3 italic">NULL</span> : displayValue(c.value)}</span>
                </div>
              ))}
          </div>
        )}
      </div>
      <button type="button" className="flex h-[28px] w-full items-center justify-between border-t border-line px-s3 text-[12.5px] text-accent-text hover:bg-hover" onClick={props.onOpen}>
        {t("Open row in {table}", { table: props.table })}
        <span className="font-mono text-[11px] text-text-3">{MOD_CLICK}</span>
      </button>
    </Floating>
  );
}

const MOD_CLICK = /Mac/i.test(navigator.userAgent) ? "⌘ click" : "Ctrl+click";

const COPY_FORMATS: { label: string; format: (table: string, dialect: SqlDialect) => ExportFormat }[] = [
  { label: "TSV", format: () => ({ format: "csv", delimiter: "\t" }) },
  { label: "CSV", format: () => ({ format: "csv" }) },
  { label: "JSON", format: () => ({ format: "json" }) },
  { label: "INSERT", format: (table, dialect) => ({ format: "sql", table, dialect }) },
];

/** T15: what can be done with the rows picked by the gutter. */
function SelectionBar(props: { tab: Tab; conn: Conn | null; view: View; count: number; range: number[]; rows: Value[][]; columns: QueryResult["columns"]; canEdit: boolean; onDelete(): void; onClear(): void }) {
  const [menu, setMenu] = useState<Anchor | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const dialect = dialectOf(props.conn?.driver.id ?? "");
  const names = props.columns.map((c) => c.name);
  const table = [props.view.namespace, props.view.name].filter(Boolean).map((part) => quoteIdent(part, dialect)).join(".");
  const first = props.view.offset + props.range[0] + 1;
  const last = props.view.offset + props.range[props.range.length - 1] + 1;
  const contiguous = props.range.length === last - first + 1;

  async function copyAs(index: number) {
    setMenu(null);
    const chosen = COPY_FORMATS[index];
    // The same code that writes export files, so the text is what an export would hold.
    const text = await api.exportPreview(names, props.rows, chosen.format(table, dialect)).catch(() => null);
    if (text === null) return;
    await navigator.clipboard?.writeText(text);
    setCopied(chosen.label);
    window.setTimeout(() => setCopied(null), 2000);
  }

  return (
    <div role="status" className="flex h-[36px] items-center gap-s3 border-t border-line bg-accent-tint px-[10.2px] text-[12.5px] whitespace-nowrap">
      <span className="font-medium">{tn(props.count, "{count} row selected", "{count} rows selected")}</span>
      {contiguous && props.count > 1 && <span className="font-mono text-[12px] text-text-2">{first}–{last}</span>}
      {copied && <span className="text-text-2">{t("Copied as {format}", { format: copied })}</span>}
      <span className="flex-1" />
      <button type="button" className={ghost} aria-haspopup="menu" onClick={(e) => setMenu(anchorOf(e.currentTarget))}>
        {t("Copy")}
        <ChevronDown size={12} />
      </button>
      {menu && (
        // The list of formats is not drawn in the design.
        <Floating anchor={menu} width={170} onClose={() => setMenu(null)} label={t("Copy")}>
          {COPY_FORMATS.map((f, i) => (
            <MenuItem key={f.label} onSelect={() => void copyAs(i)}>
              {t("as {format}", { format: f.label })}
            </MenuItem>
          ))}
        </Floating>
      )}
      <button
        type="button"
        className={ghost}
        disabled={!props.conn}
        onClick={() =>
          props.conn &&
          openDialog({
            kind: "export",
            connId: props.conn.id,
            source: { kind: "result", selection: true, title: props.view.name, result: { columns: props.columns, rows: props.rows, rows_affected: null, truncated: false, elapsed_ms: 0 } },
          })
        }
      >
        {t("Export…")}
      </button>
      <button type="button" className={`${ghost} border border-danger text-danger`} disabled={!props.canEdit} onClick={props.onDelete}>
        {t("Delete rows")}
      </button>
      <button type="button" className={ghost} onClick={props.onClear}>
        {t("Clear selection")}
      </button>
    </div>
  );
}

const MOD = /Mac/i.test(navigator.userAgent) ? "⌘" : "Ctrl+";

/** T09, T11, T12: what is pending, or how applying it went. One bar at a time. */
function PendingBar({ tab, view }: { tab: Tab; view: View }) {
  const { apply } = view;
  const count = pendingCount(view.pending);
  const parts = pendingCounts(view.pending);
  const discard = () => openDialog({ kind: "discard-changes", tabId: tab.id, then: null });

  // The success strip leaves by itself.
  useEffect(() => {
    if (apply.state !== "ok") return;
    const timer = window.setTimeout(() => dismissApply(tab.id), 6000);
    return () => window.clearTimeout(timer);
  }, [apply, tab.id]);

  if (apply.state === "ok" && count === 0) {
    return (
      <div role="status" className="flex h-[36px] items-center gap-s3 border-t border-ok bg-ok-tint px-[10.2px] text-[12.5px] whitespace-nowrap">
        <Check size={13} className="flex-none text-ok" />
        <span className="font-medium">{tn(apply.count, "{count} change applied", "{count} changes applied")}</span>
        <span className="font-mono text-[12px] text-text-2">{t("1 transaction · committed · {ms} ms", { ms: apply.ms })}</span>
        <button type="button" className="ml-auto text-accent-text hover:underline" onClick={() => dismissApply(tab.id)}>
          {t("Dismiss")}
        </button>
      </div>
    );
  }
  if (count === 0) return <div />;

  if (apply.state === "failed") {
    return (
      <div role="alert" className="flex flex-col gap-[4px] border-t border-danger bg-danger-tint px-[10.2px] py-s2 text-[12.5px]">
        <div className="flex items-center gap-s3 whitespace-nowrap">
          <TriangleAlert size={13} className="flex-none text-danger" />
          <span className="font-medium">{t("Apply failed. Nothing was changed.")}</span>
          <span className="min-w-0 truncate text-text-2">{tn(count, "The transaction was rolled back. Your change is still pending.", "The transaction was rolled back. Your {count} changes are still pending.")}</span>
          <span className="flex-1" />
          <button type="button" className={ghost} onClick={() => previewTableChanges(tab.id)}>
            {t("Preview SQL")}
          </button>
          <button type="button" className={ghost} onClick={() => requestApply(tab.id)}>
            {t("Retry")}
          </button>
          <button type="button" className={ghost} onClick={discard}>
            {t("Discard")}
          </button>
        </div>
        <p className="selectable pl-[21px] font-mono text-[12px] break-words whitespace-pre-wrap">{errorText(apply.error)}</p>
      </div>
    );
  }

  const running = apply.state === "running";
  return (
    <div role="status" className="flex h-[36px] items-center gap-s3 border-t border-line bg-accent-tint px-[10.2px] text-[12.5px] whitespace-nowrap">
      <span className="font-medium">{tn(count, "{count} change", "{count} changes")}</span>
      <span className="flex gap-s3 font-mono text-[12px]">
        {parts.added > 0 && <span className="text-ok">+ {t("{count} added", { count: parts.added })}</span>}
        {parts.modified > 0 && <span className="text-accent-text">~ {t("{count} modified", { count: parts.modified })}</span>}
        {parts.deleted > 0 && <span className="text-danger">− {t("{count} deleted", { count: parts.deleted })}</span>}
      </span>
      <span className="min-w-0 truncate text-text-2">{t("Not saved to the database yet")}</span>
      <span className="flex-1" />
      {running && <Spinner />}
      <button type="button" className={ghost} disabled={running} onClick={() => previewTableChanges(tab.id)}>
        {t("Preview SQL")}
      </button>
      <Button variant="primary" disabled={running} onClick={() => requestApply(tab.id)}>
        {t("Apply")}
        <span className="font-mono text-[11px] font-normal tracking-normal">{MOD}S</span>
      </Button>
      <button type="button" className={ghost} disabled={running} onClick={discard}>
        {t("Discard")}
      </button>
    </div>
  );
}

const OPERATOR_LABEL: Record<Operator, string> = {
  "=": msg("equals"),
  "<>": msg("is not"),
  ">": msg("greater than"),
  ">=": msg("at least"),
  "<": msg("less than"),
  "<=": msg("at most"),
  between: msg("between two values"),
  in: msg("one of a list"),
  like: msg("matches a pattern"),
  starts: msg("starts with"),
  null: msg("has no value"),
  not_null: msg("has a value"),
};

/** T02, T03: conditions built from lists, or the text that follows WHERE. */
function FilterBar({ tab, view, columns, dialect }: { tab: Tab; view: View; columns: ColumnDescription[]; dialect: SqlDialect }) {
  const [draft, setDraft] = useState(view.filter);
  // The condition being added (index = how many there are) or edited.
  const [editor, setEditor] = useState<{ anchor: Anchor; index: number } | null>(null);
  // The column a new condition starts on, when the grid asked for one.
  const [preset, setPreset] = useState<string | null>(null);
  const bar = useRef<HTMLFormElement>(null);
  // The grid's menus ask for a filter on a column, with or without a value.
  useEffect(() => {
    const onRequest = (e: Event) => {
      const detail = (e as CustomEvent<{ tabId: string; column: string; value?: string | null }>).detail;
      if (detail.tabId !== tab.id || !bar.current) return;
      if (detail.value !== undefined) {
        // A value is given: the condition is complete and is applied at once.
        const condition: Condition = detail.value === null ? { column: detail.column, operator: "null", value: "", value2: "" } : { column: detail.column, operator: "=", value: detail.value, value2: "" };
        const next = [...view.conditions, condition];
        void loadPage(tab.id, { conditions: next, filter: toWhere(next, columns, dialect), filterMode: "builder", offset: 0 });
      } else {
        if (view.filterMode !== "builder") return;
        setPreset(detail.column);
        setEditor({ anchor: bar.current.getBoundingClientRect(), index: view.conditions.length });
      }
    };
    window.addEventListener("lewide:filter", onRequest);
    return () => window.removeEventListener("lewide:filter", onRequest);
  });
  const where = view.filterMode === "where";
  const builderSql = toWhere(view.conditions, columns, dialect);
  // The builder can take over only while the applied text is what it would write itself.
  const mapped = view.filter === builderSql;
  const applyWhere = (filter: string) => loadPage(tab.id, { filter: filter.trim(), filterMode: "where", offset: 0 });
  const applyConditions = (conditions: Condition[]) => loadPage(tab.id, { conditions, filter: toWhere(conditions, columns, dialect), filterMode: "builder", offset: 0 });
  const clear = () => {
    setDraft("");
    void loadPage(tab.id, { conditions: [], filter: "", offset: 0 });
  };
  const count = where ? (view.filter ? 1 : 0) : view.conditions.length;

  return (
    <form
      ref={bar}
      className="flex items-center gap-s3 border-b border-line bg-bg px-[10.2px] whitespace-nowrap"
      onSubmit={(e) => {
        e.preventDefault();
        if (where) void applyWhere(draft);
      }}
    >
      <Filter size={13} className="flex-none text-text-3" />
      {where ? (
        <>
          <span className="font-mono text-[12px] font-medium text-accent-text">WHERE</span>
          <input
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            aria-label={t("Filter condition")}
            spellCheck={false}
            autoFocus
            className="h-[24px] min-w-0 flex-1 border border-line bg-panel px-s2 font-mono text-[12px] outline-none focus:border-accent"
          />
          <button type="submit" className={ghost} disabled={draft.trim() === view.filter}>
            {t("Apply")}
            <span className="font-mono text-[11px] text-text-3">↵</span>
          </button>
        </>
      ) : (
        <div className="flex min-w-0 flex-1 items-center gap-s2 overflow-x-auto">
          {view.conditions.length === 0 && <span className="text-[12.5px] text-text-3">{t("No filters")}</span>}
          {view.conditions.map((condition, i) => (
            <span key={i} className="flex flex-none items-center gap-s2">
              {i > 0 && <span className="label">AND</span>}
              <span className="flex h-[24px] items-center border border-line bg-panel font-mono text-[12px]">
                <button type="button" className="flex h-full items-center gap-[6px] px-s2 hover:bg-hover" title={t("Edit filter")} onClick={(e) => setEditor({ anchor: anchorOf(e.currentTarget), index: i })}>
                  <span>{condition.column}</span>
                  <span className="font-medium text-accent-text">{OPERATOR_SYMBOL[condition.operator]}</span>
                  {valueCount(condition.operator) > 0 && <span className="max-w-[220px] truncate">{conditionValueText(condition)}</span>}
                </button>
                <button type="button" aria-label={t("Remove filter")} title={t("Remove filter")} className="grid h-full w-[20px] place-items-center border-l border-line-2 text-text-3 hover:bg-hover" onClick={() => applyConditions(view.conditions.filter((_, j) => j !== i))}>
                  <X size={11} />
                </button>
              </span>
            </span>
          ))}
          <button type="button" className={`${ghost} text-accent-text`} disabled={columns.length === 0} onClick={(e) => setEditor({ anchor: anchorOf(e.currentTarget), index: view.conditions.length })}>
            <Plus size={12} />
            {t("Add filter")}
          </button>
        </div>
      )}
      {editor && (
        <ConditionEditor
          anchor={editor.anchor}
          columns={columns}
          dialect={dialect}
          initial={view.conditions[editor.index] ?? null}
          startColumn={preset}
          onClose={() => {
            setEditor(null);
            setPreset(null);
          }}
          onSave={(condition) => {
            setEditor(null);
            setPreset(null);
            const next = [...view.conditions];
            next[editor.index] = condition;
            void applyConditions(next);
          }}
        />
      )}

      {count > 0 && (
        <>
          <Tag kind="accent">{where ? "WHERE" : tn(count, "{count} filter", "{count} filters")}</Tag>
          <button type="button" className="text-[12.5px] text-accent-text hover:underline" onClick={clear}>
            {t("Clear")}
          </button>
        </>
      )}
      <Segmented
        label={t("Filter mode")}
        value={view.filterMode}
        onChange={(mode) => {
          if (mode === "where") setDraft(view.filter);
          setFilterMode(tab.id, mode);
        }}
        options={[
          { value: "builder", label: t("Builder"), disabled: !mapped, title: mapped ? undefined : t("This condition can't be shown as simple filters. Clear it to use the builder.") },
          { value: "where", label: "WHERE" },
        ]}
      />
      {view.sort && (
        <span className="flex h-[24px] flex-none items-center gap-[6px] border border-line px-s2 text-[12px]">
          {t("Sort")}
          <span className="font-mono">{view.sort.column}</span>
          {view.sort.descending ? <ArrowDown size={11} className="text-accent-text" /> : <ArrowUp size={11} className="text-accent-text" />}
        </span>
      )}
    </form>
  );
}

const select = "h-[28px] min-w-0 border border-line bg-panel px-s2 text-[12.5px] outline-none hover:border-text-3 focus:border-accent";

function ConditionEditor({ anchor, columns, dialect, initial, startColumn, onClose, onSave }: { anchor: Anchor; columns: ColumnDescription[]; dialect: SqlDialect; initial: Condition | null; startColumn?: string | null; onClose(): void; onSave(condition: Condition): void }) {
  const [condition, setCondition] = useState<Condition>(initial ?? { column: columns.find((c) => c.name === startColumn)?.name ?? columns[0].name, operator: "=", value: "", value2: "" });
  const column = columns.find((c) => c.name === condition.column) ?? columns[0];
  const operators = operatorsFor(column);
  const values = valueCount(condition.operator);
  const complete = conditionSql(condition, columns, dialect) !== null;
  const field = "h-[28px] min-w-0 flex-1 border border-line bg-panel px-s2 font-mono text-[12.5px] outline-none hover:border-text-3 focus:border-accent";
  const save = () => complete && onSave(condition);
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      e.stopPropagation();
      save();
    }
  };
  return (
    <Floating anchor={anchor} width={520} onClose={onClose} role="dialog" label={t("Filter")} className="">
      <div className="flex flex-col gap-s3 p-s4">
        <div className="flex items-end gap-s2">
          <label className="flex w-[190px] flex-none flex-col gap-[4px]">
            <span className="label">{t("Column")}</span>
            <select
              autoFocus
              className={`${select} font-mono`}
              value={condition.column}
              onChange={(e) => {
                const next = columns.find((c) => c.name === e.target.value)!;
                setCondition({ ...condition, column: next.name, operator: operatorsFor(next).includes(condition.operator) ? condition.operator : "=" });
              }}
            >
              {columns.map((c) => (
                <option key={c.name} value={c.name}>
                  {c.name}
                  {c.type_name ? ` · ${c.type_name}` : ""}
                </option>
              ))}
            </select>
          </label>
          <label className="flex w-[150px] flex-none flex-col gap-[4px]">
            <span className="label">{t("Operator")}</span>
            <select className={select} value={condition.operator} onChange={(e) => setCondition({ ...condition, operator: e.target.value as Operator })}>
              {operators.map((operator) => (
                <option key={operator} value={operator}>
                  {OPERATOR_SYMBOL[operator]} · {t(OPERATOR_LABEL[operator])}
                </option>
              ))}
            </select>
          </label>
          {values > 0 && (
            <label className="flex min-w-0 flex-1 flex-col gap-[4px]">
              <span className="label">{condition.operator === "in" ? t("Values, separated by commas") : t("Value")}</span>
              <span className="flex items-center gap-[6px]">
                <input className={field} value={condition.value} spellCheck={false} onChange={(e) => setCondition({ ...condition, value: e.target.value })} onKeyDown={onKey} />
                {values === 2 && (
                  <>
                    <span className="label">AND</span>
                    <input className={field} value={condition.value2} spellCheck={false} aria-label={t("Upper value")} onChange={(e) => setCondition({ ...condition, value2: e.target.value })} onKeyDown={onKey} />
                  </>
                )}
              </span>
            </label>
          )}
        </div>
        <p className="text-[12px] text-text-2">{t("Conditions are joined with AND. Use WHERE for OR and expressions.")}</p>
        <div className="flex justify-end gap-s2">
          <Button onClick={onClose}>{t("Cancel")}</Button>
          <Button disabled={!complete} onClick={save}>
            {initial ? t("Save filter") : t("Add filter")}
            <span className="font-mono text-[11px] text-text-3">↵</span>
          </Button>
        </div>
      </div>
    </Floating>
  );
}

function Footer({ tab, view, estimate, first, last, keys, locked }: { tab: Tab; view: View; estimate: number | null; first: number; last: number; keys: string[]; locked: boolean }) {
  const [menu, setMenu] = useState<Anchor | null>(null);
  const { page } = view;
  const pageNumber = Math.floor(view.offset / view.pageSize) + 1;
  const [draft, setDraft] = useState<string | null>(null);
  const filtered = view.filter !== "";
  const range = !page ? "…" : page.rows.length > 0 ? `${first}–${last}` : "0";
  const goTo = (text: string) => {
    const number = Number.parseInt(text, 10);
    setDraft(null);
    if (Number.isFinite(number) && number >= 1 && number !== pageNumber) void loadPage(tab.id, { offset: (number - 1) * view.pageSize });
  };

  let total: ReactNode;
  if (typeof view.total === "number") {
    total = <span>{t("Rows {range} of {total}", { range, total: view.total })}</span>;
  } else if (view.total === "counting") {
    total = (
      <>
        <span>{t("Rows {range} of {total}", { range, total: "…" })}</span>
        <Spinner />
        <span>{t("Counting rows…")}</span>
      </>
    );
  } else {
    total = (
      <>
        {estimate !== null && !filtered ? (
          <>
            <span>{t("Rows {range} of {total}", { range, total: `≈ ${compactCount(estimate)}` })}</span>
            <Tag>{t("Estimate")}</Tag>
          </>
        ) : (
          <span>{t("Rows {range}", { range })}</span>
        )}
        <button type="button" className="text-accent-text hover:underline" title={t("Counts every row; can be slow on a large table")} onClick={() => countTable(tab.id)}>
          {t("Count exactly")}
        </button>
      </>
    );
  }

  return (
    <div className="flex min-w-0 items-center gap-s3 overflow-hidden border-t border-line bg-bg px-[10.2px] text-[12px] whitespace-nowrap text-text-2">
      {total}
      {divider}
      <IconButton label={t("Previous page")} size={22} disabled={view.loading || view.offset === 0} onClick={() => loadPage(tab.id, { offset: Math.max(0, view.offset - view.pageSize) })}>
        <ChevronLeft size={14} />
      </IconButton>
      <label className="flex items-center gap-[6px]">
        {t("Page")}
        <input
          value={draft ?? String(pageNumber)}
          inputMode="numeric"
          disabled={view.loading}
          onChange={(e) => setDraft(e.target.value.replace(/\D/g, ""))}
          onBlur={(e) => goTo(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") goTo(e.currentTarget.value);
            if (e.key === "Escape") setDraft(null);
          }}
          className="h-[22px] w-[44px] border border-line bg-panel text-center font-mono text-[12px] text-text outline-none focus:border-accent disabled:opacity-45"
        />
      </label>
      <IconButton label={t("Next page")} size={22} disabled={view.loading || !page?.has_more} onClick={() => loadPage(tab.id, { offset: view.offset + view.pageSize })}>
        <ChevronRight size={14} />
      </IconButton>
      <button type="button" className="flex h-[22px] items-center gap-[4px] px-[6px] hover:bg-hover" aria-haspopup="menu" onClick={(e) => setMenu(anchorOf(e.currentTarget))}>
        {t("{count} per page", { count: view.pageSize })}
        <ChevronDown size={12} />
      </button>
      {menu && (
        <Floating anchor={menu} width={170} onClose={() => setMenu(null)} label={t("Page size")}>
          {PAGE_SIZES.map((size) => (
            <MenuItem
              key={size}
              checked={size === view.pageSize}
              onSelect={() => {
                setMenu(null);
                void loadPage(tab.id, { pageSize: size, offset: 0 });
              }}
            >
              {t("{count} per page", { count: size })}
            </MenuItem>
          ))}
        </Floating>
      )}

      <span className="ml-auto font-mono text-[11.5px]">{page ? `${page.elapsed_ms} ms` : ""}</span>
      {locked || keys.length === 0 ? (
        <span className="flex items-center gap-[6px]">
          <Lock size={12} />
          {t("Read-only")}
        </span>
      ) : (
        <span className="min-w-0 truncate">{t("Editable · key {key}", { key: keys.join(", ") })}</span>
      )}
    </div>
  );
}

/** T16, S09: nothing to show yet. */
export function Loading({ text }: { text: string }) {
  return (
    <div className="grid min-h-0 grid-rows-[2px_minmax(0,1fr)]">
      <div className="run-bar" />
      <div className="grid place-items-center">
        <div className="flex items-center gap-s3 border border-line bg-panel px-s4 py-s3 text-[12.5px]">
          <Spinner />
          <span className="font-mono">{text}</span>
        </div>
      </div>
    </div>
  );
}

/** T19, S10: the server refused or the read failed. */
export function LoadError({ message, code, conn, onRetry }: { message: string; code?: string | null; conn: Conn | null; onRetry(): void }) {
  // insufficient_privilege (PostgreSQL), and the MySQL codes for a denied table, column or command.
  const denied = !!code && /^(42501|1142|1143|1227|1044)\b/.test(code);
  const user = conn?.config.user;
  return (
    <div role="alert" className="flex flex-col gap-s3 overflow-auto px-s6 py-s4">
      <div className="flex items-center gap-s3">
        {denied ? <Lock size={16} className="flex-none text-danger" /> : <TriangleAlert size={16} className="flex-none text-danger" />}
        <Tag kind="danger">{denied ? t("No permission") : t("ERROR")}</Tag>
        {code && <span className="selectable font-mono text-[12.5px] font-medium">{code}</span>}
      </div>
      <p className="selectable font-mono text-[14px] break-words whitespace-pre-wrap">{message}</p>
      {user && <p className="text-[12.5px] text-text-2">{t("Connected as {user}.", { user })}</p>}
      <div className="flex gap-s4 text-[12.5px]">
        <button type="button" className="text-accent-text hover:underline" onClick={onRetry}>
          {t("Retry")}
        </button>
        <button type="button" className="text-accent-text hover:underline" onClick={() => void navigator.clipboard?.writeText([code, message].filter(Boolean).join(" "))}>
          {t("Copy error")}
        </button>
      </div>
    </div>
  );
}
