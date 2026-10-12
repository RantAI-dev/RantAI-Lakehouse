"""The `history` load mode through `sink.load_via_sink` (`SRC-10` sixth pass, task H3;
feature page acceptance rows 2 and 12), on the harness of
`test_load_modes_measurement.py`: a local Iceberg catalog, the real `load_via_sink`
with only its two network-bound names replaced, no network and no query engine.
Expected values come from the mode's stated rules (`adapters/history.py`).
"""

from __future__ import annotations

import sqlite3
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import pytest
from dlt.sources.sql_database import sql_database

from dispar_orchestrate.adapters import dedup
from dispar_orchestrate.adapters.history import HISTORY_COLUMNS, HistoryRefused
from dispar_orchestrate.adapters.sink import LoadPlan, SinkResult, clear_table, load_via_sink
from dispar_orchestrate.test_load_modes_measurement import Lab, lab  # noqa: F401 -- `lab` is a fixture


def _plan(*key: str, refresh: str | None = None) -> LoadPlan:
    return LoadPlan(mode="history", key=key or ("id",), refresh=refresh)


def _load(lab: Lab, rows: Any, table: str = "h", *key: str, refresh: str | None = None) -> SinkResult:
    return load_via_sink(rows, table, lab.config, _plan(*key, refresh=refresh))


def _versions(lab: Lab, table: str = "h", *key: str) -> dict[Any, list[dict[str, Any]]]:
    """Every version of every key, oldest first."""
    columns = key or ("id",)
    out: dict[Any, list[dict[str, Any]]] = {}
    for row in lab.rows(table):
        out.setdefault(tuple(row[c] for c in columns), []).append(row)
    for versions in out.values():
        versions.sort(key=lambda r: r["_valid_from"])
    return out


def _by(rows: list[dict[str, Any]], **where: Any) -> list[dict[str, Any]]:
    return [r for r in rows if all(r[k] == v for k, v in where.items())]


def test_one_changed_one_new_and_one_unchanged_row_give_one_closed_row_and_two_new_current_ones(lab: Lab) -> None:
    """Acceptance row 2."""
    first = _load(lab, [{"id": 1, "v": "a"}, {"id": 2, "v": "b"}, {"id": 3, "v": "c"}])
    assert first.rows == 3
    before = {r["id"]: r for r in lab.rows("h")}

    second = _load(lab, [{"id": 1, "v": "a"}, {"id": 2, "v": "B"}, {"id": 4, "v": "d"}, {"id": 3, "v": "c"}])

    assert second.rows == 2, "NEW versions: the closed row is not counted"
    rows = lab.rows("h")
    assert len(rows) == 5
    new_2, new_4 = _by(rows, id=2, v="B")[0], _by(rows, id=4)[0]
    closed = _by(rows, id=2, v="b")
    assert len(closed) == 1
    assert closed[0]["_is_current"] is False and closed[0]["_valid_to"] == new_2["_valid_from"]
    assert closed[0]["_valid_from"] == before[2]["_valid_from"] and closed[0]["_ingested_at"] == before[2]["_ingested_at"]
    assert closed[0]["_row_hash"] == before[2]["_row_hash"]
    for new in (new_2, new_4):
        assert new["_is_current"] is True and new["_valid_to"] is None
        assert new["_valid_from"] == new["_ingested_at"] > before[1]["_valid_from"]
    # The unchanged keys are untouched, every column.
    assert _by(rows, id=1) == [before[1]] and _by(rows, id=3) == [before[3]]
    assert sum(1 for r in rows if r["_is_current"]) == 4, "one current row per key"


def test_two_more_identical_runs_write_nothing_and_closed_rows_keep_their_times(lab: Lab) -> None:
    """Acceptance row 12."""
    _load(lab, [{"id": 1, "v": "a"}, {"id": 2, "v": "b"}])
    _load(lab, [{"id": 1, "v": "a"}, {"id": 2, "v": "B"}])
    rows, commits, snapshots = lab.rows("h"), lab.commits("h"), lab.snapshots("h")
    for _ in range(2):
        result = _load(lab, [{"id": 1, "v": "a"}, {"id": 2, "v": "B"}])
        assert result.rows == 0 and not result.has_failed_jobs
    assert lab.rows("h") == rows, "no row written or changed, closed rows included"
    assert (lab.commits("h"), lab.snapshots("h")) == (commits, snapshots), "and not one commit"
    assert {k: sum(1 for r in v if r["_is_current"]) for k, v in _versions(lab).items()} == {(1,): 1, (2,): 1}


