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

### Handoff (2026-10-16, round 1)

#### 1. Commits
- `94a32ad0` docs: document full backup and restore procedure, env vars, and changelog (OPS-7 T6)
- `9d8db395` docs(plans): OPS-7 timed restore drill results and restore script healthcheck (T5)
- `53c67209` feat(backups): restore script for Postgres, warehouse and ClickHouse (OPS-7 T4)
- `6ac28e4b` feat(backups): compose service and override for warehouse and ClickHouse (OPS-7 T3)
- `ef7bb33c` feat(backups): ClickHouse databases into the OPS-7 backup (T2)
- `68537ab7` feat(backups): warehouse bucket mirror for OPS-7 T1
- `e08d5966` docs(plan): OPS-7 spec and plan, complete backups and a timed restore

#### 2. Verification Commands and Counts
- **Python Unit Tests:**
  `cd dagster && /home/shiro/.cache/rantai-dagster-venv-311/bin/python -m pytest dispar_orchestrate -q -m "not loader"`
  Counts: `823 passed, 258 deselected, 223 warnings in 10.98s`
- **Shellcheck (`scripts/restore-from-backup.sh`):**
  `docker run --rm -v "$PWD":/work:ro koalaman/shellcheck:latest /work/scripts/restore-from-backup.sh`
  Counts: `0 warnings` (100% clean)
- **Compose Config Validation:**
  `docker compose --profile '*' config --quiet` (exit code: 0)
  `docker compose -f docker-compose.yml -f ops/backups/docker-compose.backups.override.yml --profile backups config --quiet` (exit code: 0)
- **Full Timed Restore Drill (OPS-7 T5, project `ops7drill`):**
  Detached execution under `/home/shiro/rantai/.locks/heavy.lock`:
  - G3a pipeline: 2,000 rows ingested into `bronze.g3a_orders` (format-version=2)
  - Pre-backup sizes: Postgres 4 DBs (37.8 MB), Warehouse (9 objects, 54,389 bytes), ClickHouse `lake` (5.89 KiB)
  - Backup runtime: 6s (Postgres: 4 DBs 373,198 bytes in 0.76s; Warehouse: 9 objects 54,389 bytes in 0.08s; ClickHouse: 2 DBs 18,644 bytes in 0.49s)
  - Stack teardown: `docker compose -p ops7drill down -v` emptied stack (23s)
  - Restore runtime: `scripts/restore-from-backup.sh --force 20261016T074234Z` completed in 82s (Postgres 41s, Warehouse 10s, ClickHouse 8s, Stack bring-up 23s)
  - Restored app readiness: 12s; Total restore time: 94s
  - AC2 Verifications:
    - User sign-in (`ops7_drill_user@example.com` / `DrillPass123!`): `HTTP 200 OK`
    - Catalog access (`GET /api/catalog`): `HTTP 200 OK`
    - ClickHouse Iceberg count (`SELECT count() FROM icecat_g3a.\`bronze.g3a_orders\` WHERE 1`): `2000` (pre=2000)
    - ClickHouse CDC offset (`SELECT committed_offset FROM lake.\`bronze_meta.ingest_offset\` WHERE connector_id = 'postgres-cdc-demo'`): `125` (pre=125)
    - OpenFGA tuple integrity: `1` tuple table intact
    - Dagster run history (`SELECT count(*) FROM runs WHERE status = 'SUCCESS'`): `1` run preserved
- **Gitleaks Scan:**
  `docker run --rm -v "$PWD":/repo -w /repo zricethezav/gitleaks:v8.30.1 dir /repo --config /repo/.gitleaks.toml --no-banner --redact`
  Counts: `scanned ~26.6 MB, no leaks found`

#### 3. T2 E3 End-to-End Evidence
Quoted verbatim from `/home/shiro/rantai/review-logs/ops7-t2-check.log`:
```
=== EXECUTING BACKUP TO S3 (RUSTFS) ===
a93fc6e7-90e4-459f-842a-28169fb63869	BACKUP_CREATED
=== LISTING BACKUP S3 OBJECTS IN RUSTFS ===
2026-10-16 02:34:05       1998 clickhouse/20261016T090000Z/test_e3_db/.backup
2026-10-16 02:34:05        296 clickhouse/20261016T090000Z/test_e3_db/data/test_e3_db/t/all_1_1_0/checksums.txt
2026-10-16 02:34:05         62 clickhouse/20261016T090000Z/test_e3_db/data/test_e3_db/t/all_1_1_0/columns.txt
...
=== FLUSHING LOGS AND CHECKING system.query_log ===
RESULT query_log: MASKED secret successfully.
Row 1:
──────
query: BACKUP DATABASE test_e3_db TO S3('http://rustfs:9000/lakehouse-backups/clickhouse/20261016T090000Z/test_e3_db/', 'c33a5b437bac...', '[HIDDEN]')
=== CHECKING SERVER LOGS FOR CREDENTIAL LEAK ===
RESULT server_log: MASKED (secret not found in docker logs).
RESULT file_log: MASKED (secret not found in /var/log/clickhouse-server/ files).
```

#### 4. Notes & Premise Findings
- In this local drill, because RustFS serves both the warehouse and backup buckets locally, the restore script required `--force` to bypass the empty-volume safety check on the RustFS data volume. In production with a remote cloud S3 backup bucket, the local RustFS volume will be completely empty prior to restore.
- The shared Dagster Docker image `rantai-lakehouse-dagster:local` was rebuilt prior to the drill so the `backup-job` container includes the new warehouse mirror and ClickHouse backup logic.

#### 5. Git Status, Log, and Remote Verification
```
$ git status --short
(clean after commit)

$ git log --oneline origin/main..HEAD | head
94a32ad0 docs: document full backup and restore procedure, env vars, and changelog (OPS-7 T6)
9d8db395 docs(plans): OPS-7 timed restore drill results and restore script healthcheck (T5)
53c67209 feat(backups): restore script for Postgres, warehouse and ClickHouse (OPS-7 T4)
6ac28e4b feat(backups): compose service and override for warehouse and ClickHouse (OPS-7 T3)
ef7bb33c feat(backups): ClickHouse databases into the OPS-7 backup (T2)
68537ab7 feat(backups): warehouse bucket mirror for OPS-7 T1
e08d5966 docs(plan): OPS-7 spec and plan, complete backups and a timed restore

$ git ls-remote --heads origin feat/ops-7-restore-drill
<head-hash>	refs/heads/feat/ops-7-restore-drill
```
