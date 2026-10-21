"""The byte-level readers of the upload job for compressed and self-describing files
(`DATA-17` slice 4, plan T4.2; decisions 2, 11, 12 and 13 of
`docs/core/features/upload-big-files-and-more-formats.md`).

`file_ingest.py` reads a stored object in one of five formats. This module owns what is
new in slice 4 and has nothing to do with the sink or the failure reasons:

- **Entries.** `open_entries` yields the byte streams a stored object holds: the object
  itself, the decoded stream of a gzip file, or each file of a zip archive in the
  archive's order. A pass over the entries can be made twice (a JSON file's columns are
  the keys of ALL its objects, so the load reads it once to find them and once to load it).
- **The uncompressed byte counter.** Every byte a gzip or zip entry decodes is spent from
  one `Budget`, and a load that passes it raises `Unpacked`: the file is refused, never
  truncated (decision 2: the uncompressed size counts toward the cap, so a small archive
  cannot unpack into more than was measured). The API runs the same count when the file
  arrives (`upload_formats.rs`, `count_unpacked`) and its preview never reads past it.
- **Objects.** `json_objects` streams the elements of a JSON array (`ijson`) or the lines of
  a JSON Lines file; `avro_records` the records of an Avro container (`fastavro`).
- **Text.** `json_text` and `avro_text` are the one place a value becomes the text the table
  holds, as `parquet_ingest` is for Parquet: a string as it is, a boolean `true` or `false`,
  a number as the source wrote it (`ijson` and `json.loads` hand non-integers over as
  `Decimal` for exactly this reason: `1.50` stays `1.50`), a missing key or `null` as
  nothing (a null cell), and a nested object or array as compact JSON text with its keys in
  the source's order (decision 11: no child tables).

The API's preview is the other reader of these rules (`upload_formats.rs`); it shows a
number in `serde_json`'s shortest form (`1.5` for `1.50`), which is the only difference,
and the load is the authority.
"""

from __future__ import annotations

import gzip
import io
import json
import zipfile
from collections.abc import Iterator
from contextlib import contextmanager
from datetime import date, datetime, time
from decimal import Decimal
from typing import Any, BinaryIO
from uuid import UUID

import fastavro
import ijson

# Decision 13: a zip holds up to 10 files.
MAX_ZIP_FILES = 10

# Entries named like another archive are refused without being opened (decision 13: "an
# archive inside an archive is refused"); the magic is checked too, so a renamed one is
# found as well.
ARCHIVE_SUFFIXES = (".zip", ".gz", ".tgz", ".tar", ".bz2", ".xz", ".7z", ".rar", ".zst")

# A file name that decides the format of a zip's entry (decision 13).
FORMAT_OF_EXTENSION = {
    "csv": "delimited",
    "tsv": "delimited",
    "txt": "delimited",
    "json": "json",
    "jsonl": "jsonl",
    "ndjson": "jsonl",
    "avro": "avro",
}

# The byte order mark a JSON file may start with; it is dropped, as the delimited reader drops it.
_UTF8_BOM = b"\xef\xbb\xbf"


class Unpacked(Exception):
    """The decoded bytes of a gzip or zip file passed the budget: refuse, never truncate."""


class Refused(Exception):
    """The content is not something this load reads (an archive inside an archive, a JSON
    file that is not an array of objects, a binary Avro column ...). The message is for the
    run log only."""


class Budget:
    """The uncompressed bytes a load may decode, spent by every entry of every pass."""

    def __init__(self, limit: int) -> None:
        self.limit = limit
        self.used = 0

    def spend(self, count: int) -> None:
        """Count `count` decoded bytes.

        # Errors

        Raises `Unpacked` when the total passes the limit."""
        self.used += count
        if self.used > self.limit:
            raise Unpacked(f"the file holds more than {self.limit} bytes once unpacked")


class _Counted(io.RawIOBase):
    """A decoded stream whose every byte is spent from a `Budget`."""

    def __init__(self, inner: BinaryIO, budget: Budget) -> None:
        super().__init__()
        self._inner = inner
        self._budget = budget

    def readable(self) -> bool:
        return True

    def readinto(self, buffer: Any) -> int:
        data = self._inner.read(len(buffer))
        self._budget.spend(len(data))
        buffer[: len(data)] = data
        return len(data)

    def close(self) -> None:
        self._inner.close()
        super().close()


