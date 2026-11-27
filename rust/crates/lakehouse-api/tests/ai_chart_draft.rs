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

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{
    CLICKHOUSE_UNKNOWN_IDENTIFIER, COLUMN_DOES_NOT_EXIST, TestApp, create_assistant_principal,
    mock_clickhouse_refusing, session_cookie_for_seeded_user, session_cookie_for_user,
    spin_up_with_env,
};

/// The one mart the mock lists in `serving`, its dimension and its measure.
const MART: &str = "mart_seeded_sales";
const COLUMN: &str = "region_code";
const MEASURE: &str = "sale_total";
/// A second numeric column, so a binned numeric dimension is not also the measure.
const MEASURE_TWO: &str = "sale_count";
/// A second text column, for a breakdown.
const SECOND_COLUMN: &str = "sale_month";

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
/// `serving` count finds the mart; the columns query returns the four columns above and no
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
            { "name": SECOND_COLUMN, "type": "String" },
            { "name": MEASURE, "type": "Float64" },
            { "name": MEASURE_TWO, "type": "Float64" },
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

/// Posts `body` to `uri` as the seeded user `fajar@meridian.example` (an
/// administrator, who sees every personal space), or, with `permissions`, as
/// a new person who holds those permissions besides `assistant:use` (every
/// request here is to `/api/ai/*`). Returns the status and the body.
async fn post_as(
    llm: &MockServer,
    ch: &MockServer,
    uri: &str,
    body: &Value,
    permissions: Option<&str>,
) -> (StatusCode, Value) {
    let TestApp { router, pool } = spin_up_with_env(&env_for(llm, ch)).await;
    let cookie = match permissions {
        None => session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await,
        Some(permissions) => {
            let user = create_assistant_principal(&pool, permissions).await;
            session_cookie_for_user(&pool, user).await
        }
    };
    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .header("cookie", &cookie)
                .body(Body::from(serde_json::to_vec(body).unwrap()))
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

/// The body of a build-mode `POST /api/ai/tool`.
fn tool_body(tool: &str, args: &str) -> Value {
    json!({
        "tool": tool,
        "args": serde_json::from_str::<Value>(args).unwrap(),
        "mode": "build",
    })
}

/// The body of a build-mode `POST /api/ai/chat` turn.
fn chat_body(question: &str) -> Value {
    json!({
        "mode": "build",
        "messages": [{ "role": "user", "content": question }],
    })
}

/// Calls `POST /api/ai/tool` once in build mode and returns the status and
/// the body.
async fn tool_call(
    llm: &MockServer,
    ch: &MockServer,
    tool: &str,
    args: &str,
) -> (StatusCode, Value) {
    post_as(llm, ch, "/api/ai/tool", &tool_body(tool, args), None).await
}

/// Sends one build-mode chat turn and returns the status and the body.
async fn chat(llm: &MockServer, ch: &MockServer, question: &str) -> (StatusCode, Value) {
    post_as(llm, ch, "/api/ai/chat", &chat_body(question), None).await
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

/// A `create_chart` call for the top ten `COLUMN` values by `MEASURE`, split
/// by `SECOND_COLUMN`: the call the assistant should send for "top 10 regions
/// by month".
fn top_ten_args() -> String {
    json!({
        "title": "Sales by region and month",
        "kind": "bar",
        "mart": MART,
        "dimension": COLUMN,
        "breakdown": SECOND_COLUMN,
        "measures": [MEASURE],
        "sort": { "by": MEASURE, "dir": "desc" },
        "limit": 10,
    })
    .to_string()
}

/// The text of the tool message the model reads in the round after `call_id`:
/// the second request the model received.
async fn tool_message_the_model_read(llm: &MockServer, call_id: &str) -> String {
    let requests = llm.received_requests().await.expect("recording is on");
    let second: Value = serde_json::from_slice(&requests[1].body).unwrap();
    second["messages"]
        .as_array()
        .expect("messages is a list")
        .iter()
        .find(|m| m["tool_call_id"] == call_id)
        .and_then(|m| m["content"].as_str())
        .unwrap_or_else(|| panic!("no tool message for {call_id}: {second}"))
        .to_owned()
}

#[tokio::test]
async fn a_chart_draft_lists_its_dimension_breakdown_sort_and_limit_for_the_model() {
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "create_chart", &top_ten_args())]),
        model_round(Some("Drafted."), &[]),
    ])
    .await;
    let ch = mock_clickhouse(true).await;

    let (status, body) = chat(&llm, &ch, "Top 10 regions by month.").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let step = traced_step(&body, "create_chart");
    assert_eq!(step["result"]["needs_confirmation"], json!(true), "{body}");
    let definition = step["result"]["definition"].as_str().unwrap_or_default();
    for listed in [
        "dimension: region_code",
        "breakdown: sale_month",
        "measures: sale_total",
        "sort: sale_total desc",
        "limit: 10",
    ] {
        assert!(definition.contains(listed), "{listed} is missing: {body}");
    }
    let read = tool_message_the_model_read(&llm, "call_1").await;
    assert!(
        read.contains(definition),
        "the model's tool message does not carry the digest: {read}"
    );
}

