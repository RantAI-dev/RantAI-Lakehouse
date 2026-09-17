"""Builds one real Dagster job per Postgres-authored, `ready`-status
pipeline (WS4 item E1, grand plan §6): `createPipeline`/
`generatePipelineFromPrompt` (Phase 2, Task 2.5) write a
`pipeline_definition` row that previously sat inert forever -- no engine
executed it. This module reads `GET /api/pipelines` at Dagster code-load
time (same pattern `agent_runs.py` uses for digital employees) and, for
every authored row with `status == "ready"` and a `definition` payload,
builds an `authored__<id>` job (WS4 item E3, not yet in this commit) that
reads its source, applies its `transforms`, and writes its target.

# Verified gap: `GET /api/pipelines` does not yet carry `definition` (item E1)

This module's filter (`_fetch_authored_pipelines`) looks for a
`definition` key on each authored row, matching the shape
`GET /api/pipelines/{id}` is documented to return
(`lakehouse_store::pipelines::get_definition`,
`rust/crates/lakehouse-store/src/pipelines.rs:184-221`, whose doc comment
names it as "the `definition` field `GET /api/pipelines/{id}` reports").
Read on THIS branch (WS4 items E1/E3/E4/H1 only -- the detail ROUTE is
WS4 phase C item C1, out of this task's scope and not yet landed):
`rust/crates/lakehouse-api/src/routes/mod.rs` registers no
`GET /api/pipelines/{id}` route at all, and `list_body`
(`routes/pipelines.rs:55-68`) serializes each authored row through
`lakehouse_store::pipelines::Pipeline`, which has no `transforms`/
`fbic_enabled`/`incremental_column` field (`lakehouse-store/src/
pipelines.rs:34-70`). So on a real running stack built from this branch
alone, every authored row's `definition` is genuinely absent and
`_fetch_authored_pipelines` correctly returns it filtered out -- this is
not a bug in this module, it is this module honestly reporting what the
API it depends on does not (yet) expose, per `AGENTS.md`'s "never
fabricate" rule. `build_authored_jobs()` (WS4 item E3) will start producing
jobs the moment WS4 item C1's route lands and starts including `definition`;
nothing here needs to change for that to happen.

# Verified gap: the pipeline-run service identity cannot itself read the list (item E1)

`GET /api/pipelines` is `Policy::RequiresPermission("pipeline:read")`
(`rust/crates/lakehouse-api/src/policy.rs:239`), but the
`authored-pipeline-scheduler` service identity `PIPELINE_RUN_TOKEN`
authenticates as (`main.rs::bootstrap_pipeline_run_service`, WS4 item G3,
already on this branch) is deliberately scoped to `pipeline:write` ONLY --
see that function's own doc comment ("never `*:*`"). A real
`PIPELINE_RUN_TOKEN` therefore gets a 403 from `GET /api/pipelines`, which
`_fetch_authored_pipelines` treats the same as any other non-2xx response:
logged, and an empty list. This is a real, load-bearing consequence of a
decision made in a task outside this one's scope (G3) -- broadening that
identity's scope is a `rust/` change this task set does not make. Noted
here rather than hidden, per `docs/superpowers/plans/2026-09-11-ws4-pipelines-detail-source-logs.md`'s
own H1 gate script comment, which names the identical fact for the same
reason.

Authenticates the same way `agent_runs.py`/`gold_export.py` do: a bearer
token from `PIPELINE_RUN_TOKEN`.
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from typing import Any

import requests


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


def _fetch_authored_pipelines(cfg: AuthoredPipelineConfig) -> list[dict[str, Any]]:
    """Fetch every `status == "ready"` authored pipeline that carries a
    `definition` payload, or an EMPTY list on any failure -- never raises
    (mirrors `agent_runs._fetch_schedulable_employees`; this module
    imports at Dagster code-load time, alongside every other job/schedule
    in this code location, so an uncaught exception here would take all of
    them down).

    The three distinct "empty list" causes below are NOT the same fact and
    are logged distinctly, per WS4 item E1: (1) `PIPELINE_RUN_TOKEN` unset
    -- authored scheduling is deliberately inert, not attempted; (2) the
    HTTP call itself failed (unreachable host, connection refused, a
    non-2xx status including the real, expected 403 documented in this
    module's own doc comment above) -- `lakehouse-api` was reached (or an
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
            f"{cfg.api_url}/api/pipelines",
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
            f"returned a non-JSON /api/pipelines body ({exc}); this is a FETCH "
            "FAILURE, not evidence that no authored pipelines exist -- loading "
            "with zero authored-pipeline jobs"
        )
        return []

    pipelines = body.get("pipelines", [])
    if not isinstance(pipelines, list):
        print(
            "WARNING: dispar_orchestrate.authored_factory: /api/pipelines "
            f"returned {type(pipelines).__name__} for 'pipelines', expected a "
            "list; this is a FETCH FAILURE, not evidence that no authored "
            "pipelines exist -- loading with zero authored-pipeline jobs"
        )
        return []

    return [
        p
        for p in pipelines
        if isinstance(p, dict) and p.get("status") == "ready" and p.get("definition")
    ]
