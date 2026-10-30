"""The change-capture applier (`SRC-4` slice 2, ADR 0016 point 3): reads the
change topics of a change-capture connector and writes them to Bronze.

# Where it fits

`Kafka Connect` (started by `lakehouse-api`, ADR 0016 point 1) runs Debezium for
each change-capture connector and writes one topic per source table,
`cdc.<connector id>.<schema>.<table>`. This module is the Dagster side: one static
job, `cdc_apply_job`, whose config names a connector, and one sensor,
`cdc_apply_sensor`, that starts it for a connector whose topics hold changes it has
not applied. The first version of this code is the slice 0 prototype
(`origin/measure/src-4-cdc`, measured in `docs/plans/SRC-4-RESULT.md`); what changed
is listed under "Production differences".

# What a run does

For each table the connector selects (its `sourceObjects`), in order: read the topic in
batches of up to `BATCH_EVENTS` events, turn Debezium events into rows, write them to
the table's Bronze target in the table's load mode, and commit the Kafka offsets only
after the Bronze commit (so a crash replays a batch and never loses one). A run ends
when every table has caught up, `RUN_SECONDS` have passed (the sensor starts the next
one), or a schema decision is pending.

Load modes (`SRC-4` decision S2-g; the console shows them as "Current row per key" and
"History"):

- `incremental_dedup` (the default; applied as `current`): within a batch only the LAST
  event per key counts. The batch is applied in ONE `pyiceberg` transaction of this
  module's own, as `adapters/history.py` does: delete the batch's keys, append the last
  version of every key whose last event is not a delete. A delete event only deletes.
  Replaying a batch gives the same table, so a crash between the Bronze commit and the
  offset commit is harmless. Why not `dlt`'s `hard_delete`: `dlt` 1.30.0's Iceberg merge
  has no delete branch (plan Amendment 1).
- `history`: every event is appended through `adapters/sink.py` (`LoadPlan` `append`)
  with its operation (`_op`), the source commit time (`_source_ts_ms`), the delete flag
  (`_deleted`) and the Kafka offset (`_offset`, the order of the events of one key).

The first batch of a table is written by `sink.load_via_sink`, so `dlt` makes the table
every other load mode makes (day partition on `_ingested_at`, format version 2).

# Event shape

Debezium's `ExtractNewRecordState` with `delete.tombstone.handling.mode=rewrite` (the name
Debezium 3.8 reads; `delete.handling.mode` is ignored there, `docs/plans/SRC-4-RESULT.md`)
and `add.fields=op,source.ts_ms`, JSON without schemas: a flat object of the row's
columns plus `__op` (`c`, `u`, `d`, `r`), `__source_ts_ms` and `__deleted`. A delete
carries the old row (the source tables are `REPLICA IDENTITY FULL`). A tombstone (a
message with no value) carries nothing and is skipped. Decimals arrive as text
(`decimal.handling.mode=string`); a `timestamptz` arrives as an ISO string; a `date` or
a `timestamp` WITHOUT time zone arrives as an integer (Debezium's temporal types), so
its Bronze column is an integer, not a date. That is a stated limit
(`docs/core/features/databases-and-change-capture.md`), not a conversion this module
guesses at without the column's declared type.

# Schema changes (`SRC-8`)

A batch that brings a column the Bronze table lacks is reported to the API
(`schema_observer.post_observation`, phase `before_load`) before anything is written.
The API decides (its policy): `wait` pauses the connector (the API pauses the
`Kafka Connect` connector too) and this run ends with nothing written and no offset
committed, so the same batch is read again when the connector is resumed; `load` adds
the column (a `current` table through `pyiceberg`'s `union_by_name`, a `history` table
through `dlt`) and, when the API names columns, leaves the others out. A column that
disappears is not detected here: a removed column simply stops arriving. A refused
schema is never retried: `Failure(allow_retries=False)` (a retry hid one for 13 minutes
in slice 0).

# Lag (`SRC-4` slice 4)

The orchestrator observes, the API decides what lag is (`lakehouse-api` `capture_lag.rs`). Two
best-effort posts go to `POST /api/connectors/{id}/capture/observations`:

- the sensor posts each table's backlog (unread messages on its topic) on every tick, for EVERY
  change-capture connector, paused, busy or failed ones included: a failed connector is the case
  the lag alert exists for. A table whose backlog could not be measured is left out, never sent as 0;
- the applier posts, after each Bronze commit, the largest `_source_ts_ms` of the batch, the source
  commit time of the last change it applied.

A post that cannot be sent is logged by kind and the load goes on (a report never fails the load, as
in `run_reports`); nothing is queued for later, so the API shows "not known since" instead.

# Production differences from the prototype

- Config comes from the connector (`GET /api/connectors/ingestible`): topics from its
  source objects, Bronze targets, keys (composite keys too) and load modes.
- Run reports (`run_reports.RunReporter`): start, one attempt per table, end, so
  `SRC-11`'s history and failure streak and `SRC-7`'s alerts apply unchanged.
- The broker is this deployment's own (`CDC_KAFKA_BOOTSTRAP`, set by compose), not an
  address a person typed, so the SSRF guard that wraps every person-controlled dial is
  not applied: it would refuse the compose-internal broker unless the whole code
  location were allowed to dial internal hosts, which would weaken every other
  connector. This is the one place `ssrf_guard` is gone around, and why.
- Consumers assign the topic's partitions instead of joining a group (no rebalance
  wait); the group id only holds the committed offsets. Topics are never auto-created
  by a consumer.
- Column names go through `dlt`'s naming (`schema_observer.loaded_column_name`), the
  names the Bronze table has.
"""

