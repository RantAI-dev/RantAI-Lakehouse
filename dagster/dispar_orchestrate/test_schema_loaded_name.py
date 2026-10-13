"""`SRC-8` task 11: the name the loader gives a source column, checked
against a REAL load.

The Schema tab of a Bronze table lists the table's own column names, which
`dlt` has normalised (`OrderDate` -> `order_date`). The API marks a removed
source column inactive by that name, so the orchestrator sends it as
`loadedName` with every before-load observation, computed by
`schema_observer.loaded_column_name` from `dlt`'s own naming convention. These
tests do not trust that function: they load a SQL table whose columns need
normalising through the same kind of local Iceberg destination the drift tests
use (`test_schema_drift_sql_source.py`) and read the column names back from the
Iceberg metadata. No network, no Lakekeeper.

Run with:
~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_schema_loaded_name.py -q
"""

from __future__ import annotations

import glob
import sqlite3
from pathlib import Path

import dlt
import pytest
from dlt.destinations import filesystem
from dlt.sources.sql_database import sql_database
from pyiceberg.table import StaticTable

from dispar_orchestrate import schema_observer

TABLE = "src"
# A camel-case name, one with a space, one already snake_case, one with a
# leading digit and one with a symbol: the shapes `dlt` rewrites differently.
SOURCE_COLUMNS = ["OrderDate", "Ship Date", "already_snake", "Qty", "2nd", "a-b"]


@pytest.fixture(autouse=True)
def _no_lakekeeper_catalog(monkeypatch: pytest.MonkeyPatch) -> None:
    # `sink._install_catalog_env` sets these process-wide and other test
    # modules leave them behind (see `test_schema_drift_loader.py`).
    for name in (
        "ICEBERG_CATALOG__ICEBERG_CATALOG_NAME",
        "ICEBERG_CATALOG__ICEBERG_CATALOG_TYPE",
        "ICEBERG_CATALOG__ICEBERG_CATALOG_CONFIG",
    ):
        monkeypatch.delenv(name, raising=False)


def _load(tmp_path: Path, collector: schema_observer.ReflectionCollector) -> set[str]:
    """Create the source table, load it, and return the Iceberg table's
    column names without `dlt`'s bookkeeping columns."""
    db = tmp_path / "source.db"
    bucket = tmp_path / "bucket"
    quoted = ", ".join(f'"{c}" INTEGER' for c in SOURCE_COLUMNS)
    with sqlite3.connect(db) as conn:
        conn.execute(f"CREATE TABLE {TABLE} ({quoted})")
        conn.execute(f"INSERT INTO {TABLE} VALUES ({', '.join('1' for _ in SOURCE_COLUMNS)})")
    source = sql_database(
        credentials={"drivername": "sqlite", "database": str(db)},
        table_names=[TABLE],
        table_adapter_callback=collector.callback,
    )
    pipeline = dlt.pipeline(
        pipeline_name="loadedname",
        pipelines_dir=str(tmp_path / "pipelines"),
        destination=filesystem(bucket_url=str(bucket)),
        dataset_name="bronze",
    )
    pipeline.run(source, table_name=TABLE, table_format="iceberg", write_disposition="replace")
    metadata = sorted(glob.glob(str(bucket / "**" / "metadata" / "*.metadata.json"), recursive=True))
    table = StaticTable.from_metadata(f"file://{metadata[-1]}")
    return {f.name for f in table.schema().fields if not f.name.startswith("_dlt_")}


def test_the_loaded_name_of_every_column_is_exactly_the_name_the_bronze_table_ends_up_with(tmp_path: Path) -> None:
    collector = schema_observer.ReflectionCollector()
    in_bronze = _load(tmp_path, collector)

    reflected = collector.only().columns
    assert [c.name for c in reflected] == SOURCE_COLUMNS
    assert {c.loaded_name for c in reflected} == in_bronze
    assert len(in_bronze) == len(SOURCE_COLUMNS), "no two source columns may collapse into one"


def test_the_loaded_names_of_the_usual_shapes_are_the_snake_case_ones(tmp_path: Path) -> None:
    # Pins the conventions the console meets most: a rename by `dlt` that is
    # not this fails the real-load test above first, then this one says how.
    got = {name: schema_observer.loaded_column_name(name) for name in SOURCE_COLUMNS}

    assert got["OrderDate"] == "order_date"
    assert got["Ship Date"] == "ship_date"
    assert got["already_snake"] == "already_snake"
    assert got["Qty"] == "qty"


def test_a_before_load_column_carries_its_loaded_name_on_the_wire_and_an_unknown_one_leaves_it_out() -> None:
    named = schema_observer.ReflectedColumn("OrderDate", "date", True, loaded_name="order_date")
    unnamed = schema_observer.ReflectedColumn("OrderDate", "date", True)

    assert named.to_wire() == {"name": "OrderDate", "typeName": "date", "nullable": True, "loadedName": "order_date"}
    assert "loadedName" not in unnamed.to_wire()
