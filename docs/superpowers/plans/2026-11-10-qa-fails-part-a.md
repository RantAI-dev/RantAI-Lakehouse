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
  keep the goal column as today. The server already keeps a KPI on a metric
  (`kpi_measure`, `rust/crates/lakehouse-bi/src/store.rs:3313`, "BI-2 review fix (BLOCKER) R1");
  it rejects the raw `"metric:…"` string at `:3401` (`Ident::new`). A plain column must still
  travel exactly as today (existing single-number tests stay green unchanged).
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

Developer: a non-Opus coding agent (Claude Sonnet 5.5), 2026-11-10. Base: `main` `46b1cc05`.
Branch: `fix/qa-fails-part-a` (local only, not pushed, no PR — the planner opens it after the
owner's browser QA, as the plan says).

### Commits (one per task, in plan order)

| Task | Commit | What it does |
| --- | --- | --- |
| T1 | `0b87f0a2` | `crypto.randomUUID().slice(0, 8)` → `newRunId().slice(0, 8)` at the four filterId sites (`use-data-table-filters.ts` ×2, `data-table-add-filter.tsx` ×2). |
| T2 | `21aeae80` | The palette's cmdk `filter` gives every asset row one score, so the server's order survives (`src/lib/palette-search.ts`, `makePaletteFilter`). |
| T3 | `d28af52b` | The Review step shows the check's bad values (`BadValuesReport`, shared with the Check step) and a "Back to Check" action; with `onBadValue === "stop"` Load is off while values do not fit. |
| T4 | `6a3183f9` | The single-number path builds its first measure with `entriesFromRows`, so a metric goes out as `{ metric }`. |
| T5 | `d20f4bdb` | `categoryLabel` (`src/lib/chart-transforms.ts`) names a NULL "(No value)" and an empty string "(Empty)" on every axis, legend and tooltip. |
| T6 | `6acae28b` | `dimensionChoices` keeps the number columns when the chart has questions; `builder-rules.ts` is unchanged. |
| T7 | `300b64d4` | "+ Add a question" stays at the limit, disabled, with "A chart takes at most five questions." under it. |
| T8 | `b2ec27c1` | The `not-run` variant carries `reason: "failure" \| "stopped"`; the page says which. |

The two plan commits are `023f0d49` (the plan) and `e82f896c` (the corrected KPI anchor).

### Verification (in the foreground, on the final code commit `dfd5d83d`; this handoff is the commit after it)

```bash
bun run typecheck   # no output, exit 0
bun run lint        # exit 0; 10 warnings, 0 errors — all pre-existing, none in a changed file
bun run test        # 3781 pass, 1 skip, 0 fail, 11289 expect() calls, 3782 tests across 359 files, 234.03s
git status          # only the four untracked files that were there before the branch
```

The review re-ran the block itself on `ebedac18` and counted 3778 pass / 1 skip / 0 fail; the three
tests that came after it are the review fixes' own (R2's one and R3's two).

Nothing else ran: only `src/` changed, so there was no cargo, no pytest and no compose check
(plan §3). Per-file runs while working: `bun test src/features/connectors/upload-file-page.test.tsx`
(88 pass / 0 fail), `src/components/data-table/data-table-add-filter.test.tsx` (2 pass / 0 fail),
`src/features/dashboards/chart-builder.test.tsx` (40 pass / 0 fail).

Two environment problems came first, and neither is code: `node_modules` was missing `three`
(a `bun install` installed it) and `.next/dev/types/validator.ts` was stale generated output from
2026-10-25 naming pages that no longer exist (removed; `.next` is gitignored). Before the
`bun install`, `bun run typecheck` failed on `three` and on that stale file.

### What the plan asked the handoff to report

- **T1, the other `randomUUID` sites.** Two, both guarded and both off the filter path:
  `src/features/copilot/use-copilot.ts:68` (`newMsgId()`: `"randomUUID" in crypto`, else an `m_…`
  fallback) and `src/features/queries/use-query-studio.ts:214` (`newTabId()`:
  `globalThis.crypto?.randomUUID?.()`, else a counter). The plan's literal pattern
  `crypto.randomUUID(` misses the second one, which is why the first handoff named only one.
- **T2, the score chosen.** The best score cmdk would have given any asset row
  (`makePaletteFilter`, `src/lib/palette-search.ts`). Every asset row gets that one score, so the
  stable sort keeps the server's order inside the group, and the group's place among the other
  groups is unchanged (cmdk orders groups by their best item). Every other item keeps cmdk's
  `defaultFilter`.
- **T5, the sites that read a category back into a filter.** `drillTarget`
  (`src/lib/chart-click.ts:137`) is the only one; the dashboard's click handler
  (`src/features/dashboards/dashboard-page.tsx:979`) sends that target's `value`, which is the
  stored value and never the label. A "(No value)"/"(Empty)" mark names no value, as the blank
  label of a NULL did before, unless a row really holds that text — so no filter on the literal
  "(No value)" is ever made. `binTarget` and `bucketTarget` refuse a placeholder label too.

### Review fixes (R1–R4)

The review (`## 6. Review`, no BLOCKER, four SHOULD-FIX) is answered by four commits, one per
finding; each cites `QA fails part A review R<n>` at the fix site and in the commit body.

| Finding | Commit | What the developer did |
| --- | --- | --- |
| R1 | `5d505948`, `ec9b3714` | With `"stop"` the load waits for the check's answer: Load is off while `check.status === "checking"` (`upload-file-page.tsx`), and the Review step says "Checking the values against the columns…". With `"null"` the load still goes (plan D4). The two tests that pressed Load right after arriving on the Review step now wait for the button. |
| R2 | `7cd48aff` | A test that takes the QA case's own path: the file is added to an existing table, the types come from the table, the check re-runs after "Add to its rows" and answers that the values do not fit. The Review step lists them and the load waits while the check runs. The stub now awaits a route that answers later (the `Answer` type has always allowed a promise). |
| R3 | `214b1460` | A test of the add-filter panel: with no `crypto.randomUUID` the seeded rule and a duplicate each get an 8-character `filterId`, and the two differ. Both tests fail on the base commit (the panel throws there). |
| R4 | `dfd5d83d` | The module comment of `query-result-tabs.tsx` names both reasons a statement did not run (a failure, or the person's Stop); the tab label stays "Not run". |

### Corrections and notes

- `d28af52b` says the two "Table step, folders" tests in `upload-file-page.test.tsx` fail on the
  base commit. **They do not**: at `46b1cc05` (base code and base test file) the file is 84 pass /
  0 fail, and on this branch 87 pass / 0 fail. The failure the developer saw was the environment
  above (the stale `node_modules`), not the code. The note stays in the commit body; this entry
  corrects it.
- The plan's T6 check has two halves: the number column is in the Dimension (X) choices, **and**
  the saved payload has no bin for it. The second half was missing and was added while writing
  this handoff; it is folded into the T6 commit, so that commit is rewritten (`3a9345ef` →
  `6acae28b`) and T7 and T8 were re-applied on top. The T6 fixture changed with it: a question's
  own dimension must line up with the chart's (both numbers, as a year column does) or the builder
  drops it and sends nothing at all.
- Anchor note for T8: `use-query-studio.ts:988` is the loop-top abort check (Stop), not the failure
  path; the failure path is the `markNotRun(index + 1)` further down. Said in the commit body.

### Not verified

- **Nothing in a browser.** The fixes are covered by unit tests only. The owner still has to run
  the QA cases on staging (LH-DATA-11-08, LH-DATA-12-03, LH-DATA-12-13, LH-DATA-11-06,
  LH-DATA-18-08, LH-BI-2-01, LH-BI-6-38, LH-BI-13-01, LH-BI-13-06, LH-QS-4-10) before the PR.
- No staging run, no screenshot, no console check, no `docker compose up` (nothing in compose
  changed).

## 6. Review

Reviewer: Claude Opus (planner), 2026-11-10, on `ebedac18` (code at `b2ec27c1`), diff
`git diff main...HEAD` against `46b1cc05`. Result: **no BLOCKER; four SHOULD-FIX (R1–R4).**

### Verification, re-run by the reviewer (foreground, on `ebedac18`)

```bash
bun run typecheck   # exit 0, no output
bun run lint        # exit 0; 10 warnings, 0 errors, in 9 files — none of them a file this branch changes
bun run test        # 3778 pass, 1 skip, 0 fail, 11253 expect() calls, 3779 tests across 358 files, 228.52s
git status          # only the four untracked files that were there before the branch
```

The handoff said 11252 `expect()` calls; this run counted 11253 with the same pass/skip/fail
counts. Not a finding (a count that varies by one between runs), noted so nobody reads it as a
quoting error.

**The new tests fail against the old code.** In a scratch worktree with the base (`main`)
versions of the 15 changed product files and this branch's tests:
`use-data-table-filters.test.tsx` 2 fail (both new), `command-palette.test.tsx` the new order
test fails, `upload-file-page.test.tsx` 85 pass / 2 fail (T3 a, b), `chart-builder.test.tsx`
38 / 2 (T4 metric, T6 choices), `chart-option.test.ts` 15 / 2 (both T5), `chart-click.test.ts`
the new test fails, `layers-section.test.tsx` 9 / 1 (T7 at the limit), `use-query-studio.test.ts`
both extended tests fail. The ones that pass on the base are the controls the plan asked for
(T3 c, T4 plain column, T7 below the limit) and T6's "no bin" test (see T6 below).

### Findings

- **R1 SHOULD-FIX (T3, D4).** `src/features/connectors/upload-file-page.tsx:220-226` turns Load
  off only when `check.status === "done"` with bad values. On the QA path itself (Table step →
  "Add to its rows" → Next) the preview re-reads with the table's types and the check re-runs
  (`use-column-choices.ts:71-99`: `CHECK_DELAY_MS` 400 ms plus the request), so the Review step
  opens with the check still `"checking"`, no list, and Load **on** (`ready` is true while
  checking, `use-column-choices.ts:116`). A person who clicks Load in that window gets the
  server's refusal, which is the failure LH-DATA-18-08 reported. The developer must: with
  `onBadValue === "stop"`, keep Load off while `choices.check.status === "checking"` too, and
  show one line on the Review step while it checks ("Checking the values against the
  columns…"). Fail closed (principle 3); this stays inside D4 (Load is off until we know the
  values fit). Cite `QA fails part A review R1` at the site.
- **R2 SHOULD-FIX (T3 test).** The three new tests in
  `src/features/connectors/upload-file-page.test.tsx:1563-1589` reach the bad values by changing
  a type on the Check step, where the list already showed on the base. None takes the path of
  the QA case: append to an existing table, types from the table, the check re-run after the
  Table step. Add one test that chooses "Add to its rows" on the Table step, lets the re-run
  check answer bad values, and asserts the Review step lists them with Load off; and, with R1,
  that Load is off while that check is still running.
- **R3 SHOULD-FIX (T1 test).** `src/hooks/use-data-table-filters.test.tsx` covers the two hook
  sites (`use-data-table-filters.ts:97`, `:160`). The plan's check asks for adding **and
  duplicating**; the duplicate site (`src/components/data-table/data-table-add-filter.tsx:101`)
  and the seeded first rule (`:82`) have no test. Add a test that renders
  `DataTableAdvancedFilterPanel` with `crypto.randomUUID` absent and asserts the seeded rule and
  a duplicate each get an 8-character `filterId`, and that the two differ. The current stub
  (`getRandomValues: (a) => a.fill(7)`) gives every call the same id, so it cannot show that;
  use a stub that varies its bytes.
- **R4 SHOULD-FIX (T8, principle 1 / rule 1).** The module comment at
  `src/features/queries/query-result-tabs.tsx:11-13` still says "Not run" is "the whole truth
  about it: the batch stopped at the failure before it". Since T8 a not-run statement may have
  been stopped by the person. Reword it to name both reasons (`reason: "failure" | "stopped"`),
  keeping that the tab label is "Not run" either way.

### Checked, no finding

- **D1 / rule 13.** Every commit body names its QA case and backlog ID; every fix site carries a
  `QA LH-…` comment.
- **D2 / rule 4.** T1 reuses `newRunId()`, T4 `entriesFromRows`, T3 `BadValuesList` (wrapped in
  `BadValuesReport`, which moved the Check step's heading rather than copying it). T5's
  `categoryLabel` is one function, imported by `chart-option.ts` and `combined-option.ts`; the
  two local `str`/`text` helpers it replaced are gone.
- **T1.** The four sites are changed. The handoff's grep is right for the plan's literal pattern
  `crypto.randomUUID(`; `src/features/queries/use-query-studio.ts:214`
  (`globalThis.crypto?.randomUUID?.()`) does not match it and already guards itself.
- **T2.** `makePaletteFilter` (`src/lib/palette-search.ts:74`) gives every asset row the best
  score any asset row would get, so the stable sort keeps the server's order and the group's
  place is unchanged (cmdk ranks a group by its best item). At most `PALETTE_ASSET_RESULT_LIMIT`
  (8) rows, so the 8×8 scoring per keystroke is not a cost.
- **T4.** A plain column still travels as its name: single-number kinds take the `default`
  branch of `measureLimits` (`src/lib/chart-shape.ts:189`, `own: false`), so `entriesFromRows`
  never wraps it as `{ column, aggregate }`. The goal column is unchanged.
- **T5 / D5.** `drillTarget` (`src/lib/chart-click.ts:142-147`) is the only place a drawn name is
  read back into a filter; a placeholder label names nothing unless a row holds that text, as a
  blank label did before. `binTarget` / `bucketTarget` refuse the placeholders. Table cells are
  not changed (D5 covers axis, legend and tooltip only).
- **T6.** The rule is narrower than the plan's (`layers.length > 0 && takesLayers(kind) &&
  !rowsMode`, `chart-builder.tsx:628`); layers only exist where `takesLayers`, so that is the
  same set. `builder-rules.ts` is unchanged. The comment's claim that `pickDimension` clears the
  bin holds (`chart-builder.tsx:638`). The "no bin" test also passes on the base, because bins
  are off beside questions in both; it is a guard, not a proof of the fix, and the choices test
  is the one that fails on the base. Accepted.
- **T7.** The changed assertion in `layers-section.test.tsx:38` is the behaviour the plan asks
  for (the button stays, off), not a weakened test; the commit body says so.
- **T8.** Every `not-run` the hook makes carries a `reason` (the type makes it required, so
  `tsc` would catch a missed one); the page shows the D3 sentence. Only the hook is tested, as
  the plan asked.
- **Commit `d28af52b`'s body** claims two "Table step, folders" tests fail on the base; they do
  not. The handoff corrects it by name; the commit is not to be amended for it.
- **No weakened test, lint rule, allowlist or threshold**; no file outside `src/` and the plan.

### Not verified by the reviewer

- Nothing in a browser and nothing on staging: the QA cases still need the owner's run before
  the PR (plan header).
- CI: the branch is not pushed. No check was compared against `main`'s CI.
- `main`'s own full suite was not re-run; nothing in this run failed, so no red-on-`main` check
  needed naming.

### Open at the end of this review

No BLOCKER. R1–R4 go back to the developer (rule 13: cite `QA fails part A review R<n>` at each
fix site and in the commit body). R1 and R2 should land before the owner's browser QA of
LH-DATA-18-08, since R1 is on that case's own path.

## 7. Slice 2: the API side (Rust, with the console change each one needs)

Same branch (`fix/qa-fails-part-a`, draft PR #453), on top of `b06fe132`. Anchors verified on
that commit (the Rust files are unchanged since `46b1cc05`). Four QA findings, all
"still broken in code" in the triage of 2026-11-10. Developer: another agent (not Opus); the
planner reviews as in slice 1.

### 7.1 Decisions

- **D6. A signed-in person without `gold:export` is forbidden, not signed out (LH-DATA-1-02).**
  Today `check_export_token` (`rust/crates/lakehouse-api/src/routes/gold.rs:202`) answers `401`
  whenever `GOLD_EXPORT_RUN_TOKEN` is set and the caller presents no matching token, and the
  console reads every `401` as an expired session (`src/services/http.ts:105-106`), so the Gold
  asset page of an Analyst loops between the page and `/login`. New rule, in the configured-token
  branch only: when the caller presented **no** token (header and query both `None`) and the
  principal is a `PrincipalId::User` (`rust/crates/lakehouse-auth/src/principal.rs:24`) without
  `gold:export`, answer `ApiError::Forbidden` (403). Unchanged: a token that is presented and
  wrong is still `401` (the unit test
  `configured_token_rejects_an_unpermissioned_session_presenting_the_wrong_token`,
  `gold.rs:1250`, stays as it is); a service principal and the no-token-configured branch
  (`503`) are unchanged. Fail closed still holds: nobody gets in who did not before.
- **D7. The console stops asking for what it may not read (LH-DATA-1-02).** In
  `src/features/catalog/open-format-card.tsx`, `LastFiveRuns` (`:35`, `listExportRuns`) and
  the `getLastExport` call (`:138`) run only when the publication says `canEdit`; for anyone
  else the card shows the publication state and the existing read-only note (`:200`) and no
  publish history. `http.ts` is not changed.
- **D8. A ClickHouse failure on an asset page is "unavailable", not "not found" (LH-DATA-13-03,
  LH-DATA-15-03 F4).** `clickhouse_asset_detail` (`rust/crates/lakehouse-api/src/routes/catalog.rs:2472`)
  turns every `clickhouse_source` error into `not_found()`. Keep `Ok(None)` → `not_found()`;
  map `Err(e)` through `upstream_error::ch_error_as(&CTX, &e, FailedAs::ServiceUnavailable)`
  (`rust/crates/lakehouse-api/src/upstream_error.rs:386`, `FailedAs` at `:57`, `Context::new`
  at `:85`) with a `const` context whose messages say the table's details could not be read
  right now. No engine text reaches the body (SEC-11; `tests/sec11_guard.rs` must stay green).
  Console: `getAsset` (`src/services/clients/assets.ts:117`) uses the file's own
  `errorFor(res.status, …)` (`:25`) instead of always `"not_found"`, so `ErrorState`
  (`src/features/catalog/asset-detail-page.tsx:109-110`) shows the right kind (permission,
  unavailable with retry, not found).
- **D9. The grant's audit entry says who got what (LH-DATA-15-03 F1, F3).**
  `routes::governance` builds each audit row (`rust/crates/lakehouse-api/src/routes/governance.rs:400-422`)
  without `args`, and writes `"source": "copilot"` for every row.
  - Add `"detail"`: a short sentence made from `args` for `catalog.grant` (and `catalog.revoke`
    if `catalog_grants.rs` records one; check), e.g.
    `query:read to QA Approver <qa-approver@…>, until 2026-12-06`. Other actions: no `detail`
    key. Build it from the stored fields only (`permission`, `grantee`, `expiresAt`,
    `catalog_grants.rs:206-215`); never echo an unknown `args` blob.
  - `"source"` follows `principal_kind`: `user` → `console`, `copilot` → `copilot`,
    `schedule` → `schedule`, `service` → `service`, none → `copilot` (the same fallback the
    actor label uses at `:412`). Grep for any reader of `source: "copilot"` first.
  - Console: `AuditEvent` (`src/services/contracts/governance.ts:258`) gains
    `detail?: string`; the audit drawer (`src/features/governance/audit-page.tsx:34-60`) shows a
    "Detail" row when it is present. `source` stays out of the console type (it is not shown).
- **D10. An S3 preview reads a file by what it is (LH-SRC-13-01 F1).** `preview_s3` takes the
  connector's configured format (`rust/crates/lakehouse-api/src/connector_listing.rs:1125`), so
  a `.parquet` object on a connector set to CSV is read as text. Choose the reader from the
  object key's extension, case-insensitive: `.parquet` → `FilesFormat::Parquet`, `.csv` →
  `Csv`, `.json` / `.jsonl` / `.ndjson` → `Json` (which already answers `NotSupported`
  honestly, `:1097`); anything else falls back to `dial.format`. Only the preview changes;
  the load path is out of scope.

### 7.2 Tasks (Rust first, in one sitting, then the console)

- **T9. D6.** Change `check_export_token`, update its doc comment (`gold.rs:179-201`) to the
  new third rule, and add unit tests next to the existing ones (`gold.rs:1185-1265`): user
  without the permission and no token → `Forbidden`; same user with a wrong token → still
  `Unauthorized`. `tests/gold_publication.rs:236` asserts `401 || 503` for a session without
  the permission and no token: if that test configures a token it will now see `403`. That
  assertion was written for the old status, not for a property; change it to the exact status
  the new rule gives and say why in the commit (rule 2: corrected, not weakened). Check
  `tests/gold_export_auth.rs` and `tests/route_auth.rs` the same way.
- **T10. D8, server side.** `catalog.rs:2472` as above; a test (unit or `tests/`, whichever the
  file's neighbours use) that an unreachable ClickHouse gives `503` with a fixed message and a
  reference, and a missing table still gives `404`. Update the `detail` doc comment
  (`catalog.rs:1708-1712`) if it no longer matches.
- **T11. D9, server side.** `governance.rs`; unit tests for the detail sentence (grant with and
  without `expiresAt`, an action with no detail) and for the source mapping, beside
  `actor_kind_of_narrows_copilot_principal_kinds_to_agent` (`:2768`).
- **T12. D10.** A small pure function for the choice (`format_for(key, configured)`), used at
  `:1125`, with unit tests for each extension, upper case, and the fallback.
- **T13. D7, console.** `open-format-card.tsx`, with a test that a read-only publication makes
  no request to `/api/gold/exports` or `/api/gold/export/{mart}` (GET).
- **T14. D8, console.** `assets.ts` `getAsset`, with tests for 403 → `permission_denied`, 503 →
  `unavailable`, 404 → `not_found`.
- **T15. D9, console.** Contract field and the drawer row, with a test.

### 7.3 Verification

Per commit: Rust → `cd rust && cargo fmt --check && cargo clippy -p lakehouse-api --all-targets -- -D warnings`
and `cargo test -p lakehouse-api <filter>` for the tests touched; TypeScript → `bun run typecheck`
and the touched test files. This VM: `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-qa-target`,
`CARGO_BUILD_JOBS=4` (11 GB RAM; never `cargo clean`). Integration tests under
`rust/crates/lakehouse-api/tests/` need a Postgres (`sqlx::test`); if none is reachable, say
*not verified locally* and name them; CI runs them on the PR. The checkout's `.env` leaks
`CONNECTOR_*` into one `state::tests` test; a failure of exactly that test is known, not
yours (run it from a copy of `rust/` outside the checkout if you need a clean count).

Before handoff, once, in the foreground:

```bash
cd rust && cargo fmt --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test -p lakehouse-api
bun run typecheck && bun run lint && bun run test
git status
```

A full `cargo test --workspace` is CI's job on this VM; quote what ran and what did not.

### 7.4 Out of scope for slice 2

DATA-12-07 F2 (Data Engineer `catalog:read`) and BI-11-07 stay out (owner / AI team).
DATA-15-03 F2 (`catalog:read` cannot be granted) is a product decision. SRC-13-01 F2 (Kafka,
REST timeouts) is environment.

## 8. Slice 2 handoff

(Developer appends here.)

## 9. Slice 2 review

(Planner appends here.)
