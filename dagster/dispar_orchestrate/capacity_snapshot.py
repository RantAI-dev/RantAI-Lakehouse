"""P6 Dagster job: daily capacity snapshot into
`lake.bronze_meta.capacity_snapshot`.

# Bucket size via `s3fs`, not `boto3`, not Lakekeeper's Management API

Bucket size comes from RustFS's own S3 API via `s3fs.S3FileSystem` —
NOT Lakekeeper's Management API, which is admin-scoped and which no
long-running service in this stack holds a token for (see
`maintenance.py`'s module doc for the same finding independently reached
for a different admin-scoped call), and NOT `boto3` (not installed in
this project's venv; `s3fs` already arrives transitively via
`dlt[pyiceberg,s3,postgres]` and is pinned directly in `pyproject.toml`
because this module imports it by name, not just through `dlt`).

# Credentials: `CONNECTOR_S3_ACCESS_KEY`/`CONNECTOR_S3_SECRET_KEY`, not
# `RUSTFS_ACCESS_KEY`/`RUSTFS_SECRET_KEY`

`docker-compose.yml`'s `dagster-code-location` environment gives this
container only `CONNECTOR_S3_ACCESS_KEY`/`CONNECTOR_S3_SECRET_KEY` — the
same connector-scoped names `dlt_pipeline.py` reads for the same reason
(that block's own comment: neither tool needs RustFS's root credentials,
only PUT/GET on the one warehouse bucket). `RUSTFS_ACCESS_KEY`/
`RUSTFS_SECRET_KEY` are never set in that container, so reading them here
would silently authenticate anonymously and fail. A missing or empty
value raises `dagster.Failure` (AGENTS.md: a config/auth problem is a
`Failure`, never a silent empty-string default) — see `_required_env`.

# Schema: `lake.bronze_meta.capacity_snapshot`, not `console.capacity_snapshot`

`bronze_catalog.TableSchema.create_ddl` always emits
`CREATE TABLE IF NOT EXISTS lake.\\`{table_name}\\``, and
`_assert_or_create_schema` only ever looks in database `lake`
(`system.tables WHERE database = 'lake'`). A `table_name` is ONE
identifier that may contain a dot, always inside `lake` — see the
existing `bronze_meta.maintenance_run`. So this job's schema
(`bronze_catalog._CAPACITY_SNAPSHOT_SCHEMA`) is
`table_name="bronze_meta.capacity_snapshot"`, giving
``lake.`bronze_meta.capacity_snapshot` ``, defined in `bronze_catalog.py`
so the registry schema keeps exactly one owner (R10) even though the
`INSERT` itself is issued from this module (this module's own test needs
to monkeypatch `_ch_exec` inside its own namespace, not `bronze_catalog`'s).

`clickhouse_bytes_on_disk` is NOT a column here: `GET /api/lakehouse/capacity`
reads ClickHouse's own `system.parts` live, so this table stores the
bucket-measured numbers only — one source per number, never two writers
for the same fact.
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from datetime import datetime, timezone

from dagster import DefaultScheduleStatus, Failure, ScheduleDefinition, job, op
from s3fs import S3FileSystem

from dispar_orchestrate.bronze_catalog import (
    _CAPACITY_SNAPSHOT_SCHEMA,
    ClickHouseTarget,
    _assert_or_create_all,
    _ch_exec,
    _sql_string_literal,
)

# `_CAPACITY_SNAPSHOT_SCHEMA` is defined in `bronze_catalog.py` (next to
# `_MAINTENANCE_RUN_SCHEMA`/`_MAINTENANCE_VERB_RUN_SCHEMA`), not here, so
# the registry schema keeps exactly one owner (R10) even though the
# `INSERT` itself is issued from this module — this module's own test
# needs to monkeypatch `_ch_exec` inside its own namespace, not
# `bronze_catalog`'s.


def _env(name: str, default: str) -> str:
    value = os.environ.get(name, "").strip()
    return value if value else default


def _required_env(name: str) -> str:
    """Reads `name`, raising `dagster.Failure` when it is missing or
    empty. Used for the RustFS S3 credentials this job authenticates
    with: an empty value here means an anonymous (and failing) bucket
    listing, a configuration problem — never a legitimate "nothing to
    measure" case that should degrade quietly."""
    value = os.environ.get(name, "").strip()
    if not value:
        raise Failure(f"{name} is not set — capacity_snapshot cannot authenticate to RustFS")
    return value


@dataclass(frozen=True)
class CapacityConfig:
    ch: ClickHouseTarget
    bucket_name: str
    rustfs_endpoint: str
    rustfs_access_key: str
    rustfs_secret_key: str

    @classmethod
    def from_env(cls) -> "CapacityConfig":
        return cls(
            ch=ClickHouseTarget.from_env(),
            bucket_name=_env("LAKEHOUSE_WAREHOUSE_BUCKET", "lakehouse-warehouse"),
            rustfs_endpoint=_env("CH_RUSTFS_S3_ENDPOINT", "http://rustfs:9000"),
            rustfs_access_key=_required_env("CONNECTOR_S3_ACCESS_KEY"),
            rustfs_secret_key=_required_env("CONNECTOR_S3_SECRET_KEY"),
        )


