# Handoff: Query Editor — missing states

## Overview
The states left as "not designed" in the Query Editor handoff, in the implemented **1a "Drafting table"** direction. 20 screens (Q01–Q20), each a full 1440×900 window in light and dark. The layout is unchanged; each screen differs only in the regions named below.

Decisions applied: two-letter type badges (no logos) · raw numbers in grids (`2184.50`, no thousands separator) · read-only text is `Read-only · query result` · title bar shows a logo placeholder + the `LewIDE` wordmark (Barlow Condensed 600 15px, "IDE" in `--accent-text`).

## About the design files
`Query Editor States.dc.html` is a **design reference in HTML**, not production code. Recreate it in the existing Tauri / React / TypeScript / Tailwind app with the components already built. Screen flags and sample data are in the logic class at the end of the file. Popups are positioned with fixed coordinates in the mock; anchor them to their triggers in the app.

## Fidelity
**High-fidelity.** Colours, type, spacing and copy are final.

## Tokens
**No new variables.** `tokens.css` is identical to the Connections handoff (it already contains `--scrim`, `--danger-tint`, `--ok-tint`, `--on-danger`). Mock-only helpers, not for shipping: `data-c` → `--c` (connection colour), `data-off` (disabled look).

## Shared rules used below
- Spinner: 10px ring, 1.5px `--accent`, one quarter open, 0.8s linear rotation (static under `prefers-reduced-motion`).
- Menu: `--panel`, 1px `--line`, `--shadow-md`, padding 3.4px 0; item 26px, padding 0 10.2px, shortcut right in mono 11px `--text-3`; hover / keyboard-active item `--accent-tint`; separator 1px `--line`; checked item shows a 12px check at the left and weight 500, unchecked items are indented 30px.
- Popover: same surface as a menu, with its own padding.
- Error mark is always a **shape plus text**: warning triangle (results, tabs), diamond (gutter), wavy underline (token).
- All server-provided text (errors, notices, statements) is selectable.

## Screens

### Q01 — Running (under 2 s)
- Toolbar: `RUN` disabled (opacity .45). `Stop` becomes active: 1px `--danger` border, `--danger` text, weight 500, filled square icon, shortcut `⌘.`; hover `--danger-tint`.
- Tab: type icon replaced by the spinner.
- Results: a 2px indeterminate bar directly under the results header (track `--line`, 24% `--accent` segment sliding left→right, 1.2s loop). Body, padding 20.4px: spinner + `Running statement 1 of 2` (500) + elapsed time (Barlow Condensed 600 22px, tabular, updates every 100 ms) and the statement preview (mono 12px `--text-2`, one line, ellipsis). Result sub-tabs and the read-only note are hidden; the view switch stays.
- Status bar: spinner + `Running… 0.8 s` replaces row count and duration.

### Q02 — Long-running (after 2 s)
Same as Q01, plus a bordered block (1px `--line`, padding 10.2px, max-width 720px): `Still running on <connection>` · `Started 09:41:07. You can keep working in other tabs; this tab shows a mark when it finishes.` · `Stop query ⌘.` button (danger outline, 28px). The block appears once elapsed time passes 2 s. When a query finishes in a background tab, that tab shows the unsaved-style dot in `--accent` until visited (not drawn).

### Q03 — Cancelled
Results body: bordered neutral block with a stop-square icon, `Query cancelled`, `Stopped after 14.6 s. No rows were returned. Statement 2 was not run.`, and `Run again` (secondary). Toolbar returns to idle. Status bar: `Cancelled` · `14.6 s`. Stop is sent once; while the server has not confirmed, the Stop button shows `Stopping…` disabled (not drawn).

