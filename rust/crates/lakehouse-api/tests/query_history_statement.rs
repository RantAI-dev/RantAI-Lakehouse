//! `POST /api/query/run` records the statement the user wrote in query
//! history, not the statement sent to the engine (`SEC-21`, `SEC-33`).
//!
//! History is what the console shows back to the user and what
//! `GET /api/query/run/{id}/download` re-runs. A history row that held the
//! rewritten statement showed the user the policy's expressions and made the
//! download apply the policy on top of an older copy of itself. These tests
//! drive the real router and a real per-test Postgres database, with a
//! `wiremock` stand-in for `ClickHouse` (`CH_URL` override), the arrangement
//! of `tests/query_download.rs`.

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

/// What the mask on `email` looks like in a rewritten statement.
const MASK_EXPRESSION: &str = "replaceRegexpOne(toString(`email`)";
/// The row filter as the policy authors it.
const ROW_FILTER: &str = "region != 'zz'";
/// The same filter as the rewrite prints it into a statement.
const REWRITTEN_FILTER: &str = "region <> 'zz'";
/// What the user types.
const USER_STATEMENT: &str = "SELECT email FROM serving.mart_x";

async fn spin_up_with_clickhouse(ch_uri: &str) -> TestApp {
    let mut overrides = std::collections::HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch_uri.to_owned());
    // The shared catalog belongs to one tenant, and the query path is
    // scoped to the tenant: this principal must be the catalog's own.
    overrides.insert(
        "CATALOG_TENANT_ID".to_owned(),
        "11111111-1111-4111-8111-000000000001".to_owned(),
    );
    spin_up_with_env(&overrides).await
}

/// Answers, in this order: the column lookup the rewrite makes, a download
/// (its statement ends in `FORMAT CSV`) with a small CSV, and any other
/// statement (a run) with a `FORMAT JSON` result.
async fn mount_columns_csv_and_json(ch: &MockServer) {
    Mock::given(method("POST"))
        .and(body_string_contains("system.columns"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [
                {"name": "name", "type": "String"},
                {"name": "type", "type": "String"},
                {"name": "default_kind", "type": "String"},
                {"name": "default_expression", "type": "String"},
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
        .and(body_string_contains("FORMAT CSV"))
        .respond_with(ResponseTemplate::new(200).set_body_string("email\n***\n"))
        .mount(ch)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [{"name": "email", "type": "String"}],
            "data": [{"email": "***"}],
            "rows": 1,
        })))
        .mount(ch)
        .await;
}

async fn seed_mask_and_row_filter_policy(pool: &PgPool) {
    governance::create_policy(
        pool,
        &CreatePolicyInput {
            name: "history-statement-test".to_owned(),
            kind: "Row filter".to_owned(),
            subjects: "Analyst".to_owned(),
            resources: "serving.mart_x".to_owned(),
            effect: "Permit with obligation".to_owned(),
            conditions: Some(
                json!({
                    "roles": ["Analyst"],
                    "table": "serving.mart_x",
                    "mask": ["email"],
                    "rowFilter": ROW_FILTER,
                })
                .to_string(),
            ),
            activate: true,
            owner: None,
        },
    )
    .await
    .expect("seed the governing policy");
}

/// The statements `ClickHouse` received, in order, without the rewrite's own
/// `system.*` column lookups.
async fn sent_statements(ch: &MockServer) -> Vec<String> {
    ch.received_requests()
        .await
        .expect("mock records requests")
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .filter(|b| !b.contains("system."))
        .collect()
}

async fn json_body(resp: axum::http::Response<Body>) -> Value {
    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("JSON body")
}

/// Runs `sql` as the cookie's user and returns the new history id.
async fn run_statement(router: &axum::Router, cookie: &str, sql: &str) -> String {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/query/run")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(json!({ "sql": sql }).to_string()))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(resp.status(), StatusCode::OK);
    json_body(resp).await["id"]
        .as_str()
        .expect("the run response carries its history id")
        .to_owned()
}

/// The `sql` of the history row `id`, read through the route the console
/// reads.
async fn history_sql(router: &axum::Router, cookie: &str, id: &str) -> String {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/query/history")
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(resp.status(), StatusCode::OK);
    let rows = json_body(resp).await;
    rows.as_array()
        .expect("history is an array")
        .iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("history holds the run {id}: {rows}"))["sql"]
        .as_str()
        .expect("a history row carries its sql")
        .to_owned()
}

async fn download_csv(router: &axum::Router, cookie: &str, id: &str) -> StatusCode {
    router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/query/run/{id}/download?format=csv"))
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright")
        .status()
}

#[tokio::test]
async fn history_holds_the_statement_the_user_wrote_and_not_the_policys_expressions() {
    let ch = MockServer::start().await;
    mount_columns_csv_and_json(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    seed_mask_and_row_filter_policy(&pool).await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let id = run_statement(&router, &cookie, USER_STATEMENT).await;

    let stored = history_sql(&router, &cookie, &id).await;
    assert_eq!(stored, USER_STATEMENT);
    assert!(!stored.contains(MASK_EXPRESSION), "{stored}");
    assert!(!stored.contains(REWRITTEN_FILTER), "{stored}");
}

#[tokio::test]
async fn the_engine_and_the_audit_event_get_the_rewritten_statement() {
    let ch = MockServer::start().await;
    mount_columns_csv_and_json(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    seed_mask_and_row_filter_policy(&pool).await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let id = run_statement(&router, &cookie, USER_STATEMENT).await;

    let sent = sent_statements(&ch).await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].contains(MASK_EXPRESSION), "{}", sent[0]);
    assert!(sent[0].contains(REWRITTEN_FILTER), "{}", sent[0]);

    let audited: Value = sqlx::query_scalar(
        "SELECT args FROM audit_event \
         WHERE resource_kind = 'query_history' AND resource_id = $1",
    )
    .bind(&id)
    .fetch_one(&pool)
    .await
    .expect("the run recorded an audit event");
    let audited_sql = audited["sql"].as_str().expect("the audit args carry sql");
    assert!(audited_sql.contains(MASK_EXPRESSION), "{audited_sql}");
    assert!(audited_sql.contains(REWRITTEN_FILTER), "{audited_sql}");
}

#[tokio::test]
async fn a_download_of_a_run_applies_the_policy_once() {
    let ch = MockServer::start().await;
    mount_columns_csv_and_json(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    seed_mask_and_row_filter_policy(&pool).await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let id = run_statement(&router, &cookie, USER_STATEMENT).await;
    assert_eq!(download_csv(&router, &cookie, &id).await, StatusCode::OK);

    let sent = sent_statements(&ch).await;
    assert_eq!(sent.len(), 2, "the run, then the download: {sent:?}");
    let downloaded = &sent[1];
    assert!(downloaded.contains("FORMAT CSV"), "{downloaded}");
    assert_eq!(
        downloaded.matches(MASK_EXPRESSION).count(),
        1,
        "{downloaded}"
    );
    assert_eq!(
        downloaded.matches(REWRITTEN_FILTER).count(),
        1,
        "{downloaded}"
    );
}
