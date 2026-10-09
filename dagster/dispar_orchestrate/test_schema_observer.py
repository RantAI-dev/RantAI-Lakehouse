"""Unit tests for `schema_observer.py` (`SRC-8`, task 6): reflecting a source
table without a second connection, holding columns back, and asking
`lakehouse-api` what to do.

No network: `requests.post` is monkeypatched; the reflection tests use a
SQLite file through the real `dlt.sources.sql_database` (the same hook
`adapters/sql.py` and `dlt_pipeline.py` use), never a server.

Run with:
~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_schema_observer.py -q
"""

from __future__ import annotations

import logging
import sqlite3
from pathlib import Path
from typing import Any

import pytest
import requests
from dlt.sources.sql_database import sql_database

from dispar_orchestrate import schema_observer as so

TOKEN = "tok-very-secret-123"


def _cfg(token: str = TOKEN) -> so.ObserverConfig:
    return so.ObserverConfig(api_url="http://api.test", service_token=token)


def _columns() -> list[so.ReflectedColumn]:
    return [so.ReflectedColumn("id", "INTEGER", False), so.ReflectedColumn("note", "VARCHAR(40)", True)]


class _Resp:
    def __init__(self, status: int = 200, body: Any = None, json_error: bool = False) -> None:
        self.status_code, self.body, self.json_error = status, body, json_error

    def raise_for_status(self) -> None:
        if self.status_code >= 400:
            raise requests.HTTPError(f"{self.status_code} for url with {TOKEN}", response=self)

    def json(self) -> Any:
        if self.json_error:
            raise requests.exceptions.JSONDecodeError("not json", "", 0)
        return self.body


def _post(monkeypatch, response: Any) -> list[dict]:
    calls: list[dict] = []

    def fake_post(url, **kwargs):
        calls.append({"url": url, **kwargs})
        if isinstance(response, Exception):
            raise response
        return response

    monkeypatch.setattr(so.requests, "post", fake_post)
    return calls


def _observe(**overrides: Any) -> so.Decision:
    args: dict[str, Any] = {
        "cfg": _cfg(),
        "connector_id": "conn-a",
        "object_name": "public.orders",
        "columns": _columns(),
        "primary_key": ["id"],
        "phase": "before_load",
        "run_id": "run-1",
    }
    args.update(overrides)
    return so.post_observation(**args)


# --- the request ----------------------------------------------------------


def test_the_observation_is_posted_to_the_connectors_route_with_the_wire_shape_and_a_timeout(monkeypatch) -> None:
    calls = _post(monkeypatch, _Resp(body={"action": "load", "columns": None, "changes": []}))
    _observe()
    assert calls[0]["url"] == "http://api.test/api/connectors/conn-a/schema-observations"
    assert calls[0]["json"] == {
        "object": "public.orders",
        "columns": [
            {"name": "id", "typeName": "INTEGER", "nullable": False},
            {"name": "note", "typeName": "VARCHAR(40)", "nullable": True},
        ],
        "primaryKey": ["id"],
        "phase": "before_load",
        "runId": "run-1",
    }
    assert calls[0]["timeout"] == 10


def test_the_service_token_is_sent_as_a_bearer_and_appears_in_no_message_or_log(monkeypatch, caplog) -> None:
    calls = _post(monkeypatch, _Resp(status=500))
    with caplog.at_level(logging.DEBUG):
        with pytest.raises(so.ObservationUnreachable) as caught:
            _observe()
    assert calls[0]["headers"] == {"Authorization": f"Bearer {TOKEN}"}
    # Not in the message, not in a chained exception, not in a log line.
    assert TOKEN not in str(caught.value)
    assert caught.value.__cause__ is None and caught.value.__suppress_context__
    assert TOKEN not in caplog.text


