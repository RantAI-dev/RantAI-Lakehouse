# ADR 0012 — Copilot tool governance: registry, risk tiers, approvals, audit

- **Status:** Accepted
- **Phase:** P7 (copilot operations expansion, `lakehouse/p7-copilot-ops`)
- **Date:** 2026-09-09

## Context

Before this work, the AI Copilot (`POST /api/ai/chat`) had ~15 tools
declared across two hand-maintained lists in `ai.rs` (a schema list and a
dispatch `match`), gated only by chat mode: Ask mode refused a fixed
`WRITE_TOOLS: [&str; 5]` array, Build mode ran every tool for any
authenticated caller. There was no per-tool permission check, no
human-approval path for destructive actions, and no audit trail specific
to the copilot — `GET /api/governance/audit` showed only Dagster/ClickHouse
pipeline history. Digital-employee `agent_run`/`approval_item` rows existed
in Postgres (migration `0017_agents.sql`) but nothing ever wrote to them
from a live path; the seed data was fixtures.

The goal (`docs/superpowers/plans/2026-09-08-copilot-operations-handover.md`)
was to let the copilot do most of what the console can do — alerts,
connectors, pipelines, saved queries, governance, ops, maintenance, Gold
export — while making every write either inline-confirmed or
human-approved, audited, and never able to exceed what the calling
principal (or, for a scheduled run, the digital employee) could already do
through `POLICY_TABLE`.

## Decision 1 — a static tool registry, not two hand-maintained lists

`ai.rs` is split into `routes/ai/{mod,registry,gate,audit,tools/*}.rs`.
`registry::TOOLS` is a single `&[ToolSpec]` — `name`, `schema: fn() -> Value`,
`risk: Risk`, `permission: &'static str` — and both `tool_schemas()` (fed to
the LLM) and dispatch (`tools::run_tool`, via `registry::find`) derive from
it, so the two can no longer drift apart. A characterization test
(`5ffd6d3`, before any behaviour changed) snapshotted the pre-refactor
`tool_schemas()` output to `tests/fixtures/tool_schemas.json`; the refactor
commit (`8b6a881`) asserts byte-identical output against that fixture.
`registry.rs` now carries **47 tools**: the original 15 plus 19 Tier 1
operations tools (alerts, connectors, pipelines, saved queries) and 13
Tier 2 tools (governance reads, one maintenance tool, workloads, Gold
export, three governance drafts) — measured directly:
`tool_schemas_has_forty_seven_entries` in `registry.rs`.

`Risk` has three variants: `Read`, `WriteLow` (inline chat confirmation),
`WriteHigh` (human-approval queue). Every tool's `permission` is the exact
`resource:action` string its console-route equivalent requires in
`policy.rs::POLICY_TABLE` (an empty string means the route is
`Policy::RequiresAuth` with nothing narrower) — verified per-tool against
`POLICY_TABLE` and pinned by `tier1_tools_have_the_documented_risk_and_permission`
and `tier2_tools_have_the_documented_risk_and_permission` in `registry.rs`.

## Decision 2 — permission is checked at the dispatch site, not just by mode

`chat()` now takes `principal: Option<Extension<Principal>>` and
`gate::decide(is_build, perms, spec, args)` runs for **every** tool call
about to execute — not only the ones offered to the model
(`registry::tool_schemas_for` additionally filters the advertised list per
principal, but that is an optimization; `decide` is the real gate, because
`parse_minimax_tool_calls` extracts `<invoke>` XML from free model text
regardless of what was advertised, and a hallucinated `tool_calls` entry
looks identical to a real one by the time it reaches dispatch).

`decide` checks, in order: (1) Ask mode + non-`Read` risk → refused
(unchanged from before); (2) a non-empty `permission` the principal's
merged `PermissionSet` lacks → refused with `reason: "permission"`, checked
for every risk tier including `Read` (`describe_mart` needs
`dashboard:read`); (3) the risk-tier branch (Decision 3). An absent
principal is treated as "authenticated, no grants" — every non-empty-
permission tool is refused, matching `POST /api/ai/chat`'s own
`RequiresAuth` policy already ruling out a genuinely anonymous caller.

