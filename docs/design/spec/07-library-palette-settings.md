# Handoff: Component library, command palette, settings, window variants

## Overview
The last general pieces of the LewIDE desktop client, in the implemented **1a "Drafting table"** direction:
- **Component library** — every component in every state, light and dark, with density-dependent components in Compact and Comfortable.
- **Command palette** — P01–P04.
- **Settings** — G01–G06 (the AI section is in the AI assistant handoff and uses the same window).
- **Window variants** — W01 Windows / Linux title bar; N01–N02 narrow window 1024×700.

## About the design files
Both `.dc.html` files are **design references in HTML**, not production code. Recreate them in the existing Tauri / React / TypeScript / Tailwind app. The component library is the reference for states; where an earlier handoff and the library differ, the library wins.

## Fidelity
**High-fidelity.**

## Tokens — changes in this handoff
`tokens.css` in this folder replaces earlier copies.

**Corrected (dark theme only)** — the earlier values were never visually checked. On menus, dialogs and tooltips the hairline ring in the shadow sat next to the surface's own 1px border and read as a 2px edge; the ring is removed and the shadow deepened.
| Variable | Before | Now |
| --- | --- | --- |
| `--shadow-sm` | `0 0 0 1px` ring | `0 1px 2px rgb(0 0 0 / .5)` |
| `--shadow-md` | ring + `0 6px 16px rgb(0 0 0 / .4)` | `0 8px 24px rgb(0 0 0 / .55)` |
| `--shadow-lg` | ring + `0 16px 40px rgb(0 0 0 / .5)` | `0 20px 56px rgb(0 0 0 / .65)` |
Rule that goes with it: every elevated surface keeps its own 1px `--line` border in both themes. Light-theme shadows are unchanged. The library's dark panel ends with a before/after block.

**Added**
| Variable | Light | Dark | Use |
| --- | --- | --- | --- |
| `--scroll-thumb` | `#1d1f20` at 26% | `#e3e4e6` at 26% | scrollbar thumb |
| `--scroll-thumb-hover` | at 42% | at 42% | thumb on hover |

No other value changed.

## Component library (`Component Library.dc.html`)
Two panels, light (left) and dark (right), same content. State columns are labelled; an em dash means the state does not apply.

**State rules used throughout**
- **Hover** — line-based controls (secondary, ghost, icon button, rows, menu items in lists): background `--hover`. Filled controls (primary, danger): an overlay of the control's own text colour at 12% (`color-mix(in srgb, var(--on-accent) 12%, transparent)`), so it lightens on the light theme and darkens on the dark theme.
- **Pressed** — line-based: background `--line`. Filled: the same overlay at 24%. Icon buttons that stay on (toggles): 1px `--accent` + `--accent-tint`.
- **Focus** — `outline: 2px solid var(--accent); outline-offset: 2px` (inside rows: offset −2px). Text fields also switch their border to `--accent`.
- **Disabled** — opacity .45, `cursor: not-allowed`, not focusable.
- **Error** — 1px `--danger` border; message under the field in 11.5px `--danger` with a triangle icon.
- Checkbox, radio and switch hover: a 3px `--hover` halo.

