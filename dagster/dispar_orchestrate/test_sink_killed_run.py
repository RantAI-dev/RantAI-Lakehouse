"""A run killed mid-load must not leak into the next one (`SRC-10` third pass,
finding R3; the measurement is case 6 of `test_load_modes_dedup_measurement.py`).

A process that dies between extract and load gives no `except` a chance to abort
its package. Left in place, `dlt` finishes THAT package in the next run and does not
extract the new read, and the next run reports success: for a Kafka micro-batch,
offsets committed for rows never loaded. `load_via_sink` therefore aborts the
pipeline's pending packages when a run STARTS, restoring the pipeline state (cursor,
deduplication marker) to before them. Each test per mode family kills a run after a
first load has put a state in the bucket (without one `dlt` wipes the package itself:
measurement 6, last case), changes the source, runs again and asserts what the
table holds: exactly the source's rows, none lost, none doubled.
"""

from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import Any

import dlt
import pytest
from dlt.pipeline.pipeline import Pipeline

from dispar_orchestrate.adapters import dedup
from dispar_orchestrate.adapters.sink import LoadPlan, load_via_sink
from dispar_orchestrate.test_load_modes_dedup_measurement import Killed, kill_before
from dispar_orchestrate.test_load_modes_measurement import Lab, _items_source, items_db, lab  # noqa: F401 -- fixtures

KILLED_BEFORE = ["normalize", "load"]


def _killed(monkeypatch: pytest.MonkeyPatch, at: str, run: Any) -> None:
    with monkeypatch.context() as dies:
        kill_before(dies, at)
        with pytest.raises(Killed):
            run()
    assert Pipeline is not None  # the patch is undone: the next run uses the real steps


def _values(lab: Lab, table: str) -> dict[Any, str]:
    rows = lab.rows(table)
    assert len({r["id"] for r in rows}) == len(rows), "no id twice"
    return {r["id"]: r["v"] for r in rows}


@pytest.mark.parametrize("at", KILLED_BEFORE)
def test_a_replace_after_a_kill_leaves_exactly_the_source(lab: Lab, monkeypatch: pytest.MonkeyPatch, at: str) -> None:
    def rows(v: str, ids: tuple[int, ...]) -> Any:
        return dlt.resource([{"id": n, "v": v} for n in ids], name="r")

    plan = LoadPlan(mode="replace")
    load_via_sink(rows("old", (1, 2)), "r", lab.config, plan)
    _killed(monkeypatch, at, lambda: load_via_sink(rows("dead", (1, 2, 3)), "r", lab.config, plan))
    result = load_via_sink(rows("new", (2, 3, 4)), "r", lab.config, plan)
    assert _values(lab, "r") == {2: "new", 3: "new", 4: "new"}
    assert result.rows == 3, "the count is of this run's own read"


@pytest.mark.parametrize("at", KILLED_BEFORE)
def test_a_kafka_shaped_append_after_a_kill_loads_its_own_batch_and_not_the_dead_ones(
    lab: Lab, monkeypatch: pytest.MonkeyPatch, at: str
) -> None:
    """The dead batch's offsets were never committed, so the consumer reads it again
    later; the rows of the NEXT batch must be the ones in the table when its call
    returns, because its offsets are committed then."""
    plan = LoadPlan(mode="append")
    load_via_sink([{"id": 0, "v": "first"}], "k", lab.config, plan)
    _killed(monkeypatch, at, lambda: load_via_sink([{"id": n, "v": "dead"} for n in (1, 2, 3)], "k", lab.config, plan))
    result = load_via_sink([{"id": 4, "v": "live"}, {"id": 5, "v": "live"}], "k", lab.config, plan)
    assert _values(lab, "k") == {0: "first", 4: "live", 5: "live"}
    assert result.rows == 2


@pytest.mark.parametrize("at", KILLED_BEFORE)
def test_an_incremental_run_after_a_kill_reads_the_rows_the_dead_run_read_and_the_cursor_has_not_passed_them(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path, monkeypatch: pytest.MonkeyPatch, at: str
) -> None:
    plan = LoadPlan(mode="incremental", cursor="updated_at")
    load_via_sink(_items_source(tmp_path), "items", lab.config, plan)
    items_db.execute("INSERT INTO items VALUES (4,'d',40)")
    items_db.commit()
    _killed(monkeypatch, at, lambda: load_via_sink(_items_source(tmp_path), "items", lab.config, plan))
    items_db.execute("INSERT INTO items VALUES (5,'e',50)")
    items_db.commit()
    result = load_via_sink(_items_source(tmp_path), "items", lab.config, plan)
    assert sorted(_values(lab, "items")) == [1, 2, 3, 4, 5], "row 4 (read by the dead run) and row 5, each once"
    assert result.rows == 2
    # The cursor stands at the last loaded row: nothing is left to read, nothing is read twice.
    again = load_via_sink(_items_source(tmp_path), "items", lab.config, plan)
    assert again.rows == 0 and sorted(_values(lab, "items")) == [1, 2, 3, 4, 5]


