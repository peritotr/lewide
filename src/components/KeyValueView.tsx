// A key-value tab (K01–K05): the keys of one database on the left, found with
// SCAN and never KEYS, and the selected key's value on the right. Changes to a
// hash, list, set or sorted set run at once, one command each; a string is
// saved when the user asks. On a Production connection every write asks first.
import { Check, Copy, Minus, Plus, RefreshCw, TriangleAlert, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { api, base64ToHex, compactCount, errorMessage, type KeyCommand, type KeyDetails, type KeyEntry, type KeyRange, type KeyspaceInfo, type Value } from "../lib/api";
import { describeCommand, detectMode, formatBytes, formatTtl, jsonError, parseTtl, splitKey, typeBadge, valueText } from "../lib/kv";
import { updateKeys, type Conn, type KeysView, type Tab } from "../store";
import { t, tn } from "../i18n";
import { Button, Dialog, Field, Segmented, Strip, TextInput } from "./dialog";
import { TabBar } from "./TabBar";
import { LoadError, Loading } from "./TableView";

const ghost = "flex h-[26px] flex-none items-center gap-[6px] px-s2 text-[12.5px] whitespace-nowrap enabled:hover:bg-hover disabled:cursor-not-allowed disabled:opacity-45";
const outline = "flex h-[24px] flex-none items-center gap-[6px] border border-line px-s2 text-[12px] whitespace-nowrap enabled:hover:bg-hover disabled:cursor-not-allowed disabled:opacity-45";
const head = "flex h-row flex-none items-center border-b border-line bg-bg font-mono text-[11px] tracking-[.04em] text-text-2 uppercase";
const MOD = /Mac/i.test(navigator.userAgent) ? "⌘" : "Ctrl+";
const SCAN_COUNT = 500;
const PAGE = 100;
const LARGE = 1024 * 1024;
const KINDS = ["string", "hash", "list", "set", "zset"] as const;

/** A question with a few fields: rename, a new field, a time to live, or a yes before a write. */
interface Ask {
  title: string;
  message?: ReactNode;
  fields: { label: string; initial?: string; placeholder?: string; helper?: string }[];
  submit: string;
  danger?: boolean;
  check?(values: string[]): string | null;
  onSubmit(values: string[]): void;
}

type Write = (command: KeyCommand, then?: () => void) => void;

const Badge = ({ kind }: { kind: string }) => (
  <span className="grid h-[16px] w-[38px] flex-none place-items-center border border-line font-mono text-[10px] leading-none font-medium">{typeBadge(kind)}</span>
);

const base64Of = (value: Value) => (value.type === "bytes" ? value.value : "");
const show = (value: Value) => valueText(value) ?? `\\x${base64ToHex(base64Of(value), 64)}`;

export function KeyValueView({ tab, view, conn }: { tab: Tab; view: KeysView; conn: Conn | null }) {
  const production = conn?.environment === "production";
  const session = conn?.session ?? null;
  const { db } = view;

  const [info, setInfo] = useState<KeyspaceInfo | null>(null);
  const [keys, setKeys] = useState<KeyEntry[]>([]);
  const [cursor, setCursor] = useState("0");
  const [scanned, setScanned] = useState(0);
  const [scanning, setScanning] = useState(false);
  const [listError, setListError] = useState<string | null>(null);
  const [details, setDetails] = useState<KeyDetails | null>(null);
  const [keyError, setKeyError] = useState<string | null>(null);
  const [loadingKey, setLoadingKey] = useState(false);
  const [range, setRange] = useState<KeyRange>({});
  const [toast, setToast] = useState<{ text: string; error: boolean } | null>(null);
  const [ask, setAsk] = useState<Ask | null>(null);
  const [adding, setAdding] = useState(false);
  // Replies to an older question are dropped.
  const scanRun = useRef(0);
  const readRun = useRef(0);

  const match = useMemo(() => {
    const text = view.pattern.trim();
    if (!text) return "*";
    return /[*?[\]]/.test(text) ? text : `*${text}*`;
  }, [view.pattern]);

  const loadInfo = useCallback(() => {
    if (session === null) return;
    api.kvKeyspace(session, db).then(setInfo, () => setInfo(null));
  }, [session, db]);

  const scan = useCallback(
    async (from: string, known: KeyEntry[], scannedBefore: number) => {
      if (session === null) return;
      const run = ++scanRun.current;
      setScanning(true);
      setListError(null);
      let next = from;
      let found = known;
      let count = scannedBefore;
      try {
        // A pattern that matches little would otherwise need many clicks.
        for (let round = 0; round < 4; round++) {
          const page = await api.kvScan(session, db, match, view.kind, next, SCAN_COUNT);
          if (run !== scanRun.current) return;
          const seen = new Set(found.map((k) => k.key));
          found = [...found, ...page.keys.filter((k) => !seen.has(k.key))];
          next = page.cursor;
          count += SCAN_COUNT;
          if (next === "0" || found.length - known.length >= 50) break;
        }
        setKeys(found);
        setCursor(next);
        setScanned(count);
      } catch (e) {
        if (run === scanRun.current) setListError(errorMessage(e));
      } finally {
        if (run === scanRun.current) setScanning(false);
      }
    },
    [session, db, match, view.kind],
  );

  // The first scan, and a new one shortly after the pattern or type changes.
  useEffect(() => {
    const timer = setTimeout(() => void scan("0", [], 0), 250);
    return () => clearTimeout(timer);
  }, [scan]);
  useEffect(loadInfo, [loadInfo]);

  const read = useCallback(
    async (key: string, next: KeyRange = {}) => {
      if (session === null) return;
      const run = ++readRun.current;
      setLoadingKey(true);
      setKeyError(null);
      setRange(next);
      try {
        const loaded = await api.kvRead(session, db, key, next);
        if (run !== readRun.current) return;
        setDetails(loaded);
        setKeys((list) => list.map((k) => (k.key === key ? { ...k, kind: loaded.kind, ttl_ms: loaded.ttl_ms } : k)));
      } catch (e) {
        if (run !== readRun.current) return;
        setDetails(null);
        setKeyError(errorMessage(e));
      } finally {
        if (run === readRun.current) setLoadingKey(false);
      }
    },
    [session, db],
  );

  const selected = view.selected;
  useEffect(() => {
    if (selected) void read(selected);
    else setDetails(null);
  }, [selected, read]);

  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), toast.error ? 8000 : 4000);
    return () => clearTimeout(timer);
  }, [toast]);

  const refresh = () => {
    loadInfo();
    void scan("0", [], 0);
    if (selected) void read(selected, range);
  };

  const write: Write = (command, then) => {
    if (session === null) return;
    const go = async () => {
      try {
        const ran = await api.kvWrite(session, db, command);
        setToast({ text: ran, error: false });
        if (command.op === "delete") {
          setKeys((list) => list.filter((k) => k.key !== command.key));
          updateKeys(tab.id, { selected: null });
          loadInfo();
        } else if (command.op === "rename") {
          setKeys((list) => list.map((k) => (k.key === command.key ? { ...k, key: command.to } : k)));
          updateKeys(tab.id, { selected: command.to });
        } else {
          void read(command.key, range);
        }
        then?.();
      } catch (e) {
        setToast({ text: errorMessage(e), error: true });
      }
    };
    if (!production) return void go();
    setAsk({
      title: t("Run on Production?"),
      message: (
        <>
          <p className="pb-s2">{t("This command runs at once on {name} and cannot be undone here.", { name: conn?.name ?? "" })}</p>
          <pre className="selectable overflow-auto border border-line bg-panel px-s3 py-s2 font-mono text-[12.5px] whitespace-pre-wrap">{describeCommand(command)}</pre>
        </>
      ),
      fields: [],
      submit: t("Run"),
      danger: true,
      onSubmit: () => void go(),
    });
  };

  const total = info?.keys ?? null;
  const complete = cursor === "0" && !scanning;
  const scannedShown = complete && total !== null ? total : total !== null ? Math.min(scanned, total) : scanned;
  const databases = useMemo(() => {
    const numbers = new Set<number>([db, ...(info?.databases.map(([n]) => n) ?? [])]);
    return [...numbers].sort((a, b) => a - b);
  }, [db, info]);

  if (!conn || session === null) {
    return (
      <main className="grid min-h-0 min-w-0 grid-rows-[34px_minmax(0,1fr)] bg-panel">
        <TabBar />
        <div className="grid place-items-center">
          <p className="text-text-2">{t("This tab's connection is not open. Double-click it in the sidebar.")}</p>
        </div>
      </main>
    );
  }

  return (
    <main className="relative grid min-h-0 min-w-0 grid-cols-[minmax(0,1fr)] bg-panel" style={{ gridTemplateRows: `34px ${production ? "4px" : "0px"} 36px minmax(0,1fr)` }}>
      <TabBar />
      <div title={production ? t("Production connection") : undefined} className={production ? "production-band" : ""} />

      <div className="flex items-center gap-[10.2px] border-b border-line px-[10.2px] whitespace-nowrap">
        <select aria-label={t("Database")} className="h-[26px] border border-line bg-panel px-[6px] font-mono text-[12px]" value={db} onChange={(e) => updateKeys(tab.id, { db: Number(e.target.value), selected: null })}>
          {databases.map((n) => (
            <option key={n} value={n}>
              db{n}
            </option>
          ))}
        </select>
        {info && (
          <span className="font-mono text-[12px] text-text-2">
            {tn(info.keys, "{count} key", "{count} keys")}
            {info.memory_bytes !== null && ` · ${t("{size} used", { size: formatBytes(info.memory_bytes) })}`}
          </span>
        )}
        <div className="ml-auto flex items-center gap-[2px]">
          <button type="button" className={ghost} onClick={() => setAdding(true)}>
            <Plus size={13} />
            {t("Add key")}
          </button>
          <button type="button" className={ghost} disabled={scanning} onClick={refresh}>
            <RefreshCw size={13} />
            {t("Refresh")}
          </button>
        </div>
      </div>

      <div className="grid min-h-0 grid-cols-[420px_minmax(0,1fr)]">
        <section className="flex min-h-0 flex-col border-r border-line" aria-label={t("Keys")}>
          <div className="flex flex-none flex-col gap-s2 border-b border-line bg-bg p-s2">
            <TextInput mono autoFocus placeholder={t("glob pattern")} aria-label={t("Key pattern")} value={view.pattern} onChange={(e) => updateKeys(tab.id, { pattern: e.target.value })} />
            <div role="radiogroup" aria-label={t("Type")} className="flex h-[22px] w-fit border border-line font-mono text-[11px]">
              {[null, ...KINDS].map((kind, i) => (
                <button
                  key={kind ?? "all"}
                  type="button"
                  role="radio"
                  aria-checked={view.kind === kind}
                  className={`px-[8px] ${i > 0 ? "border-l border-line" : ""} ${view.kind === kind ? "bg-accent-fill font-medium text-on-accent" : "hover:bg-hover"}`}
                  onClick={() => updateKeys(tab.id, { kind })}
                >
                  {kind ? typeBadge(kind) : t("All")}
                </button>
              ))}
            </div>
          </div>
          <div className={`${head} gap-s2 px-s2`}>
            <span className="w-[38px]">{t("Type")}</span>
            <span className="flex-1">{t("Key")}</span>
            <span>{t("TTL")}</span>
          </div>
          <div className="min-h-0 flex-1 overflow-auto" role="listbox" aria-label={t("Keys")}>
            {listError ? (
              <LoadError message={listError} conn={conn} onRetry={() => void scan("0", [], 0)} />
            ) : keys.length === 0 && !scanning ? (
              <p className="px-s3 py-s4 text-[12.5px] text-text-2">{complete ? t("No keys match.") : t("No matching keys in the part scanned so far.")}</p>
            ) : (
              keys.map((entry) => {
                const [prefix, name] = splitKey(entry.key);
                const on = entry.key === selected;
                return (
                  <div
                    key={entry.key}
                    role="option"
                    aria-selected={on}
                    tabIndex={0}
                    className={`flex h-row cursor-pointer items-center gap-s2 px-s2 ${on ? "bg-accent-tint" : "hover:bg-hover"}`}
                    style={on ? { boxShadow: "inset 2px 0 0 var(--accent)" } : undefined}
                    onClick={() => updateKeys(tab.id, { selected: entry.key })}
                    onKeyDown={(e) => e.key === "Enter" && updateKeys(tab.id, { selected: entry.key })}
                  >
                    <Badge kind={entry.kind} />
                    <span className="min-w-0 flex-1 truncate font-mono text-[12px]" title={entry.key}>
                      <span className="text-text-3">{prefix}</span>
                      {name}
                    </span>
                    <span className={`flex-none font-mono text-[11px] ${entry.ttl_ms === null ? "text-text-3" : "text-text-2"}`}>{entry.ttl_ms === null ? t("no expiry") : formatTtl(entry.ttl_ms)}</span>
                  </div>
                );
              })
            )}
          </div>
          <footer className="flex h-[30px] flex-none items-center gap-s2 border-t border-line bg-bg px-s2 text-[12px] text-text-2">
            <span className="min-w-0 flex-1 truncate font-mono text-[11.5px]">
              {scanning
                ? t("Scanning…")
                : total !== null
                  ? t("Scanned {scanned} of about {total} · {count} match", { scanned: compactCount(scannedShown), total: compactCount(total), count: keys.length })
                  : t("Scanned {scanned} · {count} match", { scanned: compactCount(scannedShown), count: keys.length })}
            </span>
            <button type="button" className={outline} disabled={scanning || cursor === "0"} onClick={() => void scan(cursor, keys, scanned)}>
              {t("Scan more")}
            </button>
          </footer>
        </section>

        <section className="flex min-h-0 min-w-0 flex-col" aria-label={t("Value")}>
          {!selected ? (
            <div className="grid flex-1 place-items-center">
              <p className="text-text-2">{t("Select a key to see its value.")}</p>
            </div>
          ) : keyError ? (
            <LoadError message={keyError} conn={conn} onRetry={() => void read(selected)} />
          ) : !details || details.key !== selected ? (
            <Loading text={t("Reading {key}…", { key: selected })} />
          ) : (
            <KeyEditor key={details.key} details={details} busy={loadingKey} range={range} write={write} read={(next) => void read(details.key, next)} ask={setAsk} toast={(text) => setToast({ text, error: false })} />
          )}
        </section>
      </div>

      {toast && (
        <div role={toast.error ? "alert" : "status"} className={`absolute right-s4 bottom-s4 z-40 flex max-w-[560px] items-start gap-s2 border bg-bg px-s3 py-s2 ${toast.error ? "border-danger" : "border-line"}`} style={{ boxShadow: "var(--shadow-lg)" }}>
          {toast.error ? <TriangleAlert size={14} className="mt-[2px] flex-none text-danger" /> : <Check size={14} className="mt-[2px] flex-none text-ok" />}
          <span className="selectable min-w-0 font-mono text-[12px] break-all">{toast.text}</span>
          <button type="button" aria-label={t("Close")} className="flex-none text-text-3 hover:text-text" onClick={() => setToast(null)}>
            <X size={13} />
          </button>
        </div>
      )}
      {ask && <AskDialog ask={ask} production={production} onClose={() => setAsk(null)} />}
      {adding && (
        <AddKeyDialog
          production={production}
          exists={async (key) => api.kvRead(session, db, key, { limit: 1 }).then(() => true, () => false)}
          onClose={() => setAdding(false)}
          onAdd={(command) => {
            setAdding(false);
            write(command, () => {
              setKeys((list) => (list.some((k) => k.key === command.key) ? list : [{ key: command.key, kind: KIND_OF[command.op] ?? "string", ttl_ms: null }, ...list]));
              updateKeys(tab.id, { selected: command.key });
              loadInfo();
            });
          }}
        />
      )}
    </main>
  );
}

