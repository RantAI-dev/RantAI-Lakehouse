"""dagster/dispar_orchestrate/discover.py -- the orchestrator's side of table
discovery (`SRC-13` decisions D1, D4, D5, D6, D9, D10): list what a source
holds and preview the columns of one object, for the connector types whose
drivers only this code location has (MongoDB, Kafka, SFTP, Oracle; and the
column preview of a REST endpoint).

# Who calls, who answers

`lakehouse-api` launches `discover_job` (`routes/connector_discovery.rs`) with
`{connector_id, request_id, kind, object, schema, glob, search, limit}` and
answers `running`; this job reads the source and POSTS the result to
`POST /api/connectors/{id}/discovery-results` on the same `ingest:read` service
identity the run reports use (`schema_observer.ObserverConfig`/`_post_json`:
the same 10-second `timeout=` and `raise_for_status()`). The API stores it
(latest request wins) and the console polls. The job never loads anything and
never writes to the source.

# The wire (what the API accepts; closed lists, no free text)

    list, done     {"requestId", "kind": "list", "state": "done",
                    "objects": [{"name", "kind", "size"?, "modifiedAt"?, "load"?}],
                    "truncated": bool}
    preview, done  {"requestId", "kind": "preview", "state": "done",
                    "columns": [{"name", "typeName", "inferred"}],
                    "primaryKey"?: [name]}
    failed         {"requestId", "kind", "state": "failed", "errorClass"}

`errorClass` is one of `ERROR_CLASSES` (decision D10's six words, `unreachable`
`sign_in_refused` `no_permission` `timeout` `too_many_objects` `not_supported`,
plus `address_refused` and `failed`). It is chosen by `classify_discovery` from
the exception's TYPE and CODE through `ingest_errors` (`SRC-11`'s classifier):
nothing of an exception's message is read, logged, posted or stored (AGENTS.md
principle 4). The job logs the class and the exception's type name, never its
text.

# Reusing the adapters' own connection code

Each read opens its connection through the adapter that loads the same type
(`mongodb.connect`, `sftp.resolve_host`/`open_session`, `oracle.connection_parts`,
`kafka.check_brokers`, `rest.first_page_records`), so a listing dials exactly as a
load does: the same hostname check, the same `ssrf_guard.resolve_checked`, the
same pinning or `checking_resolver`, the same credential resolution through the
allowlisted `secret_resolver`. A change to how a type connects changes both.

# Bounds (decision D9, D6)

At most `MAX_NAMES` (10,000) names, `MAX_SAMPLE_DOCUMENTS` / `MAX_SAMPLE_MESSAGES`
(100) samples, `MAX_SAMPLE_ROWS` (1,000) CSV rows, `MAX_COLUMNS` (2,000) columns,
and `MAX_RESULT_BYTES` of JSON. A list cut at a bound says `truncated`; a name the
API would refuse (blank, over 256 characters, a control character) is left out
rather than failing the whole answer. An SFTP directory of more than
`MAX_SCANNED` entries is `too_many_objects`.

# Kafka never commits (decision D6)

The preview assigns the topic's partitions by hand, seeks to the last messages
and reads. `enable_auto_commit` is False, nothing calls `commit`, and the
consumer's group id is `lakehouse-discover-<request id>`, which no connector's
dial uses (`groupId` is the operator's, and a connector's consumers use it).
Partitions are assigned, not subscribed, so the consumer never joins a group.
`test_discover.py` pins all three.

# Inferred types (decision D6)

A type read from a sample (a CSV, a MongoDB document, a Kafka message, a REST
record, an SFTP file) is `inferred: true`; a type a database declares (Oracle) is
`inferred: false`. The vocabulary is `integer`, `number`, `boolean`, `date`,
`timestamp`, `decimal`, `binary`, `string`, `nested` (a list or an object, which a
load refuses, `column_gate.py`), `mixed` (values of different types) and `unknown`
(only nulls).
"""

from __future__ import annotations

import csv
import io
import json
import logging
import re
import stat
import time
from collections.abc import Callable, Iterable
from contextlib import AbstractContextManager
from dataclasses import dataclass
from datetime import date, datetime, timezone
from decimal import Decimal
from typing import Any

import requests
from dagster import Failure, Field, job, op
from kafka import KafkaConsumer, TopicPartition

from dispar_orchestrate import ingest_errors, schema_observer, secret_resolver, ssrf_guard
from dispar_orchestrate.adapters import kafka as kafka_adapter
from dispar_orchestrate.adapters import mongodb as mongodb_adapter
from dispar_orchestrate.adapters import oracle as oracle_adapter
from dispar_orchestrate.adapters import rest as rest_adapter
from dispar_orchestrate.adapters import saas as saas_adapter
from dispar_orchestrate.adapters import sftp as sftp_adapter
from dispar_orchestrate.ingest_factory import IngestFactoryConfig, _fetch_one_connector, kafka_security_kwargs
from dispar_orchestrate.op_metadata import DEFAULT_RETRY_POLICY, source_metadata
from dispar_orchestrate.saas import session as saas_session
from dispar_orchestrate.saas import vendors as saas_vendors

