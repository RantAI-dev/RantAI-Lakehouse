//! Repository layer for the `agents` domain: digital employee definitions,
//! tools, workflows, run history, and the approval lifecycle. Postgres
//! backing for `src/services/mock/agents.ts`.
//!
//! # T3.2 update: a real (headless) execution runtime now exists
//!
//! Earlier revisions of this module doc comment said there was no agent
//! runtime anywhere in this repository, and that `agent_run` was "never
//! written to by a live execution path, only seeded / read". That stopped
//! being true with T0.5 (the copilot's `WriteHigh` approval flow, see
//! [`create_pending_approval`]/[`record_run_outcome`]) and, with T3.2
//! (`POST /api/agents/employees/{id}/run`, `routes::agents::run_employee`),
//! a digital employee's `agent_run` rows now come from an actual headless
//! run of the copilot's tool-calling loop — not just interactive chat's
//! `WriteHigh` calls. [`create_run`]/[`append_run_step`]/
//! [`mark_run_waiting_approval`]/[`create_linked_approval`] are that
//! path's store surface: they create a run up front (`"running"`),
//! append one step per tool call as the loop goes, and end it at a
//! terminal status (`"succeeded"`/`"failed"`) or `"waiting_approval"` when
//! a `WriteHigh` tool call inside the run needs a human decision — see
//! `routes::agents::run_employee`'s own doc comment (`lakehouse-api`) for
//! the full flow.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use sqlx::types::Json;
use time::OffsetDateTime;
use uuid::Uuid;

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
    /// Budget ceiling, in tokens (WS7 item G2's unit decision).
    pub budget_limit: f64,
    /// Real, recomputed `SUM(agent_run.budget_consumed)` over every run for
    /// this employee (WS7 item G3) — `None` only for [`list_employees`]/
    /// [`get_employee`]'s lighter projection, which does not fetch this
    /// column at all (see [`get_employee_with_metrics`], which does).
    pub budget_spent: Option<f64>,
    /// No multi-step reservation/hold concept exists anywhere in this
    /// codebase (confirmed while writing `0039_agent_metrics_recompute.sql`
    /// — nothing ever reserves a budget ahead of a run), so this stays
    /// `#[sqlx(skip)]` `None` permanently, unlike its four siblings below
    /// (WS1 task 1.11's original placeholder collapse, now narrowed to
    /// just this one column).
    #[sqlx(skip)]
    pub budget_reserved: Option<f64>,
    /// Tool names this employee may invoke.
    pub allowed_tools: Vec<String>,
    /// Human-readable data-access scope.
    pub data_scope: String,
    /// Real, recomputed approval rate over the employee's `approval_item`
    /// rows in the last 30 days (WS7 item G3) — see
    /// [`budget_spent`](Self::budget_spent)'s doc comment for the
    /// list-vs-detail projection split.
    pub approval_rate: Option<f64>,
    /// Real, recomputed success rate over the employee's `agent_run` rows
    /// in the last 30 days (WS7 item G3) — see
    /// [`budget_spent`](Self::budget_spent)'s doc comment for the
    /// list-vs-detail projection split.
    pub success_rate: Option<f64>,
    /// Real, recomputed count of `agent_run` rows in the last 30 days
    /// (WS7 item G3) — see [`budget_spent`](Self::budget_spent)'s doc
    /// comment for the list-vs-detail projection split.
    pub recent_runs: Option<i64>,
    /// The instruction a headless run sends to the copilot. `None` means
    /// this employee is not runnable.
    pub prompt: Option<String>,
    /// Cron expression for a Dagster schedule, or `None` for manual-only.
    pub schedule_cron: Option<String>,
    /// The copilot mode a run uses: `"ask"` or `"build"`.
    pub mode: String,
    /// Ceiling on what this employee's runs may do, as a comma-separated
    /// `resource:action` list in the same format as `role.permissions`
    /// (see `lakehouse_auth::permissions::PermissionSet::parse`). Empty
    /// means authenticated-only, no permissioned tools.
    pub permissions: String,
}

// WS7 item G3: budget_spent/approval_rate/success_rate/recent_runs are
// real, recomputed columns now (see `recompute_employee_metrics` in
// `0039_agent_metrics_recompute.sql`, called at every run's terminal
// transition) — but they stay OUT of this narrower SELECT list on
// purpose: a list of many employees should not pay for four
// already-materialized-but-still-extra column reads each, and the
// LIST route (`list_employees`) has never rendered them (confirmed by
// reading `src/features/agents/employees-page.tsx`'s list rendering).
// `get_employee_with_metrics` (below) is the FULL projection, used by the
// single-row DETAIL route where the cost is free.
const EMPLOYEE_COLUMNS: &str = "id, name, purpose, owner, autonomy, status, budget_limit, \
     allowed_tools, data_scope, prompt, schedule_cron, mode, permissions";

/// The narrow [`EMPLOYEE_COLUMNS`] projection, row-for-row — kept as its
/// own `FromRow` type (rather than reusing [`DigitalEmployee`] directly)
/// because `DigitalEmployee` now has four real metric columns
/// (`budget_spent`/`approval_rate`/`success_rate`/`recent_runs`, WS7 item
/// G3) this narrower query never selects; mapping through this type makes
/// that omission explicit at every call site via
/// [`EmployeeListRow::without_metrics`] rather than a silent
/// column-not-found panic at runtime.
#[derive(FromRow)]
struct EmployeeListRow {
    id: String,
    name: String,
    purpose: String,
    owner: String,
    autonomy: String,
    status: String,
    budget_limit: f64,
    allowed_tools: Vec<String>,
    data_scope: String,
    prompt: Option<String>,
    schedule_cron: Option<String>,
    mode: String,
    permissions: String,
}

