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
use wiremock::matchers::{body_string_contains, method, path};
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

/// Answers `ingest/run`'s "is a run already going?" check with `results`
/// (`runsOrError` rows carrying `runConfig`).
async fn mount_recent_runs(server: &MockServer, results: Value) {
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("runsOrError"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "runsOrError": { "__typename": "Runs", "results": results } }
        })))
        .mount(server)
        .await;
}

fn ingest_run_for(connector_id: &str, run_id: &str, status: &str) -> Value {
    json!({
        "runId": run_id, "status": status, "startTime": 1_790_670_111.0, "endTime": null,
        "runConfig": { "ops": { "run_ingest": { "config": { "connector_id": connector_id } } } }
    })
}

async fn seed_sql_connector(pool: &sqlx::PgPool, id: &str) {
    sqlx::query(
        "INSERT INTO connector (id, tenant_id, \
         name, type, direction, host, secret_ref, environment, tenant, \
         adapter, ingest_mode, dial) VALUES \
         ($1, '11111111-1111-4111-8111-000000000001', \
         $2, 'PostgreSQL', 'source', 'unused', 'env:CONNECTOR_PG_PASSWORD', \
         'production', 'meridian', 'sql', 'batch', \
         '{\"driver\":\"postgres\",\"host\":\"pg-src\",\"port\":5432,\"database\":\"d\",\"user\":\"u\"}'::jsonb)",
    )
    .bind(id)
    // `connector_name_unique`: a test that seeds two connectors needs two names.
    .bind(format!("sql ingest test {id}"))
    .execute(pool)
    .await
    .expect("seed a sql-adapter connector");
}

async fn seed_cdc_connector(pool: &sqlx::PgPool, id: &str) {
    sqlx::query(
        "INSERT INTO connector (id, tenant_id, \
         name, type, direction, host, secret_ref, environment, tenant, \
         adapter, ingest_mode, dial) VALUES \
         ($1, '11111111-1111-4111-8111-000000000001', \
         $2, 'PostgreSQL CDC', 'source', 'unused', 'env:CONNECTOR_PG_CDC_PASSWORD', \
         'production', 'meridian', 'cdc', 'cdc', \
         '{\"driver\":\"postgres\",\"host\":\"pg-src\",\"port\":5432,\"database\":\"d\",\"user\":\"u\",\
         \"slotName\":\"orders_slot\",\"publicationName\":\"orders_pub\"}'::jsonb)",
    )
    .bind(id)
    .bind(format!("cdc ingest test {id}"))
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
    mount_recent_runs(&server, json!([])).await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("launchRun"))
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
    mount_recent_runs(&server, json!([])).await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("launchRun"))
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
    // SEC-11: this used to assert that Dagster's own message ("job not
    // found") reached the body; that asserted the bug. The body now carries
    // a fixed sentence and a reference, and the orchestrator's text is only
    // in the log.
    let message = body["error"].as_str().expect("an error message");
    assert!(
        message.starts_with("The request to a backing service failed. Reference: "),
        "{message}"
    );
    assert!(!message.contains("job not found"), "{message}");
}

/// Posts `ingest/run` for `connector_id` against a `Dagster` that answers the
/// launch with `launch_run`, and returns the status, the `Retry-After`
/// header and the JSON body.
async fn ingest_run_with_launch_result(
    connector_id: &str,
    launch_run: Value,
) -> (StatusCode, Option<String>, Value) {
    let server = MockServer::start().await;
    mount_recent_runs(&server, json!([])).await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("launchRun"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "launchRun": launch_run }
        })))
        .mount(&server)
        .await;
    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, connector_id).await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/connectors/{connector_id}/ingest/run"))
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    let status = response.status();
    let retry_after = response
        .headers()
        .get("retry-after")
        .map(|v| v.to_str().expect("ascii header").to_owned());
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    (
        status,
        retry_after,
        serde_json::from_slice(&bytes).expect("valid JSON"),
    )
}

