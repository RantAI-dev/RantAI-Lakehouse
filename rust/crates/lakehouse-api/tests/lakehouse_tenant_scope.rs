//! The Iceberg catalog routes and the assistant's Iceberg tools respect the
//! tenant gate that Query Studio applies (`SEC-20`).
//!
//! An upload lands in `bronze.<raw name>` and the tenant that made it holds
//! the claim (`upload_table_claim`). Query Studio refuses another tenant's
//! upload table with a 404 and one fixed sentence. These tests hold the
//! catalog routes (`GET /api/lakehouse/tables`, `.../tables/{ns}/{table}`,
//! `.../maintenance`, `POST .../maintenance`) and the four tools that call
//! them (`list_iceberg_tables`, `describe_iceberg_table`,
//! `get_table_maintenance`, `set_table_maintenance`) to the same rule, over
//! the two seeded tenants:
//!
//! - Meridian Group owns the shared catalog (`CATALOG_TENANT_ID`) and the
//!   upload table `group_orders`;
//! - Meridian Retail owns the upload table `retail_orders`;
//! - `shared_events` is claimed by nobody.
//!
//! The Iceberg REST catalog is a `wiremock` server that lists the three
//! tables and loads each with a column only an upload table has, so a test
//! can assert that the column never reached a caller who may not read the
//! table.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::uploads::claim_table;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{
    CATALOG_TENANT, TestApp, create_principal_with_permissions, load_table_fixture,
    session_cookie_for_seeded_user, session_cookie_for_user, spin_up_with_env,
};

/// The tenant of Meridian Retail: not the catalog's owner.
const OTHER_TENANT: &str = "11111111-1111-4111-8111-000000000002";

/// The upload table the catalog's own tenant holds.
const GROUP_TABLE: &str = "group_orders";
/// The upload table the other tenant holds.
const RETAIL_TABLE: &str = "retail_orders";
/// A table nobody claimed: a shared one.
const SHARED_TABLE: &str = "shared_events";

/// The column the mock catalog gives every table, and a refusal must not
/// carry.
const MARKER_COLUMN: &str = "upload_only_column";

/// The refusal, byte for byte: Query Studio's sentence, with the table as
/// the gate canonicalises it.
const REFUSAL: &str = "Table not found or not available to your organisation: bronze.group_orders";

/// What the chat needs to run a tool, plus the permission the tool itself
/// asks for.
const CHAT_AND_OPS_READ: &str = "assistant:use, assistant:ask, ops:read";
const CHAT_AND_GOVERNANCE_WRITE: &str = "assistant:use, assistant:ask, governance:write";

/// The reader token file the catalog client needs, kept for the test's life.
fn token_file(name: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(name);
    std::fs::write(&file, "test-token").unwrap();
    std::mem::forget(dir);
    file.to_str().unwrap().to_owned()
}

/// The Iceberg REST catalog: `bronze` lists the three tables and loads each
/// one with [`MARKER_COLUMN`] as its only column.
async fn mount_catalog(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/config"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"defaults":{},"overrides":{}}"#),
        )
        .mount(server)
        .await;
    let identifiers: Vec<Value> = [GROUP_TABLE, RETAIL_TABLE, SHARED_TABLE]
        .iter()
        .map(|name| json!({ "namespace": ["bronze"], "name": name }))
        .collect();
    Mock::given(method("GET"))
        .and(path("/v1/namespaces/bronze/tables"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "identifiers": identifiers })),
        )
        .mount(server)
        .await;
    for table in [GROUP_TABLE, RETAIL_TABLE, SHARED_TABLE] {
        let body = load_table_fixture(table)
            .replace(r#""name": "id""#, &format!(r#""name": "{MARKER_COLUMN}""#));
        Mock::given(method("GET"))
            .and(path(format!("/v1/namespaces/bronze/tables/{table}")))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(server)
            .await;
    }
}

