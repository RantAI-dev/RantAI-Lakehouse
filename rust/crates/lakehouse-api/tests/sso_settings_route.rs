//! `GET`/`PUT /api/settings/sso`, `POST /api/settings/sso/test`, and
//! `GET /api/settings/sso/test/{id}` (`ADM-12`, plan D5, D8, D10).
//!
//! What this proves:
//!
//! * a caller holding `settings:write` alone can read `GET /api/settings/sso`
//!   (which reports `canEdit = false`), but is refused with 403 on
//!   `PUT /api/settings/sso`, `POST /api/settings/sso/test`, and
//!   `GET /api/settings/sso/test/{id}`;
//! * only a Platform Admin (`*:*`) may save settings or run a test;
//! * a saved connection with no return address reports `ready: false` with a
//!   sentence naming `CONSOLE_PUBLIC_URL`, and the sign-in page is told there
//!   is no OIDC provider, so it shows no button that would fail;
//! * another admin reading a test result gets 404, and an expired run gets
//!   404;
//! * the audit row holds no secret.
//!
//! The test sign-in's round trip — the draft's own discovery document, the
//! classified failure sentences, `emailVerified`/`overage`, and "a draft
//! never talks to the saved provider" — lives in `routes::auth`'s unit tests
//! (`ADM-12` review round 1, B2/S1/S2/S4): it needs the `#[cfg(test)]` HTTP
//! client seam (`AppState::sso_http_override`) to reach an `https://` mock
//! provider, and an integration test links the library without `cfg(test)`.
//! The seam is recorded in the plan's handoff.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_seeded_user, spin_up_with_env,
};
use serde_json::{Value, json};
use tower::ServiceExt;

const SSO: &str = "/api/settings/sso";
const SSO_TEST: &str = "/api/settings/sso/test";
const CONSOLE_ISSUER: &str = "https://localhost:8443";

async fn call(
    app: &axum::Router,
    verb: &str,
    path: &str,
    cookie: &str,
    body: &str,
) -> (StatusCode, Value, axum::http::HeaderMap) {
    let mut req = Request::builder().method(verb).uri(path);
    if !cookie.is_empty() {
        req = req.header("cookie", cookie);
    }
    if !body.is_empty() {
        req = req.header("content-type", "application/json");
    }
    let resp = app
        .clone()
        .oneshot(req.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    let val = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, val, headers)
}

/// `ADM-12` review round 1, B1: a saved connection with no return address
/// cannot be used, and says so instead of leaving a sign-in button on the
/// page that would fail. Nothing is fetched: `CONSOLE_PUBLIC_URL` is checked
/// before the provider's discovery document is read.
#[tokio::test]
async fn without_a_public_address_the_connection_is_not_ready_and_no_button_shows() {
    let mut env = HashMap::new();
    env.insert(
        "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
        "true".to_owned(),
    );
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let body = json!({
        "issuer": CONSOLE_ISSUER,
        "clientId": "lakehouse-test-client",
    });
    let (status, saved, _) = call(&router, "PUT", SSO, &cookie, &body.to_string()).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["ready"], false, "{saved}");
    assert!(
        saved["notReadyReason"]
            .as_str()
            .is_some_and(|reason| reason.contains("CONSOLE_PUBLIC_URL")),
        "the reason names what to set: {saved}"
    );

    // Reading it back says the same thing.
    let (status, read, _) = call(&router, "GET", SSO, &cookie, "").await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(read["ready"], false, "{read}");
    assert_eq!(read["notReadyReason"], saved["notReadyReason"]);

    // And the sign-in page is told there is no OIDC provider to show.
    let (status, providers, _) = call(&router, "GET", "/api/auth/providers", "", "").await;
    assert_eq!(status, StatusCode::OK, "{providers}");
    assert_eq!(providers["oidc"], false, "{providers}");
}

