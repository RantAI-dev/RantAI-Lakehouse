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
  schedule (F1.2);
* an `authored__<id>_lag` sensor when the pipeline has a time-based target lag
  (`targetLag.kind == 'lag'`) AND its status is `ready` (BLD-2 D3/D6). A target lag
  replaces both the cron schedule and the dependency `_after` sensor while set (D3).
  The sensor evaluates `POST /api/pipelines/runnable/{id}/lag-check` every 30 seconds
  (`minimum_interval_seconds=30`), balancing responsive freshness for pipelines with short
  targets (down to 2 minutes) against the database cost of querying input snapshot tokens
  and ClickHouse content fingerprints across authored pipelines (D6).

  One upstream is per-table rather than per-job: `file_ingest_job` loads
  ONE uploaded file into ONE Bronze table per run, so a pipeline that
  names it counts only the SUCCESS runs whose own config loaded the
  pipeline's source table (DATA-19 / D1). A load of another table is not
  a run of this chain.

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

# Reading a Bronze source

A Bronze table is Iceberg in Lakekeeper, not a ClickHouse database: this
stack's ClickHouse has `serving`, `silver` and `lake` only, so a Bronze
source cannot be read as `` bronze.`orders` ``. It is read through a
`DataLakeCatalog` database, where the table is named ``bronze.<table>``,
exactly as `connector_catalog.register_loaded_table` and
`maintenance.discover_bronze_tables` already read one. `_resolve_source`
picks the source and `_reading` attaches `allow_database_iceberg = 1` to
every statement that reads it; `_resolve_source`'s own docstring and
`_reading`'s carry the details.

BLD-1 / D5 changed WHICH database that is, and the reason is governance,
not convenience. The `SELECT` a pipeline runs is rewritten by the API
(`_enforced_sql`, below) through the same masking and row-filter rewrite
every governed surface uses, and that rewriter only recognises a governed
table under the database the API itself reads it from -- its own
`ICEBERG_QUERY_DB` (`icecat_api` in compose). Reading Bronze through this
module's former private `icecat_authored` database made the rewriter see an
unknown table, so it enforced nothing on exactly the tables that most need
it. So the source is now named `ICEBERG_QUERY_DB`.`bronze.<table>` and this
module no longer creates a catalog database of its own: the
`clickhouse-iceberg-init` compose service already creates that one, on every
`up`, and a second `CREATE DATABASE ... DataLakeCatalog` here would be a
duplicate of it that could drift.

# Reading as the pipeline's author

BLD-1 / D4: the `SELECT` this module builds is not the `SELECT` it runs.
`_enforced_sql` posts it to the API, which rewrites it as the pipeline's
recorded author (`pipeline_definition.run_as`) and returns the SQL to run;
only that returned text reaches ClickHouse. A pipeline with no recorded
author, or one whose author has been deactivated, is refused with a fixed
message (409) that no retry can fix; a refused construct is refused the
same way (422). A transport failure still propagates, because that IS
retryable and used to be.
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from typing import Any
from urllib.parse import quote

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
    sensor,
)

from dispar_orchestrate import authored_transforms, op_metadata
from dispar_orchestrate.bronze_catalog import ClickHouseTarget, _ch_exec, _ch_query_json
from dispar_orchestrate.file_ingest import JOB_NAME as FILE_INGEST_JOB


def _env(name: str, default: str) -> str:
    value = os.environ.get(name, "").strip()
    return value if value else default


