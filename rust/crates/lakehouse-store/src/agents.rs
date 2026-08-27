//! Repository layer for the `agents` domain: digital employee definitions,
//! tools, workflows, run history, and the approval lifecycle. Postgres
//! backing for `src/services/mock/agents.ts`.
//!
//! # Scope: configuration and history, not an execution runtime
//!
//! There is no agent runtime, orchestrator, or tool-invocation engine
//! anywhere in this repository (see `AI_PROJECT_INSIGHTS.md`). This module
//! persists *definitions* (employees, tools, workflows) and *records*
//! (past runs, approval decisions) — it never launches an agent, invokes a
//! tool, or produces a run that didn't already exist. `AgentService` (the
//! contract this backs) has no "run this agent" or "invoke this tool"
//! method, so nothing here is a scoped-down stand-in for one; the contract
//! itself never asked for an execution runtime.

use serde::Serialize;
use sqlx::FromRow;
use sqlx::types::Json;
use time::OffsetDateTime;

use crate::{PgPool, StoreError};

fn iso_millis(at: OffsetDateTime) -> String {
    let at = at.to_offset(time::UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second(),
        at.millisecond()
    )
}

fn iso_opt(at: Option<OffsetDateTime>) -> Option<String> {
    at.map(iso_millis)
}

const DEFAULT_OWNER: &str = "Current user";

/// A slug-based id, same shape `connectors::slug_id`/`knowledge::slug_id`
/// use.
fn slug_id(prefix: &str, name: &str) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug.trim_matches('-');
    let slug: String = slug.chars().take(32).collect();
    let slug = slug.trim_matches('-');
    let millis = OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
    #[allow(
        clippy::cast_sign_loss,
        reason = "unix millis since epoch is always positive"
    )]
    let millis = millis as u128;
    format!(
        "{prefix}-{}-{}",
        if slug.is_empty() { "new" } else { slug },
        radix36(millis)
    )
}

fn radix36(mut n: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".to_owned();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

// ---------------------------------------------------------------------
// Workflows
// ---------------------------------------------------------------------

/// Mirrors `AgentWorkflow` in `contracts/agents.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AgentWorkflow {
    /// `agent_workflow.id`.
    pub id: String,
    /// Display name; the table's natural key.
    pub name: String,
    /// Lifecycle status (`EntityStatus`).
    pub status: String,
    /// Owning team or person.
    pub owner: String,
    /// Human-readable trigger description.
    pub trigger: String,
    /// Number of steps in the workflow.
    pub steps: i64,
    /// Last run time.
    #[serde(rename = "lastRunAt", serialize_with = "ser_ts")]
    pub last_run_at: OffsetDateTime,
    /// Whether a run of this workflow requires an approval gate.
    pub approval_required: bool,
}

fn ser_ts<S: serde::Serializer>(at: &OffsetDateTime, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&iso_millis(*at))
}

const WORKFLOW_COLUMNS: &str =
    "id, name, status, owner, trigger, steps, last_run_at, approval_required";

/// List every workflow, newest first.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_workflows(pool: &PgPool) -> Result<Vec<AgentWorkflow>, StoreError> {
    let sql = format!("SELECT {WORKFLOW_COLUMNS} FROM agent_workflow ORDER BY created_at DESC");
    Ok(sqlx::query_as(&sql).fetch_all(pool).await?)
}

/// Everything [`create_workflow`] needs. Mirrors `CreateWorkflowInput`.
#[derive(Debug, Clone)]
pub struct CreateWorkflowInput {
    /// Display name; must not collide with an existing workflow.
    pub name: String,
    /// Human-readable trigger description.
    pub trigger: String,
    /// Step count (`stepKinds.length` in the contract).
    pub step_count: i64,
    /// Whether a run of this workflow requires an approval gate.
    pub approval_required: bool,
    /// Owner; defaults to [`DEFAULT_OWNER`] when absent.
    pub owner: Option<String>,
}

