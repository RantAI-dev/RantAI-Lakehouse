# QA round, Dashboards (P1): limits on save, Top N with a breakdown, the filter value list, Duplicate

Fixes for four backlog tasks opened by the QA round of 2026-11-02 to 11-12 on staging:
`BI-45` (under `BI-5`), `BI-46`, `BI-47`, `BI-48` (under `BI-6`). QA cases: `LH-BI-5-01`,
`LH-BI-6-01`, `LH-BI-6-06`, `LH-BI-6-40` (all Fail, Major; retest 2026-11-12, except
`LH-BI-6-06`, last run 2026-11-06).

Base: `main` at `512f6e37` (every anchor below was read there; the backlog rows were read at
`657c2502` and their line numbers have moved). Branch: `fix/qa-round-dashboards-p1`. One draft
PR, opened by the planner as soon as the first commit is pushed, so CI runs the Rust tests this
machine cannot (section 3).

Planner and reviewer: Claude Opus. Developer: another agent (not Opus). The developer writes
product code and tests and edits documents only where a task says so. If an anchor or a
signature below does not match the code, stop and report the mismatch in the handoff; do not
improvise.

What QA saw:

- `LH-BI-5-01`: with the time limit at 10 seconds, a slow chart cannot be saved ("request
  timeout", twice, 2026-11-05). On 2026-11-12 the preview of a single-number (KPI) chart on a
  slow SQL source (a cross join) also ended in "request timeout" after about two minutes, and
  the sentence "This chart stopped after 10 seconds…" never showed.
- `LH-BI-6-01`: "province per month, top 10" cannot be built: a date breakdown has no month
  grouping; date per month × province gives 28 rows and top 10 does not limit the provinces.
  The value list of a chart filter on `kategori` shows 11 values where the table holds 3. The
  filter column list contains `{ukuran}`, a dashboard parameter.
- `LH-BI-6-06`: the assistant's chart (month grain, province breakdown, limit 10) draws 14
  provinces and its answer says "top 10 provinces".
- `LH-BI-6-40`: a tile's menu has no Duplicate for one chart, in view mode or in layout editing.

## 1. Decisions

### Process

- **D1. One task per commit, in the order of section 2** (Rust first, then the console, then
  the documents). Each commit body cites its QA case ID and backlog ID. Each fix site carries
  a short comment naming the case (rule 13), e.g. `// QA LH-BI-5-01 (BI-45): …`. Push after
  every commit, except T1, which is pushed together with T3 (T1 adds a test that is red until
  T3).
- **D2. No new route, no migration, no new permission.** `POLICY_TABLE` and
  `tests/route_auth.rs` do not change: `GET /api/dashboard/values` keeps its row
  (`rust/crates/lakehouse-api/src/policy.rs:724`) and only gains query parameters; Duplicate
  reuses `POST /api/dashboard/specs` (`policy.rs:651`).

### BI-45: the time limit on save and on a SQL source preview

- **D3. What the code shows about the retest of 2026-11-12.** The builder previews a chart two
  ways (`src/features/dashboards/chart-builder.tsx:1146-1155`):
  - Saved, unchanged source or a mart → `POST /api/dashboard/specs/preview`
    (`rust/crates/lakehouse-api/src/routes/dashboard.rs:1082`). Every statement that reads data
    there goes through `try_run_spec_sql` (`routes/support.rs:346`), which sends the caller's
    limit and maps the engine's timeout to the sentence (`tile_failure`, `support.rs:477`). A
    KPI has one statement (`build_kpi_sql`), no layers, no bins, no second read. This path is
    limited end to end; it is the path that showed the sentence on 2026-11-05.
  - SQL that is new or edited and not saved (`previewSql`, `chart-builder.tsx:686`; the Run
    button, `use-sql-source-draft.ts:41`) → `POST /api/dashboard/sources/preview`. Its handler
    (`routes/dashboard_sources.rs:481-549`) first reads 50 rows of the source with
    `ch.query(&sql, None)` (`:514-519`): no install limit, the fixed `max_execution_time = 60`
    of `SQL_SOURCE_SETTINGS` (`lakehouse-bi/src/builder.rs:81-82`), the client's own 60 seconds,
    and no mapping of the timeout. Only after that does the chart's statement run under the
    limit (`:527`). The route's deadline is the fixed 60 seconds (`routes/mod.rs:197`, `:269-271`:
    only `/api/query/run` and the board routes follow the limit), so a source that needs its
    whole result to give 50 rows (an aggregate over a cross join does) ends in the middleware's
    "request timeout" (`mod.rs:1871-1878`) and the sentence cannot show.
  - **Explained:** the message and the missing sentence, for SQL as written. **Not explained by
    one request:** "about two minutes". Both preview routes are cut at 60 seconds, so two
    minutes is two requests in a row (Run, then the chart preview, which runs the source
    again; or Run pressed twice). The SQL QA used is not in the case, so this cannot be proven
    from the code. T1 pins the cause with a test and writes it down before the fix.
- **D4. One function sends every limited statement: `LimitedRun::send`** (new, in
  `routes/support.rs`, made from the lines `try_run_spec_sql` has today, T2). The tile and the
  chart preview reach it through `try_run_spec_sql`; saving a chart and the SQL source
  preview's row read reach it through `run_statement_limited` (new, same file). It takes the
  limit from `QueryRun::current_or_detached().limits()` (`query_limits.rs:335`, `:370`), swaps
  the SQL source's text cap with `lakehouse_bi::builder::with_run_settings`
  (`builder.rs:91`), sends `QueryRun::options` (`query_limits.rs:381`) with
  `ChClient::query_with_options`, and maps the engine's answer with `tile_failure`. No second
  copy of any of these four steps is written anywhere (rule 4).
- **D5. Saving a chart runs its statement once under the caller's limit**, as the caller's
  statement is stored (not rewritten for roles: the save keeps today's meaning, "does this
  statement execute", and only gains the limit, the row cap and the query id). This covers the
  console's save (`specs_create`, `specs_update`) and the assistant's four saves
  (`create_chart`, `update_chart`, `format_chart`, `move_chart`). When the limit ends it, the
  console's save answers **422** with the sentence
  `"This chart stopped after N seconds, the limit for this install."` (or "for your role"),
  the same status Query Studio uses for its own limit (`routes/query.rs:1532-1535`); the
  assistant's tool result is the tile's answer `{ "error": <sentence>, "limit": {…} }`, so
  the model can say the limit. Nothing is stored. The import, the version revert and
  Duplicate of a whole dashboard keep today's check (section 4).
- **D6. The SQL source preview's row read runs under the limit too.** When the limit (or a
  Stop) ends it, the route answers 422 with the same sentence and does not start the chart's
  statement.
- **D7. The route deadline follows the limit** for `/api/dashboard/specs`,
  `/api/dashboard/specs/preview` (the longest configured limit + 15 seconds, the rule of
  `/api/query/run`) and `/api/dashboard/sources/preview` (twice the longest limit + 15 seconds,
  because with a chart it runs the source twice: the row read, then the chart). Without this
  the default limit (60 seconds) can never be reported by these routes: the 60-second
  deadline fires first. `GET` and `DELETE` on `/api/dashboard/specs` share the path and so the
  bound; they send no long statement, so the longer bound changes nothing for them.

### BI-47: the value list of a chart filter

- **D8. The value list reads the chart's own relation only.** `GET /api/dashboard/values`
  takes the parameters `GET /api/dashboard/fields` already takes (`dashboard.rs:2253-2267`):
  `mart` (with optional `joins`, a JSON list) or `source` (a SQL source id). With one of them
  the values come from that mart (joined tables included) or that SQL source and from nothing
  else; `board` is then ignored. The statement goes through the same gate and
  `policy_engine::rewrite_sql_for_roles` call the route has today (`dashboard.rs:4618-4629`),
  so masks and row filters apply; the mart and every joined table are checked against table
  access first, as `/fields` does (`dashboard.rs:2369-2373`).
