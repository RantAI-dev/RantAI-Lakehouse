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

# One op per mart (PART A of `parts/1b-dagster-one-op-per-unit-and-retries.md`)

Before this split a single `run_gold_export` op looped over every
configured mart and surfaced the WHOLE job's result as one step: one
mart's HTTP 500 took down the whole nightly run, including the marts
that would have succeeded. Now `list_gold_marts` yields one
`DynamicOutput` per mart, `export_gold_mart` runs the per-mart body
in a mapped step (so each mart is its own failure unit, retryable by
`DEFAULT_RETRY_POLICY` for transient blips), and `summarize_gold_export`
logs the count. The console now sees one mapped step per mart named
`export_gold_mart[<key>]` where it previously saw the single
`run_gold_export` step.
"""

from __future__ import annotations

import os
import re
from dataclasses import dataclass
from typing import Any

import requests
from dagster import (
    AssetMaterialization,
    DefaultScheduleStatus,
    Definitions,
    DynamicOut,
    DynamicOutput,
    Failure,
    ScheduleDefinition,
    job,
    op,
)

from dispar_orchestrate.bronze_catalog import ClickHouseTarget, record_maintenance_run
from dispar_orchestrate.op_metadata import DEFAULT_RETRY_POLICY, source_metadata


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
    # `lakehouse-api` container). The value authenticates twice:
    # `lakehouse-api::main::bootstrap_gold_export_service` mints the
    # `gold-export-scheduler` identity + credential from it at API boot
    # (gold-publish-per-mart plan T1), `_headers` sends it as both the
    # bearer credential and the `x-run-token` header. Empty means neither
    # exists — the schedule then runs and fails loudly (`401`/`503`)
    # instead of silently, which is the honest state to ship.
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
    """Same two-header shape as `alerts_run.py::_headers` — see the
    schedule comment below for why both are needed. Empty when no token
    is configured, so a caller can tell "not configured" apart from
    "configured, but Dagster forgot to send it" without inspecting the
    response.
    """
    if not cfg.run_token:
        return {}
    return {
        "x-run-token": cfg.run_token,
        "Authorization": f"Bearer {cfg.run_token}",
    }


def export_one_mart(cfg: GoldExportConfig, mart: str) -> dict[str, Any]:
    resp = requests.post(
        f"{cfg.api_url}/api/gold/export/{mart}", headers=_headers(cfg), timeout=60
    )
    resp.raise_for_status()
    return resp.json()


# Dagster's `DynamicOutput.mapping_key` must match `^[A-Za-z0-9_]+$`
# (`dagster._core.definitions.utils.check_valid_chars`). The exact ASCII
# rule from `parts/1b-dagster-one-op-per-unit-and-retries.md`: every
# character outside `[A-Za-z0-9_]` becomes `_`. Precompiled once at
# module load (`re`'s default cache keeps it cheap to call per input);
# this is the same rule `gold_export._sanitize_mapping_key`,
# `ingest_factory._sanitize_target`, and `maintenance._sanitize_mapping_key`
# all enforce, so a single mapping key can serve as a schedule name, an
# op name, or a mapping key consistently. Note: `str.isalnum()` would
# be wrong here -- it is True for non-ASCII letters (`"é".isalnum()`,
# `"²".isalnum()`), which would silently produce a non-ASCII mapping
# key that Dagster then rejects.
_SANITIZE_NON_ASCII = re.compile(r"[^A-Za-z0-9_]")


def _sanitize_mapping_key(name: str) -> str:
    """Dagster's `DynamicOutput.mapping_key` derived from a mart name by
    replacing every character outside `[A-Za-z0-9_]` with `_`. Two
    distinct marts that collide after this sanitization would mean a
    single mapped step trying to do two marts' work, which Dagster's
    graph itself would refuse to build -- caught here at the fan-out
    op, not at graph-build time, with both names in the message."""
    return _SANITIZE_NON_ASCII.sub("_", name)


# The op itself only calls the API per mart; the API reads the ClickHouse
# mart and writes its Iceberg copy, which is the data flow declared here.
@op(
    out=DynamicOut(),
    retry_policy=DEFAULT_RETRY_POLICY,
    tags=source_metadata(
        "dispar_orchestrate/gold_export.py::list_gold_marts",
        reads=["env GOLD_EXPORT_MARTS"],
        # `writes` is the side-effect of this op: a `DynamicOutput` per
        # mart, which becomes a mapped `export_gold_mart[<key>]` step.
        # Declared here so the test that walks every op in this code
        # location and asserts non-empty `writes` (the same one
        # `authored_factory`'s ops already satisfy) does not flag this
        # fan-out as having no writes.
        writes=["DynamicOutput mapping_key per configured Gold mart"],
    ),
)
def list_gold_marts(context) -> Any:
    """Yield one `DynamicOutput` per configured mart, sanitized to a
    Dagster-legal mapping key. Two marts that sanitize to the same key
    are refused with a non-retryable `Failure` -- they would otherwise
    collapse into one mapped step doing two marts' work, which is a
    failure mode that only the fan-out can detect."""
    cfg = GoldExportConfig.from_env()
    context.log.info(f"exporting {len(cfg.marts)} Gold mart(s): {cfg.marts}")
    seen: dict[str, str] = {}
    for mart in cfg.marts:
        key = _sanitize_mapping_key(mart)
        if key in seen:
            raise Failure(
                f"GOLD_EXPORT_MARTS contains two marts that map to the same "
                f"Dagster step key {key!r}: {seen[key]!r} and {mart!r} -- "
                "rename one so each mart maps to a unique step",
                allow_retries=False,
            )
        seen[key] = mart
        yield DynamicOutput(mart, mapping_key=key)


@op(
    retry_policy=DEFAULT_RETRY_POLICY,
    tags=source_metadata(
        "dispar_orchestrate/gold_export.py::export_gold_mart",
        reads=["ClickHouse serving.{mart} (via the API)"],
        writes=[
            "Iceberg gold.{mart} (via POST /api/gold/export)",
            "ClickHouse lake.bronze_meta.maintenance_run",
        ],
    ),
)
def export_gold_mart(context, mart: str) -> dict[str, Any]:
    """Run `POST /api/gold/export/{mart}` for ONE mart and record the
    outcome via `record_maintenance_run` (the same `bronze_meta.*`
    registry `maintenance.py` writes to -- `GET /api/governance/
    maintenance` already surfaces that table, so a Gold export run shows
    up there too without a new console surface).

    An HTTP error is recorded as a `maintenance_run` failure row AND
    bare re-raised (no `Failure(allow_retries=False)` wrapper) -- the
    re-raise lets `DEFAULT_RETRY_POLICY` retry a transient blip (a busy
    ClickHouse on the `lakehouse-api` side, the canonical case the
    plan names), while a persistent 4xx/5xx fails after the policy's
    two attempts. Config-shaped failures (`SecretRefRejected`,
    `UnknownAdapter`, etc.) are not raised here at all -- the
    connector-resolution path in `_fetch_one_connector` /
    `_resolve_secrets` either resolves them before this op runs or
    fails out of band. Re-raising the HTTPError bare keeps the
    partition between "retryable transient" and "non-retryable
    config/auth" honest: only the latter wraps in
    `Failure(allow_retries=False)`, and they don't originate here."""
    cfg = GoldExportConfig.from_env()
    try:
        body = export_one_mart(cfg, mart)
    except requests.HTTPError as exc:
        # Bare re-raise of the HTTPError (no `Failure` wrap) -- lets
        # `DEFAULT_RETRY_POLICY` (max_retries=2, delay=30s, exponential
        # backoff with ±jitter) retry a transient 5xx (busy ClickHouse
        # on the `lakehouse-api` side). A persistent failure exhausts
        # those attempts and Dagster surfaces it as the mapped step's
        # own failed step. The failure row is written BEFORE the
        # re-raise so the governance surface (`GET /api/governance/
        # maintenance`) sees the failure immediately, not only after
        # the policy's retries have all come up empty.
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
    # Gold export never performs -- left at their honest default of 0,
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
    # The API's own count of rows it wrote to Iceberg, as a
    # materialization, so the run's step carries it and the console
    # can show rows per run. A body without an integer count reports
    # nothing rather than a guessed number.
    rows = body.get("rowsExported")
    if isinstance(rows, int) and not isinstance(rows, bool):
        context.log_event(AssetMaterialization(asset_key=f"gold.{mart}", metadata={"rows": rows}))
    return body


