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
  schedule off (`routes::pipelines::authored_status`);
* an `authored__<id>_after` run_status_sensor when the pipeline's
  `dependsOn` is non-empty AND its `status` is `ready` (R3 plan 2a, F1.2).
  The sensor fires after a SUCCESS run of any of its upstreams; it yields
  a `RunRequest` only when EVERY upstream has had a SUCCESS run that
  finished AFTER this pipeline's most-recent start time (ALL semantics —
  see `build_authored_dependency_sensor` for the rule, named
  upstream-by-upstream in the `SkipReason` so the UI can show "waiting on
  <id>"). A `paused` pipeline contributes NO sensor: the orchestrator
  keeps the sensor's RUNNING/STOPPED state across a code-location
  reload, so the `paused` branch has to skip the sensor AND the API
  `authored_status` route has to stop the stored sensor next to the
  schedule (F1.2).

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
from dagster import (
    AssetMaterialization,
DagsterRunStatus,
    DefaultScheduleStatus,
    DefaultSensorStatus,
    Failure,
    JobSelector,
    RetryPolicy,
    RunRequest,
    RunsFilter,
    ScheduleDefinition,
    SkipReason,
    job,
    op,
    run_status_sensor,
)

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
    """REPLACE `target_zone.target_table` with `select_sql`'s rows,
    atomically, and return the REAL row count of the swapped-in target.

    Semantics: the function is a `REPLACE`, not an `APPEND`. The op that
    calls it (`_op_for_pipeline`) treats each run as a full snapshot of
    the pipeline's output -- a downstream that reads `serving.<table>`
    while this op is in the middle of writing would otherwise see a
    half-populated table (`AGENTS.md` "fail closed"; F1.4 plan #57).
    Inserting into the live target is the same bug a `TRUNCATE; INSERT`
    pair would have, just hidden by the INSERT's default appending
    semantics.

    Implementation: a staging table named
    `{target_zone}.__staging_{target_table}` holds the new rows. We
    `DROP TABLE IF EXISTS` any leftover from a prior failed attempt
    (idempotency under retry), `CREATE TABLE ... AS {select_sql}`
    (re-uses the `_ensure_target_table` shape so column inference is
    consistent), `INSERT INTO staging SELECT`, then
    `EXCHANGE TABLES ...` so the staging table's contents replace the
    live target in a single ClickHouse mutation step. The final
    `SELECT count()` reads the SWAPPED-IN target (its name is still
    `target_zone.target_table`; the staging table's contents moved
    there). The staging table is dropped in the same sequence so the
    next run's `DROP IF EXISTS` is a no-op.

    Failure modes (per AGENTS.md "no invented metrics"): any raise
    from `_ch_exec` propagates unwrapped -- a partial INSERT into the
    staging table leaves the live target unchanged, the staging table
    is dropped by the next run's `DROP IF EXISTS`, and the failing
    run is retryable because no committed write happened to the live
    target.
    """
    _ensure_target_table(target, target_zone, target_table, select_sql)
    staging_table = f"__staging_{target_table}"
    # Idempotency under retry: drop any staging table left behind from
    # a prior attempt that failed between `INSERT INTO staging` and
    # `EXCHANGE TABLES`. `EXCHANGE TABLES` would otherwise refuse to
    # run if the staging table is already present in `target_zone`.
    _ch_exec(target, f"DROP TABLE IF EXISTS {target_zone}.`{staging_table}`")
    _ch_exec(
        target,
        f"CREATE TABLE {target_zone}.`{staging_table}` "
        f"ENGINE = MergeTree ORDER BY tuple() AS {select_sql} LIMIT 0",
    )
    before = _ch_query_json(target, f"SELECT count() AS n FROM {target_zone}.`{target_table}`")
    _ch_exec(target, f"INSERT INTO {target_zone}.`{staging_table}` {select_sql}")
    # ClickHouse `EXCHANGE TABLES` is the atomic-swap primitive: both
    # tables must exist in the same database, and the swap is a single
    # mutation step (no transactional gap where readers see a
    # half-empty target).
    _ch_exec(
        target,
        f"EXCHANGE TABLES {target_zone}.`{target_table}` "
        f"AND {target_zone}.`{staging_table}`",
    )
    _ch_exec(target, f"DROP TABLE {target_zone}.`{staging_table}`")
    after = _ch_query_json(target, f"SELECT count() AS n FROM {target_zone}.`{target_table}`")
    return int(after[0]["n"]) - int(before[0]["n"])


