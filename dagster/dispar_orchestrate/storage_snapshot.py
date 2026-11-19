"""Dagster job: the size of every layer, measured once a day (OPS-8 part C).

# What a layer is here

Four layers, measured by the system that already owns each one's data:

- `bronze` and `gold` are Iceberg namespaces in the REST catalog
  (`scripts/seed-clickhouse.sql` names `silver`/`serving` as ClickHouse
  databases, so they are NOT Iceberg namespaces -- the plan's "per
  namespace (bronze, silver, serving, gold)" wording was wrong about
  two of the four). Each of their tables carries TWO numbers: the
  `total-files-size` of its CURRENT snapshot (the live data), read from
  the table's own metadata document -- the same number
  `lakehouse-iceberg`'s `stats_from_summary` reads and
  `routes/catalog.rs` already shows -- and the bytes the object store
  holds under the table's own LOCATION (the live data, the older
  snapshots' files, the metadata and the orphan files).
- `silver` and `serving` are ClickHouse databases. Their size is their
  own `system.parts` bytes on disk, and their table count is their own
  `system.tables` count. There is no Iceberg metadata to read for them.

# Two numbers for an Iceberg layer: the current data and what is on disk

The `total-files-size` of a table's current snapshot counts the LIVE
data only: the files the current snapshot points at. The table's own
LOCATION holds more than that -- the older snapshots' data files, the
metadata, the manifest lists, and the files of the tables that were
dropped. On the staging warehouse of 2026-11-18 the bucket held
4,140,446,845 B while the bronze layer's current data was 1,002,969,464
B over 682 tables: the two numbers measure different things, and the
page shows both -- "current data" (the snapshot number) and "stored on
disk" (the bytes the object store holds under the table's own
location).

A table is MEASURED only when BOTH numbers were read. A table that
could not be measured is a skip with a fixed reason and contributes to
NEITHER total: never a zero, and never one number without the other.

# The object store is read through the capacity job's ONE setup

`capacity_snapshot.py` is where this project reads the object store.
Its `build_filesystem` (the credentials and the endpoint) and its
`list_stored_objects` (what counts as a stored object) are the helpers
this module uses, so there is ONE copy of that logic and not two
(`AGENTS.md` rule 4). The configuration is the capacity job's own
(`capacity_snapshot.CapacityConfig.from_env`), which reads the same
`CONNECTOR_S3_*` credentials, the same endpoint and the same bucket: the
"stored on disk" number and the bucket size are measured against the
same bucket, or they are not comparable at all.

# The object store's own failure is not a skip

The bucket root is listed ONCE, before any table is measured. A failure
there is the store's own failure -- a wrong endpoint, a refused
credential -- and it fails the RUN with `dagster.Failure` naming the
environment variables to set. A listing of ONE table's location that
fails is that table's problem: a skip with its fixed reason, and the
other tables are still measured.

# Where the rows go

`lake.bronze_meta.storage_layer_snapshot`, one row per layer per day.
The schema and the writer live in `bronze_catalog.py` beside that
module's other recorders, so the table has exactly ONE owner for its
shape and for its write -- the same split `capacity_snapshot.py` has
with `record_capacity_snapshot`.

# A table that cannot be measured is never a table of zero bytes

A table whose metadata cannot be read (or whose snapshot summary holds
no size, or whose files cannot be listed, or whose location is not under
the measured bucket) is counted in `tables_skipped`, with a FIXED reason,
and its bytes are left out of BOTH of the layer's totals. The reason is
one of four fixed sentences for the Iceberg layers (the ClickHouse
layers have their own, below): no upstream text (an error message from
the catalog, a file path, a hostname, an S3 error) ever reaches this
table, and from there the API.

# The catalog database is created by the job itself

The list of Iceberg tables comes from ClickHouse's own `SHOW TABLES
FROM icecat_maintenance`, and that database exists only because some
job created it: the bronze-maintenance job
(`maintenance.ensure_catalog_database`). This job runs at 01:00 and
that one at 03:00, and on a fresh stack NEITHER has run -- so the list
would answer `UNKNOWN_DATABASE`, no row would ever be written, and
`GET /api/usage/storage` would answer 503. The database is therefore
created here, exactly the way the sibling job creates it, before the
listing.

# The ClickHouse layers are read with the columns ClickHouse 26.8 has

`silver` and `serving` are measured with two statements: how many tables
the database holds (`system.tables`) and how many bytes its own parts
occupy on disk (`system.parts`). ClickHouse 26.8 -- the version
`docker-compose.yml` pins (`clickhouse/clickhouse-server:26.8`, 26.8.36
on the stack this was measured on) -- has NO `type` column in
`system.tables`: `DESCRIBE system.tables` lists `engine` (the engine
name: `MergeTree`, `ReplicatedMergeTree`, `View`, `MaterializedView`,
...) instead, and a statement that names a column the system table does
not have is refused with UNKNOWN_IDENTIFIER -- which failed every run
of this job on the two ClickHouse layers. The engine name is therefore
what says whether an entry is a table (`engine NOT LIKE '%View'`): a view
is not a table of its own, and the bytes of the table it reads are
already counted through `system.parts`.

They are two statements and not one because a failure must not cost the
layer its table count. A layer whose own size cannot be read is a SKIP
with a fixed reason, never a layer of zero bytes: the bytes that could
not be read are left out of the total, the tables that were not measured
are counted in `tables_skipped`, and the OTHER layers are still
measured -- a degraded-but-continuable failure never stops the loop.

# A configuration or auth failure is not a skip

Two failure idioms, chosen by class (`docs/CODE-STANDARD.md`): a
configuration/auth failure -- no `CH_OAUTH_CLIENT_ID`, a token endpoint
that refuses, a catalog that names no warehouse -- raises
`dagster.Failure` with the exact environment variable to set, because
without a token NO table of any Iceberg layer can be read, and a run
that reported "every table skipped, zero bytes" would hide a broken
configuration behind an empty number. A per-table read failure -- a 404,
a timeout, a body that is not JSON -- stays a skip with its fixed
reason, and the layer's bytes are left out rather than counted as zero.

# The helpers this module uses are the PUBLIC ones

`maintenance.mint_lakekeeper_read_token`,
`maintenance.resolve_catalog_prefix` and `ClickHouseTarget.query_json`
are the public names of the modules that own them: a private name of
another module is a name that may change under this one (`AGENTS.md`
rule 4). The one private helper this module still uses is
`bronze_catalog._sql_string_literal`, because that is THE way ClickHouse
SQL is built here (`docs/CODE-STANDARD.md`).

The object store is reached the same way: `capacity_snapshot.build_filesystem`
and `capacity_snapshot.list_stored_objects` are the PUBLIC names of the
module that owns the bucket, for the same reason.

# The run limit

The job carries its own `dagster/max_runtime` tag (thirty minutes by
default). A run whose process dies would otherwise hold a concurrency
slot forever -- the failure the staging Dagster hit on 13 Nov 2026
(ten runs left STARTED with a dead process, filling the ten-run
concurrency limit and queueing 570 runs behind them). The limit is a
tag on THIS job alone, read from its own setting, through the same
`ingest_factory.run_limit_tags` rule `ingest_job` uses -- one parsing
rule, two settings, two defaults (`AGENTS.md` rule 4).
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from datetime import datetime, timezone

import requests
from dagster import DefaultScheduleStatus, Failure, ScheduleDefinition, job, op
from s3fs import S3FileSystem

from dispar_orchestrate import bronze_catalog, capacity_snapshot, maintenance
from dispar_orchestrate.bronze_catalog import ClickHouseTarget, StorageLayerRow
from dispar_orchestrate.ingest_factory import run_limit_tags
from dispar_orchestrate.op_metadata import DEFAULT_RETRY_POLICY, source_metadata

# The Iceberg layers: namespaces of the REST catalog, measured from each
# table's current snapshot summary.
ICEBERG_LAYERS = ("bronze", "gold")

# The ClickHouse layers: databases of the warehouse, measured from their
# own `system.parts`/`system.tables`.
CLICKHOUSE_LAYERS = ("silver", "serving")

# How long one run of this job may run, in seconds. Far above a sum over
# table metadata, far below the six hours `ingest_job` gets.
RUN_LIMIT_ENV = "DAGSTER_RUN_LIMIT_STORAGE_LAYER_SNAPSHOT_JOB_SECONDS"
DEFAULT_RUN_LIMIT_SECONDS = 1_800

# The reasons an Iceberg table is skipped. Fixed sentences on purpose: the
# upstream text (a catalog error, a file path, an S3 error message) must
# not reach the API.
#
# The first two are about the catalog's own metadata document; the next two
# are about the object store the table's files live in. A table that could
# not be measured has ONE reason, the first one that applies.
SKIP_REASON_UNREADABLE = "the catalog's metadata could not be read"
SKIP_REASON_NO_SIZE = "the snapshot summary holds no size"
SKIP_REASON_NOT_LISTED = "the table's files could not be listed"
SKIP_REASON_OUTSIDE_BUCKET = "the table lives outside the measured bucket"

# The reason a ClickHouse layer's own size could not be read. Its own
# sentence, because the two above are about the Iceberg metadata: a
# ClickHouse layer is not read through the catalog at all.
SKIP_REASON_LAYER_UNREADABLE = "the layer's own size could not be read"


def _as_int(value: object) -> int | None:
    """A number as an integer, or `None` when the value is not one.

    ClickHouse returns every number as a JSON string in `FORMAT JSON`, and
    an Iceberg snapshot summary carries its sizes as strings too.
    """
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    if isinstance(value, str) and value.strip().isdigit():
        return int(value.strip())
    return None


def _total_files_size(document: object) -> int | None:
    """The `total-files-size` of a table's CURRENT snapshot, or `None`.

    Read from the metadata document the REST catalog serves for one table
    (`GET /v1/{prefix}/namespaces/{ns}/tables/{table}`). The catalog wraps
    the Iceberg metadata document in a `metadata` key; a document that
    carries the Iceberg fields at the top level is accepted too, so the
    reader does not depend on the wrapper being there.

    Two places hold the number: the table's own `snapshot-summary` (the
    copy the writer of the last commit left) and the current snapshot's
    own `summary`. The snapshot's own summary wins when both are there,
    because it is the one the `current-snapshot-id` points at.
    """
    if not isinstance(document, dict):
        return None
    metadata = document.get("metadata")
    if not isinstance(metadata, dict):
        metadata = document

    summary = metadata.get("snapshot-summary")
    if isinstance(summary, dict):
        size = _as_int(summary.get("total-files-size"))
        if size is not None:
            return size

    current_id = metadata.get("current-snapshot-id")
    snapshots = metadata.get("snapshots")
    if not isinstance(snapshots, list):
        return None
    for snapshot in snapshots:
        if not isinstance(snapshot, dict):
            continue
        if current_id is not None and snapshot.get("snapshot-id") != current_id:
            continue
        snapshot_summary = snapshot.get("summary")
        if not isinstance(snapshot_summary, dict):
            continue
        size = _as_int(snapshot_summary.get("total-files-size"))
        if size is not None:
            return size
    return None


def _table_location(document: object) -> str | None:
    """The table's own location (`s3://<bucket>/<key>`), or `None`.

    Read from the same metadata document `_total_files_size` reads, in the
    shapes the Iceberg REST catalog serves it: the table's own `location`
    field (`metadata.location`), the same field one level deeper
    (`metadata.storage.location`), and the top-level `location` of a
    document that carries the Iceberg fields without the `metadata`
    wrapper -- the reader does not depend on the wrapper being there.

    The location is where the table's files live, so it is the one thing
    the "stored on disk" number is measured from; a document that does
    not carry it cannot be measured.
    """
    if not isinstance(document, dict):
        return None
    metadata = document.get("metadata")
    if not isinstance(metadata, dict):
        metadata = document
    candidates = [metadata.get("location")]
    storage = metadata.get("storage")
    if isinstance(storage, dict):
        candidates.append(storage.get("location"))
    for candidate in candidates:
        if isinstance(candidate, str) and candidate.strip():
            return candidate.strip()
    return None


@dataclass(frozen=True)
class CatalogTable:
    """What the catalog says about one table: the size of its CURRENT
    snapshot, and the location its files live under.

    `reason` is empty when the table was read; otherwise it is the fixed
    sentence that says why it could not be measured, and `current_bytes`
    is empty with it."""

    current_bytes: int | None
    location: str
    reason: str


def _key_in_bucket(location: str, bucket: str) -> str | None:
    """The location's key inside `bucket`, or `None` when the location is
    not under it.

    A location is `s3://<bucket>/<key>`: the bucket says whether the
    table's files are in the bucket this job measures at all, and the key
    is what `s3fs` lists under. A location with another scheme, another
    bucket, or no scheme at all cannot be listed against this store's
    filesystem — `s3fs` would read the first segment of a scheme-less
    path as a bucket name — so it is not a table of this bucket."""
    scheme, separator, rest = location.partition("://")
    if not separator or scheme != "s3":
        return None
    bucket_name, _, key = rest.partition("/")
    if bucket_name != bucket or not key:
        return None
    return key


class ObjectStore:
    """The warehouse bucket, as the object store sees it.

    ONE filesystem per run, built by `capacity_snapshot.build_filesystem`
    (the one copy of the credential/endpoint logic in this project), and
    two operations: a probe of the bucket root, and the bytes under one
    table's location.

    The two operations fail in two different ways, and the difference is
    the whole point of this class: a failure of the PROBE is the store's
    own failure — a wrong endpoint, a refused credential — and it fails
    the run (`dagster.Failure`, naming the environment variables to
    set), while a failure of ONE table's listing is that table's
    problem: a skip with a fixed reason, and the other tables are still
    measured (`docs/CODE-STANDARD.md`: a configuration/auth failure is a
    `Failure`, a degraded-but-continuable failure is a skip)."""

    def __init__(self, fs: S3FileSystem, bucket_name: str) -> None:
        self._fs = fs
        self._bucket = bucket_name

    @classmethod
    def from_capacity_config(cls, cfg: capacity_snapshot.CapacityConfig) -> "ObjectStore":
        """The store this job reads, from the configuration the capacity
        job reads: the same `CONNECTOR_S3_*` credentials, the same
        endpoint and the same bucket."""
        return cls(capacity_snapshot.build_filesystem(cfg), cfg.bucket_name)

    def probe(self) -> None:
        """The bucket root, listed ONCE, before any table is measured.

        `ls` and not `find`: the root listing is the cheapest call the
        store offers, and it is enough to tell "the store answers" from
        "the store refuses to answer at all"."""
        try:
            self._fs.ls(self._bucket)
        except Exception:  # noqa: BLE001 -- the object store raises backend-specific types
            # The upstream text (an S3 error, a hostname) stays out of the
            # message (`AGENTS.md`: upstream error text never reaches a
            # response).
            raise Failure(
                "the warehouse bucket could not be listed — check "
                "CONNECTOR_S3_ACCESS_KEY, CONNECTOR_S3_SECRET_KEY, "
                "CH_RUSTFS_S3_ENDPOINT and LAKEHOUSE_WAREHOUSE_BUCKET",
                allow_retries=False,
            ) from None

    def stored_bytes(self, location: str) -> tuple[int | None, str]:
        """The bytes under one table's location, or `(None, reason)`.

        `location` is the table's own location from the catalog's metadata
        document (`s3://<bucket>/<key>`). A location that is not under the
        measured bucket is not this store's business: its bytes would not
        be comparable with the bucket's size, so the table is skipped with
        its own reason."""
        key = _key_in_bucket(location, self._bucket)
        if key is None:
            return None, SKIP_REASON_OUTSIDE_BUCKET
        # The LISTED path carries the bucket: `s3fs` reads the FIRST
        # segment of a scheme-less path as the bucket name, so the bare
        # key would list a bucket named after the table's first folder
        # (`warehouse`), not the measured one -- a bucket that does not
        # exist, or worse one that does and holds other objects. The path
        # has the same shape `capacity_snapshot.measure_bucket` lists the
        # whole bucket with: the bucket, then the key.
        path = f"{self._bucket}/{key}"
        try:
            bytes_, _objects = capacity_snapshot.list_stored_objects(self._fs, path)
        except Exception:  # noqa: BLE001 -- the object store raises backend-specific types
            return None, SKIP_REASON_NOT_LISTED
        return bytes_, ""


class _CatalogReader:
    """Reads one table's metadata document from the Iceberg REST catalog.

    The catalog's token and warehouse prefix are resolved ONCE and kept:
    a layer with fifty tables would otherwise mint fifty tokens and read
    the catalog's configuration fifty times.
    """

    def __init__(self, cfg: maintenance.MaintenanceConfig) -> None:
        self._cfg = cfg
        self._token: str | None = None
        self._prefix: str | None = None

    def _auth(self) -> tuple[str, str]:
        """The catalog's bearer token and its warehouse prefix.

        A configuration or auth failure here is a WHOLE-RUN failure, not a
        per-table skip: without a token no table of any Iceberg layer can
        be read, and a run that reported "every table skipped, zero
        bytes" would hide a broken configuration behind an empty number
        (`docs/CODE-STANDARD.md`: a configuration/auth failure is a
        `dagster.Failure`; only a degraded-but-continuable failure is a
        skip). The two cases are kept apart on purpose -- `_mint_token`
        and `_resolve_prefix` raise, `read_table` skips.
        """
        if self._token is None:
            self._token = self._mint_token()
            self._prefix = self._resolve_prefix(self._token)
        return self._token, self._prefix or ""

    def _mint_token(self) -> str:
        """A catalog read token, or a `Failure` naming what to set."""
        try:
            token = maintenance.mint_lakekeeper_read_token(self._cfg)
        except requests.RequestException:
            # The token endpoint refused the request (a 401, a 500): a
            # configuration or auth problem, never a per-table skip. The
            # upstream text stays out of the message (`AGENTS.md`: upstream
            # error text never reaches a response).
            raise Failure(
                "the Iceberg catalog's token endpoint refused the request — "
                "check CH_OAUTH_SERVER_URI (the token endpoint's URL) and "
                "CH_OAUTH_CLIENT_ID (the reader principal's client id)",
                allow_retries=False,
            ) from None
        except RuntimeError:
            # The endpoint answered, but with no access token in it.
            raise Failure(
                "the Iceberg catalog's token endpoint returned no access "
                "token — check CH_OAUTH_CLIENT_ID and CH_OAUTH_SERVER_URI",
                allow_retries=False,
            ) from None
        if not token:
            # No per-principal auth on this stack (a pre-R1 or
            # authz-disabled one): there is no way to read the catalog at
            # all, so no layer can be measured.
            raise Failure(
                "CH_OAUTH_CLIENT_ID is not set — the Iceberg catalog cannot "
                "be read: no catalog token can be minted, so neither the "
                "bronze nor the gold layer can be measured",
                allow_retries=False,
            )
        return token

    def _resolve_prefix(self, token: str) -> str:
        """The catalog's warehouse prefix, or a `Failure` naming what to set."""
        try:
            return maintenance.resolve_catalog_prefix(self._cfg, token)
        except requests.RequestException:
            raise Failure(
                "the Iceberg catalog's /v1/config refused the request — "
                "check LAKEKEEPER_CATALOG_URI (the catalog's URL) and "
                "CH_OAUTH_CLIENT_ID (the reader principal's client id)",
                allow_retries=False,
            ) from None
        except RuntimeError:
            raise Failure(
                "the Iceberg catalog returned no warehouse prefix — check "
                "LAKEKEEPER_WAREHOUSE (the warehouse name) and "
                "LAKEKEEPER_CATALOG_URI (the catalog's URL)",
                allow_retries=False,
            ) from None

    def read_table(self, table_name: str) -> CatalogTable:
        """One table's current size and the location its files live under,
        or the reason it could not be read. `table_name` is the catalog's
        own two-part name (`bronze.orders`).

        The size and the location come from the SAME document: they are
        two facts about one table, and reading the document twice would
        be two chances to read a different one."""
        namespace, _, table = table_name.partition(".")
        # The token and the prefix are resolved OUTSIDE the try below, so
        # that a configuration or auth failure there stays a whole-run
        # failure (`_auth`) and only a failure to read THIS table's
        # document is a skip.
        token, prefix = self._auth()
        try:
            resp = requests.get(
                f"{self._cfg.lakekeeper_catalog_uri}/v1/{prefix}"
                f"/namespaces/{namespace}/tables/{table}",
                headers={"Authorization": f"Bearer {token}"},
                timeout=10,
            )
            resp.raise_for_status()
            document = resp.json()
        except (requests.RequestException, ValueError):
            # A per-table read failure -- a 404, a timeout, a body that is
            # not JSON -- is the same skip: the table's bytes are unknown.
            # The reason is fixed, never the upstream text.
            return CatalogTable(None, "", SKIP_REASON_UNREADABLE)
        size = _total_files_size(document)
        if size is None:
            return CatalogTable(None, "", SKIP_REASON_NO_SIZE)
        location = _table_location(document)
        if location is None:
            # The location is part of the metadata document the table is
            # measured from: a document that does not carry it has not
            # given us what the measurement needs, and the table's files
            # cannot be listed without it.
            return CatalogTable(None, "", SKIP_REASON_UNREADABLE)
        return CatalogTable(size, location, "")