### Q04 — Query error
- Results (`role="alert"`, padding 13.6px 20.4px, gap 10.2px): row 1 — danger triangle, `ERROR` tag (Barlow Condensed 600 11px, 1px `--danger`), error code (mono 500 12.5px), condition name (mono `--text-2`), `Copy error` at the right. Row 2 — the message, mono 14px. Row 3 — a bordered grid (label column 72px, labels Barlow Condensed 600 11px `--text-3`): `POSITION` (`Line 2, column 23` + `Go to line` link), `HINT`, `DETAIL`; a missing field reads `None from the server` in `--text-3`, or omit the row. Row 4 — `Statement 1 of 2 failed after 12 ms. Statement 2 was not run.`
- Editor: error line gets a `--danger-tint` band across the full width and an 8px `--danger` diamond in the gutter (left of the number); the offending token gets `text-decoration: underline wavy var(--danger)`, 1.5px, offset 3px, with the message as tooltip. `Go to line` moves the caret there.
- Tab: type icon replaced by a 13px danger triangle until the next run or edit.
- Status bar: `Error 42703` · `12 ms`.

### Q05 — Messages view
Table, mono 12px, row height `--row-h`, columns `100px 36px 1fr 190px 80px`: TIME (`hh:mm:ss.SSS`, `--text-3`) · # · STATEMENT (one line, ellipsis, `--text-2`) · RESULT (`3 rows affected` / `100 rows returned`) · DURATION (right-aligned). Header row on `--bg` with Barlow Condensed labels.
- Server notices are sub-rows indented 136px under their statement: a tag (`NOTICE` neutral border; `WARNING` `--warn` border and `--warn-text`) followed by the text.
- A failed statement row has a `--danger-tint` fill, result `Error <code>` with a triangle in `--danger`, and a wrapped sub-row with message, position, hint and `Go to line`.
- Footer line: `4 statements · 3 succeeded · 1 failed · 107 ms total`.
- The view switch shows `Messages` selected. After a run with an error and no result set, Messages opens automatically.

### Q06 — Query plan view
- Bar under the header: segmented `Tree | Raw text`, `Estimated plan · EXPLAIN without ANALYZE`, `total cost <n>` (mono), `Run EXPLAIN ANALYZE` (secondary; on Production it follows the destructive-statement rules if the statement writes).
- Tree table, mono 12px, columns `1fr 120px 110px 220px`: NODE · EST. COST · EST. ROWS · SHARE OF COST. Depth indent 18px per level with `└` / `├` connectors in `--text-3`; node type weight 500, detail in `--text-2`. Share = node's own cost ÷ total, drawn as a 120×6px bar (track `--line-2`, fill `--text-3`) plus the percentage.
- Most expensive node (highest own cost): row fill `--accent-tint`, 2px `--accent` inset bar at the left, tag `MOST EXPENSIVE` (1px `--accent-text`), bar fill `--accent-fill`, cost and share in weight 500.
- One-line reading under the tree (plain text, generated from the plan; omit if nothing useful).
- Numbers are raw (`412600`).
- **Raw text** view: the server's EXPLAIN output verbatim in a mono 12.5px/20px block, selectable, with `Copy` — **not drawn**.

### Q07 — No rows returned
Column header row stays (columns are known). Body centred: `No rows returned` (Barlow Condensed 600 20px) · `The query ran successfully and matched nothing.` · mono `0 rows · 9 ms`. Sub-tab reads `Result 1 · 0 rows`.

### Q08 — Statement without a result set
Results body: success block (1px `--ok`, `--ok-tint`, max-width 820px): check square, `3 rows affected`, the statement (mono, one line), right-aligned mono `6 ms · no result set`. No sub-tabs, no read-only note. Status bar: `3 rows affected` · `6 ms`.

### Q09 — Row limit reached
- Strip between results header and column header (fill `--accent-tint`, bottom border `--line`, 12.5px): info icon, `First 1,000 rows shown. The query has more rows than the limit.`, buttons `Load next 1,000` and `Raise limit…` (22px, `--panel` fill). Sub-tab `1000 rows`; status bar `1000+ rows`.
- **Limit menu** (opens under the Limit select, 200px, values in mono): `100`, `500`, `1000` (checked), `5000`, `10000`, separator, `No limit` with the note `asks first` (confirmation before running without a limit — not drawn), `Custom…` (inline number field — not drawn).
- The limit is applied by the client when fetching, not by rewriting the SQL.

