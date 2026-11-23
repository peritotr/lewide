// T07: a date, time or timestamp edited in its parts, under the cell.
// The value is kept as the database wrote it; nothing is converted between zones.
import { useState } from "react";
import type { ColumnDescription } from "../lib/api";
import { joinTemporal, splitTemporal, stepTemporal, temporalError, temporalShape, type TemporalParts } from "../lib/edit";
import { t } from "../i18n";
import { Floating, type Anchor } from "./menu";

const isMac = /Mac/i.test(navigator.userAgent);

export function DateEditor({ anchor, column, text, onDone, onNull, onCancel }: { anchor: Anchor; column: ColumnDescription; text: string; onDone(text: string): void; onNull(): void; onCancel(): void }) {
  const type = column.type_name ?? "timestamp";
  const shape = temporalShape(type);
  const [parts, setParts] = useState<TemporalParts>(() => splitTemporal(text) ?? { date: "", time: "", zone: shape.zone ? "+00" : "" });
  const error = temporalError(parts, type);
  const field = (bad: boolean) => `h-[26px] min-w-0 border bg-panel px-s2 font-mono text-[12.5px] outline-none ${bad ? "border-danger" : "border-line focus:border-accent"}`;
  const small = "h-[26px] px-s2 text-[12.5px] whitespace-nowrap enabled:hover:bg-hover disabled:cursor-not-allowed disabled:opacity-45";

  const done = () => !error && onDone(joinTemporal(parts));
  const onKey = (part: "date" | "time" | null) => (e: React.KeyboardEvent) => {
    e.stopPropagation();
    if (e.key === "Enter") {
      e.preventDefault();
      done();
    } else if (e.key === "Escape") {
      e.preventDefault();
      onCancel();
    } else if (part && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
      e.preventDefault();
      // Shift moves a week, or an hour.
      const size = e.shiftKey ? (part === "date" ? 7 : 60) : 1;
      setParts(stepTemporal(parts, part, e.key === "ArrowUp" ? size : -size));
    }
  };
  const now = () => {
    const iso = new Date().toISOString();
    setParts({ date: shape.date ? iso.slice(0, 10) : "", time: shape.time ? iso.slice(11, 19) : "", zone: shape.zone ? "+00" : "" });
  };

  return (
    <Floating anchor={anchor} width={330} onClose={onCancel} role="dialog" label={column.name} className="">
      <div className="flex flex-col gap-s3 p-s3">
        <div className="flex items-end gap-s2">
          {shape.date && (
            <label className="flex min-w-0 flex-[1.3] flex-col gap-[4px]">
              <span className="label">{t("Date")}</span>
              <input autoFocus className={field(error === "date")} value={parts.date} placeholder="YYYY-MM-DD" spellCheck={false} onChange={(e) => setParts({ ...parts, date: e.target.value })} onKeyDown={onKey("date")} />
            </label>
          )}
          {shape.time && (
            <label className="flex min-w-0 flex-1 flex-col gap-[4px]">
              <span className="label">{t("Time")}</span>
              <input autoFocus={!shape.date} className={field(error === "time")} value={parts.time} placeholder="HH:MM:SS" spellCheck={false} onChange={(e) => setParts({ ...parts, time: e.target.value })} onKeyDown={onKey("time")} />
            </label>
          )}
          {shape.zone && (
            <label className="flex w-[72px] flex-none flex-col gap-[4px]">
              <span className="label">{t("Zone")}</span>
              <input className={field(error === "zone")} value={parts.zone} placeholder="+00" spellCheck={false} onChange={(e) => setParts({ ...parts, zone: e.target.value })} onKeyDown={onKey(null)} />
            </label>
          )}
        </div>
        <p className="text-[12px] leading-[1.4] text-text-2">
          {error ? <span className="text-danger">{error === "date" ? t("That date does not exist.") : error === "time" ? t("That time does not exist.") : t("A zone is written like +03 or +03:00.")}</span> : t("Saved as written here. Type, or use ↑↓ on the date or the time.")}
        </p>
      </div>
      <div className="flex h-[40px] items-center gap-[2px] border-t border-line px-s2">
        <button type="button" className={small} onClick={now}>
          {t("Now")}
        </button>
        <button type="button" className={small} disabled={!column.nullable} title={column.nullable ? undefined : t("NOT NULL column")} onClick={onNull}>
          {t("Set NULL")}
        </button>
        <span className="flex-1" />
        <button type="button" className={small} onClick={onCancel}>
          {t("Cancel")}
        </button>
        <button type="button" className={`${small} border border-line font-medium`} disabled={!!error} onClick={done}>
          {t("Done")} <span className="font-mono text-[11px] text-text-3">{isMac ? "↵" : "Enter"}</span>
        </button>
      </div>
    </Floating>
  );
}
