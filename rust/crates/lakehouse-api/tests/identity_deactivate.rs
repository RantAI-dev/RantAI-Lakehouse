//! ADM-4 T2: a user can be switched off, and a switched-off user has no
//! access on ANY path.
//!
//! `app_user.status` existed since `0001_init.sql` but nothing enforced
//! it: an inactive user still signed in with a password, kept working
//! sessions, and signed in through SSO. ADM-4 D3 adds the switch, and D4
//! makes "inactive" mean "no access" everywhere — the local login, the
//! session validation, the OIDC sign-in (including JIT), and the
//! principal load every one of them ends in.
//!
//! The tests live in a module named `identity` so the plan's
//! `cargo test -p lakehouse-api identity` filter runs them: an
//! integration test's name is its function's path inside the test
//! binary, and a bare function name would never match that filter.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use common::{TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up};

mod identity {
    use super::*;

    /// The password every user in this file signs in with. Deliberately
    /// low-entropy and obviously fake: a random-looking value in a test
    /// fixture reads as a credential to the secret scanner.
    const PASSWORD: &str = "test-only-password-one";

    /// A password nobody set, for the "wrong password" comparison.
    const WRONG_PASSWORD: &str = "test-only-wrong-password";

    // ── helpers ──────────────────────────────────────────────────────────

