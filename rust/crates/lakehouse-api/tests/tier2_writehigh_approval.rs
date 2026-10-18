//! HTTP-level integration tests for the Tier 2 `WriteHigh` tools:
//! `run_bronze_maintenance` (C2: applies real Bronze maintenance) and
//! `kill_query` (a real `KILL QUERY`). Each must create a pending
//! `approval_item` (via `POST /api/ai/tool`) instead of executing —
//! same style as `tests/tier1_writehigh_approval.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use common::{TestApp, session_cookie_for_seeded_user, spin_up};

async fn post(
    app: &axum::Router,
    uri: &str,
    cookie: &str,
    body: Value,
) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(
                    serde_json::to_vec(&body).expect("serialize body"),
                ))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright")
}

async fn json_body(resp: axum::http::Response<Body>) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read response body");
    serde_json::from_slice(&bytes).expect("valid JSON body")
}

/// Calls `POST /api/ai/tool` for `tool`/`args` as Fajar (Platform Admin:
/// `*:*`, so every tool's own permission is held) in build mode, and
/// asserts it produced a pending approval rather than executing.
async fn assert_creates_pending_approval(
    router: &axum::Router,
    pool: &sqlx::PgPool,
    tool: &str,
    args: Value,
) -> Value {
    let cookie = session_cookie_for_seeded_user(pool, "fajar@meridian.example").await;
    let resp = post(
        router,
        "/api/ai/tool",
        &cookie,
        json!({ "tool": tool, "args": args, "mode": "build" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK, "{tool}");
    let body = json_body(resp).await;
    assert_eq!(
        body["outcome"], "needs_approval",
        "{tool} must create a pending approval, not execute: {body}"
    );
    assert_eq!(body["result"]["needs_approval"], json!(true), "{tool}");
    assert!(
        body["result"]["approval_id"].as_str().is_some(),
        "{tool} must return an approval_id: {body}"
    );
    body
}

/// `run_bronze_maintenance` never launches the Dagster job on its own —
/// it must go through the approvals inbox first (C2: it genuinely deletes
/// orphan Iceberg files, so it is `WriteHigh`).
#[tokio::test]
async fn run_bronze_maintenance_creates_a_pending_approval() {
    let TestApp { router, pool } = spin_up().await;
    assert_creates_pending_approval(&router, &pool, "run_bronze_maintenance", json!({})).await;
}

/// `kill_query` never runs `KILL QUERY` on its own — same approvals-inbox
/// requirement.
#[tokio::test]
async fn kill_query_creates_a_pending_approval() {
    let TestApp { router, pool } = spin_up().await;
    assert_creates_pending_approval(&router, &pool, "kill_query", json!({ "id": "w-0" })).await;
}
