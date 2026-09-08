# Copilot Operations Expansion — Design + Implementation Plan (handover)

**Status:** approved design, not started. Written 2026-09-08 for handover to
another Claude session (Opus). No code has been written for this plan.

**Base commit:** `4e02361` (rebased P0–P6 + R1 + Gold-export stack). Work on
a new branch off that commit, e.g. `lakehouse/p7-copilot-ops`. The working
tree at handover time carries unrelated demo-rebranding edits
(`src/services/mock/*`, `next.config.ts`, deleted `package-lock.json`,
new `src/lib/tenant.ts`); do not fold them into this branch.

**Goal, in the user's words:** the AI copilot should be able to "do
anything" the console can do, through Tier 1 → Tier 3 below, with every
write governed. The sales context is `GTM/ON-PREM-SALES-PLAYBOOK.md`: after
this lands we can honestly say "an AI copilot that queries, builds
dashboards, manages alerts and connectors, runs pipelines and maintenance,
with every write approved by a human, and digital employees that run on a
schedule".

---

## 1. Decisions already made by the user (do not re-ask)

| Decision | Choice |
| --- | --- |
| Approval gate | **Two-level.** Low-risk writes execute after an inline confirm in chat. High-risk writes create a pending `approval_item` and block until a human decides on `/agents/approvals`. |
| Audit sink | **New Postgres table `audit_event`**, append-only; `GET /api/governance/audit` unions it with the existing Dagster/ClickHouse history. |
| Scheduled agent runs | **Dagster job + schedule** that POSTs `/api/agents/employees/{id}/run` with an `x-run-token`, mirroring `gold_export_job`. |
| Scope | Tier 1, Tier 2, Tier 3 (below). Document retrieval / RAG is **out of scope** (no vector store exists). |

## 2. What exists today (anchors, verified at 4e02361)

- Copilot backend: `rust/crates/lakehouse-api/src/routes/ai.rs`
  - `tool_schemas()` at ~line 130 returns the JSON tool list.
  - `run_tool(state, name, args)` at ~line 234 is a single `match name`.
  - `WRITE_TOOLS: [&str; 5]` at line 84 + `write_tool_refusal()` at line 110
    is the dispatch-site gate for Ask mode. Keep this pattern; extend it.
  - `chat()` at line 728 takes `State<AppState>` and `Bytes` only. It does
    **not** currently receive the `Principal`. Handlers that need it use
    `principal: Option<Extension<Principal>>` (see `routes/gold.rs:121`,
    `routes/alerts.rs:227`).
  - System prompt constants `SYSTEM_BASE` / `SYSTEM_ASK_SUFFIX` /
    `SYSTEM_BUILD_SUFFIX` at lines 682–686 (Indonesian). `MAX_ITER = 8`.
  - Sessions persist in ClickHouse `console.chat_session` (line 1091).
  - `parse_minimax_tool_calls()` extracts XML tool calls from free text, so
    every tool name that reaches `run_tool` must be gated at dispatch.
- Permissions: `lakehouse-auth/src/permissions.rs` — `PermissionSet::has("resource:action")`
  with whole-segment wildcards. Roles seeded in migrations 0019/0020.
- Route policy: `lakehouse-api/src/policy.rs` `POLICY_TABLE`. A route with
  no entry returns 500 `route_policy_unclassified`; `tests/route_auth.rs`
  exercises every entry. Existing perms of interest: `agent:manage`,
  `agent:approve`, `connector:manage`, `identity:write`.
- Run-token pattern: `routes/gold.rs::check_export_token` and
  `routes/alerts.rs::check_run_token` (`x-run-token` header or `?token=`,
  else a service-identity principal). Config field
  `gold_export_run_token`. Copy this shape for the agent-run token.
- Agents domain: migration `rust/migrations/0017_agents.sql` — tables
  `agent_employee`, `agent_tool`, `agent_workflow`, `agent_run` (JSONB
  `steps`, never written by a live path today), `approval_item` (status
  pending/approved/rejected, `decide_approval` at
  `lakehouse-store/src/agents.rs:744` does `SELECT … FOR UPDATE` then
  `UPDATE`). Routes in `routes/agents.rs`; `POST /api/agents/approvals/{id}/decide`
  requires `agent:approve`.