def measure_iceberg_layer(
    layer: str,
    tables: list[str],
    read_table: Callable[[str], CatalogTable],
    store: ObjectStore,
) -> StorageLayerRow:
    """The size of one Iceberg layer: the sum of its tables' sizes.

    TWO numbers per table: the CURRENT data (the current snapshot's
    `total-files-size`, from the catalog) and the bytes STORED ON DISK
    under the table's own location (from the object store). A table is
    measured only when BOTH were read: a table with one of the two
    missing is a skip with its fixed reason, and it contributes to
    NEITHER total -- a layer that could not be measured whole must never
    look measured and empty.

    `read_table` and `store` are the seams: the catalog's read and the
    object store's listing.

    `table_count` is the number of tables the catalog holds in this
    layer, measured tables and skipped ones alike: it says how many
    tables the layer HAS, and `tables_skipped` says how many of them are
    missing from the byte totals.
    """
    total = 0
    stored = 0
    skipped = 0
    reasons: list[str] = []
    for table in tables:
        read = read_table(table)
        if read.current_bytes is None:
            skipped += 1
            reasons.append(read.reason)
            continue
        stored_bytes, reason = store.stored_bytes(read.location)
        if stored_bytes is None:
            skipped += 1
            reasons.append(reason)
            continue
        total += read.current_bytes
        stored += stored_bytes
    return StorageLayerRow(
        layer=layer,
        table_count=len(tables),
        total_bytes=total,
        tables_skipped=skipped,
        skipped_reason="; ".join(dict.fromkeys(reasons)),
        stored_bytes=stored,
    )


