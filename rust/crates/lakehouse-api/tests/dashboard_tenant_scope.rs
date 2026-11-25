//! `SEC-50` slice 2: a dashboard reads as the caller's own tenant.
//!
//! The shared catalog belongs to the tenant named in `CATALOG_TENANT_ID`.
//! A member of another tenant must not read `serving.*` rows through a
//! dashboard, even though the same statement is refused by Query Studio
//! (`SEC-20`'s 404, `table_not_available`).
//!
//! The tile's own error slot carries the refusal, so the board still renders
//! and the person sees why the tile is empty; and the refusal happens BEFORE
//! the policy rewrite, so a refused tile never reaches the result cache and
//! never reaches the engine.
//!
//! The arrangement is `tests/query_tenant_scope.rs`'s: a real router over a
//! real Postgres, and a `wiremock` `ClickHouse` that records every statement
//! it is sent — so a test can say exactly what the engine was asked.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};
use lakehouse_store::reporting_settings::ReportingSettings;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

/// The tenant that owns the shared catalog, and the one the owner reads as.
const OWNER_TENANT: &str = "11111111-1111-4111-8111-000000000001";
/// Another tenant of the same deployment.
const OTHER_TENANT: &str = "11111111-1111-4111-8111-000000000002";
/// The tenants the seeded identity migration creates besides the owner's.
const SEEDED_OTHER_TENANTS: [&str; 2] = [
    "11111111-1111-4111-8111-000000000003",
    "11111111-1111-4111-8111-000000000004",
];

/// The mart the board's one tile reads.
const MART: &str = "mart_x";
/// The tile's id.
const TILE: &str = "c_a";
/// The refusal of that tile, byte for byte: the table as the gate
/// canonicalises it.
const REFUSAL: &str = "Table not found or not available to your organisation: serving.mart_x";

/// A `ClickHouse` that serves one board with one tile, and records every
/// statement it is sent.
#[derive(Clone)]
struct FakeCh {
    charts: Arc<Vec<Value>>,
    statements: Arc<Mutex<Vec<String>>>,
}

impl FakeCh {
    fn new() -> Self {
        Self::with_charts(vec![chart_row()])
    }

    /// A fake serving `charts` instead of the default tile.
    fn with_charts(charts: Vec<Value>) -> Self {
        Self {
            charts: Arc::new(charts),
            statements: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Every statement the engine was sent.
    fn statements(&self) -> Vec<String> {
        self.statements.lock().expect("the statements lock").clone()
    }

    /// Serve this fake on a fresh `ClickHouse`.
    async fn serve(&self) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(self.clone())
            .mount(&server)
            .await;
        server
    }

    /// The statements that read the tile's mart — the tile's own query.
    ///
    /// The fingerprint read (`system.parts`) names the mart too, and is not
    /// the tile's query: it is left out.
    fn tile_statements(&self) -> Vec<String> {
        self.statements()
            .into_iter()
            .filter(|sql| {
                sql.contains(&format!("serving.{MART}")) && !sql.contains("FROM system.parts")
            })
            .collect()
    }
}

impl Respond for FakeCh {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        self.statements
            .lock()
            .expect("the statements lock")
            .push(sql.clone());
        // The board's one tile.
        if sql.contains("FROM console.bi_chart") {
            return rows(
                &["id", "board", "created_by", "created_at", "spec_json"],
                &self.charts,
            );
        }
        // The column probe of a SQL source (`DESCRIBE (SELECT * FROM ...)`).
        if sql.contains("DESCRIBE") {
            return rows(
                &["name", "type"],
                &[
                    json!({ "name": "kab", "type": "String" }),
                    json!({ "name": "visitors", "type": "UInt32" }),
                ],
            );
        }
        // The board itself: the built-in one, which has no stored row.
        if sql.contains("FROM console.bi_board") {
            return rows(&["id", "name"], &[]);
        }
        // The columns of the mart the tile reads.
        if sql.contains("FROM system.columns") {
            return rows(
                &[
                    "table",
                    "name",
                    "type",
                    "default_kind",
                    "default_expression",
                ],
                &[
                    json!({ "table": MART, "name": "kab", "type": "String",
                            "default_kind": "", "default_expression": "" }),
                    json!({ "table": MART, "name": "visitors", "type": "UInt32",
                            "default_kind": "", "default_expression": "" }),
                ],
            );
        }
        // The fingerprint of the mart, so a remembered result stays valid.
        if sql.contains("FROM system.parts") {
            return rows(
                &["database", "table", "modified", "row_count"],
                &[json!({
                    "database": "serving", "table": MART,
                    "modified": "2026-01-01 00:00:00", "row_count": "10",
                })],
            );
        }
        // The rows of a download: they arrive as a stream (the names, the
        // types, then one array a row).
        if sql.contains("JSONCompactEachRowWithNamesAndTypes") {
            return ResponseTemplate::new(200)
                .set_body_string("[\"kab\",\"visitors\"]\n[\"String\",\"UInt32\"]\n[\"a\",1]\n");
        }
        // The total of a page of rows.
        if sql.contains("count() AS n") {
            return rows(&["n"], &[json!({ "n": 1 })]);
        }
        // The tile's own query.
        if sql.contains(&format!("serving.{MART}")) {
            return rows(
                &["kab", "visitors"],
                &[json!({ "kab": "a", "visitors": 1 })],
            );
        }
        // The store's own table checks.
        if sql.starts_with("CREATE") || sql.starts_with("ALTER") || sql.starts_with("DROP") {
            return ResponseTemplate::new(200).set_body_string("Ok.\n");
        }
        // Anything else (the sources, the catalog fields): success, no rows.
        rows(&[], &[])
    }
}

