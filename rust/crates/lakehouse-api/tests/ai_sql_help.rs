//! Query Studio's SQL help (plan 15): the `propose_sql` tool, its rules, and
//! the switch `AI_SQL_HELP`.
//!
//! The model is mocked round by round, as in `ai_ask_back_turn.rs`, and
//! `ClickHouse` is mocked by statement: the proposal's `EXPLAIN PLAN` gets the
//! engine's answer, and anything else that reaches `ClickHouse` is recorded so
//! a test can assert that no statement other than an `EXPLAIN` was sent.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, method, path};
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

/// `ClickHouse` for the chat: the DATA MAP's reads get an empty answer, and
/// an `EXPLAIN PLAN` of the proposed statement gets `plan_answer`.
async fn mock_clickhouse(plan_answer: ResponseTemplate) -> MockServer {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("EXPLAIN PLAN"))
        .respond_with(plan_answer)
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

/// Sends one chat turn with the given allowlist (`None` for no allowlist)
/// and the switch, and returns the status and the body.
async fn chat(
    llm: &MockServer,
    ch: &MockServer,
    switch: &str,
    tools: Option<Value>,
    question: &str,
) -> (StatusCode, Value) {
    let mut env = std::collections::HashMap::new();
    env.insert("LLM_URL".to_owned(), llm.uri());
    env.insert("CH_URL".to_owned(), ch.uri());
    env.insert("AI_SQL_HELP".to_owned(), switch.to_owned());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let mut body = json!({ "messages": [{ "role": "user", "content": question }] });
    if let Some(tools) = tools {
        body["tools"] = tools;
    }
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

/// The `tools` array and the system message the model received in its
/// first round.
async fn first_model_request(llm: &MockServer) -> (Vec<String>, String) {
    let requests = llm.received_requests().await.expect("recording is on");
    let first: Value = serde_json::from_slice(&requests[0].body).expect("a JSON request");
    let names = first["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .filter_map(|t| t["function"]["name"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let system = first["messages"][0]["content"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    (names, system)
}

/// The fix sentence, the engine, the statement and the error, as the console
/// sends them.
const FIX_MESSAGE: &str = "Why does this query fail? Fix it.\n\nENGINE: clickhouse\n\nSTATEMENT:\nSELECT no_such_col FROM silver.orders\n\nERROR:\nUNKNOWN_IDENTIFIER";

const WRONG_PROPOSAL: &str = r#"{"sql":"SELECT no_such_col FROM silver.orders","reason":"Used the wrong column.","engine":"clickhouse"}"#;
const RIGHT_PROPOSAL: &str = r#"{"sql":"SELECT order_id FROM silver.orders","reason":"The column is order_id, not no_such_col.","engine":"clickhouse"}"#;

#[tokio::test]
async fn an_allowlist_naming_propose_sql_offers_it_with_the_rules() {
    let llm = mock_model(vec![model_round(Some("Explained."), &[])]).await;
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;

    let (status, _) = chat(&llm, &ch, "true", Some(json!(["propose_sql"])), FIX_MESSAGE).await;

    assert_eq!(status, StatusCode::OK);
    let (names, system) = first_model_request(&llm).await;
    assert!(names.contains(&"propose_sql".to_owned()), "{names:?}");
    assert!(
        system.contains("Why does this query fail? Fix it."),
        "the rules name the fix sentence: {system}"
    );
    assert!(system.contains("Explain what this query does."), "{system}");
}

#[tokio::test]
async fn a_proposal_clickhouse_rejects_returns_to_the_model_and_a_second_one_ends_the_turn() {
    // The first proposal's EXPLAIN PLAN fails; the second passes.
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "propose_sql", WRONG_PROPOSAL)]),
        model_round(None, &[("call_2", "propose_sql", RIGHT_PROPOSAL)]),
    ])
    .await;
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("no_such_col"))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            "Code: 47. DB::Exception: Unknown identifier: no_such_col (UNKNOWN_IDENTIFIER) (version 26.8.1)",
        ))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("order_id"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&ch)
        .await;

    let (status, body) = chat(&llm, &ch, "true", Some(json!(["propose_sql"])), FIX_MESSAGE).await;

    assert_eq!(status, StatusCode::OK);
    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(
        trace.len(),
        2,
        "the rejected proposal, then the accepted one: {trace:?}"
    );
    assert_eq!(trace[0]["tool"], "propose_sql");
    assert_eq!(trace[0]["ok"], false);
    let error = trace[0]["result"]["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("no_such_col"),
        "the engine's text, cleaned: {error}"
    );
    assert!(
        !error.contains("DB::Exception"),
        "the prefix is cleaned: {error}"
    );
    assert_eq!(trace[1]["ok"], true);
    assert_eq!(trace[1]["result"]["proposed"], json!(true));
    assert_eq!(
        body["answer"], "The column is order_id, not no_such_col.",
        "the accepted proposal's reason is the reply: {body}"
    );
    let model_calls = llm.received_requests().await.expect("recording").len();
    assert_eq!(model_calls, 2, "no round runs after the accepted proposal");
}

