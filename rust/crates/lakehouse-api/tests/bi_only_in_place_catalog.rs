//! `BI-44` slice 2: a `bi-only` installation lists and reads its own
//! external tables through the ONE engine it started (`Trino` or
//! `ClickHouse`), and refuses a source that engine cannot read in place.
//!
//! The harness points every upstream it does not own at an immediately
//! refusing address. A `GET /api/catalog` that dialed `ClickHouse` or
//! `Lakekeeper` would answer `503`, so a `200` with the wiremock `Trino`'s
//! tables is proof that the in-place path never dialed them. The `Trino`
//! path still asserts the statements the mock actually received.
//!
//! `full` mode is covered by the existing catalog, connector and query
//! tests; nothing in this file changes that stack.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};
use lakehouse_store::governance::CreatePolicyInput;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The `bi-only` Trino deployment the two Trino tests run against: only
/// `TRINO_URL` names a live upstream; `CH_URL` is explicitly unset, the
/// shape the standalone Compose file produces.
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
        "id": "q1",
        "columns": columns.iter().map(|n| json!({"name": n, "type": "varchar"})).collect::<Vec<_>>(),
        "data": rows,
    })
}

async fn mount_trino(server: &MockServer, contains: &str, body: Value) {
    Mock::given(method("POST"))
        .and(path("/v1/statement"))
        .and(body_string_contains(contains))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// A role with a KNOWN name, because policy conditions match role names and
/// `create_principal_with_permissions` deliberately gives its role a random
/// one. The named role holds exactly `query:read`.
async fn create_named_reader(pool: &lakehouse_store::PgPool, role: &str) -> Uuid {
    let role_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO role (id, name, permissions, description) VALUES ($1, $2, $3, 'test: in-place reader')",
    )
    .bind(role_id)
    .bind(role)
    // `*:*` keeps table access unrestricted (a role with no read grant
    // would make the tenant gate hide the external table); the route's
    // own `query:read` check is still satisfied by the same permission set.
    .bind("*:*")
    .execute(pool)
    .await
    .expect("insert the named reader role");

    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO app_user (id, name, username, email, status) VALUES ($1, 'In Place Reader', $2, $3, 'active')",
    )
    .bind(user_id)
    .bind(format!("in-place-{user_id}"))
    .bind(format!("in-place-{user_id}@test.invalid"))
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

