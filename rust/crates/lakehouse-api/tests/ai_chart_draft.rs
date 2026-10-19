//! A chart draft that cannot be built is not offered (plan 18, step 3).
//!
//! A `create_chart` without `confirmed` used to come back as a
//! `needs_confirmation` draft before anything checked it. Now the same
//! validation the Confirm path runs is run on the draft, and a draft that
//! fails is refused. The check writes nothing, so `ClickHouse` must never see
//! an `INSERT`.
//!
//! The model is mocked round by round, as in `ai_sql_help.rs`, and
//! `ClickHouse` is mocked by statement: a count of `serving` tables, the
//! `serving` columns of the mart, and the saved fields of the chart.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

/// The one mart the mock lists in `serving`, its dimension and its measure.
const MART: &str = "mart_seeded_sales";
const COLUMN: &str = "region_code";
const MEASURE: &str = "sale_total";

/// One model round: the given text and tool calls, as an OpenAI-compatible
/// `/chat/completions` response.
fn model_round(content: Option<&str>, calls: &[(&str, &str, &str)]) -> Value {
    let tool_calls: Vec<Value> = calls
        .iter()
        .map(|(id, name, arguments)| {
            json!({
                "id": id,
                "type": "function",
                "function": { "name": name, "arguments": arguments },
            })
        })
        .collect();
    json!({
        "choices": [{ "message": {
            "role": "assistant",
            "content": content,
            "tool_calls": if tool_calls.is_empty() { Value::Null } else { json!(tool_calls) },
        } }],
    })
}

/// Mounts `rounds` in order on the model, then a fallback answer that ends
/// the turn if the rounds run out.
async fn mock_model(rounds: Vec<Value>) -> MockServer {
    let llm = MockServer::start().await;
    for round in rounds {
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(round))
            .up_to_n_times(1)
            .mount(&llm)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(model_round(
            Some("Fallback answer, reached only when the turn did not end."),
            &[],
        )))
        .mount(&llm)
        .await;
    llm
}

/// A `ClickHouse` answer in the shape `ChClient::query` parses.
fn rows(data: &Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "meta": [],
        "data": data,
        "rows": data.as_array().map_or(0, Vec::len),
    }))
}

/// `ClickHouse` for a chart draft. `mart_exists` decides whether the
/// `serving` count finds the mart; the columns query returns `COLUMN` and no
/// other column. Anything else (the saved fields, the `CREATE`/`ALTER` DDL)
/// gets an empty answer, and an `INSERT` is recorded by `statements`.
async fn mock_clickhouse(mart_exists: bool) -> MockServer {
    let ch = MockServer::start().await;
    let count = if mart_exists { "1" } else { "0" };
    // `validated_mart_columns` asks `system.tables` by name first.
    Mock::given(method("POST"))
        .and(body_string_contains("FROM system.tables"))
        .respond_with(rows(&json!([{ "n": count }])))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("FROM system.columns"))
        .respond_with(rows(&json!([
            { "name": COLUMN, "type": "String" },
            { "name": MEASURE, "type": "Float64" },
        ])))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&ch)
        .await;
    ch
}

/// Every statement `ClickHouse` received, as text.
async fn statements(ch: &MockServer) -> Vec<String> {
    ch.received_requests()
        .await
        .expect("recording is on")
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect()
}

/// A `create_chart` argument string for a table over `dimension` and
/// `MEASURE`, with no `confirmed`, so the gate answers with a draft.
fn chart_args(mart: &str, dimension: &str) -> String {
    json!({
        "title": "Sales by region",
        "kind": "table",
        "mart": mart,
        "dimension": dimension,
        "measures": [MEASURE],
    })
    .to_string()
}

/// `chart_args` with one extra top-level key merged in.
fn chart_args_with(dimension: &str, key: &str, value: Value) -> String {
    let mut args: Value = serde_json::from_str(&chart_args(MART, dimension)).unwrap();
    args[key] = value;
    args.to_string()
}

