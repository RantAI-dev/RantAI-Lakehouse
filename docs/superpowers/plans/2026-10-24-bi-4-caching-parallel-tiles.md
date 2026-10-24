# BI-4: parallel tiles (part A) and remembered results (part B), with a new AI item

Planner and reviewer: Claude Opus. Developer: a non-Opus coding agent.
Branch: `feat/bi-phase-5` (to be cut; name confirmed when the planner opens
the phase's draft PR), base `feat/bi-phase-4` (it rewrote `payload_for` for the
PDF print view and `canDownload`; cutting from `main` before phase 4 merges
would conflict in the same function). Rebase on `main` once phase 4 merges.
Feature page: `docs/core/features/result-caching-and-parallel-tiles.md`
(decision 1 waits for the owner, Q1 in the phase's questions; 2 to 9 planner
defaults confirmed at the phase 5 QA). Spec: `docs/core/specs/bi-4.md`.

**Order inside phase 5:** part A first (small, no correctness risk, most of
the speed), then `BI-5` (it changes how a tile query is sent: limits as
request settings, a query id, cancellation), then part B here (the cache key
must include the limits `BI-5` introduces). Part B does not start before
`BI-5`'s T2 is committed.

Anchors were read on 2026-10-23 on `feat/bi-phase-4` at `150a081a` (which
has `main` up to `#71` merged; `origin/main` at `364227e2` is not yet merged
into it). Find each by the symbol named.

## 1. Decisions already made (do not re-ask, do not change)

The feature page's table, plus these build decisions.

- **D1 Part A: tiles concurrently, inside the request.** `get_body`
  (`routes/dashboard.rs`) awaits each tile in turn today (the `for c in
  &stored_for_board` loop, and the built-in `KPIS`/`CHARTS` loops). Each
  tile's whole pipeline (`effective_chart` → `stored_chart_sql` →
  `run_spec_sql` → `annotate_*` → `annotate_layers`) becomes one future;
  the futures run with `futures::stream::iter(..).buffer_unordered(6)` on
  the request's own task (no `tokio::spawn`: everything borrows from the
  request, and the work is I/O). Results go into `results` keyed by id, as
  today. The same change in `routes/embed.rs` `render_board_payload` (the
  embed and public payloads). The `allow(clippy::too_many_lines)` reason on
  `get_body` that says "parallel tile execution" is corrected (it was not
  parallel; rule 1).
- **D2 Per-process bound.** `AppState` gains `tile_slots:
  Arc<tokio::sync::Semaphore>` (24 permits, `TILE_CONCURRENCY_MAX` env,
  clamped 4..=128, read once like `AI_CHAT_TIMEOUT`); a tile holds a permit
  only while its queries run. Same pattern as `pdf_slots`.