impl EmployeeListRow {
    /// Lifts a narrow row into [`DigitalEmployee`], with every metric
    /// field explicitly `None` — "not fetched for a list view", a real,
    /// disclosed distinction from "no run has completed yet"
    /// ([`get_employee_with_metrics`]'s own `None`), though both render
    /// as `—` client-side (matching `Measured`'s existing collapse of
    /// "unmeasured" and "not fetched" into one display state).
    fn without_metrics(self) -> DigitalEmployee {
        DigitalEmployee {
            id: self.id,
            name: self.name,
            purpose: self.purpose,
            owner: self.owner,
            autonomy: self.autonomy,
            status: self.status,
            budget_limit: self.budget_limit,
            budget_spent: None,
            budget_reserved: None,
            allowed_tools: self.allowed_tools,
            data_scope: self.data_scope,
            approval_rate: None,
            success_rate: None,
            recent_runs: None,
            prompt: self.prompt,
            schedule_cron: self.schedule_cron,
            mode: self.mode,
            permissions: self.permissions,
        }
    }
}

/// [`EMPLOYEE_COLUMNS`] plus the four real metric columns WS7 item G3
/// recomputes — backs [`get_employee_with_metrics`] only.
const FULL_EMPLOYEE_COLUMNS: &str = "id, name, purpose, owner, autonomy, status, budget_limit, allowed_tools, data_scope, \
     prompt, schedule_cron, mode, permissions, budget_spent, approval_rate, success_rate, \
     recent_runs";

/// List every digital employee, newest first. Metric columns
/// (`budgetSpent`/`successRate`/`approvalRate`/`recentRuns`) are `None` —
/// see [`EMPLOYEE_COLUMNS`]'s doc comment; use [`get_employee_with_metrics`]
/// for a single employee's real values.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_employees(pool: &PgPool) -> Result<Vec<DigitalEmployee>, StoreError> {
    let sql = format!("SELECT {EMPLOYEE_COLUMNS} FROM agent_employee ORDER BY created_at DESC");
    let rows: Vec<EmployeeListRow> = sqlx::query_as(&sql).fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(EmployeeListRow::without_metrics)
        .collect())
}

/// Fetch one employee by id. Metric columns are `None` — see
/// [`EMPLOYEE_COLUMNS`]'s doc comment; use [`get_employee_with_metrics`]
/// for the real values.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get_employee(pool: &PgPool, id: &str) -> Result<Option<DigitalEmployee>, StoreError> {
    let sql = format!("SELECT {EMPLOYEE_COLUMNS} FROM agent_employee WHERE id = $1");
    let row: Option<EmployeeListRow> = sqlx::query_as(&sql).bind(id).fetch_optional(pool).await?;
    Ok(row.map(EmployeeListRow::without_metrics))
}

/// Fetch one employee by id WITH its real, recomputed metrics
/// (`budgetSpent`/`successRate`/`approvalRate`/`recentRuns`, WS7 item G3)
/// — backs the `GET /api/agents/employees/{id}` DETAIL route, where
/// paying for four already-materialized columns on one row is free
/// (unlike [`list_employees`]'s narrower projection).
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get_employee_with_metrics(
    pool: &PgPool,
    id: &str,
) -> Result<Option<DigitalEmployee>, StoreError> {
    let sql = format!("SELECT {FULL_EMPLOYEE_COLUMNS} FROM agent_employee WHERE id = $1");
    Ok(sqlx::query_as(&sql).bind(id).fetch_optional(pool).await?)
}

/// List every digital employee that has a `schedule_cron` set, newest
/// first. This is exactly the set T3.3's Dagster schedule factory needs
/// to build one `ScheduleDefinition` per employee — a `NULL` cron means
/// manual-only and is excluded. Metric columns are `None`, same as
/// [`list_employees`]: this list is schedule-factory input, not a
/// console view.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_scheduled_employees(pool: &PgPool) -> Result<Vec<DigitalEmployee>, StoreError> {
    let sql = format!(
        "SELECT {EMPLOYEE_COLUMNS} FROM agent_employee WHERE schedule_cron IS NOT NULL \
         ORDER BY created_at DESC"
    );
    let rows: Vec<EmployeeListRow> = sqlx::query_as(&sql).fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(EmployeeListRow::without_metrics)
        .collect())
}

/// The subset of an employee's columns a headless run needs: what to send
/// the copilot, which mode to run it in, the permission ceiling to scope
/// its synthetic principal to, and whether the employee is even eligible
/// to run right now (`status`; T3.2 must refuse a run for a suspended
/// (`"paused"`) or revoked (`"cancelled"`) employee).
// `Eq` dropped (not `PartialEq, Eq`): `budget_limit: f64` cannot implement
// `Eq` (no `Copy`+total-order guarantee for floats — NaN != NaN), so this
// struct is `PartialEq`-only now that WS7 item G2 adds that field.
#[derive(Debug, Clone, PartialEq, FromRow)]
pub struct EmployeeRunConfig {
    /// `agent_employee.id`.
    pub id: String,
    /// Display name — used as `agent_run.actor` for a token-triggered
    /// (`trigger = "schedule"`) run, and as `approval_item.employee_name`
    /// for a `WriteHigh` call the run's loop hits.
    pub name: String,
    /// Lifecycle status (`EntityStatus`); a run route must refuse anything
    /// other than a runnable status.
    pub status: String,
    /// The instruction a headless run sends to the copilot. `None` means
    /// this employee is not runnable.
    pub prompt: Option<String>,
    /// The copilot mode a run uses: `"ask"` or `"build"`.
    pub mode: String,
    /// Ceiling on what this employee's runs may do, in `role.permissions`
    /// format.
    pub permissions: String,
    /// Token ceiling for a headless run of this employee (WS7 item G2:
    /// `budget_limit`/`budget_consumed` are measured in tokens, not
    /// currency — `lakehouse-llm` has no price table, see
    /// `run_headless_loop`'s own doc comment on the unit decision).
    pub budget_limit: f64,
}

