// Not drawn in the design: saved connections written to a file and read back,
// to move them to another computer or keep a copy. Passwords never go in.
import { open, save } from "@tauri-apps/plugin-dialog";
import { CircleCheck, KeyRound, TriangleAlert } from "lucide-react";
import { useState } from "react";
import { api, errorMessage, inTauri } from "../../lib/api";
import { loadSaved, openDialog, useApp } from "../../store";
import { Button, Checkbox, Dialog, Field, Strip, TextInput } from "../dialog";
import { t, tn } from "../../i18n";

type Run = { state: "idle" } | { state: "running" } | { state: "done"; count: number } | { state: "failed"; message: string };

export function ConnectionsFileDialog({ mode, ids }: { mode: "export" | "import"; ids?: string[] }) {
  const connections = useApp((s) => s.connections);
  const [chosen, setChosen] = useState<string[]>(() => (ids?.length ? ids : connections.map((c) => c.id)));
  const [path, setPath] = useState("");
  const [run, setRun] = useState<Run>({ state: "idle" });
  const exporting = mode === "export";
  const close = () => run.state !== "running" && openDialog(null);
  const ready = path.trim() !== "" && run.state !== "running" && (!exporting || chosen.length > 0);

  async function browse() {
    const picked = exporting
      ? await save({ defaultPath: "lewide-connections.json", filters: [{ name: "JSON", extensions: ["json"] }] }).catch(() => null)
      : await open({ multiple: false, directory: false, filters: [{ name: "JSON", extensions: ["json"] }] }).catch(() => null);
    if (typeof picked === "string") setPath(picked);
  }

  async function start() {
    if (!ready) return;
    setRun({ state: "running" });
    try {
      // The ids are sent in the sidebar's order, so the file lists them the same way.
      const order = connections.map((c) => c.id).filter((id) => chosen.includes(id));
      const count = exporting ? await api.exportConnections(order, path.trim()) : (await api.importConnections(path.trim())).length;
      if (!exporting) await loadSaved();
      setRun({ state: "done", count });
    } catch (e) {
      setRun({ state: "failed", message: errorMessage(e) });
    }
  }

  const done = run.state === "done";
  return (
    <Dialog
      title={exporting ? t("Export connections") : t("Import connections")}
      width={520}
      onClose={close}
      onSubmit={done ? close : () => void start()}
      footer={
        done ? (
          <Button type="submit" variant="primary">
            {t("Done")}
          </Button>
        ) : (
          <>
            <Button variant="ghost" disabled={run.state === "running"} onClick={close}>
              {t("Cancel")}
            </Button>
            <Button type="submit" variant="primary" disabled={!ready}>
              {exporting ? t("Export") : t("Import")}
            </Button>
          </>
        )
      }
    >
      <div className="flex flex-col gap-s3">
        {exporting && (
          <Field label={t("Connections")}>
            <div className="flex max-h-[220px] flex-col overflow-auto border border-line">
              {connections.map((conn) => (
                <div key={conn.id} className="flex h-[30px] items-center border-b border-line-2 px-s2 last:border-b-0">
                  <Checkbox checked={chosen.includes(conn.id)} disabled={done} onChange={(on) => setChosen(on ? [...chosen, conn.id] : chosen.filter((id) => id !== conn.id))}>
                    <span className="text-[12.5px]">{conn.name}</span>
                    <span className="ml-s2 text-[11.5px] text-text-3">{conn.driver.name}</span>
                  </Checkbox>
                </div>
              ))}
            </div>
          </Field>
        )}
        <Field label={t("File")} helper={exporting ? undefined : t("A file written by Export connections. Its connections are added beside the ones you have.")}>
          <div className="flex gap-s2">
            <TextInput mono className="flex-1" value={path} disabled={done} placeholder="/…/lewide-connections.json" onChange={(e) => setPath(e.target.value)} />
            <Button disabled={!inTauri || done} onClick={browse}>
              {t("Browse…")}
            </Button>
          </div>
        </Field>
        <Strip kind="note" icon={<KeyRound size={14} />}>
          {exporting
            ? t("Passwords and API keys are not written to the file. They stay in this computer's keychain.")
            : t("The file holds no passwords: each connection asks for its password when it is first opened.")}
        </Strip>
        {run.state === "done" && (
          <Strip kind="success" icon={<CircleCheck size={14} />}>
            {exporting
              ? tn(run.count, "{count} connection written.", "{count} connections written.")
              : run.count === 0
                ? t("The file has no connections.")
                : tn(run.count, "{count} connection added to the sidebar.", "{count} connections added to the sidebar.")}
          </Strip>
        )}
        {run.state === "failed" && (
          <Strip kind="error" icon={<TriangleAlert size={14} />}>
            <span className="selectable">{run.message}</span>
          </Strip>
        )}
      </div>
    </Dialog>
  );
}
