//! Integration tests for `lakehouse_store::agents` against a real
//! Postgres.
//!
//! # Why every test here is `#[ignore]`d
//!
//! Same reason as `tests/connectors.rs`: `#[sqlx::test]` needs a live
//! Postgres reachable via `DATABASE_URL`. Run explicitly with:
//!
//! ```sh
//! docker compose up -d postgres
//! DATABASE_URL=postgres://lakehouse:lakehouse@localhost:5432/lakehouse \
//!   cargo test -p lakehouse-store -- --ignored
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_store::StoreError;
use lakehouse_store::agents::{
    CreateEmployeeInput, Decision, create_employee, decide_approval, get_employee, get_run,
    list_approvals, list_employees, list_runs, list_tools, list_workflows, resume_employee,
    revoke_employee, suspend_employee,
};
use sqlx::PgPool;

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn seed_populates_every_agents_list(pool: PgPool) -> sqlx::Result<()> {
    assert_eq!(list_workflows(&pool).await.unwrap().len(), 2);
    assert_eq!(list_employees(&pool).await.unwrap().len(), 2);
    assert_eq!(list_tools(&pool).await.unwrap().len(), 3);
    assert_eq!(list_runs(&pool, None).await.unwrap().len(), 2);
    assert_eq!(list_approvals(&pool, None).await.unwrap().len(), 3);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn list_runs_filters_by_employee(pool: PgPool) -> sqlx::Result<()> {
    let runs = list_runs(&pool, Some("emp-risk")).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, "run-risk-01");
    Ok(())
}

/// A run's `approvals[]` is derived from `approval_item.run_id`, not
/// duplicated state — this is the regression test for that join.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn run_embeds_its_approvals(pool: PgPool) -> sqlx::Result<()> {
    let run = get_run(&pool, "run-col-01").await.unwrap().unwrap();
    assert_eq!(run.approvals.len(), 2);
    assert!(
        run.approvals
            .iter()
            .any(|a| a.id == "ap-01" && a.status == "pending")
    );
    assert!(
        run.approvals
            .iter()
            .any(|a| a.id == "ap-02" && a.status == "approved")
    );
    assert_eq!(run.steps.len(), 3);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn get_employee_none_for_unknown_id(pool: PgPool) -> sqlx::Result<()> {
    assert!(get_employee(&pool, "emp-nope").await.unwrap().is_none());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn duplicate_employee_name_is_a_conflict(pool: PgPool) -> sqlx::Result<()> {
    let input = CreateEmployeeInput {
        name: "inventory-copilot".to_owned(),
        purpose: "p".to_owned(),
        autonomy: "L1".to_owned(),
        allowed_tools: vec![],
        data_scope: "d".to_owned(),
        budget_limit: 0.0,
        owner: None,
    };
    let err = create_employee(&pool, &input).await.unwrap_err();
    assert!(matches!(err, StoreError::Conflict));
    Ok(())
}

/// Suspend -> resume -> revoke each transition `status` and round-trip
/// through `get_employee`.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
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
#[ignore = "requires a live Postgres; see module doc comment"]
async fn suspend_unknown_employee_is_not_found(pool: PgPool) -> sqlx::Result<()> {
    let err = suspend_employee(&pool, "emp-nope").await.unwrap_err();
    assert!(matches!(err, StoreError::NotFound));
    Ok(())
}

/// The core approval-lifecycle guarantee the task brief calls out: an
/// already-decided approval cannot be re-decided.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn deciding_an_already_decided_approval_is_a_conflict(pool: PgPool) -> sqlx::Result<()> {
    // ap-02 is seeded already "approved".
    let err = decide_approval(&pool, "ap-02", Decision::Rejected, None)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Conflict));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn deciding_a_pending_approval_stamps_decided_at_and_status(
    pool: PgPool,
) -> sqlx::Result<()> {
    let decided = decide_approval(&pool, "ap-01", Decision::Approved, Some("looks fine"))
        .await
        .unwrap();
    assert_eq!(decided.status, "approved");
    assert!(decided.decided_at.is_some());
    assert_eq!(decided.comment.as_deref(), Some("looks fine"));

    // Reflected in the owning run's embedded approvals too.
    let run = get_run(&pool, "run-col-01").await.unwrap().unwrap();
    assert!(
        run.approvals
            .iter()
            .any(|a| a.id == "ap-01" && a.status == "approved")
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn decide_unknown_approval_is_not_found(pool: PgPool) -> sqlx::Result<()> {
    let err = decide_approval(&pool, "ap-nope", Decision::Approved, None)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::NotFound));
    Ok(())
}
