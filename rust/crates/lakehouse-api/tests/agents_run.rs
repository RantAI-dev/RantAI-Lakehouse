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

mod common;

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use lakehouse_store::audit::{AuditFilter, list as list_audit};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

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
    assert!(!run["steps"].as_array().unwrap().is_empty());
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

/// The reserved `emp-copilot` row (seeded by `0024_agent_schedule.sql`)
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
    assert!(!for_this_run.is_empty());
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
    let (employee_id, action, run_id_col): (String, String, Option<String>) =
        sqlx::query_as("SELECT employee_id, action, run_id FROM approval_item WHERE id = $1")
            .bind(approvals[0]["id"].as_str().unwrap())
            .fetch_one(&app.pool)
            .await
            .expect("fetch the created approval row");
    assert_eq!(employee_id, "emp-headless-10");
    assert_eq!(action, "delete_chart");
    assert_eq!(run_id_col.as_deref(), Some(run_id.as_str()));
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
    assert!(!steps.is_empty());
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
