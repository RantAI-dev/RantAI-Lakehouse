"""Measurement behind the deduplicating load modes (`SRC-10`, second pass: planner
decisions 3, 4 and 5 of that pass), in the conventions of
`test_load_modes_measurement.py` (which this module imports its `Lab` and its
`_merge_load` from; read its docstring for what is and is not the production
path). Observations of `dlt` 1.30.0 and `pyiceberg` 0.12.0 on a local Iceberg
catalog; nothing here reaches Lakekeeper, ClickHouse or Trino.

Cases, cited by the loader's comments as `SRC-10 measurement 3/4/5`:

- 3: a table first loaded with `append` that holds a key twice, then replaced
  with the deduplicated rows (what the first run of a deduplicating mode does),
  merges cleanly from the second run on; and the cursor a replacing first run
  leaves is the one the next run continues from.
- 4: what `Pipeline.abort_packages` (`drop_pending_packages` is deprecated in
  1.30.0 and only calls it) does after a load that failed in each mode family,
  and to the incremental cursor.
- 5: a merge above 1000 rows is several commits; a failure part-way leaves one
  row per key, and the next clean run completes it.
- 6 (third pass, finding R3): what the next run does with a package left by a
  PROCESS that died mid-load, which no `except` ran for. Simulated by raising a
  `BaseException` out of `Pipeline.load` (or `normalize`): `load_via_sink` only
  catches `Exception`, exactly as a kill gives it no chance to abort anything.
"""

from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import Any

import dlt
import pytest
from dlt.common.destination.exceptions import DestinationTerminalException
from dlt.pipeline.exceptions import PipelineStepFailed
from dlt.pipeline.pipeline import Pipeline
from pyiceberg.table import Table, Transaction

from dispar_orchestrate.adapters import sink as sink_module
from dispar_orchestrate.test_load_modes_measurement import (  # noqa: F401 -- `lab` and `items_db` are fixtures
    Lab,
    _dedup,
    _items_source,
    _load,
    _merge_load,
    items_db,
    lab,
)

# Runs a real dlt pipeline against a local Iceberg table (registered in pyproject.toml);
# CI runs these in `dagster-loader-tests`, the rest in `dagster-unit-tests`.
pytestmark = pytest.mark.loader


class _Failure:
    """A failure injected into `owner.name` from its `from_call`-th call on, until
    `heal()`: a transient failure of the catalog or the bucket that outlasts
    `dlt`'s retries of a load job (a failure of one call alone is retried and the
    load then succeeds, which is not the case measured here)."""

    def __init__(self, monkeypatch: pytest.MonkeyPatch, owner: type, name: str, from_call: int = 1) -> None:
        self.calls = 0
        self.active = True
        if owner is Table and name in ("append", "overwrite"):
            # PR #186 review BLOCKER 6: the write of a `replace` or `append` is now staged in
            # one `pyiceberg` transaction (`adapters/iceberg_stream.py`), and `Table.append`
            # and `Table.overwrite` are only a transaction around the same two calls, so the
            # failure is injected where both paths meet.
            # `DATA-36`: the groups of a load are written into ONE snapshot producer
            # (`adapters/iceberg_snapshot.py`), so neither `Transaction.append` nor
            # `Transaction.overwrite` is called any more; the paths meet now in the one call
            # that opens the producer of the load's one snapshot. The failure it injects is
            # the same.
            owner, name = Transaction, "update_snapshot"
        original = getattr(owner, name)

        def wrapper(*args: Any, **kwargs: Any) -> Any:
            self.calls += 1
            if self.active and self.calls >= from_call:
                raise DestinationTerminalException("injected failure")
            return original(*args, **kwargs)

        monkeypatch.setattr(owner, name, wrapper)

    def heal(self) -> None:
        self.active = False


class Killed(BaseException):
    """The process died here. A `BaseException`, so no `except Exception` of the
    loader runs, as for a SIGKILL; `at` is the `Pipeline` step it dies in front of."""


