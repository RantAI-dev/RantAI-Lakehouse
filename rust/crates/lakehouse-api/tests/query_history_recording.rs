//! Every run that reaches an answer is recorded (`QS-4`, decision 3).
//!
//! `run` used to write a history row only for a completed run, so a
//! refused or failed run existed nowhere but the caller's screen. Now the
//! same best-effort write that records a completed run also records a
//! `blocked` refusal (the policy rewrite, the `SEC-20` table gate, and
//! their sibling guards) and a `failed` engine call, each row carrying the
//! exact text the response carries — so the history page can show why a
//! run looks the way it does, without the row and the screen ever
//! disagreeing.
//!
//! These tests drive the real router over the seeded tenants, with a
//! `wiremock` `ClickHouse` wired through `CH_URL` — the arrangement of
//! `tests/query_tenant_scope.rs` — and read the recorded rows back through
//! the pool. The fake engine failure's body carries a marker string that
//! must appear in neither the response nor the row: an upstream error is
//! classified before it is surfaced, and the row stores what the response
//! carried, nothing more.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::uploads::claim_table;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

/// The tenant that owns the shared catalog and its upload table below.
const OWNER_TENANT: &str = "11111111-1111-4111-8111-000000000001";
/// The upload table the owner tenant claimed.
const OWNED_TABLE: &str = "group_orders";
/// The statement that reads it.
const OWNED_STATEMENT: &str = "SELECT * FROM bronze.group_orders";
/// The table refusal, byte for byte: the table as the gate canonicalises it.
const TABLE_REFUSAL: &str =
    "Table not found or not available to your organisation: bronze.group_orders";
/// The policy refusal's fixed sentence, as `policy_engine::refusal_message`
/// words the rewrite's sensitive-system-table rule.
const POLICY_REFUSAL: &str = "Tabel sistem ini tidak bisa dibaca dari sini; gunakan halaman Audit.";
/// Text only the fake engine's failure body carries. It must appear in
/// neither the response nor the history row.
const ENGINE_MARKER: &str = "TANGERINE-PANEL-NINE";

/// The caller acts in a deployment whose shared catalog the owner tenant
/// holds (`CATALOG_TENANT_ID`), with `ClickHouse` at the mock.
async fn spin_up_with_clickhouse(ch_uri: &str) -> TestApp {
    let mut overrides = HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch_uri.to_owned());
    overrides.insert("CATALOG_TENANT_ID".to_owned(), OWNER_TENANT.to_owned());
    spin_up_with_env(&overrides).await
}

/// Answers the rewrite's own `system.*` lookups with an empty table, so
/// that only the run's own statement is left to reach the engine mock.
async fn mount_prefetch_ok(ch: &MockServer) {
    Mock::given(method("POST"))
        .and(body_string_contains("system."))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [
                {"name": "name", "type": "String"},
                {"name": "type", "type": "String"},
                {"name": "default_kind", "type": "String"},
                {"name": "default_expression", "type": "String"},
            ],
            "data": [],
            "rows": 0,
        })))
        .mount(ch)
        .await;
}

/// The engine answers every run with a failure whose body carries only the
/// marker — a text no classification is allowed to pass through.
async fn mount_prefetch_ok_and_engine_failure(ch: &MockServer) {
    mount_prefetch_ok(ch).await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string(ENGINE_MARKER))
        .mount(ch)
        .await;
}

/// Answers the column lookup the rewrite prefetches with an empty table,
/// and any other statement with one three-column row — enough for the run
/// to complete.
async fn mount_engine(ch: &MockServer) {
    mount_prefetch_ok(ch).await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [
                {"name": "database", "type": "String"},
                {"name": "table", "type": "String"},
                {"name": "rows", "type": "UInt64"},
            ],
            "data": [{"database": "bronze", "table": "group_orders", "rows": 10}],
            "rows": 1,
        })))
        .mount(ch)
        .await;
}

/// How many requests the engine received, prefetches included.
async fn engine_requests(ch: &MockServer) -> usize {
    ch.received_requests()
        .await
        .expect("mock records requests")
        .len()
}

/// The statements the engine received that were not the rewrite's own
/// `system.` lookups — the caller's statements, in short.
async fn statements_reaching_the_engine(ch: &MockServer) -> Vec<String> {
    ch.received_requests()
        .await
        .expect("mock records requests")
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .filter(|b| !b.contains("system."))
        .collect()
}

/// A tenant claims the upload table, the way an upload does.
async fn claim_table_for(pool: &PgPool, tenant: &str) {
    let claimed = claim_table(
        pool,
        tenant.parse::<Uuid>().expect("a seeded tenant id"),
        OWNED_TABLE,
        "upload-group-orders",
    )
    .await
    .expect("claim the upload table for the tenant");
    assert!(claimed, "the tenant must hold the claim");
}