fn env_for(llm: &MockServer, ch: &MockServer) -> std::collections::HashMap<String, String> {
    let mut env = std::collections::HashMap::new();
    env.insert("LLM_URL".to_owned(), llm.uri());
    env.insert("CH_URL".to_owned(), ch.uri());
    env
}

/// Calls `POST /api/ai/tool` once in build mode and returns the status and
/// the body.
async fn tool_call(
    llm: &MockServer,
    ch: &MockServer,
    tool: &str,
    args: &str,
) -> (StatusCode, Value) {
    let TestApp { router, pool } = spin_up_with_env(&env_for(llm, ch)).await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let body = json!({
        "tool": tool,
        "args": serde_json::from_str::<Value>(args).unwrap(),
        "mode": "build",
    });
    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/tool")
                .header("content-type", "application/json")
                .header("cookie", &cookie)
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// Sends one build-mode chat turn and returns the status and the body.
async fn chat(llm: &MockServer, ch: &MockServer, question: &str) -> (StatusCode, Value) {
    let TestApp { router, pool } = spin_up_with_env(&env_for(llm, ch)).await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let body = json!({
        "mode": "build",
        "messages": [{ "role": "user", "content": question }],
    });
    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/chat")
                .header("content-type", "application/json")
                .header("cookie", &cookie)
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn has_insert(sent: &[String]) -> bool {
    sent.iter().any(|s| s.contains("INSERT"))
}

#[tokio::test]
async fn a_draft_on_a_mart_that_does_not_exist_is_refused_and_not_offered() {
    let llm = mock_model(vec![]).await;
    let ch = mock_clickhouse(false).await;

    let (status, body) = tool_call(&llm, &ch, "create_chart", &chart_args(MART, COLUMN)).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["result"]["error"].is_string(), "{body}");
    assert!(
        body["result"].get("needs_confirmation").is_none(),
        "a refused draft is not offered: {body}"
    );
    assert_eq!(body["outcome"], "failed", "{body}");
    assert!(!has_insert(&statements(&ch).await), "nothing was written");
}

#[tokio::test]
async fn a_draft_with_a_column_the_mart_does_not_have_is_refused_and_not_offered() {
    let llm = mock_model(vec![]).await;
    let ch = mock_clickhouse(true).await;

    let (status, body) = tool_call(
        &llm,
        &ch,
        "create_chart",
        &chart_args(MART, "no_such_column"),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["result"]["error"].is_string(), "{body}");
    assert!(
        body["result"].get("needs_confirmation").is_none(),
        "a refused draft is not offered: {body}"
    );
    assert_eq!(body["outcome"], "failed", "{body}");
    assert!(!has_insert(&statements(&ch).await), "nothing was written");
}

#[tokio::test]
async fn a_valid_draft_is_still_offered_as_a_draft_and_writes_nothing() {
    let llm = mock_model(vec![]).await;
    let ch = mock_clickhouse(true).await;

    let (status, body) = tool_call(&llm, &ch, "create_chart", &chart_args(MART, COLUMN)).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["outcome"], "needs_confirmation", "{body}");
    assert_eq!(body["result"]["needs_confirmation"], json!(true), "{body}");
    assert!(
        !has_insert(&statements(&ch).await),
        "a draft writes nothing"
    );
}

#[tokio::test]
async fn a_valid_draft_that_carries_confirmed_false_is_still_a_draft_and_writes_nothing() {
    let llm = mock_model(vec![]).await;
    let ch = mock_clickhouse(true).await;
    let args = chart_args_with(COLUMN, "confirmed", json!(false));

    let (status, body) = tool_call(&llm, &ch, "create_chart", &args).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["result"]["error"].is_null(), "{body}");
    assert_eq!(body["outcome"], "needs_confirmation", "{body}");
    assert_eq!(body["result"]["needs_confirmation"], json!(true), "{body}");
    assert!(
        !has_insert(&statements(&ch).await),
        "a draft writes nothing"
    );
}