/// Create a workflow. `status` starts `"draft"` — same as
/// `mock/agents.ts`'s `createWorkflow`.
///
/// # Errors
///
/// Returns [`StoreError::Conflict`] (409) if the name is taken.
pub async fn create_workflow(
    pool: &PgPool,
    input: &CreateWorkflowInput,
) -> Result<AgentWorkflow, StoreError> {
    let id = slug_id("wf", &input.name);
    let owner = input.owner.as_deref().unwrap_or(DEFAULT_OWNER);
    let sql = format!(
        "INSERT INTO agent_workflow (id, name, status, owner, trigger, steps, approval_required) \
         VALUES ($1, $2, 'draft', $3, $4, $5, $6) \
         RETURNING {WORKFLOW_COLUMNS}"
    );
    Ok(sqlx::query_as(&sql)
        .bind(&id)
        .bind(&input.name)
        .bind(owner)
        .bind(&input.trigger)
        .bind(input.step_count)
        .bind(input.approval_required)
        .fetch_one(pool)
        .await?)
}

// ---------------------------------------------------------------------
// Employees
// ---------------------------------------------------------------------

/// Mirrors `DigitalEmployee` in `contracts/agents.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct DigitalEmployee {
    /// `agent_employee.id`.
    pub id: String,
    /// Display name; the table's natural key.
    pub name: String,
    /// Human-readable purpose statement.
    pub purpose: String,
    /// Owning team or person.
    pub owner: String,
    /// `"L1" | "L2" | "L3" | "L4"` (`AutonomyLevel`).
    pub autonomy: String,
    /// Lifecycle status (`EntityStatus`).
    pub status: String,
    /// Budget ceiling.
    pub budget_limit: f64,
    /// Budget spent to date.
    pub budget_spent: f64,
    /// Budget currently reserved (in-flight).
    pub budget_reserved: f64,
    /// Tool names this employee may invoke.
    pub allowed_tools: Vec<String>,
    /// Human-readable data-access scope.
    pub data_scope: String,
    /// Fraction of actions that required approval.
    pub approval_rate: f64,
    /// Fraction of runs that succeeded.
    pub success_rate: f64,
    /// Number of recent runs.
    pub recent_runs: i64,
}

const EMPLOYEE_COLUMNS: &str = "id, name, purpose, owner, autonomy, status, budget_limit, \
     budget_spent, budget_reserved, allowed_tools, data_scope, approval_rate, success_rate, \
     recent_runs";

/// List every digital employee, newest first.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_employees(pool: &PgPool) -> Result<Vec<DigitalEmployee>, StoreError> {
    let sql = format!("SELECT {EMPLOYEE_COLUMNS} FROM agent_employee ORDER BY created_at DESC");
    Ok(sqlx::query_as(&sql).fetch_all(pool).await?)
}

/// Fetch one employee by id.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get_employee(pool: &PgPool, id: &str) -> Result<Option<DigitalEmployee>, StoreError> {
    let sql = format!("SELECT {EMPLOYEE_COLUMNS} FROM agent_employee WHERE id = $1");
    Ok(sqlx::query_as(&sql).bind(id).fetch_optional(pool).await?)
}

/// Everything [`create_employee`] needs. Mirrors `CreateEmployeeInput`.
#[derive(Debug, Clone)]
pub struct CreateEmployeeInput {
    /// Display name; must not collide with an existing employee.
    pub name: String,
    /// Human-readable purpose statement.
    pub purpose: String,
    /// `"L1" | "L2" | "L3" | "L4"` (`AutonomyLevel`).
    pub autonomy: String,
    /// Tool names this employee may invoke.
    pub allowed_tools: Vec<String>,
    /// Human-readable data-access scope.
    pub data_scope: String,
    /// Budget ceiling.
    pub budget_limit: f64,
    /// Owner; defaults to [`DEFAULT_OWNER`] when absent.
    pub owner: Option<String>,
}

