//! Integration tests for `lakehouse_store::queries` against a real
//! Postgres.
//!
//! # Why every test here is `#[ignore]`d
//!
//! Same reason as `tests/identity.rs`/`tests/governance.rs`: `#[sqlx::test]`
//! needs a live Postgres reachable via `DATABASE_URL`. Run explicitly with:
//!
//! ```sh
//! docker compose up -d postgres
//! DATABASE_URL=postgres://lakehouse:lakehouse@localhost:5432/lakehouse \
//!   cargo test -p lakehouse-store -- --ignored
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_store::queries::{
    CreateCollaborationProjectInput, RecordHistoryInput, create_collaboration_project,
    list_collaboration, list_history, list_saved, record_history,
};
use sqlx::PgPool;

/// The seed lands the two `mock/queries.ts` saved-query fixtures.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn seed_populates_saved_queries(pool: PgPool) -> sqlx::Result<()> {
    let saved = list_saved(&pool).await.unwrap();
    assert_eq!(saved.len(), 2);
    assert!(saved.iter().any(|q| q.title == "Revenue by region"));
    Ok(())
}

/// The seed lands the two `mock/queries.ts` collaboration-project fixtures,
/// and a create adds a third with `members` set from the collaborator
/// count.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn seed_and_create_collaboration_projects(pool: PgPool) -> sqlx::Result<()> {
    let seeded = list_collaboration(&pool).await.unwrap();
    assert_eq!(seeded.len(), 2);

    let created = create_collaboration_project(
        &pool,
        &CreateCollaborationProjectInput {
            name: "Growth pod".to_owned(),
            collaborators: vec!["Rina".to_owned(), "Bayu".to_owned(), "Dewi".to_owned()],
            description: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(created.members, 3);
    assert_eq!(created.description, "Collaborators: Rina, Bayu, Dewi");

    let all = list_collaboration(&pool).await.unwrap();
    assert_eq!(all.len(), 3);
    Ok(())
}

/// `record_history` writes a row `list_history` then returns, most recent
/// first — the round trip `routes::query::run` depends on.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn record_history_round_trips_through_list(pool: PgPool) -> sqlx::Result<()> {
    assert!(list_history(&pool).await.unwrap().is_empty());

    record_history(
        &pool,
        &RecordHistoryInput {
            id: "q-1",
            sql: "SELECT 1",
            user: "anonymous",
            status: "completed",
            duration_ms: 42,
            scanned_bytes: 1024,
            cost_units: 0.5,
            workload_class: "hot-analytics",
            engine: "hot-store",
            cache_assisted: false,
            audit_event_id: Some("aud-query-q-1"),
        },
    )
    .await
    .unwrap();

    let history = list_history(&pool).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, "q-1");
    assert_eq!(history[0].sql, "SELECT 1");
    assert_eq!(history[0].status, "completed");
    assert_eq!(history[0].audit_event_id.as_deref(), Some("aud-query-q-1"));
    assert!(history[0].at.ends_with('Z'));
    Ok(())
}

/// Recording the same id twice must not fail or duplicate the row — a
/// caller retrying, or a genuinely reused id, degrades to a no-op rather
/// than a 500 that would (per `routes::query::run`'s contract) still not
/// surface to the caller, but would spam logs unnecessarily.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn record_history_is_idempotent_per_id(pool: PgPool) -> sqlx::Result<()> {
    let input = RecordHistoryInput {
        id: "q-dup",
        sql: "SELECT 1",
        user: "anonymous",
        status: "completed",
        duration_ms: 1,
        scanned_bytes: 1,
        cost_units: 0.1,
        workload_class: "hot-analytics",
        engine: "hot-store",
        cache_assisted: false,
        audit_event_id: None,
    };
    record_history(&pool, &input).await.unwrap();
    record_history(&pool, &input).await.unwrap();

    let history = list_history(&pool).await.unwrap();
    assert_eq!(history.len(), 1);
    Ok(())
}
