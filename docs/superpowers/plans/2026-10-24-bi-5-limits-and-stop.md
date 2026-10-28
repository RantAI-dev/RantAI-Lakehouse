# BI-5: limits an administrator sets, and a Stop that stops (with a new AI item)

Planner and reviewer: Claude Opus. Developer: a non-Opus coding agent.
Branch: `feat/bi-phase-5` (to be cut; shared with `BI-4`), base
`feat/bi-phase-4`. Built after `BI-4` part A (T1–T2) and before `BI-4` part B,
whose cache key includes the limits introduced here. Feature page:
`docs/core/features/query-limits-and-stop.md` (decision 1 waits for the
owner, Q2; 2 to 7 planner defaults confirmed at the phase 5 QA). Spec:
`docs/core/specs/bi-5.md`.

Anchors were read on 2026-10-23 on `feat/bi-phase-4` at `150a081a`. Find
each by the symbol named.

## 1. Decisions already made (do not re-ask, do not change)

The feature page's table, plus these build decisions.

- **D1 Settings.** Postgres (migration: next free number at build time;
  check every branch; `0098`+ are being taken by `BI-19`), with a
  why-header: a one-row `query_limits` table (`timeout_seconds`,
  `chart_rows`, `map_points`, `studio_rows`, each `NOT NULL` with the
  feature defaults and a `CHECK` on the range) and `query_limits_role
  (role_id UUID PRIMARY KEY REFERENCES role ON DELETE CASCADE,
  timeout_seconds INT NOT NULL CHECK ...)`. Store module
  `lakehouse-store/src/query_limits.rs`. Routes `GET /api/settings/limits`
  (`RequiresAuth`: answers the install's values and the caller's
  effective ones) and `PUT /api/settings/limits` (`settings:write`), both
  in `POLICY_TABLE` and asserted both ways in `tests/route_auth.rs`.
- **D2 `TileLimits`.** One struct, resolved once per request from the
  settings and the principal's roles (`effective_for(principal)`: longest
  role timeout, else install): `{ timeout, chart_rows, map_points,
  studio_rows, source: Install | Role(name) }`. Settings are read through
  a 30-second in-process memo so a dashboard does not read Postgres per
  tile; a `PUT` clears it in this process (other processes see the change
  within 30 s; say so in the module doc).
