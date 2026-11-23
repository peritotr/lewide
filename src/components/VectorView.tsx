// A vector search tab (V01–V02): the query on the left, the nearest points in
// the middle, and the selected point on the right. Searching reads only.
import { Plus, RefreshCw, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, errorMessage, type CollectionInfo, type ScoredPoint, type VectorCondition, type VectorOp, type VectorPoint, type VectorSearchRequest, type VectorSearchResult } from "../lib/api";
import { parseConditionValue, parseVector, summarize } from "../lib/vector";
import { setVectorCollection, type Conn, type Tab } from "../store";
import { t, tn } from "../i18n";
import { Button, Checkbox, Field, Segmented, TextInput } from "./dialog";
import { TabBar } from "./TabBar";
import { LoadError, Loading } from "./TableView";
import { Tag } from "./ui";

const ghost = "flex h-[26px] flex-none items-center gap-[6px] px-s2 text-[12.5px] whitespace-nowrap enabled:hover:bg-hover disabled:cursor-not-allowed disabled:opacity-45";
const outline = "flex h-[24px] flex-none items-center gap-[6px] border border-line px-s2 text-[12px] whitespace-nowrap enabled:hover:bg-hover disabled:cursor-not-allowed disabled:opacity-45";
const select = "h-[26px] min-w-0 border border-line bg-panel px-[6px] font-mono text-[12px]";
const label = "font-heading text-[11px] font-semibold tracking-[.08em] text-text-2 uppercase";
const head = "flex h-row flex-none items-center gap-s3 border-b border-line bg-bg px-s3 font-mono text-[11px] tracking-[.04em] text-text-2 uppercase";
const MOD = /Mac/i.test(navigator.userAgent) ? "⌘" : "Ctrl+";

type Mode = "text" | "vector" | "point";

const OPS: { op: VectorOp; symbol: string }[] = [
  { op: "eq", symbol: "=" },
  { op: "ne", symbol: "≠" },
  { op: "gt", symbol: ">" },
  { op: "gte", symbol: "≥" },
  { op: "lt", symbol: "<" },
  { op: "lte", symbol: "≤" },
  { op: "in", symbol: "in" },
  { op: "is_null", symbol: "is empty" },
  { op: "is_not_null", symbol: "is not empty" },
];
const symbolOf = (op: VectorOp) => OPS.find((o) => o.op === op)!.symbol;
const unary = (op: VectorOp) => op === "is_null" || op === "is_not_null";

