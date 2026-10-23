"""The job's Parquet reader (`DATA-17` T2.2): the text of every value, against the
fixture the API's converter is tested against, and the job around it.

No network: the fixtures are `ops/fixtures/parquet/`, written by
`make_parquet.py`, and the store is a `BytesIO`.
"""

from __future__ import annotations

import io
import json
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq
import pytest

from dispar_orchestrate import file_ingest, parquet_ingest
from dispar_orchestrate.file_ingest import (
    HEADER_NO_COLUMNS,
    LOAD_FAILED,
    NO_ROWS,
    TOO_MANY_COLUMNS,
    UNREADABLE,
    LoadFailure,
    open_parquet_stream,
)
from dispar_orchestrate.test_file_ingest import _Harness, _fails_with, _params

FIXTURES = Path(__file__).resolve().parents[2] / "ops" / "fixtures" / "parquet"


def _rows(table_batches) -> list[list[str]]:
    return [list(r) for t in table_batches for r in zip(*[t[c].to_pylist() for c in t.column_names])]


def _read(name: str) -> list[list[str]]:
    stream = open_parquet_stream(open(FIXTURES / name, "rb"))
    return _rows(stream.batches())


def test_types_parquet_is_written_to_the_records_the_api_converter_is_tested_against():
    shared = json.loads((FIXTURES / "types.expected.json").read_text(encoding="utf-8"))["records"]
    stream = open_parquet_stream(open(FIXTURES / "types.parquet", "rb"))
    assert stream.columns == shared[0]
    assert _rows(stream.batches()) == shared[1:]
    assert stream.row_count == 3


def test_orders_and_the_legacy_int96_file_come_out_as_the_api_wrote_them():
    assert _read("orders.parquet") == [
        ["A-100", "Bolt, hex M6", "10", "1.50", "2025-09-24"],
        ["A-200", "Washer", "250", "0.05", "2025-09-25"],
        ["A-300", "Flange DN50 – steel", "4", "12.00", "2025-10-01"],
    ]
    # INT96 as microseconds, local (no Z), and a year no nanosecond count can hold.
    assert _read("legacy_int96.parquet") == [
        ["1", "2025-09-24T13:30:05.123456"],
        ["2", "0001-01-01T00:00:00"],
        ["3", ""],
    ]


def test_a_file_with_no_rows_a_binary_column_or_a_nested_column_is_refused_with_its_reason():
    with pytest.raises(LoadFailure) as empty:
        open_parquet_stream(open(FIXTURES / "empty.parquet", "rb"))
    assert empty.value.reason == NO_ROWS
    for name, word in (("binary_column.parquet", "binary"), ("nested_column.parquet", "list")):
        with pytest.raises(LoadFailure) as refused:
            open_parquet_stream(open(FIXTURES / name, "rb"))
        assert refused.value.reason == UNREADABLE
        assert isinstance(refused.value.__cause__, parquet_ingest.UnsupportedColumn)
        assert word in str(refused.value.__cause__)


@pytest.mark.parametrize("raw", [b"", b"PAR1", b"not parquet at all, just text\n" * 10, b"PARE" + b"\x00" * 40])
def test_bytes_that_are_not_a_parquet_file_are_unreadable(raw):
    with pytest.raises(LoadFailure) as caught:
        open_parquet_stream(io.BytesIO(raw))
    assert caught.value.reason == UNREADABLE


def test_a_file_with_more_than_1000_columns_is_refused_before_a_row_is_read():
    wide = pa.table({f"c{n}": pa.array(["x"], pa.string()) for n in range(1_001)})
    sink = io.BytesIO()
    pq.write_table(wide, sink)
    sink.seek(0)
    with pytest.raises(LoadFailure) as caught:
        open_parquet_stream(sink)
    assert caught.value.reason == TOO_MANY_COLUMNS
    assert open_parquet_stream(_parquet_of(pa.table({f"c{n}": ["x"] for n in range(1_000)}))).row_count == 0


def _parquet_of(table: pa.Table, **options) -> io.BytesIO:
    sink = io.BytesIO()
    pq.write_table(table, sink, **options)
    sink.seek(0)
    return sink


def test_column_names_are_cleaned_as_a_csv_header_is():
    table = pa.table({"Order ID": ["1"], "Order ID ": ["2"], "größe": ["3"]})
    stream = open_parquet_stream(_parquet_of(table))
    assert stream.columns == ["order_id", "order_id_2", "gr_e"]
    batch = next(iter(stream.batches()))
    assert batch.column_names == stream.columns


