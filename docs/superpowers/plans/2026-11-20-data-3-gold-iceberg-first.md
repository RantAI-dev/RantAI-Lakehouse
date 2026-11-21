# `DATA-3`: Gold stored in Iceberg first, served from a fast ClickHouse copy

Planner and reviewer: Claude Opus. Developer: **`Hy4 flash`** (OpenCode model), one session,
the same agent for all slices. Branch: `data-3-iceberg-source`; this plan's commit is its first
commit. Spec: `docs/core/specs/data-3.md`. Feature page:
`docs/core/features/gold-in-iceberg-first.md`. ADR: `docs/adr/0019-gold-in-iceberg-first.md`
(**Proposed**). Measurements: `docs/plans/DATA-3-RESULT.md` (T0 writes it).

Every `file:line` anchor below was read at **`main` = `c7cd01a76`**. If the code you find does
not match an anchor, or a step cannot be done as written, **stop and write a "Mismatch found by
the developer" entry under Handoff**. Do not improvise another design (`AGENTS.md`).

## 0. Before you start (the build is blocked)

Nobody starts T1 or later until **all** of these hold. The planner checks them and writes
"Unblocked: <date>, base <sha>" under Handoff; until that line exists, only T0 may run.

1. **`DATA-4`** (Silver publish, branch `data-4-silver-publish`) is merged. It makes every
   publish function take a `PublishedLayer` and keys publications by `(layer, mart)`; T3 below
   uses its names.
2. **`DATA-6`** (#612, branch `data-6-latest-copy`) is merged. Its T3 extracts the snapshot
   expiry and orphan removal helpers from `run_trim_for_mart`
   (`dagster/dispar_orchestrate/gold_export.py:638`, expiry `:848-879`, orphans `:881-918`);
   T6 below reuses them and its `rantai.latest-copy-of` marker check.
3. **ADR 0019 is accepted** and the owner has signed decisions 2a, 2b, 3a and 5–14 on the
   feature page.

Then: rebase this branch onto the new `main` (rule 9, hunk by hunk), and **re-verify every
anchor** in section 3 against that `main`; write the new sha and any moved line into Handoff.

## 1. Decisions (made; do not change)

Owner-signed: copy lag 60 s and dashboard p95 within 10 % (2026-10-14). The rest are the
feature page's decisions 2a–14 and ADR 0019; they are *(proposed)* until signed, and this plan
builds to the defaults written there.

- **D1. Storage setting.** `pipeline_definition.gold_storage TEXT NOT NULL DEFAULT 'clickhouse'`,
  values `clickhouse` | `iceberg` (DB `CHECK`). Rust enum `GoldStorage { Clickhouse, Iceberg }`,
  serde `"clickhouse"` / `"iceberg"`, wire field `goldStorage` on create, on `GET`, and on
  `GET /api/pipelines/runnable`'s `definition`. `iceberg` is accepted only when the target zone
  equals `Config::gold_source_schema` (`config.rs:947`, default `serving`). Fixed at creation:
  a `PUT` that changes it is a 422, fixed sentence `GOLD_STORAGE_FIXED` = "How a Gold table is
  stored is chosen when it is created." A version restore never changes it (the column is not
  in `pipeline_definition_version`).
- **D2. The Iceberg table.** `gold.<sanitize_mart_name(targetTable)>`
  (`lakehouse-iceberg/src/gold.rs:43`, `:75`), format version 2, **unpartitioned**, the
  pipeline's columns only (no `_exported_at`), table property
  `rantai.gold-storage = iceberg-first` (constant `GOLD_STORAGE_PROPERTY` in Python and Rust,
  value `ICEBERG_FIRST`). Created with its first rows in one commit
  (`catalog.create_table_transaction` + append, the `DATA-6` D8 pattern).
- **D3. Writer.** A new module `dagster/dispar_orchestrate/gold_first.py`, called from
  `authored_factory._op_for_pipeline` (`authored_factory.py:549`) instead of
  `_write_clickhouse_table` (`:474`, called at `:662`) when `definition["goldStorage"] ==
  "iceberg"`. It streams the compiled statement (`_compiled_sql`, `:397`; Bronze read setting
  via `_reading`, `:325`) out of ClickHouse with `FORMAT ArrowStream` into pyarrow record
  batches, and replaces `gold.<t>` in **one** PyIceberg commit (`Transaction.overwrite`, or the
  streamed variant D9 picks). Catalog: `load_gold_catalog` (`gold_export.py:615`), principal
  `gold-export`. No new principal, grant, compose service or setting.
