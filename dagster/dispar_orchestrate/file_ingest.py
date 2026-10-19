"""An uploaded file, loaded into a raw Bronze table. One Dagster run per file.

`POST /api/uploads/{id}/ingest` launches `file_ingest_job` with the key the
API stored the file under, the table to load, whether to replace its rows or
add to them, and the three settings a person confirmed on the preview
screen: encoding, delimiter and header row. Nothing here guesses WHAT to
load, or how to read it, from the environment (ADR 0014, decisions 3 to 5;
plan T7 in `docs/superpowers/plans/2026-10-02-upload-file.md`). The job
writes no Iceberg of its own: the rows go through the shared sink
(`adapters/sink.py::load_via_sink`). It writes nothing to Postgres either,
because it holds no credentials for the console's database.

# Reading: the dialect, and what the load adds to it

The preview (`upload_parse.rs`) and this load are two readers of one
dialect, pinned by the fixture files in `ops/fixtures/uploads/`, which the
tests of both read: `csv.reader(io.StringIO(text, newline=""), delimiter=d)`
over text decoded as that directory's README says, with the field size
limit raised to what the preview has (none). On top of it the load applies
three rules of its own. Cells are stored as they are, never trimmed: the
preview shows them untrimmed, whitespace is data, and "Bronze keeps what the
source said" (ADR 0014, decision 4); the sketch this replaces stripped every
value. Whitespace only decides whether a record is blank (skipped below the
header) and how a column is named. A row shorter than the header is padded
with empty strings and a longer one is cut to the header's length, which
drops its extra cells. And the row cap (`MAX_ROWS`) is a failure, never a
truncation: the load that passes it fails, and nothing is written.

# The file is streamed, never held whole (`DATA-17` T2.1, plan AD2)

The job is the only reader of a whole file, and it reads it in 8 MiB blocks
(`READ_BLOCK`): bytes go through an incremental decoder, text through a line
splitter that cuts where `io.StringIO(newline="")` cuts (a lone `\r`, a `\n` or
a `\r\n`, with a `\r` at the end of a block held back until the next block shows
whether a `\n` follows), and the lines through `csv.reader`. Rows are counted
as they pass and leave in Arrow record batches of `BATCH_ROWS` (50,000) rows, so
the memory of a load is a few batches and not the file; `test_file_ingest.py`
measures that with `tracemalloc` on a 200 MB generated file. A single line with
no line break at all (one 2 GB record) would still be held whole: nothing real
produces one, and the column and cell rules would refuse it later.

Because the file is read once, the first data row is read BEFORE dlt is asked
to write anything, so a header past the end, a header with no cells, one with
too many, and a file with no rows below it all fail without a pipeline having
been made. The one failure that can only be found mid-stream is the row cap
(`MAX_ROWS`): the batch generator raises it from inside dlt's extract step, which
runs before anything is normalized or loaded, so nothing is written (measured:
`test_a_failure_while_dlt_extracts_leaves_the_destination_untouched`).

# Parquet is read here too (`DATA-17` T2.2)

With `format: parquet` the object is a Parquet file, opened for reading by ranges
(`pyarrow.parquet.ParquetFile` over the store's file object) and read in batches
of `BATCH_ROWS`. `parquet_ingest.py` writes every value as the text the API's
converter used to write, so the table is what it was when the API converted the
file; the same row, column and cell caps apply, and a binary or nested column is
refused (`UNREADABLE`, the API having refused the same file at upload).

# Every column is text, and dlt has to be told

Decision 6 of the feature page says every column is text. Plain rows do not
give that: dlt 1.30.0 detects ISO timestamps in strings and types the column
`timestamptz`. `_text_resource` declares every column `text` instead, and
`_column_names` and `_check_settings` keep the names dlt would otherwise
rewrite or merge; their docstrings carry the measurements.

# One outcome row per run, from a closed set of reasons

The API reads the result back from `lake.bronze_meta.ingest_run`
(`connector_id = "upload:<upload id>"`), so each run records exactly one row,
through `record_ingest_run`: `succeeded` with the sink's measured `rows`
(NULL when it could not measure, never `len(rows)`), or a failure whose
`error` is one of `FAILURE_REASONS` and never `str(exc)` (review finding B6).
That list equals `ops/fixtures/upload_load_failure_reasons.json`, the file the
API's constants are tested against too. Only a failure after the data was
written (its catalog entry) keeps the sink's measured `rows`; any other
failure records NULL. The detail goes to the run log, and the failure is
re-raised after the row is written so the run is red.

# No automatic retry

The op carries `DEFAULT_RETRY_POLICY`, as every op here must, and never lets
it fire: each failure leaves as `Failure(allow_retries=False)`. A second
attempt would record a second row, and an `append` that failed after it wrote
would add the rows again. "Try again" in the console is the retry, and it
starts a new run.
"""

from __future__ import annotations

import codecs
import csv
import io
import itertools
import re
import sys
from collections.abc import Callable, Iterable, Iterator, Mapping
from contextlib import contextmanager
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any, BinaryIO

import dlt
import pyarrow as pa
import s3fs
from dagster import Failure, Field, job, op
from dlt.extract.resource import DltResource

