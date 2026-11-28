//! `DATA-66` T3: `GET /api/gold/sync-due` over the real router, with a
//! `wiremock` `ClickHouse` and the real Postgres of the shared harness.
//!
//! The decision itself (every rule in D8) is unit-tested in
//! `lakehouse_api::iceberg_sync`; what this file proves is the wiring:
//! the handler reads the enabled publications, asks the engine for the
//! one `system.parts` signal and the one newest-run read, and answers
//! the due list with the deterministic key the Dagster sensor uses.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};

/// The `system.parts` rows the stand-in engine answers: one Gold mart
/// with an active part whose `max(modification_time)` is this second.
const CHANGED_S: &str = "1795773540";

/// The engine's answers, split by the statement the handler sends. Every
/// other statement (the `console.gold_export_run` DDL and its raw-error
/// count) is an empty result, which is what a real engine answers for
/// DDL and for a history table nobody has written yet.
struct EngineStub;

impl Respond for EngineStub {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let body = String::from_utf8_lossy(&request.body);
        let data = if body.contains("max(modification_time)") {
            json!([{ "database": "serving", "table": "watched_mart", "changed_s": CHANGED_S }])
        } else if body.contains("AS n FROM console.gold_export_run") {
            json!([{ "n": "0" }])
        } else {
            json!([])
        };
        let rows = data.as_array().map_or(0, Vec::len);
        ResponseTemplate::new(200).set_body_json(json!({
            "meta": [],
            "data": data,
            "rows": rows,
        }))
    }
}

async fn engine() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(EngineStub)
        .mount(&server)
        .await;
    server
}

async fn get(router: &axum::Router, path: &str, cookie: &str) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read the body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn app_with_engine(engine: &MockServer) -> TestApp {
    let mut overrides = HashMap::new();
    overrides.insert("CH_URL".to_owned(), engine.uri());
    spin_up_with_env(&overrides).await
}

/// One `on_update` publication and one `manual` one: the watcher answers
/// the change for the first and nothing for the second, with the key the
/// sensor turns into a `RunRequest` (D8).
#[tokio::test]
async fn sync_due_answers_a_change_for_an_on_update_publication_and_skips_a_manual_one() {
    let engine = engine().await;
    let app = app_with_engine(&engine).await;
    lakehouse_store::gold_publication::upsert_in_zone(
        &app.pool,
        "gold",
        "watched_mart",
        true,
        Uuid::new_v4(),
    )
    .await
    .expect("seed the on_update publication");
    lakehouse_store::gold_publication::upsert_in_zone(
        &app.pool,
        "gold",
        "manual_mart",
        true,
        Uuid::new_v4(),
    )
    .await
    .expect("seed the manual publication");
    lakehouse_store::gold_publication::set_sync_in_zone(
        &app.pool,
        "gold",
        "manual_mart",
        &lakehouse_store::gold_publication::SyncSetting {
            mode: lakehouse_store::gold_publication::SyncMode::Manual,
            schedule: None,
            time_zone: None,
        },
        Uuid::new_v4(),
    )
    .await
    .expect("mark the second publication manual")
    .expect("the row exists");

    let user = create_principal_with_permissions(&app.pool, "gold:export").await;
    let cookie = session_cookie_for_user(&app.pool, user).await;

    let (status, body) = get(&app.router, "/api/gold/sync-due", &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let due = body["due"].as_array().expect("a due list");
    assert_eq!(
        due,
        &vec![json!({
            "zone": "gold",
            "mart": "watched_mart",
            "trigger": "change",
            "runKey": format!("change:gold:watched_mart:{CHANGED_S}"),
        })]
    );
}

/// A switched-off publication is not due, and a viewer without a Gold
/// grant never reaches the handler (the SEC-66 gate answers 403).
#[tokio::test]
async fn a_disabled_publication_is_not_due_and_a_dashboard_viewer_is_refused() {
    let engine = engine().await;
    let app = app_with_engine(&engine).await;
    lakehouse_store::gold_publication::upsert_in_zone(
        &app.pool,
        "gold",
        "watched_mart",
        false,
        Uuid::new_v4(),
    )
    .await
    .expect("seed the disabled publication");

    let user = create_principal_with_permissions(&app.pool, "gold:export").await;
    let cookie = session_cookie_for_user(&app.pool, user).await;
    let (status, body) = get(&app.router, "/api/gold/sync-due", &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["due"], json!([]));

    let viewer = create_principal_with_permissions(&app.pool, "dashboard:read").await;
    let viewer_cookie = session_cookie_for_user(&app.pool, viewer).await;
    let (status, _) = get(&app.router, "/api/gold/sync-due", &viewer_cookie).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// PR #742 CI fix (`DATA-66`): an automatic `iceberg_sync_job` run sends
/// `?requireEnabled=true` so a run queued before publishing was switched
/// off (or changed to `manual`) skips cleanly without exporting.
#[tokio::test]
async fn require_enabled_export_skips_when_publishing_is_off_or_manual() {
    const TOKEN: &str = "iceberg-sync-require-enabled-token";
    let engine = engine().await;
    let mut overrides = HashMap::new();
    overrides.insert("CH_URL".to_owned(), engine.uri());
    overrides.insert("GOLD_EXPORT_RUN_TOKEN".to_owned(), TOKEN.to_owned());
    overrides.insert(
        "CATALOG_TENANT_ID".to_owned(),
        "11111111-1111-4111-8111-000000000001".to_owned(),
    );
    let app = spin_up_with_env(&overrides).await;
    crate::gold_export_auth::seed_gold_export_service_credential(&app.pool, TOKEN).await;
    lakehouse_store::gold_publication::upsert_in_zone(
        &app.pool,
        "gold",
        "disabled_mart",
        false,
        Uuid::new_v4(),
    )
    .await
    .expect("seed disabled publication");
    lakehouse_store::gold_publication::upsert_in_zone(
        &app.pool,
        "gold",
        "manual_mart",
        true,
        Uuid::new_v4(),
    )
    .await
    .expect("seed manual publication");
    lakehouse_store::gold_publication::set_sync_in_zone(
        &app.pool,
        "gold",
        "manual_mart",
        &lakehouse_store::gold_publication::SyncSetting {
            mode: lakehouse_store::gold_publication::SyncMode::Manual,
            schedule: None,
            time_zone: None,
        },
        Uuid::new_v4(),
    )
    .await
    .expect("set manual sync mode")
    .expect("row exists");

    for (mart, expected_reason) in [
        ("disabled_mart", "publishing_off"),
        ("never_configured_mart", "publishing_off"),
        ("manual_mart", "manual"),
    ] {
        let response = app
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/gold/export/{mart}?ifChanged=true&requireEnabled=true"
                    ))
                    .header("authorization", format!("Bearer {TOKEN}"))
                    .header("x-run-token", TOKEN)
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router answers");
        assert_eq!(response.status(), StatusCode::OK, "mart={mart}");
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let body: Value = serde_json::from_slice(&bytes).expect("json body");
        assert_eq!(
            body,
            json!({ "skipped": true, "reason": expected_reason }),
            "mart={mart}"
        );
    }
}
