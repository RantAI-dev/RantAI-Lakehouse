//! `GET /api/connectors/{id}/probe-history` at the route level — same
//! live-pool harness `tests/test_connection_route.rs`/`tests/route_auth.rs`
//! already use (see that file's module doc comment for why this needs a
//! real seeded row and a real authenticated session, not the in-crate
//! `state_without_pool()` helper).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use common::{session_cookie_for_seeded_user, spin_up};

async fn get(app: &common::TestApp, cookie: &str, uri: &str) -> (StatusCode, Value) {
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
    let body: Value = serde_json::from_slice(&bytes).expect("valid JSON");
    (status, body)
}

/// An unknown connector id is a 404 -- never an empty `results: []` for a
/// connector that does not exist.
#[tokio::test]
async fn probe_history_route_404s_for_an_unknown_connector() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    let (status, _body) = get(
        &app,
        &cookie,
        "/api/connectors/conn-does-not-exist/probe-history",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// `limit=0` is refused outright, not silently clamped up to 1.
#[tokio::test]
async fn probe_history_route_400s_for_limit_zero() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    let (status, body) = get(
        &app,
        &cookie,
        "/api/connectors/conn-pg-lakehouse/probe-history?limit=0",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .expect("error field")
            .contains("limit"),
        "{body}"
    );
}

/// `limit=201` is refused outright, not silently clamped down to 200.
#[tokio::test]
async fn probe_history_route_400s_for_limit_above_two_hundred() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    let (status, body) = get(
        &app,
        &cookie,
        "/api/connectors/conn-pg-lakehouse/probe-history?limit=201",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .expect("error field")
            .contains("limit"),
        "{body}"
    );
}

/// A populated response is newest-first and camelCase
/// (`testedAt`/`latencyMs`, matching `contracts/connectors.ts`'s
/// `ConnectorProbeResult`), for a connector that actually has history.
#[tokio::test]
async fn probe_history_route_returns_camel_case_results_newest_first() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    // Seed two history rows directly -- exercising the route, not another
    // real dial attempt (`connector_probe_result.rs`'s own store tests
    // already cover `record_test_result`'s write path).
    sqlx::query(
        "INSERT INTO connector_probe_result (connector_id, tested_at, ok, latency_ms, message) \
         VALUES ('conn-pg-lakehouse', now() - interval '1 minute', false, 40, 'first probe'), \
         ('conn-pg-lakehouse', now(), true, 12, 'second probe')",
    )
    .execute(&app.pool)
    .await
    .expect("seed probe history rows");

    let (status, body) = get(
        &app,
        &cookie,
        "/api/connectors/conn-pg-lakehouse/probe-history",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let results = body["results"].as_array().expect("results array");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["message"], "second probe");
    assert_eq!(results[0]["ok"], true);
    assert_eq!(results[0]["latencyMs"], 12);
    assert!(results[0]["testedAt"].as_str().is_some());
    assert_eq!(results[1]["message"], "first probe");
    assert_eq!(results[1]["ok"], false);
    assert_eq!(results[1]["latencyMs"], 40);
}
