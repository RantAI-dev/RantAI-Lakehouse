//! The history endpoint's query string is the page's only door to the
//! rows (`QS-4`, decisions 2, 5 and 7), so the door's own behaviour is
//! pinned here: a value the route cannot parse is a 400 whose body is the
//! route's fixed sentence — never a serde rendering — and a value it can
//! parse reaches the store's filters, with the row carrying its
//! `tablesRead` and, for a failed or refused run, its `message`.
//!
//! The SQL semantics behind the filters (the 14-day clamp, the cursor,
//! the literal `%`, owner scoping) are `tests/queries.rs`'s; this file
//! owns what the HTTP surface does with the caller's text.
//!
//! These tests drive the real router over a fresh database per test, with
//! a principal that holds only `query:read` — the history endpoint reads
//! no engine, so no `ClickHouse` mock is wired.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::queries::{RecordHistoryInput, record_history};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use common::{TestApp, create_principal_with_permissions, session_cookie_for_user};

/// One recorded run for the pass-through test, written the way `run`
/// writes them — the caller is the owner, the status is whatever the run
/// reached.
async fn insert_row(
    pool: &sqlx::PgPool,
    id: &str,
    owner: Uuid,
    status: &str,
    engine: &str,
    duration_ms: i64,
    tables: &[&str],
    message: Option<&str>,
) {
    let tables: Vec<String> = tables.iter().map(|t| (*t).to_owned()).collect();
    record_history(
        pool,
        &RecordHistoryInput {
            id,
            sql: "SELECT 1",
            user: &owner.to_string(),
            owner_id: Some(owner),
            status,
            duration_ms,
            scanned_bytes: 1,
            cost_units: 0.1,
            workload_class: "hot-analytics",
            engine,
            cache_assisted: false,
            tables_read: &tables,
            message,
        },
    )
    .await
    .unwrap();
}

/// `GET /api/query/history?query` as the principal's cookie carries it.
async fn get_history(app: &TestApp, cookie: &str, query: &str) -> (StatusCode, Value) {
    let uri = if query.is_empty() {
        "/api/query/history".to_owned()
    } else {
        format!("/api/query/history?{query}")
    };
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&uri)
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    (status, body)
}

/// A value outside the parser's sets is a 400 whose `error` is the fixed
/// sentence naming the field — the response never carries a serde or
/// urlencoded rendering of what went wrong.
#[tokio::test]
async fn a_bad_filter_value_is_a_400_with_the_fixed_message() {
    let app = common::spin_up().await;
    let user = create_principal_with_permissions(&app.pool, "query:read").await;
    let cookie = session_cookie_for_user(&app.pool, user).await;

    for (query, fixed) in [
        (
            "status=yesterday",
            "status must be one of completed, failed, cancelled, blocked",
        ),
        ("engine=presto", "engine must be one of clickhouse, trino"),
        (
            "minDurationMs=soon",
            "minDurationMs must be a non-negative whole number of milliseconds",
        ),
        ("from=not-a-time", "from must be an RFC 3339 time"),
        (
            "before=2026-10-20T00:00:00Z",
            "before must be <at>,<id>: the at and the id of the last row you have seen",
        ),
    ] {
        let (status, body) = get_history(&app, &cookie, query).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}: {body}");
        assert_eq!(body["error"], fixed, "{query}");
        let rendered = body.to_string();
        assert!(
            !rendered.contains("serde") && !rendered.contains("unknown variant"),
            "{query}: the response carries a library error: {rendered}"
        );
    }
}

/// The parsed filters reach the rows: the endpoint answers the page's
/// one-axis questions, and each row carries `tablesRead` — with `message`
/// only where there is something to say.
#[tokio::test]
async fn the_filters_reach_the_rows_and_the_row_carries_its_tables_and_message() {
    let app = common::spin_up().await;
    let user = create_principal_with_permissions(&app.pool, "query:read").await;
    let cookie = session_cookie_for_user(&app.pool, user).await;

    insert_row(
        &app.pool,
        "q-a",
        user,
        "completed",
        "clickhouse",
        100,
        &["marts.retails"],
        None,
    )
    .await;
    insert_row(
        &app.pool,
        "q-b",
        user,
        "failed",
        "clickhouse",
        5_000,
        &["bronze.orders"],
        Some("Arrow table is missing"),
    )
    .await;
    insert_row(
        &app.pool,
        "q-c",
        user,
        "blocked",
        "trino",
        2_500,
        &["marts.retails", "bronze.orders"],
        Some("Table not found or not available to your organisation: bronze.orders"),
    )
    .await;

    let (status, body) = get_history(&app, &cookie, "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids: Vec<&str> = body
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["q-c", "q-b", "q-a"], "newest first");

    let (status, body) = get_history(&app, &cookie, "status=failed").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["id"], "q-b");
    assert_eq!(body[0]["message"], "Arrow table is missing");
    assert_eq!(body[0]["tablesRead"], json!(["bronze.orders"]));

    let (status, body) = get_history(&app, &cookie, "table=orders").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body.as_array().unwrap().len(),
        2,
        "any element of tablesRead"
    );

    let (status, body) = get_history(&app, &cookie, "minDurationMs=1000").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body.as_array().unwrap().len(), 2);

    let (_, body) = get_history(&app, &cookie, "status=completed").await;
    assert_eq!(body[0]["id"], "q-a");
    assert!(
        body[0].get("message").is_none(),
        "a completed run has nothing to say, and the key is omitted: {body}"
    );
    assert_eq!(body[0]["tablesRead"], json!(["marts.retails"]));
}
