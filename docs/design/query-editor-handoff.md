# Handoff: Database client — Query Editor (design direction)

## Overview
Design direction study for a cross-platform database client (developers and data teams connect to many kinds of databases, browse data, run queries, edit data). This bundle covers **one screen — the Query Editor — in two candidate directions (1a, 1b), each in light and dark theme**, shown on a **Production** connection. It also defines the theme tokens the rest of the product will use.

Product name and logo are not decided: the design uses a hatched square and the text `PRODUCT NAME` as placeholders.

**Status: direction not yet chosen.** The product owner still has to pick 1a, 1b, or a mix. Ask which direction to implement before building; if unanswered, build the shared parts (tokens, grid, editor, status bar) first — they are identical in both.

## About the design files
The files in this bundle are **design references created in HTML** — they show intended look and behaviour and are not production code to copy. The task is to **recreate these designs in the target stack**:

- **Tauri** desktop app (macOS, Windows, Linux), **React + TypeScript + Tailwind CSS**.
- The app draws its own title bar (Tauri: `decorations: false` / overlay title bar). macOS: traffic lights left. Windows/Linux: minimize / maximize / close on the right.
- Mobile is out of scope, but build components so they can adapt to narrow widths later.

`Query Editor Directions.dc.html` opens directly in a browser (needs `support.js` and the `_ds/` folder next to it, and network access for fonts and logo SVGs). All styling in it is inline and references the CSS variables in `tokens.css` — read exact values from there.

In the real app replace the static mock parts with real components: a code editor (CodeMirror 6 or Monaco, themed from the `--syn-*` tokens) and a **virtualised** data grid.

## Fidelity
**High-fidelity.** Colours, typography, spacing and sizes are final for this screen. Recreate them precisely, using Tailwind mapped to the CSS variables (e.g. `theme.extend.colors.bg = 'var(--bg)'`).

## Design language (Industry system, extended)
- **Square corners everywhere** (border-radius 0), **1px hairline borders**, no filled rounded cards, no gradients, no glass/blur, no OS-specific styling.
- One accent: steel blue. Extra hues exist only for function: Production amber, connection markers, syntax, ok/danger.
- **Registration marks**: an 11×11px "+" crosshair (two 1px lines, colour `--text` at 55% alpha) centred on a corner, offset −6px. Used only on (a) all four corners of the **primary Run button**, (b) pane junctions (splitter ends), (c) the active connection in 1b's rail. Do not scatter them elsewhere.
- **Meaning is never colour-only**: every connection colour is paired with the connection name and an environment tag; status uses shape (filled square = connected, hollow square = disconnected).
- Icons: **Lucide**, stroke-width **1.5**, `currentColor`, 12–15px.
- Focus: `outline: 2px solid var(--accent); outline-offset: 2px` on `:focus-visible`. Disabled: opacity .45, `cursor: not-allowed`. Hover: `background: var(--hover)`.

### Typography
| Role | Font | Size / weight |
| --- | --- | --- |
| UI text | Barlow | 13px / 400, line-height 1.3; emphasis 500 |
| Small UI (status bar, selects) | Barlow | 12px (11.5px in status mono items) |
| Section labels, tags | Barlow Condensed 600 | 11px (tags 10px), uppercase, letter-spacing .08em |
| Primary button label | Barlow Condensed 600 | 13px, letter-spacing .06em |
| Connection title (1b panel) | Barlow Condensed 600 | 19px / 1.1 |
| Editor code | IBM Plex Mono | 13px / 20px (1a), 12.5px / 20px (1b) |
| Grid cells | IBM Plex Mono | 12.5px (1a), 12px (1b), `font-variant-numeric: tabular-nums` |
| Grid header name / type | IBM Plex Mono | 12px 500 / 11px 400 `--text-3` |
| Identifiers in tree, breadcrumbs | IBM Plex Mono | 12px; counts 11px `--text-3` |

Numbers in the grid are **right-aligned**; text left-aligned. Leave room for translation: no fixed-width text labels; labels use `white-space: nowrap` and containers flex.