- Dagster: `dagster/dispar_orchestrate/definitions.py` registers 4 jobs, 2
  schedules; `gold_export.py` shows a job that calls lakehouse-api with a
  run token. `gold_export_job` is deliberately unscheduled because it has no
  service identity; the same constraint applies to agent runs, so the new
  job must read its token from env and be documented the same way.
- Existing route/store functions to wrap as tools (call the **store** or a
  shared helper, never re-implement SQL in `ai.rs`):
  - alerts: `lakehouse-alerts` crate + `routes/alerts.rs` (CRUD + `run`).
  - connectors: `routes/connectors.rs`, `store/connectors.rs`,
    `connector_probe.rs` (SSRF guard + secretRef allowlist live here; tools
    must go through the same code path).
  - pipelines: `routes/pipelines.rs` (list/runs/trigger/pause/resume/cancel/retry)
    over `lakehouse-dagster`.
  - saved queries: `routes/query.rs` (`saved`, `history`), read-only guard
    at `routes/query.rs:61-133` (reuse for any SQL the copilot runs).
  - governance: `routes/governance.rs` + `store/governance.rs`
    (`quality|classification|audit|residency`, policies CRUD, maintenance
    metrics, replication slot health).
  - ops: `routes/ops.rs` workloads + `POST /api/ops/workloads/{id}/cancel`
    (real `KILL QUERY`, line ~410).
  - gold: `routes/gold.rs` + `gold_export.rs`.
  - maintenance: `/api/governance/maintenance` — check how it triggers
    `bronze_maintenance_job` (Dagster launch via `lakehouse-dagster`) and
    whether dry-run vs applied is a run-config flag; the tool must expose
    both.
- Frontend copilot: `src/features/copilot/` — `capabilities.ts` (3 groups
  wrapping ~15 tools), `tool-step.tsx` (renders a tool call), `use-copilot.ts`
  (chat loop, mode), `copilot-dock.tsx` (in-page dock, page context).
  Approvals page client: `src/services/clients/agents.ts`.
- Conventions: `clippy::all = deny`, `unwrap_used`/`expect_used` = deny,
  `missing_docs = warn` (`rust/Cargo.toml:45-58`). Migrations are numbered;
  latest is `0022_prune_connector_seed.sql`, so start at `0023`. Store
  tests use `#[sqlx::test(migrations = "../../migrations")]`; external
  clients are tested with wiremock. Every new route needs a `POLICY_TABLE`
  entry or the router test fails.

## 3. Architecture

### 3.1 Tool registry refactor (prerequisite, Tier 0)

`ai.rs` is ~1,300 lines with tools declared in two places (schema list and
match). Adding ~35 tools there is unmaintainable. Introduce a small
registry, still inside `lakehouse-api`:

```
routes/ai/
  mod.rs          chat(), sessions (unchanged behaviour)
  registry.rs     ToolSpec { name, description, schema, risk: Risk, permission: &'static str }, static TOOLS: &[ToolSpec], lookup by name
  gate.rs         decide(mode, principal, spec) -> Allow | RefuseAskMode | RefusePermission | NeedsConfirm | NeedsApproval
  audit.rs        record(pool, AuditEvent) — thin wrapper over store::audit
  tools/
    data.rs       run_sql, list_datasets, describe_dataset, get_lineage, get_quality, describe_mart   (moved, unchanged)
    dashboards.rs create/update/delete_chart, create/list_boards, suggest_dashboard, list_charts     (moved, unchanged)
    alerts.rs     Tier 1
    connectors.rs Tier 1
    pipelines.rs  Tier 1 (+ existing trigger/build_status moved here)
    queries.rs    Tier 1
    governance.rs Tier 2
    ops.rs        Tier 2
    maintenance.rs Tier 2
    gold.rs       Tier 2
```

`Risk` enum: `Read`, `WriteLow`, `WriteHigh`. `WRITE_TOOLS` array is
replaced by `spec.risk != Read`; keep a regression test asserting every
existing write tool still maps to a non-Read risk (the D3 test).

