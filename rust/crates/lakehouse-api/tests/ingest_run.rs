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
#![allow(
    clippy::cast_precision_loss,
    reason = "test fixture times are real-world Unix seconds, cast to f64 to match the \
              orchestrator's wire shape; no meaningful precision is lost"
)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{body_partial_json, body_string_contains, method, path};
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
    // QA `TC-1702` (SRC-39): "a run is going" now means it started recently —
    // a run older than the limit plus its grace is cancelled by the route
    // instead of answering 409. The fixed September 2026 time was older than
    // the limit, so this fixture now says what the 409 tests assert; no
    // assertion is weakened.
    let started = time::OffsetDateTime::now_utc().unix_timestamp() as f64 - 60.0;
    json!({
        "runId": run_id, "status": status, "startTime": started, "endTime": null,
        "creationTime": started,
        "runConfig": { "ops": { "run_ingest": { "config": { "connector_id": connector_id } } } }
    })
}

/// A run that `Dagster` says was created at `created` and, when it started,
/// at `started` — for the overdue tests (`SRC-39`, QA `TC-1702`).
fn ingest_run_created_at(
    connector_id: &str,
    run_id: &str,
    status: &str,
    created: f64,
    started: Option<f64>,
) -> Value {
    json!({
        "runId": run_id, "status": status, "startTime": started, "endTime": null,
        "creationTime": created,
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

/// `SRC-11` review 1 SHOULD-FIX 1: a person's launch carries the run tag
/// `lakehouse/trigger: user`, which the orchestrator's start report reads (a
/// run with no tag is recorded as `other`). The mock answers a launch only
/// when the request body carries the tag, so a launch without it fails the
/// route.
#[tokio::test]
async fn ingest_run_tags_the_launch_as_started_by_a_user() {
    let server = MockServer::start().await;
    mount_recent_runs(&server, json!([])).await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("launchRun"))
        .and(body_partial_json(json!({
            "variables": { "tags": [{ "key": "lakehouse/trigger", "value": "user" }] }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "launchRun": { "__typename": "LaunchRunSuccess", "run": { "runId": "run-tagged" } } }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-ingest-tag").await;

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/connectors/conn-ingest-tag/ingest/run")
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
    assert_eq!(body["runId"], "run-tagged");
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
    // The reason must say what actually starts ingestion now (Kafka Connect and
    // the applier), not name the removed `debezium-<id>` compose service
    // (`SRC-4` slice 2, decision S2-h).
    assert!(reason.contains("Kafka Connect"), "{reason}");
    assert!(!reason.contains("compose service"), "{reason}");
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
/// `PythonError`, ...) is a 422 with the FIXED classified body
/// `"Dagster did not accept the ingest run"` (F2.7 — never the raw
/// upstream `message` text, never a 200 claiming success, never the
/// raw transport-error branch (Z9's trap)).
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
    // SEC-11 + F2.7: this used to assert that Dagster's own message ("job not
    // found") reached the body; that asserted the bug. The body is now the
    // fixed sentence `LaunchFailure` allows, and the orchestrator's text is
    // only in the log.
    assert_eq!(body["error"], "Dagster did not accept the ingest run");
    assert!(!body.to_string().contains("job not found"), "{body}");
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

/// Any other Dagster launch failure keeps the generic 422: a fixed sentence,
/// no code, no upstream text. Words that merely look like "not loaded" in the
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
    // F2.7: the generic 422 body is the fixed sentence; the reference moved
    // to the dagster crate's log line with the raw text.
    assert_eq!(body["error"], "Dagster did not accept the ingest run");
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

/// The `terminateRun` requests the mock received, as their JSON bodies.
async fn cancel_requests(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .expect("request log")
        .iter()
        .map(|request| String::from_utf8_lossy(&request.body).into_owned())
        .filter(|body| body.contains("terminateRun"))
        .collect()
}

/// The runs of `connector_id` in the product's own record.
async fn recorded_runs(
    pool: &sqlx::PgPool,
    connector_id: &str,
) -> Vec<lakehouse_store::connector_run::RunSummary> {
    lakehouse_store::connector_run::list_runs(pool, connector_id, 50)
        .await
        .expect("the run history")
}

/// QA `TC-1702`, `LH-SRC-11-11` (`SRC-39`, D10): a run going for seven hours
/// no longer holds the connector. The route asks for it to be cancelled for
/// good, records it as failed with `timed_out`, and starts the next run.
#[tokio::test]
async fn a_run_older_than_the_limit_is_cancelled_and_the_next_run_starts() {
    let server = MockServer::start().await;
    let now = time::OffsetDateTime::now_utc().unix_timestamp() as f64;
    let started = now - 7.0 * 3600.0;
    mount_recent_runs(
        &server,
        json!([ingest_run_created_at(
            "conn-ingest-old",
            "run-old",
            "STARTED",
            started,
            Some(started)
        )]),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("terminateRun"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "terminateRun": { "__typename": "TerminateRunSuccess",
                "run": { "runId": "run-old" } } }
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("launchRun"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "launchRun": { "__typename": "LaunchRunSuccess", "run": { "runId": "run-new" } } }
        })))
        .mount(&server)
        .await;

    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-ingest-old").await;
    lakehouse_store::connector_run::start_run(
        &app.pool,
        "conn-ingest-old",
        "run-old",
        lakehouse_store::connector_run::RunTrigger::Schedule,
        None,
        time::OffsetDateTime::now_utc() - time::Duration::hours(7),
    )
    .await
    .expect("a record of the old run");

    let (status, answer) = post_run(&app, &cookie, "conn-ingest-old", None).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["runId"], "run-new");

    let cancels = cancel_requests(&server).await;
    assert_eq!(cancels.len(), 1, "{cancels:?}");
    assert!(
        cancels[0].contains("MARK_AS_CANCELED_IMMEDIATELY"),
        "{cancels:?}"
    );
    assert!(cancels[0].contains("run-old"), "{cancels:?}");

    let runs = recorded_runs(&app.pool, "conn-ingest-old").await;
    assert_eq!(runs[0].status, "failed");
    assert_eq!(runs[0].end_reason.as_deref(), Some("timed_out"));
}

/// A run that never started stops holding the connector after the start
/// limit: it is cancelled, no record is invented for it (D13), and the launch
/// goes on.
#[tokio::test]
async fn a_run_that_never_started_stops_locking_the_connector_after_the_start_limit() {
    let server = MockServer::start().await;
    let now = time::OffsetDateTime::now_utc().unix_timestamp() as f64;
    mount_recent_runs(
        &server,
        json!([ingest_run_created_at(
            "conn-ingest-queued",
            "run-queued",
            "NOT_STARTED",
            now - 20.0 * 60.0,
            None
        )]),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("terminateRun"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "terminateRun": { "__typename": "TerminateRunSuccess",
                "run": { "runId": "run-queued" } } }
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("launchRun"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "launchRun": { "__typename": "LaunchRunSuccess", "run": { "runId": "run-new" } } }
        })))
        .mount(&server)
        .await;

    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-ingest-queued").await;

    let (status, answer) = post_run(&app, &cookie, "conn-ingest-queued", None).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["runId"], "run-new");
    let cancels = cancel_requests(&server).await;
    assert_eq!(cancels.len(), 1, "{cancels:?}");
    assert!(
        cancels[0].contains("MARK_AS_CANCELED_IMMEDIATELY"),
        "{cancels:?}"
    );

    let (records,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM connector_run WHERE connector_id = $1")
            .bind("conn-ingest-queued")
            .fetch_one(&app.pool)
            .await
            .expect("count the records");
    assert_eq!(
        records, 0,
        "nothing is invented for a run that never started"
    );
}