def _op_for_pipeline(pipeline: dict[str, Any]) -> Any:
    definition = pipeline["definition"]
    pid = pipeline["id"]
    safe_name = _dagster_safe_name(pid)

    # Plan 1c (R2, day-1): each authored pipeline carries a per-row
    # `maxRetries` cap (`lakehouse-store::AuthoredDefinition.max_retries`,
    # `0051_pipeline_max_retries.sql`). Override ONLY the count of
    # `op_metadata.DEFAULT_RETRY_POLICY` — keep its `delay`, `backoff`,
    # and `jitter` so a synchronised retry storm across all authored
    # pipelines cannot return. An absent `maxRetries` (a future
    # caller that has not read the new key) resolves to 2 here, the
    # same value the migration's `DEFAULT 2` would have inserted at
    # the row's storage layer.
    base_retry = op_metadata.DEFAULT_RETRY_POLICY
    per_pipeline_retry = RetryPolicy(
        max_retries=int(definition.get("maxRetries", base_retry.max_retries)),
        delay=base_retry.delay,
        backoff=base_retry.backoff,
        jitter=base_retry.jitter,
    )

    @op(
        name=f"authored_{safe_name}",
        retry_policy=per_pipeline_retry,
        tags=op_metadata.source_metadata(
            "dispar_orchestrate/authored_factory.py::_op_for_pipeline",
            reads=[f"ClickHouse {definition.get('sourceZone')}.{definition.get('sourceTable')}"],
            writes=[f"ClickHouse {definition.get('targetZone')}.{definition.get('targetTable')}"],
        ),
    )
    def _run(context) -> dict[str, Any]:
        # PART D boundary: config-shaped failures raised by the
        # transform-grammar port (`TransformError`, a `ValueError`
        # subclass) and by this module's own authored-pipeline guard
        # (`AuthoredJobError`, a `RuntimeError` subclass) are wrapped in
        # `Failure(allow_retries=False)` here so `DEFAULT_RETRY_POLICY`
        # does not burn 60s on a config that will not change between
        # attempts. `SchemaDriftError` from `bronze_catalog.py` does
        # NOT reach this body (the `_ensure_target_table` path uses
        # `_ch_exec` directly, not `_assert_or_create_all`), so it is
        # not in the wrap tuple -- a SchemaDriftError raised elsewhere
        # would still propagate retryably as it always has. Runtime
        # errors from `_write_clickhouse_table` /
        # `requests.RequestException` from any future network call are
        # not in this tuple and propagate retryably, as the retry
        # policy intends.
        try:
            transforms = [
                authored_transforms.parse_transform(t)
                for t in definition.get("transforms", [])
            ]

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
                    raise AuthoredJobError(
                        f"unsafe ClickHouse identifier in {field!r}: {ident!r}"
                    )
        except (authored_transforms.TransformError, AuthoredJobError) as exc:
            raise Failure(
                description=f"authored pipeline {pipeline['id']!r} config rejected: {exc}",
                allow_retries=False,
            ) from exc

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
) -> tuple[
    list[Any],
    list[ScheduleDefinition],
    list[tuple[dict[str, Any], Any]],
]:
    """Every authored job, schedule, and (pipeline, job) tuple needed by the
    sensor builder, from ONE fetch of `/api/pipelines/runnable`, so the
    three lists can never describe different sets of pipelines.

    The `[(pipeline, job), ...]` half is what `build_authored_dependency_sensors`
    now consumes: the sensor body MUST use the SAME `JobDefinition` the
    code location's job list holds — building the job twice (once for
    `jobs`, once inside the sensor builder) produced two `JobDefinition`s
    with the same name, which `Dagster 1.13.20` refuses with
    `DagsterInvalidDefinitionError: Duplicate job definition found` and
    takes the whole code location load down with it (#55, F1.1 BLOCKER).

    The 3-tuple shape is the deliberate successor to the previous
    2-tuple shape (`(jobs, schedules)`). `build_authored_jobs` (HEAD's
    `... [0]`) keeps the 2-tuple contract by mapping to `[0]` of the new
    return; the old `test_jobs_and_schedules_come_from_one_fetch` test
    moves its asserts to `(jobs, schedules) == defs[0], defs[1]` and gains
    a fresh `(pipeline, job)` assert for the third element.

    Returns `(jobs, schedules, deps)` — `deps` is the sensor builder's
    input. With `PIPELINE_RUN_TOKEN` unset the fetch is a no-op and the
    three lists are empty."""
    cfg = cfg or AuthoredPipelineConfig.from_env()
    jobs: list[Any] = []
    schedules: list[ScheduleDefinition] = []
    deps: list[tuple[dict[str, Any], Any]] = []
    for pipeline in _fetch_authored_pipelines(cfg):
        authored_job = build_authored_job(pipeline)
        jobs.append(authored_job)
        schedule = build_authored_schedule(pipeline, authored_job)
        if schedule is not None:
            schedules.append(schedule)
        deps.append((pipeline, authored_job))
    return jobs, schedules, deps


