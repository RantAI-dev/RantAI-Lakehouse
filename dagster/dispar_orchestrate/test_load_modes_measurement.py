"""Measurement of what the loader does when a Bronze table is MERGED on a key,
and of the history (SCD type 2) merge built on top of it (`SRC-10` plan task 1;
findings F3, F5, F7; feature-page decisions D1, D4, D8 and D9).

`docs/plans/SRC-10-RESULT.md` first measured the merge with a throwaway script
on plain rows and a bare local table. Finding F7 says that is not the
product's path. This module repeats it on the path the product uses, and
asserts what it OBSERVES (`AGENTS.md` principle 5): a `dlt` or `pyiceberg`
upgrade that changes one of these fails the matching test, which is the point.
Observations of `dlt` 1.30.0 and `pyiceberg` 0.12.0.

# What is and is not the production path

SAME as production (`adapters/sink.py::load_via_sink`):

- `load_via_sink` ITSELF runs for the cases that need no merge (`replace`,
  `append`): `monkeypatch` swaps only its two network-bound names,
  `sink.filesystem` (the S3 destination becomes a temporary directory) and
  `sink._install_catalog_env` (no Lakekeeper variables). Everything between is
  the real function: the `_ingested_at` stamp, `iceberg_adapter` with the day
  partition on `_ingested_at` and `format-version` 2, `table_format="iceberg"`,
  dataset `bronze`, the pipeline name `bronze_ingest_<table>`.
- `load_via_sink` has no merge mode yet (task 3 adds it), so the merge cases
  call `_merge_load`, which repeats `load_via_sink`'s calls line by line
  (`sink._stamp_for_load`, the same `iceberg_adapter` arguments, the same
  pipeline name and dataset) and differs ONLY in the write disposition
  (`merge`, strategy `upsert`) and in handing the primary key to `pipeline.run`.
- A SQL source is `dlt.sources.sql_database` over SQLite, with the cursor
  attached by `sink._apply_cursor`, as the `incremental` mode does.
- Tables are read back from their Iceberg metadata with `pyiceberg`, as
  `test_schema_drift_loader.py` does.

DIFFERENT from production, and what that means for the observations:

- `dlt` uses its own ephemeral local Iceberg catalog on a local directory, not
  Lakekeeper's REST catalog on RustFS. A commit is one new metadata file in
  both; `_commits` counts those. What a REST catalog does when two writers
  commit at once is NOT measured.
- Nothing here asks ClickHouse or Trino to read a merged table (decision D8:
  the gate does that).
- Rows are a few dozen, not millions. Time and files rewritten on a large
  table are NOT measured; the one size-dependent behaviour that decides the
  design (a merge is committed in slices of 1000 rows, see the history cases)
  is measured at 1500 changed keys.
- `dlt`'s bookkeeping columns (`_dlt_id`, `_dlt_load_id`) are left out of the
  row assertions.

# History (SCD type 2) prototype

`_history_load` is the merge feature-page decision D1 describes, written here
only to measure it: columns `_valid_from`, `_valid_to`, `_is_current`,
`_row_hash`; merge key = the table's key plus `_valid_from`; per load, read the
current row of each key in the load from the table, write the closed version
of every changed key and the new versions in ONE `merge`/`upsert`.
`adapters/history.py` (task 3a) is the real module; this one is the evidence
for its docstring.

Run with:
~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_load_modes_measurement.py -q
"""

from __future__ import annotations

import glob
import hashlib
import json
import sqlite3
import time
from collections.abc import Iterable
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

import dlt
import pyarrow as pa
import pytest
from dlt.destinations import filesystem
from dlt.destinations.adapters import iceberg_adapter, iceberg_partition
from dlt.extract.resource import DltResource
from dlt.extract.source import DltSource
from dlt.pipeline.exceptions import PipelineStepFailed
from dlt.sources.sql_database import sql_database
from pyiceberg.expressions import And, EqualTo, In
from pyiceberg.table import StaticTable, Table, Transaction

from dispar_orchestrate.adapters import sink as sink_module
from dispar_orchestrate.adapters.sink import LoadPlan, SinkConfig, load_via_sink

HISTORY_COLUMNS = ("_valid_from", "_valid_to", "_is_current", "_row_hash")
T0 = datetime(2026, 1, 1, tzinfo=timezone.utc)


