"""ADR 0010 — Gold export to Iceberg, scheduled from Dagster.

This module is deliberately thin: it does not touch `ClickHouse` or
Lakekeeper itself. The export mechanics (`ClickHouse` MergeTree read ->
Arrow -> `iceberg-rust` append through Lakekeeper, vended credentials,
format-version 2) live in Rust — `lakehouse-iceberg` +
`lakehouse-api::gold_export`, wired to `POST /api/gold/export/{mart}`
(`lakehouse-api::routes::gold`) — because `iceberg-rust` is what G1(a)
proved works, and there is no Python client for it in this stack. This
job's whole job is to be the scheduled trigger: call that one endpoint,
per configured mart, on a cadence, and record the result through the same
`bronze_meta.*` registry mechanism `maintenance.py`/
`replication_metrics.py` already use (per R10 — reuse that mechanism,
don't invent a parallel one).

Why an HTTP call to `lakehouse-api`, not a Dagster op that talks to
`iceberg-rust`/Lakekeeper directly: this is the exact "Dagster calls the
Rust API over HTTP" shape `lakehouse-api::routes::pipelines` already uses
in the opposite direction (`lakehouse-api` calling Dagster's GraphQL API)
— Dagster is a Python process with no `iceberg-rust` binding, and
`lakehouse-api` is already the one process in this stack the task brief
designates as the catalog-operation owner (ADR 0003).
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from typing import Any

import requests
from dagster import Definitions, ScheduleDefinition, job, op

from dispar_orchestrate.bronze_catalog import ClickHouseTarget, record_maintenance_run


def _env(name: str, default: str) -> str:
    value = os.environ.get(name, "").strip()
    return value if value else default


def _marts_from_env() -> list[str]:
    raw = os.environ.get("GOLD_EXPORT_MARTS", "gold_export_smoke")
    return [m.strip() for m in raw.split(",") if m.strip()]


@dataclass(frozen=True)
class GoldExportConfig:
    ch: ClickHouseTarget
    api_url: str
    marts: list[str]
    # D4 shape (`routes::gold::check_export_token`): a shared token, set
    # identically here and on `lakehouse-api` (both read
    # `GOLD_EXPORT_RUN_TOKEN` from the same compose `.env` — see
    # `docker-compose.yml`'s `gold-export-test-runner` usage comment for
    # why a one-off override here would not also reach the already-running
    # `lakehouse-api` container). Empty means `lakehouse-api` requires a
    # service-identity principal instead, which this job does not carry —
    # operators wanting the schedule to actually succeed must set this.
    run_token: str

    @classmethod
    def from_env(cls) -> "GoldExportConfig":
        return cls(
            ch=ClickHouseTarget.from_env(),
            api_url=_env("LAKEHOUSE_API_URL", "http://lakehouse-api:8080"),
            marts=_marts_from_env(),
            run_token=_env("GOLD_EXPORT_RUN_TOKEN", ""),
        )


def _headers(cfg: GoldExportConfig) -> dict[str, str]:
    return {"x-run-token": cfg.run_token} if cfg.run_token else {}


def export_one_mart(cfg: GoldExportConfig, mart: str) -> dict[str, Any]:
    resp = requests.post(
        f"{cfg.api_url}/api/gold/export/{mart}", headers=_headers(cfg), timeout=60
    )
    resp.raise_for_status()
    return resp.json()


@op
def run_gold_export(context) -> list[dict[str, Any]]:
    """Runs `POST /api/gold/export/{mart}` for every mart in
    `GOLD_EXPORT_MARTS` (comma-separated, default `gold_export_smoke` — the
    same mart name the acceptance test seeds), and records each result via
    `record_maintenance_run` (the same `bronze_meta.*` registry
    `maintenance.py` writes to — `GET /api/governance/maintenance` already
    surfaces that table, so a Gold export run shows up there too without a
    new console surface)."""
    cfg = GoldExportConfig.from_env()
    context.log.info(f"exporting {len(cfg.marts)} Gold mart(s): {cfg.marts}")

    results: list[dict[str, Any]] = []
    for mart in cfg.marts:
        try:
            body = export_one_mart(cfg, mart)
        except requests.HTTPError as exc:
            context.log.error(f"export of {mart!r} failed: {exc}")
            record_maintenance_run(
                table_name=f"gold.{mart}",
                dry_run_metrics={},
                applied_metrics={},
                skipped_verbs=[f"export_failed: {exc}"],
                target=cfg.ch,
            )
            raise
        context.log.info(f"exported {mart!r}: {body}")
        # `dry_run_metrics`/`applied_metrics`' numeric fields are shaped for
        # `expire_snapshots` (data/manifest file deletion counts), which a
        # Gold export never performs — left at their honest default of 0,
        # not repurposed to mean something else. `rowsExported` (the one
        # number this run actually produced) goes into the free-text
        # `skipped_verbs` field instead, so it is still visible via
        # `GET /api/governance/maintenance` without corrupting a column
        # whose meaning `maintenance.py`'s own readers rely on.
        record_maintenance_run(
            table_name=f"gold.{mart}",
            dry_run_metrics={},
            applied_metrics={},
            skipped_verbs=[f"rows_exported={body.get('rowsExported')}"],
            target=cfg.ch,
        )
        results.append(body)

    context.add_output_metadata({"marts_exported": len(results)})
    return results


@job
def gold_export_job() -> None:
    """`DAGSTER_LOCATION`-visible job name: `gold_export_job`."""
    run_gold_export()


# Daily at 04:00 — after `bronze_maintenance_job`'s 03:00 slot, since a
# freshly-maintained Bronze table is what most Gold marts are ultimately
# built from; arbitrary but conservative cadence, same reasoning
# `bronze_maintenance_schedule` gives.
gold_export_schedule = ScheduleDefinition(
    job=gold_export_job,
    cron_schedule="0 4 * * *",
)

gold_export_defs = Definitions(
    jobs=[gold_export_job],
    schedules=[gold_export_schedule],
)
