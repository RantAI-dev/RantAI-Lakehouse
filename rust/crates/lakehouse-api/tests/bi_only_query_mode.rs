//! `BI-44` slice 1: Query Studio refuses the engine a `bi-only`
//! installation did not start, on both `/api/query/run` and
//! `/api/query/estimate`, before a parameter is filled, a policy is asked
//! or a dialect is chosen. The full stack's behaviour is covered by the
//! existing query tests.
//!
//! The harness always sets `CH_URL`, so a Trino-only test explicitly
//! overrides it with an empty value — "unset" in config terms. That is the
//! same shape the standalone Trino compose file produces.

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

/// A small read query is enough to reach the engine resolution; nothing is
/// ever sent because the request names the inactive engine.
const SQL: &str = "SELECT 1 AS n";

async fn run_query(
    app: &axum::Router,
    cookie: &str,
    route: &str,
    engine: &str,
) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(route)
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "sql": SQL, "engine": engine }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

/// A ClickHouse-only installation refuses an explicit Trino request on
/// both routes and names the one engine it actually runs.
#[tokio::test]
async fn a_clickhouse_only_installation_refuses_trino_on_run_and_estimate() {
    let env = HashMap::from([
        ("LAKEHOUSE_INSTALL_MODE".to_owned(), "bi-only".to_owned()),
        ("LAKEHOUSE_QUERY_ENGINE".to_owned(), "clickhouse".to_owned()),
    ]);
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let user = create_principal_with_permissions(&pool, "query:read").await;
    let cookie = session_cookie_for_user(&pool, user).await;
    for route in ["/api/query/run", "/api/query/estimate"] {
        let (status, body) = run_query(&router, &cookie, route, "trino").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{route}: {body}");
        let message = body["error"].as_str().unwrap_or_default();
        assert!(message.contains("clickhouse"), "{route}: {message}");
        assert!(!message.contains("trino"), "{route}: {message}");
    }
}

/// The mirror case: a Trino-only installation refuses an explicit
/// ClickHouse request. The message names Trino, never the engine that was
/// not started.
#[tokio::test]
async fn a_trino_only_installation_refuses_clickhouse_on_run_and_estimate() {
    let env = HashMap::from([
        ("LAKEHOUSE_INSTALL_MODE".to_owned(), "bi-only".to_owned()),
        ("LAKEHOUSE_QUERY_ENGINE".to_owned(), "trino".to_owned()),
        ("TRINO_URL".to_owned(), "http://127.0.0.1:1".to_owned()),
        ("CH_URL".to_owned(), String::new()),
    ]);
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let user = create_principal_with_permissions(&pool, "query:read").await;
    let cookie = session_cookie_for_user(&pool, user).await;
    for route in ["/api/query/run", "/api/query/estimate"] {
        let (status, body) = run_query(&router, &cookie, route, "clickhouse").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{route}: {body}");
        let message = body["error"].as_str().unwrap_or_default();
        assert!(message.contains("trino"), "{route}: {message}");
        assert!(!message.contains("clickhouse"), "{route}: {message}");
    }
}
