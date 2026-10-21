"""Tests for `file_ingest.py`, the job that loads an uploaded file into a raw
Bronze table (T7 of `docs/superpowers/plans/2026-10-02-upload-file.md`,
ADR 0014).

No network. The object read, the sink, the catalog registration and the
`ingest_run` write are the parameters of `run_file_load`, so a test hands it
fakes the way `test_connector_catalog.py` hands `register_connector_table`
its ClickHouse calls; the one test that goes through the real
`load_via_sink` fakes `dlt.pipeline`, as `test_sink.py` does.

The fixture files of `ops/fixtures/uploads/` and the list of failure reasons
in `ops/fixtures/upload_load_failure_reasons.json` are read from their place
in the repository, two levels above this package. A missing directory or file
is a failure, never a skip: the dialect the preview and the load share, and
the sentences the API is willing to show, are pinned by them.
"""

from __future__ import annotations

import csv
import dataclasses
import inspect
import itertools
import glob
import io
import json
import os
import random
import re
import tracemalloc
import types
from pathlib import Path

import dlt
import pyarrow as pa
import pytest
from dagster import DagsterInvalidConfigError

from dispar_orchestrate import file_ingest
from dispar_orchestrate.adapters import sink as sink_module
from dispar_orchestrate.adapters.sink import LoadPlan, SinkConfig, SinkResult, _stamp_ingested_at
from dispar_orchestrate.definitions import defs
from dispar_orchestrate.file_ingest import (
    CONFIG_SCHEMA,
    FAILURE_REASONS,
    HEADER_NO_COLUMNS,
    HEADER_PAST_END,
    LOAD_FAILED,
    NO_ROWS,
    NOT_REGISTERED,
    TABLE_NAME,
    TOO_MANY_COLUMNS,
    TOO_MANY_ROWS,
    UNREADABLE,
    FileIngestParams,
    LoadFailure,
    OutcomeNotRecorded,
    _checked_key,
    _column_names,
    _blocks,
    _dlt_keeps_table_name,
    _lines,
    _text_chunks,
    file_ingest_job,
    open_stored_object,
    open_table,
    read_table,
    run_file_load,
)

REPO_ROOT = Path(__file__).resolve().parents[2]
FIXTURES = REPO_ROOT / "ops" / "fixtures" / "uploads"
REASONS_FILE = REPO_ROOT / "ops" / "fixtures" / "upload_load_failure_reasons.json"

TENANT = "00000000-0000-0000-0000-00000000a001"
KEY = f"uploads/{TENANT}/u-1.csv"
SINK_CONFIG = SinkConfig(
    lakekeeper_catalog_uri="http://lakekeeper:8181/catalog",
    lakekeeper_warehouse="default",
    rustfs_endpoint="http://rustfs:9000",
    rustfs_access_key="k",
    rustfs_secret_key="s",
    warehouse_bucket="lakehouse-warehouse",
    lakekeeper_token="",
)


def _fixture_cases() -> list[tuple[str, Path, dict]]:
    """Every fixture as (name, data file, expected JSON). Evaluated when the
    module is collected, so a missing directory is a collection error and not
    a parametrization with zero cases that would pass by running nothing."""
    assert FIXTURES.is_dir(), f"the fixture directory is missing: {FIXTURES}"
    cases = []
    for expected_path in sorted(FIXTURES.glob("*.expected.json")):
        name = expected_path.name.removesuffix(".expected.json")
        data = [p for p in FIXTURES.iterdir() if p.stem == name and not p.name.endswith(".expected.json")]
        assert len(data) == 1, f"{name}: expected exactly one data file beside {expected_path.name}, found {data}"
        cases.append((name, data[0], json.loads(expected_path.read_text(encoding="utf-8"))))
    return cases


FIXTURE_CASES = _fixture_cases()
FIXTURE_IDS = [name for name, _, _ in FIXTURE_CASES]


def _params(**overrides) -> FileIngestParams:
    base = FileIngestParams(
        upload_id="u-1",
        storage_keys=(KEY,),
        format="delimited",
        bronze_table_name="g9_orders",
        load_mode="replace",
        encoding="utf-8",
        delimiter=",",
        header_row=0,
        columns=(),
        on_bad_value="stop",
    )
    return dataclasses.replace(base, **overrides)


def _chunked(raw: bytes, size: int | None) -> list[bytes]:
    """`raw` in blocks of `size` bytes (the whole of it when `size` is None)."""
    if size is None:
        return [raw]
    return [raw[i : i + size] for i in range(0, len(raw), size)] or [b""]


def _decode(raw: bytes, encoding: str, block: int | None = None) -> str:
    """The text the job's decoder makes of `raw`, read `block` bytes at a time."""
    return "".join(_text_chunks(_chunked(raw, block), encoding))


def parse_file(text: str, delimiter: str, header_row: int):
    """Open the table and drain it, as a load does: the columns, the number of
    rows and the rows as dicts. Raises what `open_table` or the batches raise."""
    stream = open_table(text, delimiter, header_row)
    rows = [dict(zip(stream.columns, record)) for batch in stream.batches() for record in zip(*batch.to_pydict().values())]
    return types.SimpleNamespace(
        columns=stream.columns, row_count=stream.row_count, rows=lambda: iter(rows), stream=stream
    )


class _Log:
    def __init__(self) -> None:
        self.lines: list[str] = []

    def info(self, message: str) -> None:
        self.lines.append(f"INFO {message}")

    def warning(self, message: str) -> None:
        self.lines.append(f"WARNING {message}")


class _Harness:
    """The injected collaborators of `run_file_load`, recording what reaches
    them and failing on request."""

    def __init__(
        self,
        *,
        raw: bytes = b"id,name\n1,a\n2,b\n",
        read_error: Exception | None = None,
        load_error: Exception | None = None,
        sink_rows: int | None = 2,
        has_failed_jobs: bool = False,
        register_error: Exception | None = None,
        record_error: Exception | None = None,
        config_error: Exception | None = None,
        total: int = 2,
    ) -> None:
        self.raw = raw
        self.read_error = read_error
        self.load_error = load_error
        self.sink_rows = sink_rows
        self.has_failed_jobs = has_failed_jobs
        self.register_error = register_error
        self.record_error = record_error
        self.config_error = config_error
        self.total = total
        self.reads: list[str] = []
        self.loads: list[dict] = []
        self.registered: list[dict] = []
        self.recorded: list[dict] = []
        self.record_attempts = 0
        self.reports: list[tuple[str, bytes | None]] = []
        self.log = _Log()
        self._ticks = iter(f"2026-10-02T10:00:{second:02d}+00:00" for second in range(60))

    def load_config(self) -> SinkConfig:
        if self.config_error:
            raise self.config_error
        return SINK_CONFIG

    def read(self, config: SinkConfig, key: str):
        self.reads.append(key)
        if self.read_error:
            raise self.read_error
        return io.BytesIO(self.raw)

    def load(self, resource, table: str, config: SinkConfig, plan: LoadPlan, **kwargs) -> SinkResult:
        # The rows are consumed here, as dlt would, so a generator that is
        # read after the call returns would show up as an empty list.
        self.loads.append(
            {
                "table": table,
                "plan": plan,
                "column_types": kwargs.get("column_types"),
                "columns": dict(resource.columns),
                "rows": [row for batch in resource for row in batch.to_pylist()],
            }
        )
        if self.load_error:
            raise self.load_error
        return SinkResult(rows=self.sink_rows, has_failed_jobs=self.has_failed_jobs, load_info_str="load info")

    def register(self, table: str, *, description: str, author: str) -> int:
        self.registered.append({"table": table, "description": description, "author": author})
        if self.register_error:
            raise self.register_error
        return self.total

    def write_report(self, config: SinkConfig, key: str, payload: bytes | None) -> None:
        self.reports.append((key, payload))

    def record(self, **row) -> None:
        self.record_attempts += 1
        if self.record_error:
            raise self.record_error
        self.recorded.append(row)

    def now(self) -> str:
        return next(self._ticks)

    def run(self, params: FileIngestParams | None = None):
        return run_file_load(
            params or _params(),
            log=self.log,
            load_config=self.load_config,
            open_object=self.read,
            load=self.load,
            register=self.register,
            write_report=self.write_report,
            record=self.record,
            now=self.now,
        )