logger = logging.getLogger(__name__)

# --- the contract's bounds (mirrors `lakehouse_store::connector_discovery`) ----

MAX_NAMES = 10_000
MAX_COLUMNS = 2_000
MAX_NAME_CHARS = 256
MAX_TYPE_CHARS = 128
# The API stores at most 3 MiB (`MAX_RESULT_BYTES` there); the margin covers the
# envelope the API does not count but we do not measure exactly.
MAX_RESULT_BYTES = 2_500_000
MAX_SAMPLE_DOCUMENTS = 100
MAX_SAMPLE_MESSAGES = 100
MAX_SAMPLE_ROWS = 1_000
MAX_SCANNED = 100_000
# A CSV preview reads this much of a file's head (`connector_listing.rs` reads
# the same for object storage).
CSV_HEAD_BYTES = 1024 * 1024
# How long a Kafka preview waits for the last messages.
KAFKA_PREVIEW_SECONDS = 10

# --- the closed list of failure classes (decision D10) -------------------------

UNREACHABLE = "unreachable"
SIGN_IN_REFUSED = "sign_in_refused"
ADDRESS_REFUSED = "address_refused"
NO_PERMISSION = "no_permission"
TIMEOUT = "timeout"
TOO_MANY_OBJECTS = "too_many_objects"
NOT_SUPPORTED = "not_supported"
FAILED = "failed"
ERROR_CLASSES = frozenset(
    {UNREACHABLE, SIGN_IN_REFUSED, ADDRESS_REFUSED, NO_PERMISSION, TIMEOUT, TOO_MANY_OBJECTS, NOT_SUPPORTED, FAILED}
)

KINDS = ("list", "preview")


class NotSupported(Exception):
    """The connector's type cannot be listed or previewed this way."""


class TooManyObjects(Exception):
    """The source holds more than can be scanned."""


# `ingest_errors` (SRC-11) names a failure with its own ten words; discovery
# reports D10's. A word with no discovery counterpart (`source_busy`,
# `api_unreachable`, `setting_rejected`, `schema_decision_pending`, `other`) is
# `failed`: said as an unnamed failure, not guessed into another class.
_FROM_INGEST = {
    ingest_errors.UNREACHABLE: UNREACHABLE,
    ingest_errors.TIMEOUT: TIMEOUT,
    ingest_errors.ADDRESS_REFUSED: ADDRESS_REFUSED,
    ingest_errors.UNSUPPORTED: NOT_SUPPORTED,
}


def _is_unresolvable(exc: ssrf_guard.SsrfBlocked) -> bool:
    message = str(exc)
    return message.startswith("could not resolve host") or "did not resolve to any address" in message


def classify_discovery(exc: BaseException) -> str:
    """The `ERROR_CLASSES` word for `exc`. By type and code only (see the module
    docstring); a credential refusal is checked before a permission refusal,
    because it is the more specific statement about the credential."""
    if isinstance(exc, NotSupported):
        return NOT_SUPPORTED
    if isinstance(exc, TooManyObjects):
        return TOO_MANY_OBJECTS
    if isinstance(exc, ssrf_guard.SsrfBlocked) and _is_unresolvable(exc):
        # `resolve_checked` raises `SsrfBlocked` for a name that did not resolve as
        # well as for a refused address; only the second is the address check's
        # doing. The product's own sentence is read for its start, never emitted.
        return UNREACHABLE
    if ingest_errors.is_authentication_refusal(exc) or isinstance(exc, saas_session.SaasSignInRefused):
        return SIGN_IN_REFUSED
    if ingest_errors.is_permission_refusal(exc):
        return NO_PERMISSION
    return _FROM_INGEST.get(ingest_errors.classify(exc).error_class, FAILED)


# --- the request ---------------------------------------------------------------


@dataclass(frozen=True)
class DiscoverRequest:
    """What the API launched the job for."""

    connector_id: str
    request_id: str
    kind: str
    object: str = ""
    schema: str = ""
    glob: str = ""
    search: str = ""
    limit: int = MAX_NAMES

    @classmethod
    def from_op_config(cls, config: dict[str, Any]) -> "DiscoverRequest":
        """The request out of the op's run config (the API's `launch`).

        # Errors

        `ValueError` for a kind that is not `list`/`preview`.
        """
        kind = config["kind"]
        if kind not in KINDS:
            raise ValueError("kind must be list or preview")
        return cls(
            connector_id=config["connector_id"],
            request_id=config["request_id"],
            kind=kind,
            object=config.get("object", ""),
            schema=config.get("schema", ""),
            glob=config.get("glob", ""),
            search=config.get("search", ""),
            limit=max(1, min(int(config.get("limit", MAX_NAMES)), MAX_NAMES)),
        )


