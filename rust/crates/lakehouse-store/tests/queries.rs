//! Integration tests for `lakehouse_store::queries` against a real
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

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap actually runs for this test binary (an
// unreferenced dev-dependency's rlib member can otherwise be dropped
// by the linker before its ctor section is ever considered).
use lakehouse_test_support as _;

use lakehouse_store::audit::{NewAuditEvent, insert as insert_audit_event};
use lakehouse_store::queries::{
    RecordHistoryInput, get_history_item, list_history, list_saved, record_history,
};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

/// The seed lands two saved queries, pointed at marts this lakehouse
/// actually serves (`0027_query_ownership.sql` repointed them — the
/// original fixtures named tables that do not exist here, so the only two
/// examples a new user could open both failed on Run).
#[sqlx::test(migrations = "../../migrations")]
async fn seed_populates_saved_queries(pool: PgPool) -> sqlx::Result<()> {
    let saved = list_saved(&pool).await.unwrap();
    assert_eq!(saved.len(), 2);
    assert!(
        saved
            .iter()
            .any(|q| q.title == "Top destinations by visitors")
    );
    assert!(saved.iter().all(|q| q.sql.contains("serving.mart_")));
    Ok(())
}

/// `record_history` writes a row `list_history` then returns, most recent
/// first — the round trip `routes::query::run` depends on.
#[sqlx::test(migrations = "../../migrations")]
async fn record_history_round_trips_through_list(pool: PgPool) -> sqlx::Result<()> {
    let owner = Uuid::new_v4();
    assert!(list_history(&pool, owner).await.unwrap().is_empty());

    record_history(
        &pool,
        &RecordHistoryInput {
            id: "q-1",
            sql: "SELECT 1",
            user: "Bootstrap Admin",
            owner_id: Some(owner),
            status: "completed",
            duration_ms: 42,
            scanned_bytes: 1024,
            cost_units: 0.5,
            workload_class: "hot-analytics",
            engine: "hot-store",
            cache_assisted: false,
        },
    )
    .await
    .unwrap();

    let history = list_history(&pool, owner).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, "q-1");
    assert_eq!(history[0].sql, "SELECT 1");
    assert_eq!(history[0].status, "completed");
    assert!(history[0].at.ends_with('Z'));

    // Another user's history is not this user's: the rows are private to
    // whoever ran them.
    assert!(
        list_history(&pool, Uuid::new_v4())
            .await
            .unwrap()
            .is_empty()
    );
    Ok(())
}

/// Recording the same id twice must not fail or duplicate the row — a
/// caller retrying, or a genuinely reused id, degrades to a no-op rather
/// than a 500 that would (per `routes::query::run`'s contract) still not
/// surface to the caller, but would spam logs unnecessarily.
#[sqlx::test(migrations = "../../migrations")]
async fn record_history_is_idempotent_per_id(pool: PgPool) -> sqlx::Result<()> {
    let owner = Uuid::new_v4();
    let input = RecordHistoryInput {
        id: "q-dup",
        sql: "SELECT 1",
        user: "Bootstrap Admin",
        owner_id: Some(owner),
        status: "completed",
        duration_ms: 1,
        scanned_bytes: 1,
        cost_units: 0.1,
        workload_class: "hot-analytics",
        engine: "hot-store",
        cache_assisted: false,
    };
    record_history(&pool, &input).await.unwrap();
    record_history(&pool, &input).await.unwrap();

    let history = list_history(&pool, owner).await.unwrap();
    assert_eq!(history.len(), 1);
    Ok(())
}

/// A `NewAuditEvent` fixture pinned to one query-history resource, mirroring
/// the shape `routes::query::run` would eventually write once WS5 records
/// real query-run audit events.
fn query_history_audit_event(resource_id: &str, action: &str) -> NewAuditEvent {
    NewAuditEvent {
        action: action.to_owned(),
        resource_kind: Some("query_history".to_owned()),
        resource_id: Some(resource_id.to_owned()),
        outcome: "executed".to_owned(),
        ..NewAuditEvent::default()
    }
}

/// WS1 task 1.14 (judge finding J12): no producer mints a synthetic
/// `aud-query-<id>` string anymore — `auditEventId` must be absent until a
/// real `audit_event` row exists for the query-history resource.
#[sqlx::test(migrations = "../../migrations")]
async fn history_item_has_no_audit_id_without_a_real_event(pool: PgPool) -> sqlx::Result<()> {
    let owner = Uuid::new_v4();
    record_history(
        &pool,
        &RecordHistoryInput {
            id: "q-no-audit",
            sql: "SELECT 1",
            user: "anonymous",
            owner_id: Some(owner),
            status: "completed",
            duration_ms: 1,
            scanned_bytes: 1,
            cost_units: 0.1,
            workload_class: "hot-analytics",
            engine: "hot-store",
            cache_assisted: false,
        },
    )
    .await
    .unwrap();

    let history = list_history(&pool, owner).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].audit_event_id, None);
    Ok(())
}

/// Once a real `audit_event` row names the history row's resource, the
/// list resolves the id through the lateral join — the "View audit" link
/// this task is fixing.
#[sqlx::test(migrations = "../../migrations")]
async fn history_item_resolves_the_real_audit_event_id(pool: PgPool) -> sqlx::Result<()> {
    let owner = Uuid::new_v4();
    record_history(
        &pool,
        &RecordHistoryInput {
            id: "q-audit",
            sql: "SELECT 1",
            user: "anonymous",
            owner_id: Some(owner),
            status: "completed",
            duration_ms: 1,
            scanned_bytes: 1,
            cost_units: 0.1,
            workload_class: "hot-analytics",
            engine: "hot-store",
            cache_assisted: false,
        },
    )
    .await
    .unwrap();

    let event = insert_audit_event(&pool, query_history_audit_event("q-audit", "query_sql"))
        .await
        .unwrap();

    let history = list_history(&pool, owner).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(
        history[0].audit_event_id.as_deref(),
        Some(event.id.as_str())
    );
    Ok(())
}