from __future__ import annotations

import functools
import json
import logging
import os
import time
import warnings
from collections.abc import Callable, Iterable, Mapping, Sequence
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any, Protocol

import pyarrow as pa
import requests
from dagster import (
    DagsterRunStatus,
    DefaultSensorStatus,
    Failure,
    RunRequest,
    RunsFilter,
    SensorEvaluationContext,
    SensorResult,
    job,
    op,
    sensor,
)
from dlt.common.utils import uniq_id_base64
from pyiceberg.expressions import And, BooleanExpression, EqualTo, In, Or

from dispar_orchestrate import dlt_pipeline, ingest_errors, run_reports, schema_observer
from dispar_orchestrate.adapters import history
from dispar_orchestrate.adapters import sink as sink_adapter
from dispar_orchestrate.adapters.kafka import compute_lag
from dispar_orchestrate.ingest_factory import IngestFactoryConfig, _fetch_connector_for_run, _headers
from dispar_orchestrate.op_metadata import DEFAULT_RETRY_POLICY, source_metadata

logger = logging.getLogger(__name__)

# The applier's two ways of writing; the connector's load mode names map onto them.
MODES = ("current", "history")
APPLY_MODE_OF = {"incremental_dedup": "current", "history": "history"}
DEFAULT_LOAD_MODE = "incremental_dedup"
# What Debezium adds (`add.fields=op,source.ts_ms` and the delete flag) and the names
# `dlt` gives them (a leading double underscore becomes one; slice 0 measured it).
DEBEZIUM_FIELDS = ("__op", "__source_ts_ms", "__deleted")
EVENT_COLUMNS = ("_op", "_source_ts_ms", "_deleted", "_offset")
# Columns the table has that no source column is: never reported as the source's.
OWN_COLUMNS = (*history.SYSTEM_COLUMNS, *EVENT_COLUMNS, "_ingested_at", "_dlt_id", "_dlt_load_id")
TOPIC_PREFIX = "cdc"
BATCH_EVENTS = 10_000
POLL_MS = 1_000
# The longest one run keeps applying batches before it ends; the sensor starts the next
# one. Bounds the run, not the work: a backlog just takes several runs.
RUN_SECONDS = 1_200
SENSOR_SECONDS = 30
TRIGGER_TAG = "lakehouse/trigger"
CONNECTOR_TAG = "cdc/connector"

_IN_PROGRESS = [
    DagsterRunStatus.QUEUED,
    DagsterRunStatus.NOT_STARTED,
    DagsterRunStatus.STARTING,
    DagsterRunStatus.STARTED,
]


class CdcEventError(ValueError):
    """A message is not a Debezium event in the shape this applier reads. The text is
    fixed: it never carries the message (it can hold row values)."""


class CdcSchemaChange(RuntimeError):
    """A batch carries a column the Bronze table lacks and nothing allowed adding it
    (fixed text, names no value)."""


class CdcSetupError(ValueError):
    """The connector's settings cannot be applied (no key, a load mode change capture
    does not run, a name that is not `schema.table`). Fixed text; retrying would get the
    same answer."""


@dataclass(frozen=True)
class CdcConfig:
    """Where the broker is. Unset means change capture is not configured on this
    deployment: the sensor skips with that reason and runs nothing."""

    bootstrap: str

    @classmethod
    def from_env(cls) -> "CdcConfig":
        return cls(bootstrap=os.environ.get("CDC_KAFKA_BOOTSTRAP", "").strip())


@dataclass(frozen=True)
class ApplyTarget:
    """One topic, the Bronze table it feeds, and how."""

    connector_id: str
    name: str
    topic: str
    table: str
    key: tuple[str, ...]
    mode: str
    group_id: str

    def __post_init__(self) -> None:
        if self.mode not in MODES:
            raise ValueError(f"mode {self.mode!r} is not one of {', '.join(MODES)}")
        if not self.key:
            raise ValueError("a change-capture table needs a key")


def group_of(connector_id: str) -> str:
    """The consumer group that holds a connector's committed offsets."""
    return f"lakehouse-cdc-{connector_id}"


def topic_of(connector_id: str, object_name: str, dial: Mapping[str, Any] | None = None) -> str:
    """The topic `Kafka Connect` writes a table to: `topic.prefix` is `cdc.<id>`
    (`lakehouse_store::cdc::render_connect_config`), then the name Debezium gives the table.

    - PostgreSQL: `<schema>.<table>`. MySQL: `<database>.<table>` (the source object's own name).
      Both are `cdc.<id>.<object name>`.
    - SQL Server (`dial.driver == "mssql"`): `<database>.<schema>.<table>`, the database being
      the dial's, because the object name is `<schema>.<table>` (`SRC-4` decision S3-c).

    The schema-history topic of MySQL and SQL Server (`cdc.<id>.schema-history`) is never read:
    only the topics of the connector's source objects are.

    # Errors

    `CdcSetupError` for a SQL Server dial that names no database."""
    if (dial or {}).get("driver") == "mssql":
        database = (dial or {}).get("database")
        if not isinstance(database, str) or not database:
            raise CdcSetupError("a SQL Server change-capture connector has no database in its dial")
        return f"{TOPIC_PREFIX}.{connector_id}.{database}.{object_name}"
    return f"{TOPIC_PREFIX}.{connector_id}.{object_name}"


