# Handoff: Connection window and connection management

## Overview
Creating, editing and managing database connections in the desktop database client, in the **1a "Drafting table"** direction already implemented for the Query Editor. 14 screens (C01–C14), each 1440×900, light and dark.

Scenario continues `acme-prod / shop.public`. Database types are shown with **two-letter badges only — no brand logos**.

## About the design files
The files here are **design references created in HTML**, not production code. Recreate them in the app's stack (Tauri, React + TypeScript + Tailwind), reusing the components already built for the Query Editor. `Connections.dc.html` opens in a browser (needs `support.js`, `tokens.css` and `_ds/` beside it). Form fields in the mock are static boxes; build real inputs.

## Fidelity
**High-fidelity.** Colours, type, spacing and copy are final. All values come from `tokens.css`.

## Tokens
`tokens.css` is the Query Editor file with **no existing value changed**. Added:

| Variable | Light | Dark | Use |
| --- | --- | --- | --- |
| `--scrim` | `#2b2b2d` at 45% | `rgb(0 0 0 / .55)` | modal backdrop |
| `--danger-tint` | `--danger` at 12% | `--danger` at 16% | error strip fill, danger menu hover |
| `--ok-tint` | `--ok` at 12% | `--ok` at 14% | success strip fill |
| `--on-danger` | `#f5f5f8` | `#1b1d1f` | text on a `--danger` fill |

Mock-only (do not ship): `--c`, set through `data-c="1…4"`, resolves to `--c-1…--c-4` for the current connection colour.

Dark `--shadow-*` values are used on dialogs, the menu and the popover here but **have not been visually tuned** — that check is still open.

## Shared building blocks
- **Window shell**: identical to the Query Editor (title bar 38px, sidebar 264px, tab bar 34px, status bar 24px).
- **Dialog**: background `--bg`, 1px `--line`, `box-shadow: var(--shadow-lg)`, square. Header 44px (title Barlow Condensed 600 18px, padding-left 20px, bottom border `--line`). Body padding 20px. Footer 52px (top border `--line`, padding 0 20px, buttons 28px high, gap 6.8px, right-aligned). Backdrop `--scrim` covers the whole window including the title bar. Dialogs are top-anchored: 96px from the window top (plus the per-dialog offset noted below) and horizontally centred, so height changes do not move the header.
- **Field**: label 12px `--text-2`, 4px gap; input 28px high, 1px `--line`, background `--panel`, padding 0 8px. Technical values (host, port, user, database, paths, option keys/values) use IBM Plex Mono 12.5px; the connection name uses Barlow 13px. Helper text 12px `--text-3`, 6.8px above. Two-column form grid, gap 13.6px.
- **Focused field**: border `--accent` + `outline: 2px solid var(--accent); outline-offset: 2px`.
- **Disabled**: opacity .45, `cursor: not-allowed`.
- **Buttons** (28px high in dialogs): primary = `--accent-fill` fill, `--on-accent` text, Barlow Condensed 600 13px uppercase, letter-spacing .06em, padding 0 13.6px, four registration marks. Secondary = 1px `--line`, Barlow 13px, padding 0 10.2px. Ghost = no border. Danger = `--danger` fill, `--on-danger` text, Barlow Condensed 600 13px, **no registration marks**.
- **Checkbox**: 14×14 square; unchecked 1px `--text-2`; checked `--accent-fill` fill with a 10px check in `--on-accent`. **Radio**: 14px circle, 1px `--text-2`; selected 1px `--accent-fill` with an 8px dot. **Switch**: 28×16 square track, 12×12 square knob; on = track `--accent-fill`, knob right, text "On" beside it.
- **Segmented control**: 28px high, 1px `--line`, cells padding 0 10px, 12.5px; selected cell `--accent-fill` / `--on-accent`, weight 500.
- **Strip** (inline message): 1px border + tint fill, padding 6.8px 10.2px, 12.5px text, 14px icon. Warning = `--warn` / `--warn-tint`, icon `--warn-text`. Error = `--danger` / `--danger-tint`. Success = `--ok` / `--ok-tint`. Neutral = `--line`, no fill.
- **Type badge**: two letters, IBM Plex Mono 500. In dialogs 26×18, 10.5px, 1px `--text-3`. In the sidebar 24×16, 10px, 1px `--line` with a 3px left border in the connection colour.
- **Menu / popover**: background `--panel`, 1px `--line`, `--shadow-md`.

