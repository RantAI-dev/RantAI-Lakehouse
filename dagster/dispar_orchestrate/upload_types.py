"""Column types for an uploaded file: the job's half of the rules (`DATA-18`,
plan AD3, feature page `docs/core/features/upload-column-types.md`).

The API guesses and checks on a sample (`rust/crates/lakehouse-api/src/
upload_types.rs`); this module casts the whole file. Both follow one set of
rules, pinned by the cases of `ops/fixtures/uploads/types/` which a test on each
side reads (ADR 0014 decision 3, extended to types): `cast.json` says, for a
value, a type, a date reading and a time-zone flag, whether it fits and what is
stored.

# How the cast is done

Vectorised, with `pyarrow.compute`, a batch at a time: a column of text is
trimmed, an empty cell becomes null, a value is checked against the type's
syntax with a regular expression and then cast; a value that does not fit comes
out null and, being non-empty, is counted as bad. Nothing is done row by row
except for one case the vector cast cannot report per value, an integer past 64
bits, and, in `BadReport`, to list the first 100 failures (their positions are
known from the masks).

# The rules (decisions 2 to 5 and 7 of the page)

Empty (or all-space) is null for every type but text, which keeps what it has;
spaces around a typed value are trimmed. Integer: a sign and digits (leading zeros
accepted here: the guess refuses them, a person's choice is respected), 64 bits.
Decimal: at most 29 digits before the point and 9 after (`decimal(38, 9)`); `1,234`
and `1e3` do not fit. Boolean: `true`/`false`, `yes`/`no`, `y`/`n`, `t`/`f` in any
case, and `1`/`0`. Date: ISO `YYYY-MM-DD`, or day-first / month-first with `/`, `-` or
`.` (one kind in a value, a four-digit year), as the column's reading says. Timestamp:
a date, `T` or a space, `HH:MM[:SS[.f]]` with a fraction of at most six digits, and a
zone (`Z`, `+HH:MM`, `+HHMM`) exactly when the column holds instants, which are stored
in UTC. Float: the syntax of a floating-point number, `NaN` and `inf` included.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from typing import Any

import pyarrow as pa
import pyarrow.compute as pc

TYPES = ("text", "integer", "decimal", "boolean", "date", "timestamp", "float")
DATE_FORMATS = ("iso", "dmy", "mdy")

# What a report keeps (`AD6`): the first 100 failing values, each cut to 200 characters.
MAX_LISTED = 100
MAX_VALUE_CHARS = 200

DECIMAL = pa.decimal128(38, 9)

# Column names a person may give (decision 8), as the API checks them.
COLUMN_NAME = re.compile(r"[a-z][a-z0-9]*(_[a-z0-9]+)*")
MAX_NAME_CHARS = 128


@dataclass(frozen=True)
class ColumnSpec:
    """One column of a load, as the API sends it in the run configuration."""

    source: str
    name: str
    type: str
    #: `iso`, `dmy`, `mdy`, or `""` for a type that has no date reading.
    date_format: str
    #: For a timestamp: whether its values carry a zone (and are stored as instants).
    timezone: bool

    def arrow_type(self) -> pa.DataType:
        """The Arrow type the column is written as (and so the Iceberg type)."""
        return {
            "text": pa.string(),
            "integer": pa.int64(),
            "decimal": DECIMAL,
            "boolean": pa.bool_(),
            "date": pa.date32(),
            "timestamp": pa.timestamp("us", tz="UTC") if self.timezone else pa.timestamp("us"),
            "float": pa.float64(),
        }[self.type]

    def hint(self) -> dict[str, Any]:
        """The dlt column hint that matches `arrow_type` (dlt 1.30 has `precision`,
        `scale` and `timezone`; measured: a local Iceberg table comes out `long`,
        `decimal(38, 9)`, `boolean`, `date`, `timestamp`/`timestamptz`, `string`)."""
        if self.type == "text":
            return {"name": self.name, "data_type": "text"}
        if self.type == "integer":
            return {"name": self.name, "data_type": "bigint"}
        if self.type == "decimal":
            return {"name": self.name, "data_type": "decimal", "precision": 38, "scale": 9}
        if self.type == "boolean":
            return {"name": self.name, "data_type": "bool"}
        if self.type == "date":
            return {"name": self.name, "data_type": "date"}
        if self.type == "timestamp":
            return {"name": self.name, "data_type": "timestamp", "timezone": self.timezone}
        return {"name": self.name, "data_type": "double"}

    def iceberg_type(self) -> str:
        """The Iceberg type name the column is stored as."""
        return {
            "text": "string",
            "integer": "long",
            "decimal": "decimal(38, 9)",
            "boolean": "boolean",
            "date": "date",
            "timestamp": "timestamptz" if self.timezone else "timestamp",
            "float": "double",
        }[self.type]


def specs_from_config(columns: list[dict[str, Any]]) -> list[ColumnSpec]:
    """The specs out of a run's `columns` (already checked against the config schema)."""
    return [
        ColumnSpec(
            source=c["source"], name=c["name"], type=c["type"], date_format=c["date_format"], timezone=c["timezone"]
        )
        for c in columns
    ]


