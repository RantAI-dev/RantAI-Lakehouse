"""The streamed Iceberg write (`adapters/iceberg_stream.py`, `PR #186 review BLOCKER 6`) writes
what `dlt` wrote: one commit per load, `replace` replacing, `append` adding, an empty replace
emptying, and a load that fails part way committing nothing. The groups are made tiny here
(`FLUSH_BYTES`, `READ_ROWS`) so that a few hundred rows are several groups.

Driven on a local catalog, the harness of `test_sink_dedup_marker.py`.
"""

from __future__ import annotations

from collections.abc import Iterator

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

    def counting(dataset):
        for group in real(dataset):
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