/// An orchestrator that refuses the cancel does not keep the connector
/// locked, and its own text never reaches the response (SEC-11).
#[tokio::test]
async fn an_orchestrator_that_refuses_the_cancel_does_not_keep_the_connector_locked() {
    let server = MockServer::start().await;
    let now = time::OffsetDateTime::now_utc().unix_timestamp() as f64;
    let started = now - 7.0 * 3600.0;
    mount_recent_runs(
        &server,
        json!([ingest_run_created_at(
            "conn-ingest-stuck",
            "run-stuck",
            "STARTED",
            started,
            Some(started)
        )]),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("terminateRun"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "terminateRun": { "__typename": "TerminateRunFailure",
                "message": "the run is already gone from the worker" } }
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("launchRun"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "launchRun": { "__typename": "LaunchRunSuccess", "run": { "runId": "run-new" } } }
        })))
        .mount(&server)
        .await;

    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-ingest-stuck").await;

    let (status, answer) = post_run(&app, &cookie, "conn-ingest-stuck", None).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["runId"], "run-new");
    assert!(
        !answer
            .to_string()
            .contains("the run is already gone from the worker"),
        "{answer}"
    );
}

/// `review round 1, R1` (QA `TC-1702`): the orchestrator queues every run and
/// its monitoring never ends a queued one, so a run waiting in the queue for
/// 20 minutes still holds the connector: the 409 stays and nothing is
/// cancelled. Before the fix the route cancelled it, and "Run now" replaced a
/// healthy queued run without a word.
#[tokio::test]
async fn a_queued_run_still_holds_the_connector_after_the_start_limit() {
    let server = MockServer::start().await;
    let now = time::OffsetDateTime::now_utc().unix_timestamp() as f64;
    mount_recent_runs(
        &server,
        json!([ingest_run_created_at(
            "conn-ingest-queued-long",
            "run-waiting",
            "QUEUED",
            now - 20.0 * 60.0,
            None
        )]),
    )
    .await;
    // No `terminateRun` mock: a cancel would reach `spin_up`'s dead upstream
    // and turn the answer into a 503, so this test also proves none is sent.
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
    seed_sql_connector(&app.pool, "conn-ingest-queued-long").await;

    let (status, answer) = post_run(&app, &cookie, "conn-ingest-queued-long", None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{answer}");
    assert!(
        answer["error"]
            .as_str()
            .unwrap_or_default()
            .contains("already queued"),
        "{answer}"
    );
    assert_eq!(cancel_requests(&server).await, [] as [String; 0]);
}

/// `review round 1, R2` (QA `TC-1702`): an orchestrator that cannot be reached
/// for the cancel is a retryable 503, nothing is written and nothing is
/// launched — the old run may still be loading, and Bronze is append-only.
/// Before the fix the route closed the record as "stopped", logged a stop and
/// launched on top of the live run.
#[tokio::test]
async fn an_orchestrator_that_cannot_be_reached_for_the_cancel_answers_503_and_starts_nothing() {
    let server = MockServer::start().await;
    let now = time::OffsetDateTime::now_utc().unix_timestamp() as f64;
    let started = now - 7.0 * 3600.0;
    mount_recent_runs(
        &server,
        json!([ingest_run_created_at(
            "conn-ingest-cancel-down",
            "run-down",
            "STARTED",
            started,
            Some(started)
        )]),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("terminateRun"))
        .respond_with(ResponseTemplate::new(500).set_body_string("upstream is down"))
        .mount(&server)
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
    seed_sql_connector(&app.pool, "conn-ingest-cancel-down").await;
    lakehouse_store::connector_run::start_run(
        &app.pool,
        "conn-ingest-cancel-down",
        "run-down",
        lakehouse_store::connector_run::RunTrigger::Schedule,
        None,
        time::OffsetDateTime::now_utc() - time::Duration::hours(7),
    )
    .await
    .expect("a record of the old run");

    let (status, answer) = post_run(&app, &cookie, "conn-ingest-cancel-down", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{answer}");
    assert!(
        !answer.to_string().contains("upstream is down"),
        "the orchestrator's own text never reaches the response: {answer}"
    );
    assert_eq!(
        launched_configs(&server).await,
        [] as [Value; 0],
        "nothing is launched while the old run may still be going"
    );
    let runs = recorded_runs(&app.pool, "conn-ingest-cancel-down").await;
    assert_eq!(runs[0].status, "running", "nothing was written");
    assert_eq!(runs[0].end_reason, None);
}

// --- SRC-43: a run record closed when its run list is read -------------------

/// The connector's run list as a person reads it.
async fn get_runs(app: &TestApp, cookie: &str, id: &str) -> (StatusCode, Value) {
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/connectors/{id}/runs"))
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
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

/// Answers every `pipelineRunOrError` question about `run_id` with `answer`.
async fn mount_run_status(server: &MockServer, run_id: &str, answer: Value) {
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("pipelineRunOrError"))
        .and(body_partial_json(json!({"variables": {"rid": run_id}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "pipelineRunOrError": answer }
        })))
        .mount(server)
        .await;
}

/// The `pipelineRunOrError` requests the mock received, as their JSON bodies.
async fn run_status_requests(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .expect("request log")
        .iter()
        .map(|request| String::from_utf8_lossy(&request.body).into_owned())
        .filter(|body| body.contains("pipelineRunOrError"))
        .collect()
}

/// `SRC-43` (#485 review, round 2 (R2)): the exact state after the two-step
/// failure of "Run now" — the orchestrator cancelled the run, and the store
/// failed before the record was closed. One read of the run list closes the
/// record as the cancel would have, and a second read writes nothing and asks
/// nothing.
#[tokio::test]
async fn a_record_left_running_after_its_run_was_cancelled_is_closed_by_one_read_of_the_run_list() {
    let server = MockServer::start().await;
    let now = time::OffsetDateTime::now_utc().unix_timestamp() as f64;
    mount_run_status(
        &server,
        "run-cancelled",
        json!({
            "__typename": "Run", "status": "CANCELED",
            "startTime": now - 7.0 * 3600.0, "endTime": now - 60.0, "stepStats": [],
        }),
    )
    .await;
    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-ingest-reconcile").await;
    lakehouse_store::connector_run::start_run(
        &app.pool,
        "conn-ingest-reconcile",
        "run-cancelled",
        lakehouse_store::connector_run::RunTrigger::Schedule,
        None,
        time::OffsetDateTime::now_utc() - time::Duration::hours(7),
    )
    .await
    .expect("a record of the cancelled run");

    let (status, listed) = get_runs(&app, &cookie, "conn-ingest-reconcile").await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let run = &listed["runs"][0];
    assert_eq!(run["status"], "failed", "{listed}");
    assert_eq!(run["endReason"], "timed_out", "{listed}");
    assert!(
        !run["endedAt"].is_null(),
        "the record carries the orchestrator's end: {listed}"
    );

    let stored = recorded_runs(&app.pool, "conn-ingest-reconcile").await;
    assert_eq!(stored[0].status, "failed");
    assert_eq!(stored[0].end_reason.as_deref(), Some("timed_out"));
    let log = lakehouse_store::connector_run::get_run_log(
        &app.pool,
        "conn-ingest-reconcile",
        "run-cancelled",
    )
    .await
    .expect("the run log")
    .expect("a recorded run");
    let ended: Vec<_> = log
        .lines
        .iter()
        .filter(|line| line.event == "run_ended")
        .collect();
    assert_eq!(ended.len(), 1, "{:?}", log.lines);
    assert_eq!(
        ended[0].text,
        "run ended: failed, the run did not finish in 6 h and was stopped"
    );

    // A second read answers the same, writes nothing and asks nothing.
    let (status, again) = get_runs(&app, &cookie, "conn-ingest-reconcile").await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["runs"][0]["status"], "failed", "{again}");
    let log = lakehouse_store::connector_run::get_run_log(
        &app.pool,
        "conn-ingest-reconcile",
        "run-cancelled",
    )
    .await
    .expect("the run log")
    .expect("a recorded run");
    assert_eq!(
        log.lines
            .iter()
            .filter(|line| line.event == "run_ended")
            .count(),
        1,
        "the record is closed once"
    );
    assert_eq!(
        run_status_requests(&server).await.len(),
        1,
        "the second read asks nothing: the record no longer says running"
    );

    // Closing the record is all the read does: nothing is counted.
    let counted: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pipeline_run_event WHERE run_id = $1")
            .bind("run-cancelled")
            .fetch_one(&app.pool)
            .await
            .expect("count the events");
    assert_eq!(counted, 0, "the list read counts no run");
    let streak: i32 = sqlx::query_scalar("SELECT failure_streak FROM connector WHERE id = $1")
        .bind("conn-ingest-reconcile")
        .fetch_one(&app.pool)
        .await
        .expect("read the connector's streak");
    assert_eq!(streak, 0, "the list read moves no streak");
}

