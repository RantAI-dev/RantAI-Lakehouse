#![allow(
    clippy::assert_is_empty,
    reason = "test assertions checking collection emptiness are idiomatic and avoid empty array type casts"
)]
//! The chat asks or answers in one turn, never both (plan 14, AI-17).
//!
//! `run_chat` enforces it, not the prompt: an accepted `ask_user` ends the
//! turn with its question as the answer and runs nothing after it, and an
//! `ask_user` that comes after a tool that ran in the same turn is refused
//! with a result the model reads. The mocked model here answers each round
//! with plain JSON, so the non-streamed `/api/ai/chat` path drives the same
//! loop the streamed path does.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, Request as WireRequest, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

/// Holds when the request body does NOT contain `needle`. `run_sql`'s
/// `EXPLAIN AST` dry run carries the real query inside it, so the real
/// query's mock must exclude the dry run by its absence of that marker.
struct BodyDoesNotContain(&'static str);

impl wiremock::Match for BodyDoesNotContain {
    fn matches(&self, request: &WireRequest) -> bool {
        !String::from_utf8_lossy(&request.body).contains(self.0)
    }
}

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

/// An `ask_user` call with two readings, which the tool accepts.
const ASK_TWO: &str = r#"{"term":"sales","question":"Which table do you mean by sales?","options":["orders","invoices"]}"#;
/// An `ask_user` call with one reading, which the tool rejects.
const ASK_ONE: &str =
    r#"{"term":"sales","question":"Which table do you mean by sales?","options":["orders"]}"#;
/// A `run_sql` call whose real query `ClickHouse` answers with `{"n": 5}`.
const RUN_SQL: &str = r#"{"sql":"SELECT count() AS n FROM serving.sample_rows"}"#;

/// The refusal a late `ask_user` gets. Its wording is part of the contract
/// the model reads, so the tests pin it.
const LATE_ASK_REFUSAL: &str = "ask_user must come before any query in a turn. Answer now with the reading you used, and say which reading it is.";

/// Mounts `rounds` in order on the model: each answers one request, then
/// the next one takes over (`up_to_n_times(1)`, as in `ai_chat_stream.rs`).
/// A last round with no limit answers any further request.
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

/// `ClickHouse` for `run_sql`: the dry run gets `EXPLAIN AST`'s text dump
/// of a `SELECT` (`dry_run_sql` reads its first line), the real query gets
/// one row.
async fn mock_clickhouse() -> MockServer {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("EXPLAIN AST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("SelectWithUnionQuery (children 1)\n ExpressionList ...\n"),
        )
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .and(BodyDoesNotContain("EXPLAIN AST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [{"name": "n", "type": "UInt64"}],
            "data": [{"n": 5}],
            "rows": 1,
        })))
        .mount(&ch)
        .await;
    ch
}

/// Sends one non-streamed chat turn and returns the response body.
async fn chat(llm: &MockServer, ch: &MockServer, question: &str) -> Value {
    let mut env = std::collections::HashMap::new();
    env.insert("LLM_URL".to_owned(), llm.uri());
    env.insert("CH_URL".to_owned(), ch.uri());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/chat")
                .header("content-type", "application/json")
                .header("cookie", &cookie)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "messages": [{ "role": "user", "content": question }],
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).expect("valid JSON body")
}

#[tokio::test]
async fn an_accepted_ask_ends_the_turn_with_its_question_as_the_answer() {
    let llm = mock_model(vec![model_round(
        Some("Probably the orders table, I think."),
        &[("call_1", "ask_user", ASK_TWO)],
    )])
    .await;
    let ch = mock_clickhouse().await;

    let body = chat(&llm, &ch, "berapa sales?").await;

    assert_eq!(
        body["answer"], "Which table do you mean by sales?",
        "the reply is the question alone, not the text written beside the ask: {body}"
    );
    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(trace.len(), 1, "one step, the ask: {trace:?}");
    assert_eq!(trace[0]["tool"], "ask_user");
    assert_eq!(trace[0]["ok"], true);
    let model_calls = llm
        .received_requests()
        .await
        .expect("recording is on")
        .len();
    assert_eq!(model_calls, 1, "no model round runs after an accepted ask");
}

#[tokio::test]
async fn a_call_after_an_accepted_ask_in_the_same_round_is_not_run() {
    let llm = mock_model(vec![model_round(
        None,
        &[
            ("call_1", "ask_user", ASK_TWO),
            ("call_2", "run_sql", RUN_SQL),
        ],
    )])
    .await;
    let ch = mock_clickhouse().await;

    let body = chat(&llm, &ch, "berapa sales?").await;

    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(
        trace.len(),
        1,
        "run_sql after the ask is neither run nor recorded: {trace:?}"
    );
    assert_eq!(trace[0]["tool"], "ask_user");
    // The chat's own setup reads the DATA MAP from ClickHouse before the first
    // round, so the test looks for the one statement `run_sql` would send: the
    // real query and its dry run are both about `serving.sample_rows`.
    let sample_queries = ch
        .received_requests()
        .await
        .expect("recording is on")
        .iter()
        .filter(|r| String::from_utf8_lossy(&r.body).contains("serving.sample_rows"))
        .count();
    assert_eq!(
        sample_queries, 0,
        "run_sql after the ask is not sent to ClickHouse"
    );
}

#[tokio::test]
async fn an_ask_after_a_query_in_the_same_turn_is_refused_and_the_turn_answers() {
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "run_sql", RUN_SQL)]),
        model_round(None, &[("call_2", "ask_user", ASK_TWO)]),
        model_round(Some("Ada 5 baris dari serving.sample_rows."), &[]),
    ])
    .await;
    let ch = mock_clickhouse().await;

    let body = chat(&llm, &ch, "berapa sales?").await;

    assert_eq!(
        body["answer"], "Ada 5 baris dari serving.sample_rows.",
        "the third round's answer is the reply, the refused ask did not end the turn: {body}"
    );
    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(trace.len(), 2, "run_sql then the refused ask: {trace:?}");
    assert_eq!(trace[0]["tool"], "run_sql");
    assert_eq!(trace[0]["ok"], true);
    assert_eq!(trace[1]["tool"], "ask_user");
    assert_eq!(trace[1]["ok"], false);
    assert_eq!(trace[1]["result"]["error"], LATE_ASK_REFUSAL);
    assert!(
        trace
            .iter()
            .all(|step| step["result"]["asked"] != json!(true)),
        "no step holds an accepted ask: {trace:?}"
    );
}

#[tokio::test]
async fn an_ask_the_tool_rejects_does_not_end_the_turn() {
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "ask_user", ASK_ONE)]),
        model_round(Some("Pilih tabel orders."), &[]),
    ])
    .await;
    let ch = mock_clickhouse().await;

    let body = chat(&llm, &ch, "berapa sales?").await;

    assert_eq!(
        body["answer"], "Pilih tabel orders.",
        "a rejected ask (one option) leaves the turn open for the next round: {body}"
    );
    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(
        trace.len(),
        1,
        "the rejected ask is the one step: {trace:?}"
    );
    assert_eq!(trace[0]["ok"], false);
}
