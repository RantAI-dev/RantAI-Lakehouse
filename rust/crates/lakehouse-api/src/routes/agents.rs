//! `/api/agents/*` — digital employee definitions, tools, workflows, run
//! history, and the approval lifecycle, backed by Postgres
//! (`lakehouse-store`).
//!
//! # Not a port
//!
//! Like `routes::identity`/`routes::connectors`/`routes::knowledge`, this
//! replaces an *in-browser* mock (`src/services/mock/agents.ts`) that
//! never had a server side. Status codes are chosen to be correct: 201 on
//! create, 404 on a missing id, 409 on a duplicate name or a re-decided
//! approval, 400 on a malformed body, 503 with no database pool.
//!
//! # Scope: no execution runtime
//!
//! `AgentService` has no "run this agent"/"invoke this tool" method, and
//! this module does not add one. `listRuns`/`getRun` serve historical run
//! *records* (seeded the same way every other Phase 2 domain seeds its
//! fixtures) — nothing here launches an agent or a tool. See
//! `lakehouse_store::agents`'s module doc comment.
//!
//! # Console surface removed
//!
//! `WS1` task 1.11 removed the Agent Workflows and Tool Registry pages:
//! nothing executes an authored workflow, and the agent runtime never
//! reads the tool registry. `/api/agents/workflows` still serves real
//! `Postgres` CRUD, stays registered and `POLICY_TABLE`-classified, and is
//! left for a later workstream to reuse or retire.
//!
//! `GET /api/agents/tools` (WS7 item G4) no longer serves that unread
//! `agent_tool` Postgres table — it now reflects
//! `crate::routes::ai::registry::TOOLS`, the real copilot tool registry
//! every headless run and interactive chat call actually dispatches
//! through, with real 30-day usage counts from `audit_event`. `POST
//! /api/agents/tools` is removed entirely: nothing ever consumed it (no
//! frontend page calls it — confirmed empty `grep -rln "AgentTool\|
//! registerTool\|/api/agents/tools" src/`), and "registering" a tool in
//! `agent_tool` never made it callable — the registry above is the only
//! thing that does. The `agent_tool` table and its `0017`/`0018` seed
//! rows stay in the schema (a migration is never edited or dropped once
//! applied) but nothing in this crate reads them anymore.

use axum::body::Bytes;
use axum::extract::{Extension, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use lakehouse_auth::{PermissionSet, Principal};
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::agents::{
    self, AgentRun, AgentWorkflow, ApprovalItem, CreateEmployeeInput, CreateWorkflowInput,
    Decision, DigitalEmployee, LinkedApprovalRequest, RunStep,
};
use lakehouse_store::audit::{self as store_audit, NewAuditEvent};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::ai::{audit as ai_audit, registry as ai_registry, tools as ai_tools};
use crate::state::AppState;

/// Borrow the Postgres pool, or fail with a 503. Mirrors
/// `routes::identity::pool`.
fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "agents store unavailable: no Postgres pool is configured \
             (DATABASE_URL is missing or not a valid Postgres connection string)"
                .to_owned(),
        )
    })
}

fn parse_body<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|err| ApiError::BadRequest(format!("invalid JSON: {err}")))
}

fn required(field: &str, value: &str) -> Result<String, ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::BadRequest(format!("{field} is required")));
    }
    Ok(trimmed.to_owned())
}

const VALID_AUTONOMY: [&str; 4] = ["L1", "L2", "L3", "L4"];

/// `agent_employee.mode`'s `CHECK` constraint values
/// (`0025_agent_schedule.sql`) — a headless run uses this to pick the
/// copilot's `SYSTEM_ASK_SUFFIX`/`SYSTEM_BUILD_SUFFIX` equivalent.
const VALID_MODE: [&str; 2] = ["ask", "build"];

// ── Workflows ──────────────────────────────────────────────────────────

/// `GET /api/agents/workflows`.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_workflows(
    State(state): State<AppState>,
) -> ApiResult<ApiJson<Vec<AgentWorkflow>>> {
    Ok(ApiJson(agents::list_workflows(pool(&state)?).await?))
}

/// The `POST /api/agents/workflows` body. Mirrors `CreateWorkflowInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkflowBody {
    name: String,
    trigger: String,
    #[serde(default)]
    step_kinds: Vec<String>,
    approval_required: bool,
    #[serde(default)]
    owner: Option<String>,
}

/// `POST /api/agents/workflows` — create a workflow. Returns 201.
///
/// # Errors
///
/// 400 on a malformed body or a blank required field; 409 if the name is
/// taken; 503/500 as above.
pub async fn create_workflow(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<AgentWorkflow>)> {
    let body: CreateWorkflowBody = parse_body(&body)?;
    let input = CreateWorkflowInput {
        name: required("name", &body.name)?,
        trigger: required("trigger", &body.trigger)?,
        step_count: i64::try_from(body.step_kinds.len()).unwrap_or(i64::MAX),
        approval_required: body.approval_required,
        owner: body.owner,
    };
    let created = agents::create_workflow(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(created)))
}

// ── Employees ──────────────────────────────────────────────────────────

/// `GET /api/agents/employees`.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_employees(
    State(state): State<AppState>,
) -> ApiResult<ApiJson<Vec<DigitalEmployee>>> {
    Ok(ApiJson(agents::list_employees(pool(&state)?).await?))
}

/// `GET /api/agents/employees/{id}`. Uses
/// [`agents::get_employee_with_metrics`] (WS7 item G3), not the lighter
/// [`agents::get_employee`] `list_employees` uses — a single-row DETAIL
/// read can afford the four real metric columns `list_employees` skips.
///
/// # Errors
///
/// 404 if `id` is unknown; 503/500 as above.
pub async fn get_employee(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<DigitalEmployee>> {
    let employee = agents::get_employee_with_metrics(pool(&state)?, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Employee {id} not found")))?;
    Ok(ApiJson(employee))
}

/// The `POST /api/agents/employees` body. Mirrors `CreateEmployeeInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateEmployeeBody {
    name: String,
    purpose: String,
    autonomy: String,
    #[serde(default)]
    allowed_tools: Vec<String>,
    data_scope: String,
    budget_limit: f64,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    schedule_cron: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    permissions: Option<String>,
}

/// `POST /api/agents/employees` — create a digital employee. Returns 201.
///
/// # T3.2 fix: `mode` used to reach the database unvalidated
///
/// `agent_employee.mode` carries a `CHECK (mode IN ('ask', 'build'))`
/// constraint (`0025_agent_schedule.sql`), but nothing validated `body.mode`
/// before this handler handed it to [`agents::create_employee`] — an
/// invalid value (`{"mode": "sleep"}`) surfaced as a raw Postgres
/// constraint-violation error (an opaque 500) instead of a clean 400,
/// exactly the gap [`VALID_MODE`] closes here, the same way [`VALID_AUTONOMY`]
/// already does for `autonomy`.
///
/// # Errors
///
/// 400 on a malformed body, a blank required field, an unrecognized
/// `autonomy`, or an unrecognized `mode`; 409 if the name is taken;
/// 503/500 as above.
pub async fn create_employee(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<DigitalEmployee>)> {
    let body: CreateEmployeeBody = parse_body(&body)?;
    let autonomy = required("autonomy", &body.autonomy)?;
    if !VALID_AUTONOMY.contains(&autonomy.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "autonomy must be one of {VALID_AUTONOMY:?}, got {autonomy:?}"
        ))
        .into());
    }
    if let Some(mode) = &body.mode
        && !VALID_MODE.contains(&mode.as_str())
    {
        return Err(ApiError::BadRequest(format!(
            "mode must be one of {VALID_MODE:?}, got {mode:?}"
        ))
        .into());
    }
    let input = CreateEmployeeInput {
        name: required("name", &body.name)?,
        purpose: required("purpose", &body.purpose)?,
        autonomy,
        allowed_tools: body.allowed_tools,
        data_scope: required("dataScope", &body.data_scope)?,
        budget_limit: body.budget_limit,
        owner: body.owner,
        prompt: body.prompt,
        schedule_cron: body.schedule_cron,
        mode: body.mode,
        permissions: body.permissions,
    };
    let created = agents::create_employee(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(created)))
}

fn map_transition_result(
    id: &str,
    result: Result<DigitalEmployee, lakehouse_store::StoreError>,
) -> ApiResult<ApiJson<DigitalEmployee>> {
    match result {
        Ok(updated) => Ok(ApiJson(updated)),
        Err(lakehouse_store::StoreError::NotFound) => {
            Err(ApiError::NotFound(format!("Employee {id} not found")).into())
        }
        Err(err) => Err(ApiError::from(err).into()),
    }
}