from dispar_orchestrate.adapters.sink import LoadPlan, SinkConfig, SinkResult, load_via_sink
from dispar_orchestrate.bronze_catalog import record_ingest_run
from dispar_orchestrate.connector_catalog import register_loaded_table
from dispar_orchestrate.dlt_pipeline import BronzeIngestConfig
from dispar_orchestrate.parquet_ingest import UnsupportedColumn, check_types, column_names_of, open_parquet, text_batches
from dispar_orchestrate.op_metadata import DEFAULT_RETRY_POLICY, source_metadata

JOB_NAME = "file_ingest_job"

# `ingest_run.connector_id` of an upload's run: what the API asks for.
CONNECTOR_PREFIX = "upload:"

# What the shared catalog shows as the table's author.
AUTHOR = "upload"

# The API stores every upload under this prefix of the warehouse bucket, and
# nothing else lives there. A key outside it is refused, so a mistake on the
# caller's side cannot make this job read Bronze data out of the same bucket.
UPLOAD_PREFIX = "uploads/"

# The limit on a file's data rows (decision 2 of the feature page,
# `docs/core/features/upload-file.md`, and "Limits" of the plan). The API
# checks a file's size and kind when it is uploaded and never counts its rows
# (`routes/uploads.rs` only knows this limit as the reason `JOB_TOO_MANY_ROWS`),
# so this is where the limit is enforced, as a failure and never a truncation.
MAX_ROWS = 2_000_000

# The limit on a file's columns (`SEC-17`): the header record's cells under the
# reading the load was told. A header decided how many columns a table got and
# nothing bounded it, so a 50 MB file of commas asked for tens of millions. The
# API refuses a wider file at the preview and at the ingest request
# (`upload_parse::MAX_COLUMNS`) from the first 256 KiB; this counts the whole
# file and is the authority, so it is also checked for a load that did not come
# through those routes.
MAX_COLUMNS = 1_000

# How the file is read and handed on (`DATA-17` T2.1): blocks of 8 MiB from the
# store, Arrow record batches of 50,000 rows to the sink.
READ_BLOCK = 8 * 1024 * 1024
BATCH_ROWS = 50_000

# The API's own bound on a table name (`routes::uploads::MAX_TABLE_NAME_CHARS`).
MAX_TABLE_NAME_CHARS = 128

# What an uploaded table may be called, `^[a-z][a-z0-9]*(_[a-z0-9]+)*$`: a
# lower-case letter first, then lower-case letters and digits in groups joined
# by single underscores, so no leading, trailing or doubled `_` (review finding
# C1). The API states and applies the same rule (`TABLE_NAME_RULE` and
# `table_name_problem` in `routes/uploads.rs`), so a name this job would refuse
# is refused where the user can read why. It is tighter than a connector
# target's rule (`connector_catalog.is_plain_table_name`, which connectors
# keep) because dlt writes some names that rule admits under another name and,
# measured, none that this one admits: see `_dlt_keeps_table_name`. Matched
# with `fullmatch`, since a `$` would also match in front of a trailing newline.
TABLE_NAME = re.compile(r"[a-z][a-z0-9]*(_[a-z0-9]+)*")

# Exactly what the API validates before it launches the job, and so exactly
# what this job accepts: a value outside these is a launch that did not come
# from the API, and fails closed.
LOAD_MODES = ("replace", "append")
# What the stored objects are: delimited text (a CSV or TSV, or the text the API
# converted a workbook to), or a Parquet file, which the job reads natively
# (`DATA-17` T2.2, `parquet_ingest.py`).
FORMATS = ("delimited", "parquet")
ENCODINGS = ("utf-8", "utf-16")
DELIMITERS = (",", ";", "\t", "|")

# The reasons a failed load records, in the order of
# `ops/fixtures/upload_load_failure_reasons.json` (review finding B6): the
# API shows a recorded reason only when it is one of these eight, and
# `test_file_ingest.py` asserts this list against that file.
UNREADABLE = "The stored file could not be read."
HEADER_PAST_END = "The header row is past the end of the file."
# Review finding C2: a record at the header row that has no cells (an empty
# line chosen as the header) used to be recorded as "past the end", which is
# not what happened: the file has a record there.
HEADER_NO_COLUMNS = "The header row has no columns."
NO_ROWS = "The file has no rows below the header row."
TOO_MANY_ROWS = "The file has more than 2,000,000 rows."
LOAD_FAILED = "The load into the table failed."
NOT_REGISTERED = "The table was loaded but could not be registered in the catalog."
# `SEC-17`: appended last, so the fixture's order of the first seven is kept.
TOO_MANY_COLUMNS = "The file has more than 1,000 columns."

FAILURE_REASONS = (
    UNREADABLE,
    HEADER_PAST_END,
    HEADER_NO_COLUMNS,
    NO_ROWS,
    TOO_MANY_ROWS,
    LOAD_FAILED,
    NOT_REGISTERED,
    TOO_MANY_COLUMNS,
)


