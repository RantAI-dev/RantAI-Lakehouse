//! `SEC-66`: the upgrade migration's role grants, against a real Postgres.
//!
//! # Why these live here
//!
//! The policy table's unit tests prove which permission a route asks for;
//! they cannot prove that an existing deployment's roles HOLD it after the
//! upgrade. `0153_sec66_read_permissions.sql` is that upgrade, and these
//! `#[sqlx::test(migrations = "../../migrations")]` tests read the seeded
//! `role` rows from a freshly migrated database — the same way
//! `migration_0030_…`/`migration_0031_…` in `maintenance_policy.rs` pin
//! earlier permission migrations. The two dashboard-only roles are asserted
//! in the negative: opening those pages by address is the hole SEC-66
//! closes, not a capability to preserve on upgrade.
//!
//! # Postgres backing
//!
//! Each test gets its own migrated database on a testcontainer Postgres
//! started once per binary by `lakehouse-test-support` (see
//! `maintenance_policy.rs`'s module doc). No manual compose bring-up.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_test_support as _;

use sqlx::PgPool;

/// The SQL being re-applied in the idempotence test. Embedded rather than
/// read at runtime so a migrated database cannot pass by finding a stale
/// file on disk — this is the exact text `sqlx::migrate!` compiled in.
const UPGRADE_SQL: &str = include_str!("../../../migrations/0153_sec66_read_permissions.sql");

async fn permissions_for(pool: &PgPool, name: &str) -> String {
    sqlx::query_scalar("SELECT permissions FROM role WHERE name = $1")
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("a seeded built-in role")
}

#[sqlx::test(migrations = "../../migrations")]
async fn migration_0153_grants_both_read_permissions_once_to_the_working_roles(
    pool: PgPool,
) -> sqlx::Result<()> {
    for name in [
        "Analyst",
        "Approver",
        "Governance Admin",
        "Data Engineer",
        "Data Scientist",
    ] {
        let permissions = permissions_for(&pool, name).await;
        assert_eq!(
            permissions.matches("gold:read").count(),
            1,
            "{name} must hold gold:read exactly once: {permissions}"
        );
        assert_eq!(
            permissions.matches("agent:read").count(),
            1,
            "{name} must hold agent:read exactly once: {permissions}"
        );
    }

    let admin = permissions_for(&pool, "Platform Admin").await;
    assert_eq!(admin, "*:*", "the wildcard role must be left untouched");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn migration_0153_does_not_grant_the_read_permissions_to_dashboard_only_roles(
    pool: PgPool,
) -> sqlx::Result<()> {
    for name in ["Dashboard Viewer", "Assistant only"] {
        let permissions = permissions_for(&pool, name).await;
        assert!(
            !permissions.contains("gold:read"),
            "{name} must not gain gold:read: {permissions}"
        );
        assert!(
            !permissions.contains("agent:read"),
            "{name} must not gain agent:read: {permissions}"
        );
    }
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn migration_0153_is_idempotent_for_roles_and_the_scheduler_identity(
    pool: PgPool,
) -> sqlx::Result<()> {
    // Fresh installs seed the scheduler identity at API boot, which this
    // store-only test binary does not run. Model the pre-SEC-66 row: a
    // credential exists, but the identity has no scopes because its write
    // route checks none.
    let before = permissions_for(&pool, "Analyst").await;

    // The SQL is this repository's own reviewed migration text, embedded
    // at compile time; no caller-controlled value reaches `raw_sql`.
    sqlx::raw_sql(sqlx::AssertSqlSafe(UPGRADE_SQL))
        .execute(&pool)
        .await?;

    assert_eq!(
        permissions_for(&pool, "Analyst").await,
        before,
        "re-applying the migration must not append a second token"
    );

    // The scheduler scope path: one read scope, never the write grant.
    sqlx::query(
        "INSERT INTO service_identity (id, name, scopes, environment, expires_at) \
         VALUES ($1, 'gold-export-scheduler', ARRAY[]::TEXT[], 'development', now() + interval '30 days')",
    )
    .bind(uuid::Uuid::new_v4())
    .execute(&pool)
    .await?;
    sqlx::raw_sql(sqlx::AssertSqlSafe(UPGRADE_SQL))
        .execute(&pool)
        .await?;
    sqlx::raw_sql(sqlx::AssertSqlSafe(UPGRADE_SQL))
        .execute(&pool)
        .await?;

    let scopes: Vec<String> = sqlx::query_scalar(
        "SELECT scopes FROM service_identity WHERE name = 'gold-export-scheduler'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        scopes,
        vec!["gold:read".to_owned()],
        "the scheduler must gain gold:read once and never gold:export"
    );
    Ok(())
}
