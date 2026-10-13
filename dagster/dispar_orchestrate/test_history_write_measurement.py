"""Measurement of the history (SCD type 2) write on the loader's real table
set-up (`SRC-10` sixth pass, task H1; feature-page decisions 9 and 10; plan
section "3b").

The first pass (`test_load_modes_measurement.py`, cases j to o) found that the
loader's own merge cuts a batch into slices of 1000 rows and commits each, so a
history load above 1000 rows could leave a closed row without its successor,
and that ONE `pyiceberg` transaction on the table writes the same rows in one
commit (case o, a prototype that bypassed `dlt` altogether). The product owner
chose that way (decision 10). This module measures the two things case o left
open, on the table the sink really makes -- day partition on `_ingested_at`,
format version 2, dataset `bronze`, `dlt`'s bookkeeping columns -- and asserts
what it OBSERVES (`AGENTS.md` principle 5). `dlt` 1.30.0, `pyiceberg` 0.12.0, a
local ephemeral catalog (not Lakekeeper), no query engine.

# How the table is created and how its schema grows (the choice)

Two candidates were measured:

(a) `dlt` creates and evolves the table, ours writes the data. A load whose
    table is missing a column the rows carry first runs the pipeline on a
    ZERO-ROW typed Arrow table with the full column set (`_ensure_columns`).
    `dlt` then creates the table with its hints (partition, format version 2,
    the four history columns NULLABLE) or adds the missing columns, and its
    schema, its state and its catalog entry are `dlt`'s own. The rows of the
    load are then written by ONE `pyiceberg` transaction (`_write`).
(b) `pyiceberg` alone for both.

(a) is taken, because these were measured (the tests named):

- `dlt` accepts a zero-row Arrow table, creates the table from it and evolves
  it from it (`test_creating_the_table_by_dlt_with_no_rows_...`,
  `test_a_column_added_at_the_source_is_added_by_dlt_...`). Two other forms do
  NOT work and are not used: an empty `dlt.resource([])` with column hints
  leaves the table untouched (scratch measurement: no commit, no column), and
  an Arrow table without `_dlt_load_id`/`_dlt_id` fails the load step on the
  required columns.
- The data of every load is exactly ONE commit
  (`test_a_load_of_3000_changed_keys_is_one_commit_of_6000_rows`). The schema
  step is metadata commits of its own, and they are NOT zero: creating the table
  and adding one column are each counted in the tests (read them: the assertions
  pin the numbers: 1 to create, 2 to add a column, and 1 for a `dlt` run with
  nothing to add, which is why the module runs the step only when a column is
  missing -- `test_an_ensure_step_with_nothing_to_add_...`). They carry no data, so a failure between them and the write
  leaves the table with its columns and no row, which a history load repeats
  harmlessly.
- Afterwards `dlt` and `pyiceberg` agree about the table: `dlt`'s schema lists
  every column of the table, a new working directory restores the same schema
  from the bucket, `clear_table` empties the table and keeps the columns, and a
  following history load works
  (`test_after_our_writes_dlt_agrees_about_the_table_and_clear_table_and_the_next_load_work`).

# The three properties (each with its test)

1. ONE commit for the data of a load, at any size
   (`test_a_load_of_3000_changed_keys_is_one_commit_of_6000_rows`,
   `test_a_load_of_2500_composite_keys_is_one_commit`).
2. A failure leaves the table exactly as it was, whether a row violates the
   schema or an exception is raised between staging the closed rows and the
   new rows (`test_a_row_that_violates_the_schema_...`,
   `test_an_exception_between_staging_the_closed_rows_and_the_new_ones_...`),
   and the load after a failed one succeeds.
3. A second identical load writes nothing and adds no commit
   (`test_a_second_identical_load_writes_nothing_and_adds_no_commit`).

# Prototypes

`_write`, `_ensure_columns` and `_history_load` are prototypes written for the
measurement; `adapters/history.py` (task H3) is the real module.

Run with:
~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_history_write_measurement.py -q
"""

from __future__ import annotations

import hashlib
import json
import secrets
import time
from datetime import datetime, timedelta, timezone
from typing import Any

import dlt
import pyarrow as pa
import pytest
from dlt.destinations.adapters import iceberg_adapter, iceberg_partition
from dlt.pipeline.exceptions import PipelineStepFailed
from pyiceberg.table import Table, Transaction

