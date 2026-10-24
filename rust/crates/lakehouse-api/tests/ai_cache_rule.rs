//! `set_cache_rule` through `POST /api/ai/tool` (`BI-4` part B; review P-S6,
//! the `AI` parity acceptance "the rule is set on the board"): once
//! confirmed, the rule is what `ClickHouse` receives in `bi_board.cache_json`
//! for a board, and in the chart's stored `shape.cache` for a chart.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::Request;
use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

const BOARD: &str = "b_cache";

/// A `ClickHouse` with one board and one chart, which records every statement
/// it is sent (wiremock does).
struct Store;

impl Respond for Store {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body);
        let rows = |data: Vec<Value>| {
            let n = data.len();
            ResponseTemplate::new(200).set_body_json(json!({ "meta": [], "data": data, "rows": n }))
        };
        if sql.starts_with("INSERT") || sql.starts_with("CREATE") || sql.starts_with("ALTER") {
            return ResponseTemplate::new(200).set_body_string("");
        }
        if sql.contains("FROM system.tables") {
            return rows(vec![json!({ "n": "1" })]);
        }
        if sql.contains("FROM system.columns") {
            return rows(vec![
                json!({ "name": "region", "type": "String" }),
                json!({ "name": "n", "type": "Float64" }),
            ]);
        }
        if sql.contains("FROM console.bi_chart") {
            let spec = json!({
                "id": "c_one", "title": "Visits", "kind": "table", "mart": "mart_x",
                "sql": "SELECT 1", "x": "region", "y": "n"
            });
            let def = json!({
                "title": "Visits", "mart": "mart_x", "kind": "table",
                "dimension": "region", "measures": ["n"], "board": BOARD,
            });
            return rows(vec![json!({
                "id": "c_one", "board": BOARD, "created_by": "ui", "created_at": "2026-01-01 00:00:00",
                "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
            })]);
        }
        if sql.contains("FROM console.bi_board") {
            return rows(vec![json!({
                "id": BOARD, "name": "Cache board", "description": "", "created_by": "t",
                "layout_json": "", "filters_json": "", "public_token": "", "embed_enabled": "0",
                "folder_id": "", "created_at": "2026-10-10 00:00:00", "owner_id": "",
                "is_deleted": "0", "cache_json": "",
            })]);
        }
        ResponseTemplate::new(200).set_body_string("")
    }
}

async fn app() -> (TestApp, MockServer, String) {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(Store)
        .mount(&ch)
        .await;
    let env: HashMap<String, String> = HashMap::from([("CH_URL".to_owned(), ch.uri())]);
    let app = spin_up_with_env(&env).await;
    let user =
        create_principal_with_permissions(&app.pool, "dashboard:read, dashboard:write").await;
    let cookie = session_cookie_for_user(&app.pool, user).await;
    (app, ch, cookie)
}

async fn call(app: &TestApp, cookie: &str, args: Value) -> Value {
    let resp = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/tool")
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(
                    json!({ "tool": "set_cache_rule", "args": args, "mode": "build" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn inserts_into(ch: &MockServer, table: &str) -> Vec<String> {
    ch.received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .filter(|sql| sql.starts_with("INSERT") && sql.contains(table))
        .collect()
}

#[tokio::test]
async fn a_confirmed_board_rule_is_written_to_the_boards_cache_column() {
    let (app, ch, cookie) = app().await;
    let body = call(
        &app,
        &cookie,
        json!({ "board": BOARD, "rule": "minutes", "minutes": 30, "confirmed": true }),
    )
    .await;
    assert_eq!(body["outcome"], "executed", "{body}");
    assert_eq!(body["result"]["updated"], true, "{body}");
    let writes = inserts_into(&ch, "console.bi_board").await;
    assert!(
        writes
            .iter()
            .any(|sql| sql.contains(r#""mode":"minutes""#) && sql.contains(r#""minutes":30"#)),
        "the board row was not written with the rule: {writes:?}"
    );
}

#[tokio::test]
async fn a_confirmed_chart_rule_is_written_to_the_charts_shape() {
    let (app, ch, cookie) = app().await;
    let body = call(
        &app,
        &cookie,
        json!({ "chart": "c_one", "rule": "fresh", "confirmed": true }),
    )
    .await;
    assert_eq!(body["outcome"], "executed", "{body}");
    assert_eq!(body["result"]["updated"], true, "{body}");
    let writes = inserts_into(&ch, "console.bi_chart").await;
    assert!(
        writes
            .iter()
            .any(|sql| sql.contains(r#""cache":{"mode":"fresh"}"#)),
        "the chart row was not written with the rule: {writes:?}"
    );
}
