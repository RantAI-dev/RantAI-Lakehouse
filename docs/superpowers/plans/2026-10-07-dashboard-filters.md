# Dashboard filters (BI-18 part A) — Implementation Plan

**Status:** decisions signed 2026-10-07, not started. Written by the planner
(Claude Opus) for a developer agent, under the role split in `AGENTS.md`.

**Feature page:** `docs/core/features/dashboard-filters.md`.

**Base commit:** `56b87d2` on `feat/uiux` (content identical to `main`
`980b29f`). Work on `feat/uiux`; the owner chose to stay on this branch.

**Goal:** dashboard filters at the level of Metabase and Tableau: typed
filters (date, number, text), on any column, with search and linked value
lists, values from SQL sources, and filter changes that no longer rewrite
the dashboard's default for everyone.

---

## 1. Decisions already made (do not re-ask, do not change)

| Decision | Choice |
| --- | --- |
| Filter state | Temporary, mirrored in the URL. The console stops saving filters on every change. |
| Saving | Explicit **Save as default** for callers with `dashboard:write`, on user dashboards only. **Reset** for everyone. |
| Built-in "Main" | Cannot save a default (unchanged). |
| Year | No special chip. `tahun` is an ordinary number column. The `year` query parameter stays accepted by the API. |
| Scope | Everything in the feature page ships in one PR, one task per commit. |
| Public and embed | Unchanged: saved default; embed token `params` stay locked. |
| Missing column | A filter is skipped for a tile whose relation lacks the column, and the tile says so. |
| Storage | No migration. Filters stay a JSON string on the board row; new fields are optional. |
| Time zone | Relative dates use the `ClickHouse` server clock. Stated as a limit, not solved. |

## 2. What exists today (anchors, verified at `56b87d2`)

- `rust/crates/lakehouse-bi/src/store.rs:401` `FilterDef { column, values }`; `parse_filters` (`:588`) reads the board's JSON and falls back to empty on a parse error.
- `rust/crates/lakehouse-bi/src/builder.rs:465` `filter_predicates(cols, years, filters)`: `tahun IN (..)` when the relation has `tahun`, and `col IN (..)` per filter via `Ident` + `SqlLiteral`; a filter applies only if the column is a valid identifier **and** in `cols`. Used by `sql_with_filters` (marts) and `sql_for_sql_source`.
- Column sets carry names only: `routes/support.rs:316` loads `system.columns` as `table, name`; SQL sources store `columns: [{name, type}]`.
- `rust/crates/lakehouse-api/src/routes/dashboard.rs`: `get` parses `year` and `filters` (`:105`); `filterColumns` is the distinct chart dimensions (`:197`); `values` / `values_for_roles` (`:972`) list `DISTINCT toString(col)` across every `serving` mart with that column, `LIMIT 200`, through `rewrite_sql_for_roles`.
- `routes/embed.rs:125` `params_to_filters`; `public_dashboard` renders with the board's saved filters only.
- Console: `src/features/dashboards/dashboard-filters.tsx` (bar, Year chip, value panel); `dashboard-page.tsx` `applyFilters` (`:176`) PUTs the board on every change; `crossFilter` (`:188`); the year state and `/api/dashboard/values?column=tahun` probe (`:86`, `:114`); page context `summarizeFilters` in `src/lib/page-context-summary.ts`; `FilterDef` type in `src/services/clients/bi-store.ts`.

## 3. Tasks

Each task is one commit. Rust tasks sit together (T1–T4) so the workspace
builds once per sitting.

### T1 — Typed `FilterDef` (lakehouse-bi)

Extend `FilterDef` with optional fields, `#[serde(default)]`, camelCase on
the wire, skipped when absent so a legacy filter round-trips byte-for-byte:

- `op`: `in` (default when absent) | `not_in` | `between` | `relative` | `contains` | `starts_with` | `ends_with`
- `values: Vec<String>` (for `in`, `not_in`; stays required-with-default)
- `min`, `max`: optional strings (`between`; number or ISO date `YYYY-MM-DD`; either may be absent)
- `unit`: `day|week|month|quarter|year`, `n`: integer 1..=3650, `anchor`: `last|this|previous` (`relative`)
- `text`: string, at most 200 chars (`contains`, `starts_with`, `ends_with`)

Add `FilterDef::validate()` returning our own messages: an op's required
fields present, bounds parse, `n` in range, at most 500 `values`, at most 20
filters per board (checked where the list is accepted).

*Accept:* unit tests: legacy JSON deserializes to `op = in` and serializes
back unchanged; each op validates and rejects malformed input; unknown `op`
is a deserialization error, not a silent `in`.

### T2 — Typed predicates (lakehouse-bi builder)

