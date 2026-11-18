"""Dagster job: the size of every layer, measured once a day (OPS-8 part C).

# What a layer is here

Four layers, measured by the system that already owns each one's data:

- `bronze` and `gold` are Iceberg namespaces in the REST catalog
  (`scripts/seed-clickhouse.sql` names `silver`/`serving` as ClickHouse
  databases, so they are NOT Iceberg namespaces -- the plan's "per
  namespace (bronze, silver, serving, gold)" wording was wrong about
  two of the four). Their size is the `total-files-size` of each table's
  CURRENT snapshot, read from the table's own metadata document -- the
  same number `lakehouse-iceberg`'s `stats_from_summary` reads and
  `routes/catalog.rs` already shows.
- `silver` and `serving` are ClickHouse databases. Their size is their
  own `system.parts` bytes on disk, and their table count is their own
  `system.tables` count. There is no Iceberg metadata to read for them.

# Where the rows go

`lake.bronze_meta.storage_layer_snapshot`, one row per layer per day.
The schema and the writer live in `bronze_catalog.py` beside that
module's other recorders, so the table has exactly ONE owner for its
shape and for its write -- the same split `capacity_snapshot.py` has
with `record_capacity_snapshot`.

# A table that cannot be measured is never a table of zero bytes

A table whose metadata cannot be read (or whose snapshot summary holds
no size) is counted in `tables_skipped`, with a FIXED reason, and its
bytes are left out of the layer's total. The reason is one of two
fixed sentences for the Iceberg layers (the ClickHouse layers have
their own, below): no upstream text (an error message from the catalog,
a file path, a hostname) ever reaches this table, and from there the
API.

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
from datetime import datetime, timezone

import requests
from dagster import DefaultScheduleStatus, Failure, ScheduleDefinition, job, op

from dispar_orchestrate import bronze_catalog, maintenance
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

# The only two reasons a table is skipped. Fixed sentences on purpose: the
# upstream text (a catalog error, a file path) must not reach the API.
SKIP_REASON_UNREADABLE = "the catalog's metadata could not be read"
SKIP_REASON_NO_SIZE = "the snapshot summary holds no size"

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
        and `_resolve_prefix` raise, `read_bytes` skips.
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

    def read_bytes(self, table_name: str) -> tuple[int | None, str]:
        """The size of one table, or `(None, reason)` when it cannot be
        measured. `table_name` is the catalog's own two-part name
        (`bronze.orders`)."""
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
            return None, SKIP_REASON_UNREADABLE
        size = _total_files_size(document)
        if size is None:
            return None, SKIP_REASON_NO_SIZE
        return size, ""


def measure_iceberg_layer(
    layer: str,
    tables: list[str],
    read_bytes: Callable[[str], tuple[int | None, str]],
) -> StorageLayerRow:
    """The size of one Iceberg layer: the sum of its tables' sizes.

    `read_bytes` is the seam: it answers `(size, reason)` for one table,
    and `(None, reason)` when the table cannot be measured. A table that
    cannot be measured is counted in `tables_skipped` and left OUT of the
    total -- a layer that could not be measured whole must never look
    measured and empty.

    `table_count` is the number of tables the catalog holds in this
    layer, measured tables and skipped ones alike: it says how many
    tables the layer HAS, and `tables_skipped` says how many of them are
    missing from the byte total.
    """
    total = 0
    skipped = 0
    reasons: list[str] = []
    for table in tables:
        size, reason = read_bytes(table)
        if size is None:
            skipped += 1
            reasons.append(reason)
            continue
        total += size
    return StorageLayerRow(
        layer=layer,
        table_count=len(tables),
        total_bytes=total,
        tables_skipped=skipped,
        skipped_reason="; ".join(dict.fromkeys(reasons)),
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
        )
    total_bytes = _as_int((bytes_rows[0] if bytes_rows else {}).get("total_bytes")) or 0
    return StorageLayerRow(
        layer=layer,
        table_count=table_count,
        total_bytes=total_bytes,
        tables_skipped=0,
        skipped_reason="",
    )


def measure_layers(cfg: maintenance.MaintenanceConfig) -> list[StorageLayerRow]:
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
    """
    maintenance.ensure_catalog_database(cfg)
    catalog_tables = maintenance.discover_catalog_tables(cfg)
    reader = _CatalogReader(cfg)
    rows: list[StorageLayerRow] = []
    for layer in ICEBERG_LAYERS:
        tables = [name for name in catalog_tables if name.startswith(f"{layer}.")]
        rows.append(measure_iceberg_layer(layer, tables, reader.read_bytes))
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
        ],
        writes=["ClickHouse lake.bronze_meta.storage_layer_snapshot"],
    ),
)
def run_storage_snapshot(context) -> None:
    """Measure every layer and record one row per layer for today."""
    cfg = maintenance.MaintenanceConfig.from_env()
    rows = measure_layers(cfg)
    for row in rows:
        context.log.info(
            f"[{row.layer}] {row.table_count} table(s), {row.total_bytes} byte(s)"
            + (f", {row.tables_skipped} skipped" if row.tables_skipped else "")
        )
    bronze_catalog.record_storage_layer_snapshot(
        day=_utc_today(),
        rows=rows,
        target=cfg.ch,
    )
    context.add_output_metadata({row.layer: row.total_bytes for row in rows})


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
