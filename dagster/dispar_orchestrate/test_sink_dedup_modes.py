"""`replace_dedup` and `incremental_dedup` through `sink.load_via_sink` itself
(`SRC-10` plan task 3, second pass, planner decisions 2, 3, 5 and 6).

The harness is `test_load_modes_measurement.py`'s: `load_via_sink` runs for real
with only its two network-bound names swapped (`lab`), on a local Iceberg
catalog and, for SQL, a SQLite source. Everything the measurement modules
observed about `dlt` 1.30.0 and `pyiceberg` 0.12.0 holds here; this module
asserts what the LOADER does with it.
"""

from __future__ import annotations

import sqlite3
import time
from pathlib import Path
from typing import Any

import dlt
import pytest
from dlt.extract.incremental import IncrementalResourceWrapper
from dlt.extract.items_transform import MapItem
from dlt.sources.sql_database import sql_database

from dispar_orchestrate import ingest_factory as f
from dispar_orchestrate import schema_observer
from dispar_orchestrate.adapters import dedup
from dispar_orchestrate.adapters import sink as sink_module
from dispar_orchestrate.adapters.dedup import MissingKeyValue, TooManyKeys
from dispar_orchestrate.adapters.sink import LoadPlan, UnsupportedLoadMode, load_via_sink
from dispar_orchestrate.test_load_modes_measurement import Lab, lab  # noqa: F401 -- `lab` is a fixture

# ---------------------------------------------------------------------------
# LoadPlan
# ---------------------------------------------------------------------------


def test_the_load_modes_include_the_two_deduplicating_ones() -> None:
    assert sink_module.LOAD_MODES == ("replace", "append", "incremental", "replace_dedup", "incremental_dedup", "history")


def test_a_plan_reads_the_primary_key_of_a_deduplicating_source_object() -> None:
    obj = {"loadMode": "incremental_dedup", "incrementalKey": "updated_at", "primaryKey": ["id", "line"]}
    assert LoadPlan.from_source_object(obj) == LoadPlan(mode="incremental_dedup", cursor="updated_at", key=("id", "line"))
    replace = {"loadMode": "replace_dedup", "primaryKey": ["id"]}
    assert LoadPlan.from_source_object(replace) == LoadPlan(mode="replace_dedup", key=("id",))


def test_replace_dedup_keeps_a_cursor_as_the_tie_break_but_needs_none() -> None:
    plan = LoadPlan.from_source_object({"loadMode": "replace_dedup", "incrementalKey": " seq ", "primaryKey": ["id"]})
    assert plan.cursor == "seq"


def test_a_primary_key_on_another_mode_is_ignored() -> None:
    assert LoadPlan.from_source_object({"loadMode": "append", "primaryKey": ["id"]}) == LoadPlan(mode="append")
    assert LoadPlan.from_source_object({"primaryKey": ["id"]}) == LoadPlan(mode="replace")
    # ... even a malformed one: the API ignores it too once the mode does not read it.
    assert LoadPlan.from_source_object({"loadMode": "append", "primaryKey": "id"}) == LoadPlan(mode="append")


@pytest.mark.parametrize("mode", ["replace_dedup", "incremental_dedup"])
def test_a_deduplicating_plan_without_a_key_is_refused(mode: str) -> None:
    obj = {"loadMode": mode, "incrementalKey": "updated_at"}
    with pytest.raises(UnsupportedLoadMode, match="primaryKey"):
        LoadPlan.from_source_object(obj)
    with pytest.raises(UnsupportedLoadMode, match="primaryKey"):
        LoadPlan.from_source_object({**obj, "primaryKey": []})


@pytest.mark.parametrize(
    "key",
    [
        [f"c{n}" for n in range(17)],
        ["x" * 129],
        [""],
        ["  "],
        ["a\nb"],
        ["id", "id"],
        ["id", 7],
    ],
)
def test_a_primary_key_that_the_api_would_refuse_is_refused_at_run_time_too(key: list[Any]) -> None:
    with pytest.raises(UnsupportedLoadMode, match="primaryKey"):
        LoadPlan.from_source_object({"loadMode": "replace_dedup", "primaryKey": key})


def test_a_primary_key_that_is_not_a_list_is_refused() -> None:
    with pytest.raises(UnsupportedLoadMode, match="primaryKey"):
        LoadPlan.from_source_object({"loadMode": "replace_dedup", "primaryKey": "id"})