class LoadFailure(Exception):
    """A load that ended without loading, with the one sentence that is
    recorded for it.

    The constructor refuses any sentence that is not in `FAILURE_REASONS`, so
    no other text can reach `ingest_run.error` through this class. The
    exception the failure came from is its `__cause__`: it belongs in the run
    log, never in the recorded row (review finding B6).

    `rows` is the count the sink measured, set when the data was written and
    a later step failed.
    """

    def __init__(self, reason: str, *, rows: int | None = None) -> None:
        if reason not in FAILURE_REASONS:
            raise ValueError(f"{reason!r} is not one of the failure reasons the API shows")
        super().__init__(reason)
        self.reason = reason
        self.rows = rows


class OutcomeNotRecorded(Exception):
    """`record_ingest_run` itself failed. There is nothing left to record
    the outcome in, so the run fails; the API then settles the upload as
    "The load stopped before it recorded a result."."""


@dataclass(frozen=True)
class FileIngestParams:
    """What to load and how to read it, as `routes::uploads::run_config`
    sends it under `ops.ingest_uploaded_file.config`.

    Every field is required, in `CONFIG_SCHEMA` as here. A parse nobody
    confirmed is not replaced by a default guess (ADR 0014, decision 3).
    """

    #: `file_upload.id`. The outcome is filed under `upload:<id>`.
    upload_id: str
    #: Object keys in the warehouse bucket, `uploads/<tenant id>/<upload id>[.<ext>]`,
    #: built by the API from server-generated parts only; the primary file first.
    #: One key until a load can take several files (a later slice of `DATA-17`).
    storage_keys: tuple[str, ...]
    #: What the objects are: one of `FORMATS`.
    format: str
    #: The raw table. The API checks the naming rule; `_check_settings` checks it again.
    bronze_table_name: str
    #: `replace` or `append`.
    load_mode: str
    #: `utf-8` or `utf-16`.
    encoding: str
    #: One character: `,` `;` `|` or a real tab.
    delimiter: str
    #: Zero-based index of the RECORD that holds the column names. Blank
    #: records and records that span lines count as the reader sees them.
    header_row: int

    @classmethod
    def from_op_config(cls, config: Mapping[str, Any]) -> FileIngestParams:
        """The params out of a run's `op_config`, which Dagster has already
        validated against `CONFIG_SCHEMA`."""
        values = {name: config[name] for name in CONFIG_SCHEMA}
        values["storage_keys"] = tuple(values["storage_keys"])
        return cls(**values)


# Why the op declares its config this way and not as a `dagster.Config`
# class: with `from __future__ import annotations` Dagster cannot resolve a
# `Config` subclass from the op's annotation. `ingest_factory.py` and
# `agent_runs.py` use the same `config_schema` form.
CONFIG_SCHEMA = {
    "upload_id": Field(str, description="`file_upload.id`; the outcome is recorded under `upload:<id>`."),
    "storage_keys": Field(
        [str], description="Object keys inside the warehouse bucket, under `uploads/`; the primary file first."
    ),
    "format": Field(str, description="What the objects are: `delimited` or `parquet`."),
    "bronze_table_name": Field(str, description="The raw table to load into."),
    "load_mode": Field(str, description="`replace` or `append`."),
    "encoding": Field(str, description="`utf-8` or `utf-16`."),
    "delimiter": Field(str, description="One character: a comma, a semicolon, a pipe or a real tab."),
    "header_row": Field(int, description="Zero-based index of the record that holds the column names."),
}


@dataclass(frozen=True)
class LoadSummary:
    """What a load that succeeded did."""

    table: str
    #: Rows the sink measured, or None when it could not (never a guess).
    rows: int | None
    #: Data rows the file held, counted before anything was written.
    parsed_rows: int
    columns: int
    #: The table's total after the load, as registered in the catalog.
    table_total: int


# ── Reading the file ─────────────────────────────────────────────────────


def _text_chunks(chunks: Iterable[bytes], encoding: str) -> Iterator[str]:
    """Decode blocks of bytes as `upload_parse::decode` does, one block at a
    time, so the table holds what the preview showed. UTF-8: a byte order mark is
    dropped (left in place it turns the quotes of a quoted first cell into
    literal characters). UTF-16: the byte order mark picks the byte order, and
    without one it is little endian on every platform. A trailing odd byte is
    dropped, as the preview drops it (a block of odd length keeps its last byte
    for the next block, so a unit is never split). A byte that does not belong
    becomes U+FFFD, visible in the table and never fatal, so one bad byte does
    not fail a 35,000-row export. An incremental decoder keeps a character or a
    surrogate pair that straddles two blocks whole.
    """
    if encoding == "utf-16":
        decoder: codecs.IncrementalDecoder | None = None
        carry = b""
        for chunk in chunks:
            data = carry + chunk
            if decoder is None:
                if len(data) < 2:
                    carry = data
                    continue
                if data.startswith(b"\xff\xfe"):
                    codec, data = "utf-16-le", data[2:]
                elif data.startswith(b"\xfe\xff"):
                    codec, data = "utf-16-be", data[2:]
                else:
                    codec = "utf-16-le"
                decoder = codecs.getincrementaldecoder(codec)(errors="replace")
            usable = len(data) - len(data) % 2
            carry = data[usable:]
            text = decoder.decode(data[:usable])
            if text:
                yield text
        # `carry` is a trailing odd byte, or a lone byte of a one-byte file:
        # dropped. A pending half of a surrogate pair is a bad unit.
        if decoder is not None:
            tail = decoder.decode(b"", final=True)
            if tail:
                yield tail
        return
    utf8 = codecs.getincrementaldecoder("utf-8-sig")(errors="replace")
    for chunk in chunks:
        text = utf8.decode(chunk)
        if text:
            yield text
    tail = utf8.decode(b"", final=True)
    if tail:
        yield tail