const KIND_OF: Partial<Record<KeyCommand["op"], string>> = { set_string: "string", hash_set: "hash", list_push: "list", set_add: "set", zset_add: "zset" };

function AskDialog({ ask, production, onClose }: { ask: Ask; production: boolean; onClose(): void }) {
  const [values, setValues] = useState(() => ask.fields.map((f) => f.initial ?? ""));
  const [error, setError] = useState<string | null>(null);
  const submit = () => {
    const problem = ask.check?.(values) ?? null;
    if (problem) return setError(problem);
    onClose();
    ask.onSubmit(values);
  };
  return (
    <Dialog
      title={ask.title}
      width={460}
      alert={ask.fields.length === 0}
      band={production ? "production" : undefined}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t("Cancel")}
          </Button>
          <Button type="submit" variant={ask.danger ? "danger" : "primary"}>
            {ask.submit}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-s3">
        {ask.message && <div className="text-[13px]">{ask.message}</div>}
        {ask.fields.map((field, i) => (
          <Field key={field.label} label={field.label} helper={field.helper} error={i === ask.fields.length - 1 ? (error ?? undefined) : undefined}>
            <TextInput
              mono
              data-autofocus={i === 0 ? "" : undefined}
              // A filled-in value (the current name, the current TTL) is selected, so typing replaces it.
              onFocus={(e) => e.currentTarget.select()}
              placeholder={field.placeholder}
              value={values[i]}
              invalid={!!error && i === ask.fields.length - 1}
              onChange={(e) => {
                setError(null);
                setValues(values.map((v, j) => (j === i ? e.target.value : v)));
              }}
            />
          </Field>
        ))}
      </div>
    </Dialog>
  );
}

