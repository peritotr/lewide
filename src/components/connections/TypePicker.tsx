// C02: choose a database type.
import { Search } from "lucide-react";
import { useMemo, useState } from "react";
import { TYPE_GROUPS, type DatabaseType } from "../../lib/types";
import { openDialog, useApp } from "../../store";
import { Button, Dialog, TypeBadge } from "../dialog";
import { t } from "../../i18n";

const COLUMNS = 6;

export function TypePicker({ draft }: { draft?: unknown }) {
  const drivers = useApp((s) => s.drivers);
  const [query, setQuery] = useState("");
  const available = (type: DatabaseType) => !!type.driver && drivers.some((d) => d.id === type.driver);

  const groups = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return TYPE_GROUPS.map((group) => ({
      ...group,
      types: group.types.filter(
        (t) => !needle || t.name.toLowerCase().includes(needle) || t.code.toLowerCase().includes(needle) || group.model.toLowerCase().includes(needle),
      ),
    })).filter((group) => group.types.length > 0);
  }, [query]);

  // Keyboard focus moves over the available types only.
  const ready = groups.flatMap((g, row) => g.types.map((t, col) => ({ t, row, col })).filter(({ t }) => available(t)));
  const [selectedCode, setSelectedCode] = useState("Pg");
  const selected = ready.find(({ t }) => t.code === selectedCode) ?? ready[0];

  const proceed = (type = selected?.t) => {
    if (type?.driver) openDialog({ kind: "form", driverId: type.driver, engine: type.code, draft });
  };

  function onKeyDown(e: React.KeyboardEvent) {
    if (!selected || !["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"].includes(e.key)) return;
    e.preventDefault();
    const index = ready.indexOf(selected);
    let next = selected;
    if (e.key === "ArrowRight") next = ready[Math.min(ready.length - 1, index + 1)];
    if (e.key === "ArrowLeft") next = ready[Math.max(0, index - 1)];
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      // The nearest column in the next row that has an available type.
      const step = e.key === "ArrowDown" ? 1 : -1;
      const rows = [...new Set(ready.map((r) => r.row))];
      const target = rows[rows.indexOf(selected.row) + step];
      if (target !== undefined) {
        const inRow = ready.filter((r) => r.row === target);
        next = inRow.reduce((best, r) => (Math.abs(r.col - selected.col) < Math.abs(best.col - selected.col) ? r : best));
      }
    }
    setSelectedCode(next.t.code);
  }

  return (
    <Dialog
      title={t("New connection")}
      headerExtra={<span className="text-[12.5px] text-text-2">{t("Step 1 of 2 · Choose a database type")}</span>}
      width={1000}
      onClose={() => openDialog(null)}
      onSubmit={() => proceed()}
      footer={
        <>
          <span className="mr-auto text-[11.5px] text-text-3">
            <span className="font-mono text-text-2">↑↓←→</span> {t("Move")} · <span className="font-mono text-text-2">↵</span> {t("Continue")} ·{" "}
            <span className="font-mono text-text-2">Esc</span> {t("Close")}
          </span>
          <Button variant="ghost" onClick={() => openDialog(null)}>
            {t("Cancel")}
          </Button>
          <Button variant="primary" type="submit" disabled={!selected}>
            {selected ? t("Continue with {type}", { type: selected.t.name }) : t("Continue")}
          </Button>
        </>
      }
    >
      <div onKeyDown={onKeyDown}>
        <label className="mb-s4 flex h-[30px] items-center gap-s2 border border-line bg-panel px-[8px] focus-within:border-accent focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-accent">
          <Search size={13} className="text-text-3" />
          <input
            data-autofocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("Search database types")}
            aria-label={t("Search database types")}
            className="min-w-0 flex-1 bg-transparent placeholder:text-text-3 focus:outline-none"
          />
        </label>
        <div className="flex flex-col gap-s2">
          {groups.map((group) => (
            <div key={group.model} className="grid grid-cols-[124px_minmax(0,1fr)] items-center">
              <span className="label">{t(group.model)}</span>
              <div className="grid gap-s2" style={{ gridTemplateColumns: `repeat(${COLUMNS}, minmax(0, 1fr))` }}>
                {group.types.map((type) => {
                  const ok = available(type);
                  const isSelected = ok && selected?.t.code === type.code;
                  return (
                    <button
                      key={type.code}
                      type="button"
                      tabIndex={-1}
                      aria-disabled={!ok}
                      title={ok ? undefined : t("Not supported yet")}
                      className={`flex h-[44px] items-center gap-s2 px-s2 text-left ${
                        !ok
                          ? "border border-dashed border-line text-text-3"
                          : isSelected
                            ? "border-2 border-accent bg-accent-tint px-[5.8px]"
                            : "border border-line hover:bg-hover"
                      }`}
                      onClick={() => ok && setSelectedCode(type.code)}
                      onDoubleClick={() => ok && proceed(type)}
                    >
                      <TypeBadge code={type.code} dashed={!ok} />
                      <span className="min-w-0">
                        <span className="block truncate text-[12.5px] font-medium">{type.name}</span>
                        {ok ? (
                          <span className="block truncate text-[10.5px] text-text-3">{t(group.model)}</span>
                        ) : (
                          <span className="block font-heading text-[10px] font-semibold tracking-[.1em]">{t("COMING SOON")}</span>
                        )}
                      </span>
                    </button>
                  );
                })}
              </div>
            </div>
          ))}
          {/* The "no results" state has no design yet. */}
          {groups.length === 0 && <p className="py-s4 text-text-3">{t("No database type matches “{query}”.", { query })}</p>}
        </div>
      </div>
    </Dialog>
  );
}