def test_an_unset_service_token_is_refused_without_calling_the_api(monkeypatch) -> None:
    calls = _post(monkeypatch, _Resp(body={"action": "load"}))
    with pytest.raises(so.ObservationRefused, match="INGEST_SERVICE_TOKEN is unset"):
        _observe(cfg=_cfg(token=""))
    assert calls == []


def test_a_run_id_of_none_is_left_out_of_the_body(monkeypatch) -> None:
    calls = _post(monkeypatch, _Resp(body={"action": "load"}))
    _observe(run_id=None)
    assert "runId" not in calls[0]["json"]


def test_an_unknown_phase_is_a_programming_error(monkeypatch) -> None:
    _post(monkeypatch, _Resp(body={"action": "load"}))
    with pytest.raises(ValueError):
        _observe(phase="during_load")


# --- the answer -------------------------------------------------------------


def test_a_load_answer_with_no_column_list_means_every_column(monkeypatch) -> None:
    _post(monkeypatch, _Resp(body={"action": "load", "columns": None, "changes": [{"kind": "column_added"}]}))
    decision = _observe()
    assert decision == so.Decision(action="load", columns=None, changes=[{"kind": "column_added"}])


def test_a_load_answer_with_a_column_list_carries_it(monkeypatch) -> None:
    _post(monkeypatch, _Resp(body={"action": "load", "columns": ["id"], "changes": []}))
    assert _observe().columns == ["id"]


def test_a_wait_answer_is_returned_as_wait(monkeypatch) -> None:
    _post(monkeypatch, _Resp(body={"action": "wait", "columns": None, "changes": []}))
    assert _observe().action == "wait"


@pytest.mark.parametrize(
    "body",
    [
        None,
        [],
        {},
        {"action": "maybe"},
        {"action": "load", "columns": "id"},
        {"action": "load", "columns": [1]},
        {"action": "load", "changes": "none"},
    ],
)
def test_an_answer_that_is_not_a_decision_is_refused(monkeypatch, body) -> None:
    _post(monkeypatch, _Resp(body=body))
    with pytest.raises(so.ObservationRefused):
        _observe()


def test_an_answer_that_is_not_json_is_refused(monkeypatch) -> None:
    _post(monkeypatch, _Resp(json_error=True))
    with pytest.raises(so.ObservationRefused):
        _observe()


# --- failing closed -----------------------------------------------------------


@pytest.mark.parametrize("status", [400, 401, 403, 404, 409])
def test_a_4xx_answer_is_refused_and_so_not_worth_a_retry(monkeypatch, status) -> None:
    _post(monkeypatch, _Resp(status=status))
    with pytest.raises(so.ObservationRefused, match=str(status)):
        _observe()


@pytest.mark.parametrize("status", [500, 502, 503])
def test_a_5xx_answer_is_unreachable_and_so_worth_a_retry(monkeypatch, status) -> None:
    _post(monkeypatch, _Resp(status=status))
    with pytest.raises(so.ObservationUnreachable):
        _observe()


@pytest.mark.parametrize("error", [requests.ConnectionError("down"), requests.Timeout("slow")])
def test_an_api_that_cannot_be_reached_is_unreachable(monkeypatch, error) -> None:
    _post(monkeypatch, error)
    with pytest.raises(so.ObservationUnreachable, match=type(error).__name__):
        _observe()


def test_the_two_failures_are_told_apart_by_type() -> None:
    assert not issubclass(so.ObservationRefused, so.ObservationUnreachable)
    assert not issubclass(so.ObservationUnreachable, so.ObservationRefused)
    # A source build that reflected nothing is refused, never loaded unchecked.
    assert issubclass(so.ReflectionMissing, so.ObservationRefused)


# --- reflection ---------------------------------------------------------------


def _sqlite(tmp_path: Path) -> dict[str, str]:
    db = tmp_path / "src.db"
    with sqlite3.connect(db) as conn:
        conn.execute("CREATE TABLE src (a INTEGER NOT NULL PRIMARY KEY, b VARCHAR(20), c INTEGER)")
        conn.execute("INSERT INTO src VALUES (1, 'x', 7)")
    return {"drivername": "sqlite", "database": str(db)}