- **D3 Engine-side limits, request settings.** Tile and Query Studio
  queries send `max_execution_time = timeout`, `max_result_rows` (the
  kind's cap + 1, `result_overflow_mode = break`), `query_id`, and
  `cancel_http_readonly_queries_on_client_close = 1` as **URL settings**
  through a new `ChClient::query_with_options(sql, &QueryOptions, cancel)`
  (`query_limited` becomes a call to it; one body, rule 4), with a
  per-request reqwest `timeout(limit + 10 s)` replacing the client's fixed
  60 s for these calls only. `SQL_SOURCE_SETTINGS` keeps its text form
  (SEC-21: a statement-level `SETTINGS` overrides URL settings, so the SQL
  source's own cap must stay in the text), but its numbers come from
  `TileLimits` through `Relation::settings(limits)` instead of the
  constant; `SQL_SOURCE_MAX_ROWS`, `point_limit` and their pinning test
  follow. `with_export_settings` (BI-1) is unchanged.
- **D4 Route deadline.** `timeout_middleware` becomes
  `from_fn_with_state`: for `/api/dashboard`, `/api/embed/*`,
  `/api/public/dashboard/*`, `/api/render/data` and `/api/query/run` the
  deadline is the **install's longest configured** timeout (install or any
  role, from the memo) + 15 s; every other route keeps `route_timeout`.
  The table in `route_timeout`'s doc gains the rows. A `BI-4` part A
  dashboard runs tiles 6 at a time, so a board can need more than one
  tile's limit: the dashboard deadline is `ceil(tiles / 6) × limit + 15 s`
  capped at 35 minutes, computed in the handler, which answers the tiles
  done and marks the rest "not reached in time" rather than a bare 408
  (principle 2).
- **D5 Query ids and Stop.** Every query D3 sends gets `query_id =
  "<principal uuid>:<run id>:<n>"`; the console sends a run id (UUID v4,
  header `X-Run-Id`; absent → the API makes one). `POST /api/query/cancel
  { runId }` (`RequiresAuth`, both ways in `route_auth`) runs `KILL QUERY
  WHERE query_id LIKE '<own principal uuid>:<runId>:%' ASYNC` (built with
  `SqlLiteral`, the run id checked as a UUID before use; the principal part
  always the caller's, so nobody can stop another person's query) and
  answers `{ stopped: n }` from a `system.processes` count before the kill.
  Reuse `routes/ops.rs` `kill_query_sql` by generalising it to a
  `KillTarget` (exact id or own-prefix), not a second builder (rule 4).
  `/api/ops/workloads/{id}/cancel` is unchanged.
- **D6 Messages.** `ChError` classification: ClickHouse `TIMEOUT_EXCEEDED`
  (code 159) and `QUERY_WAS_CANCELLED` (394) map to fixed sentences that
  name the limit and its source (`TileLimits::source`); the engine's text
  never reaches the response (SEC-11, `upstream_error`). A tile result
  carries `limit: { kind: "time"|"rows", value, source }` so the console
  writes the sentence.
- **D7 Assistant.** `get_query_limits` (ReadOnly), `stop_query { runId? }`
  (stops the person's own runs started from the chat; ReadOnly risk class
  because it only ends the person's own work: say so in the registry
  comment), prompt line, `capabilities.ts`, fixture regenerated with the
  ignored writer test, count pin +2, one `AI-15` case.

## 2. Anchors (verified at `150a081a`, find by symbol)

- Engine client: `lakehouse-clickhouse/src/lib.rs` `ChClient::new`
  (`.timeout(60 s)`), `query`, `query_limited`, `query_with_settings`,
  `run_cancellable`, `query_stream` (already sends
  `cancel_http_readonly_queries_on_client_close=1`, the pattern to copy),
  `ChError::Cancelled`.
- Limits in code: `lakehouse-bi/src/builder.rs` `SQL_SOURCE_SETTINGS`
  (2,000 rows, 30 s), `SQL_SOURCE_MAX_ROWS`, `with_export_settings`,
  `Relation::settings`, `POINT_LIMIT` (5,000), `point_limit`,
  `HISTOGRAM_LIMIT`; `lakehouse-bi/src/store.rs` (a hard-coded copy of
  the settings string in a test at the `max_execution_time = 30` line);
  `routes/query.rs` `MAX_RESULT_ROWS` (2,000), `execute_query_against_engine`
  (`query_limited(sql, MAX_RESULT_ROWS + 1, None)`), `SETTINGS_REFUSAL_MESSAGE`,
  `refuse_unlisted_settings`, the `"rowLimit"` field of the response.
- Deadlines: `routes/mod.rs` `route_timeout`, `DEFAULT_REQUEST_TIMEOUT`,
  `EXPORT_REQUEST_TIMEOUT`, `timeout_middleware`, `chat_timeout_from`
  (the clamp pattern).
- Kill: `routes/ops.rs` `kill_query_sql`, `cancel_workload`; `policy.rs`
  row `/api/ops/workloads/{id}/cancel` (`workload:cancel`).
- Tile path and messages: `routes/support.rs` `run_spec_sql`,
  `try_run_spec_sql`; `upstream_error` (`TILE`, `DATABASE`,
  `ch_error_for_author`).
- Settings pattern: `routes/settings.rs` `get_reporting`, `put_reporting`,
  `may_download` (role-name rules); `lakehouse-store/src/reporting_settings.rs`;
  console `src/features/admin/reporting-settings-page.tsx`.
- Console: `src/features/queries/sql-panel.tsx` (the Stop button,
  `runAct.cancel`), `use-query-studio.ts`; `src/services/clients/queries.ts`
  (`signal` only); `src/features/dashboards/dashboard-page.tsx` `load`
  (no signal, no Stop); `save-sql-source-sheet.tsx` (the sentence "at most
  2,000 rows and stop after 30 seconds", which must follow the setting).
- Proxy: `next.config.*` `rewrites` (`/api/:path*` to the API).

## 3. Tasks (one commit each, in order; Rust first)

**T1 Settings (D1, D2).** Migration, store, routes, `TileLimits`,
`effective_for`, memo. *Accept:* store round-trip; range refused with a
sentence; `effective_for` tests (no role rule, one role longer, two roles
→ longest, admin); `route_policy`/`route_auth` both ways; PUT clears the
memo.

**T2 Engine options (D3, D6).** `QueryOptions`, `query_with_options`,
`Relation::settings(limits)`, Query Studio and tiles send the options;
error classification. *Accept:* `wiremock` tests assert the URL settings
(`max_execution_time`, `max_result_rows`, `query_id`,
`cancel_http_readonly_queries_on_client_close`) and the per-request
timeout; builder tests: the SQL-source statement carries the configured
numbers, export settings unchanged; classification of codes 159 and 394
to the fixed sentences; SEC-11 guard green. The pinning test that ties
`SQL_SOURCE_MAX_ROWS` to the text is replaced, not deleted, with one that
ties the default to the migration's default (rule 2: say so in the
commit).

**T3 Deadlines (D4).** *Accept:* middleware tests: `/api/dashboard` gets
the longest configured + 15 s; other routes unchanged (the existing
`route_timeout_matches_typescript_max_duration` still passes); a board
whose tiles exceed the deadline answers the finished tiles and "not
reached in time" for the rest (mock engine).

**T4 Cancel (D5).** *Accept:* the kill builder: own prefix only, run id
must be a UUID, quotes escaped; route test: another principal's run id
never appears in the statement; `{ stopped: n }`; `route_auth` both ways.

**T5 Assistant (D7).** *Accept:* tool tests; fixture regenerated; count
pin; `capabilities.ts` test; AI-15 case.

**T6 Console.** Admin > Settings > Query limits (install values, a row per
role with its time limit); Query Studio Stop calls the cancel route then
aborts; the dashboard shows Stop while loading (sends `X-Run-Id`, cancel
then abort); tile messages from `limit`; the SQL-source sheet's sentence
reads the effective limits. Trino in Query Studio: the Stop button's
tooltip says it stops the page, not the query. *Accept:* component tests
(settings form, Stop calls cancel with the run id, tile message for time
and rows); typecheck, lint, tests of the touched files; webpack build.

**T7 Measurement.** `docs/plans/BI-5-RESULT.md`: on a running stack, a
query that runs 120 s (a `numbers()` scan sized to the machine) stopped
from Query Studio; time from click to its absence from `system.processes`,
five runs; and a 10 s limit stopping a tile at 10 s. *Accept:* commands
and raw output quoted; the 5-second target written as met or not met.

## 4. Out of scope

Trino cancellation, per-dashboard or per-chart limits, export limits,
the assistant changing limits, alert-rule query limits (`lakehouse-alerts`
keeps its own).

## 5. Must verify, not assume

- That `cancel_http_readonly_queries_on_client_close=1` cancels a
  `SELECT` sent with `FORMAT JSON` (the buffered `query` path, not only
  the stream) on ClickHouse 26.8 when reqwest drops the request; record
  in the RESULT document.
- Whether a browser abort reaches the API through the Next rewrite (the
  handler future dropped); if not, Stop relies on the cancel route alone,
  which D5 already provides; write which in the RESULT document.
- That `KILL QUERY ... ASYNC` with `LIKE` on `query_id` is accepted by the
  API's ClickHouse user (it may need `KILL QUERY` grant on its own
  queries only); if a grant change is needed, stop and report (compose and
  init SQL are owned elsewhere).
- That `max_execution_time` as a URL setting is not overridden for mart
  statements (the builder writes no `SETTINGS` there; grep the builder for
  every `SETTINGS` it emits, including `join_use_nulls`).
- Every role id reference: `role` table name and key type in
  `0002_seed_identity.sql`.

## 6. Checks

