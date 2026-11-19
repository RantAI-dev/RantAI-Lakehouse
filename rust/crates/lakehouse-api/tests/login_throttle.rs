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

use crate::common;

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use common::{TestApp, audit_rows, spin_up_with_env};

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
    login_with_user_agent(app, email, password, None).await
}

/// [`login`] with the browser the sign-in claims to come from (`ADM-14`):
/// the header the session records, or none when the request carries none.
async fn login_with_user_agent(
    app: &axum::Router,
    email: &str,
    password: &str,
    user_agent: Option<&str>,
) -> axum::http::Response<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header("content-type", "application/json");
    if let Some(user_agent) = user_agent {
        request = request.header("user-agent", user_agent);
    }
    app.clone()
        .oneshot(
            request
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

    // The row as the trail holds it: one row, and every field of it read
    // here.
    let rows = audit_rows(&pool, "auth.login_locked").await;
    assert_eq!(rows.len(), 1, "one lock, one audit event");
    let row = &rows[0];
    // QA LH-SEC-2-06 (SEC-70): the event happened before anybody signed in,
    // so it carries no principal at all and says what caused it in
    // `actor_label` instead of being handed to the assistant by the list.
    assert_eq!(
        row["actor_label"].as_str(),
        Some(lakehouse_store::audit::ACTOR_SIGN_IN_FORM),
        "the lock names the sign-in form as its cause"
    );
    assert!(
        row["principal_kind"].is_null(),
        "no principal kind: nobody signed in"
    );
    assert!(
        row["principal_id"].is_null(),
        "no principal id: nobody signed in"
    );
    // Plan D5b: the row holds no identifier material. It used to carry the
    // first 12 hex characters of the identifier's unsalted SHA-256 in
    // `resource_id` — the hash a reader could match against guessed emails,
    // and a prefix of the hash of a password typed into the identifier box.
    // The old assertion here described that leak.
    assert!(
        row["resource_kind"].is_null(),
        "the lock row names no resource kind at all"
    );
    assert!(
        row["resource_id"].is_null(),
        "the lock row holds no id, not even a hash prefix of the identifier"
    );
    assert_eq!(
        row["detail"].as_str(),
        Some("Too many failed sign-ins; this sign-in is locked for a while."),
        "the lock row says only what happened"
    );
    // Nothing of the identifier or of its throttle key is anywhere in the
    // row, and the password is in none of it either.
    let key = lakehouse_auth::throttle::key_for(THROTTLED_EMAIL);
    let text = row.to_string();
    for forbidden in [THROTTLED_EMAIL, WRONG_PASSWORD, &key[..12]] {
        assert!(
            !text.contains(forbidden),
            "the lock row must not carry {forbidden}: {text}"
        );
    }
    let args = sqlx::query_scalar::<_, Option<String>>(
        "SELECT max(args::text) FROM audit_event WHERE action = 'auth.login_locked'",
    )
    .fetch_one(&pool)
    .await
    .expect("query audit_event");
    let args = args.unwrap_or_default();
    for forbidden in [THROTTLED_EMAIL, WRONG_PASSWORD] {
        assert!(
            !args.contains(forbidden),
            "audit args must not carry {forbidden}"
        );
    }
}

/// QA LH-SEC-2-06 (SEC-70), decision D4: the lockout happened before anybody
/// signed in, so the audit list shows what caused it — the sign-in form, a
/// service, the system — never the assistant.
#[tokio::test]
async fn a_lock_is_listed_in_the_audit_log_as_the_sign_in_forms_and_not_the_assistants() {
    let TestApp { router, pool } = spin_up_with_env(&env_with(MAX_FAILURES, "300")).await;
    // `SEC-20` (plan 100): the trail is read per tenant, and the lock is a
    // row nobody signed in caused, so it belongs to the deployment and only
    // the tenant that owns the shared catalog reads it. This test is about
    // the row's actor, so the deployment becomes a one-tenant one and the
    // reach covers every row.
    common::leave_no_seeded_tenants(&pool).await;
    seed_password_identity(&pool).await;
    // Walk the account into the lock exactly as the audit test above does.
    for _ in 0..2 {
        let resp = login(&router, THROTTLED_EMAIL, WRONG_PASSWORD).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    let cookie = common::session_cookie_for_seeded_user(&pool, THROTTLED_EMAIL).await;
    let mut resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/governance/audit")
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&body_string(&mut resp).await).expect("audit body is JSON");
    let entry = body["audit"]
        .as_array()
        .expect("audit is an array")
        .iter()
        .find(|row| row["action"] == "auth.login_locked")
        .expect("the lock is listed");
    assert_eq!(entry["actor"], "Sign-in form");
    assert_eq!(entry["actorKind"], "service");
    assert_eq!(entry["source"], "system");
}