#[tokio::test]
async fn a_chart_draft_with_a_breakdown_and_no_limit_reads_limit_none() {
    let args = json!({
        "title": "Sales by region and month",
        "kind": "bar",
        "mart": MART,
        "dimension": SECOND_COLUMN,
        "breakdown": COLUMN,
        "measures": [MEASURE],
    })
    .to_string();
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "create_chart", &args)]),
        model_round(Some("Drafted."), &[]),
    ])
    .await;
    let ch = mock_clickhouse(true).await;

    let (status, body) = chat(&llm, &ch, "Sales by month and region.").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let step = traced_step(&body, "create_chart");
    let definition = step["result"]["definition"].as_str().unwrap_or_default();
    assert!(definition.contains("limit: none"), "{body}");
}

#[tokio::test]
async fn the_saved_reply_after_confirm_carries_the_digest_the_draft_showed() {
    let drafted = tool_call(
        &mock_model(vec![]).await,
        &mock_clickhouse(true).await,
        "create_chart",
        &top_ten_args(),
    )
    .await;
    let draft_digest = drafted.1["result"]["definition"].clone();
    assert!(
        draft_digest
            .as_str()
            .is_some_and(|d| d.contains("sort: sale_total desc; limit: 10")),
        "{drafted:?}"
    );

    let llm = mock_model(vec![]).await;
    let ch = mock_clickhouse(true).await;
    let mut confirmed: Value = serde_json::from_str(&top_ten_args()).unwrap();
    confirmed["confirmed"] = json!(true);
    let (status, saved) = tool_call(&llm, &ch, "create_chart", &confirmed.to_string()).await;

    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["result"]["created"], json!(true), "{saved}");
    assert_eq!(saved["result"]["definition"], draft_digest, "{saved}");
}

/// The card is built from `summary` and the call's args; the digest sits in
/// its own key so the text the person confirms does not change.
#[tokio::test]
async fn the_card_summary_of_a_chart_draft_keeps_its_text() {
    let llm = mock_model(vec![]).await;
    let ch = mock_clickhouse(true).await;

    let (status, body) = tool_call(&llm, &ch, "create_chart", &top_ten_args()).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["result"]["summary"],
        json!(
            "Creates a new chart \"Sales by region and month\" (bar) from mart mart_seeded_sales."
        ),
        "{body}"
    );
}

