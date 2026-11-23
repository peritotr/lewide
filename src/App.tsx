import { Info } from "lucide-react";
import { format } from "sql-formatter";
import { useEffect, useMemo, useRef, useState } from "react";
import { ConnectionForm } from "./components/connections/ConnectionForm";
import { FirstLaunch } from "./components/connections/FirstLaunch";
import { HostKeyChangedDialog, UnknownHostDialog } from "./components/connections/HostKeyDialogs";
import { PasswordPrompt, RemoveDialog } from "./components/connections/Prompts";
import { ConnectionsFileDialog } from "./components/connections/ConnectionsFile";
import { TypePicker } from "./components/connections/TypePicker";
import { Button, Dialog, TextInput } from "./components/dialog";
import { Editor, type EditorHandle, type ErrorMark } from "./components/Editor";
import { errorPlace, Results } from "./components/Results";
import { Sidebar, SidebarRail } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { TabBar } from "./components/TabBar";
import { KeyValueView } from "./components/KeyValueView";
import { DocumentView } from "./components/DocumentView";
import { GraphView } from "./components/GraphView";
import { VectorView } from "./components/VectorView";
import { TableView } from "./components/TableView";
import { AskPanel, reviewMarks } from "./components/ai/AskPanel";
import { ChatView } from "./components/ai/ChatView";
import { SentDialog } from "./components/ai/SentDialog";
import { usePrefs } from "./prefs";
import { Palette } from "./components/Palette";
import { Settings } from "./components/Settings";
import { togglePanel, useAi } from "./ai-store";
import { ExportDialog } from "./components/ExportDialog";
import { ImportDialog } from "./components/ImportDialog";
import { ApplyChangesDialog, DiscardChangesDialog } from "./components/TableDialogs";
import { AlterDialog } from "./components/StructureView";
import { TitleBar } from "./components/TitleBar";
import { Toolbar } from "./components/Toolbar";
import { compactCount } from "./lib/api";
import { formatConsole, formatCypher, languageOf } from "./lib/languages";
import { dialectOf, type Destructive } from "./lib/sql";
import {
  activeTab,
  cancelRun,
  closeTab,
  connect,
  connOf,
  editSql,
  endTransaction,
  explain,
  newQueryTab,
  openDialog,
  run,
  runNow,
  updateTab,
  useApp,
  type Conn,
  type RunItem,
  type Tab, setSidebarCollapsed } from "./store";
import { msg, t, tn, tx } from "./i18n";

const FORMATTER_LANGUAGE = { postgres: "postgresql", mysql: "mysql", sqlite: "sqlite", sqlserver: "transactsql", clickhouse: "clickhouse", duckdb: "duckdb" } as const;

