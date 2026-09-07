"""dlt `sql_database` -> Bronze Iceberg **through Lakekeeper**.

G3a (`docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md` §3): dlt reads a real
Postgres table and writes it to Bronze Iceberg registered in Lakekeeper's
REST catalog — never a path-based S3 destination, never dlt's own
ephemeral/local-SQLite Iceberg catalog fallback.

# Why this is provably NOT the local-SQLite fallback

dlt's `filesystem` destination's `table_format="iceberg"` defaults to an
in-memory/SQLite "technical catalog" (`iceberg_catalog_type` defaults to
`"sql"`, resolving to a local SQLite file dlt manages itself) when no
catalog config is supplied. That fallback would be exactly the
"just Parquet on disk, invisible to Lakekeeper/ClickHouse" shape the task
brief forbids. This module sets `iceberg_catalog_type = "rest"` and a
`iceberg_catalog_config` dict pointed at Lakekeeper's REST endpoint
explicitly (via `_catalog_env`, below) — confirmed empirically (not just
by reading dlt's docs) against a live Lakekeeper + RustFS stack during
P3 verification: the resulting table is registered under Lakekeeper's
`/catalog/v1/{warehouse}/namespaces/bronze/tables/...` REST surface and
is independently readable by ClickHouse's `DataLakeCatalog` engine and by
a bare `pyiceberg.catalog.rest.RestCatalog` client that never touches
this dlt pipeline's own working directory.

# Networking constraint (matches `g1_lakekeeper.rs`'s module doc exactly)

This must run **inside the compose network** — Lakekeeper's `/v1/config`
response includes the server's own canonical catalog URI
(`LAKEKEEPER__BASE_URI`), which pyiceberg's `RestCatalog` honors and uses
for every subsequent call. If `LAKEKEEPER__BASE_URI` is a host-facing
address (`http://localhost:8181`, `docker-compose.yml`'s default), a
catalog client running outside the compose network hangs/times out
resolving that address's Docker-internal counterpart, or vice versa. The
`g3a-test-runner` compose service (see `docker-compose.yml`) — like
`g1-test-runner` before it — runs this pipeline as a container attached
to the compose network, with `LAKEKEEPER_BASE_URI` set to the
compose-internal `http://lakekeeper:8181` (the same fix
`.github/workflows/ci.yml`'s G1/G2 jobs already apply for the identical
reason).

# Partitioning (ADR 0004 parity, not byte-identical)

Every row gets a `_ingested_at` timestamp stamped by this pipeline (not a
source column), and the created table is partitioned `day(_ingested_at)`
via dlt's `iceberg_adapter`/`iceberg_partition.day` — the same default
ADR 0004 established for `lakehouse-iceberg`'s Rust-side
`create_bronze_table`. The Iceberg field-id numbering dlt assigns differs
from `bronze::INGESTED_AT_FIELD_ID`'s reserved scheme (that scheme is
specific to `lakehouse-iceberg`'s own schema builder) — this is a
different table-creation code path producing the same partitioning
*behavior*, not the same on-disk field-id layout.
"""

from __future__ import annotations

import json
import os
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any

import dlt
from dlt.destinations import filesystem
from dlt.destinations.adapters import iceberg_adapter, iceberg_partition
from dlt.sources.sql_database import sql_database


def _env(name: str, default: str) -> str:
    value = os.environ.get(name, "").strip()
    return value if value else default


