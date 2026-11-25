"""P4 Dagster maintenance job: the ClickHouse Iceberg maintenance chain —
the in-engine verbs plus a catalog-side snapshot expiry — run per Bronze
table, on a schedule.

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
one active in-engine verb**, replacing ClickHouse's own `expire_snapshots`.
ClickHouse's `expire_snapshots` is still ATTEMPTED once per table
(`probe_expire_snapshots_skip`), but only to OBSERVE it: on 26.8 the
server refuses its own verb against a catalog-backed table, and the point
of the probe is that this refusal is a REAL, freshly-observed error every
run rather than a hardcoded string that could silently go stale if some
future ClickHouse version changes the behavior again. The observation goes
to the Dagster run log, per table, per run — and NOWHERE else: it is
ClickHouse's own error body, so it does NOT go into `skipped_verbs`, into
a `maintenance_verb_run` / `maintenance_run` row, or into any API
response (`GET /api/governance/maintenance`,
`GET /api/lakehouse/tables/{ns}/{table}/maintenance`) — upstream error
text never reaches a response.
The expiry this job performs is not ClickHouse's: it runs through
Trino's Iceberg connector when Trino answers and, when Trino is unreachable,
through the REST catalog (`pyiceberg`, against the same Lakekeeper catalog
the ingest job writes through — the `dlt` principal), so "trino unreachable"
is no longer the outcome of this verb, and the expiry's own recorded skip
reason comes from the engine that ran it (Trino or the catalog), never
from this probe. `OPTIMIZE` is deliberately NOT invoked (calling it would
succeed and do nothing useful — see the table above): that is a design
skip of this job's OWN work, recorded in `skipped_verbs` like every other
skip this job records — not an observation of another engine's refusal
like the probe above is.

# Lakekeeper-side snapshot expiry — investigated, NOT wired in, and why

`expire_snapshots`'s disappearance on 26.8 is "a REST catalog should own
snapshot expiry" made concrete, so this fix investigated whether Lakekeeper
(pinned `quay.io/lakekeeper/catalog:v0.13.3` in `docker-compose.yml`)
actually exposes a maintenance/expiry call — not a guess: Lakekeeper's own
docs (`table-maintenance`, present since v0.10.0, well before the pinned
v0.13.3) document a real Management API surface for this:

```
GET  /management/v1/warehouse/{warehouse_id}/task-queue/expire_snapshots/config
POST /management/v1/warehouse/{warehouse_id}/task-queue/expire_snapshots/config
```

taking `enable-expire-snapshots` / `max-snapshot-age-ms` /
`min-snapshots-to-keep` / `min-snapshots-to-expire` / `max-ref-age-ms`.

**This module does NOT call it, for two concrete reasons, not laziness:**

1. **Wrong shape.** It is a warehouse-wide "turn on Lakekeeper's own
   background expiry task queue" TOGGLE — Lakekeeper then expires
   snapshots itself, asynchronously, on its own schedule after future
   commits. There is no documented per-table, per-run, dry-run-then-apply
   call that would slot into this job's existing `dry_run` → `apply` →
   `record_maintenance_run` shape the way `run_remove_orphan_files` does;
   forcing it into that shape would mean POSTing the same idempotent
   warehouse-wide config on every scheduled run, which is a materially
   different (and riskier) operation than "run this verb against this
   table and report what it did."
2. **Wrong principal.** Calling it needs `warehouse_id` (resolved via
   `GET /management/v1/warehouse`, an ADMIN-scoped list call — see
   `lakekeeper-authz-init` in `docker-compose.yml`, which is the ONLY
   place `admin.jwt` is mounted) plus, almost certainly, an `admin`-level
   grant on the warehouse to change its task-queue config. This job holds
   two identities, and NEITHER is admin-level: `clickhouse-reader`
   (`select`+`modify` — granted for ClickHouse's OWN `expire_snapshots`
   attempt above, now failing) and the `dlt` principal
   (warehouse-scoped `create modify select`, ADR 0011 — the identity the
   catalog-side expiry above writes through, its token pre-minted and
   mounted read-only at `/tokens/dlt.jwt`). `dagster-code-location` (where
   this op runs) mounts no admin token, so Lakekeeper's task-queue API
   stays out of reach for this job.
   Minting one for a recurring, unattended scheduled job — rather than the
   one-shot bootstrap job that currently holds it — is a real
   authorization-surface change this fix does not make unreviewed, and
   this repo has no measured proof (no live-stack run against v0.13.3)
   that `clickhouse-reader`'s existing grant would even be accepted by
   that endpoint if it somehow were used.

So: an actionable, cited path exists for a future fix (enable
`expire_snapshots` task-queue config once per warehouse, from a principal
with the right grant — mirroring how `lakekeeper-authz-init` already does
one-shot admin-scoped setup), but it is not built here. What IS built here,
per the brief's explicit fallback for exactly this situation ("add an
explicit, monitored gap"), is `measure_snapshot_growth` below: every run,
per Bronze table, it reads the table's OWN metadata document straight from
Lakekeeper's Iceberg REST catalog (`GET /v1/{prefix}/namespaces/{ns}/
tables/{table}` — the SAME endpoint `ops/g3a/g3a_test.py`'s
`step_verify_format_version_2` already uses and has proven works against
this stack) and records `len(metadata["snapshots"])` and
`len(metadata["metadata-log"])` through `record_maintenance_run`, so the
snapshot growth this section describes is VISIBLE and trending in
`GET /api/governance/maintenance` history — not silent — even though
Lakekeeper's own background expiry task (the `enable-expire-snapshots`
toggle above) stays off. Snapshot METADATA is reclaimed in this stack, but
by this job's own per-run, policy-driven expiry through the REST catalog,
not by Lakekeeper's task queue. Orphan *files* are still reclaimed by
`remove_orphan_files` above — the files an expired snapshot referenced are
among them, and that verb deletes them about three days later, under its
3-day age floor.

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
snapshot hygiene only. `expire_snapshots` HAS moved to the catalog — the
expiry runs through Trino when Trino answers and through `pyiceberg` when
it does not — so snapshot hygiene is covered here too; compaction (small
files) is still Trino-only (`trino-maintenance-cron`, ADR 0009), which is
what OPS-21 is about.

# Bronze-only scoping

`discover_bronze_tables` filters `SHOW TABLES FROM {CATALOG_DB}` to names
starting with `bronze.` — the catalog's flat namespace also holds
`gold.*` (ADR 0010's Gold export target, a completely different job's
data) in the SAME flat `SHOW TABLES` listing, and prior to this fix
nothing filtered it out: applied maintenance ran against every namespace
the catalog knew about, `gold.*` included. See
`dagster/dispar_orchestrate/test_maintenance.py` for the regression test
proving a `gold.*` table is excluded.

# The pause: `MAINTENANCE_PAUSED`

The two verbs here that DELETE files, or make them reclaimable — the
snapshot expiry and the orphan-file removal — do not run while
`MAINTENANCE_PAUSED=true` is set: they skip, the reason is recorded in
`skipped_verbs`, and the run does not fail (REL-2 S4 decision D16). The
catalog-side expiry deletes no file by itself — it removes snapshot
metadata, and the files it frees are the orphan verb's to delete — so
both verbs stay under the pause.

Why: the local backup does NOT hold the warehouse files. A restore
brings the catalogs, Postgres and ClickHouse back to the moment of the
backup, while the files written after it stay as orphans — and a job
that deletes a file the RESTORED catalog still names breaks the table
it names. So `lakehousectl` sets the pause before the backup of an
update and clears it after the update succeeds or is rolled back. The
Gold export's trim deletes files too, so it reads the same variable
(`gold_export.py`).

# Per-table policy + maintenance verbs

The verbs above are the job's fixed, unconditional behavior. Operators
who want to bin-pack small files need `table_maintenance_policy`
(`rust/migrations/0030`), read once per run from
`GET /api/lakehouse/maintenance-policies`. Its `optimize` is Trino-only —
Trino's Iceberg connector is the only engine here that genuinely bin-packs
(ClickHouse's does not — see the table above) — while its
`expire_snapshots` runs through that connector when Trino answers and
through the REST catalog (`pyiceberg`) when Trino does not. Trino's
connector does not carry ClickHouse's "transactional catalog" restriction
on `expire_snapshots`.

Snapshot expiry (`expire_snapshots`) runs by default for every Bronze
table with a 7-day retention threshold (Trino's own floor, OPS-1 Decision
D1) — the same number whichever engine runs it: Trino's connector when
Trino answers, the REST catalog when it does not. Operators who want to
retain fewer or more snapshots configure `snapshotsToKeep` in
`table_maintenance_policy`, which overrides the default (D2).

* **`snapshotsToKeep`.** Trino 483's `expire_snapshots` procedure takes
  only a `retention_threshold` age, not a keep-count, so keeping the N
  newest snapshots means computing an age: read every snapshot's
  `committed_at` (newest first) from Trino's `"<table>$snapshots"`
  metadata table, and if there are more than N, the threshold is the AGE
  of the Nth-newest snapshot, in whole seconds rounded up. Exactly N
  survive — the catalog path computes the SAME age and then keeps the
  newest N by id, and it additionally protects every branch and tag head
  (`pyiceberg`'s `ExpireSnapshots.by_id` refuses to expire one). A table
  with N or fewer snapshots has nothing to expire and the verb is skipped,
  not run with a threshold of zero — a `'0d'` threshold (the naive reading
  of "expire snapshots older than everything") would expire every snapshot
  but the current one, regardless of how many the operator asked to keep,
  which is real data loss for anyone who configured a keep-count greater
  than one. Trino's `iceberg.expire-snapshots.min-retention` (a 7-day
  default, unset in this stack's compose) can also refuse a threshold
  shorter than that floor; the refusal is recorded honestly and never
  retried with a larger value — lowering the floor is a
  deployment/product decision, not something this job decides for an
  operator.
* **`orphanAgeHours`.** `remove_orphan_files` DOES take an age: the
  `iceberg_orphan_files_older_than_seconds` setting, which
  `run_remove_orphan_files` above can set. Its ClickHouse default is
  3 days — 259200 s, measured in this repo at
  `docs/superpowers/plans/2026-10-14-data-2-gold-keep-copies.md:245-246`
  — and this job calls the verb without overriding that setting, so
  files younger than 3 days are protected and the reclaim lands about
  three days later. Setting this field is recorded, honestly, as
  "stored; not applied by this build" — never silently ignored, and
  never applied on a guess.
* **`schedule`.** A per-table cadence for the expiry and compaction verbs
  only (the ClickHouse-side steps above stay unconditional, every run):
  `weekly` requires the table's last recorded verb run to be at least 7 days
  old, `daily` at least 20 hours (tolerating this job's own start jitter —
  see `bronze_maintenance_schedule` below), `null` runs every time.
* **`compactSmallFiles`.** Runs Trino's `ALTER TABLE ... EXECUTE
  optimize` — the same call `ops/trino/optimize_bronze.sh` already
  proves works against this stack's Iceberg tables.

`_trino_execute` below is a deliberate, small duplication of
`rust/crates/lakehouse-trino`'s own `/v1/statement` polling loop: this
job runs inside `dagster-code-location`, a separate Python process
outside the Rust binary that crate is built into, so there is no way to
share that implementation from here. Every statement is sent as
`X-Trino-User: trino-maintenance` — Trino's file-based access control
(`docker-compose.yml`'s `trino` service) grants that user `all` on the
`iceberg` catalog and every other user only `read-only`.

Per-verb outcomes (`applied`/`refused`/`skipped`/`failed`) are recorded
in a NEW table, `bronze_meta.maintenance_verb_run`
(`bronze_catalog.record_maintenance_verb_run`), rather than as new
columns on the existing `bronze_meta.maintenance_run` table: that
table's schema (`bronze_catalog._MAINTENANCE_RUN_SCHEMA`) is guarded by
an R10 schema-drift check with no additive path (`_assert_or_create_schema`
either creates a table fresh or requires an EXISTING one's columns to
match exactly), so widening it would raise `SchemaDriftError` on every
deployment that already has this table — the demo included — on the
very next scheduled run, stopping maintenance entirely. A brand-new
table has no such deployment to break.
"""

