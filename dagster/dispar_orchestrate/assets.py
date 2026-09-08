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
from dagster import job, op

from dispar_orchestrate.bronze_catalog import register_bronze_table
from dispar_orchestrate.dlt_pipeline import BronzeIngestConfig, run_bronze_ingest


@op
def ingest_bronze_table(context) -> dict:
    """Run the dlt pipeline: Postgres -> Bronze Iceberg through Lakekeeper."""
    summary = run_bronze_ingest()
    context.log.info(f"dlt load complete: {summary}")
    context.add_output_metadata(
        {
            "bronze_table_name": summary["bronze_table_name"],
            "source_table": f"{summary['source_schema']}.{summary['source_table']}",
        }
    )
    return summary


@op
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
