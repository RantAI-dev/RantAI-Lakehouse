"""`SRC-48` (`PR #621 review 1 SHOULD-FIX`): the load modes that keep their rows keep
the column the person left out, proven through `load_via_sink` itself. The branch's
no-data-loss test called the mode guard (`_drop_left_out_columns`) directly, so a
change to how `load_via_sink` treats a plan's mode -- an incremental first run that
happens to write as a replace, a deduplicating first run that does the same, the
history path that never reaches the guard's call site -- could have dropped the
column with every test still green. These tests load a table that holds `a, b, c`,
then run another load whose source no longer carries `c`, with `drop_columns=["c"]`,
in each row-keeping mode (`incremental`, `incremental_dedup`, `history`), and read
both stores back: the Iceberg schema and the pipeline's stored `dlt` schema, the two
places a drop touches (plan decision D3).

The harness is `test_load_modes_measurement.py`'s: `load_via_sink` runs for real with
only its two network-bound names swapped (`lab`), on a local Iceberg catalog, a
SQLite source for the incremental modes and plain rows for history. No network, no
query engine.
"""

from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import Any

import pytest

from dispar_orchestrate.adapters.sink import LoadPlan, load_via_sink
from dispar_orchestrate.test_load_modes_measurement import (  # noqa: F401 -- `lab` and `items_db` are fixtures
    Lab,
    _items_source,
    items_db,
    lab,
)

# Runs a real dlt pipeline against a local Iceberg table (registered in pyproject.toml);
# CI runs these in `dagster-loader-tests`, the rest in `dagster-unit-tests`.
pytestmark = pytest.mark.loader


def _iceberg_names(lab: Lab, table: str) -> list[str]:
    """The Iceberg schema's column names, the first store a drop touches."""
    return [field.name for field in lab.table(table).schema().fields]


def _dlt_columns(lab: Lab, table: str) -> dict[str, Any]:
    """The stored `dlt` schema of the table, the second store a drop touches."""
    return lab.pipeline(table).default_schema.tables[table]["columns"]


def test_src_48_an_incremental_load_through_the_sink_keeps_the_left_out_column_in_both_stores(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path
) -> None:
    """`PR #621 review 1 SHOULD-FIX`: an `incremental` plan keeps the column, although
    its first run writes as a replace -- the exact shape a mode-handling change would
    break, so the load goes through `load_via_sink`, not the guard."""
    load_via_sink(
        [{"id": 1, "v": "a", "c": 1.5}, {"id": 2, "v": "b", "c": 2.5}], "items", lab.config, LoadPlan(mode="replace")
    )
    result = load_via_sink(
        _items_source(tmp_path),
        "items",
        lab.config,
        LoadPlan(mode="incremental", cursor="updated_at"),
        drop_columns=["c"],
    )
    assert (result.rows, result.has_failed_jobs) == (3, False), "the first incremental run reads everything"
    assert "c" in _iceberg_names(lab, "items"), "a table that keeps its rows keeps the column (decision 10)"
    assert "c" in _dlt_columns(lab, "items"), "the stored `dlt` schema keeps it too"
    assert all(row["c"] is None for row in lab.rows("items")), "the new rows carry nulls in it, not values"


def test_src_48_an_incremental_dedup_load_through_the_sink_keeps_the_left_out_column_in_both_stores(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path
) -> None:
    """`PR #621 review 1 SHOULD-FIX`: an `incremental_dedup` plan keeps the column, its
    first run also writing as a replace; proven through `load_via_sink`."""
    load_via_sink(
        [{"id": 1, "v": "a", "c": 1.5}, {"id": 2, "v": "b", "c": 2.5}], "items", lab.config, LoadPlan(mode="replace")
    )
    result = load_via_sink(
        _items_source(tmp_path),
        "items",
        lab.config,
        LoadPlan(mode="incremental_dedup", cursor="updated_at", key=("id",)),
        drop_columns=["c"],
    )
    assert (result.rows, result.has_failed_jobs) == (3, False), "one row per key, the first run replacing"
    assert "c" in _iceberg_names(lab, "items"), "a table that keeps its rows keeps the column (decision 10)"
    assert "c" in _dlt_columns(lab, "items"), "the stored `dlt` schema keeps it too"
    assert all(row["c"] is None for row in lab.rows("items")), "the winners carry nulls in it, not values"


def test_src_48_a_history_load_through_the_sink_keeps_the_left_out_column_in_both_stores(lab: Lab) -> None:
    """`PR #621 review 1 SHOULD-FIX`: a `history` load keeps the column -- its closed
    versions keep their values in it -- proven through `load_via_sink`."""
    plan = LoadPlan(mode="history", key=("a",))
    load_via_sink([{"a": 1, "b": "x", "c": 1.5}, {"a": 2, "b": "y", "c": 2.5}], "h", lab.config, plan)
    result = load_via_sink([{"a": 1, "b": "X"}, {"a": 3, "b": "z"}], "h", lab.config, plan, drop_columns=["c"])
    assert (result.rows, result.has_failed_jobs) == (2, False), "key 1 closed and rewritten, key 3 new"
    assert "c" in _iceberg_names(lab, "h"), "the table keeps the column its old versions carry values in"
    assert "c" in _dlt_columns(lab, "h"), "the stored `dlt` schema keeps it too"
    closed = [row for row in lab.rows("h") if row["b"] == "x"]
    assert closed and closed[0]["c"] == 1.5, "the closed version keeps its value: no data is lost"
