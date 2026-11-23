import { CheckSquare, Download, Filter, Info, Lock, Square, TriangleAlert } from "lucide-react";
import { useEffect, useState } from "react";
import type { DbError } from "../lib/api";
import { readPlan } from "../lib/plan";
import { canLoadMore, cancelRun, explain, loadMore, updateTab, useApp, type Conn, type Message, type ResultView, type Tab, openDialog } from "../store";
import { DataGrid } from "./DataGrid";
import { Button, Segmented } from "./dialog";
import { isMac } from "./TitleBar";
import { IconButton, NOT_BUILT, RegMarks, Splitter } from "./ui";
import { msg, t, tn, tx } from "../i18n";

const VIEWS: { id: ResultView; label: string }[] = [
  { id: "results", label: msg("Results") },
  { id: "messages", label: msg("Messages") },
  { id: "plan", label: msg("Query plan") },
];
const MOD = isMac ? "⌘" : "Ctrl+";
const copy = (text: string) => void navigator.clipboard?.writeText(text);
const clock = (time: number) => new Date(time).toLocaleTimeString("en-GB");
const seconds = (ms: number) => `${(ms / 1000).toFixed(1)} s`;

/** Line and column of a 1-based character offset in `text`. */
export function locate(text: string, offset: number): { line: number; col: number } {
  const before = text.slice(0, Math.max(0, offset - 1));
  const line = before.split("\n").length;
  return { line, col: before.length - before.lastIndexOf("\n") };
}

/** Where an error is in the editor text: the statement's start plus the server's position. */
export function errorPlace(tab: Tab): { line: number; col: number | null; offset: number | null } | null {
  if (tab.run.state !== "error") return null;
  const { error, from, statement } = tab.run;
  // After an edit the offsets no longer point at the same text: fall back to the statement alone.
  const base = from !== null && !tab.edited ? from : null;
  if (error.position !== null) {
    const offset = base !== null ? base + error.position : null;
    const at = offset !== null ? locate(tab.sql, offset) : locate(statement, error.position);
    return { ...at, offset };
  }
  if (error.line !== null) {
    const firstLine = base !== null ? locate(tab.sql, base + 1).line : 1;
    return { line: firstLine + error.line - 1, col: null, offset: null };
  }
  return null;
}

/** Re-renders its user every 100 ms while `active`, for elapsed times. */
function useTick(active: boolean) {
  const [, tick] = useState(0);
  useEffect(() => {
    if (!active) return;
    const timer = setInterval(() => tick((n) => n + 1), 100);
    return () => clearInterval(timer);
  }, [active]);
}

export const Spinner = () => <i className="status-ring flex-none" aria-hidden />;

interface Props {
  tab: Tab;
  conn: Conn | null;
  onGoTo(place: { line: number; offset: number | null }): void;
  onRunAgain(): void;
}

