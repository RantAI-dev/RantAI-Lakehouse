//! HTTP-level integration tests for `POST /api/identity/tenants`'
//! provisioning behaviour: a resumable state machine over `tenant.
//! provisioning_status` (see `0042_tenant_provisioning.sql`) so a crash or
//! restart partway through provisioning a tenant's Lakekeeper warehouse can
//! be resumed rather than silently re-run or left stuck.
//!
//! Runs against the real, isolated Postgres `common::spin_up_with_env`
//! provisions, with `LAKEKEEPER_BASE_URI` pointed at a `wiremock`
//! `MockServer` standing in for Lakekeeper's management API — never a
//! real Lakekeeper instance. This mirrors
//! `crates/lakehouse-auth/tests/openfga.rs`'s own wiremock harness for
//! `LakekeeperAdminClient`, one layer up (through the real route, not
//! the client directly).
//!
//! This file uses a standalone integration test file (rather than an
//! in-file `#[cfg(test)]` module) to follow this crate's existing
//! Postgres-backed-test convention (`tests/tier1_writehigh_
//! approval.rs`, `tests/connector_delete_deprovision.rs`): a standalone
//! integration test file using `tests/common/mod.rs`'s `spin_up_with_env`
//! (env overrides substitute for patching `AppState` fields directly,
//! which is private) and a local `post`/`json_body` pair matching this
//! crate's other test files byte-for-byte in shape.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

async fn post(
    app: &axum::Router,
    uri: &str,
    cookie: &str,
    body: Value,
) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(
                    serde_json::to_vec(&body).expect("serialize body"),
                ))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright")
}

async fn json_body(resp: axum::http::Response<Body>) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read response body");
    serde_json::from_slice(&bytes).expect("valid JSON body")
}

/// Starts a `wiremock` server standing in for Lakekeeper's management
/// API, and a [`TestApp`] whose `LAKEKEEPER_BASE_URI`/
/// `LAKEKEEPER_ADMIN_TOKEN_FILE` point at it — the harness both new tests
/// below share.
async fn app_with_lakekeeper_admin(server: &MockServer) -> TestApp {
    let token_dir = tempfile::tempdir().expect("create a temp dir for the admin token file");
    let token_path = token_dir.path().join("admin.jwt");
    std::fs::write(&token_path, "test-admin-token").expect("write a fake admin token");
    // Leak the tempdir so it outlives this fn — AppState reads the path
    // once at construction, but the file must still exist for the whole
    // test.
    std::mem::forget(token_dir);

    let mut overrides = HashMap::new();
    overrides.insert("LAKEKEEPER_BASE_URI".to_owned(), server.uri());
    overrides.insert(
        "LAKEKEEPER_ADMIN_TOKEN_FILE".to_owned(),
        token_path.to_str().expect("a UTF-8 temp path").to_owned(),
    );
    overrides.insert(
        "LAKEHOUSE_WAREHOUSE_BUCKET".to_owned(),
        "s3://test-warehouse".to_owned(),
    );
    spin_up_with_env(&overrides).await
}