def _fails_with(harness: _Harness, reason: str, params: FileIngestParams | None = None) -> LoadFailure:
    with pytest.raises(LoadFailure) as caught:
        harness.run(params)
    assert caught.value.reason == reason
    return caught.value


# ── The reasons, and the fixtures both readers share ─────────────────────


def test_the_failure_reasons_equal_the_shared_fixture_file_the_api_is_tested_against():
    assert list(FAILURE_REASONS) == json.loads(REASONS_FILE.read_text(encoding="utf-8"))


def test_there_are_nine_different_failure_reasons_among_them_the_header_row_with_no_columns():
    # Review finding C2 made it seven: a header row with no columns has a
    # sentence of its own. SEC-17 made it eight with the column cap. The API
    # counts the same eight against the same file.
    in_file = json.loads(REASONS_FILE.read_text(encoding="utf-8"))
    assert len(FAILURE_REASONS) == len(set(FAILURE_REASONS)) == len(in_file) == 9
    assert HEADER_NO_COLUMNS == "The header row has no columns."
    assert HEADER_NO_COLUMNS != HEADER_PAST_END


def test_the_row_cap_in_the_sentence_is_the_row_cap_the_job_enforces():
    assert file_ingest.MAX_ROWS == 2_000_000
    assert f"{file_ingest.MAX_ROWS:,}" in TOO_MANY_ROWS


def test_the_column_cap_in_the_sentence_is_the_column_cap_the_job_enforces():
    assert file_ingest.MAX_COLUMNS == 1_000
    assert f"{file_ingest.MAX_COLUMNS:,}" in TOO_MANY_COLUMNS
    assert TOO_MANY_COLUMNS == "The file has more than 1,000 columns."


def test_a_load_failure_refuses_any_sentence_the_api_does_not_show():
    with pytest.raises(ValueError):
        LoadFailure("KeyError: 'amount' at /app/dagster/file_ingest.py:88")
    assert LoadFailure(UNREADABLE).reason == UNREADABLE


def test_the_fixture_directory_holds_a_data_file_for_every_expected_json_and_the_other_way_round():
    documents = {"README.md", ".gitattributes"}
    data = {
        p.stem
        for p in FIXTURES.iterdir()
        if p.is_file() and p.name not in documents and not p.name.endswith(".expected.json")
    }
    expected = {p.name.removesuffix(".expected.json") for p in FIXTURES.glob("*.expected.json")}
    assert data == expected
    assert len(expected) >= 10


@pytest.mark.parametrize("name,data_path,expected", FIXTURE_CASES, ids=FIXTURE_IDS)
def test_the_load_reads_every_fixture_to_its_expected_json(name, data_path, expected):
    text = _decode(data_path.read_bytes(), expected["encoding"])
    header, rows = read_table(text, expected["delimiter"], expected["headerRow"])
    assert header == expected["columns"]
    assert list(rows) == expected["rows"]


@pytest.mark.parametrize("name,data_path,expected", FIXTURE_CASES, ids=FIXTURE_IDS)
def test_the_rows_counted_before_the_load_are_the_rows_that_are_loaded(name, data_path, expected):
    text = _decode(data_path.read_bytes(), expected["encoding"])
    parsed = parse_file(text, expected["delimiter"], expected["headerRow"])
    loaded = list(parsed.rows())
    assert parsed.row_count == len(expected["rows"]) == len(loaded)
    assert all(list(row) == parsed.columns for row in loaded)


# ── Decoding ─────────────────────────────────────────────────────────────


def test_utf16_follows_its_byte_order_mark_and_is_little_endian_without_one():
    text = "id\tname\r\n1\tÖl\r\n"
    assert _decode(b"\xff\xfe" + text.encode("utf-16-le"), "utf-16") == text
    assert _decode(b"\xfe\xff" + text.encode("utf-16-be"), "utf-16") == text
    assert _decode(text.encode("utf-16-le"), "utf-16") == text


def test_a_trailing_odd_byte_of_utf16_is_dropped_as_the_preview_drops_it():
    assert _decode(b"\xff\xfe" + "ab".encode("utf-16-le") + b"A", "utf-16") == "ab"


def test_a_utf8_byte_order_mark_is_dropped_and_a_bad_byte_becomes_a_replacement_character():
    assert _decode(b"\xef\xbb\xbf" + b'"id",x', "utf-8") == '"id",x'
    assert _decode(b"Ren\xe9,x", "utf-8") == "Ren�,x"


# ── Streaming (DATA-17 T2.1): blocks, lines, batches ─────────────────────

STREAM_TEXT = 'id,name,note\r\n1,"a\r\nb",x\r\n2,"é, ö",y\r\n3,c,\u2028z\n4,d,\r5,e,f\n'


@pytest.mark.parametrize("block", [1, 2, 3, 5, 7, 64])
def test_utf8_split_across_blocks_decodes_to_the_same_text(block):
    raw = STREAM_TEXT.encode("utf-8")
    assert _decode(raw, "utf-8", block) == STREAM_TEXT
    # A byte order mark in a block of its own, and a bad byte, are still handled.
    assert _decode(b"\xef\xbb\xbf" + raw, "utf-8", block) == STREAM_TEXT
    assert _decode(b"Ren\xe9,x", "utf-8", block) == "Ren�,x"


@pytest.mark.parametrize("block", [1, 2, 3, 5, 7, 64])
def test_utf16_split_across_blocks_decodes_to_the_same_text_with_or_without_a_mark(block):
    text = STREAM_TEXT + "\U0001F600\n"  # a surrogate pair, which a block boundary can split
    assert _decode(b"\xff\xfe" + text.encode("utf-16-le"), "utf-16", block) == text
    assert _decode(b"\xfe\xff" + text.encode("utf-16-be"), "utf-16", block) == text
    assert _decode(text.encode("utf-16-le"), "utf-16", block) == text
    # A trailing odd byte is dropped, whatever the block size.
    assert _decode(b"\xff\xfe" + "ab".encode("utf-16-le") + b"A", "utf-16", block) == "ab"
    assert _decode(b"", "utf-16", block) == ""
    assert _decode(b"\xff", "utf-16", block) == ""