class Lab:
    """One temporary bucket, one place `load_via_sink` writes to."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.bucket = root / "bucket"
        self.config = SinkConfig("u", "w", "e", "a", "s", "b", "")

    def destination(self) -> Any:
        return filesystem(bucket_url=str(self.bucket))

    def table(self, name: str) -> Table:
        files = sorted(glob.glob(str(self.bucket / "**" / name / "metadata" / "*.metadata.json"), recursive=True))
        return StaticTable.from_metadata(f"file://{files[-1]}")

    def commits(self, name: str) -> int:
        """The number of commits the table has seen: its metadata log, which
        holds one entry per earlier metadata file (the create-table commit
        included). The difference before and after a load is that load's
        number of commits."""
        return len(self.table(name).metadata.metadata_log)

    def snapshots(self, name: str) -> int:
        return len(self.table(name).snapshots())

    def rows(self, name: str) -> list[dict[str, Any]]:
        """Every row of the table, without `dlt`'s bookkeeping columns."""
        return [
            {k: v for k, v in row.items() if not k.startswith("_dlt_")}
            for row in self.table(name).scan().to_arrow().to_pylist()
        ]

    def pipeline(self, name: str) -> Any:
        return dlt.pipeline(
            pipeline_name=f"bronze_ingest_{name}", destination=self.destination(), dataset_name="bronze"
        )