/// `PR #105 CI fix`: Dagster's typed "job not loaded" result (what a code
/// location that is still loading answers) is a 503 with the stable code
/// `orchestrator_job_not_loaded`, a `Retry-After`, a fixed sentence and a
/// reference, and none of Dagster's own words. The G6 gates retry on the
/// code; they used to match Dagster's text, which SEC-11 removed.
#[tokio::test]
async fn ingest_run_job_not_loaded_is_a_503_with_a_code_and_no_upstream_text() {
    for launch_run in [
        json!({ "__typename": "PipelineNotFoundError", "message": "planted-marker" }),
        json!({ "__typename": "PythonError", "message": "planted-marker",
                "className": "DagsterUserCodeUnreachableError" }),
    ] {
        let (status, retry_after, body) =
            ingest_run_with_launch_result("conn-ingest-sql-notloaded", launch_run).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(retry_after.as_deref(), Some("5"));
        assert_eq!(body["code"], "orchestrator_job_not_loaded");
        let message = body["error"].as_str().expect("an error message");
        assert!(
            message.starts_with(
                "The orchestrator has not loaded this job yet. Try again shortly. Reference: "
            ),
            "{message}"
        );
        assert!(!body.to_string().contains("planted-marker"), "{body}");
        assert!(
            !body.to_string().contains("PipelineNotFoundError"),
            "{body}"
        );
    }
}

/// Any other Dagster launch failure keeps the generic 422: a reference, no
/// code, no upstream text. Words that merely look like "not loaded" in the
/// message do not make it one; only the typed fields do.
#[tokio::test]
async fn ingest_run_other_launch_failures_stay_generic_without_a_code() {
    let (status, retry_after, body) = ingest_run_with_launch_result(
        "conn-ingest-sql-generic",
        json!({ "__typename": "PythonError",
                "message": "PipelineNotFoundError planted-marker", "className": "ValueError" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(retry_after.is_none());
    assert!(body.get("code").is_none(), "{body}");
    let message = body["error"].as_str().expect("an error message");
    assert!(
        message.starts_with("The request to a backing service failed. Reference: "),
        "{message}"
    );
    assert!(!body.to_string().contains("planted-marker"), "{body}");
}

/// `PR #105 CI fix`: Dagster not answering at all is a 503 with
/// `orchestrator_unreachable` and a `Retry-After`, not a bare 503 a client
/// can only guess at. `spin_up`'s default Dagster URL is a dead upstream.
#[tokio::test]
async fn ingest_run_with_dagster_down_is_a_503_with_an_unreachable_code() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-ingest-sql-down").await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/connectors/conn-ingest-sql-down/ingest/run")
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().get("retry-after").is_some());
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: Value = serde_json::from_slice(&bytes).expect("valid JSON");
    assert_eq!(body["code"], "orchestrator_unreachable");
    assert!(
        body["error"]
            .as_str()
            .expect("message")
            .starts_with("The orchestrator is not reachable. Try again shortly. Reference: "),
        "{body}"
    );
}

/// `PR #105 CI fix`: `POST /api/pipelines/{job}/trigger` answers the same
/// typed "job not loaded" launch result the same way (503 plus the code), so
/// the G3/G3a/G7 gates retry on the code too.
#[tokio::test]
async fn pipeline_trigger_job_not_loaded_is_a_503_with_a_code() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("launchRun"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "launchRun": { "__typename": "PipelineNotFoundError" } }
        })))
        .mount(&server)
        .await;
    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/pipelines/some_job/trigger")
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: Value = serde_json::from_slice(&bytes).expect("valid JSON");
    assert_eq!(body["code"], "orchestrator_job_not_loaded");
    assert!(
        !body.to_string().contains("PipelineNotFoundError"),
        "{body}"
    );
}