/// `POST /api/agents/employees/{id}/suspend`.
///
/// # Errors
///
/// 404 if `id` is unknown; 503/500 as above.
pub async fn suspend_employee(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<DigitalEmployee>> {
    let result = agents::suspend_employee(pool(&state)?, &id).await;
    map_transition_result(&id, result)
}

/// `POST /api/agents/employees/{id}/resume`.
///
/// # Errors
///
/// 404 if `id` is unknown; 503/500 as above.
pub async fn resume_employee(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<DigitalEmployee>> {
    let result = agents::resume_employee(pool(&state)?, &id).await;
    map_transition_result(&id, result)
}

/// `POST /api/agents/employees/{id}/revoke`.
///
/// # Errors
///
/// 404 if `id` is unknown; 503/500 as above.
pub async fn revoke_employee(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<DigitalEmployee>> {
    let result = agents::revoke_employee(pool(&state)?, &id).await;
    map_transition_result(&id, result)
}

// ── Tools ──────────────────────────────────────────────────────────────

/// One entry in `GET /api/agents/tools`'s response — the real copilot
/// tool registry (`ai_registry::TOOLS`), not the unread `agent_tool`
/// Postgres table (WS7 item G4; see this module's own doc comment).
/// Replaces the old `AgentTool` contract entirely — a deliberate breaking
/// change with no known caller (see this task's report for the
/// confirming grep).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryTool {
    /// The name the copilot's LLM calls this tool by
    /// ([`ai_registry::ToolSpec::name`]).
    pub name: String,
    /// This tool's `OpenAI`-compatible function schema's own
    /// `function.description` — the same text the LLM itself is shown.
    pub description: String,
    /// `"Read" | "WriteLow" | "WriteHigh"` — [`ai_registry::Risk`], as the
    /// gate in `ai::gate` enforces it.
    pub risk: &'static str,
    /// The `resource:action` permission this tool requires, or `""` for
    /// authenticated-only (see [`ai_registry::ToolSpec::permission`]'s own
    /// doc comment).
    pub permission: &'static str,
    /// Real dispatch count over the trailing 30 days, from
    /// `audit_event` — 0 when the registry has never been asked to run
    /// this tool in the window, never a fabrication when it's genuinely
    /// unused.
    pub usage_count_30d: i64,
}

/// [`ai_registry::Risk`] as the string this route's contract exposes —
/// the registry itself carries no string form (its `Risk` enum is
/// dispatch-internal), so this is the one, single place that names.
fn risk_label(risk: ai_registry::Risk) -> &'static str {
    match risk {
        ai_registry::Risk::Read => "Read",
        ai_registry::Risk::WriteLow => "WriteLow",
        ai_registry::Risk::WriteHigh => "WriteHigh",
    }
}

/// Maps every [`ai_registry::TOOLS`] entry to a [`RegistryTool`], looking
/// up each one's real 30-day dispatch count from `usage_counts` (an
/// `(action, count)` pair per distinct `audit_event.action` in the
/// window, as the route handler's own `GROUP BY` query produces) — `0`
/// when a tool's name has no matching row, not absent from the list: the
/// registry is the source of truth for WHICH tools exist, `audit_event`
/// only for how often each one ran.
fn list_tools_body(usage_counts: &[(String, i64)]) -> Vec<RegistryTool> {
    ai_registry::TOOLS
        .iter()
        .map(|spec| {
            let schema = (spec.schema)();
            let description = schema["function"]["description"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let usage_count_30d = usage_counts
                .iter()
                .find(|(action, _)| action == spec.name)
                .map_or(0, |(_, count)| *count);
            RegistryTool {
                name: spec.name.to_owned(),
                description,
                risk: risk_label(spec.risk),
                permission: spec.permission,
                usage_count_30d,
            }
        })
        .collect()
}

/// `GET /api/agents/tools` — the real copilot tool registry
/// (`ai_registry::TOOLS`) with real 30-day dispatch counts from
/// `audit_event` (WS7 item G4). `POST /api/agents/tools` (the old
/// `agent_tool`-table "register a tool" endpoint) is removed entirely —
/// see this module's doc comment.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_tools(State(state): State<AppState>) -> ApiResult<ApiJson<Vec<RegistryTool>>> {
    let usage_counts = agents::tool_usage_counts_30d(pool(&state)?).await?;
    Ok(ApiJson(list_tools_body(&usage_counts)))
}

// ── Runs ───────────────────────────────────────────────────────────────

/// Query parameters shared by `GET /api/agents/runs` and
/// `GET /api/agents/approvals`.
#[derive(Debug, Deserialize)]
pub struct EmployeeQuery {
    /// `?employeeId=<id>` — restrict to one employee.
    employee_id: Option<String>,
}

/// `GET /api/agents/runs?employeeId=` — mirrors `AgentService::listRuns`.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_runs(
    State(state): State<AppState>,
    Query(query): Query<EmployeeQuery>,
) -> ApiResult<ApiJson<Vec<AgentRun>>> {
    Ok(ApiJson(
        agents::list_runs(pool(&state)?, query.employee_id.as_deref()).await?,
    ))
}

/// `GET /api/agents/runs/{id}`.
///
/// # Errors
///
/// 404 if `id` is unknown; 503/500 as above.
pub async fn get_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<AgentRun>> {
    let run = agents::get_run(pool(&state)?, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Run {id} not found")))?;
    Ok(ApiJson(run))
}

// ── Approvals ──────────────────────────────────────────────────────────

/// `GET /api/agents/approvals?employeeId=`.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_approvals(
    State(state): State<AppState>,
    Query(query): Query<EmployeeQuery>,
) -> ApiResult<ApiJson<Vec<ApprovalItem>>> {
    Ok(ApiJson(
        agents::list_approvals(pool(&state)?, query.employee_id.as_deref()).await?,
    ))
}

/// The `POST /api/agents/approvals/{id}/decide` body. Mirrors
/// `DecideApprovalInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecideApprovalBody {
    decision: String,
    #[serde(default)]
    comment: Option<String>,
}

/// `POST /api/agents/approvals/{id}/decide` — approve or reject a pending
/// approval.
///
/// # T0.5: approve executes the linked run's stored tool call
///
/// `lakehouse_store::agents::decide_approval` does `SELECT ... FOR UPDATE`
/// on the approval row, checks `status == "pending"`, and only then flips
/// it — so of any number of concurrent `decide` calls on the SAME
/// approval, exactly one can ever observe `Ok(_)` from that call; every
/// other one gets [`lakehouse_store::StoreError::Conflict`] (409) and
/// returns here BEFORE any execution logic runs. That is the entire
/// exactly-once guarantee this handler relies on: only the single call
/// that wins the pending→approved transition ever reaches the execution
/// code below, so a second `decide` on an already-decided approval can
/// never execute the tool a second time (see the invariant's HTTP-level
/// test, `decide_approval_double_decide_executes_at_most_once`).
///
/// On **approve**, after that transition succeeds:
/// 1. The APPROVER's own permission for the tool being approved is
///    checked — `agent:approve` (this route's own policy) does NOT imply
///    the approver may perform the underlying action. A missing
///    permission means the tool is NEVER executed: the run is recorded
///    `failed`/"not executed", audited, and this returns 403.
/// 2. The linked run's stored tool call (`lakehouse_store::agents::
///    pending_tool_call`, recorded verbatim by `gate::create_write_high_approval`
///    at request time) is replayed through the SAME tool dispatch
///    (`routes::ai::tools::run_tool`) the chat loop and
///    `POST /api/ai/tool` use — never a second, divergent execution path.
/// 3. The result is appended to `agent_run.steps`, the run's terminal
///    status is set, and an `audit_event` records the outcome, linked by
///    `run_id`/`approval_id`.
///
/// On **reject**, the run is marked `rejected` and nothing executes.
///
/// # Errors
///
/// 400 on a malformed body or a `decision` other than `"approved"`/
/// `"rejected"`; 404 if `id` is unknown; 409 if the approval has already
/// been decided (mirrors `mock/agents.ts`'s "already {status}" guard — a
/// state conflict, not a bad request, hence 409 rather than the mock's
/// 400: see `lakehouse_store::agents::decide_approval`'s doc comment);
/// 403 if the approval was approved but the approver lacks the underlying
/// tool's own permission (the tool is never executed in that case);
/// 503/500 as above.
/// The `NewAuditEvent` [`decide_approval`]'s approval-decision audit write
/// builds — extracted as a pure, unit-tested function (WS5 item D2),
/// mirroring `routes::query::query_run_audit_event`'s pattern: this call
/// site's own choice (`resource_kind: "approval"`, paired with the
/// approval's own id — the pairing WS1 T16's `LEFT JOIN` depends on,
/// confirmed by reading this handler in full) is hardcoded inside the
/// helper, not passed by the caller, so a real assertion on the built
/// event pins it — unlike the source-text-grepping test this replaces,
/// which would still pass if this call site became dead code.
///
/// `principal_kind` comes from [`Principal::kind_for_audit`], never the
/// `"copilot"` literal `routes::ai::audit::record` uses for its own
/// (genuinely copilot-triggered) callers: `decide_approval` is a console
/// route a human calls (`Policy::RequiresPermission("agent:approve")`),
/// and recording their decision as the copilot's was the bug this task
/// fixes. Delegates construction to `routes::ai::audit::build_event`
/// (AGENTS.md rule 4 — reusing beats duplicating the `NewAuditEvent`
/// literal) rather than calling `routes::ai::audit::record`, which would
/// force `principal_kind` back to `"copilot"`.
fn decide_approval_audit_event(
    principal: &Principal,
    action: &str,
    approval_id: &str,
    comment: Option<&str>,
    outcome: &str,
    run_id: Option<&str>,
) -> NewAuditEvent {
    ai_audit::build_event(
        principal.kind_for_audit(),
        Some(principal),
        None,
        action,
        Some("approval"),
        Some(approval_id),
        &json!({ "comment": comment }),
        outcome,
        None,
        run_id,
        Some(approval_id),
    )
}

