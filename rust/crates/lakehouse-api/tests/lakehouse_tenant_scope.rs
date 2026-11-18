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
//!
//! The last section holds the readers that are not about one table: the
//! namespace counts, and the assistant's dataset, lineage and maintenance
//! tools. They answer from the shared catalog, so a tenant that does not own
//! it is refused with the catalog's own rule, and a tenant that does own it
//! still does not see another tenant's claimed upload table.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::column_lineage::{NewMapping, replace_producer};
use lakehouse_store::uploads::claim_table;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{body_string_contains, method, path};
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

/// The dataset the shared catalog registers: nothing a tenant that does not
/// own the catalog may learn.
const DATASET_SLUG: &str = "group-sales-ledger";
const DATASET_TITLE: &str = "Group Sales Ledger";
const DATASET_TABLE: &str = "group_sales_ledger";
/// A dashboard chart over a Gold mart, and the mart it reads.
const CHART_ID: &str = "chart_group_visits";
const CHART_TITLE: &str = "Group Visitors Chart";
const CHART_MART: &str = "mart_group_visits";

/// Serves `data` to every `ClickHouse` query whose text holds `needle`.
async fn clickhouse_rows(server: &MockServer, needle: &str, data: Vec<Value>) {
    Mock::given(method("POST"))
        .and(body_string_contains(needle))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "meta": [], "data": data, "rows": data.len() })),
        )
        .with_priority(1)
        .mount(server)
        .await;
}

/// `ClickHouse` as a deployment with data holds it: one registered dataset,
/// one stored chart, and a maintenance run for each of the three Bronze
/// tables. Every other query answers no rows.
async fn mount_clickhouse_with_data(server: &MockServer) {
    mount_empty_clickhouse(server).await;
    clickhouse_rows(
        server,
        "bronze_meta.dataset_catalog",
        vec![json!({
            "slug": DATASET_SLUG,
            "title": DATASET_TITLE,
            "description": "every sale the group booked",
            "tier": "primer",
            "table_name": DATASET_TABLE,
        })],
    )
    .await;
    clickhouse_rows(
        server,
        "FROM console.bi_chart",
        vec![json!({
            "id": CHART_ID,
            "spec_json": json!({
                "spec": { "id": CHART_ID, "title": CHART_TITLE, "kind": "bar", "mart": CHART_MART,
                          "sql": "SELECT 1", "x": "province", "y": "visitors" },
                "def": { "title": CHART_TITLE, "mart": CHART_MART, "kind": "bar",
                         "dimension": "province", "measures": ["visitors"], "board": "b_1" },
                "hasYear": false,
            })
            .to_string(),
            "board": "b_1",
            "created_by": "ui",
            "created_at": "2026-10-01 10:00:00",
        })],
    )
    .await;
    let runs: Vec<Value> = [GROUP_TABLE, RETAIL_TABLE, SHARED_TABLE]
        .iter()
        .map(|table| {
            json!({
                "table_name": format!("bronze.{table}"),
                "run_at": "2026-11-14 03:00:00",
                "dry_data": "1",
                "dry_manifests": "1",
                "applied_data": "2",
                "applied_manifests": "2",
                "skipped_verbs": "",
            })
        })
        .collect();
    clickhouse_rows(server, "bronze_meta.maintenance_run", runs).await;
}

/// The catalog's namespace list: only `bronze`, which holds the three tables.
async fn mount_namespaces(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/namespaces"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "namespaces": [["bronze"]] })),
        )
        .mount(server)
        .await;
}

/// A pipeline of the owning tenant that names both uploads as inputs: the
/// table the other tenant holds is not the owner's to draw. It builds
/// `silver.group_mixed`.
async fn add_the_mixed_pipeline(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO pipeline_definition \
            (id, name, kind, status, source, target, schedule, tenant_id, input_tables) \
         VALUES ('pl-mixed', 'Mixed', 'batch', 'draft', 'bronze.group_orders', \
                 'silver.group_mixed', '@hourly', $1, \
                 ARRAY['bronze.group_orders', 'bronze.retail_orders'])",
    )
    .bind(CATALOG_TENANT.parse::<Uuid>().expect("a seeded tenant id"))
    .execute(pool)
    .await
    .expect("add the pipeline");
}

/// The recorded column mappings of that pipeline: one from each upload.
async fn record_the_mixed_pipelines_column_mappings(pool: &PgPool) {
    let map = |from: &str| NewMapping {
        from_table: Some(from.to_owned()),
        from_column: Some("order_total".to_owned()),
        to_table: "silver.group_mixed".to_owned(),
        to_column: "total".to_owned(),
        transform: "copy".to_owned(),
        partial: false,
        reason: None,
    };
    replace_producer(
        pool,
        "pipeline",
        "pl-mixed",
        Some(CATALOG_TENANT.parse::<Uuid>().expect("a seeded tenant id")),
        &[map("bronze.group_orders"), map("bronze.retail_orders")],
    )
    .await
    .expect("record the mappings");
}