/// Create a digital employee. `status` starts `"draft"`, all counters
/// start at `0` — same as `mock/agents.ts`'s `createEmployee`.
///
/// # Errors
///
/// Returns [`StoreError::Conflict`] (409) if the name is taken.
pub async fn create_employee(
    pool: &PgPool,
    input: &CreateEmployeeInput,
) -> Result<DigitalEmployee, StoreError> {
    let id = slug_id("emp", &input.name);
    let owner = input.owner.as_deref().unwrap_or(DEFAULT_OWNER);
    let sql = format!(
        "INSERT INTO agent_employee (id, name, purpose, owner, autonomy, status, budget_limit, \
         allowed_tools, data_scope) \
         VALUES ($1, $2, $3, $4, $5, 'draft', $6, $7, $8) \
         RETURNING {EMPLOYEE_COLUMNS}"
    );
    Ok(sqlx::query_as(&sql)
        .bind(&id)
        .bind(&input.name)
        .bind(&input.purpose)
        .bind(owner)
        .bind(&input.autonomy)
        .bind(input.budget_limit)
        .bind(&input.allowed_tools)
        .bind(&input.data_scope)
        .fetch_one(pool)
        .await?)
}

/// Set an employee's `status` and return the updated row.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if `id` is unknown.
async fn set_employee_status(
    pool: &PgPool,
    id: &str,
    status: &str,
) -> Result<DigitalEmployee, StoreError> {
    let sql =
        format!("UPDATE agent_employee SET status = $2 WHERE id = $1 RETURNING {EMPLOYEE_COLUMNS}");
    let row: Option<DigitalEmployee> = sqlx::query_as(&sql)
        .bind(id)
        .bind(status)
        .fetch_optional(pool)
        .await?;
    row.ok_or(StoreError::NotFound)
}

/// `POST /api/agents/employees/{id}/suspend` — sets `status = "paused"`.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if `id` is unknown.
pub async fn suspend_employee(pool: &PgPool, id: &str) -> Result<DigitalEmployee, StoreError> {
    set_employee_status(pool, id, "paused").await
}

/// `POST /api/agents/employees/{id}/resume` — sets `status = "ready"`.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if `id` is unknown.
pub async fn resume_employee(pool: &PgPool, id: &str) -> Result<DigitalEmployee, StoreError> {
    set_employee_status(pool, id, "ready").await
}

/// `POST /api/agents/employees/{id}/revoke` — sets `status = "cancelled"`.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if `id` is unknown.
pub async fn revoke_employee(pool: &PgPool, id: &str) -> Result<DigitalEmployee, StoreError> {
    set_employee_status(pool, id, "cancelled").await
}

// ---------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------

/// Mirrors `AgentTool` in `contracts/agents.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AgentTool {
    /// `agent_tool.id`.
    pub id: String,
    /// Display name; the table's natural key.
    pub name: String,
    /// Semver-ish version label.
    pub version: String,
    /// Publishing team or vendor.
    pub publisher: String,
    /// Permission scope label (e.g. `"query:read"`).
    pub permission: String,
    /// `"healthy" | "degraded" | "unhealthy" | "unknown"`.
    pub health: String,
    /// `"pending" | "approved" | "rejected"` (`ApprovalStatus`).
    pub approval_status: String,
    /// Whether this tool version is deprecated.
    pub deprecated: bool,
    /// Rate limit label (e.g. `"60/min"`).
    pub rate_limit: String,
    /// Invocation count over the trailing 30 days.
    pub usage_30d: i64,
}

const TOOL_COLUMNS: &str = "id, name, version, publisher, permission, health, approval_status, \
     deprecated, rate_limit, usage_30d";

/// List every registered tool, newest first.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_tools(pool: &PgPool) -> Result<Vec<AgentTool>, StoreError> {
    let sql = format!("SELECT {TOOL_COLUMNS} FROM agent_tool ORDER BY created_at DESC");
    Ok(sqlx::query_as(&sql).fetch_all(pool).await?)
}