// Not drawn in the design: the same dialog as the other questions, with the type first.
function AddKeyDialog({ production, exists, onClose, onAdd }: { production: boolean; exists(key: string): Promise<boolean>; onClose(): void; onAdd(command: KeyCommand): void }) {
  const [kind, setKind] = useState<(typeof KINDS)[number]>("string");
  const [key, setKey] = useState("");
  const [field, setField] = useState("");
  const [score, setScore] = useState("0");
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);

  async function submit() {
    if (!key) return setError(t("Give the key a name."));
    if (kind === "hash" && !field) return setError(t("Give the field a name."));
    if (kind === "zset" && !Number.isFinite(Number(score.trim() || "x"))) return setError(t("The score must be a number."));
    setChecking(true);
    const taken = await exists(key);
    setChecking(false);
    if (taken) return setError(t("A key with this name already exists."));
    onAdd(
      kind === "string"
        ? { op: "set_string", key, value }
        : kind === "hash"
          ? { op: "hash_set", key, field, value }
          : kind === "list"
            ? { op: "list_push", key, value, left: false }
            : kind === "set"
              ? { op: "set_add", key, member: value }
              : { op: "zset_add", key, member: value, score: Number(score) },
    );
  }

  return (
    <Dialog
      title={t("Add key")}
      width={480}
      band={production ? "production" : undefined}
      onClose={onClose}
      onSubmit={() => void submit()}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t("Cancel")}
          </Button>
          <Button type="submit" variant="primary" disabled={checking}>
            {t("Add key")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-s3">
        <Field label={t("Type")}>
          <Segmented<(typeof KINDS)[number]> label={t("Type")} value={kind} onChange={setKind} options={KINDS.map((k) => ({ value: k, label: typeBadge(k) }))} />
        </Field>
        <Field label={t("Key")}>
          <TextInput mono data-autofocus="" value={key} placeholder="user:1001" onChange={(e) => (setError(null), setKey(e.target.value))} />
        </Field>
        {kind === "hash" && (
          <Field label={t("Field")}>
            <TextInput mono value={field} onChange={(e) => (setError(null), setField(e.target.value))} />
          </Field>
        )}
        {kind === "zset" && (
          <Field label={t("Score")}>
            <TextInput mono value={score} onChange={(e) => (setError(null), setScore(e.target.value))} />
          </Field>
        )}
        <Field label={kind === "set" || kind === "zset" ? t("Member") : kind === "list" ? t("First item") : t("Value")} error={error ?? undefined}>
          <TextInput mono value={value} onChange={(e) => (setError(null), setValue(e.target.value))} />
        </Field>
      </div>
    </Dialog>
  );
}

