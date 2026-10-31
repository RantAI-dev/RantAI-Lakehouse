//! What a build turn is offered, and how much of it fits in a small
//! context (`AI-5`, `AI-8`, `AI-15`).
//!
//! A build turn asks for a chart in words that name no dashboard ("Show the
//! share of foreign visitors by country as a pie."), and the model could
//! not reach `create_chart`: the tools were chosen by the words in the
//! question, and "pie" was in none of them. Now a build turn always offers
//! the chart and dashboard tools a chart request needs, whatever it says.
//!
//! The list must still fit a 32k context: the offered tools are the compact
//! JSON the model receives, and the bar is 60,000 characters. The console's
//! tool list is an upper bound, not a replacement, so the tools a turn needs
//! are the ones that are offered among the ones the person allowed.
//!
//! The model is mocked, and the offered list is read from the request the
//! model received, so the test measures what the model really sees.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

/// The messages that failed without a chart tool (the eval's failing set).
const CHART_MESSAGES: &[&str] = &[
    "Show the share of foreign visitors by country as a pie.",
    "Show a table of the events with their dates.",
    "Make a pivot of foreign visitors by year and month.",
    "Make a pivot of foreign visitors by year and month, with a grand total row.",
    "Add a progress bar of the total number of events against a goal of 300,000.",
    "Add a bullet graph of foreign visitors against a target of 1,000.",
];

/// The two messages that ask for a filter and expect the filter tool.
const FILTER_MESSAGES: &[&str] = &[
    "Show me events for last year only.",
    "Tampilkan hanya untuk Jawa Barat.",
];

/// The chart and dashboard tools a build turn offers whatever it says.
const CHART_CORE: &[&str] = &[
    "list_charts",
    "list_sql_sources",
    "suggest_join",
    "suggest_dashboard",
    "create_chart",
    "update_chart",
    "format_chart",
    "create_board",
    "create_tab",
    "move_charts_to_tab",
    "set_board_view_filters",
    "save_board_filters",
    "list_calculated_fields",
    "validate_formula",
    "create_calculated_field",
];

/// The tools of `CHART_CORE` that write, by the registry's own risk: Ask
/// mode must offer none of them. `set_board_view_filters` and
/// `validate_formula` are reads (the first changes only the viewer's own
/// view, the second checks a formula), so Ask mode keeps them.
const CHART_CORE_WRITES: &[&str] = &[
    "create_chart",
    "update_chart",
    "format_chart",
    "create_board",
    "create_tab",
    "move_charts_to_tab",
    "save_board_filters",
    "create_calculated_field",
];

/// The tools every turn has.
const ALWAYS: &[&str] = &[
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

/// The pipeline tools a question about the pipelines must reach.
const PIPELINE_TOOLS: &[&str] = &["list_pipeline_runs", "retry_pipeline_run"];

/// The bar for the compact JSON of the offered tools: about 17k tokens, so a
/// 32k context holds the tools, the system prompt and the conversation.
const SIZE_BAR: usize = 60_000;

/// One model round with no tool call: the turn ends after it.
fn model_round(content: &str) -> Value {
    json!({
        "choices": [{ "message": {
            "role": "assistant",
            "content": content,
            "tool_calls": Value::Null,
        } }],
    })
}

/// A model that answers every request with the given text.
async fn mock_model() -> MockServer {
    let llm = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(model_round("Done.")))
        .mount(&llm)
        .await;
    llm
}

/// `ClickHouse` that answers every read with an empty answer: the chat's
/// DATA MAP reads it, and nothing else does.
async fn mock_clickhouse() -> MockServer {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&ch)
        .await;
    ch
}

/// Every registered tool's name, from the schema fixture.
fn every_tool_name() -> Vec<String> {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/tool_schemas.json"))
        .expect("the fixture is JSON");
    fixture
        .as_array()
        .expect("the fixture is a list of tools")
        .iter()
        .filter_map(|t| t["function"]["name"].as_str().map(str::to_owned))
        .collect()
}

