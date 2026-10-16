//! Per-table quality schedules (`DATA-14`, finding F3), driven through the
//! real router against a per-test Postgres.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use common::{TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up};

async fn call(
    router: &axum::Router,
    cookie: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(body.map(|b| b.to_string()).unwrap_or_default()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn a_table_can_be_scheduled_every_n_minutes_or_on_every_load() {
    let TestApp { router, pool } = spin_up().await;
    let user = create_principal_with_permissions(&pool, "governance:write").await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (code, listed) = call(
        &router,
        &cookie,
        "GET",
        "/api/governance/quality/schedules",
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(listed["schedules"], json!([]));
    assert_eq!(listed["defaultIntervalMinutes"], 60);

    let (code, every) = call(
        &router,
        &cookie,
        "PUT",
        "/api/governance/quality/schedules/Silver.Orders",
        Some(json!({"mode": "interval", "intervalMinutes": 180})),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{every}");
    assert_eq!(every["asset"], "silver.orders");
    assert_eq!(every["mode"], "interval");
    assert_eq!(every["intervalMinutes"], 180);

    // On every load: the interval is the backstop and defaults to a day.
    let (code, on_load) = call(
        &router,
        &cookie,
        "PUT",
        "/api/governance/quality/schedules/silver.orders",
        Some(json!({"mode": "on_load"})),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{on_load}");
    assert_eq!(on_load["mode"], "on_load");
    assert_eq!(on_load["intervalMinutes"], 1440);

    let (_, listed) = call(
        &router,
        &cookie,
        "GET",
        "/api/governance/quality/schedules",
        None,
    )
    .await;
    assert_eq!(listed["schedules"].as_array().unwrap().len(), 1);

    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_event \
         WHERE action = 'quality.schedule_update' AND resource_id = 'silver.orders'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audited, 2);
}

/// A schedule that cannot be honoured is refused with the reason, and
/// nothing is stored.
#[tokio::test]
async fn a_bad_schedule_is_refused_and_stores_nothing() {
    let TestApp { router, pool } = spin_up().await;
    let user = create_principal_with_permissions(&pool, "governance:write").await;
    let cookie = session_cookie_for_user(&pool, user).await;
    let put = |asset: &'static str, body: Value| {
        let (router, cookie) = (router.clone(), cookie.clone());
        async move {
            call(
                &router,
                &cookie,
                "PUT",
                &format!("/api/governance/quality/schedules/{asset}"),
                Some(body),
            )
            .await
        }
    };
    for (asset, body) in [
        (
            "silver.orders",
            json!({"mode": "interval", "intervalMinutes": 59}),
        ),
        (
            "silver.orders",
            json!({"mode": "interval", "intervalMinutes": 1441}),
        ),
        (
            "silver.orders",
            json!({"mode": "on_load", "intervalMinutes": 30}),
        ),
        ("silver.orders", json!({"mode": "interval"})),
        (
            "silver.orders",
            json!({"mode": "hourly", "intervalMinutes": 60}),
        ),
        ("silver.orders", json!({})),
        ("orders", json!({"mode": "interval", "intervalMinutes": 60})),
        (
            "gold.orders",
            json!({"mode": "interval", "intervalMinutes": 60}),
        ),
        (
            "silver.a%3Bb",
            json!({"mode": "interval", "intervalMinutes": 60}),
        ),
    ] {
        let (code, _) = put(asset, body.clone()).await;
        assert_eq!(code, StatusCode::BAD_REQUEST, "{asset} {body}");
    }
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM quality_schedule")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 0);
}
