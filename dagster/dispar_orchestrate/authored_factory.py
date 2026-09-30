"""ONE static `authored_pipeline_job` (config: `pipeline_id`) that runs any
pipeline a console user authored (`pipeline_definition`, written by
`POST /api/pipelines` and `POST /api/pipelines/generate`).

Same shape as `ingest_factory.py`'s `ingest_job`: the run's only input is
the pipeline's id, and the op fetches that pipeline's definition from
`GET /api/pipelines/runnable` when the run starts. A pipeline activated a
moment ago therefore runs without reloading this code location. The
per-pipeline `authored__<id>` jobs this module used to build at import
time needed a container restart for every new pipeline (`dagster api grpc`
does not reload definitions), and never saw one anyway: they were built
from `GET /api/pipelines`, which is tenant-scoped (this service identity
belongs to no tenant) and carries no definition.

Launched by `POST /api/pipelines/{id}/trigger` (`lakehouse-api`) with run
config `{"ops": {"run_authored_pipeline": {"config": {"pipeline_id":
"pl-..."}}}}`; the API finds a pipeline's runs by that same config. An
authored pipeline's `schedule` is a free-text label and is not scheduled.

# What a run does

1. Fetch the pipeline's definition. Only `ready` pipelines are listed, so a
   run of a draft or paused pipeline fails here instead of executing.
2. Re-validate every transform with `authored_transforms` (the Rust
   grammar's port): the row may come from `POST /api/pipelines/generate`,
   which stores what the LLM proposed, or from an older API version.
3. Resolve the source zone to what ClickHouse reads: `bronze` is the Bronze
   Iceberg table through this job's own `DataLakeCatalog` database
   ([`CATALOG_DB`]); `silver` and `serving` are ClickHouse databases.
4. Rebuild the target with `ch_models.run_model`: `CREATE OR REPLACE TABLE`
   from one `SELECT`, so a second run replaces the rows rather than adding
   a second copy. `incrementalColumn` is not used yet. Target zones are
   `ch_models.ALLOWED_SCHEMAS` (`silver`, `serving`).

`connectorId` is lineage only: that connector's ingest loaded the Bronze
table the pipeline reads.

Authenticates the same way `agent_runs.py`/`gold_export.py` do: a bearer
token from `PIPELINE_RUN_TOKEN`, whose service identity holds
`pipeline:execute` only (`lakehouse-api::main::bootstrap_pipeline_run_service`).
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from typing import Any, Callable

import requests
from dagster import AssetKey, AssetMaterialization, Field, job, op

from dispar_orchestrate import authored_transforms, ch_models, op_metadata
from dispar_orchestrate.bronze_catalog import ClickHouseTarget

# This job's own `DataLakeCatalog` database, separate from every other
# job's for the reason `silver_transform.py` gives for its own.
CATALOG_DB = "icecat_pipelines"

# Zones a pipeline may read. Mirrors `SOURCE_ZONES` in
# `rust/crates/lakehouse-api/src/routes/pipelines.rs`, which refuses any
# other zone at create time; re-checked here like the transforms are.
SOURCE_ZONES = frozenset({"bronze", "silver", "serving"})

# Every target is a full rebuild; no column order is implied.
TARGET_ENGINE = "MergeTree ORDER BY tuple()"


def _env(name: str, default: str) -> str:
    value = os.environ.get(name, "").strip()
    return value if value else default


@dataclass(frozen=True)
class AuthoredPipelineConfig:
    api_url: str
    run_token: str

    @classmethod
    def from_env(cls) -> "AuthoredPipelineConfig":
        return cls(
            api_url=_env("LAKEHOUSE_API_URL", "http://lakehouse-api:8080"),
            run_token=_env("PIPELINE_RUN_TOKEN", ""),
        )


def _headers(cfg: AuthoredPipelineConfig) -> dict[str, str]:
    """Same two-header shape `agent_runs._headers` sends: `Authorization:
    Bearer` clears `auth_gate`'s floor, `x-run-token` is a
    belt-and-suspenders duplicate some routes check directly."""
    if not cfg.run_token:
        return {}
    return {"Authorization": f"Bearer {cfg.run_token}", "x-run-token": cfg.run_token}


# `POST /api/ai/enrich` (or any AI-enrichment endpoint) does not exist in
# this codebase -- verified by reading `rust/crates/lakehouse-api/src/
# routes/mod.rs`'s full `/api/ai/*` route list (`chat`, `tool`, `sessions`,
# `build-status`; no `enrich`) and `policy.rs`'s `POLICY_TABLE` (same four,
# no fifth). The grand plan's own text ("WS7 defines POST /api/ai/enrich")
# is NOT corroborated by WS7's actual plan document
# (`docs/superpowers/plans/2026-09-11-ws7-enforcement-citations-budgets.md`,
# grepped for "enrich"/"ai/enrich": no matches) -- so this reason names
# only the verified fact (no such route exists anywhere in this build),
# not the plan's unverified claim about which future workstream adds one.
FBIC_UNSUPPORTED_REASON = (
    "fbic: not available -- no AI-enrichment route (e.g. POST /api/ai/enrich) "
    "exists in this codebase; verified against rust/crates/lakehouse-api/src/"
    "routes/mod.rs's full /api/ai/* route list (chat, tool, sessions, "
    "build-status only) and policy.rs's POLICY_TABLE"
)


class AuthoredJobError(RuntimeError):
    """Raised inside `run_authored_pipeline` for a pipeline this build
    cannot execute. Never caught -- the Dagster run fails loudly, with this
    message in its log, rather than reporting a fabricated or
    silently-partial result."""


def fetch_runnable_pipeline(cfg: AuthoredPipelineConfig, pipeline_id: str) -> dict[str, Any]:
    """`pipeline_id`'s entry in `GET /api/pipelines/runnable`. Called at
    RUN time only, so unlike the code-load-time factories in this package
    it raises: a run that cannot read its definition has failed.

    # Errors

    `AuthoredJobError` when `PIPELINE_RUN_TOKEN` is unset or the pipeline
    is not listed (unknown, draft or paused); `requests.HTTPError` or
    `requests.RequestException` when `lakehouse-api` refuses the call or
    cannot be reached.
    """
    if not cfg.run_token:
        raise AuthoredJobError(
            "PIPELINE_RUN_TOKEN is not set in the dagster-code-location container, so this run "
            "cannot read its pipeline's definition from lakehouse-api. Set the same value for "
            "lakehouse-api and dagster-code-location and restart both."
        )
    resp = requests.get(f"{cfg.api_url}/api/pipelines/runnable", headers=_headers(cfg), timeout=10)
    resp.raise_for_status()
    for pipeline in resp.json():
        if isinstance(pipeline, dict) and pipeline.get("id") == pipeline_id:
            return pipeline
    raise AuthoredJobError(
        f"pipeline {pipeline_id!r} is not ready to run: it does not exist, or it is a draft or "
        "paused. Activate or resume it in the console, then run it again."
    )


def _is_safe_ch_identifier(name: str) -> bool:
    """Defense in depth for a zone/table name pulled out of a stored
    `pipeline_definition` row before it is interpolated into a ClickHouse
    statement -- the same non-trust-by-default posture
    `authored_transforms.py` takes for a transform string. `POST
    /api/pipelines` validates these too (`check_location`), but
    `POST /api/pipelines/generate` and older rows did not."""
    return bool(name) and not name[0].isdigit() and all(c.isascii() and (c.isalnum() or c == "_") for c in name)


def _source_sql(zone: str, table: str) -> str:
    """The table reference a pipeline's `SELECT` reads from."""
    if zone == "bronze":
        return f"{CATALOG_DB}.`bronze.{table}`"
    return f"{zone}.`{table}`"


