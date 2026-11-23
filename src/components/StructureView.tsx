// The structure of a table or view (S01, S03–S06, S08–S10). Columns can be
// added, changed and dropped (S02); the changes stay pending until their
// ALTER TABLE statements are reviewed and applied (S07). Reordering columns and
// changing indexes, foreign keys and triggers are not built.
import { ArrowUpRight, Check, KeyRound, Lock, Minus, Pencil, Plus, TriangleAlert, Undo2 } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import type { ColumnChange, ColumnDef, ColumnDescription, ForeignKey, TableStructure } from "../lib/api";
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
import { Button, Dialog, linkButton, Strip, TextInput } from "./dialog";
import { errorText, Spinner } from "./Results";
import { Loading, LoadError, qualifiedName } from "./TableView";
import { NOT_BUILT, Tag } from "./ui";
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
  const noAlter = conn?.driver.id === "sqlserver" || conn?.driver.id === "clickhouse";
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
        { id: "triggers", label: t("Triggers"), count: data?.triggers.length ?? null },
        { id: "ddl", label: "DDL", count: null },
      ];
  const current = tabs.find((x) => x.id === view.subTab) ?? tabs[0];
  // Set when "Add column" is pressed: the Columns list opens a new row for it.
  const [adding, setAdding] = useState<number | null>(null);

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
            disabled={current.id !== "columns" || !data || isView || noAlter || view.structureApply.state === "running"}
            title={current.id !== "columns" ? t(NOT_BUILT) : noAlter ? t("Changing columns is not built for {engine} yet.", { engine: conn?.driver.name ?? "" }) : undefined}
            className="ml-auto flex items-center gap-[5px] text-[12.5px] text-accent-text hover:underline disabled:cursor-not-allowed disabled:no-underline disabled:opacity-45"
            onClick={() => setAdding(Date.now())}
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
          {current.id === "indexes" && <Indexes data={state.data} />}
          {current.id === "foreign_keys" && <ForeignKeys data={state.data} view={view} conn={conn} />}
          {current.id === "triggers" && <Triggers data={state.data} />}
          {current.id === "ddl" && <Ddl data={state.data} conn={conn} isView={isView} />}
        </div>
        {current.id === "columns" && <StructureBar tab={tab} view={view} />}
        </div>
      )}
    </div>
  );
}

const target = (key: ForeignKey) => [key.referenced_namespace, key.referenced_table].filter(Boolean).join(".");

const TYPES: Record<string, string[]> = {
  postgres: ["integer", "bigint", "smallint", "numeric(10,2)", "boolean", "text", "varchar(255)", "uuid", "date", "timestamptz", "timestamp", "jsonb", "bytea", "double precision"],
  mysql: ["int", "bigint", "smallint", "decimal(10,2)", "tinyint(1)", "varchar(255)", "text", "date", "datetime", "timestamp", "json", "blob", "double"],
  sqlite: ["INTEGER", "REAL", "TEXT", "BLOB", "NUMERIC"],
};

const blank = (): ColumnDef => ({ name: "", type_name: "", nullable: true, default_value: null });
const sameDef = (a: ColumnDef, b: ColumnDef) => a.name === b.name && a.type_name.trim() === b.type_name.trim() && a.nullable === b.nullable && (a.default_value ?? "").trim() === (b.default_value ?? "").trim();
const toDef = (c: ColumnDescription): ColumnDef => ({ name: c.name, type_name: c.type_name ?? "", nullable: c.nullable, default_value: c.default_value });