#[tokio::test]
async fn the_chat_loop_refuses_a_draft_on_a_mart_that_does_not_exist() {
    let llm = mock_model(vec![
        model_round(
            None,
            &[("call_1", "create_chart", &chart_args(MART, COLUMN))],
        ),
        model_round(Some("The mart is not there."), &[]),
    ])
    .await;
    let ch = mock_clickhouse(false).await;

    let (status, body) = chat(&llm, &ch, "Buat grafik penjualan per wilayah.").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let trace = body["toolTrace"].as_array().expect("trace is a list");
    let step = trace
        .iter()
        .find(|s| s["tool"] == "create_chart")
        .expect("the create_chart step is traced");
    assert_eq!(step["ok"], false, "{trace:?}");
    assert!(
        step["result"].get("needs_confirmation").is_none(),
        "{trace:?}"
    );
    assert!(!has_insert(&statements(&ch).await), "nothing was written");
}

/// The step of a chat body's tool trace that ran `tool`.
fn traced_step<'a>(body: &'a Value, tool: &str) -> &'a Value {
    body["toolTrace"]
        .as_array()
        .expect("trace is a list")
        .iter()
        .find(|s| s["tool"] == tool)
        .unwrap_or_else(|| panic!("the {tool} step is traced: {body}"))
}

#[tokio::test]
async fn the_chat_loop_does_not_run_create_chart_on_the_models_own_confirmed_true() {
    let args = chart_args_with(COLUMN, "confirmed", json!(true));
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "create_chart", &args)]),
        model_round(Some("The chart waits for the Confirm button."), &[]),
    ])
    .await;
    let ch = mock_clickhouse(true).await;

    let (status, body) = chat(&llm, &ch, "Make a table chart of the mart.").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let step = traced_step(&body, "create_chart");
    assert_eq!(step["result"]["needs_confirmation"], json!(true), "{body}");
    assert!(
        !has_insert(&statements(&ch).await),
        "the model's own confirmation wrote a chart"
    );
}

#[tokio::test]
async fn the_chat_loop_does_not_run_create_board_on_the_models_own_confirmed_true() {
    let args = json!({ "name": "Overview", "confirmed": true }).to_string();
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "create_board", &args)]),
        model_round(Some("The board waits for the Confirm button."), &[]),
    ])
    .await;
    let ch = mock_clickhouse(true).await;

    let (status, body) = chat(&llm, &ch, "Create a board called Overview.").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let step = traced_step(&body, "create_board");
    assert_eq!(step["result"]["needs_confirmation"], json!(true), "{body}");
    assert!(
        !has_insert(&statements(&ch).await),
        "the model's own confirmation wrote a board"
    );
}

/// A `confirmed` flag of any value is removed from the offered draft's args,
/// so the Confirm button resends a clean call. The model sends `false` here:
/// the gate offers the draft either way, so only the removal can fail this.
#[tokio::test]
async fn the_chat_loop_removes_a_confirmed_flag_of_any_value_from_the_offered_draft() {
    let args = chart_args_with(COLUMN, "confirmed", json!(false));
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "create_chart", &args)]),
        model_round(Some("The chart waits for the Confirm button."), &[]),
    ])
    .await;
    let ch = mock_clickhouse(true).await;

    let (status, body) = chat(&llm, &ch, "Make a table chart of the mart.").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let step = traced_step(&body, "create_chart");
    assert_eq!(step["result"]["needs_confirmation"], json!(true), "{body}");
    assert!(
        step["result"]["args"].get("confirmed").is_none(),
        "the Confirm button would resend the model's flag: {body}"
    );
    assert!(
        step["args"].get("confirmed").is_none(),
        "the traced args keep the model's flag: {body}"
    );
    assert_eq!(step["result"]["args"]["mart"], MART, "{body}");
    assert!(!has_insert(&statements(&ch).await), "nothing was written");
}