# --- the answer ----------------------------------------------------------------


def valid_name(name: object) -> bool:
    """Whether the API would accept `name` (non-blank, bounded, no control char)."""
    return (
        isinstance(name, str)
        and bool(name.strip())
        and len(name) <= MAX_NAME_CHARS
        and not any(ord(c) < 0x20 or ord(c) == 0x7F for c in name)
    )


def listed(
    name: str,
    kind: str,
    *,
    size: int | None = None,
    modified_at: str | None = None,
    load: str | None = None,
    label: str | None = None,
) -> dict[str, Any]:
    """One entry of a list answer; `size`, `modifiedAt`, `load` (SRC-3: how a
    business-app object loads) and `label` (a name for a person where `name` is an id, slice 5 review
    BLOCKER 14) only when known."""
    entry: dict[str, Any] = {"name": name, "kind": kind}
    if load is not None:
        entry["load"] = load
    if label is not None:
        entry["label"] = label
    if size is not None:
        entry["size"] = size
    if modified_at is not None:
        entry["modifiedAt"] = modified_at
    return entry


def column(name: str, type_name: str, *, inferred: bool) -> dict[str, Any]:
    return {"name": name, "typeName": type_name, "inferred": inferred}


def _json_size(value: Any) -> int:
    return len(json.dumps(value, separators=(",", ":"), ensure_ascii=False).encode("utf-8"))


def listing(entries: Iterable[dict[str, Any]], limit: int, *, more: bool = False) -> dict[str, Any]:
    """The list result: valid names only, in the order given, at most `limit`, at
    most `MAX_RESULT_BYTES` of JSON. `more` says the source held entries that were
    not offered (the caller already cut); the result is `truncated` then, and when
    this function cuts."""
    kept: list[dict[str, Any]] = []
    size = 0
    truncated = more
    for entry in entries:
        if not valid_name(entry["name"]):
            continue
        entry_size = _json_size(entry) + 1
        if len(kept) >= min(limit, MAX_NAMES) or size + entry_size > MAX_RESULT_BYTES:
            truncated = True
            break
        kept.append(entry)
        size += entry_size
    return {"objects": kept, "truncated": truncated}


def preview(columns: list[dict[str, Any]], primary_key: list[str] | None = None) -> dict[str, Any]:
    """The preview result: valid column names only, at most `MAX_COLUMNS`. No
    column means the source had nothing to infer them from (an empty topic,
    collection or file): said as no columns, never invented."""
    kept = [c for c in columns if valid_name(c["name"]) and len(c["typeName"]) <= MAX_TYPE_CHARS][:MAX_COLUMNS]
    result: dict[str, Any] = {"columns": kept}
    names = {c["name"] for c in kept}
    key = [name for name in (primary_key or []) if name in names]
    if key:
        result["primaryKey"] = key
    return result


def contains_ignore_case(haystack: str, needle: str) -> bool:
    return not needle or needle.lower() in haystack.lower()


# --- a file glob (the same rules as `connector_listing.rs`) ----------------------


class Glob:
    """`*` is any run of characters except `/`, `**` any run including `/`, `?` one
    character except `/`; everything else is itself. Matched by a position set
    advanced one character at a time: linear in the text times the pattern, so a
    hostile pattern cannot take exponential time. Empty means everything."""

    MAX_CHARS = 256

    def __init__(self, glob: str) -> None:
        glob = glob or "**"
        if len(glob) > self.MAX_CHARS or any(ord(c) < 0x20 or ord(c) == 0x7F for c in glob):
            raise ValueError("the glob is too long or holds a control character")
        tokens: list[tuple[str, str]] = []
        index = 0
        while index < len(glob):
            char = glob[index]
            if char == "*" and glob[index + 1 : index + 2] == "*":
                tokens.append(("deep", ""))
                index += 2
                continue
            tokens.append({"*": ("star", ""), "?": ("one", "")}.get(char, ("lit", char)))
            index += 1
        self._tokens = tokens

    def _close(self, states: list[bool]) -> None:
        for at, (kind, _) in enumerate(self._tokens):
            if states[at] and kind in ("star", "deep"):
                states[at + 1] = True

    def matches(self, text: str) -> bool:
        count = len(self._tokens)
        current = [False] * (count + 1)
        current[0] = True
        self._close(current)
        for char in text:
            nxt = [False] * (count + 1)
            for at, (kind, lit) in enumerate(self._tokens):
                if not current[at]:
                    continue
                if (kind == "lit" and lit == char) or (kind == "one" and char != "/"):
                    nxt[at + 1] = True
                elif (kind == "star" and char != "/") or kind == "deep":
                    nxt[at] = True
            self._close(nxt)
            current = nxt
        return current[count]


