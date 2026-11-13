//! Restore a dropped Bronze table over the real router (`DATA-15` F4), with
//! one `wiremock` server standing in for `ClickHouse` (the dataset registry),
//! `Lakekeeper`'s Iceberg REST catalog (is the table there?) and its
//! management API (what was dropped, undrop).
//!
//! "The table exists" is decided in `assess`'s own unit tests; faking a full
//! Iceberg `LoadTableResult` here would test the mock, so these tests cover
//! the answers that start from a missing table.
//!
//! # The drop (`DATA-55`, QA `LH-DATA-15-06`, `TC-1731`)
//!
//! This file also covers the drop that makes that restore reachable from the
//! console: `GET` and `POST /api/catalog/{id}/drop`. The restore and the drop
//! are one feature — a table is dropped and comes back on the same page — so
//! the two share the stands-in server, the catalog tenant and the registry the
//! tests here already build.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    CATALOG_TENANT, TestApp, create_principal_with_permissions, in_the_catalog_tenant,
    session_cookie_for_user, spin_up_with_env,
};
use lakehouse_store::uploads::{self, LoadMode, NewTableDrop, NewUpload};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{body_json, body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const RESTORE: &str = "/api/catalog/orders/restore";
const DROP: &str = "/api/catalog/orders/drop";

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

/// The drop's app (`DATA-55`): the app above, plus the catalog tenant the
/// tenant gate asks about and the Iceberg query database the row count reads.
///
/// The seeded connector `conn-pg-lakehouse` targets the table name `orders`
/// (`0034_seed_connector_ingest_spec.sql`), which would make every `orders`
/// asset a connector's table and refuse every case here. These tests mean an
/// upload-made table of that name, so the seed's target is cleared, and the
/// one test that asserts the connector rule writes it back.
async fn drop_app(server: &MockServer, with_admin: bool) -> TestApp {
    let mut env = HashMap::new();
    env.insert("CH_URL".to_owned(), server.uri());
    env.insert("LAKEKEEPER_CATALOG_URI".to_owned(), server.uri());
    env.insert("LAKEKEEPER_WAREHOUSE".to_owned(), "default".to_owned());
    env.insert(
        "LAKEKEEPER_READ_TOKEN_FILE".to_owned(),
        token_file("read.jwt"),
    );
    env.insert("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT.to_owned());
    env.insert("ICEBERG_QUERY_DB".to_owned(), "icecat_api".to_owned());
    if with_admin {
        env.insert("LAKEKEEPER_BASE_URI".to_owned(), server.uri());
        env.insert(
            "LAKEKEEPER_ADMIN_TOKEN_FILE".to_owned(),
            token_file("admin.jwt"),
        );
    }
    let app = spin_up_with_env(&env).await;
    sqlx::query("UPDATE connector SET source_objects = '[]'::jsonb WHERE id = 'conn-pg-lakehouse'")
        .execute(&app.pool)
        .await
        .expect("clear the seeded connector's target");
    app
}

/// A person in the catalog's tenant holding exactly `catalog:read` and
/// `governance:write` — the permissions the drop's read and write need.
async fn governor(pool: &sqlx::PgPool, permissions: &str) -> String {
    let user = create_principal_with_permissions(pool, permissions).await;
    in_the_catalog_tenant(pool, user).await;
    session_cookie_for_user(pool, user).await
}

/// Narrow every role `user` holds to exactly `grants` (`(schema, table)`): a
/// role that reads only the listed tables (`GOV-7` D4; the form
/// `tests/table_access.rs` uses).
async fn narrow(pool: &sqlx::PgPool, user: Uuid, grants: &[(&str, &str)]) {
    let roles: Vec<Uuid> =
        sqlx::query_scalar("SELECT role_id FROM app_user_role WHERE user_id = $1")
            .bind(user)
            .fetch_all(pool)
            .await
            .expect("the user's roles");
    assert!(!roles.is_empty(), "the user must hold a role to narrow");
    for role in roles {
        sqlx::query("UPDATE role SET read_scope = 'listed' WHERE id = $1")
            .bind(role)
            .execute(pool)
            .await
            .expect("narrow the role");
        sqlx::query("DELETE FROM role_read_grant WHERE role_id = $1")
            .bind(role)
            .execute(pool)
            .await
            .expect("clear the grants");
        for (schema, table) in grants {
            sqlx::query(
                "INSERT INTO role_read_grant (role_id, schema_name, table_name) VALUES ($1, $2, $3)",
            )
            .bind(role)
            .bind(schema)
            .bind(table)
            .execute(pool)
            .await
            .expect("grant the table");
        }
    }
}

/// Claim `table` for `tenant` the way the ingest route does, on behalf of
/// `up-1`. The claim must be allowed.
async fn claim(pool: &sqlx::PgPool, tenant: &str, table: &str) {
    let tenant_id = Uuid::parse_str(tenant).expect("a tenant id");
    assert!(
        uploads::claim_table(pool, tenant_id, table, "up-1")
            .await
            .unwrap(),
        "{table} could not be claimed for {tenant}"
    );
}

/// The two `ClickHouse` answers a counted read needs (`DATA-55` D7): the
/// table's columns, then its rows counted with `WHERE 1`. Mounted BEFORE
/// `mount_registry`, because `wiremock` takes the first mock that matches and
/// the registry answers every `POST /`.
async fn mount_row_count(server: &MockServer, n: i64) {
    Mock::given(method("POST"))
        .and(body_string_contains("DESCRIBE TABLE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "rows": 1, "data": [{"name": "id", "type": "String"}],
        })))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("WHERE 1 SETTINGS max_execution_time"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "rows": 1, "data": [{"n": n.to_string()}],
        })))
        .mount(server)
        .await;
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

