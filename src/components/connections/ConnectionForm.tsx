// C03–C08: the connection form, for a new connection and for editing one.
import { open as openFile } from "@tauri-apps/plugin-dialog";
import { Check, CheckSquare, Clock, Eye, EyeOff, Lock, TriangleAlert } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  api,
  errorKind,
  errorMessage,
  hostKeyChanged,
  inTauri,
  unknownHostKey,
  type ConnectionConfig,
  type TlsMode,
  type UnknownHostKey,
} from "../../lib/api";
import { typeOf } from "../../lib/types";
import { connect, findConn, openDialog, saveConnection, useApp, type Conn, type Environment } from "../../store";
import { Button, Checkbox, Dialog, Field, linkButton, Radio, Segmented, Strip, Switch, TextInput, TypeBadge } from "../dialog";
import { colourVar, NOT_BUILT } from "../ui";
import { UnknownHostDialog } from "./HostKeyDialogs";
import { msg, t, tx } from "../../i18n";

export interface Draft {
  name: string;
  environment: Environment;
  colour: Conn["colour"];
  colourPicked: boolean;
  host: string;
  port: string;
  database: string;
  user: string;
  password: string;
  keepPassword: boolean;
  filePath: string;
  ssh: {
    enabled: boolean;
    host: string;
    port: string;
    user: string;
    auth: "password" | "key";
    password: string;
    keyPath: string;
    passphrase: string;
    keepSecret: boolean;
    /** A host key approved in this form, and whether to write it to known_hosts. */
    acceptFingerprint?: string;
    rememberHost: boolean;
  };
  tls: { mode: TlsMode; caFile: string };
}

const emptyDraft = (colour: Conn["colour"]): Draft => ({
  name: "",
  environment: "local",
  colour,
  colourPicked: false,
  host: "",
  port: "",
  database: "",
  user: "",
  password: "",
  keepPassword: true,
  filePath: "",
  ssh: { enabled: false, host: "", port: "", user: "", auth: "password", password: "", keyPath: "", passphrase: "", keepSecret: true, rememberHost: false },
  tls: { mode: "off", caFile: "" },
});

function draftOf(conn: Conn): Draft {
  const { config } = conn;
  const ssh = config.ssh;
  return {
    ...emptyDraft(conn.colour),
    name: conn.name,
    environment: conn.environment,
    colourPicked: true,
    host: config.host ?? "",
    port: config.port ? String(config.port) : "",
    database: conn.driver.capabilities.file_based ? "" : (config.database ?? ""),
    user: config.user ?? "",
    keepPassword: conn.secretPolicy !== "ask",
    filePath: conn.driver.capabilities.file_based ? (config.database ?? "") : "",
    ssh: {
      enabled: !!ssh,
      host: ssh?.host ?? "",
      port: ssh?.port ? String(ssh.port) : "",
      user: ssh?.user ?? "",
      auth: ssh?.auth.method === "key_file" ? "key" : "password",
      password: "",
      keyPath: ssh?.auth.method === "key_file" ? ssh.auth.path : "",
      passphrase: "",
      keepSecret: conn.secretPolicy !== "ask",
      acceptFingerprint: ssh?.accept_fingerprint,
      rememberHost: false,
    },
    tls: { mode: config.tls?.mode ?? "off", caFile: config.tls?.ca_file ?? "" },
  };
}

function configOf(driverId: string, fileBased: boolean, d: Draft, forTest: boolean, defaultPort?: number): ConnectionConfig {
  if (fileBased) return { driver: driverId, database: d.filePath.trim() };
  return {
    driver: driverId,
    host: d.host.trim(),
    port: d.port ? Number(d.port) : defaultPort,
    user: d.user.trim() || undefined,
    password: d.password || undefined,
    database: d.database.trim() || undefined,
    tls: { mode: d.tls.mode, ca_file: d.tls.mode === "verify" && d.tls.caFile ? d.tls.caFile : undefined },
    ssh: d.ssh.enabled
      ? {
          host: d.ssh.host.trim(),
          port: d.ssh.port ? Number(d.ssh.port) : undefined,
          user: d.ssh.user.trim(),
          auth:
            d.ssh.auth === "password"
              ? { method: "password", password: d.ssh.password }
              : { method: "key_file", path: d.ssh.keyPath.trim(), passphrase: d.ssh.passphrase || undefined },
          accept_fingerprint: d.ssh.acceptFingerprint,
          // Writing to known_hosts happens once, on the attempt the user approved.
          remember_host: forTest && d.ssh.rememberHost,
        }
      : undefined,
  };
}

