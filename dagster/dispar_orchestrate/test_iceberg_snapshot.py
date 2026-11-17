"""One Iceberg snapshot per load (`DATA-36`): the guard of `adapters/iceberg_snapshot.py`.

The module writes a load's groups into ONE snapshot producer, so a load leaves ONE snapshot
behind and no half-loaded state of the table can be read (every snapshot of a table is a
state the console's time-travel picker offers). The tests here are against a plain local
`pyiceberg` table, without `dlt`: the module is the thing under test, not the load paths
that call it (`test_iceberg_stream.py` and `test_typed_load.py` cover those).

The first test is the guard of the PINNED private API: `pyiceberg.io.pyarrow.
_dataframe_to_data_files` and `pyiceberg.io.pyarrow._check_pyarrow_schema_compatible` are
private functions of the pinned `pyiceberg` 0.12.0 (`dagster/pyproject.toml`), and an
upgrade that moves them has to fail here, before a load does.
"""

from __future__ import annotations

import inspect
from typing import Any

import pyarrow as pa
import pytest
from pyiceberg.catalog import load_catalog
from pyiceberg.schema import Schema
from pyiceberg.table import Operation
from pyiceberg.types import NestedField, StringType

from dispar_orchestrate.adapters import iceberg_snapshot


@pytest.fixture
def table(tmp_path) -> Any:
    """A local Iceberg table with one text column, on the disk of `tmp_path`."""
    catalog = load_catalog("local", type="in-memory", warehouse=str(tmp_path / "warehouse"))
    catalog.create_namespace("bronze")
    schema = Schema(NestedField(1, "id", StringType(), required=False))
    return catalog.create_table("bronze.t", schema=schema)


def _group(prefix: str, count: int) -> pa.Table:
    return pa.table({"id": pa.array([f"{prefix}{n}" for n in range(count)], type=pa.string())})


def _ids(table: Any) -> list[str]:
    return sorted(row["id"] for row in table.scan().to_arrow().to_pylist())


def test_the_pinned_pyiceberg_private_api_is_still_there() -> None:
    """`DATA-36`: the write uses two PRIVATE functions of `pyiceberg` 0.12.0 (the version
    pinned in `dagster/pyproject.toml`) and two producer methods. `pyiceberg` offers no
    public way to add a data file to a producer, so the two functions are pinned; an
    upgrade that moves or renames them has to fail HERE, before a load does."""
    from pyiceberg.io.pyarrow import _check_pyarrow_schema_compatible, _dataframe_to_data_files
    from pyiceberg.table.update.snapshot import UpdateSnapshot

    assert {"table_metadata", "df", "io", "write_uuid", "counter"} <= set(
        inspect.signature(_dataframe_to_data_files).parameters
    ), "pyiceberg.io.pyarrow._dataframe_to_data_files changed shape"
    assert {"requested_schema", "provided_schema", "downcast_ns_timestamp_to_us", "format_version"} <= set(
        inspect.signature(_check_pyarrow_schema_compatible).parameters
    ), "pyiceberg.io.pyarrow._check_pyarrow_schema_compatible changed shape"
    assert hasattr(UpdateSnapshot, "fast_append"), "the producer cannot fast-append any more"
    assert hasattr(UpdateSnapshot, "overwrite"), "the producer cannot overwrite any more"


def test_an_append_of_several_groups_leaves_one_snapshot(table: Any) -> None:
    with table.transaction() as transaction:
        iceberg_snapshot.write_one_snapshot(table, transaction, [_group("a", 3), _group("b", 3)])

    assert len(table.snapshots()) == 1, "one snapshot, not one per group"
    assert table.metadata.current_snapshot().summary.operation == Operation.APPEND
    # The two groups are two data files: `pyiceberg` names a data file
    # `00000-{task_id}-{write_uuid}.parquet` and the task id comes from the counter the
    # module shares over the whole load. A counter per group gives both groups the SAME
    # name, and the second one overwrites the first one's rows.
    assert _ids(table) == ["a0", "a1", "a2", "b0", "b1", "b2"], "both groups are in the table"


def test_a_replace_of_several_groups_leaves_one_snapshot_and_drops_the_old_rows(table: Any) -> None:
    with table.transaction() as transaction:
        iceberg_snapshot.write_one_snapshot(table, transaction, [_group("a", 3)])

    assert len(table.snapshots()) == 1

    with table.transaction() as transaction:
        iceberg_snapshot.write_one_snapshot(
            table, transaction, [_group("b", 3), _group("c", 3)], replace=True
        )

    assert len(table.snapshots()) == 2, "the replace is one snapshot, not one per group"
    assert table.metadata.current_snapshot().summary.operation == Operation.OVERWRITE
    assert _ids(table) == ["b0", "b1", "b2", "c0", "c1", "c2"], "the old rows are gone"


def test_a_replace_of_no_rows_leaves_one_snapshot_and_empties_the_table(table: Any) -> None:
    with table.transaction() as transaction:
        iceberg_snapshot.write_one_snapshot(table, transaction, [_group("a", 3)])

    with table.transaction() as transaction:
        iceberg_snapshot.write_one_snapshot(table, transaction, [], replace=True)

    assert len(table.snapshots()) == 2, "the replace is one snapshot"
    assert table.scan().to_arrow().num_rows == 0, "the table is empty"


def test_an_append_of_no_rows_leaves_one_snapshot(table: Any) -> None:
    with table.transaction() as transaction:
        iceberg_snapshot.write_one_snapshot(table, transaction, [])

    assert len(table.snapshots()) == 1, "the empty load commits its one snapshot"
    assert table.scan().to_arrow().num_rows == 0


def test_the_one_snapshot_carries_the_snapshot_properties_it_is_given(table: Any) -> None:
    """`DATA-36` decision 2: the snapshot properties are ONE set, on the ONE snapshot."""
    with table.transaction() as transaction:
        iceberg_snapshot.write_one_snapshot(
            table,
            transaction,
            [_group("a", 3), _group("b", 3)],
            snapshot_properties={"rantai.author": "connector c, run 1a2b"},
        )

    summary = table.metadata.current_snapshot().summary
    assert summary.additional_properties["rantai.author"] == "connector c, run 1a2b"
    assert len(table.snapshots()) == 1, "one snapshot, and it carries the properties"


def test_a_group_that_does_not_fit_the_table_is_refused(table: Any) -> None:
    """The check `pyiceberg`'s own `Transaction.append` makes: a column the table does not
    have would be DROPPED by the write, without a word."""
    with pytest.raises(ValueError):
        with table.transaction() as transaction:
            iceberg_snapshot.write_one_snapshot(
                table, transaction, [pa.table({"id": pa.array(["a"]), "extra": pa.array(["x"])})]
            )

    assert len(table.snapshots()) == 0, "nothing was committed"