- **D9. Naming a mart or a source needs `dashboard:read`.** Today a person without it must
  name a dashboard they can see (`dashboard.rs:4355-4370`). `mart=` or `source=` from such a
  person is refused with `PermissionDenied("dashboard:read")` before anything is read, whatever
  `board` says (fail closed).
- **D10. SQL that is new or edited and not saved has no value list.** The list is not read;
  the picker says **"Save the SQL source to list this column's values."** A list read from
  the saved text would show the old values (rule 14: unsupported, honestly). Follow-up line in
  section 4.
- **D11. A dashboard parameter slot (`{name}`) is not a filter or grouping column.** It stays
  offered as a dimension, breakdown and measure, where the server takes it.

### BI-46: Top N with a breakdown, and a grain on a date breakdown

- **D12. The rule (accepted by the product owner, 2026-11-12).** On a bar, hbar, line or area
  chart with a breakdown, the limit N keeps the N breakdown values with the largest total of
  the chart's measure over the chart's filtered rows (the chart's own filters and the
  dashboard's); every other breakdown value is left out. With no breakdown nothing changes.
  - A dimension **with a time grain** keeps every bucket: the limit is the breakdown's. (Today
    it keeps the latest N buckets.) The chart row limit of `BI-5` still applies to the result.
  - A dimension **without a grain** keeps its own top N as today, and the breakdown is cut to
    N as well. Planner's choice, for three reasons: no saved chart gains bars; the assistant's
    rule "Top N of X by Y is dimension X, breakdown Y, limit N"
    (`routes/ai/prompt.rs:513`) stays true; both ways of building `BI-6-AC1` then show at most
    10 provinces. Open question 1.
  - "Largest" is always the largest, also on a chart sorted ascending. A breakdown chart has
    exactly one measure (`lakehouse-bi/src/store.rs:3769-3772`), so "the measure" is that one.
  - **"Other"**: the chart's existing "Other" option gathers the rest of the *dimension*
    (`builder/ranked.rs:177`); it stays that. There is no "Other" option for a breakdown, so
    left-out breakdown values are left out, not gathered.
  - Heatmap, sankey and sunburst keep their statements: there the second column is an axis, a
    target or a ring, not a series, and a sunburst cut to N outer values in all would lose its
    ring.
  - A chart whose measure is a table calculation (running total, rank, period comparison)
    gets no breakdown cap: its windows run over the whole grouped result
    (`builder/staged.rs:14-20`). It follows the bucket rule (every bucket with a grain).
- **D13. A date or timestamp breakdown can be grouped by day, week, month, quarter or year**
  (`breakdownGrain`, a new optional field of the chart definition, beside `breakdownBin`). For
  bar, hbar, line and area. Not together with a bin. The top N then counts the buckets with
  the largest totals.
- **D14. Every chart D12 applies to is built from its definition on every read** (like a
  grained chart), so the rule reaches charts saved before it, with or without a dashboard
  filter.
- **D15. Assistant parity (the AI team reviews this part).**
  - What makes the assistant's sentence come from what was saved is the `definition` line of
    the tool result, written by `definition_digest`
    (`routes/ai/tools/dashboards.rs:102-178`), and the prompt rule that a chart is described
    only by that line (`routes/ai/prompt.rs:513-514`). Both say today that a breakdown is
    never limited. Both change to D12 (T10 gives the words).
  - Tool schema: `create_chart` / `update_chart` gain `breakdownGrain`, and the description of
    `limit` says what it counts. **`rust/crates/lakehouse-api/tests/fixtures/tool_schemas.json`
    changes** (two hunks per tool at most) and is regenerated.
  - Page context: unchanged (the page sends the chart's definition, which now may hold
    `breakdownGrain`).
  - Standard request set (`AI-15`): one new case (T10).

### BI-48: Duplicate one chart

- **D16. "Duplicate" in a chart tile's menu, after "Edit chart"**, in view mode and in layout
  editing, for a person who may edit the dashboard and a chart that is the dashboard's own and
  has a definition (the conditions of "Edit chart", `tile-write-items.tsx:19-22`). It creates
  a new chart on the same dashboard from the stored definition, titled
  `"<title> (copy)"`.
- **D17. No new endpoint.** The copy is `POST /api/dashboard/specs` with the stored definition
  (the tile's `def`, which is the stored `ChartInput`, `dashboard.rs:747`; the store's own test
  shows a definition reads back as the same input, `lakehouse-bi/src/store.rs:7682-7685`). The
  server applies every check of a new chart (edit level on the dashboard, source visibility,
  the statement run under the limit, D5).
- **D18. Placement: directly below the original, on the original's tab, same width and
  height.** Every tile on that tab that starts at or below the original's bottom moves down by
  the copy's height. All of them move, not only those under the original's columns: a uniform
  shift cannot make two tiles overlap.
- **D19. Assistant parity: follow-up for the AI team.** No tool can duplicate a chart today:
  `list_charts` returns no definition (`routes/ai/tools/dashboards.rs:959-996`), and
  `create_chart` takes only its own arguments. The smallest change is `copyOf: <chart id>` on
  `create_chart`; it is not built here because it changes what a draft and its confirmation
  mean, which the AI team plans. Follow-up line in section 4.

### Words on screen (English, as the console)

- **D20.**
  - Save or SQL source Run stopped by the limit: the server's sentence, shown as it is.
  - Value list of unsaved SQL: "Save the SQL source to list this column's values."
  - Breakdown grain picker: label "Group the breakdown by"; choices "No grouping", "Day",
    "Week", "Month", "Quarter", "Year".
  - Limit field label, bar/hbar/line/area with a breakdown: with a grain on the dimension
    "Breakdown values (Top N)"; without one "Limit (Top N, dimension and breakdown)". Every
    other case keeps today's label (`chart-builder.tsx:1588`).
  - Tile menu: "Duplicate". Failure toast title: "The chart was not duplicated".

## 2. Tasks

Rust first (T1–T10), then the console (T11–T13), then the documents (T14).

### T1. Reproduce the SQL source preview without the limit, and write the cause down (LH-BI-5-01; BI-45)

- Anchors: `rust/crates/lakehouse-api/src/routes/dashboard_sources.rs:481-549`
  (`preview_for_roles`), `:514-519` (the row read); the test module `preview_tests` (`:559`
  on), which already mounts a wiremock `ClickHouse` (`mount`, DESCRIBE answered by what the
  SQL contains).
