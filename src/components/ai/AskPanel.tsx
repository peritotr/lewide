// The Ask AI panel beside the editor (AI04–AI08, AI10–AI12), with a review's
// findings marked in the editor (AI05) and quick replies to a question (AI14).
import { ArrowUp, Sparkles, X } from "lucide-react";
import { useEffect, useRef, useState, type RefObject } from "react";
import { activeProvider, ask, chooseProvider, clearThread, isAsking, loadProviders, sendsSchema, setPanelWidth, stopAsking, togglePanel, updateTurn, usable, useAi, type Scope, type Turn } from "../../ai-store";
import { findings, verdict, type Task } from "../../lib/ai";
import { typeOf } from "../../lib/types";
import { msg, t } from "../../i18n";
import { openDialog, type Conn, type Tab } from "../../store";
import { Button } from "../dialog";
import type { AiMark, EditorHandle } from "../Editor";
import { locate } from "../Results";
import { IconButton, Splitter } from "../ui";
import { Answer } from "./Answer";

const QUICK: { label: string; task: Task }[] = [
  { label: msg("Explain this query"), task: { kind: "explain" } },
  { label: msg("Check for mistakes"), task: { kind: "review" } },
  { label: msg("Optimize"), task: { kind: "optimize" } },
  { label: msg("Add comments"), task: { kind: "ask", question: "Add short comments to this statement that explain each part. Return it as the suggested query, changing nothing but the comments." } },
];

export function ProviderSelect() {
  const providers = useAi((s) => s.providers).filter(usable);
  const active = useAi(activeProvider);
  return (
    <select
      aria-label={t("Model")}
      className="h-[22px] max-w-[190px] min-w-0 border border-line bg-panel px-[6px] font-mono text-[11.5px] outline-none focus:border-accent disabled:text-text-3"
      value={active?.id ?? ""}
      disabled={providers.length === 0}
      onChange={(e) => chooseProvider(e.target.value)}
    >
      {providers.length === 0 && <option value="">{t("No provider")}</option>}
      {providers.map((p) => (
        <option key={p.id} value={p.id}>
          {p.model || p.name}
        </option>
      ))}
    </select>
  );
}

/** The editor marks for a finished review (AI05), while its statement is unchanged. */
export function reviewMarks(turn: Turn | undefined): AiMark[] | null {
  if (!turn || turn.task.kind !== "review" || turn.state !== "done" || !turn.scope) return null;
  const review = verdict(turn.text);
  if (!review || review === "ok") return null;
  const scope = turn.scope;
  const list = findings(turn.text.replace(/^\s*verdict:[^\n]*\n?/i, "")).findings;
  const marks = list.filter((f) => f.line !== null).map((f) => ({ line: scope.fromLine + f.line! - 1, severity: f.severity, token: f.token, message: f.text }));
  return marks.length ? marks : null;
}

