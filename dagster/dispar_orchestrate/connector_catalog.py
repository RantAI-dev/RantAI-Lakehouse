"""Register a connector-ingested Bronze table in the console catalog.

`ingest_job` (`ingest_factory.py`) writes each of a connector's source
objects to a Bronze Iceberg table, but nothing put those tables in
`lake.bronze_meta.dataset_catalog`, the registry `GET /api/catalog` lists
Bronze from. A table a connector loaded therefore existed and could be
queried, yet never showed in the Catalog. File uploads (`file_ingest.py`)
and the demo Bronze job (`assets.py`) already register theirs through
`register_bronze_table`; this is the same call for the connector path.

# What is registered

- `slug`/`title` derived from the Bronze table name, the way
  `file_ingest.py` derives them.
- `row_count`: the table's TOTAL, counted through a ClickHouse
  `DataLakeCatalog` database. Not this run's rows: Bronze is append-only,
  so after a second run the table holds both loads, and the Catalog's row
  total (`dataset_sync.total`) must say so.
- The columns and their types, read the same way (`DESCRIBE`), minus
  dlt's own `_dlt_*` bookkeeping columns.

Registration is an upsert (`ReplacingMergeTree ORDER BY slug`), so every
successful run refreshes the entry.
"""

from __future__ import annotations

import re
from typing import Callable

from dispar_orchestrate import ch_models
from dispar_orchestrate.bronze_catalog import ClickHouseTarget, register_bronze_table

# This job's own `DataLakeCatalog` database, separate from every other
# job's for the reason `silver_transform.py` gives for its own.
CATALOG_DB = "icecat_ingest"

# `sourceObjects[].target` is stored unvalidated server-side (the console
# checks it, the API does not), and it is interpolated into SQL below, so
# anything but a plain lower-case identifier is refused here.
_TARGET = re.compile(r"^[a-z_][a-z0-9_]*$")


def register_connector_table(
    connector_id: str,
    obj: dict,
    *,
    ch: ClickHouseTarget | None = None,
    ch_exec: Callable[[ClickHouseTarget, str], str] = ch_models.ch_exec,
    ensure_catalog_database: Callable[[ClickHouseTarget, str], None] = ch_models.ensure_catalog_database,
    register: Callable[..., None] = register_bronze_table,
) -> int:
    """Register (or refresh) `obj["target"]`'s Bronze table in the catalog
    and return the total row count it was registered with.

    # Errors

    Raises `ValueError` for a target that is not a plain identifier, and
    whatever ClickHouse raises if the table cannot be read.
    """
    target = obj["target"]
    if not _TARGET.match(target):
        raise ValueError(f"Bronze target {target!r} is not a plain lower-case identifier; not registering it")
    ch = ch or ch_models.ch_target()
    ensure_catalog_database(ch, CATALOG_DB)
    table = f"{CATALOG_DB}.`bronze.{target}`"

    # `WHERE 1`, never an unqualified row count, for every Bronze Iceberg
    # table whether or not it carries deletes today: measured on ClickHouse
    # 26.3, an unqualified count over a merge-on-read table is answered from
    # metadata and does not subtract equality deletes (R11,
    # docs/plans/P5-RESULT.md; not re-measured on 26.8). The lint
    # `ops/lint/check_bare_iceberg_count.py` cannot see this statement,
    # because its table name arrives through `table`, so the rule is kept by
    # hand here.
    total = int(ch_exec(ch, f"SELECT count() FROM {table} WHERE 1") or "0")
    columns: list[tuple[str, str, str]] = []
    for line in ch_exec(ch, f"DESCRIBE TABLE {table} FORMAT TSV").splitlines():
        name, _, rest = line.partition("\t")
        dtype = rest.split("\t", 1)[0]
        if name and dtype and not name.startswith("_dlt_"):
            columns.append((name, dtype, ""))

    register(
        slug=target.replace("_", "-"),
        title=target.replace("_", " ").title(),
        description=(
            f"Bronze Iceberg table loaded by connector {connector_id} from its source object {obj['name']}."
        ),
        bronze_table_name=target,
        row_count=total,
        author=f"connector {connector_id}",
        columns=columns,
    )
    return total
