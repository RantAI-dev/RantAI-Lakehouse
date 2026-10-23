"""JSON, JSON Lines, Avro, gzip and zip in the upload job (`DATA-17` slice 4, T4.2).

The files are the shared fixtures of `ops/fixtures/uploads/formats/` (the API's preview
reads the same ones). No network, no dlt pipeline: the sink is the harness's fake, so none
of these needs the `loader` mark.
"""

from __future__ import annotations

import gzip
import io
import json
import zipfile
from pathlib import Path

import fastavro
import pytest

from dispar_orchestrate import file_ingest, upload_formats
from dispar_orchestrate.file_ingest import (
    HEADER_NO_COLUMNS,
    LOAD_FAILED,
    NO_ROWS,
    TOO_LARGE_UNPACKED,
    TOO_MANY_COLUMNS,
    UNREADABLE,
)
from dispar_orchestrate.test_file_ingest import REPO_ROOT, _fails_with, _Harness, _params
from dispar_orchestrate.test_typed_load import spec

FORMATS_DIR = REPO_ROOT / "ops" / "fixtures" / "uploads" / "formats"

# fixture file, format, compression
CASES = [
    ("orders.json", "json", "none"),
    ("orders.jsonl", "jsonl", "none"),
    ("orders.avro", "avro", "none"),
    ("orders.csv.gz", "delimited", "gzip"),
    ("three.zip", "delimited", "zip"),
]


def _load(raw: bytes, file_format: str, compression: str, **overrides):
    harness = _Harness(raw=raw)
    harness.run(_params(format=file_format, compression=compression, **overrides))
    return harness, harness.loads[0]


@pytest.mark.parametrize(("name", "file_format", "compression"), CASES, ids=[c[0] for c in CASES])
def test_each_fixture_loads_to_the_rows_and_columns_its_expected_file_gives(name, file_format, compression):
    expected = json.loads((FORMATS_DIR / f"{name}.expected.json").read_text(encoding="utf-8"))
    _, loaded = _load((FORMATS_DIR / name).read_bytes(), file_format, compression)
    assert list(loaded["columns"]) == expected["columns"]
    assert loaded["column_types"] == {column: "string" for column in expected["columns"]}
    got = [[row[column] for column in expected["columns"]] for row in loaded["rows"]]
    assert got == expected["rows"]


def test_a_json_number_keeps_the_digits_the_source_wrote_and_a_nested_value_keeps_its_key_order():
    raw = b'[{"b": 1.50, "a": {"z": 1, "y": [1.0, {"k": null}]}, "big": 12345678901234567890.123456789}]'
    _, loaded = _load(raw, "json", "none")
    [row] = loaded["rows"]
    assert row["b"] == "1.50"
    assert row["a"] == '{"z":1,"y":[1.0,{"k":null}]}'
    assert row["big"] == "12345678901234567890.123456789"


def test_the_columns_of_a_json_file_are_the_keys_of_every_object_even_one_far_down():
    objects = [{"a": i} for i in range(5)] + [{"a": 9, "late_key": "x"}]
    _, loaded = _load(json.dumps(objects).encode(), "json", "none")
    assert list(loaded["columns"]) == ["a", "late_key"]
    assert [row["late_key"] for row in loaded["rows"]] == [None] * 5 + ["x"]


def test_a_key_that_is_not_a_clean_name_is_cleaned_as_a_header_cell_is():
    _, loaded = _load(b'[{"Order ID": 1, "order-id": 2}]', "json", "none")
    assert list(loaded["columns"]) == ["order_id", "order_id_2"]


def test_a_json_file_with_a_byte_order_mark_loads():
    _, loaded = _load(b"\xef\xbb\xbf" + b'[{"a": 1}]', "json", "none")
    assert loaded["rows"] == [{"a": "1"}]
    _, lines = _load(b"\xef\xbb\xbf" + b'{"a": 1}\r\n{"a": 2}\r\n', "jsonl", "none")
    assert lines["rows"] == [{"a": "1"}, {"a": "2"}]


