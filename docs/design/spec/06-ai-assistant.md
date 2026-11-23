# Handoff: AI assistant

## Overview
An AI assistant for the database client: the user adds their own API key; the assistant helps write, review and improve SQL. 15 screens (AI01–AI15), each 1440×900, light and dark, in the implemented **1a "Drafting table"** direction. Scenario: `acme-prod / shop.public` (Production).

Three principles shape every screen:
1. **The assistant never runs anything.** Suggested SQL is inserted into, or replaces text in, the editor. Running is always the user's action and all Production confirmations still apply.
2. **What is sent is always visible.** Default: schema names and the statement text only; no row data. Every AI surface states this and links to the exact request.
3. **No AI colour.** AI surfaces use the existing steel accent and one icon, Lucide `sparkles` (stroke 1.5, `--accent-text`).

## About the design files
`AI Assistant.dc.html` is a **design reference in HTML**, not production code. Recreate it in the existing Tauri / React / TypeScript / Tailwind app. Screen flags and sample data are in the logic class at the end of the file. Popups and dialogs use fixed coordinates in the mock.

## Fidelity
**High-fidelity** for the AI surfaces. The Settings window shell (size, left navigation) is **provisional** — the other Settings sections are not designed yet and may adjust it.

## Tokens
**No new variables.** `tokens.css` is unchanged. Severity colours reuse existing roles: error `--danger`, warning `--warn` / `--warn-text` (already used for server WARNING notices), note neutral, success `--ok`.

## Shared pieces
- **Sparkles** marks: the toolbar button, panel title, AI chat tab icon, assistant messages, the usage line, the status-bar AI item and the Settings nav item. Nowhere else.
- **Usage line** (under every answer): sparkles 11px + `<model> · <n> in · <n> out tokens`, IBM Plex Mono 11px `--text-3`. Raw numbers.
- **Severity tag**: Barlow Condensed 600 10px, letter-spacing .08em, padding 2px 4px, 1px border — `ERROR` (`--danger`), `WARNING` (`--warn` border, `--warn-text`), `NOTE` (`--line`, `--text-2`). Always text, never colour alone.
- **AI editor marks** are deliberately different from server-error marks: gutter mark is a **hollow** 8px diamond (1.5px outline in the severity colour; server errors use a filled diamond), token underline is **dotted** 2px (server errors use wavy). Tooltip starts with `AI review:`.
- Buttons, fields, switches, strips, dialogs: as in the Connections handoff.

## Screens

### AI01 — Settings › AI › Providers
Settings dialog 980×770, top 56px, backdrop `--scrim`. Header 44px `Settings`. Left navigation 200px on `--surface`: Appearance, Editor, Data, Keyboard shortcuts, **AI** (sparkles), Security, About — rows 28px, active row `--accent-tint` + 2px `--accent` inset bar, weight 500. Content: sub-tabs `Providers | Privacy` (34px, active = weight 500 + 2px `--accent` underline), body padding 13.6px 20px, gap 10.2px.
- Top row: `Default provider` and `Default model` selects (240px each). Only providers with a working key are listed.
- One bordered block per provider. Header 36px: chevron, name (500), optional `DEFAULT` tag (1px `--accent-text`), status at the right. Body padding 10.2px, grid `1fr 240px`:
  - **Ready (Anthropic)**: `API key` shows lock + `Saved in system keychain · not shown` in `--text-3` with a `Replace` link — the saved key is never loaded into the UI. `Model` select. Row: `Test` (secondary), success square, `Key works`, mono `412 ms · 9 models available`, `Remove` (danger text) at the right. Status `Ready`.
  - **Key rejected (OpenAI)**: key field holds the typed key masked, 1px `--danger` border, show/hide button, helper `Stored in the system keychain when the test succeeds.` `Model` disabled with `Test the key to load models`. Row: `Test`, danger triangle, `Test failed`, the provider's message in mono, selectable (`401 · Incorrect API key provided: sk-proj-…7Qx2`). Status `Key rejected` in `--danger` with triangle.
  - **Not set up (Google Gemini)**: collapsed header with `Not set up` and `Add key` (secondary 24px).
  - **OpenAI-compatible endpoint (Ollama)**: header shows the user's name, `OpenAI-compatible endpoint`, and an `ON THIS DEVICE` tag when the host is localhost. Fields in one row: `Name`, `Base URL`, `API key` (`None · optional`), `Model`. Row: `Test`, `Reachable`, mono `18 ms · 3 models`, `Requests to this endpoint stay on this computer.`, `Remove`.
