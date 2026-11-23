// Q16: one cell's whole value in a panel under the cell.
import { Lock, X } from "lucide-react";
import { useState, type ReactNode } from "react";
import { base64ToHex, byteSize, displayValue, type Value } from "../lib/api";
import { t } from "../i18n";
import { Segmented } from "./dialog";
import { Floating, type Anchor } from "./menu";

/** JSON with keys, strings, numbers and null coloured as the editor colours SQL. */
function JsonView({ value, indent = 0 }: { value: unknown; indent?: number }): ReactNode {
  const pad = "  ".repeat(indent);
  if (value === null) return <span className="text-text-3 italic">null</span>;
  if (typeof value === "string") return <span style={{ color: "var(--syn-str)" }}>{JSON.stringify(value)}</span>;
  if (typeof value === "number" || typeof value === "boolean") return <span style={{ color: "var(--syn-num)" }}>{String(value)}</span>;
  if (Array.isArray(value)) {
    if (value.length === 0) return "[]";
    return (
      <>
        {"[\n"}
        {value.map((item, i) => (
          <span key={i}>
            {pad}
            {"  "}
            <JsonView value={item} indent={indent + 1} />
            {i < value.length - 1 ? ",\n" : "\n"}
          </span>
        ))}
        {pad}
        {"]"}
      </>
    );
  }
  const entries = Object.entries(value as Record<string, unknown>);
  if (entries.length === 0) return "{}";
  return (
    <>
      {"{\n"}
      {entries.map(([key, item], i) => (
        <span key={key}>
          {pad}
          {"  "}
          <span style={{ color: "var(--syn-kw)" }}>{JSON.stringify(key)}</span>
          {": "}
          <JsonView value={item} indent={indent + 1} />
          {i < entries.length - 1 ? ",\n" : "\n"}
        </span>
      ))}
      {pad}
      {"}"}
    </>
  );
}

/** The value as text for the clipboard and the Raw view: exactly what the database gave. */
export function rawText(value: Value): string {
  if (value.type === "null") return "";
  if (value.type === "bytes") return `\\x${base64ToHex(value.value, Number.MAX_SAFE_INTEGER)}`;
  return displayValue(value);
}

export function CellViewer({ anchor, column, typeName, row, value, note, onClose }: { anchor: Anchor; column: string; typeName: string | null; row: number; value: Value; note: string; onClose(): void }) {
  const [mode, setMode] = useState<"formatted" | "raw">("formatted");
  const raw = rawText(value);
  // Text that holds JSON is shown formatted too.
  let parsed: unknown;
  let isJson = value.type === "json";
  if (value.type === "json") parsed = value.value;
  else if (value.type === "text" && /^\s*[[{]/.test(value.value)) {
    try {
      parsed = JSON.parse(value.value);
      isJson = true;
    } catch {
      isJson = false;
    }
  }
  const size = value.type === "bytes" ? byteSize(value.value) : value.type === "null" ? null : t("{count} characters", { count: raw.length });

  return (
    <Floating anchor={anchor} width={440} onClose={onClose} role="dialog" label={column} className="">
      <div className="flex h-[30px] items-center gap-s2 border-b border-line px-s3 whitespace-nowrap">
        <span className="truncate font-mono text-[12px] font-medium">{column}</span>
        <span className="truncate font-mono text-[11px] text-text-3">{[typeName, size, t("row {number}", { number: row })].filter(Boolean).join(" · ")}</span>
        <span className="flex-1" />
        {isJson && (
          <Segmented<"formatted" | "raw">
            label={t("View")}
            value={mode}
            onChange={(value: "formatted" | "raw") => setMode(value)}
            options={[
              { value: "formatted", label: t("Formatted") },
              { value: "raw", label: t("Raw") },
            ]}
          />
        )}
        <button type="button" className="text-[12.5px] text-accent-text hover:underline" onClick={() => void navigator.clipboard?.writeText(raw)}>
          {t("Copy")}
        </button>
        <button type="button" aria-label={t("Close")} title={`${t("Close")} (Esc)`} className="grid size-[20px] place-items-center text-text-2 hover:bg-hover" onClick={onClose}>
          <X size={13} />
        </button>
      </div>
      <pre className="selectable max-h-[320px] overflow-auto p-[10.2px] font-mono text-[12.5px] leading-[20px] break-words whitespace-pre-wrap">
        {value.type === "null" ? (
          <span className="border border-dashed border-line px-[4px] text-[11px] text-text-3 italic">NULL</span>
        ) : value.type === "bytes" ? (
          // Not drawn in the design: the size and the first bytes in hex.
          <>
            <span className="text-text-2">{byteSize(value.value)}</span>
            {"\n"}
            <span className="break-all">{base64ToHex(value.value, 256)}</span>
          </>
        ) : isJson && mode === "formatted" ? (
          <JsonView value={parsed} />
        ) : (
          raw
        )}
      </pre>
      <div className="flex h-[26px] items-center gap-[6px] border-t border-line px-s3 text-[12px] text-text-2">
        <Lock size={11} />
        {note}
      </div>
    </Floating>
  );
}
