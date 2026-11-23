// A document tab (D01–D03): one collection, a page of documents at a time, as
// a tree, a table or JSON. A value is edited in place and written at once with
// $set on the document's _id; on a Production connection every write asks first.
import { ChevronDown, ChevronLeft, ChevronRight, Copy, Plus, RefreshCw, Trash2, TriangleAlert } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { api, dbError, errorMessage, type DocCollectionInfo, type DocumentChange, type FindPage } from "../lib/api";
import { bsonType, cellAt, isContainer, parseAs, preview, scalarText, scalarTone, tableColumns, typeLabel } from "../lib/ejson";
import { formatBytes } from "../lib/kv";
import type { Conn, Tab } from "../store";
import { t, tn } from "../i18n";
import { Button, Dialog, Segmented } from "./dialog";
import { TabBar } from "./TabBar";
import { LoadError, Loading } from "./TableView";
import { Tag } from "./ui";

const ghost = "flex h-[26px] flex-none items-center gap-[6px] px-s2 text-[12.5px] whitespace-nowrap enabled:hover:bg-hover disabled:cursor-not-allowed disabled:opacity-45";
const label = "font-heading text-[10.5px] font-semibold tracking-[.08em] text-text-2 uppercase";
const TONE = { str: "var(--syn-str)", num: "var(--syn-num)", kw: "var(--syn-kw)", fn: "var(--syn-fn)" };

type Mode = "tree" | "table" | "json";
type Doc = Record<string, unknown>;
type Part = "filter" | "projection" | "sort";
interface Query {
  filter: string;
  projection: string;
  sort: string;
  limit: string;
}
const EMPTY: Query = { filter: "", projection: "", sort: "", limit: "50" };
/** Asks before a write on Production; otherwise the write runs at once. */
interface Confirm {
  title: string;
  body: ReactNode;
  submit: string;
  run(): void;
  /** Called when the user declines, so whoever asked can stop waiting. */
  cancel?(): void;
}

const idText = (id: unknown) => (isContainer(id) ? JSON.stringify(id) : scalarText(id));