@pytest.mark.parametrize("block", [1, 2, 3, 4, 5, 8, 100])
def test_lines_are_cut_where_stringio_cuts_whatever_the_block_size(block):
    reference = list(io.StringIO(STREAM_TEXT, newline=""))
    pieces = [STREAM_TEXT[i : i + block] for i in range(0, len(STREAM_TEXT), block)]
    assert list(_lines(pieces)) == reference
    # A carriage return at the end of one block and a line feed at the start of the next are one break.
    assert list(_lines(["a\r", "\nb\r", "\r", "c"])) == ["a\r\n", "b\r", "\r", "c"]
    assert list(_lines(["a\r"])) == ["a\r"]
    assert list(_lines([])) == []


@pytest.mark.parametrize("block", [1, 3, 11, 4096])
@pytest.mark.parametrize("encoding", ["utf-8", "utf-16"])
def test_every_fixture_streams_to_its_expected_json_in_blocks_of_any_size(block, encoding, ):
    # The same fixtures the preview reads, through the streaming path, cut into
    # tiny blocks: what is loaded does not depend on where a block ends.
    for name, data_path, expected in FIXTURE_CASES:
        if expected["encoding"] != encoding:
            continue
        lines = _lines(_text_chunks(_chunked(data_path.read_bytes(), block), encoding))
        stream = open_table(lines, expected["delimiter"], expected["headerRow"])
        got = [list(r) for batch in stream.batches() for r in zip(*batch.to_pydict().values())]
        assert got == [(row + [""] * len(stream.columns))[: len(stream.columns)] for row in expected["rows"]], name


@pytest.mark.parametrize("delimiter", [",", ";", "\t", "|"])
def test_each_delimiter_streams_through_the_whole_job(delimiter):
    raw = f"id{delimiter}name\n1{delimiter}a\n2{delimiter}b\n".encode()
    harness = _Harness(raw=raw)
    harness.run(_params(delimiter=delimiter))
    assert harness.loads[0]["rows"] == [{"id": "1", "name": "a"}, {"id": "2", "name": "b"}]


def test_a_file_is_read_in_blocks_of_the_read_block_size():
    sizes: list[int] = []

    class Counting(io.BytesIO):
        def read(self, size=-1):
            sizes.append(size)
            return super().read(size)

    list(_blocks(Counting(b"x" * 10)))
    assert sizes == [8 * 1024 * 1024, 8 * 1024 * 1024]


def test_the_first_row_is_read_before_the_sink_is_asked_for_anything():
    harness = _Harness(raw=b"a,b\n")  # a header and nothing below it
    _fails_with(harness, NO_ROWS)
    assert harness.loads == []
    assert harness.registered == []


def test_the_row_cap_fails_the_run_from_inside_the_load_and_the_reason_is_not_the_loads(monkeypatch):
    monkeypatch.setattr(file_ingest, "MAX_ROWS", 3)
    monkeypatch.setattr(file_ingest, "BATCH_ROWS", 2)
    harness = _Harness(raw=b"a\n1\n2\n3\n4\n")
    # The fake load reads the resource as dlt would; the generator raises mid-way and the
    # fake wraps it as dlt does, so only the stream's own record of it tells the reason.
    original = harness.load

    def wrapping(resource, table, config, plan, **kwargs):
        try:
            return original(resource, table, config, plan, **kwargs)
        except LoadFailure as exc:
            raise RuntimeError("PipelineStepFailed") from exc

    harness.load = wrapping
    failure = _fails_with(harness, TOO_MANY_ROWS)
    assert failure.rows is None
    assert harness.registered == []
    assert [row["error"] for row in harness.recorded] == [TOO_MANY_ROWS]


def test_a_store_that_fails_while_the_file_is_read_is_unreadable_and_not_a_failed_load():
    class Dying(io.BytesIO):
        calls = 0

        def read(self, size=-1):
            Dying.calls += 1
            if Dying.calls > 1:
                raise OSError("connection reset by peer")
            return b"a,b\n" + b"1,2\n" * 10

    harness = _Harness()
    harness.read = lambda config, key: Dying()
    original = harness.load

    def wrapping(resource, table, config, plan, **kwargs):
        try:
            return original(resource, table, config, plan, **kwargs)
        except LoadFailure as exc:
            raise RuntimeError("PipelineStepFailed") from exc

    harness.load = wrapping
    failure = _fails_with(harness, UNREADABLE)
    assert isinstance(failure.__cause__, OSError)  # the store's own error, for the run log


def _local_pipeline_load(directory: str):
    """A `load` that runs the resource through a real dlt pipeline into a
    local filesystem destination, stamped as the sink stamps it. Not Iceberg:
    it proves what dlt does with a generator that raises, which is dlt's."""

    def load(resource, table, config, plan, **kwargs):
        pipeline = dlt.pipeline(
            "v4", destination=dlt.destinations.filesystem(bucket_url=directory), dataset_name="ds",
            pipelines_dir=os.path.join(directory, "_pipelines"),
        )
        resource.add_map(_stamp_ingested_at)
        pipeline.run(resource, table_name=table, write_disposition=plan.mode, loader_file_format="parquet")
        return SinkResult(rows=None, has_failed_jobs=False, load_info_str="")

    return load


def test_a_failure_while_dlt_extracts_leaves_the_destination_untouched_and_the_next_run_starts_clean(
    tmp_path, monkeypatch
):
    # V4 of the plan, measured with dlt 1.30.0: the row cap raises inside the
    # generator, which is dlt's extract step, before anything is normalized or loaded.
    directory = str(tmp_path)
    ok = _Harness(raw=b"a\n1\n2\n")
    ok.load = _local_pipeline_load(directory)
    ok.run()
    before = sorted(glob.glob(os.path.join(directory, "ds", "g9_orders", "*")))
    assert len(before) == 1

    monkeypatch.setattr(file_ingest, "MAX_ROWS", 3)
    monkeypatch.setattr(file_ingest, "BATCH_ROWS", 1)
    over = _Harness(raw=b"a\n1\n2\n3\n4\n5\n")
    over.load = _local_pipeline_load(directory)
    _fails_with(over, TOO_MANY_ROWS)
    assert sorted(glob.glob(os.path.join(directory, "ds", "g9_orders", "*"))) == before

    monkeypatch.undo()
    again = _Harness(raw=b"a\n7\n8\n9\n")
    again.load = _local_pipeline_load(directory)
    again.run()
    [after] = glob.glob(os.path.join(directory, "ds", "g9_orders", "*.parquet"))
    import pyarrow.parquet as pq

    table = pq.read_table(after)
    assert table.column("a").to_pylist() == ["7", "8", "9"]  # replaced, nothing of the failed run
    assert table.schema.field("_ingested_at").type.tz == "UTC"


class _GeneratedCsv:
    """A CSV of about `target` bytes, made as it is read: the test never holds it."""

    def __init__(self, target: int) -> None:
        self.target, self.sent, self.n = target, 0, 0
        self.pending = b"id,name,city,amount,note,code\n"

    def read(self, size: int) -> bytes:
        parts, have = [], 0
        if self.pending:
            parts.append(self.pending)
            have, self.pending = len(self.pending), b""
        while have < size and self.sent + have < self.target:
            self.n += 1
            row = (
                f"{self.n},customer name {self.n},Jakarta Selatan,{self.n * 3}.50,"
                f"a note that is a bit long to fill the row {self.n},C{self.n:09d}\n"
            ).encode()
            parts.append(row)
            have += len(row)
        data = b"".join(parts)
        self.sent += len(data)
        return data

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False


