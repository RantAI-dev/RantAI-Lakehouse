//! `GET /api/governance/lineage/columns` (`DATA-13`), through the real router
//! against a per-test Postgres and a `wiremock` stand-in for `ClickHouse`
//! (the dashboard charts and boards live there).
//!
//! What these pin: the walk and its hops, the one-year retention line, the
//! tenant rules (another tenant's rows never appear; shared rows only where
//! the shared catalog may be read), that charts and SQL-source rows need
//! `dashboard:read`, that a bad parameter is a 400 with our sentence, and
//! that the table graph's `columnMappings` carries the same rows.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::column_lineage::{self, NewMapping};
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

/// `meridian-retail`, the one tenant the seeded Analyst belongs to. Naming
/// it as the catalog tenant is what lets that Analyst read shared records
/// (`catalog_tenant_refusal`: the deployment has four tenants).
const RETAIL: &str = "11111111-1111-4111-8111-000000000002";

fn map(from: (&str, &str), to: (&str, &str), transform: &str) -> NewMapping {
    NewMapping {
        from_table: Some(from.0.to_owned()),
        from_column: Some(from.1.to_owned()),
        to_table: to.0.to_owned(),
        to_column: to.1.to_owned(),
        transform: transform.to_owned(),
        partial: false,
        reason: None,
    }
}

fn chart_row(id: &str, board: &str, def: &Value) -> Value {
    let spec_json = json!({
        "spec": {
            "id": id, "title": "Visitors by place", "kind": "bar", "mart": "mart_x",
            "sql": "SELECT 1", "x": "place", "y": "visitors",
        },
        "def": def,
        "hasYear": false,
    })
    .to_string();
    json!({ "id": id, "spec_json": spec_json, "board": board, "created_by": "ui", "created_at": "" })
}

async fn app(charts: Vec<Value>) -> (axum::Router, PgPool, MockServer) {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("console.bi_chart FINAL"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "rows": charts.len(), "data": charts,
        })))
        .with_priority(1)
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("console.bi_board FINAL"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "rows": 1, "data": [{ "id": "b1", "name": "Tourism" }],
        })))
        .with_priority(1)
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("database IN ('silver', 'serving')"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "rows": 1,
            "data": [{ "database": "serving", "name": "m", "engine": "MergeTree", "as_select": "" }],
        })))
        .with_priority(1)
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "data": [], "rows": 0,
        })))
        .mount(&ch)
        .await;
    let mut overrides = std::collections::HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch.uri());
    overrides.insert("CATALOG_TENANT_ID".to_owned(), RETAIL.to_owned());
    let TestApp { router, pool } = spin_up_with_env(&overrides).await;
    (router, pool, ch)
}

async fn get(router: &axum::Router, cookie: &str, uri: &str) -> (StatusCode, Value) {
    get_as_tenant(router, cookie, uri, None).await
}