interface EditorProps {
  details: KeyDetails;
  busy: boolean;
  range: KeyRange;
  write: Write;
  read(range?: KeyRange): void;
  ask(ask: Ask): void;
  toast(text: string): void;
}

function KeyEditor(props: EditorProps) {
  const { details, write, ask } = props;
  const { key } = details;
  const data = details.data;

  return (
    <>
      <header className="flex flex-none flex-col gap-s2 border-b border-line bg-bg px-s3 py-s2">
        <div className="flex items-center gap-s2">
          <span className="selectable min-w-0 truncate font-mono text-[15px] font-medium" title={key}>
            {key}
          </span>
          <Badge kind={details.kind} />
          <span className="flex-1" />
          <button
            type="button"
            className={outline}
            onClick={() =>
              ask({
                title: t("Rename key"),
                fields: [{ label: t("New name"), initial: key }],
                submit: t("Rename"),
                check: ([to]) => (!to ? t("Give the key a name.") : to === key ? t("That is the name it has.") : null),
                onSubmit: ([to]) => write({ op: "rename", key, to }),
              })
            }
          >
            {t("Rename")}
          </button>
          <button
            type="button"
            className={outline}
            onClick={() => {
              void navigator.clipboard?.writeText(key);
              props.toast(t("Copied {key}", { key }));
            }}
          >
            <Copy size={12} />
            {t("Copy key")}
          </button>
          <button
            type="button"
            className={`${outline} border-danger text-danger`}
            onClick={() =>
              ask({
                title: t("Delete key?"),
                message: tx_delete(key),
                fields: [],
                submit: t("Delete key"),
                danger: true,
                onSubmit: () => write({ op: "delete", key }),
              })
            }
          >
            {t("Delete key")}
          </button>
        </div>
        <div className="flex items-center gap-s3 text-[12px] text-text-2">
          <span>{details.ttl_ms === null ? t("TTL no expiry") : t("Expires in {ttl}", { ttl: formatTtl(details.ttl_ms) })}</span>
          <button
            type="button"
            className="text-accent-text hover:underline"
            onClick={() =>
              ask({
                title: t("Set TTL"),
                fields: [{ label: t("Expires in"), initial: details.ttl_ms === null ? "" : String(Math.max(1, Math.round(details.ttl_ms / 1000))), placeholder: "3600", helper: t("Seconds, or a duration such as 15m, 2h or 3d.") }],
                submit: t("Set TTL"),
                check: ([text]) => (parseTtl(text) === null ? t("Enter seconds or a duration such as 15m.") : null),
                onSubmit: ([text]) => write({ op: "expire", key, seconds: parseTtl(text)! }),
              })
            }
          >
            {t("Set TTL…")}
          </button>
          <button type="button" className="text-accent-text hover:underline disabled:cursor-not-allowed disabled:text-text-3 disabled:no-underline" disabled={details.ttl_ms === null} onClick={() => write({ op: "persist", key })}>
            {t("Persist")}
          </button>
          <span className="flex-1" />
          {details.size_bytes !== null && <span className="font-mono text-[11.5px]">{formatBytes(details.size_bytes)}</span>}
          {details.encoding && <span className="font-mono text-[11.5px]">{details.encoding}</span>}
        </div>
      </header>
      {data.kind === "string" ? (
        <StringEditor {...props} value={data.value} />
      ) : data.kind === "hash" ? (
        <HashEditor {...props} fields={data.fields} total={data.total} />
      ) : data.kind === "list" ? (
        <ListEditor {...props} items={data.items} start={data.start} total={data.total} />
      ) : data.kind === "set" ? (
        <SetEditor {...props} members={data.members} cursor={data.cursor} total={data.total} />
      ) : data.kind === "zset" ? (
        <ZsetEditor {...props} members={data.members} start={data.start} total={data.total} />
      ) : (
        // Stream, JSON module and other types are not drawn in the design.
        <div className="p-s4">
          <Strip kind="neutral">{t("LewIDE cannot show {type} values yet. Read it with its own commands in a query tab.", { type: details.kind })}</Strip>
        </div>
      )}
    </>
  );
}

