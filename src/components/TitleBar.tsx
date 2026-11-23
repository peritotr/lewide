import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Search, SlidersHorizontal, Square, X } from "lucide-react";
import { inTauri } from "../lib/api";
import { useState } from "react";
import { LANGUAGES, msg } from "../i18n";
import { chooseLanguage, openDialog, setTheme, useApp, type ThemePref } from "../store";
import { anchorOf, Floating, MenuItem, MenuSeparator, type Anchor } from "./menu";
import { IconButton, Mark } from "./ui";
import { t } from "../i18n";

const THEMES: [ThemePref, string][] = [
  ["system", msg("System")],
  ["light", msg("Light")],
  ["dark", msg("Dark")],
];

export const isMac = /Mac/i.test(navigator.userAgent);

/** 38px bar drawn by the app. macOS keeps its native traffic lights; other systems get the buttons on the right. */
export function TitleBar() {
  const theme = useApp((s) => s.theme);
  const language = useApp((s) => s.language);
  const [menu, setMenu] = useState<Anchor | null>(null);
  return (
    <header
      data-tauri-drag-region
      className="flex items-center gap-s3 border-b border-line bg-surface px-s3"
      // Room for the native traffic lights, which sit over the bar.
      style={isMac && inTauri ? { paddingLeft: 82 } : undefined}
    >
      {isMac && !inTauri && (
        <div className="mr-s2 flex gap-[8px]">
          {[0, 1, 2].map((i) => (
            <i key={i} className="size-[12px] rounded-full border border-text-3" />
          ))}
        </div>
      )}
      <div data-tauri-drag-region className="flex items-center gap-s2">
        <Mark size={18} />
        <span data-tauri-drag-region className="font-heading text-[15px] leading-none font-semibold tracking-[.01em] whitespace-nowrap text-text">
          Lew<span className="text-accent-text">IDE</span>
        </span>
      </div>

      <button
        type="button"
        title={t("Command palette")}
        onClick={() => openDialog({ kind: "palette" })}
        className="mx-auto flex h-[24px] w-[400px] min-w-0 items-center gap-s2 border border-line bg-bg px-s2 text-[12px] text-text-3"
      >
        <Search size={13} />
        <span className="flex-1 truncate text-left">{t("Go to table, connection or command")}</span>
        <span className="font-mono text-[11px]">{isMac ? "⌘K" : "Ctrl K"}</span>
      </button>

      {/* Until the Settings screen is designed, its two everyday choices live in this menu. */}
      <IconButton label={t("Settings")} onClick={(e) => setMenu(anchorOf(e.currentTarget))}>
        <SlidersHorizontal size={14} />
      </IconButton>
      {menu && (
        <Floating anchor={menu} width={220} alignRight onClose={() => setMenu(null)} label={t("Settings")}>
          <MenuItem
            onSelect={() => {
              setMenu(null);
              openDialog({ kind: "settings", section: "appearance" });
            }}
          >
            {t("Settings…")}
          </MenuItem>
          <MenuItem
            onSelect={() => {
              setMenu(null);
              openDialog({ kind: "settings", section: "ai" });
            }}
          >
            {t("AI providers…")}
          </MenuItem>
          <MenuSeparator />
          <div className="label px-s3 py-s2">{t("Theme")}</div>
          {THEMES.map(([value, label]) => (
            <MenuItem key={value} checked={theme === value} onSelect={() => setTheme(value)}>
              {t(label)}
            </MenuItem>
          ))}
          <MenuSeparator />
          <div className="label px-s3 py-s2">{t("Language")}</div>
          {LANGUAGES.map(({ code, name }) => (
            <MenuItem
              key={code}
              checked={language === code}
              onSelect={() => {
                setMenu(null);
                chooseLanguage(code);
              }}
            >
              {/* Each language is listed in its own name. */}
              {name}
            </MenuItem>
          ))}
        </Floating>
      )}

      {!isMac && inTauri && (
        <div className="-mr-s3 flex self-stretch text-text-2">
          <button type="button" title={t("Minimize")} className="grid w-[44px] place-items-center hover:bg-hover" onClick={() => getCurrentWindow().minimize()}>
            <Minus size={13} />
          </button>
          <button type="button" title={t("Maximize")} className="grid w-[44px] place-items-center hover:bg-hover" onClick={() => getCurrentWindow().toggleMaximize()}>
            <Square size={12} />
          </button>
          <button type="button" title={t("Close")} className="grid w-[44px] place-items-center hover:bg-danger hover:text-bg" onClick={() => getCurrentWindow().close()}>
            <X size={13} />
          </button>
        </div>
      )}
    </header>
  );
}