/// A second run while this connector's run is still going is refused with
/// a 409, and nothing is launched: Bronze is append-only, so it would copy
/// every table in a second time.
#[tokio::test]
async fn ingest_run_refuses_a_second_run_while_one_is_going() {
    let server = MockServer::start().await;
    mount_recent_runs(
        &server,
        json!([ingest_run_for(
            "conn-ingest-busy",
            "7bd3a0c4-busy",
            "STARTED"
        )]),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("launchRun"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "launchRun": { "__typename": "LaunchRunSuccess", "run": { "runId": "never" } } }
        })))
        .expect(0)
        .mount(&server)
        .await;

    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-ingest-busy").await;

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/connectors/conn-ingest-busy/ingest/run")
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: Value = serde_json::from_slice(&bytes).expect("valid JSON");
    let error = body["error"].as_str().expect("error is a string");
    assert!(error.contains("already running"), "{error}");
    assert!(error.contains("7bd3a0c4"), "{error}");
}

/// Another connector's run does not block this one.
#[tokio::test]
async fn ingest_run_is_not_blocked_by_another_connectors_run() {
    let server = MockServer::start().await;
    mount_recent_runs(
        &server,
        json!([ingest_run_for("conn-someone-else", "other-run", "STARTED")]),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("launchRun"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "launchRun": { "__typename": "LaunchRunSuccess", "run": { "runId": "run-mine" } } }
        })))
        .mount(&server)
        .await;

    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-ingest-free").await;

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/connectors/conn-ingest-free/ingest/run")
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");

    assert_eq!(response.status(), StatusCode::OK);
}

/// `GET .../ingest/runs` lists only this connector's runs, newest first,
/// in the pipelines pages' status vocabulary.
#[tokio::test]
async fn ingest_run_history_lists_only_this_connectors_runs() {
    let server = MockServer::start().await;
    mount_recent_runs(
        &server,
        json!([
            ingest_run_for("conn-ingest-hist", "run-2", "STARTED"),
            ingest_run_for("conn-someone-else", "run-x", "SUCCESS"),
            ingest_run_for("conn-ingest-hist", "run-1", "FAILURE"),
        ]),
    )
    .await;

    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    seed_sql_connector(&app.pool, "conn-ingest-hist").await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/connectors/conn-ingest-hist/ingest/runs")
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
    let runs = body.as_array().expect("an array");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0]["runId"], "run-2");
    assert_eq!(runs[0]["status"], "running");
    assert_eq!(runs[1]["runId"], "run-1");
    assert_eq!(runs[1]["status"], "failed");
}

/// A load mode the ingest job cannot run is refused when the spec is
/// saved, with the reason, instead of surfacing later as a rejected run.
#[tokio::test]
async fn ingest_spec_put_refuses_a_load_mode_the_job_cannot_run() {
    let app = spin_up().await;
    seed_sql_connector(&app.pool, "conn-ingest-mode").await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    for (object, expected) in [
        (
            json!({ "name": "public.orders", "target": "orders", "loadMode": "incremental" }),
            "incrementalKey",
        ),
        (
            json!({ "name": "public.orders", "target": "orders", "loadMode": "merge" }),
            "replace, append, incremental, replace_dedup, incremental_dedup or history",
        ),
    ] {
        let body = json!({
            "adapter": "sql",
            "ingestMode": "batch",
            "dial": { "driver": "postgres", "host": "pg-src", "port": 5432, "database": "d", "user": "u" },
            "sourceObjects": [object],
        });
        let response = app
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/connectors/conn-ingest-mode/ingest-spec")
                    .header("cookie", &cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("build request"),
            )
            .await
            .expect("router never fails a request outright");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let body: Value = serde_json::from_slice(&bytes).expect("valid JSON");
        let message = body["error"].as_str().unwrap_or_default();
        assert!(message.contains(expected), "{message}");
    }

    let stored: Value =
        sqlx::query_scalar("SELECT source_objects FROM connector WHERE id = 'conn-ingest-mode'")
            .fetch_one(&app.pool)
            .await
            .expect("read source objects");
    assert_eq!(stored, json!([]), "nothing was saved");
}

// --- SRC-10 decision 5: a run of ONE table with an action --------------------

