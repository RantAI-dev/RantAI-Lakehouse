"""P4 Dagster maintenance job: the in-engine ClickHouse Iceberg maintenance
chain, run per Bronze table, on a schedule.

# ClickHouse 26.8 rework — this module was rebuilt after the 26.3 -> 26.8
# bump changed which verbs work

Every fact this module used to document was measured on
`clickhouse/clickhouse-server:26.3`. The PR stack review pointed out that
the locked decision was 26.8 LTS all along, and a full re-measurement
(`docs/plans/CLICKHOUSE-26.8-REMEASUREMENT.md`) against the SAME stack
(Lakekeeper 0.13.3, RustFS, format-version 2, OpenFGA enforced) found the
working verb set had changed underneath this job:

| Verb | 26.3 | 26.8 |
| --- | --- | --- |
| `expire_snapshots` | the only verb that worked | **`Code: 48. ... not supported for Iceberg tables backed by a transactional catalog.`** — a deliberate restriction (a REST catalog owns snapshot expiry, not the query engine), not a bug |
| `remove_orphan_files` | did not exist (`Code: 36. Unknown EXECUTE command`) | **works** — `ALTER TABLE t EXECUTE remove_orphan_files(dry_run=1/0)`, gated by `allow_database_iceberg=1, allow_insert_into_iceberg=1, allow_iceberg_remove_orphan_files=1` (a setting that did not exist on 26.3 either) |
| `OPTIMIZE` | `Code: 499` HTTP 403 at runtime | **returns OK** — but measured directly against a 7-small-file partition: seven files go in, seven files come out. It does **not** bin-pack. Not a compaction remedy at any version tested. |

**This job now runs `remove_orphan_files` (dry-run, then applied) as its
one active in-engine verb**, replacing `expire_snapshots`. `expire_snapshots`
is attempted once per table anyway — specifically so the skip is a REAL,
freshly-observed error every run (`probe_expire_snapshots_skip`), not a
hardcoded string that could silently go stale if some future ClickHouse
version changes the behavior again — and its failure is logged loudly and
recorded in `skipped_verbs`, never silently dropped. `OPTIMIZE` is
deliberately NOT invoked (calling it would succeed and do nothing useful —
see the table above): that is a logged design skip, not a caught runtime
error, and the log message says so explicitly rather than pretending it is
the same kind of skip as `expire_snapshots`'s.

# Lakekeeper-side snapshot expiry — explicitly a follow-up, not built here

`expire_snapshots`'s disappearance on 26.8 is "a REST catalog should own
snapshot expiry" made concrete: Lakekeeper's own management API almost
certainly exposes (or will expose) a maintenance/expiry call that does this
correctly against the catalog rather than the query engine. This module
does **not** implement that — it is out of scope for this fix and is
recorded here as a tracked gap, not a silent one. Until it lands, Bronze
tables accumulate snapshot/manifest history indefinitely (orphan *files*
are still reclaimed by `remove_orphan_files` above; orphan *snapshots* are
not reclaimed by anything in this stack).

# G3's consequence for this job is unchanged

`remove_orphan_files` reclaims files no manifest references — not
accumulated small data files that ARE referenced and simply never get
bin-packed. `OPTIMIZE` is the only verb that could compact, and it still
does not on 26.8 (measured directly, see the table above). So, exactly as
on 26.3, this job provides ZERO small-file mitigation;
`docs/plans/G3-RESULT.md` and `ops/g3/g3_loadgen.py` are what measure that
gap, and the Trino escape hatch (`docker-compose.yml`'s
`trino-maintenance-cron` service, ADR 0009) is what actually keeps Bronze
query-planning healthy. This job's job stays narrower: orphan-file and
(once expire_snapshots moves to the catalog) snapshot hygiene only.

# Bronze-only scoping

`discover_bronze_tables` filters `SHOW TABLES FROM {CATALOG_DB}` to names
starting with `bronze.` — the catalog's flat namespace also holds
`gold.*` (ADR 0010's Gold export target, a completely different job's
data) in the SAME flat `SHOW TABLES` listing, and prior to this fix
nothing filtered it out: applied maintenance ran against every namespace
the catalog knew about, `gold.*` included. See
`dagster/dispar_orchestrate/test_maintenance.py` for the regression test
proving a `gold.*` table is excluded.
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
    # R1 (ADR 0011): `expire_snapshots` is a genuine catalog metadata
    # write ClickHouse performs on this stack's behalf, authenticated as
    # `clickhouse-reader` (granted `modify` for exactly this — see
    # `docker-compose.yml`'s `lakekeeper-authz-init`). Empty on a pre-R1
    # or authz-disabled stack.
    ch_oauth_client_id: str
    ch_oauth_server_uri: str

    @classmethod
    def from_env(cls) -> "MaintenanceConfig":
        return cls(
            ch=ClickHouseTarget.from_env(),
            lakekeeper_catalog_uri=_env(
                "LAKEKEEPER_CATALOG_URI", "http://lakekeeper:8181/catalog"
            ),
            lakekeeper_warehouse=_env("LAKEKEEPER_WAREHOUSE", "default"),
            rustfs_endpoint=_env("CH_RUSTFS_S3_ENDPOINT", "http://rustfs:9000"),
            ch_oauth_client_id=_env("CH_OAUTH_CLIENT_ID", ""),
            ch_oauth_server_uri=_env("CH_OAUTH_SERVER_URI", ""),
        )

    def ch_auth_settings(self) -> str:
        if not self.ch_oauth_client_id:
            return ""
        return (
            f", catalog_credential = '{self.ch_oauth_client_id}:unused', "
            f"oauth_server_uri = '{self.ch_oauth_server_uri}'"
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
        f"storage_endpoint = '{cfg.rustfs_endpoint}'{cfg.ch_auth_settings()} "
        "SETTINGS allow_database_iceberg = 1",
    )


def discover_bronze_tables(cfg: MaintenanceConfig) -> list[str]:
    """Every table under the catalog's flat `bronze` namespace (ADR 0004) --
    discovered from the catalog itself (`SHOW TABLES`), not from
    `bronze_meta.dataset_catalog`, so a table that failed catalog
    registration but still exists in Lakekeeper is still maintained (the
    two failure modes are independent and this job should not silently
    skip a table just because the OTHER P3 step didn't run for it).

    `SHOW TABLES FROM {CATALOG_DB}` returns every namespace the catalog
    knows about in one flat list -- `bronze.*` AND `gold.*` (ADR 0010's
    Gold export target) side by side, since `DataLakeCatalog` has no
    per-namespace listing call, only a whole-database one. PR #30 review
    blocker: this used to return that whole list unfiltered, so applied
    maintenance ran against `gold.*` too. The `bronze.` prefix filter
    below is the fix; `test_maintenance.py::DiscoverBronzeTablesTest`
    is the regression test proving a `gold.*` table is excluded."""
    text = _ch_query(
        cfg, f"SHOW TABLES FROM {CATALOG_DB} SETTINGS allow_database_iceberg=1 FORMAT TabSeparated"
    )
    all_tables = [line.strip() for line in text.splitlines() if line.strip()]
    return [name for name in all_tables if name.startswith("bronze.")]


_ICEBERG_MAINT_SETTINGS = "allow_database_iceberg=1, allow_insert_into_iceberg=1"

_REMOVE_ORPHAN_FILES_SETTINGS = f"{_ICEBERG_MAINT_SETTINGS}, allow_iceberg_remove_orphan_files=1"

_EXPIRE_SNAPSHOTS_SETTINGS = f"{_ICEBERG_MAINT_SETTINGS}, allow_experimental_expire_snapshots=1"

# `remove_orphan_files`'s result-set column names, verified empirically
# against a live ClickHouse 26.8 + Lakekeeper stack (see the module doc).
# A superset of `expire_snapshots`'s old columns (adds
# `deleted_metadata_files_count`/`skipped_missing_metadata_count`/
# `failed_deletions_count`) — `bronze_catalog.record_maintenance_run` only
# persists the three columns it always has (data/manifest files, manifest
# lists), so the extra ones are surfaced in the op's own return value/logs
# without needing a registry schema change.
_ROF_COUNT_COLUMNS = (
    "deleted_data_files_count",
    "deleted_position_delete_files_count",
    "deleted_equality_delete_files_count",
    "deleted_manifest_files_count",
    "deleted_manifest_lists_count",
    "deleted_metadata_files_count",
    "deleted_statistics_files_count",
    "skipped_missing_metadata_count",
    "failed_deletions_count",
)


def _parse_tsv_kv(text: str) -> dict[str, int]:
    """`ALTER TABLE ... EXECUTE <verb>()` returns a `key\\tvalue`
    two-column result set (verified empirically — see module doc), not the
    usual query row shape. `FORMAT TabSeparated` on an ALTER EXECUTE
    statement returns exactly this, for both `remove_orphan_files` and the
    old `expire_snapshots`."""
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


def run_remove_orphan_files(
    cfg: MaintenanceConfig, table_name: str, *, dry_run: bool
) -> dict[str, Any]:
    """Run (or dry-run) `remove_orphan_files` against one catalog-registered
    Bronze table — the working verb on ClickHouse 26.8 (did not exist on
    26.3). `table_name` is the catalog's own two-part name, e.g.
    `bronze.g3a_orders` (already backtick-safe: no user input reaches this,
    only names `discover_bronze_tables` returned, which is itself filtered
    to the `bronze.` prefix)."""
    dry_run_arg = "dry_run=1" if dry_run else "dry_run=0"
    sql = (
        f"ALTER TABLE {CATALOG_DB}.`{table_name}` "
        f"EXECUTE remove_orphan_files({dry_run_arg}) "
        f"SETTINGS {_REMOVE_ORPHAN_FILES_SETTINGS} FORMAT TabSeparated"
    )
    text = _ch_query(cfg, sql)
    counts = _parse_tsv_kv(text)
    return {
        "table_name": table_name,
        "dry_run": dry_run,
        **{k: counts.get(k, 0) for k in _ROF_COUNT_COLUMNS},
    }


def probe_expire_snapshots_skip(cfg: MaintenanceConfig, table_name: str) -> str:
    """Attempt `expire_snapshots(dry_run=1)` against `table_name` and
    return the REAL error text ClickHouse gives THIS run, rather than a
    hardcoded string that could silently drift out of sync with what the
    server actually says (the exact trap the review flagged: "a loud,
    logged skip carrying the real error — never a silent no-op"). On
    ClickHouse 26.8 this is `Code: 48 ... not supported for Iceberg tables
    backed by a transactional catalog` — a deliberate restriction (a REST
    catalog owns snapshot expiry, not the query engine), not a bug; see
    `docs/plans/CLICKHOUSE-26.8-REMEASUREMENT.md`. If some future
    ClickHouse version makes this succeed again, that is reported loudly
    too (the caller logs whatever this returns either way — this function
    never swallows the outcome, it just describes it)."""
    sql = (
        f"ALTER TABLE {CATALOG_DB}.`{table_name}` "
        f"EXECUTE expire_snapshots(dry_run=1) "
        f"SETTINGS {_EXPIRE_SNAPSHOTS_SETTINGS} FORMAT TabSeparated"
    )
    try:
        text = _ch_query(cfg, sql)
    except requests.HTTPError as exc:
        body = exc.response.text.strip() if exc.response is not None else str(exc)
        return f"unsupported this run — real ClickHouse error: {body}"
    return (
        f"unexpectedly SUCCEEDED this run (result: {text.strip()!r}) — this "
        "ClickHouse version may no longer restrict expire_snapshots against "
        "a transactional catalog; re-check docs/plans/"
        "CLICKHOUSE-26.8-REMEASUREMENT.md before trusting this as a skip"
    )


@op
def run_bronze_maintenance(context) -> list[dict[str, Any]]:
    """The P4 maintenance chain, per Bronze table: `remove_orphan_files`
    dry-run (metrics only, matching the task brief's "dry_run metrics
    surfaced in console" requirement) then the real run — the working verb
    on ClickHouse 26.8. `expire_snapshots` is attempted and its real,
    per-run failure logged (never a hardcoded skip reason); `OPTIMIZE` is
    deliberately never invoked at all (see module doc) and that is logged
    as the design decision it is, distinct from `expire_snapshots`'s
    genuine runtime rejection."""
    cfg = MaintenanceConfig.from_env()
    _ensure_catalog_database(cfg)
    tables = discover_bronze_tables(cfg)
    context.log.info(f"discovered {len(tables)} Bronze table(s): {tables}")

    results: list[dict[str, Any]] = []
    for table_name in tables:
        expire_skip_reason = probe_expire_snapshots_skip(cfg, table_name)
        context.log.warning(f"[{table_name}] skipping expire_snapshots: {expire_skip_reason}")
        context.log.warning(
            f"[{table_name}] not running OPTIMIZE: parses and returns OK on "
            "ClickHouse 26.8, but measured directly against a small-file "
            "partition it does NOT bin-pack (files in == files out) — not a "
            "compaction remedy at any version tested. Small-file compaction "
            "stays out-of-engine via trino-maintenance-cron per ADR 0009."
        )

        dry = run_remove_orphan_files(cfg, table_name, dry_run=True)
        context.log.info(f"[dry-run] {table_name}: {dry}")
        real = run_remove_orphan_files(cfg, table_name, dry_run=False)
        context.log.info(f"[applied] {table_name}: {real}")
        record_maintenance_run(
            table_name=table_name,
            dry_run_metrics=dry,
            applied_metrics=real,
            skipped_verbs=[
                f"expire_snapshots ({expire_skip_reason})",
                "OPTIMIZE (returns OK on 26.8 but does not bin-pack — not a "
                "compaction remedy; Trino owns that, see ADR 0009)",
            ],
            target=cfg.ch,
        )
        results.append(
            {
                "table_name": table_name,
                "dry_run": dry,
                "applied": real,
                "expire_snapshots_skip_reason": expire_skip_reason,
            }
        )

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
