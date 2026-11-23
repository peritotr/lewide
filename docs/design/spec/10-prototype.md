# Clickable prototype — main flow

Design file: `Prototype.dc.html`. One 1440×900 window that walks through the main flow. Tweaks: theme (Light / Dark), title bar, density.

It is a **guided prototype**: at each step exactly one control advances the flow, and that control carries a dashed accent frame. The bar above the window names the step and has `Back` and `Restart`. Other controls are visible but inert. It shows sequence and feedback, not every option; the full specs are in the area documents.

## Steps
| # | Screen | Action | What changes | Spec |
| --- | --- | --- | --- | --- |
| 1 | First launch, no connections | Click `NEW CONNECTION` | Type picker opens | 02 · C01 |
| 2 | Choose a database type | Click `PostgreSQL` | Connection form opens on General | 02 · C02 |
| 3 | Connection form | Click `Test connection`, then `SAVE` | Success strip with server version and latency; saving closes the dialog | 02 · C03 |
| 4 | Main window, connection saved and connected | Click `orders` in the sidebar | A table-data tab opens (in the app this is a double-click) | 04 · T01 |
| 5 | Table data | Click the `status` cell in row 3 | Cell gets the active ring; the enum list opens | 04 · T06 |
| 6 | Enum list | Choose `paid` | Cell shows the modified mark, the row gets `~`, the tab gets the unsaved dot, the pending bar appears | 04 · T09 |
| 7 | Pending change | Click `APPLY` | Success strip; marks cleared | 04 · T11 |
| 8 | Change committed | Click `+` in the tab bar | A query tab opens with a statement, no results yet | 01, 03 · Q19 idle results |
| 9 | Query editor | Click `RUN` | Indeterminate bar and spinner for about a second | 03 · Q01 |
| 10 | Result | — | Result grid, `100 rows`, `184 ms`, `Read-only · query result` | 01 |

`Cancel` or the close button in either dialog returns to step 1. `Discard` at step 7 returns to step 5.

## Differences from the real app
- The sample connection is **Staging**, so no Production band or name confirmation appears; on Production, step 7 opens the SQL preview with the confirmation field first (04 · T10).
- Opening a table is a single click here; in the app it is a double-click or Enter.
- The form is pre-filled; fields are not editable.
- Only one path is wired. Errors, validation and alternative choices are not part of the prototype.
- Spinner and progress bar are animated here; in the static design files they are drawn still.
