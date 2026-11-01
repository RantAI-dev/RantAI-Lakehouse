"""Unit tests for `dagster/dispar_orchestrate/bronze_catalog.py`'s
`record_ingest_run` (WS3 plan review Z9) and its `register_bronze_table`
slug guard (DATA-40 D2).

Mirrors `test_capacity_snapshot.py::test_record_capacity_snapshot_writes_one_row`'s
monkeypatch shape: `record_ingest_run` is defined IN `bronze_catalog`, so
`_ch_exec`/`_assert_or_create_all` are patched directly on the
`bronze_catalog` module object, not on some importer of it.

Run with:
`~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_bronze_catalog.py -q`
"""

from __future__ import annotations

import pytest

from dispar_orchestrate import bronze_catalog
from dispar_orchestrate.bronze_catalog import (
    ClickHouseTarget,
    SlugConflictError,
    last_committed_offset,
    record_ingest_offset,
    record_ingest_run,
    register_bronze_table,
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


# ── record_ingest_offset / last_committed_offset ─────────────────────────


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


# ── DATA-40 D2: the slug guard on the write side ─────────────────────────
#
# The catalog reads two registries (`lake.bronze_meta.*` and
# `lake.bronze_meta_sec.*`); one slug must name one dataset (D1). The
# writer therefore refuses a slug the SECONDARY registry holds, and it
# refuses it BEFORE the first insert (D2), so a refused write leaves the
# primary registry untouched.
#
# The fake below is the module's own fake-ClickHouse shape: the registry
# tables do not exist yet (the R10 schema bootstrap creates them, which is
# why `_ch_query_json` answers the `system.tables` read with no rows), and
# the secondary registry holds exactly the slugs the test gives it.


def _fake_clickhouse(monkeypatch, *, secondary_slugs):
    """A fake ClickHouse whose secondary registry holds `secondary_slugs`.

    Returns the statements that were executed and the statements that were
    read, so a test can assert both what was written and what was asked.
    """
    executed: list[str] = []
    queries: list[str] = []

    def _query_json(target, stmt):
        queries.append(stmt)
        if "FROM lake.`bronze_meta_sec.dataset_catalog`" not in stmt:
            # The R10 schema bootstrap's `system.tables` read: the registry
            # tables do not exist yet, so they are created.
            return []
        return [{"slug": slug} for slug in secondary_slugs if f"'{slug}'" in stmt]

    monkeypatch.setattr(bronze_catalog, "_ch_query_json", _query_json)
    monkeypatch.setattr(bronze_catalog, "_ch_exec", lambda target, stmt: executed.append(stmt))
    return executed, queries


def test_register_bronze_table_refuses_a_slug_the_secondary_registry_holds(monkeypatch):
    executed, _queries = _fake_clickhouse(monkeypatch, secondary_slugs=["orders"])

    with pytest.raises(SlugConflictError) as err:
        register_bronze_table(
            slug="orders",
            title="Orders",
            description="The orders",
            bronze_table_name="orders_raw",
            row_count=10,
            target=ClickHouseTarget(url="http://ch", user="default", password=""),
        )

    assert "orders" in str(err.value)
    assert not [s for s in executed if "INSERT INTO" in s], (
        "a refused slug inserts nothing: " + repr(executed)
    )


def test_register_bronze_table_registers_a_free_slug_as_before(monkeypatch):
    executed, _queries = _fake_clickhouse(monkeypatch, secondary_slugs=["returns"])

    register_bronze_table(
        slug="orders",
        title="Orders",
        description="The orders",
        bronze_table_name="orders_raw",
        row_count=10,
        columns=[("id", "UInt64", "The id")],
        target=ClickHouseTarget(url="http://ch", user="default", password=""),
    )

    inserts = [s for s in executed if "INSERT INTO" in s]
    assert len(inserts) == 3, f"catalog, sync and column rows are written: {executed!r}"
    assert all("'orders'" in s for s in inserts), inserts
    assert any("lake.`bronze_meta.dataset_catalog`" in s for s in inserts), inserts


def test_the_slug_in_the_conflict_check_is_a_bound_literal(monkeypatch):
    # A slug with a quote in it: interpolated raw it would break the
    # statement, so it has to go through `_sql_string_literal`.
    _executed, queries = _fake_clickhouse(monkeypatch, secondary_slugs=[])

    register_bronze_table(
        slug="or'ders",
        title="Orders",
        description="The orders",
        bronze_table_name="orders_raw",
        row_count=10,
        target=ClickHouseTarget(url="http://ch", user="default", password=""),
    )

    check = next(q for q in queries if "FROM lake.`bronze_meta_sec.dataset_catalog`" in q)
    assert "FINAL" in check, check
    assert "WHERE slug = 'or\\'ders'" in check, check
    assert "or'ders" not in check.replace("or\\'ders", ""), check