**Components and sizes**
| Component | Key measures |
| --- | --- |
| Button | 26px high in toolbars, 28px in dialogs. Primary: `--accent-fill`, Barlow Condensed 600 13px, .06em, uppercase, four registration marks. Secondary: 1px `--line`, Barlow 13px. Ghost: no border. Danger: `--danger` fill, `--on-danger` text, no marks. |
| Icon button | 24×24 (22 in section headers, 18 for tab close), icon 14px `--text-2`. |
| Text / password input, select | 28px, 1px `--line`, `--panel`, padding 0 8px; mono 12.5px for technical values. Hover border `--text-3`. Password: show/hide eye; saved secrets show lock + `Saved · unchanged`. Select: value + 12px chevron; open keeps the `--accent` border. |
| Checkbox / radio / switch | 14px square / 14px circle / 28×16 square track with 12px knob. On = `--accent-fill`. Checkbox has a mixed state (bar). Switch shows `On` / `Off` beside it. |
| Segmented | 26–28px, 1px `--line`, cells padding 0 10–12px; selected `--accent-fill` / `--on-accent`, 500. |
| Window tab | 34px; marker 8px, type icon 13px, title, close. Active: `--panel`, 2px top line in the connection colour, title 500. States: unsaved dot, running spinner, error triangle. |
| Sub-tab | 34px; label + count; active 500 with 2px `--accent` underline; unsupported = crossed out `--text-3`. |
| Tree row / connection row | height `--tree-h` (24 / 30px). Selected: `--accent-tint` + 2px inset bar. Count slot: number, hatched placeholder while pending, or empty. Connection marks: filled square, spinner, triangle, slashed square, hollow square. |
| Grid header / cell | height `--row-h` (28 / 36px). Header on `--bg`; key and sort marks 11px `--accent-text`; resize handle 3px `--accent`. Cell states: hover, active (2px `--accent` ring), row selected, NULL, boolean, binary, invalid (2px `--danger`), modified (corner triangle + `--accent-tint`), added (`--ok-tint`), marked for deletion (`--danger-tint` + strike). |
| Tag | Barlow Condensed 600 10px, .08em, padding 2px 4px, 1px border. |
| Type badge | Sidebar 24×16 with a 3px left border in the connection colour; dialogs 26×18 with 1px `--text-3`; coming soon = dashed. |
| Menu | 210px+, `--panel`, 1px `--line`, `--shadow-md`; item 26px; active item `--accent-tint`; checked item has a check and weight 500; danger item in `--danger`. |
| Tooltip | inverted: background `--text`, text `--bg`, 11.5px, padding 4px 8px, `--shadow-sm`; shortcut in mono at 75% opacity. Delay 500 ms, no arrow. |
| Dialog / confirmation | `--bg`, 1px `--line`, `--shadow-lg`; header 40–44px, footer 46–52px; confirmation uses the danger button and puts focus on Cancel. |
| Toast | 340px, `--panel`, 1px `--line` (errors `--danger`), `--shadow-md`; icon, title 500, detail in mono 12px, optional action, close. Bottom-right, 16px from the edges, stacked with 8px gap; leaves after 6 s, errors stay. |
| Strip | 1px border + tint, padding 6.8px 10.2px, 12.5px; variants neutral, note (`--accent-tint`), warning, error, success. |
| Progress | Determinate bar 4px; indeterminate bar 2px with a 24–30% segment, 1.2 s loop; spinner 10 / 12 / 16px, 0.8 s. All static under `prefers-reduced-motion`. |
| Empty state | Title Barlow Condensed 600 20px, one sentence 12.5px `--text-2`, optional mono detail or one action. |
| Divider | pane `--line`, row `--line-2`, toolbar 1×16px `--line`. |
| Resize handle | idle = the 1px pane border with a 5px hit area; hover 3px `--accent`; dragging 3px `--accent-fill`. |
| Scrollbar | 10px track on `--bg` with a `--line-2` edge; thumb 6px wide, `--scroll-thumb`, hover `--scroll-thumb-hover`, dragging `--text-3`. |

## Command palette (P01–P04)
Opens with `⌘K` / `Ctrl+K` or the title-bar field. Dialog 680px, top 96px, backdrop `--scrim`, `--shadow-lg`.
- **Input row** 46px: search icon 16px, text 15px, `Esc` key hint at the right.
- **Results**: group label rows (Barlow Condensed 11px + count in mono) and item rows 30px: 16px kind icon (table, bookmark = saved query, clock = recent, `>` = command, type badge = connection) · title 13px with the matched part in `--accent-text` 500 · secondary text in mono 11.5px `--text-3` · right: connection marker + name (mono 11px) · shortcut (mono 11px `--text-3`). Selected row: `--accent-tint`, 2px inset bar, and `↵ Open` in `--accent-text`.
- **Footer** 30px on `--surface`: `↑↓ Move · ↵ Open · ⌘↵ Open in new tab` and `> Commands only`.
- **P01 just opened**: `RECENT`, `CONNECTIONS`, `COMMANDS` (three items each, group count shows the total).
- **P02 typing**: groups in fixed order `TABLES`, `CONNECTIONS`, `SAVED QUERIES`, `COMMANDS`, `RECENT`; empty groups are omitted; max 5 rows per group, then `Show all` (not drawn). Matching is fuzzy on the name; tables of connected databases only.
- **P03 no results**: `Nothing matches “…”`, a line saying what was searched, `Search disconnected connections too`, `Clear`.
- **P04 `>` prefix**: commands only; row shows the command's area as secondary text; footer changes to `↵ Run command` and `Remove > to search everything`.
- Enter on a table opens its data tab; on a connection connects and selects it; on a saved query opens it in a tab; on a command runs it.

