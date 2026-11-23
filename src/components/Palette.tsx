// The command palette (P01–P04): go to a table, connection, saved query or command.
import { Bookmark, ChevronRight, Clock, Search, Table } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { togglePanel } from "../ai-store";
import { fuzzy, type Match } from "../lib/fuzzy";
import { typeOf } from "../lib/types";
import { msg, t } from "../i18n";
import { closeTab, connect, newChatTab, newQueryTab, openDialog, openSavedQuery, openTable, setTheme, useApp, type Conn } from "../store";
import { ConnectionMarker } from "./ui";

const isMac = /Mac/i.test(navigator.userAgent);
const MOD = isMac ? "⌘" : "Ctrl+";

interface Item {
  group: "tables" | "connections" | "queries" | "commands" | "recent";
  key: string;
  icon: ReactNode;
  title: string;
  secondary?: string;
  conn?: Conn;
  shortcut?: string;
  run(): void;
  match?: Match;
}

const GROUP_LABEL: Record<Item["group"], string> = {
  tables: msg("Tables"),
  connections: msg("Connections"),
  queries: msg("Saved queries"),
  commands: msg("Commands"),
  recent: msg("Recent"),
};
const TYPED_ORDER: Item["group"][] = ["tables", "connections", "queries", "commands", "recent"];
const OPEN_ORDER: Item["group"][] = ["recent", "connections", "commands"];

function Title({ text, match }: { text: string; match?: Match }) {
  if (!match || match.ranges.length === 0) return <>{text}</>;
  const parts: ReactNode[] = [];
  let at = 0;
  match.ranges.forEach(([from, to], i) => {
    parts.push(text.slice(at, from));
    parts.push(
      <span key={i} className="font-medium text-accent-text">
        {text.slice(from, to)}
      </span>,
    );
    at = to;
  });
  parts.push(text.slice(at));
  return <>{parts}</>;
}

