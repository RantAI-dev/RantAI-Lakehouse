//! HTTP-level integration tests for the T2.5 governance draft tools
//! (`draft_policy`, `draft_classification_rule`, `draft_quality_rule`):
//! every one of them must create a record that is never "active" —
//! `draft_policy` always `status = "draft"` even when the model asks for
//! `activate: true`; `draft_classification_rule`/`draft_quality_rule` have
//! no activation concept at all, so this proves they land in the same
//! unevaluated state a human authoring the same rule through the console
//! would get (`review_status = "needs-review"` / `last_status = "warning"`).

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

/// Calls `POST /api/ai/tool` for `tool`/`args` (with `confirmed: true`
/// merged in, since every draft tool is `WriteLow`) as Fajar (Platform
/// Admin: `*:*`) in build mode, and asserts it actually executed.
async fn run_write_low_tool(
    router: &axum::Router,
    pool: &sqlx::PgPool,
    tool: &str,
    mut args: Value,
) -> Value {
    let cookie = session_cookie_for_seeded_user(pool, "fajar@meridian.example").await;
    args["confirmed"] = json!(true);
    let resp = post(
        router,
        "/api/ai/tool",
        &cookie,
        json!({ "tool": tool, "args": args, "mode": "build" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK, "{tool}");
    let body = json_body(resp).await;
    assert_eq!(body["outcome"], "executed", "{tool}: {body}");
    body["result"].clone()
}

/// `draft_policy` forces `status = "draft"` in the stored row even though
/// the call asks for `activate: true` — the tool must never let a copilot
/// caller create an already-`"ready"` policy.
#[tokio::test]
async fn draft_policy_always_creates_a_draft_status_row() {
    let TestApp { router, pool } = spin_up().await;
    let result = run_write_low_tool(
        &router,
        &pool,
        "draft_policy",
        json!({
            "name": "copilot_drafted_row_filter",
            "kind": "Row filter",
            "subjects": "All analysts",
            "resources": "tenant-scoped tables",
            "effect": "Permit with obligation",
            "activate": true,
        }),
    )
    .await;
    assert_eq!(result["status"], "draft", "{result}");

    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM policy WHERE name = 'copilot_drafted_row_filter'")
            .fetch_one(&pool)
            .await
            .expect("row must exist");
    assert_eq!(status, "draft");
}

/// `draft_classification_rule` always lands `review_status = "needs-review"`
/// — the only value `create_classification_rule` ever inserts.
#[tokio::test]
async fn draft_classification_rule_is_never_reviewed_or_auto() {
    let TestApp { router, pool } = spin_up().await;
    let result = run_write_low_tool(
        &router,
        &pool,
        "draft_classification_rule",
        json!({ "asset": "core.customer.copilot_test", "classification": "confidential" }),
    )
    .await;
    assert_eq!(result["reviewStatus"], "needs-review", "{result}");

    let (review_status,): (String,) = sqlx::query_as(
        "SELECT review_status FROM classification_rule WHERE asset = 'core.customer.copilot_test'",
    )
    .fetch_one(&pool)
    .await
    .expect("row must exist");
    assert_eq!(review_status, "needs-review");
}

/// `draft_quality_rule` always lands `last_status = "warning"` — the only
/// value `create_quality_rule` ever inserts, matching "authored, not yet
/// contradicted by evidence" rather than a real passed/failed verdict.
#[tokio::test]
async fn draft_quality_rule_is_never_passed_or_failed() {
    let TestApp { router, pool } = spin_up().await;
    let result = run_write_low_tool(
        &router,
        &pool,
        "draft_quality_rule",
        json!({
            "name": "copilot_drafted_quality_rule",
            "asset": "serving.mart_wisman",
            "dimension": "completeness",
            "threshold": ">= 95%",
            "severity": "medium",
        }),
    )
    .await;
    assert_eq!(result["lastStatus"], "warning", "{result}");

    let (last_status,): (String,) = sqlx::query_as(
        "SELECT last_status FROM quality_rule WHERE name = 'copilot_drafted_quality_rule'",
    )
    .fetch_one(&pool)
    .await
    .expect("row must exist");
    assert_eq!(last_status, "warning");
}
