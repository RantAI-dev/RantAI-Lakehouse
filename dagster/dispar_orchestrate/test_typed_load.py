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
from dispar_orchestrate.file_ingest import BAD_VALUES, COLUMNS_NOT_PREVIEWED, LOAD_FAILED, LoadFailure
from dispar_orchestrate.test_file_ingest import KEY, TENANT, _Harness, _fails_with, _params
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

# A workbook's load reads the converted sheet the API wrote, beside the
# original (`DATA-31`): the report belongs beside the original, where the API
# reads it.
ORIGINAL_KEY = f"uploads/{TENANT}/u-1.xlsx"
CONVERTED_KEY = f"{ORIGINAL_KEY}.converted.0.csv"
# The converted text of an upload made before `SEC-22`, without a position.
LEGACY_CONVERTED_KEY = f"{ORIGINAL_KEY}.converted.csv"


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


def test_a_typed_load_of_a_converted_sheet_keeps_its_report_beside_the_original():
    # `DATA-31`: the job reads the converted sheet the API wrote, but the API
    # reads the report beside the ORIGINAL.
    harness = _Harness(raw=CSV)
    harness.run(_params(storage_keys=(CONVERTED_KEY,), columns=COLUMNS, on_bad_value="null"))
    [(key, payload)] = harness.reports
    assert key == ORIGINAL_KEY, "the report belongs beside the original, where the API reads it"
    assert json.loads(payload)["count"] == 3


def test_a_typed_load_of_a_legacy_converted_sheet_keeps_its_report_beside_the_original():
    # The converted text of an upload made before `SEC-22` has no position.
    harness = _Harness(raw=CSV)
    harness.run(_params(storage_keys=(LEGACY_CONVERTED_KEY,), columns=COLUMNS, on_bad_value="null"))
    [(key, payload)] = harness.reports
    assert key == ORIGINAL_KEY
    assert json.loads(payload)["count"] == 3


def test_a_clean_typed_load_of_a_converted_sheet_removes_the_report_beside_the_original():
    clean = _Harness(raw=b"Qty\n1\n2\n")
    clean.run(
        _params(
            storage_keys=(CONVERTED_KEY,),
            columns=(spec("qty", "integer", source="Qty"),),
            on_bad_value="stop",
        )
    )
    assert clean.reports == [(ORIGINAL_KEY, None)], "None removes the stale object"


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
    # PR #186 review SHOULD-FIX 1: a column list of another length has a reason of its own
    # (it was the generic LOAD_FAILED); the naming-rule and settings failures below keep LOAD_FAILED.
    failure = _fails_with(harness, COLUMNS_NOT_PREVIEWED, _params(columns=COLUMNS[:2], on_bad_value="null"))
    assert "2 columns and the file has 4" in str(failure.__cause__)
    assert harness.loads == [] or harness.loads[0]["rows"] == []
    bad_name = (spec("Qty", "integer"),) + COLUMNS[1:]
    again = _Harness(raw=CSV)
    _fails_with(again, LOAD_FAILED, _params(columns=bad_name, on_bad_value="null"))
    assert again.reads == [], "refused before the object was opened"
    _fails_with(_Harness(raw=CSV), LOAD_FAILED, _params(columns=COLUMNS, on_bad_value="ignore"))


def test_pr_186_review_should_fix_1_a_json_key_the_preview_did_not_show_fails_with_the_reason_that_says_so():
    # The preview samples the first rows, so `late` (first seen in row 3) is not in its column list;
    # the console sends that list and the job, which reads every row, finds one column more.
    raw = b'[{"a": "1"}, {"a": "2"}, {"a": "3", "late": "x"}]'
    previewed = (spec("a", "text"),)
    harness = _Harness(raw=raw)
    failure = _fails_with(harness, COLUMNS_NOT_PREVIEWED, _params(format="json", columns=previewed, on_bad_value="null"))
    assert "1 columns and the file has 2" in str(failure.__cause__)
    assert harness.registered == []
    assert [row["error"] for row in harness.recorded] == [COLUMNS_NOT_PREVIEWED]
    # With the full list the same file loads.
    full = (spec("a", "text"), spec("late", "text"))
    assert _Harness(raw=raw).run(_params(format="json", columns=full, on_bad_value="null")).parsed_rows == 3


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