- **D4. Row limit.** Rows are counted while streaming; when the count passes
  `GOLD_EXPORT_MAX_ROWS` (default 5,000,000, read with the same parser the API uses,
  `config.rs:2105`) the run stops **before** the commit: `Failure`, reason `row_limit`, nothing
  written, copy unchanged.
- **D5. Copy refresh.** Right after the commit, in the same op:
  `DROP TABLE IF EXISTS serving.__staging_<t>`; `CREATE TABLE serving.__staging_<t> ENGINE =
  MergeTree ORDER BY tuple() AS <compiled SELECT> LIMIT 0` (the SELECT's own types, exactly as
  `_ensure_target_table`, `:446`, does today); `INSERT INTO serving.__staging_<t> SELECT <cols>
  FROM <catalog db>.\`gold.<t>\`` **pinned to the committed snapshot** (D10); `EXCHANGE TABLES`;
  `DROP` staging. Reuse `_ensure_target_table` and the staging-name rule; do not copy them
  (rule 4): factor `_write_clickhouse_table`'s create/insert/exchange tail into a helper that
  takes the `INSERT` source, and call it from both paths.
- **D6. The record.** ClickHouse table `console.gold_copy_run`, created `IF NOT EXISTS` by both
  writers (the op, and the API on first read), DDL identical in both languages (section 5).
  One row per run of the write op and per refresh-only run. `status` in `success`, `failed`,
  `skipped`; `reason` from the closed set in section 6, never exception text (principle 4).
- **D7. Reconcile.** A sensor `gold_copy_reconcile_sensor`, `minimum_interval_seconds=30`,
  `default_status=RUNNING` (comment the reason, as the other sensors do,
  `gold_export.py:1072`). Per `iceberg` pipeline from `GET /api/pipelines/runnable`: load
  `gold.<t>`'s current snapshot; read the last `success` row's `snapshot_id` for that table;
  if they differ, or `serving.<t>` does not exist, yield a `RunRequest` for
  `gold_copy_refresh_job` with `run_key = "<t>:<snapshot_id>"`. Catalog unreachable:
  `SkipReason` with a fixed text, never a run. `gold_copy_refresh_job` joins `SERIAL_JOB_NAMES`
  (`ingest_factory.py:334`); run limit `DAGSTER_RUN_LIMIT_GOLD_COPY_REFRESH_JOB_SECONDS`,
  default 1800.
- **D8. Snapshots kept.** After a successful write the op expires all but the newest
  `GOLD_KEEP_COPIES` (default 7) snapshots of `gold.<t>`, then removes orphan files, with
  `DATA-6`'s extracted helpers; skipped and recorded while `MAINTENANCE_PAUSED=true`. A failed
  cleanup never fails the write; it goes into the row's `cleanup` column.
- **D9. Memory variant (decided by T0).** If T0 M1 shows `Transaction.overwrite` of a full
  `pa.Table` fits at 5,000,000 rows within the code location's memory, use it. Otherwise use the
  streamed variant: one `Transaction`, `tx.delete(AlwaysTrue())`, then `tx.append(batch_table)`
  per 100,000-row group, one `commit_transaction()`; T0 M6 must show it is one catalog commit.
  If neither fits, **stop** and report; the owner decides the row limit (decision 12).
- **D10. Pinned read (decided by T0).** If T0 M2 shows ClickHouse 26.8 honours
  `SETTINGS iceberg_snapshot_id = <id>` on a `DataLakeCatalog` table, use it. Otherwise the
  fallback: read the catalog's current snapshot before and after the `INSERT`; if they differ,
  drop staging and retry once; then fail `snapshot_moved`. The record holds the snapshot read.
- **D11. API.** Covered in T2–T5. Fixed sentences:
  `GOLD_STORAGE_ZONE` "Only a Gold table can be stored in open format first.";
  `GOLD_STORAGE_TENANT` "Only the catalog's organisation can store a Gold table in open format
  first."; `GOLD_STORAGE_NAME_TAKEN` "An open-format table with this name already exists.";
  `GOLD_STORAGE_FIXED` (D1); `GOLD_STORED_IN_OPEN_FORMAT` "This table is stored in open
  format; there is nothing to publish."; `DOOR_MASKED_TABLE` "This table has a masking or
  row-filter policy; outside tools cannot read it."