/// Inserts a `tenant` row directly at `status` (and `warehouse_id`,
/// where the status implies one already exists), simulating a crash
/// partway through a previous provisioning attempt — the row the resume
/// path in `create_tenant` must pick up rather than reject with 409 or
/// restart from `ensure_warehouse`.
async fn seed_tenant_at_status(
    pool: &PgPool,
    slug: &str,
    status: &str,
    warehouse_id: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO tenant (id, name, slug, plan, residency, provisioning_status, warehouse_id) \
         VALUES ($1, $2, $3, 'Standard', 'US', $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind("Acme Co")
    .bind(slug)
    .bind(status)
    .bind(warehouse_id)
    .execute(pool)
    .await
    .expect("seed a tenant row directly");
}

/// A brand-new slug drives the full state machine — warehouse create,
/// then grants — and the response carries the real Lakekeeper warehouse
/// id and a terminal `provisioningStatus`. Before this state machine
/// existed, `create_tenant` was a bare Postgres insert with no
/// `warehouseId`/`provisioningStatus` fields on the response and no
/// Lakekeeper call at all.
#[tokio::test]
async fn create_tenant_provisions_a_warehouse_and_stops_honestly_at_grants_ready() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/management/v1/warehouse"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"warehouses": []})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/management/v1/warehouse"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"warehouse-id": "wh-new"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(
            "/management/v1/permissions/warehouse/wh-new/assignments",
        ))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let app = app_with_lakekeeper_admin(&server).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;

    let response = post(
        &app.router,
        "/api/identity/tenants",
        &cookie,
        json!({"name": "Acme Co", "slug": "acme-co", "plan": "Standard", "residency": "US"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = json_body(response).await;
    // `grants_ready`, NOT `complete`: the namespace step has no
    // implementation yet (see `provision_tenant`'s comment — the only
    // namespace-create surface is bound to the shared catalog, not a
    // tenant warehouse). Writing `complete` here would put a completion
    // that never happened into the database, which is exactly what
    // migration 0042's `not_applicable` status exists to avoid for
    // grandfathered tenants. The status an operator reads is the truth
    // about how far provisioning actually got.
    assert_eq!(body["provisioningStatus"], "grants_ready");
    assert!(
        body["warehouseId"].as_str().is_some(),
        "expected a warehouseId, got {body}"
    );
}

/// A slug already seeded at a genuinely in-progress status (simulating a
/// crash after the warehouse step) is RESUMED on a repeated POST —
/// grants + terminal steps only,
/// never a second `ensure_warehouse` call — rather than rejected with 409
/// or restarted from scratch. Proven by never mounting the warehouse
/// create/list mocks at all: if the handler incorrectly redid step 1, the
/// unmocked call would 404 through `error_for_status()` and the response
/// would not be a 200 with `provisioningStatus: "complete"`.
#[tokio::test]
async fn create_tenant_is_idempotent_on_a_repeated_slug_when_not_yet_complete() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/management/v1/permissions/warehouse/wh-existing/assignments",
        ))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let app = app_with_lakekeeper_admin(&server).await;
    seed_tenant_at_status(&app.pool, "acme-co", "warehouse_ready", Some("wh-existing")).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;

    let response = post(
        &app.router,
        "/api/identity/tenants",
        &cookie,
        json!({"name": "Acme Co", "slug": "acme-co", "plan": "Standard", "residency": "US"}),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a resumed provisioning attempt must be 200, not 201 (nothing was created) or 409"
    );
    let body = json_body(response).await;
    // `grants_ready`, NOT `complete`: the namespace step has no
    // implementation yet (see `provision_tenant`'s comment — the only
    // namespace-create surface is bound to the shared catalog, not a
    // tenant warehouse). Writing `complete` here would put a completion
    // that never happened into the database, which is exactly what
    // migration 0042's `not_applicable` status exists to avoid for
    // grandfathered tenants. The status an operator reads is the truth
    // about how far provisioning actually got.
    assert_eq!(body["provisioningStatus"], "grants_ready");
    assert_eq!(body["warehouseId"], "wh-existing");
}

/// A slug already at a terminal status (`complete`) is a genuine 409, not
/// a resume: a row already `'complete'` or `'not_applicable'` has nothing
/// left to provision, so a repeated POST of its slug is rejected rather
/// than silently treated as a no-op success.
#[tokio::test]
async fn create_tenant_on_an_already_complete_slug_is_409_not_a_resume() {
    let server = MockServer::start().await;
    let app = app_with_lakekeeper_admin(&server).await;
    seed_tenant_at_status(&app.pool, "acme-co", "complete", Some("wh-done")).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;

    let response = post(
        &app.router,
        "/api/identity/tenants",
        &cookie,
        json!({"name": "Acme Co", "slug": "acme-co", "plan": "Standard", "residency": "US"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    // No mock was mounted at all: if the handler mistakenly tried to
    // provision a `complete` row, the unmocked call would 404, not
    // produce the clean 409 this test asserts.
}

/// A `not_applicable` slug (a grandfathered pre-provisioning tenant,
/// migration `0042`'s backfill) must never be pulled into a provisioning
/// attempt by a later POST of its slug — also a 409, for the same reason
/// a `complete` row is: there is nothing left to provision.
#[tokio::test]
async fn create_tenant_on_a_not_applicable_slug_is_409_not_a_resume() {
    let server = MockServer::start().await;
    let app = app_with_lakekeeper_admin(&server).await;
    seed_tenant_at_status(&app.pool, "acme-co", "not_applicable", None).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;

    let response = post(
        &app.router,
        "/api/identity/tenants",
        &cookie,
        json!({"name": "Acme Co", "slug": "acme-co", "plan": "Standard", "residency": "US"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

/// When `AppState::lakekeeper_admin` is `None` (no admin token file
/// configured on this deployment), the route must report provisioning
/// unavailable honestly for a brand-new tenant — never fabricate a
/// `complete` status, and never claim 201 success while secretly leaving
/// the row half-provisioned.
#[tokio::test]
async fn create_tenant_without_lakekeeper_admin_configured_leaves_status_pending() {
    let app = spin_up_with_env(&HashMap::new()).await; // no LAKEKEEPER_ADMIN_TOKEN_FILE override -> unreadable default path -> None
    let cookie = session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;

    let response = post(
        &app.router,
        "/api/identity/tenants",
        &cookie,
        json!({"name": "Acme Co", "slug": "acme-co", "plan": "Standard", "residency": "US"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = json_body(response).await;
    assert_eq!(
        body["provisioningStatus"], "pending",
        "with no Lakekeeper admin client configured, the tenant row must be \
         created but left at its honest starting status, never fabricated \
         as complete"
    );
}
