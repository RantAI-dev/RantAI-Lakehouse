"""Measurement of what the loader does when a SQL SOURCE TABLE changes its
definition (`SRC-8`, review SHOULD-FIX 1 of tasks 1-5; feature page
decisions D4 and D9), and of the column-selection mechanism task 6 uses to
hold a column back.

`test_schema_drift_loader.py` measured rows whose types `dlt` INFERS from
the data. A batch SQL source is different: `dlt.sources.sql_database`
reflects the database's own column definitions with SQLAlchemy and hands
`dlt` typed column hints (`reflection_level="full"`, the default, which the
production adapters leave untouched). This module loads a SQL table twice
into one destination, changing the table's definition at the source in
between, and asserts what `dlt`'s schema and the Iceberg metadata say
afterwards. Observations of `dlt` 1.30.0 and SQLAlchemy 2.0; a `dlt` upgrade
that changes one fails the matching test, which is the point.

# Result in one paragraph

Added and dropped columns, INTEGER to BIGINT and a longer VARCHAR behave as
for inferred rows. A type change between INTEGER and TEXT does NOT: the load
FAILS (Iceberg refuses `long -> string` and `string -> long`), no second
`<column>__v_text` column is made, and the failed package stays pending, so
later loads fail too until the pipeline's working directory is gone. This is
why SRC-8 must hold such a table back before `pipeline.run`, and it
contradicts the wording of feature-page decision 9 for SQL sources.

# What this does and does not tell us

The source here is SQLite through SQLAlchemy (stdlib `sqlite3`, no server),
with every column type declared explicitly. SQLite has type AFFINITY, not
strict types: a TEXT column accepts any value and an INTEGER column stores a
non-numeric string as text. So the loader sees what the declaration says
(SQLAlchemy reflects the declared name: `INTEGER`, `TEXT`, `BIGINT`,
`VARCHAR(20)`), and the rows below are chosen so the data is consistent with
the declaration where the database would enforce it (an INTEGER column in
PostgreSQL could never hold `'not a number'`; here such a value only appears
under a TEXT declaration).

What carries over to PostgreSQL, MySQL/MariaDB, SQL Server and Oracle: the
step from a reflected SQLAlchemy type to a `dlt` data type
(`dlt/sources/sql_database/schema_types.py`) and what `dlt` and Iceberg do
with a changed column hint. What does NOT carry over: the type spellings and
widths each dialect reflects (`int4`, `INT UNSIGNED`, `NUMBER(10)`, `NVARCHAR`
...), their precision and scale hints (not present at
`reflection_level="full"`), the dialect-specific rows the driver returns, and
any server-side type enforcement. The type NAMES SRC-8 sends to the API are
the database's own spellings and are compared by the API (`schema_diff.rs`),
not by `dlt`; this module only measures the loader.

Same as `test_schema_drift_loader.py`: `filesystem` destination on a
temporary directory, `table_format="iceberg"` on `dlt`'s own ephemeral local
catalog, no Lakekeeper, no partition, no `_ingested_at`. `dlt`'s
bookkeeping columns (`_dlt_id`, `_dlt_load_id`) are left out of the
assertions.

Run with:
~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_schema_drift_sql_source.py -q
"""

from __future__ import annotations

import glob
import sqlite3
from collections.abc import Callable
from pathlib import Path
from typing import Any

import dlt
import pytest
from dlt.destinations import filesystem
from dlt.pipeline.exceptions import PipelineStepFailed
from dlt.sources.sql_database import sql_database, sql_table
from pyiceberg.table import StaticTable

DISPOSITIONS = ["append", "replace"]
TABLE = "src"


@pytest.fixture(autouse=True)
def _no_lakekeeper_catalog(monkeypatch: pytest.MonkeyPatch) -> None:
    # Same isolation as `test_schema_drift_loader.py`: `sink._install_catalog_env`
    # sets these process-wide and other test modules leave them behind.
    for name in (
        "ICEBERG_CATALOG__ICEBERG_CATALOG_NAME",
        "ICEBERG_CATALOG__ICEBERG_CATALOG_TYPE",
        "ICEBERG_CATALOG__ICEBERG_CATALOG_CONFIG",
    ):
        monkeypatch.delenv(name, raising=False)


def _is_bookkeeping(name: str) -> bool:
    return name.startswith("_dlt_")


def _define(db: Path, ddl: str, rows: list[tuple[Any, ...]], insert: str) -> None:
    """Replace the table `src` with the definition `ddl` and `rows`."""
    with sqlite3.connect(db) as conn:
        conn.execute(f"DROP TABLE IF EXISTS {TABLE}")
        conn.execute(ddl)
        conn.executemany(insert, rows)


