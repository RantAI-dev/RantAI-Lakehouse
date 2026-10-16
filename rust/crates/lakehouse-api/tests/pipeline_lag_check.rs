//! HTTP-level tests for `POST /api/pipelines/runnable/{id}/lag-check`
//! (BLD-2 plan D5): who may ask, and what an honest answer looks like when
//! nothing can be measured.
//!
//! The decision arithmetic itself is proven in
//! `src/pipeline_lag.rs`'s own tests, where the token reader and the
//! orchestrator can both be handed a fixed answer. What is left for the
//! HTTP surface is the part a unit test cannot reach: the policy row, the
//! orchestrator-only gate, and the refusal shape for an id the caller may
//! not see. An id that exists in another tenant and an id that has never
//! existed must be indistinguishable — the same 404 body, not a 403 for
//! one and a 404 for the other.
//!
//! `ClickHouse` and Lakekeeper stay dead (`127.0.0.1:1`, see
//! `tests/common`), so the one 200 this file asserts is the honest one: a
//! pipeline whose input cannot be read is `not_measured`, never `met` and
//! never a run.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_auth::Secret;
use serde_json::Value;
use tower::ServiceExt;

use common::{TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up};
use lakehouse_store::pipelines::{self, CreatePipelineInput, TargetLag};
use sqlx::PgPool;
use uuid::Uuid;

/// A service credential for the orchestrator, seeded the way
/// `main::bootstrap_*` does: an identity with no scopes plus its token.
async fn seed_orchestrator(pool: &PgPool, token: &str) {
    let identity = lakehouse_store::identity::create_service_identity(
        pool,
        &lakehouse_store::identity::CreateServiceIdentityInput {
            name: format!("lag-check-{}", uuid::Uuid::new_v4().simple()),
            // The same scope production gives
            // `bootstrap_pipeline_run_service`'s identity: enough to clear
            // the route's `pipeline:read` policy floor.
            scopes: vec!["pipeline:read".to_owned()],
            environment: "test".to_owned(),
        },
    )
    .await
    .expect("seed the orchestrator service identity");
    let id: Uuid = identity.id.parse().expect("a UUID service-identity id");
    lakehouse_auth::service_token::ensure_service_credential(
        pool,
        id,
        &Secret::new(token.to_owned()),
    )
    .await
    .expect("seed the orchestrator credential");
}

async fn lag_check(app: &TestApp, id: &str, headers: &[(&str, String)]) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(format!("/api/pipelines/runnable/{id}/lag-check"));
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
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, body)
}

async fn seed_lagged_pipeline(pool: &PgPool, name: &str, minutes: i32) -> String {
    let created = pipelines::create_pipeline(
        pool,
        &CreatePipelineInput {
            name: name.to_owned(),
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
            tenant_id: None,
            depends_on: Vec::new(),
            connector_id: None,
            id: None,
            target_lag: Some(TargetLag::Lag { minutes }),
            // BLD-1 / D1: this test is about the lag check, not about who
            // the pipeline reads as.
            run_as: None,
        },
        None,
    )
    .await
    .expect("seed a pipeline with a target lag");
    pipelines::set_status(pool, &created.id, "ready")
        .await
        .expect("mark it ready")
        .expect("the row comes back");
    created.id
}

/// A user holding `pipeline:read` is authenticated but is NOT the
/// orchestrator: the policy row is only a floor, and the handler refuses.
#[tokio::test]
async fn a_pipeline_read_holder_is_refused_the_lag_check() {
    let app = spin_up().await;
    let user = create_principal_with_permissions(&app.pool, "pipeline:read").await;
    let cookie = session_cookie_for_user(&app.pool, user).await;
    let id = seed_lagged_pipeline(&app.pool, "lag-read-holder", 10).await;

    let (status, _) = lag_check(&app, &id, &[("cookie", cookie)]).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "only the orchestrator may ask whether a pipeline is behind"
    );
}