/// Everything [`register_tool`] needs. Mirrors `RegisterToolInput`.
#[derive(Debug, Clone)]
pub struct RegisterToolInput {
    /// Display name; must not collide with an existing tool.
    pub name: String,
    /// Semver-ish version label.
    pub version: String,
    /// Publishing team or vendor.
    pub publisher: String,
    /// Permission scope label (e.g. `"query:read"`).
    pub permission: String,
    /// Rate limit label (e.g. `"60/min"`).
    pub rate_limit: String,
}

/// Register a tool. `health` starts `"healthy"`, `approvalStatus` starts
/// `"pending"`, `usage30d` starts `0` — same as `mock/agents.ts`'s
/// `registerTool`.
///
/// # Errors
///
/// Returns [`StoreError::Conflict`] (409) if the name is taken.
pub async fn register_tool(
    pool: &PgPool,
    input: &RegisterToolInput,
) -> Result<AgentTool, StoreError> {
    let id = slug_id("tool", &input.name);
    let sql = format!(
        "INSERT INTO agent_tool (id, name, version, publisher, permission, health, \
         approval_status, deprecated, rate_limit) \
         VALUES ($1, $2, $3, $4, $5, 'healthy', 'pending', false, $6) \
         RETURNING {TOOL_COLUMNS}"
    );
    Ok(sqlx::query_as(&sql)
        .bind(&id)
        .bind(&input.name)
        .bind(&input.version)
        .bind(&input.publisher)
        .bind(&input.permission)
        .bind(&input.rate_limit)
        .fetch_one(pool)
        .await?)
}

// ---------------------------------------------------------------------
// Runs (history — see the module doc comment: never written by a live
// execution path)
// ---------------------------------------------------------------------

/// One step in an [`AgentRun`]'s recorded trace. Mirrors the anonymous
/// `steps[]` element shape of `AgentRun`.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStep {
    /// Step id, unique within the run.
    pub id: String,
    /// Human-readable step label.
    pub label: String,
    /// Lifecycle status (`EntityStatus`).
    pub status: String,
    /// Human-readable step detail.
    pub detail: String,
}

/// One approval reference embedded in an [`AgentRun`]. Mirrors the
/// anonymous `approvals[]` element shape of `AgentRun` — a projection of
/// [`ApprovalItem`], not a duplicate store.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunApprovalRef {
    /// The referenced [`ApprovalItem`]'s id.
    pub id: String,
    /// `"pending" | "approved" | "rejected"` (`ApprovalStatus`).
    pub status: String,
    /// When the approval was decided, ISO 8601.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

/// Mirrors `AgentRun` in `contracts/agents.ts`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRun {
    /// `agent_run.id`.
    pub id: String,
    /// The employee (agent) that performed this run.
    pub employee_id: String,
    /// The workflow this run belongs to, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    /// Lifecycle status (`EntityStatus`).
    pub status: String,
    /// Human-readable trigger description.
    pub trigger: String,
    /// The actor (usually the employee name) that initiated this run.
    pub actor: String,
    /// The human user this run was delegated by/to, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegated_user: Option<String>,
    /// When the run started, ISO 8601.
    pub started_at: String,
    /// When the run ended, ISO 8601, if it has.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    /// Budget units consumed so far.
    pub budget_consumed: f64,
    /// The recorded step trace.
    pub steps: Vec<RunStep>,
    /// Approvals requested in connection with this run.
    pub approvals: Vec<RunApprovalRef>,
    /// Audit log reference, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit_event_id: Option<String>,
}

#[derive(FromRow)]
struct RunRow {
    id: String,
    employee_id: String,
    workflow_id: Option<String>,
    status: String,
    trigger: String,
    actor: String,
    delegated_user: Option<String>,
    started_at: OffsetDateTime,
    ended_at: Option<OffsetDateTime>,
    budget_consumed: f64,
    steps: Json<Vec<RunStep>>,
    audit_event_id: Option<String>,
}

const RUN_COLUMNS: &str = "id, employee_id, workflow_id, status, trigger, actor, delegated_user, \
     started_at, ended_at, budget_consumed, steps, audit_event_id";

