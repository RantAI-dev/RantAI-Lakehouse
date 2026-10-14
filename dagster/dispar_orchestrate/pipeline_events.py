"""Dagster `run_failure_sensor` that turns each failed run into a single
`POST /api/pipelines/events/run-failed` to the API (plan 1e), and a
matching `run_status_sensor` for `SUCCESS` runs that posts to
`/api/pipelines/events/run-finished` (plan 1f). The latter fires the
`pipeline_slow` and `pipeline_volume_drop` alerts from the API side.

Why a sensor and not a Dagster `HookDefinition`: a hook fires inside the
run, where a transient network failure between Dagster and the API would
be reported as a run failure of its own. A sensor runs after the run is
recorded as `FAILURE`/`SUCCESS`.

What Dagster does NOT do (read in the pinned `dagster==1.13.20`,
`_core/definitions/run_status_sensor_definition.py`, `_wrapped_fn`, lines
910-983): whether the sensor function raises or returns normally, the run
status sensor advances its cursor past that run's event (`context.update_cursor`
at line 967) and yields a `DagsterRunReaction`. A raise only puts the error
on the tick (`serializable_error`, lines 961-965); it does not ask again. So
there is NO retry from Dagster for a run whose POST failed. The only retry
is the bounded one in this file (`post_run_failed`, `post_run_finished`):
`POST_ATTEMPTS` tries a few seconds apart, for a connection error, a
timeout or a 5xx answer, never for a 4xx. If the API is unreachable for the
whole window (two pauses of 3 seconds, at most 45 seconds in all,
inside the 60 seconds Dagster gives a sensor evaluation by default,
`DAGSTER_SENSOR_GRPC_TIMEOUT_SECONDS`), that run's report is lost: the
alert is not sent and the connector's health catches up at its next run.
The API dedupes by `(run_id, kind)`, so a retry that follows a lost
response cannot alert twice.

The sensor's contract with the API:

* one POST per sensor context, carrying `{runId, jobName}`;
* `runId` is `context.dagster_run.run_id`, the same id the API then
  queries `Dagster` for (`pipeline_run_status`) to confirm the run's
  status — the API, not this sensor, owns the truth about a run's
  state;
* `jobName` is `context.dagster_run.job_name`, the `Dagster` job name
  the API reverses through `authored_pipelines::job_name` to find the
  pipeline id. A name the API does not recognise is logged and ignored
  (`matched: 0`), never raised — the sensor fires on EVERY run in this
  code location, including jobs we did not author (`alerts_run_job`,
  `bronze_ingest_job`, etc.), and those would otherwise raise here.

When `PIPELINE_RUN_TOKEN` is unset the sensor is degraded-honest: it
logs and posts nothing, instead of failing its tick. Mirrors
`authored_factory.run_jobs`'s posture (the same token gates this
sensor's events).

`default_status=RUNNING` (not STOPPED): a sensor that no one
turns on cannot fire, and "you have a sensor on the off switch" is
not the same as "you get an alert when a pipeline fails." This is
this plan's deliberate choice, with a comment so the next reader
knows why RUNNING and not STOPPED.
"""

from __future__ import annotations

import os
import time
from dataclasses import dataclass

import requests
from dagster import (
    DagsterRunStatus,
    DefaultSensorStatus,
    RunFailureSensorContext,
    RunStatusSensorContext,
    run_failure_sensor,
    run_status_sensor,
)

DEFAULT_API_URL = "http://lakehouse-api:8080"
RUN_FAILED_PATH = "/api/pipelines/events/run-failed"
RUN_FINISHED_PATH = "/api/pipelines/events/run-finished"

# Bounded in-tick retry (`SRC-7` review SHOULD-FIX 4): Dagster never asks
# again (module docstring), so the only second chance is taken here.
# 3 attempts x (3 s connect + 10 s read) + 2 pauses of 3 s = 45 s worst case
# for one run, inside the 60 s default sensor evaluation limit. A sensor
# evaluation handles every run that ended since the last tick, so a long
# outage can still exceed the limit with many runs; that run then fails the
# tick and its report is lost like any other (module docstring).
POST_ATTEMPTS = 3
POST_PAUSE_SECONDS = 3
POST_TIMEOUT = (3, 10)  # (connect, read) seconds


def _env(name: str, default: str) -> str:
    value = os.environ.get(name, "").strip()
    return value if value else default


@dataclass(frozen=True)
class PipelineEventsConfig:
    api_url: str
    run_token: str

    @classmethod
    def from_env(cls) -> "PipelineEventsConfig":
        return cls(
            api_url=_env("LAKEHOUSE_API_URL", DEFAULT_API_URL),
            run_token=_env("PIPELINE_RUN_TOKEN", ""),
        )


def _headers(cfg: PipelineEventsConfig) -> dict[str, str]:
    """`Authorization: Bearer` + `x-run-token`, matching
    `authored_factory._headers`/`agent_runs._headers`. Empty when the
    token is unset: a degraded sensor must not pretend to authenticate.
    """
    if not cfg.run_token:
        return {}
    return {"Authorization": f"Bearer {cfg.run_token}", "x-run-token": cfg.run_token}


def _sleep(seconds: float) -> None:
    """The pause between attempts; a name of its own so unit tests replace it
    instead of sleeping."""
    time.sleep(seconds)


def _retryable(err: requests.RequestException) -> bool:
    """A connection error, a timeout or a 5xx answer may succeed a few
    seconds later; a 4xx (a wrong token, a refused caller) will not."""
    if isinstance(err, requests.HTTPError):
        status = err.response.status_code if err.response is not None else 0
        return status >= 500
    return isinstance(err, (requests.ConnectionError, requests.Timeout))


