//! `POST /api/query/run` caps a `ClickHouse` result inside the engine
//! (SEC-21).
//!
//! The route keeps 2,000 rows for the browser, but it used to receive the
//! engine's whole result first, so one `SELECT *` on a large table filled
//! the API's memory before `truncate` ran. These tests need the real engine,
//! because the cap is a server-side setting (`max_result_rows` with
//! `result_overflow_mode = 'break'`) that only a real `ClickHouse` enforces:
//! a `wiremock` stand-in would only prove the request carries the settings,
//! which the `lakehouse-clickhouse` unit test already does. The arrangement
//! is that of `tests/gold_publication.rs`: one disposable `ClickHouse` per
//! test binary, a per-test Postgres from `common::spin_up_with_env`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_clickhouse::ChClient;
use serde_json::{Value, json};
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::clickhouse::ClickHouse;
use tower::ServiceExt;

use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};

/// What the browser is handed: `routes::query::MAX_RESULT_ROWS`.
const ROWS_KEPT: usize = 2_000;

/// The table the huge result is read from. A table function such as
/// `numbers(..)` would be refused for a policy-governed caller, which would
/// make the tests below pass without ever reaching the cap.
const HUGE_TABLE: &str = "serving.query_result_cap";

/// A result 5,000 times the cap. Unbounded, it is about 500 MB of `JSON`.
const HUGE_SELECT: &str = "SELECT n FROM serving.query_result_cap";

/// The most rows the engine may send. `break` stops at a block boundary, so
/// the engine can send up to one block (`max_block_size`, about 65,000 rows
/// by default) beyond the cap; 100,000 is that bound with room, and still 100 times below the
/// 10,000,000 rows the statement produces.
const ENGINE_ROW_BOUND: usize = 100_000;

/// ONE `ClickHouse` per test binary; see `tests/gold_publication.rs` for why
/// per-test containers race. The container is never stopped by the test.
static CLICKHOUSE: tokio::sync::OnceCell<(String, testcontainers::ContainerAsync<ClickHouse>)> =
    tokio::sync::OnceCell::const_new();

async fn shared_clickhouse() -> &'static (String, testcontainers::ContainerAsync<ClickHouse>) {
    CLICKHOUSE
        .get_or_init(|| async {
            // The compose service's env trio; all three are required, see
            // `tests/gold_publication.rs`.
            let container = ClickHouse::default()
                .with_env_var("CLICKHOUSE_USER", "default")
                .with_env_var("CLICKHOUSE_PASSWORD", "")
                .with_env_var("CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT", "1")
                .with_tag("26.8")
                .start()
                .await
                .expect("start the ClickHouse testcontainer");
            let host = container.get_host().await.expect("container host");
            let port = container
                .get_host_port_ipv4(8123)
                .await
                .expect("container HTTP port");
            let url = format!("http://{host}:{port}");
            seed_huge_table(&ChClient::new(
                url.clone(),
                "default".to_owned(),
                String::new(),
            ))
            .await;
            (url, container)
        })
        .await
}

/// Fills `HUGE_TABLE` with 10,000,000 rows, once, when the container starts.
async fn seed_huge_table(ch: &ChClient) {
    ch.exec("CREATE DATABASE IF NOT EXISTS serving", None)
        .await
        .expect("create the serving database");
    ch.exec(
        &format!("CREATE TABLE {HUGE_TABLE} (n UInt64) ENGINE = MergeTree ORDER BY n"),
        None,
    )
    .await
    .expect("create the table");
    ch.exec(
        &format!("INSERT INTO {HUGE_TABLE} SELECT number FROM numbers(10000000)"),
        None,
    )
    .await
    .expect("fill the table");
}

async fn spin_up_with_clickhouse() -> (TestApp, ChClient) {
    let (ch_url, _container) = shared_clickhouse().await;
    let mut overrides = HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch_url.clone());
    let app = spin_up_with_env(&overrides).await;
    let ch = ChClient::new(ch_url.clone(), "default".to_owned(), String::new());
    (app, ch)
}

async fn run_as_reader(app: &TestApp, sql: &str) -> (StatusCode, Value) {
    let user = create_principal_with_permissions(&app.pool, "query:read").await;
    let cookie = session_cookie_for_user(&app.pool, user).await;
    let resp = app
        .router
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
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, body)
}

#[tokio::test]
async fn the_engine_stops_producing_rows_at_the_cap() {
    let (_app, ch) = spin_up_with_clickhouse().await;

    let result = ch
        .query_limited("SELECT number FROM numbers(10000000)", ROWS_KEPT + 1, None)
        .await
        .expect("the capped query runs");

    assert!(
        result.data.len() > ROWS_KEPT,
        "one row more than kept arrives, so the route can tell it truncated: {}",
        result.data.len()
    );
    assert!(
        result.data.len() < ENGINE_ROW_BOUND,
        "the engine sent {} rows of 10,000,000",
        result.data.len()
    );
}

#[tokio::test]
async fn a_run_over_a_huge_table_keeps_2000_rows_and_the_engine_sent_few() {
    let (app, _ch) = spin_up_with_clickhouse().await;

    let (status, body) = run_as_reader(&app, HUGE_SELECT).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["rows"].as_array().expect("rows").len(), ROWS_KEPT);
    assert_eq!(body["truncated"], json!(true), "{body}");
    // `rowCount` is what came back from the engine before the route's own
    // truncate, so it shows where the cap took effect.
    let engine_rows = usize::try_from(body["rowCount"].as_u64().expect("rowCount")).unwrap();
    assert!(
        (ROWS_KEPT + 1..ENGINE_ROW_BOUND).contains(&engine_rows),
        "the engine sent {engine_rows} rows of 10,000,000"
    );
}

#[tokio::test]
async fn a_result_of_exactly_the_cap_is_not_reported_as_truncated() {
    let (app, _ch) = spin_up_with_clickhouse().await;

    let (status, body) =
        run_as_reader(&app, "SELECT n FROM serving.query_result_cap LIMIT 2000").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["rows"].as_array().expect("rows").len(), ROWS_KEPT);
    assert_eq!(body["truncated"], json!(false), "{body}");

    let (status, body) =
        run_as_reader(&app, "SELECT n FROM serving.query_result_cap LIMIT 2001").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["rows"].as_array().expect("rows").len(), ROWS_KEPT);
    assert_eq!(body["truncated"], json!(true), "{body}");
}