- **D12. Permissions.** Choosing `iceberg`: `pipeline:write` (`policy.rs:601`, `:609`). The new
  status route: `RequiresAuth` plus the same tenant gate as the publication `GET`
  (`routes/gold.rs`, `gated_source_table`). No new permission.
- **D13. Tenants.** With more than one tenant, `iceberg` is refused unless the pipeline's tenant
  is the catalog's (`CATALOG_TENANT_ID`); with several tenants and no catalog tenant, refused.
  Same rule shape as `DATA-4` D9(b).
- **D14. Assistant.** No tool or schema change; `create_pipeline`
  (`routes/ai/tools/mod.rs:269`) never sends `goldStorage`, so it gets `clickhouse`. No
  `tool_schemas.json` change. Not a dashboard feature (spec, "Asking the assistant").
- **D15. Console developer.** The console part adds one field to the existing pipeline editor
  and a state to the existing open-format card; it is not a revamp, so it stays with
  `Hy4 flash` (`AGENTS.md` console exception; same call as `DATA-4` D12).

## 2. T0 is a stop gate, and its numbers are a required artifact

A previous plan's T0 recorded nothing and its claim ended unsupported. So:

- T0's output is **`docs/plans/DATA-3-RESULT.md`**, committed on this branch, in the template
  of section 7. Each of M1–M6 quotes the **raw JSON line** the script printed, plus machine
  type (cores, memory; no hostnames, rule 12), ClickHouse, Lakekeeper and PyIceberg versions,
  data size, and the exact command.
- A measurement not taken is written `not measured: <reason>`. **Any of M1–M5 "not measured"
  blocks T1.** The reviewer refuses any commit of T1 or later on a branch whose RESULT file
  lacks a filled M1–M5, and re-runs one measurement of their choice before approving.
- **Stop rules** (stop, write the numbers, report to the planner; do not continue, do not
  lower a target):
  - M1 overwrite adds more than one snapshot per run → the design is invalid.
  - M2 neither pinned read nor fallback returns exactly the committed snapshot's rows.
  - M3 copy-lag p95 at 1,000,000 rows > 60 s.
  - M4 any type or checksum mismatch the mapping does not already refuse.
  - M5 copy p95 > 1.10 × baseline p95.
  - M1/M6 neither variant fits in memory at 5,000,000 rows (D9).

## 3. Anchors (verified at `c7cd01a76`)

Python, `dagster/dispar_orchestrate`:
- `authored_factory.py:325` `_reading`; `:397` `_compiled_sql`; `:446` `_ensure_target_table`;
  `:474` `_write_clickhouse_table` (staging, `EXCHANGE TABLES` at `:541`); `:549`
  `_op_for_pipeline`; `:662` the write call; `:673` the materialization; `:683`
  `build_authored_job`; `:1401` `_pipeline_target`.
- `gold_export.py:308` `_enabled_publication_marts`; `:384` `list_gold_marts`; `:531-552` SQL
  literal helpers; `:555-612` trim table DDL and writer (pattern for D6); `:615`
  `load_gold_catalog`; `:638` `run_trim_for_mart`; `:1080` `evaluate_authored_success` (the
  after-authored publish sensor); `:1110` `gold_export_schedule`.
- `bronze_catalog.py:79` `ClickHouseTarget`; `:239` `_ch_exec`; `:249` `_ch_query_json`.
- `maintenance.py:249` `maintenance_paused`; `:330` `ensure_catalog_database`; `:437`
  `run_remove_orphan_files`.
- `ingest_factory.py:323` `SERIAL_TAG`; `:334` `SERIAL_JOB_NAMES`; `:345` `job_tags`.
- `definitions.py:28-31` gold imports; `:112` `jobs=[`.
- `dagster/pyproject.toml:20` `pyiceberg==0.12.0`; `:40` `pyarrow==25.0.1`.

Rust:
- `lakehouse-store/src/pipelines.rs:441` `AuthoredDefinition` (add `gold_storage` after
  `max_retries`, `:472`); `:896`, `:1087` the row-to-definition mappings; `:1217` the other
  `target_zone` struct.