#[tokio::test]
async fn run_sql_is_not_offered_and_clickhouse_gets_only_explains() {
    let llm = mock_model(vec![model_round(
        None,
        &[("call_1", "propose_sql", RIGHT_PROPOSAL)],
    )])
    .await;
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;

    let (_, _) = chat(&llm, &ch, "true", Some(json!(["propose_sql"])), FIX_MESSAGE).await;

    let (names, _) = first_model_request(&llm).await;
    assert!(!names.contains(&"run_sql".to_owned()), "{names:?}");
    // The allowlist names one tool, so the offered list is that tool alone.
    assert_eq!(names, ["propose_sql"], "{names:?}");
    // The proposal's statement reaches ClickHouse only as an `EXPLAIN PLAN`,
    // never as the query itself and never as `run_sql`'s `EXPLAIN AST` dry run.
    let sent = statements(&ch).await;
    let carrying: Vec<&String> = sent
        .iter()
        .filter(|s| s.contains("SELECT order_id FROM silver.orders"))
        .collect();
    assert!(!carrying.is_empty(), "the proposal was checked: {sent:?}");
    for statement in &carrying {
        assert!(
            statement.starts_with("EXPLAIN PLAN "),
            "the proposed statement reached ClickHouse as something but an EXPLAIN PLAN: {statement}"
        );
    }
}

#[tokio::test]
async fn with_the_switch_off_the_tool_is_not_offered_whatever_the_allowlist_says() {
    let llm = mock_model(vec![model_round(Some("No tool."), &[])]).await;
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;

    let (status, _) = chat(
        &llm,
        &ch,
        "false",
        Some(json!(["propose_sql"])),
        FIX_MESSAGE,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let (names, system) = first_model_request(&llm).await;
    assert!(!names.contains(&"propose_sql".to_owned()), "{names:?}");
    assert!(
        !system.contains("Why does this query fail? Fix it."),
        "the rules block is absent with the switch off: {system}"
    );
}

#[tokio::test]
async fn with_the_switch_off_a_direct_tool_call_is_refused_and_runs_nothing() {
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;
    let llm = mock_model(vec![]).await;
    let mut env = std::collections::HashMap::new();
    env.insert("LLM_URL".to_owned(), llm.uri());
    env.insert("CH_URL".to_owned(), ch.uri());
    env.insert("AI_SQL_HELP".to_owned(), "false".to_owned());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/tool")
                .header("content-type", "application/json")
                .header("cookie", &cookie)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "tool": "propose_sql",
                        "args": { "sql": "SELECT 1", "reason": "why", "engine": "clickhouse" },
                        "mode": "ask",
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
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let text = body.to_string();
    assert!(text.contains("SQL help is switched off"), "{text}");
    assert!(
        !text.contains("\"proposed\""),
        "a refused call proposes nothing: {text}"
    );
    assert!(
        statements(&ch)
            .await
            .iter()
            .all(|s| !s.contains("EXPLAIN PLAN")),
        "the check never reached ClickHouse"
    );
}

/// The chat with no allowlist is offered the same tools as before this
/// feature, and its system prompt is the same bytes as before. The values are
/// pinned here, not computed from the code under test: a change to either is
/// an intended change, and this fixture is then updated by hand.
const OFFERED_WITHOUT_ALLOWLIST: &[&str] = &[
    "ask_user",
    "describe_dataset",
    "describe_mart",
    "get_cdc_health",
    "get_lineage",
    "get_quality",
    "lakehouse_overview",
    "list_alert_rules",
    "list_boards",
    "list_connectors",
    "list_datasets",
    "list_pipelines",
    "run_sql",
];
const SYSTEM_SHA256_WITHOUT_ALLOWLIST: &str =
    "d451c89f816618dd5223f467d03b0c93835a32070367112ba30df4ead5c83046";

fn sha256_hex(text: &str) -> String {
    use std::fmt::Write as _;
    Sha256::digest(text.as_bytes())
        .iter()
        .fold(String::new(), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        })
}

#[tokio::test]
async fn a_chat_with_no_allowlist_is_offered_today_s_tools_and_today_s_prompt() {
    for switch in ["true", "false"] {
        let llm = mock_model(vec![model_round(Some("Hi."), &[])]).await;
        let ch = mock_clickhouse(ResponseTemplate::new(200)).await;

        let (status, _) = chat(&llm, &ch, switch, None, "berapa tamu bulan ini?").await;

        assert_eq!(status, StatusCode::OK, "switch {switch}");
        let (names, system) = first_model_request(&llm).await;
        let mut sorted = names.clone();
        sorted.sort();
        let mut expected: Vec<&str> = OFFERED_WITHOUT_ALLOWLIST.to_vec();
        expected.sort_unstable();
        assert_eq!(sorted, expected, "switch {switch}: offered tools");
        assert_eq!(
            sha256_hex(&system),
            SYSTEM_SHA256_WITHOUT_ALLOWLIST,
            "switch {switch}: the system prompt's bytes changed"
        );
    }
}