@functools.lru_cache(maxsize=4096)
def _loaded_name(name: str) -> str:
    """`dlt`'s name for a source column, or the name itself when `dlt` has none."""
    return schema_observer.loaded_column_name(name) or name


def targets_of(connector: Mapping[str, Any]) -> list[ApplyTarget]:
    """The tables of a change-capture connector as apply targets.

    # Errors

    `CdcSetupError` for an object with no `primaryKey`, a load mode other than
    `incremental_dedup` and `history`, or no target. Names the object, never a value."""
    connector_id = connector["id"]
    targets: list[ApplyTarget] = []
    for obj in connector.get("sourceObjects") or []:
        name = obj.get("name")
        table = obj.get("target")
        if not isinstance(name, str) or not isinstance(table, str) or not table:
            raise CdcSetupError("a source object has no name or no target")
        load_mode = obj.get("loadMode") or DEFAULT_LOAD_MODE
        mode = APPLY_MODE_OF.get(load_mode)
        if mode is None:
            raise CdcSetupError(f"{name!r}: change capture runs incremental_dedup and history only")
        key = tuple(_loaded_name(column) for column in obj.get("primaryKey") or [])
        if not key:
            raise CdcSetupError(f"{name!r}: change capture needs a primaryKey")
        targets.append(
            ApplyTarget(
                connector_id=connector_id,
                name=name,
                topic=topic_of(connector_id, name, connector.get("dial")),
                table=table,
                key=key,
                mode=mode,
                group_id=group_of(connector_id),
            )
        )
    return targets


# ---------------------------------------------------------------------------
# Events to rows (pure)
# ---------------------------------------------------------------------------


def event_to_row(value: bytes | str | None, offset: int) -> dict[str, Any] | None:
    """One message as a row, or `None` for a tombstone.

    # Errors

    `CdcEventError` for a message that is not a JSON object with an `__op`."""
    if value is None:
        return None
    try:
        payload = json.loads(value)
    except (ValueError, TypeError):
        raise CdcEventError("a change message is not JSON") from None
    if payload is None:
        return None
    if not isinstance(payload, dict) or "__op" not in payload:
        raise CdcEventError("a change message is not a flattened Debezium event with an __op")
    row = {_loaded_name(name): value for name, value in payload.items() if name not in DEBEZIUM_FIELDS}
    row["_op"] = payload["__op"]
    row["_source_ts_ms"] = payload.get("__source_ts_ms")
    row["_deleted"] = payload.get("__deleted") in (True, "true")
    row["_offset"] = offset
    return row


def last_per_key(rows: Iterable[dict[str, Any]], key: Sequence[str]) -> dict[tuple[Any, ...], dict[str, Any]]:
    """The last row of each key, in the order the keys were first seen."""
    last: dict[tuple[Any, ...], dict[str, Any]] = {}
    for row in rows:
        last[tuple(row[column] for column in key)] = row
    return last


def split_current(
    rows: Iterable[dict[str, Any]], key: Sequence[str]
) -> tuple[list[dict[str, Any]], list[tuple[Any, ...]]]:
    """For the current-row mode: the rows to write (the last version of each key whose
    last event is not a delete, without the event columns except `_source_ts_ms`) and
    every key the batch touched (to delete first)."""
    last = last_per_key(rows, key)
    survivors = [
        {column: value for column, value in row.items() if column not in ("_op", "_deleted", "_offset")}
        for row in last.values()
        if not row["_deleted"]
    ]
    return survivors, list(last)


def batch_columns(rows: Iterable[Mapping[str, Any]]) -> dict[str, pa.DataType]:
    """The source columns of `rows` with the Arrow type their values give (a column null
    in every row is left out, as `dlt` leaves it out)."""
    plain = ({k: v for k, v in row.items() if k not in OWN_COLUMNS} for row in rows)
    return history.column_types(plain)


# ---------------------------------------------------------------------------
# Reading a batch
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class Batch:
    rows: list[dict[str, Any]]
    # (topic partition) -> the next offset to consume (last seen + 1), kafka-python's convention.
    next_offsets: dict[Any, int]


def read_batch(consumer: Any, *, max_events: int = BATCH_EVENTS, poll_ms: int = POLL_MS) -> Batch:
    """Read up to `max_events` messages; an empty poll ends the batch (caught up).
    Commits nothing.

    The broker is this deployment's own (module docs, "Production differences"), so no
    `ssrf_guard` is applied here.

    # Errors

    `CdcEventError` for a bad message."""
    rows: list[dict[str, Any]] = []
    next_offsets: dict[Any, int] = {}
    while len(rows) < max_events:
        polled = consumer.poll(timeout_ms=poll_ms, max_records=min(500, max_events - len(rows)))
        if not polled:
            break
        for tp, records in polled.items():
            for record in records:
                next_offsets[tp] = max(next_offsets.get(tp, 0), record.offset + 1)
                row = event_to_row(record.value, record.offset)
                if row is not None:
                    rows.append(row)
    return Batch(rows=rows, next_offsets=next_offsets)


