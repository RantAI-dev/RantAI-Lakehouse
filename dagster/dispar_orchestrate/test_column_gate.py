"""Tests for `column_gate.py`, the Python port of `cdc.rs`'s
`reject_unsupported_column_types` (WS3, X9). Every case here mirrors a
case in `rust/crates/lakehouse-store/src/cdc.rs`'s own `mod tests` (read
in full before editing either side) -- the two suites are pinned to the
same inputs so a change to one gate's behavior that isn't mirrored in the
other shows up as a failing test here, not as silent drift.
"""

from __future__ import annotations

import pytest

from dispar_orchestrate.column_gate import (
    NESTED_TYPE_MARKERS,
    UnsupportedColumnType,
    reject_unsupported_column_types,
)


def test_scalar_columns_are_accepted() -> None:
    # Mirrors cdc.rs's `scalar_columns_are_accepted`.
    columns = [
        ("id", "bigint"),
        ("amount", "numeric(12,2)"),
        ("payload", "jsonb"),
        ("created_at", "timestamptz"),
        ("label", "character varying"),
    ]
    reject_unsupported_column_types(columns)  # must not raise


def test_postgres_array_display_is_rejected() -> None:
    # Mirrors cdc.rs's `postgres_array_display_is_rejected`.
    with pytest.raises(UnsupportedColumnType) as exc_info:
        reject_unsupported_column_types([("tags", "text[]")])
    assert exc_info.value.column == "tags"


def test_the_shipped_cdc_source_schema_passes_the_gate() -> None:
    # Mirrors cdc.rs's `the_shipped_cdc_source_schema_passes_the_r7_gate`:
    # docker-compose.yml's `CREATE TABLE p5_cdc.orders`.
    columns = [
        ("id", "bigint"),
        ("customer", "text"),
        ("amount", "numeric(12,2)"),
        ("updated_at", "timestamptz"),
    ]
    reject_unsupported_column_types(columns)  # must not raise


def test_adding_a_nested_column_to_that_schema_is_refused() -> None:
    # Mirrors cdc.rs's `adding_a_nested_column_to_that_schema_is_refused`.
    columns = [
        ("id", "bigint"),
        ("customer", "text"),
        ("metadata", "jsonb[]"),
    ]
    with pytest.raises(UnsupportedColumnType) as exc_info:
        reject_unsupported_column_types(columns)
    assert exc_info.value.column == "metadata"


def test_nested_struct_is_rejected() -> None:
    # Mirrors cdc.rs's `nested_struct_is_rejected`.
    for type_name in ["struct", "record", "composite", "map<string,string>", "ROW"]:
        with pytest.raises(UnsupportedColumnType):
            reject_unsupported_column_types([("nested", type_name)])


def test_first_offending_column_is_reported() -> None:
    # Mirrors cdc.rs's `first_offending_column_is_reported`: fail-fast on
    # the first match, column order as given.
    columns = [
        ("ok", "int"),
        ("bad", "struct"),
        ("also_bad", "array"),
    ]
    with pytest.raises(UnsupportedColumnType) as exc_info:
        reject_unsupported_column_types(columns)
    assert exc_info.value.column == "bad"


def test_nested_type_markers_matches_the_rust_gate() -> None:
    # Pins this list against `rust/crates/lakehouse-store/src/cdc.rs:62`'s
    # `NESTED_TYPE_MARKERS` literal -- see that constant's own comment for
    # why this is a literal-string pin, not a cross-language import (no
    # such import exists in this workspace).
    assert NESTED_TYPE_MARKERS == ("struct", "record", "array", "map", "composite", "row")