def test_the_largest_key_the_api_allows_is_accepted() -> None:
    key = [f"c{n}" for n in range(16)]
    assert LoadPlan.from_source_object({"loadMode": "replace_dedup", "primaryKey": key}).key == tuple(key)
    assert LoadPlan.from_source_object({"loadMode": "replace_dedup", "primaryKey": ["x" * 128]}).key == ("x" * 128,)


def test_incremental_dedup_needs_a_cursor_like_incremental() -> None:
    with pytest.raises(UnsupportedLoadMode, match="incrementalKey"):
        LoadPlan.from_source_object({"loadMode": "incremental_dedup", "primaryKey": ["id"]})


def test_the_factory_offers_incremental_dedup_to_sql_connectors_only_and_replace_dedup_to_every_batch_adapter() -> None:
    incremental = {"name": "o", "target": "o", "loadMode": "incremental_dedup", "incrementalKey": "u", "primaryKey": ["id"]}
    assert f._load_plan("sql", incremental).mode == "incremental_dedup"
    for adapter in ("rest", "files", "mongodb", "sftp"):
        with pytest.raises(UnsupportedLoadMode, match="only available for SQL"):
            f._load_plan(adapter, incremental)
        replace = {"name": "o", "target": "o", "loadMode": "replace_dedup", "primaryKey": ["id"]}
        assert f._load_plan(adapter, replace).mode == "replace_dedup"


# ---------------------------------------------------------------------------
# replace_dedup on plain rows (files, REST, MongoDB, SFTP)
# ---------------------------------------------------------------------------


def _values(lab: Lab, table: str) -> dict[Any, str]:
    return {r["id"]: r["v"] for r in lab.rows(table)}


def _dedup_load(lab: Lab, source: Any, table: str, mode: str, key: tuple[str, ...], cursor: str | None = None) -> Any:
    return load_via_sink(source, table, lab.config, LoadPlan(mode=mode, cursor=cursor, key=key))


def test_replace_dedup_over_plain_rows_leaves_one_row_per_key_and_the_cursor_picks_the_winner(lab: Lab) -> None:
    rows = [
        {"id": 1, "v": "old", "seq": 1},
        {"id": 1, "v": "new", "seq": 9},
        {"id": 1, "v": "mid", "seq": 5},
        {"id": 2, "v": "b", "seq": 1},
    ]
    result = _dedup_load(lab, rows, "t", "replace_dedup", ("id",), "seq")
    assert _values(lab, "t") == {1: "new", 2: "b"}
    assert result.rows == 2, "the measured count is the winners, not the rows read"
    assert all(r["_ingested_at"] is not None for r in lab.rows("t"))


def test_replace_dedup_without_a_cursor_keeps_the_last_row_read(lab: Lab) -> None:
    _dedup_load(lab, [{"id": 1, "v": "first"}, {"id": 1, "v": "last"}], "t", "replace_dedup", ("id",))
    assert _values(lab, "t") == {1: "last"}


def test_replace_dedup_replaces_the_table_on_every_run_and_never_merges(lab: Lab) -> None:
    _dedup_load(lab, [{"id": 1, "v": "a"}, {"id": 2, "v": "b"}], "t", "replace_dedup", ("id",))
    _dedup_load(lab, [{"id": 2, "v": "B"}, {"id": 3, "v": "c"}, {"id": 3, "v": "C"}], "t", "replace_dedup", ("id",))
    assert _values(lab, "t") == {2: "B", 3: "C"}, "key 1 is gone: the table is what the source returns now"


def test_replace_dedup_leaves_an_append_table_with_duplicates_clean_in_one_run(lab: Lab) -> None:
    load_via_sink([{"id": 1, "v": "a"}, {"id": 2, "v": "b"}], "t", lab.config, LoadPlan(mode="append"))
    load_via_sink([{"id": 1, "v": "a2"}], "t", lab.config, LoadPlan(mode="append"))
    assert len(lab.rows("t")) == 3
    _dedup_load(lab, [{"id": 1, "v": "A"}, {"id": 2, "v": "B"}], "t", "replace_dedup", ("id",))
    assert _values(lab, "t") == {1: "A", 2: "B"}


def test_replace_dedup_over_a_dlt_resource_deduplicates_and_stamps_the_winner(lab: Lab) -> None:
    resource = dlt.resource([{"id": 1, "v": "a"}, {"id": 1, "v": "b"}, {"id": 2, "v": "c"}], name="t")
    _dedup_load(lab, resource, "t", "replace_dedup", ("id",))
    assert _values(lab, "t") == {1: "b", 2: "c"}


