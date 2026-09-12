"""T3.3 (copilot-operations-handover plan) — Dagster's scheduled trigger for
digital-employee headless runs.

This module is deliberately the same shape as `gold_export.py`: it does not
run the copilot's tool-calling loop itself — that lives in Rust
(`lakehouse-api::routes::agents::run_employee`, T3.2), reusing the exact
gate/audit/tool-dispatch machinery interactive chat uses. This job's whole
job is to be the scheduled trigger: call
`POST /api/agents/employees/{employee_id}/run`, per employee, and log the
result. See `gold_export.py`'s module doc for why an HTTP call to
`lakehouse-api` rather than reimplementing anything here.

# Corrected 2026-09-09: `AGENT_RUN_TOKEN` now IS a real credential

Earlier revisions of this module explained why no schedule factory could be
written here: `POST /api/agents/employees/{id}/run`'s
`Policy::RequiresAuth` entry in `rust/crates/lakehouse-api/src/policy.rs`
is a FLOOR — `auth_gate` demands a real, authenticated
`lakehouse_auth::Principal` before the handler's own `x-run-token` check
ever runs, and this compose stack minted no such credential for Dagster.

That gap is now closed: `lakehouse-api::main::bootstrap_agent_run_service`
idempotently seeds a `service_identity` ("agent-run-scheduler", scoped
ONLY to `agent:manage` — never `*:*`) plus a matching `service_credential`
from `AGENT_RUN_TOKEN` on every boot. This module now sends that SAME
value two ways on every call (see `_headers` below):

* `Authorization: Bearer <AGENT_RUN_TOKEN>` — an opaque (non-JWT-shaped)
  bearer token, which `lakehouse-api::auth`'s shape-based dispatch routes
  to `lakehouse_auth::service_token::ServiceTokenAuthenticator` (see that
  module's doc comment on "Cookie vs. bearer, and disambiguating a service
  token from an OIDC token"). This is what clears `auth_gate`'s
  `Policy::RequiresAuth` floor.
* `x-run-token: <AGENT_RUN_TOKEN>` — `routes::agents::check_employee_run_auth`'s
  OWN check inside the handler body, which (when it matches) sets
  `agent_run.trigger = "schedule"` rather than `"manual"`.

With `AGENT_RUN_TOKEN` unset, both `_headers` and the schedule factory
below degrade to their old inert posture: no bearer, no `x-run-token`, and
(see `_fetch_schedulable_employees`) zero schedules — nothing here assumes
a token is present.

# The schedule factory, and why it cannot ever crash this code location

Per the plan (§3.4): at Dagster CODE-LOAD time (module import — this file
runs top-level code the moment the code location loads, no op/job
execution involved), read `GET /api/agents/employees` and build one
`ScheduleDefinition` per employee with a non-NULL `scheduleCron`
(`lakehouse_store::agents::DigitalEmployee::schedule_cron`, camelCase over
the wire).

Dagster evaluates every job/schedule in ONE code location at once — a
`bronze_ingest_job`, `bronze_maintenance_job`, and
`replication_slot_check_job` all live here too (`definitions.py`). An
uncaught exception importing this module would take ALL of them down, not
just the agent-run schedules. So `_fetch_schedulable_employees` below
catches every failure mode this HTTP call can hit — unreachable
`lakehouse-api` (`requests.RequestException`), a non-2xx response
(`requests.HTTPError` from `raise_for_status()`, covering 401 when the
token is wrong/stale as much as any other status), and a malformed/non-list
JSON body — and returns `[]` in every case, after a `print`-based warning
(there is no `context.log` outside an op/job body at import time; `print`
is what Dagster's own code-location loader captures into its own logs).
`test_agent_runs.py` proves both the "unreachable" and the "401" cases
load with zero schedules and no exception.
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from typing import Any

import requests

from dagster import Definitions, Field, ScheduleDefinition, job, op
from dispar_orchestrate.bronze_catalog import ClickHouseTarget, record_maintenance_run


def _env(name: str, default: str) -> str:
    value = os.environ.get(name, "").strip()
    return value if value else default


@dataclass(frozen=True)
class AgentRunConfig:
    api_url: str
    # D4 shape (`routes::agents::check_employee_run_auth`), PLUS (as of
    # the fix described in the module doc above) the bearer credential
    # that clears `auth_gate`'s `Policy::RequiresAuth` floor in the first
    # place: a shared token, set identically here and on `lakehouse-api`
    # (both read `AGENT_RUN_TOKEN` from the same compose `.env`).
    run_token: str

    @classmethod
    def from_env(cls) -> AgentRunConfig:
        return cls(
            api_url=_env("LAKEHOUSE_API_URL", "http://lakehouse-api:8080"),
            run_token=_env("AGENT_RUN_TOKEN", ""),
        )


def _headers(cfg: AgentRunConfig) -> dict[str, str]:
    # Never log this value — see `run_agent_employee`'s use of it below,
    # which only ever logs the header's PRESENCE, never its content. Both
    # headers carry the SAME token: `Authorization: Bearer` authenticates
    # the call at all (`auth_gate`'s floor); `x-run-token` is
    # `check_employee_run_auth`'s own belt-and-suspenders check inside the
    # handler (see the module doc comment above).
    if not cfg.run_token:
        return {}
    return {
        "x-run-token": cfg.run_token,
        "Authorization": f"Bearer {cfg.run_token}",
    }


def post_run(cfg: AgentRunConfig, employee_id: str) -> requests.Response:
    """POST `/api/agents/employees/{employee_id}/run` and return the raw
    response — status-code handling (409 vs. other 4xx/5xx vs. 2xx) is the
    caller's job (`run_agent_employee`), since each means something
    different here and none should be flattened into a single
    `raise_for_status()` call."""
    return requests.post(
        f"{cfg.api_url}/api/agents/employees/{employee_id}/run",
        headers=_headers(cfg),
        timeout=120,
    )


@op(
    config_schema={
        "employee_id": Field(
            str,
            description=(
                "The `agent_employee.id` to run headlessly, e.g. "
                "'emp-daily-quality-check'. Set via this op's run config "
                "(Dagster launchpad or a future schedule's `run_config_fn`)."
            ),
        )
    }
)
def run_agent_employee(context) -> dict[str, Any]:
    """Runs `POST /api/agents/employees/{employee_id}/run` for the employee
    named in this op's run config, and records the outcome via
    `record_maintenance_run` (the same `bronze_meta.*` registry
    `maintenance.py`/`gold_export.py` write to, so a copilot employee run
    shows up alongside them in `GET /api/governance/maintenance` without a
    new console surface — per R10, reuse that mechanism rather than invent
    a parallel one).

    Response handling (see `run_employee`'s doc comment in
    `rust/crates/lakehouse-api/src/routes/agents.rs` for the exact shapes):

    * 2xx — the run reached a terminal status (`succeeded`/`failed`) OR
      `waiting_approval` (a `WriteHigh` tool call inside the run is now
      sitting in the approvals inbox). All three are LOGGED, not raised:
      `waiting_approval` is a legitimate, expected pause — a human decides
      later via `/agents/approvals` — and `failed` is the employee's OWN
      run failing (e.g. its prompt asked for something the LLM couldn't
      do), not this job failing to trigger it. This job's job is "did the
      trigger succeed", not "did the employee's task succeed".
    * 409 — the employee is `paused` (suspended) or `cancelled` (revoked).
      Logged as a warning and treated as this op's SUCCESS, not a failure:
      an operator suspending an employee is an intentional, in-band action
      (the same reason `bronze_maintenance_job` doesn't fail when there are
      zero Bronze tables to maintain) — a suspended employee's schedule
      firing and finding nothing to do is the system working as designed,
      not an incident. Contrast this with 401/403/404/5xx below, which DO
      raise, because those mean the trigger itself is broken.
    * 4xx/5xx other than 409 — raised as `requests.HTTPError` after being
      logged (never with the token in the message — `requests`' own
      `HTTPError` string is the method/URL/status, not headers, so this is
      safe by construction, not by redaction). `record_maintenance_run`
      still gets a row so the failure is visible in the governance surface,
      exactly like `gold_export.py`'s `run_gold_export` does on export
      failure."""
    employee_id: str = context.op_config["employee_id"]
    cfg = AgentRunConfig.from_env()
    context.log.info(f"running digital employee {employee_id!r} headlessly")

    try:
        resp = post_run(cfg, employee_id)
    except requests.RequestException as exc:
        context.log.error(f"run trigger for {employee_id!r} could not reach lakehouse-api: {exc}")
        record_maintenance_run(
            table_name=f"agent_employee.{employee_id}",
            dry_run_metrics={},
            applied_metrics={},
            skipped_verbs=[f"run_trigger_unreachable: {exc}"],
            target=ClickHouseTarget.from_env(),
        )
        raise

    if resp.status_code == 409:
        # Suspended/revoked — see the doc comment above for why this is a
        # deliberate success, not a failure.
        context.log.warning(
            f"employee {employee_id!r} is suspended/revoked (409); nothing to run"
        )
        record_maintenance_run(
            table_name=f"agent_employee.{employee_id}",
            dry_run_metrics={},
            applied_metrics={},
            skipped_verbs=["run_skipped: employee suspended or revoked (409)"],
            target=ClickHouseTarget.from_env(),
        )
        context.add_output_metadata({"employee_id": employee_id, "outcome": "skipped_409"})
        return {"employee_id": employee_id, "outcome": "skipped_409"}

    try:
        resp.raise_for_status()
    except requests.HTTPError as exc:
        body_preview = resp.text[:500]
        context.log.error(
            f"run trigger for {employee_id!r} failed: {exc} — body: {body_preview}"
        )
        record_maintenance_run(
            table_name=f"agent_employee.{employee_id}",
            dry_run_metrics={},
            applied_metrics={},
            skipped_verbs=[f"run_trigger_failed: {exc}"],
            target=ClickHouseTarget.from_env(),
        )
        raise

    body = resp.json()
    run = body.get("run", {})
    run_id = run.get("id", "unknown")
    status = run.get("status", "unknown")
    context.log.info(f"employee {employee_id!r} run {run_id!r} ended with status {status!r}")

    record_maintenance_run(
        table_name=f"agent_employee.{employee_id}",
        dry_run_metrics={},
        applied_metrics={},
        skipped_verbs=[f"run_id={run_id} status={status}"],
        target=ClickHouseTarget.from_env(),
    )
    context.add_output_metadata({"employee_id": employee_id, "run_id": run_id, "status": status})
    return {"employee_id": employee_id, "run_id": run_id, "status": status}


@job
def agent_run_job() -> None:
    """`DAGSTER_LOCATION`-visible job name: `agent_run_job`. Launch with run
    config `{"ops": {"run_agent_employee": {"config": {"employee_id": "..."}}}}`
    — either from the Dagster launchpad for an on-demand run, or from one
    of `agent_run_schedules` below."""
    run_agent_employee()


def _employee_run_config(employee_id: str) -> dict[str, Any]:
    """The run config `agent_run_job` needs for one `employee_id` — shared
    by every generated `ScheduleDefinition` so there is exactly one place
    that knows this job's config shape."""
    return {"ops": {"run_agent_employee": {"config": {"employee_id": employee_id}}}}


