"""History (SCD type 2) for a Bronze table: the `history` load mode of
`adapters/sink.py` (`SRC-10`, feature page `docs/core/features/load-modes.md`
decisions 1, 9 and 10; plan `docs/superpowers/plans/2026-10-12-src-10-load-modes.md`
section 3b).

# What the table holds

Beside the source columns and the sink's `_ingested_at`, a history table has
four columns this module adds: `_valid_from` (the time of the load that first
saw this version), `_valid_to` (the time of the load that saw it replaced, null
for the current version), `_is_current` (true on the current version) and
`_row_hash` (see "The hash"). A key has one current row. A run reads the WHOLE
source (this version has no cursor), keeps one row per key (the dedup step of
`adapters/dedup.py`, with its bound), reads from the table the current row of
every key in the load, and for each key:

- not in the table: one new current row;
- in the table with the same hash: nothing, the row is not touched, not even its
  `_ingested_at`;
- in the table with another hash: its current row again, with `_valid_to` = this
  load's time and `_is_current` false (every other column as it was), and a new
  current row with `_valid_from` = this load's time. A row that changes back to
  the values of an older version is a new third version, never a reopening.

# Limits to tell a customer (decisions 9 and 10)

- `_valid_from` is the time of the LOAD that saw the version, not the time the
  change happened at the source; a run once a day sees at most one version a day.
  One version per key per run.
- A key that is no longer in the source stays as it is, current: a batch load
  cannot tell a deletion from a row it was not given.
- The whole source is read on every run and held to compare, bounded by
  `INGEST_DEDUP_MAX_ROWS` (distinct keys; `adapters/dedup.py`). Memory is the
  winners of the load plus the current row of each of their keys, so about twice
  the rows of the source; above the bound the load fails with the dedup step's
  fixed message before anything is read from the table.
- A column holds one type: a value that does not fit the type the table already
  has is refused, naming the column (never the value). A column that is null in
  every row of a load is not created until a row has a value in it.
- A table loaded in another mode is not converted: see "Refusals".

# Why the write is not `dlt`'s

`dlt` 1.30.0 commits an Iceberg merge in pieces of 1,000 rows
(`dlt/common/libs/pyiceberg.py::merge_iceberg_table`), each its own commit, so a
history load above 1,000 rows could leave a closed row without its successor
(`test_load_modes_measurement.py`, cases l and n). The rows are therefore written
by ONE `pyiceberg` transaction of ours. `dlt` still owns the table: it creates
it and adds columns (`ensure_columns`: the pipeline is run on a zero-row typed
Arrow table), so its schema, state and catalog entry stay its own.

# The three properties (measured by `test_history_write_measurement.py`)

1. ONE commit for the data of a load at any size
   (`test_a_load_of_3000_changed_keys_is_one_commit_of_6000_rows`,
   `test_a_load_of_2500_composite_keys_is_one_commit`). Creating the table and adding
   a column are metadata commits of their own, before the data and with no row in
   them: 1 to create, 2 to add a column.
2. A failure leaves the table exactly as it was, and the load after it succeeds
   (`test_a_row_that_violates_the_schema_...`,
   `test_an_exception_between_staging_the_closed_rows_and_the_new_ones_...`). The
   closed rows and the new rows are staged in one transaction that commits last.
3. A second identical load writes nothing and adds no commit
   (`test_a_second_identical_load_writes_nothing_and_adds_no_commit`): no new
   version means no `dlt` step and no transaction.

Afterwards `dlt` and `pyiceberg` agree about the table, `clear_table` empties it
and keeps its columns, and the next load works
(`test_after_our_writes_dlt_agrees_about_the_table_and_clear_table_and_the_next_load_work`).
Not measured: a Lakekeeper REST catalog, two writers at once (pyiceberg commits
optimistically, so a conflicting commit fails the load and leaves the table as it
was), a query engine reading the table (the gate).

# The hash

`row_hash` is SHA-256 over the NON-KEY source columns, in sorted column-name order.
Each column that is not null contributes three length-prefixed parts (8 bytes,
big-endian, then the UTF-8 bytes): the column name, a one-letter type tag and the
value's text. So no two different rows frame to the same bytes, and:

- a null (or a float NaN) contributes nothing: the column is skipped. Adding a
  column at the source therefore changes the hash of a row only if the row has a
  value in it, and does not close every row;
- an empty string is `s` with empty text; the string "None" is `s` with the text
  `None`; neither is a null;
- the tag separates numbers from their text: `i` 1, `f` 1.0 (its `repr`), `n` a
  `Decimal` (normalised, so 1.10 and 1.1 agree), `s` "1", `b` a bool (`1` or `0`);
- `t` a datetime in UTC with microseconds (a naive one is taken as UTC, which is
  how it is stored), `d` a date, `c` a time, `x` bytes in hex, `j` a list or
  object as JSON with sorted keys and no spaces.

Key columns, the sink's `_ingested_at` and `dlt`'s `_dlt_*` columns are not hashed.
Keys are matched by the same encoding (`key_token`), so a naive and a UTC datetime,
or `Decimal` 1.10 and 1.1, are one key.

# Refusals

Each is a `HistoryRefused`, a `ValueError` with FIXED text (names of tables and
columns and counts, never a value or an upstream message), recorded as a
`rejected` run and a non-retryable `Failure`:

- a source column named like one of the four columns above;
- a table that already holds rows and lacks those columns, or has them but holds
  a row loaded in another mode (no `_row_hash`): the message names "Refresh,
  removing rows" as the way. An EMPTY table of any origin is accepted: that is
  the state "Refresh, removing rows" leaves, and the columns are added to it;
- a null key value, and too many keys: the dedup step's own messages;
- a value of a type the table cannot hold (see the limits).

The four columns are NULLABLE in the table. `clear_table` keeps a table's columns,
and a column required by `dlt` would make every later load in another mode fail,
even after "Refresh, removing rows" (measured by
`test_required_history_columns_would_make_clear_and_a_load_in_another_mode_fail`).
Our write fills them on every row. Not refused here: a load in another mode into
a table that holds history (it would add rows with no history values; the next
history load then refuses the table). The console says a table cannot change
across this boundary without a refresh.
"""

