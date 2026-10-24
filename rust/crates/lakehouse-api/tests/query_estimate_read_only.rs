//! `POST /api/query/estimate` runs only what `POST /api/query/run` may run
//! (SEC-19).
//!
//! The estimate sends `EXPLAIN ESTIMATE <statement>` to `ClickHouse`, and
//! before that the rewrite prefetches the policy lookups, so a statement
//! that is not a read must be refused at the top of the handler, with the
//! run's status and message, before anything reaches the engine. The
//! `ClickHouse` stand-in is a `wiremock` server wired through `CH_URL`, so
//! every test can assert on exactly what the engine was sent.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The message `routes::query::run` gives for a statement that is not a read.
const NOT_A_READ: &str =
    "Only read queries (SELECT/SHOW/DESCRIBE/EXPLAIN) are allowed in Query Studio.";

/// The fixed message `sql_rewrite::enforce` gives for a table function
/// (`policy_engine::refusal_message`, `TableFunctionDenied`).
const TABLE_FUNCTION_DENIED: &str =
    "Table function tidak diizinkan pada query yang diatur kebijakan.";

async fn spin_up_with_clickhouse(ch_uri: &str) -> TestApp {
    let mut overrides = HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch_uri.to_owned());
    overrides.insert(
        "CATALOG_TENANT_ID".to_owned(),
        "11111111-1111-4111-8111-000000000001".to_owned(),
    );
    spin_up_with_env(&overrides).await
}

/// Answers the column lookup the rewrite prefetches with an empty table and
/// any other statement (the `EXPLAIN ESTIMATE`, the freshness lookup) with
/// one planner row.
async fn mount_columns_then_explain(ch: &MockServer) {
    Mock::given(method("POST"))
        .and(body_string_contains("system.columns"))
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
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [
                {"name": "database", "type": "String"},
                {"name": "table", "type": "String"},
                {"name": "rows", "type": "UInt64"},
            ],
            "data": [{"database": "serving", "table": "t", "rows": 10}],
            "rows": 1,
        })))
        .mount(ch)
        .await;
}

async fn estimate_as_reader(app: &TestApp, sql: &str) -> (StatusCode, String) {
    let user = create_principal_with_permissions(&app.pool, "query:read").await;
    // The shared catalog belongs to one tenant, and the query path is
    // scoped to the tenant: this principal is put in the catalog's own.
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2::uuid)")
        .bind(user)
        .bind("11111111-1111-4111-8111-000000000001")
        .execute(&app.pool)
        .await
        .expect("attach the fixture principal to the catalog's tenant");
    let cookie = session_cookie_for_user(&app.pool, user).await;
    let resp = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/query/estimate")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(json!({ "sql": sql }).to_string()))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Every statement the mock received, in order.
async fn received_statements(ch: &MockServer) -> Vec<String> {
    ch.received_requests()
        .await
        .expect("mock records requests")
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect()
}

#[tokio::test]
async fn an_estimate_of_a_write_is_refused_and_nothing_reaches_the_engine() {
    let ch = MockServer::start().await;
    mount_columns_then_explain(&ch).await;
    let app = spin_up_with_clickhouse(&ch.uri()).await;

    let (status, body) = estimate_as_reader(&app, "INSERT INTO serving.t VALUES (1)").await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(body.contains(NOT_A_READ), "{body}");
    let sent = received_statements(&ch).await;
    assert!(
        sent.is_empty(),
        "the guard precedes the policy prefetch, so the engine is sent nothing: {sent:?}"
    );
}

#[tokio::test]
async fn an_estimate_of_a_table_function_is_refused_and_never_explained() {
    let ch = MockServer::start().await;
    mount_columns_then_explain(&ch).await;
    let app = spin_up_with_clickhouse(&ch.uri()).await;

    let (status, body) =
        estimate_as_reader(&app, "SELECT * FROM url('http://example.invalid/x', 'CSV')").await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(body.contains(TABLE_FUNCTION_DENIED), "{body}");
    // The rewrite may prefetch a `system.` lookup; the statement itself must
    // never be sent.
    let leaked: Vec<String> = received_statements(&ch)
        .await
        .into_iter()
        .filter(|s| !s.contains("system."))
        .collect();
    assert!(
        leaked.is_empty(),
        "no statement besides policy lookups may reach the engine: {leaked:?}"
    );
}

#[tokio::test]
async fn an_estimate_of_a_read_still_reaches_explain() {
    let ch = MockServer::start().await;
    mount_columns_then_explain(&ch).await;
    let app = spin_up_with_clickhouse(&ch.uri()).await;

    let (status, body) = estimate_as_reader(&app, "SELECT a FROM serving.t").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let json: Value = serde_json::from_str(&body).expect("json");
    assert!(json["error"].is_null(), "{body}");
    assert_eq!(json["estimatedBytes"], 640, "10 rows at 64 bytes: {body}");
    let explained: Vec<String> = received_statements(&ch)
        .await
        .into_iter()
        .filter(|s| s.starts_with("EXPLAIN ESTIMATE"))
        .collect();
    assert_eq!(explained.len(), 1, "{explained:?}");
}