# ---------------------------------------------------------------------------
# Writing to Bronze
# ---------------------------------------------------------------------------


def _coerce(value: Any, field: pa.Field) -> Any:
    """A timestamp column arrives from Debezium as an ISO string (`ZonedTimestamp`);
    `dlt` typed the column as a timestamp at creation, so the string becomes one."""
    if isinstance(value, str) and pa.types.is_timestamp(field.type):
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
        return parsed if parsed.tzinfo else parsed.replace(tzinfo=timezone.utc)
    return value


def _key_filter(key: Sequence[str], touched: Sequence[tuple[Any, ...]]) -> BooleanExpression:
    """The rows whose key is one of `touched`: one `In` for a one-column key, an `Or`
    of `And`s of equalities for a composite one."""
    if len(key) == 1:
        return In(key[0], [k[0] for k in touched])
    clauses = [And(*[EqualTo(column, value) for column, value in zip(key, values, strict=True)]) for values in touched]
    return Or(*clauses) if len(clauses) > 1 else clauses[0]


def add_columns(iceberg_table: Any, new_types: Mapping[str, pa.DataType]) -> None:
    """Add `new_types` to the table's schema (one metadata commit), keeping every
    column it has. Used for a column the API has agreed to load."""
    if not new_types:
        return
    merged = pa.schema([*iceberg_table.schema().as_arrow(), *[pa.field(n, t) for n, t in new_types.items()]])
    with iceberg_table.update_schema() as update:
        update.union_by_name(merged)


def apply_current(
    iceberg_table: Any,
    rows: list[dict[str, Any]],
    key: Sequence[str],
    *,
    table: str,
    allow_new_columns: bool = False,
    now: Callable[[], datetime] = lambda: datetime.now(timezone.utc),
    author: str = "",
) -> int:
    """Apply one batch of events to an existing Bronze table in ONE transaction: delete
    every key the batch touched, append the surviving last versions. Returns the number
    of rows appended.

    # Errors

    `CdcSchemaChange` for a column the table lacks when `allow_new_columns` is false
    (before anything is written); `history.HistoryRefused` for a value that does not fit
    its column; whatever the catalog raises. A failure commits nothing (a column added
    first stays: it is metadata, and the API had agreed to it)."""
    survivors, touched = split_current(rows, key)
    known = {field.name for field in iceberg_table.schema().as_arrow()}
    extra = {name for row in survivors for name in row} - known
    if extra:
        if not allow_new_columns:
            raise CdcSchemaChange(
                f"table {table!r}: a change brings a column the table lacks and nothing allowed adding it"
            )
        add_columns(iceberg_table, {n: t for n, t in batch_columns(survivors).items() if n in extra})
        iceberg_table = iceberg_table.refresh()
    arrow_schema = iceberg_table.schema().as_arrow()
    stamp = now()
    load_id = f"{stamp.timestamp():.6f}"
    prepared = [
        {
            **{name: _coerce(value, arrow_schema.field(name)) for name, value in row.items() if name in arrow_schema.names},
            "_ingested_at": stamp,
            "_dlt_load_id": load_id,
            "_dlt_id": uniq_id_base64(10),
        }
        for row in survivors
    ]
    new_arrow = history.to_arrow(prepared, arrow_schema, table=table) if prepared else None
    props = history.snapshot_properties(author)
    with iceberg_table.transaction() as transaction:
        with warnings.catch_warnings():
            # pyiceberg warns when a delete matches nothing (a batch of fresh inserts).
            warnings.filterwarnings("ignore", message="Delete operation did not match any records")
            transaction.delete(delete_filter=_key_filter(key, touched), snapshot_properties=props)
        if new_arrow is not None:
            transaction.append(new_arrow, snapshot_properties=props)
    return len(prepared)


class Bronze(Protocol):
    """What the applier needs of a Bronze table; the real one is `IcebergBronze`."""

    def columns(self) -> dict[str, str] | None:
        """The table's source columns (name to type name), or `None` before it exists."""

    def write(self, rows: list[dict[str, Any]], *, allow_new_columns: bool) -> None:
        """Apply one batch (events as rows) in the target's mode."""


class IcebergBronze:
    """The real Bronze table of one target, through Lakekeeper (`adapters/sink.py`)."""

    def __init__(self, target: ApplyTarget, config: sink_adapter.SinkConfig, *, author: str = "") -> None:
        self._target = target
        self._config = config
        self._author = author

    def _open(self) -> Any | None:
        return sink_adapter.open_bronze_table(self._target.table, self._config)

    def columns(self) -> dict[str, str] | None:
        table = self._open()
        if table is None:
            return None
        return {f.name: str(f.type) for f in table.schema().as_arrow() if f.name not in OWN_COLUMNS}

    def write(self, rows: list[dict[str, Any]], *, allow_new_columns: bool) -> None:
        target = self._target
        if target.mode == "history":
            plan = sink_adapter.LoadPlan(mode="append")
            sink_adapter.load_via_sink(rows, target.table, self._config, plan)
            return
        table = self._open()
        if table is None:
            # First batch: `dlt` makes the table. A replay after a crash finds it and takes the
            # transaction below (delete then append), so it adds no copy.
            survivors, _ = split_current(rows, target.key)
            if survivors:
                sink_adapter.load_via_sink(survivors, target.table, self._config, sink_adapter.LoadPlan(mode="append"))
            return
        apply_current(
            table, rows, target.key, table=target.table, allow_new_columns=allow_new_columns, author=self._author
        )


