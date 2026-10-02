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

# Load modes (`LoadPlan`)

How a run's rows meet the Bronze table they land in. Each behaviour below
was checked against `dlt` 1.30.0 writing Iceberg, not taken from its docs:

- `append` adds every row the source returns. Run twice, the table holds
  two copies. This is what every load did before `LoadPlan` existed, and
  it stays the default of `load_via_sink` itself (a Kafka micro-batch is
  new rows by construction).
- `replace` overwrites the table with what the source returns now
  (pyiceberg `overwrite`; the previous snapshot stays reachable until it
  is expired). An empty source empties the table. dlt also forgets the
  resource's incremental cursor on a replace.
- `incremental` adds only rows whose cursor column is beyond the highest
  value the previous incremental run saw (`dlt.sources.incremental`,
  pushed into the query). Only a `sql_database` source is accepted; see
  `_apply_cursor` for why no other resource is. dlt keeps that value
  in the pipeline state, stored with the data in the bucket, so it
  survives a recreated container. The FIRST incremental run for a cursor
  has nothing to continue from, so it loads everything and replaces the
  table: appending the full table onto rows an earlier mode left there
  would double them. A changed row shows up again as a new version.

Because `replace` forgets the cursor and an `append` run is made to
(`_forget_cursors`), switching a table between modes never leaves a stale
cursor behind that would skip or re-add rows.
"""

from __future__ import annotations

import json
import logging
import os
from collections.abc import Iterable
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any, Protocol

import dlt
from dlt.destinations import filesystem
from dlt.destinations.adapters import iceberg_adapter, iceberg_partition
from dlt.extract.incremental import IncrementalResourceWrapper
from dlt.extract.resource import DltResource
from dlt.extract.source import DltSource
from dlt.pipeline.helpers import pipeline_drop

logger = logging.getLogger(__name__)

LOAD_MODES = ("replace", "append", "incremental")


class UnsupportedLoadMode(ValueError):
    """A source object asks for a load mode this build cannot run as
    written: an unknown mode, or `incremental` without a cursor column."""


@dataclass(frozen=True)
class LoadPlan:
    """How one load's rows meet its Bronze table -- see this module's
    "Load modes" section. `cursor` is the incremental cursor column and is
    set exactly when `mode` is `incremental`."""

    mode: str = "append"
    cursor: str | None = None

    def __post_init__(self) -> None:
        if self.mode not in LOAD_MODES:
            raise UnsupportedLoadMode(f"load mode {self.mode!r} is not one of {', '.join(LOAD_MODES)}")
        if self.mode == "incremental" and not self.cursor:
            raise UnsupportedLoadMode("load mode 'incremental' needs incrementalKey, the column that marks new rows")

    @classmethod
    def from_source_object(cls, obj: dict[str, Any]) -> "LoadPlan":
        """The plan a connector's source object asks for. One saved before
        load modes existed has no `loadMode`; it gets `replace`, so its
        next run leaves one copy of the source instead of adding another.

        # Errors

        Raises `UnsupportedLoadMode` for an unknown mode, or `incremental`
        with no `incrementalKey`.
        """
        mode = obj.get("loadMode") or "replace"
        cursor = (obj.get("incrementalKey") or "").strip() or None
        return cls(mode=mode, cursor=cursor if mode == "incremental" else None)


def _stamp_ingested_at(record: dict[str, Any]) -> dict[str, Any]:
    """Bronze's system ingestion-time column (ADR 0004's
    `bronze::INGESTED_AT_COLUMN` equivalent for the dlt write path) --
    stamped here, the ONE place any Bronze row this sink writes gets it,
    never read from the source: a row arriving with its own `_ingested_at`
    (e.g. a Kafka payload that happens to carry a same-named field) is
    OVERWRITTEN, not preserved, so partitioning never depends on a
    source-provided timestamp existing, being non-null, or meaning what
    this column means."""
    record["_ingested_at"] = datetime.now(timezone.utc)
    return record


def _stamp_rows(rows: Iterable[dict[str, Any]]) -> Iterable[dict[str, Any]]:
    """Lazily stamp a plain iterable of dicts (the Kafka micro-batch shape,
    `ingest_factory.py::run_kafka_stream_batch`'s `batch.rows`) without
    mutating the caller's own dicts -- `batch.rows` is read again by that
    caller (`_record(rows=...)` logs a COUNT, not the rows themselves, but
    nothing here should assume that stays true forever), so each row is
    copied before the stamp is written rather than stamped in place."""
    for row in rows:
        yield _stamp_ingested_at(dict(row))


def _stamp_for_load(source: Any) -> Any:
    """Make `_ingested_at` land on every row `load_via_sink` writes,
    whatever shape `source` is -- a `dlt` `DltSource` (every resource it
    carries), a bare `DltResource`, or a plain iterable of dicts (the
    Kafka micro-batch). This is the ADR 0004 stamp's ONE call site now:
    `dlt_pipeline.py::run_bronze_ingest` used to call `resource.add_map`
    itself, and `ingest_factory.py`'s other adapters never stamped at
    all -- see this module's own docstring for why that was a bug."""
    if isinstance(source, DltSource):
        for resource in source.resources.values():
            _stamp_last(resource)
        return source
    if isinstance(source, DltResource):
        _stamp_last(source)
        return source
    return _stamp_rows(source)


def _stamp_last(resource: DltResource) -> None:
    """Add the stamp as the resource's LAST step. A plain `add_map` lands
    ahead of the resource's incremental cursor step, where the stamp would
    change the hash dlt uses to recognise an already-loaded row -- see
    `_apply_cursor`."""
    resource.add_map(_stamp_ingested_at, insert_at=len(resource._pipe))


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


def _incremental_cursors(pipeline: Any) -> dict[str, dict[str, set[str]]]:
    """Every incremental cursor this pipeline's state holds, as
    `{schema name: {resource name: cursor columns}}`. dlt keeps resource
    state per source schema, and one pipeline has a schema for each kind
    of source it was ever fed."""
    cursors: dict[str, dict[str, set[str]]] = {}
    for schema_name, source_state in (pipeline.state.get("sources") or {}).items():
        for resource_name, resource_state in (source_state.get("resources") or {}).items():
            paths = set((resource_state.get("incremental") or {}).keys())
            if paths:
                cursors.setdefault(schema_name, {})[resource_name] = paths
    return cursors


def _has_cursor(pipeline: Any, cursor: str) -> bool:
    """Whether an earlier incremental run left a value for `cursor` to
    continue from. Reads the state stored with the data first, since a
    recreated container starts with no local copy of it."""
    pipeline.sync_destination()
    return any(
        cursor in paths for resources in _incremental_cursors(pipeline).values() for paths in resources.values()
    )


def _forget_cursors(pipeline: Any) -> None:
    """Drop every incremental cursor after an `append` run. That run added
    the whole source again, so a cursor kept from an earlier incremental
    run would no longer describe what the table holds; the next
    incremental run then starts over (and replaces). A no-op for a
    pipeline that never ran incrementally, which is every Kafka one.

    Schema by schema: `pipeline_drop` looks in one schema only, the
    pipeline's first by default, which is not where the cursor lives once
    a table has been fed by more than one kind of source."""
    for schema_name, resources in sorted(_incremental_cursors(pipeline).items()):
        pipeline_drop(pipeline, resources=sorted(resources), schema_name=schema_name, state_only=True)()


def _apply_cursor(source: Any, cursor: str) -> None:
    """Attach the incremental cursor to what `source` loads.

    Only a resource that takes the cursor itself is accepted, which is what
    a `sql_database` table is: it puts the cursor in its query, and its
    cursor step stays where it is, so `_stamp_last` can put the
    `_ingested_at` stamp AFTER it. That order is what makes a rerun exact.
    dlt re-reads the rows at the cursor's last value and recognises the
    ones it already loaded by their primary key, or by hashing the whole
    row when the table has none. On any other resource dlt moves the
    cursor filter behind every map step whenever hints are applied, so the
    stamp would change that hash on every run and the boundary row would
    be added again each time (seen against a real Iceberg table: +3 then
    +1 rows where +2 then 0 were due).

    # Errors

    Raises `UnsupportedLoadMode` for plain rows and for a resource that
    does not take a cursor itself.
    """
    if isinstance(source, DltSource):
        resources = list(source.selected_resources.values())
    elif isinstance(source, DltResource):
        resources = [source]
    else:
        raise UnsupportedLoadMode("load mode 'incremental' needs a dlt source or resource, not plain rows")
    for resource in resources:
        if not isinstance(resource.incremental, IncrementalResourceWrapper):
            raise UnsupportedLoadMode(
                f"load mode 'incremental' is not available for {resource.name!r}: its source cannot filter by a "
                "cursor before the rows are stamped, so reruns would add the last row again"
            )
        resource.apply_hints(incremental=dlt.sources.incremental(cursor))


def load_via_sink(
    source, bronze_table_name: str, config: SinkConfig, plan: LoadPlan = LoadPlan()
) -> SinkResult:
    """Write `source` (an already-configured `dlt` source, e.g. `sql_database(...)`
    with hints/maps already applied by the calling adapter) to Bronze
    Iceberg through Lakekeeper's REST catalog, and report the real,
    measured outcome. `plan` says how the rows meet the table (this
    module's "Load modes" section); the default appends.

    # Errors

    Raises whatever `pipeline.run` raises (`PipelineStepFailed`, etc.) --
    this stays a thin, unit-testable wrapper around dlt's own run, not a
    place that swallows load failures. Raises `UnsupportedLoadMode` for an
    incremental plan over plain rows.
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

    # ADR 0004: every Bronze row carries `_ingested_at`, stamped here --
    # the ONE owner of the column, regardless of what shape a caller
    # handed this sink (`_stamp_for_load`, above). This used to be true
    # only for `dlt_pipeline.py::run_bronze_ingest`'s own resource (it
    # called `add_map` itself before reaching this function); every other
    # caller -- `ingest_factory.py`'s batch adapters and its Kafka
    # micro-batch -- sent rows with no `_ingested_at` at all, which fails
    # outright for a `sql_database` source (`iceberg_adapter`'s partition
    # spec below names a field `build_iceberg_partition_spec` cannot find)
    # and silently under-partitions everything else. Stamping unconditionally
    # here, before `iceberg_adapter` fixes the partition spec to that
    # column, closes that gap for every caller at once.
    if plan.mode == "incremental":
        _apply_cursor(source, plan.cursor or "")
    source = _stamp_for_load(source)

    # Adapters still apply their own `apply_hints` (table name, primary
    # key, etc.) to their resource(s) before calling this function;
    # `iceberg_adapter` accepts a whole `DltSource` (applying to every
    # resource it carries), which is exactly the ONE resource a
    # single-table adapter like `run_bronze_ingest` builds -- matching the
    # original code's `iceberg_adapter(resource, ...)` (`resource` was
    # that source's only member) without this shared sink needing to know
    # the source's internal resource-name key (`apply_hints(table_name=...)`
    # does not rename that key, so this sink cannot look the resource up
    # by `bronze_table_name` the way the pre-extraction code -- which
    # built the source itself -- could).
    adapted = iceberg_adapter(
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
    # For a `DltSource`/`DltResource`, `iceberg_adapter` applies the hints
    # in place to the resource `source` already carries. For plain rows
    # (the Kafka micro-batch) it wraps them in a NEW resource and applies
    # the hints only to that, so running the raw rows would drop the
    # partition spec and `format-version` above. Run what it returned.
    if not isinstance(source, (DltSource, DltResource)):
        source = adapted

    pipeline = dlt.pipeline(
        pipeline_name=f"bronze_ingest_{bronze_table_name}",
        destination=destination,
        # Flat `bronze` dataset (ADR 0004's flat `bronze` namespace) -- every
        # Bronze table this pipeline ever writes lands in the same
        # Lakekeeper namespace `lakehouse-iceberg`'s Rust write path uses.
        dataset_name="bronze",
    )
    write_disposition = "append" if plan.mode == "incremental" else plan.mode
    if plan.mode == "incremental" and not _has_cursor(pipeline, plan.cursor or ""):
        # Nothing to continue from: load everything, and replace rather
        # than stack it onto what an earlier mode left there.
        write_disposition = "replace"
    load_info = pipeline.run(
        source, table_name=bronze_table_name, table_format="iceberg", write_disposition=write_disposition
    )

    # A load with failed jobs never produced a trustworthy normalize count
    # for this run -- `rows` is `None` here even if `row_counts` happens to
    # carry a stale/partial number (WS3 plan review Z9, case 2 of 3).
    rows = None if load_info.has_failed_jobs else _extract_rows(pipeline.last_trace, bronze_table_name)
    if rows is None and plan.mode == "incremental" and not load_info.has_failed_jobs and pipeline.last_trace:
        # The one case an absent count IS a measurement: the cursor filter
        # ran and let nothing through, so this run added zero rows.
        rows = 0

    # After the count is read: forgetting cursors is a (state-only) load of
    # its own, which replaces `pipeline.last_trace`.
    if plan.mode == "append" and not load_info.has_failed_jobs:
        _forget_cursors(pipeline)

    if load_info.has_failed_jobs:
        # `str(load_info)` can contain a file path -- log only, never
        # surfaced to a caller/response (see this module's docstring).
        logger.warning("dlt load had failed jobs: %s", str(load_info))

    return SinkResult(
        rows=rows,
        has_failed_jobs=load_info.has_failed_jobs,
        load_info_str=str(load_info),
    )