/// Fetch just the run-configuration columns for one employee — `prompt`,
/// `mode`, `permissions`, `status`, `budget_limit` — without the rest of
/// [`DigitalEmployee`].
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get_employee_run_config(
    pool: &PgPool,
    id: &str,
) -> Result<Option<EmployeeRunConfig>, StoreError> {
    let row: Option<EmployeeRunConfig> = sqlx::query_as(
        "SELECT id, name, status, prompt, mode, permissions, budget_limit \
         FROM agent_employee WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
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
    /// The instruction a headless run sends to the copilot. `None` means
    /// this employee is not runnable.
    pub prompt: Option<String>,
    /// Cron expression for a Dagster schedule, or `None` for manual-only.
    pub schedule_cron: Option<String>,
    /// The copilot mode a run uses (`"ask"` or `"build"`); defaults to
    /// `"build"` when absent.
    pub mode: Option<String>,
    /// Ceiling on what this employee's runs may do, in `role.permissions`
    /// format; defaults to `""` (authenticated-only) when absent.
    pub permissions: Option<String>,
}

const DEFAULT_MODE: &str = "build";

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
    let mode = input.mode.as_deref().unwrap_or(DEFAULT_MODE);
    let permissions = input.permissions.as_deref().unwrap_or("");
    let sql = format!(
        "INSERT INTO agent_employee (id, name, purpose, owner, autonomy, status, budget_limit, \
         allowed_tools, data_scope, prompt, schedule_cron, mode, permissions) \
         VALUES ($1, $2, $3, $4, $5, 'draft', $6, $7, $8, $9, $10, $11, $12) \
         RETURNING {EMPLOYEE_COLUMNS}"
    );
    let row: EmployeeListRow = sqlx::query_as(&sql)
        .bind(&id)
        .bind(&input.name)
        .bind(&input.purpose)
        .bind(owner)
        .bind(&input.autonomy)
        .bind(input.budget_limit)
        .bind(&input.allowed_tools)
        .bind(&input.data_scope)
        .bind(&input.prompt)
        .bind(&input.schedule_cron)
        .bind(mode)
        .bind(permissions)
        .fetch_one(pool)
        .await?;
    Ok(row.without_metrics())
}

/// Set an employee's `status` and return the updated row. Metric columns
/// on the returned [`DigitalEmployee`] are `None` — a status-only mutation
/// has no reason to also pay for the four metric columns (matching
/// [`get_employee`]'s own narrower projection); callers displaying an
/// updated row alongside real metrics should re-fetch via
/// [`get_employee_with_metrics`].
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
    let row: Option<EmployeeListRow> = sqlx::query_as(&sql)
        .bind(id)
        .bind(status)
        .fetch_optional(pool)
        .await?;
    row.map(EmployeeListRow::without_metrics)
        .ok_or(StoreError::NotFound)
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
//
// WS7 item G4: the `agent_tool` Postgres table (and its
// `AgentTool`/`list_tools`/`RegisterToolInput`/`register_tool` Rust
// surface, all removed here) was never the real tool registry — nothing
// in `routes::ai`'s dispatch (`ai_tools::run_tool`,
// `ai_registry::TOOLS`) ever read from it, so "registering" a tool here
// never made it callable. `GET /api/agents/tools` now reflects
// `ai_registry::TOOLS` directly (`routes::agents::list_tools_body`); this
// function is what backs its real 30-day usage counts, from the SAME
// `audit_event` rows `ai::audit::record` already writes unconditionally
// on every tool dispatch. The `agent_tool` table and its `0017`/`0018`
// seed rows stay in the schema (a migration is never edited or dropped
// once applied) but nothing in this crate reads them anymore.

/// `(action, count)` for every distinct `audit_event.action` dispatched
/// in the last 30 days — the real usage counts
/// `routes::agents::list_tools_body` looks each registry tool's name up
/// in.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn tool_usage_counts_30d(pool: &PgPool) -> Result<Vec<(String, i64)>, StoreError> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT action, COUNT(*) FROM audit_event WHERE at > now() - interval '30 days' \
         GROUP BY action",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
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
    /// Real token spend for this run, accumulated from every
    /// [`lakehouse_llm::Usage::total_tokens`] the LLM endpoint reported
    /// during the run (WS7 item G2), written once at every terminal
    /// transition ([`record_run_budget`]). `agent_run.budget_consumed` is
    /// `NOT NULL DEFAULT 0` at the schema level — a run that has not yet
    /// reached a terminal status (`status` is `"running"` or
    /// `"waiting_approval"`) has never had that default overwritten, so
    /// its DB value of `0` would be an unstarted placeholder, not a
    /// measurement; `hydrate_run` collapses that specific case to `None`
    /// rather than re-serving a fabricated zero (WS1 task 1.11's same
    /// concern, now applied only to the still-in-progress window).
    pub budget_consumed: Option<f64>,
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