/// What the model received in its first round: the tools it was offered
/// (in order), the system prompt and the tool list itself.
async fn first_request(
    question: &str,
    mode: &str,
    allow: Option<Value>,
) -> (Vec<String>, String, Value) {
    let llm = mock_model().await;
    let ch = mock_clickhouse().await;
    let mut env = std::collections::HashMap::new();
    env.insert("LLM_URL".to_owned(), llm.uri());
    env.insert("CH_URL".to_owned(), ch.uri());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let mut body = json!({
        "mode": mode,
        "messages": [{ "role": "user", "content": question }],
    });
    if let Some(allow) = allow {
        body["tools"] = allow;
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
    assert_eq!(resp.status(), StatusCode::OK);
    let requests = llm.received_requests().await.expect("recording is on");
    let first: Value = serde_json::from_slice(&requests[0].body).expect("a JSON request");
    let tools = first["tools"].clone();
    let names = tools
        .as_array()
        .expect("the request carries a tool list")
        .iter()
        .filter_map(|t| t["function"]["name"].as_str().map(str::to_owned))
        .collect();
    let system = first["messages"][0]["content"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    (names, system, tools)
}

/// The tools the model was offered for a turn.
async fn offered_for(question: &str, mode: &str, allow: Option<Value>) -> Vec<String> {
    first_request(question, mode, allow).await.0
}

#[tokio::test]
async fn a_build_turn_about_a_chart_offers_the_chart_tools_whatever_it_says() {
    for message in CHART_MESSAGES {
        let names = offered_for(message, "build", None).await;
        assert!(
            names.contains(&"create_chart".to_owned()),
            "a build turn must offer create_chart for {message:?}: {names:?}"
        );
    }
}

#[tokio::test]
async fn a_build_turn_about_a_filter_offers_the_filter_tool() {
    for message in FILTER_MESSAGES {
        let names = offered_for(message, "build", None).await;
        assert!(
            names.contains(&"set_board_view_filters".to_owned()),
            "a build turn must offer set_board_view_filters for {message:?}: {names:?}"
        );
        assert!(
            names.contains(&"create_chart".to_owned()),
            "a build turn must offer create_chart for {message:?}: {names:?}"
        );
    }
}

#[tokio::test]
async fn an_ask_turn_about_the_same_chart_offers_no_write_tool() {
    for message in CHART_MESSAGES.iter().chain(FILTER_MESSAGES) {
        let names = offered_for(message, "ask", None).await;
        for write in CHART_CORE_WRITES {
            assert!(
                !names.contains(&write.to_string()),
                "ask mode must not offer {write} for {message:?}: {names:?}"
            );
        }
    }
}

#[tokio::test]
async fn a_build_turn_about_the_pipelines_offers_the_chart_core_and_the_pipeline_tools() {
    let names = offered_for("Show me the pipeline runs", "build", None).await;

    for core in CHART_CORE {
        assert!(
            names.contains(&core.to_string()),
            "a build turn must offer {core}: {names:?}"
        );
    }
    for tool in PIPELINE_TOOLS {
        assert!(
            names.contains(&tool.to_string()),
            "a build turn about the pipelines must offer {tool}: {names:?}"
        );
    }
}

#[tokio::test]
async fn the_offered_tools_start_with_the_always_tools_and_the_chart_core() {
    let names = offered_for("Show me the pipeline runs", "build", None).await;

    let always_at = names
        .iter()
        .position(|n| n == "run_sql")
        .expect("the always tools are offered");
    let core_at = names
        .iter()
        .position(|n| n == "create_chart")
        .expect("the chart core is offered");
    assert!(
        always_at < core_at,
        "the always tools come before the chart core: {names:?}"
    );
}

#[tokio::test]
async fn an_allowlist_without_create_chart_never_offers_it() {
    let allow = json!([
        "run_sql",
        "list_datasets",
        "describe_dataset",
        "lakehouse_overview"
    ]);

    let names = offered_for(
        "Show the share of foreign visitors by country as a pie.",
        "build",
        Some(allow),
    )
    .await;

    assert!(
        !names.contains(&"create_chart".to_owned()),
        "a tool the person did not allow is never offered: {names:?}"
    );
}

#[tokio::test]
async fn an_allowlist_holding_every_tool_offers_the_chart_core_and_the_matched_domains() {
    // Every registered tool, so the allowlist cannot narrow the list: what
    // is offered is the per-turn selection alone.
    let allow = json!(every_tool_name());

    let names = offered_for(
        "Show the share of foreign visitors by country as a pie.",
        "build",
        Some(allow),
    )
    .await;

    for core in CHART_CORE {
        assert!(
            names.contains(&core.to_string()),
            "the chart core is offered: {names:?}"
        );
    }
    for always in ALWAYS {
        assert!(
            names.contains(&always.to_string()),
            "the always tools are offered: {names:?}"
        );
    }
    // A tool of a domain the question does not match is not offered, even
    // though the allowlist holds it.
    assert!(
        !names.contains(&"retry_pipeline_run".to_owned()),
        "a domain the question does not match stays out: {names:?}"
    );
}

#[tokio::test]
async fn two_build_messages_with_different_domains_share_the_same_prefix() {
    let (chart, chart_system, _) = first_request(
        "Show the share of foreign visitors by country as a pie.",
        "build",
        None,
    )
    .await;
    let (pipeline, pipeline_system, _) =
        first_request("Show me the pipeline runs", "build", None).await;

    let shared = chart
        .iter()
        .zip(pipeline.iter())
        .take_while(|(a, b)| a == b)
        .count();
    assert!(
        shared >= ALWAYS.len() + CHART_CORE.len(),
        "the always tools and the chart core are the shared prefix: {chart:?} / {pipeline:?}"
    );
    assert_eq!(
        chart_system, pipeline_system,
        "the system text of two build turns is the same"
    );
}

#[tokio::test]
async fn the_offered_tools_of_a_build_turn_fit_a_small_context() {
    let (_, _, tools) = first_request(
        "Show the share of foreign visitors by country as a pie.",
        "build",
        None,
    )
    .await;

    let compact = serde_json::to_string(&tools).expect("the tool list is JSON");
    // The measured size, so a reader of a failed run knows how far the
    // list is from the bar.
    println!(
        "the offered tools are {} in {} characters",
        tools.as_array().map_or(0, Vec::len),
        compact.chars().count()
    );
    assert!(
        compact.chars().count() <= SIZE_BAR,
        "the offered tools are {} characters, the bar is {SIZE_BAR}",
        compact.chars().count()
    );
}
