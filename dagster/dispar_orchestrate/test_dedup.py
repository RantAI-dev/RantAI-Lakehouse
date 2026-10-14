"""The pure deduplication of `adapters/dedup.py` (`SRC-10` plan task 3, planner
decisions 2 and 6 of the second pass). No `dlt` run, no network: rows in, rows out.
"""

from __future__ import annotations

from datetime import datetime, timezone

import pytest

from dispar_orchestrate.adapters import dedup
from dispar_orchestrate.adapters.dedup import (
    DedupLimits,
    DedupRefused,
    IncomparableCursor,
    MissingKeyValue,
    TooManyKeys,
    UnsupportedForDedup,
    Winners,
    dedup_rows,
    deduplicate,
)


def _ids(rows: list[dict], column: str = "v") -> list:
    return [row[column] for row in rows]


def test_the_row_with_the_highest_cursor_value_wins_whatever_order_the_rows_arrive_in() -> None:
    rows = [
        {"id": 1, "v": "mid", "seq": 5},
        {"id": 1, "v": "high", "seq": 9},
        {"id": 1, "v": "low", "seq": 1},
    ]
    assert _ids(deduplicate(rows, key=("id",), cursor="seq", table="t")) == ["high"]


def test_a_tie_on_the_cursor_goes_to_the_row_read_last() -> None:
    rows = [{"id": 1, "v": "first", "seq": 5}, {"id": 1, "v": "second", "seq": 5}]
    assert _ids(deduplicate(rows, key=("id",), cursor="seq", table="t")) == ["second"]


def test_without_a_cursor_the_row_read_last_wins() -> None:
    rows = [{"id": 1, "v": "first"}, {"id": 2, "v": "x"}, {"id": 1, "v": "last"}]
    assert _ids(deduplicate(rows, key=("id",), cursor=None, table="t")) == ["last", "x"]


def test_a_key_keeps_the_position_of_its_first_appearance() -> None:
    rows = [{"id": 2, "v": "b"}, {"id": 1, "v": "a"}, {"id": 2, "v": "B"}]
    assert [r["id"] for r in deduplicate(rows, key=("id",), cursor=None, table="t")] == [2, 1]


def test_a_null_cursor_loses_to_any_value_in_either_order() -> None:
    low_first = [{"id": 1, "v": "real", "seq": -10}, {"id": 1, "v": "null", "seq": None}]
    null_first = list(reversed(low_first))
    assert _ids(deduplicate(low_first, key=("id",), cursor="seq", table="t")) == ["real"]
    assert _ids(deduplicate(null_first, key=("id",), cursor="seq", table="t")) == ["real"]


def test_two_null_cursors_tie_and_the_later_row_wins() -> None:
    rows = [{"id": 1, "v": "a", "seq": None}, {"id": 1, "v": "b"}]  # the second has no cursor column at all
    assert _ids(deduplicate(rows, key=("id",), cursor="seq", table="t")) == ["b"]


def test_a_float_nan_cursor_counts_as_null() -> None:
    rows = [{"id": 1, "v": "real", "seq": 1.5}, {"id": 1, "v": "nan", "seq": float("nan")}]
    assert _ids(deduplicate(rows, key=("id",), cursor="seq", table="t")) == ["real"]


def test_cursor_values_of_different_numeric_types_compare_by_value() -> None:
    rows = [{"id": 1, "v": "int", "seq": 3}, {"id": 1, "v": "float", "seq": 2.5}, {"id": 1, "v": "bool", "seq": True}]
    assert _ids(deduplicate(rows, key=("id",), cursor="seq", table="t")) == ["int"]


def test_cursor_values_that_cannot_be_ordered_fail_with_a_fixed_message_and_no_value() -> None:
    rows = [{"id": 1, "seq": "SECRET-7"}, {"id": 1, "seq": 3}]
    with pytest.raises(IncomparableCursor) as raised:
        deduplicate(rows, key=("id",), cursor="seq", table="orders")
    assert "SECRET-7" not in str(raised.value)
    assert "'orders'" in str(raised.value) and "'seq'" in str(raised.value)


def test_a_naive_and_a_zone_aware_time_cannot_be_ordered_either() -> None:
    rows = [
        {"id": 1, "seq": datetime(2026, 1, 1)},
        {"id": 1, "seq": datetime(2026, 1, 2, tzinfo=timezone.utc)},
    ]
    with pytest.raises(IncomparableCursor):
        deduplicate(rows, key=("id",), cursor="seq", table="t")


def test_a_composite_key_matches_on_every_column() -> None:
    rows = [
        {"a": 1, "b": 1, "v": "x"},
        {"a": 1, "b": 2, "v": "y"},
        {"a": 1, "b": 2, "v": "Y"},
        {"a": 2, "b": 1, "v": "z"},
    ]
    assert _ids(deduplicate(rows, key=("a", "b"), cursor=None, table="t")) == ["x", "Y", "z"]


