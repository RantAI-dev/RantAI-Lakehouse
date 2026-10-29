"""Unit tests for `cdc_apply.py` (SRC-4 slice 0 prototype). No network: the consumer
is a fake, the Bronze table is a local `pyiceberg` SQL catalog in a temp directory.

Run with:
~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_cdc_apply.py -q
"""

from __future__ import annotations

import contextlib
import json
from datetime import datetime, timezone
from types import SimpleNamespace

import pyarrow as pa
from kafka import TopicPartition
import pytest
from pyiceberg.catalog.sql import SqlCatalog

from dispar_orchestrate import cdc_apply
from dispar_orchestrate.cdc_apply import (
    ApplyTarget,
    CdcEventError,
    CdcSchemaChange,
    apply_current,
    apply_once,
    event_to_row,
    last_per_key,
    split_current,
)


def _event(op: str, id_: int, customer: str = "c", deleted: str | None = None) -> bytes:
    body = {
        "id": id_,
        "customer": customer,
        "amount": "10.50",
        "status": "new",
        "updated_at": "2026-10-29T10:00:00.000000Z",
        "__op": op,
        "__source_ts_ms": 1_000 + id_,
        "__deleted": deleted if deleted is not None else ("true" if op == "d" else "false"),
    }
    return json.dumps(body).encode()


def test_a_create_update_and_snapshot_event_become_live_rows() -> None:
    for op in ("c", "u", "r"):
        row = event_to_row(_event(op, 7), offset=41)
        assert row is not None
        assert row["__op"] == op and row["__deleted"] is False and row["__offset"] == 41
        assert row["id"] == 7 and row["amount"] == "10.50"


def test_a_delete_event_becomes_a_deleted_row_that_keeps_the_old_values() -> None:
    row = event_to_row(_event("d", 9, customer="old"), offset=3)
    assert row is not None
    assert row["__deleted"] is True and row["customer"] == "old"


def test_a_tombstone_and_a_json_null_carry_nothing() -> None:
    assert event_to_row(None, 5) is None
    assert event_to_row(b"null", 5) is None


def test_a_message_that_is_not_an_event_is_refused_without_quoting_it() -> None:
    for bad in (b"not json", b"[1,2]", b'{"id": 1}'):
        with pytest.raises(CdcEventError) as caught:
            event_to_row(bad, 0)
        assert "id" not in str(caught.value) or "__op" in str(caught.value)
        assert bad.decode() not in str(caught.value)


def test_only_the_last_event_per_key_counts() -> None:
    rows = [event_to_row(_event("c", 1, "a"), 0), event_to_row(_event("u", 1, "b"), 1), event_to_row(_event("c", 2, "x"), 2)]
    last = last_per_key([r for r in rows if r], ("id",))
    assert [(k, v["customer"]) for k, v in last.items()] == [((1,), "b"), ((2,), "x")]


def test_a_key_whose_last_event_is_a_delete_is_touched_but_not_written() -> None:
    rows = [
        event_to_row(_event("c", 1, "a"), 0),
        event_to_row(_event("d", 1, "a"), 1),  # created and deleted in one batch
        event_to_row(_event("d", 2, "z"), 2),
        event_to_row(_event("d", 3, "gone"), 3),
        event_to_row(_event("c", 3, "back"), 4),  # deleted, then created again
    ]
    survivors, touched = split_current([r for r in rows if r], ("id",))
    assert touched == [(1,), (2,), (3,)]
    assert [(r["id"], r["customer"]) for r in survivors] == [(3, "back")]
    assert set(survivors[0]) == {"id", "customer", "amount", "status", "updated_at", "__source_ts_ms"}


# ── apply_once: the offsets are committed only after the Bronze write ────────────────


class _Broker:
    def __init__(self, host: str, port: int) -> None:
        self.host, self.port = host, port


class _FakeConsumer:
    def __init__(self, batches: list[list[tuple[int, bytes | None]]]) -> None:
        self._batches = list(batches)
        self.calls: list[str] = []
        self._client = SimpleNamespace(cluster=SimpleNamespace(brokers=lambda: [_Broker("kafka", 9092)]))

    def poll(self, timeout_ms: int, max_records: int):
        if not self._batches:
            return {}
        records = [SimpleNamespace(offset=o, value=v) for o, v in self._batches.pop(0)]
        return {TopicPartition("topic", 0): records}


def _target(mode: str = "current") -> ApplyTarget:
    return ApplyTarget("t", "topic", "tbl", ("id",), mode, "group")


@contextlib.contextmanager
def _no_resolver():
    yield


def _drive(consumer, write, commit):
    return apply_once(
        consumer,
        _target(),
        write,
        commit_offsets=commit,
        resolve_checked=lambda *a, **k: [],
        checking_resolver=_no_resolver,
    )


def test_the_offsets_are_committed_after_the_write_returns() -> None:
    consumer = _FakeConsumer([[(10, _event("c", 1)), (11, _event("u", 1))]])
    order: list[str] = []
    committed: dict[int, int] = {}

    def write(rows):
        order.append(f"write {len(rows)}")

    def commit(offsets):
        order.append("commit")
        committed.update(offsets)

    assert _drive(consumer, write, commit) == 2
    assert order == ["write 2", "commit"]
    assert committed == {0: 12}  # the NEXT offset to consume


