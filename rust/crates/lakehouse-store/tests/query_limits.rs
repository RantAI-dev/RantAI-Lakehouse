//! Integration tests for `lakehouse_store::query_limits` against a real
//! Postgres (`#[sqlx::test]`, see `tests/maintenance_policy.rs`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_test_support as _;

use lakehouse_store::StoreError;
use lakehouse_store::query_limits::{QueryLimits, get, role_limits, save};
use sqlx::PgPool;
use uuid::Uuid;

async fn role_id(pool: &PgPool, name: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM role WHERE name = $1")
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("seeded role")
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_migration_seeds_the_feature_defaults_and_the_struct_default_matches(
    pool: PgPool,
) -> sqlx::Result<()> {
    let stored = get(&pool).await.expect("get");
    assert_eq!(stored, QueryLimits::default());
    assert_eq!(
        stored,
        QueryLimits {
            timeout_seconds: 60,
            chart_rows: 2000,
            map_points: 5000,
            studio_rows: 2000
        }
    );
    assert_eq!(role_limits(&pool).await.expect("roles"), Vec::new());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_save_round_trips_and_replaces_the_role_set(pool: PgPool) -> sqlx::Result<()> {
    let analyst = role_id(&pool, "Platform Admin").await;
    let install = QueryLimits {
        timeout_seconds: 90,
        chart_rows: 500,
        map_points: 1000,
        studio_rows: 7000,
    };
    save(&pool, &install, &[(analyst, 300)], "user-a")
        .await
        .expect("save");
    assert_eq!(get(&pool).await.expect("get"), install);
    let roles = role_limits(&pool).await.expect("roles");
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].role_name, "Platform Admin");
    assert_eq!(roles[0].timeout_seconds, 300);

    save(&pool, &install, &[], "user-a").await.expect("save");
    assert_eq!(role_limits(&pool).await.expect("roles"), Vec::new());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_value_outside_the_range_is_refused_and_leaves_the_old_values(
    pool: PgPool,
) -> sqlx::Result<()> {
    let bad = QueryLimits {
        timeout_seconds: 5,
        ..QueryLimits::default()
    };
    assert!(save(&pool, &bad, &[], "u").await.is_err());
    assert_eq!(get(&pool).await.expect("get"), QueryLimits::default());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_role_that_does_not_exist_is_refused_and_nothing_is_saved(
    pool: PgPool,
) -> sqlx::Result<()> {
    let changed = QueryLimits {
        chart_rows: 300,
        ..QueryLimits::default()
    };
    let err = save(&pool, &changed, &[(Uuid::new_v4(), 60)], "u")
        .await
        .expect_err("unknown role");
    assert!(matches!(err, StoreError::ForeignKeyViolation));
    assert_eq!(get(&pool).await.expect("get"), QueryLimits::default());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn deleting_a_role_deletes_its_limit(pool: PgPool) -> sqlx::Result<()> {
    let r: Uuid = sqlx::query_scalar(
        "INSERT INTO role (name, permissions) VALUES ('Temp limit role', '') RETURNING id",
    )
    .fetch_one(&pool)
    .await?;
    save(&pool, &QueryLimits::default(), &[(r, 120)], "u")
        .await
        .expect("save");
    sqlx::query("DELETE FROM role WHERE id = $1")
        .bind(r)
        .execute(&pool)
        .await?;
    assert_eq!(role_limits(&pool).await.expect("roles"), Vec::new());
    Ok(())
}