`chat()` gains `principal: Option<Extension<Principal>>` and passes the
principal's `PermissionSet` into the gate. Tools declare the same
`resource:action` the equivalent route uses, so the copilot can never do
more than the logged-in user. Unauthenticated `chat` is already impossible
(`RequiresAuth`).

### 3.2 Approval gate

Flow inside the tool loop, per call:

1. `gate::decide`:
   - Ask mode and `risk != Read` → refusal result (existing behaviour).
   - `!perms.has(spec.permission)` → refusal result `{"error": "...", "refused": true, "reason": "permission"}`.
   - `WriteLow` and the call does not carry `"confirmed": true` in args →
     return a **pending-confirmation** result
     `{"needs_confirmation": true, "tool": name, "args": args, "summary": "..."}`.
     The model relays it; the frontend renders a Confirm button which re-sends
     the same tool call with `confirmed: true` (see 3.5). Nothing executes.
   - `WriteHigh` → insert an `approval_item` (status pending, `action` =
     tool name, `resource` = target id, `reason` = model's stated reason,
     `evidence` = args JSON, `employee_id` = the reserved copilot employee
     row, see 3.4) **and** an `agent_run` row with status `waiting_approval`
     holding the tool call; return
     `{"needs_approval": true, "approval_id": ..., "run_id": ...}`.
     Execution happens later, from the decide route (next bullet).
