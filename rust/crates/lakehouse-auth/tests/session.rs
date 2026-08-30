//! Integration tests for `lakehouse_auth::session` against a real Postgres.
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
use lakehouse_auth::session::{
    create_session, revoke_all_sessions_for_user, revoke_session, validate_session,
};
use sqlx::PgPool;
use time::Duration;
use uuid::Uuid;

const RINA: &str = "33333333-3333-4333-8333-000000000001";

#[sqlx::test(migrations = "../../migrations")]
async fn a_freshly_created_session_validates_to_its_owner(pool: PgPool) -> sqlx::Result<()> {
    let rina_id = Uuid::parse_str(RINA).unwrap();
    let token = create_session(
        &pool,
        rina_id,
        Duration::hours(1),
        Some("203.0.113.5"),
        Some("test-agent"),
    )
    .await
    .unwrap();

    let principal = validate_session(&pool, &token).await.unwrap();
    assert_eq!(principal.display_name, "Rina Wijaya");
    assert_eq!(principal.provider, "session");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_expired_session_is_rejected(pool: PgPool) -> sqlx::Result<()> {
    let rina_id = Uuid::parse_str(RINA).unwrap();
    let token = create_session(&pool, rina_id, Duration::seconds(-1), None, None)
        .await
        .unwrap();

    let err = validate_session(&pool, &token).await.unwrap_err();
    assert!(matches!(err, AuthError::SessionInvalid));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_revoked_session_is_rejected(pool: PgPool) -> sqlx::Result<()> {
    let rina_id = Uuid::parse_str(RINA).unwrap();
    let token = create_session(&pool, rina_id, Duration::hours(1), None, None)
        .await
        .unwrap();

    revoke_session(&pool, &token).await.unwrap();

    let err = validate_session(&pool, &token).await.unwrap_err();
    assert!(matches!(err, AuthError::SessionInvalid));
    Ok(())
}

/// Revoking an already-revoked (or never-existed) token is a no-op, not an
/// error — see `revoke_session`'s doc comment.
#[sqlx::test(migrations = "../../migrations")]
async fn revoking_twice_is_not_an_error(pool: PgPool) -> sqlx::Result<()> {
    let rina_id = Uuid::parse_str(RINA).unwrap();
    let token = create_session(&pool, rina_id, Duration::hours(1), None, None)
        .await
        .unwrap();
    revoke_session(&pool, &token).await.unwrap();
    revoke_session(&pool, &token).await.unwrap();
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn revoke_all_sessions_invalidates_every_live_session_for_the_user(
    pool: PgPool,
) -> sqlx::Result<()> {
    let rina_id = Uuid::parse_str(RINA).unwrap();
    let first = create_session(&pool, rina_id, Duration::hours(1), None, None)
        .await
        .unwrap();
    let second = create_session(&pool, rina_id, Duration::hours(1), None, None)
        .await
        .unwrap();

    revoke_all_sessions_for_user(&pool, rina_id).await.unwrap();

    assert!(validate_session(&pool, &first).await.is_err());
    assert!(validate_session(&pool, &second).await.is_err());
    Ok(())
}

/// A random guessed token (never issued) is rejected exactly like an
/// expired/revoked one — no distinguishing error.
#[sqlx::test(migrations = "../../migrations")]
async fn an_unknown_token_is_rejected(pool: PgPool) -> sqlx::Result<()> {
    let err = validate_session(&pool, &lakehouse_auth::Secret::new("not-a-real-token"))
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::SessionInvalid));
    Ok(())
}