/// `MAX_ITER` rounds that each call `create_chart`, then a tool-free last
/// round that answers with `last` (the answer the loop asks for once its
/// rounds are spent).
///
/// Every round calls `create_chart` with its own title, so the rounds are
/// different calls: plan 111 stops the loop after three failures of the
/// *same* call, and this helper is for the rounds' budget, not for that
/// bound. The refusal is the same in every round, because it is the mart
/// that is missing, not the title.
fn spent_rounds(args: &str, last: Option<&str>) -> Vec<Value> {
    let mut rounds: Vec<Value> = (1..=8)
        .map(|n| {
            let mut call: Value = serde_json::from_str(args).expect("the call's args are json");
            call["title"] = json!(format!("Sales by region {n}"));
            model_round(
                None,
                &[(&format!("call_{n}"), "create_chart", &call.to_string())],
            )
        })
        .collect();
    rounds.push(model_round(last, &[]));
    rounds
}

#[tokio::test]
async fn an_empty_last_answer_after_a_refused_create_chart_names_the_tool_and_its_refusal() {
    let llm = mock_model(spent_rounds(&chart_args(MART, COLUMN), Some(""))).await;
    // The mart is not there, so every draft is refused.
    let ch = mock_clickhouse(false).await;

    let (status, body) = chat(&llm, &ch, "Buat grafik penjualan per wilayah.").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let answer = body["answer"].as_str().unwrap_or_default();
    assert!(answer.starts_with("The request was not finished"), "{body}");
    assert!(answer.contains("`create_chart`"), "{body}");
    let refusal = traced_step(&body, "create_chart")["result"]["error"]
        .as_str()
        .expect("the draft was refused with a sentence")
        .to_owned();
    assert!(
        answer.contains(&refusal.chars().take(200).collect::<String>()),
        "the reply does not carry the refusal ({refusal}): {body}"
    );
    assert_eq!(body["note"], "tool-call limit reached", "{body}");
}

#[tokio::test]
async fn an_empty_last_answer_with_no_refused_tool_still_says_the_request_was_not_finished() {
    let llm = mock_model(spent_rounds(&chart_args(MART, COLUMN), None)).await;
    // The mart is there, so every draft is offered and none is refused.
    let ch = mock_clickhouse(true).await;

    let (status, body) = chat(&llm, &ch, "Buat grafik penjualan per wilayah.").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let answer = body["answer"].as_str().unwrap_or_default();
    assert!(answer.starts_with("The request was not finished"), "{body}");
    assert!(answer.contains("no tool refused a call"), "{body}");
}

#[tokio::test]
async fn a_last_answer_that_is_not_empty_is_left_as_the_model_wrote_it() {
    let llm = mock_model(spent_rounds(
        &chart_args(MART, COLUMN),
        Some("Only a draft."),
    ))
    .await;
    let ch = mock_clickhouse(true).await;

    let (status, body) = chat(&llm, &ch, "Buat grafik penjualan per wilayah.").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["answer"], "Only a draft.", "{body}");
}

/// AI-36 F1: every round calls `create_chart` with a mart that does not
/// exist, so nothing was drafted or saved, and the final model answer claims
/// a chart was made. The claim is not backed by the trace: the reply is the
/// fixed sentence, and the note stays the tool-call limit note.
#[tokio::test]
async fn a_claim_of_a_chart_that_no_tool_saved_is_replaced_by_the_fixed_sentence() {
    let llm = mock_model(spent_rounds(
        &chart_args(MART, COLUMN),
        Some("I created the chart you asked for."),
    ))
    .await;
    let ch = mock_clickhouse(false).await;

    let (status, body) = chat(&llm, &ch, "Buat grafik penjualan per wilayah.").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["answer"],
        "No chart was made because the request used up its tool calls before the chart could be saved, after these tools ran: `create_chart`.",
        "{body}"
    );
    assert_eq!(body["note"], "tool-call limit reached", "{body}");
}