@dataclass(frozen=True)
class BronzeIngestConfig:
    """Everything one dlt Bronze-ingest run needs, sourced from the
    container's environment (matching how every other service in
    `docker-compose.yml` is configured — no baked-in secrets.toml)."""

    source_database_url: str
    source_schema: str
    source_table: str
    bronze_table_name: str
    lakekeeper_catalog_uri: str
    lakekeeper_warehouse: str
    rustfs_endpoint: str
    rustfs_access_key: str
    rustfs_secret_key: str
    warehouse_bucket: str
    # R1 (ADR 0011): a pre-minted static bearer token for the `dlt`
    # principal (granted create/modify/select on the warehouse), read from
    # a file rather than an env var — this dataclass is constructed once
    # at process start inside a long-running `dagster api grpc` server, so
    # (unlike a one-shot compose job) there is no shell step between
    # container start and this code to interpolate a file's contents into
    # an env var. Empty string on a pre-R1 or authz-disabled stack, where
    # `/tokens/dlt.jwt` is not mounted.
    lakekeeper_token: str

    @classmethod
    def from_env(cls) -> "BronzeIngestConfig":
        source_table = _env("BRONZE_SOURCE_TABLE", "orders")
        token_file = _env("LAKEKEEPER_TOKEN_FILE", "")
        lakekeeper_token = ""
        if token_file and os.path.exists(token_file):
            with open(token_file, encoding="utf-8") as f:
                lakekeeper_token = f.read().strip()
        return cls(
            # `postgresql://`, not `postgres://` — this goes through
            # SQLAlchemy (dlt's `sql_database` source), which does not
            # recognize the `postgres://` scheme `DATABASE_URL` elsewhere
            # in this repo uses for `sqlx`.
            source_database_url=_env(
                "BRONZE_SOURCE_DATABASE_URL",
                "postgresql://lakehouse:lakehouse@postgres:5432/lakehouse",
            ),
            source_schema=_env("BRONZE_SOURCE_SCHEMA", "ingest_demo"),
            source_table=source_table,
            bronze_table_name=_env("BRONZE_TABLE_NAME", source_table),
            lakekeeper_catalog_uri=_env(
                "LAKEKEEPER_CATALOG_URI", "http://lakekeeper:8181/catalog"
            ),
            lakekeeper_warehouse=_env("LAKEKEEPER_WAREHOUSE", "default"),
            rustfs_endpoint=_env("CH_RUSTFS_S3_ENDPOINT", "http://rustfs:9000"),
            rustfs_access_key=_env("RUSTFS_ACCESS_KEY", "rustfsadmin"),
            rustfs_secret_key=_env("RUSTFS_SECRET_KEY", "rustfsadmin"),
            warehouse_bucket=_env("LAKEHOUSE_WAREHOUSE_BUCKET", "lakehouse-warehouse"),
            lakekeeper_token=lakekeeper_token,
        )


def _install_catalog_env(config: BronzeIngestConfig) -> None:
    """Point dlt's Iceberg `table_format` at Lakekeeper's REST catalog by
    setting the two env vars `dlt.common.libs.pyiceberg.IcebergConfig`
    resolves (`sections="iceberg_catalog"`).

    `iceberg_catalog_config` is a `Dict[str, Any]`-typed field — dlt's env
    provider does not do double-underscore key-splitting for a plain dict
    field the way it does for nested dataclasses, so the whole dict must
    be supplied as one JSON-encoded env var
    (`ICEBERG_CATALOG__ICEBERG_CATALOG_CONFIG`). Verified empirically
    against a live Lakekeeper: individual
    `ICEBERG_CATALOG__ICEBERG_CATALOG_CONFIG__URI`-style keys are silently
    ignored (dlt falls back to the ephemeral local catalog with no error),
    while the single JSON-blob env var is picked up correctly and produces
    a REST-catalog-registered table.

    `s3.force-virtual-addressing = "false"` matters on RustFS/SeaweedFS:
    without it, pyiceberg's PyArrow-backed `FileIO` tries virtual-hosted
    addressing (`<bucket>.<endpoint-host>`), which fails DNS resolution
    against a plain path-style S3-compatible endpoint like RustFS/SeaweedFS
    — this is the same path-style requirement
    `docker-compose.yml`'s `lakekeeper-warehouse-init` already sets for
    Lakekeeper's own storage profile (`"path-style-access": true`), applied
    here on dlt's side of the same S3 endpoint.
    """
    os.environ["ICEBERG_CATALOG__ICEBERG_CATALOG_NAME"] = "default"
    os.environ["ICEBERG_CATALOG__ICEBERG_CATALOG_TYPE"] = "rest"
    catalog_config: dict[str, str] = {
        "type": "rest",
        "uri": config.lakekeeper_catalog_uri,
        "warehouse": config.lakekeeper_warehouse,
        "s3.endpoint": config.rustfs_endpoint,
        "s3.access-key-id": config.rustfs_access_key,
        "s3.secret-access-key": config.rustfs_secret_key,
        "s3.region": "us-east-1",
        "s3.path-style-access": "true",
        "s3.force-virtual-addressing": "false",
    }
    if config.lakekeeper_token:
        # R1 (ADR 0011): pyiceberg's `RestCatalog` accepts a raw static
        # bearer `token` the same way `iceberg-catalog-rest` (Rust) does —
        # sent as-is on every request, no OAuth2 exchange. `None`/absent
        # on a pre-R1 or authz-disabled stack.
        catalog_config["token"] = config.lakekeeper_token
    os.environ["ICEBERG_CATALOG__ICEBERG_CATALOG_CONFIG"] = json.dumps(catalog_config)


