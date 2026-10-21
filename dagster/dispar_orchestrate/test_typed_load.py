"""Column types in the load (`DATA-18` T3.3): the job casts the file to the column
list, counts what does not fit, stops or loads it as empty, keeps a report, and the
types reach Iceberg as declared.

Sections 8 items 5 and 6 of the Phase 3 plan are answered here against a LOCAL
Iceberg table (dlt's own catalog on a filesystem destination), not Lakekeeper:
the real catalog is exercised by `ops/g9/upload_test.py` in CI.
"""

from __future__ import annotations

import glob
import json
import os

import dlt
import pyarrow as pa
import pytest

from dispar_orchestrate.adapters import sink as sink_module
from dispar_orchestrate.adapters.sink import SinkResult, _stamp_ingested_at
from dispar_orchestrate.file_ingest import BAD_VALUES, LOAD_FAILED, LoadFailure
from dispar_orchestrate.test_file_ingest import KEY, _Harness, _fails_with, _params
from dispar_orchestrate.upload_types import ColumnSpec


def spec(name: str, kind: str, date_format: str = "", timezone: bool = False, source: str | None = None) -> ColumnSpec:
    return ColumnSpec(source or name, name, kind, date_format, timezone)


CSV = b"Qty,Due,Note,Price\n1,2026-10-16,a,1.50\nabc,2026-13-01,b,2\n,2026-10-18,,x\n4,2026-10-19,d,0.25\n"
COLUMNS = (
    spec("qty", "integer", source="Qty"),
    spec("due_on", "date", "iso", source="Due"),
    spec("note", "text", source="Note"),
    spec("price", "decimal", source="Price"),
)


def test_a_typed_load_hands_the_sink_cast_batches_under_the_chosen_names_and_the_types_it_was_told():
    harness = _Harness(raw=CSV)
    harness.run(_params(columns=COLUMNS, on_bad_value="null"))
    [loaded] = harness.loads
    assert list(loaded["columns"]) == ["qty", "due_on", "note", "price"]
    assert loaded["column_types"] == {"qty": "long", "due_on": "date", "note": "string", "price": "decimal(38, 9)"}
    hints = loaded["columns"]
    assert hints["qty"]["data_type"] == "bigint"
    assert (hints["price"]["precision"], hints["price"]["scale"]) == (38, 9)
    assert [row["qty"] for row in loaded["rows"]] == [1, None, None, 4]
    assert [row["note"] for row in loaded["rows"]] == ["a", "b", "", "d"], "text keeps an empty text"
    assert str(loaded["rows"][0]["price"]) == "1.500000000"
    assert loaded["rows"][0]["due_on"].isoformat() == "2026-10-16"


def test_told_to_load_them_as_empty_it_loads_and_keeps_the_report_of_the_values_that_did_not_fit():
    harness = _Harness(raw=CSV)
    summary = harness.run(_params(columns=COLUMNS, on_bad_value="null"))
    assert summary.bad_values == 3  # "abc", "2026-13-01" and "x"; the empty cells are null, not bad
    [(key, payload)] = harness.reports
    assert key == KEY
    report = json.loads(payload)
    assert report["count"] == summary.bad_values
    assert {c["column"]: c["count"] for c in report["byColumn"]} == {"qty": 1, "due_on": 1, "price": 1}
    assert report["rows"][0] == {"row": 2, "column": "qty", "value": "abc"}
    assert [row["status"] for row in harness.recorded] == ["succeeded"]


def test_told_to_stop_it_reads_the_whole_file_writes_nothing_and_fails_with_the_report():
    harness = _Harness(raw=CSV)
    failure = _fails_with(harness, BAD_VALUES, _params(columns=COLUMNS, on_bad_value="stop"))
    assert failure.rows is None
    assert harness.registered == []
    assert [row["error"] for row in harness.recorded] == [BAD_VALUES]
    [(key, payload)] = harness.reports
    assert json.loads(payload)["count"] == 3, "the count is over the whole file, not up to the first failure"


def test_a_clean_typed_load_removes_an_earlier_report_and_an_untyped_load_touches_none():
    clean = _Harness(raw=b"Qty\n1\n2\n")
    clean.run(_params(columns=(spec("qty", "integer", source="Qty"),), on_bad_value="stop"))
    assert clean.reports == [(KEY, None)], "None removes the stale object"
    untyped = _Harness(raw=CSV)
    untyped.run(_params())
    assert untyped.reports == []
    assert untyped.loads[0]["column_types"] == {"qty": "string", "due": "string", "note": "string", "price": "string"}


def test_the_report_that_cannot_be_written_never_changes_how_the_load_ended():
    harness = _Harness(raw=CSV)

    def broken(config, key, payload):
        raise OSError("bucket gone")

    harness.write_report = broken
    summary = harness.run(_params(columns=COLUMNS, on_bad_value="null"))
    assert summary.bad_values == 3
    assert any("could not be kept" in line for line in harness.log.lines)


def test_a_column_list_that_does_not_match_the_file_or_the_naming_rule_fails_before_anything_is_written():
    harness = _Harness(raw=CSV)
    failure = _fails_with(harness, LOAD_FAILED, _params(columns=COLUMNS[:2], on_bad_value="null"))
    assert "2 columns and the file has 4" in str(failure.__cause__)
    assert harness.loads == [] or harness.loads[0]["rows"] == []
    bad_name = (spec("Qty", "integer"),) + COLUMNS[1:]
    again = _Harness(raw=CSV)
    _fails_with(again, LOAD_FAILED, _params(columns=bad_name, on_bad_value="null"))
    assert again.reads == [], "refused before the object was opened"
    _fails_with(_Harness(raw=CSV), LOAD_FAILED, _params(columns=COLUMNS, on_bad_value="ignore"))


