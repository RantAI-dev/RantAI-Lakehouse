"""Unit tests for `dagster/dispar_orchestrate/bronze_catalog.py`'s
`record_ingest_run` (WS3 plan review Z9).

Mirrors `test_capacity_snapshot.py::test_record_capacity_snapshot_writes_one_row`'s
monkeypatch shape: `record_ingest_run` is defined IN `bronze_catalog`, so
`_ch_exec`/`_assert_or_create_all` are patched directly on the
`bronze_catalog` module object, not on some importer of it.

Run with:
`~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_bronze_catalog.py -q`
"""

from __future__ import annotations

from dispar_orchestrate import bronze_catalog
from dispar_orchestrate.bronze_catalog import (
    ClickHouseTarget,
    last_committed_offset,
    record_ingest_offset,
    record_ingest_run,
)


def test_record_ingest_run_creates_the_table_and_inserts_a_row(monkeypatch):
    executed = []
    monkeypatch.setattr(bronze_catalog, "_ch_query_json", lambda target, stmt: [])
    monkeypatch.setattr(bronze_catalog, "_ch_exec", lambda target, stmt: executed.append(stmt))
    record_ingest_run(
        connector_id="conn-x",
        job="ingest_job",
        object_name="orders",
        rows=42,
        started_at="2026-09-11T00:00:00Z",
        ended_at="2026-09-11T00:00:05Z",
        status="succeeded",
        target=ClickHouseTarget(url="http://ch", user="default", password=""),
    )
    assert any("CREATE TABLE IF NOT EXISTS lake.`bronze_meta.ingest_run`" in s for s in executed)
    assert any("INSERT INTO lake.`bronze_meta.ingest_run`" in s and "'conn-x'" in s for s in executed)


def test_record_ingest_run_writes_null_rows_when_the_count_is_unmeasured(monkeypatch):
    # WS3 plan review Z9: rows=None (never a fabricated 0) renders as
    # SQL NULL, not the string "None" or the integer 0.
    executed = []
    monkeypatch.setattr(bronze_catalog, "_ch_query_json", lambda target, stmt: [])
    monkeypatch.setattr(bronze_catalog, "_ch_exec", lambda target, stmt: executed.append(stmt))
    record_ingest_run(
        connector_id="conn-x",
        job="ingest_job",
        object_name="orders",
        rows=None,
        started_at="2026-09-11T00:00:00Z",
        ended_at="2026-09-11T00:00:05Z",
        status="rejected",
        error="ssrf blocked",
        target=ClickHouseTarget(url="http://ch", user="default", password=""),
    )
    insert = next(s for s in executed if "INSERT INTO" in s)
    assert ", NULL, " in insert


def test_record_ingest_run_defaults_error_to_empty_string(monkeypatch):
    executed = []
    monkeypatch.setattr(bronze_catalog, "_ch_query_json", lambda target, stmt: [])
    monkeypatch.setattr(bronze_catalog, "_ch_exec", lambda target, stmt: executed.append(stmt))
    record_ingest_run(
        connector_id="conn-x",
        job="ingest_job",
        object_name="orders",
        rows=1,
        started_at="2026-09-11T00:00:00Z",
        ended_at="2026-09-11T00:00:05Z",
        status="succeeded",
        target=ClickHouseTarget(url="http://ch", user="default", password=""),
    )
    insert = next(s for s in executed if "INSERT INTO" in s)
    assert insert.rstrip().endswith("'')")


# ── record_ingest_offset / last_committed_offset (WS9 plan Task A1/D4) ──


def test_record_ingest_offset_creates_the_table_and_inserts_a_row(monkeypatch):
    executed = []
    monkeypatch.setattr(bronze_catalog, "_ch_query_json", lambda target, stmt: [])
    monkeypatch.setattr(bronze_catalog, "_ch_exec", lambda target, stmt: executed.append(stmt))
    record_ingest_offset(
        "conn-kafka",
        "orders",
        0,
        11,
        target=ClickHouseTarget(url="http://ch", user="default", password=""),
    )
    assert any("CREATE TABLE IF NOT EXISTS lake.`bronze_meta.ingest_offset`" in s for s in executed)
    insert = next(s for s in executed if "INSERT INTO lake.`bronze_meta.ingest_offset`" in s)
    assert "'conn-kafka'" in insert
    assert "'orders'" in insert
    assert ", 0, 11, " in insert  # partition_id, committed_offset -- integers, not quoted


def test_last_committed_offset_returns_none_when_nothing_was_ever_committed(monkeypatch):
    monkeypatch.setattr(bronze_catalog, "_ch_query_json", lambda target, stmt: [])
    result = last_committed_offset(
        ClickHouseTarget(url="http://ch", user="default", password=""), "conn-kafka", "orders", 0
    )
    assert result is None


def test_last_committed_offset_returns_none_when_the_query_fails(monkeypatch):
    # Mirrors latest_maintenance_run_at's own "table not created yet, or
    # ClickHouse unreachable -> treat as never committed" contract.
    import requests

    def _raise(target, stmt):
        raise requests.ConnectionError("unreachable")

    monkeypatch.setattr(bronze_catalog, "_ch_query_json", _raise)
    result = last_committed_offset(
        ClickHouseTarget(url="http://ch", user="default", password=""), "conn-kafka", "orders", 0
    )
    assert result is None


def test_last_committed_offset_returns_the_real_committed_value(monkeypatch):
    monkeypatch.setattr(bronze_catalog, "_ch_query_json", lambda target, stmt: [{"committed_offset": "11"}])
    result = last_committed_offset(
        ClickHouseTarget(url="http://ch", user="default", password=""), "conn-kafka", "orders", 0
    )
    assert result == 11
