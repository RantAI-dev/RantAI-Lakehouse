//! `GET /api/dashboard/frame-check?url=` over the real router (`BI-72`,
//! decision 3): who may ask, that only an address on the content sites is
//! answered, and that a site that cannot be reached is no claim (`null`).
//!
//! The header cases (`DENY`, `SAMEORIGIN`, `frame-ancestors`) are unit tests
//! in `src/routes/frame_check.rs`, against a local `wiremock` server: the
//! route itself dials `https` only, through the resolver that refuses a
//! loopback address, so a local server cannot stand in for a site here.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn call(
    app: &axum::Router,
    verb: &str,
    uri: &str,
    cookie: &str,
    body: Option<&str>,
) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(verb)
                .uri(uri)
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.unwrap_or_default().to_owned()))
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

/// An app whose `ClickHouse` lists the one zone the settings save checks.
async fn app() -> (TestApp, MockServer) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"meta":[{"name":"n","type":"UInt64"}],"data":[{"n":"1"}],"rows":1}"#,
        ))
        .mount(&server)
        .await;
    let mut env = HashMap::new();
    env.insert("CH_URL".to_owned(), server.uri());
    (spin_up_with_env(&env).await, server)
}

#[tokio::test]
async fn an_address_off_the_content_sites_is_a_400_and_a_missing_one_too() {
    let (TestApp { router, pool }, _server) = app().await;
    let reader = create_principal_with_permissions(&pool, "dashboard:read").await;
    let cookie = session_cookie_for_user(&pool, reader).await;
    for uri in [
        "/api/dashboard/frame-check",
        "/api/dashboard/frame-check?url=https://status.example/",
        "/api/dashboard/frame-check?url=http://status.example/",
        "/api/dashboard/frame-check?url=https://allowed.example@evil.example/",
    ] {
        let (status, body) = call(&router, "GET", uri, &cookie, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
        assert_eq!(
            body["error"], "url must be an address on an allowed content site.",
            "{uri}"
        );
    }
}

#[tokio::test]
async fn an_allowed_site_that_cannot_be_reached_is_no_claim() {
    let (TestApp { router, pool }, _server) = app().await;
    let writer = create_principal_with_permissions(&pool, "settings:write").await;
    let writer = session_cookie_for_user(&pool, writer).await;
    let (status, body) = call(
        &router,
        "PUT",
        "/api/settings/reporting",
        &writer,
        Some(r#"{"timeZone":"UTC","weekStart":"monday","contentSites":["https://frame-check.invalid"]}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let reader = create_principal_with_permissions(&pool, "dashboard:read").await;
    let reader = session_cookie_for_user(&pool, reader).await;
    // `.invalid` never resolves (RFC 6761), so no request leaves the test.
    let (status, body) = call(
        &router,
        "GET",
        "/api/dashboard/frame-check?url=https://frame-check.invalid/status",
        &reader,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "framable": null }));
}