def build_authored_dependency_sensors(
    pipeline_jobs: list[tuple[dict[str, Any], Any]],
) -> list[Any]:
    """R3 plan 2a: one `run_status_sensor` per authored pipeline with
    non-empty `depends_on`. Built from the `(pipeline, job)` list
    `build_authored_definitions` returned so each sensor's
    `request_job` is the SAME `JobDefinition` instance the code
    location's `jobs=` list holds (see `build_authored_definitions`'s
    docstring for the F1.1 BLOCKER this avoids). `run_jobs` does the
    HTTP fetch itself, so this function takes a plain list and is
    unit-testable without monkeypatching `requests`."""
    sensors: list[Any] = []
    for pipeline, authored_job in pipeline_jobs:
        sensor = build_authored_dependency_sensor(pipeline, authored_job)
        if sensor is not None:
            sensors.append(sensor)
    return sensors


def build_authored_jobs(cfg: AuthoredPipelineConfig | None = None) -> list[Any]:
    return build_authored_definitions(cfg)[0]


# R3 plan 2a: a run_status_sensor per authored pipeline with non-empty
# `depends_on`. Named `authored__<safe_id>_after` so the `job_name`/
# `schedule_name` sanitize rule (`_dagster_safe_name`) matches what the
# Rust API layer produces (see `routes::authored_pipelines::sensor_name`
# in `lakehouse-api`, used by `GET /api/pipelines/{id}/schedule-ticks`
# when fetching this sensor's ticks).
#
# Default status RUNNING — a sensor that ships STOPPED silently never
# fires; if the author did not want chains to trigger they would have
# left `depends_on` empty (and this function would not have been called
# for this row). DAGSTER_REPO and DAGSTER_LOCATION match
# `DgClient::with_repository`'s defaults so the selector targets the
# same code location the API's launches go to.
DAGSTER_REPO = "__repository__"
DAGSTER_LOCATION = "dispar_orchestrate.definitions"


def _upstream_job_selector(upstream_id: str) -> JobSelector:
    """Map an upstream id to a `JobSelector` for the `monitored_jobs`
    list. Authored ids become the safe job name the factory gave them
    (`authored__<safe_id>`); Dagster-native ids are used verbatim. R3
    plan 2a."""
    job_name = f"authored__{_dagster_safe_name(upstream_id)}" if upstream_id.startswith("pl-") else upstream_id
    return JobSelector(
        location_name=DAGSTER_LOCATION,
        repository_name=DAGSTER_REPO,
        job_name=job_name,
    )