## Settings (G01–G06)
Window 980×770, top 56px. Header 44px. Left navigation 200px on `--surface`: Appearance, Editor, Data, Keyboard shortcuts, AI, Security, About; rows 28px; active `--accent-tint` + inset bar. Content padding 0 24px. Group labels (Barlow Condensed 11px) sit on a `--line` rule; each setting is a row with title (500) and description (12.5px `--text-2`) at the left and the control at the right, separated by `--line-2`. Changes apply immediately; there is no Save button.
- **G01 Appearance** — Theme `System | Light | Dark`; Density `Compact | Comfortable`; Interface font size (stepper, 12–16px); Editor and data font (select); Editor and data font size (stepper, 11–20px); a live preview block.
- **G02 Editor** — Tab width `2 | 4 | 8`; Indent with spaces; Word wrap; Autocomplete (switch + `After 1 character`); Uppercase keywords; Highlight the statement under the cursor.
- **G03 Data** — Default row limit; Ask before running without a limit; Numbers `Raw | Grouped` (copying always gives the raw value); Date and time format; Time zone `UTC | Local | As stored`; NULL `Tag | Dimmed text | Blank` with a sample.
- **G04 Keyboard shortcuts** — search field with a match count, `All | Changed` filter, `Reset all`. Table `COMMAND · SHORTCUT · WHERE`. Click a shortcut to record: the cell becomes a focused field and the row is highlighted. **Conflict**: a warning strip under the row names the command that already uses the keys and where; `Assign` keeps both when they are in different places, or reassigns when they are in the same place; `Cancel` restores. Changed shortcuts show a `Reset` link. Backspace clears, Esc cancels.
- **G05 Security** — Production: Confirm destructive statements (`Always on`, locked); Type the connection name to confirm; Preview SQL before applying data changes. Passwords: Save passwords in the system keychain; Saved passwords with `Remove all…` (danger outline, asks first). SSH: known_hosts file (field + `Browse…`); Unknown hosts `Ask | Refuse`.
- **G06 About** — mark 56px + wordmark 34px, version and build (mono), `Check for updates` with status, Update automatically, links, and a copyable `SYSTEM` block.

## Windows / Linux (W01)
Same window with the title bar variant: no traffic lights; minimise, maximise and close at the right, 44px wide each, full height, close hover = `--danger` fill with `--on-danger` icon. The empty title-bar area drags the window; double-click maximises. **All shortcut labels switch to `Ctrl` forms** (`Ctrl+Enter`, `Ctrl+K`, `Ctrl+I`, …) in buttons, menus, tooltips, the palette and Settings. Paths in About follow the platform.

## Narrow window (N01, N02) — 1024×700
Minimum window size: 900×600.
- **N01**: sidebar 220px (minimum 200); title-bar field 260px. Toolbar is **icon-only** below 1040px of toolbar width: `RUN` keeps its label without the shortcut; Run selection, Stop, Explain, Format, Save and Ask AI become 26px icon buttons with tooltips; the context select shows `connection / schema`; Limit and transaction mode move into a `⋯` menu. Tabs scroll with the overflow controls. Result columns shrink, with `email` flexible and timestamps cut to the minute. Status bar drops the engine version and encoding and shortens labels.
- **N02**: sidebar collapsed to a **40px rail** (`⌘B` / `Ctrl+B`, or drag the border past 160px): an expand button, then one type badge per connection (28×22, colour spine, a 7px `--ok` square for connected, a 1.5px `--accent` frame for the active one), then `+`. Hover shows the connection name; click expands the sidebar on that connection. Results height in both: 280px by default.

## Not designed (explicitly)
`Show all` in the palette · palette prefixes other than `>` · font picker list · shortcut scopes editor · remove-all-passwords confirmation · update available / downloading states · Settings search · Linux-specific title bar styling (uses the Windows variant) · windows below 900×600.

## Files
- `Component Library.dc.html` — components and states, light + dark.
- `Palette Settings Window.dc.html` — P01–P04, G01–G06, W01, N01–N02.
- `tokens.css` — current tokens (dark shadows corrected, scrollbar tokens added).
- `support.js`, `_ds/` — needed only to open the design files in a browser.
