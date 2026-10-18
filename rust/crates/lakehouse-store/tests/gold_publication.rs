//! Integration tests for `gold_publication.rs` (DATA-1, migration
//! `0054`): the store half of the per-mart publish-to-`Iceberg` switch.
//!
//! # Postgres backing
//!
//! `#[sqlx::test(migrations = "../../migrations")]` tests, backed by the
//! `lakehouse-test-support` testcontainer bootstrap — same arrangement as
//! `pipelines.rs` (see that file's header for the why).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_test_support as _;

use lakehouse_store::gold_publication;
use sqlx::PgPool;
use uuid::Uuid;

/// A mart that was never enabled reads as `None` — the "off by default"
/// state the detail route and the scheduler both rely on.
#[sqlx::test(migrations = "../../migrations")]
async fn get_is_none_for_a_mart_that_was_never_enabled(pool: PgPool) -> sqlx::Result<()> {
    let got = gold_publication::get(&pool, "never_touched")
        .await
        .expect("get on an untouched mart should succeed");
    assert!(got.is_none(), "a missing row means off, not an error");
    Ok(())
}

/// Enabling inserts a row and flipping updates it in place: one row per
/// mart, `updated_by`/`updated_at` moved by the second write, and the
/// `enabled` value the last writer chose wins.
#[sqlx::test(migrations = "../../migrations")]
async fn upsert_inserts_then_flips_in_place(pool: PgPool) -> sqlx::Result<()> {
    let first_actor = Uuid::new_v4();

    let first = gold_publication::upsert(&pool, "orders_rollup", true, first_actor)
        .await
        .expect("first upsert should insert");
    assert_eq!(first.mart, "orders_rollup");
    assert!(first.enabled);
    assert_eq!(first.updated_by, first_actor);

    let second_actor = Uuid::new_v4();
    let second = gold_publication::upsert(&pool, "orders_rollup", false, second_actor)
        .await
        .expect("second upsert should update in place");
    assert!(!second.enabled, "the flip must reach the returned row");
    assert_eq!(second.updated_by, second_actor);
    assert!(
        second.updated_at >= first.updated_at,
        "updated_at must not go backwards: {} -> {}",
        first.updated_at,
        second.updated_at
    );

    // Exactly one row: the flip updated, it did not append.
    let got = gold_publication::get(&pool, "orders_rollup")
        .await
        .expect("get after the flip should succeed")
        .expect("the row must still exist after switching off");
    assert!(!got.enabled);
    assert_eq!(got.updated_by, second_actor);
    Ok(())
}

/// `list_enabled` returns only enabled rows — a switched-off mart stays
/// in the table but must never reach the scheduler.
#[sqlx::test(migrations = "../../migrations")]
async fn list_enabled_returns_only_enabled_rows(pool: PgPool) -> sqlx::Result<()> {
    let actor = Uuid::new_v4();
    gold_publication::upsert(&pool, "mart_on_a", true, actor)
        .await
        .expect("first enable should insert");
    gold_publication::upsert(&pool, "mart_off", true, actor)
        .await
        .expect("second enable should insert");
    gold_publication::upsert(&pool, "mart_on_b", true, actor)
        .await
        .expect("third enable should insert");
    gold_publication::upsert(&pool, "mart_off", false, actor)
        .await
        .expect("the flip off should update in place");

    let enabled: Vec<String> = gold_publication::list_enabled(&pool)
        .await
        .expect("list_enabled should succeed")
        .into_iter()
        .map(|p| p.mart)
        .collect();
    assert_eq!(
        enabled,
        vec!["mart_on_a".to_owned(), "mart_on_b".to_owned()],
        "only enabled rows, ordered by mart name"
    );
    Ok(())
}
