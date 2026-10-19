//! Restore a dropped Bronze table over the real router (`DATA-15` F4), with
//! one `wiremock` server standing in for `ClickHouse` (the dataset registry),
//! `Lakekeeper`'s Iceberg REST catalog (is the table there?) and its
//! management API (what was dropped, undrop).
//!
//! "The table exists" is decided in `assess`'s own unit tests; faking a full
//! Iceberg `LoadTableResult` here would test the mock, so these tests cover
//! the answers that start from a missing table.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const RESTORE: &str = "/api/catalog/orders/restore";

async fn call(app: &axum::Router, verb: &str, uri: &str, cookie: &str) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(verb)
                .uri(uri)
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

/// The registry answers `table_name` for any slug.
async fn mount_registry(server: &MockServer, table_name: &str) {
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!(
            r#"{{"meta":[{{"name":"table_name","type":"String"}}],"data":[{{"table_name":"{table_name}"}}],"rows":1}}"#
        )))
        .mount(server)
        .await;
}

/// The Iceberg REST catalog has no such table.
async fn mount_missing_table(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/config"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"defaults":{},"overrides":{}}"#),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/namespaces/bronze/tables/orders"))
        .respond_with(ResponseTemplate::new(404).set_body_string(
            r#"{"error":{"message":"no such table","type":"NoSuchTableException","code":404}}"#,
        ))
        .mount(server)
        .await;
}

async fn mount_warehouse(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/management/v1/warehouse"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "warehouses": [{"id": "wh-1", "name": "default"}]
        })))
        .mount(server)
        .await;
}

async fn mount_dropped(server: &MockServer, expires: &str) {
    Mock::given(method("GET"))
        .and(path("/management/v1/warehouse/wh-1/deleted-tabulars"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "tabulars": [{
                "id": "t-1", "name": "orders", "namespace": ["bronze"], "typ": "table",
                "warehouse-id": "wh-1",
                "created-at": "2026-10-01T00:00:00Z",
                "deleted-at": "2026-10-16T08:00:00Z",
                "expiration-date": expires
            }]
        })))
        .mount(server)
        .await;
}

fn token_file(name: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(name);
    std::fs::write(&file, "test-token").unwrap();
    std::mem::forget(dir);
    file.to_str().unwrap().to_owned()
}

async fn app(server: &MockServer, with_admin: bool) -> TestApp {
    let mut env = HashMap::new();
    env.insert("CH_URL".to_owned(), server.uri());
    env.insert("LAKEKEEPER_CATALOG_URI".to_owned(), server.uri());
    env.insert("LAKEKEEPER_WAREHOUSE".to_owned(), "default".to_owned());
    env.insert(
        "LAKEKEEPER_READ_TOKEN_FILE".to_owned(),
        token_file("read.jwt"),
    );
    if with_admin {
        env.insert("LAKEKEEPER_BASE_URI".to_owned(), server.uri());
        env.insert(
            "LAKEKEEPER_ADMIN_TOKEN_FILE".to_owned(),
            token_file("admin.jwt"),
        );
    }
    spin_up_with_env(&env).await
}

/// `*:*` passes the tenant gate and holds every permission.
async fn admin_cookie(pool: &sqlx::PgPool) -> String {
    let user = create_principal_with_permissions(pool, "*:*").await;
    session_cookie_for_user(pool, user).await
}