def test_batches_are_at_most_the_batch_size_and_the_rows_are_counted_as_they_pass(monkeypatch):
    monkeypatch.setattr(file_ingest, "BATCH_ROWS", 4)
    stream = open_parquet_stream(_parquet_of(pa.table({"n": list(range(10))}), row_group_size=3))
    sizes = []
    for batch in stream.batches():
        sizes.append(batch.num_rows)
        assert stream.row_count == sum(sizes)
        assert batch.schema.types == [pa.string()]
    assert max(sizes) <= 4 and sum(sizes) == 10


def test_a_page_that_cannot_be_read_is_unreadable_and_not_a_failed_load():
    good = _parquet_of(pa.table({"n": list(range(1000))}, schema=pa.schema([("n", pa.int64())]))).getvalue()
    # Damage the middle of the data (the footer at the end stays intact).
    damaged = bytearray(good)
    for i in range(10, 200):
        damaged[i] ^= 0xFF
    stream = open_parquet_stream(io.BytesIO(bytes(damaged)))
    with pytest.raises(LoadFailure) as caught:
        list(stream.batches())
    assert caught.value.reason == UNREADABLE


@pytest.mark.parametrize(
    "value,single,text",
    [
        (0.5, False, "0.5"),
        (0.1 + 0.2, False, "0.30000000000000004"),
        (1e-7, False, "1e-7"),
        (1e21, False, "1e21"),
        (1e20, False, "100000000000000000000"),
        (1e-6, False, "0.000001"),
        (1.5e22, False, "1.5e22"),
        (-2.5e-9, False, "-2.5e-9"),
        (123456.0, False, "123456"),
        (-0.0, False, "0"),
        (0.0, False, "0"),
        (float("nan"), False, "NaN"),
        (float("inf"), False, "inf"),
        (float("-inf"), False, "-inf"),
        (float(__import__("struct").unpack("f", __import__("struct").pack("f", 0.1))[0]), True, "0.1"),
        (2.0, True, "2"),
        (3.4028234663852886e38, True, "3.4028235e38"),
    ],
)
def test_floats_are_written_in_the_shortest_form_that_reads_back(value, single, text):
    assert parquet_ingest._float_text(value, single=single) == text


def test_a_date_time_or_timestamp_outside_what_the_text_can_say_is_the_integer_the_file_holds():
    assert parquet_ingest._date_text(-719_529) is None  # before year 0
    far = pa.array([2_932_897 + 10_000_000], pa.date32())
    assert parquet_ingest.text_of(far).to_pylist() == [str(2_932_897 + 10_000_000)]
    assert parquet_ingest._time_text(86_400, "s") == "86400"
    assert parquet_ingest._time_text(-1, "s") == "-1"
    assert parquet_ingest._timestamp_text(10**18, "s", True) == str(10**18)
    assert parquet_ingest._timestamp_text(0, "s", True) == "1970-01-01T00:00:00Z"
    assert parquet_ingest._timestamp_text(-1, "ms", False) == "1969-12-31T23:59:59.999"


# ── Through the job ──────────────────────────────────────────────────────


def test_a_parquet_load_hands_the_sink_text_batches_and_records_one_succeeded_row():
    harness = _Harness(raw=(FIXTURES / "orders.parquet").read_bytes(), sink_rows=3, total=3)
    summary = harness.run(_params(format="parquet", bronze_table_name="orders_raw"))
    [loaded] = harness.loads
    assert list(loaded["columns"]) == ["sku", "name", "qty", "price", "received"]
    assert {hints["data_type"] for hints in loaded["columns"].values()} == {"text"}
    assert [list(row.values()) for row in loaded["rows"]][0] == ["A-100", "Bolt, hex M6", "10", "1.50", "2025-09-24"]
    assert summary.parsed_rows == 3
    assert [row["status"] for row in harness.recorded] == ["succeeded"]


def test_a_parquet_file_the_job_cannot_read_records_the_reason_and_writes_nothing():
    harness = _Harness(raw=(FIXTURES / "binary_column.parquet").read_bytes())
    _fails_with(harness, UNREADABLE, _params(format="parquet"))
    assert harness.loads == []
    harness = _Harness(raw=(FIXTURES / "empty.parquet").read_bytes())
    _fails_with(harness, NO_ROWS, _params(format="parquet"))


def test_a_format_the_api_would_not_send_fails_the_launch_before_anything_is_read():
    harness = _Harness()
    failure = _fails_with(harness, LOAD_FAILED, _params(format="orc"))
    assert harness.reads == []
    assert "format" in str(failure.__cause__)
