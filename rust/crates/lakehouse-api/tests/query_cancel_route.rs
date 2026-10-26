//! `POST /api/query/cancel` over the real router (`BI-5`, T4): the statement
//! it sends names the caller's own id and the run, never anyone else's, and
//! the answer counts what was running.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_zero_permission_principal, session_cookie_for_user, spin_up_with_env,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

const RUN: &str = "22222222-2222-4222-8222-222222222222";
const OTHER_PERSON: &str = "99999999-9999-4999-8999-999999999999";

async fn stop(app: &axum::Router, cookie: &str, body: &str) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/query/cancel")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn engine(running: u32) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("count()"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!(
            r#"{{"meta":[{{"name":"n","type":"UInt64"}}],"data":[{{"n":"{running}"}}],"rows":1}}"#
        )))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("KILL QUERY"))
        .respond_with(ResponseTemplate::new(200).set_body_string("Ok."))
        .mount(&server)
        .await;
    server
}

async fn bodies(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect()
}

#[tokio::test]
async fn stop_kills_only_the_callers_own_prefix_and_counts_what_was_running() {
    let server = engine(3).await;
    let mut env = HashMap::new();
    env.insert("CH_URL".to_owned(), server.uri());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let user = create_zero_permission_principal(&pool).await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, body) = stop(&router, &cookie, &json!({ "runId": RUN }).to_string()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "stopped": 3 }));

    let sent = bodies(&server).await;
    let kill = sent
        .iter()
        .find(|b| b.starts_with("KILL QUERY"))
        .expect("a kill was sent");
    assert_eq!(
        kill,
        &format!("KILL QUERY WHERE query_id LIKE '{user}:{RUN}:%' ASYNC")
    );
    assert!(sent.iter().all(|b| !b.contains(OTHER_PERSON)));
}

// BI-5 review S2 (PR #245 CI): Stop always sends the kill, even when the
// count is 0, because a query can start between the count and the kill and
// `KILL` of nothing is harmless. The run's stopped mark is set by the same
// `stop_own_queries` and is asserted in `routes/ai/tools/limits.rs`.
#[tokio::test]
async fn nothing_running_is_stopped_zero_and_still_sends_a_kill_for_only_the_callers_own_run() {
    let server = engine(0).await;
    let mut env = HashMap::new();
    env.insert("CH_URL".to_owned(), server.uri());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let user = create_zero_permission_principal(&pool).await;
    let cookie = session_cookie_for_user(&pool, user).await;
    let (status, body) = stop(&router, &cookie, &json!({ "runId": RUN }).to_string()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "stopped": 0 }));

    let sent = bodies(&server).await;
    let kills: Vec<&String> = sent.iter().filter(|b| b.starts_with("KILL")).collect();
    assert_eq!(
        kills,
        vec![&format!(
            "KILL QUERY WHERE query_id LIKE '{user}:{RUN}:%' ASYNC"
        )]
    );
    assert!(sent.iter().all(|b| !b.contains(OTHER_PERSON)));
}

#[tokio::test]
async fn a_run_id_that_is_not_a_uuid_or_a_body_with_a_user_field_is_refused_before_the_engine() {
    let server = engine(1).await;
    let mut env = HashMap::new();
    env.insert("CH_URL".to_owned(), server.uri());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let user = create_zero_permission_principal(&pool).await;
    let cookie = session_cookie_for_user(&pool, user).await;
    for bad in [
        r#"{"runId":"x' OR '1'='1"}"#.to_owned(),
        format!(r#"{{"runId":"{RUN}","principal":"{OTHER_PERSON}"}}"#),
        "not json".to_owned(),
    ] {
        let (status, _) = stop(&router, &cookie, &bad).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
    assert!(
        bodies(&server).await.is_empty(),
        "the engine was never asked"
    );
}