@pytest.fixture
def lab(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Lab:
    # Same isolation as `test_schema_drift_loader.py`: other test modules leave
    # the `ICEBERG_CATALOG__*` variables `sink._install_catalog_env` sets, and
    # they would send this pipeline to a Lakekeeper that does not exist. And
    # `dlt` keeps pipeline state under `DLT_DATA_DIR`, one directory per test.
    for name in (
        "ICEBERG_CATALOG__ICEBERG_CATALOG_NAME",
        "ICEBERG_CATALOG__ICEBERG_CATALOG_TYPE",
        "ICEBERG_CATALOG__ICEBERG_CATALOG_CONFIG",
    ):
        monkeypatch.delenv(name, raising=False)
    monkeypatch.setenv("DLT_DATA_DIR", str(tmp_path / "dlt"))
    lab = Lab(tmp_path)
    monkeypatch.setattr(sink_module, "filesystem", lambda **_kw: lab.destination())
    monkeypatch.setattr(sink_module, "_install_catalog_env", lambda _config: None)
    return lab


def _load(lab: Lab, source: Any, table: str, mode: str, cursor: str | None = None) -> Any:
    """`load_via_sink` itself, for `replace`, `append` and `incremental`."""
    return load_via_sink(source, table, lab.config, LoadPlan(mode=mode, cursor=cursor))


def _merge_load(lab: Lab, source: Any, table: str, key: list[str]) -> Any:
    """`load_via_sink`'s calls, with the write disposition `merge`/`upsert`
    and the key. See the module docstring: this is the ONLY difference."""
    source = sink_module._stamp_for_load(source)
    adapted = iceberg_adapter(
        source,
        partition=[iceberg_partition.day("_ingested_at")],
        table_properties={"format-version": "2"},
    )
    if not isinstance(source, (DltSource, DltResource)):
        source = adapted
    return lab.pipeline(table).run(
        source,
        table_name=table,
        table_format="iceberg",
        write_disposition={"disposition": "merge", "strategy": "upsert"},
        primary_key=key,
    )


def _by_key(rows: Iterable[dict[str, Any]], *key: str) -> dict[Any, dict[str, Any]]:
    return {tuple(row[k] for k in key) if len(key) > 1 else row[key[0]]: row for row in rows}


def _dedup(rows: list[dict[str, Any]], key: list[str], cursor: str | None = None) -> list[dict[str, Any]]:
    """The dedup a caller must do before a merge (decision D4): one row per
    key, the highest `cursor` value winning, the LAST row read when there is no
    cursor. A stand-in for the function task 3 writes."""
    kept: dict[tuple[Any, ...], dict[str, Any]] = {}
    for row in rows:
        k = tuple(row[c] for c in key)
        if k not in kept or cursor is None or row[cursor] >= kept[k][cursor]:
            kept[k] = row
    return list(kept.values())


# ---------------------------------------------------------------------------
# (a) to (i): merge on a key
# ---------------------------------------------------------------------------


def test_a_merge_on_one_column_keeps_one_row_per_key_across_two_loads(lab: Lab) -> None:
    _merge_load(lab, [{"id": 1, "v": "a"}, {"id": 2, "v": "b"}], "t", ["id"])
    _merge_load(lab, [{"id": 2, "v": "B"}, {"id": 3, "v": "c"}], "t", ["id"])
    rows = _by_key(lab.rows("t"), "id")
    assert {k: r["v"] for k, r in rows.items()} == {1: "a", 2: "B", 3: "c"}
    assert len(lab.rows("t")) == 3


def test_a_merge_on_a_composite_key_matches_on_every_column_of_the_key(lab: Lab) -> None:
    _merge_load(lab, [{"a": 1, "b": 1, "v": "x"}, {"a": 1, "b": 2, "v": "y"}], "c", ["a", "b"])
    _merge_load(lab, [{"a": 1, "b": 2, "v": "Y"}, {"a": 2, "b": 1, "v": "z"}], "c", ["a", "b"])
    rows = _by_key(lab.rows("c"), "a", "b")
    assert {k: r["v"] for k, r in rows.items()} == {(1, 1): "x", (1, 2): "Y", (2, 1): "z"}
    assert len(lab.rows("c")) == 3


def test_a_batch_that_holds_a_key_twice_fails_the_load_and_changes_nothing(lab: Lab) -> None:
    _merge_load(lab, [{"id": 1, "v": "a"}], "d", ["id"])
    commits = lab.commits("d")
    with pytest.raises(PipelineStepFailed):
        _merge_load(lab, [{"id": 1, "v": "b"}, {"id": 1, "v": "c"}], "d", ["id"])
    assert [r["v"] for r in lab.rows("d")] == ["a"]
    assert lab.commits("d") == commits


def test_the_same_batch_loads_after_the_caller_keeps_the_row_with_the_highest_cursor_value(lab: Lab) -> None:
    _merge_load(lab, [{"id": 1, "v": "a", "seq": 1}], "dd", ["id"])
    batch = [{"id": 1, "v": "new", "seq": 9}, {"id": 1, "v": "old", "seq": 3}, {"id": 2, "v": "b", "seq": 1}]
    _merge_load(lab, _dedup(batch, ["id"], cursor="seq"), "dd", ["id"])
    assert {r["id"]: r["v"] for r in lab.rows("dd")} == {1: "new", 2: "b"}


def test_the_last_row_read_wins_when_there_is_no_cursor(lab: Lab) -> None:
    batch = [{"id": 1, "v": "first"}, {"id": 1, "v": "last"}]
    _merge_load(lab, _dedup(batch, ["id"]), "dl", ["id"])
    assert [r["v"] for r in lab.rows("dl")] == ["last"]


def test_a_row_whose_key_is_null_fails_the_load_before_anything_is_written(lab: Lab) -> None:
    # `dlt` makes a primary-key column NOT NULL, so the NORMALIZE step refuses
    # the row ("Cannot coerce NULL ... which is not nullable"). The loader
    # therefore cannot merge a null key; task 3 has to decide what the user
    # sees (this is reported to the planner).
    with pytest.raises(PipelineStepFailed) as raised:
        _merge_load(lab, [{"id": 1, "v": "a"}, {"id": None, "v": "n"}], "n", ["id"])
    assert "normalize" in str(raised.value)
    assert not list(lab.bucket.glob("**/n/metadata/*.metadata.json"))


def test_a_merge_after_a_replace_merges_into_the_rows_the_replace_left(lab: Lab) -> None:
    _load(lab, dlt.resource([{"id": 1, "v": "a"}, {"id": 2, "v": "b"}], name="t"), "t", "replace")
    _merge_load(lab, [{"id": 2, "v": "B"}, {"id": 3, "v": "c"}], "t", ["id"])
    assert {r["id"]: r["v"] for r in lab.rows("t")} == {1: "a", 2: "B", 3: "c"}


def test_a_merge_into_a_table_first_loaded_with_append_that_holds_a_key_twice_fails_and_blocks_the_next_load(
    lab: Lab,
) -> None:
    # Nothing is merged silently: pyiceberg refuses ("Target table has duplicate
    # rows") and the load fails. But the failed package stays pending in the
    # pipeline's working directory, so a LATER, clean load of the same table
    # fails too until the package is dropped (`dlt pipeline ... abort-packages`).
    _load(lab, [{"id": 1, "v": "a"}, {"id": 1, "v": "a2"}, {"id": 2, "v": "b"}], "g", "append")
    commits = lab.commits("g")
    with pytest.raises(PipelineStepFailed):
        _merge_load(lab, [{"id": 1, "v": "NEW"}], "g", ["id"])
    assert sorted(r["v"] for r in lab.rows("g")) == ["a", "a2", "b"]
    assert lab.commits("g") == commits
    with pytest.raises(PipelineStepFailed):
        _merge_load(lab, [{"id": 7, "v": "clean"}], "g", ["id"])


def test_a_merge_after_an_append_that_holds_no_duplicate_merges_normally(lab: Lab) -> None:
    _load(lab, [{"id": 1, "v": "a"}, {"id": 2, "v": "b"}], "ga", "append")
    _merge_load(lab, [{"id": 2, "v": "B"}], "ga", ["id"])
    assert {r["id"]: r["v"] for r in lab.rows("ga")} == {1: "a", 2: "B"}


def test_a_merge_keeps_the_day_partition_and_stamps_only_the_rows_it_wrote(lab: Lab) -> None:
    _merge_load(lab, [{"id": 1, "v": "a"}, {"id": 2, "v": "b"}], "p", ["id"])
    spec_before = lab.table("p").spec()
    first = _by_key(lab.rows("p"), "id")
    time.sleep(1.1)
    _merge_load(lab, [{"id": 2, "v": "B"}, {"id": 3, "v": "c"}], "p", ["id"])
    table = lab.table("p")
    assert str(table.spec()) == str(spec_before)
    assert [f.name for f in table.spec().fields] == ["_ingested_at_day"]
    assert table.metadata.properties.get("format-version", "2") == "2" and table.metadata.format_version == 2
    after = _by_key(lab.rows("p"), "id")
    assert after[1]["_ingested_at"] == first[1]["_ingested_at"], "an untouched row keeps its time"
    assert after[2]["_ingested_at"] > first[2]["_ingested_at"], "an updated row carries the time of the merge"
    assert after[3]["_ingested_at"] >= after[2]["_ingested_at"]
    assert all(row["_ingested_at"] is not None for row in after.values())


def test_a_merge_with_one_update_and_one_insert_is_one_commit_and_three_snapshots(lab: Lab) -> None:
    _merge_load(lab, [{"id": 1, "v": "a"}, {"id": 2, "v": "b"}], "s", ["id"])
    commits, snaps = lab.commits("s"), lab.snapshots("s")
    _merge_load(lab, [{"id": 2, "v": "B"}, {"id": 3, "v": "c"}], "s", ["id"])
    # pyiceberg's upsert runs one transaction: an overwrite (delete the old
    # file, write the changed row) and an append (the new key). Readers see the
    # table before or after, never between: ONE commit, three snapshots.
    assert lab.commits("s") - commits == 1
    assert lab.snapshots("s") - snaps == 3


def test_a_merge_of_new_keys_only_is_one_commit_and_one_snapshot(lab: Lab) -> None:
    _merge_load(lab, [{"id": 1, "v": "a"}], "s2", ["id"])
    commits, snaps = lab.commits("s2"), lab.snapshots("s2")
    _merge_load(lab, [{"id": 2, "v": "b"}], "s2", ["id"])
    assert (lab.commits("s2") - commits, lab.snapshots("s2") - snaps) == (1, 1)


def test_a_merge_of_rows_identical_to_the_table_changes_nothing_but_the_row_count_dlt_reports(lab: Lab) -> None:
    _merge_load(lab, [{"id": 1, "v": "a"}], "same", ["id"])
    commits = lab.commits("same")
    before = lab.rows("same")
    time.sleep(1.1)
    # The stamp `_ingested_at` differs on every load, so the "identical" row is
    # NOT identical to pyiceberg: a repeated merge still updates the row.
    _merge_load(lab, [{"id": 1, "v": "a"}], "same", ["id"])
    assert lab.commits("same") - commits == 1
    assert lab.rows("same")[0]["_ingested_at"] > before[0]["_ingested_at"]
    assert len(lab.rows("same")) == 1


# (f) an incremental (cursor-filtered) SQL load followed by a merge.


@pytest.fixture
def items_db(tmp_path: Path) -> sqlite3.Connection:
    connection = sqlite3.connect(tmp_path / "source.db")
    connection.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, v TEXT, updated_at INTEGER)")
    connection.executemany("INSERT INTO items VALUES (?,?,?)", [(1, "a", 10), (2, "b", 20), (3, "c", 30)])
    connection.commit()
    return connection


