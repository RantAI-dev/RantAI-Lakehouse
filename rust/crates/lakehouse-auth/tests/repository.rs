//! Integration tests for `lakehouse_auth::repository` against a real
//! Postgres.
//!
//! # Why every test here is `#[ignore]`d
//!
//! Same reason as `lakehouse-store`'s integration tests: `#[sqlx::test]`
//! needs a live Postgres reachable via `DATABASE_URL`, and
//! `cargo test --workspace --locked` must stay green on a machine that has
//! none. Run explicitly with:
//!
//! ```sh
//! docker compose up -d postgres
//! DATABASE_URL=postgres://lakehouse:lakehouse@localhost:5432/lakehouse \
//!   cargo test -p lakehouse-auth -- --ignored
//! ```
//!
//! Every test below provisions a database with all migrations applied
//! (including `0002_seed_identity`), so the `mock/identity.ts` fixture set
//! is present.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_auth::AuthError;
use lakehouse_auth::repository::load_principal_for_user;
use sqlx::PgPool;
use uuid::Uuid;

/// Rina Wijaya (seeded) holds `Analyst` + `Approver` and belongs to
/// `meridian-group`. `load_principal_for_user` must merge both roles'
/// permissions and collect the tenant membership.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn loads_a_seeded_users_merged_permissions_and_tenants(pool: PgPool) -> sqlx::Result<()> {
    let rina_id = Uuid::parse_str("33333333-3333-4333-8333-000000000001").unwrap();
    let meridian_group = Uuid::parse_str("11111111-1111-4111-8111-000000000001").unwrap();

    let principal = load_principal_for_user(&pool, rina_id, "local".to_owned())
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
#[ignore = "requires a live Postgres; see module doc comment"]
async fn platform_admin_wildcard_grants_everything_end_to_end(pool: PgPool) -> sqlx::Result<()> {
    let fajar_id = Uuid::parse_str("33333333-3333-4333-8333-000000000006").unwrap();
    let principal = load_principal_for_user(&pool, fajar_id, "local".to_owned())
        .await
        .unwrap();
    assert!(principal.has("anything:whatsoever"));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn an_unknown_user_id_is_not_found(pool: PgPool) -> sqlx::Result<()> {
    let err = load_principal_for_user(&pool, Uuid::nil(), "local".to_owned())
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::NotFound));
    Ok(())
}
