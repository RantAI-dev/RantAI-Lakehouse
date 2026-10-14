"""One row per key, before the rows reach the Bronze table (`SRC-10`, the
`replace_dedup` and `incremental_dedup` load modes of `adapters/sink.py`).

# Why this exists

`dlt` 1.30.0 merges into an Iceberg table with `pyiceberg`'s `upsert`, and a
batch that holds one key twice fails that load (`SRC-10` measurement F5, case
`test_a_batch_that_holds_a_key_twice_fails_the_load_and_changes_nothing` in
`test_load_modes_measurement.py`). So the duplicates are removed here, in the
loader, before anything is written. `replace_dedup` needs the same step for the
same reason (a table that "keeps one row per key" must not be given two).

# Which row wins (feature page `load-modes.md`, decision 4)

The row with the HIGHEST value of the cursor column. On a tie, and when the
load has no cursor column, the LAST row read. A null cursor value (a SQL
`NULL`, `None`, or a float `NaN`) loses to every real value; two null cursors
tie, so the later one wins. Two cursor values that Python cannot order
(a string against a number, a naive time against a zone-aware one) fail the
load with a fixed message rather than being compared by guess
(`test_dedup.py` pins each rule).

# A key with no value

A row whose key column is null or absent cannot be matched, and `dlt` makes a
key column NOT NULL, so it would fail the normalize step with an error that
names a row's data (`SRC-10` measurement case d). The step counts such rows
instead and, at the end of the read and before anything is written, fails the
load with `MissingKeyValue`: a fixed message that names the table and the key
columns and gives the COUNT of rows, never a row.

# What it costs

Deduplicating needs every row of the load for a key in hand until the read
ends. The step keeps only the best row so far per key, in a dict keyed by the
key tuple: memory is proportional to the number of DISTINCT keys, not to the
rows read (a table of 10 million rows with 1 million keys holds 1 million
rows). It is a stage the read waits at: nothing reaches the sink until the
source is exhausted, so a `replace_dedup` of a large table writes nothing for as
long as the read takes. The number of distinct keys is bounded by
`INGEST_DEDUP_MAX_ROWS` (`DedupLimits`, default 2,000,000); above it the load
fails with `TooManyKeys` naming the setting. The bound is checked as keys
arrive, so the process stops growing at the bound instead of being killed for
memory.

# The marker (`SRC-10` third pass, finding R2)

A merge never repairs a table that already holds a key twice (measurement 3), so
the first run of a deduplicating mode must REPLACE the table with the winners.
"First" cannot be read from the incremental cursor alone: a table loaded with
`incremental` on the same cursor has cursor state and several versions of a key.
So a run of either mode leaves a marker, the key columns it deduplicated on, in
the resource state of the pipeline state (`dlt.current.resource_state`, beside the
incremental cursor `sink._incremental_cursors` reads), written while the rows are
extracted and so part of the same package: it is stored with the data in the
bucket, survives a recreated container, and is rolled back to its value before
the run by `Pipeline.abort_packages` if the load fails. `sink.load_via_sink` reads
it (`markers`) and treats a run as the first one unless the marker equals its key.
A run under any other mode that writes rows removes the marker (`clear_marker_on`).
It is written only when the run emitted rows: an empty read leaves the table as it
was, so it cannot make it clean.

# How it attaches

- A plain iterable of dicts (files, REST, `MongoDB`, SFTP): `dedup_rows`, a
  generator in front of the sink's stamp.
- A `dlt` source or resource (a `sql_database` table): `DedupStep`, an
  `ItemTransform` inserted as the LAST step of the resource's pipe, after the
  incremental cursor step and before `sink._stamp_last`'s `_ingested_at` stamp.
  That order is the point: the cursor filter has already dropped the rows
  beyond reach, and the stamp is written on the winner only, so it can never
  change which row wins (`test_sink_dedup_modes.py` asserts the order of the
  steps and that the stamp is applied once per winner).
  `dlt` hands a step one item at a time and has no end-of-stream hook, so the
  step wraps the pipe's data generator to append one empty-list marker
  (`_EndOfRows`) after the last page. `dlt`'s incremental step forwards empty
  lists untouched (`IncrementalResourceWrapper.__call__`), and when the marker
  arrives the step emits the winners as one item. A resource with any other
  kind of step in front of ours (a filter could swallow the marker, and then
  the winners would never be emitted) is refused, not guessed at.
"""