# ---------------------------------------------------------------------------
# One table
# ---------------------------------------------------------------------------

# How a table's pass ended.
CAUGHT_UP = "caught_up"
OUT_OF_TIME = "out_of_time"
WAITING = "waiting"


@dataclass(frozen=True)
class TableResult:
    outcome: str
    events: int


Observe = Callable[[Sequence[schema_observer.ReflectedColumn], str], schema_observer.Decision]


def _reflected(columns: Mapping[str, str]) -> list[schema_observer.ReflectedColumn]:
    return [schema_observer.ReflectedColumn(name, type_name, True, loaded_name=name) for name, type_name in columns.items()]


def _filter_columns(rows: list[dict[str, Any]], keep: Sequence[str]) -> list[dict[str, Any]]:
    """Rows with only the columns the API named (and the event and system columns)."""
    allowed = set(keep) | set(OWN_COLUMNS)
    return [{name: value for name, value in row.items() if name in allowed} for row in rows]


def last_source_ts_ms(rows: Iterable[Mapping[str, Any]]) -> int | None:
    """The source commit time (ms) of the newest change in `rows`: the largest `_source_ts_ms`,
    or `None` when no row carries one. Never a guess: a batch of tombstones has none."""
    times = [ts for ts in (row.get("_source_ts_ms") for row in rows) if isinstance(ts, int) and not isinstance(ts, bool)]
    return max(times) if times else None


def apply_table(
    consumer: Any,
    target: ApplyTarget,
    bronze: Bronze,
    observe: Observe,
    *,
    commit_offsets: Callable[[dict[Any, int]], None],
    deadline: float,
    max_events: int = BATCH_EVENTS,
    poll_ms: int = POLL_MS,
    on_batch: Callable[[int, float], None] = lambda events, seconds: None,
    on_applied: Callable[[int], None] = lambda source_ts_ms: None,
) -> TableResult:
    """Apply one table's topic batch by batch until it is caught up, `deadline`
    (a `time.monotonic()` value) passes, or the API asks to wait.

    One batch: read, ask the API about new columns, write to Bronze, THEN commit the
    offsets. A write that raises leaves the offsets where they were, so the batch is read
    again (a replay is harmless). A batch of tombstones only commits.

    # Errors

    `CdcEventError` for a bad message; `CdcSchemaChange`, `history.HistoryRefused`,
    `schema_observer.ObservationRefused` and `ObservationUnreachable`; whatever the
    sink and the catalog raise.

    `on_applied` is called with the batch's newest source commit time after its offsets
    are committed (for the lag the API shows); it must not raise."""
    total = 0
    while time.monotonic() < deadline:
        started = time.monotonic()
        batch = read_batch(consumer, max_events=max_events, poll_ms=poll_ms)
        if not batch.next_offsets:
            return TableResult(CAUGHT_UP, total)
        rows = batch.rows
        written = 0
        if rows:
            existing = bronze.columns()
            allow_new = False
            if existing is not None:
                new_types = {n: t for n, t in batch_columns(rows).items() if n not in existing}
                if new_types:
                    # Before anything is written: the API decides what a new column means.
                    observed = [*_reflected(existing), *_reflected({n: str(t) for n, t in new_types.items()})]
                    decision = observe(observed, "before_load")
                    if decision.action == "wait":
                        return TableResult(WAITING, total)
                    if decision.columns is not None:
                        rows = _filter_columns(rows, [*existing, *decision.columns])
                    allow_new = True
            bronze.write(rows, allow_new_columns=allow_new)
            written = len(rows)
            if existing is None:
                # The table now exists: tell the API what it is (the baseline later changes
                # are compared against). Advisory, the rows are in Bronze (`after_load`).
                created = bronze.columns() or {}
                try:
                    observe(_reflected(created), "after_load")
                except (schema_observer.ObservationRefused, schema_observer.ObservationUnreachable) as exc:
                    logger.warning("%r loaded, but its schema could not be reported to the API: %s", target.name, exc)
        # Reached only when the write above returned: the Bronze commit comes first.
        commit_offsets(batch.next_offsets)
        changed_at = last_source_ts_ms(batch.rows)
        if changed_at is not None:
            on_applied(changed_at)
        total += written
        on_batch(written, time.monotonic() - started)
    return TableResult(OUT_OF_TIME, total)


# ---------------------------------------------------------------------------
# Dagster: the job and the op
# ---------------------------------------------------------------------------


def _consumer(config: CdcConfig, group_id: str) -> Any:
    from kafka import KafkaConsumer

    return KafkaConsumer(
        bootstrap_servers=config.bootstrap,
        group_id=group_id,
        enable_auto_commit=False,
        auto_offset_reset="earliest",
        # A consumer must never create a topic by asking for it: a topic exists because
        # Debezium made it for a table, or it does not exist yet.
        allow_auto_create_topics=False,
        value_deserializer=lambda v: v,
    )


def _assign(consumer: Any, topic: str) -> list[Any]:
    """Assign every partition of `topic`; none when the topic does not exist yet."""
    from kafka import TopicPartition

    partitions = consumer.partitions_for_topic(topic) or set()
    assigned = [TopicPartition(topic, p) for p in sorted(partitions)]
    if assigned:
        consumer.assign(assigned)
    return assigned


