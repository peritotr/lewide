# Handoff: Table data view

## Overview
The editable data view that opens when a table is double-clicked in the sidebar (tab type **table data**). 19 screens (T01–T19), each 1440×900, light and dark, in the implemented **1a "Drafting table"** direction. Sample table: `public.orders` on `acme-prod` (Production).

It uses the visual language of the query result grid. The difference is that this grid is editable; edits are collected as pending changes and applied together.

## About the design files
`Table Data.dc.html` is a **design reference in HTML**, not production code. Recreate it in the existing Tauri / React / TypeScript / Tailwind app with the grid, menus and dialogs already built. Screen flags and sample rows are in the logic class at the end of the file. Cell editors and popovers are placed with fixed coordinates for the compact density; anchor them to the cell in the app.

## Fidelity
**High-fidelity.** Colours, type, spacing and copy are final.

## Tokens
**No new variables.** `tokens.css` is unchanged.

## Layout (T01)
Workspace rows, top to bottom:
1. Tab bar 34px and Production band 4px (as in the query editor). Tab icon: table.
2. **View bar** 36px (bottom border `--line`, padding 0 10.2px, gap 10.2px): segmented `Data | Structure` (12px, cells padding 4px 12px; selected `--accent-fill` / `--on-accent`); qualified name `public.orders` (mono 500 12.5px); right-aligned `Add row`, `Delete row`, divider, `Refresh`, `Export`, `Columns` (ghost buttons, 13px icon + label). `Structure` switches the same tab to the structure view.
3. **Filter bar** 36px (`--bg`, bottom border `--line`): filter icon; conditions or `No filters`; `+ Add filter`; right side: filter count tag and `Clear` when filters exist, mode switch `Builder | WHERE`, sort chip `Sort created_at ↓` (24px, 1px `--line`; click opens the sort list — not drawn).
4. **Grid**: header row `--row-h` on `--bg`, then rows. Columns in the mock: `44px` row gutter · `id` 90 · `user_id` 96 · `status` 112 · `total_cents` 100 · `gift` 60 · `shipping_address` flexible · `note` 150 · `created_at` 164 · `paid_at` 164. Mono 12px, tabular figures, numbers right-aligned and raw (`18450`). Free-text columns (`note`) use Barlow 13px.
5. Optional bar 36px (pending changes, apply result or selection — one at a time).
6. **Footer** 30px (`--bg`, top border `--line`, 12px `--text-2`): `Rows 1–500 of ≈ 8.4M`, tag `ESTIMATE` + link `Count exactly`; divider; previous / `Page` + number field (44×22) / next; `500 per page ▾`; right-aligned load time (mono) and `Editable · key id`.
7. Status bar: `500 rows loaded` and the active cell as `Row 4 · total_cents`.

## Column header (T01, T04)
Cell content, left to right: key mark, name (mono 500 12px), type (mono 11px `--text-3`). Marks, 11px in `--accent-text`: **primary key** = key icon; **foreign key** = arrow-up-right icon; **sort** = arrow-down / arrow-up before the name (click cycles asc → desc → none; Shift-click adds a secondary sort). Numeric headers are right-aligned.
**Resize handle**: a 5px hit area on the right edge of each header cell; on hover or drag it shows a 3px `--accent` bar the full header height, cursor `col-resize`; double-click sizes to fit.
**Foreign key cell**: values are `--accent-text`. Hover or focus for 400 ms shows a popover (320px): header with the reference `users.id = 48213` and constraint name; up to three columns of the referenced row; `Open row in users ⌘↵`. ⌘/Ctrl-click does the same: opens the referenced table's data tab filtered to that key.