def test_a_composite_key_closes_only_the_pair_that_changed(lab: Lab) -> None:
    _load(lab, [{"a": 1, "b": 1, "v": "x"}, {"a": 1, "b": 2, "v": "y"}, {"a": 2, "b": 1, "v": "z"}], "c", "a", "b")
    _load(lab, [{"a": 1, "b": 1, "v": "x"}, {"a": 1, "b": 2, "v": "Y"}, {"a": 2, "b": 1, "v": "z"}], "c", "a", "b")
    versions = _versions(lab, "c", "a", "b")
    assert {k: len(v) for k, v in versions.items()} == {(1, 1): 1, (1, 2): 2, (2, 1): 1}
    assert [(r["v"], r["_is_current"]) for r in versions[(1, 2)]] == [("y", False), ("Y", True)]


def test_a_row_that_changes_back_to_its_old_values_is_a_third_version(lab: Lab) -> None:
    for value in ("a", "b", "a"):
        _load(lab, [{"id": 1, "v": value}])
    versions = _versions(lab)[(1,)]
    assert [(r["v"], r["_is_current"]) for r in versions] == [("a", False), ("b", False), ("a", True)]
    assert versions[0]["_valid_to"] == versions[1]["_valid_from"] and versions[1]["_valid_to"] == versions[2]["_valid_from"]
    assert versions[0]["_row_hash"] == versions[2]["_row_hash"] != versions[1]["_row_hash"]
    assert len({r["_valid_from"] for r in versions}) == 3


def test_a_column_added_at_the_source_closes_only_the_rows_that_have_a_value_in_it(lab: Lab) -> None:
    first = _load(lab, [{"id": 1, "v": "a"}, {"id": 2, "v": "b"}])
    assert [c.name for c in first.columns] == ["id", "v"], "the four history columns are not source columns"
    before = {r["id"]: r for r in lab.rows("h")}

    second = _load(lab, [{"id": 1, "v": "a", "extra": None}, {"id": 2, "v": "b", "extra": "x"}])

    assert second.rows == 1
    assert "extra" in {c.name for c in second.columns} and not {c.name for c in second.columns} & {*HISTORY_COLUMNS, "_ingested_at"}
    rows = lab.rows("h")
    assert _by(rows, id=1) == [{**before[1], "extra": None}], "a null in the new column is not a change"
    assert [(r["_is_current"], r["extra"]) for r in sorted(_by(rows, id=2), key=lambda r: r["_valid_from"])] == [
        (False, None),
        (True, "x"),
    ]


def test_a_table_made_by_the_history_mode_has_the_sinks_set_up(lab: Lab) -> None:
    _load(lab, [{"id": 1, "v": "a"}])
    table = lab.table("h")
    assert [f.name for f in table.spec().fields] == ["_ingested_at_day"]
    assert table.metadata.format_version == 2
    assert {f.name for f in table.schema().fields} >= {*HISTORY_COLUMNS, "_ingested_at", "_dlt_id", "_dlt_load_id"}
    row = lab.rows("h")[0]
    assert row["_ingested_at"] == row["_valid_from"] and isinstance(row["_ingested_at"], datetime)
    assert row["_ingested_at"].tzinfo is not None


def test_a_sql_source_is_read_whole_and_a_changed_row_becomes_a_new_version(lab: Lab, tmp_path: Path) -> None:
    db = sqlite3.connect(tmp_path / "source.db")
    db.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, v TEXT, n NUMERIC, d DATE)")
    db.executemany("INSERT INTO items VALUES (?,?,?,?)", [(1, "a", 1.5, "2026-01-02"), (2, "b", None, None)])
    db.commit()

    def source() -> Any:
        return sql_database(
            credentials={"drivername": "sqlite", "database": str(tmp_path / "source.db")}, table_names=["items"]
        )

    assert load_via_sink(source(), "items", lab.config, _plan()).rows == 2
    assert load_via_sink(source(), "items", lab.config, _plan()).rows == 0
    db.execute("UPDATE items SET v='B' WHERE id=2")
    db.commit()
    assert load_via_sink(source(), "items", lab.config, _plan()).rows == 1
    assert [(r["v"], r["_is_current"]) for r in _versions(lab, "items")[(2,)]] == [("b", False), ("B", True)]


# --- refusals -------------------------------------------------------------------------


def test_a_source_column_named_like_a_history_column_is_refused_and_nothing_is_created(lab: Lab) -> None:
    with pytest.raises(HistoryRefused) as raised:
        _load(lab, [{"id": 1, "_row_hash": "mine"}], "r")
    assert str(raised.value) == (
        "table 'r': the source has columns named _row_hash; a history table adds _valid_from, _valid_to, "
        "_is_current, _row_hash itself, so rename or leave out those columns"
    )
    assert not list(lab.bucket.glob("**/r/metadata/*.metadata.json"))