- Introduce a column kind: `Number | Date | DateTime | Text`, derived from a `ClickHouse` type string by a pure function (strip `Nullable(..)` and `LowCardinality(..)`; `Int*`/`UInt*`/`Float*`/`Decimal*` → Number; `Date`, `Date32` → Date; `DateTime*` → DateTime; everything else Text).
- `filter_predicates` takes columns with kinds (name → kind). Callers that only have names today must be changed to load types; do not guess a kind from a name.
- Predicates, all through existing `Ident` / `SqlLiteral`, never formatted from raw input:
  - `in` / `not_in`: as today (`NOT IN` for the latter).
  - `between` on Number: bounds parsed to a finite `f64` server-side and rendered as numbers; on Date/DateTime: bounds parsed as `YYYY-MM-DD` and rendered `toDate('…')`; inclusive of both ends, an absent end is open. A DateTime column compares on `toDate(col)`.
  - `relative`: only the enum and the bounded integer reach the SQL, as `ClickHouse` date functions over `today()`. `last n unit` = the n units ending today, inclusive; `this` = the current calendar period; `previous` = the one before it.
  - text ops: case-insensitive match (`positionCaseInsensitive`, or `ILIKE` with `%`, `_` and `\` escaped; pick one and test the escaping). Only on Text.
  - An op that does not fit the column's kind (for example `contains` on a number) is **skipped** for that relation and reported, not coerced.
- Return, alongside the clauses, which filters were applied and which were skipped (missing column or wrong kind), so the tile can say so.
- `years` keeps working exactly as now.

*Accept:* unit tests per op and kind, including: a text value containing `'`, `%`, `_`, `\`; an open-ended range; a date bound that is not a date is rejected at validation, never rendered; a filter on a column the relation lacks is in the skipped list; legacy filters produce the same SQL as before (the existing tests stay green unchanged).

### T3 — Dashboard payload and saving (lakehouse-api)

- Load column types where the dashboard loads column names (`support.rs`), for marts and SQL sources.
- `GET /api/dashboard`: validate `filters` from the query (400 with our message on a malformed filter, instead of silently ignoring a parse error); add `filterFields: [{ column, kind, tiles }]` — every column of every relation a chart on the board reads, with how many tiles have it; keep `filterColumns` as is for compatibility. Each chart result gains `filtersSkipped: [{ column, reason: "no_column" | "wrong_type" }]` when non-empty.
- `PUT /api/dashboard/boards` with `filters`: validate with T1 before storing.
- Public and embed paths compile against the new types and behave as before (add a test that a board saved with a typed filter renders publicly with it applied).
- No new route, no `POLICY_TABLE` change. `ClickHouse` error text never reaches a response.

*Accept:* route tests in the file's existing style for: typed filters applied to a mart tile and a SQL-source tile; a malformed `filters` query is 400; `filterFields` lists non-dimension columns with kinds; `filtersSkipped` reported.

### T4 — Values endpoint (lakehouse-api)

Extend `GET /api/dashboard/values` (same route, same policy):

- `q`: optional search text; case-insensitive substring on `toString(col)`, passed as a literal with wildcard escaping.
- `board`: optional; when present, values come from the relations the board's charts read that have the column, **including SQL sources** (each source SQL goes through the same `check_sql_source` + role rewrite as when a tile runs; reuse, do not write a second guard). Without `board`, behaviour is as today.
- `filters`: optional, validated; every other active filter is applied to each relation that has its column, so the list is narrowed (the filter on `column` itself is ignored).
- Response gains `truncated: bool` (fetch 201, return 200). Keep `values`.
- Everything still goes through `rewrite_sql_for_roles` for the caller's roles before reaching `ClickHouse`.

*Accept:* tests: search is applied and escaped; `truncated` true at 201 rows; a linked filter narrows; a SQL-source-only column returns values; a masked column's values are masked (extend the existing enforcement test).

### T5 — Console: filter model and URL state

- `FilterDef` type and client updated (`services` layering as usual).
- Pure module `src/lib/dashboard-filter-state.ts` (+ `node:test`): encode/decode filters to a URL parameter (`f`, compact JSON), normalise, compare with the saved default (order-insensitive), human-readable chip label per op (`province is Bali, Aceh`, `date in the last 30 days`, `price ≥ 100`), default op per column kind.
- `dashboard-page.tsx`: filters come from the URL when `f` is present, otherwise from the board's saved default; changing a filter updates the URL (`router.replace`, no history spam) and reloads tiles; **no PUT on change**. Switching dashboards drops `f`. Remove the year state, the `tahun` probe and the `year` prop chain in the console; drill and expand dialogs must keep working without it (check `drill.tsx`, `tile-body.tsx`, `TileExpandDialog`).
- `crossFilter` (click a value in a chart) goes through the same temporary path.

*Accept:* tests for encode/decode round trip, equality with default, labels; typecheck clean.

### T6 — Console: filter bar

Rewrite `dashboard-filters.tsx` (split into files under `src/features/dashboards/filters/` if it grows):

- "Add filter" lists `filterFields` with a kind icon and a search box; columns already filtered are not offered.
- One chip per filter with the label from T5; clicking opens an editor by kind:
  - Text: tabs "Values" (searchable, multi-select, "is" / "is not", shows "Showing the first 200, search to find more" when truncated) and "Text" (contains / starts with / ends with).
  - Number: range with optional ends, or a value list.
  - Date: "Relative" (presets: last 7, 30, 90 days, this month, this quarter, this year, previous month, previous year, and a custom n + unit) and "Range" (two date inputs).
- Value lists call the values endpoint with `board` and the other active filters (debounced search, abortable).
- **Save as default** shown when the caller has `dashboard:write` (`useAuth().hasPermission`), the board is not the built-in one, and the state differs from the default; it PUTs the board and then clears `f`. **Reset** shown when the state differs from the default. A failed save is reported in the page's existing error idiom.
- A tile with `filtersSkipped` shows a quiet marker with a tooltip naming the columns.
- Use the existing `@/components/ui` primitives (popover, tabs, input, calendar if one exists; a native date input is acceptable if not). No new dependency. Keyboard reachable; works in dark theme (no `text-primary` on dark).
- Update `summarizeFilters` and its test for the new ops and the removed year.

*Accept:* typecheck, lint, unit tests; component test for the chip label and the Save/Reset visibility rules if the repo's test setup supports it.

### T7 — Docs

`CHANGELOG.md` `[Unreleased]`: one entry a customer can read, including the
behaviour change (filters are no longer saved automatically) and the limits
from the feature page.

## 4. PR slicing

One PR, commits T1…T7 in order.

## 5. Out of scope (do not build)

Filter controls on public links and embeds; parameters and SQL variables;
time grain; per-tile filters; mapping a filter to differently named columns;
an assistant tool that sets filters; viewer time zones.

## 6. Things the developer must verify, not assume

- Which callers of `filter_predicates` / `sql_with_filters` / `sql_for_sql_source` exist (alerts, digests, embed, Copilot tools, drill-down) and that each still compiles and behaves the same for legacy filters.
- That board filters are only ever written through `PUT /api/dashboard/boards` (if another writer exists, it must validate too).
- The exact `ClickHouse` functions used for `relative` against the dev `ClickHouse` version, by running them.
- Whether a date-picker primitive exists in `src/components/ui` before adding markup for one.
- That removing the console's `year` does not change what the Copilot page context or the drill-down request sends in a way a test pins.

## 7. Handoff (developer appends one entry per PR)

### BI-18·A — T1–T7 (developer, 2026-10-07) (final; the partial notes above were written first and remain accurate)

Nothing is committed (owner's instruction: browser QA first). The session was
resumed twice after VM reboots; this partial entry is the state on disk.

**Inherited work audited (T1–T4).** The previous developer's Rust matched the
plan for T1, T2 and the T3/T4 code, but had NO tests for T3/T4 and one
compile error (`ValuesQuery` literal in the existing masking test). Fixed or
added by me:
- T1: `rust/crates/lakehouse-bi/src/filters.rs` (new; `FilterDef`, `validate`, `validate_filters`, `ColumnKind`, date/number parsing, unit tests incl. legacy round trip and unknown `op` error), `store.rs` (re-export), `lib.rs`.
- T2: `lakehouse-bi/src/builder.rs` (typed predicates, `FilterOutcome`, `*_report` fns, tests per op/kind), `sources.rs` (`column_kinds`), `lakehouse-alerts/src/lib.rs` (caller type change only).
- T3: `lakehouse-api/src/routes/support.rs` (`mart_columns` loads types, `stored_chart_sql` returns `FilteredSql`), `routes/dashboard.rs` (`parse_filters_param` -> 400, `filterFields`, `filtersSkipped`, `PUT boards` validation), `routes/embed.rs`.
- T4: `routes/dashboard.rs` (`q`, `board`, `filters`, `truncated`, source guard reuse via `check_sql_source`).
- Added by me: tests module `typed_filters` in `dashboard.rs` (11) and `embed.rs` (1), one masked-search test in `values_enforcement`; `render_board_payload` now takes `(ch, pg, ..)` instead of `&AppState` so it is testable (local adaptation); `defaultFilters` added to the `GET /api/dashboard` payload (DEVIATION: not in the plan; the console needs the saved default to offer Save/Reset while `f` overrides the filters in force); `ValuesQuery` derives `Default`; merged the two `impl SqlSource` blocks.

**Verified so far (foreground, `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target`):**
- `cargo test -p lakehouse-api --lib typed_filters` : 13 passed, 0 failed.
- `cargo test -p lakehouse-api --lib values_enforcement` : 2 passed, 0 failed.
- `cargo check -p lakehouse-bi -p lakehouse-api -p lakehouse-alerts --all-targets` was green before my later test edits.
- `relative` ClickHouse functions run against the dev ClickHouse 26.8.9.10: `subtractDays/Quarters`, `addYears`, `toStartOfWeek(x,1)`, `toStartOfQuarter`, `positionCaseInsensitiveUTF8`, `startsWith(lowerUTF8..)` all executed.
- NOT yet run: `cargo fmt --check`, clippy, `cargo test -p lakehouse-bi`, `-p lakehouse-alerts`, full `-p lakehouse-api --lib`, `--test route_auth`.

**FINDING (pre-existing, security-relevant, not fixed here).** Reported privately to the planner and handled through `SECURITY.md`; the detail is deliberately not recorded in this file.

**TypeScript so far (T5, T6 in progress):** `src/services/clients/bi-store.ts` (typed `FilterDef`, `FilterField`, `FilterSkip`), `src/lib/dashboard-filter-state.ts` + test (9 pass, `bun test src/lib/dashboard-filter-state.test.ts`), `src/lib/page-context-summary.ts` + test (`summarizeFilters` drops `year`; the old test pinned `year 2024` and was changed by design), new `src/features/dashboards/filters/{use-filter-values.ts,value-list.tsx,filter-editors.tsx,skipped-marker.tsx}`. Not yet written: filter bar, page rewiring, year removal from `TileBody` callers, CHANGELOG. Typecheck/lint/test not yet run.

**Final state (everything below supersedes "not yet run" above).**

Files per task (nothing committed; split by these lists):
- T1: `rust/crates/lakehouse-bi/src/{filters.rs (new),lib.rs,store.rs}`
- T2: `rust/crates/lakehouse-bi/src/{builder.rs,sources.rs}`, `rust/crates/lakehouse-alerts/src/lib.rs`
- T3: `rust/crates/lakehouse-api/src/routes/{support.rs,embed.rs,dashboard.rs}` (get payload, boards PUT validation, embed/public test)
- T4: `rust/crates/lakehouse-api/src/routes/dashboard.rs` (values endpoint and its tests)
- T5: `src/lib/dashboard-filter-state.ts` + `.test.ts`, `src/services/clients/bi-store.ts`, `src/features/dashboards/dashboard-page.tsx`, `tile-body.tsx`, `tile-dialogs.tsx`, `dashboard-preview.tsx`, `public-dashboard.tsx`, `embed-view.tsx`, `chart-builder.tsx`, `src/features/copilot/chart-draft-card.tsx` (year prop removed)
- T6: `src/features/dashboards/filters/{filter-bar.tsx,filter-editors.tsx,value-list.tsx,use-filter-values.ts,skipped-marker.tsx,filter-bar.test.tsx}` (new); `dashboard-filters.tsx` deleted; `src/lib/page-context-summary.ts` + `.test.ts`
- T7: `CHANGELOG.md`

Commands run in the foreground (`CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target`, `CARGO_BUILD_JOBS=2` for the last ones), all on the final tree:
- `cd rust && cargo fmt --check`: clean (after `cargo fmt`, which only touched my own hunks: every touched existing file was verified fmt-clean at HEAD).
- `cargo clippy -p lakehouse-bi -p lakehouse-api -p lakehouse-alerts --all-targets -- -D warnings`: clean.
- `cargo test -p lakehouse-bi`: 98 + 1 passed. `cargo test -p lakehouse-alerts`: 69 passed. `cargo test -p lakehouse-api --lib`: 962 passed, 0 failed, 1 ignored (pre-existing). `cargo test -p lakehouse-api --test route_auth`: 25 passed.
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 6 pre-existing warnings (none in files I touched). `bun run test`: 401 pass, 0 fail.
- `python3 ops/lint/check_intra_package_imports.py` and `check_bare_iceberg_count.py`: OK. `cargo test --workspace`, other `-p lakehouse-api --test` targets, `dagster` pytest, `docker compose config`: NOT run (not touched, or the brief scoped cargo).
- NOT verified: anything in a browser; any real ClickHouse run of the generated SQL (only the function behaviour was probed, and the SQL strings are asserted against a mock); `tests/parity.rs` (not run; the get payload gained `filterFields`, `defaultFilters`, `filtersSkipped`, which a parity corpus comparison could flag).

Deviations from the plan:
1. `defaultFilters` added to `GET /api/dashboard` (T3 did not list it). Needed so the console can compare with the saved default while `f` overrides `filters`.
2. `render_board_payload` takes `(ch, pg, ..)` instead of `&AppState` so the public/embed test needs no `AppState`.
3. `ValuesQuery` derives `Default`; `PUT boards` validates before the first write (so a bad list does not half-edit a board).
4. The page keeps calling `apiFetch` directly for the save and the values list, as the existing canvas does (the `DashboardService` contract documents the canvas as exempt); no new service method.
5. Date editor uses native `<input type="date">` and a native `<select>` (no calendar primitive exists in `src/components/ui`).
6. `BoardOpt`-level: the built-in Main board still ignores filters on its code-defined tiles (they only honour the year, which is now never sent by the console).

Existing tests changed: `page-context-summary.test.ts` "summarizeFilters ..." (pinned `year 2024`; the year is removed by design, now pins the new labels); `values_enforcement::filter_values_of_a_masked_column_are_masked` only gained `..ValuesQuery::default()` for the new fields. `store.rs`'s `parse_filters` test uses `FilterDef::in_values` (struct gained fields).

Inherited work I had to fix: the one compile error; no T3/T4 tests existed (added 13 + 1); formatting.

Process slip: I ran `git rm --cached` once on `dashboard-filters.tsx`, then undid it with `git reset -q HEAD -- <that path>`; the index is back to HEAD (nothing staged), the file is deleted in the working tree only.

**FINDING (pre-existing, security-relevant, not fixed here).** Reported privately to the planner and handled through `SECURITY.md`; the detail is deliberately not recorded in this file.

Look at closely in the browser: (a) opening a dashboard with `?f=` and without, and that Reset clears the address; (b) Save as default then the public link; (c) the Date editor presets and the custom n+unit; (d) value list for a SQL-source-only column, search, and the "first 200" note; (e) the tile marker (filter-x icon beside the drill hint) on a tile lacking a column; (f) popover width and dark theme; (g) chart-click cross-filter now going through the address; (h) a legacy dashboard with saved `{column, values}` filters and no Year chip.

### BI-18·A — fix for review BLOCKER 1 (developer, 2026-10-07)

**Fix.** `lakehouse-bi/src/builder.rs`: new `Relation::Filtered { base, predicates }`, rendered `(SELECT * FROM <base> WHERE <preds>) AS flt`; `FilterOutcome` gained `columns` (the columns the predicates read, `tahun` included); `rebuild` wraps ONLY when a predicate column equals an output alias of the kind (general/breakdown: the measures; point/geoheat: the measure; boxplot: measure and `__n`; KPI/gauge: `v`), otherwise the SQL is byte-for-byte what it was. `SETTINGS` and `point_limit` delegate to the base relation. `lakehouse-api/src/routes/dashboard.rs`: `values_select` takes the predicate columns and wraps when the searched or filtered column is `v` (the values output alias). No setting such as `prefer_column_name_to_alias` is used. Cited at the sites as `BI-18·A review BLOCKER 1`. The doc comment on `Filtered.predicates` says "joined with `AND`" (coordinator's `doc_markdown` edit kept).

**Tests.** lakehouse-bi: `a_filter_on_an_aliased_measure_is_applied_inside_the_relation` (exact SQL), `a_filter_on_another_column_keeps_the_plain_where` (no-collision, exact legacy SQL), `the_collision_wrap_covers_count_points_kpi_and_a_sql_source` (count alias, point kind, KPI with `v` collision and the KPI no-collision, SQL source incl. `SETTINGS` kept). Two earlier assertions in `the_report_carries_skipped_filters_...` changed from `WHERE jumlah >= 10` / `materials <= 5` to the wrapped shape, because the filtered column IS the measure alias there (new-feature tests, not legacy). lakehouse-api: `values_enforcement::a_tile_with_its_filter_in_an_inner_select_is_still_masked_and_row_filtered` (real policy row; the rewrite puts the mask and the row filter on the base table INSIDE the inner SELECT: `... FROM (SELECT * FROM (SELECT replaceRegexpOne(toString(`email`),..) AS `email`, `region`, `visitors` FROM serving.mart_x WHERE (region = 'north')) mart_x WHERE visitors >= 2000) AS flt ...`), and `typed_filters::a_values_filter_on_a_column_named_v_is_applied_inside_the_relation`. The test module had not imported `values_select` (E0425); fixed by importing it, tests kept.

**Real ClickHouse 26.8.9 (`docker exec lakehouse-clickhouse-1 clickhouse-client -q`), table `serving.mart_demo_map_points` (not referenced in code):**
- unwrapped `SELECT provinsi, round(sum(visitors)) AS visitors FROM T WHERE visitors >= 2 GROUP BY provinsi ...` : error 184 ILLEGAL_AGGREGATION (the bug, reproduced).
- wrapped bar `... FROM (SELECT * FROM T WHERE visitors >= 2) AS flt GROUP BY provinsi ... LIMIT 20` : 20 rows (direct `uniqExact(provinsi) WHERE visitors >= 2` = 26, so the LIMIT 20 caps it).
- wrapped `count() AS visitors` aggregate: 20 rows.
- wrapped point query (lat, lon, place, sum): 237 rows = direct `uniqExact(lat, lon, place)` 237.
- wrapped with a relative date (`visit_date > subtractDays(today(), 30) AND ...`) plus the measure filter: 10 rows.
- values shape `SELECT DISTINCT toString(provinsi) AS v FROM (SELECT * FROM T WHERE visitors >= 2) AS flt`: 26 values = direct 26.

### Rewriter literal round-trip fix (developer, 2026-10-10)

Closes the `SEC` review finding from BI-18·A (detail intentionally not recorded here; the repo is public). Nothing committed.

**Files.** `rust/crates/lakehouse-api/src/sql_rewrite.rs` (fix and unit tests), `rust/crates/lakehouse-api/src/routes/dashboard.rs` (route-level tests; the values-search test's comment and data updated).

**Root cause.** The rewriter parses with `ClickHouseDialect`, which decodes backslash escapes, then re-serialises with sqlparser's `Display`, which is not an encoder for that dialect: it never re-escapes a backslash and doubles a quote only when it judges it "not already escaped". The decoded value therefore did not survive. Fix at the render step: `substitute_governed_tables` re-encodes the single-quoted literals of the caller's parsed statement (`ReEncodeLiterals`, a `VisitorMut`, only for dialects that decode backslashes) into the form `Display` writes unchanged and ClickHouse decodes to the same value. Policy-authored expressions (parsed separately, inserted afterwards) are deliberately not re-encoded, so existing policy behaviour is unchanged. No filter value is stripped or rejected; no escaping happens downstream; no dependency change.

**Tests.** `sql_rewrite`: `a_literal_value_survives_the_rewrite_unchanged` (21 values: backslash alone, doubled, tripled, quote, doubled quote, each before/after/around a quote, mid and end of value, with `%` and `_`, non-ASCII, control characters, empty; asserts decoded literals equal the input, exactly one statement, same literal count, and that rewriting the output is a fixed point), `a_literal_value_survives_a_governed_rewrite_unchanged` (substitution path). `dashboard.rs`: `awkward_filter_values_survive_the_rewrite_on_every_tile` (filter path, in-list and contains, one GROUP BY, one `IN`), and the values-search test now uses a text with `%`, `_`, quote and backslashes (comment rewritten).

**Real ClickHouse 26.8 (read-only, `docker exec lakehouse-clickhouse-1 clickhouse-client -q`, `serving.mart_demo_map_points`, 237 rows):** four rewritten shapes (trailing backslash in an `IN`, backslash plus quote, search text form, full tile shape with GROUP BY) each returned `0` rows and no error; a decode check (`'a\\' = concat('a', char(92))`) returned `1` for the escapes used.

**Paths checked.** Every caller reaches the single rewriter `sql_rewrite::enforce` via `policy_engine::rewrite_sql_for_roles`: Query Studio (`routes/query.rs`), tiles and SQL sources (`routes/support.rs` `run_spec_sql`, `dashboard_sources.rs`), the dashboard and values endpoints (`routes/dashboard.rs`), alert evaluation (`routes/alerts.rs`), Gold export (`gold_export.rs`), the chat `run_sql` and `run_saved_query` tools (they delegate to the shared paths). `Parser::parse_sql` is used for rendering only in `sql_rewrite.rs`, so one fix covers them. Not separately route-tested: Query Studio, alerts, export (covered by the rewriter unit tests).

**Commands (CARGO_BUILD_JOBS=2, memory and disk checked first).** `cargo fmt --check` clean; `cargo clippy -p lakehouse-api -p lakehouse-bi -p lakehouse-alerts --all-targets --all-features --locked -- -D warnings` clean (crate sources touched first); `cargo test -p lakehouse-api --lib` 1504 passed, 0 failed, 1 ignored; `cargo test -p lakehouse-bi` 101 + 1 passed; `cargo test -p lakehouse-alerts` 69 passed; `--test route_auth` 30 passed; `--test security_regressions` 10 passed.

**Not verified.** Other integration targets (`parity`, `query_download`, connector targets) not run; the browser path; mask/row-filter expressions authored by admins containing backslashes (unchanged by design).

## 8. Review (planner appends findings per PR)

### BI-18·A — T1–T7 (reviewer, 2026-10-07)

Reviewed against the plan, then run against the dev `ClickHouse` (26.8) and
in a browser. The developer's tests asserted SQL against a mock only, which
is how the one blocker got through.

- **BLOCKER 1 (fixed).** A filter on a column the chart also aggregates
  (`sum(visitors) AS visitors` with `visitors >= 2000`) failed every tile
  with `ClickHouse` error 184: the alias shadowed the column in `WHERE`.
  Fixed by applying the predicates in an inner relation when, and only
  when, a predicate column equals an output alias. Re-verified by the
  reviewer on the rebuilt API: the filter returns 39 points in 15 provinces,
  equal to a direct count on the table.
- **No open BLOCKER.**

Verified by the reviewer on the final tree:

| Command | Result |
| --- | --- |
| `bun run typecheck` | clean |
| `bun run lint` | 0 errors, 6 warnings (all on `main`) |
| `bun run test` | 401 pass, 0 fail |
| `cargo fmt --check` | clean |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | clean |
| `cargo test -p lakehouse-bi` | 101 + 1 passed |
| `cargo test -p lakehouse-alerts` | 69 passed |
| `cargo test -p lakehouse-api --lib` | 964 passed, 1 ignored |
| `cargo test -p lakehouse-api --test route_auth` | 25 passed |

*Not verified:* `cargo test --workspace`. It needs about 26G of disk for the
test binaries and the shared dev machine had 30G free and has been
rebooting under cargo load; CI runs it on the PR.

Against the dev `ClickHouse`, by request to the running API: `contains`,
`not_in`, date `between`, `relative` (last, this, previous), number
`between` on a plain and on an aggregated column, legacy `in`, `wrong_type`
and `no_column` skips, a malformed filter (400), values search and linked
narrowing. In a browser (dark theme): the column list, the date and text
editors, a relative preset applied, the chip, the URL state, Reset and Save
as default appearing. The product owner ran the five QA groups on
2026-10-07 and reported them passing.

Carried forward, not part of this change:

- The developer reported a weakness in how `rewrite_sql_for_roles`
  re-renders string literals. It predates this work and affects existing
  dashboard filters. It is being handled through `SECURITY.md`; no detail
  is recorded here on purpose.
- Tile errors still carry `ClickHouse` text (`SEC-11`).
- The reviewer changed one doc comment in `builder.rs` for `doc_markdown`.

### Rewriter literal round-trip fix (reviewer, 2026-10-10)

The owner decided on 2026-10-10 to fix the carried-forward finding in this
PR. Reviewed the change in `sql_rewrite.rs`: literals the caller wrote are
re-encoded before the statement is re-serialised, nothing is stripped or
rejected, and no second escaping step was added downstream.

- **No open BLOCKER.**
- **SHOULD-FIX, not done here:** admin-authored mask and row-filter
  expressions are inserted after the re-encoding and are not covered by
  it. They are written by a privileged role, so this is a correctness
  follow-up, not the finding itself. Needs a backlog item.

Verified by the reviewer after the merge of `main` and this fix:
`cargo fmt --check` clean; `cargo test -p lakehouse-api --lib` 1504 passed,
1 ignored. On the rebuilt dev API against the dev `ClickHouse`, through
`GET /api/dashboard`: values containing a quote, a backslash, or both match
no row and return no error on all six tiles; such a value listed beside a
real one returns exactly the real value's rows (1, 6, 6, 1, 6, 1, the same
as the control); the ordinary filters return what they did before. The
values search with such text returns an empty list and no error.

*Not verified:* `cargo test --workspace`, and the `parity`,
`query_download` and connector test targets; CI runs them on the PR.

---

# Part A, second round (2026-10-10)

**Why a second round.** After the first round the roadmap gained a Target
column and `docs/core/specs/bi-18.md`. Against that target part A lacked:
specific-date, before/after and "next N" date filters, month-year and
quarter-year pickers, named equal / not-equal / greater / less number
filters, "does not contain", and required filters. This round adds them.
Two target rows stay out, with the reason on the feature page: filters
through `QS-5` parameters, and embed locking beyond signed `params`
(`BI-26`).

**Base:** `feat/uiux` at `d0dd5f3` (phase 0 is merged in: read
`upstream_error.rs` and `tests/sec11_guard.rs`; no response may carry
upstream text, and a new message of ours built with `{err}` needs a
reasoned guard entry or, better, no `{err}`).

## R1. Decisions already made (do not re-ask, do not change)

| Decision | Choice |
| --- | --- |
| Wire format | Additive only. Every filter stored or linked before this round keeps its meaning and its exact serialised form. New behaviour is new values of existing fields or new optional fields. |
| Relative "next" | `anchor: "next"` with `unit` and `n`: the `n` units starting tomorrow, today excluded. "last" is unchanged (includes today). |
| Text | New op `not_contains`. A NULL or empty value is kept by it (it does not contain the text). Case-insensitive, same escaping as `contains`. |
| Date and number comparisons | No new server ops: "on", "before", "after", "equal", "greater than", "less than", a month and a quarter are all a `between` with one or two ends, and "not equal" is `not_in` with one value. The console names them; the server already evaluates them. "Before" and "after" exclude the named date; "greater than" and "less than" exclude the number. If `between` cannot express an exclusive end exactly (it is inclusive today), add optional `minExclusive` / `maxExclusive` booleans, default false, skipped when false. Do not approximate with ±1. |
| Required | Optional `required: bool` on a filter, default false, skipped when false, meaningful only on a board's saved default. The server keeps it; the console enforces it: the chip has no remove control, clearing restores the default's value, a URL (`?f=`) that omits a required column gets the default's filter for that column added. Public and embed views already use the saved default and are unaffected. |
| Labels | The chip reads what the person picked ("on 3 Oct 2026", "before …", "in March 2026", "in Q1 2026", "= 5", "≠ 5", "> 5", "next 7 days"), derived from the stored filter by a pure function, with no extra stored hint except where two picks would serialise identically; then prefer the more specific reading (a full calendar month reads as the month). |

## R2. Tasks

One commit per task; Rust first.

### RT1 — Server: `next`, `not_contains`, exclusive ends, `required`

`lakehouse-bi` `filters.rs` and `builder.rs`, and the validation in
`routes/dashboard.rs` where filters are accepted. Follow R1. `relative`
with `anchor: next` uses the same date functions as `last`, mirrored.

*Accept:* unit tests: every pre-existing filter test passes unchanged; a
filter from round one serialises byte-for-byte as before; `next n unit`
boundaries for each unit; `not_contains` with `%`, `_`, `'`, `\\` and
its NULL behaviour; exclusive ends on number and date, each side; `required`
round-trips and is ignored by predicate building. Because mock-only tests
missed a real-engine failure in round one: run each new predicate shape
once against the dev `ClickHouse` (`docker exec lakehouse-clickhouse-1
clickhouse-client -q "…"`, read-only, over `serving.mart_demo_map_points`,
which has `visit_date Date`, `visitors UInt32`, `provinsi String`) and
quote the commands and row counts in the handoff. Do not reference that
table in code or tests.

### RT2 — Console: the named comparisons

`src/lib/dashboard-filter-state.ts` (+ tests): the pure mapping between
what the editor offers and the stored filter, both ways, and the labels.
`src/features/dashboards/filters/filter-editors.tsx`: date editor gains
"On", "Before", "After", "Next N" (beside "Last N"), a month picker and a
quarter picker (year + month, year + quarter; native controls or the
existing select, no new dependency); number editor offers equal, not
equal, greater than, less than, between, and the value list; text editor
adds "does not contain". Keep the editors compact: the owner asked for
less text in dialogs on 2026-10-10 (one line of help at most, controls
first).

*Accept:* round-trip tests for every new pick (pick → stored → label →
editor state); typecheck, lint, tests.

### RT3 — Console: required

A "Required" toggle on a filter chip's editor, shown to a caller with
`dashboard:write` on a user dashboard; it takes effect with "Save as
default". Enforcement as in R1, in the pure state module with tests (URL
without the column, clear, Reset, a default that later loses its required
flag).

*Accept:* unit tests for each enforcement path; the bar's test for the
missing remove control.

### RT4 — Docs

`CHANGELOG.md` `[Unreleased]`: extend the dashboard-filters entry.

## R3. Out of scope

`QS-5` parameters; embed-locked filters; filter widgets other than the
existing bar; time grain.

## R4. Things the developer must verify, not assume

- That a `?f=` link and a saved board from round one decode to the same filters and labels as before (add a fixture taken from a real round-one value).
- The exact inclusive/exclusive behaviour of the existing `between` on dates and on `DateTime` columns before building "before"/"after" on it.
- What "this quarter"/"next quarter" return on the dev engine at a quarter boundary date (compute with a fixed date expression, not `today()`).


### Part A, second round — partial notes

**RT1 (developer, 2026-10-10), on disk, not committed.** Files: `rust/crates/lakehouse-bi/src/{filters.rs,builder.rs}`, `rust/crates/lakehouse-api/src/routes/dashboard.rs` (only the existing malformed-filters test gained cases; validation itself is `validate_filters`, unchanged). Added `FilterOp::NotContains`, `RelativeAnchor::Next`, `minExclusive` / `maxExclusive` / `required` (all skipped on the wire when false/absent). Exclusive ends did need the booleans: numbers cannot be expressed by `between` otherwise, and dates must not be approximated with a day offset. `cargo test -p lakehouse-bi`: 126 + 1 passed; clippy for bi+api clean. Real-ClickHouse runs and the full verification block come in the final entry.

**RT2 (developer, 2026-10-10), on disk, not committed.** Files: `src/services/clients/bi-store.ts` (types), `src/lib/dashboard-filter-state.ts` + `.test.ts` (pick mappings `datePickToFilter` / `filterToDatePick` / `numberPickToFilter` / `filterToNumberPick`, labels, `filterLabel(f, kind?)`), `src/lib/page-context-summary.ts` (map callback made explicit because `filterLabel` gained a second parameter), `src/features/dashboards/filters/filter-editors.tsx`, `filter-bar.tsx` (passes the column kind to the label), new `filter-editors.test.tsx`. `bun run typecheck` clean; `bun run lint` 0 errors, 6 pre-existing warnings; `bun run test` 1025 pass, 1 skip, 0 fail.

**RT3 (developer, 2026-10-10), on disk, not committed.** Files: `src/lib/dashboard-filter-state.ts` + `.test.ts` (`isRequiredColumn`, `enforceRequired`), `src/features/dashboards/filters/filter-bar.tsx` (+ `filter-bar.test.tsx`: `savedFilters` prop, no remove control for a required column, "Required" switch in the chip editor for a caller who may save), `src/features/dashboards/dashboard-page.tsx` (enforcement on every change, and an effect that writes the required column into an address that lacks it). What counts is the saved default's flag; the working state's flag is only what Save writes. Tests: 32 pass across the three touched files; typecheck clean.

### BI-18·A second round — RT1–RT4 (developer, 2026-10-10)

Nothing is committed (owner's instruction: browser QA first). Files per task:
- RT1: `rust/crates/lakehouse-bi/src/{filters.rs,builder.rs}`; `rust/crates/lakehouse-api/src/routes/dashboard.rs` (test cases only).
- RT2: `src/services/clients/bi-store.ts`, `src/lib/dashboard-filter-state.ts` + `.test.ts`, `src/lib/page-context-summary.ts`, `src/features/dashboards/filters/{filter-editors.tsx,filter-bar.tsx}`, new `filter-editors.test.tsx`.
- RT3: `src/lib/dashboard-filter-state.ts` + `.test.ts`, `src/features/dashboards/filters/{filter-bar.tsx,filter-bar.test.tsx}`, `src/features/dashboards/dashboard-page.tsx`.
- RT4: `CHANGELOG.md`.

**Exclusive ends did need the booleans.** Number "greater than / less than" cannot be written with the inclusive `between` without a ±epsilon, and "before / after" on a date must not be a ±1 day approximation (a `DateTime` column compares on `toDate(col)`, so ±1 would also be wrong there). `minExclusive` / `maxExclusive` are skipped on the wire when false and rejected by `validate` when set without their bound. `required` is skipped when false and ignored by predicate building (test). A round-one filter serialises byte-for-byte as before (test with a real round-one string).

**Real ClickHouse 26.8 (`docker exec lakehouse-clickhouse-1 clickhouse-client -q`, read-only), `serving.mart_demo_map_points` (237 rows, `visit_date` 2025-09-03..2026-10-07, `visitors` 44..2377):**
- Existing `between` is inclusive on both ends, on `Date` and on a DateTime-by-day expression: for the max date 2026-10-07, `=`/`>=`/`>`/`<=`/`<` counted 1/1/0/237/236; through `toDate(toDateTime(visit_date))` `>=`/`>` counted 1/0.
- Exclusive date: `visit_date > toDate32('2026-10-03')` 2 rows (after); `< toDate32(...)` 235 (before).
- Exclusive number (min 44, max 2377): `countIf(visitors >= 2377)` 1, `> 2377` 0, `<= 44` 1, `< 44` 0; the tile shape `SELECT provinsi, round(sum(visitors)) AS s FROM T WHERE visitors > 44 GROUP BY provinsi` returned 26 rows; `visitors > 5` 237 and `(visitors >= 1 AND visitors < 5)` 0 rows.
- `not_contains` as generated, `ifNull(positionCaseInsensitiveUTF8(toString(provinsi), 'bali') = 0, 1)`: 231 rows (6 contain "bali", 231 + 6 = 237). NULL behaviour on a 4-row set (NULL, '', 'xBALIx', 'a'): with `ifNull` 3 rows kept; without it 2 (NULL dropped), which is why `ifNull` is there. The needle with `%`, `_`, `'` and backslashes ran without error (237 rows kept, none contain it).
- `next`: the table's dates are all in the past, so against `today()` every shape (`d > today() AND d <= addDays/Weeks/Months/Quarters/Years(today(), 2)`) returned 0 rows with no error. Boundaries were checked with a fixed date instead: with `t = 2026-09-30` and ten fixed dates, `d > t AND d <= addDays(t,7)` 2, `addWeeks(t,1)` 2, `addMonths(t,1)` 4, `addQuarters(t,1)` 6, `addYears(t,1)` 8, each equal to the hand count.
- Quarter boundaries (fixed expressions): `toStartOfQuarter('2026-09-30')` = 2026-07-01, of `2026-10-01` = 2026-10-01; `addQuarters(toStartOfQuarter('2026-10-01'),1)` = 2027-01-01; `subtractQuarters(toStartOfQuarter('2026-09-30'),1)` = 2026-04-01 (so "this quarter" and "previous quarter" are calendar quarters on both sides of the boundary). FINDING: `next 1 quarter` from 2026-09-30 is 2026-10-01..2026-12-30 (`addQuarters('2026-09-30',1)` = 2026-12-30), not the calendar quarter, because "next N units" is rolling by decision R1; the CHANGELOG states this for months.
- Not run against a real `DateTime` column (the table has none); the `toDate(col)` shape was exercised through `toDate(toDateTime(visit_date))`.

**Commands (CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target, CARGO_BUILD_JOBS=2, `free -g` and `df -h /` checked before each; disk 18G free, memory 8-9G available):**
- `cd rust && cargo fmt --check`: clean (I ran `cargo fmt` once; it changed only my hunks in the two bi files).
- `cargo clippy -p lakehouse-bi -p lakehouse-api --all-targets --all-features --locked -- -D warnings` (crate `lib.rs` touched first): clean (also checked `lakehouse-alerts`).
- `cargo test -p lakehouse-bi`: 126 + 1 passed (was 101 + 1 plus the 25 new). `cargo test -p lakehouse-api --lib`: 1542 passed, 0 failed, 1 ignored. `cargo test -p lakehouse-api --test sec11_guard --test route_auth`: 4 + 32 passed. `cargo test -p lakehouse-alerts`: 70 passed.
- Test executables over 20M deleted from `debug/deps` afterwards (7 files); `lakehouse_api-*` left alone.
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 6 pre-existing warnings. `bun run test`: 1033 pass, 1 skip, 0 fail.

**Existing tests changed:** none weakened. In `dashboard.rs`, `a_malformed_filters_query_is_a_400...` gained cases (it already covered the same function). `filter-bar.test.tsx` gained a `savedFilters` prop in its helper and two cases.

**Deviations and decisions to confirm:**
1. `filterLabel(f, kind?)` takes the column kind: "≠ 5" is shown only for a number column, so a text filter "is not 5" keeps its round-one label (R4). Ranges keep their ISO labels (`d from 2024-01-01 to 2024-02-01`); only "on / before / after" read as `3 Oct 2026`. A round-one number range with equal ends now reads `= 5`, and a full calendar month or quarter range now reads as the month or quarter (R1's "more specific reading").
2. A number `not_in` with one numeric value opens in the Compare tab as "not equal to" (not the Values list); the two picks serialise identically and no hint is stored.
3. Number editor tabs are now Compare / Values (the old Range tab is Compare's "between"); date editor tabs are Relative / Date (the old Range tab is Date's "Between"). Month and quarter use a native select plus a year number input (the year starts empty, no clock read). Native `<select>`/`<input type=date>`, no new dependency.
4. Required: what counts is the saved default's flag (a default that loses it stops enforcing at once); the working state's flag is only what "Save as default" writes. The page also runs an effect that writes a missing required column into the address (it reloads), because the first load happens before the default is known. The server ignores the flag.
5. `required` is carried over when an editor replaces a filter; the "Required" switch is in the chip's popover for callers who can save the default on user dashboards.

**Not verified:** anything in a browser; `cargo test --workspace`, other `lakehouse-api` integration targets (`parity` etc.); the "Required" switch itself (no component test), and the address-rewrite effect in the page (covered only through the pure `enforceRequired` tests); a real `DateTime` column.

### BI-18·A second round — review round 1 (developer, 2026-10-10)

Nothing committed. **BLOCKER 1 (inert saved filters make their column unreachable): fixed.**

Cause: the new bar listed a column as "used" when any filter named it, active or not, and the pre-BI-18 bar had stored `{column, values: []}`.

Console: `dropInertFilters` in `src/lib/dashboard-filter-state.ts` (keeps exactly the filters `isActiveFilter` accepts), applied to the board's `defaultFilters` and to every decoded `?f=` (in `decodeFilters`); `dashboard-page.tsx` derives the default, the working filters and the required-column enforcement from the cleaned default; `filter-bar.tsx` counts a column as used only for active filters (defence). A required placeholder with no value is dropped with the rest: it cannot be saved as required, so a stored one is treated as not required (comment at the function). Server: `lakehouse_bi::filters::without_inert`, used in `PUT /api/dashboard/boards` after validation and before the write; a bad list is still a 400, inert entries are not rejected, `GET` returns what is stored.

Other consumers checked: public and embed (server builder and `embed.rs` already use `is_active`), `summarizeFilters` (already filters active), the "N tiles" count (comes from `filterFields`, not filters), cross-filter (`toggleValue` appends to an inert `in` of the column, which is correct). Nothing else described an inert filter as active.

Tests: state module (the exact payload yields `[]`, equals no filters, is not dirty, decodes to `[]`; mixed ops; required placeholder), bar (no chip for the two placeholder filters and both columns offered in Add filter), `without_inert` unit test in `lakehouse-bi`. The PUT handler itself is not route-tested (it needs a ClickHouse store); only the pure function is.

Commands: `cargo fmt --check` clean; `cargo clippy -p lakehouse-bi -p lakehouse-api --all-targets --all-features --locked -- -D warnings` clean; `cargo test -p lakehouse-bi` 127 + 1; `cargo test -p lakehouse-api --lib` 1542 passed, 1 ignored; `--test sec11_guard --test route_auth` 4 + 32; `bun run typecheck` clean; `bun run lint` 0 errors, 6 old warnings; `bun run test` 1037 pass, 1 skip, 0 fail. Test executables >20M removed from `debug/deps`. Not verified: browser.

### BI-18·A second round — review round 2 (developer, 2026-10-10)

Nothing committed; frontend only, no Rust touched.

**SHOULD-FIX 1.** The editor opened from "Add filter" now shows the same "Required" row (shared `RequiredRow` in `src/features/dashboards/filters/filter-bar.tsx`), under the same rule (`canSaveDefault`: `dashboard:write` on a user dashboard). One Apply adds the filter with `required: true`; the switch resets when the popover closes.

**SHOULD-FIX 2.** `RequiredRow` carries a focusable `CircleHelp` button inside the repo's `Tooltip` (the root `TooltipProvider` in `app/layout.tsx` is used, none added). Its `aria-label` and the tooltip are the owner's text: "Viewers can change this filter's value but cannot remove it. Takes effect after Save as default." No visible paragraph. Same row in both places.

**Removal rule unchanged:** only the saved default's flag removes the remove control. Tests in `filter-bar.test.tsx` (stateful harness around `FilterBar`): the add editor shows the switch to a caller who can save the default and the help is reachable by label; not shown to one who cannot; add with Required on gives `price ≥ 5` with its remove control still present, and after "Save as default" the control is gone. Not verified: the tooltip popup opening on hover or focus in a browser (tests reach it by label only).

Commands: `bun run typecheck` clean; `bun run lint` 0 errors, 6 old warnings; `bun run test` 1040 pass, 1 skip, 0 fail.

### BI-18·A second round — RT1–RT4 and two review rounds (reviewer, 2026-10-10)

- **Server: no finding.** On a rebuilt API against the dev `ClickHouse`
  each new shape matched a direct count on the table: a date after
  2026-06-30 with an exclusive end 56, on one date 1, `visitors > 2000`
  exclusive 38 (the inclusive form gave 39 in round one), `< 100` exclusive
  6, `not_contains` 201, "next 7 days" 8 once the demo table had future
  dates, `required` accepted, a round-one filter unchanged, an exclusive
  flag without its bound 400.
- **BLOCKER 1 (round 1, fixed).** A board saved by the pre-BI-18 bar held
  `{column, values: []}` for two columns. The new bar drew no chip for
  them and left both out of "Add filter", so they could not be filtered at
  all. Inert filters are now dropped wherever filters enter the console's
  state, and before a board's filters are stored. Seen fixed in a browser.
- **Round 2, from the owner's QA (done).** The Required switch is also in
  the "Add filter" editor, and a help tooltip says what it does; the owner
  had asked what the feature was for.
- **Accepted deviations.** Labels take the column kind ("≠ 5" only on a
  number column); a number range with equal ends now reads "= 5" and a full
  calendar month or quarter reads as such; "next N months" is rolling from
  today, not calendar months, and the calendar reading is the Month and
  Quarter pickers.
- **No open BLOCKER.**

Verified by the reviewer on the final tree: `bun run typecheck` clean,
`bun run lint` 0 errors, `bun run test` 1040 pass, 1 skip (on `main`);
`cargo fmt --check` clean; `cargo clippy -p lakehouse-bi -p lakehouse-api
--all-targets --all-features --locked -- -D warnings` clean;
`lakehouse-bi` 127, `lakehouse-api --lib` 1542 (1 ignored), `route_auth`
32, `sec11_guard` 4. Rust was not changed after those runs. The product
owner ran the five QA groups and the Required flow on 2026-10-10.

*Not verified:* a real `DateTime` column (the demo table has only a
`Date`); the `PUT` that drops inert filters on a running system (unit
test only); the light theme; `cargo test --workspace`.

Part A against `docs/core/specs/bi-18.md`: met, except filters through
`QS-5` parameters and embed locking beyond signed `params` (`BI-26`), both
left out on the feature page with the reason.