type TestState =
  | { state: "idle" }
  | { state: "running"; since: number }
  | { state: "ok"; version: string | null; latency: number }
  | { state: "failed"; kind: string; text: string }
  | { state: "timeout" };

const TABS = [msg("General"), msg("SSH tunnel"), msg("TLS"), msg("Advanced")] as const;
const COLOURS: [Conn["colour"], string][] = [[1, msg("Amber")], [2, msg("Violet")], [3, msg("Green")], [4, msg("Steel")]];
const copy = (text: string) => void navigator.clipboard?.writeText(text);

function BrowseButton({ onPick, disabled }: { onPick(path: string): void; disabled?: boolean }) {
  return (
    <Button
      disabled={disabled || !inTauri}
      title={inTauri ? undefined : t("Available in the desktop app")}
      onClick={async () => {
        const picked = await openFile({ multiple: false, directory: false });
        if (typeof picked === "string") onPick(picked);
      }}
    >
      {t("Browse…")}
    </Button>
  );
}

function SecretInput({ value, onChange, label, unchanged, disabled }: { value: string; onChange(value: string): void; label: string; unchanged: boolean; disabled?: boolean }) {
  const [shown, setShown] = useState(false);
  // A saved secret is never loaded: until the user types, the field only says it exists.
  const untouched = unchanged && value === "";
  return (
    <TextInput
      mono
      type={shown && !untouched ? "text" : "password"}
      aria-label={label}
      value={value}
      disabled={disabled}
      onChange={(e) => onChange(e.target.value)}
      placeholder={untouched ? t("Saved · unchanged — type to replace") : undefined}
      before={untouched ? <Lock size={13} className="text-text-3" /> : undefined}
      after={
        untouched ? undefined : (
          <button type="button" tabIndex={-1} title={shown ? t("Hide") : t("Show")} aria-label={shown ? t("Hide") : t("Show")} className="text-text-2 hover:text-text" onClick={() => setShown(!shown)}>
            {shown ? <EyeOff size={14} /> : <Eye size={14} />}
          </button>
        )
      }
    />
  );
}