def test_the_string_one_and_the_number_one_are_different_keys() -> None:
    rows = [{"id": 1, "v": "number"}, {"id": "1", "v": "string"}]
    assert _ids(deduplicate(rows, key=("id",), cursor=None, table="t")) == ["number", "string"]


def test_a_key_value_that_is_a_json_array_is_a_key_too() -> None:
    rows = [{"id": [1, 2], "v": "a"}, {"id": [1, 2], "v": "b"}, {"id": [2, 1], "v": "c"}]
    assert _ids(deduplicate(rows, key=("id",), cursor=None, table="t")) == ["b", "c"]


def test_rows_whose_key_is_null_or_absent_fail_the_load_with_the_count_and_never_a_row() -> None:
    rows = [
        {"id": 1, "v": "ok", "secret": "SECRET-ROW-DATA"},
        {"id": None, "v": "null", "secret": "SECRET-ROW-DATA"},
        {"v": "absent", "secret": "SECRET-ROW-DATA"},
        {"id": float("nan"), "v": "nan"},
    ]
    with pytest.raises(MissingKeyValue) as raised:
        deduplicate(rows, key=("id",), cursor=None, table="orders")
    message = str(raised.value)
    assert message.startswith("table 'orders': 3 row(s) have no value for key column 'id'")
    assert "SECRET-ROW-DATA" not in message and "null" not in message.replace("no value", "")


def test_a_composite_key_names_every_column_that_had_a_null() -> None:
    rows = [{"a": None, "b": 1}, {"a": 1, "b": None}, {"a": 1, "b": 1}]
    with pytest.raises(MissingKeyValue) as raised:
        deduplicate(rows, key=("a", "b"), cursor=None, table="t")
    assert "2 row(s)" in str(raised.value) and "'a', 'b'" in str(raised.value)


def test_an_empty_string_is_a_key_value_not_a_missing_one() -> None:
    assert len(deduplicate([{"id": "", "v": 1}], key=("id",), cursor=None, table="t")) == 1


def test_a_row_that_is_not_a_record_is_refused() -> None:
    with pytest.raises(UnsupportedForDedup):
        deduplicate([[1, 2, 3]], key=("id",), cursor=None, table="t")  # type: ignore[list-item]


def test_memory_is_the_number_of_distinct_keys_not_the_number_of_rows() -> None:
    winners = Winners(key=("id",), cursor="seq", table="t", limits=DedupLimits(max_keys=10))
    for n in range(10_000):
        winners.add({"id": n % 10, "seq": n})
    assert len(winners._best) == 10  # noqa: SLF001 -- the claim of the module docstring
    assert [r["seq"] for r in winners.result()] == list(range(9_990, 10_000))


def test_more_distinct_keys_than_the_bound_fail_naming_the_setting() -> None:
    rows = [{"id": n} for n in range(6)]
    with pytest.raises(TooManyKeys) as raised:
        deduplicate(rows, key=("id",), cursor=None, table="t", limits=DedupLimits(max_keys=5))
    assert "INGEST_DEDUP_MAX_ROWS" in str(raised.value) and "'t'" in str(raised.value)
    # Exactly the bound is fine, and a repeated key adds nothing.
    assert len(deduplicate([*rows[:5], rows[0]], key=("id",), cursor=None, table="t", limits=DedupLimits(5))) == 5


def test_the_bound_is_read_from_the_environment_with_two_million_as_the_default(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("INGEST_DEDUP_MAX_ROWS", raising=False)
    assert DedupLimits.from_env().max_keys == 2_000_000
    monkeypatch.setenv("INGEST_DEDUP_MAX_ROWS", " 1234 ")
    assert DedupLimits.from_env().max_keys == 1234
    monkeypatch.setenv("INGEST_DEDUP_MAX_ROWS", "")
    assert DedupLimits.from_env().max_keys == 2_000_000


@pytest.mark.parametrize("value", ["abc", "0", "-5", "1.5"])
def test_a_bound_that_is_not_a_whole_number_of_at_least_one_fails_closed(
    monkeypatch: pytest.MonkeyPatch, value: str
) -> None:
    monkeypatch.setenv("INGEST_DEDUP_MAX_ROWS", value)
    with pytest.raises(DedupRefused, match="INGEST_DEDUP_MAX_ROWS"):
        DedupLimits.from_env()


def test_the_generator_form_yields_nothing_until_the_rows_are_exhausted_and_raises_then() -> None:
    seen: list[int] = []

    def rows():
        for n in (1, 2):
            seen.append(n)
            yield {"id": n}
        yield {"id": None}

    out = dedup_rows(rows(), key=("id",), cursor=None, table="t")
    with pytest.raises(MissingKeyValue):
        next(out)
    assert seen == [1, 2], "the whole read happened before the refusal"


def test_the_module_exports_what_the_sink_and_the_factory_import() -> None:
    assert issubclass(dedup.DedupRefused, ValueError)
    assert all(issubclass(c, DedupRefused) for c in (MissingKeyValue, TooManyKeys, IncomparableCursor))
