"""What `run_ingest` and `ingest_source_object` report about a run
(`SRC-11` D5, D7, D9, D13), and the pause check and left-out columns that sit
beside it. `conftest.py`'s `reports` fixture records what the ops would send;
no network.
"""

from __future__ import annotations

from types import SimpleNamespace

import pytest
import requests
from dagster import Failure, RetryRequested

import dispar_orchestrate.ingest_factory as f
from dispar_orchestrate import ingest_errors, schema_observer

# The module-level fixtures and fakes of the main ingest tests: the API that
# answers every schema observation, and a reflected two-column table.
from dispar_orchestrate.test_ingest_factory import (  # noqa: F401 -- fixtures are used by name
    _FakeTable,
    _mysql_connector,
    _no_catalog_registration,
    _stub_env,
    _wire_mysql,
    observations,
)


@pytest.fixture(autouse=True)
def _env(monkeypatch):
    _stub_env(monkeypatch)
    monkeypatch.delenv(ingest_errors.RETRY_DELAYS_ENV, raising=False)


def _run_context(config: dict, *, tags: dict | None = None, retry_number: int = 0):
    return SimpleNamespace(
        op_config=config,
        retry_number=retry_number,
        run_id="run-1",
        dagster_run=SimpleNamespace(tags=tags or {}),
        log=SimpleNamespace(info=lambda *_: None),
    )


def _run_ingest(context) -> list:
    return list(f.run_ingest.compute_fn.decorated_fn(context))


def _step_context(retry_number: int = 0):
    return SimpleNamespace(retry_number=retry_number, run_id="run-1", log_event=lambda *_: None)


def _step(context, connector: dict, obj: dict):
    return f.ingest_source_object.compute_fn.decorated_fn(context, {"connector": connector, "obj": obj})


def _batch(*names: str, **extra) -> dict:
    return {
        "id": "conn-1",
        "adapter": "sql",
        "dial": {},
        "sourceObjects": [{"name": n, "target": n} for n in names],
        **extra,
    }


# --- the start report and its trigger ----------------------------------------


@pytest.mark.parametrize(
    ("tags", "config", "expected"),
    [
        ({"lakehouse/trigger": "schedule"}, {}, "schedule"),
        ({"lakehouse/trigger": "user"}, {}, "user"),
        ({}, {"object": "t", "action": "clear"}, "clear"),
        ({}, {"object": "t", "action": "refresh_keep"}, "refresh"),
        ({}, {"object": "t", "action": "refresh_remove"}, "refresh"),
        # The action wins over a tag: it says what the run does.
        ({"lakehouse/trigger": "schedule"}, {"object": "t", "action": "clear"}, "clear"),
        # No tag (the API's launch, the orchestrator's own screen), or one we do not know.
        ({}, {}, "other"),
        ({"lakehouse/trigger": "cron-job"}, {}, "other"),
    ],
)
def test_the_run_is_reported_with_the_trigger_its_tags_and_action_show(monkeypatch, reports, tags, config, expected) -> None:
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: _batch("t"))
    _run_ingest(_run_context({"connector_id": "conn-1", **config}, tags=tags))
    # A refresh or clear names its table (SRC-11 review 1 BLOCKER 2).
    expected_report = ("start", expected, "t") if expected in ("refresh", "clear") else ("start", expected)
    assert reports[0] == expected_report


def test_a_directly_invoked_op_without_a_run_reports_other(monkeypatch, reports) -> None:
    context = _run_context({"connector_id": "conn-1"})
    del context.dagster_run
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: _batch("t"))
    _run_ingest(context)
    assert reports[0] == ("start", "other")


def test_the_start_is_reported_before_the_connector_is_fetched(monkeypatch, reports) -> None:
    def down(cfg, cid):
        raise requests.ConnectionError("api")

    monkeypatch.setattr(f, "_fetch_one_connector", down)
    with pytest.raises(RetryRequested):
        _run_ingest(_run_context({"connector_id": "conn-1"}))
    assert reports == [("start", "other")]


# --- the pause (D13, SRC-18) --------------------------------------------------