def _clickhouse_layer_statements(layer: str) -> tuple[str, str]:
    """The two statements that measure one ClickHouse layer: how many
    tables it holds, and how many bytes its own parts occupy on disk.

    They are two statements and not one because a failure must not cost
    the layer its table count: a layer whose bytes cannot be read is
    still a layer of N tables, and the row says how many of them were
    not measured. `system.parts` holds one row per part, so the bytes are
    a sum over parts and the count cannot come from it; `system.tables`
    holds one row per table, which is where the count comes from. Only
    ACTIVE parts count: a detached part's bytes are not the database's
    storage.

    ClickHouse 26.8 -- the version `docker-compose.yml` pins
    (`clickhouse/clickhouse-server:26.8`, 26.8.36 on the stack this was
    measured on) -- has NO `type` column in `system.tables`:
    `DESCRIBE system.tables` lists `engine` (the engine name:
    `MergeTree`, `ReplicatedMergeTree`, `View`, `MaterializedView`,
    `LiveView`, `WindowView`, ...) and no `type`, and a statement that
    names a column the system table does not have is refused with
    UNKNOWN_IDENTIFIER. So the engine name is what decides whether an
    entry is a table: `engine NOT LIKE '%View'` counts the tables and
    leaves the views out, because a view is not a table of its own -- the
    bytes of the table it reads are already counted through
    `system.parts`. (A materialized view's own inner storage, if it has
    any, is counted in the BYTES through `system.parts` but not in the
    table count: the two numbers come from two different system tables,
    and the bytes are the authoritative one. Neither `silver` nor
    `serving` holds a view -- `scripts/seed-clickhouse.sql` creates three
    `silver` tables and six `serving` tables, all `MergeTree`.)
    """
    # ClickHouse SQL is built through `_sql_string_literal`
    # (`docs/CODE-STANDARD.md`): the layer's name is a fixed literal of
    # this module, never end-user input.
    quoted = bronze_catalog._sql_string_literal(layer)
    count_statement = (
        "SELECT count() AS table_count FROM system.tables "
        f"WHERE database = {quoted} AND engine NOT LIKE '%View'"
    )
    bytes_statement = (
        "SELECT coalesce(sum(bytes_on_disk), 0) AS total_bytes FROM system.parts "
        f"WHERE database = {quoted} AND active"
    )
    return count_statement, bytes_statement


