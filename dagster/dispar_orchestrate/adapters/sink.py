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
- `replace_dedup` is `replace` keeping ONE row per key (`LoadPlan.key`): the
  rows are deduplicated (`adapters/dedup.py`: the highest cursor value wins,
  else the last row read) and the table is replaced with the winners on every
  run. It never merges. Any batch adapter can run it, SQL or not.
- `incremental_dedup` is `incremental` followed by a merge: the cursor filter
  runs first, the rows it lets through are deduplicated, and they are merged
  into the table on the key (`dlt` write disposition `merge`, strategy
  `upsert`; pyiceberg `Table.upsert`). Only a `sql_database` source, as for
  `incremental`. Its FIRST run loads everything and REPLACES the table with the
  winners, so a merge only ever meets a table that holds one row per key. A
  merge does NOT repair a duplicate (copies in different data files are both
  updated and both stay: `SRC-10` measurement 3), so "first" must be decided
  exactly. The rule (`SRC-10` third pass, R2): every run of a deduplicating mode
  leaves a MARKER, the key columns it deduplicated on, in the pipeline state beside
  the incremental cursor (`dedup.write_marker`, in the resource state, written while
  the rows are extracted; see `adapters/dedup.py`, "The marker"). A run of
  `incremental_dedup` is the first one, and replaces, when the marker is absent OR
  names other key columns than the plan's OR no cursor exists. When a cursor exists
  but the marker does not match (the table was loaded with `incremental`, or with
  another key), the cursors are forgotten first so that the run reads everything. A
  run of any other mode that writes rows (`replace`, `append`, `incremental`)
  removes the marker (`dedup.clear_marker`, from the stamp), because its rows may
  repeat a key. The marker is stored with the data in the bucket like the cursor,
  so it survives a recreated container, and `Pipeline.abort_packages` restores it
  with the rest of the state when a load fails, so a failed first run is still the
  first run next time (`test_sink_dedup_marker.py`). Switching back from a
  deduplicating mode to `incremental` therefore costs the next switch to
  `incremental_dedup` one full replace, never a silent duplicate.

A merge above 1000 rows is SEVERAL commits: `dlt`'s `merge_iceberg_table`
upserts the batch in slices of 1000 rows, each its own transaction
(`dlt/common/libs/pyiceberg.py`; `SRC-10` first pass, case l). For the
deduplicating modes that is acceptable and is stated here rather than hidden:
the batch has one row per key, so every slice, and therefore every commit, leaves
at most one row per key; a run that fails part-way leaves the committed slices
and not the rest; and the next run re-reads the source and brings the table to
its state, because a merge is idempotent on a key (`SRC-10` measurement 5,
`test_5_a_failure_in_a_later_slice_...`). Readers can see a table between two
slices. A mode that needs ALL its rows or none (`history`, not built) cannot use
this.

# What a failed load leaves behind (`SRC-10` measurement 4)

A load that fails leaves its package pending in the pipeline's working
directory. `dlt` then makes the next `run` load that package and NOT extract the
data it was given, so every later load of the table fails or reports the dead
run's rows until the package is dropped. Every run here re-reads its rows from
the source, so a pending package of a failed run is never the thing to retry:
`load_via_sink` aborts the pipeline's pending packages (`Pipeline.abort_packages`;
`drop_pending_packages` is deprecated in 1.30.0 and only calls it) when a load
raises or ends with failed jobs, for every mode. Aborting also restores the
pipeline state to before the package, so the incremental cursor does not move
past rows that were never loaded. Measured for replace, append, incremental,
merge and a failure at normalize (`test_load_modes_dedup_measurement.py`,
`test_sink_pending_packages.py`). Not covered: a package left by a PROCESS that
died mid-load is not seen until the next run, which would then finish it instead
of reading (reported, not changed here).

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
from dlt.pipeline.exceptions import PipelineStepFailed
from dlt.pipeline.helpers import pipeline_drop

from dispar_orchestrate.adapters import dedup

logger = logging.getLogger(__name__)