/// A `console.bi_chart` row for a raw table: a tile in rows mode, which is
/// what the table export reads.
fn table_chart_row() -> Value {
    let spec = json!({
        "id": "c_table", "title": "Table", "kind": "table", "mart": MART,
        "sql": "SELECT kab, visitors FROM serving.mart_x",
        "x": "kab", "y": "visitors"
    });
    let def = json!({
        "title": "Table", "mart": MART, "kind": "table",
        "dimension": "kab", "measures": ["visitors"],
        "tableMode": "rows", "columns": ["kab", "visitors"]
    });
    json!({
        "id": "c_table", "board": "", "created_by": "ui", "created_at": "2026-01-01 00:00:00",
        "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
    })
}

/// A `console.bi_chart` row: one bar tile on the mart, on the built-in board.
///
/// The stored `sql` is the statement the tile runs when no filter applies
/// (the builder keeps the stored text byte-for-byte then), so it names the
/// mart the gate checks.
fn chart_row() -> Value {
    let spec = json!({
        "id": TILE, "title": "Tile", "kind": "bar", "mart": MART,
        "sql": "SELECT kab, round(sum(visitors)) AS visitors FROM serving.mart_x GROUP BY kab ORDER BY kab",
        "x": "kab", "y": "visitors"
    });
    let def = json!({
        "title": "Tile", "mart": MART, "kind": "bar",
        "dimension": "kab", "measures": ["visitors"]
    });
    json!({
        "id": TILE, "board": "", "created_by": "ui", "created_at": "2026-01-01 00:00:00",
        "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
    })
}

/// A `ClickHouse` answer with `meta` and `data`.
fn rows(meta: &[&str], data: &[Value]) -> ResponseTemplate {
    let meta: Vec<Value> = meta
        .iter()
        .map(|name| json!({ "name": name, "type": "String" }))
        .collect();
    let count = data.len();
    ResponseTemplate::new(200).set_body_json(json!({
        "meta": meta, "data": data, "rows": count,
    }))
}

/// The catalog's owner is named, so the shared catalog has a tenant.
async fn spin_up_with_clickhouse(ch_uri: &str) -> TestApp {
    let mut overrides = HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch_uri.to_owned());
    overrides.insert("CATALOG_TENANT_ID".to_owned(), OWNER_TENANT.to_owned());
    spin_up_with_env(&overrides).await
}

/// A person who may read dashboards and belongs to `tenant` and to no other.
async fn member_of(pool: &PgPool, tenant: &str) -> Uuid {
    member_with(pool, tenant, "dashboard:read").await
}

/// A person holding `permissions` who belongs to `tenant` and to no other.
async fn member_with(pool: &PgPool, tenant: &str, permissions: &str) -> Uuid {
    let user = create_principal_with_permissions(pool, permissions).await;
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(user)
        .bind(tenant.parse::<Uuid>().expect("a seeded tenant id"))
        .execute(pool)
        .await
        .expect("add the membership");
    user
}

