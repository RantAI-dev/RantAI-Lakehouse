//! Per-table quality schedules (`DATA-14`, finding F3), driven through the
//! real router against a per-test Postgres.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};

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

/// A deployment whose `ClickHouse` has the `silver.` tables named in `tables`
/// and no others. A `system.columns` lookup for one of them answers one
/// column, so the table is there; a lookup for any other table answers no
/// columns, so it is not (`SEC-80`: a schedule's table must exist).
async fn spin_up_with_silver_tables(tables: &[&str]) -> (TestApp, wiremock::MockServer) {
    let ch = wiremock::MockServer::start().await;
    // Mounted first with the default priority: the table-specific answers
    // below are mounted after it at a higher priority and win.
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::body_string_contains("system.columns"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "meta": [{"name": "name", "type": "String"}, {"name": "type", "type": "String"}],
            "data": [],
            "rows": 0,
        })))
        .mount(&ch)
        .await;
    for table in tables {
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::body_string_contains(
                "database = 'silver'",
            ))
            .and(wiremock::matchers::body_string_contains(format!(
                "table = '{table}'"
            )))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "meta": [{"name": "name", "type": "String"}, {"name": "type", "type": "String"}],
                "data": [{"name": "id", "type": "String"}],
                "rows": 1,
            })))
            .with_priority(1)
            .mount(&ch)
            .await;
    }
    let mut env = std::collections::HashMap::new();
    env.insert("CH_URL".to_owned(), ch.uri());
    (spin_up_with_env(&env).await, ch)
}

#[tokio::test]
async fn a_table_can_be_scheduled_every_n_minutes_or_on_every_load() {
    let (TestApp { router, pool }, _ch) = spin_up_with_silver_tables(&["orders"]).await;
    // PR #106 review R1 SHOULD-FIX DATA-14 1 / D4: *:* passes catalog tenant gate
    let user = create_principal_with_permissions(&pool, "*:*,governance:write").await;
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
    let (TestApp { router, pool }, _ch) = spin_up_with_silver_tables(&["orders"]).await;
    // PR #106 review R1 SHOULD-FIX DATA-14 1 / D4: *:* passes catalog tenant gate
    let user = create_principal_with_permissions(&pool, "*:*,governance:write").await;
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

/// `WEB-3` S3a-D3: a table goes back to the default schedule by removing
/// its row, which is what `GET` then shows (no row for it, 60 minutes for
/// every table). Resetting a table that has no row is not an error.
#[tokio::test]
async fn a_schedule_can_go_back_to_the_default() {
    let (TestApp { router, pool }, _ch) = spin_up_with_silver_tables(&["orders"]).await;
    let user = create_principal_with_permissions(&pool, "*:*,governance:write").await;
    let cookie = session_cookie_for_user(&pool, user).await;
    let set = "/api/governance/quality/schedules/silver.orders";

    let (code, body) = call(
        &router,
        &cookie,
        "PUT",
        set,
        Some(json!({ "mode": "interval", "intervalMinutes": 120 })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{body}");
    let (_, listed) = call(
        &router,
        &cookie,
        "GET",
        "/api/governance/quality/schedules",
        None,
    )
    .await;
    assert_eq!(listed["schedules"][0]["asset"], json!("silver.orders"));

    let (code, body) = call(&router, &cookie, "DELETE", set, None).await;
    assert_eq!(code, StatusCode::NO_CONTENT, "{body}");
    let (_, listed) = call(
        &router,
        &cookie,
        "GET",
        "/api/governance/quality/schedules",
        None,
    )
    .await;
    assert_eq!(listed["schedules"], json!([]), "back on the default");
    assert_eq!(listed["defaultIntervalMinutes"], 60);

    // The outcome already holds, so asking again is not an error.
    let (code, _) = call(&router, &cookie, "DELETE", set, None).await;
    assert_eq!(code, StatusCode::NO_CONTENT);

    // A table that is not zone.table is a 400, and nothing is audited.
    let (code, body) = call(
        &router,
        &cookie,
        "DELETE",
        "/api/governance/quality/schedules/orders",
        None,
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{body}");

    let actions: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM audit_event WHERE action = 'quality.schedule_reset'",
    )
    .fetch_all(&pool)
    .await
    .expect("read the audit rows");
    assert_eq!(actions.len(), 2, "each reset is audited: {actions:?}");
}