def measure_clickhouse_layer(layer: str, target: ClickHouseTarget) -> StorageLayerRow:
    """The size of one ClickHouse layer: its own tables, and the bytes its
    own parts occupy on disk.

    A layer that cannot be read is a SKIP with the fixed reason, never a
    layer of zero bytes: the bytes that could not be read are left out of
    the total, and the tables that were not measured are counted in
    `tables_skipped` with the reason. The other layers are still measured
    -- a degraded-but-continuable failure never stops the loop
    (`docs/CODE-STANDARD.md`).
    """
    count_statement, bytes_statement = _clickhouse_layer_statements(layer)
    try:
        count_rows = target.query_json(count_statement)
    except (requests.RequestException, ValueError):
        # Not even the table count could be read: the layer is skipped as
        # a whole. The count stays 0 rather than inventing one, and the
        # `tables_skipped` marker says the layer was NOT measured, so the
        # row is never read as a measured layer of zero bytes.
        return StorageLayerRow(
            layer=layer,
            table_count=0,
            total_bytes=0,
            tables_skipped=1,
            skipped_reason=SKIP_REASON_LAYER_UNREADABLE,
            stored_bytes=None,
        )
    table_count = _as_int((count_rows[0] if count_rows else {}).get("table_count")) or 0
    try:
        bytes_rows = target.query_json(bytes_statement)
    except (requests.RequestException, ValueError):
        # The tables are known, their bytes are not: every one of them is
        # marked as not measured, and none of their bytes is counted.
        return StorageLayerRow(
            layer=layer,
            table_count=table_count,
            total_bytes=0,
            tables_skipped=table_count,
            skipped_reason=SKIP_REASON_LAYER_UNREADABLE,
            stored_bytes=None,
        )
    total_bytes = _as_int((bytes_rows[0] if bytes_rows else {}).get("total_bytes")) or 0
    return StorageLayerRow(
        layer=layer,
        table_count=table_count,
        total_bytes=total_bytes,
        tables_skipped=0,
        skipped_reason="",
        # A ClickHouse layer's bytes sit on the ClickHouse server's own
        # disk, not in the warehouse bucket: there is no location to
        # list, so there is no second number. `None` says "not measured
        # here", where a zero would say "this layer holds no bytes".
        stored_bytes=None,
    )


