"""Several files into one table (`DATA-17` slice 5, plan AD4): the job reads the objects of
`storage_keys` in order through the same readers and the same caster, matches columns by
name, and reports the sum of the rows.

No network: the store is a dictionary of bytes by key, the sink is the fake of
`test_file_ingest._Harness`.
"""

from __future__ import annotations

import gzip
import io
import json
import zipfile

import pyarrow as pa
import pyarrow.parquet as pq
import pytest

from dispar_orchestrate import file_ingest
from dispar_orchestrate.file_ingest import (
    LOAD_FAILED,
    MAX_LOAD_FILES,
    NO_ROWS,
    TOO_LARGE_UNPACKED,
    TOO_MANY_ROWS,
    UNREADABLE,
    FileIngestParams,
    LoadFailure,
)
from dispar_orchestrate.test_file_ingest import _Harness, _fails_with, _params
from dispar_orchestrate.upload_types import ColumnSpec

TENANT = "00000000-0000-0000-0000-00000000a001"


def _key(n: int, suffix: str = "csv") -> str:
    return f"uploads/{TENANT}/u-{n}.{suffix}"


class _Several(_Harness):
    """A harness whose store holds one object per key, and that remembers which were read."""

    def __init__(self, objects: dict[str, bytes], **kwargs) -> None:
        super().__init__(**kwargs)
        self.objects = objects

    def read(self, config, key: str):
        self.reads.append(key)
        return io.BytesIO(self.objects[key])


def _load(objects: dict[str, bytes], **overrides):
    harness = _Several(objects)
    params = _params(storage_keys=tuple(objects), **overrides)
    summary = harness.run(params)
    return harness, summary


A = b"id,name,qty\n1,bolt,10\n2,nut,20\n"
B = b"qty,id,name\n30,3,screw\n40,4,washer\n"  # the same columns in another order
C = b"ID,Name,Qty\n5,pin,50\n"  # the same names once cleaned, in other letters
D = b"id,name,qty,notes\n6,rivet,60,fragile\n"  # one column more


def test_three_csv_files_load_into_one_table_in_the_first_files_column_order_with_every_row():
    harness, summary = _load({_key(1): A, _key(2): B, _key(3): C})
    [loaded] = harness.loads
    assert harness.reads == [_key(1), _key(2), _key(3)], "read in the order given, each once"
    assert list(loaded["columns"]) == ["id", "name", "qty"]
    assert [(row["id"], row["name"], row["qty"]) for row in loaded["rows"]] == [
        ("1", "bolt", "10"),
        ("2", "nut", "20"),
        ("3", "screw", "30"),
        ("4", "washer", "40"),
        ("5", "pin", "50"),
    ]
    assert summary.parsed_rows == 5, "the row count is the sum"
    assert summary.columns == 3
    [(table, _)] = [(r["object_name"], r) for r in harness.recorded]
    assert table == "g9_orders" and len(harness.recorded) == 1, "one outcome, whatever the files"
    assert harness.recorded[0]["connector_id"] == "upload:u-1", "filed under the primary"


def test_a_file_with_other_columns_fails_the_load_and_writes_nothing():
    harness = _Several({_key(1): A, _key(2): D})
    failure = _fails_with(harness, LOAD_FAILED, _params(storage_keys=(_key(1), _key(2))))
    assert "same columns" in str(failure.__cause__)
    assert harness.recorded[0]["status"] == "failed"
    assert harness.registered == [], "nothing was registered"
    assert harness.loads == [], "the sink was handed no rows: the mismatch is found as they are read"


def test_a_typed_load_casts_the_rows_of_every_file_by_the_first_files_column_list():
    columns = (
        ColumnSpec("id", "id", "integer", "", False),
        ColumnSpec("name", "name", "text", "", False),
        ColumnSpec("qty", "quantity", "integer", "", False),
    )
    harness, summary = _load({_key(1): A, _key(2): B}, columns=columns, on_bad_value="stop")
    [loaded] = harness.loads
    assert list(loaded["columns"]) == ["id", "name", "quantity"]
    assert [(row["id"], row["quantity"]) for row in loaded["rows"]] == [(1, 10), (2, 20), (3, 30), (4, 40)]
    assert summary.parsed_rows == 4


