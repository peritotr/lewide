// The structure of a table or view (S01, S03–S06, S08–S10). Columns can be
// added, changed and dropped (S02); indexes and foreign keys added and dropped;
// triggers created, edited and dropped. Every change stays pending until its
// statements are reviewed and applied (S07). The index, key and trigger forms
// are not drawn in the design. Reordering columns is not built.
import { ArrowUpRight, Check, KeyRound, Lock, Minus, Pencil, Plus, Trash2, TriangleAlert, Undo2, X } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { api, errorMessage, type ColumnChange, type ColumnDef, type ColumnDescription, type ForeignKey, type TableStructure } from "../lib/api";
import { openDialog,
  applyStructureChanges,
  dismissStructureApply,
  loadStructure,
  openQueryWith,
  openTable,
  previewStructureChanges,
  setColumnEdits,
  showStructureTab,
  type Conn,
  type StructureTab,
  type Tab,
  type TableView as View,
} from "../store";
import { dialectOf, quoteIdent } from "../lib/sql";
import { Button, Checkbox, Dialog, Field, linkButton, Strip, TextInput } from "./dialog";
import { errorText, Spinner } from "./Results";
import { Loading, LoadError, qualifiedName } from "./TableView";
import { Tag } from "./ui";
import { t, tn } from "../i18n";

const dash = <span className="text-text-3">—</span>;
const head = "label flex h-row items-center border-b border-line bg-bg px-[10px] whitespace-nowrap";
const cell = "flex h-row min-w-0 items-center overflow-hidden border-b border-line-2 px-[10px] font-mono text-[12px] whitespace-nowrap";
const clip = "min-w-0 truncate";

function List({ columns, headers, children, label }: { columns: string; headers: string[]; children: ReactNode; label: string }) {
  return (
    <div role="table" aria-label={label} className="grid min-w-[900px] content-start [&>div:hover>span]:bg-hover" style={{ gridTemplateColumns: columns }}>
      <div role="row" className="contents">
        {headers.map((header, i) => (
          <span key={i} role="columnheader" className={`${head} ${i === 0 && header === "#" ? "justify-end" : ""}`}>
            {header}
          </span>
        ))}
      </div>
      {children}
    </div>
  );
}

const Row = ({ children }: { children: ReactNode }) => (
  <div role="row" className="contents">
    {children}
  </div>
);

const Cell = ({ children, className = "", title }: { children: ReactNode; className?: string; title?: string }) => (
  <span role="cell" title={title} className={`${cell} ${className}`}>
    {children}
  </span>
);

export function StructureView({ tab, view, conn, isView }: { tab: Tab; view: View; conn: Conn | null; isView: boolean }) {
  // ClickHouse changes columns only: no foreign keys or triggers, and its data-skipping indexes are written as SQL.
  const columnsOnly = conn?.driver.id === "clickhouse";
  const state = view.structure;
  const data = state?.state === "ready" ? state.data : null;
  const tabs: { id: StructureTab; label: string; count: number | null; add?: string }[] = isView
    ? [
        { id: "columns", label: t("Columns"), count: data?.columns.length ?? null },
        { id: "ddl", label: t("Definition"), count: null },
      ]
    : [
        { id: "columns", label: t("Columns"), count: data?.columns.length ?? null, add: t("Add column") },
        { id: "indexes", label: t("Indexes"), count: data?.indexes.length ?? null, add: t("Add index") },
        { id: "foreign_keys", label: t("Foreign keys"), count: data?.foreign_keys.length ?? null, add: t("Add foreign key") },
        { id: "triggers", label: t("Triggers"), count: data?.triggers.length ?? null, add: t("Add trigger") },
        { id: "ddl", label: "DDL", count: null },
      ];
  const current = tabs.find((x) => x.id === view.subTab) ?? tabs[0];
  // Set when "Add column" is pressed: the Columns list opens a new row for it.
  const [adding, setAdding] = useState<number | null>(null);
  const [form, setForm] = useState<"index" | "foreign_key" | null>(null);
  // The trigger being written: a new one, or an existing one by name.
  const [triggerForm, setTriggerForm] = useState<{ name: string | null } | null>(null);
  const driver = conn?.driver.id ?? "postgres";
  // SQLite and DuckDB declare foreign keys only when a table is created.
  const noKeys = driver === "sqlite" || driver === "duckdb";
  const addEdit = (change: ColumnChange) => setColumnEdits(tab.id, [...view.columnEdits, change]);

  return (
    <div className="grid min-h-0 grid-rows-[34px_minmax(0,1fr)]">
      <div role="tablist" className="flex items-stretch gap-[20.4px] border-b border-line px-[10.2px] whitespace-nowrap">
        {tabs.map((x) => {
          const selected = x.id === current.id;
          return (
            <button
              key={x.id}
              type="button"
              role="tab"
              aria-selected={selected}
              className={`-mb-px flex items-center gap-[6px] border-b-2 text-[13px] ${selected ? "border-accent font-medium" : "border-transparent text-text-2 hover:text-text"}`}
              onClick={() => showStructureTab(tab.id, x.id)}
            >
              {x.label}
              {x.count !== null && <span className="font-mono text-[11px] font-normal text-text-3">{x.count}</span>}
            </button>
          );
        })}
        {current.add && (
          <button
            type="button"
            disabled={!data || isView || (columnsOnly && current.id !== "columns") || view.structureApply.state === "running" || (current.id === "foreign_keys" && noKeys) || (current.id === "triggers" && driver === "duckdb")}
            title={columnsOnly && current.id === "triggers" ? t("{engine} has no triggers.", { engine: conn?.driver.name ?? "" }) : columnsOnly && current.id !== "columns" ? t("{engine} indexes are added with SQL in a query tab.", { engine: conn?.driver.name ?? "" }) : current.id === "triggers" && driver === "duckdb" ? t("{engine} has no triggers.", { engine: conn?.driver.name ?? "" }) : current.id === "foreign_keys" && noKeys ? t("{engine} adds a foreign key only when the table is created.", { engine: conn?.driver.name ?? "" }) : undefined}
            className="ml-auto flex items-center gap-[5px] text-[12.5px] text-accent-text hover:underline disabled:cursor-not-allowed disabled:no-underline disabled:opacity-45"
            onClick={() => (current.id === "columns" ? setAdding(Date.now()) : current.id === "triggers" ? setTriggerForm({ name: null }) : setForm(current.id === "indexes" ? "index" : "foreign_key"))}
          >
            <Plus size={13} />
            {current.add}
          </button>
        )}
      </div>

      {!state || state.state === "loading" ? (
        <Loading text={t("Reading structure of {name}", { name: qualifiedName(view) })} />
      ) : state.state === "error" ? (
        <LoadError message={state.error.message} code={state.error.code} conn={conn} onRetry={() => loadStructure(tab.id)} />
      ) : (
        <div className="grid min-h-0 grid-rows-[minmax(0,1fr)_auto]">
        <div className="min-h-0 overflow-auto">
          {current.id === "columns" && <Columns tab={tab} view={view} data={state.data} isView={isView} driver={conn?.driver.id ?? "postgres"} adding={adding} onAdded={() => setAdding(null)} />}
          {current.id === "indexes" && <Indexes tab={tab} view={view} data={state.data} editable={!columnsOnly} />}
          {current.id === "foreign_keys" && <ForeignKeys tab={tab} data={state.data} view={view} conn={conn} editable={!columnsOnly && !noKeys} />}
          {current.id === "triggers" && <Triggers tab={tab} view={view} data={state.data} editable={!columnsOnly && driver !== "duckdb"} onEdit={(name) => setTriggerForm({ name })} />}
          {current.id === "ddl" && <Ddl data={state.data} conn={conn} isView={isView} />}
        </div>
        {current.id !== "ddl" && <StructureBar tab={tab} view={view} />}
        </div>
      )}
      {form === "index" && data && <IndexForm view={view} data={data} pending={view.columnEdits} onAdd={addEdit} onClose={() => setForm(null)} />}
      {form === "foreign_key" && data && conn && <ForeignKeyForm view={view} conn={conn} data={data} onAdd={addEdit} onClose={() => setForm(null)} />}
      {triggerForm && data && <TriggerForm tab={tab} view={view} data={data} driver={driver} editing={triggerForm.name} onClose={() => setTriggerForm(null)} />}
    </div>
  );
}

