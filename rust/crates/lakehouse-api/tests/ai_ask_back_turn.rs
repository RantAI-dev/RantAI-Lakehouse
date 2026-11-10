//! The chat asks or answers in one turn, never both (plan 14, AI-17).
//!
//! `run_chat` enforces it, not the prompt: an accepted `ask_user` ends the
//! turn with its question as the answer and runs nothing after it, and an
//! `ask_user` that comes after a tool that returned data in the same turn is
//! refused with a result the model reads — a lookup that only found which
//! metrics or tables exist is not a query, and does not make the ask late
//! (`AI-17`). The one exception is an `ask_user` right
//! after another tool's call failed, because a refusal just said the call
//! cannot be built as asked and the person is the only one who can choose.
//! The mocked model here answers each round with plain JSON, so the
//! non-streamed `/api/ai/chat` path drives the same loop the streamed path
//! does.

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
/// A `create_chart` call. The turn is in Ask mode, so the gate refuses it
/// (a result with an `error` key), which stands for any refused or failed
/// call without mocking a tool's own refusal text.
const REFUSED_CALL: &str = "{}";

/// The refusal a late `ask_user` gets. Its wording is part of the contract
/// the model reads, so the tests pin it.
const LATE_ASK_REFUSAL: &str = "ask_user is refused: a turn may ask only before \
it runs a query, and this turn has run one. Do not apologise and do not \
mention this refusal. Answer now from the results above with the reading \
you used, and say in one sentence which reading it is. Do not end with a \
question: if another reading is possible, name it as a statement. Reply in \
the language of the user's question.";

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

/// The dry-run mock every test's `ClickHouse` starts with: a body carrying
/// `EXPLAIN AST` gets that statement's text dump of a `SELECT`
/// (`dry_run_sql` reads its first line).
async fn mount_dry_run_mock(ch: &MockServer) {
    Mock::given(method("POST"))
        .and(body_string_contains("EXPLAIN AST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("SelectWithUnionQuery (children 1)\n ExpressionList ...\n"),
        )
        .mount(ch)
        .await;
}

/// The catch-all every test's `ClickHouse` falls back to: any POST whose
/// body carries no `EXPLAIN AST` gets one row, `{"n": 5}`. Mount it LAST:
/// `wiremock` answers with the first matching mock in mount order, so the
/// catch-all only serves what nothing more specific matched.
async fn mount_catch_all_mock(ch: &MockServer) {
    Mock::given(method("POST"))
        .and(BodyDoesNotContain("EXPLAIN AST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [{"name": "n", "type": "UInt64"}],
            "data": [{"n": 5}],
            "rows": 1,
        })))
        .mount(ch)
        .await;
}

/// `ClickHouse` for `run_sql`: the dry run gets `EXPLAIN AST`'s text dump
/// of a `SELECT` (`dry_run_sql` reads its first line), the real query gets
/// one row.
async fn mock_clickhouse() -> MockServer {
    let ch = MockServer::start().await;
    mount_dry_run_mock(&ch).await;
    mount_catch_all_mock(&ch).await;
    ch
}

/// `ClickHouse` for the metric lookups: `console.bi_metric` holds one live
/// `mart` metric, so `describe_metric` finds it and `query_metric` reads its
/// value; the catch-all answers everything else. The specific mocks are
/// mounted BEFORE the catch-all (`wiremock` takes the first match in mount
/// order). The body details that matter beyond "200 OK": every mock's
/// envelope `meta` entry carries BOTH `name` and `type` — `ChResult` fails
/// to parse otherwise and reads as an EMPTY result, without an error — the
/// metric row needs `source_kind: "mart"` plus `formula`, what
/// `row_to_metric` keeps; the mart's existence count comes back as the JSON
/// STRING `"1"`, since `validated_mart_columns` parses `Value::as_str` and
/// a numeric `1` would parse as 0, "Gold mart not found"; and
/// `system.columns` returns the column the metric's `formula` names, so the
/// value query compiles.
async fn mock_clickhouse_with_metric() -> MockServer {
    let ch = MockServer::start().await;
    mount_dry_run_mock(&ch).await;
    Mock::given(method("POST"))
        .and(body_string_contains("console.bi_metric"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [
                {"name": "id", "type": "String"}, {"name": "name", "type": "String"},
                {"name": "source_kind", "type": "String"}, {"name": "source_id", "type": "String"},
                {"name": "formula", "type": "String"},
            ],
            "data": [{
                "id": "metric_kunjungan",
                "name": "kunjungan",
                "source_kind": "mart",
                "source_id": "kunjungan_harian",
                "formula": "sum(total)",
            }],
            "rows": 1,
        })))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains(
            "FROM system.tables WHERE database='serving'",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [{"name": "n", "type": "String"}],
            "data": [{"n": "1"}],
            "rows": 1,
        })))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains(
            "FROM system.columns WHERE database='serving'",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [{"name": "name", "type": "String"}, {"name": "type", "type": "String"}],
            "data": [{"name": "total", "type": "UInt64"}],
            "rows": 1,
        })))
        .mount(&ch)
        .await;
    mount_catch_all_mock(&ch).await;
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

