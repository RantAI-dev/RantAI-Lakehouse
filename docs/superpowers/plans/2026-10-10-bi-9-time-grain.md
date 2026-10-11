# BI-9: group dates by day, week, month, quarter, year — Implementation Plan

**Status:** decisions signed 2026-10-10, not started. Written by the planner
(Claude Opus) for a developer agent, under the role split in `AGENTS.md`.

**Feature page:** `docs/core/features/time-grain.md`. Specs:
`docs/core/specs/bi-9.md`, `docs/core/specs/ai-3.md` (ships together).

**Base:** `feat/uiux` at `f720e64` (phase 1 branch). Read first:
`lakehouse-bi` `builder.rs` (`QueryBuilder::build`, `Relation::Filtered`,
`rebuild`, `relative_predicate`, `date_expr`), `filters.rs` (`ColumnKind`),
`store.rs` (`ChartInput`, `ensure_board_state_columns`,
`update_board_refresh`), `upstream_error.rs` and `tests/sec11_guard.rs`,
and how `BI-18` part B added `refreshSeconds` and paged records.

---

## 1. Decisions already made (do not re-ask, do not change)

| Decision | Choice |
| --- | --- |
| Grain on a chart | Optional `grain` on `ChartInput` (and so in `def`), absent on every chart saved before. Truncations: `minute`, `hour`, `day`, `week`, `month`, `quarter`, `year`. Parts: `hour_of_day`, `day_of_week`, `day_of_month`, `week_of_year`, `month_of_year`, `quarter_of_year`. Any other string is refused at save (400, plain message). |
| Where it applies | The chart's `dimension`, and only when that column's `ColumnKind` is `Date` or `DateTime`. `minute`, `hour` and `hour_of_day` need `DateTime`. Kinds: `bar`, `hbar`, `line`, `area`, `stacked`, `combo`, `waterfall`, `heatmap`, `pie`, `rose`, `funnel`, `treemap`, `radar`; `calendar` accepts `day` only. Every other kind refuses a grain. |
| SQL | The dimension is replaced by a bucket expression that keeps the dimension's name as the result column, so the console still reads `rows[spec.x]`. Dashboard filters keep applying to the raw column, not the bucket (the alias must not shadow the column in `WHERE`; reuse the `Relation::Filtered` approach). A truncation returns the bucket's start as text: `YYYY-MM-DD` for day and coarser, `YYYY-MM-DD HH:MM:SS` for minute and hour. A part returns a number. |
| Order and limit | A grained chart is ordered by bucket ascending unless the editor chose a value order. Its limit range is 1 to 1000 (others unchanged); when more buckets exist than the limit and the order is by bucket, the latest ones are kept and the tile payload carries `truncated: true`. Day of week is ordered from the configured first day. |
| Settings | Two deployment-wide values: `reportTimeZone` (IANA name, default `Asia/Jakarta`) and `weekStart` (`monday` or `sunday`, default `monday`). Stored in Postgres (new migration, next free number at build time, with a why-header). `GET /api/settings/reporting` is `RequiresAuth`; `PUT` needs the new permission `settings:write`. A zone name is accepted only if the engine knows it. |
| Time context everywhere | Bucket expressions on a `DateTime` column, and the existing date-filter code (`date_expr`, `relative_predicate`, the hard-coded `toStartOfWeek(today(), 1)`), take the zone and week start from the settings. A `Date` column is never shifted. Because the settings can change, a grained chart's SQL is always rebuilt at read time, never served from the stored `spec.sql`. |
| Dashboard control | `GET /api/dashboard` takes `grain=<truncation>`. It replaces the grain of every chart whose own grain is a truncation; charts with a part, or without a grain, are untouched. A chart whose column cannot take the requested grain (`hour` on a `Date`) keeps its own and is reported on the tile like `filtersSkipped`. The saved default is a new `grain` column on `console.bi_board` (the `refresh_seconds` pattern), written through `PUT /api/dashboard/boards` (`dashboard:write`), returned in the payload. Embeds and public links use the saved value and offer no switch. |
| Click on a bucket | Records: the route takes the tile's bucket value and returns the rows whose raw column falls in that bucket, computed by the server in the report zone. Cross-filter: for `day` and coarser, the console sets one `between` filter, first day to last day of the bucket; `minute` and `hour` offer records only. A part offers neither; the tile menu says why. A click destination (`BI-18` part B) receives the same range (dashboard) or the bucket start text (URL). |
| Assistant | `grain` in the `create_chart` and `update_chart` schemas (enum of the thirteen values). `describe_mart` and `list_sql_sources` return each column's kind (`number`, `date`, `datetime`, `text`). `tool_schemas.json` regenerated. One entry, "Make this monthly.", added to the standard request set. |

## 2. What exists today (anchors, verified at `f720e64`)

