//! HTTP-level integration tests for `POST /api/agents/employees/{id}/run`
//! (T3.2 of the copilot-operations-handover plan): running a digital
//! employee headlessly — the same copilot tool-calling loop interactive
//! chat uses, but with no HTTP chat session, gated by the employee's OWN
//! permission ceiling rather than the triggering token's/principal's.
//!
//! The LLM is mocked with `wiremock`, following the pattern
//! `lakehouse-llm`'s own `chat_with_tools` tests use — a real Postgres is
//! used throughout (`common::spin_up_with_env`), so `agent_run`/
//! `approval_item`/`audit_event` rows are asserted for real.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use lakehouse_store::audit::{AuditFilter, list as list_audit};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{
    CLICKHOUSE_UNKNOWN_IDENTIFIER, COLUMN_DOES_NOT_EXIST, TestApp,
    create_principal_with_permissions, mock_clickhouse_refusing, model_tool_result,
    session_cookie_for_seeded_user, session_cookie_for_user, spin_up_with_env,
};

/// Insert a digital employee row directly (bypassing `POST
/// /api/agents/employees`, so a test can set an arbitrary
/// mode/permissions/prompt/status combination in one call without also
/// exercising the create route). Test-only fixture helper.
async fn insert_employee(
    pool: &PgPool,
    id: &str,
    name: &str,
    status: &str,
    mode: &str,
    permissions: &str,
    prompt: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO agent_employee (id, name, purpose, autonomy, status, prompt, mode, \
         permissions) VALUES ($1, $2, 'test purpose', 'L1', $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(name)
    .bind(status)
    .bind(prompt)
    .bind(mode)
    .bind(permissions)
    .execute(pool)
    .await
    .expect("insert a test employee");
}

async fn spin_up_with_llm(llm_url: &str, agent_run_token: Option<&str>) -> TestApp {
    let mut overrides = HashMap::new();
    overrides.insert("LLM_URL".to_owned(), llm_url.to_owned());
    if let Some(tok) = agent_run_token {
        overrides.insert("AGENT_RUN_TOKEN".to_owned(), tok.to_owned());
    }
    spin_up_with_env(&overrides).await
}

/// A `chat/completions` response with no `tool_calls` — the model answers
/// immediately, ending the headless loop `"succeeded"` after exactly one
/// step.
async fn mount_final_answer(mock: &MockServer, answer: &str) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "role": "assistant", "content": answer } }]
        })))
        .mount(mock)
        .await;
}

/// A `chat/completions` response that ALWAYS calls the given tool with the
/// given (already-JSON-encoded) arguments — used both for the `WriteHigh`
/// test (where the loop returns after exactly one such call) and the
/// permission-ceiling test (where the same response keeps coming back
/// every iteration, since the refused call is fed back as an error and the
/// mock has no state).
async fn mount_tool_call(mock: &MockServer, tool: &str, arguments: &str) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": { "name": tool, "arguments": arguments },
                    }],
                }
            }]
        })))
        .mount(mock)
        .await;
}

async fn run_employee(
    app: &axum::Router,
    id: &str,
    cookie: Option<&str>,
    token: Option<&str>,
    body: Value,
) -> axum::http::Response<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(format!("/api/agents/employees/{id}/run"))
        .header("content-type", "application/json");
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    if let Some(token) = token {
        builder = builder.header("x-run-token", token);
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

/// `POST /api/agents/approvals/{id}/decide` with a session cookie — the
/// other half of the approval flow, so a test can decide an approval the
/// headless run created.
async fn decide_approval(
    app: &axum::Router,
    approval_id: &str,
    cookie: &str,
    body: Value,
) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agents/approvals/{approval_id}/decide"))
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap()
}