async fn get(app: &axum::Router, route: &str, cookie: &str) -> (StatusCode, Value) {
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

/// In `bi-only+trino`, `GET /api/catalog` is answered by wiremock `Trino`'s
/// `SHOW CATALOGS`/`SHOW SCHEMAS`/`SHOW TABLES` and never by the registry,
/// `ClickHouse` or `Lakekeeper` (both of which are undialable here). The
/// deployment route reports the same engine's source matrix.
#[tokio::test]
async fn a_bi_only_trino_catalog_lists_only_the_active_engines_tables() {
    let server = MockServer::start().await;
    mount_trino(
        &server,
        "SHOW CATALOGS",
        trino_page(&["Catalog"], &[vec!["system"], vec!["erp_pg"]]),
    )
    .await;
    mount_trino(
        &server,
        "SHOW SCHEMAS FROM \"erp_pg\"",
        trino_page(&["Schema"], &[vec!["information_schema"], vec!["public"]]),
    )
    .await;
    mount_trino(
        &server,
        "SHOW TABLES FROM \"erp_pg\".\"public\"",
        trino_page(&["Table"], &[vec!["customers"]]),
    )
    .await;

    let TestApp { router, pool } = spin_up_with_env(&trino_env(&server.uri())).await;
    let user = create_principal_with_permissions(&pool, "catalog:read").await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, body) = get(&router, "/api/catalog", &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let assets = body["assets"].as_array().expect("an asset array");
    assert_eq!(assets.len(), 1, "{body}");
    assert_eq!(assets[0]["id"], json!("erp_pg.public.customers"));
    assert_eq!(assets[0]["zone"], json!("external"));
    assert!(
        assets[0]["rows"].is_null(),
        "no fabricated row count: {body}"
    );
    assert!(
        assets[0]["sizeBytes"].is_null(),
        "no fabricated size: {body}"
    );
    assert_eq!(body["namespaces"][0]["id"], json!("erp_pg.public"));

    let statements: Vec<String> = server
        .received_requests()
        .await
        .expect("the mock records requests")
        .iter()
        .map(|request| String::from_utf8_lossy(&request.body).into_owned())
        .collect();
    assert!(statements.iter().any(|s| s.contains("SHOW CATALOGS")));
    assert!(
        statements
            .iter()
            .any(|s| s.contains("SHOW SCHEMAS FROM \"erp_pg\""))
    );
    assert!(
        statements
            .iter()
            .any(|s| s.contains("SHOW TABLES FROM \"erp_pg\".\"public\""))
    );

    let (settings_status, _) = get(&router, "/api/settings/deployment", &cookie).await;
    // `catalog:read` alone cannot read settings; use an admin for the
    // deployment assertion rather than weakening that route's gate.
    assert_eq!(settings_status, StatusCode::FORBIDDEN);
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let admin_cookie = session_cookie_for_user(&pool, admin).await;
    let (status, deployment) = get(&router, "/api/settings/deployment", &admin_cookie).await;
    assert_eq!(status, StatusCode::OK, "{deployment}");
    assert_eq!(deployment["installMode"], json!("bi-only"));
    assert_eq!(
        deployment["supportedInPlaceSources"][0],
        json!("postgresql")
    );
}

/// A mask and a row filter authored on a 3-part external table are enforced
/// through table substitution on the `Trino` path: the derived table keeps
/// the quoted catalog, the masked column is `***` in the answer, and the
/// statement `Trino` received carries the row filter.
#[tokio::test]
async fn a_bi_only_trino_query_masks_and_filters_a_three_part_external_table() {
    let server = MockServer::start().await;
    // Column introspection is the one metadata read the rewrite needs.
    mount_trino(
        &server,
        "DESCRIBE",
        trino_page(
            &["Column", "Type", "Extra", "Comment"],
            &[
                vec!["id", "bigint", "", ""],
                vec!["email", "varchar", "", ""],
                vec!["tenant_id", "varchar", "", ""],
            ],
        ),
    )
    .await;
    // The rewritten query still reads the real external table; it is the
    // only answer that reaches the caller.
    mount_trino(
        &server,
        "SELECT",
        json!({
            "id": "q2",
            "columns": [
                {"name": "id", "type": "bigint"},
                {"name": "email", "type": "varchar"}
            ],
            "data": [[1, "***"]],
        }),
    )
    .await;

    let TestApp { router, pool } = spin_up_with_env(&trino_env(&server.uri())).await;
    let user = create_named_reader(&pool, "External Reader").await;
    let cookie = session_cookie_for_user(&pool, user).await;
    lakehouse_store::governance::create_policy(
        &pool,
        &CreatePolicyInput {
            name: "in-place-external-mask".to_owned(),
            kind: "Row filter".to_owned(),
            subjects: "External Reader".to_owned(),
            resources: "erp_pg.public.customers".to_owned(),
            effect: "Permit with obligation".to_owned(),
            conditions: Some(
                json!({
                    "roles": ["External Reader"],
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

    let (status, body) = post_json(
        &router,
        "/api/query/run",
        &cookie,
        json!({"sql": "SELECT * FROM erp_pg.public.customers", "engine": "trino"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["engine"], json!("trino"));
    assert_eq!(body["rows"][0]["email"], json!("***"), "{body}");

    let statements: Vec<String> = server
        .received_requests()
        .await
        .expect("the mock records requests")
        .iter()
        .map(|request| String::from_utf8_lossy(&request.body).into_owned())
        .collect();
    let query = statements
        .iter()
        .find(|s| s.contains("FROM (SELECT"))
        .unwrap_or_else(|| panic!("the governed query was not sent to Trino: {statements:?}"));
    assert!(
        query.contains("\"erp_pg\".\"public\".\"customers\""),
        "{query}"
    );
    assert!(
        query.contains("CASE WHEN \"email\" IS NULL THEN NULL ELSE '***' END AS \"email\""),
        "{query}"
    );
    assert!(query.contains("tenant_id = 'tenant-a'"), "{query}");
}

/// In `bi-only+clickhouse`, registering a source the installed engine
/// cannot read in place is refused with an honest 400 before any probe or
/// credential is stored; `full` mode's connector set is unchanged.
#[tokio::test]
async fn a_bi_only_clickhouse_refuses_an_unsupported_source_honestly() {
    let env = HashMap::from([
        ("LAKEHOUSE_INSTALL_MODE".to_owned(), "bi-only".to_owned()),
        ("LAKEHOUSE_QUERY_ENGINE".to_owned(), "clickhouse".to_owned()),
    ]);
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let user = create_principal_with_permissions(&pool, "connector:manage").await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let body = json!({
        "name": "unsupported-oracle-source",
        "type": "oracle",
        "direction": "source",
        "host": "db.example.invalid",
        "credential": {"source": "managed", "primary": "password", "values": {"primary": "pw"}},
        "environment": "production",
        "tenant": "g6",
        "residency": "",
        "capabilities": [],
    });
    let (status, answer) = post_json(&router, "/api/connectors", &cookie, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    let message = answer["error"].as_str().unwrap_or_default();
    assert!(message.contains("oracle"), "{message}");
    assert!(message.contains("clickhouse"), "{message}");
    assert!(message.contains("not supported in place"), "{message}");
}