def measure_layers(
    cfg: maintenance.MaintenanceConfig, store: ObjectStore
) -> list[StorageLayerRow]:
    """Every layer's size, as one row per layer.

    The Iceberg layers come from the catalog's own table list (the whole
    list, so `gold` is measured as well as `bronze`); the ClickHouse
    layers come from ClickHouse itself. A namespace that is not a layer
    (`scratch`, say) is not measured at all.

    The catalog database the list is read from is created HERE, before
    the list is read: it is the bronze-maintenance job that creates it
    (`maintenance.ensure_catalog_database`), and that job runs at 03:00
    while this one runs at 01:00 -- on a fresh stack neither has run, so
    the listing would answer `UNKNOWN_DATABASE` and no row would ever be
    written.

    The object store is probed ONCE, before any table is read: a bucket
    that cannot be listed at all is the store's own failure and fails
    the run, rather than reporting "every table skipped, zero bytes"."""
    store.probe()
    maintenance.ensure_catalog_database(cfg)
    catalog_tables = maintenance.discover_catalog_tables(cfg)
    reader = _CatalogReader(cfg)
    rows: list[StorageLayerRow] = []
    for layer in ICEBERG_LAYERS:
        tables = [name for name in catalog_tables if name.startswith(f"{layer}.")]
        rows.append(measure_iceberg_layer(layer, tables, reader.read_table, store))
    for layer in CLICKHOUSE_LAYERS:
        rows.append(measure_clickhouse_layer(layer, cfg.ch))
    return rows