### Shared small components
- **Environment tag**: Barlow Condensed 600 10px, letter-spacing .08em, padding 2px 4px, 1px border. `PROD` → border `--warn`, text `--warn-text`. `STG`, `LOCAL` → border `--line`, text `--text-2`.
- **Connection marker**: 8×8px solid square in the connection's colour (`--c-1…4`).
- **Status mark**: 7×7px square; connected = filled `--ok`; disconnected = 1px `--text-3` outline.
- **Icon button**: 24×24 (22 in section headers, 18 for tab close), no border, icon `--text-2`, hover `--hover`.
- **Select (toolbar)**: height 26px, 1px `--line`, padding 0 6.8px, 12px text, 12px chevron-down.
- **Search field**: height 26px (24 in title bar), 1px `--line`, 13px search icon, placeholder `--text-3` 12px.
- **NULL cell**: text `NULL`, italic, `--text-3`, 11px, 1px dashed `--line` box, padding 0 4px.

## Screens / Views

Both artboards are **1440×900**. Window = CSS grid, rows: title bar / body / status bar (24px).

### Shared content
- Connection: `acme-prod` — PostgreSQL 16.2, database `shop`, schema `public`, environment **Production**.
- Other connections: `acme-staging` (PostgreSQL, STG), `analytics` (ClickHouse, STG), `sessions` (Redis, LOCAL, disconnected), `catalog-docs` (MongoDB, LOCAL, disconnected), `embeddings` (Qdrant, STG, disconnected).
- Tables in `shop.public` (name · approx rows): addresses 2.9M, carts 412K, categories 86, coupons 1.2K, inventory 48K, order_items 21.7M, orders 8.4M, payments 8.1M, products 12.4K, reviews 640K, shipments 7.9M, users 1.2M. `users` is selected. Views 3, Functions 8 (1b also Sequences 12). Second schema `audit` (1a).
- Open tabs: `Top customers` (query, acme-prod, PROD, active) · `users` (table data, acme-prod, PROD) · `orders · Structure` (acme-staging) · `Query 2` (analytics, unsaved dot) · `+` new tab.
- Editor content (17 lines; caret at end of line 9; line 9 highlighted with `--hover`; a 2px `--accent` bar in the gutter spans lines 1–12 = the statement under the caret):

```sql
-- Top customers by paid revenue, last 30 days
SELECT u.id, u.email, u.country,
       COUNT(o.id) AS orders,
       SUM(o.total_cents) / 100.0 AS revenue,
       MAX(o.created_at) AS last_order
FROM users u
JOIN orders o ON o.user_id = u.id
WHERE o.status = 'paid'
  AND o.created_at >= now() - interval '30 days'
GROUP BY u.id
ORDER BY revenue DESC
LIMIT 100;

-- Order volume by status
SELECT status, COUNT(*) AS orders
FROM orders
GROUP BY status;
```
  Syntax: keywords `--syn-kw`, functions `--syn-fn`, strings `--syn-str`, numbers `--syn-num`, comments `--syn-com` italic. Line numbers `--text-3`, current line number `--text`.
- Result 1 (100 rows, shown sorted by `revenue` desc): columns `id` int8 (PK key icon, `--accent-text`), `email` text, `country` char(2), `orders` int8, `revenue` numeric (sort-desc arrow), `last_order` timestamptz. Row 5 is selected (`--accent-tint` over the whole row) with the `email` cell focused (2px `--accent` inset ring). Rows 6 and 19 have `country` = NULL. Sample rows are in the logic class of the `.dc.html` file.
- Result 2 (5 rows): `status` text, `orders` int8 — paid 6,912,408 · shipped 1,104,377 · pending 212,950 · refunded 148,221 · cancelled 61,874.
- The result is an aggregate, so the grid is **read-only**: lock icon + `Read-only · aggregate result` (1a) / `Read-only` (1b).
- Grid: row height `--row-h`; header same height, background `--bg`, bottom border `--line`; cell padding 0 10px (1a) / 0 8px (1b); row rule `--line-2`; cell rule `--line-2`; row-number column background `--bg`, text `--text-3` right-aligned, right border `--line`; row hover `--hover`.

### 1a — "Drafting table"
Grid rows: **38px** title bar / body / 24px status. Window has a 1px `--line` border.

