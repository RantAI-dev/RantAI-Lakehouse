"""Polling sensors that run the pipelines downstream of a re-upload
(`BLD-14`, `DEC-22`, D5-D9; planner defaults P1-P10). The user asked for
"run again after this table is re-uploaded" to include what the first
pipelines feed: a Silver to Gold pipeline must follow the Silver run.

Why a polling `@sensor` and not a `run_status_sensor`: a chain has to see
both a failed parent (a FAILURE run) and a diamond where two parents must
both succeed before the join runs. One run-status event cannot express
either fact; only a tick that reads every parent's state can. It also has
to record WHY a pipeline did not run, which the run-status path cannot do
for a pipeline that never got a run at all (D8). The record is posted on
the tick AFTER the launch, because a run's id exists only once Dagster has
accepted the `RunRequest`.

One sensor per pipeline that the D3 graph can reach, every 30 seconds.
`default_status=RUNNING` because a chain sensor shipped STOPPED never
fires. Each re-upload is identified by two tags on every run the chain
starts: the `file_ingest_job` run's id and the bronze table it loaded.
The same tags on the first pipelines' `_after` runs let the two launchers
step aside for each other (D7). The cursor's floor stops a new sensor from
replaying last week's re-uploads (P10); its settled list makes a record
one-shot even if the post's response was lost.

BLD-14 review R2's fix is OPTION A: the chain sensor owns the FIRST
pipeline's launch too, with the same `run_key` per re-upload and pipeline,
and `_after` steps aside for a file load whose table starts a chain. A
newer load ending while an older first run is queued still gets its own
tagged run, so downstream pipelines never wait on a re-upload that was
silently dropped. The other option offered (counting a later untagged
first run as the newer load) was not taken.

This module imports from `authored_factory`, never the reverse: the graph
and the wire helpers live there, and this file only turns them into
Dagster sensors. With `PIPELINE_RUN_TOKEN` unset it builds no sensors and
logs once, the code location's degraded posture.
"""

from __future__ import annotations

import json
import logging
import time
from collections.abc import Callable
from datetime import datetime, timezone
from dataclasses import dataclass
from typing import Any

import requests
from dagster import (
    DagsterRunStatus,
    DefaultSensorStatus,
    RunRequest,
    RunsFilter,
    SkipReason,
    sensor,
)

from dispar_orchestrate import pipeline_events
from dispar_orchestrate.authored_factory import (
    CHAIN_WAIT_LIMIT_SECONDS,
    AuthoredPipelineConfig,
    FILE_INGEST_JOB,
    FILE_INGEST_OP_CONFIG,
    REUPLOAD_ROOT_TAG,
    REUPLOAD_TABLE_TAG,
    ChainEntry,
    _authored_pipeline_jobs,
    _dagster_safe_name,
    _run_config_value,
    reupload_chain_graph,
    reupload_first_pipeline_entries,
)

_LOGGER = logging.getLogger(__name__)
CHAIN_STEP_PATH = "/api/pipelines/runnable/{id}/chain-step"
SETTLED_LIMIT = 100


def chain_sensor_name(pipeline_id: str) -> str:
    """The Rust twin's name (`routes::authored_pipelines::chain_sensor_name`):
    `authored__<safe_id>_chain`, so pause/resume switches the same sensor.
    """
    return f"authored__{_dagster_safe_name(pipeline_id)}_chain"


@dataclass(frozen=True)
class Root:
    """One re-upload: the `file_ingest_job` run and the table it loaded."""

    run_id: str
    table: str
    end_time: float


@dataclass(frozen=True)
class RunInfo:
    """The one fact `decide` needs about a run tagged with a root."""

    run_id: str
    status: str
    in_flight: bool = False


@dataclass(frozen=True)
class Lookups:
    """The three questions the pure decider asks about one re-upload."""

    pipeline_id: str
    run_for_tag: Callable[[str, str], RunInfo | None]
    newer_root: Callable[[str, str], bool]
    in_flight: Callable[[str], bool]


