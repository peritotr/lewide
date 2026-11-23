// One answer of the assistant, as the Ask AI panel and the chat tab draw it:
// prose, review findings (AI05), a clarifying question with quick replies
// (AI14), SQL blocks, a suggested statement as a diff (AI07), index advice
// (AI09), the streaming cursor (AI10), errors (AI11) and the usage line.
import { Check, Sparkles, Square, TriangleAlert } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { stopAsking, updateTurn, type Turn } from "../../ai-store";
import { clarification, findings, segments, verdict, type AiError, type Finding, type Severity } from "../../lib/ai";
import { diffCounts, diffLines } from "../../lib/diff";
import { t, tn } from "../../i18n";
import { openDialog, openQueryWith, type Conn } from "../../store";
import { Button, Strip } from "../dialog";
import { Tag } from "../ui";

const copy = (text: string) => void navigator.clipboard?.writeText(text);
const small = "h-[24px] border border-line bg-panel px-s2 text-[12.5px] whitespace-nowrap enabled:hover:bg-hover disabled:cursor-not-allowed disabled:opacity-45";
const link = "text-[12.5px] text-accent-text hover:underline disabled:cursor-not-allowed disabled:opacity-45 disabled:no-underline";

/** Prose with `code` spans in mono. */
function Prose({ text }: { text: string }) {
  return (
    <p className="selectable text-[12.5px] leading-[1.5] break-words whitespace-pre-wrap">
      {text.split(/(`[^`\n]+`)/g).map((part, i) =>
        part.startsWith("`") && part.endsWith("`") && part.length > 2 ? (
          <span key={i} className="font-mono text-[12px]">
            {part.slice(1, -1)}
          </span>
        ) : (
          part
        ),
      )}
    </p>
  );
}

export function UsageLine({ turn }: { turn: Turn }) {
  if (!turn.model && !turn.usage) return null;
  const { input_tokens: input, output_tokens: output } = turn.usage ?? { input_tokens: null, output_tokens: null };
  return (
    <p className="flex items-center gap-[6px] font-mono text-[11px] text-text-3">
      <Sparkles size={11} strokeWidth={1.5} className="flex-none text-accent-text" />
      {[turn.model, input !== null && output !== null ? t("{input} in · {output} out tokens", { input, output }) : null].filter(Boolean).join(" · ")}
    </p>
  );
}

const ERRORS: Partial<Record<AiError["reason"], { title: string; action: "settings" | "retry" }>> = {
  auth: { title: "Invalid API key", action: "settings" },
  rate_limit: { title: "Usage limit reached", action: "settings" },
  network: { title: "No network", action: "retry" },
  model: { title: "Model not found", action: "settings" },
  not_configured: { title: "No AI provider is set up", action: "settings" },
};

function ErrorBlock({ error, onRetry }: { error: AiError; onRetry?(): void }) {
  const known = ERRORS[error.reason];
  const titles: Record<string, string> = {
    "Invalid API key": t("Invalid API key"),
    "Usage limit reached": t("Usage limit reached"),
    "No network": t("No network"),
    "Model not found": t("Model not found"),
    "No AI provider is set up": t("No AI provider is set up"),
  };
  const settings = known?.action === "settings";
  return (
    <div role="alert" className="flex flex-col gap-s2 border border-danger bg-danger-tint p-s3 text-[12.5px]">
      <div className="flex items-center gap-s2">
        <TriangleAlert size={13} className="flex-none text-danger" />
        <span className="font-medium">{known ? titles[known.title] : t("The request failed")}</span>
      </div>
      <p className="selectable font-mono text-[12px] break-words whitespace-pre-wrap">{error.message}</p>
      {error.reason === "network" && <p className="text-text-2">{t("Nothing was sent.")}</p>}
      <div>
        {settings ? (
          <button type="button" className={small} onClick={() => openDialog({ kind: "settings", section: "ai" })}>
            {error.reason === "rate_limit" ? t("Switch provider") : error.reason === "model" ? t("Choose model") : t("Open AI settings")}
          </button>
        ) : (
          onRetry && (
            <button type="button" className={small} onClick={onRetry}>
              {t("Retry")}
            </button>
          )
        )}
      </div>
    </div>
  );
}

export interface AnswerActions {
  /** Panel only: replaces the scoped lines with the suggestion. Null when the text changed since the request. */
  apply?: ((sql: string) => void) | null;
  insertBelow?: ((sql: string) => void) | null;
  undo?(): void;
  /** Chat only: inserts at the caret of the connection's last query tab. */
  insert?(sql: string): void;
  retry?(): void;
  /** Panel only: moves the caret to a line of the editor and flashes it (AI05). */
  goToLine?(line: number): void;
  /** Panel only: asks for a corrected statement after a review. */
  suggestFix?(): void;
  /** Sends a quick reply to a clarifying question (AI14). */
  reply?(text: string): void;
}

const SEVERITY: Record<Severity, { word: () => string; kind: "danger" | "warn" | undefined }> = {
  error: { word: () => t("Error"), kind: "danger" },
  warning: { word: () => t("Warning"), kind: "warn" },
  note: { word: () => t("Note"), kind: undefined },
};

/** AI05: the verdict with its counts, then one block per finding. */
function Findings({ list, scope, actions }: { list: Finding[]; scope: Turn["scope"]; actions: AnswerActions }) {
  const count = (severity: Severity) => list.filter((f) => f.severity === severity).length;
  const errors = count("error");
  const counts = [
    errors ? tn(errors, "{count} error", "{count} errors") : null,
    count("warning") ? tn(count("warning"), "{count} warning", "{count} warnings") : null,
    count("note") ? tn(count("note"), "{count} note", "{count} notes") : null,
  ].filter(Boolean);
  return (
    <div className="flex flex-col gap-s2">
      <div className="flex items-center gap-s2 text-[12.5px]">
        <TriangleAlert size={14} className={errors ? "text-danger" : "text-warn-text"} />
        <span className="font-medium">{t("Problems found")}</span>
        <span className="font-mono text-[11.5px] text-text-2">{counts.join(" · ")}</span>
      </div>
      {list.map((finding, i) => {
        // The finding counts lines within the statement that was sent.
        const line = finding.line !== null && scope ? scope.fromLine + finding.line - 1 : null;
        return (
          <div key={i} className="flex flex-col gap-[6px] border border-line bg-panel p-s2">
            <div className="flex items-center gap-s2">
              <Tag kind={SEVERITY[finding.severity].kind}>{SEVERITY[finding.severity].word()}</Tag>
              <span className="flex-1" />
              {line !== null && (
                <button type="button" className="font-mono text-[11.5px] text-accent-text hover:underline disabled:no-underline" disabled={!actions.goToLine} onClick={() => actions.goToLine?.(line)}>
                  {t("line {number}", { number: line })}
                </button>
              )}
            </div>
            <Prose text={finding.text} />
          </div>
        );
      })}
      {actions.suggestFix && (
        <div>
          <button type="button" className={small} onClick={actions.suggestFix}>
            {t("Suggest a fix")}
          </button>
        </div>
      )}
    </div>
  );
}

/** AI14: one question and its likely answers as buttons; the first has the focus. */
function Question({ question, options, onReply }: { question: string; options: { value: string; gloss: string | null }[]; onReply?(text: string): void }) {
  return (
    <div className="flex flex-col gap-s2">
      <p className="selectable text-[12.5px] leading-[1.5] font-medium">{question}</p>
      {options.map((option, i) => (
        <button
          key={option.value}
          type="button"
          autoFocus={i === 0}
          disabled={!onReply}
          className="flex min-h-[30px] items-center gap-s2 border border-line bg-panel px-s3 text-left text-[12.5px] enabled:hover:border-accent enabled:hover:bg-accent-tint enabled:focus-visible:border-accent enabled:focus-visible:bg-accent-tint enabled:focus-visible:outline-none disabled:opacity-45"
          onClick={() => onReply?.(option.value)}
        >
          <span className="font-mono text-[12px]">{option.value}</span>
          {option.gloss && <span className="min-w-0 truncate text-text-2">— {option.gloss}</span>}
        </button>
      ))}
      {options.length > 0 && <p className="text-[12px] text-text-2">{t("Or type your own answer below.")}</p>}
    </div>
  );
}

function SqlBlock({ sql, title, conn, actions, children }: { sql: string; title: ReactNode; conn: Conn | null; actions: AnswerActions; children?: ReactNode }) {
  return (
    <div className="border border-line bg-bg">
      <div className="flex min-h-[30px] flex-wrap items-center gap-x-s3 gap-y-[2px] border-b border-line-2 px-s3 py-[3px]">
        <span className="label mr-auto">{title}</span>
        {actions.insert && (
          <button type="button" className={link} onClick={() => actions.insert!(sql)}>
            {t("Insert into editor")}
          </button>
        )}
        <button type="button" className={link} disabled={!conn} onClick={() => conn && openQueryWith(conn.id, sql)}>
          {t("Open in new query")}
        </button>
        <button type="button" className={link} onClick={() => copy(sql)}>
          {t("Copy")}
        </button>
      </div>
      <pre className="selectable overflow-x-auto px-s3 py-s2 font-mono text-[12px] leading-[19px] whitespace-pre-wrap">{sql}</pre>
      {children}
    </div>
  );
}

export function Answer({ thread, turn, conn, actions }: { thread: string; turn: Turn; conn: Conn | null; actions: AnswerActions }) {
  const streaming = turn.state === "streaming";
  // Elapsed time while the answer is being written.
  const [, tick] = useState(0);
  useEffect(() => {
    if (!streaming) return;
    const timer = window.setInterval(() => tick((n) => n + 1), 100);
    return () => window.clearInterval(timer);
  }, [streaming]);

  if (turn.state === "error" && turn.error) return <ErrorBlock error={turn.error} onRetry={actions.retry} />;

  if (streaming) {
    return (
      <div className="flex flex-col gap-s2">
        <p className="selectable text-[12.5px] leading-[1.5] break-words whitespace-pre-wrap">
          {turn.text}
          <i className="ml-[2px] inline-block h-[14px] w-[7px] bg-accent align-[-2px]" />
        </p>
        <div className="flex items-center gap-s3">
          <button type="button" className={`${small} flex items-center gap-[6px]`} onClick={() => stopAsking(thread)}>
            <Square size={10} />
            {t("Stop")}
            <span className="font-mono text-[11px] text-text-3">Esc</span>
          </button>
          <span className="font-mono text-[11.5px] text-text-2">{t("Writing… {seconds} s", { seconds: ((Date.now() - turn.startedAt) / 1000).toFixed(1) })}</span>
        </div>
      </div>
    );
  }

  const review = turn.task.kind === "review" ? verdict(turn.text) : null;
  const reviewed = review ? turn.text.replace(/^\s*verdict:[^\n]*\n?/i, "") : turn.text;
  // A review lists its findings one per line; they get blocks of their own.
  const found = review && review !== "ok" ? findings(reviewed) : { findings: [], rest: reviewed };
  const asked = review ? null : clarification(found.rest);
  const parts = segments(asked ? asked.rest : found.rest);
  const production = conn?.environment === "production";
  const suggested = parts.some((p) => p.kind === "sql" && p.role === "suggestion");

  return (
    <div className="flex flex-col gap-s3">
      {review === "ok" && (
        <Strip kind="success" icon={<Check size={13} />}>
          <span className="font-medium">{t("No problems found")}</span>
          <p className="text-text-2">{t("This is a review, not a guarantee.")}</p>
        </Strip>
      )}
      {found.findings.length > 0 && <Findings list={found.findings} scope={turn.scope} actions={{ ...actions, suggestFix: suggested ? undefined : actions.suggestFix }} />}
      {asked && <Question question={asked.question} options={asked.options} onReply={actions.reply} />}
      {(review === "errors" || review === "warnings") && found.findings.length === 0 && (
        <div className="flex items-center gap-s2 text-[12.5px]">
          <TriangleAlert size={14} className={review === "errors" ? "text-danger" : "text-warn-text"} />
          <span className="font-medium">{t("Problems found")}</span>
          <Tag kind={review === "errors" ? "danger" : "warn"}>{review === "errors" ? t("Error") : t("Warning")}</Tag>
        </div>
      )}

      {parts.map((part, i) => {
        if (part.kind === "text") return <Prose key={i} text={part.text} />;
        if (part.role === "suggestion" && turn.scope && actions.apply !== undefined) {
          if (turn.dismissed) return null;
          if (turn.applied) {
            return (
              <Strip key={i} kind="success" icon={<Check size={13} />}>
                <span className="font-medium">{t("Applied to the editor")}</span>
                <p className="text-text-2">{t("Lines {lines} replaced. Not run yet.", { lines: turn.applied.lines })}</p>
                <button type="button" className={link} onClick={actions.undo}>
                  {t("Undo")}
                </button>
              </Strip>
            );
          }
          const lines = diffLines(turn.scope.sql, part.sql);
          const counts = diffCounts(lines);
          const range = `${turn.scope.fromLine}–${turn.scope.toLine}`;
          return (
            <div key={i} className="border border-line bg-panel">
              <div className="flex h-[30px] items-center gap-s3 border-b border-line-2 px-s3">
                <span className="font-medium">{t("Suggested statement")}</span>
                <span className="ml-auto font-mono text-[11.5px] text-text-2">{t("{added} added · {removed} removed", counts)}</span>
              </div>
              <div className="selectable font-mono text-[11.5px] leading-[18px]">
                {lines.map((line, n) => (
                  <div key={n} className={`grid grid-cols-[18px_minmax(0,1fr)] ${line.kind === "added" ? "bg-ok-tint" : line.kind === "removed" ? "bg-danger-tint" : ""}`}>
                    <span className={`text-center font-medium select-none ${line.kind === "added" ? "text-ok" : "text-danger"}`}>{line.kind === "added" ? "+" : line.kind === "removed" ? "−" : ""}</span>
                    <span className="break-words whitespace-pre-wrap">{line.text || " "}</span>
                  </div>
                ))}
              </div>
              <div className="flex flex-wrap items-center gap-s2 border-t border-line-2 p-s3">
                <Button variant="primary" disabled={!actions.apply} title={actions.apply ? undefined : t("The statement changed since this suggestion was made.")} onClick={() => actions.apply?.(part.sql)}>
                  {t("Apply")}
                </Button>
                <button type="button" className={small} disabled={!actions.insertBelow} onClick={() => actions.insertBelow?.(part.sql)}>
                  {t("Insert below")}
                </button>
                <button type="button" className={small} onClick={() => copy(part.sql)}>
                  {t("Copy")}
                </button>
                <button type="button" className={small} onClick={() => updateTurn(thread, turn.id, { dismissed: true })}>
                  {t("Dismiss")}
                </button>
              </div>
              <p className="px-s3 pb-s3 text-[12px] text-text-2">{t("Apply replaces lines {lines} in the editor. It does not run the statement.", { lines: range })}</p>
            </div>
          );
        }
        if (part.role === "index") {
          return (
            <SqlBlock key={i} sql={part.sql} conn={conn} actions={{}} title={<Tag kind="accent">{t("Suggested index")}</Tag>}>
              {production && (
                <Strip kind="warning" icon={<TriangleAlert size={13} />} className="m-s2">
                  {t("Production. Building this index uses I/O and can take minutes. It is not run for you.")}
                </Strip>
              )}
            </SqlBlock>
          );
        }
        return (
          <SqlBlock key={i} sql={part.sql} conn={conn} actions={actions} title="SQL">
            <p className="border-t border-line-2 px-s3 py-[5px] text-[12px] text-text-2">{t("Not run. Review it, then run it yourself.")}</p>
          </SqlBlock>
        );
      })}

      {turn.state === "stopped" && <p className="text-[12.5px] text-text-3">{t("Stopped")}</p>}
      <UsageLine turn={turn} />
    </div>
  );
}