def _committer(consumer: Any) -> Callable[[dict[Any, int]], None]:
    from kafka import OffsetAndMetadata

    def commit(next_offsets: dict[Any, int]) -> None:
        consumer.commit({tp: OffsetAndMetadata(offset) for tp, offset in next_offsets.items()})

    return commit


def _observer(connector_id: str, object_name: str, key: Sequence[str], run_id: str) -> Observe:
    cfg = schema_observer.ObserverConfig.from_env()

    def observe(columns: Sequence[schema_observer.ReflectedColumn], phase: str) -> schema_observer.Decision:
        return schema_observer.post_observation(cfg, connector_id, object_name, columns, key, phase, run_id)

    return observe


def _fail(description: str) -> Failure:
    return Failure(description=description, allow_retries=False)


def run_connector(
    context: Any,
    connector: Mapping[str, Any],
    reporter: run_reports.RunReporter,
    *,
    config: CdcConfig,
    run_seconds: int = RUN_SECONDS,
) -> str:
    """Apply every table of `connector`; the outcome of the run for the end report:
    `succeeded` or `paused` (a schema decision is pending)."""
    connector_id = connector["id"]
    try:
        targets = targets_of(connector)
    except CdcSetupError as exc:
        reporter.end("failed", "other")
        raise _fail(f"change capture of {connector_id!r}: {exc}") from exc
    sink_config = sink_adapter.SinkConfig.from_bronze_ingest_config(
        dlt_pipeline.BronzeIngestConfig.from_env(),
        author=f"connector {connector_id}, run {str(context.run_id)[:8]}",
    )
    deadline = time.monotonic() + run_seconds
    observer_cfg = schema_observer.ObserverConfig.from_env()
    consumer = _consumer(config, group_of(connector_id))
    attempt = _retry_number(context) + 1
    try:
        for target in targets:
            started_at = run_reports.now()
            if not _assign(consumer, target.topic):
                # Debezium has not made the topic yet (no snapshot yet): nothing to apply.
                continue
            try:
                result = apply_table(
                    consumer,
                    target,
                    IcebergBronze(target, sink_config, author=sink_config.author),
                    _observer(connector_id, target.name, target.key, context.run_id),
                    commit_offsets=_committer(consumer),
                    deadline=deadline,
                    on_batch=lambda events, seconds, name=target.name: context.log.info(
                        f"CDC_BATCH target={name} events={events} seconds={seconds:.2f}"
                    ),
                    on_applied=lambda ts_ms, name=target.name: post_capture_observations(
                        observer_cfg, connector_id, [{"table": name, "lastSourceTsMs": ts_ms}]
                    ),
                )
            except (CdcSchemaChange, history.HistoryRefused, schema_observer.ObservationRefused, CdcEventError) as exc:
                # A refusal would be answered the same way again: no retry (slice 0: the
                # default retry policy hid one for 13 minutes). Fixed texts, no row values.
                reporter.attempt(
                    table=target.name, attempt=attempt, started_at=started_at, ended_at=run_reports.now(),
                    error_class=ingest_errors.classify(exc).error_class,
                )
                reporter.end("failed", "table_failed")
                raise _fail(f"change capture of {target.name!r}: {exc}") from exc
            except Exception as exc:  # noqa: BLE001 -- reported, then re-raised for the retry policy
                final = attempt > DEFAULT_RETRY_POLICY.max_retries
                reporter.attempt(
                    table=target.name, attempt=attempt, started_at=started_at, ended_at=run_reports.now(),
                    error_class=ingest_errors.classify(exc).error_class,
                )
                if final:
                    reporter.end("failed", "table_failed")
                raise
            reporter.attempt(
                table=target.name, attempt=attempt, started_at=started_at, ended_at=run_reports.now(),
                rows_loaded=result.events,
            )
            if result.outcome == WAITING:
                reporter.log("schema_decision_pending", table=target.name)
                return "paused"
            if result.outcome == OUT_OF_TIME:
                break
    finally:
        consumer.close()
    return "succeeded"


def _retry_number(context: Any) -> int:
    try:
        return int(context.retry_number)
    except AttributeError:
        return 0


# The default retry policy every op carries (`test_op_source_metadata.py`): a retried batch is
# safe because a replay gives the same table. A refusal raises `Failure(allow_retries=False)`
# above, which the policy does not retry.
@op(
    config_schema={"connector_id": str},
    retry_policy=DEFAULT_RETRY_POLICY,
    tags=source_metadata(
        "dispar_orchestrate/cdc_apply.py::cdc_apply_op",
        reads=["Kafka change topics (one per table)", "GET /api/connectors/ingestible"],
        writes=["Iceberg bronze.* (one table per source object)", "POST /api/connectors/{id}/run-reports"],
    ),
)
def cdc_apply_op(context) -> str:
    connector_id: str = context.op_config["connector_id"]
    reporter = run_reports.RunReporter.for_run(connector_id, context.run_id)
    reporter.start(_trigger_of(context))
    config = CdcConfig.from_env()
    if not config.bootstrap:
        reporter.end("failed", "other")
        raise _fail("change capture is not configured on this deployment (CDC_KAFKA_BOOTSTRAP is unset)")
    connector = _fetch_connector_for_run(IngestFactoryConfig.from_env(), connector_id, _retry_number(context))
    if connector.get("paused") is True:
        context.log.info(f"connector {connector_id!r} is paused; this run applies nothing")
        reporter.end("paused", "connector_paused")
        return "paused"
    if connector.get("adapter") != "cdc":
        reporter.end("failed", "other")
        raise _fail(f"connector {connector_id!r} is not a change-capture connector")
    outcome = run_connector(context, connector, reporter, config=config)
    if outcome == "paused":
        reporter.end("paused", "connector_paused")
    else:
        reporter.end("succeeded", "completed")
    return outcome