from __future__ import annotations

import math
import os
from collections.abc import Iterable, Iterator
from dataclasses import dataclass
from typing import Any

import dlt
from dlt.extract.incremental import IncrementalResourceWrapper
from dlt.extract.items_transform import ItemTransform, MapItem
from dlt.extract.resource import DltResource
from dlt.extract.source import DltSource

# The marker `sink.load_via_sink` keeps in the pipeline state (see
# "The marker" in this module's docstring).
MARKER_KEY = "_dedup_key"
MAX_KEYS_ENV = "INGEST_DEDUP_MAX_ROWS"
DEFAULT_MAX_KEYS = 2_000_000


class DedupRefused(ValueError):
    """A deduplicating load that cannot be done as asked. A `ValueError`, so the
    ingest op reports it as a non-retryable `Failure` (a retry would read the
    same rows and fail the same way), and `ingest_factory._run_one_object` records
    it as `rejected`. Every message is FIXED text: a table name, key column
    names, a setting name and a count, never a row's data."""


class MissingKeyValue(DedupRefused):
    """Rows whose key column is null or absent (`SRC-10` measurement case d)."""


class TooManyKeys(DedupRefused):
    """More distinct keys than `INGEST_DEDUP_MAX_ROWS` allows."""


class IncomparableCursor(DedupRefused):
    """Two cursor values that cannot be ordered."""


class UnsupportedForDedup(DedupRefused):
    """Rows or a pipe the step cannot work on."""


@dataclass(frozen=True)
class DedupLimits:
    """`max_keys` is the most DISTINCT keys one load may hold; read from
    `INGEST_DEDUP_MAX_ROWS` (the setting is named for rows because that is what
    an operator thinks in; what it counts is the distinct keys, the thing that
    sits in memory)."""

    max_keys: int = DEFAULT_MAX_KEYS

    @classmethod
    def from_env(cls) -> "DedupLimits":
        """# Errors

        Raises `DedupRefused` when the variable is set to something that is not a
        whole number of at least 1 -- fail closed: an unreadable bound is not
        replaced by the default behind the operator's back."""
        raw = os.environ.get(MAX_KEYS_ENV, "").strip()
        if not raw:
            return cls()
        try:
            value = int(raw)
        except ValueError:
            value = 0
        if value < 1:
            raise DedupRefused(f"{MAX_KEYS_ENV} must be a whole number of at least 1")
        return cls(max_keys=value)


def _is_null(value: Any) -> bool:
    return value is None or (isinstance(value, float) and math.isnan(value))


def _hashable(value: Any) -> Any:
    """A dict key for `value`: itself when it hashes, else its `repr` (a JSON
    array or object in a REST or `MongoDB` row). The type name is part of the
    fallback, so a list never collides with a string that spells it."""
    try:
        hash(value)
    except TypeError:
        return (type(value).__name__, repr(value))
    return value