def _source(db: Path) -> Any:
    # The way `adapters/sql.py::build_source` builds it: credentials and
    # `table_names`, every other argument at its default.
    return sql_database(credentials={"drivername": "sqlite", "database": str(db)}, table_names=[TABLE])


def _read_back(pipeline: Any, bucket: Path) -> dict[str, Any]:
    dlt_columns: dict[str, str] = {}
    primary_keys: list[str] = []
    for name, column in pipeline.default_schema.tables[TABLE]["columns"].items():
        if _is_bookkeeping(name):
            continue
        precision = column.get("precision")
        dlt_columns[name] = column["data_type"] + (f"({precision})" if precision else "")
        if column.get("primary_key"):
            primary_keys.append(name)
    metadata = sorted(glob.glob(str(bucket / "**" / "metadata" / "*.metadata.json"), recursive=True))
    table = StaticTable.from_metadata(f"file://{metadata[-1]}")
    iceberg_columns = {
        field.name: str(field.field_type) for field in table.schema().fields if not _is_bookkeeping(field.name)
    }
    return {"dlt": dlt_columns, "iceberg": iceberg_columns, "primary_key": primary_keys}


def _load_sql_twice(
    tmp_path: Path,
    disposition: str,
    first: tuple[str, list[tuple[Any, ...]], str],
    second: tuple[str, list[tuple[Any, ...]], str],
    source: Callable[[Path], Any] = _source,
) -> dict[str, Any]:
    """Define the source table by `first` (DDL, rows, INSERT), load it, redefine
    the SAME table by `second`, load it again into the same Iceberg table."""
    db = tmp_path / "source.db"
    bucket = tmp_path / "bucket"
    pipeline = dlt.pipeline(
        pipeline_name=f"sqldrift_{disposition}",
        pipelines_dir=str(tmp_path / "pipelines"),
        destination=filesystem(bucket_url=str(bucket)),
        dataset_name="bronze",
    )
    failed: str | None = None
    for ddl, rows, insert in (first, second):
        _define(db, ddl, rows, insert)
        try:
            pipeline.run(source(db), table_name=TABLE, table_format="iceberg", write_disposition=disposition)
        except PipelineStepFailed as exc:
            # A load that fails is an observation, not a test error: it is
            # exactly what SRC-8 has to be ready for.
            failed = type(exc.exception).__name__ if getattr(exc, "exception", None) else type(exc).__name__
    result = _read_back(pipeline, bucket)
    result["failed"] = failed
    return result


def _two(
    ddl1: str, rows1: list[tuple[Any, ...]], ddl2: str, rows2: list[tuple[Any, ...]]
) -> tuple[tuple[str, list[tuple[Any, ...]], str], tuple[str, list[tuple[Any, ...]], str]]:
    def insert(rows: list[tuple[Any, ...]]) -> str:
        return f"INSERT INTO {TABLE} VALUES ({', '.join('?' * len(rows[0]))})"

    return (ddl1, rows1, insert(rows1)), (ddl2, rows2, insert(rows2))


@pytest.mark.parametrize("disposition", DISPOSITIONS)
def test_a_column_added_to_the_source_table_is_added_to_the_destination(tmp_path: Path, disposition: str) -> None:
    first, second = _two(
        "CREATE TABLE src (a INTEGER PRIMARY KEY, b TEXT)",
        [(1, "x")],
        "CREATE TABLE src (a INTEGER PRIMARY KEY, b TEXT, c INTEGER)",
        [(1, "x", 7)],
    )
    got = _load_sql_twice(tmp_path, disposition, first, second)
    assert got["failed"] is None
    assert got["dlt"] == {"a": "bigint", "b": "text", "c": "bigint"}
    assert got["iceberg"] == {"a": "long", "b": "string", "c": "long"}


@pytest.mark.parametrize("disposition", DISPOSITIONS)
def test_a_column_dropped_from_the_source_table_stays_in_the_destination(tmp_path: Path, disposition: str) -> None:
    first, second = _two(
        "CREATE TABLE src (a INTEGER PRIMARY KEY, b TEXT)",
        [(1, "x")],
        "CREATE TABLE src (a INTEGER PRIMARY KEY)",
        [(1,)],
    )
    got = _load_sql_twice(tmp_path, disposition, first, second)
    assert got["failed"] is None
    assert got["dlt"] == {"a": "bigint", "b": "text"}
    assert got["iceberg"] == {"a": "long", "b": "string"}