2. `POST /api/agents/approvals/{id}/decide` with `approved`: after the
   existing `decide_approval`, load the linked `agent_run`, execute the
   stored tool call through the same `run_tool` (bypassing the gate with an
   explicit `Approved` marker, never by skipping permission checks: check
   the **approver's** permissions too), append the result to `steps`, set
   run status `succeeded|failed`, write an audit event. Rejected → run
   status `rejected`.
3. Every gate decision and every executed tool writes one `audit_event`.

Risk assignment (initial; keep in `registry.rs` as data):

| WriteLow (inline confirm) | WriteHigh (approvals inbox) |
| --- | --- |
| create/update chart, create board, save_query, create/update alert_rule, run_alert_rule, trigger_pipeline, retry_pipeline_run, create_connector (test only dials), maintenance_dry_run, draft_policy/draft_classification_rule/draft_quality_rule, gold_export | delete_chart, delete_alert_rule, delete_connector, pause/resume/cancel pipeline, kill_query, maintenance_apply, activate_policy (if exposed at all; prefer draft-only) |

### 3.3 Audit sink

Migration `0023_audit_event.sql`:

```
audit_event(
  id TEXT PK, at TIMESTAMPTZ default now(),
  principal_id TEXT, principal_kind TEXT,          -- user | service | copilot | schedule
  actor_label TEXT,                                 -- display
  action TEXT NOT NULL,                             -- tool name or route verb
  resource_kind TEXT, resource_id TEXT,
  args JSONB NOT NULL DEFAULT '{}',                 -- redacted: never secretRef values, never SQL results
  outcome TEXT NOT NULL,                            -- allowed | executed | refused | needs_confirmation | needs_approval | approved | rejected | failed
  detail TEXT,
  run_id TEXT REFERENCES agent_run(id) ON DELETE SET NULL,
  approval_id TEXT REFERENCES approval_item(id) ON DELETE SET NULL,
  session_id TEXT                                   -- chat session
)
```
Index on `(at DESC)` and `(resource_kind, resource_id)`. Store module
`lakehouse-store/src/audit.rs` with `insert` and `list(filter, limit,
cursor)`. `GET /api/governance/audit` unions these rows into its existing
response shape (add `source: "copilot"|"pipeline"`). Also set
`agent_run.audit_event_id` / `approval_item.audit_event_id` (columns exist
and are currently unused).

Optionally, in the same change, have `routes/auth.rs` login/logout and the
identity mutations write `audit_event` rows too. That closes the "no audit
sink" gap from the playbook cheaply. Keep it a separate task so it can be
dropped.

### 3.4 Scheduled agent runs ("digital employees")

- Migration `0024_agent_schedule.sql`: add to `agent_employee`:
  `prompt TEXT`, `schedule_cron TEXT NULL`, `mode TEXT default 'build'`,
  `run_token_hash TEXT NULL` is **not** needed; one shared
  `AGENT_RUN_TOKEN` config value, mirroring `gold_export_run_token`.
  Reserve one employee row `emp-copilot` (`name: "Copilot (interactive)"`)
  that interactive chat attributes its approvals/runs to.
- Route `POST /api/agents/employees/{id}/run` — policy entry: token guard
  like gold (`x-run-token` else service-identity principal with
  `agent:manage`). Body optional `{ "prompt": "...override..." }`. Creates
  an `agent_run` (trigger `schedule|manual`, actor = employee), runs the
  same chat loop **headless** with the employee's prompt, mode, and a
  synthetic principal whose `PermissionSet` is the employee's
  `allowed_tools`/scope (map to permissions; if the employee table has no
  permission column, add `permissions TEXT` in 0024). Appends each tool
  call/result to `steps`, sets `budget_consumed` from token usage if the LLM
  client exposes it, ends `succeeded|failed|waiting_approval`.
- Route `POST /api/agents/employees/{id}/run` for interactive "Run now"
  from the employee detail page, same handler, `RequiresPermission("agent:manage")`.
- Dagster: `dagster/dispar_orchestrate/agent_runs.py` — one job
  `agent_run_job` with config `employee_id`; a **schedule factory** that
  reads employees with `schedule_cron` from lakehouse-api
  (`GET /api/agents/employees`, needs a service token in env) at code-load
  time and builds one `ScheduleDefinition` per employee. Register in
  `definitions.py`. Document, exactly as `gold_export_job` does, that the
  schedule is inert until `AGENT_RUN_TOKEN` is set in compose. Add the
  env passthrough to `docker-compose.yml` and `.env.example`.
- Frontend: `/agents/employees/[id]` gets prompt, cron, mode, permissions
  fields and a "Run now" button; `/agents/runs/[id]` renders `steps` as the
  same tool-step component the copilot uses. Delete the seeded fake runs
  in a migration (`0025_drop_seeded_agent_runs.sql`) so the runs page only
  shows real history, or leave them flagged `trigger = 'seed'` and hide
  by default. Prefer delete.

### 3.5 Frontend changes (copilot)

- `capabilities.ts`: add groups Alerts, Connectors, Pipelines, Governance,
  Operations, each listing its tools; groups gate which tool schemas are
  sent (existing behaviour).
- `tool-step.tsx`: render three new result shapes:
  `needs_confirmation` → summary + **Confirm** / **Cancel** buttons; Confirm
  re-sends the tool call with `confirmed: true` as a user-turn tool
  invocation (add a small `resumeTool(callId, args)` in `use-copilot.ts`
  that appends a synthetic message the backend recognises, or simpler: a
  new `POST /api/ai/tool` route that executes one gated tool call directly
  and returns the result, which the UI then appends to the transcript).
  Recommendation: the direct route; it avoids re-prompting the LLM to
  re-emit the call.
  `needs_approval` → card with link to `/agents/approvals?id=…`.
  `refused` with `reason: permission` → plain explanation.
- Approvals page: show `evidence` (args) and the originating chat session
  link; after Approve, show the execution result inline (returned by the
  decide route).
- Governance → Audit: new "Copilot" source filter.

### 3.6 Safety invariants (write tests for each)

1. Ask mode never executes a non-Read tool, regardless of how the call
   arrived (existing D3 test, extended to the registry).
2. A principal lacking `spec.permission` never executes that tool, in chat,
   via `/api/ai/tool`, or via approval execution.
3. `WriteHigh` never executes without an `approval_item` row in `approved`
   status decided by a principal with `agent:approve`.
4. No tool accepts a raw credential; connector tools accept `secretRef`
   only and resolve through `AllowlistedSecretResolver` (existing).
5. Every tool execution writes exactly one `audit_event`; args stored are
   redacted (no `secretRef` values, no SQL result rows).
6. `run_sql` and any SQL-bearing tool pass the read-only guard in
   `routes/query.rs`.
7. Scheduled runs cannot exceed the employee's permissions; a suspended or
   revoked employee's run route returns 409.

## 3.7 Corrections made during execution (2026-09-08, verified against code)

These override the text above where they conflict.

**C1 — Exact permission strings for tool specs** (from `policy.rs`, verified):

| Area | Read | Write |
| --- | --- | --- |
| alerts | `RequiresAuth` (empty string) | `alert:write` |
| connectors | `connector:manage` | `connector:manage` (incl. `/test` and DELETE) |
| pipelines | `pipeline:read` | `pipeline:write` (trigger/pause/resume/cancel/retry) |
| query | `query:read` | `query:read` (saved/history) |
| ops workloads | `RequiresAuth` | `workload:cancel` |
| governance lineage | `lineage:read` | — |
| governance policies | `policy:read` | `policy:write` |
| governance `{kind}` (quality, classification, audit, residency, maintenance, replication) | `RequiresAuth` | `RequiresAuth` |
| agents | `RequiresAuth` | `agent:manage`, approvals `agent:approve` |

**C2 — T2.2 as written is not implementable.** There is **no** maintenance
trigger route: `GET /api/governance/maintenance` is read-only, reading
`lake.bronze_meta.maintenance_run` from ClickHouse
(`routes/governance.rs:301`). And `bronze_maintenance_job`
(`dagster/dispar_orchestrate/maintenance.py:297-299`) **always runs a
dry-run and then the applied run in one job**; there is no config switch,
and `DgClient::launch_run(job_name)` (`lakehouse-dagster/src/lib.rs:439`)
accepts a job name only, with no run config.

Revised T2.2: ship **one** tool `run_bronze_maintenance`, risk
`WriteHigh` (it applies changes), calling `launch_run("bronze_maintenance_job")`,
plus the read-only `get_maintenance_metrics`. A genuine dry-run-only mode
is a separate, larger change (Dagster job config + `launch_run` run-config
support) and is explicitly **out of scope** here. Do not advertise a
"dry run" copilot tool that actually applies changes.

**C3 — Reuse route handlers, do not re-implement.** Every Tier 1/2 tool
must call the same store function or handler helper the HTTP route uses, so
guards (SSRF, secretRef allowlist, read-only SQL) cannot be bypassed.

## 4. Tiered task list

Each task: branch-local, TDD (store tests with `sqlx::test`, route tests in
`tests/`, wiremock for LLM), `cargo clippy --all-targets` clean, docs
updated. Suggested order is dependency order; T0 must land first, T1 tasks
are independent of each other after T0.

### Tier 0 — foundation

- **T0.1 Registry refactor.** Move existing 15 tools into `routes/ai/`
  layout (3.1) with zero behaviour change. Tests: existing ai tests pass;
  new test asserts tool names and schemas are byte-identical before/after
  (snapshot the JSON of `tool_schemas()` first).
- **T0.2 Principal + permission gate.** `chat` receives the principal;
  gate refuses tools whose permission the principal lacks. Tests: analyst
  cannot `trigger_lakehouse_build`; platform admin can.
- **T0.3 Audit table + store + governance union.** Migration 0023, store
  module, `GET /api/governance/audit` union, every tool execution and gate
  refusal audited. Tests: store insert/list; audit row per execution.
- **T0.4 Inline confirmation + `/api/ai/tool` route.** `WriteLow` flow,
  policy entry, frontend Confirm button. Tests: unconfirmed call returns
  `needs_confirmation` and executes nothing; confirmed call executes and
  audits.
- **T0.5 Approvals-inbox flow.** `WriteHigh` creates `approval_item` +
  `agent_run`; decide route executes on approve. Frontend approvals page
  shows evidence and result. Tests: invariant 3; reject leaves no side
  effect; approver without the tool's permission is refused.

### Tier 1 — operations tools

- **T1.1 Alerts tools**: `list_alert_rules`, `create_alert_rule`,
  `update_alert_rule`, `delete_alert_rule` (high), `run_alert_rule`.
- **T1.2 Connector tools**: `list_connectors`, `create_connector`,
  `test_connector` (real dial, PostgreSQL/S3 only; others return
  `supported:false` exactly like the route), `delete_connector` (high).
- **T1.3 Pipeline tools**: `list_pipelines`, `list_pipeline_runs`,
  `trigger_pipeline`, `retry_pipeline_run`, `pause_pipeline`/`resume_pipeline`/`cancel_pipeline_run` (high).
- **T1.4 Saved-query tools**: `save_query`, `list_saved_queries`,
  `run_saved_query` (through the read-only guard).
- **T1.5 Prompt + capabilities**: extend `SYSTEM_BUILD_SUFFIX` with the
  new verbs (Indonesian, same style); add frontend capability groups.

### Tier 2 — governance and operations

- **T2.1 Governance read tools**: `get_audit_history`, `list_classification_rules`,
  `list_quality_rules`, `get_cdc_health` (replication slot table),
  `get_maintenance_metrics`.
- **T2.2 Maintenance tools**: `maintenance_dry_run` (low), `maintenance_apply` (high),
  both launching `bronze_maintenance_job` with the right run config.
- **T2.3 Workload tools**: `list_workloads`, `kill_query` (high).
- **T2.4 Gold export tool**: `export_gold_mart` (low; reuse
  `gold_export::run` and require the same permission as the route),
  `get_gold_export` read-back.
- **T2.5 Governance draft tools**: `draft_policy`, `draft_classification_rule`,
  `draft_quality_rule` — create in `draft` status only; activation stays in
  the console.

### Tier 3 — digital employees

- **T3.1 Migration 0024** (employee prompt/cron/mode/permissions, reserved
  `emp-copilot`), **0025** drop seeded runs/approvals.
- **T3.2 Headless run route** `POST /api/agents/employees/{id}/run` with
  token guard and permission-scoped synthetic principal; writes `agent_run`
  steps and audit events. Tests: invariant 7; run that hits a `WriteHigh`
  tool ends `waiting_approval` and the approval executes later.
- **T3.3 Dagster job + schedule factory**, compose/env wiring, docs
  paragraph in `definitions.py` mirroring the gold-export caveat.
- **T3.4 Frontend**: employee detail fields + Run now; runs detail renders
  steps; approvals link back to the run.
- **T3.5 Docs**: ADR `0012-copilot-tool-governance.md` (registry, risk
  levels, approval flow, audit sink, headless runs), update
  `docs/FEATURE_COVERAGE.md` (agents runs/approvals become REAL), README
  "Known limitations" (remove "no agent execution runtime", "no audit
  sink"; keep "no RAG"), and `GTM/ON-PREM-SALES-PLAYBOOK.md` section 7.

## 5. Acceptance (what "done" means)

- All invariants in 3.6 have named tests and pass.
- `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings`,
  `tests/route_auth.rs` green with the new policy entries.
- Manual demo on the compose stack: in Build mode, "buat alert kalau
  kunjungan harian turun 20%" → confirm → rule exists in `/alerts`;
  "hapus connector X" → approval appears on `/agents/approvals` → approve →
  connector gone, run and audit rows visible; an employee with a cron shows
  a real run in `/agents/runs` after the schedule fires.
- Ask mode still cannot mutate anything.

## 6. How to run this with subagents (for the executing session)

- Use `superpowers:subagent-driven-development` or
  `superpowers:executing-plans`. One subagent per task above, in the order
  T0.1 → T0.2 → T0.3 → T0.4 → T0.5, then T1.x in parallel (they touch
  different `tools/*.rs` files and different capability entries; have each
  add its own registry entries in a separate `const` slice to avoid merge
  conflicts in one array), then T2.x in parallel, then T3.x sequentially.
- Give every subagent this file, the anchors in §2, and the invariants in
  §3.6. Require them to run `cargo test -p lakehouse-api` and clippy before
  reporting.
- A shared cargo target dir and the per-branch verify matrix are in the
  memory note `lakehouse-stack-workflow` (Claude memory), worth reading
  before building.
- Budget: T0 is roughly two days of agent time; T1 and T2 a day each in
  parallel; T3 two days. Expect `ai.rs` refactor (T0.1) to be the riskiest
  step; snapshot the tool schema JSON before touching it.
