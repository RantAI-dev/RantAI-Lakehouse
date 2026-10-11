//! `POST /api/query/estimate` and the assistant's `run_sql` dry run explain
//! the statement the run would send: the one rewritten for the caller's
//! current roles, with the run route's own function. A statement the rewrite
//! refuses is refused with the run's status and fixed message and never
//! reaches the engine. Driven through the real router, a real per-test
//! Postgres (policies) and a `wiremock` stand-in for `ClickHouse`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::governance::{self, CreatePolicyInput};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

async fn mount(ch: &MockServer) {
    Mock::given(method("POST"))
        .and(body_string_contains("system.columns"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [
                {"name": "name", "type": "String"}, {"name": "type", "type": "String"},
                {"name": "default_kind", "type": "String"}, {"name": "default_expression", "type": "String"}
            ],
            "data": [
                {"name": "email", "type": "String", "default_kind": "", "default_expression": ""},
                {"name": "region", "type": "String", "default_kind": "", "default_expression": ""},
            ],
            "rows": 2,
        })))
        .mount(ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("EXPLAIN ESTIMATE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "data": [{"database": "serving", "table": "mart_x", "parts": 1, "rows": 10, "marks": 1}], "rows": 1,
        })))
        .mount(ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("EXPLAIN AST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("SelectWithUnionQuery (children 1)\n"),
        )
        .mount(ch)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [{"name": "email", "type": "String"}], "data": [{"email": "***"}], "rows": 1,
        })))
        .mount(ch)
        .await;
}

async fn seed_mask(pool: &PgPool, role: &str) {
    governance::create_policy(
        pool,
        &CreatePolicyInput {
            name: format!("explain-rewrite-{role}"),
            kind: "Row filter".to_owned(),
            subjects: role.to_owned(),
            resources: "serving.mart_x".to_owned(),
            effect: "Permit with obligation".to_owned(),
            conditions: Some(
                json!({"roles": [role], "table": "serving.mart_x", "mask": ["email"]}).to_string(),
            ),
            activate: true,
            owner: None,
        },
    )
    .await
    .unwrap();
}

async fn app(ch: &MockServer) -> (axum::Router, PgPool, String) {
    let mut overrides = std::collections::HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch.uri());
    let TestApp { router, pool } = spin_up_with_env(&overrides).await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    (router, pool, cookie)
}

async fn post(router: &axum::Router, uri: &str, cookie: &str, body: Value) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn explained(ch: &MockServer, kind: &str) -> Vec<String> {
    ch.received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .filter(|b| b.contains(kind))
        .collect()
}

const SQL: &str = "SELECT email FROM serving.mart_x";

#[tokio::test]
async fn the_estimate_explains_the_statement_rewritten_for_the_callers_role() {
    let ch = MockServer::start().await;
    mount(&ch).await;
    let (router, pool, cookie) = app(&ch).await;
    seed_mask(&pool, "Analyst").await;

    let (status, body) = post(
        &router,
        "/api/query/estimate",
        &cookie,
        json!({ "sql": SQL }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let sent = explained(&ch, "EXPLAIN ESTIMATE").await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(
        sent[0].contains("replaceRegexpOne(toString(`email`)"),
        "{}",
        sent[0]
    );
    // What the planner reports is what the run reads, nothing of the policy.
    assert_eq!(body["sources"], json!(["serving.mart_x"]), "{body}");
    assert!(!body.to_string().contains("replaceRegexpOne"), "{body}");
}

#[tokio::test]
async fn the_estimate_for_a_role_the_policy_does_not_govern_is_unchanged() {
    let ch = MockServer::start().await;
    mount(&ch).await;
    let (router, pool, cookie) = app(&ch).await;
    seed_mask(&pool, "Auditor").await;

    let (status, _) = post(
        &router,
        "/api/query/estimate",
        &cookie,
        json!({ "sql": SQL }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let sent = explained(&ch, "EXPLAIN ESTIMATE").await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(
        sent[0].starts_with("EXPLAIN ESTIMATE SELECT email FROM serving.mart_x"),
        "{}",
        sent[0]
    );
}

#[tokio::test]
async fn a_statement_the_rewrite_refuses_is_refused_by_the_estimate_and_never_sent() {
    let ch = MockServer::start().await;
    mount(&ch).await;
    let (router, _pool, cookie) = app(&ch).await;
    let (status, body) = post(
        &router,
        "/api/query/estimate",
        &cookie,
        json!({ "sql": "SELECT * FROM numbers(5)" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(!body.to_string().contains("numbers"), "{body}");
    assert!(
        explained(&ch, "EXPLAIN").await.is_empty(),
        "nothing may be explained"
    );
}

fn run_sql_call(sql: &str) -> Value {
    json!({ "tool": "run_sql", "args": { "sql": sql }, "mode": "build" })
}

#[tokio::test]
async fn the_assistants_dry_run_explains_the_rewritten_statement() {
    let ch = MockServer::start().await;
    mount(&ch).await;
    let (router, pool, cookie) = app(&ch).await;
    seed_mask(&pool, "Analyst").await;

    let (status, body) = post(&router, "/api/ai/tool", &cookie, run_sql_call(SQL)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let sent = explained(&ch, "EXPLAIN AST").await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(
        sent[0].contains("replaceRegexpOne(toString(`email`)"),
        "{}",
        sent[0]
    );
}

#[tokio::test]
async fn the_assistants_dry_run_for_an_ungoverned_role_is_unchanged() {
    let ch = MockServer::start().await;
    mount(&ch).await;
    let (router, pool, cookie) = app(&ch).await;
    seed_mask(&pool, "Auditor").await;

    let (status, _) = post(&router, "/api/ai/tool", &cookie, run_sql_call(SQL)).await;
    assert_eq!(status, StatusCode::OK);
    let sent = explained(&ch, "EXPLAIN AST").await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(
        sent[0].starts_with("EXPLAIN AST SELECT email FROM serving.mart_x"),
        "{}",
        sent[0]
    );
}

#[tokio::test]
async fn a_statement_the_rewrite_refuses_is_refused_by_the_assistant_and_never_explained() {
    let ch = MockServer::start().await;
    mount(&ch).await;
    let (router, _pool, cookie) = app(&ch).await;
    // A table function, which the policy refuses.
    let (status, body) = post(
        &router,
        "/api/ai/tool",
        &cookie,
        run_sql_call("SELECT * FROM numbers(5)"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["result"]["error"].is_string(), "{body}");
    assert!(
        explained(&ch, "EXPLAIN").await.is_empty(),
        "nothing may be explained"
    );
}
