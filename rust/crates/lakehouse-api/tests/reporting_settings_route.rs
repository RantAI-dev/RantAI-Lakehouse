//! `GET`/`PUT /api/settings/reporting` over the real router (`BI-9`, T1):
//! who may read and write, what is refused with a fixed message, that the
//! engine's zone list decides, and that an upstream failure never puts its
//! own text in the response.
//!
//! `ClickHouse` is a `wiremock` server answering the one `system.time_zones`
//! statement; Postgres is the per-test database from `tests/common`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_principal_with_permissions, create_zero_permission_principal,
    session_cookie_for_user, spin_up, spin_up_with_env,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn call(app: &axum::Router, verb: &str, cookie: &str, body: &str) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(verb)
                .uri("/api/settings/reporting")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// An app whose `ClickHouse` says the engine lists `known` zones (1) or not (0).
async fn app_with_engine(zone_count: u32) -> (TestApp, MockServer) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!(
            r#"{{"meta":[{{"name":"n","type":"UInt64"}}],"data":[{{"n":"{zone_count}"}}],"rows":1}}"#
        )))
        .mount(&server)
        .await;
    let mut env = HashMap::new();
    env.insert("CH_URL".to_owned(), server.uri());
    (spin_up_with_env(&env).await, server)
}

#[tokio::test]
async fn with_nothing_saved_a_signed_in_reader_gets_the_defaults() {
    let TestApp { router, pool } = spin_up().await;
    let user = create_zero_permission_principal(&pool).await;
    let cookie = session_cookie_for_user(&pool, user).await;
    let (status, body) = call(&router, "GET", &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"timeZone": "Asia/Jakarta", "weekStart": "monday", "saved": false})
    );
}

#[tokio::test]
async fn a_dashboard_editor_without_settings_write_is_refused() {
    let TestApp { router, pool } = spin_up().await;
    let user = create_principal_with_permissions(&pool, "dashboard:write, dashboard:read").await;
    let cookie = session_cookie_for_user(&pool, user).await;
    let (status, _) = call(
        &router,
        "PUT",
        &cookie,
        r#"{"timeZone":"UTC","weekStart":"monday"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&router, "GET", &cookie, "").await;
    assert_eq!(status, StatusCode::OK, "reading needs only a sign-in");
}

#[tokio::test]
async fn a_bad_body_a_third_week_start_and_a_malformed_zone_are_400_with_a_fixed_message() {
    let TestApp { router, pool } = spin_up().await;
    let user = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, user).await;
    let cases = [
        (
            "not json",
            "body must be {\"timeZone\": string, \"weekStart\": \"monday\"|\"sunday\"}",
        ),
        (
            r#"{"timeZone":"UTC"}"#,
            "body must be {\"timeZone\": string, \"weekStart\": \"monday\"|\"sunday\"}",
        ),
        (
            r#"{"timeZone":"UTC","weekStart":"friday"}"#,
            "weekStart must be monday or sunday.",
        ),
        (
            r#"{"timeZone":"UTC' OR 1=1 --","weekStart":"monday"}"#,
            "Unknown time zone.",
        ),
    ];
    for (body, message) in cases {
        let (status, got) = call(&router, "PUT", &cookie, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(got["error"].as_str(), Some(message), "{body} -> {got}");
    }
}

#[tokio::test]
async fn a_zone_the_engine_does_not_list_is_refused_and_nothing_is_saved() {
    let (TestApp { router, pool }, _server) = app_with_engine(0).await;
    let user = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, user).await;
    let (status, body) = call(
        &router,
        "PUT",
        &cookie,
        r#"{"timeZone":"Mars/Olympus","weekStart":"sunday"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.to_string().contains("Unknown time zone."), "{body}");
    let (_, got) = call(&router, "GET", &cookie, "").await;
    assert_eq!(got["saved"], json!(false));
}

#[tokio::test]
async fn a_known_zone_is_saved_and_read_back_with_the_saved_flag() {
    let (TestApp { router, pool }, _server) = app_with_engine(1).await;
    let writer = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, writer).await;
    let (status, body) = call(
        &router,
        "PUT",
        &cookie,
        r#"{"timeZone":"Europe/Berlin","weekStart":"sunday"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({"timeZone": "Europe/Berlin", "weekStart": "sunday", "saved": true})
    );
    let reader = create_zero_permission_principal(&pool).await;
    let cookie = session_cookie_for_user(&pool, reader).await;
    let (status, got) = call(&router, "GET", &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got, body);
}

#[tokio::test]
async fn an_engine_failure_is_reported_without_its_own_text() {
    // The default harness points the engine at a refused connection.
    let TestApp { router, pool } = spin_up().await;
    let user = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, user).await;
    let (status, body) = call(
        &router,
        "PUT",
        &cookie,
        r#"{"timeZone":"UTC","weekStart":"monday"}"#,
    )
    .await;
    assert!(status.is_server_error(), "{status}");
    let text = body.to_string();
    assert!(!text.contains("127.0.0.1"), "{text}");
    assert!(text.contains("Reference:"), "{text}");
}
