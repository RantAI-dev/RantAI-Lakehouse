# `DATA-66`: Iceberg sync per table (on every update, on a schedule, manual)

Planner and reviewer: Claude Opus. Developer: a non-Opus agent (`AGENTS.md`).
Base: `origin/main` at `b6617a3e7`. Branch: `t3/realtime-iceberg-sync-settings`
(this plan's commit is on it; build on top of it). One PR slice.
Spec: `docs/core/specs/data-66.md`. Feature page:
`docs/core/features/iceberg-sync-per-table.md`.

Every anchor below was read at `b6617a3e7`. If the code you find does not
match an anchor, or a step cannot be done as written, **STOP and report the
mismatch in the Handoff section**. Do not improvise a different design.

## 1. The problem

A switched-on publication (`gold_publication`, keyed by `(zone, mart)` since
`DATA-65`) is exported to Iceberg by `gold_export_job`, which fans out over
**every** enabled publication (`dagster/dispar_orchestrate/gold_export.py:484`
`list_gold_marts`). That job runs (a) after **any** `authored__*` job succeeds
(`gold_export_after_authored_sensor`, `gold_export.py:1263`, body
`evaluate_authored_success` :1250) and (b) daily at 04:00
(`gold_export_schedule` :1280). It is a serial job (`ingest_factory.py:337`
`SERIAL_JOB_NAMES`), so one pipeline's table waits behind every other table.
A change made to `silver.*`/`serving.*` outside a pipeline waits for 04:00.
There is no per-table choice.

## 2. What the code allows (read before building)

- **M1. No Rust code writes `silver.*`/`serving.*`.** Authored pipelines
  write in Dagster: `authored_factory.py` op `_run` (the op built by
  `_op_for_pipeline`, :585), `write_incrementally(...)` at :728, then
  `AssetMaterialization(asset_key=f"{target_zone}.{target_table}")` at
  :756-761. So the "right after the write" hook is in that op, after :761.
- **M2. The API already launches Dagster runs with config and tags**:
  `DgClient::launch_run_with_config_and_tags`
  (`rust/crates/lakehouse-dagster/src/lib.rs:1515`), used by
  `gold_latest_copy::launch` (`lakehouse-api/src/gold_latest_copy.rs:161`).
- **M3. The export route is the one place every publish goes through**
  (`routes/gold.rs:514` `export`): tenant gate (`gated_source_table` :425),
  `ifChanged` skip via the `DATA-10` fingerprint (:190), per-`(zone, mart)`
  single-flight lock (`zone_lock_key` :171, 409 when held), `DATA-63`
  single-commit export, history row, then the `DATA-6` latest-copy launch
  (`latest_copy_answer` :693). **This plan does not change that route.** Every
  sync, whatever its trigger, is a Dagster run that POSTs to it with
  `ifChanged=true` (`export_one_mart`, `gold_export.py:300`) and then trims
  (`trim_gold_mart` :1124, `DATA-2`). So `DATA-2`, `DATA-6`, `DATA-63`,
  `DATA-64` and `SEC-11` hold by construction.
- **M4. The `DATA-10` fingerprint is a full scan** (`count`, `sum(cityHash64(*))`,
  `read_content_fingerprint` :889). It must not run every 15 s per table. The
  watcher uses the cheap `system.parts` `max(modification_time)` signal
  (`mart_last_changed_at` :1325) to decide a run is *worth starting*; the run's
  `ifChanged=true` then makes the exact decision. A background merge moves
  `modification_time`, so it can start a run that skips; the run key (D8)
  keeps that to one run per change signal.
- **M5. `report_schedule::Schedule`** (`lakehouse-store/src/report_schedule.rs:85`)
  already has `Hourly{minute}` and `Daily{hour,minute}`, IANA zones via `jiff`,
  `next_run_after` :257 and `zone_is_known` :300. It has no "latest slot at or
  before now"; T3 adds it there.
- **M6. `console.gold_export_run`** (`gold_export_history.rs:115-147`) has
  `mart, zone, status, started_at, finished_at`. A skipped `ifChanged` export
  writes no row. Runs are written on completion.

## 3. Decisions

Signed by the product owner on 2026-11-27: D1–D3. The rest are the planner's,
listed on the feature page for the owner to confirm; build them as written.

- **D1.** ClickHouse stays primary. Every sync is the existing export route
  plus trim plus latest copy (M3). Nothing here writes Iceberg another way.
- **D2. Modes**, stored per `(zone, mart)` in `gold_publication`:
  `on_update` | `scheduled` | `manual`. JSON (camelCase) values are exactly
  those strings: `syncMode: "on_update"`.
- **D3. A sync never fails the ClickHouse write.** The authored op's call
  (T6) is wrapped so no exception, status or timeout from it can fail the op.
- **D4. Defaults.** Migration: every existing row gets `on_update`. A row
  created by switching on gets `on_update`. When the user first picks
  `scheduled`, the console pre-fills daily 04:00 in `browserTimeZone()`
  (`src/components/patterns/time-zone-select.tsx:10`; fall back to `"UTC"`
  when it returns null).
- **D5. Schedule shape.** `Hourly{minute}` or `Daily{hour,minute}` from
  `report_schedule::Schedule`, plus an IANA `timeZone` checked with
  `zone_is_known`. Weekly/Monthly are refused (400). JSON:
  `schedule: {"kind":"hourly","minute":15}` or
  `{"kind":"daily","hour":4,"minute":0}`, and `timeZone: "Asia/Jakarta"`.
- **D6. Per-table runs.** A new Dagster job `iceberg_sync_job` exports and
  trims **one** `(zone, mart)` per run, given by run config. It is **not** in
  `SERIAL_JOB_NAMES`. Instead each run carries the tag
  `lakehouse/iceberg-sync-target = "<zone>.<mart>"` and `dagster/dagster.yaml`
  limits that tag to 1 per unique value. So two tables sync in parallel, and
  two syncs of one table queue (never dropped). The API's own lock (M3) stays
  as the second guard; a 409 is a skip, as in `export_gold_mart` :567-573.
- **D7. Immediate trigger.** After the authored op's write (M1), the op POSTs
  `/api/gold/export/{target_table}/after-write?zone=<target_zone>` (new, T4).
  The API answers at once: when the publication is enabled and `on_update`, it
  launches `iceberg_sync_job` for that table (trigger `pipeline`); otherwise it
  launches nothing and says why. Rejected alternative: hooking
  `run_finished_event` (`routes/pipelines.rs:2880`) — it is reached through a
  30 s run-status sensor (`pipeline_events.py:254`), the delay the owner asked
  to remove.
- **D8. Watcher and schedule: one sensor, logic in Rust.** New route
  `GET /api/gold/sync-due` (T3) returns the runs that are due now, each with
  a deterministic `runKey`. New sensor `iceberg_sync_sensor`
  (`minimum_interval_seconds=15`, `default_status=RUNNING`) turns each into a
  `RunRequest(run_key=runKey, ...)`; Dagster drops a `run_key` it has seen, so
  each key starts at most one run. Rules, per **enabled** publication:
  - `manual`: never due.
  - `on_update`: let `changed` = the table's `max(modification_time)` over
    active parts (`system.parts`, one query for both schemas); `newest` = the
    newest run row of `(zone, mart)`; `lastOk` = the newest `success` row.
    If the table has no active parts: not due. Due with trigger `change` and
    key `change:<zone>:<mart>:<changed as unix seconds>` when there is no
    `newest`, or `changed >= newest.started_at`. Otherwise due with trigger
    `retry` and key `retry:<zone>:<mart>:<newest.finished_at as unix ms>` when
    `newest.status == "failed"` and `now - newest.finished_at >= 300 s`.
    (`lastOk` is not needed by the rule; do not compute it.)
  - `scheduled`: `slot` = the latest slot at or before `now` (T3). Due with
    trigger `schedule` and key `schedule:<zone>:<mart>:<slot as unix seconds>`
    when `slot >= publication.updated_at` (a schedule set at 10:30 first fires
    at the next slot, not the 10:00 one).
  The route answers `{"due":[{"zone","mart","trigger","runKey"}]}` with `zone`
  as `PublishZone::namespace_name()` (`"silver"`/`"gold"`).
- **D9. Retirements.** Remove `gold_export_after_authored_sensor` and
  `evaluate_authored_success` (D7 replaces them). Keep `gold_export_job` and
  its 04:00 schedule, but `list_gold_marts` now yields **only**
  `GOLD_EXPORT_MARTS` entries that are not an enabled console publication
  (feature page decision 12). Console publications follow their mode only.
- **D10. Permissions.** Changing the mode or schedule is the existing PUT,
  `Policy::RequiresPermission("gold:export")`. `after-write` is
  `Policy::RequiresAuth` plus `check_export_token` (the same pair as
  `POST /api/gold/export/{mart}`, policy.rs:566 and gold.rs:285).
  `sync-due` is `Policy::RequiresAnyPermission(&["gold:read","gold:export"])`
  plus `check_export_token` in the handler (the pair `publications` uses,
  policy.rs:584 and gold.rs:1516). The `gold-export-scheduler` identity
  already holds what both need; verify, do not add grants.
- **D11. Errors.** A failed Dagster launch in `after-write` is logged through
  `upstream_error` (the raw text to the log under a reference id) and answers
  `200 {"launched":false,"reason":"not_started"}`: the write happened, the
  watcher retries (`change` key). A ClickHouse/Postgres failure in either new
  route is classified (`upstream_error::ch_error`, `ApiError::from(StoreError)`),
  never raw text (`SEC-11`, `tests/sec11_guard.rs`).
- **D12. Constants** (each cited at its definition to the feature page
  decision number): watcher interval 15 s (Python), retry backoff 300 s
  (Rust), the job name `iceberg_sync_job` (Rust `ICEBERG_SYNC_JOB` mirrored in
  Python, like `GOLD_LATEST_COPY_JOB`), the op name `iceberg_sync_target`, the
  tag keys `lakehouse/iceberg-sync-target` and `lakehouse/iceberg-sync-trigger`
  (do **not** reuse `lakehouse/trigger`, which the run report reads with other
  values), trigger values `pipeline | change | retry | schedule`.

## 4. Tasks

Order keeps Rust together (T1–T4), then Python (T5–T6), then TypeScript (T7),
then docs (T8–T9). One commit per task. Push after every commit.

### T1. Migration `0162` and the store

- `rust/migrations/0162_data66_iceberg_sync_mode.sql`, with a why-header
  (style of `0157`):
  `sync_mode TEXT NOT NULL DEFAULT 'on_update' CHECK (sync_mode IN ('on_update','scheduled','manual'))`,
  `sync_schedule_kind TEXT NULL CHECK (sync_schedule_kind IN ('hourly','daily'))`,
  `sync_hour SMALLINT NULL CHECK (sync_hour BETWEEN 0 AND 23)`,
  `sync_minute SMALLINT NULL CHECK (sync_minute BETWEEN 0 AND 59)`,
  `sync_time_zone TEXT NULL`, and a table CHECK that `scheduled` ⇔ kind,
  minute and zone are all set, and `daily` ⇒ hour set, `hourly` ⇒ hour null.
- `lakehouse-store/src/gold_publication.rs`: add `SyncMode` (enum with
  `as_str`/`parse`), and `SyncSetting { mode, schedule: Option<Schedule>, time_zone: Option<String> }`;
  add the fields to `GoldPublication` (:50) and `PublicationRow` (:192).
  `upsert_in_zone` (:151) keeps its signature and **must not change** sync
  columns of an existing row. New `set_sync_in_zone(pool, zone, mart, &SyncSetting, updated_by)`
  that updates the row (and `updated_at`) and returns `Ok(None)` when no row
  exists; it rejects Weekly/Monthly and an unknown zone with a `StoreError`
  variant the API maps to 400 (find the existing validation variant; do not
  add a second one if one fits — rule 4).
- **Check:** store tests in `lakehouse-store/tests/gold_publication.rs`:
  a row inserted before `0162` reads back `on_update` (insert with the
  column omitted); scheduled round-trip; `upsert_in_zone` keeps a scheduled
  setting; the CHECK refuses `scheduled` without a minute (raw insert);
  `set_sync_in_zone` on a missing row is `None`.
  `cargo clippy -p lakehouse-store --all-targets -- -D warnings`,
  `cargo test -p lakehouse-store --test it gold_publication::`.

### T2. API: read and write the setting

- `PublicationBody` (`routes/gold.rs:1608`) gains optional `syncMode`,
  `schedule`, `timeZone`. `enabled` stays required (old clients send only it).
  In `set_publication` (:1658): after `upsert_in_zone`, when `syncMode` is
  present call `set_sync_in_zone`. Rules: `scheduled` needs `schedule` and a
  known `timeZone`; `on_update`/`manual` clear both (send them → 400).
  Fixed 400 messages, for example `"a scheduled sync needs a schedule and a
  time zone"`, `"unknown time zone"`, `"schedule kind must be hourly or daily"`.
- `publication_body` (:1385) and the `publications` list (:1516) add
  `syncMode`, `schedule` (null unless scheduled), `timeZone` (null unless
  scheduled), and `nextSyncAt` (RFC 3339, `next_run_after(zone, now)`, null
  unless scheduled and enabled). A missing row answers `syncMode: "on_update"`
  (D4: the mode a table gets when switched on).
- The `gold.publication_set` audit (`record_publication_audit` :1619) records
  the mode and schedule.
- **Check:** handler tests: PUT with only `{enabled:true}` keeps an existing
  scheduled setting; PUT scheduled without timeZone → 400 with the fixed
  message; PUT weekly → 400; GET returns `nextSyncAt` for scheduled; a user
  without `gold:export` still gets 403 on PUT (existing test still passes).

### T3. API: `GET /api/gold/sync-due` and the latest slot

- `report_schedule.rs`: `pub fn last_run_at_or_before(self, zone, now) -> Result<OffsetDateTime, ScheduleError>`
  for every variant: start from `next_run_after(zone, now - 32 days)` and step
  with `next_run_after` while the next is `<= now`. (Hourly: at most 768
  steps; fine — but for Hourly start from `now - 2 hours` and for Daily from
  `now - 2 days`, and use 32 days only for Weekly/Monthly.) Unit tests: hourly
  at :15 with now = 10:14 → 09:15; daily 04:00 Jakarta; a daily time in the
  skipped hour of a DST change (`America/New_York`, 02:30 on the spring day)
  agrees with what `next_run_after` produced for that day.
- New module `lakehouse-api/src/iceberg_sync.rs` with a module doc (why, D7,
  D8, M4): the pure function
  `due_syncs(now, publications: &[DuePublication], changes: &HashMap<(zone, mart), OffsetDateTime>, newest_runs: &HashMap<(zone, mart), NewestRun>) -> Vec<DueSync>`
  implementing D8 exactly, and the two reads: one `system.parts` query
  (`active`, both source schemas from `PublishZone::source_ch_schema`, grouped
  by `database, table`, `max(modification_time)`), one
  `console.gold_export_run` query (`argMax` of status/started_at/finished_at
  by `started_at`, grouped by `zone, mart`; call
  `ensure_gold_export_run_table` first). Schema names are server-controlled
  identifiers; values bound or `SqlLiteral`.
- Route `GET /api/gold/sync-due` (handler in `routes/gold.rs`, next to
  `publications`), `check_export_token` first. Register it in `routes/mod.rs`
  beside :1512; `POLICY_TABLE` entry beside policy.rs:584 with a comment;
  `policy.rs` test beside :2042-2084; `tests/route_auth.rs` asserted both
  ways (a dashboard viewer refused, the scheduler identity / `gold:export`
  accepted), following :1940-1944.
- **Check:** unit tests of `due_syncs` for every rule in D8: manual never;
  on_update never run → change; change after newest start → change; no change,
  newest failed 299 s ago → nothing, 300 s → retry with the right key; no
  active parts → nothing; scheduled slot before `updated_at` → nothing, at or
  after → schedule key; disabled → nothing. Route test with wiremock
  ClickHouse. `cargo test -p lakehouse-api --lib iceberg_sync` and
  `cargo test -p lakehouse-api --test it route_auth::`.

### T4. API: `POST /api/gold/export/{mart}/after-write`

- Handler `after_write` in `routes/gold.rs`: `check_export_token`, `parse_zone`
  (accepts `serving`), `Ident::new(mart)`, the same tenant gate as `export`
  (`gated_source_table` with `gold_export_gate_actor`), then read the
  publication. Answer `200 {"launched":false,"reason":R,"runId":null}` with
  `R` = `"publishing_off"` (no row or disabled), `"scheduled"`, `"manual"`.
  For `on_update`: launch `ICEBERG_SYNC_JOB` with
  `{"ops":{"iceberg_sync_target":{"config":{"zone":<namespace_name>,"mart":<mart>,"trigger":"pipeline"}}}}`
  and tags `lakehouse/iceberg-sync-target=<zone>.<mart>`,
  `lakehouse/iceberg-sync-trigger=pipeline`. Success → `{"launched":true,"reason":null,"runId":"…"}`.
  Launch failure → D11. No Dagster client configured → same as launch failure.
- Constants and the run-config builder live in `iceberg_sync.rs` (T3) so the
  route and its test share them.
- Route in `routes/mod.rs`, `POLICY_TABLE` `("POST","/api/gold/export/{mart}/after-write", Policy::RequiresAuth)`
  with a comment naming D10, policy test, `tests/route_auth.rs` both ways
  (anonymous refused; the run token accepted), and `audit_coverage.rs`
  (:273-274 lists the gold writes: add it there as a launch with no data
  change, following how `export` is listed).
- **Check:** handler tests with a wiremock Dagster: on_update → one
  `launchRun` whose variables carry the config and both tags; scheduled and
  manual → no request to Dagster; Dagster answers a `PythonError` → 200
  `not_started` and the response body does not contain the error text.

### T5. Dagster: `iceberg_sync_job` and `iceberg_sync_sensor`

- New `dagster/dispar_orchestrate/iceberg_sync.py` (narrative module
  docstring, `from __future__ import annotations`):
  - op `iceberg_sync_target` (config `zone`, `mart`, `trigger`) returns a
    `MartExportTarget(mart, zone)` (`gold_export.py:396`); refuse an unknown
    zone with `Failure(allow_retries=False)`.
  - `iceberg_sync_job` = `trim_gold_mart(t, export_gold_mart(t))` with
    `t = iceberg_sync_target()`, tagged `job_tags("iceberg_sync_job", ...)`
    (run limit, **not** serial: do not add it to `SERIAL_JOB_NAMES`).
  - `evaluate_sync_due(cfg)` → `GET {api}/api/gold/sync-due` with
    `_headers(cfg)`, `timeout=`, `raise_for_status()`; a malformed body is a
    `Failure`. Each item → `RunRequest(run_key=item["runKey"], run_config=…,
    tags={…both tags…})`. With no run token configured, yield one `SkipReason`
    saying so (the export would 401; the export page already says
    "automatic publishing needs the export run token").
  - `iceberg_sync_sensor`: `@sensor(job=iceberg_sync_job,
    minimum_interval_seconds=15, default_status=DefaultSensorStatus.RUNNING)`
    with the RUNNING comment the other sensors carry.
- `dagster/dagster.yaml`: add a `tag_concurrency_limits` entry for
  `lakehouse/iceberg-sync-target`, `applyLimitPerUniqueValue: true`,
  `limit: 1`, with a comment (D6).
- `gold_export.py`: delete `gold_export_after_authored_sensor`,
  `evaluate_authored_success` and their comment block (from the `PR slice C review C-S1` comment at :1226 to the sensor at :1277); change
  `list_gold_marts` per D9 and rewrite its docstring and the schedule comment
  to say so. `definitions.py` (:28-31, :141, :153): register the new job and
  sensor, drop the removed sensor.
- **Check:** `test_iceberg_sync.py` (no network; `requests` patched): sensor
  turns two due items into two `RunRequest`s with the right keys, config and
  tags; empty list → `SkipReason`; malformed body → `Failure`; no token →
  `SkipReason`; the job's ops are the trim and export ops. Update
  `test_gold_export.py`: remove the sensor tests (:514 onward) and add one that
  `list_gold_marts` drops an env mart that is an enabled publication.
  A definitions test that the sensor is registered RUNNING at 15 s.
  `(cd dagster && python -m pytest dispar_orchestrate -q)`.

### T6. Dagster: the authored op asks for its sync

- In `authored_factory.py`, after the `AssetMaterialization` (:756-761) and
  before `return`, call `request_iceberg_sync(target_zone, target_table, log)`
  (put the helper in `iceberg_sync.py`). It POSTs
  `{api}/api/gold/export/{table}/after-write?zone={zone}` with `_headers`,
  `timeout=10`. It **never raises**: any `requests` exception or non-2xx is
  logged with `context.log.warning` (status code and the fixed reason only),
  and a missing run token is logged once at info and skipped (D3).
  `GoldExportConfig.from_env()` failing (missing env) is caught the same way.
- **Check:** tests in `test_authored_factory.py` (the file's existing op
  harness): the op returns its normal result when the POST raises
  `ConnectionError`, answers 500, or times out; the POST carries
  `zone=serving` for a Gold target and `zone=silver` for a Silver target; no
  POST when the token is unset.

### T7. Console

- `src/services/contracts/gold.ts`: `GoldSyncMode = "on_update" | "scheduled" | "manual" | string`,
  `GoldSyncSchedule = { kind: "hourly"; minute: number } | { kind: "daily"; hour: number; minute: number }`;
  `GoldPublication` (:198) gains `syncMode`, `schedule`, `timeZone`,
  `nextSyncAt`. `GoldService.setPublication` gains an optional
  `sync?: { syncMode; schedule?; timeZone? }` argument; the client
  (`clients/gold.ts:75-105`) sends it in the PUT body. No mock.
- `open-format-card.tsx`: below the status line, when the publication is
  enabled, a **Sync** row: a `Select` ("On every update", "On a schedule",
  "Manual only"), disabled without `canEdit` (the existing `gold:export`
  sentence covers why) or while saving. Picking "On every update" or "Manual
  only" saves at once (`enabled: true` plus the mode). Picking "On a schedule"
  opens an inline editor: frequency (Hourly/Daily), minute (hourly) or
  time (daily), the existing `TimeZoneSelect`, prefilled per D4, and a "Save
  schedule" button. One line under it, by mode:
  - on_update: "Synced after each pipeline write, and checked every 15 seconds for other changes."
  - scheduled: "Next sync: <formatDateTime(nextSyncAt)> (<timeZone>)."
  - manual: "Synced only when you press Publish now."
  Errors render as the toggle error does (`role="alert"`). Use
  `useServiceAction`; reload the publication after a save.
- `src/features/gold/gold-exports-page.tsx`: a "Sync" column with the same
  three labels (put the label map in `src/lib/gold-freshness.ts` next to
  `publicationLabel`, one place).
- **Check:** tests in `open-format-card.test.tsx`: each mode's line; picking
  manual sends `{enabled:true,syncMode:"manual"}`; saving a schedule sends
  kind/minute/hour/timeZone; a user without `canEdit` sees the select
  disabled; the Silver block sends `?zone=silver`; `gold-exports-page.test.tsx`
  shows the column. `bun run typecheck && bun run lint && bun run test`.

### T8. Docs

- `CHANGELOG.md`: a customer-readable entry (sync modes; the nightly run of
  switched-on tables ends; `GOLD_EXPORT_MARTS` unchanged unless the table is
  switched on in the console).
- `docs/core/features/gold-publish-per-mart.md`: in "Limits to tell a
  customer", replace "The open copy is current only after the publish that
  follows a pipeline run." with a pointer to `DATA-66`.
- Check that no doc still says publishing runs after *any* pipeline or
  nightly for console tables (`grep -rn "04:00\|after_authored" docs`); fix
  what you find, naming `DATA-66`.

### T9. Measure (spec targets)

On the compose stack (`docker compose --profile '*' up`), with a 100,000-row
Gold table set to "On every update": time from the authored op's write
(`AssetMaterialization` timestamp) to the `success` row in
`console.gold_export_run`, 10 runs, and the same for an `INSERT` made with
`clickhouse-client` outside a pipeline. Write
`docs/plans/DATA-66-RESULT.md` (commit, machine, p50/p95, raw numbers). If you
cannot run the stack, write the file saying **not measured** and why; never
estimate (principle 2).

## 5. PR slicing

One PR from this branch. The planner opens it after review.

## 6. Out of scope

Row-level streaming to Iceberg; a setting on the pipeline page; cron;
changing the export route, the latest-copy job or the trim; Weekly/Monthly
schedules; the assistant.

## 7. Verification before handoff

The block in `AGENTS.md` for Rust, TypeScript, Python and compose
(`docker compose --profile '*' config --quiet`; `dagster.yaml` is not compose,
but the stack must start: T9's `up` counts as the real `up`). Use
`cargo-locked` (`~/bin` on the planner VM) if present, the shared `CARGO_TARGET_DIR`, and never
`cargo clean`. Quote every command and its counts in the Handoff.

## Handoff

Developer (DeepSeek V4.1 Flash, 2026-11-27), branch
`t3/realtime-iceberg-sync-settings`, built on `958cd7808`.

### Commits

- `c04765678` DATA-66 T1: store the per-publication sync mode and schedule
- `a35d7f70d` DATA-66 T2: read and write the sync setting over the publication routes
- `f763b35aa` DATA-66 T3: the due-sync decision, its reads and GET /api/gold/sync-due
- `9b0cbc3d7` DATA-66 T4: POST /api/gold/export/{mart}/after-write
- `a8e003544` DATA-66 T5: the per-table sync job, its watcher, and the nightly run's new scope
- `e72f0c5e0` DATA-66 T6: the authored op asks for its own table's sync after the write
- `829092427` DATA-66 T7: the Sync setting on the publish card and the all-marts page
- `bda7ed35a` DATA-66 T8: the customer-facing changelog and the old publish limit
- `91edb72b4` DATA-66 T9: the latency measurement is not measured, and why

All pushed to `origin/t3/realtime-iceberg-sync-settings`, never
force-pushed; no pull request was opened, edited or merged.

### Verification (final commit `91edb72b4`, foreground)

Rust, through `~/bin/cargo-locked` with the shared
`CARGO_TARGET_DIR=/home/shiro/rantai/cargo-target` and no
`cargo clean`:

- `cd rust && cargo fmt --check` — pass.
- `~/bin/cargo-locked clippy --workspace --all-targets --all-features -- -D warnings` — pass (finished in 6m54s).
- `~/bin/cargo-locked test --workspace` — **8,841 passed, 0 failed, 38 ignored**, across 40 test-result groups (exit 0). This includes the new store tests (`gold_publication::` 12), the `iceberg_sync` unit tests (11), the route tests (`iceberg_sync::` 2, `gold_publication::after_write` 4, `route_auth::sec66` 1, `route_auth::the_after_write` 1), `routes::route_policy_tests::every_policy_table_entry` and `audit_coverage::` 30.

TypeScript:

- `bun run typecheck` — pass.
- `bun run lint` — pass, **13 warnings, 0 errors** (all pre-existing; none in a file this branch touched).
- `bun run test` — **5,518 pass, 1 skip, 1 fail**. The one failure is
  `src/features/queries/query-studio-download.test.tsx > the elapsed time of a run > ticks every second while the run is in flight` (fake-timer). It is **pre-existing and not caused by this branch**: the same test fails in a detached worktree of `origin/main` at `509a02088` with this machine's `bun 1.4.2` (the repo pins `bun@1.3.12`). It needs a backlog item; no file this branch touched is involved. The four files this branch changed pass (`open-format-card`, `gold-exports-page`, `gold-freshness`, `gold.client`: 63 pass).

Python (no Dagster venv existed on this VM, so one was created at
`/tmp/opencode/dagster-venv` with Python 3.12, dagster 1.13.20 and this
worktree's `dagster/pyproject.toml`; nothing under `/tmp/opencode` is in
the repository):

- `python3 ops/lint/check_intra_package_imports.py` — OK.
- `python3 ops/lint/check_bare_iceberg_count.py` — OK.
- `(cd dagster && /tmp/opencode/dagster-venv/bin/python -m pytest dispar_orchestrate -q)` — **2,870 passed, 1 skipped, 45 subtests**, exit 0 (21m30s).

Compose:

- `sh ops/init-env.sh` (wrote a local `.env`; ignored by git) then
  `docker compose --profile '*' config --quiet` — exit 0.
- **No `up`**: see T9 and `docs/plans/DATA-66-RESULT.md`.

### Not verified, and why (rule 7)

- **T9 measurement: not measured.** No `lakehouse-api` / `console` /
  `dagster-code-location` image exists on this VM (all three are `build:`
  services, and the API image is a release cargo build of the workspace);
  `/` had 24 GB free at 96% used with no Docker build cache, so the image
  build plus the running stack could not be proven to fit, and the
  clean/target shortcuts that would make room are forbidden by AGENTS.md.
  The 100,000-row pipeline and outside-`INSERT` runs are also the owner's
  acceptance steps `DATA-66-AC1`/`AC2`, still not run.
  `docs/plans/DATA-66-RESULT.md` says **not measured** and gives the exact
  recipe. No number was estimated.
- **Acceptance checklist `DATA-66-AC1`–`AC7`: not run** (owner's step).
- **The compose stack was never started**, only validated with `config`.
- The pre-existing TypeScript failure above is not fixed here (out of
  scope; it has no backlog item yet, which the planner should add).

### Notes for the reviewer (decisions taken inside a task's scope)

- T1 stores the schedule as `SyncSchedule` (an owned
  `Serialize` view of `report_schedule::Schedule` limited to hourly/daily)
  on `GoldPublication`, because `Schedule` itself is not `Serialize` and
  the contract only admits hourly/daily. `SyncSetting` keeps
  `Option<Schedule>` as the plan writes it.
- `set_sync_in_zone` also enforces the mode/schedule invariant
  (`scheduled` ⇔ schedule + zone) and a known IANA zone with the same
  fixed `StoreError::Validation` sentences, so the API's validator and
  the migration's CHECK cannot drift.
- T2's `publication_body` takes the whole stored row (`Option<&GoldPublication>`)
  instead of `(enabled, updated_at)`, because it now needs the sync
  fields. The `gold.publication_set` audit records
  `syncMode`/`schedule`/`timeZone`.
- T3 added `GET /api/gold/sync-due` to the SEC-66 policy test's array,
  and the `route_auth` sec66 table pairs it with `gold:export` (the
  handler guard is `check_export_token`, as D10 and T3 say).
- T4's tests live in `tests/gold_publication.rs` (real Postgres +
  ClickHouse + a wiremock Dagster) because that file already owns the
  `Sessions`/`CATALOG_TENANT_ID` fixture; the "both ways" auth test is
  in `tests/route_auth.rs`. `seed_gold_export_service_credential` in
  `tests/gold_export_auth.rs` became `pub(crate)` so the second test
  reuses it instead of copying it.
- T5 `evaluate_sync_due` yields one `SkipReason` for an empty due list
  (the check asks for it) and refuses an unknown trigger as well as an
  unknown zone.
- Three test expectations changed by design, each with the reason at the
  site: `test_job_serial_tags.py` now expects both instance rules
  (`lakehouse/serial` and `lakehouse/iceberg-sync-target`) and gains a
  per-table counter test; `test_gold_export.py`'s zone-union test became
  "no mapped step for console publications" because D9 removed that
  union; `gold-exports-page.test.tsx`'s column indices moved by one for
  the new Sync column.
- T8's grep found no other current-behaviour doc claiming "after any
  pipeline" or a nightly 04:00 for console tables; the remaining hits are
  this plan, the spec's own "Today" column, and the historical
  2026-10-02 plan's record.

## Review

(planner appends here)