def _items_source(tmp_path: Path) -> Any:
    source = sql_database(
        credentials={"drivername": "sqlite", "database": str(tmp_path / "source.db")}, table_names=["items"]
    )
    sink_module._apply_cursor(source, "updated_at")
    return source


def test_a_cursor_filtered_sql_load_merges_only_the_changed_row_and_the_new_row(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path
) -> None:
    _merge_load(lab, _items_source(tmp_path), "items", ["id"])
    first = _by_key(lab.rows("items"), "id")
    assert {k: r["v"] for k, r in first.items()} == {1: "a", 2: "b", 3: "c"}
    time.sleep(1.1)
    items_db.execute("UPDATE items SET v='B', updated_at=40 WHERE id=2")
    items_db.execute("INSERT INTO items VALUES (4,'d',50)")
    items_db.commit()
    commits = lab.commits("items")
    _merge_load(lab, _items_source(tmp_path), "items", ["id"])
    second = _by_key(lab.rows("items"), "id")
    assert {k: r["v"] for k, r in second.items()} == {1: "a", 2: "B", 3: "c", 4: "d"}
    assert lab.commits("items") - commits == 1
    assert second[1]["_ingested_at"] == first[1]["_ingested_at"]
    assert second[3]["_ingested_at"] == first[3]["_ingested_at"]
    assert second[2]["_ingested_at"] > first[2]["_ingested_at"]
    assert [f.name for f in lab.table("items").spec().fields] == ["_ingested_at_day"]


