//! `POST /api/ai/chat` with `stream: true` sends the answer text as
//! `{"type":"delta"}` NDJSON lines while the model writes it, and still
//! ends with the citation-checked `done` body — streaming changes when the
//! user sees the words, never what the final answer is allowed to claim
//! (`citations::annotate_answer`, WS7 item F2).
//!
//! The mocked model answers its tool-calling round with plain JSON (the
//! streamed client's fallback for endpoints that ignore `stream: true`) and
//! its final round as server-sent events, so both paths run through the
//! real router here.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::fmt::Write as _;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, Request as WireRequest, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

/// Holds when the request body does NOT contain `needle` (see
/// `ai_citations.rs`: `run_sql`'s `EXPLAIN AST` dry run is a superset of
/// the real query's body).
struct BodyDoesNotContain(&'static str);

impl wiremock::Match for BodyDoesNotContain {
    fn matches(&self, request: &WireRequest) -> bool {
        !String::from_utf8_lossy(&request.body).contains(self.0)
    }
}

fn sse(pieces: &[&str]) -> String {
    let mut body = String::new();
    for piece in pieces {
        let event = json!({ "choices": [ { "delta": { "content": piece } } ] });
        let _ = write!(body, "data: {event}\n\n");
    }
    body.push_str("data: [DONE]\n\n");
    body
}

/// The mocked model (a JSON tool-call round, then an SSE final answer)
/// and the mocked `ClickHouse` behind `run_sql`.
async fn mock_backends() -> (MockServer, MockServer) {
    let llm = MockServer::start().await;
    // Round 1 (plain JSON): the model calls run_sql for a real count.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": "run_sql",
                        "arguments": r#"{"sql":"SELECT count() AS n FROM serving.mart_x"}"#,
                    },
                }],
            } }],
        })))
        .up_to_n_times(1)
        .mount(&llm)
        .await;
    // Round 2 (SSE): the final answer, with a think block split across
    // events and a table the tool never produced.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            sse(&[
                "<thi",
                "nk>draft reasoning</think>Ada 5 ",
                "baris.\n\n| region | total |\n|---|---|\n| Jakarta | 999 |",
            ]),
            "text/event-stream",
        ))
        .mount(&llm)
        .await;

    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("EXPLAIN AST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"explain": "SelectWithUnionQuery (children 1)\n ExpressionList ...\n"}],
        })))
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

    (llm, ch)
}

#[tokio::test]
async fn a_streamed_chat_sends_deltas_then_the_checked_answer() {
    let (llm, ch) = mock_backends().await;
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
                        "stream": true,
                        "messages": [{ "role": "user", "content": "berapa baris di mart_x?" }],
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
    let events: Vec<Value> = String::from_utf8_lossy(&bytes)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("every line is one JSON event"))
        .collect();

    let streamed: String = events
        .iter()
        .filter(|e| e["type"] == "delta")
        .filter_map(|e| e["text"].as_str())
        .collect();
    assert!(
        streamed.starts_with("Ada 5 baris."),
        "the answer text streams as it is written: {streamed:?}"
    );
    assert!(
        !streamed.contains("think") && !streamed.contains("draft reasoning"),
        "reasoning never streams, even with its tag split across events: {streamed:?}"
    );

    // The draft's table is backed by no tool result, so the loop runs one
    // repair round (`routes::ai::run_chat`): a `verifying` status, then the
    // model is asked again. This mock answers the same way both times, so
    // its reasoning streams once per round.
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "status" && e["phase"] == "verifying"),
        "a draft with unbacked figures gets one repair round: {events:?}"
    );
    let reasoning: String = events
        .iter()
        .filter(|e| e["type"] == "reasoning")
        .filter_map(|e| e["text"].as_str())
        .collect();
    assert_eq!(
        reasoning, "draft reasoningdraft reasoning",
        "the think block streams as reasoning, apart from the answer, once per round"
    );

    let last = events.last().expect("at least one event");
    assert_eq!(last["type"], "done", "the stream ends with the done body");
    let answer = last["body"]["answer"].as_str().expect("answer is a string");
    assert!(
        answer.contains("table omitted: not backed by a tool result"),
        "the done answer is still citation-checked: {answer}"
    );
    assert!(answer.contains('5'), "the real number survives: {answer}");
    let first_delta = events.iter().position(|e| e["type"] == "delta");
    let done = events.iter().position(|e| e["type"] == "done");
    assert!(first_delta < done, "deltas arrive before done: {events:?}");
}
