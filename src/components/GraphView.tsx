// A graph tab (R01–R02): Cypher at the top, and its result as a canvas of nodes
// and relationships, as a table, or as text. The canvas is laid out once per
// result and then stays still; nodes can be dragged, expanded and hidden.
import { Minus, Plus, TriangleAlert, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { api, dbError, displayValue, errorMessage, type CypherResult, type GraphCell, type GraphNode, type GraphRelationship, type Neighbours } from "../lib/api";
import { caption, fit, labelLetters, layout, type Pos } from "../lib/graph-layout";
import { setGraphText, type Conn, type Tab } from "../store";
import { t, tn } from "../i18n";
import { Button, Segmented, Strip } from "./dialog";
import { TabBar } from "./TabBar";
import { Loading } from "./TableView";

const outline = "flex h-[24px] flex-none items-center gap-[6px] border border-line px-s2 text-[12px] whitespace-nowrap enabled:hover:bg-hover disabled:cursor-not-allowed disabled:opacity-45";
const label = "font-heading text-[11px] font-semibold tracking-[.08em] text-text-2 uppercase";
const MOD = /Mac/i.test(navigator.userAgent) ? "⌘" : "Ctrl+";
const RADIUS = 20;
const MAX_ROWS = 1000;

type Mode = "graph" | "table" | "text";
interface Graph {
  nodes: GraphNode[];
  relationships: GraphRelationship[];
}

const tone = (value: unknown) => (typeof value === "string" ? "var(--syn-str)" : typeof value === "number" ? "var(--syn-num)" : value === null || typeof value === "boolean" ? "var(--syn-kw)" : undefined);
const propertyText = (value: unknown) => (typeof value === "string" ? value : JSON.stringify(value));

function merge(graph: Graph, more: Graph): Graph {
  const nodes = new Set(graph.nodes.map((n) => n.id));
  const relationships = new Set(graph.relationships.map((r) => r.id));
  return {
    nodes: [...graph.nodes, ...more.nodes.filter((n) => !nodes.has(n.id))],
    relationships: [...graph.relationships, ...more.relationships.filter((r) => !relationships.has(r.id))],
  };
}

export function GraphView({ tab, conn }: { tab: Tab; conn: Conn | null }) {
  const session = conn?.session ?? null;
  const production = conn?.environment === "production";
  const text = tab.graph?.text ?? "";

  const [limit, setLimit] = useState("300");
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<{ message: string; code: string | null; position: number | null; line: number | null } | null>(null);
  const [results, setResults] = useState<CypherResult[] | null>(null);
  const [shown, setShown] = useState(0);
  const [mode, setMode] = useState<Mode>("graph");
  const [graph, setGraph] = useState<Graph>({ nodes: [], relationships: [] });
  const [positions, setPositions] = useState<Record<string, Pos>>({});
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const [selected, setSelected] = useState<string | null>(null);
  const [neighbours, setNeighbours] = useState<Neighbours | null>(null);
  const [view, setView] = useState({ x: 0, y: 0, k: 1 });
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [notice, setNotice] = useState<string | null>(null);
  const canvas = useRef<HTMLDivElement>(null);
  const limitField = useRef<HTMLInputElement>(null);
  const editor = useRef<HTMLTextAreaElement>(null);
  const run = useRef(0);

  const result = results?.[shown] ?? null;
  const nodeLimit = Math.max(1, Math.min(5000, Number.parseInt(limit, 10) || 300));

  // A new result is fitted once the canvas has a size: it is drawn only after the run ends.
  const pendingFit = useRef(false);
  const show = useCallback((next: Graph, fixed: Record<string, Pos>, near: Pos | null, refit: boolean) => {
    const placed = layout(next.nodes.map((n) => n.id), next.relationships.map((r) => [r.start, r.end]), fixed, near);
    setGraph(next);
    setPositions(placed);
    if (refit) pendingFit.current = true;
  }, []);

  const execute = async () => {
    if (session === null || !text.trim() || running) return;
    const id = ++run.current;
    setRunning(true);
    setError(null);
    try {
      const loaded = await api.graphRun(session, text, MAX_ROWS, nodeLimit);
      if (id !== run.current) return;
      // The last statement that returned something is the one to look at.
      const index = Math.max(0, loaded.map((r) => r.columns.length > 0).lastIndexOf(true));
      setResults(loaded);
      setShown(index);
      setHidden(new Set());
      setSelected(null);
      const next = loaded[index]?.graph ?? { nodes: [], relationships: [] };
      if (next.nodes.length === 0 && (loaded[index]?.columns.length ?? 0) > 0) setMode((m) => (m === "graph" ? "table" : m));
      else if (next.nodes.length > 0) setMode((m) => (m === "text" ? m : "graph"));
      show(next, {}, null, true);
    } catch (e) {
      if (id !== run.current) return;
      const failed = dbError(e);
      setError({ message: errorMessage(e), code: failed.code, position: failed.position, line: failed.line });
      // The caret goes to the place the server named, as in the SQL editor.
      if (failed.position !== null && editor.current) {
        const at = Math.min(failed.position - 1, editor.current.value.length);
        editor.current.focus();
        editor.current.setSelectionRange(at, Math.min(at + 1, editor.current.value.length));
      }
    } finally {
      if (id === run.current) setRunning(false);
    }
  };

  useEffect(() => {
    const element = canvas.current;
    if (!element) return;
    const observer = new ResizeObserver(() => setSize({ width: element.clientWidth, height: element.clientHeight }));
    observer.observe(element);
    return () => observer.disconnect();
  }, [mode, results, running]);

  useEffect(() => {
    if (!pendingFit.current || size.width === 0 || size.height === 0) return;
    pendingFit.current = false;
    setView(fit(Object.values(positions), size.width, size.height));
  }, [positions, size]);

  // The selected node's relationships by type, for the side panel.
  useEffect(() => {
    setNeighbours(null);
    if (session === null || !selected) return;
    let live = true;
    api.graphNeighbours(session, selected, 1).then(
      (loaded) => live && setNeighbours(loaded),
      () => {},
    );
    return () => {
      live = false;
    };
  }, [session, selected]);

  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => setNotice(null), 4000);
    return () => clearTimeout(timer);
  }, [notice]);

  const expand = async (nodeId: string) => {
    if (session === null) return;
    try {
      const more = await api.graphNeighbours(session, nodeId, 50);
      const next = merge(graph, more);
      const added = next.nodes.length - graph.nodes.length;
      setHidden((h) => new Set([...h].filter((id) => !more.nodes.some((n) => n.id === id))));
      show(next, positions, positions[nodeId] ?? null, false);
      setNotice(added === 0 && more.total <= more.relationships.length ? t("Every neighbour is already shown.") : more.total > more.relationships.length ? t("{shown} of {total} relationships added", { shown: more.relationships.length, total: more.total }) : tn(added, "{count} node added", "{count} nodes added"));
    } catch (e) {
      setNotice(errorMessage(e));
    }
  };

  const labels = useMemo(() => {
    const order: string[] = [];
    for (const node of graph.nodes) {
      const name = node.labels[0] ?? "";
      if (!order.includes(name)) order.push(name);
    }
    return order;
  }, [graph.nodes]);
  const colourOf = (node: GraphNode) => `var(--c-${(Math.max(0, labels.indexOf(node.labels[0] ?? "")) % 4) + 1})`;
  const letters = useMemo(() => labelLetters(labels), [labels]);
  const letterOf = (name: string) => letters[name] ?? "·";

  const nodes = graph.nodes.filter((n) => !hidden.has(n.id) && positions[n.id]);
  const visible = new Set(nodes.map((n) => n.id));
  const relationships = graph.relationships.filter((r) => visible.has(r.start) && visible.has(r.end));
  const selectedNode = nodes.find((n) => n.id === selected) ?? null;

  // Dragging a node moves it, dragging the background pans; both follow the pointer until it is released.
  const drag = useRef<{ kind: "node"; id: string; moved: boolean } | { kind: "pan" } | null>(null);
  const onPointerMove = (e: ReactPointerEvent) => {
    const now = drag.current;
    if (!now) return;
    if (now.kind === "pan") setView((v) => ({ ...v, x: v.x + e.movementX, y: v.y + e.movementY }));
    else {
      now.moved = true;
      setPositions((p) => ({ ...p, [now.id]: { x: p[now.id].x + e.movementX / view.k, y: p[now.id].y + e.movementY / view.k } }));
    }
  };
  const zoom = (factor: number, at?: { x: number; y: number }) =>
    setView((v) => {
      const k = Math.max(0.1, Math.min(4, v.k * factor));
      const cx = at?.x ?? size.width / 2;
      const cy = at?.y ?? size.height / 2;
      return { k, x: cx - ((cx - v.x) / v.k) * k, y: cy - ((cy - v.y) / v.k) * k };
    });

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
    <main
      className="relative grid min-h-0 min-w-0 grid-cols-[minmax(0,1fr)] bg-panel"
      style={{ gridTemplateRows: `34px ${production ? "4px" : "0px"} 40px 104px minmax(0,1fr)` }}
      onKeyDown={(e) => {
        if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
          e.preventDefault();
          void execute();
        }
      }}
    >
      <TabBar />
      <div title={production ? t("Production connection") : undefined} className={production ? "production-band" : ""} />

      <div className="flex items-center gap-s3 border-b border-line px-[10.2px] whitespace-nowrap">
        <Button variant="primary" disabled={running || !text.trim()} onClick={() => void execute()}>
          {t("Run")} {MOD}↵
        </Button>
        <span className="font-mono text-[12px] text-text-2">
          {conn.name} / {conn.namespace ?? "neo4j"}
        </span>
        <span className="flex-1" />
        <label className="flex items-center gap-s2 text-[12px] text-text-2" title={t("The most nodes the graph draws. The table always has every row.")}>
          {t("Limit")}
          <input ref={limitField} inputMode="numeric" aria-label={t("Graph node limit")} className="h-[24px] w-[64px] border border-line bg-panel px-[6px] font-mono text-[12px] text-text focus:border-accent focus:outline-none" value={limit} onChange={(e) => setLimit(e.target.value)} />
        </label>
      </div>

      {/* The design asks for the SQL editor with Cypher colours; this is a plain field for now. */}
      <textarea
        ref={editor}
        aria-label={t("Cypher")}
        autoFocus
        spellCheck={false}
        placeholder="MATCH (n) RETURN n LIMIT 25"
        className="resize-none border-b border-line bg-panel p-s3 font-mono text-[13px] leading-[20px] focus:outline-none"
        value={text}
        onChange={(e) => setGraphText(tab.id, e.target.value)}
      />

      {running ? (
        <Loading text={t("Running…")} />
      ) : error ? (
        <div role="alert" className="flex flex-col gap-s2 overflow-auto px-s6 py-s4">
          <div className="flex items-center gap-s3">
            <TriangleAlert size={16} className="flex-none text-danger" />
            {error.code && <span className="selectable font-mono text-[12.5px] font-medium">{error.code}</span>}
          </div>
          <p className="selectable font-mono text-[13.5px] break-words whitespace-pre-wrap">{error.message}</p>
          {error.position !== null && <p className="text-[12.5px] text-text-2">{error.line !== null ? t("Line {line}, position {position}. The caret is on the place.", { line: error.line, position: error.position }) : t("Position {position}. The caret is on the place.", { position: error.position })}</p>}
        </div>
      ) : !result ? (
        <div className="grid place-items-center">
          <p className="text-text-2">{t("Run a statement to see its graph here.")}</p>
        </div>
      ) : (
        <div className="grid min-h-0 grid-rows-[34px_auto_minmax(0,1fr)]">
          <div className="flex items-center gap-s3 border-b border-line px-[10.2px] whitespace-nowrap">
            <Segmented<Mode>
              label={t("View")}
              value={mode}
              onChange={setMode}
              options={[
                { value: "graph", label: t("Graph") },
                { value: "table", label: t("Table") },
                { value: "text", label: t("Text") },
              ]}
            />
            {results && results.length > 1 && (
              <select aria-label={t("Statement")} className="h-[24px] border border-line bg-panel px-[6px] font-mono text-[12px]" value={shown} onChange={(e) => (setShown(Number(e.target.value)), setSelected(null), setHidden(new Set()), show(results[Number(e.target.value)].graph, {}, null, true))}>
                {results.map((_, i) => (
                  <option key={i} value={i}>
                    {t("Statement {n}", { n: i + 1 })}
                  </option>
                ))}
              </select>
            )}
            <span className="font-mono text-[12px] text-text-2">
              {[
                result.columns.length === 0 && result.counters ? changes(result.counters) : `${tn(result.node_count, "{count} node", "{count} nodes")} · ${tn(result.relationship_count, "{count} relationship", "{count} relationships")}`,
                `${result.elapsed_ms} ms`,
                result.truncated ? t("first {count} rows", { count: result.rows.length }) : null,
              ]
                .filter(Boolean)
                .join(" · ")}
            </span>
            <span className="flex-1" />
            <div className="flex min-w-0 items-center gap-s3 overflow-hidden">
              {labels.map((name, i) => (
                <span key={name} className="flex flex-none items-center gap-[5px] text-[12px]">
                  <span className="grid size-[14px] place-items-center rounded-full font-mono text-[8.5px] leading-none font-medium" style={{ border: `2px solid var(--c-${(i % 4) + 1})` }}>
                    {letterOf(name)}
                  </span>
                  {name || t("no label")}
                </span>
              ))}
            </div>
          </div>

          {result.nodes_omitted > 0 ? (
            <Strip kind="note" className="border-x-0 border-t-0">
              {t("Graph shows the first {shown} of {total} nodes. The table has every row.", { shown: result.graph.nodes.length, total: result.node_count })}{" "}
              <button type="button" className="text-accent-text hover:underline" onClick={() => (limitField.current?.focus(), limitField.current?.select())}>
                {t("Raise graph limit…")}
              </button>
            </Strip>
          ) : (
            <i />
          )}

          {mode === "table" ? (
            <ResultTable result={result} colour={(node) => `var(--c-${(Math.max(0, labels.indexOf(node.labels[0] ?? "")) % 4) + 1})`} />
          ) : mode === "text" ? (
            // Not drawn in the design: one row per line.
            <pre className="selectable min-h-0 overflow-auto p-s3 font-mono text-[12.5px] leading-[19px]">{result.rows.map((row) => row.map((cell, i) => `${result.columns[i]}: ${cellText(cell)}`).join("  ")).join("\n") || t("No rows")}</pre>
          ) : nodes.length === 0 && graph.nodes.length === 0 ? (
            <div className="grid place-items-center">
              <p className="text-text-2">{result.columns.length === 0 ? t("The statement ran and returned nothing.") : t("This result has no nodes. See the Table view.")}</p>
            </div>
          ) : (
            <div className="grid min-h-0" style={{ gridTemplateColumns: selectedNode ? "minmax(0,1fr) 320px" : "minmax(0,1fr)" }}>
              <div
                ref={canvas}
                className="relative min-h-0 cursor-grab overflow-hidden active:cursor-grabbing"
                onPointerDown={(e) => {
                  if ((e.target as Element).closest("[data-node],button")) return;
                  drag.current = { kind: "pan" };
                  e.currentTarget.setPointerCapture(e.pointerId);
                }}
                onPointerMove={onPointerMove}
                onPointerUp={() => (drag.current = null)}
                onWheel={(e) => {
                  const box = e.currentTarget.getBoundingClientRect();
                  zoom(e.deltaY < 0 ? 1.1 : 1 / 1.1, { x: e.clientX - box.left, y: e.clientY - box.top });
                }}
                onClick={(e) => !(e.target as Element).closest("[data-node],button") && setSelected(null)}
              >
                <svg width="100%" height="100%" role="img" aria-label={t("Graph")}>
                  <defs>
                    {["text-3", "accent"].map((name) => (
                      <marker key={name} id={`arrow-${name}`} viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
                        <path d="M0,1 L9,5 L0,9 z" fill={`var(--${name})`} />
                      </marker>
                    ))}
                  </defs>
                  <g transform={`translate(${view.x},${view.y}) scale(${view.k})`}>
                    {relationships.map((r) => {
                      const a = positions[r.start];
                      const b = positions[r.end];
                      const on = selected === r.start || selected === r.end;
                      const stroke = on ? "var(--accent)" : "var(--text-3)";
                      if (r.start === r.end) {
                        return (
                          <g key={r.id}>
                            <circle cx={a.x} cy={a.y - RADIUS - 10} r={12} fill="none" stroke={stroke} strokeWidth={on ? 1.5 : 1} />
                            <text x={a.x} y={a.y - RADIUS - 26} textAnchor="middle" className="font-mono" fontSize={9.5} fill="var(--text-2)">
                              {r.type}
                            </text>
                          </g>
                        );
                      }
                      const d = Math.hypot(b.x - a.x, b.y - a.y) || 1;
                      const ux = (b.x - a.x) / d;
                      const uy = (b.y - a.y) / d;
                      // Two relationships between the same pair bend away from each other.
                      const same = relationships.filter((o) => (o.start === r.start && o.end === r.end) || (o.start === r.end && o.end === r.start));
                      const index = same.indexOf(r);
                      const bend = same.length === 1 ? 0 : (index - (same.length - 1) / 2) * 40 * (r.start < r.end ? 1 : -1);
                      const mx = (a.x + b.x) / 2 - uy * bend;
                      const my = (a.y + b.y) / 2 + ux * bend;
                      const x1 = a.x + ux * (RADIUS + 2);
                      const y1 = a.y + uy * (RADIUS + 2);
                      const x2 = b.x - ux * (RADIUS + 4);
                      const y2 = b.y - uy * (RADIUS + 4);
                      const lx = (a.x + b.x) / 2 - uy * (bend / 2);
                      const ly = (a.y + b.y) / 2 + ux * (bend / 2);
                      return (
                        <g key={r.id}>
                          <path d={`M${x1},${y1} Q${mx},${my} ${x2},${y2}`} fill="none" stroke={stroke} strokeWidth={on ? 1.5 : 1} markerEnd={`url(#arrow-${on ? "accent" : "text-3"})`} />
                          <text x={lx} y={ly - 3} textAnchor="middle" className="font-mono" fontSize={9.5} fill="var(--text-2)" stroke="var(--panel)" strokeWidth={3} paintOrder="stroke">
                            {r.type}
                          </text>
                        </g>
                      );
                    })}
                    {nodes.map((node) => {
                      const p = positions[node.id];
                      const on = node.id === selected;
                      return (
                        <g
                          key={node.id}
                          data-node={node.id}
                          transform={`translate(${p.x},${p.y})`}
                          className="cursor-pointer"
                          onPointerDown={(e) => {
                            e.stopPropagation();
                            drag.current = { kind: "node", id: node.id, moved: false };
                            (e.currentTarget as Element).setPointerCapture(e.pointerId);
                          }}
                          onPointerMove={onPointerMove}
                          onPointerUp={() => {
                            if (drag.current?.kind === "node" && !drag.current.moved) setSelected(node.id);
                            drag.current = null;
                          }}
                          onDoubleClick={() => void expand(node.id)}
                        >
                          {on && <circle r={27} fill="var(--accent-tint)" stroke="var(--accent)" strokeWidth={2} />}
                          <circle r={RADIUS} fill="var(--panel)" stroke={colourOf(node)} strokeWidth={2.5} />
                          <text textAnchor="middle" dy="0.35em" className="font-mono select-none" fontSize={13} fontWeight={500} fill="var(--text)">
                            {letterOf(node.labels[0] ?? "")}
                          </text>
                          <text y={RADIUS + 15} textAnchor="middle" className="select-none" fontSize={11.5} fontWeight={on ? 500 : 400} fill="var(--text)" stroke="var(--panel)" strokeWidth={3} paintOrder="stroke">
                            {caption(node.properties).slice(0, 28)}
                          </text>
                        </g>
                      );
                    })}
                  </g>
                </svg>
                <div className="absolute bottom-s3 left-s3 flex items-center border border-line bg-bg text-[12px]">
                  <button type="button" aria-label={t("Zoom out")} className="grid size-[26px] place-items-center hover:bg-hover" onClick={() => zoom(1 / 1.2)}>
                    <Minus size={13} />
                  </button>
                  <span className="w-[44px] text-center font-mono text-[11.5px]">{Math.round(view.k * 100)}%</span>
                  <button type="button" aria-label={t("Zoom in")} className="grid size-[26px] place-items-center hover:bg-hover" onClick={() => zoom(1.2)}>
                    <Plus size={13} />
                  </button>
                  <button type="button" className="h-[26px] border-l border-line px-s2 hover:bg-hover" onClick={() => setView(fit(nodes.map((n) => positions[n.id]), size.width, size.height))}>
                    {t("Fit")}
                  </button>
                </div>
                {hidden.size > 0 && (
                  <button type="button" className={`${outline} absolute right-s3 bottom-s3 bg-bg`} onClick={() => setHidden(new Set())}>
                    {tn(hidden.size, "Show {count} hidden node", "Show {count} hidden nodes")}
                  </button>
                )}
              </div>

              {selectedNode && (
                <aside className="flex min-h-0 flex-col gap-s3 overflow-auto border-l border-line bg-bg p-s3" aria-label={t("Node")}>
                  <div className="flex items-center gap-s2">
                    {selectedNode.labels.map((name) => (
                      <span key={name} className="flex-none border px-[6px] py-[2px] font-mono text-[11px]" style={{ borderColor: colourOf(selectedNode) }}>
                        {name}
                      </span>
                    ))}
                    <span className="selectable min-w-0 flex-1 truncate font-mono text-[11.5px] text-text-2" title={selectedNode.id}>
                      {selectedNode.id}
                    </span>
                    <button type="button" aria-label={t("Close")} className="grid size-[22px] flex-none place-items-center text-text-3 hover:bg-hover hover:text-text" onClick={() => setSelected(null)}>
                      <X size={13} />
                    </button>
                  </div>
                  <span className={label}>
                    {t("Properties")} · {Object.keys(selectedNode.properties).length}
                  </span>
                  <dl className="grid grid-cols-[minmax(0,110px)_minmax(0,1fr)] gap-x-s3 gap-y-[5px] font-mono text-[12px]">
                    {Object.entries(selectedNode.properties).map(([key, value]) => (
                      <div key={key} className="contents">
                        <dt className="truncate text-text-3" title={key}>
                          {key}
                        </dt>
                        <dd className="selectable break-words" style={{ color: tone(value) }}>
                          {propertyText(value)}
                        </dd>
                      </div>
                    ))}
                  </dl>
                  <span className={label}>{t("Relationships")}</span>
                  {!neighbours ? (
                    <span className="text-[12px] text-text-3">…</span>
                  ) : neighbours.groups.length === 0 ? (
                    <span className="text-[12px] text-text-2">{t("None")}</span>
                  ) : (
                    <ul className="flex flex-col gap-[5px] font-mono text-[12px]">
                      {neighbours.groups.map((group) => {
                        const drawn = relationships.filter((r) => r.type === group.type && (group.outgoing ? r.start === selectedNode.id : r.end === selectedNode.id && r.start !== r.end)).length;
                        return (
                          <li key={`${group.type}-${group.outgoing}`} className="flex items-center gap-s2">
                            <span>{group.outgoing ? `–[:${group.type}]→` : `←[:${group.type}]–`}</span>
                            <span className="flex-1" />
                            <span className="text-text-2">{drawn < group.count ? t("{shown} shown of {total}", { shown: drawn, total: group.count }) : group.count}</span>
                          </li>
                        );
                      })}
                    </ul>
                  )}
                  <div className="flex flex-wrap gap-s2 pt-s2">
                    <button type="button" className={outline} onClick={() => void expand(selectedNode.id)}>
                      {t("Expand neighbours")}
                    </button>
                    <button type="button" className={outline} onClick={() => (setHidden(new Set([...hidden, selectedNode.id])), setSelected(null))}>
                      {t("Hide node")}
                    </button>
                    <button
                      type="button"
                      className={outline}
                      onClick={() => {
                        void navigator.clipboard?.writeText(`MATCH (n) WHERE elementId(n) = ${JSON.stringify(selectedNode.id)} RETURN n`);
                        setNotice(t("Copied"));
                      }}
                    >
                      {t("Copy as Cypher")}
                    </button>
                  </div>
                </aside>
              )}
            </div>
          )}
        </div>
      )}

      {notice && (
        <div role="status" className="absolute bottom-s4 left-1/2 z-40 max-w-[560px] -translate-x-1/2 border border-line bg-bg px-s3 py-s2 text-[12.5px]" style={{ boxShadow: "var(--shadow-lg)" }}>
          {notice}
        </div>
      )}
    </main>
  );
}

