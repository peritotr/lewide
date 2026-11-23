import { AlignLeft, Check, ChevronDown, Network, Play, Save, Sparkles, Square, StepForward, TriangleAlert, Undo2 } from "lucide-react";
import { togglePanel, useAi } from "../ai-store";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { typeOf } from "../lib/types";
import { cancelRun, connect, endTransaction, saveTabQuery, setTransactionMode, updateTab, useApp, useNamespace, type Conn, type Tab } from "../store";
import { Button, TextInput } from "./dialog";
import { anchorOf, Floating, MenuItem, MenuSeparator, type Anchor } from "./menu";
import { isMac } from "./TitleBar";
import { colourVar, ConnectionMarker, EnvTag, NOT_BUILT, RegMarks } from "./ui";
import { t } from "../i18n";

const LIMITS = [100, 500, 1000, 5000, 10000];
const MOD = isMac ? "⌘" : "Ctrl ";

function Ghost({ children, ...rest }: React.ButtonHTMLAttributes<HTMLButtonElement> & { children: ReactNode }) {
  return (
    <button type="button" className="flex h-[26px] items-center gap-[6px] px-s2 whitespace-nowrap enabled:hover:bg-hover" {...rest}>
      {children}
    </button>
  );
}

/** A bordered toolbar select that opens a menu or popover under itself. */
function Select({ children, label, onOpen, disabled, mono, accent, title }: {
  children: ReactNode;
  label: string;
  onOpen(anchor: Anchor): void;
  disabled?: boolean;
  mono?: boolean;
  /** The next thing to do: drawn in the accent colour. */
  accent?: boolean;
  title?: string;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      aria-haspopup="menu"
      title={title}
      disabled={disabled}
      className={`flex h-[26px] items-center gap-[6px] border px-s2 text-[12px] whitespace-nowrap enabled:hover:bg-hover ${mono ? "font-mono" : ""} ${
        accent ? "border-accent text-accent-text outline-2 outline-offset-2 outline-accent" : "border-line"
      }`}
      onClick={(e) => onOpen(anchorOf(e.currentTarget))}
    >
      {children}
      <ChevronDown size={12} />
    </button>
  );
}

interface Props {
  tab: Tab;
  conn: Conn | null;
  onRun(): void;
  onRunSelection(): void;
  /** Left out for languages without query plans. */
  onExplain?: () => void;
  onFormat(): void;
}

type Open = { menu: "context" | "limit" | "mode" | "save"; anchor: Anchor } | null;

