//! A query reads only what the caller's tenant may read (`SEC-20`).
//!
//! The shared rewrite refuses a table the caller's tenant may not read,
//! before the statement reaches the engine: an upload table another tenant
//! claimed is refused with a 404 and the fixed sentence `Table not found or
//! not available to your organisation: <name>`. A 403 would confirm the
//! table exists to a caller who must not learn that, and Query Studio
//! reads a 403 as "this user may not run anything at all" and refuses the
//! whole batch.
//!
//! The same rule covers every statement that names a table: a `DESCRIBE`,
//! a `DESC` and a `SHOW COLUMNS FROM` are gated like a `SELECT`. A
//! statement that lists objects without naming one table (`SHOW TABLES`,
//! `SHOW DATABASES`) names no table to check, so it is refused for a
//! tenant that does not own the shared catalog: a 422 with the fixed
//! sentence `This statement is not available to your organisation.`
//!
//! These tests drive the real router over two seeded tenants (`rina@meridian.example`
//! in Meridian Group, `sari@meridian.example` in Meridian Retail), with the
//! shared catalog owned by Meridian Group (`CATALOG_TENANT_ID`) and a
//! `wiremock` `ClickHouse` wired through `CH_URL`, so every test can assert
//! exactly what the engine was sent — the arrangement of
//! `tests/query_settings_rule.rs` and `tests/query_download.rs`.
//!
//! The last test is the other side of the same rule: a deployment that
//! holds one tenant has no other tenant's table to reach, so nothing
//! changes there.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::queries::{RecordHistoryInput, record_history};
use lakehouse_store::uploads::claim_table;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_seeded_user,
    session_cookie_for_user, spin_up_with_env,
};

/// The tenant that owns the shared catalog, and the upload table below.
const OWNER_TENANT: &str = "11111111-1111-4111-8111-000000000001";
/// Another tenant of the same deployment.
const OTHER_TENANT: &str = "11111111-1111-4111-8111-000000000002";
/// The tenants the seeded identity migration creates besides the owner's.
const SEEDED_OTHER_TENANTS: [&str; 2] = [
    "11111111-1111-4111-8111-000000000003",
    "11111111-1111-4111-8111-000000000004",
];

/// The upload table the owner tenant claimed.
const OWNED_TABLE: &str = "group_orders";
/// The statement that reads it.
const OWNED_STATEMENT: &str = "SELECT * FROM bronze.group_orders";
/// The refusal, byte for byte: the table as the rewrite canonicalises it.
const REFUSAL: &str = "Table not found or not available to your organisation: bronze.group_orders";
/// The refusal of a statement that lists objects without naming one table,
/// byte for byte.
const LISTING_REFUSAL: &str = "This statement is not available to your organisation.";

/// The catalog's owner, in a deployment that holds the seeded tenants.
async fn spin_up_with_clickhouse(ch_uri: &str) -> TestApp {
    let mut overrides = HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch_uri.to_owned());
    overrides.insert("CATALOG_TENANT_ID".to_owned(), OWNER_TENANT.to_owned());
    spin_up_with_env(&overrides).await
}

/// Answers the column lookup the rewrite prefetches with an empty table, and
/// any other statement with one `JSON` row.
async fn mount_engine(ch: &MockServer) {
    Mock::given(method("POST"))
        .and(body_string_contains("system.columns"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [
                {"name": "name", "type": "String"},
                {"name": "type", "type": "String"},
                {"name": "default_kind", "type": "String"},
                {"name": "default_expression", "type": "String"},
            ],
            "data": [],
            "rows": 0,
        })))
        .mount(ch)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [
                {"name": "database", "type": "String"},
                {"name": "table", "type": "String"},
                {"name": "rows", "type": "UInt64"},
            ],
            "data": [{"database": "bronze", "table": "group_orders", "rows": 10}],
            "rows": 1,
        })))
        .mount(ch)
        .await;
}

/// The statements the engine received, without the rewrite's own `system.`
/// lookups.
async fn sent_statements(ch: &MockServer) -> Vec<String> {
    ch.received_requests()
        .await
        .expect("mock records requests")
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .filter(|b| !b.contains("system."))
        .collect()
}

/// Every request the engine received, `system.` lookups included.
async fn engine_requests(ch: &MockServer) -> usize {
    ch.received_requests()
        .await
        .expect("mock records requests")
        .len()
}

