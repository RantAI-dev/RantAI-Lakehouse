//! Integration tests for `lakehouse_store::agents` against a real
//! Postgres.
//!
//! # Postgres backing
//!
//! These are `#[sqlx::test(migrations = "../../migrations")]` tests: each
//! one gets a freshly migrated, isolated database. The Postgres *server*
//! itself is started once per test binary by the `lakehouse-test-support`
//! dev-dependency, which spins up a `testcontainers`-managed Postgres and
//! points `DATABASE_URL` at it before any test runs — no manual
//! `docker compose up`, no external database required. Docker must be
//! reachable from the environment running `cargo test`.

// WS7 item G2: `budget_limit`/`budget_consumed` assertions below compare
// f64s produced by an exact-literal insert against an exact-literal
// expectation (no accumulated floating-point arithmetic in between), so a
// strict `==` is exactly what's being tested (matching
// `lakehouse-alerts`'s own identical `#![allow(clippy::float_cmp)]` for
// the same reason).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap actually runs for this test binary (an
// unreferenced dev-dependency's rlib member can otherwise be dropped
// by the linker before its ctor section is ever considered).
use lakehouse_test_support as _;

use lakehouse_store::StoreError;
use lakehouse_store::agents::{
    COPILOT_EMPLOYEE_ID, CreateEmployeeInput, CreatedApproval, Decision, LinkedApprovalRequest,
    NewApprovalRequest, RunStep, append_run_step, count_active_agent_runs,
    count_agent_run_outcomes, count_pending_approvals, create_employee, create_linked_approval,
    create_pending_approval, create_run, decide_approval, finish_run, get_employee,
    get_employee_run_config, get_employee_with_metrics, get_run, list_approvals, list_employees,
    list_runs, list_scheduled_employees, list_tools, list_workflows, mark_run_waiting_approval,
    pending_tool_call, recompute_employee_metrics, record_run_budget, record_run_outcome,
    resume_employee, revoke_employee, suspend_employee,
};
use serde_json::json;
use sqlx::PgPool;

/// Insert a minimal `agent_run` row directly, bypassing the store's own
/// [`create_run`] (kept as a plain-SQL fixture helper for tests that only
/// care about a run existing, not about `create_run`'s own contract, which
/// gets its own tests below). Test-only fixture helper.
async fn insert_run(pool: &PgPool, id: &str, employee_id: &str) {
    sqlx::query(
        "INSERT INTO agent_run (id, employee_id, status, trigger, actor, steps) \
         VALUES ($1, $2, 'running', 'test', 'test-actor', '[]'::jsonb)",
    )
    .bind(id)
    .bind(employee_id)
    .execute(pool)
    .await
    .unwrap();
}