def check_specs(specs: list[ColumnSpec]) -> list[str]:
    """What is wrong with a column list, as sentences for the run log (the API has
    checked the same rules; this is for a launch that did not come through it)."""
    problems: list[str] = []
    seen: set[str] = set()
    for spec in specs:
        if not COLUMN_NAME.fullmatch(spec.name) or len(spec.name) > MAX_NAME_CHARS or spec.name.startswith("_dlt"):
            problems.append(f"column name {spec.name!r} breaks the naming rule")
        if spec.name in seen:
            problems.append(f"column name {spec.name!r} is used twice")
        seen.add(spec.name)
        if spec.type not in TYPES:
            problems.append(f"column {spec.name!r} has the unknown type {spec.type!r}")
        dated = spec.type in ("date", "timestamp")
        if dated != (spec.date_format in DATE_FORMATS):
            problems.append(f"column {spec.name!r}: a date reading {spec.date_format!r} does not go with type {spec.type!r}")
        if spec.type != "timestamp" and spec.timezone:
            problems.append(f"column {spec.name!r}: only a timestamp may carry a time zone")
    return problems


# ── The cast ─────────────────────────────────────────────────────────────

_INTEGER = r"^[+-]?[0-9]+$"
_DECIMAL = r"^[+-]?([0-9]+|[0-9]*\.[0-9]{1,9})$"
_FLOAT = r"^[+-]?(([0-9]+\.?[0-9]*|\.[0-9]+)([eE][+-]?[0-9]+)?|[iI][nN][fF]([iI][nN][iI][tT][yY])?|[nN][aA][nN])$"
_ISO_DATE = r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$"
_TIMESTAMP = (
    r"^(?P<date>[^T ]+)[T ][ ]*(?P<h>[0-9]{2}):(?P<m>[0-9]{2})(?::(?P<s>[0-9]{2})(?:\.(?P<f>[0-9]{1,6}))?)?"
    r"(?P<z>Z|[+-][0-9]{2}:?[0-9]{2})?$"
)
_TRUE = pa.array(["true", "yes", "y", "t", "1"])
_FALSE = pa.array(["false", "no", "n", "f", "0"])


def _null_where_not(array: pa.Array, valid: pa.Array) -> pa.Array:
    return pc.if_else(valid, array, pa.scalar(None, array.type))


def _iso_of_date(text: pa.Array, date_format: str) -> pa.Array:
    """`YYYY-MM-DD` text for each date in the reading, null where it is not one in
    that reading (the calendar is checked by the parse that follows)."""
    if date_format == "iso":
        return _null_where_not(text, pc.match_substring_regex(text, _ISO_DATE))
    parts: list[pa.Array] = []
    for separator in ("/", "-", "."):
        pattern = (
            rf"^(?P<a>[0-9]{{1,2}}){re.escape(separator)}(?P<b>[0-9]{{1,2}}){re.escape(separator)}(?P<y>[0-9]{{4}})$"
        )
        found = pc.extract_regex(text, pattern)
        day, month = ("a", "b") if date_format == "dmy" else ("b", "a")
        iso = pc.binary_join_element_wise(
            found.field("y"),
            pc.utf8_lpad(found.field(month), 2, "0"),
            pc.utf8_lpad(found.field(day), 2, "0"),
            "-",
        )
        # The children of a struct that did not match are not null: its validity says.
        parts.append(_null_where_not(iso, pc.is_valid(found)))
    return pc.coalesce(*parts)


def _dates(iso: pa.Array) -> pa.Array:
    """date32 for ISO text, null where the date does not exist (or is year 0)."""
    parsed = pc.strptime(iso, format="%Y-%m-%d", unit="s", error_is_null=True)
    # Arrow's parse rolls 2026-02-30 over to March 2: a date is real only if it reads back as itself.
    real = pc.and_(pc.equal(pc.strftime(parsed, format="%Y-%m-%d"), iso), pc.invert(pc.starts_with(iso, "0000")))
    return _null_where_not(parsed, pc.fill_null(real, False)).cast(pa.date32())