# Phase 3 slice 3 review BLOCKER 2: the tests below run a real dlt pipeline, so they are
# `loader` tests (CI's unit job runs `-m "not loader"`), and each one starts without the
# `ICEBERG_CATALOG__*` variables that `sink._install_catalog_env` sets in the process (and
# that a CI job may carry): left set, dlt would send the table to a REST catalog
# (`lakekeeper:8181` in CI) instead of the local one the pipeline below names.
loader = pytest.mark.loader
CATALOG_ENV = (
    "ICEBERG_CATALOG__ICEBERG_CATALOG_NAME",
    "ICEBERG_CATALOG__ICEBERG_CATALOG_TYPE",
    "ICEBERG_CATALOG__ICEBERG_CATALOG_CONFIG",
)


@pytest.fixture
def local_catalog(monkeypatch, tmp_path):
    for name in CATALOG_ENV:
        monkeypatch.delenv(name, raising=False)
    monkeypatch.setenv("DLT_DATA_DIR", str(tmp_path / "dlt"))  # dlt's own state, one directory per test


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


@loader
def test_v6_the_six_types_reach_iceberg_as_declared(local_catalog, tmp_path):
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


def _required_flags(directory: str) -> dict[str, bool]:
    latest = sorted(glob.glob(directory + "/**/*.metadata.json", recursive=True))[-1]
    meta = json.load(open(latest))
    current = next(s for s in meta["schemas"] if s["schema-id"] == meta["current-schema-id"])
    return {f["name"]: f["required"] for f in current["fields"]}


@loader
@pytest.mark.parametrize("name,kind", [("orders.json", "json"), ("orders.jsonl", "jsonl"), ("orders.avro", "avro")])
def test_pr_186_review_blocker_4_a_missing_key_is_an_optional_field_and_a_null_in_the_iceberg_table(
    local_catalog, tmp_path, name, kind
):
    # The job's own resource (`_text_resource`, every column declared `text`) through dlt into a
    # local Iceberg table: the level between the Arrow batch and the catalog, where a null could
    # become "" or a field could become `required`. Measured with dlt 1.30.0 and pyiceberg 0.12.0:
    # neither does. (The Rust API's `/api/query/run` renders a NULL as "" -- `stringify_cell` --
    # which is what turned the first run of the gate red; the gate now asks `isNull`.)
    from pathlib import Path

    from dispar_orchestrate import file_ingest

    expected = json.loads(
        (Path(__file__).resolve().parents[2] / "ops" / "fixtures" / "uploads" / "formats" / f"{name}.expected.json").read_text(
            encoding="utf-8"
        )
    )
    params = _params(format=kind, compression="none", bronze_table_name="t")
    directory = str(tmp_path)
    with (Path(__file__).resolve().parents[2] / "ops" / "fixtures" / "uploads" / "formats" / name).open("rb") as handle:
        stream = file_ingest._open_stream(handle, params)
        resource = file_ingest._text_resource(stream, "t")
        resource.add_map(_stamp_ingested_at)
        pipeline = _local_pipeline(directory)
        pipeline.run(resource, table_name="t", write_disposition="replace", loader_file_format="parquet", table_format="iceberg")
    assert not any(_required_flags(directory)[column] for column in stream.columns), "every text column may be empty"
    table = sink_module.history.open_table(pipeline, "t", sync=False).scan().to_arrow()
    rows = sorted(
        ([row[column] for column in stream.columns] for row in table.to_pylist()), key=lambda row: str(row[0])
    )
    assert rows == sorted(expected["rows"], key=lambda row: str(row[0])), "a null stays a null, an empty text stays empty"
    assert any(None in row for row in rows)