/// Insert an `audit_event` row directly, with an explicit `at`, bypassing
/// `audit::insert`'s `now()` default. Test-only fixture helper: it exists
/// so [`history_item_uses_the_newest_of_several_events_without_duplicating_the_row`]
/// can assert "newest wins" deterministically instead of relying on two
/// `now()`-stamped inserts landing in different microseconds (they are not
/// guaranteed to — see `tests/audit.rs`'s `list_orders_newest_first`, which
/// documents the same tie risk for `audit::insert`).
async fn insert_audit_event_at(pool: &PgPool, id: &str, resource_id: &str, at: OffsetDateTime) {
    sqlx::query(
        "INSERT INTO audit_event (id, at, action, resource_kind, resource_id, outcome) \
         VALUES ($1, $2, 'query_sql', 'query_history', $3, 'executed')",
    )
    .bind(id)
    .bind(at)
    .bind(resource_id)
    .execute(pool)
    .await
    .unwrap();
}

/// Several `audit_event` rows can exist for one query-history resource
/// (e.g. a re-run of the audit pipeline). The lateral join must pick the
/// newest one and must never duplicate the history row — a bare `LEFT
/// JOIN` would return one row per matching event.
#[sqlx::test(migrations = "../../migrations")]
async fn history_item_uses_the_newest_of_several_events_without_duplicating_the_row(
    pool: PgPool,
) -> sqlx::Result<()> {
    let owner = Uuid::new_v4();
    record_history(
        &pool,
        &RecordHistoryInput {
            id: "q-multi",
            sql: "SELECT 1",
            user: "anonymous",
            owner_id: Some(owner),
            status: "completed",
            duration_ms: 1,
            scanned_bytes: 1,
            cost_units: 0.1,
            workload_class: "hot-analytics",
            engine: "hot-store",
            cache_assisted: false,
        },
    )
    .await
    .unwrap();

    let now = OffsetDateTime::now_utc();
    insert_audit_event_at(&pool, "audit-multi-older", "q-multi", now).await;
    insert_audit_event_at(
        &pool,
        "audit-multi-newer",
        "q-multi",
        now + time::Duration::seconds(60),
    )
    .await;

    let history = list_history(&pool, owner).await.unwrap();
    assert_eq!(history.len(), 1, "the lateral join must not duplicate rows");
    assert_eq!(
        history[0].audit_event_id.as_deref(),
        Some("audit-multi-newer")
    );
    Ok(())
}

/// WS2 §4 — `record_history` carries the real engine and a display-name
/// `user`; ownership scoping lives in the separate `owner_id` column
/// (`0046_query_ownership.sql`), which is what `list_history`'s `owner_id`
/// argument filters on — rewritten from the pre-0046 version of this test,
/// which asserted `user` itself held the principal's uuid (superseded once
/// `owner_id` became its own column).
#[sqlx::test(migrations = "../../migrations")]
async fn record_history_carries_the_real_engine_and_is_scoped_by_owner_id(
    pool: PgPool,
) -> sqlx::Result<()> {
    let owner = Uuid::new_v4();
    record_history(
        &pool,
        &RecordHistoryInput {
            id: "q-real-user",
            sql: "SELECT 1",
            user: "Real User",
            owner_id: Some(owner),
            status: "completed",
            duration_ms: 1,
            scanned_bytes: 1,
            cost_units: 0.1,
            workload_class: "hot-analytics",
            engine: "trino",
            cache_assisted: false,
        },
    )
    .await
    .unwrap();

    let history = list_history(&pool, owner).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].user, "Real User");
    assert_eq!(history[0].engine, "trino");
    Ok(())
}

/// [`get_history_item`] fetches the same row [`list_history`] would, by
/// id, including the lateral `audit_event` resolution.
#[sqlx::test(migrations = "../../migrations")]
async fn get_history_item_fetches_one_row_by_id(pool: PgPool) -> sqlx::Result<()> {
    record_history(
        &pool,
        &RecordHistoryInput {
            id: "q-get-one",
            sql: "SELECT 1",
            user: "8f14e45f-ceea-467e-bd0d-8fbcae0b0000",
            owner_id: Some(Uuid::new_v4()),
            status: "completed",
            duration_ms: 1,
            scanned_bytes: 1,
            cost_units: 0.1,
            workload_class: "hot-analytics",
            engine: "clickhouse",
            cache_assisted: false,
        },
    )
    .await
    .unwrap();
    let event = insert_audit_event(&pool, query_history_audit_event("q-get-one", "query_sql"))
        .await
        .unwrap();

    let item = get_history_item(&pool, "q-get-one")
        .await
        .unwrap()
        .expect("row must exist");
    assert_eq!(item.id, "q-get-one");
    assert_eq!(item.user, "8f14e45f-ceea-467e-bd0d-8fbcae0b0000");
    assert_eq!(item.engine, "clickhouse");
    assert_eq!(item.audit_event_id.as_deref(), Some(event.id.as_str()));
    Ok(())
}

/// A non-existent id is `Ok(None)`, not an error — the caller (a later
/// task's ownership check) distinguishes "no such query" from a database
/// failure.
#[sqlx::test(migrations = "../../migrations")]
async fn get_history_item_returns_none_for_an_unknown_id(pool: PgPool) -> sqlx::Result<()> {
    let item = get_history_item(&pool, "q-does-not-exist").await.unwrap();
    assert_eq!(item, None);
    Ok(())
}