def test_a_failing_write_commits_nothing() -> None:
    consumer = _FakeConsumer([[(0, _event("c", 1))]])
    committed: list[dict[int, int]] = []

    def write(rows):
        raise RuntimeError("bronze is down")

    with pytest.raises(RuntimeError):
        _drive(consumer, write, committed.append)
    assert committed == []


def test_an_empty_topic_writes_and_commits_nothing() -> None:
    consumer = _FakeConsumer([])
    calls: list[str] = []
    assert _drive(consumer, lambda rows: calls.append("w"), lambda o: calls.append("c")) == 0
    assert calls == []


def test_a_batch_of_tombstones_commits_without_a_write() -> None:
    consumer = _FakeConsumer([[(4, None), (5, None)]])
    calls: list[str] = []
    committed: list[dict[int, int]] = []
    assert _drive(consumer, lambda rows: calls.append("w"), committed.append) == 0
    assert calls == [] and committed == [{0: 6}]


# ── apply_current on a real (local) Iceberg table ────────────────────────────────────

UTC = pa.timestamp("us", tz="UTC")
SCHEMA = pa.schema(
    [
        pa.field("id", pa.int64()),
        pa.field("customer", pa.string()),
        pa.field("amount", pa.string()),
        pa.field("status", pa.string()),
        pa.field("updated_at", UTC),
        pa.field("__source_ts_ms", pa.int64()),
        pa.field("_ingested_at", UTC),
        pa.field("_dlt_load_id", pa.string(), nullable=False),
        pa.field("_dlt_id", pa.string(), nullable=False),
    ]
)


@pytest.fixture
def table(tmp_path):
    catalog = SqlCatalog("t", uri=f"sqlite:///{tmp_path}/catalog.db", warehouse=f"file://{tmp_path}/wh")
    catalog.create_namespace("bronze")
    return catalog.create_table("bronze.tbl", schema=SCHEMA)


def _rows(*events: bytes, first_offset: int = 0) -> list[dict]:
    parsed = [event_to_row(e, first_offset + i) for i, e in enumerate(events)]
    return [r for r in parsed if r]


def _state(table) -> dict[int, str]:
    return {r["id"]: r["customer"] for r in table.scan().to_arrow().to_pylist()}


NOW = lambda: datetime(2026, 10, 29, 12, 0, tzinfo=timezone.utc)  # noqa: E731


def test_a_batch_inserts_updates_and_deletes_in_one_commit(table) -> None:
    apply_current(table, _rows(_event("r", 1, "a"), _event("r", 2, "b"), _event("r", 3, "c")), ("id",), table="tbl", now=NOW)
    snapshots_before = len(table.metadata.snapshots)
    batch = _rows(_event("u", 1, "a2"), _event("d", 2, "b"), _event("c", 4, "d"), first_offset=3)
    written = apply_current(table, batch, ("id",), table="tbl", now=NOW)
    table = table.refresh()
    assert written == 2
    assert _state(table) == {1: "a2", 3: "c", 4: "d"}
    # delete + append share one transaction, so one commit of each kind landed: the table
    # is never seen between the two (snapshots grow by the delete and the append of that commit).
    assert len(table.metadata.snapshots) - snapshots_before <= 2


def test_replaying_a_batch_gives_the_same_table(table) -> None:
    batch = _rows(_event("r", 1, "a"), _event("r", 2, "b"), _event("u", 1, "a2"), _event("d", 2, "b"))
    apply_current(table, batch, ("id",), table="tbl", now=NOW)
    first = _state(table.refresh())
    apply_current(table.refresh(), batch, ("id",), table="tbl", now=NOW)  # the crash-and-replay case
    assert _state(table.refresh()) == first == {1: "a2"}


def test_a_key_created_and_deleted_inside_one_batch_leaves_no_row(table) -> None:
    apply_current(table, _rows(_event("c", 9, "x"), _event("d", 9, "x")), ("id",), table="tbl", now=NOW)
    assert _state(table.refresh()) == {}


def test_a_column_the_table_lacks_is_refused_before_anything_is_written(table) -> None:
    row = event_to_row(_event("c", 1), 0)
    assert row is not None
    row["surprise"] = 1
    with pytest.raises(CdcSchemaChange):
        apply_current(table, [row], ("id",), table="tbl", now=NOW)
    assert _state(table.refresh()) == {}


def test_the_sensor_asks_for_a_run_only_when_the_topic_has_unconsumed_messages() -> None:
    class _Instance:
        def __init__(self, active):
            self._active = active

        def get_runs(self, filters, limit):
            return self._active

    target = _target()
    ctx = lambda active: SimpleNamespace(instance=_Instance(active))  # noqa: E731
    skip = list(cdc_apply.evaluate_target(ctx([]), target, lag=lambda t: 0))
    assert len(skip) == 1 and isinstance(skip[0], cdc_apply.SkipReason)
    unmeasured = list(cdc_apply.evaluate_target(ctx([]), target, lag=lambda t: None))
    assert isinstance(unmeasured[0], cdc_apply.SkipReason)
    busy = list(cdc_apply.evaluate_target(ctx(["run"]), target, lag=lambda t: 5))
    assert isinstance(busy[0], cdc_apply.SkipReason)
    go = list(cdc_apply.evaluate_target(ctx([]), target, lag=lambda t: 5))
    assert len(go) == 1 and isinstance(go[0], cdc_apply.RunRequest)
    assert go[0].tags["cdc/target"] == "t"