const tx_delete = (key: string) => (
  <>
    <p className="pb-s2">{t("The key and everything in it is removed. This cannot be undone.")}</p>
    <pre className="selectable overflow-auto border border-line bg-panel px-s3 py-s2 font-mono text-[12.5px]">{key}</pre>
  </>
);

function StringEditor({ details, value, write, busy }: EditorProps & { value: Value }) {
  const original = valueText(value);
  const binary = original === null;
  const [mode, setMode] = useState(() => detectMode(value));
  const [text, setText] = useState(original ?? "");
  const [shown, setShown] = useState((details.size_bytes ?? 0) <= LARGE);
  const gutter = useRef<HTMLDivElement>(null);
  const dirty = !binary && text !== original;
  const invalid = mode === "json" ? jsonError(text) : null;
  const hex = useMemo(() => {
    if (mode !== "hex") return "";
    if (binary) return base64ToHex(base64Of(value), 64 * 1024);
    return [...new TextEncoder().encode(text.slice(0, 64 * 1024))].map((b) => b.toString(16).padStart(2, "0")).join(" ");
  }, [mode, binary, value, text]);
  const lines = useMemo(() => text.split("\n").length, [text]);

  // A reload after a save brings the stored text back.
  useEffect(() => setText(original ?? ""), [original]);

  const save = () => {
    if (dirty && !invalid && mode !== "hex") write({ op: "set_string", key: details.key, value: text });
  };

  return (
    <div
      className="flex min-h-0 flex-1 flex-col"
      onKeyDown={(e) => {
        if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") {
          e.preventDefault();
          save();
        }
      }}
    >
      <div className="flex h-[36px] flex-none items-center gap-s3 border-b border-line px-s3">
        <Segmented<"text" | "json" | "hex">
          label={t("Mode")}
          value={mode}
          onChange={setMode}
          options={[
            { value: "text", label: t("Text"), disabled: binary },
            { value: "json", label: "JSON", disabled: binary },
            { value: "hex", label: t("Hex") },
          ]}
        />
        {mode === "json" &&
          (invalid ? (
            <span className="flex min-w-0 items-center gap-[6px] text-[12px] text-danger" title={invalid}>
              <TriangleAlert size={12} className="flex-none" />
              <span className="truncate">{t("Not valid JSON")}</span>
            </span>
          ) : (
            <span className="flex items-center gap-[6px] text-[12px] text-text-2">
              <i className="size-[8px] bg-ok" />
              {t("Valid JSON")}
            </span>
          ))}
        <span className="flex-1" />
        {mode === "json" && (
          <button type="button" className={outline} disabled={!!invalid} onClick={() => setText(JSON.stringify(JSON.parse(text), null, 2))}>
            {t("Format")}
          </button>
        )}
      </div>
      {!shown ? (
        <div className="flex-1 p-s4">
          <Strip kind="warning" icon={<TriangleAlert size={14} />}>
            {t("This value is {size}. It is not shown until you ask.", { size: formatBytes(details.size_bytes ?? 0) })}{" "}
            <button type="button" className="text-accent-text hover:underline" onClick={() => setShown(true)}>
              {t("Load anyway")}
            </button>
          </Strip>
        </div>
      ) : mode === "hex" ? (
        <pre className="selectable min-h-0 flex-1 overflow-auto p-s3 font-mono text-[12.5px] leading-[20px] break-all whitespace-pre-wrap">{hex}</pre>
      ) : (
        <div className="flex min-h-0 flex-1">
          <div ref={gutter} aria-hidden className="w-[44px] flex-none overflow-hidden border-r border-line bg-bg py-s2 pr-s2 text-right font-mono text-[12.5px] leading-[20px] text-text-3 select-none">
            {Array.from({ length: Math.min(lines, 20000) }, (_, i) => (
              <div key={i}>{i + 1}</div>
            ))}
          </div>
          <textarea
            aria-label={t("Value")}
            spellCheck={false}
            wrap="off"
            className="min-w-0 flex-1 resize-none bg-panel p-s2 font-mono text-[12.5px] leading-[20px] focus:outline-none"
            value={text}
            onChange={(e) => setText(e.target.value)}
            onScroll={(e) => {
              if (gutter.current) gutter.current.scrollTop = e.currentTarget.scrollTop;
            }}
          />
        </div>
      )}
      <footer className="flex h-[44px] flex-none items-center gap-s2 border-t border-line bg-bg px-s3">
        <span className="min-w-0 flex-1 truncate text-[12px] text-text-2">{binary ? t("Binary value · read-only") : t("Saving runs SET … KEEPTTL")}</span>
        <Button variant="ghost" disabled={!dirty} onClick={() => setText(original ?? "")}>
          {t("Revert")}
        </Button>
        <Button variant="primary" disabled={!dirty || !!invalid || mode === "hex" || busy} onClick={save}>
          {t("Save")} {MOD}S
        </Button>
      </footer>
    </div>
  );
}

