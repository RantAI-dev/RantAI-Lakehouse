//! `GET /api/query/run/{id}/download` — owner-scoped re-run download
//! (`QS-2`).
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

use std::collections::HashMap;
use std::io::Read;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::governance::{self, CreatePolicyInput};
use lakehouse_store::queries::{RecordHistoryInput, record_history};
use lakehouse_store::uploads::claim_table;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

/// The tenant that owns the shared catalog, and the upload table below.
const OWNER_TENANT: &str = "11111111-1111-4111-8111-000000000001";
/// The upload table the owner tenant claimed.
const OWNED_TABLE: &str = "group_orders";
/// The refusal of a table of another tenant, byte for byte.
const REFUSAL: &str = "Table not found or not available to your organisation: bronze.group_orders";

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
        tables_read: &[],
        message: None,
    };
    record_history(pool, &input)
        .await
        .expect("insert a query_history fixture row");
}

/// The download of a run, as the console asks for it: the format, and the
/// run the download belongs to when the caller names one.
async fn download_with_cookie(
    router: &axum::Router,
    id: &str,
    query: &str,
    cookie: &str,
) -> axum::http::Response<Body> {
    router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/query/run/{id}/download?{query}"))
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright")
}

/// The error body of a refusal, as the console reads it.
async fn error_body(resp: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(&bytes).into_owned()))
}

/// The engine's row stream: the column names, the column types, then one
/// JSON array per row — the shape `JSONCompactEachRowWithNamesAndTypes`
/// writes, and the one the row path (the comma file, the tab-separated one
/// and the workbook) reads.
fn row_stream(name: &str, rows: usize) -> String {
    let mut body = format!("[\"{name}\"]\n[\"Int64\"]\n");
    for row in 1..=rows {
        body.push('[');
        body.push_str(&row.to_string());
        body.push_str("]\n");
    }
    body
}

/// The engine's row stream for one text column: the same shape, with the
/// values as they are.
fn text_row_stream(name: &str, values: &[&str]) -> String {
    let mut body = format!("[\"{name}\"]\n[\"String\"]\n");
    for value in values {
        body.push('[');
        body.push_str(&serde_json::to_string(value).expect("a json string"));
        body.push_str("]\n");
    }
    body
}

/// A `ClickHouse`-shaped `wiremock` stand-in, wired via `CH_URL` — see
/// `common::spin_up_with_env`'s doc comment on why every other upstream
/// stays the fail-fast `DEAD_UPSTREAM` default while this one test file
/// needs a real HTTP endpoint to assert requests against.
async fn spin_up_with_clickhouse(ch_uri: &str) -> TestApp {
    let mut overrides = std::collections::HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch_uri.to_owned());
    // The shared catalog belongs to one tenant, and the query path is
    // scoped to the tenant: this principal must be the catalog's own.
    overrides.insert("CATALOG_TENANT_ID".to_owned(), OWNER_TENANT.to_owned());
    spin_up_with_env(&overrides).await
}

