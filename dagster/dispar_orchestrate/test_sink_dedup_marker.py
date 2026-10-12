"""The marker a deduplicating mode leaves in the pipeline state (`SRC-10` third
pass, finding R2): which key columns the table was last deduplicated on.

Why: a table loaded with `incremental` on a cursor has cursor state AND several
versions of a key. The first run of `incremental_dedup` used to find that cursor
state, merge, and leave the old versions silently (a merge updates every copy, it
never removes one: `test_load_modes_dedup_measurement.py`, case 3). Now the first
run under a deduplicating mode, or under one with another key, reads everything and
replaces the table with the winners. These tests drive `sink.load_via_sink` itself
on a local catalog and a SQLite source (the harness of `test_load_modes_measurement.py`).
"""

from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import Any

import dlt
import pytest
from dlt.pipeline.exceptions import PipelineStepFailed
from dlt.sources.sql_database import sql_database
from pyiceberg.table import Table

from dispar_orchestrate.adapters import dedup
from dispar_orchestrate.adapters.sink import LoadPlan, load_via_sink
from dispar_orchestrate.test_load_modes_dedup_measurement import _Failure
from dispar_orchestrate.test_load_modes_measurement import Lab, lab  # noqa: F401 -- `lab` is a fixture


@pytest.fixture
def items_db(tmp_path: Path) -> sqlite3.Connection:
    db = sqlite3.connect(tmp_path / "source.db")
    db.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, v TEXT, updated_at INTEGER)")
    db.executemany("INSERT INTO items VALUES (?,?,?)", [(1, "a", 10), (2, "b", 20), (3, "c", 30)])
    db.commit()
    return db


def _source(tmp_path: Path) -> Any:
    return sql_database(credentials={"drivername": "sqlite", "database": str(tmp_path / "source.db")}, table_names=["items"])


def _run(lab: Lab, tmp_path: Path, mode: str, key: tuple[str, ...] = (), cursor: str | None = "updated_at") -> Any:
    return load_via_sink(_source(tmp_path), "items", lab.config, LoadPlan(mode=mode, cursor=cursor, key=key))


def _markers(lab: Lab) -> list[list[str]]:
    """What a NEW pipeline object sees: the state is read back from the bucket."""
    pipeline = lab.pipeline("items")
    pipeline.sync_destination()
    return dedup.markers(pipeline.state)


def _versions(lab: Lab) -> dict[int, list[str]]:
    out: dict[int, list[str]] = {}
    for row in lab.rows("items"):
        out.setdefault(row["id"], []).append(row["v"])
    return {k: sorted(v) for k, v in out.items()}


def _two_versions_of_key_2(lab: Lab, items_db: sqlite3.Connection, tmp_path: Path) -> None:
    _run(lab, tmp_path, "incremental")
    items_db.execute("UPDATE items SET v='B', updated_at=40 WHERE id=2")
    items_db.commit()
    _run(lab, tmp_path, "incremental")
    assert _versions(lab)[2] == ["B", "b"], "the setup: an old and a new version of id 2"


def test_switching_incremental_to_incremental_dedup_replaces_on_the_first_run_and_merges_from_the_second(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path
) -> None:
    _two_versions_of_key_2(lab, items_db, tmp_path)
    assert _markers(lab) == []
    first = _run(lab, tmp_path, "incremental_dedup", ("id",))
    assert _versions(lab) == {1: ["a"], 2: ["B"], 3: ["c"]}, "run 1 leaves one row per key"
    assert first.rows == 3, "run 1 read everything, not just what the old cursor allowed"
    assert _markers(lab) == [["id"]]
    commits = lab.commits("items")
    items_db.execute("UPDATE items SET v='BB', updated_at=50 WHERE id=2")
    items_db.execute("INSERT INTO items VALUES (4,'d',60)")
    items_db.commit()
    second = _run(lab, tmp_path, "incremental_dedup", ("id",))
    assert second.rows == 2
    assert _versions(lab) == {1: ["a"], 2: ["BB"], 3: ["c"], 4: ["d"]}
    assert lab.commits("items") - commits == 1, "run 2 is one merge, not a replace"


def test_a_change_of_key_columns_replaces_the_table_again(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path
) -> None:
    _run(lab, tmp_path, "incremental_dedup", ("id",))
    assert _markers(lab) == [["id"]]
    # The same key again merges: nothing new, no commit.
    commits = lab.commits("items")
    _run(lab, tmp_path, "incremental_dedup", ("id",))
    assert lab.commits("items") == commits
    # Another key: every row is read again and the table replaced under the new key.
    items_db.execute("INSERT INTO items VALUES (4,'a',5)")  # shares v='a' with id 1, below the cursor
    items_db.commit()
    result = _run(lab, tmp_path, "incremental_dedup", ("v",))
    assert result.rows == 3, "four source rows, one per value of v"
    assert sorted(r["v"] for r in lab.rows("items")) == ["a", "b", "c"]
    assert _markers(lab) == [["v"]]