def _trigger_of(context: Any) -> str:
    """What started this run (`run_reports.TRIGGERS`): the run's tag. The sensor tags its
    runs `schedule` (the orchestrator's own timer, the closest word of the closed list);
    anything else is reported as `other`, not guessed."""
    try:
        tag = (context.dagster_run.tags or {}).get(TRIGGER_TAG)
    except Exception:  # noqa: BLE001 -- a directly invoked op has no run
        tag = None
    return tag if tag in run_reports.TRIGGERS else "other"


@job
def cdc_apply_job() -> None:
    """Config: `ops.cdc_apply_op.config.connector_id`, a change-capture connector."""
    cdc_apply_op()


# ---------------------------------------------------------------------------
# The sensor
# ---------------------------------------------------------------------------


def fetch_cdc_connectors(cfg: IngestFactoryConfig) -> list[dict[str, Any]]:
    """Every change-capture connector, paused or not, from `GET /api/connectors/ingestible`
    WITHOUT the due window (that list filters `cdc` out: a change-capture connector is
    never due on a schedule).

    # Errors

    Propagates a request failure, a non-2xx answer and a body that is not a list: the
    sensor tick then fails visibly in Dagster and is tried again."""
    resp = requests.get(f"{cfg.api_url}/api/connectors/ingestible", headers=_headers(cfg), timeout=10)
    resp.raise_for_status()
    connectors = resp.json()
    if not isinstance(connectors, list):
        raise RuntimeError("GET /api/connectors/ingestible did not return a list")
    return [c for c in connectors if c.get("adapter") == "cdc"]


def _topic_backlog(consumer: Any, topic: str) -> int | None:
    """Messages on `topic` past the consumer group's committed offsets (every partition), or
    `None` when that could not be measured (never a made-up 0). A topic that does not exist yet
    counts 0."""
    from kafka import TopicPartition

    try:
        total = 0
        for partition in sorted(consumer.partitions_for_topic(topic) or set()):
            tp = TopicPartition(topic, partition)
            committed = consumer.committed(tp)
            if committed is None:
                committed = consumer.beginning_offsets([tp]).get(tp, 0)
            lag = compute_lag(consumer, topic=topic, partition=partition, committed_offset=committed)
            if lag is None:
                return None
            total += lag
        return total
    except Exception:  # noqa: BLE001 -- any broker failure means "not measured"
        return None


def backlog_by_table(
    config: CdcConfig, group_id: str, topics: Mapping[str, str], consumer: Any | None = None
) -> dict[str, int | None]:
    """The backlog of each table (`topics` maps a source object's name to its topic): the number
    of messages the applier has not read, or `None` for a table that could not be measured."""
    owned = consumer is None
    consumer = consumer or _consumer(config, group_id)
    try:
        return {name: _topic_backlog(consumer, topic) for name, topic in topics.items()}
    finally:
        if owned:
            consumer.close()


def total_backlog(backlogs: Mapping[str, int | None]) -> int | None:
    """The sum of the tables' backlogs, or `None` when any of them could not be measured."""
    if any(value is None for value in backlogs.values()):
        return None
    return sum(value for value in backlogs.values() if value is not None)


def _total_of(backlogs: Mapping[str, int | None] | None) -> int | None:
    """`total_backlog`, with `None` for a connector that could not be measured at all."""
    return None if backlogs is None else total_backlog(backlogs)


def unconsumed(config: CdcConfig, group_id: str, topics: Sequence[str], consumer: Any | None = None) -> int | None:
    """Messages on `topics` past the group's committed offsets (every partition), or
    `None` when that could not be measured (never a made-up 0). A topic that does not
    exist yet counts 0."""
    return total_backlog(backlog_by_table(config, group_id, {topic: topic for topic in topics}, consumer))


def post_capture_observations(cfg: schema_observer.ObserverConfig, connector_id: str, tables: list[dict[str, Any]]) -> bool:
    """Tell the API what the orchestrator saw (`tables` is `[{table, backlog?, lastSourceTsMs?}]`).
    Returns whether it was sent and never raises: a report never fails the load, and nothing is
    queued (the API then shows "not known since"). The log names the kind of failure only."""
    if not tables:
        return True
    try:
        schema_observer._post_json(
            cfg, f"/api/connectors/{connector_id}/capture/observations", {"tables": tables}, "capture observation"
        )
    except (schema_observer.ObservationRefused, schema_observer.ObservationUnreachable) as exc:
        # Their messages are fixed: the kind of failure and an HTTP status.
        logger.warning("the capture observation of %s was not sent: %s", connector_id, exc)
        return False
    except Exception as exc:  # noqa: BLE001 -- a report never fails the load, whatever went wrong
        logger.warning("the capture observation of %s was not sent (%s)", connector_id, type(exc).__name__)
        return False
    return True