_LINE = re.compile(r"[^\r\n]*(?:\r\n|\n|\r)")


def _lines(texts: Iterable[str]) -> Iterator[str]:
    """Cut text that arrives in blocks into lines, line breaks kept, exactly
    where `io.StringIO(text, newline="")` cuts: at `\r\n`, `\n` or a lone `\r`
    (and nowhere else: `str.splitlines` also cuts at form feeds and U+2028, which
    the csv reader reads as data). A `\r` at the very end of what is buffered is
    held until the next block shows whether a `\n` follows, so a `\r\n` split
    between two blocks is not read as two line breaks.
    """
    buffer = ""
    for text in texts:
        buffer += text
        end = 0
        for match in _LINE.finditer(buffer):
            if match.end() == len(buffer) and buffer.endswith("\r"):
                break
            yield match.group()
            end = match.end()
        buffer = buffer[end:]
    if buffer:
        for match in _LINE.finditer(buffer):
            yield match.group()
        rest = _LINE.sub("", buffer, count=0)
        if rest:
            yield rest


def _is_blank(record: list[str]) -> bool:
    """Every cell empty or whitespace (`str.isspace()`, the set the preview
    spells out in `upload_parse::is_space`). An empty line is blank."""
    return all(not cell or cell.isspace() for cell in record)


def read_table(
    source: str | Iterable[str], delimiter: str, header_row: int
) -> tuple[list[str], Iterator[list[str]]]:
    """The header record and the records below it that are not blank, as the
    dialect reads them: raw cells, not padded, not cut, not trimmed. `source` is
    the text, or its lines with their line breaks (`_lines`).

    `header_row` is a zero-based index of a RECORD, blank ones included, which
    is what `ops/fixtures/uploads/*.expected.json` calls `headerRow`.

    # Errors

    Raises `LoadFailure(HEADER_PAST_END)` when the text has no record at that
    index.
    """
    lines = io.StringIO(source, newline="") if isinstance(source, str) else source
    records = csv.reader(lines, delimiter=delimiter)
    for index, record in enumerate(records):
        if index == header_row:
            return record, (row for row in records if not _is_blank(row))
    raise LoadFailure(HEADER_PAST_END)


def _column_names(header: list[str]) -> list[str]:
    """Header cells turned into column names that are unique, only
    `[a-z0-9_]`, and unchanged by dlt's own naming.

    Real exports have blank header cells, duplicated labels and names like
    `Material description` or `A.scrap`. A blank cell becomes `col_<index>`, a
    name that starts with a digit gets `col_` in front, and a repeat gets
    `_2`, `_3`, and so on. Two things the first version of this rule (the
    sketch from `f9793cd`) got wrong, measured with dlt 1.30.0 on 2026-10-02
    through `load_via_sink` against a local filesystem bucket:

    - It kept non-ASCII letters, and dlt rewrites them: `größe` was written as
      `gr_e`, and two different names, `名前` and `値`, both became `x`. dlt
      logged "got normalized into x which collides with other column. Both
      columns got merged into one" and kept one column's values: silent data
      loss. Only ASCII letters and digits are names here; any other character
      is a separator, as every punctuation mark already was.
    - A suffixed repeat could equal another column's own name: `a, a, a_2`
      gave `a, a_2, a_2`, and the second `a_2` overwrote the first. The
      suffix now moves on past any name taken.

    A header whose names the first version produced without either problem
    gets the same names.
    """
    names: list[str] = []
    taken: set[str] = set()
    seen: dict[str, int] = {}
    for index, cell in enumerate(header):
        base = "".join(c.lower() if c.isascii() and c.isalnum() else "_" for c in cell.strip()).strip("_")
        while "__" in base:
            base = base.replace("__", "_")
        if not base or base[0].isdigit():
            base = f"col_{index}" if not base else f"col_{base}"
        seen[base] = seen.get(base, 0) + 1
        name = base if seen[base] == 1 else f"{base}_{seen[base]}"
        while name in taken:
            seen[base] += 1
            name = f"{base}_{seen[base]}"
        taken.add(name)
        names.append(name)
    return names


def _shape(record: list[str], width: int) -> list[str]:
    """The record at the header's width: padded with empty strings when
    shorter, cut when longer."""
    if len(record) >= width:
        return record[:width]
    return record + [""] * (width - len(record))