#[tokio::test]
async fn a_different_owner_gets_404() {
    let ch = MockServer::start().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;

    let owner_id = seeded_user_id(&pool, "rina@meridian.example").await;
    let stranger_cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;
    insert_history_row(&pool, "q-1", "SELECT 1", Some(owner_id), "clickhouse").await;

    let resp = download_with_cookie(&router, "q-1", "format=csv", &stranger_cookie).await;
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

    let resp = download_with_cookie(&router, "q-2", "format=csv", &cookie).await;
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

    let resp = download_with_cookie(&router, "q-3", "format=csv", &cookie).await;
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

    let resp = download_with_cookie(&router, "q-4", "format=csv", &cookie).await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = error_body(resp).await;
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
        .respond_with(ResponseTemplate::new(200).set_body_string(row_stream("n", 2)))
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

    let resp = download_with_cookie(&router, "q-5", "format=csv", &cookie).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "text/csv; charset=utf-8"
    );
    assert_eq!(
        resp.headers().get("content-disposition").unwrap(),
        "attachment; filename=\"q-5.csv\""
    );
    assert_eq!(resp.headers().get("x-export-rows").unwrap(), "2");
    assert_eq!(resp.headers().get("x-export-cut").unwrap(), "false");
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&bytes[..], "\u{feff}n\r\n1\r\n2\r\n".as_bytes());

    let requests = ch.received_requests().await.expect("mock records requests");
    assert_eq!(requests.len(), 1, "exactly one re-run reached ClickHouse");
    let sent_body = String::from_utf8(requests[0].body.clone()).expect("utf8 request body");
    assert!(
        sent_body.contains("SELECT n FROM t"),
        "the stored statement must be the one that runs: {sent_body}"
    );
    assert!(
        sent_body.contains("FORMAT JSONCompactEachRowWithNamesAndTypes"),
        "the rows are read in the shape the row path writes: {sent_body}"
    );
    assert!(
        !sent_body.contains("LIMIT"),
        "the download holds the whole result, so it caps nothing: {sent_body}"
    );
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

// ── The download applies the same policy rewrite as the run ──────────────

/// Answers the column lookup the rewrite makes, then any other statement
/// with a row stream holding one masked column.
async fn mount_mart_columns_then_rows(ch: &MockServer) {
    Mock::given(method("POST"))
        .and(body_string_contains("system.columns"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "meta": [
                {"name": "name", "type": "String"},
                {"name": "type", "type": "String"},
                {"name": "default_kind", "type": "String"},
                {"name": "default_expression", "type": "String"},
            ],
            "data": [
                {"name": "email", "type": "String", "default_kind": "", "default_expression": ""},
                {"name": "region", "type": "String", "default_kind": "", "default_expression": ""},
            ],
            "rows": 2,
        })))
        .mount(ch)
        .await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(text_row_stream("email", &["***"])),
        )
        .mount(ch)
        .await;
}

async fn seed_mask_and_filter_policy(pool: &PgPool) {
    governance::create_policy(
        pool,
        &CreatePolicyInput {
            name: "download-rewrite-test".to_owned(),
            kind: "Row filter".to_owned(),
            subjects: "Analyst".to_owned(),
            resources: "serving.mart_x".to_owned(),
            effect: "Permit with obligation".to_owned(),
            conditions: Some(
                r#"{"roles":["Analyst"],"table":"serving.mart_x","mask":["email"]}"#.to_owned(),
            ),
            activate: true,
            owner: None,
        },
    )
    .await
    .expect("seed the governing policy");
}

async fn sent_statements(ch: &MockServer) -> Vec<String> {
    ch.received_requests()
        .await
        .expect("mock records requests")
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .filter(|b| !b.contains("system."))
        .collect()
}