/// Insert a minimal `approval_item` row directly, bypassing the store.
/// Test-only fixture helper.
async fn insert_approval(
    pool: &PgPool,
    id: &str,
    employee_id: &str,
    employee_name: &str,
    run_id: Option<&str>,
    status: &str,
) {
    sqlx::query(
        "INSERT INTO approval_item (id, employee_id, employee_name, run_id, action, status) \
         VALUES ($1, $2, $3, $4, 'test action', $5)",
    )
    .bind(id)
    .bind(employee_id)
    .bind(employee_name)
    .bind(run_id)
    .bind(status)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
async fn seed_populates_every_agents_list(pool: PgPool) -> sqlx::Result<()> {
    assert_eq!(list_workflows(&pool).await.unwrap().len(), 2);
    // emp-inventory, emp-risk, and the reserved emp-copilot (0024).
    assert_eq!(list_employees(&pool).await.unwrap().len(), 3);
    assert_eq!(list_tools(&pool).await.unwrap().len(), 3);
    // 0025 drops the seeded run/approval fixture history; there is still
    // no live execution path in this migration set, so both are empty.
    assert_eq!(list_runs(&pool, None).await.unwrap().len(), 0);
    assert_eq!(list_approvals(&pool, None).await.unwrap().len(), 0);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn migration_0025_removed_seeded_runs_and_approvals(pool: PgPool) -> sqlx::Result<()> {
    let seeded_run_ids = ["run-col-01", "run-risk-01"];
    for id in seeded_run_ids {
        assert!(
            get_run(&pool, id).await.unwrap().is_none(),
            "{id} should be gone"
        );
    }
    let seeded_approval_ids = ["ap-01", "ap-02", "ap-03"];
    let approvals = list_approvals(&pool, None).await.unwrap();
    for id in seeded_approval_ids {
        assert!(!approvals.iter().any(|a| a.id == id), "{id} should be gone");
    }
    // The tables are still usable: a fresh run/approval can be inserted
    // and read back.
    insert_run(&pool, "run-fresh-01", "emp-risk").await;
    insert_approval(
        &pool,
        "ap-fresh-01",
        "emp-risk",
        "ops-sentinel",
        Some("run-fresh-01"),
        "pending",
    )
    .await;
    assert!(get_run(&pool, "run-fresh-01").await.unwrap().is_some());
    assert_eq!(list_approvals(&pool, None).await.unwrap().len(), 1);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_runs_filters_by_employee(pool: PgPool) -> sqlx::Result<()> {
    insert_run(&pool, "run-risk-fresh", "emp-risk").await;
    insert_run(&pool, "run-inv-fresh", "emp-inventory").await;
    let runs = list_runs(&pool, Some("emp-risk")).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, "run-risk-fresh");
    Ok(())
}

/// A run's `approvals[]` is derived from `approval_item.run_id`, not
/// duplicated state — this is the regression test for that join.
#[sqlx::test(migrations = "../../migrations")]
async fn run_embeds_its_approvals(pool: PgPool) -> sqlx::Result<()> {
    insert_run(&pool, "run-col-fresh", "emp-inventory").await;
    insert_approval(
        &pool,
        "ap-fresh-a",
        "emp-inventory",
        "inventory-copilot",
        Some("run-col-fresh"),
        "pending",
    )
    .await;
    insert_approval(
        &pool,
        "ap-fresh-b",
        "emp-inventory",
        "inventory-copilot",
        Some("run-col-fresh"),
        "approved",
    )
    .await;
    let run = get_run(&pool, "run-col-fresh").await.unwrap().unwrap();
    assert_eq!(run.approvals.len(), 2);
    assert!(
        run.approvals
            .iter()
            .any(|a| a.id == "ap-fresh-a" && a.status == "pending")
    );
    assert!(
        run.approvals
            .iter()
            .any(|a| a.id == "ap-fresh-b" && a.status == "approved")
    );
    Ok(())
}

/// Deleting an employee cascades to its runs — `0017_agents.sql` declares
/// `agent_run.employee_id REFERENCES agent_employee (id) ON DELETE
/// CASCADE`.
#[sqlx::test(migrations = "../../migrations")]
async fn deleting_employee_cascades_to_its_runs(pool: PgPool) -> sqlx::Result<()> {
    insert_run(&pool, "run-cascade-01", "emp-risk").await;
    assert!(get_run(&pool, "run-cascade-01").await.unwrap().is_some());
    sqlx::query("DELETE FROM agent_employee WHERE id = $1")
        .bind("emp-risk")
        .execute(&pool)
        .await
        .unwrap();
    assert!(get_run(&pool, "run-cascade-01").await.unwrap().is_none());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn get_employee_none_for_unknown_id(pool: PgPool) -> sqlx::Result<()> {
    assert!(get_employee(&pool, "emp-nope").await.unwrap().is_none());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn duplicate_employee_name_is_a_conflict(pool: PgPool) -> sqlx::Result<()> {
    let input = CreateEmployeeInput {
        name: "inventory-copilot".to_owned(),
        purpose: "p".to_owned(),
        autonomy: "L1".to_owned(),
        allowed_tools: vec![],
        data_scope: "d".to_owned(),
        budget_limit: 0.0,
        owner: None,
        prompt: None,
        schedule_cron: None,
        mode: None,
        permissions: None,
    };
    let err = create_employee(&pool, &input).await.unwrap_err();
    assert!(matches!(err, StoreError::Conflict));
    Ok(())
}

/// Suspend -> resume -> revoke each transition `status` and round-trip
/// through `get_employee`.
#[sqlx::test(migrations = "../../migrations")]
async fn employee_lifecycle_transitions(pool: PgPool) -> sqlx::Result<()> {
    let suspended = suspend_employee(&pool, "emp-inventory").await.unwrap();
    assert_eq!(suspended.status, "paused");
    let resumed = resume_employee(&pool, "emp-inventory").await.unwrap();
    assert_eq!(resumed.status, "ready");
    let revoked = revoke_employee(&pool, "emp-inventory").await.unwrap();
    assert_eq!(revoked.status, "cancelled");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn suspend_unknown_employee_is_not_found(pool: PgPool) -> sqlx::Result<()> {
    let err = suspend_employee(&pool, "emp-nope").await.unwrap_err();
    assert!(matches!(err, StoreError::NotFound));
    Ok(())
}

/// The core approval-lifecycle guarantee the task brief calls out: an
/// already-decided approval cannot be re-decided.
#[sqlx::test(migrations = "../../migrations")]
async fn deciding_an_already_decided_approval_is_a_conflict(pool: PgPool) -> sqlx::Result<()> {
    insert_approval(
        &pool,
        "ap-already-decided",
        "emp-risk",
        "ops-sentinel",
        None,
        "approved",
    )
    .await;
    let err = decide_approval(&pool, "ap-already-decided", Decision::Rejected, None)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Conflict));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn deciding_a_pending_approval_stamps_decided_at_and_status(
    pool: PgPool,
) -> sqlx::Result<()> {
    insert_run(&pool, "run-decide-01", "emp-inventory").await;
    insert_approval(
        &pool,
        "ap-decide-01",
        "emp-inventory",
        "inventory-copilot",
        Some("run-decide-01"),
        "pending",
    )
    .await;
    let decided = decide_approval(
        &pool,
        "ap-decide-01",
        Decision::Approved,
        Some("looks fine"),
    )
    .await
    .unwrap();
    assert_eq!(decided.status, "approved");
    assert!(decided.decided_at.is_some());
    assert_eq!(decided.comment.as_deref(), Some("looks fine"));
    // WS1 task 1.14 (judge finding J12): `decide_approval` no longer mints
    // a synthetic `aud-approval-<id>-<status>` id, and it re-reads before
    // `routes::agents::decide_approval` has written the real audit event —
    // so a fresh decision always carries `None` here (see the doc comment
    // on `lakehouse_store::agents::decide_approval`).
    assert_eq!(decided.audit_event_id, None);

    // Reflected in the owning run's embedded approvals too.
    let run = get_run(&pool, "run-decide-01").await.unwrap().unwrap();
    assert!(
        run.approvals
            .iter()
            .any(|a| a.id == "ap-decide-01" && a.status == "approved")
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn decide_unknown_approval_is_not_found(pool: PgPool) -> sqlx::Result<()> {
    let err = decide_approval(&pool, "ap-nope", Decision::Approved, None)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::NotFound));
    Ok(())
}

/// WS1 task 1.14 (judge finding J12): once the real `audit_event` row
/// `routes::agents::decide_approval` writes via `ai_audit::record` lands
/// (`resource_kind = 'approval'`, `resource_id = <approval id>`),
/// `list_approvals` resolves it through the lateral join — the "View
/// audit" link this task is fixing. Before that write, the link stays
/// absent rather than pointing at a synthetic id.
#[sqlx::test(migrations = "../../migrations")]
async fn list_approvals_resolves_the_real_audit_event_id_after_decision(
    pool: PgPool,
) -> sqlx::Result<()> {
    insert_run(&pool, "run-audit-01", "emp-inventory").await;
    insert_approval(
        &pool,
        "ap-audit-01",
        "emp-inventory",
        "inventory-copilot",
        Some("run-audit-01"),
        "pending",
    )
    .await;
    decide_approval(&pool, "ap-audit-01", Decision::Approved, None)
        .await
        .unwrap();

    let before = list_approvals(&pool, None).await.unwrap();
    let item = before.iter().find(|a| a.id == "ap-audit-01").unwrap();
    assert_eq!(item.audit_event_id, None);

    let event = lakehouse_store::audit::insert(
        &pool,
        lakehouse_store::audit::NewAuditEvent {
            action: "approve_action".to_owned(),
            resource_kind: Some("approval".to_owned()),
            resource_id: Some("ap-audit-01".to_owned()),
            outcome: "approved".to_owned(),
            ..lakehouse_store::audit::NewAuditEvent::default()
        },
    )
    .await
    .unwrap();

    let after = list_approvals(&pool, None).await.unwrap();
    let item = after.iter().find(|a| a.id == "ap-audit-01").unwrap();
    assert_eq!(item.audit_event_id.as_deref(), Some(event.id.as_str()));
    Ok(())
}

// ---------------------------------------------------------------------
// T3.1: schedulable digital employees (migration 0025/0026)
// ---------------------------------------------------------------------

/// The four new columns round-trip through `create_employee` and
/// `get_employee`.
#[sqlx::test(migrations = "../../migrations")]
async fn new_columns_round_trip(pool: PgPool) -> sqlx::Result<()> {
    let input = CreateEmployeeInput {
        name: "schedulable-copilot".to_owned(),
        purpose: "p".to_owned(),
        autonomy: "L2".to_owned(),
        allowed_tools: vec![],
        data_scope: "d".to_owned(),
        budget_limit: 0.0,
        owner: None,
        prompt: Some("Summarize yesterday's alerts.".to_owned()),
        schedule_cron: Some("0 6 * * *".to_owned()),
        mode: Some("ask".to_owned()),
        permissions: Some("alert:write, query:read".to_owned()),
    };
    let created = create_employee(&pool, &input).await.unwrap();
    assert_eq!(
        created.prompt.as_deref(),
        Some("Summarize yesterday's alerts.")
    );
    assert_eq!(created.schedule_cron.as_deref(), Some("0 6 * * *"));
    assert_eq!(created.mode, "ask");
    assert_eq!(created.permissions, "alert:write, query:read");

    let fetched = get_employee(&pool, &created.id).await.unwrap().unwrap();
    assert_eq!(fetched, created);
    Ok(())
}

/// Fields left unset default to manual-only, `"build"` mode, and empty
/// (authenticated-only) permissions.
#[sqlx::test(migrations = "../../migrations")]
async fn new_columns_default_when_absent(pool: PgPool) -> sqlx::Result<()> {
    let input = CreateEmployeeInput {
        name: "default-copilot".to_owned(),
        purpose: "p".to_owned(),
        autonomy: "L1".to_owned(),
        allowed_tools: vec![],
        data_scope: "d".to_owned(),
        budget_limit: 0.0,
        owner: None,
        prompt: None,
        schedule_cron: None,
        mode: None,
        permissions: None,
    };
    let created = create_employee(&pool, &input).await.unwrap();
    assert_eq!(created.prompt, None);
    assert_eq!(created.schedule_cron, None);
    assert_eq!(created.mode, "build");
    assert_eq!(created.permissions, "");
    Ok(())
}

/// The `mode` CHECK constraint rejects anything other than `ask`/`build`.
#[sqlx::test(migrations = "../../migrations")]
async fn mode_check_rejects_invalid_value(pool: PgPool) -> sqlx::Result<()> {
    let err = sqlx::query(
        "INSERT INTO agent_employee (id, name, purpose, owner, autonomy, mode) \
         VALUES ('emp-bad-mode', 'bad-mode-employee', 'p', 'o', 'L1', 'sleep')",
    )
    .execute(&pool)
    .await
    .unwrap_err();
    let db_err = err.as_database_error().expect("a database error");
    assert_eq!(db_err.constraint(), Some("agent_employee_mode_check"));
    Ok(())
}

/// `list_scheduled_employees` returns exactly the employees with a
/// non-NULL `schedule_cron`, excluding the reserved `emp-copilot` row
/// (which is manual-only).
#[sqlx::test(migrations = "../../migrations")]
async fn list_scheduled_employees_returns_only_cron_set(pool: PgPool) -> sqlx::Result<()> {
    assert!(list_scheduled_employees(&pool).await.unwrap().is_empty());

    let input = CreateEmployeeInput {
        name: "cron-copilot".to_owned(),
        purpose: "p".to_owned(),
        autonomy: "L2".to_owned(),
        allowed_tools: vec![],
        data_scope: "d".to_owned(),
        budget_limit: 0.0,
        owner: None,
        prompt: Some("Run nightly export.".to_owned()),
        schedule_cron: Some("0 2 * * *".to_owned()),
        mode: Some("build".to_owned()),
        permissions: Some("gold:export".to_owned()),
    };
    let created = create_employee(&pool, &input).await.unwrap();

    let scheduled = list_scheduled_employees(&pool).await.unwrap();
    assert_eq!(scheduled.len(), 1);
    assert_eq!(scheduled[0].id, created.id);
    assert!(!scheduled.iter().any(|e| e.id == "emp-copilot"));
    Ok(())
}

/// `emp-copilot` exists after migration 0025, is manual-only, and is not
/// runnable (no prompt).
#[sqlx::test(migrations = "../../migrations")]
async fn emp_copilot_exists_and_is_manual_only(pool: PgPool) -> sqlx::Result<()> {
    let copilot = get_employee(&pool, "emp-copilot")
        .await
        .unwrap()
        .expect("emp-copilot must exist after migration 0025");
    assert_eq!(copilot.schedule_cron, None);
    assert_eq!(copilot.prompt, None);
    assert_eq!(copilot.permissions, "");

    let config = get_employee_run_config(&pool, "emp-copilot")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(config.name, "Copilot (interactive)");
    assert_eq!(config.status, "ready");
    assert_eq!(config.prompt, None);
    assert_eq!(config.mode, "build");
    assert_eq!(config.permissions, "");
    Ok(())
}

/// `get_employee_run_config` round-trips prompt/mode/permissions/status
/// for a runnable employee.
#[sqlx::test(migrations = "../../migrations")]
async fn get_employee_run_config_round_trips(pool: PgPool) -> sqlx::Result<()> {
    let input = CreateEmployeeInput {
        name: "runnable-copilot".to_owned(),
        purpose: "p".to_owned(),
        autonomy: "L2".to_owned(),
        allowed_tools: vec![],
        data_scope: "d".to_owned(),
        budget_limit: 0.0,
        owner: None,
        prompt: Some("Do the thing.".to_owned()),
        schedule_cron: None,
        mode: Some("ask".to_owned()),
        permissions: Some("query:read".to_owned()),
    };
    let created = create_employee(&pool, &input).await.unwrap();
    let config = get_employee_run_config(&pool, &created.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(config.name, "runnable-copilot");
    assert_eq!(config.status, "draft");
    assert_eq!(config.prompt.as_deref(), Some("Do the thing."));
    assert_eq!(config.mode, "ask");
    assert_eq!(config.permissions, "query:read");
    Ok(())
}

// ---------------------------------------------------------------------
// WriteHigh copilot approvals (T0.5, copilot-operations-handover plan)
// ---------------------------------------------------------------------

/// `create_pending_approval` creates exactly one `agent_run`
/// (`waiting_approval`, attributed to `emp-copilot`) and one linked
/// `approval_item` (`pending`), and the run's stored tool call round-trips
/// through `pending_tool_call` exactly.
#[sqlx::test(migrations = "../../migrations")]
async fn create_pending_approval_creates_linked_run_and_approval(pool: PgPool) -> sqlx::Result<()> {
    let args = json!({ "id": "c-1" });
    let CreatedApproval {
        run_id,
        approval_id,
    } = create_pending_approval(
        &pool,
        NewApprovalRequest {
            tool: "delete_chart",
            actor: "Fajar Nugroho",
            resource: Some("c-1"),
            reason: "Menghapus chart c-1.",
            risk: "tinggi",
            redacted_args: &args,
        },
    )
    .await
    .unwrap();

    let run = get_run(&pool, &run_id).await.unwrap().unwrap();
    assert_eq!(run.employee_id, COPILOT_EMPLOYEE_ID);
    assert_eq!(run.status, "waiting_approval");
    assert_eq!(run.trigger, "copilot");
    assert_eq!(run.actor, "Fajar Nugroho");
    assert_eq!(run.approvals.len(), 1);
    assert_eq!(run.approvals[0].id, approval_id);
    assert_eq!(run.approvals[0].status, "pending");

    let pending = pending_tool_call(&run).expect("pending tool call recorded");
    assert_eq!(pending.tool, "delete_chart");
    assert_eq!(pending.args, args);

    let approvals = list_approvals(&pool, Some(COPILOT_EMPLOYEE_ID))
        .await
        .unwrap();
    let approval = approvals
        .iter()
        .find(|a| a.id == approval_id)
        .expect("the created approval is listed");
    assert_eq!(approval.run_id.as_deref(), Some(run_id.as_str()));
    assert_eq!(approval.action, "delete_chart");
    assert_eq!(approval.resource.as_deref(), Some("c-1"));
    assert_eq!(approval.status, "pending");
    assert_eq!(
        approval.evidence.as_deref(),
        Some([serde_json::to_string(&args).unwrap()].as_slice())
    );
    Ok(())
}

/// `record_run_outcome` appends a step and sets the terminal status —
/// proven here for the "rejected" ending.
#[sqlx::test(migrations = "../../migrations")]
async fn record_run_outcome_sets_status_and_appends_step(pool: PgPool) -> sqlx::Result<()> {
    let CreatedApproval { run_id, .. } = create_pending_approval(
        &pool,
        NewApprovalRequest {
            tool: "delete_chart",
            actor: "Fajar Nugroho",
            resource: Some("c-1"),
            reason: "Menghapus chart c-1.",
            risk: "tinggi",
            redacted_args: &json!({ "id": "c-1" }),
        },
    )
    .await
    .unwrap();

    record_run_outcome(
        &pool,
        &run_id,
        "rejected",
        "Rejected",
        "does not meet policy",
    )
    .await
    .unwrap();

    let run = get_run(&pool, &run_id).await.unwrap().unwrap();
    assert_eq!(run.status, "rejected");
    assert!(run.ended_at.is_some());
    assert_eq!(run.steps.len(), 2);
    assert_eq!(run.steps[1].status, "rejected");
    assert_eq!(run.steps[1].detail, "does not meet policy");
    // The original pending-call step must still be there, untouched.
    assert_eq!(run.steps[0].status, "pending");
    Ok(())
}

/// `record_run_outcome` against an unknown run id is a
/// [`StoreError::NotFound`], not a silent no-op.
#[sqlx::test(migrations = "../../migrations")]
async fn record_run_outcome_unknown_run_is_not_found(pool: PgPool) -> sqlx::Result<()> {
    let err = record_run_outcome(&pool, "run-nope", "failed", "x", "y")
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::NotFound));
    Ok(())
}

// ---------------------------------------------------------------------
// Headless employee runs (T3.2, copilot-operations-handover plan)
// ---------------------------------------------------------------------

/// `create_run` inserts a `"running"` run with empty `steps`, attributed
/// to the given trigger/actor.
#[sqlx::test(migrations = "../../migrations")]
async fn create_run_inserts_running_row(pool: PgPool) -> sqlx::Result<()> {
    create_run(
        &pool,
        "run-headless-01",
        "emp-risk",
        "schedule",
        "risk-sentinel",
    )
    .await
    .unwrap();
    let run = get_run(&pool, "run-headless-01").await.unwrap().unwrap();
    assert_eq!(run.employee_id, "emp-risk");
    assert_eq!(run.status, "running");
    assert_eq!(run.trigger, "schedule");
    assert_eq!(run.actor, "risk-sentinel");
    assert!(run.steps.is_empty());
    assert!(run.ended_at.is_none());
    Ok(())
}

/// `append_run_step` adds to `steps` without touching `status`/`ended_at`,
/// and can be called more than once.
#[sqlx::test(migrations = "../../migrations")]
async fn append_run_step_accumulates_without_finishing(pool: PgPool) -> sqlx::Result<()> {
    create_run(
        &pool,
        "run-headless-02",
        "emp-risk",
        "manual",
        "Fajar Nugroho",
    )
    .await
    .unwrap();
    append_run_step(
        &pool,
        "run-headless-02",
        RunStep {
            id: "step-1".to_owned(),
            label: "run_sql".to_owned(),
            status: "succeeded".to_owned(),
            detail: "{}".to_owned(),
        },
    )
    .await
    .unwrap();
    append_run_step(
        &pool,
        "run-headless-02",
        RunStep {
            id: "step-2".to_owned(),
            label: "list_datasets".to_owned(),
            status: "succeeded".to_owned(),
            detail: "{}".to_owned(),
        },
    )
    .await
    .unwrap();

    let run = get_run(&pool, "run-headless-02").await.unwrap().unwrap();
    assert_eq!(run.status, "running");
    assert!(run.ended_at.is_none());
    assert_eq!(run.steps.len(), 2);
    assert_eq!(run.steps[0].label, "run_sql");
    assert_eq!(run.steps[1].label, "list_datasets");
    Ok(())
}

/// `append_run_step` against an unknown run id is `NotFound`, not a
/// silent no-op.
#[sqlx::test(migrations = "../../migrations")]
async fn append_run_step_unknown_run_is_not_found(pool: PgPool) -> sqlx::Result<()> {
    let err = append_run_step(
        &pool,
        "run-nope",
        RunStep {
            id: "step-1".to_owned(),
            label: "x".to_owned(),
            status: "succeeded".to_owned(),
            detail: "{}".to_owned(),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, StoreError::NotFound));
    Ok(())
}

/// `finish_run` sets a terminal status and `ended_at`, WITHOUT appending a
/// step — unlike `record_run_outcome`.
#[sqlx::test(migrations = "../../migrations")]
async fn finish_run_sets_terminal_status_without_a_step(pool: PgPool) -> sqlx::Result<()> {
    create_run(
        &pool,
        "run-headless-03",
        "emp-risk",
        "manual",
        "Fajar Nugroho",
    )
    .await
    .unwrap();
    append_run_step(
        &pool,
        "run-headless-03",
        RunStep {
            id: "step-1".to_owned(),
            label: "run_sql".to_owned(),
            status: "succeeded".to_owned(),
            detail: "{}".to_owned(),
        },
    )
    .await
    .unwrap();
    finish_run(&pool, "run-headless-03", "succeeded")
        .await
        .unwrap();

    let run = get_run(&pool, "run-headless-03").await.unwrap().unwrap();
    assert_eq!(run.status, "succeeded");
    assert!(run.ended_at.is_some());
    // Still exactly the one step `append_run_step` added — `finish_run`
    // must not add a second one.
    assert_eq!(run.steps.len(), 1);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn finish_run_unknown_run_is_not_found(pool: PgPool) -> sqlx::Result<()> {
    let err = finish_run(&pool, "run-nope", "failed").await.unwrap_err();
    assert!(matches!(err, StoreError::NotFound));
    Ok(())
}

/// `mark_run_waiting_approval` sets the status WITHOUT setting `ended_at`
/// — the run is paused, not over.
#[sqlx::test(migrations = "../../migrations")]
async fn mark_run_waiting_approval_does_not_end_the_run(pool: PgPool) -> sqlx::Result<()> {
    create_run(
        &pool,
        "run-headless-04",
        "emp-risk",
        "schedule",
        "risk-sentinel",
    )
    .await
    .unwrap();
    mark_run_waiting_approval(&pool, "run-headless-04")
        .await
        .unwrap();
    let run = get_run(&pool, "run-headless-04").await.unwrap().unwrap();
    assert_eq!(run.status, "waiting_approval");
    assert!(run.ended_at.is_none());
    Ok(())
}

// ---------------------------------------------------------------------
// WS7 item G2: real, accumulated token-based budget consumption
// ---------------------------------------------------------------------

/// A run still in progress (`status = "running"`) has never had
/// `agent_run.budget_consumed`'s `NOT NULL DEFAULT 0` overwritten by
/// `record_run_budget` — `get_run` must report `None`, not the DB
/// default `0`, for this window: a fabricated zero would claim "zero
/// tokens consumed" when the true answer is "not measured yet". Named
/// per the program's "each honest-null field needs a named test"
/// requirement.
#[sqlx::test(migrations = "../../migrations")]
async fn budget_consumed_is_none_not_a_fabricated_zero_while_a_run_is_still_running(
    pool: PgPool,
) -> sqlx::Result<()> {
    create_run(
        &pool,
        "run-budget-06",
        "emp-risk",
        "manual",
        "Fajar Nugroho",
    )
    .await
    .unwrap();
    let run = get_run(&pool, "run-budget-06").await.unwrap().unwrap();
    assert_eq!(run.status, "running");
    assert_eq!(run.budget_consumed, None);
    Ok(())
}

/// `record_run_budget` writes the real, caller-accumulated value, and
/// `get_run` reports it as `Some` once the run has reached a terminal
/// status.
#[sqlx::test(migrations = "../../migrations")]
async fn record_run_budget_writes_the_real_accumulated_value(pool: PgPool) -> sqlx::Result<()> {
    create_run(
        &pool,
        "run-budget-07",
        "emp-risk",
        "manual",
        "Fajar Nugroji",
    )
    .await
    .unwrap();
    record_run_budget(&pool, "run-budget-07", 120.0)
        .await
        .unwrap();
    finish_run(&pool, "run-budget-07", "succeeded")
        .await
        .unwrap();
    let run = get_run(&pool, "run-budget-07").await.unwrap().unwrap();
    assert_eq!(run.budget_consumed, Some(120.0));
    Ok(())
}

/// `record_run_budget` against a run id that does not exist is
/// [`StoreError::NotFound`], matching every other single-row `agent_run`
/// mutator in this module (`finish_run`, `mark_run_waiting_approval`).
#[sqlx::test(migrations = "../../migrations")]
async fn record_run_budget_unknown_run_is_not_found(pool: PgPool) -> sqlx::Result<()> {
    let err = record_run_budget(&pool, "run-nope", 10.0)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::NotFound));
    Ok(())
}

/// `get_employee_run_config` now also carries `budget_limit` — the
/// headless loop's own enforcement ceiling (WS7 item G2), previously
/// absent from this narrower projection entirely.
#[sqlx::test(migrations = "../../migrations")]
async fn get_employee_run_config_includes_budget_limit(pool: PgPool) -> sqlx::Result<()> {
    let input = CreateEmployeeInput {
        name: "budget-limit-employee".to_owned(),
        purpose: "p".to_owned(),
        autonomy: "L2".to_owned(),
        allowed_tools: vec![],
        data_scope: "d".to_owned(),
        budget_limit: 250.0,
        owner: None,
        prompt: Some("do the thing".to_owned()),
        schedule_cron: None,
        mode: Some("build".to_owned()),
        permissions: None,
    };
    let created = create_employee(&pool, &input).await.unwrap();
    let config = get_employee_run_config(&pool, &created.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(config.budget_limit, 250.0);
    Ok(())
}

/// `create_linked_approval` creates a `pending` approval attributed to the
/// REAL employee (not `emp-copilot`), linked to an ALREADY-EXISTING run,
/// and does not itself touch that run's `steps`/`status`.
#[sqlx::test(migrations = "../../migrations")]
async fn create_linked_approval_attaches_to_existing_run_and_real_employee(
    pool: PgPool,
) -> sqlx::Result<()> {
    create_run(
        &pool,
        "run-headless-05",
        "emp-risk",
        "schedule",
        "risk-sentinel",
    )
    .await
    .unwrap();
    let args = json!({ "id": "wl-1" });
    let approval_id = create_linked_approval(
        &pool,
        LinkedApprovalRequest {
            employee_id: "emp-risk",
            employee_name: "risk-sentinel",
            run_id: "run-headless-05",
            tool: "cancel_pipeline",
            resource: Some("wl-1"),
            reason: "Membatalkan pipeline wl-1.",
            risk: "tinggi",
            redacted_args: &args,
        },
    )
    .await
    .unwrap();

    let approvals = list_approvals(&pool, Some("emp-risk")).await.unwrap();
    let approval = approvals
        .iter()
        .find(|a| a.id == approval_id)
        .expect("the created approval is listed under the REAL employee");
    assert_eq!(approval.employee_id, "emp-risk");
    assert_eq!(approval.employee_name, "risk-sentinel");
    assert_eq!(approval.run_id.as_deref(), Some("run-headless-05"));
    assert_eq!(approval.action, "cancel_pipeline");
    assert_eq!(approval.status, "pending");

    // The run itself is untouched by `create_linked_approval` alone.
    let run = get_run(&pool, "run-headless-05").await.unwrap().unwrap();
    assert_eq!(run.status, "running");
    assert!(run.steps.is_empty());
    // But the run's own `approvals[]` projection already sees it, via the
    // `run_id` join — same mechanism `run_embeds_its_approvals` covers.
    assert_eq!(run.approvals.len(), 1);
    assert_eq!(run.approvals[0].id, approval_id);
    Ok(())
}

// ---------------------------------------------------------------------
// WS1 task 1.11: unmeasured budget/outcome metrics are `None`, not
// insert-time defaults presented as measurements.
// ---------------------------------------------------------------------

/// `agent_employee.budget_spent`, `budget_reserved`, `approval_rate`,
/// `success_rate` and `recent_runs` are real columns nothing ever updates.
/// The store must not select them back as `0`/`0.0` — they come back
/// `None`, and serialize to JSON `null` (not omitted).
#[sqlx::test(migrations = "../../migrations")]
async fn unmeasured_employee_metrics_are_none_not_insert_time_defaults(
    pool: PgPool,
) -> sqlx::Result<()> {
    let input = CreateEmployeeInput {
        name: "unmeasured-metrics-employee".to_owned(),
        purpose: "p".to_owned(),
        autonomy: "L1".to_owned(),
        allowed_tools: vec![],
        data_scope: "d".to_owned(),
        budget_limit: 100.0,
        owner: None,
        prompt: None,
        schedule_cron: None,
        mode: None,
        permissions: None,
    };
    let created = create_employee(&pool, &input).await.unwrap();
    assert_eq!(created.budget_spent, None);
    assert_eq!(created.budget_reserved, None);
    assert_eq!(created.approval_rate, None);
    assert_eq!(created.success_rate, None);
    assert_eq!(created.recent_runs, None);

    // Also true when fetched back through `get_employee`, not just the
    // `RETURNING` row from the insert.
    let fetched = get_employee(&pool, &created.id).await.unwrap().unwrap();
    assert_eq!(fetched.budget_spent, None);
    assert_eq!(fetched.budget_reserved, None);
    assert_eq!(fetched.approval_rate, None);
    assert_eq!(fetched.success_rate, None);
    assert_eq!(fetched.recent_runs, None);

    let value = serde_json::to_value(&fetched).unwrap();
    assert_eq!(value["budgetSpent"], serde_json::Value::Null);
    assert_eq!(value["budgetReserved"], serde_json::Value::Null);
    assert_eq!(value["approvalRate"], serde_json::Value::Null);
    assert_eq!(value["successRate"], serde_json::Value::Null);
    assert_eq!(value["recentRuns"], serde_json::Value::Null);
    // The keys are present, not omitted — `null` on the wire, not a
    // missing field a client would have to distinguish from `undefined`.
    assert!(value.as_object().unwrap().contains_key("budgetSpent"));
    Ok(())
}

/// `agent_run.budget_consumed` is the same story: a real column nothing
/// ever updates, so a run fetched through the store reports it `None`.
#[sqlx::test(migrations = "../../migrations")]
async fn unmeasured_run_budget_consumed_is_none_not_insert_time_default(
    pool: PgPool,
) -> sqlx::Result<()> {
    insert_run(&pool, "run-unmeasured-budget", "emp-risk").await;
    let run = get_run(&pool, "run-unmeasured-budget")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.budget_consumed, None);
    Ok(())
}

/// `overview.pendingApprovals` (WS5 item B2): only `status = 'pending'`
/// rows count -- an `approved`/`rejected` approval is no longer waiting on
/// anyone.
#[sqlx::test(migrations = "../../migrations")]
async fn count_pending_approvals_counts_only_pending_status(pool: PgPool) -> sqlx::Result<()> {
    insert_approval(
        &pool,
        "appr-pending-1",
        "emp-inventory",
        "inventory-copilot",
        None,
        "pending",
    )
    .await;
    insert_approval(
        &pool,
        "appr-pending-2",
        "emp-inventory",
        "inventory-copilot",
        None,
        "pending",
    )
    .await;
    insert_approval(
        &pool,
        "appr-approved-1",
        "emp-inventory",
        "inventory-copilot",
        None,
        "approved",
    )
    .await;
    insert_approval(
        &pool,
        "appr-rejected-1",
        "emp-risk",
        "ops-sentinel",
        None,
        "rejected",
    )
    .await;

    assert_eq!(count_pending_approvals(&pool).await.unwrap(), 2);
    Ok(())
}

/// `overview.agents.activeRuns` (WS5 item B2): only `status = 'running'`
/// rows count -- a `succeeded`/`failed`/`waiting_approval` run has already
/// finished (or paused), it is not an active run.
#[sqlx::test(migrations = "../../migrations")]
async fn count_active_agent_runs_counts_only_running_status(pool: PgPool) -> sqlx::Result<()> {
    insert_run(&pool, "run-active-1", "emp-inventory").await;
    insert_run(&pool, "run-active-2", "emp-risk").await;
    sqlx::query("INSERT INTO agent_run (id, employee_id, status, trigger, actor, steps) VALUES ($1, $2, 'succeeded', 'test', 'test-actor', '[]'::jsonb)")
        .bind("run-succeeded-1")
        .bind("emp-inventory")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO agent_run (id, employee_id, status, trigger, actor, steps) VALUES ($1, $2, 'failed', 'test', 'test-actor', '[]'::jsonb)")
        .bind("run-failed-1")
        .bind("emp-risk")
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(count_active_agent_runs(&pool).await.unwrap(), 2);
    Ok(())
}

/// `overview.agentSuccessRate`/`ops.observability.agentSuccessRate` (WS5
/// item B5): `running` runs are excluded from both counts and from the
/// denominator -- a run still in progress has no outcome yet.
#[sqlx::test(migrations = "../../migrations")]
async fn count_agent_run_outcomes_excludes_running_runs(pool: PgPool) -> sqlx::Result<()> {
    // Two succeeded, one failed, all "completed" within the 24h window
    // (ended_at defaults to now() via the fixture below); one still
    // running, which must not count toward either bucket.
    for (id, status) in [
        ("run-outcome-succeeded-1", "succeeded"),
        ("run-outcome-succeeded-2", "succeeded"),
        ("run-outcome-failed-1", "failed"),
    ] {
        sqlx::query(
            "INSERT INTO agent_run (id, employee_id, status, trigger, actor, steps, ended_at) \
             VALUES ($1, $2, $3, 'test', 'test-actor', '[]'::jsonb, now())",
        )
        .bind(id)
        .bind("emp-inventory")
        .bind(status)
        .execute(&pool)
        .await
        .unwrap();
    }
    insert_run(&pool, "run-outcome-running-1", "emp-risk").await;

    let (succeeded, failed) = count_agent_run_outcomes(&pool).await.unwrap();
    assert_eq!(succeeded, 2);
    assert_eq!(failed, 1);
    Ok(())
}

/// Zero completed runs in the window ⇒ `(0, 0)` -- the route-level
/// wrapper turns that into `None` ("no data"), not `Some(0.0)` ("0%
/// success"), but the store layer's own contract is just the raw counts.
#[sqlx::test(migrations = "../../migrations")]
async fn count_agent_run_outcomes_is_zero_zero_with_no_completed_runs(
    pool: PgPool,
) -> sqlx::Result<()> {
    insert_run(&pool, "run-outcome-only-running", "emp-inventory").await;
    let (succeeded, failed) = count_agent_run_outcomes(&pool).await.unwrap();
    assert_eq!((succeeded, failed), (0, 0));
    Ok(())
}

// ---------------------------------------------------------------------
// WS7 item G3: real, recomputed employee metrics
// ---------------------------------------------------------------------

async fn seed_metrics_employee(pool: &PgPool, name: &str, budget_limit: f64) -> String {
    let input = CreateEmployeeInput {
        name: name.to_owned(),
        purpose: "p".to_owned(),
        autonomy: "L2".to_owned(),
        allowed_tools: vec![],
        data_scope: "d".to_owned(),
        budget_limit,
        owner: None,
        prompt: None,
        schedule_cron: None,
        mode: None,
        permissions: None,
    };
    create_employee(pool, &input).await.unwrap().id
}

/// `recompute_employee_metrics` computes `budgetSpent`/`successRate`/
/// `recentRuns` from real `agent_run` rows -- a `succeeded` and a
/// `failed` run both count toward `recentRuns` and the `successRate`
/// denominator; only `succeeded` counts toward its numerator; a run
/// still `running` counts toward `recentRuns` (a real run that started)
/// but is excluded from `successRate` entirely (no outcome yet).
#[sqlx::test(migrations = "../../migrations")]
async fn recompute_employee_metrics_reflects_real_runs(pool: PgPool) -> sqlx::Result<()> {
    let employee_id = seed_metrics_employee(&pool, "metrics-employee", 1000.0).await;

    create_run(&pool, "run-metrics-1", &employee_id, "manual", "tester")
        .await
        .unwrap();
    record_run_budget(&pool, "run-metrics-1", 60.0)
        .await
        .unwrap();
    finish_run(&pool, "run-metrics-1", "succeeded")
        .await
        .unwrap();

    create_run(&pool, "run-metrics-2", &employee_id, "manual", "tester")
        .await
        .unwrap();
    record_run_budget(&pool, "run-metrics-2", 40.0)
        .await
        .unwrap();
    finish_run(&pool, "run-metrics-2", "failed").await.unwrap();

    // Still running: counts toward recent_runs, excluded from success_rate.
    create_run(&pool, "run-metrics-3", &employee_id, "manual", "tester")
        .await
        .unwrap();

    recompute_employee_metrics(&pool, &employee_id)
        .await
        .unwrap();

    let employee = get_employee_with_metrics(&pool, &employee_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(employee.budget_spent, Some(100.0));
    assert_eq!(employee.success_rate, Some(0.5));
    assert_eq!(employee.recent_runs, Some(3));
    Ok(())
}

/// Honest-null path (program requirement: "if no runs have completed, a
/// success rate is null, not 100" -- and, symmetrically, not 0 either): an
/// employee with zero terminal runs in the window has `successRate ==
/// None`, never a fabricated `Some(0.0)`. `budgetSpent`/`recentRuns` DO
/// report a real `Some(0.0)`/`Some(0)` here -- "zero rows summed" and
/// "zero rows counted" are true answers, unlike an undefined ratio.
#[sqlx::test(migrations = "../../migrations")]
async fn recompute_employee_metrics_leaves_success_rate_null_with_no_terminal_runs(
    pool: PgPool,
) -> sqlx::Result<()> {
    let employee_id = seed_metrics_employee(&pool, "no-runs-employee", 1000.0).await;
    recompute_employee_metrics(&pool, &employee_id)
        .await
        .unwrap();
    let employee = get_employee_with_metrics(&pool, &employee_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(employee.success_rate, None);
    assert_eq!(employee.approval_rate, None);
    assert_eq!(employee.budget_spent, Some(0.0));
    assert_eq!(employee.recent_runs, Some(0));
    Ok(())
}

/// `approval_rate` is computed from real `approval_item` rows linked to
/// the employee's runs -- a still-`pending` approval is excluded from
/// both the numerator and the denominator, same as a still-running run
/// for `success_rate`.
#[sqlx::test(migrations = "../../migrations")]
async fn recompute_employee_metrics_computes_approval_rate_from_real_approval_items(
    pool: PgPool,
) -> sqlx::Result<()> {
    let employee_id = seed_metrics_employee(&pool, "approval-metrics-employee", 1000.0).await;
    create_run(
        &pool,
        "run-appr-metrics-1",
        &employee_id,
        "manual",
        "tester",
    )
    .await
    .unwrap();
    insert_approval(
        &pool,
        "ap-metrics-1",
        &employee_id,
        "approval-metrics-employee",
        Some("run-appr-metrics-1"),
        "approved",
    )
    .await;
    insert_approval(
        &pool,
        "ap-metrics-2",
        &employee_id,
        "approval-metrics-employee",
        Some("run-appr-metrics-1"),
        "rejected",
    )
    .await;
    insert_approval(
        &pool,
        "ap-metrics-3",
        &employee_id,
        "approval-metrics-employee",
        Some("run-appr-metrics-1"),
        "pending",
    )
    .await;

    recompute_employee_metrics(&pool, &employee_id)
        .await
        .unwrap();
    let employee = get_employee_with_metrics(&pool, &employee_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(employee.approval_rate, Some(0.5));
    Ok(())
}

/// `list_employees`/`get_employee` never fetch the four metric columns
/// (a real, disclosed "not fetched for a list view" distinct from "no run
/// has completed yet") -- `get_employee_with_metrics` is the only
/// function that returns their real, recomputed values.
#[sqlx::test(migrations = "../../migrations")]
async fn list_and_get_employee_omit_metrics_that_get_employee_with_metrics_reports(
    pool: PgPool,
) -> sqlx::Result<()> {
    let employee_id = seed_metrics_employee(&pool, "list-vs-detail-employee", 1000.0).await;
    create_run(
        &pool,
        "run-list-vs-detail",
        &employee_id,
        "manual",
        "tester",
    )
    .await
    .unwrap();
    record_run_budget(&pool, "run-list-vs-detail", 25.0)
        .await
        .unwrap();
    finish_run(&pool, "run-list-vs-detail", "succeeded")
        .await
        .unwrap();
    recompute_employee_metrics(&pool, &employee_id)
        .await
        .unwrap();

    let via_get = get_employee(&pool, &employee_id).await.unwrap().unwrap();
    assert_eq!(via_get.budget_spent, None);
    assert_eq!(via_get.recent_runs, None);

    let via_list = list_employees(&pool).await.unwrap();
    let listed = via_list.iter().find(|e| e.id == employee_id).unwrap();
    assert_eq!(listed.budget_spent, None);

    let via_detail = get_employee_with_metrics(&pool, &employee_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(via_detail.budget_spent, Some(25.0));
    assert_eq!(via_detail.recent_runs, Some(1));
    Ok(())
}