def _peak_traced_bytes(target_bytes: int) -> tuple[int, int]:
    """(peak traced memory, rows) of streaming a generated file of this size."""
    source = _GeneratedCsv(target_bytes)
    tracemalloc.start()
    try:
        stream = open_table(_lines(_text_chunks(_blocks(source), "utf-8")), ",", 0)
        rows = sum(batch.num_rows for batch in stream.batches())
        return tracemalloc.get_traced_memory()[1], rows
    finally:
        tracemalloc.stop()


def test_peak_memory_does_not_grow_with_the_size_of_the_file(monkeypatch):
    # Smaller blocks and batches than production's, so the steady state is reached
    # within a file that is quick to read under tracemalloc; the 200 MB test below
    # runs the production sizes.
    monkeypatch.setattr(file_ingest, "READ_BLOCK", 1024 * 1024)
    monkeypatch.setattr(file_ingest, "BATCH_ROWS", 5_000)
    small, small_rows = _peak_traced_bytes(6 * 1024 * 1024)
    large, large_rows = _peak_traced_bytes(24 * 1024 * 1024)
    assert large_rows > 3 * small_rows  # the large file really is about four times the small one
    assert large < 16 * 1024 * 1024  # a fixed bound, far below the 24 MB file
    assert large < small * 1.3  # ... that the larger file does not move


@pytest.mark.skipif(not os.environ.get("FILE_INGEST_LARGE"), reason="200 MB under tracemalloc takes over a minute; set FILE_INGEST_LARGE=1")
def test_a_200_mb_generated_file_streams_in_a_fixed_bound():
    peak, rows = _peak_traced_bytes(200 * 1024 * 1024)
    assert rows > 1_500_000  # about 200 MB of rows, under the 2,000,000-row cap
    assert peak < 128 * 1024 * 1024


def test_the_sink_stamps_an_arrow_table_and_a_record_batch_and_replaces_a_column_of_the_same_name():
    table = pa.table({"a": pa.array(["1", "2"], pa.string()), "_ingested_at": pa.array(["old", "old"], pa.string())})
    stamped = _stamp_ingested_at(table)
    assert stamped.column_names == ["a", "_ingested_at"]
    assert stamped.schema.field("_ingested_at").type == pa.timestamp("us", tz="UTC")
    assert stamped.num_rows == 2 and stamped.column("a").to_pylist() == ["1", "2"]
    from_batch = _stamp_ingested_at(pa.record_batch({"a": pa.array(["x"], pa.string())}))
    assert from_batch.column_names == ["a", "_ingested_at"] and from_batch.num_rows == 1
    # A row (a dict) is stamped as before.
    assert _stamp_ingested_at({"a": "1"})["_ingested_at"].tzinfo is not None


# ── What the load does on top of the dialect ─────────────────────────────


def test_a_short_row_is_padded_and_a_long_row_is_cut_to_the_headers_length():
    text = _decode((FIXTURES / "ragged_rows.csv").read_bytes(), "utf-8")
    parsed = parse_file(text, ",", 0)
    assert list(parsed.rows()) == [
        {"id": "1", "name": "short", "qty": ""},
        {"id": "2", "name": "full", "qty": "5"},
        {"id": "3", "name": "long", "qty": "6"},
    ]


def test_cells_are_stored_with_the_whitespace_the_file_has_and_are_never_trimmed():
    text = _decode((FIXTURES / "duplicate_blank_headers.csv").read_bytes(), "utf-8")
    first = next(parse_file(text, ",", 0).rows())
    assert first["name_2"] == " steel "
    assert first["qty"] == "10"
    padded = parse_file("id,note\n1,  two spaces  \n", ",", 0)
    assert next(padded.rows()) == {"id": "1", "note": "  two spaces  "}


def test_values_are_the_text_they_are_whatever_they_look_like():
    parsed = parse_file("code,when,qty\n0250161,2025-02-07T10:00:00+07:00,007\n", ",", 0)
    assert next(parsed.rows()) == {"code": "0250161", "when": "2025-02-07T10:00:00+07:00", "qty": "007"}


def test_blank_records_below_the_header_are_skipped_and_are_not_counted():
    parsed = parse_file("a,b\n1,2\n\n   ,\t\n3,4\n,\n", ",", 0)
    assert parsed.row_count == 2
    assert [row["a"] for row in parsed.rows()] == ["1", "3"]


def test_the_header_row_counts_records_blank_ones_included():
    text = "\n\nid,name\n1,a\n"  # four records: two empty ones, the header, one row
    parsed = parse_file(text, ",", 2)
    assert parsed.columns == ["id", "name"]
    assert parsed.row_count == 1
    with pytest.raises(LoadFailure) as below:
        parse_file(text, ",", 3)  # the row itself as the header: nothing is left below it
    assert below.value.reason == NO_ROWS
    with pytest.raises(LoadFailure) as past:
        parse_file(text, ",", 4)
    assert past.value.reason == HEADER_PAST_END
    for empty in (0, 1):  # the two empty records are records, with no cells in them
        with pytest.raises(LoadFailure) as none:
            parse_file(text, ",", empty)
        assert none.value.reason == HEADER_NO_COLUMNS


def test_a_header_past_the_end_of_the_file_is_that_failure():
    for text in ["", "a,b\n1,2\n"]:
        with pytest.raises(LoadFailure) as caught:
            parse_file(text, ",", 9)
        assert caught.value.reason == HEADER_PAST_END


def test_a_header_record_with_no_cells_has_a_sentence_of_its_own_and_is_not_one_past_the_end():
    # Review finding C2. The preview answers both with an empty `columns`; the
    # load says which one it is, because a record IS there in the first case.
    for text, header_row in [("\nid\n1\n", 0), ("id\n\n1\n", 1), ("a,b\n1,2\n\n", 2)]:
        with pytest.raises(LoadFailure) as caught:
            parse_file(text, ",", header_row)
        assert caught.value.reason == HEADER_NO_COLUMNS, (text, header_row)
        assert isinstance(caught.value.__cause__, ValueError)  # what was wrong, for the run log
    # Its neighbours keep their own: a record with a cell, and a record that is not there.
    assert parse_file("\nid\n1\n", ",", 1).columns == ["id"]
    with pytest.raises(LoadFailure) as past:
        parse_file("\nid\n1\n", ",", 3)
    assert past.value.reason == HEADER_PAST_END
    # A cell that is only whitespace is a cell: the column gets a name, as before.
    assert parse_file(" \nx\n", ",", 0).columns == ["col_0"]


@pytest.mark.parametrize("text", ["a,b\n", "a,b\n\n  \n,\n", "a,b"])
def test_a_file_with_nothing_below_the_header_has_no_rows(text):
    with pytest.raises(LoadFailure) as caught:
        parse_file(text, ",", 0)
    assert caught.value.reason == NO_ROWS


def test_a_header_of_1000_columns_is_parsed():
    text = ",".join(f"c{n}" for n in range(1_000)) + "\n" + ",".join("1" for _ in range(1_000)) + "\n"
    parsed = parse_file(text, ",", 0)
    assert len(parsed.columns) == 1_000
    assert parsed.row_count == 1


