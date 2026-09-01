# G3 result — measured, small-file compaction gate

P4's compaction gate (`docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md` §3): generate
a synthetic small-file load equivalent to ~14 days of CDC-rate writes
against a Bronze table; measure file count per partition and query
planning time per partition before/after maintenance; escalate to
Trino-as-cron if planning degrades beyond 2x.

Measured against a fresh `docker compose -p p4check` stack (RustFS,
Lakekeeper `0.13.3`, ClickHouse `26.3.26.3`), independently reproducible via
`ops/g3/g3_loadgen.py` (load generator) and the commands below. Volumes
destroyed afterward.

## Headline: which maintenance verbs actually work on a catalog-registered Iceberg table

Measured directly against `bronze.g3a_orders` (a real Lakekeeper-registered
table created by the G3a job) and a dedicated synthetic table,
`bronze.g3_loadtest`, via `icecat` (a `DataLakeCatalog` database over the
same Lakekeeper/RustFS backend every other Bronze consumer uses).

| Verb | Result | Evidence |
| --- | --- | --- |
| `expire_snapshots` | **Works.** | `ALTER TABLE t EXECUTE expire_snapshots()` — gated by `allow_database_iceberg=1, allow_insert_into_iceberg=1, allow_experimental_expire_snapshots=1`. Returns a 7-row `key\tvalue` result set (`deleted_data_files_count`, ..., `dry_run`). |
| `expire_snapshots(dry_run=1)` | **Works — this is the dry-run mechanism.** | Same result shape, `dry_run` echoes `1`, no deletion performed. **This corrects `CLICKHOUSE-MAINTENANCE-FINDINGS.md`'s claim that `OPTIMIZE ... DRY RUN` is the dry-run mechanism** — see below. |
| `remove_orphan_files` | **Does not exist for Iceberg tables.** | `Code: 36. DB::Exception: Unknown EXECUTE command 'remove_orphan_files' for Iceberg table. (BAD_ARGUMENTS)` — a specific per-engine dispatch rejection (distinct from the generic `NOT_IMPLEMENTED` a MergeTree table gives any Iceberg-only verb). This is new information: `CLICKHOUSE-MAINTENANCE-FINDINGS.md` had left this "unverified either way." It is now verified: it does not work, full stop, on 26.3. |
| `OPTIMIZE` (compaction) | **Parses, gated correctly, but fails at runtime on every attempt.** | `Code: 499. DB::Exception: Failed to get object info: No response body.. HTTP response code: 403.` Reproduced identically with (a) Lakekeeper vended credentials and (b) static RustFS admin credentials with full bucket access — `aws-cli` against the exact same object with the exact same static credentials succeeds, ruling out a permissions/credentials cause. This is a genuine ClickHouse-side defect in the Iceberg `OPTIMIZE` write path against a REST-catalog-registered table on 26.3, consistent with `docs/plans/G1-RESULT.md`'s broader finding that ClickHouse cannot reliably write to catalog-registered Iceberg tables on this version. |
| `OPTIMIZE ... DRY RUN` | **Not valid for Iceberg tables at all.** | Grammatically, `DRY RUN` on `OPTIMIZE` requires `DRY RUN PARTS '<list>'` (a MergeTree-only clause) — bare `DRY RUN` errors `Expected PARTS`. Even with `PARTS` supplied: `Code: 36. DB::Exception: OPTIMIZE DRY RUN is only supported for MergeTree family tables. (BAD_ARGUMENTS)`. **This directly corrects `CLICKHOUSE-MAINTENANCE-FINDINGS.md`'s "`DRY RUN` is accepted -- that is the mechanism for P4's dry_run metrics requirement."** That statement was true of the generic `OPTIMIZE` grammar (any table) but false for Iceberg tables specifically, which is the only case P4 needs. |

**Consequence:** of the three-command chain
`CLICKHOUSE-MAINTENANCE-FINDINGS.md` narrowed the original four-command
brief to, only **one** (`expire_snapshots`) actually works against a
catalog-registered Iceberg table. `dagster/dispar_orchestrate/
maintenance.py` runs exactly that one verb and explicitly logs, every run,
that the other two are skipped and why — not a silent no-op.

Reproduce (from a clean `p4check` stack with `bronze.g3a_orders` already
ingested via the `dagster` profile / G3a job):

```
docker exec <clickhouse-container> clickhouse-client --multiquery --query "
SET allow_database_iceberg=1, allow_insert_into_iceberg=1, allow_experimental_expire_snapshots=1;
ALTER TABLE icecat.\`bronze.g3a_orders\` EXECUTE expire_snapshots(dry_run=1);
"
```