/** A cell that turns into a field on double-click; Enter writes, Escape leaves it. */
function EditCell({ value, onCommit, className = "" }: { value: Value | string; onCommit(text: string): void; className?: string }) {
  const text = typeof value === "string" ? value : valueText(value);
  const [draft, setDraft] = useState<string | null>(null);
  if (draft !== null) {
    return (
      <input
        autoFocus
        onFocus={(e) => e.currentTarget.select()}
        className={`h-[22px] min-w-0 flex-1 border border-accent bg-panel px-[4px] font-mono text-[12.5px] focus:outline-none ${className}`}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => setDraft(null)}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Escape") setDraft(null);
          if (e.key === "Enter") {
            if (draft !== text) onCommit(draft);
            setDraft(null);
          }
        }}
      />
    );
  }
  return (
    <span
      className={`selectable min-w-0 flex-1 truncate font-mono text-[12.5px] ${className}`}
      title={text === null ? t("Binary value · read-only") : t("Double-click to edit")}
      onDoubleClick={() => text !== null && setDraft(text)}
    >
      {typeof value === "string" ? value : show(value)}
    </span>
  );
}

function Row({ selected, onSelect, children }: { selected: boolean; onSelect(): void; children: ReactNode }) {
  return (
    <div role="row" aria-selected={selected} className={`flex h-row cursor-default items-center gap-s3 border-b border-line px-s3 ${selected ? "bg-accent-tint" : "hover:bg-hover"}`} style={selected ? { boxShadow: "inset 2px 0 0 var(--accent)" } : undefined} onClick={onSelect}>
      {children}
    </div>
  );
}

