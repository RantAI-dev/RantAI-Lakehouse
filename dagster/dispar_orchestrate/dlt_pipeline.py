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

import os
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any

from dlt.sources.sql_database import sql_database

from dispar_orchestrate.adapters.sink import SinkConfig, load_via_sink


def _env(name: str, default: str) -> str:
    value = os.environ.get(name, "").strip()
    return value if value else default


@dataclass(frozen=True)
class BronzeIngestConfig:
    """Everything one dlt Bronze-ingest run needs, sourced from the
    container's environment (matching how every other service in
    `docker-compose.yml` is configured — no baked-in secrets.toml)."""

    # CREDENTIAL HYGIENE (review finding): these five replace what used to
    # be a single `source_database_url` field built from
    # `BRONZE_SOURCE_DATABASE_URL` — a full `postgresql://user:password@
    # host/db` DSN passed through docker-compose.yml as ONE env var, so
    # the password was visible whole to `docker inspect` and to anything
    # that can read this container's environment. The DSN itself is now
    # only ever assembled in-process (see `run_bronze_ingest`, below),
    # from components that already existed elsewhere in the compose file
    # (`POSTGRES_USER`/`POSTGRES_DB`) plus the connector-dedicated
    # password (`BRONZE_SOURCE_DB_PASSWORD`, which docker-compose.yml
    # defaults from `CONNECTOR_PG_PASSWORD` — the same least-privilege
    # secret name `lakehouse-api`'s own connector definitions use, not
    # the console's own `POSTGRES_PASSWORD`).
    source_db_host: str
    source_db_port: str
    source_db_user: str
    source_db_password: str
    source_db_name: str
    source_schema: str
    source_table: str
    bronze_table_name: str
    lakekeeper_catalog_uri: str
    lakekeeper_warehouse: str
    rustfs_endpoint: str
    # CREDENTIAL HYGIENE (review finding): these used to read
    # `RUSTFS_ACCESS_KEY`/`RUSTFS_SECRET_KEY` directly — RustFS's own ROOT
    # credentials, the same ones the bucket-bootstrap/admin-shaped jobs in
    # docker-compose.yml use. dlt only ever PUTs/GETs objects under the
    # warehouse bucket's `bronze/` prefix, so it does not need root. Read
    # the connector-scoped names instead (`CONNECTOR_S3_ACCESS_KEY`/
    # `CONNECTOR_S3_SECRET_KEY` — see docker-compose.yml's
    # `dagster-code-location` env block for why these still default to
    # the RustFS root key today: this stack's RustFS version has no
    # non-proprietary-admin-API way to mint a narrower S3 identity, and
    # this project's locked decision is "plain S3 API only, never a
    # store's proprietary admin call" — see the `rustfs` service's own
    # comment in docker-compose.yml). A deployment that CAN provision a
    # least-privilege RustFS/S3 identity only needs to point these two
    # vars at it; no code change here.
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
            source_db_host=_env("BRONZE_SOURCE_DB_HOST", "postgres"),
            source_db_port=_env("BRONZE_SOURCE_DB_PORT", "5432"),
            source_db_user=_env("BRONZE_SOURCE_DB_USER", "lakehouse"),
            source_db_password=_env("BRONZE_SOURCE_DB_PASSWORD", "lakehouse"),
            source_db_name=_env("BRONZE_SOURCE_DB_NAME", "lakehouse"),
            source_schema=_env("BRONZE_SOURCE_SCHEMA", "ingest_demo"),
            source_table=source_table,
            bronze_table_name=_env("BRONZE_TABLE_NAME", source_table),
            lakekeeper_catalog_uri=_env(
                "LAKEKEEPER_CATALOG_URI", "http://lakekeeper:8181/catalog"
            ),
            lakekeeper_warehouse=_env("LAKEKEEPER_WAREHOUSE", "default"),
            rustfs_endpoint=_env("CH_RUSTFS_S3_ENDPOINT", "http://rustfs:9000"),
            rustfs_access_key=_env("CONNECTOR_S3_ACCESS_KEY", "rustfsadmin"),
            rustfs_secret_key=_env("CONNECTOR_S3_SECRET_KEY", "rustfsadmin"),
            warehouse_bucket=_env("LAKEHOUSE_WAREHOUSE_BUCKET", "lakehouse-warehouse"),
            lakekeeper_token=lakekeeper_token,
        )