/// The names of the maintenance rows a tool answered, in order.
fn maintenance_tables(result: &Value) -> Vec<String> {
    result["maintenance"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of maintenance rows: {result}"))
        .iter()
        .map(|row| row["tableName"].as_str().expect("a table name").to_owned())
        .collect()
}

/// The ids of the nodes of a lineage graph, in order.
fn node_ids(graph: &Value) -> Vec<String> {
    graph["nodes"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of lineage nodes: {graph}"))
        .iter()
        .map(|n| n["id"].as_str().expect("a node id").to_owned())
        .collect()
}

/// The `tableCount` of the namespace `name` in a `namespaces` body.
fn table_count(body: &Value, name: &str) -> Value {
    body["namespaces"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of namespaces: {body}"))
        .iter()
        .find(|ns| ns["name"] == json!(name))
        .unwrap_or_else(|| panic!("namespace {name} in {body}"))["tableCount"]
        .clone()
}

/// The reason the shared catalog's own rule gives a tenant that does not own
/// it, read from `GET /api/catalog` (the route the rule was written for) so
/// the tools' sentence is compared with that route's, not retyped here. The
/// caller is a member of the other tenant, which does not own the catalog.
async fn the_rules_reason(router: &axum::Router, pool: &PgPool) -> String {
    let cookie = cookie_in_tenant(pool, "catalog:read", OTHER_TENANT).await;
    let (status, body) = get(router, "/api/catalog", &cookie, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["supported"], json!(false), "{body}");
    body["reason"].as_str().expect("a reason").to_owned()
}

/// What a tool says to a tenant that does not own the shared catalog.
fn assert_refused_like_the_catalog(result: &Value, reason: &str) {
    assert_eq!(result["supported"], json!(false), "{result}");
    assert_eq!(result["reason"], json!(reason), "{result}");
}

/// Nothing the shared catalog holds may appear in `answer`.
fn assert_none_of_the_groups_data(answer: &Value) {
    let text = answer.to_string();
    for secret in [
        DATASET_SLUG,
        DATASET_TITLE,
        DATASET_TABLE,
        CHART_ID,
        CHART_TITLE,
        CHART_MART,
        GROUP_TABLE,
        SHARED_TABLE,
    ] {
        assert!(!text.contains(secret), "{secret} must not appear: {answer}");
    }
}

const CATALOG_READER: &str = "assistant:use, assistant:ask, catalog:read";
const LINEAGE_READER: &str = "assistant:use, assistant:ask, lineage:read, dashboard:read";

/// The deployment: Meridian Group owns the shared catalog, the catalog is
/// the mock above, and `ClickHouse` has no history.
async fn spin_up_with_catalog(catalog: &MockServer, ch: &MockServer) -> TestApp {
    spin_up_with_catalog_and_llm(catalog, ch, None).await
}

/// [`spin_up_with_catalog`] with the model at `llm` (`None` for no model).
async fn spin_up_with_catalog_and_llm(
    catalog: &MockServer,
    ch: &MockServer,
    llm: Option<&MockServer>,
) -> TestApp {
    let mut env = HashMap::new();
    if let Some(llm) = llm {
        env.insert("LLM_URL".to_owned(), llm.uri());
    }
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

// (e) The readers that answer from the shared catalog.

#[tokio::test]
async fn listing_datasets_is_refused_to_a_tenant_that_does_not_own_the_shared_catalog() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_clickhouse_with_data(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let other = cookie_in_tenant(&pool, CATALOG_READER, OTHER_TENANT).await;
    let reason = the_rules_reason(&router, &pool).await;

    let listed = call_tool(&router, &other, "list_datasets", json!({})).await;

    assert_refused_like_the_catalog(&listed["result"], &reason);
    assert_none_of_the_groups_data(&listed);
}

#[tokio::test]
async fn describing_a_dataset_is_refused_to_a_tenant_that_does_not_own_the_shared_catalog() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_clickhouse_with_data(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let other = cookie_in_tenant(&pool, CATALOG_READER, OTHER_TENANT).await;
    let reason = the_rules_reason(&router, &pool).await;

    let described = call_tool(
        &router,
        &other,
        "describe_dataset",
        json!({ "slug": DATASET_SLUG }),
    )
    .await;

    assert_refused_like_the_catalog(&described["result"], &reason);
    assert_none_of_the_groups_data(&described);
}

#[tokio::test]
async fn the_owning_tenant_still_lists_and_describes_its_datasets() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_clickhouse_with_data(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, CATALOG_READER, CATALOG_TENANT).await;

    let listed = call_tool(&router, &owner, "list_datasets", json!({})).await;
    assert_eq!(listed["result"]["total"], json!(1), "{listed}");
    assert_eq!(
        listed["result"]["datasets"][0]["slug"],
        json!(DATASET_SLUG),
        "{listed}"
    );

    let described = call_tool(
        &router,
        &owner,
        "describe_dataset",
        json!({ "slug": DATASET_SLUG }),
    )
    .await;
    assert_eq!(
        described["result"]["title"],
        json!(DATASET_TITLE),
        "{described}"
    );
}

// `SEC-20`, plan 97: the shared registry also holds a row for an upload
// another tenant made (its `bronze.<table_name>` is that tenant's claim). The
// tenant that owns the catalog passes the shared-catalog rule and still must
// not read that row's slug, title, description or columns.

/// The registry row of the upload the other tenant holds.
const FOREIGN_SLUG: &str = "retail-orders-sheet";
const FOREIGN_TITLE: &str = "Retail Orders Sheet";
const FOREIGN_DESCRIPTION: &str = "orders only the retail tenant uploaded";

fn registry_row(slug: &str, title: &str, description: &str, table: &str) -> Value {
    json!({
        "slug": slug,
        "title": title,
        "description": description,
        "tier": "primer",
        "table_name": table,
    })
}

/// The group's own registry row and the row of the other tenant's upload.
fn both_registry_rows() -> Vec<Value> {
    vec![
        registry_row(
            DATASET_SLUG,
            DATASET_TITLE,
            "every sale the group booked",
            DATASET_TABLE,
        ),
        registry_row(
            FOREIGN_SLUG,
            FOREIGN_TITLE,
            FOREIGN_DESCRIPTION,
            RETAIL_TABLE,
        ),
    ]
}

/// `ClickHouse` whose registry holds `rows` and nothing else.
async fn mount_registry(server: &MockServer, rows: Vec<Value>) {
    mount_empty_clickhouse(server).await;
    clickhouse_rows(server, "bronze_meta.dataset_catalog", rows).await;
}

/// Nothing of the other tenant's registry row may appear in `answer`.
fn assert_none_of_the_foreign_dataset(answer: &Value) {
    let text = answer.to_string();
    for secret in [
        FOREIGN_SLUG,
        FOREIGN_TITLE,
        FOREIGN_DESCRIPTION,
        RETAIL_TABLE,
    ] {
        assert!(!text.contains(secret), "{secret} must not appear: {answer}");
    }
}

#[tokio::test]
async fn listing_datasets_leaves_out_another_tenants_upload_for_the_owning_tenant() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_registry(&ch, both_registry_rows()).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, CATALOG_READER, CATALOG_TENANT).await;
    let operator = operator_cookie(&pool).await;

    let listed = call_tool(&router, &owner, "list_datasets", json!({})).await;
    assert_eq!(listed["result"]["total"], json!(1), "{listed}");
    assert_eq!(
        listed["result"]["datasets"][0]["slug"],
        json!(DATASET_SLUG),
        "{listed}"
    );
    assert_none_of_the_foreign_dataset(&listed);

    // An operator's reach is everything: nothing is left out.
    let listed = call_tool(&router, &operator, "list_datasets", json!({})).await;
    assert_eq!(listed["result"]["total"], json!(2), "{listed}");
}

#[tokio::test]
async fn describing_another_tenants_upload_is_refused_for_the_owning_tenant() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_registry(
        &ch,
        vec![registry_row(
            FOREIGN_SLUG,
            FOREIGN_TITLE,
            FOREIGN_DESCRIPTION,
            RETAIL_TABLE,
        )],
    )
    .await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, CATALOG_READER, CATALOG_TENANT).await;
    let operator = operator_cookie(&pool).await;
    let args = json!({ "slug": FOREIGN_SLUG });

    let described = call_tool(&router, &owner, "describe_dataset", args.clone()).await;
    // The sentence names the slug the caller asked for, never the registry's
    // own table name, which is the other tenant's upload id.
    assert_eq!(
        described["result"],
        json!({ "error": format!(
            "Table not found or not available to your organisation: {FOREIGN_SLUG}"
        ) }),
        "{described}"
    );
    assert!(
        !described.to_string().contains(RETAIL_TABLE),
        "the other tenant's table is named nowhere: {described}"
    );

    // An operator's reach is everything.
    let described = call_tool(&router, &operator, "describe_dataset", args).await;
    assert_eq!(
        described["result"]["title"],
        json!(FOREIGN_TITLE),
        "{described}"
    );
}