# --- inferring a type from a sample (decision D6) --------------------------------

_DATE = re.compile(r"\d{4}-\d{2}-\d{2}")
_STAMP = re.compile(r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:?\d{2})?")


def infer_text_type(values: Iterable[str]) -> str:
    """The narrowest type every non-blank text value fits (a CSV cell)."""
    cells = [v.strip() for v in values if v is not None and v.strip()]
    if not cells:
        return "string"

    def fits(test: Callable[[str], bool]) -> bool:
        return all(test(cell) for cell in cells)

    def is_int(cell: str) -> bool:
        try:
            int(cell)
        except ValueError:
            return False
        return True

    def is_number(cell: str) -> bool:
        try:
            number = float(cell)
        except ValueError:
            return False
        return number == number and number not in (float("inf"), float("-inf"))

    if fits(is_int):
        return "integer"
    if fits(is_number):
        return "number"
    if fits(lambda cell: cell.lower() in ("true", "false")):
        return "boolean"
    if fits(lambda cell: bool(_DATE.fullmatch(cell))):
        return "date"
    if fits(lambda cell: bool(_STAMP.fullmatch(cell))):
        return "timestamp"
    return "string"


_BSON_NAMES = {"ObjectId": "string", "Decimal128": "decimal", "Binary": "binary", "Int64": "integer"}


def value_type(value: Any) -> str | None:
    """The type of one decoded value (a document field, a JSON message, a REST
    record), or `None` for a null (which says nothing about the type)."""
    if value is None:
        return None
    if isinstance(value, bool):
        return "boolean"
    if isinstance(value, int):
        return "integer"
    if isinstance(value, float):
        return "number"
    if isinstance(value, Decimal):
        return "decimal"
    if isinstance(value, str):
        return "string"
    if isinstance(value, datetime):
        return "timestamp"
    if isinstance(value, date):
        return "date"
    if isinstance(value, (bytes, bytearray)):
        return "binary"
    if isinstance(value, (list, tuple, dict)):
        return "nested"
    return _BSON_NAMES.get(type(value).__name__, "string")


def columns_from_records(records: Iterable[Any]) -> list[dict[str, Any]]:
    """The columns of a sample of records: the union of their top-level keys in
    first-seen order, each typed from its values (`integer` with `number` is
    `number`; any other disagreement is `mixed`; only nulls is `unknown`). All
    inferred. A record that is not an object adds nothing."""
    seen: dict[str, set[str]] = {}
    for record in records:
        if not isinstance(record, dict):
            continue
        for key, value in record.items():
            types = seen.setdefault(str(key), set())
            found = value_type(value)
            if found is not None:
                types.add(found)
    columns = []
    for name, types in seen.items():
        if not types:
            type_name = "unknown"
        elif len(types) == 1:
            type_name = next(iter(types))
        elif types == {"integer", "number"}:
            type_name = "number"
        else:
            type_name = "mixed"
        columns.append(column(name, type_name, inferred=True))
    return columns


def csv_columns(text: str, *, head_is_partial: bool) -> list[dict[str, Any]]:
    """The columns of a CSV file's head: the header, each typed from up to
    `MAX_SAMPLE_ROWS` rows. A head cut from a longer file loses its last, partial
    line first. A blank header cell is `column_N`."""
    rows = list(csv.reader(io.StringIO(text)))
    if head_is_partial and rows:
        rows.pop()
    if not rows:
        return []
    header, body = rows[0], rows[1 : 1 + MAX_SAMPLE_ROWS]
    columns = []
    for index, cell in enumerate(header):
        name = "".join(c for c in cell if ord(c) >= 0x20 and ord(c) != 0x7F)[:MAX_NAME_CHARS]
        name = name if name.strip() else f"column_{index + 1}"
        values = [row[index] for row in body if index < len(row)]
        columns.append(column(name, infer_text_type(values), inferred=True))
    return columns


# --- the injected drivers --------------------------------------------------------


def _real_kafka_consumer(spec: dict, secrets: dict[str, str], *, group_id: str) -> Any:
    """A consumer for a listing or a preview: no auto commit, a group id the
    connector never uses, and request timeouts a hung broker cannot stretch."""
    return KafkaConsumer(
        bootstrap_servers=spec["bootstrapServers"],
        group_id=group_id,
        enable_auto_commit=False,
        value_deserializer=lambda v: v,
        request_timeout_ms=30_000,
        # SRC-13 review 3 BLOCKER 1: no `api_version_auto_timeout_ms` here. kafka-python 3.0.11 does not
        # know that key and raises KafkaConfigurationError ("Unrecognized configs") at construction, which
        # `classify_discovery` could only call `failed`. Every key above must be in
        # `KafkaConsumer.DEFAULT_CONFIG` (test_discover.py pins that against the installed library).
        **kafka_security_kwargs(spec, secrets),
    )


