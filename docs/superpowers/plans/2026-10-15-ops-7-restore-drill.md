# `OPS-7` Complete backups and a timed restore drill: plan

Planner: Claude Opus (Security & Delivery lane). Developer: a non-Opus agent.
Base: `origin/main` after #83 (`SEC-18`) merged. Branch:
`feat/ops-7-restore-drill`, worktree `/home/shiro/rantai/wt/ops-7`. Spec:
`docs/core/specs/ops-7.md` (D1, D2 signed 2026-10-15). A Task.

## 1. Anchors (verified at `8e2f6a19`)

- `dagster/dispar_orchestrate/backup_job.py`: module docstring (dedicated
  bucket and why; external trigger, not a Dagster schedule);
  `DATABASES` (line 71); `resolve_secret_ref` (79); `BackupConfig.from_env`
  (117); `run_backups` (159) dumps with `pg_dump -Fc` and uploads through
  `s3fs.S3FileSystem` to `<BACKUP_S3_BUCKET>/<db>/<ts>.dump`;
  `_prune_expired` (188) with `BACKUP_RETENTION_DAYS`.
- `dagster/dispar_orchestrate/test_backup_job.py`: unit tests (no network).
- `docker-compose.yml` service `backup-job` (`profiles: ["backups"]`,
  `PG*` env, `restart: "no"`); `ops/backups/docker-compose.backups.override.yml`
  carries the must-set `BACKUP_S3_*` (why it is an override: `${X:?}` is
  interpolated for every service).
- `scripts/backup-postgres.sh`, `scripts/restore-postgres.sh`: the older
  app-database-only pair. `docs/OPERATIONS.md` lines ~961-1089: both
  procedures and the restore steps.
- The warehouse bucket: `LAKEHOUSE_WAREHOUSE_BUCKET` on RustFS
  (`.env.example`); Iceberg data and metadata files are immutable once
  written.
- `ClickHouse` databases: `bronze_meta` (ingest runs, offsets, slots,
  catalog, maintenance), plus any other non-system database; `icecat_*`
  are `DataLakeCatalog` databases (views onto the Iceberg catalog, not
  data) and are recreated by `clickhouse-iceberg-init`.
- Result-file shape: `docs/plans/G3-RESULT.md`, `docs/plans/SRC-10-RESULT.md`.

## 2. Decisions

- E1. One backup job, three parts, in `backup_job.py`: the existing
  Postgres dumps; the warehouse bucket; `ClickHouse`. Each part is timed
  and its size reported in the job's log (AC1). A part that fails is a
  `dagster.Failure` naming the part; parts already uploaded stay.
- E2. Warehouse bucket: an incremental mirror into
  `<BACKUP_S3_BUCKET>/warehouse/` keyed by object key: copy an object only
  when the backup has no object of that key and size. Iceberg files never
  change after they are written, so this is correct and makes later runs
  cheap. Objects removed from the warehouse are not removed from the
  backup (a restore copies files Iceberg no longer references; harmless).
  Read access to the warehouse uses its own pair,
  `BACKUP_WAREHOUSE_ACCESS_KEY_SECRET_REF`/`…_SECRET_KEY_SECRET_REF`
  (same `env:` scheme), must-set in the override file, never the backup
  bucket's pair (the backup bucket stays write-only for warehouse
  credentials and vice versa).
- E3. `ClickHouse`: `BACKUP DATABASE <db> TO S3('<backup endpoint>/<bucket>/clickhouse/<ts>/<db>/', …)`
  for each non-system, non-`DataLakeCatalog` database. **Verify first**
  that `ClickHouse` 26.8 masks the credentials of `S3(...)` in
  `system.query_log` and the server log; if it does not, stop and report
  (do not ship credentials into a query log). Prune `clickhouse/` by the
  same retention as the dumps.
- E4. Restore is a script, `scripts/restore-from-backup.sh`, plus the
  documented order in `docs/OPERATIONS.md`. Order: Postgres up alone;
  `pg_restore` the four databases (before the `*-migrate`/`*-init` jobs
  run, or prove they are no-ops on restored schemas); warehouse objects
  copied back; `ClickHouse` `RESTORE`; then the rest of the stack. The
  script takes the backup timestamp as an argument, prints each step's
  duration, and refuses to run against a stack whose volumes are not
  empty unless given `--force`.
- E5. Secrets never reach argv or logs: values through the environment,
  as `backup_job.py` and `ops/init-env.sh` already do.
- E6. Compose: the new variables go in the override file; the
  `backup-job` service in `docker-compose.yml` gains only non-secret
  settings it needs (warehouse endpoint and bucket name, `ClickHouse` URL
  and user). Compose changes are proven with a real `up` (rule 8).

## 3. Tasks

**T1. Warehouse mirror (E2, E5) with unit tests** (`s3fs` mocked; names are
sentences: copies a new object, skips one already there with the same
size, copies one whose size differs, reports count and bytes).
*Check:* `test_backup_job.py` and the whole Dagster package under the lock;
quote totals; mutation: make the size check always true, show a test fail.

**T2. `ClickHouse` backup (E3) with unit tests** (the SQL built, the
database list filtering `system`, `INFORMATION_SCHEMA`, `information_schema`
and `DataLakeCatalog` engines). Identifiers from `system.databases` only,
quoted; the S3 URL from config, never input.
*Check:* as T1, plus the query-log masking evidence quoted in the handoff.

**T3. Compose and override (E6).** *Check:* `docker compose --profile '*' config --quiet`
with an `ops/init-env.sh` `.env`; the override-file command from
`docs/OPERATIONS.md` validates.

**T4. Restore script (E4).** `shellcheck` clean if available.

**T5. The drill (D2).** Under the heavy lock, project `-p ops7drill`,
profiles `dagster` and `backups`, a backup bucket created on the same
RustFS for the drill (say so: not the production layout):
1. Seed: the G3a or G4 gate's data path (say which) so at least one Bronze
   table, one change-capture table with a stored offset, one extra user, one
   connector and its permission tuples exist. Record row counts with a
   `WHERE` (R11) and the size of each component.
2. Run the backup; record per-part time and size (AC1).
3. `down -v`; bring the stack back empty; run the restore script; record
   per-step and total time.
4. Verify (AC2): sign in as the extra user; Lakekeeper lists the tables;
   the same counts; an OpenFGA check passes for the connector's owner;
   Dagster shows the earlier runs; one change-capture run resumes from its
   offset without re-reading the snapshot (quote the run's log line).
5. `down -v`.
Write `docs/plans/OPS-7-RESULT.md` in `G3-RESULT.md`'s shape: environment
and versions, data size, a table of times per step, verification results,
what this decides, "Not measured" (large data, a remote backup target).

**T6. Docs.** `docs/OPERATIONS.md` backup and restore sections rewritten to
the new job and script; `.env.example` entries for the new variables
(empty, with how to set them); `README.md` if it describes backups;
`CHANGELOG.md` one bullet.

## 4. Pull request

One slice; the planner opens it as a draft after T3 so CI runs on the
compose change.

## 5. Out of scope

Scheduling the job inside Dagster (the module docstring says why);
point-in-time recovery; off-site replication; Kafka.

## 6. Verify, do not assume

- Does `ClickHouse` 26.8 accept `BACKUP … TO S3` against RustFS (path
  style)? Measure it in T2 before building on it.
- Do the `*-migrate`/`*-init` containers run destructively on a restored
  database? Find out in T5 and write the answer into E4's order.
- Does `s3fs` stream large objects without loading them into memory?

## 7. Handoff

(developer)

## 8. Review

(planner)
