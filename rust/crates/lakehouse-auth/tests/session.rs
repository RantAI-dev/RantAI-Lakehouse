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
        &[],
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
    let token = create_session(&pool, rina_id, Duration::seconds(-1), None, None, &[])
        .await
        .unwrap();

    let err = validate_session(&pool, &token).await.unwrap_err();
    assert!(matches!(err, AuthError::SessionInvalid));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_revoked_session_is_rejected(pool: PgPool) -> sqlx::Result<()> {
    let rina_id = Uuid::parse_str(RINA).unwrap();
    let token = create_session(&pool, rina_id, Duration::hours(1), None, None, &[])
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
    let token = create_session(&pool, rina_id, Duration::hours(1), None, None, &[])
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
    let first = create_session(&pool, rina_id, Duration::hours(1), None, None, &[])
        .await
        .unwrap();
    let second = create_session(&pool, rina_id, Duration::hours(1), None, None, &[])
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

// ── `oidc_mapped_roles` (0045_session_oidc_mapped_roles.sql) ───────────────
//
// Regression coverage for the bug this migration fixes: an OIDC login's
// `OIDC_ROLE_MAP`-mapped permissions used to vanish the moment
// `session::create_session` was called, because only `app_user_id` was
// persisted. These tests exercise `validate_session`'s re-resolution of
// `oidc_mapped_roles` directly against Postgres — `0045`'s migration and
// `repository::permissions_for_role_names` are the code under test, not a
// mock.

/// A session created with a mapped role name (`oidc_mapped_roles`) yields a
/// `Principal` carrying that role's permissions on every `validate_session`
/// call, not just the login response that created it — the exact gap
/// `routes::auth::oidc_callback` used to have before this migration.
#[sqlx::test(migrations = "../../migrations")]
async fn a_session_with_mapped_roles_yields_their_permissions_on_resolve(
    pool: PgPool,
) -> sqlx::Result<()> {
    let rina_id = Uuid::parse_str(RINA).unwrap();
    // Rina's own local roles (Analyst + Approver, seeded) never grant
    // `identity:write` — only the mapped "Platform Admin" role does, so a
    // pass here proves the mapped role's permissions actually reached the
    // resolved principal, not merely that resolution didn't error.
    let token = create_session(
        &pool,
        rina_id,
        Duration::hours(1),
        None,
        None,
        &["Platform Admin".to_owned()],
    )
    .await
    .unwrap();

    let principal = validate_session(&pool, &token).await.unwrap();
    assert!(principal.has("identity:write"));
    // `role_names` stays exactly Rina's real `app_user_role` memberships —
    // a mapped role widens permissions, never fabricates a role
    // membership (same rule WS7's access-grant merge already holds).
    assert!(!principal.role_names.iter().any(|r| r == "Platform Admin"));
    Ok(())
}

/// A session with no mapped roles (`oidc_mapped_roles` at its `'{}'`
/// default — every password/service session) resolves exactly as before
/// this migration: only the user's real `app_user_role` grants.
#[sqlx::test(migrations = "../../migrations")]
async fn a_password_session_without_mapped_roles_is_unchanged(pool: PgPool) -> sqlx::Result<()> {
    let rina_id = Uuid::parse_str(RINA).unwrap();
    let token = create_session(&pool, rina_id, Duration::hours(1), None, None, &[])
        .await
        .unwrap();

    let principal = validate_session(&pool, &token).await.unwrap();
    assert!(!principal.has("identity:write"));
    Ok(())
}

/// A mapped role name that no longer names a real `role` row (deleted or
/// renamed after the session was created) is ignored, not an error —
/// `repository::permissions_for_role_names`'s documented behavior. A
/// session must stay usable even if the mapping it was minted under has
/// since drifted.
#[sqlx::test(migrations = "../../migrations")]
async fn a_mapped_role_that_no_longer_exists_does_not_break_resolution(
    pool: PgPool,
) -> sqlx::Result<()> {
    let rina_id = Uuid::parse_str(RINA).unwrap();
    let token = create_session(
        &pool,
        rina_id,
        Duration::hours(1),
        None,
        None,
        &["Role That Was Deleted".to_owned()],
    )
    .await
    .unwrap();

    let principal = validate_session(&pool, &token).await.unwrap();
    // No crash, no error — just no extra permission from the vanished role.
    assert!(!principal.has("identity:write"));
    Ok(())
}