def _build_select_sql(source: str, transforms: list[authored_transforms.Transform]) -> str:
    """Compose the whole `transforms` list into ONE `SELECT ... FROM
    source [WHERE ...] [ORDER BY ... LIMIT 1 BY ...]` statement -- see
    this task's plan doc: a `Dedupe` becomes an `ORDER BY` clause paired
    with `LIMIT BY` executed server-side in ClickHouse; `Filter`/`Cast`/
    `Rename`/`Select` compose into the column list and `WHERE` clause.
    Every `Filter` clause is rendered through
    `authored_transforms.render_clickhouse` directly (item E2's actual
    function, never a re-derivation of its literal-escaping)."""
    columns: list[str] | None = None
    renames: dict[str, str] = {}
    casts: dict[str, str] = {}
    filters: list[str] = []
    dedupe_key: str | None = None

    for t in transforms:
        if isinstance(t, authored_transforms.Select):
            columns = list(t.columns)
        elif isinstance(t, authored_transforms.Rename):
            renames[t.from_col] = t.to_col
        elif isinstance(t, authored_transforms.Cast):
            casts[t.column] = t.target_type
        elif isinstance(t, authored_transforms.Filter):
            filters.append(authored_transforms.render_clickhouse(t))
        elif isinstance(t, authored_transforms.Dedupe):
            dedupe_key = t.key
        else:  # unreachable -- authored_transforms.Transform is a closed union
            raise AuthoredJobError(f"unrenderable transform: {t!r}")

    if columns is None:
        select_cols = "*"
    else:
        rendered = []
        for c in columns:
            expr = f"CAST({c} AS {casts[c]})" if c in casts else c
            if c in renames:
                expr = f"{expr} AS {renames[c]}"
            rendered.append(expr)
        select_cols = ", ".join(rendered)

    sql = f"SELECT {select_cols} FROM {source}"
    if filters:
        sql += " WHERE " + " AND ".join(filters)
    if dedupe_key:
        sql += f" ORDER BY {dedupe_key} LIMIT 1 BY {dedupe_key}"
    return sql


