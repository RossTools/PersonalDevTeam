# Local sheets: engine and file format spike (PDT-313, 2026-10-31)

Question: which open-source spreadsheet can PDT ship for local sheets (part 3 of the Documents epic, PDT-312) so agents
can drive values, formulas, formatting, several sheets and charts, with the daemon reading and writing without a browser?

**Decision: IronCalc on both sides.** The same engine runs in pdtd (the `ironcalc` crate) and in the browser
(`@ironcalc/workbook`, its React widget over the same engine in WASM). The file of record is `.xlsx`. Charts are
PDT's own: a chart spec per sheet, drawn with Chart.js. An open sheet stays live through IronCalc's own sync diffs, with
pdtd in the middle.

Prototype (throwaway, not for merging): branch `spike/PDT-313-sheet-prototype`, `spikes/sheet/README.md`.

## The web components

Versions, sizes and activity were checked on 2026-10-31 from the official repos, docs and npm.

| | Univer OSS | IronCalc | Fortune-sheet | Jspreadsheet CE 5 | x-spreadsheet |
|---|---|---|---|---|---|
| Licence | Apache-2.0 (core); Pro is commercial | MIT OR Apache-2.0 | MIT | MIT (its formula package's licence is unclear) | MIT |
| Formulas | 528 functions, async calculation | ~497 functions | ~400 (formulajs) | own engine, count not published | 11 |
| Charts | **Pro only** (`@univerjs-pro/sheets-chart`, ECharts) | none (issue #348 is open) | none (on the roadmap) | paid extension | none |
| Formatting | full: number formats, fonts, fills, borders, merge, **conditional formatting (OSS)**, data validation, filter/sort | number formats, fonts, fills, borders, merge, alignment, wrap, **conditional formatting**, named styles, themes; no filter/sort | full, including conditional formatting | basic | basic |
| `.xlsx` | **Pro only**, through a conversion server (10 free exports a month) | built in (Rust `ironcalc`, `@ironcalc/wasm-xlsx`) | FortuneExcel (MIT, separate) | paid | none |
| React, size (gzip) | `createUniver` in an effect; ~1.64 MB | `<IronCalc model>`; ~159 KB JS + ~720 KB WASM | `@fortune-sheet/react`; ~559 KB | wrapper; ~125 KB | vanilla; ~42 KB |
| Activity | v1.3.x, weekly releases, 22.8k stars | v0.9.1 (2026-10-21), commits daily, 4.2k stars, "early stage" | last commit 2025-11 (stalled) | npm 5.0.4 is from 2025-08 | dormant since 2023 |
| API for agents' edits | Facade: `getRange().setValue/setFormula/setBackground…`, command events | `Model`: `setUserInput`, `updateRangeStyle`, `addConditionalFormatting`, `newSheet`…; `flushSendQueue`/`applyExternalDiffs` | `applyOp`/`onOp`, `setCellValue`… | methods | few |
| Engine in Rust | no (TypeScript; headless only in Node) | **yes, the same engine** | no | no | no |

Also checked and ruled out:
- Handsontable is proprietary (free only for evaluation), and HyperFormula is GPLv3 or commercial.
- SheetXL depends on its commercial SDK.
- Luckysheet is archived; Univer is its successor.
- EtherCalc is CPAL and a server app.
- RevoGrid and Glide Data Grid are grids with no formulas.

### Why IronCalc over Univer

Univer has the better editor today: more functions, filter/sort, and a polished UI. But for PDT, agents do most of the
work, and they work through the daemon:
- **Computed values in the daemon.** Agents must read computed results without a browser (`sheet_read`). Only IronCalc
  evaluates formulas in Rust. With Univer, pdtd would need a Node runtime to run Univer headless, or a second engine
  that could disagree with the browser.
- **`.xlsx` is open in IronCalc and paid in Univer.**
- **Live edits come built in.** IronCalc's diffs (`flush_send_queue` / `apply_external_diffs`) are designed for keeping
  two models in sync, and they work the same in Rust and in WASM.
- **Charts are missing in both,** since Univer's are Pro. We build them either way.
- **The bundle is about a third of Univer's.**

The risks are IronCalc's age (0.9, "early stage") and the gaps below. Univer OSS stays the fallback for the editor.
It would keep IronCalc in pdtd for values and `.xlsx`, at the cost of converting between Univer's `IWorkbookData` and
IronCalc's model.

## The file format and the Rust side

| Option | For | Against |
|---|---|---|
| **`.xlsx`, written by IronCalc** (chosen) | Opens in Excel, LibreOffice and Google Sheets. Stable and versioned by a standard, unlike IronCalc's bytes. Keeps cached values, so other readers see results. Import and export cost nothing. | Slower to write than bitcode, so pdtd keeps the model in memory and saves after edits. IronCalc drops what it doesn't model (charts, images, pivots) from imported files, so warn on import. |
| IronCalc's `.ic` (bitcode of its `Workbook`) | Fast, and exactly IronCalc's model. | No version header: a field added in a later IronCalc can make old files unreadable. Nothing else opens it. |
| A component's JSON snapshot (Univer `IWorkbookData`) | Diffable. | No Rust engine reads it, and `.xlsx` would need conversion both ways. |
| A PDT JSON model | We own it. | We would have to write a converter to both the engine and `.xlsx`. |

Checked in the prototype:
- An IronCalc-made workbook (values, formulas, a cross-sheet formula, bold, fills, number formats `#,##0` and `0.0%`,
  two sheets) round-trips through `.xlsx`.
- After a headless `set`, the formulas are recalculated and their cached `<v>` values are written.

Still to check in part 3 (IronCalc has export code for all of them): conditional formatting, merged cells, frozen
panes, column widths and defined names.

**Charts are stored outside the `.xlsx`**, as a list of chart specs in the document's metadata in the store:
- Each spec has an id, a kind (bar, line, pie, scatter or area), a title, a sheet, the category range, the series
  ranges and names, an anchor cell and a size.
- A `.xlsx` export without charts is acceptable at first. Exporting with charts means adding DrawingML chart parts to
  IronCalc's zip, which is follow-up work.

## Formula evaluation

Formulas are evaluated in both places by the same engine:
- **The browser** calculates for the user as they type.
- **pdtd** calculates for agents: when it loads a file (it never trusts the cached values a file brings) and after
  every edit. `sheet_read` returns each cell's input or formula and its computed, formatted value.

Neither needs another engine. The open-source alternatives are weaker: formulajs is a function library without a
parser, formualizer is young, and xlformula_engine is unmaintained.

## Live agent edits

pdtd holds **one live `UserModel` per open sheet**:
- It loads the model when a view opens the sheet or an agent edits it.
- It saves after edits (to a temp file, then renamed over the original) and drops the model when the sheet is idle.

Messages flow like this:
1. A view loads the model's bytes (`toBytes` → `Model.fromBytes`).
2. The view's own edits leave as `flushSendQueue()` diffs, sent to pdtd every ~200 ms.
3. pdtd applies them, saves, and passes them on to the other views.
4. An agent's `sheet_edit` edits the same model in pdtd. Its diffs go to every open view over pdtd's event stream,
   where `applyExternalDiffs` applies them.

The prototype proves both directions:
- A user typed into C2 in the grid; the `.xlsx` had the new value and D2 recomputed.
- An agent's edit landed in the open view with its formulas recomputed, the selection kept and the chart redrawn.

Notes for part 3:
- **pdtd sequences the diffs.** IronCalc's diffs are plain operations (no OT or CRDT). If a view's edit races an
  agent's, the two can apply in different orders. The view should send the last sequence number it applied. When pdtd
  has applied newer diffs since, the view reloads the model from pdtd and shows a conflict notice ("last write wins
  with a notice", as part 3 allows).
- **Pin the versions.** Bytes and diffs are IronCalc's bitcode, so the `ironcalc` crate and `@ironcalc/workbook`
  must be the same version, updated together. A test should check that.
- **Redrawing.** The 0.9.1 widget has no way to redraw after external diffs. The prototype remounts it (selection and
  scroll live in the model, so they survive). IronCalc's main branch adds `IronCalcHandle.workbook.refresh()`; use it
  once it is released.

## The gaps, sized

| Gap | Smallest viable build | Size |
|---|---|---|
| **Charts** (none in any OSS component) | PDT chart specs in the store, drawn with Chart.js (MIT; tree-shaken bar, line and pie add ~60 KB gzip) from ranges read through the model, redrawn on every edit. They float over the grid at their anchor cell, positioned from `getColumnWidth`, `getRowHeight` and `getScrollX/Y`, since the widget exposes no cell geometry. Agents add and update specs through `sheet_edit`. | 2–3 days (a side strip of charts instead: 1 day). Export with charts: 3–5 more days, later |
| **Typed cell values in the browser** | The 0.9.1 WASM build has no raw-value getter, so the prototype parses formatted text for the chart. Charts get their numbers from pdtd (Rust has typed access), or from a small upstream PR. | ½ day |
| **Sort and filter** (none in IronCalc) | Sort as a pdtd operation that reorders rows with IronCalc's row moves, which keep references right. Filter as hidden rows (`set_rows_hidden`) with no filter UI. Until then `sheet_edit` returns "unsupported". | 1–2 days |
| **Theme** | Map PDT's tokens to IronCalc's `themeVariables`; `darkThemeVariables` is exported. | ½ day |
| **Bundle** | Lazy-load the sheet view; the WASM loads only when a sheet opens. | (part 3) |

## Changes to the later parts

- **Part 3 (local provider, PDT-315):**
  - A sheet is one `.xlsx` in the documents folder, plus its chart specs and comments in the store.
  - pdtd owns the live model and the diff relay above.
  - Import: `.xlsx` through IronCalc, with a warning listing the parts it drops (charts, images, pivots).
  - Export: `.xlsx` (charts later) and `.csv` per sheet.
  - Comments stay PDT's, anchored to cells.
- **Part 5 (agent tools, PDT-317):**
  - `sheet_read` returns inputs, formulas and computed values from pdtd's engine.
  - `sheet_edit` maps to IronCalc: `set_user_input`, `update_range_style` (style paths such as `font.b`, `fill.color`,
    `num_fmt`, `alignment.horizontal`), `new_sheet`/`rename_sheet`/`delete_sheet`, conditional formatting, merges,
    row and column inserts.
  - Charts are PDT specs.
  - Sort and filter are either built (above) or return "unsupported".
