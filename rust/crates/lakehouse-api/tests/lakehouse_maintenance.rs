//! HTTP-level integration tests for `POST /api/lakehouse/tables/{ns}/{table}/maintenance`
//! and `GET /api/lakehouse/maintenance-policies` (WS2 §4, WS2 plan review W8).
//!
//! Uses namespace `silver` on purpose, not `bronze`: `GET .../maintenance`
//! only queries `ClickHouse` when the namespace is `bronze`
//! (`routes::lakehouse::last_run_applies`), and `spin_up` points
//! `ClickHouse` at an instantly-refusing dead upstream (see
//! `tests/common/mod.rs`'s module doc comment). A `silver` table lets the
//! GET half of this file assert a real `200` rather than a `ClickHouse`
//! failure.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use common::{TestApp, session_cookie_for_seeded_user, spin_up};

async fn request(
    router: &axum::Router,
    method: &str,
    uri: &str,
    cookie: &str,
    body: Option<&Value>,
) -> axum::http::Response<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie);
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(serde_json::to_vec(value).expect("serialize body"))
        }
        None => Body::empty(),
    };
    router
        .clone()
        .oneshot(builder.body(body).expect("build request"))
        .await
        .expect("router never fails a request outright")
}

async fn json_body(resp: axum::http::Response<Body>) -> Value {
    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("valid JSON body")
}

fn valid_policy_body() -> Value {
    json!({
        "snapshotsToKeep": 10,
        "orphanAgeHours": 48,
        "compactSmallFiles": true,
        "schedule": "daily",
    })
}

/// A Governance Admin's POST is stored, and both the per-table GET (read
/// back as a `catalog:read`-holding principal — see the comment at that
/// call below) and the cross-table list reflect exactly what was stored.
#[tokio::test]
async fn governance_admin_sets_a_policy_and_reads_it_back() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "dewi@meridian.example").await;

    let post_resp = request(
        &router,
        "POST",
        "/api/lakehouse/tables/silver/orders/maintenance",
        &cookie,
        Some(&valid_policy_body()),
    )
    .await;
    assert_eq!(post_resp.status(), StatusCode::OK);
    let post_body = json_body(post_resp).await;
    assert_eq!(post_body["namespace"], json!("silver"));
    assert_eq!(post_body["tableName"], json!("orders"));
    assert_eq!(post_body["configured"], json!(true));
    assert_eq!(post_body["snapshotsToKeep"], json!(10));
    assert_eq!(post_body["orphanAgeHours"], json!(48));
    assert_eq!(post_body["compactSmallFiles"], json!(true));
    assert_eq!(post_body["schedule"], json!("daily"));

    // `GET .../maintenance` (unlike the POST this test just proved) is
    // gated on `catalog:read` (a pre-existing, out-of-scope-for-B2 policy
    // row) — `dewi@meridian.example`'s Governance Admin role
    // (`policy:*, residency:*, audit:read, governance:write`) does not
    // hold it, so the round-trip read uses a session that does
    // (`fajar@meridian.example`, Platform Admin, `*:*`) rather than
    // asserting a 200 the auth gate would never actually return for the
    // user who wrote the policy.
    let reader_cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let get_resp = request(
        &router,
        "GET",
        "/api/lakehouse/tables/silver/orders/maintenance",
        &reader_cookie,
        None,
    )
    .await;
    assert_eq!(get_resp.status(), StatusCode::OK);
    let get_body = json_body(get_resp).await;
    assert_eq!(get_body["configured"], json!(true));
    assert_eq!(get_body["snapshotsToKeep"], json!(10));
    assert_eq!(get_body["orphanAgeHours"], json!(48));
    assert_eq!(get_body["compactSmallFiles"], json!(true));
    assert_eq!(get_body["schedule"], json!("daily"));
    assert_eq!(get_body["lastRun"], Value::Null);

    let list_resp = request(
        &router,
        "GET",
        "/api/lakehouse/maintenance-policies",
        &cookie,
        None,
    )
    .await;
    assert_eq!(list_resp.status(), StatusCode::OK);
    let list_body = json_body(list_resp).await;
    let policies = list_body["policies"]
        .as_array()
        .expect("policies is an array");
    assert!(
        policies.iter().any(|p| p["namespace"] == json!("silver")
            && p["tableName"] == json!("orders")
            && p["snapshotsToKeep"] == json!(10)
            && p["orphanAgeHours"] == json!(48)
            && p["compactSmallFiles"] == json!(true)
            && p["schedule"] == json!("daily")),
        "expected silver/orders in {policies:?}"
    );
}

/// A seeded Analyst (`query:read, catalog:read, lineage:read` — no
/// `governance:write`) is refused the POST.
#[tokio::test]
async fn analyst_is_denied_the_post() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let resp = request(
        &router,
        "POST",
        "/api/lakehouse/tables/silver/orders/maintenance",
        &cookie,
        Some(&valid_policy_body()),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

/// `snapshotsToKeep: 0` is refused: the current snapshot is always kept,
/// so 0 is never a valid keep-count.
#[tokio::test]
async fn zero_snapshots_to_keep_is_a_400() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "dewi@meridian.example").await;

    let resp = request(
        &router,
        "POST",
        "/api/lakehouse/tables/silver/orders/maintenance",
        &cookie,
        Some(&json!({
            "snapshotsToKeep": 0,
            "orphanAgeHours": null,
            "compactSmallFiles": false,
            "schedule": null,
        })),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// A malformed JSON body is a 400, not a 500.
#[tokio::test]
async fn malformed_body_is_a_400() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "dewi@meridian.example").await;

    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/lakehouse/tables/silver/orders/maintenance")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from("not json at all"))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// An uppercase namespace segment fails `validate_ident`'s
/// `^[a-z0-9_]+$` check before the body is even parsed.
#[tokio::test]
async fn uppercase_namespace_is_a_400() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "dewi@meridian.example").await;

    let resp = request(
        &router,
        "POST",
        "/api/lakehouse/tables/Silver/orders/maintenance",
        &cookie,
        Some(&valid_policy_body()),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