export function Palette() {
  const connections = useApp((s) => s.connections);
  const recents = useApp((s) => s.recents);
  const activeTabId = useApp((s) => s.activeTabId);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const list = useRef<HTMLDivElement>(null);
  const close = () => openDialog(null);

  const all = useMemo(() => {
    const badge = (conn: Conn) => <span className="grid h-[16px] w-[22px] place-items-center border border-text-3 font-mono text-[10px] leading-none">{conn.engine}</span>;
    const items: Item[] = [];
    for (const conn of connections) {
      items.push({ group: "connections", key: `c:${conn.id}`, icon: badge(conn), title: conn.name, secondary: [typeOf(conn.engine, conn.driver.id).name, conn.config.host].filter(Boolean).join(" · "), conn, run: () => void connect(conn.id) });
      if (conn.session === null) continue;
      for (const [namespace, objects] of Object.entries(conn.objects)) {
        for (const object of objects) {
          items.push({ group: "tables", key: `t:${conn.id}:${namespace}:${object.name}`, icon: <Table size={14} />, title: object.name, secondary: namespace || undefined, conn, run: () => openTable(conn.id, namespace, object.name) });
        }
      }
      for (const saved of conn.savedQueries ?? []) {
        items.push({ group: "queries", key: `q:${saved.id}`, icon: <Bookmark size={14} />, title: saved.name, secondary: saved.sql.replace(/\s+/g, " ").slice(0, 60), conn, run: () => openSavedQuery(conn.id, saved) });
      }
    }
    for (const recent of recents) {
      const conn = connections.find((c) => c.id === recent.connId);
      if (!conn) continue;
      const saved = recent.kind === "query" ? conn.savedQueries?.find((q) => q.id === recent.queryId) : undefined;
      if (recent.kind === "query" && !saved) continue;
      items.push({
        group: "recent",
        key: `r:${recent.kind}:${recent.connId}:${recent.namespace ?? ""}:${recent.name}`,
        icon: <Clock size={14} />,
        title: recent.name,
        secondary: recent.kind === "table" ? recent.namespace : t("Saved query"),
        conn,
        run: () => (saved ? openSavedQuery(conn.id, saved) : openTable(conn.id, recent.namespace ?? "", recent.name)),
      });
    }
    const command = (title: string, area: string, run: () => void, shortcut?: string): Item => ({ group: "commands", key: `k:${title}`, icon: <ChevronRight size={14} />, title, secondary: area, shortcut, run });
    items.push(
      command(t("New connection"), t("Connections"), () => openDialog({ kind: "picker" }), `${MOD}N`),
      command(t("New query tab"), t("Editor"), () => void newQueryTab(), `${MOD}T`),
      command(t("New AI chat"), "AI", () => void newChatTab()),
      command(t("Ask AI about the statement"), "AI", () => activeTabId && togglePanel(activeTabId, true), `${MOD}I`),
      command(t("Close tab"), t("Editor"), () => activeTabId && closeTab(activeTabId), `${MOD}W`),
      command(t("Open Settings"), t("Settings"), () => openDialog({ kind: "settings", section: "appearance" })),
      command(t("AI providers…"), t("Settings"), () => openDialog({ kind: "settings", section: "ai" })),
      command(t("Keyboard shortcuts"), t("Settings"), () => openDialog({ kind: "settings", section: "shortcuts" })),
      command(t("Theme: Light"), t("Appearance"), () => setTheme("light")),
      command(t("Theme: Dark"), t("Appearance"), () => setTheme("dark")),
      command(t("Theme: System"), t("Appearance"), () => setTheme("system")),
    );
    return items;
  }, [connections, recents, activeTabId]);

  const commandsOnly = query.startsWith(">");
  const needle = (commandsOnly ? query.slice(1) : query).trim();

  // Groups in their fixed order, each with its total and the rows shown.
  const groups = useMemo(() => {
    const order = commandsOnly ? (["commands"] as Item["group"][]) : needle === "" ? OPEN_ORDER : TYPED_ORDER;
    const limit = commandsOnly ? 50 : needle === "" ? 3 : 5;
    return order
      .map((group) => {
        const matched = all
          .filter((item) => item.group === group)
          .map((item) => ({ ...item, match: fuzzy(needle, item.title) ?? undefined }))
          .filter((item) => item.match);
        if (needle !== "") matched.sort((a, b) => b.match!.score - a.match!.score);
        return { group, total: matched.length, rows: matched.slice(0, limit) };
      })
      .filter((g) => g.total > 0);
  }, [all, needle, commandsOnly]);
  const rows = groups.flatMap((g) => g.rows);

  useEffect(() => setSelected(0), [query]);
  useEffect(() => {
    list.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
  }, [selected]);

  const run = (item: Item | undefined) => {
    if (!item) return;
    close();
    item.run();
  };

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-scrim pt-[96px]" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div role="dialog" aria-modal="true" aria-label={t("Command palette")} className="flex max-h-[calc(100vh-140px)] w-[680px] max-w-[calc(100vw-32px)] flex-col border border-line bg-bg" style={{ boxShadow: "var(--shadow-lg)" }}>
        <div className="flex h-[46px] flex-none items-center gap-s3 border-b border-line px-s4">
          <Search size={16} className="flex-none text-text-3" />
          <input
            autoFocus
            value={query}
            spellCheck={false}
            role="combobox"
            aria-expanded="true"
            aria-controls="palette-list"
            aria-label={t("Go to table, connection or command")}
            placeholder={t("Go to table, connection or command")}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") {
                e.preventDefault();
                setSelected((n) => Math.min(rows.length - 1, n + 1));
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                setSelected((n) => Math.max(0, n - 1));
              } else if (e.key === "Enter") {
                e.preventDefault();
                run(rows[selected]);
              } else if (e.key === "Escape") {
                e.preventDefault();
                e.stopPropagation();
                close();
              }
            }}
            className="min-w-0 flex-1 bg-transparent text-[15px] outline-none placeholder:text-text-3"
          />
          <span className="border border-line px-[5px] py-[1px] font-mono text-[11px] text-text-3">Esc</span>
        </div>

        <div ref={list} id="palette-list" role="listbox" className="min-h-0 overflow-auto py-s1">
          {rows.length === 0 ? (
            // P03
            <div className="flex flex-col items-start gap-s2 px-s4 py-s4">
              <span className="font-heading text-[20px] leading-[1.2] font-semibold">{t("Nothing matches “{query}”", { query: needle })}</span>
              <span className="text-[12.5px] text-text-2">{commandsOnly ? t("Commands were searched.") : t("Tables of connected databases, connections, saved queries and commands were searched.")}</span>
              <button type="button" className="text-[12.5px] text-accent-text hover:underline" onClick={() => setQuery(commandsOnly ? ">" : "")}>
                {t("Clear")}
              </button>
            </div>
          ) : (
            groups.map((g) => (
              <div key={g.group}>
                <div className="label flex items-center gap-s2 px-s4 pt-s2 pb-[4px]">
                  {t(GROUP_LABEL[g.group])}
                  <span className="font-mono font-normal tracking-normal">{g.total}</span>
                </div>
                {g.rows.map((item) => {
                  const index = rows.indexOf(item);
                  const active = index === selected;
                  return (
                    <div
                      key={item.key}
                      role="option"
                      aria-selected={active}
                      className={`flex h-[30px] cursor-pointer items-center gap-s3 px-s4 ${active ? "bg-accent-tint" : "hover:bg-hover"}`}
                      style={active ? { boxShadow: "inset 2px 0 0 var(--accent)" } : undefined}
                      onMouseMove={() => setSelected(index)}
                      onClick={() => run(item)}
                    >
                      <span className="grid w-[22px] flex-none place-items-center text-text-2">{item.icon}</span>
                      <span className="truncate text-[13px]">
                        <Title text={item.title} match={item.match} />
                      </span>
                      {item.secondary && <span className="min-w-0 truncate font-mono text-[11.5px] text-text-3">{item.secondary}</span>}
                      <span className="flex-1" />
                      {item.conn && item.group !== "connections" && (
                        <span className="flex flex-none items-center gap-[6px] font-mono text-[11px] text-text-2">
                          <ConnectionMarker colour={item.conn.colour} />
                          {item.conn.name}
                        </span>
                      )}
                      {item.shortcut && <span className="flex-none font-mono text-[11px] text-text-3">{item.shortcut}</span>}
                      {active && <span className="flex-none text-[12px] text-accent-text">↵ {commandsOnly || item.group === "commands" ? t("Run") : t("Open")}</span>}
                    </div>
                  );
                })}
              </div>
            ))
          )}
        </div>

        <div className="flex h-[30px] flex-none items-center gap-s4 border-t border-line bg-surface px-s4 font-mono text-[11px] text-text-2">
          <span>↑↓ {t("Move")}</span>
          <span>↵ {commandsOnly ? t("Run command") : t("Open")}</span>
          <span className="flex-1" />
          <span>{commandsOnly ? t("Remove > to search everything") : t("> Commands only")}</span>
        </div>
      </div>
    </div>
  );
}