#[tokio::test]
async fn a_download_is_masked_for_the_callers_role() {
    let ch = MockServer::start().await;
    mount_mart_columns_then_rows(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    seed_mask_and_filter_policy(&pool).await;

    let owner_id = seeded_user_id(&pool, "rina@meridian.example").await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    insert_history_row(
        &pool,
        "q-10",
        "SELECT email FROM serving.mart_x",
        Some(owner_id),
        "clickhouse",
    )
    .await;

    let resp = download_with_cookie(&router, "q-10", "format=csv", &cookie).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let sent = sent_statements(&ch).await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(
        sent[0].contains("replaceRegexpOne(toString(`email`)"),
        "{}",
        sent[0]
    );
}

#[tokio::test]
async fn a_role_the_policy_does_not_govern_downloads_the_statement_unchanged() {
    let ch = MockServer::start().await;
    mount_mart_columns_then_rows(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    // The policy governs a different role than the caller's.
    governance::create_policy(
        &pool,
        &CreatePolicyInput {
            name: "download-rewrite-other-role".to_owned(),
            kind: "Row filter".to_owned(),
            subjects: "Auditor".to_owned(),
            resources: "serving.mart_x".to_owned(),
            effect: "Permit with obligation".to_owned(),
            conditions: Some(
                r#"{"roles":["Auditor"],"table":"serving.mart_x","mask":["email"]}"#.to_owned(),
            ),
            activate: true,
            owner: None,
        },
    )
    .await
    .expect("seed a policy for another role");

    let owner_id = seeded_user_id(&pool, "rina@meridian.example").await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    insert_history_row(
        &pool,
        "q-11",
        "SELECT email FROM serving.mart_x",
        Some(owner_id),
        "clickhouse",
    )
    .await;

    let resp = download_with_cookie(&router, "q-11", "format=csv", &cookie).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let sent = sent_statements(&ch).await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(!sent[0].contains("replaceRegexpOne"), "{}", sent[0]);
    assert!(
        sent[0].contains("SELECT email FROM serving.mart_x"),
        "{}",
        sent[0]
    );
}

#[tokio::test]
async fn a_statement_the_rewrite_refuses_is_refused_with_the_run_routes_status_and_never_sent() {
    let ch = MockServer::start().await;
    mount_mart_columns_then_rows(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;

    let owner_id = seeded_user_id(&pool, "rina@meridian.example").await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    // A table function is one of the shapes the rewrite refuses outright.
    insert_history_row(
        &pool,
        "q-12",
        "SELECT * FROM numbers(5)",
        Some(owner_id),
        "clickhouse",
    )
    .await;

    let resp = download_with_cookie(&router, "q-12", "format=csv", &cookie).await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = error_body(resp).await;
    let text = body["error"].as_str().unwrap_or_default().to_owned();
    assert!(
        !text.contains("numbers"),
        "no statement text in the refusal: {text}"
    );
    assert!(
        sent_statements(&ch).await.is_empty(),
        "a refused statement must not reach the engine"
    );
}

/// `SEC-17`: a cell that would run as a formula in a spreadsheet gets a
/// leading apostrophe in every text format; a number in a number column is
/// untouched.
#[tokio::test]
async fn a_cell_that_starts_a_formula_is_guarded_in_every_text_format() {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("[\"txt\",\"num\"]\n[\"String\",\"Int64\"]\n[\"=1+1\",\"-5\"]\n"),
        )
        .mount(&ch)
        .await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    let owner_id = seeded_user_id(&pool, "rina@meridian.example").await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    insert_history_row(
        &pool,
        "q-7",
        "SELECT n FROM t",
        Some(owner_id),
        "clickhouse",
    )
    .await;

    let resp = download_with_cookie(&router, "q-7", "format=csv", &cookie).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&bytes[..], "\u{feff}txt,num\r\n'=1+1,-5\r\n".as_bytes());

    let resp = download_with_cookie(&router, "q-7", "format=tsv", &cookie).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&bytes[..], "\u{feff}txt\tnum\r\n'=1+1\t-5\r\n".as_bytes());

    let resp = download_with_cookie(&router, "q-7", "format=xlsx", &cookie).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let text = workbook_text(bytes.to_vec());
    assert!(
        text.contains("'=1+1") || text.contains("&apos;=1+1"),
        "the formula-looking cell is text in the workbook: {text}"
    );
}

/// `SEC-17`: Parquet has typed columns and runs no formula, so its bytes are
/// returned exactly as `ClickHouse` wrote them, even when they look like CSV.
#[tokio::test]
async fn a_parquet_download_is_returned_unchanged() {
    let ch = MockServer::start().await;
    let body: &[u8] = b"PAR1\x00=1+1,\"=2\"\n\xff\xfe";
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(&ch)
        .await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    let owner_id = seeded_user_id(&pool, "rina@meridian.example").await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    insert_history_row(
        &pool,
        "q-8",
        "SELECT n FROM t",
        Some(owner_id),
        "clickhouse",
    )
    .await;

    let resp = download_with_cookie(&router, "q-8", "format=parquet", &cookie).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&bytes[..], body);
}

