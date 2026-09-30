"""Builds one real Dagster job per console-authored pipeline (WS4 items
E1/E3, grand plan §6), plus a schedule when its authored schedule is a cron.

`createPipeline`/`generatePipelineFromPrompt` write a `pipeline_definition`
row. At code-load time this module reads `GET /api/pipelines/runnable`
(every `ready` or `paused` authored pipeline, with its definition, across
all tenants) and builds, per pipeline:

* an `authored__<id>` job that reads its source, applies its `transforms`,
  and writes its target (item E3, below);
* an `authored__<id>_schedule` when the pipeline's schedule is a five-field
  cron. `default_status` is RUNNING for a `ready` pipeline and STOPPED for a
  `paused` one: the schedule the author set is meant to fire, and one that
  is not wanted is paused from the console, which also switches this
  schedule off (`routes::pipelines::authored_status`).

The API asks the webserver to reload this code location whenever a
pipeline becomes ready, is edited, paused, resumed or deleted
(`routes::authored_pipelines::reload_orchestrator`). The code location runs
`dagster code-server start`, which re-imports this module on reload.

# The gaps this used to document, and how they closed

Earlier this module read `GET /api/pipelines`, which (1) carried no
`definition` on authored rows and (2) is tenant-scoped, while the
`authored-pipeline-scheduler` service identity has no tenant, so it saw no
authored rows at all; and (3) `PIPELINE_RUN_TOKEN` was never passed to any
container. `GET /api/pipelines/runnable` answers (1) and (2): it returns
definitions, across tenants, to service identities only. `docker-compose.yml`
now passes `PIPELINE_RUN_TOKEN` to both `lakehouse-api` and this code
location, answering (3).

Authenticates the same way `agent_runs.py`/`gold_export.py` do: a bearer
token from `PIPELINE_RUN_TOKEN`.
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from typing import Any

import requests
from dagster import AssetMaterialization, DefaultScheduleStatus, ScheduleDefinition, job, op

from dispar_orchestrate import authored_transforms, op_metadata
from dispar_orchestrate.bronze_catalog import ClickHouseTarget, _ch_exec, _ch_query_json


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


RUNNABLE_STATUSES = ("ready", "paused")


def _fetch_authored_pipelines(cfg: AuthoredPipelineConfig) -> list[dict[str, Any]]:
    """Fetch every runnable (`ready` or `paused`) authored pipeline that
    carries a `definition` payload, or an EMPTY list on any failure -- never raises
    (mirrors `agent_runs._fetch_schedulable_employees`; this module
    imports at Dagster code-load time, alongside every other job/schedule
    in this code location, so an uncaught exception here would take all of
    them down).

    The three distinct "empty list" causes below are NOT the same fact and
    are logged distinctly, per WS4 item E1: (1) `PIPELINE_RUN_TOKEN` unset
    -- authored scheduling is deliberately inert, not attempted; (2) the
    HTTP call itself failed (unreachable host, connection refused, a
    non-2xx status, e.g. a 403 when the token does not belong to a
    service identity) -- `lakehouse-api` was reached (or an
    attempt was made) and the answer was "no" or "couldn't tell", never
    silently treated as "there are no authored pipelines"; (3) the call
    succeeded and returned valid JSON with genuinely zero rows meeting the
    filter -- the only case that is actually "there are no [ready,
    defined] authored pipelines right now", and the only one that does not
    print a WARNING.
    """
    if not cfg.run_token:
        print(
            "dispar_orchestrate.authored_factory: PIPELINE_RUN_TOKEN is unset; "
            "loading with zero authored-pipeline jobs"
        )
        return []

    try:
        resp = requests.get(
            f"{cfg.api_url}/api/pipelines/runnable",
            headers=_headers(cfg),
            timeout=10,
        )
        resp.raise_for_status()
    except requests.RequestException as exc:
        print(
            f"WARNING: dispar_orchestrate.authored_factory: could not fetch "
            f"authored pipelines from {cfg.api_url!r} ({exc}); this is a FETCH "
            "FAILURE, not evidence that no authored pipelines exist -- loading "
            "with zero authored-pipeline jobs"
        )
        return []

    try:
        body = resp.json()
    except ValueError as exc:
        print(
            f"WARNING: dispar_orchestrate.authored_factory: lakehouse-api "
            f"returned a non-JSON /api/pipelines/runnable body ({exc}); this is a FETCH "
            "FAILURE, not evidence that no authored pipelines exist -- loading "
            "with zero authored-pipeline jobs"
        )
        return []

    pipelines = body.get("pipelines", [])
    if not isinstance(pipelines, list):
        print(
            "WARNING: dispar_orchestrate.authored_factory: /api/pipelines/runnable "
            f"returned {type(pipelines).__name__} for 'pipelines', expected a "
            "list; this is a FETCH FAILURE, not evidence that no authored "
            "pipelines exist -- loading with zero authored-pipeline jobs"
        )
        return []

    return [
        p
        for p in pipelines
        if isinstance(p, dict) and p.get("status") in RUNNABLE_STATUSES and p.get("definition")
    ]


# ── item E3: build_authored_job -- read, transform, write, FBIC stub ─────

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

CONNECTOR_SOURCE_UNSUPPORTED_REASON = (
    "connector-sourced authored pipelines are not executed by this build: WS4 "
    "item E3 implements the direct ClickHouse-zone-to-ClickHouse-zone read/"
    "write path only. Wiring WS3's four adapters.*.build_source modules "
    "(sql/files/rest/sheets) plus secret resolution and a bulk ClickHouse "
    "load is real, separate integration work this task does not fake with an "
    "empty or partial read -- reported here as an honest run failure instead."
)


class AuthoredJobError(RuntimeError):
    """Raised inside an authored pipeline's op body for a condition this
    build genuinely cannot execute (see the two reasons above). Never
    caught -- the whole point is that the Dagster run fails loudly rather
    than reporting a fabricated or silently-partial result."""


def _dagster_safe_name(raw: str) -> str:
    """Dagster job/op names must match `^[A-Za-z0-9_]+$`
    (`dagster._core.definitions.utils.check_valid_chars`, same fact
    `agent_runs._schedule_name`'s doc comment cites) -- `pipeline_definition`
    ids are `pl-<slug>-<base36 millis>` (hyphenated), so hyphens (and any
    other non-matching character) are replaced with `_`."""
    return "".join(char if char.isalnum() or char == "_" else "_" for char in raw)


def _is_safe_ch_identifier(name: str) -> bool:
    """Defense in depth for a zone/table name pulled out of a stored
    `pipeline_definition` row before it is interpolated into a ClickHouse
    statement -- the same non-trust-by-default posture
    `authored_transforms.py` takes for a transform string, applied here to
    the four identifier fields this module itself builds SQL from
    (`source_zone`/`source_table`/`target_zone`/`target_table` are
    validated server-side at `POST /api/pipelines` time, but this op
    re-checks rather than assuming that validation can never be bypassed by
    a future/older API version)."""
    return bool(name) and not name[0].isdigit() and all(c.isascii() and (c.isalnum() or c == "_") for c in name)


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


def _ensure_target_table(target: ClickHouseTarget, target_zone: str, target_table: str, select_sql: str) -> None:
    """Schema-on-write for an authored pipeline's target: create the
    database and (if it does not already exist) the table, inferring
    columns from `select_sql` via ClickHouse's own `CREATE TABLE ... AS
    SELECT ...` (`LIMIT 0` so this never inserts rows itself -- inserting
    is `_write_clickhouse_table`'s job, run every time this job runs, not
    only on first creation)."""
    _ch_exec(target, f"CREATE DATABASE IF NOT EXISTS {target_zone}")
    _ch_exec(
        target,
        f"CREATE TABLE IF NOT EXISTS {target_zone}.`{target_table}` "
        f"ENGINE = MergeTree ORDER BY tuple() AS {select_sql} LIMIT 0",
    )


def _write_clickhouse_table(target: ClickHouseTarget, target_zone: str, target_table: str, select_sql: str) -> int:
    """Insert `select_sql`'s rows into `target_zone.target_table` and
    return the REAL row-count delta measured before/after -- never a
    fabricated or assumed count (`AGENTS.md`: "no invented metrics")."""
    _ensure_target_table(target, target_zone, target_table, select_sql)
    before = _ch_query_json(target, f"SELECT count() AS n FROM {target_zone}.`{target_table}`")
    _ch_exec(target, f"INSERT INTO {target_zone}.`{target_table}` {select_sql}")
    after = _ch_query_json(target, f"SELECT count() AS n FROM {target_zone}.`{target_table}`")
    return int(after[0]["n"]) - int(before[0]["n"])


def _op_for_pipeline(pipeline: dict[str, Any]) -> Any:
    definition = pipeline["definition"]
    pid = pipeline["id"]
    safe_name = _dagster_safe_name(pid)

    @op(
        name=f"authored_{safe_name}",
        tags=op_metadata.source_metadata(
            "dispar_orchestrate/authored_factory.py::_op_for_pipeline",
            reads=[f"ClickHouse {definition.get('sourceZone')}.{definition.get('sourceTable')}"],
            writes=[f"ClickHouse {definition.get('targetZone')}.{definition.get('targetTable')}"],
        ),
    )
    def _run(context) -> dict[str, Any]:
        # Re-validate every transform against this module's own grammar
        # port (item E2) BEFORE building any SQL from it -- a Postgres row
        # could in principle have been written by a future/older API
        # version with a looser grammar (authored_transforms.py's own
        # module doc). `parse_transform` raises `TransformError`
        # (deliberately uncaught here) on any invalid string, which fails
        # THIS op/run loudly -- a rejected transform is never dropped.
        transforms = [authored_transforms.parse_transform(t) for t in definition.get("transforms", [])]

        connector_id = definition.get("connectorId")
        if connector_id:
            raise AuthoredJobError(CONNECTOR_SOURCE_UNSUPPORTED_REASON)

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

        ch = ClickHouseTarget.from_env()
        source = f"{source_zone}.`{source_table}`"
        select_sql = _build_select_sql(source, transforms)
        written = _write_clickhouse_table(ch, target_zone, target_table, select_sql)

        if definition.get("fbicEnabled"):
            context.log.info(FBIC_UNSUPPORTED_REASON)
            skipped_verbs = [FBIC_UNSUPPORTED_REASON]
        else:
            skipped_verbs = []

        context.log_event(
            AssetMaterialization(
                asset_key=f"{target_zone}.{target_table}",
                metadata={"rows": written},
            )
        )
        return {"rows": written, "skipped_verbs": skipped_verbs}

    return _run


def build_authored_job(pipeline: dict[str, Any]) -> Any:
    op_fn = _op_for_pipeline(pipeline)
    safe_name = _dagster_safe_name(pipeline["id"])

    @job(name=f"authored__{safe_name}")
    def _authored_job() -> None:
        op_fn()

    return _authored_job


def _is_five_field_cron(schedule: Any) -> bool:
    return isinstance(schedule, str) and len(schedule.split()) == 5


def build_authored_schedule(pipeline: dict[str, Any], authored_job: Any) -> ScheduleDefinition | None:
    """The pipeline's schedule, or `None` when it has no cron (`"manual"`,
    `"On demand"`, ...) or its cron is one Dagster refuses. A refused cron
    is logged and skipped rather than raised: one bad row must not take
    down every job in this code location."""
    cron = pipeline.get("schedule")
    if not _is_five_field_cron(cron):
        return None
    # `default_status` follows the pipeline: RUNNING for `ready` (the
    # author set this schedule to have it fire), STOPPED for `paused`.
    status = (
        DefaultScheduleStatus.RUNNING
        if pipeline.get("status") == "ready"
        else DefaultScheduleStatus.STOPPED
    )
    try:
        return ScheduleDefinition(
            name=f"{authored_job.name}_schedule",
            cron_schedule=cron,
            job=authored_job,
            default_status=status,
        )
    except Exception as exc:  # noqa: BLE001 -- see the docstring
        print(
            f"WARNING: dispar_orchestrate.authored_factory: pipeline {pipeline.get('id')!r} "
            f"has a schedule Dagster refused ({type(exc).__name__}); building its job "
            "without a schedule"
        )
        return None


def build_authored_definitions(
    cfg: AuthoredPipelineConfig | None = None,
) -> tuple[list[Any], list[ScheduleDefinition]]:
    """Every authored job and schedule, from ONE fetch, so the two lists
    can never describe different sets of pipelines."""
    cfg = cfg or AuthoredPipelineConfig.from_env()
    jobs: list[Any] = []
    schedules: list[ScheduleDefinition] = []
    for pipeline in _fetch_authored_pipelines(cfg):
        authored_job = build_authored_job(pipeline)
        jobs.append(authored_job)
        schedule = build_authored_schedule(pipeline, authored_job)
        if schedule is not None:
            schedules.append(schedule)
    return jobs, schedules


def build_authored_jobs(cfg: AuthoredPipelineConfig | None = None) -> list[Any]:
    return build_authored_definitions(cfg)[0]


# Built at Dagster code-load time, same pattern `agent_run_schedules`
# (`agent_runs.py`) uses -- see `_fetch_authored_pipelines`'s doc comment
# for every way this degrades to empty lists without raising.
# Two plain assignments rather than tuple unpacking, so
# `ops/lint/check_intra_package_imports.py` (which reads module-level
# names statically) sees both names `definitions.py` imports.
_authored_definitions = build_authored_definitions()
authored_jobs = _authored_definitions[0]
authored_schedules = _authored_definitions[1]
