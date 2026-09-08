//! Integration tests for `lakehouse_store::audit` against a real Postgres.
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

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap actually runs for this test binary (an
// unreferenced dev-dependency's rlib member can otherwise be dropped
// by the linker before its ctor section is ever considered).
use lakehouse_test_support as _;

use lakehouse_store::StoreError;
use lakehouse_store::audit::{AuditFilter, NewAuditEvent, insert, list};
use serde_json::json;
use sqlx::PgPool;

fn sample(action: &str) -> NewAuditEvent {
    NewAuditEvent {
        principal_id: Some("user-1".to_owned()),
        principal_kind: Some("user".to_owned()),
        actor_label: Some("Rina Wijaya".to_owned()),
        action: action.to_owned(),
        resource_kind: Some("connector".to_owned()),
        resource_id: Some("conn-1".to_owned()),
        args: Some(json!({ "foo": "bar" })),
        outcome: "executed".to_owned(),
        detail: None,
        run_id: None,
        approval_id: None,
        session_id: Some("sess-1".to_owned()),
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn insert_then_read_back(pool: PgPool) -> sqlx::Result<()> {
    let written = insert(&pool, sample("query_sql")).await.unwrap();
    assert!(written.id.starts_with("audit-"));
    assert_eq!(written.action, "query_sql");
    assert_eq!(written.args, json!({ "foo": "bar" }));
    assert_eq!(written.outcome, "executed");

    let events = list(&pool, AuditFilter::default()).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id, written.id);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_orders_newest_first(pool: PgPool) -> sqlx::Result<()> {
    let first = insert(&pool, sample("first")).await.unwrap();
    let second = insert(&pool, sample("second")).await.unwrap();
    let third = insert(&pool, sample("third")).await.unwrap();

    let events = list(&pool, AuditFilter::default()).await.unwrap();
    assert_eq!(events.len(), 3);
    // All three rows are written with `at DEFAULT now()` from the same
    // statement-timestamp burst, so `id` order (insertion order) is used
    // as the tiebreak-independent check instead of asserting a strict
    // timestamp ordering that could tie.
    let ids: Vec<&str> = events.iter().map(|e| e.id.as_str()).collect();
    assert!(ids.contains(&third.id.as_str()));
    assert!(ids.contains(&second.id.as_str()));
    assert!(ids.contains(&first.id.as_str()));
    // Newest-first: the most recently inserted row's `at` must not be
    // earlier than an earlier one's.
    assert!(events[0].at >= events[1].at);
    assert!(events[1].at >= events[2].at);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_filters_by_resource(pool: PgPool) -> sqlx::Result<()> {
    let mut a = sample("a");
    a.resource_kind = Some("connector".to_owned());
    a.resource_id = Some("conn-1".to_owned());
    insert(&pool, a).await.unwrap();

    let mut b = sample("b");
    b.resource_kind = Some("alert_rule".to_owned());
    b.resource_id = Some("alert-9".to_owned());
    insert(&pool, b).await.unwrap();

    let filtered = list(
        &pool,
        AuditFilter {
            resource_kind: Some("connector".to_owned()),
            resource_id: Some("conn-1".to_owned()),
            ..AuditFilter::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].action, "a");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_filters_by_principal(pool: PgPool) -> sqlx::Result<()> {
    let mut mine = sample("mine");
    mine.principal_id = Some("user-1".to_owned());
    insert(&pool, mine).await.unwrap();

    let mut other = sample("other");
    other.principal_id = Some("user-2".to_owned());
    insert(&pool, other).await.unwrap();

    let filtered = list(
        &pool,
        AuditFilter {
            principal_id: Some("user-2".to_owned()),
            ..AuditFilter::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].action, "other");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_respects_limit(pool: PgPool) -> sqlx::Result<()> {
    for i in 0..5 {
        insert(&pool, sample(&format!("action-{i}"))).await.unwrap();
    }
    let limited = list(
        &pool,
        AuditFilter {
            limit: 2,
            ..AuditFilter::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(limited.len(), 2);
    Ok(())
}

/// `run_id` references `agent_run(id) ON DELETE SET NULL` — deleting the
/// referenced run must null out the audit row's `run_id`, not fail the
/// delete or orphan the audit row.
#[sqlx::test(migrations = "../../migrations")]
async fn deleting_referenced_run_nulls_out_run_id(pool: PgPool) -> sqlx::Result<()> {
    let mut event = sample("run_tool");
    event.run_id = Some("run-col-01".to_owned());
    let written = insert(&pool, event).await.unwrap();
    assert_eq!(written.run_id.as_deref(), Some("run-col-01"));

    sqlx::query("DELETE FROM agent_run WHERE id = $1")
        .bind("run-col-01")
        .execute(&pool)
        .await
        .unwrap();

    let events = list(&pool, AuditFilter::default()).await.unwrap();
    let row = events.iter().find(|e| e.id == written.id).unwrap();
    assert_eq!(row.run_id, None);
    Ok(())
}

/// Same `ON DELETE SET NULL` guarantee for `approval_id` ->
/// `approval_item(id)`.
#[sqlx::test(migrations = "../../migrations")]
async fn deleting_referenced_approval_nulls_out_approval_id(pool: PgPool) -> sqlx::Result<()> {
    let mut event = sample("decide_approval");
    event.approval_id = Some("ap-01".to_owned());
    let written = insert(&pool, event).await.unwrap();
    assert_eq!(written.approval_id.as_deref(), Some("ap-01"));

    sqlx::query("DELETE FROM approval_item WHERE id = $1")
        .bind("ap-01")
        .execute(&pool)
        .await
        .unwrap();

    let events = list(&pool, AuditFilter::default()).await.unwrap();
    let row = events.iter().find(|e| e.id == written.id).unwrap();
    assert_eq!(row.approval_id, None);
    Ok(())
}

/// An unknown `run_id` is a foreign-key violation, not a silent insert.
#[sqlx::test(migrations = "../../migrations")]
async fn insert_with_unknown_run_id_is_a_foreign_key_violation(pool: PgPool) -> sqlx::Result<()> {
    let mut event = sample("nope");
    event.run_id = Some("run-does-not-exist".to_owned());
    let err = insert(&pool, event).await.unwrap_err();
    assert!(matches!(err, StoreError::ForeignKeyViolation));
    Ok(())
}
