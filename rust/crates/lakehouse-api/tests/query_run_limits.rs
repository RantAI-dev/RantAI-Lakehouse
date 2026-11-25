//! `POST /api/query/run` over the real router sends the caller's run to the
//! engine (`BI-5` review S7): the query id is `<user uuid>:<run id>:0`, from the
//! `X-Run-Id` header the console sends, and the time limit is the longest one of
//! the caller's roles, so a Stop by that run id finds the query and a role's own
//! limit reaches the engine.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

#[tokio::test]
async fn a_query_run_sends_the_callers_query_id_and_the_roles_time_limit() {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("system.columns"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [
                {"name": "name", "type": "String"}, {"name": "type", "type": "String"},
                {"name": "default_kind", "type": "String"}, {"name": "default_expression", "type": "String"},
            ],
            "data": [{"name": "n", "type": "UInt8", "default_kind": "", "default_expression": ""}],
            "rows": 1,
        })))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [{"name": "n", "type": "UInt8"}], "data": [{"n": 1}], "rows": 1,
        })))
        .mount(&ch)
        .await;
    let mut env = std::collections::HashMap::new();
    env.insert("CH_URL".to_owned(), ch.uri());
    env.insert(
        "CATALOG_TENANT_ID".to_owned(),
        "11111111-1111-4111-8111-000000000001".to_owned(),
    );
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let email = "fajar@meridian.example";
    let (user, role): (uuid::Uuid, uuid::Uuid) = sqlx::query_as(
        "SELECT ur.user_id, ur.role_id FROM app_user_role ur \
         JOIN app_user u ON u.id = ur.user_id WHERE u.email = $1 LIMIT 1",
    )
    .bind(email)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO query_limits_role (role_id, timeout_seconds) VALUES ($1, 300)")
        .bind(role)
        .execute(&pool)
        .await
        .unwrap();
    let cookie = session_cookie_for_seeded_user(&pool, email).await;
    let run = uuid::Uuid::new_v4();
    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/query/run")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .header("x-run-id", run.to_string())
                .body(Body::from(json!({ "sql": "SELECT 1 AS n" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let sent = ch.received_requests().await.unwrap();
    let ran: Vec<_> = sent
        .iter()
        .filter(|r| r.url.query_pairs().any(|(k, _)| k == "query_id"))
        .collect();
    assert_eq!(ran.len(), 1, "one statement carried a query id");
    let q: std::collections::HashMap<_, _> = ran[0].url.query_pairs().into_owned().collect();
    assert_eq!(q["query_id"], format!("{user}:{run}:0"));
    assert_eq!(q["max_execution_time"], "300");
}

/// `BI-63` (`BI-5-AC2`): the console aborts the run request right after it
/// presses Stop, and axum drops the handler with the connection. The row of
/// that run must still be in History, as `stopped`: the engine call and the
/// write run in a detached task. The engine here answers after a delay, which
/// stands in for a query that runs until it is killed; the request is dropped
/// (a timeout drops the future the way a closed connection does) and the run
/// is marked stopped the way `POST /api/query/cancel` marks it.
#[tokio::test]
async fn a_run_whose_connection_is_dropped_after_a_stop_is_recorded_as_stopped() {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("system.columns"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [
                {"name": "name", "type": "String"}, {"name": "type", "type": "String"},
                {"name": "default_kind", "type": "String"}, {"name": "default_expression", "type": "String"},
            ],
            "data": [{"name": "n", "type": "UInt8", "default_kind": "", "default_expression": ""}],
            "rows": 1,
        })))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_secs(3))
                .set_body_json(json!({
                    "meta": [{"name": "n", "type": "UInt8"}], "data": [{"n": 1}], "rows": 1,
                })),
        )
        .mount(&ch)
        .await;
    let mut env = std::collections::HashMap::new();
    env.insert("CH_URL".to_owned(), ch.uri());
    env.insert(
        "CATALOG_TENANT_ID".to_owned(),
        "11111111-1111-4111-8111-000000000001".to_owned(),
    );
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let email = "fajar@meridian.example";
    let user: uuid::Uuid = sqlx::query_scalar(
        "SELECT ur.user_id FROM app_user_role ur \
         JOIN app_user u ON u.id = ur.user_id WHERE u.email = $1 LIMIT 1",
    )
    .bind(email)
    .fetch_one(&pool)
    .await
    .unwrap();
    let cookie = session_cookie_for_seeded_user(&pool, email).await;
    let run = uuid::Uuid::new_v4();
    let request = Request::builder()
        .method("POST")
        .uri("/api/query/run")
        .header("cookie", &cookie)
        .header("content-type", "application/json")
        .header("x-run-id", run.to_string())
        .body(Body::from(json!({ "sql": "SELECT 1 AS n" }).to_string()))
        .unwrap();

    // The connection is dropped before the engine answers.
    let dropped = tokio::time::timeout(
        std::time::Duration::from_millis(800),
        router.oneshot(request),
    )
    .await;
    assert!(dropped.is_err(), "the engine had not answered yet");
    lakehouse_api::query_limits::mark_stopped(user, run);

    // The detached task finishes after the engine answers and writes the row.
    let mut status: Option<String> = None;
    for _ in 0..50 {
        status = sqlx::query_scalar("SELECT status FROM query_history WHERE owner_id = $1")
            .bind(user)
            .fetch_optional(&pool)
            .await
            .unwrap();
        if status.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(status.as_deref(), Some("stopped"));
}