def test_a_table_loaded_in_another_mode_is_refused_not_converted(lab: Lab) -> None:
    load_via_sink([{"id": 1, "v": "a"}], "m", lab.config, LoadPlan(mode="replace"))
    rows, commits = lab.rows("m"), lab.commits("m")
    with pytest.raises(HistoryRefused) as raised:
        _load(lab, [{"id": 1, "v": "a"}], "m")
    message = str(raised.value)
    assert "Refresh, removing rows" in message and message.startswith("table 'm' holds rows")
    assert (lab.rows("m"), lab.commits("m")) == (rows, commits)


def test_a_history_table_that_was_then_loaded_in_another_mode_is_refused(lab: Lab) -> None:
    _load(lab, [{"id": 1, "v": "a"}], "back")
    load_via_sink([{"id": 5, "v": "z"}], "back", lab.config, LoadPlan(mode="replace"))
    assert [r["_row_hash"] for r in lab.rows("back")] == [None], "the setup: a row with no history values"
    with pytest.raises(HistoryRefused, match='"Refresh, removing rows"'):
        _load(lab, [{"id": 1, "v": "a"}], "back")


def test_a_null_key_value_is_refused_with_the_dedup_steps_message(lab: Lab) -> None:
    with pytest.raises(dedup.MissingKeyValue) as raised:
        _load(lab, [{"id": 1, "v": "a"}, {"id": None, "v": "secret"}], "n")
    assert str(raised.value) == (
        "table 'n': 1 row(s) have no value for key column 'id'; a deduplicating load needs every row to carry its key"
    )
    assert not list(lab.bucket.glob("**/n/metadata/*.metadata.json"))


def test_a_source_that_holds_a_key_twice_is_deduplicated_to_the_last_row(lab: Lab) -> None:
    result = _load(lab, [{"id": 1, "v": "first"}, {"id": 1, "v": "last"}])
    assert result.rows == 1 and [r["v"] for r in lab.rows("h")] == ["last"]


def test_more_keys_than_the_bound_fail_the_load_before_anything_is_read_from_the_table(
    lab: Lab, monkeypatch: pytest.MonkeyPatch
) -> None:
    _load(lab, [{"id": 1, "v": "a"}])
    rows = lab.rows("h")
    monkeypatch.setenv("INGEST_DEDUP_MAX_ROWS", "2")
    with pytest.raises(dedup.TooManyKeys, match="INGEST_DEDUP_MAX_ROWS"):
        _load(lab, [{"id": i, "v": "x"} for i in range(3)])
    assert lab.rows("h") == rows


def test_a_value_that_does_not_fit_the_columns_type_fails_the_load_and_leaves_the_table(lab: Lab) -> None:
    _load(lab, [{"id": 1, "n": 5}])
    rows, commits = lab.rows("h"), lab.commits("h")
    with pytest.raises(HistoryRefused) as raised:
        _load(lab, [{"id": 1, "n": "five, a text"}])
    assert "five" not in str(raised.value) and "'n'" in str(raised.value)
    assert (lab.rows("h"), lab.commits("h")) == (rows, commits)


# --- refresh and clear -----------------------------------------------------------------


def test_refresh_remove_empties_the_table_and_loads_every_key_as_a_first_version(lab: Lab) -> None:
    _load(lab, [{"id": 1, "v": "a"}, {"id": 2, "v": "b"}])
    _load(lab, [{"id": 1, "v": "A"}, {"id": 2, "v": "b"}])
    assert len(lab.rows("h")) == 3
    result = _load(lab, [{"id": 1, "v": "A"}, {"id": 2, "v": "b"}], refresh="remove")
    assert result.rows == 2
    rows = lab.rows("h")
    assert sorted((r["id"], r["v"]) for r in rows) == [(1, "A"), (2, "b")]
    assert all(r["_is_current"] and r["_valid_to"] is None for r in rows), "no closed row survives the refresh"


def test_refresh_keep_of_a_history_table_is_an_ordinary_history_load(lab: Lab) -> None:
    _load(lab, [{"id": 1, "v": "a"}, {"id": 2, "v": "b"}])
    rows, commits = lab.rows("h"), lab.commits("h")
    unchanged = _load(lab, [{"id": 1, "v": "a"}, {"id": 2, "v": "b"}], refresh="keep")
    assert unchanged.rows == 0 and (lab.rows("h"), lab.commits("h")) == (rows, commits)
    changed = _load(lab, [{"id": 1, "v": "a"}, {"id": 2, "v": "B"}], refresh="keep")
    assert changed.rows == 1 and len(lab.rows("h")) == 3


