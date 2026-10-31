//! A request with steps left ends its turn with the next step, and the
//! console sends it back after the person confirms the draft (`AI-15`).
//!
//! A one-message request with two dependent writes could not finish: the
//! first draft waited for its Confirm and the conversation stopped there.
//! Now the model ends such a turn with a `NEXT:` line, the server takes the
//! line off the answer and returns it as `nextStep`, and the console sends
//! it back as a message once the draft is confirmed.
//!
//! The model is mocked round by round, as in `ai_ask_back_turn.rs`, and the
//! draft is a `create_board` call, which the gate answers with
//! `needs_confirmation` without writing anything.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

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

/// `ClickHouse` that answers every read with an empty answer.
async fn mock_clickhouse() -> MockServer {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&ch)
        .await;
    ch
}

/// A `create_board` call, which the gate answers with a draft.
const CREATE_BOARD: &str = r#"{"name":"Overview"}"#;

/// Sends one build-mode chat turn and returns the response body.
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
                        "mode": "build",
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
async fn a_turn_with_a_pending_draft_returns_the_next_step_and_takes_the_line_off_the_answer() {
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "create_board", CREATE_BOARD)]),
        model_round(Some("The board draft is ready.\nNEXT: add two charts"), &[]),
    ])
    .await;
    let ch = mock_clickhouse().await;

    let body = chat(&llm, &ch, "Build me a dashboard about events.").await;

    let step = traced_step(&body, "create_board");
    assert_eq!(step["result"]["needs_confirmation"], json!(true), "{body}");
    assert_eq!(
        body["nextStep"], "add two charts",
        "the next step is the line's text: {body}"
    );
    assert_eq!(
        body["answer"], "The board draft is ready.",
        "the line is not part of the answer: {body}"
    );
    assert!(
        !body["answer"]
            .as_str()
            .unwrap_or_default()
            .contains("NEXT:"),
        "the line is off the answer: {body}"
    );
}

#[tokio::test]
async fn a_turn_without_a_pending_draft_keeps_the_line_and_has_no_next_step() {
    let llm = mock_model(vec![model_round(
        Some("All done.\nNEXT: add two charts"),
        &[],
    )])
    .await;
    let ch = mock_clickhouse().await;

    let body = chat(&llm, &ch, "What can you do?").await;

    assert!(
        body.get("nextStep").is_none(),
        "no draft waits for a Confirm, so there is no next step: {body}"
    );
    assert_eq!(
        body["answer"], "All done.\nNEXT: add two charts",
        "the line stays where the model wrote it: {body}"
    );
}

#[tokio::test]
async fn a_next_line_in_the_middle_of_an_answer_is_left_alone() {
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "create_board", CREATE_BOARD)]),
        model_round(Some("NEXT: add two charts\nThe board draft is ready."), &[]),
    ])
    .await;
    let ch = mock_clickhouse().await;

    let body = chat(&llm, &ch, "Build me a dashboard about events.").await;

    assert!(
        body.get("nextStep").is_none(),
        "only the last line is a next step: {body}"
    );
    assert_eq!(
        body["answer"], "NEXT: add two charts\nThe board draft is ready.",
        "the answer is untouched: {body}"
    );
}

#[tokio::test]
async fn a_next_line_without_its_text_is_no_next_step() {
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "create_board", CREATE_BOARD)]),
        model_round(Some("The board draft is ready.\nNEXT:"), &[]),
    ])
    .await;
    let ch = mock_clickhouse().await;

    let body = chat(&llm, &ch, "Build me a dashboard about events.").await;

    assert!(
        body.get("nextStep").is_none(),
        "an empty step is no step: {body}"
    );
    assert!(
        body["answer"]
            .as_str()
            .unwrap_or_default()
            .contains("NEXT:"),
        "the empty line stays in the answer: {body}"
    );
}