from __future__ import annotations

import hashlib
import json
from collections.abc import Callable, Iterable, Mapping
from dataclasses import dataclass
from datetime import date, datetime, time, timezone
from decimal import Decimal
from typing import Any

import dlt
import pyarrow as pa
from dlt.common.destination.exceptions import DestinationUndefinedEntity
from dlt.common.utils import uniq_id_base64
from dlt.extract.resource import DltResource
from dlt.extract.source import DltSource
from pyiceberg.expressions import And, EqualTo, In, IsNull

from dispar_orchestrate.adapters import dedup

HISTORY_COLUMNS = ("_valid_from", "_valid_to", "_is_current", "_row_hash")
# The columns the sink and `dlt` own: never taken from the source, never hashed.
OWN_COLUMNS = ("_ingested_at", "_dlt_id", "_dlt_load_id")
SYSTEM_COLUMNS = (*HISTORY_COLUMNS, *OWN_COLUMNS)

# How many keys one read of the table asks for (`In` list of a filter).
READ_CHUNK = 1000

# All four NULLABLE, for the reason in the module docstring ("Refusals").
HISTORY_HINTS: dict[str, Any] = {
    "_valid_from": {"data_type": "timestamp", "nullable": True},
    "_valid_to": {"data_type": "timestamp", "nullable": True},
    "_is_current": {"data_type": "bool", "nullable": True},
    "_row_hash": {"data_type": "text", "nullable": True},
}
UTC_US = pa.timestamp("us", tz="UTC")

ANOTHER_MODE = (
    'table {table!r} holds rows that were not loaded in the history mode; use "Refresh, removing rows" '
    "on it to load it with history"
)


class HistoryRefused(dedup.DedupRefused):
    """A history load that cannot be done as asked. A `DedupRefused` because the
    load deduplicates first and is refused the same way: one `ValueError` arm in the
    ingest op (non-retryable) and one `rejected` status in `ingest_factory`."""


