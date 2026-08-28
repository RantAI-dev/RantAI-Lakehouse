//! Integration tests for `lakehouse_auth::password` against a real
//! Postgres.
//!
//! Same `#[ignore]` rationale as `tests/repository.rs`. Run with:
//!
//! ```sh
//! docker compose up -d postgres
//! DATABASE_URL=postgres://lakehouse:lakehouse@localhost:5432/lakehouse \
//!   cargo test -p lakehouse-auth -- --ignored
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_auth::password::{
    change_password, create_local_identity, must_change_password, verify,
};
use lakehouse_auth::{AuthError, Secret};
use sqlx::PgPool;
use uuid::Uuid;

const RINA: &str = "33333333-3333-4333-8333-000000000001";
const RINA_EMAIL: &str = "rina@meridian.example";

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
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
#[ignore = "requires a live Postgres; see module doc comment"]
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
#[ignore = "requires a live Postgres; see module doc comment"]
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
#[ignore = "requires a live Postgres; see module doc comment"]
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
#[ignore = "requires a live Postgres; see module doc comment"]
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
