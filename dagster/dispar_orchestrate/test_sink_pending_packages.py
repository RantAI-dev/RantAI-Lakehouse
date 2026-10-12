"""A failed load must not block the next one (`SRC-10` second pass, planner
decision 4; the finding of `SRC-8` that a failed package keeps every later load of
the table failing).

`load_via_sink` aborts the pipeline's pending packages when a load raises or ends
with failed jobs, for every mode. Each test fails one load at the write (the
catalog or bucket error, outlasting `dlt`'s retries: `_Failure`), then runs a clean
load of the same table and asserts what the table holds: no row lost, none doubled.
The measurement behind it is `test_load_modes_dedup_measurement.py` (cases 4);
here the loader is what is tested.
"""

from __future__ import annotations

import sqlite3
from pathlib import Path
from types import SimpleNamespace
from typing import Any

import dlt
import pytest
from dlt.pipeline.exceptions import PipelineStepFailed
from dlt.pipeline.pipeline import Pipeline
from pyiceberg.table import Table

from dispar_orchestrate.adapters import sink as sink_module
from dispar_orchestrate.adapters.sink import LoadPlan, load_via_sink
from dispar_orchestrate.test_load_modes_dedup_measurement import _Failure
from dispar_orchestrate.test_load_modes_measurement import Lab, _items_source, items_db, lab  # noqa: F401 -- fixtures


def _ids(lab: Lab, table: str) -> list[Any]:
    return sorted(r["id"] for r in lab.rows(table))


def _values(lab: Lab, table: str) -> dict[Any, str]:
    return {r["id"]: r["v"] for r in lab.rows(table)}


def _nothing_pending(lab: Lab, table: str) -> bool:
    return not lab.pipeline(table).has_pending_data


def test_a_replace_that_fails_leaves_nothing_pending_and_the_next_replace_succeeds(
    lab: Lab, monkeypatch: pytest.MonkeyPatch
) -> None:
    rows = lambda v: dlt.resource([{"id": 1, "v": v}, {"id": 2, "v": v}], name="r")  # noqa: E731
    load_via_sink(rows("old"), "r", lab.config, LoadPlan(mode="replace"))
    failure = _Failure(monkeypatch, Table, "overwrite")
    with pytest.raises(PipelineStepFailed):
        load_via_sink(rows("new"), "r", lab.config, LoadPlan(mode="replace"))
    assert _nothing_pending(lab, "r")
    assert set(_values(lab, "r").values()) == {"old"}, "the failed replace changed nothing"
    failure.heal()
    result = load_via_sink(rows("new"), "r", lab.config, LoadPlan(mode="replace"))
    assert set(_values(lab, "r").values()) == {"new"} and _ids(lab, "r") == [1, 2]
    assert result.rows == 2, "the count is of THIS run's read, not of a package a dead run left"


def test_an_append_that_fails_leaves_nothing_pending_and_the_next_append_loses_and_doubles_no_row(
    lab: Lab, monkeypatch: pytest.MonkeyPatch
) -> None:
    load_via_sink([{"id": 1, "v": "a"}], "ap", lab.config, LoadPlan(mode="append"))
    failure = _Failure(monkeypatch, Table, "append")
    with pytest.raises(PipelineStepFailed):
        load_via_sink([{"id": 2, "v": "b"}], "ap", lab.config, LoadPlan(mode="append"))
    assert _nothing_pending(lab, "ap")
    assert _ids(lab, "ap") == [1]
    failure.heal()
    result = load_via_sink([{"id": 2, "v": "b"}], "ap", lab.config, LoadPlan(mode="append"))
    assert _ids(lab, "ap") == [1, 2]
    assert result.rows == 1


def test_a_first_ever_load_that_fails_does_not_block_the_next(lab: Lab, monkeypatch: pytest.MonkeyPatch) -> None:
    failure = _Failure(monkeypatch, Table, "append")
    with pytest.raises(PipelineStepFailed):
        load_via_sink([{"id": 1, "v": "a"}], "fresh", lab.config, LoadPlan(mode="append"))
    failure.heal()
    load_via_sink([{"id": 1, "v": "a"}], "fresh", lab.config, LoadPlan(mode="append"))
    assert _ids(lab, "fresh") == [1]


