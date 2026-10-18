//! Acceptance tests for the password-login throttle (plan
//! `2026-10-02-login-throttle-session-cleanup`, T3).
//!
//! Each test builds a real router over an isolated migrated database via
//! the shared harness, with `LOGIN_MAX_FAILURES`/`LOGIN_LOCKOUT_SECS`
//! overridden to small values so a test can walk an email through the
//! whole lock/expiry lifecycle in seconds. The non-enumeration property
//! ("same statuses and bodies whether or not the account exists") is
//! asserted by replaying the identical sequence against a non-existent
//! email on a fresh app and comparing wire bodies byte for byte.
//!
//! These tests run against Postgres via `lakehouse-test-support`'s
//! testcontainer, like every other file in this directory.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use common::{TestApp, spin_up_with_env};

const THROTTLED_EMAIL: &str = "fajar@meridian.example";
const REAL_PASSWORD: &str = "Co7rect!Passw0rd";
const WRONG_PASSWORD: &str = "not-the-password";

/// Two failures then a lock — small enough that every test walks the full
/// lifecycle, large enough that a single stray wrong password cannot lock
/// a key by accident mid-sequence.
const MAX_FAILURES: &str = "2";

/// The non-existent email the non-enumeration test replays the sequence
/// against; declared here so the item sits before the statements that use
/// it (clippy `items_after_statements`).
const GHOST_EMAIL: &str = "nobody@meridian.example";

fn env_with(max_failures: &str, lockout_secs: &str) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("LOGIN_MAX_FAILURES".to_owned(), max_failures.to_owned());
    env.insert("LOGIN_LOCKOUT_SECS".to_owned(), lockout_secs.to_owned());
    env
}

async fn login(app: &axum::Router, email: &str, password: &str) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"email": "{email}", "password": "{password}"}}"#
                )))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright")
}

async fn body_string(resp: &mut axum::http::Response<Body>) -> String {
    let bytes = axum::body::to_bytes(
        std::mem::take(resp).into_body(),
        // The login error bodies are one short JSON object; the cap only
        // has to exceed that, never a real payload.
        64 * 1024,
    )
    .await
    .expect("read body");
    String::from_utf8(bytes.to_vec()).expect("UTF-8 body")
}

/// Seeds a working `provider = 'local'` password identity for the seeded
/// Platform Admin, with `must_change_password = false` so a correct login
/// is a 200 and the only variable under test is the throttle.
async fn seed_password_identity(pool: &sqlx::PgPool) {
    let (user_id,): (uuid::Uuid,) = sqlx::query_as("SELECT id FROM app_user WHERE email = $1")
        .bind(THROTTLED_EMAIL)
        .fetch_one(pool)
        .await
        .expect("seeded fajar@meridian.example (0002_seed_identity.sql)");
    lakehouse_auth::password::create_local_identity(
        pool,
        user_id,
        &lakehouse_auth::Secret::new(REAL_PASSWORD),
        false,
    )
    .await
    .expect("create the local identity under test");
}

#[tokio::test]
async fn max_failures_wrong_passwords_then_the_correct_password_gets_429_with_retry_after() {
    let TestApp { router, pool } = spin_up_with_env(&env_with(MAX_FAILURES, "300")).await;
    seed_password_identity(&pool).await;

    let mut resp = login(&router, THROTTLED_EMAIL, WRONG_PASSWORD).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let first_401_body = body_string(&mut resp).await;

    let mut resp = login(&router, THROTTLED_EMAIL, WRONG_PASSWORD).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let second_401_body = body_string(&mut resp).await;

    // The lock is set by the second failure; every further attempt — with
    // the CORRECT password included — is 429 for the lock's lifetime.
    let mut resp = login(&router, THROTTLED_EMAIL, REAL_PASSWORD).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after = resp
        .headers()
        .get("retry-after")
        .expect("429 carries Retry-After")
        .to_str()
        .expect("ASCII header value")
        .parse::<i64>()
        .expect("Retry-After is an integer");
    assert!(retry_after >= 1, "Retry-After must be a positive wait");
    let the_429_body = body_string(&mut resp).await;

    // The two 401s are identical to each other (non-enumeration within
    // one key), and the 429 body is its own constant, not a leaked
    // upstream message.
    assert_eq!(first_401_body, second_401_body);
    assert!(the_429_body.contains("Too many failed sign-in attempts"));
}

#[tokio::test]
async fn a_nonexistent_email_gets_the_same_statuses_and_bodies_as_a_real_one() {
    let real = spin_up_with_env(&env_with(MAX_FAILURES, "300")).await;
    seed_password_identity(&real.pool).await;
    let mut real_bodies = Vec::new();
    let mut real_statuses = Vec::new();
    for password in [WRONG_PASSWORD, WRONG_PASSWORD, REAL_PASSWORD] {
        let mut resp = login(&real.router, THROTTLED_EMAIL, password).await;
        real_statuses.push(resp.status());
        real_bodies.push(body_string(&mut resp).await);
    }

    let ghost = spin_up_with_env(&env_with(MAX_FAILURES, "300")).await;
    let mut ghost_bodies = Vec::new();
    let mut ghost_statuses = Vec::new();
    for password in [WRONG_PASSWORD, WRONG_PASSWORD, REAL_PASSWORD] {
        let mut resp = login(&ghost.router, GHOST_EMAIL, password).await;
        ghost_statuses.push(resp.status());
        ghost_bodies.push(body_string(&mut resp).await);
    }

    assert_eq!(real_statuses, ghost_statuses);
    assert_eq!(real_bodies, ghost_bodies);
}

