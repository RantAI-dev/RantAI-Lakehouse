//! HTTP-level integration tests for T0.5 of the copilot-operations-handover
//! plan: `WriteHigh` copilot calls create a pending approval, and
//! `POST /api/agents/approvals/{id}/decide` is the only path that ever
//! executes it — proven end to end against a real Postgres, exercising
//! `POST /api/ai/tool` (the gate side) and `POST /api/agents/approvals/{id}/decide`
//! (the execution side) together.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use lakehouse_store::audit::{AuditFilter, list};
use serde_json::{Value, json};
use tower::ServiceExt;

use common::{TestApp, session_cookie_for_seeded_user, spin_up};

async fn post(
    app: &axum::Router,
    uri: &str,
    cookie: Option<&str>,
    body: Value,
) -> axum::http::Response<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
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

/// Create a pending `delete_chart` approval as `fajar@meridian.example`
/// (Platform Admin — has both `dashboard:write`, the tool's own
/// permission, and `agent:approve`) and return its `approval_id`.
async fn create_pending_delete_chart_approval(
    router: &axum::Router,
    pool: &sqlx::PgPool,
) -> String {
    let cookie = session_cookie_for_seeded_user(pool, "fajar@meridian.example").await;
    let resp = post(
        router,
        "/api/ai/tool",
        Some(&cookie),
        json!({ "tool": "delete_chart", "args": { "id": "c-1", "confirmed": true }, "mode": "build" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    body["result"]["approval_id"]
        .as_str()
        .expect("approval_id present")
        .to_owned()
}

/// Approving a pending `WriteHigh` approval, by a principal who holds the
/// underlying tool's permission, executes it exactly once through the
/// real dispatch, flips the run to a terminal status, and audits both the
/// `approved` decision and the execution outcome.
#[tokio::test]
async fn approve_executes_the_tool_and_flips_run_status() {
    let TestApp { router, pool } = spin_up().await;
    let approval_id = create_pending_delete_chart_approval(&router, &pool).await;

    // Approve as Fajar (Platform Admin: `*:*`, holds both `agent:approve`
    // and the tool's own `dashboard:write`).
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let resp = post(
        &router,
        &format!("/api/agents/approvals/{approval_id}/decide"),
        Some(&cookie),
        json!({ "decision": "approved" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["executed"], json!(true));
    assert_eq!(body["approval"]["status"], json!("approved"));
    // `ClickHouse` is a dead upstream in this harness (see `common::spin_up`),
    // so the actual `delete_chart` call fails — but it DID run (that's the
    // point): the response carries a `result`, not the un-attempted shape.
    assert!(body.get("result").is_some());

    let run_id = body["approval"]["runId"]
        .as_str()
        .expect("runId present")
        .to_owned();
    let run = lakehouse_store::agents::get_run(&pool, &run_id)
        .await
        .expect("get run")
        .expect("run exists");
    assert_ne!(run.status, "waiting_approval");
    assert!(run.status == "succeeded" || run.status == "failed");
    assert_eq!(
        run.steps.len(),
        2,
        "the pending step plus the execution step"
    );

    let events = list(&pool, AuditFilter::default())
        .await
        .expect("list audit events");
    let approved: Vec<_> = events.iter().filter(|e| e.outcome == "approved").collect();
    assert_eq!(approved.len(), 1);
    assert_eq!(approved[0].run_id.as_deref(), Some(run_id.as_str()));
    assert_eq!(
        approved[0].approval_id.as_deref(),
        Some(approval_id.as_str())
    );
}

/// Rejecting a pending approval executes nothing: the run flips to
/// `rejected`, never `succeeded`/`failed`, and is audited `rejected`.
#[tokio::test]
async fn reject_executes_nothing_and_flips_run_to_rejected() {
    let TestApp { router, pool } = spin_up().await;
    let approval_id = create_pending_delete_chart_approval(&router, &pool).await;

    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let resp = post(
        &router,
        &format!("/api/agents/approvals/{approval_id}/decide"),
        Some(&cookie),
        json!({ "decision": "rejected", "comment": "tidak sesuai kebijakan" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["executed"], json!(false));
    assert_eq!(body["approval"]["status"], json!("rejected"));

    let run_id = body["approval"]["runId"]
        .as_str()
        .expect("runId present")
        .to_owned();
    let run = lakehouse_store::agents::get_run(&pool, &run_id)
        .await
        .expect("get run")
        .expect("run exists");
    assert_eq!(run.status, "rejected");
    // The pending step plus a "rejected" outcome step — but no
    // EXECUTION step, since nothing ran.
    assert_eq!(run.steps.len(), 2);
    assert_eq!(run.steps[1].status, "rejected");

    let events = list(&pool, AuditFilter::default())
        .await
        .expect("list audit events");
    let rejected: Vec<_> = events.iter().filter(|e| e.outcome == "rejected").collect();
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0].run_id.as_deref(), Some(run_id.as_str()));
}

/// Double-decide: approving an already-decided approval a second time is
/// a 409, and the tool is never executed twice (proven by the run only
/// ever carrying ONE execution step, not two).
#[tokio::test]
async fn double_decide_does_not_execute_twice() {
    let TestApp { router, pool } = spin_up().await;
    let approval_id = create_pending_delete_chart_approval(&router, &pool).await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let first = post(
        &router,
        &format!("/api/agents/approvals/{approval_id}/decide"),
        Some(&cookie),
        json!({ "decision": "approved" }),
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first_body = json_body(first).await;
    let run_id = first_body["approval"]["runId"]
        .as_str()
        .expect("runId present")
        .to_owned();

    // A fresh session for the second request — approving twice with the
    // SAME cookie is fine too, but a fresh one matches how a second
    // browser tab / a retried request would actually look.
    let cookie2 = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let second = post(
        &router,
        &format!("/api/agents/approvals/{approval_id}/decide"),
        Some(&cookie2),
        json!({ "decision": "approved" }),
    )
    .await;
    assert_eq!(second.status(), StatusCode::CONFLICT);

    let run = lakehouse_store::agents::get_run(&pool, &run_id)
        .await
        .expect("get run")
        .expect("run exists");
    assert_eq!(
        run.steps.len(),
        2,
        "exactly one execution step ever appended, no matter how many decide calls happen"
    );
}

/// Plan invariant 3 / the permission rule: an approver who holds
/// `agent:approve` but NOT the tool's own permission (`dashboard:write`
/// for `delete_chart`) can decide the approval, but the tool is never
/// executed — the run ends `failed`/"not executed", and the HTTP response
/// is an explicit error, not a silent 200.
#[tokio::test]
async fn approver_lacking_the_tools_own_permission_does_not_execute() {
    let TestApp { router, pool } = spin_up().await;
    let approval_id = create_pending_delete_chart_approval(&router, &pool).await;

    // rina@meridian.example is a seeded Approver: `agent:approve,
    // policy:review` — no `dashboard:*` at all (0002_seed_identity.sql).
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let resp = post(
        &router,
        &format!("/api/agents/approvals/{approval_id}/decide"),
        Some(&cookie),
        json!({ "decision": "approved" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // The approval itself WAS decided (approved) — `decide_approval`'s own
    // transaction already committed by the time the permission check
    // runs — but the underlying tool never ran.
    let approvals = lakehouse_store::agents::list_approvals(&pool, None)
        .await
        .expect("list approvals");
    let approval = approvals
        .iter()
        .find(|a| a.id == approval_id)
        .expect("approval exists");
    assert_eq!(approval.status, "approved");

    let run_id = approval.run_id.clone().expect("run_id present");
    let run = lakehouse_store::agents::get_run(&pool, &run_id)
        .await
        .expect("get run")
        .expect("run exists");
    assert_eq!(run.status, "failed");
    assert_eq!(run.steps.len(), 2);
    assert!(
        run.steps[1].detail.contains("dashboard:write"),
        "the not-executed step must name the missing permission: {}",
        run.steps[1].detail
    );

    let events = list(&pool, AuditFilter::default())
        .await
        .expect("list audit events");
    // One `approved` (the decision itself) and one `failed` (not
    // executed, permission denied) — never `executed`.
    let for_this_run: Vec<_> = events
        .iter()
        .filter(|e| e.run_id.as_deref() == Some(run_id.as_str()))
        .collect();
    assert!(for_this_run.iter().any(|e| e.outcome == "approved"));
    assert!(for_this_run.iter().any(|e| e.outcome == "failed"));
    assert!(!for_this_run.iter().any(|e| e.outcome == "executed"));
}

/// Evidence stored on the approval is redacted (plan invariant 5): a
/// secret-shaped argument never survives into `approval_item.evidence`.
#[tokio::test]
async fn approval_evidence_is_redacted() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let resp = post(
        &router,
        "/api/ai/tool",
        Some(&cookie),
        json!({
            "tool": "delete_chart",
            "args": { "id": "c-1", "confirmed": true, "apiKey": "sk-super-secret" },
            "mode": "build",
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    let approval_id = body["result"]["approval_id"].as_str().unwrap().to_owned();

    let approvals = lakehouse_store::agents::list_approvals(&pool, None)
        .await
        .expect("list approvals");
    let approval = approvals.iter().find(|a| a.id == approval_id).unwrap();
    let evidence = approval.evidence.as_ref().expect("evidence present");
    assert_eq!(evidence.len(), 1);
    assert!(
        !evidence[0].contains("sk-super-secret"),
        "evidence must never contain the raw secret: {}",
        evidence[0]
    );
    assert!(evidence[0].contains("[redacted]"));
}
