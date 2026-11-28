//! How big the chat prompt and its offered tools are, per mode and per
//! principal (`AI-35`, measurement only).
//!
//! The Build prompt measured about 30k tokens in the 2026-11-12 local-model
//! eval, so every Build round took minutes. This report reads what the model
//! really receives on the first round of a turn (the system message and the
//! tool list), for Ask and Build, for a Platform Admin and a Dashboard
//! Viewer, on the seed data. It changes no prompt.
//!
//! Sizes are in characters, and in estimated tokens (characters / 4). The
//! estimate is a rule of thumb, not a tokenizer count.
//!
//! Run with `cargo test -q -p lakehouse-api --test it ai_prompt_size:: -- --ignored --nocapture`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_seeded_user,
    session_cookie_for_user, spin_up_with_env,
};

/// The question every case asks. It is a chart request, so a Build turn
/// offers the chart tools.
const QUESTION: &str = "Show the share of foreign visitors by country as a pie.";

/// One model round with no tool call, so the turn ends after the first.
fn model_round(content: &str) -> Value {
    json!({
        "choices": [{ "message": {
            "role": "assistant",
            "content": content,
            "tool_calls": Value::Null,
        } }],
    })
}

/// What the model received on its first round for one case.
struct Measured {
    system: String,
    tools: Value,
}

/// Runs one turn as `cookie` and reads the first request the model got.
async fn measure(mode: &str, principal: Principal) -> Measured {
    // `OPS-36`: the DATA MAP cache lives on each `AppState`, so this app
    // builds and reuses its own map. A chat that ran earlier in the same
    // process can no longer leave its map behind, and every total is
    // measured against the map this test's own mocks produce (`AI-32`).
    let llm = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(model_round("Done.")))
        .mount(&llm)
        .await;
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&ch)
        .await;
    let mut env = std::collections::HashMap::new();
    env.insert("LLM_URL".to_owned(), llm.uri());
    env.insert("CH_URL".to_owned(), ch.uri());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let cookie = match principal {
        Principal::Admin => session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await,
        Principal::Viewer => {
            let id =
                create_principal_with_permissions(&pool, "dashboard:read, assistant:use").await;
            session_cookie_for_user(&pool, id).await
        }
    };
    let body = json!({
        "mode": mode,
        "messages": [{ "role": "user", "content": QUESTION }],
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
    assert_eq!(resp.status(), StatusCode::OK);
    let requests = llm.received_requests().await.expect("recording is on");
    let first: Value = serde_json::from_slice(&requests[0].body).expect("a JSON request");
    Measured {
        system: first["messages"][0]["content"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        tools: first["tools"].clone(),
    }
}

/// Every `description` string under a schema node, in characters.
fn description_chars(node: &Value) -> usize {
    match node {
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| match (k.as_str(), v) {
                ("description", Value::String(s)) => s.chars().count(),
                _ => description_chars(v),
            })
            .sum(),
        Value::Array(items) => items.iter().map(description_chars).sum(),
        _ => 0,
    }
}

/// Prints the description characters of each offered tool, largest first:
/// the tool-level text and the property-level text, as the size test counts them.
fn print_description_blocks(tools: &Value) {
    let mut rows: Vec<(usize, usize, String)> = tools
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|tool| {
            let function = &tool["function"];
            let name = function["name"].as_str().unwrap_or_default().to_owned();
            let tool_level = function["description"]
                .as_str()
                .map_or(0, |s| s.chars().count());
            let property_level = description_chars(&function["parameters"]);
            (tool_level, property_level, name)
        })
        .collect();
    rows.sort_by_key(|(tool, property, _)| std::cmp::Reverse(tool + property));
    println!("per-tool description characters (tool-level, property-level, total):");
    for (tool, property, name) in rows {
        println!("{name:<32} {tool:>6} {property:>6} {:>6}", tool + property);
    }
}

