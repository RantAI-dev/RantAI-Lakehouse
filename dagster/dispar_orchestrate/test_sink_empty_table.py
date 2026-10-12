"""Measurement for `SRC-10` decision 5 (refresh and clear): how a Bronze Iceberg
table is emptied through the loader's own path.

The question, asked before anything was built (plan task 5): does
`load_via_sink` over an empty list of rows, under `replace`, empty the table and
KEEP its columns, its day partition on `_ingested_at` and `format-version` 2, so
that the next load meets the same table? `sink.py`'s docstring says "An empty
source empties the table", and the second pass saw it for a bare table; this file
asserts it on a table a `sql_database` source created, which is what a clear meets.
Also measured: an empty `dlt` RESOURCE does NOT empty it (so it is the wrong form),
and a table that was never loaded survives being emptied.

The tests drive `sink.load_via_sink` itself on a local catalog (the harness of
`test_load_modes_measurement.py`; same limits: not Lakekeeper, no query engine).
Observations of `dlt` 1.30.0 and `pyiceberg` 0.12.0.
"""

from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import Any

import dlt
import pytest
from dlt.sources.sql_database import sql_database

from dispar_orchestrate.adapters import dedup
from dispar_orchestrate.adapters.sink import LoadPlan, clear_table, load_via_sink
from dispar_orchestrate.test_load_modes_measurement import Lab, lab  # noqa: F401 -- `lab` is a fixture


@pytest.fixture
def items_db(tmp_path: Path) -> sqlite3.Connection:
    db = sqlite3.connect(tmp_path / "source.db")
    db.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, v TEXT, updated_at INTEGER)")
    db.executemany("INSERT INTO items VALUES (?,?,?)", [(1, "a", 10), (2, "b", 20), (3, "c", 30)])
    db.commit()
    return db


def _source(tmp_path: Path) -> Any:
    return sql_database(
        credentials={"drivername": "sqlite", "database": str(tmp_path / "source.db")}, table_names=["items"]
    )


def _shape(lab: Lab, name: str) -> tuple[list[tuple[str, str]], list[str], str]:
    """Columns (name, type), partition fields and format version: what a later
    load meets."""
    table = lab.table(name)
    columns = [(f.name, str(f.field_type)) for f in table.schema().fields if not f.name.startswith("_dlt_")]
    return (
        columns,
        [f.name for f in table.spec().fields],
        table.metadata.properties.get("format-version", str(table.metadata.format_version)),
    )


def test_a_replace_of_an_empty_list_empties_the_table_and_keeps_its_shape(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path
) -> None:
    load_via_sink(_source(tmp_path), "items", lab.config, LoadPlan(mode="replace"))
    assert len(lab.rows("items")) == 3
    before = _shape(lab, "items")
    assert before[1] == ["_ingested_at_day"], "the setup: the day partition is there"

    clear_table("items", lab.config)

    assert lab.rows("items") == []
    assert _shape(lab, "items") == before, "same columns, same partition spec, same format version"
    assert lab.table("items").metadata.format_version == 2


def test_the_emptied_table_takes_the_next_load_as_any_other(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path
) -> None:
    load_via_sink(_source(tmp_path), "items", lab.config, LoadPlan(mode="replace"))
    clear_table("items", lab.config)
    result = load_via_sink(_source(tmp_path), "items", lab.config, LoadPlan(mode="append"))
    assert result.rows == 3
    assert sorted(r["id"] for r in lab.rows("items")) == [1, 2, 3]


def test_an_empty_list_through_load_via_sink_does_not_empty_a_table_a_sql_source_created(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path
) -> None:
    """Why `clear_table` runs the pipeline itself: the form the docstring of
    `sink.py` suggests ("an empty source empties the table") left all three rows in
    place over a table `sql_database` created. (Over a table plain rows created it
    did empty it: `test_sink_dedup_modes.py`.)"""
    load_via_sink(_source(tmp_path), "items", lab.config, LoadPlan(mode="replace"))
    load_via_sink([], "items", lab.config, LoadPlan(mode="replace"))
    assert len(lab.rows("items")) == 3


def test_an_empty_dlt_resource_does_not_empty_the_table(lab: Lab) -> None:
    """The form to avoid: no data reaches the destination, so nothing is replaced."""
    columns = {"id": {"data_type": "bigint"}, "v": {"data_type": "text"}}
    load_via_sink([{"id": 1, "v": "a"}], "t", lab.config, LoadPlan(mode="replace"))
    load_via_sink(dlt.resource([], name="t", columns=columns), "t", lab.config, LoadPlan(mode="replace"))
    assert len(lab.rows("t")) == 1


def test_clearing_a_table_that_was_never_loaded_leaves_nothing_and_fails_nothing(lab: Lab) -> None:
    clear_table("never_loaded", lab.config)
    clear_table("never_loaded", lab.config)  # and again: idempotent


def test_clear_table_empties_the_table_and_forgets_the_cursor_and_the_marker(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path
) -> None:
    plan = LoadPlan(mode="incremental_dedup", cursor="updated_at", key=("id",))
    load_via_sink(_source(tmp_path), "items", lab.config, plan)
    pipeline = lab.pipeline("items")
    pipeline.sync_destination()
    assert dedup.markers(pipeline.state) == [["id"]], "the setup: a marker and a cursor"
    before = _shape(lab, "items")

    clear_table("items", lab.config)

    assert lab.rows("items") == []
    assert _shape(lab, "items") == before
    pipeline = lab.pipeline("items")
    pipeline.sync_destination()
    assert dedup.markers(pipeline.state) == []
    from dispar_orchestrate.adapters.sink import _incremental_cursors

    assert _incremental_cursors(pipeline) == {}
    # The next run of the mode reads everything again (nothing to continue from).
    again = load_via_sink(_source(tmp_path), "items", lab.config, plan)
    assert again.rows == 3
    assert sorted(r["id"] for r in lab.rows("items")) == [1, 2, 3]