/// The deployment holds one tenant: every other tenant and its memberships go.
async fn leave_one_tenant(pool: &PgPool) {
    for tenant in [
        OTHER_TENANT,
        SEEDED_OTHER_TENANTS[0],
        SEEDED_OTHER_TENANTS[1],
    ] {
        let tenant_id: Uuid = tenant.parse().expect("a seeded tenant id");
        // The membership rows go first: `app_user_tenant.tenant_id` refuses
        // the deletion of a tenant that still has members.
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
}

/// The dashboard, as the console asks for it.
async fn get_dashboard(router: &axum::Router, cookie: &str) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/dashboard")
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

/// The settings that let a result be remembered for an hour — the cache test
/// needs the tile to be remembered, so the second read could have been
/// served from memory.
fn remembering_settings() -> ReportingSettings {
    ReportingSettings {
        time_zone: "UTC".to_owned(),
        week_start: "monday".to_owned(),
        number_locale: "en-US".to_owned(),
        company_palette: Vec::new(),
        fonts: Vec::new(),
        content_sites: Vec::new(),
        download_denied_roles: Vec::new(),
        report_allowed_domains: Vec::new(),
        public_links_enabled: true,
        result_cache_mode: "minutes".to_owned(),
        result_cache_minutes: Some(60),
    }
}

/// `SEC-50` slice 2: a member of another tenant sees the tile refused, and
/// the engine is never asked for the tile's rows.
#[tokio::test]
async fn another_tenants_member_sees_the_tile_refused_and_the_engine_is_never_asked() {
    let fake = FakeCh::new();
    let server = fake.serve().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&server.uri()).await;
    let other = member_of(&pool, OTHER_TENANT).await;
    let cookie = session_cookie_for_user(&pool, other).await;

    let (status, body) = get_dashboard(&router, &cookie).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["results"][TILE]["error"], REFUSAL, "{body}");
    assert!(
        body["results"][TILE].get("rows").is_none(),
        "the refused tile has no rows: {body}"
    );
    let asked = fake.tile_statements();
    assert!(
        asked.is_empty(),
        "the refusal precedes the engine: nothing may be asked ({asked:?})"
    );
}

/// `SEC-50` slice 2: the tenant that owns the shared catalog reads the rows.
#[tokio::test]
async fn the_catalog_tenant_reads_the_tile() {
    let fake = FakeCh::new();
    let server = fake.serve().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&server.uri()).await;
    let owner = member_of(&pool, OWNER_TENANT).await;
    let cookie = session_cookie_for_user(&pool, owner).await;

    let (status, body) = get_dashboard(&router, &cookie).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["results"][TILE].get("error").is_none(),
        "the owning tenant reads its own catalog: {body}"
    );
    assert_eq!(
        body["results"][TILE]["rows"].as_array().map(Vec::len),
        Some(1),
        "{body}"
    );
    assert!(
        !fake.tile_statements().is_empty(),
        "the tile's own statement reached the engine"
    );
}

/// `SEC-50` slice 2 R1: the unrestricted principal reads every table, so the
/// gate changes nothing for it.
#[tokio::test]
async fn the_unrestricted_principal_reads_the_tile() {
    let fake = FakeCh::new();
    let server = fake.serve().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&server.uri()).await;
    let admin = create_principal_with_permissions(&pool, "*:*").await;
    let cookie = session_cookie_for_user(&pool, admin).await;

    let (status, body) = get_dashboard(&router, &cookie).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["results"][TILE].get("error").is_none(),
        "an unrestricted principal reads every table: {body}"
    );
    assert_eq!(
        body["results"][TILE]["rows"].as_array().map(Vec::len),
        Some(1),
        "{body}"
    );
}

/// `SEC-50` slice 2 D2: a deployment with one tenant holds no other tenant's
/// table to reach, so nothing changes there.
#[tokio::test]
async fn a_deployment_with_one_tenant_reads_the_tile() {
    let fake = FakeCh::new();
    let server = fake.serve().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&server.uri()).await;
    leave_one_tenant(&pool).await;
    let owner = member_of(&pool, OWNER_TENANT).await;
    let cookie = session_cookie_for_user(&pool, owner).await;

    let (status, body) = get_dashboard(&router, &cookie).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["results"][TILE].get("error").is_none(),
        "one tenant in the deployment: nothing changes: {body}"
    );
    assert_eq!(
        body["results"][TILE]["rows"].as_array().map(Vec::len),
        Some(1),
        "{body}"
    );
}