function Workspace({ tab, conn }: { tab: Tab; conn: Conn | null }) {
  const resultsHeight = useApp((s) => s.resultsHeight);
  const editor = useRef<EditorHandle | null>(null);
  // What the last Run executed, for "Run again".
  const lastRun = useRef<RunItem[]>([]);
  const dialect = dialectOf(conn?.driver.id);
  const language = useMemo(() => languageOf(conn?.driver.id, dialect), [conn?.driver.id, dialect]);
  const production = conn?.environment === "production";
  const lost = conn !== null && conn.session === null && conn.status !== "connecting";

  // Table names complete at once; their columns arrive as they are loaded.
  const schema = useMemo(() => {
    const tables = conn?.objects[conn.namespace ?? ""] ?? [];
    return Object.fromEntries(tables.map((t) => [t.name, conn?.columns[t.name] ?? []]));
  }, [conn?.objects, conn?.columns, conn?.namespace]);

  // Where the last run failed, until the text is edited or run again.
  const place = errorPlace(tab);
  const errorMark = useMemo<ErrorMark | null>(
    () =>
      tab.run.state === "error" && place && !tab.edited
        ? { line: place.line, offset: place.offset !== null ? place.offset - 1 : null, message: tab.run.error.message }
        : null,
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [tab.run, tab.edited],
  );

  const aiOpen = useAi((s) => !!s.open[tab.id]);
  // AI05: the last review's findings, marked in the editor while the panel is open.
  const lastTurn = useAi((s) => s.threads[tab.id]?.slice(-1)[0]);
  const aiMarks = useMemo(() => (aiOpen ? reviewMarks(lastTurn) : null), [aiOpen, lastTurn]);
  const execute = (items: RunItem[]) => {
    if (items.length === 0) return;
    lastRun.current = items;
    run(tab.id, items);
  };
  const runAtCaret = () => {
    const statement = editor.current?.statementAtCaret();
    if (statement) execute([{ text: statement.text, from: statement.from }]);
  };
  const runSelection = () => {
    const statements = editor.current?.selectedStatements();
    if (statements) execute(statements.map((s) => ({ text: s.text, from: s.from })));
  };
  const runExplain = () => {
    const statement = editor.current?.statementAtCaret();
    if (statement) void explain(tab.id, statement.text);
  };
  const formatText = () => {
    const text = editor.current?.text();
    if (!text?.trim()) return;
    try {
      if (language.kind === "console") editor.current?.replaceText(formatConsole(text, usePrefs.getState().tabWidth));
      else if (language.kind === "cypher") editor.current?.replaceText(formatCypher(text));
      else editor.current?.replaceText(format(text, { language: FORMATTER_LANGUAGE[language.dialect], keywordCase: usePrefs.getState().uppercaseKeywords ? "upper" : "preserve", tabWidth: usePrefs.getState().tabWidth }));
    } catch {
      // Text the formatter cannot parse is left as it is.
    }
  };

  // Shortcuts that belong to the tab in front.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || useApp.getState().dialog) return;
      if (e.key === "i" && !e.shiftKey && !e.altKey) {
        e.preventDefault();
        togglePanel(tab.id);
      } else if (e.key === ".") {
        e.preventDefault();
        void cancelRun(tab.id);
      } else if (e.key === "s") {
        e.preventDefault();
        document.querySelector<HTMLButtonElement>("[data-save-button]")?.click();
      } else if (e.key === "w") {
        e.preventDefault();
        closeTab(tab.id);
      } else if (e.key === "Enter" && e.altKey && conn?.tx) {
        e.preventDefault();
        void endTransaction(conn.id, "COMMIT");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [tab.id, conn?.id, conn?.tx]);

  return (
    <main
      className="grid min-h-0 min-w-0 grid-cols-[minmax(0,1fr)] bg-panel"
      // Tab bar / Production band (only for Production) / toolbar / notice / editor / results.
      style={{ gridTemplateRows: `34px ${production ? "4px" : "0px"} 40px minmax(0,1fr)` }}
    >
      <TabBar />
      <div title={production ? t("Production connection") : undefined} className={production ? "production-band" : ""} />
      <Toolbar tab={tab} conn={conn} onRun={runAtCaret} onRunSelection={runSelection} onExplain={language.kind === "sql" ? runExplain : undefined} onFormat={formatText} />
      <div className="flex min-h-0 min-w-0">
      <div className="grid min-h-0 min-w-0 flex-1 grid-cols-[minmax(0,1fr)]" style={{ gridTemplateRows: `auto minmax(0,1fr) ${resultsHeight}px` }}>
      {conn === null ? (
        <div className="mx-s3 mt-s2 flex items-center gap-s2 border border-line px-s3 py-s2 text-[12.5px]">
          <Info size={14} className="flex-none text-text-2" />
          {t("This tab has no connection. You can write here; choose a connection to run or get completions.")}
        </div>
      ) : lost ? (
        <div className="mx-s3 mt-s2 flex items-center gap-s3 border border-warn bg-warn-tint px-s3 py-[5px] text-[12.5px]">
          <i className="size-[9px] flex-none border border-warn-text" style={{ background: "linear-gradient(to top right, transparent calc(50% - 0.6px), var(--warn-text) calc(50% - 0.6px), var(--warn-text) calc(50% + 0.6px), transparent calc(50% + 0.6px))" }} />
          <span className="min-w-0 flex-1">{t("{connection} is disconnected. Your query text is kept. Reconnect to run it.", { connection: conn.name })}</span>
          <button type="button" className="h-[22px] border border-line bg-panel px-s2 hover:bg-hover" onClick={() => connect(conn.id)}>
            {t("Reconnect")}
          </button>
          <button type="button" className="h-[22px] px-s2 hover:bg-hover" onClick={() => updateTab(tab.id, { connectionId: null })}>
            {t("Use another connection")}
          </button>
        </div>
      ) : (
        <div />
      )}
      <Editor
        key={`${tab.id}:${language.kind}:${dialect}`}
        language={language}
        schema={schema}
        initialText={tab.sql}
        error={errorMark}
        aiMarks={aiMarks}
        handle={editor}
        onChange={(sql) => editSql(tab.id, sql)}
        onCaret={(line, col) => updateTab(tab.id, { caret: { line, col } })}
        onRun={runAtCaret}
        onRunSelection={runSelection}
        onExplain={() => language.kind === "sql" && runExplain()}
      />
      <Results
        tab={tab}
        conn={conn}
        onGoTo={(to) => editor.current?.goTo({ line: to.line, offset: to.offset !== null ? to.offset - 1 : null })}
        onRunAgain={() => execute(lastRun.current)}
      />
      </div>
      {aiOpen && <AskPanel tab={tab} conn={conn} editor={editor} />}
      </div>
    </main>
  );
}