def test_a_cursor_filtered_sql_load_with_nothing_new_makes_no_commit(
    lab: Lab, items_db: sqlite3.Connection, tmp_path: Path
) -> None:
    _merge_load(lab, _items_source(tmp_path), "items", ["id"])
    commits = lab.commits("items")
    _merge_load(lab, _items_source(tmp_path), "items", ["id"])
    assert lab.commits("items") == commits
    assert len(lab.rows("items")) == 3


# ---------------------------------------------------------------------------
# History (SCD type 2) prototype and its measurements (j) to (o)
# ---------------------------------------------------------------------------


def _row_hash(row: dict[str, Any], key: list[str]) -> str:
    """Prototype hash: SHA-256 of the non-key source columns in name order.
    Task 3a defines the real one (typed, null-safe)."""
    body = [[c, row[c]] for c in sorted(row) if c not in key and not c.startswith("_")]
    return hashlib.sha256(json.dumps(body, default=str).encode()).hexdigest()


def _history_resource(rows: list[dict[str, Any]], table: str, key: list[str]) -> DltResource:
    # Type hints are needed: `_valid_to` is null in a first load, and `dlt`
    # cannot infer a column from nulls alone (it would leave it out).
    return dlt.resource(
        rows,
        name=table,
        columns={
            "_valid_from": {"data_type": "timestamp", "nullable": False},
            "_valid_to": {"data_type": "timestamp", "nullable": True},
            "_is_current": {"data_type": "bool", "nullable": False},
            "_row_hash": {"data_type": "text", "nullable": False},
        },
    )


def _current_rows(
    lab: Lab, table: str, key: list[str], keys: list[tuple[Any, ...]], chunk: int = 1000
) -> dict[tuple[Any, ...], dict[str, Any]]:
    """The current row of each key in `keys`, read from the table through the
    destination client of the very pipeline that writes it (`dlt`'s own catalog
    configuration -- in production the Lakekeeper REST catalog that
    `sink._install_catalog_env` installs), with a row filter on `_is_current`
    and the key, in chunks. No query engine."""
    try:
        with lab.pipeline(table).destination_client() as client:  # type: ignore[attr-defined]
            iceberg_table = client.load_open_table("iceberg", table)
    except Exception:  # noqa: BLE001 -- measurement helper: a table not made yet has no current rows
        return {}
    found: dict[tuple[Any, ...], dict[str, Any]] = {}
    for start in range(0, len(keys), chunk):
        part = keys[start : start + chunk]
        # A filter on every key column separately is a superset of the pairs
        # asked for (a composite key is exact only after the check below).
        row_filter: Any = EqualTo("_is_current", True)
        for position, column in enumerate(key):
            row_filter = And(row_filter, In(column, sorted({k[position] for k in part})))
        wanted = set(part)
        for row in iceberg_table.scan(row_filter=row_filter).to_arrow().to_pylist():
            k = tuple(row[c] for c in key)
            if k in wanted:
                found[k] = row
    return found


def _history_load(
    lab: Lab, table: str, key: list[str], rows: list[dict[str, Any]], now: datetime, *, closed_first: bool = True
) -> int:
    """One history load of `rows` (already one row per key). Returns the number
    of rows handed to the merge; `0` means nothing was written, and no commit
    was attempted."""
    current = _current_rows(lab, table, key, [tuple(r[c] for c in key) for r in rows])
    closed: list[dict[str, Any]] = []
    opened: list[dict[str, Any]] = []
    for row in rows:
        k = tuple(row[c] for c in key)
        digest = _row_hash(row, key)
        version = {**row, "_valid_from": now, "_valid_to": None, "_is_current": True, "_row_hash": digest}
        existing = current.get(k)
        if existing is None:
            opened.append(version)
        elif existing["_row_hash"] != digest:
            kept = {c: v for c, v in existing.items() if not c.startswith("_dlt_")}
            closed.append({**kept, "_valid_to": now, "_is_current": False})
            opened.append(version)
    batch = [*closed, *opened] if closed_first else [*opened, *closed]
    if not batch:
        return 0
    _merge_load(lab, _history_resource(batch, table, key), table, [*key, "_valid_from"])
    return len(batch)


