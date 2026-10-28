"""A Parquet file as text, value by value: the job's reader for `format:
parquet` (`DATA-17` T2.2, plan AD2).

Until this existed the API converted a Parquet file to CSV in memory
(`upload_parquet.rs::convert`) and the job loaded the CSV. The API no longer
holds the file, so the job reads it, in batches (`pyarrow.parquet.ParquetFile
.iter_batches`), and every column leaves as `string`, exactly the text the API's
converter wrote, until the column-spec contract (`DATA-18`, slice 3) lets a
column keep its type.

# One text, two writers

The rules are those of the module doc of `upload_parquet.rs`, and
`ops/fixtures/parquet/types.expected.json` is the file both writers are tested
against: the Rust converter writes it, this module reads `types.parquet` to the
same records. The rules in short: null is empty text; text as stored; integers
as digits; a float as the shortest text that reads back to the same value, in
plain digits from `1e-6` up to `1e21` and in exponent form outside (`1e-7`,
`1.5e22`), negative zero as `0`, `NaN`, `inf` and `-inf`, and a `float` (32 bit)
in its own shortest form (`0.1`, never the double it widens to); a decimal as the
exact digits with its scale (`12.50`); a boolean as `true` or `false`; a date as
`YYYY-MM-DD`; a timestamp as `YYYY-MM-DDTHH:MM:SS` with the fraction only when
there is one and its trailing zeros dropped, and a trailing `Z` when the column
holds instants (a time zone is set); a time of day as `HH:MM:SS` and the same
fraction. A date, timestamp or time outside what the text can say (a year before
0000 or after 9999, a time outside one day) is written as the integer the file
holds, not as a wrong date.

The legacy `INT96` timestamp is read as microseconds (`coerce_int96_timestamp_unit`)
because nanoseconds since 1970 only reach the years 1677 to 2262; it carries no
zone, so it is written as a local time.

# What is refused

A column of a type with no honest text form: binary (including a `uuid` stored
as 16 bytes), nested (list, map, struct), interval and duration. `UnsupportedColumn`
names the first one; `file_ingest` records it as an unreadable file, and the API
has already refused the same file at upload with a sentence that names the column.
"""

from __future__ import annotations

import struct
from collections.abc import Iterator
from decimal import Decimal

import pyarrow as pa
import pyarrow.compute as pc
import pyarrow.parquet as pq

__all__ = ["UnsupportedColumn", "open_parquet", "text_batches", "column_names_of"]

_SECONDS_PER_DAY = 86_400

# How a timestamp or time unit divides a second, and the digits of its fraction.
_PER_SECOND = {"s": (1, 0), "ms": (1_000, 3), "us": (1_000_000, 6), "ns": (1_000_000_000, 9)}


class UnsupportedColumn(ValueError):
    """A column whose values have no honest text form."""

    def __init__(self, column: str, type_name: str) -> None:
        super().__init__(f"column {column!r} holds {type_name} values, which cannot be loaded as text")
        self.column = column
        self.type_name = type_name


def open_parquet(handle) -> pq.ParquetFile:
    """The file, opened for reading by ranges, with `INT96` as microseconds."""
    # `pre_buffer=False` (`PR #186 review BLOCKER 6`): the default caches every byte range it
    # fetched until the file is closed, so reading a file in batches held the whole file.
    return pq.ParquetFile(handle, coerce_int96_timestamp_unit="us", pre_buffer=False)


def column_names_of(parquet_file: pq.ParquetFile) -> list[str]:
    return list(parquet_file.schema_arrow.names)


def check_types(parquet_file: pq.ParquetFile) -> None:
    """Refuse the first column that has no text form.

    # Errors

    Raises `UnsupportedColumn`.
    """
    for field in parquet_file.schema_arrow:
        if _kind(field.type) is None:
            raise UnsupportedColumn(field.name, _word(field.type))


def _word(data_type: pa.DataType) -> str:
    if pa.types.is_binary(data_type) or pa.types.is_large_binary(data_type) or pa.types.is_fixed_size_binary(data_type):
        return "binary"
    if pa.types.is_list(data_type) or pa.types.is_large_list(data_type) or pa.types.is_fixed_size_list(data_type):
        return "list"
    if pa.types.is_map(data_type):
        return "map"
    if pa.types.is_struct(data_type):
        return "struct"
    if pa.types.is_interval(data_type) or pa.types.is_duration(data_type):
        return "interval"
    return "unsupported"


def _kind(data_type: pa.DataType) -> str | None:
    """How a type is written, or None when it has no text form."""
    t = pa.types
    if t.is_null(data_type):
        return "null"
    if t.is_boolean(data_type) or t.is_integer(data_type) or t.is_decimal(data_type):
        return "cast"
    if t.is_float16(data_type) or t.is_float32(data_type):
        return "float32"
    if t.is_float64(data_type):
        return "float64"
    if t.is_date32(data_type):
        return "date"
    if t.is_timestamp(data_type):
        return "timestamp"
    if t.is_time32(data_type) or t.is_time64(data_type):
        return "time"
    if t.is_string(data_type) or t.is_large_string(data_type):
        return "string"
    return None


# ── Values to text ───────────────────────────────────────────────────────