export function DocumentView({ tab: _tab, database, collection, conn }: { tab: Tab; database: string; collection: string; conn: Conn | null }) {
  const session = conn?.session ?? null;
  const production = conn?.environment === "production";

  const [mode, setMode] = useState<Mode>("tree");
  const [info, setInfo] = useState<DocCollectionInfo | null>(null);
  const [draft, setDraft] = useState<Query>(EMPTY);
  const [applied, setApplied] = useState<Query>(EMPTY);
  const [skip, setSkip] = useState(0);
  const [page, setPage] = useState<FindPage | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [invalid, setInvalid] = useState<{ part: Part; message: string; position: number | null } | null>(null);
  const [total, setTotal] = useState<number | null>(null);
  const [notice, setNotice] = useState<{ text: string; error: boolean } | null>(null);
  const [confirm, setConfirm] = useState<Confirm | null>(null);
  const [adding, setAdding] = useState(false);
  const run = useRef(0);

  const readOnly = info?.kind === "view";
  const limitOf = (q: Query) => Math.max(1, Math.min(1000, Number.parseInt(q.limit, 10) || 50));

  const find = useCallback(
    async (query: Query, from: number) => {
      if (session === null) return;
      const id = ++run.current;
      setLoading(true);
      setError(null);
      setInvalid(null);
      try {
        const loaded = await api.docFind(session, { database, collection, filter: query.filter || null, projection: query.projection || null, sort: query.sort || null, skip: from, limit: limitOf(query) });
        if (id !== run.current) return;
        setPage(loaded);
        setApplied(query);
        setSkip(from);
        if (query.filter !== applied.filter || total === null) {
          setTotal(null);
          api.docCount(session, database, collection, query.filter || null).then(
            (n) => id === run.current && setTotal(n),
            () => {},
          );
        }
      } catch (e) {
        if (id !== run.current) return;
        const failed = dbError(e);
        const part = /^invalid_(filter|projection|sort)$/.exec(failed?.code ?? "")?.[1] as Part | undefined;
        // D03: text that does not read leaves the last results where they are.
        if (failed && part) setInvalid({ part, message: failed.message, position: failed.position ?? null });
        else if (page) setNotice({ text: errorMessage(e), error: true });
        else setError(errorMessage(e));
      } finally {
        if (id === run.current) setLoading(false);
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [session, database, collection, applied.filter, total, page],
  );

  const loadInfo = useCallback(() => {
    if (session === null) return;
    api.docCollections(session, database).then(
      (list) => setInfo(list.find((c) => c.name === collection) ?? null),
      () => {},
    );
  }, [session, database, collection]);

  useEffect(() => {
    loadInfo();
    void find(EMPTY, 0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session, database, collection]);

  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => setNotice(null), notice.error ? 8000 : 3500);
    return () => clearTimeout(timer);
  }, [notice]);

  /** `after` lets a dialog stay open and show the server's refusal itself. */
  const write = (change: DocumentChange, describe: string, done: string, after?: WriteOutcome) => {
    if (session === null) return;
    const go = async () => {
      try {
        const outcome = await api.docWrite(session, database, collection, change);
        if (change.op !== "insert" && outcome.matched + outcome.deleted === 0) setNotice({ text: t("The document is no longer there. Nothing was changed."), error: true });
        else setNotice({ text: done, error: false });
        setTotal(null);
        void find(applied, skip);
        if (change.op !== "update") loadInfo();
        after?.ok();
      } catch (e) {
        if (after) after.failed(errorMessage(e));
        else setNotice({ text: errorMessage(e), error: true });
      }
    };
    if (!production) return void go();
    setConfirm({
      title: t("Run on Production?"),
      body: (
        <>
          <p className="pb-s2">{t("This change is written at once on {name} and cannot be undone here.", { name: conn?.name ?? "" })}</p>
          <pre className="selectable max-h-[200px] overflow-auto border border-line bg-panel px-s3 py-s2 font-mono text-[12.5px] whitespace-pre-wrap">{describe}</pre>
        </>
      ),
      submit: t("Run"),
      run: () => void go(),
      cancel: () => after?.failed(""),
    });
  };

  const setField = (id: unknown, path: string, value: unknown) =>
    write({ op: "update", id, update: JSON.stringify({ $set: { [path]: value } }) }, `db.${collection}.updateOne({_id: ${idText(id)}}, {$set: {${path}: …}})`, t("Updated {path}", { path }));

  const remove = (id: unknown) =>
    setConfirm({
      title: t("Delete document?"),
      body: (
        <>
          <p className="pb-s2">{t("The document is removed from {collection}. This cannot be undone.", { collection })}</p>
          <pre className="selectable overflow-auto border border-line bg-panel px-s3 py-s2 font-mono text-[12.5px]">_id: {idText(id)}</pre>
        </>
      ),
      submit: t("Delete document"),
      run: () => {
        // The question was just answered: a Production connection is not asked twice.
        if (session === null) return;
        api.docWrite(session, database, collection, { op: "delete", id }).then(
          () => {
            setNotice({ text: t("Document deleted"), error: false });
            setTotal(null);
            void find(applied, skip);
            loadInfo();
          },
          (e) => setNotice({ text: errorMessage(e), error: true }),
        );
      },
    });

  const documents = useMemo(() => page?.documents.map((d) => d.document) ?? [], [page]);
  const submit = () => void find(draft, 0);
  const limit = limitOf(applied);

  if (!conn || session === null) {
    return (
      <main className="grid min-h-0 min-w-0 grid-rows-[34px_minmax(0,1fr)] bg-panel">
        <TabBar />
        <div className="grid place-items-center">
          <p className="text-text-2">{t("This tab's connection is not open. Double-click it in the sidebar.")}</p>
        </div>
      </main>
    );
  }

  const field = (part: keyof Query, name: string, className: string, placeholder: string) => (
    <label className={`flex min-w-0 flex-col gap-[3px] ${className}`}>
      <span className={label}>{name}</span>
      <input
        aria-label={name}
        aria-invalid={invalid?.part === part}
        spellCheck={false}
        autoCapitalize="off"
        autoCorrect="off"
        placeholder={placeholder}
        className={`h-[26px] min-w-0 border bg-panel px-[8px] font-mono text-[12.5px] placeholder:text-text-3 focus:border-accent focus:outline-none ${invalid?.part === part ? "border-danger" : "border-line"}`}
        value={draft[part]}
        onChange={(e) => setDraft({ ...draft, [part]: e.target.value })}
      />
    </label>
  );

  return (
    <main className="relative grid min-h-0 min-w-0 bg-panel" style={{ gridTemplateRows: `34px ${production ? "4px" : "0px"} 36px auto minmax(0,1fr) 30px` }}>
      <TabBar />
      <div title={production ? t("Production connection") : undefined} className={production ? "production-band" : ""} />

      <div className="flex items-center gap-[10.2px] border-b border-line px-[10.2px] whitespace-nowrap">
        <Segmented<Mode>
          label={t("View")}
          value={mode}
          onChange={setMode}
          options={[
            { value: "tree", label: t("Tree") },
            { value: "table", label: t("Table") },
            { value: "json", label: "JSON" },
          ]}
        />
        <span className="selectable min-w-0 truncate font-mono text-[12.5px] font-medium">
          {database}.{collection}
        </span>
        {info && (
          <span className="min-w-0 truncate text-[12px] text-text-2">
            {[
              info.kind === "view" ? t("View · read-only") : info.kind === "timeseries" ? t("Time series") : t("Collection"),
              info.document_count !== null ? tn(info.document_count, "{count} document", "{count} documents") : null,
              info.avg_document_bytes ? t("avg {size}", { size: formatBytes(info.avg_document_bytes) }) : null,
            ]
              .filter(Boolean)
              .join(" · ")}
          </span>
        )}
        <div className="ml-auto flex items-center gap-[2px]">
          <button type="button" className={ghost} disabled={readOnly} onClick={() => setAdding(true)}>
            <Plus size={13} />
            {t("Add document")}
          </button>
          <button type="button" className={ghost} disabled={loading} onClick={() => (loadInfo(), void find(applied, skip))}>
            <RefreshCw size={13} />
            {t("Refresh")}
          </button>
        </div>
      </div>

      <form
        className="flex flex-col gap-[6px] border-b border-line bg-bg px-[10.2px] pt-[6.8px] pb-[10.2px]"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <div className="flex items-end gap-s2">
          {field("filter", t("Filter"), "flex-1", "{ }")}
          {field("projection", t("Project"), "w-[150px] flex-none", "{ }")}
          {field("sort", t("Sort"), "w-[190px] flex-none", "{ _id: 1 }")}
          {field("limit", t("Limit"), "w-[70px] flex-none", "50")}
          <Button type="submit" variant="primary" disabled={loading}>
            {t("Find")} ↵
          </Button>
          <Button
            variant="ghost"
            onClick={() => {
              setDraft(EMPTY);
              void find(EMPTY, 0);
            }}
          >
            {t("Reset")}
          </Button>
        </div>
        {invalid && (
          <p role="alert" className="flex items-start gap-[6px] text-[12.5px] text-danger">
            <TriangleAlert size={13} className="mt-[2px] flex-none" />
            <span className="selectable">
              {invalid.part === "filter" ? t("Filter is not valid:") : invalid.part === "sort" ? t("Sort is not valid:") : t("Projection is not valid:")} {invalid.message}
              {invalid.position !== null && ` ${t("at position {position}", { position: invalid.position })}`}. {page && t("The last results stay below.")}
            </span>
          </p>
        )}
      </form>

      {error ? (
        <LoadError message={error} conn={conn} onRetry={() => void find(applied, skip)} />
      ) : !page ? (
        <Loading text={t("Reading {name}…", { name: collection })} />
      ) : documents.length === 0 ? (
        <div className="grid place-items-center">
          <p className="text-text-2">{applied.filter ? t("No documents match the filter.") : t("This collection is empty.")}</p>
        </div>
      ) : mode === "tree" ? (
        <div className="min-h-0 overflow-auto">
          {page.documents.map((found, i) => (
            <DocumentBlock key={idText(found.document._id) + i} ordinal={skip + i + 1} document={found.document} size={found.size_bytes} startOpen={i === 0} editable={!readOnly && !applied.projection} onSet={setField} onDelete={remove} />
          ))}
        </div>
      ) : mode === "table" ? (
        <DocumentTable documents={documents} />
      ) : (
        <JsonText documents={documents} onCopied={() => setNotice({ text: t("Copied"), error: false })} />
      )}

      <footer className="flex items-center gap-s3 border-t border-line bg-bg px-s3 text-[12px] text-text-2">
        <span className="font-mono text-[11.5px]">
          {documents.length === 0
            ? t("No documents")
            : total !== null
              ? applied.filter
                ? t("Documents {first}–{last} of {total} matching", { first: skip + 1, last: skip + documents.length, total })
                : t("Documents {first}–{last} of {total}", { first: skip + 1, last: skip + documents.length, total })
              : t("Documents {first}–{last}", { first: skip + 1, last: skip + documents.length })}
        </span>
        <div className="flex items-center">
          <button type="button" aria-label={t("Previous page")} className="grid size-[22px] place-items-center enabled:hover:bg-hover disabled:opacity-40" disabled={skip === 0 || loading} onClick={() => void find(applied, Math.max(0, skip - limit))}>
            <ChevronLeft size={13} />
          </button>
          <button type="button" aria-label={t("Next page")} className="grid size-[22px] place-items-center enabled:hover:bg-hover disabled:opacity-40" disabled={!page?.has_more || loading} onClick={() => void find(applied, skip + limit)}>
            <ChevronRight size={13} />
          </button>
        </div>
        {invalid && <span className="text-danger">{t("Filter not applied")}</span>}
        <span className="flex-1" />
        {page && <span className="font-mono text-[11.5px]">{page.elapsed_ms} ms</span>}
        <span>{readOnly ? t("Read-only · view") : applied.projection ? t("Read-only while a projection is set") : t("Editable · key _id")}</span>
      </footer>

      {notice && (
        <div role={notice.error ? "alert" : "status"} className={`absolute right-s4 bottom-[42px] z-40 max-w-[560px] border bg-bg px-s3 py-s2 font-mono text-[12px] break-words ${notice.error ? "border-danger text-danger" : "border-line"}`} style={{ boxShadow: "var(--shadow-lg)" }}>
          {notice.text}
        </div>
      )}
      {confirm && (
        <Dialog
          title={confirm.title}
          width={480}
          alert
          nested={adding}
          band={production ? "production" : undefined}
          onClose={() => {
            confirm.cancel?.();
            setConfirm(null);
          }}
          onSubmit={() => {
            const go = confirm.run;
            setConfirm(null);
            go();
          }}
          footer={
            <>
              <Button
                variant="ghost"
                onClick={() => {
                  confirm.cancel?.();
                  setConfirm(null);
                }}
              >
                {t("Cancel")}
              </Button>
              <Button type="submit" variant="danger">
                {confirm.submit}
              </Button>
            </>
          }
        >
          <div className="text-[13px]">{confirm.body}</div>
        </Dialog>
      )}
      {adding && (
        <AddDocument
          production={production}
          onClose={() => setAdding(false)}
          onAdd={(text, after) =>
            write({ op: "insert", document: text }, `db.${collection}.insertOne(${text.length > 300 ? `${text.slice(0, 300)}…` : text})`, t("Document added"), {
              ok: () => {
                setAdding(false);
                after.ok();
              },
              failed: after.failed,
            })
          }
        />
      )}
    </main>
  );
}