/// A pipeline in another tenant and a pipeline that has never existed must
/// be the same answer, for the one caller shape that is admitted but not
/// cross-tenant: a Platform Admin acting inside one tenant (`X-Tenant`).
/// Everyone else is refused earlier with an id-independent 403, so this
/// route cannot be used to learn whether an id exists at all.
#[tokio::test]
async fn another_tenants_pipeline_and_an_unknown_id_answer_the_same_404() {
    let app = spin_up().await;
    let tenant_a = provision_tenant(&app.pool, "lag-a").await;
    let tenant_b = provision_tenant(&app.pool, "lag-b").await;

    let theirs = pipelines::create_pipeline(
        &app.pool,
        &CreatePipelineInput {
            name: format!("lag-theirs-{}", uuid::Uuid::new_v4().simple()),
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
            tenant_id: Some(tenant_b),
            depends_on: Vec::new(),
            connector_id: None,
            id: None,
            target_lag: Some(TargetLag::Lag { minutes: 10 }),
            // BLD-1 / D1: this test is about tenant scoping, not about who
            // the pipeline reads as.
            run_as: None,
        },
        None,
    )
    .await
    .expect("seed another tenant's pipeline");

    // The admin must belong to the tenant it acts as: `X-Tenant` is a
    // selection among the caller's OWN tenants, never a grant.
    sqlx::query(
        "INSERT INTO app_user_tenant (user_id, tenant_id) \
         SELECT id, $1 FROM app_user WHERE email = 'fajar@meridian.example'",
    )
    .bind(tenant_a)
    .execute(&app.pool)
    .await
    .expect("attach the platform admin to the tenant");
    let admin = common::session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;
    let headers = vec![
        ("cookie", admin.clone()),
        ("x-tenant", tenant_a.to_string()),
    ];
    let (foreign_status, foreign_body) = lag_check(&app, &theirs.id, &headers).await;
    let (unknown_status, unknown_body) = lag_check(&app, "pl-never-existed", &headers).await;

    assert_eq!(foreign_status, StatusCode::NOT_FOUND);
    assert_eq!(
        foreign_status, unknown_status,
        "another tenant's pipeline and an unknown id must be indistinguishable"
    );
    // Both bodies name ONLY the id the caller sent (which it already
    // knows); neither mentions the other pipeline, so the difference
    // carries no information.
    assert_eq!(
        foreign_body["error"].as_str(),
        Some(format!("Pipeline {} not found", theirs.id).as_str())
    );
    assert_eq!(
        unknown_body["error"].as_str(),
        Some("Pipeline pl-never-existed not found")
    );
}

async fn provision_tenant(pool: &PgPool, prefix: &str) -> Uuid {
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

/// With the orchestrator's own credentials and every upstream dead, the
/// check answers `not_measured` — not `met`, not a run. This is the AC5
/// shape at the HTTP boundary.
#[tokio::test]
async fn the_orchestrator_gets_not_measured_when_the_input_cannot_be_read() {
    let app = spin_up().await;
    let token = "lag-check-orchestrator-token";
    seed_orchestrator(&app.pool, token).await;
    let id = seed_lagged_pipeline(&app.pool, "lag-not-measured", 5).await;

    let (status, body) =
        lag_check(&app, &id, &[("authorization", format!("Bearer {token}"))]).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["decision"], "wait",
        "nothing may be requested on a guess"
    );
    assert_eq!(body["status"], "not_measured");
    assert_eq!(body["lagSeconds"], Value::Null);
    assert_eq!(
        body["targetSeconds"], 300,
        "the target is reported in seconds"
    );
    assert_eq!(body["reason"], "an input could not be read");
    let job = body["jobName"].as_str().expect("a job name");
    assert!(
        job.starts_with("authored__") && !job.contains('-'),
        "the job name is the pipeline's Dagster-safe name, got {job:?}"
    );

    // And the state row says the same thing, so the page cannot disagree
    // with the answer the orchestrator was given.
    let stored = pipelines::get_lag_state(&app.pool, &id)
        .await
        .expect("read the lag state")
        .expect("the check wrote a row");
    assert_eq!(stored.last_status.as_deref(), Some("not_measured"));
    assert_eq!(stored.behind_streak, 0);
    assert_eq!(stored.requested_run_key, None);
}

/// A `Dagster`-native job name is not an authored pipeline: the same 404
/// as an unknown authored id, never a check against a job the database
/// knows nothing about.
#[tokio::test]
async fn a_dagster_native_job_name_is_not_checked() {
    let app = spin_up().await;
    let token = "lag-check-native-token";
    seed_orchestrator(&app.pool, token).await;

    let (status, body) = lag_check(
        &app,
        "ingest_job",
        &[("authorization", format!("Bearer {token}"))],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "Pipeline ingest_job not found");
}
