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


## 8. Review (planner appends)

Planner, 2026-10-24: the console's shapes are accepted as the contract for the Rust tasks (the `/api/settings/limits` GET/PUT shape, the failed tile's `limit`, `X-Run-Id`, `POST /api/query/cancel`). The row-cut marker on a tile result is named **`limitHit`**, because `limit` is already a raw table's page size. T1 to T5 build to these shapes.