@pytest.mark.parametrize("disposition", DISPOSITIONS)
def test_an_integer_column_redefined_as_text_makes_the_second_load_fail_and_leaves_the_iceberg_column_alone(
    tmp_path: Path, disposition: str
) -> None:
    # CONTRADICTS feature-page decision 9 ("a second column beside the old
    # one") for a SQL source: the reflected TEXT hint retypes the column in
    # `dlt`'s schema (no `a__v_text` variant is made, unlike an inferred
    # type), and the Iceberg table refuses `long -> string`.
    first, second = _two(
        "CREATE TABLE src (id INTEGER PRIMARY KEY, a INTEGER)",
        [(1, 10)],
        "CREATE TABLE src (id INTEGER PRIMARY KEY, a TEXT)",
        [(1, "not a number")],
    )
    got = _load_sql_twice(tmp_path, disposition, first, second)
    assert got["failed"] == "LoadClientJobRetry"
    assert got["dlt"] == {"id": "bigint", "a": "text"}
    assert got["iceberg"] == {"id": "long", "a": "long"}


@pytest.mark.parametrize("disposition", DISPOSITIONS)
def test_a_text_column_redefined_as_integer_makes_the_second_load_fail_and_leaves_the_iceberg_column_alone(
    tmp_path: Path, disposition: str
) -> None:
    # Also contradicts the inferred-type measurement, where the text column
    # simply took the integers (`test_schema_drift_loader.py`): the reflected
    # INTEGER hint retypes the column in `dlt`'s schema and Iceberg refuses
    # `string -> long`.
    first, second = _two(
        "CREATE TABLE src (id INTEGER PRIMARY KEY, a TEXT)",
        [(1, "ten")],
        "CREATE TABLE src (id INTEGER PRIMARY KEY, a INTEGER)",
        [(1, 10)],
    )
    got = _load_sql_twice(tmp_path, disposition, first, second)
    assert got["failed"] == "LoadClientJobRetry"
    assert got["dlt"] == {"id": "bigint", "a": "bigint"}
    assert got["iceberg"] == {"id": "long", "a": "string"}


@pytest.mark.parametrize("disposition", DISPOSITIONS)
def test_an_integer_column_redefined_as_bigint_with_huge_values_keeps_one_column(
    tmp_path: Path, disposition: str
) -> None:
    first, second = _two(
        "CREATE TABLE src (id INTEGER PRIMARY KEY, a INTEGER)",
        [(1, 5)],
        "CREATE TABLE src (id INTEGER PRIMARY KEY, a BIGINT)",
        [(1, 5_000_000_000)],
    )
    got = _load_sql_twice(tmp_path, disposition, first, second)
    assert got["failed"] is None
    assert got["dlt"] == {"id": "bigint", "a": "bigint"}
    assert got["iceberg"] == {"id": "long", "a": "long"}


@pytest.mark.parametrize("disposition", DISPOSITIONS)
def test_a_varchar_column_that_gets_a_longer_declared_length_keeps_one_column(
    tmp_path: Path, disposition: str
) -> None:
    first, second = _two(
        "CREATE TABLE src (id INTEGER PRIMARY KEY, a VARCHAR(5))",
        [(1, "abc")],
        "CREATE TABLE src (id INTEGER PRIMARY KEY, a VARCHAR(500))",
        [(1, "abc" * 50)],
    )
    got = _load_sql_twice(tmp_path, disposition, first, second)
    assert got["failed"] is None
    assert got["dlt"] == {"id": "bigint", "a": "text"}
    assert got["iceberg"] == {"id": "long", "a": "string"}


@pytest.mark.parametrize("disposition", DISPOSITIONS)
def test_a_primary_key_that_moves_to_another_column_takes_the_new_key_in_the_dlt_schema_only(
    tmp_path: Path, disposition: str
) -> None:
    first, second = _two(
        "CREATE TABLE src (a INTEGER NOT NULL, b INTEGER NOT NULL, PRIMARY KEY (a))",
        [(1, 2)],
        "CREATE TABLE src (a INTEGER NOT NULL, b INTEGER NOT NULL, PRIMARY KEY (b))",
        [(1, 2)],
    )
    got = _load_sql_twice(tmp_path, disposition, first, second)
    assert got["failed"] is None
    assert got["dlt"] == {"a": "bigint", "b": "bigint"}
    assert got["iceberg"] == {"a": "long", "b": "long"}
    # The key is a `dlt` hint (used by `merge`); Iceberg keeps no key here.
    assert got["primary_key"] == ["b"]