// ── The whole result, every format ───────────────────────────────────────

/// The workbook's cells and styles, read out of the file the download
/// returned: the strings a reader sees, in the order the sheet holds them.
///
/// The workbook is written in constant-memory mode, so its strings sit in the
/// sheet itself and there is no shared-strings table to read.
fn workbook_text(bytes: Vec<u8>) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = String::new();
    for name in ["xl/sharedStrings.xml", "xl/worksheets/sheet1.xml"] {
        let Ok(mut file) = zip.by_name(name) else {
            continue;
        };
        let mut text = String::new();
        file.read_to_string(&mut text).unwrap();
        out.push_str(&text);
    }
    out
}

/// `QS-2`: a download holds the whole result, not the first ten thousand
/// rows. Every format answers with all of it, and the three the row path
/// writes say how many rows they wrote.
#[tokio::test]
async fn every_format_answers_with_the_whole_result_and_says_how_many_rows_it_wrote() {
    let ch = MockServer::start().await;
    // The `Parquet` file is written by the engine, so its mock answers the
    // statement that asks for one; the row stream answers the other three.
    // The first mock mounted wins, so the specific one goes first.
    Mock::given(method("POST"))
        .and(body_string_contains("FORMAT Parquet"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"PAR1\x00whole\xffPAR1".to_vec()))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(row_stream("n", 12_000)))
        .mount(&ch)
        .await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;

    let owner_id = seeded_user_id(&pool, "rina@meridian.example").await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    insert_history_row(
        &pool,
        "q-20",
        "SELECT n FROM t",
        Some(owner_id),
        "clickhouse",
    )
    .await;

    for format in ["csv", "tsv", "xlsx"] {
        let resp =
            download_with_cookie(&router, "q-20", &format!("format={format}"), &cookie).await;
        assert_eq!(resp.status(), StatusCode::OK, "{format}");
        assert_eq!(
            resp.headers().get("x-export-rows").unwrap(),
            "12000",
            "{format}"
        );
        assert_eq!(
            resp.headers().get("x-export-cut").unwrap(),
            "false",
            "{format}"
        );
    }

    // The `Parquet` file comes back whole, and no row count is claimed: the
    // engine wrote that file, this service never saw its rows.
    let resp = download_with_cookie(&router, "q-20", "format=parquet", &cookie).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers().get("x-export-cut").unwrap(), "false");
    assert!(resp.headers().get("x-export-rows").is_none());
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&bytes[..], b"PAR1\x00whole\xffPAR1");
}