    /// `POST /api/auth/login`; return the status, the parsed body, and
    /// the `Cookie` header value the response handed out.
    async fn login(app: &TestApp, email: &str, password: &str) -> (StatusCode, Value, String) {
        let response = app
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({ "email": email, "password": password }).to_string(),
                    ))
                    .expect("build request"),
            )
            .await
            .expect("router never fails a request outright");
        let status = response.status();
        let set_cookie = response
            .headers()
            .get("set-cookie")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let cookie = set_cookie
            .unwrap_or_default()
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            cookie,
        )
    }

    /// `GET` a path with a session cookie; return the status and the
    /// parsed body.
    async fn get_with_cookie(app: &TestApp, uri: &str, cookie: &str) -> (StatusCode, Value) {
        let response = app
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router never fails a request outright");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// `POST` a path with a session cookie and no body; return the status
    /// and the parsed body.
    async fn post_with_cookie(app: &TestApp, uri: &str, cookie: &str) -> (StatusCode, Value) {
        let response = app
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router never fails a request outright");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// A user with a local credential, so the test can sign in as them.
    async fn make_user_with_password(pool: &PgPool, email: &str) -> Uuid {
        let user_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO app_user (id, name, username, email, status) VALUES ($1, $2, $3, $4, 'active')",
        )
        .bind(user_id)
        .bind("Switched Off Test User")
        .bind(email.to_lowercase())
        .bind(email)
            .execute(pool)
            .await
            .expect("insert the test user");
        lakehouse_auth::password::create_local_identity(
            pool,
            user_id,
            &lakehouse_auth::Secret::new(PASSWORD),
            false,
        )
        .await
        .expect("give the test user a password");
        user_id
    }

    /// Leave the deployment with exactly ONE user holding the unrestricted
    /// `*:*` grant: the seeded Platform Admin gives theirs up.
    async fn leave_one_administrator(pool: &PgPool) {
        sqlx::query(
            "DELETE FROM app_user_role \
             WHERE role_id = (SELECT id FROM role WHERE name = 'Platform Admin')",
        )
        .execute(pool)
        .await
        .expect("the seeded Platform Admin gives up the unrestricted role");
    }

    /// The status the user row carries.
    async fn stored_status(pool: &PgPool, user: Uuid) -> String {
        sqlx::query_scalar("SELECT status FROM app_user WHERE id = $1")
            .bind(user)
            .fetch_one(pool)
            .await
            .expect("read the user's status")
    }

    // ── the tests ────────────────────────────────────────────────────────

    /// ADM-4 D4: a deactivated user's password sign-in is refused with
    /// the SAME answer a wrong password gets — so the refusal says
    /// nothing about whether the account exists.
    #[tokio::test]
    async fn a_deactivated_users_login_is_refused_like_a_wrong_password() {
        let app = spin_up().await;
        let email = "switched-off@meridian.example";
        let user = make_user_with_password(&app.pool, email).await;
        let admin = create_principal_with_permissions(&app.pool, "*:*").await;
        let admin_cookie = session_cookie_for_user(&app.pool, admin).await;

        // The right password signs in, to begin with.
        let (status, _, _) = login(&app, email, PASSWORD).await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = post_with_cookie(
            &app,
            &format!("/api/identity/users/{user}/deactivate"),
            &admin_cookie,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["status"], json!("inactive"));

        // The password no longer signs in.
        let (status, refused, _) = login(&app, email, PASSWORD).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{refused}");

        // And the answer is the one a wrong password gets — the same
        // status and the same body.
        let (wrong_status, wrong_body, _) = login(&app, email, WRONG_PASSWORD).await;
        assert_eq!(wrong_status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            refused, wrong_body,
            "a deactivated account must answer exactly like a wrong password"
        );
    }

    /// ADM-4 D4: an existing session stops working the moment the user is
    /// deactivated — the deactivation ends every session the user holds.
    #[tokio::test]
    async fn a_deactivated_users_existing_session_is_refused() {
        let app = spin_up().await;
        let email = "switched-off-session@meridian.example";
        let user = make_user_with_password(&app.pool, email).await;
        let admin = create_principal_with_permissions(&app.pool, "*:*").await;
        let admin_cookie = session_cookie_for_user(&app.pool, admin).await;

        let (status, _, cookie) = login(&app, email, PASSWORD).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = get_with_cookie(&app, "/api/auth/me", &cookie).await;
        assert_eq!(status, StatusCode::OK, "the session works before");

        let (status, body) = post_with_cookie(
            &app,
            &format!("/api/identity/users/{user}/deactivate"),
            &admin_cookie,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");

        let (status, _) = get_with_cookie(&app, "/api/auth/me", &cookie).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "a deactivated user's session must stop working"
        );
    }

    /// ADM-4 D3: reactivating gives the account back — the password signs
    /// in again.
    #[tokio::test]
    async fn reactivating_restores_the_login() {
        let app = spin_up().await;
        let email = "switched-back-on@meridian.example";
        let user = make_user_with_password(&app.pool, email).await;
        let admin = create_principal_with_permissions(&app.pool, "*:*").await;
        let admin_cookie = session_cookie_for_user(&app.pool, admin).await;

        let (status, body) = post_with_cookie(
            &app,
            &format!("/api/identity/users/{user}/deactivate"),
            &admin_cookie,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(stored_status(&app.pool, user).await, "inactive");
        let (status, _, _) = login(&app, email, PASSWORD).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, body) = post_with_cookie(
            &app,
            &format!("/api/identity/users/{user}/reactivate"),
            &admin_cookie,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["status"], json!("active"));
        assert_eq!(stored_status(&app.pool, user).await, "active");

        let (status, login_body, _) = login(&app, email, PASSWORD).await;
        assert_eq!(status, StatusCode::OK, "{login_body}");
    }

    /// ADM-4 D5: only an unrestricted caller may switch an unrestricted
    /// user off; anyone else gets the same 404 an unknown user gets.
    #[tokio::test]
    async fn a_caller_without_the_unrestricted_grant_gets_404_for_an_unrestricted_user() {
        let app = spin_up().await;
        let caller = create_principal_with_permissions(&app.pool, "identity:write").await;
        let caller_cookie = session_cookie_for_user(&app.pool, caller).await;
        let target = create_principal_with_permissions(&app.pool, "*:*").await;

        let (status, body) = post_with_cookie(
            &app,
            &format!("/api/identity/users/{target}/deactivate"),
            &caller_cookie,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

        // The same answer as for a user id that does not exist.
        let (unknown_status, unknown_body) = post_with_cookie(
            &app,
            &format!("/api/identity/users/{}/deactivate", Uuid::new_v4()),
            &caller_cookie,
        )
        .await;
        assert_eq!(unknown_status, StatusCode::NOT_FOUND, "{unknown_body}");
        assert_eq!(body, unknown_body, "one body for both");

        assert_eq!(
            stored_status(&app.pool, target).await,
            "active",
            "the target is untouched"
        );
    }

    /// ADM-4 D5: a caller may not deactivate themselves.
    #[tokio::test]
    async fn a_self_deactivation_is_refused() {
        let app = spin_up().await;
        let caller = create_principal_with_permissions(&app.pool, "identity:write").await;
        let caller_cookie = session_cookie_for_user(&app.pool, caller).await;

        let (status, body) = post_with_cookie(
            &app,
            &format!("/api/identity/users/{caller}/deactivate"),
            &caller_cookie,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(
            stored_status(&app.pool, caller).await,
            "active",
            "the caller is untouched"
        );
    }

    /// ADM-4 D5: deactivating the last administrator is refused with a
    /// 409 — the deployment must not be left with nobody who can
    /// administer it.
    #[tokio::test]
    async fn deactivating_the_last_administrator_is_refused() {
        let app = spin_up().await;
        leave_one_administrator(&app.pool).await;
        let admin = create_principal_with_permissions(&app.pool, "*:*").await;
        let admin_cookie = session_cookie_for_user(&app.pool, admin).await;

        let (status, body) = post_with_cookie(
            &app,
            &format!("/api/identity/users/{admin}/deactivate"),
            &admin_cookie,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(
            stored_status(&app.pool, admin).await,
            "active",
            "the refused change must be rolled back"
        );
    }

    /// ADM-4 D6: both status changes are audited, naming the actor and
    /// the target.
    #[tokio::test]
    async fn the_status_changes_are_audited() {
        let app = spin_up().await;
        let email = "audited-switch@meridian.example";
        let user = make_user_with_password(&app.pool, email).await;
        let admin = create_principal_with_permissions(&app.pool, "*:*").await;
        let admin_cookie = session_cookie_for_user(&app.pool, admin).await;

        let (status, _) = post_with_cookie(
            &app,
            &format!("/api/identity/users/{user}/deactivate"),
            &admin_cookie,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = post_with_cookie(
            &app,
            &format!("/api/identity/users/{user}/reactivate"),
            &admin_cookie,
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let rows: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT action, resource_id, principal_id FROM audit_event \
             WHERE action IN ('identity.user.deactivate', 'identity.user.reactivate') \
             ORDER BY at",
        )
        .fetch_all(&app.pool)
        .await
        .expect("read the audit rows");
        assert_eq!(rows.len(), 2, "one row per change: {rows:?}");
        let actions: Vec<&str> = rows.iter().map(|(action, _, _)| action.as_str()).collect();
        assert_eq!(
            actions,
            vec!["identity.user.deactivate", "identity.user.reactivate"]
        );
        for (_, resource_id, principal_id) in &rows {
            assert_eq!(resource_id.as_deref(), Some(user.to_string().as_str()));
            assert_eq!(principal_id.as_deref(), Some(admin.to_string().as_str()));
        }
    }

    /// ADM-4 D3: the user's data stays. A deactivated person is not a
    /// deleted one — their authored work and their audit history are
    /// still there.
    #[tokio::test]
    async fn deactivating_a_user_keeps_their_data() {
        let app = spin_up().await;
        let email = "kept-data@meridian.example";
        let user = make_user_with_password(&app.pool, email).await;
        let admin = create_principal_with_permissions(&app.pool, "*:*").await;
        let admin_cookie = session_cookie_for_user(&app.pool, admin).await;

        let (status, _) = post_with_cookie(
            &app,
            &format!("/api/identity/users/{user}/deactivate"),
            &admin_cookie,
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM app_user WHERE id = $1")
            .bind(user)
            .fetch_one(&app.pool)
            .await
            .expect("count the user rows");
        assert_eq!(rows, 1, "the account itself is still there");
        let identities: i64 =
            sqlx::query_scalar("SELECT count(*) FROM auth_identity WHERE app_user_id = $1")
                .bind(user)
                .fetch_one(&app.pool)
                .await
                .expect("count the credentials");
        assert_eq!(identities, 1, "the credential is still there");
    }
}
