//! `BI-44` slice 3: assistant tools and unsaved dashboard SQL-source preview
//! read an external table through the ONE engine a `bi-only` install started.
//!
//! The harness gives these tests a wiremock `Trino` and an immediately
//! refusing `ClickHouse` (`127.0.0.1:1`). A path that dialed the uninstalled
//! engine could not answer with governed rows: success, plus the exact
//! statements the `Trino` mock received, is the proof that no `ClickHouse`
//! call was made. `EXPLAIN AST` (the assistant's `ClickHouse` dry run) would
//! also be visible in those statements, so its absence is asserted too.
//!
//! `full` mode's assistant and preview tests are unchanged; this file adds
//! only the one-engine branch.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{TestApp, session_cookie_for_user, spin_up_with_env};
use lakehouse_store::governance::CreatePolicyInput;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A `bi-only` Trino deployment: only `TRINO_URL` names a live upstream;
/// `CH_URL` is explicitly empty, so an accidental `ClickHouse` dial fails.
fn trino_env(trino_uri: &str) -> HashMap<String, String> {
    HashMap::from([
        ("LAKEHOUSE_INSTALL_MODE".to_owned(), "bi-only".to_owned()),
        ("LAKEHOUSE_QUERY_ENGINE".to_owned(), "trino".to_owned()),
        ("TRINO_URL".to_owned(), trino_uri.to_owned()),
        ("CH_URL".to_owned(), String::new()),
    ])
}

fn trino_page(columns: &[&str], rows: &[Vec<&str>]) -> Value {
    json!({
        "id": "q",
        "columns": columns.iter().map(|n| json!({"name": n, "type": "varchar"})).collect::<Vec<_>>(),
        "data": rows,
    })
}

/// `priority(1)` makes a narrow matcher win over the general `SELECT` one
/// when a statement carries both phrases (the column probe does).
async fn mount_trino(server: &MockServer, contains: &str, body: Value, priority: u8) {
    Mock::given(method("POST"))
        .and(path("/v1/statement"))
        .and(body_string_contains(contains))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .with_priority(priority)
        .mount(server)
        .await;
}

async fn mount_catalog(server: &MockServer) {
    mount_trino(
        server,
        "SHOW CATALOGS",
        trino_page(&["Catalog"], &[vec!["system"], vec!["erp_pg"]]),
        5,
    )
    .await;
    mount_trino(
        server,
        "SHOW SCHEMAS FROM \"erp_pg\"",
        trino_page(&["Schema"], &[vec!["information_schema"], vec!["public"]]),
        5,
    )
    .await;
    mount_trino(
        server,
        "SHOW TABLES FROM \"erp_pg\".\"public\"",
        trino_page(&["Table"], &[vec!["customers"]]),
        5,
    )
    .await;
}

/// The one metadata read the governance rewrite needs to resolve the real
/// columns of the 3-part external table.
async fn mount_describe(server: &MockServer) {
    mount_trino(
        server,
        "DESCRIBE",
        trino_page(
            &["Column", "Type", "Extra", "Comment"],
            &[
                vec!["id", "bigint", "", ""],
                vec!["email", "varchar", "", ""],
                vec!["tenant_id", "varchar", "", ""],
            ],
        ),
        1,
    )
    .await;
}

async fn statements(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .expect("the Trino mock records requests")
        .iter()
        .map(|request| String::from_utf8_lossy(&request.body).into_owned())
        .collect()
}