def _text_table(directory: str):
    """A local Iceberg table of three text columns and two rows, loaded by dlt."""
    first = pa.table({
        "a": pa.array(["1", "2"], pa.string()),
        "b": pa.array(["x", "y"], pa.string()),
        "c": pa.array(["1.5", "2.5"], pa.string()),
    })
    pipeline = _local_pipeline(directory)
    pipeline.run(
        _source(first, [{"name": n, "data_type": "text"} for n in "abc"]),
        table_name="t", write_disposition="replace", loader_file_format="parquet", table_format="iceberg",
    )
    return pipeline, sink_module.history.open_table(pipeline, "t", sync=False)


def _state(table) -> tuple:
    table.refresh()
    return (
        [(f.name, str(f.field_type)) for f in table.schema().fields],
        sorted(table.scan().to_arrow().column("a").to_pylist()),
        table.current_snapshot().snapshot_id,
        [s.snapshot_id for s in table.snapshots()],
    )


@loader
def test_v5_a_replace_that_changes_a_columns_type_makes_no_variant_column_and_dlt_alone_is_refused_by_iceberg(local_catalog, tmp_path):
    directory = str(tmp_path)
    _text_table(directory)
    typed = pa.table({"a": pa.array([1, 2, 3], pa.int64())})
    plain = _local_pipeline(directory)
    with pytest.raises(Exception) as refused:
        plain.run(
            _source(typed, [{"name": "a", "data_type": "bigint"}]),
            table_name="t", write_disposition="replace", loader_file_format="parquet", table_format="iceberg",
        )
    assert "Cannot change column type" in str(refused.value.__cause__ or refused.value) or "Cannot change column type" in repr(refused.value)
    assert not any("__v_" in name for name, _ in _iceberg_fields(directory))
    plain.drop_pending_packages()


@loader
def test_phase_3_slice_3_review_blocker_1_a_type_changing_replace_is_one_commit_that_keeps_the_history(local_catalog, tmp_path):
    _, table = _text_table(str(tmp_path))
    schema_before, _, snapshot_before, snapshots_before = _state(table)
    typed = pa.table({
        "a": pa.array([7, 8, 9], pa.int64()),
        "b": pa.array(["p", "q", "r"], pa.string()),
        "c": pa.array([1, 2, 3], pa.float64()),
    })
    source = _source(typed, [])
    changed = sink_module._changed_columns(table, {"a": "long", "b": "string", "c": "double"})
    assert changed == ["a", "c"], "b has the type it is told: not a difference"
    written = sink_module._replace_changing_types(table, source, changed, {"a": "long", "b": "string", "c": "double"})
    assert written == 3
    schema, rows, snapshot, snapshots = _state(table)
    assert [n for n, _ in schema] == [n for n, _ in schema_before], "the column order is kept"
    assert dict(schema)["a"] == "long" and dict(schema)["c"] == "double" and dict(schema)["b"] == "string"
    assert rows == [7, 8, 9]
    assert set(snapshots_before) <= set(snapshots), "the table's history is kept"
    assert snapshot != snapshot_before
    full = table.scan().to_arrow()
    assert sorted(full.column("c").to_pylist()) == [1.0, 2.0, 3.0]
    for bookkeeping in ("_dlt_id", "_dlt_load_id"):  # filled in when the table has them (Arrow loads may not create them)
        if bookkeeping in full.schema.names:
            assert all(full.column(bookkeeping).to_pylist())
    assert full.column("_ingested_at").null_count == 0
    # `DATA-36`: ONE snapshot, not a delete and an append per group. Every snapshot of a
    # table is a state a reader can pick (the time-travel picker).
    assert len(snapshots) - len(snapshots_before) == 1, "one snapshot: the load leaves one state behind"


