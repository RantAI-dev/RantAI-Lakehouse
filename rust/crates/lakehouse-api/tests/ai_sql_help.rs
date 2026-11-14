//! Query Studio's SQL help (plan 15): the `propose_sql` tool, its rules, and
//! the switch `AI_SQL_HELP`.
//!
//! The model is mocked round by round, as in `ai_ask_back_turn.rs`, and
//! `ClickHouse` is mocked by statement: the proposal's `EXPLAIN PLAN` gets the
//! engine's answer, and anything else that reaches `ClickHouse` is recorded so
//! a test can assert that no statement other than an `EXPLAIN` was sent.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{
    TestApp, create_assistant_principal, session_cookie_for_seeded_user, session_cookie_for_user,
    spin_up_with_env,
};

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

/// Posts `body` to `/api/ai/chat` with the switch and returns the status and
/// the raw response bytes.
async fn post_chat(
    llm: &MockServer,
    ch: &MockServer,
    switch: &str,
    body: &Value,
) -> (StatusCode, bytes::Bytes) {
    let mut env = std::collections::HashMap::new();
    env.insert("LLM_URL".to_owned(), llm.uri());
    env.insert("CH_URL".to_owned(), ch.uri());
    env.insert("AI_SQL_HELP".to_owned(), switch.to_owned());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/chat")
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
    (status, bytes)
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
    let mut body = json!({ "messages": [{ "role": "user", "content": question }] });
    if let Some(tools) = tools {
        body["tools"] = tools;
    }
    let (status, bytes) = post_chat(llm, ch, switch, &body).await;
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

/// The console's Fix message for `SELECT no_such_col FROM silver.orders` on
/// `clickhouse`, template `proposeFix` in src/services/clients/queries.ts.
/// The server matches the leading sentence only.
const FIX_MESSAGE: &str = "Why does this query fail? Fix it.\n\nEngine: clickhouse\nSQL:\nSELECT no_such_col FROM silver.orders\nError:\nUNKNOWN_IDENTIFIER";

const WRONG_PROPOSAL: &str = r#"{"sql":"SELECT no_such_col FROM silver.orders","reason":"Used the wrong column.","engine":"clickhouse"}"#;
const RIGHT_PROPOSAL: &str = r#"{"sql":"SELECT order_id FROM silver.orders","reason":"The column is order_id, not no_such_col.","engine":"clickhouse"}"#;

/// The fixed sentence `ClickHouse`'s code 47 (`UNKNOWN_IDENTIFIER`) gets, and
/// so the browser's copy of a statement failure (owner 2026-11-12,
/// LH-QS-2-08): the person reads this sentence and a reference, never the
/// engine's own text.
const COLUMN_DOES_NOT_EXIST: &str = "This query names a column that does not exist.";

/// The engine's own answer for a statement that names a column that does not
/// exist: code 47, `DB::Exception`, the identifier and the server version.
const CLICKHOUSE_UNKNOWN_IDENTIFIER: &str = "Code: 47. DB::Exception: Unknown expression identifier 'pengunjungg' in scope. (UNKNOWN_IDENTIFIER) (version 26.8.1)";

/// A `run_sql` call whose statement the engine refuses.
const RUN_SQL_FAILING: &str = r#"{"sql":"SELECT pengunjungg FROM silver.orders"}"#;

/// `ClickHouse` for a `run_sql` whose statement the engine answers with
/// `answer`: the dry run (`EXPLAIN AST`) confirms a `SELECT`, the statement
/// itself gets the engine's own answer.
///
/// The mocks are mounted in that order because `wiremock` answers with the
/// FIRST matching mock in mount order: the dry run's body carries the
/// statement too, so its mock has to come first to keep the statement's own
/// answer out of the dry run.
async fn mock_clickhouse_refusing(failing: &str, answer: ResponseTemplate) -> MockServer {
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
        .and(body_string_contains(failing))
        .respond_with(answer)
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [{"name": "n", "type": "UInt64"}],
            "data": [{"n": 5}],
            "rows": 1,
        })))
        .mount(&ch)
        .await;
    ch
}

