"""The job's caster against the shared cases of `ops/fixtures/uploads/types/`
(`DATA-18` section 8, item 10: the vectorised casts give the fixtures' answers
exactly) and the pieces around it. No network.
"""

from __future__ import annotations

import json
from pathlib import Path

import pyarrow as pa
import pytest

from dispar_orchestrate.upload_types import (
    BadReport,
    ColumnSpec,
    cast_batch,
    cast_column,
    canonical,
    check_specs,
)

TYPES_DIR = Path(__file__).resolve().parents[2] / "ops" / "fixtures" / "uploads" / "types"


def _cases(name: str) -> list[dict]:
    return json.loads((TYPES_DIR / name).read_text(encoding="utf-8"))["cases"]


def _spec(case: dict) -> ColumnSpec:
    return ColumnSpec(
        source="c",
        name="c",
        type=case["type"],
        date_format=case.get("dateFormat") or "",
        timezone=bool(case.get("timezone")),
    )


@pytest.mark.parametrize("case", _cases("cast.json"), ids=lambda c: f"{c['type']}:{c['dateFormat']}:{c['value']!r}")
def test_each_value_of_the_shared_cast_cases_fits_and_is_stored_as_the_fixture_says(case):
    spec = _spec(case)
    text = pa.array([case["value"]], pa.string())
    cast = cast_column(text, spec)
    got = canonical(cast.to_pylist()[0], spec)
    assert got == case["cast"], case
    # `ok` is whether the value fits: null for a non-empty value is a value that did not.
    empty = case["value"].strip() == ""
    fits = empty or cast.to_pylist()[0] is not None or spec.type == "text"
    assert fits == case["ok"], case


def test_the_whole_shared_set_cast_as_one_column_gives_the_same_answers_as_one_at_a_time():
    for type_, date_format, tz in {(c["type"], c["dateFormat"], bool(c["timezone"])) for c in _cases("cast.json")}:
        cases = [c for c in _cases("cast.json") if (c["type"], c["dateFormat"], bool(c["timezone"])) == (type_, date_format, tz)]
        spec = _spec(cases[0])
        batch = cast_column(pa.array([c["value"] for c in cases], pa.string()), spec).to_pylist()
        assert [canonical(v, spec) for v in batch] == [c["cast"] for c in cases], (type_, date_format)


def test_the_arrow_types_and_hints_match_what_a_local_iceberg_table_was_measured_to_hold():
    expected = {
        ("integer", False): ("int64", "long"),
        ("decimal", False): ("decimal128(38, 9)", "decimal(38, 9)"),
        ("boolean", False): ("bool", "boolean"),
        ("date", False): ("date32[day]", "date"),
        ("timestamp", False): ("timestamp[us]", "timestamp"),
        ("timestamp", True): ("timestamp[us, tz=UTC]", "timestamptz"),
        ("text", False): ("string", "string"),
        ("float", False): ("double", "double"),
    }
    for (type_, tz), (arrow, iceberg) in expected.items():
        spec = ColumnSpec("c", "c", type_, "iso" if type_ in ("date", "timestamp") else "", tz)
        assert str(spec.arrow_type()) == arrow
        assert spec.iceberg_type() == iceberg


def test_a_batch_is_cast_by_the_column_list_and_the_bad_values_are_counted_and_listed():
    specs = [
        ColumnSpec("Qty", "qty", "integer", "", False),
        ColumnSpec("Due", "due", "date", "iso", False),
        ColumnSpec("Note", "note", "text", "", False),
    ]
    batch = pa.table(
        {
            "a": ["1", "abc", "", "4"],
            "b": ["2026-10-16", "2026-02-30", "x", ""],
            "c": ["p", "", "q", "r"],
        }
    )
    report = BadReport(["qty", "due", "note"])
    typed = cast_batch(batch, specs, first_row=10, report=report)
    assert typed.column_names == ["qty", "due", "note"]
    assert typed.schema.types == [pa.int64(), pa.date32(), pa.string()]
    assert typed.column("qty").to_pylist() == [1, None, None, 4]
    assert typed.column("note").to_pylist() == ["p", "", "q", "r"], "text keeps an empty text"
    assert report.count == 3
    assert report.as_json()["byColumn"] == [{"column": "qty", "count": 1}, {"column": "due", "count": 2}]
    assert report.as_json()["rows"][0] == {"row": 11, "column": "qty", "value": "abc"}


def test_a_report_keeps_the_first_hundred_and_cuts_a_long_value():
    report = BadReport(["n"])
    values = pa.array(["x" * 500] * 150, pa.string())
    report.add(0, 1, list(range(150)), values)
    out = report.as_json()
    assert out["count"] == 150 and len(out["rows"]) == 100
    assert len(out["rows"][0]["value"]) == 200


def test_an_integer_past_64_bits_is_a_bad_value_and_the_rest_of_the_column_is_kept():
    spec = ColumnSpec("n", "n", "integer", "", False)
    got = cast_column(pa.array(["1", "9223372036854775808", "-9223372036854775809", "3"], pa.string()), spec)
    assert got.to_pylist() == [1, None, None, 3]


def test_a_column_list_is_checked_the_way_the_api_checks_it():
    good = [ColumnSpec("a", "a", "integer", "", False), ColumnSpec("b", "b", "timestamp", "dmy", True)]
    assert check_specs(good) == []
    bad = [
        ColumnSpec("A", "A", "integer", "", False),
        ColumnSpec("a", "a_", "text", "", False),
        ColumnSpec("c", "c", "date", "", False),
        ColumnSpec("c", "c", "integer", "iso", False),
        ColumnSpec("d", "d", "integer", "", True),
        ColumnSpec("e", "e", "time", "", False),
    ]
    problems = " | ".join(check_specs(bad))
    for needle in ("'A' breaks", "'a_' breaks", "'c' is used twice", "date reading ''", "only a timestamp", "unknown type 'time'"):
        assert needle in problems, (needle, problems)
