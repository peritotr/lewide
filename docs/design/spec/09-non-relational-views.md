# Non-relational views

Design file: `Non-relational Views.dc.html` — document D01–D03, key-value K01–K05, vector search V01–V02, graph R01–R02. Each 1440×900, light and dark. High-fidelity. No new tokens.

Sample connections: `catalog-docs` (MongoDB, LOCAL), `sessions` (Redis, LOCAL), `embeddings` (Qdrant, STG), `shop-graph` (Neo4j, STG). None is Production here; on a Production connection the usual band, tab mark, status-bar segment and confirmations apply unchanged.

## Shared
- **Window shell** as everywhere. The object tree changes with the engine: MongoDB — database › `Collections`; Redis — databases (`db0 · 48214 keys`) › key prefixes split on `:`; Qdrant — `Collections`; Neo4j — database › `Node labels` and `Relationship types`.
- **Tab icons**: document = braces, key-value = key, vector = diagonal arrow, graph = three linked nodes. Marker, connection name and environment tag as on every tab.
- **View bar** 36px: view switch or namespace select at the left, a one-line summary in 12px `--text-2`, actions at the right.
- **Status bar**: connection, environment tag, engine version, namespace, result count, duration.
- Grids, rows, tags, strips, buttons: as in the component library. Raw numbers.

## Document view — MongoDB (D01–D03)
Rows: view bar · query bar · content · footer 30px.
- **View bar**: segmented `Tree | Table | JSON`, qualified name `catalog.products`, `Collection · 12408 documents · avg 412 B`, `Add document`, `Refresh`, `Export`.
- **Query bar** (`--bg`, padding 6.8px 10.2px 10.2px): four labelled mono fields in one row — `FILTER` (flexible), `PROJECT` 150px, `SORT` 190px, `LIMIT` 70px — then `Find ↵` and `Reset`. Fields take MongoDB query documents with JSON syntax colours (keys `--syn-kw`, strings `--syn-str`, numbers `--syn-num`). Enter in any field runs the query.
- **D01 Tree**: one block per document. Document header row (24px, `--bg`): chevron, ordinal, `_id`, a one-line preview of the first fields when collapsed, and `n fields · size` at the right. Field rows (24px, mono 12.5px, indent 18px per level): chevron for objects and arrays, key in `--syn-kw`, `:`, value with syntax colour, and the BSON type at the right in 11px `--text-3` (`string`, `double`, `int32`, `date`, `objectId`, `null`, `object {2}`, `array [3]`). Selected field: `--accent-tint` + inset bar. Collapsed containers show a dimmed summary of their keys. Double-click a value to edit it in place; the type can be changed from the row menu (editing and menu not drawn).
- **D02 Table**: the grid language. Columns are the union of top-level fields in the page, with nested scalars one level deep as dotted paths (`price.amount`); deeper objects and arrays appear as chips (`array [3]`, `object {4}`) that open the cell viewer. `_id` is shortened with a dimmed prefix. Missing fields are blank; explicit nulls use the NULL tag.
- **D03 Invalid filter**: the field gets a 1px `--danger` border and a line under the bar: triangle + `Filter is not valid: a value is expected after $lt: at position 49. The last results stay below.` The previous result stays; status bar `Filter not applied`.
- **Footer**: `Documents 1–50 of 318 matching`, page control, duration, `Editable · key _id`.
- `JSON` view (not drawn): the page as one read-only mono text block with `Copy`.

## Key-value view — Redis (K01–K05)
View bar: database select (`db0`), `48214 keys · 61.3 MB used`, `Add key`, `Refresh`. Content: key list 420px | value editor.
- **Key list**: pattern field (glob, focused on open, hint `glob pattern`) and a type filter `All | STR | HASH | LIST | SET | ZSET` (mono 11px). Header `TYPE · KEY · TTL`. Rows `--row-h`: type badge (38px wide, mono 500 10px, 1px `--line`), key in mono with the prefix up to the last `:` in `--text-3`, TTL right-aligned (`23m 14s`, `41s`, `3d 4h`, `no expiry`). Selected row: `--accent-tint` + inset bar. Footer: `Scanned 2000 of about 48.2K · 14 match` + `Scan more`. Keys are listed with `SCAN`, never `KEYS`; the count is an estimate.
- **Editor header**: key name (mono 500 15px), type badge, `Rename`, `Copy key`, `Delete key` (danger outline; asks first). Second line: `Expires in <ttl>` (or `TTL no expiry`), `Set TTL…`, `Persist`, size, encoding.
- **K01 String**: mode `Text | JSON | Hex` (detected), validity indicator, `Format`; editor with line numbers; footer note `Saving runs SET … KEEPTTL`, `Revert`, primary `SAVE ⌘S`. Values over 1 MB open read-only with `Load anyway` (not drawn).
- **K02 Hash**: table `FIELD · VALUE` (280px | flexible), selected row tinted; footer: field count, `Add field`, `Delete field`. Cells edit in place.
- **K03 List**: table `INDEX · VALUE`; footer `Items 0–99 of 2140`, `Load next 100`, `Push left`, `Push right`, `Remove item`.
- **K04 Set**: single column `MEMBER`; footer `7 of 1284 members shown · unordered`, `Scan more`, `Add member`, `Remove member`.
- **K05 Sorted set**: `RANK · SCORE · MEMBER`; footer range, order switch `High to low | Low to high`, `Add member`, `Remove member`.
- Edits to hash, list, set and sorted set apply immediately, one command each, and show a toast with the command; there is no pending bar. On Production every write asks for confirmation.
- Not drawn: stream, JSON module and bitmap types (show the type badge and a read-only raw view).