def _real_sftp_session(spec: dict, secrets: dict[str, str], *, resolve_checked: Callable) -> tuple[Any, Any]:
    resolved = sftp_adapter.resolve_host(spec, resolve_checked=resolve_checked)
    return sftp_adapter.open_session(spec, secrets, resolved)


def _real_oracle_engine(spec: dict, secrets: dict[str, str], *, resolve_checked: Callable) -> Any:
    from sqlalchemy import create_engine
    from sqlalchemy.engine import URL

    credentials, engine_kwargs, _resolved = oracle_adapter.connection_parts(
        spec, secrets, resolve_checked=resolve_checked
    )
    connect_args = dict(engine_kwargs.get("connect_args", {}))
    # A listing must not wait the driver's default minute for a dead host.
    connect_args.setdefault("tcp_connect_timeout", 10)
    return create_engine(URL.create(**credentials), **{**engine_kwargs, "connect_args": connect_args})


@dataclass(frozen=True)
class Backends:
    """The things that touch the network, each replaceable by a test."""

    resolve_checked: Callable = ssrf_guard.resolve_checked
    checking_resolver: Callable[[], AbstractContextManager] = ssrf_guard.checking_resolver
    mongo_connect: Callable = mongodb_adapter.connect
    kafka_consumer: Callable = _real_kafka_consumer
    sftp_session: Callable = _real_sftp_session
    oracle_engine: Callable = _real_oracle_engine
    rest_first_page: Callable = rest_adapter.first_page_records
    saas_open: Callable = saas_adapter.open_session
    clock: Callable[[], float] = time.monotonic


# --- MongoDB (decision D5, D6) -----------------------------------------------------


def _mongo(dial: dict, secrets: dict, request: DiscoverRequest, b: Backends) -> dict[str, Any]:
    client, _resolved = b.mongo_connect(dial, secrets, resolve_checked=b.resolve_checked)
    try:
        database = client[dial["database"]]
        with b.checking_resolver():
            if request.kind == "list":
                names = sorted(n for n in database.list_collection_names() if not n.startswith("system."))
                matching = [n for n in names if contains_ignore_case(n, request.search)]
                return listing((listed(n, "collection") for n in matching), request.limit)
            documents = list(database[request.object].find({}).limit(MAX_SAMPLE_DOCUMENTS))
        return preview(columns_from_records(documents))
    finally:
        client.close()


# --- Kafka (decision D4, D6) -----------------------------------------------------------


def _kafka(dial: dict, secrets: dict, request: DiscoverRequest, b: Backends) -> dict[str, Any]:
    # A group id the connector never uses (module docstring, "Kafka never commits").
    consumer = b.kafka_consumer(dial, secrets, group_id=f"lakehouse-discover-{request.request_id}")
    try:
        kafka_adapter.check_brokers(consumer, resolve_checked=b.resolve_checked)
        with b.checking_resolver():
            if request.kind == "list":
                topics = sorted(t for t in consumer.topics() if not t.startswith("__"))
                matching = [t for t in topics if contains_ignore_case(t, request.search)]
                return listing((listed(t, "topic") for t in matching), request.limit)
            return preview(columns_from_records(_last_messages(consumer, request.object, b)))
    finally:
        consumer.close()


def _last_messages(consumer: Any, topic: str, b: Backends) -> list[Any]:
    """The last `MAX_SAMPLE_MESSAGES` JSON messages of `topic`, read without
    committing anything. Messages that are not a JSON value are skipped."""
    partitions = consumer.partitions_for_topic(topic)
    if not partitions:
        raise ValueError("the topic is not one the broker lists")
    assigned = [TopicPartition(topic, p) for p in sorted(partitions)]
    consumer.assign(assigned)
    ends = consumer.end_offsets(assigned)
    begins = consumer.beginning_offsets(assigned)
    for tp in assigned:
        consumer.seek(tp, max(begins[tp], ends[tp] - MAX_SAMPLE_MESSAGES))
    records: list[Any] = []
    deadline = b.clock() + KAFKA_PREVIEW_SECONDS
    while b.clock() < deadline and any(consumer.position(tp) < ends[tp] for tp in assigned):
        for batch in consumer.poll(timeout_ms=1000, max_records=500).values():
            records.extend(batch)
    records.sort(key=lambda r: (r.timestamp or 0, r.partition, r.offset))
    decoded = []
    for record in records[-MAX_SAMPLE_MESSAGES:]:
        try:
            decoded.append(json.loads(record.value))
        except (ValueError, TypeError):
            continue
    return decoded


