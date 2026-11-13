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
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    CATALOG_TENANT, TestApp, create_principal_with_permissions, in_the_catalog_tenant,
    mount_existing_iceberg_table, session_cookie_for_user, spin_up_with_env, spin_up_with_state,
};
use lakehouse_auth::Secret;
use lakehouse_auth::openfga::LakekeeperAdminClient;
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

/// The catalog's deleted list holds no table of this name.
async fn mount_nothing_deleted(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/management/v1/warehouse/wh-1/deleted-tabulars"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "tabulars": [] })))
        .mount(server)
        .await;
}

/// The catalog's deleted list cannot be read at all: the answer that leaves an
/// unknown drop unknown (`DATA-55` review R2).
async fn mount_deleted_unreadable(server: &MockServer, marker: &str) {
    Mock::given(method("GET"))
        .and(path("/management/v1/warehouse/wh-1/deleted-tabulars"))
        .respond_with(ResponseTemplate::new(500).set_body_string(marker.to_owned()))
        .mount(server)
        .await;
}

/// The counted read's two questions, the second failing: the rows "could not
/// be counted" (`DATA-55` D7), and the `null` a test sees comes from that
/// failure rather than from a registry row without a count (review R5).
/// Mounted BEFORE `mount_registry`, like [`mount_row_count`].
async fn mount_count_unreadable(server: &MockServer, marker: &str) {
    Mock::given(method("POST"))
        .and(body_string_contains("DESCRIBE TABLE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "rows": 1, "data": [{"name": "id", "type": "String"}],
        })))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("WHERE 1 SETTINGS max_execution_time"))
        .respond_with(ResponseTemplate::new(500).set_body_string(marker.to_owned()))
        .mount(server)
        .await;
}

/// A registry that answers no dataset for `slug`, and `table_name` for every
/// other: the "unknown id" half of `DATA-55` D5 (review R5).
async fn mount_unknown_slug(server: &MockServer, slug: &str) {
    Mock::given(method("POST"))
        .and(body_string_contains(format!("'{slug}'")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [{"name": "table_name", "type": "String"}], "data": [], "rows": 0,
        })))
        .with_priority(2)
        .mount(server)
        .await;
}

/// The tenant gate's own sentence (`routes::catalog::
/// CATALOG_TENANT_REFUSAL_NOT_OWNER`), asserted rather than repeated from the
/// handler (review R5), and the `403` body it is carried in.
const NOT_OWNER: &str = "this deployment's shared catalog is owned by the tenant named in CATALOG_TENANT_ID; the caller does not belong to it";

/// The Iceberg REST catalog's answer to one table's load: "no such table"
/// first, then the table — the table coming back while the request is running.
struct BackAfterUndrop(std::sync::atomic::AtomicUsize);

impl wiremock::Respond for BackAfterUndrop {
    fn respond(&self, _request: &wiremock::Request) -> ResponseTemplate {
        let seen = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if seen == 0 {
            ResponseTemplate::new(404).set_body_string(
                r#"{"error":{"message":"no such table","type":"NoSuchTableException","code":404}}"#,
            )
        } else {
            ResponseTemplate::new(200).set_body_string(common::load_table_fixture("orders"))
        }
    }
}

/// The keep's answer held until the test's own write is done: the window
/// `DATA-55` review R3 closes between the drop's checks and its record. The
/// drop cannot pass this call, so the load it must see is provably on record
/// before it writes its own.
struct HeldKeep {
    entered: std::sync::Mutex<std::sync::mpsc::Sender<()>>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl wiremock::Respond for HeldKeep {
    fn respond(&self, _request: &wiremock::Request) -> ResponseTemplate {
        self.entered
            .lock()
            .unwrap()
            .send(())
            .expect("the test is still waiting for the keep");
        self.release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(30))
            .expect("the test releases the keep");
        ResponseTemplate::new(204)
    }
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
    drop_app_with(server, with_admin, |_state| {}).await
}