def kill_before(monkeypatch: pytest.MonkeyPatch, at: str) -> None:
    def die(*_args: Any, **_kwargs: Any) -> Any:
        raise Killed(at)

    monkeypatch.setattr(Pipeline, at, die)


def _v(lab: Lab, table: str) -> dict[Any, str]:
    return {r["id"]: r["v"] for r in lab.rows(table)}


# ---------------------------------------------------------------------------
# Measurement 3: a table that already holds duplicates
# ---------------------------------------------------------------------------


def test_3_a_replace_with_the_deduplicated_rows_cleans_an_append_table_and_the_next_merge_succeeds(lab: Lab) -> None:
    # Two runs of `append`, as a real table gets them: the copies of key 1 sit in
    # different data files (see the next case for why that matters).
    _load(lab, [{"id": 1, "v": "a"}, {"id": 2, "v": "b"}], "g", "append")
    _load(lab, [{"id": 1, "v": "a2"}], "g", "append")
    assert len(lab.rows("g")) == 3
    batch = [{"id": 1, "v": "a3"}, {"id": 1, "v": "a4"}, {"id": 2, "v": "B"}]
    # Run 1 of the deduplicating mode: no cursor state yet, so it replaces.
    _load(lab, _dedup(batch, ["id"]), "g", "replace")
    assert _v(lab, "g") == {1: "a4", 2: "B"}
    # Run 2 merges into a clean table.
    _merge_load(lab, _dedup([{"id": 2, "v": "BB"}, {"id": 3, "v": "c"}], ["id"]), "g", ["id"])
    assert _v(lab, "g") == {1: "a4", 2: "BB", 3: "c"}


def test_3_the_cursor_a_replacing_first_incremental_run_leaves_is_the_one_a_merge_continues_from(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path
) -> None:
    _load(lab, _items_source(tmp_path), "items", "append")
    items_db.execute("UPDATE items SET v='B', updated_at=40 WHERE id=2")
    items_db.execute("INSERT INTO items VALUES (4,'d',50)")
    items_db.commit()
    # `incremental`'s first run: no cursor, replaces; it leaves its cursor at 50.
    _load(lab, _items_source(tmp_path), "items", "incremental", "updated_at")
    assert _v(lab, "items") == {1: "a", 2: "B", 3: "c", 4: "d"}
    commits = lab.commits("items")
    # A merge run over the same cursor sees nothing new ...
    _merge_load(lab, _items_source(tmp_path), "items", ["id"])
    assert lab.commits("items") == commits
    # ... and one new row is merged on its own.
    items_db.execute("INSERT INTO items VALUES (5,'e',60)")
    items_db.commit()
    _merge_load(lab, _items_source(tmp_path), "items", ["id"])
    assert _v(lab, "items") == {1: "a", 2: "B", 3: "c", 4: "d", 5: "e"}
    assert lab.commits("items") - commits == 1


def test_3_a_merge_into_a_table_that_holds_a_key_in_two_data_files_does_not_fail_it_updates_both_rows(
    lab: Lab,
) -> None:
    """CORRECTS the first pass's case (g), which loaded both copies in ONE batch
    (so one data file) and saw pyiceberg refuse ("Target table has duplicate
    rows"). Copies in different files, which is what two runs of `append` leave,
    are NOT refused: the merge succeeds, updates both rows and leaves the
    duplicate in place. A merge is therefore never a repair, and the first run
    of a deduplicating mode has to replace the table (case 1 above), not rely on
    the merge noticing."""
    _load(lab, [{"id": 1, "v": "a"}, {"id": 2, "v": "b"}], "g", "append")
    _load(lab, [{"id": 1, "v": "a2"}], "g", "append")
    _merge_load(lab, [{"id": 1, "v": "NEW"}], "g", ["id"])
    assert sorted((r["id"], r["v"]) for r in lab.rows("g")) == [(1, "NEW"), (1, "NEW"), (2, "b")]