/// Fetch the `{id, status, at}` approval refs for one or more runs.
async fn approvals_for_run(pool: &PgPool, run_id: &str) -> Result<Vec<RunApprovalRef>, StoreError> {
    let rows: Vec<(String, String, Option<OffsetDateTime>)> = sqlx::query_as(
        "SELECT id, status, decided_at FROM approval_item WHERE run_id = $1 ORDER BY requested_at",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, status, at)| RunApprovalRef {
            id,
            status,
            at: iso_opt(at),
        })
        .collect())
}

async fn hydrate_run(pool: &PgPool, row: RunRow) -> Result<AgentRun, StoreError> {
    let approvals = approvals_for_run(pool, &row.id).await?;
    Ok(AgentRun {
        id: row.id,
        employee_id: row.employee_id,
        workflow_id: row.workflow_id,
        status: row.status,
        trigger: row.trigger,
        actor: row.actor,
        delegated_user: row.delegated_user,
        started_at: iso_millis(row.started_at),
        ended_at: iso_opt(row.ended_at),
        budget_consumed: row.budget_consumed,
        steps: row.steps.0,
        approvals,
        audit_event_id: row.audit_event_id,
    })
}

/// List runs, newest first, optionally filtered to one employee.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if a query fails.
pub async fn list_runs(
    pool: &PgPool,
    employee_id: Option<&str>,
) -> Result<Vec<AgentRun>, StoreError> {
    let sql = format!(
        "SELECT {RUN_COLUMNS} FROM agent_run WHERE ($1::text IS NULL OR employee_id = $1) \
         ORDER BY started_at DESC"
    );
    let rows: Vec<RunRow> = sqlx::query_as(&sql)
        .bind(employee_id)
        .fetch_all(pool)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(hydrate_run(pool, row).await?);
    }
    Ok(out)
}

/// Fetch one run by id.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if a query fails.
pub async fn get_run(pool: &PgPool, id: &str) -> Result<Option<AgentRun>, StoreError> {
    let sql = format!("SELECT {RUN_COLUMNS} FROM agent_run WHERE id = $1");
    let row: Option<RunRow> = sqlx::query_as(&sql).bind(id).fetch_optional(pool).await?;
    match row {
        Some(row) => Ok(Some(hydrate_run(pool, row).await?)),
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------
// Approvals
// ---------------------------------------------------------------------

/// Mirrors `ApprovalItem` in `contracts/agents.ts`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalItem {
    /// `approval_item.id`.
    pub id: String,
    /// The employee (agent) this approval was requested for.
    pub employee_id: String,
    /// Denormalized employee display name, at request time.
    pub employee_name: String,
    /// The run this approval is connected to, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// The workflow this approval is connected to, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    /// Human-readable description of the action awaiting approval.
    pub action: String,
    /// The resource the action targets, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    /// Why the action was proposed, if given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Human-readable impact statement, if given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impact: Option<String>,
    /// Supporting evidence lines, if given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<String>>,
    /// The governing policy label, if given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    /// Estimated cost of the action, if given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_estimate: Option<f64>,
    /// When this approval expires, ISO 8601, if it does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// When this approval was requested, ISO 8601.
    pub requested_at: String,
    /// `"pending" | "approved" | "rejected"` (`ApprovalStatus`).
    pub status: String,
    /// Human-readable risk statement.
    pub risk: String,
    /// When this approval was decided, ISO 8601, if it has been.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<String>,
    /// The deciding reviewer's comment, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    /// Audit log reference, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit_event_id: Option<String>,
}

#[derive(FromRow)]
struct ApprovalRow {
    id: String,
    employee_id: String,
    employee_name: String,
    run_id: Option<String>,
    workflow_id: Option<String>,
    action: String,
    resource: Option<String>,
    reason: Option<String>,
    impact: Option<String>,
    evidence: Option<Vec<String>>,
    policy: Option<String>,
    cost_estimate: Option<f64>,
    expires_at: Option<OffsetDateTime>,
    requested_at: OffsetDateTime,
    status: String,
    risk: String,
    decided_at: Option<OffsetDateTime>,
    comment: Option<String>,
    audit_event_id: Option<String>,
}

impl From<ApprovalRow> for ApprovalItem {
    fn from(row: ApprovalRow) -> Self {
        Self {
            id: row.id,
            employee_id: row.employee_id,
            employee_name: row.employee_name,
            run_id: row.run_id,
            workflow_id: row.workflow_id,
            action: row.action,
            resource: row.resource,
            reason: row.reason,
            impact: row.impact,
            evidence: row.evidence,
            policy: row.policy,
            cost_estimate: row.cost_estimate,
            expires_at: iso_opt(row.expires_at),
            requested_at: iso_millis(row.requested_at),
            status: row.status,
            risk: row.risk,
            decided_at: iso_opt(row.decided_at),
            comment: row.comment,
            audit_event_id: row.audit_event_id,
        }
    }
}

const APPROVAL_COLUMNS: &str = "id, employee_id, employee_name, run_id, workflow_id, action, \
     resource, reason, impact, evidence, policy, cost_estimate, expires_at, requested_at, \
     status, risk, decided_at, comment, audit_event_id";

/// List approvals, newest-requested first, optionally filtered to one
/// employee.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_approvals(
    pool: &PgPool,
    employee_id: Option<&str>,
) -> Result<Vec<ApprovalItem>, StoreError> {
    let sql = format!(
        "SELECT {APPROVAL_COLUMNS} FROM approval_item WHERE ($1::text IS NULL OR employee_id = \
         $1) ORDER BY requested_at DESC"
    );
    let rows: Vec<ApprovalRow> = sqlx::query_as(&sql)
        .bind(employee_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(ApprovalItem::from).collect())
}

