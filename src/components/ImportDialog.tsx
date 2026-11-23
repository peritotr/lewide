// The import wizard (I01–I05) for CSV and JSON files, appending to one table.
// Excel and SQL files, the Replace and Update-by-key modes, a new table from
// the file, live progress and stopping are not built.
import { open } from "@tauri-apps/plugin-dialog";
import { ArrowRight, Check, FileInput, TriangleAlert } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { api, displayValue, errorMessage, inTauri, type ColumnDescription, type ImportCheck, type ImportFormat, type ImportInspection, type ImportMapping, type ImportSummary } from "../lib/api";
import { msg, t, tn } from "../i18n";
import { loadPage, openDialog, type Conn, type TableView } from "../store";
import { Button, Checkbox, Dialog, Field, Segmented, Strip, Switch, TextInput } from "./dialog";
import { Spinner } from "./Results";
import { NOT_BUILT, Tag } from "./ui";

type Kind = "csv" | "json" | "xlsx" | "sql";
const FORMATS: { kind: Kind; badge: string; name: string; line: string }[] = [
  { kind: "csv", badge: "CSV", name: msg("Delimited text"), line: msg("Comma, semicolon, tab or pipe separated.") },
  { kind: "json", badge: "JSON", name: "JSON", line: msg("An array of objects, or one object per line.") },
  { kind: "xlsx", badge: "XLSX", name: "Excel", line: msg("A sheet of a workbook.") },
  { kind: "sql", badge: "SQL", name: msg("SQL statements"), line: msg("Statements are run as they are.") },
];
const STEPS = [msg("File and format"), msg("Columns"), msg("Preview"), msg("Import"), msg("Summary")];
const DELIMITERS = [
  { value: ",", label: msg("Comma") },
  { value: ";", label: msg("Semicolon") },
  { value: "\t", label: msg("Tab") },
  { value: "|", label: msg("Pipe") },
];
const SKIP = "";