/// A run request with the given JSON body, as the console sends it.
async fn post_json(router: &axum::Router, cookie: &str, body: Value) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/query/run")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, body)
}

/// A run request carrying a statement.
async fn post_sql(router: &axum::Router, cookie: &str, sql: &str) -> (StatusCode, Value) {
    post_json(router, cookie, json!({ "sql": sql })).await
}

/// How many rows the history holds — each test runs against its own fresh
/// database, so the count is the test's own.
async fn history_rows(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM query_history")
        .fetch_one(pool)
        .await
        .expect("read the run history")
}

/// The one row the caller's run recorded: its status, its message, and the
/// tables its statement named.
async fn the_recorded_run(pool: &PgPool, email: &str) -> (String, Option<String>, Vec<String>) {
    let (user_id,): (Uuid,) = sqlx::query_as("SELECT id FROM app_user WHERE email = $1")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("seeded user {email} must exist (0002_seed_identity.sql): {e}"));
    sqlx::query_as("SELECT status, message, tables_read FROM query_history WHERE owner_id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .expect("the run's row is recorded")
}

#[tokio::test]
async fn a_failed_engine_run_records_failed_with_the_responses_message_and_not_the_engines_text() {
    let ch = MockServer::start().await;
    mount_prefetch_ok_and_engine_failure(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let (status, body) = post_sql(&router, &cookie, "SELECT 1").await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let response_message = body["error"].as_str().expect("the fixed error text");
    assert!(response_message.contains("Reference: "), "{body}");
    assert!(!response_message.contains(ENGINE_MARKER), "{body}");
    let (recorded_status, message, tables_read) =
        the_recorded_run(&pool, "rina@meridian.example").await;
    assert_eq!(
        recorded_status, "failed",
        "the engine's failure is the run's outcome"
    );
    assert_eq!(
        message.as_deref(),
        Some(response_message),
        "the row says what the caller was told"
    );
    assert!(
        !message
            .expect("a failed row carries a message")
            .contains(ENGINE_MARKER)
    );
    assert!(
        tables_read.is_empty(),
        "SELECT 1 names no table: {tables_read:?}"
    );
}

#[tokio::test]
async fn a_policy_refusal_records_blocked_and_the_statement_never_runs() {
    let ch = MockServer::start().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let (status, body) = post_sql(&router, &cookie, "SELECT query FROM system.query_log").await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"], POLICY_REFUSAL, "{body}");
    let (recorded_status, message, tables_read) =
        the_recorded_run(&pool, "rina@meridian.example").await;
    assert_eq!(recorded_status, "blocked");
    assert_eq!(message.as_deref(), Some(POLICY_REFUSAL));
    assert_eq!(
        tables_read,
        vec!["system.query_log".to_owned()],
        "{tables_read:?}"
    );
    // The rewrite's own `system.` prefetch may reach the engine; the
    // caller's statement itself must not.
    assert!(
        statements_reaching_the_engine(&ch).await.is_empty(),
        "the statement the caller wrote never reaches the engine"
    );
}

#[tokio::test]
async fn a_table_another_tenant_claims_records_blocked_with_the_fixed_refusal() {
    let ch = MockServer::start().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    claim_table_for(&pool, OWNER_TENANT).await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let (status, body) = post_sql(&router, &cookie, OWNED_STATEMENT).await;

    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], TABLE_REFUSAL, "{body}");
    assert_eq!(
        engine_requests(&ch).await,
        0,
        "the tenant gate refuses before anything is asked of the engine"
    );
    let (recorded_status, message, tables_read) =
        the_recorded_run(&pool, "sari@meridian.example").await;
    assert_eq!(recorded_status, "blocked");
    assert_eq!(message.as_deref(), Some(TABLE_REFUSAL));
    assert_eq!(
        tables_read,
        vec!["bronze.group_orders".to_owned()],
        "{tables_read:?}"
    );
}

#[tokio::test]
async fn a_completed_run_records_the_tables_its_statement_names() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    claim_table_for(&pool, OWNER_TENANT).await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let (status, body) = post_sql(&router, &cookie, OWNED_STATEMENT).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let (recorded_status, message, tables_read) =
        the_recorded_run(&pool, "rina@meridian.example").await;
    assert_eq!(recorded_status, "completed");
    assert_eq!(message, None, "a completed run has nothing to say");
    assert_eq!(
        tables_read,
        vec!["bronze.group_orders".to_owned()],
        "{tables_read:?}"
    );
}

#[tokio::test]
async fn a_body_without_sql_records_nothing() {
    let ch = MockServer::start().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    for body in [json!({}), json!({ "sql": "" })] {
        let (status, response) = post_json(&router, &cookie, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
        assert_eq!(response["error"], "sql is required", "{response}");
    }
    assert_eq!(
        history_rows(&pool).await,
        0,
        "nothing to record: no statement, no run"
    );
}
