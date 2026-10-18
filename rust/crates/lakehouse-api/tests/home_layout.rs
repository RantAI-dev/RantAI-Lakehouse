//! HTTP-level tests for `GET`/`PUT`/`DELETE /api/home/layout`: the round
//! trip, the default (`layout: null`), that two users never see each
//! other's layout, and that a bad body is a 400 with this route's own
//! message. The `RequiresAuth` floor itself is asserted for all three
//! methods by the `POLICY_TABLE` loops in `tests/route_auth.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use common::{create_zero_permission_principal, session_cookie_for_user, spin_up};

async fn send(
    router: &axum::Router,
    method: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri("/api/home/layout")
        .header("content-type", "application/json");
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    let request = builder
        .body(Body::from(body.map_or_else(String::new, |v| v.to_string())))
        .expect("build request");
    let response = router.clone().oneshot(request).await.expect("request");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn layout() -> Value {
    json!({
        "cards": ["sources", "recent"],
        "shortcuts": ["new-query", "connect-source"],
        "previewBoardId": "board-1",
    })
}

#[tokio::test]
async fn a_new_user_has_no_layout_then_save_get_and_reset_round_trip() {
    let app = spin_up().await;
    let user = create_zero_permission_principal(&app.pool).await;
    let cookie = session_cookie_for_user(&app.pool, user).await;

    let (status, body) = send(&app.router, "GET", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "supported": true, "layout": null }));

    let (status, body) = send(&app.router, "PUT", Some(&cookie), Some(layout())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "supported": true, "layout": layout() }));

    let (_, body) = send(&app.router, "GET", Some(&cookie), None).await;
    assert_eq!(body, json!({ "supported": true, "layout": layout() }));

    let (status, body) = send(&app.router, "DELETE", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "supported": true, "layout": null }));
    let (_, body) = send(&app.router, "GET", Some(&cookie), None).await;
    assert_eq!(body, json!({ "supported": true, "layout": null }));
}

#[tokio::test]
async fn one_user_never_sees_or_resets_another_users_layout() {
    let app = spin_up().await;
    let alice = create_zero_permission_principal(&app.pool).await;
    let bob = create_zero_permission_principal(&app.pool).await;
    let alice_cookie = session_cookie_for_user(&app.pool, alice).await;
    let bob_cookie = session_cookie_for_user(&app.pool, bob).await;

    send(&app.router, "PUT", Some(&alice_cookie), Some(layout())).await;

    let (_, body) = send(&app.router, "GET", Some(&bob_cookie), None).await;
    assert_eq!(body, json!({ "supported": true, "layout": null }));
    send(&app.router, "DELETE", Some(&bob_cookie), None).await;
    let (_, body) = send(&app.router, "GET", Some(&alice_cookie), None).await;
    assert_eq!(body["layout"], layout());
}

#[tokio::test]
async fn an_invalid_layout_is_a_400_with_our_message_and_saves_nothing() {
    let app = spin_up().await;
    let user = create_zero_permission_principal(&app.pool).await;
    let cookie = session_cookie_for_user(&app.pool, user).await;

    let bad = json!({ "cards": ["recent", "recent"], "shortcuts": [], "previewBoardId": null });
    let (status, body) = send(&app.router, "PUT", Some(&cookie), Some(bad)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "cards must not repeat an entry");

    let (status, _) = send(&app.router, "PUT", Some(&cookie), None).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an empty body is not a layout"
    );

    let (_, body) = send(&app.router, "GET", Some(&cookie), None).await;
    assert_eq!(body["layout"], Value::Null);
}

#[tokio::test]
async fn without_a_session_every_method_is_401() {
    let app = spin_up().await;
    for method in ["GET", "PUT", "DELETE"] {
        let (status, _) = send(&app.router, method, None, Some(layout())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method}");
    }
}