/// The one sentence a table that is not an upload's gets, whichever of D4's
/// first three rules failed (`DATA-55` D5).
const ONLY_UPLOAD: &str = "Only a table made by a file upload can be dropped here.";
/// A drop inside its window is already on record (`DATA-55` D9).
const ALREADY_DROPPED: &str = "This table is already dropped. It can be restored from its page.";
/// A load is running into the name (`DATA-55` D4 rule 5).
const LOADING: &str = "A file is being loaded into this table, so it cannot be dropped yet.";
/// No admin client, so nothing could bring the table back (`DATA-55` D4 rule 6).
const NOT_SET_UP: &str = "Tables cannot be dropped on this install: restore is not configured, so a dropped table could not come back.";

/// `DATA-55` D4/D7: the read names an upload's table, and pays for the count
/// only when the dialog asks for it — opening the page costs no scan.
#[tokio::test]
async fn the_drop_status_names_an_uploads_table_and_counts_its_rows_only_when_asked() {
    let server = MockServer::start().await;
    mount_row_count(&server, 1234).await;
    mount_registry(&server, "orders").await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "GET", DROP, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["droppable"], json!(true));
    assert_eq!(body["table"], json!("bronze.orders"));
    assert_eq!(body["keptDays"], json!(7));
    assert!(
        body.get("rowCount").is_none(),
        "no count without `?withRowCount=true`: {body}"
    );

    let (status, body) = call(
        &router,
        "GET",
        "/api/catalog/orders/drop?withRowCount=true",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["rowCount"], json!(1234));
}

/// `DATA-55` D7: an uncounted table is `null`, never 0 — the dialog says the
/// rows could not be counted, and 0 is a real answer for an empty table.
#[tokio::test]
async fn the_drop_status_says_null_when_the_rows_cannot_be_counted_and_never_zero() {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(
        &router,
        "GET",
        "/api/catalog/orders/drop?withRowCount=true",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["droppable"], json!(true));
    assert_eq!(body["rowCount"], Value::Null);
    assert_ne!(body["rowCount"], json!(0), "an unknown count is never 0");
}