/** What a statement without columns changed, e.g. "2 nodes created · 1 properties set". */
function changes(counters: Record<string, number>): string {
  const n = (name: string) => counters[name] ?? 0;
  const parts = [
    n("nodes_created") > 0 && tn(n("nodes_created"), "{count} node created", "{count} nodes created"),
    n("nodes_deleted") > 0 && tn(n("nodes_deleted"), "{count} node deleted", "{count} nodes deleted"),
    n("relationships_created") > 0 && tn(n("relationships_created"), "{count} relationship created", "{count} relationships created"),
    n("relationships_deleted") > 0 && tn(n("relationships_deleted"), "{count} relationship deleted", "{count} relationships deleted"),
    n("properties_set") > 0 && tn(n("properties_set"), "{count} property set", "{count} properties set"),
    n("labels_added") > 0 && tn(n("labels_added"), "{count} label added", "{count} labels added"),
    n("labels_removed") > 0 && tn(n("labels_removed"), "{count} label removed", "{count} labels removed"),
    n("schema_changes") > 0 && tn(n("schema_changes"), "{count} schema change", "{count} schema changes"),
  ].filter((part): part is string => !!part);
  return parts.length > 0 ? parts.join(" · ") : t("No changes");
}

function cellText(cell: GraphCell): string {
  if (cell.kind === "value") return displayValue(cell.value);
  if (cell.kind === "node") return `(:${cell.labels.join(":")} ${JSON.stringify(cell.properties)})`;
  if (cell.kind === "relationship") return `[:${cell.type} ${JSON.stringify(cell.properties)}]`;
  return cell.nodes.map((n, i) => `(${caption(n.properties) || n.labels[0] || ""})${cell.relationships[i] ? `-[:${cell.relationships[i].type}]-` : ""}`).join("");
}