/// The tool result the model read in the round at `index` (`0` is the first
/// round), as the model received it: the tool's own result, with the
/// model's copy of the failure in `error`.
async fn model_tool_result(llm: &MockServer, index: usize) -> Value {
    let requests = llm.received_requests().await.expect("recording is on");
    let request: Value =
        serde_json::from_slice(&requests[index].body).expect("a JSON model request");
    let message = request["messages"]
        .as_array()
        .expect("messages are a list")
        .iter()
        .rev()
        .find(|m| m["role"] == "tool")
        .unwrap_or_else(|| panic!("the model read a tool result: {request}"));
    serde_json::from_str(message["content"].as_str().unwrap_or_default())
        .expect("the tool result is a JSON object")
}

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
    // LH-QS-2-08 (owner 2026-11-12): the person reads a fixed sentence and
    // a reference, never the engine's text. The engine's diagnosis is the
    // MODEL's copy, and it is in the model's next request.
    let error = trace[0]["result"]["error"].as_str().unwrap_or_default();
    let reference = error
        .strip_prefix(&format!("{COLUMN_DOES_NOT_EXIST} Reference: "))
        .unwrap_or_else(|| panic!("the fixed sentence and a reference: {error}"));
    assert!(!reference.is_empty(), "{error}");
    for engine_text in [
        "no_such_col",
        "DB::Exception",
        "Code:",
        "UNKNOWN_IDENTIFIER",
    ] {
        assert!(!error.contains(engine_text), "{engine_text} in {error}");
    }
    assert_eq!(trace[1]["ok"], true);
    assert_eq!(trace[1]["result"]["proposed"], json!(true));
    assert_eq!(
        body["answer"], "The column is order_id, not no_such_col.",
        "the accepted proposal's reason is the reply: {body}"
    );
    let model_calls = llm.received_requests().await.expect("recording").len();
    assert_eq!(model_calls, 2, "no round runs after the accepted proposal");
}

/// Plan 86, step 2 (b): the same check for `propose_sql` — the model's copy
/// keeps the engine's diagnosis and the SAME reference the person reads.
#[tokio::test]
async fn a_rejected_proposal_keeps_the_diagnosis_for_the_model_and_the_fixed_sentence_for_the_person()
 {
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "propose_sql", WRONG_PROPOSAL)]),
        model_round(Some("Explained."), &[]),
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
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&ch)
        .await;

    let (status, body) = chat(&llm, &ch, "true", Some(json!(["propose_sql"])), FIX_MESSAGE).await;

    assert_eq!(status, StatusCode::OK);
    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(trace.len(), 1, "one step, the rejected proposal: {trace:?}");
    let error = trace[0]["result"]["error"].as_str().unwrap_or_default();
    let reference = error
        .strip_prefix(&format!("{COLUMN_DOES_NOT_EXIST} Reference: "))
        .unwrap_or_else(|| panic!("the fixed sentence and a reference: {error}"));
    // The model reads the engine's diagnosis, with the SAME reference.
    let model = model_tool_result(&llm, 1).await;
    let model_error = model["error"].as_str().unwrap_or_default();
    assert!(
        model_error.contains("UNKNOWN_IDENTIFIER"),
        "the model reads the diagnosis: {model}"
    );
    assert!(
        model_error.contains(&format!(" Reference: {reference}")),
        "the model's copy carries the same reference: {model}"
    );
}

/// Plan 86, step 2 (a): `run_sql` whose statement the engine refuses. The
/// person reads the fixed sentence and the reference; the model keeps the
/// engine's diagnosis, byte for byte (SEC-11, the query-author exception).
#[tokio::test]
async fn a_failed_run_sql_shows_the_person_a_fixed_sentence_and_the_model_the_diagnosis() {
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "run_sql", RUN_SQL_FAILING)]),
        model_round(Some("The column is order_id."), &[]),
    ])
    .await;
    let ch = mock_clickhouse_refusing(
        "pengunjungg",
        ResponseTemplate::new(400).set_body_string(CLICKHOUSE_UNKNOWN_IDENTIFIER),
    )
    .await;

    let (status, body) = chat(
        &llm,
        &ch,
        "true",
        Some(json!(["run_sql"])),
        "berapa tamu bulan ini?",
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(trace.len(), 1, "one step, the failed run: {trace:?}");
    assert_eq!(trace[0]["tool"], "run_sql");
    assert_eq!(trace[0]["ok"], false);

    // The person's copy: the fixed sentence and the reference, and nothing of
    // the engine's own text.
    let error = trace[0]["result"]["error"].as_str().unwrap_or_default();
    let reference = error
        .strip_prefix(&format!("{COLUMN_DOES_NOT_EXIST} Reference: "))
        .unwrap_or_else(|| panic!("the fixed sentence and a reference: {error}"));
    assert!(!reference.is_empty(), "{error}");
    for engine_text in [
        "DB::Exception",
        "Code:",
        "pengunjungg",
        "UNKNOWN_IDENTIFIER",
    ] {
        assert!(!error.contains(engine_text), "{engine_text} in {error}");
    }

    // The model's copy: the engine's diagnosis and the SAME reference.
    let model = model_tool_result(&llm, 1).await;
    let model_error = model["error"].as_str().unwrap_or_default();
    assert!(
        model_error.contains("Unknown expression identifier 'pengunjungg' in scope."),
        "the model reads the engine's diagnosis: {model}"
    );
    assert!(
        model_error.contains(&format!(" Reference: {reference}")),
        "the model's copy carries the same reference: {model}"
    );
}

