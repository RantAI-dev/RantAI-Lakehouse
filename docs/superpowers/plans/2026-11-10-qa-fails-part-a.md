# QA fails, part A, slice 1: console fixes

Fixes for QA cases that failed on staging (Lark QA Cases, Lakehouse, tested 2026-11-02..09),
triaged against `main` `46b1cc05` on 2026-11-10. This slice is the console-only part
(TypeScript). Slice 2 (Rust: DATA-1-02, DATA-13-03, DATA-15-03, SRC-13-01) gets its own plan
section later; the Data Engineer role (DATA-12-07 F2) and the assistant's target line
(BI-11-07) are out of scope until the owner and the AI team decide.

Base: `main` at `46b1cc05` (every anchor below verified there). Branch:
`fix/qa-fails-part-a`. One draft PR for the whole of part A, opened after the owner's
browser QA of slice 1.

Planner and reviewer: Claude Opus. Developer: another agent (not Opus).

The owning features already exist; each task fixes a gap against an acceptance criterion that
is already signed, so there are no new decisions to sign except the wording called out below.

## 1. Decisions

- **D1. One task per commit, in this order.** Each commit body cites its QA case ID
  (`LH-DATA-11-08` …) and the backlog ID. Each fix site carries a short comment naming the
  case (rule 13), e.g. `// QA LH-BI-2-01: …`.
- **D2. No new helper where one exists** (rule 4). The UUID fallback is `newRunId()`
  (`src/lib/run-id.ts:8`); the measure conversion is `entriesFromRows`
  (`src/lib/chart-shape.ts:120`); the bad-values list is `BadValuesList`
  (`src/features/connectors/upload-columns.tsx`).
- **D3. Text on screen (English, as the console).**
  - #4: no text change.
  - #7: the Review step shows the same "N values do not fit" heading and list as the Check
    step, plus a "Back to Check" action.
  - #9: a NULL category reads **"(No value)"**, an empty string reads **"(Empty)"**.
  - #11: **"A chart takes at most five questions."**
  - #13: **"Stopped: you stopped the run before this statement finished."**
- **D4. #7 follows the chosen bad-value option.** `UploadOnBadValue` is `"stop" | "null"`
  (`src/services/contracts/uploads.ts:67`). With `"stop"` the load would stop and write
  nothing, so Load is disabled while values do not fit. With `"null"` Load stays enabled and
  the list carries a line saying those values load as empty.
- **D5. #9 changes labels only.** Click-to-filter, drill and cross-filter keep sending the raw
  value (`null` / `""`); only axis labels, legend and tooltip text change.

## 2. Tasks

### T1. The add-filter menu works without `crypto.randomUUID` (LH-DATA-11-08, LH-DATA-12-03, LH-DATA-12-13; DATA-11, DATA-12)

Over plain HTTP (staging opened by IP) `crypto.randomUUID` does not exist, so choosing any
option in "+ Filter" throws in `onSelect` and nothing is added.

- Anchors: `src/hooks/use-data-table-filters.ts:96`, `:158`;
  `src/components/data-table/data-table-add-filter.tsx:81`, `:99`.
- Change: `crypto.randomUUID().slice(0, 8)` → `newRunId().slice(0, 8)` at all four sites.
  Grep `src/` for any other `crypto.randomUUID(` and report each one in the handoff (fix it
  only if it is on the same filter path).
- Check: a unit test that stubs `globalThis.crypto` without `randomUUID` and shows adding and
  duplicating a filter produces a filter with an 8-character `filterId`.

### T2. Ctrl+K keeps the server's order for catalog assets (LH-DATA-11-06; DATA-11)

`Command.Dialog` (`src/components/command-palette.tsx:212`) has no `filter`, so cmdk scores
and re-sorts the asset rows; `forceMount` (`:320-332`) only stops it hiding them.

- Change: pass a `filter` that gives every asset row (value starts with `asset `, see `:329`)
  the same score and leaves every other item on cmdk's `defaultFilter` (exported by `cmdk`
  1.1.1). Check that the group order the palette shows today does not change; if a fixed score
  moves the asset group above or below where it is today, pick a score that keeps it, and say
  in the handoff what you chose.
- Check: a unit test that renders the palette with assets returned in order A, B, C whose
  names would score C > A under fuzzy match, types the query, and asserts the rendered order
  is A, B, C.

### T3. The Review step lists values that do not fit an existing table (LH-DATA-18-08; DATA-18)

After "Add to its rows", the types come from the table and the check re-runs, but only the
Check step shows its result. On the forward path the mismatch is first seen when Load fails.
AC `DATA-18-AC8`: "the mismatch is listed before loading".