class Winners:
    """The best row so far for each key of one load (see the module docstring)."""

    def __init__(
        self, *, key: tuple[str, ...], cursor: str | None, table: str, limits: DedupLimits | None = None
    ) -> None:
        self._key = key
        self._cursor = cursor
        self._table = table
        self._max_keys = (limits or DedupLimits.from_env()).max_keys
        self._best: dict[tuple[Any, ...], dict[str, Any]] = {}
        self._rows_without_key = 0
        self._key_columns_without_value: set[str] = set()

    @property
    def key(self) -> tuple[str, ...]:
        return self._key

    def add(self, row: dict[str, Any]) -> None:
        """Offer one row.

        # Errors

        Raises `UnsupportedForDedup` for a row that is not a dict,
        `TooManyKeys` when it would be the key after the bound, and
        `IncomparableCursor` when its cursor value cannot be ordered against
        the incumbent's."""
        if not isinstance(row, dict):
            raise UnsupportedForDedup(
                f"table {self._table!r}: a deduplicating load needs rows as records (column to value)"
            )
        values = [row.get(column) for column in self._key]
        missing = [column for column, value in zip(self._key, values) if _is_null(value)]
        if missing:
            self._rows_without_key += 1
            self._key_columns_without_value.update(missing)
            return
        key = tuple(_hashable(value) for value in values)
        incumbent = self._best.get(key)
        if incumbent is None:
            if len(self._best) >= self._max_keys:
                raise TooManyKeys(
                    f"table {self._table!r}: the load holds more than {self._max_keys} distinct keys, "
                    f"the limit set by {MAX_KEYS_ENV}"
                )
            self._best[key] = row
        elif self._wins(row, incumbent):
            # Same slot: the key keeps the position of its first appearance.
            self._best[key] = row

    def _wins(self, candidate: dict[str, Any], incumbent: dict[str, Any]) -> bool:
        if self._cursor is None:
            return True  # no cursor: the last row read wins
        new, old = candidate.get(self._cursor), incumbent.get(self._cursor)
        if _is_null(new):
            return _is_null(old)  # a null cursor loses to any value; two nulls tie, the later wins
        if _is_null(old):
            return True
        try:
            return bool(new >= old)  # a tie goes to the later row
        except TypeError:
            raise IncomparableCursor(
                f"table {self._table!r}: the values of cursor column {self._cursor!r} cannot be ordered "
                "against each other (mixed types)"
            ) from None

    def result(self) -> list[dict[str, Any]]:
        """The winners, in the order their keys first appeared.

        # Errors

        Raises `MissingKeyValue` if any row had no key value."""
        if self._rows_without_key:
            columns = ", ".join(repr(column) for column in sorted(self._key_columns_without_value))
            raise MissingKeyValue(
                f"table {self._table!r}: {self._rows_without_key} row(s) have no value for key column "
                f"{columns}; a deduplicating load needs every row to carry its key"
            )
        return list(self._best.values())


def deduplicate(
    rows: Iterable[dict[str, Any]],
    *,
    key: tuple[str, ...],
    cursor: str | None,
    table: str,
    limits: DedupLimits | None = None,
) -> list[dict[str, Any]]:
    """The pure function behind the step: one row per `key` from `rows`, by the
    rules of this module's docstring.

    # Errors

    Raises `MissingKeyValue`, `TooManyKeys`, `IncomparableCursor` or
    `UnsupportedForDedup`; see `Winners`."""
    winners = Winners(key=key, cursor=cursor, table=table, limits=limits)
    for row in rows:
        winners.add(row)
    return winners.result()


def dedup_rows(
    rows: Iterable[dict[str, Any]],
    *,
    key: tuple[str, ...],
    cursor: str | None,
    table: str,
    limits: DedupLimits | None = None,
) -> Iterator[dict[str, Any]]:
    """`deduplicate` as a generator, for a plain iterable of dicts. Nothing is
    yielded until `rows` is exhausted (and its refusals are raised then)."""
    winners = deduplicate(rows, key=key, cursor=cursor, table=table, limits=limits)
    if winners:
        write_marker(key)
    yield from winners


def write_marker(key: tuple[str, ...], resource_name: str | None = None) -> None:
    """Record `key` as the columns this resource's rows were deduplicated on. Must
    run while `dlt` extracts (inside the resource), where the resource state is
    writable and travels with the package."""
    dlt.current.resource_state(resource_name)[MARKER_KEY] = list(key)


def markers(state: dict[str, Any]) -> list[list[str]]:
    """Every marker a pipeline state holds, across all source schemas and
    resources (the same walk as `sink._incremental_cursors`)."""
    found: list[list[str]] = []
    for source_state in (state.get("sources") or {}).values():
        for resource_state in (source_state.get("resources") or {}).values():
            marker = resource_state.get(MARKER_KEY)
            if isinstance(marker, list):
                found.append([str(column) for column in marker])
    return found