/// Plan 86, step 2 (d): an engine failure that is not the statement's own
/// (a proxy answering instead of the engine) keeps the same fixed message in
/// both copies, as today.
#[tokio::test]
async fn a_run_sql_that_fails_for_another_reason_gives_both_copies_the_same_text() {
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "run_sql", RUN_SQL_FAILING)]),
        model_round(Some("The engine is not answering."), &[]),
    ])
    .await;
    // A proxy's page, not the engine answering that the STATEMENT is wrong:
    // the text is this repository's own, so there is nothing to keep from the
    // engine and both copies are the same.
    let ch = mock_clickhouse_refusing(
        "pengunjungg",
        ResponseTemplate::new(502).set_body_string("<html><body>502 Bad Gateway</body></html>"),
    )
    .await;

    let (status, body) = chat(
        &llm,
        &ch,
        "true",
        Some(json!(["run_sql"])),
        "berapa tamu bulan ini?",
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(trace.len(), 1, "one step, the failed run: {trace:?}");
    let error = trace[0]["result"]["error"].as_str().unwrap_or_default();
    assert!(
        error.starts_with("The database request failed. Reference: "),
        "the fixed message and a reference: {error}"
    );
    let model = model_tool_result(&llm, 1).await;
    let model_error = model["error"].as_str().unwrap_or_default();
    assert_eq!(
        error, model_error,
        "both copies are the same fixed message: {model}"
    );
}

/// Plan 86, step 2 (e): the key the browser's copy travels under is the
/// server's alone. It reaches neither the browser nor the model.
#[tokio::test]
async fn the_server_only_key_reaches_neither_the_browser_nor_the_model() {
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "propose_sql", WRONG_PROPOSAL)]),
        model_round(Some("Explained."), &[]),
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
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&ch)
        .await;

    let (status, body) = chat(&llm, &ch, "true", Some(json!(["propose_sql"])), FIX_MESSAGE).await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        !body.to_string().contains("_browserError"),
        "the key never reaches the browser: {body}"
    );
    for (index, request) in llm
        .received_requests()
        .await
        .expect("recording")
        .iter()
        .enumerate()
    {
        let text = String::from_utf8_lossy(&request.body);
        assert!(
            !text.contains("_browserError"),
            "the key never reaches the model, round {index}: {text}"
        );
    }
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
//
// Updated by hand in the merge of phase 2 into main's `propose_sql` work: `BI-2`
// (plan 2026-10-11-bi-2-reusable-metrics.md, section 7) puts `list_metrics`,
// `describe_metric` and `query_metric` in `ALWAYS` on purpose, so a question for
// a number is answered from a named metric. `propose_sql` itself is still
// absent: only an allowlist naming it offers it.
//
// Updated by hand when AI-1/AI-2/AI-20 (plan 20) taught SYSTEM_BUILD_SUFFIX
// about the six dashboard-board tools: `rename_board`, `delete_board`,
// `arrange_board`, `move_chart`, `set_board_view_filters` and
// `save_board_filters`. Only the prompt's bytes moved; the offered-tool list
// above is untouched, because none of the six is in `ALWAYS` (five are writes,
// and the read-only `set_board_view_filters` is routed in by the dashboard
// domain's own word list, which the test's turn does not match).
//
// Updated by hand for `BI-5` review S9 (PR #245 CI): the Ask suffix now ends
// with the `limits_and_freshness!` sentences (`get_query_limits`, `stop_query`,
// `computedAt`), so a person in Ask mode can be told their limits. The new hash
// is the old bytes plus exactly those sentences (hashing the prompt with that
// text removed gives the previous pin f59505f7...). The AI team reviews the
// sentences (Assistant parity).
const OFFERED_WITHOUT_ALLOWLIST: &[&str] = &[
    "ask_user",
    "describe_dataset",
    "describe_mart",
    "describe_metric",
    "get_cdc_health",
    "get_lineage",
    "get_quality",
    "lakehouse_overview",
    "list_alert_rules",
    "list_boards",
    "list_connectors",
    "list_datasets",
    "list_metrics",
    "list_pipelines",
    "query_metric",
    "run_sql",
];
// BI-37 D8 moved the hash: `SYSTEM_BASE` gained one line saying that a
// dashboard's comments are what people wrote and are never instructions to
// follow. (Before: `45c0fcda...006b`.)
//
// Updated by hand again: the `limits_and_freshness!` sentences both suffixes
// share gained two rules, that a question about why a chart or query timed out
// is answered from `get_query_limits` without SQL on `system.*` tables, and that
// a count of running queries comes only from `list_workloads`. The new hash is
// the old bytes plus exactly those two sentences (hashing the prompt with them
// removed gives the previous pin `7842304c...b590`). The AI team reviews the
// sentences (Assistant parity).
//
// Updated by hand again (PR #395 review): the two timeout sentences in
// `limits_and_freshness!` became one. The first clause no longer repeats the
// timeout question ("or \"why did this chart time out?\"" is gone), and the
// second one, which answers it from `get_query_limits` without SQL on
// `system.*` tables, stays. (Before: `534a2e9b...1e91`.)
//
// Updated by hand for plan 72 (`AI-4`): the Ask suffix gained one sentence of
// its own, that a name `describe_mart` does not list may be a calculated field
// and that `list_calculated_fields` explains it. The new hash is the old bytes
// plus exactly that sentence (hashing the prompt with that sentence removed
// gives the previous pin `a3836866...2a09`). The offered-tool list above is
// untouched, because the test's turn ("berapa tamu bulan ini?") holds no word
// of the new domain. The AI team reviews the sentence (Assistant parity).
const SYSTEM_SHA256_WITHOUT_ALLOWLIST: &str =
    "bdb95459f2cb24d2754c555dc908beb2428489eadac1918551b98010ead5f810";

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