As in the phase 4 plans (same cargo environment:
`CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target CARGO_BUILD_JOBS=2
CARGO_INCREMENTAL=0`, one cargo command at a time, `--test-threads=4`,
`route_auth` at 2; check `free -g` / `df -h /` first; never `cargo build -p
lakehouse-api`; no `rust/target`; no prettier; touch the crate sources
before clippy when several worktrees share the target dir). Per task the
crates touched (`lakehouse-clickhouse`, `lakehouse-store`, `lakehouse-bi`,
`lakehouse-api`), the `lakehouse-api` lib filters for the modules touched
(`query`, `support`, `ops`, `settings`, `routes::tests` for the
middleware, `ai`), the integration files `route_auth`, `route_policy`,
`sec11_guard`, `ai_tool`. Once, before each handoff: `cargo fmt --check`
and whole-workspace `cargo clippy --workspace --all-targets
--all-features -- -D warnings` with the environment above (no full
workspace test run on this machine; CI runs it on the PR). TS: typecheck,
lint, tests of the files touched; the webpack build after T6.

## 7. Handoff (developer appends)

### T6 Console (developer, branch `feat/bi-phase-5`; Rust T1 to T5 and T7 not built)

Commit: `bef364f4` BI-5 feat(console): query limits settings, Stop, and limit messages on tiles (T6). TypeScript only; no cargo command run, no Rust change.