/// `ClickHouse` with no maintenance history: every query answers no rows.
async fn mount_empty_clickhouse(server: &MockServer) {
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
        .mount(server)
        .await;
}

/// The deployment: Meridian Group owns the shared catalog, the catalog is
/// the mock above, and `ClickHouse` has no history.
async fn spin_up_with_catalog(catalog: &MockServer, ch: &MockServer) -> TestApp {
    let mut env = HashMap::new();
    env.insert("CH_URL".to_owned(), ch.uri());
    env.insert("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT.to_owned());
    env.insert("LAKEKEEPER_CATALOG_URI".to_owned(), catalog.uri());
    env.insert("LAKEKEEPER_WAREHOUSE".to_owned(), "default".to_owned());
    env.insert(
        "LAKEKEEPER_READ_TOKEN_FILE".to_owned(),
        token_file("read.jwt"),
    );
    spin_up_with_env(&env).await
}

/// Both tenants claim their upload table, the way an upload does.
async fn claim_the_uploads(pool: &PgPool) {
    for (tenant, table) in [(CATALOG_TENANT, GROUP_TABLE), (OTHER_TENANT, RETAIL_TABLE)] {
        let claimed = claim_table(
            pool,
            tenant.parse::<Uuid>().expect("a seeded tenant id"),
            table,
            &format!("upload-{table}"),
        )
        .await
        .expect("claim the upload table");
        assert!(claimed, "the tenant must hold the claim on {table}");
    }
}

/// A principal that holds exactly `permissions` and belongs to exactly
/// `tenant`, as a session cookie.
async fn cookie_in_tenant(pool: &PgPool, permissions: &str, tenant: &str) -> String {
    let user = create_principal_with_permissions(pool, permissions).await;
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(user)
        .bind(tenant.parse::<Uuid>().expect("a seeded tenant id"))
        .execute(pool)
        .await
        .expect("add the membership");
    session_cookie_for_user(pool, user).await
}

/// A principal that holds every permission and belongs to no tenant: an
/// operator, whose reach is everything.
async fn operator_cookie(pool: &PgPool) -> String {
    let user = create_principal_with_permissions(pool, "*:*").await;
    session_cookie_for_user(pool, user).await
}

async fn send(
    router: &axum::Router,
    method: &str,
    uri: &str,
    cookie: &str,
    tenant: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie);
    if let Some(tenant) = tenant {
        builder = builder.header("x-tenant", tenant);
    }
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let resp = router
        .clone()
        .oneshot(builder.body(body).expect("build request"))
        .await
        .expect("router never fails a request outright");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, body)
}

async fn get(
    router: &axum::Router,
    uri: &str,
    cookie: &str,
    tenant: Option<&str>,
) -> (StatusCode, Value) {
    send(router, "GET", uri, cookie, tenant, None).await
}

fn policy_body() -> Value {
    json!({
        "snapshotsToKeep": 10,
        "orphanAgeHours": 48,
        "compactSmallFiles": true,
        "schedule": "daily",
    })
}