def test_a_paused_connectors_run_loads_nothing_and_ends_paused(monkeypatch, reports) -> None:
    ran: list = []
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: _batch("orders", "customers", paused=True))
    monkeypatch.setattr(f, "_run_one_object", lambda *a, **k: ran.append(a))
    monkeypatch.setattr(f, "_discover_new_tables", lambda *a: ran.append("discover") or False)
    outputs = _run_ingest(_run_context({"connector_id": "conn-1"}))
    assert outputs == [] and ran == []
    assert reports == [("start", "other"), ("end", "paused", "connector_paused")]


def test_a_paused_connector_is_not_run_even_for_one_table_with_an_action(monkeypatch, reports) -> None:
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: _batch("orders", paused=True))
    outputs = _run_ingest(_run_context({"connector_id": "conn-1", "object": "orders", "action": "clear"}))
    assert outputs == []
    assert reports[-1] == ("end", "paused", "connector_paused")


@pytest.mark.parametrize("paused", [False, None, "true", 1])
def test_only_an_explicit_true_pauses(monkeypatch, reports, paused) -> None:
    connector = _batch("orders")
    if paused is not None:
        connector["paused"] = paused
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: connector)
    outputs = _run_ingest(_run_context({"connector_id": "conn-1"}))
    assert len(outputs) == 1 and ("end", "paused", "connector_paused") not in reports


# --- the end of a run the op carries out alone --------------------------------


def test_a_cdc_run_is_reported_started_and_completed(monkeypatch, reports) -> None:
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: {"id": cid, "adapter": "cdc"})
    _run_ingest(_run_context({"connector_id": "conn-1"}))
    assert reports == [("start", "other"), ("end", "succeeded", "completed")]


def _stream_connector() -> dict:
    return {
        "id": "k1",
        "adapter": "kafka",
        "ingestMode": "stream",
        "dial": {"topic": "events"},
        "sourceObjects": [{"name": "events", "target": "events"}],
    }


def test_a_kafka_micro_batch_reports_its_attempt_and_its_end(monkeypatch, reports) -> None:
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: _stream_connector())
    monkeypatch.setattr(f, "_run_stream_connector", lambda connector, run_id=None: 17)
    _run_ingest(_run_context({"connector_id": "k1"}))
    kinds = [r[0] for r in reports]
    assert kinds == ["start", "attempt", "end"]
    attempt = reports[1][1]
    assert (attempt["table"], attempt["attempt"], attempt["rows_loaded"]) == ("events", 1, 17)
    assert attempt.get("error_class") is None
    assert reports[2] == ("end", "succeeded", "completed")


def test_a_kafka_failure_that_will_be_retried_reports_the_class_and_the_wait_and_no_end(monkeypatch, reports) -> None:
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: _stream_connector())

    def down(connector, run_id=None):
        raise ConnectionRefusedError("broker")

    monkeypatch.setattr(f, "_run_stream_connector", down)
    with pytest.raises(RetryRequested):
        _run_ingest(_run_context({"connector_id": "k1"}, retry_number=1))
    [_, (_, attempt)] = reports
    assert (attempt["attempt"], attempt["error_class"], attempt["wait_seconds"]) == (2, "unreachable", 300)


def test_a_kafka_failure_that_ends_the_run_reports_it_failed(monkeypatch, reports) -> None:
    monkeypatch.setattr(f, "_fetch_one_connector", lambda cfg, cid: _stream_connector())

    def refused(connector, run_id=None):
        raise ValueError("bad setting")

    monkeypatch.setattr(f, "_run_stream_connector", refused)
    with pytest.raises(Failure):
        _run_ingest(_run_context({"connector_id": "k1"}))
    assert reports[1][1]["error_class"] == "setting_rejected"
    assert reports[1][1]["wait_seconds"] is None
    assert reports[-1] == ("end", "failed", "table_failed")


# --- one report per attempt ----------------------------------------------------