@pytest.mark.parametrize("at", KILLED_BEFORE)
def test_a_replace_dedup_after_a_kill_leaves_one_row_per_key_of_the_source_as_it_is_now(
    lab: Lab, monkeypatch: pytest.MonkeyPatch, at: str
) -> None:
    plan = LoadPlan(mode="replace_dedup", key=("id",))
    load_via_sink([{"id": 1, "v": "a"}], "d", lab.config, plan)
    _killed(
        monkeypatch, at, lambda: load_via_sink([{"id": 1, "v": "dead"}, {"id": 1, "v": "dead2"}], "d", lab.config, plan)
    )
    load_via_sink([{"id": 1, "v": "x"}, {"id": 1, "v": "y"}, {"id": 2, "v": "z"}], "d", lab.config, plan)
    assert _values(lab, "d") == {1: "y", 2: "z"}


@pytest.mark.parametrize("at", KILLED_BEFORE)
def test_an_incremental_dedup_merge_after_a_kill_leaves_one_row_per_key_and_the_marker_it_had(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path, monkeypatch: pytest.MonkeyPatch, at: str
) -> None:
    plan = LoadPlan(mode="incremental_dedup", cursor="updated_at", key=("id",))
    source = lambda: _plain_items(tmp_path)  # noqa: E731
    load_via_sink(source(), "items", lab.config, plan)
    items_db.execute("UPDATE items SET v='B', updated_at=40 WHERE id=2")
    items_db.commit()
    _killed(monkeypatch, at, lambda: load_via_sink(source(), "items", lab.config, plan))
    items_db.execute("UPDATE items SET v='BB', updated_at=50 WHERE id=2")
    items_db.execute("INSERT INTO items VALUES (4,'d',60)")
    items_db.commit()
    result = load_via_sink(source(), "items", lab.config, plan)
    assert _values(lab, "items") == {1: "a", 2: "BB", 3: "c", 4: "d"}
    assert result.rows == 2, "the merge read what the cursor of the last LOADED run allows: rows 2 and 4"
    pipeline = lab.pipeline("items")
    pipeline.sync_destination()
    assert dedup.markers(pipeline.state) == [["id"]]


def test_a_first_deduplicating_run_killed_is_still_the_first_run_next_time(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A table loaded with `incremental` (two versions of id 2), then a switch to
    `incremental_dedup` whose first run dies after it extracted: the marker and the
    forgotten cursor are not trusted, the next run replaces and repairs the table."""
    load_via_sink(_plain_items(tmp_path), "items", lab.config, LoadPlan(mode="incremental", cursor="updated_at"))
    items_db.execute("UPDATE items SET v='B', updated_at=40 WHERE id=2")
    items_db.commit()
    load_via_sink(_plain_items(tmp_path), "items", lab.config, LoadPlan(mode="incremental", cursor="updated_at"))
    plan = LoadPlan(mode="incremental_dedup", cursor="updated_at", key=("id",))
    _killed(monkeypatch, "load", lambda: load_via_sink(_plain_items(tmp_path), "items", lab.config, plan))
    load_via_sink(_plain_items(tmp_path), "items", lab.config, plan)
    assert _values(lab, "items") == {1: "a", 2: "B", 3: "c"}


def test_a_pending_package_that_cannot_be_aborted_fails_the_run_instead_of_being_finished(
    lab: Lab, monkeypatch: pytest.MonkeyPatch
) -> None:
    plan = LoadPlan(mode="append")
    load_via_sink([{"id": 0, "v": "first"}], "k", lab.config, plan)
    _killed(monkeypatch, "load", lambda: load_via_sink([{"id": 1, "v": "dead"}], "k", lab.config, plan))

    def cannot(*_a: Any, **_k: Any) -> Any:
        raise OSError("disk")

    monkeypatch.setattr(Pipeline, "abort_packages", cannot)
    with pytest.raises(OSError):
        load_via_sink([{"id": 2, "v": "live"}], "k", lab.config, plan)
    assert _values(lab, "k") == {0: "first"}, "nothing was loaded on top of a package that could not be dropped"


def test_a_run_with_nothing_pending_does_not_call_the_abort(lab: Lab, monkeypatch: pytest.MonkeyPatch) -> None:
    calls: list[int] = []
    original = Pipeline.abort_packages
    monkeypatch.setattr(Pipeline, "abort_packages", lambda self, *a, **k: calls.append(1) or original(self, *a, **k))
    load_via_sink([{"id": 1, "v": "a"}], "k", lab.config, LoadPlan(mode="append"))
    load_via_sink([{"id": 2, "v": "b"}], "k", lab.config, LoadPlan(mode="append"))
    assert calls == []


def _plain_items(tmp_path: Path) -> Any:
    from dlt.sources.sql_database import sql_database

    return sql_database(
        credentials={"drivername": "sqlite", "database": str(tmp_path / "source.db")}, table_names=["items"]
    )
