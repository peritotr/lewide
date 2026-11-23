// T10: the statements pending row changes would run, shown before applying.
// And the question asked before pending changes are thrown away (not drawn in the design).
import { useState } from "react";
import { pendingCount, pendingCounts } from "../lib/edit";
import { applyTableChanges, discardTableChanges, openDialog, type Conn, type TableView } from "../store";
import { Button, Dialog, TextInput } from "./dialog";
import { Tag } from "./ui";
import { t, tn } from "../i18n";

const SYMBOL: Record<string, { mark: string; colour: string; tint: string }> = {
  INSERT: { mark: "+", colour: "text-ok", tint: "bg-ok-tint" },
  UPDATE: { mark: "~", colour: "text-accent-text", tint: "bg-accent-tint" },
  DELETE: { mark: "−", colour: "text-danger", tint: "bg-danger-tint" },
};

export function ApplyChangesDialog({ tabId, view, conn, statements }: { tabId: string; view: TableView; conn: Conn; statements: string[] }) {
  const production = conn.environment === "production";
  const [typed, setTyped] = useState("");
  const allowed = !production || typed === conn.name;
  const count = pendingCount(view.pending);
  const apply = () => {
    if (!allowed) return;
    openDialog(null);
    void applyTableChanges(tabId);
  };
  return (
    <Dialog
      alert={production}
      title={tn(count, "Apply {count} change to {table}", "Apply {count} changes to {table}", { table: view.name })}
      headerExtra={production ? <Tag kind="warn">{t("PRODUCTION")}</Tag> : undefined}
      band={production ? "production" : undefined}
      width={760}
      onClose={() => openDialog(null)}
      onSubmit={apply}
      footer={
        <>
          <span className="mr-auto">
            <Button variant="ghost" onClick={() => void navigator.clipboard?.writeText(statements.join(";\n") + ";")}>
              {t("Copy SQL")}
            </Button>
          </span>
          <Button onClick={() => openDialog(null)}>{t("Cancel")}</Button>
          <Button variant="primary" type="submit" disabled={!allowed}>
            {tn(count, "Apply {count} change", "Apply {count} changes")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-s3">
        <p className="text-[12.5px] text-text-2">{tn(statements.length, "One transaction. The statement is applied, or nothing is.", "One transaction. All {count} statements are applied together, or none of them.")}</p>
        <div className="max-h-[360px] overflow-auto border border-line">
          {statements.map((statement, i) => {
            const look = SYMBOL[statement.trimStart().slice(0, 6).toUpperCase()] ?? { mark: "", colour: "", tint: "" };
            return (
              <div key={i} className={`grid grid-cols-[28px_minmax(0,1fr)] border-b border-line-2 last:border-b-0 ${look.tint}`}>
                <span className={`pt-[6px] text-center font-mono text-[13px] font-medium ${look.colour}`}>{look.mark}</span>
                <pre className="selectable py-[6px] pr-s3 font-mono text-[12.5px] leading-[20px] break-words whitespace-pre-wrap">{statement}</pre>
              </div>
            );
          })}
        </div>
        {production && (
          <label className="flex flex-col gap-[4px]">
            <span className="text-[12px] text-text-2">{t("Type the connection name to apply on Production")}</span>
            <TextInput mono data-autofocus value={typed} onChange={(e) => setTyped(e.target.value)} after={<span className="font-mono text-[12px] whitespace-nowrap text-text-3">{conn.name}</span>} />
          </label>
        )}
      </div>
    </Dialog>
  );
}

export function DiscardChangesDialog({ tabId, view, then }: { tabId: string; view: TableView; then: (() => void) | null }) {
  const parts = pendingCounts(view.pending);
  const count = pendingCount(view.pending);
  const discard = () => {
    openDialog(null);
    discardTableChanges(tabId);
    then?.();
  };
  return (
    <Dialog
      alert
      title={tn(count, "Discard {count} change?", "Discard {count} changes?")}
      width={480}
      onClose={() => openDialog(null)}
      footer={
        <>
          <Button data-autofocus onClick={() => openDialog(null)}>
            {t("Cancel")}
          </Button>
          <Button variant="danger" onClick={discard}>
            {t("Discard")}
          </Button>
        </>
      }
    >
      <p className="leading-[1.5]">
        {t("Changes that are not applied will be lost.")}{" "}
        <span className="font-mono text-[12px] text-text-2">
          {[parts.added > 0 && `+ ${parts.added}`, parts.modified > 0 && `~ ${parts.modified}`, parts.deleted > 0 && `− ${parts.deleted}`].filter(Boolean).join("  ")}
        </span>
      </p>
    </Dialog>
  );
}