def test_values_of_a_json_file_are_typed_by_the_column_list_as_those_of_any_text():
    columns = (spec("id", "integer"), spec("price", "decimal"), spec("active", "boolean"))
    raw = b'[{"id": 1, "price": 1.50, "active": true}, {"id": 2, "price": 12, "active": false}]'
    _, loaded = _load(raw, "json", "none", columns=columns, on_bad_value="stop")
    assert [row["id"] for row in loaded["rows"]] == [1, 2]
    assert [str(row["price"]) for row in loaded["rows"]] == ["1.500000000", "12.000000000"]
    assert [row["active"] for row in loaded["rows"]] == [True, False]
    assert loaded["column_types"] == {"id": "long", "price": "decimal(38, 9)", "active": "boolean"}


@pytest.mark.parametrize(
    ("raw", "reason"),
    [
        (b'{"a": 1}', UNREADABLE),  # an object, not an array
        (b"[1, 2]", UNREADABLE),  # not objects
        (b'[{"a": 1}, ', UNREADABLE),  # cut short
        (b'[{"a": 1}] trailing', UNREADABLE),
        (b"not json", UNREADABLE),
        (b"[]", NO_ROWS),
        (b"[{}]", HEADER_NO_COLUMNS),
    ],
)
def test_a_json_file_that_is_not_an_array_of_objects_fails_before_anything_is_written(raw, reason):
    harness = _Harness(raw=raw)
    _fails_with(harness, reason, _params(format="json"))
    assert harness.loads == []


def test_a_json_lines_file_with_a_line_that_is_not_an_object_or_not_json_fails_before_anything_is_written():
    for raw in (b'{"a": 1}\n[1]\n', b'{"a": 1}\nnope\n'):
        harness = _Harness(raw=raw)
        _fails_with(harness, UNREADABLE, _params(format="jsonl"))
        assert harness.loads == []
    _fails_with(_Harness(raw=b"\n\n"), NO_ROWS, _params(format="jsonl"))


def test_more_than_a_thousand_keys_fail_as_too_many_columns():
    raw = json.dumps([{f"k{i}": i for i in range(1001)}]).encode()
    _fails_with(_Harness(raw=raw), TOO_MANY_COLUMNS, _params(format="json"))


# ── Avro ─────────────────────────────────────────────────────────────────


def _avro(schema: dict, records: list[dict]) -> bytes:
    buffer = io.BytesIO()
    fastavro.writer(buffer, fastavro.parse_schema(schema), records)
    return buffer.getvalue()


def test_an_avro_union_with_null_is_a_column_that_may_be_empty_and_a_decimal_is_exact():
    import decimal

    schema = {
        "type": "record",
        "name": "R",
        "fields": [
            {"name": "n", "type": ["null", "long"]},
            {"name": "d", "type": {"type": "bytes", "logicalType": "decimal", "precision": 10, "scale": 2}},
            {"name": "m", "type": {"type": "map", "values": "int"}},
            {"name": "ts", "type": {"type": "long", "logicalType": "timestamp-millis"}},
        ],
    }
    import datetime

    raw = _avro(
        schema,
        [
            {"n": None, "d": decimal.Decimal("1.50"), "m": {"b": 2, "a": 1}, "ts": datetime.datetime(2026, 10, 16, 1, 2, 3, tzinfo=datetime.timezone.utc)},
            {"n": 7, "d": decimal.Decimal("-0.05"), "m": {}, "ts": datetime.datetime(2026, 10, 17, tzinfo=datetime.timezone.utc)},
        ],
    )
    _, loaded = _load(raw, "avro", "none")
    assert [row["n"] for row in loaded["rows"]] == [None, "7"]
    assert [row["d"] for row in loaded["rows"]] == ["1.50", "-0.05"]
    assert [row["m"] for row in loaded["rows"]] == ['{"b":2,"a":1}', "{}"]
    assert loaded["rows"][0]["ts"] == "2026-10-16T01:02:03+00:00"


def test_an_avro_file_with_a_binary_column_or_a_scalar_schema_or_other_bytes_is_unreadable():
    binary = _avro(
        {"type": "record", "name": "R", "fields": [{"name": "b", "type": "bytes"}]}, [{"b": b"\x00"}]
    )
    nested = _avro(
        {"type": "record", "name": "R", "fields": [{"name": "a", "type": {"type": "array", "items": "bytes"}}]},
        [{"a": [b"x"]}],
    )
    scalar = _avro({"type": "long"}, [1, 2])
    for raw in (binary, nested, scalar, b"Obj\x01 not really", b"hello"):
        harness = _Harness(raw=raw)
        _fails_with(harness, UNREADABLE, _params(format="avro"))
        assert harness.loads == []


