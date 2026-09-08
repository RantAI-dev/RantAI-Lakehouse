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
use axum::http::StatusCode;
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::agents::{
    self, AgentRun, AgentTool, AgentWorkflow, ApprovalItem, CreateEmployeeInput,
    CreateWorkflowInput, Decision, DigitalEmployee, RegisterToolInput,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

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
/// # Errors
///
/// 400 on a malformed body, a blank required field, or an unrecognized
/// `autonomy`; 409 if the name is taken; 503/500 as above.
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
}