def test_3_a_table_with_two_versions_of_a_key_from_incremental_keeps_both_through_a_merge_that_carries_the_cursor_on(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path
) -> None:
    """The case the first-run rule does NOT cover: switching `incremental` to
    `incremental_dedup` keeps the cursor (it is the same cursor), so run 1 merges
    instead of replacing, into a table that holds an old and a new version of
    key 2, and (previous case) the merge updates both. This is the raw merge, which
    stays as measured; `load_via_sink` closes the gap with the marker it keeps in the
    pipeline state (`test_sink_dedup_marker.py`, planner finding R2 of the second
    pass): the first run under a deduplicating mode replaces, so it never gets here."""
    _load(lab, _items_source(tmp_path), "items", "incremental", "updated_at")
    items_db.execute("UPDATE items SET v='B', updated_at=40 WHERE id=2")
    items_db.commit()
    _load(lab, _items_source(tmp_path), "items", "incremental", "updated_at")
    assert sorted(r["v"] for r in lab.rows("items")) == ["B", "a", "b", "c"], "an old and a new version of id 2"
    items_db.execute("UPDATE items SET v='BB', updated_at=60 WHERE id=2")
    items_db.commit()
    _merge_load(lab, _items_source(tmp_path), "items", ["id"])
    assert sorted((r["id"], r["v"]) for r in lab.rows("items")) == [(1, "a"), (2, "BB"), (2, "BB"), (3, "c")]


# ---------------------------------------------------------------------------
# Measurement 4: what aborting the pending packages does
# ---------------------------------------------------------------------------


def test_4_drop_pending_packages_is_deprecated_and_abort_packages_is_what_it_calls(lab: Lab) -> None:
    pipeline = lab.pipeline("x")
    with pytest.warns(Warning, match="abort_packages"):
        pipeline.drop_pending_packages()


def test_4_after_a_merge_that_fails_on_a_repeated_key_aborting_lets_the_next_clean_merge_through(lab: Lab) -> None:
    _merge_load(lab, [{"id": 1, "v": "a"}], "d", ["id"])
    commits = lab.commits("d")
    with pytest.raises(PipelineStepFailed):
        _merge_load(lab, [{"id": 1, "v": "b"}, {"id": 1, "v": "c"}], "d", ["id"])
    assert lab.pipeline("d").has_pending_data
    lab.pipeline("d").abort_packages()
    assert not lab.pipeline("d").has_pending_data
    _merge_load(lab, [{"id": 1, "v": "b"}, {"id": 2, "v": "n"}], "d", ["id"])
    assert _v(lab, "d") == {1: "b", 2: "n"}
    assert lab.commits("d") - commits == 1


def test_4_after_a_null_key_fails_the_normalize_step_aborting_clears_the_extracted_package(lab: Lab) -> None:
    with pytest.raises(PipelineStepFailed):
        _merge_load(lab, [{"id": 1, "v": "a"}, {"id": None, "v": "n"}], "n", ["id"])
    assert lab.pipeline("n").has_pending_data
    lab.pipeline("n").abort_packages()
    assert not lab.pipeline("n").has_pending_data
    _merge_load(lab, [{"id": 1, "v": "a"}], "n", ["id"])
    assert _v(lab, "n") == {1: "a"}


def test_4_after_a_replace_that_fails_at_the_write_aborting_lets_the_next_replace_through_with_the_old_rows_intact(
    lab: Lab, monkeypatch: pytest.MonkeyPatch
) -> None:
    _load(lab, dlt.resource([{"id": 1, "v": "old"}], name="r"), "r", "replace")
    failure = _Failure(monkeypatch, Table, "overwrite")
    with pytest.raises(PipelineStepFailed):
        _load(lab, dlt.resource([{"id": 1, "v": "new"}, {"id": 2, "v": "x"}], name="r"), "r", "replace")
    assert _v(lab, "r") == {1: "old"}
    failure.heal()
    lab.pipeline("r").abort_packages()
    _load(lab, dlt.resource([{"id": 1, "v": "new"}, {"id": 2, "v": "x"}], name="r"), "r", "replace")
    assert _v(lab, "r") == {1: "new", 2: "x"}