- `rust/crates/lakehouse-bi/src/store.rs:254` `ChartInput` (no `deny_unknown_fields`: an unknown `grain` is dropped silently today); `:1498` `validated_mart_columns` returns names only, no types; `:2028` `limit_and_order` (calendar 1 to 366, others 1 to 100); `:492` `ensure_board_state_columns`; `:1066` `update_board_refresh`.
- `rust/crates/lakehouse-bi/src/builder.rs`: `QueryBuilder::build` prints the dimension bare, with no alias, in the plain form, six times in the breakdown form, and in the box-plot and point-label forms; `:840` `rebuild` (rebuilds from `spec.def`); `:491` `sql_with_filters_report` returns the stored `spec.sql` unchanged when no filter applies; `:761` `date_expr` (`toDate(col)`, no zone); `:807` `relative_predicate`, Monday hard-coded at `:811`.
- `rust/crates/lakehouse-bi/src/filters.rs:367` `ColumnKind::from_clickhouse_type` (`Number`, `Date`, `DateTime`, `Text`).
- `rust/crates/lakehouse-api/src/routes/support.rs:352` `mart_columns` (name and type from `system.columns`); `:323` `stored_chart_sql`, also called by `routes/embed.rs`.
- `rust/crates/lakehouse-api/src/routes/dashboard.rs:44` `DashboardQuery`; `:640` `BoardEditBody`; `:1184` `records_for_roles`; `:1690` `chart_def_fields` (YAML export lists `def` fields by hand).
- `rust/crates/lakehouse-api/src/routes/ai/registry.rs:186` / `:207` chart tool schemas; `:1280` `write_tool_schema_fixture` (`cargo test -p lakehouse-api --lib write_tool_schema_fixture -- --ignored`).
- No time zone or week-start setting, no settings table, route or page anywhere. Latest migration: `0061_semantic_entry_roles.sql`. Admin pages: `src/app/(admin)/admin/*`, nav at `src/components/app-shell/nav-config.ts:221`.
- `src/features/dashboards/chart-builder.tsx`: keeps only `dimensions` / `measures` from `/api/dashboard/fields` and discards `columns[{name,type}]`. `src/features/dashboards/chart-option.ts`: the x axis is a category axis of raw strings; `:308` calendar `firstDay: 1`. `src/lib/chart-axis.ts` `monthlyAxisLabel` matches `YYYY-MM` only. `src/lib/chart-transforms.ts` `toCalendar` slices the first ten characters.
- Compose runs `clickhouse/clickhouse-server:26.8`.

## 3. Tasks

One commit per task; Rust first, the migration in the same sitting as T1's code.

### T1 — Reporting settings (server)

Migration, store functions, `GET` / `PUT /api/settings/reporting`, `POLICY_TABLE` rows, `tests/route_auth.rs` both ways. Zone validated against the engine's own list; a database or engine failure goes through `upstream_error`.

Accept: unit and route tests; `PUT` without `settings:write` is 403; an unknown zone and a third `weekStart` value are 400 with a fixed message; with no row saved, `GET` returns the defaults.

### T2 — Grain in `lakehouse-bi`

A `Grain` type (parse, truncation or part, needs-time), a `TimeContext { zone, week_start }` passed into the builder, the bucket expression, kind and column-kind validation at save (so the save path must learn column types, for marts and SQL sources), order, the 1 to 1000 limit with latest-N and `truncated`, filters on the raw column. `date_expr` and `relative_predicate` take the `TimeContext`. A grained chart always rebuilds.

Accept: unit tests for every one of the thirteen values on `Date` and `DateTime`, both week starts, the refusals, and the breakdown form; existing filter tests updated only where the zone argument now appears, each change named in the handoff.

### T3 — API wiring

Settings loaded once per request and passed down (dashboard, preview, records, values, export, embed, public). `grain` on `DashboardQuery`; saved `grain` on the board; `grainSkipped` and `truncated` on tiles; records by bucket; `chart_def_fields`; column kinds in `describe_mart` / `list_sql_sources`; tool schemas and fixture.

Accept: route tests for the override, the skip report, the saved default, records by bucket, and embed using the saved grain; `sec11_guard` and `route_auth` pass; **every generated statement run against the real engine** (section 6).

### T4 — Console: Admin > Settings

Contract, client, service, page under `src/app/(admin)/admin/settings`, nav entry. Two selects (zone list searchable), Save, shown read-only without `settings:write`. Little text.

Accept: component tests; typecheck, lint.

### T5 — Console: grain in the chart builder and on axes

Keep column kinds from the fields response; a "Group by" select beside the dimension, shown only for a date or timestamp column, offering what that column and kind allow; calendar on a timestamp column sets `day`. A `src/lib/time-grain.ts` that labels buckets (`Mar 2026`, `Q1 2026`, `2026`, `Mon`, `Jan`, week by its first day) and feeds the axis, tooltip and legend; the calendar's first day follows the setting.

Accept: unit tests for labels in both week starts; builder tests for when the select appears.

### T6 — Console: dashboard grain control

A compact control in the filter row, present only when the dashboard has a truncation-grained chart; the choice travels in the page URL beside `f`, is sent as `grain`, and is saved by "Save as default". A skipped tile shows the existing skipped marker with its reason; a cut-off tile shows a mark.

Accept: tests for URL round trip, save, and the markers.

### T7 — Console: click on a bucket

Drill menu and click destinations per the decision table: records by bucket, `between` filter for day and coarser, nothing for parts with the reason in the tile menu.

Accept: tests for the range of a month, a quarter, a week in both week starts, and February of a leap year.

### T8 — Docs

`CHANGELOG.md` `[Unreleased]` (including the behaviour change to relative date filters); the standard request set entry for `AI-3`; `docs/core/specs/bi-9.md` "Today" left alone.

## 4. PR slicing

