"""The change-capture applier, PROTOTYPE (`SRC-4` slice 0, ADR 0016 point 3).

Slice 0 measures whether "Debezium on Kafka Connect, then a Dagster job that
writes the change topic to Bronze" meets the product owner's targets
(`docs/superpowers/plans/2026-10-29-src-4-databases-cdc.md`, slice 0; result in
`docs/plans/SRC-4-RESULT.md`). This module is what runs between the topic and
Bronze in that measurement. It lives on the never-merged branch
`measure/src-4-cdc`; slice 2 rewrites what the numbers support.

# What it does

Reads ONE topic in batches, turns Debezium events into rows, writes them in one
of two modes, and commits the Kafka offsets only after the Bronze commit:

- `current` (one row per key): within one batch only the LAST event per key
  counts. The batch is applied in ONE `pyiceberg` transaction of this module's
  own, as `adapters/history.py` does: delete the batch's keys from the table,
  then append the last version of every key whose last event is not a delete. A
  delete event only deletes. Replaying a batch gives the same table, so a crash
  between the Bronze commit and the offset commit is harmless (at-least-once
  delivery, exactly-once effect). Why not `dlt`'s `hard_delete` on a merge:
  `dlt` 1.30.0's Iceberg merge (`dlt/common/libs/pyiceberg.py`,
  `merge_iceberg_table`) accepts only the `upsert` and `insert-only` strategies
  and has no delete branch (plan Amendment 1).
- `history`: every event is appended through `adapters/sink.py` (`LoadPlan`
  `append`), with its operation (`_op`), the source commit time
  (`_source_ts_ms`), the delete flag (`_deleted`) and the Kafka offset
  (`_offset`, the order of the events of one key).

The table is created by the first batch through `sink.load_via_sink`, so it is
the table every other load mode makes (day partition on `_ingested_at`,
format-version 2). A later batch that brings a column the table lacks is refused
(`CdcSchemaChange`): schema change handling is `SRC-8`'s, not this prototype's.

# Event shape

Debezium's `ExtractNewRecordState` transform on the connector with
`delete.tombstone.handling.mode=rewrite` (Debezium 3.8's name; the older `delete.handling.mode` is ignored there) and `add.fields=op,source.ts_ms`, JSON converter
without schemas: a flat object of the row's columns plus `__op` (`c`, `u`, `d`,
`r`), `__source_ts_ms` and `__deleted` (`"true"`/`"false"`). A delete carries
the old row (the source tables are `REPLICA IDENTITY FULL`). A tombstone (a
message with no value) carries nothing and is skipped. The harness
(`ops/src4/measure_cdc.py`) sets `decimal.handling.mode=string`, so `amount`
arrives as text and keeps its scale.

# Limits of the prototype (stated, not hidden)

One partition per topic, one key column, no schema change, no secret handling
(the broker is plaintext and internal). The Dagster pieces at the end are
registered for the measurement only.
"""

from __future__ import annotations

import json
import time
import warnings
from collections.abc import Callable, Iterable, Sequence
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any

from dagster import (
    DagsterRunStatus,
    DefaultSensorStatus,
    RunRequest,
    RunsFilter,
    SensorEvaluationContext,
    SkipReason,
    job,
    op,
    sensor,
)
from dlt.common.utils import uniq_id_base64
from pyiceberg.expressions import In

from dispar_orchestrate import ssrf_guard
from dispar_orchestrate.adapters import history
from dispar_orchestrate.adapters import sink as sink_adapter
from dispar_orchestrate.adapters.kafka import check_brokers, compute_lag
from dispar_orchestrate.op_metadata import DEFAULT_RETRY_POLICY, source_metadata

MODES = ("current", "history")
# Columns the applier adds or Debezium adds; none is a column of the source table.
DEBEZIUM_FIELDS = ("__op", "__source_ts_ms", "__deleted")
EVENT_COLUMNS = ("_op", "_source_ts_ms", "_deleted", "_offset")
BATCH_EVENTS = 10_000
POLL_MS = 1_000
# The longest one run keeps applying batches before it ends; the sensor starts the
# next one. Bounds the run, not the work: a backlog just takes several runs.
RUN_SECONDS = 1_200


class CdcEventError(ValueError):
    """A message is not a Debezium event in the shape this applier reads. The text is
    fixed: it never carries the message (it can hold row values)."""