def test_4_after_an_append_that_fails_at_the_write_aborting_loses_no_row_and_doubles_none(
    lab: Lab, monkeypatch: pytest.MonkeyPatch
) -> None:
    _load(lab, [{"id": 1, "v": "a"}], "ap", "append")
    failure = _Failure(monkeypatch, Table, "append")
    with pytest.raises(PipelineStepFailed):
        _load(lab, [{"id": 2, "v": "b"}], "ap", "append")
    assert sorted(_v(lab, "ap")) == [1]
    failure.heal()
    lab.pipeline("ap").abort_packages()
    _load(lab, [{"id": 2, "v": "b"}], "ap", "append")
    assert sorted(r["id"] for r in lab.rows("ap")) == [1, 2], "each row exactly once"


def test_4_without_the_abort_the_next_run_only_finishes_the_failed_package_and_does_not_extract_its_own_read(
    lab: Lab, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Why the loader aborts. `dlt` logs "The pipeline `run` method will now load
    the pending load packages. The data you passed to the run function will not be
    extracted": the next run reports success for the PREVIOUS run's read, and
    `pipeline.last_trace.last_normalize_info` is `None` (so `sink._extract_rows`
    cannot count it; see `test_sink_pending_packages.py`). Nothing is doubled here
    because both reads were the same rows.

    Changed with task D (it ran before it through `load_via_sink`, which now aborts
    the package itself): the abort is switched off here so the test still measures
    what `dlt` does without it."""
    monkeypatch.setattr(sink_module, "_abort_pending", lambda *_a, **_k: None)
    _load(lab, [{"id": 1, "v": "a"}], "ap", "append")
    failure = _Failure(monkeypatch, Table, "append")
    with pytest.raises(PipelineStepFailed):
        _load(lab, [{"id": 2, "v": "b"}], "ap", "append")
    failure.heal()
    pipeline = lab.pipeline("ap")
    pipeline.run([{"id": 3, "v": "newer"}], table_name="ap", table_format="iceberg", write_disposition="append")
    assert sorted(r["id"] for r in lab.rows("ap")) == [1, 2], "the failed run's rows, not the new read's"
    assert pipeline.last_trace.last_normalize_info is None


def test_4_the_incremental_cursor_does_not_advance_past_rows_of_an_aborted_package(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _load(lab, _items_source(tmp_path), "items", "incremental", "updated_at")
    assert sorted(_v(lab, "items")) == [1, 2, 3]
    items_db.execute("INSERT INTO items VALUES (4,'d',40)")
    items_db.execute("INSERT INTO items VALUES (5,'e',50)")
    items_db.commit()
    failure = _Failure(monkeypatch, Table, "append")
    with pytest.raises(PipelineStepFailed):
        _load(lab, _items_source(tmp_path), "items", "incremental", "updated_at")
    assert failure.calls, "the failure was injected at the write"
    assert sorted(_v(lab, "items")) == [1, 2, 3]
    failure.heal()
    lab.pipeline("items").abort_packages()
    # The next incremental run still sees rows 4 and 5, each once.
    _load(lab, _items_source(tmp_path), "items", "incremental", "updated_at")
    assert sorted(r["id"] for r in lab.rows("items")) == [1, 2, 3, 4, 5]


def test_4_without_the_abort_the_next_incremental_run_finishes_the_failed_package_and_the_one_after_reads_nothing_new(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """What the abort changes for the cursor. `dlt` has already moved the cursor
    to 40 in the failed package's state, so once that package is finished a
    fresh read is filtered to nothing: no row is lost, but the table is complete
    only because a package a dead run left was finished, not because of this
    run's read.

    Changed with task D, as the case above: `load_via_sink`'s own abort is switched
    off so the test still measures `dlt` without it."""
    monkeypatch.setattr(sink_module, "_abort_pending", lambda *_a, **_k: None)
    _load(lab, _items_source(tmp_path), "items", "incremental", "updated_at")
    items_db.execute("INSERT INTO items VALUES (4,'d',40)")
    items_db.commit()
    failure = _Failure(monkeypatch, Table, "append")
    with pytest.raises(PipelineStepFailed):
        _load(lab, _items_source(tmp_path), "items", "incremental", "updated_at")
    assert lab.pipeline("items").has_pending_data
    failure.heal()
    lab.pipeline("items").run()
    assert sorted(r["id"] for r in lab.rows("items")) == [1, 2, 3, 4]
    _load(lab, _items_source(tmp_path), "items", "incremental", "updated_at")
    assert sorted(r["id"] for r in lab.rows("items")) == [1, 2, 3, 4]


# ---------------------------------------------------------------------------
# Measurement 5: a merge above 1000 rows is several commits
# ---------------------------------------------------------------------------


def test_5_a_failure_in_a_later_slice_of_a_2500_row_merge_leaves_one_row_per_key_and_the_next_run_completes_it(
    lab: Lab, monkeypatch: pytest.MonkeyPatch
) -> None:
    keys = range(2500)
    _merge_load(lab, [{"id": i, "v": "old"} for i in keys], "big", ["id"])
    commits = lab.commits("big")
    failure = _Failure(monkeypatch, Table, "upsert", from_call=3)
    source = [{"id": i, "v": "new"} for i in keys]
    with pytest.raises(PipelineStepFailed):
        _merge_load(lab, source, "big", ["id"])
    assert failure.calls >= 3
    assert lab.commits("big") - commits == 2, "two slices of 1000 rows were committed before the failure"
    rows = lab.rows("big")
    assert len(rows) == 2500 and len({r["id"] for r in rows}) == 2500, "one row per key after the partial merge"
    assert sorted({r["v"] for r in rows}) == ["new", "old"]
    assert sum(1 for r in rows if r["v"] == "new") == 2000
    failure.heal()
    lab.pipeline("big").abort_packages()
    _merge_load(lab, source, "big", ["id"])
    assert {r["v"] for r in lab.rows("big")} == {"new"} and len(lab.rows("big")) == 2500


def test_5_a_failure_in_a_later_slice_of_a_merge_that_inserts_new_keys_also_leaves_one_row_per_key(
    lab: Lab, monkeypatch: pytest.MonkeyPatch
) -> None:
    source = [{"id": i, "v": "new"} for i in range(2500)]
    _merge_load(lab, [{"id": 0, "v": "old"}], "ins", ["id"])
    failure = _Failure(monkeypatch, Table, "upsert", from_call=3)
    with pytest.raises(PipelineStepFailed):
        _merge_load(lab, source, "ins", ["id"])
    rows = lab.rows("ins")
    assert len({r["id"] for r in rows}) == len(rows), "no key twice"
    failure.heal()
    lab.pipeline("ins").abort_packages()
    _merge_load(lab, source, "ins", ["id"])
    assert len(lab.rows("ins")) == 2500 and {r["v"] for r in lab.rows("ins")} == {"new"}


# ---------------------------------------------------------------------------
# Measurement 6: a package a killed process left
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("killed_before", ["normalize", "load"])
def test_6_without_an_abort_at_the_start_the_run_after_a_kill_finishes_the_dead_package_and_loads_none_of_its_own_rows(
    lab: Lab, monkeypatch: pytest.MonkeyPatch, killed_before: str
) -> None:
    """What the next `load_via_sink` does TODAY (the abort at the start of a run is
    switched off here, as in the cases of 4): a Kafka-shaped micro-batch [1, 2, 3] dies
    between extract and load; the next micro-batch [4, 5] is reported as a success
    with the DEAD batch's row count (3, or none; never 2) while the
    table holds the dead batch's rows and not its own. A consumer that commits offsets
    after the call returns has then committed 4 and 5 for rows that are not in the table."""
    monkeypatch.setattr(sink_module, "_abort_pending", lambda *_a, **_k: None)
    _load(lab, [{"id": 0, "v": "first"}], "k", "append")  # the table exists: the bucket holds a state
    with monkeypatch.context() as dies:
        kill_before(dies, killed_before)
        with pytest.raises(Killed):
            _load(lab, [{"id": n, "v": "dead"} for n in (1, 2, 3)], "k", "append")
    assert lab.pipeline("k").has_pending_data, "the dead run left its package behind"
    result = _load(lab, [{"id": 4, "v": "live"}, {"id": 5, "v": "live"}], "k", "append")
    assert not result.has_failed_jobs, "reported as a success"
    # Killed before normalize, the run normalized the DEAD package and counts its 3 rows;
    # killed after it, no normalize ran in this run at all and there is no count. Never 2.
    assert result.rows == {"normalize": 3, "load": None}[killed_before], "no count of this batch's own 2 rows"
    assert sorted(_v(lab, "k")) == [0, 1, 2, 3], "the dead batch's rows are in; this batch's rows are not"
    # The batch after that one finds nothing pending and loads its own rows.
    _load(lab, [{"id": 6, "v": "next"}], "k", "append")
    assert sorted(_v(lab, "k")) == [0, 1, 2, 3, 6]


def test_6_a_run_killed_before_the_table_ever_had_a_state_is_dropped_by_the_next_run_not_finished(
    lab: Lab, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The one case that needs no abort: with no state in the bucket yet, `dlt`'s
    restore at the start of `run` wipes the working folder, package included, so the
    next run reads its own rows. (Why the cases above load a row first.)"""
    monkeypatch.setattr(sink_module, "_abort_pending", lambda *_a, **_k: None)
    with monkeypatch.context() as dies:
        kill_before(dies, "load")
        with pytest.raises(Killed):
            _load(lab, [{"id": n, "v": "dead"} for n in (1, 2, 3)], "k", "append")
    result = _load(lab, [{"id": 4, "v": "live"}], "k", "append")
    assert result.rows == 1 and sorted(_v(lab, "k")) == [4]


def test_6_the_incremental_cursor_moved_by_a_killed_run_is_kept_and_the_dead_package_is_finished_by_the_next_run(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """For a cursor the dead package carries the cursor's move (`dlt` saves the
    state at extract), so finishing the package keeps rows and cursor consistent; what
    is lost is only the next run's own read, which waits for the run after it."""
    monkeypatch.setattr(sink_module, "_abort_pending", lambda *_a, **_k: None)
    _load(lab, _items_source(tmp_path), "items", "incremental", "updated_at")
    items_db.execute("INSERT INTO items VALUES (4,'d',40)")
    items_db.commit()
    with monkeypatch.context() as dies:
        kill_before(dies, "load")
        with pytest.raises(Killed):
            _load(lab, _items_source(tmp_path), "items", "incremental", "updated_at")
    items_db.execute("INSERT INTO items VALUES (5,'e',50)")
    items_db.commit()
    result = _load(lab, _items_source(tmp_path), "items", "incremental", "updated_at")
    assert result.rows == 0, "reported as 'nothing new' although the dead package's row 4 was loaded"
    assert sorted(_v(lab, "items")) == [1, 2, 3, 4], "row 4 from the dead package; row 5 is not read yet"
    _load(lab, _items_source(tmp_path), "items", "incremental", "updated_at")
    assert sorted(r["id"] for r in lab.rows("items")) == [1, 2, 3, 4, 5]
