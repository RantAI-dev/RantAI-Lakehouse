"""Dagster `run_failure_sensor` that turns each failed run into a single
`POST /api/pipelines/events/run-failed` to the API (plan 1e).

Why a sensor and not a Dagster `HookDefinition`: a hook fires inside the
run, where a transient network failure between Dagster and the API would
be reported as a run failure of its own and re-evaluate on retry. A sensor
runs after the run is recorded as `FAILURE`, with its own retry semantics
in Dagster — and the API side dedupes by `(run_id, kind)` so a sensor
retry still does not double-alert.

The sensor's contract with the API:

* one POST per `RunFailureSensorContext`, carrying `{runId, jobName}`;
* `runId` is `context.dagster_run.run_id`, the same id the API then
  queries `Dagster` for (`pipeline_run_status`) to confirm `status ==
  "FAILURE"` and to enumerate `stepStats` with `status == "FAILURE"` —
  the API, not this sensor, owns the truth about which steps failed;
* `jobName` is `context.dagster_run.job_name`, the `Dagster` job name
  the API reverses through `authored_pipelines::job_name` to find the
  pipeline id. A name the API does not recognise is logged and ignored
  (`matched: 0`), never raised — the sensor fires on EVERY failure in
  this code location, including jobs we did not author
  (`alerts_run_job`, `bronze_ingest_job`, etc.), and those would
  otherwise raise here.

When `PIPELINE_RUN_TOKEN` is unset the sensor is degraded-honest: it
logs and posts nothing, instead of failing its tick. Mirrors
`authored_factory.run_jobs`'s posture (the same token gates this
sensor's events).

`default_status=RUNNING` (not STOPPED): a failure sensor that no one
turns on cannot fire, and "you have a sensor on the off switch" is
not the same as "you get an alert when a pipeline fails." This is
this plan's deliberate choice, with a comment so the next reader
knows why RUNNING and not STOPPED.
"""

from __future__ import annotations

import os
from dataclasses import dataclass

import requests
from dagster import DefaultSensorStatus, RunFailureSensorContext, run_failure_sensor

DEFAULT_API_URL = "http://lakehouse-api:8080"
RUN_FAILED_PATH = "/api/pipelines/events/run-failed"


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


def post_run_failed(cfg: PipelineEventsConfig, run_id: str, job_name: str) -> None:
    """POST one failed-run notification. Best-effort: a sensor tick that
    raises here is logged by Dagster as a sensor failure and retried;
    the API's dedupe makes a retry safe (`{matched: 0}` on the second
    call), so we surface nothing here other than the network error
    itself.
    """
    if not cfg.run_token:
        # Degraded mode: same posture as `authored_factory.run_jobs` — log
        # through the caller-provided context, do not POST.
        return
    requests.post(
        f"{cfg.api_url}{RUN_FAILED_PATH}",
        json={"runId": run_id, "jobName": job_name},
        headers=_headers(cfg),
        timeout=30,
    ).raise_for_status()


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
        # Logged at WARNING; Dagster will retry the tick. The API's
        # `(run_id, kind)` dedupe makes a retry non-duplicative.
        context.log.warning(
            "POST /api/pipelines/events/run-failed failed for run %s (job %s): %s",
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
