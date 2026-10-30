//! Integration tests for `lakehouse_store::capture_lag` against a real Postgres
//! (`SRC-4` slice 4, migration `0118`; same setup as `tests/connector_run.rs`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_test_support as _;

use lakehouse_store::StoreError;
use lakehouse_store::capture_lag::{
    flip_lag_alert_open, get_lag_limit, list_tables, record_backlog, record_last_change,
    set_lag_limit,
};
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};

const PG: &str = "conn-pg-lakehouse";

/// A backlog report creates the row; a caught-up report moves `caught_up_at`
/// and a later non-caught-up report keeps it (`GREATEST`, never backwards).
#[sqlx::test(migrations = "../../migrations")]
async fn a_backlog_report_keeps_the_last_moment_the_table_was_caught_up(
    pool: PgPool,
) -> sqlx::Result<()> {
    let t0 = OffsetDateTime::now_utc().replace_nanosecond(0).unwrap() - Duration::minutes(5);
    record_backlog(&pool, PG, "public.orders", 0, true, t0)
        .await
        .unwrap();
    record_backlog(
        &pool,
        PG,
        "public.orders",
        40,
        false,
        t0 + Duration::seconds(30),
    )
    .await
    .unwrap();
    let tables = list_tables(&pool, PG).await.unwrap();
    assert_eq!(tables.len(), 1);
    let row = &tables[0];
    assert_eq!(row.table_name, "public.orders");
    assert_eq!(row.backlog, Some(40));
    assert_eq!(row.observed_at, Some(t0 + Duration::seconds(30)));
    assert_eq!(
        row.caught_up_at,
        Some(t0),
        "an older caught-up time is kept"
    );
    assert_eq!(row.last_source_ts, None, "nothing was reported for it");
    Ok(())
}

/// The last-change time never moves backwards and does not set `observed_at`.
#[sqlx::test(migrations = "../../migrations")]
async fn the_last_change_time_never_moves_backwards(pool: PgPool) -> sqlx::Result<()> {
    let t0 = OffsetDateTime::now_utc().replace_nanosecond(0).unwrap() - Duration::minutes(5);
    record_last_change(&pool, PG, "public.orders", t0)
        .await
        .unwrap();
    record_last_change(&pool, PG, "public.orders", t0 - Duration::seconds(9))
        .await
        .unwrap();
    let row = list_tables(&pool, PG).await.unwrap().remove(0);
    assert_eq!(row.last_source_ts, Some(t0));
    assert_eq!(row.observed_at, None);
    assert_eq!(row.backlog, None);
    Ok(())
}

/// An unknown connector is `NotFound`, not a foreign-key error.
#[sqlx::test(migrations = "../../migrations")]
async fn a_report_for_an_unknown_connector_is_not_found(pool: PgPool) -> sqlx::Result<()> {
    let now = OffsetDateTime::now_utc();
    let err = record_backlog(&pool, "no-such", "t", 0, true, now)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::NotFound));
    let err = record_last_change(&pool, "no-such", "t", now)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::NotFound));
    Ok(())
}

/// The limit defaults to 15, can be set within 1..=1440 and the column refuses
/// anything else.
#[sqlx::test(migrations = "../../migrations")]
async fn the_lag_limit_defaults_to_fifteen_and_the_column_refuses_out_of_range(
    pool: PgPool,
) -> sqlx::Result<()> {
    let limit = get_lag_limit(&pool, PG).await.unwrap();
    assert_eq!((limit.minutes, limit.open), (15, false));
    set_lag_limit(&pool, PG, 1).await.unwrap();
    assert_eq!(get_lag_limit(&pool, PG).await.unwrap().minutes, 1);
    assert!(set_lag_limit(&pool, PG, 0).await.is_err());
    assert!(set_lag_limit(&pool, PG, 1441).await.is_err());
    assert!(matches!(
        set_lag_limit(&pool, "no-such", 5).await,
        Err(StoreError::NotFound)
    ));
    Ok(())
}

/// Only the call that changes the flag gets `true`: that is what keeps an alert
/// to once per crossing.
#[sqlx::test(migrations = "../../migrations")]
async fn only_one_caller_wins_each_flip_of_the_open_flag(pool: PgPool) -> sqlx::Result<()> {
    assert!(flip_lag_alert_open(&pool, PG, false).await.unwrap());
    assert!(!flip_lag_alert_open(&pool, PG, false).await.unwrap());
    assert!(get_lag_limit(&pool, PG).await.unwrap().open);
    assert!(flip_lag_alert_open(&pool, PG, true).await.unwrap());
    assert!(!flip_lag_alert_open(&pool, PG, true).await.unwrap());
    Ok(())
}

/// Deleting the connector deletes its table rows.
#[sqlx::test(migrations = "../../migrations")]
async fn deleting_a_connector_deletes_its_capture_rows(pool: PgPool) -> sqlx::Result<()> {
    record_backlog(
        &pool,
        PG,
        "public.orders",
        0,
        true,
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    sqlx::query("DELETE FROM connector WHERE id = $1")
        .bind(PG)
        .execute(&pool)
        .await?;
    assert_eq!(list_tables(&pool, PG).await.unwrap().len(), 0);
    Ok(())
}

/// Slice 4 review SHOULD-FIX 3: a resume moves `caught_up_at` of every row to the resume, so
/// a long pause does not read as lag.
#[sqlx::test(migrations = "../../migrations")]
async fn a_resume_counts_the_lag_from_the_resume(pool: PgPool) -> sqlx::Result<()> {
    let old = OffsetDateTime::now_utc().replace_nanosecond(0).unwrap() - Duration::hours(3);
    record_backlog(&pool, PG, "public.orders", 0, true, old)
        .await
        .unwrap();
    record_last_change(&pool, PG, "public.items", old)
        .await
        .unwrap();
    lakehouse_store::connectors::pause_connector(&pool, PG)
        .await
        .unwrap();
    lakehouse_store::connectors::resume_connector(&pool, PG)
        .await
        .unwrap();
    let tables = list_tables(&pool, PG).await.unwrap();
    let before = OffsetDateTime::now_utc() - Duration::minutes(1);
    for row in &tables {
        assert!(
            row.caught_up_at.unwrap() > before,
            "{}: {:?}",
            row.table_name,
            row.caught_up_at
        );
    }
    assert_eq!(tables.len(), 2);
    Ok(())
}