/// A role with a KNOWN name, because policy conditions match role names.
/// `*:*` keeps the tenant gate unrestricted; the route's own permission
/// checks are still satisfied by the same permission set.
async fn create_named_reader(pool: &lakehouse_store::PgPool, role: &str) -> Uuid {
    let role_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO role (id, name, permissions, description) VALUES ($1, $2, '*:*', 'test: in-place BI assistant reader')",
    )
    .bind(role_id)
    .bind(role)
    .execute(pool)
    .await
    .expect("insert the named reader role");

    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO app_user (id, name, username, email, status) VALUES ($1, 'BI Assistant Reader', $2, $3, 'active')",
    )
    .bind(user_id)
    .bind(format!("bi-assistant-{user_id}"))
    .bind(format!("bi-assistant-{user_id}@test.invalid"))
    .execute(pool)
    .await
    .expect("insert the reader");
    sqlx::query("INSERT INTO app_user_role (user_id, role_id) VALUES ($1, $2)")
        .bind(user_id)
        .bind(role_id)
        .execute(pool)
        .await
        .expect("attach the reader role");
    user_id
}

async fn seed_external_mask(pool: &lakehouse_store::PgPool, role: &str) {
    lakehouse_store::governance::create_policy(
        pool,
        &CreatePolicyInput {
            name: "bi-assistant-external-mask".to_owned(),
            kind: "Row filter".to_owned(),
            subjects: role.to_owned(),
            resources: "erp_pg.public.customers".to_owned(),
            effect: "Permit with obligation".to_owned(),
            conditions: Some(
                json!({
                    "roles": [role],
                    "table": "erp_pg.public.customers",
                    "mask": ["email"],
                    "rowFilter": "tenant_id = 'tenant-a'",
                })
                .to_string(),
            ),
            activate: true,
            owner: None,
        },
    )
    .await
    .expect("seed the governing external policy");
}

/// A saved query owned by `user`. The store has no engine column, which is
/// why `run_saved_query` takes the installation's engine in `bi-only` mode.
async fn save_query_for(pool: &lakehouse_store::PgPool, user: Uuid) -> String {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO saved_query (title, sql, owner, owner_id, tenant_id) \
         VALUES ('In-place customers', 'SELECT id, email FROM erp_pg.public.customers', \
                 'BI Assistant Reader', $1, NULL) RETURNING id",
    )
    .bind(user)
    .fetch_one(pool)
    .await
    .expect("insert the saved query");
    id.to_string()
}

async fn post_json(
    app: &axum::Router,
    route: &str,
    cookie: &str,
    body: Value,
) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(route)
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn get_json(app: &axum::Router, route: &str, cookie: &str) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(route)
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn post_tool(
    app: &axum::Router,
    cookie: &str,
    tool: &str,
    args: Value,
) -> (StatusCode, Value) {
    post_json(
        app,
        "/api/ai/tool",
        cookie,
        json!({ "tool": tool, "args": args }),
    )
    .await
}

