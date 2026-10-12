"""Measurement of what the loader does when a source table changes shape
(`SRC-8`, finding F7; feature page decision D9).

`SRC-8` promises, in `docs/core/features/source-schema-changes.md`
decision 9, that a widened type the table cannot hold in the same column
lands in "a second column beside the old one", and decision 4 that a
column which stops arriving stays in the table. Neither was ever measured
here (`AGENTS.md` principle 5), so every test below loads a table twice
into one destination and asserts what the columns are afterwards. These
are observations of `dlt` 1.30.0, the version this branch's Dagster venv
installs; a `dlt` upgrade that changes one of them fails the matching test,
which is the point.

# What is and is not the production path

Production (`adapters/sink.py::load_via_sink`) runs
`dlt.pipeline(...).run(source, table_name=..., table_format="iceberg",
write_disposition=...)` against a `filesystem` destination whose bucket is
S3 (RustFS), with a Lakekeeper REST catalog in `os.environ`, the
`iceberg_adapter` day partition on `_ingested_at`, and `_ingested_at` stamped
on every row. This module keeps what decides the columns and drops what
needs a network:

- SAME: `dlt`'s normalizer and schema inference, `table_format="iceberg"`
  (so the column list is also read back from the real Iceberg table
  metadata with `pyiceberg`, not only from `dlt`'s own schema), and the
  `append` / `replace` dispositions `load_via_sink` passes (`incremental`
  becomes `append`).

- DIFFERENT: `dlt` uses its own ephemeral local Iceberg catalog, because
  the fixture below removes the `ICEBERG_CATALOG__*` variables that
  `sink._install_catalog_env` sets process-wide (other test modules leave
  them behind, and they would send this pipeline to a Lakekeeper that does
  not exist); the bucket is a temporary local directory, there is no
  Lakekeeper, there is no partition spec and no `_ingested_at` column, and
  rows are plain dicts instead of a `sql_database` source. The one case a
  SQL source adds, a column hint whose precision grows from 32 to 64 bits,
  is reproduced by handing `dlt.resource` the same `columns` hint
  (`bigint`, `precision`) a reflected source carries.

`dlt`'s bookkeeping columns (`_dlt_id`, `_dlt_load_id`) are left out of the
assertions.

Run with:
~/.cache/rantai-dagster-venv/bin/python -m pytest dagster/dispar_orchestrate/test_schema_drift_loader.py -q
"""

from __future__ import annotations

import glob
from pathlib import Path
from typing import Any

import dlt
import pytest
from dlt.destinations import filesystem
from pyiceberg.table import StaticTable

DISPOSITIONS = ["append", "replace"]
TABLE = "drift"


@pytest.fixture(autouse=True)
def _no_lakekeeper_catalog(monkeypatch: pytest.MonkeyPatch) -> None:
    for name in (
        "ICEBERG_CATALOG__ICEBERG_CATALOG_NAME",
        "ICEBERG_CATALOG__ICEBERG_CATALOG_TYPE",
        "ICEBERG_CATALOG__ICEBERG_CATALOG_CONFIG",
    ):
        monkeypatch.delenv(name, raising=False)


def _is_bookkeeping(name: str) -> bool:
    return name.startswith("_dlt_")