def _utc_today() -> str:
    """Today's UTC date, as `YYYY-MM-DD` -- the `day` column's own type."""
    return datetime.now(timezone.utc).strftime("%Y-%m-%d")


@op(
    retry_policy=DEFAULT_RETRY_POLICY,
    tags=source_metadata(
        "dispar_orchestrate/storage_snapshot.py::run_storage_snapshot",
        reads=[
            "Iceberg REST catalog (every table's current snapshot)",
            "ClickHouse system.tables, system.parts",
            "RustFS warehouse bucket (every table's own location)",
        ],
        writes=["ClickHouse lake.bronze_meta.storage_layer_snapshot"],
    ),
)
def run_storage_snapshot(context) -> None:
    """Measure every layer and record one row per layer for today."""
    cfg = maintenance.MaintenanceConfig.from_env()
    # The object store's own configuration: the SAME one the capacity job
    # reads (`CONNECTOR_S3_*`, `CH_RUSTFS_S3_ENDPOINT`,
    # `LAKEHOUSE_WAREHOUSE_BUCKET`), so the credentials and the endpoint
    # are read in one place and the "stored on disk" number and the
    # bucket size are measured against the same bucket. A missing
    # credential is a `Failure` here, before any table is read.
    store = ObjectStore.from_capacity_config(capacity_snapshot.CapacityConfig.from_env())
    rows = measure_layers(cfg, store)
    for row in rows:
        context.log.info(
            f"[{row.layer}] {row.table_count} table(s), {row.total_bytes} byte(s)"
            + (
                f", {row.stored_bytes} byte(s) on disk"
                if row.stored_bytes is not None
                else ""
            )
            + (f", {row.tables_skipped} skipped" if row.tables_skipped else "")
        )
    bronze_catalog.record_storage_layer_snapshot(
        day=_utc_today(),
        rows=rows,
        target=cfg.ch,
    )
    context.add_output_metadata(
        {row.layer: row.total_bytes for row in rows}
        | {f"{row.layer} stored on disk": row.stored_bytes for row in rows}
    )


