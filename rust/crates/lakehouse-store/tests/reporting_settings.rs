//! Integration tests for `lakehouse_store::reporting_settings` against a real
//! Postgres (`#[sqlx::test]`, see `tests/maintenance_policy.rs` for how the
//! database is provided).

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap runs for this test binary.
use lakehouse_test_support as _;

use lakehouse_store::reporting_settings::{ReportingSettings, get, upsert};
use sqlx::PgPool;

#[sqlx::test(migrations = "../../migrations")]
async fn a_deployment_that_never_saved_has_no_row(pool: PgPool) -> sqlx::Result<()> {
    assert_eq!(get(&pool).await.expect("get"), None);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_save_round_trips_and_a_second_save_replaces_the_one_row(
    pool: PgPool,
) -> sqlx::Result<()> {
    let first = upsert(&pool, "Europe/Berlin", "sunday", "user-a")
        .await
        .expect("upsert");
    assert_eq!(
        first,
        ReportingSettings {
            time_zone: "Europe/Berlin".to_owned(),
            week_start: "sunday".to_owned()
        }
    );
    assert_eq!(get(&pool).await.expect("get"), Some(first));

    upsert(&pool, "Asia/Jakarta", "monday", "user-b")
        .await
        .expect("upsert again");
    let (rows, who): (i64, String) =
        sqlx::query_as("SELECT count(*), max(updated_by) FROM reporting_settings")
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        rows, 1,
        "the second save replaces the row, it does not add one"
    );
    assert_eq!(who, "user-b");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_table_refuses_a_third_week_start_and_a_zone_that_is_not_shaped_like_one(
    pool: PgPool,
) -> sqlx::Result<()> {
    for (zone, week) in [
        ("Asia/Jakarta", "friday"),
        ("Asia/Jakarta'; DROP TABLE x; --", "monday"),
        ("", "monday"),
        ("Asia//Jakarta", "monday"),
    ] {
        assert!(
            upsert(&pool, zone, week, "u").await.is_err(),
            "{zone:?} {week:?} should be refused by the table's CHECKs"
        );
    }
    assert_eq!(get(&pool).await.expect("get"), None, "nothing was stored");
    Ok(())
}
