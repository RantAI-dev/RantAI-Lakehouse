"""Refresh of one table through `sink.load_via_sink` (`SRC-10` decision 5): a plan
with `refresh="keep"` forgets the table's cursor and deduplication marker and
loads by its mode; `refresh="remove"` empties the table first.

Driven on a local catalog and a SQLite source, the harness of
`test_sink_dedup_marker.py` (not Lakekeeper, no query engine).
"""

from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import Any

import pytest
from dlt.sources.sql_database import sql_database

from dispar_orchestrate.adapters import dedup
from dispar_orchestrate.adapters.sink import LoadPlan, load_via_sink
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


def _ids(lab: Lab) -> list[int]:
    return sorted(r["id"] for r in lab.rows("items"))


def _dedup_plan(refresh: str | None = None) -> LoadPlan:
    return LoadPlan(mode="incremental_dedup", cursor="updated_at", key=("id",), refresh=refresh)


def test_refresh_keep_of_a_deduplicating_table_reads_everything_and_leaves_one_row_per_key(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path
) -> None:
    load_via_sink(_source(tmp_path), "items", lab.config, _dedup_plan())
    nothing_new = load_via_sink(_source(tmp_path), "items", lab.config, _dedup_plan())
    assert nothing_new.rows == 0, "the setup: the cursor is past every row"

    refreshed = load_via_sink(_source(tmp_path), "items", lab.config, _dedup_plan("keep"))

    assert refreshed.rows == 3, "the cursor was forgotten, so the whole source was read"
    assert _ids(lab) == [1, 2, 3], "and the table still holds one row per key"
    # The marker is back (the refresh was a first run), and the next ordinary run merges.
    pipeline = lab.pipeline("items")
    pipeline.sync_destination()
    assert dedup.markers(pipeline.state) == [["id"]]
    assert load_via_sink(_source(tmp_path), "items", lab.config, _dedup_plan()).rows == 0


def test_refresh_keep_of_an_incremental_table_replaces_with_the_whole_source(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path
) -> None:
    plan = LoadPlan(mode="incremental", cursor="updated_at")
    load_via_sink(_source(tmp_path), "items", lab.config, plan)
    items_db.execute("INSERT INTO items VALUES (4,'d',40)")
    items_db.commit()

    refreshed = load_via_sink(
        _source(tmp_path), "items", lab.config, LoadPlan(mode="incremental", cursor="updated_at", refresh="keep")
    )

    assert refreshed.rows == 4
    assert _ids(lab) == [1, 2, 3, 4]


def test_refresh_keep_of_an_append_table_adds_the_whole_source_on_top_of_what_is_there(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path
) -> None:
    """For `append`, "keeping" means duplicates of what is already there: the
    same as Airbyte's "refresh, retain"."""
    load_via_sink(_source(tmp_path), "items", lab.config, LoadPlan(mode="append"))
    load_via_sink(_source(tmp_path), "items", lab.config, LoadPlan(mode="append", refresh="keep"))
    assert _ids(lab) == [1, 1, 2, 2, 3, 3]


def test_a_refresh_of_a_deduplicating_table_is_a_first_run_so_a_row_deleted_at_the_source_is_gone(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path
) -> None:
    """Measured: for a deduplicating mode, "keeping" already replaces with the
    deduplicated source (the first-run rule), so it and "removing" end in the same
    rows; only `append` differs between them."""
    for refresh in ("keep", "remove"):
        items_db.execute("INSERT OR REPLACE INTO items VALUES (2,'b',20)")
        items_db.commit()
        load_via_sink(_source(tmp_path), "items", lab.config, _dedup_plan("remove"))
        assert _ids(lab) == [1, 2, 3]
        items_db.execute("DELETE FROM items WHERE id = 2")
        items_db.commit()

        result = load_via_sink(_source(tmp_path), "items", lab.config, _dedup_plan(refresh))

        assert result.rows == 2, refresh
        assert _ids(lab) == [1, 3], refresh


def test_refresh_remove_of_an_append_table_leaves_one_copy_of_the_source(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path
) -> None:
    load_via_sink(_source(tmp_path), "items", lab.config, LoadPlan(mode="append"))
    load_via_sink(_source(tmp_path), "items", lab.config, LoadPlan(mode="append"))
    assert len(_ids(lab)) == 6
    load_via_sink(_source(tmp_path), "items", lab.config, LoadPlan(mode="append", refresh="remove"))
    assert _ids(lab) == [1, 2, 3]


def test_refresh_remove_of_a_source_that_is_now_empty_empties_the_table(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path
) -> None:
    """Why the table is emptied first and not left to `replace`: a source with no
    rows sends nothing to the destination, and a replace of nothing leaves the
    old rows (`test_sink_empty_table.py`)."""
    load_via_sink(_source(tmp_path), "items", lab.config, LoadPlan(mode="replace"))
    items_db.execute("DELETE FROM items")
    items_db.commit()
    load_via_sink(_source(tmp_path), "items", lab.config, LoadPlan(mode="replace", refresh="remove"))
    assert _ids(lab) == []