/// A tenant claims the upload table, the way an upload does.
async fn claim_table_for(pool: &PgPool, tenant: &str) {
    let claimed = claim_table(
        pool,
        tenant.parse::<Uuid>().expect("a seeded tenant id"),
        OWNED_TABLE,
        "upload-group-orders",
    )
    .await
    .expect("claim the upload table for the tenant");
    assert!(claimed, "the tenant must hold the claim");
}

/// The owner tenant claims the upload table, the way an upload does.
async fn claim_the_owned_table(pool: &PgPool) {
    claim_table_for(pool, OWNER_TENANT).await;
}

/// The caller belongs to exactly these tenants, in this order.
///
/// The order is the point: a principal that sends no `X-Tenant` header
/// acts as its FIRST tenant, so a test that says which tenant that is has
/// to put it first. The rows are written in that order, and the primary
/// key's own order agrees with it (`tenant_id` sorts the same way), so
/// the read finds them the way they were written.
async fn belong_to(pool: &PgPool, user: Uuid, tenants: [&str; 2]) {
    sqlx::query("DELETE FROM app_user_tenant WHERE user_id = $1")
        .bind(user)
        .execute(pool)
        .await
        .expect("drop the seeded memberships");
    for tenant in tenants {
        sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
            .bind(user)
            .bind(tenant.parse::<Uuid>().expect("a seeded tenant id"))
            .execute(pool)
            .await
            .expect("add the membership");
    }
}

/// A statement the owner tenant may read, in the catalog's own tenant.
async fn post_sql(
    router: &axum::Router,
    cookie: &str,
    uri: &str,
    sql: &str,
) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(json!({ "sql": sql }).to_string()))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, body)
}

/// The same request, with the header that selects the tenant the caller
/// acts as.
async fn post_sql_for_tenant(
    router: &axum::Router,
    cookie: &str,
    uri: &str,
    sql: &str,
    tenant: &str,
) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .header("x-tenant", tenant)
                .body(Body::from(json!({ "sql": sql }).to_string()))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, body)
}

/// The console's own saved-query list.
async fn get_saved(router: &axum::Router, cookie: &str) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/query/saved")
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, body)
}

/// The console's own "save this statement" button.
async fn post_saved(
    router: &axum::Router,
    cookie: &str,
    title: &str,
    sql: &str,
) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/query/saved")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "title": title, "sql": sql }).to_string(),
                ))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, body)
}

/// One of the chat's tools, called the way the chat calls it.
async fn call_tool(router: &axum::Router, cookie: &str, tool: &str, args: Value) -> Value {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/tool")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "tool": tool, "args": args, "mode": "build" }).to_string(),
                ))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
    serde_json::from_slice(&bytes).expect("a JSON body")
}

/// The titles in a saved-query list.
fn titles(list: &Value) -> Vec<String> {
    list.as_array()
        .expect("a list of saved queries")
        .iter()
        .filter_map(|q| q.get("title").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

/// The download of a run, as the console asks for it.
async fn download_csv(router: &axum::Router, cookie: &str, id: &str) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/query/run/{id}/download?format=csv"))
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, body)
}

/// How many runs the history holds.
async fn history_rows(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM query_history")
        .fetch_one(pool)
        .await
        .expect("read the run history")
}

/// The status of every run the history holds, oldest first.
async fn history_statuses(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT status FROM query_history ORDER BY at, id")
        .fetch_all(pool)
        .await
        .expect("read the run history")
}

/// A run the caller owns, so the download's own ownership check passes.
async fn insert_history_row(pool: &PgPool, id: &str, sql: &str, owner: Uuid) {
    let input = RecordHistoryInput {
        id,
        sql,
        user: &owner.to_string(),
        owner_id: Some(owner),
        status: "completed",
        duration_ms: 12,
        scanned_bytes: 34,
        cost_units: 1.0,
        workload_class: "hot-analytics",
        engine: "clickhouse",
        cache_assisted: false,
        tables_read: &[],
        message: None,
    };
    record_history(pool, &input)
        .await
        .expect("insert a query_history fixture row");
}

async fn user_id(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM app_user WHERE email = $1")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("seeded user {email} must exist (0002_seed_identity.sql): {e}"))
}

