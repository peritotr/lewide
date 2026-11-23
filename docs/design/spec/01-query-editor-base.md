# Query editor — base layout

Design file: `Query Editor.dc.html` — screen E01, 1440×900, light and dark. Tweaks in the file switch the title bar (macOS / Windows-Linux) and density (Compact / Comfortable). High-fidelity.

This is the window every other area builds on. States of this screen (running, errors, menus, autocomplete, …) are in `03-query-editor-states.md`; component states are in `07-library-palette-settings.md`.

## Design language
- **Square corners** (radius 0), **1px hairlines**, no gradients, no blur, no OS-specific styling.
- One accent, steel blue. Other hues only carry function: Production amber, connection markers, syntax, ok / danger.
- **Registration marks** — an 11×11px "+" (two 1px lines, `--text` at 55%), centred on a corner, offset −6px — appear only on the four corners of the primary button and at pane junctions (sidebar split, editor / results split).
- Icons: Lucide, stroke 1.5, `currentColor`, 12–15px.
- Meaning is never colour alone: a connection colour always comes with the connection name and an environment tag; status marks differ by shape.

## Typography
| Role | Font | Size |
| --- | --- | --- |
| UI text | Barlow 400 (emphasis 500) | 13px / 1.3 |
| Small UI (status bar, selects) | Barlow | 12px |
| Section labels, tags | Barlow Condensed 600, uppercase, .08em | 11px (tags 10px) |
| Primary button | Barlow Condensed 600, .06em | 13px |
| Wordmark in the title bar | Barlow Condensed 600, .01em | 15px |
| Editor | IBM Plex Mono | 13px / 20px |
| Grid cells | IBM Plex Mono, tabular figures | 12.5px |
| Grid header name / type | IBM Plex Mono 500 / 400 | 12px / 11px |
| Tree identifiers / counts | IBM Plex Mono | 12px / 11px |
Numbers in grids are right-aligned and **raw** (`2184.50`). Leave room for translation: no fixed-width labels.

## Window
CSS grid rows: title bar **38px** · body · status bar **24px**. Body columns: sidebar **264px** (resizable, collapsible — see N02) · workspace.

### Title bar (`--surface`, bottom border `--line`, padding 0 10.2px, gap 10.2px)
- macOS: three 12px circles at the left (the OS draws the real ones; reserve 68px).
- Mark 18px + wordmark `LewIDE` ("IDE" in `--accent-text`).
- Centred command field 400×24px: search icon, `Go to table, connection or command`, shortcut `⌘K` (mono 11px).
- Settings icon button (24px).
- Windows / Linux: minimise, maximise, close at the right, 44px wide each, full height; close hover = `--danger`.
- The empty area drags the window.

### Sidebar (right border `--line`)
1. Filter field, 6.8px inset: 26px high, `Filter connections and objects`.
2. Header `CONNECTIONS` + `+` (22px icon button).
3. Connection rows, height `--tree-h`, gap 6.8px, padding 0 6.8px: chevron 12px · **type badge** (24×16, 1px `--line`, 3px left border in the connection colour, two letters in mono 500 10px) · name (ellipsis) · environment tag (`PROD` = `--warn` border and `--warn-text`; `STG`, `LOCAL` neutral) · status mark (7px filled `--ok` square = connected, hollow = disconnected). The active connection is weight 500 with chevron-down.
4. A hairline with a registration mark at its right end, then header `OBJECTS · <connection>`.
5. Tree: `Saved queries` (bookmark, count) · database (database icon, indent 6.8px) · schema (layers icon, indent 22px) · `Tables` + count (indent 36px) · table rows (indent 54px: table icon `--text-3`, mono name, count right-aligned) · `Views`, `Functions` collapsed. Selected row: `--accent-tint` + 2px `--accent` inset bar.