#[tokio::test]
async fn a_correct_login_before_the_limit_resets_the_failure_count() {
    let TestApp { router, pool } = spin_up_with_env(&env_with(MAX_FAILURES, "300")).await;
    seed_password_identity(&pool).await;

    let resp = login(&router, THROTTLED_EMAIL, WRONG_PASSWORD).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let resp = login(&router, THROTTLED_EMAIL, REAL_PASSWORD).await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Success clears the stored count for this key entirely.
    let cleared: Option<(i32,)> =
        sqlx::query_as("SELECT failures FROM login_throttle WHERE key_hash = $1")
            .bind(lakehouse_auth::throttle::key_for(THROTTLED_EMAIL))
            .fetch_optional(&pool)
            .await
            .expect("query login_throttle");
    assert!(
        cleared.is_none(),
        "a successful login must clear the throttle row"
    );

    // One further failure is therefore only failure #1 since the reset:
    // had the reset not happened, this would be #2 and would lock.
    let resp = login(&router, THROTTLED_EMAIL, REAL_PASSWORD).await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "one failure since the reset must not lock"
    );
}

#[tokio::test]
async fn the_lock_sets_exactly_one_audit_event_naming_neither_the_email_nor_the_password() {
    let TestApp { router, pool } = spin_up_with_env(&env_with(MAX_FAILURES, "300")).await;
    seed_password_identity(&pool).await;

    let resp = login(&router, THROTTLED_EMAIL, WRONG_PASSWORD).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let resp = login(&router, THROTTLED_EMAIL, WRONG_PASSWORD).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let (count, resource_id, args): (i64, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT count(*), max(resource_id), max(args::text) FROM audit_event \
         WHERE action = 'auth.login_locked'",
    )
    .fetch_one(&pool)
    .await
    .expect("query audit_event");
    assert_eq!(count, 1, "one lock, one audit event");
    let resource_id = resource_id.expect("resource_id on the lock event");
    assert_eq!(resource_id.len(), 12, "resource_id is the first 12 hex");
    assert!(
        !resource_id.chars().any(|c| !c.is_ascii_hexdigit()),
        "resource_id is hex, not a raw identifier"
    );
    let args = args.unwrap_or_default();
    for forbidden in [THROTTLED_EMAIL, WRONG_PASSWORD] {
        assert!(
            !args.contains(forbidden),
            "audit args must not carry {forbidden}"
        );
    }
}

#[tokio::test]
async fn a_storage_failure_during_login_is_not_rendered_or_counted_as_a_wrong_password() {
    let TestApp { router, pool } = spin_up_with_env(&env_with(MAX_FAILURES, "300")).await;
    seed_password_identity(&pool).await;

    // Break the login lookup itself: `password::verify` SELECTs from
    // `auth_identity`, so renaming the table turns every login into a
    // storage error for the duration of the rename. A storage error must
    // surface as its classified 500 — never as the non-enumerated 401 a
    // wrong password gets, and never as a counted failure (review
    // blocker 1, 2026-10-03: five database hiccups would otherwise lock
    // a real user out).
    sqlx::query("ALTER TABLE auth_identity RENAME TO auth_identity_renamed")
        .execute(&pool)
        .await
        .expect("rename auth_identity to simulate a storage failure");
    let mut resp = login(&router, THROTTLED_EMAIL, WRONG_PASSWORD).await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let wrong_password_body = body_string(&mut resp).await;
    let mut resp = login(&router, THROTTLED_EMAIL, REAL_PASSWORD).await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let correct_password_body = body_string(&mut resp).await;
    sqlx::query("ALTER TABLE auth_identity_renamed RENAME TO auth_identity")
        .execute(&pool)
        .await
        .expect("restore auth_identity");

    assert!(
        wrong_password_body.contains("authentication error"),
        "a storage failure renders its classified message, got: {wrong_password_body}"
    );
    assert!(
        correct_password_body.contains("authentication error"),
        "the classified message must not depend on the password's correctness, got: {correct_password_body}"
    );
    assert!(
        !wrong_password_body.contains("Too many failed sign-in"),
        "a storage failure must not render as a lockout"
    );

    // Nothing was recorded against the key: no throttle row exists, and
    // once the lookup works again the CORRECT password signs in on the
    // first try (and a wrong one would still have two failures of
    // headroom, not zero).
    let (rows,): (i64,) = sqlx::query_as("SELECT count(*) FROM login_throttle")
        .fetch_one(&pool)
        .await
        .expect("query login_throttle");
    assert_eq!(rows, 0, "a storage failure must not count toward the lock");
    let resp = login(&router, THROTTLED_EMAIL, REAL_PASSWORD).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn an_expired_lock_lets_the_correct_password_through_again() {
    let TestApp { router, pool } = spin_up_with_env(&env_with(MAX_FAILURES, "1")).await;
    seed_password_identity(&pool).await;

    let resp = login(&router, THROTTLED_EMAIL, WRONG_PASSWORD).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let resp = login(&router, THROTTLED_EMAIL, WRONG_PASSWORD).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let resp = login(&router, THROTTLED_EMAIL, REAL_PASSWORD).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

    // LOGIN_LOCKOUT_SECS=1: wait out the lock plus a margin for the
    // one-second granularity of the locked_until comparison.
    tokio::time::sleep(std::time::Duration::from_millis(1300)).await;

    let resp = login(&router, THROTTLED_EMAIL, REAL_PASSWORD).await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "a correct password after the lock expires must sign in"
    );
}
