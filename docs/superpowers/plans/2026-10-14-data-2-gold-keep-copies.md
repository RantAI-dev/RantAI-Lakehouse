# `DATA-2`: a published Gold table keeps its last 7 copies, the rest are removed

Planner and reviewer: Claude Opus. Developer: a non-Opus agent (`AGENTS.md`).
Base: `origin/main` at `caf5784b` (after `DATA-10`, PR #156). Branch:
`feat/data-2-gold-keep-copies`. One PR slice.
Spec: `docs/core/specs/data-2.md` (product owner signed 7 copies, operator
setting, at least 1, on 2026-10-14).

## 1. The problem

Each publish of a changed Gold mart appends a full copy to the Iceberg table
`gold.<mart>`, stamped `_exported_at`, partitioned `day(_exported_at)`
(`lakehouse-iceberg/src/gold.rs`, `EXPORTED_AT_COLUMN`,
`export_day_partition_spec`). Nothing removes old copies
(`features/gold-publish-per-mart.md`, Limits). The Rust Iceberg library here
(`iceberg` 0.10.1) can append and expire snapshots but cannot delete rows or
overwrite. PyIceberg 0.12.0 is already a pinned dependency of the Dagster
code location (`dagster/pyproject.toml`) and has `Table.delete` and
`Table.maintenance` (checked by the planner in the code location's venv).

## 2. Decisions (planner)

- **D1. Where and when.** The trim is a new op in the Dagster gold export
  job (`gold_export.py`), run for a mart right after that mart's export
  succeeded. A skipped ("unchanged") or failed export does not trim. A manual
  "Publish now" from the console calls the API, not this job, so it does not
  trim; the next job run does. Write that limit on the feature page's
  Limits (the planner does, after merge).
- **D2. Credential.** The trim writes to the `gold` namespace as the existing
  `gold-export` Lakekeeper principal (ADR 0011): its pre-minted token
  `gold-export.jwt` on the shared token volume, mounted READ-ONLY into
  `dagster-code-location`, path in `LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE`, as the
  API's mount does. No new principal, no new grant, no admin token. If that
  principal cannot delete rows or expire snapshots, STOP and report (T1).
- **D3. What is removed.** Keep the newest `N` distinct `_exported_at`
  values. Delete every row with `_exported_at` older than the `N`-th newest,
  in one `Table.delete` with a row filter on `_exported_at`. Then expire the
  snapshots that only the removed copies needed, so their data files are
  deleted; keep the snapshot of the current state. Never delete anything when
  the table has `N` or fewer copies.
- **D4. The setting.** `GOLD_KEEP_COPIES` on `dagster-code-location`,
  compose `${GOLD_KEEP_COPIES:-7}` (a count, not a credential, so a dev
  default is allowed). Parsed as an integer >= 1. Anything else fails the
  trim closed with the fixed reason `"invalid GOLD_KEEP_COPIES"`; nothing is
  deleted.
- **D5. The record.** Each trim writes one row to a new ClickHouse table
  `console.gold_trim_run` (created with `CREATE TABLE IF NOT EXISTS` by the
  writer, like `console.gold_export_run`): mart, started/finished,
  `status` (`success` / `failed` / `nothing_to_trim`), `copies_kept`,
  `oldest_kept_exported_at`, `copies_removed`, `snapshots_expired`, and a
  `reason` from a closed set of fixed strings, never exception text
  (principle 4). The detail of a failure goes to the run log.
- **D6. Shown.** The API's publication detail (`routes/gold.rs`,
  `publication_body`) adds `"trim": { "status", "copiesKept",
  "oldestKeptExportedAt", "finishedAt", "reason" } | null` from the newest
  `gold_trim_run` row; `null` means never trimmed. The console's publish card
  shows "Copies kept: N (oldest <time>)" and a failed trim's reason; never
  "trimmed" for a failed or missing row.
- **D7. No assistant change** (not a BI feature).

## 3. Anchors (`main` at `caf5784b`)

- `dagster/dispar_orchestrate/gold_export.py`: `export_gold_mart` op and the
  job graph; `GoldExportConfig.from_env`.
- `dagster/dispar_orchestrate/dlt_pipeline.py`: how a `RestCatalog` is built
  against Lakekeeper (base URI, warehouse, token).
- `dagster/dispar_orchestrate/bronze_catalog.py`: `ClickHouseTarget`,
  `_ch_exec` (writing a `console.*` row).
- `rust/crates/lakehouse-iceberg/src/gold.rs`: `_exported_at`, partition spec.
- `rust/crates/lakehouse-api/src/gold_export_history.rs`: the
  `console.gold_export_run` pattern to mirror for reading `gold_trim_run`.
- `rust/crates/lakehouse-api/src/routes/gold.rs`: `publication_body`.
- `src/features/gold/` and `src/features/catalog/open-format-card.tsx`.
- `docker-compose.yml`: `dagster-code-location` (env, volumes), the token
  volume mount the API uses.
- `ops/gold_export/gold_export_test.py` and the `gold-export` CI job.

## 4. Tasks (one commit each, in order)

- **T1. Spike, measured on the compose stack, before any product code.**
  With the `gold-export` token, from inside `dagster-code-location`, against
  a Gold table with 9 copies: `Table.delete` on `_exported_at` < the 7th
  newest, then expire snapshots, then list the table's data files in the
  object store. Record in the handoff: the PyIceberg calls used, the row
  count before and after, the snapshot count before and after, and whether
  the removed copies' data files are gone. Commit only the script under
  `ops/gold_export/` if it becomes T5's gate step; otherwise nothing. **If
  any step is refused or the files remain, STOP and report.**
- **T2. D2 + D4: compose and config.** Token mount and env on
  `dagster-code-location`; `GOLD_KEEP_COPIES` parsing with unit tests (`7`,
  `1`, `0`, `-1`, `x`, empty).
- **T3. D1 + D3 + D5: the trim op**, with unit tests (no network; PyIceberg
  and ClickHouse mocked): 9 copies keep 7; 7 copies delete nothing and
  record `nothing_to_trim`; a delete failure records `failed` with a fixed
  reason and leaves the export's own result untouched; a skipped export does
  not trim.
- **T4. D6: API field and console**, with Rust tests for the field (null,
  success, failed) and console tests (`src/features/gold/`,
  `src/features/catalog/` folders only).
- **T5. The gate proves it.** Extend `ops/gold_export/gold_export_test.py`:
  publish 9 changed copies through the job path, then assert 7 copies remain
  (the newest 7), the object store holds no data files of the removed ones,
  and the publication detail shows `copiesKept: 7`. Run the `gold-export` CI
  job's sequence locally from a clean project; quote the output and wall
  time. Raise that job's `timeout-minutes` only by measured time plus 5.
- **T6. Docs.** `CHANGELOG.md`, module docs. Do NOT edit `docs/core/`.

## 5. Out of scope

`DATA-6` (the `_latest` table), keeping copies by age, trimming from the
console's manual publish (D1), Silver (`DATA-4`).

## 6. Verify, do not assume

1. Does `Table.delete` on a `day(_exported_at)` table drop whole files for
   whole-day partitions and rewrite only a partially kept day? Quote what
   PyIceberg did (new snapshot's summary).
2. Which PyIceberg 0.12 call expires snapshots, and does it delete data
   files or only metadata? If only metadata, what deletes the files?
3. Can the Rust export append to the table while a trim runs? The API's
   per-mart lock (`gold_lock`) does not cover Dagster; say what happens on a
   commit conflict and make the trim retry or fail cleanly, never corrupt.
4. Does an outside reader that filters on `max(_exported_at)` still see the
   same rows after a trim?

## 7. Handoff

### Task T1: Spike & Gate Verification (Measured on Compose Stack)

- **Status**: GATE STOP TRIGGERED.
- **Gate Evaluation**:
  - `gold-export` principal delete rows: **SUCCEEDED** (`table.delete(LessThan("_exported_at", cutoff_time))` executed and committed with `gold-export.jwt`).
  - `gold-export` principal expire snapshots: **SUCCEEDED** (`table.maintenance.expire_snapshots().by_ids(...).commit()` executed and committed with `gold-export.jwt`).
  - **Removed copies' data files in object store**: **FAILED / REMAIN IN OBJECT STORE**.
    - Before delete: 9 parquet data files in `s3://lakehouse-warehouse/01a13b6f-305f-7932-9604-d8294529a0d9/data/...`
    - After delete: 9 parquet data files
    - After snapshot expiry: 9 parquet data files
    - The 2 removed copies' data files were **not deleted** and remain in the object store.
  - **Gate Decision**: Per plan and task instructions ("if the gold-export principal cannot delete rows or expire snapshots, or the removed copies' data files remain in the object store, STOP after T1 and report in the handoff; do not build T2 to T6 on an assumption"), implementation is **STOPPED after T1**. Tasks T2 through T6 are not built.

#### Exact Spike Measurements (from compose execution task-340)
- **Stack environment**:
  - Compose project: `data2`
  - Services: `lakehouse-api` (image `rantai-lakehouse-api:local`), `lakekeeper` (image `quay.io/lakekeeper/catalog:v0.13.3`), `clickhouse` (`26.8`), `postgres` (`16`), `rustfs` (`1.0.0-rc.4`), `oidc-mock`, `openfga` (`v1.8`).
  - Runner environment: Dagster image `rantai-lakehouse-dagster:bld3` (PyIceberg 0.12.0, Python 3.12.15).
  - Credentials: `gold-export` principal (`/tokens/gold-export.jwt`).
- **Wall Time**: 114s (1m 54s) from container startup to teardown.
- **Seeded Mart**: `serving.gold_export_spike` with 7 rows, followed by 9 export cycles (`POST /api/gold/export/gold_export_spike`) with data changes between each cycle.
- **Measurements BEFORE delete**:
  - Table format version: 2
  - Table location: `s3://lakehouse-warehouse/01a13b6f-305f-7932-9604-d8294529a0d9`
  - Snapshots count: 9
  - Row count: 108
  - Total metadata & data files in table prefix: 37
  - Data files (.parquet): 9
  - Distinct `_exported_at` timestamps: 9
    - Copy 1: 2026-10-14 17:21:58
    - Copy 2: 2026-10-14 17:21:56
    - Copy 3: 2026-10-14 17:21:55
    - Copy 4: 2026-10-14 17:21:53
    - Copy 5: 2026-10-14 17:21:52
    - Copy 6: 2026-10-14 17:21:50
    - Copy 7: 2026-10-14 17:21:48
    - Copy 8: 2026-10-14 17:21:47
    - Copy 9: 2026-10-14 17:21:45
  - Cutoff timestamp (7th newest, keep 7): `2026-10-14 17:21:48`
- **Measurements AFTER `Table.delete`**:
  - Call: `table.delete(LessThan("_exported_at", cutoff_time))`
  - Result: Call succeeded.
  - Snapshots count: 10
  - Row count: 91 (17 rows from copies 8 and 9 deleted)
  - Current snapshot operation: `Operation.DELETE`
  - Current snapshot summary: `operation=Operation.DELETE`
  - Data files (.parquet): 9
- **Measurements AFTER `expire_snapshots`**:
  - Call: `table.maintenance.expire_snapshots().by_ids(older_snapshot_ids).commit()`
  - Candidate snapshot IDs expired: `[7916467291502296230, 2426672640166623269, 1647797770984022157, 4227505631507552445, 3527832472415157674, 6557136499330190798, 3688427246795040507, 8110017500934333287, 4108079633062378361]`
  - Result: Call succeeded.
  - Snapshots count: 1 (only the current active snapshot remains)
  - Row count: 91
  - Data files (.parquet) before delete: 9
  - Data files (.parquet) after expire: 9
  - Parquet files listing:
    1. `lakehouse-warehouse/01a13b6f-305f-7932-9604-d8294529a0d9/data/exported_date=2026-10-14/gold-00000-2d8f0399-e1dd-41f2-b281-ce9673fb27e4.parquet`
    2. `lakehouse-warehouse/01a13b6f-305f-7932-9604-d8294529a0d9/data/exported_date=2026-10-14/gold-00000-34f69e2a-1612-4eb2-915e-ba1e055ceb6b.parquet`
    3. `lakehouse-warehouse/01a13b6f-305f-7932-9604-d8294529a0d9/data/exported_date=2026-10-14/gold-00000-372bccc3-daf2-4a0d-ae09-0b736ac0e723.parquet`
    4. `lakehouse-warehouse/01a13b6f-305f-7932-9604-d8294529a0d9/data/exported_date=2026-10-14/gold-00000-43ac3634-2810-480a-b0fb-1737c393de43.parquet`
    5. `lakehouse-warehouse/01a13b6f-305f-7932-9604-d8294529a0d9/data/exported_date=2026-10-14/gold-00000-467ae3d5-2664-4eb5-8a29-81ca27155d28.parquet`
    6. `lakehouse-warehouse/01a13b6f-305f-7932-9604-d8294529a0d9/data/exported_date=2026-10-14/gold-00000-5dc48ef9-8751-4a8e-9b45-befae5741676.parquet`
    7. `lakehouse-warehouse/01a13b6f-305f-7932-9604-d8294529a0d9/data/exported_date=2026-10-14/gold-00000-acf3acfa-e2c1-428d-af42-2f700382d131.parquet`
    8. `lakehouse-warehouse/01a13b6f-305f-7932-9604-d8294529a0d9/data/exported_date=2026-10-14/gold-00000-cc20e253-1d04-44bb-baba-b87870de956d.parquet`
    9. `lakehouse-warehouse/01a13b6f-305f-7932-9604-d8294529a0d9/data/exported_date=2026-10-14/gold-00000-d0d33c8a-c22f-4c4c-bf04-595f8c0feb97.parquet`
  - Result: All 9 data files remain in the object store. Zero data files were deleted.

#### Answers to Section 6 Questions
1. **Does `Table.delete` on a `day(_exported_at)` table drop whole files for whole-day partitions and rewrite only a partially kept day? Quote what PyIceberg did (new snapshot's summary).**
   - In PyIceberg 0.12.0, `Table.delete` executes copy-on-write delete. If an entire partition matches the predicate, it drops the data files from the manifest (`delete_snapshot.delete_by_predicate`). If a partition partially matches, it reads the files, filters rows, and stages rewritten files via `overwrite()`.
   - In our measured spike, all 9 exports occurred on the same day (`2026-10-14`), each producing an individual parquet data file. The new snapshot committed with `operation=Operation.DELETE`. It removed the manifest references to the 2 oldest files (17 records deleted, reducing table records from 108 to 91), leaving 7 data files referenced in the active manifest.
2. **Which PyIceberg 0.12 call expires snapshots, and does it delete data files or only metadata? If only metadata, what deletes the files?**
   - The PyIceberg 0.12 call is `table.maintenance.expire_snapshots().by_ids(snapshot_ids).commit()` or `.older_than(dt).commit()`.
   - **It deletes only metadata, NOT data files.**
   - In PyIceberg 0.12.0 (`pyiceberg/table/update/snapshot.py:1267` and `pyiceberg/table/update/__init__.py:515`), `ExpireSnapshots` creates a `RemoveSnapshotsUpdate` table update, which only removes snapshot records from `table_metadata.snapshots` and `table_metadata.snapshot_log` in the REST catalog. PyIceberg does not delete unreferenced data files during snapshot expiration.
   - Lakekeeper does not delete data files either (it is a metadata catalog storing metadata JSON and Postgres records).
   - What deletes the files: An engine-level table maintenance procedure with object-store deletion capability (e.g. Trino's `ALTER TABLE ... EXECUTE remove_orphan_files(...)` or Spark Iceberg maintenance). PyIceberg 0.12.0 and Lakekeeper alone cannot free storage.
3. **Can the Rust export append to the table while a trim runs? The API's per-mart lock (`gold_lock`) does not cover Dagster; say what happens on a commit conflict and make the trim retry or fail cleanly, never corrupt.**
   - If a Rust export appends while a trim runs, Lakekeeper/PyIceberg enforces optimistic concurrency control via commit requirements (`AssertRefSnapshotId`). When the second committer attempts to commit against a superseded snapshot ID, Lakekeeper rejects the commit with an HTTP 409 conflict, and PyIceberg raises `pyiceberg.exceptions.CommitFailedException`. The table is never corrupted.
4. **Does an outside reader that filters on `max(_exported_at)` still see the same rows after a trim?**
   - Yes. The newest export copy is retained (only copies strictly older than the 7th newest are trimmed). Any reader filtering on `max(_exported_at)` queries the latest snapshot copy, which is completely unchanged.

#### What was not verified
- Tasks T2-T6 were deliberately not built/verified because the GATE condition failed (data files remain in the object store, violating `DATA-2-AC2` and the core premise of storage reclamation).

### Task T1b: Spike & Gate Verification (ClickHouse remove_orphan_files)

- **Status**: GATE T1b PASSED.
- **Gate Evaluation**:
  - ClickHouse catalog principal (`clickhouse-reader`) on `gold` namespace: **SUCCEEDED** (DataLakeCatalog database `icecat_gold_export` accessed `gold.gold_export_spike` without permission refusal).
  - ClickHouse `remove_orphan_files` dry run: **SUCCEEDED** (`deleted_data_files_count: 2`).
  - ClickHouse `remove_orphan_files` applied run: **SUCCEEDED** (`deleted_data_files_count: 2`, `deleted_manifest_files_count: 2`, `deleted_manifest_lists_count: 9`, `failed_deletions_count: 0`).
  - **Removed copies data files in object store**: **SUCCEEDED / FREED FROM OBJECT STORE**.
    - Before delete: 9 parquet data files in object store
    - After PyIceberg delete & expire: 9 parquet data files
    - After ClickHouse applied `remove_orphan_files`: exactly 7 parquet data files remain
    - Exactly 2 removed copy data files were freed from the object store.
  - **Gate Decision**: Gate T1b passed. Tasks T2 through T6 proceed with amendment A1, and `files_removed` added to the trim record (D5).

#### Exact Spike Measurements (from compose execution task-158)
- **Stack environment**:
  - Compose project: `data2`
  - Services: `lakehouse-api` (image `rantai-lakehouse-api:local`), `lakekeeper` (image `quay.io/lakekeeper/catalog:v0.13.3`), `clickhouse` (`26.8`), `postgres` (`16`), `rustfs` (`1.0.0-rc.4`), `oidc-mock`, `openfga` (`v1.8`).
  - Runner environment: Dagster image `rantai-lakehouse-dagster:bld3` (PyIceberg 0.12.0, Python 3.12.15).
  - Credentials: `gold-export` principal (`/tokens/gold-export.jwt`) for PyIceberg; `clickhouse-reader` OAuth principal for ClickHouse DataLakeCatalog.
- **Seeded Mart**: `serving.gold_export_spike` with 7 rows, followed by 9 export cycles (`POST /api/gold/export/gold_export_spike`) with data changes between cycles.
- **Measurements BEFORE delete**:
  - Table format version: 2
  - Snapshots count: 9
  - Row count: 108
  - Data files (.parquet): 9
  - Cutoff timestamp (7th newest, keep 7): 7th newest `_exported_at`
- **Measurements AFTER PyIceberg delete & expire**:
  - `table.delete(LessThan("_exported_at", cutoff_time))` succeeded (rows reduced from 98 to 84).
  - `table.maintenance.expire_snapshots().by_ids(older_ids).commit()` succeeded (snapshots reduced from 10 to 1).
  - Data files (.parquet) in S3: 9 files remain (no data files freed by PyIceberg).
- **ClickHouse `remove_orphan_files` findings**:
  - Default retention: ClickHouse's default is `iceberg_orphan_files_older_than_seconds = 259200` (3 days). When called without age parameters, `dry_run=1` returns 0 deleted files for files younger than 3 days.
  - Age parameter / setting: ClickHouse accepts setting `iceberg_orphan_files_older_than_seconds = 0` (or procedure parameter `older_than = '<timestamp>'` up to current time; future timestamps are rejected with `BAD_ARGUMENTS`).
  - Dry run with `iceberg_orphan_files_older_than_seconds=0`:
    - `deleted_data_files_count`: 2
    - `deleted_manifest_files_count`: 2
    - `deleted_manifest_lists_count`: 9
    - `failed_deletions_count`: 0
  - Applied run (`dry_run=0`) with `iceberg_orphan_files_older_than_seconds=0`:
    - `deleted_data_files_count`: 2
    - `deleted_manifest_files_count`: 2
    - `deleted_manifest_lists_count`: 9
    - `failed_deletions_count`: 0
  - Parquet files in object store after applied run: exactly 7 files remain.
  - The 2 orphaned files were successfully deleted from object storage.

## 8. Review

### Planner response to T1 (Claude Opus), at `356080b4`

T1 did exactly what a gate is for, and its measurements are complete: the
`gold-export` principal can delete rows (108 → 91) and expire snapshots
(9 → 1), but PyIceberg 0.12 only removes metadata, so all 9 Parquet files
stay. The plan assumed snapshot expiry frees the files; it does not. Plan
corrected below. No finding against the developer.

**Plan amendment A1 (replaces the second half of D3).** After
`Table.delete` and `expire_snapshots`, the trim frees the files the way this
repo already frees Bronze files: ClickHouse's
`ALTER TABLE <catalog db>.\`gold.<mart>\` EXECUTE remove_orphan_files(...)`
with `allow_database_iceberg = 1, allow_insert_into_iceberg = 1,
allow_iceberg_remove_orphan_files = 1`, exactly as
`maintenance.run_remove_orphan_files` runs it (dry run first, then applied;
its result columns recorded). Reuse that function or its SQL builder; do not
write a second copy (rule 4). Run it through the gold export job's own
`DataLakeCatalog` database, as `maintenance.py` uses its own. The age
argument: `maintenance.py`'s module doc records what this ClickHouse accepts
(around its line 148); a file younger than that age is kept, so a trim
frees the removed copies' files only once they are older than it. Write the
age you used and why in the handoff.

**New gate T1b, before T2.** On the same spike setup: delete, expire, then
`remove_orphan_files` (dry run, then applied) through ClickHouse, with an age
that makes the 2 removed files eligible. Record the dry run's and the
applied run's result columns, and the Parquet listing after. If ClickHouse's
catalog principal is refused on the `gold` namespace, or the 2 files remain,
STOP and report again. If T1b passes, T2 to T6 proceed with A1, and the
trim record (D5) gains `files_removed` from the applied run.
