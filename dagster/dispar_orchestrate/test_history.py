"""The pure parts of `adapters/history.py` (`SRC-10` sixth pass, task H3): the row
hash, the key encoding, the versions one load writes, and the column typing. No
`dlt` run, no table, no network: rows in, rows out. Expected values are written from
the module's stated rules ("The hash" in its docstring), never copied from a run.
"""

from __future__ import annotations

from datetime import date, datetime, time, timedelta, timezone
from decimal import Decimal

import pyarrow as pa
import pytest

from dispar_orchestrate.adapters import dedup, history
from dispar_orchestrate.adapters.history import (
    HISTORY_COLUMNS,
    HistoryRefused,
    column_types,
    key_token,
    row_hash,
    to_arrow,
    versions_to_write,
)
from dispar_orchestrate.adapters.sink import LoadPlan, UnsupportedLoadMode

KEY = ("id",)
T1 = datetime(2026, 3, 1, 12, 0, tzinfo=timezone.utc)
T2 = T1 + timedelta(days=1)


def _h(**columns: object) -> str:
    return row_hash({"id": 1, **columns}, KEY)


# --- the hash ----------------------------------------------------------------


def test_a_null_an_empty_string_and_the_string_none_are_three_different_values() -> None:
    hashes = {_h(v=None), _h(v=""), _h(v="None"), _h(v="null")}
    # A null is skipped, so a row with a null equals a row without the column; the other
    # three are values and differ from it and from each other.
    assert len(hashes) == 4
    assert _h(v=None) == row_hash({"id": 1}, KEY)


def test_a_number_and_its_text_are_different_values() -> None:
    values = [1, "1", 1.0, True, Decimal("1")]
    assert len({_h(v=value) for value in values}) == len(values)
    assert _h(v=False) != _h(v=0)
    assert _h(v=Decimal("1.10")) == _h(v=Decimal("1.1")), "a decimal is compared by value, not by its scale"


def test_dates_and_timestamps_are_encoded_by_their_instant_and_kind() -> None:
    noon = datetime(2026, 1, 2, 12, 0, tzinfo=timezone.utc)
    plus_two = noon.astimezone(timezone(timedelta(hours=2)))
    assert _h(v=noon) == _h(v=plus_two), "one instant, however it is written"
    assert _h(v=noon) == _h(v=noon.replace(tzinfo=None)), "a naive time is taken as UTC, which is how it is stored"
    assert _h(v=noon) != _h(v=noon + timedelta(microseconds=1))
    kinds = [date(2026, 1, 2), noon, time(12, 0), "2026-01-02", "2026-01-02T12:00:00+00:00"]
    assert len({_h(v=value) for value in kinds}) == len(kinds)


def test_bytes_json_and_text_are_different_kinds_and_json_ignores_key_order() -> None:
    assert _h(v=b"ab") != _h(v="ab")
    assert _h(v={"a": 1, "b": [1, 2]}) == _h(v={"b": [1, 2], "a": 1})
    assert _h(v={"a": 1}) != _h(v='{"a":1}')
    assert _h(v=[1, 2]) != _h(v=[2, 1])


def test_the_columns_are_hashed_in_name_order_and_framed_so_two_rows_never_collide() -> None:
    assert row_hash({"id": 1, "a": "x", "b": "y"}, KEY) == row_hash({"b": "y", "a": "x", "id": 1}, KEY)
    assert _h(a="bc") != _h(ab="c"), "the boundary between a name and a value is part of the hash"
    assert _h(a="x", b="y") != _h(a="xy"), "and between two values"
    assert _h(a="x", b="y") != _h(a="y", b="x"), "a value belongs to its column"


def test_key_columns_and_the_columns_the_sink_and_dlt_own_are_not_hashed() -> None:
    base = row_hash({"id": 1, "v": "a"}, KEY)
    assert row_hash({"id": 2, "v": "a"}, KEY) == base
    for own in ("_ingested_at", "_dlt_id", "_dlt_load_id", *HISTORY_COLUMNS):
        assert row_hash({"id": 1, "v": "a", own: "anything"}, KEY) == base, own
    assert row_hash({"a": 1, "b": 2, "v": "a"}, ("a", "b")) == row_hash({"a": 9, "b": 8, "v": "a"}, ("a", "b"))