const Foot = ({ note, children }: { note: string; children: ReactNode }) => (
  <footer className="flex h-[40px] flex-none items-center gap-s2 border-t border-line bg-bg px-s3">
    <span className="min-w-0 flex-1 truncate font-mono text-[11.5px] text-text-2">{note}</span>
    {children}
  </footer>
);

const FootButton = ({ icon, children, ...rest }: { icon?: "plus" | "minus"; children: ReactNode; disabled?: boolean; onClick(): void }) => (
  <button type="button" className={outline} {...rest}>
    {icon === "plus" ? <Plus size={12} /> : icon === "minus" ? <Minus size={12} /> : null}
    {children}
  </button>
);

const needText = (label: string) => (values: string[]) => (values[0] === "" ? label : null);

function HashEditor({ details, fields, total, write, ask }: EditorProps & { fields: [string, Value][]; total: number }) {
  const [selected, setSelected] = useState<string | null>(null);
  const { key } = details;
  return (
    <>
      <div className={`${head} gap-s3 px-s3`}>
        <span className="w-[280px] flex-none">{t("Field")}</span>
        <span className="flex-1">{t("Value")}</span>
      </div>
      <div role="table" className="min-h-0 flex-1 overflow-auto">
        {fields.map(([field, value]) => (
          <Row key={field} selected={selected === field} onSelect={() => setSelected(field)}>
            <span className="selectable w-[280px] flex-none truncate font-mono text-[12.5px]" title={field}>
              {field}
            </span>
            <EditCell value={value} onCommit={(text) => write({ op: "hash_set", key, field, value: text })} />
          </Row>
        ))}
      </div>
      <Foot note={fields.length < total ? t("{shown} of {total} fields shown", { shown: fields.length, total }) : tn(total, "{count} field", "{count} fields")}>
        <FootButton
          icon="plus"
          onClick={() =>
            ask({
              title: t("Add field"),
              fields: [{ label: t("Field") }, { label: t("Value") }],
              submit: t("Add field"),
              check: needText(t("Give the field a name.")),
              onSubmit: ([field, value]) => write({ op: "hash_set", key, field, value }),
            })
          }
        >
          {t("Add field")}
        </FootButton>
        <FootButton icon="minus" disabled={selected === null} onClick={() => selected !== null && write({ op: "hash_delete", key, field: selected }, () => setSelected(null))}>
          {t("Delete field")}
        </FootButton>
      </Foot>
    </>
  );
}

function ListEditor({ details, items, start, total, write, read, ask, range }: EditorProps & { items: Value[]; start: number; total: number }) {
  const [selected, setSelected] = useState<number | null>(null);
  const { key } = details;
  const push = (left: boolean) =>
    ask({
      title: left ? t("Push left") : t("Push right"),
      fields: [{ label: t("Value") }],
      submit: left ? t("Push left") : t("Push right"),
      onSubmit: ([value]) => write({ op: "list_push", key, value, left }),
    });
  const last = start + items.length - 1;
  return (
    <>
      <div className={`${head} gap-s3 px-s3`}>
        <span className="w-[72px] flex-none text-right">{t("Index")}</span>
        <span className="flex-1">{t("Value")}</span>
      </div>
      <div role="table" className="min-h-0 flex-1 overflow-auto">
        {items.map((value, i) => (
          <Row key={start + i} selected={selected === start + i} onSelect={() => setSelected(start + i)}>
            <span className="w-[72px] flex-none text-right font-mono text-[12px] text-text-3">{start + i}</span>
            <EditCell value={value} onCommit={(text) => write({ op: "list_set", key, index: start + i, value: text })} />
          </Row>
        ))}
      </div>
      <Foot note={items.length === 0 ? t("No items") : t("Items {first}–{last} of {total}", { first: start, last, total })}>
        <FootButton disabled={last + 1 >= total} onClick={() => read({ ...range, offset: 0, limit: items.length + PAGE })}>
          {t("Load next {count}", { count: PAGE })}
        </FootButton>
        <FootButton icon="plus" onClick={() => push(true)}>
          {t("Push left")}
        </FootButton>
        <FootButton icon="plus" onClick={() => push(false)}>
          {t("Push right")}
        </FootButton>
        <FootButton icon="minus" disabled={selected === null} onClick={() => selected !== null && write({ op: "list_remove", key, index: selected }, () => setSelected(null))}>
          {t("Remove item")}
        </FootButton>
      </Foot>
    </>
  );
}

