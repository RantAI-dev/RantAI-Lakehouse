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
use time::OffsetDateTime;
use uuid::Uuid;

/// `bi-dashboard-reader` (seeded), scopes `["query:read", "catalog:read"]`,
/// `expires_at` 30 days out.
const BI_DASHBOARD_READER: &str = "44444444-4444-4444-8444-000000000001";
/// `price-crawler-agent` (seeded), already expired.
const EXPIRED_IDENTITY: &str = "44444444-4444-4444-8444-000000000006";

/// Seed a `service_identity` row exactly the shape a freshly created one
/// takes (name + scopes + environment + future expiry + `rotation_status =
/// 'current'`), so the last-used-at tests below exercise input matching what
/// production seeding would create. The `name` is suffixed with a fresh
/// `Uuid` because `#[sqlx::test]` runs every test in the same binary in
/// the SAME database — only the test function gets a fresh connection per
/// run, not a fresh schema (the `service_identity_name_unique` constraint
/// would otherwise block a second test reusing a literal name). Mirrors
/// `lakehouse-store`'s `seed_service_identity` (`tests/identity.rs:606`)
/// so the two crates' fixtures stay in lockstep.
async fn seed_service_identity(pool: &PgPool, name: &str) -> sqlx::Result<Uuid> {
    let unique_name = format!("{name}-{}", Uuid::new_v4().simple());
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO service_identity (name, scopes, environment, rotation_status, expires_at) \
         VALUES ($1, $2, $3, 'current', now() + interval '90 days') RETURNING id",
    )
    .bind(&unique_name)
    .bind(vec!["query:read".to_owned()])
    .bind("production")
    .fetch_one(pool)
    .await?;
    Ok(id)
}

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

