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
//!
//! The last section of all holds the audit trail, which is read per tenant
//! for the same reason and by the same rule (`SEC-20`, plan 100): the trail
//! has no tenant column, so a row's tenant is read from its actor, at read
//! time, and the rows of another organisation's are not returned to a
//! reader of this one.

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
/// `tenant`: its id and its session cookie.
async fn principal_in_tenant(pool: &PgPool, permissions: &str, tenant: &str) -> (Uuid, String) {
    let user = create_principal_with_permissions(pool, permissions).await;
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(user)
        .bind(tenant.parse::<Uuid>().expect("a seeded tenant id"))
        .execute(pool)
        .await
        .expect("add the membership");
    let cookie = session_cookie_for_user(pool, user).await;
    (user, cookie)
}

/// A principal that holds exactly `permissions` and belongs to exactly
/// `tenant`, as a session cookie.
async fn cookie_in_tenant(pool: &PgPool, permissions: &str, tenant: &str) -> String {
    principal_in_tenant(pool, permissions, tenant).await.1
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

/// The catalog's own tenant's upload is named nowhere in `body`.
fn assert_no_trace_of_the_group_upload(uri: &str, body: &Value) {
    let text = body.to_string();
    assert!(
        !text.contains(GROUP_TABLE) && !text.contains(GROUP_SLUG),
        "the owning tenant's upload is named nowhere in {uri}: {body}"
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

// ── The quality readers and the agent run history (SEC-20, plan ai-118).
//
// The quality pages answer with the rows of every table a job recorded, and
// the agent run history answers with the runs of the deployment. Both name
// what another organisation holds, so both answer by the tenant gate: a
// quality reader keeps the rows of the tables the caller's reach covers, and
// the run history belongs to the deployment, so only the caller the shared
// catalog's own rule admits reads it.

/// `ClickHouse` with a quality observation for each of `tables`.
async fn mount_observed_quality(server: &MockServer, tables: &[&str]) {
    mount_empty_clickhouse(server).await;
    let rows: Vec<Value> = tables
        .iter()
        .map(|table| {
            json!({
                "tabel": format!("bronze.{table}"),
                "cek": "row_count",
                "verdict": "warn",
                "nilai": "10",
                "at": "2026-11-14 03:00:00",
            })
        })
        .collect();
    clickhouse_rows(server, "_silver_meta.quality", rows).await;
}

/// `ClickHouse` with the run history of the rule `id`, which checked `table`.
async fn mount_the_run_history(server: &MockServer, id: Uuid, table: &str) {
    clickhouse_rows(
        server,
        &format!("rule_id = '{id}'"),
        vec![json!({
            "rule_id": id.to_string(),
            "asset": format!("bronze.{table}"),
            "status": "failed",
            "value": "10 rows",
            "run_by": "schedule",
            "threshold": ">= 95%",
            "at": "2026-11-14 03:00:00",
            "ran_at": "2026-11-14T03:00:00Z",
        })],
    )
    .await;
}

// ── The audit trail is read per tenant (`SEC-20`, plan 100) ─────────────

/// What a reader of the trail holds: `audit:read` for the events route and
/// the CSV export, `governance:read` for the assistant's tool, and
/// `assistant:use` to call a tool at all.
const AUDIT_READER: &str = "audit:read, governance:read, assistant:use";

/// The deployment of the audit-trail tests: Meridian Group owns the shared
/// catalog (`CATALOG_TENANT_ID`), so the deployment's own rows — the ones
/// nobody signed in caused — are in its reach, and Meridian Retail's are
/// not. The other seeded tenants stay, so the deployment holds several and
/// the reach is a real one.
async fn spin_up_for_the_audit_trail() -> TestApp {
    let mut env = HashMap::new();
    env.insert("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT.to_owned());
    spin_up_with_env(&env).await
}

/// One row of the trail, written the way the trail's writers write one:
/// the actor's id as text, the kind of the actor, a label, and the action.
async fn seed_audit_row(
    pool: &PgPool,
    id: &str,
    at: &str,
    principal_id: Option<&str>,
    principal_kind: &str,
    actor_label: &str,
    action: &str,
    resource_id: &str,
) {
    sqlx::query(
        "INSERT INTO audit_event (id, at, principal_id, principal_kind, actor_label, action, \
         resource_kind, resource_id, args, outcome) \
         VALUES ($1, $2::timestamptz, $3, $4, $5, $6, 'dashboard', $7, '{}'::jsonb, 'executed')",
    )
    .bind(id)
    .bind(at)
    .bind(principal_id)
    .bind(principal_kind)
    .bind(actor_label)
    .bind(action)
    .bind(resource_id)
    .execute(pool)
    .await
    .expect("seed an audit_event row");
}

/// The trail the audit-trail tests read: one row of Meridian Group, one of
/// Meridian Retail, and one the deployment caused itself — nobody signed in
/// for that one, so it belongs to the deployment.
async fn seed_the_trail(pool: &PgPool, group: Uuid, retail: Uuid) {
    seed_audit_row(
        pool,
        "trail-of-group",
        "2099-01-01T10:00:00Z",
        Some(&group.to_string()),
        "user",
        "Ayu of Meridian Group",
        "dashboard.update",
        "row-of-tenant-a",
    )
    .await;
    seed_audit_row(
        pool,
        "trail-of-retail",
        "2099-01-02T10:00:00Z",
        Some(&retail.to_string()),
        "user",
        "Rafi of Meridian Retail",
        "query.run",
        "row-of-tenant-b",
    )
    .await;
    seed_audit_row(
        pool,
        "trail-of-the-deployment",
        "2099-01-03T10:00:00Z",
        None,
        "schedule",
        "Dagster",
        "pipeline.build",
        "row-of-the-deployment",
    )
    .await;
}

/// The quality rows of both tenants' upload tables: a rule, an incident, a
/// schedule and a load each. The two rule ids come back, so a test can ask
/// for the run history of one of them.
async fn seed_the_quality_rows(pool: &PgPool) -> (Uuid, Uuid) {
    let mut rules = Vec::new();
    for table in [GROUP_TABLE, RETAIL_TABLE] {
        let asset = format!("bronze.{table}");
        let (id,): (Uuid,) = sqlx::query_as(
            "INSERT INTO quality_rule (name, asset, dimension, threshold, severity) \
             VALUES ($1, $2, 'completeness', '>= 95%', 'medium') RETURNING id",
        )
        .bind(format!("the rule on {table}"))
        .bind(&asset)
        .fetch_one(pool)
        .await
        .expect("add the quality rule");
        rules.push(id);
        sqlx::query(
            "INSERT INTO quality_incident (asset, kind, detail) \
             VALUES ($1, 'volume_anomaly', $2)",
        )
        .bind(&asset)
        .bind(format!("the load of {table} dropped"))
        .execute(pool)
        .await
        .expect("add the quality incident");
        sqlx::query(
            "INSERT INTO quality_schedule (asset, mode, interval_minutes) \
             VALUES ($1, 'interval', 60)",
        )
        .bind(&asset)
        .execute(pool)
        .await
        .expect("add the quality schedule");
        sqlx::query("INSERT INTO table_load (asset, loaded_at, rows_after) VALUES ($1, now(), 10)")
            .bind(&asset)
            .execute(pool)
            .await
            .expect("add the load");
    }
    (rules[0], rules[1])
}

/// The Bronze tables the quality rules name, sorted and without repeats.
///
/// The rules of the shared catalog's own tables (`silver.`/`serving.`) are
/// named too: they belong to the catalog, so the tenant that owns the
/// catalog reads them, and they are not another organisation's upload.
fn quality_assets(body: &Value) -> Vec<String> {
    let mut assets: Vec<String> = body["quality"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of quality rules: {body}"))
        .iter()
        .filter_map(|row| row["asset"].as_str())
        .filter(|asset| asset.starts_with("bronze."))
        .map(str::to_owned)
        .collect();
    assets.sort();
    assets.dedup();
    assets
}

/// The ids of the observed quality rows of a body, in the order they came.
///
/// An observed row's id is `q-<n>` and numbers the rows of every tenant's
/// observations, so it is the one field of a visible row that can say how many
/// rows a filter dropped; an authored rule's id is a UUID and is not one.
fn observed_quality_ids(body: &Value) -> Vec<String> {
    body["quality"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of quality rules: {body}"))
        .iter()
        .filter_map(|row| row["id"].as_str())
        .filter(|id| id.starts_with("q-"))
        .map(str::to_owned)
        .collect()
}

/// The assets of the incidents in a body, sorted.
fn incident_assets(body: &Value) -> Vec<String> {
    let mut assets: Vec<String> = body["incidents"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of incidents: {body}"))
        .iter()
        .filter_map(|row| row["asset"].as_str())
        .map(str::to_owned)
        .collect();
    assets.sort();
    assets
}

/// The assets of the schedules in a body, sorted.
fn schedule_assets(body: &Value) -> Vec<String> {
    let mut assets: Vec<String> = body["schedules"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of schedules: {body}"))
        .iter()
        .filter_map(|row| row["asset"].as_str())
        .map(str::to_owned)
        .collect();
    assets.sort();
    assets
}

/// A run of a seeded employee, so the run history has a row to show.
async fn seed_an_agent_run(pool: &PgPool) -> String {
    sqlx::query(
        "INSERT INTO agent_run (id, employee_id, status, trigger, actor, started_at, steps) \
         VALUES ('run-sec-20', 'emp-inventory', 'completed', 'Manual investigation', \
                 'inventory-copilot', now() - interval '2 minutes', '[]'::jsonb)",
    )
    .execute(pool)
    .await
    .expect("add the agent run");
    "run-sec-20".to_owned()
}

/// The grant of a plain reader of the agents pages. SEC-66 put the run
/// history behind `agent:read`, so this now carries that grant alongside
/// `dashboard:read`; the tenant split under test is unchanged.
const AGENTS_READER: &str = "dashboard:read, agent:read";

#[tokio::test]
async fn the_quality_list_holds_only_the_tables_the_caller_may_read() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    seed_the_quality_rows(&pool).await;
    // The other tenant's table is mounted first on purpose: the ids of the
    // observed rows number every tenant's rows, so a visible row that sorted
    // after a hidden one would carry a gap in its id and say how many rows of
    // another organisation's tables came before it.
    mount_observed_quality(&ch, &[RETAIL_TABLE, GROUP_TABLE]).await;
    let uri = "/api/governance/quality";
    let owner = cookie_in_tenant(&pool, GOVERNANCE_READER, CATALOG_TENANT).await;
    let other = cookie_in_tenant(&pool, GOVERNANCE_READER, OTHER_TENANT).await;
    let operator = operator_cookie(&pool).await;

    // The owning tenant: its own upload, never the upload the other tenant
    // holds.
    let (status, body) = get(&router, uri, &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        quality_assets(&body),
        vec![format!("bronze.{GROUP_TABLE}")],
        "{body}"
    );
    // The visible row's id says nothing about the hidden row that sorted
    // before it.
    assert_eq!(
        observed_quality_ids(&body),
        vec!["q-0".to_owned()],
        "{body}"
    );
    assert_no_trace_of_the_retail_upload(uri, &body);

    // Another tenant: the shared catalog's own rule refuses it, so it reads
    // no upload row at all.
    let (status, body) = get(&router, uri, &other, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_no_trace_of_the_retail_upload(uri, &body);
    assert_no_trace_of_the_group_upload(uri, &body);

    // An operator reaches everything.
    let (status, body) = get(&router, uri, &operator, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        quality_assets(&body),
        vec![
            format!("bronze.{GROUP_TABLE}"),
            format!("bronze.{RETAIL_TABLE}")
        ],
        "{body}"
    );
    // Nothing is hidden from an operator, so the ids are the ones every row
    // carries.
    assert_eq!(
        observed_quality_ids(&body),
        vec!["q-0".to_owned(), "q-1".to_owned()],
        "{body}"
    );
}

#[tokio::test]
async fn the_rule_runs_of_another_tenants_upload_table_are_the_unknown_404() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let (group_rule, retail_rule) = seed_the_quality_rows(&pool).await;
    mount_the_run_history(&ch, group_rule, GROUP_TABLE).await;
    mount_the_run_history(&ch, retail_rule, RETAIL_TABLE).await;
    let owner = cookie_in_tenant(&pool, GOVERNANCE_READER, CATALOG_TENANT).await;
    let other = cookie_in_tenant(&pool, GOVERNANCE_READER, OTHER_TENANT).await;

    // The owning tenant reads the history of the rule on its own upload.
    let (status, body) = get(
        &router,
        &format!("/api/governance/quality/{group_rule}/runs"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["runs"].as_array().map(Vec::len),
        Some(1),
        "the rule's own run: {body}"
    );

    // The rule on the other tenant's upload table is not there at all.
    let (status, body) = get(
        &router,
        &format!("/api/governance/quality/{retail_rule}/runs"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], json!("Quality rule not found"), "{body}");

    // Another tenant: the shared catalog's own rule refuses it.
    let (status, body) = get(
        &router,
        &format!("/api/governance/quality/{group_rule}/runs"),
        &other,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

#[tokio::test]
async fn the_quality_incidents_hold_only_the_tables_the_caller_may_read() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    seed_the_quality_rows(&pool).await;
    let uri = "/api/governance/quality/incidents";
    let owner = cookie_in_tenant(&pool, GOVERNANCE_READER, CATALOG_TENANT).await;
    let other = cookie_in_tenant(&pool, GOVERNANCE_READER, OTHER_TENANT).await;
    let operator = operator_cookie(&pool).await;

    // The owning tenant: its own upload, never the upload the other tenant
    // holds.
    let (status, body) = get(&router, uri, &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        incident_assets(&body),
        vec![format!("bronze.{GROUP_TABLE}")],
        "{body}"
    );
    assert_no_trace_of_the_retail_upload(uri, &body);

    // Another tenant: the shared catalog's own rule refuses it.
    let (status, body) = get(&router, uri, &other, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_no_trace_of_the_group_upload(uri, &body);

    // An operator reaches everything.
    let (status, body) = get(&router, uri, &operator, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        incident_assets(&body),
        vec![
            format!("bronze.{GROUP_TABLE}"),
            format!("bronze.{RETAIL_TABLE}")
        ],
        "{body}"
    );
}

#[tokio::test]
async fn the_quality_schedules_hold_only_the_tables_the_caller_may_read() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    seed_the_quality_rows(&pool).await;
    let uri = "/api/governance/quality/schedules";
    let owner = cookie_in_tenant(&pool, GOVERNANCE_READER, CATALOG_TENANT).await;
    let other = cookie_in_tenant(&pool, GOVERNANCE_READER, OTHER_TENANT).await;
    let operator = operator_cookie(&pool).await;

    // The owning tenant: its own upload, never the upload the other tenant
    // holds.
    let (status, body) = get(&router, uri, &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        schedule_assets(&body),
        vec![format!("bronze.{GROUP_TABLE}")],
        "{body}"
    );
    assert_no_trace_of_the_retail_upload(uri, &body);

    // Another tenant: the shared catalog's own rule refuses it.
    let (status, body) = get(&router, uri, &other, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_no_trace_of_the_group_upload(uri, &body);

    // An operator reaches everything.
    let (status, body) = get(&router, uri, &operator, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        schedule_assets(&body),
        vec![
            format!("bronze.{GROUP_TABLE}"),
            format!("bronze.{RETAIL_TABLE}")
        ],
        "{body}"
    );
}

#[tokio::test]
async fn the_learning_status_of_another_tenants_upload_table_is_the_unknown_404() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    seed_the_quality_rows(&pool).await;
    let owner = cookie_in_tenant(&pool, GOVERNANCE_READER, CATALOG_TENANT).await;
    let other = cookie_in_tenant(&pool, GOVERNANCE_READER, OTHER_TENANT).await;

    // The owning tenant reads the learning status of its own upload.
    let uri = format!("/api/governance/quality/learning/bronze.{GROUP_TABLE}");
    let (status, body) = get(&router, &uri, &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["asset"],
        json!(format!("bronze.{GROUP_TABLE}")),
        "{body}"
    );
    assert_eq!(body["loads"], json!(1), "{body}");

    // The other tenant's upload table is not there at all.
    let uri = format!("/api/governance/quality/learning/bronze.{RETAIL_TABLE}");
    let (status, body) = get(&router, &uri, &owner, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // Another tenant: the shared catalog's own rule refuses it.
    let uri = format!("/api/governance/quality/learning/bronze.{GROUP_TABLE}");
    let (status, body) = get(&router, &uri, &other, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

// ── The quality writers respect the upload claims (`SEC-80`) ────────────

/// Holds `governance:write` and nothing else: what a person needs to change
/// quality state.
const GOVERNANCE_WRITER: &str = "governance:write";

/// The refusal a writer gives for a table the caller may not read
/// (`routes::quality::unreadable_table`), for the other tenant's table.
const UNREADABLE_RETAIL: &str = "\"bronze.retail_orders\" is not a table you can read. Pick the table as silver.<table>, serving.<table> or bronze.<table>.";
/// The answer of a rule or incident the caller may not reach: the same one an
/// unknown id gets.
const RULE_NOT_FOUND: &str = "Quality rule not found";
const INCIDENT_NOT_FOUND: &str = "Quality incident not found";

/// A rule on `table`'s upload table that a run can measure: a row count,
/// which needs no column the mock catalog lacks.
async fn add_a_rule(pool: &PgPool, table: &str) -> Uuid {
    add_a_rule_on_asset(
        pool,
        &format!("a rule on {table}"),
        &format!("bronze.{table}"),
    )
    .await
}

/// A measurable rule stored with `asset` spelled exactly as given, the way a
/// row written before the writers canonicalised a key would hold it.
async fn add_a_rule_on_asset(pool: &PgPool, name: &str, asset: &str) -> Uuid {
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO quality_rule (name, asset, dimension, threshold, severity) \
         VALUES ($1, $2, 'completeness', 'rows >= 0', 'medium') RETURNING id",
    )
    .bind(name)
    .bind(asset)
    .fetch_one(pool)
    .await
    .expect("add the quality rule");
    id
}

/// The rules whose table is `table`, in the store, under any spelling of the
/// asset: a refused write that stored a mixed-case row would be counted.
async fn rules_on(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM quality_rule WHERE lower(asset) = lower($1)")
        .bind(format!("bronze.{table}"))
        .fetch_one(pool)
        .await
        .expect("count the quality rules")
}

/// The sentence a writer gives for `asset` as the caller spelled it.
fn unreadable_sentence(asset: &str) -> String {
    format!(
        "\"{asset}\" is not a table you can read. Pick the table as \
         silver.<table>, serving.<table> or bronze.<table>."
    )
}

/// How many requests the mock `ClickHouse` has answered that measure or record
/// something: a refused run must add none. A table lookup (`DESCRIBE`,
/// `system.columns`) reads the table's shape and is not counted (`SEC-80`).
async fn clickhouse_requests(ch: &MockServer) -> usize {
    let requests = ch.received_requests().await.unwrap_or_default();
    let mut measured = 0;
    for request in &requests {
        let body = String::from_utf8_lossy(&request.body);
        if !body.contains("DESCRIBE TABLE") && !body.contains("system.columns") {
            measured += 1;
        }
    }
    measured
}

fn rule_body(name: &str, table: &str, threshold: &str) -> Value {
    json!({
        "name": name,
        "asset": format!("bronze.{table}"),
        "dimension": "completeness",
        "threshold": threshold,
        "severity": "medium",
    })
}

fn update_rule_body(table: &str, threshold: &str) -> Value {
    json!({
        "asset": format!("bronze.{table}"),
        "threshold": threshold,
        "severity": "medium",
        "dimension": "completeness",
    })
}

/// The deployment of the quality writers: the catalog deployment with an
/// Iceberg query database, so a rule can be written for a bronze table. The
/// `DESCRIBE` of each upload table named here answers one column, so it is
/// there; any other bronze name answers no columns, so it is not (`SEC-80`).
async fn spin_up_for_the_quality_writers(catalog: &MockServer, ch: &MockServer) -> TestApp {
    for table in [GROUP_TABLE, RETAIL_TABLE, "Retail_Orders"] {
        Mock::given(method("POST"))
            .and(body_string_contains("DESCRIBE TABLE"))
            .and(body_string_contains(format!("bronze.{table}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [{"name": "name", "type": "String"}, {"name": "type", "type": "String"}],
                "data": [{"name": MARKER_COLUMN, "type": "String"}],
                "rows": 1,
            })))
            .with_priority(1)
            .mount(ch)
            .await;
    }
    spin_up_with_env(&quality_writers_env(catalog, ch)).await
}

/// The environment of [`spin_up_for_the_quality_writers`], without its
/// `DESCRIBE` answers: a test that makes `ClickHouse` fail mounts its own.
fn quality_writers_env(catalog: &MockServer, ch: &MockServer) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("CH_URL".to_owned(), ch.uri());
    env.insert("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT.to_owned());
    env.insert("ICEBERG_QUERY_DB".to_owned(), "icecat_api".to_owned());
    env.insert("LAKEKEEPER_CATALOG_URI".to_owned(), catalog.uri());
    env.insert("LAKEKEEPER_WAREHOUSE".to_owned(), "default".to_owned());
    env.insert(
        "LAKEKEEPER_READ_TOKEN_FILE".to_owned(),
        token_file("read.jwt"),
    );
    env
}

#[tokio::test]
async fn the_catalog_owner_cannot_create_a_rule_on_another_tenants_upload_table() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_for_the_quality_writers(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, GOVERNANCE_WRITER, CATALOG_TENANT).await;
    let operator = operator_cookie(&pool).await;
    let uri = "/api/governance/quality";

    // The catalog owner is the catalog's tenant, so the catalog rule admits
    // it; the other tenant's upload table is still refused, and nothing is
    // stored.
    let (status, body) = send(
        &router,
        "POST",
        uri,
        &owner,
        None,
        Some(rule_body("owner on retail", RETAIL_TABLE, "rows >= 0")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], json!(UNREADABLE_RETAIL), "{body}");
    assert_eq!(rules_on(&pool, RETAIL_TABLE).await, 0);

    // The owner's own upload table is allowed.
    let (status, body) = send(
        &router,
        "POST",
        uri,
        &owner,
        None,
        Some(rule_body("owner on group", GROUP_TABLE, "rows >= 0")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(rules_on(&pool, GROUP_TABLE).await, 1);

    // An operator reaches every table.
    let (status, body) = send(
        &router,
        "POST",
        uri,
        &operator,
        None,
        Some(rule_body("operator on retail", RETAIL_TABLE, "rows >= 0")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(rules_on(&pool, RETAIL_TABLE).await, 1);
}

#[tokio::test]
async fn the_catalog_owner_cannot_address_another_tenants_upload_table_in_another_spelling() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_for_the_quality_writers(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, GOVERNANCE_WRITER, CATALOG_TENANT).await;
    let operator = operator_cookie(&pool).await;
    let uri = "/api/governance/quality";

    // Upload table names are stored lowercase, but the zone and the table
    // may be spelled in capitals. Each spelling names the same claimed table,
    // so each gets the lowercase refusal and writes nothing.
    for asset in ["BRONZE.retail_orders", "bronze.Retail_Orders"] {
        let (status, body) = send(
            &router,
            "POST",
            uri,
            &owner,
            None,
            Some(json!({
                "name": format!("owner on {asset}"),
                "asset": asset,
                "dimension": "completeness",
                "threshold": "rows >= 0",
                "severity": "medium",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{asset}: {body}");
        assert_eq!(body["error"], json!(unreadable_sentence(asset)), "{body}");
    }
    assert_eq!(rules_on(&pool, RETAIL_TABLE).await, 0, "nothing stored");

    // The owner's own table is not refused for its spelling.
    let (status, body) = send(
        &router,
        "POST",
        uri,
        &owner,
        None,
        Some(json!({
            "name": "owner on BRONZE.group_orders",
            "asset": "BRONZE.group_orders",
            "dimension": "completeness",
            "threshold": "rows >= 0",
            "severity": "medium",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // A rule already stored under an upper-case zone is an unknown rule to
    // the owner, and the operator runs it.
    let rule = add_a_rule_on_asset(&pool, "a rule under BRONZE", "BRONZE.retail_orders").await;
    let before = clickhouse_requests(&ch).await;
    let (status, body) = send(
        &router,
        "POST",
        &format!("/api/governance/quality/{rule}/run"),
        &owner,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], json!(RULE_NOT_FOUND), "{body}");
    assert_eq!(
        clickhouse_requests(&ch).await,
        before,
        "a refused run measures nothing"
    );
    let (status, body) = send(
        &router,
        "POST",
        &format!("/api/governance/quality/{rule}/run"),
        &operator,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn the_catalog_owner_cannot_run_a_rule_on_another_tenants_upload_table() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_for_the_quality_writers(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let retail_rule = add_a_rule(&pool, RETAIL_TABLE).await;
    let group_rule = add_a_rule(&pool, GROUP_TABLE).await;
    let owner = cookie_in_tenant(&pool, GOVERNANCE_WRITER, CATALOG_TENANT).await;
    let operator = operator_cookie(&pool).await;

    // A rule on the other tenant's table answers as an unknown rule does, and
    // nothing is measured or recorded.
    let before = clickhouse_requests(&ch).await;
    let (status, body) = send(
        &router,
        "POST",
        &format!("/api/governance/quality/{retail_rule}/run"),
        &owner,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], json!(RULE_NOT_FOUND), "{body}");
    let (status, unknown) = send(
        &router,
        "POST",
        &format!("/api/governance/quality/{}/run", Uuid::new_v4()),
        &owner,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{unknown}");
    assert_eq!(unknown, body, "an unknown rule must look the same");
    assert_eq!(
        clickhouse_requests(&ch).await,
        before,
        "a refused run measures nothing"
    );

    // The owner's own table runs.
    let (status, body) = send(
        &router,
        "POST",
        &format!("/api/governance/quality/{group_rule}/run"),
        &owner,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // An operator runs the other tenant's rule.
    let (status, body) = send(
        &router,
        "POST",
        &format!("/api/governance/quality/{retail_rule}/run"),
        &operator,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn the_catalog_owner_cannot_change_a_rule_on_another_tenants_upload_table() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_for_the_quality_writers(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let retail_rule = add_a_rule(&pool, RETAIL_TABLE).await;
    let group_rule = add_a_rule(&pool, GROUP_TABLE).await;
    let owner = cookie_in_tenant(&pool, GOVERNANCE_WRITER, CATALOG_TENANT).await;
    let operator = operator_cookie(&pool).await;

    // The owner's own rule cannot be moved onto the other tenant's table.
    let (status, body) = send(
        &router,
        "PUT",
        &format!("/api/governance/quality/{group_rule}"),
        &owner,
        None,
        Some(update_rule_body(RETAIL_TABLE, "rows >= 0")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], json!(UNREADABLE_RETAIL), "{body}");
    assert_eq!(rules_on(&pool, RETAIL_TABLE).await, 1, "{body}");
    assert_eq!(rules_on(&pool, GROUP_TABLE).await, 1, "{body}");

    // A rule already on the other tenant's table cannot be edited.
    let (status, body) = send(
        &router,
        "PUT",
        &format!("/api/governance/quality/{retail_rule}"),
        &owner,
        None,
        Some(update_rule_body(RETAIL_TABLE, "rows >= 5")),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], json!(RULE_NOT_FOUND), "{body}");
    let threshold: String = sqlx::query_scalar("SELECT threshold FROM quality_rule WHERE id = $1")
        .bind(retail_rule)
        .fetch_one(&pool)
        .await
        .expect("read the rule");
    assert_eq!(threshold, "rows >= 0", "a refused edit changes nothing");

    // The owner's own rule is edited, and an operator edits the other
    // tenant's rule.
    let (status, body) = send(
        &router,
        "PUT",
        &format!("/api/governance/quality/{group_rule}"),
        &owner,
        None,
        Some(update_rule_body(GROUP_TABLE, "rows >= 1")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = send(
        &router,
        "PUT",
        &format!("/api/governance/quality/{retail_rule}"),
        &operator,
        None,
        Some(update_rule_body(RETAIL_TABLE, "rows >= 5")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn the_catalog_owner_cannot_delete_a_rule_on_another_tenants_upload_table() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_for_the_quality_writers(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let retail_rule = add_a_rule(&pool, RETAIL_TABLE).await;
    let group_rule = add_a_rule(&pool, GROUP_TABLE).await;
    let owner = cookie_in_tenant(&pool, GOVERNANCE_WRITER, CATALOG_TENANT).await;
    let operator = operator_cookie(&pool).await;

    // A rule already on the other tenant's table cannot be deleted.
    let (status, body) = send(
        &router,
        "DELETE",
        &format!("/api/governance/quality/{retail_rule}"),
        &owner,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], json!(RULE_NOT_FOUND), "{body}");
    assert_eq!(
        rules_on(&pool, RETAIL_TABLE).await,
        1,
        "a refused delete keeps the rule"
    );

    let (status, body) = send(
        &router,
        "DELETE",
        &format!("/api/governance/quality/{retail_rule}"),
        &operator,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], json!(true), "{body}");
    assert_eq!(
        rules_on(&pool, RETAIL_TABLE).await,
        0,
        "the operator deleted it"
    );
    let (status, body) = send(
        &router,
        "DELETE",
        &format!("/api/governance/quality/{group_rule}"),
        &owner,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn the_catalog_owner_cannot_set_or_reset_another_tenants_upload_tables_schedule() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_for_the_quality_writers(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    // Both tables carry a schedule of 60 minutes.
    seed_the_quality_rows(&pool).await;
    let owner = cookie_in_tenant(&pool, GOVERNANCE_WRITER, CATALOG_TENANT).await;
    let operator = operator_cookie(&pool).await;
    let retail = format!("/api/governance/quality/schedules/bronze.{RETAIL_TABLE}");
    let group = format!("/api/governance/quality/schedules/bronze.{GROUP_TABLE}");
    let interval = json!({ "mode": "interval", "intervalMinutes": 120 });

    let (status, body) = send(
        &router,
        "PUT",
        &retail,
        &owner,
        None,
        Some(interval.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], json!(UNREADABLE_RETAIL), "{body}");
    let (status, body) = send(&router, "DELETE", &retail, &owner, None, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], json!(UNREADABLE_RETAIL), "{body}");
    let minutes: i32 =
        sqlx::query_scalar("SELECT interval_minutes FROM quality_schedule WHERE asset = $1")
            .bind(format!("bronze.{RETAIL_TABLE}"))
            .fetch_one(&pool)
            .await
            .expect("read the schedule");
    assert_eq!(
        minutes, 60,
        "a refused change leaves the schedule as it was"
    );

    // The owner's own table: set, then reset.
    let (status, body) = send(&router, "PUT", &group, &owner, None, Some(interval.clone())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = send(&router, "DELETE", &group, &owner, None, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // An operator sets the other tenant's table, then resets it.
    let (status, body) = send(&router, "PUT", &retail, &operator, None, Some(interval)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = send(&router, "DELETE", &retail, &operator, None, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

/// The schedules stored under `asset`, under any spelling.
async fn schedules_on(pool: &PgPool, asset: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM quality_schedule WHERE lower(asset) = lower($1)")
        .bind(asset)
        .fetch_one(pool)
        .await
        .expect("count the quality schedules")
}

#[tokio::test]
async fn an_unknown_table_and_another_tenants_table_get_one_schedule_refusal() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_for_the_quality_writers(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, GOVERNANCE_WRITER, CATALOG_TENANT).await;
    let retail = format!("bronze.{RETAIL_TABLE}");
    let set = json!({ "mode": "interval", "intervalMinutes": 120 });

    // A name no table has and a name another tenant claimed: the same 400 for
    // a set and for a reset, so the answer says nothing about upload names.
    for asset in ["bronze.no_such_table", retail.as_str()] {
        let uri = format!("/api/governance/quality/schedules/{asset}");
        let (status, body) = send(&router, "PUT", &uri, &owner, None, Some(set.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{asset}: {body}");
        assert_eq!(body["error"], json!(unreadable_sentence(asset)), "{body}");
        let (status, body) = send(&router, "DELETE", &uri, &owner, None, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{asset}: {body}");
        assert_eq!(body["error"], json!(unreadable_sentence(asset)), "{body}");
    }
    assert_eq!(schedules_on(&pool, "bronze.no_such_table").await, 0);
    assert_eq!(schedules_on(&pool, &retail).await, 0);
}

#[tokio::test]
async fn the_catalog_owner_sets_and_resets_a_schedule_on_its_own_table() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_for_the_quality_writers(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, GOVERNANCE_WRITER, CATALOG_TENANT).await;
    let group = format!("/api/governance/quality/schedules/bronze.{GROUP_TABLE}");

    let (status, body) = send(
        &router,
        "PUT",
        &group,
        &owner,
        None,
        Some(json!({ "mode": "interval", "intervalMinutes": 120 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        schedules_on(&pool, &format!("bronze.{GROUP_TABLE}")).await,
        1
    );
    let (status, body) = send(&router, "DELETE", &group, &owner, None, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(
        schedules_on(&pool, &format!("bronze.{GROUP_TABLE}")).await,
        0
    );
}

/// `SEC-80`: while `ClickHouse` cannot answer, a name another tenant claimed and
/// a name no table has get the same answer, so the outage says nothing about
/// which upload names are claimed.
#[tokio::test]
async fn an_outage_answers_a_claimed_schedule_name_and_an_unknown_one_alike() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    // ClickHouse fails every query, so no table can be checked.
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("ClickHouse is down"))
        .with_priority(1)
        .mount(&ch)
        .await;
    let TestApp { router, pool } = spin_up_with_env(&quality_writers_env(&catalog, &ch)).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, GOVERNANCE_WRITER, CATALOG_TENANT).await;
    let retail = format!("bronze.{RETAIL_TABLE}");
    let set = json!({ "mode": "interval", "intervalMinutes": 120 });

    for verb in ["PUT", "DELETE"] {
        let mut answers = Vec::new();
        for asset in ["bronze.no_such_table", retail.as_str()] {
            let uri = format!("/api/governance/quality/schedules/{asset}");
            let body = (verb == "PUT").then(|| set.clone());
            answers.push(send(&router, verb, &uri, &owner, None, body).await);
        }
        assert_eq!(
            answers[0].0,
            StatusCode::SERVICE_UNAVAILABLE,
            "{verb}: {:?}",
            answers[0].1
        );
        assert_eq!(
            answers[0], answers[1],
            "{verb}: a claimed name and an unknown name answer alike during an outage"
        );
    }
}

/// `SEC-80`: while `ClickHouse` cannot answer, a rule writer gives a table
/// another tenant claimed and a table no one has the same answer, for create
/// and for run.
#[tokio::test]
async fn an_outage_answers_a_claimed_rule_table_and_an_unknown_one_alike() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("ClickHouse is down"))
        .with_priority(1)
        .mount(&ch)
        .await;
    let TestApp { router, pool } = spin_up_with_env(&quality_writers_env(&catalog, &ch)).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, GOVERNANCE_WRITER, CATALOG_TENANT).await;

    // Create: the claimed table and the unknown one answer the same 503.
    let mut creates = Vec::new();
    for table in [RETAIL_TABLE, "no_such_table"] {
        creates.push(
            send(
                &router,
                "POST",
                "/api/governance/quality",
                &owner,
                None,
                Some(rule_body("owner in outage", table, "rows >= 0")),
            )
            .await,
        );
    }
    assert_eq!(
        creates[0].0,
        StatusCode::SERVICE_UNAVAILABLE,
        "{:?}",
        creates[0].1
    );
    assert_eq!(
        creates[0], creates[1],
        "create: a claimed and an unknown table answer alike"
    );
    assert_eq!(
        rules_on(&pool, RETAIL_TABLE).await,
        0,
        "a refused create stores nothing"
    );

    // Run: a rule on a table in the owner's reach is checked, so the outage
    // answers 503. A rule on a claimed table is covered by the test below.
    let unknown_rule = add_a_rule(&pool, "no_such_table").await;
    let (status, body) = send(
        &router,
        "POST",
        &format!("/api/governance/quality/{unknown_rule}/run"),
        &owner,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
}

/// `SEC-80`: a rule the caller cannot reach answers as an unknown rule id
/// does, and during an outage it asks no table at all: a run, an edit and a
/// delete of such a rule give the same `404` as an id that does not exist.
#[tokio::test]
async fn an_outage_answers_a_rule_out_of_reach_like_an_unknown_rule_id() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("ClickHouse is down"))
        .with_priority(1)
        .mount(&ch)
        .await;
    let TestApp { router, pool } = spin_up_with_env(&quality_writers_env(&catalog, &ch)).await;
    claim_the_uploads(&pool).await;
    let owner = cookie_in_tenant(&pool, GOVERNANCE_WRITER, CATALOG_TENANT).await;
    let claimed_rule = add_a_rule(&pool, RETAIL_TABLE).await;
    let unknown_id = Uuid::new_v4();
    let rule_uri = |id: Uuid| format!("/api/governance/quality/{id}");

    let run = |id: Uuid| {
        let (router, owner) = (router.clone(), owner.clone());
        async move {
            send(
                &router,
                "POST",
                &format!("{}/run", rule_uri(id)),
                &owner,
                None,
                None,
            )
            .await
        }
    };
    let (status, unknown) = run(unknown_id).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{unknown}");
    assert_eq!(unknown["error"], json!(RULE_NOT_FOUND), "{unknown}");
    assert_eq!(run(claimed_rule).await, (status, unknown.clone()), "run");

    let update = |id: Uuid| {
        let (router, owner) = (router.clone(), owner.clone());
        async move {
            send(
                &router,
                "PUT",
                &rule_uri(id),
                &owner,
                None,
                Some(update_rule_body(RETAIL_TABLE, "rows >= 5")),
            )
            .await
        }
    };
    assert_eq!(
        update(claimed_rule).await,
        (status, unknown.clone()),
        "update"
    );
    assert_eq!(
        update(unknown_id).await,
        (status, unknown.clone()),
        "update"
    );

    let delete = |id: Uuid| {
        let (router, owner) = (router.clone(), owner.clone());
        async move { send(&router, "DELETE", &rule_uri(id), &owner, None, None).await }
    };
    assert_eq!(
        delete(claimed_rule).await,
        (status, unknown.clone()),
        "delete"
    );
    assert_eq!(delete(unknown_id).await, (status, unknown), "delete");
    assert_eq!(
        rules_on(&pool, RETAIL_TABLE).await,
        1,
        "a refused delete keeps the rule"
    );
}

#[tokio::test]
async fn a_schedule_on_a_dropped_table_can_be_reset_but_not_set_again() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_for_the_quality_writers(&catalog, &ch).await;
    // The table was dropped after it was scheduled: no mock answers its
    // `DESCRIBE`, so it is absent, and the schedule row is still there.
    sqlx::query(
        "INSERT INTO quality_schedule (asset, mode, interval_minutes) VALUES ($1, 'interval', 60)",
    )
    .bind("bronze.dropped_orders")
    .execute(&pool)
    .await
    .expect("add the schedule");
    let owner = cookie_in_tenant(&pool, GOVERNANCE_WRITER, CATALOG_TENANT).await;
    let uri = "/api/governance/quality/schedules/bronze.dropped_orders";

    let (status, body) = send(
        &router,
        "PUT",
        uri,
        &owner,
        None,
        Some(json!({ "mode": "interval", "intervalMinutes": 120 })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["error"],
        json!(unreadable_sentence("bronze.dropped_orders")),
        "{body}"
    );
    let (status, body) = send(&router, "DELETE", uri, &owner, None, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(schedules_on(&pool, "bronze.dropped_orders").await, 0);
}

#[tokio::test]
async fn the_catalog_owner_cannot_resolve_an_incident_on_another_tenants_upload_table() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_for_the_quality_writers(&catalog, &ch).await;
    claim_the_uploads(&pool).await;
    seed_the_quality_rows(&pool).await;
    let owner = cookie_in_tenant(&pool, GOVERNANCE_WRITER, CATALOG_TENANT).await;
    let operator = operator_cookie(&pool).await;
    let incident_on = |table: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, Uuid>("SELECT id FROM quality_incident WHERE asset = $1")
                .bind(format!("bronze.{table}"))
                .fetch_one(&pool)
                .await
                .expect("find the incident")
        }
    };
    let retail_incident = incident_on(RETAIL_TABLE).await;
    let group_incident = incident_on(GROUP_TABLE).await;
    let resolve = |id: Uuid| format!("/api/governance/quality/incidents/{id}/resolve");
    let body = json!({ "resolution": "checked by hand" });

    // An incident on the other tenant's table answers as an unknown one, and
    // stays open.
    let (status, reply) = send(
        &router,
        "POST",
        &resolve(retail_incident),
        &owner,
        None,
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{reply}");
    assert_eq!(reply["error"], json!(INCIDENT_NOT_FOUND), "{reply}");
    let status_now: String =
        sqlx::query_scalar("SELECT status FROM quality_incident WHERE id = $1")
            .bind(retail_incident)
            .fetch_one(&pool)
            .await
            .expect("read the incident");
    assert_eq!(status_now, "open", "a refused resolve changes nothing");

    // The owner resolves its own; an operator resolves the other tenant's.
    let (status, reply) = send(
        &router,
        "POST",
        &resolve(group_incident),
        &owner,
        None,
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    let (status, reply) = send(
        &router,
        "POST",
        &resolve(retail_incident),
        &operator,
        None,
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reply}");
}

#[tokio::test]
async fn the_agent_run_detail_is_read_by_the_catalog_owner_and_by_nobody_else() {
    let catalog = MockServer::start().await;
    mount_catalog(&catalog).await;
    let ch = MockServer::start().await;
    mount_empty_clickhouse(&ch).await;
    let TestApp { router, pool } = spin_up_with_catalog(&catalog, &ch).await;
    let run_id = seed_an_agent_run(&pool).await;
    let owner = cookie_in_tenant(&pool, AGENTS_READER, CATALOG_TENANT).await;
    let other = cookie_in_tenant(&pool, AGENTS_READER, OTHER_TENANT).await;
    let operator = operator_cookie(&pool).await;

    // WEB-4 retired the runs LIST; only `GET /runs/{id}` stays (an approval
    // links to one run's trace). A tenant that does not own the shared
    // catalog gets the unknown 404, so the answer says nothing about
    // whether the run exists.
    let (status, body) = get(&router, &format!("/api/agents/runs/{run_id}"), &other, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        body["error"],
        json!(format!("Run {run_id} not found")),
        "{body}"
    );

    // The tenant that owns the shared catalog reads the run.
    let (status, body) = get(&router, &format!("/api/agents/runs/{run_id}"), &owner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["id"], json!(run_id), "{body}");

    // An operator reaches it too.
    let (status, body) = get(
        &router,
        &format!("/api/agents/runs/{run_id}"),
        &operator,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["id"], json!(run_id), "{body}");
}

/// The ids of the events a page of the trail holds.
fn event_ids(page: &Value) -> Vec<String> {
    page["events"]
        .as_array()
        .unwrap_or_else(|| panic!("an events array: {page}"))
        .iter()
        .map(|event| event["id"].as_str().expect("an event id").to_owned())
        .collect()
}

/// The ids of the rows the old governance list holds.
fn governance_ids(body: &Value) -> Vec<String> {
    body["audit"]
        .as_array()
        .unwrap_or_else(|| panic!("an audit array: {body}"))
        .iter()
        .map(|row| row["id"].as_str().expect("an audit row id").to_owned())
        .collect()
}

/// The ids of the entries the Overview feed holds.
fn feed_ids(body: &Value) -> Vec<String> {
    body["activity"]
        .as_array()
        .unwrap_or_else(|| panic!("an activity array: {body}"))
        .iter()
        .map(|entry| entry["id"].as_str().expect("an activity id").to_owned())
        .collect()
}

/// The CSV the export route writes, as text.
async fn export_csv(router: &axum::Router, cookie: &str) -> String {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/governance/audit/export.csv")
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read the file");
    String::from_utf8_lossy(&bytes).into_owned()
}

/// An auditor of the tenant that owns the shared catalog sees its own
/// organisation's rows and the deployment's own rows, and not another
/// organisation's — on the events route, on the CSV export and in the
/// assistant's audit history.
#[tokio::test]
async fn an_auditor_of_the_catalog_owners_organisation_sees_its_own_rows_and_the_deployments_rows()
{
    let TestApp { router, pool } = spin_up_for_the_audit_trail().await;
    let (group, group_cookie) = principal_in_tenant(&pool, AUDIT_READER, CATALOG_TENANT).await;
    let (retail, _retail_cookie) = principal_in_tenant(&pool, AUDIT_READER, OTHER_TENANT).await;
    seed_the_trail(&pool, group, retail).await;

    // The events route.
    let (status, page) = get(&router, "/api/governance/audit/events", &group_cookie, None).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(
        event_ids(&page),
        ["trail-of-the-deployment", "trail-of-group"],
        "the owner's own rows and the deployment's, newest first"
    );

    // The CSV export. The export writes a row of its own before it streams
    // the file, so the file holds that row too: what it must not hold is
    // another organisation's.
    let csv = export_csv(&router, &group_cookie).await;
    assert!(csv.contains("row-of-tenant-a"), "{csv}");
    assert!(csv.contains("row-of-the-deployment"), "{csv}");
    assert!(!csv.contains("row-of-tenant-b"), "{csv}");

    // The assistant's audit history.
    let history = call_tool(&router, &group_cookie, "get_audit_history", json!({})).await;
    assert_eq!(history["outcome"], json!("executed"), "{history}");
    let ids = governance_ids(&history["result"]);
    assert!(
        ids.contains(&String::from("trail-of-group")),
        "the owner's own row: {ids:?}"
    );
    assert!(
        ids.contains(&String::from("trail-of-the-deployment")),
        "the deployment's own row: {ids:?}"
    );
    assert!(
        !ids.contains(&String::from("trail-of-retail")),
        "no other organisation's row: {ids:?}"
    );
}

/// An auditor of another organisation sees its own organisation's rows and
/// nothing else: not the deployment's own rows, and not the catalog owner's.
#[tokio::test]
async fn an_auditor_of_another_organisation_sees_only_its_own_rows() {
    let TestApp { router, pool } = spin_up_for_the_audit_trail().await;
    let (group, _group_cookie) = principal_in_tenant(&pool, AUDIT_READER, CATALOG_TENANT).await;
    let (retail, retail_cookie) = principal_in_tenant(&pool, AUDIT_READER, OTHER_TENANT).await;
    seed_the_trail(&pool, group, retail).await;

    let (status, page) = get(
        &router,
        "/api/governance/audit/events",
        &retail_cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(event_ids(&page), ["trail-of-retail"]);

    let csv = export_csv(&router, &retail_cookie).await;
    assert!(csv.contains("row-of-tenant-b"), "{csv}");
    assert!(!csv.contains("row-of-tenant-a"), "{csv}");
    assert!(!csv.contains("row-of-the-deployment"), "{csv}");

    let history = call_tool(&router, &retail_cookie, "get_audit_history", json!({})).await;
    assert_eq!(history["outcome"], json!("executed"), "{history}");
    let ids = governance_ids(&history["result"]);
    assert!(
        ids.contains(&String::from("trail-of-retail")),
        "its own row: {ids:?}"
    );
    assert!(
        !ids.contains(&String::from("trail-of-group")),
        "no other organisation's row: {ids:?}"
    );
    assert!(
        !ids.contains(&String::from("trail-of-the-deployment")),
        "the deployment's own row is not this organisation's: {ids:?}"
    );
}

/// An unrestricted caller (`*:*`) sees every row of the trail: it is not
/// a member of any organisation, and its reach is everything.
#[tokio::test]
async fn an_unrestricted_caller_sees_every_row_of_the_trail() {
    let TestApp { router, pool } = spin_up_for_the_audit_trail().await;
    let (group, _group_cookie) = principal_in_tenant(&pool, AUDIT_READER, CATALOG_TENANT).await;
    let (retail, _retail_cookie) = principal_in_tenant(&pool, AUDIT_READER, OTHER_TENANT).await;
    seed_the_trail(&pool, group, retail).await;
    let operator = operator_cookie(&pool).await;

    let (status, page) = get(&router, "/api/governance/audit/events", &operator, None).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(
        event_ids(&page),
        [
            "trail-of-the-deployment",
            "trail-of-retail",
            "trail-of-group"
        ]
    );

    let csv = export_csv(&router, &operator).await;
    for row in [
        "row-of-tenant-a",
        "row-of-tenant-b",
        "row-of-the-deployment",
    ] {
        assert!(csv.contains(row), "{row} must be in the file: {csv}");
    }
}

/// The Overview feed is read per tenant as well: it holds the rows of the
/// caller's own organisation, and the deployment's own rows only for the
/// tenant that owns the shared catalog.
#[tokio::test]
async fn the_overview_feed_holds_only_the_callers_own_organisations_rows() {
    let TestApp { router, pool } = spin_up_for_the_audit_trail().await;
    let (group, group_cookie) = principal_in_tenant(&pool, AUDIT_READER, CATALOG_TENANT).await;
    let (retail, retail_cookie) = principal_in_tenant(&pool, AUDIT_READER, OTHER_TENANT).await;
    seed_the_trail(&pool, group, retail).await;

    let (status, feed) = send(&router, "POST", "/api/overview", &group_cookie, None, None).await;
    assert_eq!(status, StatusCode::OK, "{feed}");
    assert_eq!(
        feed_ids(&feed),
        ["trail-of-the-deployment", "trail-of-group"],
        "the owner's own rows and the deployment's"
    );

    let (status, feed) = send(&router, "POST", "/api/overview", &retail_cookie, None, None).await;
    assert_eq!(status, StatusCode::OK, "{feed}");
    assert_eq!(feed_ids(&feed), ["trail-of-retail"]);
}