def test_an_avro_file_with_a_codec_other_than_null_or_deflate_is_unreadable_and_deflate_loads():
    schema = fastavro.parse_schema({"type": "record", "name": "R", "fields": [{"name": "a", "type": "int"}]})
    for codec in ("bzip2", "xz"):
        buffer = io.BytesIO()
        fastavro.writer(buffer, schema, [{"a": 1}], codec=codec)
        _fails_with(_Harness(raw=buffer.getvalue()), UNREADABLE, _params(format="avro"))
    buffer = io.BytesIO()
    fastavro.writer(buffer, schema, [{"a": 1}], codec="deflate")
    _, loaded = _load(buffer.getvalue(), "avro", "none")
    assert loaded["rows"] == [{"a": "1"}]


def test_an_avro_file_with_no_records_has_no_rows():
    raw = _avro({"type": "record", "name": "R", "fields": [{"name": "a", "type": "int"}]}, [])
    _fails_with(_Harness(raw=raw), NO_ROWS, _params(format="avro"))


# ── gzip and zip ─────────────────────────────────────────────────────────


def _zip(entries: list[tuple[str, bytes]]) -> bytes:
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, data in entries:
            archive.writestr(name, data)
    return buffer.getvalue()


def test_a_gzip_file_holds_text_json_or_json_lines_and_reads_as_the_plain_file_does():
    csv_text = b"id,name\n1,a\n2,b\n"
    _, plain = _load(csv_text, "delimited", "none")
    _, packed = _load(gzip.compress(csv_text), "delimited", "gzip")
    assert packed["rows"] == plain["rows"] and list(packed["columns"]) == list(plain["columns"])
    _, as_json = _load(gzip.compress(b'[{"a": 1}]'), "json", "gzip")
    assert as_json["rows"] == [{"a": "1"}]
    _, as_lines = _load(gzip.compress(b'{"a": 1}\n{"a": 2}\n'), "jsonl", "gzip")
    assert as_lines["rows"] == [{"a": "1"}, {"a": "2"}]


def test_a_gzip_bomb_fails_and_is_never_truncated_whether_it_is_text_or_json():
    # 60,000 identical lines compress to a few hundred bytes; the limit is 100,000 bytes.
    bomb_csv = gzip.compress(b"id,name\n" + b"1,abcdefgh\n" * 60_000)
    assert len(bomb_csv) < 2_000
    harness = _Harness(raw=bomb_csv)
    _fails_with(harness, TOO_LARGE_UNPACKED, _params(format="delimited", compression="gzip", max_bytes=100_000))
    assert harness.loads == [] or harness.recorded[-1]["status"] == "failed"
    assert harness.registered == [], "a failed load registers nothing"
    assert harness.recorded[-1]["error"] == TOO_LARGE_UNPACKED

    bomb_json = gzip.compress(b"[" + b'{"a": 1},' * 60_000 + b'{"a": 1}]')
    json_harness = _Harness(raw=bomb_json)
    _fails_with(json_harness, TOO_LARGE_UNPACKED, _params(format="json", compression="gzip", max_bytes=100_000))
    assert json_harness.loads == [], "found in the first pass, before anything is written"


def test_a_load_just_under_the_limit_is_not_refused_and_one_byte_over_is():
    body = b"a\n" + b"x\n" * 49  # 100 bytes
    assert len(body) == 100
    _load(gzip.compress(body), "delimited", "gzip", max_bytes=100)
    _fails_with(_Harness(raw=gzip.compress(body)), TOO_LARGE_UNPACKED, _params(format="delimited", compression="gzip", max_bytes=99))


def test_a_zip_bomb_is_refused_by_its_declared_size_and_by_the_bytes_it_really_decodes():
    big = b"id\n" + b"1\n" * 80_000
    archive = _zip([("a.csv", big)])
    _fails_with(_Harness(raw=archive), TOO_LARGE_UNPACKED, _params(format="delimited", compression="zip", max_bytes=100_000))
    # Two entries that are each under the limit and over it together.
    pair = _zip([("a.csv", big[:60_000]), ("b.csv", big[:60_000])])
    harness = _Harness(raw=pair)
    _fails_with(harness, TOO_LARGE_UNPACKED, _params(format="delimited", compression="zip", max_bytes=100_000))
    assert harness.registered == []


