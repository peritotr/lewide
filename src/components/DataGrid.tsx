// The data grid: read-only for query results, editable for a table when `edit` is given.
// Only the rows in view are in the DOM.
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDown, ArrowUp, ChevronDown, KeyRound } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { byteSize, displayValue, isNumeric, type ColumnDescription, type QueryResult, type SortColumn, type Value } from "../lib/api";
import { editorKind, editText, isJsonColumn, parseCell } from "../lib/edit";
import { groupDigits, usePrefs } from "../prefs";
import { CellViewer, rawText } from "./CellViewer";
import { DateEditor } from "./DateEditor";
import { anchorOf, Floating, MenuItem, MenuSeparator, type Anchor } from "./menu";
import { t } from "../i18n";

const ROW_NUMBER_WIDTH = 48;
const isMac = /Mac/i.test(navigator.userAgent);
const CHAR_WIDTH = 7.6; // IBM Plex Mono at 12.5px
const NUMERIC_TYPES = /int|numeric|decimal|float|double|real|serial|money|oid/i;

type Sort = { column: number; descending: boolean } | null;

function compare(a: Value, b: Value): number {
  if (a.type === "null" || b.type === "null") return a.type === b.type ? 0 : a.type === "null" ? 1 : -1;
  if (isNumeric(a) && isNumeric(b)) return Number(a.value) - Number(b.value);
  return displayValue(a).localeCompare(displayValue(b));
}

function columnIsNumeric(result: QueryResult, column: number): boolean {
  const type = result.columns[column].type_name;
  if (type) return NUMERIC_TYPES.test(type);
  const first = result.rows.find((row) => row[column].type !== "null");
  return first ? isNumeric(first[column]) : false;
}

/** Widths that fit the header and the first rows, between 64 and 420 pixels. */
function measure(result: QueryResult): number[] {
  return result.columns.map((column, c) => {
    let chars = column.name.length + (column.type_name?.length ?? 0) + 3;
    for (const row of result.rows.slice(0, 100)) {
      chars = Math.max(chars, Math.min(displayValue(row[c]).length, 48));
    }
    return Math.min(420, Math.max(64, Math.round(chars * CHAR_WIDTH + 22)));
  });
}

/** What the gutter and the cells of one row show about its pending change. */
export interface RowMeta {
  mark: "+" | "~" | "−" | null;
  /** Per column: the value differs from what was loaded. */
  changed: boolean[] | null;
  /** The loaded values of a modified row, for the tooltip. */
  original: Value[] | null;
  /** Per column of an added row: left to the database's default. */
  unset: boolean[] | null;
  /** The row's number, or null for a row that is not in the table yet. */
  number: number | null;
}

/** Editing, for a table's rows. Committing a cell only records a pending change. */
export interface GridEdit {
  columns: ColumnDescription[];
  meta: RowMeta[];
  onChange(row: number, column: number, value: Value): void;
  onUndo(): void;
  onActiveRow(row: number | null): void;
}

type Editing = { row: number; column: number; draft: string; error: string | null; long: boolean; list?: boolean; date?: boolean };

/** Row selection by the gutter: click, Shift-click for a range, ⌘/Ctrl-click to toggle. */
export interface GridSelection {
  selected: ReadonlySet<number>;
  onChange(selected: Set<number>): void;
}

/** What the cell and header menus can do beyond copying; an item is left out when its handler is. */
export interface GridActions {
  /** A cell that points at another table's row (a foreign key) was ⌘/Ctrl-clicked, or hovered for a moment. */
  onFollowLink?(column: string, value: Value): void;
  onPeekLink?(column: string, value: Value, anchor: Anchor): void;
  onFilterValue?(column: string, value: Value): void;
  onFilterColumn?(column: string): void;
  onHideColumn?(column: string): void;
}

interface Props {
  result: QueryResult;
  edit?: GridEdit;
  selection?: GridSelection;
  actions?: GridActions;
  /** Per column: its values are keys of another table's rows. */
  linked?: boolean[];
  /** The line under the cell viewer, e.g. "Read-only · query result". */
  note?: string;
  /** Number of the first row, for pages of a larger table. */
  firstRow?: number;
  /** Names of primary key columns, marked in the header. */
  keys?: string[];
  /**
   * When set, sorting is done by the caller (the database, for a paged table)
   * and `serverSort` says what is applied. Otherwise the loaded rows are sorted here.
   */
  onSort?(sort: SortColumn | null): void;
  serverSort?: SortColumn | null;
}