from __future__ import annotations

import math
import os
import re
import time
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from typing import Any

import requests
from dagster import (
    DefaultScheduleStatus,
    Definitions,
    DynamicOut,
    DynamicOutput,
    Failure,
    ScheduleDefinition,
    job,
    op,
)

from dispar_orchestrate.bronze_catalog import (
    ClickHouseTarget,
    latest_maintenance_run_at,
    record_maintenance_run,
    record_maintenance_verb_run,
)
from dispar_orchestrate.ingest_factory import job_tags
from dispar_orchestrate.op_metadata import DEFAULT_RETRY_POLICY, source_metadata

CATALOG_DB = "icecat_maintenance"

# How long one run of this job may run, in seconds. A daily maintenance
# pass fans out over every Bronze table, so it gets room for a slow day:
# the longest run measured on staging on 2026-11-16 was 2674 s and the
# limit is twice that, far below the six hours a load job gets.
RUN_LIMIT_ENV = "DAGSTER_RUN_LIMIT_BRONZE_MAINTENANCE_JOB_SECONDS"
DEFAULT_RUN_LIMIT_SECONDS = 10_800

# The environment variable that pauses the maintenance jobs (REL-2 S4
# decision D16).
MAINTENANCE_PAUSED_VARIABLE = "MAINTENANCE_PAUSED"


def maintenance_paused() -> bool:
    """Whether the jobs that delete files are paused.

    `MAINTENANCE_PAUSED=true` pauses the two verbs that DELETE files:
    the snapshot expiry and the orphan-file removal. While it is set
    they skip, the reason is recorded in `skipped_verbs`, and the run
    does not fail.

    Why it exists: the local backup does NOT hold the warehouse files.
    A restore brings the catalogs, Postgres and ClickHouse back to the
    moment of the backup, while the files written after it stay as
    orphans — and a job that deletes a file the RESTORED catalog still
    names breaks the table it names. So `lakehousectl` sets the pause
    before the backup of an update and clears it after the update
    succeeds or is rolled back (REL-2 S4 decision D16).

    The value is read as `== "true"`: anything else is not a pause, and
    an unset variable is not a pause.
    """
    return os.environ.get(MAINTENANCE_PAUSED_VARIABLE, "").strip() == "true"


def _env(name: str, default: str) -> str:
    value = os.environ.get(name, "").strip()
    return value if value else default