# --- SFTP (decision D5, D6) --------------------------------------------------------------


def _rfc3339(epoch: float) -> str:
    return datetime.fromtimestamp(epoch, tz=timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _sftp(dial: dict, secrets: dict, request: DiscoverRequest, b: Backends) -> dict[str, Any]:
    client, sftp = b.sftp_session(dial, secrets, resolve_checked=b.resolve_checked)
    try:
        base = dial["path"].rstrip("/")
        if request.kind == "preview":
            with sftp.open(f"{base}/{request.object}") as handle:
                head = handle.read(CSV_HEAD_BYTES + 1)
            partial = len(head) > CSV_HEAD_BYTES
            text = head[:CSV_HEAD_BYTES].decode("utf-8-sig", errors="replace")
            return preview(csv_columns(text, head_is_partial=partial))
        glob = Glob(request.glob)
        entries = sftp.listdir_attr(base or "/")
        if len(entries) > MAX_SCANNED:
            raise TooManyObjects("the directory holds too many entries")
        files = sorted(
            (e for e in entries if stat.S_ISREG(e.st_mode or 0)),
            key=lambda e: e.filename,
        )
        matching = (
            listed(
                e.filename,
                "file",
                size=e.st_size,
                modified_at=_rfc3339(e.st_mtime) if e.st_mtime is not None else None,
            )
            for e in files
            if glob.matches(e.filename) and contains_ignore_case(e.filename, request.search)
        )
        return listing(matching, request.limit)
    finally:
        client.close()


# --- Oracle (decision D5, D7, D8) ------------------------------------------------------------

_ORACLE_SCHEMAS = (
    "SELECT DISTINCT t.owner FROM all_tables t JOIN all_users u ON u.username = t.owner "
    "WHERE u.oracle_maintained = 'N' "
    "AND (:search IS NULL OR UPPER(t.owner) LIKE '%' || UPPER(:search) || '%' ESCAPE '\\') "
    "ORDER BY t.owner FETCH FIRST :max_rows ROWS ONLY"
)
# `all_tables` holds base tables only, not views (decision D8).
_ORACLE_TABLES = (
    "SELECT table_name FROM all_tables WHERE owner = :owner AND nested = 'NO' "
    "AND (:search IS NULL OR UPPER(table_name) LIKE '%' || UPPER(:search) || '%' ESCAPE '\\') "
    "ORDER BY table_name FETCH FIRST :max_rows ROWS ONLY"
)
_ORACLE_COLUMNS = (
    "SELECT column_name, data_type FROM all_tab_columns "
    "WHERE owner = :owner AND table_name = :table_name ORDER BY column_id"
)
_ORACLE_KEY = (
    "SELECT cols.column_name FROM all_constraints cons "
    "JOIN all_cons_columns cols ON cols.owner = cons.owner "
    "AND cols.constraint_name = cons.constraint_name "
    "WHERE cons.constraint_type = 'P' AND cons.owner = :owner AND cons.table_name = :table_name "
    "ORDER BY cols.position"
)


def like_escape(text: str) -> str:
    """`text` as a LIKE operand that matches itself (backslash is the escape)."""
    return "".join("\\" + c if c in "\\%_" else c for c in text)


def _oracle(dial: dict, secrets: dict, request: DiscoverRequest, b: Backends) -> dict[str, Any]:
    from sqlalchemy import text

    engine = b.oracle_engine(dial, secrets, resolve_checked=b.resolve_checked)
    try:
        with b.checking_resolver(), engine.connect() as connection:

            def rows(sql: str, **params: Any) -> list[tuple]:
                return [tuple(r) for r in connection.execute(text(sql), params)]

            search = like_escape(request.search) or None
            fetch = request.limit + 1
            if request.kind == "list" and not request.schema:
                owners = [r[0] for r in rows(_ORACLE_SCHEMAS, search=search, max_rows=fetch)]
                return _cut(owners, request.limit, lambda name: listed(name, "schema"))
            if request.kind == "list":
                names = [r[0] for r in rows(_ORACLE_TABLES, owner=request.schema, search=search, max_rows=fetch)]
                return _cut(names, request.limit, lambda name: listed(f"{request.schema}.{name}", "table"))
            owner, _, table = request.object.partition(".")
            if not owner or not table:
                raise ValueError("an Oracle object is SCHEMA.TABLE")
            columns = rows(_ORACLE_COLUMNS, owner=owner, table_name=table)
            key = [r[0] for r in rows(_ORACLE_KEY, owner=owner, table_name=table)]
            return preview([column(name, type_name, inferred=False) for name, type_name in columns], key)
    finally:
        engine.dispose()


def _cut(names: list[str], limit: int, make: Callable[[str], dict[str, Any]]) -> dict[str, Any]:
    """A list from names fetched one past `limit`: the extra says it was cut."""
    return listing((make(n) for n in names[:limit]), limit, more=len(names) > limit)


# --- REST (decision D6): only the column preview, the list is the API's -------------------------


def _rest(dial: dict, secrets: dict, request: DiscoverRequest, b: Backends) -> dict[str, Any]:
    if request.kind != "preview":
        raise NotSupported("a REST connector's list is its declared endpoints, listed by the API")
    records = b.rest_first_page(
        dial,
        secrets,
        request.object,
        limit=MAX_SAMPLE_DOCUMENTS,
        resolve_checked=b.resolve_checked,
        # SRC-12 part C: a preview that signs in with a refresh token may be
        # handed a new one, which must reach the API (the old one is retired).
        connector_id=request.connector_id,
    )
    return preview(columns_from_records(records))


# --- business apps (SRC-3 T1.10): the vendor module lists, the session reads ------------------------


def _saas(dial: dict, secrets: dict, request: DiscoverRequest, b: Backends) -> dict[str, Any]:
    module, _vendor, parts, session = b.saas_open(
        dial, secrets, connector_id=request.connector_id, resolve_checked=b.resolve_checked
    )
    if request.kind == "list" and request.schema == "fields":
        # Slice 3 review BLOCKER 9: a vendor whose objects the person DEFINES from the vendor's own
        # fields (Google Analytics: a report is dimensions and metrics) lists those fields, so the
        # console offers them instead of asking to type them. `schema` is unused by a business app
        # otherwise; the kinds are `dimension` and `metric`.
        fields_of = getattr(module, "list_fields", None)
        if fields_of is None:
            raise NotSupported("this vendor has no fields to build an object from")
        found = [
            f
            for f in fields_of(session, parts, dial)
            if contains_ignore_case(f["name"], request.search) or contains_ignore_case(f.get("label") or "", request.search)
        ]
        found.sort(key=lambda f: (f["kind"], f.get("label") or f["name"], f["name"]))
        return listing((listed(f["name"], f["kind"], label=f.get("label")) for f in found), request.limit)
    if request.kind == "list":
        declared = [
            d
            for d in module.list_objects(session, parts, dial)
            if contains_ignore_case(d.name, request.search) or contains_ignore_case(d.label or "", request.search)
        ]
        declared.sort(key=lambda d: (d.label or d.name, d.name))
        return listing((listed(d.name, "object", load=d.load_description(), label=d.label) for d in declared), request.limit)
    describe = getattr(module, "describe", None)
    if describe is None:
        raise NotSupported("this vendor does not describe its fields before a load")
    decl = module.declaration(session, parts, dial, request.object)
    fields = describe(session, parts, dial, request.object)
    if fields is None:
        raise NotSupported("this vendor does not describe the fields of this object before a load")
    return preview([column(f.name, f.type_name, inferred=False) for f in fields], list(decl.primary_key))


# --- the dispatch ------------------------------------------------------------------------------


def _reader_for(connector: dict) -> Callable[[dict, dict, DiscoverRequest, Backends], dict[str, Any]]:
    adapter, dial = connector["adapter"], connector["dial"]
    if adapter == "sql" and dial.get("driver") == "oracle":
        return _oracle
    readers = {"mongodb": _mongo, "kafka": _kafka, "sftp": _sftp, "rest": _rest, "saas": _saas}
    reader = readers.get(adapter)
    if reader is None:
        raise NotSupported("the orchestrator does not read this connector type")
    return reader


def discover(connector: dict, request: DiscoverRequest, *, backends: Backends | None = None) -> dict[str, Any]:
    """Read the source and return the result part of the answer (`objects` and
    `truncated`, or `columns` and `primaryKey`).

    # Errors

    Raises whatever the read raises -- `NotSupported`, `TooManyObjects`,
    `ssrf_guard.SsrfBlocked`, `secret_resolver.SecretRefRejected`, a driver's
    errors -- for `classify_discovery` to name. Nothing is caught here.
    """
    reader = _reader_for(connector)
    dial = connector["dial"]
    secrets = secret_resolver.resolve_secrets(
        connector["adapter"],
        # SRC-3: a saas dial's key is resolved by the vendor table.
        saas_vendors.auth_type_of(dial) if connector["adapter"] == "saas" else (dial.get("auth") or {}).get("type"),
        connector.get("secretRef"),
        connector.get("secretRefSecondary"),
    )
    return reader(dial, secrets, request, backends or Backends())


def result_body(request: DiscoverRequest, *, result: dict[str, Any] | None, error_class: str | None) -> dict[str, Any]:
    """The POST body: the request's id and kind, then either the result's fields or
    the failure's class. Exactly one of the two."""
    body: dict[str, Any] = {"requestId": request.request_id, "kind": request.kind}
    if error_class is not None:
        if error_class not in ERROR_CLASSES:
            raise ValueError("error_class is not one of ERROR_CLASSES")
        return {**body, "state": "failed", "errorClass": error_class}
    return {**body, "state": "done", **(result or {})}


def post_result(cfg: schema_observer.ObserverConfig, connector_id: str, body: dict[str, Any]) -> None:
    """POST `body` to the API (`schema_observer._post_json`: the service token,
    `timeout=`, `raise_for_status()`).

    # Errors

    `schema_observer.ObservationRefused` for a 4xx or an unset token,
    `schema_observer.ObservationUnreachable` when the API cannot be reached or
    answers 5xx. Both messages are fixed sentences.
    """
    schema_observer._post_json(  # noqa: SLF001 -- the one POST helper of the service identity
        cfg, f"/api/connectors/{connector_id}/discovery-results", body, "discovery result"
    )


def run_request(
    request: DiscoverRequest,
    *,
    connector_loader: Callable[[str], dict],
    poster: Callable[[dict[str, Any]], None],
    backends: Backends | None = None,
) -> str | None:
    """Read the source, then post the answer or the class of the failure. Returns
    the failure's class (`None` for a success), for the log.

    A failure of the READ is posted as a failed answer and does not fail the run.
    A failure of the POST does (it raises): then nobody has the answer, and the
    run must say so.
    """
    error_class: str | None = None
    result: dict[str, Any] | None = None
    try:
        connector = connector_loader(request.connector_id)
        result = discover(connector, request, backends=backends)
    except Exception as exc:  # noqa: BLE001 -- every failure of the read is classified and posted
        error_class = classify_discovery(exc)
        # The class and the exception's TYPE NAME only: never its text.
        # Warning level (SRC-13 review 3 BLOCKER 1): an unclassified failure must be visible in CI logs;
        # the type name is the library's class name, not upstream text.
        logger.warning("discovery failed: class=%s type=%s.%s", error_class, type(exc).__module__, type(exc).__name__)
    poster(result_body(request, result=result, error_class=error_class))
    return error_class


def _fetch_connector(connector_id: str) -> dict:
    """The connector with its dial and secret REFERENCES, from the API, as an ingest
    run fetches it (`ingest_factory._fetch_one_connector`). A connector the API
    does not list as ingestible is `StopIteration` there; said as `NotSupported`."""
    try:
        return _fetch_one_connector(IngestFactoryConfig.from_env(), connector_id)
    except StopIteration:
        raise NotSupported("the connector has no ingest spec") from None
    except requests.HTTPError as exc:
        status = getattr(exc.response, "status_code", None)
        if status in (401, 403):
            raise ingest_errors.ApiRefused("the API refused the connector request") from None
        raise ingest_errors.ApiUnreachable("the API failed on the connector request") from None
    except requests.RequestException:
        raise ingest_errors.ApiUnreachable("the API could not be reached") from None


@op(
    config_schema={
        "connector_id": Field(str, description="The connector.id to read."),
        "request_id": Field(str, description="The API's id for this request; echoed in the result."),
        "kind": Field(str, description="list | preview."),
        "object": Field(str, default_value="", description="The object to preview."),
        "schema": Field(str, default_value="", description="A SQL source's schema."),
        "glob": Field(str, default_value="", description="A file glob (SFTP)."),
        "search": Field(str, default_value="", description="Part of a name to search for."),
        "limit": Field(int, default_value=MAX_NAMES, description="The most objects to list."),
    },
    retry_policy=DEFAULT_RETRY_POLICY,
    tags=source_metadata(
        "dispar_orchestrate/discover.py::run_discover",
        reads=["The connector's source (names, a bounded sample)"],
        writes=["POST /api/connectors/{id}/discovery-results"],
    ),
)
def run_discover(context) -> None:
    """List or preview one connector's source and post the answer (module
    docstring). Never loads, never writes to the source."""
    request = DiscoverRequest.from_op_config(context.op_config)
    cfg = schema_observer.ObserverConfig.from_env()
    try:
        error_class = run_request(
            request,
            connector_loader=_fetch_connector,
            poster=lambda body: post_result(cfg, request.connector_id, body),
        )
    except schema_observer.ObservationRefused as exc:
        # A 4xx would be the same answer again: no retry (`DEFAULT_RETRY_POLICY`).
        raise Failure(str(exc), allow_retries=False) from None
    context.log.info(f"discovery {request.kind} for {request.connector_id!r} ended: {error_class or 'done'}")


@job(name="discover_job")
def discover_job() -> None:
    """Launched by `POST /api/connectors/{id}/discovery/{kind}` with run config
    `{"ops": {"run_discover": {"config": {...}}}}` (the API's `launch`)."""
    run_discover()
