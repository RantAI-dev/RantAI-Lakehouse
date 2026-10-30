"""Unit tests for `cdc_apply.py` (SRC-4 slice 2). No network: the consumer is a fake,
the API and Lakekeeper are fakes, the Bronze table of the transaction tests is a local
`pyiceberg` SQL catalog in a temp directory.

Run with:
~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_cdc_apply.py -q
"""

from __future__ import annotations

import json
import time
from datetime import datetime, timezone
from types import SimpleNamespace
from typing import Any

import pyarrow as pa
import pytest
from dagster import Failure
from kafka import TopicPartition
from pyiceberg.catalog.sql import SqlCatalog

from dispar_orchestrate import cdc_apply, schema_observer
from dispar_orchestrate.adapters import history
from dispar_orchestrate.cdc_apply import (
    ApplyTarget,
    CdcConfig,
    CdcEventError,
    CdcSchemaChange,
    CdcSetupError,
    apply_current,
    apply_table,
    event_to_row,
    last_per_key,
    split_current,
    targets_of,
)

TP = TopicPartition("cdc.conn-a.public.orders", 0)


def _event(op: str, id_: int, customer: str = "c", deleted: str | None = None, **extra: Any) -> bytes:
    body = {
        "id": id_,
        "customer": customer,
        "amount": "10.50",
        "status": "new",
        "updated_at": "2026-10-29T10:00:00.000000Z",
        "__op": op,
        "__source_ts_ms": 1_000 + id_,
        "__deleted": deleted if deleted is not None else ("true" if op == "d" else "false"),
        **extra,
    }
    return json.dumps(body).encode()


# ── events to rows ─────────────────────────────────────────────────────────────────────


def test_a_create_update_and_snapshot_event_become_live_rows() -> None:
    for op in ("c", "u", "r"):
        row = event_to_row(_event(op, 7), offset=41)
        assert row is not None
        assert row["_op"] == op and row["_deleted"] is False and row["_offset"] == 41
        assert row["id"] == 7 and row["amount"] == "10.50"


def test_a_delete_event_becomes_a_deleted_row_that_keeps_the_old_values() -> None:
    row = event_to_row(_event("d", 9, customer="old"), offset=3)
    assert row is not None
    assert row["_deleted"] is True and row["customer"] == "old"


def test_the_columns_the_applier_adds_are_the_names_dlt_gives_them() -> None:
    # Slice 0 (first CI run): dlt turned `__source_ts_ms` into `_source_ts_ms`, so the table
    # lacked the column the row carried. Every name the applier writes must be a fixed point
    # of dlt's naming (SRC-4 amendment 3).
    from dlt.common.normalizers.naming.snake_case import NamingConvention

    naming = NamingConvention()
    row = event_to_row(_event("c", 1), 0)
    assert row is not None
    for name in row:
        assert naming.normalize_identifier(name) == name, name
    for name in cdc_apply.EVENT_COLUMNS:
        assert naming.normalize_identifier(name) == name, name


def test_source_column_names_take_the_name_the_bronze_table_has() -> None:
    row = event_to_row(_event("c", 1, OrderDate="x"), 0)
    assert row is not None and "order_date" in row and "OrderDate" not in row


def test_a_tombstone_and_a_json_null_carry_nothing() -> None:
    assert event_to_row(None, 5) is None
    assert event_to_row(b"null", 5) is None


def test_a_message_that_is_not_an_event_is_refused_without_quoting_it() -> None:
    for bad in (b"not json", b"[1,2]", b'{"id": 1}'):
        with pytest.raises(CdcEventError) as caught:
            event_to_row(bad, 0)
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
    assert set(survivors[0]) == {"id", "customer", "amount", "status", "updated_at", "_source_ts_ms"}


# ── the connector's tables become targets ──────────────────────────────────────────────


def _connector(*objects: dict[str, Any], paused: bool = False) -> dict[str, Any]:
    return {"id": "conn-a", "adapter": "cdc", "paused": paused, "sourceObjects": list(objects)}


def _object(name: str = "public.orders", **extra: Any) -> dict[str, Any]:
    return {"name": name, "target": "orders", "primaryKey": ["id"], **extra}


