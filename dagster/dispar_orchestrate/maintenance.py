"""P4 Dagster maintenance job: the in-engine ClickHouse Iceberg maintenance
chain, run per Bronze table, on a schedule.

# What this actually runs, and why it is ONE verb, not three or four

The build brief's original chain was four commands: `expire_snapshots` ->
`remove_orphan_files` -> `OPTIMIZE` -> `OPTIMIZE ... MANIFEST`.
`docs/plans/CLICKHOUSE-MAINTENANCE-FINDINGS.md` had already cut that to
three (`OPTIMIZE ... MANIFEST` is a `SYNTAX_ERROR` on 26.3) and flagged the
remaining three as unverified against a real catalog-registered Iceberg
table. This module is the result of that P4 measurement, run against a
real Lakekeeper-registered Bronze table (`bronze.g3a_orders` and a
synthetic `bronze.g3_loadtest`) on `clickhouse/clickhouse-server:26.3`:

| Verb | Result |
| --- | --- |
| `expire_snapshots` | **Works.** `ALTER TABLE t EXECUTE expire_snapshots()`, gated by `allow_database_iceberg=1, allow_insert_into_iceberg=1, allow_experimental_expire_snapshots=1`. Also accepts a keyword argument `expire_snapshots(dry_run=1)` returning the same result-set shape (deletion counts) without deleting anything — **this, not `OPTIMIZE ... DRY RUN`, is the dry-run mechanism** (see below). |
| `remove_orphan_files` | **Does not exist for Iceberg tables.** `Code: 36. DB::Exception: Unknown EXECUTE command 'remove_orphan_files' for Iceberg table. (BAD_ARGUMENTS)` — a specific per-engine dispatch rejection, not a generic NOT_IMPLEMENTED. The verb is real syntax (dispatched generically by `ALTER TABLE ... EXECUTE`) but Iceberg's own dispatcher does not implement it, full stop. |
| `OPTIMIZE` (compaction) | Parses, gated by `allow_insert_into_iceberg=1, allow_experimental_iceberg_compaction=1`, but **fails at runtime**: `Code: 499. DB::Exception: Failed to get object info: No response body.. HTTP response code: 403.` This reproduces identically with vended catalog credentials AND with static admin RustFS credentials with full bucket access (`aws-cli` against the exact same object, same credentials, succeeds) — this rules out a permissions/credentials problem and points to a genuine bug in ClickHouse's Iceberg `OPTIMIZE` write path against a REST-catalog-registered table on 26.3. |

So **this job runs exactly one verb: `expire_snapshots`.** It does NOT run
`remove_orphan_files` (does not exist for Iceberg) or `OPTIMIZE`
(exists but errors on every attempt) — running either would be exactly
the "build a job that silently no-ops" the task brief forbids. Each
skipped verb is logged explicitly, with the real error, every run — see
`_run_expire_snapshots`'s caller in `run_bronze_maintenance`.

# `OPTIMIZE ... DRY RUN` is NOT the dry-run mechanism for Iceberg

`CLICKHOUSE-MAINTENANCE-FINDINGS.md` said "`DRY RUN` **is** accepted --
that is the mechanism for P4's dry_run metrics requirement." Measured
against a real Iceberg table, this is WRONG and is corrected here:
`OPTIMIZE TABLE t DRY RUN` is grammatically valid only as `DRY RUN PARTS
'<part list>'` (a MergeTree-specific clause), and even then:
`Code: 36. DB::Exception: OPTIMIZE DRY RUN is only supported for MergeTree
family tables. (BAD_ARGUMENTS)`. The actual, working dry-run mechanism for
Iceberg maintenance on 26.3 is `expire_snapshots(dry_run=1)` — confirmed
empirically to return the same `deleted_*_count`/`dry_run` result-set shape
without deleting anything (`dry_run` echoes back `1` in the result).

# G3's consequence for this job

Because none of the three measured verbs perform small-file compaction --
`expire_snapshots` reclaims old snapshots/manifests, not accumulated small
data files, and `OPTIMIZE` (the only verb that could) does not work -- this
job provides ZERO small-file mitigation. `docs/plans/G3-RESULT.md` measured
this directly: with no working compaction, planning time on a
20-small-file partition was ~15-22x a 1-file-per-partition baseline, far
past the 2x threshold. The escape hatch (Trino-as-cron running `optimize`
on Bronze only, `docker-compose.yml`'s `trino-maintenance-cron` service)
is what actually keeps Bronze query-planning healthy; this job's job is
narrower: snapshot/manifest hygiene only. See ADR 0009.
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from typing import Any

import requests
from dagster import Definitions, ScheduleDefinition, job, op

from dispar_orchestrate.bronze_catalog import ClickHouseTarget, record_maintenance_run

CATALOG_DB = "icecat_maintenance"


def _env(name: str, default: str) -> str:
    value = os.environ.get(name, "").strip()
    return value if value else default


@dataclass(frozen=True)
class MaintenanceConfig:
    ch: ClickHouseTarget
    lakekeeper_catalog_uri: str
    lakekeeper_warehouse: str
    rustfs_endpoint: str

    @classmethod
    def from_env(cls) -> "MaintenanceConfig":
        return cls(
            ch=ClickHouseTarget.from_env(),
            lakekeeper_catalog_uri=_env(
                "LAKEKEEPER_CATALOG_URI", "http://lakekeeper:8181/catalog"
            ),
            lakekeeper_warehouse=_env("LAKEKEEPER_WAREHOUSE", "default"),
            rustfs_endpoint=_env("CH_RUSTFS_S3_ENDPOINT", "http://rustfs:9000"),
        )


def _ch_query(cfg: MaintenanceConfig, sql: str) -> str:
    resp = requests.post(
        cfg.ch.url, auth=(cfg.ch.user, cfg.ch.password), data=sql.encode("utf-8"), timeout=60
    )
    resp.raise_for_status()
    return resp.text


def _ensure_catalog_database(cfg: MaintenanceConfig) -> None:
    """A dedicated `DataLakeCatalog` database for maintenance, separate
    from any per-test catalog database (`icecat`/`icecat_g3a` in
    `ops/g3a/g3a_test.py`) so a maintenance run never depends on test
    fixtures having created one first."""
    _ch_query(
        cfg,
        f"CREATE DATABASE IF NOT EXISTS {CATALOG_DB} "
        f"ENGINE = DataLakeCatalog('{cfg.lakekeeper_catalog_uri}') "
        f"SETTINGS catalog_type = 'rest', warehouse = '{cfg.lakekeeper_warehouse}', "
        f"storage_endpoint = '{cfg.rustfs_endpoint}' "
        "SETTINGS allow_database_iceberg = 1",
    )


def discover_bronze_tables(cfg: MaintenanceConfig) -> list[str]:
    """Every table under the catalog's flat `bronze` namespace (ADR 0004) --
    discovered from the catalog itself (`SHOW TABLES`), not from
    `bronze_meta.dataset_catalog`, so a table that failed catalog
    registration but still exists in Lakekeeper is still maintained (the
    two failure modes are independent and this job should not silently
    skip a table just because the OTHER P3 step didn't run for it)."""
    text = _ch_query(
        cfg, f"SHOW TABLES FROM {CATALOG_DB} SETTINGS allow_database_iceberg=1 FORMAT TabSeparated"
    )
    return [line.strip() for line in text.splitlines() if line.strip()]


_EXPIRE_SNAPSHOTS_SETTINGS = (
    "allow_database_iceberg=1, allow_insert_into_iceberg=1, "
    "allow_experimental_expire_snapshots=1"
)

_COUNT_COLUMNS = (
    "deleted_data_files_count",
    "deleted_position_delete_files_count",
    "deleted_equality_delete_files_count",
    "deleted_manifest_files_count",
    "deleted_manifest_lists_count",
    "deleted_statistics_files_count",
)


def _parse_expire_snapshots_result(text: str) -> dict[str, int]:
    """`ALTER TABLE ... EXECUTE expire_snapshots()` returns a `key\\tvalue`
    two-column result set (verified empirically — see module doc), not the
    usual query row shape. `FORMAT TabSeparated` on an ALTER EXECUTE
    statement returns exactly this."""
    counts: dict[str, int] = {}
    for line in text.splitlines():
        if not line.strip():
            continue
        key, _, value = line.partition("\t")
        try:
            counts[key] = int(value)
        except ValueError:
            counts[key] = 0
    return counts


def run_expire_snapshots(
    cfg: MaintenanceConfig, table_name: str, *, dry_run: bool
) -> dict[str, Any]:
    """Run (or dry-run) `expire_snapshots` against one catalog-registered
    Bronze table. `table_name` is the catalog's own two-part name, e.g.
    `bronze.g3a_orders` (already backtick-safe: no user input reaches this,
    only names `SHOW TABLES` returned)."""
    dry_run_arg = "dry_run=1" if dry_run else "dry_run=0"
    sql = (
        f"ALTER TABLE {CATALOG_DB}.`{table_name}` "
        f"EXECUTE expire_snapshots({dry_run_arg}) "
        f"SETTINGS {_EXPIRE_SNAPSHOTS_SETTINGS} FORMAT TabSeparated"
    )
    text = _ch_query(cfg, sql)
    counts = _parse_expire_snapshots_result(text)
    return {
        "table_name": table_name,
        "dry_run": dry_run,
        **{k: counts.get(k, 0) for k in _COUNT_COLUMNS},
    }


@op
def run_bronze_maintenance(context) -> list[dict[str, Any]]:
    """The P4 maintenance chain, per Bronze table: a dry-run
    `expire_snapshots` pass (metrics only, matching the task brief's
    "dry_run metrics surfaced in console" requirement), then the real run.
    `remove_orphan_files` and `OPTIMIZE` are explicitly NOT run — see the
    module doc for why — and every run logs that omission plainly rather
    than silently skipping it."""
    cfg = MaintenanceConfig.from_env()
    _ensure_catalog_database(cfg)
    tables = discover_bronze_tables(cfg)
    context.log.info(f"discovered {len(tables)} Bronze table(s): {tables}")
    context.log.warning(
        "skipping remove_orphan_files (does not exist for Iceberg tables on "
        "ClickHouse 26.3: 'Unknown EXECUTE command remove_orphan_files for "
        "Iceberg table') and OPTIMIZE compaction (parses but fails at "
        "runtime with an S3 403 on this ClickHouse version — see "
        "docs/plans/G3-RESULT.md). Only expire_snapshots runs in-engine; "
        "small-file compaction is handled out-of-engine by "
        "trino-maintenance-cron per ADR 0009."
    )

    results: list[dict[str, Any]] = []
    for table_name in tables:
        dry = run_expire_snapshots(cfg, table_name, dry_run=True)
        context.log.info(f"[dry-run] {table_name}: {dry}")
        real = run_expire_snapshots(cfg, table_name, dry_run=False)
        context.log.info(f"[applied] {table_name}: {real}")
        record_maintenance_run(
            table_name=table_name,
            dry_run_metrics=dry,
            applied_metrics=real,
            skipped_verbs=[
                "remove_orphan_files (unsupported for Iceberg tables)",
                "OPTIMIZE (S3 403 on ClickHouse 26.3 Iceberg compaction path)",
            ],
            target=cfg.ch,
        )
        results.append({"table_name": table_name, "dry_run": dry, "applied": real})

    context.add_output_metadata({"tables_maintained": len(results)})
    return results


@job
def bronze_maintenance_job() -> None:
    """`DAGSTER_LOCATION`-visible job name: `bronze_maintenance_job`.
    Launched the same way `bronze_ingest_job` (P3) is — no Rust-side
    special-casing needed."""
    run_bronze_maintenance()


# Daily at 03:00 — arbitrary but conservative cadence for a Bronze table
# under active CDC/dlt writes; `dagster-daemon` (already in the P3 compose
# topology, see ADR 0005) is what actually fires this.
bronze_maintenance_schedule = ScheduleDefinition(
    job=bronze_maintenance_job,
    cron_schedule="0 3 * * *",
)

maintenance_defs = Definitions(
    jobs=[bronze_maintenance_job],
    schedules=[bronze_maintenance_schedule],
)
