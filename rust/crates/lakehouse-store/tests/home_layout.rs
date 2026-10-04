//! Integration tests for `lakehouse_store::home_layout` against a real
//! Postgres (`#[sqlx::test]`, see `tests/maintenance_policy.rs` for how the
//! database is provided).

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap runs for this test binary.
use lakehouse_test_support as _;

use lakehouse_store::home_layout::{delete, get, upsert};
use serde_json::{Value, json};
use sqlx::PgPool;

fn layout(cards: &[&str]) -> Value {
    json!({ "cards": cards, "shortcuts": ["new-query"], "previewBoardId": null })
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_user_who_never_saved_has_no_layout(pool: PgPool) -> sqlx::Result<()> {
    assert_eq!(get(&pool, "owner-a").await.expect("get"), None);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn upsert_then_get_round_trips_and_a_second_upsert_replaces(
    pool: PgPool,
) -> sqlx::Result<()> {
    let first = layout(&["recent", "sources"]);
    assert_eq!(
        upsert(&pool, "owner-a", &first).await.expect("upsert"),
        first
    );
    assert_eq!(get(&pool, "owner-a").await.expect("get"), Some(first));

    let second = layout(&["pipeline-runs"]);
    upsert(&pool, "owner-a", &second)
        .await
        .expect("upsert again");
    assert_eq!(get(&pool, "owner-a").await.expect("get"), Some(second));

    let (rows,): (i64,) = sqlx::query_as("SELECT count(*) FROM home_layout")
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        rows, 1,
        "the second save replaces the row, it does not add one"
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn one_owner_cannot_read_or_delete_another_owners_layout(pool: PgPool) -> sqlx::Result<()> {
    let mine = layout(&["recent"]);
    upsert(&pool, "owner-a", &mine).await.expect("upsert");

    assert_eq!(get(&pool, "owner-b").await.expect("get"), None);
    assert!(!delete(&pool, "owner-b").await.expect("delete"));
    assert_eq!(
        get(&pool, "owner-a").await.expect("get"),
        Some(mine),
        "another owner's delete must not touch this one"
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn delete_returns_the_owner_to_the_default_and_is_idempotent(
    pool: PgPool,
) -> sqlx::Result<()> {
    upsert(&pool, "owner-a", &layout(&["recent"]))
        .await
        .expect("upsert");
    assert!(delete(&pool, "owner-a").await.expect("delete"));
    assert_eq!(get(&pool, "owner-a").await.expect("get"), None);
    assert!(!delete(&pool, "owner-a").await.expect("delete again"));
    Ok(())
}