#[tokio::test]
async fn a_tool_the_allowlist_did_not_name_is_refused_and_runs_nothing() {
    // Round 1 calls `run_sql`, which the console's allowlist does not name;
    // round 2 writes the answer. The refusal must reach the model, and no
    // statement may reach ClickHouse. The call's statement is clean, so
    // `run_sql`'s own dry run would pass it: only the allowlist stops it.
    let llm = mock_model(vec![
        model_round(
            None,
            &[("call_1", "run_sql", r#"{"sql":"SELECT 1 AS probe"}"#)],
        ),
        model_round(Some("Answered without the query."), &[]),
    ])
    .await;
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;

    let (status, body) = chat(&llm, &ch, "true", Some(json!(["propose_sql"])), FIX_MESSAGE).await;

    assert_eq!(status, StatusCode::OK);
    // The DATA MAP's `system.*` reads are sent by the prompt on every turn, so
    // the check is on the model's own statement only. It comes first: with the
    // refusal removed, the statement is what must fail this test.
    let sent = statements(&ch).await;
    assert!(
        sent.iter().all(|s| !s.contains("AS probe")),
        "the refused query reached ClickHouse: {sent:?}"
    );
    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(trace.len(), 1, "one refused call: {trace:?}");
    assert_eq!(trace[0]["tool"], "run_sql");
    assert_eq!(trace[0]["ok"], false, "{trace:?}");
    assert_eq!(
        trace[0]["result"]["error"], "run_sql is not available in this chat. Answer without it.",
        "{trace:?}"
    );
    assert_eq!(body["answer"], "Answered without the query.", "{body}");
}

#[tokio::test]
async fn query_studio_s_question_box_is_held_to_its_four_tools() {
    // The console's question box sends these four names. `list_charts` is a
    // read tool the box did not name, so the loop refuses it.
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "list_charts", "{}")]),
        model_round(Some("Answered without the chart list."), &[]),
    ])
    .await;
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;
    let allow = json!([
        "run_sql",
        "list_datasets",
        "describe_dataset",
        "lakehouse_overview"
    ]);

    let (status, body) = chat(&llm, &ch, "true", Some(allow), "Apa saja grafik yang ada?").await;

    assert_eq!(status, StatusCode::OK);
    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(trace.len(), 1, "{trace:?}");
    assert_eq!(trace[0]["tool"], "list_charts");
    assert_eq!(trace[0]["ok"], false, "{trace:?}");
}

#[tokio::test]
async fn a_chat_with_no_allowlist_still_runs_a_tool_the_turn_selection_left_out() {
    // No `tools` in the request: `list_charts` is outside the per-turn offer
    // for this message, but plain chat dispatches as it did before plan 17.
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "list_charts", "{}")]),
        model_round(Some("Done."), &[]),
    ])
    .await;
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;

    let (status, body) = chat(&llm, &ch, "true", None, "berapa tamu bulan ini?").await;

    assert_eq!(status, StatusCode::OK);
    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(trace.len(), 1, "{trace:?}");
    assert_eq!(trace[0]["tool"], "list_charts");
    assert_eq!(trace[0]["ok"], true, "{trace:?}");
}

