"""Silver transform: Bronze Iceberg (through Lakekeeper) -> ClickHouse `silver.*`.

Bronze is append-only and faithful to the source (ADR 0004): a second
`bronze_ingest_job` run appends the same rows again rather than replacing
them, so any aggregate read straight off Bronze double-counts. This job is
where that is corrected — the deduplication, typing and naming pass the
medallion architecture assigns to Silver.

# Why ClickHouse `MergeTree` and not another Iceberg table

ClickHouse cannot `CREATE TABLE` through the Iceberg catalog on the pinned
26.8 (see the README's "Status / Known limitations" and
`docs/plans/G1-RESULT.md`), and the one working Iceberg write path in this
repo goes through Rust (`lakehouse-iceberg`, ADR 0010). A Silver layer
written as Iceberg would therefore need a new caller of that crate, not a
SQL job. Silver lives in ClickHouse `MergeTree` instead, which is also
where every console read path already looks for it
(`routes::catalog` reads Silver/Gold straight off `system.tables`).

# Why the target table name matters

`routes::ai::tools::data::describe_dataset` counts a dataset's rows from
`silver.<table_name>`, where `<table_name>` is what
`lake.bronze_meta.dataset_catalog` recorded at ingest time. Naming the
target after that column (not after the source table) is what makes the
copilot, the asset detail page and this job agree on one number.
"""

from __future__ import annotations

import os
from dataclasses import dataclass

import requests
from dagster import job, op

from dispar_orchestrate.bronze_catalog import ClickHouseTarget

# A dedicated `DataLakeCatalog` database for this job, separate from
# `icecat_maintenance` for the same reason that one is separate from the
# per-test catalogs: a transform must never depend on another job's
# fixtures having created its database first.
CATALOG_DB = "icecat_silver"


def _env(name: str, default: str) -> str:
    value = os.environ.get(name, "").strip()
    return value or default


@dataclass(frozen=True)
class SilverConfig:
    ch: ClickHouseTarget
    catalog_uri: str
    warehouse: str
    rustfs_endpoint: str
    oauth_client_id: str
    oauth_server_uri: str
    bronze_table: str
    silver_table: str

    @classmethod
    def from_env(cls) -> "SilverConfig":
        bronze_table = _env("BRONZE_TABLE_NAME", "g3a_orders")
        return cls(
            ch=ClickHouseTarget.from_env(),
            catalog_uri=_env("LAKEKEEPER_CATALOG_URI", "http://lakekeeper:8181/catalog"),
            warehouse=_env("LAKEKEEPER_WAREHOUSE", "default"),
            rustfs_endpoint=_env("CH_RUSTFS_S3_ENDPOINT", "http://rustfs:9000"),
            oauth_client_id=_env("CH_OAUTH_CLIENT_ID", ""),
            oauth_server_uri=_env("CH_OAUTH_SERVER_URI", ""),
            bronze_table=bronze_table,
            # Defaults to the Bronze table's own name so `silver.<table>`
            # matches `dataset_catalog.table_name` — see the module doc.
            silver_table=_env("SILVER_TABLE_NAME", bronze_table),
        )

    def ch_auth_settings(self) -> str:
        """R1 (ADR 0011): with Lakekeeper authorization enabled, ClickHouse
        reads the catalog as the granted `clickhouse-reader` principal.
        Empty on an authz-disabled stack, matching `maintenance.py`."""
        if not self.oauth_client_id:
            return ""
        return (
            f", catalog_credential = '{self.oauth_client_id}:unused', "
            f"oauth_server_uri = '{self.oauth_server_uri}'"
        )


def _ch_exec(cfg: SilverConfig, statement: str) -> str:
    """Duplicated from `bronze_catalog._ch_exec` rather than imported, for
    the same module-privacy reason `maintenance.py` and
    `replication_metrics.py` already carry their own copies."""
    resp = requests.post(
        cfg.ch.url,
        auth=(cfg.ch.user, cfg.ch.password),
        data=statement.encode("utf-8"),
        timeout=120,
    )
    resp.raise_for_status()
    return resp.text.strip()


