//! HTTP-level integration tests for the Tier 1 (T1.1-T1.3) operations
//! tools' `WriteHigh` members: `delete_connector`, `delete_alert_rule`,
//! `pause_pipeline`, `cancel_pipeline_run`. Each must create a pending
//! `approval_item` (via `POST /api/ai/tool`) instead of executing —
//! exactly like `delete_chart` in `tests/agents_approval.rs`, which this
//! file follows the style of.
//!
//! These deliberately do NOT re-prove the full approve/execute round trip
//! (`agents_approval.rs` already proves that generically, for any
//! `WriteHigh` tool, via `delete_chart`) — they prove the ONE thing that
//! is specific to each of these four tools: that they are wired up as
//! `WriteHigh` at all, so a copilot call never deletes a connector/alert
//! rule or pauses/cancels a pipeline without a human approval in between.

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
/// asserts it produced a pending approval rather than executing —
/// returning the response body for any further, tool-specific assertion.
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

#[tokio::test]
async fn delete_connector_creates_a_pending_approval_and_does_not_delete() {
    let TestApp { router, pool } = spin_up().await;
    // A real connector row, so "still present after the call" is a
    // meaningful assertion rather than vacuously true.
    sqlx::query(
        "INSERT INTO connector (id, name, type, direction, health, environment, tenant, host, \
         secret_ref, residency, capabilities, owner) VALUES \
         ('conn-t1', 'Test Connector', 'PostgreSQL', 'source', 'healthy', 'production', \
         'Meridian', 'db:5432', 'env:PW', '', '{}', 'ops@meridian.example')",
    )
    .execute(&pool)
    .await
    .expect("seed a connector row");

    assert_creates_pending_approval(
        &router,
        &pool,
        "delete_connector",
        json!({ "id": "conn-t1" }),
    )
    .await;

    let (still_there,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM connector WHERE id = 'conn-t1'")
            .fetch_one(&pool)
            .await
            .expect("count connector rows");
    assert_eq!(
        still_there, 1,
        "delete_connector must not have executed yet"
    );
}

#[tokio::test]
async fn delete_alert_rule_creates_a_pending_approval() {
    let TestApp { router, pool } = spin_up().await;
    assert_creates_pending_approval(
        &router,
        &pool,
        "delete_alert_rule",
        json!({ "id": "al-t1" }),
    )
    .await;
}

#[tokio::test]
async fn pause_pipeline_creates_a_pending_approval() {
    let TestApp { router, pool } = spin_up().await;
    assert_creates_pending_approval(&router, &pool, "pause_pipeline", json!({ "id": "pl-t1" }))
        .await;
}

#[tokio::test]
async fn cancel_pipeline_run_creates_a_pending_approval() {
    let TestApp { router, pool } = spin_up().await;
    assert_creates_pending_approval(
        &router,
        &pool,
        "cancel_pipeline_run",
        json!({ "runId": "run-t1" }),
    )
    .await;
}
