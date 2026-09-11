//! Integration tests for `lakehouse_store::maintenance_policy` against a real
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

use lakehouse_store::StoreError;
use lakehouse_store::maintenance_policy::{
    MaintenancePolicyInput, get_policy, list_all_policies, upsert_policy,
};
use sqlx::PgPool;

fn valid_input() -> MaintenancePolicyInput {
    MaintenancePolicyInput {
        namespace: "bronze".to_owned(),
        table_name: "orders".to_owned(),
        snapshots_to_keep: Some(10),
        orphan_age_hours: Some(72),
        compact_small_files: true,
        schedule: Some("daily".to_owned()),
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn upsert_then_get_round_trips(pool: PgPool) -> sqlx::Result<()> {
    let input = valid_input();
    upsert_policy(&pool, &input).await.expect("upsert");
    let got = get_policy(&pool, "bronze", "orders")
        .await
        .expect("query")
        .expect("row present");
    assert_eq!(got.snapshots_to_keep, Some(10));
    assert_eq!(got.schedule, Some("daily".to_owned()));
    assert!(got.compact_small_files);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn second_upsert_of_the_same_key_replaces_it_instead_of_duplicating(
    pool: PgPool,
) -> sqlx::Result<()> {
    upsert_policy(&pool, &valid_input())
        .await
        .expect("first upsert");
    let mut updated = valid_input();
    updated.snapshots_to_keep = Some(3);
    updated.schedule = Some("weekly".to_owned());
    upsert_policy(&pool, &updated).await.expect("second upsert");

    let all = list_all_policies(&pool).await.expect("list");
    let matching: Vec<_> = all
        .iter()
        .filter(|row| row.namespace == "bronze" && row.table_name == "orders")
        .collect();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].snapshots_to_keep, Some(3));
    assert_eq!(matching[0].schedule, Some("weekly".to_owned()));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn get_returns_none_for_a_table_with_no_policy(pool: PgPool) -> sqlx::Result<()> {
    let got = get_policy(&pool, "bronze", "never_configured")
        .await
        .expect("query");
    assert!(got.is_none());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_all_policies_orders_by_namespace_then_table_name(pool: PgPool) -> sqlx::Result<()> {
    let mut b_orders = valid_input();
    b_orders.namespace = "bronze".to_owned();
    b_orders.table_name = "orders".to_owned();
    let mut b_events = valid_input();
    b_events.namespace = "bronze".to_owned();
    b_events.table_name = "events".to_owned();
    let mut s_orders = valid_input();
    s_orders.namespace = "silver".to_owned();
    s_orders.table_name = "orders".to_owned();

    upsert_policy(&pool, &b_orders)
        .await
        .expect("upsert b_orders");
    upsert_policy(&pool, &b_events)
        .await
        .expect("upsert b_events");
    upsert_policy(&pool, &s_orders)
        .await
        .expect("upsert s_orders");

    let all = list_all_policies(&pool).await.expect("list");
    let keys: Vec<(String, String)> = all
        .into_iter()
        .map(|row| (row.namespace, row.table_name))
        .collect();
    assert_eq!(
        keys,
        vec![
            ("bronze".to_owned(), "events".to_owned()),
            ("bronze".to_owned(), "orders".to_owned()),
            ("silver".to_owned(), "orders".to_owned()),
        ]
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn migration_0030_grants_governance_write_to_governance_admin_exactly_once(
    pool: PgPool,
) -> sqlx::Result<()> {
    let permissions: String =
        sqlx::query_scalar("SELECT permissions FROM role WHERE name = 'Governance Admin'")
            .fetch_one(&pool)
            .await
            .expect("seeded Governance Admin role");
    assert_eq!(permissions.matches("governance:write").count(), 1);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn migration_0031_grants_catalog_read_to_governance_admin_exactly_once(
    pool: PgPool,
) -> sqlx::Result<()> {
    let permissions: String =
        sqlx::query_scalar("SELECT permissions FROM role WHERE name = 'Governance Admin'")
            .fetch_one(&pool)
            .await
            .expect("seeded Governance Admin role");
    assert_eq!(permissions.matches("catalog:read").count(), 1);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn invalid_schedule_is_refused_by_the_check_constraint(pool: PgPool) -> sqlx::Result<()> {
    let mut input = valid_input();
    input.schedule = Some("hourly".to_owned());
    let err = upsert_policy(&pool, &input)
        .await
        .expect_err("hourly schedule must be refused");
    assert!(matches!(err, StoreError::Database(_)));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn table_name_with_a_sql_injection_attempt_is_refused_by_the_check_constraint(
    pool: PgPool,
) -> sqlx::Result<()> {
    let mut input = valid_input();
    input.table_name = "orders; drop".to_owned();
    let err = upsert_policy(&pool, &input)
        .await
        .expect_err("non-conforming table name must be refused");
    assert!(matches!(err, StoreError::Database(_)));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn table_name_with_uppercase_is_refused_by_the_check_constraint(
    pool: PgPool,
) -> sqlx::Result<()> {
    let mut input = valid_input();
    input.table_name = "Orders".to_owned();
    let err = upsert_policy(&pool, &input)
        .await
        .expect_err("uppercase table name must be refused");
    assert!(matches!(err, StoreError::Database(_)));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn zero_snapshots_to_keep_is_refused_by_the_check_constraint(
    pool: PgPool,
) -> sqlx::Result<()> {
    let mut input = valid_input();
    input.snapshots_to_keep = Some(0);
    let err = upsert_policy(&pool, &input)
        .await
        .expect_err("a keep-count of 0 must be refused: the current snapshot is always kept");
    assert!(matches!(err, StoreError::Database(_)));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn zero_orphan_age_hours_is_refused_by_the_check_constraint(
    pool: PgPool,
) -> sqlx::Result<()> {
    let mut input = valid_input();
    input.orphan_age_hours = Some(0);
    let err = upsert_policy(&pool, &input).await.expect_err(
        "an orphan age of 0 must be refused: in-flight writes may still need those files",
    );
    assert!(matches!(err, StoreError::Database(_)));
    Ok(())
}
