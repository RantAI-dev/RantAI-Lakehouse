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
import json
import random
from pathlib import Path

import dlt
import pytest
from dagster import DagsterInvalidConfigError

from dispar_orchestrate import file_ingest
from dispar_orchestrate.adapters import sink as sink_module
from dispar_orchestrate.adapters.sink import LoadPlan, SinkConfig, SinkResult
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
    TOO_MANY_ROWS,
    UNREADABLE,
    FileIngestParams,
    LoadFailure,
    OutcomeNotRecorded,
    _checked_key,
    _column_names,
    _decode,
    _dlt_keeps_table_name,
    file_ingest_job,
    parse_file,
    read_stored_object,
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
        storage_key=KEY,
        bronze_table_name="g9_orders",
        load_mode="replace",
        encoding="utf-8",
        delimiter=",",
        header_row=0,
    )
    return dataclasses.replace(base, **overrides)


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
        self.log = _Log()
        self._ticks = iter(f"2026-10-02T10:00:{second:02d}+00:00" for second in range(60))

    def load_config(self) -> SinkConfig:
        if self.config_error:
            raise self.config_error
        return SINK_CONFIG

    def read(self, config: SinkConfig, key: str) -> bytes:
        self.reads.append(key)
        if self.read_error:
            raise self.read_error
        return self.raw

    def load(self, resource, table: str, config: SinkConfig, plan: LoadPlan) -> SinkResult:
        # The rows are consumed here, as dlt would, so a generator that is
        # read after the call returns would show up as an empty list.
        self.loads.append(
            {"table": table, "plan": plan, "columns": dict(resource.columns), "rows": list(resource)}
        )
        if self.load_error:
            raise self.load_error
        return SinkResult(rows=self.sink_rows, has_failed_jobs=self.has_failed_jobs, load_info_str="load info")

    def register(self, table: str, *, description: str, author: str) -> int:
        self.registered.append({"table": table, "description": description, "author": author})
        if self.register_error:
            raise self.register_error
        return self.total

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
            read_object=self.read,
            load=self.load,
            register=self.register,
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


def test_there_are_seven_different_failure_reasons_among_them_the_header_row_with_no_columns():
    # Review finding C2 made it seven: a header row with no columns has a
    # sentence of its own. The API counts the same seven against the same file.
    in_file = json.loads(REASONS_FILE.read_text(encoding="utf-8"))
    assert len(FAILURE_REASONS) == len(set(FAILURE_REASONS)) == len(in_file) == 7
    assert HEADER_NO_COLUMNS == "The header row has no columns."
    assert HEADER_NO_COLUMNS != HEADER_PAST_END


def test_the_row_cap_in_the_sentence_is_the_row_cap_the_job_enforces():
    assert file_ingest.MAX_ROWS == 2_000_000
    assert f"{file_ingest.MAX_ROWS:,}" in TOO_MANY_ROWS


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


def test_the_row_cap_is_a_failure_found_while_counting_and_a_file_at_the_cap_passes(monkeypatch):
    monkeypatch.setattr(file_ingest, "MAX_ROWS", 3)
    assert parse_file("a\n1\n2\n3\n", ",", 0).row_count == 3
    with pytest.raises(LoadFailure) as caught:
        parse_file("a\n1\n2\n3\n4\n", ",", 0)
    assert caught.value.reason == TOO_MANY_ROWS


def test_rows_are_a_lazy_pass_over_the_text_and_not_a_list_held_in_memory():
    parsed = parse_file("a\n1\n2\n", ",", 0)
    assert inspect.isgenerator(parsed.rows())
    assert list(parsed.rows()) == list(parsed.rows())


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
    _fails_with(harness, UNREADABLE, _params(storage_key=key))
    assert harness.reads == []
    assert harness.loads == []
    assert [row["error"] for row in harness.recorded] == [UNREADABLE]


def test_a_key_the_api_builds_passes_the_guard():
    assert _checked_key(KEY) == KEY
    assert _checked_key(f"uploads/{TENANT}/u-1") == f"uploads/{TENANT}/u-1"


def test_the_object_is_read_from_the_warehouse_bucket_with_the_sinks_credentials():
    seen: dict = {}

    class FakeFilesystem:
        def __init__(self, **kwargs) -> None:
            seen["kwargs"] = kwargs

        def cat_file(self, path: str) -> bytes:
            seen["path"] = path
            return b"bytes"

    assert read_stored_object(SINK_CONFIG, KEY, filesystem_factory=FakeFilesystem) == b"bytes"
    assert seen["path"] == f"lakehouse-warehouse/{KEY}"
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
        self.run_source = None
        self.table_name = None
        self.write_disposition = None
        self.materialized_rows: list[dict] = []

    def sync_destination(self) -> None:
        pass

    def run(self, source, table_name, table_format, write_disposition):
        self.run_source = source
        self.table_name = table_name
        self.write_disposition = write_disposition
        self.materialized_rows = list(source)
        return _FakeLoadInfo()


@pytest.mark.parametrize("mode", ["replace", "append"])
def test_the_real_sink_replaces_or_appends_text_rows_that_the_job_hands_it(monkeypatch, mode):
    pipelines: list[_FakePipeline] = []

    def factory(**kwargs):
        pipelines.append(_FakePipeline(**kwargs))
        return pipelines[-1]

    monkeypatch.setattr("dispar_orchestrate.adapters.sink.dlt.pipeline", factory)
    monkeypatch.setattr(sink_module, "_install_catalog_env", lambda config: None)  # it sets process-wide env vars

    harness = _Harness(raw=b"id,when\n1,2025-02-07T10:00:00Z\n2,x\n")
    summary = run_file_load(
        _params(load_mode=mode),
        log=harness.log,
        load_config=harness.load_config,
        read_object=harness.read,
        register=harness.register,
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
    assert pipeline.materialized_rows[0]["when"] == "2025-02-07T10:00:00Z"
    # The fake pipeline has no trace, so the sink measured nothing: not the 2 the file held.
    assert summary.rows is None and harness.recorded[0]["rows"] is None


# ── The job in the code location ─────────────────────────────────────────

RUN_CONFIG = {
    "ops": {
        "ingest_uploaded_file": {
            "config": {
                "upload_id": "u-1",
                "storage_key": KEY,
                "bronze_table_name": "g9_orders",
                "load_mode": "append",
                "encoding": "utf-16",
                "delimiter": "\t",
                "header_row": 4,
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
            storage_key=KEY,
            bronze_table_name="g9_orders",
            load_mode="append",
            encoding="utf-16",
            delimiter="\t",
            header_row=4,
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