@dataclass(frozen=True)
class Versions:
    """What one load writes: the current rows to close (each as the row was,
    with `_valid_to` and `_is_current` changed) and the new current rows."""

    closed: list[dict[str, Any]]
    new: list[dict[str, Any]]


@dataclass(frozen=True)
class HistoryOutcome:
    """`new_versions` is the number of NEW rows written; the closed rows are the
    same rows as before with two columns changed, and are not counted."""

    new_versions: int
    closed: int


# ---------------------------------------------------------------------------
# Pure functions
# ---------------------------------------------------------------------------


def _is_null(value: Any) -> bool:
    return dedup._is_null(value)  # noqa: SLF001 -- the ONE definition of "no value" (None or NaN)


def _utc(value: datetime) -> datetime:
    return value.replace(tzinfo=timezone.utc) if value.tzinfo is None else value.astimezone(timezone.utc)


def _canonical(column: str, value: Any) -> tuple[str, str]:
    """`(tag, text)` of a non-null value; see "The hash".

    # Errors

    Raises `HistoryRefused` for a type that has no encoding (the column is named,
    the value is not)."""
    # `bool` before `int`, `datetime` before `date`: the subclasses.
    if isinstance(value, bool):
        return "b", "1" if value else "0"
    if isinstance(value, int):
        return "i", str(value)
    if isinstance(value, float):
        return "f", repr(value)
    if isinstance(value, Decimal):
        return "n", format(value.normalize(), "f")
    if isinstance(value, datetime):
        return "t", _utc(value).isoformat(timespec="microseconds")
    if isinstance(value, date):
        return "d", value.isoformat()
    if isinstance(value, time):
        return "c", value.isoformat(timespec="microseconds")
    if isinstance(value, (bytes, bytearray, memoryview)):
        return "x", bytes(value).hex()
    if isinstance(value, str):
        return "s", value
    if isinstance(value, (dict, list, tuple)):
        return "j", json.dumps(value, sort_keys=True, separators=(",", ":"), default=str)
    raise HistoryRefused(f"column {column!r} holds a value of a type a history table cannot keep ({type(value).__name__})")


def row_hash(row: Mapping[str, Any], key: Iterable[str]) -> str:
    """The hash of a source row's non-key columns; see "The hash" in the module
    docstring for the exact encoding.

    # Errors

    Raises `HistoryRefused` for a value of a type with no encoding."""
    skipped = {*key, *SYSTEM_COLUMNS}
    digest = hashlib.sha256()
    for column in sorted(c for c in row if c not in skipped):
        value = row[column]
        if _is_null(value):
            continue
        tag, text = _canonical(column, value)
        for part in (column, tag, text):
            data = part.encode("utf-8", errors="surrogatepass")
            digest.update(len(data).to_bytes(8, "big"))
            digest.update(data)
    return digest.hexdigest()


def key_token(row: Mapping[str, Any], key: Iterable[str]) -> tuple[tuple[str, str], ...]:
    """The key of `row` in the encoding the hash uses, so a source row and a row
    read back from the table meet on one key whatever their Python types.

    # Errors

    Raises `HistoryRefused` for a key value with no encoding."""
    return tuple(_canonical(column, row[column]) for column in key)


def versions_to_write(
    current_rows: Mapping[tuple[tuple[str, str], ...], Mapping[str, Any]],
    source_rows: Iterable[Mapping[str, Any]],
    key: tuple[str, ...],
    load_time: datetime,
) -> Versions:
    """The rows one load writes. `current_rows` are the current rows of the
    load's keys as the table holds them, by `key_token`; `source_rows` have one
    row per key (the dedup step). Pure: no clock, no table.

    A key with no current row gets one new row; a key whose current row has the
    same `_row_hash` gets nothing; any other gets its current row closed
    (`_valid_to` = `load_time`, `_is_current` false, everything else as it was,
    `_ingested_at` and the `dlt` columns included) and a new row. A new row is the
    source row without the columns the sink and `dlt` own, with `_ingested_at` and
    `_valid_from` = `load_time`.

    # Errors

    Raises `HistoryRefused` for a value with no encoding."""
    closed: list[dict[str, Any]] = []
    new: list[dict[str, Any]] = []
    for row in source_rows:
        digest = row_hash(row, key)
        existing = current_rows.get(key_token(row, key))
        if existing is not None and existing.get("_row_hash") == digest:
            continue
        if existing is not None:
            closed.append({**existing, "_valid_to": load_time, "_is_current": False})
        version = {column: value for column, value in row.items() if column not in SYSTEM_COLUMNS}
        version.update(
            _ingested_at=load_time,
            _valid_from=load_time,
            _valid_to=None,
            _is_current=True,
            _row_hash=digest,
        )
        new.append(version)
    return Versions(closed=closed, new=new)


