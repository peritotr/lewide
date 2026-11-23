# Handoff: Table structure view

## Overview
The view that shows and changes a table's structure (tab type **table structure**, tab title `orders · Structure`). 11 screens (S01–S11), each 1440×900, light and dark, in the implemented **1a "Drafting table"** direction. Sample: `public.orders` on `acme-prod` (Production).

## About the design files
`Table Structure.dc.html` is a **design reference in HTML**, not production code. Recreate it in the existing Tauri / React / TypeScript / Tailwind app using the grid, sub-tab, bar and dialog components already built. Screen flags and sample data are in the logic class at the end of the file.

## Fidelity
**High-fidelity.** Colours, type, spacing and copy are final.

## Tokens
**No new variables.** `tokens.css` is unchanged.

## Layout
Workspace rows: tab bar 34px (tab icon: columns) · Production band 4px · **view bar** 36px · **sub-tab row** 34px · content · optional pending bar 36px.
- **View bar**: segmented `Data | Structure` (Structure selected; `Data` switches the same tab to the data view), qualified name (mono 500 12.5px), a summary in 12px `--text-2` (`Table · about 8.4M rows · 2.1 GB`), `Refresh` at the right.
- **Sub-tabs**: `Columns 14`, `Indexes 4`, `Foreign keys 2`, `Triggers 2`, `DDL` — label + count (mono 11px `--text-3`), gap 20.4px; selected = weight 500 + 2px `--accent` underline. At the right, one contextual action in `--accent-text` with a plus icon: `Add column`, `Add index`, `Add foreign key` (none for Triggers and DDL).
- **Lists** use the grid language: header row `--row-h` on `--bg` with Barlow Condensed labels, rows `--row-h`, row rule `--line-2`, cell padding 0 10px, mono 12px, names in weight 500, hover `--hover`. Missing values are an em dash in `--text-3`.

## Screens

### S01 — Columns
Columns `56px 220px 190px 110px 1fr 170px 1.3fr`: `#` (ordinal, right-aligned) · `NAME` · `TYPE` · `NULLABLE` (`NOT NULL` in ink, or `nullable` in `--text-3`) · `DEFAULT` (expression as stored) · `KEY` (key icon + `Primary key`, or arrow icon + a link to the referenced column such as `users.id`; clicking it opens the Foreign keys tab) · `COMMENT` (Barlow 12.5px `--text-2`, ellipsis, full text in a tooltip).
Double-click a row or press Enter to edit it.

### S02 — Columns, editing
- A **grip** (six dots, `--text-3`) appears at the left of every row; drag to reorder. While dragging: the source row is dimmed to 60% and a 2px `--accent` line marks the drop position.
- **Row in edit mode**: cells become 22px fields — name (focused: `--accent` border + ring), type (select with search; mono), `Nullable` checkbox, default (`No default` placeholder), comment (`Comment` placeholder). Enter commits the row, Esc cancels, Tab moves between fields.
- **Pending marks** (same grammar as the data view; symbol + tint, never colour alone): new column = `+` in `--ok` and row `--ok-tint`; modified = `~` in `--accent-text` on the row, and the changed cell gets `--accent-tint` with a corner triangle; marked for removal = `−` in `--danger`, row `--danger-tint` and a strike line. Removing is done with `⌫` or the row menu (menu not drawn); it only marks the column.
- **Pending bar**: `4 structure changes`, breakdown (`+ 1 column`, `~ 1 type, 1 position`, `− 1 column`), `Not applied to the table yet`, `Preview SQL`, primary `APPLY ⌘S`, `Discard`. The tab shows the unsaved dot and the Columns count includes the new column.

### S03 — Indexes
Columns: `NAME` · `COLUMNS` (in order, with `DESC` where set) · `UNIQUE` (key icon + `Primary key`, `Unique`, or dash) · `METHOD` (`btree`, `gin`, …) · `SIZE` (right-aligned, as reported by the server) · `CONDITION` (partial-index predicate). Note under the list: `Sizes come from the server and are approximate.` Adding or dropping an index follows the same pending flow (not drawn).

### S04 — Foreign keys
- Outgoing: `NAME` · `COLUMNS` · `REFERENCES` (link `schema.table` + columns) · `ON DELETE` · `ON UPDATE` · an `Open <table>` action that opens the target table's Structure tab.
- A second list `REFERENCED BY · 3`: constraints in other tables that point here (`NAME`, `FROM` as a link, `COLUMNS` as `order_id → id`, `ON DELETE`, `ON UPDATE`). Read-only.