- Last row: `+ Add OpenAI-compatible endpoint` (ghost, `--accent-text`) and the hint `Ollama, OpenRouter, Groq, Mistral and similar services`.
- Test states: idle; testing (button disabled, spinner + `Testing…` + elapsed — **not drawn**, reuse the Connections testing pattern); success; failed (message verbatim); timeout (`Timed out after 10 s` + `Retry` — not drawn).
- `Remove` asks for confirmation (not drawn; use the Remove connection dialog). Removing the default provider clears the default.

### AI02 — Settings › AI, no provider
Centred 520px column: sparkles 24px, `No AI provider yet` (Barlow Condensed 600 24px), explanation, a 2×2 grid of 36px add-buttons (`Anthropic`, `OpenAI`, `Google Gemini`, `OpenAI-compatible endpoint`, each with a plus icon; first one shows keyboard focus), and `Keys are stored in the system keychain, not in the app's settings file.`

### AI03 — Settings › AI › Privacy
Two groups of switch rows (switch, title 500, state word `On`/`Off`, description 12.5px `--text-2`; rows separated by `--line-2`).
- `WHAT IS SENT WITH A REQUEST`: `Send schema` **On** — names of tables, columns, types and indexes, no values. `Send sample rows` **Off** — up to 5 rows per table in context. `Send query results` **Off** — rows of the current result when the user asks about it.
- Line: `Always sent: your question and the statement you are working on. Never sent: passwords, hosts and other connection details.`
- `ON PRODUCTION CONNECTIONS` + PROD tag: `Send schema` On (can be turned off); `Send sample rows` and `Send query results` Off and **Locked** (row at 60% opacity, word `Locked`, switch not focusable).
- Neutral strip: `Local models. With an endpoint on this device, such as Ollama, requests do not leave this computer. The switches above still decide what the model sees.`
- Changes apply immediately; there is no Save button.

### AI04 — Ask AI panel
- Toolbar: after `Save`, a divider and `Ask AI` (sparkles, shortcut `⌘I`; Ctrl+I on Windows/Linux). While the panel is open the button is pressed: 1px `--accent`, `--accent-tint`, weight 500.
- Panel: **380px** column at the right of the workspace, below the toolbar, full height (editor and results shrink; the result grid keeps a flexible `email` column). Left border `--line`, background `--bg`. Resizable 320–560px (drag the border).
- Header 34px: sparkles, `Ask AI`, model select (mono 11.5px, 22px high), close (`Esc` when the panel has focus).
- Scope block (bottom border `--line-2`): `WORKING ON` + chip `Statement at cursor · lines 1–7` (mono 11.5px, 1px `--line`). With a selection the chip reads `Selection · lines 3–5`. Second line: `Sends this statement and the schema of 2 tables. No row data.` + `View` (opens AI15). The sentence is generated from the privacy switches and the tables referenced.
- Body: four quick actions, 30px, full width, `--panel`, with number shortcuts `1`–`4`: `Explain this query`, `Check for mistakes`, `Optimize`, `Add comments`. Hover / keyboard focus: 1px `--accent` + `--accent-tint`. Note: `Nothing is run for you. Suggestions go into the editor, and you decide when to run them.`
- Footer (top border `--line`): input `Ask about this statement…` (28px) and a 28px send button (`--accent-fill`, arrow-up). Enter sends, Shift+Enter adds a line.

### AI05 — Review result
Panel body: `YOU` label + the request; verdict row — icon + `Problems found` (500) + counts in mono (`1 error · 2 warnings · 1 note`). Verdict icon: danger triangle if any error, warning triangle in `--warn-text` if only warnings. Then one bordered block per finding, ordered by severity: severity tag, line link at the right (mono, accent), description 12.5px with identifiers in mono. `Suggest a fix` (secondary) and the usage line.
Editor: each referenced line gets the hollow diamond in the gutter; the token gets the dotted underline in the severity colour. Clicking a line link moves the caret and briefly flashes the line with `--accent-tint` (200 ms). Marks clear when the statement is edited or the panel is closed.

