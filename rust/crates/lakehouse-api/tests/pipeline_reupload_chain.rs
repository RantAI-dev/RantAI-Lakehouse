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

use common::{TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up};
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
            target_table: "orders_clean".to_owned(),
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
