// The export wizard (X01, X02). Options for CSV and JSON, the progress step
// and the summary are not drawn in the design: they reuse the wizard's patterns.
import { save } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { Check, FileOutput, GripVertical, TriangleAlert } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { api, errorMessage, inTauri, type ExportFormat, type QueryResult, type Value } from "../lib/api";
import { dialectOf, quoteIdent } from "../lib/sql";
import { openDialog, useApp, type Conn, type Dialog as DialogState } from "../store";
import { Button, Checkbox, Dialog, Field, Segmented, Strip, Switch, TextInput } from "./dialog";
import { Spinner } from "./Results";
import { NOT_BUILT, Tag } from "./ui";
import { msg, t, tn } from "../i18n";

type Source = Extract<DialogState, { kind: "export" }>["source"];
type Kind = "csv" | "json" | "xlsx" | "sql";

const FORMATS: { kind: Kind; badge: string; name: string; line: string; extension: string }[] = [
  { kind: "csv", badge: "CSV", name: msg("Delimited text"), line: msg("Opens in spreadsheets and most tools."), extension: "csv" },
  { kind: "json", badge: "JSON", name: "JSON", line: msg("One object per row, keyed by column."), extension: "json" },
  { kind: "xlsx", badge: "XLSX", name: "Excel", line: msg("A workbook with one sheet."), extension: "xlsx" },
  { kind: "sql", badge: "SQL", name: msg("SQL statements"), line: msg("INSERT statements that rebuild the rows."), extension: "sql" },
];
const STEPS = [msg("Source and format"), msg("Columns and preview"), msg("Export"), msg("Summary")];
const DELIMITERS = [
  { value: ",", label: msg("Comma") },
  { value: ";", label: msg("Semicolon") },
  { value: "\t", label: msg("Tab") },
  { value: "|", label: msg("Pipe") },
];
const PREVIEW_ROWS = 20;

interface Column {
  source: string;
  name: string;
  include: boolean;
}

type Run = { state: "idle" } | { state: "running" } | { state: "ok"; rows: number; ms: number } | { state: "failed"; message: string };

