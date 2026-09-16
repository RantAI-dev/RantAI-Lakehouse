"""The shared Lakekeeper/Iceberg sink every ingest adapter writes through.

Extracted from `dlt_pipeline.py` (the sink-extraction step of
`docs/superpowers/plans/2026-09-11-ws3-ingestion-tier1.md`) so the
adapters that follow it in the same plan (SQL -- already
`dlt_pipeline.py`'s own `run_bronze_ingest` -- files, REST) share ONE
Lakekeeper-catalog-env install, ONE `filesystem(...)`/`iceberg_adapter`
destination construction, and ONE `dlt.pipeline(...).run(...)` call,
instead of each adapter copying it. `dlt_pipeline.py`'s module docstring
carries the full "why Lakekeeper's REST catalog, not dlt's local-SQLite
fallback" rationale; it is not repeated here since this is the same code,
moved, not a new decision.

# The real source of the row count (WS3 plan review Z9)

Verified against `dlt` 1.30.0, the version installed in this branch's
Dagster venv: `dlt.pipeline(...).run(source, table_name=bronze_table_name,
...)` returns a `LoadInfo`, and the SAME `Pipeline` object's
`pipeline.last_trace.last_normalize_info.row_counts`
(`dlt.pipeline.trace.PipelineTrace.last_normalize_info`, a `NormalizeInfo`
whose `row_counts` property sums `table_metrics[table].items_count` across
every normalize package -- `dlt/common/pipeline.py`) is a `dict[str, int]`
keyed by table name -- the real, measured count of rows dlt's normalize
stage wrote for `bronze_table_name`, not an estimate.

`SinkResult.rows` is `int | None` -- `None`, never a fabricated `0`
(AGENTS.md rule 2, "never fabricate"), in any of three cases where the key
is genuinely absent:
  1. `bronze_table_name` is not present in `row_counts` (a load that wrote
     zero NEW rows this run, and a load that never ran, are NOT the same
     thing -- dlt's own API does not let this code tell them apart without
     deeper inspection this task does not add);
  2. the `LoadInfo` has failed jobs;
  3. `pipeline.last_trace` is itself `None` (can happen when `run()`
     raises before completing a trace).

`SinkResult.load_info_str` is `str(load_info)`, dlt's own human-readable
load summary -- used ONLY for a warning-level log line by callers, never
surfaced to a caller's response/API, since it can contain a file path.
"""

from __future__ import annotations

import json
import logging
import os
from dataclasses import dataclass
from typing import Protocol

import dlt
from dlt.destinations import filesystem
from dlt.destinations.adapters import iceberg_adapter, iceberg_partition

logger = logging.getLogger(__name__)


class BronzeIngestConfigLike(Protocol):
    """Structural type for `SinkConfig.from_bronze_ingest_config`'s input
    -- deliberately a `Protocol`, not an import of
    `dlt_pipeline.BronzeIngestConfig`, so this shared sink module has no
    import-time dependency on any one adapter's config shape (`dlt_pipeline.py`
    imports `adapters/sink.py`, not the other way around)."""

    lakekeeper_catalog_uri: str
    lakekeeper_warehouse: str
    rustfs_endpoint: str
    rustfs_access_key: str
    rustfs_secret_key: str
    warehouse_bucket: str
    lakekeeper_token: str