#[tokio::test]
async fn with_the_switch_off_a_proposal_the_model_makes_anyway_is_refused() {
    // The switch removes `propose_sql` from the offer, but a model can still
    // name it. The call must end `ok: false` and reach ClickHouse as nothing.
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "propose_sql", RIGHT_PROPOSAL)]),
        model_round(Some("Explained without a proposal."), &[]),
    ])
    .await;
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;

    let (status, body) = chat(
        &llm,
        &ch,
        "false",
        Some(json!(["propose_sql"])),
        FIX_MESSAGE,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(trace.len(), 1, "{trace:?}");
    assert_eq!(trace[0]["tool"], "propose_sql");
    assert_eq!(trace[0]["ok"], false, "{trace:?}");
    assert!(
        statements(&ch)
            .await
            .iter()
            .all(|s| !s.contains("EXPLAIN PLAN") && !s.contains("order_id")),
        "the proposal never reached ClickHouse: {:?}",
        statements(&ch).await
    );
}

/// The console's Explain message for `SELECT 1 AS probe` on `clickhouse`,
/// template `explainSql` in src/services/clients/queries.ts. The server
/// matches the leading sentence only.
const EXPLAIN_MESSAGE: &str =
    "Explain what this query does.\n\nEngine: clickhouse\nSQL:\nSELECT 1 AS probe";

#[tokio::test]
async fn an_explain_turn_offers_no_tool_and_keeps_the_sql_help_rules() {
    let llm = mock_model(vec![model_round(Some("It returns one row."), &[])]).await;
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;

    let (status, _) = chat(
        &llm,
        &ch,
        "true",
        Some(json!(["propose_sql"])),
        EXPLAIN_MESSAGE,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let (names, system) = first_model_request(&llm).await;
    assert!(
        names.is_empty(),
        "an Explain turn offers no tool: {names:?}"
    );
    assert!(
        system.contains("SQL HELP"),
        "the rules stay for the Explain turn: {system}"
    );
}

#[tokio::test]
async fn a_proposal_in_an_explain_turn_is_refused_and_the_explanation_is_the_answer() {
    // The staging failure: the model proposes a statement during an Explain.
    // RIGHT_PROPOSAL would pass the check, so only the offer decides here.
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "propose_sql", RIGHT_PROPOSAL)]),
        model_round(Some("It returns one row."), &[]),
    ])
    .await;
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;

    let (status, body) = chat(
        &llm,
        &ch,
        "true",
        Some(json!(["propose_sql"])),
        EXPLAIN_MESSAGE,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    // The check first, as in T1: the proposal's SQL must not reach ClickHouse.
    let sent = statements(&ch).await;
    assert!(
        sent.iter()
            .all(|s| !s.contains("EXPLAIN PLAN") && !s.contains("order_id")),
        "the proposal reached ClickHouse: {sent:?}"
    );
    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(trace.len(), 1, "{trace:?}");
    assert_eq!(trace[0]["tool"], "propose_sql");
    assert_eq!(trace[0]["ok"], false, "{trace:?}");
    assert_eq!(
        trace[0]["result"]["error"],
        "propose_sql is not available in this chat. Answer without it.",
        "{trace:?}"
    );
    assert!(
        trace
            .iter()
            .all(|step| step["result"]["proposed"] != json!(true)),
        "no step proposes: {trace:?}"
    );
    assert_eq!(body["answer"], "It returns one row.", "{body}");
}

