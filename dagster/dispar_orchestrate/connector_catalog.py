"""Register a loaded Bronze table in the console catalog.

`ingest_job` (`ingest_factory.py`) writes each of a connector's source
objects to a Bronze Iceberg table, and `file_ingest_job` (`file_ingest.py`)
writes an uploaded file to one. Nothing else put those tables in
`lake.bronze_meta.dataset_catalog`, the registry `GET /api/catalog` lists
Bronze from, so a table loaded either way existed and could be queried, yet
never showed in the Catalog. `register_loaded_table` is that registration,
one helper for both callers so the upload job carries no second copy of it
(review finding A1 of `docs/superpowers/plans/2026-10-02-upload-file.md`);
`register_connector_table` is the connector's wrapper around it. The demo
Bronze job (`assets.py`) registers its own table through
`register_bronze_table` directly.

# What is registered

- `slug`/`title` derived from the Bronze table name.
- The table's TOTAL, counted through a ClickHouse `DataLakeCatalog` database
  after the load, not the rows this run loaded. What a table holds depends
  on the load mode (`adapters/sink.py`, "Load modes"): a `replace` run, the
  default since `0941ce4`, leaves one copy of its source, and an `append`
  run adds to what is already there. The Catalog's row total
  (`dataset_sync.total`) must say what the table holds now, and only a total
  taken after the load does. (Review finding A2: this docstring used to say
  Bronze is append-only, which stopped being true with load modes.)
- The columns and their types, read the same way (`DESCRIBE`), minus dlt's
  own `_dlt_*` bookkeeping columns.

Registration is an upsert (`ReplacingMergeTree ORDER BY slug`), so every
successful run refreshes the entry.

# The ClickHouse helpers

`ch_target`, `ensure_catalog_database` and `ch_exec` used to live in
`ch_models.py`, a runner for Silver and Gold SQL models that had no caller
once the transformation modules left (review finding A1). They moved here
with the one caller that still needed them, and the rest of that file went.
Every statement below interpolates only fixed literals, `CATALOG_DB`, settings
an operator sets in the environment, and a table name that passed
`is_plain_table_name`. An upload's table name is typed by a user and a
connector's target is stored unvalidated, so that check is all that stands
between either and the SQL.
"""

from __future__ import annotations

import os
import re
from typing import Callable

import requests

from dispar_orchestrate.bronze_catalog import ClickHouseTarget, register_bronze_table

# This job's own `DataLakeCatalog` database, separate from every other
# job's (`maintenance.py` keeps `icecat_maintenance` the same way), so
# registering a table never depends on another job having created a
# database first.
CATALOG_DB = "icecat_ingest"

# A table name is interpolated into SQL below. `sourceObjects[].target` is
# stored unvalidated server-side (the console checks it, the API does not),
# and an upload's table name is the user's, so anything but a plain
# lower-case identifier is refused here.
_TABLE_NAME = re.compile(r"[a-z_][a-z0-9_]*")


def is_plain_table_name(name: str) -> bool:
    """Whether `name` is a plain lower-case identifier, safe to interpolate
    into the SQL below.

    `fullmatch`, not `match` with a `$`: `$` also matches in front of a
    trailing newline, so `"orders\\n"` would have passed.
    """
    return _TABLE_NAME.fullmatch(name) is not None


def _env(name: str, default: str) -> str:
    value = os.environ.get(name, "").strip()
    return value or default


def ch_target() -> ClickHouseTarget:
    return ClickHouseTarget.from_env()


def ensure_catalog_database(ch: ClickHouseTarget, db: str) -> None:
    """Create a `DataLakeCatalog` database so ClickHouse can read Bronze
    Iceberg tables through Lakekeeper.

    Settings mirror `maintenance.py`'s own catalog database, including the
    `clickhouse-reader` OAuth credential R1/ADR 0011 requires when
    Lakekeeper authorization is enabled (empty on an authz-disabled stack).
    """
    auth = ""
    client_id = _env("CH_OAUTH_CLIENT_ID", "")
    if client_id:
        auth = (
            f", catalog_credential = '{client_id}:unused', "
            f"oauth_server_uri = '{_env('CH_OAUTH_SERVER_URI', '')}'"
        )
    ch_exec(
        ch,
        f"CREATE DATABASE IF NOT EXISTS {db} "
        f"ENGINE = DataLakeCatalog('{_env('LAKEKEEPER_CATALOG_URI', 'http://lakekeeper:8181/catalog')}') "
        f"SETTINGS catalog_type = 'rest', warehouse = '{_env('LAKEKEEPER_WAREHOUSE', 'default')}', "
        f"storage_endpoint = '{_env('CH_RUSTFS_S3_ENDPOINT', 'http://rustfs:9000')}'{auth} "
        "SETTINGS allow_database_iceberg = 1",
    )