## Filter bar (T02, T03)
- **Builder**: each condition is a chip (24px, 1px `--line`, `--panel`, mono 12px): column, operator in `--accent-text` weight 500, value, remove button. Chips are joined by `AND` labels. Click a chip to edit it.
- **Adding or editing** opens a popover (520px): `Column` (190px, shows type), `Operator` (150px), `Value`; note `Conditions are joined with AND. Use WHERE for OR and expressions.`; `Cancel`, `Add filter ↵`. Operator list (200px, 24px rows): `=`, `≠`, `>`, `≥`, `<`, `≤`, `BETWEEN`, `IN`, `IS NULL`, `IS NOT NULL` — symbol in mono `--accent-text` plus a plain-language label. Operators depend on the column type (text adds `LIKE`, `starts with`; not drawn). `IS NULL` hides the value field; `BETWEEN` shows two.
- **Raw WHERE**: the bar becomes `WHERE` + one mono field with syntax colours, focused, and `Apply ↵`. Switching Builder → WHERE writes the current conditions as SQL. Switching back is possible only while the text still maps to simple AND conditions; otherwise `Builder` is disabled with a tooltip.
- Count tag: `2 FILTERS` or `WHERE` (1px `--accent-text`). `Clear` removes all. The footer total and the status bar show the filtered state.
- A WHERE error from the server appears as an error strip under the bar (not drawn; reuse the query error block).

## Cell editing (T05–T08)
Enter, F2 or double-click starts editing; typing replaces the value. Enter commits and moves down, Tab commits and moves right, Esc cancels. Committing only marks the cell as pending. The active cell has a 2px `--accent` ring.
- **Text / number (T05)**: the cell becomes an input in place (same size, `--panel`). Invalid input: 2px `--danger` border and a popover under the cell (1px `--danger`): `Not a whole number. int4 takes digits only, from −2147483648 to 2147483647. Esc restores 18450.` The cell cannot be committed while invalid. Validation is by column type and nullability; server-side constraints are reported at Apply.
- **Boolean**: Space toggles `true` / `false` in place; no editor.
- **Enum (T06)**: list under the cell (190px, mono, 24px rows), current value checked, keyboard-active row `--accent-tint` + inset bar; type to filter. Last row `Set NULL`, disabled with the note `NOT NULL column` when not nullable.
- **Date and time (T07)**: popover 330px with `Date`, `Time`, `Zone` fields (↑↓ changes the focused part), note `Shown and saved in UTC. Type or use ↑↓ on a part.`, `Now`, `Set NULL`, `Cancel`, `Done ↵`. A calendar picker is **not designed**.
- **JSON and long text (T08)**: popover 460px: header with column, type, row, a validity indicator (`Valid JSON` with check square, or `Invalid JSON · line 3` in `--danger`) and `Format`; an editor area with line numbers (mono 12.5px/20px, `--bg`); footer `Set NULL`, `Cancel`, `Done ⌘↵`. Long `text` uses the same editor without the validity indicator. `Done` is disabled while JSON is invalid.
- **Set NULL**: also `⌘⌫` on a selected cell. NULL renders as the dashed `NULL` tag.

## Pending changes (T09)
Three marks, each with a symbol, a position and a tint — never colour alone:
| Change | Gutter symbol | Row / cell |
| --- | --- | --- |
| Added row | `+` in `--ok` | whole row `--ok-tint`; columns left empty show `default` in italic `--text-3` |
| Modified cell | `~` in `--accent-text` on the row | the changed cell gets `--accent-tint` and a 4px corner triangle in `--accent` at top-left; tooltip shows the original value |
| Row marked for deletion | `−` in `--danger` | whole row `--danger-tint` and a 1px `--danger` strike line through the cells |
Symbols are mono 500 13px at the left of the 44px gutter; the row number stays at the right. Added rows are placed at the top and have no number.
**Pending bar** (36px, `--accent-tint`, top border `--line`): `3 changes` (500); breakdown `+ 1 added`, `~ 1 modified`, `− 1 deleted` (mono 12px); `Not saved to the database yet`; right: `Preview SQL` (secondary), primary `APPLY ⌘S`, `Discard` (ghost; asks for confirmation when more than one change — not drawn). The tab shows the unsaved dot. Sorting, filtering, paging or refreshing with pending changes asks first (not drawn).
Undo: `⌘Z` reverts the last edit; right-click a changed cell or row → `Revert` (menu not drawn).

## Preview SQL (T10)
Dialog 760px. On a Production connection: 1px `--warn` border, hatched band, `PRODUCTION` tag. Header `Apply 3 changes to orders`. Body: info line `One transaction. All three statements are applied together, or none of them.` + `Copy SQL`; a statement list in mono 12.5px/20px, one block per statement with the change symbol in a 28px column and the matching tint (`+` INSERT, `~` UPDATE, `−` DELETE), selectable, order = inserts, updates, deletes. On Production: field `Type the connection name to apply on Production`; the primary `APPLY 3 CHANGES` is disabled until it matches. Off Production the field is absent. Footer: `Cancel`, primary.
`APPLY ⌘S` from the pending bar opens this dialog on Production; elsewhere it applies directly (setting, default on for Staging — not drawn).