def test_a_value_of_the_second_file_that_does_not_fit_is_reported_with_its_row_counted_across_files():
    columns = (
        ColumnSpec("id", "id", "integer", "", False),
        ColumnSpec("name", "name", "text", "", False),
        ColumnSpec("qty", "qty", "integer", "", False),
    )
    bad = b"id,name,qty\n7,cap,n/a\n"
    harness = _Several({_key(1): A, _key(2): bad})
    summary = harness.run(_params(storage_keys=(_key(1), _key(2)), columns=columns, on_bad_value="null"))
    assert summary.bad_values == 1
    [(key, payload)] = harness.reports
    assert key == _key(1), "the report is kept beside the primary"
    assert json.loads(payload)["rows"] == [{"row": 3, "column": "qty", "value": "n/a"}]


def test_parquet_files_load_together_matched_by_name_whatever_the_order_they_wrote_them_in():
    def parquet(table: pa.Table) -> bytes:
        sink = io.BytesIO()
        pq.write_table(table, sink)
        return sink.getvalue()

    first = parquet(pa.table({"id": [1, 2], "name": ["a", "b"]}))
    second = parquet(pa.table({"name": ["c"], "id": [3]}))
    harness = _Several({_key(1, "parquet"): first, _key(2, "parquet"): second})
    summary = harness.run(_params(format="parquet", storage_keys=(_key(1, "parquet"), _key(2, "parquet"))))
    [loaded] = harness.loads
    assert list(loaded["columns"]) == ["id", "name"]
    assert [(row["id"], row["name"]) for row in loaded["rows"]] == [("1", "a"), ("2", "b"), ("3", "c")]
    assert summary.parsed_rows == 3


def test_parquet_files_with_other_columns_fail_the_load_before_anything_is_written():
    def parquet(table: pa.Table) -> bytes:
        sink = io.BytesIO()
        pq.write_table(table, sink)
        return sink.getvalue()

    harness = _Several(
        {
            _key(1, "parquet"): parquet(pa.table({"id": [1]})),
            _key(2, "parquet"): parquet(pa.table({"id": [2], "extra": ["x"]})),
        }
    )
    _fails_with(harness, LOAD_FAILED, _params(format="parquet", storage_keys=(_key(1, "parquet"), _key(2, "parquet"))))
    assert harness.loads == [], "the footers are compared before the sink is asked"


def test_json_lines_files_load_together_with_the_keys_of_all_of_them_as_columns():
    first = b'{"id": 1, "name": "a"}\n{"id": 2, "name": "b"}\n'
    second = b'{"name": "c", "id": 3}\n'
    harness, summary = _load({_key(1, "jsonl"): first, _key(2, "jsonl"): second}, format="jsonl")
    [loaded] = harness.loads
    assert list(loaded["columns"]) == ["id", "name"]
    assert [(row["id"], row["name"]) for row in loaded["rows"]] == [("1", "a"), ("2", "b"), ("3", "c")]
    assert summary.parsed_rows == 3


def test_zips_load_together_each_zips_files_one_after_the_other():
    def archive(entries: list[tuple[str, bytes]]) -> bytes:
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as zipped:
            for name, data in entries:
                zipped.writestr(name, data)
        return buffer.getvalue()

    objects = {_key(1, "zip"): archive([("a.csv", A), ("b.csv", B)]), _key(2, "zip"): archive([("c.csv", C)])}
    harness, summary = _load(objects, compression="zip")
    assert summary.parsed_rows == 5
    assert [row["id"] for row in harness.loads[0]["rows"]] == ["1", "2", "3", "4", "5"]