### AI06 — No problems found
Success strip (1px `--ok`, `--ok-tint`): check square, `No problems found`, `Checked syntax, join conditions, NULL handling and row limits against the schema. This is a review, not a guarantee.` Usage line. No editor marks.

### AI07 — Suggestion with diff
Bordered `--panel` block: header `Suggested statement` + mono `5 added · 4 removed`. Diff body, mono 11.5px/18px, two columns: an 18px marker column and the text (wraps, never scrolls sideways). Removed lines: `−` in `--danger`, row fill `--danger-tint`. Added lines: `+` in `--ok`, row fill `--ok-tint`. Unchanged lines: no marker, no fill. The marker carries the meaning; the tint only supports it.
Actions: primary `APPLY` (replaces the scoped lines; one undo step), `Insert below` (inserts after the statement, leaving the original), `Copy`, `Dismiss` (removes the block). Note: `Apply replaces lines 1–7 in the editor. It does not run the statement.` Usage line.

### AI08 — After Apply
Editor shows the new statement; changed lines carry a 2px `--accent` bar at the right edge of the gutter until the next edit or run. Panel: success strip `Applied to the editor` / `Lines 1–8 replaced. Not run yet.` with `Undo ⌘Z` (restores the previous text and the suggestion block). Then `Check again` and `Show changes`. Undo in the editor (`⌘Z`) does the same thing.

### AI09 — Advice
Shown for `Optimize` and after a review when relevant.
- **Main card** (bordered `--panel`): header tag `MISSING INDEX` (1px `--accent-text`) + line link. Body: one sentence; a two-row mono comparison (`now` / `with index`, clearly marked as an estimate); the proposed statement in a bordered mono block (selectable); on a Production connection a warning strip `Production. Building this index uses I/O and can take minutes. It is not run for you.`; actions `Open in new query` (opens a new query tab on the same connection with the statement, not run) and `Copy`.
- **Compact rows** (bordered, one line each): tag (`FULL TABLE SCAN`, `NO LIMIT`, `SELECT *`), short text, line link.
- Estimates come from the model and the schema, not from the server; label them as estimates.

### AI10 — Streaming
The answer text appears as it arrives with a 7×14px `--accent` block cursor at the end. Under it: `Stop` (stop-square icon, shortcut `Esc`) and mono `Writing… 3.1 s`. The input row is disabled while streaming. Stop keeps the partial text and adds `Stopped` in `--text-3` (not drawn). Status bar item: `<model> · writing…`.

### AI11 — Errors
Error block (1px `--danger`, `--danger-tint`): triangle + title (500), one sentence with the provider's code in mono (selectable), one action (24px, `--panel`).
| Title | Text | Action |
| --- | --- | --- |
| Invalid API key | `<Provider> rejected the key. 401 authentication_error` | Open AI settings |
| Usage limit reached | `The provider reports that the quota for this key is used up. 429 rate_limit_error` | Switch provider |
| No network | `Could not reach <host>. Nothing was sent.` | Retry |
| Model not found | `<model> is not available for this key. 404 not_found_error` | Choose model |
The mock shows all four in one panel for reference; in the app one block replaces the failed answer.

### AI12 — No provider
Pressing `Ask AI` without a provider opens the panel with: sparkles 22px, `Set up an AI provider`, explanation, primary `OPEN AI SETTINGS` (opens AI02), `Keys are stored in the system keychain.` Model select reads `No provider`; the input row is disabled.