def test_a_header_of_1001_columns_is_refused_before_any_row_is_read():
    # No row below the header: read in order, the file would fail as NO_ROWS,
    # so TOO_MANY_COLUMNS shows the header was judged first.
    text = ",".join(f"c{n}" for n in range(1_001)) + "\n"
    with pytest.raises(LoadFailure) as caught:
        parse_file(text, ",", 0)
    assert caught.value.reason == TOO_MANY_COLUMNS


def test_a_header_of_nothing_but_commas_is_over_the_column_cap():
    with pytest.raises(LoadFailure) as caught:
        parse_file("," * 5_000 + "\n1\n", ",", 0)
    assert caught.value.reason == TOO_MANY_COLUMNS


def test_the_row_cap_is_a_failure_found_while_counting_and_a_file_at_the_cap_passes(monkeypatch):
    monkeypatch.setattr(file_ingest, "MAX_ROWS", 3)
    assert parse_file("a\n1\n2\n3\n", ",", 0).row_count == 3
    with pytest.raises(LoadFailure) as caught:
        parse_file("a\n1\n2\n3\n4\n", ",", 0)
    assert caught.value.reason == TOO_MANY_ROWS


def test_rows_leave_as_arrow_batches_of_at_most_the_batch_size_and_not_as_a_list_held_in_memory(monkeypatch):
    monkeypatch.setattr(file_ingest, "BATCH_ROWS", 2)
    stream = open_table("a,b\n1,x\n2,y\n3,z\n4,w\n5,v\n", ",", 0)
    batches = stream.batches()
    assert inspect.isgenerator(batches)
    first = next(batches)
    assert first.num_rows == 2 and first.schema.types == [pa.string(), pa.string()]
    assert stream.row_count == 2  # counted as they pass, not before
    rest = list(batches)
    assert [b.num_rows for b in rest] == [2, 1]
    assert stream.row_count == 5


# ── Column names ─────────────────────────────────────────────────────────


def test_blank_duplicate_and_awkward_header_cells_become_stable_unique_names():
    header = ["Name", "Name", "", "Qty ", "", "A.scrap", "2024", "Material description"]
    assert _column_names(header) == [
        "name",
        "name_2",
        "col_2",
        "qty",
        "col_4",
        "a_scrap",
        "col_2024",
        "material_description",
    ]


def test_a_suffixed_repeat_never_takes_the_name_another_column_already_has():
    assert _column_names(["a", "a", "a_2"]) == ["a", "a_2", "a_2_2"]
    assert _column_names(["a_2", "a", "a"]) == ["a_2", "a", "a_3"]
    assert _column_names(["col_1", ""]) == ["col_1", "col_1_2"]


def test_non_ascii_letters_are_separators_so_two_such_names_cannot_merge():
    assert _column_names(["Größe", "名前", "値", "Ünïcode"]) == ["gr_e", "col_1", "col_2", "n_code"]


def _sketch_column_names(header: list[str]) -> list[str]:
    """The rule `file_ingest.py` had in the sketch from `f9793cd`, kept as the
    oracle for what must not change."""
    names: list[str] = []
    seen: dict[str, int] = {}
    for index, cell in enumerate(header):
        base = "".join(c.lower() if c.isalnum() else "_" for c in cell.strip()).strip("_")
        while "__" in base:
            base = base.replace("__", "_")
        if not base or base[0].isdigit():
            base = f"col_{index}" if not base else f"col_{base}"
        seen[base] = seen.get(base, 0) + 1
        names.append(base if seen[base] == 1 else f"{base}_{seen[base]}")
    return names


def test_an_ascii_header_gets_the_names_the_sketch_gave_unless_the_sketch_collided():
    rng = random.Random(20261002)
    alphabet = "abcXYZ019 _-.,/()"
    compared = 0
    for _ in range(4000):
        header = [
            "".join(rng.choice(alphabet) for _ in range(rng.randint(0, 6))) for _ in range(rng.randint(1, 7))
        ]
        old = _sketch_column_names(header)
        if len(set(old)) != len(old):
            continue
        compared += 1
        assert _column_names(header) == old, header
    assert compared > 1000


def test_every_derived_name_survives_dlts_own_naming_unchanged_and_no_two_collide():
    naming = dlt.Schema("upload").naming
    rng = random.Random(20261003)
    alphabet = list("abAB019 _-.,;|\t\"'+*@/éÉüßöñä²ǅ") + ["名", "前", "値", "ж", "٣", " ", "\n"]
    for _ in range(3000):
        header = [
            "".join(rng.choice(alphabet) for _ in range(rng.randint(0, 8))) for _ in range(rng.randint(1, 9))
        ]
        names = _column_names(header)
        assert len(set(names)) == len(names), header
        for name in names:
            assert naming.normalize_identifier(name) == name, (header, name)


# ── The key, the object, the settings ────────────────────────────────────


@pytest.mark.parametrize(
    "key",
    ["", "uploads", "uploads/", "/uploads/x", "Uploads/x", "bronze/orders/x", "uploads/../bronze/x", "uploads/a/../../x"],
)
def test_a_key_outside_the_uploads_prefix_or_climbing_out_of_it_is_never_read(key):
    harness = _Harness()
    _fails_with(harness, UNREADABLE, _params(storage_keys=(key,)))
    assert harness.reads == []
    assert harness.loads == []
    assert [row["error"] for row in harness.recorded] == [UNREADABLE]


def test_a_key_the_api_builds_passes_the_guard():
    assert _checked_key(KEY) == KEY
    assert _checked_key(f"uploads/{TENANT}/u-1") == f"uploads/{TENANT}/u-1"


def test_the_object_is_opened_for_streaming_from_the_warehouse_bucket_with_the_sinks_credentials():
    seen: dict = {}

    class FakeFilesystem:
        def __init__(self, **kwargs) -> None:
            seen["kwargs"] = kwargs

        def open(self, path: str, mode: str, **kwargs):
            seen.update(path=path, mode=mode, open_kwargs=kwargs)
            return io.BytesIO(b"bytes")

    assert open_stored_object(SINK_CONFIG, KEY, filesystem_factory=FakeFilesystem).read() == b"bytes"
    assert seen["path"] == f"lakehouse-warehouse/{KEY}"
    assert seen["mode"] == "rb"
    assert seen["open_kwargs"] == {"block_size": 8 * 1024 * 1024}
    assert seen["kwargs"] == {
        "key": "k",
        "secret": "s",
        "client_kwargs": {"endpoint_url": "http://rustfs:9000"},
    }


@pytest.mark.parametrize(
    "overrides",
    [
        {"upload_id": "  "},
        {"load_mode": "incremental"},
        {"load_mode": "merge"},
        {"load_mode": ""},
        {"encoding": "latin-1"},
        {"encoding": "UTF-8"},
        {"delimiter": ":"},
        {"delimiter": ",,"},
        {"delimiter": ""},
        {"delimiter": '"'},
        {"header_row": -1},
        {"bronze_table_name": "Orders"},
        {"bronze_table_name": "orders 2025"},
        {"bronze_table_name": "1orders"},
        {"bronze_table_name": "orders\n"},
        {"bronze_table_name": "orders`; DROP TABLE x"},
        {"bronze_table_name": ""},
        {"bronze_table_name": "t" * 129},
    ],
    ids=lambda overrides: next(iter(overrides)) + "=" + repr(next(iter(overrides.values())))[:24],
)
def test_a_launch_the_api_would_not_have_sent_fails_before_anything_is_read(overrides):
    harness = _Harness()
    failure = _fails_with(harness, LOAD_FAILED, _params(**overrides))
    assert harness.reads == [] and harness.loads == [] and harness.registered == []
    assert [row["error"] for row in harness.recorded] == [LOAD_FAILED]
    assert failure.__cause__ is not None  # what was wrong, for the run log