def test_a_column_added_at_the_source_changes_the_hash_only_of_a_row_that_has_a_value_in_it() -> None:
    before = {"id": 1, "v": "a"}
    assert row_hash({**before, "extra": None}, KEY) == row_hash(before, KEY), "a null in the new column: same version"
    assert row_hash({**before, "extra": float("nan")}, KEY) == row_hash(before, KEY), "NaN is no value, as in the dedup step"
    assert row_hash({**before, "extra": "x"}, KEY) != row_hash(before, KEY), "a value in it: a new version"
    assert row_hash({**before, "extra": ""}, KEY) != row_hash(before, KEY), "an empty string is a value"


def test_a_value_with_no_encoding_is_refused_naming_the_column_and_not_the_value() -> None:
    with pytest.raises(HistoryRefused) as raised:
        _h(v=object())
    assert "'v'" in str(raised.value) and "object at" not in str(raised.value)


def test_keys_meet_whatever_python_type_carried_them() -> None:
    aware = datetime(2026, 1, 2, tzinfo=timezone.utc)
    assert key_token({"k": aware}, ("k",)) == key_token({"k": aware.replace(tzinfo=None)}, ("k",))
    assert key_token({"k": Decimal("1.10")}, ("k",)) == key_token({"k": Decimal("1.1")}, ("k",))
    assert key_token({"k": 1}, ("k",)) != key_token({"k": "1"}, ("k",))
    assert key_token({"a": 1, "b": 2}, ("a", "b")) != key_token({"a": 2, "b": 1}, ("a", "b"))


# --- the versions of a load ------------------------------------------------------


def _current(row: dict, when: datetime = T1) -> dict:
    """A row as the table holds it after a load at `when`."""
    return {
        **row,
        "_ingested_at": when,
        "_valid_from": when,
        "_valid_to": None,
        "_is_current": True,
        "_row_hash": row_hash(row, KEY),
        "_dlt_id": "id-" + str(row["id"]),
        "_dlt_load_id": "1.0",
    }


def _table(*rows: dict) -> dict:
    return {key_token(row, KEY): row for row in map(_current, rows)}


def test_a_changed_row_is_closed_a_new_key_is_added_and_an_unchanged_row_is_left_alone() -> None:
    current = _table({"id": 1, "v": "a"}, {"id": 2, "v": "b"})
    source = [{"id": 1, "v": "a"}, {"id": 2, "v": "B"}, {"id": 3, "v": "c"}]
    versions = versions_to_write(current, source, KEY, T2)
    assert [r["id"] for r in versions.closed] == [2]
    closed = versions.closed[0]
    assert (closed["v"], closed["_valid_to"], closed["_is_current"]) == ("b", T2, False)
    assert closed["_valid_from"] == T1 and closed["_ingested_at"] == T1 and closed["_dlt_id"] == "id-2"
    assert [(r["id"], r["v"]) for r in versions.new] == [(2, "B"), (3, "c")]
    for row in versions.new:
        assert (row["_valid_from"], row["_ingested_at"], row["_valid_to"], row["_is_current"]) == (T2, T2, None, True)
        assert row["_row_hash"] == row_hash(row, KEY)
    assert all(r["id"] != 1 for r in versions.closed + versions.new), "the unchanged key gets nothing"


def test_an_identical_load_has_nothing_to_write() -> None:
    current = _table({"id": 1, "v": "a"}, {"id": 2, "v": "b"})
    versions = versions_to_write(current, [{"id": 2, "v": "b"}, {"id": 1, "v": "a"}], KEY, T2)
    assert (versions.closed, versions.new) == ([], [])


def test_a_new_row_never_carries_the_columns_the_sink_and_dlt_own_from_the_source() -> None:
    versions = versions_to_write({}, [{"id": 1, "v": "a", "_ingested_at": "x", "_dlt_id": "y"}], KEY, T2)
    assert versions.new[0]["_ingested_at"] == T2 and "_dlt_id" not in versions.new[0]


def test_a_row_that_changed_back_is_a_third_version_not_a_reopening() -> None:
    first = _current({"id": 1, "v": "a"})
    # The table now holds the second version "b" as current (the first one is closed elsewhere).
    second = _current({"id": 1, "v": "b"}, T2)
    versions = versions_to_write({key_token(second, KEY): second}, [{"id": 1, "v": "a"}], KEY, T2 + timedelta(days=1))
    assert [r["v"] for r in versions.closed] == ["b"]
    assert [r["v"] for r in versions.new] == ["a"]
    assert versions.new[0]["_valid_from"] == T2 + timedelta(days=1) != first["_valid_from"]
    assert versions.new[0]["_row_hash"] == first["_row_hash"], "the same values, a new version"