- **Title bar** (`--surface`, bottom border `--line`, padding 0 10.2px, gap 10.2px): [macOS: three 12px circles, 1px `--text-3` outline, gap 8px] · logo placeholder 16px + `PRODUCT NAME` (Barlow Condensed 600 14px, .06em, `--text-2`) · centred command field 400×24 `Go to table, connection or command` + `⌘K` (mono 11px) · Settings icon button · [Windows/Linux: three 44px-wide full-height buttons; close hover = `--danger` fill].
- **Body**: columns **264px** sidebar / 1fr workspace.
- **Sidebar** (right border `--line`): filter field `Filter connections and objects` (6.8px inset) → header `CONNECTIONS` + `+` button → 6 connection rows (height `--tree-h`, gap 6.8px, padding 0 6.8px): chevron 12px, **engine badge** (24×16, 1px `--line`, 3px left border in connection colour, 11px monochrome logo or 2-letter code in mono 500 10px), name (ellipsis), environment tag, status mark. Active connection is weight 500 with chevron-down. → hairline + registration mark at the right end → header `OBJECTS · ACME-PROD` → tree: `shop` (database icon, indent 6.8px) → `public` (layers icon, indent 22px) → `Tables` + count (indent 36px) → table rows (indent 54px: 13px table icon `--text-3`, mono 12px name, mono 11px count right) → `Views`, `Functions` collapsed (indent 36px) → `audit` collapsed (indent 22px). Selected tree row: `--accent-tint` + 2px `--accent` inset bar on the left.
- **Workspace** (`--panel`) rows: 34px tab bar / 4px Production band / 40px toolbar / editor (1fr) / **400px** results.
  - **Tab bar** (`--surface`, bottom border `--line`): tabs are flex cells, padding 0 6.8px 0 10.2px, gap 6.8px, right border `--line`: connection marker · type icon 13px (terminal = query, table = data, columns = structure) · title · connection name (mono 11px `--text-3`) · PROD tag when production · close button. Active tab: background `--panel`, **2px inset top line in the connection colour**, title weight 500, overlaps the bar's bottom border by 1px. Inactive: `--text-2`, hover `--hover`. Unsaved = 7px round dot instead of close.
  - **Production band**: 4px tall, `repeating-linear-gradient(135deg, var(--warn) 0 6px, transparent 6px 12px)`. Rendered only when the active tab's connection is Production (collapse the row otherwise).
  - **Toolbar** (bottom border `--line`, padding 0 10.2px, gap 6.8px, buttons 26px high): **RUN** primary (fill `--accent-fill`, text `--on-accent`, 12px play icon, label `RUN`, shortcut `⌘↵` mono 11px, four registration marks) · `Run selection` · `Stop` (disabled until a query runs) · 1×16px divider · `Explain` · `Format` · `Save` (ghost buttons: 13px icon + 13px label, hover `--hover`) · right-aligned: context select `■ acme-prod / shop / public ▾` (mono 12px) · `Limit 1000 ▾` · `Auto-commit ▾`.
  - **Editor**: gutter 52px (numbers right-aligned, 14px right padding), code padding-left 12px, top padding 6.8px.
  - **Results** (top border `--line`, registration mark at top-left junction): header 34px — result-set sub-tabs `Result 1` `100 rows` / `Result 2` `5 rows` (active: 2px `--accent` inset underline, weight 500; count mono 11px `--text-3`) · read-only note · right: segmented view switch `Results | Messages | Query plan` (1px `--line`, cells padding 4px 10px, 12px; active = `--accent-fill` / `--on-accent`, weight 500) · Filter and Export icon buttons. Grid columns: `48px 104px minmax(0,1fr) 110px 110px 136px 230px`. Header cell: name + type; numeric headers right-aligned with the type before the name.
- **Status bar** (`--surface`, top border `--line`, 12px, `--text-2`, gap 13.6px): first segment full-height, background `--warn-tint`, text `--warn-text`, weight 500: status mark + `Connected · acme-prod` + `PRODUCTION` tag (non-production: no tint, normal colours) · `PostgreSQL 16.2` · `shop.public` · right-aligned: `100 rows` · `184 ms` · `Auto-commit · no open transaction` · `Ln 9, Col 49` · `UTF-8`.

### 1b — "Instrument panel"
Grid rows: **40px** title bar / body / 24px status.