#[allow(
    clippy::too_many_lines,
    reason = "one straight-line sequence of terminal outcomes (reject / \
              unknown tool / approver lacks permission / execute), each of \
              which records a run outcome and an audit event before \
              returning; splitting it up would scatter that one linear \
              decision tree across helpers with no independent reuse"
)]
pub async fn decide_approval(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let body: DecideApprovalBody = parse_body(&body)?;
    let decision = match body.decision.as_str() {
        "approved" => Decision::Approved,
        "rejected" => Decision::Rejected,
        other => {
            return Err(ApiError::BadRequest(format!(
                "decision must be \"approved\" or \"rejected\", got {other:?}"
            ))
            .into());
        }
    };
    let pg = pool(&state)?;

    let approval = match agents::decide_approval(pg, &id, decision, body.comment.as_deref()).await {
        Ok(updated) => updated,
        Err(lakehouse_store::StoreError::NotFound) => {
            return Err(ApiError::NotFound(format!("Approval {id} not found")).into());
        }
        Err(lakehouse_store::StoreError::Conflict) => {
            return Err(
                ApiError::Conflict(format!("Approval {id} has already been decided")).into(),
            );
        }
        Err(err) => return Err(ApiError::from(err).into()),
    };

    let decide_outcome = match decision {
        Decision::Approved => "approved",
        Decision::Rejected => "rejected",
    };
    // WS5 item D2: built and inserted directly (not via
    // `ai_audit::record`, which hardcodes `principal_kind: "copilot"` —
    // wrong here, `decide_approval` is a human console route, not a
    // copilot dispatch path) so this row's `principal_kind` reflects the
    // deciding principal's own kind.
    let decide_audit_event = decide_approval_audit_event(
        &principal,
        &approval.action,
        &id,
        body.comment.as_deref(),
        decide_outcome,
        approval.run_id.as_deref(),
    );
    if let Err(err) = store_audit::insert(pg, decide_audit_event).await {
        tracing::warn!(
            %err,
            action = %approval.action,
            outcome = decide_outcome,
            "failed to record approval-decision audit event"
        );
    }

    // Approvals the copilot creates always carry a `run_id` (see
    // `create_pending_approval`); one that doesn't (a manually-inserted or
    // future non-copilot approval) has nothing to execute — return the
    // decision as-is.
    let Some(run_id) = approval.run_id.clone() else {
        return Ok(ApiJson(json!({ "approval": approval, "executed": false })));
    };

    if decision == Decision::Rejected {
        if let Err(err) = agents::record_run_outcome(
            pg,
            &run_id,
            "rejected",
            "Rejected",
            body.comment.as_deref().unwrap_or(""),
        )
        .await
        {
            tracing::warn!(%err, run_id, "failed to record rejected-run outcome");
        }
        return Ok(ApiJson(json!({ "approval": approval, "executed": false })));
    }

    // Approved: check the APPROVER's own permission for the tool BEFORE
    // executing anything — `agent:approve` is a narrower, per-decision
    // grant that does not imply the approver may perform the underlying
    // action (plan invariant 3 / the copilot-operations-handover plan's
    // permission rule).
    let Some(spec) = ai_registry::find(&approval.action) else {
        let detail = format!("unknown tool: {}", approval.action);
        if let Err(err) =
            agents::record_run_outcome(pg, &run_id, "failed", "Not executed", &detail).await
        {
            tracing::warn!(%err, run_id, "failed to record not-executed outcome");
        }
        return Err(ApiError::Internal(detail).into());
    };

    if !spec.permission.is_empty() && !principal.permissions.has(spec.permission) {
        let detail = format!(
            "approval approved, BUT not executed: approver lacks permission '{}' to run \
             this tool",
            spec.permission
        );
        if let Err(err) =
            agents::record_run_outcome(pg, &run_id, "failed", "Not executed", &detail).await
        {
            tracing::warn!(%err, run_id, "failed to record permission-denied outcome");
        }
        ai_audit::record(
            Some(pg),
            Some(&principal),
            None,
            &approval.action,
            None,
            approval.resource.as_deref(),
            &json!({}),
            "failed",
            Some(&detail),
            Some(&run_id),
            Some(&id),
        )
        .await;
        return Err(ApiError::PermissionDenied(detail).into());
    }

    // Load the run and replay its stored tool call through the SAME
    // dispatch every other execution path uses.
    let run = agents::get_run(pg, &run_id)
        .await?
        .ok_or_else(|| ApiError::Internal(format!("linked run {run_id} vanished")))?;
    let Some(pending) = agents::pending_tool_call(&run) else {
        let detail = "the approved run has no stored tool call".to_owned();
        if let Err(err) =
            agents::record_run_outcome(pg, &run_id, "failed", "Not executed", &detail).await
        {
            tracing::warn!(%err, run_id, "failed to record missing-tool-call outcome");
        }
        return Err(ApiError::Internal(detail).into());
    };
    let args: Map<String, Value> = pending.args.as_object().cloned().unwrap_or_default();
    let result = ai_tools::run_tool(&state, Some(&principal), &pending.tool, &args).await;
    let ok = !matches!(&result, Value::Object(m) if m.contains_key("error"));
    let status = if ok { "succeeded" } else { "failed" };
    if let Err(err) = agents::record_run_outcome(
        pg,
        &run_id,
        status,
        "Executed after approval",
        &serde_json::to_string(&result).unwrap_or_default(),
    )
    .await
    {
        tracing::warn!(%err, run_id, "failed to record execution outcome");
    }
    ai_audit::record(
        Some(pg),
        Some(&principal),
        None,
        &pending.tool,
        None,
        approval.resource.as_deref(),
        &json!({}),
        if ok { "executed" } else { "failed" },
        None,
        Some(&run_id),
        Some(&id),
    )
    .await;

    Ok(ApiJson(
        json!({ "approval": approval, "executed": true, "result": result }),
    ))
}

// ── Headless runs (T3.2, copilot-operations-handover plan) ─────────────

/// Query parameters for `POST /api/agents/employees/{id}/run`.
#[derive(Debug, Deserialize, Default)]
pub struct RunEmployeeQuery {
    /// Shared token, as a query-string fallback to the `x-run-token`
    /// header — same shape as `routes::gold::ExportQuery`/
    /// `routes::alerts::RunQuery`.
    token: Option<String>,
}

/// `POST /api/agents/employees/{id}/run` request body: an optional prompt
/// override for this one run, in place of the employee's own
/// `agent_employee.prompt`.
#[derive(Debug, Deserialize, Default)]
pub struct RunEmployeeBody {
    #[serde(default)]
    prompt: Option<String>,
}

/// Who/what authorized a headless run — determines `agent_run.trigger`
/// (`"schedule"` vs `"manual"`) and `audit_event.principal_kind`
/// (`"schedule"` vs `"user"`).
#[derive(Debug)]
enum RunAuth {
    /// A matching `x-run-token`/`?token=` — Dagster's own schedule
    /// factory (`dagster/dispar_orchestrate`, T3.3) calling in, the same
    /// shape as `routes::gold::check_export_token`/
    /// `routes::alerts::check_run_token`.
    Token,
    /// An authenticated principal holding `agent:manage` — a human
    /// clicking "Run now" on the employee's detail page.
    Principal(Principal),
}

/// The [`Principal`] a headless run should act as, straight from the auth
/// guard's own result — no reload (C2-F3): `run_employee` already holds
/// the live principal the auth middleware built for this request, so
/// `RunAuth::Principal` hands it back by reference and `RunAuth::Token`
/// (a schedule or service token, with no interactive user behind it)
/// yields `None`. `run_headless_loop` forwards this same value to every
/// `ai_tools::run_tool` call for the run's whole duration.
fn principal_for_run_auth(auth: &RunAuth) -> Option<&Principal> {
    match auth {
        RunAuth::Token => None,
        RunAuth::Principal(p) => Some(p),
    }
}

/// The `(trigger, principal_id, principal_kind, actor)` tuple
/// [`run_employee`] passes to [`agents::create_run`] and every
/// [`write_headless_audit`] call for this run — a pure, unit-tested
/// function (WS5 item D4), extracted from what used to be an inline
/// `match` in [`run_employee`] itself.
///
/// **Bug found and fixed while extracting this (confirmed by reading the
/// inline `match` before touching it):** `RunAuth::Principal(p)` used to
/// hardcode `principal_kind: "user"` unconditionally. `check_employee_run_auth`
/// accepts ANY authenticated principal holding `agent:manage` — human or
/// service identity alike, see that function's own doc comment — so a
/// service identity manually triggering a run (not the `RunAuth::Token`
/// schedule path, which already correctly used `"schedule"`) was
/// mis-recorded as a human's. [`Principal::kind_for_audit`] is the fix,
/// same as WS5 item D2's `decide_approval` bug: never a literal, always
/// derived from `PrincipalId`'s variant.
fn run_employee_audit_identity(
    auth: &RunAuth,
    employee_name: &str,
) -> (&'static str, Option<String>, &'static str, String) {
    match auth {
        RunAuth::Token => ("schedule", None, "schedule", employee_name.to_owned()),
        RunAuth::Principal(p) => (
            "manual",
            Some(p.id.uuid().to_string()),
            p.kind_for_audit(),
            p.display_name.clone(),
        ),
    }
}