# ---------------------------------------------------------------------------
# Column types
# ---------------------------------------------------------------------------


def _arrow_type(column: str, value: Any) -> pa.DataType:
    if isinstance(value, bool):
        return pa.bool_()
    if isinstance(value, int):
        return pa.int64()
    if isinstance(value, float):
        return pa.float64()
    if isinstance(value, Decimal):
        return pa.decimal128(38, 9)  # `dlt`'s own default for a decimal
    if isinstance(value, datetime):
        return UTC_US
    if isinstance(value, date):
        return pa.date32()
    if isinstance(value, time):
        return pa.time64("us")
    if isinstance(value, (bytes, bytearray, memoryview)):
        return pa.binary()
    if isinstance(value, (str, dict, list, tuple)):
        return pa.string()  # a list or object is kept as JSON text, as `dlt` keeps it
    raise HistoryRefused(f"column {column!r} holds a value of a type a history table cannot keep ({type(value).__name__})")


def column_types(rows: Iterable[Mapping[str, Any]]) -> dict[str, pa.DataType]:
    """The Arrow type of each source column of `rows`, from its non-null values.
    A column null in every row is left out (as `dlt` leaves it out); int and
    float together are float.

    # Errors

    Raises `HistoryRefused` for a column with values of two types that cannot share
    one, or of a type with no Arrow form."""
    types: dict[str, pa.DataType] = {}
    for row in rows:
        for column, value in row.items():
            if column in SYSTEM_COLUMNS or _is_null(value):
                continue
            found = _arrow_type(column, value)
            known = types.get(column)
            if known is None or known == found:
                types[column] = found
            elif {known, found} == {pa.int64(), pa.float64()}:
                types[column] = pa.float64()
            else:
                raise HistoryRefused(
                    f"column {column!r} holds values of different types; a history table keeps one type per column"
                )
    return types


def _arrow_value(value: Any) -> Any:
    if _is_null(value):
        return None
    if isinstance(value, (dict, list, tuple)):
        return json.dumps(value, sort_keys=True, separators=(",", ":"), default=str)
    return value


def to_arrow(rows: list[dict[str, Any]], schema: pa.Schema, *, table: str) -> pa.Table:
    """`rows` as an Arrow table of exactly `schema` (the table's own). Column by
    column, so a value that does not fit names its column.

    # Errors

    Raises `HistoryRefused` for a value that does not fit its column's type, or a
    null in a column the table requires."""
    arrays = []
    for field in schema:
        values = [_arrow_value(row.get(field.name)) for row in rows]
        try:
            array = pa.array(values, type=field.type)
        except (pa.ArrowInvalid, pa.ArrowTypeError, OverflowError, ValueError, TypeError):
            raise HistoryRefused(
                f"table {table!r}: a value does not fit the type of column {field.name!r}"
            ) from None
        if not field.nullable and array.null_count:
            raise HistoryRefused(f"table {table!r}: column {field.name!r} is required but a row has no value for it")
        arrays.append(array)
    return pa.Table.from_arrays(arrays, schema=schema)


# ---------------------------------------------------------------------------
# The table
# ---------------------------------------------------------------------------


def open_table(pipeline: Any, table: str) -> Any | None:
    """The `pyiceberg` table through the destination client of the pipeline that
    owns it (in production Lakekeeper's REST catalog, as the sink installs it), or
    `None` when `dlt` does not know the table. Syncs the pipeline from the bucket
    first: a recreated container has no local copy of its schema."""
    pipeline.sync_destination()
    try:
        with pipeline.destination_client() as client:
            return client.load_open_table("iceberg", table)
    except DestinationUndefinedEntity:
        return None