def _fetch_schedulable_employees(cfg: AgentRunConfig) -> list[dict[str, Any]]:
    """`GET /api/agents/employees` (with the same `Authorization: Bearer`
    credential `post_run` sends), filtered to employees carrying a
    non-empty `scheduleCron`. Returns `[]` on ANY failure — see the module
    doc comment ("The schedule factory, and why it cannot ever crash this
    code location") for why this must never raise: it runs at Dagster
    code-load time, alongside every other job/schedule in this code
    location.

    Also returns `[]` immediately, with no HTTP call at all, when
    `cfg.run_token` is empty — an unset `AGENT_RUN_TOKEN` means "digital
    employee schedules are inert", not "try anonymously and see what
    happens" (the route is `Policy::RequiresAuth`; an unauthenticated call
    would 401 every single time regardless)."""
    if not cfg.run_token:
        print(
            "dispar_orchestrate.agent_runs: AGENT_RUN_TOKEN is unset; "
            "loading with zero digital-employee schedules"
        )
        return []

    try:
        resp = requests.get(
            f"{cfg.api_url}/api/agents/employees",
            headers=_headers(cfg),
            timeout=10,
        )
        resp.raise_for_status()
    except requests.RequestException as exc:
        print(
            f"WARNING: dispar_orchestrate.agent_runs: could not reach "
            f"lakehouse-api to build digital-employee schedules ({exc}); "
            "loading with zero agent schedules"
        )
        return []

    try:
        employees = resp.json()
    except ValueError as exc:
        print(
            f"WARNING: dispar_orchestrate.agent_runs: lakehouse-api returned "
            f"a non-JSON /api/agents/employees body ({exc}); loading with "
            "zero agent schedules"
        )
        return []

    if not isinstance(employees, list):
        print(
            "WARNING: dispar_orchestrate.agent_runs: /api/agents/employees "
            f"returned {type(employees).__name__}, expected a list; loading "
            "with zero agent schedules"
        )
        return []

    return [
        employee
        for employee in employees
        if isinstance(employee, dict) and employee.get("id") and employee.get("scheduleCron")
    ]