# The table name, review finding C1. The rule is the API's
# (`routes::uploads::table_name_problem`): a lower-case letter, then groups of
# lower-case letters and digits joined by single underscores, at most 128
# characters. The two sides' tests use the plan's names alike.


@pytest.mark.parametrize(
    "table",
    ["a", "a1", "a_1", "sap_material_master", "orders", "g9_upload_0a1b2c3d", "t1", "a_b", "q1_2", "t" * 128],
)
def test_a_table_name_that_follows_the_upload_rule_is_loaded(table):
    harness = _Harness()
    harness.run(_params(bronze_table_name=table))
    assert harness.loads[0]["table"] == table


@pytest.mark.parametrize(
    "table",
    [
        # The plan's refusals.
        "x_",
        "_x",
        "a__b",
        "1a",
        "Orders",
        "t" * 129,
        # What the rule before C1 admitted: dlt keeps `_x`, `a__b` and `a___b`
        # and renames the rest (`x_` as `xx`, `__x` as `x`, `s__1` as `s___1`).
        "a___b",
        "__x",
        "a__",
        "s__1",
        "q1__2",
        "_",
        "__",
        "___",
        "_a_",
        "a_1_",
        "_staging",
        # Never valid: the pattern has to match the whole name, not a prefix.
        "",
        "orders 2025",
        "orders-raw",
        "orders\n",
        "orders`; DROP TABLE x",
    ],
)
def test_a_table_name_that_breaks_the_upload_rule_is_refused_before_dlt_is_asked(table, monkeypatch):
    asked: list[str] = []
    monkeypatch.setattr(file_ingest, "_dlt_keeps_table_name", lambda name: asked.append(name) or True)
    harness = _Harness()
    failure = _fails_with(harness, LOAD_FAILED, _params(bronze_table_name=table))
    assert harness.reads == [] and harness.loads == [] and harness.registered == []
    assert [row["error"] for row in harness.recorded] == [LOAD_FAILED]
    assert asked == [], "the pattern refused it; dlt is only asked about a name that passed"
    assert "single underscores" in str(failure.__cause__)  # the pattern's problem, in the run log


def test_a_name_that_passes_the_pattern_is_still_refused_when_dlt_would_write_it_under_another_name(monkeypatch):
    # The guard behind the pattern. With this dlt it never refuses a name the
    # pattern admits (the enumeration below), so it is driven by one that does.
    asked: list[str] = []

    def renames(name: str) -> bool:
        asked.append(name)
        return False

    monkeypatch.setattr(file_ingest, "_dlt_keeps_table_name", renames)
    harness = _Harness()
    failure = _fails_with(harness, LOAD_FAILED, _params(bronze_table_name="orders"))
    assert asked == ["orders"]
    assert "dlt would write under another one" in str(failure.__cause__)
    assert harness.reads == [] and harness.loads == []


@pytest.mark.parametrize("table", ["orders", "a_1", "sap_material_master", "_x", "a__b", "a___b"])
def test_dlt_keeps_a_name_its_naming_leaves_alone(table):
    # `_x`, `a__b` and `a___b` are kept by dlt and still refused by the upload
    # rule, which is a sentence a person can follow and not dlt's own.
    assert _dlt_keeps_table_name(table) is True


@pytest.mark.parametrize("table", ["x_", "__x", "a__", "s__1", "q1__2", "_", "__", "___", "_a_", "a_1_"])
def test_dlt_would_write_these_names_under_another_one(table):
    # Measured with dlt 1.30.0: `x_` becomes `xx`, `__x` becomes `x`, `s__1`
    # becomes `s___1`; the rows would land in a table nobody asked for.
    assert _dlt_keeps_table_name(table) is False


def test_no_short_name_the_upload_rule_admits_is_renamed_by_dlt():
    # What makes the rule worth having: everything it admits is what dlt keeps.
    # Every name of up to nine characters over `a`, `1` and `_`, which holds the
    # cases that went wrong before (a trailing `_`, a doubled `_`, a digit after
    # `__`). The reviewer put 59,052 names through the same question on
    # 2026-10-02 (plan section 9, review of slice C). The naming is built once:
    # `_dlt_keeps_table_name` builds a schema per call, which is right for one
    # call per run and too slow for thousands.
    naming = dlt.Schema("upload").naming
    admitted = 0
    for length in range(1, 10):
        for characters in itertools.product("a1_", repeat=length):
            name = "".join(characters)
            if TABLE_NAME.fullmatch(name):
                admitted += 1
                assert naming.normalize_tables_path(name) == name, name
    assert admitted == 3861  # the count over this alphabet, so the loop cannot quietly check nothing


def test_every_launch_field_is_required_in_the_op_config_and_the_params_have_no_other():
    assert set(CONFIG_SCHEMA) == {field.name for field in dataclasses.fields(FileIngestParams)}
    assert all(field.is_required for field in CONFIG_SCHEMA.values())


# ── A load that succeeds ─────────────────────────────────────────────────


def test_a_successful_load_records_exactly_one_succeeded_row_with_the_sinks_count():
    harness = _Harness(sink_rows=2, total=7)
    summary = harness.run()

    assert harness.recorded == [
        {
            "connector_id": "upload:u-1",
            "job": "file_ingest_job",
            "object_name": "g9_orders",
            "rows": 2,
            "started_at": "2026-10-02T10:00:00+00:00",
            "ended_at": "2026-10-02T10:00:01+00:00",
            "status": "succeeded",
            "error": "",
        }
    ]
    assert harness.record_attempts == 1
    assert (summary.table, summary.rows, summary.parsed_rows, summary.columns, summary.table_total) == (
        "g9_orders",
        2,
        2,
        2,
        7,
    )


def test_a_row_count_the_sink_could_not_measure_is_recorded_as_null_and_never_as_the_files_rows():
    harness = _Harness(sink_rows=None)
    summary = harness.run()
    assert summary.parsed_rows == 2  # the file held two rows ...
    assert summary.rows is None
    assert harness.recorded[0]["rows"] is None  # ... but nothing measured them as loaded


def test_the_load_mode_reaches_the_sinks_plan():
    for mode in ("replace", "append"):
        harness = _Harness()
        harness.run(_params(load_mode=mode))
        assert harness.loads[0]["plan"] == LoadPlan(mode=mode)


def test_every_column_is_declared_text_in_the_files_own_order_and_every_value_is_a_string():
    raw = b'id,when,code\n1,2025-02-07T10:00:00+07:00,0250161\n2,not a date,"007"\n'
    harness = _Harness(raw=raw)
    harness.run()
    loaded = harness.loads[0]
    assert list(loaded["columns"]) == ["id", "when", "code"]
    assert {hints["data_type"] for hints in loaded["columns"].values()} == {"text"}
    assert loaded["rows"] == [
        {"id": "1", "when": "2025-02-07T10:00:00+07:00", "code": "0250161"},
        {"id": "2", "when": "not a date", "code": "007"},
    ]