def test_after_a_failed_type_change_the_next_load_fails_too_even_when_the_source_is_put_back(tmp_path: Path) -> None:
    """Why SRC-8 must stop such a table BEFORE `pipeline.run`: the failed
    package stays pending in the pipeline's working directory (production:
    `bronze_ingest_<table>` under `dlt`'s default pipelines dir in the code
    location container), and the next `run` tries it again first. Putting
    the source column back to INTEGER does not unblock it."""
    db = tmp_path / "source.db"
    pipeline = dlt.pipeline(
        pipeline_name="sqlwedge",
        pipelines_dir=str(tmp_path / "pipelines"),
        destination=filesystem(bucket_url=str(tmp_path / "bucket")),
        dataset_name="bronze",
    )
    first, second = _two(
        "CREATE TABLE src (id INTEGER PRIMARY KEY, a INTEGER)",
        [(1, 10)],
        "CREATE TABLE src (id INTEGER PRIMARY KEY, a TEXT)",
        [(1, "not a number")],
    )
    outcomes = []
    for ddl, rows, insert in (first, second, second, first):
        _define(db, ddl, rows, insert)
        try:
            pipeline.run(_source(db), table_name=TABLE, table_format="iceberg", write_disposition="append")
            outcomes.append("ok")
        except PipelineStepFailed:
            outcomes.append("failed")
    assert outcomes == ["ok", "failed", "failed", "failed"]


# --- Column selection (task 6: hold a column back) --------------------------


def _without(*names: str) -> Callable[[Any], None]:
    """A `table_adapter_callback` that removes columns from the reflected
    `Table` before `dlt` builds its hints and its SELECT."""

    def adapter(table: Any) -> None:
        for column in list(table._columns):
            if column.name in names:
                table._columns.remove(column)

    return adapter


def _run_once(tmp_path: Path, source: Any) -> dict[str, Any]:
    bucket = tmp_path / "bucket"
    pipeline = dlt.pipeline(
        pipeline_name="sqlselect",
        pipelines_dir=str(tmp_path / "pipelines"),
        destination=filesystem(bucket_url=str(bucket)),
        dataset_name="bronze",
    )
    pipeline.run(source, table_name=TABLE, table_format="iceberg", write_disposition="replace")
    return _read_back(pipeline, bucket)


def _three_columns(tmp_path: Path) -> Path:
    db = tmp_path / "source.db"
    _define(
        db,
        "CREATE TABLE src (a INTEGER PRIMARY KEY, b TEXT, c INTEGER)",
        [(1, "x", 7)],
        "INSERT INTO src VALUES (?, ?, ?)",
    )
    return db


def test_a_table_adapter_callback_that_removes_a_column_keeps_it_out_of_the_destination(tmp_path: Path) -> None:
    db = _three_columns(tmp_path)
    source = sql_database(
        credentials={"drivername": "sqlite", "database": str(db)},
        table_names=[TABLE],
        table_adapter_callback=_without("c"),
    )
    got = _run_once(tmp_path, source)
    assert got["dlt"] == {"a": "bigint", "b": "text"}
    assert got["iceberg"] == {"a": "long", "b": "string"}


def test_sql_table_included_columns_keeps_the_unselected_column_out_of_the_destination(tmp_path: Path) -> None:
    db = _three_columns(tmp_path)
    resource = sql_table(
        credentials={"drivername": "sqlite", "database": str(db)},
        table=TABLE,
        included_columns=["a", "b"],
    )
    got = _run_once(tmp_path, resource)
    assert got["dlt"] == {"a": "bigint", "b": "text"}
    assert got["iceberg"] == {"a": "long", "b": "string"}


def test_a_table_adapter_callback_sees_the_reflected_columns_and_key_before_any_row_is_read(tmp_path: Path) -> None:
    """Task 6 reads the reflection from this callback: it runs while the
    source is being BUILT, from the one reflection `dlt` does anyway, so no
    second connection is needed to know the columns."""
    db = _three_columns(tmp_path)
    seen: list[tuple[list[tuple[str, str, bool]], list[str]]] = []

    def capture(table: Any) -> None:
        seen.append(
            (
                [(c.name, str(c.type), bool(c.nullable)) for c in table.columns],
                [c.name for c in table.primary_key.columns],
            )
        )

    sql_database(
        credentials={"drivername": "sqlite", "database": str(db)},
        table_names=[TABLE],
        table_adapter_callback=capture,
    )
    # `a` reads nullable because SQLite reports a rowid-alias primary key that
    # way; a server database reports NOT NULL for a key column.
    assert seen == [([("a", "INTEGER", True), ("b", "TEXT", True), ("c", "INTEGER", True)], ["a"])]