### Workspace (`--panel`)
Rows: tab bar **34px** · Production band **4px** (only for a Production connection) · toolbar **40px** · editor (flexible) · results **400px** (resizable).
- **Tab bar** (`--surface`, bottom border `--line`): each tab is a cell with right border `--line`, padding 0 6.8px 0 10.2px, gap 6.8px: 8px connection marker · type icon 13px (terminal = query, table = data, columns = structure) · title · connection name (mono 11px `--text-3`) · `PROD` tag when Production · close button (18px). Active tab: `--panel`, 2px top line in the connection colour, title 500, overlaps the bar's bottom border. Unsaved: a 7px round dot replaces the close button. `+` opens a new query tab.
- **Production band**: `repeating-linear-gradient(135deg, var(--warn) 0 6px, transparent 6px 12px)`. It follows the active tab's connection.
- **Toolbar** (bottom border `--line`, padding 0 10.2px, gap 6.8px, controls 26px high): primary `RUN ⌘↵` (four registration marks) · `Run selection` · `Stop` (disabled until a query runs) · divider · `Explain` · `Format` · `Save` · divider · `Ask AI ⌘I` (sparkles in `--accent-text`) · right-aligned selects: context `■ connection / database / schema ▾` (mono 12px), `Limit 1000 ▾`, `Auto-commit ▾`.
- **Editor**: gutter 52px with right-aligned line numbers (`--text-3`; current line `--text`); code padding-left 12px; top padding 6.8px. Current line: `--hover` band. A 2px `--accent` bar in the gutter spans the statement under the caret — the one `Run` executes. Syntax: keywords `--syn-kw`, functions `--syn-fn`, strings `--syn-str`, numbers `--syn-num`, comments `--syn-com` italic.
- **Results** (top border `--line`, registration mark at the top-left junction):
  - Header 34px: result-set sub-tabs `Result 1 · 100 rows` (active = 2px `--accent` underline, 500; count mono 11px `--text-3`) · lock icon + `Read-only · query result` · right: segmented `Results | Messages | Query plan` (selected `--accent-fill` / `--on-accent`).
  - Grid: header row `--row-h` on `--bg` (name mono 500 12px + type mono 11px `--text-3`; key icon for a primary key and a sort arrow in `--accent-text`; numeric headers right-aligned with the type before the name). Rows `--row-h`, row rule `--line-2`, cell padding 0 8px, row-number column on `--bg` with a `--line` right border, hover `--hover`. In the mock: `48px 104px 1fr 110px 110px 136px 230px`.

### Status bar (`--surface`, top border `--line`, 12px `--text-2`, gap 13.6px)
First segment, full height: status mark + `Connected · <connection>` + environment tag — tinted `--warn-tint` / `--warn-text`, weight 500, for Production. Then engine version and `database.schema` (mono 11.5px). Right-aligned: row count, duration, transaction state, caret position (`Ln 9, Col 49`), encoding.

## Behaviour
- `Run` (`⌘/Ctrl+Enter`) runs the statement under the caret; `Run selection` (`⇧⌘/Ctrl+Shift+Enter`) the selection; `Explain` opens the Query plan view; `Save` (`⌘/Ctrl+S`).
- Several statements give several result sets as sub-tabs.
- Sidebar: nodes expand and collapse; double-click a table opens its data tab; the border between sidebar and workspace, the split inside the sidebar and the editor / results split are draggable (5px hit area, 3px `--accent` on hover).
- Tabs: click to activate, middle-click or `⌘/Ctrl+W` to close, drag to reorder.
- Grid: virtualised rows; click selects a cell (row tint + 2px `--accent` ring); arrow keys move; headers sort and resize.
- Title bar variant follows the OS; density is a user setting.

## State
`connections[]` (id, name, typeCode, environment, colour 1–4, status) · `activeConnectionId`, database, schema · object tree per connection, loaded lazily · `tabs[]` (id, kind, title, connectionId, dirty), `activeTabId` · per query tab: text, caret, run state, `resultSets[]`, active result, active view, limit, transaction mode · preferences: theme, density, sidebar width / collapsed, split sizes.

## Not designed here
Everything dynamic on this screen is specified in `03-query-editor-states.md`; the AI panel in `06-ai-assistant.md`; narrow and Windows variants in `07-library-palette-settings.md`.
