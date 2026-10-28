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

/// What the browser is handed: the install's Query Studio row limit
/// (`QueryLimits::studio_rows`), which migration `0115` set to 64,000.
const ROWS_KEPT: usize = 64_000;

/// The table the huge result is read from. A table function such as
/// `numbers(..)` would be refused for a policy-governed caller, which would
/// make the tests below pass without ever reaching the cap.
const HUGE_TABLE: &str = "serving.query_result_cap";

/// A result 5,000 times the cap. Unbounded, it is about 500 MB of `JSON`.
const HUGE_SELECT: &str = "SELECT n FROM serving.query_result_cap";

/// The server-side byte cap of one result: 10 MB (10 * 1024 * 1024 bytes) of
/// cell text (`QS-2`).
/// The route keeps this number in `routes::query`, where it is private to
/// the module, so it is repeated here and pinned exactly by
/// [`a_result_over_the_byte_cap_is_cut_by_bytes`].
const BYTE_CAP: usize = 10 * 1024 * 1024;

/// The text of one cell of [`WIDE_SELECT`]: 2,100 bytes, plus the one byte
/// the cost rule charges for every cell (the separator), so 2,101 bytes a
/// row.
const WIDE_CELL_TEXT: usize = 2_100;

/// The rows of [`WIDE_SELECT`]: under the row limit (64,000) and over the
/// byte cap (5,000 rows of 2,101 bytes is 10,505,000 bytes).
const WIDE_ROWS: usize = 5_000;

/// A result that passes the row limit and fails the byte cap: one column
/// whose cell carries 2,100 bytes of text, 5,000 rows.
const WIDE_SELECT: &str =
    "SELECT repeat('x', 2100) AS pad FROM serving.query_result_cap LIMIT 5000";

/// A result the row limit cuts and the byte cap does not: one column whose
/// cell carries 162 bytes of text, and no `LIMIT`, so the engine sends one
/// block more than the row limit.
const WIDE_ROW_LIMIT_SELECT: &str = "SELECT repeat('x', 162) AS pad FROM serving.query_result_cap";

/// The most rows the engine may send. `break` stops at a block boundary, so
/// the engine can send up to one block (`max_block_size`, about 65,000 rows
/// by default) beyond the cap; 130,000 is that bound with room (64,001 plus
/// one block of 65,536 is 129,537), and still far below the 10,000,000 rows
/// the statement produces.
const ENGINE_ROW_BOUND: usize = 130_000;

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
    overrides.insert(
        "CATALOG_TENANT_ID".to_owned(),
        "11111111-1111-4111-8111-000000000001".to_owned(),
    );
    let app = spin_up_with_env(&overrides).await;
    let ch = ChClient::new(ch_url.clone(), "default".to_owned(), String::new());
    (app, ch)
}

async fn run_as_reader(app: &TestApp, sql: &str) -> (StatusCode, Value) {
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

/// The byte cost of one cell, by the server's rule (`QS-2`): the
/// length of its text — a JSON string's own bytes without the quotes, any
/// other value's compact JSON text — plus one byte for the separator.
fn cell_bytes(cell: &Value) -> usize {
    match cell {
        Value::String(text) => text.len() + 1,
        other => other.to_string().len() + 1,
    }
}

/// The byte cost of one row: the sum of its cells' costs.
fn row_bytes(row: &Value) -> usize {
    match row {
        Value::Object(cells) => cells.values().map(cell_bytes).sum(),
        Value::Array(cells) => cells.iter().map(cell_bytes).sum(),
        other => cell_bytes(other),
    }
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
async fn a_run_over_a_huge_table_keeps_64000_rows_and_the_engine_sent_few() {
    let (app, _ch) = spin_up_with_clickhouse().await;

    let (status, body) = run_as_reader(&app, HUGE_SELECT).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["rows"].as_array().expect("rows").len(), ROWS_KEPT);
    assert_eq!(body["truncated"], json!(true), "{body}");
    // The row limit is the limit that cut this one: the cells are short
    // (a number), so the result weighs about half a megabyte and never
    // reaches the byte cap.
    assert_eq!(
        body["cutBy"],
        json!("rows"),
        "cutBy must name the row limit"
    );
    // `rowCount` is what came back from the engine before the route's own
    // truncate, so it shows where the cap took effect.
    let engine_rows = usize::try_from(body["rowCount"].as_u64().expect("rowCount")).unwrap();
    assert!(
        (ROWS_KEPT + 1..ENGINE_ROW_BOUND).contains(&engine_rows),
        "the engine sent {engine_rows} rows of 10,000,000"
    );
}

#[tokio::test]
async fn a_result_over_the_byte_cap_is_cut_by_bytes() {
    let (app, _ch) = spin_up_with_clickhouse().await;

    let (status, body) = run_as_reader(&app, WIDE_SELECT).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["cutBy"],
        json!("bytes"),
        "cutBy must name the byte cap"
    );
    assert_eq!(body["truncated"], json!(true), "a cut result is truncated");
    // `rowCount` is the engine's own count and does not change with the byte
    // cut, so the console cannot read it as "the rows you see".
    assert_eq!(
        body["rowCount"],
        json!(WIDE_ROWS),
        "rowCount is the engine's count"
    );

    let rows = body["rows"].as_array().expect("rows");
    // 5,000 rows of 2,101 bytes is 10,505,000 bytes, over the 10 MB cap.
    // The cap cuts at the last whole row that fits: 4,990 of them.
    assert_eq!(
        rows.len(),
        4_990,
        "the cap cuts at the last whole row that fits"
    );
    let kept_bytes: usize = rows.iter().map(row_bytes).sum();
    assert!(
        kept_bytes <= BYTE_CAP,
        "the kept rows weigh {kept_bytes} bytes, over the {BYTE_CAP}-byte cap"
    );
    assert!(
        kept_bytes + WIDE_CELL_TEXT + 1 > BYTE_CAP,
        "one more row would have fit, so the cut was not at the last whole row"
    );
}