class CdcSchemaChange(RuntimeError):
    """A batch carries a column the Bronze table lacks (fixed text, names no value)."""


@dataclass(frozen=True)
class ApplyTarget:
    """One topic, the Bronze table it feeds, and how."""

    name: str
    topic: str
    table: str
    key: tuple[str, ...]
    mode: str
    group_id: str

    def __post_init__(self) -> None:
        if self.mode not in MODES:
            raise ValueError(f"mode {self.mode!r} is not one of {', '.join(MODES)}")
        if self.mode == "current" and len(self.key) != 1:
            raise ValueError("this prototype keys the current-row mode on exactly one column")


# The two targets the measurement runs: one source table per load mode, so each
# mode starts from a clean source (`ops/src4/docker-compose.src4.override.yml`). The
# topic is `<topic.prefix>.<schema>.<table>`; the prefix `s4` is set by the harness.
TARGETS: tuple[ApplyTarget, ...] = (
    ApplyTarget("current", "s4.src4.orders", "src4_orders_current", ("id",), "current", "src4-apply-current"),
    ApplyTarget("history", "s4.src4.orders_history", "src4_orders_history", ("id",), "history", "src4-apply-history"),
)


def target_named(name: str) -> ApplyTarget:
    for target in TARGETS:
        if target.name == name:
            return target
    raise ValueError(f"no apply target named {name!r}")


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
    # The columns the applier adds are named the way `dlt` names them (a leading double
    # underscore becomes one: measured on the first CI run, a table made by `dlt` had
    # `_source_ts_ms`, and a row carrying `__source_ts_ms` was refused as a new column).
    row = {name: value for name, value in payload.items() if name not in DEBEZIUM_FIELDS}
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
    """For the current-row mode: the rows to write (the last version of each key
    whose last event is not a delete, without the event columns except
    `__source_ts_ms`) and every key the batch touched (to delete first)."""
    last = last_per_key(rows, key)
    survivors = [
        {column: value for column, value in row.items() if column not in ("_op", "_deleted", "_offset")}
        for row in last.values()
        if not row["_deleted"]
    ]
    return survivors, list(last)


# ---------------------------------------------------------------------------
# Reading a batch
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class Batch:
    rows: list[dict[str, Any]]
    # partition -> the next offset to consume (last seen + 1), kafka-python's convention.
    next_offsets: dict[int, int]


def read_batch(
    consumer: Any,
    *,
    max_events: int = BATCH_EVENTS,
    poll_ms: int = POLL_MS,
    resolve_checked=ssrf_guard.resolve_checked,
    checking_resolver=ssrf_guard.checking_resolver,
) -> Batch:
    """Read up to `max_events` messages; an empty poll ends the batch (caught up).
    Checks the advertised brokers first and keeps the checking resolver installed for
    the whole read, as `adapters/kafka.py::consume_one_batch` does. Commits nothing.

    # Errors

    `ssrf_guard.SsrfBlocked` for a blocked broker; `CdcEventError` for a bad message."""
    rows: list[dict[str, Any]] = []
    next_offsets: dict[int, int] = {}
    with checking_resolver():
        check_brokers(consumer, resolve_checked=resolve_checked)
        while len(rows) < max_events:
            polled = consumer.poll(timeout_ms=poll_ms, max_records=min(500, max_events - len(rows)))
            if not polled:
                break
            for tp, records in polled.items():
                for record in records:
                    next_offsets[tp.partition] = max(next_offsets.get(tp.partition, 0), record.offset + 1)
                    row = event_to_row(record.value, record.offset)
                    if row is not None:
                        rows.append(row)
    return Batch(rows=rows, next_offsets=next_offsets)


def apply_once(
    consumer: Any,
    target: ApplyTarget,
    write: Callable[[list[dict[str, Any]]], None],
    *,
    commit_offsets: Callable[[dict[int, int]], None],
    max_events: int = BATCH_EVENTS,
    poll_ms: int = POLL_MS,
    resolve_checked=ssrf_guard.resolve_checked,
    checking_resolver=ssrf_guard.checking_resolver,
) -> int:
    """One batch: read, write to Bronze, THEN commit the offsets. A write that raises
    leaves the offsets where they were, so the batch is read again (a replay is
    harmless, see the module docstring). Returns the number of events written; 0 when
    the topic held nothing new (nothing is written or committed)."""
    batch = read_batch(
        consumer, max_events=max_events, poll_ms=poll_ms, resolve_checked=resolve_checked, checking_resolver=checking_resolver
    )
    if not batch.next_offsets:
        return 0
    if batch.rows:
        write(batch.rows)
    # Reached only when the write above returned: the Bronze commit comes first.
    commit_offsets(batch.next_offsets)
    return len(batch.rows)