@dataclass(frozen=True)
class Decision:
    """What one (re-upload, pipeline) pair should do on this tick."""

    action: str  # "run", "record" or "wait"
    outcome: str | None = None
    reason: str | None = None
    cause: str | None = None
    run_id: str | None = None
    run_key: str | None = None
    tags: dict[str, str] | None = None
    detail: str | None = None


def decide(root: Root, entry: ChainEntry, lookups: Lookups, now: float) -> Decision:
    """One pipeline's decision for one re-upload, in the order of D9/P4/P5.

    A run already tagged with this root settles the `started` record; a
    newer root of the same table supersedes an older one that has not
    launched; a failed or cancelled ancestor is the cause; otherwise every
    parent must have a SUCCESS run before this pipeline launches, and it
    never launches twice at once (P4). Waiting past the authored run limit
    records `wait_expired` naming the first parent without SUCCESS (P5).
    """
    own = lookups.run_for_tag(lookups.pipeline_id, root.run_id)
    if own is not None:
        return Decision(action="record", outcome="started", run_id=own.run_id)
    if lookups.newer_root(root.table, root.run_id):
        return Decision(action="record", outcome="superseded")

    failed: Decision | None = None
    for ancestor in entry.ancestors:
        run = lookups.run_for_tag(ancestor, root.run_id)
        if run is not None and run.status == "FAILURE":
            failed = Decision(
                action="record",
                outcome="not_run",
                reason="upstream_failed",
                cause=ancestor,
            )
            break
        if run is not None and run.status == "CANCELED":
            failed = Decision(
                action="record",
                outcome="not_run",
                reason="upstream_cancelled",
                cause=ancestor,
            )
            break
    if failed is not None:
        return failed

    parents = [lookups.run_for_tag(parent, root.run_id) for parent in entry.parents]
    if all(run is not None and run.status == "SUCCESS" for run in parents):
        if lookups.in_flight(lookups.pipeline_id):
            return Decision(action="wait", detail="a run is already queued or running")
        return Decision(
            action="run",
            run_key=f"reupload-chain:{root.run_id}:{lookups.pipeline_id}",
            tags={REUPLOAD_ROOT_TAG: root.run_id, REUPLOAD_TABLE_TAG: root.table},
        )

    if now - root.end_time > CHAIN_WAIT_LIMIT_SECONDS:
        first = next(
            parent
            for parent, run in zip(entry.parents, parents)
            if run is None or run.status != "SUCCESS"
        )
        return Decision(
            action="record",
            outcome="not_run",
            reason="wait_expired",
            cause=first,
        )
    return Decision(action="wait", detail="waiting for every parent of this re-upload")


def _post_chain_step(cfg: pipeline_events.PipelineEventsConfig, path: str, body: dict) -> bool:
    """POST one outcome with the same bounded retry as pipeline events.

    `pipeline_events._post_event` owns the retry primitives (`_retryable`,
    `_sleep`, `POST_ATTEMPTS`, `POST_TIMEOUT`) and this module reuses them;
    only the body differs (a chain step is not a `{runId, jobName}` event).
    Returns whether the API accepted the record; a failed post is logged
    by the caller and retried on the next tick, never raised.
    """
    for attempt in range(1, pipeline_events.POST_ATTEMPTS + 1):
        try:
            requests.post(
                f"{cfg.api_url}{path}",
                json=body,
                headers=pipeline_events._headers(cfg),
                timeout=pipeline_events.POST_TIMEOUT,
            ).raise_for_status()
            return True
        except requests.RequestException as err:
            if attempt == pipeline_events.POST_ATTEMPTS or not pipeline_events._retryable(err):
                return False
            pipeline_events._sleep(pipeline_events.POST_PAUSE_SECONDS)
    return False