/// AI-36 F5: a binned chart with no `limit` is cut by the builder's default
/// row cap. The draft's result carries the cap, so the server says it even
/// though the model's own answer does not mention it.
#[tokio::test]
async fn a_binned_chart_draft_with_no_limit_makes_the_reply_name_the_default_cap() {
    // A bin needs a bar: `chart_args` makes a table, which refuses bins. Its
    // measure must differ from the binned column, or the call is refused for
    // grouping by its own measure.
    let mut args: Value =
        serde_json::from_str(&chart_args_with(MEASURE, "kind", json!("bar"))).unwrap();
    args["measures"] = json!([MEASURE_TWO]);
    args["bin"] = json!({ "mode": "width", "width": 10 });
    let args = args.to_string();
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "create_chart", &args)]),
        model_round(Some("Drafted a chart of sales by bin."), &[]),
    ])
    .await;
    let ch = mock_clickhouse(true).await;

    let (status, body) = chat(&llm, &ch, "Make a chart binned by sale total.").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let step = traced_step(&body, "create_chart");
    assert_eq!(step["result"]["needs_confirmation"], json!(true), "{body}");
    assert_eq!(step["result"]["defaultCap"], json!(20), "{body}");
    let answer = body["answer"].as_str().unwrap_or_default();
    assert!(
        answer.ends_with(
            "This chart shows at most 20 values by default; set a `limit` to show more."
        ),
        "{body}"
    );
}

/// A bar chart binned by width over `MEASURE`, with the measure `MEASURE_TWO`:
/// the shape the default-cap tests save, with or without a `limit`.
fn binned_chart_args(extra: &Value) -> Value {
    let mut args = json!({
        "title": "Sales by bin",
        "kind": "bar",
        "mart": MART,
        "dimension": MEASURE,
        "measures": [MEASURE_TWO],
        "bin": { "mode": "width", "width": 10 },
    });
    for (key, value) in extra.as_object().into_iter().flatten() {
        args[key] = value.clone();
    }
    args
}

/// AI-36 F5 (review round 2): the saved reply of a binned chart with no `limit`
/// carries the cap as a number, so the turn can say it after the confirm.
#[tokio::test]
async fn a_binned_chart_saved_with_no_limit_reports_the_default_cap_in_its_result() {
    let llm = mock_model(vec![]).await;
    let ch = mock_clickhouse(true).await;
    let args = binned_chart_args(&json!({ "confirmed": true }));

    let (status, saved) = tool_call(&llm, &ch, "create_chart", &args.to_string()).await;

    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["result"]["created"], json!(true), "{saved}");
    assert_eq!(saved["result"]["defaultCap"], json!(20), "{saved}");
}

/// AI-36 F5 (review round 2): a binned chart that sets its own `limit` is not
/// cut by the default, so its saved result carries no `defaultCap`.
#[tokio::test]
async fn a_binned_chart_saved_with_a_limit_reports_no_default_cap() {
    let llm = mock_model(vec![]).await;
    let ch = mock_clickhouse(true).await;
    let args = binned_chart_args(&json!({ "confirmed": true, "limit": 5 }));

    let (status, saved) = tool_call(&llm, &ch, "create_chart", &args.to_string()).await;

    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["result"]["created"], json!(true), "{saved}");
    assert!(saved["result"].get("defaultCap").is_none(), "{saved}");
}

/// AI-36 F5 (review round 2): the confirmed update of a binned chart with no
/// `limit` reports the default cap in its result, as the create does.
#[tokio::test]
async fn a_binned_update_saved_with_no_limit_reports_the_default_cap_in_its_result() {
    let llm = mock_model(vec![]).await;
    let ch = mock_clickhouse_with_stored_chart("", false).await;
    let mut args = binned_chart_args(&json!({ "confirmed": true }));
    args["id"] = json!(STORED_CHART);

    let (status, saved) = tool_call(&llm, &ch, "update_chart", &args.to_string()).await;

    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["result"]["updated"], json!(true), "{saved}");
    assert_eq!(saved["result"]["defaultCap"], json!(20), "{saved}");
}

