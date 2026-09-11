"""WS0 item 11 (docs/superpowers/plans/2026-09-10-ws0-delta-audit.md) —
Dagster's scheduled trigger for POST /api/alerts/run.

Same shape as `agent_runs.py`/`gold_export.py`: this does not evaluate
alert rules itself (that lives in Rust, `lakehouse-alerts::run_rules`,
reused as-is by `routes::alerts::run`) — this job's whole job is to be
the scheduled trigger.

# The auth floor, and why this needed a real credential

`POST /api/alerts/run`'s `Policy::RequiresAuth` entry in
`rust/crates/lakehouse-api/src/policy.rs` is a FLOOR: `auth_gate` demands
a real, authenticated `lakehouse_auth::Principal` before
`routes::alerts::check_run_token`'s own `x-run-token` check ever runs —
the exact shape `agent_runs.py`'s own module doc comment documents for
`POST /api/agents/employees/{id}/run`. `lakehouse-api::main::
bootstrap_alerts_run_service` closes it via the SAME generalized
mechanism `bootstrap_agent_run_service` uses (both now call a shared
`bootstrap_service_run_identity` core): it idempotently seeds a
`service_identity` ("alerts-run-scheduler") plus a matching
`service_credential` from `ALERTS_RUN_TOKEN` on every boot. This module
sends that SAME value two ways, exactly like `agent_runs.py::_headers`:

* `Authorization: Bearer <ALERTS_RUN_TOKEN>` — clears `auth_gate`'s floor
  via `ServiceTokenAuthenticator`.
* `x-run-token: <ALERTS_RUN_TOKEN>` — clears `check_run_token`'s own check
  inside the handler body, which is what actually gates this route.

`alerts-run-scheduler` is seeded with NO scopes, not `alert:write` — see
`bootstrap_alerts_run_service`'s doc comment in `main.rs` for why: the
route checks no permission at all, only that a token authenticates, so a
broader grant would be pure unused privilege a leaked token could abuse
against `POST`/`PUT`/`DELETE /api/alerts` instead.

With `ALERTS_RUN_TOKEN` unset, `_headers` returns `{}` and the schedule
below is still REGISTERED (unlike `agent_run_schedules`, which is
per-employee and genuinely has zero entries when unset) but every run it
triggers will 401/503 until the token is set — this mirrors
`replication_slot_check_schedule`'s own posture of "always scheduled, the
job's own auth decides whether a given run does anything," not
`agent_run_schedules`'s "zero schedules exist at all" posture, because
alert evaluation (unlike a per-employee run) is not parameterized by data
this module would need a successful API call to discover.
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from typing import Any

import requests
from dagster import DefaultScheduleStatus, ScheduleDefinition, job, op


def _env(name: str, default: str) -> str:
    value = os.environ.get(name, "").strip()
    return value if value else default


@dataclass(frozen=True)
class AlertsRunConfig:
    """Everything `run_alerts` needs, read once from the environment."""

    run_token: str
    api_url: str

    @classmethod
    def from_env(cls) -> AlertsRunConfig:
        return cls(
            run_token=_env("ALERTS_RUN_TOKEN", ""),
            api_url=_env("LAKEHOUSE_API_URL", "http://lakehouse-api:8080"),
        )


def _headers(cfg: AlertsRunConfig) -> dict[str, str]:
    """Same two-header shape as `agent_runs.py::_headers` — see the module
    doc comment's "The auth floor" section for why both are needed. Empty
    when no token is configured, so a caller can tell "not configured"
    apart from "configured, but Dagster forgot to send it" without
    inspecting the response.
    """
    if not cfg.run_token:
        return {}
    return {
        "x-run-token": cfg.run_token,
        "Authorization": f"Bearer {cfg.run_token}",
    }


def run_alerts(context: Any, cfg: AlertsRunConfig) -> dict[str, Any]:
    """`POST /api/alerts/run` — evaluate every alert/digest rule.

    Raises `requests.HTTPError` on a non-2xx response (401 when the token
    is wrong/unset and no service credential exists yet, as much as any
    other status) — Dagster surfaces that as a failed run, which is the
    correct, visible failure mode for a misconfigured schedule, matching
    `agent_runs.py::run_agent_employee`'s posture.
    """
    response = requests.post(
        f"{cfg.api_url}/api/alerts/run",
        headers=_headers(cfg),
        timeout=30,
    )
    response.raise_for_status()
    result = response.json()
    context.log.info(f"alerts run: {result.get('ran')} rule(s) evaluated")
    return result


@op
def run_alerts_op(context) -> dict[str, Any]:
    return run_alerts(context, AlertsRunConfig.from_env())


@job
def alerts_run_job() -> None:
    """`DAGSTER_LOCATION`-visible job name: `alerts_run_job`."""
    run_alerts_op()


# Every 15 minutes, matching `replication_slot_check_schedule`'s own cadence
# and rationale (`replication_metrics.py`): an alert condition going
# undetected for a full day is the same class of same-day operational risk
# a stuck replication slot is. This is this plan's best-founded guess, not a
# value read from any spec.
#
# default_status=RUNNING (not STOPPED): the schedule is always registered —
# the job's own auth decides whether a given run does anything, matching
# `replication_slot_check_schedule`'s posture, not `agent_run_schedules`'s
# (which has zero entries at all when its token is unset).
alerts_run_schedule = ScheduleDefinition(
    job=alerts_run_job,
    cron_schedule="*/15 * * * *",
    default_status=DefaultScheduleStatus.RUNNING,
)
