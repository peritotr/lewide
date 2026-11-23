// The AI chat tab (AI13): a conversation about one connection's database,
// with quick replies to a question and the trimmed-context strip (AI14).
// The table checklist needs support from the core and is not built yet.
import { ArrowUp, Info, Sparkles } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { activeProvider, ask, isAsking, loadProviders, sendsSchema, stopAsking, useAi } from "../../ai-store";
import { ai, type SchemaContext } from "../../lib/ai";
import { typeOf } from "../../lib/types";
import { t } from "../../i18n";
import { editSql, openDialog, openQueryWith, useApp, type Conn, type Tab } from "../../store";
import { Button, Strip } from "../dialog";
import { TabBar } from "../TabBar";
import { EnvTag } from "../ui";
import { Answer } from "./Answer";
import { ProviderSelect } from "./AskPanel";

export function ChatView({ tab, conn }: { tab: Tab; conn: Conn | null }) {
  const turns = useAi((s) => s.threads[tab.id]) ?? [];
  const provider = useAi(activeProvider);
  const loaded = useAi((s) => s.loaded);
  const privacy = useAi((s) => s.privacy);
  const busy = useAi((s) => isAsking(tab.id, s));
  const [question, setQuestion] = useState("");
  const body = useRef<HTMLDivElement>(null);
  const production = conn?.environment === "production";
  const connected = !!conn && conn.session !== null;
  const schema = connected && sendsSchema(production, privacy);
  const tables = conn ? (conn.objects[conn.namespace ?? ""] ?? []).filter((o) => o.kind === "table").length : 0;
  // How much of the structure a request carries: large schemas are cut down.
  const [context, setContext] = useState<SchemaContext | null>(null);

  useEffect(() => void loadProviders(), []);
  useEffect(() => {
    if (!schema || conn?.session == null) return setContext(null);
    let live = true;
    ai.previewContext(conn.session, conn.namespace ?? undefined).then(
      (c) => live && setContext(c),
      () => live && setContext(null),
    );
    return () => {
      live = false;
    };
  }, [schema, conn?.session, conn?.namespace]);
  const trimmed = context !== null && context.tables_detailed < context.tables_total;
  useEffect(() => {
    body.current?.scrollTo({ top: body.current.scrollHeight });
  }, [turns.length, turns[turns.length - 1]?.text.length]); // eslint-disable-line react-hooks/exhaustive-deps

  const send = (text: string) => {
    if (busy || !provider || !text.trim()) return;
    ask(tab.id, {
      request: text.trim(),
      task: { kind: "ask", question: text.trim() },
      session: conn?.session ?? null,
      engine: conn ? typeOf(conn.engine, conn.driver.id).name : undefined,
      namespace: conn?.namespace ?? undefined,
      production,
      conversation: true,
    });
  };

  // "Insert into editor": the most recently opened query tab of this connection, or a new one.
  const insert = (sql: string) => {
    if (!conn) return;
    const target = [...useApp.getState().tabs].reverse().find((x) => x.kind === "query" && x.connectionId === conn.id);
    if (!target) return openQueryWith(conn.id, sql);
    editSql(target.id, target.sql.trim() ? `${target.sql.replace(/\s+$/, "")}\n\n${sql}\n` : `${sql}\n`);
    useApp.setState({ activeTabId: target.id });
  };

  return (
    <main className="grid min-h-0 min-w-0 bg-panel" style={{ gridTemplateRows: `34px ${production ? "4px" : "0px"} 36px auto minmax(0,1fr) auto` }} onKeyDown={(e) => e.key === "Escape" && busy && stopAsking(tab.id)}>
      <TabBar />
      <div title={production ? t("Production connection") : undefined} className={production ? "production-band" : ""} />

      <div className="flex items-center gap-s3 border-b border-line px-[10.2px] whitespace-nowrap">
        <span className="font-mono text-[12.5px] font-medium">{conn ? [conn.name, conn.namespace].filter(Boolean).join(" / ") : t("No connection")}</span>
        {conn && production && <EnvTag environment={conn.environment} />}
        {schema && <span className="text-[12.5px] font-medium">{trimmed ? t("{detailed} of {total} tables in context", { detailed: context.tables_detailed, total: context.tables_total }) : t("{count} tables in context", { count: context?.tables_total ?? tables })}</span>}
        <span className="min-w-0 truncate text-[12.5px] text-text-2">{schema ? t("Schema and messages only · no row data") : t("Messages only · no schema, no row data")}</span>
        <button type="button" className="text-[12.5px] text-accent-text hover:underline" onClick={() => openDialog({ kind: "ai-sent", connId: schema ? conn!.id : null, sql: null, lines: null })}>
          {t("View what is sent")}
        </button>
        <span className="flex-1" />
        <ProviderSelect />
      </div>
      {trimmed ? (
        // "Choose tables" needs the core to take a table list; it is not built.
        <Strip kind="note" icon={<Info size={13} />} className="border-x-0 border-t-0 bg-accent-tint">
          {t("Context trimmed. The schema is too large for this model, so the {detailed} most relevant of {total} tables are described in full and the others by name only. Answers about other tables may be wrong.", { detailed: context.tables_detailed, total: context.tables_total })}
        </Strip>
      ) : (
        <div />
      )}

      <div ref={body} className="min-h-0 overflow-auto">
        {loaded && !provider ? (
          <div className="mx-auto flex max-w-[520px] flex-col items-start gap-s3 p-s6">
            <Sparkles size={22} strokeWidth={1.5} className="text-accent-text" />
            <h3 className="font-heading text-[20px] leading-[1.1] font-semibold">{t("Set up an AI provider")}</h3>
            <p className="text-[12.5px] leading-[1.5] text-text-2">{t("Add a key from Anthropic, OpenAI, Google or your own endpoint to explain, check and improve SQL here.")}</p>
            <Button variant="primary" onClick={() => openDialog({ kind: "settings", section: "ai" })}>
              {t("Open AI settings")}
            </Button>
          </div>
        ) : turns.length === 0 ? (
          <div className="flex max-w-[800px] flex-col gap-s2 p-[20.4px]">
            <h3 className="font-heading text-[20px] leading-[1.1] font-semibold">{t("Describe the query you need")}</h3>
            <p className="text-[12.5px] leading-[1.5] text-text-2">{t("The assistant writes SQL for this database from its structure, asks when something is ambiguous, and points out indexes that would help. It cannot see rows and never runs anything.")}</p>
          </div>
        ) : (
          <div className="flex max-w-[800px] flex-col gap-[13.6px] p-[20.4px]">
            {turns.map((turn) => (
              <div key={turn.id} className="flex flex-col gap-[13.6px]">
                <div className="grid grid-cols-[44px_minmax(0,1fr)]">
                  <span className="label pt-[3px]">{t("You")}</span>
                  <p className="selectable text-[13px] leading-[1.5] break-words whitespace-pre-wrap">{turn.request}</p>
                </div>
                <div className="grid grid-cols-[44px_minmax(0,1fr)]">
                  <Sparkles size={14} strokeWidth={1.5} className="mt-[2px] text-accent-text" />
                  <Answer thread={tab.id} turn={turn} conn={conn} actions={{ insert, retry: () => send(turn.request), reply: busy ? undefined : send }} />
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      <form
        className="flex items-end gap-s2 border-t border-line p-s3"
        onSubmit={(e) => {
          e.preventDefault();
          send(question);
          setQuestion("");
        }}
      >
        <textarea
          value={question}
          rows={Math.min(6, question.split("\n").length)}
          autoFocus
          disabled={busy || !provider}
          placeholder={t("Ask for a query, or about this database…")}
          aria-label={t("Ask for a query, or about this database…")}
          onChange={(e) => setQuestion(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              e.currentTarget.form?.requestSubmit();
            }
          }}
          className="min-h-[32px] max-w-[800px] min-w-0 flex-1 resize-none border border-line bg-panel px-s3 py-[7px] text-[13px] leading-[16px] outline-none placeholder:text-text-3 focus:border-accent disabled:opacity-45"
        />
        <button type="submit" aria-label={t("Send")} title={t("Send")} disabled={busy || !provider || question.trim() === ""} className="grid size-[32px] flex-none place-items-center bg-accent-fill text-on-accent disabled:cursor-not-allowed disabled:opacity-45">
          <ArrowUp size={15} />
        </button>
      </form>
    </main>
  );
}