@op(
    retry_policy=DEFAULT_RETRY_POLICY,
    tags=source_metadata(
        "dispar_orchestrate/gold_export.py::summarize_gold_export",
        # The output of this op is the run's summary count, surfaced
        # through `add_output_metadata({"marts_exported": N})`; declared
        # as a `writes` phrase so the every-op walk test (which refuses
        # empty `writes`) treats this no-input collect step as having a
        # real artifact.
        reads=["results list from export_gold_mart[*]"],
        writes=["output metadata marts_exported"],
    ),
)
def summarize_gold_export(context, results: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Collect every mapped `export_gold_mart` step's result and log the
    marts-exporter count (the single number this whole job owes to the
    rest of the code location -- a per-mart rows count is on each
    `export_gold_mart` step's own materialization, never aggregated
    here)."""
    context.log.info(f"exported {len(results)} Gold mart(s) this run")
    context.add_output_metadata({"marts_exported": len(results)})
    return results


@job
def gold_export_job() -> None:
    """`DAGSTER_LOCATION`-visible job name: `gold_export_job`. A fan-out
    op (`list_gold_marts`) yields one `DynamicOutput` per mart; the
    mapped `export_gold_mart` runs the per-mart body so a single mart's
    HTTP error fails only that step; `summarize_gold_export` collects
    every mapped step's result for the run summary."""
    summarize_gold_export(list_gold_marts().map(lambda mart: export_gold_mart(mart)).collect())


# Daily at 04:00 — after `bronze_maintenance_job`'s 03:00 slot, since a
# freshly-maintained Bronze table is what most Gold marts are ultimately
# built from; arbitrary but conservative cadence, same reasoning
# `bronze_maintenance_schedule` gives.
#
# `default_status=DefaultScheduleStatus.RUNNING`, matching every other
# schedule this code location registers (`bronze_maintenance_schedule`,
# `capacity_snapshot_schedule`, `alerts_run_schedule`): a schedule created
# stopped never fires until someone notices and manually flips it on in
# the Dagster UI, and from outside, a stopped schedule looks identical to
# a healthy one — there is no dashboard signal that distinguishes "off on
# purpose" from "off because nobody remembered." Registering RUNNING makes
# the eventual failure loud (a run appears in the Dagster UI's run list,
# failed, every night) instead of silent (no run ever appears at all).
# `default_status` must be the `DefaultScheduleStatus` enum member, not a
# bare string — the installed Dagster version raises
# `dagster._core.errors.ParameterCheckError` for `default_status="RUNNING"`
# (confirmed directly against this repo's `~/.cache/rantai-dagster-venv`;
# a different task in this programme hit exactly this).
#
# Auth (gold-publish-per-mart plan T1): `POST /api/gold/export/{mart}`'s
# `Policy::RequiresAuth` floor is enforced by `auth_gate` BEFORE the
# handler (and `check_export_token`'s own `x-run-token` check) ever runs,
# so a real credential is required — `_headers` sends the shared token two
# ways, exactly like `alerts_run.py`: `Authorization: Bearer
# <GOLD_EXPORT_RUN_TOKEN>` clears the floor via the `gold-export-scheduler`
# identity that `lakehouse-api::main::bootstrap_gold_export_service`
# idempotently mints from the same env value at API boot, and
# `x-run-token: <GOLD_EXPORT_RUN_TOKEN>` satisfies `check_export_token`'s
# token branch, which is what actually gates the export. With the token
# unset, no identity is seeded and `_headers` returns `{}` — the schedule
# below is still registered and every run fails loudly (`401`/`503`),
# which is the honest, visible failure mode, not a silent one.
gold_export_schedule = ScheduleDefinition(
    job=gold_export_job,
    cron_schedule="0 4 * * *",
    default_status=DefaultScheduleStatus.RUNNING,
)

gold_export_defs = Definitions(
    jobs=[gold_export_job],
    schedules=[gold_export_schedule],
)