# The `dagster/max_runtime` tag is what run monitoring enforces: a run
# still going after the limit is cancelled and failed by the daemon, so a
# run whose process dies does not hold a concurrency slot forever. It sits
# on THIS job alone -- `run_monitoring.max_runtime_seconds` would cap
# every job of the instance -- and it is `tags` and not `run_tags` for
# the reason `ingest_factory.py` gives: both launch paths copy a job's
# `tags` onto the run when `run_tags` is unset.
@job(name="storage_snapshot_job", tags=run_limit_tags(RUN_LIMIT_ENV, DEFAULT_RUN_LIMIT_SECONDS))
def storage_snapshot_job() -> None:
    """`DAGSTER_LOCATION`-visible job name: `storage_snapshot_job`."""
    run_storage_snapshot()


# Daily at 01:00 -- before `capacity_snapshot_schedule`'s 02:00 (the
# bucket size) and `bronze_maintenance_schedule`'s 03:00 (the maintenance
# pass), so a storage read never overlaps either of them.
# `default_status=DefaultScheduleStatus.RUNNING` for the reason
# `capacity_snapshot.py` states explicitly: a schedule created STOPPED
# never fires until someone notices and toggles it in the Dagster UI, and
# a stopped schedule looks identical to a healthy one from outside --
# storage silently never being measured is exactly the failure this job
# exists to prevent.
storage_snapshot_schedule = ScheduleDefinition(
    job=storage_snapshot_job,
    cron_schedule="0 1 * * *",
    default_status=DefaultScheduleStatus.RUNNING,
)