## Screens

### C01 — First launch
No connections. Sidebar: filter field, `CONNECTIONS` header with `+`, then `No connections yet. Saved connections are listed here.` (12px `--text-3`). Tab bar holds only `+`. Workspace: a 440px column centred in the pane, left-aligned, gap 13.6px: 40px logo placeholder · `Connect to a database` (Barlow Condensed 600 30px) · `Add your first connection to browse tables, run queries and edit data. Connection details are stored on this device.` (`--text-2`, line-height 1.5) · primary button 32px `+ NEW CONNECTION ⌘N` · hairline · shortcut list `⌘K Command palette`, `⌘, Settings`. Status bar: hollow status mark + `No connection`.

### C02 — Choose a database type
Dialog **1000px** wide. Header: `New connection` + `Step 1 of 2 · Choose a database type` (12.5px `--text-2`) + close. Search field 30px, focused on open, placeholder `Search database types`. Body: one row per data model — label column 124px (Barlow Condensed 600 11px uppercase `--text-3`), then a 6-column grid, gap 6.8px. Groups in order: Relational, Distributed SQL, Analytic, Document, Key-value, Vector, Graph, Time series, Search.
- **Card**: 44px high, 1px `--line`, padding 0 6.8px: badge, then name (12.5px 500) over the model label (10.5px `--text-3`). Hover `--hover`. Selected / keyboard-focused: 2px `--accent` border + `--accent-tint` fill.
- **Coming soon**: 1px **dashed** `--line` card and badge, text `--text-3`, second line `COMING SOON` (Barlow Condensed 600 10px, .1em), `cursor: not-allowed`, not focusable, tooltip `Not supported yet`. Which types are marked is illustrative — drive it from the driver registry.
- Footer: key hints `↑↓←→ Move · ↵ Continue · Esc Close`, then `Cancel` (ghost) and primary `CONTINUE WITH <TYPE>`.
- Behaviour: typing filters by name, code and model; empty groups disappear; arrow keys move in the grid; Enter or double-click continues.
- Not designed: the search "no results" state.

### C03–C08 — Connection form
Dialog **760px** wide. Header: title (`New connection` / `Edit connection`), type badge, type name, `Change type` link (new only, returns to C02 keeping entered values), close. Tabs row 34px, gap 20.4px: `General`, `SSH tunnel`, `TLS`, `Advanced`; active = weight 500 + 2px `--accent` underline; a small `ON` tag after `SSH tunnel` when the tunnel is enabled. Body is a fixed **300px** high so the dialog does not resize between tabs. Footer: `Test connection` (secondary, left) · `Cancel` (ghost) · primary save button.

