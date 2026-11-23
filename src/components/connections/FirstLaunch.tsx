// C01: the workspace before any connection exists.
import { Plus } from "lucide-react";
import { openDialog } from "../../store";
import { Button } from "../dialog";
import { Mark } from "../ui";
import { isMac } from "../TitleBar";
import { t } from "../../i18n";

const MOD = isMac ? "⌘" : "Ctrl+";

export function FirstLaunch() {
  return (
    <main className="grid min-w-0 grid-rows-[34px_minmax(0,1fr)] bg-panel">
      <div className="border-b border-line bg-surface" />
      <div className="grid place-items-center">
        <div className="flex w-[440px] max-w-[calc(100%-40px)] flex-col items-start gap-s4">
          <Mark size={44} />
          <h1 className="font-heading text-[30px] leading-[1.1] font-semibold">{t("Connect to a database")}</h1>
          <p className="leading-[1.5] text-text-2">
            {t("Add your first connection to browse tables and run queries. Connection details are stored on this device.")}
          </p>
          <Button variant="primary" tall onClick={() => openDialog({ kind: "picker" })}>
            <Plus size={13} />
            {t("New connection")}
            <span className="font-mono text-[11px] font-normal tracking-normal">{MOD}N</span>
          </Button>
        </div>
      </div>
    </main>
  );
}