## Small-file load generation

`ops/g3/g3_loadgen.py generate`, run inside the `dagster-code-location`
image (already has `pyiceberg`/`pyarrow` pinned — see ADR 0005) attached to
the compose network:

- 14 day-partitions (`day(_ingested_at)`, ADR 0004's default transform).
- 20 append commits/day (a CI-practical stand-in for real CDC micro-batch
  cadence of roughly one flush every ~15 minutes / ~100/day — the
  file-count-per-partition signal this gate measures is a function of file
  *count*, not of how long 14 real days actually took to simulate).
- 40 rows/commit.
- Result: **280 data files total, exactly 20 files per partition across 14
  partitions**, 11,200 rows — confirmed via `pyiceberg`'s
  `table.inspect.files()` (`ops/g3/g3_loadgen.py measure-files`), not
  approximated from row counts.

A control table, `bronze.g3_control` (`ops/g3/g3_loadgen.py
generate-control`), has the identical row totals and partitioning but ONE
commit per partition — i.e. the file layout compaction *would* produce.
This isolates "how much does small-file accumulation degrade planning
time" from "does the measured maintenance chain fix it" (it does not — see
below).

## Measurement: file count and query planning time, before/after maintenance

"Maintenance" here means the one working in-engine verb,
`expire_snapshots` (dry-run then applied), run via
`dagster/dispar_orchestrate/maintenance.py`'s `bronze_maintenance_job`.

| State | Data files (`bronze.g3_loadtest`, 14 partitions) | Planning-time proxy* (single-partition `COUNT`, `use_iceberg_metadata_files_cache=0`, avg of 5 reps) |
| --- | --- | --- |
| Before maintenance (20 files/partition) | 280 total, 20/partition | ~1.05s |
| After `expire_snapshots` (dry-run then applied) | 280 total, 20/partition — **unchanged** | ~1.05s — **unchanged** |
| Control (`bronze.g3_control`, 1 file/partition, same row totals) | 14 total, 1/partition | ~0.067s |
| After Trino `ALTER TABLE ... EXECUTE optimize` (escape hatch) | 14 total, 1/partition | ~0.053s |

\* "Query planning time" is proxied by wall-clock time of a
single-partition `SELECT count()` with the Iceberg metadata-files cache
disabled per query — this makes each run re-read that partition's manifest
list/manifests from scratch, which is where the planning cost of many
small files actually lives. `EXPLAIN`-level planning-only timing is not
exposed by ClickHouse for Iceberg tables; this is a defensible proxy,
documented as such rather than presented as an official `EXPLAIN` metric.

**`expire_snapshots` produced zero change in either metric** — expected,
since it reclaims old snapshots/manifests once they age out of
`iceberg_expire_default_max_snapshot_age_ms` (5 days by default); a
table whose data was all written in the last few minutes has nothing old
enough to expire, and even when it does, expiring old snapshot metadata
does not touch small *data* files at all. This is the direct, measured
confirmation of R2's escalation in `CLICKHOUSE-MAINTENANCE-FINDINGS.md`:
**no in-engine mechanism on ClickHouse 26.3 performs small-data-file
compaction against a catalog-registered Iceberg table.**

**Degradation ratio: ~15-20x** between the small-file state (280
files/14 partitions) and the control/compacted state (14 files) — **far
past the plan's 2x threshold.**

## Decision: Trino-as-cron escape hatch is REQUIRED

Per the plan's pre-authorized rule, this triggers the escape hatch. Before
committing it, Trino's ability to actually compact this exact table
through the exact same Lakekeeper/RustFS stack was verified directly (not
assumed):

```
$ trino --execute "ALTER TABLE iceberg.bronze.g3_loadtest EXECUTE optimize"
ALTER TABLE EXECUTE
"rewritten_data_files_count","280"
"removed_delete_files_count","0"
"added_data_files_count","14"
```

280 files -> 14 (exactly one per partition), and the planning-time proxy
dropped from ~1.05s to ~0.053s — matching the control table, i.e. full
recovery. See `docker-compose.yml`'s `trino` / `trino-maintenance-cron`
services (`trino` profile) and ADR 0009.

## Scope note

This is the one-time performance measurement that decided the escape
hatch. `ops/g3/g3_test.py` (the `g3-maintenance-test-runner` compose
service, run in CI as the `g3-maintenance` job) is a lighter, CI-repeatable
**functional** check — it proves the maintenance job runs and its metrics
surface through `GET /api/governance/maintenance`, not this phase's full
280-file synthetic load on every CI run (that would be slow and is not
needed once the gate decision above is made and recorded).
