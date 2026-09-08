//! HTTP-level integration tests for `POST /api/ai/tool` (T0.4 of the
//! copilot-operations-handover plan) and its audit trail (T0.3's leftover
//! half): the SAME `gate::decide` path the chat loop runs, exercised
//! through the real router with a real Postgres-backed `audit_event`
//! table, so these prove "one audit row per call, with the right
//! outcome" end to end rather than only at the unit level (see
//! `crates/lakehouse-api/src/routes/ai/gate.rs` and `.../audit.rs` for
//! those).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use lakehouse_store::audit::{AuditFilter, list};
use serde_json::{Value, json};
use tower::ServiceExt;

use common::{TestApp, session_cookie_for_seeded_user, spin_up};

async fn post_tool(
    app: &axum::Router,
    cookie: Option<&str>,
    body: Value,
) -> axum::http::Response<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/api/ai/tool")
        .header("content-type", "application/json");
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    app.clone()
        .oneshot(
            builder
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

/// Ask mode never executes a non-`Read` tool, even through
/// `POST /api/ai/tool` — the same D3 invariant the chat loop enforces, now
/// proven at the HTTP layer too. Exactly one `audit_event` row is written,
/// `outcome = "refused"`.
#[tokio::test]
async fn ask_mode_refuses_write_low_tool_and_audits_it() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({ "tool": "create_board", "args": { "name": "Wisatawan" }, "mode": "ask" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["outcome"], json!("refused"));
    assert_eq!(body["result"]["refused"], json!(true));
    assert!(
        body["result"].get("reason").is_none(),
        "ask-mode refusal carries no permission reason"
    );

    let events = list(&pool, AuditFilter::default())
        .await
        .expect("list audit events");
    let ours: Vec<_> = events
        .iter()
        .filter(|e| e.action == "create_board")
        .collect();
    assert_eq!(ours.len(), 1, "exactly one audit row for this call");
    assert_eq!(ours[0].outcome, "refused");
    assert_eq!(ours[0].principal_kind.as_deref(), Some("copilot"));
}

/// A principal lacking the tool's permission is refused via
/// `POST /api/ai/tool`, with the same `reason: "permission"` shape the
/// chat loop's gate produces, and exactly one audit row.
#[tokio::test]
async fn missing_permission_refuses_and_audits_with_permission_reason() {
    let TestApp { router, pool } = spin_up().await;
    let user_id = common::create_zero_permission_principal(&pool).await;
    let cookie = common::session_cookie_for_user(&pool, user_id).await;

    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({ "tool": "create_chart", "args": { "title": "T", "kind": "bar", "confirmed": true }, "mode": "build" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["outcome"], json!("refused"));
    assert_eq!(body["result"]["reason"], json!("permission"));
    assert_eq!(body["result"]["required"], json!("dashboard:write"));

    let events = list(&pool, AuditFilter::default())
        .await
        .expect("list audit events");
    let ours: Vec<_> = events
        .iter()
        .filter(|e| e.action == "create_chart")
        .collect();
    assert_eq!(ours.len(), 1);
    assert_eq!(ours[0].outcome, "refused");
}

/// A `WriteLow` tool called in build mode WITHOUT `confirmed: true`
/// returns `needs_confirmation` and executes nothing — proven here by the
/// audited outcome, since the real `create_board` execution would hit the
/// (deliberately dead) `ClickHouse` upstream and come back `failed`
/// instead, which this test asserts it never reaches.
#[tokio::test]
async fn write_low_tool_without_confirmed_needs_confirmation_and_never_executes() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({ "tool": "create_board", "args": { "name": "Wisatawan" }, "mode": "build" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["outcome"], json!("needs_confirmation"));
    assert_eq!(body["result"]["needs_confirmation"], json!(true));
    assert_eq!(body["result"]["tool"], json!("create_board"));
    assert!(
        body["result"]["summary"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "must carry a non-empty human-readable summary"
    );

    let events = list(&pool, AuditFilter::default())
        .await
        .expect("list audit events");
    let ours: Vec<_> = events
        .iter()
        .filter(|e| e.action == "create_board")
        .collect();
    assert_eq!(ours.len(), 1);
    assert_eq!(ours[0].outcome, "needs_confirmation");
}

/// The SAME call, resent with `confirmed: true`, passes the gate — it
/// reaches `run_tool` (proven by the outcome no longer being `refused` or
/// `needs_confirmation`; it becomes `failed` here only because the test
/// harness points `ClickHouse` at a dead upstream, not because the gate
/// blocked it) — and is still audited exactly once, distinctly from the
/// unconfirmed attempt above.
#[tokio::test]
async fn write_low_tool_confirmed_passes_the_gate_and_is_audited_as_executed_or_failed() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({ "tool": "create_board", "args": { "name": "Wisatawan", "confirmed": true }, "mode": "build" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_ne!(body["outcome"], json!("refused"));
    assert_ne!(body["outcome"], json!("needs_confirmation"));

    let events = list(&pool, AuditFilter::default())
        .await
        .expect("list audit events");
    let ours: Vec<_> = events
        .iter()
        .filter(|e| e.action == "create_board")
        .collect();
    assert_eq!(
        ours.len(),
        1,
        "exactly one audit row for the confirmed call"
    );
    assert_ne!(ours[0].outcome, "refused");
    assert_ne!(ours[0].outcome, "needs_confirmation");
}

/// `WriteHigh` (`delete_chart`) is NEVER executed by `POST /api/ai/tool`,
/// even with `confirmed: true` (that flag only means something for
/// `WriteLow`) — instead it creates a pending `agent_run` +
/// `approval_item` pair (T0.5) and is audited `needs_approval`, linked by
/// `run_id`/`approval_id`.
#[tokio::test]
async fn write_high_tool_creates_a_pending_approval_and_never_executes() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({ "tool": "delete_chart", "args": { "id": "c-1", "confirmed": true }, "mode": "build" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["outcome"], json!("needs_approval"));
    assert_eq!(body["result"]["needs_approval"], json!(true));
    assert_eq!(body["result"]["tool"], json!("delete_chart"));
    let approval_id = body["result"]["approval_id"]
        .as_str()
        .expect("approval_id present")
        .to_owned();
    let run_id = body["result"]["run_id"]
        .as_str()
        .expect("run_id present")
        .to_owned();

    // Exactly one approval + one run, and NOTHING executed (the chart
    // still doesn't exist, but more importantly there is no way to tell
    // from this response alone — the run's own status proves it).
    let approvals = lakehouse_store::agents::list_approvals(&pool, None)
        .await
        .expect("list approvals");
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].id, approval_id);
    assert_eq!(approvals[0].status, "pending");
    assert_eq!(approvals[0].action, "delete_chart");

    let run = lakehouse_store::agents::get_run(&pool, &run_id)
        .await
        .expect("get run")
        .expect("run exists");
    assert_eq!(run.status, "waiting_approval");
    assert_eq!(run.employee_id, "emp-copilot");

    let events = list(&pool, AuditFilter::default())
        .await
        .expect("list audit events");
    let ours: Vec<_> = events
        .iter()
        .filter(|e| e.action == "delete_chart")
        .collect();
    assert_eq!(ours.len(), 1);
    assert_eq!(ours[0].outcome, "needs_approval");
    assert_eq!(ours[0].run_id.as_deref(), Some(run_id.as_str()));
    assert_eq!(ours[0].approval_id.as_deref(), Some(approval_id.as_str()));
}

/// Plan invariant 5: a secret-shaped argument is never stored in
/// `audit_event.args`, even though it is happily accepted (and refused for
/// an unrelated reason) at the HTTP layer.
#[tokio::test]
async fn secret_shaped_args_are_redacted_before_they_reach_the_audit_row() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({
            "tool": "create_board",
            "args": { "name": "Wisatawan", "apiKey": "sk-super-secret" },
            "mode": "build",
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let events = list(&pool, AuditFilter::default())
        .await
        .expect("list audit events");
    let ours = events
        .iter()
        .find(|e| e.action == "create_board")
        .expect("an audit row for this call");
    assert_eq!(ours.args["apiKey"], json!("[redacted]"));
    assert_eq!(ours.args["name"], json!("Wisatawan"));
}

/// An unregistered tool name is a plain 400, not a 500 or a silent pass —
/// and it is never audited (there is no `ToolSpec` to attribute the
/// action to).
#[tokio::test]
async fn unknown_tool_name_is_a_400_and_is_not_audited() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = post_tool(
        &router,
        Some(&cookie),
        json!({ "tool": "not_a_real_tool", "args": {}, "mode": "build" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let events = list(&pool, AuditFilter::default())
        .await
        .expect("list audit events");
    assert!(events.iter().all(|e| e.action != "not_a_real_tool"));
}

/// `POST /api/ai/tool` is `Policy::RequiresAuth` — an unauthenticated
/// request is refused before the handler (and the gate) ever runs.
#[tokio::test]
async fn unauthenticated_request_is_refused_by_the_policy_layer() {
    let TestApp { router, .. } = spin_up().await;
    let resp = post_tool(
        &router,
        None,
        json!({ "tool": "create_board", "args": {}, "mode": "build" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