function SetEditor({ details, members, cursor, total, write, read, ask, range }: EditorProps & { members: Value[]; cursor: string; total: number }) {
  const [selected, setSelected] = useState<string | null>(null);
  const { key } = details;
  return (
    <>
      <div className={`${head} px-s3`}>
        <span>{t("Member")}</span>
      </div>
      <div role="table" className="min-h-0 flex-1 overflow-auto">
        {members.map((member, i) => {
          const text = valueText(member);
          return (
            <Row key={i} selected={text !== null && selected === text} onSelect={() => setSelected(text)}>
              <span className="selectable min-w-0 flex-1 truncate font-mono text-[12.5px]">{show(member)}</span>
            </Row>
          );
        })}
      </div>
      <Foot note={t("{shown} of {total} members shown · unordered", { shown: members.length, total })}>
        <FootButton disabled={cursor === "0" || members.length >= total} onClick={() => read({ ...range, limit: (range.limit ?? PAGE) + PAGE })}>
          {t("Scan more")}
        </FootButton>
        <FootButton
          icon="plus"
          onClick={() =>
            ask({
              title: t("Add member"),
              fields: [{ label: t("Member") }],
              submit: t("Add member"),
              onSubmit: ([member]) => write({ op: "set_add", key, member }),
            })
          }
        >
          {t("Add member")}
        </FootButton>
        <FootButton icon="minus" disabled={selected === null} onClick={() => selected !== null && write({ op: "set_remove", key, member: selected }, () => setSelected(null))}>
          {t("Remove member")}
        </FootButton>
      </Foot>
    </>
  );
}

function ZsetEditor({ details, members, start, total, write, read, ask, range }: EditorProps & { members: [Value, number][]; start: number; total: number }) {
  const [selected, setSelected] = useState<string | null>(null);
  const { key } = details;
  const descending = range.descending ?? false;
  const score = (text: string) => (Number.isFinite(Number(text.trim() || "x")) ? null : t("The score must be a number."));
  return (
    <>
      <div className={`${head} gap-s3 px-s3`}>
        <span className="w-[56px] flex-none text-right">{t("Rank")}</span>
        <span className="w-[140px] flex-none text-right">{t("Score")}</span>
        <span className="flex-1">{t("Member")}</span>
      </div>
      <div role="table" className="min-h-0 flex-1 overflow-auto">
        {members.map(([member, memberScore], i) => {
          const text = valueText(member);
          return (
            <Row key={i} selected={text !== null && selected === text} onSelect={() => setSelected(text)}>
              <span className="w-[56px] flex-none text-right font-mono text-[12px] text-text-3">{start + i + 1}</span>
              <span className="flex w-[140px] flex-none justify-end">
                <EditCell
                  className="text-right font-medium"
                  value={String(memberScore)}
                  onCommit={(next) => {
                    if (text !== null && score(next) === null) write({ op: "zset_add", key, member: text, score: Number(next) });
                  }}
                />
              </span>
              <span className="selectable min-w-0 flex-1 truncate font-mono text-[12.5px]">{show(member)}</span>
            </Row>
          );
        })}
      </div>
      <Foot note={members.length === 0 ? t("No members") : t("Ranks {first}–{last} of {total}", { first: start + 1, last: start + members.length, total })}>
        <Segmented
          label={t("Order")}
          value={descending ? "high" : "low"}
          onChange={(order) => read({ ...range, descending: order === "high" })}
          options={[
            { value: "high", label: t("High to low") },
            { value: "low", label: t("Low to high") },
          ]}
        />
        <FootButton disabled={start + members.length >= total} onClick={() => read({ ...range, descending, offset: 0, limit: members.length + PAGE })}>
          {t("Load next {count}", { count: PAGE })}
        </FootButton>
        <FootButton
          icon="plus"
          onClick={() =>
            ask({
              title: t("Add member"),
              fields: [{ label: t("Member") }, { label: t("Score"), initial: "0" }],
              submit: t("Add member"),
              check: ([, text]) => score(text),
              onSubmit: ([member, text]) => write({ op: "zset_add", key, member, score: Number(text) }),
            })
          }
        >
          {t("Add member")}
        </FootButton>
        <FootButton icon="minus" disabled={selected === null} onClick={() => selected !== null && write({ op: "zset_remove", key, member: selected }, () => setSelected(null))}>
          {t("Remove member")}
        </FootButton>
      </Foot>
    </>
  );
}
