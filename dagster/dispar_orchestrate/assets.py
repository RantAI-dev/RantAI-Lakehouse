"""P3 Dagster job: dlt `sql_database` -> Bronze Iceberg (through Lakekeeper)
-> console catalog registration.

One op, not several `@asset`s wired by inference: the three steps
(ingest, count, register) share one `BronzeIngestConfig` and are not
independently useful as separate materializations for G3a's scope. A
future P4 (maintenance jobs) or P5 (CDC) adds genuinely independent assets
under this same package (see ADR 0005) — this job is not meant to be the
final shape of `dispar_orchestrate`, only the first one.
"""

from __future__ import annotations

import psycopg2
from dagster import Failure, job, op

from dispar_orchestrate.bronze_catalog import register_bronze_table
from dispar_orchestrate.dlt_pipeline import BronzeIngestConfig, run_bronze_ingest
from dispar_orchestrate.op_metadata import DEFAULT_RETRY_POLICY, source_metadata


@op(
    retry_policy=DEFAULT_RETRY_POLICY,
    tags=source_metadata(
        "dispar_orchestrate/assets.py::ingest_bronze_table",
        reads=["Postgres BRONZE_SOURCE_SCHEMA.TABLE (via dlt)"],
        writes=["Iceberg bronze.{BRONZE_TABLE_NAME}"],
    ),
)
def ingest_bronze_table(context) -> dict:
    """Run the dlt pipeline: Postgres -> Bronze Iceberg through Lakekeeper.

    PART D wrap: `dlt_pipeline.BronzeIngestConfig.from_dial`
    (`dlt_pipeline.py:198,200`) raises `ValueError` when the dial's
    `driver` is not `postgres`/`postgresql` or when the connector's
    `source_objects` list is empty -- a config problem. Wrap it at this
    op's boundary so the `DEFAULT_RETRY_POLICY` doesn't burn 60s on a
    config that will not change between attempts. `RuntimeError` from
    `dlt_pipeline.run_bronze_ingest` (`dlt_pipeline.py:300`, failed dlt
    load jobs) is left retryable -- that is a transient / load-side
    failure, not a config rejection."""
    try:
        summary = run_bronze_ingest()
    except ValueError as exc:
        raise Failure(
            description=f"bronze ingest config rejected: {exc}",
            allow_retries=False,
        ) from exc
    context.log.info(f"dlt load complete: {summary}")
    context.add_output_metadata(
        {
            "bronze_table_name": summary["bronze_table_name"],
            "source_table": f"{summary['source_schema']}.{summary['source_table']}",
        }
    )
    return summary


@op(
    retry_policy=DEFAULT_RETRY_POLICY,
    tags=source_metadata(
        "dispar_orchestrate/assets.py::register_in_catalog",
        reads=["Postgres source table (row count)", "ClickHouse system.tables, system.columns"],
        writes=["ClickHouse lake.bronze_meta.dataset_catalog", "ClickHouse lake.bronze_meta.dataset_sync"],
    ),
)
def register_in_catalog(context, summary: dict) -> None:
    """Make the ingested table show up on `GET /api/catalog` and the
    `governance/lineage`/`governance/classification` surfaces, by writing
    the same registry rows those routes already read
    (`lakehouse-api::routes::catalog`) — see `bronze_catalog`'s module doc.
    """
    cfg = BronzeIngestConfig.from_env()
    row_count = _count_source_rows(cfg)
    slug = summary["bronze_table_name"].replace("_", "-")
    register_bronze_table(
        slug=slug,
        title=summary["bronze_table_name"].replace("_", " ").title(),
        description=(
            f"Bronze Iceberg table ingested via dlt sql_database from "
            f"Postgres {summary['source_schema']}.{summary['source_table']}, "
            f"through Lakekeeper (G3a)."
        ),
        bronze_table_name=summary["bronze_table_name"],
        row_count=row_count,
        author="dagster",
    )
    context.log.info(f"registered '{slug}' in lake.bronze_meta.dataset_catalog ({row_count} rows)")


def _count_source_rows(cfg: BronzeIngestConfig) -> int:
    # Discrete kwargs, not a DSN string — see `BronzeIngestConfig`'s field
    # comment in `dlt_pipeline.py` on why this pipeline stopped assembling
    # a single `postgresql://user:password@host/db` connection string
    # (that string used to travel through docker-compose.yml as one
    # plaintext env var; the credential-hygiene fix splits it into
    # components and only ever joins them back together in-process, here
    # and in `run_bronze_ingest`).
    with psycopg2.connect(
        host=cfg.source_db_host,
        port=cfg.source_db_port,
        user=cfg.source_db_user,
        password=cfg.source_db_password,
        dbname=cfg.source_db_name,
    ) as conn:
        with conn.cursor() as cur:
            cur.execute(f'SELECT count(*) FROM "{cfg.source_schema}"."{cfg.source_table}"')
            row = cur.fetchone()
            return int(row[0]) if row else 0


@job
def bronze_ingest_job() -> None:
    """`DAGSTER_LOCATION`-visible job name: `bronze_ingest_job`. Launched
    exactly the way `lakehouse-dagster::DgClient::launch_run` launches any
    other job — no special-casing needed on the Rust side for this job to
    light up `POST /api/pipelines/{id}/run` once `DAGSTER_URL` points at a
    live webserver (see `docs/adr/0005-...md`)."""
    summary = ingest_bronze_table()
    register_in_catalog(summary)