export function Toolbar({ tab, conn, onRun, onRunSelection, onExplain, onFormat }: Props) {
  const connections = useApp((s) => s.connections);
  const aiOpen = useAi((s) => !!s.open[tab.id]);
  // N01: below 1040px of toolbar width the labels go, the icons and tooltips stay.
  const bar = useRef<HTMLDivElement>(null);
  const [compact, setCompact] = useState(false);
  useEffect(() => {
    const element = bar.current;
    if (!element) return;
    const observer = new ResizeObserver(() => setCompact(element.clientWidth < 1040));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const label = compact ? "sr-only" : "";
  const [open, setOpen] = useState<Open>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const close = () => setOpen(null);
  const running = tab.run.state === "running";
  const stopping = tab.run.state === "running" && tab.run.stopping;
  const connected = conn !== null && conn.session !== null;
  const ready = connected && !running;

  const save = (anchor: Anchor) => {
    // A query that already has a name is saved without asking again.
    if (tab.savedQueryId) void saveTabQuery(tab.id, tab.title).then(setNotice);
    else setOpen({ menu: "save", anchor });
  };
  const finish = (how: "COMMIT" | "ROLLBACK") => conn && void endTransaction(conn.id, how).then(setNotice);

  return (
    <div ref={bar} className="flex min-w-0 items-center gap-s2 overflow-hidden border-b border-line px-s3">
      <button
        type="button"
        disabled={!ready}
        title={`${t("Run")} (${MOD}↵)`}
        onClick={onRun}
        className="relative mr-s2 flex h-[26px] items-center gap-[6px] bg-accent-fill px-s3 font-heading text-[13px] leading-none font-semibold tracking-[.06em] text-on-accent"
      >
        <RegMarks corners={["tl", "tr", "bl", "br"]} />
        <Play size={12} />
        {t("RUN")}
        {!compact && <span className="font-mono text-[11px] font-normal tracking-normal">{MOD}↵</span>}
      </button>
      <Ghost disabled={!ready} onClick={onRunSelection} title={`${t("Run the selected text")} (⇧${MOD}↵)`}>
        <StepForward size={13} />
        <span className={label}>{t("Run selection")}</span>
      </Ghost>
      {running ? (
        <button
          type="button"
          disabled={stopping || !conn?.driver.capabilities.cancel}
          className="flex h-[26px] items-center gap-[6px] border border-danger px-s2 font-medium whitespace-nowrap text-danger enabled:hover:bg-danger-tint"
          onClick={() => cancelRun(tab.id)}
        >
          <Square size={12} className="fill-current" />
          <span className={label}>{stopping ? t("Stopping…") : t("Stop")}</span>
          {!stopping && !compact && <span className="font-mono text-[11px] font-normal">{MOD}.</span>}
        </button>
      ) : (
        <Ghost disabled title={t("Stop")}>
          <Square size={12} />
          <span className={label}>{t("Stop")}</span>
        </Ghost>
      )}
      <i className="h-[16px] w-px bg-line" />
      <Ghost disabled={!ready || !onExplain} onClick={onExplain} title={`${t("Show the query plan")} (${MOD}E)`}>
        <Network size={13} />
        <span className={label}>{t("Explain")}</span>
      </Ghost>
      <Ghost disabled={!connected} onClick={onFormat} title={t("Format")}>
        <AlignLeft size={13} />
        <span className={label}>{t("Format")}</span>
      </Ghost>
      <Ghost disabled={!tab.connectionId} title={`${t("Save this query")} (${MOD}S)`} data-save-button onClick={(e) => save(anchorOf(e.currentTarget))}>
        <Save size={13} />
        <span className={label}>{t("Save")}</span>
      </Ghost>
      <i className="h-[16px] w-px bg-line" />
      <button
        type="button"
        aria-pressed={aiOpen}
        title={`${t("Ask AI")} (${MOD}I)`}
        className={`flex h-[26px] items-center gap-[6px] border px-s2 whitespace-nowrap ${aiOpen ? "border-accent bg-accent-tint font-medium" : "border-transparent hover:bg-hover"}`}
        onClick={() => togglePanel(tab.id)}
      >
        <Sparkles size={13} strokeWidth={1.5} className="text-accent-text" />
        <span className={label}>{t("Ask AI")}</span>
      </button>
      {conn?.tx && (
        <>
          <i className="h-[16px] w-px bg-line" />
          <button
            type="button"
            disabled={running}
            className="flex h-[26px] items-center gap-[6px] border border-accent-fill px-s2 font-medium whitespace-nowrap text-accent-text enabled:hover:bg-accent-tint"
            onClick={() => finish("COMMIT")}
          >
            <Check size={13} />
            {t("Commit")}
          </button>
          <button type="button" disabled={running} className="flex h-[26px] items-center gap-[6px] border border-line px-s2 whitespace-nowrap enabled:hover:bg-hover" onClick={() => finish("ROLLBACK")}>
            <Undo2 size={13} />
            {t("Rollback")}
          </button>
        </>
      )}
      {notice && (
        <span className="selectable flex min-w-0 items-center gap-[6px] truncate text-[12px] text-danger" title={notice}>
          <TriangleAlert size={12} className="flex-none" />
          <span className="truncate">{notice}</span>
        </span>
      )}

      <div className="ml-auto" />
      {conn ? (
        <Select label={t("Connection, database and schema")} mono onOpen={(anchor) => setOpen({ menu: "context", anchor })}>
          <ConnectionMarker colour={conn.colour} />
          {[conn.name, conn.driver.capabilities.schemas && !compact ? conn.config.database : null, conn.namespace].filter(Boolean).map((part, i) => (
            <span key={i} className="contents">
              {i > 0 && <span className="text-text-3">/</span>}
              {part}
            </span>
          ))}
        </Select>
      ) : (
        <Select label={t("Choose connection")} accent onOpen={(anchor) => setOpen({ menu: "context", anchor })}>
          {t("Choose connection")}
        </Select>
      )}
      <Select label={t("Row limit")} disabled={!connected} onOpen={(anchor) => setOpen({ menu: "limit", anchor })}>
        {t("Limit")}<span className="font-mono">{tab.limit}</span>
      </Select>
      {/* Hidden, not disabled, for engines without transactions: there is no mode to choose. */}
      {(!conn || conn.driver.capabilities.transactions) && (
        <Select label={t("Transaction mode")} disabled={!connected} onOpen={(anchor) => setOpen({ menu: "mode", anchor })}>
          {conn?.txMode === "manual" ? t("Manual") : t("Auto-commit")}
        </Select>
      )}

      {open?.menu === "limit" && (
        <Floating anchor={open.anchor} width={200} alignRight onClose={close} label={t("Row limit")}>
          {LIMITS.map((n) => (
            <MenuItem key={n} checked={tab.limit === n} onSelect={() => { updateTab(tab.id, { limit: n }); close(); }}>
              <span className="font-mono text-[12.5px]">{n}</span>
            </MenuItem>
          ))}
          <MenuSeparator />
          <MenuItem checked={false} disabled hint={t("asks first")} title={t(NOT_BUILT)}>
            {t("No limit")}
          </MenuItem>
          <MenuItem checked={false} disabled title={t(NOT_BUILT)}>
            {t("Custom…")}
          </MenuItem>
        </Floating>
      )}

      {open?.menu === "mode" && conn && (
        <Floating anchor={open.anchor} width={340} alignRight onClose={close} label={t("Transaction mode")}>
          {([
            ["auto", t("Auto-commit"), t("Each statement is committed as soon as it runs.")],
            ["manual", t("Manual"), t("Statements run in one transaction until you commit or roll back.")],
          ] as const).map(([mode, title, text]) => (
            <MenuItem key={mode} checked={conn.txMode === mode} disabled={mode === "auto" && !!conn.tx} onSelect={() => { setTransactionMode(conn.id, mode); close(); }}>
              <span className="block py-[4px] leading-[1.35]">
                {title}
                <span className="block text-[12px] font-normal text-text-2">{text}</span>
              </span>
            </MenuItem>
          ))}
          {conn.tx && (
            <p className="flex items-start gap-s2 px-s3 py-s2 text-[12px] leading-[1.4] text-warn-text">
              <TriangleAlert size={12} className="mt-[2px] flex-none" />{t("A transaction is open. Commit or roll back before switching to Auto-commit.")}
            </p>
          )}
        </Floating>
      )}

      {open?.menu === "context" && (
        <Floating anchor={open.anchor} width={conn ? 600 : 300} alignRight onClose={close} role="dialog" label={t("Connection and schema")} className="">
          <div className={`grid ${conn ? "grid-cols-[1.3fr_1fr_1fr]" : "grid-cols-1"}`}>
            <div className="py-s1">
              <div className="label px-s3 py-s2">{t("Connection")}</div>
              {connections.length === 0 && <p className="px-s3 pb-s2 text-[12px] text-text-3">{t("No connections yet.")}</p>}
              {connections.map((c) => {
                const current = c.id === conn?.id;
                const isOpen = c.session !== null;
                return (
                  <button
                    key={c.id}
                    type="button"
                    className={`flex h-[26px] w-full items-center gap-s2 px-s3 text-left hover:bg-accent-tint ${current ? "bg-accent-tint font-medium" : ""} ${isOpen ? "" : "text-text-3"}`}
                    style={current ? { boxShadow: "inset 2px 0 0 var(--accent)" } : undefined}
                    onClick={() => {
                      // The band, tab mark and status bar follow the tab's connection at once.
                      updateTab(tab.id, { connectionId: c.id });
                      if (!isOpen) void connect(c.id);
                      close();
                    }}
                  >
                    <span className="grid h-[16px] w-[24px] flex-none place-items-center border border-line font-mono text-[10px] font-medium" style={{ borderLeft: `3px solid ${colourVar(c.colour)}` }} title={typeOf(c.engine, c.driver.id).name}>
                      {c.engine}
                    </span>
                    <span className="min-w-0 flex-1 truncate">{c.name}</span>
                    {!isOpen && <span className="text-[12px] text-accent-text">{t("Connect…")}</span>}
                    <EnvTag environment={c.environment} />
                  </button>
                );
              })}
            </div>
            {conn && (
              <>
                <div className="border-l border-line-2 py-s1">
                  <div className="label px-s3 py-s2">{t("Database")}</div>
                  {/* Changing database means a new connection on these engines: shown, not switchable here. */}
                  <div className="flex h-[26px] items-center gap-s2 px-s3 font-mono text-[12.5px] font-medium">
                    <Check size={12} />
                    <span className="truncate">{conn.driver.capabilities.schemas ? (conn.config.database ?? conn.config.user ?? "default") : (conn.namespace ?? "—")}</span>
                  </div>
                </div>
                <div className="max-h-[320px] overflow-y-auto border-l border-line-2 py-s1">
                  <div className="label px-s3 py-s2">{conn.driver.capabilities.schemas ? t("Schema") : conn.driver.capabilities.file_based ? t("Schema") : t("Use database")}</div>
                  {conn.session === null && <p className="px-s3 text-[12px] text-text-3">{t("Not connected.")}</p>}
                  {conn.namespaces.map((namespace) => {
                    const current = namespace === conn.namespace;
                    return (
                      <button
                        key={namespace}
                        type="button"
                        className={`flex h-[26px] w-full items-center gap-s2 px-s3 text-left font-mono text-[12.5px] hover:bg-accent-tint ${current ? "font-medium" : ""}`}
                        onClick={() => {
                          close();
                          void useNamespace(conn.id, namespace).then(setNotice);
                        }}
                      >
                        <span className="grid w-[12px] flex-none place-items-center">{current && <Check size={12} />}</span>
                        <span className="truncate">{namespace}</span>
                      </button>
                    );
                  })}
                </div>
              </>
            )}
          </div>
        </Floating>
      )}

      {open?.menu === "save" && conn && <SavePopover tab={tab} conn={conn} anchor={open.anchor} onClose={close} />}
    </div>
  );
}

export function SavePopover({ tab, conn, anchor, onClose }: { tab: Tab; conn: Conn; anchor: Anchor; onClose(): void }) {
  const [name, setName] = useState(tab.title);
  const [error, setError] = useState<string | null>(null);
  const taken = conn.savedQueries.some((q) => q.name.toLowerCase() === name.trim().toLowerCase());
  return (
    <Floating anchor={anchor} width={340} onClose={onClose} role="dialog" label={t("Save query")} className="p-s4">
      <form
        className="flex flex-col gap-s3"
        onSubmit={async (e) => {
          e.preventDefault();
          if (!name.trim()) return;
          const failed = await saveTabQuery(tab.id, name.trim());
          if (failed) setError(failed);
          else onClose();
        }}
      >
        <span className="font-heading text-[16px] leading-none font-semibold">{t("Save query")}</span>
        <label className="flex flex-col gap-[4px]">
          <span className="text-[12px] text-text-2">{t("Name")}</span>
          <TextInput autoFocus value={name} onChange={(e) => setName(e.target.value)} onFocus={(e) => e.target.select()} />
        </label>
        <span className="text-[12px] leading-[1.4] text-text-3">{t("Listed under Saved queries for {connection}.", { connection: conn.name })}</span>
        {taken && <span className="text-[12px] text-warn-text">{t("Another saved query already has this name.")}</span>}
        {error && <span className="selectable text-[12px] text-danger">{error}</span>}
        <div className="flex justify-end gap-s2">
          <Button variant="ghost" onClick={onClose}>
            {t("Cancel")}
          </Button>
          <Button variant="primary" type="submit" disabled={!name.trim()}>
            {t("Save")}{" "}<span className="font-mono text-[11px] font-normal tracking-normal">↵</span>
          </Button>
        </div>
      </form>
    </Floating>
  );
}