def _versions(lab: Lab, table: str) -> dict[Any, list[dict[str, Any]]]:
    out: dict[Any, list[dict[str, Any]]] = {}
    for row in lab.rows(table):
        out.setdefault(row["id"], []).append(row)
    for versions in out.values():
        versions.sort(key=lambda r: r["_valid_from"])
    return out


def _two_history_loads(lab: Lab, table: str = "h") -> None:
    first = [{"id": 1, "v": "a"}, {"id": 2, "v": "b"}]
    assert _history_load(lab, table, ["id"], first, T0) == 2
    second = [{"id": 1, "v": "a"}, {"id": 2, "v": "B"}, {"id": 3, "v": "c"}]
    assert _history_load(lab, table, ["id"], second, T0 + timedelta(days=1)) == 3


def test_history_two_loads_close_the_changed_row_add_the_new_ones_and_leave_the_unchanged_one(lab: Lab) -> None:
    """(j)"""
    _two_history_loads(lab)
    versions = _versions(lab, "h")
    assert len(lab.rows("h")) == 4
    assert len(versions[1]) == 1, "the unchanged key has no second version"
    assert versions[1][0]["_is_current"] is True and versions[1][0]["_valid_to"] is None
    assert versions[1][0]["_valid_from"] == T0
    old, new = versions[2]
    assert (old["v"], old["_is_current"], old["_valid_from"], old["_valid_to"]) == ("b", False, T0, T0 + timedelta(days=1))
    assert (new["v"], new["_is_current"], new["_valid_from"], new["_valid_to"]) == (
        "B",
        True,
        T0 + timedelta(days=1),
        None,
    )
    assert len(versions[3]) == 1 and versions[3][0]["_is_current"] is True
    assert sum(1 for r in lab.rows("h") if not r["_is_current"]) == 1, "exactly one closed row"
    for rows in versions.values():
        assert sum(1 for r in rows if r["_is_current"]) == 1, "one current row per key"


def test_history_a_third_identical_load_writes_no_row_and_adds_no_snapshot(lab: Lab) -> None:
    """(k) Nothing is handed to the merge, so nothing is committed."""
    _two_history_loads(lab)
    commits, snaps, rows = lab.commits("h"), lab.snapshots("h"), lab.rows("h")
    third = [{"id": 1, "v": "a"}, {"id": 2, "v": "B"}, {"id": 3, "v": "c"}]
    assert _history_load(lab, "h", ["id"], third, T0 + timedelta(days=2)) == 0
    assert (lab.commits("h"), lab.snapshots("h")) == (commits, snaps)
    assert lab.rows("h") == rows


def test_history_an_empty_merge_would_also_make_no_commit_so_the_skip_is_not_load_bearing(lab: Lab) -> None:
    """(k) The same, if the loader ran the pipeline with no rows at all."""
    _two_history_loads(lab)
    commits = lab.commits("h")
    _merge_load(lab, _history_resource([], "h", ["id"]), "h", ["id", "_valid_from"])
    assert lab.commits("h") == commits


def test_history_each_small_load_is_exactly_one_commit(lab: Lab) -> None:
    """(l) A first load of two rows, and a second with one closed row, one
    new version and one new key: one commit each."""
    first = [{"id": 1, "v": "a"}, {"id": 2, "v": "b"}]
    _history_load(lab, "h", ["id"], first[:1], T0)  # makes the table
    commits = lab.commits("h")
    _history_load(lab, "h", ["id"], first, T0 + timedelta(days=1))
    assert lab.commits("h") - commits == 1
    commits = lab.commits("h")
    _history_load(lab, "h", ["id"], [{"id": 1, "v": "A"}, {"id": 2, "v": "b"}, {"id": 3, "v": "c"}], T0 + timedelta(days=2))
    assert lab.commits("h") - commits == 1


def test_history_a_history_merge_keeps_the_day_partition(lab: Lab) -> None:
    _two_history_loads(lab)
    table = lab.table("h")
    assert [f.name for f in table.spec().fields] == ["_ingested_at_day"]
    assert table.metadata.format_version == 2
    assert {f.name for f in table.schema().fields} >= set(HISTORY_COLUMNS)