/// `SEC-21`: the mask that governs the caller today governs the file, in
/// every format.
#[tokio::test]
async fn a_masked_column_is_masked_in_every_format() {
    let ch = MockServer::start().await;
    mount_mart_columns_then_rows(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    seed_mask_and_filter_policy(&pool).await;

    let owner_id = seeded_user_id(&pool, "rina@meridian.example").await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    insert_history_row(
        &pool,
        "q-21",
        "SELECT email FROM serving.mart_x",
        Some(owner_id),
        "clickhouse",
    )
    .await;

    for format in ["csv", "tsv", "xlsx", "parquet"] {
        let resp =
            download_with_cookie(&router, "q-21", &format!("format={format}"), &cookie).await;
        assert_eq!(resp.status(), StatusCode::OK, "{format}");
    }
    let sent = sent_statements(&ch).await;
    assert_eq!(sent.len(), 4, "{sent:?}");
    for statement in &sent {
        assert!(
            statement.contains("replaceRegexpOne(toString(`email`)"),
            "{statement}"
        );
    }
}

// ── The refusals that must reach no engine ───────────────────────────────

/// `SEC-20`: a table of another tenant is refused before the engine is
/// asked, whatever the statement's kind.
#[tokio::test]
async fn a_table_of_another_tenant_is_refused_before_the_engine_is_asked() {
    let ch = MockServer::start().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    claim_table(
        &pool,
        OWNER_TENANT.parse::<Uuid>().unwrap(),
        OWNED_TABLE,
        "upload-group-orders",
    )
    .await
    .expect("claim the upload table for the owner tenant");
    let sari = seeded_user_id(&pool, "sari@meridian.example").await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    for (id, statement) in [
        ("q-30", "SELECT * FROM bronze.group_orders"),
        ("q-31", "DESCRIBE bronze.group_orders"),
    ] {
        insert_history_row(&pool, id, statement, Some(sari), "clickhouse").await;
        let resp = download_with_cookie(&router, id, "format=csv", &cookie).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{statement}");
        let body = error_body(resp).await;
        assert_eq!(body["error"], REFUSAL, "{body}");
    }
    assert_eq!(
        ch.received_requests().await.unwrap().len(),
        0,
        "the refusal precedes the re-run: nothing may be asked"
    );
}

/// `BI-1` D7: a role that is switched off from downloading may not download
/// here either, and nothing is read from the engine for them.
#[tokio::test]
async fn a_role_that_may_not_download_is_refused_before_the_engine_is_asked() {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(row_stream("n", 1)))
        .mount(&ch)
        .await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    // Every role the caller holds is switched off: the rule is about the
    // roles a person has, not about one of them.
    let roles: Vec<String> = sqlx::query_scalar(
        "SELECT r.name FROM app_user_role ur JOIN role r ON r.id = ur.role_id \
         JOIN app_user u ON u.id = ur.user_id WHERE u.email = 'rina@meridian.example'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(!roles.is_empty(), "the seeded user holds at least one role");
    sqlx::query(
        "INSERT INTO reporting_settings (singleton, time_zone, week_start, download_denied_roles) \
         VALUES (TRUE, 'UTC', 'monday', $1)",
    )
    .bind(&roles)
    .execute(&pool)
    .await
    .unwrap();

    let owner_id = seeded_user_id(&pool, "rina@meridian.example").await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    insert_history_row(
        &pool,
        "q-40",
        "SELECT n FROM t",
        Some(owner_id),
        "clickhouse",
    )
    .await;

    let resp = download_with_cookie(&router, "q-40", "format=csv", &cookie).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body = error_body(resp).await;
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|e| e.contains("Downloads are switched off for your role.")),
        "{body}"
    );
    assert_eq!(
        ch.received_requests().await.unwrap().len(),
        0,
        "a refused download reads nothing"
    );
}

/// `SEC-11`: the engine's own text never reaches a response. The caller
/// gets a fixed sentence and a reference id.
#[tokio::test]
async fn an_engine_failure_answers_a_fixed_sentence_with_a_reference_and_no_engine_text() {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string(
            "Code: 47. DB::Exception: Unknown identifier: nope. (SYNTAX_ERROR) (version 26.8.1)",
        ))
        .mount(&ch)
        .await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;

    let owner_id = seeded_user_id(&pool, "rina@meridian.example").await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    insert_history_row(
        &pool,
        "q-50",
        "SELECT n FROM t",
        Some(owner_id),
        "clickhouse",
    )
    .await;

    let resp = download_with_cookie(&router, "q-50", "format=csv", &cookie).await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = error_body(resp).await;
    let text = body["error"].as_str().unwrap_or_default();
    assert!(
        text.starts_with("The database request failed. Reference: "),
        "{text}"
    );
    assert!(!text.contains("Unknown identifier"), "{text}");
    assert!(!text.contains("DB::Exception"), "{text}");
    assert!(!text.contains("SYNTAX_ERROR"), "{text}");
}

// ── What the request may ask for ─────────────────────────────────────────

