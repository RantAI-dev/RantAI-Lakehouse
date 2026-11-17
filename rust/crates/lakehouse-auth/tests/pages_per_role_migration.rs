//! ADM-13 review round 1, B1, S1
//! ADM-13 T1: migration `0133_pages_per_role.sql` verification (`D2`, `D3`).
//!
//! Asserts that:
//! - an Analyst role gets the four permissions (`assistant:use`, `ops:read`, `alert:read`, `governance:read`);
//! - Dashboard Viewer does not get them;
//! - a `*:*` role is untouched;
//! - Assistant only exists;
//! - custom roles with existing permissions get them;
//! - a role with `*:*` in a list (`catalog:read, *:*`) is untouched (S1);
//! - a role with empty permissions gets them without a leading comma (S1);
//! - running the migration file itself twice (`include_str!`, whole file) adds nothing twice.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_test_support as _;
use sqlx::PgPool;
use uuid::Uuid;

const MIGRATION_SQL: &str = include_str!("../../../migrations/0133_pages_per_role.sql");

#[sqlx::test(migrations = "../../migrations")]
async fn migration_0133_updates_roles_correctly(pool: PgPool) -> sqlx::Result<()> {
    // 1. Analyst gets the four permissions
    let (analyst_perms,): (String,) =
        sqlx::query_as("SELECT permissions FROM role WHERE name = 'Analyst'")
            .fetch_one(&pool)
            .await?;
    assert!(analyst_perms.contains("assistant:use"));
    assert!(analyst_perms.contains("ops:read"));
    assert!(analyst_perms.contains("alert:read"));
    assert!(analyst_perms.contains("governance:read"));
    assert!(analyst_perms.contains("query:read"));
    assert!(analyst_perms.contains("catalog:read"));
    assert!(analyst_perms.contains("lineage:read"));

    // 2. Dashboard Viewer does not get the four permissions
    let (viewer_perms,): (String,) =
        sqlx::query_as("SELECT permissions FROM role WHERE name = 'Dashboard Viewer'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(viewer_perms, "dashboard:read");
    assert!(!viewer_perms.contains("assistant:use"));
    assert!(!viewer_perms.contains("ops:read"));
    assert!(!viewer_perms.contains("alert:read"));
    assert!(!viewer_perms.contains("governance:read"));

    // 3. Platform Admin (*:*) is untouched
    let (admin_perms,): (String,) =
        sqlx::query_as("SELECT permissions FROM role WHERE name = 'Platform Admin'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(admin_perms, "*:*");

    // 4. Assistant only exists with assistant:ask,dashboard:read
    let (asst_id, asst_perms, asst_desc): (Uuid, String, String) = sqlx::query_as(
        "SELECT id, permissions, description FROM role WHERE name = 'Assistant only'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        asst_id,
        Uuid::parse_str("22222222-2222-4222-8222-000000000008").unwrap()
    );
    assert_eq!(asst_perms, "assistant:ask,dashboard:read");
    assert_ne!(asst_desc, "");

    // 5. Test custom roles:
    // a. Standard custom role with one permission
    let custom_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO role (id, name, permissions, description) VALUES ($1, 'Custom Auditor', 'audit:read', 'A custom role')",
    )
    .bind(custom_id)
    .execute(&pool)
    .await?;

    // b. Custom role with wildcard in list: `catalog:read, *:*` (S1)
    let wildcard_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO role (id, name, permissions, description) VALUES ($1, 'Custom Wildcard', 'catalog:read, *:*', 'A wildcard role')",
    )
    .bind(wildcard_id)
    .execute(&pool)
    .await?;

    // c. Custom role with empty permissions: `""` (S1)
    let empty_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO role (id, name, permissions, description) VALUES ($1, 'Custom Empty', '', 'An empty role')",
    )
    .bind(empty_id)
    .execute(&pool)
    .await?;

    // Run the migration file itself (first time over the newly added custom roles)
    sqlx::raw_sql(MIGRATION_SQL).execute(&pool).await?;

    // Check custom role gained them
    let (custom_perms,): (String,) = sqlx::query_as("SELECT permissions FROM role WHERE id = $1")
        .bind(custom_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        custom_perms,
        "audit:read,assistant:use,ops:read,alert:read,governance:read"
    );

    // Check custom wildcard role is untouched (S1)
    let (wildcard_perms,): (String,) = sqlx::query_as("SELECT permissions FROM role WHERE id = $1")
        .bind(wildcard_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        wildcard_perms, "catalog:read, *:*",
        "Role with catalog:read, *:* must be untouched"
    );

    // Check empty role has no leading comma (S1)
    let (empty_perms,): (String,) = sqlx::query_as("SELECT permissions FROM role WHERE id = $1")
        .bind(empty_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        empty_perms, "assistant:use,ops:read,alert:read,governance:read",
        "Role with empty permissions must not have a leading comma"
    );

    // 6. Idempotence: capture permissions of all roles before running migration file second time
    let roles_before: Vec<(String, String)> =
        sqlx::query_as("SELECT name, permissions FROM role ORDER BY name")
            .fetch_all(&pool)
            .await?;

    // Run the migration file a second time (S1: whole file, twice)
    sqlx::raw_sql(MIGRATION_SQL).execute(&pool).await?;

    let roles_after: Vec<(String, String)> =
        sqlx::query_as("SELECT name, permissions FROM role ORDER BY name")
            .fetch_all(&pool)
            .await?;

    assert_eq!(
        roles_before, roles_after,
        "Running the migration file twice must add nothing twice"
    );

    Ok(())
}
