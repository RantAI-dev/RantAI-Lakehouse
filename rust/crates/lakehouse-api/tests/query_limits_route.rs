//! `GET`/`PUT /api/settings/limits` over the real router (`BI-5`, T1): the
//! defaults before anything is saved, the saved values and the caller's
//! effective ones (install, then a longer role limit), the sentence a value
//! outside its range gets, and that a save is seen at once by the process
//! that handled it (the memo is cleared).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn call(app: &axum::Router, verb: &str, cookie: &str, body: &str) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(verb)
                .uri("/api/settings/limits")
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

#[tokio::test]
async fn nothing_saved_reads_the_feature_defaults_with_the_install_as_source() {
    let TestApp { router, pool } = spin_up().await;
    let user = create_principal_with_permissions(&pool, "dashboard:read").await;
    let cookie = session_cookie_for_user(&pool, user).await;
    let (status, body) = call(&router, "GET", &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    // BI-5 review S6: a reader without settings:write is told what applies to
    // them, not the install's numbers or which roles have a limit of their own.
    assert_eq!(
        body,
        json!({
            "effective": {
                "timeoutSeconds": 60, "chartRows": 2000, "mapPoints": 5000, "studioRows": 2000,
                "source": {"kind": "install"}
            }
        })
    );
    // The administrator's read adds both.
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let (status, full) = call(&router, "GET", &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        full["install"],
        json!({"timeoutSeconds": 60, "chartRows": 2000, "mapPoints": 5000, "studioRows": 2000})
    );
    assert_eq!(full["roles"], json!([]));
    assert_eq!(full["effective"], body["effective"]);
}

#[tokio::test]
async fn a_save_is_read_back_at_once_and_a_role_limit_shows_as_the_callers_source() {
    let TestApp { router, pool } = spin_up().await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    // Warm the memo with the defaults, then prove a PUT clears it.
    assert_eq!(call(&router, "GET", &cookie, "").await.0, StatusCode::OK);

    let role: Uuid = sqlx::query_scalar(
        "SELECT r.id FROM role r JOIN app_user_role ur ON ur.role_id = r.id \
         WHERE ur.user_id = $1 LIMIT 1",
    )
    .bind(admin)
    .fetch_one(&pool)
    .await
    .unwrap();
    let role_name: String = sqlx::query_scalar("SELECT name FROM role WHERE id = $1")
        .bind(role)
        .fetch_one(&pool)
        .await
        .unwrap();
    let put = json!({
        "install": {"timeoutSeconds": 30, "chartRows": 500, "mapPoints": 1000, "studioRows": 4000},
        "roles": [{"roleId": role, "timeoutSeconds": 300}]
    });
    let (status, saved) = call(&router, "PUT", &cookie, &put.to_string()).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["install"], put["install"]);
    assert_eq!(saved["roles"], put["roles"]);
    assert_eq!(
        saved["effective"],
        json!({
            "timeoutSeconds": 300, "chartRows": 500, "mapPoints": 1000, "studioRows": 4000,
            "source": {"kind": "role", "name": role_name}
        })
    );
    let (_, again) = call(&router, "GET", &cookie, "").await;
    assert_eq!(again, saved, "the memo was cleared by the PUT");
}

#[tokio::test]
async fn a_value_outside_its_range_a_missing_role_and_a_bad_shape_are_400_and_store_nothing() {
    let TestApp { router, pool } = spin_up().await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let install =
        json!({"timeoutSeconds": 9, "chartRows": 2000, "mapPoints": 5000, "studioRows": 2000});
    let (status, body) = call(
        &router,
        "PUT",
        &cookie,
        &json!({"install": install, "roles": []}).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["error"],
        "Time limit: a whole number from 10 to 1800 seconds."
    );

    let ok =
        json!({"timeoutSeconds": 60, "chartRows": 2000, "mapPoints": 5000, "studioRows": 2000});
    let (status, body) = call(
        &router,
        "PUT",
        &cookie,
        &json!({"install": ok, "roles": [{"roleId": Uuid::new_v4(), "timeoutSeconds": 60}]})
            .to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "The role limits must name roles that exist.");

    let (status, _) = call(&router, "PUT", &cookie, "not json").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let stored: i32 = sqlx::query_scalar("SELECT timeout_seconds FROM query_limits")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 60);
}