### Q10 — Autocomplete
Listbox anchored under the caret, 420px wide, mono 12.5px, rows 24px, padding 0 6.8px: kind chip (18×14, 1px `--text-3`, mono 10px: `ƒ` function, `C` column, `T` table, `K` keyword) · label with the typed prefix in `--accent-text` weight 500 · detail at the right in Barlow 11.5px `--text-2` (column: `table · type`; table: `schema · table`; function: kind → return type; keyword: `keyword`). Selected row: `--accent-tint` + 2px `--accent` inset bar. Footer row (top border `--line-2`): `↑↓ Move · ↵ Tab Accept · Esc Close` and position `1 / 23`. Max 8 rows visible, then scroll. Opens after 1 typed character or `Ctrl+Space`. Ranking: columns of tables in the statement, then tables, functions, keywords.
Not drawn: documentation side panel, snippet placeholders.

### Q11 — Context selector
Popover under the context select, 600px, three columns `1.3fr 1fr 1fr` separated by `--line-2`: CONNECTION (badge, name, environment tag; current row `--accent-tint` + inset bar; disconnected connections in `--text-3` with `Connect…`), DATABASE and SCHEMA (mono 12.5px, current item weight 500 with a check). Choosing a connection reloads the other two columns; choosing a Production connection changes the band, tab mark and status bar immediately. Arrow keys move within a column, ←/→ between columns.

### Q12 — Manual transaction open
- Toolbar: after `Save`, a divider, then `Commit` (1px `--accent-fill` border, `--accent-text`, weight 500, check icon, `⌥⌘↵`) and `Rollback` (secondary, undo icon). They exist only while a transaction is open. Mode select reads `Manual`.
- **Mode menu** (340px): `Auto-commit — Each statement is committed as soon as it runs.` / `Manual — Statements run in one transaction until you commit or roll back.` (checked). While a transaction is open, a line in `--warn-text`: `A transaction is open. Commit or roll back before switching to Auto-commit.` and the Auto-commit item is disabled.
- Status bar: the transaction text becomes a full-height `--warn-tint` segment in `--warn-text`, weight 500, with a triangle: `Transaction open · 2 statements · 01:12` (statement count and running age).
- Closing the tab or the app with an open transaction asks `Commit`, `Roll back` or `Cancel` — not drawn.

### Q13 — Destructive statement on Production
`role="alertdialog"`, 600px, 1px `--warn` border, 4px hatched `--warn` band on top, backdrop `--scrim`. Header `Run this on Production?` + `PRODUCTION` tag. Body: one sentence naming the risk (`This UPDATE has no WHERE clause.` in weight 500, then the table, connection and estimated row count), the statement in a bordered `--panel` block with syntax colours (selectable, scrolls after 8 lines), then `Type the connection name to confirm` — a mono field, focused on open, with the expected name as a ghost hint at the right. Footer: `Cancel` (secondary) and a danger button `RUN <VERB>` that stays disabled until the typed text equals the connection name exactly. Enter runs only when enabled.
Triggers: `DELETE` or `UPDATE` without `WHERE`, `DROP`, `TRUNCATE`; one dialog per run listing every matching statement (multi-statement list not drawn). Applies to Run, Run selection and EXPLAIN ANALYZE.

### Q14 — Tab states, overflow, tab menu
- Per-tab mark, in the type-icon slot: running = spinner; error = danger triangle; otherwise the type icon. Unsaved = 7px round dot in `--text-2` replacing the close button (close appears on hover).
- Overflow: tabs keep their natural width and the strip scrolls horizontally; the active tab is scrolled into view. A fixed block at the right edge holds `‹` `›` scroll buttons and an `N tabs ▾` button that lists all tabs (list not drawn; use the menu style with marker, title and connection). A soft edge shadow marks clipped tabs.
- Tab menu (right-click or Shift+F10, 220px): `Close ⌘W`, `Close others`, separator, `Duplicate`, `Rename… F2`. Rename edits the title in place (not drawn).