def test_clear_empties_a_history_table_and_the_next_load_starts_it_over(lab: Lab) -> None:
    _load(lab, [{"id": 1, "v": "a"}])
    _load(lab, [{"id": 1, "v": "b"}])
    clear_table("h", lab.config)
    assert lab.rows("h") == []
    result = _load(lab, [{"id": 1, "v": "b"}])
    assert result.rows == 1
    assert [(r["v"], r["_is_current"]) for r in lab.rows("h")] == [("b", True)]


def test_refresh_remove_turns_a_table_of_another_mode_into_a_history_table(lab: Lab, tmp_path: Path) -> None:
    db = sqlite3.connect(tmp_path / "source.db")
    db.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, v TEXT)")
    db.execute("INSERT INTO items VALUES (1, 'a')")
    db.commit()

    def source() -> Any:
        return sql_database(
            credentials={"drivername": "sqlite", "database": str(tmp_path / "source.db")}, table_names=["items"]
        )

    load_via_sink(source(), "items", lab.config, LoadPlan(mode="replace"))
    result = load_via_sink(source(), "items", lab.config, _plan(refresh="remove"))
    assert result.rows == 1
    assert [(r["v"], r["_is_current"]) for r in lab.rows("items")] == [("a", True)]
    assert load_via_sink(source(), "items", lab.config, _plan()).rows == 0


@pytest.mark.xfail(
    strict=True,
    reason="SRC-10 sixth pass, found while testing: clear_table (fourth pass) leaves the rows of a table that "
    "`load_via_sink` made from PLAIN rows (files, REST, MongoDB, SFTP): 'Delete operation did not match any "
    "records'. It empties a table a sql_database source or the history mode made. Reported, not fixed here.",
)
def test_clear_table_empties_a_table_made_from_plain_rows(lab: Lab) -> None:
    load_via_sink([{"id": 1, "v": "a"}], "plain", lab.config, LoadPlan(mode="replace"))
    clear_table("plain", lab.config)
    assert lab.rows("plain") == []


def test_an_empty_source_writes_nothing_and_leaves_every_key_current(lab: Lab) -> None:
    _load(lab, [{"id": 1, "v": "a"}])
    rows = lab.rows("h")
    result = _load(lab, [])
    assert result.rows == 0 and lab.rows("h") == rows


def test_a_load_time_is_the_time_the_load_started(lab: Lab) -> None:
    before = datetime.now(timezone.utc)
    _load(lab, [{"id": 1, "v": "a"}])
    after = datetime.now(timezone.utc)
    assert before <= lab.rows("h")[0]["_valid_from"] <= after


# --- the ingest op ----------------------------------------------------------------------


def test_the_ingest_factory_plans_history_for_every_batch_adapter() -> None:
    from dispar_orchestrate import ingest_factory as f

    obj = {"name": "t", "target": "t", "loadMode": "history", "primaryKey": ["id"]}
    for adapter in ("sql", "files", "rest", "mongodb", "sftp"):
        assert f._load_plan(adapter, obj) == LoadPlan(mode="history", key=("id",))


def test_a_history_refusal_is_recorded_as_a_rejected_run_with_its_fixed_message(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from dispar_orchestrate import ingest_factory as f

    recorded: list[dict[str, Any]] = []
    monkeypatch.setattr(f, "record_ingest_run", lambda **kw: recorded.append(kw))
    monkeypatch.setattr(f.secret_resolver, "resolve_secret_ref", lambda ref: "s3cret")
    monkeypatch.setattr(f, "_make_gate", lambda *a, **k: None)
    refusal = HistoryRefused('table \'orders\' holds rows that were not loaded in the history mode; use "Refresh, removing rows" on it to load it with history')

    def refuse(cfg: Any, plan: Any, gate: Any = None) -> None:
        raise refusal

    monkeypatch.setattr(f.dlt_pipeline, "run_bronze_ingest", refuse)
    connector = {
        "id": "conn-pg",
        "adapter": "sql",
        "dial": {"driver": "postgres", "host": "db.internal", "port": 5432, "database": "d", "user": "u"},
        "secretRef": "env:CONNECTOR_PG_PASSWORD",
        "secretRefSecondary": None,
    }
    obj = {"name": "public.orders", "target": "orders", "loadMode": "history", "primaryKey": ["id"]}
    with pytest.raises(HistoryRefused):
        f._run_one_object(connector, obj)
    assert recorded[-1]["status"] == "rejected" and recorded[-1]["error"] == str(refusal)