/// `SRC-43` D13: a run cancelled before its limit is closed as `failed` with
/// the reason `other`. The record has no word for cancelled and
/// `table_failed` would invent a cause.
#[tokio::test]
async fn a_run_cancelled_before_its_limit_is_closed_as_failed_with_no_reason_invented() {
    let server = MockServer::start().await;
    let now = time::OffsetDateTime::now_utc().unix_timestamp() as f64;
    mount_run_status(
        &server,
        "run-stopped",
        json!({
            "__typename": "Run", "status": "CANCELED",
            "startTime": now - 600.0, "endTime": now - 300.0, "stepStats": [],
        }),
    )
    .await;
    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-ingest-stopped").await;
    lakehouse_store::connector_run::start_run(
        &app.pool,
        "conn-ingest-stopped",
        "run-stopped",
        lakehouse_store::connector_run::RunTrigger::User,
        None,
        time::OffsetDateTime::now_utc() - time::Duration::minutes(10),
    )
    .await
    .expect("a record of the stopped run");

    let (status, listed) = get_runs(&app, &cookie, "conn-ingest-stopped").await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["runs"][0]["status"], "failed", "{listed}");
    assert_eq!(listed["runs"][0]["endReason"], "other", "{listed}");
    let log = lakehouse_store::connector_run::get_run_log(
        &app.pool,
        "conn-ingest-stopped",
        "run-stopped",
    )
    .await
    .expect("the run log")
    .expect("a recorded run");
    assert_eq!(
        log.lines.last().expect("a last line").text,
        "run ended: failed, no further reason is recorded"
    );
}

