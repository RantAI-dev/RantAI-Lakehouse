"""A tiny SQL-model runner for the ClickHouse-resident layers (Silver, Gold).

Everything above Bronze in this stack is plain `INSERT … SELECT` / `CREATE
… AS SELECT` against ClickHouse — no catalog client, no S3 credentials, no
Iceberg writer. What those transforms actually need from Dagster is only
scheduling, logging and run history, not a bespoke job each.

So a transform here is **data, not code**: one [`Model`] entry names its
target table, its engine, and the `SELECT` that fills it. One job runs a
whole list of them in order. Adding a mart is adding an entry — not
copying a 180-line module and changing twelve lines of it, which is what
`silver_transform.py` would have you do a second time.

This is deliberately the same shape a model-definition TABLE would have
(see the `pipeline_definition` gap discussed in the console's Pipelines
page): when the definitions move from this Python list into Postgres so
users can author them from the UI, only the SOURCE of the list changes —
[`run_models`] stays exactly as it is.

# Why full refresh, not incremental

Every model here rebuilds its target from scratch with `CREATE OR REPLACE
TABLE … AS SELECT`. For an aggregate mart over a few million Silver rows
that costs seconds, and it is *correct by construction*: an incrementally
maintained aggregate has to reason about late-arriving rows, restatements
and the `ReplacingMergeTree` versions its source still carries
un-collapsed. Incremental loading belongs one layer down, where
`silver_transform.py` does it on a real `_ingested_at` watermark.

`CREATE OR REPLACE` is atomic on ClickHouse's default `Atomic` database
engine: readers see either the old table or the new one, never a
half-filled one.
"""

from __future__ import annotations

import os
from dataclasses import dataclass

import requests

from dispar_orchestrate.bronze_catalog import ClickHouseTarget

# Schemas a model may write into. A guard, not a formality: these jobs
# execute SQL that is assembled from a model's own strings, and the day
# those strings come from a user-facing form (rather than this file) the
# blast radius must already be bounded to the two layers models are
# allowed to own. `console`, `lake` and `system` hold the product's own
# state and the Bronze registry — never a transform's output.
ALLOWED_SCHEMAS = frozenset({"silver", "serving"})


@dataclass(frozen=True)
class Model:
    """One SQL model: a target table and the `SELECT` that fills it."""

    #: Short name for logs and run metadata, e.g. `gold_orders_daily`.
    name: str
    #: Fully qualified target, `<schema>.<table>`; `<schema>` must be in
    #: [`ALLOWED_SCHEMAS`], and Gold marts belong in `serving` under a
    #: `mart_` prefix — that prefix is what the copilot's schema context
    #: (`routes::agent::schema_context`) and the dashboard field picker
    #: look for.
    target: str
    #: ClickHouse engine clause, e.g. `MergeTree ORDER BY order_date`.
    engine: str
    #: The `SELECT` that produces the table's contents. Reads a Silver
    #: table with `FINAL` when that table is a `ReplacingMergeTree`, or the
    #: aggregate counts rows a merge has not collapsed yet.
    select: str


def _env(name: str, default: str) -> str:
    value = os.environ.get(name, "").strip()
    return value or default


def ch_target() -> ClickHouseTarget:
    return ClickHouseTarget.from_env()


def ensure_catalog_database(ch: ClickHouseTarget, db: str) -> None:
    """Create a `DataLakeCatalog` database so ClickHouse can read Bronze
    Iceberg tables through Lakekeeper. Needed by any model whose source is
    Bronze; a model reading only `silver.*` does not call this.

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


def _split_target(target: str) -> tuple[str, str]:
    schema, _, table = target.partition(".")
    if not schema or not table or "." in table:
        raise ValueError(f"target must be '<schema>.<table>', got {target!r}")
    if schema not in ALLOWED_SCHEMAS:
        raise ValueError(
            f"model target schema {schema!r} is not allowed; "
            f"expected one of {sorted(ALLOWED_SCHEMAS)}"
        )
    return schema, table


def run_model(ch: ClickHouseTarget, model: Model) -> int:
    """Rebuild one model's target table and return its row count."""
    schema, table = _split_target(model.target)
    ch_exec(ch, f"CREATE DATABASE IF NOT EXISTS {schema}")
    ch_exec(
        ch,
        f"CREATE OR REPLACE TABLE {schema}.`{table}` "
        f"ENGINE = {model.engine} AS {model.select}",
    )
    return int(ch_exec(ch, f"SELECT count() FROM {schema}.`{table}`"))


def run_models(models: list[Model], log=None) -> dict[str, int]:
    """Run every model in order, returning `{model name: row count}`.

    Order matters and is the list's own: a Gold model reading a Silver
    table that an earlier entry rebuilt must come after it. There is no
    dependency inference here on purpose — a list you can read top to
    bottom is easier to reason about than an inferred graph, at this size.
    """
    ch = ch_target()
    counts: dict[str, int] = {}
    for model in models:
        rows = run_model(ch, model)
        counts[model.name] = rows
        if log is not None:
            log.info(f"{model.name}: {model.target} rebuilt with {rows} rows")
    return counts