def test_replace_dedup_of_an_empty_source_does_what_replace_does_with_it(lab: Lab) -> None:
    """Measured, not assumed: plain rows `[]` empty the table under `replace`; an empty
    `dlt` resource leaves it untouched (no data reaches the destination). The dedup
    step must not change either."""
    columns = {"id": {"data_type": "bigint"}, "v": {"data_type": "text"}}
    for table, empty in (("plain", []), ("resource", dlt.resource([], name="resource", columns=columns))):
        load_via_sink([{"id": 1, "v": "a"}], table, lab.config, LoadPlan(mode="replace"))
        load_via_sink(empty, table, lab.config, LoadPlan(mode="replace"))
        replaced = len(lab.rows(table))
        load_via_sink([{"id": 1, "v": "a"}], f"{table}_dedup", lab.config, LoadPlan(mode="replace"))
        _dedup_load(lab, empty, f"{table}_dedup", "replace_dedup", ("id",))
        assert len(lab.rows(f"{table}_dedup")) == replaced
    assert lab.rows("plain_dedup") == []


# ---------------------------------------------------------------------------
# A key with no value (measurement case d), the bound, and the unwrap
# ---------------------------------------------------------------------------


def test_a_null_key_fails_the_load_with_the_fixed_message_before_anything_is_written(lab: Lab) -> None:
    rows = [{"id": 1, "v": "a"}, {"id": None, "v": "SECRET-ROW-DATA"}, {"v": "no key at all"}]
    with pytest.raises(MissingKeyValue) as raised:
        _dedup_load(lab, rows, "n", "replace_dedup", ("id",))
    assert str(raised.value) == (
        "table 'n': 2 row(s) have no value for key column 'id'; a deduplicating load needs every row to carry its key"
    )
    assert not list(lab.bucket.glob("**/n/metadata/*.metadata.json")), "nothing was written"
    # A null key is not a retryable fault of the pipeline: the failure is a ValueError the op boundary
    # turns into `Failure(allow_retries=False)`, and the next clean load goes through.
    assert isinstance(raised.value, ValueError)
    _dedup_load(lab, [{"id": 1, "v": "a"}], "n", "replace_dedup", ("id",))
    assert _values(lab, "n") == {1: "a"}


def test_a_null_key_in_a_sql_table_fails_the_same_way(lab: Lab, tmp_path: Path) -> None:
    db = sqlite3.connect(tmp_path / "source.db")
    db.execute("CREATE TABLE events (id INTEGER, v TEXT, updated_at INTEGER)")
    db.executemany("INSERT INTO events VALUES (?,?,?)", [(1, "a", 1), (None, "b", 2), (None, "c", 3)])
    db.commit()
    with pytest.raises(MissingKeyValue, match=r"2 row\(s\) have no value for key column 'id'"):
        _dedup_load(lab, _events_source(tmp_path), "events", "incremental_dedup", ("id",), "updated_at")