/// `SRC-43` D12: a record left `running` after its run succeeded or failed is
/// closed by the list read exactly as the sensor's report would close it, and
/// nothing is counted.
#[tokio::test]
async fn a_record_left_running_after_its_run_succeeded_or_failed_is_closed_as_the_run_report_would_close_it()
 {
    let server = MockServer::start().await;
    let now = time::OffsetDateTime::now_utc().unix_timestamp() as f64;
    for (run_id, run_status) in [("run-won", "SUCCESS"), ("run-lost", "FAILURE")] {
        mount_run_status(
            &server,
            run_id,
            json!({
                "__typename": "Run", "status": run_status,
                "startTime": now - 120.0, "endTime": now - 60.0, "stepStats": [],
            }),
        )
        .await;
    }
    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-ingest-open-times").await;
    for (run_id, trigger) in [
        (
            "run-won",
            lakehouse_store::connector_run::RunTrigger::Schedule,
        ),
        ("run-lost", lakehouse_store::connector_run::RunTrigger::User),
    ] {
        lakehouse_store::connector_run::start_run(
            &app.pool,
            "conn-ingest-open-times",
            run_id,
            trigger,
            None,
            time::OffsetDateTime::now_utc() - time::Duration::minutes(10),
        )
        .await
        .expect("a record of the run");
    }

    let (status, listed) = get_runs(&app, &cookie, "conn-ingest-open-times").await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let runs = listed["runs"].as_array().expect("the runs");
    let won = runs
        .iter()
        .find(|run| run["runId"] == "run-won")
        .expect("the succeeded run");
    assert_eq!(won["status"], "succeeded", "{won}");
    assert_eq!(won["endReason"], "completed", "{won}");
    let lost = runs
        .iter()
        .find(|run| run["runId"] == "run-lost")
        .expect("the failed run");
    assert_eq!(lost["status"], "failed", "{lost}");
    assert_eq!(lost["endReason"], "table_failed", "{lost}");

    for run_id in ["run-won", "run-lost"] {
        let counted: i64 =
            sqlx::query_scalar("SELECT count(*) FROM pipeline_run_event WHERE run_id = $1")
                .bind(run_id)
                .fetch_one(&app.pool)
                .await
                .expect("count the events");
        assert_eq!(counted, 0, "the list read counts no run");
    }
}