/// The drop's app, with the built [`lakehouse_api::state::AppState`] offered to
/// `prepare` before the router takes it — for the one test that pins the
/// admin client's bound on the drop's own request (`DATA-55` review R1).
async fn drop_app_with(
    server: &MockServer,
    with_admin: bool,
    prepare: impl FnOnce(&mut lakehouse_api::state::AppState),
) -> TestApp {
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
    let (app, _state) = spin_up_with_state(&env, prepare).await;
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
    // `DATA-55` D8: two entries now, `allowed` before the undrop and `executed`
    // after it. The count changed because the trail must never be wrong: an
    // `executed` written before the catalog answered could be false.
    let allowed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_event WHERE action = 'catalog.restore' \
         AND resource_id = 'orders' AND outcome = 'allowed'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let executed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_event WHERE action = 'catalog.restore' \
         AND resource_id = 'orders' AND outcome = 'executed'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((allowed, executed), (1, 1));
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
/// rows could not be counted, and 0 is a real answer for an empty table. The
/// `null` comes from a count query the stand-in answers with a 500, not from a
/// registry row that answers without a row count; review R5.
#[tokio::test]
async fn the_drop_status_says_null_when_the_rows_cannot_be_counted_and_never_zero() {
    let server = MockServer::start().await;
    mount_count_unreadable(&server, "planted-upstream-marker").await;
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

/// The keep could not be confirmed, so nothing was dropped (`DATA-55` D3).
const KEEP_UNCONFIRMED: &str = "The catalog could not confirm that a dropped table is kept for 7 days, so nothing was dropped. Try again shortly.";
/// The `allowed` entry could not be written, so nothing was dropped (D8).
const NOT_AUDITED: &str =
    "The drop could not be written to the audit trail, so nothing was dropped.";
/// The catalog holds no table of that name (`DATA-55` D5).
const NO_TABLE: &str = "There is no table to drop: the catalog holds no table of that name.";
/// The catalog refused the drop (`DATA-55` D5).
const REFUSED: &str = "The catalog refused to drop the table. Nothing was dropped.";
/// The catalog did not answer, so the outcome is not known (`DATA-55` D5).
const UNKNOWN: &str = "The catalog did not answer, so it is not known whether the table was dropped. Open the table's page again: it says so if it was.";
/// The catalog could not be looked up at all, so nothing was dropped (`DATA-55`
/// D5).
const UNREACHABLE: &str =
    "The catalog could not be reached, so nothing was dropped. Try again shortly.";
/// The catalog did not answer AND the table is still there, so nothing was
/// dropped (`DATA-55` review R2, the second unknown-outcome case).
const NOT_DROPPED: &str =
    "The catalog did not answer and the table was not dropped. Try again shortly.";

/// The warehouse lookup, the keep write and the one `DELETE` the drop sends
/// (`DATA-55` D2/D3): the profile `POST` answers 204, and the catalog's drop
/// answers `template`, `times` times.
async fn mount_drop(server: &MockServer, template: ResponseTemplate, times: u64) {
    mount_warehouse(server).await;
    Mock::given(method("POST"))
        .and(path("/management/v1/warehouse/wh-1/delete-profile"))
        .respond_with(ResponseTemplate::new(204))
        .mount(server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/catalog/v1/wh-1/namespaces/bronze/tables/orders"))
        .respond_with(template)
        .expect(times)
        .mount(server)
        .await;
}

/// How many `DELETE` requests the server received: the drop's own call, and
/// nothing else this file sends one for.
async fn deletes(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "DELETE")
        .count()
}

/// The statement of T4's "the audit entry cannot be written" tests: a check
/// constraint that refuses every `catalog.drop` row, in this test's own
/// database.
async fn refuse_drop_audit(pool: &sqlx::PgPool, action: &str) {
    // A `CHECK` expression cannot take a bind parameter, and both callers pass
    // one of this file's own constants, never a value a caller chose.
    let sql = format!(
        "ALTER TABLE audit_event ADD CONSTRAINT no_drop_audit CHECK (action <> '{action}')"
    );
    sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
        .execute(pool)
        .await
        .expect("the audit table takes the constraint");
}

/// `DATA-55` D2/D8: a permitted person drops an upload's table, the catalog is
/// asked once, and the record and the answer carry the counted rows and the
/// 7-day window.
#[tokio::test]
async fn a_permitted_person_drops_an_uploads_table_and_the_catalog_is_asked_once() {
    let server = MockServer::start().await;
    mount_row_count(&server, 1234).await;
    mount_registry(&server, "orders").await;
    mount_drop(&server, ResponseTemplate::new(204), 1).await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["table"], json!("bronze.orders"));
    assert_eq!(body["rowCount"], json!(1234));
    let dropped_at = time::OffsetDateTime::parse(
        body["droppedAt"].as_str().expect("a droppedAt"),
        &time::format_description::well_known::Rfc3339,
    )
    .expect("RFC 3339 droppedAt");
    let expires_at = time::OffsetDateTime::parse(
        body["expiresAt"].as_str().expect("an expiresAt"),
        &time::format_description::well_known::Rfc3339,
    )
    .expect("RFC 3339 expiresAt");
    assert_eq!(
        (expires_at - dropped_at).whole_seconds(),
        604_800,
        "the answer names the 7 days the catalog keeps"
    );

    assert_eq!(deletes(&server).await, 1, "the catalog is asked once");
    let requests = server.received_requests().await.unwrap();
    let profile = requests
        .iter()
        .position(|r| r.url.path().ends_with("/delete-profile"))
        .expect("the keep was written");
    let drop = requests
        .iter()
        .position(|r| r.method.as_str() == "DELETE")
        .expect("the drop was sent");
    assert!(
        profile < drop,
        "the keep is written before the table is dropped"
    );

    let recorded: (String, Option<i64>) = sqlx::query_as(
        "SELECT dropped_by, row_count FROM upload_table_drop WHERE bronze_table = 'orders'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(recorded.0, "Single Perm Test User");
    assert_eq!(recorded.1, Some(1234));
}

/// `DATA-55` D8: two entries per action, `allowed` then `executed`, each
/// naming who, which table and its row count.
#[tokio::test]
async fn the_drop_writes_who_dropped_which_table_and_its_row_count_to_the_audit_trail() {
    let server = MockServer::start().await;
    mount_row_count(&server, 1234).await;
    mount_registry(&server, "orders").await;
    mount_drop(&server, ResponseTemplate::new(204), 1).await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    let user = create_principal_with_permissions(&pool, "catalog:read, governance:write").await;
    in_the_catalog_tenant(&pool, user).await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let rows: Vec<(String, Option<String>, Value)> = sqlx::query_as(
        "SELECT outcome, principal_id, args FROM audit_event \
          WHERE action = 'catalog.drop' AND resource_id = 'orders' ORDER BY at, id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2, "one entry before and one after: {rows:?}");
    assert_eq!(rows[0].0, "allowed");
    assert_eq!(rows[1].0, "executed");
    for (_, principal_id, args) in &rows {
        assert_eq!(principal_id.as_deref(), Some(user.to_string().as_str()));
        assert_eq!(args["table"], json!("orders"));
        assert_eq!(args["rowCount"], json!(1234));
        assert_eq!(args["keptDays"], json!(7));
    }
}

/// `DATA-55` D9: a second drop is a clear `409` and the catalog is not asked
/// again — answering "dropped" twice would name a second person on a drop
/// they did not make.
#[tokio::test]
async fn a_second_drop_of_a_dropped_table_is_a_409_and_the_catalog_is_not_asked_again() {
    let server = MockServer::start().await;
    mount_row_count(&server, 7).await;
    mount_registry(&server, "orders").await;
    mount_drop(&server, ResponseTemplate::new(204), 1).await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], json!(ALREADY_DROPPED));
    assert_eq!(deletes(&server).await, 1, "the catalog was asked once only");
}

/// `DATA-55` D4/D5: none of a connector's table, another tenant's upload's
/// table, a name a connector also targets and a Silver table is dropped, and
/// the catalog is never asked.
#[tokio::test]
async fn a_table_that_is_not_an_upload_of_the_callers_tenant_is_not_dropped_and_the_catalog_is_not_asked()
 {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    mount_drop(&server, ResponseTemplate::new(204), 0).await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::CONFLICT, "no claim at all: {body}");
    assert_eq!(body["error"], json!(ONLY_UPLOAD));

    claim(&pool, "11111111-1111-4111-8111-000000000002", "orders").await;
    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::CONFLICT, "another tenant: {body}");
    assert_eq!(body["error"], json!(ONLY_UPLOAD));

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
    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a connector targets it: {body}"
    );
    assert_eq!(body["error"], json!(ONLY_UPLOAD));

    let (status, body) = call(&router, "POST", "/api/catalog/silver.orders/drop", &cookie).await;
    assert_eq!(status, StatusCode::CONFLICT, "a Silver table: {body}");
    assert_eq!(body["error"], json!(ONLY_UPLOAD));

    assert_eq!(deletes(&server).await, 0, "nothing was dropped");
    let audited: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_event WHERE action = 'catalog.drop'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(audited, 0, "a refused drop writes no audit entry");
}