@pytest.mark.parametrize(
    ("dial", "name", "topic"),
    [
        ({"driver": "postgres", "database": "orders"}, "public.orders", "cdc.conn-a.public.orders"),
        ({"driver": "mysql", "database": "shop"}, "shop.orders", "cdc.conn-a.shop.orders"),
        ({"driver": "mssql", "database": "shop"}, "dbo.orders", "cdc.conn-a.shop.dbo.orders"),
        (None, "public.orders", "cdc.conn-a.public.orders"),
    ],
)
def test_the_topic_of_a_table_follows_the_drivers_naming(dial, name, topic) -> None:
    connector = {**_connector(_object(name)), "dial": dial}
    (target,) = targets_of(connector)
    assert target.topic == topic


def test_a_sql_server_dial_without_a_database_is_a_setup_error_not_a_guessed_topic() -> None:
    connector = {**_connector(_object("dbo.orders")), "dial": {"driver": "mssql"}}
    with pytest.raises(CdcSetupError):
        targets_of(connector)


def test_a_table_with_no_mode_is_the_current_row_per_key() -> None:
    (target,) = targets_of(_connector(_object()))
    assert target.topic == "cdc.conn-a.public.orders"
    assert target.mode == "current" and target.key == ("id",) and target.table == "orders"
    assert target.group_id == "lakehouse-cdc-conn-a"


def test_history_is_a_mode_and_a_composite_key_is_kept_in_order() -> None:
    (target,) = targets_of(_connector(_object(loadMode="history", primaryKey=["OrderId", "line"])))
    assert target.mode == "history" and target.key == ("order_id", "line")


def test_a_table_without_a_key_or_with_another_mode_is_a_setup_error_naming_no_value() -> None:
    for bad in (_object(primaryKey=[]), _object(loadMode="replace"), {"name": "public.t"}):
        with pytest.raises(CdcSetupError):
            targets_of(_connector(bad))


# ── a batch: the offsets are committed only after the Bronze write ─────────────────────


class _FakeConsumer:
    def __init__(self, batches: list[list[tuple[int, bytes | None]]]) -> None:
        self._batches = list(batches)

    def poll(self, timeout_ms: int, max_records: int):
        if not self._batches:
            return {}
        records = [SimpleNamespace(offset=o, value=v) for o, v in self._batches.pop(0)]
        return {TP: records}


class _FakeBronze:
    def __init__(self, columns: dict[str, str] | None = None, log: list[str] | None = None, fail: bool = False) -> None:
        self._columns = columns
        self.log = log if log is not None else []
        self.fail = fail
        self.writes: list[tuple[list[dict[str, Any]], bool]] = []

    def columns(self) -> dict[str, str] | None:
        return self._columns

    def write(self, rows: list[dict[str, Any]], *, allow_new_columns: bool) -> None:
        if self.fail:
            raise RuntimeError("bronze is down")
        self.log.append(f"write {len(rows)}")
        self.writes.append((rows, allow_new_columns))
        if self._columns is None:
            self._columns = {"id": "int64", "customer": "string"}


def _target(mode: str = "current") -> ApplyTarget:
    return ApplyTarget("conn-a", "public.orders", TP.topic, "orders", ("id",), mode, "lakehouse-cdc-conn-a")


FOREVER = time.monotonic() + 3600
KNOWN = {"id": "int64", "customer": "string", "amount": "string", "status": "string", "updated_at": "timestamp[us, tz=UTC]"}


def _decision(action: str = "load", columns: list[str] | None = None) -> schema_observer.Decision:
    return schema_observer.Decision(action=action, columns=columns, changes=[])


def _drive(consumer, bronze, observe=None, committed=None, log=None):
    seen = committed if committed is not None else []

    def commit(offsets):
        if log is not None:
            log.append("commit")
        seen.append(offsets)

    return apply_table(
        consumer, _target(), bronze, observe or (lambda columns, phase: _decision()),
        commit_offsets=commit, deadline=FOREVER, poll_ms=0,
    )


def test_the_offsets_are_committed_after_the_write_returns() -> None:
    log: list[str] = []
    committed: list[dict] = []
    consumer = _FakeConsumer([[(10, _event("c", 1)), (11, _event("u", 1))]])
    result = _drive(consumer, _FakeBronze(dict(KNOWN), log), committed=committed, log=log)
    assert result.events == 2 and result.outcome == cdc_apply.CAUGHT_UP
    assert log == ["write 2", "commit"]
    assert committed == [{TP: 12}]  # the NEXT offset to consume