def test_a_key_that_is_no_longer_in_the_source_is_not_touched() -> None:
    current = _table({"id": 1, "v": "a"}, {"id": 2, "v": "b"})
    versions = versions_to_write(current, [{"id": 1, "v": "a"}], KEY, T2)
    assert (versions.closed, versions.new) == ([], [])


def test_a_composite_key_matches_on_every_column() -> None:
    key = ("a", "b")
    old = {"a": 1, "b": 2, "v": "x"}
    stored = {**old, "_row_hash": row_hash(old, key), "_valid_from": T1, "_is_current": True}
    versions = versions_to_write(
        {key_token(old, key): stored}, [{"a": 1, "b": 2, "v": "y"}, {"a": 2, "b": 1, "v": "x"}], key, T2
    )
    assert [(r["a"], r["b"]) for r in versions.closed] == [(1, 2)]
    assert [(r["a"], r["b"], r["v"]) for r in versions.new] == [(1, 2, "y"), (2, 1, "x")]


# --- column types -------------------------------------------------------------------


def test_the_types_of_the_source_columns_come_from_their_non_null_values() -> None:
    rows = [
        {"i": 1, "f": 1, "s": None, "d": date(2026, 1, 1), "n": None, "j": {"a": 1}},
        {"i": 2, "f": 1.5, "s": "x", "d": None, "n": None, "j": [1]},
    ]
    types = column_types(rows)
    assert types["i"] == pa.int64() and types["f"] == pa.float64() and types["s"] == pa.string()
    assert types["d"] == pa.date32() and types["j"] == pa.string()
    assert "n" not in types, "a column null in every row is not created"


def test_a_column_with_values_of_two_kinds_is_refused_by_name() -> None:
    with pytest.raises(HistoryRefused, match="'v'"):
        column_types([{"v": 1}, {"v": "x"}])


def test_a_value_that_does_not_fit_its_column_is_refused_naming_the_column_not_the_value() -> None:
    schema = pa.schema([pa.field("id", pa.int64()), pa.field("v", pa.int64())])
    with pytest.raises(HistoryRefused) as raised:
        to_arrow([{"id": 1, "v": "secret text"}], schema, table="t")
    assert str(raised.value) == "table 't': a value does not fit the type of column 'v'"
    required = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    with pytest.raises(HistoryRefused, match="column 'id' is required"):
        to_arrow([{"id": None}], required, table="t")


def test_json_values_are_kept_as_json_text() -> None:
    table = to_arrow([{"j": {"b": 1, "a": [1]}}], pa.schema([pa.field("j", pa.string())]), table="t")
    assert table.column("j").to_pylist() == ['{"a":[1],"b":1}']


# --- the plan -------------------------------------------------------------------


def test_a_history_plan_needs_a_key_takes_no_cursor_and_cannot_key_on_a_column_it_adds() -> None:
    assert LoadPlan(mode="history", key=("id",)).key == ("id",)
    with pytest.raises(UnsupportedLoadMode, match="primaryKey"):
        LoadPlan(mode="history")
    with pytest.raises(UnsupportedLoadMode, match="takes no incrementalKey"):
        LoadPlan(mode="history", key=("id",), cursor="updated_at")
    for reserved in HISTORY_COLUMNS:
        with pytest.raises(UnsupportedLoadMode, match="the columns the mode adds"):
            LoadPlan(mode="history", key=("id", reserved))


def test_a_history_source_object_gives_a_plan_with_its_key_and_no_cursor() -> None:
    plan = LoadPlan.from_source_object(
        {"loadMode": "history", "primaryKey": ["id", "line"], "incrementalKey": "  "}, refresh="remove"
    )
    assert plan == LoadPlan(mode="history", key=("id", "line"), cursor=None, refresh="remove")
    # A key kept on a table in another mode is ignored, as for the deduplicating modes.
    assert LoadPlan.from_source_object({"loadMode": "replace", "primaryKey": ["id"]}).key == ()


def test_a_history_refusal_is_a_dedup_refusal_so_it_is_a_rejected_run_and_a_non_retryable_failure() -> None:
    assert issubclass(history.HistoryRefused, dedup.DedupRefused) and issubclass(history.HistoryRefused, ValueError)