export function DataGrid({ result, firstRow = 1, keys = [], onSort, serverSort = null, edit, selection, actions, linked, note }: Props) {
  const hover = useRef<number | null>(null);
  const [cellMenu, setCellMenu] = useState<{ anchor: Anchor; row: number; column: number } | null>(null);
  const [headerMenu, setHeaderMenu] = useState<{ anchor: Anchor; column: number } | null>(null);
  const [viewer, setViewer] = useState<{ anchor: Anchor; row: number; column: number } | null>(null);
  const lastPicked = useRef<number | null>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const nullStyle = usePrefs((s) => s.nullStyle);
  const grouped = usePrefs((s) => s.numbers === "grouped");
  const [widths, setWidths] = useState(() => measure(result));
  const [resized, setResized] = useState(false);
  const [available, setAvailable] = useState(0);
  const [localSort, setLocalSort] = useState<Sort>(null);
  const serverColumn = serverSort ? result.columns.findIndex((c) => c.name === serverSort.column) : -1;
  const sort: Sort = onSort ? (serverColumn >= 0 ? { column: serverColumn, descending: serverSort!.descending } : null) : localSort;
  const [cell, setCellState] = useState<{ row: number; column: number } | null>(null);
  const [editing, setEditing] = useState<Editing | null>(null);
  const [anchor, setAnchor] = useState<DOMRect | null>(null);
  const setCell = (next: { row: number; column: number } | null) => {
    setCellState(next);
    edit?.onActiveRow(next ? next.row : null);
  };

  const numeric = useMemo(() => result.columns.map((_, c) => columnIsNumeric(result, c)), [result]);
  // Free text (sentences) reads better in the interface font; identifiers and codes stay mono.
  const prose = useMemo(
    () =>
      result.columns.map((column, c) => {
        if (!/text|char|clob|string/i.test(column.type_name ?? "")) return false;
        const sample = result.rows.slice(0, 50).map((row) => row[c]).filter((v): v is Extract<Value, { type: "text" }> => v.type === "text");
        return sample.length > 0 && sample.filter((v) => v.value.trim().includes(" ")).length * 2 > sample.length;
      }),
    [result],
  );
  const copy = (text: string) => void navigator.clipboard?.writeText(text);
  const cellAnchor = (row: number, column: number): Anchor | null => scroller.current?.querySelector(`[data-cell="${row}:${column}"]`)?.getBoundingClientRect() ?? null;
  const openViewer = (row: number, column: number) => {
    const anchor = cellAnchor(row, column);
    if (anchor) setViewer({ anchor, row, column });
  };
  const order = useMemo(() => {
    const indices = result.rows.map((_, i) => i);
    if (sort && !onSort) {
      const direction = sort.descending ? -1 : 1;
      indices.sort((x, y) => direction * compare(result.rows[x][sort.column], result.rows[y][sort.column]) || x - y);
    }
    return indices;
  }, [result, sort, onSort]);

  useEffect(() => {
    const element = scroller.current;
    if (!element) return;
    const observer = new ResizeObserver(() => setAvailable(element.clientWidth));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  // Until the user resizes a column, spare width goes to the first text column, as in the design.
  const layout = useMemo(() => {
    const total = ROW_NUMBER_WIDTH + widths.reduce((a, b) => a + b, 0);
    if (resized || total >= available) return widths;
    const flexible = Math.max(0, numeric.findIndex((n) => !n));
    return widths.map((w, i) => (i === flexible ? w + available - total : w));
  }, [widths, resized, available, numeric]);
  const template = `${ROW_NUMBER_WIDTH}px ${layout.map((w) => `${w}px`).join(" ")}`;
  const totalWidth = ROW_NUMBER_WIDTH + layout.reduce((a, b) => a + b, 0);

  const rowHeight = () => parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--row-h")) || 28;
  const virtual = useVirtualizer({
    count: order.length,
    getScrollElement: () => scroller.current,
    estimateSize: rowHeight,
    overscan: 12,
  });

  // Ascending, descending, then back to the natural order.
  function cycleSort(column: number) {
    const next: Sort = sort?.column !== column ? { column, descending: false } : sort.descending ? null : { column, descending: true };
    if (onSort) onSort(next && { column: result.columns[next.column].name, descending: next.descending });
    else setLocalSort(next);
  }

  const editable = (row: number, column: number) => !!edit && edit.meta[row]?.mark !== "−" && editorKind(edit.columns[column], result.rows[row][column]) !== "none";

  function startEdit(row: number, column: number, typed?: string) {
    if (!edit || !editable(row, column)) return;
    const value = result.rows[row][column];
    const kind = editorKind(edit.columns[column], value);
    if (kind === "bool" && typed === undefined) {
      edit.onChange(row, column, { type: "bool", value: !(value.type === "bool" && value.value) });
      return;
    }
    const unset = edit.meta[row]?.unset?.[column];
    const long = kind === "long";
    // A date or time opens in its parts, unless something is typed to replace it.
    if (kind === "date" && typed === undefined) {
      const element = scroller.current?.querySelector(`[data-cell="${row}:${column}"]`);
      setAnchor(element?.getBoundingClientRect() ?? null);
      setEditing({ row, column, draft: unset ? "" : editText(value), error: null, long: true, date: true });
      return;
    }
    if (kind === "enum") {
      const element = scroller.current?.querySelector(`[data-cell="${row}:${column}"]`);
      setAnchor(element?.getBoundingClientRect() ?? null);
      setEditing({ row, column, draft: typed ?? "", error: null, long: true, list: true });
      return;
    }
    if (long) {
      const element = scroller.current?.querySelector(`[data-cell="${row}:${column}"]`);
      setAnchor(element?.getBoundingClientRect() ?? null);
    }
    setEditing({ row, column, draft: typed ?? (unset ? "" : editText(value)), error: null, long });
  }

  /** Records the draft as a pending change. False when it is not a value of the column's type. */
  function commit(move: [number, number] | null): boolean {
    if (!edit || !editing) return true;
    const { row, column, draft } = editing;
    const parsed = parseCell(draft, edit.columns[column], result.rows[row][column]);
    if ("error" in parsed) {
      setEditing({ ...editing, error: parsed.error });
      return false;
    }
    edit.onChange(row, column, parsed.value);
    setEditing(null);
    scroller.current?.focus();
    if (move) {
      const next = { row: Math.min(order.length - 1, row + move[0]), column: Math.min(result.columns.length - 1, column + move[1]) };
      setCell(next);
      virtual.scrollToIndex(next.row);
    }
    return true;
  }

  function cancelEdit() {
    setEditing(null);
    scroller.current?.focus();
  }

  function setNull(row: number, column: number) {
    if (!edit || !editable(row, column)) return;
    if (!edit.columns[column].nullable) return;
    edit.onChange(row, column, { type: "null" });
    setEditing(null);
    scroller.current?.focus();
  }

  function onKeyDown(e: React.KeyboardEvent) {
    if (editing) return;
    const mod = e.metaKey || e.ctrlKey;
    if (selection && mod && e.key === "a") {
      e.preventDefault();
      selection.onChange(new Set(order.map((_, i) => i)));
      return;
    }
    if (edit && mod && !e.shiftKey && e.key === "z") {
      e.preventDefault();
      edit.onUndo();
      return;
    }
    if (!cell) return;
    if (mod && e.shiftKey && e.key.toLowerCase() === "c") {
      e.preventDefault();
      copy(result.rows[order[cell.row]].map(rawText).join("\t"));
      return;
    }
    if (e.key === " " && !(edit && editorKind(edit.columns[cell.column], result.rows[cell.row][cell.column]) === "bool" && editable(cell.row, cell.column))) {
      e.preventDefault();
      openViewer(cell.row, cell.column);
      return;
    }
    if (edit) {
      if (mod && (e.key === "Backspace" || e.key === "Delete")) {
        e.preventDefault();
        setNull(cell.row, cell.column);
        return;
      }
      if (e.key === "Enter" || e.key === "F2") {
        e.preventDefault();
        startEdit(cell.row, cell.column, editorKind(edit.columns[cell.column], result.rows[cell.row][cell.column]) === "bool" ? "" : undefined);
        return;
      }
      if (e.key === " " && editorKind(edit.columns[cell.column], result.rows[cell.row][cell.column]) === "bool") {
        e.preventDefault();
        startEdit(cell.row, cell.column);
        return;
      }
      // Typing replaces the value.
      if (e.key.length === 1 && e.key !== " " && !mod && !e.altKey && ["text", "enum", "date"].includes(editorKind(edit.columns[cell.column], result.rows[cell.row][cell.column]))) {
        e.preventDefault();
        startEdit(cell.row, cell.column, e.key);
        return;
      }
    }
    if ((e.metaKey || e.ctrlKey) && e.key === "c") {
      const value = result.rows[order[cell.row]][cell.column];
      void navigator.clipboard?.writeText(value.type === "null" ? "" : displayValue(value));
      return;
    }
    const step: Record<string, [number, number]> = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] };
    const move = step[e.key];
    if (!move) return;
    e.preventDefault();
    const row = Math.min(order.length - 1, Math.max(0, cell.row + move[0]));
    const column = Math.min(result.columns.length - 1, Math.max(0, cell.column + move[1]));
    setCell({ row, column });
    virtual.scrollToIndex(row);
  }

  return (
    <div ref={scroller} role="grid" tabIndex={0} onKeyDown={onKeyDown} className="min-h-0 overflow-auto focus:outline-none">
      <div className="sticky top-0 z-[3] grid h-row items-center border-b border-line bg-bg whitespace-nowrap" style={{ gridTemplateColumns: template, minWidth: totalWidth }}>
        <div className="sticky left-0 z-[1] h-full border-r border-line bg-bg" />
        {result.columns.map((column, c) => {
          const sorted = sort?.column === c ? sort : null;
          const arrow = sorted && (sorted.descending ? <ArrowDown size={12} className="text-accent-text" /> : <ArrowUp size={12} className="text-accent-text" />);
          return (
            <div
              key={c}
              role="columnheader"
              aria-sort={sorted ? (sorted.descending ? "descending" : "ascending") : "none"}
              className={`group/h relative flex h-full cursor-pointer items-center gap-[6px] overflow-hidden px-[10px] hover:bg-hover ${
                c < result.columns.length - 1 ? "border-r border-line" : ""
              } ${numeric[c] ? "justify-end" : ""} ${headerMenu?.column === c ? "bg-accent-tint" : ""}`}
              onClick={() => cycleSort(c)}
            >
              {keys.includes(column.name) && <KeyRound size={12} className={`flex-none text-accent-text ${numeric[c] ? "mr-auto" : ""}`} aria-label={t("Primary key")} />}
              {numeric[c] && arrow}
              {numeric[c] && column.type_name && <span className="font-mono text-[11px] text-text-3">{column.type_name}</span>}
              <span className="truncate font-mono text-[12px] font-medium">{column.name}</span>
              {!numeric[c] && column.type_name && <span className="font-mono text-[11px] text-text-3">{column.type_name}</span>}
              {!numeric[c] && arrow}
              <button
                type="button"
                aria-label={t("Column menu for {column}", { column: column.name })}
                aria-haspopup="menu"
                className={`absolute top-0 right-[5px] bottom-0 grid w-[18px] place-items-center bg-bg text-text-2 hover:bg-hover ${headerMenu?.column === c ? "" : "opacity-0 group-hover/h:opacity-100 focus-visible:opacity-100"}`}
                onClick={(e) => {
                  e.stopPropagation();
                  setHeaderMenu({ anchor: anchorOf(e.currentTarget.parentElement!), column: c });
                }}
              >
                <ChevronDown size={12} />
              </button>
              <i
                role="separator"
                className="absolute top-0 right-0 bottom-0 w-[5px] cursor-col-resize"
                onClick={(e) => e.stopPropagation()}
                onPointerDown={(e) => {
                  e.preventDefault();
                  e.currentTarget.setPointerCapture(e.pointerId);
                  // Freeze the current layout so only the dragged column changes.
                  setWidths(layout);
                  setResized(true);
                }}
                onPointerMove={(e) => {
                  if (!e.currentTarget.hasPointerCapture(e.pointerId)) return;
                  setWidths((w) => w.map((width, i) => (i === c ? Math.max(48, width + e.movementX) : width)));
                }}
              />
            </div>
          );
        })}
      </div>

      <div className="relative font-mono text-[12.5px] tabular-nums" style={{ height: virtual.getTotalSize(), minWidth: totalWidth }}>
        {virtual.getVirtualItems().map((item) => {
          const row = result.rows[order[item.index]];
          const selected = cell?.row === item.index;
          const meta = edit?.meta[item.index] ?? null;
          const picked = selection?.selected.has(item.index) ?? false;
          const tint = meta?.mark === "+" ? "bg-ok-tint" : meta?.mark === "−" ? "bg-danger-tint" : "";
          const markColour = meta?.mark === "+" ? "text-ok" : meta?.mark === "−" ? "text-danger" : "text-accent-text";
          const editingRow = editing?.row === item.index;
          return (
            <div
              key={item.key}
              role="row"
              className={`absolute top-0 left-0 grid h-row w-full border-b border-line-2 leading-[var(--row-h)] whitespace-nowrap hover:bg-hover ${tint} ${editingRow ? "z-[2]" : ""}`}
              style={{ gridTemplateColumns: template, transform: `translateY(${item.start}px)` }}
            >
              {selected && <i className="pointer-events-none absolute inset-0 bg-accent-tint" />}
              {meta?.mark === "−" && <i className="pointer-events-none absolute top-1/2 right-0 z-[1] h-px bg-danger" style={{ left: ROW_NUMBER_WIDTH }} />}
              {picked && <i className="pointer-events-none absolute inset-0 bg-accent-tint" />}
              <div
                className={`sticky left-0 z-[1] flex justify-between border-r border-line bg-bg px-[8px] text-[11.5px] text-text-3 ${selection ? "cursor-pointer hover:bg-hover" : ""}`}
                onMouseDown={(e) => {
                  if (!selection) return;
                  e.preventDefault();
                  const next = new Set(selection.selected);
                  if (e.shiftKey && lastPicked.current !== null) {
                    const [from, to] = [Math.min(lastPicked.current, item.index), Math.max(lastPicked.current, item.index)];
                    for (let i = from; i <= to; i++) next.add(i);
                  } else if (e.metaKey || e.ctrlKey) {
                    if (next.has(item.index)) next.delete(item.index);
                    else next.add(item.index);
                  } else {
                    const only = next.size === 1 && next.has(item.index);
                    next.clear();
                    if (!only) next.add(item.index);
                  }
                  lastPicked.current = item.index;
                  selection.onChange(next);
                }}
              >
                <span className={`text-[13px] font-medium ${markColour}`} title={meta?.mark === "+" ? t("Added row") : meta?.mark === "~" ? t("Modified row") : meta?.mark === "−" ? t("Marked for deletion") : undefined}>
                  {meta?.mark ?? ""}
                </span>
                <span className="flex items-center gap-[4px]">
                  {picked && <i className="grid size-[10px] place-items-center bg-accent-fill text-[8px] leading-none text-on-accent">✓</i>}
                  {meta ? (meta.number ?? "") : firstRow + item.index}
                </span>
              </div>
              {row.map((value, c) => {
                const active = selected && cell?.column === c;
                const changed = meta?.changed?.[c] ?? false;
                const unset = meta?.unset?.[c] ?? false;
                const inPlace = editingRow && editing!.column === c && !editing!.long;
                const link = !!linked?.[c] && value.type !== "null" && meta?.mark !== "+";
                return (
                  <div
                    key={c}
                    role="gridcell"
                    data-cell={`${item.index}:${c}`}
                    title={changed && meta?.original ? t("Was {value}", { value: displayValue(meta.original[c]) }) : undefined}
                    className={`relative px-[10px] ${inPlace ? "" : "truncate"} ${c < row.length - 1 ? "border-r border-line-2" : ""} ${numeric[c] ? "text-right" : ""} ${
                      value.type === "temporal" ? "text-text-2" : ""
                    } ${changed ? "bg-accent-tint" : ""} ${link ? "text-accent-text" : ""}`}
                    onMouseDown={(e) => {
                      if (editing && !(editing.row === item.index && editing.column === c) && !commit(null)) return;
                      setCell({ row: item.index, column: c });
                      if (link && (e.metaKey || e.ctrlKey)) {
                        e.preventDefault();
                        actions?.onFollowLink?.(result.columns[c].name, value);
                      }
                    }}
                    onMouseEnter={(e) => {
                      if (!link || !actions?.onPeekLink) return;
                      const element = e.currentTarget;
                      hover.current = window.setTimeout(() => actions.onPeekLink!(result.columns[c].name, value, element.getBoundingClientRect()), 400);
                    }}
                    onMouseLeave={() => {
                      if (hover.current !== null) window.clearTimeout(hover.current);
                      hover.current = null;
                    }}
                    onDoubleClick={() => (editable(item.index, c) ? startEdit(item.index, c, edit && editorKind(edit.columns[c], value) === "bool" ? "" : undefined) : openViewer(item.index, c))}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      setCell({ row: item.index, column: c });
                      setCellMenu({ anchor: { x: e.clientX, y: e.clientY }, row: item.index, column: c });
                    }}
                  >
                    {unset ? (
                      <span className="text-text-3 italic">default</span>
                    ) : value.type === "null" ? (
                      nullStyle === "tag" ? (
                        <span className="border border-dashed border-line px-[4px] text-[11px] text-text-3 italic">NULL</span>
                      ) : nullStyle === "dimmed" ? (
                        <span className="text-text-3">NULL</span>
                      ) : null
                    ) : value.type === "bool" ? (
                      <span className="inline-flex items-center gap-[6px]">
                        <i className={`size-[8px] flex-none ${value.value ? "bg-text-2" : "border border-text-3"}`} />
                        {String(value.value)}
                      </span>
                    ) : value.type === "bytes" ? (
                      <span className="inline-flex items-center gap-[6px]">
                        <span className="border border-line px-[4px] font-heading text-[10px] leading-[14px] font-semibold tracking-[.08em] text-text-2">BIN</span>
                        <span className="text-text-2">{byteSize(value.value)}</span>
                      </span>
                    ) : grouped && isNumeric(value) ? (
                      groupDigits(displayValue(value))
                    ) : prose[c] ? (
                      <span className="font-body text-[13px]">{displayValue(value)}</span>
                    ) : (
                      displayValue(value)
                    )}
                    {changed && <i className="pointer-events-none absolute top-0 left-0 border-t-[6px] border-r-[6px] border-t-accent border-r-transparent" />}
                    {active && !inPlace && <i className="pointer-events-none absolute inset-0 border-2 border-accent" />}
                    {inPlace && (
                      <>
                        <input
                          autoFocus
                          value={editing!.draft}
                          spellCheck={false}
                          aria-label={result.columns[c].name}
                          aria-invalid={editing!.error !== null}
                          onChange={(e) => setEditing({ ...editing!, draft: e.target.value, error: null })}
                          onFocus={(e) => (editing!.draft.length > 1 ? e.currentTarget.select() : undefined)}
                          onKeyDown={(e) => {
                            e.stopPropagation();
                            if (e.key === "Enter") {
                              e.preventDefault();
                              commit([1, 0]);
                            } else if (e.key === "Tab") {
                              e.preventDefault();
                              commit([0, e.shiftKey ? -1 : 1]);
                            } else if (e.key === "Escape") {
                              e.preventDefault();
                              cancelEdit();
                            } else if ((e.metaKey || e.ctrlKey) && e.key === "Backspace") {
                              e.preventDefault();
                              setNull(item.index, c);
                            }
                          }}
                          className={`absolute inset-0 border-2 bg-panel px-[8px] font-mono text-[12.5px] text-text outline-none ${numeric[c] ? "text-right" : ""} ${editing!.error ? "border-danger" : "border-accent"}`}
                        />
                        {editing!.error && (
                          <div role="alert" className="absolute top-full left-0 z-[5] mt-[2px] w-max max-w-[420px] border border-danger bg-panel px-s3 py-s2 text-left font-body text-[12.5px] leading-[1.4] whitespace-normal text-text" style={{ boxShadow: "var(--shadow-md)" }}>
                            {editing!.error}{" "}
                            <span className="text-text-2">{t("Esc restores {value}.", { value: displayValue(value) })}</span>
                          </div>
                        )}
                      </>
                    )}
                  </div>
                );
              })}
            </div>
          );
        })}
      {cellMenu &&
        (() => {
          const { row, column } = cellMenu;
          const value = result.rows[order[row]][column];
          const name = result.columns[column].name;
          const meta = edit?.meta[row];
          const close = () => setCellMenu(null);
          const item = (run: () => void) => () => {
            close();
            run();
          };
          return (
            <Floating anchor={cellMenu.anchor} width={230} onClose={close} label={name}>
              <MenuItem hint={`${isMac ? "⌘" : "Ctrl+"}C`} onSelect={item(() => copy(rawText(value)))}>
                {t("Copy")}
              </MenuItem>
              <MenuItem onSelect={item(() => copy(JSON.stringify(value.type === "null" ? null : value.type === "json" ? value.value : value.type === "bool" || value.type === "float" ? value.value : value.type === "int" ? (Number.isSafeInteger(Number(value.value)) ? Number(value.value) : value.value) : rawText(value))))}>
                {t("Copy as JSON")}
              </MenuItem>
              <MenuItem hint={isMac ? "⇧⌘C" : "Ctrl+Shift+C"} onSelect={item(() => copy(result.rows[order[row]].map(rawText).join("\t")))}>
                {t("Copy row")}
              </MenuItem>
              <MenuSeparator />
              <MenuItem hint="Space" onSelect={item(() => openViewer(row, column))}>
                {t("View value")}
              </MenuItem>
              {actions?.onFilterValue && <MenuItem onSelect={item(() => actions.onFilterValue!(name, value))}>{t("Filter by value")}</MenuItem>}
              {edit && editable(row, column) && (
                <>
                  <MenuSeparator />
                  <MenuItem onSelect={item(() => startEdit(row, column, editorKind(edit.columns[column], value) === "bool" ? "" : undefined))}>{t("Edit the cell")}</MenuItem>
                  <MenuItem disabled={!edit.columns[column].nullable || value.type === "null"} onSelect={item(() => setNull(row, column))}>
                    {t("Set NULL")}
                  </MenuItem>
                  {/* Not drawn in the design: takes one cell back to the loaded value. */}
                  {meta?.changed?.[column] && meta.original && <MenuItem onSelect={item(() => edit.onChange(row, column, meta.original![column]))}>{t("Revert change")}</MenuItem>}
                </>
              )}
            </Floating>
          );
        })()}
      {headerMenu &&
        (() => {
          const c = headerMenu.column;
          const name = result.columns[c].name;
          const close = () => setHeaderMenu(null);
          const item = (run: () => void) => () => {
            close();
            run();
          };
          const sortBy = (descending: boolean) => (onSort ? onSort({ column: name, descending }) : setLocalSort({ column: c, descending }));
          return (
            <Floating anchor={headerMenu.anchor} width={240} onClose={close} label={name}>
              <MenuItem checked={sort?.column === c && !sort.descending} onSelect={item(() => sortBy(false))}>
                {t("Sort ascending")}
              </MenuItem>
              <MenuItem checked={sort?.column === c && sort.descending} onSelect={item(() => sortBy(true))}>
                {t("Sort descending")}
              </MenuItem>
              <MenuSeparator />
              <MenuItem onSelect={item(() => copy(name))}>{t("Copy column name")}</MenuItem>
              <MenuItem onSelect={item(() => copy(order.map((i) => rawText(result.rows[i][c])).join("\n")))}>{t("Copy column values")}</MenuItem>
              <MenuSeparator />
              {actions?.onFilterColumn && <MenuItem onSelect={item(() => actions.onFilterColumn!(name))}>{t("Filter this column…")}</MenuItem>}
              {actions?.onHideColumn && (
                <MenuItem disabled={result.columns.length === 1} onSelect={item(() => actions.onHideColumn!(name))}>
                  {t("Hide column")}
                </MenuItem>
              )}
              <MenuItem
                onSelect={item(() => {
                  const fit = measure(result)[c];
                  setWidths(layout.map((w, i) => (i === c ? fit : w)));
                  setResized(true);
                })}
              >
                {t("Size to fit")}
              </MenuItem>
            </Floating>
          );
        })()}
      {viewer && (
        <CellViewer
          anchor={viewer.anchor}
          column={result.columns[viewer.column].name}
          typeName={result.columns[viewer.column].type_name}
          row={edit?.meta[viewer.row]?.number ?? firstRow + viewer.row}
          value={result.rows[order[viewer.row]][viewer.column]}
          note={note ?? t("Read-only · query result")}
          onClose={() => {
            setViewer(null);
            scroller.current?.focus();
          }}
        />
      )}
      {edit && editing?.list && anchor && (
        <EnumEditor
          anchor={anchor}
          column={edit.columns[editing.column]}
          current={result.rows[editing.row][editing.column]}
          filter={editing.draft}
          onFilter={(draft) => setEditing({ ...editing, draft })}
          onPick={(value) => {
            edit.onChange(editing.row, editing.column, { type: "text", value });
            cancelEdit();
          }}
          onNull={() => setNull(editing.row, editing.column)}
          onCancel={cancelEdit}
        />
      )}
      {edit && editing?.date && anchor && (
        <DateEditor
          anchor={anchor}
          column={edit.columns[editing.column]}
          text={editing.draft}
          onDone={(text) => {
            edit.onChange(editing.row, editing.column, { type: "temporal", value: text });
            cancelEdit();
          }}
          onNull={() => setNull(editing.row, editing.column)}
          onCancel={cancelEdit}
        />
      )}
      {edit && editing?.long && !editing.list && !editing.date && anchor && (
        <LongEditor
          anchor={anchor}
          column={edit.columns[editing.column]}
          json={isJsonColumn(edit.columns[editing.column], result.rows[editing.row][editing.column])}
          rowLabel={edit.meta[editing.row]?.number}
          draft={editing.draft}
          error={editing.error}
          onDraft={(draft) => setEditing({ ...editing, draft, error: null })}
          onDone={() => commit(null)}
          onNull={() => setNull(editing.row, editing.column)}
          onCancel={cancelEdit}
        />
      )}
      </div>
    </div>
  );
}

