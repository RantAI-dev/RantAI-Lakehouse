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

# Why there is no live schedule factory in this module

The plan (`docs/superpowers/plans/2026-09-08-copilot-operations-handover.md`,
§3.4) describes building one `ScheduleDefinition` per employee by reading
`GET /api/agents/employees` at Dagster code-load time. That route is
`Policy::RequiresAuth` in `rust/crates/lakehouse-api/src/policy.rs` —
verified directly against `rust/crates/lakehouse-api/src/policy.rs::auth_gate`
and its own test suite: `auth_gate` demands a REAL credential (a session
cookie or an `Authorization: Bearer` service/OIDC token) BEFORE the handler
ever runs, for every `RequiresAuth`/`RequiresPermission` route. `AGENT_RUN_TOKEN`
(the `x-run-token` header this module sends) is not such a credential — it is
checked only inside `run_employee`'s own body, which `auth_gate` never
reaches without a bearer/cookie first. `rust/crates/lakehouse-api/tests/agents_run.rs::valid_token_runs_the_employee_to_a_terminal_status_with_steps`
proves this directly: even a VALID `x-run-token` still 401s without an
accompanying session cookie in the test, and the comment there says so
explicitly ("The router's `Policy::RequiresAuth` floor still needs a REAL
credential ... regardless of `x-run-token`").

This compose stack provisions no service-identity bearer token for Dagster
(no `service_credential` row, no minted token file mounted into the
`dagster-code-location` container) — exactly the same gap `gold_export_job`
documents for itself. So:

* Building a schedule factory that calls `GET /api/agents/employees` at
  code-load time would, on this stack, ALWAYS get a 401 back. Doing that
  unconditionally would either (a) crash the whole code location the moment
  Dagster loads it (breaking bronze ingest, maintenance, and the slot check
  too — unacceptable), or (b) require swallowing every error into "zero
  schedules" — at which point the factory is dead code that can never
  produce a schedule under any config this repo actually ships, which is
  worse than not writing it: a reader sees a factory function and
  reasonably assumes it sometimes fires.
* Per the plan's own instruction for this exact situation ("if you cannot
  authenticate ... follow the gold_export_job precedent: register the job
  WITHOUT schedules, document precisely why"), `agent_run_job` below is
  registered in `definitions.py` with NO schedule attached, and none is
  invented here.

To enable real schedules: give Dagster a genuine credential — mint a
`service_credential` for a `schedule` principal (same primitive
`gold_export.py`'s Lakekeeper-side token file uses, just for `lakehouse-api`
auth instead), mount it into `dagster-code-location` the way
`LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE` is mounted, send it as
`Authorization: Bearer <token>` from a schedule-factory HTTP call to
`GET /api/agents/employees` at code-load time, and STILL wrap that call so a
transient failure yields zero schedules and a loud `context.log`-equivalent
warning (Dagster evaluates code locations synchronously at load time, so
`print`/`warnings.warn` is what is available outside an op/job body) rather
than an exception — that half of the plan's caution applies regardless of
how auth eventually gets solved.
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from typing import Any

import requests

from dagster import Definitions, Field, job, op
from dispar_orchestrate.bronze_catalog import ClickHouseTarget, record_maintenance_run


def _env(name: str, default: str) -> str:
    value = os.environ.get(name, "").strip()
    return value if value else default


@dataclass(frozen=True)
class AgentRunConfig:
    api_url: str
    # D4 shape (`routes::agents::check_employee_run_auth`): a shared token,
    # set identically here and on `lakehouse-api` (both read
    # `AGENT_RUN_TOKEN` from the same compose `.env`). As the module doc
    # above explains, this token alone does NOT clear `auth_gate`'s
    # `Policy::RequiresAuth` floor on `POST /api/agents/employees/{id}/run`
    # — a real bearer/session credential is also required, which this job
    # does not carry today. Kept anyway (never invented away) so the run
    # at least reaches the handler's own token check the moment a service
    # credential is added, without a second code change here.
    run_token: str

    @classmethod
    def from_env(cls) -> AgentRunConfig:
        return cls(
            api_url=_env("LAKEHOUSE_API_URL", "http://lakehouse-api:8080"),
            run_token=_env("AGENT_RUN_TOKEN", ""),
        )


def _headers(cfg: AgentRunConfig) -> dict[str, str]:
    # Never log this value — see `run_agent_employee`'s use of it below,
    # which only ever logs the header's PRESENCE, never its content.
    return {"x-run-token": cfg.run_token} if cfg.run_token else {}


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
    — either from the Dagster launchpad for an on-demand run, or from a
    future schedule's `run_config_fn` once a real service credential lets a
    schedule authenticate (see this module's doc comment)."""
    run_agent_employee()


# NOT scheduled — see this module's doc comment for the full reasoning.
# `agent_run_job` is registered in `definitions.py` so it remains launchable
# on demand (e.g. an operator triggering one employee's run manually from
# the Dagster UI), exactly how `gold_export_job` stays registered without a
# schedule.

agent_runs_defs = Definitions(
    jobs=[agent_run_job],
)