from dispar_orchestrate.adapters import sink as sink_module
from dispar_orchestrate.test_load_modes_measurement import (
    HISTORY_COLUMNS,
    T0,
    Lab,
    _current_rows,
    lab,  # noqa: F401 -- a fixture
)

# Runs a real dlt pipeline against a local Iceberg table (registered in pyproject.toml);
# CI runs these in `dagster-loader-tests`, the rest in `dagster-unit-tests`.
pytestmark = pytest.mark.loader

# All four NULLABLE: the table keeps its columns through `clear_table`, so a table
# that carries them as REQUIRED could never again be loaded in another mode, even
# after "Refresh, removing rows" (`test_required_history_columns_...`); and a
# required column cannot be added to a table that already exists. Our own write
# fills them on every row.
HISTORY_HINTS = {
    "_valid_from": {"data_type": "timestamp", "nullable": True},
    "_valid_to": {"data_type": "timestamp", "nullable": True},
    "_is_current": {"data_type": "bool", "nullable": True},
    "_row_hash": {"data_type": "text", "nullable": True},
}
REQUIRED_HINTS = {name: {**hint, "nullable": name != "_valid_to"} for name, hint in HISTORY_HINTS.items()}
UTC_US = pa.timestamp("us", tz="UTC")


def _source_schema(rows: list[dict[str, Any]], extra: list[str] = ()) -> list[tuple[str, pa.DataType]]:
    """The Arrow columns of the SOURCE rows (types inferred by `pyarrow`; the real
    module types them itself, task H3), then the system columns."""
    inferred = pa.Table.from_pylist(rows)
    fields = [(f.name, f.type) for f in inferred.schema]
    for name in extra:
        if name not in {n for n, _ in fields}:
            fields.append((name, pa.string()))
    return fields


def _open(lab: Lab, table: str) -> Table | None:
    """The table through the destination client of the pipeline that owns it:
    in production the Lakekeeper REST catalog the sink installs."""
    try:
        with lab.pipeline(table).destination_client() as client:  # type: ignore[attr-defined]
            return client.load_open_table("iceberg", table)
    except Exception:  # noqa: BLE001 -- measurement helper: a table not made yet
        return None


def _ensure_columns(lab: Lab, table: str, rows: list[dict[str, Any]], hints: dict | None = None) -> None:
    """Let `dlt` create the table, or add the columns the rows carry that it
    lacks, by running the pipeline on a zero-row typed Arrow table."""
    fields = [
        *_source_schema(rows),
        ("_ingested_at", UTC_US),
        ("_valid_from", UTC_US),
        ("_valid_to", UTC_US),
        ("_is_current", pa.bool_()),
        ("_row_hash", pa.string()),
    ]
    fields = [*fields, pa.field("_dlt_load_id", pa.string(), nullable=False), pa.field("_dlt_id", pa.string(), nullable=False)]
    resource = dlt.resource(pa.Table.from_pylist([], schema=pa.schema(fields)), name=table, columns=hints or HISTORY_HINTS)
    iceberg_adapter(
        resource, partition=[iceberg_partition.day("_ingested_at")], table_properties={"format-version": "2"}
    )
    lab.pipeline(table).run(resource, table_name=table, table_format="iceberg", write_disposition="append")


def _write(iceberg_table: Table, key: list[str], closed: list[dict], new: list[dict], strict: bool = True) -> None:
    """ONE transaction: the closed versions replace the current rows they close
    (matched on the key and `_valid_from`) and the new versions are inserted."""
    schema = iceberg_table.schema().as_arrow()
    if not strict:
        # Relax the Arrow schema so the rows reach pyiceberg's own check INSIDE
        # the transaction (the stage a real schema violation would be caught at).
        schema = pa.schema([f.with_nullable(True) for f in schema])
    arrow = pa.Table.from_pylist([*closed, *new], schema=schema)
    with iceberg_table.transaction() as transaction:
        transaction.upsert(arrow, join_cols=[*key, "_valid_from"])


