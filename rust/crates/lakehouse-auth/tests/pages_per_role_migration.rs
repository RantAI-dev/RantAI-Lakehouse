//! ADM-13 T1: migration `0132_pages_per_role.sql` verification (`D2`, `D3`).
//!
//! Asserts that:
//! - an Analyst role gets the four permissions (`assistant:use`, `ops:read`, `alert:read`, `governance:read`);
//! - Dashboard Viewer does not get them;
//! - a custom role gets them;
//! - a `*:*` role is untouched;
//! - Assistant only exists;
//! - running the UPDATE twice adds nothing twice.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_test_support as _;
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrations = "../../migrations")]
async fn migration_0132_updates_roles_correctly(pool: PgPool) -> sqlx::Result<()> {
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

    // 5. A custom role gets them, and running UPDATE twice adds nothing twice:
    // Create a custom role with initial permissions.
    let custom_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO role (id, name, permissions, description) VALUES ($1, 'Custom Auditor', 'audit:read', 'A custom role')",
    )
    .bind(custom_id)
    .execute(&pool)
    .await?;

    // Execute the exact UPDATE query from 0132_pages_per_role.sql
    let update_sql = r"
UPDATE role
SET permissions = permissions || ',assistant:use,ops:read,alert:read,governance:read'
WHERE name NOT IN ('Dashboard Viewer', 'Assistant only')
  AND permissions != '*:*'
  AND permissions !~ '(^|,)\s*\*:*\s*(,|$)'
  AND permissions !~ '(^|,)\s*assistant:use\s*(,|$)'
  AND permissions !~ '(^|,)\s*ops:read\s*(,|$)'
  AND permissions !~ '(^|,)\s*alert:read\s*(,|$)'
  AND permissions !~ '(^|,)\s*governance:read\s*(,|$)';
";
    sqlx::query(update_sql).execute(&pool).await?;

    // Check custom role gained them
    let (custom_perms,): (String,) = sqlx::query_as("SELECT permissions FROM role WHERE id = $1")
        .bind(custom_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        custom_perms,
        "audit:read,assistant:use,ops:read,alert:read,governance:read"
    );

    // Capture permissions of all roles before second update
    let roles_before: Vec<(String, String)> =
        sqlx::query_as("SELECT name, permissions FROM role ORDER BY name")
            .fetch_all(&pool)
            .await?;

    // Run the UPDATE a second time
    sqlx::query(update_sql).execute(&pool).await?;

    let roles_after: Vec<(String, String)> =
        sqlx::query_as("SELECT name, permissions FROM role ORDER BY name")
            .fetch_all(&pool)
            .await?;

    assert_eq!(
        roles_before, roles_after,
        "Running the UPDATE twice must add nothing twice"
    );

    Ok(())
}