const normal = (name: string) => name.toLowerCase().replace(/[^a-z0-9]/g, "");
const bytesText = (bytes: number) => (bytes < 1024 ? `${bytes} B` : bytes < 1024 * 1024 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1024 / 1024).toFixed(1)} MB`);
const select = "h-[26px] min-w-0 border border-line bg-panel px-s2 font-mono text-[12px] outline-none hover:border-text-3 focus:border-accent";

type Run = { state: "idle" } | { state: "running" } | { state: "ok"; summary: ImportSummary } | { state: "failed"; message: string };

export function ImportDialog({ conn, tabId, view }: { conn: Conn; tabId: string; view: TableView }) {
  const columns: ColumnDescription[] = view.page?.columns ?? [];
  const tableName = [view.namespace, view.name].filter(Boolean).join(".");
  const target = { namespace: view.namespace || undefined, table: view.name };
  const production = conn.environment === "production";

  const [step, setStep] = useState(0);
  const [path, setPath] = useState("");
  const [kind, setKind] = useState<Kind>("csv");
  const [csv, setCsv] = useState({ delimiter: ",", header: true, trim: true, nulls: "NULL, \\N" });
  const [lines, setLines] = useState(false);
  const [excel, setExcel] = useState<{ sheet: string | null; header: boolean }>({ sheet: null, header: true });
  const [inspection, setInspection] = useState<ImportInspection | { error: string } | "loading" | null>(null);
  // Per file column: the table column it feeds, or SKIP. `manual` marks the ones the user chose.
  const [targets, setTargets] = useState<string[]>([]);
  const [manual, setManual] = useState<boolean[]>([]);
  const [check, setCheck] = useState<ImportCheck | { error: string } | "loading" | null>(null);
  const [errorsOnly, setErrorsOnly] = useState(false);
  const [skipBad, setSkipBad] = useState(true);
  // Without transactions an import cannot be undone as a whole.
  const transactions = conn.driver.capabilities.transactions;
  const [oneTransaction, setOneTransaction] = useState(transactions);
  const [typed, setTyped] = useState("");
  const [run, setRun] = useState<Run>({ state: "idle" });
  const [report, setReport] = useState<string | null>(null);

  const format = useMemo<ImportFormat>(() => {
    if (kind === "csv") return { format: "csv", delimiter: csv.delimiter, header: csv.header, trim: csv.trim, null_tokens: csv.nulls.split(",").map((x) => x.trim()).filter(Boolean) };
    if (kind === "xlsx") return { format: "xlsx", sheet: excel.sheet, header: excel.header };
    return { format: lines ? "json_lines" : "json" };
  }, [kind, csv, lines, excel]);

  const autoMatch = (file: string[]) => file.map((name) => columns.find((c) => normal(c.name) === normal(name))?.name ?? SKIP);

  // The file is read again whenever the path or the way to read it changes.
  useEffect(() => {
    if (step !== 0) return;
    if (path.trim() === "" || kind === "sql") {
      setInspection(null);
      return;
    }
    let current = true;
    setInspection("loading");
    const timer = window.setTimeout(() => {
      api
        .importInspect(path.trim(), format)
        .then((result) => {
          if (!current) return;
          setInspection(result);
          setTargets(autoMatch(result.columns));
          setManual(result.columns.map(() => false));
        })
        .catch((e) => current && setInspection({ error: errorMessage(e) }));
    }, 300);
    return () => {
      current = false;
      window.clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path, format, step]);

  const file = inspection && inspection !== "loading" && "columns" in inspection ? inspection : null;
  const mapping: ImportMapping[] = targets.flatMap((column, source) => (column === SKIP ? [] : [{ source, column }]));
  const duplicated = mapping.find((m, i) => mapping.findIndex((o) => o.column === m.column) !== i)?.column ?? null;
  // ClickHouse fills a column left out with its type's default, so none is required there.
  const implicitDefaults = conn.driver.id === "clickhouse";
  const required = implicitDefaults ? [] : columns.filter((c) => !c.nullable && c.default_value === null && !mapping.some((m) => m.column === c.name)).map((c) => c.name);
  const unfilled = columns.filter((c) => !mapping.some((m) => m.column === c.name) && !required.includes(c.name)).map((c) => c.name);
  const mappingValid = mapping.length > 0 && !duplicated && required.length === 0;

  useEffect(() => {
    if (step !== 2 || conn.session === null) return;
    let current = true;
    setCheck("loading");
    api
      .importCheck(conn.session, target, path.trim(), format, mapping)
      .then((result) => current && setCheck(result))
      .catch((e) => current && setCheck({ error: errorMessage(e) }));
    return () => {
      current = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step]);
  const checked = check && check !== "loading" && "rows" in check ? check : null;

  const close = () => openDialog(null);

  async function browse() {
    const picked = await open({ multiple: false, filters: [{ name: t("Data files"), extensions: ["csv", "tsv", "txt", "json", "jsonl", "ndjson", "xlsx", "xlsm", "xls", "ods"] }] }).catch(() => null);
    if (typeof picked === "string") choose(picked);
  }
  function choose(next: string) {
    setPath(next);
    // The format follows the file's extension.
    if (/\.(jsonl|ndjson)$/i.test(next)) {
      setKind("json");
      setLines(true);
    } else if (/\.json$/i.test(next)) {
      setKind("json");
      setLines(false);
    } else if (/\.(xlsx|xlsm|xls|ods)$/i.test(next)) {
      setKind("xlsx");
      setExcel({ sheet: null, header: true });
    } else if (/\.(csv|tsv|txt)$/i.test(next)) {
      setKind("csv");
      if (/\.tsv$/i.test(next)) setCsv((c) => ({ ...c, delimiter: "\t" }));
    }
  }

  async function start() {
    if (conn.session === null) return;
    setStep(3);
    setRun({ state: "running" });
    try {
      const summary = await api.importRun(conn.session, target, path.trim(), format, mapping, { skip_bad_rows: skipBad, one_transaction: oneTransaction });
      setRun({ state: "ok", summary });
      setStep(4);
      void loadPage(tabId);
    } catch (e) {
      setRun({ state: "failed", message: errorMessage(e) });
    }
  }

  async function saveReport(summary: ImportSummary) {
    const out = `${path.trim()}.errors.csv`;
    const text = (value: string | number | null) => (value === null ? ({ type: "null" } as const) : ({ type: "text", value: String(value) } as const));
    await api
      .exportRows(["line", "column", "value", "reason"], summary.skipped.map((e) => [text(e.line), text(e.column), text(e.value), text(e.reason)]), { format: "csv" }, out)
      .then(() => setReport(out))
      .catch((e) => setReport(errorMessage(e)));
  }

  const total = file?.total_rows ?? 0;
  const status =
    step === 0
      ? t("Nothing has been written yet.")
      : step === 1
        ? tn(mapping.length, "{count} column will be imported.", "{count} columns will be imported.")
        : step === 2
          ? t("Nothing has been written yet.")
          : run.state === "running"
            ? oneTransaction
              ? t("Writing in one transaction…")
              : t("Writing…")
            : "";

  return (
    <Dialog
      title={t("Import into {table}", { table: tableName })}
      headerExtra={production ? <Tag kind="warn">{t("PRODUCTION")}</Tag> : undefined}
      band={production ? "production" : undefined}
      width={900}
      offset={-26}
      bodyClassName=""
      onClose={close}
      footer={
        <>
          <span className="mr-auto min-w-0 truncate text-[12px] text-text-2">{status}</span>
          {(step === 1 || step === 2 || (step === 3 && run.state === "failed")) && (
            <Button variant="ghost" onClick={() => setStep(step === 3 ? 2 : step - 1)}>
              {t("Back")}
            </Button>
          )}
          {step < 4 && run.state !== "running" && <Button onClick={close}>{t("Cancel")}</Button>}
          {step === 0 && (
            <Button variant="primary" disabled={!file || total === 0} onClick={() => setStep(1)}>
              {t("Next: columns")}
            </Button>
          )}
          {step === 1 && (
            <Button variant="primary" disabled={!mappingValid} onClick={() => setStep(2)}>
              {t("Next: preview")}
            </Button>
          )}
          {step === 2 && (
            <Button variant="primary" disabled={!checked || conn.session === null || (production && typed !== conn.name)} onClick={start}>
              {tn(total, "Import {count} row", "Import {count} rows")}
            </Button>
          )}
          {step === 3 && run.state === "failed" && (
            <Button variant="primary" onClick={start}>
              {t("Retry")}
            </Button>
          )}
          {step === 4 && (
            <Button variant="primary" onClick={close}>
              {t("Done")}
            </Button>
          )}
        </>
      }
    >
      <ol className="flex h-[40px] items-center gap-s2 border-b border-line bg-surface px-[20px] whitespace-nowrap">
        {STEPS.map((label, i) => {
          const done = i < step;
          const current = i === step;
          const canGoBack = done && step < 3;
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
            <div className="flex items-center gap-s3 border border-dashed border-text-3 p-s3">
              <FileInput size={16} className="flex-none text-text-3" />
              <TextInput mono className="flex-1" value={path} placeholder={t("Path of the file to import")} aria-label={t("Path of the file to import")} onChange={(e) => choose(e.target.value)} />
              {file && <span className="flex-none font-mono text-[11.5px] text-text-2">{bytesText(file.bytes)}</span>}
              <Button disabled={!inTauri} onClick={browse}>
                {t("Choose file…")}
              </Button>
            </div>

            <div className="flex flex-col gap-[6px]">
              <span className="label">{t("Format")}</span>
              <div className="grid grid-cols-4 gap-s3">
                {FORMATS.map((f) => {
                  const selected = f.kind === kind;
                  const unavailable = f.kind === "sql";
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
                  <Field label={t("Read as NULL")} className="w-[180px]">
                    <TextInput mono value={csv.nulls} onChange={(e) => setCsv({ ...csv, nulls: e.target.value })} />
                  </Field>
                  <div className="flex h-[28px] items-center gap-[20px]">
                    <Switch checked={csv.header} onChange={(header) => setCsv({ ...csv, header })}>
                      {t("First row is a header")}
                    </Switch>
                    <Checkbox checked={csv.trim} onChange={(trim) => setCsv({ ...csv, trim })}>
                      {t("Trim spaces around values")}
                    </Checkbox>
                  </div>
                </>
              )}
              {kind === "xlsx" && (
                <>
                  {/* Not drawn in the design: the sheet to read, as the CSV options are laid out. */}
                  {file && file.sheets.length > 1 && (
                    <Field label={t("Sheet")} className="w-[220px]">
                      <select aria-label={t("Sheet")} className={select} value={excel.sheet ?? file.sheets[0]} onChange={(e) => setExcel({ ...excel, sheet: e.target.value })}>
                        {file.sheets.map((sheet) => (
                          <option key={sheet} value={sheet}>
                            {sheet}
                          </option>
                        ))}
                      </select>
                    </Field>
                  )}
                  <div className="flex h-[28px] items-center">
                    <Switch checked={excel.header} onChange={(header) => setExcel({ ...excel, header })}>
                      {t("First row is a header")}
                    </Switch>
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
            </div>

            <div className="mt-auto">
              {inspection === "loading" ? (
                <div className="flex items-center gap-s2 text-[12.5px]">
                  <Spinner />
                  {t("Reading the file…")}
                </div>
              ) : inspection && "error" in inspection ? (
                <Strip kind="error" icon={<TriangleAlert size={13} />}>
                  <span className="selectable font-mono text-[12px] break-words">{inspection.error}</span>
                </Strip>
              ) : file ? (
                <Strip kind="note">
                  {kind === "xlsx"
                    ? t("{rows} rows and {columns} columns found. Dates are read as ISO 8601 text.", { rows: file.total_rows, columns: file.columns.length })
                    : t("{rows} rows and {columns} columns found. Quotes are double quotes; the text is read as UTF-8.", { rows: file.total_rows, columns: file.columns.length })}
                </Strip>
              ) : (
                <p className="text-[12.5px] text-text-3">{t("Choose a file to see what it holds.")}</p>
              )}
            </div>
          </>
        )}

        {step === 1 && file && (
          <>
            <div className="flex items-end gap-s4">
              <Field label={t("Import into")} className="w-[260px]">
                <TextInput mono readOnly value={tableName} />
              </Field>
              <div className="flex flex-col gap-[6px]">
                <span className="label">{t("Mode")}</span>
                <Segmented
                  label={t("Mode")}
                  value="append"
                  onChange={() => {}}
                  options={[
                    { value: "append", label: t("Append") },
                    { value: "replace", label: t("Replace all rows"), disabled: true, title: t(NOT_BUILT) },
                    { value: "update", label: t("Update by key"), disabled: true, title: t(NOT_BUILT) },
                  ]}
                />
              </div>
              <span className="pb-[6px] text-[12.5px] text-text-2">{t("Rows are added; existing rows are not touched.")}</span>
            </div>

            <div className="min-h-0 flex-1 overflow-auto border border-line">
              <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)_24px_minmax(0,1fr)_minmax(0,0.8fr)_minmax(0,1fr)]">
                {[t("File column"), t("Sample"), "", t("Table column"), t("Type"), t("Status")].map((header, i) => (
                  <span key={i} className="label sticky top-0 z-[1] flex h-row items-center border-b border-line bg-bg px-[10px]">
                    {header}
                  </span>
                ))}
                {file.columns.map((name, i) => {
                  const chosen = columns.find((c) => c.name === targets[i]);
                  const skipped = !chosen;
                  const sample = file.sample.map((row) => row[i]).find((value) => value !== null && value !== "") ?? null;
                  const cell = `flex h-[32px] min-w-0 items-center border-b border-line-2 px-[10px] ${skipped ? "text-text-3" : ""}`;
                  return (
                    <div key={i} className="contents">
                      <span className={`${cell} font-mono text-[12px] font-medium`}>
                        <span className="truncate">{name}</span>
                      </span>
                      <span className={`${cell} font-mono text-[12px]`}>
                        <span className="truncate">{sample ?? "—"}</span>
                      </span>
                      <span className={cell}>
                        <ArrowRight size={12} className="text-text-3" />
                      </span>
                      <span className={cell}>
                        <select
                          aria-label={t("Table column for {column}", { column: name })}
                          className={`${select} w-full`}
                          value={targets[i] ?? SKIP}
                          onChange={(e) => {
                            setTargets(targets.map((x, j) => (j === i ? e.target.value : x)));
                            setManual(manual.map((x, j) => (j === i ? true : x)));
                          }}
                        >
                          {columns.map((c) => (
                            <option key={c.name} value={c.name}>
                              {c.name}
                            </option>
                          ))}
                          <option value={SKIP}>{t("Skip this column")}</option>
                        </select>
                      </span>
                      <span className={`${cell} font-mono text-[11.5px]`}>
                        <span className="truncate">{chosen?.type_name ?? "—"}</span>
                      </span>
                      <span className={`${cell} gap-[6px] text-[12.5px]`}>
                        {skipped ? (
                          t("Not imported")
                        ) : chosen.name === duplicated ? (
                          <>
                            <TriangleAlert size={12} className="flex-none text-danger" />
                            <span className="truncate text-danger">{t("Mapped twice")}</span>
                          </>
                        ) : manual[i] ? (
                          <>
                            <span className="font-mono font-medium text-accent-text">~</span>
                            {t("Mapped by you")}
                          </>
                        ) : (
                          <>
                            <i className="size-[9px] flex-none bg-ok" />
                            {t("Matched by name")}
                          </>
                        )}
                      </span>
                    </div>
                  );
                })}
              </div>
            </div>

            {required.length > 0 ? (
              <Strip kind="error" icon={<TriangleAlert size={13} />}>
                {t("These table columns need a value and have no default: {columns}. Map a file column to each.", { columns: required.join(", ") })}
              </Strip>
            ) : (
              <p className="text-[12.5px] text-text-2">
                {unfilled.length > 0 ? t("Not in the file, filled with their default or NULL: {columns}.", { columns: unfilled.join(", ") }) : t("Every table column gets a value from the file.")}{" "}
                <button
                  type="button"
                  className="text-accent-text hover:underline"
                  onClick={() => {
                    setTargets(autoMatch(file.columns));
                    setManual(file.columns.map(() => false));
                  }}
                >
                  {t("Match by name again")}
                </button>
              </p>
            )}
          </>
        )}

        {step === 2 && (
          <>
            {check === "loading" || check === null ? (
              <div className="flex items-center gap-s2 text-[12.5px]">
                <Spinner />
                {t("Checking the first rows…")}
              </div>
            ) : "error" in check ? (
              <Strip kind="error" icon={<TriangleAlert size={13} />}>
                <span className="selectable font-mono text-[12px] break-words">{check.error}</span>
              </Strip>
            ) : (
              <>
                <div className="flex items-center gap-s4 text-[12.5px]">
                  <span className="flex items-center gap-[6px]">
                    <i className="size-[9px] bg-ok" />
                    <span className="font-medium">{tn(check.checked - check.errors, "{count} row ready", "{count} rows ready")}</span>
                  </span>
                  {check.errors > 0 && (
                    <span className="flex items-center gap-[6px] text-danger">
                      <TriangleAlert size={12} />
                      {tn(check.errors, "{count} row has an error", "{count} rows have errors")}
                    </span>
                  )}
                  <span className="text-text-2">{check.total_rows > check.checked ? t("The first {checked} of {total} rows are checked here; the whole file is checked during the import.", { checked: check.checked, total: check.total_rows }) : ""}</span>
                  <span className="flex-1" />
                  <Segmented
                    label={t("Rows shown")}
                    value={errorsOnly ? "errors" : "all"}
                    onChange={(value) => setErrorsOnly(value === "errors")}
                    options={[
                      { value: "all", label: t("All rows") },
                      { value: "errors", label: t("Errors only"), disabled: check.errors === 0 },
                    ]}
                  />
                </div>
                <div className="min-h-0 flex-1 overflow-auto border border-line">
                  <table className="w-full border-collapse font-mono text-[12px] whitespace-nowrap">
                    <thead>
                      <tr>
                        <th className="label sticky top-0 h-row border-b border-line bg-bg px-[10px] text-right">#</th>
                        {mapping.map((m) => (
                          <th key={m.column} className="sticky top-0 h-row border-b border-line bg-bg px-[10px] text-left text-[12px] font-medium">
                            {m.column}
                          </th>
                        ))}
                        <th className="sticky top-0 h-row border-b border-line bg-bg" />
                      </tr>
                    </thead>
                    <tbody>
                      {check.rows
                        .filter((row) => !errorsOnly || row.error)
                        .map((row) => (
                          <tr key={row.line} className={row.error ? "bg-danger-tint" : ""}>
                            <td className="h-row border-b border-line-2 px-[10px] text-right text-text-3">
                              {row.error && <TriangleAlert size={11} className="mr-[4px] inline text-danger" />}
                              {row.line}
                            </td>
                            {mapping.map((m, i) => {
                              const bad = row.error?.column === m.column;
                              const value = row.values[i];
                              return (
                                <td key={m.column} className="h-row max-w-[220px] truncate border-b border-line-2 px-[10px]">
                                  {row.error ? (
                                    <span className={bad ? "underline decoration-danger decoration-wavy" : ""}>{row.texts[i] ?? <span className="text-text-3 italic">NULL</span>}</span>
                                  ) : value.type === "null" ? (
                                    <span className="text-text-3 italic">NULL</span>
                                  ) : (
                                    displayValue(value)
                                  )}
                                </td>
                              );
                            })}
                            <td className="h-row border-b border-line-2 px-[10px] font-body text-[12.5px] text-danger">{row.error ? `${row.error.column ?? ""} ${row.error.reason}` : ""}</td>
                          </tr>
                        ))}
                    </tbody>
                  </table>
                </div>
                {check.unfilled.length > 0 && <p className="text-[12.5px] text-text-2">{t("Not in the file, filled with their default or NULL: {columns}.", { columns: check.unfilled.join(", ") })}</p>}
                <div className="flex flex-wrap items-center gap-x-[20px] gap-y-s2">
                  <div className="flex items-center gap-s3">
                    <span className="text-[12.5px]">{t("If a row has an error")}</span>
                    <Segmented
                      label={t("If a row has an error")}
                      value={skipBad ? "skip" : "stop"}
                      onChange={(value) => setSkipBad(value === "skip")}
                      options={[
                        { value: "skip", label: t("Skip the row") },
                        { value: "stop", label: t("Stop the import") },
                      ]}
                    />
                  </div>
                  {transactions ? (
                    <Switch checked={oneTransaction} onChange={setOneTransaction}>
                      {t("Import in one transaction")}
                    </Switch>
                  ) : (
                    <span className="font-medium">{t("This database has no transactions.")}</span>
                  )}
                  <span className="text-[12px] text-text-2">{oneTransaction ? t("A row the database rejects cancels the whole import.") : t("Rows the database rejects are skipped; the others stay.")}</span>
                </div>
                {production && (
                  <label className="flex flex-col gap-[4px]">
                    <span className="text-[12px] text-text-2">{t("Type the connection name to import into Production")}</span>
                    <TextInput mono value={typed} onChange={(e) => setTyped(e.target.value)} after={<span className="font-mono text-[12px] whitespace-nowrap text-text-3">{conn.name}</span>} />
                  </label>
                )}
              </>
            )}
          </>
        )}

        {step === 3 && (
          <div className="flex flex-1 flex-col justify-center gap-s3">
            {run.state === "failed" ? (
              <Strip kind="error" icon={<TriangleAlert size={13} />}>
                <span className="font-medium">{t("The import failed.")}</span>
                <p className="selectable mt-[4px] font-mono text-[12px] break-words whitespace-pre-wrap">{run.message}</p>
              </Strip>
            ) : (
              <>
                <div className="flex items-center gap-s3">
                  <Spinner />
                  <span className="font-heading text-[20px] font-semibold">{t("Importing into {table}", { table: tableName })}</span>
                </div>
                <div className="run-bar" />
                <p className="font-mono text-[12px] text-text-2">{tn(total, "{count} row", "{count} rows")}</p>
              </>
            )}
          </div>
        )}

        {step === 4 && run.state === "ok" && (
          <>
            <Strip kind="success" icon={<Check size={13} />}>
              {tn(run.summary.inserted, "{count} row imported into {table} on {connection}.", "{count} rows imported into {table} on {connection}.", { table: tableName, connection: conn.name })}
            </Strip>
            <div className="grid grid-cols-4 gap-s3">
              {[
                [String(run.summary.inserted), t("Inserted"), false],
                [String(run.summary.skipped_count), t("Skipped"), run.summary.skipped_count > 0],
                [String(targets.filter((x) => x === SKIP).length), t("Columns not imported"), false],
                [run.summary.elapsed_ms < 1000 ? `${run.summary.elapsed_ms} ms` : `${(run.summary.elapsed_ms / 1000).toFixed(1)} s`, t("Duration"), false],
              ].map(([figure, label, danger]) => (
                <div key={String(label)} className="flex flex-col gap-[4px] border border-line p-s3">
                  <span className={`font-heading text-[28px] leading-none font-semibold ${danger ? "text-danger" : ""}`}>{figure}</span>
                  <span className="label">{label}</span>
                </div>
              ))}
            </div>
            {run.summary.skipped.length > 0 && (
              <div className="flex min-h-0 flex-1 flex-col border border-line">
                <div className="flex h-[30px] flex-none items-center gap-s3 border-b border-line-2 px-s3">
                  <span className="label">{t("Skipped rows")}</span>
                  {run.summary.skipped_count > run.summary.skipped.length && <span className="text-[12px] text-text-2">{t("the first {count}", { count: run.summary.skipped.length })}</span>}
                  <span className="flex-1" />
                  <button type="button" className="text-[12.5px] text-accent-text hover:underline" onClick={() => saveReport(run.summary)}>
                    {t("Save error report…")}
                  </button>
                </div>
                <div className="selectable min-h-0 overflow-auto font-mono text-[12px]">
                  {run.summary.skipped.map((error, i) => (
                    <div key={i} className="grid grid-cols-[70px_minmax(0,1fr)] border-b border-line-2 px-s3 py-[4px] last:border-b-0">
                      <span className="text-text-3">{t("line {line}", { line: error.line })}</span>
                      <span className="break-words">{[error.column, error.reason].filter(Boolean).join(" ")}</span>
                    </div>
                  ))}
                </div>
              </div>
            )}
            {report && <p className="selectable font-mono text-[12px] break-all text-text-2">{report}</p>}
          </>
        )}
      </div>
    </Dialog>
  );
}
