"""Tests for `column_gate.py`, the Python port of `cdc.rs`'s
`reject_unsupported_column_types` (WS3, X9), plus
`reject_unsupported_column_types_from_sample` for the document-shaped
`mongodb`/`kafka` adapters. The `reject_unsupported_column_types` cases
here each mirror a case in `rust/crates/lakehouse-store/src/cdc.rs`'s own
`mod tests` (read in full before editing either side) -- the two suites
are pinned to the same inputs so a change to one gate's behavior that
isn't mirrored in the other shows up as a failing test here, not as
silent drift. `reject_unsupported_column_types_from_sample` has no Rust
counterpart (Mongo/Kafka carry no relational type catalogue for `cdc.rs`
to gate in the first place -- see `column_gate.py`'s module docstring),
so its cases are pinned only against this module's own fixtures below.
"""

from __future__ import annotations

import pytest

from dispar_orchestrate.column_gate import (
    NESTED_TYPE_MARKERS,
    UnsupportedColumnType,
    reject_unsupported_column_types,
    reject_unsupported_column_types_from_sample,
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


def test_reject_unsupported_column_types_covers_a_mongo_document_with_a_nested_array() -> None:
    # Mongo has no fixed schema -- the gate runs over a SAMPLED document's
    # inferred field types (BSON type names decoded by pymongo as native
    # Python list/dict), the same NESTED_TYPE_MARKERS rule cdc.rs:79
    # already enforces for a relational nested/array column.
    sample_doc = {"id": 1, "tags": ["a", "b"]}  # array -- unsupported, same rule as a Postgres ARRAY column
    with pytest.raises(UnsupportedColumnType) as exc_info:
        reject_unsupported_column_types_from_sample(sample_doc)
    assert exc_info.value.column == "tags"
    assert exc_info.value.type_name == "list"


def test_reject_unsupported_column_types_accepts_a_flat_mongo_document() -> None:
    reject_unsupported_column_types_from_sample({"id": 1, "name": "a", "active": True})  # must not raise


def test_kafka_message_schema_gate_rejects_a_nested_object_value() -> None:
    # WS9-specific: a Kafka message's JSON value can carry arbitrary
    # nesting -- the SAME marker-based rule applies to its decoded shape.
    with pytest.raises(UnsupportedColumnType) as exc_info:
        reject_unsupported_column_types_from_sample({"id": 1, "meta": {"nested": True}})
    assert exc_info.value.column == "meta"
    assert exc_info.value.type_name == "dict"


def test_kafka_message_schema_gate_accepts_a_flat_decoded_message() -> None:
    reject_unsupported_column_types_from_sample({"id": 1, "topic": "orders", "amount": 12.5})  # must not raise


def test_reject_unsupported_column_types_from_sample_reports_the_first_offending_key() -> None:
    # Fail-fast on the first nested value found, in dict iteration
    # (insertion) order -- mirrors reject_unsupported_column_types's own
    # first-offending-column behavior.
    sample_doc = {"ok": 1, "bad": ["x"], "also_bad": {"y": 1}}
    with pytest.raises(UnsupportedColumnType) as exc_info:
        reject_unsupported_column_types_from_sample(sample_doc)
    assert exc_info.value.column == "bad"