class _Borrowed(io.RawIOBase):
    """The stored object lent to a reader: closing the reader must not close the object,
    which the next pass over the entries rewinds."""

    def __init__(self, inner: BinaryIO) -> None:
        super().__init__()
        self._inner = inner

    def readable(self) -> bool:
        return True

    def readinto(self, buffer: Any) -> int:
        data = self._inner.read(len(buffer))
        buffer[: len(data)] = data
        return len(data)

    def close(self) -> None:  # the owner closes it
        super().close()


def _buffered(stream: BinaryIO, budget: Budget | None) -> BinaryIO:
    """The stream, counted when there is a budget, with `peek` and line reading."""
    raw = _Counted(stream, budget) if budget is not None else _Borrowed(stream)
    return io.BufferedReader(raw, buffer_size=1024 * 1024)  # type: ignore[return-value]


def _is_archive(name: str, head: bytes) -> bool:
    return name.lower().endswith(ARCHIVE_SUFFIXES) or head.startswith((b"PK\x03\x04", b"\x1f\x8b"))


def zip_file_infos(archive: zipfile.ZipFile) -> list[zipfile.ZipInfo]:
    """The files of an archive that a load reads: not the folders, and not macOS's
    `__MACOSX/` resource forks. At most `MAX_ZIP_FILES`, none encrypted.

    # Errors

    Raises `Refused` for more files than that, an encrypted one, or none."""
    infos = [
        info
        for info in archive.infolist()
        if not info.is_dir() and not info.filename.startswith("__MACOSX/")
    ]
    if not infos:
        raise Refused("the archive holds no files")
    if len(infos) > MAX_ZIP_FILES:
        raise Refused(f"the archive holds {len(infos)} files, more than {MAX_ZIP_FILES}")
    if any(info.flag_bits & 0x1 for info in infos):
        raise Refused("the archive has an encrypted file")
    return infos


def format_of_entry(name: str) -> str | None:
    """The format a zip entry's name says it is, or `None` for a name this load does not read."""
    extension = name.rsplit(".", 1)[-1].lower() if "." in name.rsplit("/", 1)[-1] else ""
    return FORMAT_OF_EXTENSION.get(extension)


@contextmanager
def open_entries(
    handle: BinaryIO, compression: str, budget: Budget | None, file_format: str
) -> Iterator[Iterator[tuple[str, BinaryIO]]]:
    """The byte streams of a stored object, from its start: `("", handle)` for a plain one,
    `("", decoded)` for gzip and `(name, decoded)` for each file of a zip. A decoded stream
    counts its bytes against `budget`. One pass; call again for another (the handle is
    rewound).

    # Errors

    Raises `Refused` for a zip with no files, over the file limit, with an encrypted file,
    with an entry that is itself an archive, or with an entry that is not of `file_format`;
    `Unpacked` as the budget runs out (while the entries are read, not here).
    """
    handle.seek(0)
    if compression == "none":
        yield iter([("", _buffered(handle, None))])
        return
    if compression == "gzip":
        decoded = gzip.GzipFile(fileobj=handle, mode="rb")
        stream = _buffered(decoded, budget)
        if stream.peek(4)[:4].startswith((b"PK\x03\x04", b"\x1f\x8b")):  # type: ignore[attr-defined]
            raise Refused("the gzip file holds an archive")
        yield iter([("", stream)])
        return
    archive = zipfile.ZipFile(handle)
    infos = zip_file_infos(archive)
    declared = sum(info.file_size for info in infos)
    if budget is not None and declared > budget.limit:
        raise Unpacked(f"the archive declares {declared} bytes, more than {budget.limit}")
    for info in infos:
        if info.filename.lower().endswith(ARCHIVE_SUFFIXES):
            raise Refused("the archive holds another archive")
        if format_of_entry(info.filename) != file_format:
            raise Refused("the archive holds files that are not all of one supported format")

    def entries() -> Iterator[tuple[str, BinaryIO]]:
        for info in infos:
            with archive.open(info) as entry:
                stream = _buffered(entry, budget)  # type: ignore[arg-type]
                if _is_archive(info.filename, stream.peek(4)[:4]):
                    raise Refused("the archive holds another archive")
                yield info.filename, stream

    try:
        yield entries()
    finally:
        archive.close()


# ── JSON ─────────────────────────────────────────────────────────────────


def _number_text(value: Any) -> str:
    return str(value)


def json_text(value: Any) -> str | None:
    """The text a value of a JSON object is in the table: `None` for a missing key or `null`,
    a string as it is, `true` or `false`, a number as the source wrote it, and a nested
    object or array as compact JSON in the source's key order."""
    if value is None:
        return None
    if isinstance(value, str):
        return value
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, Decimal)):
        return _number_text(value)
    if isinstance(value, float):
        return repr(value)
    return nested_text(value)