// WS7 item G2: budget_consumed is back in this SELECT list —
// `record_run_budget` now writes a real accumulated-token-usage value at
// every terminal transition of `run_headless_loop`, so selecting it once
// more serves a real measurement, not the insert-time default WS1 task
// 1.11 stopped serving.
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
    let still_in_progress = matches!(row.status.as_str(), "running" | "waiting_approval");
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
        budget_consumed: if still_in_progress {
            None
        } else {
            Some(row.budget_consumed)
        },
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
    /// The newest real `audit_event` row recorded against this approval
    /// (`resource_kind = 'approval'`, `resource_id = id`), resolved on read
    /// via a lateral join in [`list_approvals`]/[`decide_approval`] — never
    /// stored. `routes::agents::decide_approval` writes the matching event
    /// via `ai_audit::record` *after* the store call returns, so a
    /// `decide_approval` return value never carries this id even on
    /// success; a later `list_approvals` call does (see
    /// [`decide_approval`]'s doc comment).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit_event_id: Option<String>,
    /// `"tool_call" | "access"` (WS7 item E1, `0040_access_requests.sql`).
    /// Every approval inserted before that migration backfilled to
    /// `"tool_call"`; an access request (WS7 item E2) is always
    /// `"access"`. `routes::agents::decide_approval` and
    /// `routes::catalog::decide_access_request` (WS7 item E3) each refuse
    /// the OTHER kind with a 404 — see that pair's own doc comments.
    pub kind: String,
    /// The human who requested this approval — set for `kind = "access"`
    /// (WS7 item E2's `create_access_request`), `None` for `kind =
    /// "tool_call"` (a digital employee acted; no single human requester
    /// exists). Used by `decide_access_request` (WS7 item E3) to refuse a
    /// principal approving their own request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_by_user_id: Option<Uuid>,
}

#[derive(FromRow)]
struct ApprovalRow {
    id: String,
    employee_id: Option<String>,
    employee_name: Option<String>,
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
    kind: String,
    requested_by_user_id: Option<Uuid>,
}

impl From<ApprovalRow> for ApprovalItem {
    fn from(row: ApprovalRow) -> Self {
        Self {
            id: row.id,
            // WS7 item E1 widened `employee_id`/`employee_name` to
            // nullable (an access request names no digital employee at
            // all) — `ApprovalItem`'s own fields stay non-`Option`
            // (every EXISTING reader assumes a tool-call approval, which
            // still always sets both) and default to `String::new()` for
            // the `kind = "access"` shape this `From` impl also now
            // handles, rather than widening every call site's field type
            // for a case they never read.
            employee_id: row.employee_id.unwrap_or_default(),
            employee_name: row.employee_name.unwrap_or_default(),
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
            kind: row.kind,
            requested_by_user_id: row.requested_by_user_id,
        }
    }
}

/// Columns qualified against the `a` alias `list_approvals`/`decide_approval`
/// give `approval_item`, plus `ae.id AS audit_event_id` resolved from the
/// [`APPROVAL_AUDIT_JOIN`] lateral join rather than read from
/// `approval_item.audit_event_id` (that column stays in the table — see
/// `0022_approvals.sql`/migration history — but nothing writes it anymore;
/// see [`ApprovalItem::audit_event_id`]'s doc comment). Qualification is
/// required, not stylistic: `id` and the other column names exist on both
/// `approval_item` and (for `id`) `audit_event`, so an unqualified list
/// would be ambiguous once the lateral join is in the `FROM` clause.
const APPROVAL_COLUMNS_QUALIFIED: &str = "a.id, a.employee_id, a.employee_name, a.run_id, \
     a.workflow_id, a.action, a.resource, a.reason, a.impact, a.evidence, a.policy, \
     a.cost_estimate, a.expires_at, a.requested_at, a.status, a.risk, a.decided_at, a.comment, \
     ae.id AS audit_event_id, a.kind, a.requested_by_user_id";

/// The lateral join that resolves an approval's real audit-event id: the
/// newest `audit_event` row with `resource_kind = 'approval'` and
/// `resource_id` equal to the approval's id. A plain `LEFT JOIN` would
/// duplicate the approval row once per matching event (several audit
/// events can exist for one approval, e.g. `needs_approval` then
/// `approved`); the lateral subquery's `ORDER BY ... LIMIT 1` keeps one row
/// per approval. The composite index `audit_event_resource_idx
/// (resource_kind, resource_id)` (`0024_audit_event.sql`) covers this
/// lookup.
const APPROVAL_AUDIT_JOIN: &str = "FROM approval_item a \
     LEFT JOIN LATERAL ( \
         SELECT ae.id FROM audit_event ae \
          WHERE ae.resource_kind = 'approval' AND ae.resource_id = a.id \
          ORDER BY ae.at DESC LIMIT 1 \
     ) ae ON true";

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
        "SELECT {APPROVAL_COLUMNS_QUALIFIED} {APPROVAL_AUDIT_JOIN} WHERE ($1::text IS NULL OR \
         a.employee_id = $1) ORDER BY a.requested_at DESC"
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
/// # `auditEventId` is always `None` on return (WS1 task 1.14, judge
/// finding J12)
///
/// This used to stamp `aud-approval-<id>-<status>` into
/// `approval_item.audit_event_id` on every decision — a string that named
/// no `audit_event` row (the real event `routes::agents::decide_approval`
/// writes via `ai_audit::record` gets its own generated id), so "View
/// audit" always 404ed. It no longer writes that column at all: after the
/// `UPDATE`, this re-reads the row inside the same transaction through
/// [`APPROVAL_COLUMNS_QUALIFIED`]/[`APPROVAL_AUDIT_JOIN`], the same
/// resolve-on-read lateral lookup `list_approvals` uses. Because
/// `routes::agents::decide_approval` calls `ai_audit::record` *after* this
/// function returns, the matching `audit_event` row does not exist yet at
/// re-read time — so the `ApprovalItem` this returns always carries
/// `audit_event_id: None`, even for a successful decision. That is honest:
/// a later `list_approvals` call (after the audit write lands) resolves the
/// real id. This task does not reorder the audit write into the store —
/// that is WS5's to own.
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
    sqlx::query(
        "UPDATE approval_item SET status = $2, decided_at = now(), comment = $3 WHERE id = $1",
    )
    .bind(id)
    .bind(decision.as_status())
    .bind(comment)
    .execute(&mut *tx)
    .await?;
    let select_sql =
        format!("SELECT {APPROVAL_COLUMNS_QUALIFIED} {APPROVAL_AUDIT_JOIN} WHERE a.id = $1");
    let row: ApprovalRow = sqlx::query_as(&select_sql)
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(row.into())
}