/** Marks an existing index, key or trigger to be dropped, or takes the mark back. */
function DropToggle({ tab, view, change, label }: { tab: Tab; view: View; change: ColumnChange & { name: string }; label: string }) {
  const edits = view.columnEdits;
  const index = edits.findIndex((e) => e.kind === change.kind && "name" in e && e.name === change.name);
  const marked = index >= 0;
  return (
    <button
      type="button"
      title={marked ? t("Keep {name}", { name: change.name }) : t("Drop {name}", { name: change.name })}
      aria-label={marked ? t("Keep {name}", { name: change.name }) : label}
      className={`flex size-[22px] flex-none items-center justify-center hover:bg-hover ${marked ? "text-accent-text" : "text-text-3 hover:text-danger"}`}
      onClick={() => setColumnEdits(tab.id, marked ? edits.filter((_, i) => i !== index) : [...edits, change])}
    >
      {marked ? <Undo2 size={13} /> : <Trash2 size={13} />}
    </button>
  );
}

/** A pending addition, shown above the existing rows with a way to take it back. */
function RemoveAdded({ tab, view, change }: { tab: Tab; view: View; change: ColumnChange }) {
  return (
    <button type="button" title={t("Remove this change")} aria-label={t("Remove this change")} className="flex size-[22px] flex-none items-center justify-center text-text-3 hover:bg-hover hover:text-text" onClick={() => setColumnEdits(tab.id, view.columnEdits.filter((e) => e !== change))}>
      <X size={13} />
    </button>
  );
}

const ACTIONS = ["", "CASCADE", "SET NULL", "SET DEFAULT", "RESTRICT", "NO ACTION"];
const select = "h-[28px] min-w-0 border border-line bg-panel px-[6px] font-mono text-[12.5px] focus:border-accent focus:outline-none";

// Not drawn in the design: the index form, in the pending flow of S02.
function IndexForm({ view, data, pending, onAdd, onClose }: { view: View; data: TableStructure; pending: ColumnChange[]; onAdd(change: ColumnChange): void; onClose(): void }) {
  const [keys, setKeys] = useState<{ name: string; descending: boolean }[]>([]);
  const [unique, setUnique] = useState(false);
  const [name, setName] = useState("");
  const suggested = `${view.name}_${keys.map((k) => k.name).join("_") || "idx"}${unique ? "_key" : "_idx"}`;
  const finalName = name.trim() || suggested;
  const taken = data.indexes.some((i) => i.name === finalName) || pending.some((e) => e.kind === "add_index" && e.name === finalName);
  const columns = data.columns.map((c) => c.name);
  return (
    <Dialog
      title={t("Add index to {table}", { table: qualifiedName(view) })}
      width={520}
      onClose={onClose}
      onSubmit={() => {
        if (keys.length === 0 || taken) return;
        onAdd({ kind: "add_index", name: finalName, columns: keys, unique });
        onClose();
      }}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t("Cancel")}
          </Button>
          <Button type="submit" variant="primary" disabled={keys.length === 0 || taken}>
            {t("Add index")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-s3">
        <Field label={t("Name")} error={taken ? t("An index of this name exists.") : undefined}>
          <TextInput mono value={name} placeholder={suggested} onChange={(e) => setName(e.target.value)} />
        </Field>
        <Field label={t("Columns")} helper={t("In key order: the order you tick them in.")}>
          <div className="flex max-h-[220px] flex-col overflow-auto border border-line">
            {columns.map((column) => {
              const at = keys.findIndex((k) => k.name === column);
              return (
                <div key={column} className="flex h-[30px] items-center gap-s3 border-b border-line-2 px-s2 last:border-b-0">
                  <Checkbox checked={at >= 0} onChange={(on) => setKeys(on ? [...keys, { name: column, descending: false }] : keys.filter((k) => k.name !== column))}>
                    <span className="font-mono text-[12.5px]">{column}</span>
                  </Checkbox>
                  {at >= 0 && (
                    <>
                      <span className="font-mono text-[11px] text-text-3">#{at + 1}</span>
                      <button type="button" className="ml-auto h-[22px] border border-line px-s2 font-mono text-[11px] hover:bg-hover" onClick={() => setKeys(keys.map((k, i) => (i === at ? { ...k, descending: !k.descending } : k)))}>
                        {keys[at].descending ? "DESC" : "ASC"}
                      </button>
                    </>
                  )}
                </div>
              );
            })}
          </div>
        </Field>
        <Checkbox checked={unique} onChange={setUnique}>
          {t("Unique: no two rows may have the same values")}
        </Checkbox>
      </div>
    </Dialog>
  );
}