/// `get` acting as `tenant` through `X-Tenant` (`tenant_scope::resolve`).
async fn get_as_tenant(
    router: &axum::Router,
    cookie: &str,
    uri: &str,
    tenant: Option<&str>,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method("GET")
        .uri(uri)
        .header("cookie", cookie);
    if let Some(tenant) = tenant {
        req = req.header("x-tenant", tenant);
    }
    let resp = router
        .clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// Gold `serving.m.total` is built from `silver.a.x1`, which a pipeline
/// copies from `bronze.a.x`.
async fn seed_chain(pool: &PgPool, pipeline_tenant: Option<Uuid>) {
    column_lineage::replace_producer(
        pool,
        "pipeline",
        "p1",
        pipeline_tenant,
        &[map(("bronze.a", "x"), ("silver.a", "x1"), "copy")],
    )
    .await
    .unwrap();
    column_lineage::replace_producer(
        pool,
        "build",
        "serving.m",
        None,
        &[map(
            ("silver.a", "x1"),
            ("serving.m", "total"),
            "expression",
        )],
    )
    .await
    .unwrap();
}

const UP: &str = "/api/governance/lineage/columns?table=serving.m&column=total&direction=up";

#[tokio::test]
async fn an_analyst_reads_a_column_back_to_bronze_with_hops_and_the_retention_line() {
    let (router, pool, _ch) = app(Vec::new()).await;
    seed_chain(&pool, Some(RETAIL.parse().unwrap())).await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let (status, body) = get(&router, &cookie, UP).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = body["mappings"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{body}");
    assert_eq!(
        rows[0]["from"],
        json!({ "table": "silver.a", "column": "x1" })
    );
    assert_eq!(
        rows[0]["to"],
        json!({ "table": "serving.m", "column": "total" })
    );
    assert_eq!(rows[0]["transform"], "expression");
    assert_eq!(rows[0]["hop"], 1);
    assert_eq!(rows[0]["partial"], false);
    assert_eq!(
        rows[0]["producer"],
        json!({ "kind": "build", "ref": "serving.m" })
    );
    assert_eq!(
        rows[1]["from"],
        json!({ "table": "bronze.a", "column": "x" })
    );
    assert_eq!(rows[1]["hop"], 2);
    assert_eq!(rows[1]["producer"]["kind"], "pipeline");
    assert_eq!(body["retention"]["days"], 365);
    assert!(body["retention"]["oldestRecordedAt"].is_string(), "{body}");
    assert_eq!(body["direction"], "up");
}

#[tokio::test]
async fn another_tenants_pipeline_rows_are_not_shown() {
    let (router, pool, _ch) = app(Vec::new()).await;
    // The pipeline belongs to a tenant the Analyst is not in.
    seed_chain(&pool, Some(Uuid::new_v4())).await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let (status, body) = get(&router, &cookie, UP).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = body["mappings"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "only the shared build row: {body}");
    assert_eq!(rows[0]["producer"]["kind"], "build");
}

#[tokio::test]
async fn a_caller_in_no_tenant_sees_neither_shared_nor_tenant_rows_and_is_told_why() {
    let (router, pool, _ch) = app(Vec::new()).await;
    seed_chain(&pool, Some(RETAIL.parse().unwrap())).await;
    // `lineage:read` and nothing else, and no tenant: outside the catalog
    // tenant, so the shared build row is refused (`catalog_tenant_refusal`),
    // and with no tenant the pipeline's row is not theirs either.
    let user = create_principal_with_permissions(&pool, "lineage:read").await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, body) = get(&router, &cookie, UP).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["mappings"], json!([]), "{body}");
    assert_eq!(body["coverage"].as_array().unwrap().len(), 2, "{body}");
}