/// `SRC-43` D11: a run that is still going, or still being cancelled, keeps
/// its record `running`. Every such word was asked about, none was guessed at.
#[tokio::test]
async fn a_run_that_is_still_going_keeps_its_record_running() {
    let server = MockServer::start().await;
    let now = time::OffsetDateTime::now_utc().unix_timestamp() as f64;
    let words = [
        "QUEUED",
        "NOT_STARTED",
        "STARTING",
        "STARTED",
        "MANAGED",
        "CANCELING",
    ];
    for (n, word) in words.iter().enumerate() {
        mount_run_status(
            &server,
            &format!("run-going-{n}"),
            json!({
                "__typename": "Run", "status": word,
                "startTime": now - 60.0, "endTime": null, "stepStats": [],
            }),
        )
        .await;
    }
    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-ingest-going").await;
    for n in 0..words.len() {
        lakehouse_store::connector_run::start_run(
            &app.pool,
            "conn-ingest-going",
            &format!("run-going-{n}"),
            lakehouse_store::connector_run::RunTrigger::Schedule,
            None,
            time::OffsetDateTime::now_utc() - time::Duration::minutes(10)
                + time::Duration::seconds(i64::try_from(n).expect("a small index")),
        )
        .await
        .expect("a record of the run");
    }

    let (status, listed) = get_runs(&app, &cookie, "conn-ingest-going").await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let runs = listed["runs"].as_array().expect("the runs");
    assert_eq!(runs.len(), words.len());
    for run in runs {
        assert_eq!(run["status"], "running", "{run}");
        assert!(run["endReason"].is_null(), "{run}");
    }
    let stored = recorded_runs(&app.pool, "conn-ingest-going").await;
    assert!(
        stored
            .iter()
            .all(|run| run.status == "running" && run.end_reason.is_none()),
        "{stored:?}"
    );
    assert_eq!(
        run_status_requests(&server).await.len(),
        words.len(),
        "every run was asked about and left alone"
    );
}

