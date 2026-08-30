//! Integration tests for `lakehouse_auth::repository` against a real
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
//!
//! Every test below provisions a database with all migrations applied
//! (including `0002_seed_identity`), so the `mock/identity.ts` fixture set
//! is present.

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap actually runs for this test binary (an
// unreferenced dev-dependency's rlib member can otherwise be dropped
// by the linker before its ctor section is ever considered).
use lakehouse_test_support as _;

use lakehouse_auth::AuthError;
use lakehouse_auth::repository::load_principal_for_user;
use sqlx::PgPool;
use uuid::Uuid;

/// Rina Wijaya (seeded) holds `Analyst` + `Approver` and belongs to
/// `meridian-group`. `load_principal_for_user` must merge both roles'
/// permissions and collect the tenant membership.
#[sqlx::test(migrations = "../../migrations")]
async fn loads_a_seeded_users_merged_permissions_and_tenants(pool: PgPool) -> sqlx::Result<()> {
    let rina_id = Uuid::parse_str("33333333-3333-4333-8333-000000000001").unwrap();
    let meridian_group = Uuid::parse_str("11111111-1111-4111-8111-000000000001").unwrap();

    let principal = load_principal_for_user(&pool, rina_id, "local".to_owned(), false)
        .await
        .unwrap();

    assert_eq!(principal.display_name, "Rina Wijaya");
    assert_eq!(principal.provider, "local");
    // Analyst -> query:read, catalog:read, lineage:read
    assert!(principal.has("query:read"));
    assert!(principal.has("catalog:read"));
    // Approver -> agent:approve, policy:review
    assert!(principal.has("agent:approve"));
    assert!(principal.has("policy:review"));
    // Never granted to either role.
    assert!(!principal.has("pipeline:run"));
    assert!(principal.in_tenant(meridian_group));
    Ok(())
}

/// Platform Admin's `*:*` must round-trip through the real column, not
/// just through unit-tested parsing logic.
#[sqlx::test(migrations = "../../migrations")]
async fn platform_admin_wildcard_grants_everything_end_to_end(pool: PgPool) -> sqlx::Result<()> {
    let fajar_id = Uuid::parse_str("33333333-3333-4333-8333-000000000006").unwrap();
    let principal = load_principal_for_user(&pool, fajar_id, "local".to_owned(), false)
        .await
        .unwrap();
    assert!(principal.has("anything:whatsoever"));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_unknown_user_id_is_not_found(pool: PgPool) -> sqlx::Result<()> {
    let err = load_principal_for_user(&pool, Uuid::nil(), "local".to_owned(), false)
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::NotFound));
    Ok(())
}

/// `0020_extend_role_grants.sql` end to end: Bayu Pratama (seeded Data
/// Engineer) must hold the two tokens that migration adds to that role
/// (`workload:cancel`, `alert:write` — gating `POST /api/ops/workloads/{id}/
/// cancel` and `/api/alerts` CRUD in `lakehouse_api::policy`), but neither
/// of the two deliberately Platform-Admin-only tokens from the same
/// change (`agent:manage`, `storage:restore`) — this is a real column
/// read, not just a parsed literal, so it catches the migration failing to
/// apply as much as it catches a typo in the token string.
#[sqlx::test(migrations = "../../migrations")]
async fn data_engineer_gains_workload_cancel_and_alert_write_from_0020(
    pool: PgPool,
) -> sqlx::Result<()> {
    let bayu_id = Uuid::parse_str("33333333-3333-4333-8333-000000000002").unwrap();
    let principal = load_principal_for_user(&pool, bayu_id, "local".to_owned(), false)
        .await
        .unwrap();

    assert!(principal.has("pipeline:read"), "still holds pipeline:*");
    assert!(principal.has("workload:cancel"));
    assert!(principal.has("alert:write"));
    assert!(!principal.has("agent:manage"));
    assert!(!principal.has("storage:restore"));
    Ok(())
}

/// The same migration, from the other side: Platform Admin's `*:*` still
/// satisfies every one of the four new tokens, including the two
/// deliberately admin-only ones with no explicit grant row.
#[sqlx::test(migrations = "../../migrations")]
async fn platform_admin_satisfies_all_four_new_tokens_from_0020(pool: PgPool) -> sqlx::Result<()> {
    let fajar_id = Uuid::parse_str("33333333-3333-4333-8333-000000000006").unwrap();
    let principal = load_principal_for_user(&pool, fajar_id, "local".to_owned(), false)
        .await
        .unwrap();

    assert!(principal.has("agent:manage"));
    assert!(principal.has("workload:cancel"));
    assert!(principal.has("storage:restore"));
    assert!(principal.has("alert:write"));
    Ok(())
}