interface WriteOutcome {
  ok(): void;
  failed(message: string): void;
}

// Not drawn in the design: the document is written as text, as in the query fields.
// It stays open until the server accepts the document, so a refusal does not lose the text.
function AddDocument({ production, onClose, onAdd }: { production: boolean; onClose(): void; onAdd(text: string, after: WriteOutcome): void }) {
  const [text, setText] = useState('{\n  "name": ""\n}');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const submit = () => {
    if (!text.trim() || busy) return;
    setBusy(true);
    setError(null);
    onAdd(text, {
      ok: () => setBusy(false),
      failed: (message) => {
        setBusy(false);
        setError(message);
      },
    });
  };
  return (
    <Dialog
      title={t("Add document")}
      width={560}
      band={production ? "production" : undefined}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t("Cancel")}
          </Button>
          <Button type="submit" variant="primary" disabled={!text.trim() || busy}>
            {t("Add document")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-s2">
        <textarea
          data-autofocus=""
          aria-label={t("Document")}
          spellCheck={false}
          className="h-[240px] resize-y border border-line bg-panel p-s2 font-mono text-[12.5px] leading-[19px] focus:border-accent focus:outline-none"
          value={text}
          onChange={(e) => {
            setText(e.target.value);
            setError(null);
          }}
          onKeyDown={(e) => e.key === "Enter" && !e.metaKey && !e.ctrlKey && e.stopPropagation()}
        />
        {error ? (
          <p role="alert" className="selectable font-mono text-[12px] break-words text-danger">{error}</p>
        ) : (
          <p className="text-[12px] text-text-2">{t("Extended JSON or shell syntax, such as ObjectId(\"…\") and ISODate(\"…\"). Without an _id the server gives one.")}</p>
        )}
      </div>
    </Dialog>
  );
}