- `lakehouse-api/src/routes/pipelines.rs:3020` `CreatePipelineBody`; `:3106` `create`.
- `lakehouse-api/src/routes/authored_pipelines.rs:1669` `runnable`.
- `lakehouse-api/src/gold_export.rs:90` `GoldExportError`; `:282` `ch_type_to_primitive` (the
  type mapping T1 moves into a fixture-checked table).
- `lakehouse-api/src/routes/gold.rs:247` `check_export_token`; `:453` `export`; `:1172`
  `publication_body`; `:1282` `publications`; `:1410` `set_publication`.
- `lakehouse-api/src/gold_export_history.rs:49-93` cached `ensure_*` tables (pattern for D6).
- `lakehouse-api/src/routes/iceberg_door.rs:304` `refuse`; `:315` `namespace_is_gold`; `:340`
  `table_name_is_allowed`; `:923` `table`; `:981` `credentials`.
- `lakehouse-api/src/policy_engine.rs:49` `PolicyCondition`;
  `lakehouse-store/src/governance.rs:203` `list_policies`.
- `lakehouse-api/src/policy.rs:524-547` the gold routes; `:600-632` the pipeline routes.
- `lakehouse-iceberg/src/catalog.rs:374` `load_gold_table_if_exists`; `:563` `GoldTable`;
  `:598` `current_snapshot_id`; `:608` `current_snapshot_timestamp_ms`.
- `lakehouse-api/tests/it.rs:114-115` gold test modules.
- Migrations: highest on `main` is `0151_kafka_connectors_stream.sql`; `DATA-4` adds one. Use
  the next free number **at rebase time**; pattern `rust/migrations/0084_pipeline_target_lag.sql`.

Console, `src/`:
- `src/features/pipelines/pipeline-editor.tsx:119` `targetZone` state; `:293` the save body.
- `src/features/catalog/asset-overview.tsx:276` `OpenFormatCard` on Gold assets;
  `src/features/catalog/open-format-card.tsx` (after `DATA-4`/`DATA-6` merge, re-anchor).
- `src/components/copyable.tsx` `Copyable` (reuse; rule 4).

Gate, bench, CI:
- `ops/bench/dashboard_open_bench.py:70-71` arguments (T0 adds `--fresh`).
- `ops/gold_export/gold_export_test.py:477` outside-tool read; `:534` door refusals; `:796`
  `main`. `docker-compose.yml:1917` `gold-export-test-runner`. `.github/workflows/ci.yml:3808`
  the `gold-export` job.

## 4. Tasks

One task per commit, in this order. Each commit runs only the checks for what it touched
(`AGENTS.md`, "Keep build time down"); all cargo through `bin/cargo-locked`, shared
`CARGO_TARGET_DIR`, never `cargo clean`. Push after every commit.

### T0. Measure, then stop for review (slice A)

- Add `--fresh` to `ops/bench/dashboard_open_bench.py` (sends `fresh=1`, BI-4 part B; a test
  in `ops/bench/` for the URL it builds).
- Write `ops/bench/gold_first_t0.py`: a script, run against a throwaway stack
  (`docker compose -p data3t0 --profile dagster up`, torn down after), that prints one JSON
  line per measurement:
  - **M1** builds a reference table in ClickHouse (1,000,000 rows × 20 columns covering every
    type `ch_type_to_primitive` accepts; then 5,000,000), streams it as `ArrowStream`, does a
    PyIceberg `overwrite` into `gold.t0_ref`; reports snapshots before/after, the snapshot
    `operation`, wall ms, peak RSS (`resource.getrusage`) — 3 runs each size.
  - **M2** commits two different snapshots, reads the older one with
    `SETTINGS iceberg_snapshot_id = <older>` through a `DataLakeCatalog` database; reports the
    row count and checksum against each snapshot's; if refused, runs D10's fallback and
    reports it.
  - **M3** runs commit → copy refresh (D5) 20 times at 1,000,000 rows and 5 at 5,000,000;
    reports per run the commit timestamp (snapshot `timestamp-ms`), swap finish
    (`now64(3)` after `EXCHANGE`), lag ms; p50/p95/max.
  - **M4** compares `system.columns` types of a today's-path table and the copy built from the
    same `SELECT`, and `sum(cityHash64(*))` of both.
  - **M5** creates a 12-tile board on today's table and an identical one on the copy (same
    rows); runs `dashboard_open_bench.py --fresh --rounds 20` three times each; reports p95s.
  - **M6** the streamed variant of D9: one transaction, delete + appends of 100,000-row groups;
    snapshots added by the commit and peak RSS at 5,000,000.