// Not drawn in the design: the foreign key form, in the pending flow of S02.
function ForeignKeyForm({ view, conn, data, onAdd, onClose }: { view: View; conn: Conn; data: TableStructure; onAdd(change: ColumnChange): void; onClose(): void }) {
  const tables = (conn.objects[view.namespace] ?? []).filter((o) => o.kind === "table").map((o) => o.name);
  const [refTable, setRefTable] = useState(tables.find((name) => name !== view.name) ?? view.name);
  const [refColumns, setRefColumns] = useState<string[] | { error: string } | null>(null);
  const [pairs, setPairs] = useState<{ column: string; ref: string }[]>([{ column: data.columns[0]?.name ?? "", ref: "" }]);
  const [onDelete, setOnDelete] = useState("");
  const [onUpdate, setOnUpdate] = useState("");
  const [name, setName] = useState("");
  useEffect(() => {
    if (conn.session === null) return;
    setRefColumns(null);
    let live = true;
    api.describeObject(conn.session, refTable, view.namespace || undefined).then(
      (d) => {
        if (!live) return;
        const names = d.columns.map((c) => c.name);
        setRefColumns(names);
        // The referenced table's key is the usual target.
        const key = d.columns.find((c) => c.primary_key)?.name ?? names[0] ?? "";
        setPairs((p) => p.map((pair, i) => ({ ...pair, ref: i === 0 ? key : "" })));
      },
      (e) => live && setRefColumns({ error: errorMessage(e) }),
    );
    return () => {
      live = false;
    };
  }, [conn.session, refTable, view.namespace]);
  const suggested = `${view.name}_${pairs.map((p) => p.column).join("_")}_fkey`;
  const ready = Array.isArray(refColumns) && pairs.every((p) => p.column && p.ref);
  return (
    <Dialog
      title={t("Add foreign key to {table}", { table: qualifiedName(view) })}
      width={600}
      onClose={onClose}
      onSubmit={() => {
        if (!ready) return;
        onAdd({ kind: "add_foreign_key", name: name.trim() || suggested, columns: pairs.map((p) => p.column), ref_table: refTable, ref_columns: pairs.map((p) => p.ref), on_delete: onDelete || null, on_update: onUpdate || null });
        onClose();
      }}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t("Cancel")}
          </Button>
          <Button type="submit" variant="primary" disabled={!ready}>
            {t("Add foreign key")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-s3">
        <Field label={t("Name")}>
          <TextInput mono value={name} placeholder={suggested} onChange={(e) => setName(e.target.value)} />
        </Field>
        <Field label={t("References")}>
          <select aria-label={t("References")} className={select} value={refTable} onChange={(e) => setRefTable(e.target.value)}>
            {tables.map((table) => (
              <option key={table} value={table}>
                {[view.namespace, table].filter(Boolean).join(".")}
              </option>
            ))}
          </select>
        </Field>
        <Field label={t("Columns")} helper={t("Each column of this table and the column it points to.")}>
          <div className="flex flex-col gap-s2">
            {pairs.map((pair, i) => (
              <div key={i} className="grid grid-cols-[minmax(0,1fr)_20px_minmax(0,1fr)_24px] items-center gap-s2">
                <select aria-label={t("Column")} className={select} value={pair.column} onChange={(e) => setPairs(pairs.map((p, j) => (j === i ? { ...p, column: e.target.value } : p)))}>
                  {data.columns.map((c) => (
                    <option key={c.name} value={c.name}>
                      {c.name}
                    </option>
                  ))}
                </select>
                <span className="text-center text-text-3">→</span>
                {Array.isArray(refColumns) ? (
                  <select aria-label={t("Referenced column")} className={select} value={pair.ref} onChange={(e) => setPairs(pairs.map((p, j) => (j === i ? { ...p, ref: e.target.value } : p)))}>
                    <option value="" />
                    {refColumns.map((c) => (
                      <option key={c} value={c}>
                        {c}
                      </option>
                    ))}
                  </select>
                ) : (
                  <span className="text-[12px] text-text-3">{refColumns === null ? t("Loading…") : refColumns.error}</span>
                )}
                {pairs.length > 1 ? (
                  <button type="button" aria-label={t("Remove this pair")} className="flex size-[22px] items-center justify-center text-text-3 hover:bg-hover" onClick={() => setPairs(pairs.filter((_, j) => j !== i))}>
                    <X size={13} />
                  </button>
                ) : (
                  <span />
                )}
              </div>
            ))}
            <button type="button" className={`${linkButton} self-start text-[12.5px]`} onClick={() => setPairs([...pairs, { column: data.columns[0]?.name ?? "", ref: "" }])}>
              {t("Add a column pair")}
            </button>
          </div>
        </Field>
        <div className="grid grid-cols-2 gap-s3">
          {(
            [
              ["ON DELETE", onDelete, setOnDelete],
              ["ON UPDATE", onUpdate, setOnUpdate],
            ] as const
          ).map(([label, value, set]) => (
            <Field key={label} label={label}>
              <select aria-label={label} className={select} value={value} onChange={(e) => set(e.target.value)}>
                {ACTIONS.map((a) => (
                  <option key={a} value={a}>
                    {a || t("The database's default")}
                  </option>
                ))}
              </select>
            </Field>
          ))}
        </div>
      </div>
    </Dialog>
  );
}

const target = (key: ForeignKey) => [key.referenced_namespace, key.referenced_table].filter(Boolean).join(".");

const TYPES: Record<string, string[]> = {
  postgres: ["integer", "bigint", "smallint", "numeric(10,2)", "boolean", "text", "varchar(255)", "uuid", "date", "timestamptz", "timestamp", "jsonb", "bytea", "double precision"],
  mysql: ["int", "bigint", "smallint", "decimal(10,2)", "tinyint(1)", "varchar(255)", "text", "date", "datetime", "timestamp", "json", "blob", "double"],
  sqlite: ["INTEGER", "REAL", "TEXT", "BLOB", "NUMERIC"],
  // Nullability is the switch beside the type, so Nullable(…) is left out here.
  clickhouse: ["UInt32", "UInt64", "Int32", "Int64", "Float64", "Decimal(18, 2)", "String", "LowCardinality(String)", "Bool", "Date", "DateTime", "DateTime64(3)", "UUID", "Array(String)"],
};

const blank = (): ColumnDef => ({ name: "", type_name: "", nullable: true, default_value: null });
const sameDef = (a: ColumnDef, b: ColumnDef) => a.name === b.name && a.type_name.trim() === b.type_name.trim() && a.nullable === b.nullable && (a.default_value ?? "").trim() === (b.default_value ?? "").trim();
const toDef = (c: ColumnDescription): ColumnDef => ({ name: c.name, type_name: c.type_name ?? "", nullable: c.nullable, default_value: c.default_value });