def _ensure_catalog_database(cfg: SilverConfig) -> None:
    _ch_exec(
        cfg,
        f"CREATE DATABASE IF NOT EXISTS {CATALOG_DB} "
        f"ENGINE = DataLakeCatalog('{cfg.catalog_uri}') "
        f"SETTINGS catalog_type = 'rest', warehouse = '{cfg.warehouse}', "
        f"storage_endpoint = '{cfg.rustfs_endpoint}'{cfg.ch_auth_settings()} "
        "SETTINGS allow_database_iceberg = 1",
    )


def _ensure_silver_table(cfg: SilverConfig) -> None:
    _ch_exec(cfg, "CREATE DATABASE IF NOT EXISTS silver")
    # `ReplacingMergeTree(_ingested_at) ORDER BY id`: re-running this job is
    # idempotent by construction — a row that arrives again keeps only its
    # newest `_ingested_at` version, which is exactly the Bronze
    # double-append this layer exists to collapse. Readers must use `FINAL`
    # (merges are not scheduled), the same discipline the `bronze_meta*`
    # registry tables need.
    _ch_exec(
        cfg,
        f"CREATE TABLE IF NOT EXISTS silver.`{cfg.silver_table}` ("
        "  id Int64,"
        "  customer String,"
        "  amount Decimal(12, 2),"
        "  created_at DateTime64(3, 'UTC'),"
        "  _ingested_at DateTime64(3, 'UTC')"
        ") ENGINE = ReplacingMergeTree(_ingested_at) ORDER BY id",
    )


def _load_increment(cfg: SilverConfig) -> None:
    """Only rows newer than what Silver already holds, watermarked on
    `_ingested_at` (stamped by the dlt pipeline per load, NOT a source
    column — `created_at` is identical across loads and would let a whole
    re-ingest slip through)."""
    _ch_exec(
        cfg,
        f"INSERT INTO silver.`{cfg.silver_table}` "
        "SELECT toInt64(id) AS id,"
        "       trim(toString(customer)) AS customer,"
        "       toDecimal64(amount, 2) AS amount,"
        "       toDateTime64(created_at, 3, 'UTC') AS created_at,"
        "       toDateTime64(_ingested_at, 3, 'UTC') AS _ingested_at "
        f"FROM {CATALOG_DB}.`bronze.{cfg.bronze_table}` "
        "WHERE toDateTime64(_ingested_at, 3, 'UTC') > ("
        "  SELECT coalesce(max(_ingested_at), toDateTime64('1970-01-01 00:00:00', 3, 'UTC'))"
        f"  FROM silver.`{cfg.silver_table}`)",
    )


@op
def transform_bronze_to_silver(context) -> dict:
    cfg = SilverConfig.from_env()
    _ensure_catalog_database(cfg)
    _ensure_silver_table(cfg)

    bronze_rows = int(
        _ch_exec(cfg, f"SELECT count() FROM {CATALOG_DB}.`bronze.{cfg.bronze_table}`")
    )
    _load_increment(cfg)
    # `FINAL`: without it this counts rows that a merge has not yet
    # collapsed, i.e. the very duplicates this job just removed.
    silver_rows = int(
        _ch_exec(cfg, f"SELECT count() FROM silver.`{cfg.silver_table}` FINAL")
    )

    context.log.info(
        f"bronze.{cfg.bronze_table}: {bronze_rows} rows -> "
        f"silver.{cfg.silver_table}: {silver_rows} rows (deduplicated)"
    )
    context.add_output_metadata(
        {
            "bronze_table": f"bronze.{cfg.bronze_table}",
            "silver_table": f"silver.{cfg.silver_table}",
            "bronze_rows": bronze_rows,
            "silver_rows": silver_rows,
        }
    )
    return {"silver_table": cfg.silver_table, "silver_rows": silver_rows}


@job
def silver_transform_job() -> None:
    """`DAGSTER_LOCATION`-visible job name: `silver_transform_job`. Launched
    the same way `bronze_ingest_job` is — no Rust-side special-casing."""
    transform_bronze_to_silver()