def _schedule_name(employee_id: str) -> str:
    """A Dagster-legal schedule name derived from `employee_id`. Dagster
    names must match `^[A-Za-z0-9_]+$` (`dagster._core.definitions.utils.check_valid_chars`)
    — `agent_employee.id` values in this repo are conventionally
    hyphenated (e.g. `emp-daily-quality-check`, `emp-copilot`), so hyphens
    (and any other non-matching character) are replaced with `_` rather
    than rejected outright. Collisions between two employee ids that only
    differ by hyphen-vs-underscore are not a concern this schema's own
    `agent_employee.id` primary key already rules out (two distinct ids
    can't exist), so this mapping never needs to be reversed."""
    return "agent_run_schedule__" + "".join(
        char if char.isalnum() or char == "_" else "_" for char in employee_id
    )


def build_agent_run_schedules(cfg: AgentRunConfig) -> list[ScheduleDefinition]:
    """One `ScheduleDefinition` per employee `_fetch_schedulable_employees`
    returns, named after the employee id (via `_schedule_name`) so two
    employees never collide and a schedule's identity is stable across
    reloads. Never raises (see `_fetch_schedulable_employees`)."""
    schedules: list[ScheduleDefinition] = []
    for employee in _fetch_schedulable_employees(cfg):
        employee_id = employee["id"]
        cron = employee["scheduleCron"]
        schedules.append(
            ScheduleDefinition(
                name=_schedule_name(employee_id),
                cron_schedule=cron,
                job=agent_run_job,
                run_config=_employee_run_config(employee_id),
            )
        )
    return schedules


# Built once, at code-load time (module import), per the plan (§3.4):
# "a schedule factory that reads employees with schedule_cron from
# lakehouse-api ... at code-load time and builds one ScheduleDefinition
# per employee". See `build_agent_run_schedules`/
# `_fetch_schedulable_employees` for why this expression can never raise —
# it degrades to `[]` instead. `agent_run_job` stays registered in
# `definitions.py` regardless of whether any schedule was built, so it
# remains launchable on demand from the Dagster UI, exactly how
# `gold_export_job` does.
agent_run_schedules: list[ScheduleDefinition] = build_agent_run_schedules(AgentRunConfig.from_env())

agent_runs_defs = Definitions(
    jobs=[agent_run_job],
    schedules=agent_run_schedules,
)