export function ExportDialog({ conn, source }: { conn: Conn; source: Source }) {
  const tab = useApp((s) => (source.kind === "table" ? s.tabs.find((x) => x.id === source.tabId) : undefined));
  const view = tab?.table ?? null;
  const dialect = dialectOf(conn.driver.id);
  // What is on screen: the columns, and rows to show in the preview.
  const loaded: QueryResult | null = source.kind === "result" ? source.result : view?.page ? { columns: view.page.columns, rows: view.page.rows, rows_affected: null, truncated: false, elapsed_ms: 0 } : null;
  const tableName = view ? [view.namespace, view.name].filter(Boolean).join(".") : source.kind === "result" ? source.title : "";
  const baseName = (view?.name ?? (source.kind === "result" ? source.title : "export")).replace(/[^\w.-]+/g, "_").toLowerCase() || "export";

  const [step, setStep] = useState(0);
  const [kind, setKind] = useState<Kind>("csv");
  const [csv, setCsv] = useState({ delimiter: ",", header: true, null_text: "", crlf: false, bom: false });
  const [lines, setLines] = useState(false);
  const [sqlTable, setSqlTable] = useState(() => (view ? [view.namespace, view.name].filter(Boolean).map((part) => quoteIdent(part, dialect)).join(".") : quoteIdent(baseName, dialect)));
  const [path, setPath] = useState("");
  const [columns, setColumns] = useState<Column[]>(() => (loaded?.columns ?? []).map((c) => ({ source: c.name, name: c.name, include: true })));
  const [dragged, setDragged] = useState<number | null>(null);
  const [preview, setPreview] = useState<{ text: string } | { error: string } | null>(null);
  const [run, setRun] = useState<Run>({ state: "idle" });

  const format = useMemo<ExportFormat>(() => {
    if (kind === "csv") return { format: "csv", ...csv };
    if (kind === "sql") return { format: "sql", table: sqlTable, dialect };
    return { format: lines ? "json_lines" : "json" };
  }, [kind, csv, lines, sqlTable, dialect]);
  const extension = kind === "json" && lines ? "jsonl" : FORMATS.find((f) => f.kind === kind)!.extension;
  const chosen = columns.filter((c) => c.include);
  const names = chosen.map((c) => c.name.trim());
  const namesValid = chosen.length > 0 && names.every((name) => name !== "") && new Set(names).size === names.length;

  /** The loaded rows with only the chosen columns, in the chosen order. */
  const project = (rows: Value[][]): Value[][] => {
    const index = chosen.map((c) => loaded!.columns.findIndex((x) => x.name === c.source));
    return rows.map((row) => index.map((i) => row[i]));
  };

  // The preview is written by the same code that writes the file.
  useEffect(() => {
    if (step !== 1 || !loaded) return;
    if (!namesValid) {
      setPreview(null);
      return;
    }
    let current = true;
    api
      .exportPreview(names, project(loaded.rows.slice(0, PREVIEW_ROWS)), format)
      .then((text) => current && setPreview({ text }))
      .catch((e) => current && setPreview({ error: errorMessage(e) }));
    return () => {
      current = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step, format, JSON.stringify(columns)]);

  const close = () => openDialog(null);
  if (!loaded) return null;

  const total = source.kind === "table" ? (typeof view?.total === "number" ? view.total : null) : loaded.rows.length;
  const sentence =
    source.kind === "table"
      ? view?.filter
        ? t("Every row of {table} that matches the current filter, {columns} columns. Rows are read from the database again, in pages.", { table: tableName, columns: loaded.columns.length })
        : t("Every row of {table}, {columns} columns. Rows are read from the database again, in pages.", { table: tableName, columns: loaded.columns.length })
      : loaded.truncated
        ? t("{rows} rows, {columns} columns. Rows beyond the limit are not included.", { rows: loaded.rows.length, columns: loaded.columns.length })
        : t("{rows} rows, {columns} columns.", { rows: loaded.rows.length, columns: loaded.columns.length });

  async function browse() {
    const picked = await save({ defaultPath: `${baseName}.${extension}`, filters: [{ name: extension.toUpperCase(), extensions: [extension] }] }).catch(() => null);
    if (picked) setPath(picked);
  }

  async function start() {
    if (conn.session === null) return;
    setStep(2);
    setRun({ state: "running" });
    const started = performance.now();
    try {
      const rows =
        source.kind === "table" && view
          ? await api.exportTable(conn.session, { namespace: view.namespace || undefined, table: view.name, order_by: view.sort ? [view.sort] : [], filter: view.filter || undefined }, format, path, chosen.map((c) => ({ source: c.source, name: c.name.trim() })))
          : await api.exportRows(names, project(loaded!.rows), format, path);
      setRun({ state: "ok", rows, ms: Math.round(performance.now() - started) });
      setStep(3);
    } catch (e) {
      setRun({ state: "failed", message: errorMessage(e) });
    }
  }

  const move = (from: number, to: number) => {
    if (from === to) return;
    const next = [...columns];
    next.splice(to, 0, next.splice(from, 1)[0]);
    setColumns(next);
  };

  const size = preview && "text" in preview && total !== null && loaded.rows.length > 0 ? Math.round((new Blob([preview.text]).size / Math.min(PREVIEW_ROWS, loaded.rows.length)) * total) : null;
  const sizeText = size === null ? null : size < 1024 ? `${size} B` : size < 1024 * 1024 ? `${(size / 1024).toFixed(1)} KB` : `${(size / 1024 / 1024).toFixed(1)} MB`;

  const status =
    step === 0
      ? t("Nothing has been written yet.")
      : step === 1
        ? tn(chosen.length, "{count} column will be written.", "{count} columns will be written.")
        : run.state === "running"
          ? t("Writing the file…")
          : "";

  return (
    <Dialog
      title={source.kind === "table" ? t("Export {table}", { table: tableName }) : t("Export result")}
      headerExtra={conn.environment === "production" ? <Tag kind="warn">{t("PRODUCTION")}</Tag> : undefined}
      band={conn.environment === "production" ? "production" : undefined}
      width={900}
      offset={-26}
      bodyClassName=""
      onClose={close}
      footer={
        <>
          <span className="mr-auto min-w-0 truncate text-[12px] text-text-2">{status}</span>
          {(step === 1 || (step === 2 && run.state === "failed")) && (
            <Button variant="ghost" onClick={() => setStep(step === 2 ? 1 : 0)}>
              {t("Back")}
            </Button>
          )}
          {step < 3 && <Button onClick={close}>{run.state === "running" ? t("Close") : t("Cancel")}</Button>}
          {step === 0 && (
            <Button variant="primary" disabled={path.trim() === "" || kind === "xlsx"} onClick={() => setStep(1)}>
              {t("Next: columns")}
            </Button>
          )}
          {step === 1 && (
            <Button variant="primary" disabled={!namesValid || conn.session === null} onClick={start}>
              {t("Export")}
            </Button>
          )}
          {step === 2 && run.state === "failed" && (
            <Button variant="primary" onClick={start}>
              {t("Retry")}
            </Button>
          )}
          {step === 3 && (
            <>
              {inTauri && <Button onClick={() => void revealItemInDir(path)}>{t("Show in folder")}</Button>}
              <Button variant="primary" onClick={close}>
                {t("Done")}
              </Button>
            </>
          )}
        </>
      }
    >
      <ol className="flex h-[40px] items-center gap-s2 border-b border-line bg-surface px-[20px] whitespace-nowrap">
        {STEPS.map((label, i) => {
          const done = i < step;
          const current = i === step;
          const canGoBack = done && step < 2;
          return (
            <li key={label} className="flex items-center gap-s2">
              {i > 0 && <i className="h-px w-[28px] bg-line" />}
              <button type="button" disabled={!canGoBack} aria-current={current ? "step" : undefined} className={`flex items-center gap-[6px] text-[12.5px] ${current ? "font-medium" : done ? "text-text-2" : "text-text-3"} ${canGoBack ? "hover:text-text" : ""}`} onClick={() => setStep(i)}>
                <span className={`grid size-[18px] place-items-center font-mono text-[11px] ${current ? "bg-accent-fill text-on-accent" : done ? "bg-text-2 text-bg" : "border border-text-3"}`}>{done ? <Check size={11} /> : i + 1}</span>
                {t(label)}
              </button>
            </li>
          );
        })}
      </ol>

      <div className="flex h-[500px] flex-col gap-[13.6px] overflow-auto p-[20px]">
        {step === 0 && (
          <>
            <div className="flex flex-col gap-[6px]">
              <span className="label">{t("Export")}</span>
              <div className="flex items-center gap-s3">
                <Segmented
                  label={t("Export")}
                  value={source.kind === "result" && source.selection ? ("selection" as Source["kind"]) : source.kind}
                  onChange={() => {}}
                  options={[
                    { value: "result", label: t("This result"), disabled: source.kind !== "result" || !!source.selection },
                    { value: "table", label: t("Whole table"), disabled: source.kind !== "table" },
                    { value: "selection" as Source["kind"], label: t("Selected rows"), disabled: !(source.kind === "result" && source.selection) },
                  ]}
                />
                <span className="min-w-0 text-[12.5px] text-text-2">{sentence}</span>
              </div>
            </div>

            <div className="flex flex-col gap-[6px]">
              <span className="label">{t("Format")}</span>
              <div className="grid grid-cols-4 gap-s3">
                {FORMATS.map((f) => {
                  const selected = f.kind === kind;
                  const unavailable = f.kind === "xlsx";
                  return (
                    <button
                      key={f.kind}
                      type="button"
                      aria-pressed={selected}
                      disabled={unavailable}
                      title={unavailable ? t(NOT_BUILT) : undefined}
                      className={`relative flex flex-col items-start gap-[6px] border p-s3 text-left disabled:cursor-not-allowed disabled:opacity-45 ${selected ? "border-accent bg-accent-tint outline outline-1 outline-accent" : "border-line enabled:hover:bg-hover"}`}
                      onClick={() => setKind(f.kind)}
                    >
                      <span className="border border-text-3 px-[5px] py-[2px] font-mono text-[10.5px] leading-none font-medium">{f.badge}</span>
                      <span className="text-[13px] font-medium">{t(f.name)}</span>
                      <span className="text-[12px] leading-[1.35] text-text-2">{t(f.line)}</span>
                      {selected && (
                        <span className="absolute top-s2 right-s2 grid size-[14px] place-items-center bg-accent-fill text-on-accent">
                          <Check size={10} />
                        </span>
                      )}
                    </button>
                  );
                })}
              </div>
            </div>

            <div className="flex flex-wrap items-end gap-x-[20px] gap-y-s3">
              {kind === "csv" && (
                <>
                  <div className="flex flex-col gap-[6px]">
                    <span className="label">{t("Delimiter")}</span>
                    <Segmented label={t("Delimiter")} value={csv.delimiter} onChange={(delimiter) => setCsv({ ...csv, delimiter })} options={DELIMITERS.map((d) => ({ value: d.value, label: t(d.label) }))} />
                  </div>
                  <Field label={t("NULL as")} className="w-[140px]">
                    <TextInput mono value={csv.null_text} placeholder={t("nothing")} onChange={(e) => setCsv({ ...csv, null_text: e.target.value })} />
                  </Field>
                  <div className="flex h-[28px] items-center gap-[20px]">
                    <Switch checked={csv.header} onChange={(header) => setCsv({ ...csv, header })}>
                      {t("Header row")}
                    </Switch>
                    <Checkbox checked={csv.crlf} onChange={(crlf) => setCsv({ ...csv, crlf })}>
                      {t("Windows line endings")}
                    </Checkbox>
                    <Checkbox checked={csv.bom} onChange={(bom) => setCsv({ ...csv, bom })}>
                      {t("Byte-order mark, for Excel")}
                    </Checkbox>
                  </div>
                </>
              )}
              {kind === "json" && (
                <div className="flex flex-col gap-[6px]">
                  <span className="label">{t("Layout")}</span>
                  <Segmented
                    label={t("Layout")}
                    value={lines ? "lines" : "array"}
                    onChange={(value) => setLines(value === "lines")}
                    options={[
                      { value: "array", label: t("One array") },
                      { value: "lines", label: t("One object per line") },
                    ]}
                  />
                </div>
              )}
              {kind === "sql" && (
                <Field label={t("Table name in the file")} helper={t("Written as it is, so quote it the way the target database expects.")} className="w-[420px]">
                  <TextInput mono value={sqlTable} onChange={(e) => setSqlTable(e.target.value)} />
                </Field>
              )}
            </div>

            <Field label={t("Save to")} className="mt-auto">
              <div className="flex gap-s2">
                <TextInput mono className="flex-1" value={path} placeholder={`/…/${baseName}.${extension}`} onChange={(e) => setPath(e.target.value)} />
                <Button disabled={!inTauri} onClick={browse}>
                  {t("Browse…")}
                </Button>
              </div>
            </Field>
          </>
        )}

        {step === 1 && (
          <div className="grid min-h-0 flex-1 grid-cols-[360px_minmax(0,1fr)] gap-[20px]">
            <div className="flex min-h-0 flex-col border border-line">
              <div className="grid h-row flex-none grid-cols-[24px_24px_minmax(0,1fr)_minmax(0,1fr)] items-center gap-s2 border-b border-line bg-bg px-s2">
                <span />
                <span />
                <span className="label">{t("Column")}</span>
                <span className="label">{t("Name in file")}</span>
              </div>
              <div className="min-h-0 overflow-auto">
                {columns.map((column, i) => (
                  <div
                    key={column.source}
                    draggable
                    onDragStart={() => setDragged(i)}
                    onDragEnd={() => setDragged(null)}
                    onDragOver={(e) => e.preventDefault()}
                    onDrop={() => {
                      if (dragged !== null) move(dragged, i);
                      setDragged(null);
                    }}
                    className={`grid h-[32px] grid-cols-[24px_24px_minmax(0,1fr)_minmax(0,1fr)] items-center gap-s2 border-b border-line-2 px-s2 ${column.include ? "" : "text-text-3"} ${dragged === i ? "opacity-60" : ""}`}
                  >
                    <GripVertical size={13} className="cursor-grab text-text-3" aria-label={t("Drag to reorder")} />
                    <Checkbox checked={column.include} onChange={(include) => setColumns(columns.map((c, j) => (j === i ? { ...c, include } : c)))}>
                      <span className="sr-only">{column.source}</span>
                    </Checkbox>
                    <span className="truncate font-mono text-[12px]">{column.source}</span>
                    <input
                      value={column.name}
                      disabled={!column.include}
                      spellCheck={false}
                      aria-label={t("Name in file")}
                      onChange={(e) => setColumns(columns.map((c, j) => (j === i ? { ...c, name: e.target.value } : c)))}
                      className="h-[24px] min-w-0 border border-line bg-panel px-s2 font-mono text-[12px] outline-none focus:border-accent disabled:border-transparent disabled:bg-transparent"
                    />
                  </div>
                ))}
              </div>
            </div>
            <div className="flex min-h-0 min-w-0 flex-col gap-s2">
              <div className="flex items-baseline gap-s3">
                <span className="label">{t("Preview")}</span>
                <span className="text-[12px] text-text-2">{tn(Math.min(PREVIEW_ROWS, loaded.rows.length), "The first row, exactly as it will be written.", "The first {count} rows, exactly as they will be written.")}</span>
                {sizeText && <span className="ml-auto font-mono text-[11.5px] text-text-2">{t("about {size} in all", { size: sizeText })}</span>}
              </div>
              {!namesValid ? (
                <Strip kind="warning" icon={<TriangleAlert size={13} />}>
                  {chosen.length === 0 ? t("Choose at least one column.") : t("Every column needs its own name in the file.")}
                </Strip>
              ) : preview && "error" in preview ? (
                <Strip kind="error" icon={<TriangleAlert size={13} />}>
                  <span className="selectable font-mono text-[12px]">{preview.error}</span>
                </Strip>
              ) : (
                <pre className="selectable min-h-0 flex-1 overflow-auto border border-line bg-bg p-s3 font-mono text-[12px] leading-[18px] whitespace-pre">{preview && "text" in preview ? preview.text : ""}</pre>
              )}
            </div>
          </div>
        )}

        {step === 2 && (
          <div className="flex flex-1 flex-col justify-center gap-s3">
            {run.state === "failed" ? (
              <Strip kind="error" icon={<TriangleAlert size={13} />}>
                <span className="font-medium">{t("The export failed. No file was left behind.")}</span>
                <p className="selectable mt-[4px] font-mono text-[12px] break-words whitespace-pre-wrap">{run.message}</p>
              </Strip>
            ) : (
              <>
                <div className="flex items-center gap-s3">
                  <Spinner />
                  <span className="font-heading text-[20px] font-semibold">{t("Exporting {name}", { name: tableName })}</span>
                </div>
                <div className="run-bar" />
                <p className="selectable font-mono text-[12px] text-text-2">{path}</p>
              </>
            )}
          </div>
        )}

        {step === 3 && run.state === "ok" && (
          <>
            <Strip kind="success" icon={<Check size={13} />}>
              {tn(run.rows, "{count} row written from {name} on {connection}.", "{count} rows written from {name} on {connection}.", { name: tableName, connection: conn.name })}
            </Strip>
            <div className="grid grid-cols-3 gap-s3">
              {[
                [String(run.rows), t("Rows")],
                [String(chosen.length), t("Columns")],
                [run.ms < 1000 ? `${run.ms} ms` : `${(run.ms / 1000).toFixed(1)} s`, t("Duration")],
              ].map(([figure, label]) => (
                <div key={label} className="flex flex-col gap-[4px] border border-line p-s3">
                  <span className="font-heading text-[28px] leading-none font-semibold">{figure}</span>
                  <span className="label">{label}</span>
                </div>
              ))}
            </div>
            <div className="flex items-center gap-s2 text-[12.5px]">
              <FileOutput size={13} className="flex-none text-text-3" />
              <span className="selectable min-w-0 font-mono text-[12px] break-all">{path}</span>
            </div>
          </>
        )}
      </div>
    </Dialog>
  );
}