function ResultTable({ result, colour }: { result: CypherResult; colour(node: GraphNode): string }) {
  const chip = (node: GraphNode) => (
    <span className="flex items-center gap-[6px]">
      <span className="flex-none border px-[5px] py-[1px] font-mono text-[11px]" style={{ borderColor: colour(node) }}>
        {node.labels[0] ?? "·"}
      </span>
      <span className="selectable truncate">{caption(node.properties)}</span>
    </span>
  );
  if (result.columns.length === 0) {
    return (
      <div className="grid place-items-center">
        <p className="text-text-2">{result.counters ? changes(result.counters) : t("No rows")}</p>
      </div>
    );
  }
  return (
    <div className="min-h-0 overflow-auto">
      <table className="border-collapse text-[12.5px]">
        <thead>
          <tr>
            {result.columns.map((column) => (
              <th key={column} className="sticky top-0 z-10 h-row border-r border-b border-line bg-bg px-s2 text-left font-mono text-[12px] font-medium whitespace-nowrap">
                {column}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {result.rows.map((row, i) => (
            <tr key={i} className="hover:bg-hover">
              {row.map((cell, j) => (
                <td key={j} className="h-row max-w-[360px] truncate border-r border-b border-line px-s2 whitespace-nowrap" title={cell.kind === "value" ? undefined : cellText(cell)}>
                  {cell.kind === "node" ? chip(cell) : cell.kind === "relationship" ? <span className="font-mono">[:{cell.type}]</span> : cell.kind === "path" ? <span className="font-mono text-[12px]">{cellText(cell)}</span> : <span className={`selectable font-mono ${cell.value.type === "null" ? "text-text-3" : ""}`}>{displayValue(cell.value)}</span>}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
