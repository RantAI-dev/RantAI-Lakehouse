//! Integration tests for `lakehouse_auth::password` against a real
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

use lakehouse_auth::password::{
    change_password, create_local_identity, must_change_password, verify,
};
use lakehouse_auth::{AuthError, Secret};
use sqlx::PgPool;
use uuid::Uuid;

const RINA: &str = "33333333-3333-4333-8333-000000000001";
const RINA_EMAIL: &str = "rina@meridian.example";

#[sqlx::test(migrations = "../../migrations")]
async fn a_freshly_linked_local_identity_authenticates_with_the_right_password(
    pool: PgPool,
) -> sqlx::Result<()> {
    let rina_id = Uuid::parse_str(RINA).unwrap();
    create_local_identity(
        &pool,
        rina_id,
        &Secret::new("correct horse battery staple"),
        false,
    )
    .await
    .unwrap();

    let principal = verify(
        &pool,
        RINA_EMAIL,
        &Secret::new("correct horse battery staple"),
    )
    .await
    .unwrap();
    assert_eq!(principal.display_name, "Rina Wijaya");
    assert_eq!(principal.provider, "local");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_wrong_password_is_rejected(pool: PgPool) -> sqlx::Result<()> {
    let rina_id = Uuid::parse_str(RINA).unwrap();
    create_local_identity(
        &pool,
        rina_id,
        &Secret::new("correct horse battery staple"),
        false,
    )
    .await
    .unwrap();

    let err = verify(&pool, RINA_EMAIL, &Secret::new("wrong password"))
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::InvalidCredentials));
    Ok(())
}

/// The core non-enumeration guarantee: an email with no local identity at
/// all fails with the exact same error as a wrong password for a real
/// account — see `password.rs`'s module doc comment.
#[sqlx::test(migrations = "../../migrations")]
async fn an_unknown_email_fails_identically_to_a_wrong_password(pool: PgPool) -> sqlx::Result<()> {
    let err = verify(&pool, "nobody@meridian.example", &Secret::new("anything"))
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::InvalidCredentials));

    let rina_id = Uuid::parse_str(RINA).unwrap();
    create_local_identity(
        &pool,
        rina_id,
        &Secret::new("correct horse battery staple"),
        false,
    )
    .await
    .unwrap();
    let err2 = verify(&pool, RINA_EMAIL, &Secret::new("wrong password"))
        .await
        .unwrap_err();

    assert_eq!(err.to_string(), err2.to_string());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_bootstrapped_identity_starts_flagged_must_change_password(
    pool: PgPool,
) -> sqlx::Result<()> {
    let rina_id = Uuid::parse_str(RINA).unwrap();
    create_local_identity(
        &pool,
        rina_id,
        &Secret::new("temporary-bootstrap-password"),
        true,
    )
    .await
    .unwrap();

    assert!(must_change_password(&pool, rina_id).await.unwrap());

    change_password(&pool, rina_id, &Secret::new("a-real-chosen-password"))
        .await
        .unwrap();
    assert!(!must_change_password(&pool, rina_id).await.unwrap());

    // The old bootstrap credential no longer works after the change.
    let err = verify(
        &pool,
        RINA_EMAIL,
        &Secret::new("temporary-bootstrap-password"),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, AuthError::InvalidCredentials));

    // The new one does.
    let principal = verify(&pool, RINA_EMAIL, &Secret::new("a-real-chosen-password"))
        .await
        .unwrap();
    assert_eq!(principal.display_name, "Rina Wijaya");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn linking_a_second_local_identity_to_the_same_user_conflicts(
    pool: PgPool,
) -> sqlx::Result<()> {
    let rina_id = Uuid::parse_str(RINA).unwrap();
    create_local_identity(&pool, rina_id, &Secret::new("first-password"), false)
        .await
        .unwrap();
    let err = create_local_identity(&pool, rina_id, &Secret::new("second-password"), false)
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::Conflict(_)));
    Ok(())
}