- Write `docs/plans/DATA-3-RESULT.md` (section 7 template), quoting each raw line.
- **Acceptance:** RESULT has M1–M6 filled with raw output and environment; stop rules
  evaluated in a "Verdict" line per measurement; `python3 -m pytest ops/bench -q` green.
  Open slice A's PR (the planner opens it); **wait for the planner's "T0 accepted" under
  Handoff** before T1.

### T1. Storage column and the type table (Rust; slice B)

- Migration `NNNN_pipeline_gold_storage.sql` (why-header: ADR 0019, D1; default keeps every
  existing row on today's path), adding `gold_storage` with the `CHECK`.
- `lakehouse-store`: `GoldStorage` enum; read/write it on create, get, list, runnable; tests in
  `tests/` via `it.rs`.
- Move the ClickHouse-to-Iceberg type rules into one fixture,
  `tests/fixtures/gold_type_map.json` (`[{"clickhouse": "...", "iceberg": "...", "nullable": bool}
  | {"clickhouse": "...", "refused": true}]`), and a Rust test that asserts
  `ch_type_to_primitive` (`gold_export.rs:282`) matches every row. The function itself does not
  change.
- **Acceptance:** `cargo test -p lakehouse-store`; `cargo test -p lakehouse-api --lib
  gold_export`; a store test proves an old row reads `clickhouse`, and the DB refuses `'x'`.

### T2. Create and update rules (Rust)

- `routes/pipelines.rs` `create` / update: accept `goldStorage`; refuse with D11's sentences:
  wrong zone (422), tenant rule D13 (422), name taken (409: `gold.<t>` exists without
  `rantai.gold-storage = iceberg-first`, via `load_gold_table_if_exists`; catalog unreachable
  is a 503 with the existing fixed catalog sentence, never a pass), a name ending `_latest`
  (reuse `DATA-6`'s guard), change after creation (422).
- `runnable` returns `goldStorage` in `definition`.
- **Acceptance:** `cargo test -p lakehouse-api --test it pipelines::` (new tests, one per
  refusal, each asserting the fixed sentence and that nothing was stored); `route_auth::` green.

### T3. Publishing refuses these tables (Rust)

- `routes/gold.rs` `export` and `set_publication`: an `iceberg` table → 422
  `GOLD_STORED_IN_OPEN_FORMAT`, **not** recorded as a failed publish (a request turned away,
  `DATA-56` rule). `publications` never lists one. The Rust export (`export_mart`) refuses to
  append to a table carrying `GOLD_STORAGE_PROPERTY` (`GoldExportError::TargetIsGoldFirst`),
  as `DATA-6` D7 does for its marker.
- **Acceptance:** `cargo test -p lakehouse-api --test it gold_publication::` with new tests for
  each refusal and for "no history row written".

### T4. Copy status route (Rust)

- `GET /api/gold/storage/{mart}` in `POLICY_TABLE` (`RequiresAuth`) and asserted both ways in
  `tests/route_auth.rs`. Body (camelCase):
  `{storage, table, icebergSnapshotId, icebergCommittedAt, copySnapshotId, copyRefreshedAt,
  lagMs, state, reason, lastRun}`. `state` rules, in order: no `iceberg` pipeline →
  `storage: "clickhouse"` and nothing else; catalog unreadable → `not_measured`, reason
  `catalog_unreachable`; table absent → `never_written`; record unreadable → `not_measured`,
  reason `record_unreachable`; last refresh row `failed` and snapshot behind →
  `refresh_failed`; copy snapshot ≠ Iceberg snapshot → `behind`; equal → `current`. Never
  `current` on a read error.
- Ensure `console.gold_copy_run` with the cached `ensure_*` pattern
  (`gold_export_history.rs:49-93`).
- **Acceptance:** `cargo test -p lakehouse-api --test it gold_storage::` (new file, `mod` line
  in `it.rs`): one test per state, a `wiremock` ClickHouse and catalog; `sec11_guard` green.

### T5. The door refuses masked open-format-first tables (Rust)

- `iceberg_door.rs` `table` (`:923`) and `credentials` (`:981`): after `refuse` and the
  namespace check, if the loaded table carries `GOLD_STORAGE_PROPERTY` and any `ready` policy
  (`list_policies`, parsed with `PolicyCondition`) names `<gold_source_schema>.<t>` with a mask
  or row filter → 403 in the Iceberg error shape, `DOOR_MASKED_TABLE`. Policy store
  unreadable → 503, fail closed. Not counted as a read.
- **Acceptance:** `cargo test -p lakehouse-api --test it iceberg_door::` new tests: masked →
  403, unmasked → served, policy store down → 503, a published (today's path) table with a
  policy → unchanged behaviour.

### T6. The write path (Python)

- `gold_first.py` (narrative module docstring; frozen config dataclass with `from_env()`):
  stream (D3), row limit (D4), type mapping read from `tests/fixtures/gold_type_map.json`
  (path resolved from the package, a unit test asserts every row), create-or-overwrite (D2, D9),
  refresh (D5, D10), record (D6), snapshots (D8). Branch in `_op_for_pipeline` on
  `goldStorage`; the materialization metadata gains `snapshot_id`.
- `evaluate_authored_success` (`gold_export.py:1080`) skips `iceberg` pipelines (they have
  nothing to publish).
- Before writing, refuse `name_taken` if `gold.<t>` exists without the marker, or carries
  `DATA-6`'s `rantai.latest-copy-of`.
- **Acceptance:** `cd dagster && python -m pytest dispar_orchestrate -q -k "gold_first or
  authored or gold_export"`; tests (full-sentence names, no network) cover: one commit per run,
  row limit commits nothing, refresh failure leaves Iceberg committed and records
  `copy_refresh_failed`, pinned snapshot recorded, cleanup failure does not fail the write,
  `MAINTENANCE_PAUSED` skips cleanup.

### T7. Reconcile sensor and refresh-only job (Python)

- `gold_copy_reconcile_sensor` and `gold_copy_refresh_job` (D7), registered in
  `definitions.py`; the job on `SERIAL_JOB_NAMES` and in `test_job_serial_tags.py`.
- **Acceptance:** pytest as T6 plus `-k "reconcile or serial"`: differing snapshot → one
  `RunRequest` with the run key; equal → `SkipReason`; catalog down → `SkipReason`, no run;
  copy table missing → `RunRequest`.

### T8. Gate (compose test profile)

- Extend `ops/gold_export/gold_export_test.py` with steps (no new service): create an
  `iceberg` pipeline over the seeded mart; run it; Iceberg row count and checksum equal the
  copy's; `GET /api/gold/storage` says `current` and quotes the snapshot; the record's
  `lag_ms` is printed; publish refused (T3); the outside tool reads `gold.<t>` (no publish);
  add a masking policy → the door refuses (T5); commit a snapshot directly with PyIceberg while
  the sensor is stopped → `behind`; start the sensor → `current` within 90 s; drop
  `serving.<t>` → rebuilt.
- **Acceptance:** `docker compose -p data3gate --profile '*' up` of the `gold-export` gate from
  a clean project, exit 0, log attached to Handoff (rule 8); CI job `gold-export` green.

### T9. Console (slice C)

- Contracts → clients → `services/index.ts` → features (one direction): `goldStorage` on the
  pipeline contract (`"clickhouse" | "iceberg" | string`), `GoldStorageStatus` and
  `getGoldStorage(mart)` via `apiFetch`.
- Pipeline editor: when the target zone is Gold, a choice "Analytics engine (today)" /
  "Open format (Iceberg), with a fast copy for dashboards"; read-only after creation, with the
  sentence of D1. Hidden for other zones.
- Gold table page: for an `iceberg` table the open-format card shows, instead of the publish
  switch: `gold.<t>` with `Copyable`, last write time, and the state label of the feature page
  item 3 with its reason; no "Publish now". `useService` for state.
- **Acceptance:** `bun run typecheck && bun run lint && bun run test` (new tests: each state
  label, never "Copy current" for `behind` / `not_measured`; editor field hidden outside Gold,
  read-only after save).

### T10. Docs (slice C)

- `docs/OPERATIONS.md`: the new path, the record table, the sensor, the masking refusal.
- `CHANGELOG.md` entry a customer can read; feature page "Limits" gets the measured lag at
  5,000,000 rows from RESULT.
- **Acceptance:** links resolve; no hostnames or credentials (rule 12).

## 5. `console.gold_copy_run` DDL (identical in Rust and Python)

```sql
CREATE TABLE IF NOT EXISTS console.gold_copy_run (
    run_id String,
    mart String,
    kind LowCardinality(String),       -- 'write' | 'refresh'
    status LowCardinality(String),     -- 'success' | 'failed' | 'skipped'
    reason LowCardinality(String),     -- section 6, '' on success
    snapshot_id Nullable(Int64),
    committed_at Nullable(DateTime64(3, 'UTC')),
    copied_at Nullable(DateTime64(3, 'UTC')),
    lag_ms Nullable(Int64),
    rows Nullable(UInt64),
    cleanup LowCardinality(String),    -- 'done' | 'paused' | 'failed' | 'not_run'
    started_at DateTime64(3, 'UTC'),
    finished_at DateTime64(3, 'UTC')
) ENGINE = MergeTree ORDER BY (mart, started_at)
```

## 6. Reasons (closed set)

`row_limit`, `type_refused`, `name_taken`, `catalog_unreachable`, `commit_failed`,
`commit_conflict`, `copy_refresh_failed`, `snapshot_moved`, `record_unreachable`,
`publishing_not_applicable`. Anything else is a bug.

## 7. `docs/plans/DATA-3-RESULT.md` template (T0 fills it)

```
# DATA-3 result: T0 measurements (ADR 0019)
Plan: docs/superpowers/plans/2026-11-20-data-3-gold-iceberg-first.md
Date / commit / machine type (cores, memory) / ClickHouse / Lakekeeper / PyIceberg / pyarrow
Command: <exact>
## M1 overwrite (1M, 5M)      raw: <json lines>   Verdict: pass | STOP (<rule>)
## M2 pinned read             raw: ...           Verdict: ... (pinned | fallback)
## M3 copy lag (20 × 1M, 5 × 5M)  raw: ...  p50/p95/max   Verdict: ...
## M4 types and checksums     raw: ...           Verdict: ...
## M5 dashboard p95 (baseline vs copy, 3 × 20 rounds, --fresh)  raw: ...  Verdict: ...
## M6 streamed variant        raw: ...           Verdict: ... ; D9 choice: <variant>
## Not measured (each with its reason)
```

## 8. PR slicing

- **Slice A — T0 only** (`ops/bench/*`, `docs/plans/DATA-3-RESULT.md`). Small PR; the planner
  reviews the numbers against section 2 and writes "T0 accepted" or a stop.
- **Slice B — T1–T8** (Rust, Python, gate). Rust tasks T1–T5 sit together and the migration
  lands with T1's code (one rebuild). Draft PR opened as soon as T1 is pushed, so CI runs while
  T2–T8 are built.
- **Slice C — T9–T10** (console, docs). Can start once T4's route shape is merged on the branch.

## 9. Verification before each handoff

Run the lines for what the slice touched, on its final commit, in the foreground; quote the
command and counts (rule 7).

```bash
cd rust && cargo fmt --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace   # slice B; CI's nextest is final
bun run typecheck && bun run lint && bun run test                                              # slice C
python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py
(cd dagster && python -m pytest dispar_orchestrate -q)                                         # slice B
python3 -m pytest ops/bench -q                                                                 # slice A
docker compose --profile '*' config --quiet   # after `sh ops/init-env.sh`; plus the T8 `up`
git status                                    # nothing generated staged
```

## 10. Out of scope

- Switching an existing Gold table to the new path; moving tables automatically.
- Silver on this path (`DATA-4` keeps ADR 0010's path for Silver).
- Masked outside reads of an open-format-first table (refused, D11).
- Incremental (merge) writes; each run replaces the table.
- Changing the default path for new tables.
- Any change to the assistant's tools or `tool_schemas.json`.
- ADR 0010's status line (the planner edits it on acceptance).

## Handoff

(The developer appends one entry per slice: commits, commands with counts, anything skipped
or *not verified* with the reason.)

## Review

(The planner writes findings here, tagged `BLOCKER` or `SHOULD-FIX`.)
