//! SEC-11: no response body carries text that came from an upstream.
//!
//! A `wiremock` stand-in for `ClickHouse` answers every request with a 500
//! whose body holds a planted marker (what the real server puts there: a
//! table name and a version). The routes below are the ones the audit named:
//! the public and embed dashboard links read by someone with no sign-in, and
//! the signed-in dashboard. Each must answer without the marker and with a
//! reference id the operator can look up in the log.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};
use serde_json::Value;
use tower::ServiceExt;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const MARKER: &str = "planted-marker-table-x";

/// A `ClickHouse` that fails every request the way the real one does.
async fn failing_clickhouse() -> MockServer {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string(format!(
            "Code: 60. DB::Exception: Table {MARKER} does not exist. (UNKNOWN_TABLE) (version 0.0.0)"
        )))
        .mount(&ch)
        .await;
    ch
}

async fn app_over(ch: &MockServer) -> TestApp {
    let mut overrides = HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch.uri());
    spin_up_with_env(&overrides).await
}

async fn send(router: &axum::Router, request: Request<Body>) -> (StatusCode, String) {
    let resp = router
        .clone()
        .oneshot(request)
        .await
        .expect("router never fails a request outright");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read the body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn the_public_dashboard_link_never_carries_the_database_text() {
    let ch = failing_clickhouse().await;
    let TestApp { router, .. } = app_over(&ch).await;

    let (status, body) = send(
        &router,
        Request::builder()
            .uri("/api/public/dashboard/some-token")
            .body(Body::empty())
            .expect("build request"),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!body.contains(MARKER), "the database text leaked: {body}");
    assert!(!body.contains("version 0.0.0"), "{body}");
    let json: Value = serde_json::from_str(&body).expect("a JSON body");
    assert_eq!(json["error"], "The database request failed.");
    assert!(json["errorId"].is_string(), "{body}");
}

#[tokio::test]
async fn the_embed_data_route_never_carries_the_database_text() {
    let ch = failing_clickhouse().await;
    let TestApp { router, .. } = app_over(&ch).await;

    let (status, body) = send(
        &router,
        Request::builder()
            .method("POST")
            .uri("/api/embed/data")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"jwt":"not-a-real-token"}"#))
            .expect("build request"),
    )
    .await;

    assert!(!body.contains(MARKER), "the database text leaked: {body}");
    assert!(!body.contains("version 0.0.0"), "{body}");
    // Whichever step answered, no database text is in it. SEC-12: the secret
    // is no longer looked up in the database, so with no `EMBED_SECRET` set
    // here the route answers 503 "embedding is not configured" before any
    // query; 500 (a board lookup failing) and 401 (a refused token) remain
    // the other possible answers.
    assert!(
        status == StatusCode::INTERNAL_SERVER_ERROR
            || status == StatusCode::UNAUTHORIZED
            || status == StatusCode::SERVICE_UNAVAILABLE,
        "{status}: {body}"
    );
}

#[tokio::test]
async fn the_signed_in_dashboard_never_carries_the_database_text() {
    let ch = failing_clickhouse().await;
    let TestApp { router, pool } = app_over(&ch).await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    for uri in [
        "/api/dashboard",
        "/api/dashboard/specs",
        "/api/dashboard/boards",
    ] {
        let (_status, body) = send(
            &router,
            Request::builder()
                .uri(uri)
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await;
        assert!(
            !body.contains(MARKER),
            "{uri} leaked the database text: {body}"
        );
        assert!(!body.contains("version 0.0.0"), "{uri}: {body}");
    }
}

#[tokio::test]
async fn a_malformed_dashboard_filter_keeps_our_own_message() {
    let ch = failing_clickhouse().await;
    let TestApp { router, pool } = app_over(&ch).await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let (status, body) = send(
        &router,
        Request::builder()
            .method("POST")
            .uri("/api/dashboard/specs")
            .header("cookie", &cookie)
            .header("content-type", "application/json")
            .body(Body::from("not json"))
            .expect("build request"),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("body JSON is invalid"), "{body}");
}

/// Every route below reads `ClickHouse`; none may repeat what it said.
/// A sweep rather than one test per route: the guard (`tests/sec11_guard.rs`)
/// is what stops a new leak, this proves the ones the audit found are gone.
#[tokio::test]
async fn no_clickhouse_route_repeats_the_database_text() {
    let ch = failing_clickhouse().await;
    let TestApp { router, pool } = app_over(&ch).await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let gets = [
        "/api/alerts",
        "/api/overview",
        "/api/overview/alerts",
        "/api/storage",
        "/api/storage/policies",
        "/api/catalog",
        "/api/catalog/silver.some_table/profile",
        "/api/catalog/silver.some_table/sample",
        "/api/governance/quality",
        "/api/governance/audit",
        "/api/governance/ingest-runs",
        "/api/lakehouse/capacity",
        "/api/dashboard/fields",
        "/api/dashboard/fields?mart=some_mart",
        "/api/gold/export/history?mart=some_mart",
        "/api/ai/sessions",
    ];
    let mut seen_reference = false;
    for uri in gets {
        let (_status, body) = send(
            &router,
            Request::builder()
                .uri(uri)
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await;
        assert!(
            !body.contains(MARKER),
            "{uri} leaked the database text: {body}"
        );
        assert!(!body.contains("version 0.0.0"), "{uri}: {body}");
        seen_reference |= body.contains("errorId") || body.contains("Reference: ");
    }
    assert!(
        seen_reference,
        "no route reported a failure with a reference id; the mock was not reached"
    );

    let (status, body) = send(
        &router,
        Request::builder()
            .method("POST")
            .uri("/api/query/run")
            .header("cookie", &cookie)
            .header("content-type", "application/json")
            .body(Body::from(r#"{"sql":"SELECT x FROM serving.nothing"}"#))
            .expect("build request"),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    // SEC-11 query-author exception (product owner, 2026-10-10): the editor
    // is the one route that shows its author the engine's diagnosis (the
    // table the statement named included), minus the version suffix.
    // `tests/query_author_exception.rs` pins this in full.
    assert!(
        body.contains(MARKER),
        "the author lost the diagnosis: {body}"
    );
    assert!(!body.contains("version 0.0.0"), "{body}");
    assert!(body.contains("Reference: "), "{body}");
}

/// The orchestrator and `ClickHouse` are unreachable (`127.0.0.1:1`). What
/// the HTTP client says about that names the address; no body may carry it.
#[tokio::test]
async fn an_unreachable_orchestrator_does_not_name_its_address() {
    let TestApp { router, pool } = common::spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    for uri in [
        "/api/pipelines",
        "/api/pipelines/runs",
        "/api/overview",
        "/api/governance/audit",
        "/api/storage",
    ] {
        let (_status, body) = send(
            &router,
            Request::builder()
                .uri(uri)
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await;
        assert!(
            !body.contains("127.0.0.1"),
            "{uri} named the upstream address: {body}"
        );
    }
}