/** T08: JSON and long text are edited in a panel under the cell. */
function LongEditor(props: {
  anchor: DOMRect;
  column: ColumnDescription;
  json: boolean;
  rowLabel: number | null | undefined;
  draft: string;
  error: string | null;
  onDraft(draft: string): void;
  onDone(): void;
  onNull(): void;
  onCancel(): void;
}) {
  let invalid: string | null = null;
  if (props.json) {
    try {
      JSON.parse(props.draft);
    } catch (e) {
      invalid = e instanceof Error ? e.message : String(e);
    }
  }
  const lines = props.draft.split("\n").length;
  const small = "h-[26px] px-s3 text-[12.5px] enabled:hover:bg-hover disabled:cursor-not-allowed disabled:opacity-45";
  return (
    <Floating anchor={props.anchor} width={460} onClose={props.onCancel} role="dialog" label={props.column.name} className="">
      <div className="flex h-[34px] items-center gap-s3 border-b border-line px-s3 text-[12px] whitespace-nowrap">
        <span className="font-mono font-medium">{props.column.name}</span>
        <span className="font-mono text-[11px] text-text-3">{props.column.type_name}</span>
        {props.rowLabel != null && <span className="text-text-3">{t("Row {number}", { number: props.rowLabel })}</span>}
        <span className="flex-1" />
        {props.json && (
          <>
            <span className={`min-w-0 truncate ${invalid ? "text-danger" : "text-text-2"}`} title={invalid ?? undefined}>
              {invalid ? t("Invalid JSON") : t("Valid JSON")}
            </span>
            <button type="button" className="text-accent-text hover:underline disabled:opacity-45" disabled={!!invalid} onClick={() => props.onDraft(JSON.stringify(JSON.parse(props.draft), null, 2))}>
              {t("Format")}
            </button>
          </>
        )}
      </div>
      <div className="grid max-h-[280px] grid-cols-[36px_minmax(0,1fr)] overflow-auto bg-bg py-s2 font-mono text-[12.5px] leading-[20px]">
        <div aria-hidden className="pr-s2 text-right text-text-3 select-none">
          {Array.from({ length: lines }, (_, i) => (
            <div key={i}>{i + 1}</div>
          ))}
        </div>
        <textarea
          autoFocus
          value={props.draft}
          spellCheck={false}
          rows={Math.max(6, lines)}
          aria-label={props.column.name}
          onChange={(e) => props.onDraft(e.target.value)}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === "Escape") props.onCancel();
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && !invalid) props.onDone();
          }}
          className="resize-none bg-transparent pr-s3 whitespace-pre outline-none"
        />
      </div>
      {props.error && <p role="alert" className="border-t border-danger px-s3 py-s2 text-[12px] text-danger">{props.error}</p>}
      <div className="flex h-[40px] items-center gap-s2 border-t border-line px-s3">
        <button type="button" className={small} disabled={!props.column.nullable} title={props.column.nullable ? undefined : t("NOT NULL column")} onClick={props.onNull}>
          {t("Set NULL")}
        </button>
        <span className="flex-1" />
        <button type="button" className={small} onClick={props.onCancel}>
          {t("Cancel")}
        </button>
        <button type="button" className={`${small} border border-line font-medium`} disabled={!!invalid} onClick={props.onDone}>
          {t("Done")} <span className="font-mono text-[11px] text-text-3">{isMac ? "⌘↵" : "Ctrl+Enter"}</span>
        </button>
      </div>
    </Floating>
  );
}