@dataclass
class TableStream:
    """A file read once, from its first data row to its last, handed on in
    Arrow record batches. It holds the columns and the row in hand, never the
    rows: `batches()` is the only producer of rows and counts them as they pass.

    `failure` is set when the stream itself ends the load (the row cap, or the
    store failing mid-read) so that `_write` can tell that from a failure of
    dlt, whose exception wraps whatever the generator raised.
    """

    columns: list[str]
    first: list[str]
    records: Iterator[list[str]]
    row_count: int = 0
    failure: LoadFailure | None = field(default=None, repr=False)

    def _table(self, cells: list[list[str]]) -> pa.Table:
        return pa.table({name: pa.array(values, type=pa.string()) for name, values in zip(self.columns, cells)})

    def batches(self) -> Iterator[pa.Table]:
        """The data rows as Arrow tables of at most `BATCH_ROWS` rows, every
        column `string` and every value the text it is.

        # Errors

        Raises `LoadFailure(TOO_MANY_ROWS)` (also kept in `failure`) at the row
        after `MAX_ROWS`, before it is added to a batch: a file over the cap is
        a failure and never a truncation. Raises `LoadFailure(UNREADABLE)` when
        the store fails while the file is read.
        """
        width = len(self.columns)
        cells: list[list[str]] = [[] for _ in range(width)]
        try:
            for record in itertools.chain([self.first], self.records):
                self.row_count += 1
                if self.row_count > MAX_ROWS:
                    self.failure = LoadFailure(TOO_MANY_ROWS)
                    raise self.failure
                for column, value in zip(cells, _shape(record, width)):
                    column.append(value)
                if len(cells[0]) >= BATCH_ROWS:
                    yield self._table(cells)
                    cells = [[] for _ in range(width)]
        except LoadFailure:
            raise
        except Exception as exc:  # noqa: BLE001 -- the store or the decoder failed mid-read; the detail is the cause
            self.failure = LoadFailure(UNREADABLE)
            self.failure.__cause__ = exc
            raise self.failure from exc
        if cells[0]:
            yield self._table(cells)


def open_table(source: str | Iterable[str], delimiter: str, header_row: int) -> TableStream:
    """Read a file as far as its first data row, writing nothing.

    # Errors

    Raises `LoadFailure` with `HEADER_PAST_END` (no record at `header_row`),
    `HEADER_NO_COLUMNS` (a record there, with no cells at all: an empty line;
    the preview reports it and the case above alike as an empty `columns`, and
    the two have a sentence each since review finding C2), `NO_ROWS` (nothing
    below the header that is not blank) or `TOO_MANY_COLUMNS` (a header of more
    than `MAX_COLUMNS` cells, found before any row is read). The row cap is
    found later, by `TableStream.batches`.
    """
    header, records = read_table(source, delimiter, header_row)
    if not header:
        raise LoadFailure(HEADER_NO_COLUMNS) from ValueError("the header record has no cells")
    # `SEC-17`: before the first row is read, so a file that cannot load costs
    # nothing beyond its header.
    if len(header) > MAX_COLUMNS:
        raise LoadFailure(TOO_MANY_COLUMNS) from ValueError(
            f"the header record has {len(header)} cells, more than {MAX_COLUMNS}"
        )
    first = next(records, None)
    if first is None:
        raise LoadFailure(NO_ROWS)
    return TableStream(columns=_column_names(header), first=first, records=records)


@dataclass
class ParquetStream:
    """A Parquet file read in batches, with the interface of `TableStream`:
    `columns` (cleaned as a header would be), `batches()`, `row_count` and
    `failure`. The text of every value is `parquet_ingest`'s."""

    parquet_file: Any
    columns: list[str]
    row_count: int = 0
    failure: LoadFailure | None = field(default=None, repr=False)

    def batches(self) -> Iterator[pa.Table]:
        """The rows as tables of `string` columns, at most `BATCH_ROWS` each.

        # Errors

        Raises `LoadFailure(TOO_MANY_ROWS)` (also kept in `failure`) before the
        batch that would pass `MAX_ROWS`, and `LoadFailure(UNREADABLE)` when the
        store or the file fails while it is read.
        """
        try:
            for batch in text_batches(self.parquet_file, BATCH_ROWS):
                if self.row_count + batch.num_rows > MAX_ROWS:
                    self.failure = LoadFailure(TOO_MANY_ROWS)
                    raise self.failure
                self.row_count += batch.num_rows
                yield batch.rename_columns(self.columns)
        except LoadFailure:
            raise
        except Exception as exc:  # noqa: BLE001 -- the store or a page failed mid-read; the detail is the cause
            self.failure = LoadFailure(UNREADABLE)
            self.failure.__cause__ = exc
            raise self.failure from exc


def open_parquet_stream(handle: Any) -> ParquetStream:
    """Open a Parquet file as far as its footer, writing nothing.

    # Errors

    Raises `LoadFailure` with `UNREADABLE` (the file does not open, or a column
    has no text form: binary or nested), `HEADER_NO_COLUMNS` (no columns),
    `TOO_MANY_COLUMNS` or `NO_ROWS`.
    """
    try:
        parquet_file = open_parquet(handle)
        names = column_names_of(parquet_file)
        rows = parquet_file.metadata.num_rows
    except Exception as exc:  # noqa: BLE001 -- not Parquet, cut short, encrypted or damaged
        raise LoadFailure(UNREADABLE) from exc
    if not names:
        raise LoadFailure(HEADER_NO_COLUMNS) from ValueError("the file has no columns")
    if len(names) > MAX_COLUMNS:
        raise LoadFailure(TOO_MANY_COLUMNS) from ValueError(f"the file has {len(names)} columns, more than {MAX_COLUMNS}")
    try:
        check_types(parquet_file)
    except UnsupportedColumn as exc:
        raise LoadFailure(UNREADABLE) from exc
    if rows == 0:
        raise LoadFailure(NO_ROWS)
    return ParquetStream(parquet_file=parquet_file, columns=_column_names(names))