LOAD_MODES = ("replace", "append", "incremental", "replace_dedup", "incremental_dedup")
# SRC-10: the modes that keep one row per key, and the modes that read only rows
# beyond a cursor (the cursor is pushed into the query, so only `sql_database`).
DEDUP_MODES = ("replace_dedup", "incremental_dedup")
INCREMENTAL_MODES = ("incremental", "incremental_dedup")

# The most columns and the longest column name a key may have: the same bounds
# as `validate_load_modes` in `rust/crates/lakehouse-store/src/ingest_spec.rs`,
# which refuses a key at save time; a spec that got here another way is refused
# again at run time.
MAX_KEY_COLUMNS = 16
MAX_KEY_COLUMN_LENGTH = 128


class UnsupportedLoadMode(ValueError):
    """A source object asks for a load mode this build cannot run as
    written: an unknown mode, `incremental` without a cursor column, or a
    deduplicating mode without a usable key."""


def _check_key(mode: str, key: tuple[str, ...]) -> None:
    if not key:
        raise UnsupportedLoadMode(
            f"load mode {mode!r} needs primaryKey, the columns that identify one row"
        )
    problem = (
        len(key) > MAX_KEY_COLUMNS
        or len(set(key)) != len(key)
        or any(
            not isinstance(column, str)
            or not column.strip()
            or len(column) > MAX_KEY_COLUMN_LENGTH
            or any(ord(ch) < 32 or ord(ch) == 127 for ch in column)
            for column in key
        )
    )
    if problem:
        raise UnsupportedLoadMode(
            f"load mode {mode!r} has a primaryKey that is not a list of at most {MAX_KEY_COLUMNS} distinct, "
            f"non-blank column names of at most {MAX_KEY_COLUMN_LENGTH} characters without control characters"
        )


@dataclass(frozen=True)
class LoadPlan:
    """How one load's rows meet its Bronze table -- see this module's
    "Load modes" section. `cursor` is the incremental cursor column: required
    for `incremental` and `incremental_dedup`, optional for `replace_dedup`
    (where it only decides which of two rows with one key wins), unused
    otherwise. `key` is the columns that identify one row, required for the
    deduplicating modes and empty for the others (SRC-10)."""

    mode: str = "append"
    cursor: str | None = None
    key: tuple[str, ...] = ()

    def __post_init__(self) -> None:
        if self.mode not in LOAD_MODES:
            raise UnsupportedLoadMode(f"load mode {self.mode!r} is not one of {', '.join(LOAD_MODES)}")
        if self.mode in INCREMENTAL_MODES and not self.cursor:
            raise UnsupportedLoadMode(
                f"load mode {self.mode!r} needs incrementalKey, the column that marks new rows"
            )
        if self.mode in DEDUP_MODES:
            _check_key(self.mode, self.key)

    @classmethod
    def from_source_object(cls, obj: dict[str, Any]) -> "LoadPlan":
        """The plan a connector's source object asks for. One saved before
        load modes existed has no `loadMode`; it gets `replace`, so its
        next run leaves one copy of the source instead of adding another.

        `primaryKey` is read only for a deduplicating mode: the console keeps
        a detected key on the object while the user switches modes, and a key
        on another mode is ignored, as the API ignores it (SRC-10).

        # Errors

        Raises `UnsupportedLoadMode` for an unknown mode, an incremental mode
        with no `incrementalKey`, or a deduplicating mode with no usable
        `primaryKey`.
        """
        mode = obj.get("loadMode") or "replace"
        cursor = (obj.get("incrementalKey") or "").strip() or None
        raw_key = obj.get("primaryKey") if mode in DEDUP_MODES else None
        if raw_key is not None and not isinstance(raw_key, (list, tuple)):
            raise UnsupportedLoadMode(f"load mode {mode!r} has a primaryKey that is not a list of column names")
        return cls(
            mode=mode,
            cursor=cursor if mode in INCREMENTAL_MODES or mode == "replace_dedup" else None,
            key=tuple(raw_key or ()),
        )


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


