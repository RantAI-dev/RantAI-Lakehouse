"""Measurement and tests for `SRC-10` decision 5 (refresh and clear): how a Bronze
Iceberg table is emptied, for EVERY origin of the table.

The question (plan task 5): which way empties the table and KEEPS its columns, its day
partition on `_ingested_at` and `format-version` 2, so that the next load meets the same
table? Fourth pass: a `dlt` run of no rows. Seventh pass (`SRC-10 review BLOCKER 4`): that
way empties a table by accident of a NAME. `dlt` writes the empty items file that
truncates a replace table only for a replace resource named as the stored schema records
it for the table (`dlt/extract/extract.py::_handle_empty_tables`); `sql_database` names its
resource like the table, plain rows are recorded under the generator's name `_stamp_rows`.
So `pipeline.run([], table_name=T)` emptied a SQL table and not a plain-rows one, and
`load_via_sink([], ...)` did the opposite. `sink.empty_table` is a `pyiceberg` delete and
does not depend on the name; the tests below run it for each origin (plain rows, a
`sql_database` source, the history mode) and for each caller (`clear_table`, a refresh
that removes rows in every load mode).

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


def test_clear_table_empties_a_sql_source_table_and_keeps_its_shape(
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
    """The cause, half one: the empty-source form `sink.py` once suggested left all
    three rows in place over a table `sql_database` created, because the empty
    resource is named `_stamp_rows` and the stored schema says `items`."""
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


# --- the cause, and the one way, for every origin (seventh pass, SRC-10 review BLOCKER 4) ----


def test_a_dlt_run_of_no_rows_does_not_empty_a_table_made_from_plain_rows(lab: Lab) -> None:
    """The cause, half two: `pipeline.run([], table_name=T)` names its resource `T`, the
    stored schema records `_stamp_rows` for the table, so `dlt` extracts nothing ("No data
    found to normalize") and writes no empty items file. This is what `clear_table` was
    until the seventh pass."""
    load_via_sink([{"id": 1, "v": "a"}], "plain", lab.config, LoadPlan(mode="replace"))
    pipeline = lab.pipeline("plain")
    pipeline.sync_destination()
    assert pipeline.default_schema.tables["plain"]["resource"] == "_stamp_rows"
    pipeline.run([], table_name="plain", table_format="iceberg", write_disposition="replace")
    assert len(lab.rows("plain")) == 1


ORIGINS = ("plain", "sql", "history")


def _write_source(tmp_path: Path, rows: list[tuple[int, str, int]]) -> None:
    db = sqlite3.connect(tmp_path / "source.db")
    db.execute("DROP TABLE IF EXISTS items")
    db.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, v TEXT, updated_at INTEGER)")
    db.executemany("INSERT INTO items VALUES (?,?,?)", rows)
    db.commit()
    db.close()


def _rows_source(origin: str, tmp_path: Path, rows: list[tuple[int, str, int]]) -> Any:
    """What the connector would hand the sink for `rows`: a list of dicts (files, REST,
    MongoDB, SFTP) or a `sql_database` source."""
    if origin == "sql":
        _write_source(tmp_path, rows)
        return _source(tmp_path)
    return [{"id": i, "v": v, "updated_at": u} for i, v, u in rows]


def _origin_plan(origin: str, refresh: str | None = None) -> LoadPlan:
    return LoadPlan(mode="history", key=("id",), refresh=refresh) if origin == "history" else LoadPlan(
        mode="replace", refresh=refresh
    )


FIRST = [(1, "a", 10), (2, "b", 20), (3, "c", 30)]
SECOND = [(2, "B", 25), (3, "c", 30), (4, "d", 40)]


@pytest.mark.parametrize("origin", ORIGINS)
def test_clear_table_empties_a_table_of_every_origin_and_the_next_load_works(
    lab: Lab, tmp_path: Path, origin: str
) -> None:
    load_via_sink(_rows_source(origin, tmp_path, FIRST), "t", lab.config, _origin_plan(origin))
    assert len(lab.rows("t")) == 3
    before = _shape(lab, "t")
    assert before[1] == ["_ingested_at_day"]

    clear_table("t", lab.config)

    assert lab.rows("t") == []
    assert _shape(lab, "t") == before, "same columns, same partition spec, same format version"
    assert lab.table("t").metadata.format_version == 2
    clear_table("t", lab.config)  # twice is fine
    assert lab.rows("t") == []

    again = load_via_sink(_rows_source(origin, tmp_path, SECOND), "t", lab.config, _origin_plan(origin))
    assert again.rows == 3
    assert sorted(r["id"] for r in lab.rows("t")) == [2, 3, 4]
    assert _shape(lab, "t") == before


# Every mode family, with the origins it accepts (an incremental mode takes a `sql_database` source only).
_FAMILIES = [
    ("plain", "replace"),
    ("plain", "append"),
    ("plain", "replace_dedup"),
    ("plain", "history"),
    ("sql", "replace"),
    ("sql", "append"),
    ("sql", "incremental"),
    ("sql", "incremental_dedup"),
    ("sql", "replace_dedup"),
    ("sql", "history"),
]


def _family_plan(mode: str, refresh: str | None = None) -> LoadPlan:
    return LoadPlan(
        mode=mode,
        cursor="updated_at" if mode.startswith("incremental") else None,
        key=("id",) if mode in ("incremental_dedup", "replace_dedup", "history") else (),
        refresh=refresh,
    )


@pytest.mark.parametrize(("origin", "mode"), _FAMILIES)
def test_refresh_remove_leaves_exactly_the_sources_rows_in_every_mode_and_origin(
    lab: Lab, tmp_path: Path, origin: str, mode: str
) -> None:
    load_via_sink(_rows_source(origin, tmp_path, FIRST), "t", lab.config, _family_plan(mode))
    assert len(lab.rows("t")) >= 3
    before = _shape(lab, "t")

    result = load_via_sink(_rows_source(origin, tmp_path, SECOND), "t", lab.config, _family_plan(mode, "remove"))

    # The source's rows and nothing of the first load: id 1 is gone, id 2 has its new value
    # once, whatever the mode would have done with a plain run (append would stack them).
    assert sorted((r["id"], r["v"]) for r in lab.rows("t")) == [(2, "B"), (3, "c"), (4, "d")]
    assert result.rows == 3
    assert _shape(lab, "t") == before
    if mode == "history":
        assert all(r["_is_current"] and r["_valid_to"] is None for r in lab.rows("t"))


def test_clearing_a_table_that_was_never_loaded_creates_nothing(lab: Lab) -> None:
    clear_table("never_loaded", lab.config)
    clear_table("never_loaded", lab.config)
    assert list(lab.bucket.glob("**/never_loaded")) == [], "no table, no directory"
    assert load_via_sink(
        [{"id": 1, "v": "a"}], "never_loaded", lab.config, LoadPlan(mode="replace", refresh="remove")
    ).rows == 1, "and a refresh that removes rows loads a table that was not there"


# --- an ordinary replace of a source that now returns nothing (seventh pass, item 4) ----------


@pytest.mark.parametrize("mode", ["replace", "replace_dedup"])
@pytest.mark.parametrize("origin", ["plain", "sql"])
def test_a_replace_of_a_source_that_now_returns_no_rows_leaves_the_table_empty(
    lab: Lab, tmp_path: Path, origin: str, mode: str
) -> None:
    """What "replace" promises, measured to hold already (the fifth pass's note that a
    SQL table empty at the source kept its rows could not be reproduced: that was
    `load_via_sink([])`, whose resource name differs). The resource of each run keeps
    the name the stored schema records, so `dlt` truncates. The trade-off is the one
    every `replace` has: a read that is wrongly empty empties the table."""
    plan = LoadPlan(mode=mode, key=("id",) if mode == "replace_dedup" else ())
    load_via_sink(_rows_source(origin, tmp_path, FIRST), "t", lab.config, plan)
    assert len(lab.rows("t")) == 3
    before = _shape(lab, "t")

    load_via_sink(_rows_source(origin, tmp_path, []), "t", lab.config, plan)

    assert lab.rows("t") == []
    assert _shape(lab, "t") == before
    assert load_via_sink(_rows_source(origin, tmp_path, SECOND), "t", lab.config, plan).rows == 3
