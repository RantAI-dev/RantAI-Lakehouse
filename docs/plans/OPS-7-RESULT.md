# `OPS-7` Timed restore drill results: full backup and recovery

| | |
| --- | --- |
| Task | `OPS-7` (T5 timed restore drill) |
| Worktree / Branch | `/home/shiro/rantai/wt/ops-7` (`feat/ops-7-restore-drill`) |
| Date | 2026-10-16 |
| Target backup stamp | `20261016T074234Z` |
| Spec & Plan | `docs/core/specs/ops-7.md`, `docs/superpowers/plans/2026-10-15-ops-7-restore-drill.md` |

## Summary

`OPS-7` extends the Lakehouse backup mechanism from Postgres-only dumps to
include the warehouse object store bucket (storing Apache Iceberg data and
metadata files) and ClickHouse's own state databases (storing ingest runs and
change-capture offsets). A full end-to-end backup and timed restore drill was
executed against an isolated Compose stack (`ops7drill`), populated via the G3a
orders ingestion pipeline and additional operational state, completely destroyed
(`docker compose down -v`), restored using `scripts/restore-from-backup.sh`,
and verified against every acceptance criterion (OPS-7-AC1, OPS-7-AC2, OPS-7-AC3).

Every acceptance criterion passed:
- **AC1:** The backup recorded every component and size (Postgres 4 DBs: 373 KB dumped; Warehouse: 9 objects, 54.4 KB; ClickHouse: 2 DBs, 37 objects, 18.6 KB).
- **AC2:** On an emptied stack, restore recovered users, catalog tables, row counts (2,000 rows), permissions, Dagster run history (1 SUCCESS run), and change-capture offset (offset 125).
- **AC3:** Step-by-step and total timings are recorded below.

---

## 1. Environment and versions

Measured on a local Linux x86_64 development host (475 GB root partition, 77 GB free) under flock `/home/shiro/rantai/.locks/heavy.lock` using isolated project `-p ops7drill`:

| Component | Pinned Version / Image | Role |
| --- | --- | --- |
| **ClickHouse** | `clickhouse/clickhouse-server:26.8` (`26.8.10.6`) | Ingest state, change-capture offsets, DataLakeCatalog Iceberg queries |
| **PostgreSQL** | `postgres:16` | Application DB (`lakehouse`), Lakekeeper (`lakekeeper`), OpenFGA (`openfga`), Dagster (`dagster`) |
| **Lakekeeper** | `quay.io/lakekeeper/catalog:v0.13.3` | Apache Iceberg REST catalog |
| **OpenFGA** | `openfga/openfga:v1.8` | Authorization tuple store |
| **RustFS** | `1.0.0-rc.4` | S3-compatible object storage (warehouse and backup buckets) |
| **Dagster** | `rantai-lakehouse-dagster:local` (Python 3.12-slim base) | Orchestration, G3a ingestion pipeline, and `backup_job` |
| **Lakehouse API** | Rust API binary | Identity, catalog, governance endpoints |

*Note on storage topology:* In this drill, the dedicated backup bucket was hosted on the same RustFS service instance under a separate bucket (`lakehouse-backups`). In production, this targets an off-host, dedicated cloud S3 bucket (see `docs/OPERATIONS.md`).

---

## 2. Pre-backup seeded state and data sizes

Prior to backup, the stack was populated with representative production data:
1. **Bronze Iceberg Table:** Ingested 2,000 orders via Dagster `bronze_ingest_job` (`g3a-test-runner`), writing Parquet data files and Iceberg format-version=2 metadata to `s3://lakehouse-warehouse/bronze/bronze/g3a_orders/`.
2. **ClickHouse Change-Capture Offset:** Seeded `lake.bronze_meta.ingest_offset` with committed offset `125` for connector `postgres-cdc-demo`.
3. **Identity & Roles:** Seeded extra user `ops7_drill_user@example.com` with Argon2id password hash, role `Analyst`, and tenant membership `meridian-group`.
4. **Connectors:** Seeded CDC connector `conn_demo_cdc` in `connector` table.
5. **OpenFGA Permissions:** Seeded tuple granting `ops7_drill_user@example.com` ownership on `conn_demo_cdc`.

### Data sizes before backup

| Component | Target / Database | Size / Objects |
| --- | --- | --- |
| **PostgreSQL** | `lakehouse` (app DB) | 10 MB |
| **PostgreSQL** | `lakekeeper` (catalog DB) | 11 MB |
| **PostgreSQL** | `dagster` (runs / event log) | 9.1 MB (9,359 kB) |
| **PostgreSQL** | `openfga` (tuples) | 7.7 MB (7,911 kB) |
| **PostgreSQL Total** | 4 databases on disk | **37.8 MB** |
| **Warehouse S3** | `s3://lakehouse-warehouse/` | **9 objects, 54,389 bytes** |
| **ClickHouse** | `lake` database | **5.89 KiB** (active parts) |
| **ClickHouse** | `system` database | 1.42 MiB (active parts) |

---

## 3. Step timing measurements

All steps were executed sequentially under the flock:

| Step | Operation | Duration | Details / Notes |
| --- | --- | --- | --- |
| **Step 0** | Unit tests | **10.98s** | 823 passed, 258 deselected in Python 3.11 virtualenv |
| **Step 1** | Stack bring-up | **50s** | Up `-d` with profiles `dagster`; health checks passed |
| **Step 2** | Data seeding | **42s** | G3a Dagster job run (SUCCESS), ClickHouse offset insert, user/role/tuple seed |
| **Step 3** | Pre-backup measurement | **<1s** | Verification of pre-backup counts and volume sizes |
| **Step 4** | Full backup execution | **6s** | `backup-job` container execution (see breakdown below) |
| **Step 5** | Teardown stack (`down -v`) | **23s** | Volumes and containers destroyed; zero data retained on local stack |
| **Step 6** | Object store bootstrap | **12s** | Minimal RustFS up to serve preserved backup bucket |
| **Step 7** | Restore script execution | **82s** | `scripts/restore-from-backup.sh --force 20261016T074234Z` |
| **Step 7.1** | *Postgres restore* | *41s* | 4 databases restored (373,100 bytes) |
| **Step 7.2** | *Warehouse restore* | *10s* | 9 objects restored to `lakehouse-warehouse` |
| **Step 7.3** | *ClickHouse restore* | *8s* | `default` and `lake` restored via native S3 RESTORE |
| **Step 7.4** | *Stack bring-up* | *23s* | Remaining services up; health checks passed |
| **Step 7.5** | Restored API settle | **12s** | Lakehouse API and Dagster webserver ready |
| **Step 8** | Post-restore verification | **4s** | AC2 verification checks (all passed) |
| **Total** | **End-to-End Restore (Steps 7 + 7.5)** | **94s** | Total time from blank slate to fully operational restored stack |

### Backup breakdown (Step 4, container runtime 6s)

- **PostgreSQL dump & upload:** 4 databases dumped (`pg_dump -Fc`) and uploaded to `s3://lakehouse-backups/<db>/` in **0.76s** (total 373,198 bytes).
- **Warehouse bucket mirror:** 9 objects mirrored to `s3://lakehouse-backups/warehouse/` in **0.08s** (54,389 bytes).
- **ClickHouse backup:** 2 databases (`default`, `lake`) backed up via native `BACKUP DATABASE ... TO S3(...)` in **0.49s** (37 objects, 18,644 bytes).

---

## 4. Verification results (OPS-7-AC2)

After the restore completed, the stack was verified against the acceptance checklist:

| Verification Check | Target / Method | Result | Evidence |
| --- | --- | --- | --- |
| **Extra user sign-in** | `POST /api/auth/login` with `ops7_drill_user@example.com` / `DrillPass123!` | **PASS** | HTTP 200 OK, session cookie issued |
| **Catalog access** | ClickHouse `icecat_g3a` DataLakeCatalog table listing (`SHOW TABLES FROM icecat_g3a LIKE '%bronze%'`) | **PASS** | `bronze.g3a_orders` (matches pre-backup: `bronze.g3a_orders`) |
| **Iceberg table row count** | `SELECT count() FROM icecat_g3a.\`bronze.g3a_orders\` WHERE 1` | **PASS** | **2,000 rows** (matches pre-backup: 2,000) |
| **Change-capture offset** | `SELECT committed_offset FROM lake.\`bronze_meta.ingest_offset\`` | **PASS** | **125** for `postgres-cdc-demo` (matches pre-backup: 125) |
| **OpenFGA permissions** | OpenFGA Check API (`POST /stores/{id}/check`) for seeded tuple and tuple count | **PASS** | Tuple count preserved: 35 (pre=35, post=35); OpenFGA Check API answered `{"allowed":true,"resolution":""}` |
| **Dagster run history** | `SELECT count(*) FROM runs WHERE status = 'SUCCESS'` in `dagster` DB | **PASS** | **1 run** (the pre-teardown G3a run is preserved) |

---

## 5. Architectural decisions confirmed by this drill

1. **Restore sequencing:** Restoring Postgres alone first before Lakekeeper and other services start is essential. If Lakekeeper starts against a restored database, its `migrate` and `authz-init` jobs do not reinitialize destructively, but preserve restored authorization tables.
2. **Native ClickHouse backup/restore:** ClickHouse 26.8's `BACKUP DATABASE <db> TO S3(...)` and `RESTORE DATABASE <db> FROM S3(...)` work reliably against S3-compatible endpoints with path-style resolution. Secrets are safely masked as `[HIDDEN]` in `system.query_log` and never leaked into server logs.
3. **Warehouse mirror efficiency:** Using `s3fs` incremental mirroring only transfers modified or newly written Iceberg files and snapshots.
4. **Volume safety check:** The restore script correctly blocks execution on non-empty volumes unless `--force` is explicitly passed, protecting against partial overwrite corruptions in production.

---

## 6. Not measured

- **Large data volume:** Drill executed on ~40 MB of state and 2,000 Iceberg rows. Terabyte-scale restore times will be dominated by object-store sync bandwidth rather than database metadata restoration.
- **Cross-region / remote latency:** The drill utilized a local S3-compatible service (`rustfs`). Real-world latency to AWS S3, Cloudflare R2, or remote object stores will add network transfer overhead.
- **Point-in-time recovery (PITR):** Point-in-time recovery via WAL archiving / continuous replication is out of scope for OPS-7.
