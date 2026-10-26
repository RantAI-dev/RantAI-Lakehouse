//! `lakehouse_store::alert_state` against a real Postgres (`BI-19` part C, D11):
//! a rule without a row, `since` moving only when the state changes, and a
//! failed check leaving the state and the last value alone.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_store::alert_state::{get, record_check, record_failed_check};
use lakehouse_test_support as _;
use sqlx::PgPool;
use time::OffsetDateTime;

fn at(secs: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_800_000_000 + secs).unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_rule_without_a_row_has_no_state(pool: PgPool) {
    assert_eq!(get(&pool, "al_none").await.unwrap(), None);
}

#[sqlx::test(migrations = "../../migrations")]
async fn since_moves_only_when_the_state_changes(pool: PgPool) {
    record_check(&pool, "al_1", "firing", Some(150.0), None, at(0))
        .await
        .unwrap();
    record_check(&pool, "al_1", "firing", Some(160.0), None, at(900))
        .await
        .unwrap();
    let row = get(&pool, "al_1").await.unwrap().unwrap();
    assert_eq!(row.state, "firing");
    assert_eq!(row.since, at(0));
    assert_eq!(row.last_checked_at, at(900));
    assert_eq!(row.last_value, Some(160.0));
    record_check(&pool, "al_1", "ok", Some(40.0), None, at(1800))
        .await
        .unwrap();
    assert_eq!(get(&pool, "al_1").await.unwrap().unwrap().since, at(1800));
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_failed_check_moves_the_time_and_reason_and_nothing_else(pool: PgPool) {
    record_check(&pool, "al_1", "firing", Some(150.0), None, at(0))
        .await
        .unwrap();
    record_failed_check(&pool, "al_1", "the value could not be read.", at(900))
        .await
        .unwrap();
    let row = get(&pool, "al_1").await.unwrap().unwrap();
    assert_eq!(row.state, "firing");
    assert_eq!(row.last_value, Some(150.0));
    assert_eq!(row.last_checked_at, at(900));
    assert_eq!(row.reason.as_deref(), Some("the value could not be read."));
    // A rule never checked before gets an `ok` row.
    record_failed_check(&pool, "al_2", "x", at(0))
        .await
        .unwrap();
    assert_eq!(get(&pool, "al_2").await.unwrap().unwrap().state, "ok");
}
