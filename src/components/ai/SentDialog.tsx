// AI15: what a request carries. The structure text comes from the core, which
// is also what builds the real request; rows are never part of it.
import { useEffect, useState } from "react";
import { activeProvider, useAi } from "../../ai-store";
import { ai, type SchemaContext } from "../../lib/ai";
import { t } from "../../i18n";
import { openDialog, type Conn } from "../../store";
import { Button, Dialog, linkButton, Strip } from "../dialog";
import { Spinner } from "../Results";

function Block({ number, label, children }: { number: number; label: string; children: string }) {
  return (
    <div className="flex flex-col gap-[4px]">
      <span className="label">
        {number} · {label}
      </span>
      <pre className="selectable max-h-[240px] overflow-auto border border-line bg-bg p-s3 font-mono text-[12px] leading-[20px] whitespace-pre-wrap">{children}</pre>
    </div>
  );
}

export function SentDialog({ conn, sql, lines }: { conn: Conn | null; sql: string | null; lines: string | null }) {
  const provider = useAi(activeProvider);
  const [context, setContext] = useState<SchemaContext | "loading" | null>(conn?.session != null ? "loading" : null);
  useEffect(() => {
    if (conn?.session == null) return;
    ai.previewContext(conn.session, conn.namespace ?? undefined, sql ?? undefined)
      .then(setContext)
      .catch(() => setContext(null));
  }, [conn?.session, conn?.namespace, sql]);
  const schema = context && context !== "loading" ? context : null;
  const all = [schema?.text, sql].filter(Boolean).join("\n\n");
  const close = () => openDialog(null);

  return (
    <Dialog
      title={t("What is sent")}
      headerExtra={provider ? <span className="font-mono text-[11.5px] text-text-2">{[provider.name, provider.model].filter(Boolean).join(" · ")}</span> : undefined}
      width={800}
      offset={-26}
      onClose={close}
      footer={
        <>
          <button type="button" className={`${linkButton} mr-auto`} onClick={() => openDialog({ kind: "settings", section: "ai", aiTab: "privacy" })}>
            {t("Privacy settings")}
          </button>
          <Button onClick={() => void navigator.clipboard?.writeText(all)}>{t("Copy all")}</Button>
          <Button data-autofocus onClick={close}>
            {t("Close")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-s4">
        <p className="text-[12.5px] text-text-2">{t("Fixed instructions for the assistant come first; they contain nothing about your database.")}</p>
        {context === "loading" ? (
          <div className="flex items-center gap-s2 text-[12.5px]">
            <Spinner />
            {t("Reading the schema…")}
          </div>
        ) : schema ? (
          <Block number={1} label={t("Schema · {detailed} of {total} tables · names and types only", { detailed: schema.tables_detailed, total: schema.tables_total })}>
            {schema.text}
          </Block>
        ) : (
          <Strip kind="neutral">{t("No schema is sent with this request.")}</Strip>
        )}
        {sql && (
          <Block number={schema ? 2 : 1} label={lines ? t("Statement · lines {lines}", { lines }) : t("Statement")}>
            {sql}
          </Block>
        )}
        <Strip kind="neutral">{t("Not included: rows of any table, query results, passwords, hosts and other connection details.")}</Strip>
      </div>
    </Dialog>
  );
}