def test_history_the_current_rows_of_a_load_are_read_through_the_pipelines_own_catalog_with_a_key_filter(
    lab: Lab,
) -> None:
    """(m) Reading is a plain pyiceberg scan, in chunks; it commits nothing."""
    _two_history_loads(lab)
    commits = lab.commits("h")
    current = _current_rows(lab, "h", ["id"], [(2,), (3,), (99,)])
    assert sorted(current) == [(2,), (3,)]
    assert current[(2,)]["v"] == "B" and current[(2,)]["_is_current"] is True
    assert lab.commits("h") == commits, "the read commits nothing (no schema evolution on an existing table)"
    # chunking returns the same rows as one scan
    assert _current_rows(lab, "h", ["id"], [(1,), (2,), (3,)], chunk=1) == _current_rows(
        lab, "h", ["id"], [(1,), (2,), (3,)]
    )
    assert _current_rows(lab, "never_made", ["id"], [(1,)]) == {}


def test_history_a_key_filter_skips_files_whose_key_range_misses_it_and_reads_all_when_ranges_overlap(
    lab: Lab,
) -> None:
    """(m) How much of the table a small load reads. pyiceberg prunes by the
    files' min/max statistics of the key column, so a table whose loads brought
    DISJOINT key ranges (an auto-increment key) is read in one file of four;
    one whose files each span the key range (a random key) is read whole."""
    for number in range(4):
        keys = range(number * 100, number * 100 + 100)
        _history_load(lab, "ordered", ["id"], [{"id": i, "v": "x"} for i in keys], T0 + timedelta(days=number))
        spread = range(number, 400, 4)
        _history_load(lab, "spread", ["id"], [{"id": i, "v": "x"} for i in spread], T0 + timedelta(days=number))
    for table, expected_files in (("ordered", 1), ("spread", 4)):
        with lab.pipeline(table).destination_client() as client:  # type: ignore[attr-defined]
            iceberg_table = client.load_open_table("iceberg", table)
        scan = iceberg_table.scan(row_filter=And(EqualTo("_is_current", True), In("id", [150])))
        assert len(list(iceberg_table.scan().plan_files())) == 4
        assert len(list(scan.plan_files())) == expected_files


def test_history_a_failed_write_leaves_the_table_as_it_was_with_no_closed_row_and_no_commit(
    lab: Lab, monkeypatch: pytest.MonkeyPatch
) -> None:
    """(n) For a load that fits one upsert batch (at most 1000 rows). The
    failure is injected at the worst moment: AFTER pyiceberg has staged the
    overwrite that closes the old row and BEFORE the append of the new
    version. The transaction is never committed, so nothing is visible."""
    first = [{"id": 1, "v": "a"}, {"id": 2, "v": "b"}]
    _history_load(lab, "h", ["id"], first, T0)
    commits, rows = lab.commits("h"), lab.rows("h")

    original_append, original_overwrite = Transaction.append, Transaction.overwrite
    staged_overwrites: list[int] = []

    def overwrite_that_is_recorded(self: Transaction, df: pa.Table, *args: Any, **kwargs: Any) -> None:
        staged_overwrites.append(df.num_rows)
        return original_overwrite(self, df, *args, **kwargs)

    def append_that_fails_for_new_versions(self: Transaction, df: pa.Table, *args: Any, **kwargs: Any) -> None:
        if True in df.column("_is_current").to_pylist():
            raise RuntimeError("injected failure after the closing overwrite was staged")
        return original_append(self, df, *args, **kwargs)

    monkeypatch.setattr(Transaction, "overwrite", overwrite_that_is_recorded)
    monkeypatch.setattr(Transaction, "append", append_that_fails_for_new_versions)
    with pytest.raises(PipelineStepFailed):
        _history_load(lab, "h", ["id"], [{"id": 1, "v": "A"}, {"id": 2, "v": "b"}, {"id": 3, "v": "c"}], T0 + timedelta(days=1))
    assert staged_overwrites, "the closing overwrite WAS staged before the failure"
    assert lab.commits("h") == commits
    assert lab.rows("h") == rows
    assert not [r for r in lab.rows("h") if not r["_is_current"]]


def test_history_a_load_dying_after_the_read_and_before_the_write_has_written_nothing(lab: Lab) -> None:
    """(n) The read opens no commit and holds no state; if the process dies
    between it and the write, the table is what it was and the next run reads
    the same current rows and decides the same."""
    _history_load(lab, "h", ["id"], [{"id": 1, "v": "a"}], T0)
    commits, rows = lab.commits("h"), lab.rows("h")
    _current_rows(lab, "h", ["id"], [(1,)])  # ... and the process dies here
    assert lab.commits("h") == commits and lab.rows("h") == rows
    assert _history_load(lab, "h", ["id"], [{"id": 1, "v": "A"}], T0 + timedelta(days=1)) == 2


