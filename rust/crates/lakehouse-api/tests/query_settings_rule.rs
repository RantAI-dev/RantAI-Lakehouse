//! A Query Studio statement may name one setting, `iceberg_snapshot_id`
//! (`SEC-21`).
//!
//! The run caps a `ClickHouse` result with request-level settings, and a
//! statement's own `SETTINGS max_result_rows = 0` overrides them. The rule
//! (`routes::query::refuse_unlisted_settings`) refuses every other setting
//! on the statement that goes to the engine, in `run`, `estimate` and
//! `download`. The rule itself is unit-tested next to the function; these
//! tests prove each route calls it before the engine is asked anything. The
//! `ClickHouse` stand-in is a `wiremock` server wired through `CH_URL`, the
//! arrangement of `tests/query_estimate_read_only.rs` and
//! `tests/query_download.rs`, so every test asserts on exactly what the
//! engine was sent. The real-engine proof is in `tests/query_result_cap.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::queries::{RecordHistoryInput, record_history};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

/// What every refusal says, byte for byte.
const SETTINGS_REFUSAL: &str =
    "Query Studio accepts only iceberg_snapshot_id in SETTINGS; the server sets the others.";

/// A statement that would lift the run's cap.
const LIFTS_THE_CAP: &str = "SELECT n FROM serving.t SETTINGS max_result_rows = 0";

/// The version picker's output (`src/lib/snapshot-picker.ts`).
const PICKER_STATEMENT: &str = "SELECT n FROM serving.t\nSETTINGS iceberg_snapshot_id = 1";

async fn spin_up_with_clickhouse(ch_uri: &str) -> TestApp {
    let mut overrides = HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch_uri.to_owned());
    // The shared catalog belongs to one tenant, and the query path is
    // scoped to the tenant: this principal must be the catalog's own.
    overrides.insert(
        "CATALOG_TENANT_ID".to_owned(),
        "11111111-1111-4111-8111-000000000001".to_owned(),
    );
    spin_up_with_env(&overrides).await
}

/// Answers the column lookup the rewrite prefetches with an empty table, a
/// download (its statement ends in `FORMAT CSV`) with a small CSV, and any
/// other statement (a run, the `EXPLAIN ESTIMATE`) with one `JSON` row.
async fn mount_engine(ch: &MockServer) {
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
        .and(body_string_contains("FORMAT CSV"))
        .respond_with(ResponseTemplate::new(200).set_body_string("n\n1\n"))
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

/// The statements the engine received, without the rewrite's own `system.`
/// lookups.
async fn sent_statements(ch: &MockServer) -> Vec<String> {
    ch.received_requests()
        .await
        .expect("mock records requests")
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .filter(|b| !b.contains("system."))
        .collect()
}

async fn post_sql(
    router: &axum::Router,
    cookie: &str,
    uri: &str,
    sql: &str,
) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(json!({ "sql": sql }).to_string()))
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

async fn insert_history_row(pool: &PgPool, id: &str, sql: &str) {
    let owner: Uuid = sqlx::query_scalar("SELECT id FROM app_user WHERE email = $1")
        .bind("rina@meridian.example")
        .fetch_one(pool)
        .await
        .expect("seeded user rina@meridian.example (0002_seed_identity.sql)");
    let input = RecordHistoryInput {
        id,
        sql,
        user: &owner.to_string(),
        owner_id: Some(owner),
        status: "completed",
        duration_ms: 12,
        scanned_bytes: 34,
        cost_units: 1.0,
        workload_class: "hot-analytics",
        engine: "clickhouse",
        cache_assisted: false,
    };
    record_history(pool, &input)
        .await
        .expect("insert a query_history fixture row");
}

async fn download_csv(router: &axum::Router, cookie: &str, id: &str) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/query/run/{id}/download?format=csv"))
                .header("cookie", cookie)
                .body(Body::empty())
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

#[tokio::test]
async fn a_run_with_another_setting_is_refused_and_the_engine_is_sent_no_statement() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let (status, body) = post_sql(&router, &cookie, "/api/query/run", LIFTS_THE_CAP).await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"], SETTINGS_REFUSAL, "{body}");
    let sent = sent_statements(&ch).await;
    assert!(sent.is_empty(), "the rule precedes the run: {sent:?}");
}

#[tokio::test]
async fn a_run_with_the_pickers_setting_reaches_the_engine() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let (status, body) = post_sql(&router, &cookie, "/api/query/run", PICKER_STATEMENT).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let sent = sent_statements(&ch).await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].contains("iceberg_snapshot_id = 1"), "{}", sent[0]);
}

#[tokio::test]
async fn an_estimate_with_another_setting_is_refused_and_the_engine_is_sent_no_statement() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let (status, body) = post_sql(&router, &cookie, "/api/query/estimate", LIFTS_THE_CAP).await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"], SETTINGS_REFUSAL, "{body}");
    let sent = sent_statements(&ch).await;
    assert!(sent.is_empty(), "the rule precedes the explain: {sent:?}");
}

#[tokio::test]
async fn an_estimate_with_the_pickers_setting_reaches_explain() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let (status, body) = post_sql(&router, &cookie, "/api/query/estimate", PICKER_STATEMENT).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let explained: Vec<String> = sent_statements(&ch)
        .await
        .into_iter()
        .filter(|s| s.starts_with("EXPLAIN ESTIMATE"))
        .collect();
    assert_eq!(explained.len(), 1, "{explained:?}");
}

#[tokio::test]
async fn a_download_of_a_row_with_another_setting_is_refused_and_the_engine_is_sent_no_statement() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    // A row `run` would no longer record, but one written before the rule, or
    // by another writer, must not be re-run with its own cap-lifting setting.
    insert_history_row(&pool, "q-100", LIFTS_THE_CAP).await;

    let (status, body) = download_csv(&router, &cookie, "q-100").await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"], SETTINGS_REFUSAL, "{body}");
    let sent = sent_statements(&ch).await;
    assert!(sent.is_empty(), "the rule precedes the re-run: {sent:?}");
}

#[tokio::test]
async fn a_download_keeps_its_own_cap_after_the_check() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    insert_history_row(&pool, "q-101", PICKER_STATEMENT).await;

    let (status, body) = download_csv(&router, &cookie, "q-101").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let sent = sent_statements(&ch).await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].contains("iceberg_snapshot_id = 1"), "{}", sent[0]);
    assert!(
        sent[0].contains("SETTINGS max_result_rows =") && sent[0].contains("result_overflow_mode"),
        "the download's own SETTINGS is added after the check: {}",
        sent[0]
    );
}