def test_a_table_that_loaded_reports_its_rows(monkeypatch, reports) -> None:
    monkeypatch.setattr(f, "_run_one_object", lambda connector, obj, run_id=None: 42)
    assert _step(_step_context(), {"id": "conn-1"}, {"name": "shop.orders", "target": "orders"}) == 42
    [(kind, attempt)] = reports
    assert kind == "attempt"
    assert (attempt["table"], attempt["attempt"], attempt["rows_loaded"]) == ("shop.orders", 1, 42)
    assert attempt.get("error_class") is None
    assert attempt["started_at"] <= attempt["ended_at"]


def test_a_failure_that_will_be_retried_reports_its_class_and_the_wait(monkeypatch, reports) -> None:
    def down(connector, obj, run_id=None):
        raise ConnectionRefusedError("db.internal:5432")

    monkeypatch.setattr(f, "_run_one_object", down)
    with pytest.raises(RetryRequested):
        _step(_step_context(retry_number=2), {"id": "conn-1"}, {"name": "orders", "target": "orders"})
    [(_, attempt)] = reports
    assert (attempt["attempt"], attempt["error_class"], attempt["wait_seconds"]) == (3, "unreachable", 900)
    assert "db.internal" not in repr(reports)


def test_a_failure_that_will_not_be_retried_reports_its_class_and_no_wait(monkeypatch, reports) -> None:
    import psycopg2

    def refused(connector, obj, run_id=None):
        raise psycopg2.OperationalError('FATAL:  password authentication failed for user "g6"')

    monkeypatch.setattr(f, "_run_one_object", refused)
    with pytest.raises(Failure):
        _step(_step_context(), {"id": "conn-1"}, {"name": "orders", "target": "orders"})
    [(_, attempt)] = reports
    assert attempt["error_class"] == "sign_in_refused" and attempt["wait_seconds"] is None
    assert "g6" not in repr(reports) and "password" not in repr(reports)


def test_the_last_attempt_after_the_retries_are_used_up_reports_no_wait(monkeypatch, reports) -> None:
    def down(connector, obj, run_id=None):
        raise TimeoutError()

    monkeypatch.setattr(f, "_run_one_object", down)
    with pytest.raises(Failure):
        _step(_step_context(retry_number=3), {"id": "conn-1"}, {"name": "orders", "target": "orders"})
    [(_, attempt)] = reports
    assert (attempt["attempt"], attempt["error_class"], attempt["wait_seconds"]) == (4, "timeout", None)


def test_a_report_that_cannot_be_sent_does_not_fail_the_load(monkeypatch) -> None:
    class Broken:
        def attempt(self, **_):
            return False

        def log(self, *_a, **_k):
            return False

    monkeypatch.setattr(f.run_reports.RunReporter, "for_run", classmethod(lambda cls, c, r: Broken()))
    monkeypatch.setattr(f, "_run_one_object", lambda connector, obj, run_id=None: 5)
    assert _step(_step_context(), {"id": "conn-1"}, {"name": "o", "target": "o"}) == 5


# --- a table waiting for a decision, and left-out columns ----------------------


def test_a_table_that_waits_reports_the_pending_class_and_its_log_line(monkeypatch, reports, observations) -> None:
    loads: list = []
    _wire_mysql(monkeypatch, loads=loads, builds=[])
    observations.answer = schema_observer.Decision(action="wait", columns=None, changes=[{"kind": "type_changed"}])
    assert _step(_step_context(), _mysql_connector(), {"name": "shop.orders", "target": "orders"}) is None
    assert loads == []
    assert [r[0] for r in reports] == ["attempt", "log"]
    assert reports[0][1]["error_class"] == "schema_decision_pending" and reports[0][1]["rows_loaded"] == 0
    assert reports[1] == ("log", "schema_decision_pending", "shop.orders", None)


def test_excluded_columns_are_left_out_of_the_load_after_the_whole_table_was_observed(
    monkeypatch, reports, observations
) -> None:
    loads: list = []
    builds: list = []
    _wire_mysql(monkeypatch, loads=loads, builds=builds)
    obj = {"name": "shop.orders", "target": "orders", "excludedColumns": ["note"]}
    assert _step(_step_context(), _mysql_connector(), obj) == 5
    # The API saw both columns: a column left out is not "removed at the source".
    [call] = observations.calls
    assert [c.name for c in call["columns"]] == ["id", "note"]
    # The load selected only `id`.
    assert builds[-1] == ["id"] and loads == ["orders"]
    assert [r[0] for r in reports] == ["attempt", "log"]
    assert reports[1] == ("log", "columns_left_out", "shop.orders", 1)


