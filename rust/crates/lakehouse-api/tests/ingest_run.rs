//! WS3 item 29/30: `POST /api/connectors/{id}/ingest/run` at the ROUTE
//! level — a real seeded connector row, a real authenticated session, and
//! (for the launch-success case) a real `wiremock`-mocked `Dagster`
//! GraphQL endpoint, following the same harness
//! `tests/test_connection_route.rs` already uses for other connector
//! routes.
//!
//! # What this proves that `routes::connectors`'s own unit tests cannot
//!
//! `lakehouse_dagster::DgClient::launch_run_with_config` (WS3 item 29, Z9)
//! is exercised end to end here: a `sql`-adapter connector's `ingest/run`
//! actually reaches `Dagster` with `runConfigData` carrying this
//! connector's id, and a `cdc`-adapter connector's `ingest/run` reports
//! `supported: false` WITHOUT ever calling `Dagster` at all (no mock is
//! mounted for that test — an unexpected call would hit `spin_up`'s
//! default dead upstream and fail loudly instead of silently passing).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up, spin_up_with_env};

/// Same helper shape as `tests/agents_run.rs`'s `spin_up_with_llm`: points
/// `DAGSTER_URL` at a `wiremock::MockServer` instead of `spin_up`'s
/// default dead upstream, for the one test that needs `Dagster` to
/// actually answer.
async fn spin_up_with_dagster(dagster_graphql_url: &str) -> TestApp {
    let mut overrides = HashMap::new();
    overrides.insert("DAGSTER_URL".to_owned(), dagster_graphql_url.to_owned());
    spin_up_with_env(&overrides).await
}

async fn seed_sql_connector(pool: &sqlx::PgPool, id: &str) {
    sqlx::query(
        "INSERT INTO connector (id, name, type, direction, host, secret_ref, environment, tenant, \
         adapter, ingest_mode, dial) VALUES \
         ($1, 'sql ingest test', 'PostgreSQL', 'source', 'unused', 'env:CONNECTOR_PG_PASSWORD', \
         'production', 'meridian', 'sql', 'batch', \
         '{\"driver\":\"postgres\",\"host\":\"pg-src\",\"port\":5432,\"database\":\"d\",\"user\":\"u\"}'::jsonb)",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("seed a sql-adapter connector");
}

async fn seed_cdc_connector(pool: &sqlx::PgPool, id: &str) {
    sqlx::query(
        "INSERT INTO connector (id, name, type, direction, host, secret_ref, environment, tenant, \
         adapter, ingest_mode, dial) VALUES \
         ($1, 'cdc ingest test', 'PostgreSQL CDC', 'source', 'unused', 'env:CONNECTOR_PG_CDC_PASSWORD', \
         'production', 'meridian', 'cdc', 'cdc', \
         '{\"driver\":\"postgres\",\"host\":\"pg-src\",\"port\":5432,\"database\":\"d\",\"user\":\"u\",\
         \"slotName\":\"orders_slot\",\"publicationName\":\"orders_pub\"}'::jsonb)",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("seed a cdc-adapter connector");
}

/// `POST /api/connectors/{id}/ingest/run` for a `sql`-adapter connector
/// launches `Dagster`'s static `ingest_job` with `runConfigData` carrying
/// this connector's id (WS3 item 29, Z9), and the route reports the run
/// id `Dagster` returns.
#[tokio::test]
async fn ingest_run_launches_the_static_ingest_job_with_this_connectors_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "launchRun": { "__typename": "LaunchRunSuccess", "run": { "runId": "run-ingest-1" } } }
        })))
        .mount(&server)
        .await;

    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-ingest-sql").await;

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/connectors/conn-ingest-sql/ingest/run")
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: Value = serde_json::from_slice(&bytes).expect("valid JSON");
    assert_eq!(body["runId"], "run-ingest-1");
}

/// A `cdc`-adapter connector's `ingest/run` reports `supported: false`
/// with the ADR-0008-grounded reason, WITHOUT calling `Dagster` at all —
/// `spin_up` (not `spin_up_with_dagster`) points `DAGSTER_URL` at
/// `127.0.0.1:1`, so a launch attempt here would fail loudly (502/503),
/// not silently succeed, if the route branched wrong.
#[tokio::test]
async fn ingest_run_reports_cdc_as_unsupported_not_a_launch() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_cdc_connector(&app.pool, "conn-ingest-cdc").await;

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/connectors/conn-ingest-cdc/ingest/run")
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: Value = serde_json::from_slice(&bytes).expect("valid JSON");
    assert_eq!(body["supported"], false);
    let reason = body["reason"].as_str().expect("reason is a string");
    assert!(reason.contains("ADR 0008"), "{reason}");
    assert!(reason.contains("snapshot.mode=initial"), "{reason}");
    // Names the ACTUAL compose service this connector's ingestion starts
    // from (`ops/debezium/render_compose.py`'s `debezium-<sanitized_id>`,
    // same `-` -> `_` sanitize transform as `connector_slug_for_id`) --
    // not a generic placeholder, so an operator reading this reason can
    // act on it directly.
    assert!(
        reason.contains("debezium-conn_ingest_cdc"),
        "reason must name this connector's own compose service, not a \
         generic placeholder: {reason}"
    );
}

/// An unknown connector id is a 404, not a 500/422 — the route must check
/// `get_ingest_spec` before ever touching `Dagster`.
#[tokio::test]
async fn ingest_run_unknown_connector_is_404() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/connectors/does-not-exist/ingest/run")
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// A `Dagster`-reported launch failure (`RunConfigValidationInvalid`, a
/// `PythonError`, ...) is a 422 naming the failure — never a 200 claiming
/// success, and never the raw transport-error branch (Z9's trap).
#[tokio::test]
async fn ingest_run_dagster_launch_failure_is_422_not_200() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "launchRun": { "__typename": "PythonError", "message": "job not found" } }
        })))
        .mount(&server)
        .await;

    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-ingest-sql-fail").await;

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/connectors/conn-ingest-sql-fail/ingest/run")
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: Value = serde_json::from_slice(&bytes).expect("valid JSON");
    assert_eq!(body["error"], "job not found");
}