- **Production frame**: a 2px `--warn` border around the whole window (non-interactive overlay above everything), only while the active tab's connection is Production.
- **Title bar with embedded tabs** (`--surface`, bottom border `--line`, children stretch full height): [macOS circles, padding 0 4px 0 14px] · 52px cell with 18px logo placeholder · **two-line tabs** (padding 0 13.6px 0 10.2px, right border `--line`, **3px inset left spine in the connection colour**): line 1 title 13px, line 2 `connection · ENV` mono 10.5px (`--warn-text` for production, else `--text-3`). Active: background `--panel`, weight 500, left border `--line`, overlaps bottom border by 1px. · `+` (38px wide) · right-aligned: `Go to anything ⌘K` button (24px high, 1px `--line`) · **PRODUCTION** tag (24px high, 1px `--warn`, text `--warn-text`, Barlow Condensed 600 11px, .12em, background `repeating-linear-gradient(135deg, var(--warn-tint) 0 4px, transparent 4px 8px)`) · [Windows/Linux window buttons]. The empty stretch is the window drag region.
- **Body**: columns **52px** rail / **248px** object panel / 1fr workspace.
- **Connection rail** (`--surface`, right border `--line`, padding 13.6px 0 10.2px, gap 10.2px): one 34×34 button per connection (1px `--line`; 17px monochrome engine logo, or the model-family glyph when no logo exists; 8×8 connection-colour square in the bottom-left corner) with a 10px Barlow Condensed environment label under it (`PROD` in `--warn-text`). Active connection: background `--panel`, border `--text-2`, four registration marks. Bottom: dashed 34×34 `+` (New connection), Settings icon. Each button has a tooltip `name · engine · environment`.
- **Object panel** (scoped to the active connection; right border `--line`): title `acme-prod` + PROD tag → status mark + `PostgreSQL 16.2 · eu-west-1` (mono 11px `--text-3`) → two selects side by side: `DB shop ▾`, `SCHEMA public ▾` (26px; prefix label Barlow Condensed 600 10px `--text-3`, value mono 12px) → `Filter objects` field → collapsible groups (26px header: chevron, Barlow Condensed 600 11px label, count right; top rule `--line-2`): `TABLES 12` (expanded; rows indent 26px, same row style as 1a), `VIEWS 3`, `FUNCTIONS 8`, `SEQUENCES 12`.
- **Workspace** (`--panel`) columns: **44px** tool rail / editor (1fr) / **616px** results.
  - **Tool rail** (`--bg`, right border `--line`, padding 10.2px 0, gap 3.4px, 28×28 buttons, tooltips with shortcuts): Run (primary fill, registration marks, `⌘↵`) · Run selection (`⇧⌘↵`) · Stop (disabled) · 16px divider · Explain (`⌘E`) · Format (`⌥⌘F`) · Save (`⌘S`) · bottom: Query history.
  - **Editor column** rows 30px / 1fr / 26px: context strip `■ acme-prod / shop / public` (mono 12px) with `Limit 1000 ▾` and `Auto-commit ▾` right-aligned (bottom rule `--line-2`) · code (gutter 44px) · shortcut hints `⌘↵ Run · ⇧⌘↵ Run selection · ⌘E Explain` (11.5px `--text-3`, keys mono `--text-2`, top rule `--line-2`).
  - **Results column** (left border `--line`, registration marks at top-left and bottom-left junctions): 34px view tabs `Results | Messages | Query plan` (active: 2px `--accent` inset underline, weight 500) + `Read-only` + Filter / Export · then **result sets stacked**, each with a 26px collapsible header (`--bg`: chevron, index chip mono 500 11px in a 1px box, statement preview mono 11.5px `--text-2` with ellipsis, `100 rows · 171 ms` mono 11px `--text-3`) followed by its grid. Set 1 takes the remaining height; set 2 is sized to its 5 rows. Grid 1 columns: `36px 64px minmax(0,1fr) 62px 62px 92px 138px` (`country` header abbreviated `ctry`; `last_order` shown to the minute). Grid 2: `36px 180px 140px 1fr`.
- **Status bar**: status mark + `Connected` · `acme-prod` · `PostgreSQL 16.2` · `shop.public` · right-aligned: `105 rows` · `184 ms` · `Auto-commit · no open transaction` · `Ln 9, Col 49` · `UTF-8`. (No tint — the frame carries the Production warning.)

## Interactions & behaviour
The design files are static mocks; implement the following.
- **Run** (`⌘/Ctrl+Enter`) runs the statement under the caret (the gutter bar shows its extent); **Run selection** (`⇧⌘/Ctrl+Enter`) runs the selection; **Stop** is enabled only while running; **Explain** (`⌘/Ctrl+E`) opens the Query plan view; **Format** reformats the SQL; **Save** (`⌘/Ctrl+S`).
- Multiple statements → multiple result sets: sub-tabs in 1a, stacked collapsible sections in 1b.
- View switch: Results / Messages / Query plan.
- Command field / button opens the command palette (`⌘/Ctrl+K`).
- Sidebar: tree nodes expand/collapse; double-click a table opens a table-data tab; sidebar is collapsible and its width resizable (drag the border); 1a's connections/objects split and the editor/results split are draggable too.
- Tabs: click to activate, close button, middle-click close, drag to reorder; unsaved dot replaces the close button until hover.
- Grid: virtualised rows; click selects a cell (row tint + cell ring); arrow keys move; column header click cycles sort; columns resizable. Read-only results cannot be edited.
- Production treatment follows the **active tab's connection**, not the window.
- Tweaks in the mock: title bar variant (macOS / Windows-Linux) → pick at runtime from the OS; density Compact / Comfortable → user setting stored in preferences.
- No animations are specified; keep transitions to ≤120ms opacity/background fades, and respect `prefers-reduced-motion`.
- Not designed yet (do not invent final visuals; stub them): running/long-query state, query error state, autocomplete popup, empty result, Messages and Query plan contents, context menus, dialogs.