- **General (C03, C08)**: row 1 — `Name` | `Environment` (segmented: Local / Staging / Production) and `Color` (four 18px squares `--c-1…4`, titled Amber / Violet / Green / Steel; selected = check inside + 1.5px `--text` frame 4px outside). Row 2 — `Host` + `Port` (84px) | `Database`. Row 3 — `User` | `Password` with a show/hide eye button, and under it the checkbox `Save in system keychain`.
- **Production (C03)**: selecting Production (a) turns the selected segment into `--warn-tint` fill, `--warn-text` text, 1px `--warn` inset, with a check icon; (b) shows a 4px hatched `--warn` band under the dialog header (same pattern as the Production band under the tab bar); (c) shows a warning strip: `Production. A warning stays visible on every tab of this connection, and DELETE, DROP, TRUNCATE or UPDATE without WHERE ask for confirmation.`; (d) changes the save label to `SAVE PRODUCTION CONNECTION`. Selecting Production also pre-selects Amber unless the user already picked a colour.
- **File-based types (C04, SQLite)**: rows 2–3 are replaced by `Database file` (path field + `Browse…`) and the helper `File-based databases open a local file. There is no server, user or password.` SSH tunnel and TLS tabs are not applicable — disable them (not drawn).
- **SSH tunnel (C05)**: switch `Use SSH tunnel`; `SSH host` + `Port` | `SSH user`; `Authentication` segmented `Password` / `Private key`; with Private key: `Private key file` + `Browse…` (helper `The host key is checked against ~/.ssh/known_hosts.`) | `Key passphrase` + `Save in system keychain`. With Password: a single `SSH password` field + keychain checkbox (not drawn). Switch off: fields disabled.
- **TLS (C06)**: radio list `Off` — Traffic is not encrypted. / `Verify certificate` — Encrypted, and the server certificate is checked against a CA. / `Encrypt only` — Encrypted, but the server's identity is not checked. `Encrypt only` shows a warning strip: `Encrypt only does not stop impersonation. Anyone who can intercept this connection can pose as the server. Use Verify certificate outside a trusted network.` `CA certificate file` + `Browse…` is enabled only for Verify certificate (drawn disabled).
- **Advanced (C07)**: `Driver options are passed to the <type> driver exactly as written.` Table `OPTION` (260px) | `VALUE` | remove button; rows are two mono fields; `+ Add option` ghost button in `--accent-text`.
- **Edit mode (C08)**: title `Edit connection`, no `Change type`, save label `SAVE CHANGES`. The saved password is never loaded or shown: the field shows a lock icon, `Saved · unchanged` and `Type to replace` in `--text-3`. Typing replaces it; clearing the field again returns to unchanged. No show/hide button until the user has typed.

**Test connection** — the result appears as a strip between body and footer (margin 0 20px 10.2px); the dialog grows downward:
- Testing (C07): neutral strip, indeterminate bar 120×4px (track `--line`, 30% `--accent` segment sliding left→right, 1.2s linear loop; static under `prefers-reduced-motion`), `Testing connection…`, elapsed time (mono), `Cancel test`. The `Test connection` button is disabled meanwhile.
- Success (C03): success strip, 14px check square, `Connection successful`, right-aligned mono `<server version> · <latency> ms`.
- Failed (C05): error strip, `Connection failed`, error code (mono `--text-2`), `Copy`; second line is the server's message verbatim, mono 12px, **selectable**, wraps.
- Timed out (C06): warning strip with clock icon, `Timed out after 10 s. No response from <host:port>. Check the host, port, firewall or SSH tunnel.` and `Retry`.
- Editing any field clears the previous result. Saving does not require a successful test.

### C09 — Unknown SSH host
Dialog 560px (extra 80px top offset). `Unknown SSH host`. Text names the host, its IP and that it is not in `~/.ssh/known_hosts`. Fingerprint box (`--panel`, 1px `--line`): header `ED25519 KEY FINGERPRINT` + `Copy`; value mono 12.5px, selectable, e.g. `SHA256:Q7vY3mB1kRz0tXh8cJf2LwNe9sUaD4oGpIyK6ZbV5nE`. Helper: `Compare this with the fingerprint from your server administrator. Trust it only if they match.` Checkbox `Add this host to known_hosts` (checked). Footer: `Cancel` (secondary, **default focus**) · primary `TRUST AND CONNECT`. Enter must not trigger the primary action.

### C10 — SSH host key changed
Security warning, **no way to proceed**. Dialog 600px, 1px `--danger` border, 4px hatched `--danger` band on top, `role="alertdialog"`. Header: danger triangle, `SSH host key has changed`, tag `CONNECTION REFUSED`. Body: explanation; a two-row table with `SAVED · LINE 14` and `RECEIVED` (label in `--danger`) fingerprints, mono, selectable; instruction to confirm with the administrator and remove the line from `known_hosts`. Footer: `Copy details` (ghost) and `Close` (secondary, default focus). There is no trust/override button. The connection row shows the failed state afterwards.

### C11 — Password prompt
Shown when connecting to a connection without a saved password. Dialog 440px (extra 120px top offset). Header `Connect to <name>` + environment tag (PROD tag and hatched band for Production). `user@host:port` in mono 12px `--text-2`. `Password` field focused on open, with show/hide. Checkbox `Save in system keychain` (unchecked). Footer `Cancel` · primary `CONNECT ↵`. A wrong password keeps the dialog open with an error strip under the field (not drawn — reuse the Failed strip).