#[tokio::test]
async fn a_table_dropped_inside_its_window_is_reported_restorable_and_undropped() {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    mount_missing_table(&server).await;
    mount_warehouse(&server).await;
    mount_dropped(&server, "2099-01-01T00:00:00Z").await;
    Mock::given(method("POST"))
        .and(path(
            "/management/v1/warehouse/wh-1/deleted-tabulars/undrop",
        ))
        .and(body_json(
            json!({"targets": [{"id": "t-1", "type": "table"}]}),
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let TestApp { router, pool } = app(&server, true).await;
    let cookie = admin_cookie(&pool).await;

    let (status, body) = call(&router, "GET", RESTORE, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["restorable"], json!(true));
    assert_eq!(body["droppedAt"], json!("2026-10-16T08:00:00Z"));
    assert_eq!(body["expiresAt"], json!("2099-01-01T00:00:00Z"));

    let (status, body) = call(&router, "POST", RESTORE, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_event WHERE action = 'catalog.restore' AND resource_id = 'orders'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audited, 1);
}

#[tokio::test]
async fn a_table_past_its_window_is_not_restorable_and_the_undrop_is_410() {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    mount_missing_table(&server).await;
    mount_warehouse(&server).await;
    mount_dropped(&server, "2020-01-01T00:00:00Z").await;
    let TestApp { router, pool } = app(&server, true).await;
    let cookie = admin_cookie(&pool).await;

    let (_, body) = call(&router, "GET", RESTORE, &cookie).await;
    assert_eq!(body["restorable"], json!(false));
    assert_eq!(
        body["reason"],
        json!("the 7-day window to restore this table has passed")
    );
    let (status, body) = call(&router, "POST", RESTORE, &cookie).await;
    assert_eq!(status, StatusCode::GONE, "{body}");
    assert_eq!(
        body["error"],
        json!("the 7-day window to restore this table has passed")
    );
}

#[tokio::test]
async fn with_no_dropped_copy_the_status_says_so_and_the_undrop_is_404() {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    mount_missing_table(&server).await;
    mount_warehouse(&server).await;
    Mock::given(method("GET"))
        .and(path("/management/v1/warehouse/wh-1/deleted-tabulars"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"tabulars": []})))
        .mount(&server)
        .await;
    let TestApp { router, pool } = app(&server, true).await;
    let cookie = admin_cookie(&pool).await;

    let (_, body) = call(&router, "GET", RESTORE, &cookie).await;
    assert_eq!(body["restorable"], json!(false));
    assert!(body["reason"].as_str().unwrap().contains("no dropped copy"));
    let (status, _) = call(&router, "POST", RESTORE, &cookie).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// PR #106 review R1 SHOULD-FIX DATA-15 2
#[tokio::test]
async fn when_deleted_list_is_truncated_status_answers_could_not_tell_and_undrop_is_503() {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    mount_missing_table(&server).await;
    mount_warehouse(&server).await;
    Mock::given(method("GET"))
        .and(path("/management/v1/warehouse/wh-1/deleted-tabulars"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "tabulars": [],
            "next-page-token": "more"
        })))
        .mount(&server)
        .await;
    let TestApp { router, pool } = app(&server, true).await;
    let cookie = admin_cookie(&pool).await;

    let (_, body) = call(&router, "GET", RESTORE, &cookie).await;
    assert_eq!(body["restorable"], json!(false));
    assert!(body["reason"].as_str().unwrap().contains("could not tell"));
    let (status, body) = call(&router, "POST", RESTORE, &cookie).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body["error"].as_str().unwrap().contains("could not tell"));
}

#[tokio::test]
async fn without_the_admin_client_restore_is_unsupported_and_the_undrop_is_503() {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    let TestApp { router, pool } = app(&server, false).await;
    let cookie = admin_cookie(&pool).await;

    let (status, body) = call(&router, "GET", RESTORE, &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["supported"], json!(false));
    assert_eq!(
        body["reason"],
        json!("restore is not configured on this install")
    );
    let (status, body) = call(&router, "POST", RESTORE, &cookie).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body["error"],
        json!("restore is not configured on this install")
    );
}

#[tokio::test]
async fn a_clickhouse_table_is_unsupported_and_has_nothing_to_undrop() {
    let server = MockServer::start().await;
    let TestApp { router, pool } = app(&server, true).await;
    let cookie = admin_cookie(&pool).await;
    let uri = "/api/catalog/silver.orders/restore";

    let (status, body) = call(&router, "GET", uri, &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["supported"], json!(false));
    let (status, _) = call(&router, "POST", uri, &cookie).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_refused_undrop_is_a_conflict_with_a_fixed_sentence_and_no_upstream_text() {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    mount_missing_table(&server).await;
    mount_warehouse(&server).await;
    mount_dropped(&server, "2099-01-01T00:00:00Z").await;
    Mock::given(method("POST"))
        .and(path(
            "/management/v1/warehouse/wh-1/deleted-tabulars/undrop",
        ))
        .respond_with(
            ResponseTemplate::new(409)
                .set_body_string("table already exists at s3://internal-bucket/x"),
        )
        .mount(&server)
        .await;
    let TestApp { router, pool } = app(&server, true).await;
    let cookie = admin_cookie(&pool).await;

    let (status, body) = call(&router, "POST", RESTORE, &cookie).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(!body.to_string().contains("internal-bucket"), "{body}");
}

#[tokio::test]
async fn a_caller_without_governance_write_cannot_undrop() {
    let server = MockServer::start().await;
    let TestApp { router, pool } = app(&server, true).await;
    let user = create_principal_with_permissions(&pool, "catalog:read, catalog:write").await;
    let cookie = session_cookie_for_user(&pool, user).await;
    let (status, body) = call(&router, "POST", RESTORE, &cookie).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], json!("permission_denied: governance:write"));
}