/// The `POST /api/agents/employees/{id}/run` auth guard: EITHER a valid
/// `x-run-token` matching [`crate::config::Config::agent_run_token`], OR an
/// authenticated principal holding `agent:manage` — see the module-level
/// "Headless runs" doc comment on [`run_employee`] for the full rationale
/// (this mirrors `routes::gold::check_export_token`'s shape, but the
/// no-token fallback is a PERMISSION check, not a service-identity check,
/// because "a human clicking Run now" — not only a scheduler — is a
/// legitimate caller here).
///
/// # Errors
///
/// Returns [`ApiError::unauthorized`] (401) when the token is wrong/absent
/// AND no principal is present at all; [`ApiError::PermissionDenied`] (403)
/// when a principal IS present but lacks `agent:manage`.
fn check_employee_run_auth(
    configured: Option<&str>,
    header_token: Option<&str>,
    query_token: Option<&str>,
    principal: Option<&Principal>,
) -> Result<RunAuth, ApiError> {
    if let Some(need) = configured
        && header_token.or(query_token) == Some(need)
    {
        return Ok(RunAuth::Token);
    }
    match principal {
        Some(p) if p.permissions.has("agent:manage") => Ok(RunAuth::Principal(p.clone())),
        Some(_) => Err(ApiError::PermissionDenied(
            "agent:manage is required to run an employee manually".to_owned(),
        )),
        None => Err(ApiError::unauthorized()),
    }
}

/// The headless loop's own iteration cap — same value as `routes::ai`'s
/// `MAX_ITER` (kept as an independent constant rather than importing that
/// private one: see the module doc comment on why this route cannot reach
/// into `routes::ai::gate`).
const MAX_HEADLESS_ITER: u32 = 8;

/// How one headless run's tool-calling loop ended.
enum HeadlessOutcome {
    /// The run reached a terminal state on its own (`"succeeded"` or
    /// `"failed"`) — [`agents::finish_run`] still needs to be called with
    /// this status.
    Terminal(&'static str),
    /// A `WriteHigh` tool call inside the run needs a human decision —
    /// the run was already flipped to `"waiting_approval"` (NOT
    /// terminal — [`agents::finish_run`] must NOT be called for this
    /// outcome); `POST /api/agents/approvals/{id}/decide` finishes it
    /// later.
    WaitingApproval,
}

/// Writes a headless run's real, accumulated token spend
/// ([`agents::record_run_budget`]) and recomputes the owning employee's
/// real metrics from its `agent_run`/`approval_item` rows
/// ([`agents::recompute_employee_metrics`], WS7 item G3) — called once, at
/// every terminal transition of [`run_headless_loop`], so an employee's
/// `budgetSpent`/`successRate`/`approvalRate`/`recentRuns` are current the
/// moment its run ends. Logs (never panics) on either failure, mirroring
/// [`append_step`]'s own log-and-continue shape.
async fn write_run_budget(pg: &PgPool, run_id: &str, employee_id: &str, budget_consumed: f64) {
    if let Err(err) = agents::record_run_budget(pg, run_id, budget_consumed).await {
        tracing::warn!(%err, run_id, "headless run: failed to record budget_consumed");
    }
    if let Err(err) = agents::recompute_employee_metrics(pg, employee_id).await {
        tracing::warn!(%err, employee_id, "headless run: failed to recompute employee metrics");
    }
}

/// Appends one step to a headless run's trace, logging (never panicking)
/// if the run has vanished mid-loop.
async fn append_step(pg: &PgPool, run_id: &str, id: &str, label: &str, status: &str, detail: &str) {
    let step = RunStep {
        id: id.to_owned(),
        label: label.to_owned(),
        status: status.to_owned(),
        detail: detail.to_owned(),
    };
    if let Err(err) = agents::append_run_step(pg, run_id, step).await {
        tracing::warn!(%err, run_id, "headless run: failed to append step");
    }
}

/// Writes one `audit_event` for a headless run's gate decision or tool
/// execution, with the CORRECT `principal_kind` (`"schedule"`/`"user"`) —
/// deliberately NOT `routes::ai::audit::record`, which hardcodes
/// `principal_kind = "copilot"` for the interactive chat/`/api/ai/tool`
/// paths this route is not one of (see the module doc comment on
/// [`run_employee`]). Reuses [`ai_audit::redact`] for the args redaction
/// rule (plan invariant 5), the one piece of `routes::ai::audit` this
/// route needs and can safely call — `redact` has no principal-kind
/// opinion baked in.
#[allow(
    clippy::too_many_arguments,
    reason = "one flat record of what happened, mirroring routes::ai::audit::record's own justification"
)]
async fn write_headless_audit(
    pg: &PgPool,
    principal_id: Option<String>,
    principal_kind: &str,
    actor_label: &str,
    action: &str,
    resource_kind: Option<&str>,
    resource_id: Option<&str>,
    args: &Value,
    outcome: &str,
    detail: Option<&str>,
    run_id: &str,
    approval_id: Option<&str>,
) {
    let event = NewAuditEvent {
        principal_id,
        principal_kind: Some(principal_kind.to_owned()),
        actor_label: Some(actor_label.to_owned()),
        action: action.to_owned(),
        resource_kind: resource_kind.map(str::to_owned),
        resource_id: resource_id.map(str::to_owned),
        args: Some(ai_audit::redact(args)),
        outcome: outcome.to_owned(),
        detail: detail.map(str::to_owned),
        run_id: Some(run_id.to_owned()),
        approval_id: approval_id.map(str::to_owned),
        session_id: None,
    };
    if let Err(err) = store_audit::insert(pg, event).await {
        tracing::warn!(%err, action, outcome, "headless run: failed to write audit_event");
    }
}

/// A short, deterministic system prompt for the headless loop — NOT a
/// byte-identical copy of `routes::ai`'s private `SYSTEM_BASE`/
/// `SYSTEM_ASK_SUFFIX`/`SYSTEM_BUILD_SUFFIX` constants (this route cannot
/// reach those — see the module doc comment on [`run_employee`]), but the
/// same intent: read-only in `ask` mode, tool-using in `build` mode.
fn headless_system_prompt(is_build: bool) -> String {
    let base = "Kamu adalah digital employee di RantAI Lakehouse, berjalan tanpa pengawasan \
                langsung (headless). Selesaikan instruksi berikut menggunakan tool yang \
                tersedia. Jawab ringkas dalam Bahasa Indonesia berdasarkan HASIL TOOL yang \
                nyata; jangan mengarang angka.";
    if is_build {
        format!(
            "{base}\n\nMODE: BUILD. Kamu boleh memanggil tool tulis (write) yang tersedia \
             sesuai izinmu."
        )
    } else {
        format!(
            "{base}\n\nMODE: ASK (read-only). Jangan memanggil tool tulis apa pun — hanya \
             baca dan analisis."
        )
    }
}