def _fail_second_slice(monkeypatch: pytest.MonkeyPatch) -> None:
    """Make every upsert slice that carries a NEW current version fail, as a
    crash or a catalog conflict on the second commit would."""
    original = Table.upsert

    def upsert(self: Table, df: pa.Table, *args: Any, **kwargs: Any) -> Any:
        if True in df.column("_is_current").to_pylist():
            raise RuntimeError("injected failure on the slice that opens new versions")
        return original(self, df, *args, **kwargs)

    monkeypatch.setattr(Table, "upsert", upsert)


def test_history_a_load_of_1500_changed_keys_is_three_commits_not_one(lab: Lab) -> None:
    """(l) dlt's Iceberg merge (`dlt/common/libs/pyiceberg.py::merge_iceberg_table`)
    calls `table.upsert` once per 1000 rows, each its own transaction and
    commit; its own TODO says so. 1500 changed keys are 3000 rows: three
    commits. STOP CONDITION of the plan (the history load is more than one
    commit)."""
    keys = list(range(1500))
    _history_load(lab, "big", ["id"], [{"id": i, "v": "old"} for i in keys], T0)
    commits = lab.commits("big")
    assert _history_load(lab, "big", ["id"], [{"id": i, "v": "new"} for i in keys], T0 + timedelta(days=1)) == 3000
    assert lab.commits("big") - commits == 3
    assert sum(1 for r in lab.rows("big") if r["_is_current"]) == 1500


def test_history_a_failure_between_the_slices_of_a_big_load_leaves_closed_rows_without_a_successor(
    lab: Lab, monkeypatch: pytest.MonkeyPatch
) -> None:
    """(n) The same 3000 rows, closed rows first, the slice that opens new
    versions failing. The first slice (1000 closed rows) is already committed:
    those keys now have NO current row. STOP CONDITION of the plan (a closed
    row without its successor)."""
    keys = list(range(1500))
    _history_load(lab, "big", ["id"], [{"id": i, "v": "old"} for i in keys], T0)
    _fail_second_slice(monkeypatch)
    with pytest.raises(PipelineStepFailed):
        _history_load(lab, "big", ["id"], [{"id": i, "v": "new"} for i in keys], T0 + timedelta(days=1))
    by_key = _versions(lab, "big")
    without_successor = [k for k, versions in by_key.items() if not any(r["_is_current"] for r in versions)]
    assert len(without_successor) == 1000


def test_history_one_pyiceberg_transaction_writes_the_same_3000_rows_in_one_commit_and_rolls_back_whole(
    lab: Lab, monkeypatch: pytest.MonkeyPatch
) -> None:
    """(o) What would make "one commit" true at any size: take the table from
    the destination client and run ONE `Transaction.upsert` on the whole batch,
    outside `dlt`'s load step. Prototype only: it needs `_dlt_id` and
    `_dlt_load_id` filled by hand because the table declares them required, and
    it leaves `dlt`'s package, schema and state bookkeeping out. Not a decision;
    the planner chooses."""
    keys = list(range(1500))
    _history_load(lab, "big", ["id"], [{"id": i, "v": "old"} for i in keys], T0)
    with lab.pipeline("big").destination_client() as client:  # type: ignore[attr-defined]
        iceberg_table = client.load_open_table("iceberg", "big")
    old = {r["id"]: r for r in iceberg_table.scan().to_arrow().to_pylist()}
    now = T0 + timedelta(days=1)
    rows: list[dict[str, Any]] = []
    for i in keys:
        rows.append({**old[i], "_valid_to": now, "_is_current": False})
    for i in keys:
        rows.append(
            {
                **old[i],
                "v": "new",
                "_valid_from": now,
                "_valid_to": None,
                "_is_current": True,
                "_row_hash": "h2",
                "_dlt_id": f"n{i}",
                "_dlt_load_id": "manual",
            }
        )
    arrow = pa.Table.from_pylist(rows, schema=iceberg_table.schema().as_arrow())

    original_append = Transaction.append

    def failing(self: Transaction, df: pa.Table, *args: Any, **kwargs: Any) -> None:
        raise RuntimeError("injected")

    commits = lab.commits("big")
    monkeypatch.setattr(Transaction, "append", failing)
    with pytest.raises(RuntimeError):
        iceberg_table.upsert(arrow, join_cols=["id", "_valid_from"])
    monkeypatch.setattr(Transaction, "append", original_append)
    assert lab.commits("big") == commits, "a failure inside the transaction commits nothing"

    iceberg_table.upsert(arrow, join_cols=["id", "_valid_from"])
    assert lab.commits("big") - commits == 1
    assert sum(1 for r in lab.rows("big") if r["_is_current"]) == 1500
    assert sum(1 for r in lab.rows("big") if not r["_is_current"]) == 1500