/** T06: the values of an enumeration, under the cell. Typing filters; the last row sets NULL. */
function EnumEditor(props: { anchor: DOMRect; column: ColumnDescription; current: Value; filter: string; onFilter(filter: string): void; onPick(value: string): void; onNull(): void; onCancel(): void }) {
  const all = props.column.enum_values ?? [];
  const shown = all.filter((value) => value.toLowerCase().includes(props.filter.toLowerCase()));
  const currentText = props.current.type === "null" ? null : displayValue(props.current);
  const [active, setActive] = useState(() => Math.max(0, shown.indexOf(currentText ?? "")));
  const index = Math.min(active, Math.max(0, shown.length - 1));
  return (
    <Floating anchor={props.anchor} width={Math.max(190, props.anchor.width)} onClose={props.onCancel} role="listbox" label={props.column.name} className="">
      <input
        autoFocus
        value={props.filter}
        spellCheck={false}
        aria-label={t("Filter values")}
        placeholder={t("Type to filter")}
        onChange={(e) => {
          props.onFilter(e.target.value);
          setActive(0);
        }}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "ArrowDown") {
            e.preventDefault();
            setActive(Math.min(shown.length - 1, index + 1));
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            setActive(Math.max(0, index - 1));
          } else if (e.key === "Enter") {
            e.preventDefault();
            if (shown[index] !== undefined) props.onPick(shown[index]);
          } else if (e.key === "Escape") {
            e.preventDefault();
            props.onCancel();
          }
        }}
        className="h-[26px] w-full border-b border-line bg-panel px-s2 font-mono text-[12px] outline-none placeholder:font-body placeholder:text-text-3"
      />
      <div className="max-h-[240px] overflow-auto py-[2px]">
        {shown.map((value, i) => (
          <button
            key={value}
            type="button"
            role="option"
            aria-selected={i === index}
            className={`flex h-[24px] w-full items-center gap-s2 px-s2 text-left font-mono text-[12px] ${i === index ? "bg-accent-tint" : "hover:bg-hover"}`}
            style={i === index ? { boxShadow: "inset 2px 0 0 var(--accent)" } : undefined}
            onMouseEnter={() => setActive(i)}
            onClick={() => props.onPick(value)}
          >
            <span className="w-[12px] flex-none text-accent-text">{value === currentText ? "✓" : ""}</span>
            <span className="truncate">{value}</span>
          </button>
        ))}
        {shown.length === 0 && <p className="px-s2 py-[4px] text-[12px] text-text-3">{t("No value matches.")}</p>}
      </div>
      <button
        type="button"
        disabled={!props.column.nullable}
        className="flex h-[26px] w-full items-center justify-between border-t border-line px-s2 text-left text-[12.5px] enabled:hover:bg-hover disabled:cursor-not-allowed disabled:text-text-3"
        onClick={props.onNull}
      >
        {t("Set NULL")}
        {!props.column.nullable && <span className="text-[11.5px]">{t("NOT NULL column")}</span>}
      </button>
    </Floating>
  );
}