def test_the_decoded_text_with_the_confirmed_delimiter_and_header_row_is_what_is_loaded():
    expected = json.loads((FIXTURES / "sap_report_utf16.expected.json").read_text(encoding="utf-8"))
    raw = (FIXTURES / "sap_report_utf16.xls").read_bytes()
    harness = _Harness(raw=raw)
    harness.run(_params(encoding="utf-16", delimiter="\t", header_row=4))
    assert list(harness.loads[0]["columns"]) == ["plnt", "material", "description", "qty"]
    assert [list(row.values()) for row in harness.loads[0]["rows"]] == expected["rows"]


def test_a_cell_longer_than_pythons_default_limit_loads_and_the_limit_is_put_back():
    before = csv.field_size_limit()
    huge = "x" * (before + 1000)
    with pytest.raises(csv.Error):  # why the job raises it: the default refuses this cell
        parse_file(f'a,b\n"{huge}",1\n', ",", 0)
    harness = _Harness(raw=f'a,b\n"{huge}",1\n'.encode())
    harness.run()
    assert harness.loads[0]["rows"] == [{"a": huge, "b": "1"}]
    assert csv.field_size_limit() == before


def test_the_table_is_registered_for_the_shared_catalog_without_the_tenant_or_the_file_name():
    harness = _Harness()
    harness.run()
    [registered] = harness.registered
    assert registered["table"] == "g9_orders"
    assert registered["author"] == "upload"
    assert "u-1" in registered["description"]
    assert TENANT not in registered["description"]
    assert ".csv" not in registered["description"]


def test_a_table_registered_by_an_upload_reaches_the_catalog_through_the_shared_helper():
    from dispar_orchestrate import connector_catalog

    assert file_ingest.register_loaded_table is connector_catalog.register_loaded_table
    assert inspect.signature(run_file_load).parameters["register"].default is connector_catalog.register_loaded_table


# ── A load that fails: one row, one reason, never the exception's text ───


def test_an_object_that_cannot_be_read_records_the_unreadable_reason_and_keeps_the_detail_in_the_log():
    marker = "AccessDenied for key AKIA-MARKER at 10.1.2.3:9000"
    harness = _Harness(read_error=OSError(marker))
    failure = _fails_with(harness, UNREADABLE)

    [row] = harness.recorded
    assert (row["status"], row["error"], row["rows"]) == ("failed", UNREADABLE, None)
    assert marker not in json.dumps(row)
    assert any(marker in line for line in harness.log.lines)  # the run log has it
    assert isinstance(failure.__cause__, OSError)
    assert harness.loads == [] and harness.registered == []


@pytest.mark.parametrize(
    "raw,params,reason",
    [
        (b"a,b\n1,2\n", {"header_row": 5}, HEADER_PAST_END),
        (b"\nid\n1\n", {}, HEADER_NO_COLUMNS),  # review finding C2
        (b"id\n\n1\n", {"header_row": 1}, HEADER_NO_COLUMNS),
        (b"a,b\n\n \n", {}, NO_ROWS),
        (b"a,b\n", {}, NO_ROWS),
    ],
)
def test_a_file_that_cannot_be_loaded_writes_nothing_and_records_its_reason(raw, params, reason):
    harness = _Harness(raw=raw)
    _fails_with(harness, reason, _params(**params))
    assert harness.loads == [] and harness.registered == []
    [row] = harness.recorded
    assert (row["status"], row["error"], row["rows"]) == ("failed", reason, None)


def test_a_file_over_the_row_cap_fails_and_nothing_is_written(monkeypatch):
    monkeypatch.setattr(file_ingest, "MAX_ROWS", 3)
    harness = _Harness(raw=b"a\n1\n2\n3\n4\n")
    _fails_with(harness, TOO_MANY_ROWS)
    assert harness.loads == [] and harness.registered == []
    [row] = harness.recorded
    assert (row["status"], row["error"], row["rows"]) == ("failed", TOO_MANY_ROWS, None)


def test_a_failure_inside_the_sink_records_the_load_reason_and_registers_nothing():
    marker = "pyiceberg CommitFailedException at s3://lakehouse-warehouse/bronze/x"
    harness = _Harness(load_error=RuntimeError(marker))
    _fails_with(harness, LOAD_FAILED)
    [row] = harness.recorded
    assert (row["status"], row["error"], row["rows"]) == ("failed", LOAD_FAILED, None)
    assert marker not in json.dumps(row)
    assert any(marker in line for line in harness.log.lines)
    assert harness.registered == []


def test_a_load_with_failed_jobs_is_a_failure_and_never_a_success_with_no_count():
    harness = _Harness(has_failed_jobs=True, sink_rows=None)
    _fails_with(harness, LOAD_FAILED)
    [row] = harness.recorded
    assert (row["status"], row["error"]) == ("failed", LOAD_FAILED)
    assert harness.registered == []


def test_a_table_loaded_but_not_registered_records_that_reason_and_the_rows_the_sink_measured():
    harness = _Harness(register_error=RuntimeError("clickhouse says no"), sink_rows=2)
    _fails_with(harness, NOT_REGISTERED)
    [row] = harness.recorded
    assert (row["status"], row["error"], row["rows"]) == ("failed", NOT_REGISTERED, 2)
    assert len(harness.loads) == 1  # the data was written; only its catalog entry is missing


def test_a_sink_configuration_that_cannot_be_built_fails_before_anything_is_read():
    harness = _Harness(config_error=PermissionError("token file"))
    _fails_with(harness, LOAD_FAILED)
    assert harness.reads == []
    assert [row["error"] for row in harness.recorded] == [LOAD_FAILED]


def test_a_bug_in_the_job_still_records_its_one_row_with_a_reason_from_the_closed_set(monkeypatch):
    def broken(key: str) -> str:
        raise RuntimeError("a bug")

    monkeypatch.setattr(file_ingest, "_checked_key", broken)
    harness = _Harness()
    failure = _fails_with(harness, LOAD_FAILED)
    assert isinstance(failure.__cause__, RuntimeError)
    assert [row["error"] for row in harness.recorded] == [LOAD_FAILED]


def test_every_failure_leaves_one_row_and_its_error_is_always_one_of_the_closed_set():
    scenarios = [
        _Harness(read_error=OSError("x")),
        _Harness(raw=b"a\n"),
        _Harness(raw=b"\nid\n1\n"),  # a header row with no columns (review finding C2)
        _Harness(load_error=RuntimeError("x")),
        _Harness(register_error=RuntimeError("x")),
    ]
    for harness in scenarios:
        with pytest.raises(LoadFailure):
            harness.run()
        assert len(harness.recorded) == harness.record_attempts == 1
        assert harness.recorded[0]["error"] in FAILURE_REASONS


def test_when_the_outcome_cannot_be_recorded_the_run_fails_and_the_write_is_not_tried_twice():
    harness = _Harness(record_error=RuntimeError("clickhouse down"))
    with pytest.raises(OutcomeNotRecorded):
        harness.run()
    assert harness.record_attempts == 1

    failing = _Harness(raw=b"a\n", record_error=RuntimeError("clickhouse down"))
    with pytest.raises(OutcomeNotRecorded):
        failing.run()
    assert failing.record_attempts == 1


