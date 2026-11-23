# Handoff: LewIDE brand — mark, wordmark, app icon, loading screen

## Overview
Brand assets for **LewIDE**, direction **2a "Axis"**: the letter L drawn as two table axes that cross at the corner (the crossing is the "+" registration mark used in the interface), with one steel square — a single cell.

## About the files
- `lewide-mark.svg`, `lewide-mark-reversed.svg` — the mark on light and on dark grounds. Ready to use.
- `lewide-lockup.svg`, `lewide-lockup-reversed.svg` — mark + wordmark. **The wordmark is live text in Barlow Condensed 600**; the font must be available, or convert the text to outlines before distributing.
- `app-icon.svg`, `png/app-icon-{16…1024}.png` — square, full-bleed icon for Windows and Linux.
- `app-icon-macos.svg`, `png/app-icon-macos-{128…1024}.png` — macOS icon: 824px rounded tile (radius 185px) centred on a 1024px transparent canvas.
- `Logo and Loading.dc.html` — design reference (HTML). Option **2a** at the top is the chosen one; the other options are history.

Generate the platform bundles from the masters with Tauri: `npm run tauri icon design_handoff_brand/png/app-icon-1024.png` (produces `.ico`, `.icns` and the PNG set). For the macOS `.icns`, use `app-icon-macos-1024.png` as the source instead so the icon has the system margin and corner shape.

## Mark geometry (32-unit grid)
- Vertical stroke: x = 8, from y = 2 to y = 29. Horizontal stroke: y = 26, from x = 5 to x = 30. Stroke width **3.5**, butt caps, square joins. The strokes overshoot the crossing by 3 units on each side.
- Cell: square 8×8 at (14, 12), no stroke.
- Clear space: 4 units on every side. Minimum size: 16px.
- In the title bar the mark is 18px; on the first-launch screen 44px.

## App icon geometry
- Tile fill `#1d2d3d`. Mark scaled to 72% and centred (macOS: 60% inside the rounded tile); strokes `#f2f2f3` at width **4** (heavier than the logo so it holds at 16px); cell 9×9 at (14, 11) in `#94bce3`.
- No gradients, no shadow, no gloss.

## Colours
| Use | Light ground | Dark / steel ground |
| --- | --- | --- |
| Strokes, "Lew" | `#1d1f20` | `#f2f2f3` |
| Cell | `#5980a6` | `#94bce3` |
| "IDE" | `#416180` | `#94bce3` |
| Ground | `#f2f2f3` | `#1d2d3d` (icon, steel field) |
In the app use the tokens: strokes `--text`, cell `--accent`, "IDE" `--accent-text`.

## Wordmark
`LewIDE` — one word, capitals exactly as written. Barlow Condensed 600, letter-spacing .01em. "Lew" in ink, "IDE" in steel. In the lockup the cap height matches the mark's vertical stroke; gap between mark and word = 14px at 44px type. Title bar: 15px.

## Loading screen (splash window)
480×300 window, no title bar, not resizable, centred on the primary display; background `--bg`, 1px `--line` border, `--shadow-lg`.
- Inner frame inset 28px: 1px `--line`, with a "+" registration mark on each corner (11px crosshair, `--text` at 55%), padding 28px.
- Top-left: mark 52px, then `LewIDE` (Barlow Condensed 600 34px) with the version under it (IBM Plex Mono 11.5px, `--text-3`), gap 14px.
- Bottom: status text (IBM Plex Mono 11.5px, `--text-2`), e.g. `Opening workspace…`; 8px below, a 2px progress track (`--line`) with a 30% `--accent` segment moving left → right, 1.4s linear, infinite. Under `prefers-reduced-motion` the segment is static and centred.
- Follows the system theme (light / dark tokens). Show it only if startup takes longer than 300 ms; close it when the main window is ready.
- Status texts in order: `Starting…`, `Loading settings…`, `Opening workspace…`.

## Not designed
Monochrome (single-colour) mark for tray or template icons · Windows tile and installer art · document/file-type icons · animated mark.