/// `DATA-55` D4 rule 5: a table a load is running into is not dropped.
#[tokio::test]
async fn a_table_with_a_load_in_progress_is_not_dropped() {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    mount_drop(&server, ResponseTemplate::new(204), 0).await;
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

    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], json!(LOADING));
    assert_eq!(deletes(&server).await, 0);
}

/// `DATA-55` D5: a table the caller's roles may not read is a `404` as for an
/// unknown asset, and learns nothing else.
#[tokio::test]
async fn a_caller_whose_role_does_not_read_the_table_cannot_drop_it_and_learns_nothing() {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    mount_drop(&server, ResponseTemplate::new(204), 0).await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    let user = create_principal_with_permissions(&pool, "catalog:read, governance:write").await;
    in_the_catalog_tenant(&pool, user).await;
    narrow(&pool, user, &[("finance", "*")]).await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], json!("Asset not found"));
    assert_eq!(deletes(&server).await, 0);
}

/// `DATA-55` D8: a drop whose `allowed` entry cannot be written does not
/// happen, and the catalog is not asked.
#[tokio::test]
async fn a_drop_is_refused_when_its_audit_entry_cannot_be_written() {
    let server = MockServer::start().await;
    mount_row_count(&server, 3).await;
    mount_registry(&server, "orders").await;
    mount_drop(&server, ResponseTemplate::new(204), 0).await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    refuse_drop_audit(&pool, "catalog.drop").await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"], json!(NOT_AUDITED));
    assert_eq!(deletes(&server).await, 0);
    let recorded: i64 =
        sqlx::query_scalar("SELECT count(*) FROM upload_table_drop WHERE bronze_table = 'orders'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(recorded, 0, "no record was written");
}

/// `DATA-55` D3: the keep is written before every drop, and nothing is dropped
/// when it cannot be confirmed.
#[tokio::test]
async fn a_drop_is_refused_when_the_seven_day_keep_cannot_be_confirmed() {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    mount_warehouse(&server).await;
    Mock::given(method("POST"))
        .and(path("/management/v1/warehouse/wh-1/delete-profile"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/catalog/v1/wh-1/namespaces/bronze/tables/orders"))
        .respond_with(ResponseTemplate::new(204))
        .expect(0)
        .mount(&server)
        .await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"], json!(KEEP_UNCONFIRMED));
    assert_eq!(deletes(&server).await, 0);
}

/// `DATA-55` review R2 (replacing D8 step 5, which was wrong). An unknown
/// outcome is not "nothing was dropped": if the catalog went on to list the
/// table as deleted, the drop DID happen, so the record is kept, `executed` is
/// written and the caller is told the drop succeeded.
#[tokio::test]
async fn a_catalog_that_answers_500_and_lists_the_table_as_deleted_keeps_the_record_and_answers_a_drop()
 {
    const MARKER: &str = "planted-upstream-marker";
    let server = MockServer::start().await;
    mount_row_count(&server, 3).await;
    mount_registry(&server, "orders").await;
    mount_drop(
        &server,
        ResponseTemplate::new(500).set_body_string(MARKER),
        1,
    )
    .await;
    mount_dropped(&server, "2099-01-01T00:00:00Z").await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["table"], json!("bronze.orders"));
    assert_eq!(body["rowCount"], json!(3));
    assert!(
        !body.to_string().contains(MARKER),
        "upstream text reached the response: {body}"
    );
    assert!(
        uploads::dropped_table(&pool, "orders")
            .await
            .unwrap()
            .is_some(),
        "the drop happened, so its record stays"
    );
    let outcomes: Vec<String> = sqlx::query_scalar(
        "SELECT outcome FROM audit_event WHERE action = 'catalog.drop' ORDER BY at, id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(outcomes, vec!["allowed", "executed"]);
}

/// `DATA-55` review R2: the catalog did not answer, the table is NOT among the
/// deleted ones, and the table itself is still there — so nothing was dropped.
/// The record goes, `failed` is written, and the sentence says what happened
/// instead of saying the outcome is unknown.
#[tokio::test]
async fn a_catalog_that_answers_500_and_a_table_that_still_exists_removes_the_record_and_says_nothing_was_dropped()
 {
    const MARKER: &str = "planted-upstream-marker";
    let server = MockServer::start().await;
    mount_row_count(&server, 3).await;
    mount_registry(&server, "orders").await;
    mount_drop(
        &server,
        ResponseTemplate::new(500).set_body_string(MARKER),
        1,
    )
    .await;
    mount_nothing_deleted(&server).await;
    mount_existing_iceberg_table(&server, "orders").await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"], json!(NOT_DROPPED));
    assert!(
        !body.to_string().contains(MARKER),
        "upstream text reached the response: {body}"
    );
    assert!(
        uploads::dropped_table(&pool, "orders")
            .await
            .unwrap()
            .is_none(),
        "the table was not dropped, so the record goes"
    );
    let outcomes: Vec<String> = sqlx::query_scalar(
        "SELECT outcome FROM audit_event WHERE action = 'catalog.drop' ORDER BY at, id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(outcomes, vec!["allowed", "failed"]);
}

/// `DATA-55` review R2: the catalog did not answer and the lookup that would
/// say whether the table is gone fails too. Nothing is known, so the record is
/// KEPT (fail closed: the name stays closed), `failed` says why, and the
/// caller gets D5's sentence — never the catalog's own text.
#[tokio::test]
async fn a_catalog_that_answers_500_and_a_lookup_that_fails_keeps_the_record_and_says_the_outcome_is_not_known()
 {
    const MARKER: &str = "planted-upstream-marker";
    let server = MockServer::start().await;
    mount_row_count(&server, 3).await;
    mount_registry(&server, "orders").await;
    mount_drop(
        &server,
        ResponseTemplate::new(500).set_body_string(MARKER),
        1,
    )
    .await;
    mount_deleted_unreadable(&server, MARKER).await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"], json!(UNKNOWN));
    assert!(!body.to_string().contains(MARKER), "{body}");
    assert!(
        uploads::dropped_table(&pool, "orders")
            .await
            .unwrap()
            .is_some(),
        "an unknown outcome keeps the name closed"
    );
    let outcomes: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT outcome, detail FROM audit_event WHERE action = 'catalog.drop' ORDER BY at, id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(outcomes.len(), 2, "one entry before and one after");
    assert_eq!(outcomes[1].0, "failed");
    assert_eq!(
        outcomes[1].1.as_deref(),
        Some("the outcome is not known"),
        "the trail names the reason in our own words"
    );
}

/// `DATA-55` review R1: a catalog that accepts the connection and never answers
/// is ended by the client's own bound on the drop, not by the route's 60-second
/// deadline, and it ends as the R2 third case: the record is kept, `failed` is
/// written, and the caller is told the outcome is not known.
///
/// The bound is set the way the review allows: the client takes it as a
/// constructor argument (`with_drop_timeout`), so this test sets it to one
/// second instead of waiting the deployment's fifteen.
#[tokio::test]
async fn a_catalog_that_does_not_answer_within_the_clients_bound_keeps_the_record_and_says_the_outcome_is_not_known()
 {
    const MARKER: &str = "planted-upstream-marker";
    let server = MockServer::start().await;
    mount_row_count(&server, 3).await;
    mount_registry(&server, "orders").await;
    mount_drop(
        &server,
        ResponseTemplate::new(204).set_delay(Duration::from_secs(20)),
        1,
    )
    .await;
    mount_deleted_unreadable(&server, MARKER).await;
    let base = server.uri();
    let TestApp { router, pool } = drop_app_with(&server, true, |state| {
        state.lakekeeper_admin = Some(Arc::new(
            LakekeeperAdminClient::new(base, Secret::new("test-token".to_owned()))
                .with_drop_timeout(Duration::from_secs(1)),
        ));
    })
    .await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"], json!(UNKNOWN));
    assert!(
        uploads::dropped_table(&pool, "orders")
            .await
            .unwrap()
            .is_some(),
        "the record is kept: whether it was dropped is not known"
    );
    let outcomes: Vec<String> = sqlx::query_scalar(
        "SELECT outcome FROM audit_event WHERE action = 'catalog.drop' ORDER BY at, id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(outcomes, vec!["allowed", "failed"]);
}

/// `DATA-55` review R2, the kept-record rule: a record is believed only while
/// the table is absent. A record for a table that IS there is stale — the load
/// that recreated it was not the record's — so the next status read removes it
/// and answers as if it were not there.
#[tokio::test]
async fn a_stale_record_of_a_table_that_exists_is_removed_by_the_next_status_read() {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    mount_existing_iceberg_table(&server, "orders").await;
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
    .expect("the record is written");
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "GET", DROP, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["droppable"],
        json!(true),
        "a table that is there is not dropped: {body}"
    );
    assert!(
        uploads::dropped_table(&pool, "orders")
            .await
            .unwrap()
            .is_none(),
        "the stale record was removed"
    );
}

/// `DATA-55` review R7: a registry table name that is not a plain identifier
/// would put a path segment the catalog must not see into its URL, so it is
/// refused in `assess` rule 1, with D5's one sentence and the client's own
/// check, before anything is written.
#[tokio::test]
async fn a_registry_name_that_is_not_a_plain_identifier_is_refused_before_anything_is_written() {
    let server = MockServer::start().await;
    mount_registry(&server, "orders/../evil").await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;
    claim(&pool, CATALOG_TENANT, "orders/../evil").await;

    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], json!(ONLY_UPLOAD));
    assert_eq!(deletes(&server).await, 0, "the catalog was not asked");
    let audited: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_event WHERE action = 'catalog.drop'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(audited, 0, "nothing was written");
    let recorded: i64 = sqlx::query_scalar("SELECT count(*) FROM upload_table_drop")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(recorded, 0, "nothing was recorded");
}

/// `DATA-55` review R2, the restore's mirror case: the undrop did not answer but
/// the table is there — the restore happened. The record is cleared, `executed`
/// is written, and the caller is told the restore succeeded.
#[tokio::test]
async fn an_undrop_that_does_not_answer_and_a_table_that_exists_clears_the_record_and_answers_a_restore()
 {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    mount_warehouse(&server).await;
    mount_dropped(&server, "2099-01-01T00:00:00Z").await;
    // The first load of the table (before the undrop) says it is gone; the
    // second, after the undrop that did not answer, says it is back.
    Mock::given(method("GET"))
        .and(path("/v1/config"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"defaults":{},"overrides":{}}"#),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/namespaces/bronze/tables/orders"))
        .respond_with(BackAfterUndrop(std::sync::atomic::AtomicUsize::new(0)))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(
            "/management/v1/warehouse/wh-1/deleted-tabulars/undrop",
        ))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    let tenant_id = Uuid::parse_str(CATALOG_TENANT).expect("a tenant id");
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

    let (status, body) = call(&router, "POST", RESTORE, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], json!(true));
    assert!(
        uploads::dropped_table(&pool, "orders")
            .await
            .unwrap()
            .is_none(),
        "the table is back, so the record goes"
    );
    let outcomes: Vec<String> = sqlx::query_scalar(
        "SELECT outcome FROM audit_event WHERE action = 'catalog.restore' ORDER BY at, id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(outcomes, vec!["allowed", "executed"]);
}