# ---------------------------------------------------------------------------
# Writing to Bronze
# ---------------------------------------------------------------------------


def _coerce(value: Any, field: Any) -> Any:
    """A timestamp column arrives from Debezium as an ISO string (`ZonedTimestamp`);
    `dlt` typed the column as a timestamp at creation, so the string becomes one."""
    if isinstance(value, str) and "timestamp" in str(field.type):
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
        return parsed if parsed.tzinfo else parsed.replace(tzinfo=timezone.utc)
    return value


def apply_current(
    iceberg_table: Any,
    rows: list[dict[str, Any]],
    key: Sequence[str],
    *,
    table: str,
    now: Callable[[], datetime] = lambda: datetime.now(timezone.utc),
    author: str = "",
) -> int:
    """Apply one batch of events to an existing Bronze table in ONE transaction:
    delete every key the batch touched, append the surviving last versions. Returns
    the number of rows appended.

    # Errors

    `CdcSchemaChange` for a column the table lacks (before anything is written);
    `history.HistoryRefused` for a value that does not fit its column; whatever the
    catalog raises. A failure commits nothing."""
    survivors, touched = split_current(rows, key)
    column = key[0]
    arrow_schema = iceberg_table.schema().as_arrow()
    known = {field.name for field in arrow_schema}
    extra = sorted({name for row in survivors for name in row} - known)
    if extra:
        raise CdcSchemaChange(f"table {table!r}: a change brings a column the table lacks; schema change is not handled here")
    stamp = now()
    load_id = f"{stamp.timestamp():.6f}"
    prepared = [
        {
            **{name: _coerce(value, arrow_schema.field(name)) for name, value in row.items()},
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
            transaction.delete(delete_filter=In(column, [k[0] for k in touched]), snapshot_properties=props)
        if new_arrow is not None:
            transaction.append(new_arrow, snapshot_properties=props)
    return len(prepared)


def bronze_writer(target: ApplyTarget, *, author: str = "") -> Callable[[list[dict[str, Any]]], None]:
    """The function `apply_once` calls with a batch, bound to the real Lakekeeper sink.

    Goes round `sink`'s public entry point on purpose for the `current` mode after the
    first batch: `sink.load_via_sink` has no mode that deletes (plan Amendment 1), so
    this reaches `_install_catalog_env` and `_bronze_pipeline`, the two helpers that
    address a Bronze table, directly. Slice 2 moves this into `sink.py` if the
    measurement holds."""
    from dispar_orchestrate import dlt_pipeline  # imported late: reads the environment only when called

    config = sink_adapter.SinkConfig.from_bronze_ingest_config(dlt_pipeline.BronzeIngestConfig.from_env(), author=author)

    def write(rows: list[dict[str, Any]]) -> None:
        if target.mode == "history":
            sink_adapter.load_via_sink(rows, target.table, config, sink_adapter.LoadPlan(mode="append"))
            return
        sink_adapter._install_catalog_env(config)  # noqa: SLF001 -- see the docstring
        pipeline = sink_adapter._bronze_pipeline(target.table, config)  # noqa: SLF001
        iceberg_table = history.open_table(pipeline, target.table)
        if iceberg_table is None:
            # First batch: `dlt` makes the table. A replay after a crash finds it and takes the
            # transaction below (delete then append), so it adds no copy.
            survivors, _ = split_current(rows, target.key)
            if survivors:
                sink_adapter.load_via_sink(survivors, target.table, config, sink_adapter.LoadPlan(mode="append"))
            return
        apply_current(iceberg_table, rows, target.key, table=target.table, author=author)

    return write


# ---------------------------------------------------------------------------
# Dagster: the job and the sensor (measurement only)
# ---------------------------------------------------------------------------


def _consumer(target: ApplyTarget) -> Any:
    import os

    from kafka import KafkaConsumer

    return KafkaConsumer(
        bootstrap_servers=os.environ["SRC4_KAFKA_BOOTSTRAP"],
        group_id=target.group_id,
        enable_auto_commit=False,
        auto_offset_reset="earliest",
        value_deserializer=lambda v: v,
    )


def run_target(context: Any, target: ApplyTarget, *, run_seconds: int = RUN_SECONDS) -> int:
    """Apply batches until the topic is drained or `run_seconds` pass. Logs one
    `CDC_BATCH` line per batch (the harness reads them from the run's log for the
    batch run times, a measurement and not a metric the product exposes)."""
    from kafka import OffsetAndMetadata, TopicPartition

    consumer = _consumer(target)
    consumer.subscribe([target.topic])
    write = bronze_writer(target, author=f"cdc apply {target.name}")

    def commit(next_offsets: dict[int, int]) -> None:
        consumer.commit({TopicPartition(target.topic, p): OffsetAndMetadata(o) for p, o in next_offsets.items()})

    total = 0
    deadline = time.monotonic() + run_seconds
    try:
        while time.monotonic() < deadline:
            started = time.monotonic()
            written = apply_once(consumer, target, write, commit_offsets=commit)
            if written == 0:
                break
            total += written
            context.log.info(f"CDC_BATCH target={target.name} events={written} seconds={time.monotonic() - started:.2f}")
    finally:
        consumer.close()
    context.log.info(f"CDC_APPLY_DONE target={target.name} events={total}")
    return total


# The default retry policy every op carries (`test_op_source_metadata.py`); a retried
# batch is safe because a replay gives the same table.
@op(
    config_schema={"target": str},
    retry_policy=DEFAULT_RETRY_POLICY,
    tags=source_metadata(
        "dispar_orchestrate/cdc_apply.py::cdc_apply_op",
        reads=["Kafka change topic (SRC-4 slice 0)"],
        writes=["Iceberg bronze.* (one table per target)"],
    ),
)
def cdc_apply_op(context) -> int:
    return run_target(context, target_named(context.op_config["target"]))


@job
def cdc_apply_job() -> None:
    """Config: `ops.cdc_apply_op.config.target` is a `TARGETS` name (its topic, table,
    key and mode are fixed there for the measurement)."""
    cdc_apply_op()


_ACTIVE = [
    DagsterRunStatus.QUEUED,
    DagsterRunStatus.NOT_STARTED,
    DagsterRunStatus.STARTING,
    DagsterRunStatus.STARTED,
]


def unconsumed(target: ApplyTarget, consumer: Any | None = None) -> int | None:
    """Messages on the topic past the group's committed offset (partition 0), or
    `None` when that could not be measured (never a made-up 0): the topic may not
    exist yet."""
    from kafka import TopicPartition

    owned = consumer is None
    consumer = consumer or _consumer(target)
    try:
        with ssrf_guard.checking_resolver():
            check_brokers(consumer)
            committed = consumer.committed(TopicPartition(target.topic, 0)) or 0
            return compute_lag(consumer, topic=target.topic, partition=0, committed_offset=committed)
    finally:
        if owned:
            consumer.close()


def evaluate_target(context: SensorEvaluationContext, target: ApplyTarget, lag: Callable[[ApplyTarget], int | None] = unconsumed):
    """The sensor body: one run per target at a time, only when the topic has
    messages the group has not consumed."""
    active = context.instance.get_runs(
        filters=RunsFilter(job_name="cdc_apply_job", tags={"cdc/target": target.name}, statuses=_ACTIVE), limit=1
    )
    if active:
        yield SkipReason(f"a run for {target.name} is still active")
        return
    pending = lag(target)
    if pending is None:
        yield SkipReason(f"the backlog of {target.topic} could not be measured")
        return
    if pending == 0:
        yield SkipReason(f"{target.topic} has nothing unconsumed")
        return
    yield RunRequest(
        run_key=f"{target.name}-{time.time_ns()}",
        run_config={"ops": {"cdc_apply_op": {"config": {"target": target.name}}}},
        tags={"cdc/target": target.name},
    )


def _sensor_for(target: ApplyTarget):
    # default_status=RUNNING: a sensor that ships stopped silently never fires. The
    # harness stops one by name over GraphQL for the "stop and catch up" step.
    @sensor(
        name=f"cdc_apply_sensor_{target.name}",
        job=cdc_apply_job,
        minimum_interval_seconds=30,
        default_status=DefaultSensorStatus.RUNNING,
    )
    def cdc_apply_sensor(context: SensorEvaluationContext):
        yield from evaluate_target(context, target)

    return cdc_apply_sensor


cdc_apply_sensors = [_sensor_for(target) for target in TARGETS]