#[tokio::test]
async fn an_ask_right_after_a_refused_call_is_accepted_even_after_a_query() {
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "run_sql", RUN_SQL)]),
        model_round(None, &[("call_2", "create_chart", REFUSED_CALL)]),
        model_round(None, &[("call_3", "ask_user", ASK_TWO)]),
    ])
    .await;
    let ch = mock_clickhouse().await;

    let body = chat(&llm, &ch, "berapa sales?").await;

    assert_eq!(
        body["answer"], "Which table do you mean by sales?",
        "the ask after the refused call ends the turn with the question: {body}"
    );
    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(trace.len(), 3, "query, refused call, ask: {trace:?}");
    assert_eq!(trace[0]["ok"], true);
    assert_eq!(trace[1]["tool"], "create_chart");
    assert_eq!(trace[1]["ok"], false);
    assert_eq!(trace[2]["tool"], "ask_user");
    assert_eq!(trace[2]["ok"], true);
    assert_eq!(trace[2]["result"]["asked"], true);
}

#[tokio::test]
async fn an_ask_after_two_queries_is_still_refused() {
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "run_sql", RUN_SQL)]),
        model_round(None, &[("call_2", "run_sql", RUN_SQL)]),
        model_round(None, &[("call_3", "ask_user", ASK_TWO)]),
        model_round(Some("Ada 5 baris."), &[]),
    ])
    .await;
    let ch = mock_clickhouse().await;

    let body = chat(&llm, &ch, "berapa sales?").await;

    assert_eq!(
        body["answer"], "Ada 5 baris.",
        "the refused ask left the turn open: {body}"
    );
    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(
        trace.len(),
        3,
        "two queries then the refused ask: {trace:?}"
    );
    assert_eq!(trace[2]["tool"], "ask_user");
    assert_eq!(trace[2]["ok"], false);
    assert_eq!(trace[2]["result"]["error"], LATE_ASK_REFUSAL);
}

#[tokio::test]
async fn only_the_latest_step_counts_so_an_ask_after_a_success_that_follows_a_refusal_is_refused() {
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "create_chart", REFUSED_CALL)]),
        model_round(None, &[("call_2", "run_sql", RUN_SQL)]),
        model_round(None, &[("call_3", "ask_user", ASK_TWO)]),
        model_round(Some("Ada 5 baris."), &[]),
    ])
    .await;
    let ch = mock_clickhouse().await;

    let body = chat(&llm, &ch, "berapa sales?").await;

    assert_eq!(
        body["answer"], "Ada 5 baris.",
        "the refused ask left the turn open: {body}"
    );
    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(
        trace.len(),
        3,
        "refused call, query, refused ask: {trace:?}"
    );
    assert_eq!(trace[0]["ok"], false);
    assert_eq!(trace[1]["ok"], true);
    assert_eq!(trace[2]["tool"], "ask_user");
    assert_eq!(trace[2]["result"]["error"], LATE_ASK_REFUSAL);
}

#[tokio::test]
async fn a_rejected_ask_does_not_open_the_way_for_a_second_ask_after_a_query() {
    // The rejected ask (one option) is itself a failed step. If it counted as
    // "the previous call failed", the second ask would pass although the turn
    // has run a query and no other tool failed since.
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "run_sql", RUN_SQL)]),
        model_round(None, &[("call_2", "create_chart", REFUSED_CALL)]),
        model_round(None, &[("call_3", "ask_user", ASK_ONE)]),
        model_round(None, &[("call_4", "ask_user", ASK_TWO)]),
        model_round(Some("Ada 5 baris."), &[]),
    ])
    .await;
    let ch = mock_clickhouse().await;

    let body = chat(&llm, &ch, "berapa sales?").await;

    assert_eq!(
        body["answer"], "Ada 5 baris.",
        "the second ask did not end the turn: {body}"
    );
    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(trace.len(), 4, "query, refused call, two asks: {trace:?}");
    assert_eq!(trace[2]["tool"], "ask_user");
    assert_eq!(trace[2]["ok"], false);
    assert_ne!(
        trace[2]["result"]["error"], LATE_ASK_REFUSAL,
        "the first ask follows a failed call, so the tool itself rejects it: {trace:?}"
    );
    assert_eq!(trace[3]["tool"], "ask_user");
    assert_eq!(trace[3]["result"]["error"], LATE_ASK_REFUSAL);
}

#[tokio::test]
async fn an_ask_after_a_successful_metric_lookup_is_accepted() {
    // `AI-17`: a lookup is not a query. `list_metrics` answers names and
    // labels, not data rows, so the ask after it is on time.
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "list_metrics", "{}")]),
        model_round(None, &[("call_2", "ask_user", ASK_TWO)]),
    ])
    .await;
    let ch = mock_clickhouse().await;

    let body = chat(&llm, &ch, "berapa sales?").await;

    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(trace.len(), 2, "the lookup then the ask: {trace:?}");
    assert_eq!(trace[0]["tool"], "list_metrics");
    assert_eq!(
        trace[0]["ok"], true,
        "the lookup must have succeeded on the shared mocks: {trace:?}"
    );
    assert_eq!(trace[1]["tool"], "ask_user");
    assert_eq!(
        trace[1]["ok"], true,
        "a lookup does not make the ask late: {trace:?}"
    );
    assert_eq!(
        body["answer"], "Which table do you mean by sales?",
        "the accepted ask ends the turn with its question: {body}"
    );
}