def _stamp_rows(rows: Iterable[dict[str, Any]], clear_marker: bool = False) -> Iterable[dict[str, Any]]:
    """Lazily stamp a plain iterable of dicts (the Kafka micro-batch shape,
    `ingest_factory.py::run_kafka_stream_batch`'s `batch.rows`) without
    mutating the caller's own dicts -- `batch.rows` is read again by that
    caller (`_record(rows=...)` logs a COUNT, not the rows themselves, but
    nothing here should assume that stays true forever), so each row is
    copied before the stamp is written rather than stamped in place."""
    for row in rows:
        if clear_marker:
            dedup.clear_marker()
        yield _stamp_ingested_at(dict(row))


def _stamp_for_load(source: Any, clear_marker: bool = False) -> Any:
    """Make `_ingested_at` land on every row `load_via_sink` writes,
    whatever shape `source` is -- a `dlt` `DltSource` (every resource it
    carries), a bare `DltResource`, or a plain iterable of dicts (the
    Kafka micro-batch). This is the ADR 0004 stamp's ONE call site now:
    `dlt_pipeline.py::run_bronze_ingest` used to call `resource.add_map`
    itself, and `ingest_factory.py`'s other adapters never stamped at
    all -- see this module's own docstring for why that was a bug.

    `clear_marker` (SRC-10 R2): the same stamp also removes the deduplication
    marker, because a load that is not a deduplicating one writes rows that may
    repeat a key (`dedup.clear_marker`). It rides on the stamp instead of being a
    step of its own so the pipe keeps the steps it had."""
    if isinstance(source, DltSource):
        for resource in source.resources.values():
            _stamp_last(resource, clear_marker)
        return source
    if isinstance(source, DltResource):
        _stamp_last(source, clear_marker)
        return source
    return _stamp_rows(source, clear_marker)


def _stamp_last(resource: DltResource, clear_marker: bool = False) -> None:
    """Add the stamp as the resource's LAST step. A plain `add_map` lands
    ahead of the resource's incremental cursor step, where the stamp would
    change the hash dlt uses to recognise an already-loaded row -- see
    `_apply_cursor`."""
    if not clear_marker:
        resource.add_map(_stamp_ingested_at, insert_at=len(resource._pipe))
        return
    name = resource.name

    def stamp_and_clear(record: dict[str, Any]) -> dict[str, Any]:
        dedup.clear_marker(name)
        return _stamp_ingested_at(record)

    resource.add_map(stamp_and_clear, insert_at=len(resource._pipe))


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
class LoadedColumn:
    """One column of the Bronze table as `dlt`'s schema holds it after a load
    (SRC-8): `data_type` is `dlt`'s own name (`bigint`, `text`, `timestamp`,
    ...)."""

    name: str
    data_type: str
    nullable: bool


@dataclass(frozen=True)
class SinkResult:
    """`rows` is the real, measured row count from dlt's own normalize
    trace (see module docstring) -- `None`, never `0`, when unmeasured.
    `load_info_str` (`str(load_info)`) is for a warning-level log line
    only; it can contain a file path and must never reach a caller's
    response.

    `columns` (SRC-8, `schema_observer.py`) are the table's columns after the
    load, WITHOUT `dlt`'s bookkeeping and `_ingested_at` (see
    `_loaded_columns`); `()` when they could not be read, never a guess."""

    rows: int | None
    has_failed_jobs: bool
    load_info_str: str
    columns: tuple[LoadedColumn, ...] = ()


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
    if trace is None or trace.last_normalize_info is None:
        # No normalize step ran in this run: `dlt` only finished a package an
        # earlier run left pending (SRC-10 measurement 4).
        return None
    return trace.last_normalize_info.row_counts.get(bronze_table_name)