/// `SRC-43` D14: a run the orchestrator no longer knows is left as it is.
/// Nothing is guessed, and the orchestrator's own message never reaches the
/// answer.
#[tokio::test]
async fn a_record_whose_run_the_orchestrator_does_not_know_is_left_running() {
    let server = MockServer::start().await;
    mount_run_status(
        &server,
        "run-gone",
        json!({
            "__typename": "RunNotFoundError",
            "message": "the run is gone from the store",
        }),
    )
    .await;
    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-ingest-gone").await;
    lakehouse_store::connector_run::start_run(
        &app.pool,
        "conn-ingest-gone",
        "run-gone",
        lakehouse_store::connector_run::RunTrigger::Schedule,
        None,
        time::OffsetDateTime::now_utc() - time::Duration::minutes(10),
    )
    .await
    .expect("a record of the run");

    let (status, listed) = get_runs(&app, &cookie, "conn-ingest-gone").await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["runs"][0]["status"], "running", "{listed}");
    assert!(listed["runs"][0]["endReason"].is_null(), "{listed}");
    assert!(
        !listed
            .to_string()
            .contains("the run is gone from the store"),
        "the orchestrator's own text never reaches the response: {listed}"
    );
    let log =
        lakehouse_store::connector_run::get_run_log(&app.pool, "conn-ingest-gone", "run-gone")
            .await
            .expect("the run log")
            .expect("a recorded run");
    assert!(
        !log.lines.iter().any(|line| line.event == "run_ended"),
        "nothing is written for a run the orchestrator does not know"
    );
    assert_eq!(run_status_requests(&server).await.len(), 1);
}

/// `SRC-43` D15: the list answers even when the orchestrator cannot be asked.
/// The loop stops at the first failure, so a dead orchestrator costs one call,
/// and its text never reaches the answer.
#[tokio::test]
async fn the_run_list_still_answers_when_the_orchestrator_cannot_be_asked() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("pipelineRunOrError"))
        .respond_with(ResponseTemplate::new(500).set_body_string("upstream is down"))
        .mount(&server)
        .await;
    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-ingest-down").await;
    for run_id in ["run-down-1", "run-down-2"] {
        lakehouse_store::connector_run::start_run(
            &app.pool,
            "conn-ingest-down",
            run_id,
            lakehouse_store::connector_run::RunTrigger::Schedule,
            None,
            time::OffsetDateTime::now_utc() - time::Duration::minutes(10),
        )
        .await
        .expect("a record of the run");
    }

    let (status, listed) = get_runs(&app, &cookie, "conn-ingest-down").await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let runs = listed["runs"].as_array().expect("the runs");
    assert_eq!(runs.len(), 2);
    assert!(
        runs.iter()
            .all(|run| run["status"] == "running" && run["endReason"].is_null()),
        "{listed}"
    );
    assert!(
        !listed.to_string().contains("upstream is down"),
        "the orchestrator's own text never reaches the response: {listed}"
    );
    assert_eq!(
        run_status_requests(&server).await.len(),
        1,
        "the loop stops at the first failure"
    );
}

/// `SRC-43` D17: two reads at once close the record once and write one line.
#[tokio::test]
async fn two_reads_at_once_close_a_record_once() {
    let server = MockServer::start().await;
    let now = time::OffsetDateTime::now_utc().unix_timestamp() as f64;
    mount_run_status(
        &server,
        "run-racing",
        json!({
            "__typename": "Run", "status": "CANCELED",
            "startTime": now - 7.0 * 3600.0, "endTime": now - 60.0, "stepStats": [],
        }),
    )
    .await;
    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-ingest-racing").await;
    lakehouse_store::connector_run::start_run(
        &app.pool,
        "conn-ingest-racing",
        "run-racing",
        lakehouse_store::connector_run::RunTrigger::Schedule,
        None,
        time::OffsetDateTime::now_utc() - time::Duration::hours(7),
    )
    .await
    .expect("a record of the cancelled run");

    let (first, second) = tokio::join!(
        get_runs(&app, &cookie, "conn-ingest-racing"),
        get_runs(&app, &cookie, "conn-ingest-racing"),
    );
    assert_eq!(first.0, StatusCode::OK, "{}", first.1);
    assert_eq!(second.0, StatusCode::OK, "{}", second.1);

    let stored = recorded_runs(&app.pool, "conn-ingest-racing").await;
    assert_eq!(stored[0].status, "failed");
    assert_eq!(stored[0].end_reason.as_deref(), Some("timed_out"));
    let log =
        lakehouse_store::connector_run::get_run_log(&app.pool, "conn-ingest-racing", "run-racing")
            .await
            .expect("the run log")
            .expect("a recorded run");
    assert_eq!(
        log.lines
            .iter()
            .filter(|line| line.event == "run_ended")
            .count(),
        1,
        "the record is closed once"
    );
}

