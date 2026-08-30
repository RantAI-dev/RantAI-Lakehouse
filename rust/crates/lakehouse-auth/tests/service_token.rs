//! Integration tests for `lakehouse_auth::service_token` against a real
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

use lakehouse_auth::AuthError;
use lakehouse_auth::principal::PrincipalId;
use lakehouse_auth::service_token::{
    create_service_credential, revoke_service_credential, verify_service_token,
};
use sqlx::PgPool;
use uuid::Uuid;

/// `bi-dashboard-reader` (seeded), scopes `["query:read", "catalog:read"]`,
/// `expires_at` 30 days out.
const BI_DASHBOARD_READER: &str = "44444444-4444-4444-8444-000000000001";
/// `price-crawler-agent` (seeded), already expired.
const EXPIRED_IDENTITY: &str = "44444444-4444-4444-8444-000000000006";

#[sqlx::test(migrations = "../../migrations")]
async fn a_freshly_issued_token_verifies_with_the_identitys_scopes_as_permissions(
    pool: PgPool,
) -> sqlx::Result<()> {
    let service_id = Uuid::parse_str(BI_DASHBOARD_READER).unwrap();
    let token = create_service_credential(&pool, service_id).await.unwrap();

    let principal = verify_service_token(&pool, &token).await.unwrap();
    assert_eq!(principal.id, PrincipalId::Service(service_id));
    assert_eq!(principal.display_name, "bi-dashboard-reader");
    assert!(principal.has("query:read"));
    assert!(principal.has("catalog:read"));
    assert!(!principal.has("pipeline:run"));
    assert!(principal.tenant_ids.is_empty());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_revoked_service_token_is_rejected(pool: PgPool) -> sqlx::Result<()> {
    let service_id = Uuid::parse_str(BI_DASHBOARD_READER).unwrap();
    let token = create_service_credential(&pool, service_id).await.unwrap();
    revoke_service_credential(&pool, &token).await.unwrap();

    let err = verify_service_token(&pool, &token).await.unwrap_err();
    assert!(matches!(err, AuthError::ServiceCredentialInvalid));
    Ok(())
}

/// A token for an identity whose `expires_at` has already passed (the
/// seeded `price-crawler-agent`) must not verify, even if the credential
/// row itself was never explicitly revoked.
#[sqlx::test(migrations = "../../migrations")]
async fn a_token_for_an_expired_service_identity_is_rejected(pool: PgPool) -> sqlx::Result<()> {
    let service_id = Uuid::parse_str(EXPIRED_IDENTITY).unwrap();
    let token = create_service_credential(&pool, service_id).await.unwrap();

    let err = verify_service_token(&pool, &token).await.unwrap_err();
    assert!(matches!(err, AuthError::ServiceCredentialInvalid));
    Ok(())
}