- Change: no product code. Add one test to `preview_tests`:
  **`the_row_read_of_a_sql_source_preview_carries_the_callers_time_limit_and_says_the_limit_sentence`**.
  Mount: DESCRIBE answers `describe_body()`; every other request answers status 500 with the
  body `Code: 159. DB::Exception: planted-marker Timeout exceeded (TIMEOUT_EXCEEDED)` (tell
  the two apart by what the SQL contains, as the module's `mount` does). Call
  `preview_for_roles` with no `chart`. Assert: (a) the result is
  `Err(ApiError::Unprocessable(m))` with
  `m == "This chart stopped after 60 seconds, the limit for this install."` (a test runs
  outside a request, so the limits are the defaults, as in `support.rs:1605`); (b) `m` does
  not contain `planted-marker`; (c) the row-read request (the one whose body contains
  `LIMIT 50`) carried the URL setting `max_execution_time=60` (read the recorded requests
  with `server.received_requests()`).
- On today's code (a) fails (the answer is the fixed "The SQL source failed to run.") and (c)
  fails (no URL setting). That is the reproduction.
- Write under "5. Handoff", heading "T1 cause": which statements of the request went out
  without the limit, in one paragraph, and whether you ran the test or read it (section 3).
- Check: the test above. It is red until T3. This is the only commit on the branch that may
  hold a red test; say so in its body; push it together with T3.

### T2. One place sends a limited statement: `LimitedRun` (BI-45)

- Anchors: `rust/crates/lakehouse-api/src/routes/support.rs:346-467` (`try_run_spec_sql`):
  `:381-385` (the run and its limits), `:386` (seconds), `:393-397` (`with_run_settings`),
  `:402-404` (`rows_cap`), `:405-417` (stopped check, options, `query_with_options`,
  `tile_failure`), `:444-455` (the cache key reads the statement and the limits).
- Change, in `support.rs`, no behaviour change:
  - `pub(crate) struct LimitedRun` holding the `Arc<QueryRun>` and the `TileLimits`.
  - `LimitedRun::current() -> Result<Self, SpecRunFailure>`: the lines `:381-385` moved, the
    same `Refused("The query limits could not be read.")`.
  - `LimitedRun::limits(&self) -> &TileLimits`.
  - `LimitedRun::statement(&self, sql: &str) -> String`: `:386` and `:393-397` moved
    (`with_run_settings(sql, chart_rows + 1, seconds)`).
  - `LimitedRun::rows_cap(&self) -> u32`: `:402-404` moved.
  - `LimitedRun::send(&self, ch: &ChClient, statement: &str, rows_cap: u32) -> Result<ChResult, SpecRunFailure>`:
    the stopped check, `run.options(&limits, rows_cap)`, `query_with_options` and
    `.map_err(|err| tile_failure(err, &limits))` moved from `:408-417`.
  - `pub(crate) async fn run_statement_limited(ch: &ChClient, sql: &str) -> Result<ChResult, SpecRunFailure>`:
    `LimitedRun::current()`, then `send(ch, &run.statement(sql), run.rows_cap())`.
  - `try_run_spec_sql` uses these in place of its own lines. The order stays: gate, rewrite,
    limits, `referenced_tables` on the rewritten text **before** the settings swap, the swap,
    the cache key from the swapped text. The closure `execute` calls `send` and keeps
    everything after it (columns, the cut, `limitHit`, `Computed`).
  - Each new item gets a doc comment and `# Errors` where it returns `Result`; the struct's
    comment says it is the one sender (BI-45, D4).
- Check: the existing tests of this file pass unchanged
  (`a_tile_the_time_limit_or_a_stop_ended_says_so_without_the_engines_text` `:1599`,
  `a_tile_query_sends_its_limits_and_its_id_as_url_settings` `:1650`,
  `a_point_map_and_a_pivot_read_past_the_chart_row_limit_and_a_bar_does_not` `:1702`,
  `a_tile_of_a_stopped_run_sends_no_query` `:1772`). Add, in the same test module, as a plain
  `#[tokio::test]` (no Postgres):
  **`a_statement_run_under_limits_sends_the_time_limit_and_says_the_limit_sentence_when_the_engine_stops_it`**:
  wiremock answers 500 with the code 159 body; `run_statement_limited(&ch, "SELECT 1")` gives
  `Err(SpecRunFailure::Tile(v))` with `v["error"]` the 60-second sentence and
  `v["limit"]["kind"] == "time"`, and the request carried `max_execution_time=60`,
  `max_result_rows=2001` and a `query_id`.

### T3. The SQL source preview's row read goes through it (LH-BI-5-01; BI-45, D6)

- Anchor: `routes/dashboard_sources.rs:514-519`.
- Change: replace `ch.query(&sql, None)` by `run_statement_limited(ch, &sql)`. Map its error
  with one new function in `support.rs`, used here and in T4:
  `pub(crate) fn failure_as_api_error(failure: SpecRunFailure, on_engine: impl FnOnce(&ChError) -> ApiError) -> ApiError`:
  `Tile(answer)` → `ApiError::Unprocessable(<the string at answer["error"]>)` (if it is not a
  string, `"The query did not finish."`); `Refused(m)` → `ApiError::Unprocessable(m)`;
  `Gated(e)` → `e`; `Clickhouse(err)` → `on_engine(&err)`. Here `on_engine` is this file's
  `classify_ch_error` (`:102`). The statement text stays as it is built today (the
  `SQL_SOURCE_SETTINGS` suffix is what `with_run_settings` swaps). Update the module comment
  (`:14-15` says "2 000 rows, 30 s": write that the rows and seconds are the administrator's
  limits, defaults 2,000 and 60).
- Check: T1's test passes. Add to `preview_tests`:
  **`a_chart_is_not_run_when_the_row_read_of_its_sql_source_hit_the_time_limit`**: with
  `chart` in the body and the row read answering code 159, the result is the 422 sentence and
  no request whose body contains the chart's aggregate (`sum(`) was received. The existing
  tests of the module stay green unchanged.

### T4. Saving a chart from the console runs under the limit (LH-BI-5-01; BI-45, D5)

- Anchors: `lakehouse-bi/src/store.rs:5745-5764` (`insert_chart`, `smoke_test_chart`,
  unlimited), `:5772` (`insert_chart_unchecked`);
  `lakehouse-api/src/routes/dashboard.rs:1008-1016` (`specs_create`), `:1286-1292`
  (`specs_update`).
- Change:
  - `support.rs`: `pub(crate) async fn insert_chart_checked(ch: &ChClient, spec: &StoredChartSpec) -> Result<(), SpecRunFailure>`:
    when `spec.spec.sql` is not empty, `run_statement_limited(ch, &spec.spec.sql)` (the rows
    are dropped); then `store::insert_chart_unchecked(ch, spec)` with its error as
    `SpecRunFailure::Clickhouse`. Doc comment: why the statement is not rewritten for roles
    (D5), and that import, revert and board duplication still use `store::smoke_test_chart`.
  - `specs_create` and `specs_update`: call `insert_chart_checked` and map with
    `failure_as_api_error(f, |err| upstream_error::ch_error_as(&upstream_error::DATABASE, err, upstream_error::FailedAs::BadRequest))`,
    which is today's mapping for an engine failure. Update the two `# Errors` comments (a
    422 when the limit ends the statement).
  - Do not change `lakehouse-bi/src/store.rs`.
- Check, in `support.rs` tests, plain `#[tokio::test]`:
  **`a_chart_whose_statement_hits_the_time_limit_is_not_saved_and_the_failure_is_the_limit_sentence`**
  (wiremock answers code 159 to the `SELECT`; the result is `Tile` with the sentence; no
  request containing `INSERT INTO console.bi_chart` was received) and
  **`a_chart_whose_statement_runs_is_saved_after_one_limited_run`** (the `SELECT` carried
  `max_execution_time=60`; one `INSERT` followed). And
  **`a_limit_failure_becomes_a_422_with_the_sentence_and_an_engine_failure_keeps_the_callers_mapping`**
  for `failure_as_api_error` (pure). The route-level statuses are covered in CI by the
  existing `tests/` suites; name any that change in the handoff.

### T5. The assistant's saves run under the limit (BI-45, D5; assistant parity of `BI-5`)

- Anchors: `routes/ai/tools/dashboards.rs:595` and `:817` (`create_chart`, `update_chart`),
  `routes/ai/tools/look.rs:382` (`format_chart`), `routes/ai/tools/boards.rs:555`
  (`move_chart`): each calls `store::insert_chart`.