Built (feature decisions 1, 2, 4, 5; plan D1, D5, D6): Admin > Settings > Query limits (`features/admin/query-limits-settings.tsx`, rendered under the reporting settings in `app/(admin)/admin/settings/page.tsx`); `X-Run-Id` and `queryService.cancel(runId)` in contract and client; Stop in Query Studio (`use-query-studio.ts` `stopRun`: cancel best effort, then abort; Trino is only aborted and the button's title says so) and a Stop button on a loading dashboard (`dashboard-page.tsx`, "Stopped" sentence, tiles on screen kept); tile sentences for time and rows (`lib/query-limits.ts`, `tile-body.tsx`); the SQL source sheet and panel print the effective limits from the setting, and no number when they cannot be read (`hooks/use-effective-limits.ts`).

**Not on the server yet (the console calls or reads each of these):**
- `GET /api/settings/limits` and `PUT /api/settings/limits` (T1). Until then the settings section shows its read error with Retry, and the SQL source texts print no number.
- `POST /api/query/cancel { runId }` answering `{ stopped: n }` (T4). Until then Stop calls it, gets a 404 that is swallowed, and aborts the request, so the query keeps running on the engine. The Stop tooltip on ClickHouse says "Ends the query on the engine" and is only true once T4 lands.
- The `X-Run-Id` request header on `/api/query/run` and `/api/dashboard` (D5); the server ignores it today.
- `limit` on a failed tile and a row-cut marker on a tile result (D6, T2); no tile carries either today, so the new sentences are never shown against this server.

**Shapes I had to choose (the plan names fields but not the JSON; T1/T2 must return exactly these or the console changes):**
- `GET`: `{ install: { timeoutSeconds, chartRows, mapPoints, studioRows }, roles: [{ roleId, timeoutSeconds }], effective: { timeoutSeconds, chartRows, mapPoints, studioRows, source } }`, with `source` = `{ kind: "install" }` or `{ kind: "role", name }` (D2's `Install | Role(name)`). `PUT` body: `{ install, roles }` (a role not listed has no limit of its own); answers the same shape as `GET`.
- Failed tile (D6): `{ error, errorId?, limit: { kind: "time", value: seconds, source } }`.
- Cut tile: D6 says "a tile result carries `limit`", but a raw table's result already has a numeric `limit` (its page size, `Rows.limit`). The console reads the row-cut object as `limitHit: { kind: "rows", value, source }` on the result. T2 must use that name or the plan must choose another; this is a plan conflict, not a decision of mine.
- Ranges and defaults are in `lib/query-limits.ts` from the feature page (time 10 to 1,800 s, default 60; chart rows 100 to 100,000, default 2,000; map points 100 to 50,000, default 5,000; Query Studio rows 100 to 100,000, default 2,000). The form only says the range; the server has the last word.

Left out: the tile message for "the running query was stopped on the engine" (QUERY_WAS_CANCELLED) other than the page-level "Stopped": the plan gives no per-tile shape for it. Trino's Stop has no component test (the engine select is not driven in the page test); the rule is one condition in `stopRun`. The assistant tools (T5) and the measurement (T7) are not part of this handoff.

Checks (from `/home/hv/lakehouse-phase5-build`):
- `bun run typecheck`: exit 0.
- `bun run lint`: 0 errors, 9 warnings, none in a file of this commit.
- `bun test` of the new and touched files: `src/lib/query-limits.test.ts` and `src/features/admin/query-limits-settings.test.tsx` 12 pass; `src/services/clients/queries.test.ts`, `src/features/queries/query-studio-stop.test.tsx`, `src/features/dashboards/tile-limits.test.tsx` 28 pass; `src/features/dashboards/dashboard-stop.test.tsx` 1 pass; all 0 fail.
- `bun run test`: 2455 pass, 1 skip, 0 fail (255 files).
- Not verified: the webpack build (plan asks for it after T6); any of this in a real browser or against a server that has the routes; the settings page layout (two sections stacked in one page).


### Handoff (main merge, developer, branch `feat/bi-phase-5`)

Merged `origin/feat/bi-phase-4b` (QS-1, the Query Studio editor) into the branch; two conflicts, both in `src/features/queries`. Resolution: QS-1's `runBatch` owns the run (one request per statement), and BI-5 moved into it. Each statement is sent with its own `X-Run-Id` (`Batch.runId`, `Batch.engine`); `stopRun` calls `queryService.cancel` for the statement in flight on ClickHouse (best effort; Trino is only aborted, the button title says so), then aborts the batch controller, and the statements after it are reported "not run" as QS-1 does for a failure. `runAct` is gone from the hook and the panel. Query Studio has no limit sentence of its own in the T6 code (limit sentences are on tiles), so there is none to carry over; a limit error shows as the statement's error tab like any other.

- Added a Run all case to `src/features/queries/query-studio-stop.test.tsx`.
- `bun run typecheck`: exit 0. `bun run lint`: 0 errors, 9 warnings (none in these files).
- `bun test src/features/queries src/services/clients/queries.test.ts src/features/dashboards/dashboard-stop.test.tsx`: 85 pass, 0 fail (before the new test); `query-studio-stop.test.tsx` alone: 3 pass.
- `bun run test`: 2610 pass, 0 fail (2611 tests, 270 files).
- Not verified: a real browser; a server with the cancel route (T4 is not built); the webpack build.

### Handoff (server T1-T5, T7), developer, branch `feat/bi-phase-5`

Commits: `2c7207b1` T1 settings, `27400302` T2 engine options, `b937755a` T3 deadlines, `c23331bd` T4 cancel, `9cf44c94` T5 assistant, `95dcfc77` T2 follow-up found by T7, `3b57dee9` T7 `docs/plans/BI-5-RESULT.md`; `origin/feat/bi-phase-4b` merged twice (`50ed5d31`, `5b2c36ab`). Migration is `0101_query_limits.sql`. The console contract of section 7's T6 handoff is met as written (`/api/settings/limits` GET/PUT, `limit` on a time-limited tile, `limitHit` on a row-cut tile result, `X-Run-Id`, `POST /api/query/cancel { runId }` answering `{ stopped }`).

**Deviations (the planner accepted the first two in the coordinator's note):**
1. D3: the SQL-source `SETTINGS` text is swapped at run time (`builder::with_run_settings`, one body with `with_export_settings`) and the point-map `LIMIT` by `builder::with_point_limit`, not `Relation::settings(limits)`: a saved chart stores that text, and `ReadContext` has about 30 literal construction sites. The run (principal, `X-Run-Id`, counter) is a task-local set by `query_limits::scope_middleware`; tiles run on the request's task. `SQL_SOURCE_SETTINGS` now says 60 s (the new default); the pinned texts follow. The pin tying `SQL_SOURCE_MAX_ROWS` to the text stays, and a new test ties the builder text, `QueryLimits::default` and migration 0101's defaults.
2. D4: board routes get the 35-minute cap as the middleware deadline and the handler's `ceil(tiles/6) x limit + 15 s` decides (a one-limit middleware deadline would cut a two-round board first); `/api/query/run` gets the longest limit + 15 s. Written at `deadline_for`.
3. D7: `stop_query` with no `runId` stops all of the person's running queries, not only those started in the chat (query ids do not say which surface started them).
4. A tile query is cut when more than `chart_rows + 1` rows come back (the engine is asked for one more than the cap; exactly cap + 1 is kept, because pivots and previews read one past their own cap to say they cut).

**Found by T7 (see `docs/plans/BI-5-RESULT.md`):** `cancel_http_readonly_queries_on_client_close` alone does NOT cancel a buffered `FORMAT JSON` query when the client leaves (still running after 40 s); progress headers make it go in 0.5 to 3 s, so `query_with_options` sends them. With progress headers the time-limit error arrives as an HTTP 200 body; the client now reads that as an error instead of an empty result. Both are fixed in `95dcfc77`.

**Limits of this slice (backlog follow-ups):** the SQL source preview in the editor keeps the default text and does not follow the setting; a tile cut at the map-point limit carries no `limitHit` marker; a board-deadline end-to-end test with a mock engine is not written (the mechanism, `tile_pool::run_tiles_until`, and `board_deadline` have unit tests).

**Commands run (from `rust/`, `CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0`, under `flock`):**
- `cargo fmt --check`: clean.
- `cargo clippy -p lakehouse-store -p lakehouse-api --all-targets -- -D warnings` (T1) and `-p lakehouse-clickhouse -p lakehouse-bi -p lakehouse-api` (T2, T3): clean on the 1.96 toolchain. On 1.99.1 `-p lakehouse-api` reports three lints in code that is not BI-5's (`dashboard.rs:196` and `render.rs:152` `result_large_err`, `export.rs:484` `unused_async_trait_impl`); the BI-5 lints on 1.99 (`Duration::from_mins`, `assert_is_empty`) are fixed. `-p lakehouse-clickhouse --all-targets -- -D warnings` clean on 1.99.1 after the follow-up.
- `cargo test -p lakehouse-clickhouse --lib`: 33 passed. `-p lakehouse-bi --lib`: 554 passed, 3 ignored (T2). `-p lakehouse-api --lib` with filters: `query_limits` 12 passed (T1); `run_spec_sql_enforcement` 6 passed; `routes::dashboard routes::query routes::embed routes::layers support query_limits` 327 passed, 16 ignored; `tile_pool query_limits routes::tests route_timeout routes::dashboard routes::embed` 217 passed; `routes::ops query_limits route_policy policy::` 83 passed; `routes::ai routes::ops tools:: prompt` 460 passed, 1 ignored; the ignored `write_tool_schema_fixture` run once (count pin now 108).
- `bun test src/features/copilot`: 73 pass. `python3 -I -m unittest` on `ops/ai_eval/test_ai_eval.py`: 165 OK (the new case is `limits_why_timed_out`).
- T7: scripts against a throwaway ClickHouse 26.8 container (removed afterwards).

**Not verified:** every integration test (`tests/query_limits.rs` in the store, `tests/query_limits_route.rs`, `tests/query_cancel_route.rs`, the new `route_auth.rs` cases, `sec11_guard`, `ai_tool`) was written but not run here, per the machine rules; CI on #245 runs them. The full workspace clippy/test on 1.99.1. The end-to-end Stop timing through console, proxy and API, whether a browser abort reaches the API through the Next rewrite, and the API ClickHouse user's `KILL QUERY` grant (section 5 items) are *not verified*. The webpack build. The AI-15 run itself (the case is added, not run).

### Review fixes (server), developer: Sonnet, 2026-10-25

Commits: `f3db9aad` (B1, B2, S1, S2, S3, S4, S7 server part) and `57ea84a6` (S6, S9, S5/E6).

- **B1.** `support::with_tile_rows` wraps the tile's `run_spec_sql` (dashboard, embed/public, builder
  preview) and sets `query_limits::with_row_cap`: `map_points` for `pointmap`/`geoheat`, the
  statement's own trailing `LIMIT` minus 1 for `pivot` (`builder::trailing_limit`; the pivot's
  `pivot_row_cap`, up to 10,000), nothing for any other kind. `try_run_spec_sql` uses
  `max(chart_rows, cap)` for the outer `max_result_rows`, the cut check, the truncation and
  `limitHit`; the SQL-source `SETTINGS` text keeps `chart_rows + 1` (a map over a SQL source is still
  held to the source cap). The cap is in `max_result_rows`, so it is in the BI-4 cache key. Test:
  `a_point_map_and_a_pivot_read_past_the_chart_row_limit_and_a_bar_does_not`.
- **B2.** `stream_chat` takes `QueryRun::current_or_detached()` in the request task and runs the
  spawned chat in `query_limits::scope` (which now takes `impl Into<Arc<QueryRun>>`). For a run
  with no request at all (a scheduled headless agent) `run_headless_loop` is a thin wrapper that
  uses `scope_if_missing`, a run built with `QueryRun::for_request(state, principal, None)`: the
  install snapshot and the agent's roles, not `Snapshot::default()`. `QueryRun::detached()` itself
  is unchanged (it has no `AppState` to read from); the callers that can have one now use the new
  function. Test: `a_streamed_chats_run_sql_carries_the_callers_query_id_and_the_limit_settings`.
- **S1.** `is_board_path` matches `/api/dashboard`, `/api/render/data`, `/api/embed/data` and
  `/api/public/dashboard/{token}` (one segment) only; `route_timeout` asserted for `/api/embed/frame`,
  `/api/embed/values`, `.../unlock`.
- **S2.** `query_limits::mark_stopped` (in-process map, 40-minute TTL, 10,000 marks) is set by the
  cancel route before it asks the engine; `try_run_spec_sql` answers `Stopped` for a marked run
  before it sends. The `KILL` is now sent even when the count is 0. Test:
  `a_tile_of_a_stopped_run_sends_no_query`. The browser-abort question stays *not verified* (QA).
- **S3.** `builder::find_source_settings` matches the cap with any two numbers; used by
  `with_run_settings` (so `with_export_settings`) and `with_point_limit`. Test with the pre-BI-5
  text (`max_execution_time = 30`).
- **S4.** `export_statement_settings` takes the caller's limits and applies them to a grouped
  download. Test: `a_grouped_download_takes_the_callers_limits_like_its_tile`.
- **S5.** `docs/plans/BI-5-RESULT.md` E6, with the planner's numbers.
- **S6.** `GET /api/settings/limits` sends `install` and `roles` only when the caller holds
  `settings:write`; the console's `QueryLimits.install/roles` are optional and a reader sees
  "For you: ..." only. Route test and a console test.
- **S7.** `tests/query_run_limits.rs`: `/api/query/run` with `X-Run-Id` sends
  `query_id=<user>:<run>:0` and the role's `max_execution_time`. **Not written:** the board
  deadline end-to-end test with a mock engine. The deadline is `ceil(tiles/6) x limit + 15 s` with a
  10 s minimum limit, so the test would run at least 35 s of real time (a paused clock does not
  hold with a real socket in the loop); the mechanism (`run_tiles_until`, `board_deadline`) keeps
  its unit tests.
- **S8.** Unchanged: *not verified* until QA.
- **S9.** The limits/Stop sentences (and the BI-4 `computedAt` read) are in `SYSTEM_ASK_SUFFIX` and
  `SYSTEM_BUILD_SUFFIX` through one `macro_rules!` + `concat!`. They are **not** in `SYSTEM_BASE`:
  `routes::ai::tests::the_base_prompt_is_byte_for_byte_what_it_was` pins that measured prompt
  (length and FNV hash) and failed when the line was first moved there; the pin was kept.

Large-future note: `with_tile_rows` returns a `BoxFuture` and `query_metric` boxes `value_query`
because the tile query's future grew past the `clippy::large_futures` size with the cache.

Commands (from `rust/`, shared target, `CARGO_BUILD_JOBS=2`, `CARGO_INCREMENTAL=0`, under `flock`,
1.99.1; `touch crates/*/src/lib.rs` before clippy):
- `cargo fmt -p lakehouse-api -p lakehouse-bi`; `cargo clippy -p lakehouse-bi -p lakehouse-api
  --all-targets --all-features -- -D warnings`: clean (last run after the S9 change, `-p
  lakehouse-api`; `lakehouse-bi` after S3).
- `cargo test -p lakehouse-bi --lib builder`: 119 passed.
- `cargo test -p lakehouse-api --lib -- routes::support routes::ai query_limits routes::dashboard
  routes::embed`: 655 passed, 4 ignored (earlier run with `routes::agents routes::tests` added: 683
  passed before the two failures it found, now fixed).
- `bun run typecheck` 0 errors; `bun test src/features/admin src/features/dashboards
  src/features/copilot src/lib src/services`: 1662 pass, 0 fail.

*Not verified:* the new integration files (`tests/query_run_limits.rs`, the added cases in
`ai_chat_stream.rs` and `query_limits_route.rs`) compiled by clippy `--all-targets`, not run; the
full workspace clippy and test; the board-deadline test above; the browser abort; the `KILL` grant;
the webpack build.

### Re-review fixes R1, R2, developer: Sonnet, 2026-10-25

Commits on `feat/bi-phase-5`: `43e34c9b` (R1), `5aaa2fc7` (R2).

- **R1.** `build_and_send` loads the owner, then runs the rest (`deliver`) inside
  `query_limits::scope_if_missing(state, Some(&owner), ...)`, boxed. New integration test
  `a_scheduled_delivery_asks_the_engine_as_the_owner_under_the_saved_limits` (saves a 17-second
  limit, checks every tile query's `query_id` starts with the owner's id and carries
  `max_execution_time=17`).
- **R2.** `query_cancel::stop_own_queries` (mark the run, count, always send the kill) is used by
  `POST /api/query/cancel` and by `stop_query`; only `routes/ai/tools/limits.rs` was touched under
  `routes/ai/`. Unit test: `stop_query` with a `runId` marks the run (`is_stopped`) and sends the
  kill when the count is 0.

Commands: as in the BI-4 entry (fmt clean, clippy `--all-targets --all-features -D warnings` clean,
37 lib tests passed including `tools::limits`).

*Not verified:* the R1 integration test and the rest of `tests/report_runner.rs` (needs Postgres;
written and compiled only); the full workspace run; `route_auth`/`sec11_guard` were not re-run.

### PR #245 CI: the system-prompt pin in `ai_sql_help.rs` (developer)

`ai_sql_help.rs::a_chat_with_no_allowlist_is_offered_today_s_tools_and_today_s_prompt` failed
(`45c0fcda...` against the pinned `f59505f7...`). The pin belongs to main's AI team (added in #127,
last moved in #188), and its comment says a change of the bytes is updated by hand when intended.
The test's turn runs in Ask mode, so only the Ask suffix reaches it. The cause is BI-5 review S9:
`SYSTEM_ASK_SUFFIX` now ends with the `limits_and_freshness!` sentences. Proof: hashing the captured
system message with exactly that text removed gives the old pin `f59505f7...` again, so nothing else
moved (no merge artifact, no changed wording). The Build-only additions (set_cache_rule, the
export and report lines) do not reach this test, and the offered-tool list assertion still passes.
`feat/bi-phase-6` passes with the old pin because it does not contain S9 (no
`limits_and_freshness!`); its Distance/RegionOf line is in the Build suffix only.

Pin updated to `45c0fcda7b755bbcb6eb7fa7bb48df1f724b6be61b5738546c55c41fabbe006b`. **The AI team must
review these added Ask-mode sentences (Assistant parity):**

> Charts and Query Studio run under time and row limits an administrator sets: for "what are my
> query limits?" or "why did this chart time out?" call get_query_limits and say the limit and where
> it comes from; you cannot change limits, an administrator does in Admin > Settings > Query limits.
> To "stop the running query", call stop_query (it ends only the person's own running queries).
> Dashboards remember the results of their charts; each tile result carries computedAt (when the
> data was computed) and remembered (true when it came from memory), so for "how fresh is this
> dashboard?" read them from the data and say the time.

### PR #245 CI: `query_cancel_route.rs` encoded the pre-review Stop (developer)

`nothing_running_is_stopped_zero_and_sends_no_kill` asserted that no `KILL` is sent when the count
is 0. That is the behaviour before BI-5 review S2 / re-review R2, which decided Stop always sends the
kill (a query can start between the count and the kill; `KILL` of nothing is harmless). The test was
wrong, not the product. Renamed to
`nothing_running_is_stopped_zero_and_still_sends_a_kill_for_only_the_callers_own_run`: it now asserts
`{stopped: 0}`, exactly one `KILL`, that it is `KILL QUERY WHERE query_id LIKE '<caller>:<run>:%'
ASYNC`, and that no request names another person. The stopped mark is not asserted here (the
integration app exposes no `AppState`); it is set by the same `stop_own_queries` and asserted in
`routes/ai/tools/limits.rs`.

**Not verified:** this test was not compiled or run, and the full `cargo test -p lakehouse-api` sweep
was not run, because `df -h /` showed 3.8 GB free (below the 5 GB stop line).

## 8. Review (planner appends)

Planner, 2026-10-24: the console's shapes are accepted as the contract for the Rust tasks (the `/api/settings/limits` GET/PUT shape, the failed tile's `limit`, `X-Run-Id`, `POST /api/query/cancel`). The row-cut marker on a tile result is named **`limitHit`**, because `limit` is already a raw table's page size. T1 to T5 build to these shapes.

### Review of the server half (T1–T5, T7), planner, 2026-10-25

Commits read with `git show` at `e0076536`; the reviewer did not run cargo. Security is clean:
- **Cancel scope.** `KillTarget::own_run` builds its pattern only from the caller's own UUID plus a run id parsed as a UUID, and binds it via `SqlLiteral`. The principal comes from the session, and `deny_unknown_fields` is on.
- **`X-Run-Id`.** A missing or invalid value gets a fresh UUID.
- **SQL text.** Saved SQL sources cannot carry `SETTINGS`, and `with_run_settings` rewrites only the builder's own clause.
- **Standards.** SEC-11 holds, both routes are in `POLICY_TABLE` and `route_auth`, there are `# Errors` docs, and no `unwrap` outside tests.

- **BLOCKER B1, point maps and pivots are cut at `chart_rows`.**
  - Where: `routes/support.rs:316`/`:329`. `try_run_spec_sql` sends `max_result_rows = chart_rows + 1`, then truncates to `chart_rows` with `limitHit: rows`. That happens after `with_map_limit` set a point map's `LIMIT` to `map_points`.
  - Effect at the defaults: a mart point map shows 2,000 of 5,000 points and says "rows", so decision 2's map setting does nothing above `chart_rows`. A mart pivot's own cap (`pivot_row_cap`, up to 10,000) is cut to 2,000.
  - Fix: pass the cap per kind: `map_points` for `Pointmap`/`Geoheat`, and the pivot's own cap for pivots (planner decision: pivots keep the cap they had before BI-5; no behaviour change for them).
  - Test: a mart point map of 3,000 rows comes back whole with no `limitHit`.
- **BLOCKER B2, the assistant's queries ignore the settings and cannot be stopped.**
  - Where: `routes/ai/mod.rs:1158–1177`. `stream_chat` runs `run_chat` in `tokio::spawn`, which drops the task-local. `run_sql` then falls back to `QueryRun::current_or_detached()`: nil principal, no roles, `Snapshot::default()`.
  - Effect: the chat runs at the compiled defaults whatever the administrator set, and `stop_query` cannot find its `000…:` queries.
  - Fix: capture the run before the spawn and run the spawned future inside `query_limits::scope(...)` (let `scope` take an `Arc`). Make `detached()` read the install snapshot when `AppState` is available.
  - Test: a streamed chat `run_sql` sends `query_id=<user>:` and the role's `max_execution_time`.
- **SHOULD-FIX S1, `is_board_path` gives a 35-min outer deadline to non-board routes** (`routes/mod.rs:200`, `:218`): `/api/embed/frame`, `/api/embed/values`, `/api/public/dashboard/{token}/unlock`. Match only whole-board routes (`/api/dashboard`, `/api/render/data`, `/api/embed/data`, `GET /api/public/dashboard/{token}`), and assert `route_timeout` for the other three.
- **SHOULD-FIX S2, Stop and later tile rounds.**
  - On a board with more than 6 tiles, the next round may start new queries after Stop, unless the browser abort reaches the handler through the Next rewrite (*not verified*).
  - Fix: add a short-lived stopped mark per (principal, run) that `try_run_spec_sql` checks before sending (answer "Stopped"). Send the `KILL` even when the count is 0.
  - Verify at QA on a board of more than 6 slow tiles (`BI-5-AC2`).
- **SHOULD-FIX S3, `with_run_settings`/`with_export_settings` only match the current default text** (`lakehouse-bi/src/builder.rs:91`).
  - Effect: statements saved with the pre-BI-5 `max_execution_time = 30` keep the old limits and lose `BI-1`'s 1,000,000-row export lift.
  - Fix: match the clause with any numbers, and add tests with the old text.
- **SHOULD-FIX S4, a grouped export no longer matches its tile** (`routes/dashboard.rs:3566`, `export_statement_settings`). Apply the caller's limits there too, so the file holds what the tile shows.
- **Accepted, measured S5.** Does `max_result_rows` + `break` cut subqueries?
  - Measured by the planner on the dev ClickHouse 26.8.9.10 with `max_result_rows=2001, result_overflow_mode='break'`:
    - `SELECT sum(x), count() FROM (SELECT number x FROM numbers(10000))` → `49995000, 10000` (whole);
    - an `IN (SELECT … 5000 rows)` filter → `5000` (whole);
    - `count()` over a 100,000-row `GROUP BY` subquery → `100000` (whole).
  - Only the final result is cut. Record this as E6 in `docs/plans/BI-5-RESULT.md`.
- **SHOULD-FIX S6, `GET /api/settings/limits` returns more than decision 6 allows** (`routes/query_limits.rs:140`). Return `install` and `roles` only to `settings:write`; others get `effective` only. Update the console and the route test.
- **SHOULD-FIX S7, missing acceptance tests.**
  - T3's board-deadline route test with a mock engine: the finished tiles plus "Not reached in time".
  - A router-level test that `X-Run-Id` reaches the engine: `/api/query/run` sends `query_id=<user>:<run>:0` and the role's `max_execution_time`.
- **SHOULD-FIX S8, still not verified on the real stack:** the API ClickHouse user's `KILL QUERY` grant, and the click-to-gone time. Close both at QA (`BI-5-AC2`). `BI-5-RESULT.md` says *not verified* until then.
- **SHOULD-FIX S9, the prompt line is in Build mode only** (`SYSTEM_BUILD_SUFFIX`), but both tools are offered in Ask mode and the AI-15 case runs in Ask mode. Move the line into the shared prompt.
- **Accepted:**
  - Deviations: D3 (task-local + text swap; conditional on B2 and S3), D4 (35-min middleware cap, the handler decides; conditional on S1), D7 (`stop_query` with no `runId` stops all of the caller's own queries; narrowing, DEC-8), and the row cut at `chart_rows + 1`.
  - Small notes: the one post-deadline poll, the memo race inside 30 s, `to_u32`'s fallback (unreachable through the `CHECK`s), `body_is_exception` only after a failed parse, and progress headers ≤ 80.

Verdict: not ready to merge until B1 and B2 are fixed; then S1–S4 and S6–S9.

### Re-review of the server fixes, planner, 2026-10-25

- **B1 closed:** `rows_cap` + `with_tile_rows`, applied in the dashboard, embed/public and preview paths, with a test. The cap is in the BI-4 key.
- **B2 closed for the chat and the agents.**
  - **SHOULD-FIX R1:** the report runner calls `payload_for` from its spawned loop (`report_runner.rs:1257` → `:681`) with no run, so scheduled reports use the nil principal and `Snapshot::default()`. Wrap the delivery in `query_limits::scope_if_missing(state, Some(owner), …)`. Test: a delivery sends the owner's query id and the configured `max_execution_time`.
- **S1 closed.**
- **S2 closed for the route.**
  - **SHOULD-FIX R2:** the assistant's `stop_query` (`ai/tools/limits.rs:58-86`) is a second copy of the cancel logic, with no stopped mark and still `if stopped > 0` (rule 4). Make one helper for both. Test: `stop_query` with a `runId` makes a later tile of that run answer "Stopped".
- **S3, S4, S5, S6, S9 closed.**
  - Accepted note on S4: a mart-based grouped export is not cut at `chart_rows`. It is the same rewritten statement, so nothing leaks.
- **S7 partly done, accepted:** the board-deadline end-to-end test is replaced by the `tile_pool` unit test plus a QA step in `BI-5-AC2`.
- **S8 open:** the `KILL` grant and the click-to-gone time, at QA.

Verdict: B1 and B2 are fixed; R1 and R2 are SHOULD-FIX; the new integration tests must run green before merge.

### Re-review of R1 and R2, and S8 (Evan's planner, 2026-10-26, at `4b96fc4e`)

- **R1, closed.** `report_runner.rs` runs the delivery inside `query_limits::scope_if_missing(state, Some(owner), …)`, so a scheduled report sends its owner's query id and the configured limits.
- **R2, closed.** The assistant's `stop_query` (`ai/tools/limits.rs`) and `POST /api/query/cancel` share one `query_cancel::stop_own_queries`. It marks the run stopped and always sends the kill.
- **S8, grant part: closed by measurement** (`docs/plans/BI-5-RESULT.md`, 2026-10-26). An engine user needs `SELECT ON system.processes` and no `KILL QUERY` grant to stop its own queries, and the engine refuses to let it stop another user's. The compose API user holds every grant.
- **S8, click-to-gone part:** still open, and taken at the owner's QA (`BI-5-AC2`) after merge.

No BLOCKER is open. After merge, `BI-5` is In Acceptance, not Released, until `BI-5-AC2` passes.

### Handoff (merge main after #235), developer, branch `feat/bi-phase-5`

**Merge commit `50c13352`** — "Merge remote-tracking branch 'origin/main' into
`feat/bi-phase-5`", parents `48d6aefb` (the branch) and `7fe61227` (PR #235,
`BI-19` part A and `BI-26`, merged to `main`). Pushed to
`origin/feat/bi-phase-5` as a fast-forward (`48d6aefb..50c13352`).

**Why the merge was small.** The branch had already merged
`origin/feat/bi-phase-4b` (the #235 branch) at `5916b839`, so most of #235 was
already in the tree; the merge that had to be finished was the one against
`main`'s own copy of it. The audit was done hunk by hunk against both parents
(`git diff HEAD -- <f>`, `git diff MERGE_HEAD -- <f>`, `git show HEAD:<f>`,
`git show MERGE_HEAD:<f>`), not by taking a side of the whole file (rule 9).

**Per-file resolution of the seven conflicts:**

- **`rust/crates/lakehouse-api/src/routes/embed.rs`** — main's `embed_grant`
  (BI-26's one admission: `verified_claims` then `admit`) is restored and
  `embed_map_geojson` calls it, so the map route and the data route cannot
  disagree about what a token opens. The public map route resolves the link
  the way `public_dashboard` does (BI-19 part A: Postgres, with the password
  ticket). The branch's BI-4 work is untouched: `render_board_payload` takes
  the tile slots and runs the tiles through `tile_pool`, `data` and
  `public_dashboard` wrap it in `result_cache::scoped`, and BI-5's board
  deadline and row cut stay in the same loop. `database_failure` stays as the
  branch's helper for the map route's store failure (same 500, same
  `upstream_error::report_ch` payload; the engine's text never reaches the
  answer, SEC-11).
- **`rust/crates/lakehouse-api/src/routes/settings.rs`** — both switches are
  on the reporting settings: `publicLinksEnabled` (BI-19 part A) and
  `resultCache` (BI-4 part B), and the test asserts both callers (a reader
  without `settings:write` and an administrator read the same switch).
- **`rust/crates/lakehouse-api/tests/maps_routes.rs`** — main's version: the
  public-link row is written through `lakehouse_store::public_links::replace`
  (the store helper, rule 4) instead of a raw `INSERT`, and `EmbedResource`
  names `chart` (BI-26). Same behaviour (a live link row in Postgres, token
  of at least eight characters).
- **`rust/crates/lakehouse-api/tests/reporting_settings_route.rs`** —
  `publicLinksEnabled` and `resultCache` are everybody's, so they belong to
  the helper every caller reads; BI-4 part B's save/keep/refuse test stays.
- **`rust/crates/lakehouse-api/tests/route_auth.rs`** — unchanged, and
  complete: the branch already carried #235's two gate tests (it merged
  `origin/feat/bi-phase-4b` at `5916b839`), and BI-5's two gates sit beside
  them. Every route of both sides is in `POLICY_TABLE` (checked: `/api/embed/values`,
  `/api/dashboard/embed-settings`, `/api/public/dashboard/{token}/unlock`,
  `/api/dashboard/public-link` GET/PUT/DELETE, `/api/settings/limits`
  GET/PUT, `/api/query/cancel`) and the table-driven loops assert each one
  both ways; the "every registered route is in `POLICY_TABLE`" test passes.
- **`rust/crates/lakehouse-auth/src/throttle.rs`** — main's `record_attempt`
  (BI-19 part A review fix (BLOCKER) R1) is kept: one atomic UPSERT that does
  not push a public link's lock further out when a guess continues, which is
  what BI-19 R1 decided. The SEC-49 decision (PR #236) is kept by it too: the
  attempt is counted before the password is hashed, the caller verifies only
  while the count is within `max_failures`, `clear` on success and
  `refund_attempt` on a storage error. The branch's copy was the same
  function as it stood before SEC-49 landed (an artefact of the earlier
  merge), not a decision of this plan; the doc comment at the site names the
  provenance and which copy is kept.
- **`src/services/contracts/settings.ts`** — unchanged, and complete: the
  branch already carried #235's `publicLinksEnabled` beside BI-4/BI-5's
  `resultCache`, `QueryLimits` and `TileLimit`.

Three files git merged cleanly but that carry #235's side, kept whole:
`policy.rs` (the public block holds #235/BI-26's two public routes and the
count is 13, `exactly_thirteen_public_entries_exist`; BI-5's
`settings/limits` and `query/cancel` rows stay),
`src/features/dashboards/embed-view.tsx` and
`src/features/dashboards/public-dashboard.tsx` (the map request carries the
public link's ticket, `X-Public-Link-Ticket`, which the merged
`public_map_geojson` asks for of a link with a password).

The two #235 plan documents take main's version, which carries the re-review
of part A and the scope of PR #235; the branch's own handoff sections in them
are already in main's copy (they came from `origin/feat/bi-phase-4b`).

**Checks run** (from `rust/`, shared `CARGO_TARGET_DIR=/home/shiro/rantai/cargo-target`
through `/home/shiro/rantai/bin/cargo-locked`, foreground, one command at a
time; `free -g` and `df -h /` checked first — 10 GB RAM free, 208 GB disk
free):

- `cargo fmt --check`: clean (exit 0).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  clean (exit 0), 1 m 26 s.
- `cargo test --workspace`: exit 0 — **8195 passed, 0 failed, 49 ignored over
  155 test binaries** (39 m 38 s). The binaries this merge touches:
  `route_auth` 64, `embed_tokens` 41, `reporting_settings_route` 13,
  `maps_routes` 11, `public_links` 10 (API) and 12 (store), `sec11_guard` 4,
  `upstream_errors` 6, and `lakehouse-auth`'s `throttle` 14.
- `bun run typecheck`: exit 0.
- `bun run lint`: exit 0 — 0 errors, 11 warnings, all pre-existing and in
  files this merge does not touch.
- `bun run test`: exit 0 — 2890 pass, 1 skip, 0 fail (2891 tests, 297 files).
- `python3 ops/lint/check_intra_package_imports.py` and
  `python3 ops/lint/check_bare_iceberg_count.py`: clean.
- `docker compose --profile '*' config --quiet`: clean (with throwaway
  `POSTGRES_PASSWORD`, `RUSTFS_ACCESS_KEY`, `RUSTFS_SECRET_KEY`,
  `LAKEKEEPER_ENCRYPTION_KEY`, `CONNECTOR_SECRETS_KEY` exported).

**Not verified:**

- `cargo audit` and `cargo deny`: red on `main` for the zstd advisories;
  PR #261 fixes them. No dependency was touched by this merge, so nothing here
  was run against them.
- The dagster suite (`python -m pytest dispar_orchestrate -q`): not run. This
  merge touches no file under `dagster/`; the branch's own BI-4/BI-5 work
  never did either, and CI runs it on the PR.
- A real `docker compose up` (compose files were not changed by this merge).
- The webpack build, and any browser check of the public link's password page
  or of the ticket-carrying map request. The ticket header is covered where it
  is sent for the board's data (`public-link-access.test.tsx`); the map
  request's header is asserted for its URL and for not going through
  `apiFetch` (`public-map-boundary.test.tsx`), not for the header itself.
- The owner's QA items still open from the reviews above: `BI-5-AC2` (the
  click-to-gone time on a board of more than six slow tiles) and the
  `S8` click-to-gone part.