/// The deployment holds one tenant: every other tenant and its memberships
/// go, and the caller joins the one that is left.
async fn leave_one_tenant(pool: &PgPool, caller: Uuid) {
    for tenant in [
        OTHER_TENANT,
        SEEDED_OTHER_TENANTS[0],
        SEEDED_OTHER_TENANTS[1],
    ] {
        let tenant_id: Uuid = tenant.parse().expect("a seeded tenant id");
        // The membership rows go first: `app_user_tenant.tenant_id`
        // refuses the deletion of a tenant that still has members.
        sqlx::query("DELETE FROM app_user_tenant WHERE tenant_id = $1")
            .bind(tenant_id)
            .execute(pool)
            .await
            .expect("drop the memberships of the tenant that goes");
        sqlx::query("DELETE FROM tenant WHERE id = $1")
            .bind(tenant_id)
            .execute(pool)
            .await
            .expect("drop the tenant");
    }
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(caller)
        .bind(uuid::uuid!("11111111-1111-4111-8111-000000000001"))
        .execute(pool)
        .await
        .expect("add the caller to the one tenant that is left");
}

#[tokio::test]
async fn another_tenants_upload_table_is_refused_recorded_and_never_asked() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    claim_the_owned_table(&pool).await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let (status, body) = post_sql(&router, &cookie, "/api/query/run", OWNED_STATEMENT).await;

    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], REFUSAL, "{body}");
    assert_eq!(
        history_rows(&pool).await,
        1,
        "the refusal is a run the history now records (`QS-4`, decision 3)"
    );
    assert_eq!(
        engine_requests(&ch).await,
        0,
        "the refusal precedes the engine: nothing may be asked"
    );
}

#[tokio::test]
async fn a_describe_of_another_tenants_upload_table_is_refused_recorded_and_never_asked() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    claim_the_owned_table(&pool).await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let (status, body) = post_sql(
        &router,
        &cookie,
        "/api/query/run",
        "DESCRIBE bronze.group_orders",
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], REFUSAL, "{body}");
    assert_eq!(
        history_rows(&pool).await,
        1,
        "the refusal is a run the history now records (`QS-4`, decision 3)"
    );
    assert_eq!(
        engine_requests(&ch).await,
        0,
        "the refusal precedes the engine: nothing may be asked"
    );
}

#[tokio::test]
async fn a_desc_and_a_show_columns_of_another_tenants_upload_table_are_refused() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    claim_the_owned_table(&pool).await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    // The other two spellings of the same question: the table they name
    // is the one the gate checks, whatever the statement's kind.
    for statement in [
        "DESC bronze.group_orders",
        "SHOW COLUMNS FROM bronze.group_orders",
    ] {
        let (status, body) = post_sql(&router, &cookie, "/api/query/run", statement).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{statement}: {body}");
        assert_eq!(body["error"], REFUSAL, "{statement}: {body}");
    }
    assert_eq!(
        engine_requests(&ch).await,
        0,
        "the refusals precede the engine: nothing may be asked"
    );
}

#[tokio::test]
async fn show_tables_is_refused_for_a_tenant_that_does_not_own_the_catalog() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let (status, body) = post_sql(
        &router,
        &cookie,
        "/api/query/run",
        "SHOW TABLES FROM bronze",
    )
    .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"], LISTING_REFUSAL, "{body}");
    assert_eq!(
        engine_requests(&ch).await,
        0,
        "the refusal precedes the engine: nothing may be asked"
    );
    assert_eq!(
        history_statuses(&pool).await,
        vec!["blocked".to_owned()],
        "the refusal is a run the history records as blocked (`QS-4`, decision 3)"
    );
}

#[tokio::test]
async fn show_databases_is_refused_for_a_tenant_that_does_not_own_the_catalog() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let (status, body) = post_sql(&router, &cookie, "/api/query/run", "SHOW DATABASES").await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"], LISTING_REFUSAL, "{body}");
    assert_eq!(
        engine_requests(&ch).await,
        0,
        "the refusal precedes the engine: nothing may be asked"
    );
}

#[tokio::test]
async fn the_catalog_tenant_still_lists_tables_and_databases() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    // The tenant that owns the shared catalog may list it: the refusal is
    // for a tenant that does not own it.
    for statement in ["SHOW TABLES FROM bronze", "SHOW DATABASES"] {
        let (status, body) = post_sql(&router, &cookie, "/api/query/run", statement).await;
        assert_eq!(status, StatusCode::OK, "{statement}: {body}");
    }
    let sent = sent_statements(&ch).await;
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(sent[0].contains("SHOW TABLES"), "{}", sent[0]);
    assert!(sent[1].contains("SHOW DATABASES"), "{}", sent[1]);
}