export function Results({ tab, conn, onGoTo, onRunAgain }: Props) {
  const running = tab.run.state === "running";
  const withRows = tab.sets.map((set, index) => ({ set, index })).filter(({ set }) => set.result.columns.length > 0);
  const current = tab.sets[tab.activeSet]?.result.columns.length ? tab.sets[tab.activeSet] : withRows[0]?.set;
  const showSets = tab.run.state === "done" && tab.view === "results";
  const lost = conn !== null && conn.session === null;
  const exportable = showSets && current ? current.result : null;

  return (
    <section className="relative z-[2] grid min-h-0 grid-rows-[34px_auto_minmax(0,1fr)] border-t border-line bg-panel">
      <RegMarks corners={["tl"]} />
      <Splitter
        orientation="horizontal"
        className="-top-[3px]"
        onDrag={(dy) => useApp.setState((s) => ({ resultsHeight: Math.min(window.innerHeight - 260, Math.max(120, s.resultsHeight - dy)) }))}
      />
      {/* In a narrow pane (the Ask AI panel open) the note gives way first; the row never spills. */}
      <div className="flex min-w-0 items-stretch gap-s4 overflow-hidden border-b border-line px-s3">
        {showSets &&
          withRows.map(({ set, index }, n) => {
            const active = set === current;
            return (
              <button
                key={index}
                type="button"
                role="tab"
                aria-selected={active}
                className={`flex flex-none items-center gap-[6px] whitespace-nowrap ${active ? "font-medium" : "text-text-2"}`}
                style={active ? { boxShadow: "inset 0 -2px 0 var(--accent)" } : undefined}
                onClick={() => updateTab(tab.id, { activeSet: index })}
              >
                {t("Result {number}", { number: n + 1 })}
                <span className="font-mono text-[11px] font-normal text-text-3">
                  {withRows.length === 1 && set.result.rows.length === 0 ? "· " : ""}
                  {tn(set.result.rows.length, "{count} row", "{count} rows")}
                </span>
              </button>
            );
          })}
        {showSets && current && (
          <div className="ml-s3 flex min-w-0 items-center gap-[6px] self-center text-[12px] whitespace-nowrap text-text-2">
            <Lock size={12} className="flex-none" />
            <span className="min-w-0 truncate">{lost && tab.finishedAt ? t("Results from {time} · connection closed", { time: clock(tab.finishedAt) }) : t("Read-only · query result")}</span>
          </div>
        )}

        <div role="tablist" className="ml-auto flex flex-none self-center border border-line text-[12px] whitespace-nowrap">
          {VIEWS.map((view, i) => (
            <button
              key={view.id}
              type="button"
              role="tab"
              aria-selected={tab.view === view.id}
              className={`px-[10px] py-[4px] ${i > 0 ? "border-l border-line" : ""} ${
                tab.view === view.id ? "bg-accent-fill font-medium text-on-accent" : "hover:bg-hover"
              }`}
              onClick={() => updateTab(tab.id, { view: view.id })}
            >
              {t(view.label)}
            </button>
          ))}
        </div>
        <div className="flex flex-none gap-[2px] self-center">
          <IconButton label={`${t("Filter results")} · ${t(NOT_BUILT)}`} disabled>
            <Filter size={14} />
          </IconButton>
          <IconButton label={t("Export")} disabled={!exportable || !conn} onClick={() => conn && exportable && openDialog({ kind: "export", connId: conn.id, source: { kind: "result", title: tab.title, result: exportable } })}>
            <Download size={14} />
          </IconButton>
        </div>
      </div>

      {running ? <div className="run-bar" /> : <div />}

      {tab.view === "messages" ? (
        <MessagesView tab={tab} onGoTo={onGoTo} />
      ) : tab.view === "plan" ? (
        <PlanView tab={tab} />
      ) : tab.run.state === "running" ? (
        <RunningBody tab={tab} conn={conn} />
      ) : tab.run.state === "cancelled" ? (
        <div className="p-s6">
          <div className="flex max-w-[720px] items-start gap-s3 border border-line p-s3">
            <Square size={14} className="mt-[2px] fill-current text-text-2" />
            <div className="flex flex-col items-start gap-s2">
              <span className="font-medium">{t("Query cancelled")}</span>
              <span className="text-text-2">
                {t("Stopped after {time}. No rows were returned.", { time: seconds(tab.run.elapsedMs) })}
                {tab.run.notRun > 0 && ` ${tn(tab.run.notRun, "The next statement was not run.", "{count} statements were not run.")}`}
              </span>
              <Button onClick={onRunAgain}>{t("Run again")}</Button>
            </div>
          </div>
        </div>
      ) : tab.run.state === "error" ? (
        <ErrorPanel tab={tab} onGoTo={onGoTo} />
      ) : current ? (
        <div className="grid min-h-0 grid-rows-[auto_minmax(0,1fr)]">
          {current.result.truncated ? (
            <div className="flex items-center gap-s3 border-b border-line bg-accent-tint px-s3 py-[5px] text-[12.5px]">
              <Info size={14} className="flex-none text-accent-text" />
              <span>
                {t("First {count} rows shown. The query has more rows than the limit.", { count: current.result.rows.length.toLocaleString() })}
              </span>
              {canLoadMore(tab, conn) && (
                <button type="button" className="h-[22px] border border-line bg-panel px-s2 hover:bg-hover" title={t("Runs the statement again with a larger limit")} onClick={() => loadMore(tab.id, 1000)}>
                  {t("Load next 1,000")}
                </button>
              )}
              <span className="text-[12px] text-text-2">{t("Raise the limit in the toolbar to fetch more on the next run.")}</span>
            </div>
          ) : (
            <div />
          )}
          <div className="relative grid min-h-0">
            <DataGrid key={`${tab.id}:${tab.sets.indexOf(current)}:${tab.finishedAt}`} result={current.result} note={t("Read-only · query result")} />
            {current.result.rows.length === 0 && (
              // The column header stays: the columns are known even with nothing to list.
              <div className="pointer-events-none absolute inset-0 top-[var(--row-h)] grid place-items-center">
                <div className="flex flex-col items-center gap-s1">
                  <span className="font-heading text-[20px] font-semibold">{t("No rows returned")}</span>
                  <span className="text-text-2">{t("The query ran successfully and matched nothing.")}</span>
                  <span className="font-mono text-[12px] text-text-3">{t("0 rows · {ms} ms", { ms: current.result.elapsed_ms })}</span>
                </div>
              </div>
            )}
          </div>
        </div>
      ) : tab.sets.length > 0 ? (
        // Statements that return no result set: what they did.
        <div className="flex flex-col gap-s2 overflow-auto p-s6">
          {tab.sets.map((set, i) => (
            <div key={i} className="flex max-w-[820px] items-center gap-s3 border border-ok bg-ok-tint px-s3 py-s2">
              <CheckSquare size={14} className="flex-none text-ok" />
              <span className="font-medium whitespace-nowrap">
                {tn(set.result.rows_affected ?? 0, "{count} row affected", "{count} rows affected")}
              </span>
              <span className="selectable min-w-0 flex-1 truncate font-mono text-[12px] text-text-2">{set.statement.replace(/\s+/g, " ")}</span>
              <span className="font-mono text-[12px] whitespace-nowrap text-text-2">{t("{ms} ms · no result set", { ms: set.result.elapsed_ms })}</span>
            </div>
          ))}
        </div>
      ) : (
        <div className="grid place-items-center">
          <div className="flex flex-col items-center gap-s1">
            <span className="font-heading text-[20px] font-semibold">{t("No results yet")}</span>
            <span className="text-text-2">
              {tx("Run a statement with {keys} to see its result here.", { keys: <span key="keys" className="font-mono text-[12px]">{MOD}↵</span> })}
            </span>
          </div>
        </div>
      )}
    </section>
  );
}

