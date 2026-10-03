//! PR #57 review (NIT) F1.10: axum's `matchit` router resolves static
//! segments ahead of path parameters regardless of registration order.
//! This integration test pins the resolution the inline comment in
//! `routes::pipelines_router` and the corresponding entry in
//! `policy::POLICY_TABLE` rely on: the literal path
//! `/api/pipelines/{id}/runs/steps` MUST reach the runs × steps matrix
//! handler, and `/api/pipelines/{id}/runs/{runId}/steps` with a real
//! `runId` MUST reach the per-run handler.
//!
//! Both paths go through the real `routes::router` built by
//! `common::spin_up_with_env`, not a hand-built state and a direct
//! handler call (the in-crate version of this assertion in
//! `routes/pipelines.rs::matrix_route_returns_a_well_shaped_body_for_a_valid_path`
//! tests the handler body but not the routing — the call there
//! bypasses the router entirely).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

/// Two distinct wiremock mocks with `.expect(1)` so the test proves
/// BOTH that the path reaches the correct handler (the response body
/// shape) AND that the wrong handler's mock never fires (the
/// `.expect(1)` would over-fire and the test would fail).
///
/// Returns the [`TestApp`] (real router pointed at the mock), the
/// Platform Admin session cookie to send with both requests, and the
/// [`wiremock::MockServer`] itself. The caller MUST keep the server
/// alive for the test body: dropping it runs wiremock's `.expect(1)`
/// verification (and stops the mock), so a server local to this helper
/// would be verified before either request is sent.
async fn app_with_two_distinct_dagster_responses() -> (TestApp, String, wiremock::MockServer) {
    let server = wiremock::MockServer::start().await;
    // Matrix query: `runsOrError(filter: { pipelineName: $job })` with
    // nested `stepStats`. The matrix handler is the only caller of this
    // shape; if the literal `/runs/steps` path is mis-routed to
    // `run_steps`, this mock never fires and `.expect(1)` fails the
    // test (and the response body comes from `run_steps`, which is the
    // next assertion).
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::body_string_contains("runsOrError"))
        .and(wiremock::matchers::body_string_contains("stepStats"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "runsOrError": { "__typename": "Runs", "results": [
                    { "runId": "r-matrix", "status": "SUCCESS",
                      "startTime": 1.0, "endTime": 2.0,
                      "stepStats": [
                          { "stepKey": "extract", "status": "SUCCESS",
                            "startTime": 1.0, "endTime": 2.0 }
                      ] }
                ] } }
            })),
        )
        .expect(1)
        .mount(&server)
        .await;
    // Per-run query: `runOrError(runId: $rid)`. `run_steps` is the
    // only caller; if a real-run-id `/runs/{runId}/steps` path is
    // mis-routed to the matrix handler, this mock never fires.
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::body_string_contains(
            "runOrError(runId:$rid)",
        ))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "runOrError": {
                    "__typename": "Run",
                    "runId": "r-real",
                    "stepStats": [
                        { "stepKey": "extract", "status": "SUCCESS",
                          "startTime": 1.0, "endTime": 2.0,
                          "materializations": [] }
                    ]
                } }
            })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut overrides = HashMap::new();
    overrides.insert(
        "DAGSTER_URL".to_owned(),
        format!("{}/graphql", server.uri()),
    );
    let app = spin_up_with_env(&overrides).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;
    (app, cookie, server)
}

async fn get(app: &axum::Router, path: &str, cookie: &str) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright")
}

/// The two `/runs/...` paths are routed to two different handlers,
/// each via the REAL axum router (PR #57 review F1.10). Hitting the
/// literal `/runs/steps` reaches the matrix handler; hitting
/// `/runs/{real-run-id}/steps` reaches the per-run handler. The two
/// wiremock `.expect(1)` counters together catch either routing
/// mistake as a "wrong mock fired" failure.
#[tokio::test]
async fn both_paths_resolve_to_their_own_handler() {
    let (app, cookie, _server) = app_with_two_distinct_dagster_responses().await;
    // 1. Literal `/runs/steps` — matrix handler. The body must carry
    //    `runs` (plural), the matrix's own shape. A wrong match to
    //    `run_steps` would surface here as `{"steps": ...}` instead.
    let matrix = get(
        &app.router,
        "/api/pipelines/gold_export_job/runs/steps",
        &cookie,
    )
    .await;
    assert_eq!(
        matrix.status(),
        StatusCode::OK,
        "literal /runs/steps must reach the matrix handler (a wrong match \
         would have surfaced 500 from the auth gate denying the path it \
         saw matched, or a 503 from run_steps on the literal run id \"steps\")"
    );
    let matrix_body: serde_json::Value = serde_json::from_slice(
        &to_bytes(matrix.into_body(), usize::MAX)
            .await
            .expect("collect body"),
    )
    .expect("valid JSON");
    assert!(
        matrix_body["runs"].is_array(),
        "matrix handler returns `runs`, got {matrix_body:?}"
    );
    assert!(
        matrix_body.get("steps").is_none(),
        "matrix handler does not return `steps` (that would mean the path \
         reached run_steps instead), got {matrix_body:?}"
    );

    // 2. `/runs/r-real/steps` — `run_steps` handler. The body must
    //    carry `steps` (singular), the per-run shape. A wrong match
    //    to `runs_step_matrix` would surface here as `{"runs": ...}`.
    let per_run = get(
        &app.router,
        "/api/pipelines/gold_export_job/runs/r-real/steps",
        &cookie,
    )
    .await;
    assert_eq!(
        per_run.status(),
        StatusCode::OK,
        "/runs/r-real/steps must reach run_steps (a wrong match to the \
         matrix handler would have surfaced a different shape, caught \
         by the assertion below)"
    );
    let per_run_body: serde_json::Value = serde_json::from_slice(
        &to_bytes(per_run.into_body(), usize::MAX)
            .await
            .expect("collect body"),
    )
    .expect("valid JSON");
    assert!(
        per_run_body["steps"].is_array(),
        "run_steps returns `steps`, got {per_run_body:?}"
    );
    assert!(
        per_run_body.get("runs").is_none(),
        "run_steps does not return `runs` (that would mean the path \
         reached runs_step_matrix instead), got {per_run_body:?}"
    );
}