# ── Through the real sink, with dlt's pipeline faked ─────────────────────


class _FakeLoadInfo:
    has_failed_jobs = False


class _FakePipeline:
    def __init__(self, **kwargs) -> None:
        self.init_kwargs = kwargs
        self.last_trace = None
        self.state: dict = {}
        # The real pipeline's flag; `load_via_sink` asks it before every run (SRC-10 R3).
        self.has_pending_data = False
        # SRC-10 review BLOCKER 5: the history check looks at the schemas dlt holds (none here).
        self.schemas: dict = {}
        self.run_source = None
        self.table_name = None
        self.write_disposition = None
        self.materialized_rows: list[dict] = []
        self.run_batches: list = []
        self.extracted = None

    def sync_destination(self) -> None:
        pass

    # The retyping replace runs the three steps itself (`DATA-18`).
    def extract(self, source, table_name, table_format, write_disposition):
        self.run_source = source
        self.table_name = table_name
        self.write_disposition = write_disposition
        self.run_batches = list(source)
        self.materialized_rows = [row for batch in self.run_batches for row in batch.to_pylist()]

    def normalize(self) -> None:
        pass

    def load(self):
        return _FakeLoadInfo()

    def run(self, source, table_name, table_format, write_disposition):
        self.run_source = source
        self.table_name = table_name
        self.write_disposition = write_disposition
        # The job hands over Arrow batches, stamped by the sink: flatten to rows.
        self.run_batches = list(source)
        self.materialized_rows = [row for batch in self.run_batches for row in batch.to_pylist()]
        return _FakeLoadInfo()


@pytest.mark.parametrize("mode", ["replace", "append"])
def test_the_real_sink_replaces_or_appends_text_rows_that_the_job_hands_it(monkeypatch, mode):
    pipelines: list[_FakePipeline] = []

    def factory(**kwargs):
        pipelines.append(_FakePipeline(**kwargs))
        return pipelines[-1]

    monkeypatch.setattr("dispar_orchestrate.adapters.sink.dlt.pipeline", factory)
    monkeypatch.setattr(sink_module, "_install_catalog_env", lambda config: None)  # it sets process-wide env vars
    monkeypatch.setattr(sink_module, "_drop_table_if_types_differ", lambda *args: False)  # no catalog here

    harness = _Harness(raw=b"id,when\n1,2025-02-07T10:00:00Z\n2,x\n")
    summary = run_file_load(
        _params(load_mode=mode),
        log=harness.log,
        load_config=harness.load_config,
        open_object=harness.read,
        register=harness.register,
        write_report=harness.write_report,
        record=harness.record,
        now=harness.now,
    )

    [pipeline] = pipelines
    assert pipeline.write_disposition == mode
    assert pipeline.table_name == "g9_orders"
    assert {hints["data_type"] for hints in pipeline.run_source.columns.values()} == {"text"}
    assert len(pipeline.materialized_rows) == 2
    for row in pipeline.materialized_rows:
        assert set(row) == {"id", "when", "_ingested_at"}  # the sink stamps Bronze's own column
        assert row["_ingested_at"].tzinfo is not None
    assert all(table.schema.field("_ingested_at").type.tz == "UTC" for table in pipeline.run_batches)
    assert pipeline.materialized_rows[0]["when"] == "2025-02-07T10:00:00Z"
    # The fake pipeline has no trace, so the sink measured nothing: not the 2 the file held.
    assert summary.rows is None and harness.recorded[0]["rows"] is None


# ── The job in the code location ─────────────────────────────────────────

RUN_CONFIG = {
    "ops": {
        "ingest_uploaded_file": {
            "config": {
                "upload_id": "u-1",
                "storage_keys": [KEY],
                "format": "delimited",
                "bronze_table_name": "g9_orders",
                "load_mode": "append",
                "encoding": "utf-16",
                "delimiter": "\t",
                "header_row": 4,
                "columns": [],
                "on_bad_value": "stop",
            }
        }
    }
}


def test_the_job_is_registered_in_the_code_location_without_a_schedule_or_a_sensor():
    assert "file_ingest_job" in {job.name for job in defs.resolve_all_job_defs()}
    assert "file_ingest_job" not in {schedule.job_name for schedule in defs.schedules or []}
    assert "file_ingest_job" not in {
        getattr(target, "job_name", None) for sensor in defs.sensors or [] for target in sensor.targets
    }


def test_the_run_config_the_api_sends_reaches_the_load_as_the_same_settings(monkeypatch):
    seen: list[FileIngestParams] = []

    def fake_run(params, *, log):
        seen.append(params)
        return file_ingest.LoadSummary(table="g9_orders", rows=None, parsed_rows=2, columns=4, table_total=9)

    monkeypatch.setattr(file_ingest, "run_file_load", fake_run)
    result = file_ingest_job.execute_in_process(run_config=RUN_CONFIG)
    assert result.success
    assert seen == [
        FileIngestParams(
            upload_id="u-1",
            storage_keys=(KEY,),
            format="delimited",
            bronze_table_name="g9_orders",
            load_mode="append",
            encoding="utf-16",
            delimiter="\t",
            header_row=4,
            columns=(),
            on_bad_value="stop",
        )
    ]
    assert result.output_for_node("ingest_uploaded_file") == {
        "bronze_table_name": "g9_orders",
        "rows": None,
        "table_total": 9,
    }


@pytest.mark.parametrize("missing", list(CONFIG_SCHEMA))
def test_a_run_config_that_leaves_a_field_out_is_refused_at_launch_and_not_filled_with_a_guess(missing):
    config = {"ops": {"ingest_uploaded_file": {"config": dict(RUN_CONFIG["ops"]["ingest_uploaded_file"]["config"])}}}
    del config["ops"]["ingest_uploaded_file"]["config"][missing]
    with pytest.raises(DagsterInvalidConfigError):
        file_ingest_job.execute_in_process(run_config=config)


def test_a_failed_load_fails_the_run_after_one_attempt_and_is_never_retried(monkeypatch):
    attempts: list[int] = []

    def fake_run(params, *, log):
        attempts.append(1)
        raise LoadFailure(LOAD_FAILED)

    monkeypatch.setattr(file_ingest, "run_file_load", fake_run)
    result = file_ingest_job.execute_in_process(run_config=RUN_CONFIG, raise_on_error=False)

    assert not result.success
    assert len(attempts) == 1
    assert [e for e in result.all_events if e.event_type_value == "STEP_UP_FOR_RETRY"] == []
    [failure] = [e for e in result.all_events if e.event_type_value == "STEP_FAILURE"]
    assert LOAD_FAILED in failure.event_specific_data.error.message


def test_an_outcome_that_could_not_be_recorded_fails_the_run_after_one_attempt(monkeypatch):
    attempts: list[int] = []

    def fake_run(params, *, log):
        attempts.append(1)
        raise OutcomeNotRecorded("clickhouse down")

    monkeypatch.setattr(file_ingest, "run_file_load", fake_run)
    result = file_ingest_job.execute_in_process(run_config=RUN_CONFIG, raise_on_error=False)
    assert not result.success
    assert len(attempts) == 1