Commits T1…T8 on the phase 1 branch (`feat/uiux`, PR #104). The AI team reviews the assistant part of T3 and T8.

## 5. Out of scope (do not build)

Grains on kinds not listed; per-chart linking to the dashboard control; fiscal calendars; a switch in embeds or public links; per-user or per-tenant zones; first days other than Monday and Sunday; built-in tiles.

## 6. Things the developer must verify, not assume

Mock-only tests have missed real-engine failures three times on this branch. Run each of these against the dev `ClickHouse` (26.8) and quote the statement and result in the handoff:

- The exact truncation and part functions and their zone and week-mode arguments, on `Date`, `Date32`, `DateTime`, `DateTime('zone')`, `DateTime64` and their `Nullable` forms; what a null date groups as.
- That the aliased bucket does not capture a `WHERE` on the same column name, in the plain and the breakdown form, with a filter on the grained column and with one on an aggregated column.
- Week of year: which numbering the engine gives for each week start around 1 January, and that the console's label agrees.
- A zone with daylight saving (for example `Europe/Berlin`) at an hour grain across a change.
- How the engine's list of zone names is read, and that reading it does not need a privilege the API's user lacks.
- That latest-N then ascending order returns the right rows when the limit cuts.
- Whether the migration number is still free against `origin/main` and open PRs on the day; if not, take the next and say so.
- Where the standard request set of `AI-15` lives; if it does not exist yet, report that instead of inventing a file.
- That `settings:write` follows the rules in the `policy.rs` module doc for introducing a permission string.

## 7. Handoff (developer appends one entry per PR)

### Partial notes (developer, uncommitted)

- 2026-10-10 T1 done (not committed): migration `0066_reporting_settings.sql` (0061 is the latest on origin/main but open PR #101 holds 0062 to 0065, so 0066 was taken), `lakehouse-store/src/reporting_settings.rs` + test, `routes/settings.rs`, `POLICY_TABLE` rows, policy test, `tests/reporting_settings_route.rs` (6 pass). `lakehouse-bi/src/grain.rs` (Grain, TimeContext, resolve, trim) written, 16 unit tests. Next: T2 builder (Relation::Bucketed, TimeContext in filters, grain in store).
- 2026-10-10 T2 done (not committed): `lakehouse-bi` builder (`Relation::Bucketed`, `ReadContext`, `grained_sql`, `RecordsValue`), `ChartInput.grain`, save-time validation with column types, `Board.grain` + `update_board_grain` (the ClickHouse `grain` column on `bi_board`, T3's storage). `cargo test -p lakehouse-bi`: 184 lib pass; clippy clean. API crate does not compile yet against the new signatures: next is T3 wiring.
- 2026-10-10 T3 done (not committed): API wiring (`routes/settings.rs::time_context`, `ReadContext` through `stored_chart_sql`, `DashboardQuery.grain`, `appliedGrain`/`reporting` in the payload, `grainSkipped`/`truncated` on tiles, records by bucket, board `grain` on `PUT /boards`, embed uses the saved grain, export, AI tools + schemas + fixture regenerated). Real-engine checks run (see handoff when written). `cargo test -p lakehouse-api --lib`: 1566 pass; route_auth/sec11_guard/embed_tokens pass. Next: T4 (Admin > Settings), T5-T7 console, T8 docs.
- 2026-10-10 T4 done (not committed): contract/client/service for settings, `features/admin/reporting-settings-page.tsx` (+test, 3 pass; zone picker is an `Input` with a native `datalist`, a Combobox popup was not testable in happy-dom), `lib/time-zones.ts`, page under `src/app/(admin)/admin/settings`, nav entry. typecheck and lint clean. Next: T5.
- 2026-10-10 T5 to T7 done (not committed): `lib/time-grain.ts`, `lib/grain-switch.ts`, `lib/bucket-click.ts` (+tests), chart builder "Group by" (kinds from the fields response), `TileBody` relabels buckets and takes the calendar first day from `ReportingProvider`, dashboard grain switch + markers + bucket click in `dashboard-page.tsx`, server adds `grain`/`grainColumn`/`reporting`/`own`. Next: T8 (CHANGELOG), final checks, handoff.

### BI-9 / AI-3 — T1 to T8 (developer, 2026-10-10)

**Not committed**, as instructed: everything is in the working tree of `feat/uiux` on top of `acd3b36`. Nothing was seen in a browser. The dev API binary and dev servers were not touched, and the new migration was not applied to the dev Postgres (so the running API still lacks the routes).

**Migration number.** `0066_reporting_settings.sql`. `origin/main` ends at `0061`, but open PR #101 (`feat(sources): connector reliability, part 1`) holds `0062` to `0065`, so I took `0066` (plan section 6).

**Files per task**

- T1: `rust/migrations/0066_reporting_settings.sql`; `lakehouse-store/src/reporting_settings.rs` (+`lib.rs`, `tests/reporting_settings.rs`); `lakehouse-api/src/routes/settings.rs` (+`routes/mod.rs`); `policy.rs` (two rows, module doc, test `settings_write_needs_no_seed_change_and_no_seeded_role_has_it`); `tests/reporting_settings_route.rs`.
- T2: new `lakehouse-bi/src/grain.rs` (`Grain`, `TimeContext`, `WeekStart`, `TimeZoneName`, `resolve`, `trim_to_latest`); `builder.rs` (`Relation::Bucketed`, `ReadContext`, `RecordsValue`, `GrainedChart`, `grained_sql`, `QueryBuilder::grain`, `FilteredSql.{grain_skipped,grain,grain_column,latest_limit}`, `TimeContext` in `filter_predicates` / `records_sql` / date and relative predicates); `store.rs` (`ChartInput.grain`, column kinds in `Resolved`, save-time validation, `Board.grain`, `update_board_grain`, `bi_board.grain` column, `spec_from_input` / `spec_from_inline_sql` take a `&TimeContext`).
- T3: `routes/support.rs` (`stored_chart_sql` takes `&ReadContext`, `annotate_grain`, `annotate_saved_grain`); `routes/dashboard.rs` (`DashboardQuery.grain`, `GrainRequest`, `appliedGrain` / `grain` / `reporting` in the payload, records by `grain`, `grain` on `PUT /boards`, `chart_def_fields`, specs create/update/preview with the settings); `routes/embed.rs`; `routes/dashboard_sources.rs`; `routes/ai/registry.rs`, `tools/{mod,dashboards,data}.rs`; `tests/fixtures/tool_schemas.json`; `lakehouse-alerts/src/lib.rs` (passes the default context; its tiles are KPI/gauge with no filters).
- T4: `src/services/{contracts,clients}/settings.ts`, `services/index.ts`, `src/features/admin/reporting-settings-page.tsx` (+test), `src/lib/time-zones.ts` (+test), `src/app/(admin)/admin/settings/page.tsx`, `nav-config.ts`.
- T5: `src/lib/time-grain.ts` (+test), `chart-builder.tsx` (kinds from the fields response, "Group by"), `tile-body.tsx` (relabels buckets), `chart-option.ts` (calendar first day), `reporting-context.tsx`, `tile-dialogs.tsx`, `embed-view.tsx`, `public-dashboard.tsx`.
- T6: `src/lib/grain-switch.ts` (+test), `features/dashboards/grain-switch.tsx`, `grain-marker.tsx` (+test), `dashboard-page.tsx`.
- T7: `src/lib/bucket-click.ts` (+test), `chart-click.ts` (+tests), `records.ts`, `drill.tsx`, `dashboard-page.tsx`.
- T8: `CHANGELOG.md`.

**Real engine (`clickhouse/clickhouse-server` 26.8.9.10, container `lakehouse-clickhouse-1`, user `default`, a scratch database `scratch_bi9` dropped afterwards; `serving.mart_demo_map_points` read only)**

- Functions and types, on `Date`, `Date32`, `DateTime`, `DateTime('Asia/Tokyo')`, `DateTime64(3)` and `Nullable(Date)` / `Nullable(DateTime)`, every grain: all ran. Results: week to year via `date_trunc` return `Date` (`Date32` for a `Date32` and a `DateTime64` input); `toDate32(toTimeZone(c,'Asia/Jakarta'))` is `Date32`; `date_trunc('minute'|'hour', toTimeZone(c, ..))` is `DateTime('Asia/Jakarta')` (`DateTime64(0, ..)` for `DateTime64`); parts are `UInt8`; `Nullable` input gives `Nullable` output.
- **Falsified plan assumption:** `toStartOfMonth`, `toStartOfWeek`, `toStartOfQuarter`, `toStartOfYear`, `toDate` clip a `Date32` outside 1970 to 2149 silently: `SELECT toStartOfMonth(toDate32('1960-05-15'))` returned `1970-01-01` (type `Date`); `toStartOfInterval` returned `2139-06-07` for the same value. `date_trunc('month', toDate32('1960-05-15'))` returned `1960-05-01` (`Date32`). So the builder uses `date_trunc` (week and coarser) and `toDate32` (day of a timestamp); documented in `grain.rs`. Verified on the scratch table (`d32` 1960 row: month `1960-05-01`, week `1960-05-09`, year `1960-01-01`; `dt64` 1960 day `1960-05-15`).
- A null date groups as `NULL`: `SELECT date_trunc('month', nd) AS b, count() ... GROUP BY b ORDER BY b DESC LIMIT 3` returned the three latest months, with `NULL` sorting last in both directions (so latest-N drops it first; `trim_to_latest` also drops `NULL` first).
- Alias shadowing: the first design (`SELECT date_trunc('month', d) AS d ... WHERE d ...`) was replaced by an inner relation: the bucket is projected under the dimension's name in `(SELECT <bucket> AS d, <breakdown>, <measures> FROM (SELECT * FROM t WHERE <raw predicates>) AS flt) AS bkt`, so a `WHERE` on the same column name reads the raw column. Run against the scratch table: plain month with a filter on `d` itself (`d >= toDate32('2025-12-01') AND d <= toDate32('2026-03-31')`) returned `2025-12-01 18, 2026-01-01 38, 2026-02-01 139, 2026-03-01 48`; a filter on the aggregated column (`v >= 20`) ran; the breakdown form (quarter on a timestamp, and month with a filter on the grained column) ran and returned the bucket-then-series order. Same shapes ran on `serving.mart_demo_map_points`: `SELECT * FROM (SELECT visit_date, round(sum(visitors)) AS visitors FROM (SELECT date_trunc('month', visit_date) AS visit_date, visitors FROM (SELECT * FROM serving.mart_demo_map_points WHERE (visit_date >= toDate32('2020-01-01'))) AS flt) AS bkt GROUP BY visit_date ORDER BY visit_date DESC LIMIT 4) ORDER BY visit_date` returned `2026-08-01 10929, 2026-09-01 18357, 2026-10-01 31758, 2026-11-01 11299`.
- Week of year: `toWeek(d, 3)` (Monday, ISO) gave 53 for 2021-01-01 and 1 for 2026-01-01; `toWeek(d, 6)` (Sunday, at least four days) gave 53 for 2021-01-01, 1 for 2021-01-03 and 53 for 2026-01-01 (the Sunday week 28 Dec to 3 Jan has three days of 2026); across 2020-12-27 to 2027-01-04 both stay within 1 to 53. The console labels `W<number>` from the number the engine returns, so label and engine agree by construction.
- Daylight saving, `Europe/Berlin`, `date_trunc('hour', toTimeZone(t,'Europe/Berlin'))` over 2026-10-25 (fall back): four groups, the repeated `02:00:00` appears twice (two instants, `toUnixTimestamp` 1792886400 and 1792890000), so the typed bucket keeps the two instants apart (a `toString` grouping gave four rows too in my one run, so I did not measure a merge; the typed bucket is chosen because it groups by instant by construction). Spring forward 2026-03-29: `01:00`, `03:00`, `04:00`, `05:00` (no `02:00`).
- Zone list: `SELECT count() FROM system.time_zones` = 598; a user created with only `GRANT SELECT ON scratch_bi9.*` read `SELECT count() FROM system.time_zones WHERE time_zone='Europe/Berlin'` and got 1 (the throwaway user was dropped), so the check needs no privilege the API's user lacks. The API's user is `default` (config `CH_USER`).
- Latest-N then ascending: `SELECT * FROM (SELECT d, round(sum(v)) AS v FROM (SELECT date_trunc('month', d) AS d, v FROM scratch_bi9.t) AS bkt GROUP BY d ORDER BY d DESC LIMIT 5) ORDER BY d` over data spanning Nov 2025 to Jul 2026 returned the five latest months, ascending (`2026-03-01` to `2026-07-01`). Sunday-first day of week: `ORDER BY d % 7` returned `7, 1, 2, 3, 4, 5, 6`.
- Sunday-first week bucket: `date_trunc('week', d + toIntervalDay(1)) - toIntervalDay(1)` gave a Sunday for a Saturday, a Monday and a Wednesday input, and kept the `Date` / `Date32` / `Nullable(Date)` types. `toStartOfWeek(toDate(now('Asia/Jakarta')), 1|0)` gave `2026-10-05` / `2026-10-04` on 2026-10-10.
- The relative filter's "today" is `toDate(now('Asia/Jakarta'))` (type `Date`). The Rust unit tests pin the exact text of each statement; I ran the same shapes by hand, not the Rust output, against the engine.
- The API with the new code against the real engine was **not verified** (the dev API binary was not rebuilt or restarted); the API tests use `wiremock` for `ClickHouse`.

**Where the standard request set of `AI-15` lives.** It does not exist yet (`docs/core/specs/ai-15.md` is a spec; no file or test holds requests). I did not create one. The T8 line "Make this monthly." for the standard request set is therefore **not done**; `AI-3-AC1` is covered only by the tool schema (`grain` enum) and `describe_mart` / `list_sql_sources` returning column kinds.

**`settings:write` and the `policy.rs` rules.** No seed grant; the string is in the module doc, in the `POLICY_TABLE` comment, and a test proves Platform Admin's `*:*` has it and none of the six seeded role grants does (`dashboard:write` alone does not). `tests/route_auth.rs` sweeps the table, so both routes are walked both ways (`route_auth`: 32 pass); `tests/reporting_settings_route.rs` adds a `dashboard:write` principal refused 403 and a `settings:write` principal allowed.

**Commands run (all foreground, `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target CARGO_BUILD_JOBS=2`, one at a time, `-p` scoped; lib.rs touched before clippy)**

- `cd rust && cargo fmt --check`: clean (after `cargo fmt`, which only touched my hunks; the tree was clean before).
- `cargo clippy -p lakehouse-bi -p lakehouse-alerts --all-targets -- -D warnings`: clean. `cargo clippy -p lakehouse-api --all-targets -- -D warnings`: clean (it found a `let...else`, a 122-line function split into `records_value`, a `&Option`; one `#[allow(clippy::too_many_lines, reason = ..)]` on `spec_from_chart_input`).
- `cargo test -p lakehouse-bi`: 184 lib + 1 + 0 pass. `cargo test -p lakehouse-api --lib`: 1568 pass, 1 ignored. `--test route_auth`: 32; `--test sec11_guard`: 4; `--test embed_tokens`: 21; `--test reporting_settings_route`: 6. `cargo test -p lakehouse-store --test reporting_settings`: 3. `cargo test -p lakehouse-alerts --lib`: 70.
- `bun run typecheck` clean; `bun run lint` 0 errors, 6 warnings (all in files I did not touch); `bun run test`: 1138 pass, 1 skip, 0 fail, 1139 tests in 127 files.
- Test executables over 20M in `deps` deleted afterwards (8 files); `debug/incremental` removed; free disk 24G.
- *Not verified:* anything in a browser; the real API with the new routes; `cargo test` for other crates and other `lakehouse-api` test files (`security_regressions`, `parity`, ...); workspace-wide clippy and tests; `docker compose`; `ops/g*` gates; the migration applied to a Postgres other than the test containers.

**Existing tests changed, and why**

- `lakehouse-bi` `builder::tests`: four relative-date and date-range tests (`a_date_range_renders_re_printed_dates_and_a_datetime_compares_by_day`, `exclusive_ends_use_strict_comparisons_on_numbers_and_dates`, `relative_dates_use_only_the_enum_and_the_bounded_number`, `next_n_units_start_tomorrow_and_leave_today_out_for_every_unit`) now expect `toDate(now('Asia/Jakarta'))` instead of `today()`, the first day from the setting, and `toDate32(toTimeZone(col, 'Asia/Jakarta'))` instead of `toDate(col)` (decision 8, the behaviour change). The other builder and store tests call shims in their test modules (`sql_with_filters`, `records_sql`, `spec_from_inline_sql`, ...) that pass the default context, so their bodies are unchanged.
- `lakehouse-bi` `store::tests::ensure_bi_table_runs_ddl_at_most_once_per_process`: 17 to 18 DDL statements (the `grain` column).
- `lakehouse-api` `routes::dashboard::typed_filters::typed_filters_narrow_a_mart_tile_and_a_sql_source_tile`: expected range text uses the report-zone today.
- `lakehouse-api` tests that call `get_body`, `records_for_roles`, `values_for_roles`, `preview_for_roles`, `render_board_payload`, `DashboardQuery { .. }` and `Board { .. }` literals gained the new argument or field.
- `tool_schemas.json` regenerated by the ignored writer test (+39 lines: `grain` on `create_chart` and `update_chart`, and the `describe_mart` description).
- `describe_mart` and `list_sql_sources` now also return `columns: [{name, kind}]` (additive).

**Deviations from the plan (for you to accept or reverse)**

1. The bucket is a typed `Date`/`DateTime`, not text (reason in the engine section: a text bucket merges a repeated local hour). A truncation still reaches the console as `YYYY-MM-DD` / `YYYY-MM-DD HH:MM:SS` through the JSON output.
2. Day of week is `1` (Monday) to `7` (Sunday) always; only the order moves with the first-day setting (`ORDER BY d % 7`). A part's label ("Mon") therefore does not depend on the setting.
3. `grain=own` is accepted by `GET /api/dashboard` (each chart keeps its own grain, saved default set aside). Without it a saved default could not be left for a session. Not in the plan's table.
4. Added to the dashboard payload: `grain` (saved), `appliedGrain`, `reporting {timeZone, weekStart}` (also in the embed and public payloads), and per tile `grain`, `grainColumn`, `grainSkipped`, `truncated`; `specs/preview` tiles carry `grain` / `truncated` too. The console needs them to label buckets, to offer hour and minute only when every grained chart is on a timestamp (decision 9) and to take the calendar's first day from the setting.
5. The zone picker is an `Input` with a native `datalist` (searchable by typing), not a popup `Combobox`: the popup could not be exercised in happy-dom. The server still checks the name.
6. T5 "builder tests for when the select appears": the rule (`grainChoices` / `grainFits`) is unit-tested in `time-grain.test.ts`, but there is **no render test of `ChartBuilder`** (a 1000-line dialog that needs services, theme and fetch); not verified in a browser.
7. The plan said the stored `spec.sql` is never served for a grained chart; it is still built at save (default or current settings zone) because `insert_chart` smoke-tests it and the builder's preview runs it, and the dashboard and embed always rebuild.
8. A grained chart whose column no longer fits (its type changed) is rebuilt without the grain and not reported on the tile (plan only specifies the switch case).
9. Records by bucket accepts truncations only (a part is a 400); a grain that does not fit the column is a 400, not an empty list.
10. A Rust lakehouse-alerts call site needed the new argument; it passes the default context (KPI/gauge tiles with no filters never read it).

**Plan vs code mismatches that need a planner decision**

- The AI-15 standard request set does not exist (above).
- `ChartInput.grain` is read from `def`; `render_stored_spec` does not copy it into the render spec (the console reads `def.grain`), as `click` is.
- `docs/core/specs/bi-9.md` and `ai-3.md` were left alone as instructed.

**What to check in a browser (none of it seen by me)**

1. Admin > Settings: as Platform Admin pick `Europe/Berlin` and Sunday, Save, reload; as a role without `settings:write` the controls are disabled and there is no Save; an unknown name typed in is refused with "Unknown time zone.".
2. Chart builder on a table with a `Date` column: "Group by" appears under the dimension with ten choices (no minute, hour, hour of day); with a timestamp column thirteen; with a text column it is absent. Pick Month on a line chart: Sort switches to "Natural", the limit allows 1000, the preview shows "Mar 2026" labels.
3. A calendar chart on a timestamp column: "Group by" shows Day only and cannot be cleared.
4. Day of week: seven bars `Mon` to `Sun`; set the first day to Sunday in Settings and reload without re-saving the chart: `Sun` first.
5. Dashboard with a day-grained line and a month-grained bar: a "Group by" control appears in the filter row; Month regroups both; "Each chart's own" restores them; a copied link keeps `?grain=`; an editor sees "Save as default" only when the choice differs from the saved one; reload as another user with `dashboard:write` removed: no Save button.
6. A chart on a plain date with hour chosen on the dashboard: the tile keeps its grouping and shows the small marker.
7. A grouped chart over more than 1000 hours: only the latest 1000, with the scissors marker.
8. Click "Mar 2026": View records lists only March (the count matches), "Filter dashboard by this" sets one 1 to 31 March range, a second click on the same bucket clears it; hour buckets list records but offer no filter; clicking a weekday shows "Nothing to open from this value", and the tile menu has "Why clicking does nothing".
9. A chart whose click opens another dashboard: clicking a month opens it with the date range; a click URL gets the bucket start text.
10. Change the report time zone and check a timestamp near midnight moves to the other day on an existing chart, and that "today" in a relative date filter follows the zone.
11. Embed and public link of a board with a saved grain: charts use the saved grain, no switch, month labels, the calendar's first day follows the setting.
12. The assistant: "Make this monthly." on a daily chart (needs the rebuilt API); `describe_mart` lists column kinds.

### BI-9 — fix handoff for the first review (developer, 2026-10-10)

Not committed. Findings cited at the fix sites as "BI-9 review fix (BLOCKER) R1" and "(SHOULD-FIX) R2".

- **R1** (`lakehouse-bi`): new `grain::cuts_to_latest(grain, order)` (truncation and bucket order only), `grain::effective_limit` and `PART_RANGE_LIMIT = 64`. A part ordered by bucket is built with `LIMIT 64` (more than 24, 7, 31, 54, 12, 4), is a plain ordered query with no latest-N wrapper and no `+1`, and never sets `latest_limit` or `truncated`. Truncations keep latest-N; a part with a value order keeps the user's limit. Applied in `QueryBuilder::build`, `grained()`, `GrainedChart::from_def`, the save-time SQL (`grained_chart_sql`) and `annotate_saved_grain`. Tests added: `grain` unit test for every part, and in `builder` `a_part_ordered_by_bucket_returns_its_whole_range_whatever_the_limit` (limits 1, 5, 20, 1000 on four parts) and `a_part_with_a_value_order_keeps_the_editors_top_n_and_a_truncation_keeps_latest_n`. One existing test changed: `day_of_week_is_ordered_from_the_configured_first_day` now expects `GROUP BY d ORDER BY d LIMIT 64` (Monday) and `ORDER BY d % 7 LIMIT 64` (Sunday) instead of the latest-N wrapper.
- **Real engine** (`serving.mart_demo_events`, 1440 hourly rows, read only), the generated shape `SELECT event_time, round(sum(orders)) AS orders FROM (SELECT toHour(toTimeZone(event_time, 'Asia/Jakarta')) AS event_time, orders FROM serving.mart_demo_events) AS bkt GROUP BY event_time ORDER BY event_time LIMIT 64`: 24 rows, hours 0 to 23. The same with `toDayOfMonth(...)`: 31 rows, days 1 to 31 (a direct `GROUP BY` gives 31 distinct days). Run by hand from the unit-tested text, not through the rebuilt API.
- **R2** (`chart-builder.tsx`, `lib/time-grain.ts`): `limitAfterGrainPick` lifts an untouched default limit (20) to 1000 when a truncation is chosen (a limit the person typed, or an edited chart's saved limit, is kept); `limitHasEffect` hides the limit field for a part ordered by bucket. Tests: two new in `time-grain.test.ts`; the builder dialog itself still has no render test.
- **Checks** (foreground, scoped): `cargo fmt --check` clean; `cargo clippy -p lakehouse-bi -p lakehouse-api --all-targets -- -D warnings` clean; `cargo test -p lakehouse-bi` 186 lib pass (+1); `cargo test -p lakehouse-api --lib` 1568 pass, 1 ignored; `--test route_auth` 32 and `--test sec11_guard` 4 pass; `bun run typecheck` clean; `bun run lint` 0 errors, 6 warnings; `bun test src/lib/time-grain.test.ts` 14 pass; full `bun run test`: 1140 pass, 1 skip, 0 fail (1141 tests, 127 files).
- *Not verified:* the rebuilt API against the engine after this change; the builder in a browser.

### BI-9 — fix handoff 2, for the QA findings (developer, 2026-10-10)

Console only, not committed; findings cited at the fix sites.

- **R3 (BLOCKER):** the builder's preview effect depended on a hand-written field list without `grain`. The list is now one pure function, `previewKey(...)` in `src/lib/preview-key.ts` (every input that reaches the SQL or render spec, `grain` included), and the effect depends on `[open, previewInputsKey, heldPreview, previewSql]`. I did **not** render the whole dialog (impractical); `src/lib/preview-key.test.ts` pins that changing the grain changes the key and that every input does, so a field dropped from the key fails it. Checked against the old list: no other field was missing (`limit` already covers the limit change from `limitTouched`; `click` is not part of the preview payload's effect and was not in the old list either). Not covered by a test: that the effect uses the key (a review of the diff).
- **R4 (SHOULD-FIX):** `grain-switch.tsx` lost its button and the props behind it. `dashboard-page.tsx` has one `rowDirty` (filters differ or the grain differs from the saved one), one `saveRowDefault` (one `PUT /api/dashboard/boards` body with `filters` and/or `grain`, only what differs; the route already applies both; one error path) and one `resetRow` (drops `f` and `grain` in a single navigation). The filter row now renders the filter bar whenever there are grouped charts, so the button exists with no filter fields; on the built-in Main board the bar's `canSaveDefault` is still false (Main cannot save filters), so a grouping cannot be saved there from the row as it could before. Tests: `grain-switch.test.tsx` now asserts no button of its own; the filter bar's own tests are unchanged. No component test of the combined save (the page is not rendered in tests).
- **R5 (SHOULD-FIX):** `chart-option.ts`: `MARKER_LIMIT = 60`, `showsPointMarkers(kind, points)`; a line with more than 60 category points sets `showSymbol: false` (single series and breakdown), hover unchanged (ECharts still shows the symbol under the axis pointer; not seen in a browser). Area already drew none. `chart-option.test.ts`: 60 true, 61 false, 1000 false, area, breakdown.
- **Commands:** `bun test` on the new and touched files: 23 pass; `bun run typecheck` clean; `bun run lint` 0 errors, 6 warnings; full `bun run test`: 1145 pass  1 skip  0 fail  2601 expect() calls Ran 1146 tests across 129 files. [109.47s]. No Rust touched. *Not verified:* the builder, the combined save and Reset, and the line markers in a browser.

### BI-9 — fix handoff 3, flaky settings test R6 (developer, 2026-10-10)

Console only, not committed, no cargo run. Cited as "BI-9 review fix R6".

- **Cause:** two tests read state before it existed (`.value` read once the input existed, before the load filled it), and the datalist of several hundred zones rendered slowly in happy-dom under load.
- **Fix:** `ReportingSettingsForm` takes an optional `zoneList` prop (default `listTimeZones`, so users see the same list); the tests pass a three-name list. Every test now waits for the loaded value (`waitFor` on the input's value, and on the first-day trigger text "Monday") before asserting or editing, instead of waiting for the element. No assertion removed (one added: the first day shown); no timeout raised.
- **Runs:** `bun test` on the file 3 pass; `bun run typecheck` clean; `bun run lint` 0 errors, 6 warnings. Full `bun run test` three times: 1145 pass, 1 skip, 0 fail (1146 tests, 129 files) each time. Not run while the machine was under the reviewer's cargo load, as far as I can tell.

## 8. Review (planner appends findings per PR)

### BI-9 — T1–T8, first review (reviewer, 2026-10-10)

Probed on the rebuilt dev API against the real engine (26.8), demo tables of 237 dated rows and 1440 hourly timestamp rows.

**Passed:** settings defaults, an unknown zone and a third week start refused (400), a save and its effect (day of week ordered from Sunday, weeks starting on Sunday then on Monday again); month totals equal the day-of-week totals (278 955); refusals for `hour` on a date column, a text column, an unknown grain, a map kind; the dashboard override (`month`: 15 buckets; `hour` on a date column: kept `day`, reported `grainSkipped`); a grain with a filter on another column, on the grained column, and both, each equal to a direct query; records for March 2026 (13 rows, total 13, equal to a direct count); a timestamp column grouped by day in `Asia/Jakarta` equal to a direct query; 1440 hours cut to the latest 1000 with `truncated: true`.

**Findings**

- `BLOCKER` R1 — A date part ordered by bucket is cut by the limit. `hour_of_day` with no limit sent returns 20 rows, hours 4 to 23: hours 0 to 3 are dropped by the latest-N rule. The same would drop days of a month and weeks of a year. A part ordered by bucket must return its whole range (24, 7, 31, 53 or 54, 12, 4) whatever limit was sent; latest-N is for truncations only. A part with a value order (top N) keeps the limit.
- `SHOULD-FIX` R2 — In the chart builder, choosing a truncation leaves the limit at the default 20, so a daily chart shows 20 days and a cut-off mark. When a grain is chosen and the limit is still the untouched default, set it to the grained maximum; for a part ordered by bucket, the limit field has no effect and should not be shown.

**Planner answers to the handoff's questions**

- Bucket as a typed date, day of week always 1 to 7 with only the order moving: accepted.
- `grain=own`, and `grain`, `appliedGrain`, `reporting`, `grainColumn` in the payload: accepted.
- Zone picker as an input with a datalist: accepted.
- The `AI-15` request set does not exist: dropped from T8. `AI-3-AC1` is checked by hand at QA; the entry is owed when `AI-15` is built.

### BI-9 — second review (reviewer, 2026-10-10)

R1 and R2 are closed. On the API rebuilt from the fixed tree, against the real engine: `hour_of_day` with no limit returns 24 rows (0 to 23), `day_of_month` with `limit: 5` returns 31, neither marked cut off; `hour_of_day` with a value order and `limit: 5` returns 5; an hourly truncation still cuts 1440 hours to the latest 1000 with `truncated: true`; the dashboard override to `month` regroups both grained charts. The dashboard page (with its "Group by" control) and Admin > Settings render in a headless browser with no error. No open `BLOCKER`. Not yet done: the reviewer's own re-run of the test suites on the final tree, and the product owner's QA; both precede the commit.

### BI-9 — findings from the product owner's QA (reviewer, 2026-10-10)

QA passed: the dashboard control (day, week, month and back), records for a clicked month (13 rows), Filter from a clicked month (`visit_date` in March 2026), an unknown zone refused, the cut-off mark, the picker hidden for a text column.

- `BLOCKER` R3 — Chart builder: the preview does not change when "Group by" changes. `grain` is missing from the dependency list of the preview effect in `chart-builder.tsx` (the list sits under an `eslint-disable`), so no new preview is requested. Add it, and add a test that would have caught it.
- `SHOULD-FIX` R4 — With a grain chosen and a filter set, the filter row shows two "Save as default" buttons. Show one: the filter bar's button saves the filters and the grain together, appears when either differs from the saved default, and Reset puts both back.
- `SHOULD-FIX` R5 — A line or area chart with many points (the hourly QA chart, 1000 points) draws a marker on every point and reads as a solid block. Draw no point markers above a threshold (planner default: 60 points); the hover tooltip still works.

### BI-9 — final review (reviewer, 2026-10-10)

No open `BLOCKER`. R1 to R6 are closed.

- R3, R4, R5: confirmed by the product owner in a browser (the builder preview follows "Group by"; one "Save as default" and one Reset for filters and grain; no point markers on the 1000-point line).
- `BLOCKER` R6, found by the reviewer's re-run — `reporting-settings-page.test.tsx` failed one test in each of three full runs (it read the input before the settings had loaded, and rendered several hundred zone options under load). Fixed in the tests and with an injectable zone list; closed after the developer's three full runs and the reviewer's one, all 0 fail.

**Re-run by the reviewer on the tree that is committed**

- `bun run typecheck`: clean. `bun run lint`: 0 errors, 6 warnings. `bun run test`: 1145 pass, 1 skip, 0 fail, 129 files.
- `cargo fmt --check`: clean. `cargo clippy -p lakehouse-bi -p lakehouse-store --all-targets -- -D warnings` and `-p lakehouse-api`: clean.
- `cargo test -p lakehouse-bi`: 186 + 1 pass. `cargo test -p lakehouse-api --lib`: 1568 pass, 1 ignored. `--test route_auth --test sec11_guard --test embed_tokens --test reporting_settings_route`: 32 + 4 + 21 + 6 pass. `cargo test -p lakehouse-store reporting_settings`: pass.
- The Rust runs predate the console-only fixes R3 to R6; no Rust file changed after them.
- *Not verified locally:* the workspace-wide clippy and test run and the other `lakehouse-api` integration files (CI runs them); `docker compose`; the gates.

**Product owner QA (2026-10-10), passed:** `BI-9-AC1`, `AC2` (both week starts), `AC5`, `AC6`, an unknown zone refused, the picker absent on a text column.

**Not seen by anyone in a browser:** `BI-9-AC3` as a user without `dashboard:write` or `settings:write` (route tests only); `AC4` (a zone change moving a timestamp across midnight; engine probe only); `AI-3-AC1` through the assistant; a grain on an embed or public link; a SQL-source chart with a grain; the calendar chart on a timestamp column.

**Owed**

- Decisions 6 to 9 on the feature page are still planner defaults; the owner has not said yes or no.
- The `AI-3` entry in the `AI-15` request set, when that set exists.
- Migration `0066`: `0062` to `0065` are taken by open PR #101, so the number is free only while that holds.
- The built-in Main board cannot save a grain default (it accepts layout changes only).
