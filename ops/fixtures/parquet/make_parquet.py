"""Regenerates the Parquet fixtures of this directory.

    PYTHONPATH=<dir holding pyarrow> python3 ops/fixtures/parquet/make_parquet.py

Needs `pyarrow` (any recent release; 25.0.1 wrote the committed files). It is
not a dependency of the repo: unpack its wheel into a scratch directory and
put that on `PYTHONPATH`. Every value is invented. The files are not
byte-reproducible (the writer records its version), and nothing compares
their bytes: the tests compare what they hold, against the literals in
`rust/crates/lakehouse-api/src/upload_parquet.rs` and
`rust/crates/lakehouse-api/tests/upload_routes.rs`.

Written here:

- `orders.parquet`: three rows of an order list (string, int64, decimal(10,2),
  date32). The Parquet twin of the `Stock` sheet of `../workbooks/stock.xlsx`.
- `types.parquet`: one column of every type the conversion writes as text, and
  a last row of nothing but nulls.
- `legacy_int96.parquet`: timestamps in the legacy INT96 physical type, one of
  them in the year 1 (outside what nanoseconds since 1970 can hold).
- `binary_column.parquet`: a binary column, which is refused.
- `nested_column.parquet`: a list column, which is refused.
- `empty.parquet`: the columns of `orders.parquet` and no rows.
"""

from __future__ import annotations

import datetime as dt
import decimal
import pathlib

import pyarrow as pa
import pyarrow.parquet as pq

HERE = pathlib.Path(__file__).parent
D = decimal.Decimal


def write(name: str, table: pa.Table, **options) -> None:
    pq.write_table(table, HERE / name, **options)
    print(f"wrote {name}: {table.num_rows} rows, {table.num_columns} columns")


ORDERS_SCHEMA = pa.schema(
    [
        ("sku", pa.string()),
        ("name", pa.string()),
        ("qty", pa.int64()),
        ("price", pa.decimal128(10, 2)),
        ("received", pa.date32()),
    ]
)

orders = pa.table(
    {
        "sku": ["A-100", "A-200", "A-300"],
        "name": ["Bolt, hex M6", "Washer", "Flange DN50 – steel"],
        "qty": [10, 250, 4],
        "price": [D("1.50"), D("0.05"), D("12.00")],
        "received": [dt.date(2025, 9, 24), dt.date(2025, 9, 25), dt.date(2025, 10, 1)],
    },
    schema=ORDERS_SCHEMA,
)
write("orders.parquet", orders)

write("empty.parquet", ORDERS_SCHEMA.empty_table())

types = pa.table(
    {
        "id": pa.array([1, 9007199254740993, None], pa.int64()),
        "small": pa.array([-8, 127, None], pa.int8()),
        "count": pa.array([7, 18446744073709551615, None], pa.uint64()),
        "ratio": pa.array([0.5, 0.1 + 0.2, None], pa.float64()),
        "tiny": pa.array([1e-7, 1e21, None], pa.float64()),
        "weight": pa.array([0.1, 2.0, None], pa.float32()),
        "price": pa.array([D("12.50"), D("-0.05"), None], pa.decimal128(10, 2)),
        "ledger": pa.array([D("123456789012345.6789"), D("0.0001"), None], pa.decimal128(20, 4)),
        "day": pa.array([dt.date(2025, 9, 24), dt.date(1969, 12, 31), None], pa.date32()),
        "at_utc_us": pa.array(
            [
                dt.datetime(2025, 9, 24, 13, 30, 5, 123456, tzinfo=dt.timezone.utc),
                dt.datetime(2025, 9, 24, 0, 0, 0, tzinfo=dt.timezone.utc),
                None,
            ],
            pa.timestamp("us", tz="UTC"),
        ),
        "at_local_ms": pa.array(
            [dt.datetime(2025, 9, 24, 13, 30, 5, 120000), dt.datetime(2000, 2, 29, 23, 59, 59), None],
            pa.timestamp("ms"),
        ),
        # Nanoseconds are not expressible as a Python datetime, so they are
        # given as integers: 2025-09-24T13:30:05.000000001 and
        # 1969-12-31T23:59:59.5 (half a second before the epoch).
        "at_local_ns": pa.array([1758720605000000001, -500000000, None], pa.int64()).cast(pa.timestamp("ns")),
        "at_utc_ns": pa.array([1758720605000000001, 0, None], pa.int64()).cast(pa.timestamp("ns", tz="UTC")),
        "clock": pa.array([dt.time(13, 30, 0), dt.time(0, 0, 0, 500000), None], pa.time64("us")),
        "clock_ms": pa.array([dt.time(23, 59, 59, 999000), dt.time(0, 0, 0), None], pa.time32("ms")),
        "flag": pa.array([True, False, None], pa.bool_()),
        "note": pa.array(['plain', 'a, "quoted"\nline two', None], pa.string()),
    }
)
write("types.parquet", types)

# The legacy physical type that Spark and Impala used for timestamps. The
# second value is in the year 1: as nanoseconds since 1970 it would not fit
# in 64 bits, so a reader that goes through nanoseconds gets it wrong.
legacy = pa.table(
    {
        "id": pa.array([1, 2, 3], pa.int64()),
        "seen": pa.array(
            [dt.datetime(2025, 9, 24, 13, 30, 5, 123456), dt.datetime(1, 1, 1, 0, 0, 0), None],
            pa.timestamp("us"),
        ),
    }
)
write("legacy_int96.parquet", legacy, use_deprecated_int96_timestamps=True)

write(
    "binary_column.parquet",
    pa.table({"id": pa.array([1, 2], pa.int64()), "blob": pa.array([b"\x00\x01", b"\xff"], pa.binary())}),
)
write(
    "nested_column.parquet",
    pa.table({"id": pa.array([1, 2], pa.int64()), "tags": pa.array([["a", "b"], []], pa.list_(pa.string()))}),
)