/// One of the chat's tools, called the way the chat calls it.
async fn call_tool(router: &axum::Router, cookie: &str, tool: &str, args: Value) -> Value {
    let (status, body) = send(
        router,
        "POST",
        "/api/ai/tool",
        cookie,
        None,
        Some(json!({ "tool": tool, "args": args, "mode": "build" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// The names in a `tables` body, in order.
fn table_names(body: &Value) -> Vec<String> {
    body["tables"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of tables: {body}"))
        .iter()
        .map(|t| t["name"].as_str().expect("a table name").to_owned())
        .collect()
}

/// How many times the catalog was asked to load a table.
async fn table_loads(catalog: &MockServer) -> usize {
    catalog
        .received_requests()
        .await
        .expect("mock records requests")
        .iter()
        .filter(|r| r.url.path().starts_with("/v1/namespaces/bronze/tables/"))
        .count()
}

/// How many maintenance policies the store holds for `table`.
async fn stored_policies(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM table_maintenance_policy WHERE table_name = $1")
        .bind(table)
        .fetch_one(pool)
        .await
        .expect("count the policies")
}

// (a) A table's page.

#[tokio::test]
async fn the_page_of_another_tenants_upload_table_is_refused_with_query_studios_sentence() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let (status, body) = get(
        &router,
        "/api/lakehouse/tables/bronze/group_orders",
        &cookie,
        None,
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], REFUSAL, "{body}");
    assert!(
        !body.to_string().contains(MARKER_COLUMN),
        "no column may reach the caller: {body}"
    );
    assert_eq!(
        table_loads(&catalog).await,
        0,
        "the refusal precedes the catalog: nothing may be asked"
    );
}

#[tokio::test]
async fn the_owning_tenant_reads_the_page_of_its_upload_table() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let (status, body) = get(
        &router,
        "/api/lakehouse/tables/bronze/group_orders",
        &cookie,
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["schema"][0]["name"], MARKER_COLUMN, "{body}");
}

#[tokio::test]
async fn the_page_of_a_table_follows_the_tenant_the_x_tenant_header_selects() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    // Dewi belongs to Meridian Group, Retail and Logistics.
    let cookie = session_cookie_for_seeded_user(&pool, "dewi@meridian.example").await;
    let uri = "/api/lakehouse/tables/bronze/group_orders";

    let (as_owner, body) = get(&router, uri, &cookie, Some(CATALOG_TENANT)).await;
    assert_eq!(as_owner, StatusCode::OK, "{body}");

    let (as_other, body) = get(&router, uri, &cookie, Some(OTHER_TENANT)).await;
    assert_eq!(as_other, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], REFUSAL, "{body}");
}

// (b) A table's maintenance.

#[tokio::test]
async fn the_maintenance_of_another_tenants_upload_table_is_refused() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let other = cookie_in_tenant(&pool, "catalog:read", OTHER_TENANT).await;
    let owner = cookie_in_tenant(&pool, "catalog:read", CATALOG_TENANT).await;
    let uri = "/api/lakehouse/tables/bronze/group_orders/maintenance";

    let (status, body) = get(&router, uri, &other, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], REFUSAL, "{body}");
    assert_eq!(
        ch.received_requests().await.unwrap().len(),
        0,
        "the refusal precedes the history lookup: nothing may be asked"
    );

    let (status, body) = get(&router, uri, &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["configured"], json!(false), "{body}");
}

#[tokio::test]
async fn a_policy_for_another_tenants_upload_table_is_refused_and_not_stored() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let other = cookie_in_tenant(&pool, "catalog:read, governance:write", OTHER_TENANT).await;
    let owner = cookie_in_tenant(&pool, "catalog:read, governance:write", CATALOG_TENANT).await;
    let uri = "/api/lakehouse/tables/bronze/group_orders/maintenance";

    let (status, body) = send(&router, "POST", uri, &other, None, Some(policy_body())).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], REFUSAL, "{body}");
    assert_eq!(
        stored_policies(&pool, GROUP_TABLE).await,
        0,
        "a refused write stores nothing"
    );

    let (status, body) = send(&router, "POST", uri, &owner, None, Some(policy_body())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(stored_policies(&pool, GROUP_TABLE).await, 1);
}

// (c) The listing.

