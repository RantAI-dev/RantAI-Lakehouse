//! ADM-11 T2 (D4, D6): password rules, rules route, login by either identifier,
//! and `verify_for_user`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_auth::Secret;
use lakehouse_auth::password::create_local_identity;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use common::{TestApp, session_cookie_for_user, spin_up};

const RINA_ID: &str = "33333333-3333-4333-8333-000000000001";
const RINA_EMAIL: &str = "rina@meridian.example";
const RINA_PASSWORD: &str = "correct-horse-battery-staple-2026";

async fn post_json(
    router: &axum::Router,
    cookie: Option<&str>,
    uri: &str,
    body: &Value,
) -> (StatusCode, Value, Option<String>) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(c) = cookie {
        builder = builder.header("cookie", c);
    }
    let response = router
        .clone()
        .oneshot(
            builder
                .body(Body::from(body.to_string()))
                .expect("build request"),
        )
        .await
        .expect("send request");
    let status = response.status();
    let set_cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let json_body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json_body, set_cookie)
}

async fn get_json(router: &axum::Router, cookie: Option<&str>, uri: &str) -> (StatusCode, Value) {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(c) = cookie {
        builder = builder.header("cookie", c);
    }
    let response = router
        .clone()
        .oneshot(builder.body(Body::empty()).expect("build request"))
        .await
        .expect("send request");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let json_body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json_body)
}

#[tokio::test]
async fn password_rules_route_returns_min_and_max_length() {
    let TestApp { router, .. } = spin_up().await;
    let (status, body) = get_json(&router, None, "/api/auth/password-rules").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["minLength"], 8);
    assert_eq!(body["maxLength"], 1024);
}

#[tokio::test]
async fn password_rules_is_reachable_while_must_change_password_is_set() {
    let TestApp { router, pool } = spin_up().await;
    let rina_uuid = Uuid::parse_str(RINA_ID).unwrap();
    create_local_identity(&pool, rina_uuid, &Secret::new(RINA_PASSWORD), true)
        .await
        .unwrap();

    let cookie = session_cookie_for_user(&pool, rina_uuid).await;
    let (status, body) = get_json(&router, Some(&cookie), "/api/auth/password-rules").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["minLength"], 8);
}

#[tokio::test]
async fn sign_in_by_username_and_by_email_in_capitals_and_legacy_body() {
    let TestApp { router, pool } = spin_up().await;
    let rina_uuid = Uuid::parse_str(RINA_ID).unwrap();

    // Distinct username from email
    sqlx::query("UPDATE app_user SET username = 'rina.wijaya' WHERE id = $1")
        .bind(rina_uuid)
        .execute(&pool)
        .await
        .unwrap();

    create_local_identity(&pool, rina_uuid, &Secret::new(RINA_PASSWORD), false)
        .await
        .unwrap();

    // 1. Sign in by username using { "identifier": "rina.wijaya", "password": ... }
    let (status1, body1, cookie1) = post_json(
        &router,
        None,
        "/api/auth/login",
        &json!({
            "identifier": "rina.wijaya",
            "password": RINA_PASSWORD
        }),
    )
    .await;
    assert_eq!(status1, StatusCode::OK, "login by username: {body1}");
    assert!(cookie1.is_some());

    // 2. Sign in by email in capitals
    let (status2, body2, cookie2) = post_json(
        &router,
        None,
        "/api/auth/login",
        &json!({
            "identifier": "RINA@MERIDIAN.EXAMPLE",
            "password": RINA_PASSWORD
        }),
    )
    .await;
    assert_eq!(status2, StatusCode::OK, "login by email caps: {body2}");
    assert!(cookie2.is_some());

    // 3. Old { "email": ..., "password": ... } body still works
    let (status3, body3, cookie3) = post_json(
        &router,
        None,
        "/api/auth/login",
        &json!({
            "email": RINA_EMAIL,
            "password": RINA_PASSWORD
        }),
    )
    .await;
    assert_eq!(
        status3,
        StatusCode::OK,
        "login by legacy email field: {body3}"
    );
    assert!(cookie3.is_some());
}