def _cursor(raw: str | None, now: float) -> tuple[float, list[str]]:
    """The cursor's floor and settled roots; an absent cursor sets the
    floor to now and launches nothing (P10).
    """
    if not raw:
        return now, []
    try:
        value = json.loads(raw)
        floor = float(value["floor"])
        settled = [str(item) for item in value.get("settled", [])]
        return floor, settled[-SETTLED_LIMIT:]
    except (ValueError, TypeError, KeyError):
        return now, []


def build_reupload_chain_sensor(
    pipeline: dict[str, Any],
    job: Any,
    entries: dict[str, ChainEntry],
    cfg: AuthoredPipelineConfig | None = None,
) -> Any:
    """One polling sensor for the pipelines reachable from `pipeline`.

    `entries` is this pipeline's `reupload_chain_graph` value (bronze
    table -> parents/ancestors). The sensor builds its lookups from the
    Dagster instance on every tick: roots are the newest successful loads
    of a table in `entries`, and the tagged runs are the runs of each
    parent's authored job carrying the same root tag.
    """
    cfg = cfg or AuthoredPipelineConfig.from_env()
    events_cfg = pipeline_events.PipelineEventsConfig(
        api_url=cfg.api_url, run_token=cfg.run_token
    )
    pipeline_id = str(pipeline.get("id"))
    job_name = f"authored__{_dagster_safe_name(pipeline_id)}"

    @sensor(
        name=chain_sensor_name(pipeline_id),
        job=job,
        minimum_interval_seconds=30,
        default_status=DefaultSensorStatus.RUNNING,
    )
    def _chain_sensor(context: Any) -> Any:
        now = time.time()
        raw = context.cursor
        floor, settled = _cursor(raw, now)
        if not raw:
            context.update_cursor(json.dumps({"floor": floor, "settled": settled}))
            yield SkipReason("BLD-14: the first tick sets the re-upload cursor floor")
            return

        # BLD-14 review R1 BLOCKER: a chain may wait six hours, so a
        # recency-capped scan of the newest 20 loads across every table
        # loses a re-upload behind other tables' loads: no run, no record,
        # no expiry. Find open re-uploads by their own key instead -- the
        # six-hour window (P5) -- with no row cap; the table filter below
        # narrows them to this pipeline's chain.
        root_records = context.instance.get_run_records(
            filters=RunsFilter(
                job_name=FILE_INGEST_JOB,
                statuses=[DagsterRunStatus.SUCCESS],
                updated_after=datetime.fromtimestamp(
                    now - CHAIN_WAIT_LIMIT_SECONDS, tz=timezone.utc
                ),
            ),
        )
        roots: list[Root] = []
        for record in root_records:
            table = _run_config_value(record.dagster_run, FILE_INGEST_OP_CONFIG)
            if not table or table not in entries or (record.end_time or 0.0) < floor:
                continue
            if record.dagster_run.run_id in settled:
                continue
            roots.append(
                Root(run_id=record.dagster_run.run_id, table=table, end_time=record.end_time or 0.0)
            )

        def run_for_tag(pid: str, root_id: str) -> RunInfo | None:
            tagged = context.instance.get_run_records(
                filters=RunsFilter(
                    job_name=f"authored__{_dagster_safe_name(pid)}",
                    tags={REUPLOAD_ROOT_TAG: root_id},
                ),
                limit=1,
            )
            for record in tagged:
                tags = getattr(record.dagster_run, "tags", None) or {}
                if tags.get(REUPLOAD_TABLE_TAG) is None:
                    continue
                status = record.dagster_run.status
                return RunInfo(
                    run_id=record.dagster_run.run_id,
                    status=status.name if hasattr(status, "name") else str(status),
                    in_flight=status
                    in (DagsterRunStatus.QUEUED, DagsterRunStatus.NOT_STARTED, DagsterRunStatus.STARTED),
                )
            return None

        def in_flight(pid: str) -> bool:
            runs = context.instance.get_run_records(
                filters=RunsFilter(
                    job_name=f"authored__{_dagster_safe_name(pid)}",
                    statuses=[
                        DagsterRunStatus.QUEUED,
                        DagsterRunStatus.NOT_STARTED,
                        DagsterRunStatus.STARTED,
                    ],
                ),
                limit=1,
            )
            return bool(runs)

        waits: list[str] = []
        for root in roots:
            entry = entries[root.table]
            lookups = Lookups(
                pipeline_id=pipeline_id,
                run_for_tag=run_for_tag,
                newer_root=lambda table, root_id, r=root: any(
                    other.table == table and other.end_time > r.end_time for other in roots
                ),
                in_flight=in_flight,
            )
            decision = decide(root, entry, lookups, now)
            if decision.action == "run":
                yield RunRequest(run_key=decision.run_key, tags=decision.tags)
                waits.append(f"{root.run_id}: launched; recording on the next tick")
            elif decision.action == "record":
                body = {
                    "rootRunId": root.run_id,
                    "sourceTable": f"bronze.{root.table}",
                    "outcome": decision.outcome,
                    "reason": decision.reason,
                    "runId": decision.run_id,
                    "causePipelineId": decision.cause,
                }
                path = CHAIN_STEP_PATH.replace("{id}", pipeline_id)
                accepted = _post_chain_step(events_cfg, path, body)
                if accepted:
                    # Only an accepted record settles a root. A launched
                    # run is recorded on the next tick, when its id exists.
                    if root.run_id not in settled:
                        settled.append(root.run_id)
                    waits.append(f"{root.run_id}: recorded {decision.outcome}")
                else:
                    _LOGGER.warning(
                        "BLD-14: chain step for %s / %s was not accepted; retry next tick",
                        pipeline_id,
                        root.run_id,
                    )
                    waits.append(f"{root.run_id}: record not accepted, retrying")
            else:
                waits.append(f"{root.run_id}: {decision.detail}")

        context.update_cursor(
            json.dumps({"floor": floor, "settled": settled[-SETTLED_LIMIT:]})
        )
        if not roots:
            yield SkipReason("BLD-14: no re-upload in the window for this pipeline")
        elif waits:
            yield SkipReason("; ".join(waits))

    return _chain_sensor