def pipeline_model(pipeline: dict[str, Any]) -> tuple[ch_models.Model, bool]:
    """The `ch_models.Model` one authored pipeline rebuilds, and whether its
    source is Bronze (which needs [`CATALOG_DB`] to exist first). Builds no
    SQL from a field it has not validated.

    # Errors

    `authored_transforms.TransformError` for a transform outside the
    grammar; `AuthoredJobError` for a zone this job cannot read or write,
    or a zone/table that is not a plain identifier.
    """
    definition = pipeline["definition"]
    transforms = [authored_transforms.parse_transform(t) for t in definition.get("transforms", [])]

    source_zone = definition["sourceZone"]
    source_table = definition["sourceTable"]
    target_zone = definition["targetZone"]
    target_table = definition["targetTable"]
    for ident, field in (
        (source_zone, "sourceZone"),
        (source_table, "sourceTable"),
        (target_zone, "targetZone"),
        (target_table, "targetTable"),
    ):
        if not _is_safe_ch_identifier(ident):
            raise AuthoredJobError(f"unsafe ClickHouse identifier in {field!r}: {ident!r}")
    if source_zone not in SOURCE_ZONES:
        raise AuthoredJobError(
            f"source zone {source_zone!r} cannot be read; use one of {sorted(SOURCE_ZONES)}"
        )
    if target_zone not in ch_models.ALLOWED_SCHEMAS:
        raise AuthoredJobError(
            f"target zone {target_zone!r} cannot be written; use one of "
            f"{sorted(ch_models.ALLOWED_SCHEMAS)}"
        )

    model = ch_models.Model(
        name=pipeline["id"],
        target=f"{target_zone}.{target_table}",
        engine=TARGET_ENGINE,
        select=_build_select_sql(_source_sql(source_zone, source_table), transforms),
    )
    return model, source_zone == "bronze"


def execute_pipeline(
    pipeline: dict[str, Any],
    *,
    ch: ClickHouseTarget | None = None,
    run_model: Callable[[ClickHouseTarget, ch_models.Model], int] = ch_models.run_model,
    ensure_catalog_database: Callable[[ClickHouseTarget, str], None] = ch_models.ensure_catalog_database,
) -> tuple[ch_models.Model, int]:
    """Rebuild one pipeline's target and return the model and the target's
    row count afterwards.

    # Errors

    Everything [`pipeline_model`] raises, and `AuthoredJobError` carrying
    ClickHouse's own message when it refuses the SQL (a missing source
    table, a column a transform names that does not exist):
    `ch_models.ch_exec` raises a bare HTTP status, which says nothing about
    what to fix.
    """
    model, reads_bronze = pipeline_model(pipeline)
    ch = ch or ch_models.ch_target()
    try:
        if reads_bronze:
            ensure_catalog_database(ch, CATALOG_DB)
        rows = run_model(ch, model)
    except requests.HTTPError as exc:
        detail = exc.response.text.strip() if exc.response is not None else ""
        raise AuthoredJobError(f"ClickHouse refused this pipeline's SQL: {detail[:1000] or exc}") from exc
    return model, rows


@op(
    config_schema={"pipeline_id": Field(str, description="The pipeline_definition.id (pl-...) to run.")},
    tags=op_metadata.source_metadata("dispar_orchestrate/authored_factory.py::run_authored_pipeline"),
)
def run_authored_pipeline(context) -> dict[str, Any]:
    pipeline_id: str = context.op_config["pipeline_id"]
    try:
        pipeline = fetch_runnable_pipeline(AuthoredPipelineConfig.from_env(), pipeline_id)
        model, rows = execute_pipeline(pipeline)
    except Exception as exc:
        # The console's run log lists log messages, and a failed step's
        # own event says only that it failed; without this line the reason
        # (a missing table, a paused pipeline) is not on the page.
        context.log.error(f"{type(exc).__name__}: {exc}")
        raise
    context.log.info(f"{pipeline['name']}: {model.target} rebuilt with {rows} rows")

    if pipeline["definition"].get("fbicEnabled"):
        context.log.info(FBIC_UNSUPPORTED_REASON)
        skipped_verbs = [FBIC_UNSUPPORTED_REASON]
    else:
        skipped_verbs = []

    context.log_event(
        AssetMaterialization(
            asset_key=AssetKey(model.target.split(".")),
            metadata={"rows": rows, "pipeline_id": pipeline_id},
        )
    )
    return {"rows": rows, "skipped_verbs": skipped_verbs}


@job(name="authored_pipeline_job")
def authored_pipeline_job() -> None:
    """The SAME static job for every authored pipeline, launched with run
    config `{"ops": {"run_authored_pipeline": {"config": {"pipeline_id":
    "..."}}}}` (mirrors `ingest_job`'s `connector_id` shape)."""
    run_authored_pipeline()