/// The chart the update tests name, and the board it is filed on.
const STORED_CHART: &str = "c_stored";
const STORED_BOARD: &str = "b_stored";
/// A formatting key the stored chart carries and the mart does not return:
/// a draft that merged the stored chart would name it in its refusal.
const STALE_FORMAT: &str = "hidden_revenue";

/// `ClickHouse` for an update of the stored chart `STORED_CHART`, which sits
/// on a board owned by `owner` (an empty owner is a shared board). With
/// `stale_format` the stored chart carries a value format that names a
/// column the mart does not return. The mock cannot filter by viewer: it
/// answers every read, so only the board's owner and the caller the draft
/// reads as decide whether the chart is shown.
async fn mock_clickhouse_with_stored_chart(owner: &str, stale_format: bool) -> MockServer {
    let ch = MockServer::start().await;
    let mut def = json!({
        "title": "Sales by region",
        "kind": "bar",
        "mart": MART,
        "dimension": COLUMN,
        "measures": [MEASURE],
        "board": STORED_BOARD,
    });
    if stale_format {
        def["valueFormats"] = json!({ STALE_FORMAT: { "style": "number" } });
    }
    let document = json!({
        "spec": {
            "id": STORED_CHART,
            "title": "Sales by region",
            "kind": "bar",
            "mart": MART,
            "sql": "SELECT region_code, sum(sale_total) AS sale_total FROM serving.mart_seeded_sales GROUP BY region_code",
            "x": COLUMN,
            "y": MEASURE,
        },
        "def": def,
        "hasYear": false,
    });
    Mock::given(method("POST"))
        .and(body_string_contains(
            "SELECT is_deleted, board FROM console.bi_chart",
        ))
        .respond_with(rows(&json!([{ "is_deleted": 0, "board": STORED_BOARD }])))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains(
            "FROM console.bi_chart FINAL WHERE is_deleted = 0",
        ))
        .respond_with(rows(&json!([{
            "id": STORED_CHART,
            "spec_json": document.to_string(),
            "board": STORED_BOARD,
            "created_by": "ai",
            "created_at": "2026-10-22 00:00:00",
        }])))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains(
            "SELECT is_deleted, owner_id FROM console.bi_board",
        ))
        .respond_with(rows(&json!([{ "is_deleted": 0, "owner_id": owner }])))
        .mount(&ch)
        .await;
    // The level check of the save, which the draft runs too, reads the board
    // itself. It is mounted after the state read above, whose statement it
    // would also match.
    Mock::given(method("POST"))
        .and(body_string_contains("FROM console.bi_board FINAL"))
        .respond_with(rows(&json!([{
            "id": STORED_BOARD,
            "name": "Sales",
            "folder_id": "",
            "owner_id": owner,
        }])))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("FROM system.tables"))
        .respond_with(rows(&json!([{ "n": "1" }])))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("FROM system.columns"))
        .respond_with(rows(&json!([
            { "name": COLUMN, "type": "String" },
            { "name": SECOND_COLUMN, "type": "String" },
            { "name": MEASURE, "type": "Float64" },
            { "name": MEASURE_TWO, "type": "Float64" },
        ])))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .respond_with(rows(&json!([])))
        .mount(&ch)
        .await;
    ch
}

/// A rename of the stored chart that names no data field, so the stored ones
/// are merged into the draft.
fn rename_args() -> String {
    json!({ "id": STORED_CHART, "title": "Sales by area", "kind": "bar" }).to_string()
}

/// The person who edits dashboards but administers nothing: unlike the seeded
/// administrator, they see their own personal space and no one else's.
const WRITER: &str = "dashboard:read, dashboard:write, assistant:use";

/// `POST /api/ai/tool` as the `WRITER`.
async fn tool_call_as_writer(
    llm: &MockServer,
    ch: &MockServer,
    tool: &str,
    args: &str,
) -> (StatusCode, Value) {
    post_as(
        llm,
        ch,
        "/api/ai/tool",
        &tool_body(tool, args),
        Some(WRITER),
    )
    .await
}