def test_the_collector_reads_names_types_nullability_and_key_from_the_reflected_table(tmp_path) -> None:
    collector = so.ReflectionCollector()
    sql_database(credentials=_sqlite(tmp_path), table_names=["src"], table_adapter_callback=collector.callback)
    table = collector.only()
    assert [(c.name, c.type_name) for c in table.columns] == [("a", "INTEGER"), ("b", "VARCHAR(20)"), ("c", "INTEGER")]
    assert table.primary_key == ("a",)
    assert [c.nullable for c in table.columns][1:] == [True, True]


def test_the_collector_has_read_the_columns_before_any_row_is_read(tmp_path) -> None:
    collector = so.ReflectionCollector()
    source = sql_database(
        credentials=_sqlite(tmp_path), table_names=["src"], table_adapter_callback=collector.callback
    )
    # The source exists and nothing has iterated it: reflection is done.
    assert len(collector.tables) == 1
    assert "src" in source.resources


def test_a_collector_that_saw_no_table_refuses_instead_of_guessing() -> None:
    with pytest.raises(so.ReflectionMissing):
        so.ReflectionCollector().only()


def test_keep_only_leaves_the_unlisted_column_out_of_the_rows_that_would_load(tmp_path) -> None:
    source = sql_database(
        credentials=_sqlite(tmp_path), table_names=["src"], table_adapter_callback=so.keep_only(["a", "b"])
    )
    rows = [row for batch in source.resources["src"] for row in (batch if isinstance(batch, list) else [batch])]
    assert rows == [{"a": 1, "b": "x"}]


# --- the new-table report (SRC-8 task 8) --------------------------------------


def test_new_tables_are_posted_to_the_tables_route_and_the_answer_is_parsed(monkeypatch) -> None:
    calls = _post(
        monkeypatch,
        _Resp(body={"added": ["public.b"], "notAdded": [{"table": "public.a", "reason": "Not added: ..."}]}),
    )
    answer = so.post_new_tables(_cfg(), "conn-a", ["public.a", "public.b"], "run-1")
    assert calls[0]["url"] == "http://api.test/api/connectors/conn-a/schema-observations/tables"
    assert calls[0]["json"] == {"tables": ["public.a", "public.b"], "runId": "run-1"}
    assert calls[0]["headers"] == {"Authorization": f"Bearer {TOKEN}"}
    assert calls[0]["timeout"] == 10
    assert answer == so.NewTablesAnswer(
        added=["public.b"], not_added=[{"table": "public.a", "reason": "Not added: ..."}]
    )


def test_a_409_on_the_tables_route_is_refused_and_a_5xx_is_unreachable(monkeypatch) -> None:
    _post(monkeypatch, _Resp(status=409))
    with pytest.raises(so.ObservationRefused, match="409"):
        so.post_new_tables(_cfg(), "conn-a", ["a.b"], None)
    _post(monkeypatch, _Resp(status=503))
    with pytest.raises(so.ObservationUnreachable):
        so.post_new_tables(_cfg(), "conn-a", ["a.b"], None)


@pytest.mark.parametrize("body", [None, {}, {"added": "x", "notAdded": []}, {"added": [], "notAdded": ["x"]}])
def test_an_answer_that_is_not_an_added_and_not_added_list_is_refused(monkeypatch, body) -> None:
    _post(monkeypatch, _Resp(body=body))
    with pytest.raises(so.ObservationRefused):
        so.post_new_tables(_cfg(), "conn-a", ["a.b"], None)


def test_more_tables_than_the_api_accepts_in_one_request_is_a_programming_error() -> None:
    with pytest.raises(ValueError):
        so.post_new_tables(_cfg(), "conn-a", [f"s.t{i}" for i in range(so.MAX_TABLES_PER_REQUEST + 1)], None)