def build_authored_dependency_sensor(
    pipeline: dict[str, Any], authored_job: Any
) -> Any | None:
    """One `run_status_sensor` for `pipeline`, or `None` when it has no
    `dependsOn`. Reads `dependsOn` (camelCase), matching the wire
    format the Rust `RunnablePipeline` serializes under
    `#[serde(rename_all = "camelCase")]` — R3 plan 2a wire-format fix.
    The sensor watches every upstream's SUCCESS run; on a firing tick
    it computes "has every upstream had a SUCCESS that finished after
    this pipeline's most-recent start?" -- ALL semantics (R3 plan 2a).
    When yes, yield `RunRequest(run_key=<upstream run id>)` so
    Dagster's own dedup keeps a re-firing upstream from launching the
    downstream twice for the same upstream run; when no, yield a
    `SkipReason` naming the upstream that is still behind.

    `monitored_jobs` accepts `JobSelector`s (for cross-location
    upstreams like `ingest_job`); `request_job` must be the
    `JobDefinition` this code location built -- Dagster 1.13.20's
    `SensorDefinition.__init__` coerces `request_job` through
    `AutomationTarget.from_coercible`, which only handles
    `JobDefinition`/`UnresolvedAssetJobDefinition` (not selectors).
    Verified against dagster 1.13.20's `run_status_sensor` signature.
    """
    depends_on = pipeline.get("dependsOn") or []
    if not depends_on:
        return None
    # F1.2 SHOULD-FIX (#57): a paused pipeline must NOT get a chain sensor.
    # The orchestrator keeps the sensor's RUNNING/STOPPED state across a
    # reload (Dagster keys instigator state by a name-based `selector_id`
    # in `sql_schedule_storage.py:110-128`, and the daemon never stops
    # states that disappeared from code). Pausing a chained pipeline
    # therefore meant a sensor shipped RUNNING continued to fire; the
    # `routes::pipelines::authored_status` toggle stops the sensor at
    # pause time so the chain truly goes quiet (the same toggle that
    # stops the schedule). A `ready` pipeline re-enters `default_status
    # =RUNNING` after a reload; the API path that flips `paused` ->
    # `ready` also re-starts the sensor in the same toggle.
    if pipeline.get("status") == "paused":
        return None
    monitored = [_upstream_job_selector(dep) for dep in depends_on]
    downstream_safe = _dagster_safe_name(pipeline["id"])
    upstream_ids = list(depends_on)

    @run_status_sensor(
        run_status=DagsterRunStatus.SUCCESS,
        name=f"authored__{downstream_safe}_after",
        monitored_jobs=monitored,
        request_job=authored_job,
        default_status=DefaultSensorStatus.RUNNING,
        description=(
            f"Triggers pipeline {pipeline['id']!r} when every upstream in "
            f"{upstream_ids!r} has had a SUCCESS run after this pipeline's "
            "most-recent start (R3 plan 2a)."
        ),
    )
    def _dependency_sensor(context: Any) -> Any:
        # `context.dagster_run` is the upstream run that triggered THIS
        # tick -- it is always SUCCESS here (`run_status=SUCCESS` on the
        # decorator).
        upstream_run = context.dagster_run
        # The downstream's most-recent run, regardless of outcome:
        # what we need to beat is "started after this point in time".
        # F1.3 (#57): a downstream with no `start_time` (its latest run
        # is QUEUED / NOT_STARTED / STARTED-but-empty) is NOT the same
        # as "no run yet" -- the queued run is the chain's previous
        # launch and we MUST wait for it before re-firing. The walk
        # below checks every upstream against either the downstream's
        # `start_time` (when it has one) or its `creation_time` (when
        # the latest run has not started yet) -- never "no comparison
        # point", which was the buggy "first-success fires the chain"
        # behaviour the old code shipped.
        downstream_runs = context.instance.get_runs(
            filters=RunsFilter(job_name=f"authored__{downstream_safe}"),
            limit=1,
        )
        latest_downstream = downstream_runs[0] if downstream_runs else None
        # F1.3 (#57): "downstream has a queued run" is the
        # `latest_downstream is not None and latest_downstream.start_time
        # is None` branch -- the chain's previous launch is still in
        # flight, so we MUST NOT yield a fresh RunRequest until the
        # queued run actually starts (and beats its upstream checks).
        if latest_downstream is not None and latest_downstream.start_time is None:
            yield SkipReason(
                f"downstream has a queued run ({latest_downstream.run_id}); "
                "skip until it starts so we do not double-launch"
            )
            return
        # The freshness anchor: every upstream must have a SUCCESS run
        # that finished AFTER this point. `start_time` when the
        # downstream has run; `None` when it has never run (the
        # "first-ever" case where the old code already let the chain
        # fire without waiting -- we keep that branch but ALSO require
        # EVERY upstream to have had a SUCCESS, see below).
        downstream_start = (
            latest_downstream.start_time if latest_downstream is not None else None
        )
        # F1.3 (#57): every upstream must have a SUCCESS run, not
        # "any one upstream has a SUCCESS run". Walking every upstream
        # in `upstream_ids` and asking `instance.get_runs` with a
        # SUCCESS filter is what catches the "one upstream never
        # succeeded, but the others have" case the old code missed --
        # yielding a SkipReason that names the upstream that is still
        # behind. The first stale upstream encountered is the one we
        # name in the skip reason; the walk stops there to keep the
        # per-tick work bounded by the number of upstreams.
        #
        # `run_key` is derived from the SORTED set of the upstreams'
        # latest SUCCESS run ids, so one round of upstream successes
        # requests exactly ONE downstream run, not one per upstream:
        # two upstreams that succeed in the same tick (each firing
        # its own sensor tick) ask the daemon for the same
        # `run_key`, and the daemon's own dedup drops the second.
        latest_success_ids: list[str] = []
        for upstream_id in upstream_ids:
            upstream_job_name = (
                f"authored__{_dagster_safe_name(upstream_id)}"
                if upstream_id.startswith("pl-")
                else upstream_id
            )
            upstream_runs = context.instance.get_runs(
                filters=RunsFilter(
                    job_name=upstream_job_name,
                    statuses=[DagsterRunStatus.SUCCESS],
                ),
                limit=1,
            )
            latest = upstream_runs[0] if upstream_runs else None
            if latest is None or latest.end_time is None:
                yield SkipReason(
                    f"upstream {upstream_id!r} has never had a SUCCESS run; "
                    f"this upstream run ({upstream_run.run_id}) is not enough"
                )
                return
            if latest.end_time <= (downstream_start or 0.0):
                yield SkipReason(
                    f"upstream {upstream_id!r} has no SUCCESS run after the "
                    f"downstream's most-recent start ({downstream_start}); "
                    f"this upstream run ({upstream_run.run_id}) is stale"
                )
                return
            latest_success_ids.append(latest.run_id)
        # ALL upstreams have a fresh SUCCESS run -> one RunRequest,
        # keyed on the SORTED tuple of those upstream run ids. The
        # `run_key` is a string the daemon dedups, so two ticks that
        # see the same set of upstreams ask for the same downstream
        # launch exactly once.
        joined = ",".join(sorted(latest_success_ids))
        yield RunRequest(run_key=f"authored-deps:{joined}")

    return _dependency_sensor


# Built at Dagster code-load time, same pattern `agent_run_schedules`
# (`agent_runs.py`) uses -- see `_fetch_authored_pipelines`'s doc comment
# for every way this degrades to empty lists without raising.
# Three plain assignments rather than tuple unpacking, so
# `ops/lint/check_intra_package_imports.py` (which reads module-level
# names statically) sees all four names `definitions.py` imports.
_authored_jobs_and_schedules_and_deps = build_authored_definitions()
authored_jobs = _authored_jobs_and_schedules_and_deps[0]
authored_schedules = _authored_jobs_and_schedules_and_deps[1]
_authored_pipeline_jobs = _authored_jobs_and_schedules_and_deps[2]
authored_dependency_sensors = build_authored_dependency_sensors(_authored_pipeline_jobs)