/// One build-mode chat turn as the `WRITER`.
async fn chat_as_writer(llm: &MockServer, ch: &MockServer, question: &str) -> (StatusCode, Value) {
    post_as(llm, ch, "/api/ai/chat", &chat_body(question), Some(WRITER)).await
}

/// What a refused update must not carry: nothing of the stored chart.
fn assert_nothing_of_the_stored_chart(body: &Value) {
    let text = body.to_string();
    for hidden in [STALE_FORMAT, STORED_BOARD, MART, COLUMN, MEASURE] {
        assert!(!text.contains(hidden), "{hidden} leaked: {body}");
    }
}

/// Call site of the confirm path (`POST /api/ai/tool`): the draft of an update
/// is read as the caller, so a chart on somebody else's private board answers
/// what `update_chart` answers and not a refusal built from the chart.
#[tokio::test]
async fn the_confirm_path_draft_of_an_update_of_a_chart_on_a_private_board_says_the_tile_does_not_exist()
 {
    let llm = mock_model(vec![]).await;
    let ch = mock_clickhouse_with_stored_chart("someone-else", true).await;

    let (status, body) = tool_call_as_writer(&llm, &ch, "update_chart", &rename_args()).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["result"],
        json!({ "error": "this tile does not exist" }),
        "{body}"
    );
    assert_eq!(body["outcome"], "failed", "{body}");
    assert_nothing_of_the_stored_chart(&body["result"]);
    assert!(!has_insert(&statements(&ch).await), "nothing was written");
}

/// Call site of the chat loop (`POST /api/ai/chat`): the same refusal.
#[tokio::test]
async fn the_chat_loop_draft_of_an_update_of_a_chart_on_a_private_board_says_the_tile_does_not_exist()
 {
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "update_chart", &rename_args())]),
        model_round(Some("That chart is not there."), &[]),
    ])
    .await;
    let ch = mock_clickhouse_with_stored_chart("someone-else", true).await;

    let (status, body) = chat_as_writer(&llm, &ch, "Rename the sales chart.").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let step = traced_step(&body, "update_chart");
    assert_eq!(
        step["result"],
        json!({ "error": "this tile does not exist" }),
        "{body}"
    );
    assert_eq!(step["ok"], false, "{body}");
    assert_nothing_of_the_stored_chart(&step["result"]);
    assert!(!has_insert(&statements(&ch).await), "nothing was written");
}

/// The control for the two tests above: the same update of a chart on a
/// shared board is merged with the stored chart and offered as a draft, so
/// the refusals above come from the caller's reach and not from a draft that
/// refuses every update. It carries the digest of the merged chart, as the
/// saved reply does.
#[tokio::test]
async fn the_draft_of_an_update_of_a_chart_on_a_shared_board_is_still_offered() {
    let llm = mock_model(vec![]).await;
    let ch = mock_clickhouse_with_stored_chart("", false).await;

    let (status, body) = tool_call_as_writer(&llm, &ch, "update_chart", &rename_args()).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["outcome"], "needs_confirmation", "{body}");
    assert_eq!(body["result"]["needs_confirmation"], json!(true), "{body}");
    assert_eq!(
        body["result"]["definition"],
        json!("dimension: region_code; measures: sale_total"),
        "{body}"
    );
    assert!(
        !has_insert(&statements(&ch).await),
        "a draft writes nothing"
    );
}

/// The stored chart is read as the caller, so a chart on a shared board whose
/// formatting no longer fits is refused with the sentence about it: the
/// merge still happens for a chart the caller sees.
#[tokio::test]
async fn the_draft_of_an_update_of_a_visible_chart_is_merged_with_the_stored_chart() {
    let llm = mock_model(vec![]).await;
    let ch = mock_clickhouse_with_stored_chart("", true).await;

    let (status, body) = tool_call_as_writer(&llm, &ch, "update_chart", &rename_args()).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["outcome"], "failed", "{body}");
    assert!(
        body["result"]["error"]
            .as_str()
            .is_some_and(|e| e.contains(STALE_FORMAT)),
        "{body}"
    );
}