## State management
- `connections[]`: id, name, engine, environment (`local | staging | production`), colour (`c-1…c-4`), status (`connected | disconnected | error`).
- `activeConnectionId`, `activeDatabase`, `activeSchema`; object tree per connection (lazy-loaded per node).
- `tabs[]`: id, kind (`query | table-data | table-structure`), title, connectionId, dirty; `activeTabId`.
- Per query tab: sql text, caret/selection, run state (`idle | running | done | error`), `resultSets[]` (columns with name/type/key flags, rows, row count, duration, editable flag), active result index, active view (`results | messages | plan`), row limit, transaction mode.
- UI prefs: theme (`light | dark | system`), density, sidebar width / collapsed, split sizes.
- Status bar derives from the active tab: connection status, engine version, db.schema, row count, duration, transaction state, caret position, encoding.

## Design tokens
All tokens are in **`tokens.css`** (light + dark, density, focus). Summary of the light theme: grounds `#f2f2f3` / `#e9e9ea` / `#f5f5f8`; ink `#1d1f20` / `#424244` / `#66676a`; accent `#5980a6`, accent text/fill `#416180`; Production `oklch(.66 .13 70)`; spacing 3.4 / 6.8 / 10.2 / 13.6 / 20.4 / 27.2px; radius 0 in app chrome; row height 28px (36 comfortable), tree row 24px (30 comfortable).

Suggested Tailwind mapping: expose each variable as a colour (`bg`, `surface`, `panel`, `text`, `text-2`, `text-3`, `line`, `line-2`, `hover`, `accent`, `accent-text`, `accent-fill`, `on-accent`, `accent-tint`, `warn`, `warn-text`, `warn-tint`, `danger`, `ok`), font families `heading` / `body` / `mono`, and `height.row = var(--row-h)`, `height.tree = var(--tree-h)`.

## Assets
- **Fonts**: Barlow, Barlow Condensed, IBM Plex Mono (Google Fonts; all SIL OFL). Bundle them locally in the Tauri app — do not load from the network.
- **Icons**: Lucide at stroke-width 1.5 (`lucide-react`). Used: play, square, chevron-right, chevron-down, search, x, plus, minus, table, columns, terminal, save, key, arrow-down, lock, sliders-horizontal, filter, download, database, layers, history. A few toolbar icons in the mock (run selection, explain, format) are simplified stand-ins — pick the closest Lucide icon.
- **Engine logos**: monochrome SVGs from **Simple Icons v13** (CC0; each brand's own trademark rules still apply — needs a legal check before release). In the mock they load from jsDelivr; **bundle them locally**. Logos used: postgresql, mysql, sqlite, cockroachlabs, clickhouse, duckdb, snowflake, mongodb, redis, neo4j. Rendered black with `filter: var(--logo-filter)` (inverted in dark) at opacity .85.
- **No logo available** → fallback: SQL Server `Ms`, Oracle `Or`, TiDB `Ti`, Qdrant `Qd`, pgvector `Pv`. In 1a the fallback is the 2-letter code in the badge; in 1b it is the **model-family glyph** (16×16, 1.1 stroke): relational, distributed SQL, analytic, document, key-value, vector, graph, time series, search — SVG paths are in the icon strip under 1b in the design file.
- Product logo: placeholder only.

## Files
- `Query Editor Directions.dc.html` — the design: options `1a` and `1b`, each light + dark at 1440×900. Template markup is inline-styled; sample data is in the logic class at the bottom of the file.
- `tokens.css` — theme tokens ready to drop into the app.
- `support.js` — runtime needed only to open the design file in a browser.
- `_ds/…/styles.css`, `_ds/…/readme.md` — the Industry design system the visual language comes from (reference).
