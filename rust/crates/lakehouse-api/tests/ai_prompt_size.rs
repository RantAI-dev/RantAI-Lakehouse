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