/// Give `id` two selected tables (`public.orders`, `public.items`).
async fn select_two_tables(pool: &sqlx::PgPool, id: &str) {
    sqlx::query("UPDATE connector SET source_objects = $2 WHERE id = $1")
        .bind(id)
        .bind(json!([
            {"name": "public.orders", "target": "orders"},
            {"name": "public.items", "target": "items"},
        ]))
        .execute(pool)
        .await
        .expect("selected tables");
}

async fn post_run(
    app: &TestApp,
    cookie: &str,
    id: &str,
    body: Option<&str>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri(format!("/api/connectors/{id}/ingest/run"))
        .header("cookie", cookie);
    let payload = match body {
        Some(text) => {
            request = request.header("content-type", "application/json");
            Body::from(text.to_owned())
        }
        None => Body::empty(),
    };
    let response = app
        .router
        .clone()
        .oneshot(request.body(payload).expect("build request"))
        .await
        .expect("router never fails a request outright");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// A `Dagster` mock that answers the run list with `runs` and launches `run_id`.
async fn dagster_that_launches(runs: Value, run_id: &str) -> MockServer {
    let server = MockServer::start().await;
    mount_recent_runs(&server, runs).await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("launchRun"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "launchRun": { "__typename": "LaunchRunSuccess", "run": { "runId": run_id } } }
        })))
        .mount(&server)
        .await;
    server
}

/// The run config of every `launchRun` the mock received.
async fn launched_configs(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .expect("request log")
        .iter()
        .filter_map(|request| serde_json::from_slice::<Value>(&request.body).ok())
        .filter(|body| {
            body["query"]
                .as_str()
                .is_some_and(|q| q.contains("launchRun"))
        })
        .map(|body| body["variables"]["cfg"].clone())
        .collect()
}

/// `SRC-10` decision 5: each action reaches the orchestrator as `object` and
/// `action` beside `connector_id` under the same op config key, the answer is the
/// run id as before, and the audit event names the table and the action.
#[tokio::test]
async fn a_run_of_one_table_sends_object_and_action_to_the_orchestrator_and_is_audited() {
    for action in ["refresh_keep", "refresh_remove", "clear"] {
        let server = dagster_that_launches(json!([]), "run-one-table").await;
        let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
        let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
        seed_sql_connector(&app.pool, "conn-one-table").await;
        select_two_tables(&app.pool, "conn-one-table").await;

        let body = json!({"object": "public.orders", "action": action}).to_string();
        let (status, answer) = post_run(&app, &cookie, "conn-one-table", Some(&body)).await;

        assert_eq!(status, StatusCode::OK, "{action}: {answer}");
        assert_eq!(answer, json!({"runId": "run-one-table"}), "{action}");
        assert_eq!(
            launched_configs(&server).await,
            vec![json!({"ops": {"run_ingest": {"config": {
                "connector_id": "conn-one-table",
                "object": "public.orders",
                "action": action,
            }}}})],
            "{action}"
        );
        let (events, args): (i64, Option<Value>) = sqlx::query_as(
            "SELECT count(*), max(args::text)::jsonb FROM audit_event \
             WHERE resource_id = 'conn-one-table' AND action = $1",
        )
        .bind(format!("connector.ingest.{action}"))
        .fetch_one(&app.pool)
        .await
        .expect("audit rows");
        assert_eq!(events, 1, "{action}");
        let args = args.expect("args");
        assert_eq!(args["object"], "public.orders", "{action}");
        assert_eq!(args["action"], action);
    }
}

/// An empty body, and `{}`, are the run of every table: the config holds the
/// connector id and nothing else, as before `SRC-10`, and nothing is audited.
#[tokio::test]
async fn a_run_without_a_body_sends_only_the_connector_id() {
    for body in [None, Some(""), Some("{}")] {
        let server = dagster_that_launches(json!([]), "run-all").await;
        let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
        let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
        seed_sql_connector(&app.pool, "conn-all-tables").await;

        let (status, answer) = post_run(&app, &cookie, "conn-all-tables", body).await;

        assert_eq!(status, StatusCode::OK, "{body:?}: {answer}");
        assert_eq!(
            launched_configs(&server).await,
            vec![json!({"ops": {"run_ingest": {"config": {"connector_id": "conn-all-tables"}}}})],
            "{body:?}"
        );
        let (events,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM audit_event WHERE resource_id = 'conn-all-tables' \
             AND action LIKE 'connector.ingest.%'",
        )
        .fetch_one(&app.pool)
        .await
        .expect("audit rows");
        assert_eq!(events, 0, "{body:?}");
    }
}

