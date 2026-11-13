# QA round, Dashboards (P2): radar measures, the date grouping control, a question outside the viewer's access, a refused parameter value, the builder's limits

Fixes for five backlog tasks opened by the QA round of 2026-11-02 to 11-12 on staging:
`BI-49` (under `BI-6`), `BI-50` (under `BI-9`), `BI-51` (under `BI-13`), `BI-52` (under
`BI-11`), `BI-54` (under `BI-2`, `BI-7`, `BI-10`). QA cases: `LH-BI-6-25`, `LH-BI-9-05`
(retest 2026-11-12), `LH-BI-13-03`, `LH-BI-11-05`, `LH-BI-11-06`, `LH-BI-10-08`,
`LH-BI-7-06`, `LH-BI-2-05`.

Base: `main` after pull request #486 is merged. Every anchor below was read at `7ac20f8be`
(the head of #486, which is what `main` holds after that merge). Nobody starts this slice
before that merge. The backlog rows were read at `657c2502` and their line numbers have
moved. Where an anchor is given by its text, the line it was found on at `7ac20f8be` is
added in brackets.

Two other open pull requests are near this slice:

- **#497 (`fix/qa-round-web-11`, `WEB-11` slice S1)** adds the shared mapping of a refused
  request (`deniedFor`, `readDenied`, `NO_ACCESS` in `src/services/errors.ts`). It may or
  may not be merged when this slice starts. One step depends on it, T13 step 3 (the 403
  sentence of the metrics client), and that step says what to build in either case. This
  slice does not move `src/services/clients/dashboards.ts` onto `deniedFor`: that is
  `WEB-11` slice S2.
- **#491 (`fix/engine-failure-fixed-sentence`)** changes
  `rust/crates/lakehouse-api/src/routes/query.rs`, `src/upstream_error.rs` and the test
  files `tests/query_author_exception.rs`, `tests/query_history.rs`,
  `tests/sec11_guard.rs` and `tests/upstream_errors.rs`. No task of this slice edits any of
  them. T7 reaches `upstream_error::report_ch` only through the existing `failure_of`,
  which it does not change.

Branch: `fix/qa-round-dashboards-p2`. One draft pull request for the whole slice, opened by
the planner as soon as the first commit is pushed, so CI runs the full suites.

Planner and reviewer: Claude Opus. Developer: another agent (not Opus). The developer writes
product code and tests and edits documents only where a task says so. If an anchor, a
signature or a quoted text below does not match the code, stop and report the mismatch in
the handoff; do not improvise.

What QA saw:

- `LH-BI-6-25` (fail): a radar chart has one "Measure (Y)" and no "Add measure"; two
  measures cannot be compared on a radar.
- `LH-BI-9-05` (fail, retest 2026-11-12): "Group dates by = Month" changes the date chart to
  monthly; the chart by city does not change and shows no message.
- `LH-BI-13-03` (fail): a viewer with a mask on the columns of the second question sees the
  first question drawn and, under the chart, "<the question's label>: This chart could not be
  loaded. Reference: 6dc126c3d0". The legend lists only the first question, and the message
  squeezes the bar chart's Y axis.
- `LH-BI-11-05`, `LH-BI-11-06`: a parameter value in the address that is not valid replaces
  the whole dashboard with an error message, and a value the server refused is still written
  to the address.
- `LH-BI-10-08`: "Add a level" is hidden after six levels, with no sentence.
- `LH-BI-7-06`: "Join a table" is hidden after four joins, with no sentence.
- `LH-BI-2-05`: "New metric", "New segment" and the full forms are shown without
  `metric:write`; the refusal comes only at Create, and the refusal for a segment says
  "metrics".

## 1. Decisions

### Process

- **D1. One task per commit, in the order of section 2** (Rust first, then the console, then
  the standard request, then the documents). Each commit body cites its QA case ID and
  backlog ID. Each fix site carries a short comment naming the case (rule 13), e.g.
  `// QA LH-BI-9-05 (BI-50): …`. Push after every commit.
- **D2. No new route, no migration, no new permission.** `POLICY_TABLE` and
  `tests/route_auth.rs` do not change. `GET /api/dashboard` keeps its row and only gains
  fields in its answer. The highest migration in `rust/migrations/` is `0140`. If a review
  round asks for one after all, it is `0141`; the open plan `data-p4` also expects `0141`,
  so whichever merges second renumbers.
- **D3. Every new field is additive.** A tile gains `grainNotApplied`, a question's entry
  gains `cause`, the payload gains `refusedParams`. A console that does not know them draws
  what it drew before.

### BI-49: a radar chart takes several measures

- **D4. One to six measures, each with an aggregate of its own**, the rule bar, hbar, line,
  area and a grouped table have (`src/lib/chart-shape.ts:183-184`). A radar has no reason for
  fewer: it is a grouped chart like them.
- **D5. One shape per measure over the same categories, all on one scale.** Every spoke and
  every measure share one radial scale: the largest drawn value of any measure times 1.1
  (today's rule for the one measure, `src/features/dashboards/chart-option.ts:196-199`,
  taken over all of them). A measure whose numbers are much smaller than another's is drawn
  near the centre. That is the honest picture; scaling each measure to its own maximum
  would draw 5 and 5,000,000 as the same length. The feature page says so (T15).
- **D6. With more than one measure the radar shows a legend**, one entry per measure, named
  as every other chart names a measure (`labelOf`, `chart-option.ts:111`).
- **D7. A radar has no breakdown, before and after.** The server refuses one
  (`breakdown_allowed`, `rust/crates/lakehouse-bi/src/store.rs:129-140`; the sentence at
  `:3792-3795`) and the builder does not offer one (`canBreakdown`, in `chart-builder.tsx`:
  find `const canBreakdown = kind === "bar"` [`:615`]). So the question "breakdown or several
  measures, which wins" does not arise for a radar, and the rule of the kinds that have both
  ("with a breakdown, use exactly one measure", `store.rs:3797-3801`) is never reached.
- **D8. The server does not change.** A radar is a grouped kind (`is_grouped_kind`,
  `rust/crates/lakehouse-bi/src/shape.rs:652-677`), so it already takes several measures and
  own aggregates; only the console held it at one (`measureLimits` falls to its default,
  `chart-shape.ts:196-197`). The server has no upper count of measures for any kind; six is
  the console's, as for bar and line. T1 pins what the console now relies on.
- **D9. Assistant.** No tool, schema or prompt change: `create_chart` and `update_chart`
  take a list of `measures` for every kind (`measures_schema`,
  `rust/crates/lakehouse-api/src/routes/ai/registry.rs:200-219`) and the server already
  saved a radar with several. Until this slice the tile drew only the first; after T8 the
  assistant's radar is drawn in full. `tests/fixtures/tool_schemas.json` is not regenerated.
  One standard request is added (T14). The page context needs no change: a tile's rows
  already list every result column (`src/lib/page-context-summary.ts:106`).

### BI-50: "Group dates by" says which charts it does not change

- **D10. What the server reports.** While a read applies a dashboard grain (the request's
  `grain`, or the dashboard's saved one: `applied_grain`, `routes/dashboard.rs:446-450`),
  the tile of every stored chart that has rows and that the grain did not change carries
  `grainNotApplied`, one of four typed reasons:
  - `kind`: this type of chart takes no date grouping at all (a KPI, a table, a scatter, a
    map …: every kind `Grain::kind_takes` refuses, `lakehouse-bi/src/grain.rs:427-446`);
  - `part`: the chart groups by a part of the date (day of week …), which the control never
    replaces (`grain.rs:605-608`);
  - `not_a_date`: the chart's dimension is a column that is not a date or a timestamp;
  - `no_grain`: the chart has no date grouping of its own for the control to change (a date
    dimension drawn as stored, or a dimension whose kind the read does not know).
  Nothing is added when the control changed the chart, when the chart already had that
  grain, or when the chart kept its own grain because the new one does not fit its column:
  that last case keeps today's `grainSkipped` (`routes/support.rs:975-977`).
- **D11. One function decides, next to `resolve`.** `lakehouse_bi::grain::switch_skip`
  (new, T2) takes the chart's kind, the grain the statement was built with
  (`FilteredSql::grain`, `lakehouse-bi/src/builder.rs:1502-1504`), the kind of the
  dimension's column and the dashboard's grain. `routes::support::annotate_grain_switch`
  (new, T4) calls it for each tile. `FilteredSql` gets no new field: it is built in five
  places, and a chart that is not rebuilt returns before any grain is resolved
  (`builder.rs:1722-1724`), so a field there would be empty for exactly the charts this is
  about.
- **D12. The dimension's kind comes from the columns the read already loaded**
  (`own_columns`, `routes/support.rs:729-741`). A joined column, a calculated field and a
  pivot's fields are not among them: the reason is then `no_grain`, which is true of every
  chart without a grain (rule 14: the general true sentence, not a guess).
- **D13. The console shows the mark only while the control is in use**: the answer applied
  a grain (`appliedGrain` is not null) and the control is on the page (the dashboard has a
  chart with a day-to-year grain; find `const grainedCharts =` in `dashboard-page.tsx`
  [`:453`]). It
  is the quiet icon of the existing grain marks (`grain-marker.tsx:20-30`) with the sentence
  in its tooltip and its `aria-label`.
- **D14. Not marked:** the tiles of the built-in dashboard, which are fixed SQL and are not
  annotated at all (feature page `time-grain.md:36`), text, heading, divider, section, image
  and web tiles, a tile that carries an error (it says its own error), and the tiles of an
  embed or a public link (section 4).
- **D15. Assistant.** No tool or schema change. The page context gains the reason on the
  tile's line (`summarizeTiles`, `src/lib/page-context-summary.ts:92-117`), so "why did this
  chart not change to monthly?" can be answered from what the page sent.

### BI-51: a question the viewer's access does not cover

- **D16. What counts as the viewer's access.** A question (a layer) failed because of the
  viewer's access in exactly three cases:
  1. the tenant gate answered "not found" for a table the question reads
     (`SpecRunFailure::Gated` carrying `ApiError::NotFound`; `routes/support.rs:362`);
  2. the policy rewrite refused the statement and a table it reads is not one the viewer's
     roles include (`EnforcementError::TableNotReadable`, `policy_engine.rs:313-334`);
  3. the engine refused the statement (a `ClickHouse` error with a code) and one of the
     question's own columns (its dimension, a measure's column, a column of one of its own
     filters) is masked for the viewer's roles on a table the statement reads. This is the
     case QA saw: the rewrite turns a masked number into the text `***`
     (`sql_rewrite.rs:2116-2120`) and the engine then refuses the sum.
  Everything else keeps the sentence it has today: a time limit or a Stop, the limits that
  cannot be read, any other refusal of the rewrite, a connection failure, and an engine
  failure on a question that names no masked column (that one keeps "This chart could not be
  loaded." with its reference).
- **D17. How it is told apart.** `try_run_spec_sql` flattens a refusal to a sentence
  (`support.rs:375`), and an engine failure does not know about masks. So after a question
  fails, the runner asks the policy once more, with one new function,
  `PolicyEngineObligations::access_cause` (T6): the same table-access check the rewrite
  makes, then the masks of the tables the statement reads. It runs only for a question that
  already failed. A masked column reached through a join (`table__column`), a calculated
  field, a metric, or a SQL source that renames it is not recognised: such a question keeps
  the reference sentence (section 4). Nothing is guessed in the other direction.
- **D18. What the entry carries.** `cause: "access"` and
  `error: "Not available with your access."`; `errorId` is removed. The engine's text is
  still logged under its reference by `upstream_error::report_ch` exactly as before; only
  the reference is not shown, because it would send the viewer to an operator for something
  that is their own access. No column, mask or policy name is in the entry. This sentence
  is not `deniedFor`'s business (#497): `deniedFor` words a request the API refused with
  401 or 403, while here the dashboard's read answered 200 and the sentence is one
  question's entry inside a tile, about the viewer's access to data.
- **D19. The rewrite keeps its behaviour.** The loop in `rewrite_sql_for_roles` that finds a
  table outside the roles' access moves into one private helper that the rewrite and
  `access_cause` both call (rule 4: one copy). No status, sentence or order changes.
- **D20. The console.** A question with `cause: "access"`:
  - is listed in the legend, greyed, as `<label> (not available with your access)`. It is a
    series with no points on the axis the chart's own first series uses, so it adds no axis;
  - is not in the red list under the chart ("Questions that could not be drawn");
  - is named in one muted line under the chart, on a single line that is cut with an
    ellipsis: `Not available with your access: <label>, <label>`. The legend can be hidden
    and cannot be read without a pointer, so the text is also on the page. It is the same
    one-line size as the existing "not every value is shown" line; the three-line block
    that squeezed the axis is gone.
  A question that failed for another reason keeps the red entry with its sentence and
  reference.
- **D21. The chart's own question is not changed.** A tile whose base question the viewer
  may not read keeps today's tile error. This task is about the questions added to a chart.
- **D22. Assistant.** No tool or schema change. The page context already writes each
  question's `error` (`page-context-summary.ts:114`), which now reads "Not available with
  your access." for such a question; it names no column.

### BI-52: a parameter value that is not valid

- **D23. Only the dashboard page's read changes.** `GET /api/dashboard`, as the console's
  page calls it, no longer fails when a parameter value does not fit. Every other read
  stays strict and
  answers 400 as today: the PDF's print view, a scheduled report and a chart alert (they
  build their query with `DashboardQuery::for_render`, `routes/dashboard.rs:85-100`: a
  document that silently used another value would be a different document), the builder's
  preview, the record list and the exports.
- **D24. The value in force.** A parameter whose value is refused takes the value it has
  without the request: the dashboard's saved value, else the parameter's default. That is
  what `ParamValues::resolve` already does for a parameter the request does not name
  (`lakehouse-bi/src/params.rs:673-681`) and what Reset shows.
- **D25. A name that is not a parameter is ignored** (a link kept after a parameter was
  deleted). It is not repeated anywhere, as today (`params.rs:666-670`), and it is not in
  the list of refusals, which is keyed by defined parameters. `params` that is not a JSON
  object is still a 400: the console never sends that (`readParamsParam`,
  `src/lib/params.ts:92-105`).
- **D26. The answer lists the refusals:** `refusedParams: [{ "name", "reason" }]`, an empty
  list when there are none. `name` is the definition's name, read from the store, never the
  request's text. `reason` is the fixed sentence of `check_value`
  (`params.rs:478-545`), which names the parameter and the rule and never repeats the value.
- **D27. The security check stays.** A refused value is dropped where it is checked; the
  set of values a statement is built from (`ParamValues`) holds only checked values, so a
  refused value reaches no statement and no response. T5 proves it with the rewrite tests'
  own helpers.
- **D28. The console.** The refusal is shown under that parameter's control, the control
  goes back to the value in force, and the dashboard is drawn. A refusal for a parameter
  the dashboard does not show is listed after the controls. The refusal stays until the
  person changes that parameter again, presses Reset, or opens another dashboard.
- **D29. The address.** The console keeps writing a new value to the address first: the
  address is how a value reaches the request. When the answer refuses it, the console takes
  it out of the address at once (and takes out any name the answer does not know), so the
  address never keeps a refused value. Asking first and writing afterwards would need a
  second place that holds values between the control and the request; that is not built
  (section 4). Taking a value out of the address starts one more read of the dashboard.
- **D30. Assistant.** None needed. No tool sets a parameter's value for a view, and the
  page context lists the values the read used (`summarizeParams`), which are the values in
  force; a refused value never was.

### BI-54: the builder says its limits and hides what a person may not use

- **D31. The limit stays on screen.** "Add a level" and "Join a table" are always drawn; at
  the limit they are disabled and the sentence is under them, exactly as PR #453 did for
  "Add a question" (`layers-section.tsx:204-211`, test `layers-section.test.tsx:31-48`).
- **D32. Without `metric:write` the page is a list to read.** The console's permission
  check is `useAuth().hasPermission(...)` (`src/features/auth/auth-provider.tsx:216`,
  `:234`; used the same way at `src/features/admin/users-page.tsx:51-52`). Without
  `metric:write`: no "New metric"; a metric's row opens no editor; the Segments dialog
  lists the segments with no "New segment" and no "Edit"; one muted sentence says who can
  change them. The route to the page keeps `dashboard:read`
  (`src/components/app-shell/nav-config.ts`: find `pattern: "/dashboards/metrics"`
  [`:287`]). The server keeps refusing whatever the console shows (`policy.rs:712-720`);
  nothing there changes.
- **D33. A segment's refusal does not say "metrics".** Today the client writes one 403
  sentence for every call (`src/services/clients/metrics.ts:24`). This is the one place
  where the slice changes how a refused request reads, so it uses #497's shared mapping
  when that is in the tree: the client then writes no sentence of its own, and the refusal
  names the permission the server named, the same for a metric and a segment. When #497 is
  not merged yet, the sentence is chosen per call. T13 step 3 has both.
- **D34. Assistant.** None needed. The assistant's `create_hierarchy`, its joins and its
  `create_metric` are refused by the server with the server's own sentences, as before; no
  assistant tool creates a segment.

### Words on screen (English, as the console). None of these has been seen by the product owner.

- Radar: no new text.
- The date grouping mark (tooltip title "Group dates by does not apply", then one of):
  - `kind`: **"Group dates by does not apply to this type of chart."**
  - `part`: **"Group dates by does not apply: this chart groups by a part of the date, such
    as the day of the week."**
  - `not_a_date`: **"Group dates by does not apply: this chart's dimension is not a date."**
  - `no_grain`: **"Group dates by does not apply: this chart has no date grouping of its
    own to change."**
  - a reason the console does not know: **"Group dates by does not apply to this chart."**
- A question outside the viewer's access: the server's sentence **"Not available with your
  access."**; the legend entry **"<label> (not available with your access)"**; the line
  under the chart **"Not available with your access: <label>, <label>"**.
- A refused parameter value: the server's sentence as it is, e.g. "parameter 'target' must
  be at most 50."
- **"A hierarchy takes at most six levels."** and **"A chart joins at most four tables."**
- **"You can read metrics and segments. Defining or changing one needs the metric:write
  permission."**
- A refused change of a segment. With #497 merged: the shared sentence "You don't have
  access. This needs the metric:write permission." (the wording is `WEB-11`'s, not this
  slice's). Without it: **"You do not have permission to define or change segments
  (metric:write)."**

## 2. Tasks

Rust first (T1 to T7), then the console (T8 to T13), the standard request (T14), the
documents (T15). T1 to T3 are `lakehouse-bi`; T4 to T7 are `lakehouse-api`.

### T1. Pin that the server saves a radar with several measures and refuses it a breakdown (LH-BI-6-25; BI-49, D7, D8)

- Anchors: `rust/crates/lakehouse-bi/src/store.rs:7936-7947` (`bi6_save`), `:7949-7954`
  (`bi6_refusal`), `:7957-7984` (the test
  `a_measure_with_its_own_aggregate_is_saved_named_by_aggregate_and_column`, the model);
  `store.rs:3792-3795` (the breakdown sentence).
- Change: tests only, added after the model test. No product code.
- Check (`store.rs` tests). Both pass on today's code: they pin the server behaviour the
  console starts to rely on in T8, so a later change cannot take it away unseen. Say so in
  the commit body.
  - **`a_radar_with_two_measures_is_saved_with_both_columns`**:
    `bi6_save(&json!({ "kind": "radar", "measures": ["visitors", {"column": "stay", "aggregate": "avg"}] }))`
    is `Ok`; `spec.spec.y` is `ChartY::Multi(["visitors", "avg_stay"])`; the statement
    contains `round(sum(visitors)) AS visitors` and `round(avg(stay)) AS avg_stay`.
  - **`a_radar_is_refused_a_breakdown`**:
    `bi6_refusal(&json!({ "kind": "radar", "breakdown": "cat" }))` contains
    `breakdown is only for`.

### T2. `switch_skip`: why the dashboard's grain left a chart as it was (LH-BI-9-05; BI-50, D10, D11)

- Anchors: `rust/crates/lakehouse-bi/src/grain.rs:427-446` (`kind_takes`), `:595-641`
  (`Resolution`, `resolve`), `:982-1008` (the test of `resolve`, the model);
  `rust/crates/lakehouse-bi/src/specs.rs:111-116` (`is_static`).
- Change, in `grain.rs`, after `resolve`:
  1. `pub enum SwitchSkip { Kind, Part, NotADate, NoGrain }` with
     `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]` and
     `#[serde(rename_all = "snake_case")]`, each variant documented with the meaning in D10.
  2. `#[must_use] pub fn switch_skip(kind: ChartKind, built: Option<Grain>, column: Option<ColumnKind>, requested: Option<Grain>) -> Option<SwitchSkip>`.
     `built` is the grain the chart's statement was built with; `column` the kind of its
     dimension's column when the caller knows it. In this order:
     - `requested` is `None` → `None`;
     - `kind.is_static()` → `None`;
     - `!Grain::Day.kind_takes(kind)` → `Some(SwitchSkip::Kind)` (`Day` is taken by every
       kind that takes any grain; say so in a comment);
     - `built` is a truncation (`is_truncation()`) → `None` (the switch replaced it, it was
       already that grain, or `Resolution::skipped` reports it);
     - `built` is any other grain → `Some(SwitchSkip::Part)`;
     - `built` is `None`: `column` is `Some(Date)`, `Some(DateTime)` or `None` →
       `Some(SwitchSkip::NoGrain)`; any other column kind → `Some(SwitchSkip::NotADate)`.
  3. Add one line about `switch_skip` to the module doc's "What lives here" list.
- Check (`grain.rs` tests), **`the_switch_says_why_it_left_a_chart_as_it_was`**, one assert
  per line, with `month = Some(Grain::Month)`:
  - `switch_skip(Line, Some(Grain::Month), Some(Date), month)` is `None`;
  - `switch_skip(Line, Some(Grain::DayOfWeek), Some(Date), month)` is `Some(Part)`;
  - `switch_skip(Bar, None, Some(Text), month)` is `Some(NotADate)`;
  - `switch_skip(Bar, None, Some(Number), month)` is `Some(NotADate)`;
  - `switch_skip(Bar, None, Some(Date), month)` is `Some(NoGrain)`;
  - `switch_skip(Bar, None, None, month)` is `Some(NoGrain)`;
  - `switch_skip(Kpi, None, None, month)` and `switch_skip(Table, None, Some(Date), month)`
    and `switch_skip(Kpi, Some(Grain::Month), None, month)` are `Some(Kind)` (the last is a
    KPI that compares periods: its period is not the control's);
  - `switch_skip(Text, None, None, month)` is `None`;
  - `switch_skip(Bar, None, Some(Text), None)` is `None`;
  - `switch_skip(Calendar, Some(Grain::Day), Some(Date), month)` is `None`;
  - `serde_json::to_value(SwitchSkip::NotADate)` is `"not_a_date"` and of `NoGrain`
    `"no_grain"`.

### T3. `resolve_each`: one refused value does not fail the others (LH-BI-11-05; BI-52, D24–D27)

- Anchors: `rust/crates/lakehouse-bi/src/params.rs:593-601` (`Resolved`), `:615-692`
  (`ParamValues`, `resolve`), `:1713-1758` (the test of `resolve`, with the helpers `base`
  `:1515` and `list` `:1531`).
- Change, in `params.rs`:
  1. `pub struct RefusedParam { pub name: String, pub reason: String }` with
     `#[derive(Debug, Clone, PartialEq, Eq, Serialize)]`. Doc: `name` is the definition's
     name, never the request's text; `reason` is `check_value`'s sentence, which never
     repeats the value.
  2. `pub struct RequestOutcome { pub values: ParamValues, pub refused: Vec<RefusedParam>, pub unknown_name: bool }`
     (`#[derive(Debug, Clone, PartialEq, Eq)]`).
  3. `#[must_use] pub fn resolve_each(defs: &[ParamDef], saved: &[BoardParam], request: Option<&Map<String, Value>>) -> RequestOutcome`
     in `impl ParamValues`. It is the loop `resolve` has today with two differences: a name
     of the request that no definition has sets `unknown_name` and is otherwise ignored;
     a value `check_value` refuses is pushed to `refused` (the definition's name, the
     sentence) and the parameter then takes the saved value (checked, as today) or its
     default, as if the request had not named it. `refused` is in the order of `defs`.
  4. `resolve` becomes a caller of `resolve_each`, so the loop exists once (rule 4):
     `unknown_name` → `Err("the request names a parameter that does not exist.")`; else a
     first entry of `refused` → `Err(its reason)`; else `Ok(values)`. Its doc comment and
     `# Errors` stay true as written; every existing test of `resolve` must pass unchanged.
- Check (`params.rs` tests, using `base` and `list`):
  - **`a_refused_value_keeps_the_saved_value_and_is_listed_with_its_reason`**: `target`
    (number, default 10, max 50) and `pick` (list `a`, `b`, default `a`); saved
    `target = 20`; request `{ "target": 51, "pick": "b" }`. `values.get("target")` is
    `Number(20.0)`, `values.get("pick")` is `Text("b")`, `refused` has one entry whose
    `name` is `"target"` and whose `reason` is the `Err` text of
    `check_value(&defs[0], &json!(51))` (it contains `at most 50`), `unknown_name` is
    false. With no saved value the same request gives `Number(10.0)`.
  - **`an_unknown_name_is_ignored_and_never_repeated`**: request
    `{ "evil'; --": 1, "target": 30 }`: `unknown_name` is true, `refused` is empty,
    `values.get("target")` is `Number(30.0)`, and the `Debug` text of the outcome does not
    contain `evil`.
  - **`a_refused_value_is_not_in_its_reason`**: request `{ "target": "1; DROP TABLE t" }`:
    one refusal, whose reason does not contain `DROP`.

### T4. Each tile the dashboard's grain did not change says why (LH-BI-9-05; BI-50, D10–D12, D14)

- Anchors: `rust/crates/lakehouse-api/src/routes/support.rs:729-741` (`own_columns`),
  `:743-746` (`SourceColumns`), `:960-995` (`annotate_grain`);
  `rust/crates/lakehouse-api/src/routes/dashboard.rs:446-450` (`applied_grain`), `:477-481`
  (`read`), `:552-554` (the `*_ref` names), `:629-630` (the two `annotate_*` calls in the
  tile's future). In the same file, by text: `mod time_grain {` (the tests) [`:8161`], and
  inside it `fn chart_row(` [`:8203`], `async fn mount_board(` [`:8221`],
  `async fn dashboard(` [`:8298`], and the test
  `a_chart_that_cannot_take_the_switch_keeps_its_own_and_says_so` [`:8485`]. The file has
  two more sets of `chart_row`, `mount_board` and `dashboard` helpers in other test
  modules (from `:6885` and from `:8703`): use only the ones inside `mod time_grain`.
- Change:
  1. `support.rs`, after `annotate_grain`:
     `pub(crate) fn annotate_grain_switch(tile: &mut Value, chart: &store::StoredChartSpec, built: Option<lakehouse_bi::grain::Grain>, requested: Option<lakehouse_bi::grain::Grain>, columns: SourceColumns<'_>)`.
     A tile that is not an object or has no `rows` key is left alone. The dimension's kind is
     `own_columns(chart, columns.0, columns.1).and_then(|c| c.get(&chart.def.dimension).copied())`.
     When `lakehouse_bi::grain::switch_skip(chart.spec.kind, built, kind, requested)` is
     `Some(reason)`, insert `"grainNotApplied": json!(reason)`. The doc comment says what
     D10 and D12 say and names the QA case.
  2. `dashboard.rs`, directly after
     `crate::routes::support::annotate_grain(&mut val, &filtered, c);` in the stored
     chart's future:
     `crate::routes::support::annotate_grain_switch(&mut val, c, filtered.grain, read_ref.grain, (cols_ref, sources_ref));`.
     `c` there is the chart with its parameters resolved, which is the one to use. Nothing
     else calls the new function (not `embed.rs`, not `layers.rs`, not `metrics.rs`).
- Check, in `mod time_grain` of `dashboard.rs`:
  - Turn `mount_board(server, board_grain)` into a thin caller of a new
    `mount_board_with(server, board_grain, charts: Vec<Value>, columns: Vec<Value>)` that
    holds its body; `mount_board` passes today's three charts and three columns, so every
    existing test is untouched. Add a helper for a chart row without a `grain` key in its
    definition (`chart_row` always writes one).
  - **`while_the_switch_is_on_every_chart_it_does_not_change_says_why`**: a board with
    `c_line` (line, dimension `day`, grain `day`), `c_dow` (bar, `day`, `day_of_week`),
    `c_city` (bar, dimension `city`, no grain), `c_plain` (bar, dimension `day`, no grain)
    and `c_kpi` (kind `kpi`, `"dimension": ""`, measures `["visitors"]`); `mart_a` has
    `day` `Date`, `city` `String`, `visitors` `UInt32`. With `Some(Grain::Month)`:
    `c_line` has no `grainNotApplied`; `c_dow` is `"part"`; `c_city` is `"not_a_date"`;
    `c_plain` is `"no_grain"`; `c_kpi` is `"kind"`.
  - **`without_a_dashboard_grain_no_tile_carries_the_mark`**: the same board, requested
    `None`, board grain `""`: no tile has the key.
  - **`a_chart_that_kept_its_grain_because_the_new_one_does_not_fit_carries_only_grain_skipped`**:
    `mount_board`, `Some(Grain::Hour)`: `c_line` has `grainSkipped: "hour"` and no
    `grainNotApplied`.
  The first test fails on today's code (the key does not exist).

### T5. The dashboard's read keeps going when a parameter value does not fit (LH-BI-11-05, LH-BI-11-06; BI-52, D23–D27)

- Anchors: `rust/crates/lakehouse-api/src/routes/params.rs:43-87` (`Loaded`, `load`);
  `rust/crates/lakehouse-api/src/routes/dashboard.rs:49-80` (`DashboardQuery`, with the
  `#[serde(skip)] only` field as the model), `:85-100` (`for_render`), `:138-148`
  (`BodyError`), `:244-247` (the `# Errors` of `payload_for`), `:464-476` (the comment and
  the call of `load`), `:787-789` (`params`, `defaultParams`, `appliedParams` in the body).
  The other three callers of `load`, by text: `crate::routes::params::load(` in
  `dashboard.rs` (the builder's preview `specs_preview` [`:1109`], the record list
  `records` [`:2936`], the export `export_tile_statement` [`:4144`]). The seven struct
  literals of `DashboardQuery` in tests: grep `only: None` in
  `rust/crates/lakehouse-api/src/routes/` (`dashboard.rs` [`:6985`, `:8308`, `:8432`,
  `:8829`], `layout_tabs.rs` [`:118`, `:655`], `organise_paths.rs` [`:199`]; the eighth
  hit, `dashboard.rs:98`, is `for_render` itself).
  `rust/crates/lakehouse-api/src/routes/param_rewrite.rs:173-179`
  (`def`), `:195-197` (`values`), `:199` (`chart`), `:241-259` (`tile_sql`), `:264-294`
  (the model test).
- Change:
  1. `params.rs`: `pub(crate) enum OnRefusal { Fail, Report }`
     (`#[derive(Debug, Clone, Copy, PartialEq, Eq)]`, each variant documented: `Fail` is a
     400 for the whole read; `Report` keeps the read and lists the refusal). `Loaded` gains
     `pub refused: Vec<params::RefusedParam>`. `load` takes a fourth argument
     `on_refusal: OnRefusal`: `Fail` is today's body (`ParamValues::resolve`, `refused`
     empty); `Report` calls `ParamValues::resolve_each` and keeps its `values` and
     `refused`. `parse_request`'s 400 stays in both. The store-failure branch returns an
     empty `refused`. Update the doc comment and `# Errors`, and the module doc's line
     about `load`.
  2. `dashboard.rs`: `DashboardQuery` gains
     `#[serde(skip)] strict_params: bool`, documented like `only`: no request can set it;
     a read that produces a document or decides something from the numbers sets it, and
     then a value that does not fit refuses the whole read. `for_render` sets it to `true`.
     Add `strict_params: false` to the seven test literals.
  3. `get_body`: pass `if q.strict_params { OnRefusal::Fail } else { OnRefusal::Report }`
     to `load`; rewrite the comment above the call so it says what is true now. Add
     `"refusedParams": loaded.refused,` to the body after `"appliedParams"`. Correct the
     `# Errors` of `payload_for`.
  4. The other three callers of `load` pass `OnRefusal::Fail`.
- Check:
  - `mod time_grain` of `dashboard.rs` (it has the wiremock board; mount the answer for
    `FROM console.bi_param FINAL` **before** `mount_board`, because `mount_board` ends with
    a catch-all): one parameter row with the columns `id`, `name`, `def_json`,
    `created_by`, `updated_at`, whose `def_json` is
    `{"name":"target","label":"Target","type":"number","default":30,"min":0,"max":50}`.
    - **`a_parameter_value_that_does_not_fit_is_listed_and_the_dashboard_is_still_read`**:
      `params: Some(r#"{"target":51}"#)`, `strict_params: false`: the call is `Ok`;
      `body["refusedParams"]` has one entry, its `name` is `"target"` and its `reason`
      contains `at most 50` and does not contain `51`; `body["appliedParams"]["target"]`
      is `30`; `body["results"]["c_line"]` has `rows`.
    - **`a_name_that_is_not_a_parameter_is_ignored_by_the_dashboards_read`**:
      `params: Some(r#"{"gone":1}"#)`: `Ok`, `refusedParams` is `[]`.
    - **`a_read_that_makes_a_document_still_refuses_the_whole_read`**:
      `DashboardQuery::for_render("b1", None, Some(r#"{"target":51}"#), None)`: the result
      is `Err(BodyError::Params(ApiError::BadRequest(m)))` and `m` contains `at most 50`.
    - **`params_that_are_not_an_object_are_still_a_bad_request`**: `params: Some("[1]")`,
      `strict_params: false`: `Err(BodyError::Params(_))`.
  - `param_rewrite.rs`, **`a_refused_value_reaches_no_statement`**: `target` is
    `def("target", ParamType::Number, json!(1000), &json!({}))`, the field and the chart
    are those of `a_formula_parameter_is_a_literal_in_a_governed_statement`.
    `ParamValues::resolve_each(&defs, &[], json!({ "target": "1; DROP TABLE t" }).as_object())`
    has one refusal; `tile_sql` with its `values` is byte for byte `tile_sql` with
    `values(&defs, &json!({}))`, and does not contain `DROP`.
  The first two fail on today's code. The third and fourth pass on it: they pin what must
  not change. The fifth needs only T3.

### T6. `access_cause`: whether a principal's access explains a statement that gave no rows (LH-BI-13-03; BI-51, D17, D19)

- Anchors: `rust/crates/lakehouse-api/src/policy_engine.rs:299-347`
  (`rewrite_sql_for_roles`; the loop to move is `:313-334`, from
  `if !access.is_all() {` to its closing brace), `:566-571` (`table_access`), `:761-782`
  (`prefetch_for_tables`), `:789-806` (`PrefetchedObligations`), `:957-1085` (the test
  module `iceberg_columns`: `seed`, `clickhouse`, and
  `masks_a_governed_iceberg_read_through_clickhouse`, the model);
  `rust/crates/lakehouse-api/src/sql_rewrite.rs:118-121` (`referenced_tables`), `:205`
  (`gate_targets`), `:1331-1342` (`TableObligations`);
  `rust/crates/lakehouse-api/src/table_access.rs:64-66`, `:96-101` (`is_all`,
  `allows_key`).
- Change, all in `policy_engine.rs`:
  1. A private `enum HiddenRead { Table(String), Listing }` and a private
     `fn hidden_read<D: Dialect>(sql: &str, dialect: &D, access: &crate::table_access::TableAccess) -> Option<HiddenRead>`:
     `None` when `access.is_all()`; otherwise, from `sql_rewrite::gate_targets(sql, dialect)`:
     the first named table for which `!access.allows_key(..)` → `Table(that table)`;
     `ListsObjects` → `Listing`; a statement that does not parse → `None`. Move the comment
     of the loop onto it.
  2. `rewrite_sql_for_roles` calls it in place of the loop:
     `Some(HiddenRead::Table(t))` → `return Err(EnforcementError::TableNotReadable(t))`;
     `Some(HiddenRead::Listing)` → `return Err(EnforcementError::Unresolvable)`. Nothing
     else in that function changes.
  3. `pub enum AccessCause { HiddenTable, MaskedColumn }`
     (`#[derive(Debug, Clone, Copy, PartialEq, Eq)]`), documented with D16.
  4. In `impl PolicyEngineObligations<'_>`, after `prefetch_for_tables`:
     `pub async fn access_cause<D: Dialect + Sync>(&self, sql: &str, dialect: &D, columns: &[String], roles: &[String]) -> Option<AccessCause>`.
     Resolve `sql_rewrite::referenced_tables(sql, dialect)` first (`None` → `None`). Read
     `self.table_access(roles)`; an error → `None`. If `hidden_read` gives `Table(_)` →
     `Some(HiddenTable)`. Then `self.prefetch_for_tables(&tables, roles)`; an error →
     `None`. If, for any of those tables, `obligations_for(table, roles)` has a `mask`
     entry equal (`eq_ignore_ascii_case`) to one of `columns` → `Some(MaskedColumn)`. Else
     `None`. The doc comment says: it never decides whether data may be read (the rewrite
     did, before); it only says why a statement that already failed gave no rows, and
     `None` means "cannot tell", so the caller keeps the sentence it had. It has no
     `# Errors` section because it returns no `Result`.
- Check:
  - `policy_engine.rs`, in `mod iceberg_columns`,
    **`a_mask_on_a_column_the_statement_names_is_the_cause_and_another_column_is_not`**
    (`#[sqlx::test(migrations = "../../migrations")]`, `seed`, `clickhouse()`,
    `.with_iceberg_db(Some(&db))`, as the model): for
    ``SELECT sum(email) FROM icecat_api.`bronze.orders` `` and roles `["Analyst"]`,
    `access_cause` with columns `["email"]` is `Some(AccessCause::MaskedColumn)` and with
    `["id"]` is `None`; with columns `["email"]` and roles `["Data Scientist"]` (a seeded
    role the policy does not name) it is `None`. Do not use a role name that does not
    exist: `table_access::for_roles` gives such a name no table at all
    (`table_access.rs:148-169`), which is the hidden-table answer, tested in T7.
  - `cargo test -p lakehouse-api --test table_access` passes unchanged: it holds the tests
    of the hidden table (404), the listing (422) and the mask on an allowed table, which is
    what step 2 must not move.

### T7. A question the viewer's access does not cover is marked, without a reference (LH-BI-13-03; BI-51, D16–D18, D21)

- Anchors: `rust/crates/lakehouse-api/src/routes/layers.rs:13-22` (the module doc's "A
  layer never fails the tile"), `:85-88` (`LayerRunner`), `:92-114` (`RoleRunner`),
  `:241-267` (`Failure`, `plain`, `failure_of`), `:315-368` (`ask`; the run is `:344`);
  `rust/crates/lakehouse-api/src/routes/layers_rewrite.rs:105-131` (the engine runner's
  `LayerRunner` impl), `:612-665` (`Stub`, with `run_now` and the `impl LayerRunner`),
  `:691-711` (`capacity_layer`, `by_province`, `tile_of`), `:737-770` (the test
  `a_failing_or_refused_statement_is_that_layers_sentence_and_its_neighbours_answer`);
  `rust/crates/lakehouse-bi/src/layers.rs:74-114` (`LayerDef`);
  `rust/crates/lakehouse-api/tests/table_access.rs:138-149` (`narrowed`), `:402-429` (how a
  mask policy is seeded for the user's role), `:464-476` (`board_chart`), `:480-545`
  (`BoardFake`), `:547-590` (the tile test, the model).
- Change, in `layers.rs`:
  1. `pub(crate) const NOT_AVAILABLE_WITH_YOUR_ACCESS: &str = "Not available with your access.";`
     and `enum LayerCause { Access }` (`#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]`,
     `#[serde(rename_all = "snake_case")]`).
  2. `LayerRunner` gains
     `async fn access_is_the_cause(&self, sql: &str, columns: &[String], failure: &SpecRunFailure) -> bool;`
     with no default body.
  3. A private `fn explains(failure: &SpecRunFailure, cause: Option<crate::policy_engine::AccessCause>) -> bool`:
     `Tile(_)` → false; `Gated(err)` → true only for `lakehouse_core::ApiError::NotFound(_)`;
     `Refused(_)` → `cause == Some(HiddenTable)`; `Clickhouse(err)` →
     `err.server_code().is_some() && cause == Some(MaskedColumn)`.
  4. `impl LayerRunner for RoleRunner`: for `Refused(_)` and `Clickhouse(_)` it awaits
     `self.obligations.access_cause(sql, &sqlparser::dialect::ClickHouseDialect {}, columns, self.roles)`;
     for `Tile(_)` and `Gated(_)` the cause is `None` (no second read of the policies);
     it returns `explains(failure, cause)`.
  5. A private `fn layer_columns(layer: &LayerDef) -> Vec<String>`: the dimension, the
     column of every measure (`layer.measures.entries()`), the column of every own filter;
     empty names left out.
  6. In `ask`, replace `ctx.runner.run(&filtered.sql).await.map_err(failure_of)?` by a
     `match`: on `Err(failure)`, first
     `ctx.runner.access_is_the_cause(&filtered.sql, &layer_columns(layer), &failure).await`,
     then `failure_of(failure)` (it still logs an engine failure under its reference);
     when access is the cause, in that map set `error` to the constant, remove `errorId`,
     insert `"cause": json!(LayerCause::Access)`; return it as the `Err`.
  7. Add to the module doc, under "A layer never fails the tile", what D16 and D18 say.
     Leave `failure_of` as it is, to the letter: `tests/sec11_guard.rs` allows its line
     `SpecRunFailure::Gated(err) => json!({ "error": err.to_string() }),` by its text
     (`sec11_guard.rs:172-176`), and #491 edits that test file and `upstream_error.rs`,
     which this task must not touch. No new line in `src/routes/` may put an error value's
     own text into a response (`explains` only matches on the variant).
  8. `layers_rewrite.rs`: the two other runners implement the new method without an
     `await` in the way `Stub::run` does (`std::future::ready`, see the comment at `:660`):
     the engine runner answers `false`; `Stub` answers
     `sql.contains("MASKEDCOLUMN") || sql.contains("HIDDENTABLE")`. `Stub::run_now` gains:
     a statement containing `MASKEDCOLUMN` → `Err(SpecRunFailure::Clickhouse(ChError::Server("Code: 43. Illegal type String of argument for aggregate function sum on db-internal.example.net".to_owned())))`;
     one containing `HIDDENTABLE` →
     `Err(SpecRunFailure::Refused("Table not found or not available to your organisation: serving.x".to_owned()))`.
- Check:
  - `layers_rewrite.rs`,
    **`a_question_the_viewers_access_does_not_cover_says_so_without_a_reference_or_the_engines_text`**:
    `by_province(5, &[capacity_layer("hotels"), capacity_layer("MASKEDCOLUMN"), capacity_layer("HIDDENTABLE"), capacity_layer("SECRETCOLUMN")])`.
    Layer 0 answers. Layers 1 and 2: `error` is `"Not available with your access."`,
    `cause` is `"access"`, no `errorId`, `rows` is `[]`. Layer 3 (an engine failure the
    runner does not put down to access): `error` is `"This chart could not be loaded."`,
    `errorId` is a string, no `cause`. The whole tile's text contains none of
    `MASKEDCOLUMN`, `Code: 43`, `db-internal`, `serving.x`. The base rows are untouched.
  - `layers_rewrite.rs`,
    **`only_the_gates_not_found_a_hidden_table_and_a_masked_column_the_engine_refused_count_as_access`**
    (a table over `super::explains`): `Gated(NotFound)` → true with any cause;
    `Gated(Unprocessable)` → false; `Refused` with `Some(HiddenTable)` → true, with
    `Some(MaskedColumn)` or `None` → false; `Clickhouse(Server("Code: 43. x"))` with
    `Some(MaskedColumn)` → true, with `None` or `Some(HiddenTable)` → false;
    `Clickhouse(Server("no code"))` with `Some(MaskedColumn)` → false; `Tile(..)` → false.
  - `tests/table_access.rs`, three tests over the real router and a real Postgres, with a
    new fake modelled on `BoardFake` that serves **one** stored chart: a `bar` on a base
    mart (`dimension: "kota"`, `measures: ["n"]`, `aggregate: "sum"`, `limit: 20`) with one
    entry in `layers` on a second mart (`dimension: "kota"`, `measures: ["omzet"]`,
    `aggregate: "sum"`, `draw: "line"`, `axis: "right"`). The fake answers
    `FROM system.columns` with the columns of **both** marts (`kota` `String`, `n` or
    `omzet` `UInt64`, each row also with `table`, `default_kind`, `default_expression`),
    and with one mart's columns only when the statement contains `AND table=` and that
    mart's name; `FROM system.tables` with no rows, as `FakeCh` does at `:113-116` (no
    table is a view); a read of the base mart with one row; every other statement with no
    rows.
    - **`a_question_on_a_table_the_viewers_role_does_not_read_is_marked_as_access`**: the
      viewer is `narrowed(&pool, "dashboard:read", &[("serving", <base mart>)])`.
      `results[<chart>]` has `rows`; `layers[0]["cause"]` is `"access"`, `error` is the
      sentence, no `errorId`; the text of `layers[0]` does not contain
      `Table not found`; no statement the fake received reads the second mart with a
      `FROM serving.<second mart>`.
    - **`a_question_that_sums_a_column_masked_for_the_viewer_is_marked_as_access`**: the
      viewer is narrowed to both marts and a policy masks `omzet` on
      `serving.<second mart>` for the viewer's role (seeded as at `:402-429`). The fake
      answers a statement that contains both `replaceRegexpOne` and the second mart's name
      with status 500 and the body
      `Code: 43. DB::Exception: Illegal type String of argument for aggregate function sum. (ILLEGAL_TYPE_OF_ARGUMENT)`.
      `layers[0]["cause"]` is `"access"`, `error` is the sentence, no `errorId`; the text
      of `layers[0]` contains neither `omzet` nor `Code: 43` nor `Illegal`.
    - **`a_question_the_engine_refuses_for_a_viewer_without_a_mask_keeps_its_reference`**:
      the viewer is narrowed to both marts, no policy; the fake answers the second mart's
      read with the same 500. `layers[0]["error"]` is `"This chart could not be loaded."`,
      `errorId` is a string, there is no `cause`.
    The first two fail on today's code. If the harness cannot produce one of these
    statements for a reason this plan did not foresee, stop and report it; do not weaken
    the assertion (rule 2).

### T8. A radar takes up to six measures and draws one shape for each (LH-BI-6-25; BI-49, D4–D6)

- Anchors: `src/lib/chart-shape.ts:180-199` (`measureLimits`; the first `case` line is
  `:183`); `src/lib/chart-shape.test.ts:40-46`;
  `src/features/dashboards/chart-option.ts:17` (`LEGEND_TEXT`), `:111` (`labelOf`), `:123`
  (the pie's legend, the model), `:139-140` (`yArr`, `y0`), `:195-205` (the radar);
  `src/features/dashboards/chart-shape-section.tsx:111-117` ("Add measure", shown when the
  kind takes more than its minimum); `src/features/dashboards/chart-builder.test.tsx:52-56`
  (`barDef`, `renderBuilder`), `:135-152` (the test "offers 'Other', steps and questions to
  a bar, and none of them to a KPI": the model, it gives the builder a chart of a kind
  through `renderBuilder`, it does not click a type), `:19-29` (`bodies`, every request
  body the builder sent), `:689-706` (`choose`, a helper local to that `describe`, and a
  test that picks an option and waits for the preview request: the model for both).
- Change:
  1. `chart-shape.ts`: add `case "radar":` to the first `case` line of `measureLimits`
     (bar, hbar, line, area, table), so a radar returns `{ min: 1, max: 6, own: true }`.
     Nothing else in the builder changes: `MeasureRows` reads these limits.
  2. `chart-option.ts`, the radar branch:
     - the scale: `Math.max(1, ...rows.flatMap((r) => yArr.map((c) => num(r[c]))))`, used
       as today (`max: maxVal * 1.1` on every indicator);
     - the series' `data`: one entry per column of `yArr`,
       `{ value: rows.map((r) => num(r[c])), name: labelOf(c) }` (one measure: one entry,
       now named by its label like every other chart);
     - only when `yArr.length > 1`: a `legend` exactly like the pie's at `:123` plus
       `type: "scroll"` and `data: yArr.map(labelOf)`, and on the `radar` object
       `center: ["50%", "46%"]` and `radius: "62%"` so the legend has room. With one
       measure the option has no `legend` key and no `center` or `radius`, as today.
- Check:
  - `chart-shape.test.ts`, **"a radar takes one to six measures, each with its own
    aggregate"**: `measureLimits("radar")` is `{ min: 1, max: 6, own: true }`.
  - `chart-option.test.ts`:
    - **"a radar draws one shape per measure on one shared scale and names each in the
      legend"**: spec `{ kind: "radar", x: "kota", y: ["a", "b"] }`, rows
      `[{ kota: "X", a: 10, b: 2000 }, { kota: "Y", a: 30, b: 1000 }]`: the series has one
      entry with two `data` items, values `[10, 30]` and `[2000, 1000]`, names `a` and
      `b`; every indicator's `max` is `2200`; `legend.data` is `["a", "b"]`.
    - **"a radar with one measure has no legend and keeps the scale of its own values"**:
      `y: "a"`: one data item, every `max` is `33`, the option has no `legend`.
  - `chart-builder.test.tsx`, **"a radar offers Add measure and sends both measures"**:
    `renderBuilder({ ...barDef, kind: "radar" })`, press "Add measure", pick a column in
    the new row the way `choose` does (copy the helper into the new test's `describe`):
    a body in `bodies` has `measures` with both. If the second column cannot be picked in
    a test, assert only that "Add measure" is on the page for a radar and say so in the
    handoff.
  All fail on today's code.

### T9. The tile says when "Group dates by" does not apply to it (LH-BI-9-05; BI-50, D13, D15)

- Anchors: `src/lib/grain-switch.ts` (whole file, 56 lines);
  `src/features/dashboards/grain-marker.tsx:13-31`;
  `src/features/dashboards/tile-dialogs.tsx:33-34` (`grainSkipped` in `Rows`);
  `src/features/dashboards/dashboard-page.tsx:119` (`appliedGrain` in `Payload`), and by
  text: `const grainedCharts =` [`:453`] and the tile's `hint` block that starts
  `{hasRows(cell) && (cell.grainSkipped ||` [`:1156-1157`];
  `src/lib/page-context-summary.ts:34-38` (`CellLike`), `:92-117` (`summarizeTiles`).
- Change:
  1. `grain-switch.ts`:
     `export type GrainNotApplied = "kind" | "part" | "not_a_date" | "no_grain"`;
     `export function grainNotAppliedSentence(reason: string | undefined): string | null`:
     `null` for an absent or empty reason, the D-section sentence for each of the four, and
     "Group dates by does not apply to this chart." for any other text. Say in the doc
     comment that the type mirrors `lakehouse_bi::grain::SwitchSkip`.
  2. `tile-dialogs.tsx`: `Rows` gains `grainNotApplied?: GrainNotApplied | string` with a
     one-line comment (`BI-50`).
  3. `grain-marker.tsx`: `GrainMarkers` gains `notApplied?: string` (the sentence). When
     set it draws one more mark: the `CalendarOff` icon in the same trigger as the
     `skipped` mark, `aria-label` the sentence, tooltip title "Group dates by does not
     apply" and the sentence under it. Update the component's doc comment.
  4. `dashboard-page.tsx`: where the grain marks are drawn, compute
     `const notApplied = hasRows(cell) && data?.appliedGrain && grainedCharts.length > 0 ? grainNotAppliedSentence(cell.grainNotApplied) : null`
     and render `GrainMarkers` when there is a `grainSkipped`, a cut-off, or `notApplied`,
     passing `notApplied={notApplied ?? undefined}`.
  5. `page-context-summary.ts`: the rows variant of `CellLike` gains
     `grainNotApplied?: string`; `summarizeTiles` adds, after the `binned` part of the
     line and only for a tile that has the field, `; date grouping: <sentence>` where
     `<sentence>` is `grainNotAppliedSentence(cell.grainNotApplied)`, so the words exist
     once. Nothing else is added: the context has a size budget.
- Check:
  - `src/lib/grain-switch.test.ts`: **"each reason the server gives has its sentence, an
    unknown reason the general one, and no reason none"**.
  - New `src/features/dashboards/grain-marker.test.tsx`: **"a chart the control does not
    change carries the mark with the sentence as its label"** (render with
    `notApplied="Group dates by does not apply: this chart's dimension is not a date."`,
    find it by label), **"without a reason no such mark is drawn"**.
  - `src/lib/page-context-summary.test.ts`: **"a tile the date grouping does not apply to
    says so on its line"**.
  - `dashboard-page-role.test.tsx` (the nearest page test): **"the mark is drawn only
    while the dashboard applied a grain"**: a payload with one chart whose definition has
    `grain: "day"`, one chart by `city` whose result has `grainNotApplied: "not_a_date"`,
    and `appliedGrain: "month"` shows the mark's label on the page; the same payload with
    `appliedGrain: null` does not.

### T10. A question outside the viewer's access is in the legend, greyed, and not in the red list (LH-BI-13-03; BI-51, D20)

- Anchors: `src/features/dashboards/combined-option.ts:27-40` (`LayerCell`), `:81-87`
  (`answered`, `failedLayers`), `:108` (`Series`), `:133-144` (the layers loop), `:163`
  (the legend), `:179-192` (the series of the option), `:194` (`owners`);
  `src/features/dashboards/tile-body.tsx:12` (the import), `:408-409`, `:430-441` (the
  lines under the chart); `src/features/dashboards/combined-option.test.ts:75-81`,
  `src/features/dashboards/combined-tile.test.tsx:31-48` (the models).
- Change:
  1. `combined-option.ts`:
     - `LayerCell` gains `cause?: "access" | string` (comment: `BI-51`, mirrors
       `routes::layers::LayerCause`);
     - `export const NOT_AVAILABLE_WITH_YOUR_ACCESS = "Not available with your access"`
       (not `NO_ACCESS`: #497 gives that name to another sentence, "You don't have
       access.", in `src/services/errors.ts`);
     - `failedLayers` leaves out a layer whose `cause` is `"access"`;
     - `export function unavailableLayers(layers: readonly LayerCell[] | undefined): string[]`:
       the labels of the layers whose `cause` is `"access"`;
     - in the layers loop, a layer that did not answer and whose `cause` is `"access"`
       pushes one series: name `` `${l.label} (not available with your access)` ``, type
       `l.draw`, axis the axis of `series[0]` (the chart's own first series, so no axis is
       added), values `keys.map(() => null)`, owner its index, `area: false`, and a new
       `off: true` on the `Series` type. Every other layer that did not answer is skipped
       as today;
     - in the option, an `off` series also gets `silent: true` and a muted colour
       (`dark ? "#52525b" : "#a1a1aa"`) as `itemStyle.color` and `lineStyle.color`;
     - only when at least one series is `off`, the legend gains
       `data: series.map((s) => (s.off ? { name: s.name, textStyle: { color: <the muted colour> }, itemStyle: { color: <the muted colour> } } : { name: s.name }))`.
       With no such series the option is what it is today, key for key.
  2. `tile-body.tsx`: after the "cut off" line and before the red list, when
     `unavailableLayers(layers)` is not empty:
     `<p className="truncate px-1 pt-1 text-[11px] text-muted-foreground" aria-label="Questions not available with your access" title={<the same text>}>{NOT_AVAILABLE_WITH_YOUR_ACCESS}: {labels.join(", ")}</p>`.
- Check:
  - `combined-option.test.ts`, **"a question outside the viewer's access is a greyed
    legend entry with no points and adds no axis"**: base on the left, a layer on the right
    axis with `error` set and `cause: "access"`: the series names are the base's and
    `"<label> (not available with your access)"`; that series' data is all `null`; the
    option has one `yAxis`; `legend.data` has that name with a `textStyle.color`;
    `failedLayers` is empty and `unavailableLayers` is `["<label>"]`; `owners` has the
    layer's index for it.
  - `combined-option.test.ts`, **"a chart with no such question has the legend it always
    had"**: no `data` key in `legend`.
  - `combined-tile.test.tsx`, **"a question outside the viewer's access is named in one
    muted line and not among the questions that could not be drawn"**: the line
    "Not available with your access: Occupancy" is on the page; there is no element
    labelled "Questions that could not be drawn"; a second layer that failed with an
    `errorId` and no `cause` still shows its sentence and reference.

### T11. A refused parameter value is shown on its control and leaves the address (LH-BI-11-05, LH-BI-11-06; BI-52, D28, D29)

- Anchors: `src/lib/params.ts:92-119` (`readParamsParam`, `paramsQuery`,
  `withParamsParam`); `src/features/dashboards/param-controls.tsx:9-70` (whole file);
  `src/features/dashboards/param-controls.test.tsx`;
  `src/features/dashboards/dashboard-page.tsx:105-135` (`Payload`), `:222-223` (`pParam`,
  `pValues`), `:305-340` (`load`), `:356` (the effect on `[board]`), `:472-486`
  (`paramControls`, `changeParam`, `resetRow`), and by text
  `<ParamControls controls={paramControls} onChange={changeParam} />` [`:1317`] with the
  condition of the `FilterRow` above it [`:1308-1309`]; `src/features/dashboards/dashboard-comments-param.test.tsx:1-40`
  (the model of a page test that reads the address and records `router.replace`).
- Change:
  1. `params.ts`: `export type RefusedParam = { name: string; reason: string }` (mirrors
     `lakehouse_bi::params::RefusedParam`) and
     `export function acceptedParams(sent: Record<string, ParamValue> | null, refused: readonly RefusedParam[] | undefined, applied: Record<string, ParamValue> | undefined): Record<string, ParamValue> | null`:
     `sent` without the refused names and, when `applied` is given, without the names
     `applied` does not have; `null` when nothing is left or `sent` is `null`.
  2. `param-controls.tsx`: `ParamControls` gains
     `refusals?: Readonly<Record<string, string>>`. `ParamField` gets its parameter's
     refusal: it is drawn under the control as
     `<p role="alert" className="text-xs text-destructive">{refusal}</p>` (wrap the
     existing row and this line in one `div` that keeps `data-param`), and the effect that
     copies `value` into `typed` also runs when the refusal changes, so the control returns
     to the value in force. After the controls, each refusal whose name has no control is
     drawn the same way. Correct the doc comment (it says a refusal comes back as the
     page's error).
  3. `dashboard-page.tsx`:
     - `Payload` gains `refusedParams?: RefusedParam[]`;
     - two states: `paramRefusals` (`Record<string, string>`, empty at first) and
       `paramAnswer` (`{ sent: Record<string, ParamValue> | null; refused: RefusedParam[]; applied?: Record<string, ParamValue> } | null`);
     - in `load`, after `setData(...)`: when `json.refusedParams` is not empty, merge its
       names and reasons into `paramRefusals`; set
       `paramAnswer` to `{ sent: pValues, refused: json.refusedParams ?? [], applied: json.appliedParams }`.
       `load`'s dependency list does not change (`pValues` is already in it; do **not** add
       `router` to it: the page tests' router is a new object on every render and `load`
       runs in an effect);
     - one effect on `[paramAnswer, pValues, router, pathname]`: nothing without a
       `paramAnswer`; nothing when `paramsQuery(paramAnswer.sent) !== paramsQuery(pValues)`
       (the address has moved on: the next answer decides); otherwise
       `kept = acceptedParams(paramAnswer.sent, paramAnswer.refused, paramAnswer.applied)`
       and, when `paramsQuery(kept) !== paramsQuery(pValues)`, replace the address with
       `withParamsParam(window.location.search, kept)` exactly as `changeParam` does;
     - `changeParam` removes `name` from `paramRefusals` before it writes the address;
       `resetRow` and the effect on `[board]` empty `paramRefusals`;
     - pass `refusals={paramRefusals}` to `ParamControls`, and show the filter row also
       when `paramRefusals` is not empty.
- Check:
  - `src/lib/params.test.ts`: **"a refused value and a name the answer does not know leave
    what the address carries, and nothing left is no parameter at all"**.
  - `param-controls.test.tsx`: **"a refusal is shown under its control and the control
    goes back to the value in force"** (type 999, blur, rerender with
    `refusals={{ target: "parameter 'target' must be at most 5000." }}`: the alert has the
    sentence and the input reads `1000`), **"a refusal for a parameter the dashboard does
    not show is listed after the controls"**, **"without a refusal no alert is drawn"**.
  - New `src/features/dashboards/dashboard-refused-param.test.tsx`, modelled on
    `dashboard-comments-param.test.tsx`, with the address `p={"target":"abc"}` (given to
    `URLSearchParams` percent-encoded) and a
    dashboard answer (status 200) that has `params: [{ param: <target, number>, value: 30 }]`,
    `appliedParams: { target: 30 }` and
    `refusedParams: [{ name: "target", reason: "parameter 'target' must be a number." }]`:
    **"a value in the address that does not fit draws the dashboard, says why on the
    parameter, and is taken out of the address"**: the sentence is on the page in an
    alert; the page's red error banner is not; a recorded `router.replace` URL has no
    `p=`.
  All fail on today's code.

### T12. "Add a level" and "Join a table" stay at their limit, off, with the sentence (LH-BI-10-08, LH-BI-7-06; BI-54, D31)

- Anchors: `src/features/dashboards/hierarchy-dialog.tsx:106-112`;
  `src/features/dashboards/joins-section.tsx:104-110`;
  `src/features/dashboards/layers-section.tsx:204-211` (the pattern);
  `src/features/dashboards/hierarchy-dialog.test.tsx:38-44`,
  `src/features/dashboards/joins-section.test.tsx:18-28` (each asserts today that the
  button is gone at the limit); `src/services/contracts/hierarchies.ts:28` (`MAX_LEVELS`),
  `src/lib/chart-joins.ts:23` (`MAX_JOINS`).
- Change, the pattern of `layers-section.tsx:205-211` in both files: a
  `<div className="space-y-1">` that always holds the button, disabled at the limit
  (`levels.length >= MAX_LEVELS`; for joins `disabled || !baseMart || joins.length >= MAX_JOINS`),
  and under it, only at the limit,
  `<p className="px-2 text-xs text-muted-foreground">` with "A hierarchy takes at most six
  levels." or "A chart joins at most four tables.". "Add a pair" is not changed
  (section 4).
- Check. The two existing assertions change because the rule changed (rule 2: say so in
  the commit body): the button used to vanish, it now stays.
  - `hierarchy-dialog.test.tsx`, in "adds levels up to six and never fewer than two":
    after four clicks the button is still on the page and disabled, and "A hierarchy takes
    at most six levels." is shown. New test **"says nothing about a limit below six
    levels"**: at five levels the button is enabled and the sentence is absent.
  - `joins-section.test.tsx`, in "adds a blank join, up to four, and removes one": at four
    joins the button is on the page and disabled, and "A chart joins at most four tables."
    is shown. New test **"keeps the button on and says nothing about a limit below four
    joins"**.

### T13. Without `metric:write` the metrics page is a list to read, and a segment's refusal does not say "metrics" (LH-BI-2-05; BI-54, D32, D33)

- Anchors: `src/features/dashboards/metrics-page.tsx:16-39` (the doc comment and the
  header's buttons), `:57-66` (a row), `:72-78` (the two dialogs);
  `src/features/dashboards/segments-dialog.tsx:25` (the props), `:89-101` (the list, "Edit"
  and "New segment"); `src/services/clients/metrics.ts:5-28` (`call`), `:63-77` (the four
  segment calls); `src/features/auth/auth-provider.tsx:216`, `:234`, and `:151-173` (the
  session check, which reads `me.tenants` since `ADM-15`);
  `src/features/dashboards/dashboard-page-role.test.tsx:1-80` (how a page test gives the
  signed-in person their permissions through `/api/auth/me`; the answer is at `:34-45`);
  `src/services/clients/dashboards.test.ts:1-30` (the model of a client test). Only when
  #497 is merged: `src/services/errors.ts` (`deniedFor`) and `call` of
  `src/services/clients/calc-fields.ts` (the model of a client that uses it).
- Change:
  1. `metrics-page.tsx`: `const canWrite = useAuth().hasPermission("metric:write")`
     (`import { useAuth } from "@/features/auth/auth-provider"`). "New metric" is rendered
     only with `canWrite`. Without it a row has no `onClick` and no `cursor-pointer`, and
     the name is a plain `span` with the same classes minus the hover underline, so no
     editor opens. Under the header, without `canWrite`:
     `<p className="text-xs text-muted-foreground">You can read metrics and segments. Defining or changing one needs the metric:write permission.</p>`.
     Pass `canWrite` to `SegmentsDialog`. Rewrite the doc comment to say this.
  2. `segments-dialog.tsx`: a new prop `canWrite: boolean`; "New segment" and each row's
     "Edit" are rendered only with it.
  3. `metrics.ts` (D33). First run
     `grep -n "export function deniedFor" src/services/errors.ts` and quote its output in
     the handoff. Build exactly one of the two:
     - **It prints a line (#497 is merged).** In `call`, as the first lines inside
       `if (!res.ok) {`: `const denied = deniedFor(res.status, json)` and
       `if (denied) throw denied` (import `deniedFor` from `"../errors"`), as `call` of
       `calc-fields.ts` does. Remove the `401 || 403` arm of `kind` and the hand-written
       403 sentence, so `message` is `json?.error ?? fallback`. The client then has no
       sentence of its own: the server names `metric:write` for a metric and for a segment
       alike (`policy.rs:712-720`), and the mapping words both as "You don't have access.
       This needs the metric:write permission." Correct the file's doc comment. This takes
       `metrics.ts` off the list of `WEB-11` slice S2; say so in the handoff.
     - **It prints nothing (#497 is not merged).** Two constants, the metrics sentence as
       it is today and
       `"You do not have permission to define or change segments (metric:write)."`; `call`
       takes the 403 sentence as a fourth argument that defaults to the metrics one;
       `createSegment`, `updateSegment` and `removeSegment` pass the segments one. Correct
       the file's doc comment. `WEB-11` slice S2 replaces both with `deniedFor` later.
- Check:
  - New `src/features/dashboards/metrics-page.test.tsx`, under `AuthProvider`, with
    `fetch` answering `/api/auth/me` (every field of the model's answer, with
    `tenants: []`: an answer without `tenants` fails the session check and the person
    then holds no permission), `/api/dashboard/metrics` (one metric) and
    `/api/dashboard/segments` (one segment); `window.localStorage.clear()` in `afterEach`,
    as the model:
    - **"a person without metric:write reads the list and is offered nothing to define or
      change"**: permissions `["dashboard:read"]`: the metric is listed; there is no "New
      metric"; a click on its name opens no dialog; the sentence of D32 is shown; after
      "Segments" the segment is listed with no "New segment" and no "Edit".
    - **"a person with metric:write is offered New metric, New segment and Edit"**:
      permissions `["dashboard:read", "metric:write"]`.
  - New `src/services/clients/metrics.test.ts`, **"a refused segment change does not say
    metrics"**, with `fetch` answering 403 and the body
    `{ "error": "permission_denied: metric:write" }` (what the server sends):
    - built with `deniedFor`: `metricService.createSegment(...)` and
      `metricService.create(...)` both reject with a `ServiceError` whose `code` is
      `permission_denied` and whose message is "You don't have access. This needs the
      metric:write permission."; the segment's message does not contain `metrics`;
    - built without it: `metricService.createSegment(...)` rejects with the segments
      sentence and `metricService.create(...)` with the metrics sentence.
  The first and the third fail on today's code, in either form.

### T14. One standard request: a radar that compares two measures (BI-49, D9; `AI-15`)

- Anchors: `ops/ai_eval/dashboard_cases.json`, the case `chart_top_n_breakdown_over_time`
  (it creates its own SQL source in `setup` and names it in the request: the model);
  `ops/ai_eval/ai_eval.py` (how `expect_tool.args_contains` and `expect_state.where` match).
- Change: append one case, id `chart_radar_two_measures`, `mode: "build"`,
  `confirm: true`. `setup`: a SQL source titled "Eval Radar Two Measures" with
  `SELECT negara, sum(jumlah) AS kunjungan, count() AS baris FROM serving.mart_wisman GROUP BY negara`,
  `id_at: "source.id"`, name `src`. One turn: `Draw a radar chart from the saved source
  "Eval Radar Two Measures" that compares kunjungan and baris per country (negara).`
  `expect_tool`: `create_chart` with `args_contains` `{ "sqlSource": "{src}", "kind": "radar" }`.
  `expect_state`: one chart in `/api/dashboard/specs` where `sqlSource` is `{src}` and
  `kind` is `radar`. If `where` can match a list of plain names (read `ai_eval.py`; the
  only list form used today is `def.measures[].metric`), also require both measures;
  if it cannot, leave that out and say so in the handoff. The case is not run in this slice
  (it needs a model): write *not verified* for its result.
- Check: `python3 -m json.tool ops/ai_eval/dashboard_cases.json > /dev/null` and
  `python3 -m pytest ops/ai_eval/test_ai_eval.py -q` (it reads the case file; no network).

### T15. The feature pages say the new rules (BI-49, BI-50, BI-51, BI-52, BI-54)

In every "Signed" cell this task writes, the text is exactly
`changed 2026-11-12 (<backlog ID>), *(proposed, not yet signed)*`, with the backlog ID in
backticks. Never write that the product owner signed or accepted any of this wording. In
the lines quoted below, `\`` stands for a plain backtick.

- `docs/core/features/chart-builder-questions.md` (#486 split row 7 into rows 7, 8 and 9;
  the last row is 9, at `:53`):
  - Decisions table, a new row after row 9:
    `| 10 | A radar chart takes one to six measures, each with its own aggregation, and draws one shape per measure on one shared scale; it has no breakdown | Planner default (QA round, \`BI-49\`) | changed 2026-11-12 (\`BI-49\`), *(proposed, not yet signed)* |`
  - "Limits to tell a customer", a new last line:
    `- On a radar chart every measure shares one scale, so a measure with much smaller numbers is drawn near the centre.`
- `docs/core/features/time-grain.md`:
  - Decisions table, a new row after row 9:
    `| 10 | While the dashboard control groups dates, each chart it does not change carries a quiet mark that says why: its type takes no date grouping, it groups by a part of the date, its dimension is not a date, or it has no date grouping of its own | Planner default (QA round, \`BI-50\`) | changed 2026-11-12 (\`BI-50\`), *(proposed, not yet signed)* |`
  - "Not included", a new last line:
    `- The mark of decision 10 on an embed or a public link, and on the built-in dashboard's tiles.`
- `docs/core/features/combined-charts.md`:
  - Decisions table, row 3: append to its first cell
    ` A question the viewer's access does not cover (a table their role or organisation does not read, or a masked column the question computes on) is listed in the legend, greyed, as "not available with your access"; no column, mask or policy is named (\`BI-51\`).`
    and replace its "Signed" cell by
    `changed 2026-11-12 (\`BI-51\`), *(proposed, not yet signed)*`.
  - "Limits to tell a customer", a new last line:
    `- A masked column that a question reaches through a join, a calculated field, a metric or a SQL source that renames it is not recognised as an access matter: that question shows the general failure sentence with a reference.`
- `docs/core/features/parameters.md`:
  - Decisions table, a new row after row 6:
    `| 7 | A value in the dashboard's address that does not fit its parameter is refused for that parameter alone: the dashboard is drawn with the value the parameter has without it (the dashboard's saved value, else the default), the refusal is shown on the parameter's control, and the value is taken out of the address. A PDF, a scheduled report and an alert still refuse the whole read | Planner default (QA round, \`BI-52\`) | changed 2026-11-12 (\`BI-52\`), *(proposed, not yet signed)* |`
- `docs/core/features/reusable-metrics.md`, "Limits to tell a customer", a new last line:
  `- A person without \`metric:write\` reads the list of metrics and segments; the buttons to define or change one are not shown to them.`
- No acceptance row changes on any page. `BACKLOG.md`, `PRODUCT.md` and `HANDOFF.md` are
  the planner's.

## 3. Verification

Per commit, only for what the commit touched. Never `cargo clean`, never
`cargo test --workspace`, never a bare `cargo test -p lakehouse-api <filter>`: CI runs the
full suites on the pull request.

```bash
# Rust (T1–T7)
cd rust && cargo fmt --check
cargo clippy -p lakehouse-bi --all-targets -- -D warnings     # T1, T2, T3
cargo clippy -p lakehouse-api --all-targets -- -D warnings    # T4–T7 (and once after T3, which it depends on)
cargo test -p lakehouse-api --test sec11_guard                # T4–T7: every commit that touches lakehouse-api/src
cargo test -p lakehouse-api --test route_auth                 # the same commits
# TypeScript (T8–T13), after `bun install --frozen-lockfile` once
bun run typecheck && bun run lint
bun test <the test files the commit touched>
# always
python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py
git status    # nothing generated staged
```

Rust tests, each by name, counts quoted in the handoff:

- `lakehouse-bi` (T1, T2, T3): `cargo test -p lakehouse-bi --lib <test name>` for each test
  the task names; after T3 also `cargo test -p lakehouse-bi --lib params::tests` (the
  existing tests of `resolve` must pass unchanged).
- `lakehouse-api` unit tests (T4, T5, T6, T7): `cargo test -p lakehouse-api --lib <test name>`
  for each named test; after T4 and T5 also `cargo test -p lakehouse-api --lib time_grain`;
  after T5 also `--lib param_rewrite`, `--lib layout_tabs` and `--lib organise_paths`
  (their `DashboardQuery` literals changed); after T7 also `--lib layers_rewrite`.
- Integration test files a task adds to or changes:
  `cargo test -p lakehouse-api --test table_access` after T6 and after T7. Each
  integration test binary of `lakehouse-api` also runs one test from `src/tls_proxy.rs`,
  so its count is one more than the file's own tests (`table_access` has 13 today and
  gains three in T7). If a container image has no ARM64 build or a test cannot start for a
  reason of the machine, write *not verified locally, CI* with the error's first line; do
  not change the test.
- T14: the two commands in its check.

Before the handoff, once, in the foreground, on the final commit:

```bash
cd rust && cargo fmt --check
cargo clippy -p lakehouse-bi --all-targets -- -D warnings
cargo clippy -p lakehouse-api --all-targets -- -D warnings
cargo test -p lakehouse-api --test table_access
cargo test -p lakehouse-api --test sec11_guard
cargo test -p lakehouse-api --test route_auth
bun run typecheck && bun run lint && bun run test
python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py
git status
```

CI on the pull request (x86_64) is the final word. The handoff quotes each command with its
counts and says *not verified* for anything not run, with the reason (rule 7). It has a
part "Assistant changes for the AI team to review" (assistant parity): the tile's line and
a question's sentence in the page context (T9 step 5, D22) and the new `AI-15` case (T14).
Nothing here is proven in a browser: the owner runs `LH-BI-6-25`, `LH-BI-9-05`,
`LH-BI-13-03`, `LH-BI-11-05`, `LH-BI-11-06`, `LH-BI-10-08`, `LH-BI-7-06` and `LH-BI-2-05`
on staging after the merge.

## 3a. Machine

- The developer works on the ARM64 build machine, in the checkout
  `/home/rismana/rantai/lakehouse-agents-3`.
- Before the first cargo command of a shell:
  `export CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-3 CARGO_BUILD_JOBS=6`.
  Use plain `cargo`: there is no `cargo-locked` on that machine.
- Per Rust commit: `cargo fmt --check`;
  `cargo clippy -p <crate> --all-targets -- -D warnings`; the named unit tests as
  `cargo test -p <crate> --lib <name>`; each touched integration test file as
  `cargo test -p lakehouse-api --test <file>` (Docker runs there).
- ALWAYS also `cargo test -p lakehouse-api --test sec11_guard` and
  `cargo test -p lakehouse-api --test route_auth` when a commit touches
  `lakehouse-api/src`.
- Never `cargo test --workspace`, never a bare `cargo test -p lakehouse-api <filter>`,
  never `cargo clean`.
- Each integration test binary of `lakehouse-api` also runs one test from
  `src/tls_proxy.rs`, so its count is one more than the file's own tests.
- Before the first TypeScript check: `bun install --frozen-lockfile`. `bun run test` (the
  whole console suite) runs once, before the handoff.
- CI on the pull request (x86_64) is the final word.
- Work only under `/home/rismana/rantai/`, and start no container you do not remove.

## 4. Out of scope

- A click on a radar. Today a click on the shape names the measure, not a category
  (`src/lib/chart-click.ts`, the `default` branch of `drillTarget`); that is older than
  this slice and is not changed by it.
- An upper count of measures on the server. The server takes any number for the grouped
  kinds; six is the console's.
- The date grouping mark on an embed and on a public link (their tiles are annotated in
  `routes/embed.rs`, which this slice does not touch), and on the built-in dashboard's
  tiles.
- A chart whose base question the viewer may not read (D21), and the language of the
  rewrite's other refusal sentences (`policy_engine::refusal_message`).
- A masked column reached through a join, a calculated field, a metric or a renaming SQL
  source (D17).
- Asking the server before a parameter value is written to the address (D29), and the one
  extra read of the dashboard after a refused value is taken out of it.
- A refusal's list on the PDF, a scheduled report, an alert, the builder's preview, the
  record list and the exports: they keep the 400 (D23).
- "Add a pair" (four pairs of a join) and "Add measure" (the kind's maximum), which are
  also hidden at their limit; and, when T13 step 3 is built without `deniedFor`, the 403
  sentence of the two `list` calls of the metrics client.
- Moving `src/services/clients/dashboards.ts`, or any client other than `metrics.ts`, onto
  `deniedFor`: that is `WEB-11` slice S2.
- A read-only view of one metric's formula for a person without `metric:write`.

Follow-up lines for the planner to add to `docs/core/BACKLOG.md` (the developer does not
edit it):

- `BI-<next>` Task: a click on a radar chart names a category or does nothing; today it
  offers to filter by the measure's name. Under `BI-6`.
- `BI-<next>` Task: "Add a pair" and "Add measure" stay at their limit, off, with the
  sentence. Under `BI-6`, `BI-7`.
- `BI-<next>` Task: the date grouping mark on embeds and public links. Under `BI-9`,
  `BI-26`.
- `BI-<next>` Task: a question that reads a masked column through a join, a calculated
  field, a metric or a SQL source is recognised as an access matter. Under `BI-13`.
- `BI-<next>` Task: a person without `metric:write` can open a metric to read its formula.
  Under `BI-2`.

## Open questions for the owner

None blocks the build; the plan builds the answer named first. None of the wording in this
plan has been seen by the owner.

1. **The sentence for a question outside the viewer's access** (`BI-51`; the backlog row
   says it is the owner's to sign). Planned: "Not available with your access" (the legend
   entry, the line under the chart, the server's sentence). It appears for a table the
   viewer's role or organisation does not read and for a masked column the question
   computes on, and names neither.
2. **Where the "Group dates by" mark shows** (`BI-50`). Planned: whenever the dashboard
   applied a grain, which includes a dashboard whose saved default is a grain, so every
   viewer of such a dashboard sees the quiet mark on the charts the control does not
   change. The other reading: only after the viewer changed the control. And the four
   sentences in section 1.
3. **The value used after a refusal** (`BI-52`). Planned: the dashboard's saved value, else
   the parameter's default (what Reset shows), and a PDF, a scheduled report and an alert
   keep refusing the whole read.
4. **A radar's scale** (`BI-49`). Planned: one scale for every measure, so a measure with
   small numbers is drawn near the centre. The other choice, each measure to its own
   maximum, compares shapes and hides sizes.
5. **A person without `metric:write`** (`BI-54`). Planned: they read the list and can no
   longer open a metric or a segment (the dialog is an editor). Confirm, or ask for the
   read-only view first.

## 5. Handoff

Branch `fix/qa-round-dashboards-p2`, based on `eeb8d189` (the plan commit),
whose parent is `43d36f55`, the merge of #486.
Fifteen commits, one per task, pushed after each:

| Commit | Task |
| --- | --- |
| `8e6106ea` | T1 `test(bi): pin a radar with several measures and its breakdown refusal` |
| `1869fd21` | T2 `feat(bi): switch_skip says why the dashboard grain left a chart as it was` |
| `e632e77f` | T3 `feat(bi): resolve_each lists a refused value and keeps the rest` |
| `997d87b9` | T4 `feat(api): mark each dashboard tile the date grouping did not change` |
| `0d9a36d3` | T5 `feat(api): the dashboard read lists a refused parameter value and keeps going` |
| `6b0b9226` | T6 `feat(api): access_cause says a mask explains a statement that gave no rows` |
| `07e3ebd7` | T7 `feat(api): a question outside the viewer's access says so without a reference` |
| `172dcd8e` | T8 `feat(console): a radar draws one shape per measure on one shared scale` |
| `2c29f0f8` | T9 `feat(console): the tile says when the date grouping does not apply to it` |
| `d0e9f666` | T10 `feat(console): a question outside the viewer's access is greyed, not a red error` |
| `5da69aaf` | T11 `feat(console): a refused parameter value is shown on its control and leaves the address` |
| `584a5ff7` | T12 `feat(console): "Add a level" and "Join a table" stay at their limit, off, with the sentence` |
| `1e3353b3` | T13 `feat(console): without metric:write the metrics page is a list to read` |
| `abb4f33e` | T14 `test(ai-eval): a standard request for a radar that compares two measures` |
| `07abae4d` | T15 `docs(features): the radar, the date grouping mark, an access question, a refused value, metric:write` |

Every commit body cites its QA case IDs and backlog ID. Nothing was committed
outside the plan's scope; `BACKLOG.md`, `PRODUCT.md` and `HANDOFF.md` are
untouched.

### What each task changed, and the tests added

T1 (tests only): `a_radar_with_two_measures_is_saved_with_both_columns`,
`a_radar_is_refused_a_breakdown`. Both pass on today's server, as the plan says.

T2: `grain::SwitchSkip` and `grain::switch_skip`. Test
`the_switch_says_why_it_left_a_chart_as_it_was`.

T3: `params::{RefusedParam, RequestOutcome, ParamValues::resolve_each}`; `resolve`
is now a caller. Tests `a_refused_value_keeps_the_saved_value_and_is_listed_with_its_reason`,
`an_unknown_name_is_ignored_and_never_repeated`, `a_refused_value_is_not_in_its_reason`.

T4: `routes::support::annotate_grain_switch`, called once from the stored chart's
future in `routes::dashboard`. `mount_board` became a caller of a new
`mount_board_with(server, board_grain, charts, columns)`; a `chart_row_without_grain`
helper and `chart_row_with` were added inside `mod time_grain`. Tests
`while_the_switch_is_on_every_chart_it_does_not_change_says_why`,
`without_a_dashboard_grain_no_tile_carries_the_mark`,
`a_chart_that_kept_its_grain_because_the_new_one_does_not_fit_carries_only_grain_skipped`.

T5: `routes::params::OnRefusal`, `Loaded::refused`, a fourth argument on `load`;
`DashboardQuery::strict_params` (`#[serde(skip)]`) plus `strict_params: false` on
the seven test literals and `true` in `for_render`; the body's `refusedParams`;
`payload_for`'s `# Errors` corrected. Tests in `mod time_grain`:
`a_parameter_value_that_does_not_fit_is_listed_and_the_dashboard_is_still_read`,
`a_name_that_is_not_a_parameter_is_ignored_by_the_dashboards_read`,
`a_read_that_makes_a_document_still_refuses_the_whole_read`,
`params_that_are_not_an_object_are_still_a_bad_request`, with a
`mount_target_param` and a `dashboard_with_params` helper; and in
`param_rewrite`, `a_refused_value_reaches_no_statement`.

T6: `HiddenRead`/`hidden_read` (the rewrite's loop moved into one helper),
`AccessCause`, `PolicyEngineObligations::access_cause`. Test
`a_mask_on_a_column_the_statement_names_is_the_cause_and_another_column_is_not`.

T7: `NOT_AVAILABLE_WITH_YOUR_ACCESS`, `LayerCause`, `LayerRunner::access_is_the_cause`,
`explains`, `layer_columns`, the `match` in `ask`, and the two runners in
`layers_rewrite` (`Engine` answers false, `Stub` reads `MASKEDCOLUMN`/`HIDDENTABLE`).
`failure_of` is unchanged, to the letter. Tests
`a_question_the_viewers_access_does_not_cover_says_so_without_a_reference_or_the_engines_text`,
`only_the_gates_not_found_a_hidden_table_and_a_masked_column_the_engine_refused_count_as_access`,
and in `tests/table_access.rs` (a new `LayerFake`, modelled on `BoardFake`)
`a_question_on_a_table_the_viewers_role_does_not_read_is_marked_as_access`,
`a_question_that_sums_a_column_masked_for_the_viewer_is_marked_as_access`,
`a_question_the_engine_refuses_for_a_viewer_without_a_mask_keeps_its_reference`.

T8: `measureLimits("radar")` is `{ min: 1, max: 6, own: true }`; the radar branch
draws one shape per measure on one shared scale, with a legend only when there is
more than one. Tests `a radar takes one to six measures, each with its own
aggregate`; `a radar draws one shape per measure on one shared scale and names each
in the legend`, `a radar with one measure has no legend and keeps the scale of its
own values`; `a radar offers Add measure and sends both measures` (the column *is*
picked in the test, so nothing was left out).

T9: `GrainNotApplied`, `grainNotAppliedSentence`; `Rows.grainNotApplied`;
`GrainMarkers.notApplied`; the mark on the tile only while `appliedGrain` is set
and a grained chart is on the page; `; date grouping: <sentence>` on the tile's
page-context line. Tests: `each reason the server gives has its sentence, an
unknown reason the general one, and no reason none`; new
`src/features/dashboards/grain-marker.test.tsx` with `a chart the control does not
change carries the mark with the sentence as its label` and `without a reason no
such mark is drawn`; `a tile the date grouping does not apply to says so on its
line`; `the mark is drawn only while the dashboard applied a grain` in
`dashboard-page-role.test.tsx`.

T10: `LayerCell.cause`, `NOT_AVAILABLE_WITH_YOUR_ACCESS` (no full stop, as the
line composes it), `unavailableLayers`, `failedLayers` leaving access questions
out, the greyed `off` series, the legend `data` only when there is one, and the
muted line in `tile-body`. Tests `a question outside the viewer's access is a
greyed legend entry with no points and adds no axis`, `a chart with no such
question has the legend it always had`, `a question outside the viewer's access is
named in one muted line and not among the questions that could not be drawn`.

T11: `RefusedParam`, `acceptedParams`; `ParamControls.refusals` with the refusal
under its control (and after the controls for a parameter the page does not show);
`Payload.refusedParams`, `paramRefusals`, `paramAnswer`, the address effect, and
the three places a refusal goes away. Tests `a refused value and a name the answer
does not know leave what the address carries, and nothing left is no parameter at
all`; `a refusal is shown under its control and the control goes back to the value
in force`, `a refusal for a parameter the dashboard does not show is listed after
the controls`, `without a refusal no alert is drawn`; new
`dashboard-refused-param.test.tsx` with `a value in the address that does not fit
draws the dashboard, says why on the parameter, and is taken out of the address`.

T12: both buttons always drawn, disabled at the limit, with the sentence. The two
existing assertions changed because the rule changed (the button used to vanish),
as the plan says; new tests `says nothing about a limit below six levels` and
`keeps the button on and says nothing about a limit below four joins` (these two
also pass before the change, since below the limit nothing moved).

T13: `useAuth().hasPermission("metric:write")` on the page, the row without an
editor, the D32 sentence, `SegmentsDialog.canWrite`, and `metrics.ts` on
`deniedFor`. Tests `a person without metric:write reads the list and is offered
nothing to define or change`, `a person with metric:write is offered New metric,
New segment and Edit`, and the client test `a refused segment change does not say
metrics`.

T14: the case `chart_radar_two_measures` appended to
`ops/ai_eval/dashboard_cases.json`.

T15: the five feature pages, exactly as the plan's lines read; every "Signed" cell
written keeps `*(proposed, not yet signed)*`.

### Commands and counts (all run in the foreground, `CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-3`, `CARGO_BUILD_JOBS=6`)

Rust, per task (the plan's named tests):

- T1 `cargo test -p lakehouse-bi --lib radar` → 2 passed, 0 failed.
- T2 `cargo test -p lakehouse-bi --lib the_switch_says_why_it_left_a_chart_as_it_was`
  → 1 passed; `cargo test -p lakehouse-bi --lib grain::tests` → 17 passed, 0 failed.
- T3 `cargo test -p lakehouse-bi --lib params::tests` → 21 passed, 0 failed (the
  existing `resolve` tests included, unchanged).
- T4 `cargo test -p lakehouse-api --lib time_grain::` → 14 passed, 0 failed.
- T5 the five named tests one at a time → 1 passed each; `--lib time_grain::` → 18
  passed; `--lib param_rewrite` → 7 passed; `--lib layout_tabs` → 13 passed, 2
  ignored; `--lib organise_paths` → 62 passed, 1 ignored.
- T6 `cargo test -p lakehouse-api --lib a_mask_on_a_column_the_statement_names_is_the_cause_and_another_column_is_not`
  → 1 passed; `cargo test -p lakehouse-api --test table_access` → 14 passed, 0
  failed (13 tests + the `tls_proxy` one).
- T7 `cargo test -p lakehouse-api --lib layers_rewrite` → 8 passed, 7 ignored
  (the engine-needing ones); `--test table_access` → 17 passed, 0 failed.
- Every commit that touched `lakehouse-api/src` (T4–T7) also ran
  `--test sec11_guard` (4 passed) and `--test route_auth` (94 passed), and again in
  the handoff run; all green.
- Per commit: `cargo fmt --check` and `cargo clippy -p lakehouse-bi` or
  `-p lakehouse-api --all-targets -- -D warnings`; no warning.

Tests confirmed to fail before their fix (run, then the fix restored):

- T4: with the `annotate_grain_switch` call removed, the first test failed
  `left: Null, right: String("part")`.
- T5: with `OnRefusal::Fail` for every read, the first two failed
  `a refused value no longer fails the page's read: Params(BadRequest("parameter 'target' must be at most 50."))`
  and `an unknown name does not fail the page's read: Params(BadRequest("the request names a parameter that does not exist."))`.
- T6: with `access_cause` returning `None`, the test failed
  `left: None, right: Some(MaskedColumn)`.
- T7: with `access_is_the_cause` returning `false`, the two real-router tests
  failed `left: Null, right: String("access")` (the entries carried the gate's
  `Table not found …` and `This chart could not be loaded.` with an `errorId`).
- T8, T10, T11, T13: the test files were run with the product change stashed.
  T8 → 3 failed; T10 → 2 failed; T11 → 4 failed (the `acceptedParams` test cannot
  import it, the two control tests and the page test); T13 → 2 failed.
- T12: the two changed assertions fail on the previous code (the button was gone).
- T2, T3: not run stashed. Both add a new public item (`switch_skip`,
  `resolve_each`), so the pre-change failure is a compile error, not an assertion;
  said here rather than claimed.

Console, per task (after `bun install --frozen-lockfile`):

- T8 `bun test src/lib/chart-shape.test.ts src/features/dashboards/chart-option.test.ts`
  → 57 pass; `bun test src/features/dashboards/chart-builder.test.tsx` → 48 pass.
- T9 `bun test src/lib/grain-switch.test.ts src/lib/page-context-summary.test.ts` →
  29 pass; `grain-marker.test.tsx` + `dashboard-page-role.test.tsx` → 4 pass;
  `grain-switch.test.tsx` (the older `GrainMarkers` tests) → still green.
- T10 `combined-option.test.ts` → 10 pass; `combined-tile.test.tsx` → 6 pass.
- T11 `params.test.ts` + `param-controls.test.tsx` → 21 pass;
  `dashboard-refused-param.test.tsx` → 1 pass.
- T12 `hierarchy-dialog.test.tsx` + `joins-section.test.tsx` → 9 pass (with
  `layers-section.test.tsx` alongside → 19 pass).
- T13 `metrics-page.test.tsx` → 2 pass; `metrics.test.ts` → 1 pass.
- Each TypeScript commit: `bun run typecheck` (clean) and `bun run lint`
  (0 errors; the same 11 warnings as before this slice, all in files it does not
  touch).

T14:

- `python3 -m json.tool ops/ai_eval/dashboard_cases.json > /dev/null` → OK.
- `python3 -m pytest ops/ai_eval/test_ai_eval.py -q` → **204 passed**. `pytest` is
  not installed for this machine's `python3`, so it was run from a throwaway venv
  at `/home/rismana/rantai/venv-p2` with `pytest==8.3.4` (CI's pin) and `requests`
  (which `ai_eval.py` imports).

Before handoff, once, on the final commit (`07abae4d`):

- `cargo fmt --check` → clean.
- `cargo clippy -p lakehouse-bi --all-targets -- -D warnings` → clean.
- `cargo clippy -p lakehouse-api --all-targets -- -D warnings` → clean.
- `cargo test -p lakehouse-api --test table_access` → 17 passed, 0 failed.
- `cargo test -p lakehouse-api --test sec11_guard` → 4 passed, 0 failed.
- `cargo test -p lakehouse-api --test route_auth` → 94 passed, 0 failed.
- `bun run typecheck` → clean.
- `bun run lint` → 0 errors, 11 warnings (none in a file this slice touched).
- `bun run test` → **4238 pass, 1 skip, 0 fail**, 13060 `expect()` calls, 392
  files, 100.25 s.
- `python3 ops/lint/check_bare_iceberg_count.py` → "no bare count() against a
  Bronze Iceberg table found".
- `python3 ops/lint/check_intra_package_imports.py` → "all intra-package …
  imports resolve".
- `git status` → clean.

### Assistant changes for the AI team to review

- **The page context gained two facts, no tool or schema change** (T9 step 5, D15;
  D22). A tile's line now ends, after its `binned` part and only when the server
  sent `grainNotApplied`, with `; date grouping: <sentence>` — the same sentence as
  the mark's tooltip, from `grainNotAppliedSentence` in
  `src/lib/grain-switch.ts`, so the words exist once. A
  question's `error` in the tile's `also "…"` part reads "Not available with your
  access." for an access failure, because the server's entry now carries that
  sentence; nothing in the console rewrote it, so no code changed for D22.
- **One `AI-15` standard request added** (T14): `chart_radar_two_measures` in
  `ops/ai_eval/dashboard_cases.json` (mode `build`, `confirm: true`, its own SQL
  source "Eval Radar Two Measures", one turn, `create_chart` with
  `sqlSource`/`kind: "radar"`). `where_matches` has no list form (`_is_or_holds`
  takes one value, and the only fan-out in use is `def.measures[].metric`), so both
  measures are required by two `expect_state` checks — `def.measures` `kunjungan`
  and `def.measures` `baris`, each `count: 1` — beside the `sqlSource`/`kind`
  check.
- **`tests/fixtures/tool_schemas.json` was not regenerated**: no tool's schema
  changed in this slice (D9, D22, D30, D34 all say "none needed").

### Not verified, and why

- The `chart_radar_two_measures` case's **result**: it needs a model. Only its
  shape was checked (the two commands above).
- The eight QA cases in a browser on staging (`LH-BI-6-25`, `LH-BI-9-05`,
  `LH-BI-13-03`, `LH-BI-11-05`, `LH-BI-11-06`, `LH-BI-10-08`, `LH-BI-7-06`,
  `LH-BI-2-05`): the owner runs them after the merge.
- The seven `#[ignore]`d engine tests in `routes::layers_rewrite`: they need the dev
  `ClickHouse`, and this slice did not touch their path beyond the new trait method.
- `cargo test --workspace` and the whole-crate integration suites: per section 3 and
  3a they run once on the pull request (x86_64), not here.
- `cargo test -p lakehouse-api --test table_access` needs Docker and did run here
  (17 passed); the containers it starts are sqlx::test's Postgres ones, and none of
  them is among the containers still up (see "Machine notes").

### Machine notes

- `docker ps -a` before the first integration run: `dreamy_dhawan` and `upbeat_tu`
  (`postgres:16-alpine`, up 19 min) and `upbeat_engelbart` (`postgres:16-alpine`,
  up 9 h). The first two are gone afterwards (not by me); `upbeat_engelbart` is
  still there.
- After the runs, nine `clickhouse/clickhouse-server:26.8` testcontainers are up
  (created 10:23–11:25). They are not from any command this slice ran — its
  integration files use `wiremock` and sqlx's Postgres — so they were left in place
  for the run that started them (the `dashboards-p3` slice on this machine runs the
  ClickHouse-backed suites).
- A throwaway venv was created at `/home/rismana/rantai/venv-p2` only to run
  `ops/ai_eval/test_ai_eval.py` (see above); nothing in the repository refers to it.

### MISMATCH

- **T7 step 3 (`explains`)**: the plan says a private `fn explains`, but the plan's
  own second test lives in `routes::layers_rewrite` and reads it. A private item of
  the sibling `routes::layers` is not visible there, and the plan's path
  (`super::explains`) resolves to `routes::explains`, which does not exist. The
  function is therefore `pub(crate)` (still invisible outside the crate) and the
  test calls `crate::routes::layers::explains`. Nothing else changed; the doc
  comment says why it is `pub(crate)`.

No other anchor, signature or quoted text differed from the code at `7ac20f8be`.

## 6. Review
