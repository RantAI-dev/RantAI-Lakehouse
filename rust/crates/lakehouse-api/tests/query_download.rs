//! `GET /api/query/run/{id}/download` — owner-scoped re-run download
//! (WS2 §13, WS2 plan review W9).
//!
//! `download` reads `state.pg` (to look up the `query_history` row) and
//! calls `ClickHouse` (to re-run its `SQL`), so a pure unit test cannot
//! exercise it end to end — this file drives it through the real router,
//! a real per-test Postgres database (`common::spin_up_with_env`), and a
//! `wiremock` stand-in for `ClickHouse` (`CH_URL` override), following
//! `tests/debezium_properties.rs`'s exact real-database-plus-real-router
//! shape. Two seeded users (`rina@meridian.example`,
//! `sari@meridian.example`, both `Analyst` / `query:read` —
//! `0002_seed_identity.sql`) stand in for "the owner" and "a different
//! principal": there is no public `Principal` constructor, so every
//! request here authenticates through a real session cookie, never a
//! struct literal.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::queries::{RecordHistoryInput, record_history};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

async fn seeded_user_id(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM app_user WHERE email = $1")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("seeded user {email} must exist (0002_seed_identity.sql): {e}"))
}

/// Insert a `query_history` row directly through the store — the same
/// write `routes::query::run` performs on a successful execution — with
/// `owner_id` set to `owner` (`None` stands in for a legacy pre-`0046`
/// row, which `download`'s ownership check must match nobody). `download`
/// now checks `owner_id`, not the `user` display name — see
/// `routes::query::download`'s doc comment.
async fn insert_history_row(pool: &PgPool, id: &str, sql: &str, owner: Option<Uuid>, engine: &str) {
    let user = owner.map_or_else(|| "anonymous".to_owned(), |id| id.to_string());
    let input = RecordHistoryInput {
        id,
        sql,
        user: &user,
        owner_id: owner,
        status: "completed",
        duration_ms: 12,
        scanned_bytes: 34,
        cost_units: 1.0,
        workload_class: "hot-analytics",
        engine,
        cache_assisted: false,
    };
    record_history(pool, &input)
        .await
        .expect("insert a query_history fixture row");
}

async fn download_with_cookie(
    router: &axum::Router,
    id: &str,
    format: &str,
    cookie: &str,
) -> axum::http::Response<Body> {
    router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/query/run/{id}/download?format={format}"))
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright")
}

/// A `ClickHouse`-shaped `wiremock` stand-in, wired via `CH_URL` — see
/// `common::spin_up_with_env`'s doc comment on why every other upstream
/// stays the fail-fast `DEAD_UPSTREAM` default while this one test file
/// needs a real HTTP endpoint to assert requests against.
async fn spin_up_with_clickhouse(ch_uri: &str) -> TestApp {
    let mut overrides = std::collections::HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch_uri.to_owned());
    spin_up_with_env(&overrides).await
}

#[tokio::test]
async fn a_different_owner_gets_404() {
    let ch = MockServer::start().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;

    let owner_id = seeded_user_id(&pool, "rina@meridian.example").await;
    let stranger_cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;
    insert_history_row(&pool, "q-1", "SELECT 1", Some(owner_id), "clickhouse").await;

    let resp = download_with_cookie(&router, "q-1", "csv", &stranger_cookie).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert!(
        ch.received_requests()
            .await
            .expect("mock records requests")
            .is_empty(),
        "a mismatched owner must never reach ClickHouse"
    );
}

#[tokio::test]
async fn a_legacy_anonymous_row_gets_404_even_for_an_authenticated_caller() {
    let ch = MockServer::start().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;

    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    insert_history_row(&pool, "q-2", "SELECT 1", None, "clickhouse").await;

    let resp = download_with_cookie(&router, "q-2", "csv", &cookie).await;
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "a pre-C2 \"anonymous\" row must match nobody, by design"
    );
    assert!(
        ch.received_requests()
            .await
            .expect("mock records requests")
            .is_empty()
    );
}

#[tokio::test]
async fn stored_sql_that_is_not_read_only_is_refused_with_zero_clickhouse_requests() {
    let ch = MockServer::start().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;

    let owner_id = seeded_user_id(&pool, "rina@meridian.example").await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    // Defense in depth: routes::query::run already refuses this at insert
    // time, but a row could in principle exist another way (a hand-rolled
    // fixture, a future writer) — download must re-check, not trust the
    // stored row.
    insert_history_row(&pool, "q-3", "DELETE FROM t", Some(owner_id), "clickhouse").await;

    let resp = download_with_cookie(&router, "q-3", "csv", &cookie).await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        ch.received_requests()
            .await
            .expect("mock records requests")
            .is_empty(),
        "a non-read-only stored statement must never be re-run against ClickHouse"
    );
}

#[tokio::test]
async fn a_non_clickhouse_engine_is_refused_with_422() {
    let ch = MockServer::start().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;

    let owner_id = seeded_user_id(&pool, "rina@meridian.example").await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    insert_history_row(&pool, "q-4", "SELECT 1", Some(owner_id), "trino").await;

    let resp = download_with_cookie(&router, "q-4", "csv", &cookie).await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"], "download supports ClickHouse runs only");
    assert!(
        ch.received_requests()
            .await
            .expect("mock records requests")
            .is_empty(),
        "re-running on the wrong engine is not an option"
    );
}

#[tokio::test]
async fn the_happy_path_returns_bytes_and_download_headers() {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("n\n1\n2\n"))
        .mount(&ch)
        .await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;

    let owner_id = seeded_user_id(&pool, "rina@meridian.example").await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    insert_history_row(
        &pool,
        "q-5",
        "SELECT n FROM t",
        Some(owner_id),
        "clickhouse",
    )
    .await;

    let resp = download_with_cookie(&router, "q-5", "csv", &cookie).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers().get("content-type").unwrap(), "text/csv");
    assert_eq!(
        resp.headers().get("content-disposition").unwrap(),
        "attachment; filename=\"q-5.csv\""
    );
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&bytes[..], b"n\n1\n2\n");

    let requests = ch.received_requests().await.expect("mock records requests");
    assert_eq!(requests.len(), 1, "exactly one re-run reached ClickHouse");
    let sent_body = String::from_utf8(requests[0].body.clone()).expect("utf8 request body");
    assert!(
        sent_body.contains("SELECT * FROM (\nSELECT n FROM t\n) LIMIT 10000"),
        "the stored SQL must be wrapped, not appended to: {sent_body}"
    );
    assert!(sent_body.contains("FORMAT CSV"));
}

/// The `legacy "anonymous" row` case above proves 404 for an authenticated
/// caller; this proves the same route also 401s an entirely unauthenticated
/// request, per `Policy::RequiresPermission("query:read")` — defense in
/// depth exercised through the real router, not only the policy table.
#[tokio::test]
async fn an_unauthenticated_request_is_401_before_touching_clickhouse() {
    let ch = MockServer::start().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    insert_history_row(&pool, "q-6", "SELECT 1", None, "clickhouse").await;

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/api/query/run/q-6/download?format=csv")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert!(
        ch.received_requests()
            .await
            .expect("mock records requests")
            .is_empty()
    );
}