def test_a_failing_write_commits_nothing() -> None:
    committed: list[dict] = []
    with pytest.raises(RuntimeError):
        _drive(_FakeConsumer([[(0, _event("c", 1))]]), _FakeBronze(dict(KNOWN), fail=True), committed=committed)
    assert committed == []


def test_an_empty_topic_writes_and_commits_nothing() -> None:
    bronze = _FakeBronze(dict(KNOWN))
    committed: list[dict] = []
    assert _drive(_FakeConsumer([]), bronze, committed=committed).events == 0
    assert bronze.writes == [] and committed == []


def test_a_batch_of_tombstones_commits_without_a_write() -> None:
    bronze = _FakeBronze(dict(KNOWN))
    committed: list[dict] = []
    assert _drive(_FakeConsumer([[(4, None), (5, None)]]), bronze, committed=committed).events == 0
    assert bronze.writes == [] and committed == [{TP: 6}]


def test_the_first_batch_creates_the_table_and_then_tells_the_api_what_it_is() -> None:
    calls: list[tuple[list[str], str]] = []

    def observe(columns, phase):
        calls.append(([c.name for c in columns], phase))
        return _decision()

    bronze = _FakeBronze(None)
    _drive(_FakeConsumer([[(0, _event("c", 1))]]), bronze, observe=observe)
    assert calls == [(["id", "customer"], "after_load")]


def test_a_batch_that_brings_no_new_column_is_not_reported() -> None:
    calls: list[str] = []
    _drive(
        _FakeConsumer([[(0, _event("c", 1))]]), _FakeBronze(dict(KNOWN)),
        observe=lambda columns, phase: calls.append(phase) or _decision(),
    )
    assert calls == []


def test_a_new_column_is_reported_before_anything_is_written_and_a_wait_stops_the_table() -> None:
    log: list[str] = []
    committed: list[dict] = []

    def observe(columns, phase):
        log.append(f"observe {phase} {sorted(c.name for c in columns)[-1]}")
        return _decision("wait")

    bronze = _FakeBronze(dict(KNOWN), log)
    result = _drive(_FakeConsumer([[(0, _event("c", 1, surprise=1))]]), bronze, observe=observe, committed=committed)
    assert result.outcome == cdc_apply.WAITING
    assert log == ["observe before_load updated_at"]
    # Nothing written and no offset committed: the same batch is read again after the decision.
    assert bronze.writes == [] and committed == []


def test_a_new_column_the_api_accepts_is_written_with_adding_allowed() -> None:
    bronze = _FakeBronze(dict(KNOWN))
    _drive(_FakeConsumer([[(0, _event("c", 1, surprise=1))]]), bronze)
    (rows, allow_new), = bronze.writes
    assert allow_new is True and rows[0]["surprise"] == 1


def test_columns_the_api_leaves_out_are_dropped_from_the_rows() -> None:
    bronze = _FakeBronze(dict(KNOWN))
    observe = lambda columns, phase: _decision("load", ["wanted"])  # noqa: E731
    _drive(_FakeConsumer([[(0, _event("c", 1, wanted=1, unwanted=2))]]), bronze, observe=observe)
    (rows, _), = bronze.writes
    assert "wanted" in rows[0] and "unwanted" not in rows[0]
    assert rows[0]["id"] == 1 and rows[0]["_op"] == "c"  # source and event columns stay


def test_an_observation_the_api_refuses_is_raised_not_swallowed() -> None:
    def observe(columns, phase):
        raise schema_observer.ObservationRefused("the API refused the schema observation (HTTP 403)")

    with pytest.raises(schema_observer.ObservationRefused):
        _drive(_FakeConsumer([[(0, _event("c", 1, surprise=1))]]), _FakeBronze(dict(KNOWN)), observe=observe)


def test_a_run_out_of_time_stops_between_batches() -> None:
    bronze = _FakeBronze(dict(KNOWN))
    result = apply_table(
        _FakeConsumer([[(0, _event("c", 1))]]), _target(), bronze, lambda c, p: _decision(),
        commit_offsets=lambda o: None, deadline=time.monotonic() - 1, poll_ms=0,
    )
    assert result == cdc_apply.TableResult(cdc_apply.OUT_OF_TIME, 0) and bronze.writes == []