#[tokio::test]
async fn a_caller_without_dashboard_access_gets_no_charts_and_no_sql_source_rows() {
    let (router, pool, _ch) = app(Vec::new()).await;
    seed_chain(&pool, Some(RETAIL.parse().unwrap())).await;
    column_lineage::replace_producer(
        &pool,
        "sql_source",
        "s1",
        None,
        &[map(("serving.m", "total"), ("sql_source:s1", "t"), "copy")],
    )
    .await
    .unwrap();
    let analyst = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;
    let admin = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let down = "/api/governance/lineage/columns?table=bronze.a&column=x&direction=down";

    let (status, body) = get(&router, &analyst, down).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["chartsHidden"], true, "{body}");
    assert!(body.get("charts").is_none(), "{body}");
    let tables: Vec<&str> = body["mappings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["to"]["table"].as_str().unwrap())
        .collect();
    assert_eq!(tables, vec!["silver.a", "serving.m"], "{body}");

    // DATA-13 CI fix: the seeded admin belongs to three tenants and, with no
    // `X-Tenant`, acts as the first of them (`tenant_scope::resolve`), which
    // is not `meridian-retail`; like the table route, a tenant-member admin
    // reads that one tenant's rows, so the pipeline row is hidden. Naming the
    // pipeline's tenant is what this half of the test meant to do.
    let (_, body) = get_as_tenant(&router, &admin, down, Some(RETAIL)).await;
    let tables: Vec<&str> = body["mappings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["to"]["table"].as_str().unwrap())
        .collect();
    assert_eq!(
        tables,
        vec!["silver.a", "serving.m", "sql_source:s1"],
        "{body}"
    );
    assert!(body.get("chartsHidden").is_none(), "{body}");

    // An unrestricted caller who belongs to no tenant reads every tenant's
    // rows (`all_tenants`), as the table route does for the same principal.
    let root = create_principal_with_permissions(&pool, "*:*").await;
    let root_cookie = session_cookie_for_user(&pool, root).await;
    let (_, body) = get(&router, &root_cookie, down).await;
    let tables: Vec<&str> = body["mappings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["to"]["table"].as_str().unwrap())
        .collect();
    assert_eq!(
        tables,
        vec!["silver.a", "serving.m", "sql_source:s1"],
        "{body}"
    );
}

#[tokio::test]
async fn a_downstream_walk_lists_each_chart_that_reads_a_reached_column_with_its_dashboard() {
    let charts = vec![
        // Reads the SQL source's column `t`, which is reached.
        chart_row(
            "c-source",
            "b1",
            &json!({
                "title": "T", "kind": "bar", "sqlSource": "s1", "dimension": "t",
                "measures": [],
            }),
        ),
        // Reads the mart column `total` directly.
        chart_row(
            "c-mart",
            "b1",
            &json!({
                "title": "T", "kind": "bar", "mart": "m", "dimension": "total", "measures": [],
            }),
        ),
        // Names a column nothing reaches.
        chart_row(
            "c-other",
            "b1",
            &json!({
                "title": "T", "kind": "bar", "mart": "m", "dimension": "place", "measures": [],
            }),
        ),
        // Right column name, wrong relation.
        chart_row(
            "c-elsewhere",
            "b1",
            &json!({
                "title": "T", "kind": "bar", "mart": "other", "dimension": "total",
                "measures": [],
            }),
        ),
    ];
    let (router, pool, _ch) = app(charts).await;
    seed_chain(&pool, None).await;
    column_lineage::replace_producer(
        &pool,
        "sql_source",
        "s1",
        None,
        &[map(("serving.m", "total"), ("sql_source:s1", "t"), "copy")],
    )
    .await
    .unwrap();
    let admin = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let (status, body) = get(
        &router,
        &admin,
        "/api/governance/lineage/columns?table=bronze.a&column=x&direction=down",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut ids: Vec<&str> = body["charts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["chartId"].as_str().unwrap())
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec!["c-mart", "c-source"], "{body}");
    assert_eq!(body["charts"][0]["dashboardId"], "b1");
    assert_eq!(body["charts"][0]["dashboardTitle"], "Tourism");
    assert_eq!(body["charts"][0]["chartTitle"], "Visitors by place");
}

#[tokio::test]
async fn a_bad_parameter_is_a_400_with_our_sentence() {
    let (router, pool, _ch) = app(Vec::new()).await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;
    for (uri, needle) in [
        (
            "/api/governance/lineage/columns?column=x",
            "table is required",
        ),
        (
            "/api/governance/lineage/columns?table=serving.m",
            "column is required",
        ),
        (
            "/api/governance/lineage/columns?table=serving.m&column=x&direction=sideways",
            "direction must be up or down",
        ),
    ] {
        let (status, body) = get(&router, &cookie, uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
        assert!(
            body["error"].as_str().is_some_and(|e| e.contains(needle)),
            "{uri}: {body}"
        );
    }
}

#[tokio::test]
async fn a_column_nothing_was_recorded_for_answers_an_empty_list_not_an_error() {
    let (router, pool, _ch) = app(Vec::new()).await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;
    let (status, body) = get(&router, &cookie, UP).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["mappings"], json!([]));
    assert_eq!(body["retention"]["oldestRecordedAt"], Value::Null);
}

#[tokio::test]
async fn the_table_graph_carries_the_mappings_that_touch_its_focus() {
    let (router, pool, _ch) = app(Vec::new()).await;
    seed_chain(&pool, None).await;
    let admin = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let (status, body) = get(&router, &admin, "/api/governance/lineage?focus=serving.m").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = body["columnMappings"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{body}");
    assert_eq!(
        rows[0]["from"],
        json!({ "table": "silver.a", "column": "x1" })
    );
    assert_eq!(
        rows[0]["to"],
        json!({ "table": "serving.m", "column": "total" })
    );

    // A focus the graph does not know keeps the empty list.
    let (_, body) = get(&router, &admin, "/api/governance/lineage?focus=nothing").await;
    assert_eq!(body["columnMappings"], json!([]));
}