def nested_text(value: Any) -> str:
    """Compact JSON of a nested value, numbers as written (`Decimal`) and keys in order."""
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, Decimal)):
        return _number_text(value)
    if isinstance(value, float):
        return repr(value)
    if isinstance(value, (datetime, date, time)):
        return json.dumps(value.isoformat())
    if isinstance(value, UUID):
        return json.dumps(str(value))
    if isinstance(value, dict):
        return "{" + ",".join(f"{json.dumps(str(key), ensure_ascii=False)}:{nested_text(item)}" for key, item in value.items()) + "}"
    if isinstance(value, (list, tuple)):
        return "[" + ",".join(nested_text(item) for item in value) + "]"
    if isinstance(value, (bytes, bytearray)):
        raise Refused("a binary value has no text form")
    return json.dumps(str(value), ensure_ascii=False)


def _skip_bom(stream: BinaryIO) -> None:
    if stream.peek(3)[:3] == _UTF8_BOM:  # type: ignore[attr-defined]
        stream.read(3)


def json_objects(stream: BinaryIO, lines: bool) -> Iterator[dict[str, Any]]:
    """The objects of one stream: the elements of a top-level array (`lines=False`), or one
    object per line, blank lines skipped (`lines=True`).

    # Errors

    Raises `Refused` for text that is not JSON, for a top-level value that is not an array
    (or, for lines, a line that is not an object) and for an element that is not an object.
    """
    if not hasattr(stream, "peek"):
        stream = io.BufferedReader(stream, buffer_size=1024 * 1024)  # type: ignore[arg-type,assignment]
    _skip_bom(stream)
    try:
        if lines:
            for line in stream:
                if not line.strip():
                    continue
                item = json.loads(line, parse_float=Decimal)
                if not isinstance(item, dict):
                    raise Refused("a line is not a JSON object")
                yield item
            return
        head = stream.peek(1024)  # type: ignore[attr-defined]
        if head.lstrip()[:1] != b"[":
            raise Refused("the JSON file is not a top-level array")
        for item in ijson.items(stream, "item"):
            if not isinstance(item, dict):
                raise Refused("an element of the array is not a JSON object")
            yield item
    except (ValueError, ijson.JSONError) as exc:
        if isinstance(exc, Refused):
            raise
        raise Refused("the file is not valid JSON") from exc


# ── Avro ─────────────────────────────────────────────────────────────────


def _has_binary(schema: Any) -> bool:
    """Whether a schema holds a `bytes` or `fixed` value that is not a decimal: there is no
    text form of it, and the load refuses the file as it refuses a binary Parquet column."""
    if isinstance(schema, str):
        return schema == "bytes"
    if isinstance(schema, list):
        return any(_has_binary(branch) for branch in schema)
    if isinstance(schema, dict):
        if schema.get("logicalType") == "decimal":
            return False
        kind = schema.get("type")
        if kind in ("bytes", "fixed"):
            return True
        if kind == "record":
            return any(_has_binary(field["type"]) for field in schema.get("fields", []))
        if kind == "array":
            return _has_binary(schema.get("items"))
        if kind == "map":
            return _has_binary(schema.get("values"))
        return _has_binary(kind) if isinstance(kind, (dict, list)) else False
    return False


def avro_text(value: Any) -> str | None:
    """The text a value of an Avro record is in the table (decision 12)."""
    if value is None:
        return None
    if isinstance(value, str):
        return value
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, Decimal)):
        return str(value)
    if isinstance(value, float):
        return repr(value)
    if isinstance(value, (datetime, date, time)):
        return value.isoformat()
    if isinstance(value, UUID):
        return str(value)
    return nested_text(value)


def avro_reader(stream: BinaryIO) -> Any:
    """A `fastavro` reader over `stream`, for an Avro object container file whose top-level
    schema is a record.

    # Errors

    Raises `Refused` for a file that is not an Avro container, whose schema is not a record,
    or that holds a binary column."""
    try:
        reader = fastavro.reader(stream)
        schema = reader.writer_schema
    except Exception as exc:  # noqa: BLE001 -- fastavro raises several types for a damaged header
        raise Refused("the file is not an Avro container") from exc
    if not isinstance(schema, dict) or schema.get("type") != "record":
        raise Refused("the Avro schema is not a record")
    if _has_binary(schema):
        raise Refused("the Avro schema has a binary column")
    return reader