function RunningBody({ tab, conn }: { tab: Tab; conn: Conn | null }) {
  useTick(true);
  if (tab.run.state !== "running") return null;
  const elapsed = Date.now() - tab.run.startedAt;
  return (
    <div className="flex flex-col gap-s4 p-s6">
      <div className="flex items-center gap-s3">
        <Spinner />
        <span className="font-medium">
          {t("Running statement {index} of {total}", { index: tab.run.index + 1, total: tab.run.total })}
        </span>
        <span className="font-heading text-[22px] leading-none font-semibold tabular-nums">{seconds(elapsed)}</span>
      </div>
      <span className="selectable truncate font-mono text-[12px] text-text-2">{tab.run.preview}</span>
      {elapsed > 2000 && (
        <div className="flex max-w-[720px] flex-col items-start gap-s2 border border-line p-s3">
          <span className="font-medium">{t("Still running on {connection}", { connection: conn?.name ?? "" })}</span>
          <span className="text-text-2">
            {t("Started {time}. You can keep working in other tabs; this tab shows a mark when it finishes.", { time: clock(tab.run.startedAt) })}
          </span>
          <button
            type="button"
            disabled={tab.run.stopping}
            className="flex h-[28px] items-center gap-[6px] border border-danger px-s3 font-medium text-danger enabled:hover:bg-danger-tint"
            onClick={() => cancelRun(tab.id)}
          >
            <Square size={12} className="fill-current" />
            {tab.run.stopping ? t("Stopping…") : t("Stop query")}
            {!tab.run.stopping && <span className="font-mono text-[11px] font-normal">{MOD}.</span>}
          </button>
        </div>
      )}
    </div>
  );
}

export function errorText(error: DbError): string {
  return [error.code && `Error ${error.code}`, error.message, error.detail && `Detail: ${error.detail}`, error.hint && `Hint: ${error.hint}`].filter(Boolean).join("\n");
}

