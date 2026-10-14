"""A table that holds history takes no load in another mode, and the other way round
(`SRC-10 review BLOCKER 5`; plan section 3b: "Switching a table to or from `history`
when it already holds rows is refused ... it is never converted silently").

The four history columns are nullable, so nothing in Iceberg stops a `replace`, `append`,
`incremental` or deduplicating load from adding rows with no history values to a history
table; `history.refuse_history_rows` refuses it before anything is written. Expected
values come from that rule and from the fixed message, not from a run.
"""

from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import Any

import pytest
from dlt.sources.sql_database import sql_database

from dispar_orchestrate.adapters import history
from dispar_orchestrate.adapters.history import HISTORY_COLUMNS, HISTORY_HELD, HistoryRefused
from dispar_orchestrate.adapters.sink import LoadPlan, clear_table, load_via_sink
from dispar_orchestrate.test_load_modes_measurement import Lab, lab  # noqa: F401 -- `lab` is a fixture

# Runs a real dlt pipeline against a local Iceberg table (registered in pyproject.toml);
# CI runs these in `dagster-loader-tests`, the rest in `dagster-unit-tests`.
pytestmark = pytest.mark.loader

FIRST = [(1, "zebra-one", 10), (2, "zebra-two", 20), (3, "zebra-three", 30)]
SECOND = [(2, "B", 25), (3, "c", 30), (4, "d", 40)]

# Every mode family with the origins that can run it (an incremental mode takes a `sql_database` source only).
FAMILIES = [
    ("plain", "replace"),
    ("plain", "append"),
    ("plain", "replace_dedup"),
    ("sql", "replace"),
    ("sql", "append"),
    ("sql", "incremental"),
    ("sql", "incremental_dedup"),
    ("sql", "replace_dedup"),
]


def _source(origin: str, tmp_path: Path, rows: list[tuple[int, str, int]]) -> Any:
    if origin == "plain":
        return [{"id": i, "v": v, "updated_at": u} for i, v, u in rows]
    db = sqlite3.connect(tmp_path / "source.db")
    db.execute("DROP TABLE IF EXISTS items")
    db.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, v TEXT, updated_at INTEGER)")
    db.executemany("INSERT INTO items VALUES (?,?,?)", rows)
    db.commit()
    db.close()
    return sql_database(
        credentials={"drivername": "sqlite", "database": str(tmp_path / "source.db")}, table_names=["items"]
    )


def _plan(mode: str, refresh: str | None = None) -> LoadPlan:
    return LoadPlan(
        mode=mode,
        cursor="updated_at" if mode.startswith("incremental") else None,
        key=("id",) if mode in ("incremental_dedup", "replace_dedup", "history") else (),
        refresh=refresh,
    )


def _history_table(lab: Lab, tmp_path: Path, origin: str, table: str = "t") -> None:
    load_via_sink(_source(origin, tmp_path, FIRST), table, lab.config, _plan("history"))
    assert len(lab.rows(table)) == 3 and all(r["_row_hash"] for r in lab.rows(table)), "the setup: history rows"


@pytest.mark.parametrize(("origin", "mode"), FAMILIES)
def test_a_load_in_another_mode_into_a_table_holding_history_is_refused_and_writes_nothing(
    lab: Lab, tmp_path: Path, origin: str, mode: str
) -> None:
    _history_table(lab, tmp_path, origin)
    rows, commits, snapshots = lab.rows("t"), lab.commits("t"), lab.snapshots("t")

    with pytest.raises(HistoryRefused) as raised:
        load_via_sink(_source(origin, tmp_path, SECOND), "t", lab.config, _plan(mode))

    assert str(raised.value) == HISTORY_HELD.format(table="t")
    assert '"Refresh, removing rows"' in str(raised.value)
    assert "zebra" not in str(raised.value), "a table name and the way out, never row data"
    assert (lab.rows("t"), lab.commits("t"), lab.snapshots("t")) == (rows, commits, snapshots)


@pytest.mark.parametrize(("origin", "mode"), FAMILIES)
def test_the_refusal_is_the_config_shaped_kind_the_ingest_op_records_as_rejected(
    lab: Lab, tmp_path: Path, origin: str, mode: str
) -> None:
    """`ingest_factory` lists `DedupRefused` among the exceptions it records as a
    `rejected` run and raises as a non-retryable `Failure`; the refusal is one."""
    from dispar_orchestrate.adapters import dedup

    _history_table(lab, tmp_path, origin)
    with pytest.raises(dedup.DedupRefused):
        load_via_sink(_source(origin, tmp_path, SECOND), "t", lab.config, _plan(mode))


