//! `PUT /api/identity/users/{id}/tenants/{tenant_id}` — giving an existing
//! user a tenant.
//!
//! Before it, a user could join a tenant only when invited
//! (`POST /api/identity/users`), so the bootstrap admin, seeded with no
//! tenant, could never get one and every connector it created was
//! unreachable by its own per-connector routes.
//!
//! Seeded fixtures (`0002_seed_identity.sql`): Meridian Group and Meridian
//! Retail tenants; Fajar is Platform Admin (`*:*`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

use common::{TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up};

const MERIDIAN_GROUP: &str = "11111111-1111-4111-8111-000000000001";
const MERIDIAN_RETAIL: &str = "11111111-1111-4111-8111-000000000002";

async fn put(app: &TestApp, cookie: &str, uri: &str) -> (StatusCode, Value) {
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(uri)
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn get(app: &TestApp, cookie: &str, uri: &str) -> Value {
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(response.status(), StatusCode::OK, "GET {uri}");
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("JSON body")
}

async fn membership_rows(app: &TestApp, user_id: Uuid, tenant_id: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM app_user_tenant WHERE user_id = $1 AND tenant_id = $2::uuid",
    )
    .bind(user_id)
    .bind(tenant_id)
    .fetch_one(&app.pool)
    .await
    .expect("count membership rows")
}

fn route(user_id: Uuid, tenant_id: &str) -> String {
    format!("/api/identity/users/{user_id}/tenants/{tenant_id}")
}

/// The bootstrap admin's case: an unrestricted caller that belongs to no
/// tenant adds itself to one, and its very next request, on the session it
/// already holds, lists that tenant.
#[tokio::test]
async fn an_unrestricted_caller_with_no_tenant_can_add_itself_and_sees_it_on_its_next_request() {
    let app = spin_up().await;
    let admin = create_principal_with_permissions(&app.pool, "*:*").await;
    let cookie = session_cookie_for_user(&app.pool, admin).await;

    let before = get(&app, &cookie, "/api/auth/me").await;
    assert_eq!(before["tenants"].as_array().map(Vec::len), Some(0));

    let (status, body) = put(&app, &cookie, &route(admin, MERIDIAN_GROUP)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["tenants"], serde_json::json!(["Meridian Group"]));

    let after = get(&app, &cookie, "/api/auth/me").await;
    let ids: Vec<&str> = after["tenants"]
        .as_array()
        .expect("tenants array")
        .iter()
        .filter_map(|t| t["id"].as_str())
        .collect();
    assert_eq!(ids, vec![MERIDIAN_GROUP]);
}

/// Adding a member again is a success, leaves one row and records one
/// audit event, not two.
#[tokio::test]
async fn adding_the_same_member_twice_succeeds_leaves_one_row_and_one_audit_event() {
    let app = spin_up().await;
    let admin = create_principal_with_permissions(&app.pool, "*:*").await;
    let target = create_principal_with_permissions(&app.pool, "query:read").await;
    let cookie = session_cookie_for_user(&app.pool, admin).await;

    for _ in 0..2 {
        let (status, body) = put(&app, &cookie, &route(target, MERIDIAN_RETAIL)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["tenants"], serde_json::json!(["Meridian Retail"]));
    }
    assert_eq!(membership_rows(&app, target, MERIDIAN_RETAIL).await, 1);

    let rows: Vec<(String, Option<String>, String, Option<Value>)> = sqlx::query_as(
        "SELECT action, resource_id, outcome, args FROM audit_event \
         WHERE action = 'identity.user.tenant.add' AND resource_id = $1",
    )
    .bind(target.to_string())
    .fetch_all(&app.pool)
    .await
    .expect("read audit rows");
    assert_eq!(
        rows.len(),
        1,
        "only the add that changed something is recorded"
    );
    assert_eq!(rows[0].2, "executed");
    assert_eq!(
        rows[0].3.as_ref().and_then(|a| a["tenantId"].as_str()),
        Some(MERIDIAN_RETAIL)
    );
}

/// A caller that is neither unrestricted nor a member of the tenant gets
/// the unknown-tenant answer, and no row is written.
#[tokio::test]
async fn a_caller_who_is_not_in_the_tenant_and_not_unrestricted_gets_404_and_no_row() {
    let app = spin_up().await;
    let caller = create_principal_with_permissions(&app.pool, "identity:write").await;
    let target = create_principal_with_permissions(&app.pool, "query:read").await;
    let cookie = session_cookie_for_user(&app.pool, caller).await;

    let (status, body) = put(&app, &cookie, &route(target, MERIDIAN_GROUP)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (unknown_status, unknown_body) =
        put(&app, &cookie, &route(target, &Uuid::new_v4().to_string())).await;
    assert_eq!(unknown_status, StatusCode::NOT_FOUND);
    assert_eq!(body, unknown_body, "no oracle for which tenants exist");
    assert_eq!(membership_rows(&app, target, MERIDIAN_GROUP).await, 0);
}

/// A member of a tenant who holds `identity:write` but not `*:*` may add
/// someone to that tenant, and still not to another one.
#[tokio::test]
async fn a_member_may_add_to_their_own_tenant_but_not_to_another() {
    let app = spin_up().await;
    let caller = create_principal_with_permissions(&app.pool, "identity:write").await;
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2::uuid)")
        .bind(caller)
        .bind(MERIDIAN_GROUP)
        .execute(&app.pool)
        .await
        .expect("join the tenant");
    let target = create_principal_with_permissions(&app.pool, "query:read").await;
    let cookie = session_cookie_for_user(&app.pool, caller).await;

    let (status, body) = put(&app, &cookie, &route(target, MERIDIAN_GROUP)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(membership_rows(&app, target, MERIDIAN_GROUP).await, 1);

    let (status, _) = put(&app, &cookie, &route(target, MERIDIAN_RETAIL)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(membership_rows(&app, target, MERIDIAN_RETAIL).await, 0);
}

/// An unknown user and an unknown tenant are 404; a malformed id is 400.
#[tokio::test]
async fn an_unknown_user_or_tenant_is_404_and_a_malformed_id_is_400() {
    let app = spin_up().await;
    let admin = create_principal_with_permissions(&app.pool, "*:*").await;
    let cookie = session_cookie_for_user(&app.pool, admin).await;

    let (status, _) = put(&app, &cookie, &route(Uuid::new_v4(), MERIDIAN_GROUP)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown user");

    let (status, _) = put(&app, &cookie, &route(admin, &Uuid::new_v4().to_string())).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown tenant");

    let (status, _) = put(
        &app,
        &cookie,
        &format!("/api/identity/users/not-a-uuid/tenants/{MERIDIAN_GROUP}"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "malformed user id");

    let (status, _) = put(
        &app,
        &cookie,
        &format!("/api/identity/users/{admin}/tenants/not-a-uuid"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "malformed tenant id");
}