def topics_of(connector: Mapping[str, Any]) -> dict[str, str]:
    """A connector's tables, each source object's name mapped to its topic.

    # Errors

    `CdcSetupError` for a SQL Server dial that names no database."""
    return {
        o["name"]: topic_of(connector["id"], o["name"], connector.get("dial"))
        for o in connector.get("sourceObjects") or []
        if o.get("name")
    }


def measure_backlogs(
    config: CdcConfig,
    connectors: Sequence[Mapping[str, Any]],
    measure: Callable[[CdcConfig, str, Mapping[str, str]], dict[str, int | None]] = backlog_by_table,
) -> dict[str, dict[str, int | None] | None]:
    """The backlog of every table of every connector, paused or not. A connector whose topics
    cannot even be named (a setup error) maps to `None`: the run reports that error, and the
    sensor treats the connector as unmeasured."""
    measured: dict[str, dict[str, int | None] | None] = {}
    for connector in connectors:
        try:
            topics = topics_of(connector)
        except CdcSetupError as exc:
            logger.warning("the topics of %s could not be named: %s", connector["id"], exc)
            measured[connector["id"]] = None
            continue
        measured[connector["id"]] = measure(config, group_of(connector["id"]), topics)
    return measured


def report_backlogs(
    cfg: schema_observer.ObserverConfig, measured: Mapping[str, Mapping[str, int | None] | None]
) -> None:
    """Post each connector's measured backlogs. A table that was not measured is left out, not
    sent as 0."""
    for connector_id, backlogs in measured.items():
        tables = [{"table": name, "backlog": value} for name, value in (backlogs or {}).items() if value is not None]
        post_capture_observations(cfg, connector_id, tables)


def _busy(instance: Any) -> set[str]:
    """Connectors with a `cdc_apply_job` run still queued or going."""
    busy: set[str] = set()
    for run in instance.get_runs(filters=RunsFilter(job_name=cdc_apply_job.name, statuses=_IN_PROGRESS)):
        config = (run.run_config or {}).get("ops", {}).get("cdc_apply_op", {}).get("config", {})
        if isinstance(config.get("connector_id"), str):
            busy.add(config["connector_id"])
    return busy


def evaluate(
    instance: Any,
    connectors: Sequence[Mapping[str, Any]],
    *,
    lag: Callable[[Mapping[str, Any]], int | None],
    now: Callable[[], int] = time.time_ns,
) -> tuple[list[RunRequest], list[str]]:
    """One `RunRequest` per change-capture connector that is not paused, not busy and has
    changes it has not applied; and the reasons for the others (for the sensor's log).
    A connector whose backlog cannot be measured is skipped with that said."""
    busy = _busy(instance)
    requests_: list[RunRequest] = []
    notes: list[str] = []
    for connector in connectors:
        connector_id = connector["id"]
        if connector.get("paused") is True:
            notes.append(f"{connector_id} is paused")
        elif connector_id in busy:
            notes.append(f"{connector_id} still has a run going")
        else:
            pending = lag(connector)
            if pending is None:
                notes.append(f"the backlog of {connector_id} could not be measured")
            elif pending == 0:
                notes.append(f"{connector_id} has nothing unapplied")
            else:
                requests_.append(
                    RunRequest(
                        run_key=f"{connector_id}-{now()}",
                        run_config={"ops": {"cdc_apply_op": {"config": {"connector_id": connector_id}}}},
                        tags={CONNECTOR_TAG: connector_id, TRIGGER_TAG: "schedule"},
                    )
                )
    return requests_, notes


# default_status=RUNNING: a sensor that ships stopped silently never fires, and change capture
# is a stream, not a schedule a person turns on.
@sensor(
    name="cdc_apply_sensor",
    job=cdc_apply_job,
    minimum_interval_seconds=SENSOR_SECONDS,
    default_status=DefaultSensorStatus.RUNNING,
    description=(
        "Starts cdc_apply_job for every change-capture connector whose topics hold changes not yet "
        "applied to Bronze. Connectors are read from lakehouse-api on every evaluation."
    ),
)
def cdc_apply_sensor(context: SensorEvaluationContext) -> SensorResult:
    config = CdcConfig.from_env()
    if not config.bootstrap:
        return SensorResult(skip_reason="change capture is not configured on this deployment (CDC_KAFKA_BOOTSTRAP is unset)")
    cfg = IngestFactoryConfig.from_env()
    if not cfg.service_token:
        return SensorResult(skip_reason="INGEST_SERVICE_TOKEN is unset, so change-capture connectors cannot be read")
    connectors = fetch_cdc_connectors(cfg)
    # Every connector's backlog is measured and reported, paused, busy or failed ones too: the
    # API needs it for the lag, and a failed connector is what the lag alert is for.
    measured = measure_backlogs(config, connectors)
    report_backlogs(schema_observer.ObserverConfig.from_env(), measured)
    run_requests, notes = evaluate(
        context.instance, connectors, lag=lambda connector: _total_of(measured.get(connector["id"]))
    )
    for note in notes:
        context.log.info(note)
    if not run_requests:
        return SensorResult(skip_reason="no change-capture connector has changes to apply")
    return SensorResult(run_requests=run_requests)