// ── Plan 111 (AI-1): a rename with only the new title, and an audit row
// that says why a call failed ─────────────────────────────────────────────

/// A rename that names the chart and its new title and nothing else: the
/// kind is left out, so the stored one is kept (`#377`'s rule: a field the
/// call leaves out keeps its saved value).
fn rename_only_title_args() -> String {
    json!({ "id": STORED_CHART, "title": "Sales by area" }).to_string()
}

/// The request the Auditor sent on staging (plan 111): the model renames a
/// chart in chat and sends only the chart's id and the new title. The draft
/// must be offered, with the same `needs_confirmation` card the full call
/// gets: the fields the call leaves out are kept.
#[tokio::test]
async fn a_rename_that_names_only_the_new_title_is_offered_as_a_draft() {
    let llm = mock_model(vec![
        model_round(
            None,
            &[("call_1", "update_chart", &rename_only_title_args())],
        ),
        model_round(Some("The rename waits for the Confirm button."), &[]),
    ])
    .await;
    let ch = mock_clickhouse_with_stored_chart("", false).await;

    let (status, body) = chat(&llm, &ch, "Rename the sales chart to Sales by area.").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let step = traced_step(&body, "update_chart");
    assert_eq!(step["result"]["needs_confirmation"], json!(true), "{body}");
    // The draft carries the stored chart's own fields: the kind the call
    // left out, and the data it left out too (`#377`'s rule).
    assert_eq!(
        step["result"]["definition"],
        json!("dimension: region_code; measures: sale_total"),
        "{body}"
    );
    assert!(
        !has_insert(&statements(&ch).await),
        "a draft writes nothing"
    );
}

/// The confirm of that rename: the title changes, and the fields the call
/// left out stay as they are stored.
#[tokio::test]
async fn the_confirmed_rename_changes_the_title_and_keeps_the_stored_fields() {
    let llm = mock_model(vec![]).await;
    let ch = mock_clickhouse_with_stored_chart("", false).await;
    let mut confirmed: Value = serde_json::from_str(&rename_only_title_args()).unwrap();
    confirmed["confirmed"] = json!(true);

    let (status, body) = tool_call(&llm, &ch, "update_chart", &confirmed.to_string()).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["outcome"], "executed", "{body}");
    assert_eq!(body["result"]["title"], "Sales by area", "{body}");
    let sent = statements(&ch).await;
    let insert = sent
        .iter()
        .find(|s| s.contains("INSERT INTO console.bi_chart"))
        .unwrap_or_else(|| panic!("the chart was written: {sent:?}"));
    assert!(insert.contains("Sales by area"), "{insert}");
    for kept in [COLUMN, MEASURE] {
        assert!(insert.contains(kept), "{kept} was dropped: {insert}");
    }
}