/// The download names its query after the caller and the run, so Stop's
/// pattern (`<caller uuid>:<runId>:%`) finds it.
#[tokio::test]
async fn the_download_names_its_query_after_the_caller_and_the_run() {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(row_stream("n", 1)))
        .mount(&ch)
        .await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;

    let owner_id = seeded_user_id(&pool, "rina@meridian.example").await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    insert_history_row(
        &pool,
        "q-60",
        "SELECT n FROM t",
        Some(owner_id),
        "clickhouse",
    )
    .await;

    let run_id = "22222222-2222-4222-8222-222222222222";
    let resp = download_with_cookie(
        &router,
        "q-60",
        &format!("format=csv&runId={run_id}"),
        &cookie,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let requests = ch.received_requests().await.expect("mock records requests");
    let settings: HashMap<String, String> = requests[0].url.query_pairs().into_owned().collect();
    assert_eq!(
        settings.get("query_id").map(String::as_str),
        Some(format!("{owner_id}:{run_id}:download").as_str()),
        "{settings:?}"
    );
}

/// A format the route does not offer is refused, and so is a run id that is
/// not a `UUID`.
#[tokio::test]
async fn a_format_the_route_does_not_offer_is_refused() {
    let ch = MockServer::start().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;

    let owner_id = seeded_user_id(&pool, "rina@meridian.example").await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    insert_history_row(
        &pool,
        "q-70",
        "SELECT n FROM t",
        Some(owner_id),
        "clickhouse",
    )
    .await;

    let resp = download_with_cookie(&router, "q-70", "format=pdf", &cookie).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = error_body(resp).await;
    assert_eq!(body["error"], "format must be csv, tsv, xlsx or parquet");
    assert_eq!(
        ch.received_requests().await.unwrap().len(),
        0,
        "a refused format reads nothing"
    );
}

#[tokio::test]
async fn a_run_id_that_is_not_a_uuid_is_refused() {
    let ch = MockServer::start().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;

    let owner_id = seeded_user_id(&pool, "rina@meridian.example").await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    insert_history_row(
        &pool,
        "q-71",
        "SELECT n FROM t",
        Some(owner_id),
        "clickhouse",
    )
    .await;

    let resp = download_with_cookie(&router, "q-71", "format=csv&runId=nope", &cookie).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = error_body(resp).await;
    assert_eq!(body["error"], "runId must be a UUID");
    assert_eq!(
        ch.received_requests().await.unwrap().len(),
        0,
        "a refused run id reads nothing"
    );
}

/// The headers the console reads: the file's name and kind, its length, and
/// the counts of the rows it holds.
#[tokio::test]
async fn the_download_carries_the_headers_the_console_reads() {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(row_stream("n", 1)))
        .mount(&ch)
        .await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;

    let owner_id = seeded_user_id(&pool, "rina@meridian.example").await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    insert_history_row(
        &pool,
        "q-80",
        "SELECT n FROM t",
        Some(owner_id),
        "clickhouse",
    )
    .await;

    let resp = download_with_cookie(&router, "q-80", "format=csv", &cookie).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "text/csv; charset=utf-8"
    );
    assert_eq!(
        resp.headers().get("content-disposition").unwrap(),
        "attachment; filename=\"q-80.csv\""
    );
    assert_eq!(resp.headers().get("cache-control").unwrap(), "no-store");
    assert_eq!(resp.headers().get("x-export-rows").unwrap(), "1");
    assert_eq!(resp.headers().get("x-export-cut").unwrap(), "false");
    assert!(
        resp.headers().get("x-export-cut-by").is_none(),
        "a file that was not cut names no limit"
    );
    let length: usize = resp
        .headers()
        .get("content-length")
        .expect("the file's length")
        .to_str()
        .expect("a length of digits")
        .parse()
        .expect("a number of bytes");
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes.len(), length);
    assert_eq!(bytes.len(), "\u{feff}n\r\n1\r\n".len());
}
