import { TriangleAlert } from "lucide-react";
import { useEffect, useState } from "react";
import { typeOf } from "../lib/types";
import { activeTab, connOf, useApp } from "../store";
import { Spinner } from "./Results";
import { EnvTag } from "./ui";
import { t, tn } from "../i18n";

/** Re-renders once a second while `active`, for the transaction's age. */
function useSecond(active: boolean) {
  const [, tick] = useState(0);
  useEffect(() => {
    if (!active) return;
    const timer = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(timer);
  }, [active]);
}

const age = (since: number) => {
  const seconds = Math.max(0, Math.floor((Date.now() - since) / 1000));
  return `${String(Math.floor(seconds / 60)).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`;
};

export function StatusBar() {
  const tab = useApp(activeTab);
  const conn = useApp((s) => connOf(s, activeTab(s)));
  const running = tab?.run.state === "running";
  useSecond(!!conn?.tx || running);
  const connected = conn !== null && conn.session !== null;
  const production = conn?.environment === "production";
  const set = tab?.sets[tab.activeSet];
  const page = tab?.table?.page;
  const mono = "font-mono text-[11.5px]";
  const state = conn === null ? null : connected ? t("Connected") : conn.status === "connecting" ? t("Connecting") : t("Disconnected");

  return (
    <footer className="flex items-center gap-s4 border-t border-line bg-surface pr-s3 text-[12px] whitespace-nowrap text-text-2">
      <div className={`flex h-full items-center gap-[6px] border-r border-line px-s3 ${production && connected ? "bg-warn-tint font-medium text-warn-text" : ""}`}>
        {connected ? (
          <i className="size-[7px] flex-none bg-ok" />
        ) : conn?.status === "lost" ? (
          <i className="size-[9px] flex-none border border-warn-text" style={{ background: "linear-gradient(to top right, transparent calc(50% - 0.6px), var(--warn-text) calc(50% - 0.6px), var(--warn-text) calc(50% + 0.6px), transparent calc(50% + 0.6px))" }} />
        ) : (
          <i className="size-[7px] flex-none border border-text-3" />
        )}
        {conn ? `${state} · ${conn.name}` : tab ? t("No connection for this tab") : t("No connection")}
        {production && <EnvTag environment="production" long />}
      </div>
      {conn && <span className={mono}>{[typeOf(conn.engine, conn.driver.id).name, conn.version?.split(" ")[0]].filter(Boolean).join(" ")}</span>}
      {conn && <span className={mono}>{[conn.driver.capabilities.schemas ? conn.config.database : null, conn.namespace].filter(Boolean).join(".")}</span>}
      <span className="ml-auto" />

      {tab?.run.state === "running" ? (
        <span className="flex items-center gap-[6px]">
          <Spinner />
          {t("Running…")} <span className={mono}>{((Date.now() - tab.run.startedAt) / 1000).toFixed(1)} s</span>
        </span>
      ) : tab?.run.state === "cancelled" ? (
        <>
          <span>{t("Cancelled")}</span>
          <span className={mono}>{(tab.run.elapsedMs / 1000).toFixed(1)} s</span>
        </>
      ) : tab?.run.state === "error" ? (
        <>
          <span className="text-danger">{tab.run.error.code ? t("Error {code}", { code: tab.run.error.code }) : t("Error")}</span>
          <span className={mono}>{tab.run.elapsedMs} ms</span>
        </>
      ) : set ? (
        <>
          <span className={mono}>
            {set.result.rows_affected !== null
              ? tn(set.result.rows_affected, "{count} row affected", "{count} rows affected")
              : set.result.truncated
                ? t("{count}+ rows", { count: set.result.rows.length })
                : tn(set.result.rows.length, "{count} row", "{count} rows")}
          </span>
          <span className={mono}>{tab?.sets.reduce((total, s) => total + s.result.elapsed_ms, 0)} ms</span>
        </>
      ) : null}
      {page && (
        <>
          <span className={mono}>{tn(page.rows.length, "{count} row", "{count} rows")}</span>
          <span className={mono}>{page.elapsed_ms} ms</span>
        </>
      )}

      {conn?.tx ? (
        // An open transaction holds locks and unsaved work: it gets the warning treatment.
        <span className="flex h-full items-center gap-[6px] bg-warn-tint px-s3 font-medium text-warn-text">
          <TriangleAlert size={12} />
          {t("Transaction open")} · {tn(conn.tx.statements, "{count} statement", "{count} statements")} · <span className={mono}>{age(conn.tx.openedAt)}</span>
        </span>
      ) : (
        connected && <span>{conn.txMode === "manual" ? t("Manual · no open transaction") : t("Auto-commit · no open transaction")}</span>
      )}
      {tab?.kind === "query" && <span className={mono}>{t("Ln {line}, Col {column}", { line: tab.caret.line, column: tab.caret.col })}</span>}
      <span>UTF-8</span>
    </footer>
  );
}
