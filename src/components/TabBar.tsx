import { Bookmark, Braces, Waypoints, ChevronDown, ChevronLeft, ChevronRight, KeyRound, Plus, Radar, Sparkles, Table, Terminal, TriangleAlert, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { activateTab, closeOtherTabs, closeTab, duplicateTab, newChatTab, newQueryTab, updateTab, useApp, type Tab } from "../store";
import { anchorOf, Floating, MenuItem, MenuSeparator, type Anchor } from "./menu";
import { Spinner } from "./Results";
import { isMac } from "./TitleBar";
import { colourVar, ConnectionMarker, EnvTag, IconButton } from "./ui";
import { t, tn } from "../i18n";

const MOD = isMac ? "⌘" : "Ctrl+";

/** What the tab is doing, in the slot of its type icon: a shape, not only a colour. */
function TabMark({ tab }: { tab: Tab }) {
  if (tab.run.state === "running" || tab.table?.loading) return <Spinner />;
  if (tab.run.state === "error" && !tab.edited) return <TriangleAlert size={13} className="text-danger" aria-label={t("The last run failed")} />;
  if (tab.kind === "table-data") return <Table size={13} />;
  if (tab.kind === "key-value") return <KeyRound size={13} />;
  if (tab.kind === "vector") return <Radar size={13} />;
  if (tab.kind === "document") return <Braces size={13} />;
  if (tab.kind === "graph") return <Waypoints size={13} />;
  if (tab.kind === "ai-chat") return <Sparkles size={13} strokeWidth={1.5} className="text-accent-text" />;
  return tab.savedQueryId ? <Bookmark size={13} /> : <Terminal size={13} />;
}

export function TabBar() {
  const tabs = useApp((s) => s.tabs);
  const activeTabId = useApp((s) => s.activeTabId);
  const connections = useApp((s) => s.connections);
  const strip = useRef<HTMLDivElement>(null);
  const [overflow, setOverflow] = useState(false);
  const [menu, setMenu] = useState<{ tabId: string; anchor: Anchor } | null>(null);
  const [list, setList] = useState<Anchor | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);

  // The strip scrolls when the tabs do not fit; the active tab is kept in view.
  useEffect(() => {
    const element = strip.current;
    if (!element) return;
    const measure = () => setOverflow(element.scrollWidth > element.clientWidth + 1);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [tabs.length]);
  useEffect(() => {
    strip.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [activeTabId]);

  const scroll = (direction: number) => strip.current?.scrollBy({ left: direction * 240, behavior: "smooth" });
  const menuTab = tabs.find((t) => t.id === menu?.tabId);

  return (
    // min-w-0: as a grid item it would otherwise grow to fit every tab and push the window sideways.
    <div className="flex min-w-0 items-stretch border-b border-line bg-surface">
      <div ref={strip} role="tablist" className="flex min-w-0 flex-1 items-stretch overflow-x-auto [scrollbar-width:none]">
        {tabs.map((tab) => {
          const conn = connections.find((c) => c.id === tab.connectionId);
          const active = tab.id === activeTabId;
          return (
            <div
              key={tab.id}
              role="tab"
              tabIndex={0}
              aria-selected={active}
              className={`group flex flex-none cursor-pointer items-center gap-s2 border-r border-line pr-s2 pl-s3 whitespace-nowrap ${
                active ? "-mb-px bg-panel" : "text-text-2 hover:bg-hover"
              }`}
              // The active tab carries its connection's colour along the top edge.
              style={active && conn ? { boxShadow: `inset 0 2px 0 ${colourVar(conn.colour)}` } : undefined}
              onClick={() => activateTab(tab.id)}
              onAuxClick={(e) => e.button === 1 && closeTab(tab.id)}
              onContextMenu={(e) => {
                e.preventDefault();
                setMenu({ tabId: tab.id, anchor: { x: e.clientX, y: e.clientY } });
              }}
              onKeyDown={(e) => {
                if (e.key === "F10" && e.shiftKey) setMenu({ tabId: tab.id, anchor: anchorOf(e.currentTarget) });
                if (e.key === "F2" && !tab.table) setRenaming(tab.id);
              }}
            >
              {conn ? <ConnectionMarker colour={conn.colour} /> : <i className="size-[8px] flex-none bg-text-3" />}
              <TabMark tab={tab} />
              {renaming === tab.id ? (
                <input
                  autoFocus
                  defaultValue={tab.title}
                  aria-label={t("Tab name")}
                  className="selectable w-[140px] border border-accent bg-panel px-[4px] font-medium focus:outline-none"
                  onFocus={(e) => e.target.select()}
                  onClick={(e) => e.stopPropagation()}
                  onBlur={(e) => {
                    const title = e.target.value.trim();
                    if (title) updateTab(tab.id, { title, dirty: tab.savedQueryId ? true : tab.dirty });
                    setRenaming(null);
                  }}
                  onKeyDown={(e) => {
                    e.stopPropagation();
                    if (e.key === "Enter") e.currentTarget.blur();
                    if (e.key === "Escape") setRenaming(null);
                  }}
                />
              ) : (
                <span className={active ? "font-medium" : ""}>{tab.title}</span>
              )}
              <span className="font-mono text-[11px] text-text-3">{conn ? conn.name : t("No connection")}</span>
              {conn?.environment === "production" && <EnvTag environment="production" />}
              {/* A finished run in a background tab, or unsaved text: a dot until the tab is visited or hovered. */}
              {(tab.unseen || tab.dirty) && (
                <i
                  title={tab.unseen ? t("Finished while you were in another tab") : t("Unsaved changes")}
                  className={`mx-[5.5px] size-[7px] flex-none rounded-full group-hover:hidden ${tab.unseen ? "bg-accent" : "bg-text-2"}`}
                />
              )}
              <IconButton
                label={`${t("Close tab")} (${MOD}W)`}
                size={18}
                className={`text-text-3 hover:text-text ${tab.unseen || tab.dirty ? "hidden group-hover:grid" : ""}`}
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(tab.id);
                }}
              >
                <X size={12} />
              </IconButton>
            </div>
          );
        })}
        <button type="button" title={`${t("New query tab")} (${MOD}T)`} aria-label={t("New query tab")} className="grid w-[34px] flex-none place-items-center text-text-2 hover:bg-hover" onClick={() => newQueryTab()}>
          <Plus size={14} />
        </button>
        {/* The entry point for a chat tab is not drawn in the design. */}
        <button type="button" title={t("New AI chat")} aria-label={t("New AI chat")} className="grid w-[30px] flex-none place-items-center text-accent-text hover:bg-hover" onClick={() => newChatTab()}>
          <Sparkles size={14} strokeWidth={1.5} />
        </button>
      </div>

      {overflow && (
        <div className="flex flex-none items-stretch border-l border-line bg-surface" style={{ boxShadow: "-8px 0 8px -8px color-mix(in srgb, var(--text) 25%, transparent)" }}>
          <button type="button" title={t("Scroll tabs left")} aria-label={t("Scroll tabs left")} className="grid w-[26px] place-items-center text-text-2 hover:bg-hover" onClick={() => scroll(-1)}>
            <ChevronLeft size={14} />
          </button>
          <button type="button" title={t("Scroll tabs right")} aria-label={t("Scroll tabs right")} className="grid w-[26px] place-items-center text-text-2 hover:bg-hover" onClick={() => scroll(1)}>
            <ChevronRight size={14} />
          </button>
          <button type="button" className="flex items-center gap-[4px] border-l border-line px-s2 text-[12px] whitespace-nowrap text-text-2 hover:bg-hover" onClick={(e) => setList(anchorOf(e.currentTarget))}>
            {tn(tabs.length, "{count} tab", "{count} tabs")}
            <ChevronDown size={12} />
          </button>
        </div>
      )}

      {menu && menuTab && (
        <Floating anchor={menu.anchor} width={220} onClose={() => setMenu(null)} label={t("Tab")}>
          <MenuItem hint={`${MOD}W`} onSelect={() => { setMenu(null); closeTab(menuTab.id); }}>
            {t("Close")}
          </MenuItem>
          <MenuItem disabled={tabs.length < 2} onSelect={() => { setMenu(null); closeOtherTabs(menuTab.id); }}>
            {t("Close others")}
          </MenuItem>
          <MenuSeparator />
          <MenuItem disabled={menuTab.kind !== "query"} onSelect={() => { setMenu(null); duplicateTab(menuTab.id); }}>
            {t("Duplicate")}
          </MenuItem>
          <MenuItem hint="F2" disabled={!!menuTab.table} onSelect={() => { setMenu(null); setRenaming(menuTab.id); }}>
            {t("Rename…")}
          </MenuItem>
        </Floating>
      )}

      {/* The list of all tabs has no design yet: the menu style with marker, title and connection. */}
      {list && (
        <Floating anchor={list} width={300} alignRight onClose={() => setList(null)} label={t("All tabs")}>
          {tabs.map((tab) => {
            const conn = connections.find((c) => c.id === tab.connectionId);
            return (
              <MenuItem key={tab.id} checked={tab.id === activeTabId} hint={conn?.name} onSelect={() => { setList(null); activateTab(tab.id); }}>
                <span className="flex items-center gap-s2">
                  {conn && <ConnectionMarker colour={conn.colour} />}
                  <span className="truncate">{tab.title}</span>
                </span>
              </MenuItem>
            );
          })}
        </Floating>
      )}
    </div>
  );
}