/// A bad body is a 400 and launches nothing: unknown fields, one of the two
/// fields alone, an action that does not exist, a blank table.
#[tokio::test]
async fn a_bad_run_body_is_a_400_and_launches_nothing() {
    let server = dagster_that_launches(json!([]), "never").await;
    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-bad-body").await;
    select_two_tables(&app.pool, "conn-bad-body").await;

    for body in [
        r#"{"object":"public.orders","action":"clear","extra":1}"#,
        r#"{"object":"public.orders"}"#,
        r#"{"action":"clear"}"#,
        r#"{"object":"public.orders","action":"drop_everything"}"#,
        r#"{"object":"  ","action":"clear"}"#,
        r#"{"object":null,"action":"clear"}"#,
        "not json",
    ] {
        let (status, answer) = post_run(&app, &cookie, "conn-bad-body", Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {answer}");
    }
    assert!(launched_configs(&server).await.is_empty());
}

/// A table the connector does not load is a 404 with a fixed text that repeats
/// nothing from the request; nothing is launched.
#[tokio::test]
async fn a_table_the_connector_does_not_load_is_a_404_with_a_fixed_message() {
    let server = dagster_that_launches(json!([]), "never").await;
    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-no-such-table").await;
    select_two_tables(&app.pool, "conn-no-such-table").await;

    let body = json!({"object": "public.secret_payroll", "action": "clear"}).to_string();
    let (status, answer) = post_run(&app, &cookie, "conn-no-such-table", Some(&body)).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(answer["error"], "This connector does not load that table.");
    assert!(launched_configs(&server).await.is_empty());
}

/// `SRC-8` observation step: a table that waits for a schema decision cannot be
/// refreshed or cleared, whichever the action; a table that does not wait can,
/// and a waiting OTHER table does not block it.
#[tokio::test]
async fn a_table_that_waits_for_a_schema_decision_cannot_be_refreshed_or_cleared() {
    let server = dagster_that_launches(json!([]), "run-ok").await;
    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-waits").await;
    select_two_tables(&app.pool, "conn-waits").await;
    sqlx::query(
        "INSERT INTO connector_schema_change \
         (id, connector_id, object_name, kind, column_name, before_value, after_value, breaking, \
          status, detected_at) \
         VALUES ('chg-1', 'conn-waits', 'public.orders', 'column_removed', 'note', 'text', NULL, \
                 true, 'pending', now())",
    )
    .execute(&app.pool)
    .await
    .expect("a waiting change");

    for action in ["refresh_keep", "refresh_remove", "clear"] {
        let body = json!({"object": "public.orders", "action": action}).to_string();
        let (status, answer) = post_run(&app, &cookie, "conn-waits", Some(&body)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{action}");
        assert_eq!(
            answer["error"],
            "This table is waiting for a schema decision, so it cannot be refreshed or cleared \
             yet. Approve or resolve the change first.",
            "{action}"
        );
    }
    assert!(launched_configs(&server).await.is_empty());

    let body = json!({"object": "public.items", "action": "clear"}).to_string();
    let (status, answer) = post_run(&app, &cookie, "conn-waits", Some(&body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
}

/// The existing 409s come first: a run already going is the answer even for a
/// table the connector does not load, and a paused connector starts nothing.
#[tokio::test]
async fn the_409s_for_a_run_going_and_a_paused_connector_are_answered_before_the_body_is_judged() {
    let server = dagster_that_launches(
        json!([ingest_run_for(
            "conn-busy-table",
            "7bd3a0c4-busy",
            "STARTED"
        )]),
        "never",
    )
    .await;
    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-busy-table").await;
    select_two_tables(&app.pool, "conn-busy-table").await;

    let body = json!({"object": "public.not_selected", "action": "clear"}).to_string();
    let (status, answer) = post_run(&app, &cookie, "conn-busy-table", Some(&body)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        answer["error"]
            .as_str()
            .unwrap()
            .contains("already running")
    );

    seed_sql_connector(&app.pool, "conn-paused-table").await;
    select_two_tables(&app.pool, "conn-paused-table").await;
    sqlx::query(
        "UPDATE connector SET paused_reason = 'paused by a test', paused_at = now() WHERE id = $1",
    )
    .bind("conn-paused-table")
    .execute(&app.pool)
    .await
    .expect("pause");
    let (status, answer) = post_run(&app, &cookie, "conn-paused-table", Some(&body)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(answer["error"].as_str().unwrap().contains("paused"));
    assert!(launched_configs(&server).await.is_empty());
}

/// A `cdc` connector answers the honest `supported: false` also for a run of one
/// table, and so does a stream connector; neither calls the orchestrator.
#[tokio::test]
async fn cdc_and_stream_connectors_answer_supported_false_for_a_run_of_one_table() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_cdc_connector(&app.pool, "conn-cdc-table").await;
    seed_sql_connector(&app.pool, "conn-stream-table").await;
    sqlx::query("UPDATE connector SET ingest_mode = 'stream', source_objects = $2 WHERE id = $1")
        .bind("conn-stream-table")
        .bind(json!([{"name": "public.orders", "target": "orders"}]))
        .execute(&app.pool)
        .await
        .expect("stream mode");

    let body = json!({"object": "public.orders", "action": "clear"}).to_string();
    for id in ["conn-cdc-table", "conn-stream-table"] {
        let (status, answer) = post_run(&app, &cookie, id, Some(&body)).await;
        assert_eq!(status, StatusCode::OK, "{id}");
        assert_eq!(answer["supported"], false, "{id}");
        assert!(
            answer["reason"].as_str().is_some_and(|r| !r.is_empty()),
            "{id}"
        );
    }
}

/// Decision 7: the same permission as "Run now". A caller without
/// `connector:manage` is refused before the body is read.
#[tokio::test]
async fn a_run_of_one_table_needs_connector_manage() {
    let server = dagster_that_launches(json!([]), "never").await;
    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    seed_sql_connector(&app.pool, "conn-no-permission").await;
    select_two_tables(&app.pool, "conn-no-permission").await;
    let user = common::create_principal_with_permissions(&app.pool, "ingest:read").await;
    let cookie = common::session_cookie_for_user(&app.pool, user).await;

    let body = json!({"object": "public.orders", "action": "clear"}).to_string();
    let (status, _) = post_run(&app, &cookie, "conn-no-permission", Some(&body)).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(launched_configs(&server).await.is_empty());
}

/// `GET .../ingest/runs` names the table and the action of a run of one table,
/// and `null` for a run of every table.
#[tokio::test]
async fn the_run_list_names_the_table_and_the_action_of_a_run_of_one_table() {
    let server = MockServer::start().await;
    mount_recent_runs(
        &server,
        json!([
            {
                "runId": "run-clear", "status": "SUCCESS", "startTime": 1_790_670_111.0,
                "endTime": 1_790_670_153.0,
                "runConfig": { "ops": { "run_ingest": { "config": {
                    "connector_id": "conn-run-list", "object": "public.orders", "action": "clear"
                } } } }
            },
            ingest_run_for("conn-run-list", "run-all", "SUCCESS"),
        ]),
    )
    .await;
    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    seed_sql_connector(&app.pool, "conn-run-list").await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/connectors/conn-run-list/ingest/runs")
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let runs: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(runs[0]["runId"], "run-clear");
    assert_eq!(runs[0]["object"], "public.orders");
    assert_eq!(runs[0]["action"], "clear");
    assert_eq!(runs[1]["runId"], "run-all");
    assert_eq!(runs[1]["object"], Value::Null);
    assert_eq!(runs[1]["action"], Value::Null);
}