/// `SRC-43` D16 and D18: a list with no `running` record asks the orchestrator
/// nothing, and the read invents no record.
#[tokio::test]
async fn a_connector_with_no_running_record_asks_the_orchestrator_nothing() {
    let server = MockServer::start().await;
    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-ingest-ended").await;
    let started = time::OffsetDateTime::now_utc() - time::Duration::minutes(10);
    lakehouse_store::connector_run::start_run(
        &app.pool,
        "conn-ingest-ended",
        "run-ended",
        lakehouse_store::connector_run::RunTrigger::Schedule,
        None,
        started,
    )
    .await
    .expect("a record of the run");
    lakehouse_store::connector_run::end_run(
        &app.pool,
        "conn-ingest-ended",
        "run-ended",
        lakehouse_store::connector_run::RunStatus::Succeeded,
        lakehouse_store::connector_run::EndReason::Completed,
        started + time::Duration::minutes(5),
    )
    .await
    .expect("the run's end");

    let (status, listed) = get_runs(&app, &cookie, "conn-ingest-ended").await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let runs = listed["runs"].as_array().expect("the runs");
    assert_eq!(runs.len(), 1, "{listed}");
    assert_eq!(runs[0]["status"], "succeeded");
    assert_eq!(run_status_requests(&server).await, [] as [String; 0]);
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM connector_run WHERE connector_id = $1")
            .bind("conn-ingest-ended")
            .fetch_one(&app.pool)
            .await
            .expect("count the records");
    assert_eq!(stored, 1, "the read invents no record");
}

/// `SRC-43` D16: one read asks about at most ten records that say `running`.
#[tokio::test]
async fn one_read_asks_about_at_most_ten_running_records() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("pipelineRunOrError"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "pipelineRunOrError": {
                "__typename": "Run", "status": "STARTED",
                "startTime": null, "endTime": null, "stepStats": [],
            } },
        })))
        .mount(&server)
        .await;
    let app = spin_up_with_dagster(&format!("{}/graphql", server.uri())).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    seed_sql_connector(&app.pool, "conn-ingest-many").await;
    for n in 0..12 {
        lakehouse_store::connector_run::start_run(
            &app.pool,
            "conn-ingest-many",
            &format!("run-{n:02}"),
            lakehouse_store::connector_run::RunTrigger::Schedule,
            None,
            time::OffsetDateTime::now_utc() - time::Duration::minutes(30)
                + time::Duration::seconds(n),
        )
        .await
        .expect("a record of the run");
    }

    let (status, listed) = get_runs(&app, &cookie, "conn-ingest-many").await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(
        listed["runs"].as_array().expect("the runs").len(),
        12,
        "every record is still answered"
    );
    assert_eq!(
        run_status_requests(&server).await.len(),
        10,
        "one read asks about at most ten running records"
    );
}

/// `review round 1, R3` (QA `TC-1702`): the request's own refusals come before
/// any run is touched. A body naming a table the connector does not load is a
/// 404, and the overdue run is neither cancelled nor closed for it. Before the
/// fix the guard ran first and lost the run for a typo.
#[tokio::test]
async fn a_table_the_connector_does_not_load_is_a_404_and_stops_no_run() {
    let server = MockServer::start().await;
    let now = time::OffsetDateTime::now_utc().unix_timestamp() as f64;
    let started = now - 7.0 * 3600.0;
    mount_recent_runs(
        &server,
        json!([ingest_run_created_at(
            "conn-ingest-typo",
            "run-typo",
            "STARTED",
            started,
            Some(started)
        )]),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("terminateRun"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "terminateRun": { "__typename": "TerminateRunSuccess",
                "run": { "runId": "run-typo" } } }
        })))
        .expect(0)
        .mount(&server)
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
    seed_sql_connector(&app.pool, "conn-ingest-typo").await;
    select_two_tables(&app.pool, "conn-ingest-typo").await;
    lakehouse_store::connector_run::start_run(
        &app.pool,
        "conn-ingest-typo",
        "run-typo",
        lakehouse_store::connector_run::RunTrigger::Schedule,
        None,
        time::OffsetDateTime::now_utc() - time::Duration::hours(7),
    )
    .await
    .expect("a record of the old run");

    let body = json!({"object": "public.not_a_table", "action": "clear"}).to_string();
    let (status, answer) = post_run(&app, &cookie, "conn-ingest-typo", Some(&body)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{answer}");
    assert_eq!(cancel_requests(&server).await, [] as [String; 0]);
    let runs = recorded_runs(&app.pool, "conn-ingest-typo").await;
    assert_eq!(
        runs[0].status, "running",
        "the refused request wrote nothing"
    );
}