def test_gzip_files_share_one_unpacked_budget_so_the_cap_counts_all_of_the_load():
    one = gzip.compress(A)
    second = gzip.compress(B)
    each = max(len(A), len(B))
    # Each file fits the cap alone; together they do not.
    ok = _Several({_key(1, "gz"): one})
    ok.run(_params(storage_keys=(_key(1, "gz"),), compression="gzip", max_bytes=len(A)))
    together = _Several({_key(1, "gz"): one, _key(2, "gz"): second})
    _fails_with(
        together,
        TOO_LARGE_UNPACKED,
        _params(storage_keys=(_key(1, "gz"), _key(2, "gz")), compression="gzip", max_bytes=each + 8),
    )
    assert together.loads == []
    fits = _Several({_key(1, "gz"): one, _key(2, "gz"): second})
    fits.run(_params(storage_keys=(_key(1, "gz"), _key(2, "gz")), compression="gzip", max_bytes=len(A) + len(B)))


def test_the_row_cap_counts_the_rows_of_all_the_files(monkeypatch):
    monkeypatch.setattr(file_ingest, "MAX_ROWS", 3)
    harness = _Several({_key(1): A, _key(2): B})
    _fails_with(harness, TOO_MANY_ROWS, _params(storage_keys=(_key(1), _key(2))))
    ok = _Several({_key(1): A, _key(2): C})
    ok.run(_params(storage_keys=(_key(1), _key(2))))


def test_a_file_that_cannot_be_opened_is_unreadable_and_the_ones_opened_are_closed():
    opened: list[io.BytesIO] = []

    class Failing(_Several):
        def read(self, config, key: str):
            if key == _key(2):
                raise OSError("no such object")
            handle = super().read(config, key)
            opened.append(handle)
            return handle

    harness = Failing({_key(1): A, _key(2): B})
    _fails_with(harness, UNREADABLE, _params(storage_keys=(_key(1), _key(2))))
    assert opened and all(handle.closed for handle in opened)


def test_a_load_takes_one_to_ten_different_files():
    keys = tuple(_key(n) for n in range(MAX_LOAD_FILES + 1))
    objects = dict.fromkeys(keys, A)
    for bad in [(), keys, (_key(1), _key(1))]:
        failure = _fails_with(_Several(objects), LOAD_FAILED, _params(storage_keys=bad))
        assert "storage_keys" in str(failure.__cause__)
    ok = _Several(objects)
    ok.run(_params(storage_keys=keys[:MAX_LOAD_FILES]))
    assert len(ok.loads[0]["rows"]) == 2 * MAX_LOAD_FILES


def test_a_key_outside_the_uploads_prefix_in_the_list_is_refused_before_anything_is_read():
    harness = _Several({_key(1): A, "other/u-2.csv": B})
    _fails_with(harness, UNREADABLE, _params(storage_keys=(_key(1), "other/u-2.csv")))
    assert harness.reads == []


def test_several_files_with_no_row_at_all_fail_with_no_rows_below_the_header():
    header_only = b"id,name,qty\n"
    _fails_with(_Several({_key(1): header_only, _key(2): header_only}), NO_ROWS, _params(storage_keys=(_key(1), _key(2))))


def test_the_op_config_takes_a_list_of_keys_and_the_params_keep_their_order():
    config = {
        "upload_id": "u-1",
        "storage_keys": [_key(1), _key(2), _key(3)],
        "format": "delimited",
        "compression": "none",
        "max_bytes": 100,
        "bronze_table_name": "g9_orders",
        "load_mode": "replace",
        "encoding": "utf-8",
        "delimiter": ",",
        "header_row": 0,
        "columns": [],
        "on_bad_value": "stop",
    }
    params = FileIngestParams.from_op_config(config)
    assert params.storage_keys == (_key(1), _key(2), _key(3))
    assert "bronze_table_name" in file_ingest.CONFIG_SCHEMA, "the DATA-19 sensor path does not move"
    assert set(file_ingest.CONFIG_SCHEMA) == set(config), "the config stays closed, every field required"