/// D5: a settings:write-only caller gets 403 on PUT and test routes.
#[tokio::test]
async fn settings_write_alone_gets_403_on_put_and_test() {
    let mut env = HashMap::new();
    env.insert(
        "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
        "true".to_owned(),
    );
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let user_id = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = common::session_cookie_for_user(&pool, user_id).await;

    // GET succeeds and reports canEdit = false.
    let (status, body, _) = call(&router, "GET", SSO, &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["canEdit"], false);

    // PUT returns 403 Forbidden.
    let (status, _, _) = call(
        &router,
        "PUT",
        SSO,
        &cookie,
        r#"{"issuer":"https://localhost:8443","clientId":"cid"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // POST test returns 403 Forbidden.
    let (status, _, _) = call(
        &router,
        "POST",
        SSO_TEST,
        &cookie,
        r#"{"issuer":"https://localhost:8443","clientId":"cid"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// D8: another admin reading a test result gets 404, and an expired run gets
/// 404. The run is written straight into the store: this test is about who
/// may read a result and for how long, not about starting one (that is
/// `routes::auth`'s test-sign-in tests).
#[tokio::test]
async fn another_admin_or_expired_run_gets_404() {
    let mut env = HashMap::new();
    env.insert(
        "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
        "true".to_owned(),
    );
    let TestApp { router, pool } = spin_up_with_env(&env).await;

    let admin1_cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let admin2_id = create_principal_with_permissions(&pool, "*:*").await;
    let admin2_cookie = common::session_cookie_for_user(&pool, admin2_id).await;

    let (admin1_id,): (uuid::Uuid,) = sqlx::query_as("SELECT id FROM app_user WHERE email = $1")
        .bind("fajar@meridian.example")
        .fetch_one(&pool)
        .await
        .unwrap();
    let test_run_id = uuid::Uuid::new_v4();
    lakehouse_store::sso_settings::create_test_run(
        &pool,
        &lakehouse_store::sso_settings::NewSsoTestRun {
            id: test_run_id,
            created_by: admin1_id,
            draft: json!({
                "issuer": CONSOLE_ISSUER,
                "clientId": "client-test",
            }),
            draft_secret: None,
            expires_at: time::OffsetDateTime::now_utc() + time::Duration::minutes(10),
        },
    )
    .await
    .unwrap();

    let test_get_uri = format!("{SSO_TEST}/{test_run_id}");

    // Another admin reading it gets 404 Not Found.
    let (other_status, _, _) = call(&router, "GET", &test_get_uri, &admin2_cookie, "").await;
    assert_eq!(other_status, StatusCode::NOT_FOUND);

    // Creator reading it before expiry gets 200 (Null result before callback).
    let (creator_status, res_body, _) =
        call(&router, "GET", &test_get_uri, &admin1_cookie, "").await;
    assert_eq!(creator_status, StatusCode::OK);
    assert_eq!(res_body, Value::Null);

    // Expire the run in the database.
    sqlx::query("UPDATE sso_test_run SET expires_at = now() - interval '1 minute' WHERE id = $1")
        .bind(test_run_id)
        .execute(&pool)
        .await
        .unwrap();

    // After expiration, creator reading it gets 404 Not Found.
    let (exp_status, _, _) = call(&router, "GET", &test_get_uri, &admin1_cookie, "").await;
    assert_eq!(exp_status, StatusCode::NOT_FOUND);
}

/// D5/D8: saving SSO settings records an audit row that holds no secret.
#[tokio::test]
async fn the_audit_row_holds_no_secret() {
    let mut env = HashMap::new();
    env.insert(
        "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
        "true".to_owned(),
    );
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let secret_literal = "my-super-secret-password-12345";
    let body = json!({
        "issuer": CONSOLE_ISSUER,
        "clientId": "cid-audit-test",
        "clientSecret": secret_literal,
        "providerName": "audit-idp",
        "jitProvisioning": true,
    });

    let (status, res_body, _) = call(&router, "PUT", SSO, &cookie, &body.to_string()).await;
    assert_eq!(status, StatusCode::OK, "{res_body}");

    // Query audit_event rows.
    let events: Vec<(String, Option<Value>)> =
        sqlx::query_as("SELECT action, args FROM audit_event WHERE action LIKE 'settings.sso.%'")
            .fetch_all(&pool)
            .await
            .unwrap();

    assert!(
        !events.is_empty(),
        "expected an audit event for settings.sso.update"
    );
    for (action, args) in events {
        let args_str = args.map(|a| a.to_string()).unwrap_or_default();
        assert!(
            !args_str.contains(secret_literal),
            "audit event {action} leaked secret in args: {args_str}"
        );
    }
}