@loader
def test_phase_3_slice_3_review_blocker_1_a_failure_inside_the_transaction_leaves_the_table_as_it_was(local_catalog, tmp_path):
    _, table = _text_table(str(tmp_path))
    before = _state(table)

    def rows():
        yield pa.table({"a": pa.array([7], pa.int64()), "b": pa.array(["p"]), "c": pa.array([1.0])})
        raise RuntimeError("the stream stops")

    with pytest.raises(RuntimeError, match="the stream stops"):
        sink_module._replace_changing_types(table, rows(), ["a", "c"], {"a": "long", "b": "string", "c": "double"})
    assert _state(table) == before
    # A value the cast refuses is a failure inside the transaction too.
    with pytest.raises(Exception):
        sink_module._replace_changing_types(
            table, [pa.table({"a": pa.array(["x"]), "b": pa.array(["p"]), "c": pa.array([1.0])})], ["a", "c"],
            {"a": "long", "b": "string", "c": "double"},
        )
    assert _state(table) == before


@loader
def test_pr_186_review_should_fix_1_a_type_change_and_a_new_column_writes_the_new_column(local_catalog, tmp_path):
    """`PR #186 review SHOULD-FIX 1`: a replace that changes a column's type AND brings a
    column the table does not have wrote the rows but not the new column, and reported
    success. The new column is added to the table and written."""
    _, table = _text_table(str(tmp_path))
    types = {"a": "long", "b": "string", "c": "double", "d": "string"}
    typed = pa.table({
        "a": pa.array([7, 8, 9], pa.int64()),
        "b": pa.array(["p", "q", "r"], pa.string()),
        "c": pa.array([1.0, 2.0, 3.0], pa.float64()),
        "d": pa.array(["x", "y", "z"], pa.string()),
    })
    changed = sink_module._changed_columns(table, types)
    assert changed == ["a", "c"], "only the columns whose type differs"
    written = sink_module._replace_changing_types(table, _source(typed, []), changed, types)
    assert written == 3
    full = table.scan().to_arrow()
    assert "d" in full.schema.names, "the table carries the new column"
    assert sorted(full.column("d").to_pylist()) == ["x", "y", "z"], "the new column is written, not dropped"
    assert sorted(full.column("a").to_pylist()) == [7, 8, 9], "the changed type is written"
    assert dict((f.name, str(f.field_type)) for f in table.schema().fields)["d"] == "string"


@loader
def test_the_sink_writes_a_type_changing_replace_through_the_transaction_and_other_loads_through_dlt(local_catalog, tmp_path, monkeypatch):
    directory = str(tmp_path)
    pipeline, table = _text_table(directory)
    monkeypatch.setattr(sink_module, "_install_catalog_env", lambda config: None)
    monkeypatch.setattr(sink_module, "_bronze_pipeline", lambda name, config: pipeline)
    monkeypatch.setattr(sink_module, "_abort_pending", lambda *args, **kwargs: None)
    monkeypatch.setattr(sink_module.history, "refuse_history_rows", lambda *args, **kwargs: False)
    typed = pa.table({
        "a": pa.array([7, 8, 9], pa.int64()), "b": pa.array(["p", "q", "r"]), "c": pa.array(["u", "v", "w"]),
    })
    hints = [{"name": "a", "data_type": "bigint"}, {"name": "b", "data_type": "text"}, {"name": "c", "data_type": "text"}]
    purges: list[str] = []
    monkeypatch.setattr(type(table.catalog), "purge_table", lambda self, name: purges.append(str(name)), raising=False)
    monkeypatch.setattr(type(table.catalog), "drop_table", lambda self, name: purges.append(str(name)), raising=False)
    result = sink_module.load_via_sink(
        _source(typed, hints), "t", None, sink_module.LoadPlan(mode="replace"),
        column_types={"a": "long", "b": "string", "c": "string"},
    )
    assert (result.rows, result.has_failed_jobs) == (3, False)
    assert purges == [], "the table is never purged or dropped"
    assert dict(_iceberg_fields(directory))["a"] == "long"
    reopened = sink_module.history.open_table(pipeline, "t", sync=False)  # the sink held its own client's table
    assert _state(reopened)[1] == [7, 8, 9]
    # Same types: dlt loads, as before.
    again = sink_module.load_via_sink(
        _source(typed, hints), "t", None, sink_module.LoadPlan(mode="replace"),
        column_types={"a": "long", "b": "string", "c": "string"},
    )
    assert again.rows == 3
    assert "replaced in one commit" not in again.load_info_str