function ErrorPanel({ tab, onGoTo }: { tab: Tab; onGoTo: Props["onGoTo"] }) {
  if (tab.run.state !== "error") return null;
  const { error, index, total, elapsedMs } = tab.run;
  const place = errorPlace(tab);
  const none = <span className="text-text-3">{t("None from the server")}</span>;
  const notRun = total - index - 1;
  return (
    <div role="alert" className="flex flex-col gap-s3 overflow-auto px-s6 py-s4">
      <div className="flex items-center gap-s3">
        <TriangleAlert size={16} className="flex-none text-danger" />
        <span className="border border-danger px-[4px] py-[2px] font-heading text-[11px] leading-none font-semibold tracking-[.08em] text-danger">{t("ERROR")}</span>
        {error.code && <span className="selectable font-mono text-[12.5px] font-medium">{error.code}</span>}
        <button type="button" className="ml-auto text-[12.5px] text-accent-text hover:underline" onClick={() => copy(errorText(error))}>
          {t("Copy error")}
        </button>
      </div>
      <p className="selectable font-mono text-[14px] break-words whitespace-pre-wrap">{error.message}</p>
      <div className="grid max-w-[900px] grid-cols-[72px_minmax(0,1fr)] border border-line">
        <span className="label border-b border-line-2 px-s3 py-s2">{t("Position")}</span>
        <span className="flex gap-s3 border-b border-line-2 px-s3 py-s2 text-[12.5px]">
          {place ? (
            <>
              <span>
                {place.col !== null ? t("Line {line}, column {column}", { line: place.line, column: place.col }) : t("Line {line}", { line: place.line })}
              </span>
              <button type="button" className="text-accent-text hover:underline" onClick={() => onGoTo(place)}>
                {t("Go to line")}
              </button>
            </>
          ) : (
            none
          )}
        </span>
        <span className="label border-b border-line-2 px-s3 py-s2">{t("Hint")}</span>
        <span className="selectable border-b border-line-2 px-s3 py-s2 text-[12.5px]">{error.hint ?? none}</span>
        <span className="label px-s3 py-s2">{t("Detail")}</span>
        <span className="selectable px-s3 py-s2 text-[12.5px]">{error.detail ?? none}</span>
      </div>
      <p className="text-[12.5px] text-text-2">
        {t("Statement {index} of {total} failed after {ms} ms.", { index: index + 1, total, ms: elapsedMs })}
        {notRun > 0 && ` ${tn(notRun, "The next statement was not run.", "{count} statements were not run.")}`}
      </p>
    </div>
  );
}

function MessagesView({ tab, onGoTo }: { tab: Tab; onGoTo: Props["onGoTo"] }) {
  if (tab.messages.length === 0) {
    return <p className="p-s6 text-text-3">{t("No messages yet. Each statement you run is listed here with what it did.")}</p>;
  }
  const columns = "grid grid-cols-[100px_36px_minmax(0,1fr)_190px_80px] items-center gap-s2 px-s3";
  const failed = tab.messages.filter((m) => m.error).length;
  const total = tab.messages.reduce((sum, m) => sum + m.durationMs, 0);
  const time = (m: Message) => `${clock(m.time)}.${String(m.time % 1000).padStart(3, "0")}`;
  return (
    <div className="flex min-h-0 flex-col overflow-auto font-mono text-[12px]">
      <div className={`${columns} label sticky top-0 h-row flex-none border-b border-line bg-bg`}>
        <span>{t("Time")}</span>
        <span>#</span>
        <span>{t("Statement")}</span>
        <span>{t("Result")}</span>
        <span className="text-right">{t("Duration")}</span>
      </div>
      {tab.messages.map((message, i) => (
        <div key={i} className={`flex-none border-b border-line-2 ${message.error ? "bg-danger-tint" : ""}`}>
          <div className={`${columns} h-row`}>
            <span className="text-text-3">{time(message)}</span>
            <span>{message.index + 1}</span>
            <span className="selectable truncate text-text-2">{message.preview}</span>
            <span className={`flex items-center gap-[6px] ${message.error ? "text-danger" : ""}`}>
              {message.error && <TriangleAlert size={12} />}
              {message.result}
            </span>
            <span className="text-right">{message.durationMs} ms</span>
          </div>
          {message.error && (
            <div className="selectable flex flex-col gap-[2px] pr-s3 pb-s2 pl-[136px] font-body text-[12.5px] whitespace-pre-wrap">
              <span className="font-mono text-[12px]">{message.error.message}</span>
              {message.error.detail && <span className="text-text-2">{t("Detail: {text}", { text: message.error.detail })}</span>}
              {message.error.hint && <span className="text-text-2">{t("Hint: {text}", { text: message.error.hint })}</span>}
              {i === tab.messages.length - 1 && errorPlace(tab) && (
                <button type="button" className="self-start text-accent-text hover:underline" onClick={() => onGoTo(errorPlace(tab)!)}>
                  {t("Go to line {line}", { line: errorPlace(tab)!.line })}
                </button>
              )}
            </div>
          )}
        </div>
      ))}
      <div className="flex-none px-s3 py-s2 font-body text-[12.5px] text-text-2">
        {[
          tn(tab.messages.length, "{count} statement", "{count} statements"),
          t("{count} succeeded", { count: tab.messages.length - failed }),
          failed > 0 ? t("{count} failed", { count: failed }) : null,
          t("{ms} ms total", { ms: total }),
        ]
          .filter(Boolean)
          .join(" · ")}
      </div>
    </div>
  );
}