#[tokio::test]
async fn an_ask_after_three_successful_lookups_is_still_accepted() {
    // `AI-17`: none of `list_metrics`, `describe_metric` or `list_datasets`
    // returns data rows, so even the full lookup sequence leaves the turn
    // free to ask.
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "list_metrics", "{}")]),
        model_round(
            None,
            &[("call_2", "describe_metric", r#"{"name":"kunjungan"}"#)],
        ),
        model_round(None, &[("call_3", "list_datasets", "{}")]),
        model_round(None, &[("call_4", "ask_user", ASK_TWO)]),
    ])
    .await;
    let ch = mock_clickhouse_with_metric().await;

    let body = chat(&llm, &ch, "berapa sales?").await;

    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(trace.len(), 4, "three lookups then the ask: {trace:?}");
    assert_eq!(trace[0]["tool"], "list_metrics");
    assert_eq!(trace[0]["ok"], true, "the lookup must have run: {trace:?}");
    assert_eq!(trace[1]["tool"], "describe_metric");
    assert_eq!(
        trace[1]["ok"], true,
        "describe_metric must have found the saved metric, or the ask below would pass through the failed-call exception instead: {trace:?}"
    );
    assert_eq!(trace[2]["tool"], "list_datasets");
    assert_eq!(
        trace[2]["ok"], true,
        "the dataset list must have run: {trace:?}"
    );
    assert_eq!(trace[3]["tool"], "ask_user");
    assert_eq!(
        trace[3]["ok"], true,
        "lookups are not queries: the ask is accepted: {trace:?}"
    );
    assert_eq!(
        body["answer"], "Which table do you mean by sales?",
        "the accepted ask ends the turn with its question: {body}"
    );
}

#[tokio::test]
async fn an_ask_after_a_lookup_followed_by_a_query_is_still_refused() {
    // `AI-17` narrowed the late-ask rule to tools that read data; `run_sql`
    // is one of them, so a lookup before it changes nothing.
    let llm = mock_model(vec![
        model_round(None, &[("call_1", "list_metrics", "{}")]),
        model_round(None, &[("call_2", "run_sql", RUN_SQL)]),
        model_round(None, &[("call_3", "ask_user", ASK_TWO)]),
        model_round(Some("Ada 5 baris."), &[]),
    ])
    .await;
    let ch = mock_clickhouse().await;

    let body = chat(&llm, &ch, "berapa sales?").await;

    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(trace.len(), 3, "lookup, query, refused ask: {trace:?}");
    assert_eq!(trace[0]["tool"], "list_metrics");
    assert_eq!(trace[0]["ok"], true, "the lookup must have run: {trace:?}");
    assert_eq!(trace[1]["tool"], "run_sql");
    assert_eq!(trace[1]["ok"], true, "the query must have run: {trace:?}");
    assert_eq!(trace[2]["tool"], "ask_user");
    assert_eq!(trace[2]["ok"], false);
    assert_eq!(trace[2]["result"]["error"], LATE_ASK_REFUSAL);
    assert_eq!(
        body["answer"], "Ada 5 baris.",
        "the refused ask left the turn open: {body}"
    );
}

#[tokio::test]
async fn an_ask_after_a_successful_metric_value_query_is_refused() {
    // `AI-17`: `query_metric` returns data rows, so it counts as a query and
    // the ask after it is late. It must SUCCEED here: a failed query_metric
    // would make the ask pass through the failed-call exception instead,
    // which this test must not rely on.
    let llm = mock_model(vec![
        model_round(
            None,
            &[("call_1", "query_metric", r#"{"name":"kunjungan"}"#)],
        ),
        model_round(None, &[("call_2", "ask_user", ASK_TWO)]),
        model_round(Some("Ada 5 baris."), &[]),
    ])
    .await;
    let ch = mock_clickhouse_with_metric().await;

    let body = chat(&llm, &ch, "berapa sales?").await;

    let trace = body["toolTrace"].as_array().expect("trace is a list");
    assert_eq!(
        trace.len(),
        2,
        "the value query then the refused ask: {trace:?}"
    );
    assert_eq!(trace[0]["tool"], "query_metric");
    assert_eq!(
        trace[0]["ok"], true,
        "query_metric must have returned data here, or the refusal below is the failed-call exception, not the late-ask rule: {trace:?}"
    );
    assert_eq!(trace[1]["tool"], "ask_user");
    assert_eq!(trace[1]["ok"], false);
    assert_eq!(trace[1]["result"]["error"], LATE_ASK_REFUSAL);
    assert_eq!(
        body["answer"], "Ada 5 baris.",
        "the refused ask left the turn open: {body}"
    );
}