#[tokio::test]
async fn ambiguous_identifier_is_refused_with_invalid_credentials_error() {
    let TestApp { router, pool } = spin_up().await;
    let rina_uuid = Uuid::parse_str(RINA_ID).unwrap();

    // Update Rina's username so it differs from her email
    sqlx::query("UPDATE app_user SET username = 'rina.wijaya' WHERE id = $1")
        .bind(rina_uuid)
        .execute(&pool)
        .await
        .unwrap();

    create_local_identity(&pool, rina_uuid, &Secret::new(RINA_PASSWORD), false)
        .await
        .unwrap();

    // Create a second user whose username equals Rina's email
    let second_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO app_user (id, name, username, email) VALUES ($1, 'Impostor', $2, 'impostor@example.com')",
    )
    .bind(second_id)
    .bind(RINA_EMAIL)
    .execute(&pool)
    .await
    .unwrap();

    create_local_identity(&pool, second_id, &Secret::new(RINA_PASSWORD), false)
        .await
        .unwrap();

    let (status, body, _) = post_json(
        &router,
        None,
        "/api/auth/login",
        &json!({
            "identifier": RINA_EMAIL,
            "password": RINA_PASSWORD
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "Invalid username/email or password.");
}

#[tokio::test]
async fn password_change_enforces_password_rules() {
    let TestApp { router, pool } = spin_up().await;
    let rina_uuid = Uuid::parse_str(RINA_ID).unwrap();

    sqlx::query("UPDATE app_user SET username = 'rina.wijaya' WHERE id = $1")
        .bind(rina_uuid)
        .execute(&pool)
        .await
        .unwrap();

    create_local_identity(&pool, rina_uuid, &Secret::new(RINA_PASSWORD), false)
        .await
        .unwrap();

    let cookie = session_cookie_for_user(&pool, rina_uuid).await;

    // 1. 7-character change gives 400 naming the rule
    let (status1, body1, _) = post_json(
        &router,
        Some(&cookie),
        "/api/auth/change-password",
        &json!({
            "oldPassword": RINA_PASSWORD,
            "newPassword": "short-7"
        }),
    )
    .await;
    assert_eq!(status1, StatusCode::BAD_REQUEST);
    assert!(
        body1["error"]
            .as_str()
            .unwrap()
            .contains("at least 8 characters"),
        "error must name the rule: {body1}"
    );

    // 2. Password equal to username is refused
    let (status2, body2, _) = post_json(
        &router,
        Some(&cookie),
        "/api/auth/change-password",
        &json!({
            "oldPassword": RINA_PASSWORD,
            "newPassword": "RINA.WIJAYA"
        }),
    )
    .await;
    assert_eq!(status2, StatusCode::BAD_REQUEST);
    assert!(
        body2["error"]
            .as_str()
            .unwrap()
            .contains("equal to the username"),
        "error must name the rule: {body2}"
    );

    // 3. Password equal to email is refused
    let (status3, body3, _) = post_json(
        &router,
        Some(&cookie),
        "/api/auth/change-password",
        &json!({
            "oldPassword": RINA_PASSWORD,
            "newPassword": RINA_EMAIL
        }),
    )
    .await;
    assert_eq!(status3, StatusCode::BAD_REQUEST);
    assert!(
        body3["error"]
            .as_str()
            .unwrap()
            .contains("equal to the email address"),
        "error must name the rule: {body3}"
    );

    // 4. Valid new password succeeds
    let (status4, _, _) = post_json(
        &router,
        Some(&cookie),
        "/api/auth/change-password",
        &json!({
            "oldPassword": RINA_PASSWORD,
            "newPassword": "a-new-valid-password-2026"
        }),
    )
    .await;
    assert_eq!(status4, StatusCode::NO_CONTENT);
}