interface BlockProps {
  ordinal: number;
  document: Doc;
  size: number;
  startOpen: boolean;
  editable: boolean;
  onSet(id: unknown, path: string, value: unknown): void;
  onDelete(id: unknown): void;
}

function DocumentBlock({ ordinal, document, size, startOpen, editable, onSet, onDelete }: BlockProps) {
  const [open, setOpen] = useState(startOpen);
  const [selected, setSelected] = useState<string | null>(null);
  const id = document._id;
  const count = Object.keys(document).length;
  return (
    <div className="border-b border-line">
      <div className="group flex h-[24px] cursor-pointer items-center gap-[6px] bg-bg px-s2 font-mono text-[12.5px]" onClick={() => setOpen(!open)}>
        {open ? <ChevronDown size={12} className="flex-none" /> : <ChevronRight size={12} className="flex-none" />}
        <span className="w-[28px] flex-none text-right text-[11px] text-text-3">{ordinal}</span>
        <span className="selectable flex-none" style={{ color: id === undefined ? undefined : TONE[scalarTone(id)] }}>
          {id === undefined ? t("no _id") : idText(id)}
        </span>
        {!open && <span className="min-w-0 flex-1 truncate text-text-3">{preview(document)}</span>}
        <span className="flex-1" />
        {editable && id !== undefined && (
          <button
            type="button"
            aria-label={t("Delete document")}
            title={t("Delete document")}
            className="hidden size-[20px] place-items-center text-text-3 group-hover:grid hover:text-danger focus-visible:grid"
            onClick={(e) => {
              e.stopPropagation();
              onDelete(id);
            }}
          >
            <Trash2 size={12} />
          </button>
        )}
        <span className="flex-none text-[11px] text-text-3">
          {tn(count, "{count} field", "{count} fields")} · {formatBytes(size)}
        </span>
      </div>
      {open && <Fields value={document} depth={1} path="" selected={selected} onSelect={setSelected} onSet={editable && id !== undefined ? (path, value) => onSet(id, path, value) : null} />}
    </div>
  );
}