/// `DATA-55` D4 rules 1 to 3 and D5: a connector's table, another tenant's
/// upload's table, a name a connector also targets and a Silver table are all
/// refused with the SAME one sentence, so the answer never says whose upload
/// made a table.
#[tokio::test]
async fn the_drop_status_refuses_every_table_that_is_not_an_upload_of_the_callers_tenant_with_one_sentence()
 {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "GET", DROP, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["droppable"], json!(false));
    assert_eq!(body["reason"], json!(ONLY_UPLOAD), "no claim at all");

    claim(&pool, "11111111-1111-4111-8111-000000000002", "orders").await;
    let (_, body) = call(&router, "GET", DROP, &cookie).await;
    assert_eq!(body["droppable"], json!(false));
    assert_eq!(
        body["reason"],
        json!(ONLY_UPLOAD),
        "another tenant claimed it"
    );

    sqlx::query("DELETE FROM upload_table_claim WHERE bronze_table = 'orders'")
        .execute(&pool)
        .await
        .unwrap();
    claim(&pool, CATALOG_TENANT, "orders").await;
    sqlx::query(
        "UPDATE connector SET source_objects = '[{\"name\":\"public.orders\",\"target\":\"orders\"}]'::jsonb \
         WHERE id = 'conn-pg-lakehouse'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let (_, body) = call(&router, "GET", DROP, &cookie).await;
    assert_eq!(body["droppable"], json!(false));
    assert_eq!(body["reason"], json!(ONLY_UPLOAD), "a connector targets it");

    let (status, body) = call(&router, "GET", "/api/catalog/silver.orders/drop", &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["droppable"], json!(false));
    assert_eq!(body["reason"], json!(ONLY_UPLOAD), "a Silver table");
}

/// `DATA-55` D5: a table the caller's roles may not read answers the same
/// `404 Asset not found` as an unknown slug, so "not there" and "not yours"
/// cannot be told apart.
#[tokio::test]
async fn the_drop_status_of_a_table_the_callers_role_does_not_read_is_the_unknown_assets_404() {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    let user = create_principal_with_permissions(&pool, "catalog:read, governance:write").await;
    in_the_catalog_tenant(&pool, user).await;
    narrow(&pool, user, &[("finance", "*")]).await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, body) = call(&router, "GET", DROP, &cookie).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], json!("Asset not found"));
}

/// `DATA-55` D4 rule 5: a table with a load in progress is not droppable, and
/// the answer says why.
#[tokio::test]
async fn the_drop_status_says_a_load_is_in_progress() {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    let tenant_id = Uuid::parse_str(CATALOG_TENANT).expect("a tenant id");
    uploads::insert(
        &pool,
        &NewUpload {
            id: "up-1",
            original_filename: "stock.csv",
            storage_key: "uploads/up-1/up-1.csv",
            content_type: "text/csv",
            size_bytes: 8,
            sha256: "",
            uploaded_by: "Seeded Uploader",
            tenant_id,
        },
    )
    .await
    .unwrap();
    claim(&pool, CATALOG_TENANT, "orders").await;
    let options = json!({ "encoding": "utf-8", "delimiter": ",", "headerRow": 0 });
    uploads::mark_ingesting(&pool, "up-1", &options, "orders", LoadMode::Replace, None)
        .await
        .unwrap()
        .expect("the upload is loadable");
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "GET", DROP, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["droppable"], json!(false));
    assert_eq!(body["reason"], json!(LOADING));
}

/// `DATA-55` D3/D4 rule 6: with no admin client nothing can be dropped, and
/// the read says so rather than guessing.
#[tokio::test]
async fn without_the_admin_client_the_drop_status_says_dropping_is_not_set_up() {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    let TestApp { router, pool } = drop_app(&server, false).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "GET", DROP, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["droppable"], json!(false));
    assert_eq!(body["reason"], json!(NOT_SET_UP));
}

/// `DATA-55` D4 rule 4 / D9: a table already dropped is not droppable again,
/// and the sentence says where it can be restored.
#[tokio::test]
async fn the_drop_status_of_a_dropped_table_says_it_is_already_dropped() {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    let tenant_id = Uuid::parse_str(CATALOG_TENANT).expect("a tenant id");
    claim(&pool, CATALOG_TENANT, "orders").await;
    uploads::record_table_drop(
        &pool,
        &NewTableDrop {
            bronze_table: "orders",
            tenant_id,
            asset_id: "orders",
            dropped_by: "Ada Lovelace",
            row_count: Some(1),
            kept_seconds: 604_800,
        },
    )
    .await
    .unwrap()
    .expect("the drop is recorded");
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "GET", DROP, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["droppable"], json!(false));
    assert_eq!(body["reason"], json!(ALREADY_DROPPED));
}