def _loaded_columns(pipeline: Any, bronze_table_name: str) -> tuple[LoadedColumn, ...]:
    """The columns `dlt`'s schema holds for `bronze_table_name` after a load,
    for the after-load schema observation (SRC-8, decision D4).

    Left OUT, on purpose: `dlt`'s own bookkeeping (`_dlt_id`, `_dlt_load_id`,
    every `_dlt_*` name) and `_ingested_at`, the system column this module
    stamps itself (ADR 0004). None of them is a column of the SOURCE, so
    reporting them would make every first observation of a table carry them
    and a source that really has a column of that name could not be told
    apart. A `<column>__v_<type>` variant column, which `dlt` adds when a
    value does not fit the column's type, IS a real column of the Bronze
    table and is reported as it is.

    The table lives in whichever of the pipeline's schemas the source was
    named after, so every schema is searched, the default one first. Reading
    the schema must never fail a load that already succeeded: any problem
    gives `()` and a warning that names only the exception type.
    """
    try:
        schemas = [pipeline.default_schema, *pipeline.schemas.values()]
        for schema in schemas:
            table = schema.tables.get(bronze_table_name)
            if table is None:
                continue
            return tuple(
                LoadedColumn(
                    name=name,
                    data_type=column["data_type"],
                    nullable=column.get("nullable", True),
                )
                for name, column in table.get("columns", {}).items()
                if "data_type" in column and not name.startswith("_dlt_") and name != "_ingested_at"
            )
    except Exception as exc:  # noqa: BLE001 -- see docstring: reported, never raised
        logger.warning("could not read the loaded columns of %r (%s)", bronze_table_name, type(exc).__name__)
    return ()


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


def _has_cursor(pipeline: Any, cursor: str, *, sync: bool = True) -> bool:
    """Whether an earlier incremental run left a value for `cursor` to
    continue from. Reads the state stored with the data first, since a
    recreated container starts with no local copy of it (`sync=False` when the
    caller already did)."""
    if sync:
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


def _apply_cursor(source: Any, cursor: str, mode: str = "incremental") -> None:
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
        raise UnsupportedLoadMode(f"load mode {mode!r} needs a dlt source or resource, not plain rows")
    for resource in resources:
        if not isinstance(resource.incremental, IncrementalResourceWrapper):
            raise UnsupportedLoadMode(
                f"load mode {mode!r} is not available for {resource.name!r}: its source cannot filter by a "
                "cursor before the rows are stamped, so reruns would add the last row again"
            )
        resource.apply_hints(incremental=dlt.sources.incremental(cursor))


def _abort_pending(pipeline: Any) -> None:
    """Abort the pipeline's pending packages and restore its state to before
    them (`Pipeline.abort_packages`; `SRC-10` measurement 4). Must never mask
    the failure that got us here, so a problem aborting is logged by type and
    left."""
    try:
        if pipeline.has_pending_data:
            pipeline.abort_packages()
    except Exception as exc:  # noqa: BLE001 -- see docstring: reported, never raised
        logger.warning("could not abort the pending packages of %r (%s)", pipeline.pipeline_name, type(exc).__name__)