def test_switching_back_to_incremental_or_append_clears_the_marker(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path
) -> None:
    _run(lab, tmp_path, "incremental_dedup", ("id",))
    assert _markers(lab) == [["id"]]
    items_db.execute("UPDATE items SET v='B', updated_at=40 WHERE id=2")
    items_db.commit()
    _run(lab, tmp_path, "incremental")
    assert _markers(lab) == [], "incremental wrote a second version of id 2: the table is no longer deduplicated"
    assert _versions(lab)[2] == ["B", "b"]
    # And back: the marker is absent, so the next dedup run replaces and repairs it.
    _run(lab, tmp_path, "incremental_dedup", ("id",))
    assert _versions(lab)[2] == ["B"]
    assert _markers(lab) == [["id"]]
    _run(lab, tmp_path, "append")
    assert _markers(lab) == []


def test_replace_dedup_leaves_the_marker_and_replace_clears_it(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path
) -> None:
    _run(lab, tmp_path, "replace_dedup", ("id",), None)
    assert _markers(lab) == [["id"]]
    _run(lab, tmp_path, "replace")
    assert _markers(lab) == []


def test_a_marker_for_plain_rows_is_written_and_cleared_too(lab: Lab) -> None:
    load_via_sink([{"id": 1, "v": "a"}, {"id": 1, "v": "b"}], "p", lab.config, LoadPlan(mode="replace_dedup", key=("id",)))
    pipeline = lab.pipeline("p")
    pipeline.sync_destination()
    assert dedup.markers(pipeline.state) == [["id"]]
    load_via_sink([{"id": 2, "v": "x"}], "p", lab.config, LoadPlan(mode="replace"))
    pipeline = lab.pipeline("p")
    pipeline.sync_destination()
    assert dedup.markers(pipeline.state) == []


def test_the_marker_survives_a_recreated_container(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _run(lab, tmp_path, "incremental_dedup", ("id",))
    # A new container: no local pipeline directory, only the bucket.
    monkeypatch.setenv("DLT_DATA_DIR", str(tmp_path / "another-container"))
    assert _markers(lab) == [["id"]], "read back from the bucket by a new pipeline object"
    before = {r["id"]: r["_ingested_at"] for r in lab.rows("items")}
    commits = lab.commits("items")
    items_db.execute("INSERT INTO items VALUES (4,'d',60)")
    items_db.commit()
    result = _run(lab, tmp_path, "incremental_dedup", ("id",))
    assert result.rows == 1, "continued from the stored cursor and marker: merged, did not replace"
    assert lab.commits("items") - commits == 1
    after = {r["id"]: r["_ingested_at"] for r in lab.rows("items")}
    assert all(after[k] == before[k] for k in before), "the rows of the first run were not rewritten"


def test_a_failed_first_run_is_a_first_run_next_time(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _two_versions_of_key_2(lab, items_db, tmp_path)
    failure = _Failure(monkeypatch, Table, "overwrite")
    with pytest.raises(PipelineStepFailed):
        _run(lab, tmp_path, "incremental_dedup", ("id",))
    assert failure.calls, "the failure was injected at the replace"
    assert _versions(lab)[2] == ["B", "b"], "the failed replace changed nothing"
    assert _markers(lab) == [], "no marker for a run that did not load"
    failure.heal()
    _run(lab, tmp_path, "incremental_dedup", ("id",))
    assert _versions(lab) == {1: ["a"], 2: ["B"], 3: ["c"]}, "the next run replaces again"
    assert _markers(lab) == [["id"]]


def test_abort_packages_restores_the_marker_to_its_value_before_a_failed_run(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The run writes its new key into the state while it extracts, so the marker
    is part of the package; the loader's abort rolls it back. `replace_dedup` never
    forgets the state first, so the marker is there to be restored."""
    _run(lab, tmp_path, "replace_dedup", ("id",), None)
    assert _markers(lab) == [["id"]]
    failure = _Failure(monkeypatch, Table, "overwrite")
    with pytest.raises(PipelineStepFailed):
        _run(lab, tmp_path, "replace_dedup", ("id", "v"), None)
    assert failure.calls
    # Rolled back both locally and in what a new container reads.
    assert dedup.markers(lab.pipeline("items").state) == [["id"]]
    assert _markers(lab) == [["id"]]
    failure.heal()
    _run(lab, tmp_path, "replace_dedup", ("id", "v"), None)
    assert _markers(lab) == [["id", "v"]]


def test_the_marker_is_not_written_by_a_run_that_emitted_no_rows(lab: Lab) -> None:
    columns = {"id": {"data_type": "bigint"}, "v": {"data_type": "text"}}
    empty = dlt.resource([], name="e", columns=columns)
    load_via_sink(empty, "e", lab.config, LoadPlan(mode="replace_dedup", key=("id",)))
    pipeline = lab.pipeline("e")
    pipeline.sync_destination()
    assert dedup.markers(pipeline.state) == [], "an empty read leaves the table as it was, so it cannot make it clean"