function Columns({ tab, view, data, isView, driver, adding, onAdded }: { tab: Tab; view: View; data: TableStructure; isView: boolean; driver: string; adding: number | null; onAdded(): void }) {
  const edits = view.columnEdits;
  // The row being edited: an existing column by name, or an added one by its id.
  const [editing, setEditing] = useState<{ key: string; draft: ColumnDef } | null>(null);
  const [nextId, setNextId] = useState(1);
  // T-SQL changes columns differently (sp_rename, named default constraints); agreed to come later.
  const locked = isView || driver === "sqlserver" || driver === "clickhouse" || view.structureApply.state === "running";

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
  const count = (kind: ColumnChange["kind"]) => edits.filter((e) => e.kind === kind).length;

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
      <span className="font-medium">{tn(edits.length, "{count} structure change", "{count} structure changes")}</span>
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
  const dataLoss = edits.some((e) => e.kind === "drop" || (e.kind === "modify" && e.from.type_name.trim() !== e.to.type_name.trim()));
  const transactional = conn.driver.id !== "mysql";
  const allowed = !production || typed === conn.name;
  const apply = () => {
    if (!allowed) return;
    openDialog(null);
    void applyStructureChanges(tab.id);
  };
  const look = (statement: string) => (/\bDROP COLUMN\b/i.test(statement) ? { mark: "−", colour: "text-danger", tint: "bg-danger-tint" } : /\bADD COLUMN\b/i.test(statement) ? { mark: "+", colour: "text-ok", tint: "bg-ok-tint" } : { mark: "~", colour: "text-accent-text", tint: "bg-accent-tint" });
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
            {t("Dropping a column or changing its type can lose data. Values that do not fit the new type make the statement fail.")}
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

function Indexes({ data }: { data: TableStructure }) {
  if (data.indexes.length === 0) return <Nothing>{t("This table has no indexes.")}</Nothing>;
  return (
    <>
      <List label={t("Indexes")} columns="240px minmax(200px,1.4fr) 150px 110px 110px minmax(120px,1fr)" headers={[t("Name"), t("Columns"), t("Unique"), t("Method"), t("Size"), t("Condition")]}>
        {data.indexes.map((index) => (
          <Row key={index.name}>
            <Cell className="font-medium">
              <span className={`selectable ${clip}`}>{index.name}</span>
            </Cell>
            <Cell title={index.columns.join(", ")}>
              <span className={`selectable ${clip}`}>{index.columns.join(", ")}</span>
            </Cell>
            <Cell className="gap-[6px] font-body text-[12.5px]">
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
            <Cell>{index.method ?? dash}</Cell>
            <Cell className="justify-end">{index.size ?? dash}</Cell>
            <Cell title={index.condition ?? undefined}>{index.condition ? <span className={`selectable ${clip}`}>{index.condition}</span> : dash}</Cell>
          </Row>
        ))}
      </List>
      {data.indexes.some((index) => index.size) && <p className="px-[10px] py-s2 text-[12px] text-text-3">{t("Sizes come from the server and are approximate.")}</p>}
    </>
  );
}

function ForeignKeys({ data, view, conn }: { data: TableStructure; view: View; conn: Conn | null }) {
  const incoming = data.referenced_by ?? [];
  if (data.foreign_keys.length === 0 && incoming.length === 0) return <Nothing>{t("This table has no foreign keys.")}</Nothing>;
  return (
    <>
      {data.foreign_keys.length === 0 ? (
        <Nothing>{t("This table has no foreign keys.")}</Nothing>
      ) : (
        <List label={t("Foreign keys")} columns="240px minmax(140px,1fr) minmax(200px,1.4fr) 120px 120px 170px" headers={[t("Name"), t("Columns"), t("References"), "ON DELETE", "ON UPDATE", ""]}>
          {data.foreign_keys.map((key, i) => {
            const open = () => conn && openTable(conn.id, key.referenced_namespace ?? view.namespace, key.referenced_table, "structure");
            return (
              <Row key={key.name ?? i}>
                <Cell className="font-medium">{key.name ? <span className={`selectable ${clip}`}>{key.name}</span> : dash}</Cell>
                <Cell>
                  <span className={`selectable ${clip}`}>{key.columns.join(", ")}</span>
                </Cell>
                <Cell className="gap-[6px]">
                  <button type="button" className={`${linkButton} ${clip}`} onClick={open}>
                    {target(key)}
                  </button>
                  {key.referenced_columns.length > 0 && <span className={`${clip} text-text-2`}>({key.referenced_columns.join(", ")})</span>}
                </Cell>
                <Cell>{key.on_delete ?? dash}</Cell>
                <Cell>{key.on_update ?? dash}</Cell>
                <Cell className="font-body text-[12.5px]">
                  <button type="button" className={`${linkButton} ${clip}`} onClick={open}>
                    {t("Open {table}", { table: key.referenced_table })}
                  </button>
                </Cell>
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

function Triggers({ data }: { data: TableStructure }) {
  if (data.triggers.length === 0) return <Nothing>{t("This table has no triggers.")}</Nothing>;
  return (
    <List label={t("Triggers")} columns="240px 200px 160px minmax(240px,1fr)" headers={[t("Name"), t("Event"), t("Timing"), t("Definition")]}>
      {data.triggers.map((trigger) => (
        <Row key={trigger.name}>
          <Cell className="font-medium">
            <span className={`selectable ${clip}`}>{trigger.name}</span>
          </Cell>
          <Cell>{trigger.events.length > 0 ? trigger.events.join(" OR ") : dash}</Cell>
          <Cell>{trigger.timing ?? dash}</Cell>
          <Cell title={trigger.definition ?? undefined}>{trigger.definition ? <span className={`selectable ${clip}`}>{trigger.definition}</span> : dash}</Cell>
        </Row>
      ))}
    </List>
  );
}

function Ddl({ data, conn, isView }: { data: TableStructure; conn: Conn | null; isView: boolean }) {
  if (!data.ddl) return <Nothing>{t("This database does not give the statement that creates this object.")}</Nothing>;
  const lines = data.ddl.split("\n");
  return (
    <div className="flex min-h-full flex-col">
      <Strip kind="neutral" icon={<Lock size={13} />} className="items-center border-x-0 border-t-0">
        <div className="flex items-center gap-s4">
          <span className="min-w-0 flex-1">{isView || conn?.driver.id === "sqlserver" || conn?.driver.id === "clickhouse" ? t("Read-only. Generated from the catalog.") : t("Read-only. Generated from the catalog; edit the table in the other tabs.")}</span>
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