def ch_exec(ch: ClickHouseTarget, statement: str) -> str:
    """Duplicated from `bronze_catalog._ch_exec` rather than imported, for
    the same module-privacy reason `maintenance.py` and
    `replication_metrics.py` already carry their own copies."""
    resp = requests.post(
        ch.url,
        auth=(ch.user, ch.password),
        data=statement.encode("utf-8"),
        timeout=300,
    )
    resp.raise_for_status()
    return resp.text.strip()


def register_loaded_table(
    table: str,
    *,
    description: str,
    author: str,
    ch: ClickHouseTarget | None = None,
    ch_exec: Callable[[ClickHouseTarget, str], str] = ch_exec,
    ensure_catalog_database: Callable[[ClickHouseTarget, str], None] = ensure_catalog_database,
    register: Callable[..., None] = register_bronze_table,
) -> int:
    """Register (or refresh) the Bronze table `table` in the catalog and
    return the total row count it was registered with.

    `description` and `author` are the caller's: they say who loaded the
    table. They end up in the shared catalog, so a caller keeps anything it
    would not show every reader of the catalog out of them.

    # Errors

    Raises `ValueError` for a table name that is not a plain identifier, and
    whatever ClickHouse raises if the table cannot be read.
    """
    if not is_plain_table_name(table):
        raise ValueError(f"Bronze target {table!r} is not a plain lower-case identifier; not registering it")
    ch = ch or ch_target()
    ensure_catalog_database(ch, CATALOG_DB)
    bronze = f"{CATALOG_DB}.`bronze.{table}`"

    # `WHERE 1`, never an unqualified row count, for every Bronze Iceberg
    # table whether or not it carries deletes today: measured on ClickHouse
    # 26.3, an unqualified count over a merge-on-read table is answered from
    # metadata and does not subtract equality deletes (R11,
    # docs/plans/P5-RESULT.md; not re-measured on 26.8). The lint
    # `ops/lint/check_bare_iceberg_count.py` cannot see this statement,
    # because its table name arrives through `bronze`, so the rule is kept by
    # hand here.
    total = int(ch_exec(ch, f"SELECT count() FROM {bronze} WHERE 1") or "0")
    columns: list[tuple[str, str, str]] = []
    for line in ch_exec(ch, f"DESCRIBE TABLE {bronze} FORMAT TSV").splitlines():
        name, _, rest = line.partition("\t")
        dtype = rest.split("\t", 1)[0]
        if name and dtype and not name.startswith("_dlt_"):
            columns.append((name, dtype, ""))

    register(
        slug=table.replace("_", "-"),
        title=table.replace("_", " ").title(),
        description=description,
        bronze_table_name=table,
        row_count=total,
        author=author,
        columns=columns,
    )
    return total


def register_connector_table(
    connector_id: str,
    obj: dict,
    *,
    ch: ClickHouseTarget | None = None,
    ch_exec: Callable[[ClickHouseTarget, str], str] = ch_exec,
    ensure_catalog_database: Callable[[ClickHouseTarget, str], None] = ensure_catalog_database,
    register: Callable[..., None] = register_bronze_table,
) -> int:
    """Register (or refresh) `obj["target"]`'s Bronze table in the catalog
    and return the total row count it was registered with.

    # Errors

    Raises `ValueError` for a target that is not a plain identifier, and
    whatever ClickHouse raises if the table cannot be read.
    """
    return register_loaded_table(
        obj["target"],
        description=(
            f"Bronze Iceberg table loaded by connector {connector_id} from its source object {obj['name']}."
        ),
        author=f"connector {connector_id}",
        ch=ch,
        ch_exec=ch_exec,
        ensure_catalog_database=ensure_catalog_database,
        register=register,
    )