def test_a_zip_holds_up_to_ten_files_of_one_format_and_folders_are_ignored():
    _, ten = _load(_zip([(f"{i}.csv", b"a\n1\n") for i in range(10)]), "delimited", "zip")
    assert len(ten["rows"]) == 10
    _fails_with(_Harness(raw=_zip([(f"{i}.csv", b"a\n1\n") for i in range(11)])), UNREADABLE, _params(format="delimited", compression="zip"))
    mixed = _zip([("a.csv", b"a\n1\n"), ("b.json", b'[{"a": 1}]')])
    _fails_with(_Harness(raw=mixed), UNREADABLE, _params(format="delimited", compression="zip"))
    unknown = _zip([("a.csv", b"a\n1\n"), ("b.xlsx", b"PK")])
    _fails_with(_Harness(raw=unknown), UNREADABLE, _params(format="delimited", compression="zip"))
    _, with_folder = _load(_zip([("d/", b""), ("d/a.csv", b"a\n1\n"), ("__MACOSX/._a.csv", b"\x00")]), "delimited", "zip")
    assert with_folder["rows"] == [{"a": "1"}]


def test_an_archive_inside_an_archive_is_refused_by_its_name_and_by_its_bytes():
    inner = _zip([("a.csv", b"a\n1\n")])
    for name in ("inner.zip", "disguised.csv"):
        _fails_with(_Harness(raw=_zip([(name, inner)])), UNREADABLE, _params(format="delimited", compression="zip"))
    _fails_with(_Harness(raw=_zip([("x.csv.gz", gzip.compress(b"a\n1\n"))])), UNREADABLE, _params(format="delimited", compression="zip"))
    _fails_with(_Harness(raw=gzip.compress(gzip.compress(b"a\n1\n"))), UNREADABLE, _params(format="delimited", compression="gzip"))
    _fails_with(_Harness(raw=gzip.compress(inner)), UNREADABLE, _params(format="delimited", compression="gzip"))


def test_the_files_of_a_zip_are_matched_by_column_name_and_a_different_set_fails_the_load():
    ok = _zip([("a.csv", b"id,name\n1,a\n"), ("b.csv", b"name,id\nb,2\n")])
    _, loaded = _load(ok, "delimited", "zip")
    assert loaded["rows"] == [{"id": "1", "name": "a"}, {"id": "2", "name": "b"}]
    differs = _zip([("a.csv", b"id,name\n1,a\n"), ("b.csv", b"id,other\n2,b\n")])
    harness = _Harness(raw=differs)
    _fails_with(harness, LOAD_FAILED, _params(format="delimited", compression="zip"))


def test_a_zip_with_no_files_has_no_rows_and_a_zip_that_is_not_a_zip_is_unreadable():
    _fails_with(_Harness(raw=_zip([("d/", b"")])), UNREADABLE, _params(format="delimited", compression="zip"))
    _fails_with(_Harness(raw=b"not a zip at all"), UNREADABLE, _params(format="delimited", compression="zip"))
    _fails_with(_Harness(raw=b"not gzip"), UNREADABLE, _params(format="delimited", compression="gzip"))


def test_a_zip_of_avro_or_json_files_is_read_entry_by_entry():
    schema = {"type": "record", "name": "R", "fields": [{"name": "a", "type": "int"}]}
    archive = _zip([("1.avro", _avro(schema, [{"a": 1}])), ("2.avro", _avro(schema, [{"a": 2}]))])
    _, loaded = _load(archive, "avro", "zip")
    assert loaded["rows"] == [{"a": "1"}, {"a": "2"}]
    jsons = _zip([("1.json", b'[{"a": 1}]'), ("2.jsonl", b'{"a": 2}\n')])
    _fails_with(_Harness(raw=jsons), UNREADABLE, _params(format="json", compression="zip"))


def test_the_settings_a_gzip_parquet_or_unknown_compression_would_need_are_refused():
    for overrides in (
        {"format": "parquet", "compression": "gzip"},
        {"format": "avro", "compression": "gzip"},
        {"format": "parquet", "compression": "zip"},
        {"format": "delimited", "compression": "rar"},
        {"format": "delimited", "max_bytes": 0},
        {"format": "xml"},
    ):
        _fails_with(_Harness(), LOAD_FAILED, _params(**overrides))


def test_the_budget_counts_each_decoded_byte_once_and_a_new_pass_spends_a_new_budget():
    budget = upload_formats.Budget(10)
    budget.spend(10)
    with pytest.raises(upload_formats.Unpacked):
        budget.spend(1)