/// `list_datasets` and `describe_dataset` are answered by wiremock Trino's
/// own catalog in `bi-only+trino`. The response keeps the assistant's shape
/// (slug, sourceKind, columns) and marks in-place tables honestly; the
/// `ClickHouse` registry is not reachable in this deployment.
#[tokio::test]
async fn a_bi_only_trino_assistant_lists_and_describes_external_tables_from_trino_only() {
    let server = MockServer::start().await;
    mount_catalog(&server).await;
    mount_describe(&server).await;

    let TestApp { router, pool } = spin_up_with_env(&trino_env(&server.uri())).await;
    let user = create_named_reader(&pool, "BI Catalog Reader").await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, listed) = post_tool(&router, &cookie, "list_datasets", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["outcome"], json!("executed"), "{listed}");
    let datasets = listed["result"]["datasets"]
        .as_array()
        .expect("a dataset array");
    assert_eq!(datasets.len(), 1, "{listed}");
    assert_eq!(datasets[0]["fullName"], json!("erp_pg.public.customers"));
    assert_eq!(datasets[0]["zone"], json!("external"));
    assert_eq!(datasets[0]["engine"], json!("trino"));
    assert!(datasets[0]["goldTable"].is_null(), "{listed}");
    assert!(datasets[0]["rows"].is_null(), "{listed}");

    let (status, described) = post_tool(
        &router,
        &cookie,
        "describe_dataset",
        json!({ "slug": "erp_pg.public.customers" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{described}");
    assert_eq!(described["outcome"], json!("executed"), "{described}");
    assert_eq!(
        described["result"]["columns"][0]["name"],
        json!("id"),
        "{described}"
    );
    assert_eq!(
        described["result"]["columns"][1]["type"],
        json!("varchar"),
        "{described}"
    );
    assert_eq!(described["result"]["zone"], json!("external"));

    let sent = statements(&server).await;
    for phrase in [
        "SHOW CATALOGS",
        "SHOW SCHEMAS FROM \"erp_pg\"",
        "SHOW TABLES FROM \"erp_pg\".\"public\"",
        "DESCRIBE \"erp_pg\".\"public\".\"customers\"",
    ] {
        assert!(
            sent.iter().any(|s| s.contains(phrase)),
            "{phrase} not in {sent:?}"
        );
    }
    assert!(
        sent.iter().all(|s| !s.contains("EXPLAIN AST")),
        "the assistant must not dry-run on ClickHouse: {sent:?}"
    );
}

/// `run_sql` on an external 3-part table uses `Engine::Trino` governance:
/// the mask and the row filter reach Trino with the full quoted identifier,
/// and the `ClickHouse` `EXPLAIN AST` dry run is gone.
#[tokio::test]
async fn a_bi_only_trino_run_sql_masks_and_filters_on_the_active_engine() {
    let server = MockServer::start().await;
    mount_describe(&server).await;
    mount_trino(
        &server,
        "SELECT",
        json!({
            "id": "q",
            "columns": [
                {"name": "id", "type": "bigint"},
                {"name": "email", "type": "varchar"}
            ],
            "data": [[1, "***"]],
        }),
        5,
    )
    .await;

    let TestApp { router, pool } = spin_up_with_env(&trino_env(&server.uri())).await;
    let user = create_named_reader(&pool, "BI SQL Reader").await;
    seed_external_mask(&pool, "BI SQL Reader").await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, answer) = post_tool(
        &router,
        &cookie,
        "run_sql",
        json!({ "sql": "SELECT id, email FROM erp_pg.public.customers" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["outcome"], json!("executed"), "{answer}");
    assert_eq!(
        answer["result"]["rows"][0]["email"],
        json!("***"),
        "{answer}"
    );

    let sent = statements(&server).await;
    let governed = sent
        .iter()
        .find(|s| s.contains("FROM (SELECT"))
        .unwrap_or_else(|| panic!("no governed statement reached Trino: {sent:?}"));
    assert!(
        governed.contains("\"erp_pg\".\"public\".\"customers\""),
        "{governed}"
    );
    assert!(
        governed.contains("CASE WHEN \"email\" IS NULL THEN NULL ELSE '***' END AS \"email\""),
        "{governed}"
    );
    assert!(governed.contains("tenant_id = 'tenant-a'"), "{governed}");
    assert!(
        sent.iter().all(|s| !s.contains("EXPLAIN AST")),
        "ClickHouse was not deployed: {sent:?}"
    );
}

/// A saved query has no engine column of its own, so in `bi-only+trino`
/// `run_saved_query` runs it on the installation's engine with the same
/// governance rewrite and no `ClickHouse` dial.
#[tokio::test]
async fn a_bi_only_trino_saved_query_runs_on_the_active_engine() {
    let server = MockServer::start().await;
    mount_describe(&server).await;
    mount_trino(
        &server,
        "SELECT",
        json!({
            "id": "q",
            "columns": [
                {"name": "id", "type": "bigint"},
                {"name": "email", "type": "varchar"}
            ],
            "data": [[7, "***"]],
        }),
        5,
    )
    .await;

    let TestApp { router, pool } = spin_up_with_env(&trino_env(&server.uri())).await;
    let user = create_named_reader(&pool, "BI Saved Query Reader").await;
    seed_external_mask(&pool, "BI Saved Query Reader").await;
    let id = save_query_for(&pool, user).await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, answer) =
        post_tool(&router, &cookie, "run_saved_query", json!({ "id": id })).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["outcome"], json!("executed"), "{answer}");
    assert_eq!(
        answer["result"]["rows"][0]["email"],
        json!("***"),
        "{answer}"
    );

    let sent = statements(&server).await;
    let governed = sent
        .iter()
        .find(|s| s.contains("FROM (SELECT"))
        .unwrap_or_else(|| panic!("no governed saved-query run reached Trino: {sent:?}"));
    assert!(
        governed.contains("\"erp_pg\".\"public\".\"customers\""),
        "{governed}"
    );
    assert!(
        sent.iter().all(|s| !s.contains("EXPLAIN AST")),
        "ClickHouse was not deployed: {sent:?}"
    );
}

/// `POST /api/dashboard/sources/preview` over unsaved SQL validates with the
/// active engine's guard, applies the same governance rewrite, probes columns
/// and reads rows on Trino. No `SETTINGS`/`FORMAT` text and no `ClickHouse`
/// dial is part of that path. Saved sources are refused honestly (503)
/// because their store lives in the uninstalled engine.
#[tokio::test]
async fn a_bi_only_trino_source_preview_runs_on_trino_with_masking() {
    let server = MockServer::start().await;
    mount_describe(&server).await;
    mount_trino(&server, "LIMIT 0", trino_page(&["id", "email"], &[]), 1).await;
    mount_trino(
        &server,
        "SELECT",
        json!({
            "id": "q",
            "columns": [
                {"name": "id", "type": "bigint"},
                {"name": "email", "type": "varchar"}
            ],
            "data": [[1, "***"]],
        }),
        5,
    )
    .await;

    let TestApp { router, pool } = spin_up_with_env(&trino_env(&server.uri())).await;
    let user = create_named_reader(&pool, "BI Preview Reader").await;
    seed_external_mask(&pool, "BI Preview Reader").await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, preview) = post_json(
        &router,
        "/api/dashboard/sources/preview",
        &cookie,
        json!({ "sql": "SELECT id, email FROM erp_pg.public.customers" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["columns"][0]["name"], json!("id"), "{preview}");
    assert_eq!(preview["rows"][0]["email"], json!("***"), "{preview}");

    let sent = statements(&server).await;
    let governed = sent
        .iter()
        .find(|s| s.contains("FROM (SELECT"))
        .unwrap_or_else(|| panic!("no governed preview reached Trino: {sent:?}"));
    assert!(
        governed.contains("\"erp_pg\".\"public\".\"customers\""),
        "{governed}"
    );
    assert!(
        governed.contains("CASE WHEN \"email\" IS NULL THEN NULL ELSE '***' END AS \"email\""),
        "{governed}"
    );
    assert!(sent.iter().any(|s| s.contains("LIMIT 0")), "{sent:?}");
    assert!(
        sent.iter().any(|s| s.contains("LIMIT 50")),
        "the preview row read carries its own cap: {sent:?}"
    );
    assert!(
        sent.iter()
            .all(|s| !s.contains("SETTINGS") && !s.contains("EXPLAIN AST")),
        "no ClickHouse construct or dry run: {sent:?}"
    );

    // Persistence is not fabricated: the ClickHouse metadata store is absent
    // in this installation, so the list route says so before any dial.
    let (status, refused) = get_json(&router, "/api/dashboard/sources", &cookie).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{refused}");
    let message = refused["error"].as_str().unwrap_or_default();
    assert!(message.contains("ClickHouse"), "{message}");
    assert!(message.contains("does not deploy"), "{message}");

    // A chart over unsaved SQL is honest about the builder's limit too.
    let (status, chart) = post_json(
        &router,
        "/api/dashboard/sources/preview",
        &cookie,
        json!({
            "sql": "SELECT id, email FROM erp_pg.public.customers",
            "chart": {
                "title": "Customers", "kind": "hbar",
                "dimension": "email", "measures": ["id"],
            },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{chart}");
    assert!(
        chart["error"]
            .as_str()
            .unwrap_or_default()
            .contains("Inline chart previews are not available"),
        "{chart}"
    );
}