def test_a_parquet_file_is_cast_from_the_text_the_api_converter_wrote():
    from pathlib import Path

    raw = (Path(__file__).resolve().parents[2] / "ops" / "fixtures" / "parquet" / "orders.parquet").read_bytes()
    columns = (
        spec("sku", "text"),
        spec("name", "text"),
        spec("qty", "integer"),
        spec("price", "decimal"),
        spec("received", "date", "iso"),
    )
    harness = _Harness(raw=raw)
    harness.run(_params(format="parquet", bronze_table_name="orders_raw", columns=columns, on_bad_value="stop"))
    first = harness.loads[0]["rows"][0]
    assert (first["qty"], str(first["price"]), first["received"].isoformat()) == (10, "1.500000000", "2025-09-24")
    assert harness.reports == [(KEY, None)]


# ── Section 8, items 5 and 6, against a local Iceberg table ──────────────


def _local_pipeline(directory: str):
    return dlt.pipeline(
        "typed",
        destination=dlt.destinations.filesystem(bucket_url=directory),
        dataset_name="ds",
        pipelines_dir=os.path.join(directory, "_p"),
    )


def _source(table: pa.Table, hints: list[dict]):
    resource = dlt.resource([table], name="t")
    resource.apply_hints(columns=hints)
    resource.add_map(_stamp_ingested_at)
    return resource


def _iceberg_fields(directory: str) -> list[tuple[str, str]]:
    latest = sorted(glob.glob(directory + "/**/*.metadata.json", recursive=True))[-1]
    meta = json.load(open(latest))
    current = next(s for s in meta["schemas"] if s["schema-id"] == meta["current-schema-id"])
    return [(f["name"], f["type"]) for f in current["fields"]]


def test_v6_the_six_types_reach_iceberg_as_declared(tmp_path):
    specs = [
        spec("i", "integer"),
        spec("d", "decimal"),
        spec("b", "boolean"),
        spec("day", "date", "iso"),
        spec("ts", "timestamp", "iso", False),
        spec("tsz", "timestamp", "iso", True),
        spec("s", "text"),
        spec("f", "float"),
    ]
    values = {
        "i": "1", "d": "1234567890123456789012345678.123456789", "b": "yes", "day": "2026-10-16",
        "ts": "2026-10-16T01:02:03.456789", "tsz": "2026-10-16T01:02:03.456789Z", "s": "x", "f": "1.5",
    }
    text = pa.table({name: pa.array([value], pa.string()) for name, value in values.items()})
    from dispar_orchestrate.upload_types import BadReport, cast_batch

    report = BadReport([s.name for s in specs])
    typed = cast_batch(text, specs, 1, report)
    assert report.count == 0
    pipeline = _local_pipeline(str(tmp_path))
    pipeline.run(
        _source(typed, [s.hint() for s in specs]),
        table_name="t",
        write_disposition="replace",
        loader_file_format="parquet",
        table_format="iceberg",
    )
    fields = dict(_iceberg_fields(str(tmp_path)))
    assert {name: fields[name] for name in values} == {
        "i": "long",
        "d": "decimal(38, 9)",
        "b": "boolean",
        "day": "date",
        "ts": "timestamp",
        "tsz": "timestamptz",
        "s": "string",
        "f": "double",
    }
    assert {s.name: s.iceberg_type() for s in specs} == {n: fields[n] for n in values}


def test_v5_a_replace_that_changes_a_columns_type_makes_no_variant_column_and_is_refused_by_iceberg_unless_dropped(tmp_path):
    directory = str(tmp_path)
    first = pa.table({"x": pa.array(["1", "2"], pa.string())})
    _local_pipeline(directory).run(
        _source(first, [{"name": "x", "data_type": "text"}]),
        table_name="t", write_disposition="replace", loader_file_format="parquet", table_format="iceberg",
    )
    typed = pa.table({"x": pa.array([1, 2, 3], pa.int64())})
    hints = [{"name": "x", "data_type": "bigint"}]

    # Without the drop: no `x__v_bigint`, but pyiceberg refuses the write.
    plain = _local_pipeline(directory)
    with pytest.raises(Exception) as refused:
        plain.run(
            _source(typed, hints),
            table_name="t", write_disposition="replace", loader_file_format="parquet", table_format="iceberg",
        )
    assert "Cannot change column type" in str(refused.value.__cause__ or refused.value) or "Cannot change column type" in repr(refused.value)
    assert not any("__v_" in name for name, _ in _iceberg_fields(directory))
    plain.drop_pending_packages()

    # With it, as `load_via_sink` does when told the types: dropped, then created with the new type.
    pipeline = _local_pipeline(directory)
    pipeline.extract(_source(typed, hints), table_name="t", table_format="iceberg", write_disposition="replace")
    pipeline.normalize()
    assert sink_module._drop_table_if_types_differ(pipeline, "t", {"x": "long"}) is True
    pipeline.load()
    assert dict(_iceberg_fields(directory))["x"] == "long"
    table = sink_module.history.open_table(pipeline, "t", sync=False)
    assert sorted(table.scan().to_arrow().column("x").to_pylist()) == [1, 2, 3]
    # The same types: nothing is dropped.
    assert sink_module._drop_table_if_types_differ(pipeline, "t", {"x": "long"}) is False
    # A column the load does not write is not a difference.
    assert sink_module._drop_table_if_types_differ(pipeline, "t", {"y": "string"}) is False