def _checked_key(storage_key: str) -> str:
    """The key, if it is inside the uploads prefix and cannot climb out of it.

    # Errors

    Raises `LoadFailure(UNREADABLE)` for any other key: it is not read.
    """
    if not storage_key.startswith(UPLOAD_PREFIX) or storage_key == UPLOAD_PREFIX or ".." in storage_key:
        raise LoadFailure(UNREADABLE) from ValueError(
            f"storage_key must start with {UPLOAD_PREFIX!r}, name an object, and contain no '..'"
        )
    return storage_key


def open_stored_object(
    config: SinkConfig, key: str, *, filesystem_factory: Callable[..., Any] | None = None
) -> BinaryIO:
    """The object, opened for streaming from the warehouse bucket with the
    sink's own credentials. `key` has passed `_checked_key`. Nothing is read
    here: the caller reads it in `READ_BLOCK` blocks and closes it."""
    make_fs = filesystem_factory or s3fs.S3FileSystem
    fs = make_fs(
        key=config.rustfs_access_key,
        secret=config.rustfs_secret_key,
        client_kwargs={"endpoint_url": config.rustfs_endpoint},
    )
    return fs.open(f"{config.warehouse_bucket}/{key}", "rb", block_size=READ_BLOCK)


def _blocks(handle: BinaryIO) -> Iterator[bytes]:
    """The object in blocks of `READ_BLOCK` bytes, until it ends."""
    while True:
        block = handle.read(READ_BLOCK)
        if not block:
            return
        yield block


@contextmanager
def _unbounded_csv_fields() -> Iterator[None]:
    """Raise Python's per-cell limit (131,072 characters) to none, as the
    preview has none: a file the preview showed must not fail to load. The
    limit is process-wide, so it is put back afterwards."""
    previous = csv.field_size_limit(sys.maxsize)
    try:
        yield
    finally:
        csv.field_size_limit(previous)


# ── Writing it ───────────────────────────────────────────────────────────


def _text_resource(stream: TableStream | ParquetStream, table: str) -> DltResource:
    """The record batches as a dlt resource with every column declared `text`,
    for `load_via_sink` (which takes an already configured resource).

    Plain rows would let dlt type what it recognises. Measured with dlt 1.30.0
    on 2026-10-02 through `load_via_sink` against a local filesystem bucket
    and dlt's own in-memory SQLite catalog: a column holding `2025-02-07T10:00:00Z` came
    back `timestamptz`, and when other values of that column were not
    timestamps dlt put them in a new `<column>__v_text` column and left NULLs
    behind. With `data_type: text` declared, every value came back exactly as
    written. The batches are Arrow tables whose columns are `string`, which
    carries the same declaration; the hint stays so the intent is written where
    the reader looks. `ops/g9/upload_test.py` checks the types on a deployed
    stack.
    """
    resource = dlt.resource(stream.batches(), name=table)
    resource.apply_hints(columns=[{"name": name, "data_type": "text"} for name in stream.columns])
    return resource


def _dlt_keeps_table_name(table: str) -> bool:
    """Whether dlt writes the table under this name: the guard behind
    `TABLE_NAME`. dlt runs a name through its naming convention, and the rule
    the API and this job had before review finding C1 (`^[a-z_][a-z0-9_]*$`)
    admitted names that convention changes: `x_` is written as `xx`, `__x` as
    `x`, `s__1` as `s___1`. The load then lands in a table nobody asked for,
    the sink reports no row count, and registering the name that was asked for
    fails. Measured with dlt 1.30.0 on 2026-10-02, through `load_via_sink`
    against a local filesystem bucket, on 25 names that rule admitted: 14 were
    not written under the name given (one of them, `__`, failed outright) and
    11 were. `normalize_tables_path` agreed with the writer on all 25, so it is
    what is asked here instead of a second pattern of our own.

    `TABLE_NAME` admits none of those. Of the 59,052 names it matches that the
    reviewer put through the naming on 2026-10-02 (plan section 9, review of
    slice C), the naming changed none, and a test of this module enumerates
    short names to the same end. So with this dlt the guard refuses nothing
    that passed the pattern; it is here for the day a newer dlt names things
    differently, when it refuses the launch before the file is read and not
    after the rows landed in the wrong table.
    """
    return dlt.Schema("upload").naming.normalize_tables_path(table) == table