def _load_twice(
    tmp_path: Path,
    disposition: str,
    first: list[dict[str, Any]],
    second: list[dict[str, Any]],
    first_hints: dict[str, Any] | None = None,
    second_hints: dict[str, Any] | None = None,
) -> tuple[dict[str, str], dict[str, str]]:
    """Load `first`, then `second`, into one table and return its columns
    twice: from `dlt`'s schema (name to data type, with the precision
    appended when set) and from the Iceberg table's own metadata (name to
    Iceberg type)."""
    bucket = tmp_path / "bucket"
    pipeline = dlt.pipeline(
        pipeline_name=f"drift_{disposition}",
        pipelines_dir=str(tmp_path / "pipelines"),
        destination=filesystem(bucket_url=str(bucket)),
        dataset_name="bronze",
    )
    for rows, hints in ((first, first_hints), (second, second_hints)):
        resource = dlt.resource(rows, name=TABLE, columns=hints or {})
        pipeline.run(resource, table_name=TABLE, table_format="iceberg", write_disposition=disposition)

    dlt_columns: dict[str, str] = {}
    for name, column in pipeline.default_schema.tables[TABLE]["columns"].items():
        if _is_bookkeeping(name):
            continue
        precision = column.get("precision")
        dlt_columns[name] = column["data_type"] + (f"({precision})" if precision else "")

    metadata = sorted(glob.glob(str(bucket / "**" / "metadata" / "*.metadata.json"), recursive=True))
    table = StaticTable.from_metadata(f"file://{metadata[-1]}")
    iceberg_columns = {
        field.name: str(field.field_type) for field in table.schema().fields if not _is_bookkeeping(field.name)
    }
    return dlt_columns, iceberg_columns


@pytest.mark.parametrize("disposition", DISPOSITIONS)
def test_a_column_that_appears_in_the_second_run_is_added_to_the_table(tmp_path: Path, disposition: str) -> None:
    dlt_columns, iceberg_columns = _load_twice(
        tmp_path, disposition, [{"a": 1, "b": 2}], [{"a": 1, "b": 2, "c": 3}]
    )
    assert dlt_columns == {"a": "bigint", "b": "bigint", "c": "bigint"}
    assert iceberg_columns == {"a": "long", "b": "long", "c": "long"}


@pytest.mark.parametrize("disposition", DISPOSITIONS)
def test_a_column_that_stops_arriving_stays_in_the_table(tmp_path: Path, disposition: str) -> None:
    dlt_columns, iceberg_columns = _load_twice(tmp_path, disposition, [{"a": 1, "b": 2}], [{"a": 1}])
    assert dlt_columns == {"a": "bigint", "b": "bigint"}
    assert iceberg_columns == {"a": "long", "b": "long"}


@pytest.mark.parametrize("disposition", DISPOSITIONS)
def test_integers_that_turn_into_text_land_in_a_second_column_beside_the_integer_one(
    tmp_path: Path, disposition: str
) -> None:
    dlt_columns, iceberg_columns = _load_twice(tmp_path, disposition, [{"a": 1}], [{"a": "not a number"}])
    assert dlt_columns == {"a": "bigint", "a__v_text": "text"}
    assert iceberg_columns == {"a": "long", "a__v_text": "string"}


@pytest.mark.parametrize("disposition", DISPOSITIONS)
def test_text_that_turns_into_integers_keeps_the_text_column_and_adds_no_second_one(
    tmp_path: Path, disposition: str
) -> None:
    dlt_columns, iceberg_columns = _load_twice(tmp_path, disposition, [{"a": "not a number"}], [{"a": 5}])
    assert dlt_columns == {"a": "text"}
    assert iceberg_columns == {"a": "string"}


@pytest.mark.parametrize("disposition", DISPOSITIONS)
def test_small_integers_that_grow_past_32_bits_stay_in_the_same_column(tmp_path: Path, disposition: str) -> None:
    dlt_columns, iceberg_columns = _load_twice(tmp_path, disposition, [{"a": 5}], [{"a": 5_000_000_000}])
    assert dlt_columns == {"a": "bigint"}
    assert iceberg_columns == {"a": "long"}


@pytest.mark.parametrize("disposition", DISPOSITIONS)
def test_a_column_hint_that_grows_from_32_to_64_bits_keeps_one_column_and_takes_the_new_precision(
    tmp_path: Path, disposition: str
) -> None:
    # The shape a reflected `sql_database` source hands the loader when the
    # source column goes from INT to BIGINT.
    dlt_columns, iceberg_columns = _load_twice(
        tmp_path,
        disposition,
        [{"a": 5}],
        [{"a": 5_000_000_000}],
        first_hints={"a": {"data_type": "bigint", "precision": 32}},
        second_hints={"a": {"data_type": "bigint", "precision": 64}},
    )
    assert dlt_columns == {"a": "bigint(64)"}
    assert iceberg_columns == {"a": "long"}
