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
truncation: rows are counted in a first pass, before anything is written.

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

import csv
import io
import re
import sys
from collections.abc import Callable, Iterator, Mapping
from contextlib import contextmanager
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any

import dlt
import s3fs
from dagster import Failure, Field, job, op
from dlt.extract.resource import DltResource

from dispar_orchestrate.adapters.sink import LoadPlan, SinkConfig, SinkResult, load_via_sink
from dispar_orchestrate.bronze_catalog import record_ingest_run
from dispar_orchestrate.connector_catalog import register_loaded_table
from dispar_orchestrate.dlt_pipeline import BronzeIngestConfig
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
    #: Object key in the warehouse bucket, `uploads/<tenant id>/<upload id>[.<ext>]`,
    #: built by the API from server-generated parts only.
    storage_key: str
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
        return cls(**{name: config[name] for name in CONFIG_SCHEMA})


# Why the op declares its config this way and not as a `dagster.Config`
# class: with `from __future__ import annotations` Dagster cannot resolve a
# `Config` subclass from the op's annotation. `ingest_factory.py` and
# `agent_runs.py` use the same `config_schema` form.
CONFIG_SCHEMA = {
    "upload_id": Field(str, description="`file_upload.id`; the outcome is recorded under `upload:<id>`."),
    "storage_key": Field(str, description="Object key inside the warehouse bucket, under `uploads/`."),
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


def _decode(raw: bytes, encoding: str) -> str:
    """Decode as `upload_parse::decode` does, so the table holds what the
    preview showed. UTF-8: a byte order mark is dropped (left in place it
    turns the quotes of a quoted first cell into literal characters). UTF-16:
    the byte order mark picks the byte order, and without one it is little
    endian on every platform. A trailing odd byte is dropped, as the preview
    drops it. A byte that does not belong becomes U+FFFD, visible in the
    table and never fatal, so one bad byte does not fail a 35,000-row export.
    """
    if encoding == "utf-16":
        if raw.startswith(b"\xff\xfe"):
            codec, body = "utf-16-le", raw[2:]
        elif raw.startswith(b"\xfe\xff"):
            codec, body = "utf-16-be", raw[2:]
        else:
            codec, body = "utf-16-le", raw
        return body[: len(body) - len(body) % 2].decode(codec, errors="replace")
    return raw.decode("utf-8-sig", errors="replace")


def _is_blank(record: list[str]) -> bool:
    """Every cell empty or whitespace (`str.isspace()`, the set the preview
    spells out in `upload_parse::is_space`). An empty line is blank."""
    return all(not cell or cell.isspace() for cell in record)


def read_table(text: str, delimiter: str, header_row: int) -> tuple[list[str], Iterator[list[str]]]:
    """The header record and the records below it that are not blank, as the
    dialect reads them: raw cells, not padded, not cut, not trimmed.

    `header_row` is a zero-based index of a RECORD, blank ones included, which
    is what `ops/fixtures/uploads/*.expected.json` calls `headerRow`.

    # Errors

    Raises `LoadFailure(HEADER_PAST_END)` when the text has no record at that
    index.
    """
    records = csv.reader(io.StringIO(text, newline=""), delimiter=delimiter)
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


@dataclass(frozen=True)
class ParsedFile:
    """A file that was read once, to check and count it, and is read again,
    lazily, to load it. It holds the text and never the rows, so even a file
    at the row cap is not in memory as rows; and `rows()` is the only
    producer of rows, so what was counted is what is loaded."""

    text: str
    delimiter: str
    header_row: int
    columns: list[str]
    row_count: int

    def rows(self) -> Iterator[dict[str, str]]:
        """The data rows, one dict per row, every value the text it is. A fresh
        pass over the text on each call."""
        _, records = read_table(self.text, self.delimiter, self.header_row)
        width = len(self.columns)
        for record in records:
            yield dict(zip(self.columns, _shape(record, width)))


def parse_file(text: str, delimiter: str, header_row: int) -> ParsedFile:
    """Check a decoded file and count its rows, writing nothing.

    # Errors

    Raises `LoadFailure` with `HEADER_PAST_END` (no record at `header_row`),
    `HEADER_NO_COLUMNS` (a record there, with no cells at all: an empty line;
    the preview reports it and the case above alike as an empty `columns`, and
    the two have a sentence each since review finding C2), `NO_ROWS` (nothing
    below the header that is not blank) or `TOO_MANY_ROWS` (more than
    `MAX_ROWS`, found while counting, so nothing was cut) or `TOO_MANY_COLUMNS`
    (a header of more than `MAX_COLUMNS` cells, found before any row is read).
    """
    header, records = read_table(text, delimiter, header_row)
    if not header:
        raise LoadFailure(HEADER_NO_COLUMNS) from ValueError("the header record has no cells")
    # `SEC-17`: before the first row is read, so a file that cannot load costs
    # nothing beyond its header.
    if len(header) > MAX_COLUMNS:
        raise LoadFailure(TOO_MANY_COLUMNS) from ValueError(
            f"the header record has {len(header)} cells, more than {MAX_COLUMNS}"
        )
    count = 0
    for _ in records:
        count += 1
        if count > MAX_ROWS:
            raise LoadFailure(TOO_MANY_ROWS)
    if count == 0:
        raise LoadFailure(NO_ROWS)
    return ParsedFile(
        text=text, delimiter=delimiter, header_row=header_row, columns=_column_names(header), row_count=count
    )


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


def read_stored_object(
    config: SinkConfig, key: str, *, filesystem_factory: Callable[..., Any] | None = None
) -> bytes:
    """The object's bytes, read from the warehouse bucket with the sink's own
    credentials. `key` has passed `_checked_key`. The API caps a file at
    50 MiB, so reading it whole is bounded; this does not cap it again."""
    make_fs = filesystem_factory or s3fs.S3FileSystem
    fs = make_fs(
        key=config.rustfs_access_key,
        secret=config.rustfs_secret_key,
        client_kwargs={"endpoint_url": config.rustfs_endpoint},
    )
    return fs.cat_file(f"{config.warehouse_bucket}/{key}")


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


def _text_resource(parsed: ParsedFile, table: str) -> DltResource:
    """The rows as a dlt resource with every column declared `text`, for
    `load_via_sink` (which takes an already configured resource).

    Plain rows would let dlt type what it recognises. Measured with dlt 1.30.0
    on 2026-10-02 through `load_via_sink` against a local filesystem bucket
    and dlt's own in-memory SQLite catalog: a column holding `2025-02-07T10:00:00Z` came
    back `timestamptz`, and when other values of that column were not
    timestamps dlt put them in a new `<column>__v_text` column and left NULLs
    behind. With `data_type: text` declared, every value came back exactly as
    written. `ops/g9/upload_test.py` checks the types on a deployed stack.
    """
    resource = dlt.resource(parsed.rows(), name=table)
    resource.apply_hints(columns=[{"name": name, "data_type": "text"} for name in parsed.columns])
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


def _read_and_parse(
    params: FileIngestParams,
    sink_config: SinkConfig,
    key: str,
    read_object: Callable[[SinkConfig, str], bytes],
) -> ParsedFile:
    try:
        raw = read_object(sink_config, key)
        text = _decode(raw, params.encoding)
        del raw  # the second pass needs the text, not the bytes
        return parse_file(text, params.delimiter, params.header_row)
    except LoadFailure:
        raise
    except Exception as exc:  # noqa: BLE001 -- every other failure here is an unreadable file; the detail is the cause
        raise LoadFailure(UNREADABLE) from exc


def _write(
    parsed: ParsedFile,
    params: FileIngestParams,
    sink_config: SinkConfig,
    load: Callable[..., SinkResult],
) -> SinkResult:
    table = params.bronze_table_name
    try:
        result = load(_text_resource(parsed, table), table, sink_config, LoadPlan(mode=params.load_mode))
    except Exception as exc:  # noqa: BLE001 -- whatever dlt or the catalog raised is the cause, never the reason
        raise LoadFailure(LOAD_FAILED) from exc
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
    read_object: Callable[[SinkConfig, str], bytes],
    load: Callable[..., SinkResult],
    register: Callable[..., int],
) -> LoadSummary:
    _check_settings(params)
    key = _checked_key(params.storage_key)
    try:
        sink_config = load_config()
    except Exception as exc:  # noqa: BLE001 -- e.g. an unreadable token file; nothing was read or written
        raise LoadFailure(LOAD_FAILED) from exc
    with _unbounded_csv_fields():
        parsed = _read_and_parse(params, sink_config, key, read_object)
        log.info(f"upload {params.upload_id}: {parsed.row_count} rows x {len(parsed.columns)} columns to load")
        result = _write(parsed, params, sink_config, load)
    if result.rows is not None and result.rows != parsed.row_count:
        log.warning(
            f"upload {params.upload_id}: the file held {parsed.row_count} rows and the sink measured {result.rows}"
        )
    total = _register(params, result, register)
    return LoadSummary(
        table=params.bronze_table_name,
        rows=result.rows,
        parsed_rows=parsed.row_count,
        columns=len(parsed.columns),
        table_total=total,
    )


def run_file_load(
    params: FileIngestParams,
    *,
    log: Any,
    load_config: Callable[[], SinkConfig] = _sink_config_from_env,
    read_object: Callable[[SinkConfig, str], bytes] = read_stored_object,
    load: Callable[..., SinkResult] = load_via_sink,
    register: Callable[..., int] = register_loaded_table,
    record: Callable[..., None] = record_ingest_run,
    now: Callable[[], str] = _now,
) -> LoadSummary:
    """Load one uploaded file and record exactly one `ingest_run` row for it.

    Everything that touches a network is a parameter, so a test drives this
    with fakes. The row is written once, after the outcome is known, and is
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
        summary = _load(params, log, load_config, read_object, load, register)
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
    """Read the uploaded object, check and count it, write the Bronze table,
    register it, and record the outcome. See the module docstring."""
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