export function AskPanel({ tab, conn, editor }: { tab: Tab; conn: Conn | null; editor: RefObject<EditorHandle | null> }) {
  const width = useAi((s) => s.panelWidth);
  const turns = useAi((s) => s.threads[tab.id]) ?? [];
  const loaded = useAi((s) => s.loaded);
  const provider = useAi(activeProvider);
  const privacy = useAi((s) => s.privacy);
  const busy = useAi((s) => isAsking(tab.id, s));
  const [question, setQuestion] = useState("");
  const [scope, setScope] = useState<Scope | null>(null);
  const body = useRef<HTMLDivElement>(null);
  const production = conn?.environment === "production";
  const schema = !!conn && conn.session !== null && sendsSchema(production, privacy);

  useEffect(() => void loadProviders(), []);

  // What a request would be about: the selection, or the statement under the caret.
  const readScope = (): Scope | null => {
    const handle = editor.current;
    if (!handle) return null;
    const text = handle.text();
    const selected = handle.selectedStatements();
    const statement = selected?.length ? { from: selected[0].from, to: selected[selected.length - 1].to } : handle.statementAtCaret();
    if (!statement) return null;
    const sql = text.slice(statement.from, statement.to);
    if (!sql.trim()) return null;
    return { kind: selected?.length ? "selection" : "statement", from: statement.from, to: statement.to, fromLine: locate(text, statement.from).line, toLine: locate(text, Math.max(statement.from, statement.to - 1)).line, sql };
  };
  useEffect(() => {
    setScope(readScope());
    // The caret and the text decide the scope.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab.caret.line, tab.caret.col, tab.sql]);

  useEffect(() => {
    body.current?.scrollTo({ top: body.current.scrollHeight });
  }, [turns.length, turns[turns.length - 1]?.text.length]); // eslint-disable-line react-hooks/exhaustive-deps

  /** `followUp` sends the earlier turns too, for an answer that refers to them. */
  const send = (request: string, task: Task, followUp = false) => {
    const current = readScope();
    if (busy || !provider) return;
    if (!current && task.kind !== "ask") return;
    ask(tab.id, {
      request,
      task,
      scope: current,
      session: conn?.session ?? null,
      engine: conn ? typeOf(conn.engine, conn.driver.id).name : undefined,
      namespace: conn?.namespace ?? undefined,
      production,
      conversation: followUp,
    });
  };

  // The suggestion can be written over the scoped text only while that text is still what was sent.
  const actionsFor = (turn: Turn) => {
    const handle = editor.current;
    const scoped = turn.scope;
    const intact = !!handle && !!scoped && handle.text().slice(scoped.from, scoped.to) === scoped.sql;
    return {
      apply: intact
        ? (sql: string) => {
            const text = handle!.text();
            handle!.replaceText(text.slice(0, scoped!.from) + sql + text.slice(scoped!.to));
            updateTurn(tab.id, turn.id, { applied: { previous: text, lines: `${scoped!.fromLine}–${scoped!.toLine}` } });
          }
        : null,
      insertBelow: intact
        ? (sql: string) => {
            const text = handle!.text();
            handle!.replaceText(`${text.slice(0, scoped!.to)}\n\n${sql}${text.slice(scoped!.to)}`);
          }
        : null,
      undo: () => {
        if (turn.applied && handle) handle.replaceText(turn.applied.previous);
        updateTurn(tab.id, turn.id, { applied: null });
      },
      retry: () => send(turn.request, turn.task),
      goToLine: intact ? (line: number) => handle!.flashLine(line) : undefined,
      suggestFix: () => send(t("Suggest a fix"), { kind: "ask", question: "Give the corrected statement for the problems you found, as the suggested query." }, true),
      reply: busy ? undefined : (text: string) => send(text, { kind: "ask", question: text }, true),
    };
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      if (busy) stopAsking(tab.id);
      else togglePanel(tab.id, false);
    }
  };

  return (
    <aside className="relative flex min-h-0 flex-col border-l border-line bg-bg" style={{ width }} onKeyDown={onKeyDown} aria-label={t("Ask AI")}>
      <Splitter orientation="vertical" className="-left-[3px]" onDrag={(dx) => setPanelWidth(useAi.getState().panelWidth - dx)} />
      <header className="flex h-[34px] flex-none items-center gap-s2 border-b border-line px-s3">
        <Sparkles size={14} strokeWidth={1.5} className="flex-none text-accent-text" />
        <span className="font-medium">{t("Ask AI")}</span>
        <span className="flex-1" />
        <ProviderSelect />
        <IconButton label={`${t("Close")} (Esc)`} size={22} onClick={() => togglePanel(tab.id, false)}>
          <X size={13} />
        </IconButton>
      </header>

      {loaded && !provider ? (
        // AI12
        <div className="flex flex-1 flex-col items-start gap-s3 p-s4">
          <Sparkles size={22} strokeWidth={1.5} className="text-accent-text" />
          <h3 className="font-heading text-[20px] leading-[1.1] font-semibold">{t("Set up an AI provider")}</h3>
          <p className="text-[12.5px] leading-[1.5] text-text-2">{t("Add a key from Anthropic, OpenAI, Google or your own endpoint to explain, check and improve SQL here.")}</p>
          <Button variant="primary" onClick={() => openDialog({ kind: "settings", section: "ai" })}>
            {t("Open AI settings")}
          </Button>
          <p className="text-[12px] text-text-2">{t("Keys are stored in the system keychain.")}</p>
        </div>
      ) : (
        <>
          <div className="flex flex-none flex-col gap-[6px] border-b border-line-2 p-s3">
            <div className="flex items-center gap-s2">
              <span className="label">{t("Working on")}</span>
              <span className="border border-line px-[6px] py-[2px] font-mono text-[11.5px]">
                {scope ? (scope.kind === "selection" ? t("Selection · lines {from}–{to}", { from: scope.fromLine, to: scope.toLine }) : t("Statement at cursor · lines {from}–{to}", { from: scope.fromLine, to: scope.toLine })) : t("No statement")}
              </span>
            </div>
            <p className="text-[12px] leading-[1.45] text-text-2">
              {schema ? t("Sends this statement and the schema of the tables it names. No row data.") : t("Sends this statement only. No schema, no row data.")}{" "}
              <button type="button" className="text-accent-text hover:underline" onClick={() => openDialog({ kind: "ai-sent", connId: schema ? conn!.id : null, sql: scope?.sql ?? null, lines: scope ? `${scope.fromLine}–${scope.toLine}` : null })}>
                {t("View")}
              </button>
            </p>
          </div>

          <div ref={body} className="flex min-h-0 flex-1 flex-col gap-s4 overflow-auto p-s3">
            {turns.length === 0 ? (
              <>
                <div className="flex flex-col gap-[6px]">
                  {QUICK.map((quick, i) => (
                    <button
                      key={quick.label}
                      type="button"
                      disabled={!scope || busy}
                      className="flex h-[30px] items-center justify-between border border-line bg-panel px-s3 text-left text-[13px] enabled:hover:border-accent enabled:hover:bg-accent-tint enabled:focus-visible:border-accent enabled:focus-visible:bg-accent-tint disabled:cursor-not-allowed disabled:opacity-45"
                      onClick={() => send(t(quick.label), quick.task)}
                    >
                      {t(quick.label)}
                      <span className="font-mono text-[11px] text-text-3">{i + 1}</span>
                    </button>
                  ))}
                </div>
                <p className="text-[12px] leading-[1.45] text-text-2">{t("Nothing is run for you. Suggestions go into the editor, and you decide when to run them.")}</p>
              </>
            ) : (
              <>
                {turns.map((turn) => (
                  <div key={turn.id} className="flex flex-col gap-s2">
                    <span className="label">{t("You")}</span>
                    <p className="text-[12.5px] leading-[1.45] break-words whitespace-pre-wrap">{turn.request}</p>
                    <Answer thread={tab.id} turn={turn} conn={conn} actions={actionsFor(turn)} />
                  </div>
                ))}
                {!busy && (
                  <div className="flex flex-wrap gap-s2">
                    {QUICK.slice(0, 3).map((quick) => (
                      <button key={quick.label} type="button" disabled={!scope} className="h-[24px] border border-line bg-panel px-s2 text-[12.5px] enabled:hover:bg-hover disabled:opacity-45" onClick={() => send(t(quick.label), quick.task)}>
                        {t(quick.label)}
                      </button>
                    ))}
                    <button type="button" className="h-[24px] px-s2 text-[12.5px] text-accent-text hover:underline" onClick={() => clearThread(tab.id)}>
                      {t("Clear")}
                    </button>
                  </div>
                )}
              </>
            )}
          </div>

          <form
            className="flex flex-none items-end gap-s2 border-t border-line p-s3"
            onSubmit={(e) => {
              e.preventDefault();
              const text = question.trim();
              if (!text) return;
              send(text, { kind: "ask", question: text });
              setQuestion("");
            }}
          >
            <textarea
              value={question}
              rows={Math.min(4, question.split("\n").length)}
              disabled={busy || !provider}
              placeholder={t("Ask about this statement…")}
              aria-label={t("Ask about this statement…")}
              data-ai-input
              onChange={(e) => setQuestion(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  e.currentTarget.form?.requestSubmit();
                } else if (question === "" && /^[1-4]$/.test(e.key) && scope && !busy) {
                  // The quick actions' number shortcuts, while nothing is typed.
                  e.preventDefault();
                  const quick = QUICK[Number(e.key) - 1];
                  send(t(quick.label), quick.task);
                }
              }}
              className="min-h-[28px] min-w-0 flex-1 resize-none border border-line bg-panel px-s2 py-[5px] text-[13px] leading-[16px] outline-none placeholder:text-text-3 focus:border-accent disabled:opacity-45"
            />
            <button type="submit" aria-label={t("Send")} title={t("Send")} disabled={busy || !provider || question.trim() === ""} className="grid size-[28px] flex-none place-items-center bg-accent-fill text-on-accent disabled:cursor-not-allowed disabled:opacity-45">
              <ArrowUp size={14} />
            </button>
          </form>
        </>
      )}
    </aside>
  );
}
