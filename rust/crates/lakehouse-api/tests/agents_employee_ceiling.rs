//! `SEC-20`, plan 97: creating a digital employee needs `agent:manage`, and the
//! employee's permissions never exceed its creator's.
//!
//! A scheduled run takes its `PermissionSet` from the employee's own
//! `permissions` column, and a scheduled run needs no `agent:manage`. Before
//! this plan any signed-in person could therefore create an employee holding
//! permissions they lacked, schedule it, and read what it read. Every test
//! here goes through `POST /api/agents/employees` with a real session and a
//! real Postgres, then reads the `agent_employee` row, because the stored
//! column is what a later run trusts.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_seeded_user,
    session_cookie_for_user, spin_up,
};

/// A valid create body for `name`, asking for `permissions` when given.
fn employee_body(name: &str, permissions: Option<&str>) -> Value {
    let mut body = json!({
        "name": name,
        "purpose": "p",
        "autonomy": "L1",
        "dataScope": "d",
        "budgetLimit": 0,
        "scheduleCron": "0 6 * * *",
    });
    if let Some(permissions) = permissions {
        body["permissions"] = json!(permissions);
    }
    body
}

async fn create_employee(app: &axum::Router, cookie: &str, body: Value) -> (StatusCode, String) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agents/employees")
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .expect("router never fails a request outright");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read response body");
    (
        status,
        String::from_utf8(bytes.to_vec()).expect("utf-8 body"),
    )
}

/// The `permissions` column of the employee named `name`, or `None` when no
/// such employee was stored.
async fn stored_permissions(pool: &PgPool, name: &str) -> Option<String> {
    sqlx::query_scalar("SELECT permissions FROM agent_employee WHERE name = $1")
        .bind(name)
        .fetch_optional(pool)
        .await
        .expect("read the stored employee")
}

#[tokio::test]
async fn creating_an_employee_needs_agent_manage() {
    let TestApp { router, pool } = spin_up().await;

    let analyst = create_principal_with_permissions(&pool, "dashboard:read").await;
    let cookie = session_cookie_for_user(&pool, analyst).await;
    let (status, _) = create_employee(&router, &cookie, employee_body("sec20-denied", None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(stored_permissions(&pool, "sec20-denied").await, None);

    let manager = create_principal_with_permissions(&pool, "agent:manage").await;
    let cookie = session_cookie_for_user(&pool, manager).await;
    let (status, body) =
        create_employee(&router, &cookie, employee_body("sec20-allowed", None)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        stored_permissions(&pool, "sec20-allowed").await.as_deref(),
        Some("")
    );
}

#[tokio::test]
async fn an_employee_cannot_be_given_a_permission_its_creator_lacks() {
    let TestApp { router, pool } = spin_up().await;
    let creator = create_principal_with_permissions(&pool, "agent:manage, dashboard:read").await;
    let cookie = session_cookie_for_user(&pool, creator).await;

    let (status, body) = create_employee(
        &router,
        &cookie,
        employee_body("sec20-widen", Some("query:read")),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("query:read"), "names the permission: {body}");
    assert_eq!(stored_permissions(&pool, "sec20-widen").await, None);

    // One permission held and one not: the refusal names the one not held.
    let (status, body) = create_employee(
        &router,
        &cookie,
        employee_body("sec20-mixed", Some("dashboard:read, query:read")),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("query:read"), "names the permission: {body}");
    assert!(
        !body.contains("dashboard:read"),
        "does not name the one held: {body}"
    );
    assert_eq!(stored_permissions(&pool, "sec20-mixed").await, None);

    let (status, body) = create_employee(
        &router,
        &cookie,
        employee_body("sec20-narrow", Some("dashboard:read")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        stored_permissions(&pool, "sec20-narrow").await.as_deref(),
        Some("dashboard:read")
    );
}

#[tokio::test]
async fn a_wildcard_permission_for_an_employee_needs_a_creator_who_holds_it() {
    let TestApp { router, pool } = spin_up().await;

    let creator = create_principal_with_permissions(&pool, "agent:manage, dashboard:read").await;
    let cookie = session_cookie_for_user(&pool, creator).await;
    for (name, asked) in [("sec20-star", "*:*"), ("sec20-action-star", "dashboard:*")] {
        let (status, body) =
            create_employee(&router, &cookie, employee_body(name, Some(asked))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{asked}: {body}");
        assert_eq!(stored_permissions(&pool, name).await, None, "{asked}");
    }

    // An action wildcard (`dashboard:*`) covers a narrower grant on its own
    // resource, and nothing wider.
    let creator = create_principal_with_permissions(&pool, "agent:manage, dashboard:*").await;
    let cookie = session_cookie_for_user(&pool, creator).await;
    let (status, body) = create_employee(
        &router,
        &cookie,
        employee_body("sec20-covered", Some("dashboard:read")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (status, body) =
        create_employee(&router, &cookie, employee_body("sec20-wider", Some("*:*"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // `*:*` is the Platform Admin's to give.
    let admin = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let (status, body) =
        create_employee(&router, &admin, employee_body("sec20-admin", Some("*:*"))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        stored_permissions(&pool, "sec20-admin").await.as_deref(),
        Some("*:*")
    );
}

/// The ceiling checks the tokens the run will parse. A token that is not
/// `resource:action` grants nothing when parsed, so it would slip past a
/// check of the raw string; the route refuses it instead of storing it.
#[tokio::test]
async fn a_malformed_permission_token_is_refused_and_nothing_is_stored() {
    let TestApp { router, pool } = spin_up().await;
    let creator = create_principal_with_permissions(&pool, "agent:manage, dashboard:read").await;
    let cookie = session_cookie_for_user(&pool, creator).await;

    for (name, asked) in [
        ("sec20-no-colon", "dashboard-read"),
        ("sec20-empty-action", "dashboard:"),
        ("sec20-mixed-bad", "dashboard:read, query"),
    ] {
        let (status, body) =
            create_employee(&router, &cookie, employee_body(name, Some(asked))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{asked}: {body}");
        assert_eq!(stored_permissions(&pool, name).await, None, "{asked}");
    }
}