/// `DATA-55` review R2, the restore's mirror case that did not happen: the
/// undrop did not answer and the table is still gone. The record stays, `failed`
/// is written, and the answer is the fixed 503.
#[tokio::test]
async fn an_undrop_that_does_not_answer_and_a_table_that_is_still_gone_keeps_the_record_and_writes_a_failed_entry()
 {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    mount_missing_table(&server).await;
    mount_warehouse(&server).await;
    mount_dropped(&server, "2099-01-01T00:00:00Z").await;
    Mock::given(method("POST"))
        .and(path(
            "/management/v1/warehouse/wh-1/deleted-tabulars/undrop",
        ))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    let tenant_id = Uuid::parse_str(CATALOG_TENANT).expect("a tenant id");
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

    let (status, body) = call(&router, "POST", RESTORE, &cookie).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(
        uploads::dropped_table(&pool, "orders")
            .await
            .unwrap()
            .is_some(),
        "the table is still dropped, so the record stays"
    );
    let outcomes: Vec<String> = sqlx::query_scalar(
        "SELECT outcome FROM audit_event WHERE action = 'catalog.restore' ORDER BY at, id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(outcomes, vec!["allowed", "failed"]);
}

/// `DATA-55` D5: a catalog that has no such table, or refuses the drop, is a
/// `409` in our own words, with no record left behind.
#[tokio::test]
async fn a_catalog_that_refuses_the_drop_or_has_no_such_table_is_a_409_with_no_upstream_text() {
    const MARKER: &str = "planted-upstream-marker";

    let refused = MockServer::start().await;
    mount_row_count(&refused, 3).await;
    mount_registry(&refused, "orders").await;
    mount_drop(
        &refused,
        ResponseTemplate::new(403).set_body_string(MARKER),
        1,
    )
    .await;
    let TestApp { router, pool } = drop_app(&refused, true).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;
    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], json!(REFUSED));
    assert!(!body.to_string().contains(MARKER), "{body}");
    let recorded: i64 =
        sqlx::query_scalar("SELECT count(*) FROM upload_table_drop WHERE bronze_table = 'orders'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(recorded, 0);

    let missing = MockServer::start().await;
    mount_row_count(&missing, 3).await;
    mount_registry(&missing, "orders").await;
    mount_drop(&missing, ResponseTemplate::new(404), 1).await;
    let TestApp { router, pool } = drop_app(&missing, true).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;
    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], json!(NO_TABLE));
    let recorded: i64 =
        sqlx::query_scalar("SELECT count(*) FROM upload_table_drop WHERE bronze_table = 'orders'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(recorded, 0);
}