#[tokio::test]
async fn a_listing_leaves_out_the_tables_the_caller_may_not_read() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let uri = "/api/lakehouse/tables?namespace=bronze";

    // The catalog's own tenant: its own upload and the unclaimed shared
    // table, never the other tenant's upload.
    let owner = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let (status, body) = get(&router, uri, &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        table_names(&body),
        vec![GROUP_TABLE, SHARED_TABLE],
        "{body}"
    );

    // Another tenant: its own upload only. An unclaimed table is a shared
    // one, and only the catalog's tenant reads it, as in Query Studio.
    let other = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;
    let (status, body) = get(&router, uri, &other, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(table_names(&body), vec![RETAIL_TABLE], "{body}");
    assert!(
        !body.to_string().contains(GROUP_TABLE),
        "the other tenant's table is not named anywhere: {body}"
    );

    // A caller in no tenant reads no table of a deployment with several.
    let nobody = create_principal_with_permissions(&pool, "catalog:read").await;
    let nobody = session_cookie_for_user(&pool, nobody).await;
    let (status, body) = get(&router, uri, &nobody, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(table_names(&body).is_empty(), "{body}");

    // An operator reaches everything.
    let operator = operator_cookie(&pool).await;
    let (status, body) = get(&router, uri, &operator, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        table_names(&body),
        vec![GROUP_TABLE, RETAIL_TABLE, SHARED_TABLE],
        "{body}"
    );
}

// (d) The assistant's tools.

#[tokio::test]
async fn describing_another_tenants_upload_table_in_the_chat_gives_the_refusal_and_no_column() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let other = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;
    let owner = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let args = json!({ "namespace": "bronze", "table": GROUP_TABLE });

    let described = call_tool(&router, &other, "describe_iceberg_table", args.clone()).await;
    assert_eq!(described["result"]["error"], json!(REFUSAL), "{described}");
    assert!(
        !described.to_string().contains(MARKER_COLUMN),
        "no column may reach the chat: {described}"
    );
    assert_eq!(table_loads(&catalog).await, 0);

    let described = call_tool(&router, &owner, "describe_iceberg_table", args).await;
    assert_eq!(
        described["result"]["schema"][0]["name"],
        json!(MARKER_COLUMN),
        "{described}"
    );
}

#[tokio::test]
async fn listing_iceberg_tables_in_the_chat_leaves_out_another_tenants_upload_table() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let other = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;
    let args = json!({ "namespace": "bronze" });

    let listed = call_tool(&router, &other, "list_iceberg_tables", args).await;

    assert_eq!(
        table_names(&listed["result"]),
        vec![RETAIL_TABLE],
        "{listed}"
    );
    assert!(
        !listed.to_string().contains(GROUP_TABLE),
        "the other tenant's table is not named anywhere: {listed}"
    );
}

#[tokio::test]
async fn reading_the_maintenance_of_another_tenants_upload_table_in_the_chat_is_refused() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let other = cookie_in_tenant(&pool, CHAT_AND_OPS_READ, OTHER_TENANT).await;
    let owner = cookie_in_tenant(&pool, CHAT_AND_OPS_READ, CATALOG_TENANT).await;
    let args = json!({ "namespace": "bronze", "table": GROUP_TABLE });

    let read = call_tool(&router, &other, "get_table_maintenance", args.clone()).await;
    assert_eq!(read["result"]["error"], json!(REFUSAL), "{read}");

    let read = call_tool(&router, &owner, "get_table_maintenance", args).await;
    assert_eq!(read["result"]["configured"], json!(false), "{read}");
}

#[tokio::test]
async fn setting_the_maintenance_of_another_tenants_upload_table_in_the_chat_is_refused() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let other = cookie_in_tenant(&pool, CHAT_AND_GOVERNANCE_WRITE, OTHER_TENANT).await;
    let owner = cookie_in_tenant(&pool, CHAT_AND_GOVERNANCE_WRITE, CATALOG_TENANT).await;
    let args = json!({
        "namespace": "bronze",
        "table": GROUP_TABLE,
        "snapshotsToKeep": 10,
        "confirmed": true,
    });

    let set = call_tool(&router, &other, "set_table_maintenance", args.clone()).await;
    assert_eq!(set["result"]["error"], json!(REFUSAL), "{set}");
    assert_eq!(stored_policies(&pool, GROUP_TABLE).await, 0);

    let set = call_tool(&router, &owner, "set_table_maintenance", args).await;
    assert_eq!(set["result"]["configured"], json!(true), "{set}");
    assert_eq!(stored_policies(&pool, GROUP_TABLE).await, 1);
}
