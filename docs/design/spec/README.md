# LewIDE — complete design handoff

Everything designed for the LewIDE desktop database client, in one package. Direction: **1a "Drafting table"** on the Industry design language — square corners, 1px hairlines, one steel-blue accent, Barlow / Barlow Condensed / IBM Plex Mono, registration marks only on the primary button and at pane junctions.

## How to use this package
- The `.dc.html` files are **design references in HTML**, not production code. Recreate them in the app's stack: **Tauri, React + TypeScript, Tailwind CSS**. Open any of them in a browser from this folder (they need `support.js`, `tokens.css` and `_ds/` beside them, plus network access for fonts).
- Each area has a spec in `docs/` with exact measures, copy, behaviour, state shape and an explicit **Not designed** list. Read the spec; use the HTML to see it.
- `tokens.css` is the single source for colour, type, spacing, elevation and density, in light and dark. Map Tailwind to these variables.
- Fidelity is **high** throughout. Screen ids (Q01, T05, …) are stable and shown as badges in the design files.

## Contents
| # | Area | Spec | Design file | Screens |
| --- | --- | --- | --- | --- |
| 0 | Brand: mark, wordmark, app icon, splash | `docs/00-brand.md` | `Logo and Loading.dc.html` (option 2a) + `brand/` | — |
| 1 | Query editor, base layout | `docs/01-query-editor-base.md` | `Query Editor.dc.html` | E01 |
| 2 | Connections | `docs/02-connections.md` | `Connections.dc.html` | C01–C14 |
| 3 | Query editor states | `docs/03-query-editor-states.md` | `Query Editor States.dc.html` | Q01–Q20 |
| 4 | Table data | `docs/04-table-data.md` | `Table Data.dc.html` | T01–T19 |
| 5 | Table structure | `docs/05-table-structure.md` | `Table Structure.dc.html` | S01–S11 |
| 6 | AI assistant | `docs/06-ai-assistant.md` | `AI Assistant.dc.html` | AI01–AI15 |
| 7 | Component library, command palette, settings, window variants | `docs/07-library-palette-settings.md` | `Component Library.dc.html`, `Palette Settings Window.dc.html` | P01–P04, G01–G06, W01, N01–N02 |
| 8 | Import and export wizard | `docs/08-import-export.md` | `Import Export.dc.html` | I01–I05, X01–X02 |
| 9 | Non-relational views | `docs/09-non-relational-views.md` | `Non-relational Views.dc.html` | D01–D03, K01–K05, V01–V02, R01–R02 |
| 10 | Clickable prototype of the main flow | `docs/10-prototype.md` | `Prototype.dc.html` | 10 steps |

## Read this first — what supersedes what
1. **The component library (area 7) wins** wherever an earlier spec describes a component state differently.
2. **Rules that apply to every screen** (the base spec and file already follow them):
   - Database types are shown with **two-letter badges**, no brand logos.
   - Numbers in grids are **raw** (`2184.50`).
   - Read-only query results say **`Read-only · query result`**.
   - Row counts in the object tree are estimates and may arrive late (hatched placeholder while pending, nothing when unavailable).
   - The title bar shows the **LewIDE mark (2a "Axis") and wordmark**.
   The earlier two-direction study (1a / 1b) is not part of this package; only 1a exists here.
3. **`tokens.css` here is the current one.** Compared with the first query-editor handoff it adds `--scrim`, `--danger-tint`, `--ok-tint`, `--on-danger`, `--scroll-thumb`, `--scroll-thumb-hover`, and corrects the dark-theme `--shadow-sm/md/lg` (the ring was removed; see `docs/07`). No other value changed.
4. **Shortcuts**: designs show macOS glyphs (`⌘↵`). On Windows and Linux every label uses the `Ctrl` form (`Ctrl+Enter`); see W01.

## Principles that hold everywhere
- **Production safety**: a connection marked Production always shows the hatched amber band, the PROD tag on its tabs and the tinted status-bar segment; destructive statements and data changes ask for the connection name.
- **Meaning is never colour alone**: every colour is paired with a symbol, shape or word (`+ ~ −` for pending changes, filled / hollow / slashed marks for connection state, tags with text).
- **Nothing runs or writes by itself**: the AI assistant only proposes; edits are pending until applied; imports write only after the preview step.
- **Server text is selectable** wherever it appears (errors, notices, statements).
- **Density**: Compact is the default (`--row-h` 28px, `--tree-h` 24px); Comfortable is 36 / 30px.
- **Motion**: only spinners, indeterminate bars and ≤120 ms fades; everything is static under `prefers-reduced-motion`.
- **Accessibility**: WCAG AA contrast, visible 2px `--accent` focus ring, full keyboard paths described per area.

## Suggested build order
1. Tokens, fonts, component library (area 7).
2. Window shell and query editor base (area 1), then its states (area 3).
3. Connections (area 2).
4. Table data (area 4) and table structure (area 5).
5. Command palette and settings (area 7).
6. AI assistant (area 6).
7. Import / export (area 8).
8. Non-relational views (area 9), one driver at a time.

Use `Prototype.dc.html` (area 10) at any point to check how the main flow should feel end to end.
9. Brand assets and splash (area 0) can land at any point; run `tauri icon` on `brand/png/app-icon-1024.png`.

## Known gaps across the package
Each spec ends with its own **Not designed** list. The larger ones: time-series and search-engine views; document and graph editing; calendar picker; export progress; chat history for the AI tab; several small menus and confirmation dialogs that reuse existing patterns. Where something is not designed, reuse the closest pattern from the component library and flag it for design review instead of inventing a new visual.

## Folder layout
```
README.md               this file
tokens.css              theme tokens (light, dark, density)
docs/                   one spec per area (00–10)
brand/                  mark, lockups, app icons (SVG + PNG)
*.dc.html               design references (open in a browser)
support.js, _ds/        runtime for the design references only
```
