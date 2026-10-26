"""Regenerates the files of this directory (`DATA-17` slice 4). Every value is invented.

Run from anywhere with a Python that has `fastavro` (the Dagster venv does):

    python ops/fixtures/uploads/formats/make_fixtures.py

The output is deterministic (fixed gzip and zip timestamps, a fixed Avro sync marker), so
the files in git only change when this script does. The expected results are written by
hand beside it (`*.expected.json`): a script that wrote both would test itself.
"""

from __future__ import annotations

import datetime
import gzip
import io
import json
import zipfile
from pathlib import Path

import fastavro

HERE = Path(__file__).resolve().parent

OBJECTS = [
    {"id": 1, "name": "Anna", "price": 1.5, "active": True, "tags": ["a", "b"], "meta": {"x": 1, "y": "z"}, "note": None},
    {"id": 2, "name": "Budi", "price": 12, "active": False, "tags": [], "meta": {}, "extra": "only here"},
    {"id": 3, "name": "Citra é", "price": 0.25, "active": True, "tags": ["c"], "meta": {"x": 2}},
]

CSV_A = "id,name,price\n1,Anna,1.5\n2,Budi,12\n"
CSV_B = "price,id,name\n0.25,3,Citra\n"  # the same columns in another order
CSV_C = "id,name,price\n4,Dewi,7\n"


def main() -> None:
    (HERE / "orders.json").write_text(json.dumps(OBJECTS, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    (HERE / "orders.jsonl").write_text(
        "".join(json.dumps(o, ensure_ascii=False) + "\n" for o in OBJECTS) + "\n", encoding="utf-8"
    )  # a blank line at the end is part of the case

    schema = {
        "type": "record",
        "name": "Order",
        "fields": [
            {"name": "id", "type": "int"},
            {"name": "name", "type": "string"},
            {"name": "price", "type": "double"},
            {"name": "active", "type": "boolean"},
            {"name": "tags", "type": {"type": "array", "items": "string"}},
            {
                "name": "meta",
                "type": {
                    "type": "record",
                    "name": "Meta",
                    "fields": [{"name": "x", "type": "int"}, {"name": "y", "type": ["null", "string"]}],
                },
            },
            {"name": "note", "type": ["null", "string"]},
            {"name": "shipped", "type": {"type": "int", "logicalType": "date"}},
        ],
    }
    records = [
        {"id": 1, "name": "Anna", "price": 1.5, "active": True, "tags": ["a", "b"], "meta": {"x": 1, "y": "z"}, "note": None, "shipped": datetime.date(2026, 10, 16)},
        {"id": 2, "name": "Budi", "price": 12.0, "active": False, "tags": [], "meta": {"x": 0, "y": None}, "note": "late", "shipped": datetime.date(2026, 10, 17)},
    ]
    buffer = io.BytesIO()
    fastavro.writer(buffer, fastavro.parse_schema(schema), records, sync_marker=b"0123456789abcdef")
    (HERE / "orders.avro").write_bytes(buffer.getvalue())

    with (HERE / "orders.csv.gz").open("wb") as raw, gzip.GzipFile(fileobj=raw, mode="wb", mtime=0) as packed:
        packed.write((CSV_A + "3,Citra,0.25\n").encode())

    with zipfile.ZipFile(HERE / "three.zip", "w", zipfile.ZIP_DEFLATED) as archive:
        for name, text in (("a.csv", CSV_A), ("sub/", None), ("sub/b.csv", CSV_B), ("c.csv", CSV_C)):
            info = zipfile.ZipInfo(name, date_time=(2026, 10, 16, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            if text is None:
                info.external_attr = 0o40755 << 16
                archive.writestr(info, b"")
            else:
                archive.writestr(info, text.encode())


if __name__ == "__main__":
    main()