def _refusal_in(exc: BaseException) -> dedup.DedupRefused | None:
    """The `DedupRefused` a deduplication step raised, or `None`. `dlt` wraps an
    exception of a pipe step twice -- `ResourceExtractionError` or
    `PipeItemProcessingError` inside `PipelineStepFailed` (measured) -- so the
    chain is walked (bounded, as `ingest_factory._classify_exception` does).
    Unwrapped so the caller sees the fixed-message refusal and the ingest op can
    report it as a config-shaped failure."""
    current: BaseException | None = exc
    for _ in range(8):
        if current is None:
            return None
        if isinstance(current, dedup.DedupRefused):
            return current
        inner = current.exception if isinstance(current, PipelineStepFailed) else None
        current = inner if isinstance(inner, BaseException) else (current.__cause__ or current.__context__)
    return None


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
    incremental plan over plain rows, and `dedup.DedupRefused` (a `ValueError`
    with a fixed message) when a deduplicating plan cannot be applied: a row
    with no key value, more distinct keys than `INGEST_DEDUP_MAX_ROWS`, cursor
    values that cannot be ordered. On any failure the pipeline's pending
    packages are aborted first (see "What a failed load leaves behind").
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
    if plan.mode in INCREMENTAL_MODES:
        _apply_cursor(source, plan.cursor or "", plan.mode)
    # SRC-10 (planner decision 6): deduplicate AFTER the cursor filter and BEFORE
    # the stamp. The filter has then already dropped the rows beyond reach, and the
    # stamp is written on the winner only, so it can never decide which row wins.
    dedup_steps: list[Any] = []
    if plan.mode in DEDUP_MODES:
        source, dedup_steps = dedup.attach(
            source, key=plan.key, cursor=plan.cursor, table=bronze_table_name
        )
    # SRC-10 R2: unconditional for every mode that does not deduplicate (a no-op
    # when no marker is there, and it changes no state then), so a replace or append
    # run costs no read of the pipeline state in the bucket. A plain-rows `append` is
    # the Kafka micro-batch, which no deduplicating mode shares a table with.
    plain_append = plan.mode == "append" and not isinstance(source, (DltSource, DltResource))
    source = _stamp_for_load(source, clear_marker=plan.mode not in DEDUP_MODES and not plain_append)

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
    # What the pipeline state says about earlier runs, read from the bucket first
    # (a recreated container has no local copy). Only the modes that continue from
    # a cursor consult it; the others cost no extra read.
    key_marked = False
    if plan.mode in INCREMENTAL_MODES:
        pipeline.sync_destination()
        markers = dedup.markers(pipeline.state)
        key_marked = bool(markers) and all(marker == list(plan.key) for marker in markers)
    run_options: dict[str, Any] = {}
    if plan.mode == "incremental":
        write_disposition: Any = "append"
    elif plan.mode == "replace_dedup":
        # Never a merge: the winners replace the table on every run.
        write_disposition = "replace"
    elif plan.mode == "incremental_dedup":
        write_disposition = {"disposition": "merge", "strategy": "upsert"}
        run_options["primary_key"] = list(plan.key)
    else:
        write_disposition = plan.mode
    if plan.mode == "incremental_dedup" and not key_marked and _has_cursor(pipeline, plan.cursor or "", sync=False):
        # SRC-10 R2: a cursor exists (the table was loaded with `incremental`, or
        # with another key) but no run has left the table deduplicated on THIS key.
        # The run must read everything and replace, so the cursor goes first. Done
        # before the run and failing safe: with no cursor, a failed run is "first"
        # again next time.
        _forget_cursors(pipeline)
    if plan.mode in INCREMENTAL_MODES and (
        not _has_cursor(pipeline, plan.cursor or "", sync=False)
        or (plan.mode == "incremental_dedup" and not key_marked)
    ):
        # Nothing to continue from: load everything, and replace rather
        # than stack it onto what an earlier mode left there. For
        # `incremental_dedup` the replace also rewrites a table an earlier
        # `append` run left with duplicate keys (SRC-10 measurement 3). The key
        # stays on this run too: dlt recognises the rows at the cursor boundary
        # by a hash of the key (else of the whole row), so the run that stores
        # the hash and the runs that compare it must agree, or every later run
        # re-reads and re-merges the last row.
        write_disposition = "replace"
    try:
        load_info = pipeline.run(
            source,
            table_name=bronze_table_name,
            table_format="iceberg",
            write_disposition=write_disposition,
            **run_options,
        )
        dedup.assert_flushed(dedup_steps, table=bronze_table_name)
    except Exception as exc:
        # SRC-10 measurement 4: a failed load leaves its package pending and the
        # next run would finish THAT instead of reading; this run's rows are
        # re-read from the source anyway, so drop it, for every mode.
        _abort_pending(pipeline)
        refused = _refusal_in(exc)
        if refused is not None:
            raise refused from exc
        raise

    # A load with failed jobs never produced a trustworthy normalize count
    # for this run -- `rows` is `None` here even if `row_counts` happens to
    # carry a stale/partial number (WS3 plan review Z9, case 2 of 3).
    rows = None if load_info.has_failed_jobs else _extract_rows(pipeline.last_trace, bronze_table_name)
    if rows is None and plan.mode in INCREMENTAL_MODES and not load_info.has_failed_jobs and pipeline.last_trace:
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
        _abort_pending(pipeline)

    return SinkResult(
        rows=rows,
        has_failed_jobs=load_info.has_failed_jobs,
        load_info_str=str(load_info),
        columns=() if load_info.has_failed_jobs else _loaded_columns(pipeline, bronze_table_name),
    )