@pytest.mark.parametrize(("origin", "mode"), FAMILIES)
def test_after_a_refresh_that_removes_rows_the_table_takes_the_other_mode_and_keeps_taking_it(
    lab: Lab, tmp_path: Path, origin: str, mode: str
) -> None:
    _history_table(lab, tmp_path, origin)

    refreshed = load_via_sink(_source(origin, tmp_path, SECOND), "t", lab.config, _plan(mode, "remove"))

    assert sorted((r["id"], r["v"]) for r in lab.rows("t")) == [(2, "B"), (3, "c"), (4, "d")]
    assert refreshed.rows == 3
    # The history columns stay as nullable columns, null in every row (the rows hold no history).
    assert all(r[c] is None for r in lab.rows("t") for c in HISTORY_COLUMNS)
    # They are not the source's: the schema observation of this load does not list them.
    assert not {c.name for c in refreshed.columns} & set(HISTORY_COLUMNS)
    assert {c.name for c in refreshed.columns} == {"id", "v", "updated_at"}

    # A later load of the mode is not refused: the table has the columns but holds no history row.
    again = load_via_sink(_source(origin, tmp_path, SECOND), "t", lab.config, _plan(mode))
    assert {r["id"] for r in lab.rows("t")} == {2, 3, 4}
    assert again.rows is not None
    assert not {c.name for c in again.columns} & set(HISTORY_COLUMNS)


@pytest.mark.parametrize(("origin", "mode"), FAMILIES)
def test_after_a_clear_the_table_takes_the_other_mode(lab: Lab, tmp_path: Path, origin: str, mode: str) -> None:
    _history_table(lab, tmp_path, origin)
    clear_table("t", lab.config)
    load_via_sink(_source(origin, tmp_path, SECOND), "t", lab.config, _plan(mode))
    assert {r["id"] for r in lab.rows("t")} == {2, 3, 4}


@pytest.mark.parametrize(("origin", "mode"), FAMILIES)
def test_a_table_of_another_mode_with_rows_is_refused_as_history_and_takes_it_after_a_refresh(
    lab: Lab, tmp_path: Path, origin: str, mode: str
) -> None:
    load_via_sink(_source(origin, tmp_path, FIRST), "t", lab.config, _plan(mode))
    rows, commits = lab.rows("t"), lab.commits("t")

    with pytest.raises(HistoryRefused, match='"Refresh, removing rows"'):
        load_via_sink(_source(origin, tmp_path, SECOND), "t", lab.config, _plan("history"))
    assert (lab.rows("t"), lab.commits("t")) == (rows, commits)

    result = load_via_sink(_source(origin, tmp_path, SECOND), "t", lab.config, _plan("history", "remove"))
    assert result.rows == 3
    assert sorted((r["id"], r["v"]) for r in lab.rows("t")) == [(2, "B"), (3, "c"), (4, "d")]
    assert all(r["_is_current"] for r in lab.rows("t"))


def test_a_refused_history_table_is_refused_from_a_fresh_container_too(
    lab: Lab, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The check reads the schema `dlt` stored with the data, not a local copy: a
    recreated container (a new `DLT_DATA_DIR`) has none."""
    _history_table(lab, tmp_path, "plain")
    monkeypatch.setenv("DLT_DATA_DIR", str(tmp_path / "fresh-container"))
    with pytest.raises(HistoryRefused):
        load_via_sink(_source("plain", tmp_path, SECOND), "t", lab.config, _plan("replace"))
    with pytest.raises(HistoryRefused):
        load_via_sink(_source("sql", tmp_path, SECOND), "t", lab.config, _plan("append"))


def test_a_table_without_the_history_columns_costs_no_catalog_round_trip(
    lab: Lab, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The cost of the check for a table that never held history: the schemas `dlt`
    holds, and nothing else (no table opened). A never-loaded table costs the same."""
    opened: list[str] = []
    real = history.open_table
    monkeypatch.setattr(history, "open_table", lambda *a, **k: opened.append(a[1]) or real(*a, **k))

    load_via_sink(_source("plain", tmp_path, FIRST), "plain_t", lab.config, _plan("replace"))  # never loaded
    load_via_sink(_source("plain", tmp_path, SECOND), "plain_t", lab.config, _plan("append"))
    load_via_sink(_source("sql", tmp_path, SECOND), "sql_t", lab.config, _plan("replace"))

    assert opened == []


def test_a_table_with_the_history_columns_opens_the_catalog_once_per_load(lab: Lab, tmp_path: Path) -> None:
    """...and the other side of that cost: with the columns, one open and one filtered
    `limit 1` read."""
    _history_table(lab, tmp_path, "plain")
    clear_table("t", lab.config)
    opened: list[str] = []
    real = history.open_table
    history.open_table = lambda *a, **k: opened.append(a[1]) or real(*a, **k)  # type: ignore[assignment]
    try:
        load_via_sink(_source("plain", tmp_path, SECOND), "t", lab.config, _plan("replace"))
    finally:
        history.open_table = real  # type: ignore[assignment]
    assert opened == ["t"]