@dataclass(frozen=True)
class AuthoredPipelineConfig:
    api_url: str
    run_token: str
    # BLD-1 / D5: the API's own `DataLakeCatalog` database. A Bronze
    # source is read through THIS one, not through a database private to
    # this module, because the API's policy rewriter only recognises a
    # governed table under the database it reads it from -- named any
    # other way, `SELECT * FROM <other-db>.`bronze.t`` reaches ClickHouse
    # unmasked. Defaults to the compose default; `lakehouse-api` and
    # `clickhouse-iceberg-init` read the same variable, so one `.env`
    # value moves all three together.
    iceberg_query_db: str = "icecat_api"

    @classmethod
    def from_env(cls) -> "AuthoredPipelineConfig":
        return cls(
            api_url=_env("LAKEHOUSE_API_URL", "http://lakehouse-api:8080"),
            run_token=_env("PIPELINE_RUN_TOKEN", ""),
            iceberg_query_db=_env("ICEBERG_QUERY_DB", "icecat_api"),
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

BRONZE_TARGET_REFUSED_REASON = (
    "a pipeline cannot write to bronze; bronze is written by ingestion"
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


# ── Where a pipeline reads FROM: BLD-3 / D1, D2 ────────────────────────

# Bronze is not a ClickHouse database on this stack: `scripts/seed-clickhouse.sql`
# creates `serving`, `silver` and `lake` only. A Bronze table is Iceberg in
# Lakekeeper and is readable from ClickHouse ONLY through a
# `DataLakeCatalog` database, where the table's name is the catalog's own
# flat `bronze.<table>` (ADR 0004) -- `` <catalog db>.`bronze.<table>` ``.
# Reading it as a plain `` bronze.`orders` `` is the 404 this module's
# Bronze source used to raise (plan `2026-10-14-bld-3-bronze-pipeline-source`,
# T1's reproduced trace).
#
# WHICH catalog database is BLD-1 / D5's change, and it is a governance
# decision rather than tidiness: this module used to own a private
# `icecat_authored` and create it here, the way every other job owns its
# own (`icecat_maintenance`, `icecat_ingest`, `ops/g3a`'s `icecat_g3a`), so
# a pipeline never depended on another job having created one first. That
# independence now costs more than it buys: the API's policy rewriter
# resolves a governed table by its qualified name under the database the
# API itself reads it from (`ICEBERG_QUERY_DB`), so `SELECT`ting through a
# private database produced SQL the rewriter did not recognise, and the
# masking a pipeline is supposed to inherit never applied to Bronze -- the
# tables that hold the rawest personal data. So the name comes from
# `AuthoredPipelineConfig.iceberg_query_db` (the API's own `ICEBERG_QUERY_DB`,
# the same one `lakehouse-api` and `clickhouse-iceberg-init` use), and this
# module creates nothing: `clickhouse-iceberg-init` already creates that
# database on every `up`, so a `CREATE DATABASE` here would be a second
# copy of a statement that already exists and could drift from it.

# Every statement that READS the catalog database needs this setting;
# ClickHouse refuses an Iceberg read without it. `maintenance.py` sends it
# on `SHOW TABLES` (`SETTINGS allow_database_iceberg=1`) and on its own
# `CREATE DATABASE ... DataLakeCatalog`, which is the precedent this
# module follows. It is attached per statement rather than as a session
# setting because `_ch_exec` is a stateless `requests.post` per statement:
# there is no session to set it on.
ICEBERG_READ_SETTINGS = "allow_database_iceberg = 1"


@dataclass(frozen=True)
class _PipelineSource:
    """One authored pipeline's ClickHouse source: the name to read, and
    whether reading it is an Iceberg read (which decides whether the
    statements that touch it carry `ICEBERG_READ_SETTINGS`).

    `clickhouse_name` is a ClickHouse identifier expression, never a bare
    zone: for a Bronze source it is the catalog database plus the
    catalog's own ``bronze.<table>`` name inside one pair of backticks.
    """
    clickhouse_name: str
    reads_iceberg: bool = False


def _resolve_source(
    source_zone: str, source_table: str, catalog_db: str
) -> _PipelineSource:
    """The source an authored pipeline reads from, and whether reading it
    is an Iceberg read (which decides whether the statements that touch it
    carry `ICEBERG_READ_SETTINGS`).

    `silver`/`gold` (and every other ClickHouse zone) are read as
    `<zone>.<table>`, exactly as before. A `bronze` source is read through
    the API's own catalog database -- `catalog_db`, the caller's
    `ICEBERG_QUERY_DB` -- where the table is ``bronze.<table>``. See the
    catalog-database comment above for why that is the API's database and
    not one of this module's own, and why nothing is created here any more.

    # Errors

    None. Every value it returns is a name this module's own identifier
    check (`_is_safe_ch_identifier`, applied to the zone and table in
    `_op_for_pipeline`) has already vouched for.
    """
    if source_zone == "bronze":
        return _PipelineSource(
            f"{catalog_db}.`bronze.{source_table}`", reads_iceberg=True
        )
    return _PipelineSource(f"{source_zone}.`{source_table}`")


def _reading(source: _PipelineSource, statement: str) -> str:
    """`statement`, plus `SETTINGS allow_database_iceberg = 1` when it
    reads the catalog database (D2). ClickHouse reads `SETTINGS` as the
    last clause of the `SELECT` it belongs to, so this is appended to the
    WHOLE statement -- after its `LIMIT 0`, after its `FORMAT`, never in
    the middle of a `FROM`."""
    if not source.reads_iceberg:
        return statement
    return f"{statement} SETTINGS {ICEBERG_READ_SETTINGS}"


# The API's answer to "I will not run this SELECT for you, and running it
# again will not change my mind": 409 when the pipeline has no author to
# read as (BLD-1 / D2) or that author is gone, 422 when the SQL touches a
# governed table through a construct the policy engine will not rewrite.
# Both carry a FIXED message (`policy_engine::enforcement_error_message` /
# D2's own string), which is why `_enforced_sql` can put it in an op
# failure without leaking anything: there is no upstream text in it to
# leak.
_ENFORCE_REFUSAL_STATUSES = (409, 422)

# Fallback when the API refuses but its body carries no `error` string. A
# fixed message of our own, never the raw body: an unexpected body is
# exactly the case where its text is least trustworthy.
_ENFORCE_REFUSAL_FALLBACK = (
    "the lakehouse API refused to enforce this pipeline's query; "
    "see the API logs for the reason"
)

# Fixed text for a 2xx whose body is not the `{ "sql": "..." }` shape the
# route documents. Retrying the identical request cannot repair a response
# shape, so this is a refusal rather than a transient failure -- but it is
# OURS, not the API's, and it says nothing about what came back.
_ENFORCE_BAD_BODY = (
    "the lakehouse API returned an unreadable enforcement response for "
    "this pipeline's query"
)


class AuthoredEnforceRefused(RuntimeError):
    """The API refused a pipeline's `SELECT` in a way no retry can fix.

    A `RuntimeError` subclass, like [`AuthoredJobError`], so `_op_for_pipeline`
    wraps it in `Failure(allow_retries=False)` instead of spending the retry
    policy on a decision that will be identical on every attempt.
    """


def _refusal_message(resp: requests.Response) -> str:
    """The API's own fixed refusal text, or a fixed fallback of ours.

    Never echoes an arbitrary body: `error` is used only when it is a
    non-empty string, and everything else falls back. The body of a
    refusal is classified text by construction (every `ApiError` variant
    carries a message the API chose), which is what makes it safe to put
    in an op failure description at all.
    """
    try:
        body = resp.json()
    except ValueError:
        return _ENFORCE_REFUSAL_FALLBACK
    if isinstance(body, dict):
        message = body.get("error")
        if isinstance(message, str) and message.strip():
            return message
    return _ENFORCE_REFUSAL_FALLBACK


def _enforced_sql(cfg: AuthoredPipelineConfig, pipeline_id: str, select_sql: str) -> str:
    """The SQL to RUN for `select_sql`: the API's rewrite of it as the
    pipeline's recorded author, or [`AuthoredEnforceRefused`].

    This is BLD-1 / D4's whole point, and the reason the returned text --
    never `select_sql` itself -- is what `_write_clickhouse_table` writes
    into the `CREATE TABLE ... AS`, the staging `INSERT` and every other
    statement that reads the source. The API applies the same masking and
    row-filter rewrite Query Studio, dashboards, alerts and the Gold export
    apply, so a pipeline cannot be used to copy a masked column into a
    Silver table in clear (`GOV-4`).

    # Errors

    [`AuthoredEnforceRefused`] when the API refuses (409/422) or answers
    with a body that is not `{ "sql": ... }` -- neither of which a retry
    changes, so the op fails without retrying.

    `requests.RequestException` propagates UNWRAPPED, and the op retries:
    an unreachable API, a timeout or a 5xx are the infrastructure failures
    the retry policy exists for, and they are exactly as retryable as they
    were before this call existed.
    """
    resp = requests.post(
        f"{cfg.api_url}/api/pipelines/runnable/{quote(pipeline_id, safe='')}/enforce",
        headers={**_headers(cfg), "content-type": "application/json"},
        json={"sql": select_sql},
        timeout=60,
    )
    if resp.status_code in _ENFORCE_REFUSAL_STATUSES:
        raise AuthoredEnforceRefused(_refusal_message(resp))
    # Any other non-2xx (401 a token that is not the run identity, 404 a
    # pipeline that vanished without a reload, 5xx) raises here and is
    # retried, as before this call existed.
    resp.raise_for_status()
    try:
        body = resp.json()
    except ValueError as exc:
        raise AuthoredEnforceRefused(_ENFORCE_BAD_BODY) from exc
    if not isinstance(body, dict) or not isinstance(body.get("sql"), str):
        raise AuthoredEnforceRefused(_ENFORCE_BAD_BODY)
    return body["sql"]


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


def _ensure_target_table(
    target: ClickHouseTarget,
    target_zone: str,
    target_table: str,
    select_sql: str,
    source: _PipelineSource,
) -> None:
    """Schema-on-write for an authored pipeline's target: create the
    database and (if it does not already exist) the table, inferring
    columns from `select_sql` via ClickHouse's own `CREATE TABLE ... AS
    SELECT ...` (`LIMIT 0` so this never inserts rows itself -- inserting
    is `_write_clickhouse_table`'s job, run every time this job runs, not
    only on first creation).

    `SELECT ... AS ... LIMIT 0` is the FIRST statement that reads a Bronze
    source, so it carries `source`'s read setting (`_reading`); the
    `CREATE DATABASE` above it writes only and never carries one."""
    _ch_exec(target, f"CREATE DATABASE IF NOT EXISTS {target_zone}")
    _ch_exec(
        target,
        _reading(
            source,
            f"CREATE TABLE IF NOT EXISTS {target_zone}.`{target_table}` "
            f"ENGINE = MergeTree ORDER BY tuple() AS {select_sql} LIMIT 0",
        ),
    )


def _write_clickhouse_table(
    target: ClickHouseTarget,
    target_zone: str,
    target_table: str,
    select_sql: str,
    source: _PipelineSource,
) -> int:
    """REPLACE `target_zone.target_table` with `select_sql`'s rows,
    atomically, and return the REAL row-count DELTA between before and
    after the swap -- what the caller records as rows written -- not
    the absolute row count of the swapped-in target.

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
    _ensure_target_table(target, target_zone, target_table, select_sql, source)
    staging_table = f"__staging_{target_table}"
    # Idempotency under retry: drop any staging table left behind from
    # a prior attempt that failed between `INSERT INTO staging` and
    # `EXCHANGE TABLES`. `EXCHANGE TABLES` would otherwise refuse to
    # run if the staging table is already present in `target_zone`.
    _ch_exec(target, f"DROP TABLE IF EXISTS {target_zone}.`{staging_table}`")
    _ch_exec(
        target,
        _reading(
            source,
            f"CREATE TABLE {target_zone}.`{staging_table}` "
            f"ENGINE = MergeTree ORDER BY tuple() AS {select_sql} LIMIT 0",
        ),
    )
    before = _ch_query_json(target, f"SELECT count() AS n FROM {target_zone}.`{target_table}`")
    _ch_exec(target, _reading(source, f"INSERT INTO {target_zone}.`{staging_table}` {select_sql}"))
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
            # BLD-3 / D4: Bronze is written by ingestion, never by a
            # pipeline. Accepting a `bronze` target here would run
            # `CREATE DATABASE IF NOT EXISTS bronze` in ClickHouse and
            # fill a MergeTree table that is not Bronze (it is not
            # Iceberg, it is not in the catalog, nothing reads it) --
            # a silent wrong answer. Refused here, in the API on
            # create/update/restore and in the assistant's
            # `create_pipeline` schema, with this same wording. A stored
            # pipeline that predates the refusal now fails its run with
            # this message instead of writing somewhere meaningless.
            if target_zone == "bronze":
                raise AuthoredJobError(BRONZE_TARGET_REFUSED_REASON)
        except (authored_transforms.TransformError, AuthoredJobError) as exc:
            raise Failure(
                description=f"authored pipeline {pipeline['id']!r} config rejected: {exc}",
                allow_retries=False,
            ) from exc

        ch = ClickHouseTarget.from_env()
        cfg = AuthoredPipelineConfig.from_env()
        source = _resolve_source(source_zone, source_table, cfg.iceberg_query_db)
        select_sql = _build_select_sql(source.clickhouse_name, transforms)
        # BLD-1 / D4: what we BUILT is not what we RUN. The API rewrites
        # the SELECT as the pipeline's recorded author, and only the SQL it
        # returns reaches ClickHouse -- through `_write_clickhouse_table`,
        # which uses this one value for the `CREATE TABLE ... AS`, the
        # staging INSERT and every other statement that reads the source.
        # A refusal (no author, or a construct the policy engine will not
        # rewrite) is not retried: the same pipeline asked again minutes
        # later gets the same answer. A transport failure is not caught
        # here at all, so it stays retryable exactly as it always was.
        try:
            enforced_sql = _enforced_sql(cfg, pipeline["id"], select_sql)
        except AuthoredEnforceRefused as exc:
            raise Failure(
                description=f"authored pipeline {pipeline['id']!r} refused: {exc}",
                allow_retries=False,
            ) from exc
        written = _write_clickhouse_table(ch, target_zone, target_table, enforced_sql, source)

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
    # BLD-2 D3: A target lag (either kind) replaces the cron schedule while it is set.
    # The schedule string stays stored, so clearing the target lag restores the schedule (AC8).
    if pipeline.get("targetLag") is not None:
        return None
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


# ── DATA-19 / D1: a re-upload triggers only the pipelines that read it ────

# Where a load's run config says which table it wrote:
# `ops.ingest_uploaded_file.config.bronze_table_name`, exactly the shape
# `routes::uploads::run_config` launches `file_ingest_job` with. The op
# NAME is the key, not the job name, so a rename of the job would not
# silently change the filter into a match-nothing one.
FILE_INGEST_OP_CONFIG = ("ops", "ingest_uploaded_file", "config", "bronze_table_name")

# How many recent SUCCESS runs of `file_ingest_job` one tick looks at when
# it has to find the newest one that loaded THIS pipeline's table. The
# window is bounded for the same reason `MAX_DEPENDS_ON` is: a per-tick
# scan over an unbounded run history would grow with every upload anyone
# ever made. It is a window, not a cap on what triggers the chain -- a
# table whose newest load is older than the window simply waits for its
# next upload, and the SkipReason says exactly that.
FILE_INGEST_SCAN_LIMIT = 20


def _run_config_value(run: Any, path: tuple[str, ...]) -> str | None:
    """The string at `path` inside a run's `run_config`, or `None`.

    `DagsterRun.run_config` is the run's RESOLVED config, so a launch's
    `{"ops": {"ingest_uploaded_file": {"config": {...}}}}` comes back with
    the same shape and its op entries merged with the op's own defaults --
    which is why the launch needs no extra tag for this to be readable
    (`DATA-19` plan D1). Anything that is not that shape -- a run launched
    without a config, a config whose shape Dagster rewrote -- answers
    `None`, and the caller treats that as "this run is not about our
    table", never as a match.
    """
    value: Any = getattr(run, "run_config", None)
    for key in path:
        if not isinstance(value, dict) or key not in value:
            return None
        value = value[key]
    return value if isinstance(value, str) and value else None


def _latest_record_for_table(records: Sequence[Any], table: str) -> Any | None:
    """The newest of `records` whose run loaded `table`, or `None`.

    `instance.get_run_records` returns the run storage's default order --
    the `runs.id` column descending, an autoincrement integer, so newest
    first on every storage this build runs on -- and this is the first
    match. An empty `table` matches nothing: a pipeline whose source zone
    is not `bronze` has no Bronze table to filter on, and fail-closed
    here means its chain waits with an honest reason instead of firing on
    every upload of every table.
    """
    if not table:
        return None
    for record in records:
        if _run_config_value(record.dagster_run, FILE_INGEST_OP_CONFIG) == table:
            return record
    return None


def build_authored_dependency_sensor(
    pipeline: dict[str, Any], authored_job: Any
) -> Any | None:
    """One `run_status_sensor` for `pipeline`, or `None` when it has no
    `dependsOn`. Reads `dependsOn` (camelCase), matching the wire
    format the Rust `RunnablePipeline` serializes under
    `#[serde(rename_all = "camelCase")]` — R3 plan 2a wire-format fix.
    A `paused` pipeline gets no sensor at all (F1.2, #57): the
    orchestrator keeps a vanished sensor's RUNNING state across a
    reload, so a paused chain has to be stopped rather than rebuilt.

    The sensor watches every upstream's SUCCESS run. On each firing
    tick it requires ALL upstreams to have a SUCCESS run that ended
    after the downstream's most-recent run started -- or simply a
    SUCCESS run when the downstream has never run (R3 plan 2a ALL
    semantics). The downstream's latest run is the freshness anchor;
    while that run has no `start_time` (queued / not started /
    started-but-empty) the sensor skips, so a queued launch is never
    double-fired. When an upstream is behind, the sensor yields a
    `SkipReason` naming it, stopping the walk at the first one to keep
    the per-tick work bounded by the number of upstreams.

    When every upstream is fresh, the sensor yields ONE
    `RunRequest(run_key=f"authored-deps:{sorted latest upstream SUCCESS
    run ids}")` per round of upstream successes, not one per upstream.
    Two upstreams that succeed in the same tick ask for the same
    sorted key, so Dagster's own dedup drops the second and the
    downstream launches once (R3 plan 2a).

    `monitored_jobs` accepts `JobSelector`s (for cross-location
    upstreams like `ingest_job`); `request_job` must be the
    `JobDefinition` this code location built -- Dagster 1.13.20's
    `SensorDefinition.__init__` coerces `request_job` through
    `AutomationTarget.from_coercible`, which only handles
    `JobDefinition`/`UnresolvedAssetJobDefinition` (not selectors).
    Verified against dagster 1.13.20's `run_status_sensor` signature.

    `file_ingest_job` is the one upstream the freshness walk filters
    (DATA-19 / D1): it is a per-table job, so a pipeline counts only the
    SUCCESS runs whose own config says they loaded ITS source table, and a
    load of any other table never triggers it. Every other upstream keeps
    the unfiltered rule above, byte for byte.
    """
    # BLD-2 D3: A target lag (either kind) replaces the dependency (_after) sensor while it is set.
    # The dependsOn list stays stored, so clearing the target lag restores the chain sensor (AC8).
    if pipeline.get("targetLag") is not None:
        return None
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
    # DATA-19 / D1: the Bronze table whose re-upload may trigger THIS
    # pipeline, and `None` for every other shape. A `file_ingest_job`
    # upstream only ever means "run me again when this table is
    # re-uploaded", and only a `bronze` source names such a table -- the
    # zone an upload's load writes (`routes::uploads::run_config`). A
    # pipeline whose source is `silver`/`gold`/... has no Bronze table to
    # match, so its filtered upstream matches nothing and the chain waits
    # with a reason rather than firing on every upload. The API refuses
    # that combination on create/update/restore
    # (`routes::authored_pipelines::validate_file_ingest_upstream`); this
    # is the same rule applied to a stored row that predates it.
    definition = pipeline.get("definition") or {}
    source_zone = definition.get("sourceZone")
    source_table = definition.get("sourceTable")
    bronze_table = source_table if source_zone == "bronze" else None
    source_label = (
        f"{source_zone}.{source_table}"
        if source_zone and source_table
        else "a source that names no bronze table"
    )

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
        # Why RECORDS and not runs (`DATA-19` gate finding): in Dagster
        # 1.13.20 a run's times live on the storage record
        # (`RunRecord.start_time` / `.end_time`), NOT on the `DagsterRun`
        # that `instance.get_runs` returns -- `DagsterRun.__new__` takes
        # no `start_time` at all. Reading it off the run raised
        # `AttributeError: 'DagsterRun' object has no attribute
        # 'start_time'` on EVERY tick of a real chain, so the chain never
        # fired; `ops/g9`'s step 10 is what caught it, from the sensor
        # tick's own FAILURE. `R3 plan 2a`'s unit tests could not: their
        # stub defined the attributes the storage no longer puts there.
        downstream_records = context.instance.get_run_records(
            filters=RunsFilter(job_name=f"authored__{downstream_safe}"),
            limit=1,
        )
        latest_downstream = downstream_records[0] if downstream_records else None
        # F1.3 (#57): "downstream has a queued run" is the
        # `latest_downstream is not None and latest_downstream.start_time
        # is None` branch -- the chain's previous launch is still in
        # flight, so we MUST NOT yield a fresh RunRequest until the
        # queued run actually starts (and beats its upstream checks).
        if latest_downstream is not None and latest_downstream.start_time is None:
            yield SkipReason(
                f"downstream has a queued run ({latest_downstream.dagster_run.run_id}); "
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
            # DATA-19 / D1: `file_ingest_job` is the one upstream whose
            # runs are about ONE table each, so its newest run is not
            # enough -- the newest run THAT LOADED THIS PIPELINE'S TABLE
            # is. Everything else asks for one run exactly as before.
            filtered = upstream_id == FILE_INGEST_JOB
            upstream_records = context.instance.get_run_records(
                filters=RunsFilter(
                    job_name=upstream_job_name,
                    statuses=[DagsterRunStatus.SUCCESS],
                ),
                limit=FILE_INGEST_SCAN_LIMIT if filtered else 1,
            )
            latest = (
                _latest_record_for_table(upstream_records, bronze_table or "")
                if filtered
                else (upstream_records[0] if upstream_records else None)
            )
            if latest is None or latest.end_time is None:
                if filtered:
                    if not bronze_table:
                        yield SkipReason(
                            f"upstream {upstream_id!r} loads uploaded files into bronze "
                            f"tables and this pipeline reads {source_label}, so no upload "
                            "can trigger it"
                        )
                    else:
                        yield SkipReason(
                            f"upstream {upstream_id!r} has no SUCCESS run that loaded "
                            f"{bronze_table!r} in its last {FILE_INGEST_SCAN_LIMIT} runs; "
                            "a load of another table never triggers this pipeline"
                        )
                    return
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
            latest_success_ids.append(latest.dagster_run.run_id)
        # ALL upstreams have a fresh SUCCESS run -> one RunRequest,
        # keyed on the SORTED tuple of those upstream run ids. The
        # `run_key` is a string the daemon dedups, so two ticks that
        # see the same set of upstreams ask for the same downstream
        # launch exactly once.
        joined = ",".join(sorted(latest_success_ids))
        yield RunRequest(run_key=f"authored-deps:{joined}")

    return _dependency_sensor


def lag_sensor_name(pipeline_id: str) -> str:
    """BLD-2 review R1 SHOULD-FIX 14: build the sensor name with the same helper as the factory."""
    safe_id = _dagster_safe_name(pipeline_id)
    return f"authored__{safe_id}_lag"


def build_authored_lag_sensor(
    pipeline: dict[str, Any],
    authored_job: Any,
    allowed_jobs: list[Any] | None = None,
    cfg: AuthoredPipelineConfig | None = None,
) -> Any | None:
    """BLD-2 D6: one sensor per authored pipeline with a time-based target lag
    (`targetLag.kind == 'lag'`). Downstream lag pipelines (`targetLag.kind == 'downstream'`)
    do not poll: their refresh is requested when an upstream pipeline's check runs.

    A `paused` pipeline gets no sensor at all (as with the dependency sensor, F1.2 #57):
    the orchestrator preserves a sensor's RUNNING state across reloads, so a paused
    pipeline must not continue checking lag in the background.

    `minimum_interval_seconds=30`: checks run at most every 30 seconds to balance
    freshness responsiveness for short lag targets against the query cost of input tokens
    and ClickHouse content fingerprints across authored pipelines.
    """
    target_lag = pipeline.get("targetLag")
    if not isinstance(target_lag, dict) or target_lag.get("kind") != "lag":
        return None

    if pipeline.get("status") == "paused":
        return None

    pipeline_id = pipeline.get("id", "")
    sensor_name = lag_sensor_name(pipeline_id)

    # BLD-2 review R1 BLOCKER 2: give the sensor every job it may name
    jobs_to_attach = list(allowed_jobs) if allowed_jobs else [authored_job]
    if authored_job not in jobs_to_attach:
        jobs_to_attach.insert(0, authored_job)
    allowed_job_names = {j.name for j in jobs_to_attach}

    @sensor(
        name=sensor_name,
        jobs=jobs_to_attach,
        minimum_interval_seconds=30,
        default_status=DefaultSensorStatus.RUNNING,
    )
    def _lag_sensor(context: Any) -> Any:
        cfg_ = cfg or AuthoredPipelineConfig.from_env()
        if not cfg_.run_token:
            # BLD-2 review R2 SHOULD-FIX 12: log once why no checks happen when run token unset
            cursor = getattr(context, "cursor", None)
            if cursor != "token_unset_warned":
                context.log.warning(
                    f"Target lag check for pipeline {pipeline_id!r}: DISPAR_RUN_TOKEN is unset; no lag checks will run"
                )
                if hasattr(context, "update_cursor"):
                    context.update_cursor("token_unset_warned")
            return None

        url = f"{cfg_.api_url}/api/pipelines/runnable/{pipeline_id}/lag-check"
        try:
            resp = requests.post(
                url,
                headers=_headers(cfg_),
                timeout=10,
            )
        except requests.RequestException as exc:
            # D6: transport error is skipped for this tick (next tick asks again); never raises.
            context.log.warning(
                f"Target lag check for pipeline {pipeline_id!r} transport failure ({exc}); skipping tick"
            )
            return None

        if 400 <= resp.status_code < 500:
            # D6: 4xx is logged and skipped
            context.log.warning(
                f"Target lag check for pipeline {pipeline_id!r} returned {resp.status_code} ({resp.text}); skipping tick"
            )
            return None

        if resp.status_code >= 500:
            # D6: 5xx is skipped for this tick
            context.log.warning(
                f"Target lag check for pipeline {pipeline_id!r} returned server error {resp.status_code}; skipping tick"
            )
            return None

        try:
            body = resp.json()
        except ValueError as exc:
            context.log.warning(
                f"Target lag check for pipeline {pipeline_id!r} returned non-JSON response ({exc}); skipping tick"
            )
            return None

        decision = body.get("decision")
        if decision == "run":
            # BLD-2 review R1 BLOCKER 2: use body['jobName'] and validate against allowed jobs
            run_key = body.get("runKey")
            target_job_name = body.get("jobName")
            if not run_key:
                context.log.warning(
                    f"Target lag check for pipeline {pipeline_id!r} returned decision 'run' without runKey; skipping tick"
                )
                return None
            if not target_job_name or target_job_name not in allowed_job_names:
                context.log.warning(
                    f"Target lag check for pipeline {pipeline_id!r} returned unexpected jobName {target_job_name!r} "
                    f"(allowed: {sorted(allowed_job_names)}); skipping tick"
                )
                return None
            yield RunRequest(run_key=run_key, job_name=target_job_name)

    return _lag_sensor


def _pipeline_target(p: dict[str, Any]) -> str:
    if p.get("target"):
        return str(p["target"])
    defn = p.get("definition") or {}
    zone = defn.get("targetZone") or p.get("targetZone") or p.get("target_zone") or ""
    table = defn.get("targetTable") or p.get("targetTable") or p.get("target_table") or ""
    if zone and table:
        return f"{zone}.{table}"
    return ""


def _pipeline_source(p: dict[str, Any]) -> str:
    if p.get("source"):
        return str(p["source"])
    defn = p.get("definition") or {}
    zone = defn.get("sourceZone") or p.get("sourceZone") or p.get("source_zone") or ""
    table = defn.get("sourceTable") or p.get("sourceTable") or p.get("source_table") or ""
    if zone and table:
        return f"{zone}.{table}"
    return ""


def _pipeline_tenant(p: dict[str, Any]) -> str | None:
    t = p.get("tenantId") or p.get("tenant_id")
    return str(t) if t is not None else None


def _pipeline_depends_on(p: dict[str, Any]) -> list[str]:
    deps = p.get("dependsOn") or p.get("depends_on") or []
    return [str(d) for d in deps]


def build_authored_lag_sensors(
    pipeline_jobs: list[tuple[dict[str, Any], Any]],
    cfg: AuthoredPipelineConfig | None = None,
) -> list[Any]:
    """One lag sensor per authored pipeline with a time-based target lag.
    BLD-2 review R2 SHOULD-FIX 7: attach only jobs of downstream pipelines this pipeline actually reads
    (direct source/dependsOn or recursive chain in the same tenant).
    """
    sensors: list[Any] = []
    for pipeline, authored_job in pipeline_jobs:
        target_lag = pipeline.get("targetLag")
        if not isinstance(target_lag, dict) or target_lag.get("kind") != "lag":
            continue

        # Find downstream pipelines read by this pipeline (direct or transitive in same tenant)
        visited_ids: set[str] = set()
        queue = [pipeline]
        while queue:
            curr = queue.pop(0)
            curr_tenant = _pipeline_tenant(curr)
            curr_source = _pipeline_source(curr)
            curr_deps = set(_pipeline_depends_on(curr))
            for other_p, other_j in pipeline_jobs:
                other_id = other_p.get("id", "")
                if not other_id or other_id in visited_ids or other_id == pipeline.get("id"):
                    continue
                other_lag = other_p.get("targetLag")
                if not isinstance(other_lag, dict) or other_lag.get("kind") != "downstream":
                    continue
                if _pipeline_tenant(other_p) != curr_tenant:
                    continue
                other_target = _pipeline_target(other_p)
                if (other_target and other_target == curr_source) or (other_id in curr_deps):
                    visited_ids.add(other_id)
                    queue.append(other_p)

        allowed = [authored_job] + [
            j for other_p, j in pipeline_jobs
            if other_p.get("id") in visited_ids and j.name != authored_job.name
        ]
        sensor_ = build_authored_lag_sensor(pipeline, authored_job, allowed_jobs=allowed, cfg=cfg)
        if sensor_ is not None:
            sensors.append(sensor_)
    return sensors


# Built at Dagster code-load time, same pattern `agent_run_schedules`
# (`agent_runs.py`) uses -- see `_fetch_authored_pipelines`'s doc comment
# for every way this degrades to empty lists without raising.
#
# Three plain assignments rather than tuple unpacking, so
# `ops/lint/check_intra_package_imports.py` (which reads module-level
# names statically) sees all four names `definitions.py` imports.
_authored_jobs_and_schedules_and_deps = build_authored_definitions()
authored_jobs = _authored_jobs_and_schedules_and_deps[0]
authored_schedules = _authored_jobs_and_schedules_and_deps[1]
_authored_pipeline_jobs = _authored_jobs_and_schedules_and_deps[2]
authored_dependency_sensors = build_authored_dependency_sensors(_authored_pipeline_jobs)
authored_lag_sensors = build_authored_lag_sensors(_authored_pipeline_jobs)