/// `SEC-50` slice 2 M3: the gate runs BEFORE the result cache, so a tile the
/// owner had remembered is refused for another tenant rather than served
/// from memory.
#[tokio::test]
async fn a_refused_reader_is_not_served_from_the_remembered_tile() {
    let fake = FakeCh::new();
    let server = fake.serve().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&server.uri()).await;
    lakehouse_store::reporting_settings::upsert(&pool, &remembering_settings(), "test")
        .await
        .expect("the settings are saved");
    let owner = member_of(&pool, OWNER_TENANT).await;
    let owner_cookie = session_cookie_for_user(&pool, owner).await;
    let other = member_of(&pool, OTHER_TENANT).await;
    let other_cookie = session_cookie_for_user(&pool, other).await;

    // The owner reads the board first: the tile is remembered.
    let (status, body) = get_dashboard(&router, &owner_cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["results"][TILE]["rows"].as_array().map(Vec::len),
        Some(1),
        "{body}"
    );
    assert!(
        !fake.tile_statements().is_empty(),
        "the owner's tile reached the engine"
    );

    // The owner's own second read is served from memory: the tile really is
    // remembered, so the next read could have been served from it.
    let (status, body) = get_dashboard(&router, &owner_cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["results"][TILE]["remembered"],
        json!(true),
        "the tile is remembered: {body}"
    );
    let asked_before = fake.tile_statements().len();

    // The other tenant's member reads the same board.
    let (status, body) = get_dashboard(&router, &other_cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["results"][TILE]["error"], REFUSAL,
        "a remembered tile is not served to another tenant: {body}"
    );
    assert!(
        body["results"][TILE].get("rows").is_none(),
        "the refused tile has no rows: {body}"
    );
    assert_eq!(
        fake.tile_statements().len(),
        asked_before,
        "the refusal precedes the cache lookup and the engine: nothing more may be asked"
    );
}

// ── the other signed-in sites (SEC-50 slice 2 T3) ───────────────────────────

/// The three callers of the matrix: the tenant that owns the shared catalog,
/// a member of another tenant, and an unrestricted principal.
struct Callers {
    owner: String,
    other: String,
    admin: String,
}

/// The three callers, each holding `permissions` and each in its own tenant.
async fn callers(pool: &PgPool, permissions: &str) -> Callers {
    let owner = member_with(pool, OWNER_TENANT, permissions).await;
    let other = member_with(pool, OTHER_TENANT, permissions).await;
    let admin = create_principal_with_permissions(pool, "*:*").await;
    Callers {
        owner: session_cookie_for_user(pool, owner).await,
        other: session_cookie_for_user(pool, other).await,
        admin: session_cookie_for_user(pool, admin).await,
    }
}

/// One request, as the console sends it.
async fn call(
    router: &axum::Router,
    cookie: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie);
    let body = match body {
        Some(value) => {
            request = request.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let resp = router
        .clone()
        .oneshot(request.body(body).expect("build request"))
        .await
        .expect("router never fails a request outright");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
    let parsed = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, parsed)
}

/// The matrix of `SEC-50` slice 2 T3: the tenant that owns the shared
/// catalog is served, an unrestricted principal is served, and a member of
/// another tenant is refused with `SEC-20`'s sentence — and the refusal
/// reaches neither the engine nor the result cache.
///
/// `served` says what a served answer looks like; the refusal is the same
/// everywhere.
async fn assert_matrix<F>(
    router: &axum::Router,
    fake: &FakeCh,
    callers: &Callers,
    method: &str,
    uri: &str,
    body: Option<Value>,
    served: F,
) where
    F: Fn(StatusCode, &Value) -> bool,
{
    // The catalog's own tenant reads it.
    let (status, answer) = call(router, &callers.owner, method, uri, body.clone()).await;
    assert!(
        served(status, &answer),
        "the owning tenant: {status} {answer}"
    );

    // So does an unrestricted principal.
    let (status, answer) = call(router, &callers.admin, method, uri, body.clone()).await;
    assert!(
        served(status, &answer),
        "the unrestricted: {status} {answer}"
    );

    // How many statements the engine was asked for, so far.
    let asked = fake.tile_statements().len();

    // A member of another tenant is refused, and nothing more is asked.
    let (status, answer) = call(router, &callers.other, method, uri, body).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{answer}");
    assert_eq!(answer["error"], REFUSAL, "{answer}");
    assert_eq!(
        fake.tile_statements().len(),
        asked,
        "the refusal precedes the engine: nothing more may be asked"
    );
}