/// QA LH-SEC-5-01 (`ADM-14` decisions D9 and D14): a sign-in records the
/// browser it came from, records no network address (the API has no trusted
/// source for one), and creates exactly one session — the six rows QA saw for
/// one administrator are six sign-ins, not one sign-in that repeats.
#[tokio::test]
async fn a_sign_in_records_the_browser_and_creates_exactly_one_session() {
    let TestApp { router, pool } = spin_up_with_env(&env_with(MAX_FAILURES, "300")).await;
    seed_password_identity(&pool).await;

    let resp = login_with_user_agent(
        &router,
        THROTTLED_EMAIL,
        REAL_PASSWORD,
        Some("QA-Browser/1.0"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let (count, user_agent, created_ip): (i64, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT count(*), max(s.user_agent), max(s.created_ip) FROM session s \
         JOIN app_user u ON u.id = s.app_user_id WHERE u.email = $1",
    )
    .bind(THROTTLED_EMAIL)
    .fetch_one(&pool)
    .await
    .expect("query session");
    assert_eq!(count, 1, "one sign-in, one session");
    assert_eq!(user_agent.as_deref(), Some("QA-Browser/1.0"));
    assert_eq!(
        created_ip, None,
        "the network address is not recorded (ADM-14 D10)"
    );
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
    let TestApp { router, pool } = spin_up_with_env(&env_with(MAX_FAILURES, "300")).await;
    seed_password_identity(&pool).await;

    let resp = login(&router, THROTTLED_EMAIL, WRONG_PASSWORD).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let resp = login(&router, THROTTLED_EMAIL, WRONG_PASSWORD).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let resp = login(&router, THROTTLED_EMAIL, REAL_PASSWORD).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

    // SEC-49 (PR #236 CI): the lock used to last one second and the test
    // slept it out, but the second wrong try hashes with Argon2 AFTER the
    // lock is set, so on a slow runner the lock expired before the 429
    // check above. The lock is now long (so the refusal cannot race the
    // clock) and expiry is made by moving `locked_until` into the past:
    // the product still decides by comparing it to the clock, and nothing
    // in the lock behaviour is loosened.
    sqlx::query(
        "UPDATE login_throttle SET locked_until = now() - interval '1 second' \
         WHERE locked_until IS NOT NULL",
    )
    .execute(&pool)
    .await
    .expect("expire the lock");

    let resp = login(&router, THROTTLED_EMAIL, REAL_PASSWORD).await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "a correct password after the lock expires must sign in"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn twenty_concurrent_wrong_logins_let_at_most_the_limit_reach_the_password_check() {
    // SEC-49: a 401 is only produced after the password verifier ran, a 429
    // never reaches it. So the number of 401s is the number of tries that
    // were hashed; it must not exceed the limit however many arrive at once.
    const LIMIT: usize = 5;
    let TestApp { router, pool } = spin_up_with_env(&env_with("5", "300")).await;
    seed_password_identity(&pool).await;

    let mut handles = Vec::new();
    for _ in 0..20 {
        let router = router.clone();
        handles.push(tokio::spawn(async move {
            login(&router, THROTTLED_EMAIL, WRONG_PASSWORD)
                .await
                .status()
        }));
    }
    let mut verified = 0;
    let mut refused = 0;
    for handle in handles {
        match handle.await.expect("login task") {
            StatusCode::UNAUTHORIZED => verified += 1,
            StatusCode::TOO_MANY_REQUESTS => refused += 1,
            other => panic!("unexpected status {other}"),
        }
    }
    assert_eq!(
        verified, LIMIT,
        "only the first {LIMIT} tries may be hashed"
    );
    assert_eq!(refused, 20 - LIMIT);

    // The correct password is refused for the lock's lifetime.
    let resp = login(&router, THROTTLED_EMAIL, REAL_PASSWORD).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
}

/// `GOV-1`: a sign-in that works is on the audit trail. Nobody was signed in
/// when the form was submitted, so the actor is the form; the principal is
/// known only because the password verified.
#[tokio::test]
async fn a_successful_sign_in_writes_one_auth_login_row_without_the_email_or_the_password() {
    let TestApp { router, pool } = spin_up_with_env(&env_with(MAX_FAILURES, "300")).await;
    seed_password_identity(&pool).await;

    let mut resp = login(&router, THROTTLED_EMAIL, REAL_PASSWORD).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let answer = body_string(&mut resp).await;
    let user_id =
        serde_json::from_str::<serde_json::Value>(&answer).expect("login body is JSON")["id"]
            .as_str()
            .expect("login body names the user")
            .to_owned();

    let rows = audit_rows(&pool, "auth.login").await;
    assert_eq!(rows.len(), 1, "one sign-in, one row: {rows:?}");
    let row = &rows[0];
    assert_eq!(row["outcome"], "executed");
    assert_eq!(
        row["actor_label"],
        lakehouse_store::audit::ACTOR_SIGN_IN_FORM
    );
    assert_eq!(row["principal_id"], user_id.as_str());
    assert_eq!(row["principal_kind"], "user");
    assert_eq!(row["resource_kind"], "user");
    assert_eq!(row["resource_id"], user_id.as_str());
    let text = row.to_string();
    for forbidden in [THROTTLED_EMAIL, REAL_PASSWORD] {
        assert!(
            !text.contains(forbidden),
            "the row must not carry {forbidden}: {text}"
        );
    }
}

/// `GOV-1`: a refused sign-in is on the audit trail too, with no principal
/// (nobody proved who they were) and nothing the visitor typed.
#[tokio::test]
async fn a_wrong_password_writes_one_refused_auth_login_row_without_the_email_or_the_password() {
    let TestApp { router, pool } = spin_up_with_env(&env_with(MAX_FAILURES, "300")).await;
    seed_password_identity(&pool).await;

    let resp = login(&router, THROTTLED_EMAIL, WRONG_PASSWORD).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    // The same sentence for an account that does not exist: the trail must
    // not become the oracle the response is built not to be.
    let resp = login(&router, GHOST_EMAIL, WRONG_PASSWORD).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let rows = audit_rows(&pool, "auth.login").await;
    assert_eq!(rows.len(), 2, "two refused tries, two rows: {rows:?}");
    for row in &rows {
        assert_eq!(row["outcome"], "refused");
        assert_eq!(
            row["actor_label"],
            lakehouse_store::audit::ACTOR_SIGN_IN_FORM
        );
        assert!(row["principal_id"].is_null(), "nobody signed in: {row}");
        assert!(row["principal_kind"].is_null(), "nobody signed in: {row}");
        assert_eq!(row["detail"], "The email or the password did not match.");
        // `GOV-1 review`: the first 48 bits of an unsalted hash of what the
        // visitor typed would let a reader of the trail test guessed emails
        // against it (and a password typed into the identifier box leaks a
        // prefix of its hash), so the row names no key at all.
        assert!(row["resource_id"].is_null(), "no key prefix: {row}");
        let text = row.to_string();
        for forbidden in [THROTTLED_EMAIL, GHOST_EMAIL, WRONG_PASSWORD, REAL_PASSWORD] {
            assert!(
                !text.contains(forbidden),
                "the row must not carry {forbidden}: {text}"
            );
        }
    }
    assert_eq!(
        rows[0]["detail"], rows[1]["detail"],
        "an existing and a missing account read the same"
    );
}

/// `GOV-1`: signing out is on the audit trail, naming the person and never
/// the session cookie.
#[tokio::test]
async fn signing_out_writes_one_auth_logout_row_for_the_signed_in_user() {
    let TestApp { router, pool } = spin_up_with_env(&env_with(MAX_FAILURES, "300")).await;
    let cookie = common::session_cookie_for_seeded_user(&pool, THROTTLED_EMAIL).await;
    let (user_id,): (uuid::Uuid,) = sqlx::query_as("SELECT id FROM app_user WHERE email = $1")
        .bind(THROTTLED_EMAIL)
        .fetch_one(&pool)
        .await
        .expect("seeded user");

    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let rows = audit_rows(&pool, "auth.logout").await;
    assert_eq!(rows.len(), 1, "one sign-out, one row: {rows:?}");
    let row = &rows[0];
    assert_eq!(row["outcome"], "executed");
    assert_eq!(row["principal_id"], user_id.to_string().as_str());
    assert_eq!(row["principal_kind"], "user");
    assert!(row["actor_label"].is_string(), "the person is named: {row}");
    let token = cookie.trim_start_matches("lh_session=");
    assert!(
        !row.to_string().contains(token),
        "the row must not carry the session token: {row}"
    );
}

async fn change_password(
    app: &axum::Router,
    cookie: &str,
    body: serde_json::Value,
) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/change-password")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright")
}

const NEW_PASSWORD: &str = "An0ther!Passw0rd7";

/// `GOV-1`: a password change is on the audit trail, and neither password is.
#[tokio::test]
async fn changing_the_password_writes_one_auth_password_change_row_without_either_password() {
    let TestApp { router, pool } = spin_up_with_env(&env_with(MAX_FAILURES, "300")).await;
    seed_password_identity(&pool).await;
    let cookie = common::session_cookie_for_seeded_user(&pool, THROTTLED_EMAIL).await;

    let resp = change_password(
        &router,
        &cookie,
        serde_json::json!({ "oldPassword": REAL_PASSWORD, "newPassword": NEW_PASSWORD }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let rows = audit_rows(&pool, "auth.password_change").await;
    assert_eq!(rows.len(), 1, "one change, one row: {rows:?}");
    let row = &rows[0];
    assert_eq!(row["outcome"], "executed");
    assert_eq!(row["principal_kind"], "user");
    assert_eq!(row["resource_kind"], "user");
    assert_eq!(
        row["resource_id"], row["principal_id"],
        "the caller's own account"
    );
    let text = row.to_string();
    for forbidden in [REAL_PASSWORD, NEW_PASSWORD] {
        assert!(
            !text.contains(forbidden),
            "the row must not carry a password: {text}"
        );
    }
}

/// `GOV-1`: a refused password change (the current password is wrong, or the
/// new one breaks the rules) is on the audit trail as refused.
#[tokio::test]
async fn a_refused_password_change_writes_a_refused_auth_password_change_row() {
    let TestApp { router, pool } = spin_up_with_env(&env_with(MAX_FAILURES, "300")).await;
    seed_password_identity(&pool).await;
    let cookie = common::session_cookie_for_seeded_user(&pool, THROTTLED_EMAIL).await;

    let resp = change_password(
        &router,
        &cookie,
        serde_json::json!({ "oldPassword": WRONG_PASSWORD, "newPassword": NEW_PASSWORD }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let resp = change_password(
        &router,
        &cookie,
        serde_json::json!({ "oldPassword": REAL_PASSWORD, "newPassword": "short" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let rows = audit_rows(&pool, "auth.password_change").await;
    assert_eq!(rows.len(), 2, "two refusals, two rows: {rows:?}");
    for row in &rows {
        assert_eq!(row["outcome"], "refused");
        assert_eq!(row["principal_kind"], "user");
        let text = row.to_string();
        for forbidden in [REAL_PASSWORD, WRONG_PASSWORD, NEW_PASSWORD, "short"] {
            assert!(
                !text.contains(forbidden),
                "the row must not carry a password: {text}"
            );
        }
    }
    assert_eq!(rows[0]["detail"], "The current password did not match.");
    assert_eq!(
        rows[1]["detail"],
        "The new password does not meet the password rules."
    );
}