interface FieldsProps {
  value: unknown;
  depth: number;
  path: string;
  selected: string | null;
  onSelect(path: string): void;
  /** Null where values cannot be written. */
  onSet: ((path: string, value: unknown) => void) | null;
}

function Fields({ value, depth, path, selected, onSelect, onSet }: FieldsProps) {
  const entries: [string, unknown][] = Array.isArray(value) ? value.map((v, i) => [String(i), v]) : Object.entries(value as Doc);
  return (
    <>
      {entries.map(([key, child]) => (
        <FieldRow key={key} name={key} value={child} depth={depth} path={path ? `${path}.${key}` : key} selected={selected} onSelect={onSelect} onSet={onSet} />
      ))}
    </>
  );
}

function FieldRow({ name, value, depth, path, selected, onSelect, onSet }: FieldsProps & { name: string }) {
  const container = isContainer(value);
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const on = selected === path;
  const type = bsonType(value);
  // The _id cannot be changed, and a null has no type to keep.
  const canEdit = !!onSet && !container && path !== "_id" && !("error" in parseAs(scalarText(value), value));

  const commit = () => {
    if (draft === null) return;
    const parsed = parseAs(draft, value);
    if ("error" in parsed) return setProblem(t("Expected {what}.", { what: parsed.error }));
    if (JSON.stringify(parsed.value) !== JSON.stringify(value)) onSet?.(path, parsed.value);
    setDraft(null);
  };

  return (
    <>
      <div
        role="row"
        aria-selected={on}
        className={`flex h-[24px] items-center gap-[6px] pr-s2 font-mono text-[12.5px] ${on ? "bg-accent-tint" : "hover:bg-hover"}`}
        style={{ paddingLeft: 8 + depth * 18, boxShadow: on ? "inset 2px 0 0 var(--accent)" : undefined }}
        onClick={() => onSelect(path)}
      >
        {container ? (
          <button type="button" aria-label={open ? t("Collapse") : t("Expand")} className="grid size-[14px] flex-none place-items-center" onClick={() => setOpen(!open)}>
            {open ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
          </button>
        ) : (
          <i className="w-[14px] flex-none" />
        )}
        <span className="flex-none" style={{ color: "var(--syn-kw)" }}>
          {name}
        </span>
        <span className="flex-none text-text-3">:</span>
        {container ? (
          <span className="min-w-0 flex-1 truncate text-text-3" onDoubleClick={() => setOpen(!open)}>
            {open ? "" : Array.isArray(value) ? value.slice(0, 6).map((v) => (isContainer(v) ? typeLabel(v) : scalarText(v))).join(", ") : Object.keys(value as Doc).join(", ")}
          </span>
        ) : draft !== null ? (
          <>
            <input
              autoFocus
              aria-label={name}
              className={`h-[20px] min-w-0 flex-1 border bg-panel px-[4px] font-mono text-[12.5px] focus:outline-none ${problem ? "border-danger" : "border-accent"}`}
              value={draft}
              onFocus={(e) => e.currentTarget.select()}
              onChange={(e) => (setProblem(null), setDraft(e.target.value))}
              onBlur={() => (setDraft(null), setProblem(null))}
              onKeyDown={(e) => {
                if (e.key === "Enter") commit();
                if (e.key === "Escape") (setDraft(null), setProblem(null));
              }}
            />
            {problem && <span className="flex-none text-[11.5px] text-danger">{problem}</span>}
          </>
        ) : (
          <span className="selectable min-w-0 flex-1 truncate" style={{ color: TONE[scalarTone(value)] }} title={canEdit ? t("Double-click to edit") : undefined} onDoubleClick={() => canEdit && setDraft(scalarText(value))}>
            {type === "string" ? JSON.stringify(value) : scalarText(value)}
          </span>
        )}
        <span className="flex-none text-[11px] text-text-3">{typeLabel(value)}</span>
      </div>
      {container && open && <Fields value={value} depth={depth + 1} path={path} selected={selected} onSelect={onSelect} onSet={onSet} />}
    </>
  );
}

function DocumentTable({ documents }: { documents: Doc[] }) {
  const columns = useMemo(() => tableColumns(documents), [documents]);
  return (
    <div className="min-h-0 overflow-auto">
      <table className="border-collapse font-mono text-[12.5px]">
        <thead>
          <tr>
            {columns.map((column) => (
              <th key={column} className="sticky top-0 z-10 h-row border-r border-b border-line bg-bg px-s2 text-left text-[12px] font-medium whitespace-nowrap">
                {column}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {documents.map((document, i) => (
            <tr key={i} className="hover:bg-hover">
              {columns.map((column) => {
                const value = cellAt(document, column);
                return (
                  <td key={column} className="h-row max-w-[320px] truncate border-r border-b border-line px-s2 whitespace-nowrap">
                    {value === undefined ? null : value === null ? (
                      <Tag>NULL</Tag>
                    ) : isContainer(value) ? (
                      // The cell viewer for these is not built yet; the whole value is in the tooltip.
                      <span className="border border-line px-[5px] py-[1px] text-[11px] text-text-2" title={JSON.stringify(value, null, 2).slice(0, 2000)}>
                        {typeLabel(value)}
                      </span>
                    ) : bsonType(value) === "objectId" ? (
                      <span className="selectable" title={scalarText(value)}>
                        <span className="text-text-3">{scalarText(value).slice(0, 18)}</span>
                        {scalarText(value).slice(18)}
                      </span>
                    ) : (
                      <span className="selectable" style={{ color: TONE[scalarTone(value)] }}>
                        {scalarText(value)}
                      </span>
                    )}
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

// Not drawn in the design: the page as one read-only text.
function JsonText({ documents, onCopied }: { documents: Doc[]; onCopied(): void }) {
  const text = useMemo(() => JSON.stringify(documents, null, 2), [documents]);
  return (
    <div className="relative min-h-0">
      <pre className="selectable h-full overflow-auto p-s3 font-mono text-[12.5px] leading-[19px]">{text}</pre>
      <button
        type="button"
        className="absolute top-s2 right-s4 flex h-[24px] items-center gap-[6px] border border-line bg-bg px-s2 text-[12px] hover:bg-hover"
        onClick={() => {
          void navigator.clipboard?.writeText(text);
          onCopied();
        }}
      >
        <Copy size={12} />
        {t("Copy")}
      </button>
    </div>
  );
}
