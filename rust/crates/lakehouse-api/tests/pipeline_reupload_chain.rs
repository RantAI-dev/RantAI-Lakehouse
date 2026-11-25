//! HTTP tests for `POST /api/pipelines/runnable/{id}/chain-step`
//! (`BLD-14`, D8): who may record a re-upload chain outcome, the field
//! validation at the boundary, and the first-write-wins answer.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_auth::Secret;
use serde_json::{Value, json};
use tower::ServiceExt;

use std::collections::HashMap;

use common::{
    CATALOG_TENANT, TestApp, create_principal_with_permissions, session_cookie_for_seeded_user,
    session_cookie_for_user, spin_up, spin_up_with_env,
};
use lakehouse_store::pipelines::{self, CreatePipelineInput};

async fn seed_orchestrator(pool: &sqlx::PgPool, token: &str) {
    let identity = lakehouse_store::identity::create_service_identity(
        pool,
        &lakehouse_store::identity::CreateServiceIdentityInput {
            name: format!("chain-step-{}", uuid::Uuid::new_v4().simple()),
            scopes: vec!["pipeline:read".to_owned()],
            environment: "test".to_owned(),
        },
    )
    .await
    .expect("seed the orchestrator service identity");
    let id: uuid::Uuid = identity.id.parse().expect("a UUID service-identity id");
    lakehouse_auth::service_token::ensure_service_credential(
        pool,
        id,
        &Secret::new(token.to_owned()),
    )
    .await
    .expect("seed the orchestrator credential");
}

async fn seed_pipeline(pool: &sqlx::PgPool, prefix: &str, tenant_id: Option<uuid::Uuid>) -> String {
    let created = pipelines::create_pipeline(
        pool,
        &CreatePipelineInput {
            inputs: Vec::new(),
            steps: Vec::new(),
            name: format!("{prefix}-{}", uuid::Uuid::new_v4().simple()),
            kind: "batch".to_owned(),
            source_zone: "bronze".to_owned(),
            source_table: "orders".to_owned(),
            incremental_column: None,
            transforms: Vec::new(),
            fbic_enabled: false,
            target_zone: "silver".to_owned(),
            // BLD-20 enforces a unique output table across live pipelines, so
            // each seeded pipeline needs its own valid ClickHouse identifier
            // (lowercase ASCII/digits/underscores — UUID simple has no hyphens).
            target_table: format!("orders_clean_{}", uuid::Uuid::new_v4().simple()),
            schedule: "manual".to_owned(),
            owner: None,
            description: None,
            max_retries: None,
            tenant_id,
            depends_on: Vec::new(),
            connector_id: None,
            id: None,
            target_lag: None,
            run_as: None,
        },
        None,
    )
    .await
    .expect("seed a pipeline");
    created.id
}

async fn provision_tenant(pool: &sqlx::PgPool, prefix: &str) -> uuid::Uuid {
    let tenant = lakehouse_store::identity::create_tenant(
        pool,
        &lakehouse_store::identity::CreateTenantInput {
            name: prefix.to_owned(),
            slug: format!("{prefix}-{}", uuid::Uuid::new_v4().simple()),
            plan: "starter".to_owned(),
            residency: "in-region".to_owned(),
        },
    )
    .await
    .expect("create a tenant");
    tenant.id.parse().expect("a UUID tenant id")
}