### C12 — Connection row states
Status mark at the row's right edge, 12px slot, each with a distinct **shape** and a tooltip:
- Connected: 7px filled square `--ok`.
- Disconnected: 7px hollow square, 1px `--text-3`.
- Connecting: 10px ring, 1.5px `--accent`, one quarter open, rotating 0.8s linear (static under reduced motion). Tooltip `Connecting…`.
- Failed: 12px warning triangle `--danger`. Tooltip `Connection failed — click for details`.
- Lost: 9px square, 1px `--warn-text`, with a diagonal slash. Tooltip `Connection lost — click to reconnect`. Open tabs stay open; their toolbar is disabled (see Query Editor "disconnected tab", not designed here).
- **Error details**: clicking the failed mark (or Enter on the row) opens a 400px popover beside the row: header triangle + `<name> · Connection failed` + time; the error text verbatim, mono 12px, selectable; `Retry`, `Edit connection`, `Copy error`. Esc or outside click closes it.
- **Row counts in the tree**: counts are estimates and may arrive late. While a count is pending the row shows a 20×6px hatched placeholder (tooltip `Counting rows…`); when the driver cannot provide one, nothing is shown. Rows never shift when the number arrives (right-aligned slot).

### C13 — Context menu
Right-click or Shift+F10 on a connection row; the row keeps the selected tint. Menu 220px, padding 3.4px 0, items 26px high, padding 0 10.2px, hover/active `--accent-tint`, separators 1px `--line`. Items: `Disconnect` (or `Connect` when not connected — one item, never both) · `New query ⌘T` · separator · `Edit…` · `Duplicate` · separator · `Remove… ⌫` in `--danger` with `--danger-tint` hover. Duplicate creates `<name> copy` without the password and opens it in Edit.

### C14 — Remove confirmation
Dialog 460px, `role="alertdialog"`. `Remove <name>?` · `The connection and its saved password are removed from this app. N open tabs for this connection will close.` · `The database itself is not affected.` Footer: `Cancel` (secondary, default focus) · danger `REMOVE CONNECTION`. For a Production connection add the hatched band under the header.

## Interactions & behaviour
- `⌘/Ctrl+N` opens C02. Double-click or Enter on a connection row connects; a row without a saved password opens C11 first.
- Dialogs trap focus, close on Esc (except while a destructive action is running) and restore focus to the trigger.
- Unsaved form changes: closing asks `Discard changes?` (not drawn — use the C14 layout with a danger `DISCARD` button).
- Validation (not drawn): required fields are Name and Host or Database file; show an error strip-style message under the field, border `--danger`.
- Passwords and passphrases go to the OS keychain when the checkbox is on; otherwise they are kept in memory for the session only.

## State
- `Connection`: id, name, typeCode, environment, colour (1–4), host, port, user, database | filePath, `savePassword`, ssh { enabled, host, port, user, auth: 'password' | 'key', keyPath, savePassphrase }, tls { mode: 'off' | 'verify' | 'encrypt-only', caPath }, options[]{key, value}.
- Runtime status per connection: `disconnected | connecting | connected | failed | lost`, plus `lastError { text, code, time }`.
- Form: active tab, dirty flag, `test: idle | running(elapsed) | ok(version, latencyMs) | failed(code, text) | timeout(seconds)`.
- Host-key flow: `unknown(fingerprint, keyType, host, ip)` → user decision; `changed(saved, received, line)` → always refused.

## Not designed (explicitly)
Type-picker search with no results · field validation errors · SSH password-auth variant · disabled SSH/TLS tabs for file-based types · wrong password in C11 · discard-changes dialog · Production variants of C11 and C14 · connection groups/folders · import/export of connections · narrow window (1024×700).

## Assets
Fonts: Barlow, Barlow Condensed, IBM Plex Mono (bundle locally). Icons: Lucide, stroke 1.5 (plus, x, search, chevrons, eye, lock, check, triangle-alert, clock, database, layers, table, sliders-horizontal). No brand logos.

## Files
- `Connections.dc.html` — all 14 screens, light + dark. Screen data is in the logic class at the end of the file.
- `tokens.css` — theme tokens including the four additions above.
- `support.js`, `_ds/` — needed only to open the design file in a browser.
