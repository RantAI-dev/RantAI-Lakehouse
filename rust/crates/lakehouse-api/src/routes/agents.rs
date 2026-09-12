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

use axum::body::Bytes;
use axum::extract::{Extension, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use lakehouse_auth::{PermissionSet, Principal};
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::agents::{
    self, AgentRun, AgentTool, AgentWorkflow, ApprovalItem, CreateEmployeeInput,
    CreateWorkflowInput, Decision, DigitalEmployee, LinkedApprovalRequest, RegisterToolInput,
    RunStep,
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
        return Err(ApiError::BadRequest(format!("{field} wajib diisi")));
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

/// `GET /api/agents/employees/{id}`.
///
/// # Errors
///
/// 404 if `id` is unknown; 503/500 as above.
pub async fn get_employee(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<DigitalEmployee>> {
    let employee = agents::get_employee(pool(&state)?, &id)
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

/// `GET /api/agents/tools`.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_tools(State(state): State<AppState>) -> ApiResult<ApiJson<Vec<AgentTool>>> {
    Ok(ApiJson(agents::list_tools(pool(&state)?).await?))
}

/// The `POST /api/agents/tools` body. Mirrors `RegisterToolInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterToolBody {
    name: String,
    version: String,
    publisher: String,
    permission: String,
    rate_limit: String,
}

/// `POST /api/agents/tools` — register a tool. Returns 201.
///
/// # Errors
///
/// 400 on a malformed body or a blank required field; 409 if the name is
/// taken; 503/500 as above.
pub async fn register_tool(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<AgentTool>)> {
    let body: RegisterToolBody = parse_body(&body)?;
    let input = RegisterToolInput {
        name: required("name", &body.name)?,
        version: required("version", &body.version)?,
        publisher: required("publisher", &body.publisher)?,
        permission: required("permission", &body.permission)?,
        rate_limit: required("rateLimit", &body.rate_limit)?,
    };
    let created = agents::register_tool(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(created)))
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
    ai_audit::record(
        Some(pg),
        Some(&principal),
        None,
        &approval.action,
        Some("approval"),
        Some(&id),
        &json!({ "comment": body.comment }),
        decide_outcome,
        None,
        approval.run_id.as_deref(),
        Some(&id),
    )
    .await;

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
            "Ditolak",
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
        let detail = format!("tool tidak dikenal: {}", approval.action);
        if let Err(err) =
            agents::record_run_outcome(pg, &run_id, "failed", "Tidak dieksekusi", &detail).await
        {
            tracing::warn!(%err, run_id, "failed to record not-executed outcome");
        }
        return Err(ApiError::Internal(detail).into());
    };

    if !spec.permission.is_empty() && !principal.permissions.has(spec.permission) {
        let detail = format!(
            "approval disetujui, TAPI tidak dieksekusi: approver tidak punya izin '{}' untuk \
             menjalankan tool ini",
            spec.permission
        );
        if let Err(err) =
            agents::record_run_outcome(pg, &run_id, "failed", "Tidak dieksekusi", &detail).await
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
        let detail = "run yang disetujui tidak menyimpan tool call".to_owned();
        if let Err(err) =
            agents::record_run_outcome(pg, &run_id, "failed", "Tidak dieksekusi", &detail).await
        {
            tracing::warn!(%err, run_id, "failed to record missing-tool-call outcome");
        }
        return Err(ApiError::Internal(detail).into());
    };
    let args: Map<String, Value> = pending.args.as_object().cloned().unwrap_or_default();
    let result = ai_tools::run_tool(&state, &pending.tool, &args).await;
    let ok = !matches!(&result, Value::Object(m) if m.contains_key("error"));
    let status = if ok { "succeeded" } else { "failed" };
    if let Err(err) = agents::record_run_outcome(
        pg,
        &run_id,
        status,
        "Eksekusi setelah disetujui",
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
            "agent:manage wajib untuk menjalankan employee secara manual".to_owned(),
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
    principal_id: Option<String>,
    principal_kind: &str,
    actor_label: &str,
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

    let mut step_no: u32 = 0;

    for _ in 0..MAX_HEADLESS_ITER {
        let msg = match state
            .llm
            .chat_with_tools(&messages, &tools, ChatOptions::default())
            .await
        {
            Ok(m) => m,
            Err(err) => {
                step_no += 1;
                let detail = format!("AI Copilot tak tersedia: {err}");
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
                    "ditolak: mode ask tidak boleh menjalankan tool tulis ({})",
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
                    "ditolak: employee ini tidak punya izin '{}' untuk tool {}",
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
                    "Menjalankan tool berisiko tinggi {} yang butuh persetujuan manusia \
                     (dipicu oleh run headless employee {employee_id}).",
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
                        risk: "Tindakan berisiko tinggi (WriteHigh): tidak dapat dibatalkan \
                               setelah dijalankan.",
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
                            Some("gagal membuat approval"),
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
            let result = ai_tools::run_tool(state, &call.function.name, &args).await;
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
        "batas iterasi tool tercapai",
    )
    .await;
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
/// principal's own grants. A Platform Admin clicking "Run now" on an
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
            "emp-copilot adalah baris interaktif khusus dan tidak bisa dijalankan headless"
                .to_owned(),
        )
        .into());
    }

    let config = agents::get_employee_run_config(pg, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Employee {id} not found")))?;

    if matches!(config.status.as_str(), "paused" | "cancelled") {
        return Err(ApiError::Conflict(format!(
            "Employee {id} berstatus \"{}\" dan tidak dapat dijalankan",
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
            "Employee {id} tidak punya prompt, dan tidak ada override yang diberikan"
        ))
        .into());
    };

    let (trigger, principal_id, principal_kind, actor): (&str, Option<String>, &str, String) =
        match &auth {
            RunAuth::Token => ("schedule", None, "schedule", config.name.clone()),
            RunAuth::Principal(p) => (
                "manual",
                Some(p.id.uuid().to_string()),
                "user",
                p.display_name.clone(),
            ),
        };

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
        principal_id,
        principal_kind,
        &actor,
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
}
