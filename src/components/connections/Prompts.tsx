// C11 (password prompt) and C14 (remove confirmation).
import { Eye, EyeOff, TriangleAlert } from "lucide-react";
import { useState } from "react";
import type { ConnectAnswers, Secrets } from "../../lib/api";
import { connect, openDialog, removeConnection, useApp, type Conn } from "../../store";
import { Button, Checkbox, Dialog, Field, Strip, TextInput } from "../dialog";
import { EnvTag } from "../ui";
import { t, tn } from "../../i18n";

export function PasswordPrompt({ conn, answers, error }: { conn: Conn; answers: ConnectAnswers; error?: string }) {
  const [secrets, setSecrets] = useState<Secrets>({});
  const [save, setSave] = useState(false);
  const [shown, setShown] = useState(false);
  const { config, requires } = conn;
  const where = conn.driver.capabilities.file_based
    ? config.database
    : `${config.user ? `${config.user}@` : ""}${config.host}:${config.port ?? conn.driver.default_port ?? ""}`;

  const field = (key: keyof Secrets, label: string, first: boolean) => (
    <Field label={label}>
      <TextInput
        mono
        data-autofocus={first ? "" : undefined}
        type={shown ? "text" : "password"}
        value={secrets[key] ?? ""}
        onChange={(e) => setSecrets({ ...secrets, [key]: e.target.value || undefined })}
        after={
          <button type="button" tabIndex={-1} title={shown ? t("Hide") : t("Show")} aria-label={shown ? t("Hide") : t("Show")} className="text-text-2 hover:text-text" onClick={() => setShown(!shown)}>
            {shown ? <EyeOff size={14} /> : <Eye size={14} />}
          </button>
        }
      />
    </Field>
  );
  // What is stored already is filled in by the core; only these are typed.
  const wanted: [keyof Secrets, string][] = [
    ...(requires.password ? [["password", t("Password")] as [keyof Secrets, string]] : []),
    ...(requires.ssh_password ? [["ssh_password", t("SSH password for {user}", { user: config.ssh?.user ?? "" })] as [keyof Secrets, string]] : []),
    ...(requires.ssh_passphrase ? [["ssh_passphrase", t("Key passphrase")] as [keyof Secrets, string]] : []),
  ];

  return (
    <Dialog
      title={t("Connect to {connection}", { connection: conn.name })}
      headerExtra={<EnvTag environment={conn.environment} />}
      band={conn.environment === "production" ? "production" : undefined}
      width={440}
      offset={120}
      onClose={() => openDialog(null)}
      onSubmit={() => connect(conn.id, { ...answers, secrets, saveSecrets: save })}
      footer={
        <>
          <Button onClick={() => openDialog(null)}>{t("Cancel")}</Button>
          <Button variant="primary" type="submit">
            {t("Connect")} <span className="font-mono text-[11px] font-normal tracking-normal">↵</span>
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-s3">
        <span className="selectable font-mono text-[12px] text-text-2">{where}</span>
        {wanted.map(([key, label], i) => (
          <div key={key}>{field(key, label, i === 0)}</div>
        ))}
        {error && (
          <Strip kind="error" icon={<TriangleAlert size={14} />}>
            <span className="font-medium">{t("Connection failed")}</span>
            <p className="selectable pt-[2px] font-mono text-[12px] break-words whitespace-pre-wrap">{error}</p>
          </Strip>
        )}
        <Checkbox checked={save} onChange={setSave}>
          {t("Save in system keychain")}
        </Checkbox>
      </div>
    </Dialog>
  );
}

export function RemoveDialog({ conn }: { conn: Conn }) {
  const openTabs = useApp((s) => s.tabs.filter((t) => t.connectionId === conn.id).length);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <Dialog
      alert
      title={t("Remove {connection}?", { connection: conn.name })}
      band={conn.environment === "production" ? "production" : undefined}
      width={460}
      offset={120}
      // A removal in progress cannot be closed away.
      onClose={() => !busy && openDialog(null)}
      footer={
        <>
          <Button data-autofocus disabled={busy} onClick={() => openDialog(null)}>
            {t("Cancel")}
          </Button>
          <Button
            variant="danger"
            disabled={busy}
            onClick={() => {
              setBusy(true);
              removeConnection(conn.id).catch((e) => {
                setBusy(false);
                setError(String((e as { message?: unknown })?.message ?? e));
              });
            }}
          >
            {t("Remove connection")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-s2 leading-[1.5]">
        <p>
          {t("The connection and its saved password are removed from this app.")}
          {openTabs > 0 && ` ${tn(openTabs, "{count} open tab for this connection will close.", "{count} open tabs for this connection will close.")}`}
        </p>
        <p className="text-text-2">{t("The database itself is not affected.")}</p>
        {error && <p className="selectable text-[12px] text-danger">{error}</p>}
      </div>
    </Dialog>
  );
}