- Change: each calls `support::insert_chart_checked`. One new function in `support.rs`,
  `pub(crate) fn failure_as_tool_reply(failure: SpecRunFailure) -> Value`: `Tile(answer)` →
  `answer`; `Refused(m)` → `json!({ "error": m })`; `Gated(e)` →
  `json!({ "error": e.to_string() })`; `Clickhouse(err)` →
  `upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json()` (what three of the
  four sites do today). `boards.rs:555` keeps its own `upstream_error_json` for the
  `Clickhouse` case (match that variant first, the rest through `failure_as_tool_reply`).
  No tool schema changes.
- Check: **`a_tool_reply_for_a_save_the_limit_ended_is_the_sentence_with_its_limit`** in
  `support.rs` tests (pure: a `Tile` answer comes back as it is; a `Clickhouse` failure has an
  `error` and never the engine's text). Grep `store::insert_chart(` and
  `store::smoke_test_chart(` under `rust/crates/lakehouse-api/src` afterwards and list every
  remaining call in the handoff; the expected ones are `routes/dashboard_organise.rs:616`,
  `routes/dashboard_file.rs:466` and the test `routes/organise_paths.rs:2753`.

### T6. The save and preview routes wait for the limit (LH-BI-5-01; BI-45, D7)

- Anchors: `routes/mod.rs:269-271` (`limit_bearing`), `:299-307` (`deadline_for`), `:220-240`
  (the table in the doc comment), `:1995-2031` (the test).
- Change: `limit_bearing` also holds `/api/dashboard/specs`, `/api/dashboard/specs/preview`
  and `/api/dashboard/sources/preview`. In `deadline_for`, before the general
  `limit_bearing` arm, an arm for `/api/dashboard/sources/preview`:
  `longest.saturating_mul(2).saturating_add(DEADLINE_MARGIN)`. Add the three routes to the
  doc table and say why the SQL source preview gets twice the limit (D7).
- Check: extend `limit_bearing_routes_follow_the_longest_configured_limit`: with a longest
  limit of 300 seconds, `/api/dashboard/specs` and `/api/dashboard/specs/preview` give 315
  seconds, `/api/dashboard/sources/preview` gives 615, and each gives `route_timeout(path)`
  when the limit is `None`. The "other" list stays as it is.

### T7. The value list reads the chart's own mart or SQL source (LH-BI-6-01; BI-47, D8, D9)

- Anchors: `routes/dashboard.rs:4294-4322` (`ValuesQuery`, `for_board`), `:4343-4403`
  (`values`), `:4355-4370` (the check for a person without `dashboard:read`), `:4454-4524`
  (`board_value_selects`; the loop body `:4479-4510` makes one relation per chart),
  `:4576-4611` (board branch, then every `serving` mart with the column), `:4618-4629` (gate
  and role rewrite); `:2359-2387` (`joined_fields_body`: the joins parse `:2365-2366`, the
  table-access check `:2369-2373` and `chart_relation` `:2386-2387`, to mirror), `:2490-2495` (`source_fields_body`: the visible-source lookup to mirror).
- Change:
  - `ValuesQuery` gains `mart: Option<String>`, `joins: Option<String>`, `source:
    Option<String>` (each `#[serde(default)]`, with a doc comment); `for_board` sets them
    `None`.
  - `values` (the handler): when `mart` or `source` is present and not empty, require
    `principal.has("dashboard:read")`, else `ApiError::PermissionDenied("dashboard:read")`;
    this check comes before the existing board check and does not replace it. For `mart`,
    read `crate::routes::catalog::table_access_for` and refuse the mart and every joined
    table the caller's roles hide with `mart_not_found(table)`, as `:2369-2373` does.
  - Move the part of the loop in `board_value_selects` that turns one chart's source into
    `(key, from, cols)` into one function (a SQL source that passes `check_sql_source`,
    rendered with `Relation::Sql(..).render()`; or a mart with its joins through
    `lakehouse_bi::joins::chart_relation`), and call it from the loop and from the new branch.
    No second copy (rule 4).
  - `values_with_limit`: a new first branch when `mart` or `source` is given. `source`:
    `lakehouse_bi::sources::get_source`, kept only if
    `crate::routes::dashboard_sources::source_visible(viewer, &s)`, else
    `ApiError::NotFound(format!("SQL source '{}' not found.", id))`. `mart`: strip
    `serving.`, validate with `Ident::new` (else `BadRequest("invalid mart")`), parse `joins`
    (else `BadRequest("joins is invalid")`), `mart_columns(ch)`, `chart_relation` (its
    message as `BadRequest`, as `:2386-2387`). If the relation has no column of that name, the
    answer is the empty list. The other filters narrow the list as on a board
    (`filter_predicates`, `values_select`); `SQL_SOURCE_SETTINGS` when it is a source. Then
    the existing gate, rewrite and read, unchanged. `board` is not read in this branch.
  - Update the doc comments of `ValuesQuery` and `values`.
- Check, in the module `values_enforcement` (`dashboard.rs:5984`), following its two
  harnesses:
  - **`the_values_of_a_charts_own_mart_are_read_from_that_mart_and_no_other`**: two marts
    both have the column; with `mart=<one>` the statement sent names that mart and not the
    other. Fails today (the parameter is ignored and both are read).
  - **`the_values_of_a_charts_sql_source_are_read_from_that_source`**: with `source=<id>` the
    statement wraps the source's SQL and names no other table.
  - **`a_masked_viewer_gets_the_masked_values_of_the_charts_own_mart`**: the existing masked
    case of this module, with `mart=`.
  - **`a_column_the_charts_own_relation_does_not_have_gives_an_empty_list`**.
  - In `rust/crates/lakehouse-api/tests/dashboard_access_routes.rs`, beside the cases at
    `:655-675`: a person who holds only a rule gets 403 for
    `/api/dashboard/values?column=province&mart=<mart>&board=<a board they see>`
    (**`a_person_without_dashboard_read_cannot_name_a_mart_for_a_value_list`**). CI only.

### T8. Top N keeps the N breakdown values with the largest total (LH-BI-6-01, LH-BI-6-06; BI-46, D12, D14)

All in `rust/crates/lakehouse-bi`.

- Anchors: `src/builder.rs:574-594` (`QueryBuilder` fields), `:780-895` (`build`: `:819-822`
  `latest` and `bucket_limit`, `:836-869` the breakdown statement), `:1509-1539` (`grained`,
  `:1535` `latest_limit`), `:1633-1645` (`rebuilds`), `:2479-2605` (`rebuild_chart_over`,
  `:2590-2605` the last builder), `:2794-2840` (`GrainedChart`, `from_def`, `:2828-2829` its
  limit), `:2853-2922` (`grained_sql`); `src/builder/ranked.rs:58-64` (`where_sql`), `:85-93`
  (`rank_expr`), `:137-173` (`ranked_sql`, `:151-164` the kept dimension values);
  `src/builder/staged.rs:80-83` (`latest`, `keep`), `:178`; `src/store.rs:129-140`
  (`breakdown_allowed`), `:431-433` (`uses_shaping`), `:459-466` (`effective_order`),
  `:3878-3892` (the `GrainedChart` literal); `src/specs.rs` (`ChartKind`).
- Change:
  1. `specs.rs`: `ChartKind::breakdown_is_series(self) -> bool`, true for `Bar`, `Hbar`,
     `Line`, `Area`.
  2. `store.rs`, on `ChartInput`: `caps_breakdown(&self) -> bool` (the kind's breakdown is a
     series and `breakdown` is present and not empty) and
     `keeps_latest_buckets(&self, grain: Grain) -> bool`
     (`!self.caps_breakdown() && grain::cuts_to_latest(grain, &self.effective_order())`).
     `uses_shaping` also returns true when `caps_breakdown()` (D14: such a chart is saved by
     `finish_shaping` and rebuilt on every read; say so in its comment).
  3. `builder.rs`: `QueryBuilder` gains `cap_breakdown: bool` (false in `over`).
     `ranked.rs`: `pub(super) fn breakdown_cap(&self) -> Option<String>`: `None` unless there
     is a breakdown, no steps, no groupings, a first measure, and that measure is not a table
     calculation (`self.from.table_calc(..)`); else
     `"{b} IN (SELECT {b} FROM {from}{where} GROUP BY {b} ORDER BY {rank} DESC, {b} LIMIT {n})"`
     with `{from}` = `self.from.render()`, `{where}` = `self.where_sql()`, `{rank}` =
     `self.rank_expr(first measure)`, `{n}` = `self.limit`. And
     `pub(crate) fn with_breakdown_cap(mut self) -> Self`: sets `cap_breakdown` to whether
     there is a breakdown, pushes `breakdown_cap()` (computed before the push) onto
     `where_clauses`, returns `self`.
  4. The dimension when `cap_breakdown` is true **and** the builder has a grain: no bucket is
     cut. `build` (`:819`): `latest` is false when `cap_breakdown`; the breakdown statement
     (`:836-869`) is then written without the `{dimension} IN (SELECT … LIMIT …)` guard:
     `SELECT {dimension}, {breakdown}, {agg_expr} FROM {from} {where_sql}GROUP BY {dimension}, {breakdown} ORDER BY {key}{key_dir}, {breakdown}{settings}`.
     `ranked_sql`: the guard is the plain `WHERE` of the clauses (none when there are none).
     `staged.rs`: `latest` is false and the final `WHERE __rk <= {keep}` is left out. Without
     a grain, or with `cap_breakdown` false, every statement is byte for byte what it is
     today.
  5. `rebuild_chart_over`: after the `raw_where` loop (`:2600-2605`),
     `if def.caps_breakdown() { builder = builder.with_breakdown_cap(); }`.
     `GrainedChart` gains `pub cap_breakdown: bool`; `from_def` sets it from
     `def.caps_breakdown()` and, when it is true, takes
     `grain::clamp_limit(kind, def.limit_value())` as `limit` in place of `effective_limit`
     (with a breakdown the limit is the breakdown's, so a date part must not swap it for its
     range). `grained_sql` calls `with_breakdown_cap()` before `build()` when the field is
     true. The literal at `store.rs:3878` sets `cap_breakdown: false` with a comment like the
     one on `sort: None` there (`finish_shaping` rebuilds a capped chart).
  6. `grained` (`:1535`): `latest_limit` is `Some` only when
     `!pivot && def.keeps_latest_buckets(grain)`.
  7. Update the module comment of `ranked.rs` and the doc of `build` with the rule and
     `BI-46`.
- Check, in `builder.rs` tests (they use `grained_spec`, `dated`, `sql_with_filters_report`):
  - Replace `a_breakdown_keeps_the_latest_buckets_and_orders_by_bucket_then_series`
    (`:4306`), which pins the rule the owner changed, by
    **`a_breakdown_beside_a_time_grain_keeps_every_bucket_and_the_top_breakdown_values`**:
    same chart (line, quarter, breakdown `g`, limit 8); the statement is
    `SELECT d, g, round(sum(v)) AS v FROM <bkt> WHERE g IN (SELECT g FROM <bkt> GROUP BY g ORDER BY sum(v) DESC, g LIMIT 8) GROUP BY d, g ORDER BY d, g`
    where `<bkt>` is the bucketed relation of the old expectation, and `latest_limit` is
    `None`. Say in the commit body that the test changed because the rule did (rule 2).
  - **`a_breakdown_without_a_grain_keeps_the_top_dimension_values_and_the_top_breakdown_values`**:
    a bar, limit 3: the statement contains the breakdown cap with `LIMIT 3` and still contains
    the dimension guard.
  - **`a_dashboard_filter_narrows_the_rows_the_breakdown_values_are_ranked_over`**: the
    filter's predicate is inside the cap's subquery.
  - **`a_heatmap_keeps_its_statement_and_its_second_column_is_not_capped`**: no
    `IN (SELECT <breakdown>`.
  - **`a_date_part_with_a_breakdown_caps_the_breakdown_at_the_charts_own_limit`**:
    `day_of_week`, breakdown, limit 5: the cap says `LIMIT 5`, not 64.
  - In `store.rs` tests: **`a_bar_chart_with_a_breakdown_is_saved_with_the_capped_statement`**
    (`uses_shaping()` is true, the stored statement holds the cap) and
    **`a_chart_without_a_breakdown_is_saved_with_the_statement_it_always_had`**.
  - Every other existing test that fails must be one that pins the old rule for a bar, hbar,
    line or area chart with a breakdown: change its expectation to the new rule and list it in
    the handoff. If any other test fails, stop and report.

### T9. A date breakdown can be grouped by a grain (LH-BI-6-01; BI-46, D13)

All in `rust/crates/lakehouse-bi`.

- Anchors: `src/shape.rs:512-521` (`breakdown_bin`), `:607-619` (`is_used`), `:699-768`
  (`check_kind_fields`), `:911-975` (`check_against_relation`; `RelationCheck::breakdown`
  `:894`), `:1163-1170` (a grouping's grain: parse and `validate_column`), `:1026-1027` (the
  "grain or bins, not both" sentence); `src/builder.rs:2438-2476` (`rebuild_chart`; the bins
  branch `:2455-2467` is the model), `:257-268` (`Relation::Bucketed`), `:2756-2771` (nested
  buckets for a grouped table); `src/builder/binned.rs` (`needed_columns`);
  `src/grain.rs:515` (`bucket_expr`).
- Change:
  1. `ShapeFields` gains `breakdown_grain: Option<String>` (`rename = "breakdownGrain"`,
     skipped when `None`, default), after `breakdown_bin`; `is_used` counts it.
  2. `check_kind_fields`: when it is set, the kind must be bar, hbar, line or area
     (`ChartKind::breakdown_is_series`), else
     `"a breakdown grain is for bar, hbar, line and area charts."`; the name must parse to
     day, week, month, quarter or year, else
     `"invalid breakdown grain '<raw>': use day, week, month, quarter or year."`; with `bin`
     or `breakdownBin` set, the existing sentence
     `"a chart groups by a grain or by bins, not both."`.
  3. `check_against_relation`: when it is set, an empty `check.breakdown` is
     `"a breakdown grain needs a breakdown column."`; otherwise
     `grain.validate_column(kinds.get(check.breakdown).copied())?` (the same call a
     grouping's grain makes).
  4. `rebuild_chart`: after the bins branch, when the definition has a `breakdownGrain` and a
     breakdown: resolve the measures, take
     `binned::needed_columns(from, &[dimension, breakdown], &measures)?`, put the predicates
     into a `Relation::Filtered` when there are any, and wrap it in a `Relation::Bucketed`
     whose `dimension` is the breakdown's name, whose `expr` is
     `grain.bucket_expr(&breakdown_ident, kind_of_the_breakdown, read.time)?` and whose
     `carried` is the needed columns without the breakdown. Then
     `rebuild_chart_over(spec, &relation, Vec::new(), &[], (read, cols), grained)`: the
     dimension's own grain, when it has one, nests its bucket on top, as a grouped table's
     grains do. Comment the branch: the breakdown already is its bucket in the relation, so
     everything after it (the cap of T8 included) reads buckets.
- Check, `builder.rs` tests:
  **`a_date_breakdown_grouped_by_month_is_bucketed_under_the_chart_and_capped_by_bucket`**
  (the statement holds the month bucket expression aliased to the breakdown's name, groups by
  dimension and breakdown, and its cap ranks the buckets), and
  **`a_grain_on_the_dimension_and_on_the_breakdown_nest_one_bucket_each`**. `shape.rs` tests:
  **`a_breakdown_grain_is_refused_without_a_breakdown_on_a_text_column_on_a_pie_and_beside_a_bin`**
  (one assertion per sentence above) and
  **`a_breakdown_grain_reads_back_as_it_was_saved`** (serde round trip, absent when not set).
  Add a second date column to the test fixture if it has only one.

### T10. The assistant says what the limit counts, and can set the breakdown grain (LH-BI-6-06; BI-46, D15)

In `rust/crates/lakehouse-api`. The AI team reviews this commit.

- Anchors: `src/routes/support.rs:767-790` (`annotate_saved_grain`, `:783`);
  `src/routes/ai/tools/dashboards.rs:102-178` (`definition_digest`, `:146`, `:162-164`), its
  tests `:3040-3090`; `src/routes/ai/prompt.rs:514` (the limit rule) and its test
  `:1427-1465`; `src/routes/ai/registry.rs:276-281` (`with_shape_properties`, `limit`),
  `:334` (`breakdownBin`), `:3324-3373` (the field-list test), `:3271-3299` (the fixture
  writer `:3276` and the snapshot test `:3288`); `tests/fixtures/tool_schemas.json`;
  `ops/ai_eval/dashboard_cases.json:58-70` (`chart_line_monthly`, the model for a case with
  a source); `src/features/copilot/chart-draft-card.tsx:99`.
- Change:
  1. `annotate_saved_grain`: `latest_limit` uses `chart.def.keeps_latest_buckets(grain)`.
  2. `definition_digest`: the "latest buckets" arm (`:146`) applies only when
     `def.keeps_latest_buckets(grain)`. When `def.caps_breakdown()` and a limit is set, the
     part reads, with a grain on the dimension,
     `limit: <n> (the <n> <breakdown> values with the largest total; every <grain> bucket of <dimension> is kept)`
     and without one
     `limit: <n> <dimension> values, and the <n> <breakdown> values with the largest total`.
     A parameter limit keeps `parameter <name>` in place of the number, as today. A chart
     with a breakdown that is not capped (heatmap, sankey, sunburst) keeps
     `; the breakdown is not limited`. A `breakdownGrain` adds the part
     `breakdown grain: <grain>` after `grain`. Update the function's doc comment.
  3. `prompt.rs:514`: replace the first two sentences (from "A chart's `limit` counts" to
     "…saying the list is fixed at today's top N.") by:
     `A chart's \`limit\` on a bar, hbar, line or area chart with a breakdown keeps the N breakdown values with the largest total; a time grain on the dimension then keeps every period, and a dimension without a grain keeps its own top N as well. Without a breakdown, \`limit\` counts values of the dimension, and with a time grain ordered by that date it keeps the latest periods. For \"top N of Y over time\" use the date as the dimension with a grain, Y as the breakdown and limit N.`
     The sentences after it stay. Update the test at `:1427-1465` to the new clauses and keep
     its check that the rule follows the top-N sentence and is absent from the Ask prompt.
  4. `registry.rs`: the `limit` description gains
     `; on a bar, hbar, line or area chart with a breakdown it keeps that many breakdown values, the ones with the largest total`.
     After `breakdownBin` add
     `"breakdownGrain": { "type": "string", "enum": ["day", "week", "month", "quarter", "year"], "description": "group a date or timestamp \`breakdown\` by this period; bar, hbar, line and area only; not with a bin" }`.
     Add `"breakdownGrain"` to both field lists of the test at `:3324`.
  5. Regenerate the fixture with the ignored writer
     (`write_tool_schema_fixture`, section 3). The diff must touch only `limit` and
     `breakdownGrain` of `create_chart` and `update_chart`; anything else in the diff is a
     mistake.
  6. `ops/ai_eval/dashboard_cases.json`: one case after `chart_line_monthly`, id
     `chart_top_n_breakdown_over_time`, mode `build`, `confirm: true`, the same `setup` with
     the title `"Eval Visitors Top Regions"`, the turn
     `Draw a line chart of foreign visitors per month from the saved source "Eval Visitors Top Regions", one line per region (kawasan), top 3 regions only.`,
     `expect_tool` `create_chart` with `{"sqlSource": "{src}", "grain": "month", "breakdown": "kawasan", "limit": 3}`,
     and `expect_state` with `def.grain` `month`, `def.breakdown` `kawasan`, `def.limit` 3,
     count 1.
  7. `chart-draft-card.tsx:99`: carry `breakdownGrain` from the tool's arguments into the
     draft's definition when it is a string, beside `breakdownBin` (this one console line
     belongs to this commit so the draft preview matches the tool).
- Check: update the two digest tests (`:3040`, `:3068`) to the new lines and add
  **`a_definition_digest_says_a_limit_beside_a_breakdown_counts_the_breakdown`** (grain and
  no grain, a heatmap unchanged, a breakdown grain listed). The prompt test and the registry
  test as changed. `tool_schemas_snapshot_is_byte_identical` passes. The request set itself
  is not run here (it needs a model endpoint): *not verified*, the AI team runs it.

### T11. The builder asks for its own source's values, and hides parameter slots from filters (LH-BI-6-01; BI-47, D8, D10, D11)

- Anchors: `src/features/dashboards/filters/use-filter-values.ts:24-58`;
  `src/features/dashboards/filters/filter-editors.tsx:16-25` (`EditorProps`), `:46`, `:94`;
  `src/features/dashboards/filters/value-list.tsx:14-25`;
  `src/features/dashboards/chart-shape-section.tsx:134-143` (`ChartFilters`), `:181-184`
  (`board=""`); the four users of `ChartFilters`: `chart-builder.tsx:1616`,
  `segments-dialog.tsx:123`, `metric-dialog.tsx:205`, `layers-section.tsx:171`;
  `value-group-dialog.tsx:58`; `chart-builder.tsx:409` (`choice`), `:682-686`
  (`heldPreview`), `:803-806` (slots pushed into the pickers), `:925-929` (`columnChoices`);
  `src/lib/params.ts:54` (`slotParam`); `src/lib/chart-joins.ts` (`joinsPayload`).
- Change:
  1. `use-filter-values.ts`: export
     `type OwnSource = { mart: string; joins?: unknown[] } | { sqlSource: string } | { unsaved: true }`.
     The hook takes an optional `source?: OwnSource`. With a mart it sends `column`, `mart`
     and, when the list is not empty, `joins` as JSON, and no `board`; with a SQL source it
     sends `column` and `source`; with `{ unsaved: true }` it sends nothing and answers
     `{ values: [], truncated: false, loading: false, error: "Save the SQL source to list this column's values." }`.
     Without `source` it behaves as today. The effect depends on the source as text.
  2. `EditorProps` gains `source?: OwnSource`; `TextEditor`, `NumberEditor` and `ValueList`
     hand it to the hook.
  3. `ChartFilters` gains a required `source: OwnSource` and passes it to `FilterEditor`;
     remove the comment at `:182`. Its four users pass the source they already hold:
     `chart-builder.tsx` passes `{ unsaved: true }` while `heldPreview` is true or there is no
     `choice`, `{ mart: choice.name, joins: joinsPayload(shape.joins).joins }` for a mart,
     `{ sqlSource: choice.id }` for a saved SQL source; the segment and metric dialogs pass
     their mart or SQL source; a question of a combined chart passes its own. If one of them
     has no source at hand, stop and report.
  4. `value-group-dialog.tsx:58`: pass the dialog's `source` in the same shape.
  5. `chart-builder.tsx:925-929`: `columnChoices` leaves out every name for which
     `slotParam(name)` is not null. The pushes at `:803-806` stay.
- Check:
  - New `src/features/dashboards/filters/use-filter-values.test.tsx`:
    **"the value list of a chart filter asks for the chart's own mart and names no board"**,
    **"it sends the joins of a joined chart"**, **"it asks for a SQL source by id"**,
    **"an unsaved SQL source makes no request and says to save it"**, **"a dashboard filter
    still asks by board"**.
  - `chart-shape-section.test.tsx`: opening a text filter requests
    `/api/dashboard/values` with the mart it was given.
  - `chart-builder.test.tsx`: with a list parameter `ukuran` defined, **"a parameter slot is
    offered as a dimension and not as a filter column"**.
  - `value-group-dialog.test.tsx` stays green; add an assertion that the request names the
    dialog's source.

### T12. Duplicate in a chart tile's menu (LH-BI-6-40; BI-48, D16–D18)

- Anchors: `src/features/dashboards/tile-write-items.tsx:10-24`;
  `src/features/dashboards/dashboard-page.tsx:659-666` (`persistLayout`), `:748-755`
  (`chartIds`, `shownLayout`), `:758` (`canWrite`), `:822-838` (`addSectionTile`: create,
  place, reload, the model), `:1061-1067` (`chartWriteItems` call), `:1105-1107` (where the
  entries go); `src/lib/dashboard-tabs.ts:108-112` (`tabOf`), `:171` (`tilesOnTab`);
  `src/services/contracts/dashboards.ts:90` (`saveBoardLayout`), `:103` (`saveChart`);
  `src/services/clients/dashboards.ts:168-179`.
- Change:
  1. `saveChart` returns `Promise<{ id: string }>`: the id of `chart` in the route's answer
     (`{ ok, chart }`), an empty string if it is absent. Callers that ignore the result need
     no change. Update the contract's comment.
  2. `src/lib/dashboard-tabs.ts`:
     `export function placeCopyBelow(tabs, layout, ids, originalId, copyId): LayoutMap`. The
     copy's box is the original's with `y` at the original's bottom (same `x`, `w`, `h`,
     `tab`); every other tile on the original's tab (`tabOf`) whose `y` is at or below that
     bottom gets `y + h`; tiles on other tabs are untouched. An original with no box gives
     the layout back unchanged.
  3. `chartWriteItems` takes `onDuplicate` and returns it in `edit`, after "Edit chart", only
     when `hasDef`: `{ label: "Duplicate", icon: <Copy />, onSelect: opts.onDuplicate }`.
     Update its doc comment.
  4. `dashboard-page.tsx`: `duplicateChart(spec)` calls
     `dashboardService.saveChart({ ...spec.def, title: \`${spec.title} (copy)\`, board }, false)`;
     with a non-empty id it saves
     `placeCopyBelow(tabs, shownLayout, chartIds, spec.id, id)` with
     `dashboardService.saveBoardLayout(board, …)` (awaited, not the debounced
     `persistLayout`); then `await load()`. A failure is
     `notifyFailure("The chart was not duplicated", writeError(e))`. Pass it as `onDuplicate`
     at `:1061`.