/// A decision made on a pending [`ApprovalItem`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The action is approved.
    Approved,
    /// The action is rejected.
    Rejected,
}

impl Decision {
    fn as_status(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }
}

/// Decide a pending approval. Mirrors `mock/agents.ts`'s `decideApproval`:
/// only a `"pending"` approval can be decided.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if `id` is unknown. Returns
/// [`StoreError::Conflict`] (409) if the approval has already been decided
/// (`status != "pending"`) — an already-approved or already-rejected item
/// cannot be re-decided, matching the mock's `ServiceError("invalid_request",
/// ...)` guard (surfaced here as a 409 rather than the mock's 400, because
/// this is a state-conflict, not a malformed request — see
/// `routes::agents::decide_approval`'s doc comment for the status-code
/// rationale).
pub async fn decide_approval(
    pool: &PgPool,
    id: &str,
    decision: Decision,
    comment: Option<&str>,
) -> Result<ApprovalItem, StoreError> {
    let mut tx = pool.begin().await?;
    let current: Option<(String,)> =
        sqlx::query_as("SELECT status FROM approval_item WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((status,)) = current else {
        return Err(StoreError::NotFound);
    };
    if status != "pending" {
        return Err(StoreError::Conflict);
    }
    let sql = format!(
        "UPDATE approval_item SET status = $2, decided_at = now(), comment = $3, \
         audit_event_id = $4 WHERE id = $1 RETURNING {APPROVAL_COLUMNS}"
    );
    let audit_event_id = format!("aud-approval-{id}-{}", decision.as_status());
    let row: ApprovalRow = sqlx::query_as(&sql)
        .bind(id)
        .bind(decision.as_status())
        .bind(comment)
        .bind(&audit_event_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(row.into())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn slug_id_uses_prefix_and_lowercases() {
        let id = slug_id("emp", "Inventory Copilot!!");
        assert!(id.starts_with("emp-inventory-copilot-"));
    }

    #[test]
    fn radix36_matches_js_to_string_36() {
        assert_eq!(radix36(0), "0");
        assert_eq!(radix36(35), "z");
        assert_eq!(radix36(36), "10");
    }

    #[test]
    fn decision_maps_to_lowercase_status() {
        assert_eq!(Decision::Approved.as_status(), "approved");
        assert_eq!(Decision::Rejected.as_status(), "rejected");
    }
}