def clear_marker(resource_name: str | None = None) -> None:
    """Remove the marker: a load that is NOT a deduplicating one wrote rows that
    may repeat a key, so the table is no longer known to hold one row per key.
    `sink._stamp_for_load` calls it from the stamp of every such load, inside the
    extract like `write_marker`. A no-op, changing no state, when there is no
    marker, so such a load costs no extra package."""
    dlt.current.resource_state(resource_name).pop(MARKER_KEY, None)


class _EndOfRows(list):
    """The empty-list marker the wrapped data generator appends after its last
    page; see the module docstring."""


def _marked(items: Iterator[Any]) -> Iterator[Any]:
    yield from items
    yield _EndOfRows()


def _identity(item: Any) -> Any:  # pragma: no cover -- `ItemTransform` wants a one-argument function
    return item


class DedupStep(ItemTransform[Any, dict]):
    """The `dlt` pipe step; see the module docstring, "How it attaches"."""

    def __init__(self, winners: Winners) -> None:
        super().__init__(_identity)
        self.winners = winners
        self.flushed = False
        self._marked_gen: Any = None
        self._pipe_name: str | None = None

    def bind(self, pipe: Any) -> "DedupStep":
        # `dlt` calls `bind` each time it evaluates the pipe; wrap the data
        # generator once per generator.
        self._pipe_name = pipe.name
        if pipe.gen is not self._marked_gen:
            self._marked_gen = _marked(pipe.gen)
            pipe.replace_gen(self._marked_gen)
        return self

    def __call__(self, item: Any, meta: Any = None) -> Any:
        if isinstance(item, _EndOfRows):
            rows = self.winners.result()
            self.flushed = True
            if rows:
                write_marker(self.winners.key, self._pipe_name)
            return rows or None
        if isinstance(item, list):
            for row in item:
                self.winners.add(row)
        else:
            self.winners.add(item)
        return None


def attach(
    source: Any, *, key: tuple[str, ...], cursor: str | None, table: str, limits: DedupLimits | None = None
) -> tuple[Any, list[DedupStep]]:
    """Put the deduplication in front of the sink's stamp, whatever shape `source`
    is. Returns the source to load and the steps attached (empty for plain rows),
    which `assert_flushed` checks after the run.

    # Errors

    Raises `UnsupportedForDedup` for a `dlt` resource whose pipe holds a step
    other than the incremental cursor and a map: such a step could drop the
    end-of-rows marker, and the winners would then never be emitted."""
    limits = limits or DedupLimits.from_env()
    if isinstance(source, DltSource):
        resources = list(source.selected_resources.values())
    elif isinstance(source, DltResource):
        resources = [source]
    else:
        return dedup_rows(source, key=key, cursor=cursor, table=table, limits=limits), []
    steps: list[DedupStep] = []
    for resource in resources:
        for step in resource._pipe.steps[1:]:
            if not isinstance(step, (IncrementalResourceWrapper, MapItem)):
                raise UnsupportedForDedup(
                    f"table {table!r}: resource {resource.name!r} has a pipe step "
                    f"({type(step).__name__}) a deduplicating load cannot be combined with"
                )
        step = DedupStep(Winners(key=key, cursor=cursor, table=table, limits=limits))
        # `insert_step` at the end, not `add_map`/`append_step`: those honour the
        # incremental step's "stick to the end" affinity and would put this step
        # BEFORE the cursor filter (the same reason `sink._stamp_last` inserts).
        resource._pipe.insert_step(step, len(resource._pipe))
        steps.append(step)
    return source, steps


def assert_flushed(steps: list[DedupStep], *, table: str) -> None:
    """After a successful run: every step saw the end of its rows. A run that
    only finished a package an earlier run left pending does not extract its own
    read (`SRC-10` measurement 4); its winners were never emitted.

    # Errors

    Raises `DedupRefused` if a step did not flush."""
    if not all(step.flushed for step in steps):
        raise DedupRefused(f"table {table!r}: the rows of this load were not read")