def _post_event(cfg: PipelineEventsConfig, path: str, run_id: str, job_name: str) -> None:
    """POST `{runId, jobName}` with up to `POST_ATTEMPTS` tries,
    `POST_PAUSE_SECONDS` apart, retrying only what `_retryable` allows.

    Raises the last `requests.RequestException` once the attempts are used
    or the failure is not retryable; the caller logs it. Dagster does not
    ask again after that (module docstring).
    """
    for attempt in range(1, POST_ATTEMPTS + 1):
        try:
            requests.post(
                f"{cfg.api_url}{path}",
                json={"runId": run_id, "jobName": job_name},
                headers=_headers(cfg),
                timeout=POST_TIMEOUT,
            ).raise_for_status()
            return
        except requests.RequestException as err:
            if attempt == POST_ATTEMPTS or not _retryable(err):
                raise
            _sleep(POST_PAUSE_SECONDS)


def post_run_failed(cfg: PipelineEventsConfig, run_id: str, job_name: str) -> None:
    """POST one failed-run notification, retrying inside the tick (see
    `_post_event`). With no token, posts nothing. Raises
    `requests.RequestException` when the attempts are used up or the API
    answers 4xx; nothing retries after that, because Dagster's run sensor
    moves past the run whether the function raised or not (module
    docstring). The API's dedupe makes a repeated delivery safe
    (`{matched: 0}`).
    """
    if not cfg.run_token:
        # Degraded mode: same posture as `authored_factory.run_jobs` — log
        # through the caller-provided context, do not POST.
        return
    _post_event(cfg, RUN_FAILED_PATH, run_id, job_name)


def post_run_finished(cfg: PipelineEventsConfig, run_id: str, job_name: str) -> None:
    """POST one successful-run notification (plan 1f); same retry and the
    same limit as `post_run_failed`."""
    if not cfg.run_token:
        return
    _post_event(cfg, RUN_FINISHED_PATH, run_id, job_name)


def evaluate_failed_run(context: RunFailureSensorContext) -> None:
    """Plan 1e: one POST per failed run. We do not iterate over
    `get_step_failure_events()` — the API side reads
    `Dagster.pipeline_run_status(run_id).steps` for itself and uses
    its own truth. One POST per run, not one per step, keeps the
    sensor's job to "tell the API a run failed" and leaves the
    fan-out to the alert rules.

    Split out from the sensor body so unit tests can drive it without a
    live `DagsterInstance` (a `SensorDefinition.evaluate_tick` rejects
    any context that isn't a `SensorEvaluationContext`, which we cannot
    build here without a Dagster instance).
    """
    cfg = PipelineEventsConfig.from_env()
    run = context.dagster_run
    if not cfg.run_token:
        context.log.info(
            "PIPELINE_RUN_TOKEN is unset; pipeline_run_failed_sensor is degraded-honest "
            "and will not POST until the API and this code location both have it set."
        )
        return
    try:
        post_run_failed(cfg, run.run_id, run.job_name)
    except requests.RequestException as err:
        # Dagster does not retry (module docstring): the cursor has moved
        # past this run, so after the in-tick attempts this report is lost.
        context.log.warning(
            "POST /api/pipelines/events/run-failed gave up for run %s (job %s), "
            "the alert for it is not sent: %s",
            run.run_id,
            run.job_name,
            err,
        )


def evaluate_finished_run(context: RunStatusSensorContext) -> None:
    """Plan 1f: one POST per successful run. Mirrors
    `evaluate_failed_run`'s posture — degraded-honest on a missing token;
    on a network error it has already retried inside the tick
    (`post_run_finished`), then logs and returns. Dagster does not ask
    again (module docstring).
    """
    cfg = PipelineEventsConfig.from_env()
    run = context.dagster_run
    if not cfg.run_token:
        context.log.info(
            "PIPELINE_RUN_TOKEN is unset; pipeline_run_finished_sensor is degraded-honest "
            "and will not POST until the API and this code location both have it set."
        )
        return
    try:
        post_run_finished(cfg, run.run_id, run.job_name)
    except requests.RequestException as err:
        context.log.warning(
            "POST /api/pipelines/events/run-finished gave up for run %s (job %s), "
            "the report for it is lost: %s",
            run.run_id,
            run.job_name,
            err,
        )


@run_failure_sensor(
    monitored_jobs=None,
    default_status=DefaultSensorStatus.RUNNING,
    name="pipeline_run_failed_sensor",
)
def pipeline_run_failed_sensor(
    context: RunFailureSensorContext,
) -> None:
    """Dagster `run_failure_sensor` that fires `evaluate_failed_run`
    for every failed run in this code location. Default-status
    RUNNING (not STOPPED): see the module docstring for the
    reasoning — a failure sensor that no one turns on cannot fire."""
    evaluate_failed_run(context)


@run_status_sensor(
    run_status=DagsterRunStatus.SUCCESS,
    monitored_jobs=None,
    default_status=DefaultSensorStatus.RUNNING,
    name="pipeline_run_finished_sensor",
)
def pipeline_run_finished_sensor(
    context: RunStatusSensorContext,
) -> None:
    """Dagster `run_status_sensor(SUCCESS)` that fires
    `evaluate_finished_run` for every successful run in this code
    location (plan 1f). Same posture as
    [`pipeline_run_failed_sensor`]: RUNNING by default, refuses to
    fire silently when `PIPELINE_RUN_TOKEN` is unset; a network error is
    retried inside the tick and then logged (Dagster does not retry)."""
    evaluate_finished_run(context)