- **D3 Part B: one choke point.** The cache sits inside
  `routes/support.rs` `try_run_spec_sql`, **after**
  `policy_engine::rewrite_sql_for_roles` and before the engine call. Every
  tile path already reaches it (`run_spec_sql`, `layers.rs` `RoleRunner`,
  `annotate_table`'s count, `dashboard_sources.rs`, `metrics.rs`), so no
  caller can read around it. `try_run_spec_sql` takes a small
  `TileEngine<'_> { ch, cache: Option<&ResultCache>, rule: CacheRule,
  limits: &TileLimits }` in place of `ch` (`TileLimits` is `BI-5`'s); the
  callers are updated, not given a second function (rule 4).
- **D4 Key.** SHA-256 over: the rewritten statement text, the request
  settings sent with it (`BI-5`'s `max_execution_time`, `max_result_rows`),
  and the engine name. Not the user id: the rewritten text already carries
  the person's masks and expanded `__principal_id__`/tenant placeholders
  (`sql_rewrite::PlaceholderValues`), so equal text means equal visible
  data. A module doc says this and names feature decision 4; a test proves
  two role sets with different masks produce different keys and two
  principals with a `__principal_id__` row filter produce different keys.
- **D5 Store.** `moka::future::Cache<[u8; 32], Arc<CachedResult>>` with a
  byte weigher (serialized JSON length), capacity `RESULT_CACHE_MAX_MB`
  (default 256, clamped 0..=4096; 0 switches caching off), per-entry cap
  8 MB, `time_to_live` 24 h as the outer bound. `CachedResult { value,
  computed_at, tables: Vec<(db, table)>, fingerprint }`. `moka` is already
  in `Cargo.lock` (0.12.16, via `iceberg`); add it to `lakehouse-api` at
  that version with only the `future` feature. Use `get_with`-style
  coalescing so two tiles with one key run one query.
- **D6 Freshness (feature decision 1, as signed).** On a hit, the entry is
  valid while (a) its age is under the rule's duration and (b) the
  fingerprint of its tables is unchanged. The fingerprint is
  `SELECT database, table, max(modification_time), sum(rows) FROM
  system.parts WHERE active AND (database, table) IN (...)` for the
  tables the statement reads (`sql_rewrite::referenced_tables` after
  rewrite), **one query per dashboard request** for all tiles' tables,
  not per tile. A table absent from `system.parts` (a view, an Iceberg
  table engine, a table function) makes the entry "unwatchable": it falls
  back to the 10-minute bound. A failed fingerprint read is treated as
  "changed" (fail closed: compute fresh), logged, never surfaced.
- **D7 Rule storage.** Install: a Postgres setting beside
  `reporting_settings` (migration: next free number at build time; check
  every branch, `0098`+ are being taken by `BI-19`): `result_cache_mode
  TEXT NOT NULL DEFAULT 'default'`, `result_cache_minutes INT NULL`, with a
  why-header and a `CHECK` on the range. Dashboard: a `cache_json` column on
  `console.bi_board` via `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` in
  `lakehouse-bi/src/store.rs` (the pattern `refresh_seconds` uses) and in
  `BOARD_COLS`/`BOARD_KEEP_COLS`. Chart: an optional `cache` field on the
  chart definition next to `referenceLines`, validated in `lakehouse-bi`.
- **D8 Payload.** Each tile result gains `computedAt` (RFC 3339, UTC) and
  `remembered: bool`; the payload gains `cacheRule` (the effective rule of
  the board). `GET /api/dashboard` and the single-tile reads take
  `fresh=1` (feature decision 6), which skips the lookup and replaces the
  entry. A tile result with `error` is never stored.
- **D9 Assistant.** `set_cache_rule` (write risk class, needs approval);
  chart-data tools add `computedAt`/`remembered`; prompt line; named in
  `src/features/copilot/capabilities.ts`; `tests/fixtures/tool_schemas.json`
  regenerated with the ignored writer test (`write_tool_schema_fixture`);
  count pin +1; one `AI-15` case in `ops/ai_eval/dashboard_cases.json`.

## 2. Anchors (verified at `150a081a`, find by symbol)

- Tile path: `routes/dashboard.rs` `get`, `payload_for`, `get_body` (the
  three sequential loops), `effective_chart`, `stored_chart_sql`;
  `routes/support.rs` `run_spec_sql`, `try_run_spec_sql`, `SpecRunFailure`,
  `annotate_table`, `annotate_grain`; `routes/layers.rs` `RoleRunner`,
  `LayerContext`; `routes/embed.rs` `render_board_payload` (its own
  sequential loop); `routes/dashboard_sources.rs` and `routes/metrics.rs`
  (other `try_run_spec_sql`/`run_spec_sql` callers); the render route of
  `BI-1` calls `payload_for`.
- Policy: `policy_engine.rs` `rewrite_sql_for_roles`,
  `PolicyEngineObligations::from_state`; `sql_rewrite.rs`
  `PlaceholderValues`, `PRINCIPAL_ID_PLACEHOLDER`, `referenced_tables`.
- Engine: `lakehouse-clickhouse/src/lib.rs` `ChClient::new` (60 s client
  timeout), `query`, `query_with_settings`, `run_cancellable`.
- State and limits: `state.rs` `AppState` (`pdf_slots` pattern);
  `routes/mod.rs` `route_timeout`, `DEFAULT_REQUEST_TIMEOUT` (60 s),
  `timeout_middleware`, `AI_CHAT_TIMEOUT` (env read once).
- Storage: `lakehouse-bi/src/store.rs` `BOARD_COLS`, `BOARD_KEEP_COLS`,
  the `ALTER TABLE console.bi_board ADD COLUMN IF NOT EXISTS
  refresh_seconds` line; `lakehouse-store/src/reporting_settings.rs`
  `ReportingSettings`; `routes/settings.rs` `get_reporting`,
  `put_reporting`, `format_settings`; console
  `src/features/admin/reporting-settings-page.tsx`.
- Console: `src/features/dashboards/dashboard-page.tsx` `load` (one
  `apiFetch("/api/dashboard?...")` for the whole board), `useAutoRefresh`;
  tile footer in `tile-body.tsx`; data dialog in `tile-dialogs.tsx`.
- Assistant: `routes/ai/registry.rs` (`TOOLS`, count test,
  `write_tool_schema_fixture`), `routes/ai/tools/look.rs` (`format_chart`,
  the closest write tool), `src/features/copilot/capabilities.ts`,
  `page-context.ts`, `ops/ai_eval/dashboard_cases.json`.
- Crates: `moka 0.12.16` in `rust/Cargo.lock` (dependency of `iceberg`),
  `futures 0.3.34`, `sha2`.

## 3. Tasks (one commit each, in order; Rust first)

### Part A: parallel tiles (ships alone if part B slips)

**T1 Concurrent tiles (D1, D2).** `get_body` and `render_board_payload`
run tiles through `buffer_unordered(6)` under `tile_slots`. *Accept:* a
`sqlx`-free unit test with a fake engine (a `wiremock` ClickHouse that
delays each answer 300 ms) shows 12 tiles finish in under 1.5 s and never
more than 6 requests in flight (count in the mock); results are identical
(same JSON) to the sequential run for a fixture board; `tile_slots` bounds
two concurrent boards to 24 in flight; `lakehouse-api` lib tests for
`dashboard` and `embed` green.

**T2 Part A measurement.** A script under `ops/bench/` (no secrets, reads
`API_URL` and a session from the environment) times `GET /api/dashboard`
for a named board N times and prints p50/p95; `docs/plans/BI-4-RESULT.md`
created with the before (on `feat/bi-phase-4`) and after numbers, machine,
data size, ClickHouse version. *Accept:* the document quotes the command
and its raw output; nothing estimated.

### Part B: remembered results (after `BI-5` T2)

**T3 Install rule and storage (D7).** Migration (next free number),
store read/write, `GET/PUT /api/settings/reporting` carry the two fields,
`settings:write` to change. Board `cache_json` column and the chart
`cache` field with validation. *Accept:* store round-trip test; range
refused with a sentence; route refuses without `settings:write`
(`route_policy`); the board column survives `boards_update` (keep-cols
test).

**T4 Cache module (D3–D6).** `result_cache.rs` in `lakehouse-api`:
`ResultCache`, key, weigher, fingerprint query, validity check;
`try_run_spec_sql` takes `TileEngine`; every caller updated. *Accept:*
unit tests: key differs for different masks, for different
`__principal_id__`, for different limits; equal for equal text; entry over
8 MB not stored; error results not stored; a changed fingerprint misses; an
unwatchable table expires at 10 minutes (mock clock); a failed fingerprint
read misses; two concurrent identical keys run one query (mock count);
`RESULT_CACHE_MAX_MB=0` never stores. Masking route test: user A (masked
column) then user B (unmasked) on one board, B's value unmasked and A's
masked, in both orders.

**T5 Payload and refresh (D8).** `computedAt`, `remembered`, `cacheRule`,
`fresh=1` on the dashboard, embed, public and render reads. *Accept:*
route test: second read `remembered: true` with the first read's
`computedAt`; `fresh=1` gives a new `computedAt`; the PDF render payload
carries the fields; SEC-11 guard green.

**T6 Assistant (D9).** *Accept:* tool test (approval required, rule set on
the board, refused without `dashboard:write`); fixture regenerated; count
pin; `capabilities.ts` test; AI-15 case added.

**T7 Console.** Tile footer "Data from HH:MM" (report time zone) and
"remembered"; Refresh passes `fresh=1`; "Refresh this tile" in the tile
menu; Admin > Settings "Remembered results" section; dashboard settings and
chart builder "Freshness" select. *Accept:* component tests for the footer
(remembered and fresh), the settings form (range errors), the builder field
round-trip; typecheck, lint, tests of the touched files; webpack build.

**T8 Part B measurement.** `BI-4-RESULT.md` gains the remembered-results
numbers for the same board. *Accept:* as T2; the 3-second target written
as met or not met.

## 4. Out of scope

Cross-process cache (Redis or similar), scheduled pre-computation, Query
Studio and `run_sql` caching, streaming tiles to the page one by one,
Trino-engine tiles (none exist today; if one appears it is not cached).

## 5. Must verify, not assume

- That `system.parts.modification_time` moves on every insert and on a
  `CREATE OR REPLACE TABLE ... AS SELECT` (the Gold build's statement,
  `routes/lineage/builds.rs` `BUILDS_SQL` shows both kinds) on ClickHouse
  26.8; record the check in the RESULT document. If it does not, stop and
  report: decision 1 then needs another signal.
- Whether `referenced_tables` on the **rewritten** statement names the
  physical tables (masking may wrap them in subqueries or views).
- That `moka`'s `future` feature adds no crate to `Cargo.lock`
  (`cargo tree -e features -i moka`); `cargo deny check` and `cargo audit`
  green.
- That `buffer_unordered` over futures borrowing `&cols`, `&sources`,
  `&read` compiles without cloning the large maps; if not, report the
  clone cost rather than spawning.
- That no tile path calls `ch.query` directly around `try_run_spec_sql`
  (grep `ch.query(` and `.query_limited(` under `routes/dashboard*`,
  `embed.rs`, `layers.rs`, `support.rs`); a direct call is a finding.

## 6. Checks

As in the phase 4 plans (same cargo environment:
`CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-uiux-target CARGO_BUILD_JOBS=2
CARGO_INCREMENTAL=0`, one cargo command at a time, `--test-threads=4`,
`route_auth` at 2; check `free -g` / `df -h /` first; never `cargo build -p
lakehouse-api`; no `rust/target`; no prettier; touch the crate sources
before clippy when several worktrees share the target dir). Per task the
crates touched (`lakehouse-store`, `lakehouse-bi`, `lakehouse-api`), the
`lakehouse-api` lib filters for the modules touched (`dashboard`, `embed`,
`support`, `result_cache`, `settings`, `ai`), the integration files
`route_auth`, `route_policy`, `sec11_guard`, `reporting_settings_route`,
`ai_tool`; `cargo deny check` and `cargo audit` after T4 (dependency
added). Once, before each handoff: `cargo fmt --check` and whole-workspace
`cargo clippy --workspace --all-targets --all-features -- -D warnings` with
the environment above (no full workspace test run on this machine; CI runs
it on the PR). TS: typecheck, lint, tests of the files touched; the webpack
build after T7.

## 7. Handoff (developer appends)

### Part A (parallel tiles), developer: Sonnet

Commits on `feat/bi-phase-5`: T1 `22b34c23` (tiles side by side), then the fix
commit and the T2 commit that follow it in `git log` (test fixture fix, clippy
allows, `ops/bench/dashboard_open_bench.py`, `docs/plans/BI-4-RESULT.md`).

What changed: `src/tile_pool.rs` (`run_tiles`, 6 per request; `tile_slots`
semaphore, `TILE_CONCURRENCY_MAX`, default 24, clamped 4..=128);
`AppState.tile_slots`; `get_body` and `embed::render_board_payload` run each
tile as one future; the wrong "parallel tile execution" allow reason on
`get_body` is corrected.

Deviations from the plan (the planner decides):
- `FuturesOrdered` refilled by hand (= `buffered(6)`) instead of
  `buffer_unordered(6)`: it keeps the tile order, so the payload JSON is
  identical. A stream-combinator closure made the handlers' futures fail the
  `Send` check, hence the hand-written loop.
- `get_body` gets the semaphore as a fourth tuple member (it was at the
  argument limit); its test callers pass a fresh 24-permit semaphore.
- The built-in KPIs/charts and stored charts now run in one batch, after
  `sources_for`: a `sources_for` failure now skips the built-in queries
  instead of running them first. Output is unchanged.
- The plan's `wiremock` test measures the call as a whole (under 1.5 s) and
  the arrival of the first six queries; it does not count requests in flight
  in the mock. The in-flight bound is proved in `tile_pool::tests` with a paused
  clock (peak 6; two boards peak at the process bound).
- "Results identical to the sequential run" is checked as payload key order
  and per-tile values on a fixture, not against a run of the removed old code.
- `#[allow(clippy::too_many_lines)]` with reasons on `render_board_payload`
  and `AppState::new` (the second crossed 100 by one line).

Commands run (this VM, shared target, `CARGO_BUILD_JOBS=2`, `CARGO_INCREMENTAL=0`):
- `cargo check -p lakehouse-api --lib --tests`: exit 0.
- `cargo test -p lakehouse-api --lib -- dashboard embed tile_pool --test-threads=1`:
  229 passed, 0 failed, 3 ignored (before the clippy allows were added).
- `cargo test -p lakehouse-api --lib -- twelve_slow_tiles --test-threads=1`:
  1 passed (0.70 s).
- `cargo fmt --check`: clean on the final tree.

Verified after the allows (second round):
- `cargo test -p lakehouse-api --lib -- dashboard embed tile_pool --test-threads=1`:
  229 passed, 0 failed, 3 ignored.
- `cargo clippy --workspace --all-targets -- -D warnings`: exit 0 (without
  `--all-features`).
- `cargo test -p lakehouse-api --test dashboard_export -- --test-threads=1`:
  10 passed, 0 failed.

NOT verified (disk fell under 9 GB and the run was stopped, again):
- `cargo test -p lakehouse-api --test embed_tokens`, `--test route_auth` and
  `--test sec11_guard`: not run. No route, policy-table entry or error path
  changed, so a break is not expected, but that is an expectation, not a result.
- Clippy with `--all-features` (the AGENTS.md block): not run.
- Real-engine before/after timing: not measured; see `docs/plans/BI-4-RESULT.md`.

Plan section 5 grep (direct engine calls around `try_run_spec_sql`), done
with `grep -n "\.query\w*("` over `routes/dashboard*.rs`, `embed.rs`,
`layers.rs`, `support.rs`, `metrics.rs`, `render.rs`, `params.rs`:
- Tile path (`get_body`, `render_board_payload`, `layers.rs`, `annotate_*`):
  no direct call; all go through `run_spec_sql`/`try_run_spec_sql`
  (`support.rs`, the one `.query(&rewritten, None)` inside it).
- Direct calls found, none on the dashboard-payload path, each after its own
  `rewrite_sql_for_roles` (or the SQL-source rewrite), so part B must decide
  for each whether it is cached or stays fresh:
  `dashboard.rs` `records_for_roles` (`ch.query` rows and `ch.rows` count, the
  drill-through/table paging read), `dashboard.rs` chart export
  (`query_stream`, `EXPORT_TIMEOUT`), `dashboard_sources.rs` SQL-source preview
  (`ch.query` on the editor's unsaved SQL). They are findings for D3, not
  defects in part A.

### Part A after merging `origin/feat/bi-phase-4b` (merge `224a0ee2`)

Resolution (details in the merge commit body): `render_board_payload` keeps
BI-26's `only_chart` and `editable` and part A's pool; `time` and `tile_slots`
travel as one tuple to stay under the 8-argument clippy threshold; the pooled
tile loop uses 4b's `rendered_chart`; a chart token's single tile goes through
the pool. `public_links.rs` calls neither `get_body` nor `render_board_payload`.

Run after the merge:
- `cargo test -p lakehouse-api --lib -- embed dashboard tile_pool --test-threads=1`:
  232 passed, 0 failed, 3 ignored.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: one
  error, `too_many_lines` (103/100) on 4b's test
  `a_board_saved_with_a_typed_filter_renders_publicly_with_it_applied`, grown by
  the tile-slot tuple in its call. Fixed with an `allow` and a reason; clippy
  NOT re-run after the fix.
- NOT run (disk fell to 8.4 GB, under the 9 GB floor): `embed_tokens`,
  `route_auth`, `sec11_guard`, `public_links` integration files, and the clippy
  re-run.

### Part B (server and console as built), developer: Sonnet

Commits on `feat/bi-phase-5`: T3 `e6c821da` (+ `0e833172` test literals), T4 `a971fbff`,
T5 `1190f5a5`, T6 `e0076536`, T7 `6fbe1fe7`, T8 the BI-4-RESULT part B commit that follows.
`origin/feat/bi-phase-4b` merged twice (`dbdb6638`, `995c1d24`).

**Migration number: `0110_result_cache_rule.sql`** (not 0103: BI-19 part B takes 0103+; 0098-0102 taken).

**Deviations (the planner decides):**
1. **D3, the scope is a task-local, not a `TileEngine` parameter.** `result_cache::CacheScope`
   is opened by `payload_for` (dashboard, PDF print view) and the two embed/public handlers
   with `scoped(..)`, narrowed by `get_body` / `render_board_payload` to the board's rule and
   by `in_chart` to the chart's. `try_run_spec_sql` reads it with `current()`. Reason: about
   twenty call sites (and every test) would change, and the safe default is better: a call
   with no scope (Query Studio, preview, export, SQL-source preview, `records_for_roles`) is
   never remembered. The key, the choke point and "no caller can read around it" are as D3/D4.
2. **D6, "one fingerprint query per request"** holds when the board's marts are known: `get_body`
   and `render_board_payload` pass every `serving.<mart>` of the board as candidates, so the
   first tile's fingerprint read covers them all (test: 1 query for 3 concurrent tiles). A
   table outside the candidates costs one more query. A table with no active part (empty table)
   is absent from `system.parts` and so counts as unwatchable (10 minutes).
3. **`fresh` is not read from embed or public-link requests** (plan T5 says all four reads): a
   reader with no sign-in could otherwise force the engine to recompute a board. The PDF render
   reads through `payload_for` with no `fresh`. Planner to confirm.
4. **`fresh=<chart id>[,id]`** (besides `fresh=1`) exists for "Refresh this tile"; the plan had
   no per-tile read.
5. **`set_cache_rule` is `WriteLow`** (the chat's confirmation, like `format_chart` and
   `set_dashboard_theme`), not `WriteHigh`: the plan says "write risk class, needs approval" and
   the feature page says "asks for approval like the other dashboard writes", which are
   confirmations. Change the `ToolSpec` risk if an approval queue is meant.
6. Under an explicit `minutes` rule the unwatchable bound is the person's minutes (they asked
   for N); under the default it is 10 minutes.
7. A version restore does not carry the dashboard's rule (`BoardFields` unchanged); it keeps the live one.
8. `query_metric` / `value_query` open their own scope under the install's rule (one more
   settings read); it is boxed because the future crossed the `large_futures` size.
9. Extra payload key `boardCacheRule` (the board's own rule) for the Freshness dialog.
10. A stored board rule that does not parse reads as "always fresh" (fail closed).

**What never reaches memory:** errors (time limit, Stop, engine), a result the row cap cut
(`limitHit`: BI-5 D6), over 8 MB, a statement whose tables could not be named, anything computed
while the fingerprint read failed. A cancelled run inserts nothing (test:
`a_cancelled_computation_leaves_nothing_behind`). The key is SHA-256 of engine, the rewritten
statement (which carries the person's masks and expanded principal values) and
`max_execution_time` / `max_result_rows`; the text also carries the row cap and the time cap.
**BI-5 review B1** (done in `f3db9aad`): the per-kind cap (`map_points` for point maps, the pivot's own cap) moves the outer `max_result_rows`, and that number is in the key.

**Commands run** (from `rust/`, shared target, `CARGO_BUILD_JOBS=2`, `CARGO_INCREMENTAL=0`,
under `flock`; 1.99.1):
- `cargo fmt --check -p lakehouse-bi -p lakehouse-store -p lakehouse-api`: clean.
- `cargo clippy -p lakehouse-bi -p lakehouse-store -p lakehouse-api --all-targets --all-features -- -D warnings`:
  clean for `lakehouse-api` after the 4b lint fixes were merged (last run at T7); the bi/store
  crates were clean at T3.
- `cargo test -p lakehouse-bi --lib cache`: 4 passed; `ensure_bi_table`: 1 passed.
- `cargo test -p lakehouse-api --lib -- result_cache`: 21 passed (incl. the fingerprint, key,
  coalescing, cancel and per-chart tests); with `routes::dashboard routes::embed routes::support
  tile_pool routes::settings routes::layout_tabs routes::organise`: 253 passed, 3 ignored (this
  includes the `sqlx::test` masking test `a_masked_and_an_unmasked_viewer_never_share_a_remembered_result`
  in both orders and `a_second_open_is_remembered_and_fresh_computes_again`, which ran here).
- `cargo test -p lakehouse-api --lib -- routes::ai routes::metrics routes::tests`: 432 passed, 1 ignored;
  the ignored `write_tool_schema_fixture` run once (count pin 109).
- `bun run typecheck`: exit 0. `bun run lint`: 0 errors, 9 warnings (none in these files).
  `bun run test`: 2650 pass, 1 skip, 0 fail (2651 tests, 276 files). `python3 -m unittest
  test_ai_eval` (from `ops/ai_eval`): 165 OK.

**Not verified:** every integration test file (`reporting_settings` in the store,
`reporting_settings_route`, the new `ai_tool` cases) was written, compiled by clippy
`--all-targets`, not run; `route_auth`, `sec11_guard`; `cargo deny` / `cargo audit`; the webpack
build; the 3-second target and any real-engine timing (BI-4-RESULT section 4.2); the console in a
browser; the AI-15 run itself (case `dash_make_chart_always_fresh` added, not run); a full
workspace clippy and test run. `system.parts.modification_time` was checked on ClickHouse
26.8.9.10 (BI-4-RESULT 4.1): it moves on insert and on `CREATE OR REPLACE TABLE ... AS SELECT`.

## 8. Review (planner appends)