- Check:
  - `src/lib/dashboard-tabs.test.ts`: **"a copy sits directly below its original with the
    same size and tab"**, **"every tile at or below the original's bottom on that tab moves
    down by the copy's height"**, **"tiles on another tab do not move"**, **"an original with
    no box leaves the layout as it is"**.
  - New `src/features/dashboards/tile-write-items.test.tsx`: **"a person who may edit gets
    Duplicate after Edit chart"**, **"a person who may not edit gets no Duplicate"**, **"a
    built-in tile and a chart with no definition get no Duplicate"**.
  - `src/services/clients/dashboards.test.ts`: `saveChart` returns the new chart's id.
  - `dashboard-page-role.test.tsx` (or the page test nearest to the tile menu): choosing
    Duplicate posts the chart's definition with the title ending in `" (copy)"` and the
    board, then saves a layout that holds the new id.

### T13. The builder offers a grain on a date breakdown and says what the limit counts (LH-BI-6-01; BI-46, D12, D13, D20)

- Anchors: `src/lib/time-grain.ts:20-25` (`GRAINS`, `TRUNCATIONS`), `:84` (`grainChoices`),
  `:109` (`bucketLabel`), `:220-222` (`limitHasEffect`); `src/lib/chart-shape.ts:82`,
  `:264-292` (`ShapeDraft`, `emptyShapeDraft`, `shapeDraftFromDef`), `:333-339` (the bin
  payload, the model); `src/services/clients/bi-store.ts` (`ChartInput`);
  `src/features/dashboards/chart-builder.tsx:548-549`, `:611` (`canBreakdown`), `:617-620`,
  `:644-647` (`pickBreakdown`), `:1479-1484` (the breakdown's extra controls), `:1588` (the
  limit label); `src/features/dashboards/tile-body.tsx:193-197` (bucket labels of the
  dimension).