def build_reupload_chain_sensors(
    pipeline_jobs: list[tuple[dict[str, Any], Any]],
    cfg: AuthoredPipelineConfig | None = None,
) -> list[Any]:
    """One chain sensor per pipeline with a non-empty `reupload_chain_graph`
    entry map. With no `PIPELINE_RUN_TOKEN` this logs once and builds none.
    """
    cfg = cfg or AuthoredPipelineConfig.from_env()
    if not cfg.run_token:
        _LOGGER.warning(
            "BLD-14: PIPELINE_RUN_TOKEN is unset; no re-upload chain sensors are built"
        )
        return []
    # BLD-14 review R2, option A: the chain owns the first pipeline's
    # launch as well, with the same per-reupload run_key. Its empty entry
    # makes `decide` launch it as soon as its parent set is empty, while a
    # downstream pipeline keeps the graph's real parents.
    payloads = [pipeline for pipeline, _ in pipeline_jobs]
    graph = reupload_chain_graph(payloads)
    first_entries = reupload_first_pipeline_entries(payloads)
    sensors: list[Any] = []
    for pipeline, job in pipeline_jobs:
        pipeline_id = str(pipeline.get("id"))
        entries = {**first_entries.get(pipeline_id, {}), **graph.get(pipeline_id, {})}
        if not entries:
            continue
        sensors.append(build_reupload_chain_sensor(pipeline, job, entries, cfg))
    return sensors


# Code-load time, same pattern as `authored_factory`'s sensor lists: one
# assignment so `ops/lint/check_intra_package_imports.py` sees the name
# `definitions.py` imports, and the no-token posture lives in one builder.
reupload_chain_sensors = build_reupload_chain_sensors(_authored_pipeline_jobs)