def test_a_load_with_more_distinct_keys_than_the_bound_fails_naming_the_setting_and_writes_nothing(
    lab: Lab, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("INGEST_DEDUP_MAX_ROWS", "3")
    with pytest.raises(TooManyKeys, match="INGEST_DEDUP_MAX_ROWS"):
        _dedup_load(lab, [{"id": n} for n in range(4)], "big", "replace_dedup", ("id",))
    assert not list(lab.bucket.glob("**/big/metadata/*.metadata.json"))
    _dedup_load(lab, [{"id": n} for n in range(3)], "big", "replace_dedup", ("id",))
    assert len(lab.rows("big")) == 3


def test_an_unreadable_bound_fails_the_load_closed(lab: Lab, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("INGEST_DEDUP_MAX_ROWS", "lots")
    with pytest.raises(dedup.DedupRefused, match="INGEST_DEDUP_MAX_ROWS"):
        _dedup_load(lab, [{"id": 1}], "b", "replace_dedup", ("id",))


def test_the_ingest_run_records_a_dedup_refusal_as_rejected_and_the_op_boundary_makes_it_non_retryable() -> None:
    assert issubclass(dedup.DedupRefused, ValueError), "the op boundary maps ValueError to Failure(allow_retries=False)"
    assert f.dedup_adapter.DedupRefused is dedup.DedupRefused


# ---------------------------------------------------------------------------
# incremental_dedup over a SQL source
# ---------------------------------------------------------------------------


@pytest.fixture
def events_db(tmp_path: Path) -> sqlite3.Connection:
    """A table with NO primary key, holding the same id more than once: the
    shape a deduplicating mode exists for."""
    db = sqlite3.connect(tmp_path / "source.db")
    db.execute("CREATE TABLE events (id INTEGER, v TEXT, updated_at INTEGER)")
    db.executemany(
        "INSERT INTO events VALUES (?,?,?)",
        [(1, "a-old", 10), (1, "a-new", 20), (2, "b", 15), (3, "c", 30), (3, "c-older", 5)],
    )
    db.commit()
    return db


def _events_source(tmp_path: Path, callback: Any = None) -> Any:
    extra = {} if callback is None else {"table_adapter_callback": callback}
    return sql_database(
        credentials={"drivername": "sqlite", "database": str(tmp_path / "source.db")}, table_names=["events"], **extra
    )


def _incremental_dedup(lab: Lab, tmp_path: Path, callback: Any = None) -> Any:
    return _dedup_load(lab, _events_source(tmp_path, callback), "events", "incremental_dedup", ("id",), "updated_at")


def test_incremental_dedup_first_run_loads_everything_once_per_key_and_the_next_run_merges_what_changed(
    lab: Lab, events_db: sqlite3.Connection, tmp_path: Path
) -> None:
    first = _incremental_dedup(lab, tmp_path)
    assert _values(lab, "events") == {1: "a-new", 2: "b", 3: "c"}
    assert first.rows == 3
    before = {r["id"]: r["_ingested_at"] for r in lab.rows("events")}
    commits = lab.commits("events")
    time.sleep(1.1)
    events_db.execute("INSERT INTO events VALUES (2, 'b2', 40)")
    events_db.execute("INSERT INTO events VALUES (4, 'd', 50)")
    events_db.commit()
    second = _incremental_dedup(lab, tmp_path)
    assert _values(lab, "events") == {1: "a-new", 2: "b2", 3: "c", 4: "d"}
    assert second.rows == 2
    assert lab.commits("events") - commits == 1, "one merge, one commit"
    after = {r["id"]: r["_ingested_at"] for r in lab.rows("events")}
    assert after[1] == before[1] and after[3] == before[3], "an untouched row keeps its time"
    assert after[2] > before[2], "a merged row carries the time of the merge"
    assert [f.name for f in lab.table("events").spec().fields] == ["_ingested_at_day"]


def test_incremental_dedup_with_nothing_new_reports_zero_rows_and_commits_nothing(
    lab: Lab, events_db: sqlite3.Connection, tmp_path: Path
) -> None:
    _incremental_dedup(lab, tmp_path)
    commits = lab.commits("events")
    again = _incremental_dedup(lab, tmp_path)
    assert again.rows == 0
    assert lab.commits("events") == commits
    assert len(lab.rows("events")) == 3


def test_incremental_dedup_deduplicates_the_new_rows_of_a_run_before_merging_them(
    lab: Lab, events_db: sqlite3.Connection, tmp_path: Path
) -> None:
    _incremental_dedup(lab, tmp_path)
    events_db.executemany("INSERT INTO events VALUES (?,?,?)", [(5, "e1", 60), (5, "e2", 70), (5, "e0", 55)])
    events_db.commit()
    _incremental_dedup(lab, tmp_path)  # a repeated key in one merge batch would fail the load
    assert _values(lab, "events")[5] == "e2"
    assert len(lab.rows("events")) == 4


def test_switching_an_append_table_with_duplicates_to_incremental_dedup_cleans_it_on_run_1_and_run_2_merges(
    lab: Lab, events_db: sqlite3.Connection, tmp_path: Path
) -> None:
    """Planner decision 3 / measurement 3. Two `append` runs leave the same ids
    in two data files, which a merge does not notice (see the measurement)."""
    for _ in range(2):
        load_via_sink(_events_source(tmp_path), "events", lab.config, LoadPlan(mode="append"))
    assert len(lab.rows("events")) == 10, "every source row twice"
    _incremental_dedup(lab, tmp_path)  # run 1: no cursor state, replaces with the winners
    assert _values(lab, "events") == {1: "a-new", 2: "b", 3: "c"}
    assert len(lab.rows("events")) == 3
    events_db.execute("UPDATE events SET v='b2', updated_at=99 WHERE id=2")
    events_db.commit()
    _incremental_dedup(lab, tmp_path)  # run 2: merges, does not fail
    assert _values(lab, "events") == {1: "a-new", 2: "b2", 3: "c"}
    assert len(lab.rows("events")) == 3


def test_replace_dedup_over_a_sql_table_replaces_with_the_winners_every_run(
    lab: Lab, events_db: sqlite3.Connection, tmp_path: Path
) -> None:
    _dedup_load(lab, _events_source(tmp_path), "events", "replace_dedup", ("id",), "updated_at")
    assert _values(lab, "events") == {1: "a-new", 2: "b", 3: "c"}
    events_db.execute("DELETE FROM events WHERE id=2")
    events_db.commit()
    _dedup_load(lab, _events_source(tmp_path), "events", "replace_dedup", ("id",), "updated_at")
    assert _values(lab, "events") == {1: "a-new", 3: "c"}


def test_incremental_dedup_works_with_the_schema_observers_column_selection(
    lab: Lab, events_db: sqlite3.Connection, tmp_path: Path
) -> None:
    """SRC-8's `table_adapter_callback` (here `keep_only`) still reaches the query:
    the dropped column is neither read nor loaded, and the dedup runs on what is left."""
    _incremental_dedup(lab, tmp_path, schema_observer.keep_only(["id", "v", "updated_at"]))
    assert _values(lab, "events") == {1: "a-new", 2: "b", 3: "c"}
    source = _events_source(tmp_path, schema_observer.keep_only(["id", "updated_at"]))
    _dedup_load(lab, source, "events2", "replace_dedup", ("id",), "updated_at")
    assert all("v" not in r for r in lab.rows("events2")) and len(lab.rows("events2")) == 3


def test_a_key_column_the_selection_removed_fails_as_a_missing_key_value(
    lab: Lab, events_db: sqlite3.Connection, tmp_path: Path
) -> None:
    with pytest.raises(MissingKeyValue, match=r"5 row\(s\) have no value for key column 'id'"):
        _dedup_load(
            lab,
            _events_source(tmp_path, schema_observer.keep_only(["v", "updated_at"])),
            "events",
            "replace_dedup",
            ("id",),
            "updated_at",
        )


# ---------------------------------------------------------------------------
# Order of the steps: cursor filter, then dedup, then stamp (decision 6)
# ---------------------------------------------------------------------------


def test_the_pipe_runs_the_cursor_filter_then_the_dedup_then_the_stamp(
    lab: Lab, events_db: sqlite3.Connection, tmp_path: Path
) -> None:
    source = _events_source(tmp_path)
    _dedup_load(lab, source, "events", "incremental_dedup", ("id",), "updated_at")
    resource = next(iter(source.resources.values()))
    steps = resource._pipe.steps
    assert [type(s) for s in steps[1:]] == [IncrementalResourceWrapper, dedup.DedupStep, MapItem]


def test_the_stamp_is_applied_to_the_winners_only(
    lab: Lab, events_db: sqlite3.Connection, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    stamped: list[Any] = []
    original = sink_module._stamp_ingested_at

    def counting(record: dict[str, Any]) -> dict[str, Any]:
        stamped.append(record["id"])
        return original(record)

    monkeypatch.setattr(sink_module, "_stamp_ingested_at", counting)
    _dedup_load(lab, _events_source(tmp_path), "events", "incremental_dedup", ("id",), "updated_at")
    assert sorted(stamped) == [1, 2, 3], "5 rows were read, 3 winners were stamped"
    stamped.clear()
    _dedup_load(lab, [{"id": 1, "v": "a"}, {"id": 1, "v": "b"}], "plain", "replace_dedup", ("id",))
    assert stamped == [1]


def test_the_cursor_filter_runs_before_the_dedup_so_a_row_below_the_cursor_never_competes(
    lab: Lab, events_db: sqlite3.Connection, tmp_path: Path
) -> None:
    """Behaviour that only the filter-first order gives. After run 1 the cursor is
    30 (`id` 3). A row for id 3 with a LOWER cursor value is added and also a row
    that is not: only the second is read, and a repeated key among the rows read
    does not survive. If the dedup ran first over the whole table, the lower row
    would never matter either way, so the order itself is asserted structurally in
    the previous test; here we assert the rows read are exactly the filtered ones."""
    _incremental_dedup(lab, tmp_path)
    events_db.execute("INSERT INTO events VALUES (3, 'stale', 12)")
    events_db.execute("INSERT INTO events VALUES (3, 'fresh', 31)")
    events_db.commit()
    result = _incremental_dedup(lab, tmp_path)
    assert result.rows == 1
    assert _values(lab, "events")[3] == "fresh"