def _stamp_ingested_at(record: dict[str, Any]) -> dict[str, Any]:
    """Bronze's system ingestion-time column (ADR 0004's
    `bronze::INGESTED_AT_COLUMN` equivalent for the dlt write path) —
    stamped here, not read from the source, so partitioning never depends
    on a source-provided timestamp existing/being non-null."""
    record["_ingested_at"] = datetime.now(timezone.utc)
    return record


def run_bronze_ingest(config: BronzeIngestConfig | None = None) -> dict[str, Any]:
    """Run the dlt pipeline once: build the Postgres `sql_database` source
    (unchanged from before this refactor), then write it through the
    shared sink (`adapters/sink.py`, extracted per
    `docs/superpowers/plans/2026-09-11-ws3-ingestion-tier1.md`) every other ingest
    adapter (SQL/files/REST) writes through too. Returns a small summary
    dict (row/table names, measured row count) for the caller (the
    Dagster op in `assets.py`) to attach as metadata and pass on to Bronze
    catalog registration.

    # Errors

    Raises `RuntimeError` if the dlt load had failed jobs — this stays a
    thin, unit-testable wrapper, not a place that swallows load failures.
    """
    cfg = config or BronzeIngestConfig.from_env()

    # Assembled here, in-process, from discrete components (`from_env`,
    # above) — this dict is the ONE place a full Postgres DSN exists for
    # this pipeline; it is never passed through docker-compose.yml (or any
    # other environment) as a single connection-string value. dlt's
    # `sql_database` source accepts this shape directly (a
    # `ConnectionStringCredentials`-compatible mapping), same as it would
    # accept a raw `postgresql://...` string — SQLAlchemy builds the DSN
    # from these fields internally either way.
    source_credentials = {
        "drivername": "postgresql",
        "username": cfg.source_db_user,
        "password": cfg.source_db_password,
        "host": cfg.source_db_host,
        "port": int(cfg.source_db_port),
        "database": cfg.source_db_name,
    }
    source = sql_database(
        credentials=source_credentials,
        schema=cfg.source_schema,
        table_names=[cfg.source_table],
    )
    resource = source.resources[cfg.source_table]
    resource.apply_hints(table_name=cfg.bronze_table_name)
    resource.add_map(_stamp_ingested_at)

    result = load_via_sink(source, cfg.bronze_table_name, SinkConfig.from_bronze_ingest_config(cfg))
    if result.has_failed_jobs:
        # `load_info_str` (`str(load_info)`) can contain a file path --
        # fine in a raised exception that stays server-side (this op is
        # not a route handler), but see `adapters/sink.py`'s module doc
        # for why it must never reach a caller/response elsewhere.
        raise RuntimeError(f"dlt load had failed jobs: {result.load_info_str}")

    return {
        "bronze_table_name": cfg.bronze_table_name,
        "source_schema": cfg.source_schema,
        "source_table": cfg.source_table,
        # WS3 plan review Z9: the real, measured row count from dlt's own
        # normalize trace (`adapters/sink.py`'s module doc) -- `None`,
        # never a fabricated `0`, when unmeasured.
        "rows": result.rows,
    }


if __name__ == "__main__":
    # `python -m dispar_orchestrate.dlt_pipeline` — a standalone run for
    # local debugging, bypassing Dagster entirely. Not what G3a's
    # acceptance test exercises (that goes through the Dagster GraphQL
    # `launchRun` path, matching `lakehouse-dagster::DgClient`), but useful
    # for isolating a dlt/pyiceberg-versus-Dagster problem quickly.
    print(run_bronze_ingest())