def test_excluded_columns_are_subtracted_from_the_apis_own_column_list(monkeypatch, observations) -> None:
    builds: list = []
    _wire_mysql(monkeypatch, loads=[], builds=builds, table=_FakeTable(
        columns=(("id", "INTEGER", False), ("note", "VARCHAR(40)", True), ("qty", "INTEGER", True))
    ))
    # ask_first: the API says load these two; the object also leaves `qty` out.
    observations.answer = schema_observer.Decision(action="load", columns=["id", "qty"], changes=[])
    f._run_one_object(_mysql_connector(), {"name": "shop.orders", "target": "orders", "excludedColumns": ["qty"]})
    assert builds[-1] == ["id"]


def test_a_name_the_table_does_not_have_leaves_nothing_out_and_reports_nothing(monkeypatch, reports, observations) -> None:
    builds: list = []
    _wire_mysql(monkeypatch, loads=[], builds=builds)
    obj = {"name": "shop.orders", "target": "orders", "excludedColumns": ["gone"]}
    _step(_step_context(), _mysql_connector(), obj)
    assert builds == [["id", "note"]]
    assert [r[0] for r in reports] == ["attempt"]


def test_no_excluded_columns_changes_nothing(monkeypatch, reports, observations) -> None:
    builds: list = []
    _wire_mysql(monkeypatch, loads=[], builds=builds)
    for excluded in (None, []):
        builds.clear()
        reports.clear()
        _step(_step_context(), _mysql_connector(), {"name": "shop.orders", "target": "orders", "excludedColumns": excluded})
        assert builds == [["id", "note"]] and [r[0] for r in reports] == ["attempt"]


def test_leaving_out_every_column_loads_nothing_and_is_not_retried(monkeypatch, reports, observations) -> None:
    loads: list = []
    _wire_mysql(monkeypatch, loads=loads, builds=[])
    obj = {"name": "shop.orders", "target": "orders", "excludedColumns": ["id", "note"]}
    with pytest.raises(Failure) as ended:
        _step(_step_context(), _mysql_connector(), obj)
    assert ended.value.allow_retries is False and loads == []
    assert reports[0][1]["error_class"] == "setting_rejected"


def test_the_columns_a_failed_last_attempt_left_out_are_reported_with_it(monkeypatch, reports, observations) -> None:
    _wire_mysql(monkeypatch, loads=[], builds=[])

    def broken_load(*_a, **_k):
        raise ValueError("bad setting")

    monkeypatch.setattr(f.sink_adapter, "load_via_sink", broken_load)
    obj = {"name": "shop.orders", "target": "orders", "excludedColumns": ["note"]}
    with pytest.raises(Failure):
        _step(_step_context(), _mysql_connector(), obj)
    assert [r[0] for r in reports] == ["attempt", "log"] and reports[1][3] == 1


def test_a_retried_attempt_does_not_repeat_the_left_out_line(monkeypatch, reports, observations) -> None:
    _wire_mysql(monkeypatch, loads=[], builds=[])

    def down(*_a, **_k):
        raise ConnectionRefusedError()

    monkeypatch.setattr(f.sink_adapter, "load_via_sink", down)
    obj = {"name": "shop.orders", "target": "orders", "excludedColumns": ["note"]}
    with pytest.raises(RetryRequested):
        _step(_step_context(), _mysql_connector(), obj)
    assert [r[0] for r in reports] == ["attempt"]


def test_the_note_is_cleared_between_steps(monkeypatch, reports, observations) -> None:
    _wire_mysql(monkeypatch, loads=[], builds=[])
    _step(_step_context(), _mysql_connector(), {"name": "a", "target": "a", "excludedColumns": ["note"]})
    reports.clear()
    _step(_step_context(), _mysql_connector(), {"name": "b", "target": "b"})
    assert [r[0] for r in reports] == ["attempt"]
    assert f._NOTES.get() is None