def _timestamps(text: pa.Array, spec: ColumnSpec) -> pa.Array:
    found = pc.extract_regex(text, _TIMESTAMP)
    matched = pc.is_valid(found)

    def optional(name: str) -> pa.Array:
        # A group that did not take part is an empty string or a null, by version.
        child = found.field(name)
        return pc.if_else(pc.equal(child, ""), pa.scalar(None, pa.string()), child)

    iso = _iso_of_date(found.field("date"), spec.date_format)
    seconds = pc.coalesce(optional("s"), pa.scalar("00"))
    clock = pc.binary_join_element_wise(found.field("h"), found.field("m"), seconds, ":")
    stamp = pc.binary_join_element_wise(iso, clock, "T")
    local = pc.strptime(stamp, format="%Y-%m-%dT%H:%M:%S", unit="us", error_is_null=True)
    # Read back as itself, as for a date (the parse rolls 31 April over to 1 May, 24:00 to the next day).
    real = pc.and_(pc.equal(pc.utf8_slice_codeunits(pc.strftime(local, format="%Y-%m-%dT%H:%M:%S"), 0, 19), stamp), pc.invert(pc.starts_with(iso, "0000")))
    local = _null_where_not(local, pc.fill_null(real, False)).cast(pa.int64())
    fraction = pc.utf8_rpad(pc.coalesce(optional("f"), pa.scalar("")), 6, "0")
    micros = pc.cast(fraction, pa.int64())
    zone = optional("z")
    has_zone = pc.invert(pc.is_null(zone))
    # A zone must be there exactly when the column holds instants.
    zone_ok = has_zone if spec.timezone else pc.invert(has_zone)
    # Offset: `Z` (or none) is 0; otherwise the sign, hours and minutes.
    plain = pc.replace_substring(pc.coalesce(zone, pa.scalar("Z")), ":", "")
    zulu = pc.equal(plain, "Z")
    sign = pc.if_else(pc.starts_with(plain, "-"), pa.scalar(-1, pa.int64()), pa.scalar(1, pa.int64()))
    hours = pc.cast(pc.if_else(zulu, pa.scalar("00"), pc.utf8_slice_codeunits(plain, 1, 3)), pa.int64())
    minutes = pc.cast(pc.if_else(zulu, pa.scalar("00"), pc.utf8_slice_codeunits(plain, 3, 5)), pa.int64())
    zone_in_range = pc.and_(pc.less(hours, 24), pc.less(minutes, 60))
    offset = pc.multiply(sign, pc.add(pc.multiply(hours, 3600), pc.multiply(minutes, 60)))
    offset = pc.multiply(offset, 1_000_000)
    instant = pc.subtract(pc.add(local, micros), offset)
    valid = pc.and_(pc.and_(matched, zone_ok), zone_in_range)
    target = pa.timestamp("us", tz="UTC") if spec.timezone else pa.timestamp("us")
    return _null_where_not(instant, pc.fill_null(valid, False)).cast(target, safe=False)


def _integers(trimmed: pa.Array) -> pa.Array:
    valid = pc.match_substring_regex(trimmed, _INTEGER)
    cleaned = pc.replace_substring_regex(trimmed, r"^\+", "")
    candidates = _null_where_not(cleaned, valid)
    try:
        return candidates.cast(pa.int64())
    except pa.ArrowInvalid:
        # One value past 64 bits: the vector cast cannot say which, so decide per value.
        low, high = -(2**63), 2**63 - 1
        return pa.array(
            [None if v is None else (int(v) if low <= int(v) <= high else None) for v in candidates.to_pylist()],
            type=pa.int64(),
        )


def _decimals(trimmed: pa.Array) -> pa.Array:
    valid = pc.match_substring_regex(trimmed, _DECIMAL)
    cleaned = pc.replace_substring_regex(trimmed, r"^\+", "")
    # At most 29 digits before the point, leading zeros not counted.
    whole = pc.replace_substring_regex(cleaned, r"^[+-]?0*([0-9]*)(\.[0-9]*)?$", r"\1")
    valid = pc.and_(valid, pc.less_equal(pc.utf8_length(whole), 29))
    candidates = _null_where_not(cleaned, valid)
    # A leading `.` or `-.` is spelled with a zero for the parse.
    candidates = pc.replace_substring_regex(candidates, r"^\.", "0.")
    candidates = pc.replace_substring_regex(candidates, r"^-\.", "-0.")
    return candidates.cast(DECIMAL)


