// Not drawn in the design ("update available / downloading states", G06): a
// small notice at the bottom right once a downloaded update is ready, so the
// user restarts when it suits them.
import { RefreshCw, X } from "lucide-react";
import { dismissUpdate, installUpdate, useUpdates } from "../updates";
import { t } from "../i18n";
import { Button } from "./dialog";

export function UpdateNotice() {
  const update = useUpdates();
  if (update.dismissed || (update.status !== "ready" && update.status !== "installing")) return null;
  const installing = update.status === "installing";
  return (
    <div role="status" className="fixed right-s4 bottom-[34px] z-40 flex w-[340px] flex-col gap-s2 border border-line bg-bg p-s3 text-[12.5px]" style={{ boxShadow: "var(--shadow-lg)" }}>
      <div className="flex items-center gap-s2">
        <RefreshCw size={13} className="flex-none text-accent-text" />
        <span className="flex-1 font-medium">{t("LewIDE {version} is ready", { version: update.info.version })}</span>
        <button type="button" aria-label={t("Later")} title={t("Later")} className="grid size-[20px] place-items-center text-text-2 hover:bg-hover" disabled={installing} onClick={dismissUpdate}>
          <X size={13} />
        </button>
      </div>
      <p className="text-text-2">{t("Downloaded and its signature checked. Restarting closes the open tabs: save the queries you want to keep first.")}</p>
      <div className="flex justify-end gap-s2">
        <Button variant="ghost" disabled={installing} onClick={dismissUpdate}>
          {t("Later")}
        </Button>
        <Button variant="primary" disabled={installing} onClick={() => void installUpdate()}>
          {installing ? t("Restarting…") : t("Restart now")}
        </Button>
      </div>
    </div>
  );
}