/// A valid `x-run-token` runs the employee end to end: the run reaches a
/// terminal status (`"succeeded"`, since the mocked LLM answers with no
/// tool call) and has at least one recorded step.
#[tokio::test]
async fn valid_token_runs_the_employee_to_a_terminal_status_with_steps() {
    let mock = MockServer::start().await;
    mount_final_answer(&mock, "Semua kunjungan sudah dicek, tidak ada anomali.").await;
    let app = spin_up_with_llm(&mock.uri(), Some("secret-run-token")).await;

    insert_employee(
        &app.pool,
        "emp-headless-01",
        "headless-runner-01",
        "ready",
        "ask",
        "",
        Some("Cek kunjungan harian."),
    )
    .await;
    // The router's `Policy::RequiresAuth` floor still needs a REAL
    // credential (session/bearer) regardless of `x-run-token` — see
    // `routes::agents`' module doc comment and `routes::gold`/
    // `routes::alerts`'s identical "belt-and-suspenders" shape. A
    // real Dagster caller would send a service-token bearer here; a
    // session cookie exercises the exact same code path. Deliberately an
    // Analyst (no `agent:manage`) — the point of this test is that the
    // TOKEN, not the principal's own permission, is what authorizes it.
    let cookie = session_cookie_for_seeded_user(&app.pool, "sari@meridian.example").await;

    let resp = run_employee(
        &app.router,
        "emp-headless-01",
        Some(&cookie),
        Some("secret-run-token"),
        json!({}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    let run = &body["run"];
    assert_eq!(run["employeeId"], json!("emp-headless-01"));
    assert_eq!(run["trigger"], json!("schedule"));
    assert_eq!(run["status"], json!("succeeded"));
    assert_ne!(
        run["steps"].as_array().unwrap().as_slice(),
        &[] as &[serde_json::Value]
    );
}

/// Wrong/absent token AND no principal at all → 401.
#[tokio::test]
async fn wrong_token_and_no_principal_is_unauthorized() {
    let mock = MockServer::start().await;
    let app = spin_up_with_llm(&mock.uri(), Some("secret-run-token")).await;
    insert_employee(
        &app.pool,
        "emp-headless-02",
        "headless-runner-02",
        "ready",
        "ask",
        "",
        Some("p"),
    )
    .await;

    let resp = run_employee(
        &app.router,
        "emp-headless-02",
        None,
        Some("wrong"),
        json!({}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let resp = run_employee(&app.router, "emp-headless-02", None, None, json!({})).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// Wrong/absent token AND an authenticated principal who lacks
/// `agent:manage` → 403 (an Analyst has no `agent:*` grant at all).
#[tokio::test]
async fn wrong_token_and_unpermitted_principal_is_forbidden() {
    let mock = MockServer::start().await;
    let app = spin_up_with_llm(&mock.uri(), Some("secret-run-token")).await;
    insert_employee(
        &app.pool,
        "emp-headless-03",
        "headless-runner-03",
        "ready",
        "ask",
        "",
        Some("p"),
    )
    .await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "sari@meridian.example").await;

    let resp = run_employee(
        &app.router,
        "emp-headless-03",
        Some(&cookie),
        Some("wrong"),
        json!({}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

/// A principal holding `agent:manage` (Platform Admin) can run the
/// employee with NO token at all — `trigger` is `"manual"`.
#[tokio::test]
async fn principal_with_agent_manage_can_run_without_a_token() {
    let mock = MockServer::start().await;
    mount_final_answer(&mock, "Selesai.").await;
    let app = spin_up_with_llm(&mock.uri(), Some("secret-run-token")).await;
    insert_employee(
        &app.pool,
        "emp-headless-04",
        "headless-runner-04",
        "ready",
        "ask",
        "",
        Some("Cek data."),
    )
    .await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;

    let resp = run_employee(
        &app.router,
        "emp-headless-04",
        Some(&cookie),
        None,
        json!({}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["run"]["trigger"], json!("manual"));
    assert_eq!(body["run"]["actor"], json!("Fajar Nugroho"));
    assert_eq!(body["run"]["status"], json!("succeeded"));
}

/// A suspended (`"paused"`) employee is refused with 409.
#[tokio::test]
async fn suspended_employee_is_refused_with_409() {
    let mock = MockServer::start().await;
    let app = spin_up_with_llm(&mock.uri(), Some("secret-run-token")).await;
    insert_employee(
        &app.pool,
        "emp-headless-05",
        "headless-runner-05",
        "paused",
        "ask",
        "",
        Some("p"),
    )
    .await;

    let cookie = session_cookie_for_seeded_user(&app.pool, "sari@meridian.example").await;
    let resp = run_employee(
        &app.router,
        "emp-headless-05",
        Some(&cookie),
        Some("secret-run-token"),
        json!({}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

/// A revoked (`"cancelled"`) employee is refused with 409.
#[tokio::test]
async fn revoked_employee_is_refused_with_409() {
    let mock = MockServer::start().await;
    let app = spin_up_with_llm(&mock.uri(), Some("secret-run-token")).await;
    insert_employee(
        &app.pool,
        "emp-headless-06",
        "headless-runner-06",
        "cancelled",
        "ask",
        "",
        Some("p"),
    )
    .await;

    let cookie = session_cookie_for_seeded_user(&app.pool, "sari@meridian.example").await;
    let resp = run_employee(
        &app.router,
        "emp-headless-06",
        Some(&cookie),
        Some("secret-run-token"),
        json!({}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

/// The reserved `emp-copilot` row (seeded by `0025_agent_schedule.sql`)
/// can never be run headlessly.
#[tokio::test]
async fn emp_copilot_cannot_be_run_headlessly() {
    let mock = MockServer::start().await;
    let app = spin_up_with_llm(&mock.uri(), Some("secret-run-token")).await;

    let cookie = session_cookie_for_seeded_user(&app.pool, "sari@meridian.example").await;
    let resp = run_employee(
        &app.router,
        "emp-copilot",
        Some(&cookie),
        Some("secret-run-token"),
        json!({}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// An employee with no `prompt` and no override in the body is refused
/// with 400, not sent to the copilot with an empty instruction.
#[tokio::test]
async fn no_prompt_and_no_override_is_bad_request() {
    let mock = MockServer::start().await;
    let app = spin_up_with_llm(&mock.uri(), Some("secret-run-token")).await;
    insert_employee(
        &app.pool,
        "emp-headless-07",
        "headless-runner-07",
        "ready",
        "ask",
        "",
        None,
    )
    .await;

    let cookie = session_cookie_for_seeded_user(&app.pool, "sari@meridian.example").await;
    let resp = run_employee(
        &app.router,
        "emp-headless-07",
        Some(&cookie),
        Some("secret-run-token"),
        json!({}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// A body `{"prompt": "..."}` override is used in place of the employee's
/// own (or absent) `agent_employee.prompt`.
#[tokio::test]
async fn body_prompt_override_is_used_when_employee_has_none() {
    let mock = MockServer::start().await;
    mount_final_answer(&mock, "Oke, dikerjakan.").await;
    let app = spin_up_with_llm(&mock.uri(), Some("secret-run-token")).await;
    insert_employee(
        &app.pool,
        "emp-headless-08",
        "headless-runner-08",
        "ready",
        "ask",
        "",
        None,
    )
    .await;

    let cookie = session_cookie_for_seeded_user(&app.pool, "sari@meridian.example").await;
    let resp = run_employee(
        &app.router,
        "emp-headless-08",
        Some(&cookie),
        Some("secret-run-token"),
        json!({ "prompt": "Override instruction." }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["run"]["status"], json!("succeeded"));
}

/// Named test (plan invariant 7 / T3.2 acceptance): a run whose employee
/// `permissions` lacks a tool's required permission does NOT execute that
/// tool — even when the run is triggered by a Platform Admin (whose OWN
/// permissions would easily satisfy it). The employee's `permissions`
/// column is the ceiling, never the triggering principal's/token's own
/// grants.
#[tokio::test]
async fn run_employee_permission_ceiling_is_the_employees_own_not_the_triggering_principals() {
    let mock = MockServer::start().await;
    // The model always tries to call `list_saved_queries` (needs
    // `query:read`), which this employee's `permissions` column does NOT
    // grant (`""` — authenticated-only).
    mount_tool_call(&mock, "list_saved_queries", "{}").await;
    let app = spin_up_with_llm(&mock.uri(), Some("secret-run-token")).await;
    insert_employee(
        &app.pool,
        "emp-headless-09",
        "headless-runner-09",
        "ready",
        "build",
        "", // no permissions granted at all
        Some("Tampilkan saved query."),
    )
    .await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;

    // Triggered by fajar@meridian.example — Platform Admin, `*:*` — whose
    // OWN permissions would trivially satisfy `query:read`.
    let resp = run_employee(
        &app.router,
        "emp-headless-09",
        Some(&cookie),
        None,
        json!({}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    let run_id = body["run"]["id"].as_str().unwrap().to_owned();

    // The run never reaches "succeeded" via that tool — it exhausts the
    // iteration budget refusing the same call every time, ending "failed".
    assert_eq!(body["run"]["status"], json!("failed"));

    // Every step the run recorded for `list_saved_queries` is a refusal,
    // never a real execution.
    let steps = body["run"]["steps"].as_array().unwrap();
    let tool_steps: Vec<&Value> = steps
        .iter()
        .filter(|s| s["label"] == json!("list_saved_queries"))
        .collect();
    assert!(
        !tool_steps.is_empty(),
        "the disallowed tool must have been attempted"
    );
    for step in tool_steps {
        assert_eq!(step["status"], json!("refused"));
        assert!(step["detail"].as_str().unwrap().contains("query:read"));
    }

    // The audit trail agrees: only "refused" outcomes for that action, on
    // this run, never "executed".
    let events = list_audit(&app.pool, AuditFilter::default())
        .await
        .expect("list audit events");
    let for_this_run: Vec<_> = events
        .iter()
        .filter(|e| {
            e.run_id.as_deref() == Some(run_id.as_str()) && e.action == "list_saved_queries"
        })
        .collect();
    assert_ne!(for_this_run, [] as [&lakehouse_store::audit::AuditEvent; 0]);
    assert!(for_this_run.iter().all(|e| e.outcome == "refused"));
}

/// A run that triggers a `WriteHigh` tool call ends `"waiting_approval"`
/// and leaves a linked, `pending` `approval_item` attributed to the REAL
/// running employee (never `emp-copilot`).
#[tokio::test]
async fn write_high_tool_call_ends_waiting_approval_with_a_linked_approval() {
    let mock = MockServer::start().await;
    mount_tool_call(&mock, "delete_chart", r#"{"id":"c-999"}"#).await;
    let app = spin_up_with_llm(&mock.uri(), Some("secret-run-token")).await;
    insert_employee(
        &app.pool,
        "emp-headless-10",
        "headless-runner-10",
        "ready",
        "build",
        "dashboard:write",
        Some("Hapus chart lama."),
    )
    .await;

    let cookie = session_cookie_for_seeded_user(&app.pool, "sari@meridian.example").await;
    let resp = run_employee(
        &app.router,
        "emp-headless-10",
        Some(&cookie),
        Some("secret-run-token"),
        json!({}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["run"]["status"], json!("waiting_approval"));
    let run_id = body["run"]["id"].as_str().unwrap().to_owned();

    let approvals: Vec<Value> = body["run"]["approvals"].as_array().unwrap().clone();
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0]["status"], json!("pending"));

    // The approval row itself is attributed to the REAL employee, not
    // `emp-copilot`.
    let (employee_id, action, run_id_col, risk): (String, String, Option<String>, String) =
        sqlx::query_as("SELECT employee_id, action, run_id, risk FROM approval_item WHERE id = $1")
            .bind(approvals[0]["id"].as_str().unwrap())
            .fetch_one(&app.pool)
            .await
            .expect("fetch the created approval row");
    assert_eq!(employee_id, "emp-headless-10");
    assert_eq!(action, "delete_chart");
    assert_eq!(run_id_col.as_deref(), Some(run_id.as_str()));
    // The risk sentence is the one the chat's approval card shows for
    // `delete_chart`: a deleted chart goes to the trash and stays
    // restorable, so the generic "cannot be undone" sentence would be false
    // on this very card (`AI-1`, `GOV-13`).
    assert_eq!(
        risk,
        "Tindakan berisiko tinggi (WriteHigh): chart masuk tempat sampah dan bisa dipulihkan \
         selama 30 hari.",
        "the headless approval stores the same risk sentence as the chat's"
    );
}

/// LLM unavailable (transport failure — no mock mounted for this test's
/// dead upstream) → the run ends `"failed"` with a recorded reason, and
/// the handler never panics.
#[tokio::test]
async fn llm_unavailable_ends_the_run_failed_without_a_panic() {
    let app = spin_up_with_llm("http://127.0.0.1:1", Some("secret-run-token")).await;
    insert_employee(
        &app.pool,
        "emp-headless-11",
        "headless-runner-11",
        "ready",
        "ask",
        "",
        Some("Cek sesuatu."),
    )
    .await;

    let cookie = session_cookie_for_seeded_user(&app.pool, "sari@meridian.example").await;
    let resp = run_employee(
        &app.router,
        "emp-headless-11",
        Some(&cookie),
        Some("secret-run-token"),
        json!({}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["run"]["status"], json!("failed"));
    let steps = body["run"]["steps"].as_array().unwrap();
    assert_ne!(steps.as_slice(), &[] as &[serde_json::Value]);
    assert_eq!(steps[0]["status"], json!("failed"));
}

/// D (T3.2's fix): `POST /api/agents/employees` with an invalid `mode`
/// returns a clean 400, not a raw database `CHECK`-violation error.
#[tokio::test]
async fn create_employee_with_invalid_mode_is_bad_request() {
    let mock = MockServer::start().await;
    let app = spin_up_with_llm(&mock.uri(), None).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;

    let resp = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agents/employees")
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": "bad-mode-employee-http",
                        "purpose": "p",
                        "autonomy": "L1",
                        "dataScope": "d",
                        "budgetLimit": 0,
                        "mode": "sleep",
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// SEC-57: a SCHEDULED run's approval names NO requester — the schedule
/// asked, not a person — so anybody holding `agent:approve` may decide
/// it, exactly as before the self-approval rule. The run here starts with
/// the `x-run-token`, which is `RunAuth::Token`: there is no principal
/// behind it at all.
#[tokio::test]
async fn a_scheduled_runs_approval_has_no_requester_and_any_approver_may_decide_it() {
    let mock = MockServer::start().await;
    mount_tool_call(&mock, "delete_chart", r#"{"id":"c-999"}"#).await;
    let app = spin_up_with_llm(&mock.uri(), Some("secret-run-token")).await;
    insert_employee(
        &app.pool,
        "emp-headless-12",
        "headless-runner-12",
        "ready",
        "build",
        "dashboard:write",
        Some("Hapus chart lama."),
    )
    .await;

    // The route's policy floor is `RequiresAuth`, so a session has to ride
    // along; the matched `x-run-token` is what makes the run a SCHEDULED
    // one — `RunAuth::Token`, so the run has no principal behind it and
    // its audit rows say `trigger = "schedule"`.
    let cookie = session_cookie_for_seeded_user(&app.pool, "sari@meridian.example").await;
    let resp = run_employee(
        &app.router,
        "emp-headless-12",
        Some(&cookie),
        Some("secret-run-token"),
        json!({}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["run"]["status"], json!("waiting_approval"));
    let approvals: Vec<Value> = body["run"]["approvals"].as_array().unwrap().clone();
    assert_eq!(approvals.len(), 1);
    let approval_id = approvals[0]["id"].as_str().unwrap().to_owned();

    // No requester recorded — nobody asked in person.
    let (requester,): (Option<uuid::Uuid>,) =
        sqlx::query_as("SELECT requested_by_user_id FROM approval_item WHERE id = $1")
            .bind(&approval_id)
            .fetch_one(&app.pool)
            .await
            .expect("the approval row exists");
    assert_eq!(
        requester, None,
        "SEC-57: a scheduled run names no human requester, so nobody is refused as 'the requester'"
    );

    // Anybody holding `agent:approve` may decide it.
    let approver_id = create_principal_with_permissions(
        &app.pool,
        "dashboard:write, dashboard:read, agent:approve",
    )
    .await;
    let cookie = session_cookie_for_user(&app.pool, approver_id).await;
    let decided = decide_approval(
        &app.router,
        &approval_id,
        &cookie,
        json!({ "decision": "approved" }),
    )
    .await;
    assert_eq!(decided.status(), StatusCode::OK);
    let decided_body = json_body(decided).await;
    assert_eq!(decided_body["approval"]["status"], json!("approved"));
}

// ── LH-QS-2-08: a run's detail shows the fixed sentence, not the engine's text
//
// An engine failure reaches a person as a fixed sentence and a reference
// (owner rule LH-QS-2-08, 2026-11-12). A run's step and an outcome executed
// after approval are read by a browser on the run page, so they carry that
// copy. The model that drives a run still reads the engine's diagnosis.

/// The statement the mock engine refuses.
const FAILING_SQL: &str = "SELECT pengunjungg FROM silver.orders";

/// A statement the mock engine answers.
const WORKING_SQL: &str = "SELECT 1 AS n FROM silver.orders";

/// The text a person must never read from a failed statement.
const ENGINE_TEXT: [&str; 4] = [
    "UNKNOWN_IDENTIFIER",
    "Code:",
    "DB::Exception",
    "pengunjungg",
];

fn engine_refuses_the_statement() -> ResponseTemplate {
    ResponseTemplate::new(400).set_body_string(CLICKHOUSE_UNKNOWN_IDENTIFIER)
}

/// A proxy's page, not the engine saying the statement is wrong: the text
/// is this repository's own, so the person's copy and the model's copy are
/// the same fixed message.
fn engine_is_not_answering() -> ResponseTemplate {
    ResponseTemplate::new(502).set_body_string("<html><body>502 Bad Gateway</body></html>")
}

/// Runs an employee holding `query:read` whose model calls `run_sql` once
/// with `sql`, against a `ClickHouse` that refuses `FAILING_SQL` with
/// `answer`, then ends the run. Returns the run's `run_sql` step and the
/// mock model, so the model's side can be read.
async fn run_one_sql_step(sql: &str, answer: ResponseTemplate) -> (Value, MockServer) {
    let llm = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call-1",
                    "type": "function",
                    "function": { "name": "run_sql", "arguments": json!({ "sql": sql }).to_string() },
                }],
            } }]
        })))
        .up_to_n_times(1)
        .mount(&llm)
        .await;
    mount_final_answer(&llm, "The column is order_id.").await;
    let ch = mock_clickhouse_refusing("pengunjungg", answer).await;
    let mut env = HashMap::new();
    env.insert("LLM_URL".to_owned(), llm.uri());
    env.insert("CH_URL".to_owned(), ch.uri());
    let app = spin_up_with_env(&env).await;
    insert_employee(
        &app.pool,
        "emp-headless-sql",
        "headless-runner-sql",
        "ready",
        "ask",
        "query:read",
        Some("Cek kunjungan harian."),
    )
    .await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;

    let resp = run_employee(
        &app.router,
        "emp-headless-sql",
        Some(&cookie),
        None,
        json!({}),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    let steps = body["run"]["steps"].as_array().expect("steps are a list");
    let step = steps
        .iter()
        .find(|s| s["label"] == json!("run_sql"))
        .unwrap_or_else(|| panic!("the run recorded a run_sql step: {steps:?}"))
        .clone();
    (step, llm)
}

/// The step's `detail`, which is the tool result serialised as JSON.
fn step_detail(step: &Value) -> Value {
    serde_json::from_str(step["detail"].as_str().expect("a detail string"))
        .expect("the detail is JSON")
}

/// LH-QS-2-08: a `run_sql` step whose statement the engine refuses stores
/// the fixed sentence and the reference, and none of the engine's text,
/// while the model's next request carries the engine's diagnosis with the
/// SAME reference.
#[tokio::test]
async fn a_failed_run_sql_step_stores_the_fixed_sentence_and_the_model_reads_the_diagnosis() {
    let (step, llm) = run_one_sql_step(FAILING_SQL, engine_refuses_the_statement()).await;

    assert_eq!(step["status"], json!("failed"));
    let detail = step_detail(&step);
    let stored = detail["error"].as_str().unwrap_or_default();
    let reference = stored
        .strip_prefix(&format!("{COLUMN_DOES_NOT_EXIST} Reference: "))
        .unwrap_or_else(|| panic!("the fixed sentence and a reference: {detail}"));
    assert!(!reference.is_empty(), "{detail}");
    for engine_text in ENGINE_TEXT {
        assert!(
            !step["detail"].as_str().unwrap().contains(engine_text),
            "{engine_text} in {detail}"
        );
    }
    assert!(!step["detail"].as_str().unwrap().contains("_browserError"));

    let model = model_tool_result(&llm, 1).await;
    let model_error = model["error"].as_str().unwrap_or_default();
    assert!(
        model_error.contains("Unknown expression identifier 'pengunjungg' in scope."),
        "the model reads the engine's diagnosis: {model}"
    );
    assert!(
        model_error.contains(&format!(" Reference: {reference}")),
        "the model's copy carries the same reference: {model}"
    );
}

/// LH-QS-2-08, unchanged: a successful `run_sql` is stored exactly as the
/// model read it.
#[tokio::test]
async fn a_successful_run_sql_step_is_stored_as_the_model_read_it() {
    let (step, llm) = run_one_sql_step(WORKING_SQL, engine_refuses_the_statement()).await;

    assert_eq!(step["status"], json!("succeeded"));
    let detail = step_detail(&step);
    assert_eq!(detail["rowCount"], json!(1), "{detail}");
    assert_eq!(detail, model_tool_result(&llm, 1).await);
}

/// LH-QS-2-08, unchanged: a failure that is already a fixed message (the
/// engine did not answer) is stored as it is, and it is the model's text
/// too.
#[tokio::test]
async fn a_run_sql_step_that_fails_for_another_reason_stores_the_same_text_the_model_read() {
    let (step, llm) = run_one_sql_step(FAILING_SQL, engine_is_not_answering()).await;

    assert_eq!(step["status"], json!("failed"));
    let detail = step_detail(&step);
    assert!(
        detail["error"]
            .as_str()
            .unwrap_or_default()
            .starts_with("The database request failed. Reference: "),
        "{detail}"
    );
    assert_eq!(detail, model_tool_result(&llm, 1).await);
}

/// Creates a pending approval for a `run_sql` call, the way a copilot call
/// would have (an approval for a `Read` tool does not arise in practice,
/// but the decision route replays whatever tool the approval names), and
/// has a second principal approve it. Returns the HTTP body of the decision
/// and the run's last step, the outcome the route stored.
async fn approve_a_pending_run_sql(sql: &str, answer: ResponseTemplate) -> (Value, Value) {
    let ch = mock_clickhouse_refusing("pengunjungg", answer).await;
    let mut env = HashMap::new();
    env.insert("CH_URL".to_owned(), ch.uri());
    env.insert(
        "CATALOG_TENANT_ID".to_owned(),
        common::CATALOG_TENANT.to_owned(),
    );
    let app = spin_up_with_env(&env).await;
    let args = json!({ "sql": sql });
    let created = lakehouse_store::agents::create_pending_approval(
        &app.pool,
        lakehouse_store::agents::NewApprovalRequest {
            tool: "run_sql",
            actor: "test",
            resource: None,
            reason: "test",
            risk: "test",
            redacted_args: &args,
            requested_by: None,
        },
    )
    .await
    .expect("create a pending approval");
    let approver =
        create_principal_with_permissions(&app.pool, "query:read, catalog:read, agent:approve")
            .await;
    // The statement reads the shared catalog, which only its own tenant's
    // members may.
    common::in_the_catalog_tenant(&app.pool, approver).await;
    let cookie = session_cookie_for_user(&app.pool, approver).await;

    let resp = decide_approval(
        &app.router,
        &created.approval_id,
        &cookie,
        json!({ "decision": "approved" }),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    let run = lakehouse_store::agents::get_run(&app.pool, &created.run_id)
        .await
        .expect("get the run")
        .expect("the run exists");
    let outcome = run.steps.last().expect("the run has an outcome step");
    assert_eq!(outcome.label, "Executed after approval");
    let stored = serde_json::to_value(outcome).expect("serialise the step");
    (body, stored)
}

/// LH-QS-2-08: an outcome executed after approval shows the fixed sentence
/// and the reference, in the run's stored outcome and in the HTTP answer,
/// and none of the engine's text.
#[tokio::test]
async fn an_outcome_executed_after_approval_shows_the_fixed_sentence() {
    let (body, outcome) =
        approve_a_pending_run_sql(FAILING_SQL, engine_refuses_the_statement()).await;

    assert_eq!(outcome["status"], json!("failed"));
    let stored = step_detail(&outcome);
    let stored_error = stored["error"].as_str().unwrap_or_default();
    assert!(
        stored_error.starts_with(&format!("{COLUMN_DOES_NOT_EXIST} Reference: ")),
        "{stored}"
    );
    let answered_error = body["result"]["error"].as_str().unwrap_or_default();
    assert_eq!(answered_error, stored_error, "{body}");
    for engine_text in ENGINE_TEXT {
        assert!(
            !outcome["detail"].as_str().unwrap().contains(engine_text),
            "{engine_text} in the stored outcome {stored}"
        );
        // The answer's `approval` echoes the statement itself, so only the
        // tool's `result` is read.
        assert!(
            !body["result"].to_string().contains(engine_text),
            "{engine_text} in the answer {body}"
        );
    }
    assert!(
        !body["result"].to_string().contains("_browserError"),
        "{body}"
    );
}

/// LH-QS-2-08, unchanged: an outcome whose tool succeeded, or failed with
/// a message that is already fixed, is stored and answered as the tool
/// returned it.
#[tokio::test]
async fn an_outcome_executed_after_approval_is_unchanged_without_an_engine_failure() {
    let (body, outcome) =
        approve_a_pending_run_sql(WORKING_SQL, engine_refuses_the_statement()).await;
    assert_eq!(outcome["status"], json!("succeeded"));
    assert_eq!(step_detail(&outcome)["rowCount"], json!(1));
    assert_eq!(body["result"], step_detail(&outcome));

    let (body, outcome) = approve_a_pending_run_sql(FAILING_SQL, engine_is_not_answering()).await;
    assert_eq!(outcome["status"], json!("failed"));
    let stored = step_detail(&outcome);
    assert!(
        stored["error"]
            .as_str()
            .unwrap_or_default()
            .starts_with("The database request failed. Reference: "),
        "{stored}"
    );
    assert_eq!(body["result"], stored);
}