#[tokio::test]
async fn a_query_in_an_explain_turn_is_refused_and_runs_nothing() {
    let llm = mock_model(vec![
        model_round(
            None,
            &[("call_1", "run_sql", r#"{"sql":"SELECT 1 AS probe"}"#)],
        ),
        model_round(Some("It returns one row."), &[]),
    ])
    .await;
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;

    let (status, body) = chat(
        &llm,
        &ch,
        "true",
        Some(json!(["propose_sql"])),
        EXPLAIN_MESSAGE,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let sent = statements(&ch).await;
    assert!(
        sent.iter().all(|s| !s.contains("AS probe")),
        "the query reached ClickHouse: {sent:?}"
    );
    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(trace.len(), 1, "{trace:?}");
    assert_eq!(trace[0]["tool"], "run_sql");
    assert_eq!(trace[0]["ok"], false, "{trace:?}");
}

#[tokio::test]
async fn the_explain_sentence_inside_a_comment_does_not_end_the_offer() {
    // The sentence is matched at the start of the message only. A Fix message
    // whose statement holds the sentence in a comment keeps the tool.
    let fix_with_comment = "Why does this query fail? Fix it.\n\nEngine: clickhouse\nSQL:\nSELECT 1 /* Explain what this query does. */ AS probe\nError:\nUNKNOWN_IDENTIFIER";
    let llm = mock_model(vec![model_round(Some("Explained."), &[])]).await;
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;

    let (status, _) = chat(
        &llm,
        &ch,
        "true",
        Some(json!(["propose_sql"])),
        fix_with_comment,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let (names, _) = first_model_request(&llm).await;
    assert!(names.contains(&"propose_sql".to_owned()), "{names:?}");
}

/// Sends one chat turn as a fresh user who holds `permissions` besides
/// `assistant:use` (every request here is to `/api/ai/chat`), and
/// returns the system prompt the model received and the statements that
/// reached `ClickHouse`.
async fn system_prompt_for_user_holding(permissions: &str) -> (String, Vec<String>) {
    let llm = mock_model(vec![model_round(Some("Hi."), &[])]).await;
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;
    let mut env = std::collections::HashMap::new();
    env.insert("LLM_URL".to_owned(), llm.uri());
    env.insert("CH_URL".to_owned(), ch.uri());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let user = create_assistant_principal(&pool, permissions).await;
    let cookie = session_cookie_for_user(&pool, user).await;
    let body = json!({ "messages": [{ "role": "user", "content": "berapa tamu bulan ini?" }] });
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
    assert_eq!(resp.status(), StatusCode::OK);
    let (_, system) = first_model_request(&llm).await;
    (system, statements(&ch).await)
}

/// The refusal text a caller without `catalog:read` gets in place of the
/// DATA MAP (SEC-24).
const MAP_NEEDS_CATALOG_READ: &str = "(withheld: reading the data map needs catalog:read)";

#[tokio::test]
async fn a_user_without_catalog_read_gets_no_data_map_and_the_warehouse_is_not_asked() {
    let (system, sent) = system_prompt_for_user_holding("dashboard:read").await;

    assert!(system.contains(MAP_NEEDS_CATALOG_READ), "{system}");
    assert!(
        sent.iter().all(|s| !s.contains("system.")),
        "the DATA MAP was built for a caller who may not read it: {sent:?}"
    );
}

#[tokio::test]
async fn a_user_with_catalog_read_is_not_refused_the_data_map_for_a_missing_permission() {
    // The seeded test database has several tenants, so the shared-catalog
    // rule may still withhold the map with its own reason. The check here is
    // that the permission is not what refuses this caller.
    let (system, _) = system_prompt_for_user_holding("catalog:read").await;

    assert!(!system.contains(MAP_NEEDS_CATALOG_READ), "{system}");
}

// ── `AI-17` (plan 80): the server-side script gate ─────────────────────
//
// On staging the model answered the Latin-script question "Berapa total
// kunjungan?" with text holding 吸引力, ignoring the prompt's "never in
// Chinese" line, so the server now asks once — with no tools — for a
// rewrite when the latest user message is in Latin script and the answer
// holds a CJK character. These tests count the model's requests, so the
// mock answers carry no digits: a bare figure would trip the citation
// repair (WS7), which adds its own round and would mask the rewrite under
// test.

#[tokio::test]
async fn an_answer_with_a_cjk_character_to_a_latin_question_is_asked_once_for_a_rewrite_and_the_clean_rewrite_is_the_reply()
 {
    let llm = mock_model(vec![
        model_round(Some("Total kunjungan adalah 吸引力."), &[]),
        model_round(Some("Total kunjungan tercatat."), &[]),
    ])
    .await;
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;

    let (status, body) = chat(&llm, &ch, "true", None, "Berapa total kunjungan?").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["answer"], "Total kunjungan tercatat.",
        "the clean rewrite is the reply: {body}"
    );
    let model_calls = llm.received_requests().await.expect("recording").len();
    assert_eq!(
        model_calls, 2,
        "exactly one rewrite round was added: {model_calls}"
    );
}

#[tokio::test]
async fn a_rewrite_that_still_holds_a_cjk_character_is_discarded_and_the_first_answer_is_the_reply()
{
    let llm = mock_model(vec![
        model_round(Some("Total kunjungan adalah 吸引力."), &[]),
        model_round(Some("Jawabannya tetap 吸引力."), &[]),
    ])
    .await;
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;

    let (status, body) = chat(&llm, &ch, "true", None, "Berapa total kunjungan?").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["answer"], "Total kunjungan adalah 吸引力.",
        "the first answer is kept when the rewrite is not clean: {body}"
    );
    let model_calls = llm.received_requests().await.expect("recording").len();
    assert_eq!(
        model_calls, 2,
        "no round runs after the discarded rewrite: {model_calls}"
    );
}

#[tokio::test]
async fn a_latin_answer_to_a_latin_question_sends_no_rewrite_round() {
    let llm = mock_model(vec![model_round(Some("Total kunjungan tercatat."), &[])]).await;
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;

    let (status, body) = chat(&llm, &ch, "true", None, "Berapa total kunjungan?").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["answer"], "Total kunjungan tercatat.", "{body}");
    let model_calls = llm.received_requests().await.expect("recording").len();
    assert_eq!(
        model_calls, 1,
        "a clean Latin answer is not rewritten: {model_calls}"
    );
}

#[tokio::test]
async fn a_chinese_answer_to_a_chinese_question_sends_no_rewrite_round() {
    let llm = mock_model(vec![model_round(Some("总共四次。"), &[])]).await;
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;

    let (status, body) = chat(&llm, &ch, "true", None, "总共多少?").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["answer"], "总共四次。", "{body}");
    let model_calls = llm.received_requests().await.expect("recording").len();
    assert_eq!(
        model_calls, 1,
        "a question in another script is not checked: {model_calls}"
    );
}