/// A service identity's scopes are not roles (WS7 item A1): no authored
/// policy names a service principal by role, so `Principal.role_names`
/// must come back empty rather than derived from `scopes`.
#[sqlx::test(migrations = "../../migrations")]
async fn role_names_empty_for_a_service_principal(pool: PgPool) -> sqlx::Result<()> {
    let service_id = Uuid::parse_str(BI_DASHBOARD_READER).unwrap();
    let token = create_service_credential(&pool, service_id).await.unwrap();

    let principal = verify_service_token(&pool, &token).await.unwrap();
    assert!(principal.role_names.is_empty());
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

/// Behaviour 1 of `verify_service_token`'s last-used-at throttle:
/// `verify_service_token` writes
/// `last_used_at` on first use past the throttle window. `0001_init.sql`
/// defaults `last_used_at` to `now()` at INSERT time, so "first use" here
/// means the value the seed already set moves forward past that original
/// insert timestamp — asserted directly, not inferred from an `Option`
/// that is never actually `None` in practice (`identity.rs:125`'s
/// `DEFAULT NOW()` means the column is `NOT NULL` from the moment a row
/// exists). Back-dating 400s deliberately ages the seeded timestamp past
/// the 300s throttle window so THIS test's "after-verify" timestamp is
/// unambiguously a write from `verify_service_token`, not a relic of
/// `INSERT` — and so the assertion is independent of the throttle logic
/// covered separately below.
#[sqlx::test(migrations = "../../migrations")]
async fn verify_service_token_writes_last_used_at_on_first_use(pool: PgPool) -> sqlx::Result<()> {
    let identity_id = seed_service_identity(&pool, "ingestion-worker").await?;
    let (seeded_at,): (OffsetDateTime,) =
        sqlx::query_as("SELECT last_used_at FROM service_identity WHERE id = $1")
            .bind(identity_id)
            .fetch_one(&pool)
            .await?;
    sqlx::query("UPDATE service_identity SET last_used_at = $1 WHERE id = $2")
        .bind(seeded_at - time::Duration::seconds(400))
        .bind(identity_id)
        .execute(&pool)
        .await?;

    let token = create_service_credential(&pool, identity_id).await.unwrap();
    verify_service_token(&pool, &token).await.unwrap();

    let (after,): (OffsetDateTime,) =
        sqlx::query_as("SELECT last_used_at FROM service_identity WHERE id = $1")
            .bind(identity_id)
            .fetch_one(&pool)
            .await?;
    assert!(
        after > seeded_at - time::Duration::seconds(400),
        "after-verify last_used_at must move past the back-dated seed, got {after}"
    );
    assert!(
        after > OffsetDateTime::now_utc() - time::Duration::seconds(5),
        "after-verify last_used_at must be recent (within 5s of now), got {after}"
    );
    Ok(())
}

/// Behaviour 2 of the last-used-at throttle: within the 300s throttle
/// window, a successful verification must NOT move `last_used_at` at all. This is
/// the throttle's load-bearing assertion: a naive "always write"
/// implementation would fail here. The throttle's
/// `WHERE last_used_at < now() - ($N * INTERVAL '1 second')` predicate
/// matches zero rows when the row is within the window, and `rows_affected
/// == 0` is a silent no-op (behaviour 4). The back-dated value is
/// re-read through Postgres before the equality check, because
/// `TIMESTAMPTZ` only stores microseconds (a `time::OffsetDateTime` has
/// nanoseconds, so the bind-side and read-side values differ in their
/// trailing digits — same row, different `OffsetDateTime`).
#[sqlx::test(migrations = "../../migrations")]
async fn verify_service_token_does_not_rewrite_last_used_at_within_the_throttle_window(
    pool: PgPool,
) -> sqlx::Result<()> {
    let identity_id = seed_service_identity(&pool, "ingestion-worker").await?;
    let token = create_service_credential(&pool, identity_id).await.unwrap();
    // A real "used ten seconds ago" row: created an hour ago, last used
    // ten seconds ago. The earlier fixture moved only `last_used_at`, to a
    // moment BEFORE the row's own `created_at` — a last use that predates
    // the identity, which no real row can have, and which the throttle now
    // (correctly) reads as "never used" and lets through.
    sqlx::query(
        "UPDATE service_identity \
         SET created_at = now() - interval '1 hour', \
             last_used_at = now() - interval '10 seconds' \
         WHERE id = $1",
    )
    .bind(identity_id)
    .execute(&pool)
    .await?;
    let (recent,): (OffsetDateTime,) =
        sqlx::query_as("SELECT last_used_at FROM service_identity WHERE id = $1")
            .bind(identity_id)
            .fetch_one(&pool)
            .await?;

    verify_service_token(&pool, &token).await.unwrap();

    let (after,): (OffsetDateTime,) =
        sqlx::query_as("SELECT last_used_at FROM service_identity WHERE id = $1")
            .bind(identity_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        after, recent,
        "a verification within the 300s window must not move last_used_at at all"
    );
    Ok(())
}

/// Behaviour 3 of the last-used-at throttle: past the 300s throttle
/// window, a successful verification MUST move `last_used_at` forward again. The
/// 301s back-date is one second past the window so a slow CI clock can't
/// slip this into the "no-op" case.
#[sqlx::test(migrations = "../../migrations")]
async fn verify_service_token_rewrites_last_used_at_after_the_throttle_window(
    pool: PgPool,
) -> sqlx::Result<()> {
    let identity_id = seed_service_identity(&pool, "ingestion-worker").await?;
    let token = create_service_credential(&pool, identity_id).await.unwrap();
    let stale = OffsetDateTime::now_utc() - time::Duration::seconds(301); // just past the 300s window
    sqlx::query("UPDATE service_identity SET last_used_at = $1 WHERE id = $2")
        .bind(stale)
        .bind(identity_id)
        .execute(&pool)
        .await?;

    verify_service_token(&pool, &token).await.unwrap();

    let (after,): (OffsetDateTime,) =
        sqlx::query_as("SELECT last_used_at FROM service_identity WHERE id = $1")
            .bind(identity_id)
            .fetch_one(&pool)
            .await?;
    assert!(
        after > stale,
        "a verification past the 300s window must move last_used_at forward (got {after}, stale was {stale})"
    );
    Ok(())
}

/// A first use that lands inside the throttle window of the identity's own
/// CREATION must still be recorded. The column defaults to the insert time,
/// so without the `last_used_at <= created_at` branch the plain throttle
/// would skip it, and the store — which serves `lastUsedAt` only once the
/// column has moved past `created_at` — would report a used identity as
/// never used until some second call happened to land after the window.
#[sqlx::test(migrations = "../../migrations")]
async fn verify_service_token_records_a_first_use_made_right_after_creation(
    pool: PgPool,
) -> sqlx::Result<()> {
    let identity_id = seed_service_identity(&pool, "ingestion-worker").await?;
    sqlx::query("UPDATE service_identity SET last_used_at = created_at WHERE id = $1")
        .bind(identity_id)
        .execute(&pool)
        .await?;
    let token = create_service_credential(&pool, identity_id).await.unwrap();

    verify_service_token(&pool, &token).await.unwrap();

    let (created_at, last_used_at): (OffsetDateTime, OffsetDateTime) =
        sqlx::query_as("SELECT created_at, last_used_at FROM service_identity WHERE id = $1")
            .bind(identity_id)
            .fetch_one(&pool)
            .await?;
    assert!(
        last_used_at > created_at,
        "a first use must move last_used_at past created_at, got created {created_at} / used {last_used_at}"
    );
    Ok(())
}