/// One build-mode `POST /api/ai/chat` turn, the body it answered with, and
/// the `audit_event` rows it wrote for `action`: the audit trail is a table,
/// so a test that proves what a row carries needs the pool it was written
/// with.
async fn chat_and_audit_rows(
    llm: &MockServer,
    ch: &MockServer,
    question: &str,
    action: &str,
) -> (StatusCode, Value, Vec<Value>) {
    let TestApp { router, pool } = spin_up_with_env(&env_for(llm, ch)).await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/chat")
                .header("content-type", "application/json")
                .header("cookie", &cookie)
                .body(Body::from(
                    serde_json::to_vec(&chat_body(question)).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let rows = common::audit_rows(&pool, action).await;
    (status, body, rows)
}

/// A statement the mocked engine refuses, with the engine's own answer.
const FAILING_SQL: &str = "SELECT pengunjungg FROM silver.orders";

/// The `run_sql` arguments of that statement.
fn failing_sql_args() -> String {
    json!({ "sql": FAILING_SQL }).to_string()
}

/// A failed tool call's audit row says why (plan 111, `AI-1`): the detail is
/// the browser's copy of the failure — the fixed sentence and the reference,
/// the same text the person sees — and never the engine's own text, which
/// only the model reads.
#[tokio::test]
async fn a_failed_tool_call_writes_an_audit_row_that_says_why() {
    let args = failing_sql_args();
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "run_sql", &args)]),
        model_round(Some("The column is order_id."), &[]),
    ])
    .await;
    let ch = mock_clickhouse_refusing(
        "pengunjungg",
        ResponseTemplate::new(400).set_body_string(CLICKHOUSE_UNKNOWN_IDENTIFIER),
    )
    .await;

    let (status, body, rows) =
        chat_and_audit_rows(&llm, &ch, "berapa tamu bulan ini?", "run_sql").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let row = rows
        .last()
        .unwrap_or_else(|| panic!("the call wrote a row"));
    assert_eq!(row["outcome"], "failed", "{row}");
    let detail = row["detail"]
        .as_str()
        .unwrap_or_else(|| panic!("the row says why: {row}"));
    assert!(
        detail.starts_with(&format!("{COLUMN_DOES_NOT_EXIST} Reference: ")),
        "the browser's sentence and the reference: {detail}"
    );
    for engine_text in [
        "DB::Exception",
        "Code:",
        "pengunjungg",
        "UNKNOWN_IDENTIFIER",
    ] {
        assert!(!detail.contains(engine_text), "{engine_text} in {detail}");
    }
}

/// A model that repeats the same failing call ends the turn with the
/// refusal, well inside the 120 s the turn may take (plan 111, the parking
/// lot's item 6): the loop stops calling tools after the third failure of
/// one call and asks the model for its final answer, as the iteration
/// budget does.
#[tokio::test]
async fn a_model_that_repeats_a_failing_call_ends_with_the_refusal() {
    // The same call with the same arguments, three times: the loop stops
    // calling tools after the third failure of one call.
    let args = failing_sql_args();
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "run_sql", &args)]),
        model_round(None, &[("call_2", "run_sql", &args)]),
        model_round(None, &[("call_3", "run_sql", &args)]),
        // The round the loop asks for the final answer in.
        model_round(None, &[]),
    ])
    .await;
    let ch = mock_clickhouse_refusing(
        "pengunjungg",
        ResponseTemplate::new(400).set_body_string(CLICKHOUSE_UNKNOWN_IDENTIFIER),
    )
    .await;

    let started = std::time::Instant::now();
    let (status, body) = chat(&llm, &ch, "berapa tamu bulan ini?").await;
    let elapsed = started.elapsed();

    assert_eq!(status, StatusCode::OK, "{body}");
    // The loop stopped after the third failure of that one call: the trace
    // holds three steps, and the turn did not run out of rounds.
    assert_eq!(
        body["toolTrace"].as_array().map(Vec::len),
        Some(3),
        "{body}"
    );
    assert_eq!(
        body["note"], "the same call failed too many times",
        "{body}"
    );
    let answer = body["answer"].as_str().unwrap_or_default();
    assert!(
        answer.contains("run_sql"),
        "the answer names the call: {answer}"
    );
    assert!(
        answer.contains(COLUMN_DOES_NOT_EXIST),
        "the answer carries the refusal: {answer}"
    );
    for engine_text in [
        "DB::Exception",
        "Code:",
        "pengunjungg",
        "UNKNOWN_IDENTIFIER",
    ] {
        assert!(!answer.contains(engine_text), "{engine_text} in {answer}");
    }
    assert!(
        elapsed < std::time::Duration::from_secs(60),
        "the turn ended in {elapsed:?}"
    );
    eprintln!("the repeated-call turn ended in {elapsed:?}");
}