#[derive(Clone, Copy)]
enum Principal {
    Admin,
    Viewer,
}

/// Characters and estimated tokens (characters / 4).
fn size(text: &str) -> (usize, usize) {
    let chars = text.chars().count();
    (chars, chars / 4)
}

/// The admin Build total, the admin Ask total and the viewer Build total, in
/// characters: the system message plus the offered tool schemas, as the
/// model receives them. Each has a ceiling below.
///
/// The owner approved these ceilings (owner, 2026-11-22, via the Auditor).
/// They sit at or above the totals [`measure`] reports on purpose: the headroom is
/// reserved for a following change, so no one lowers a ceiling to the
/// measured total. The admin Build ceiling is above the owner's earlier
/// target of 68,000, which the sweep did not reach.
///
/// `OPS-36` scoped the data-map cache to each `AppState`, so a total is the
/// same when this file runs alone and when the whole suite runs (`AI-32`).
///
/// Raised for `AI-11`'s three sharing tools (owner, 2026-11-26, via the
/// Auditor): the schemas and the prompt's sharing rule moved the measured
/// totals to 78,143 / 20,214 / 31,627, deltas +1,736 / +490 / +1,928.
const ADMIN_BUILD_CEILING: usize = 78_143;
const ADMIN_ASK_CEILING: usize = 20_214;
const VIEWER_BUILD_CEILING: usize = 31_627;

#[tokio::test]
async fn the_chat_prompt_and_its_tools_stay_under_the_owner_target() {
    let admin_build = measure("build", Principal::Admin).await;
    let admin_ask = measure("ask", Principal::Admin).await;
    let viewer_build = measure("build", Principal::Viewer).await;
    let total = |m: &Measured| {
        m.system.chars().count()
            + serde_json::to_string(&m.tools)
                .unwrap_or_default()
                .chars()
                .count()
    };
    let mut over = Vec::new();
    if total(&admin_build) > ADMIN_BUILD_CEILING {
        over.push(format!(
            "admin Build total {} is over {ADMIN_BUILD_CEILING}",
            total(&admin_build)
        ));
    }
    if total(&admin_ask) > ADMIN_ASK_CEILING {
        over.push(format!(
            "admin Ask total {} is over {ADMIN_ASK_CEILING}",
            total(&admin_ask)
        ));
    }
    if total(&viewer_build) > VIEWER_BUILD_CEILING {
        over.push(format!(
            "viewer Build total {} is over {VIEWER_BUILD_CEILING}",
            total(&viewer_build)
        ));
    }
    assert!(over.is_empty(), "{}", over.join("; "));
}

#[tokio::test]
#[ignore = "measurement report for AI-35; run with --ignored --nocapture"]
async fn ai_prompt_size_report() {
    println!();
    println!("AI-35 prompt measurement (characters; tokens = characters / 4, an estimate)");
    println!("question: {QUESTION:?}");
    for (label, mode, principal) in [
        ("ask, platform admin", "ask", Principal::Admin),
        ("build, platform admin", "build", Principal::Admin),
        ("ask, dashboard viewer", "ask", Principal::Viewer),
        ("build, dashboard viewer", "build", Principal::Viewer),
    ] {
        let m = measure(mode, principal).await;
        if mode == "build" && matches!(principal, Principal::Admin) {
            print_description_blocks(&m.tools);
        }
        let (sys_chars, sys_tokens) = size(&m.system);
        let tools = m.tools.as_array().map_or(0, Vec::len);
        let tool_json = serde_json::to_string(&m.tools).unwrap_or_default();
        let (tool_chars, tool_tokens) = size(&tool_json);
        println!(
            "{label:<26} system {sys_chars:>6} ch ~{sys_tokens:>5} tok | tools {tools:>3} ({tool_chars:>6} ch ~{tool_tokens:>5} tok) | system+tools ~{:>5} tok",
            sys_tokens + tool_tokens
        );
    }
}