def _read_token_file(path: str) -> str:
    """The bearer token in `path`, or `""` when there is no such file.

    A missing file is a stack without that identity, not an error: the
    caller records an honest skip. The file is read rather than an env
    var because this container's `command:` is a fixed exec array, not
    a shell script (see `MaintenanceConfig`'s comment)."""
    if not path or not os.path.exists(path):
        return ""
    with open(path, encoding="utf-8") as handle:
        return handle.read().strip()


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
    # Per-table policy + maintenance verbs (module doc, "Per-table
    # policy + maintenance verbs"). Defaulted so every existing
    # positional/keyword construction of this dataclass elsewhere (this
    # module's own test module included) keeps working unchanged.
    api_url: str = "http://lakehouse-api:8080"
    maintenance_token: str = ""
    trino_url: str = "http://trino:8080"
    # The `dlt` principal's pre-minted Lakekeeper token (ADR 0011) —
    # the identity the ingest job already writes through, and the one
    # the pyiceberg expiry fallback uses. It is read from a FILE, not
    # from an env var, for the same reason `dlt_pipeline.py` reads it
    # from one: this container's `command:` is a fixed exec array, not
    # a shell script, so there is no step between container start and
    # this code to interpolate a file's contents into an env var. Both
    # fields default to empty so a stack that mounts no token file
    # (pre-R1 or authz-disabled) keeps working: the fallback then
    # records an honest skip, never a crash.
    lakekeeper_token_file: str = ""
    lakekeeper_token: str = ""
    # The object-store credentials the catalog needs to read a table's
    # metadata and to commit an expiry — the same connector-scoped
    # `CONNECTOR_S3_*` pair the ingest job already uses.
    rustfs_access_key: str = ""
    rustfs_secret_key: str = ""

    @classmethod
    def from_env(cls) -> "MaintenanceConfig":
        token_file = _env("LAKEKEEPER_TOKEN_FILE", "")
        return cls(
            ch=ClickHouseTarget.from_env(),
            lakekeeper_catalog_uri=_env(
                "LAKEKEEPER_CATALOG_URI", "http://lakekeeper:8181/catalog"
            ),
            lakekeeper_warehouse=_env("LAKEKEEPER_WAREHOUSE", "default"),
            rustfs_endpoint=_env("CH_RUSTFS_S3_ENDPOINT", "http://rustfs:9000"),
            ch_oauth_client_id=_env("CH_OAUTH_CLIENT_ID", ""),
            ch_oauth_server_uri=_env("CH_OAUTH_SERVER_URI", ""),
            api_url=_env("LAKEHOUSE_API_URL", "http://lakehouse-api:8080"),
            maintenance_token=_env("LAKEHOUSE_MAINTENANCE_TOKEN", ""),
            trino_url=_env("TRINO_URL", "http://trino:8080"),
            lakekeeper_token_file=token_file,
            lakekeeper_token=_read_token_file(token_file),
            rustfs_access_key=_env("CONNECTOR_S3_ACCESS_KEY", ""),
            rustfs_secret_key=_env("CONNECTOR_S3_SECRET_KEY", ""),
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


def ensure_catalog_database(cfg: Any, db_name: str = CATALOG_DB) -> None:
    """A dedicated `DataLakeCatalog` database for maintenance/export jobs, separate
    from any per-test catalog database (`icecat`/`icecat_g3a` in
    `ops/g3a/g3a_test.py`) so a run never depends on test
    fixtures having created one first."""
    auth = cfg.ch_auth_settings() if hasattr(cfg, "ch_auth_settings") else ""
    _ch_query(
        cfg,
        f"CREATE DATABASE IF NOT EXISTS {db_name} "
        f"ENGINE = DataLakeCatalog('{cfg.lakekeeper_catalog_uri}') "
        f"SETTINGS catalog_type = 'rest', warehouse = '{cfg.lakekeeper_warehouse}', "
        f"storage_endpoint = '{cfg.rustfs_endpoint}'{auth} "
        "SETTINGS allow_database_iceberg = 1",
    )


def _ensure_catalog_database(cfg: MaintenanceConfig) -> None:
    ensure_catalog_database(cfg, CATALOG_DB)


def discover_catalog_tables(cfg: MaintenanceConfig) -> list[str]:
    """Every table the catalog knows about, as the catalog's own flat
    two-part names (`bronze.orders`, `gold.mart_wisman`) -- discovered
    from the catalog itself (`SHOW TABLES`), not from
    `bronze_meta.dataset_catalog`, so a table that failed catalog
    registration but still exists in Lakekeeper is still listed (the
    two failure modes are independent and a caller should not silently
    skip a table just because the OTHER P3 step didn't run for it).

    `SHOW TABLES FROM {CATALOG_DB}` returns every namespace the catalog
    knows about in one flat list -- `bronze.*` AND `gold.*` (ADR 0010's
    Gold export target) side by side, since `DataLakeCatalog` has no
    per-namespace listing call, only a whole-database one.

    `discover_bronze_tables` (below) narrows this list to `bronze.*`;
    this function keeps the whole list, for a caller that measures more
    than one namespace (`storage_snapshot.py`'s per-layer job)."""
    text = _ch_query(
        cfg, f"SHOW TABLES FROM {CATALOG_DB} SETTINGS allow_database_iceberg=1 FORMAT TabSeparated"
    )
    return [line.strip() for line in text.splitlines() if line.strip()]


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
    return [name for name in discover_catalog_tables(cfg) if name.startswith("bronze.")]


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
    cfg: Any,
    table_name: str,
    *,
    dry_run: bool,
    catalog_db: str = CATALOG_DB,
    older_than_seconds: int | None = None,
) -> dict[str, Any]:
    """Run (or dry-run) `remove_orphan_files` against one catalog-registered
    Iceberg table — the working verb on ClickHouse 26.8 (did not exist on
    26.3). `table_name` is the catalog's own two-part name, e.g.
    `bronze.g3a_orders` or `gold.mart_name`."""
    dry_run_arg = "dry_run=1" if dry_run else "dry_run=0"
    settings = _REMOVE_ORPHAN_FILES_SETTINGS
    if older_than_seconds is not None:
        settings = f"{settings}, iceberg_orphan_files_older_than_seconds={int(older_than_seconds)}"
    sql = (
        f"ALTER TABLE {catalog_db}.`{table_name}` "
        f"EXECUTE remove_orphan_files({dry_run_arg}) "
        f"SETTINGS {settings} FORMAT TabSeparated"
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
    ClickHouse version makes this succeed again, that is reported too (the
    caller logs whatever this returns either way — this function never
    swallows the outcome, it just describes it). What it returns is
    ClickHouse's own text (its error body, or the result of an unexpected
    success), so it lands in the Dagster run log and NOWHERE else: not in
    `skipped_verbs`, not in a recorded row, and so not in an API
    response."""
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


def mint_lakekeeper_read_token(cfg: MaintenanceConfig) -> str | None:
    """Mint a short-lived bearer token from `ops/oidc-mock`'s
    client-credentials endpoint as the `clickhouse-reader` principal — the
    SAME pattern `ops/g3a/g3a_test.py`'s `_mint_lakekeeper_read_token` uses
    to hit Lakekeeper's Iceberg REST catalog directly, reusing this job's
    existing `select` grant (see `lakekeeper-authz-init` in
    `docker-compose.yml`) rather than requesting a new one. Returns `None`
    on a pre-R1/authz-disabled stack (`ch_oauth_client_id` unset there
    too), where `measure_snapshot_growth` degrades to an honest
    "not measured" rather than a failure.

    Public because `storage_snapshot.py` reads the SAME catalog and needs
    the SAME token: a private name of another module is a name that may
    change under it (`AGENTS.md` rule 4)."""
    if not cfg.ch_oauth_client_id:
        return None
    resp = requests.post(
        cfg.ch_oauth_server_uri, data={"client_id": cfg.ch_oauth_client_id}, timeout=10
    )
    resp.raise_for_status()
    token = resp.json().get("access_token")
    if not token:
        raise RuntimeError(f"token endpoint returned no access_token: {resp.json()}")
    return token


def resolve_catalog_prefix(cfg: MaintenanceConfig, token: str) -> str:
    """`GET /v1/config` resolves the warehouse NAME (`cfg.lakekeeper_warehouse`)
    to the REST catalog's own path `prefix` — the same two-step resolution
    `ops/g3a/g3a_test.py`'s `step_verify_format_version_2` already performs
    and has proven works against this stack.

    Public for the same reason `mint_lakekeeper_read_token` is: another
    module (`storage_snapshot.py`) reads the SAME catalog."""
    resp = requests.get(
        f"{cfg.lakekeeper_catalog_uri}/v1/config",
        params={"warehouse": cfg.lakekeeper_warehouse},
        headers={"Authorization": f"Bearer {token}"},
        timeout=10,
    )
    resp.raise_for_status()
    body = resp.json()
    prefix = {**body.get("defaults", {}), **body.get("overrides", {})}.get("prefix")
    if not prefix:
        raise RuntimeError(f"catalog /v1/config returned no warehouse prefix: {body}")
    return prefix


def measure_snapshot_growth(cfg: MaintenanceConfig, table_name: str) -> dict[str, Any]:
    """Measure the snapshot growth the module doc describes:
    `snapshot_count` (from the table's own `metadata.snapshots`) and
    `metadata_log_count` (from `metadata.metadata-log`, the retained-old-
    metadata-file trail) — both read straight from the Iceberg REST
    catalog's table-metadata document (`GET /v1/{prefix}/namespaces/{ns}/
    tables/{table}`), never guessed or derived from ClickHouse. The
    snapshot expiry DOES reduce `snapshot_count`: it runs through the
    catalog (Trino's connector when Trino answers, `pyiceberg` when it
    does not) and removes the snapshot entries the policy expires.
    `remove_orphan_files` reduces NEITHER number — it deletes files, not
    snapshot metadata — and because this function runs AFTER the expiry
    (OPS-20 decision 4) the count it records is the count the policy
    LEAVES BEHIND, not the count the run started with.
    `table_name` is the catalog's own two-part name (e.g.
    `bronze.g3a_orders`, same shape `discover_bronze_tables` returns).

    Returns `{"measured": False, ...}` (zeros) rather than raising when
    `CH_OAUTH_CLIENT_ID` is unset (pre-R1/authz-disabled stack): the
    token this measurement mints for itself cannot be minted then, and
    that token is the only credential its read path has. The expiry is
    not this measurement — it runs through Trino when Trino answers and
    through the catalog (`pyiceberg`, the `dlt` principal) when it does
    not, so a stack with no `CH_OAUTH_CLIENT_ID` can still expire
    snapshots: this measurement is skipped because ITS read path has no
    token, not because nothing can write. A skip here stays consistent
    with the rest of this module's degrade-gracefully posture on that
    same condition."""
    token = mint_lakekeeper_read_token(cfg)
    if token is None:
        return {"measured": False, "snapshot_count": 0, "metadata_log_count": 0}

    prefix = resolve_catalog_prefix(cfg, token)
    namespace, _, table = table_name.partition(".")
    resp = requests.get(
        f"{cfg.lakekeeper_catalog_uri}/v1/{prefix}/namespaces/{namespace}/tables/{table}",
        headers={"Authorization": f"Bearer {token}"},
        timeout=10,
    )
    resp.raise_for_status()
    metadata = resp.json().get("metadata") or {}
    snapshots = metadata.get("snapshots") or []
    metadata_log = metadata.get("metadata-log") or []
    return {
        "measured": True,
        "snapshot_count": len(snapshots),
        "metadata_log_count": len(metadata_log),
    }


def _table_namespace_and_name(table_name: str) -> tuple[str, str]:
    """Splits the catalog's own two-part name (e.g. `bronze.g3a_orders`,
    the shape `discover_bronze_tables` returns) into
    `(namespace, table_name)` — the exact key shape
    `table_maintenance_policy` (and its store's `(namespace, table_name)`
    primary key) uses, and the two identifiers a Trino statement
    interpolates as `iceberg.<namespace>."<table_name>"`."""
    namespace, _, name = table_name.partition(".")
    return namespace, name


# Dagster's `DynamicOutput.mapping_key` must match `^[A-Za-z0-9_]+$`
# (`dagster._core.definitions.utils.check_valid_chars`). The exact ASCII
# rule from `parts/1b-dagster-one-op-per-unit-and-retries.md`: every
# character outside `[A-Za-z0-9_]` becomes `_`. Precompiled once at
# module load; the same rule `gold_export._sanitize_mapping_key` and
# `ingest_factory._sanitize_target` enforce so a table name can serve
# as a schedule name, an op name, or a mapping key consistently. Note:
# `str.isalnum()` would be wrong here -- it is True for non-ASCII
# letters (`"é".isalnum()`, `"²".isalnum()`), which would silently
# produce a non-ASCII mapping key that Dagster then rejects.
_SANITIZE_NON_ASCII = re.compile(r"[^A-Za-z0-9_]")


def _sanitize_mapping_key(name: str) -> str:
    """Dagster-legal `DynamicOutput.mapping_key` derived from a
    catalog table name -- the exact ASCII `[A-Za-z0-9_]+` rule
    `gold_export._sanitize_mapping_key` and
    `ingest_factory._sanitize_target` already enforce. Bronze tables
    happen to be `<namespace>.<name>` and both segments are already
    `[A-Za-z0-9_]+`, so this is in practice a no-op for today's catalog;
    kept consistent with the other fan-out modules for the
    collision-detection rule (and so a future table name that violates
    the rule does not silently break graph-build at runtime)."""
    return _SANITIZE_NON_ASCII.sub("_", name)


def _run_maintenance_for_table(
    cfg: MaintenanceConfig,
    table_name: str,
    policy: dict[str, Any] | None,
    policy_fetch_error: str | None,
    context: Any = None,
) -> dict[str, Any]:
    """The per-table body of the maintenance chain, extracted from
    `run_bronze_maintenance` so PART C's `maintain_bronze_table` mapped
    op and the existing `test_maintenance.py` Trino-integration tests
    can both drive it. The order is: probe, the policy-driven
    `expire_snapshots`, dry-run + applied orphan removal, growth
    measurement, `optimize`, then `record_maintenance_run` +
    `record_maintenance_verb_run`. The expiry moved ahead of the orphan
    removal (OPS-20 decision 4) so the files the expired snapshots
    referenced are orphans by the time `remove_orphan_files` runs.

    `context` is the op's own context, so the ClickHouse probe's outcome
    can be logged (see below). It is optional because this helper is
    driven directly by the tests, which have no op context to hand."""
    skipped_verbs: list[str] = []

    if maintenance_paused():
        # The pause (REL-2 S4 decision D16): nothing that deletes a
        # file runs, the reason is recorded, and the run is still
        # recorded — so the operator sees that the maintenance did
        # nothing and why. The growth measurement is skipped with it:
        # it is a measurement of the same tables, and a paused
        # maintenance does not read them either.
        reason = (
            "maintenance paused: MAINTENANCE_PAUSED=true, so no snapshot was expired and no "
            "orphan file was removed"
        )
        skipped_verbs.append(reason)
        record_maintenance_run(
            table_name=table_name,
            dry_run_metrics={},
            applied_metrics={},
            skipped_verbs=skipped_verbs,
            snapshot_growth=None,
            target=cfg.ch,
        )
        return {
            "table_name": table_name,
            "dry_run": {},
            "applied": {},
            "skipped_verbs": skipped_verbs,
            "snapshot_growth": {},
        }

    # The probe OBSERVES ClickHouse's own `expire_snapshots` refusal; it
    # expires nothing (this job's expiry runs through Trino or the REST
    # catalog — see the module doc). Its outcome is ClickHouse's own error
    # body, so it is logged in the Dagster run log and NEVER recorded:
    # `skipped_verbs` and the `maintenance_run` / `maintenance_verb_run`
    # rows are served by the API, and upstream error text never reaches a
    # response. INFO, not WARNING: the refusal is ClickHouse's deliberate
    # behaviour on 26.8, not a fault of this job, and the probe's own text
    # says which case it is.
    expire_probe_outcome = probe_expire_snapshots_skip(cfg, table_name)
    log = context.log if context else None
    if log is not None:
        log.info(
            f"ClickHouse's own expire_snapshots probe for {table_name}: "
            f"{expire_probe_outcome} — this job's snapshot expiry runs "
            "through Trino (or the REST catalog when Trino is unreachable), "
            "not through ClickHouse"
        )
    namespace, name = _table_namespace_and_name(table_name)
    now = datetime.now(timezone.utc)

    verb_runs: list[dict[str, str]] = []

    if policy_fetch_error is not None:
        skipped_verbs.append(f"policy fetch failed: {policy_fetch_error}")

    last_run_at: datetime | None = None
    if policy is not None:
        if policy.get("orphanAgeHours") is not None:
            detail = (
                "orphan_age_hours not applied: remove_orphan_files runs with "
                f"ClickHouse's default age floor ({ORPHAN_FILE_AGE_FLOOR_SECONDS}s "
                "= 3 days), not with this policy's value"
            )
            skipped_verbs.append(detail)
            verb_runs.append(
                {
                    "verb": "orphan_age_hours",
                    "engine": "clickhouse",
                    "outcome": "skipped",
                    "detail": detail,
                }
            )
        last_run_at = _parse_run_at(latest_maintenance_run_at(cfg.ch, table_name))

    # The expiry runs BEFORE the orphan-file removal (OPS-20 decision 4):
    # the files the expired snapshots referenced are orphans by the time
    # `remove_orphan_files` runs — but the reclaim lands about three days
    # later, not in this run: `remove_orphan_files` runs with ClickHouse's
    # default `iceberg_orphan_files_older_than_seconds`
    # (`ORPHAN_FILE_AGE_FLOOR_SECONDS`, 259200 s = 3 days), which protects
    # files younger than that, and the measured proof run deleted 0 files
    # for that reason. The orphan verb itself is unchanged.
    if policy_fetch_error is not None:
        expire_skip = f"expire_snapshots (policy fetch failed: {policy_fetch_error})"
        expire_verb_run = None
    else:
        expire_skip, expire_verb_run = _expire_snapshots_result(
            cfg, namespace, name, policy, now, last_run_at
        )
    if expire_skip is not None and expire_skip not in skipped_verbs:
        skipped_verbs.append(expire_skip)
    if expire_verb_run is not None:
        verb_runs.append(expire_verb_run)

    dry = run_remove_orphan_files(cfg, table_name, dry_run=True)
    real = run_remove_orphan_files(cfg, table_name, dry_run=False)
    growth = measure_snapshot_growth(cfg, table_name)

    optimize_skip, optimize_verb_run = _optimize_result(
        cfg, namespace, name, policy, now, last_run_at
    )
    if optimize_skip is not None and optimize_skip not in skipped_verbs:
        skipped_verbs.append(optimize_skip)
    if optimize_verb_run is not None:
        verb_runs.append(optimize_verb_run)

    record_maintenance_run(
        table_name=table_name,
        dry_run_metrics=dry,
        applied_metrics=real,
        skipped_verbs=skipped_verbs,
        snapshot_growth=growth,
        target=cfg.ch,
    )
    if verb_runs:
        record_maintenance_verb_run(
            table_name=table_name,
            run_at=datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            verb_runs=verb_runs,
            target=cfg.ch,
        )
    return {
        "table_name": table_name,
        "dry_run": dry,
        "applied": real,
        "skipped_verbs": skipped_verbs,
        "snapshot_growth": growth,
    }


def _fetch_policy_index(
    cfg: MaintenanceConfig,
) -> tuple[dict[tuple[str, str], dict[str, Any]], str | None]:
    """Fetches every configured maintenance policy from
    `GET /api/lakehouse/maintenance-policies` ONCE per run (not once per
    table), authenticating as the `lakehouse-maintenance-policy-reader`
    service identity with `LAKEHOUSE_MAINTENANCE_TOKEN` as a bearer
    token — the same `requests.get(..., headers={"Authorization": f"Bearer
    {token}"}, timeout=10)` shape `measure_snapshot_growth`'s Lakekeeper
    call already uses. Returns an index keyed by
    `(namespace, tableName)`.

    A 401/403 means the token is missing or wrong — a configuration or
    authentication problem, so this raises `dagster.Failure` rather than
    degrading (AGENTS.md: config/auth problems are a `Failure`, not a
    silent skip). Anything else that keeps this job from getting a real
    policy list — unreachable, a 5xx, or a malformed/unexpected body — is
    NOT the same as "no policies configured": it returns an empty index
    together with a non-`None` error string, so the caller records
    `"policy fetch failed: <reason>"` in `skipped_verbs` instead of
    silently treating every table as unconfigured.
    """
    if not cfg.maintenance_token:
        return {}, "LAKEHOUSE_MAINTENANCE_TOKEN is not set"

    url = f"{cfg.api_url}/api/lakehouse/maintenance-policies"
    try:
        resp = requests.get(
            url,
            headers={"Authorization": f"Bearer {cfg.maintenance_token}"},
            timeout=10,
        )
    except requests.RequestException as exc:
        return {}, f"{type(exc).__name__}"

    if resp.status_code in (401, 403):
        # PART D: a 401/403 from the maintenance-policy API is a
        # configuration / authentication problem (the token is wrong,
        # or the Lakekeeper role no longer grants the policy endpoint)
        # — retrying won't help. `allow_retries=False` is the
        # canonical non-retryable pattern; otherwise the default retry
        # policy burns its 60s budget on a problem the operator has to
        # fix out-of-band.
        raise Failure(
            f"maintenance policy fetch failed with HTTP {resp.status_code} "
            f"from {url} — check LAKEHOUSE_MAINTENANCE_TOKEN",
            allow_retries=False,
        )

    try:
        resp.raise_for_status()
        policies = resp.json()["policies"]
        index = {(p["namespace"], p["tableName"]): p for p in policies}
    except (requests.HTTPError, ValueError, KeyError, TypeError) as exc:
        return {}, f"{type(exc).__name__}"

    return index, None


class TrinoQueryError(RuntimeError):
    """Trino reported a query-level error (a `/v1/statement` response
    page's `error.message`) — the real message, so a caller can record it
    (e.g. the 7-day `expire-snapshots.min-retention` floor refusing a
    shorter threshold), never an invented one."""


class TrinoUnavailableError(RuntimeError):
    """The `/v1/statement` request failed at the transport level, or the
    60-second result-paging cap (below) was exceeded — a degraded state
    (Trino runs only under this stack's `trino` compose profile) rather
    than a query-level failure, and never raised as a job failure."""


_TRINO_PAGING_CAP_SECONDS = 60.0


def _trino_execute(cfg: MaintenanceConfig, sql: str) -> list[list[Any]]:
    """Runs `sql` against Trino's `/v1/statement` REST protocol, sent as
    `X-Trino-User: trino-maintenance`, following the response's `nextUri`
    with plain synchronous `requests` calls under a 60-second wall-clock
    cap (`time.monotonic()`-based, matching this module's existing
    synchronous style deliberately — no `asyncio` introduced). This is a
    small, deliberate duplication of `rust/crates/lakehouse-trino`'s own
    polling loop (see the module doc): this job runs in a separate Python
    process outside the Rust binary that crate is built into.

    The `columns`/`data`/`nextUri`/`error` field names and the
    `error.message` shape below match Trino's documented `/v1/statement`
    protocol and the exact fixture shape `lakehouse-trino`'s own
    `wiremock`-backed tests already assert against
    (`rust/crates/lakehouse-trino/src/lib.rs`); this job's own G2 gate
    exercises the real syntax live.

    # Errors
    Raises [`TrinoQueryError`] with Trino's own message on a query-level
    error, [`TrinoUnavailableError`] on a transport failure or if the
    60-second cap is exceeded before paging finishes.
    """
    deadline = time.monotonic() + _TRINO_PAGING_CAP_SECONDS

    def _post(uri: str, *, body: bytes | None) -> dict[str, Any]:
        try:
            if body is None:
                resp = requests.get(uri, timeout=10)
            else:
                resp = requests.post(
                    uri, data=body, headers={"X-Trino-User": "trino-maintenance"}, timeout=10
                )
            resp.raise_for_status()
            return resp.json()
        except requests.RequestException as exc:
            raise TrinoUnavailableError(f"trino unreachable: {exc}") from exc

    page = _post(f"{cfg.trino_url}/v1/statement", body=sql.encode("utf-8"))
    rows: list[list[Any]] = []
    while True:
        error = page.get("error")
        if error:
            raise TrinoQueryError(error.get("message", "trino query failed"))
        rows.extend(page.get("data") or [])
        next_uri = page.get("nextUri")
        if not next_uri:
            return rows
        if time.monotonic() >= deadline:
            raise TrinoUnavailableError(
                "trino query exceeded the 60s result-paging cap"
            )
        page = _post(next_uri, body=None)


def _parse_trino_timestamp(value: str) -> datetime:
    """Parses a Trino `timestamp(p) with time zone` column value from the
    `/v1/statement` protocol — the documented rendering is
    `YYYY-MM-DD HH:MM:SS[.fff] <zone name>` (e.g.
    `2026-01-01 00:00:00.000 UTC`) — into an aware UTC `datetime`. Falls
    back to `datetime.fromisoformat` for a plain ISO-8601 string with no
    trailing zone name, so a differently-configured Trino session still
    parses. Not measured against a live Trino response: the procedure
    syntax is resolved from `ops/trino/optimize_bronze.sh`'s existing proof
    and Trino's documented form, not a fresh live capture; this job's own
    G2 gate exercises the real response shape."""
    text = value.strip()
    if text.endswith(" UTC"):
        text = text[: -len(" UTC")]
        fmt = "%Y-%m-%d %H:%M:%S.%f" if "." in text else "%Y-%m-%d %H:%M:%S"
        return datetime.strptime(text, fmt).replace(tzinfo=timezone.utc)
    parsed = datetime.fromisoformat(text.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed


def _parse_run_at(value: str | None) -> datetime | None:
    """Parses `bronze_catalog._utc_now_iso`'s fixed
    `%Y-%m-%dT%H:%M:%SZ` format (the `bronze_meta.maintenance_run.run_at`
    column's own shape) back into an aware UTC `datetime`, or `None` for
    a missing/unparseable value — "never run before" is exactly the case
    the schedule cadence gate below treats as "always allowed"."""
    if not value:
        return None
    try:
        return datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)
    except ValueError:
        return None


def _cadence_allows(
    schedule: str | None, now: datetime, last_run_at: datetime | None
) -> tuple[bool, str | None]:
    """Per-table schedule cadence for the expiry and compaction verbs
    (module doc). `schedule=None` or no prior recorded run always allows a
    run. Returns `(True, None)` when the run is allowed, `(False, <reason>)`
    otherwise. Pure — takes `now` and `last_run_at` explicitly so this is
    testable on fixed times, with no wall-clock or network dependency."""
    if schedule is None or last_run_at is None:
        return True, None
    elapsed = now - last_run_at
    if schedule == "weekly" and elapsed < timedelta(days=7):
        return False, f"schedule=weekly: last verb run {elapsed} ago, needs >= 7 days"
    if schedule == "daily" and elapsed < timedelta(hours=20):
        return False, f"schedule=daily: last verb run {elapsed} ago, needs >= 20 hours"
    return True, None


def _compute_retention_threshold(
    committed_at: list[datetime], keep: int, now: datetime
) -> tuple[str | None, str | None]:
    """Computes the `expire_snapshots(retention_threshold => ...)`
    argument that keeps exactly the `keep` newest snapshots, from
    `committed_at` (every snapshot's commit time, NEWEST FIRST — the
    order `SELECT committed_at FROM iceberg.<ns>."<t>$snapshots" ORDER BY
    committed_at DESC` returns). Trino 483's `expire_snapshots` takes only
    an age, never a keep-count, so a table with `keep` or fewer snapshots
    has nothing to expire (returns `(None, <skip reason>)` — never a
    `'0d'` threshold, which would expire every snapshot but the current
    one regardless of `keep`, a real data-loss bug the naive reading of
    this procedure invites). Otherwise the threshold is the AGE of the
    `keep`-th newest snapshot, in whole seconds rounded up
    (`'<secs>s'`) — snapshots newer than that survive, and there are
    exactly `keep` of them. Pure — `now` is explicit so this is testable
    on fixed times."""
    if len(committed_at) <= keep:
        return None, (
            f"snapshotsToKeep={keep}: table has {len(committed_at)} snapshot(s), "
            "nothing to expire"
        )
    nth_newest = committed_at[keep - 1]
    age_seconds = math.ceil((now - nth_newest).total_seconds())
    return f"{age_seconds}s", None


# The OPTIMIZE verb's skip when Trino is not there. Only the optimize
# verb is skipped: the snapshot expiry falls back to the catalog
# (`pyiceberg`) when Trino is unreachable, so a text that also claimed
# the expiry was skipped would put a false statement in the recorded
# run.
_TRINO_UNREACHABLE_SKIP = "optimize skipped: trino unreachable"

# OPS-1 (Decision D1): Tables without a policy, or a policy with no
# `snapshotsToKeep`, keep 7 days of snapshots. Trino 483's Iceberg
# connector enforces `iceberg.expire-snapshots.min-retention` (7 days by
# default), so 7d is both the product floor and Trino's system floor.
DEFAULT_SNAPSHOT_RETENTION = "7d"

# ClickHouse's `iceberg_orphan_files_older_than_seconds` default, measured
# in this repo at
# `docs/superpowers/plans/2026-10-14-data-2-gold-keep-copies.md:245-246`
# (3 days). `run_remove_orphan_files` calls the verb WITHOUT overriding
# that setting, so files younger than this age are protected and the
# space an expiry frees is reclaimed about three days later, not in the
# same run. The expiry's recorded detail says so, because "where did my
# space go" is otherwise unanswerable from the recorded run.
ORPHAN_FILE_AGE_FLOOR_SECONDS = 259_200

# The units a retention threshold string may carry: `7d` is this module's
# default retention, `<n>s` is what `_compute_retention_threshold`
# emits.
_RETENTION_UNITS = {"s": 1, "m": 60, "h": 3600, "d": 86_400}


def _retention_seconds(threshold: str) -> int:
    """The number of seconds a retention threshold string denotes
    (`'86400s'`, `'7d'`).

    No duration parser existed in this repo before this change (the two
    parsers that do exist, `gold_export.parse_gold_keep_copies` and
    `parse_gold_orphan_min_age_seconds`, read plain integers), so this
    is a small local one. It is needed because
    `_compute_retention_threshold` returns an AGE and the pyiceberg path
    needs the CUTOFF INSTANT that age describes."""
    text = threshold.strip()
    unit = text[-1:]
    if unit not in _RETENTION_UNITS:
        raise ValueError(f"unsupported retention threshold: {threshold!r}")
    return int(text[:-1]) * _RETENTION_UNITS[unit]


def _snapshot_datetime(timestamp_ms: int) -> datetime:
    """A snapshot's commit time as an aware UTC `datetime`, exact to the
    millisecond (Iceberg stores milliseconds, not microseconds)."""
    return datetime.fromtimestamp(timestamp_ms // 1000, tz=timezone.utc) + timedelta(
        milliseconds=timestamp_ms % 1000
    )


def _snapshot_ids_to_expire(
    table: Any, keep: int | None, now: datetime
) -> tuple[list[int], str | None, str | None]:
    """The snapshot ids the pyiceberg expiry must remove, the threshold
    that decided them, and a skip reason (`None` when there is nothing to
    skip).

    Pure over the loaded table — no I/O — so it is testable with a fake
    table.

    * The snapshots are ordered newest first, exactly the order
      `_compute_retention_threshold` expects.
    * The threshold is the AGE of the Nth-newest snapshot when the
      policy has a `snapshotsToKeep`, and this module's default
      retention (`7d`) when it does not — the same number the Trino path
      hands to `expire_snapshots(retention_threshold => ...)`.
    * The ids to KEEP are the newest N, every branch and tag head, and
      the table's CURRENT snapshot.
      - A branch or tag head is PROTECTED: `pyiceberg`'s
        `ExpireSnapshots.by_id` RAISES `ValueError` on a protected id,
        so they must be filtered out here, before the call, not left to
        the library.
      - The current snapshot is protected for another reason:
        `pyiceberg` 0.12.0's `RemoveSnapshotRefUpdate` — the update it
        applies for a ref whose head is the removed snapshot — sets
        `current_snapshot_id = None` when the removed ref is the main
        branch (`pyiceberg/table/update/__init__.py:558-568`), so expiring
        the current snapshot would leave the table with NO current snapshot
        at all. A format-version-1 table has no refs at all, so the
        branch-head protection does not cover it.
    * A snapshot is expired when it is STRICTLY older than the threshold
      (`committed < cutoff`): the cutoff is EXCLUSIVE. Iceberg's own
      expiry is strict on that boundary — `pyiceberg` 0.12.0's
      `ExpireSnapshots.older_than` expires a snapshot only when
      `snapshot.timestamp_ms < expire_from`
      (`pyiceberg/table/update/snapshot.py:1335`), and Iceberg's Java
      `RemoveSnapshots` — what Trino's `expire_snapshots` calls through
      `expireOlderThan` — keeps a snapshot while
      `snapshot.timestampMillis() >= expireOlderThan`
      (`RemoveSnapshots.unreferencedSnapshotsToRetain` and
      `RemoveSnapshots.computeBranchSnapshotsToRetain`). The plan's
      decision 1 is "fallback, not replacement": the fallback has to
      behave like the engine it replaces, so the two engines must agree
      on the boundary.
    * Strict is also what makes the newest N safe BY the threshold
      alone: `_compute_retention_threshold` returns the Nth-newest
      snapshot's age rounded UP, so the cutoff instant is at or BEFORE
      the Nth-newest's commit instant, and the newest N are therefore
      never strictly older than it. The keep filter above is then a
      second, independent guard rather than the only one — a bug in it
      could at worst expire FEWER snapshots, never one the policy
      keeps. With an inclusive comparison the keep filter was the only
      guard for the newest N, which is the plan's stop condition ("the
      proof shows a snapshot that the policy keeps being removed")
      resting on hand-computed code.
    * The honest consequence of the strict comparison: the cutoff is
      `now` minus the Nth-newest's AGE rounded UP to whole seconds (the
      `'<secs>s'` threshold Trino takes), so the cutoff instant lands
      at or BEFORE the Nth-newest's commit instant, by up to one
      second. Every snapshot committed inside that one-second window
      (up to one second before the Nth-newest) is not strictly older
      than the cutoff, so it survives too — the 2026-11-21 proof run
      kept 3 snapshots for a policy that keeps 2, the 3rd-newest
      committed 789 ms before the 2nd-newest. So "the newest N survive"
      is "AT LEAST N survive". Trino's own `expire_snapshots` does the
      same with the same threshold (Iceberg expires a snapshot only
      when `timestamp_ms < now - retention`), and that is why the
      comparison is strict: with a strict cutoff the newest N can never
      be removed.
    * A table with N or fewer snapshots is skipped — never a zero
      threshold, which would expire every snapshot but the current one
      regardless of `keep`."""
    from pyiceberg.table.refs import SnapshotRefType

    snapshots = sorted(table.snapshots(), key=lambda s: s.timestamp_ms, reverse=True)
    committed_at = [_snapshot_datetime(s.timestamp_ms) for s in snapshots]

    if keep is None:
        threshold = DEFAULT_SNAPSHOT_RETENTION
    else:
        threshold, skip_reason = _compute_retention_threshold(committed_at, keep, now)
        if skip_reason is not None:
            return [], threshold, skip_reason

    keep_ids = {
        ref.snapshot_id
        for ref in table.refs().values()
        if ref.snapshot_ref_type in (SnapshotRefType.BRANCH, SnapshotRefType.TAG)
    }
    if keep is not None:
        keep_ids.update(snapshot.snapshot_id for snapshot in snapshots[:keep])
    current = table.current_snapshot()
    if current is not None:
        keep_ids.add(current.snapshot_id)

    cutoff = now - timedelta(seconds=_retention_seconds(threshold))
    ids = [
        snapshot.snapshot_id
        for snapshot, committed in zip(snapshots, committed_at)
        if snapshot.snapshot_id not in keep_ids and committed < cutoff
    ]
    return ids, threshold, None


def _pyiceberg_catalog(cfg: MaintenanceConfig) -> Any:
    """The `pyiceberg` `RestCatalog` this job expires snapshots through.

    The identity is the `dlt` principal — the one the ingest job already
    writes through. ADR 0011 grants it `create modify select` on the
    warehouse, and `modify` covers `expire_snapshots`, so this change
    adds no principal, no grant and no compose entry: the token file
    the ingest job reads is already mounted read-only into this
    container.

    `s3.path-style-access=true` and `s3.force-virtual-addressing=false`
    are what RustFS/SeaweedFS need — see `adapters/sink.py`'s
    `_install_catalog_env` for the measured reason: without them
    pyiceberg tries virtual-hosted addressing and fails DNS.

    The `RestCatalog` constructor fetches the catalog's own config, so
    THIS is the seam the tests patch: no test reaches the network."""
    from pyiceberg.catalog.rest import RestCatalog

    return RestCatalog(
        name="lakekeeper",
        warehouse=cfg.lakekeeper_warehouse,
        uri=cfg.lakekeeper_catalog_uri,
        token=cfg.lakekeeper_token,
        **{
            "s3.endpoint": cfg.rustfs_endpoint,
            "s3.access-key-id": cfg.rustfs_access_key,
            "s3.secret-access-key": cfg.rustfs_secret_key,
            "s3.region": "us-east-1",
            "s3.path-style-access": "true",
            "s3.force-virtual-addressing": "false",
        },
    )


def _expire_snapshots_pyiceberg(
    cfg: MaintenanceConfig,
    namespace: str,
    name: str,
    policy: dict[str, Any] | None,
    now: datetime,
) -> tuple[str | None, dict[str, str] | None]:
    """Expires this table's snapshots through `pyiceberg` — the fallback
    for a stack where Trino is not running. Returns the same
    `(skipped_verbs_entry, verb_run_record)` shape as the Trino path.

    `pyiceberg`'s expiry removes snapshot METADATA only: the files the
    expired snapshots referenced become orphans, and the
    `remove_orphan_files` verb — which runs after this one — is what
    deletes them, under ClickHouse's 3-day age floor."""
    keep = None if policy is None else policy.get("snapshotsToKeep")

    if not cfg.lakekeeper_token:
        reason = "no Lakekeeper catalog identity on this stack (no dlt token)"
        return (
            f"expire_snapshots ({reason})",
            {
                "verb": "expire_snapshots",
                "engine": "pyiceberg",
                "outcome": "skipped",
                "detail": reason,
            },
        )

    try:
        catalog = _pyiceberg_catalog(cfg)
        table = catalog.load_table((namespace, name))
        # The id computation is inside this try on purpose: one table's
        # unreadable metadata is one degraded table, never a run that
        # takes the whole maintenance down with it.
        ids, threshold, skip_reason = _snapshot_ids_to_expire(table, keep, now)
    except Exception as exc:  # noqa: BLE001 -- any catalog failure is one degraded table, not a broken run
        # The TYPE only: this detail is recorded in
        # `lake.bronze_meta.maintenance_verb_run` and served by the API,
        # so upstream (Lakekeeper/pyiceberg) text must never reach it
        # (the same rule `_fetch_policy_index` follows).
        detail = f"{type(exc).__name__}"
        return (
            f"expire_snapshots (the pyiceberg fallback could not read the table: "
            f"{type(exc).__name__})",
            {
                "verb": "expire_snapshots",
                "engine": "pyiceberg",
                "outcome": "failed",
                "detail": detail,
            },
        )
    if skip_reason is not None:
        # The reason states WHY the verb was skipped — the `engine` column
        # already says `pyiceberg`, which is itself the signal that Trino
        # was unreachable.
        reason = skip_reason
        return (
            f"expire_snapshots ({reason})",
            {
                "verb": "expire_snapshots",
                "engine": "pyiceberg",
                "outcome": "skipped",
                "detail": reason,
            },
        )
    if not ids:
        reason = f"no snapshot older than {threshold}"
        return (
            f"expire_snapshots ({reason})",
            {
                "verb": "expire_snapshots",
                "engine": "pyiceberg",
                "outcome": "skipped",
                "detail": reason,
            },
        )

    try:
        table.maintenance.expire_snapshots().by_ids(ids).commit()
    except Exception as exc:  # noqa: BLE001 -- the expiry failed; the table's other maintenance still runs
        # The TYPE only, for the same reason as the load failure above:
        # the recorded detail is served by the API.
        detail = f"{type(exc).__name__}"
        return (
            f"expire_snapshots (the pyiceberg fallback failed: {type(exc).__name__})",
            {
                "verb": "expire_snapshots",
                "engine": "pyiceberg",
                "outcome": "failed",
                "detail": detail,
            },
        )

    detail = (
        f"expired {len(ids)} snapshot(s) older than {threshold}; their files are "
        f"reclaimed by remove_orphan_files, which protects files younger than "
        f"{ORPHAN_FILE_AGE_FLOOR_SECONDS}s (3 days), so the space comes back "
        "about 3 days later"
    )
    return (
        None,
        {
            "verb": "expire_snapshots",
            "engine": "pyiceberg",
            "outcome": "applied",
            "detail": detail,
        },
    )


def _expire_snapshots_trino(
    cfg: MaintenanceConfig,
    namespace: str,
    name: str,
    policy: dict[str, Any] | None,
    now: datetime,
) -> tuple[str | None, dict[str, str] | None]:
    """This table's `expire_snapshots` verb through Trino — the path that
    runs whenever Trino answers. Returns the same
    `(skipped_verbs_entry, verb_run_record)` shape as
    `_expire_snapshots_result`.

    A Trino that is NOT there raises `TrinoUnavailableError` out of here,
    and the caller falls back to `pyiceberg`: an unreachable Trino is a
    degraded state (Trino runs only under this stack's `trino` compose
    profile), never a refusal. A Trino that ANSWERS and refuses (e.g.
    its `iceberg.expire-snapshots.min-retention` floor) raises
    `TrinoQueryError` and keeps today's behaviour exactly: `refused` or
    `failed`, never a fallback."""
    keep = None if policy is None else policy.get("snapshotsToKeep")
    if keep is None:
        sql = (
            f'ALTER TABLE iceberg.{namespace}."{name}" '
            f"EXECUTE expire_snapshots(retention_threshold => '{DEFAULT_SNAPSHOT_RETENTION}')"
        )
        try:
            _trino_execute(cfg, sql)
        except TrinoQueryError as exc:
            detail = str(exc)
            return (
                f"expire_snapshots refused: {detail}",
                {
                    "verb": "expire_snapshots",
                    "engine": "trino",
                    "outcome": "refused",
                    "detail": detail,
                },
            )
        return (
            None,
            {
                "verb": "expire_snapshots",
                "engine": "trino",
                "outcome": "applied",
                "detail": f"default retention {DEFAULT_SNAPSHOT_RETENTION} (no snapshotsToKeep policy)",
            },
        )

    try:
        rows = _trino_execute(
            cfg,
            f'SELECT committed_at FROM iceberg.{namespace}."{name}$snapshots" '
            "ORDER BY committed_at DESC",
        )
    except TrinoQueryError as exc:
        detail = str(exc)
        return (
            f"expire_snapshots (failed reading snapshot times: {detail})",
            {"verb": "expire_snapshots", "engine": "trino", "outcome": "failed", "detail": detail},
        )

    committed_at = [_parse_trino_timestamp(row[0]) for row in rows]
    threshold, skip_reason = _compute_retention_threshold(committed_at, keep, now)
    if skip_reason is not None:
        return (
            f"expire_snapshots ({skip_reason})",
            {"verb": "expire_snapshots", "engine": "trino", "outcome": "skipped", "detail": skip_reason},
        )

    sql = (
        f'ALTER TABLE iceberg.{namespace}."{name}" '
        f"EXECUTE expire_snapshots(retention_threshold => '{threshold}')"
    )
    try:
        _trino_execute(cfg, sql)
    except TrinoQueryError as exc:
        # Most likely Trino's `iceberg.expire-snapshots.min-retention`
        # floor (7 days by default, unset in this stack) refusing a
        # shorter threshold — recorded honestly and never retried larger.
        detail = str(exc)
        return (
            f"expire_snapshots refused: {detail}",
            {"verb": "expire_snapshots", "engine": "trino", "outcome": "refused", "detail": detail},
        )

    return (
        None,
        {
            "verb": "expire_snapshots",
            "engine": "trino",
            "outcome": "applied",
            "detail": f"retention_threshold={threshold}",
        },
    )


def _expire_snapshots_result(
    cfg: MaintenanceConfig,
    namespace: str,
    name: str,
    policy: dict[str, Any] | None,
    now: datetime,
    last_run_at: datetime | None,
) -> tuple[str | None, dict[str, str] | None]:
    """Decides and executes this table's `expire_snapshots` verb. Returns
    `(skipped_verbs_entry, verb_run_record)`: `skipped_verbs_entry` is
    `None` exactly when the verb genuinely ran (it must not also be
    recorded as skipped); `verb_run_record` is `None` only when no verb
    ran or was skipped with a recorded run.

    Trino first, exactly as before: when Trino answers, its path is the
    one that runs. Only an UNREACHABLE Trino (this stack's `trino`
    compose profile is off) switches to the `pyiceberg` fallback, which
    expires the same snapshots through the same catalog the ingest job
    writes through — "trino unreachable" is no longer the outcome of
    this verb.

    The schedule cadence is checked BEFORE any engine is tried: a
    cadence refusal is a policy decision, not an engine problem, so it
    never triggers the fallback.

    When `snapshotsToKeep` is specified in `policy`, the retention
    threshold preserves that many snapshots. When no policy is set or
    `snapshotsToKeep` is None, applies `DEFAULT_SNAPSHOT_RETENTION`
    ('7d') unconditionally (OPS-1 Decision D1)."""
    keep = None if policy is None else policy.get("snapshotsToKeep")

    if keep is not None:
        cadence_ok, cadence_reason = _cadence_allows(policy.get("schedule"), now, last_run_at)
        if not cadence_ok:
            # The cadence gate refuses BEFORE an engine is chosen, so no
            # engine ran: `none`, not a guess at the one that would have
            # (the verb may run through Trino or through the catalog).
            return (
                f"expire_snapshots ({cadence_reason})",
                {"verb": "expire_snapshots", "engine": "none", "outcome": "skipped", "detail": cadence_reason or ""},
            )

    try:
        return _expire_snapshots_trino(cfg, namespace, name, policy, now)
    except TrinoUnavailableError:
        return _expire_snapshots_pyiceberg(cfg, namespace, name, policy, now)


def _optimize_result(
    cfg: MaintenanceConfig,
    namespace: str,
    name: str,
    policy: dict[str, Any] | None,
    now: datetime,
    last_run_at: datetime | None,
) -> tuple[str | None, dict[str, str] | None]:
    """Decides and (if requested) executes this table's `optimize` verb
    via Trino — the same shape [`_expire_snapshots_result`] documents.
    With no policy, or `compactSmallFiles=False`, this keeps today's
    exact fixed design-skip message: `OPTIMIZE` genuinely never runs on
    this ClickHouse (it does not bin-pack — see the module doc), and
    Trino owns compaction instead only when a policy asks for it."""
    if policy is None or not policy.get("compactSmallFiles"):
        return (
            "OPTIMIZE (returns OK on 26.8 but does not bin-pack — not a "
            "compaction remedy; Trino owns that, see ADR 0009)",
            None,
        )

    cadence_ok, cadence_reason = _cadence_allows(policy.get("schedule"), now, last_run_at)
    if not cadence_ok:
        return (
            f"optimize ({cadence_reason})",
            {"verb": "optimize", "engine": "trino", "outcome": "skipped", "detail": cadence_reason or ""},
        )

    sql = f'ALTER TABLE iceberg.{namespace}."{name}" EXECUTE optimize'
    try:
        _trino_execute(cfg, sql)
    except TrinoUnavailableError:
        return (
            _TRINO_UNREACHABLE_SKIP,
            {"verb": "optimize", "engine": "trino", "outcome": "skipped", "detail": "trino unreachable"},
        )
    except TrinoQueryError as exc:
        detail = str(exc)
        return (
            f"optimize failed: {detail}",
            {"verb": "optimize", "engine": "trino", "outcome": "failed", "detail": detail},
        )

    return None, {"verb": "optimize", "engine": "trino", "outcome": "applied", "detail": ""}


@op(
    out=DynamicOut(),
    retry_policy=DEFAULT_RETRY_POLICY,
    # sql=None: this op runs one REMOVE/EXPIRE statement per discovered
    # table, not one fixed template.
    tags=source_metadata(
        "dispar_orchestrate/maintenance.py::list_bronze_tables",
        sql=None,
        reads=["Iceberg bronze.* (every table)", "GET /api/lakehouse/maintenance-policies"],
        # `writes` is the side-effect of this op: a `DynamicOutput` per
        # Bronze table, which becomes a mapped
        # `maintain_bronze_table[<sanitized_table>]` step. Declared
        # here so the every-op walk test that requires non-empty
        # `writes` does not flag this fan-out as having no writes.
        writes=["DynamicOutput per discovered Bronze table"],
    ),
)
def list_bronze_tables(context) -> Any:
    """PART C of `parts/1b-dagster-one-op-per-unit-and-retries.md`:
    fan-out op. Runs the per-run SETUP work that must not repeat per
    table (catalog database creation, policy list fetch) and yields
    one `DynamicOutput` per Bronze table, carrying that table's
    policy (already looked up) and the run-wide `policy_fetch_error`
    flag, so the mapped `maintain_bronze_table` op needs no
    per-call re-fetch.

    Two tables that sanitize to the same mapping_key raise
    `Failure(allow_retries=False)` -- they would otherwise collapse
    into one mapped step doing two tables' work, which Dagster's
    graph would refuse at build time and which this fan-out catches
    first with both names in the message.

    The 401/403 policy-fetch `Failure` raise from `_fetch_policy_index`
    is the same as before -- it raises BEFORE any table loop runs,
    so it remains a whole-run failure (not a per-table skip)."""
    cfg = MaintenanceConfig.from_env()
    _ensure_catalog_database(cfg)
    tables = discover_bronze_tables(cfg)
    context.log.info(f"discovered {len(tables)} Bronze table(s): {tables}")

    policy_index, policy_fetch_error = _fetch_policy_index(cfg)
    if policy_fetch_error is not None:
        context.log.warning(
            f"maintenance policy fetch failed ({policy_fetch_error}) — every "
            "table is treated as unconfigured this run, unconditional "
            "orphan-file removal still runs"
        )

    seen: dict[str, str] = {}
    for table_name in tables:
        namespace, name = _table_namespace_and_name(table_name)
        policy = None if policy_fetch_error is not None else policy_index.get((namespace, name))
        key = _sanitize_mapping_key(table_name)
        if key in seen:
            raise Failure(
                f"discovered two Bronze tables whose names sanitize to the same "
                f"Dagster step key {key!r}: {seen[key]!r} and {table_name!r} -- "
                "rename one so each table maps to a unique mapped step",
                allow_retries=False,
            )
        seen[key] = table_name
        yield DynamicOutput(
            value={
                "table_name": table_name,
                "policy": policy,
                "policy_fetch_error": policy_fetch_error,
            },
            mapping_key=key,
        )


@op(
    retry_policy=DEFAULT_RETRY_POLICY,
    tags=source_metadata(
        "dispar_orchestrate/maintenance.py::maintain_bronze_table",
        sql=None,
        reads=["Iceberg bronze.{table_name}", "maintenance policy for {table_name}"],
        writes=[
            "Iceberg bronze.{table_name} (orphan files, snapshots, compaction)",
            "ClickHouse lake.bronze_meta.maintenance_run",
            "ClickHouse lake.bronze_meta.maintenance_verb_run",
        ],
    ),
)
def maintain_bronze_table(context, payload: dict[str, Any]) -> dict[str, Any]:
    """The per-table maintenance chain. Carries the table's already-
    resolved policy in `payload["policy"]` (looked up once by the
    fan-out op, not once per table). The body delegates to
    `_run_maintenance_for_table`, the same helper
    `test_maintenance.py`'s `RunBronzeMaintenanceTrinoIntegrationTest`
    drives to assert the per-table behaviour end-to-end -- the tests
    and this op are the same code path."""
    cfg = MaintenanceConfig.from_env()
    return _run_maintenance_for_table(
        cfg,
        payload["table_name"],
        payload["policy"],
        payload["policy_fetch_error"],
        context=context,
    )


@op(
    retry_policy=DEFAULT_RETRY_POLICY,
    tags=source_metadata(
        "dispar_orchestrate/maintenance.py::summarize_bronze_maintenance",
        sql=None,
        reads=["per-table result list from maintain_bronze_table[*]"],
        writes=["output metadata tables_maintained"],
    ),
)
def summarize_bronze_maintenance(
    context, results: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    """Collects every mapped `maintain_bronze_table` step's result and
    logs the per-run count (the only number this whole job owes to
    the rest of the code location -- per-table dry-run/applied
    counts live on each `maintain_bronze_table` step's
    `record_maintenance_run` row, not aggregated here)."""
    context.add_output_metadata({"tables_maintained": len(results)})
    return results


# The `dagster/max_runtime` tag is what run monitoring enforces: a run
# whose process died is cancelled and failed by the daemon, so it does not
# hold a run-queue slot forever (`OPS-35`). `ingest_factory.py` says why it
# is a tag and not `run_monitoring.max_runtime_seconds`.
@job(tags=job_tags("bronze_maintenance_job", RUN_LIMIT_ENV, DEFAULT_RUN_LIMIT_SECONDS))
def bronze_maintenance_job() -> None:
    """`DAGSTER_LOCATION`-visible job name: `bronze_maintenance_job`.
    PART C: `list_bronze_tables` is the fan-out (one DynamicOutput per
    Bronze table) -> `maintain_bronze_table` (mapped per-table body)
    -> `summarize_bronze_maintenance` (collects). The `.map(...)`
    form is mandatory here -- `maintain_bronze_table(list_bronze_tables())`
    would emit an "uninvoked op" warning then a graph-build error,
    because the output of `list_bronze_tables()` is a DynamicOutput,
    not a value the mapped op can consume directly."""
    summarize_bronze_maintenance(
        list_bronze_tables().map(lambda payload: maintain_bronze_table(payload)).collect()
    )


# Daily at 03:00 — arbitrary but conservative cadence for a Bronze table
# under active CDC/dlt writes; `dagster-daemon` (already in the P3 compose
# topology, see ADR 0005) is what actually fires this.
# `default_status=RUNNING` is load-bearing, not decoration. A
# `ScheduleDefinition` without it is created STOPPED, so on a fresh
# deployment this never fires until somebody notices and toggles it in the
# Dagster UI — and nothing surfaces that it is off. Bronze maintenance
# silently never running is exactly the failure this job exists to prevent,
# and an off schedule looks identical to a healthy one from outside.
#
# Stated because RUNNING is the less reversible default: on first boot this
# fires at the next 03:00 without anyone opting in. That is intended — an
# operator who wants it off stops it in the UI, a visible action, whereas
# the previous default required a hidden action to get the behaviour the
# module already documented.
bronze_maintenance_schedule = ScheduleDefinition(
    job=bronze_maintenance_job,
    cron_schedule="0 3 * * *",
    default_status=DefaultScheduleStatus.RUNNING,
)

maintenance_defs = Definitions(
    jobs=[bronze_maintenance_job],
    schedules=[bronze_maintenance_schedule],
)