def _history_load(lab: Lab, table: str, key: list[str], rows: list[dict[str, Any]], now: datetime) -> int:
    """One history load of `rows` (one row per key). Returns the number of rows
    written (closed plus new); 0 means nothing was written."""
    iceberg_table = _open(lab, table)
    # An empty table of another mode (what "Refresh, removing rows" leaves) has no `_is_current`.
    has_history = iceberg_table is not None and "_is_current" in {f.name for f in iceberg_table.schema().fields}
    current = _current_rows(lab, table, key, [tuple(r[c] for c in key) for r in rows]) if has_history else {}
    closed: list[dict[str, Any]] = []
    new: list[dict[str, Any]] = []
    for row in rows:
        k = tuple(row[c] for c in key)
        digest = _row_hash(row, key)
        existing = current.get(k)
        if existing is not None and existing["_row_hash"] == digest:
            continue
        if existing is not None:
            closed.append({**existing, "_valid_to": now, "_is_current": False})
        new.append(
            {
                **row,
                "_ingested_at": now,
                "_valid_from": now,
                "_valid_to": None,
                "_is_current": True,
                "_row_hash": digest,
                "_dlt_load_id": f"{now.timestamp()}",
                "_dlt_id": secrets.token_urlsafe(10),
            }
        )
    if not new:
        return 0
    have = {f.name for f in iceberg_table.schema().fields} if iceberg_table is not None else set()
    if not {*(c for r in rows for c in r), *HISTORY_COLUMNS, "_ingested_at"} <= have:
        _ensure_columns(lab, table, rows)
        iceberg_table = _open(lab, table)
    assert iceberg_table is not None
    _write(iceberg_table, key, closed, new)
    return len(closed) + len(new)


def _row_hash(row: dict[str, Any], key: list[str]) -> str:
    """Prototype hash: SHA-256 over the non-key source columns that are NOT null, in
    name order. A null is left out so a column added at the source changes the hash
    of a row only if the row has a value in it (task H3 defines the real one)."""
    body = [[c, row[c]] for c in sorted(row) if c not in key and not c.startswith("_") and row[c] is not None]
    return hashlib.sha256(json.dumps(body, default=str).encode()).hexdigest()


def _later(days: int) -> datetime:
    return T0 + timedelta(days=days)


def test_creating_the_table_by_dlt_with_no_rows_gives_the_loaders_set_up_and_one_commit_of_data(lab: Lab) -> None:
    rows = [{"id": i, "v": "a"} for i in range(3)]
    _ensure_columns(lab, "h", rows)
    created = lab.table("h")
    # What the sink makes: the day partition on `_ingested_at`, format version 2,
    # the history columns, and `dlt`'s own bookkeeping columns.
    assert [f.name for f in created.spec().fields] == ["_ingested_at_day"]
    assert created.metadata.format_version == 2
    names = {f.name for f in created.schema().fields}
    assert names >= {*HISTORY_COLUMNS, "id", "v", "_ingested_at", "_dlt_id", "_dlt_load_id"}
    required = {f.name for f in created.schema().fields if f.required}
    assert not required & set(HISTORY_COLUMNS), "the four history columns are nullable"
    assert {"_dlt_id", "_dlt_load_id"} <= required, "dlt's own columns stay required: our write fills them"
    assert len(lab.rows("h")) == 0, "the creation wrote no row"
    after_create = lab.commits("h")
    # Creating the table is metadata commits of its own, counted here:
    assert after_create == 1, "table creation and its empty first append"

    assert _history_load(lab, "h", ["id"], rows, T0) == 3
    assert lab.commits("h") - after_create == 1, "the data of the first load is ONE commit"
    assert len(lab.rows("h")) == 3
    assert all(r["_is_current"] and r["_valid_to"] is None and r["_valid_from"] == T0 for r in lab.rows("h"))


def test_a_load_of_3000_changed_keys_is_one_commit_of_6000_rows(lab: Lab) -> None:
    keys = list(range(3000))
    _history_load(lab, "big", ["id"], [{"id": i, "v": "old"} for i in keys], T0)
    commits, snaps = lab.commits("big"), lab.snapshots("big")
    written = _history_load(lab, "big", ["id"], [{"id": i, "v": "new"} for i in keys], _later(1))
    assert written == 6000
    assert lab.commits("big") - commits == 1, "one new entry in the metadata log"
    # pyiceberg's upsert stages an overwrite (a delete of the old file, a write of the
    # closed rows) and an append (the new versions): three snapshots, ONE commit.
    assert lab.snapshots("big") - snaps == 3
    rows = lab.rows("big")
    assert len(rows) == 6000
    assert sum(1 for r in rows if r["_is_current"]) == 3000
    closed = [r for r in rows if not r["_is_current"]]
    assert len(closed) == 3000 and all(r["_valid_to"] == _later(1) and r["v"] == "old" for r in closed)