#[tokio::test]
async fn the_lakehouse_overview_leaves_out_another_tenants_upload_for_the_owning_tenant() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_registry(&ch, both_registry_rows()).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, CATALOG_READER, CATALOG_TENANT).await;
    let operator = operator_cookie(&pool).await;

    let overview = call_tool(&router, &owner, "lakehouse_overview", json!({})).await;
    let slugs: Vec<&str> = overview["result"]["bronze"]["datasets"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of datasets: {overview}"))
        .iter()
        .filter_map(|d| d["slug"].as_str())
        .collect();
    assert_eq!(slugs, vec![DATASET_SLUG], "{overview}");
    assert_none_of_the_foreign_dataset(&overview);

    let overview = call_tool(&router, &operator, "lakehouse_overview", json!({})).await;
    assert_eq!(
        overview["result"]["bronze"]["datasets"]
            .as_array()
            .map(Vec::len),
        Some(2),
        "{overview}"
    );
}

/// The system prompt the model receives for `permissions` in `tenant`, with
/// `rows` in the registry and one served Gold table, so the map has a table
/// and writes its closing line about registry datasets with no Gold table.
async fn the_prompt_for(rows: Vec<Value>, permissions: &str, tenant: &str) -> String {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_registry(&ch, rows).await;
    clickhouse_rows(
        &ch,
        "FROM system.tables",
        vec![json!({ "database": "serving", "name": "mart_ok", "total_rows": "5" })],
    )
    .await;
    let llm = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "role": "assistant", "content": "Hi." } }]
        })))
        .mount(&llm)
        .await;
    let TestApp { router, pool } = spin_up_with_catalog_and_llm(&catalog, &ch, Some(&llm)).await;
    claim_the_uploads(&pool).await;
    let cookie = cookie_in_tenant(&pool, permissions, tenant).await;
    let (status, body) = send(
        &router,
        "POST",
        "/api/ai/chat",
        &cookie,
        None,
        Some(json!({ "messages": [{ "role": "user", "content": "berapa tamu bulan ini?" }] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let requests = llm.received_requests().await.expect("recording is on");
    let first: Value = serde_json::from_slice(&requests[0].body).expect("a JSON request");
    first["messages"][0]["content"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// `assistant:use` and `catalog:read` open the map without `query:read`, so it
/// is built for this request alone and never shared through the cache.
const MAP_READER: &str = "assistant:use, catalog:read";

#[tokio::test]
async fn the_data_map_leaves_out_another_tenants_upload_for_the_owning_tenant() {
    let prompt = the_prompt_for(both_registry_rows(), MAP_READER, CATALOG_TENANT).await;

    assert!(
        prompt.contains("Catalog datasets with no Gold table: group-sales-ledger ("),
        "the owner's own unserved dataset stays: {prompt}"
    );
    for secret in [FOREIGN_SLUG, FOREIGN_TITLE, RETAIL_TABLE] {
        assert!(
            !prompt.contains(secret),
            "{secret} must not appear: {prompt}"
        );
    }
}

#[tokio::test]
async fn the_lineage_tool_refuses_a_tenant_that_does_not_own_the_shared_catalog() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_clickhouse_with_data(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let other = cookie_in_tenant(&pool, LINEAGE_READER, OTHER_TENANT).await;
    let reason = the_rules_reason(&router, &pool).await;

    let graph = call_tool(
        &router,
        &other,
        "get_lineage",
        json!({ "focus": format!("serving.{CHART_MART}") }),
    )
    .await;

    assert_refused_like_the_catalog(&graph["result"], &reason);
    assert_none_of_the_groups_data(&graph);
}

#[tokio::test]
async fn the_owning_tenant_still_reads_the_lineage_of_its_chart() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_clickhouse_with_data(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, LINEAGE_READER, CATALOG_TENANT).await;

    let graph = call_tool(
        &router,
        &owner,
        "get_lineage",
        json!({ "focus": format!("serving.{CHART_MART}") }),
    )
    .await;

    let ids = node_ids(&graph["result"]);
    assert!(
        ids.contains(&format!("chart:{CHART_ID}")),
        "the owner's chart is drawn: {ids:?}"
    );
}

#[tokio::test]
async fn the_lineage_tool_leaves_out_another_tenants_upload_table_for_the_owning_tenant() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_clickhouse_with_data(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    add_the_mixed_pipeline(&pool).await;
    let owner = cookie_in_tenant(&pool, LINEAGE_READER, CATALOG_TENANT).await;

    let graph = call_tool(
        &router,
        &owner,
        "get_lineage",
        json!({ "focus": "silver.group_mixed" }),
    )
    .await;

    let ids = node_ids(&graph["result"]);
    assert!(
        ids.contains(&format!("bronze:{GROUP_TABLE}")),
        "the owner's own upload is drawn: {ids:?}"
    );
    assert!(
        !graph.to_string().contains(RETAIL_TABLE),
        "the other tenant's upload is named nowhere: {graph}"
    );
}

#[tokio::test]
async fn the_maintenance_tool_refuses_a_tenant_that_does_not_own_the_shared_catalog() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_clickhouse_with_data(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let other = cookie_in_tenant(&pool, CHAT_AND_OPS_READ, OTHER_TENANT).await;
    let reason = the_rules_reason(&router, &pool).await;

    let metrics = call_tool(&router, &other, "get_maintenance_metrics", json!({})).await;

    assert_refused_like_the_catalog(&metrics["result"], &reason);
    assert_none_of_the_groups_data(&metrics);
    assert!(
        !metrics.to_string().contains(RETAIL_TABLE),
        "a refused caller gets no maintenance row at all: {metrics}"
    );
}

#[tokio::test]
async fn the_maintenance_tool_leaves_out_another_tenants_upload_table_for_the_owning_tenant() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_clickhouse_with_data(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, CHAT_AND_OPS_READ, CATALOG_TENANT).await;
    let operator = operator_cookie(&pool).await;

    // The owning tenant: its own upload and the unclaimed shared table, never
    // the table the other tenant holds.
    let metrics = call_tool(&router, &owner, "get_maintenance_metrics", json!({})).await;
    assert_eq!(
        maintenance_tables(&metrics["result"]),
        vec![
            format!("bronze.{GROUP_TABLE}"),
            format!("bronze.{SHARED_TABLE}")
        ],
        "{metrics}"
    );
    assert!(
        !metrics.to_string().contains(RETAIL_TABLE),
        "the other tenant's table is named nowhere: {metrics}"
    );

    // An operator reaches everything.
    let metrics = call_tool(&router, &operator, "get_maintenance_metrics", json!({})).await;
    assert_eq!(maintenance_tables(&metrics["result"]).len(), 3, "{metrics}");
}

#[tokio::test]
async fn the_namespace_count_holds_only_the_tables_the_caller_may_read() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    mount_namespaces(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let uri = "/api/lakehouse/namespaces";
    let owner = cookie_in_tenant(&pool, "catalog:read", CATALOG_TENANT).await;
    let other = cookie_in_tenant(&pool, "catalog:read", OTHER_TENANT).await;
    let operator = operator_cookie(&pool).await;

    // The owning tenant counts its own upload and the shared table, not the
    // upload the other tenant holds.
    let (status, body) = get(&router, uri, &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(table_count(&body, "bronze"), json!(2), "{body}");

    // Another tenant counts its own upload only.
    let (status, body) = get(&router, uri, &other, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(table_count(&body, "bronze"), json!(1), "{body}");

    // An operator reaches all three.
    let (status, body) = get(&router, uri, &operator, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(table_count(&body, "bronze"), json!(3), "{body}");
}

#[tokio::test]
async fn the_lineage_tool_leaves_out_the_column_mappings_of_another_tenants_upload_table() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_clickhouse_with_data(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    add_the_mixed_pipeline(&pool).await;
    record_the_mixed_pipelines_column_mappings(&pool).await;
    let owner = cookie_in_tenant(&pool, LINEAGE_READER, CATALOG_TENANT).await;

    // Focus on the table the pipeline builds: the mapping from the owner's
    // own upload is drawn, the one from the other tenant's is not.
    let graph = call_tool(
        &router,
        &owner,
        "get_lineage",
        json!({ "focus": "silver.group_mixed" }),
    )
    .await;
    let mappings = graph["result"]["columnMappings"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of mappings: {graph}"));
    assert_eq!(mappings.len(), 1, "{graph}");
    assert_eq!(
        mappings[0]["from"]["table"],
        json!(format!("bronze.{GROUP_TABLE}")),
        "{graph}"
    );
    assert!(!graph.to_string().contains(RETAIL_TABLE), "{graph}");

    // Focus on the other tenant's table itself: nothing is drawn for it.
    let graph = call_tool(
        &router,
        &owner,
        "get_lineage",
        json!({ "focus": format!("bronze.{RETAIL_TABLE}") }),
    )
    .await;
    assert_eq!(graph["result"]["columnMappings"], json!([]), "{graph}");
    assert!(
        !node_ids(&graph["result"]).contains(&format!("bronze:{RETAIL_TABLE}")),
        "{graph}"
    );
}

#[tokio::test]
async fn the_namespace_count_of_a_listing_of_many_claimed_tables_is_complete() {
    // 250 tables, all claimed by the owning tenant, in one namespace.
    let catalog = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/config"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"defaults":{},"overrides":{}}"#),
        )
        .mount(&catalog)
        .await;
    mount_namespaces(&catalog).await;
    let identifiers: Vec<Value> = (0..250)
        .map(|n| json!({ "namespace": ["bronze"], "name": format!("many_{n}") }))
        .collect();
    Mock::given(method("GET"))
        .and(path("/v1/namespaces/bronze/tables"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "identifiers": identifiers })),
        )
        .mount(&catalog)
        .await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    for n in 0..250 {
        let claimed = claim_table(
            &pool,
            CATALOG_TENANT.parse::<Uuid>().expect("a seeded tenant id"),
            &format!("many_{n}"),
            &format!("upload-many-{n}"),
        )
        .await
        .expect("claim the upload table");
        assert!(claimed);
    }
    let owner = cookie_in_tenant(&pool, "catalog:read", CATALOG_TENANT).await;
    let other = cookie_in_tenant(&pool, "catalog:read", OTHER_TENANT).await;

    let (status, body) = get(&router, "/api/lakehouse/namespaces", &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(table_count(&body, "bronze"), json!(250), "{body}");

    // All 250 belong to the owner: the other tenant counts none of them.
    let (status, body) = get(&router, "/api/lakehouse/namespaces", &other, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(table_count(&body, "bronze"), Value::Null, "{body}");
}

// ── Plan 95, part 2: the maintenance routes and the catalog routes ──────

/// A service identity with no scopes, and its bearer token: the shape of
/// `lakehouse-maintenance-policy-reader`, the identity Dagster reads the
/// policy list with (`bootstrap_lakehouse_maintenance_service`).
async fn service_token_without_scopes(pool: &PgPool) -> String {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO service_identity (id, name, scopes, environment, expires_at) \
         VALUES ($1, $2, '{}', 'production', now() + interval '1 day')",
    )
    .bind(id)
    .bind(format!("tenant-scope-reader-{id}"))
    .execute(pool)
    .await
    .expect("create the service identity");
    lakehouse_auth::service_token::create_service_credential(pool, id)
        .await
        .expect("mint the service identity's credential")
        .expose()
        .to_owned()
}

/// `GET uri` as the holder of a service bearer token.
async fn get_as_service(router: &axum::Router, uri: &str, token: &str) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
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

/// The `namespace.tableName` of each policy a policy list answered, sorted.
fn policy_names(body: &Value) -> Vec<String> {
    let mut names: Vec<String> = body["policies"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of policies: {body}"))
        .iter()
        .map(|p| {
            format!(
                "{}.{}",
                p["namespace"].as_str().expect("a namespace"),
                p["tableName"].as_str().expect("a table name")
            )
        })
        .collect();
    names.sort();
    names
}

/// One policy for each of the three Bronze tables, written by an operator
/// (whose reach is everything).
async fn store_a_policy_for_each_table(router: &axum::Router, operator: &str) {
    for table in [GROUP_TABLE, RETAIL_TABLE, SHARED_TABLE] {
        let (status, body) = send(
            router,
            "POST",
            &format!("/api/lakehouse/tables/bronze/{table}/maintenance"),
            operator,
            None,
            Some(policy_body()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
}

#[tokio::test]
async fn the_maintenance_history_route_leaves_out_the_tables_the_caller_may_not_read() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_clickhouse_with_data(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let uri = "/api/governance/maintenance";
    let owner = cookie_in_tenant(&pool, "governance:read", CATALOG_TENANT).await;
    let other = cookie_in_tenant(&pool, "governance:read", OTHER_TENANT).await;
    let operator = operator_cookie(&pool).await;

    // The owning tenant: its own upload and the shared table, never the
    // upload the other tenant holds.
    let (status, body) = get(&router, uri, &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        maintenance_tables(&body),
        vec![
            format!("bronze.{GROUP_TABLE}"),
            format!("bronze.{SHARED_TABLE}")
        ],
        "{body}"
    );

    // Another tenant: its own upload only. The shared table and the owning
    // tenant's upload are named nowhere.
    let (status, body) = get(&router, uri, &other, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        maintenance_tables(&body),
        vec![format!("bronze.{RETAIL_TABLE}")],
        "{body}"
    );

    // An operator reaches everything.
    let (status, body) = get(&router, uri, &operator, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(maintenance_tables(&body).len(), 3, "{body}");
}

#[tokio::test]
async fn the_policy_list_holds_only_the_policies_of_the_tables_a_person_may_read() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let operator = operator_cookie(&pool).await;
    store_a_policy_for_each_table(&router, &operator).await;
    let uri = "/api/lakehouse/maintenance-policies";
    let owner = cookie_in_tenant(&pool, "catalog:read", CATALOG_TENANT).await;
    let other = cookie_in_tenant(&pool, "catalog:read", OTHER_TENANT).await;

    // The owning tenant: its own upload and the shared table, not the other
    // tenant's upload (the reverse direction).
    let (status, body) = get(&router, uri, &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        policy_names(&body),
        vec![
            format!("bronze.{GROUP_TABLE}"),
            format!("bronze.{SHARED_TABLE}")
        ],
        "{body}"
    );

    // Another tenant: its own upload only.
    let (status, body) = get(&router, uri, &other, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        policy_names(&body),
        vec![format!("bronze.{RETAIL_TABLE}")],
        "{body}"
    );

    // An operator reaches all three.
    let (status, body) = get(&router, uri, &operator, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(policy_names(&body).len(), 3, "{body}");
}

#[tokio::test]
async fn the_policy_list_stays_whole_for_the_service_identity_dagster_reads_it_with() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let operator = operator_cookie(&pool).await;
    store_a_policy_for_each_table(&router, &operator).await;
    let token = service_token_without_scopes(&pool).await;

    // The maintenance job runs every table's policy, whichever tenant holds
    // it, so its identity (no tenant, no scope) reads the whole list.
    let (status, body) =
        get_as_service(&router, "/api/lakehouse/maintenance-policies", &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        policy_names(&body),
        vec![
            format!("bronze.{GROUP_TABLE}"),
            format!("bronze.{RETAIL_TABLE}"),
            format!("bronze.{SHARED_TABLE}")
        ],
        "{body}"
    );
}

/// The registry rows of the two upload assets: the slug each is addressed
/// by, and the table it names.
const GROUP_SLUG: &str = "ds_group_orders";
const RETAIL_SLUG: &str = "ds_retail_orders";
/// A dataset whose registry row records no table name.
const KEYLESS_SLUG: &str = "ds_no_table";

/// `ClickHouse` as the Bronze registry holds two upload datasets, one on the
/// owning tenant's table and one on the other tenant's, both readable
/// through the Iceberg query database. Every other query answers no rows.
#[derive(Clone, Default)]
struct RegistryFake {
    /// Also registers a dataset whose row names no table.
    keyless: bool,
}

fn ch_rows(meta: &[&str], data: &[Value]) -> ResponseTemplate {
    let count = data.len();
    let meta: Vec<Value> = meta
        .iter()
        .map(|name| json!({ "name": name, "type": "String" }))
        .collect();
    ResponseTemplate::new(200).set_body_json(json!({ "meta": meta, "data": data, "rows": count }))
}

/// The table a slug of the two registry rows names.
fn table_of_slug(sql: &str) -> Option<&'static str> {
    [(GROUP_SLUG, GROUP_TABLE), (RETAIL_SLUG, RETAIL_TABLE)]
        .into_iter()
        .find(|(slug, _)| sql.contains(&format!("'{slug}'")))
        .map(|(_, table)| table)
}

impl wiremock::Respond for RegistryFake {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        // The list: both registry rows.
        if sql.starts_with("SELECT slug, title, description, tier, updated_at, table_name") {
            let mut registered = vec![(GROUP_SLUG, GROUP_TABLE), (RETAIL_SLUG, RETAIL_TABLE)];
            if self.keyless {
                registered.push((KEYLESS_SLUG, ""));
            }
            let rows: Vec<Value> = registered
                .into_iter()
                .map(|(slug, table)| {
                    json!({
                        "slug": slug, "title": slug, "description": "orders",
                        "tier": "primer", "updated_at": "2026-11-01T00:00:00Z",
                        "table_name": table, "lag": "60",
                    })
                })
                .collect();
            return ch_rows(&["slug"], &rows);
        }
        // The page's header row for one slug.
        if sql.starts_with("SELECT slug, title, description, tier, table_name, toString(total)") {
            return match table_of_slug(&sql) {
                Some(table) => ch_rows(
                    &["slug"],
                    &[json!({
                        "slug": if table == GROUP_TABLE { GROUP_SLUG } else { RETAIL_SLUG },
                        "title": "orders", "description": "orders", "tier": "primer",
                        "table_name": table, "total": "10", "author": "", "frekuensi": "",
                        "satuan": "", "klasifikasi": "", "updated_at": "2026-11-01T00:00:00Z",
                        "lag": "60",
                    })],
                ),
                None => ch_rows(&[], &[]),
            };
        }
        // The table a slug names, for the sample and the profile.
        if sql.starts_with("SELECT table_name FROM lake.`bronze_meta.dataset_sync`") {
            return match table_of_slug(&sql) {
                Some(table) => ch_rows(&["table_name"], &[json!({ "table_name": table })]),
                None => ch_rows(&[], &[]),
            };
        }
        if sql.contains("FROM system.columns") && sql.contains("default_kind") {
            return ch_rows(
                &["name", "default_kind", "default_expression"],
                &[json!({ "name": "id", "default_kind": "", "default_expression": "" })],
            );
        }
        if sql.starts_with("DESCRIBE TABLE") {
            return ch_rows(
                &["name", "type"],
                &[json!({ "name": "id", "type": "Int64" })],
            );
        }
        if sql.contains("icecat_api") {
            return ch_rows(&["id"], &[json!({ "id": "1" })]);
        }
        ch_rows(&[], &[])
    }
}

/// The deployment of the catalog routes: Meridian Group owns the shared
/// catalog, `fake` is its `ClickHouse`, and `iceberg` says whether the
/// deployment has an Iceberg query database to read the upload tables
/// through.
async fn start_registry_app(ch: &MockServer, fake: RegistryFake, iceberg: bool) -> TestApp {
    Mock::given(method("POST"))
        .respond_with(fake)
        .mount(ch)
        .await;
    let mut env = HashMap::new();
    env.insert("CH_URL".to_owned(), ch.uri());
    env.insert("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT.to_owned());
    if iceberg {
        env.insert("ICEBERG_QUERY_DB".to_owned(), "icecat_api".to_owned());
    }
    spin_up_with_env(&env).await
}

/// The registry holds the two upload datasets, and both are readable.
async fn spin_up_with_registry(ch: &MockServer) -> TestApp {
    start_registry_app(ch, RegistryFake { keyless: false }, true).await
}

/// The ids of the assets a catalog list or page answered.
fn asset_ids(body: &Value, key: &str) -> Vec<String> {
    let mut ids: Vec<String> = body[key]
        .as_array()
        .unwrap_or_else(|| panic!("a list of {key}: {body}"))
        .iter()
        .map(|a| a["id"].as_str().expect("an asset id").to_owned())
        .collect();
    ids.sort();
    ids
}

const CATALOG_READER_WITH_SAMPLES: &str = "catalog:read, query:read";

/// The registry, as the owning tenant and an operator read it.
async fn registry_readers() -> (MockServer, axum::Router, String, String) {
    let ch = MockServer::start().await;
    let TestApp { router, pool } = spin_up_with_registry(&ch).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, "catalog:read", CATALOG_TENANT).await;
    let operator = operator_cookie(&pool).await;
    // The mock server stands in for `ClickHouse` only while it is alive.
    (ch, router, owner, operator)
}

/// The other tenant's upload is named nowhere in `body`.
fn assert_no_trace_of_the_retail_upload(uri: &str, body: &Value) {
    let text = body.to_string();
    assert!(
        !text.contains(RETAIL_TABLE) && !text.contains(RETAIL_SLUG),
        "the other tenant's upload is named nowhere in {uri}: {body}"
    );
}

#[tokio::test]
async fn the_catalog_list_leaves_out_another_tenants_upload_asset_for_the_owning_tenant() {
    let (_ch, router, owner, operator) = registry_readers().await;

    let (status, body) = get(&router, "/api/catalog", &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(asset_ids(&body, "assets"), vec![GROUP_SLUG], "{body}");
    assert_no_trace_of_the_retail_upload("/api/catalog", &body);

    // An operator reaches both.
    let (status, body) = get(&router, "/api/catalog", &operator, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(asset_ids(&body, "assets").len(), 2, "{body}");
}

#[tokio::test]
async fn the_catalog_search_leaves_out_another_tenants_upload_asset_for_the_owning_tenant() {
    let (_ch, router, owner, _operator) = registry_readers().await;

    let (status, body) = get(&router, "/api/catalog?q=orders", &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(asset_ids(&body, "assets"), vec![GROUP_SLUG], "{body}");
    assert_no_trace_of_the_retail_upload("/api/catalog?q=orders", &body);

    // The count of a limited search is of the assets the caller may see.
    let (status, body) = get(&router, "/api/catalog?q=orders&limit=1", &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], json!(1), "{body}");
}

#[tokio::test]
async fn the_paged_catalog_query_leaves_out_another_tenants_upload_asset_for_the_owning_tenant() {
    let (_ch, router, owner, _operator) = registry_readers().await;

    // The page counts what it holds before it pages, so the total is the
    // caller's.
    let (status, body) = get(&router, "/api/catalog/query", &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(asset_ids(&body, "items"), vec![GROUP_SLUG], "{body}");
    assert_eq!(body["totalItems"], json!(1), "{body}");
    assert_no_trace_of_the_retail_upload("/api/catalog/query", &body);
}

/// The read at `/api/catalog/{slug}{suffix}`, as the owning tenant makes it:
/// the other tenant's upload answers the 404 an unknown id answers, byte for
/// byte, and the owning tenant's own upload still opens. With `iceberg` unset
/// no upload table is readable, so the sample and the profile have no source
/// to read and the check cannot rest on one: the owner's own upload then
/// answers its "nothing to read" 200 instead.
async fn assert_the_other_tenants_upload_is_the_unknown_404(suffix: &str, iceberg: bool) {
    let ch = MockServer::start().await;
    let TestApp { router, pool } =
        start_registry_app(&ch, RegistryFake { keyless: false }, iceberg).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, CATALOG_READER_WITH_SAMPLES, CATALOG_TENANT).await;

    let (status, unknown) = get(
        &router,
        &format!("/api/catalog/ds_no_such{suffix}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{unknown}");

    let (status, refused) = get(
        &router,
        &format!("/api/catalog/{RETAIL_SLUG}{suffix}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{refused}");
    assert_eq!(refused, unknown, "a refusal is no oracle");

    let (status, own) = get(
        &router,
        &format!("/api/catalog/{GROUP_SLUG}{suffix}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{own}");
}

#[tokio::test]
async fn the_page_of_another_tenants_upload_asset_is_the_unknown_404_for_the_owning_tenant() {
    assert_the_other_tenants_upload_is_the_unknown_404("", true).await;
}

#[tokio::test]
async fn the_sample_of_another_tenants_upload_asset_is_the_unknown_404_for_the_owning_tenant() {
    assert_the_other_tenants_upload_is_the_unknown_404("/sample", true).await;
}

#[tokio::test]
async fn the_profile_of_another_tenants_upload_asset_is_the_unknown_404_for_the_owning_tenant() {
    assert_the_other_tenants_upload_is_the_unknown_404("/profile", true).await;
}

#[tokio::test]
async fn the_sample_of_another_tenants_upload_asset_is_the_unknown_404_when_no_table_can_be_read() {
    assert_the_other_tenants_upload_is_the_unknown_404("/sample", false).await;
}

#[tokio::test]
async fn the_profile_of_another_tenants_upload_asset_is_the_unknown_404_when_no_table_can_be_read()
{
    assert_the_other_tenants_upload_is_the_unknown_404("/profile", false).await;
}

#[tokio::test]
async fn an_asset_that_names_no_table_stays_in_the_lists_of_the_owning_tenant() {
    let ch = MockServer::start().await;
    let TestApp { router, pool } =
        start_registry_app(&ch, RegistryFake { keyless: true }, true).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, "catalog:read", CATALOG_TENANT).await;
    let expected = vec![GROUP_SLUG.to_owned(), KEYLESS_SLUG.to_owned()];

    // The owner holds the shared catalog, and an asset with no table is the
    // catalog's own, not another tenant's upload.
    let (status, body) = get(&router, "/api/catalog", &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(asset_ids(&body, "assets"), expected, "{body}");

    let (status, body) = get(&router, "/api/catalog/query", &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(asset_ids(&body, "items"), expected, "{body}");
    assert_no_trace_of_the_retail_upload("/api/catalog/query", &body);
}

// ── Plan 96: the console's lineage routes and the classification list ───
//
// Plan 95 put the shared-catalog rule in the assistant's `get_lineage` tool
// only. The console routes draw the same dashboard charts, and a tenant that
// does not own the shared catalog must not read them there either. A dashboard
// chart carries no tenant, so it belongs to the shared catalog's owner.

/// What a lineage reader holds, to open the console routes (`lineage:read`)
/// and name charts (`dashboard:read`).
const LINEAGE_ROUTE_READER: &str = "lineage:read, dashboard:read";

/// The lineage graph's nodes and edges are empty.
fn assert_an_empty_graph(body: &Value) {
    assert_eq!(body["supported"], json!(true), "{body}");
    assert_eq!(node_ids(body), Vec::<String>::new(), "{body}");
    assert_eq!(body["edges"], json!([]), "{body}");
    assert_eq!(body["columnMappings"], json!([]), "{body}");
}

#[tokio::test]
async fn the_lineage_page_draws_no_dashboard_chart_for_a_tenant_that_does_not_own_the_shared_catalog()
 {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_clickhouse_with_data(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let other = cookie_in_tenant(&pool, LINEAGE_ROUTE_READER, OTHER_TENANT).await;

    // By the table the chart reads, by the chart's id, and by its title.
    for focus in [
        format!("serving.{CHART_MART}"),
        format!("chart:{CHART_ID}"),
        CHART_TITLE.to_owned(),
    ] {
        let (status, body) = get(
            &router,
            &format!(
                "/api/governance/lineage?focus={}",
                focus.replace(' ', "%20")
            ),
            &other,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_an_empty_graph(&body);
    }
}

#[tokio::test]
async fn the_lineage_page_still_draws_the_dashboard_charts_for_the_tenant_that_owns_the_shared_catalog()
 {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_clickhouse_with_data(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, LINEAGE_ROUTE_READER, CATALOG_TENANT).await;
    let operator = operator_cookie(&pool).await;

    for cookie in [&owner, &operator] {
        let (status, body) = get(
            &router,
            &format!("/api/governance/lineage?focus=serving.{CHART_MART}"),
            cookie,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let ids = node_ids(&body);
        assert!(
            ids.contains(&format!("chart:{CHART_ID}")),
            "the chart is drawn: {ids:?}"
        );
    }
}

#[tokio::test]
async fn the_chart_lineage_route_answers_the_unknown_404_to_a_tenant_that_does_not_own_the_shared_catalog()
 {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_clickhouse_with_data(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, LINEAGE_ROUTE_READER, CATALOG_TENANT).await;
    let other = cookie_in_tenant(&pool, LINEAGE_ROUTE_READER, OTHER_TENANT).await;

    // The answer an id nobody has gets, from the owner.
    let (status, unknown) = get(
        &router,
        "/api/dashboard/charts/chart_no_such/lineage",
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{unknown}");

    // Another tenant asking for the owner's chart gets the same answer, byte
    // for byte: no title, no table, and nothing that says the chart exists.
    let (status, refused) = get(
        &router,
        &format!("/api/dashboard/charts/{CHART_ID}/lineage"),
        &other,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{refused}");
    assert_eq!(refused, unknown, "a refusal is no oracle");
    assert_none_of_the_groups_data(&refused);
}

#[tokio::test]
async fn the_chart_lineage_route_still_answers_the_tenant_that_owns_the_shared_catalog() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_clickhouse_with_data(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, LINEAGE_ROUTE_READER, CATALOG_TENANT).await;
    let operator = operator_cookie(&pool).await;

    for cookie in [&owner, &operator] {
        let (status, body) = get(
            &router,
            &format!("/api/dashboard/charts/{CHART_ID}/lineage"),
            cookie,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["chart"]["title"], json!(CHART_TITLE), "{body}");
    }
}

#[tokio::test]
async fn the_used_by_list_names_no_chart_to_a_tenant_that_does_not_own_the_shared_catalog() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_clickhouse_with_data(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, LINEAGE_ROUTE_READER, CATALOG_TENANT).await;
    let other = cookie_in_tenant(&pool, LINEAGE_ROUTE_READER, OTHER_TENANT).await;
    let uri = format!("/api/governance/lineage/used-by?table=serving.{CHART_MART}");

    // The owner lists its chart.
    let (status, body) = get(&router, &uri, &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["charts"][0]["chartId"], json!(CHART_ID), "{body}");
    assert_eq!(
        body["charts"][0]["chartTitle"],
        json!(CHART_TITLE),
        "{body}"
    );

    // Another tenant gets the shared catalog's own refusal: no chart and no
    // count, so the answer cannot be read as "no chart reads this table". It
    // is the same sentence for every table, so it is no oracle.
    let reason = the_rules_reason(&router, &pool).await;
    let (status, body) = get(&router, &uri, &other, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["supported"], json!(false), "{body}");
    assert_eq!(body["reason"], json!(reason), "{body}");
    assert!(body.get("charts").is_none(), "{body}");
    assert!(body.get("hiddenCount").is_none(), "{body}");
    assert_none_of_the_groups_data(&body);
    let other_table = "/api/governance/lineage/used-by?table=serving.mart_no_such";
    let (_, elsewhere) = get(&router, other_table, &other, None).await;
    assert_eq!(elsewhere, body, "the same answer for every table");
}

#[tokio::test]
async fn the_column_lineage_route_names_no_dependent_chart_to_a_tenant_that_does_not_own_the_shared_catalog()
 {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_clickhouse_with_data(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, LINEAGE_ROUTE_READER, CATALOG_TENANT).await;
    let other = cookie_in_tenant(&pool, LINEAGE_ROUTE_READER, OTHER_TENANT).await;
    // The caller names the starting column, and a chart may read that column.
    let uri = format!(
        "/api/governance/lineage/columns?table=serving.{CHART_MART}&column=visitors&direction=down"
    );

    // The owner is told which chart reads the column.
    let (status, body) = get(&router, &uri, &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["charts"][0]["chartId"], json!(CHART_ID), "{body}");

    // Another tenant is told of no chart: they are hidden, as they are from a
    // caller without `dashboard:read`.
    let (status, body) = get(&router, &uri, &other, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.get("charts").is_none(), "{body}");
    assert_eq!(body["chartsHidden"], json!(true), "{body}");
    assert!(!body.to_string().contains(CHART_TITLE), "{body}");
}

// The classification list.

const GOVERNANCE_READER: &str = "governance:read";
const CHAT_AND_GOVERNANCE_READ: &str = "assistant:use, assistant:ask, governance:read";

/// `ClickHouse` whose Bronze registry classifies the owner's upload, the
/// other tenant's upload, and one table of the shared catalog.
async fn mount_classified_registry(server: &MockServer) {
    mount_empty_clickhouse(server).await;
    let row = |slug: &str, table: &str, tier: &str| json!({ "slug": slug, "table_name": table, "tier": tier });
    clickhouse_rows(
        server,
        "bronze_meta.dataset_catalog",
        vec![
            row(GROUP_SLUG, GROUP_TABLE, "confidential"),
            row(RETAIL_SLUG, RETAIL_TABLE, "restricted"),
            row(DATASET_SLUG, DATASET_TABLE, "internal"),
        ],
    )
    .await;
}

/// The Bronze tables a classification answer names, sorted.
fn classified_bronze_assets(body: &Value) -> Vec<String> {
    let mut assets: Vec<String> = body["classifications"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of classifications: {body}"))
        .iter()
        .filter_map(|row| row["asset"].as_str())
        .filter(|asset| asset.starts_with("bronze."))
        .map(str::to_owned)
        .collect();
    assets.sort();
    assets
}

#[tokio::test]
async fn the_classification_list_holds_only_the_tables_the_caller_may_read() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_classified_registry(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let uri = "/api/governance/classification";
    let owner = cookie_in_tenant(&pool, GOVERNANCE_READER, CATALOG_TENANT).await;
    let other = cookie_in_tenant(&pool, GOVERNANCE_READER, OTHER_TENANT).await;
    let operator = operator_cookie(&pool).await;

    // The owning tenant: its own upload and the shared catalog's table, never
    // the upload the other tenant holds.
    let (status, body) = get(&router, uri, &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        classified_bronze_assets(&body),
        vec![
            format!("bronze.{GROUP_TABLE}"),
            format!("bronze.{DATASET_TABLE}")
        ],
        "{body}"
    );
    assert_no_trace_of_the_retail_upload(uri, &body);

    // Another tenant: its own upload only. Neither the shared catalog's table
    // nor the owning tenant's upload is named.
    let (status, body) = get(&router, uri, &other, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        classified_bronze_assets(&body),
        vec![format!("bronze.{RETAIL_TABLE}")],
        "{body}"
    );
    let text = body.to_string();
    for hidden in [GROUP_TABLE, GROUP_SLUG, DATASET_TABLE, DATASET_SLUG] {
        assert!(!text.contains(hidden), "{hidden} must not appear: {body}");
    }

    // An operator reaches all three.
    let (status, body) = get(&router, uri, &operator, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(classified_bronze_assets(&body).len(), 3, "{body}");
}

#[tokio::test]
async fn the_classification_tool_holds_only_the_tables_the_person_may_read() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_classified_registry(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, CHAT_AND_GOVERNANCE_READ, CATALOG_TENANT).await;
    let other = cookie_in_tenant(&pool, CHAT_AND_GOVERNANCE_READ, OTHER_TENANT).await;

    let listed = call_tool(&router, &owner, "list_classification_rules", json!({})).await;
    assert_eq!(
        classified_bronze_assets(&listed["result"]),
        vec![
            format!("bronze.{GROUP_TABLE}"),
            format!("bronze.{DATASET_TABLE}")
        ],
        "{listed}"
    );
    assert_no_trace_of_the_retail_upload("list_classification_rules", &listed);

    let listed = call_tool(&router, &other, "list_classification_rules", json!({})).await;
    assert_eq!(
        classified_bronze_assets(&listed["result"]),
        vec![format!("bronze.{RETAIL_TABLE}")],
        "{listed}"
    );
    let text = listed.to_string();
    for hidden in [GROUP_TABLE, GROUP_SLUG, DATASET_TABLE, DATASET_SLUG] {
        assert!(!text.contains(hidden), "{hidden} must not appear: {listed}");
    }
}
