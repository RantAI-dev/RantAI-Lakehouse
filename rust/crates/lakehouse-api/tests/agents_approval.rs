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

// ── WS7 item E3: a dedicated, access:approve-gated decide route, distinct
//    from agent:approve's tool-call decide route ──────────────────────────
//
// NOT RUN as part of this task's own verification (see the implementer's
// report): this file is a pre-existing, real-Postgres-backed integration
// binary (`cargo test --test agents_approval`), and the WS7 task brief
// explicitly restricts verification to `cargo test -p lakehouse-api --lib`
// — this suite is deferred to the WS7 acceptance gate a judge runs, same as
// `--test route_auth`. Written here, failing-test-first against the
// pre-E3 router (`POST /api/catalog/access-requests/{id}/decide` doesn't
// exist yet — `router()` 404s any unmounted path), but not executed.

/// `POST /api/catalog/{id}/access-request` as `requester_email`, returning
/// the new `approval_id`.
async fn create_pending_access_request(
    router: &axum::Router,
    pool: &sqlx::PgPool,
    requester_email: &str,
    catalog_id: &str,
    permission: &str,
) -> String {
    let cookie = session_cookie_for_seeded_user(pool, requester_email).await;
    let resp = post(
        router,
        &format!("/api/catalog/{catalog_id}/access-request"),
        Some(&cookie),
        json!({ "permission": permission, "reason": "need it for a real task" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    body["approvalId"]
        .as_str()
        .expect("approvalId present")
        .to_owned()
}

/// A `kind = "tool_call"` approval id (via the existing `delete_chart`
/// helper) is refused by the ACCESS decide route with 404, never decided.
#[tokio::test]
async fn access_decide_route_refuses_a_tool_call_kind_approval_with_404() {
    let TestApp { router, pool } = spin_up().await;
    let approval_id = create_pending_delete_chart_approval(&router, &pool).await;

    // dewi@meridian.example: Governance Admin, holds access:approve.
    let cookie = session_cookie_for_seeded_user(&pool, "dewi@meridian.example").await;
    let resp = post(
        &router,
        &format!("/api/catalog/access-requests/{approval_id}/decide"),
        Some(&cookie),
        json!({ "decision": "approved" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// A `kind = "access"` approval id is refused by the AGENT decide route
/// with 404, never decided.
#[tokio::test]
async fn agent_decide_route_refuses_an_access_kind_approval_with_404() {
    let TestApp { router, pool } = spin_up().await;
    // sari@meridian.example: Analyst ONLY (0002_seed_identity.sql:84), so it
    // holds catalog:read but NOT catalog:write. andi@meridian.example cannot be
    // used here: the seed grants andi BOTH Data Engineer and Analyst (:82-83),
    // and Data Engineer already holds catalog:write (:43), so the route
    // correctly refuses the request as one the principal already satisfies.
    let approval_id = create_pending_access_request(
        &router,
        &pool,
        "sari@meridian.example",
        "cat-1",
        "catalog:write",
    )
    .await;

    // fajar@meridian.example: Platform Admin, holds agent:approve (via *:*).
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let resp = post(
        &router,
        &format!("/api/agents/approvals/{approval_id}/decide"),
        Some(&cookie),
        json!({ "decision": "approved" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// A principal lacking `access:approve` (Analyst) is 401/403'd deciding an
/// access request — `route_auth.rs`'s own table-driven loop already proves
/// this generically for every `POLICY_TABLE` row; this is a targeted,
/// named regression for the row WS7 item E3 adds.
#[tokio::test]
async fn access_decide_route_401s_or_403s_a_principal_lacking_access_approve() {
    let TestApp { router, pool } = spin_up().await;
    let approval_id = create_pending_access_request(
        &router,
        &pool,
        "sari@meridian.example",
        "cat-1",
        "catalog:write",
    )
    .await;

    // sari@meridian.example: Analyst only, no access:approve.
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;
    let resp = post(
        &router,
        &format!("/api/catalog/access-requests/{approval_id}/decide"),
        Some(&cookie),
        json!({ "decision": "approved" }),
    )
    .await;
    assert!(matches!(
        resp.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ));
}

/// The named acceptance criterion: a principal who both requested AND
/// could otherwise decide (holds `access:approve`) is refused deciding
/// their OWN request with 403.
#[tokio::test]
async fn access_decide_route_refuses_a_principal_approving_their_own_access_request() {
    let TestApp { router, pool } = spin_up().await;
    // dewi@meridian.example: Governance Admin, which `0040` grants
    // `access:approve` (so she could otherwise decide) and which does NOT
    // hold `catalog:write` (so she can legitimately request it).
    // fajar@meridian.example cannot be used here: Platform Admin is `*:*`,
    // which matches every permission, so `access_request` correctly refuses
    // the request itself as one the principal already satisfies — a
    // wildcard holder can never file an access request at all.
    let approval_id = create_pending_access_request(
        &router,
        &pool,
        "dewi@meridian.example",
        "cat-1",
        "catalog:write",
    )
    .await;

    let cookie = session_cookie_for_seeded_user(&pool, "dewi@meridian.example").await;
    let resp = post(
        &router,
        &format!("/api/catalog/access-requests/{approval_id}/decide"),
        Some(&cookie),
        json!({ "decision": "approved" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

/// The before/after proof (WS7 plan Phase E acceptance): a DIFFERENT
/// access:approve holder may decide, and approving actually widens what
/// the requester can do.
#[tokio::test]
async fn access_decide_route_allows_a_different_access_approve_holder_and_the_grant_widens_access()
{
    let TestApp { router, pool } = spin_up().await;
    let approval_id = create_pending_access_request(
        &router,
        &pool,
        "sari@meridian.example",
        "cat-1",
        "catalog:write",
    )
    .await;

    // BEFORE: andi does not hold catalog:write.
    let (requester_id,): (uuid::Uuid,) = sqlx::query_as("SELECT id FROM app_user WHERE email = $1")
        .bind("sari@meridian.example")
        .fetch_one(&pool)
        .await
        .expect("seeded user sari@meridian.example must exist");
    let before = lakehouse_auth::repository::load_principal_for_user(
        &pool,
        requester_id,
        "local".to_owned(),
        false,
    )
    .await
    .expect("load principal before grant");
    assert!(!before.has("catalog:write"));

    // dewi@meridian.example: Governance Admin, holds access:approve, is
    // NOT the requester.
    let cookie = session_cookie_for_seeded_user(&pool, "dewi@meridian.example").await;
    let resp = post(
        &router,
        &format!("/api/catalog/access-requests/{approval_id}/decide"),
        Some(&cookie),
        json!({ "decision": "approved", "comment": "looks fine" }),
    )
    .await;
    assert!(!matches!(
        resp.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
    ));
    assert_eq!(resp.status(), StatusCode::OK);

    // AFTER: the same principal now has catalog:write, through the real
    // access_grant row this decision created — not a fabricated success.
    let after = lakehouse_auth::repository::load_principal_for_user(
        &pool,
        requester_id,
        "local".to_owned(),
        false,
    )
    .await
    .expect("load principal after grant");
    assert!(after.has("catalog:write"));
}
