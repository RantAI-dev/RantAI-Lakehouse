//! `OPS-8` part B: migration `0148_usage_rate_and_permission.sql` gives the
//! Governance Admin role `usage:read`, and gives it once.
//!
//! The migration file is run again over the migrated database, the whole file
//! with `include_str!`, the way `pages_per_role_migration.rs` proves the
//! `0133` grant: an UPDATE that appended the token on every run would pass a
//! test that ran it once.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_test_support as _;
use sqlx::PgPool;

const MIGRATION_SQL: &str = include_str!("../../../migrations/0148_usage_rate_and_permission.sql");

async fn permissions_of(pool: &PgPool, role: &str) -> String {
    sqlx::query_scalar("SELECT permissions FROM role WHERE name = $1")
        .bind(role)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("role {role} must exist: {e}"))
}

#[sqlx::test(migrations = "../../migrations")]
async fn governance_admin_holds_usage_read_and_nobody_else_gains_it(pool: PgPool) {
    let admin = permissions_of(&pool, "Governance Admin").await;
    assert!(
        admin.split(',').any(|token| token.trim() == "usage:read"),
        "Governance Admin must hold usage:read, holds: {admin}"
    );
    // The roles that are not the tenant administrator's keep what they had.
    for role in ["Analyst", "Data Engineer", "Data Scientist", "Approver"] {
        assert!(
            !permissions_of(&pool, role).await.contains("usage:read"),
            "{role} must not gain usage:read"
        );
    }
    assert_eq!(permissions_of(&pool, "Platform Admin").await, "*:*");
    assert_eq!(
        permissions_of(&pool, "Dashboard Viewer").await,
        "dashboard:read"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn running_the_migration_file_again_changes_no_role(pool: PgPool) {
    let before: Vec<(String, String)> =
        sqlx::query_as("SELECT name, permissions FROM role ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();
    sqlx::raw_sql(MIGRATION_SQL).execute(&pool).await.unwrap();
    sqlx::raw_sql(MIGRATION_SQL).execute(&pool).await.unwrap();
    let after: Vec<(String, String)> =
        sqlx::query_as("SELECT name, permissions FROM role ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(before, after, "the grant must be idempotent");
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_governance_admin_without_the_grant_gets_it_once_without_a_leading_comma(pool: PgPool) {
    // An administrator edited the role down to nothing, then to one token.
    sqlx::query("UPDATE role SET permissions = '' WHERE name = 'Governance Admin'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(MIGRATION_SQL).execute(&pool).await.unwrap();
    assert_eq!(
        permissions_of(&pool, "Governance Admin").await,
        "usage:read"
    );

    sqlx::query("UPDATE role SET permissions = 'audit:read' WHERE name = 'Governance Admin'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(MIGRATION_SQL).execute(&pool).await.unwrap();
    sqlx::raw_sql(MIGRATION_SQL).execute(&pool).await.unwrap();
    assert_eq!(
        permissions_of(&pool, "Governance Admin").await,
        "audit:read,usage:read"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_governance_admin_that_already_holds_everything_is_left_alone(pool: PgPool) {
    sqlx::query(
        "UPDATE role SET permissions = 'catalog:read, *:*' WHERE name = 'Governance Admin'",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(MIGRATION_SQL).execute(&pool).await.unwrap();
    assert_eq!(
        permissions_of(&pool, "Governance Admin").await,
        "catalog:read, *:*"
    );
}
