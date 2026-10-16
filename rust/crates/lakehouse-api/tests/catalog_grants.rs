//! Grant and revoke from an asset's page over the real router (`DATA-15`
//! F3): who may, what is refused with a fixed sentence, that a grant gives
//! the permission and a revoke takes it away at the next principal load,
//! and that the asset's history names both people.
//!
//! The caller is a principal holding `*:*`: it passes the catalog tenant
//! gate (an unrestricted principal is never refused) and holds
//! `access:approve`. The grantee is a zero-permission user.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_principal_with_permissions, create_zero_permission_principal,
    session_cookie_for_user, spin_up,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const ASSET: &str = "silver.orders";

async fn call(
    app: &axum::Router,
    verb: &str,
    path: &str,
    cookie: &str,
    body: &str,
) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(verb)
                .uri(path)
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_owned()))
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

fn grants_path() -> String {
    format!("/api/catalog/{ASSET}/grants")
}

async fn email_of(pool: &PgPool, user: Uuid) -> String {
    sqlx::query_scalar("SELECT email FROM app_user WHERE id = $1")
        .bind(user)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// An admin cookie and a zero-permission grantee with their e-mail.
async fn admin_and_grantee(pool: &PgPool) -> (String, Uuid, Uuid, String) {
    let admin = create_principal_with_permissions(pool, "*:*").await;
    let grantee = create_zero_permission_principal(pool).await;
    let email = email_of(pool, grantee).await;
    (
        session_cookie_for_user(pool, admin).await,
        admin,
        grantee,
        email,
    )
}

async fn holds(pool: &PgPool, user: Uuid, permission: &str) -> bool {
    lakehouse_auth::repository::load_principal_for_user(pool, user, "local".to_owned(), false)
        .await
        .unwrap()
        .has(permission)
}

#[tokio::test]
async fn a_direct_grant_is_listed_gives_the_permission_and_a_revoke_takes_it_away() {
    let TestApp { router, pool } = spin_up().await;
    let (cookie, _admin, grantee, email) = admin_and_grantee(&pool).await;
    assert!(!holds(&pool, grantee, "query:read").await);

    let (status, made) = call(
        &router,
        "POST",
        &grants_path(),
        &cookie,
        &json!({"email": email, "permission": "query:read"}).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{made}");
    let grant_id = made["id"].as_str().expect("the grant id").to_owned();
    assert!(holds(&pool, grantee, "query:read").await);

    let (status, listed) = call(&router, "GET", &grants_path(), &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    let grants = listed["grants"].as_array().expect("a list");
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0]["id"], json!(grant_id));
    assert_eq!(grants[0]["permission"], json!("query:read"));
    assert_eq!(grants[0]["via"], json!("direct"));
    assert_eq!(grants[0]["userEmail"], json!(email));
    assert_eq!(grants[0]["grantedBy"], json!("Single Perm Test User"));

    let (status, _) = call(
        &router,
        "DELETE",
        &format!("{}/{grant_id}", grants_path()),
        &cookie,
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // A revoked grant no longer reaches the principal the next request loads.
    assert!(!holds(&pool, grantee, "query:read").await);
    let (_, listed) = call(&router, "GET", &grants_path(), &cookie, "").await;
    assert_eq!(listed["grants"], json!([]));

    // Already revoked: 404, with the fixed sentence.
    let (status, again) = call(
        &router,
        "DELETE",
        &format!("{}/{grant_id}", grants_path()),
        &cookie,
        "",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(again["error"], json!("no such grant in force"));
}

#[tokio::test]
async fn a_grant_and_a_revoke_are_audited_and_named_in_the_assets_history() {
    let TestApp { router, pool } = spin_up().await;
    let (cookie, _admin, _grantee, email) = admin_and_grantee(&pool).await;
    let (_, made) = call(
        &router,
        "POST",
        &grants_path(),
        &cookie,
        &json!({"email": email, "permission": "lineage:read"}).to_string(),
    )
    .await;
    let grant_id = made["id"].as_str().unwrap().to_owned();
    call(
        &router,
        "DELETE",
        &format!("{}/{grant_id}", grants_path()),
        &cookie,
        "",
    )
    .await;

    let actions: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM audit_event WHERE resource_kind = 'catalog' AND resource_id = $1 \
         ORDER BY at, id",
    )
    .bind(ASSET)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(actions, vec!["catalog.grant", "catalog.revoke"]);
    let args: Value = sqlx::query_scalar(
        "SELECT args FROM audit_event WHERE action = 'catalog.grant' AND resource_id = $1",
    )
    .bind(ASSET)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(args["permission"], json!("lineage:read"));
    assert_eq!(args["grantee"], json!("Zero Perm Test User"));
    assert!(args["expiresAt"].is_string());
}

#[tokio::test]
async fn every_refusal_has_its_status_and_a_fixed_sentence_and_stores_nothing() {
    let TestApp { router, pool } = spin_up().await;
    let (cookie, admin, grantee, email) = admin_and_grantee(&pool).await;
    let admin_email = email_of(&pool, admin).await;

    let post = |body: Value| {
        let router = router.clone();
        let cookie = cookie.clone();
        async move { call(&router, "POST", &grants_path(), &cookie, &body.to_string()).await }
    };

    let (status, body) = call(&router, "POST", &grants_path(), &cookie, "not json").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["error"],
        json!("body must be JSON with an \"email\" and a \"permission\"")
    );

    let (status, body) = post(json!({"email": email, "permission": "*:*"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["error"],
        json!("permission must be catalog:write, lineage:read or query:read")
    );

    let (status, body) = post(json!({"email": "  ", "permission": "query:read"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], json!("enter the e-mail of a user"));

    let (status, body) =
        post(json!({"email": "nobody@nowhere.invalid", "permission": "query:read"})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], json!("no user with that email"));

    let (status, body) = post(json!({"email": admin_email, "permission": "query:read"})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        body["error"],
        json!("permission_denied: you cannot grant access to yourself")
    );

    // Granting twice: the second finds the user already holds it.
    let (status, _) = post(json!({"email": email, "permission": "query:read"})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = post(json!({"email": email, "permission": "query:read"})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        body["error"],
        json!("that user already holds this permission")
    );

    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM access_grant WHERE user_id = $1")
        .bind(grantee)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 1, "only the one successful grant is stored");
}

#[tokio::test]
async fn a_revoke_from_another_assets_page_finds_no_such_grant() {
    let TestApp { router, pool } = spin_up().await;
    let (cookie, _admin, grantee, email) = admin_and_grantee(&pool).await;
    let (_, made) = call(
        &router,
        "POST",
        &grants_path(),
        &cookie,
        &json!({"email": email, "permission": "query:read"}).to_string(),
    )
    .await;
    let grant_id = made["id"].as_str().unwrap();
    let (status, _) = call(
        &router,
        "DELETE",
        &format!("/api/catalog/silver.other/grants/{grant_id}"),
        &cookie,
        "",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(holds(&pool, grantee, "query:read").await);
}

#[tokio::test]
async fn a_user_who_already_has_the_permission_through_a_role_is_refused() {
    let TestApp { router, pool } = spin_up().await;
    let admin = create_principal_with_permissions(&pool, "*:*").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let reader = create_principal_with_permissions(&pool, "query:read").await;
    let email = email_of(&pool, reader).await;
    let (status, body) = call(
        &router,
        "POST",
        &grants_path(),
        &cookie,
        &json!({"email": email, "permission": "query:read"}).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

#[tokio::test]
async fn a_caller_who_passes_the_permission_but_not_the_tenant_gate_is_refused_before_the_body_is_read()
 {
    // `access:approve` without `*:*` and without the catalog tenant: the
    // seeded app has several tenants and no `CATALOG_TENANT_ID`.
    let TestApp { router, pool } = spin_up().await;
    let approver = create_principal_with_permissions(&pool, "access:approve").await;
    let cookie = session_cookie_for_user(&pool, approver).await;
    for (verb, path) in [
        ("GET", grants_path()),
        ("POST", grants_path()),
        ("DELETE", format!("{}/grant-x", grants_path())),
    ] {
        let (status, body) = call(&router, verb, &path, &cookie, "not json").await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{verb} {path}");
        assert!(
            body["error"]
                .as_str()
                .is_some_and(|e| e.starts_with("permission_denied: per-dataset tenant ownership")),
            "{verb} {path}: {body}"
        );
    }
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM access_grant")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 0);
}

#[tokio::test]
async fn without_access_approve_every_grant_route_is_refused_by_the_permission_gate() {
    let TestApp { router, pool } = spin_up().await;
    let user = create_principal_with_permissions(&pool, "catalog:read, catalog:write").await;
    let cookie = session_cookie_for_user(&pool, user).await;
    for (verb, path) in [
        ("GET", grants_path()),
        ("POST", grants_path()),
        ("DELETE", format!("{}/grant-x", grants_path())),
    ] {
        let (status, body) = call(&router, verb, &path, &cookie, "{}").await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{verb} {path}");
        assert_eq!(body["error"], json!("permission_denied: access:approve"));
    }
}