def _float_text(value: float, *, single: bool) -> str:
    """The shortest text that reads back to the same value (`float_text` in
    `upload_workbook.rs`): plain digits from `1e-6` up to `1e21`, exponent form
    outside, `0` for either zero, `NaN`, `inf` and `-inf`. `single` finds the
    shortest text for a 32-bit float, so `0.1` stays `0.1`."""
    if value != value:
        return "NaN"
    if value in (float("inf"), float("-inf")):
        return "inf" if value > 0 else "-inf"
    if value == 0:
        return "0"
    if single:
        shortest = next(
            text
            for text in (f"{value:.{digits}g}" for digits in range(1, 10))
            if struct.unpack("f", struct.pack("f", float(text)))[0] == value
        )
    else:
        shortest = repr(value)
    exact = Decimal(shortest)
    if 1e-6 <= abs(value) < 1e21:
        plain = format(exact.normalize(), "f")
        return plain
    sign, digits, exponent = exact.normalize().as_tuple()
    mantissa = "".join(str(d) for d in digits)
    scientific = len(mantissa) + int(exponent) - 1
    text = mantissa[0] + (f".{mantissa[1:]}" if len(mantissa) > 1 else "") + f"e{scientific}"
    return f"-{text}" if sign else text


def _date_text(days: int) -> str | None:
    """The civil date `days` after 1970-01-01 as `YYYY-MM-DD`, or None outside the
    years 0000 to 9999 (Howard Hinnant's `civil_from_days`, as the Rust side)."""
    shifted = days + 719_468
    era = shifted // 146_097
    day_of_era = shifted % 146_097
    year_of_era = (day_of_era - day_of_era // 1_460 + day_of_era // 36_524 - day_of_era // 146_096) // 365
    day_of_year = day_of_era - (365 * year_of_era + year_of_era // 4 - year_of_era // 100)
    month_index = (5 * day_of_year + 2) // 153
    day = day_of_year - (153 * month_index + 2) // 5 + 1
    month = month_index + 3 if month_index < 10 else month_index - 9
    year = year_of_era + era * 400 + (1 if month <= 2 else 0)
    if not 0 <= year <= 9999:
        return None
    return f"{year:04d}-{month:02d}-{day:02d}"


def _clock_text(second_of_day: int, fraction: int, digits: int) -> str:
    text = f"{second_of_day // 3600:02d}:{second_of_day // 60 % 60:02d}:{second_of_day % 60:02d}"
    if fraction != 0:
        text += "." + f"{fraction:0{digits}d}".rstrip("0")
    return text


def _timestamp_text(value: int, unit: str, zoned: bool) -> str:
    per_second, digits = _PER_SECOND[unit]
    seconds, fraction = divmod(value, per_second)
    date = _date_text(seconds // _SECONDS_PER_DAY)
    if date is None:
        return str(value)
    clock = _clock_text(seconds % _SECONDS_PER_DAY, fraction, digits)
    return f"{date}T{clock}{'Z' if zoned else ''}"


def _time_text(value: int, unit: str) -> str:
    per_second, digits = _PER_SECOND[unit]
    if not 0 <= value < _SECONDS_PER_DAY * per_second:
        return str(value)
    return _clock_text(value // per_second, value % per_second, digits)


def _mapped(array: pa.Array, as_type: pa.DataType, show) -> pa.Array:
    """`show` applied to every non-null value (read as `as_type`), null as empty text."""
    values = array.cast(as_type).to_pylist()
    return pa.array(["" if value is None else show(value) for value in values], type=pa.string())


def text_of(array: pa.Array) -> pa.Array:
    """One column as `string`, null as empty text.

    # Errors

    Raises `UnsupportedColumn` for a type with no text form.
    """
    data_type = array.type
    kind = _kind(data_type)
    if kind == "null":
        return pa.array([""] * len(array), type=pa.string())
    if kind in ("cast", "string"):
        return pc.fill_null(array.cast(pa.string()), "")
    if kind == "float32":
        return _mapped(array, pa.float32(), lambda v: _float_text(v, single=True))
    if kind == "float64":
        return _mapped(array, pa.float64(), lambda v: _float_text(v, single=False))
    if kind == "date":
        return _mapped(array, pa.int32(), lambda days: _date_text(days) or str(days))
    if kind == "timestamp":
        return _mapped(
            array, pa.int64(), lambda v: _timestamp_text(v, _unit_of(data_type), data_type.tz is not None)
        )
    if kind == "time":
        carrier = pa.int32() if pa.types.is_time32(data_type) else pa.int64()
        return _mapped(array, carrier, lambda v: _time_text(v, _unit_of(data_type)))
    raise UnsupportedColumn("?", _word(data_type))


def _unit_of(data_type: pa.DataType) -> str:
    return {"s": "s", "ms": "ms", "us": "us", "ns": "ns"}[data_type.unit]


def text_batches(parquet_file: pq.ParquetFile, batch_rows: int) -> Iterator[pa.Table]:
    """The file's rows as tables of `string` columns, at most `batch_rows` rows each.

    # Errors

    Raises `UnsupportedColumn` for a column with no text form; whatever
    `pyarrow` raises for a page that cannot be decoded.
    """
    names = column_names_of(parquet_file)
    for batch in parquet_file.iter_batches(batch_size=batch_rows):
        yield pa.table({name: text_of(column) for name, column in zip(names, batch.columns)})