/// `DATA-55` D3/D4 rule 6: with no admin client nothing is dropped, and the
/// answer says why.
#[tokio::test]
async fn without_the_admin_client_nothing_is_dropped() {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    let TestApp { router, pool } = drop_app(&server, false).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"], json!(NOT_SET_UP));
    assert_eq!(deletes(&server).await, 0);
}

/// `DATA-55` D8: a restore of a table dropped from the console clears the
/// record of its drop, so the name is free for a new load again.
#[tokio::test]
async fn restoring_a_table_dropped_from_the_console_clears_its_record() {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    mount_missing_table(&server).await;
    mount_warehouse(&server).await;
    mount_dropped(&server, "2099-01-01T00:00:00Z").await;
    Mock::given(method("POST"))
        .and(path(
            "/management/v1/warehouse/wh-1/deleted-tabulars/undrop",
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    let tenant_id = Uuid::parse_str(CATALOG_TENANT).expect("a tenant id");
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

    let (status, body) = call(&router, "POST", RESTORE, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        uploads::dropped_table(&pool, "orders")
            .await
            .unwrap()
            .is_none(),
        "the record is gone after the restore"
    );
}

/// `DATA-55` D8: a restore whose `allowed` entry cannot be written does not
/// happen, and the catalog is not asked.
#[tokio::test]
async fn a_restore_is_refused_when_its_audit_entry_cannot_be_written() {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    mount_missing_table(&server).await;
    mount_warehouse(&server).await;
    mount_dropped(&server, "2099-01-01T00:00:00Z").await;
    Mock::given(method("POST"))
        .and(path(
            "/management/v1/warehouse/wh-1/deleted-tabulars/undrop",
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(0)
        .mount(&server)
        .await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    refuse_drop_audit(&pool, "catalog.restore").await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "POST", RESTORE, &cookie).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(
        body["error"],
        json!("The restore could not be written to the audit trail, so nothing was restored.")
    );
}

/// `DATA-55` D8: a failed undrop keeps the record the drop wrote, so the name
/// stays closed and the table stays restorable, and the trail says `failed`.
#[tokio::test]
async fn a_failed_undrop_keeps_the_record_and_writes_a_failed_audit_entry() {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    mount_missing_table(&server).await;
    mount_warehouse(&server).await;
    mount_dropped(&server, "2099-01-01T00:00:00Z").await;
    Mock::given(method("POST"))
        .and(path(
            "/management/v1/warehouse/wh-1/deleted-tabulars/undrop",
        ))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    let tenant_id = Uuid::parse_str(CATALOG_TENANT).expect("a tenant id");
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

    let (status, body) = call(&router, "POST", RESTORE, &cookie).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(
        uploads::dropped_table(&pool, "orders")
            .await
            .unwrap()
            .is_some(),
        "the table is still dropped"
    );
    let outcomes: Vec<String> = sqlx::query_scalar(
        "SELECT outcome FROM audit_event WHERE action = 'catalog.restore' ORDER BY at, id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(outcomes, vec!["allowed", "failed"]);
}

/// `DATA-55` D5: a caller whose roles may not read the table cannot restore
/// it, and learns nothing beyond the unknown asset's 404.
#[tokio::test]
async fn a_caller_whose_role_does_not_read_the_table_cannot_restore_it() {
    let server = MockServer::start().await;
    mount_registry(&server, "orders").await;
    mount_missing_table(&server).await;
    mount_warehouse(&server).await;
    mount_dropped(&server, "2099-01-01T00:00:00Z").await;
    Mock::given(method("POST"))
        .and(path(
            "/management/v1/warehouse/wh-1/deleted-tabulars/undrop",
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(0)
        .mount(&server)
        .await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    let user = create_principal_with_permissions(&pool, "catalog:read, governance:write").await;
    in_the_catalog_tenant(&pool, user).await;
    narrow(&pool, user, &[("finance", "*")]).await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, body) = call(&router, "POST", RESTORE, &cookie).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], json!("Asset not found"));
}

/// `DATA-55` D13: the list is the caller's tenants' unexpired drops, and a row
/// the caller's roles may not read is not shown at all.
#[tokio::test]
async fn the_dropped_list_shows_the_callers_tenants_unexpired_tables_they_may_read_with_when_each_expires()
 {
    let server = MockServer::start().await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    let tenant_id = Uuid::parse_str(CATALOG_TENANT).expect("a tenant id");
    let other_tenant =
        Uuid::parse_str("11111111-1111-4111-8111-000000000002").expect("a tenant id");
    for (table, tenant, asset, rows) in [
        ("orders", tenant_id, "orders", Some(12)),
        ("expired_t", tenant_id, "expired-t", Some(1)),
        ("other_t", other_tenant, "other-t", None),
        ("hidden_t", tenant_id, "hidden-t", Some(3)),
    ] {
        uploads::record_table_drop(
            &pool,
            &NewTableDrop {
                bronze_table: table,
                tenant_id: tenant,
                asset_id: asset,
                dropped_by: "Ada Lovelace",
                row_count: rows,
                kept_seconds: 604_800,
            },
        )
        .await
        .unwrap()
        .expect("the drop is recorded");
    }
    sqlx::query(
        "UPDATE upload_table_drop SET expires_at = now() - interval '1 second' \
         WHERE bronze_table = 'expired_t'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let all = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "GET", "/api/catalog/dropped/tables", &all).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let tables = body["tables"].as_array().expect("a list");
    let names: Vec<&str> = tables
        .iter()
        .map(|t| t["table"].as_str().expect("a table name"))
        .collect();
    assert_eq!(names.len(), 2, "the expired and the other tenant's: {body}");
    assert!(names.contains(&"bronze.orders"));
    assert!(names.contains(&"bronze.hidden_t"));
    let orders = tables
        .iter()
        .find(|t| t["table"] == json!("bronze.orders"))
        .expect("orders is listed");
    assert_eq!(orders["assetId"], json!("orders"));
    assert_eq!(orders["droppedBy"], json!("Ada Lovelace"));
    assert_eq!(orders["rowCount"], json!(12));
    let expires = time::OffsetDateTime::parse(
        orders["expiresAt"].as_str().expect("an expiresAt"),
        &time::format_description::well_known::Rfc3339,
    )
    .expect("RFC 3339 expiresAt");
    assert!(expires > time::OffsetDateTime::now_utc());

    let user = create_principal_with_permissions(&pool, "catalog:read, governance:write").await;
    in_the_catalog_tenant(&pool, user).await;
    narrow(&pool, user, &[("bronze", "orders")]).await;
    let narrowed = session_cookie_for_user(&pool, user).await;
    let (status, body) = call(&router, "GET", "/api/catalog/dropped/tables", &narrowed).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let names: Vec<&str> = body["tables"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|t| t["table"].as_str().expect("a table name"))
        .collect();
    assert_eq!(names, vec!["bronze.orders"], "the hidden row is not shown");
}

/// `DATA-55` D8/D13: a table is listed after its drop and gone from the list
/// after its restore. The asset itself stays in `GET /api/catalog` by design
/// (D12): what a drop removes is the Iceberg table, and the page that restores
/// it lives in the registry.
#[tokio::test]
async fn a_table_is_listed_after_its_drop_and_gone_from_the_list_after_its_restore() {
    let server = MockServer::start().await;
    mount_row_count(&server, 3).await;
    mount_registry(&server, "orders").await;
    mount_drop(&server, ResponseTemplate::new(204), 1).await;
    mount_missing_table(&server).await;
    mount_dropped(&server, "2099-01-01T00:00:00Z").await;
    Mock::given(method("POST"))
        .and(path(
            "/management/v1/warehouse/wh-1/deleted-tabulars/undrop",
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = call(&router, "GET", "/api/catalog/dropped/tables", &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let names: Vec<&str> = body["tables"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|t| t["table"].as_str().expect("a table name"))
        .collect();
    assert_eq!(names, vec!["bronze.orders"]);

    let (status, body) = call(&router, "POST", RESTORE, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = call(&router, "GET", "/api/catalog/dropped/tables", &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["tables"],
        json!([]),
        "the list is empty once the table is back"
    );
}

/// `DATA-55` review R3: the drop writes its record first and asks again whether
/// a load is running second, so a load that started between the two cannot pass
/// its own check while the drop passes its earlier one. The load is marked
/// `ingesting` while the drop waits on the keep, which is provably after
/// `assess` and before the record; the drop is refused with D5's sentence, the
/// catalog is not asked, the record goes, and the trail says `allowed` then
/// `failed`.
#[tokio::test]
async fn a_drop_whose_upload_starts_loading_between_its_checks_is_a_409_and_leaves_no_record() {
    let server = MockServer::start().await;
    mount_row_count(&server, 3).await;
    mount_registry(&server, "orders").await;
    mount_drop(&server, ResponseTemplate::new(204), 0).await;
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    Mock::given(method("POST"))
        .and(path("/management/v1/warehouse/wh-1/delete-profile"))
        .respond_with(HeldKeep {
            entered: std::sync::Mutex::new(entered_tx),
            release: std::sync::Mutex::new(release_rx),
        })
        .with_priority(2)
        .mount(&server)
        .await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    let tenant_id = Uuid::parse_str(CATALOG_TENANT).expect("a tenant id");
    claim(&pool, CATALOG_TENANT, "orders").await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let writer_pool = pool.clone();
    let writer = tokio::spawn(async move {
        tokio::task::spawn_blocking(move || {
            entered_rx
                .recv_timeout(Duration::from_secs(30))
                .expect("the drop asks to write the keep");
        })
        .await
        .expect("the gate did not panic");
        uploads::insert(
            &writer_pool,
            &NewUpload {
                id: "up-loading",
                original_filename: "stock.csv",
                storage_key: "uploads/up-loading/up-loading.csv",
                content_type: "text/csv",
                size_bytes: 8,
                sha256: "",
                uploaded_by: "Seeded Uploader",
                tenant_id,
            },
        )
        .await
        .expect("the upload is written");
        let options = json!({ "encoding": "utf-8", "delimiter": ",", "headerRow": 0 });
        uploads::mark_ingesting(
            &writer_pool,
            "up-loading",
            &options,
            "orders",
            LoadMode::Replace,
            None,
        )
        .await
        .expect("the load is marked")
        .expect("the upload is loadable");
        release_tx.send(()).expect("the drop is waiting");
    });

    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    writer
        .await
        .expect("the load was marked while the drop waited");

    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], json!(LOADING));
    assert_eq!(deletes(&server).await, 0, "the catalog was not asked");
    assert!(
        uploads::dropped_table(&pool, "orders")
            .await
            .unwrap()
            .is_none(),
        "no record is left behind"
    );
    let outcomes: Vec<String> = sqlx::query_scalar(
        "SELECT outcome FROM audit_event WHERE action = 'catalog.drop' ORDER BY at, id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(outcomes, vec!["allowed", "failed"]);
}

/// `DATA-55` D5 row 1 (review R5): the tenant gate is asked first on all three
/// routes. The write is a `403` with the gate's reason; the two reads answer
/// `200` with `supported: false` rather than a bare empty answer; the catalog
/// and the record are never reached.
#[tokio::test]
async fn the_tenant_gate_refuses_the_drop_and_its_two_reads_without_reaching_the_catalog() {
    let server = MockServer::start().await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    let outsider = create_principal_with_permissions(&pool, "catalog:read, governance:write").await;
    let cookie = session_cookie_for_user(&pool, outsider).await;

    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        body["error"],
        json!(format!("permission_denied: {NOT_OWNER}"))
    );

    let (status, body) = call(&router, "GET", DROP, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["supported"], json!(false));
    assert_eq!(body["reason"], json!(NOT_OWNER));

    let (status, body) = call(&router, "GET", "/api/catalog/dropped/tables", &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["supported"], json!(false));
    assert_eq!(body["reason"], json!(NOT_OWNER));
    assert_eq!(body["tables"], json!([]), "and nothing else: {body}");

    assert_eq!(deletes(&server).await, 0, "the catalog was not asked");
}

/// `DATA-55` D5 row 2 (review R5): an id the registry does not hold is the same
/// `404 Asset not found` on the status read and on the write, and nothing is
/// asked of the catalog.
#[tokio::test]
async fn an_unknown_asset_id_is_a_404_on_the_drop_status_and_on_its_post() {
    let server = MockServer::start().await;
    mount_unknown_slug(&server, "nope").await;
    mount_registry(&server, "orders").await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "GET", "/api/catalog/nope/drop", &cookie).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], json!("Asset not found"));

    let (status, body) = call(&router, "POST", "/api/catalog/nope/drop", &cookie).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], json!("Asset not found"));
    assert_eq!(deletes(&server).await, 0, "the catalog was not asked");
}

/// `DATA-55` D5 row 8 (review R5): the warehouse cannot be looked up — an error
/// and an empty list are the same answer, `503` in our own words — and nothing
/// is dropped.
#[tokio::test]
async fn a_drop_whose_warehouse_cannot_be_looked_up_is_503_and_asks_the_catalog_nothing() {
    let failing = MockServer::start().await;
    mount_registry(&failing, "orders").await;
    Mock::given(method("GET"))
        .and(path("/management/v1/warehouse"))
        .respond_with(ResponseTemplate::new(500).set_body_string("planted-upstream-marker"))
        .mount(&failing)
        .await;
    let TestApp { router, pool } = drop_app(&failing, true).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"], json!(UNREACHABLE));
    assert_eq!(deletes(&failing).await, 0, "the catalog was not asked");

    let empty = MockServer::start().await;
    mount_registry(&empty, "orders").await;
    Mock::given(method("GET"))
        .and(path("/management/v1/warehouse"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "warehouses": [] })))
        .mount(&empty)
        .await;
    let TestApp { router, pool } = drop_app(&empty, true).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"], json!(UNREACHABLE));
    assert_eq!(deletes(&empty).await, 0, "the catalog was not asked");
}