def test_a_load_of_2500_composite_keys_is_one_commit(lab: Lab) -> None:
    keys = [(i // 50, i % 50) for i in range(2500)]
    old = [{"a": a, "b": b, "v": "old"} for a, b in keys]
    _history_load(lab, "comp", ["a", "b"], old, T0)
    commits = lab.commits("comp")
    # Only half change: the rest must be left alone.
    mixed = [{"a": a, "b": b, "v": "new" if (a + b) % 2 else "old"} for a, b in keys]
    started = time.monotonic()
    written = _history_load(lab, "comp", ["a", "b"], mixed, _later(1))
    elapsed = time.monotonic() - started
    changed = sum(1 for a, b in keys if (a + b) % 2)
    assert written == 2 * changed
    assert lab.commits("comp") - commits == 1
    rows = lab.rows("comp")
    assert len(rows) == 2500 + changed
    current = {(r["a"], r["b"]): r for r in rows if r["_is_current"]}
    assert len(current) == 2500, "one current row per composite key"
    assert elapsed < 120, "the read of 2500 composite keys and the write stay in seconds, not minutes"
    # The read of current rows is exact for a composite key: the per-column filter is a
    # superset, and the pair check removes the rest.
    got = _current_rows(lab, "comp", ["a", "b"], [(0, 1), (1, 0), (49, 49)], chunk=2)
    assert sorted(got) == [(0, 1), (1, 0), (49, 49)]


def test_a_second_identical_load_writes_nothing_and_adds_no_commit(lab: Lab) -> None:
    rows = [{"id": i, "v": "a"} for i in range(50)]
    _history_load(lab, "same", ["id"], rows, T0)
    commits, snaps, before = lab.commits("same"), lab.snapshots("same"), lab.rows("same")
    assert _history_load(lab, "same", ["id"], rows, _later(1)) == 0
    assert _history_load(lab, "same", ["id"], rows, _later(2)) == 0
    assert (lab.commits("same"), lab.snapshots("same")) == (commits, snaps)
    assert lab.rows("same") == before


def test_a_row_that_violates_the_schema_leaves_the_table_exactly_as_it_was_and_the_next_load_succeeds(
    lab: Lab,
) -> None:
    keys = list(range(3000))
    _history_load(lab, "viol", ["id"], [{"id": i, "v": "old"} for i in keys], T0)
    commits, snaps, before = lab.commits("viol"), lab.snapshots("viol"), lab.rows("viol")

    iceberg_table = _open(lab, "viol")
    assert iceberg_table is not None
    current = _current_rows(lab, "viol", ["id"], [(i,) for i in keys])
    closed = [{**row, "_valid_to": _later(1), "_is_current": False} for row in current.values()]
    new = [
        {**row, "v": "new", "_valid_from": _later(1), "_valid_to": None, "_is_current": True, "_row_hash": "x"}
        for row in current.values()
    ]
    new[-1]["_dlt_id"] = None  # the table says `_dlt_id` is required (dlt's own bookkeeping column)
    with pytest.raises(Exception):  # noqa: B017 -- pyiceberg's own error type is not the point, the state is
        _write(iceberg_table, ["id"], closed, new, strict=False)
    assert lab.commits("viol") == commits and lab.snapshots("viol") == snaps
    assert lab.rows("viol") == before, "no closed row, no new row"

    assert _history_load(lab, "viol", ["id"], [{"id": i, "v": "new"} for i in keys], _later(2)) == 6000
    assert lab.commits("viol") - commits == 1, "the load after a failed one succeeds, in one commit"


def test_an_exception_between_staging_the_closed_rows_and_the_new_ones_leaves_the_table_exactly_as_it_was(
    lab: Lab, monkeypatch: pytest.MonkeyPatch
) -> None:
    keys = list(range(3000))
    _history_load(lab, "mid", ["id"], [{"id": i, "v": "old"} for i in keys], T0)
    commits, snaps, before = lab.commits("mid"), lab.snapshots("mid"), lab.rows("mid")

    staged: list[int] = []
    original_overwrite, original_append = Transaction.overwrite, Transaction.append

    def overwrite_recorded(self: Transaction, df: pa.Table, *args: Any, **kwargs: Any) -> None:
        staged.append(df.num_rows)
        return original_overwrite(self, df, *args, **kwargs)

    def append_that_fails(self: Transaction, df: pa.Table, *args: Any, **kwargs: Any) -> None:
        raise RuntimeError("injected failure after the closed rows were staged")

    with monkeypatch.context() as patched:
        patched.setattr(Transaction, "overwrite", overwrite_recorded)
        patched.setattr(Transaction, "append", append_that_fails)
        with pytest.raises(RuntimeError, match="injected"):
            _history_load(lab, "mid", ["id"], [{"id": i, "v": "new"} for i in keys], _later(1))
    assert staged == [3000], "the 3000 closed rows WERE staged before the failure"
    assert lab.commits("mid") == commits and lab.snapshots("mid") == snaps
    assert lab.rows("mid") == before
    assert not [r for r in lab.rows("mid") if not r["_is_current"]], "no closed row without its successor"

    assert _history_load(lab, "mid", ["id"], [{"id": i, "v": "new"} for i in keys], _later(2)) == 6000
    assert lab.commits("mid") - commits == 1


def test_a_column_added_at_the_source_is_added_by_dlt_in_metadata_commits_of_its_own_and_the_data_is_one_commit(
    lab: Lab,
) -> None:
    _history_load(lab, "grow", ["id"], [{"id": 1, "v": "a"}, {"id": 2, "v": "b"}], T0)
    commits = lab.commits("grow")
    rows = [{"id": 1, "v": "a", "extra": "x"}, {"id": 2, "v": "b", "extra": None}]
    # The ensure step alone: the columns of the rows are added, no row is written.
    _ensure_columns(lab, "grow", rows)
    schema_commits = lab.commits("grow") - commits
    assert "extra" in {f.name for f in lab.table("grow").schema().fields}
    assert schema_commits == 2, "adding a column: the schema change and dlt's empty append, two metadata commits"
    assert len(lab.rows("grow")) == 2
    # The data load of the same rows: one commit.
    commits = lab.commits("grow")
    assert _history_load(lab, "grow", ["id"], rows, _later(1)) == 2, "only id 1 (a non-null value in the new column)"
    assert lab.commits("grow") - commits == 1


def test_an_ensure_step_with_nothing_to_add_still_costs_dlts_empty_append_so_the_module_skips_it(lab: Lab) -> None:
    rows = [{"id": 1, "v": "a"}]
    _history_load(lab, "again", ["id"], rows, T0)
    commits = lab.commits("again")
    _ensure_columns(lab, "again", rows)
    assert lab.commits("again") - commits == 1, "one empty append commit: the ensure step is gated on a missing column"
    # ... and `_history_load` gates it: a load whose columns the table has runs no ensure step.
    commits = lab.commits("again")
    _history_load(lab, "again", ["id"], [{"id": 1, "v": "b"}], _later(1))
    assert lab.commits("again") - commits == 1, "the data commit only"


def test_after_our_writes_dlt_agrees_about_the_table_and_clear_table_and_the_next_load_work(
    lab: Lab, tmp_path: Any, monkeypatch: pytest.MonkeyPatch
) -> None:
    rows = [{"id": 1, "v": "a"}, {"id": 2, "v": "b"}]
    _history_load(lab, "agree", ["id"], rows, T0)
    _history_load(lab, "agree", ["id"], [{"id": 1, "v": "A", "extra": "x"}, {"id": 2, "v": "b"}], _later(1))
    iceberg_names = [f.name for f in lab.table("agree").schema().fields]

    # dlt's schema, in the pipeline that wrote the table, lists every column of the table ...
    pipeline = lab.pipeline("agree")
    dlt_names = list(pipeline.default_schema.tables["agree"]["columns"])
    assert sorted(dlt_names) == sorted(iceberg_names)
    # ... and so does the schema a fresh working directory restores from the bucket.
    monkeypatch.setenv("DLT_DATA_DIR", str(tmp_path / "dlt-restored"))
    restored = lab.pipeline("agree")
    restored.sync_destination()
    assert sorted(restored.default_schema.tables["agree"]["columns"]) == sorted(iceberg_names)

    # `clear_table` (the first half of "Refresh, removing rows") empties it and keeps the columns.
    cleared = sink_module.clear_table("agree", lab.config)
    assert cleared.rows == 0
    assert len(lab.rows("agree")) == 0
    assert [f.name for f in lab.table("agree").schema().fields] == iceberg_names
    assert [f.name for f in lab.table("agree").spec().fields] == ["_ingested_at_day"]
    assert lab.table("agree").metadata.format_version == 2

    # The next history load works, and every key is a first version.
    commits = lab.commits("agree")
    assert _history_load(lab, "agree", ["id"], [{"id": 1, "v": "A", "extra": "x"}, {"id": 2, "v": "b"}], _later(2)) == 2
    assert lab.commits("agree") - commits == 1
    rows_now = lab.rows("agree")
    assert len(rows_now) == 2 and all(r["_is_current"] and r["_valid_from"] == _later(2) for r in rows_now)
    # And one more load after that one still sees its current rows (dlt left no stale state behind).
    assert _history_load(lab, "agree", ["id"], [{"id": 1, "v": "B", "extra": "x"}, {"id": 2, "v": "b"}], _later(3)) == 2


def test_the_read_of_current_rows_through_the_sinks_client_chunks_2500_keys(lab: Lab) -> None:
    keys = [(i,) for i in range(2500)]
    _history_load(lab, "read", ["id"], [{"id": i, "v": "a"} for i in range(2500)], T0)
    _history_load(lab, "read", ["id"], [{"id": i, "v": "b"} for i in range(2500)], _later(1))
    commits = lab.commits("read")
    found = _current_rows(lab, "read", ["id"], keys, chunk=1000)
    assert len(found) == 2500 and all(row["v"] == "b" and row["_is_current"] for row in found.values())
    assert lab.commits("read") == commits, "a read commits nothing"


def test_a_table_loaded_in_another_mode_and_then_emptied_takes_the_history_columns_from_the_ensure_step(
    lab: Lab, tmp_path: Any
) -> None:
    """The way into history is "Refresh, removing rows": `clear_table`, then a
    history load on a table that was made by another mode. The table is made from a
    SQL source: `clear_table` does not empty one made from plain rows (reported in
    the handoff; `test_sink_history.py`, the `xfail` test)."""
    import sqlite3

    from dlt.sources.sql_database import sql_database

    db = sqlite3.connect(tmp_path / "mixed.db")
    db.execute("CREATE TABLE mixed (id INTEGER PRIMARY KEY, v TEXT)")
    db.execute("INSERT INTO mixed VALUES (1, 'a')")
    db.commit()
    source = sql_database(credentials={"drivername": "sqlite", "database": str(tmp_path / "mixed.db")}, table_names=["mixed"])
    sink_module.load_via_sink(source, "mixed", lab.config, sink_module.LoadPlan(mode="replace"))
    sink_module.clear_table("mixed", lab.config)
    assert lab.rows("mixed") == []
    assert not {*HISTORY_COLUMNS} & {f.name for f in lab.table("mixed").schema().fields}
    assert _history_load(lab, "mixed", ["id"], [{"id": 1, "v": "a"}], T0) == 1
    assert {*HISTORY_COLUMNS} <= {f.name for f in lab.table("mixed").schema().fields}
    assert [r["_is_current"] for r in lab.rows("mixed")] == [True]


def test_the_way_out_of_history_is_clear_and_a_load_in_another_mode_with_nullable_columns(lab: Lab) -> None:
    _history_load(lab, "out", ["id"], [{"id": 1, "v": "a"}], T0)
    sink_module.clear_table("out", lab.config)
    assert lab.rows("out") == []
    sink_module.load_via_sink([{"id": 7, "v": "z"}], "out", lab.config, sink_module.LoadPlan(mode="replace"))
    rows = lab.rows("out")
    assert [(r["id"], r["_row_hash"]) for r in rows] == [(7, None)], "a row of another mode has no history values"


def test_required_history_columns_would_make_clear_and_a_load_in_another_mode_fail(lab: Lab) -> None:
    """Why the columns are nullable. Measured with REQUIRED hints: after `clear_table`
    a replace load of rows without them fails at `dlt`'s normalize step ("declared
    non-nullable but contains nulls"), so a history table could never leave history."""
    rows = [{"id": 1, "v": "a"}]
    _ensure_columns(lab, "req", rows, hints=REQUIRED_HINTS)
    sink_module.clear_table("req", lab.config)
    with pytest.raises(PipelineStepFailed):
        sink_module.load_via_sink([{"id": 7, "v": "z"}], "req", lab.config, sink_module.LoadPlan(mode="replace"))
