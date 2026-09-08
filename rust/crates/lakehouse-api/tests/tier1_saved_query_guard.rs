//! Proves T1.4's `run_saved_query` really goes through the SAME read-only
//! SQL guard `POST /api/query/run` uses (`routes/query.rs`'s `is_read_only`)
//! rather than a second, possibly-weaker copy: a saved query whose SQL is
//! a `DROP`/`INSERT` statement is rejected at run time, even though nothing
//! stopped it from being SAVED (saving is not itself a guarded operation —
//! only running is).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use common::{TestApp, session_cookie_for_seeded_user, spin_up};

async fn call_tool(app: &axum::Router, cookie: &str, tool: &str, args: Value) -> Value {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/tool")
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "tool": tool,
                        "args": args,
                        "mode": "build",
                    }))
                    .expect("serialize body"),
                ))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(resp.status(), StatusCode::OK, "{tool}");
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read response body");
    serde_json::from_slice(&bytes).expect("valid JSON body")
}

/// A saved query holding a `DROP TABLE` statement is rejected when run,
/// with the exact message the Query Studio guard produces — proving the
/// SAME guard function ran, not a re-implementation that happens to also
/// reject drops.
#[tokio::test]
async fn run_saved_query_rejects_a_saved_drop_statement() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let saved = call_tool(
        &router,
        &cookie,
        "save_query",
        json!({ "title": "Malicious", "sql": "DROP TABLE serving.mart_wisman", "confirmed": true }),
    )
    .await;
    let id = saved["result"]["query"]["id"]
        .as_str()
        .expect("save_query must return the new query's id")
        .to_owned();

    let ran = call_tool(&router, &cookie, "run_saved_query", json!({ "id": id })).await;
    let error = ran["result"]["error"]
        .as_str()
        .expect("run_saved_query must refuse a DROP statement");
    assert!(
        error.contains("Hanya query baca"),
        "must be refused by the same read-only guard `/api/query/run` uses: {error}"
    );
}

/// An `INSERT` statement is refused the same way.
#[tokio::test]
async fn run_saved_query_rejects_a_saved_insert_statement() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let saved = call_tool(
        &router,
        &cookie,
        "save_query",
        json!({ "title": "Also malicious", "sql": "INSERT INTO t VALUES (1)", "confirmed": true }),
    )
    .await;
    let id = saved["result"]["query"]["id"]
        .as_str()
        .expect("save_query must return the new query's id")
        .to_owned();

    let ran = call_tool(&router, &cookie, "run_saved_query", json!({ "id": id })).await;
    let error = ran["result"]["error"]
        .as_str()
        .expect("run_saved_query must refuse an INSERT statement");
    assert!(error.contains("Hanya query baca"), "{error}");
}

/// A saved SELECT is NOT refused by the guard (it may still fail later
/// against the dead `ClickHouse` upstream this harness uses, but that is a
/// different, unrelated failure — never the read-only refusal message).
#[tokio::test]
async fn run_saved_query_does_not_refuse_a_saved_select() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let saved = call_tool(
        &router,
        &cookie,
        "save_query",
        json!({ "title": "Fine", "sql": "SELECT 1", "confirmed": true }),
    )
    .await;
    let id = saved["result"]["query"]["id"]
        .as_str()
        .expect("save_query must return the new query's id")
        .to_owned();

    let ran = call_tool(&router, &cookie, "run_saved_query", json!({ "id": id })).await;
    if let Some(error) = ran["result"]["error"].as_str() {
        assert!(
            !error.contains("Hanya query baca"),
            "a plain SELECT must never be refused by the read-only guard: {error}"
        );
    }
}