#[tokio::test]
async fn the_unrestricted_principal_still_lists_tables() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    // A principal holding every permission (`"*:*"`) reaches every table,
    // so it may list them too.
    let admin = create_principal_with_permissions(&pool, "*:*").await;
    let cookie = session_cookie_for_user(&pool, admin).await;

    let (status, body) = post_sql(
        &router,
        &cookie,
        "/api/query/run",
        "SHOW TABLES FROM bronze",
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let sent = sent_statements(&ch).await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].contains("SHOW TABLES"), "{}", sent[0]);
}

#[tokio::test]
async fn a_deployment_with_one_tenant_still_describes_and_lists() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    claim_the_owned_table(&pool).await;
    let sari = user_id(&pool, "sari@meridian.example").await;
    leave_one_tenant(&pool, sari).await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    // One tenant in the whole deployment: there is no other tenant's
    // table to reach, so nothing changes — not the table, not the listing.
    for statement in ["DESCRIBE bronze.group_orders", "SHOW TABLES FROM bronze"] {
        let (status, body) = post_sql(&router, &cookie, "/api/query/run", statement).await;
        assert_eq!(status, StatusCode::OK, "{statement}: {body}");
    }
    let sent = sent_statements(&ch).await;
    assert_eq!(sent.len(), 2, "{sent:?}");
}

#[tokio::test]
async fn a_select_that_names_no_table_still_runs() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let (status, body) = post_sql(&router, &cookie, "/api/query/run", "SELECT 1").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let sent = sent_statements(&ch).await;
    assert_eq!(sent.len(), 1, "{sent:?}");
}

#[tokio::test]
async fn an_estimate_of_a_describe_of_another_tenants_upload_table_is_refused() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    claim_the_owned_table(&pool).await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let (status, body) = post_sql(
        &router,
        &cookie,
        "/api/query/estimate",
        "DESCRIBE bronze.group_orders",
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], REFUSAL, "{body}");
    assert_eq!(
        engine_requests(&ch).await,
        0,
        "the refusal precedes the explain: nothing may be asked"
    );
}

#[tokio::test]
async fn an_estimate_of_show_databases_is_refused_for_a_tenant_that_does_not_own_the_catalog() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let (status, body) = post_sql(&router, &cookie, "/api/query/estimate", "SHOW DATABASES").await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"], LISTING_REFUSAL, "{body}");
    assert_eq!(
        engine_requests(&ch).await,
        0,
        "the refusal precedes the explain: nothing may be asked"
    );
}

#[tokio::test]
async fn the_chat_cannot_run_another_tenants_upload_table() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    claim_the_owned_table(&pool).await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    // The chat's own `run_sql` admits a `SELECT` only, so this is the
    // shape it can carry: the refusal is the gate's, not the tool's.
    let ran = call_tool(
        &router,
        &cookie,
        "run_sql",
        json!({ "sql": OWNED_STATEMENT }),
    )
    .await;

    assert_eq!(ran["outcome"], json!("failed"), "{ran}");
    assert_eq!(ran["result"]["error"], json!(REFUSAL), "{ran}");
    assert_eq!(
        engine_requests(&ch).await,
        0,
        "the refusal precedes the engine: nothing may be asked"
    );
}

#[tokio::test]
async fn the_owning_tenant_runs_its_own_upload_table() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    claim_the_owned_table(&pool).await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let (status, body) = post_sql(&router, &cookie, "/api/query/run", OWNED_STATEMENT).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let sent = sent_statements(&ch).await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].contains("bronze.group_orders"), "{}", sent[0]);
    assert_eq!(
        history_rows(&pool).await,
        1,
        "a run that happened is recorded"
    );
}

#[tokio::test]
async fn an_estimate_of_another_tenants_upload_table_is_refused() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    claim_the_owned_table(&pool).await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let (status, body) = post_sql(&router, &cookie, "/api/query/estimate", OWNED_STATEMENT).await;

    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], REFUSAL, "{body}");
    assert_eq!(
        engine_requests(&ch).await,
        0,
        "the refusal precedes the explain: nothing may be asked"
    );
}