function Columns({ tab, view, data, isView, driver, adding, onAdded }: { tab: Tab; view: View; data: TableStructure; isView: boolean; driver: string; adding: number | null; onAdded(): void }) {
  const edits = view.columnEdits;
  // The row being edited: an existing column by name, or an added one by its id.
  const [editing, setEditing] = useState<{ key: string; draft: ColumnDef } | null>(null);
  const [nextId, setNextId] = useState(1);
  const locked = isView || view.structureApply.state === "running";

  useEffect(() => {
    if (adding === null) return;
    const id = nextId;
    setNextId(id + 1);
    setColumnEdits(tab.id, [...edits, { kind: "add", column: { ...blank(), name: "" } }]);
    setEditing({ key: `add:${edits.filter((e) => e.kind === "add").length}:${id}`, draft: blank() });
    onAdded();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [adding]);

  const adds = edits.map((e, i) => ({ e, i })).filter((x): x is { e: Extract<ColumnChange, { kind: "add" }>; i: number } => x.e.kind === "add");
  const modified = (name: string) => edits.find((e): e is Extract<ColumnChange, { kind: "modify" }> => e.kind === "modify" && e.from.name === name);
  const dropped = (name: string) => edits.some((e) => e.kind === "drop" && e.column === name);

  const save = (key: string, draft: ColumnDef) => {
    if (!draft.name.trim() || !draft.type_name.trim()) return;
    if (key.startsWith("add:")) {
      const index = Number(key.split(":")[1]);
      const at = adds[index]?.i;
      if (at === undefined) return;
      setColumnEdits(tab.id, edits.map((e, i) => (i === at ? { kind: "add", column: draft } : e)));
    } else {
      const from = toDef(data.columns.find((c) => c.name === key)!);
      const rest = edits.filter((e) => !(e.kind === "modify" && e.from.name === key));
      setColumnEdits(tab.id, sameDef(from, draft) ? rest : [...rest, { kind: "modify", from, to: draft }]);
    }
    setEditing(null);
  };
  const cancel = (key: string) => {
    // An added row that never got a name and a type is taken away again.
    if (key.startsWith("add:")) {
      const at = adds[Number(key.split(":")[1])]?.i;
      const column = at !== undefined ? (edits[at] as Extract<ColumnChange, { kind: "add" }>).column : null;
      if (column && (!column.name.trim() || !column.type_name.trim())) setColumnEdits(tab.id, edits.filter((_, i) => i !== at));
    }
    setEditing(null);
  };
  const toggleDrop = (name: string) => {
    if (dropped(name)) setColumnEdits(tab.id, edits.filter((e) => !(e.kind === "drop" && e.column === name)));
    else setColumnEdits(tab.id, [...edits.filter((e) => !(e.kind === "modify" && e.from.name === name)), { kind: "drop", column: name }]);
  };

  const grid = "56px 220px 190px 110px minmax(120px,1fr) 170px minmax(120px,1.3fr) 60px";
  const headers = ["#", t("Name"), t("Type"), t("Nullable"), t("Default"), t("Key"), t("Comment"), ""];
  const iconButton = "grid size-[22px] place-items-center text-text-2 enabled:hover:bg-hover disabled:cursor-not-allowed disabled:opacity-45";
  const field = "h-[22px] min-w-0 border border-line bg-panel px-[6px] font-mono text-[12px] outline-none focus:border-accent";

  const editorRow = (key: string, draft: ColumnDef, number: ReactNode, mark: string, tint: string) => {
    const set = (patch: Partial<ColumnDef>) => setEditing({ key, draft: { ...draft, ...patch } });
    const onKey = (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        save(key, draft);
      } else if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        cancel(key);
      }
    };
    return (
      <div key={key} role="row" className={`contents [&>span]:${tint}`}>
        <Cell className={`justify-between ${tint}`}>
          <span className="font-medium text-accent-text">{mark}</span>
          <span className="text-text-3">{number}</span>
        </Cell>
        <Cell className={tint}>
          <input autoFocus aria-label={t("Name")} className={`${field} w-full font-medium`} value={draft.name} spellCheck={false} onFocus={(e) => e.currentTarget.select()} onChange={(e) => set({ name: e.target.value })} onKeyDown={onKey} />
        </Cell>
        <Cell className={tint}>
          <input aria-label={t("Type")} list="column-types" className={`${field} w-full`} value={draft.type_name} spellCheck={false} onChange={(e) => set({ type_name: e.target.value })} onKeyDown={onKey} />
        </Cell>
        <Cell className={tint}>
          <label className="flex items-center gap-[6px] font-body text-[12.5px]">
            <input type="checkbox" checked={draft.nullable} onChange={(e) => set({ nullable: e.target.checked })} onKeyDown={onKey} />
            {t("Nullable")}
          </label>
        </Cell>
        <Cell className={tint}>
          <input aria-label={t("Default")} placeholder={t("No default")} className={`${field} w-full placeholder:font-body placeholder:text-text-3`} value={draft.default_value ?? ""} spellCheck={false} onChange={(e) => set({ default_value: e.target.value || null })} onKeyDown={onKey} />
        </Cell>
        <Cell className={tint}>{dash}</Cell>
        <Cell className={`font-body text-[12.5px] text-text-3 ${tint}`}>{dash}</Cell>
        <Cell className={`justify-end gap-[2px] ${tint}`}>
          <button type="button" className={iconButton} title={`${t("Done")} (Enter)`} aria-label={t("Done")} disabled={!draft.name.trim() || !draft.type_name.trim()} onClick={() => save(key, draft)}>
            <Check size={13} />
          </button>
          <button type="button" className={iconButton} title={`${t("Cancel")} (Esc)`} aria-label={t("Cancel")} onClick={() => cancel(key)}>
            <Undo2 size={13} />
          </button>
        </Cell>
      </div>
    );
  };

  return (
    <List label={t("Columns")} columns={grid} headers={headers}>
      <datalist id="column-types">
        {(TYPES[driver] ?? TYPES.postgres).map((type) => (
          <option key={type} value={type} />
        ))}
      </datalist>
      {data.columns.map((column, i) => {
        if (editing?.key === column.name) return editorRow(column.name, editing.draft, i + 1, "~", "bg-accent-tint");
        const change = modified(column.name);
        const shown = change?.to ?? toDef(column);
        const gone = dropped(column.name);
        const key = data.foreign_keys.find((k) => k.columns.includes(column.name));
        const referenced = key?.referenced_columns[key.columns.indexOf(column.name)];
        const tint = gone ? "bg-danger-tint" : "";
        const was = (differs: boolean, before: string | null | undefined) => (differs ? t("Was {value}", { value: before ?? "—" }) : undefined);
        const changedTint = (differs: boolean) => (differs && !gone ? "bg-accent-tint" : tint);
        const strike = gone ? "line-through decoration-danger" : "";
        const from = toDef(column);
        return (
          <div key={column.name} role="row" className="group contents" onDoubleClick={() => !locked && !gone && setEditing({ key: column.name, draft: shown })}>
            <Cell className={`justify-between ${tint}`}>
              <span className={`font-medium ${gone ? "text-danger" : "text-accent-text"}`} title={gone ? t("Marked for removal") : change ? t("Modified") : undefined}>
                {gone ? "−" : change ? "~" : ""}
              </span>
              <span className="text-text-3">{i + 1}</span>
            </Cell>
            <Cell className={`font-medium ${changedTint(!!change && from.name !== shown.name)}`} title={was(!!change && from.name !== shown.name, from.name)}>
              <span className={`selectable ${clip} ${strike}`}>{shown.name}</span>
            </Cell>
            <Cell className={changedTint(!!change && from.type_name !== shown.type_name)} title={was(!!change && from.type_name !== shown.type_name, from.type_name)}>
              <span className={`selectable ${clip} ${strike}`}>{shown.type_name || dash}</span>
            </Cell>
            <Cell className={changedTint(!!change && from.nullable !== shown.nullable)}>{shown.nullable ? <span className="text-text-3">nullable</span> : "NOT NULL"}</Cell>
            <Cell className={changedTint(!!change && (from.default_value ?? "") !== (shown.default_value ?? ""))} title={was(!!change && (from.default_value ?? "") !== (shown.default_value ?? ""), from.default_value) ?? shown.default_value ?? undefined}>
              {shown.default_value != null && shown.default_value !== "" ? <span className={`selectable ${clip}`}>{shown.default_value}</span> : dash}
            </Cell>
            <Cell className={`gap-[6px] font-body text-[12.5px] ${tint}`}>
              {column.primary_key ? (
                <>
                  <KeyRound size={11} className="flex-none text-accent-text" />
                  {t("Primary key")}
                </>
              ) : key && !isView ? (
                <>
                  <ArrowUpRight size={11} className="flex-none text-accent-text" />
                  <button type="button" className={`${linkButton} ${clip} font-mono text-[12px]`} onClick={() => showStructureTab(tab.id, "foreign_keys")}>
                    {[key.referenced_table, referenced].filter(Boolean).join(".")}
                  </button>
                </>
              ) : (
                dash
              )}
            </Cell>
            <Cell className={`font-body text-[12.5px] text-text-2 ${tint}`} title={column.comment ?? undefined}>
              {column.comment ? <span className={`selectable ${clip}`}>{column.comment}</span> : dash}
            </Cell>
            <Cell className={`justify-end gap-[2px] ${tint}`}>
              {!locked && (
                <span className="flex gap-[2px] opacity-0 group-hover:opacity-100 focus-within:opacity-100">
                  {!gone && (
                    <button type="button" className={iconButton} title={t("Edit column")} aria-label={t("Edit column")} onClick={() => setEditing({ key: column.name, draft: shown })}>
                      <Pencil size={12} />
                    </button>
                  )}
                  <button type="button" className={iconButton} title={gone ? t("Keep column") : t("Remove column")} aria-label={gone ? t("Keep column") : t("Remove column")} onClick={() => toggleDrop(column.name)}>
                    {gone ? <Undo2 size={12} /> : <Minus size={12} />}
                  </button>
                </span>
              )}
            </Cell>
          </div>
        );
      })}
      {adds.map(({ e, i }, n) => {
        const key = `add:${n}:${i}`;
        if (editing?.key.startsWith(`add:${n}:`)) return editorRow(editing.key, editing.draft, "", "+", "bg-ok-tint");
        return (
          <div key={key} role="row" className="group contents" onDoubleClick={() => !locked && setEditing({ key, draft: e.column })}>
            <Cell className="bg-ok-tint">
              <span className="font-medium text-ok" title={t("Added column")}>
                +
              </span>
            </Cell>
            <Cell className="bg-ok-tint font-medium">
              <span className={`selectable ${clip}`}>{e.column.name}</span>
            </Cell>
            <Cell className="bg-ok-tint">
              <span className={`selectable ${clip}`}>{e.column.type_name}</span>
            </Cell>
            <Cell className="bg-ok-tint">{e.column.nullable ? <span className="text-text-3">nullable</span> : "NOT NULL"}</Cell>
            <Cell className="bg-ok-tint">{e.column.default_value ? <span className={`selectable ${clip}`}>{e.column.default_value}</span> : dash}</Cell>
            <Cell className="bg-ok-tint">{dash}</Cell>
            <Cell className="bg-ok-tint">{dash}</Cell>
            <Cell className="justify-end gap-[2px] bg-ok-tint">
              {!locked && (
                <span className="flex gap-[2px] opacity-0 group-hover:opacity-100 focus-within:opacity-100">
                  <button type="button" className={iconButton} title={t("Edit column")} aria-label={t("Edit column")} onClick={() => setEditing({ key, draft: e.column })}>
                    <Pencil size={12} />
                  </button>
                  <button type="button" className={iconButton} title={t("Remove column")} aria-label={t("Remove column")} onClick={() => setColumnEdits(tab.id, edits.filter((_, x) => x !== i))}>
                    <Minus size={12} />
                  </button>
                </span>
              )}
            </Cell>
          </div>
        );
      })}
    </List>
  );
}