- Change:
  1. `time-grain.ts`: `BREAKDOWN_GRAINS` (day, week, month, quarter, year);
     `breakdownGrainChoices(chartKind, column)`: those five when the kind is bar, hbar, line
     or area and the column is a date or a timestamp, else none;
     `limitHasEffect(grain, order, capsBreakdown = false)` is true when `capsBreakdown`;
     `labelSeriesBuckets(rows, series, grain)`: the rows with the `series` column replaced by
     `bucketLabel(grain, value)`.
  2. `chart-shape.ts` and the client `ChartInput`: `breakdownGrain` in the definition type,
     in the draft (`Grain | ""`, empty by default, read from a saved definition when it is a
     grain), and in the payload only when the context has a breakdown and the kind is one of
     the four.
  3. `chart-builder.tsx`: `pickBreakdown` clears `breakdownGrain` when the new column is not
     a date or timestamp. Where the breakdown's extra controls are drawn, a select "Group the
     breakdown by" with "No grouping" and the five grains, shown when
     `breakdownGrainChoices` is not empty (the bin picker beside it stays for number
     columns). `capsBreakdown` (the kind is one of the four, `canBreakdown`, a breakdown is
     chosen) goes to `limitHasEffect` (`:620`) and picks the limit label of D20 (`:1588`).
  4. `tile-body.tsx`: after the dimension's labels, when the definition has a
     `breakdownGrain` that is a grain and the chart has a series column, the rows go through
     `labelSeriesBuckets` before `relabelBins`.