- Anchors: `src/features/connectors/upload-file-page.tsx:213-220` (`canProceed`), `:292`
  (`UploadReviewStep`); `src/features/connectors/upload-review-step.tsx:43`;
  `src/features/connectors/upload-columns.tsx:200-230` (the Check step's section);
  `src/features/connectors/use-column-choices.ts:44` (`onBadValue`), `:52` (`fixed`).
- Change: pass the check state to the Review step and render the bad-values heading and
  `BadValuesList` there when `check.status === "done" && check.result.count > 0`, with a
  "Back to Check" action that sets the step to `CHECK_STEP`. Per D4, the Review `canProceed`
  branch is false while values do not fit and `onBadValue === "stop"`.
- Check: unit tests for Review with (a) bad values + `"stop"` → list shown, Load disabled;
  (b) bad values + `"null"` → list shown with the "load as empty" line, Load enabled;
  (c) no bad values → nothing extra, Load enabled.

### T4. A single-number chart takes a reusable metric (LH-BI-2-01; BI-2)

The kpi / gauge / progress path sends the raw row column, so a metric goes out as the string
`"metric:qa_total_wisman"` and the preview answers 400 "invalid or missing measure column".
The bar path converts through `entriesFromRows` and works.

- Anchor: `src/features/dashboards/chart-builder.tsx:1035` (`measures: [measure, …]`;
  `measure` is `measureRows[0]?.column`, `:416`).
- Change: build the first measure with `entriesFromRows(measureRows.slice(0, 1), limits.own)`;
  keep the goal column as today. Confirm against `rust/crates/lakehouse-bi/src/store.rs:3402-3408`
  that a plain column still travels in the shape the server expects for kpi/gauge/progress
  (existing single-number tests must stay green unchanged).
- Check: a unit test of the payload for a kpi on a metric (`{ metric: "qa_total_wisman" }`)
  and on a plain column (unchanged).

### T5. NULL and empty categories get their own labels (LH-BI-6-38; BI-6)

- Anchors: `src/features/dashboards/chart-option.ts:59` and `src/lib/chart-transforms.ts:13`
  (`const str = (v) => String(v ?? "")`).
- Change: a single display-label function (put it next to one of these and import it in the
  other) that returns "(No value)" for `null`/`undefined`, "(Empty)" for `""`, else
  `String(v)`. Use it where a value becomes an axis category, a legend name or tooltip text.
  Per D5, do not change what a click sends; find each place that reads a category back into a
  filter and keep it on the raw value. List those sites in the handoff.
- Check: unit tests for `buildOption` on rows holding `null`, `""` and `"Bali"`: categories are
  "(No value)", "(Empty)", "Bali"; and a test that the click value for the "(No value)" bar is
  `null`.

### T6. Number columns stay offered as the dimension when a chart has more questions (LH-BI-13-01; BI-13)

`builderRules` turns bins off when the chart has layers (`src/lib/builder-rules.ts:98`), and
`dimensionChoices` (`src/features/dashboards/chart-builder.tsx:626`) adds number columns only
when `binKind`, so a year column disappears from Dimension (X) as soon as a second question is
added. The server accepts any real column as the dimension
(`rust/crates/lakehouse-bi/src/store.rs:3729`), and "Lined up on" already offers numbers.

- Change: when `layers.length > 0`, `dimensionChoices` includes `numberColumns` (unbinned).
  Do not change `builder-rules.ts` (bins stay off with layers).
- Check: a unit test (or the existing builder test file) that with one layer a number column is
  in the Dimension (X) choices and the saved payload has no bin for it.

### T7. The question limit says why the button is off (LH-BI-13-06; BI-13)

- Anchor: `src/features/dashboards/layers-section.tsx:191-197` (button rendered only while
  `layers.length < MAX_LAYERS`; `MAX_LAYERS = 4`, `src/lib/chart-layers.ts:34`).
- Change: always render "+ Add a question"; disable it at the limit and show
  "A chart takes at most five questions." under it.
- Check: a unit test at 4 layers: button present and disabled, sentence shown; at 3: enabled,
  no sentence.

### T8. A stopped statement says it was stopped (LH-QS-4-10 F2; QS-2)

Stop and a failure both mark the remaining statements `not-run`, and the page shows "Not run:
the batch stopped at the failure before this statement." for every `not-run`.

- Anchors: `src/features/queries/use-query-studio.ts:128` (the `not-run` variant), `:973-983`
  (`markNotRun`), `:988` (after a failure), `:1021` (aborted by Stop), `:1142` (the Stop
  handler); `src/features/queries/query-studio-page.tsx:176-179`.
- Change: give the `not-run` variant `reason: "failure" | "stopped"`; set `"stopped"` at
  `:1021` and `:1142`, `"failure"` at `:988`. The page shows the D3 sentence for
  `"stopped"` and the current sentence for `"failure"`. The tab label "Not run"
  (`query-result-tabs.tsx:40`) stays.
- Check: extend `src/features/queries/use-query-studio.test.ts` (`:454`, `:505` already assert
  `not-run`) so a stopped batch carries `reason: "stopped"` and a failed one `"failure"`.

## 3. Verification

Per commit: `bun run typecheck` plus the tests of the files touched
(`bun run test <path>`). No cargo, no Python, no compose (nothing outside `src/` changes).

Before handoff, in the foreground, on the final commit:

```bash
bun run typecheck && bun run lint && bun run test
git status   # nothing generated staged
```

Do not run prettier (the repo has no prettier config; match the file's style by hand).

## 4. Out of scope

- Slice 2 (Rust): DATA-1-02 / DATA-12-07 F1, DATA-13-03 / DATA-15-03 F4, DATA-15-03 F1/F3,
  SRC-13-01.
- DATA-12-07 F2 (Data Engineer `catalog:read`, a migration): waits for the owner.
- BI-11-07 (assistant target line): waits for the AI team.
- Everything triaged as needing a product decision or as environment (BI-6-01/06, BI-6-25,
  BI-6-40, BI-9-05, RPT-1-06, SRC-7-04, DATA-17-04, DATA-35-03, BI-5-01, BI-13-03, DATA-9-37).

## 5. Handoff

(Developer appends here: commits, commands with counts, anything not verified.)

## 6. Review

(Planner appends here.)