/** S02, S07 results: what is pending in the structure, or how applying it went. */
function StructureBar({ tab, view }: { tab: Tab; view: View }) {
  const edits = view.columnEdits;
  const apply = view.structureApply;
  const ghost = "flex h-[26px] flex-none items-center gap-[6px] px-s2 text-[12.5px] whitespace-nowrap enabled:hover:bg-hover disabled:cursor-not-allowed disabled:opacity-45";
  // An edited trigger is a drop and a create of the same name: it counts as one change.
  const replacedTrigger = (e: ColumnChange) => (e.kind === "create_trigger" || e.kind === "drop_trigger") && edits.some((o) => o !== e && (o.kind === "create_trigger" || o.kind === "drop_trigger") && o.name === e.name);
  const count = (kind: "add" | "modify" | "drop") =>
    edits.filter((e) =>
      kind === "add"
        ? e.kind.startsWith("add") || (e.kind === "create_trigger" && !replacedTrigger(e))
        : kind === "drop"
          ? e.kind.startsWith("drop") && !replacedTrigger(e)
          : e.kind === kind || (e.kind === "create_trigger" && replacedTrigger(e)),
    ).length;

  useEffect(() => {
    if (apply.state !== "ok") return;
    const timer = window.setTimeout(() => dismissStructureApply(tab.id), 6000);
    return () => window.clearTimeout(timer);
  }, [apply, tab.id]);
  // ⌘S opens the preview, as the bar says.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && e.key === "s") {
        e.preventDefault();
        void previewStructureChanges(tab.id);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [tab.id]);

  if (apply.state === "ok" && edits.length === 0) {
    return (
      <div role="status" className="flex h-[36px] items-center gap-s3 border-t border-ok bg-ok-tint px-[10.2px] text-[12.5px] whitespace-nowrap">
        <Check size={13} className="flex-none text-ok" />
        <span className="font-medium">{tn(apply.count, "{count} structure change applied", "{count} structure changes applied")}</span>
        <span className="font-mono text-[12px] text-text-2">{apply.ms} ms</span>
        <button type="button" className="ml-auto text-accent-text hover:underline" onClick={() => dismissStructureApply(tab.id)}>
          {t("Dismiss")}
        </button>
      </div>
    );
  }
  if (edits.length === 0) return <div />;
  if (apply.state === "failed") {
    return (
      <div role="alert" className="flex flex-col gap-[4px] border-t border-danger bg-danger-tint px-[10.2px] py-s2 text-[12.5px]">
        <div className="flex items-center gap-s3 whitespace-nowrap">
          <TriangleAlert size={13} className="flex-none text-danger" />
          <span className="font-medium">{t("The structure change failed.")}</span>
          <span className="flex-1" />
          <button type="button" className={ghost} onClick={() => previewStructureChanges(tab.id)}>
            {t("Preview SQL")}
          </button>
          <button type="button" className={ghost} onClick={() => setColumnEdits(tab.id, [])}>
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
      <span className="font-medium">{tn(count("add") + count("modify") + count("drop"), "{count} structure change", "{count} structure changes")}</span>
      <span className="flex gap-s3 font-mono text-[12px]">
        {count("add") > 0 && <span className="text-ok">+ {t("{count} added", { count: count("add") })}</span>}
        {count("modify") > 0 && <span className="text-accent-text">~ {t("{count} modified", { count: count("modify") })}</span>}
        {count("drop") > 0 && <span className="text-danger">− {t("{count} deleted", { count: count("drop") })}</span>}
      </span>
      <span className="min-w-0 truncate text-text-2">{t("Not applied to the table yet")}</span>
      <span className="flex-1" />
      {running && <Spinner />}
      <button type="button" className={ghost} disabled={running} onClick={() => setColumnEdits(tab.id, [])}>
        {t("Discard")}
      </button>
      <Button variant="primary" disabled={running} onClick={() => previewStructureChanges(tab.id)}>
        {t("Preview SQL")}
        <span className="font-mono text-[11px] font-normal tracking-normal">{MOD}S</span>
      </Button>
    </div>
  );
}

const MOD = /Mac/i.test(navigator.userAgent) ? "⌘" : "Ctrl+";

