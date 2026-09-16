"""R7 (docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md) gate: reject a nested
struct/array/map source column before Bronze ever writes it.

This is a verbatim port of `rust/crates/lakehouse-store/src/cdc.rs`'s
`reject_unsupported_column_types` -- same marker list, same array-display
check, same fail-fast-on-first-match behavior -- so the two gates cannot
drift (WS3 plan judge review X9). The Rust original lives at
`rust/crates/lakehouse-store/src/cdc.rs:79` (`NESTED_TYPE_MARKERS` at
`cdc.rs:62`); a change to either side's marker list or matching rule must
be made to BOTH sides in the same commit, and both `test_column_gate.py`
(this module's test) and `cdc.rs`'s own `mod tests` pin the same fixture
inputs so a one-sided change fails a test immediately.
"""

from __future__ import annotations

# Ported verbatim from rust/crates/lakehouse-store/src/cdc.rs:62 -- keep
# these two lists identical; a Rust-side change to NESTED_TYPE_MARKERS
# must be mirrored here in the SAME commit.
NESTED_TYPE_MARKERS: tuple[str, ...] = ("struct", "record", "array", "map", "composite", "row")


class UnsupportedColumnType(Exception):
    """A column whose type this connector contract cannot propagate to
    Bronze -- mirrors `cdc.rs`'s `UnsupportedColumnType` error, including
    its message text.
    """

    def __init__(self, column: str, type_name: str) -> None:
        self.column = column
        self.type_name = type_name
        super().__init__(
            f"column {column!r} has type {type_name!r}, which ClickHouse cannot read as "
            "Iceberg nested data (R7): reject this connector at registration, not after "
            "Bronze already has unreadable data"
        )


def reject_unsupported_column_types(columns: list[tuple[str, str]]) -> None:
    """Reject a source schema containing a nested struct/array/map column,
    mirroring `cdc.rs`'s `reject_unsupported_column_types` exactly.

    Each item in `columns` is `(name, type_name)`, matching `cdc.rs`'s
    `SourceColumn { name, type_name }`.

    # Errors

    Raises [`UnsupportedColumnType`] on the FIRST nested column found,
    column order as given -- matching `cdc.rs`'s fail-fast (return on
    first match) behavior exactly. Does not aggregate every offending
    column.
    """
    for name, type_name in columns:
        lower = type_name.lower()
        is_array_display = lower.rstrip().endswith("[]")
        has_marker = any(marker in lower for marker in NESTED_TYPE_MARKERS)
        if is_array_display or has_marker:
            raise UnsupportedColumnType(name, type_name)