### S05 — Triggers
`NAME` · `EVENT` · `TIMING` (`BEFORE · each row`) · `FUNCTION` (link; opens the function definition — not drawn) · `STATE` (filled `--ok` square + `Enabled`, or hollow square + `Disabled`). Read-only in this version.

### S06 — DDL
A bar: lock icon, `Read-only. Generated from the catalog; edit the table in the other tabs.`, `Copy`, `Open in new query`. Below, the `CREATE TABLE` statement followed by its `CREATE INDEX` statements in the editor style: line numbers, mono 12.5px/20px, syntax colours from `--syn-*`, selectable, not editable. With pending changes the DDL still shows the current table and a note `Pending changes are not included` (not drawn).

### S07 — Apply structure changes
Dialog 780px; on Production: 1px `--warn` border, hatched band, `PRODUCTION` tag. Header `Alter public.orders`.
- Statement list (mono 12.5px/20px, selectable), one block per statement with the change symbol and tint: `+ ADD COLUMN`, `~ ALTER COLUMN … TYPE`, `− DROP COLUMN`. Changes that produce no SQL are listed as a plain line (e.g. column order: `PostgreSQL cannot reorder columns`; the order is kept as a display preference).
- **Lock warning** (warning strip): shown when a statement rewrites the table or takes an exclusive lock; names the column, the approximate row count and the effect.
- **Data-loss warning** (error strip): shown for `DROP COLUMN`, type changes that can truncate, and `DROP INDEX`/constraint removals.
- On Production: `Type the connection name to apply on Production`; the button stays disabled until it matches. The button is a **danger** button (`APPLY 3 STATEMENTS`) whenever a data-loss warning is present, otherwise the primary button.
- Footer: `Copy SQL` (left), `Cancel`, apply button. Statements run in order; on engines with transactional DDL they run in one transaction, otherwise the dialog says `Statements run one by one and cannot be rolled back` (not drawn).
- Result: success strip in place of the pending bar; on failure an error bar naming the statement and the server message, with all changes kept pending (same as the data view; not drawn).

### S08 — View
Tab `v_daily_revenue · Structure`, summary `View · read-only`. Sub-tabs: `Columns` and `Definition` only. One page shows both: the column list (`#`, `NAME`, `TYPE`, `SOURCE` expression) and, under a `DEFINITION` label, the view's query in the read-only editor style with `Copy` and `Open in new query`. No edit affordances.

### S09 — Loading
2px indeterminate bar under the sub-tabs, empty ruled rows, centred box with spinner and `Reading structure of public.orders`. Sub-tab counts are hidden until loaded.

### S10 — No permission
Error block: lock icon in `--danger`, `NO PERMISSION` tag, code, the server message (mono 14px, selectable), an explanation naming the connected user and what is still readable, `Retry`, `Copy error`. Shown per sub-tab: other sub-tabs the user can read stay available.

### S11 — Sub-tab not supported by the driver
Unsupported sub-tabs stay in the row, **crossed out** in `--text-3` with no count, and remain selectable. Selected, they show a centred message: a title naming the engine and the missing feature (`ClickHouse has no foreign keys`), one sentence, and a pointer to the tabs that do exist. No error styling — this is not a failure. The driver declares which sub-tabs it supports and may rename them (e.g. `Sorting key` for ClickHouse — not drawn).

## State
`structure: loading | ready | error(code, message)` per sub-tab; `columns[]` (ordinal, name, type, nullable, default, key, comment); `indexes[]`, `foreignKeys[]`, `referencedBy[]`, `triggers[]`, `ddl`; `capabilities` from the driver; `pending: { added[], modified{name → fields}, removed[], order[] }`; `editingRow`; `apply: idle | running | ok | failed(statementIndex, message)`.

## Not designed (explicitly)
Add / edit index and foreign key forms · trigger editing · function definition view · row context menu · type picker list · DDL with pending changes note · non-transactional DDL message · apply result bars · driver-renamed sub-tabs · narrow window.

## Files
- `Table Structure.dc.html` — S01–S11, light + dark.
- `tokens.css` — unchanged.
- `support.js`, `_ds/` — needed only to open the design file in a browser.