## Apply results (T11, T12)
- **Succeeded**: bar turns into a success strip (1px `--ok` top border, `--ok-tint`): check square, `3 changes applied`, mono `1 transaction · committed · 41 ms`, `Dismiss`; it leaves after 6 s. Marks are cleared, new rows get their real keys, deleted rows disappear.
- **Failed**: all changes run in one transaction, so a failure applies nothing. Error bar (1px `--danger` top border, `--danger-tint`), two lines: `Apply failed. Nothing was changed.` + `Statement 3 of 3 failed, so the transaction was rolled back. Your 3 changes are still pending.` with `Preview SQL`, `Retry`, `Discard`; second line in mono, selectable: row, key, statement kind, error code and the server message. The failing row also gets a 1.5px `--danger` outline. All pending marks stay.

## Read-only (T13)
For a table without a primary key: strip under the filter bar — lock icon, `Read-only. This table has no primary key, so a row can't be identified for editing.`, link `Add a primary key in Structure`. For a view the text is `Read-only. This is a view, so its rows can't be edited here.` (not drawn). `Add row` / `Delete row` are disabled, the view bar and footer show `Read-only` with a lock, headers show no key mark, cells do not enter edit mode; copying and the cell viewer still work.

## Large tables (T01, T14)
The total comes from table statistics first: `≈ 8.4M` with the `ESTIMATE` tag. `Count exactly` runs a count: the total shows `…`, with a spinner, `Counting rows…`, elapsed time and `Stop`. The exact number then replaces the estimate (raw, no separators) and the tag disappears. Page size menu (170px, opens upward): `100`, `500` (checked), `1000`, `5000` per page. The grid is virtualised within a page.

## Row selection (T15)
Click a gutter cell to select a row; Shift-click extends; ⌘/Ctrl-click toggles; `⌘A` selects the page. Selected rows: `--accent-tint` and a 12px check square in the gutter. **Selection bar** (36px, `--accent-tint`): `5 rows selected`, range in mono, right: `Copy ▾` (as TSV, CSV, JSON, INSERT — list not drawn), `Export…`, `Delete rows` (1px `--danger`, `--danger` text; marks the rows for deletion as pending changes, it does not delete immediately), `Clear selection`.

## States
- **T16 Loading**: 2px indeterminate bar under the header; empty ruled rows; centred box with spinner, `Loading public.orders`, elapsed time, `Cancel`. Footer numbers show `…`; paging disabled.
- **T17 Empty table**: header stays; `This table is empty` · `It has columns but no rows yet.` · `Add row ⌘⏎`.
- **T18 No match**: header and chips stay; `No rows match these filters` · `2 filters are active. The table itself has about 8.4M rows.` · `Clear filters`.
- **T19 Load error**: no header; error block with triangle, `ERROR` tag, code, condition name, message (mono 14px), a line naming the user, `Retry`, `Copy error`. Editing controls disabled.

## State
`rows[]` for the page; `page`, `pageSize`, `total: estimate(n) | counting | exact(n)`; `filters: builder(conditions[]) | raw(text)`; `sort[]`; `selection` (rows) and `activeCell`; `edit: none | editing(cell, draft, error)`; `pending: { inserts[], updates{key → {column → value}}, deletes[key] }`; `apply: idle | running | ok(ms) | failed(statementIndex, key, code, message)`; `editable: boolean` with reason (`no-primary-key | view | no-permission`).

## Not designed (explicitly)
Sort list · type-specific extra operators · WHERE syntax error · calendar picker · binary cell editor · revert menu · discard confirmation · leaving with pending changes · Copy formats list · export dialog · Columns (show/hide) list · view read-only text variant · narrow window.

## Files
- `Table Data.dc.html` — T01–T19, light + dark.
- `tokens.css` — unchanged.
- `support.js`, `_ds/` — needed only to open the design file in a browser.