def test_an_incremental_load_that_fails_does_not_advance_the_cursor_past_rows_never_loaded(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    plan = LoadPlan(mode="incremental", cursor="updated_at")
    load_via_sink(_items_source(tmp_path), "items", lab.config, plan)
    assert _ids(lab, "items") == [1, 2, 3]
    items_db.execute("INSERT INTO items VALUES (4,'d',40)")
    items_db.execute("INSERT INTO items VALUES (5,'e',50)")
    items_db.commit()
    failure = _Failure(monkeypatch, Table, "append")
    with pytest.raises(PipelineStepFailed):
        load_via_sink(_items_source(tmp_path), "items", lab.config, plan)
    assert _nothing_pending(lab, "items")
    failure.heal()
    result = load_via_sink(_items_source(tmp_path), "items", lab.config, plan)
    assert _ids(lab, "items") == [1, 2, 3, 4, 5], "rows 4 and 5 are loaded, each once"
    assert result.rows == 2


def test_a_merge_that_fails_leaves_nothing_pending_and_the_next_merge_completes_it_with_one_row_per_key(
    lab: Lab, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    db = sqlite3.connect(tmp_path / "source.db")
    db.execute("CREATE TABLE events (id INTEGER, v TEXT, updated_at INTEGER)")
    db.executemany("INSERT INTO events VALUES (?,?,?)", [(1, "a", 1), (2, "b", 2)])
    db.commit()
    plan = LoadPlan(mode="incremental_dedup", cursor="updated_at", key=("id",))

    def source() -> Any:
        from dlt.sources.sql_database import sql_database

        return sql_database(
            credentials={"drivername": "sqlite", "database": str(tmp_path / "source.db")}, table_names=["events"]
        )

    load_via_sink(source(), "events", lab.config, plan)
    db.execute("UPDATE events SET v='B', updated_at=20 WHERE id=2")
    db.execute("INSERT INTO events VALUES (3, 'c', 30)")
    db.commit()
    failure = _Failure(monkeypatch, Table, "upsert")
    with pytest.raises(PipelineStepFailed):
        load_via_sink(source(), "events", lab.config, plan)
    assert _nothing_pending(lab, "events")
    assert _values(lab, "events") == {1: "a", 2: "b"}
    failure.heal()
    load_via_sink(source(), "events", lab.config, plan)
    assert _values(lab, "events") == {1: "a", 2: "B", 3: "c"} and len(lab.rows("events")) == 3


def test_a_replace_dedup_that_fails_leaves_nothing_pending_and_the_next_one_succeeds(
    lab: Lab, monkeypatch: pytest.MonkeyPatch
) -> None:
    plan = LoadPlan(mode="replace_dedup", key=("id",))
    load_via_sink([{"id": 1, "v": "old"}], "d", lab.config, plan)
    failure = _Failure(monkeypatch, Table, "overwrite")
    with pytest.raises(PipelineStepFailed):
        load_via_sink([{"id": 1, "v": "x"}, {"id": 1, "v": "new"}], "d", lab.config, plan)
    assert _nothing_pending(lab, "d") and _values(lab, "d") == {1: "old"}
    failure.heal()
    load_via_sink([{"id": 1, "v": "x"}, {"id": 1, "v": "new"}], "d", lab.config, plan)
    assert _values(lab, "d") == {1: "new"}


def test_a_failure_to_abort_never_masks_the_failure_of_the_load(
    lab: Lab, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    _Failure(monkeypatch, Table, "append")

    def broken(self: Pipeline, *args: Any, **kwargs: Any) -> None:
        raise OSError("SECRET-PATH-/var/dlt/leak")

    monkeypatch.setattr(Pipeline, "abort_packages", broken)
    with pytest.raises(PipelineStepFailed):
        load_via_sink([{"id": 1, "v": "a"}], "m", lab.config, LoadPlan(mode="append"))
    assert "SECRET-PATH" not in caplog.text, "the log names the exception type only"
    assert "OSError" in caplog.text


def test_a_load_that_ends_with_failed_jobs_aborts_what_is_pending() -> None:
    aborted: list[bool] = []
    pipeline = SimpleNamespace(
        pipeline_name="p", has_pending_data=True, abort_packages=lambda: aborted.append(True)
    )
    sink_module._abort_pending(pipeline)
    assert aborted == [True]
    idle = SimpleNamespace(pipeline_name="p", has_pending_data=False, abort_packages=lambda: aborted.append(False))
    sink_module._abort_pending(idle)
    assert aborted == [True], "nothing pending, nothing aborted"


def test_a_run_that_only_finished_an_old_package_has_no_row_count_rather_than_a_crash() -> None:
    # `pipeline.last_trace.last_normalize_info` is None when no normalize step ran
    # (SRC-10 measurement 4); `_extract_rows` used to raise AttributeError there.
    trace = SimpleNamespace(last_normalize_info=None)
    assert sink_module._extract_rows(trace, "t") is None
    assert sink_module._extract_rows(None, "t") is None


def test_a_failure_in_a_later_slice_of_a_2500_row_incremental_dedup_merge_is_completed_by_the_next_run(
    lab: Lab, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Planner decision 5 through the loader: a merge above 1000 rows is several
    commits (`dlt` upserts it in slices of 1000). Each commit leaves one row per key;
    the run that fails in the third slice leaves two committed slices; and because
    the abort restores the cursor, the next run re-reads all 2,500 rows and brings
    the table to the source's state."""
    from dlt.sources.sql_database import sql_database

    db = sqlite3.connect(tmp_path / "source.db")
    db.execute("CREATE TABLE events (id INTEGER, v TEXT, updated_at INTEGER)")
    db.executemany("INSERT INTO events VALUES (?,?,?)", [(n, "old", n) for n in range(2500)])
    db.commit()
    plan = LoadPlan(mode="incremental_dedup", cursor="updated_at", key=("id",))

    def run() -> Any:
        source = sql_database(
            credentials={"drivername": "sqlite", "database": str(tmp_path / "source.db")}, table_names=["events"]
        )
        return load_via_sink(source, "events", lab.config, plan)

    run()
    assert {r["v"] for r in lab.rows("events")} == {"old"}
    commits = lab.commits("events")
    db.execute("UPDATE events SET v='new', updated_at=updated_at + 10000")
    db.commit()
    failure = _Failure(monkeypatch, Table, "upsert", from_call=3)
    with pytest.raises(PipelineStepFailed):
        run()
    rows = lab.rows("events")
    assert lab.commits("events") - commits == 2
    assert len(rows) == 2500 and len({r["id"] for r in rows}) == 2500, "one row per key after the partial merge"
    assert {r["v"] for r in rows} == {"old", "new"}
    failure.heal()
    result = run()
    rows = lab.rows("events")
    assert len(rows) == 2500 and {r["v"] for r in rows} == {"new"}
    assert result.rows == 2500, "the aborted run's cursor was restored, so the next run re-read all 2500"