- Check: `time-grain.test.ts` (**"a date breakdown on a line chart is offered day to
  year"**, **"a text breakdown and a pie are offered none"**, **"the limit has an effect
  beside a breakdown even on a date part"**, **"series buckets are labelled like the axis"**);
  `chart-shape.test.ts` (**"the payload carries the breakdown grain only with a breakdown on
  a bar, hbar, line or area"**, **"a saved breakdown grain reads back into the draft"**);
  `chart-builder.test.tsx` (**"choosing a date breakdown offers its grain and the preview
  request carries it"**, **"the limit label names the breakdown when the chart has one"**).

### T14. The feature pages say the new rules (BI-45, BI-46, BI-47)

- `docs/core/features/chart-builder-questions.md`:
  - Decisions table, a new last row:
    `| 7 | On a bar, hbar, line or area chart with a breakdown, the limit keeps the N breakdown values with the largest total of the measure over the chart's filtered rows; the others are left out. A dimension with a time grain keeps every period; a dimension without one keeps its own top N as before. A date breakdown can be grouped by day, week, month, quarter or year | Owner, QA round 2026-11-12 (\`BI-46\`); the rule for a dimension without a grain is the planner's | Accepted by the owner, 2026-11-12 |`
  - "Limits to tell a customer", two new lines:
    `- A chart with a time grain and a breakdown shows every period (up to the chart row limit); to keep only recent periods, add a relative date filter to the chart.`
    `- The value list of a chart filter reads the chart's own table or SQL source only; while a SQL source is new or edited and not saved, no list is offered.`
- `docs/core/features/query-limits-and-stop.md`, Decisions table, row 5, append to its first
  cell: ` Saving a chart and running or previewing a SQL source stop at the same limit with the same sentence (\`BI-45\`).`
- No acceptance row changes. `BACKLOG.md`, `PRODUCT.md` and `HANDOFF.md` are the planner's.

## 3. Verification

This build machine has 11 GB of memory and little disk. Never `cargo clean`. All cargo
commands go through `/home/shiro/rantai/bin/cargo-locked`, one at a time.

Per commit, only for what the commit touched:

```bash
# Rust
cd rust && cargo fmt --check
/home/shiro/rantai/bin/cargo-locked check -p lakehouse-bi -j 2     # T8, T9
/home/shiro/rantai/bin/cargo-locked check -p lakehouse-api -j 2    # T2–T7, T10 (and after T8/T9, which it depends on)
# TypeScript
bun run typecheck && bun run lint
bun test <the test files the commit touched>
# always
python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py
git status    # nothing generated staged
```

Rust tests:

- `lakehouse-bi` (T8, T9): `/home/shiro/rantai/bin/cargo-locked test -p lakehouse-bi --lib <test name> -j 2`,
  one name at a time. Quote the counts.
- `lakehouse-api` unit tests (T1–T7, T10): only as
  `/home/shiro/rantai/bin/cargo-locked test -p lakehouse-api --lib <test name> -j 2`, never
  without `--lib` and never without a name. The test build of this crate did not finish on
  this machine before. Try it once, for T1; if it has not finished after 45 minutes or memory
  runs short, stop it and from then on write every `lakehouse-api` test as *not verified
  locally, CI* (rule 7), naming each test so the reviewer finds it in the CI log.
- The fixture of T10: `/home/shiro/rantai/bin/cargo-locked test -p lakehouse-api --lib write_tool_schema_fixture -j 2 -- --ignored`.
  If that build cannot finish here, edit `tests/fixtures/tool_schemas.json` by hand (the file
  is `serde_json::to_string_pretty` output, keys in the order the code inserts them, the same
  hunk in `create_chart` and in `update_chart`) and write *not verified locally, CI*:
  `tool_schemas_snapshot_is_byte_identical` in CI decides.
- Integration tests under `rust/crates/lakehouse-api/tests/` (T7's access case) and every
  `sqlx::test` are left to CI: list them as *not verified locally, CI*.
- `cargo clippy` is CI's on this machine: *not verified locally, CI*.
- T10's case file: `python3 -m json.tool ops/ai_eval/dashboard_cases.json > /dev/null`.

Before the handoff, once, in the foreground, on the final commit:

```bash
cd rust && cargo fmt --check
/home/shiro/rantai/bin/cargo-locked check -p lakehouse-api -j 2
bun run typecheck && bun run lint && bun run test
python3 ops/lint/check_bare_iceberg_count.py && python3 ops/lint/check_intra_package_imports.py
git status
```

The handoff quotes each command with its counts, and names everything not run with the
reason. Nothing here was proven in a browser: the owner runs `LH-BI-5-01`, `LH-BI-6-01`,
`LH-BI-6-06` and `LH-BI-6-40` on staging after the merge.

## 3a. Change of machine (planner, 2026-11-12)

The developer works on the ARM64 build machine, in the checkout
`/home/rismana/rantai/lakehouse-agents-3` (20 CPUs, 121 GB of RAM), not on the machine
section 3 describes. Section 3's list of what to run stays; how to run it changes:

- Before the first cargo command of a shell: `export CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-3 CARGO_BUILD_JOBS=6`.
  There is no `/home/shiro/rantai/bin/cargo-locked` here: where section 3 writes
  `/home/shiro/rantai/bin/cargo-locked <args> -j 2`, run `cargo <args>`.
- Per Rust commit, for each crate the commit touched: `cargo fmt --check`,
  `cargo clippy -p <crate> --all-targets -- -D warnings`, and the unit tests the task names,
  as `cargo test -p <crate> --lib <test name>`. These are no longer left to CI.
- An integration test file under `rust/crates/lakehouse-api/tests/` that a task adds to or
  changes is run as `cargo test -p lakehouse-api --test <file>` (Docker is running here). Never
  a bare `cargo test -p lakehouse-api <filter>`. If a container image has no ARM64 build or a
  test cannot start for a reason of the machine, write *not verified locally, CI* with the
  error's first line; do not change the test.
- Before the first TypeScript check: `bun install --frozen-lockfile`.
- CI on the pull request (x86_64) is still the final word. The handoff quotes each command
  with its counts and says *not verified* for anything not run (rule 7).
- The machine is shared with another project: start no container you do not remove, stop no
  process you did not start, and work only under `/home/rismana/rantai/`.

## 4. Out of scope

- The import of a dashboard file (`routes/dashboard_file.rs:466`), the version revert
  (`routes/dashboard_organise.rs:616`) and Duplicate of a whole dashboard
  (`lakehouse-bi/src/store.rs:2955`) keep `store::smoke_test_chart` without the limit. They
  check up to 200 charts in one request and need their own rule for a chart the limit ends.
- A preview that runs several statements each close to the limit (a chart with more
  questions; a chart with steps, whose rows are read a second time by `preview_rows`,
  `support.rs:1111`) can still pass the route deadline.
- The value list's own read (`dashboard.rs:4630-4631`) is not put under the chart time limit.
- `GET /api/dashboard/values` with neither a board nor a source still reads every `serving`
  mart that has the column (the parity corpus and two tests call it that way). The console no
  longer calls it so after T11.
- A value list for SQL that is not saved (D10), and for a calculated field used as a filter
  column (empty today and after).
- A breakdown cap for a chart whose measure is a table calculation; "Other" for a breakdown;
  the limit on heatmap, sankey and sunburst (D12).
- The assistant duplicating a chart (D19).
- `/api/ai/chat` keeps its own deadline (120 seconds by default): a limit longer than that
  still ends a tool's save with the chat's timeout.
- The phone layout: a copy takes the place a new chart takes there today.

Follow-up lines for the planner to add to `docs/core/BACKLOG.md` (the developer does not edit
it):

- `AI-<next>` Task: the assistant can duplicate a chart: `create_chart` takes
  `copyOf: <chart id>`, reads the stored definition as the caller and applies the call's
  other arguments on top; regenerate `tests/fixtures/tool_schemas.json`; one `AI-15` case.
  Under `AI-6`. **Waits for `BI-48`.**
- `BI-<next>` Task: import, version revert and board duplication check each chart under the
  time limit. Under `BI-5`.
- `BI-<next>` Task: a preview with several statements stays inside one time limit; the
  second read of a chart with steps is skipped when the first one failed. Under `BI-5`.
- `BI-<next>` Task: a value list for a SQL source that is not saved yet. Under `BI-6`.

## Open questions for the owner

None blocks the build; the plan builds the answer named first.

1. **Top N with a breakdown, when the dimension has no time grain.** Planned: the dimension
   keeps its own top N as today and the breakdown is cut to N as well (D12). The other
   reading of the accepted rule is that the limit moves to the breakdown entirely and every
   dimension value is shown. Which one?
2. **Saved charts with a time grain and a breakdown.** Under the accepted rule they show
   every period, where today they show the latest N. A chart such as "daily visitors by
   channel, limit 30" will show every day until someone adds a relative date filter ("last 30
   days"), and past the chart row limit (2,000 rows by default) it is cut. Confirm that this
   goes out as it is, with a line in the release notes.
3. **No value list for SQL that is not saved (D10).** Confirm at QA, or ask for the
   follow-up to come first.

## 5. Handoff

## 6. Review