@dataclass(frozen=True)
class SinkConfig:
    """The six fields `load_via_sink` needs to reach Lakekeeper's REST
    catalog and the RustFS/S3-compatible warehouse bucket behind it --
    strictly a subset of `dlt_pipeline.BronzeIngestConfig` (see
    `docs/superpowers/plans/2026-09-11-ws3-ingestion-tier1.md`'s
    sink-extraction step): no source-database field belongs here, since
    the sink knows nothing about where a `dlt` source's rows came from."""

    lakekeeper_catalog_uri: str
    lakekeeper_warehouse: str
    rustfs_endpoint: str
    rustfs_access_key: str
    rustfs_secret_key: str
    warehouse_bucket: str
    lakekeeper_token: str

    @classmethod
    def from_bronze_ingest_config(cls, config: BronzeIngestConfigLike) -> "SinkConfig":
        return cls(
            lakekeeper_catalog_uri=config.lakekeeper_catalog_uri,
            lakekeeper_warehouse=config.lakekeeper_warehouse,
            rustfs_endpoint=config.rustfs_endpoint,
            rustfs_access_key=config.rustfs_access_key,
            rustfs_secret_key=config.rustfs_secret_key,
            warehouse_bucket=config.warehouse_bucket,
            lakekeeper_token=config.lakekeeper_token,
        )


@dataclass(frozen=True)
class SinkResult:
    """`rows` is the real, measured row count from dlt's own normalize
    trace (see module docstring) -- `None`, never `0`, when unmeasured.
    `load_info_str` (`str(load_info)`) is for a warning-level log line
    only; it can contain a file path and must never reach a caller's
    response."""

    rows: int | None
    has_failed_jobs: bool
    load_info_str: str


def _install_catalog_env(config: SinkConfig) -> None:
    """Point dlt's Iceberg `table_format` at Lakekeeper's REST catalog by
    setting the two env vars `dlt.common.libs.pyiceberg.IcebergConfig`
    resolves (`sections="iceberg_catalog"`).

    `iceberg_catalog_config` is a `Dict[str, Any]`-typed field -- dlt's env
    provider does not do double-underscore key-splitting for a plain dict
    field the way it does for nested dataclasses, so the whole dict must
    be supplied as one JSON-encoded env var
    (`ICEBERG_CATALOG__ICEBERG_CATALOG_CONFIG`). Verified empirically
    against a live Lakekeeper: individual
    `ICEBERG_CATALOG__ICEBERG_CATALOG_CONFIG__URI`-style keys are silently
    ignored (dlt falls back to the ephemeral local catalog with no error),
    while the single JSON-blob env var is picked up correctly and produces
    a REST-catalog-registered table.

    `s3.force-virtual-addressing = "false"` matters on RustFS/SeaweedFS:
    without it, pyiceberg's PyArrow-backed `FileIO` tries virtual-hosted
    addressing (`<bucket>.<endpoint-host>`), which fails DNS resolution
    against a plain path-style S3-compatible endpoint like RustFS/SeaweedFS
    -- this is the same path-style requirement
    `docker-compose.yml`'s `lakekeeper-warehouse-init` already sets for
    Lakekeeper's own storage profile (`"path-style-access": true`), applied
    here on dlt's side of the same S3 endpoint.
    """
    os.environ["ICEBERG_CATALOG__ICEBERG_CATALOG_NAME"] = "default"
    os.environ["ICEBERG_CATALOG__ICEBERG_CATALOG_TYPE"] = "rest"
    catalog_config: dict[str, str] = {
        "type": "rest",
        "uri": config.lakekeeper_catalog_uri,
        "warehouse": config.lakekeeper_warehouse,
        "s3.endpoint": config.rustfs_endpoint,
        "s3.access-key-id": config.rustfs_access_key,
        "s3.secret-access-key": config.rustfs_secret_key,
        "s3.region": "us-east-1",
        "s3.path-style-access": "true",
        "s3.force-virtual-addressing": "false",
    }
    if config.lakekeeper_token:
        # R1 (ADR 0011): pyiceberg's `RestCatalog` accepts a raw static
        # bearer `token` the same way `iceberg-catalog-rest` (Rust) does --
        # sent as-is on every request, no OAuth2 exchange. `None`/absent
        # on a pre-R1 or authz-disabled stack.
        catalog_config["token"] = config.lakekeeper_token
    os.environ["ICEBERG_CATALOG__ICEBERG_CATALOG_CONFIG"] = json.dumps(catalog_config)


