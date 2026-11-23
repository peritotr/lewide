# Import and export wizard

Design file: `Import Export.dc.html` — I01–I05 (import), X01–X02 (export). Each 1440×900, light and dark. High-fidelity. No new tokens.

## Shell
Dialog 900px wide, top 70px, backdrop `--scrim`, `--shadow-lg`.
- **Header** 44px: title (`Import into public.coupons` / `Export result`), `PRODUCTION` tag and the 4px hatched `--warn` band when the target connection is Production, close button.
- **Stepper** 40px on `--surface`: steps in a row, each an 18px square + label, joined by a 28px hairline. Done = `--text-2` square with a check, label `--text-2`. Current = `--accent-fill` square with the number, label weight 500. To do = outlined square, `--text-3`. Steps are not clickable forward; done steps are clickable to go back.
- **Body** fixed 500px high, padding 20px, gap 13.6px, so the dialog never resizes between steps.
- **Footer** 52px: a status sentence at the left (12px `--text-2`), then `Back` (ghost), a secondary button (`Cancel`, `Stop import`, `Open coupons`), and the primary button (`NEXT: COLUMNS`, `IMPORT 1237 ROWS`, `DONE`).

## Import
Entry points: table data view `Import…` (in the `⋯` / Export area), sidebar table menu, command palette. Steps: `File and format`, `Columns`, `Preview`, `Import`, `Summary`.

### I01 — File and format
- File row (1px dashed `--text-3`): file icon, name (mono 500), size and modified time, `Choose file…`; the whole row is a drop target.
- `FORMAT`: four cards in a row — badge (`CSV`, `JSON`, `XLSX`, `SQL`), name, one line. Selected: 2px `--accent` frame, `--accent-tint`, check square top-right. The format is pre-selected from the file.
- Options for the selected format. CSV: `Delimiter` (Comma / Semicolon / Tab / Pipe), `Quote`, `Encoding`, `Read as NULL` (comma-separated tokens), switches `First row is a header`, `Trim spaces around values`. JSON: root path, array vs lines (not drawn). Excel: sheet, header row, range (not drawn). SQL: statements are run as they are; steps 2 and 3 are replaced by a statement list (not drawn).
- Note strip with what was detected and the row and column counts.

### I02 — Column mapping
- Top row: `Import into` (table select; `New table…` — not drawn), `Mode` (`Append` / `Replace all rows` / `Update by key`) with a one-line explanation. `Replace all rows` and `Update by key` add a warning strip (not drawn).
- Mapping table, rows 32px: `FILE COLUMN` · `SAMPLE` (first non-empty value) · arrow · `TABLE COLUMN` (select; last option `Skip this column`) · `TYPE` · `STATUS`.
  - `Matched by name` — success square.
  - `Mapped by you` — `~` in `--accent-text`.
  - Conversion warning — warning triangle, target select with a `--warn` border, and the rule that will be applied (`yes / no will be read as true / false`).
  - Skipped — whole row `--text-3`, status `Not imported`.
  - Not drawn: type that cannot be converted (error, blocks Next), a required table column with no source.
- Line under the table: table columns absent from the file and how they are filled. `Match by name again` resets manual mappings.

### I03 — Preview
- Summary: `1237 rows ready` (success square) and `3 rows have errors` (triangle, `--danger`); filter `All rows` / `Errors only`.
- Grid of the rows as they will be written (file row number at the left). A row with an error: `--danger-tint`, triangle in the gutter, the offending value with a wavy `--danger` underline, and the reason at the end of the row. The first 200 rows are checked for the preview; the full file is validated during the import.
- `If a row has an error`: `Skip the row` / `Stop the import`. Switch `Import in one transaction` (on by default).
- On Production: `Type the connection name to import into Production`; the primary button is disabled until it matches. Nothing is written before this step is confirmed (`Nothing has been written yet`).

### I04 — Progress
Percentage (Barlow Condensed 600 28px), title, mono line `768 of 1237 rows · 2140 rows/s · about 0.2 s left`; determinate bar 4px; a log box (mono 12px, last lines, auto-scroll): transaction start, one line per batch, one line per skipped row. Footer: `Stop import` (with one transaction: `Stopping rolls the transaction back`). Closing the dialog keeps the import running and reports with a toast.

### I05 — Summary
Success strip naming rows, table and connection. Four figures (Barlow Condensed 600 28px): inserted, skipped (in `--danger` when above zero), columns not imported, duration. `SKIPPED ROWS` box listing row number and reason (first 50), with `Save error report…` (CSV of the failed rows plus a reason column). Footer: `Open coupons`, primary `DONE`.
Failure (not drawn): error strip with the statement or row and the server message; with one transaction, `Nothing was imported`.

## Export
Entry points: result panel and table data `Export`, selection bar `Export…`, command palette. Steps: `Source and format`, `Columns and preview`, `Export`, `Summary`.

### X01 — Source, format, options
`Export`: `This result` / `Whole table` / `Selected rows` (only the applicable ones are enabled) with a sentence stating rows, columns and that rows beyond the limit are not included. Format cards as in import. Options for the format — SQL: `Table name in the file`, `Rows per INSERT`, `Include CREATE TABLE`; CSV: delimiter, quote, encoding, header row, NULL as; JSON: array or lines, pretty-print; Excel: sheet name (not drawn). `Save to`: path field + `Browse…`.

### X02 — Columns and preview
Left (360px): column list — grip to reorder, checkbox to include, source name, editable `NAME IN FILE`. Unchecked rows are `--text-3`. Right: preview of the first rows exactly as they will be written (mono 12px, syntax colours for SQL), with the estimated size. Footer: primary `EXPORT`.
Progress and summary reuse I04 and I05: `Whole table` exports stream in pages and can be stopped; the summary offers `Show in folder` (not drawn).

## Not designed (explicitly)
JSON, Excel and SQL import options · new table from file · Replace / Update-by-key warnings · unmappable type error · import failure summary · export options for CSV, JSON, Excel · export progress and summary · import/export of connections.