// ---------------------------------------------------------------------
// WriteHigh copilot approvals (T0.5, copilot-operations-handover plan)
// ---------------------------------------------------------------------

/// The reserved interactive-copilot employee row `0025_agent_schedule.sql`
/// inserts — every `WriteHigh` copilot call is attributed to this id.
pub const COPILOT_EMPLOYEE_ID: &str = "emp-copilot";

/// [`COPILOT_EMPLOYEE_ID`]'s display name, denormalized into
/// `approval_item.employee_name` at request time (matching every other
/// approval's `employee_name`, which is never re-derived from
/// `agent_employee` after the fact).
pub const COPILOT_EMPLOYEE_NAME: &str = "Copilot (interactive)";

/// Everything [`create_pending_approval`] needs to create the linked
/// `agent_run` (`waiting_approval`) + `approval_item` (`pending`) pair for
/// one `WriteHigh` copilot tool call.
#[derive(Debug, Clone)]
pub struct NewApprovalRequest<'a> {
    /// The tool name — becomes `approval_item.action`, and is looked back
    /// up in the tool registry when the approval is later decided.
    pub tool: &'a str,
    /// Display label for whoever/whatever triggered this call (e.g. the
    /// chat principal's display name).
    pub actor: &'a str,
    /// The target resource id, if knowable ahead of execution (see
    /// `routes::ai::audit::resource_for`).
    pub resource: Option<&'a str>,
    /// A deterministic, human-readable reason sentence (never LLM-authored
    /// — see `routes::ai::gate`).
    pub reason: &'a str,
    /// A human-readable risk statement.
    pub risk: &'a str,
    /// The tool call's REDACTED arguments (plan invariant 5 — the caller
    /// must have already run these through `routes::ai::audit::redact`).
    /// Stored twice: once as `approval_item.evidence` (as a single JSON
    /// string element, since the column is `TEXT[]`), and once inside the
    /// linked run's first step, so [`pending_tool_call`] can replay the
    /// exact call later.
    pub redacted_args: &'a Value,
}

/// The ids [`create_pending_approval`] created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedApproval {
    /// The new `agent_run.id` (status `waiting_approval`).
    pub run_id: String,
    /// The new `approval_item.id` (status `pending`).
    pub approval_id: String,
}

/// Creates the linked `agent_run` + `approval_item` pair for a `WriteHigh`
/// copilot tool call that has already passed the ask-mode and permission
/// checks (`routes::ai::gate::decide`) — nothing executes here, and
/// nothing executes until a later `POST /api/agents/approvals/{id}/decide`
/// approves it (see `routes::agents::decide_approval`).
///
/// Both rows are created in one transaction: either both exist or
/// neither does, so `approval_item.run_id` never dangles.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if either insert fails (e.g. the
/// `emp-copilot` row is missing — it should never be, see
/// `0025_agent_schedule.sql`).
pub async fn create_pending_approval(
    pool: &PgPool,
    req: NewApprovalRequest<'_>,
) -> Result<CreatedApproval, StoreError> {
    let mut tx = pool.begin().await?;

    let run_id = format!("run-appr-{}", Uuid::new_v4());
    let pending = serde_json::json!({ "tool": req.tool, "args": req.redacted_args });
    let step = RunStep {
        id: "step-1".to_owned(),
        label: format!("Menunggu persetujuan: {}", req.tool),
        status: "pending".to_owned(),
        detail: serde_json::to_string(&pending).unwrap_or_default(),
    };
    sqlx::query(
        "INSERT INTO agent_run (id, employee_id, status, trigger, actor, steps) \
         VALUES ($1, $2, 'waiting_approval', 'copilot', $3, $4)",
    )
    .bind(&run_id)
    .bind(COPILOT_EMPLOYEE_ID)
    .bind(req.actor)
    .bind(Json(vec![step]))
    .execute(&mut *tx)
    .await?;

    let approval_id = format!("appr-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO approval_item \
         (id, employee_id, employee_name, run_id, action, resource, reason, evidence, status, risk) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending', $9)",
    )
    .bind(&approval_id)
    .bind(COPILOT_EMPLOYEE_ID)
    .bind(COPILOT_EMPLOYEE_NAME)
    .bind(&run_id)
    .bind(req.tool)
    .bind(req.resource)
    .bind(req.reason)
    .bind(vec![serde_json::to_string(req.redacted_args).unwrap_or_default()])
    .bind(req.risk)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(CreatedApproval {
        run_id,
        approval_id,
    })
}

/// The tool call [`create_pending_approval`] recorded into a
/// `waiting_approval` run's first step, recovered so
/// `routes::agents::decide_approval` can replay it EXACTLY on approval —
/// never re-derived from the model or the approver's own input.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PendingToolCall {
    /// The tool name to execute.
    pub tool: String,
    /// The (already redacted at request time — see [`NewApprovalRequest`])
    /// arguments to execute it with.
    pub args: Value,
}