/// `SEC-50` slice 2 T3: the drill-down reads as the caller's own tenant.
#[tokio::test]
async fn the_records_of_another_tenant_are_refused() {
    let fake = FakeCh::new();
    let server = fake.serve().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&server.uri()).await;
    let callers = callers(&pool, "dashboard:read").await;

    assert_matrix(
        &router,
        &fake,
        &callers,
        "GET",
        "/api/dashboard/records?mart=mart_x&column=kab&value=a",
        None,
        |status, answer| status == StatusCode::OK && answer["rows"].is_array(),
    )
    .await;
}

/// `SEC-50` slice 2 T3/M9: the filter values (no board) read as the caller's
/// own tenant.
#[tokio::test]
async fn the_filter_values_of_another_tenant_are_refused() {
    let fake = FakeCh::new();
    let server = fake.serve().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&server.uri()).await;
    let callers = callers(&pool, "dashboard:read").await;

    assert_matrix(
        &router,
        &fake,
        &callers,
        "GET",
        "/api/dashboard/values?column=kab",
        None,
        |status, answer| status == StatusCode::OK && answer["values"].is_array(),
    )
    .await;
}

/// `SEC-50` slice 2 T3/M9: the filter values of a board read as the caller's
/// own tenant.
#[tokio::test]
async fn the_filter_values_of_a_board_of_another_tenant_are_refused() {
    let fake = FakeCh::new();
    let server = fake.serve().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&server.uri()).await;
    let callers = callers(&pool, "dashboard:read").await;

    assert_matrix(
        &router,
        &fake,
        &callers,
        "GET",
        "/api/dashboard/values?column=kab&board=default",
        None,
        |status, answer| status == StatusCode::OK && answer["values"].is_array(),
    )
    .await;
}

/// `SEC-50` slice 2 T3: the chart download reads as the caller's own tenant.
#[tokio::test]
async fn the_chart_download_of_another_tenant_is_refused() {
    let fake = FakeCh::new();
    let server = fake.serve().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&server.uri()).await;
    let callers = callers(&pool, "dashboard:read").await;

    for sql in fake.statements() {
        eprintln!("STATEMENT: {sql}");
    }
    assert_matrix(
        &router,
        &fake,
        &callers,
        "GET",
        "/api/dashboard/chart-export?chart=c_a&format=csv",
        None,
        |status, _| status == StatusCode::OK,
    )
    .await;
}

/// `SEC-50` slice 2 T3: the table download reads as the caller's own tenant.
#[tokio::test]
async fn the_table_download_of_another_tenant_is_refused() {
    let fake = FakeCh::with_charts(vec![table_chart_row()]);
    let server = fake.serve().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&server.uri()).await;
    let callers = callers(&pool, "dashboard:read").await;

    assert_matrix(
        &router,
        &fake,
        &callers,
        "GET",
        "/api/dashboard/table-export?chart=c_table",
        None,
        |status, _| status == StatusCode::OK,
    )
    .await;
}

/// `SEC-50` slice 2 T3: a SQL source's preview reads as the caller's own
/// tenant.
#[tokio::test]
async fn the_sql_source_preview_of_another_tenant_is_refused() {
    let fake = FakeCh::new();
    let server = fake.serve().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&server.uri()).await;
    let callers = callers(&pool, "dashboard:read,dashboard:sql").await;

    assert_matrix(
        &router,
        &fake,
        &callers,
        "POST",
        "/api/dashboard/sources/preview",
        Some(json!({ "sql": "SELECT kab, visitors FROM serving.mart_x" })),
        |status, answer| status == StatusCode::OK && answer["columns"].is_array(),
    )
    .await;
}

/// `SEC-50` slice 2 T3: a SQL source is saved as the caller's own tenant, so
/// a source over a table another tenant owns is refused.
#[tokio::test]
async fn the_sql_source_of_another_tenant_is_refused() {
    let fake = FakeCh::new();
    let server = fake.serve().await;
    let TestApp { router, pool } = spin_up_with_clickhouse(&server.uri()).await;
    let callers = callers(&pool, "dashboard:read,dashboard:sql").await;

    assert_matrix(
        &router,
        &fake,
        &callers,
        "POST",
        "/api/dashboard/sources",
        Some(json!({
            "title": "A source",
            "sql": "SELECT kab, visitors FROM serving.mart_x",
        })),
        |status, answer| status == StatusCode::OK && answer["source"].is_object(),
    )
    .await;
}