## Vector search view — Qdrant (V01–V02)
View bar: collection select, `12400 points · 1536 dimensions · Cosine`, `Refresh`. Content: query panel 400px | results | optional point panel 340px.
- **Query panel** (`--bg`, padding 13.6px, gap 13.6px):
  - `QUERY` mode `Text | Vector | Point ID`.
  - **Text** (V01): text area (84px), focused; under it the embedding model and provider and the sentence `The text is sent to that provider.` Text mode needs an embedding provider from Settings › AI; without one the mode is disabled with a link to Settings (not drawn).
  - **Vector** (V02): mono text area accepting a JSON array or comma-separated numbers; validity line `1536 of 1536 values` with a success square, or the mismatch in `--danger` (`1024 of 1536 values`), which disables Search.
  - **Point ID** (not drawn): one field; searches with that point's vector.
  - `PAYLOAD FILTER`: condition chips joined by `AND` (same chip as the table filter bar) and `Add condition`.
  - `Results (top K)` and `Minimum score` fields; checkbox `Return payload`.
  - Primary `SEARCH ⌘↵` and the note `Read-only. Nothing is written.`
- **Results**: header `8 results`, mono `38 ms · highest score first · 2 below 0.50 left out`, `Export`. Columns `# · SCORE · ID · PAYLOAD`. Rows min 44px: rank; score (mono 500, four decimals) with a 70×6px bar scaled 0–1 (first row `--accent-fill`, others `--text-3`); point id; payload summary — the first text field in weight 500, tag fields as tags, a second line with the longest text field, ellipsis. Selected row: `--accent-tint` + inset bar.
- **Point panel** (V02): `POINT` + id + score; `PAYLOAD` as formatted JSON; `VECTOR` preview (first values, total count); `Find similar` (runs a search with this point), `Copy vector`, `Copy payload`.
- Not drawn: empty result, search error, collection without payload, named vectors (a select beside the mode switch).

## Graph view — Neo4j (R01–R02)
Rows: toolbar 40px · Cypher editor 104px (resizable) · result area.
- **Toolbar**: `RUN ⌘↵`, `Explain`, `Format`, `Save`; context select `shop-graph / neo4j`; `Limit 300`. The editor is the SQL editor with Cypher highlighting (keywords `--syn-kw`, labels and relationship types `--syn-fn`, strings `--syn-str`).
- **Result header** 34px: view `Graph | Table | Text`, mono summary `11 nodes · 12 relationships · 64 ms`, and a legend: one entry per node label — a 14px ring in the label's colour with its letter, then the label name.
- **R01 Canvas**: nodes are circles, radius 20, fill `--panel`, 2.5px ring. Ring colours reuse the four connection colours `--c-1…--c-4`, assigned per label in order of first appearance; **the letter inside (first letter of the label) and the legend carry the meaning**, colour only supports it. Beyond four labels, colours repeat and letters stay distinct. Caption under the node: its display property (first of `name`, `title`, `id`), 11.5px. Relationships: 1px `--text-3` lines with an arrowhead and the type in mono 9.5px caps on the line.
  - **Selected node**: an outer ring (radius 27, 2px `--accent`, `--accent-tint` fill), caption in weight 500, and its relationships drawn in `--accent` at 1.5px.
  - Interaction: drag a node to move it, double-click to expand its neighbours, scroll to zoom, drag the background to pan. Controls bottom-left: zoom out, percentage, zoom in, `Fit`. Layout is force-directed and settles within 600 ms; no continuous motion.
- **Side panel** 320px: label chip + `id`; `PROPERTIES · n` as a two-column mono list (keys `--text-3`, values with syntax colours, selectable); `RELATIONSHIPS` — type and direction, target label chip, count (`3 shown of 14`); actions `Expand neighbours`, `Hide node`, `Copy as Cypher`. Selecting a relationship shows its type, endpoints and properties in the same panel (not drawn).
- **R02 Table**: one column per returned variable; node cells are label chip + display property, relationship cells are `[:TYPE]` in mono. A note strip appears when the graph view is capped: `Graph shows the first 300 of 12480 nodes. The table has every row.` + `Raise graph limit…`.
- `Text` view (not drawn): rows as plain text.

## Not designed (explicitly)
Document editing and row menus · MongoDB aggregation pipeline · JSON view · Redis streams and module types · key creation dialog · TTL dialog · vector Point ID mode, errors and named vectors · graph relationship selection, Text view, path highlighting · time-series and search-engine views.
