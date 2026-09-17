//! End-to-end proof (WS7 item F3, Hard Requirement 2 of the phase brief):
//! a `POST /api/ai/chat` turn where the mocked LLM's final answer
//! contains a Markdown table with NO backing tool call gets that table
//! visibly OMITTED in the real HTTP response body — not merely at the
//! unit level (`citations::annotate_answer`, `crates/lakehouse-api/src/
//! routes/ai/citations.rs`), but through the real router, the real tool
//! dispatch, and the real `chat_response_body` assembly. The one real
//! number the model DID get from a tool call survives untouched.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request as WireRequest, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

/// An OpenAI-compatible `/chat/completions` response carrying ONE tool
/// call, matching `lakehouse-llm`'s own mocked-test shape
/// (`crates/lakehouse-llm/src/lib.rs`'s `chat_with_tools_returns_tool_
/// calls_when_present`).
fn tool_call_response(tool: &str, arguments_json: &str) -> Value {
    json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": { "name": tool, "arguments": arguments_json },
                }],
            },
        }],
    })
}

/// A plain final-answer response (no more tool calls) — ends the chat
/// loop.
fn final_answer_response(answer: &str) -> Value {
    json!({ "choices": [{ "message": { "role": "assistant", "content": answer } }] })
}

/// A [`wiremock::Match`] that holds whenever the request body does NOT
/// contain `needle` — the complement of [`wiremock::matchers::body_
/// string_contains`]. Needed here because `run_sql`'s `EXPLAIN AST`
/// dry-run (WS7 item F4) sends `"EXPLAIN AST " + <the real SELECT>` —
/// the dry-run body is a strict SUPERSET of the real query's body, so a
/// plain `body_string_contains` on the real query's own SQL text would
/// also match the dry-run request. Anchoring on the ABSENCE of `"EXPLAIN
/// AST"` is what keeps the two mocks mutually exclusive.
struct BodyDoesNotContain(&'static str);

impl wiremock::Match for BodyDoesNotContain {
    fn matches(&self, request: &WireRequest) -> bool {
        !String::from_utf8_lossy(&request.body).contains(self.0)
    }
}

async fn post_chat(app: &axum::Router, cookie: &str, question: &str) -> Value {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/chat")
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "messages": [{ "role": "user", "content": question }],
                    }))
                    .expect("serialize body"),
                ))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "expected a normal chat response"
    );
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read response body");
    serde_json::from_slice(&bytes).expect("valid JSON body")
}

#[tokio::test]
async fn a_fabricated_table_with_no_tool_call_is_omitted_in_the_real_response() {
    let llm = MockServer::start().await;
    // Turn 1: the model calls run_sql for a real count.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tool_call_response(
            "run_sql",
            r#"{"sql":"SELECT count() AS n FROM serving.mart_x"}"#,
        )))
        .up_to_n_times(1)
        .mount(&llm)
        .await;
    // Turn 2: the model's FINAL answer invents a second table the tool
    // never produced (a fabricated "Jakarta"/999 row with no run_sql
    // behind it at all).
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(final_answer_response(
                "Ada 5 baris.\n\n| region | total |\n|---|---|\n| Jakarta | 999 |",
            )),
        )
        .mount(&llm)
        .await;

    let ch = MockServer::start().await;
    // `run_sql`'s dry run (WS7 item F4): `EXPLAIN AST SELECT count() AS n
    // FROM serving.mart_x` — must report a real `SELECT` so the query is
    // allowed through.
    Mock::given(method("POST"))
        .and(wiremock::matchers::body_string_contains("EXPLAIN AST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"explain": "SelectWithUnionQuery (children 1)\n ExpressionList ...\n"}],
        })))
        .mount(&ch)
        .await;
    // The real query itself — the ONLY genuine tool result this turn:
    // `{"n": 5}`.
    Mock::given(method("POST"))
        .and(BodyDoesNotContain("EXPLAIN AST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [{"name": "n", "type": "UInt64"}],
            "data": [{"n": 5}],
            "rows": 1,
        })))
        .mount(&ch)
        .await;

    let mut env = std::collections::HashMap::new();
    env.insert("LLM_URL".to_owned(), llm.uri());
    env.insert("CH_URL".to_owned(), ch.uri());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let body = post_chat(&router, &cookie, "berapa baris di mart_x?").await;
    let answer = body["answer"].as_str().expect("answer is a string");

    assert!(
        answer.contains("table omitted: not backed by a tool result"),
        "the fabricated table must be visibly omitted, not silently passed through: {answer}"
    );
    assert!(
        !answer.contains("Jakarta"),
        "the fabricated row must not survive into the response: {answer}"
    );
    assert!(
        answer.contains('5'),
        "the REAL number the tool actually returned must survive: {answer}"
    );
}