/// Recovers the [`PendingToolCall`] stored in a `waiting_approval` run's
/// LAST step's `detail`. `None` if the run has no steps, or its last
/// step's `detail` isn't the JSON shape this module writes (e.g. a run
/// this module never created).
///
/// The LAST step, not the first: [`create_pending_approval`] (the
/// interactive-copilot path) always creates a run with exactly one step,
/// so first/last are the same row there — but a headless run (T3.2,
/// `routes::agents::run_employee`) already has zero or more earlier steps
/// (every tool call executed before the `WriteHigh` one that paused it)
/// by the time it appends its own pending-call step and stops; that
/// pending call is always the most recent step, never the first.
#[must_use]
pub fn pending_tool_call(run: &AgentRun) -> Option<PendingToolCall> {
    let last = run.steps.last()?;
    serde_json::from_str(&last.detail).ok()
}

/// Appends one outcome step to a run's `steps` and sets its terminal
/// `status`/`ended_at` — used for every way a `WriteHigh` approval's story
/// can end: rejected, approved-but-not-executed (approver lacks the
/// tool's own permission), or executed (`succeeded`/`failed`).
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the run id does not exist or the
/// update fails.
pub async fn record_run_outcome(
    pool: &PgPool,
    run_id: &str,
    status: &str,
    label: &str,
    detail: &str,
) -> Result<(), StoreError> {
    let step = RunStep {
        id: "step-2".to_owned(),
        label: label.to_owned(),
        status: status.to_owned(),
        detail: detail.to_owned(),
    };
    let result = sqlx::query(
        "UPDATE agent_run SET status = $2, ended_at = now(), steps = steps || $3::jsonb \
         WHERE id = $1",
    )
    .bind(run_id)
    .bind(status)
    .bind(Json(vec![step]))
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Headless employee runs (T3.2, copilot-operations-handover plan)
// ---------------------------------------------------------------------

/// Creates the `agent_run` row a headless `POST
/// /api/agents/employees/{id}/run` starts with: `status = "running"`,
/// `steps = []`, `id` caller-supplied so the route can keep referring to
/// it (matches the `run-appr-<uuid>` id shape [`create_pending_approval`]
/// already uses for the interactive-copilot equivalent, but this one is
/// generated by the caller rather than returned — see
/// `routes::agents::run_employee`).
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the insert fails (e.g. `employee_id`
/// does not reference an existing `agent_employee` row).
pub async fn create_run(
    pool: &PgPool,
    id: &str,
    employee_id: &str,
    trigger: &str,
    actor: &str,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO agent_run (id, employee_id, status, trigger, actor, steps) \
         VALUES ($1, $2, 'running', $3, $4, '[]'::jsonb)",
    )
    .bind(id)
    .bind(employee_id)
    .bind(trigger)
    .bind(actor)
    .execute(pool)
    .await?;
    Ok(())
}

/// Appends one step to a run's `steps` WITHOUT touching `status` or
/// `ended_at` — for a headless run's tool-call trace, recorded as the loop
/// goes rather than only at the end (unlike [`record_run_outcome`], which
/// always sets a terminal status).
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if `run_id` does not exist, or
/// [`StoreError::Database`] on any other failure.
pub async fn append_run_step(pool: &PgPool, run_id: &str, step: RunStep) -> Result<(), StoreError> {
    let result = sqlx::query("UPDATE agent_run SET steps = steps || $2::jsonb WHERE id = $1")
        .bind(run_id)
        .bind(Json(vec![step]))
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

/// Sets a run's terminal `status` (`"succeeded"` | `"failed"`) and
/// `ended_at`, without appending a step — used when the headless loop's
/// own steps already recorded everything worth showing (via
/// [`append_run_step`]) and only the final status/timestamp remain to be
/// set. Kept distinct from [`record_run_outcome`] (which always appends
/// one more step) because a headless run's LAST step IS its final
/// tool-call result; adding a second, redundant "done" step on top of it
/// would double up the trace.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if `run_id` does not exist, or
/// [`StoreError::Database`] on any other failure.
pub async fn finish_run(pool: &PgPool, run_id: &str, status: &str) -> Result<(), StoreError> {
    let result = sqlx::query("UPDATE agent_run SET status = $2, ended_at = now() WHERE id = $1")
        .bind(run_id)
        .bind(status)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

/// Writes a run's real, accumulated token spend (WS7 item G2) — called
/// once, at every terminal outcome of [`crate`]'s headless loop (success,
/// failure, refusal-exhausted-iterations, and budget-exhausted alike), not
/// per LLM call, so this is a single write per run rather than a
/// write-per-call.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if `run_id` does not exist, or
/// [`StoreError::Database`] on any other failure.
pub async fn record_run_budget(
    pool: &PgPool,
    run_id: &str,
    budget_consumed: f64,
) -> Result<(), StoreError> {
    let result = sqlx::query("UPDATE agent_run SET budget_consumed = $2 WHERE id = $1")
        .bind(run_id)
        .bind(budget_consumed)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

/// Recomputes `budget_spent`/`success_rate`/`approval_rate`/`recent_runs`
/// for one employee from its real `agent_run`/`approval_item` rows (WS7
/// item G3, `recompute_employee_metrics` SQL function,
/// `0039_agent_metrics_recompute.sql`) and writes the result — called once
/// at every terminal transition of a headless run
/// (`routes::agents::run_headless_loop`), so an employee's own metrics are
/// current the moment its run ends, not on the next unrelated read.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails. Never
/// [`StoreError::NotFound`]: the underlying `UPDATE ... WHERE e.id = ...`
/// inside the SQL function is a no-op (not an error) if `employee_id` does
/// not exist, matching `PERFORM`-style void-function semantics — this
/// mirrors `record_run_budget`'s caller (`write_run_budget`), which only
/// ever calls this for an `employee_id` a run already references via a
/// foreign key, so a missing employee here would mean a foreign key
/// violation happened earlier, not something this function itself detects.
pub async fn recompute_employee_metrics(
    pool: &PgPool,
    employee_id: &str,
) -> Result<(), StoreError> {
    sqlx::query("SELECT recompute_employee_metrics($1)")
        .bind(employee_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Sets a run's status to `"waiting_approval"` WITHOUT setting `ended_at`
/// — the run isn't over, it's paused until a human decides the linked
/// [`ApprovalItem`] (`POST /api/agents/approvals/{id}/decide`); that route
/// is what eventually calls [`finish_run`] on the SAME run id.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if `run_id` does not exist, or
/// [`StoreError::Database`] on any other failure.
pub async fn mark_run_waiting_approval(pool: &PgPool, run_id: &str) -> Result<(), StoreError> {
    let result = sqlx::query("UPDATE agent_run SET status = 'waiting_approval' WHERE id = $1")
        .bind(run_id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

/// Everything [`create_linked_approval`] needs to create a `pending`
/// `approval_item` for one `WriteHigh` tool call hit by an ALREADY-EXISTING
/// headless run (unlike [`NewApprovalRequest`]/[`create_pending_approval`],
/// which always create a brand-new `agent_run` attributed to the reserved
/// `emp-copilot` row — a headless run already has its own `agent_run`,
/// created by [`create_run`], and its own real employee id/name).
#[derive(Debug, Clone)]
pub struct LinkedApprovalRequest<'a> {
    /// The employee this run belongs to — `approval_item.employee_id`.
    pub employee_id: &'a str,
    /// That employee's display name — `approval_item.employee_name`
    /// (denormalized at request time, matching every other approval).
    pub employee_name: &'a str,
    /// The already-existing `agent_run.id` this approval is linked to.
    pub run_id: &'a str,
    /// The tool name — becomes `approval_item.action`.
    pub tool: &'a str,
    /// The target resource id, if knowable ahead of execution.
    pub resource: Option<&'a str>,
    /// A deterministic, human-readable reason sentence.
    pub reason: &'a str,
    /// A human-readable risk statement.
    pub risk: &'a str,
    /// The tool call's REDACTED arguments — the caller must have already
    /// run these through `routes::ai::audit::redact`.
    pub redacted_args: &'a Value,
}

/// Creates one `pending` `approval_item` linked to an existing run (see
/// [`LinkedApprovalRequest`]) — does NOT touch the run's own `steps`/
/// `status`; the caller is responsible for calling [`append_run_step`]
/// with the pending tool call and [`mark_run_waiting_approval`] itself, so
/// the two writes stay independently retryable rather than hidden inside
/// one all-or-nothing transaction the caller cannot see into.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the insert fails (e.g. `run_id`/
/// `employee_id` do not reference existing rows).
pub async fn create_linked_approval(
    pool: &PgPool,
    req: LinkedApprovalRequest<'_>,
) -> Result<String, StoreError> {
    let approval_id = format!("appr-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO approval_item \
         (id, employee_id, employee_name, run_id, action, resource, reason, evidence, status, risk) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending', $9)",
    )
    .bind(&approval_id)
    .bind(req.employee_id)
    .bind(req.employee_name)
    .bind(req.run_id)
    .bind(req.tool)
    .bind(req.resource)
    .bind(req.reason)
    .bind(vec![serde_json::to_string(req.redacted_args).unwrap_or_default()])
    .bind(req.risk)
    .execute(pool)
    .await?;
    Ok(approval_id)
}

// ---------------------------------------------------------------------
// Access requests (WS7 item E2/E3) — `kind = 'access'` approval_item rows
// ---------------------------------------------------------------------

/// Everything [`create_access_request`] needs. An access request is still
/// an `ApprovalItem` (`kind = 'access'`) — see this module's own "Access
/// requests" section doc — not a second, parallel table.
#[derive(Debug, Clone)]
pub struct NewAccessRequest<'a> {
    /// The requesting human's `app_user.id`.
    pub requested_by_user_id: Uuid,
    /// The catalog entry the requester wants more access to —
    /// `approval_item.resource`.
    pub catalog_id: &'a str,
    /// The permission token requested (e.g. `"catalog:write"`) —
    /// `approval_item.action` is stored as `format!("access:{permission}")`
    /// so it reads clearly alongside every `kind = 'tool_call'` action
    /// (a tool name) in the same column.
    pub permission: &'a str,
    /// Why the requester wants it.
    pub reason: &'a str,
}

/// Creates one `pending`, `kind = 'access'` `approval_item` row. The
/// request itself never expires (`expires_at: None` — only the eventual
/// GRANT does, see [`decide_access_request`]).
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the insert fails (e.g.
/// `requested_by_user_id` does not reference an existing `app_user` row).
pub async fn create_access_request(
    pool: &PgPool,
    req: &NewAccessRequest<'_>,
) -> Result<ApprovalItem, StoreError> {
    let approval_id = format!("appr-{}", Uuid::new_v4());
    let action = format!("access:{}", req.permission);
    sqlx::query(
        "INSERT INTO approval_item \
         (id, kind, requested_by_user_id, action, resource, reason, status, risk) \
         VALUES ($1, 'access', $2, $3, $4, $5, 'pending', '')",
    )
    .bind(&approval_id)
    .bind(req.requested_by_user_id)
    .bind(&action)
    .bind(req.catalog_id)
    .bind(req.reason)
    .execute(pool)
    .await?;
    let select_sql =
        format!("SELECT {APPROVAL_COLUMNS_QUALIFIED} {APPROVAL_AUDIT_JOIN} WHERE a.id = $1");
    let row: ApprovalRow = sqlx::query_as(&select_sql)
        .bind(&approval_id)
        .fetch_one(pool)
        .await?;
    Ok(row.into())
}

/// `overview.pendingApprovals` (WS5 item B2): the count of approval items
/// still waiting on a human decision.
///
/// # Errors
///
/// [`StoreError::Database`] on any query failure.
pub async fn count_pending_approvals(pool: &PgPool) -> Result<i64, StoreError> {
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM approval_item WHERE status = 'pending'")
        .fetch_one(pool)
        .await?;
    Ok(n)
}

/// `GET /api/notifications` (WS5 item F1): every approval still awaiting a
/// human decision, in full — the list-shaped sibling of
/// [`count_pending_approvals`], which stays as its own fn for
/// `overview.pendingApprovals`'s count-only need. Both read `approval_item`
/// in a different projection for a different caller, matching the existing
/// pattern where `overview.rs` and this module already read the same table
/// two different ways.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_pending_approvals(pool: &PgPool) -> Result<Vec<ApprovalItem>, StoreError> {
    let sql = format!(
        "SELECT {APPROVAL_COLUMNS_QUALIFIED} {APPROVAL_AUDIT_JOIN} WHERE a.status = 'pending' \
         ORDER BY a.requested_at DESC"
    );
    let rows: Vec<ApprovalRow> = sqlx::query_as(&sql).fetch_all(pool).await?;
    Ok(rows.into_iter().map(ApprovalItem::from).collect())
}

/// `overview.agents.activeRuns` (WS5 item B2): the count of `agent_run`
/// rows currently executing (`status = 'running'`) -- a run that has
/// finished (`succeeded`/`failed`) or is paused on a human decision
/// (`waiting_approval`) does not count.
///
/// # Errors
///
/// [`StoreError::Database`] on any query failure.
pub async fn count_active_agent_runs(pool: &PgPool) -> Result<i64, StoreError> {
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_run WHERE status = 'running'")
        .fetch_one(pool)
        .await?;
    Ok(n)
}

/// `(succeeded, failed)` counts of `agent_run` rows completed in the last
/// 24h -- `overview.agentSuccessRate`/`ops.observability.agentSuccessRate`
/// (WS5 item B5). A `running` run is excluded from both counts and from
/// the denominator: it has no outcome yet, so it is neither a success nor
/// a failure.
///
/// # Errors
///
/// [`StoreError::Database`] on any query failure.
pub async fn count_agent_run_outcomes(pool: &PgPool) -> Result<(i64, i64), StoreError> {
    let row: (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE status = 'succeeded'), \
                count(*) FILTER (WHERE status = 'failed') \
         FROM agent_run WHERE ended_at > now() - INTERVAL '24 hours'",
    )
    .fetch_one(pool)
    .await?;
    Ok(row)
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

// ── WS7 item E2: `create_access_request` inserts a `kind = 'access'`
//    approval_item row ────────────────────────────────────────────────────
#[cfg(test)]
mod access_request_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use sqlx::PgPool;
    use uuid::Uuid;

    // Forces `lakehouse-test-support`'s `#[ctor]`-started Postgres
    // testcontainer to link into THIS crate's `--lib` test binary — this
    // crate's other `#[sqlx::test]`s live in `tests/` integration binaries
    // (which reference it directly), so without this, the linker strips
    // the ctor as dead code from the `--lib` binary and `#[sqlx::test]`
    // fails with "DATABASE_URL must be set" instead of getting a live
    // container. Same idiom `lakehouse-api`'s own `--lib` tests use
    // (`routes::agents::tests::budget_end_to_end`).
    use lakehouse_test_support as _;

    use super::{NewAccessRequest, create_access_request};

    /// Seeds a bare `app_user` row (no role/tenant membership needed —
    /// `create_access_request` only needs `requested_by_user_id` to
    /// reference a real `app_user`), returning its id.
    async fn seed_test_user(pool: &PgPool, email: &str) -> Uuid {
        let (id,): (Uuid,) =
            sqlx::query_as("INSERT INTO app_user (name, email) VALUES ($1, $2) RETURNING id")
                .bind(email)
                .bind(email)
                .fetch_one(pool)
                .await
                .expect("seeding a test app_user must succeed");
        id
    }

    /// Failing-test-first for WS7 item E2: before this task,
    /// `create_access_request`/`NewAccessRequest` did not exist, and
    /// `approval_item` had no `kind`/`requested_by_user_id` columns at
    /// all. Quoted failure text (`cargo test -p lakehouse-store
    /// agents::create_access_request 2>&1 | tail -30` against the pre-E1/E2
    /// state): `error[E0433]: failed to resolve: use of undeclared type
    /// \`NewAccessRequest\`` / `error[E0425]: cannot find function
    /// \`create_access_request\` in this scope`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn create_access_request_inserts_a_pending_approval_with_kind_access(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let requester = seed_test_user(&pool, "req@tenant-a.invalid").await;
        let req = create_access_request(
            &pool,
            &NewAccessRequest {
                requested_by_user_id: requester,
                catalog_id: "catalog-1",
                permission: "catalog:write",
                reason: "need to correct a mapping error",
            },
        )
        .await
        .expect("creating a well-formed access request must succeed");
        assert_eq!(req.kind, "access");
        assert_eq!(req.status, "pending");
        assert_eq!(req.requested_by_user_id, Some(requester));
        assert_eq!(req.action, "access:catalog:write");
        assert_eq!(req.resource.as_deref(), Some("catalog-1"));
        Ok(())
    }
}