def _check_settings(params: FileIngestParams) -> None:
    """Refuse a launch that the API would not have sent, before anything is
    read: this job may be launched by something else, and the table name is
    interpolated into SQL when the table is registered.

    # Errors

    Raises `LoadFailure(LOAD_FAILED)`, with the problems as its cause, which
    names no reason of its own: none of the other six describes a setting.
    """
    problems: list[str] = []
    if not params.upload_id.strip():
        problems.append("upload_id is empty")
    if params.load_mode not in LOAD_MODES:
        problems.append(f"load_mode {params.load_mode!r} is not one of {', '.join(LOAD_MODES)}")
    if params.encoding not in ENCODINGS:
        problems.append(f"encoding {params.encoding!r} is not one of {', '.join(ENCODINGS)}")
    if params.delimiter not in DELIMITERS:
        problems.append(f"delimiter {params.delimiter!r} is not one of {DELIMITERS!r}")
    if params.header_row < 0:
        problems.append(f"header_row {params.header_row} is negative")
    if params.format not in FORMATS:
        problems.append(f"format {params.format!r} is not one of {', '.join(FORMATS)}")
    if len(params.storage_keys) != 1:
        problems.append(f"storage_keys holds {len(params.storage_keys)} keys; this version loads exactly one file")
    table = params.bronze_table_name
    # Review finding C1: the upload rule first, the same one the API refuses
    # with; dlt is asked only about a name that passed it. The SQL that
    # registers the table has its own guard (`register_loaded_table`), which
    # every name this pattern admits satisfies.
    if not TABLE_NAME.fullmatch(table) or len(table) > MAX_TABLE_NAME_CHARS:
        problems.append(
            f"bronze_table_name {table!r} is not a lower-case letter followed by lower-case letters and "
            f"digits in groups joined by single underscores, at most {MAX_TABLE_NAME_CHARS} characters"
        )
    elif not _dlt_keeps_table_name(table):
        problems.append(f"bronze_table_name {table!r} is a name dlt would write under another one")
    if problems:
        raise LoadFailure(LOAD_FAILED) from ValueError("; ".join(problems))


def _sink_config_from_env() -> SinkConfig:
    return SinkConfig.from_bronze_ingest_config(BronzeIngestConfig.from_env())


def _now() -> str:
    """RFC 3339 in UTC, the form the API parses `ended_at` from: six digits of
    microseconds and `+00:00`, never `Z`, and no fraction at all when the
    microsecond is 0 (`2026-10-02T10:00:00.123456+00:00`,
    `2026-10-02T10:00:05+00:00`). `ingest_run.ended_at` is a text column, so
    the API reads these exact characters back; a unit test of
    `routes::uploads::ended_after` pins that it reads this form (review
    finding C3)."""
    return datetime.now(timezone.utc).isoformat()


def _write(
    stream: TableStream | ParquetStream,
    params: FileIngestParams,
    sink_config: SinkConfig,
    load: Callable[..., SinkResult],
) -> SinkResult:
    table = params.bronze_table_name
    try:
        result = load(_text_resource(stream, table), table, sink_config, LoadPlan(mode=params.load_mode))
    except Exception as exc:  # noqa: BLE001 -- whatever dlt or the catalog raised is the cause, never the reason
        # dlt wraps what the generator raised: the stream knows if it was the
        # row cap or the store, and that is the reason, not "the load failed".
        if stream.failure is not None:
            raise stream.failure  # its own cause (the store's error) is kept
        raise LoadFailure(LOAD_FAILED) from exc
    if stream.failure is not None:
        raise stream.failure
    if result.has_failed_jobs:
        # `load_info_str` can carry a file path: it goes to the cause, which is
        # the run log, and never into the recorded reason.
        raise LoadFailure(LOAD_FAILED) from RuntimeError(f"dlt load had failed jobs: {result.load_info_str}")
    return result


def _register(
    params: FileIngestParams,
    result: SinkResult,
    register: Callable[..., int],
) -> int:
    try:
        return register(
            params.bronze_table_name,
            # The catalog is shared across tenants, so the description names the
            # upload and not its tenant or its file name.
            description=f"Bronze Iceberg table loaded from an uploaded file (upload {params.upload_id}).",
            author=AUTHOR,
        )
    except Exception as exc:  # noqa: BLE001 -- the data is in the table; only its catalog entry is missing
        raise LoadFailure(NOT_REGISTERED, rows=result.rows) from exc


def _load(
    params: FileIngestParams,
    log: Any,
    load_config: Callable[[], SinkConfig],
    open_object: Callable[[SinkConfig, str], BinaryIO],
    load: Callable[..., SinkResult],
    register: Callable[..., int],
) -> LoadSummary:
    _check_settings(params)
    key = _checked_key(params.storage_keys[0])
    try:
        sink_config = load_config()
    except Exception as exc:  # noqa: BLE001 -- e.g. an unreadable token file; nothing was read or written
        raise LoadFailure(LOAD_FAILED) from exc
    with _unbounded_csv_fields():
        try:
            handle = open_object(sink_config, key)
        except Exception as exc:  # noqa: BLE001 -- no such object, or no way to reach the store
            raise LoadFailure(UNREADABLE) from exc
        with handle:
            try:
                if params.format == "parquet":
                    stream = open_parquet_stream(handle)
                else:
                    lines = _lines(_text_chunks(_blocks(handle), params.encoding))
                    stream = open_table(lines, params.delimiter, params.header_row)
            except LoadFailure:
                raise
            except Exception as exc:  # noqa: BLE001 -- every other failure here is an unreadable file; the detail is the cause
                raise LoadFailure(UNREADABLE) from exc
            log.info(f"upload {params.upload_id}: {len(stream.columns)} columns to load, streaming")
            result = _write(stream, params, sink_config, load)
    log.info(f"upload {params.upload_id}: {stream.row_count} rows read")
    if result.rows is not None and result.rows != stream.row_count:
        log.warning(
            f"upload {params.upload_id}: the file held {stream.row_count} rows and the sink measured {result.rows}"
        )
    total = _register(params, result, register)
    return LoadSummary(
        table=params.bronze_table_name,
        rows=result.rows,
        parsed_rows=stream.row_count,
        columns=len(stream.columns),
        table_total=total,
    )