const RISK: Record<Destructive, { verb: string; sentence: string }> = {
  "update without WHERE": { verb: "UPDATE", sentence: msg("This UPDATE has no WHERE clause.") },
  "delete without WHERE": { verb: "DELETE", sentence: msg("This DELETE has no WHERE clause.") },
  delete: { verb: "DELETE", sentence: msg("This DELETE removes every row its condition matches.") },
  drop: { verb: "DROP", sentence: msg("This DROP removes the object and everything in it.") },
  truncate: { verb: "TRUNCATE", sentence: msg("This TRUNCATE removes every row of the table.") },
  alter: { verb: "ALTER", sentence: msg("This ALTER drops part of the table.") },
};

/** Statements that remove or mass-change data, held until the connection's name is typed. */
function ConfirmRun({ conn, tabId, items, found }: { conn: Conn; tabId: string; items: RunItem[]; found: { kind: Destructive; text: string }[] }) {
  const [typed, setTyped] = useState("");
  const matches = typed === conn.name;
  const first = found[0];
  const risk = RISK[first.kind];
  // The table the statement names, to show how many rows the database thinks it has.
  const table = /\b(?:update|from|table|truncate)\s+(?:only\s+)?[`"]?([\w$]+)[`"]?(?:\.[`"]?([\w$]+)[`"]?)?/i.exec(first.text);
  const name = table?.[2] ?? table?.[1];
  const estimate = Object.values(conn.objects).flat().find((o) => o.name === name)?.row_estimate ?? null;
  const proceed = () => {
    if (!matches) return;
    openDialog(null);
    void runNow(tabId, items);
  };

  return (
    <Dialog
      alert
      title={t("Run this on Production?")}
      headerExtra={<span className="border border-warn px-[4px] py-[2px] font-heading text-[10px] leading-none font-semibold tracking-[.08em] text-warn-text">{t("PRODUCTION")}</span>}
      band="production"
      width={600}
      offset={60}
      onClose={() => openDialog(null)}
      onSubmit={proceed}
      footer={
        <>
          <Button onClick={() => openDialog(null)}>{t("Cancel")}</Button>
          <Button variant="danger" type="submit" disabled={!matches}>
            {found.length === 1 ? t("Run {verb}", { verb: risk.verb }) : tn(found.length, "Run {count} statement", "Run {count} statements")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-s3 leading-[1.5]">
        <p>
          <span className="font-medium">{t(risk.sentence)}</span>{" "}
          {name
            ? estimate !== null
              ? tx("It runs on {table} in {connection}, which has about {rows} rows.", {
                  table: <span key="table" className="font-mono text-[12.5px]">{name}</span>,
                  connection: conn.name,
                  rows: compactCount(estimate),
                })
              : tx("It runs on {table} in {connection}.", { table: <span key="table" className="font-mono text-[12.5px]">{name}</span>, connection: conn.name })
            : t("It runs on {connection}.", { connection: conn.name })}{" "}
          {t("It cannot be undone from this app.")}
        </p>
        {/* Several matching statements in one run: listed one under another (not drawn in the design). */}
        {found.map((item, i) => (
          <pre key={i} className="selectable max-h-[168px] overflow-auto border border-line bg-panel px-s3 py-s2 font-mono text-[12.5px] leading-[20px] whitespace-pre-wrap">
            {item.text}
          </pre>
        ))}
        <label className="flex flex-col gap-[4px]">
          <span className="text-[12px] text-text-2">{t("Type the connection name to confirm")}</span>
          <TextInput mono data-autofocus value={typed} onChange={(e) => setTyped(e.target.value)} after={<span className="font-mono text-[12px] whitespace-nowrap text-text-3">{conn.name}</span>} />
        </label>
      </div>
    </Dialog>
  );
}

/** The one dialog on screen, chosen by the store. */
function Dialogs() {
  const dialog = useApp((s) => s.dialog);
  const connections = useApp((s) => s.connections);
  if (!dialog) return null;
  const conn = "connId" in dialog ? connections.find((c) => c.id === dialog.connId) : undefined;
  switch (dialog.kind) {
    case "picker":
      return <TypePicker draft={dialog.draft} />;
    case "form":
      return <ConnectionForm key={`${dialog.driverId}:${dialog.editId ?? "new"}`} driverId={dialog.driverId} engine={dialog.engine} editId={dialog.editId} initial={dialog.draft} />;
    case "password":
      return conn ? <PasswordPrompt key={dialog.error} conn={conn} answers={dialog.answers} error={dialog.error} /> : null;
    case "unknown-host":
      return conn ? (
        <UnknownHostDialog
          host={conn.config.ssh?.host ?? ""}
          hostKey={dialog.key}
          onCancel={() => openDialog(null)}
          onTrust={(rememberHost) => connect(conn.id, { ...dialog.answers, acceptFingerprint: dialog.key.fingerprint, rememberHost })}
        />
      ) : null;
    case "host-changed":
      return <HostKeyChangedDialog change={dialog.change} onClose={() => openDialog(null)} />;
    case "remove":
      return conn ? <RemoveDialog conn={conn} /> : null;
    case "connections-file":
      return <ConnectionsFileDialog mode={dialog.mode} ids={dialog.ids} />;
    case "alter": {
      const tab = useApp.getState().tabs.find((t) => t.id === dialog.tabId);
      const target = connections.find((c) => c.id === tab?.connectionId);
      return tab?.table && target ? <AlterDialog tab={tab} view={tab.table} conn={target} statements={dialog.statements} /> : null;
    }
    case "apply-changes":
    case "discard-changes": {
      const tab = useApp.getState().tabs.find((t) => t.id === dialog.tabId);
      const target = connections.find((c) => c.id === tab?.connectionId);
      if (!tab?.table || !target) return null;
      return dialog.kind === "apply-changes" ? (
        <ApplyChangesDialog tabId={tab.id} view={tab.table} conn={target} statements={dialog.statements} />
      ) : (
        <DiscardChangesDialog tabId={tab.id} view={tab.table} then={dialog.then} />
      );
    }
    case "palette":
      return <Palette />;
    case "settings":
      return <Settings section={dialog.section} aiTab={dialog.aiTab} />;
    case "ai-sent":
      return <SentDialog conn={connections.find((c) => c.id === dialog.connId) ?? null} sql={dialog.sql} lines={dialog.lines} />;
    case "import": {
      const tab = useApp.getState().tabs.find((t) => t.id === dialog.tabId);
      return conn && tab?.table ? <ImportDialog conn={conn} tabId={tab.id} view={tab.table} /> : null;
    }
    case "export":
      return conn ? <ExportDialog conn={conn} source={dialog.source} /> : null;
    case "confirm-run": {
      const target = connections.find((c) => c.id === useApp.getState().tabs.find((t) => t.id === dialog.tabId)?.connectionId);
      return target ? <ConfirmRun conn={target} tabId={dialog.tabId} items={dialog.items} found={dialog.found} /> : null;
    }
  }
}

export default function App() {
  const tab = useApp(activeTab);
  const conn = useApp((s) => connOf(s, activeTab(s)));
  const theme = useApp((s) => s.theme);
  const sidebarWidth = useApp((s) => s.sidebarWidth);
  const sidebarCollapsed = useApp((s) => s.sidebarCollapsed);
  const hasConnections = useApp((s) => s.connections.length > 0);
  // A new key rebuilds the interface, so every text is read again in the chosen language.
  const language = useApp((s) => s.language);

  // Application shortcuts: new connection, new query tab.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey) return;
      if (e.key === "k") {
        // The palette opens over anything but another dialog, and closes on the same keys.
        const open = useApp.getState().dialog;
        if (open && open.kind !== "palette") return;
        e.preventDefault();
        openDialog(open ? null : { kind: "palette" });
        return;
      }
      if (useApp.getState().dialog) return;
      if (e.key === "b" && !e.shiftKey) {
        e.preventDefault();
        setSidebarCollapsed(!useApp.getState().sidebarCollapsed);
        return;
      }
      if (e.key === "n") {
        e.preventDefault();
        openDialog({ kind: "picker" });
      } else if (e.key === "t" && useApp.getState().activeConnectionId) {
        e.preventDefault();
        newQueryTab();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // "system" follows the operating system and keeps following it while the app is open.
  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      document.documentElement.dataset.theme = theme === "system" ? (media.matches ? "dark" : "light") : theme;
    };
    apply();
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [theme]);

  return (
    <div key={language} className="grid h-full grid-rows-[38px_minmax(0,1fr)_24px] overflow-hidden">
      <TitleBar />
      <div className="grid min-h-0" style={{ gridTemplateColumns: `${sidebarCollapsed ? 40 : sidebarWidth}px minmax(0,1fr)` }}>
        {sidebarCollapsed ? <SidebarRail /> : <Sidebar />}
        {tab?.table ? (
          <TableView tab={tab} view={tab.table} conn={conn} />
        ) : tab?.keys ? (
          <KeyValueView key={`${tab.id}:${tab.keys.db}`} tab={tab} view={tab.keys} conn={conn} />
        ) : tab?.doc ? (
          <DocumentView key={tab.id} tab={tab} database={tab.doc.database} collection={tab.doc.collection} conn={conn} />
        ) : tab?.graph ? (
          <GraphView key={tab.id} tab={tab} conn={conn} />
        ) : tab?.vector ? (
          <VectorView key={tab.id} tab={tab} collection={tab.vector.collection} conn={conn} />
        ) : tab?.kind === "ai-chat" ? (
          <ChatView key={tab.id} tab={tab} conn={conn} />
        ) : tab ? (
          <Workspace tab={tab} conn={conn} />
        ) : hasConnections ? (
          // With connections but no tab open: not designed yet.
          <main className="grid min-w-0 place-items-center bg-panel">
            <p className="text-text-2">{t("Double-click a connection to open it.")}</p>
          </main>
        ) : (
          <FirstLaunch />
        )}
      </div>
      <StatusBar />
      <Dialogs />
    </div>
  );
}