export function VectorView({ tab, collection, conn }: { tab: Tab; collection: string | null; conn: Conn | null }) {
  const session = conn?.session ?? null;
  const production = conn?.environment === "production";

  const [collections, setCollections] = useState<string[] | null>(null);
  const [info, setInfo] = useState<CollectionInfo | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [mode, setMode] = useState<Mode>("vector");
  const [using, setUsing] = useState<string | null>(null);
  const [vectorText, setVectorText] = useState("");
  const [pointId, setPointId] = useState("");
  const [filter, setFilter] = useState<VectorCondition[]>([]);
  const [adding, setAdding] = useState(false);
  const [topK, setTopK] = useState("8");
  const [minScore, setMinScore] = useState("");
  const [withPayload, setWithPayload] = useState(true);
  const [searching, setSearching] = useState(false);
  const [result, setResult] = useState<{ data: VectorSearchResult; minScore: number | null; ascending: boolean } | null>(null);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [selected, setSelected] = useState<ScoredPoint | null>(null);
  const [point, setPoint] = useState<VectorPoint | null>(null);
  const run = useRef(0);

  const loadCollections = useCallback(() => {
    if (session === null) return;
    setLoadError(null);
    api.vectorCollections(session).then(
      (names) => {
        setCollections(names);
        if (!collection && names.length > 0) setVectorCollection(tab.id, names[0]);
      },
      (e) => setLoadError(errorMessage(e)),
    );
  }, [session, collection, tab.id]);
  useEffect(loadCollections, [loadCollections]);

  const loadInfo = useCallback(() => {
    if (session === null || !collection) return;
    setInfo(null);
    api.vectorCollectionInfo(session, collection).then(
      (loaded) => {
        setInfo(loaded);
        setUsing(loaded.vectors[0]?.name ?? null);
      },
      (e) => setLoadError(errorMessage(e)),
    );
  }, [session, collection]);
  useEffect(() => {
    setResult(null);
    setSelected(null);
    setSearchError(null);
    loadInfo();
  }, [loadInfo]);

  const spec = info?.vectors.find((v) => v.name === using) ?? info?.vectors[0] ?? null;
  const values = useMemo(() => parseVector(vectorText), [vectorText]);
  const sizeOk = values !== null && values.length > 0 && (spec?.size == null || values.length === spec.size);
  const limit = Number(topK);
  const min = minScore.trim() === "" ? null : Number(minScore);
  const ready = !!collection && Number.isInteger(limit) && limit >= 1 && limit <= 1000 && (min === null || Number.isFinite(min)) && (mode === "vector" ? sizeOk : mode === "point" ? pointId.trim() !== "" : false);

  const search = useCallback(
    async (query: VectorSearchRequest["query"]) => {
      if (session === null || !collection) return;
      const id = ++run.current;
      setSearching(true);
      setSearchError(null);
      try {
        const data = await api.vectorSearch(session, { collection, query, using, limit, min_score: min, filter, with_payload: withPayload, with_vector: false });
        if (id !== run.current) return;
        setResult({ data, minScore: min, ascending: spec ? !spec.higher_is_better : false });
        setSelected(null);
      } catch (e) {
        if (id === run.current) setSearchError(errorMessage(e));
      } finally {
        if (id === run.current) setSearching(false);
      }
    },
    [session, collection, using, limit, min, filter, withPayload, spec],
  );
  const submit = () => {
    if (!ready || searching) return;
    void search(mode === "point" ? { by: "point", id: pointId.trim() } : { by: "vector", values: values! });
  };

  // The selected point's vector is read when its panel opens.
  useEffect(() => {
    setPoint(null);
    if (session === null || !collection || !selected) return;
    let live = true;
    api.vectorPoint(session, collection, selected.id).then(
      (loaded) => live && setPoint(loaded),
      () => {},
    );
    return () => {
      live = false;
    };
  }, [session, collection, selected]);

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

  const hits = result?.data.hits ?? [];
  const firstVector = point?.vectors.find((v) => v.name === using) ?? point?.vectors[0] ?? null;
  const dense = firstVector?.kind === "dense" ? firstVector.values : null;

  return (
    <main
      className="grid min-h-0 min-w-0 grid-cols-[minmax(0,1fr)] bg-panel"
      style={{ gridTemplateRows: `34px ${production ? "4px" : "0px"} 36px minmax(0,1fr)` }}
      onKeyDown={(e) => {
        if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
          e.preventDefault();
          submit();
        }
      }}
    >
      <TabBar />
      <div title={production ? t("Production connection") : undefined} className={production ? "production-band" : ""} />

      <div className="flex items-center gap-[10.2px] border-b border-line px-[10.2px] whitespace-nowrap">
        <select aria-label={t("Collection")} className={select} value={collection ?? ""} onChange={(e) => setVectorCollection(tab.id, e.target.value)}>
          {!collection && <option value="">{collections?.length === 0 ? t("No collections") : "…"}</option>}
          {(collections ?? (collection ? [collection] : [])).map((name) => (
            <option key={name} value={name}>
              {name}
            </option>
          ))}
        </select>
        {info && (
          <span className="min-w-0 truncate font-mono text-[12px] text-text-2">
            {[info.points !== null ? tn(info.points, "{count} point", "{count} points") : null, spec?.size != null ? tn(spec.size, "{count} dimension", "{count} dimensions") : spec?.kind === "sparse" ? t("sparse") : null, spec?.distance].filter(Boolean).join(" · ")}
          </span>
        )}
        <button type="button" className={`${ghost} ml-auto`} onClick={() => (loadCollections(), loadInfo())}>
          <RefreshCw size={13} />
          {t("Refresh")}
        </button>
      </div>

      {loadError ? (
        <LoadError message={loadError} conn={conn} onRetry={() => (loadCollections(), loadInfo())} />
      ) : collections?.length === 0 ? (
        <div className="grid place-items-center">
          <p className="text-text-2">{t("This database has no collections yet.")}</p>
        </div>
      ) : (
        <div className="grid min-h-0" style={{ gridTemplateColumns: `400px minmax(0,1fr) ${selected ? "340px" : ""}` }}>
          <form
            className="flex min-h-0 flex-col gap-[13.6px] overflow-auto border-r border-line bg-bg p-[13.6px]"
            onSubmit={(e) => {
              e.preventDefault();
              submit();
            }}
          >
            <div className="flex items-center gap-s2">
              <span className={label}>{t("Query")}</span>
              <span className="flex-1" />
              {info && info.vectors.length > 1 && (
                // Not drawn in the design: named vectors are chosen beside the mode switch.
                <select aria-label={t("Vector")} className={select} value={using ?? ""} onChange={(e) => setUsing(e.target.value || null)}>
                  {info.vectors.map((v) => (
                    <option key={v.name ?? ""} value={v.name ?? ""}>
                      {v.name ?? t("default")}
                    </option>
                  ))}
                </select>
              )}
            </div>
            <Segmented<Mode>
              label={t("Query")}
              value={mode}
              onChange={setMode}
              options={[
                { value: "text", label: t("Text"), disabled: true, title: t("Text search needs an embedding provider, which LewIDE does not have yet.") },
                { value: "vector", label: t("Vector") },
                { value: "point", label: t("Point ID") },
              ]}
            />
            {mode === "point" ? (
              <Field label={t("Point ID")} helper={t("Searches with this point's vector.")}>
                <TextInput mono autoFocus value={pointId} placeholder="42" onChange={(e) => setPointId(e.target.value)} />
              </Field>
            ) : (
              <div className="flex flex-col gap-[6px]">
                <textarea
                  aria-label={t("Vector")}
                  autoFocus
                  spellCheck={false}
                  placeholder="[0.12, -0.48, 0.33, …]"
                  className="h-[110px] resize-y border border-line bg-panel p-s2 font-mono text-[12px] leading-[18px] focus:border-accent focus:outline-none"
                  value={vectorText}
                  onChange={(e) => setVectorText(e.target.value)}
                />
                {vectorText.trim() !== "" && (
                  <span className={`flex items-center gap-[6px] font-mono text-[11.5px] ${sizeOk ? "text-text-2" : "text-danger"}`}>
                    {sizeOk && <i className="size-[8px] bg-ok" />}
                    {values === null ? t("Not a list of numbers") : spec?.size != null ? t("{count} of {size} values", { count: values.length, size: spec.size }) : tn(values.length, "{count} value", "{count} values")}
                  </span>
                )}
              </div>
            )}

            <div className="flex flex-col gap-s2">
              <span className={label}>{t("Payload filter")}</span>
              <div className="flex flex-wrap items-center gap-[6px]">
                {filter.map((condition, i) => (
                  <span key={i} className="flex items-center gap-[6px]">
                    {i > 0 && <span className="font-mono text-[10.5px] text-text-3">AND</span>}
                    <span className="flex h-[22px] items-center gap-[6px] border border-line bg-panel pr-[2px] pl-[6px] font-mono text-[11.5px]">
                      {condition.field} <span className="text-text-3">{symbolOf(condition.op)}</span> {unary(condition.op) ? "" : JSON.stringify(condition.value)}
                      <button type="button" aria-label={t("Remove condition")} className="grid size-[18px] place-items-center text-text-3 hover:bg-hover hover:text-text" onClick={() => setFilter(filter.filter((_, j) => j !== i))}>
                        <X size={11} />
                      </button>
                    </span>
                  </span>
                ))}
                {!adding && (
                  <button type="button" className={outline} onClick={() => setAdding(true)}>
                    <Plus size={12} />
                    {t("Add condition")}
                  </button>
                )}
              </div>
              {adding && <ConditionForm fields={info?.payload_fields.map((f) => f.name) ?? []} onCancel={() => setAdding(false)} onAdd={(condition) => (setFilter([...filter, condition]), setAdding(false))} />}
            </div>

            <div className="grid grid-cols-2 gap-s3">
              <Field label={t("Results (top K)")}>
                <TextInput mono inputMode="numeric" value={topK} invalid={!(Number.isInteger(limit) && limit >= 1 && limit <= 1000)} onChange={(e) => setTopK(e.target.value)} />
              </Field>
              <Field label={spec && !spec.higher_is_better ? t("Maximum distance") : t("Minimum score")}>
                <TextInput mono inputMode="decimal" value={minScore} placeholder={t("none")} invalid={min !== null && !Number.isFinite(min)} onChange={(e) => setMinScore(e.target.value)} />
              </Field>
            </div>
            <Checkbox checked={withPayload} onChange={setWithPayload}>
              {t("Return payload")}
            </Checkbox>
            <div className="flex items-center gap-s3">
              <Button type="submit" variant="primary" tall disabled={!ready || searching}>
                {t("Search")} {MOD}↵
              </Button>
              <span className="text-[12px] text-text-2">{t("Read-only. Nothing is written.")}</span>
            </div>
          </form>

          <section className="flex min-h-0 min-w-0 flex-col" aria-label={t("Results")}>
            {searching ? (
              <Loading text={t("Searching {collection}…", { collection: collection ?? "" })} />
            ) : searchError ? (
              <LoadError message={searchError} conn={conn} onRetry={submit} />
            ) : !result ? (
              <div className="grid flex-1 place-items-center">
                <p className="text-text-2">{t("Enter a vector or a point ID and search.")}</p>
              </div>
            ) : (
              <>
                <div className="flex h-[36px] flex-none items-center gap-s3 border-b border-line px-s3">
                  <span className="font-medium">{tn(hits.length, "{count} result", "{count} results")}</span>
                  <span className="min-w-0 truncate font-mono text-[11.5px] text-text-2">
                    {[
                      `${result.data.elapsed_ms} ms`,
                      result.ascending ? t("nearest first") : t("highest score first"),
                      result.data.below_min_score > 0 && result.minScore !== null
                        ? result.ascending
                          ? t("{count} beyond {score} left out", { count: result.data.below_min_score, score: result.minScore.toFixed(2) })
                          : t("{count} below {score} left out", { count: result.data.below_min_score, score: result.minScore.toFixed(2) })
                        : null,
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </span>
                </div>
                <div className={head}>
                  <span className="w-[28px] flex-none text-right">#</span>
                  <span className="w-[150px] flex-none">{t("Score")}</span>
                  <span className="w-[150px] flex-none">ID</span>
                  <span className="flex-1">{t("Payload")}</span>
                </div>
                <div className="min-h-0 flex-1 overflow-auto" role="listbox" aria-label={t("Results")}>
                  {hits.length === 0 && <p className="px-s3 py-s4 text-[12.5px] text-text-2">{t("No points match.")}</p>}
                  {hits.map((hit, i) => {
                    const on = selected?.id === hit.id;
                    const summary = summarize(hit.payload);
                    const width = Math.max(0, Math.min(1, hit.score)) * 70;
                    return (
                      <div
                        key={hit.id}
                        role="option"
                        aria-selected={on}
                        tabIndex={0}
                        className={`flex min-h-[44px] cursor-pointer items-center gap-s3 border-b border-line px-s3 py-[5px] ${on ? "bg-accent-tint" : "hover:bg-hover"}`}
                        style={on ? { boxShadow: "inset 2px 0 0 var(--accent)" } : undefined}
                        onClick={() => setSelected(on ? null : hit)}
                        onKeyDown={(e) => e.key === "Enter" && setSelected(on ? null : hit)}
                      >
                        <span className="w-[28px] flex-none text-right font-mono text-[12px] text-text-3">{hit.rank}</span>
                        <span className="flex w-[150px] flex-none items-center gap-s2">
                          <span className="font-mono text-[12.5px] font-medium">{hit.score.toFixed(4)}</span>
                          {!result.ascending && (
                            <span className="h-[6px] w-[70px] flex-none border border-line">
                              <i className={`block h-full ${i === 0 ? "bg-accent-fill" : "bg-text-3"}`} style={{ width }} />
                            </span>
                          )}
                        </span>
                        <span className="selectable w-[150px] flex-none truncate font-mono text-[12px]" title={hit.id}>
                          {hit.id}
                        </span>
                        <span className="flex min-w-0 flex-1 flex-col gap-[3px]">
                          {hit.payload === null ? (
                            <span className="text-[12px] text-text-3">{t("Payload not returned")}</span>
                          ) : summary.title === null && summary.tags.length === 0 ? (
                            <span className="truncate font-mono text-[12px] text-text-2">{JSON.stringify(hit.payload)}</span>
                          ) : (
                            <>
                              <span className="flex min-w-0 items-center gap-s2">
                                <span className="truncate text-[12.5px] font-medium">{summary.title}</span>
                                {summary.tags.map((tag) => (
                                  <Tag key={tag}>{tag}</Tag>
                                ))}
                              </span>
                              {summary.text && <span className="truncate text-[12px] text-text-2">{summary.text}</span>}
                            </>
                          )}
                        </span>
                      </div>
                    );
                  })}
                </div>
              </>
            )}
          </section>

          {selected && (
            <aside className="flex min-h-0 flex-col gap-s3 overflow-auto border-l border-line bg-bg p-s3" aria-label={t("Point")}>
              <div className="flex items-center gap-s2">
                <span className={label}>{t("Point")}</span>
                <span className="selectable min-w-0 flex-1 truncate font-mono text-[12.5px] font-medium" title={selected.id}>
                  {selected.id}
                </span>
                <span className="font-mono text-[12px] text-text-2">{selected.score.toFixed(4)}</span>
                <button type="button" aria-label={t("Close")} className="grid size-[22px] place-items-center text-text-3 hover:bg-hover hover:text-text" onClick={() => setSelected(null)}>
                  <X size={13} />
                </button>
              </div>
              <span className={label}>{t("Payload")}</span>
              <pre className="selectable max-h-[45%] min-h-[60px] flex-none overflow-auto border border-line bg-panel p-s2 font-mono text-[12px] leading-[18px] whitespace-pre-wrap">
                {point ? JSON.stringify(point.payload, null, 2) : selected.payload ? JSON.stringify(selected.payload, null, 2) : "…"}
              </pre>
              <span className={label}>{t("Vector")}</span>
              <pre className="selectable flex-none overflow-auto border border-line bg-panel p-s2 font-mono text-[12px] leading-[18px] whitespace-pre-wrap">
                {!point
                  ? "…"
                  : dense
                    ? `[${dense.slice(0, 8).map((v) => v.toFixed(4)).join(", ")}${dense.length > 8 ? ", …" : ""}]\n${tn(dense.length, "{count} value", "{count} values")}`
                    : firstVector
                      ? t("{kind} vector", { kind: firstVector.kind })
                      : t("No vector")}
              </pre>
              <div className="flex flex-wrap gap-s2">
                <button
                  type="button"
                  className={outline}
                  onClick={() => {
                    setMode("point");
                    setPointId(selected.id);
                    void search({ by: "point", id: selected.id });
                  }}
                >
                  {t("Find similar")}
                </button>
                <button type="button" className={outline} disabled={!dense} onClick={() => dense && void navigator.clipboard?.writeText(JSON.stringify(dense))}>
                  {t("Copy vector")}
                </button>
                <button type="button" className={outline} disabled={!point && !selected.payload} onClick={() => void navigator.clipboard?.writeText(JSON.stringify(point?.payload ?? selected.payload, null, 2))}>
                  {t("Copy payload")}
                </button>
              </div>
            </aside>
          )}
        </div>
      )}
    </main>
  );
}

function ConditionForm({ fields, onAdd, onCancel }: { fields: string[]; onAdd(condition: VectorCondition): void; onCancel(): void }) {
  const [field, setField] = useState(fields[0] ?? "");
  const [op, setOp] = useState<VectorOp>("eq");
  const [value, setValue] = useState("");
  const ok = field.trim() !== "" && (unary(op) || value.trim() !== "");
  const add = () => ok && onAdd(unary(op) ? { field: field.trim(), op } : { field: field.trim(), op, value: parseConditionValue(value, op === "in") });
  return (
    <div
      className="flex flex-col gap-s2 border border-line bg-panel p-s2"
      onKeyDown={(e) => {
        if (e.key === "Enter" && !e.metaKey && !e.ctrlKey) {
          e.preventDefault();
          e.stopPropagation();
          add();
        }
        if (e.key === "Escape") onCancel();
      }}
    >
      <div className="flex gap-s2">
        {/* The first known field is filled in and selected, so typing another name replaces it. */}
        <TextInput mono autoFocus className="flex-1" list="payload-fields" placeholder={t("field")} aria-label={t("Field")} value={field} onFocus={(e) => e.currentTarget.select()} onChange={(e) => setField(e.target.value)} />
        <datalist id="payload-fields">
          {fields.map((name) => (
            <option key={name} value={name} />
          ))}
        </datalist>
        <select aria-label={t("Operator")} className={select} value={op} onChange={(e) => setOp(e.target.value as VectorOp)}>
          {OPS.map((o) => (
            <option key={o.op} value={o.op}>
              {o.symbol}
            </option>
          ))}
        </select>
      </div>
      {!unary(op) && <TextInput mono placeholder={op === "in" ? "en, de" : t("value")} aria-label={t("Value")} value={value} onChange={(e) => setValue(e.target.value)} />}
      <div className="flex justify-end gap-s2">
        <button type="button" className={outline} onClick={onCancel}>
          {t("Cancel")}
        </button>
        <button type="button" className={outline} disabled={!ok} onClick={add}>
          {t("Add condition")}
        </button>
      </div>
    </div>
  );
}