### The escalation this closes

`POST /api/dashboard/specs` (`create_chart`'s console equivalent) requires
`dashboard:write`; of the seven seeded roles (`0002_seed_identity.sql`)
only Platform Admin holds it. Before this change, `run_tool` executed
`create_chart` for any authenticated Build-mode principal with **no**
permission check at all — an Analyst refused a chart in the console (403)
could get the same chart by asking the copilot instead. The copilot was an
unguarded path around `POLICY_TABLE`. `gate.rs`'s named regression test,
`analyst_cannot_create_chart_the_console_would_refuse`, pins the fix. The
module doc comment on `gate.rs` also tables the visible behaviour change
for all seven seeded roles (six of them lose copilot dashboard access they
previously had by accident); restoring it is a deliberate, separate grant
decision, not made by this change.

## Decision 3 — two-level approval: inline confirm, then a human queue

- **`WriteLow`**: a call without `"confirmed": true` in its args gets
  `{"needs_confirmation": true, "tool", "args", "summary"}` back and
  executes nothing; `summary` is generated deterministically from the
  tool's own args (never asked of the LLM), so it can't drift from what
  will actually run. `POST /api/ai/tool` runs exactly one gated tool call
  directly (bypassing another LLM round-trip) — the frontend's Confirm
  button re-sends the identical call with `confirmed: true` merged in;
  Cancel sends nothing.
- **`WriteHigh`**: always produces an internal pending marker (never
  returned to the model/frontend as-is), turned into a real `approval_item`
  (status `pending`) + linked `agent_run` (status `waiting_approval`,
  attributed to the reserved `emp-copilot` employee row) in one
  transaction — `store_agents::create_pending_approval`. The model/frontend
  see `{"needs_approval": true, "approval_id", "run_id", "summary"}`;
  nothing executes until a human decides.

`POST /api/agents/approvals/{id}/decide` executes an approved call through
the *same* `tools::run_tool` dispatch the chat loop uses (never a second,
divergent code path), after re-checking the **approver's own** permission
for the underlying tool — `agent:approve` is a narrower grant than "may
perform every action a human might approve," so an approver without
`dashboard:write` who approves a pending `delete_chart` gets the approval
recorded as `approved` but the execution refused (403-shaped), not a silent
bypass. Rejecting a `WriteHigh` call executes nothing and leaves the run
`rejected`.

### Exactly-once execution

`lakehouse_store::agents::decide_approval` opens a transaction, does
`SELECT status FROM approval_item WHERE id = $1 FOR UPDATE`, and returns
`StoreError::Conflict` if `status != "pending"` before ever updating it — a
second `decide` call on the same id (concurrent double-click, retried
request) gets a 409, never a second execution. This was the exact shape
already used by the pre-existing digital-employee approval flow
(`routes::agents::decide_approval`); the copilot's `WriteHigh` path reuses
it rather than inventing a second one.

## Decision 4 — the audit sink

Migration `0023_audit_event.sql` adds an append-only `audit_event` table
(`principal_id`, `principal_kind`, `action`, `resource_kind`/`resource_id`,
`args` JSONB, `outcome`, `run_id`, `approval_id`, `session_id`).
`lakehouse-store::audit` provides `insert`/`list`; `GET
/api/governance/audit` unions these rows into its existing response shape
with `source: "copilot"` (`7467822`).

`routes/ai/audit.rs::record` is the single write path from the copilot: it
never returns a `Result` — a failed write is logged and swallowed, so a
degraded audit sink cannot take down chat or `/api/ai/tool`. Every gate
decision and every executed tool call writes exactly one row. `redact()`
is mandatory on the way in: any object key matching `secret`/`password`/
`token`/`key` (case-insensitively, substring match) has its value replaced
with `"[redacted]"` regardless of shape, recursively through nested
objects/arrays; any other string over 500 characters is truncated with a
`…[truncated]` marker. `redact()` only ever sees a call's input `args`,
never its result — invariant 5's "never store SQL result rows" half is
enforced by construction (callers never hand a result to `redact`), not by
trying to recognise a result-shaped value.

This audit sink is scoped to the copilot's own dispatch paths
(`principal_kind` is always `"copilot"` here); it does not extend to
`routes/auth.rs` login/logout or other console mutations — that was an
optional, separable task in the handover plan and was not done in this
branch.

## Decision 5 — headless digital-employee runs, with a permission ceiling

Migration `0024_agent_schedule.sql` adds `prompt`, `schedule_cron`, `mode`,
`permissions` to `agent_employee` and reserves `emp-copilot` ("Copilot
(interactive)") for approvals created from interactive chat. Migration
`0025` drops the seeded fixture `agent_run`/`approval_item` rows, so
`/agents/runs` only ever shows real history from here on.

`POST /api/agents/employees/{id}/run` (`ff75eba`) runs the same chat/tool
loop headlessly, off the stored `prompt`/`mode`, for either an interactive
"Run now" click (`RequiresPermission("agent:manage")`, `RunAuth::Principal`)
or a scheduled Dagster trigger authenticated via the service identity
below (`RunAuth::Token`). **The run's `PermissionSet` is built from the
employee's own `permissions` column, never the triggering caller's** — an
employee with an empty `permissions` string gets a run that can execute no
permissioned tool at all, regardless of who clicked "Run now" or which
service token fired the schedule. A run that hits a `WriteHigh` tool ends
`waiting_approval`, exactly like an interactive one, and the approval
executes later through the same `decide_approval` path.

## Decision 6 — a real service identity for scheduled runs, because `RequiresAuth` is a floor

`dagster/dispar_orchestrate/agent_runs.py` registers `agent_run_job`
(`1c299db`) and a schedule factory that reads employees with
`schedule_cron` from lakehouse-api at code-load time — following the exact
precedent `gold_export_job` set (`f8bccc8`): left deliberately unscheduled
until a caller can actually authenticate.

`POST /api/agents/employees/{id}/run`'s `POLICY_TABLE` entry is
`Policy::RequiresAuth` — a **floor**, not the whole guard.
`crate::policy::auth_gate` demands a real, authenticated
`lakehouse_auth::Principal` **before**
`routes::agents::check_employee_run_auth`'s own `x-run-token` check ever
runs. A compose stack that sets `AGENT_RUN_TOKEN` for Dagster to send as
`x-run-token`, but never mints Dagster a credential to authenticate *with*,
still gets `401` at the `auth_gate` layer — the route's own token branch is
never reached. `main.rs::bootstrap_agent_run_service` (`92b7977`) closes
this: on boot, if `AGENT_RUN_TOKEN` is set, it seeds one
`lakehouse_auth::Principal` that authenticates via `Authorization: Bearer
<AGENT_RUN_TOKEN>` (routed to `ServiceTokenAuthenticator` by the bearer-shape
dispatch), scoped to **exactly** `agent:manage` — never `*:*`. The same
configured token is then also sent as `x-run-token`, satisfying both the
floor and the route's own check with one env var. Bootstrap is idempotent:
a repeat identity insert is treated as "already seeded" via
`StoreError::Conflict`, and the credential insert is `ON CONFLICT
(token_hash) DO NOTHING`. With `AGENT_RUN_TOKEN` unset, bootstrap seeds
nothing and logs a warning — schedules stay inert, the same safe posture
`gold_export_job` shipped with.

## Corrections made during implementation

### C1 — the planned `maintenance_dry_run`/`maintenance_apply` pair was not built

The plan (§3.2's risk table, §4 T2.2) assumed a copilot-visible dry-run
tool distinct from an apply tool. Two facts, measured against the code,
rule that out:

- `bronze_maintenance_job` (`dagster/dispar_orchestrate/maintenance.py`,
  around lines 297–299) always runs a dry pass **and then** the applied
  pass, in the same job, for every discovered Bronze table — there is no
  run-config flag that selects one or the other.
- `DgClient::launch_run(job_name)` (`rust/crates/lakehouse-dagster/src/lib.rs:439`)
  takes a job name only; there is no run-config parameter to thread a
  dry-run-only override through even if the Dagster job supported one.

A tool advertised to a user as "dry run" that actually launched
`bronze_maintenance_job` would have mutated Bronze (deleted orphan Iceberg
data/manifest files) on every call. Rather than build that misleading
surface, exactly **one** tool ships: `run_bronze_maintenance`, `WriteHigh`,
whose description says plainly "ini BUKAN dry run" (this is not a dry
run). A genuine dry-run-only mode needs a Dagster job-config change plus
`launch_run` run-config support, and is out of scope for this branch.

### C2 — `get_maintenance_metrics`'s description named the wrong verb

The tool's description originally said the maintenance history was about
`expire_snapshots`. Measured against the current stack (ClickHouse 26.8,
per `docs/plans/CLICKHOUSE-26.8-REMEASUREMENT.md`, predating this branch):
`expire_snapshots` is rejected with `Code: 48 ... not supported for
Iceberg tables backed by a transactional catalog`, and
`bronze_maintenance_job` records that rejection only as a logged skip, not
a result. `remove_orphan_files` is the verb `maintenance.py` actually runs
(dry, then applied) and the only one that produces real deleted-file
counts. `get_maintenance_metrics`'s description was corrected to say so,
and to name the `expire_snapshots` rejection explicitly rather than let a
reader assume the tool surfaces snapshot-expiry history.

### C3 — `Policy::RequiresAuth` is a floor, not the whole guard

The plan's anchors (§2) described the run-token pattern
(`check_export_token`/`check_run_token`) as if the header check were the
entire authentication story, mirroring how `gold_export_job` and
`alerts_run` already worked. Building the headless run route the same way
surfaced that an `x-run-token` header alone cannot authenticate a request
at all: `auth_gate` runs first and rejects an unauthenticated request
before the route's own token-matching code is ever reached. This is why
Decision 6's service identity exists — it is not optional polish, it is
the only way a `x-run-token`-bearing Dagster schedule can reach the route's
own check in the first place. See Decision 6.

## Consequences

- The copilot can now do most of what `docs/superpowers/plans/2026-09-08-copilot-operations-handover.md`
  set out to enable (alerts, connectors, pipelines, saved queries,
  governance reads and draft authoring, workloads, Gold export, Bronze
  maintenance), gated uniformly by the same registry, gate, and audit
  path — not a per-tool bespoke check.
- Every write the copilot performs is either inline-confirmed by the same
  human who is chatting, or queued for a **different** human (or the same
  one, with their own permission re-checked) to approve — there is no
  write path left that skips both.
- Scheduled digital-employee runs are real, but bounded strictly by the
  employee's own `permissions` column — a compromised or misconfigured
  schedule cannot do more than its employee row was granted, no matter
  what token fired it.
- Document retrieval / RAG remains explicitly out of scope (no vector
  store exists in this codebase); the copilot is a tool-calling agent over
  the 47 registered tools, not a general autonomous worker and not a
  document-grounded assistant.
- `cargo test -p lakehouse-api --lib` is 253 tests passing, including the
  new suites this ADR describes (`security_regressions` 10,
  `agents_approval` 5, `ai_tool` 8, `tier1_writehigh_approval` 4,
  `tier1_saved_query_guard` 3, `tier2_writehigh_approval` 2,
  `tier2_governance_drafts` 3, `route_auth` 7).