/** S07: the statements, the warnings that go with them, and the confirmation Production asks for. */
export function AlterDialog({ tab, view, conn, statements }: { tab: Tab; view: View; conn: Conn; statements: string[] }) {
  const production = conn.environment === "production";
  const [typed, setTyped] = useState("");
  const edits = view.columnEdits;
  const dataLoss = edits.some((e) => e.kind.startsWith("drop") || (e.kind === "modify" && e.from.type_name.trim() !== e.to.type_name.trim()));
  const transactional = conn.driver.id !== "mysql" && conn.driver.id !== "clickhouse";
  const allowed = !production || typed === conn.name;
  const apply = () => {
    if (!allowed) return;
    openDialog(null);
    void applyStructureChanges(tab.id);
  };
  const look = (statement: string) => (/\bDROP (COLUMN|INDEX|CONSTRAINT|FOREIGN KEY|TRIGGER)\b/i.test(statement) ? { mark: "−", colour: "text-danger", tint: "bg-danger-tint" } : /\bADD COLUMN\b|^ALTER TABLE \S+ ADD \[|^CREATE (UNIQUE )?INDEX\b|\bADD CONSTRAINT\b|^CREATE (OR REPLACE )?(TRIGGER|FUNCTION)\b/i.test(statement) ? { mark: "+", colour: "text-ok", tint: "bg-ok-tint" } : { mark: "~", colour: "text-accent-text", tint: "bg-accent-tint" });
  return (
    <Dialog
      alert={production || dataLoss}
      title={t("Alter {table}", { table: qualifiedName(view) })}
      headerExtra={production ? <Tag kind="warn">{t("PRODUCTION")}</Tag> : undefined}
      band={production ? "production" : undefined}
      width={780}
      onClose={() => openDialog(null)}
      onSubmit={apply}
      footer={
        <>
          <span className="mr-auto">
            <Button variant="ghost" onClick={() => void navigator.clipboard?.writeText(statements.join(";\n") + ";")}>
              {t("Copy SQL")}
            </Button>
          </span>
          <Button data-autofocus={production ? undefined : true} onClick={() => openDialog(null)}>
            {t("Cancel")}
          </Button>
          <Button variant={dataLoss ? "danger" : "primary"} type="submit" disabled={!allowed}>
            {tn(statements.length, "Apply {count} statement", "Apply {count} statements")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-s3">
        <div className="max-h-[300px] overflow-auto border border-line">
          {statements.map((statement, i) => {
            const m = look(statement);
            return (
              <div key={i} className={`grid grid-cols-[28px_minmax(0,1fr)] border-b border-line-2 last:border-b-0 ${m.tint}`}>
                <span className={`pt-[6px] text-center font-mono text-[13px] font-medium ${m.colour}`}>{m.mark}</span>
                <pre className="selectable py-[6px] pr-s3 font-mono text-[12.5px] leading-[20px] break-words whitespace-pre-wrap">{statement}</pre>
              </div>
            );
          })}
        </div>
        {dataLoss && (
          <Strip kind="error" icon={<TriangleAlert size={13} />}>
            {t("Dropping a column, an index, a key or a trigger, or changing a column's type, can lose data or the checks on it. Values that do not fit a new type make the statement fail.")}
          </Strip>
        )}
        <Strip kind={transactional ? "neutral" : "warning"} icon={transactional ? undefined : <TriangleAlert size={13} />}>
          {transactional ? t("The statements run in one transaction: all of them are applied, or none.") : t("Statements run one by one and cannot be rolled back.")}
        </Strip>
        {production && (
          <label className="flex flex-col gap-[4px]">
            <span className="text-[12px] text-text-2">{t("Type the connection name to apply on Production")}</span>
            <TextInput mono data-autofocus value={typed} onChange={(e) => setTyped(e.target.value)} after={<span className="font-mono text-[12px] whitespace-nowrap text-text-3">{conn.name}</span>} />
          </label>
        )}
      </div>
    </Dialog>
  );
}

function Indexes({ tab, view, data, editable }: { tab: Tab; view: View; data: TableStructure; editable: boolean }) {
  const added = view.columnEdits.filter((e): e is Extract<ColumnChange, { kind: "add_index" }> => e.kind === "add_index");
  const dropped = (name: string) => view.columnEdits.some((e) => e.kind === "drop_index" && e.name === name);
  if (data.indexes.length === 0 && added.length === 0) return <Nothing>{t("This table has no indexes.")}</Nothing>;
  return (
    <>
      <List label={t("Indexes")} columns="240px minmax(200px,1.4fr) 150px 110px 110px minmax(120px,1fr) 34px" headers={[t("Name"), t("Columns"), t("Unique"), t("Method"), t("Size"), t("Condition"), ""]}>
        {added.map((index) => (
          <Row key={`+${index.name}`}>
            <Cell className="gap-[6px] bg-ok-tint font-medium">
              <span className="text-ok">+</span>
              <span className={`selectable ${clip}`}>{index.name}</span>
            </Cell>
            <Cell className="bg-ok-tint">
              <span className={clip}>{index.columns.map((c) => c.name + (c.descending ? " DESC" : "")).join(", ")}</span>
            </Cell>
            <Cell className="bg-ok-tint font-body text-[12.5px]">{index.unique ? t("Unique") : dash}</Cell>
            <Cell className="bg-ok-tint">{dash}</Cell>
            <Cell className="bg-ok-tint">{dash}</Cell>
            <Cell className="bg-ok-tint">{dash}</Cell>
            <Cell className="bg-ok-tint px-[4px]">
              <RemoveAdded tab={tab} view={view} change={index} />
            </Cell>
          </Row>
        ))}
        {data.indexes.map((index) => {
          const gone = dropped(index.name);
          const tint = gone ? "bg-danger-tint text-text-3 line-through" : "";
          return (
            <Row key={index.name}>
              <Cell className={`font-medium ${tint}`}>
                <span className={`selectable ${clip}`}>{index.name}</span>
              </Cell>
              <Cell title={index.columns.join(", ")} className={tint}>
                <span className={`selectable ${clip}`}>{index.columns.join(", ")}</span>
              </Cell>
              <Cell className={`gap-[6px] font-body text-[12.5px] ${tint}`}>
                {index.primary ? (
                  <>
                    <KeyRound size={11} className="flex-none text-accent-text" />
                    {t("Primary key")}
                  </>
                ) : index.unique ? (
                  t("Unique")
                ) : (
                  dash
                )}
              </Cell>
              <Cell className={tint}>{index.method ?? dash}</Cell>
              <Cell className={`justify-end ${tint}`}>{index.size ?? dash}</Cell>
              <Cell title={index.condition ?? undefined} className={tint}>{index.condition ? <span className={`selectable ${clip}`}>{index.condition}</span> : dash}</Cell>
              {/* The primary key's index goes with the key, not on its own. */}
              <Cell className={`px-[4px] ${gone ? "bg-danger-tint" : ""}`}>{editable && !index.primary ? <DropToggle tab={tab} view={view} change={{ kind: "drop_index", name: index.name }} label={t("Drop index {name}", { name: index.name })} /> : null}</Cell>
            </Row>
          );
        })}
      </List>
      {data.indexes.some((index) => index.size) && <p className="px-[10px] py-s2 text-[12px] text-text-3">{t("Sizes come from the server and are approximate.")}</p>}
    </>
  );
}

function ForeignKeys({ tab, data, view, conn, editable }: { tab: Tab; data: TableStructure; view: View; conn: Conn | null; editable: boolean }) {
  const incoming = data.referenced_by ?? [];
  const added = view.columnEdits.filter((e): e is Extract<ColumnChange, { kind: "add_foreign_key" }> => e.kind === "add_foreign_key");
  const dropped = (name: string | null) => !!name && view.columnEdits.some((e) => e.kind === "drop_foreign_key" && e.name === name);
  if (data.foreign_keys.length === 0 && incoming.length === 0 && added.length === 0) return <Nothing>{t("This table has no foreign keys.")}</Nothing>;
  return (
    <>
      {data.foreign_keys.length === 0 && added.length === 0 ? (
        <Nothing>{t("This table has no foreign keys.")}</Nothing>
      ) : (
        <List label={t("Foreign keys")} columns="240px minmax(140px,1fr) minmax(200px,1.4fr) 120px 120px 170px 34px" headers={[t("Name"), t("Columns"), t("References"), "ON DELETE", "ON UPDATE", "", ""]}>
          {added.map((key) => (
            <Row key={`+${key.name}`}>
              <Cell className="gap-[6px] bg-ok-tint font-medium">
                <span className="text-ok">+</span>
                <span className={`selectable ${clip}`}>{key.name}</span>
              </Cell>
              <Cell className="bg-ok-tint">
                <span className={clip}>{key.columns.join(", ")}</span>
              </Cell>
              <Cell className="bg-ok-tint">
                <span className={clip}>
                  {[key.ref_namespace ?? view.namespace, key.ref_table].filter(Boolean).join(".")} ({key.ref_columns.join(", ")})
                </span>
              </Cell>
              <Cell className="bg-ok-tint">{key.on_delete ?? dash}</Cell>
              <Cell className="bg-ok-tint">{key.on_update ?? dash}</Cell>
              <Cell className="bg-ok-tint">{dash}</Cell>
              <Cell className="bg-ok-tint px-[4px]">
                <RemoveAdded tab={tab} view={view} change={key} />
              </Cell>
            </Row>
          ))}
          {data.foreign_keys.map((key, i) => {
            const open = () => conn && openTable(conn.id, key.referenced_namespace ?? view.namespace, key.referenced_table, "structure");
            const gone = dropped(key.name);
            const tint = gone ? "bg-danger-tint text-text-3 line-through" : "";
            return (
              <Row key={key.name ?? i}>
                <Cell className={`font-medium ${tint}`}>{key.name ? <span className={`selectable ${clip}`}>{key.name}</span> : dash}</Cell>
                <Cell className={tint}>
                  <span className={`selectable ${clip}`}>{key.columns.join(", ")}</span>
                </Cell>
                <Cell className={`gap-[6px] ${tint}`}>
                  <button type="button" className={`${linkButton} ${clip}`} onClick={open}>
                    {target(key)}
                  </button>
                  {key.referenced_columns.length > 0 && <span className={`${clip} text-text-2`}>({key.referenced_columns.join(", ")})</span>}
                </Cell>
                <Cell className={tint}>{key.on_delete ?? dash}</Cell>
                <Cell className={tint}>{key.on_update ?? dash}</Cell>
                <Cell className={`font-body text-[12.5px] ${gone ? "bg-danger-tint" : ""}`}>
                  <button type="button" className={`${linkButton} ${clip}`} onClick={open}>
                    {t("Open {table}", { table: key.referenced_table })}
                  </button>
                </Cell>
                <Cell className={`px-[4px] ${gone ? "bg-danger-tint" : ""}`}>{editable && key.name ? <DropToggle tab={tab} view={view} change={{ kind: "drop_foreign_key", name: key.name }} label={t("Drop foreign key {name}", { name: key.name })} /> : null}</Cell>
              </Row>
            );
          })}
        </List>
      )}
      {incoming.length > 0 && (
        <>
          <div className="label flex h-[34px] items-end gap-s2 px-[10px] pb-s2">
            {t("Referenced by")}
            <span className="font-mono font-normal tracking-normal">{incoming.length}</span>
          </div>
          <List label={t("Referenced by")} columns="240px minmax(200px,1.2fr) minmax(200px,1.4fr) 120px 120px" headers={[t("Name"), t("From"), t("Columns"), "ON DELETE", "ON UPDATE"]}>
            {incoming.map((reference, i) => (
              <Row key={`${reference.table}:${reference.name ?? i}`}>
                <Cell className="font-medium">{reference.name ? <span className={`selectable ${clip}`}>{reference.name}</span> : dash}</Cell>
                <Cell>
                  <button type="button" className={`${linkButton} ${clip}`} onClick={() => conn && openTable(conn.id, reference.namespace ?? view.namespace, reference.table, "structure")}>
                    {[reference.namespace, reference.table].filter(Boolean).join(".")}
                  </button>
                </Cell>
                <Cell>
                  <span className={`selectable ${clip}`}>
                    {reference.columns.join(", ")}
                    {reference.referenced_columns.length > 0 ? ` → ${reference.referenced_columns.join(", ")}` : ""}
                  </span>
                </Cell>
                <Cell>{reference.on_delete ?? dash}</Cell>
                <Cell>{reference.on_update ?? dash}</Cell>
              </Row>
            ))}
          </List>
        </>
      )}
    </>
  );
}

function Triggers({ tab, view, data, editable, onEdit }: { tab: Tab; view: View; data: TableStructure; editable: boolean; onEdit(name: string): void }) {
  const created = view.columnEdits.filter((e): e is Extract<ColumnChange, { kind: "create_trigger" }> => e.kind === "create_trigger");
  const fresh = created.filter((c) => !data.triggers.some((x) => x.name === c.name));
  if (data.triggers.length === 0 && fresh.length === 0) return <Nothing>{t("This table has no triggers.")}</Nothing>;
  const added = "bg-ok-tint";
  return (
    <List label={t("Triggers")} columns="240px 200px 160px minmax(240px,1fr) 60px" headers={[t("Name"), t("Event"), t("Timing"), t("Definition"), ""]}>
      {fresh.map((change) => (
        <Row key={`new-${change.name}`}>
          <Cell className={`font-medium ${added}`}>
            <span className={`selectable ${clip}`}>{change.name}</span>
          </Cell>
          <Cell className={added}>{dash}</Cell>
          <Cell className={added}>{dash}</Cell>
          <Cell title={change.definition} className={added}>
            <span className={`selectable ${clip}`}>{change.definition}</span>
          </Cell>
          <Cell className={`gap-[2px] px-[4px] ${added}`}>
            <EditButton label={t("Edit trigger {name}", { name: change.name })} onClick={() => onEdit(change.name)} />
            <RemoveAdded tab={tab} view={view} change={change} />
          </Cell>
        </Row>
      ))}
      {data.triggers.map((trigger) => {
        const replaced = created.find((c) => c.name === trigger.name);
        const gone = !replaced && view.columnEdits.some((e) => e.kind === "drop_trigger" && e.name === trigger.name);
        const tint = gone ? "bg-danger-tint text-text-3 line-through" : replaced ? "bg-accent-tint" : "";
        return (
          <Row key={trigger.name}>
            <Cell className={`font-medium ${tint}`}>
              <span className={`selectable ${clip}`}>{trigger.name}</span>
            </Cell>
            <Cell className={tint}>{trigger.events.length > 0 ? trigger.events.join(" OR ") : dash}</Cell>
            <Cell className={tint}>{trigger.timing ?? dash}</Cell>
            <Cell title={replaced?.definition ?? trigger.definition ?? undefined} className={tint}>
              {replaced ? <span className={`selectable ${clip}`}>{replaced.definition}</span> : trigger.definition ? <span className={`selectable ${clip}`}>{trigger.definition}</span> : dash}
            </Cell>
            <Cell className={`gap-[2px] px-[4px] ${tint}`}>
              {editable && !gone && <EditButton label={t("Edit trigger {name}", { name: trigger.name })} onClick={() => onEdit(trigger.name)} />}
              {editable && replaced && (
                <button
                  type="button"
                  title={t("Keep {name} as it is", { name: trigger.name })}
                  aria-label={t("Keep {name} as it is", { name: trigger.name })}
                  className="flex size-[22px] flex-none items-center justify-center text-accent-text hover:bg-hover"
                  onClick={() => setColumnEdits(tab.id, view.columnEdits.filter((e) => !((e.kind === "create_trigger" || e.kind === "drop_trigger") && e.name === trigger.name)))}
                >
                  <Undo2 size={13} />
                </button>
              )}
              {editable && !replaced && <DropToggle tab={tab} view={view} change={{ kind: "drop_trigger", name: trigger.name }} label={t("Drop trigger {name}", { name: trigger.name })} />}
            </Cell>
          </Row>
        );
      })}
    </List>
  );
}

const EditButton = ({ label, onClick }: { label: string; onClick(): void }) => (
  <button type="button" title={label} aria-label={label} className="flex size-[22px] flex-none items-center justify-center text-text-3 hover:bg-hover hover:text-text" onClick={onClick}>
    <Pencil size={13} />
  </button>
);

/** A starting text for a new trigger, in the engine's own form. */
function triggerTemplate(driver: string, view: View, name: string): string {
  const dialect = dialectOf(driver);
  const q = (n: string) => quoteIdent(n, dialect);
  const table = view.namespace ? `${q(view.namespace)}.${q(view.name)}` : q(view.name);
  const inSchema = (n: string) => (view.namespace ? `${q(view.namespace)}.${q(n)}` : q(n));
  switch (driver) {
    case "postgres":
    case "pgvector":
      return [
        `CREATE OR REPLACE FUNCTION ${inSchema(`${name}_fn`)}() RETURNS trigger LANGUAGE plpgsql AS $$`,
        "BEGIN",
        "  -- NEW is the row being written; OLD is the row before it.",
        "  RETURN NEW;",
        "END;",
        "$$;",
        `CREATE TRIGGER ${q(name)} BEFORE UPDATE ON ${table}`,
        `  FOR EACH ROW EXECUTE FUNCTION ${inSchema(`${name}_fn`)}();`,
      ].join("\n");
    case "sqlserver":
      return [`CREATE TRIGGER ${inSchema(name)} ON ${table} AFTER UPDATE AS`, "BEGIN", "  SET NOCOUNT ON;", "  -- inserted and deleted hold the rows the statement changed.", "END"].join("\n");
    case "sqlite":
      return [`CREATE TRIGGER ${q(name)} AFTER UPDATE ON ${table} FOR EACH ROW`, "BEGIN", "  -- NEW is the row after the change; OLD is the row before it.", "  SELECT 1;", "END"].join("\n");
    default:
      return [`CREATE TRIGGER ${q(name)} BEFORE UPDATE ON ${table} FOR EACH ROW`, "BEGIN", "  -- NEW is the row being written; OLD is the row before it.", "END"].join("\n");
  }
}

// Not drawn in the design: a trigger written as SQL, in the pending flow of S02.
function TriggerForm({ tab, view, data, driver, editing, onClose }: { tab: Tab; view: View; data: TableStructure; driver: string; editing: string | null; onClose(): void }) {
  const edits = view.columnEdits;
  const existing = editing ? data.triggers.find((x) => x.name === editing) : undefined;
  const pending = editing ? edits.find((e): e is Extract<ColumnChange, { kind: "create_trigger" }> => e.kind === "create_trigger" && e.name === editing) : undefined;
  const initial = (): string => {
    if (pending) return pending.definition;
    const definition = existing?.definition?.trim();
    if (!existing || !definition) return editing ? triggerTemplate(driver, view, editing) : "";
    if (/^create\b/i.test(definition)) return definition;
    // MySQL keeps the body alone; the rest comes from the trigger's own fields.
    const dialect = dialectOf(driver);
    const table = view.namespace ? `${quoteIdent(view.namespace, dialect)}.${quoteIdent(view.name, dialect)}` : quoteIdent(view.name, dialect);
    return `CREATE TRIGGER ${quoteIdent(existing.name, dialect)} ${existing.timing ?? "BEFORE"} ${existing.events[0] ?? "UPDATE"} ON ${table} FOR EACH ROW\n${definition}`;
  };
  const [name, setName] = useState(editing ?? "");
  const [text, setText] = useState(initial);
  const [touched, setTouched] = useState(editing !== null);
  const finalName = name.trim();
  const taken = !editing && (data.triggers.some((x) => x.name === finalName) || edits.some((e) => e.kind === "create_trigger" && e.name === finalName));
  const ready = finalName !== "" && !taken && text.trim() !== "";
  // A new trigger's text follows its name until the text is changed by hand.
  const rename = (value: string) => {
    setName(value);
    if (!touched) setText(value.trim() ? triggerTemplate(driver, view, value.trim()) : "");
  };
  return (
    <Dialog
      title={editing ? t("Edit trigger {name}", { name: editing }) : t("Add trigger to {table}", { table: qualifiedName(view) })}
      width={680}
      onClose={onClose}
      onSubmit={() => {
        if (!ready) return;
        const others = edits.filter((e) => !((e.kind === "create_trigger" || e.kind === "drop_trigger") && e.name === finalName));
        const change: ColumnChange = { kind: "create_trigger", name: finalName, definition: text };
        setColumnEdits(tab.id, existing ? [...others, { kind: "drop_trigger", name: finalName }, change] : [...others, change]);
        onClose();
      }}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t("Cancel")}
          </Button>
          <Button type="submit" variant="primary" disabled={!ready}>
            {editing ? t("Save trigger") : t("Add trigger")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-s3">
        <Field label={t("Name")} error={taken ? t("A trigger of this name exists.") : undefined}>
          <TextInput mono autoFocus={!editing} disabled={!!editing} value={name} onChange={(e) => rename(e.target.value)} />
        </Field>
        <Field
          label={t("Definition")}
          helper={
            existing
              ? t("Applying drops the trigger and creates it from this text, in one transaction where the engine allows it.")
              : t("The whole statement that creates the trigger. It runs as you wrote it.")
          }
        >
          <textarea
            aria-label={t("Definition")}
            spellCheck={false}
            className="h-[240px] w-full resize-y border border-line bg-panel p-s2 font-mono text-[12.5px] leading-[19px] focus:border-accent focus:outline-none"
            value={text}
            onChange={(e) => {
              setTouched(true);
              setText(e.target.value);
            }}
            onKeyDown={(e) => {
              // Enter writes a new line here; ⌘↵ submits.
              if (e.key === "Enter" && !(e.metaKey || e.ctrlKey)) e.stopPropagation();
            }}
          />
        </Field>
        {(driver === "postgres" || driver === "pgvector") && existing && (
          <Strip kind="note">{t("PostgreSQL keeps a trigger's body in its function. To change the body, add CREATE OR REPLACE FUNCTION above the trigger.")}</Strip>
        )}
      </div>
    </Dialog>
  );
}

function Ddl({ data, conn, isView }: { data: TableStructure; conn: Conn | null; isView: boolean }) {
  if (!data.ddl) return <Nothing>{t("This database does not give the statement that creates this object.")}</Nothing>;
  const lines = data.ddl.split("\n");
  return (
    <div className="flex min-h-full flex-col">
      <Strip kind="neutral" icon={<Lock size={13} />} className="items-center border-x-0 border-t-0">
        <div className="flex items-center gap-s4">
          <span className="min-w-0 flex-1">{isView ? t("Read-only. Generated from the catalog.") : t("Read-only. Generated from the catalog; edit the table in the other tabs.")}</span>
          <button type="button" className={linkButton} onClick={() => void navigator.clipboard?.writeText(data.ddl!)}>
            {t("Copy")}
          </button>
          <button type="button" className={linkButton} disabled={!conn} onClick={() => conn && openQueryWith(conn.id, data.ddl!)}>
            {t("Open in new query")}
          </button>
        </div>
      </Strip>
      <div className="grid grid-cols-[48px_minmax(0,1fr)] py-s2 font-mono text-[12.5px] leading-[20px]">
        <div aria-hidden className="pr-s3 text-right text-text-3 select-none">
          {lines.map((_, i) => (
            <div key={i}>{i + 1}</div>
          ))}
        </div>
        <pre className="selectable overflow-x-auto whitespace-pre">{data.ddl}</pre>
      </div>
    </div>
  );
}

const Nothing = ({ children }: { children: ReactNode }) => <p className="px-s6 py-s5 text-[12.5px] text-text-2">{children}</p>;