#[tokio::test]
async fn a_download_of_another_tenants_upload_table_is_refused() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    claim_the_owned_table(&pool).await;
    let sari = user_id(&pool, "sari@meridian.example").await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;
    // A run of the owner's statement, owned by the caller: the download's
    // own ownership check passes, so the refusal under test is the table's.
    insert_history_row(&pool, "q-1700000000000", OWNED_STATEMENT, sari).await;

    let (status, body) = download_csv(&router, &cookie, "q-1700000000000").await;

    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], REFUSAL, "{body}");
    assert_eq!(
        engine_requests(&ch).await,
        0,
        "the refusal precedes the re-run: nothing may be asked"
    );
}

#[tokio::test]
async fn a_deployment_with_one_tenant_runs_the_table_as_before() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    claim_the_owned_table(&pool).await;
    let sari = user_id(&pool, "sari@meridian.example").await;
    leave_one_tenant(&pool, sari).await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let (status, body) = post_sql(&router, &cookie, "/api/query/run", OWNED_STATEMENT).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let sent = sent_statements(&ch).await;
    assert_eq!(sent.len(), 1, "{sent:?}");
}

#[tokio::test]
async fn the_x_tenant_header_selects_the_tenant_that_may_read_the_table() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    // The caller belongs to two tenants: the catalog's own first, and the
    // one that holds the upload table second.
    let rina = user_id(&pool, "rina@meridian.example").await;
    belong_to(&pool, rina, [OWNER_TENANT, OTHER_TENANT]).await;
    claim_table_for(&pool, OTHER_TENANT).await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    // With the header, the caller acts as the tenant that holds the
    // table: its own upload is readable, whoever owns the catalog.
    let (status, body) = post_sql_for_tenant(
        &router,
        &cookie,
        "/api/query/run",
        OWNED_STATEMENT,
        OTHER_TENANT,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Without the header, the caller acts as its FIRST tenant — the
    // catalog's own — and the other tenant's upload is out of reach.
    let (status, body) = post_sql(&router, &cookie, "/api/query/run", OWNED_STATEMENT).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], REFUSAL, "{body}");
}

#[tokio::test]
async fn a_saved_query_of_another_tenant_is_not_listed_and_cannot_be_run() {
    let ch = MockServer::start().await;
    mount_engine(&ch).await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&ch.uri()).await;
    let owner_cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let other_cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    // The catalog's own tenant saves a query.
    let (status, body) = post_saved(
        &router,
        &owner_cookie,
        "Group revenue",
        "SELECT * FROM serving.mart_pelanggan",
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let saved_id = body["id"]
        .as_str()
        .expect("the saved query's id")
        .to_owned();

    // Another tenant's list holds neither that query nor the rows that
    // belong to no tenant: those are the catalog's own.
    let (status, body) = get_saved(&router, &other_cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        !titles(&body).contains(&"Group revenue".to_owned()),
        "another tenant's saved query must not be listed: {body}"
    );
    assert!(
        titles(&body).is_empty(),
        "the rows that belong to no tenant are the catalog's own: {body}"
    );

    // The chat's own list says the same.
    let listed = call_tool(&router, &other_cookie, "list_saved_queries", json!({})).await;
    let queries = listed["result"]["queries"]
        .as_array()
        .expect("the tool's list of saved queries");
    assert!(
        queries.is_empty(),
        "another tenant's saved query must not be listed: {listed}"
    );

    // And the chat cannot run it by id: it is not found at all.
    let ran = call_tool(
        &router,
        &other_cookie,
        "run_saved_query",
        json!({ "id": saved_id }),
    )
    .await;
    assert_eq!(ran["outcome"], json!("failed"), "{ran}");
    assert_eq!(
        ran["result"]["error"],
        json!("saved query tidak ditemukan"),
        "{ran}"
    );

    // The catalog's own tenant sees its query, and the rows that belong
    // to no tenant.
    let (status, body) = get_saved(&router, &owner_cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let listed = titles(&body);
    assert!(
        listed.contains(&"Group revenue".to_owned()),
        "the catalog's tenant sees its own query: {body}"
    );
    assert!(
        listed.len() > 1,
        "the rows that belong to no tenant are the catalog's own: {body}"
    );
}