async fn post_step(
    app: &TestApp,
    id: &str,
    headers: &[(&str, String)],
    body: Value,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(format!("/api/pipelines/runnable/{id}/chain-step"))
        .header("content-type", "application/json");
    for (name, value) in headers {
        builder = builder.header(*name, value.clone());
    }
    let response = app
        .router
        .clone()
        .oneshot(
            builder
                .body(Body::from(body.to_string()))
                .expect("build request"),
        )
        .await
        .expect("the router answers");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("read body");
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

fn step_body(
    outcome: &str,
    reason: Option<&str>,
    run_id: Option<&str>,
    cause: Option<&str>,
) -> Value {
    json!({
        "rootRunId": "root-1",
        "sourceTable": "bronze.orders",
        "outcome": outcome,
        "reason": reason,
        "runId": run_id,
        "causePipelineId": cause,
    })
}

/// A holder of `pipeline:write` is a person, not the orchestrator: the
/// policy row's `pipeline:read` is only a floor.
#[tokio::test]
async fn a_user_with_pipeline_write_cannot_record_a_chain_step() {
    let app = spin_up().await;
    let user = create_principal_with_permissions(&app.pool, "pipeline:write").await;
    let cookie = session_cookie_for_user(&app.pool, user).await;
    let id = seed_pipeline(&app.pool, "chain-step-user", None).await;

    let (status, _) = post_step(
        &app,
        &id,
        &[("cookie", cookie)],
        step_body("started", None, Some("run-1"), None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// The orchestrator's first write wins; a retried sensor post changes
/// nothing and is told so.
#[tokio::test]
async fn the_orchestrator_records_a_step_once() {
    let app = spin_up().await;
    let token = "chain-step-orchestrator-token";
    seed_orchestrator(&app.pool, token).await;
    let id = seed_pipeline(&app.pool, "chain-step-gold", None).await;
    let cause = seed_pipeline(&app.pool, "chain-step-silver", None).await;
    let auth = [("authorization", format!("Bearer {token}"))];

    let (status, body) = post_step(
        &app,
        &id,
        &auth,
        step_body("not_run", Some("upstream_failed"), None, Some(&cause)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["recorded"], true);

    let (status, body) = post_step(
        &app,
        &id,
        &auth,
        step_body("started", None, Some("run-2"), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["recorded"], false);

    let stored = lakehouse_store::pipeline_chain::latest_chain_step(&app.pool, &id)
        .await
        .expect("read the step")
        .expect("the first write kept the row");
    assert_eq!(stored.outcome, "not_run");
    assert_eq!(stored.reason.as_deref(), Some("upstream_failed"));
}

/// A `not_run` outcome without a reason is refused at the boundary with a
/// sentence naming the field.
#[tokio::test]
async fn a_not_run_step_without_a_reason_is_refused() {
    let app = spin_up().await;
    let token = "chain-step-no-reason-token";
    seed_orchestrator(&app.pool, token).await;
    let id = seed_pipeline(&app.pool, "chain-step-no-reason", None).await;
    let cause = seed_pipeline(&app.pool, "chain-step-cause", None).await;
    let auth = [("authorization", format!("Bearer {token}"))];

    let (status, body) = post_step(
        &app,
        &id,
        &auth,
        step_body("not_run", None, None, Some(&cause)),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body["error"].as_str().is_some_and(|s| s.contains("reason")),
        "the field is named: {body}"
    );
}

/// A `started` outcome without a run id is refused.
#[tokio::test]
async fn a_started_step_without_a_run_id_is_refused() {
    let app = spin_up().await;
    let token = "chain-step-no-run-token";
    seed_orchestrator(&app.pool, token).await;
    let id = seed_pipeline(&app.pool, "chain-step-no-run", None).await;
    let auth = [("authorization", format!("Bearer {token}"))];

    let (status, body) = post_step(&app, &id, &auth, step_body("started", None, None, None)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body["error"].as_str().is_some_and(|s| s.contains("runId")),
        "the field is named: {body}"
    );
}

/// The cause's name is shown only for pipelines in the same tenant: a
/// cross-tenant cause is refused before anything is stored.
#[tokio::test]
async fn a_cause_from_another_tenant_is_refused() {
    let app = spin_up().await;
    let token = "chain-step-cross-tenant-token";
    seed_orchestrator(&app.pool, token).await;
    let a = provision_tenant(&app.pool, "chain-tenant-a").await;
    let b = provision_tenant(&app.pool, "chain-tenant-b").await;
    let id = seed_pipeline(&app.pool, "chain-step-tenant-a", Some(a)).await;
    let cause = seed_pipeline(&app.pool, "chain-step-tenant-b", Some(b)).await;
    let auth = [("authorization", format!("Bearer {token}"))];

    let (status, body) = post_step(
        &app,
        &id,
        &auth,
        step_body("not_run", Some("upstream_failed"), None, Some(&cause)),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|s| s.contains("causePipelineId")),
        "the field is named: {body}"
    );
    assert!(
        lakehouse_store::pipeline_chain::latest_chain_step(&app.pool, &id)
            .await
            .expect("read the steps")
            .is_none()
    );
}

/// The table the re-upload loaded is a bronze location, not any string.
#[tokio::test]
async fn a_source_table_outside_bronze_is_refused() {
    let app = spin_up().await;
    let token = "chain-step-silver-token";
    seed_orchestrator(&app.pool, token).await;
    let id = seed_pipeline(&app.pool, "chain-step-silver-source", None).await;
    let auth = [("authorization", format!("Bearer {token}"))];

    let mut body = step_body("started", None, Some("run-1"), None);
    body["sourceTable"] = json!("silver.orders");
    let (status, response) = post_step(&app, &id, &auth, body).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        response["error"]
            .as_str()
            .is_some_and(|s| s.contains("sourceTable")),
        "the field is named: {response}"
    );
}

/// A reader with `pipeline:read` in one tenant: the page is an ordinary
/// permission-gated read, not an orchestrator-only route.
async fn reader_in(app: &TestApp, tenant: uuid::Uuid) -> String {
    let user = create_principal_with_permissions(&app.pool, "pipeline:read").await;
    sqlx::query(
        "INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2) \
         ON CONFLICT DO NOTHING",
    )
    .bind(user)
    .bind(tenant)
    .execute(&app.pool)
    .await
    .expect("put the reader in the tenant");
    session_cookie_for_user(&app.pool, user).await
}

async fn get_chain(app: &TestApp, id: &str, headers: &[(&str, String)]) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("GET")
        .uri(format!("/api/pipelines/{id}/reupload-chain"));
    for (name, value) in headers {
        builder = builder.header(*name, value.clone());
    }
    let response = app
        .router
        .clone()
        .oneshot(builder.body(Body::empty()).expect("build request"))
        .await
        .expect("the router answers");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("read body");
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

/// Before a re-upload there is no step to show: the page reads `null`.
#[tokio::test]
async fn the_page_reads_null_before_any_reupload() {
    let app = spin_up().await;
    let tenant: uuid::Uuid = CATALOG_TENANT.parse().expect("the seeded catalog tenant");
    let id = seed_pipeline(&app.pool, "chain-read-empty", Some(tenant)).await;
    let cookie = reader_in(&app, tenant).await;

    let (status, body) = get_chain(&app, &id, &[("cookie", cookie)]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["last"], Value::Null);
}

/// The page reads the newest step, with the cause's name when it is in the
/// same tenant.
#[tokio::test]
async fn the_page_reads_the_newest_step_with_the_cause_name() {
    let app = spin_up().await;
    let tenant: uuid::Uuid = CATALOG_TENANT.parse().expect("the seeded catalog tenant");
    let id = seed_pipeline(&app.pool, "chain-read-gold", Some(tenant)).await;
    let cause_id = seed_pipeline(&app.pool, "chain-read-silver", Some(tenant)).await;
    let cause = lakehouse_store::pipelines::get_pipeline(&app.pool, &cause_id)
        .await
        .expect("read the cause")
        .expect("the cause exists");
    seed_orchestrator(&app.pool, "chain-read-token").await;
    let token = [("authorization", "Bearer chain-read-token".to_owned())];
    post_step(
        &app,
        &id,
        &token,
        step_body("started", None, Some("run-1"), None),
    )
    .await;
    let mut newer = step_body("not_run", Some("upstream_failed"), None, Some(&cause_id));
    newer["rootRunId"] = json!("root-2");
    post_step(&app, &id, &token, newer).await;

    let cookie = reader_in(&app, tenant).await;
    let (status, body) = get_chain(&app, &id, &[("cookie", cookie)]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["last"]["rootRunId"], "root-2");
    assert_eq!(body["last"]["outcome"], "not_run");
    assert_eq!(body["last"]["reason"], "upstream_failed");
    assert_eq!(body["last"]["causePipelineId"], cause_id);
    assert_eq!(body["last"]["causePipelineName"], cause.name);
}

/// Another tenant's id and an unknown id are the same 404 on the page read.
#[tokio::test]
async fn another_tenants_pipeline_answers_the_same_404_as_an_unknown_one() {
    let app = spin_up().await;
    let reader_tenant = provision_tenant(&app.pool, "chain-reader-tenant").await;
    let other_tenant = provision_tenant(&app.pool, "chain-other-tenant").await;
    let other = seed_pipeline(&app.pool, "chain-read-other", Some(other_tenant)).await;
    let headers = [("cookie", reader_in(&app, reader_tenant).await)];

    let (foreign_status, foreign_body) = get_chain(&app, &other, &headers).await;
    let (unknown_status, unknown_body) = get_chain(&app, "pl-never-existed", &headers).await;
    assert_eq!(foreign_status, StatusCode::NOT_FOUND);
    assert_eq!(foreign_status, unknown_status);
    // Both bodies name only the id the caller sent; neither reveals
    // anything from the other tenant.
    assert_eq!(
        foreign_body["error"].as_str(),
        Some(format!("Pipeline {other} not found").as_str())
    );
    assert_eq!(
        unknown_body["error"].as_str(),
        Some("Pipeline pl-never-existed not found")
    );
}

/// Pausing stops the chain sensor as well as the `_after` sensor and the
/// cron schedule; resuming starts it. BLD-14 D6's sensor ships RUNNING, so
/// an untouched stored state would keep starting the chain after a pause.
#[tokio::test]
async fn pausing_a_pipeline_stops_its_chain_sensor_and_resuming_starts_it() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/graphql"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "data": {
                "startSchedule": { "__typename": "ScheduleStateResult" },
                "stopRunningSchedule": { "__typename": "ScheduleStateResult" },
                "sensorOrError": {
                    "__typename": "Sensor",
                    "sensorState": { "id": "state-id", "status": "RUNNING" }
                },
                "stopSensor": {
                    "__typename": "StopSensorMutationResult",
                    "instigationState": { "id": "state-id", "status": "STOPPED" }
                },
                "startSensor": { "__typename": "Sensor", "sensorState": { "status": "RUNNING" } },
                "reloadLocation": {
                    "__typename": "WorkspaceLocationEntry",
                    "name": "dispar_orchestrate.definitions"
                }
            }
        })))
        .mount(&server)
        .await;
    let mut env = HashMap::new();
    env.insert(
        "DAGSTER_URL".to_owned(),
        format!("{}/graphql", server.uri()),
    );
    let app = spin_up_with_env(&env).await;
    let tenant: uuid::Uuid = CATALOG_TENANT.parse().expect("the seeded catalog tenant");
    let id = seed_pipeline(&app.pool, "chain-pause", Some(tenant)).await;
    lakehouse_store::pipelines::set_status(&app.pool, &id, "ready")
        .await
        .expect("mark it ready");
    let cookie = session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;

    for action in ["pause", "resume"] {
        let response = app
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/pipelines/{id}/{action}"))
                    .header("cookie", cookie.clone())
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("the router answers");
        assert!(
            response.status().is_success(),
            "{action}: {}",
            response.status()
        );
    }

    let requests = server
        .received_requests()
        .await
        .expect("the received requests");
    let bodies: Vec<String> = requests
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect();
    // `authored__<safe_id>_chain`: safe_id replaces `-` with `_`, the same
    // `_dagster_safe_name` rule the factory and the Rust helper use.
    let chain = format!("authored__{}_chain", id.replace('-', "_"));
    assert!(
        bodies
            .iter()
            .any(|b| b.contains(&chain) && b.contains("sensorOrError")),
        "pausing looked up the chain sensor's stored state: {bodies:?}"
    );
    // `stopSensor` carries only the instigation-state id, not the name:
    // two stops (after and chain) are the proof the chain was switched.
    assert_eq!(
        bodies.iter().filter(|b| b.contains("stopSensor")).count(),
        2,
        "pausing stopped both sensors: {bodies:?}"
    );
    assert!(
        bodies
            .iter()
            .any(|b| b.contains(&chain) && b.contains("startSensor")),
        "resuming started the chain sensor: {bodies:?}"
    );
}