def check_table(iceberg_table: Any, *, table: str) -> None:
    """Refuse a table that holds rows loaded in another mode.

    # Errors

    Raises `HistoryRefused` (naming "Refresh, removing rows") for a table with rows
    that lacks a history column, or has them but holds a row with no `_row_hash`.
    An empty table is accepted whatever it has."""
    if iceberg_table.scan(limit=1).to_arrow().num_rows == 0:
        return
    names = {field.name for field in iceberg_table.schema().fields}
    if not set(HISTORY_COLUMNS) <= names:
        raise HistoryRefused(ANOTHER_MODE.format(table=table))
    if iceberg_table.scan(row_filter=IsNull("_row_hash"), limit=1).to_arrow().num_rows:
        raise HistoryRefused(ANOTHER_MODE.format(table=table))


def read_current(
    iceberg_table: Any, key: tuple[str, ...], rows: list[dict[str, Any]], *, chunk: int = READ_CHUNK
) -> dict[tuple[tuple[str, str], ...], dict[str, Any]]:
    """The current row of every key in `rows`, by `key_token`. The table is read
    `chunk` keys at a time with a filter on `_is_current` and, per key column, on
    the values asked for. For a composite key that filter is a superset of the pairs,
    so every row read is checked against the exact keys. `pyiceberg` prunes files by
    the key column's min/max, so a table whose loads brought disjoint key ranges is
    read in a fraction of its files (`test_load_modes_measurement.py`, case m).

    # Errors

    Raises `HistoryRefused` for a key value with no encoding."""
    wanted = {key_token(row, key): row for row in rows}
    tokens = list(wanted)
    found: dict[tuple[tuple[str, str], ...], dict[str, Any]] = {}
    for start in range(0, len(tokens), chunk):
        part = [wanted[token] for token in tokens[start : start + chunk]]
        row_filter: Any = EqualTo("_is_current", True)
        for column in key:
            row_filter = And(row_filter, In(column, [row[column] for row in part]))
        for stored in iceberg_table.scan(row_filter=row_filter).to_arrow().to_pylist():
            token = key_token(stored, key)
            if token in wanted:
                found[token] = stored
    return found


def ensure_columns(
    pipeline: Any,
    table: str,
    schema: pa.Schema,
    adapt: Callable[[DltResource], Any],
) -> None:
    """Let `dlt` create the table, or add the columns of `schema` it lacks, by running
    the pipeline on a ZERO-row Arrow table of that schema. `adapt` puts the sink's
    table hints (day partition on `_ingested_at`, format version 2) on the resource,
    the one definition `sink.load_via_sink` uses too. A `dlt` run with nothing to add
    still costs one empty commit, so the caller runs it only when a column is missing
    (`test_an_ensure_step_with_nothing_to_add_...`)."""
    resource = dlt.resource(pa.Table.from_pylist([], schema=schema), name=table, columns=HISTORY_HINTS)
    adapt(resource)
    pipeline.run(resource, table_name=table, table_format="iceberg", write_disposition="append")


def write_versions(
    iceberg_table: Any, key: tuple[str, ...], versions: Versions, *, table: str, load_time: datetime
) -> None:
    """ONE transaction: the closed rows replace the current rows they close (matched
    on the key and `_valid_from`) and the new rows are appended. A failure anywhere
    inside leaves nothing: the transaction commits last.

    # Errors

    Raises `HistoryRefused` for a value that does not fit, and when a row to close
    was not found in the table (another writer changed it): nothing is committed."""
    schema = iceberg_table.schema().as_arrow()
    load_id = f"{load_time.timestamp():.6f}"
    new = [{**row, "_dlt_load_id": load_id, "_dlt_id": uniq_id_base64(10)} for row in versions.new]
    closed_arrow = to_arrow(versions.closed, schema, table=table) if versions.closed else None
    new_arrow = to_arrow(new, schema, table=table)
    with iceberg_table.transaction() as transaction:
        if closed_arrow is not None:
            result = transaction.upsert(
                closed_arrow, join_cols=[*key, "_valid_from"], when_not_matched_insert_all=False
            )
            if result.rows_updated != len(versions.closed):
                raise HistoryRefused(f"table {table!r}: its current rows changed while this load ran; load again")
        transaction.append(new_arrow)