def cast_column(text: pa.Array, spec: ColumnSpec) -> pa.Array:
    """One column of text cast to its type. An empty cell is null; a value that does
    not fit is null too (the caller counts it, being non-empty).

    # Errors

    Raises `ValueError` for a type this module does not know.
    """
    if spec.type == "text":
        return text
    if spec.type not in TYPES:
        raise ValueError(f"unknown column type {spec.type!r}")
    trimmed = pc.utf8_trim_whitespace(text)
    if spec.type == "integer":
        return _integers(trimmed)
    if spec.type == "decimal":
        return _decimals(trimmed)
    if spec.type == "boolean":
        lower = pc.utf8_lower(trimmed)
        known = pc.or_(pc.is_in(lower, value_set=_TRUE), pc.is_in(lower, value_set=_FALSE))
        return _null_where_not(pc.is_in(lower, value_set=_TRUE), known)
    if spec.type == "date":
        return _dates(_iso_of_date(trimmed, spec.date_format))
    if spec.type == "timestamp":
        return _timestamps(trimmed, spec)
    valid = pc.match_substring_regex(trimmed, _FLOAT)
    return _null_where_not(trimmed, valid).cast(pa.float64())


def _empty_mask(text: pa.Array) -> pa.Array:
    return pc.fill_null(pc.equal(pc.utf8_length(pc.utf8_trim_whitespace(text)), 0), True)


@dataclass
class BadReport:
    """The values that did not fit, counted over the whole file and listed for the
    first 100: `{count, byColumn, rows}` (`AD6`; the API's `BadValues`)."""

    columns: list[str]
    counts: list[int] = field(default_factory=list)
    rows: list[dict[str, Any]] = field(default_factory=list)

    def __post_init__(self) -> None:
        self.counts = [0] * len(self.columns)

    @property
    def count(self) -> int:
        return sum(self.counts)

    def add(self, column: int, first_row: int, bad_positions: list[int], values: pa.Array) -> None:
        """Count the bad values of one column of a batch whose first row is `first_row`
        (1-based) and keep the first ones that fit the listing."""
        self.counts[column] += len(bad_positions)
        for position in bad_positions:
            if len(self.rows) >= MAX_LISTED:
                break
            self.rows.append(
                {
                    "row": first_row + position,
                    "column": self.columns[column],
                    "value": (values[position].as_py() or "")[:MAX_VALUE_CHARS],
                }
            )

    def as_json(self) -> dict[str, Any]:
        return {
            "count": self.count,
            "byColumn": [{"column": c, "count": n} for c, n in zip(self.columns, self.counts) if n],
            "rows": sorted(self.rows, key=lambda r: r["row"]),
        }


def cast_batch(
    batch: pa.Table, specs: list[ColumnSpec], first_row: int, report: BadReport
) -> pa.Table:
    """A batch of text columns cast to the column list, bad values counted in `report`.
    `first_row` is the 1-based number of the batch's first data row. The columns leave
    under their `name`, in the list's order."""
    columns: list[pa.Array] = []
    for index, spec in enumerate(specs):
        text = batch.column(index).combine_chunks() if isinstance(batch.column(index), pa.ChunkedArray) else batch.column(index)
        cast = cast_column(text, spec)
        if spec.type != "text":
            bad = pc.and_(pc.invert(_empty_mask(text)), pc.is_null(cast))
            positions = [i for i, flag in enumerate(bad.to_pylist()) if flag]
            if positions:
                report.add(index, first_row, positions, text)
        columns.append(cast)
    return pa.table(columns, names=[spec.name for spec in specs])


def canonical(value: Any, spec: ColumnSpec) -> str | None:
    """A cast value as the canonical text of `cast.json`: integer digits; decimal with
    nine fractional digits; `true`/`false`; `YYYY-MM-DD`; `YYYY-MM-DDTHH:MM:SS.ffffff`
    (and `Z` for instants, in UTC); a float as Python's repr; None for null."""
    if value is None:
        return None
    if spec.type == "decimal":
        return f"{value:.9f}"
    if spec.type == "boolean":
        return "true" if value else "false"
    if spec.type == "date":
        return value.isoformat()
    if spec.type == "timestamp":
        text = value.strftime("%Y-%m-%dT%H:%M:%S.%f")
        return text + ("Z" if spec.timezone else "")
    if spec.type == "float":
        return repr(float(value))
    return str(value)