function PlanView({ tab }: { tab: Tab }) {
  const [raw, setRaw] = useState(false);
  const state = tab.plan;
  if (!state) return <p className="p-s6 text-text-3">{t("Use Explain to see how the database would run the statement under the caret.")}</p>;
  if (state.error) {
    return (
      <div role="alert" className="flex items-start gap-s3 p-s6">
        <TriangleAlert size={16} className="flex-none text-danger" />
        <p className="selectable font-mono text-[13px] whitespace-pre-wrap">{state.error}</p>
      </div>
    );
  }
  const plan = state.plan;
  const showRaw = raw || !plan;
  const reading = plan && readPlan(plan);
  const columns = "grid grid-cols-[minmax(0,1fr)_120px_110px_220px] items-center gap-s3 px-s3";
  return (
    <div className="flex min-h-0 flex-col">
      <div className="flex flex-none items-center gap-s4 border-b border-line-2 px-s3 py-s2 text-[12.5px]">
        {plan && (
          <Segmented
            label={t("Plan view")}
            value={showRaw ? "raw" : "tree"}
            onChange={(v) => setRaw(v === "raw")}
            options={[
              { value: "tree", label: t("Tree") },
              { value: "raw", label: t("Raw text") },
            ]}
          />
        )}
        <span className="text-text-2">{t("Estimated plan · EXPLAIN without ANALYZE")}</span>
        {plan && <span className="font-mono text-[12px]">{t("total cost {cost}", { cost: plan.totalCost })}</span>}
        <button type="button" className="ml-auto text-accent-text hover:underline" onClick={() => copy(state.raw)}>
          {t("Copy")}
        </button>
        <Button onClick={() => explain(tab.id, state.statement)}>{t("Refresh")}</Button>
      </div>
      {showRaw || !plan ? (
        <pre className="selectable min-h-0 flex-1 overflow-auto p-s3 font-mono text-[12.5px] leading-[20px]">{state.raw}</pre>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto font-mono text-[12px]">
          <div className={`${columns} label sticky top-0 h-row border-b border-line bg-bg`}>
            <span>{t("Node")}</span>
            <span className="text-right">{t("Est. cost")}</span>
            <span className="text-right">{t("Est. rows")}</span>
            <span>{t("Share of cost")}</span>
          </div>
          {plan.nodes.map((node, i) => (
            <div
              key={i}
              className={`${columns} h-row border-b border-line-2 ${node.mostExpensive ? "bg-accent-tint" : ""}`}
              style={node.mostExpensive ? { boxShadow: "inset 2px 0 0 var(--accent)" } : undefined}
            >
              <span className="flex min-w-0 items-center gap-s2" style={{ paddingLeft: Math.max(0, node.depth - 1) * 18 }}>
                {node.depth > 0 && <span className="text-text-3">{node.last ? "└" : "├"}</span>}
                <span className="font-medium whitespace-nowrap">{node.type}</span>
                <span className="selectable truncate text-text-2">{node.detail}</span>
                {node.mostExpensive && (
                  <span className="flex-none border border-accent-text px-[4px] py-[1px] font-heading text-[10px] leading-none font-semibold tracking-[.08em] text-accent-text">{t("MOST EXPENSIVE")}</span>
                )}
              </span>
              <span className={`text-right ${node.mostExpensive ? "font-medium" : ""}`}>{node.totalCost}</span>
              <span className="text-right">{node.rows}</span>
              <span className="flex items-center gap-s2">
                <span className="h-[6px] w-[120px] flex-none bg-line-2">
                  <i className={`block h-full ${node.mostExpensive ? "bg-accent-fill" : "bg-text-3"}`} style={{ width: `${Math.round(node.share * 100)}%` }} />
                </span>
                <span className={node.mostExpensive ? "font-medium" : ""}>{Math.round(node.share * 100)}%</span>
              </span>
            </div>
          ))}
          {reading && <p className="px-s3 py-s3 font-body text-[12.5px] text-text-2">{reading}</p>}
        </div>
      )}
    </div>
  );
}