# ---------------------------------------------------------------------------
# One load
# ---------------------------------------------------------------------------


def _rows_of(source: Any) -> Iterable[dict[str, Any]]:
    """The rows of a `dlt` source or resource (run here, without a pipeline: a
    history load has no cursor and writes nothing through `dlt`), or of a plain
    iterable of dicts."""
    if isinstance(source, DltSource):
        for resource in source.selected_resources.values():
            yield from resource
    elif isinstance(source, DltResource):
        yield from source
    else:
        yield from source


def load_history(
    pipeline: Any,
    source: Any,
    table: str,
    key: tuple[str, ...],
    adapt: Callable[[DltResource], Any],
    *,
    now: Callable[[], datetime] = lambda: datetime.now(timezone.utc),
    limits: dedup.DedupLimits | None = None,
) -> HistoryOutcome:
    """One history load; see the module docstring. `pipeline` is the table's
    `dlt` pipeline with no package pending. Returns the number of NEW rows written
    (the closed rows are not counted) and of rows closed; `0, 0` when nothing
    changed, in which case nothing was written and nothing committed.

    # Errors

    Raises `HistoryRefused` (and the dedup step's `DedupRefused`) as listed in the
    module docstring; whatever reading the source, the catalog or the bucket raises."""
    # The time of the load that first sees a version: taken before anything is read.
    load_time = now()
    winners = dedup.deduplicate(_rows_of(source), key=key, cursor=None, table=table, limits=limits)
    reserved = sorted({column for row in winners for column in row if column in HISTORY_COLUMNS})
    if reserved:
        raise HistoryRefused(
            f"table {table!r}: the source has columns named {', '.join(reserved)}; a history table adds "
            f"{', '.join(HISTORY_COLUMNS)} itself, so rename or leave out those columns"
        )
    if not winners:
        return HistoryOutcome(new_versions=0, closed=0)

    iceberg_table = open_table(pipeline, table)
    current: dict[tuple[tuple[str, str], ...], dict[str, Any]] = {}
    if iceberg_table is not None:
        check_table(iceberg_table, table=table)
        # An empty table of another mode (what "Refresh, removing rows" leaves) has no
        # `_is_current` to filter on, and no current row either.
        if "_is_current" in {field.name for field in iceberg_table.schema().fields}:
            current = read_current(iceberg_table, key, winners)
    versions = versions_to_write(current, winners, key, load_time)
    if not versions.new:
        return HistoryOutcome(new_versions=0, closed=0)

    # The columns the rows to write carry, typed by the table where it has them.
    have = {} if iceberg_table is None else {f.name: f.type for f in iceberg_table.schema().as_arrow()}
    wanted = column_types(versions.new)
    needs = {*wanted, *HISTORY_COLUMNS, "_ingested_at"} - set(have)
    if iceberg_table is None or needs:
        fields = [pa.field(name, have.get(name, kind)) for name, kind in wanted.items()]
        fields += [
            pa.field("_ingested_at", UTC_US),
            pa.field("_valid_from", UTC_US),
            pa.field("_valid_to", UTC_US),
            pa.field("_is_current", pa.bool_()),
            pa.field("_row_hash", pa.string()),
            # `dlt` bookkeeping, required as in every Bronze table (measured: a zero-row Arrow
            # table makes them optional unless declared here); the write fills them.
            pa.field("_dlt_load_id", pa.string(), nullable=False),
            pa.field("_dlt_id", pa.string(), nullable=False),
        ]
        ensure_columns(pipeline, table, pa.schema(fields), adapt)
        iceberg_table = open_table(pipeline, table)
    if iceberg_table is None:
        raise HistoryRefused(f"table {table!r}: the table could not be created")
    write_versions(iceberg_table, key, versions, table=table, load_time=load_time)
    return HistoryOutcome(new_versions=len(versions.new), closed=len(versions.closed))
