//! Every `/api/connectors/{id}/*` route answers only for a connector in one
//! of the caller's tenants (`routes::connectors::require_connector_in_tenants`).
//! Before it, `connector:manage` in one tenant reached every tenant's
//! connectors by id.
//!
//! Seeded fixtures (`0002_seed_identity.sql`, `0042_tenant_provisioning.sql`):
//! `conn-pg-lakehouse` belongs to Meridian Group; Bayu (Data Engineer) is in
//! Meridian Group, Andi (Data Engineer) only in Meridian Retail, and Fajar
//! (Platform Admin) in all three.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use common::{TestApp, session_cookie_for_seeded_user, spin_up};

const GROUP_CONNECTOR: &str = "conn-pg-lakehouse";
const MERIDIAN_GROUP: &str = "11111111-1111-4111-8111-000000000001";

async fn send(
    app: &TestApp,
    cookie: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie);
    if body.is_some() {
        request = request.header("content-type", "application/json");
    }
    let response = app
        .router
        .clone()
        .oneshot(
            request
                .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
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

async fn connector_name(app: &TestApp, id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT name FROM connector WHERE id = $1")
        .bind(id)
        .fetch_optional(&app.pool)
        .await
        .expect("read connector")
}

/// Another tenant's connector answers exactly like one that does not
/// exist, on every kind of route — reads, writes, runs and deletes — and
/// nothing is changed.
#[tokio::test]
async fn another_tenants_connector_is_not_found_on_every_route() {
    let app = spin_up().await;
    let andi = session_cookie_for_seeded_user(&app.pool, "andi@meridian.example").await;
    let before = connector_name(&app, GROUP_CONNECTOR).await;

    let (unknown_status, unknown_body) = send(
        &app,
        &andi,
        "GET",
        "/api/connectors/conn-does-not-exist",
        None,
    )
    .await;
    assert_eq!(unknown_status, StatusCode::NOT_FOUND);

    let (status, body) = send(
        &app,
        &andi,
        "GET",
        &format!("/api/connectors/{GROUP_CONNECTOR}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Same wording as the unknown id, bar the id itself: no oracle.
    assert_eq!(
        body["error"]
            .as_str()
            .map(|m| m.replace(GROUP_CONNECTOR, "ID")),
        unknown_body["error"]
            .as_str()
            .map(|m| m.replace("conn-does-not-exist", "ID")),
    );

    for (method, path, body) in [
        ("PATCH", "", Some(json!({ "name": "taken over" }))),
        ("GET", "/ingest-spec", None),
        (
            "PUT",
            "/credential",
            Some(json!({ "primary": { "kind": "password", "value": "x" } })),
        ),
        ("POST", "/test", None),
        ("GET", "/probe-history", None),
        ("POST", "/ingest/run", None),
        ("DELETE", "", None),
    ] {
        let uri = format!("/api/connectors/{GROUP_CONNECTOR}{path}");
        let (status, _) = send(&app, &andi, method, &uri, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}");
    }
    assert_eq!(
        connector_name(&app, GROUP_CONNECTOR).await,
        before,
        "untouched and not deleted"
    );
}

/// A member of the connector's tenant still reaches it.
#[tokio::test]
async fn a_member_of_the_connectors_tenant_reaches_it() {
    let app = spin_up().await;
    let bayu = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    let (status, body) = send(
        &app,
        &bayu,
        "GET",
        &format!("/api/connectors/{GROUP_CONNECTOR}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["id"], GROUP_CONNECTOR);
}

/// A connector with no tenant is reachable by nobody until identity
/// administration assigns one — `PUT .../tenant` itself stays reachable.
#[tokio::test]
async fn an_unassigned_connector_is_reachable_once_assigned() {
    let app = spin_up().await;
    sqlx::query("UPDATE connector SET tenant_id = NULL WHERE id = $1")
        .bind(GROUP_CONNECTOR)
        .execute(&app.pool)
        .await
        .expect("unassign");
    let bayu = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    let fajar = session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;
    let uri = format!("/api/connectors/{GROUP_CONNECTOR}");

    assert_eq!(
        send(&app, &bayu, "GET", &uri, None).await.0,
        StatusCode::NOT_FOUND
    );

    let (status, body) = send(
        &app,
        &fajar,
        "PUT",
        &format!("{uri}/tenant"),
        Some(json!({ "tenantId": MERIDIAN_GROUP })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(send(&app, &bayu, "GET", &uri, None).await.0, StatusCode::OK);
}

/// The copilot's connector tools call the handlers without the router, so
/// they apply the same rule themselves: another tenant's connector is not
/// found there either.
#[tokio::test]
async fn the_copilot_cannot_reach_another_tenants_connector() {
    let app = spin_up().await;
    let andi = session_cookie_for_seeded_user(&app.pool, "andi@meridian.example").await;
    let bayu = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    let call = json!({
        "tool": "test_connector",
        "args": { "id": GROUP_CONNECTOR, "confirmed": true },
        "mode": "build",
    });

    let (status, body) = send(&app, &andi, "POST", "/api/ai/tool", Some(call.clone())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let refused = body["result"]["error"].as_str().unwrap_or_default();
    assert!(refused.contains("not found"), "{body}");

    // Its own tenant's member gets past the rule (whatever the probe says).
    let (_, body) = send(&app, &bayu, "POST", "/api/ai/tool", Some(call)).await;
    let error = body["result"]["error"].as_str().unwrap_or_default();
    assert!(!error.contains("not found"), "{body}");
}