def _stamp_ingested_at(record: dict[str, Any]) -> dict[str, Any]:
    """Bronze's system ingestion-time column (ADR 0004's
    `bronze::INGESTED_AT_COLUMN` equivalent for the dlt write path) —
    stamped here, not read from the source, so partitioning never depends
    on a source-provided timestamp existing/being non-null."""
    record["_ingested_at"] = datetime.now(timezone.utc)
    return record


def run_bronze_ingest(config: BronzeIngestConfig | None = None) -> dict[str, Any]:
    """Run the dlt pipeline once. Returns a small summary dict (row/table
    names, load id) for the caller (the Dagster asset) to attach as
    metadata and pass on to Bronze catalog registration.

    # Errors

    Raises whatever `pipeline.run` raises (`PipelineStepFailed`, etc.) —
    this is a thin, unit-testable wrapper around dlt's own run, not a
    place that swallows load failures.
    """
    cfg = config or BronzeIngestConfig.from_env()
    _install_catalog_env(cfg)

    destination = filesystem(
        bucket_url=f"s3://{cfg.warehouse_bucket}/bronze",
        credentials={
            "aws_access_key_id": cfg.rustfs_access_key,
            "aws_secret_access_key": cfg.rustfs_secret_key,
            "endpoint_url": cfg.rustfs_endpoint,
        },
    )

    source = sql_database(
        credentials=cfg.source_database_url,
        schema=cfg.source_schema,
        table_names=[cfg.source_table],
    )
    resource = source.resources[cfg.source_table]
    resource.apply_hints(table_name=cfg.bronze_table_name)
    resource.add_map(_stamp_ingested_at)
    iceberg_adapter(
        resource,
        partition=[iceberg_partition.day("_ingested_at")],
        # PR #29 review: format-version 2 was claimed "confirmed" without
        # ever being set or asserted. dlt's iceberg destination does not
        # default this itself (pyiceberg's own table-creation default is
        # already v2, but that is pyiceberg's default, not a guarantee this
        # pipeline makes) — set it explicitly, at table-creation time, so
        # the guarantee is this module's own rather than inherited
        # incidentally from whatever pyiceberg happens to default to.
        # `ops/g3a/g3a_test.py::step_verify_format_version_2` asserts this
        # against the catalog's own REST metadata, not against what dlt
        # reports back.
        table_properties={"format-version": "2"},
    )

    pipeline = dlt.pipeline(
        pipeline_name=f"bronze_ingest_{cfg.bronze_table_name}",
        destination=destination,
        # Flat `bronze` dataset (ADR 0004's flat `bronze` namespace) — every
        # Bronze table this pipeline ever writes lands in the same
        # Lakekeeper namespace `lakehouse-iceberg`'s Rust write path uses.
        dataset_name="bronze",
    )
    load_info = pipeline.run(source, table_format="iceberg")
    if load_info.has_failed_jobs:
        raise RuntimeError(f"dlt load had failed jobs: {load_info.load_packages}")

    return {
        "bronze_table_name": cfg.bronze_table_name,
        "source_schema": cfg.source_schema,
        "source_table": cfg.source_table,
        "load_id": load_info.loads_ids[-1] if load_info.loads_ids else None,
    }


if __name__ == "__main__":
    # `python -m dispar_orchestrate.dlt_pipeline` — a standalone run for
    # local debugging, bypassing Dagster entirely. Not what G3a's
    # acceptance test exercises (that goes through the Dagster GraphQL
    # `launchRun` path, matching `lakehouse-dagster::DgClient`), but useful
    # for isolating a dlt/pyiceberg-versus-Dagster problem quickly.
    print(run_bronze_ingest())