/// `DATA-55` D8 step 2 and review R5/R7: a record that cannot be written is the
/// store's own `database error` (never the database's text), the catalog is not
/// asked, and the `allowed` entry gets its `failed` ending.
#[tokio::test]
async fn a_drop_whose_record_cannot_be_written_answers_the_database_error_and_asks_the_catalog_nothing()
 {
    let server = MockServer::start().await;
    mount_row_count(&server, 3).await;
    mount_registry(&server, "orders").await;
    mount_drop(&server, ResponseTemplate::new(204), 0).await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    // The record's own table refuses this name: the insert fails in this test's
    // database, the way a database failure does in a deployment.
    sqlx::query(
        "ALTER TABLE upload_table_drop ADD CONSTRAINT no_orders CHECK (bronze_table <> 'orders')",
    )
    .execute(&pool)
    .await
    .expect("the record table takes the constraint");
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(body["error"], json!("database error"));
    assert_eq!(deletes(&server).await, 0, "the catalog was not asked");
    let outcomes: Vec<String> = sqlx::query_scalar(
        "SELECT outcome FROM audit_event WHERE action = 'catalog.drop' ORDER BY at, id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(outcomes, vec!["allowed", "failed"]);
}

/// `DATA-55` D9 (review R5): two drops of one table sent at the same moment —
/// the primary key of the record decides, so exactly one answers a drop, one is
/// refused, and the catalog is asked once.
#[tokio::test]
async fn two_drops_of_one_table_at_the_same_moment_are_one_drop_and_one_conflict() {
    let server = MockServer::start().await;
    mount_row_count(&server, 3).await;
    mount_registry(&server, "orders").await;
    mount_drop(&server, ResponseTemplate::new(204), 1).await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let first = call(&router, "POST", DROP, &cookie);
    let second = call(&router, "POST", DROP, &cookie);
    let ((one, one_body), (two, two_body)) = tokio::join!(first, second);
    let statuses = [one, two];
    let bodies = [one_body, two_body];

    let dropped = statuses.iter().filter(|s| **s == StatusCode::OK).count();
    let refused = statuses
        .iter()
        .filter(|s| **s == StatusCode::CONFLICT)
        .count();
    assert_eq!((dropped, refused), (1, 1), "{statuses:?} {bodies:?}");
    assert_eq!(deletes(&server).await, 1, "the catalog was asked once");
}

/// `DATA-55` D7 (review R5): a table whose rows cannot be counted is still
/// dropped, and `null` is what the answer, the record and both audit entries
/// say — never 0, which is a real answer for an empty table.
#[tokio::test]
async fn a_drop_whose_rows_cannot_be_counted_still_drops_with_a_null_row_count() {
    let server = MockServer::start().await;
    mount_count_unreadable(&server, "planted-upstream-marker").await;
    mount_registry(&server, "orders").await;
    mount_drop(&server, ResponseTemplate::new(204), 1).await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    claim(&pool, CATALOG_TENANT, "orders").await;
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "POST", DROP, &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["rowCount"], Value::Null);
    let recorded: Option<i64> =
        sqlx::query_scalar("SELECT row_count FROM upload_table_drop WHERE bronze_table = 'orders'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(recorded, None, "the record says not counted");
    let counted: Vec<Option<Value>> = sqlx::query_scalar(
        "SELECT args -> 'rowCount' FROM audit_event WHERE action = 'catalog.drop' ORDER BY at, id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(counted.len(), 2);
    for entry in counted {
        assert_eq!(entry, Some(Value::Null), "both entries say not counted");
    }
}

/// `DATA-55` D13 (review R5): an unrestricted caller in no tenant is shown
/// nothing — the tenant rule has no "every tenant" reading for this list — even
/// when its own tables are on record.
#[tokio::test]
async fn the_dropped_list_of_an_unrestricted_caller_in_no_tenant_is_empty() {
    let server = MockServer::start().await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    let tenant_id = Uuid::parse_str(CATALOG_TENANT).expect("a tenant id");
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
    let cookie = admin_cookie(&pool).await;

    let (status, body) = call(&router, "GET", "/api/catalog/dropped/tables", &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["tables"], json!([]), "nothing is shown: {body}");
}

/// `DATA-55` D13 (review R5): the list stops at 100 rows, newest first.
#[tokio::test]
async fn the_dropped_list_stops_at_one_hundred_rows() {
    let server = MockServer::start().await;
    let TestApp { router, pool } = drop_app(&server, true).await;
    let tenant_id = Uuid::parse_str(CATALOG_TENANT).expect("a tenant id");
    for n in 0..101 {
        uploads::record_table_drop(
            &pool,
            &NewTableDrop {
                bronze_table: &format!("t_{n:03}"),
                tenant_id,
                asset_id: &format!("t-{n:03}"),
                dropped_by: "Ada Lovelace",
                row_count: None,
                kept_seconds: 604_800,
            },
        )
        .await
        .unwrap()
        .expect("the drop is recorded");
    }
    let cookie = governor(&pool, "catalog:read, governance:write").await;

    let (status, body) = call(&router, "GET", "/api/catalog/dropped/tables", &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["tables"].as_array().expect("a list").len(),
        100,
        "at most 100 rows: {body}"
    );
}