#[tokio::test]
async fn a_result_the_row_limit_cut_says_rows_and_not_bytes() {
    let (app, _ch) = spin_up_with_clickhouse().await;

    let (status, body) = run_as_reader(&app, WIDE_ROW_LIMIT_SELECT).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    // The rows that are sent weigh 64,000 x 163 bytes — 10,432,000 bytes,
    // under the 10 MB cap — so the byte cap did not cut this result and the
    // limit that did is the row limit. The engine's whole result weighs more
    // than the cap, and it does not matter: the cap counts the rows that are
    // sent, which is why it runs after the row cut.
    let rows = body["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), ROWS_KEPT, "the row limit cut this result");
    let kept_bytes: usize = rows.iter().map(row_bytes).sum();
    assert!(
        kept_bytes <= BYTE_CAP,
        "the sent rows weigh {kept_bytes} bytes, over the {BYTE_CAP}-byte cap"
    );
    assert_eq!(
        body["cutBy"],
        json!("rows"),
        "cutBy must name the row limit"
    );
    assert_eq!(body["truncated"], json!(true), "a cut result is truncated");
}

#[tokio::test]
async fn a_result_under_both_caps_carries_no_cut_by() {
    let (app, _ch) = spin_up_with_clickhouse().await;

    let (status, body) =
        run_as_reader(&app, "SELECT n FROM serving.query_result_cap LIMIT 100").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["rows"].as_array().expect("rows").len(), 100);
    assert_eq!(body["truncated"], json!(false), "nothing was cut");
    // The key is absent when nothing was cut — not `null`, which would claim
    // a cut that did not happen.
    assert!(body.get("cutBy").is_none(), "no cut means no cutBy");
}

#[tokio::test]
async fn a_result_of_exactly_the_cap_is_not_reported_as_truncated() {
    let (app, _ch) = spin_up_with_clickhouse().await;

    let (status, body) =
        run_as_reader(&app, "SELECT n FROM serving.query_result_cap LIMIT 64000").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["rows"].as_array().expect("rows").len(), ROWS_KEPT);
    assert_eq!(body["truncated"], json!(false), "{body}");

    let (status, body) =
        run_as_reader(&app, "SELECT n FROM serving.query_result_cap LIMIT 64001").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["rows"].as_array().expect("rows").len(), ROWS_KEPT);
    assert_eq!(body["truncated"], json!(true), "{body}");
}

/// The refusal `routes::query::refuse_unlisted_settings` gives.
const SETTINGS_REFUSAL: &str =
    "Query Studio accepts only iceberg_snapshot_id in SETTINGS; the server sets the others.";

/// How many statements the engine logged whose text contains `needle`,
/// not counting the lookup itself. `system.query_log` is written in the
/// background, so it is flushed first.
async fn logged_statements_containing(ch: &ChClient, needle: &str) -> u64 {
    ch.exec("SYSTEM FLUSH LOGS", None)
        .await
        .expect("flush the query log");
    let result = ch
        .query(
            &format!(
                "SELECT count() AS c FROM system.query_log \
                 WHERE query LIKE '%{needle}%' AND query NOT LIKE '%system.query_log%'"
            ),
            None,
        )
        .await
        .expect("read the query log");
    let count = &result.data[0]["c"];
    // 64-bit integers come back as strings in `FORMAT JSON`.
    count
        .as_u64()
        .or_else(|| count.as_str().and_then(|s| s.parse().ok()))
        .expect("count() is a number")
}

#[tokio::test]
async fn a_statement_cannot_lift_the_cap_with_its_own_settings() {
    let (app, ch) = spin_up_with_clickhouse().await;
    // `max_result_rows = 0` in the statement overrides the cap the run sends
    // with the request: measured on `ClickHouse` 26.8, 500,000 of 500,000
    // rows arrived. The rule refuses it before the engine sees it.
    let statement = format!("{HUGE_SELECT} LIMIT 500000 SETTINGS max_result_rows = 0");

    let (status, body) = run_as_reader(&app, &statement).await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"], SETTINGS_REFUSAL, "{body}");
    assert_eq!(
        logged_statements_containing(&ch, "LIMIT 500000 SETTINGS max_result_rows = 0").await,
        0,
        "the engine ran no statement that carries the setting"
    );
}

#[tokio::test]
async fn a_statement_with_the_pickers_setting_is_still_capped_and_runs() {
    let (app, _ch) = spin_up_with_clickhouse().await;
    // `iceberg_snapshot_id` names no Iceberg table here, so it changes
    // nothing; this shows the rule lets the picker's statement through to a
    // real engine, which accepts the setting by name.
    let statement = format!("{HUGE_SELECT}\nSETTINGS iceberg_snapshot_id = 1");

    let (status, body) = run_as_reader(&app, &statement).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["rows"].as_array().expect("rows").len(), ROWS_KEPT);
    assert_eq!(body["truncated"], json!(true), "{body}");
}
