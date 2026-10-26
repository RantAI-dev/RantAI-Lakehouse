"""The streamed Iceberg write (`adapters/iceberg_stream.py`, `PR #186 review BLOCKER 6`) writes
what `dlt` wrote: one commit per load, `replace` replacing, `append` adding, an empty replace
emptying, and a load that fails part way committing nothing. The groups are made tiny here
(`FLUSH_BYTES`, `READ_ROWS`) so that a few hundred rows are several groups.

Driven on a local catalog, the harness of `test_sink_dedup_marker.py`.
"""

from __future__ import annotations

from collections.abc import Iterator
from datetime import datetime, timezone

import dlt
import pyarrow as pa
import pytest

from dispar_orchestrate.adapters import iceberg_stream
from dispar_orchestrate.adapters.sink import LoadPlan, load_via_sink
from dispar_orchestrate.test_load_modes_measurement import Lab, lab  # noqa: F401 -- `lab` is a fixture

# Runs a real dlt pipeline against a local Iceberg table (registered in pyproject.toml);
# CI runs these in `dagster-loader-tests`, the rest in `dagster-unit-tests`.
pytestmark = pytest.mark.loader


@pytest.fixture(autouse=True)
def tiny_groups(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(iceberg_stream, "FLUSH_BYTES", 2_000)
    monkeypatch.setattr(iceberg_stream, "READ_ROWS", 50)


def _batches(rows: int, fail_after: int | None = None) -> Iterator[pa.Table]:
    for start in range(0, rows, 100):
        if fail_after is not None and start >= fail_after:
            raise RuntimeError("the reader failed")
        stop = min(start + 100, rows)
        yield pa.table({"id": pa.array([str(n) for n in range(start, stop)], type=pa.string())})


def _resource(rows: int, fail_after: int | None = None) -> dlt.sources.DltResource:
    resource = dlt.resource(_batches(rows, fail_after), name="streamed")
    resource.apply_hints(columns=[{"name": "id", "data_type": "text"}])
    return resource


def _ids(lab: Lab) -> list[str]:
    return sorted((row["id"] for row in lab.rows("streamed")), key=int)


def test_a_replace_of_several_groups_is_one_commit_with_every_row(lab: Lab, monkeypatch: pytest.MonkeyPatch) -> None:
    cut: list[int] = []
    real = iceberg_stream.groups

    def counting(dataset, schema=None):
        # `PR #186 review SHOULD-FIX 2`: `groups` takes the unified schema now (it computes
        # it when it is not given one), so the wrapper takes it too.
        for group in real(dataset, schema):
            cut.append(group.num_rows)
            yield group

    monkeypatch.setattr(iceberg_stream, "groups", counting)
    load_via_sink(_resource(400), "streamed", lab.config, LoadPlan(mode="replace"))
    assert _ids(lab) == [str(n) for n in range(400)]
    assert len(cut) > 1, "the load went through the streamed path, in several groups"
    first = lab.commits("streamed")

    load_via_sink(_resource(250), "streamed", lab.config, LoadPlan(mode="replace"))

    assert _ids(lab) == [str(n) for n in range(250)], "the earlier rows are replaced, not kept"
    assert lab.commits("streamed") - first == 1, "all the groups of a load are one commit"


def test_an_append_of_several_groups_adds_to_what_the_table_holds(lab: Lab) -> None:
    load_via_sink(_resource(300), "streamed", lab.config, LoadPlan(mode="append"))
    load_via_sink(_resource(300), "streamed", lab.config, LoadPlan(mode="append"))
    assert len(lab.rows("streamed")) == 600


def test_a_load_that_fails_after_some_groups_leaves_the_table_as_it_was(lab: Lab) -> None:
    load_via_sink(_resource(120), "streamed", lab.config, LoadPlan(mode="replace"))
    before = lab.commits("streamed")

    # The reader fails inside dlt's extract, before any group is written; the point is that
    # whatever the load had staged is not committed.
    with pytest.raises(Exception):  # noqa: B017,PT011 -- dlt wraps what the generator raised
        load_via_sink(_resource(500, fail_after=300), "streamed", lab.config, LoadPlan(mode="replace"))

    assert lab.commits("streamed") == before
    assert len(lab.rows("streamed")) == 120


def test_groups_are_cut_by_bytes_and_none_is_lost(tmp_path) -> None:
    import pyarrow.parquet as pq

    path = tmp_path / "part.parquet"
    pq.write_table(pa.table({"id": pa.array([str(n) for n in range(1000)], type=pa.string())}), path, row_group_size=100)

    class Dataset:
        files = [str(path)]

    cut = list(iceberg_stream.groups(Dataset()))

    assert len(cut) > 1, "2,000 bytes is far less than the file"
    assert sum(group.num_rows for group in cut) == 1000


def test_groups_of_files_with_different_columns_are_joined_and_padded(tmp_path) -> None:
    """`PR #186 review SHOULD-FIX 2`: `dlt` starts a new normalized file when a table's column
    count changes, so the files of one load do not all have the same schema, and
    `pa.Table.from_batches` refuses batches of different schemas."""
    import pyarrow.parquet as pq

    first = tmp_path / "one.parquet"
    second = tmp_path / "two.parquet"
    pq.write_table(pa.table({"id": pa.array(["1"], type=pa.string())}), first)
    pq.write_table(
        pa.table({"id": pa.array(["2"], type=pa.string()), "late": pa.array(["x"], type=pa.string())}),
        second,
    )

    class Dataset:
        files = [str(first), str(second)]

    cut = list(iceberg_stream.groups(Dataset()))

    assert sum(group.num_rows for group in cut) == 2
    table = pa.concat_tables(cut)
    assert table.schema.names == ["id", "late"], "the union of the files' columns"
    assert table.column("late").to_pylist() == [None, "x"], "the column a file does not have is null in it"


def test_a_write_whose_files_have_different_columns_adds_the_column_and_writes_it(lab: Lab, tmp_path) -> None:
    """`PR #186 review SHOULD-FIX 2`, at the write: the files of one load do not all have the
    same columns, the column the first file does not have is null in it, and the table
    carries it (the table is evolved inside the transaction that writes the rows)."""
    import pyarrow.parquet as pq

    from dispar_orchestrate.adapters import sink as sink_module

    load_via_sink(_resource(10), "streamed", lab.config, LoadPlan(mode="replace"))
    pipeline = lab.pipeline("streamed")
    table = sink_module.history.open_table(pipeline, "streamed", sync=False)
    arrow = table.schema().as_arrow()

    def _row(with_late: bool) -> pa.Table:
        data = {}
        for field in arrow:
            if pa.types.is_timestamp(field.type):
                data[field.name] = pa.array([datetime.now(timezone.utc)], type=field.type)
            else:
                data[field.name] = pa.array(["x"], type=field.type)
        if with_late:
            data["late"] = pa.array(["y"], type=pa.string())
        return pa.table(data)

    first = tmp_path / "one.parquet"
    second = tmp_path / "two.parquet"
    pq.write_table(_row(False), first)
    pq.write_table(_row(True), second)

    class Dataset:
        files = [str(first), str(second)]
        schema = arrow

    iceberg_stream._write(table, iceberg_stream._Streamed(Dataset()), "append", lambda **_kwargs: None)

    written = lab.rows("streamed")
    assert len(written) == 12, "the ten rows of the load and the two of the write"
    lates = [row.get("late") for row in written]
    assert lates.count("y") == 1, "the new column is written"
    assert lates.count(None) == 11, "the column the first file does not have is null in it"
    assert "late" in [field.name for field in lab.table("streamed").schema().fields], "the table carries it"


def test_a_load_whose_later_file_has_a_new_column_writes_it(lab: Lab) -> None:
    """`PR #186 review SHOULD-FIX 2`, end to end: a resource whose second batch brings a field
    the first one does not have. The load writes the column; the table carries it."""
    def rows():
        yield pa.table({"id": pa.array(["1"], type=pa.string())})
        yield pa.table({"id": pa.array(["2"], type=pa.string()), "late": pa.array(["x"], type=pa.string())})

    resource = dlt.resource(rows(), name="streamed")
    resource.apply_hints(columns=[{"name": "id", "data_type": "text"}])
    load_via_sink(resource, "streamed", lab.config, LoadPlan(mode="replace"))

    written = {row["id"]: row for row in lab.rows("streamed")}
    assert set(written) == {"1", "2"}, "both rows are written"
    assert written["1"].get("late") is None, "the column the first file does not have is null in it"
    assert written["2"].get("late") == "x", "the new column is written, not dropped"
    fields = [field.name for field in lab.table("streamed").schema().fields]
    assert "late" in fields, "the table carries the new column"


def test_a_load_of_no_rows_still_commits_one_snapshot_as_dlt_did(lab: Lab) -> None:
    """`history.ensure_columns` adds a column with a zero-row load and a scan reads the schema
    of the current snapshot, so the empty commit is part of the contract."""
    load_via_sink(_resource(10), "streamed", lab.config, LoadPlan(mode="append"))
    before = lab.snapshots("streamed")

    nothing = dlt.resource([pa.table({"id": pa.array([], type=pa.string())})], name="streamed")
    nothing.apply_hints(columns=[{"name": "id", "data_type": "text"}])
    load_via_sink(nothing, "streamed", lab.config, LoadPlan(mode="append"))

    assert lab.snapshots("streamed") == before + 1
    assert len(lab.rows("streamed")) == 10