/// `review round 1, R10` (QA `TC-1702`): the assistant's `run_ingest` has no
/// signed-in person, and stopping a run is a person's decision (its own
/// `cancel_pipeline_run` is a `WriteHigh` tool). An overdue run is the 409 for
/// the tool — no `terminateRun` is sent — and its sentence says "Run now" on
/// the connector's page, where a person decides.
#[tokio::test]
async fn the_assistants_run_ingest_answers_409_for_an_overdue_run_and_stops_nothing() {
    let server = MockServer::start().await;
    let now = time::OffsetDateTime::now_utc().unix_timestamp() as f64;
    let started = now - 7.0 * 3600.0;
    mount_recent_runs(
        &server,
        json!([ingest_run_created_at(
            "conn-ingest-tool",
            "run-tool",
            "STARTED",
            started,
            Some(started)
        )]),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("terminateRun"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "terminateRun": { "__typename": "TerminateRunSuccess",
                "run": { "runId": "run-tool" } } }
        })))
        .expect(0)
        .mount(&server)
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
    seed_sql_connector(&app.pool, "conn-ingest-tool").await;

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/tool")
                .header("content-type", "application/json")
                .header("cookie", &cookie)
                .body(Body::from(
                    json!({
                        "tool": "run_ingest",
                        "args": { "id": "conn-ingest-tool", "confirmed": true },
                        "mode": "build"
                    })
                    .to_string(),
                ))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: Value = serde_json::from_slice(&bytes).expect("valid JSON");
    // The tool answer wraps the route's body: `{"tool", "outcome", "result"}`.
    let error = body["result"]["error"]
        .as_str()
        .or_else(|| body["error"].as_str())
        .unwrap_or_default();
    assert!(error.contains("run too long"), "{body}");
    assert!(error.contains("Run now"), "{body}");
    assert_eq!(cancel_requests(&server).await, [] as [String; 0]);
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

/// `SRC-21`: the assistant's `set_ingest_spec` tool carries `loadMode` and
/// `primaryKey` through to the saved spec. Before, the schema did not list
/// them, so a model re-sending a connector's tables dropped every mode back to
/// replace. The tool forwards `sourceObjects` as given; this proves nothing on
/// the way strips the two keys.
#[tokio::test]
async fn the_assistants_set_ingest_spec_saves_load_mode_and_primary_key() {
    // The saved dial names a loopback address, which the save-time SSRF check
    // refuses unless the block is off; what is proved here is the objects.
    let app = spin_up_with_env(&HashMap::from([(
        "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
        "true".to_owned(),
    )]))
    .await;
    seed_sql_connector(&app.pool, "conn-ingest-tool-mode").await;
    // Saved dial must equal the stored one, or the save is a re-point (SEC-14)
    // and ends in a 409; the seeded host `pg-src` does not resolve here.
    sqlx::query(
        "UPDATE connector SET dial = \
         '{\"driver\":\"postgres\",\"host\":\"127.0.0.1\",\"port\":5432,\"database\":\"d\",\"user\":\"u\"}'::jsonb \
         WHERE id = 'conn-ingest-tool-mode'",
    )
    .execute(&app.pool)
    .await
    .expect("point the seeded dial at loopback");
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/tool")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "tool": "set_ingest_spec",
                        "mode": "build",
                        "args": {
                            "id": "conn-ingest-tool-mode",
                            "adapter": "sql",
                            "ingestMode": "batch",
                            "dial": {
                                "driver": "postgres", "host": "127.0.0.1", "port": 5432,
                                "database": "d", "user": "u",
                            },
                            "sourceObjects": [{
                                "name": "public.orders",
                                "target": "orders",
                                "incrementalKey": "updated_at",
                                "loadMode": "incremental_dedup",
                                "primaryKey": ["id"],
                            }],
                            "confirmed": true,
                        },
                    })
                    .to_string(),
                ))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: Value = serde_json::from_slice(&bytes).expect("valid JSON");
    assert!(body["result"].get("error").is_none(), "{body}");

    let stored: Value = sqlx::query_scalar(
        "SELECT source_objects FROM connector WHERE id = 'conn-ingest-tool-mode'",
    )
    .fetch_one(&app.pool)
    .await
    .expect("read source objects");
    assert_eq!(stored[0]["loadMode"], "incremental_dedup", "{stored}");
    assert_eq!(stored[0]["primaryKey"], json!(["id"]), "{stored}");
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
    assert_eq!(
        launched_configs(&server).await,
        [] as [serde_json::Value; 0]
    );
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
    assert_eq!(
        launched_configs(&server).await,
        [] as [serde_json::Value; 0]
    );
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
    assert_eq!(
        launched_configs(&server).await,
        [] as [serde_json::Value; 0]
    );

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
    assert_eq!(
        launched_configs(&server).await,
        [] as [serde_json::Value; 0]
    );
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
    assert_eq!(
        launched_configs(&server).await,
        [] as [serde_json::Value; 0]
    );
}