def run_file_load(
    params: FileIngestParams,
    *,
    log: Any,
    load_config: Callable[[], SinkConfig] = _sink_config_from_env,
    open_object: Callable[[SinkConfig, str], BinaryIO] = open_stored_object,
    load: Callable[..., SinkResult] = load_via_sink,
    register: Callable[..., int] = register_loaded_table,
    record: Callable[..., None] = record_ingest_run,
    now: Callable[[], str] = _now,
) -> LoadSummary:
    """Load one uploaded file and record exactly one `ingest_run` row for it.

    Everything that touches a network is a parameter, so a test drives this
    with fakes. `open_object` returns the stored object as a binary file. The row is written once, after the outcome is known, and is
    never retried: a second attempt after a failed insert could leave two
    rows for one run.

    # Errors

    Raises `LoadFailure` after recording it (`reason` is what was recorded),
    and `OutcomeNotRecorded` when the row itself could not be written.
    """
    started_at = now()
    failure: LoadFailure | None = None
    summary: LoadSummary | None = None
    try:
        summary = _load(params, log, load_config, open_object, load, register)
    except LoadFailure as caught:
        failure = caught
    except Exception as exc:  # noqa: BLE001 -- a bug in this module; the run still gets its one row
        failure = LoadFailure(LOAD_FAILED)
        failure.__cause__ = exc
    if failure is not None:
        cause = failure.__cause__
        detail = f" ({type(cause).__name__}: {cause})" if cause is not None else ""
        log.warning(f"upload {params.upload_id}: {failure.reason}{detail}")
    try:
        record(
            connector_id=f"{CONNECTOR_PREFIX}{params.upload_id}",
            job=JOB_NAME,
            object_name=params.bronze_table_name,
            rows=failure.rows if failure is not None else summary.rows,
            started_at=started_at,
            ended_at=now(),
            status="failed" if failure is not None else "succeeded",
            error=failure.reason if failure is not None else "",
        )
    except Exception as exc:  # noqa: BLE001 -- nowhere left to record it; the run fails with this as its cause
        raise OutcomeNotRecorded("the outcome of the load could not be written to ingest_run") from exc
    if failure is not None:
        raise failure
    return summary


# ── The job ──────────────────────────────────────────────────────────────


def _failure_description(exc: Exception) -> str:
    if isinstance(exc, LoadFailure):
        return exc.reason
    if isinstance(exc, OutcomeNotRecorded):
        return "The outcome of the load could not be recorded."
    return "The load stopped unexpectedly."


@op(
    config_schema=CONFIG_SCHEMA,
    retry_policy=DEFAULT_RETRY_POLICY,
    tags=source_metadata(
        "dispar_orchestrate/file_ingest.py::ingest_uploaded_file",
        reads=["RustFS warehouse bucket uploads/ (the uploaded file)"],
        writes=[
            "Iceberg bronze.{bronze_table_name}",
            "ClickHouse lake.bronze_meta.dataset_catalog, dataset_sync and dataset_column",
            "ClickHouse lake.bronze_meta.ingest_run",
        ],
    ),
)
def ingest_uploaded_file(context) -> dict:
    """Stream the uploaded object into the Bronze table, register it, and
    record the outcome. See the module docstring."""
    try:
        summary = run_file_load(FileIngestParams.from_op_config(context.op_config), log=context.log)
    except Exception as exc:  # noqa: BLE001 -- every failure leaves as one non-retryable Failure (module docstring)
        raise Failure(description=_failure_description(exc), allow_retries=False) from exc
    metadata: dict[str, Any] = {
        "bronze_table": summary.table,
        "rows_in_file": summary.parsed_rows,
        "columns": summary.columns,
        "table_total": summary.table_total,
    }
    if summary.rows is not None:
        metadata["rows_loaded"] = summary.rows
    context.add_output_metadata(metadata)
    return {"bronze_table_name": summary.table, "rows": summary.rows, "table_total": summary.table_total}


@job
def file_ingest_job() -> None:
    """`DAGSTER_LOCATION`-visible job name: `file_ingest_job`. Launched only
    with a run config (`POST /api/uploads/{id}/ingest`): it has no schedule
    and no sensor, because there is no such thing as "the" uploaded file."""
    ingest_uploaded_file()