### AI13 — AI chat tab
New tab type `AI chat`, bound to a connection (marker, name, PROD tag as usual), tab icon sparkles. Opened from the tab `+` menu, the connection context menu or the command palette (entry points not drawn).
- **Context bar** 36px: connection / database / schema select, PROD tag, `12 tables in context` (500), `Schema and messages only · no row data` (generated from the privacy switches), link `View what is sent`, model select at the right.
- **Conversation** (left, max-width 800px, padding 20.4px, gap 13.6px): each turn is a two-column row (44px label column). User: `YOU` label + text. Assistant: sparkles + text, then optional blocks.
- **SQL block**: bordered, `--bg`; header 30px with `SQL` label and `Insert into editor` (inserts at the caret of the most recently used query tab of this connection; if none, behaves like the next button), `Open in new query` (new tab, same connection, not run), `Copy`. Body mono 12.5px/20px with syntax colours, selectable. Under it: `Not run. Review it, then run it yourself.` and the usage line.
- **Input** at the bottom: 32px field, send button.
- **Right column** 300px `IN CONTEXT`: `n of m tables`, a checkbox list of tables (mono name, column count) — unchecking removes a table from the next request; below, a summary `Schema Sent / Sample rows Not sent / Query results Not sent` and `Privacy settings`.

### AI14 — Clarifying question, trimmed context
- When the request is ambiguous the assistant asks one question in weight 500 and offers quick replies: 30px buttons, left-aligned, stacked, each with the value in mono and a short gloss (`orders.created_at — when the order was placed`). First is keyboard-focused. Clicking one sends it as the user's reply. `Or type your own answer below.`
- **Context trimmed**: when the schema exceeds the model's limit, a strip under the context bar (`--accent-tint`, info icon): `Context trimmed. The schema is too large for this model, so the 40 most relevant of 212 tables are sent. Answers about other tables may be wrong.` + `Choose tables`. The right column shows which tables are in and how many were left out. In the Ask AI panel the same fact appears in the scope line (`… schema of 40 of 212 tables`) — not drawn.

### AI15 — View what is sent
Dialog 800px, top 70px. Header `What was sent` + mono `provider · model · time · tokens`. Three numbered, read-only, selectable mono blocks, each with a label and its token count: `1 · SYSTEM INSTRUCTIONS`, `2 · SCHEMA · 2 TABLES · NAMES AND TYPES ONLY`, `3 · STATEMENT · LINES 1–7`. When enabled in Privacy, further blocks appear (`SAMPLE ROWS`, `QUERY RESULT`) — not drawn. A neutral strip lists what was **not** included. Footer: `Privacy settings` link, `Copy all`, `Close` (default focus). Blocks longer than 12 lines scroll inside the block. The dialog shows the last request of the panel or chat it was opened from; before the first request it shows what would be sent.

## Behaviour rules
- Requests are made only on an explicit user action (quick action, send, quick reply). Typing never sends anything.
- Applying or inserting never triggers a run. If the inserted statement is destructive, the normal Production confirmation appears when the user runs it.
- On a Production connection every AI surface keeps the usual Production marks; sample rows and results are never sent.
- Keys live in the OS keychain; the UI never receives a saved key back.
- The status bar shows the active model (sparkles + name); clicking it opens the model select.

## State
- `providers[]`: id, kind (`anthropic | openai | gemini | compatible`), name, baseUrl, hasKey, models[], model, test: `idle | running | ok(ms, modelCount) | failed(message) | timeout`.
- `defaults`: providerId, model. `privacy`: sendSchema, sendSampleRows, sendResults, production: { sendSchema }.
- Panel per query tab: open, width, scope (`statement | selection`, line range), `thread[]` of { request, state: `streaming | done | stopped | error(kind)`, answer, findings[]{severity, line, column, text}, suggestion{before, after, applied}, advice[], usage{in, out}, requestSnapshot }.
- Chat tab: connectionId, schema scope, contextTables[], trimmed{sent, total}, `messages[]` (same answer shape plus quickReplies[]).

## Not designed (explicitly)
Other Settings sections (window shell is provisional) · provider test "testing" and "timeout" rows · remove-provider confirmation · selection scope chip · stopped-answer state · `Explain this query` and `Add comments` answers (plain text / a suggestion diff respectively) · sample-rows and results blocks in AI15 · entry points for opening an AI chat tab · chat history list and renaming · panel resize handle · narrow window.

## Files
- `AI Assistant.dc.html` — AI01–AI15, light + dark.
- `tokens.css` — unchanged.
- `support.js`, `_ds/` — needed only to open the design file in a browser.