# ── apply_current on a real (local) Iceberg table ──────────────────────────────────────

UTC = pa.timestamp("us", tz="UTC")
SCHEMA = pa.schema(
    [
        pa.field("id", pa.int64()),
        pa.field("line", pa.int64()),
        pa.field("customer", pa.string()),
        pa.field("amount", pa.string()),
        pa.field("status", pa.string()),
        pa.field("updated_at", UTC),
        pa.field("_source_ts_ms", pa.int64()),
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


def _state(table) -> dict[tuple, str]:
    return {(r["id"], r["line"]): r["customer"] for r in table.scan().to_arrow().to_pylist()}


NOW = lambda: datetime(2026, 10, 29, 12, 0, tzinfo=timezone.utc)  # noqa: E731


def _line(event: bytes, line: int = 1) -> bytes:
    body = json.loads(event)
    body["line"] = line
    return json.dumps(body).encode()


def test_a_batch_inserts_updates_and_deletes_in_one_commit(table) -> None:
    apply_current(table, _rows(*(_line(_event("r", i, c)) for i, c in ((1, "a"), (2, "b"), (3, "c")))), ("id", "line"), table="tbl", now=NOW)
    before = len(table.refresh().metadata.snapshots)
    batch = _rows(_line(_event("u", 1, "a2")), _line(_event("d", 2, "b")), _line(_event("c", 4, "d")), first_offset=3)
    written = apply_current(table.refresh(), batch, ("id", "line"), table="tbl", now=NOW)
    table = table.refresh()
    assert written == 2
    assert _state(table) == {(1, 1): "a2", (3, 1): "c", (4, 1): "d"}
    # delete + append share one transaction: the table is never seen between the two.
    assert len(table.metadata.snapshots) - before <= 2


def test_a_composite_key_deletes_only_the_row_with_both_values(table) -> None:
    seed = [_line(_event("r", 1, "l1"), 1), _line(_event("r", 1, "l2"), 2)]
    apply_current(table, _rows(*seed), ("id", "line"), table="tbl", now=NOW)
    apply_current(table.refresh(), _rows(_line(_event("d", 1, "l1"), 1)), ("id", "line"), table="tbl", now=NOW)
    assert _state(table.refresh()) == {(1, 2): "l2"}


def test_replaying_a_batch_gives_the_same_table(table) -> None:
    batch = _rows(*(_line(e) for e in (_event("r", 1, "a"), _event("r", 2, "b"), _event("u", 1, "a2"), _event("d", 2, "b"))))
    apply_current(table, batch, ("id", "line"), table="tbl", now=NOW)
    first = _state(table.refresh())
    apply_current(table.refresh(), batch, ("id", "line"), table="tbl", now=NOW)  # the crash-and-replay case
    assert _state(table.refresh()) == first == {(1, 1): "a2"}


def test_a_key_created_and_deleted_inside_one_batch_leaves_no_row(table) -> None:
    apply_current(table, _rows(_line(_event("c", 9, "x")), _line(_event("d", 9, "x"))), ("id", "line"), table="tbl", now=NOW)
    assert _state(table.refresh()) == {}


def test_a_column_the_table_lacks_is_refused_before_anything_is_written(table) -> None:
    row = event_to_row(_line(_event("c", 1)), 0)
    assert row is not None
    row["surprise"] = 1
    with pytest.raises(CdcSchemaChange):
        apply_current(table, [row], ("id", "line"), table="tbl", now=NOW)
    assert _state(table.refresh()) == {}


def test_a_column_the_api_agreed_to_is_added_and_the_row_is_written(table) -> None:
    row = event_to_row(_line(_event("c", 1, "x", surprise=5)), 0)
    assert row is not None
    apply_current(table, [row], ("id", "line"), table="tbl", allow_new_columns=True, now=NOW)
    refreshed = table.refresh()
    assert "surprise" in refreshed.schema().as_arrow().names
    stored = refreshed.scan().to_arrow().to_pylist()
    assert [(r["id"], r["surprise"]) for r in stored] == [(1, 5)]


def test_a_value_that_does_not_fit_its_column_is_refused_and_commits_nothing(table) -> None:
    apply_current(table, _rows(_line(_event("r", 1, "a"))), ("id", "line"), table="tbl", now=NOW)
    bad = event_to_row(_line(_event("u", 1, "b")), 1)
    assert bad is not None
    bad["id"] = "not a number"
    with pytest.raises(history.HistoryRefused):
        apply_current(table.refresh(), [bad], ("id", "line"), table="tbl", now=NOW)
    assert _state(table.refresh()) == {(1, 1): "a"}


# ── the run: reports, refusals, no retry ───────────────────────────────────────────────


class _Reporter:
    def __init__(self) -> None:
        self.calls: list[tuple[str, tuple, dict]] = []

    def __getattr__(self, name: str):
        def record(*args, **kwargs):
            self.calls.append((name, args, kwargs))
            return True

        return record

    def kinds(self) -> list[str]:
        return [name for name, _, _ in self.calls]


def _context(retry_number: int = 0):
    return SimpleNamespace(run_id="run-1234567890", retry_number=retry_number, log=SimpleNamespace(info=lambda *_: None))


@pytest.fixture
def run_env(monkeypatch):
    """`run_connector` with the broker, Lakekeeper and the table pass faked."""
    monkeypatch.setattr(cdc_apply, "_consumer", lambda config, group: SimpleNamespace(close=lambda: None))
    monkeypatch.setattr(cdc_apply, "_assign", lambda consumer, topic: [TP])
    monkeypatch.setattr(cdc_apply, "_committer", lambda consumer: (lambda offsets: None))
    monkeypatch.setattr(cdc_apply, "_observer", lambda *a, **k: (lambda columns, phase: _decision()))
    monkeypatch.setattr(
        cdc_apply.dlt_pipeline.BronzeIngestConfig,
        "from_env",
        classmethod(lambda cls: SimpleNamespace(
            lakekeeper_catalog_uri="u", lakekeeper_warehouse="w", rustfs_endpoint="e", rustfs_access_key="k",
            rustfs_secret_key="s", warehouse_bucket="b", lakekeeper_token="")),
    )


def test_a_refused_schema_ends_the_run_failed_and_is_not_retried(run_env, monkeypatch) -> None:
    def refuse(*args, **kwargs):
        raise CdcSchemaChange("table 'orders': a change brings a column the table lacks")

    monkeypatch.setattr(cdc_apply, "apply_table", refuse)
    reporter = _Reporter()
    with pytest.raises(Failure) as caught:
        cdc_apply.run_connector(_context(), _connector(_object()), reporter, config=CdcConfig("kafka:9092"))
    assert caught.value.allow_retries is False
    assert reporter.kinds() == ["attempt", "end"]
    assert reporter.calls[-1][1] == ("failed", "table_failed")


def test_an_unexpected_failure_is_reported_and_raised_for_the_retry_policy(run_env, monkeypatch) -> None:
    def boom(*args, **kwargs):
        raise RuntimeError("lakekeeper unreachable")

    monkeypatch.setattr(cdc_apply, "apply_table", boom)
    first = _Reporter()
    with pytest.raises(RuntimeError):
        cdc_apply.run_connector(_context(0), _connector(_object()), first, config=CdcConfig("kafka:9092"))
    assert first.kinds() == ["attempt"]  # a retry is still to come: the run is not ended
    last = _Reporter()
    with pytest.raises(RuntimeError):
        cdc_apply.run_connector(_context(2), _connector(_object()), last, config=CdcConfig("kafka:9092"))
    assert last.kinds() == ["attempt", "end"]  # the policy's last attempt ends the run failed


def test_a_pending_schema_decision_pauses_the_run_and_says_so(run_env, monkeypatch) -> None:
    monkeypatch.setattr(cdc_apply, "apply_table", lambda *a, **k: cdc_apply.TableResult(cdc_apply.WAITING, 0))
    reporter = _Reporter()
    outcome = cdc_apply.run_connector(_context(), _connector(_object()), reporter, config=CdcConfig("kafka:9092"))
    assert outcome == "paused"
    assert reporter.kinds() == ["attempt", "log"]
    assert reporter.calls[-1][1] == ("schema_decision_pending",)


def test_a_table_without_a_topic_yet_is_skipped_not_failed(run_env, monkeypatch) -> None:
    monkeypatch.setattr(cdc_apply, "_assign", lambda consumer, topic: [])
    reporter = _Reporter()
    assert cdc_apply.run_connector(_context(), _connector(_object()), reporter, config=CdcConfig("kafka:9092")) == "succeeded"
    assert reporter.kinds() == []


def test_a_connector_the_applier_cannot_read_fails_without_retry(run_env) -> None:
    reporter = _Reporter()
    with pytest.raises(Failure) as caught:
        cdc_apply.run_connector(_context(), _connector(_object(primaryKey=[])), reporter, config=CdcConfig("kafka:9092"))
    assert caught.value.allow_retries is False and reporter.kinds() == ["end"]


def test_the_op_without_a_broker_address_fails_closed_and_ends_the_run(monkeypatch) -> None:
    from dagster import build_op_context

    reporter = _Reporter()
    monkeypatch.setattr(cdc_apply.run_reports.RunReporter, "for_run", classmethod(lambda cls, c, r: reporter))
    monkeypatch.delenv("CDC_KAFKA_BOOTSTRAP", raising=False)
    with pytest.raises(Failure):
        cdc_apply.cdc_apply_op(build_op_context(op_config={"connector_id": "conn-a"}))
    assert reporter.kinds() == ["start", "end"] and reporter.calls[-1][1] == ("failed", "other")


def test_the_op_loads_nothing_for_a_paused_connector(monkeypatch) -> None:
    from dagster import build_op_context

    reporter = _Reporter()
    monkeypatch.setattr(cdc_apply.run_reports.RunReporter, "for_run", classmethod(lambda cls, c, r: reporter))
    monkeypatch.setenv("CDC_KAFKA_BOOTSTRAP", "kafka:9092")
    monkeypatch.setattr(cdc_apply, "_fetch_connector_for_run", lambda cfg, cid, retry: _connector(paused=True))
    assert cdc_apply.cdc_apply_op(build_op_context(op_config={"connector_id": "conn-a"})) == "paused"
    assert reporter.calls[-1][1] == ("paused", "connector_paused")


def test_the_op_keeps_the_default_retry_policy_and_the_job_is_registered() -> None:
    from dispar_orchestrate.definitions import defs
    from dispar_orchestrate.op_metadata import DEFAULT_RETRY_POLICY

    assert cdc_apply.cdc_apply_op.retry_policy == DEFAULT_RETRY_POLICY
    assert defs.get_job_def("cdc_apply_job") is not None
    assert defs.get_sensor_def("cdc_apply_sensor").minimum_interval_seconds == 30


# ── the sensor ─────────────────────────────────────────────────────────────────────────


class _Instance:
    def __init__(self, busy: list[str] | None = None) -> None:
        self._busy = busy or []

    def get_runs(self, filters, limit=None):
        return [
            SimpleNamespace(run_config={"ops": {"cdc_apply_op": {"config": {"connector_id": cid}}}}) for cid in self._busy
        ]


def test_the_sensor_asks_for_a_run_only_for_a_connector_with_unapplied_changes() -> None:
    requests_, notes = cdc_apply.evaluate(
        _Instance(), [_connector()], lag=lambda c: 5, now=lambda: 7
    )
    assert len(requests_) == 1 and notes == []
    request = requests_[0]
    assert request.run_config == {"ops": {"cdc_apply_op": {"config": {"connector_id": "conn-a"}}}}
    assert request.tags[cdc_apply.CONNECTOR_TAG] == "conn-a" and request.run_key == "conn-a-7"


def test_the_sensor_skips_a_paused_a_busy_a_caught_up_and_an_unmeasured_connector() -> None:
    def one(connector, busy=None, lag=5):
        return cdc_apply.evaluate(_Instance(busy), [connector], lag=lambda c: lag)

    assert one(_connector(paused=True))[0] == []
    assert one(_connector(), busy=["conn-a"])[0] == []
    assert one(_connector(), lag=0)[0] == []
    requests_, notes = one(_connector(), lag=None)
    assert requests_ == [] and "could not be measured" in notes[0]


def test_the_sensor_without_a_broker_address_skips_with_that_reason(monkeypatch) -> None:
    from dagster import build_sensor_context

    monkeypatch.delenv("CDC_KAFKA_BOOTSTRAP", raising=False)
    result = cdc_apply.cdc_apply_sensor(build_sensor_context())
    assert "CDC_KAFKA_BOOTSTRAP" in str(result.skip_reason)


def test_the_backlog_counts_from_the_committed_offset_and_a_missing_topic_is_zero() -> None:
    class _Lag:
        def partitions_for_topic(self, topic):
            return {0} if topic == "present" else None

        def committed(self, tp):
            return 10

        def beginning_offsets(self, tps):
            return {tp: 0 for tp in tps}

        def end_offsets(self, tps):
            return {tp: 25 for tp in tps}

    config = CdcConfig("kafka:9092")
    assert cdc_apply.unconsumed(config, "g", ["present", "absent"], consumer=_Lag()) == 15
    assert cdc_apply.unconsumed(config, "g", ["absent"], consumer=_Lag()) == 0


# ── lag: what the orchestrator tells the API (SRC-4 slice 4) ─────────────────────────────


def test_the_newest_source_time_of_a_batch_is_the_largest_one_and_a_batch_without_one_has_none() -> None:
    assert cdc_apply.last_source_ts_ms([{"_source_ts_ms": 5}, {"_source_ts_ms": 9}, {"_source_ts_ms": 7}]) == 9
    assert cdc_apply.last_source_ts_ms([{"_source_ts_ms": None}, {"id": 1}, {"_source_ts_ms": True}]) is None
    assert cdc_apply.last_source_ts_ms([]) is None


def test_the_applier_reports_the_newest_source_time_only_after_the_offsets_are_committed() -> None:
    log: list[str] = []
    seen: list[int] = []

    def commit(offsets):
        log.append("commit")

    def applied(ts_ms):
        log.append("applied")
        seen.append(ts_ms)

    apply_table(
        _FakeConsumer([[(10, _event("c", 1)), (11, _event("c", 5))]]), _target(), _FakeBronze(dict(KNOWN), log),
        lambda columns, phase: _decision(), commit_offsets=commit, deadline=FOREVER, poll_ms=0, on_applied=applied,
    )
    assert log == ["write 2", "commit", "applied"]
    assert seen == [1_005]  # `__source_ts_ms` is 1_000 + id


def test_a_batch_of_tombstones_reports_no_source_time() -> None:
    seen: list[int] = []
    apply_table(
        _FakeConsumer([[(4, None)]]), _target(), _FakeBronze(dict(KNOWN)), lambda columns, phase: _decision(),
        commit_offsets=lambda offsets: None, deadline=FOREVER, poll_ms=0, on_applied=seen.append,
    )
    assert seen == []


def test_the_run_posts_each_tables_last_applied_change_to_the_capture_route(run_env, monkeypatch) -> None:
    posts: list[tuple[str, list[dict]]] = []
    monkeypatch.setattr(cdc_apply, "post_capture_observations", lambda cfg, cid, tables: posts.append((cid, tables)) or True)

    def fake_apply(consumer, target, bronze, observe, **kwargs):
        kwargs["on_applied"](1_234)
        return cdc_apply.TableResult(cdc_apply.CAUGHT_UP, 1)

    monkeypatch.setattr(cdc_apply, "apply_table", fake_apply)
    cdc_apply.run_connector(_context(), _connector(_object()), _Reporter(), config=CdcConfig("kafka:9092"))
    assert posts == [("conn-a", [{"table": "public.orders", "lastSourceTsMs": 1_234}])]


def _observer_cfg() -> schema_observer.ObserverConfig:
    return schema_observer.ObserverConfig(api_url="http://api", service_token="t")


def test_a_post_that_cannot_be_sent_is_logged_and_never_raises(monkeypatch) -> None:
    tables = [{"table": "public.orders", "backlog": 3}]

    def unreachable(*args, **kwargs):
        raise schema_observer.ObservationUnreachable("the API could not be reached (ConnectionError)")

    monkeypatch.setattr(schema_observer, "_post_json", unreachable)
    assert cdc_apply.post_capture_observations(_observer_cfg(), "conn-a", tables) is False

    def refused(*args, **kwargs):
        raise schema_observer.ObservationRefused("the API refused the capture observation (HTTP 422)")

    monkeypatch.setattr(schema_observer, "_post_json", refused)
    assert cdc_apply.post_capture_observations(_observer_cfg(), "conn-a", tables) is False

    def odd(*args, **kwargs):
        raise RuntimeError("anything else")

    monkeypatch.setattr(schema_observer, "_post_json", odd)
    assert cdc_apply.post_capture_observations(_observer_cfg(), "conn-a", tables) is False


def test_a_post_goes_to_the_capture_route_and_an_empty_one_is_not_sent(monkeypatch) -> None:
    sent: list[tuple[str, dict]] = []
    monkeypatch.setattr(schema_observer, "_post_json", lambda cfg, path, payload, what: sent.append((path, payload)))
    assert cdc_apply.post_capture_observations(_observer_cfg(), "conn-a", []) is True
    assert sent == []
    assert cdc_apply.post_capture_observations(_observer_cfg(), "conn-a", [{"table": "t", "backlog": 0}]) is True
    assert sent == [("/api/connectors/conn-a/capture/observations", {"tables": [{"table": "t", "backlog": 0}]})]


def test_every_connector_is_measured_and_reported_paused_ones_too_and_an_unmeasured_table_is_left_out(monkeypatch) -> None:
    connectors = [
        {**_connector(_object("public.orders"), _object("public.items")), "id": "conn-a"},
        {**_connector(_object("public.orders"), paused=True), "id": "conn-b"},
        {**_connector(_object("dbo.orders")), "id": "conn-c", "dial": {"driver": "mssql"}},
    ]
    asked: list[tuple[str, dict[str, str]]] = []

    def measure(config, group_id, topics, absent=None):
        asked.append((group_id, dict(topics)))
        return {name: (None if name == "public.items" else 7) for name in topics}

    measured = cdc_apply.measure_backlogs(CdcConfig("kafka:9092"), connectors, measure=measure)
    assert [group for group, _ in asked] == ["lakehouse-cdc-conn-a", "lakehouse-cdc-conn-b"]
    assert asked[0][1] == {
        "public.orders": "cdc.conn-a.public.orders",
        "public.items": "cdc.conn-a.public.items",
    }
    assert measured["conn-c"] is None, "a SQL Server dial with no database cannot name its topics"

    posts: list[tuple[str, list[dict]]] = []
    monkeypatch.setattr(cdc_apply, "post_capture_observations", lambda cfg, cid, tables: posts.append((cid, tables)) or True)
    cdc_apply.report_backlogs(_observer_cfg(), measured)
    assert posts == [
        ("conn-a", [{"table": "public.orders", "backlog": 7}]),
        ("conn-b", [{"table": "public.orders", "backlog": 7}]),
        ("conn-c", []),
    ]


def test_the_sensors_backlog_is_the_sum_of_its_tables_and_unknown_when_any_is_unmeasured() -> None:
    assert cdc_apply._total_of({"a": 2, "b": 3}) == 5
    assert cdc_apply._total_of({"a": 2, "b": None}) is None
    assert cdc_apply._total_of({}) == 0
    assert cdc_apply._total_of(None) is None


def test_the_backlog_of_each_table_is_measured_apart(monkeypatch) -> None:
    class _Lag:
        def partitions_for_topic(self, topic):
            return {0} if topic != "absent" else None

        def committed(self, tp):
            return 10

        def beginning_offsets(self, tps):
            return {tp: 0 for tp in tps}

        def end_offsets(self, tps):
            return {tp: 25 for tp in tps}

    config = CdcConfig("kafka:9092")
    got = cdc_apply.backlog_by_table(config, "g", {"t1": "one", "t2": "absent"}, consumer=_Lag())
    assert got == {"t1": 15, "t2": None}, "a missing topic is unmeasured for the lag post (SHOULD-FIX 4)"
    assert cdc_apply.backlog_by_table(config, "g", {"t2": "absent"}, consumer=_Lag(), absent=0) == {"t2": 0}


def test_a_table_without_a_topic_yet_is_left_out_of_the_post_but_does_not_hold_back_the_run() -> None:
    """Slice 4 review SHOULD-FIX 4."""
    connectors = [{**_connector(_object("public.orders"), _object("public.items")), "id": "conn-a"}]

    def measure(config, group_id, topics, absent=None):
        return {"public.orders": 4, "public.items": absent}

    config = CdcConfig("kafka:9092")
    posted = cdc_apply.measure_backlogs(config, connectors, measure=measure)
    assert posted == {"conn-a": {"public.orders": 4, "public.items": None}}
    decided = cdc_apply.run_backlogs(config, connectors, posted, measure=measure)
    assert cdc_apply._total_of(decided["conn-a"]) == 4
    assert cdc_apply._total_of(posted["conn-a"]) is None