### Q15 — Cell values and cell menu
Result `products`: columns `id int8 (PK)`, `sku text`, `name text`, `active bool`, `attributes jsonb`, `thumbnail bytea`, `description text`.
- **Boolean**: 8px square (filled `--text-2` = true, hollow 1px `--text-3` = false) followed by the literal `true` / `false`.
- **Binary**: tag `BIN` + size (`14.2 KB`), never the bytes.
- **JSON** and **long text**: single line with ellipsis; free text columns use Barlow 13px, structured values mono.
- **NULL**: italic `NULL` in `--text-3` inside a 1px dashed `--line` box.
- Cell menu (230px): `Copy ⌘C`, `Copy as JSON`, `Copy row ⇧⌘C`, separator, `View value Space`, `Filter by value`. Copy puts the raw value on the clipboard (NULL copies as an empty string).

### Q16 — Cell viewer
Popover under the cell, 440px: header 30px — column name (mono 500), `type · size · row n` (mono 11px `--text-3`), segmented `Formatted | Raw`, `Copy`, close. Body padding 10.2px, mono 12.5px/20px, selectable; JSON is pretty-printed with keys `--syn-kw`, strings `--syn-str`, numbers `--syn-num`, null italic `--text-3`; long text wraps. Max height 320px, then scroll. Footer 26px: lock + `Read-only · query result`. Opens with Space, double-click or the cell menu; Esc closes.
Not drawn: binary preview (show size, a hex excerpt and `Save to file…`).

### Q17 — Column header menu
Opens from the chevron in a header cell (the cell gets `--accent-tint`), 240px: `Sort ascending`, `Sort descending`, separator, `Copy column name`, `Copy column values`, separator, `Filter this column…`, `Hide column`, `Size to fit`. Sorting a query result is done client-side on the fetched rows.

### Q18 — Tab whose connection is closed
- Warning strip at the top of the editor (margin 6.8px 10.2px 0): slashed-square mark, `acme-prod is disconnected. Your query text is kept. Reconnect to run it.`, `Reconnect` (secondary on `--panel`), `Use another connection` (ghost).
- Toolbar: Run, Run selection, Explain, Format, Limit and mode are disabled; Save stays enabled; the context select stays readable.
- Editor stays editable. Earlier results stay visible; the note reads `Results from 09:41:08 · connection closed`.
- Sidebar row and status bar show the lost state (`Disconnected · acme-prod`).

### Q19 — Tab with no connection
Tab shows a grey marker and `No connection`; no Production band. Context select is replaced by `Choose connection ▾` in `--accent-text` with the focus ring — it is the next action. Neutral strip: `This tab has no connection. You can write here; choose a connection to run or get completions.` Editor placeholder `Write SQL here` in `--text-3`. Results: `No results yet` · `Run a statement with ⌘↵ to see its result here.` Status bar: `No connection for this tab`.

### Q20 — Save query
- Popover under `Save`, 340px, padding 13.6px: title `Save query`, `Name` field focused with the current tab title selected, a line `Listed under Saved queries for <connection> and in ⌘K`, `Cancel` and primary `SAVE ↵`. After the first save, `⌘S` saves silently and clears the unsaved dot; the tab title becomes the name.
- Sidebar: a `Saved queries` node (bookmark icon, count) sits first under `OBJECTS · <connection>`, collapsed by default; expanded it lists queries (terminal icon, name) — double-click opens one in a tab.
- Not drawn: duplicate-name warning, rename/delete of a saved query (use the connection row menu pattern).

## State
Per query tab: `run: idle | running(startedAt, statementIndex, total) | cancelling | cancelled(elapsed) | done | error(code, message, position, hint, detail)`; `messages[]` (time, index, preview, result, duration, notices[], error); `plan` (nodes[], total, raw); `resultSets[]` with `truncated` flag; `limit`; `txMode: auto | manual`; `tx: none | open(statementCount, openedAt)`; `connectionState: connected | lost | none`; `savedQueryId`, `dirty`.

## Not designed (explicitly)
Raw-text plan view · EXPLAIN ANALYZE actual timings · `Stopping…` state · background-tab finished mark · No-limit confirmation and Custom limit field · autocomplete docs panel · closing with an open transaction · multi-statement destructive list · all-tabs list · in-place tab rename · binary cell preview · duplicate saved-query name · result filter and export (toolbar icons) · narrow window.

## Files
- `Query Editor States.dc.html` — Q01–Q20, light + dark.
- `tokens.css` — unchanged from the Connections handoff.
- `support.js`, `_ds/` — needed only to open the design file in a browser.
