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

## 8. Review (planner appends findings per PR)