def _extract_rows(trace, bronze_table_name: str) -> int | None:
    """The real row-count source (WS3 plan review Z9) -- `trace` is
    `pipeline.last_trace` after `pipeline.run(...)` returns. `None`
    (never `0`) when there is no trace at all, or when
    `bronze_table_name` did not appear in this run's normalize row
    counts (a load that wrote zero new rows and a load that never ran
    are both, honestly, "unknown" from this dict alone)."""
    if trace is None:
        return None
    return trace.last_normalize_info.row_counts.get(bronze_table_name)


def load_via_sink(source, bronze_table_name: str, config: SinkConfig) -> SinkResult:
    """Write `source` (an already-configured `dlt` source, e.g. `sql_database(...)`
    with hints/maps already applied by the calling adapter) to Bronze
    Iceberg through Lakekeeper's REST catalog, and report the real,
    measured outcome.

    # Errors

    Raises whatever `pipeline.run` raises (`PipelineStepFailed`, etc.) --
    this stays a thin, unit-testable wrapper around dlt's own run, not a
    place that swallows load failures.
    """
    _install_catalog_env(config)

    destination = filesystem(
        bucket_url=f"s3://{config.warehouse_bucket}/bronze",
        credentials={
            "aws_access_key_id": config.rustfs_access_key,
            "aws_secret_access_key": config.rustfs_secret_key,
            "endpoint_url": config.rustfs_endpoint,
        },
    )

    # Adapters apply their own `apply_hints`/`add_map` to their resource(s)
    # before calling this function; `iceberg_adapter` accepts a whole
    # `DltSource` (applying to every resource it carries), which is exactly
    # the ONE resource a single-table adapter like `run_bronze_ingest`
    # builds -- matching the original code's `iceberg_adapter(resource,
    # ...)` (`resource` was that source's only member) without this shared
    # sink needing to know the source's internal resource-name key
    # (`apply_hints(table_name=...)` does not rename that key, so this sink
    # cannot look the resource up by `bronze_table_name` the way the
    # pre-extraction code -- which built the source itself -- could).
    iceberg_adapter(
        source,
        partition=[iceberg_partition.day("_ingested_at")],
        # PR #29 review: format-version 2 was claimed "confirmed" without
        # ever being set or asserted. dlt's iceberg destination does not
        # default this itself (pyiceberg's own table-creation default is
        # already v2, but that is pyiceberg's default, not a guarantee this
        # pipeline makes) -- set it explicitly, at table-creation time, so
        # the guarantee is this module's own rather than inherited
        # incidentally from whatever pyiceberg happens to default to.
        # `ops/g3a/g3a_test.py::step_verify_format_version_2` asserts this
        # against the catalog's own REST metadata, not against what dlt
        # reports back.
        table_properties={"format-version": "2"},
    )

    pipeline = dlt.pipeline(
        pipeline_name=f"bronze_ingest_{bronze_table_name}",
        destination=destination,
        # Flat `bronze` dataset (ADR 0004's flat `bronze` namespace) -- every
        # Bronze table this pipeline ever writes lands in the same
        # Lakekeeper namespace `lakehouse-iceberg`'s Rust write path uses.
        dataset_name="bronze",
    )
    load_info = pipeline.run(source, table_name=bronze_table_name, table_format="iceberg")

    # A load with failed jobs never produced a trustworthy normalize count
    # for this run -- `rows` is `None` here even if `row_counts` happens to
    # carry a stale/partial number (WS3 plan review Z9, case 2 of 3).
    rows = None if load_info.has_failed_jobs else _extract_rows(pipeline.last_trace, bronze_table_name)

    if load_info.has_failed_jobs:
        # `str(load_info)` can contain a file path -- log only, never
        # surfaced to a caller/response (see this module's docstring).
        logger.warning("dlt load had failed jobs: %s", str(load_info))

    return SinkResult(
        rows=rows,
        has_failed_jobs=load_info.has_failed_jobs,
        load_info_str=str(load_info),
    )