@dataclass(frozen=True)
class BucketMeasurement:
    bytes: int
    objects: int


def measure_bucket(cfg: CapacityConfig) -> BucketMeasurement:
    """Lists every object under `cfg.bucket_name` via `s3fs.S3FileSystem`
    (pointed at RustFS's endpoint with this job's `CONNECTOR_S3_*`
    credentials) and sums their sizes.

    Uses `find`, NOT `ls`: `ls` is non-recursive (fsspec/s3fs return only
    the immediate children of `path` — top-level prefix "directories" and
    any root-level objects), and this bucket's Iceberg layout nests every
    real data/metadata file several prefixes deep
    (`warehouse/namespace/table/{data,metadata}/...`). An `ls`-based
    count would silently undercount bytes and objects by orders of
    magnitude — everything below the first level would never be seen.
    `find(path, detail=True)` returns a `dict[str, dict]` of every path
    under `path`, recursively, keyed by full path; entries whose own
    `type` is `"directory"` (some backends emit zero-size directory
    markers) are excluded from both the byte sum and the object count —
    a marker is not a stored object.

    `S3FileSystem` is imported at module scope so this function's own
    test can monkeypatch the name directly, per this module's
    no-network testing rule."""
    fs = S3FileSystem(
        key=cfg.rustfs_access_key,
        secret=cfg.rustfs_secret_key,
        client_kwargs={"endpoint_url": cfg.rustfs_endpoint},
    )
    entries = fs.find(cfg.bucket_name, detail=True)
    files = [info for info in entries.values() if info.get("type") != "directory"]
    return BucketMeasurement(
        bytes=sum(info["size"] for info in files),
        objects=len(files),
    )


def _utc_now_ch_timestamp() -> str:
    """A `DateTime64(3, 'UTC')`-parseable literal with millisecond
    precision, e.g. `2026-09-13 12:34:56.789` — no explicit zone suffix,
    because the column's own declared type already fixes it to UTC."""
    now = datetime.now(timezone.utc)
    return now.strftime("%Y-%m-%d %H:%M:%S.") + f"{now.microsecond // 1000:03d}"


def record_capacity_snapshot(
    *,
    bucket_name: str,
    bytes_: int,
    objects: int,
    target: "ClickHouseTarget | None" = None,
) -> None:
    """Insert one row into `lake.bronze_meta.capacity_snapshot`
    (`_CAPACITY_SNAPSHOT_SCHEMA`, owned by `bronze_catalog.py` per R10).
    One row per run — `ReplacingMergeTree ORDER BY (bucket_name,
    measured_at)` means two rows in the same millisecond for the same
    bucket would collide, which cannot happen for a job invoked at most
    once a day."""
    ch = target or ClickHouseTarget.from_env()
    _assert_or_create_all(ch, (_CAPACITY_SNAPSHOT_SCHEMA,))
    values = (
        f"({_sql_string_literal(_utc_now_ch_timestamp())}, "
        f"{_sql_string_literal(bucket_name)}, {int(bytes_)}, {int(objects)})"
    )
    _ch_exec(
        ch,
        "INSERT INTO lake.`bronze_meta.capacity_snapshot` "
        "(measured_at, bucket_name, bytes, objects) VALUES " + values,
    )


@op
def run_capacity_snapshot(context) -> None:
    """P6 op: measure `cfg.bucket_name`'s live object count/bytes via
    `measure_bucket` and record one row per run into
    `lake.bronze_meta.capacity_snapshot`."""
    cfg = CapacityConfig.from_env()
    measurement = measure_bucket(cfg)
    context.log.info(
        f"[{cfg.bucket_name}] {measurement.objects} object(s), "
        f"{measurement.bytes} byte(s)"
    )
    record_capacity_snapshot(
        bucket_name=cfg.bucket_name,
        bytes_=measurement.bytes,
        objects=measurement.objects,
        target=cfg.ch,
    )
    context.add_output_metadata(
        {
            "bucket_name": cfg.bucket_name,
            "bytes": measurement.bytes,
            "objects": measurement.objects,
        }
    )


@job
def capacity_snapshot_job() -> None:
    """`DAGSTER_LOCATION`-visible job name: `capacity_snapshot_job`."""
    run_capacity_snapshot()


# Daily at 02:00 — ahead of `bronze_maintenance_schedule`'s 03:00 so a
# capacity read never overlaps that job's own ClickHouse/Trino load.
# `default_status=DefaultScheduleStatus.RUNNING` for the same reason
# `bronze_maintenance_schedule` states explicitly (`maintenance.py`): a
# schedule created STOPPED never fires until someone notices and toggles
# it in the Dagster UI, and a stopped schedule looks identical to a
# healthy one from outside — capacity silently never being measured is
# exactly the failure this job exists to prevent.
capacity_snapshot_schedule = ScheduleDefinition(
    job=capacity_snapshot_job,
    cron_schedule="0 2 * * *",
    default_status=DefaultScheduleStatus.RUNNING,
)