/// BLD-14 review R5: a `runId` is an orchestrator run id, not arbitrary
/// text. The POST route validates its shape and length like `rootRunId`.
#[tokio::test]
async fn a_run_id_with_illegal_characters_or_over_sixty_four_bytes_is_refused() {
    let app = spin_up().await;
    let token = "chain-step-bad-run-token";
    seed_orchestrator(&app.pool, token).await;
    let id = seed_pipeline(&app.pool, "chain-step-bad-run", None).await;
    let auth = [("authorization", format!("Bearer {token}"))];

    for bad in ["not a run!", &"x".repeat(65)] {
        let mut body = step_body("started", None, Some(bad), None);
        body["runId"] = json!(bad);
        let (status, response) = post_step(&app, &id, &auth, body).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad:?}");
        assert!(
            response["error"]
                .as_str()
                .is_some_and(|s| s.contains("runId")),
            "the field is named: {response}"
        );
    }
}

/// BLD-14 review R5: the page's GET route belongs with `runs` in
/// `routes/pipelines.rs`, not in the orchestrator-only
/// `authored_pipelines_router` next to `compile`/`lag-check`.
#[test]
fn the_reupload_chain_read_is_registered_next_to_runs() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let pipelines = std::fs::read_to_string(format!("{manifest}/src/routes/pipelines.rs"))
        .expect("read pipelines.rs");
    let mod_rs =
        std::fs::read_to_string(format!("{manifest}/src/routes/mod.rs")).expect("read mod.rs");
    let authored = std::fs::read_to_string(format!("{manifest}/src/routes/authored_pipelines.rs"))
        .expect("read authored_pipelines.rs");
    assert!(
        pipelines.contains("pub async fn reupload_chain"),
        "the chain read's handler lives with runs in pipelines.rs"
    );
    let runs_at = mod_rs
        .find(".route(\"/api/pipelines/{id}/runs\"")
        .expect("the runs route is registered");
    let chain_at = mod_rs
        .find("\"/api/pipelines/{id}/reupload-chain\"")
        .expect("the chain read is registered");
    assert!(
        chain_at > runs_at,
        "the chain read is registered beside runs, not in the authored-only router"
    );
    assert!(
        !authored.contains("reupload-chain"),
        "the chain read is not an orchestrator-only authored route"
    );
}

/// An unknown authored id is a 404, not a stored orphan row.
#[tokio::test]
async fn an_unknown_pipeline_answers_404() {
    let app = spin_up().await;
    let token = "chain-step-unknown-token";
    seed_orchestrator(&app.pool, token).await;
    let auth = [("authorization", format!("Bearer {token}"))];

    let (status, _) = post_step(
        &app,
        "pl-never-existed",
        &auth,
        step_body("started", None, Some("run-1"), None),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