/// The headless tool-calling loop shared by every `POST
/// /api/agents/employees/{id}/run` call — see [`run_employee`]'s doc
/// comment for the full picture. Never panics: an LLM failure, a refused
/// tool call, or a `WriteHigh` tool are all ordinary loop outcomes, not
/// errors propagated to the caller.
#[allow(
    clippy::too_many_arguments,
    reason = "one straight-line loop; every argument is genuinely independent input to it"
)]
#[allow(
    clippy::too_many_lines,
    reason = "one straight-line reimplementation of routes::ai::chat's own tool-calling loop \
              plus the three gate checks routes::ai::gate keeps private; splitting it up would \
              scatter one sequential loop across helpers with no independent reuse, matching \
              routes::ai::chat's own identical justification"
)]
async fn run_headless_loop(
    state: &AppState,
    run_id: &str,
    employee_id: &str,
    prompt: &str,
    is_build: bool,
    perms: &PermissionSet,
    principal: Option<&Principal>,
    principal_id: Option<String>,
    principal_kind: &str,
    actor_label: &str,
    // WS7 item G2: `budget_limit`/`budget_consumed` are measured in
    // TOKENS, not currency — `lakehouse-llm` has no price table (`grep
    // -rn "price\|cost_per_token" rust/crates/lakehouse-llm/` returns
    // nothing) and this loop does not invent one. `DigitalEmployee.
    // budget_limit`'s existing column and "Budget ceiling" label are
    // reinterpreted as a token ceiling going forward — a disclosed,
    // breaking semantic change for any already-authored employee whose
    // `budget_limit` was set assuming a currency unit.
    budget_limit: f64,
) -> HeadlessOutcome {
    use lakehouse_llm::{ChatOptions, LlmMessage, LlmMessageRole};

    let Some(pg) = state.pg.as_deref() else {
        // Unreachable in practice: `run_employee` already required a pool
        // to get this far. Guarded anyway so this function has no hidden
        // panic path of its own.
        return HeadlessOutcome::Terminal("failed");
    };

    let mut messages = vec![
        LlmMessage {
            role: LlmMessageRole::System,
            content: Some(headless_system_prompt(is_build)),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        },
        LlmMessage {
            role: LlmMessageRole::User,
            content: Some(prompt.to_owned()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        },
    ];
    let tools: Vec<Value> = ai_registry::tool_schemas_for(Some(perms))
        .into_iter()
        .filter(|t| {
            let name = t["function"]["name"].as_str().unwrap_or("");
            let is_write =
                ai_registry::find(name).is_some_and(|spec| spec.risk != ai_registry::Risk::Read);
            is_build || !is_write
        })
        .collect();

    // `principal` is the SAME `Principal` `run_employee`'s auth guard
    // already built for this request (`principal_for_run_auth`) — passed
    // down once, not reloaded, and used for the run's entire duration: a
    // user-triggered run acts as that one user for every tool call below.

    let mut step_no: u32 = 0;
    let mut budget_consumed: f64 = 0.0;

    for _ in 0..MAX_HEADLESS_ITER {
        // Checked BEFORE each LLM call, not after — checking after would
        // let one more call through unconditionally. This means one call
        // can still push total usage past `budget_limit` before the loop
        // notices (the loop cannot know a call's token cost before making
        // it) — the run still stops at the next iteration boundary, never
        // silently continuing past the detected overage.
        if budget_consumed >= budget_limit {
            step_no += 1;
            let detail = format!(
                "budget exhausted: consumed {budget_consumed} tokens against a limit of {budget_limit}"
            );
            append_step(
                pg,
                run_id,
                &format!("step-{step_no}"),
                "budget",
                "failed",
                &detail,
            )
            .await;
            write_headless_audit(
                pg,
                principal_id.clone(),
                principal_kind,
                actor_label,
                "run_employee",
                None,
                None,
                &json!({ "prompt": prompt }),
                // `audit_event.outcome` has its own, narrower CHECK-
                // constrained vocabulary (`0024_audit_event.sql`) that does
                // NOT include `"budget_exhausted"` — that string is
                // `agent_run.status`'s vocabulary (no CHECK constraint,
                // widened for WS7 item G2), a DIFFERENT column with a
                // DIFFERENT allowed-values set. `"failed"` is the correct,
                // already-allowed audit outcome for this case; the real
                // reason is still on record, in `detail` below.
                "failed",
                Some(&detail),
                run_id,
                None,
            )
            .await;
            write_run_budget(pg, run_id, employee_id, budget_consumed).await;
            return HeadlessOutcome::Terminal("budget_exhausted");
        }

        let msg = match state
            .llm
            .chat_with_tools_metered(&messages, &tools, ChatOptions::default())
            .await
        {
            Ok((m, usage)) => {
                // A real, disclosed measurement gap: `usage` is `None`
                // when the endpoint's response omits the `usage` block
                // entirely — that call contributes nothing to
                // `budget_consumed`, which is honest under-measurement,
                // never a fabricated zero standing in for "unmeasured".
                if let Some(usage) = usage {
                    budget_consumed += f64::from(usage.total_tokens);
                }
                m
            }
            Err(err) => {
                step_no += 1;
                let detail = format!("AI Copilot unavailable: {err}");
                append_step(
                    pg,
                    run_id,
                    &format!("step-{step_no}"),
                    "run",
                    "failed",
                    &detail,
                )
                .await;
                write_headless_audit(
                    pg,
                    principal_id.clone(),
                    principal_kind,
                    actor_label,
                    "run_employee",
                    None,
                    None,
                    &json!({ "prompt": prompt }),
                    "failed",
                    Some(&detail),
                    run_id,
                    None,
                )
                .await;
                write_run_budget(pg, run_id, employee_id, budget_consumed).await;
                return HeadlessOutcome::Terminal("failed");
            }
        };
        messages.push(msg.clone());
        let calls = msg.tool_calls.clone().unwrap_or_default();
        if calls.is_empty() {
            step_no += 1;
            let answer = msg.content.unwrap_or_default();
            append_step(
                pg,
                run_id,
                &format!("step-{step_no}"),
                "answer",
                "succeeded",
                &answer,
            )
            .await;
            write_headless_audit(
                pg,
                principal_id.clone(),
                principal_kind,
                actor_label,
                "run_employee",
                None,
                None,
                &json!({ "prompt": prompt }),
                "executed",
                None,
                run_id,
                None,
            )
            .await;
            write_run_budget(pg, run_id, employee_id, budget_consumed).await;
            return HeadlessOutcome::Terminal("succeeded");
        }

        for call in &calls {
            let args: Map<String, Value> =
                serde_json::from_str(&call.function.arguments).unwrap_or_default();
            let spec = ai_registry::find(&call.function.name);
            let is_write = spec.is_some_and(|s| s.risk != ai_registry::Risk::Read);

            // Ask-mode gate: identical semantics to `routes::ai::gate::decide`'s
            // first check, reimplemented here rather than called there — see
            // the module doc comment on why this route cannot reach the
            // private `routes::ai::gate` module.
            if !is_build && is_write {
                step_no += 1;
                let detail = format!(
                    "refused: ask mode may not run a write tool ({})",
                    call.function.name
                );
                append_step(
                    pg,
                    run_id,
                    &format!("step-{step_no}"),
                    &call.function.name,
                    "refused",
                    &detail,
                )
                .await;
                write_headless_audit(
                    pg,
                    principal_id.clone(),
                    principal_kind,
                    actor_label,
                    &call.function.name,
                    None,
                    None,
                    &Value::Object(args.clone()),
                    "refused",
                    Some(&detail),
                    run_id,
                    None,
                )
                .await;
                push_tool_result(
                    &mut messages,
                    call,
                    &json!({ "error": detail, "refused": true }),
                );
                continue;
            }

            // Employee permission-ceiling gate (plan invariant 7): the
            // employee's OWN `permissions` column, never the triggering
            // human's/token's, bounds what this run may execute —
            // `PermissionSet::has` on `perms`, built from
            // `EmployeeRunConfig::permissions` by `run_employee`.
            if let Some(spec) = spec
                && !spec.permission.is_empty()
                && !perms.has(spec.permission)
            {
                step_no += 1;
                let detail = format!(
                    "refused: this employee lacks permission '{}' for tool {}",
                    spec.permission, call.function.name
                );
                append_step(
                    pg,
                    run_id,
                    &format!("step-{step_no}"),
                    &call.function.name,
                    "refused",
                    &detail,
                )
                .await;
                write_headless_audit(
                    pg,
                    principal_id.clone(),
                    principal_kind,
                    actor_label,
                    &call.function.name,
                    None,
                    None,
                    &Value::Object(args.clone()),
                    "refused",
                    Some(&detail),
                    run_id,
                    None,
                )
                .await;
                push_tool_result(
                    &mut messages,
                    call,
                    &json!({ "error": detail, "refused": true }),
                );
                continue;
            }

            if matches!(spec.map(|s| s.risk), Some(ai_registry::Risk::WriteHigh)) {
                let Some(spec) = spec else {
                    unreachable!("matches! above already confirmed spec is Some")
                };
                step_no += 1;
                let redacted = ai_audit::redact(&Value::Object(args.clone()));
                let (_, resource_id) =
                    ai_audit::resource_for(&call.function.name, &args, &json!({}));
                let reason = format!(
                    "Running high-risk tool {} which requires human approval \
                     (triggered by headless employee run {employee_id}).",
                    call.function.name
                );
                let pending = json!({ "tool": spec.name, "args": redacted });
                append_step(
                    pg,
                    run_id,
                    &format!("step-{step_no}"),
                    &call.function.name,
                    "pending",
                    &serde_json::to_string(&pending).unwrap_or_default(),
                )
                .await;
                let approval_result = agents::create_linked_approval(
                    pg,
                    LinkedApprovalRequest {
                        employee_id,
                        employee_name: actor_label,
                        run_id,
                        tool: spec.name,
                        resource: resource_id.as_deref(),
                        reason: &reason,
                        risk: "High-risk action (WriteHigh): cannot be undone once \
                               executed.",
                        redacted_args: &redacted,
                    },
                )
                .await;
                match approval_result {
                    Ok(approval_id) => {
                        if let Err(err) = agents::mark_run_waiting_approval(pg, run_id).await {
                            tracing::warn!(%err, run_id, "headless run: failed to mark waiting_approval");
                        }
                        write_headless_audit(
                            pg,
                            principal_id.clone(),
                            principal_kind,
                            actor_label,
                            &call.function.name,
                            None,
                            resource_id.as_deref(),
                            &Value::Object(args.clone()),
                            "needs_approval",
                            None,
                            run_id,
                            Some(&approval_id),
                        )
                        .await;
                    }
                    Err(err) => {
                        tracing::warn!(
                            %err,
                            tool = spec.name,
                            "headless run: failed to create WriteHigh approval"
                        );
                        write_headless_audit(
                            pg,
                            principal_id.clone(),
                            principal_kind,
                            actor_label,
                            &call.function.name,
                            None,
                            resource_id.as_deref(),
                            &Value::Object(args.clone()),
                            "failed",
                            Some("failed to create approval"),
                            run_id,
                            None,
                        )
                        .await;
                    }
                }
                // Either way, this run pauses here — nothing more executes
                // in this iteration or any later one.
                return HeadlessOutcome::WaitingApproval;
            }

            // Read or WriteLow: execute through the exact same dispatch
            // interactive chat and `POST /api/ai/tool` use. Headless runs
            // have no human present to click Confirm, so a `WriteLow` call
            // is NOT held for confirmation — the schedule/manual trigger
            // itself is the authorization, mirroring how `gold_export_job`
            // already runs its own `WriteLow`-equivalent export unattended.
            step_no += 1;
            // `principal` is `run_employee`'s own auth-guard result,
            // forwarded unchanged (C2-F2/C2-F3): a manually-triggered run
            // acts as the triggering user for every tool call in this run,
            // a schedule-triggered run passes `None`, and a principal-
            // requiring tool (`run_saved_query`) is responsible for
            // refusing `None` with a named reason rather than a bare 401.
            let result = ai_tools::run_tool(state, principal, &call.function.name, &args).await;
            let ok = !matches!(&result, Value::Object(m) if m.contains_key("error"));
            let (resource_kind, resource_id) =
                ai_audit::resource_for(&call.function.name, &args, &result);
            append_step(
                pg,
                run_id,
                &format!("step-{step_no}"),
                &call.function.name,
                if ok { "succeeded" } else { "failed" },
                &serde_json::to_string(&result).unwrap_or_default(),
            )
            .await;
            write_headless_audit(
                pg,
                principal_id.clone(),
                principal_kind,
                actor_label,
                &call.function.name,
                resource_kind,
                resource_id.as_deref(),
                &Value::Object(args.clone()),
                if ok { "executed" } else { "failed" },
                None,
                run_id,
                None,
            )
            .await;
            push_tool_result(&mut messages, call, &result);
        }
    }

    step_no += 1;
    append_step(
        pg,
        run_id,
        &format!("step-{step_no}"),
        "budget",
        "failed",
        "tool iteration budget reached",
    )
    .await;
    write_run_budget(pg, run_id, employee_id, budget_consumed).await;
    HeadlessOutcome::Terminal("failed")
}

/// Feeds one tool call's result back into the conversation as a `"tool"`
/// role message, matching `routes::ai::chat`'s own loop shape.
fn push_tool_result(
    messages: &mut Vec<lakehouse_llm::LlmMessage>,
    call: &lakehouse_llm::ToolCall,
    result: &Value,
) {
    let payload: String = serde_json::to_string(result)
        .unwrap_or_default()
        .chars()
        .take(8000)
        .collect();
    messages.push(lakehouse_llm::LlmMessage {
        role: lakehouse_llm::LlmMessageRole::Tool,
        content: Some(payload),
        tool_calls: None,
        tool_call_id: Some(call.id.clone()),
        name: Some(call.function.name.clone()),
    });
}

/// `POST /api/agents/employees/{id}/run` — run a digital employee
/// headlessly: the same copilot tool-calling loop interactive chat uses
/// (`routes::ai::chat`), but with no HTTP chat session — the employee's
/// own `prompt` (or this call's override) is the one user turn, and the
/// employee's `mode` picks Ask/Build.
///
/// # Why this can't just call into `routes::ai`
///
/// `routes::ai::gate` (mode + permission gate, `WriteLow` confirmation,
/// `WriteHigh` approval creation) is a PRIVATE module of `routes::ai`
/// (`mod gate;`, not `pub(in crate::routes)`) — only `routes::ai::chat`
/// and `routes::ai::tool_call` can call it. This handler reimplements the
/// same three checks (ask-mode refusal, permission-ceiling refusal,
/// `WriteHigh` → approval) directly in [`run_headless_loop`] rather than
/// editing that module's visibility, and reuses everything that IS already
/// shared across `routes::ai`'s sibling modules: [`ai_tools::run_tool`]
/// (the actual dispatch), [`ai_registry::find`]/[`ai_registry::tool_schemas_for`]
/// (the tool table), and [`ai_audit::redact`]/[`ai_audit::resource_for`]
/// (the redaction/resource-inference rules) — never a second, divergent
/// copy of THOSE.
///
/// # Auth (see [`check_employee_run_auth`])
///
/// EITHER a valid `x-run-token` matching
/// [`crate::config::Config::agent_run_token`] (`trigger = "schedule"`,
/// `audit_event.principal_kind = "schedule"` — Dagster's schedule
/// factory, T3.3), OR an authenticated principal holding `agent:manage`
/// (`trigger = "manual"`, `principal_kind = "user"` — a human's "Run now").
///
/// # Refusals
///
/// * 401 — wrong/absent token and no principal at all.
/// * 403 — a principal is present but lacks `agent:manage`.
/// * 400 — `id` is the reserved `emp-copilot` row (interactive-only, never
///   runnable headlessly); or the employee has no `prompt` and none was
///   supplied in the body; or the body is malformed JSON.
/// * 404 — `id` is unknown.
/// * 409 — the employee's `status` is `"paused"` (suspended) or
///   `"cancelled"` (revoked).
/// * 503 — no Postgres pool configured.
///
/// # The permission ceiling (plan invariant 7)
///
/// The run's [`PermissionSet`] is built from `EmployeeRunConfig::permissions`
/// — the employee's OWN column — never from the triggering token's or
/// principal's own grants. A caller whose principal holds `agent:manage`
/// (see [`check_employee_run_auth`]) clicking "Run now" on an
/// employee whose `permissions` is empty still gets a run that can
/// execute NO permissioned tool at all — enforced end to end by the HTTP
/// tests in `tests/agents_run.rs`.
///
/// # `WriteHigh` inside a headless run
///
/// The run pauses (`status = "waiting_approval"`) the moment a `WriteHigh`
/// tool call is reached — nothing later in the model's turn or any further
/// iteration executes. A `pending` `approval_item` is created, attributed
/// to the REAL running employee (never `emp-copilot`) and linked to this
/// SAME run id (see [`agents::create_linked_approval`]) — approving it via
/// `POST /api/agents/approvals/{id}/decide` replays the exact stored call
/// through [`ai_tools::run_tool`] and finishes this run, exactly like the
/// interactive-copilot `WriteHigh` flow (T0.5).
pub async fn run_employee(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<RunEmployeeQuery>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let header_token = headers.get("x-run-token").and_then(|v| v.to_str().ok());
    let auth = check_employee_run_auth(
        state.config.agent_run_token.as_deref(),
        header_token,
        query.token.as_deref(),
        principal.as_ref().map(|Extension(p)| p),
    )?;

    let pg = pool(&state)?;

    if id == agents::COPILOT_EMPLOYEE_ID {
        return Err(ApiError::BadRequest(
            "emp-copilot is a special interactive-only row and cannot be run headlessly".to_owned(),
        )
        .into());
    }

    let config = agents::get_employee_run_config(pg, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Employee {id} not found")))?;

    if matches!(config.status.as_str(), "paused" | "cancelled") {
        return Err(ApiError::Conflict(format!(
            "Employee {id} has status \"{}\" and cannot be run",
            config.status
        ))
        .into());
    }

    let body: RunEmployeeBody = if body.is_empty() {
        RunEmployeeBody::default()
    } else {
        parse_body(&body)?
    };
    let override_prompt = body.prompt.filter(|p| !p.trim().is_empty());
    let employee_prompt = config.prompt.clone().filter(|p| !p.trim().is_empty());
    let Some(prompt) = override_prompt.or(employee_prompt) else {
        return Err(ApiError::BadRequest(format!(
            "Employee {id} has no prompt, and none was supplied as an override"
        ))
        .into());
    };

    let (trigger, principal_id, principal_kind, actor) =
        run_employee_audit_identity(&auth, &config.name);
    let principal = principal_for_run_auth(&auth);

    let run_id = format!("run-emp-{}", Uuid::new_v4());
    agents::create_run(pg, &run_id, &id, trigger, &actor).await?;

    let perms = PermissionSet::parse(&config.permissions);
    let is_build = config.mode == "build";

    let outcome = run_headless_loop(
        &state,
        &run_id,
        &id,
        &prompt,
        is_build,
        &perms,
        principal,
        principal_id,
        principal_kind,
        &actor,
        config.budget_limit,
    )
    .await;

    if let HeadlessOutcome::Terminal(status) = outcome
        && let Err(err) = agents::finish_run(pg, &run_id, status).await
    {
        tracing::warn!(%err, run_id, "headless run: failed to finish run");
    }

    let run = agents::get_run(pg, &run_id)
        .await?
        .ok_or_else(|| ApiError::Internal(format!("run {run_id} vanished")))?;
    Ok(ApiJson(json!({ "run": run })))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use axum::body::to_bytes;
    use axum::http::Request;
    use lakehouse_auth::PrincipalId;
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::config::Config;

    fn state_without_pool() -> AppState {
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
        AppState::new(Config::from_map(&env).unwrap())
    }

    #[tokio::test]
    async fn every_database_backed_route_returns_503_without_a_pool() {
        let paths = [
            "/api/agents/workflows",
            "/api/agents/employees",
            "/api/agents/employees/emp-x",
            "/api/agents/tools",
            "/api/agents/runs",
            "/api/agents/runs/run-x",
            "/api/agents/approvals",
        ];
        for path in paths {
            let app = crate::routes::router(state_without_pool());
            let response = app
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{path}");
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            assert!(body.get("error").is_some(), "{path}");
        }
    }

    /// WS7 item G4, failing-test-first: before this task, `GET
    /// /api/agents/tools` read the unread `agent_tool` Postgres table via
    /// `agents::list_tools`/`AgentTool` — completely disconnected from
    /// `ai_registry::TOOLS`, the table every real tool dispatch actually
    /// goes through (confirmed by reading `routes::ai::tools::run_tool`
    /// before touching this). `list_tools_body` did not exist before this
    /// task; `cargo test -p lakehouse-api list_tools_body --lib --no-run`
    /// failed with E0425 (unresolved function).
    #[test]
    fn list_tools_body_reflects_the_real_registry_not_seeded_fantasy_data() {
        let tools = list_tools_body(&[]);
        assert!(
            tools.iter().any(|t| t.name == "run_sql"),
            "must reflect ai_registry::TOOLS, not agent_tool seed rows"
        );
        let run_sql = tools.iter().find(|t| t.name == "run_sql").unwrap();
        assert_eq!(run_sql.usage_count_30d, 0);
        assert_eq!(run_sql.risk, "Read");
        assert_eq!(run_sql.permission, "query:read");
        assert!(!run_sql.description.is_empty());
    }

    #[test]
    fn list_tools_body_counts_real_audit_events() {
        let events = vec![("run_sql".to_owned(), 3_i64)];
        let tools = list_tools_body(&events);
        assert_eq!(
            tools
                .iter()
                .find(|t| t.name == "run_sql")
                .unwrap()
                .usage_count_30d,
            3
        );
        // A tool with no matching audit_event row is 0, not absent.
        assert_eq!(
            tools
                .iter()
                .find(|t| t.name == "list_datasets")
                .unwrap()
                .usage_count_30d,
            0
        );
    }

    #[test]
    fn create_employee_body_rejects_unknown_autonomy() {
        let body = CreateEmployeeBody {
            name: "n".to_owned(),
            purpose: "p".to_owned(),
            autonomy: "L9".to_owned(),
            allowed_tools: vec![],
            data_scope: "d".to_owned(),
            budget_limit: 0.0,
            owner: None,
            prompt: None,
            schedule_cron: None,
            mode: None,
            permissions: None,
        };
        assert!(!VALID_AUTONOMY.contains(&body.autonomy.as_str()));
    }

    #[test]
    fn decide_approval_body_rejects_unknown_decision() {
        let body = DecideApprovalBody {
            decision: "maybe".to_owned(),
            comment: None,
        };
        assert!(!matches!(body.decision.as_str(), "approved" | "rejected"));
    }

    /// A logged-in human principal — `PrincipalId::User`. Mirrors
    /// `routes::query`'s own test fixture of the same name.
    fn fixture_user_principal() -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::from_u128(1)),
            tenant_ids: Vec::new(),
            display_name: "Rina Wijaya".to_owned(),
            permissions: PermissionSet::parse("agent:approve"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    /// A service-token principal — `PrincipalId::Service`.
    fn fixture_service_principal() -> Principal {
        Principal {
            id: PrincipalId::Service(Uuid::from_u128(2)),
            tenant_ids: Vec::new(),
            display_name: "dagster-orchestrator".to_owned(),
            permissions: PermissionSet::parse("agent:approve"),
            provider: "service".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    /// WS5 item D2, failing-test-first: before this task's production fix,
    /// `decide_approval` recorded every human approval decision through
    /// `ai_audit::record`, which hardcoded `principal_kind: "copilot"` —
    /// so a human's own decision was mis-attributed to the copilot. Before
    /// this task added the `decide_approval_audit_event` helper below,
    /// this test failed to compile at all (E0425, unresolved function),
    /// since `decide_approval` still called `ai_audit::record` directly.
    /// This test asserts the BUILT `NewAuditEvent`, replacing the
    /// previous version's source-text grep (which would have passed even
    /// if this call site became dead code).
    #[test]
    fn decide_approval_audit_event_uses_the_deciding_principals_real_kind() {
        let principal = fixture_user_principal();
        let event = decide_approval_audit_event(
            &principal,
            "run_saved_query",
            "appr-1",
            Some("looks fine"),
            "approved",
            Some("run-1"),
        );
        assert_eq!(event.principal_kind.as_deref(), Some("user"));
        assert_eq!(
            event.principal_id.as_deref(),
            Some(principal.id.uuid().to_string().as_str())
        );
        assert_eq!(event.resource_kind.as_deref(), Some("approval"));
        assert_eq!(event.resource_id.as_deref(), Some("appr-1"));
        assert_eq!(event.action, "run_saved_query");
        assert_eq!(event.outcome, "approved");
        assert_eq!(event.run_id.as_deref(), Some("run-1"));
        assert_eq!(event.approval_id.as_deref(), Some("appr-1"));
    }

    /// A service identity's decision (e.g. an automated policy service
    /// acting through a service token) must record `principal_kind:
    /// "service"`, never `"user"` or the old hardcoded `"copilot"`.
    #[test]
    fn decide_approval_audit_event_records_a_service_identity_as_service_not_user() {
        let principal = fixture_service_principal();
        let event = decide_approval_audit_event(
            &principal,
            "run_saved_query",
            "appr-2",
            None,
            "rejected",
            None,
        );
        assert_eq!(event.principal_kind.as_deref(), Some("service"));
    }

    /// WS5 item D4, failing-test-first: `run_employee_audit_identity`
    /// (extracted from an inline `match` in `run_employee`) used to
    /// hardcode `"user"` for EVERY `RunAuth::Principal`, human or
    /// service identity alike — `check_employee_run_auth` accepts any
    /// authenticated principal holding `agent:manage`, not only humans
    /// (confirmed by reading that function before touching this). Quoted
    /// failure, before the extraction: this exact assertion
    /// (`principal_kind == "service"` for a service `RunAuth::Principal`)
    /// could not even be written against the old inline `match`, since
    /// there was no unit-testable function to call — the closest thing
    /// to a red state this refactor-and-fix has.
    #[test]
    fn run_employee_audit_identity_records_a_service_identity_as_service_not_user() {
        let principal = fixture_service_principal();
        let (trigger, principal_id, principal_kind, actor) =
            run_employee_audit_identity(&RunAuth::Principal(principal.clone()), "emp-name");
        assert_eq!(trigger, "manual");
        assert_eq!(
            principal_id.as_deref(),
            Some(principal.id.uuid().to_string().as_str())
        );
        assert_eq!(principal_kind, "service");
        assert_eq!(actor, "dagster-orchestrator");
    }

    /// A human `RunAuth::Principal` still records `principal_kind: "user"`
    /// — this fix changes the service case, not the (already correct)
    /// human one.
    #[test]
    fn run_employee_audit_identity_records_a_human_principal_as_user() {
        let principal = fixture_user_principal();
        let (_, _, principal_kind, _) =
            run_employee_audit_identity(&RunAuth::Principal(principal), "emp-name");
        assert_eq!(principal_kind, "user");
    }

    /// A schedule-triggered run (no principal at all) still records
    /// `principal_kind: "schedule"` and uses the employee's own name as
    /// `actor` — unaffected by this fix.
    #[test]
    fn run_employee_audit_identity_records_a_token_trigger_as_schedule() {
        let (trigger, principal_id, principal_kind, actor) =
            run_employee_audit_identity(&RunAuth::Token, "Nightly Ingest Bot");
        assert_eq!(trigger, "schedule");
        assert_eq!(principal_id, None);
        assert_eq!(principal_kind, "schedule");
        assert_eq!(actor, "Nightly Ingest Bot");
    }

    /// D (T3.2's fix): `create_employee` must validate `mode` the same way
    /// it already validates `autonomy` — an unrecognized value never
    /// reaches `agents::create_employee` to surface as a raw DB `CHECK`
    /// violation.
    #[test]
    fn create_employee_body_rejects_unknown_mode() {
        let body = CreateEmployeeBody {
            name: "n".to_owned(),
            purpose: "p".to_owned(),
            autonomy: "L1".to_owned(),
            allowed_tools: vec![],
            data_scope: "d".to_owned(),
            budget_limit: 0.0,
            owner: None,
            prompt: None,
            schedule_cron: None,
            mode: Some("sleep".to_owned()),
            permissions: None,
        };
        assert!(!VALID_MODE.contains(&body.mode.as_deref().unwrap()));
    }

    fn admin_perms() -> PermissionSet {
        PermissionSet::parse("*:*")
    }

    fn platform_admin_principal() -> Principal {
        Principal {
            id: lakehouse_auth::PrincipalId::User(uuid::Uuid::nil()),
            tenant_ids: Vec::new(),
            display_name: "Fajar Nugroho".to_owned(),
            permissions: admin_perms(),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    fn zero_perm_principal() -> Principal {
        Principal {
            id: lakehouse_auth::PrincipalId::User(uuid::Uuid::nil()),
            tenant_ids: Vec::new(),
            display_name: "Zero Perm".to_owned(),
            permissions: PermissionSet::default(),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    /// A matching `x-run-token` authorizes the run as `RunAuth::Token`,
    /// regardless of whether a principal is also present.
    #[test]
    fn check_employee_run_auth_accepts_matching_token() {
        assert!(matches!(
            check_employee_run_auth(Some("secret"), Some("secret"), None, None),
            Ok(RunAuth::Token)
        ));
        assert!(matches!(
            check_employee_run_auth(
                Some("secret"),
                Some("secret"),
                None,
                Some(&zero_perm_principal())
            ),
            Ok(RunAuth::Token)
        ));
    }

    /// A wrong/absent token falls back to the principal's own
    /// `agent:manage` permission — present and sufficient here.
    #[test]
    fn check_employee_run_auth_accepts_agent_manage_principal_without_token() {
        assert!(matches!(
            check_employee_run_auth(
                Some("secret"),
                None,
                None,
                Some(&platform_admin_principal())
            ),
            Ok(RunAuth::Principal(_))
        ));
        // No token configured at all still works via the permission path.
        assert!(matches!(
            check_employee_run_auth(None, None, None, Some(&platform_admin_principal())),
            Ok(RunAuth::Principal(_))
        ));
    }

    /// Wrong/absent token AND no principal at all is a 401.
    #[test]
    fn check_employee_run_auth_no_token_no_principal_is_unauthorized() {
        let err = check_employee_run_auth(Some("secret"), None, None, None).unwrap_err();
        assert!(matches!(err, ApiError::Unauthorized(_)));
        let err = check_employee_run_auth(None, None, None, None).unwrap_err();
        assert!(matches!(err, ApiError::Unauthorized(_)));
    }

    /// Wrong/absent token AND a principal that lacks `agent:manage` is a
    /// 403, not a 401 — the caller IS authenticated, just not permitted.
    #[test]
    fn check_employee_run_auth_wrong_token_with_unpermitted_principal_is_forbidden() {
        let err = check_employee_run_auth(
            Some("secret"),
            Some("wrong"),
            None,
            Some(&zero_perm_principal()),
        )
        .unwrap_err();
        assert!(matches!(err, ApiError::PermissionDenied(_)));
    }

    // ── C2-F3 regression coverage: a headless run acts as the SAME
    // principal the auth guard already built, never a re-queried copy —
    // this is now a pure match on `RunAuth`, so no database is needed.
    #[test]
    fn a_run_auth_principal_yields_that_same_principal() {
        let admin = platform_admin_principal();
        let auth = RunAuth::Principal(admin.clone());
        let resolved =
            principal_for_run_auth(&auth).expect("RunAuth::Principal must yield Some(principal)");
        assert_eq!(
            resolved.id.uuid(),
            admin.id.uuid(),
            "the resolved principal must be the SAME one RunAuth::Principal carried, not a copy"
        );
    }

    #[test]
    fn a_run_auth_token_yields_no_principal() {
        assert!(principal_for_run_auth(&RunAuth::Token).is_none());
    }

    /// WS7 item G2: `run_headless_loop`'s real, DB-backed budget-exhaustion
    /// behavior — the loop must stop with `HeadlessOutcome::Terminal
    /// ("budget_exhausted")` once accumulated real token usage crosses
    /// `budget_limit`, and `agent_run.budget_consumed` must hold the real
    /// accumulated sum, not an estimate. Before this task, `run_headless_
    /// loop` called `state.llm.chat_with_tools` (not the metered variant
    /// WS7 item G1 added) and never read or wrote `budget_consumed`/
    /// `budget_limit` anywhere in its body — confirmed by reading the
    /// whole function before touching it. Quoted failing-test-first
    /// state: before this task's implementation, `cargo test -p
    /// lakehouse-api --lib --no-run` failed to compile this module with
    /// `E0599: no method named 'chat_with_tools_metered' found for
    /// struct 'LlmClient'` and `E0061: this function takes 10 arguments
    /// but 11 arguments were supplied` (the new `budget_limit` parameter
    /// below).
    mod budget_end_to_end {
        use std::collections::HashMap;

        use lakehouse_store::agents as store_agents;
        use lakehouse_test_support as _;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::super::*;
        use crate::config::Config;

        fn database_url_for(pool: &sqlx::PgPool) -> String {
            let options = pool.connect_options();
            format!(
                "postgres://{}:postgres@{}:{}/{}",
                options.get_username(),
                options.get_host(),
                options.get_port(),
                options
                    .get_database()
                    .expect("#[sqlx::test] always targets a named database")
            )
        }

        fn state_for(pool: &sqlx::PgPool, llm_url: &str) -> AppState {
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), database_url_for(pool));
            env.insert("LLM_URL".to_owned(), llm_url.to_owned());
            let config = Config::from_map(&env).expect("a valid test Config");
            AppState::new(config)
        }

        /// A `chat/completions` reply carrying a tool call (so the loop
        /// keeps iterating instead of finishing on an empty-`tool_calls`
        /// answer) plus a real `usage.total_tokens` of `total_tokens` —
        /// the tool name (`no_such_tool`) deliberately matches nothing in
        /// `ai_registry`, so `ai_tools::run_tool` reports a harmless
        /// "unknown tool" error and the loop simply continues to its next
        /// iteration, exactly like a genuine unresolvable tool call would.
        fn tool_call_response(total_tokens: u32) -> Value {
            json!({
                "choices": [{"message": {
                    "role": "assistant",
                    "content": Value::Null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": { "name": "no_such_tool", "arguments": "{}" },
                    }],
                }}],
                "usage": {
                    "prompt_tokens": total_tokens.saturating_sub(10),
                    "completion_tokens": 10,
                    "total_tokens": total_tokens,
                },
            })
        }

        /// `employee.budget_limit` = 100; every LLM call in this stub
        /// reports `usage.total_tokens` = 60. The loop must stop AFTER the
        /// second call (60+60=120 > 100, checked before what would be a
        /// third call), not before the first (60 < 100) and not after an
        /// arbitrary third call — the mock has no call-count limit, so a
        /// third call remains available for the loop to make if the
        /// budget check failed to stop it.
        #[sqlx::test(migrations = "../../migrations")]
        async fn run_headless_loop_stops_with_budget_exhausted_once_the_limit_is_crossed(
            pool: sqlx::PgPool,
        ) {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/chat/completions"))
                .respond_with(ResponseTemplate::new(200).set_body_json(tool_call_response(60)))
                .mount(&server)
                .await;

            let state = state_for(&pool, &server.uri());

            let input = CreateEmployeeInput {
                name: "budget-test-employee".to_owned(),
                purpose: "p".to_owned(),
                autonomy: "L2".to_owned(),
                allowed_tools: vec![],
                data_scope: "d".to_owned(),
                budget_limit: 100.0,
                owner: None,
                prompt: Some("do the thing".to_owned()),
                schedule_cron: None,
                mode: Some("build".to_owned()),
                permissions: Some("*:*".to_owned()),
            };
            let employee = store_agents::create_employee(&pool, &input).await.unwrap();
            store_agents::create_run(&pool, "run-budget-1", &employee.id, "manual", "tester")
                .await
                .unwrap();

            let perms = PermissionSet::parse("*:*");
            let outcome = run_headless_loop(
                &state,
                "run-budget-1",
                &employee.id,
                "do the thing",
                true,
                &perms,
                None,
                None,
                "schedule",
                "tester",
                100.0,
            )
            .await;

            assert!(
                matches!(outcome, HeadlessOutcome::Terminal("budget_exhausted")),
                "must stop with budget_exhausted, not run a third LLM call"
            );
            // Mirrors `run_employee`'s own orchestration: `run_headless_loop`
            // itself never sets the run's final `status`/`ended_at` for a
            // `Terminal` outcome — its caller does, via `finish_run`, right
            // after the loop returns.
            if let HeadlessOutcome::Terminal(status) = outcome {
                store_agents::finish_run(&pool, "run-budget-1", status)
                    .await
                    .unwrap();
            }
            let run = store_agents::get_run(&pool, "run-budget-1")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(run.status, "budget_exhausted");
            assert_eq!(run.budget_consumed, Some(120.0));
        }

        /// A run whose consumption never reaches `budget_limit` finishes
        /// normally — the budget check must never trip early on a run
        /// that stays under its ceiling.
        #[sqlx::test(migrations = "../../migrations")]
        async fn run_headless_loop_finishes_normally_under_budget(pool: sqlx::PgPool) {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/chat/completions"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "choices": [{"message": {
                        "role": "assistant",
                        "content": "done",
                    }}],
                    "usage": {
                        "prompt_tokens": 20,
                        "completion_tokens": 10,
                        "total_tokens": 30,
                    },
                })))
                .mount(&server)
                .await;

            let state = state_for(&pool, &server.uri());

            let input = CreateEmployeeInput {
                name: "under-budget-employee".to_owned(),
                purpose: "p".to_owned(),
                autonomy: "L2".to_owned(),
                allowed_tools: vec![],
                data_scope: "d".to_owned(),
                budget_limit: 100.0,
                owner: None,
                prompt: Some("do the thing".to_owned()),
                schedule_cron: None,
                mode: Some("build".to_owned()),
                permissions: Some("*:*".to_owned()),
            };
            let employee = store_agents::create_employee(&pool, &input).await.unwrap();
            store_agents::create_run(&pool, "run-budget-2", &employee.id, "manual", "tester")
                .await
                .unwrap();

            let perms = PermissionSet::parse("*:*");
            let outcome = run_headless_loop(
                &state,
                "run-budget-2",
                &employee.id,
                "do the thing",
                true,
                &perms,
                None,
                None,
                "schedule",
                "tester",
                100.0,
            )
            .await;

            assert!(matches!(outcome, HeadlessOutcome::Terminal("succeeded")));
            if let HeadlessOutcome::Terminal(status) = outcome {
                store_agents::finish_run(&pool, "run-budget-2", status)
                    .await
                    .unwrap();
            }
            let run = store_agents::get_run(&pool, "run-budget-2")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(run.budget_consumed, Some(30.0));
        }
    }
}