export function ConnectionForm({ driverId, engine, editId, initial }: { driverId: string; engine?: string; editId?: string; initial?: unknown }) {
  const drivers = useApp((s) => s.drivers);
  const connectionCount = useApp((s) => s.connections.length);
  const driver = drivers.find((d) => d.id === driverId);
  const editing = editId ? findConn(editId) : null;
  const [draft, setDraftState] = useState<Draft>(
    () => (initial as Draft | undefined) ?? (editing ? draftOf(editing) : emptyDraft((((connectionCount % 4) + 1) as Conn["colour"]))),
  );
  const [tab, setTab] = useState<(typeof TABS)[number]>("General");
  const [dirty, setDirty] = useState(false);
  const [showErrors, setShowErrors] = useState(false);
  const [test, setTest] = useState<TestState>({ state: "idle" });
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const [hostPrompt, setHostPrompt] = useState<UnknownHostKey | null>(null);
  const [, tick] = useState(0);
  // Each test gets a number; a result for an older number is ignored (cancelled or superseded).
  const testRun = useRef(0);

  useEffect(() => {
    if (test.state !== "running") return;
    const timer = setInterval(() => tick((n) => n + 1), 250);
    return () => clearInterval(timer);
  }, [test.state]);

  if (!driver) return null;
  const type = typeOf(engine ?? editing?.engine, driver.id);
  // CockroachDB listens on its own port although it uses the PostgreSQL driver.
  const defaultPort = type.port ?? driver.default_port ?? undefined;
  const fileBased = driver.capabilities.file_based;
  const production = draft.environment === "production";

  // Any edit clears the previous test result.
  const update = (patch: Partial<Draft>) => {
    setDraftState((d) => ({ ...d, ...patch }));
    setDirty(true);
    setSaveError(null);
    testRun.current++;
    setTest({ state: "idle" });
  };
  const updateSsh = (patch: Partial<Draft["ssh"]>) => update({ ssh: { ...draft.ssh, ...patch } });
  const text = (key: "name" | "host" | "port" | "database" | "user" | "filePath") => ({
    value: draft[key],
    onChange: (e: React.ChangeEvent<HTMLInputElement>) => update({ [key]: e.target.value }),
  });

  const errors = {
    name: draft.name.trim() ? undefined : t("Enter a name."),
    host: fileBased || draft.host.trim() ? undefined : t("Enter a host."),
    filePath: !fileBased || draft.filePath.trim() ? undefined : t("Choose a database file."),
    sshHost: !draft.ssh.enabled || draft.ssh.host.trim() ? undefined : t("Enter the SSH host."),
    sshUser: !draft.ssh.enabled || draft.ssh.user.trim() ? undefined : t("Enter the SSH user."),
  };
  const valid = Object.values(errors).every((e) => !e);
  const shown = (key: keyof typeof errors) => (showErrors ? errors[key] : undefined);

  function close() {
    if (dirty) setConfirmDiscard(true);
    else openDialog(null);
  }

  async function runTest(d = draft) {
    if (!valid) {
      setShowErrors(true);
      return;
    }
    const run = ++testRun.current;
    setTest({ state: "running", since: Date.now() });
    try {
      const result = await api.testConnection(configOf(driver!.id, fileBased, d, true, defaultPort), editId);
      if (run === testRun.current) setTest({ state: "ok", version: result.version, latency: result.latency_ms });
    } catch (e) {
      if (run !== testRun.current) return;
      const unknown = unknownHostKey(e);
      const changed = hostKeyChanged(e);
      if (unknown) {
        setTest({ state: "idle" });
        setHostPrompt(unknown);
      } else if (errorKind(e) === "timeout") setTest({ state: "timeout" });
      else if (changed) setTest({ state: "failed", kind: "host_key_changed", text: t("The SSH host key of {host} has changed (known_hosts line {line}). Received {fingerprint}.", { host: changed.host, line: changed.line ?? "?", fingerprint: changed.received }) });
      else setTest({ state: "failed", kind: errorKind(e) ?? "error", text: errorMessage(e) });
    }
  }

  async function save() {
    if (!valid) {
      setShowErrors(true);
      return;
    }
    setSaving(true);
    try {
      const id = await saveConnection(
        editId ?? "",
        { name: draft.name.trim(), engine: type.code, environment: draft.environment, colour: draft.colour, config: configOf(driver!.id, fileBased, draft, false, defaultPort) },
        { keepPassword: draft.keepPassword, keepSshSecret: draft.ssh.keepSecret, unchangedIfEmpty: !!editId },
      );
      openDialog(null);
      // A new connection is opened right away; an edited one keeps its current session.
      if (!editId) void connect(id);
    } catch (e) {
      setSaveError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  }

  const elapsed = test.state === "running" ? ((Date.now() - test.since) / 1000).toFixed(1) : "";
  const target = fileBased ? draft.filePath : `${draft.host}:${draft.port || defaultPort || ""}`;
  const strip: ReactNode =
    test.state === "running" ? (
      <Strip kind="neutral">
        <div className="flex items-center gap-s3">
          <i className="test-bar" />
          <span>{t("Testing connection…")}</span>
          <span className="font-mono text-[12px] text-text-2">{elapsed} s</span>
          <button type="button" className={`ml-auto ${linkButton}`} onClick={() => { testRun.current++; setTest({ state: "idle" }); }}>
            {t("Cancel test")}
          </button>
        </div>
      </Strip>
    ) : test.state === "ok" ? (
      <Strip kind="success" icon={<CheckSquare size={14} />}>
        <div className="flex items-center gap-s3">
          <span className="font-medium">{t("Connection successful")}</span>
          <span className="ml-auto font-mono text-[12px] text-text-2">
            {[test.version, `${test.latency} ms`].filter(Boolean).join(" · ")}
          </span>
        </div>
      </Strip>
    ) : test.state === "failed" ? (
      <Strip kind="error" icon={<TriangleAlert size={14} />}>
        <div className="flex items-center gap-s3">
          <span className="font-medium">{t("Connection failed")}</span>
          <span className="font-mono text-[12px] text-text-2">{test.kind}</span>
          <button type="button" className={`ml-auto ${linkButton}`} onClick={() => copy(test.text)}>
            {t("Copy")}
          </button>
        </div>
        <p className="selectable pt-[2px] font-mono text-[12px] break-words whitespace-pre-wrap">{test.text}</p>
      </Strip>
    ) : test.state === "timeout" ? (
      <Strip kind="warning" icon={<Clock size={14} />}>
        <div className="flex items-start gap-s3">
          <span>
            <span className="font-medium">{t("Timed out after 10 s.")}</span>{" "}
            {tx("No response from {address}. Check the host, port, firewall or SSH tunnel.", { address: <span key="address" className="font-mono text-[12px]">{target}</span> })}
          </span>
          <button type="button" className={`ml-auto ${linkButton}`} onClick={() => runTest()}>
            {t("Retry")}
          </button>
        </div>
      </Strip>
    ) : null;

  const disabledTab = (name: (typeof TABS)[number]) =>
    name === "Advanced" ? `${t("Driver options")} · ${t(NOT_BUILT)}` : fileBased && name !== "General" ? t("Not used by {type}", { type: type.name }) : null;

  return (
    <>
      <Dialog
        title={editId ? t("Edit connection") : t("New connection")}
        headerExtra={
          <>
            <TypeBadge code={type.code} />
            <span className="text-[13px]">{type.name}</span>
            {!editId && (
              <button type="button" className={`text-[12.5px] ${linkButton}`} onClick={() => openDialog({ kind: "picker", draft })}>
                {t("Change type")}
              </button>
            )}
          </>
        }
        width={760}
        band={production ? "production" : undefined}
        onClose={close}
        onSubmit={save}
        bodyClassName=""
        beforeFooter={
          (strip || saveError) && (
            <div className="mx-[20px] mb-s3 flex flex-col gap-s2">
              {strip}
              {saveError && (
                <Strip kind="error" icon={<TriangleAlert size={14} />}>
                  <span className="selectable">{saveError}</span>
                </Strip>
              )}
            </div>
          )
        }
        footer={
          <>
            <Button className="mr-auto" disabled={test.state === "running"} onClick={() => runTest()}>
              {t("Test connection")}
            </Button>
            <Button variant="ghost" onClick={close}>
              {t("Cancel")}
            </Button>
            <Button variant="primary" type="submit" disabled={saving}>
              {editId ? t("Save changes") : production ? t("Save Production connection") : t("Save")}
            </Button>
          </>
        }
      >
        <div role="tablist" className="flex h-[34px] gap-s6 border-b border-line px-[20px]">
          {TABS.map((name) => {
            const why = disabledTab(name);
            return (
              <button
                key={name}
                type="button"
                role="tab"
                aria-selected={tab === name}
                disabled={!!why}
                title={why ?? undefined}
                className={`flex items-center gap-[6px] whitespace-nowrap ${tab === name ? "font-medium" : "text-text-2"}`}
                style={tab === name ? { boxShadow: "inset 0 -2px 0 var(--accent)" } : undefined}
                onClick={() => setTab(name)}
              >
                {t(name)}
                {name === "SSH tunnel" && draft.ssh.enabled && (
                  <span className="border border-line px-[3px] py-[1px] font-heading text-[10px] leading-none font-semibold tracking-[.08em] text-text-2">{t("ON")}</span>
                )}
              </button>
            );
          })}
        </div>

        {/* A fixed height, so the dialog does not resize between tabs. */}
        <div className="h-[300px] overflow-y-auto p-[20px]">
          {tab === "General" && (
            <div className="grid grid-cols-2 gap-s4">
              <Field label={t("Name")} error={shown("name")}>
                <TextInput data-autofocus invalid={!!shown("name")} {...text("name")} />
              </Field>
              <div className="flex gap-s4">
                <div className="flex flex-col gap-[4px]">
                  <span className="text-[12px] text-text-2">{t("Environment")}</span>
                  <Segmented
                    label={t("Environment")}
                    value={draft.environment}
                    onChange={(environment) =>
                      // Production takes the amber marker unless the user already chose a colour.
                      update({ environment, ...(environment === "production" && !draft.colourPicked ? { colour: 1 as const } : {}) })
                    }
                    options={[
                      { value: "local", label: t("Local") },
                      { value: "staging", label: t("Staging") },
                      { value: "production", label: t("Production"), warn: true },
                    ]}
                  />
                </div>
                <div className="flex flex-col gap-[4px]">
                  <span className="text-[12px] text-text-2">{t("Color")}</span>
                  <div role="radiogroup" aria-label={t("Color")} className="flex h-[28px] items-center gap-[10px] pl-[4px]">
                    {COLOURS.map(([colour, name]) => (
                      <button
                        key={colour}
                        type="button"
                        role="radio"
                        aria-checked={draft.colour === colour}
                        title={t(name)}
                        aria-label={t(name)}
                        className="grid size-[18px] place-items-center"
                        style={{ background: colourVar(colour), outline: draft.colour === colour ? "1.5px solid var(--text)" : undefined, outlineOffset: 4 }}
                        onClick={() => update({ colour, colourPicked: true })}
                      >
                        {draft.colour === colour && <Check size={12} strokeWidth={2.5} className="text-bg" />}
                      </button>
                    ))}
                  </div>
                </div>
              </div>

              {fileBased ? (
                <Field
                  className="col-span-2"
                  label={t("Database file")}
                  error={shown("filePath")}
                  helper={t("File-based databases open a local file. There is no server, user or password.")}
                >
                  <div className="flex gap-s2">
                    <TextInput mono className="flex-1" invalid={!!shown("filePath")} {...text("filePath")} />
                    <BrowseButton onPick={(filePath) => update({ filePath })} />
                  </div>
                </Field>
              ) : (
                <>
                  <div className="flex gap-s2">
                    <Field label={t("Host")} className="flex-1" error={shown("host")}>
                      <TextInput mono invalid={!!shown("host")} {...text("host")} />
                    </Field>
                    <Field label={t("Port")} className="w-[84px]">
                      <TextInput mono inputMode="numeric" placeholder={String(defaultPort ?? "")} {...text("port")} />
                    </Field>
                  </div>
                  <Field label={t("Database")}>
                    <TextInput mono {...text("database")} />
                  </Field>
                  <Field label={t("User")}>
                    <TextInput mono {...text("user")} />
                  </Field>
                  <div className="flex flex-col gap-s2">
                    <Field label={t("Password")}>
                      <SecretInput label={t("Password")} value={draft.password} onChange={(password) => update({ password })} unchanged={!!editing?.requires.password} />
                    </Field>
                    <Checkbox checked={draft.keepPassword} onChange={(keepPassword) => update({ keepPassword })}>
                      {t("Save in system keychain")}
                    </Checkbox>
                  </div>
                </>
              )}

              {production && (
                <Strip kind="warning" icon={<TriangleAlert size={14} />} className="col-span-2">
                  <span className="font-medium">{t("Production.")}</span>{" "}{t("A warning stays visible on every tab of this connection, and DELETE, DROP, TRUNCATE or UPDATE without WHERE ask for confirmation.")}
                </Strip>
              )}
            </div>
          )}

          {tab === "SSH tunnel" && (
            <div className="flex flex-col gap-s4">
              <Switch checked={draft.ssh.enabled} onChange={(enabled) => updateSsh({ enabled })}>
                {t("Use SSH tunnel")}
              </Switch>
              <fieldset disabled={!draft.ssh.enabled} aria-disabled={!draft.ssh.enabled} className="grid grid-cols-2 gap-s4">
                <div className="flex gap-s2">
                  <Field label={t("SSH host")} className="flex-1" error={shown("sshHost")}>
                    <TextInput mono invalid={!!shown("sshHost")} value={draft.ssh.host} onChange={(e) => updateSsh({ host: e.target.value, acceptFingerprint: undefined })} />
                  </Field>
                  <Field label={t("Port")} className="w-[84px]">
                    <TextInput mono inputMode="numeric" placeholder="22" value={draft.ssh.port} onChange={(e) => updateSsh({ port: e.target.value })} />
                  </Field>
                </div>
                <Field label={t("SSH user")} error={shown("sshUser")}>
                  <TextInput mono invalid={!!shown("sshUser")} value={draft.ssh.user} onChange={(e) => updateSsh({ user: e.target.value })} />
                </Field>
                <div className="col-span-2 flex flex-col gap-[4px]">
                  <span className="text-[12px] text-text-2">{t("Authentication")}</span>
                  <Segmented
                    label={t("Authentication")}
                    value={draft.ssh.auth}
                    disabled={!draft.ssh.enabled}
                    onChange={(auth) => updateSsh({ auth })}
                    options={[
                      { value: "password", label: t("Password") },
                      { value: "key", label: t("Private key") },
                    ]}
                  />
                </div>
                {draft.ssh.auth === "key" ? (
                  <>
                    <Field label={t("Private key file")} helper={t("The host key is checked against ~/.ssh/known_hosts.")}>
                      <div className="flex gap-s2">
                        <TextInput mono className="flex-1" value={draft.ssh.keyPath} onChange={(e) => updateSsh({ keyPath: e.target.value })} />
                        <BrowseButton disabled={!draft.ssh.enabled} onPick={(keyPath) => updateSsh({ keyPath })} />
                      </div>
                    </Field>
                    <div className="flex flex-col gap-s2">
                      <Field label={t("Key passphrase")}>
                        <SecretInput label={t("Key passphrase")} value={draft.ssh.passphrase} onChange={(passphrase) => updateSsh({ passphrase })} unchanged={!!editing?.requires.ssh_passphrase} disabled={!draft.ssh.enabled} />
                      </Field>
                      <Checkbox checked={draft.ssh.keepSecret} disabled={!draft.ssh.enabled} onChange={(keepSecret) => updateSsh({ keepSecret })}>
                        {t("Save in system keychain")}
                      </Checkbox>
                    </div>
                  </>
                ) : (
                  <div className="flex flex-col gap-s2">
                    <Field label={t("SSH password")} helper={t("The host key is checked against ~/.ssh/known_hosts.")}>
                      <SecretInput label={t("SSH password")} value={draft.ssh.password} onChange={(password) => updateSsh({ password })} unchanged={!!editing?.requires.ssh_password} disabled={!draft.ssh.enabled} />
                    </Field>
                    <Checkbox checked={draft.ssh.keepSecret} disabled={!draft.ssh.enabled} onChange={(keepSecret) => updateSsh({ keepSecret })}>
                      {t("Save in system keychain")}
                    </Checkbox>
                  </div>
                )}
              </fieldset>
            </div>
          )}

          {tab === "TLS" && (
            <div className="flex flex-col gap-s3">
              <div role="radiogroup" aria-label={t("TLS mode")}>
                <Radio title={t("Off")} checked={draft.tls.mode === "off"} onChange={() => update({ tls: { ...draft.tls, mode: "off" } })}>
                  {t("Traffic is not encrypted.")}
                </Radio>
                <Radio title={t("Verify certificate")} checked={draft.tls.mode === "verify"} onChange={() => update({ tls: { ...draft.tls, mode: "verify" } })}>
                  {t("Encrypted, and the server certificate is checked against a CA.")}
                </Radio>
                <Radio title={t("Encrypt only")} checked={draft.tls.mode === "insecure"} onChange={() => update({ tls: { ...draft.tls, mode: "insecure" } })}>
                  {t("Encrypted, but the server's identity is not checked.")}
                </Radio>
              </div>
              {draft.tls.mode === "insecure" && (
                <Strip kind="warning" icon={<TriangleAlert size={14} />}>
                  <span className="font-medium">{t("Encrypt only does not stop impersonation.")}</span>{" "}{t("Anyone who can intercept this connection can pose as the server. Use Verify certificate outside a trusted network.")}
                </Strip>
              )}
              <fieldset disabled={draft.tls.mode !== "verify"} aria-disabled={draft.tls.mode !== "verify"}>
                <Field label={t("CA certificate file")} helper={t("Optional. Public certificate authorities are trusted without it.")}>
                  <div className="flex gap-s2">
                    <TextInput mono className="flex-1" value={draft.tls.caFile} onChange={(e) => update({ tls: { ...draft.tls, caFile: e.target.value } })} />
                    <BrowseButton disabled={draft.tls.mode !== "verify"} onPick={(caFile) => update({ tls: { ...draft.tls, caFile } })} />
                  </div>
                </Field>
              </fieldset>
            </div>
          )}
        </div>
      </Dialog>

      {hostPrompt && (
        <UnknownHostDialog
          nested
          host={draft.ssh.host}
          hostKey={hostPrompt}
          onCancel={() => setHostPrompt(null)}
          onTrust={(rememberHost) => {
            const approved = { ...draft, ssh: { ...draft.ssh, acceptFingerprint: hostPrompt.fingerprint, rememberHost } };
            setDraftState(approved);
            setDirty(true);
            setHostPrompt(null);
            void runTest(approved);
          }}
        />
      )}

      {/* Not drawn in the design: the remove confirmation's layout with a DISCARD button. */}
      {confirmDiscard && (
        <Dialog
          alert
          nested
          title={t("Discard changes?")}
          width={460}
          offset={120}
          onClose={() => setConfirmDiscard(false)}
          footer={
            <>
              <Button data-autofocus onClick={() => setConfirmDiscard(false)}>
                {t("Keep editing")}
              </Button>
              <Button variant="danger" onClick={() => openDialog(null)}>
                {t("Discard")}
              </Button>
            </>
          }
        >
          <p>{t("What you entered for this connection has not been saved.")}</p>
        </Dialog>
      )}
    </>
  );
}