/// The `messages` array of the model's `n`th request (0-based), without the
/// system prompt, as roles and contents, so a test can see what the model was
/// shown in a later round.
async fn model_request_messages(llm: &MockServer, n: usize) -> Vec<(String, String)> {
    let requests = llm.received_requests().await.expect("recording is on");
    let request: Value = serde_json::from_slice(&requests[n].body).expect("a JSON request");
    request["messages"]
        .as_array()
        .expect("a messages array")
        .iter()
        .skip(1)
        .map(|m| {
            (
                m["role"].as_str().unwrap_or_default().to_owned(),
                m["content"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

/// `AI-17` review fix (BLOCKER 1): a clean rewrite that then trips the
/// citation repair (the figure is in no tool result) must reach the repair
/// round as the model's own assistant message. Before the fix the rewrite's
/// reply was never pushed, so the repair request followed the rewrite
/// request, the model repaired its CJK answer, and the repaired answer was
/// returned with no CJK check.
#[tokio::test]
async fn a_clean_rewrite_the_citation_check_flags_is_shown_to_the_model_as_its_own_message_in_the_repair_round()
 {
    let llm = mock_model(vec![
        model_round(Some("Total kunjungan adalah 吸引力."), &[]),
        model_round(Some("Total kunjungan adalah 48213."), &[]),
        model_round(Some("Total kunjungan tercatat."), &[]),
    ])
    .await;
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;

    let (status, body) = chat(&llm, &ch, "true", None, "Berapa total kunjungan?").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["answer"], "Total kunjungan tercatat.",
        "the repaired answer is the reply: {body}"
    );
    let model_calls = llm.received_requests().await.expect("recording").len();
    assert_eq!(
        model_calls, 3,
        "the answer, one rewrite, one repair: {model_calls}"
    );
    let repair = model_request_messages(&llm, 2).await;
    let roles: Vec<&str> = repair.iter().map(|(role, _)| role.as_str()).collect();
    assert_eq!(
        roles,
        ["user", "assistant", "user", "assistant", "user"],
        "question, answer, rewrite request, rewrite, repair request: {repair:?}"
    );
    assert!(
        repair[3].1.contains("Total kunjungan adalah 48213."),
        "the repair round shows the model its rewrite: {repair:?}"
    );
}

/// `AI-17` review fix (BLOCKER 1): a rewrite that is discarded leaves the
/// conversation as it was before the rewrite request, so a later repair
/// round does not see two user messages in a row.
#[tokio::test]
async fn a_discarded_rewrite_leaves_no_trace_in_the_conversation_the_repair_round_sees() {
    let llm = mock_model(vec![
        model_round(Some("Total kunjungan adalah 48213 吸引力."), &[]),
        model_round(Some("Jawabannya tetap 吸引力."), &[]),
        model_round(Some("Total kunjungan tercatat."), &[]),
    ])
    .await;
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;

    let (status, body) = chat(&llm, &ch, "true", None, "Berapa total kunjungan?").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["answer"], "Total kunjungan tercatat.", "{body}");
    let repair = model_request_messages(&llm, 2).await;
    let roles: Vec<&str> = repair.iter().map(|(role, _)| role.as_str()).collect();
    assert_eq!(
        roles,
        ["user", "assistant", "user"],
        "question, answer, repair request: {repair:?}"
    );
}

/// `AI-17` review fix (BLOCKER 2): the iteration-budget path runs the same
/// check. `MAX_ITER` (8) rounds that call a tool, then a final answer with a
/// CJK character, then the rewrite: ten requests in all.
#[tokio::test]
async fn a_final_answer_after_the_tool_call_limit_with_a_cjk_character_is_replaced_by_its_clean_rewrite()
 {
    let mut rounds: Vec<Value> = (1..=8)
        .map(|n| model_round(None, &[(&format!("call_{n}"), "no_such_tool", "{}")]))
        .collect();
    rounds.push(model_round(Some("Total kunjungan adalah 吸引力."), &[]));
    rounds.push(model_round(Some("Total kunjungan tercatat."), &[]));
    let llm = mock_model(rounds).await;
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;

    let (status, body) = chat(&llm, &ch, "true", None, "Berapa total kunjungan?").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["answer"], "Total kunjungan tercatat.",
        "the clean rewrite is the reply: {body}"
    );
    assert_eq!(body["note"], "tool-call limit reached", "{body}");
    let model_calls = llm.received_requests().await.expect("recording").len();
    assert_eq!(
        model_calls, 10,
        "eight tool rounds, the final answer, one rewrite: {model_calls}"
    );
    // `AI-17` review fix round 2 (BLOCKER): the rewrite request follows the
    // answer it asks about. Before the fix the answer was never pushed, so
    // the request came right after "Give your final answer now" and the model
    // was asked to rewrite an answer it had not seen.
    let rewrite_round = model_request_messages(&llm, 9).await;
    let tail: Vec<(&str, &str)> = rewrite_round[rewrite_round.len() - 3..]
        .iter()
        .map(|(role, content)| (role.as_str(), content.as_str()))
        .collect();
    assert_eq!(
        tail.iter().map(|(role, _)| *role).collect::<Vec<_>>(),
        ["user", "assistant", "user"],
        "final-answer request, the CJK answer, the rewrite request: {tail:?}"
    );
    assert_eq!(tail[1].1, "Total kunjungan adalah 吸引力.", "{tail:?}");
}

/// `AI-17` review fix round 2 (optional item): the rewrite round offers no
/// tools, so a structured tool call in its reply has no tool result to
/// follow it. The message kept for the repair round carries none.
#[tokio::test]
async fn a_clean_rewrite_that_carries_a_tool_call_is_kept_for_the_repair_round_without_it() {
    let llm = mock_model(vec![
        model_round(Some("Total kunjungan adalah 吸引力."), &[]),
        model_round(
            Some("Total kunjungan adalah 48213."),
            &[("call_x", "no_such_tool", "{}")],
        ),
        model_round(Some("Total kunjungan tercatat."), &[]),
    ])
    .await;
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;

    let (status, body) = chat(&llm, &ch, "true", None, "Berapa total kunjungan?").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["answer"], "Total kunjungan tercatat.", "{body}");
    let requests = llm.received_requests().await.expect("recording is on");
    assert_eq!(requests.len(), 3, "answer, rewrite, repair");
    let repair: Value = serde_json::from_slice(&requests[2].body).expect("a JSON request");
    let kept = repair["messages"]
        .as_array()
        .expect("a messages array")
        .iter()
        .find(|m| {
            m["content"]
                .as_str()
                .is_some_and(|c| c.contains("Total kunjungan adalah 48213."))
        })
        .expect("the rewrite is in the repair round");
    assert!(
        kept.get("tool_calls").is_none_or(Value::is_null),
        "the kept rewrite has no tool call: {kept}"
    );
}

/// `AI-17` review fix (SHOULD-FIX 2): a streaming client clears its draft
/// only on a `status` or `tool` event, so the rewrite round sends a status
/// before the model writes, or its deltas would be appended to the CJK
/// answer's text.
#[tokio::test]
async fn a_streamed_rewrite_round_announces_itself_with_a_status_event_before_its_deltas() {
    let llm = mock_model(vec![
        model_round(Some("Total kunjungan adalah 吸引力."), &[]),
        model_round(Some("Total kunjungan tercatat."), &[]),
    ])
    .await;
    let ch = mock_clickhouse(ResponseTemplate::new(200)).await;
    let body = json!({
        "stream": true,
        "messages": [{ "role": "user", "content": "Berapa total kunjungan?" }],
    });

    let (status, bytes) = post_chat(&llm, &ch, "true", &body).await;

    assert_eq!(status, StatusCode::OK);
    let events: Vec<Value> = String::from_utf8_lossy(&bytes)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("every line is one JSON event"))
        .collect();
    let rewrite_delta = events
        .iter()
        .position(|e| e["type"] == "delta" && e["text"] == "Total kunjungan tercatat.")
        .unwrap_or_else(|| panic!("the rewrite streams as a delta: {events:?}"));
    let status_before = events[..rewrite_delta]
        .iter()
        .rposition(|e| e["type"] == "status");
    let first_answer_delta = events
        .iter()
        .position(|e| e["type"] == "delta")
        .expect("the first answer streams");
    assert!(
        status_before.is_some_and(|s| s > first_answer_delta),
        "a status event sits between the first answer's text and the rewrite's: {events:?}"
    );
    assert_eq!(
        events.last().expect("events")["body"]["answer"],
        "Total kunjungan tercatat.",
        "{events:?}"
    );
}
