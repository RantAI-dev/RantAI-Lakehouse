//! `lakehouse_store::alert_delivery` against a real Postgres (`SRC-40`,
//! `TC-1695`; migration `0139`): a rule never delivered to, the latest attempt
//! replacing the one before it, an older attempt losing to a newer one, a
//! deleted rule's row going away, and the table refusing a reason on a
//! delivered row.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_store::alert_delivery::{delete, last_for, record};
use lakehouse_store::overview::DeliveryRecord;
use lakehouse_test_support as _;
use sqlx::PgPool;
use time::OffsetDateTime;

fn at(secs: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_800_000_000 + secs).unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_rule_never_delivered_to_has_no_row(pool: PgPool) {
    let rows = last_for(&pool, &["al_none".to_owned()]).await.unwrap();
    assert!(rows.is_empty());
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_latest_attempt_replaces_the_one_before_it(pool: PgPool) {
    record(
        &pool,
        "al_1",
        DeliveryRecord::NotDelivered("the webhook server answered with an error"),
        at(0),
    )
    .await
    .unwrap();
    record(&pool, "al_1", DeliveryRecord::Delivered, at(60))
        .await
        .unwrap();
    let rows = last_for(&pool, &["al_1".to_owned()]).await.unwrap();
    let row = rows.get("al_1").unwrap();
    assert!(row.delivered);
    assert_eq!(row.reason, None);
    assert_eq!(row.at, "2027-01-15T08:01:00.000Z");
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_older_attempt_never_replaces_a_newer_one(pool: PgPool) {
    record(&pool, "al_1", DeliveryRecord::Delivered, at(60))
        .await
        .unwrap();
    record(
        &pool,
        "al_1",
        DeliveryRecord::NotDelivered("the webhook server answered with an error"),
        at(0),
    )
    .await
    .unwrap();
    let rows = last_for(&pool, &["al_1".to_owned()]).await.unwrap();
    let row = rows.get("al_1").unwrap();
    assert!(row.delivered);
    assert_eq!(row.reason, None);
    assert_eq!(row.at, "2027-01-15T08:01:00.000Z");
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_deleted_rules_row_is_gone(pool: PgPool) {
    record(&pool, "al_1", DeliveryRecord::Delivered, at(0))
        .await
        .unwrap();
    delete(&pool, "al_1").await.unwrap();
    let rows = last_for(&pool, &["al_1".to_owned()]).await.unwrap();
    assert!(rows.is_empty());
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_table_refuses_a_reason_on_a_delivered_row(pool: PgPool) {
    let written = sqlx::query(
        "INSERT INTO alert_rule_delivery (rule_id, at, delivery_status, delivery_reason) \
         VALUES ($1, now(), 'delivered', 'x')",
    )
    .bind("al_r")
    .execute(&pool)
    .await;
    assert!(written.is_err());
}
