//! `POST /api/pipelines/runnable/{id}/enforce` (BLD-1 / D3): the rewrite a
//! pipeline's AUTHOR is allowed to run, which is the only SQL the
//! orchestrator runs (`dagster/dispar_orchestrate/authored_factory.py`).
//!
//! Driven through the real router against a real per-test Postgres (users,
//! roles, policies) and a `wiremock` stand-in for `ClickHouse`, the same
//! harness `explain_rewrite.rs` uses for the masking rewrite — the point of
//! these tests is that this route reaches the SAME engine those routes do,
//! so the harness has to be the same one that proves they do.
//!
//! The cases are the ones that decide whether the feature is safe: a
//! masked column comes back rewritten for an author who has the
//! obligation, comes back untouched for one who does not, a pipeline with
//! no usable author cannot run at all, a refused construct is refused with
//! a fixed message, a non-orchestrator caller cannot reach it, and an id
//! that is not this caller's pipeline answers the same 404 as an id that
//! does not exist at all.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::governance::{self, CreatePolicyInput};
use lakehouse_store::pipelines::{self, CreatePipelineInput};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

/// The table every test here reads. A `serving.` table, because that is
/// what an authored pipeline actually reads, and a name no other test in
/// this binary shares.
const SQL: &str = "SELECT email FROM serving.enforced_x";

async fn mount(ch: &MockServer) {
    // The column metadata the mask needs: the obligations source reads a
    // governed table's columns to decide what a mask is substituted OVER,
    // and refuses the substitution (`UnprovableSubstitution`) rather than
    // guessing when it cannot. Same mock `explain_rewrite.rs` mounts, for
    // the same reason.
    Mock::given(method("POST"))
        .and(body_string_contains("system.columns"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [
                {"name": "name", "type": "String"}, {"name": "type", "type": "String"},
                {"name": "default_kind", "type": "String"}, {"name": "default_expression", "type": "String"}
            ],
            "data": [
                {"name": "email", "type": "String", "default_kind": "", "default_expression": ""},
            ],
            "rows": 1,
        })))
        .mount(ch)
        .await;
    // The one other ClickHouse call the rewrite makes: the view catalog
    // lookup (`PolicyEngineObligations::prefetch_for_tables` ->
    // `system.tables`). No rows: none of these tables is a view, and an
    // empty answer keeps the tests about the MASK rather than the lookup.
    Mock::given(method("POST"))
        .and(body_string_contains("system.tables"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "data": [], "rows": 0,
        })))
        .mount(ch)
        .await;
}

async fn app() -> (axum::Router, PgPool, String) {
    let ch = MockServer::start().await;
    mount(&ch).await;
    let mut overrides = std::collections::HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch.uri());
    let TestApp { router, pool } = spin_up_with_env(&overrides).await;
    // A Platform Admin: `runnable`'s rule allows a service identity OR an
    // unrestricted principal, and a test cannot mint the orchestrator's
    // service credential through the router. The Admin is the other branch
    // of the same rule, so the allowed-path tests still prove the route is
    // reachable and that `route_auth.rs` proves the permission floor.
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    (router, pool, cookie)
}

async fn seed_mask(pool: &PgPool, role: &str) {
    governance::create_policy(
        pool,
        &CreatePolicyInput {
            name: format!("pipeline-enforce-{role}"),
            kind: "Row filter".to_owned(),
            subjects: role.to_owned(),
            resources: "serving.enforced_x".to_owned(),
            effect: "Permit with obligation".to_owned(),
            conditions: Some(
                json!({"roles": [role], "table": "serving.enforced_x", "mask": ["email"]})
                    .to_string(),
            ),
            activate: true,
            owner: None,
        },
    )
    .await
    .unwrap();
}

/// A pipeline authored by `run_as`, which may be `None` (the "created by a
/// service identity" case), in `tenant_id` (also possibly `None`).
async fn seed_pipeline(pool: &PgPool, run_as: Option<Uuid>, tenant_id: Option<Uuid>) -> String {
    let created = pipelines::create_pipeline(
        pool,
        &CreatePipelineInput {
            name: "enforce-probe".to_owned(),
            kind: "batch".to_owned(),
            source_zone: "bronze".to_owned(),
            source_table: "orders".to_owned(),
            target_zone: "silver".to_owned(),
            target_table: "orders_clean".to_owned(),
            schedule: "0 3 * * *".to_owned(),
            description: None,
            incremental_column: None,
            transforms: Vec::new(),
            fbic_enabled: false,
            max_retries: None,
            owner: None,
            tenant_id,
            depends_on: Vec::new(),
            connector_id: None,
            id: None,
            run_as,
        },
        run_as,
    )
    .await
    .unwrap();
    created.id
}

/// The tenant a caller resolves to when they send no `X-Tenant`: their
/// first one (`tenant_scope::resolve`). `GET /api/pipelines/{id}` scopes
/// against it, so a pipeline these tests READ back has to live in it --
/// a tenant-NULL row is correctly invisible to a restricted caller.
async fn first_tenant_of(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar(
        "SELECT tenant_id FROM app_user_tenant \
         WHERE user_id = (SELECT id FROM app_user WHERE email = $1) \
         ORDER BY tenant_id LIMIT 1",
    )
    .bind(email)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn enforce(router: &axum::Router, cookie: &str, id: &str, sql: &str) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/pipelines/runnable/{id}/enforce"))
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(json!({ "sql": sql }).to_string()))
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

async fn detail(
    router: &axum::Router,
    cookie: &str,
    id: &str,
    tenant: Option<Uuid>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method("GET")
        .uri(format!("/api/pipelines/{id}"))
        .header("cookie", cookie);
    // The detail read is tenant-scoped, and with no `X-Tenant` the scope
    // is `principal.tenant_ids.first()` -- which the auth query returns in
    // whatever order the database feels like. Naming the tenant makes the
    // test say what it means instead of depending on that.
    if let Some(tenant) = tenant {
        request = request.header("x-tenant", tenant.to_string());
    }
    let resp = router
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// The console's source for "Reads as: <name>" (BLD-1 / D2's console half).
///
/// It cannot get the name from `GET /api/identity/users` — that route is
/// `identity:read`, which no ordinary role holds, and the people who
/// operate pipelines are Data Engineers. So the detail read carries it,
/// and this pins both halves: a named author arrives as `runAsName`, and a
/// pipeline with no author carries `runAsName: null` rather than an
/// invented one.
#[tokio::test]
async fn the_pipeline_detail_carries_the_author_name_for_the_console() {
    let (router, pool, cookie) = app().await;
    let tenant = first_tenant_of(&pool, "fajar@meridian.example").await;
    let id = seed_pipeline(
        &pool,
        Some(seeded_user_id(&pool, "rina@meridian.example").await),
        Some(tenant),
    )
    .await;

    let (status, body) = detail(&router, &cookie, &id, Some(tenant)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["runAsName"].as_str(), Some("Rina Wijaya"), "{body}");
    assert!(
        body["runAs"].is_string(),
        "the author id must travel too: {body}"
    );
}

#[tokio::test]
async fn the_pipeline_detail_carries_no_author_name_when_there_is_no_author() {
    let (router, pool, cookie) = app().await;
    let tenant = first_tenant_of(&pool, "fajar@meridian.example").await;
    let id = seed_pipeline(&pool, None, Some(tenant)).await;

    let (status, body) = detail(&router, &cookie, &id, Some(tenant)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["runAsName"], Value::Null, "{body}");
    assert!(
        body.get("runAs").is_none(),
        "an absent author must be an absent key, not a null one: {body}"
    );
}

async fn seeded_user_id(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM app_user WHERE email = $1")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The headline of the whole feature: a pipeline authored by a user the
/// mask governs gets the column rewritten, so the orchestrator cannot copy
/// a masked value into a Silver table in clear (`GOV-4`).
#[tokio::test]
async fn a_masked_column_is_rewritten_for_an_author_who_has_the_obligation() {
    let (router, pool, cookie) = app().await;
    seed_mask(&pool, "Analyst").await;
    let id = seed_pipeline(
        &pool,
        Some(seeded_user_id(&pool, "rina@meridian.example").await),
        None,
    )
    .await;

    let (status, body) = enforce(&router, &cookie, &id, SQL).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let sql = body["sql"].as_str().expect("a sql string");
    assert!(
        sql.contains("replaceRegexpOne(toString(`email`)"),
        "the mask was not applied: {sql}"
    );
    // The rewrite wraps the governed table in a masked subquery and
    // projects the ORIGINAL column name from it, so the text that must be
    // gone is the original statement, not the bare word `SELECT email`.
    assert!(
        !sql.contains("SELECT email FROM serving.enforced_x"),
        "the unmasked statement survived the rewrite: {sql}"
    );
}

/// The other half, and the one that would make the feature useless if it
/// were wrong: an author the policy does not govern reads its own rows.
#[tokio::test]
async fn an_author_without_the_obligation_gets_the_query_unchanged() {
    let (router, pool, cookie) = app().await;
    // Masked for the Analyst; the author below is a Data Engineer, which
    // the policy does not name.
    seed_mask(&pool, "Analyst").await;
    let id = seed_pipeline(
        &pool,
        Some(seeded_user_id(&pool, "yoga@meridian.example").await),
        None,
    )
    .await;

    let (status, body) = enforce(&router, &cookie, &id, SQL).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["sql"].as_str(), Some(SQL), "{body}");
}

/// D2, first case: no author at all. The op must not be told to run the
/// query unrewritten, and must be told why in a fixed message.
#[tokio::test]
async fn a_pipeline_with_no_author_is_refused_with_d2s_message() {
    let (router, pool, cookie) = app().await;
    seed_mask(&pool, "Analyst").await;
    let id = seed_pipeline(&pool, None, None).await;

    let (status, body) = enforce(&router, &cookie, &id, SQL).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        body["error"].as_str(),
        Some("this pipeline has no author to read as; an administrator must assign one"),
        "{body}"
    );
}

/// D2, second case: the author row exists but is deactivated, so loading
/// its roles would enforce the query as if the user were still working.
#[tokio::test]
async fn a_pipeline_authored_by_a_deactivated_user_is_refused_with_d2s_message() {
    let (router, pool, cookie) = app().await;
    seed_mask(&pool, "Analyst").await;
    let author = seeded_user_id(&pool, "rina@meridian.example").await;
    let id = seed_pipeline(&pool, Some(author), None).await;
    sqlx::query("UPDATE app_user SET status = 'inactive' WHERE id = $1")
        .bind(author)
        .execute(&pool)
        .await
        .unwrap();

    let (status, body) = enforce(&router, &cookie, &id, SQL).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        body["error"].as_str(),
        Some("this pipeline has no author to read as; an administrator must assign one"),
        "{body}"
    );
}

/// A user deleted outright leaves `run_as` NULL (`ON DELETE SET NULL`),
/// which is the first case again — reached without anyone deciding it.
#[tokio::test]
async fn a_pipeline_whose_author_was_deleted_is_refused_with_d2s_message() {
    let (router, pool, cookie) = app().await;
    seed_mask(&pool, "Analyst").await;
    let author = seeded_user_id(&pool, "rina@meridian.example").await;
    let id = seed_pipeline(&pool, Some(author), None).await;
    sqlx::query("DELETE FROM app_user WHERE id = $1")
        .bind(author)
        .execute(&pool)
        .await
        .unwrap();

    let (status, body) = enforce(&router, &cookie, &id, SQL).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        body["error"].as_str(),
        Some("this pipeline has no author to read as; an administrator must assign one"),
        "{body}"
    );
}

/// A construct the rewriter will not accept is refused with the engine's
/// own FIXED message, and never with the parser's or `ClickHouse`'s text
/// (`AGENTS.md` rule 4).
#[tokio::test]
async fn a_refused_construct_is_refused_with_the_fixed_message_only() {
    let (router, pool, cookie) = app().await;
    seed_mask(&pool, "Analyst").await;
    let id = seed_pipeline(
        &pool,
        Some(seeded_user_id(&pool, "rina@meridian.example").await),
        None,
    )
    .await;

    let (status, body) = enforce(&router, &cookie, &id, "SELECT * FROM numbers(5)").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let message = body["error"].as_str().expect("an error string");
    assert!(!message.is_empty(), "{body}");
    assert!(
        !message.contains("numbers"),
        "the refusal echoed the statement it refused: {message}"
    );
}

/// The orchestrator-only rule: a seeded Data Engineer holds `pipeline:*`
/// (so they pass the policy floor) and is still refused, because this
/// route answers for every tenant's pipelines.
#[tokio::test]
async fn a_caller_who_is_neither_service_nor_admin_is_refused() {
    let (router, pool, _cookie) = app().await;
    seed_mask(&pool, "Analyst").await;
    let id = seed_pipeline(
        &pool,
        Some(seeded_user_id(&pool, "rina@meridian.example").await),
        None,
    )
    .await;
    let data_engineer = session_cookie_for_seeded_user(&pool, "bayu@meridian.example").await;

    let (status, body) = enforce(&router, &data_engineer, &id, SQL).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

/// An id that is not a pipeline answers 404 — and so does a `Dagster`
/// job id, which has no `pipeline_definition` row and therefore no
/// `run_as`: the same answer either way, so this route cannot be used to
/// tell a real pipeline from a job, or a live tenant from another one.
#[tokio::test]
async fn an_unknown_pipeline_and_a_dagster_job_id_answer_the_same_404() {
    let (router, pool, cookie) = app().await;
    seed_mask(&pool, "Analyst").await;
    let id = seed_pipeline(
        &pool,
        Some(seeded_user_id(&pool, "rina@meridian.example").await),
        None,
    )
    .await;

    let (unknown_status, unknown_body) = enforce(&router, &cookie, "pl-does-not-exist", SQL).await;
    let (job_status, job_body) = enforce(&router, &cookie, "ingest_job", SQL).await;

    assert_eq!(unknown_status, StatusCode::NOT_FOUND, "{unknown_body}");
    assert_eq!(job_status, StatusCode::NOT_FOUND, "{job_body}");
    // The same SHAPE, differing only in the id each request asked for --
    // which is the no-leak property that matters here: nothing in either
    // answer says whether a live pipeline with that id exists, so this
    // route cannot be used to enumerate another tenant's pipelines.
    assert_eq!(
        unknown_body["error"]
            .as_str()
            .map(|m| m.replace("pl-does-not-exist", "<id>")),
        job_body["error"]
            .as_str()
            .map(|m| m.replace("ingest_job", "<id>")),
        "the two 404s differ in a way that leaks existence"
    );
    assert_ne!(unknown_body["error"].as_str(), Some(id.as_str()));
}

/// The author's roles are read live, not copied at create: a user who is
/// given a governed role AFTER authoring the pipeline starts being masked
/// on its next run.
#[tokio::test]
async fn the_authors_roles_are_read_at_enforcement_time_not_at_create() {
    let (router, pool, cookie) = app().await;
    seed_mask(&pool, "Analyst").await;
    let author = seeded_user_id(&pool, "yoga@meridian.example").await;
    let id = seed_pipeline(&pool, Some(author), None).await;

    let (status, body) = enforce(&router, &cookie, &id, SQL).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["sql"].as_str(), Some(SQL), "not governed yet: {body}");

    sqlx::query(
        "INSERT INTO app_user_role (user_id, role_id) \
         SELECT $1, id FROM role WHERE name = 'Analyst'",
    )
    .bind(author)
    .execute(&pool)
    .await
    .unwrap();

    let (status, body) = enforce(&router, &cookie, &id, SQL).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let sql = body["sql"].as_str().expect("a sql string");
    assert!(
        sql.contains("replaceRegexpOne(toString(`email`)"),
        "the newly granted role did not take effect: {sql}"
    );
}

/// Seeds the orchestrator's service identity and credential the way
/// `main::bootstrap_gold_export_service` and
/// `security_regressions.rs::seed_agent_run_service_credential` do — the
/// same production functions, because those bootstraps are private to the
/// `bin` crate and this HTTP-level harness cannot call them. The scope is
/// the `pipeline:read` floor `POLICY_TABLE` puts on this route.
async fn seed_orchestrator_credential(pool: &PgPool, token: &str) {
    let identity = lakehouse_store::identity::create_service_identity(
        pool,
        &lakehouse_store::identity::CreateServiceIdentityInput {
            name: "pipeline-enforce-orchestrator".to_owned(),
            scopes: vec!["pipeline:read".to_owned()],
            environment: "test".to_owned(),
        },
    )
    .await
    .unwrap();
    let service_identity_id: Uuid = identity.id.parse().unwrap();
    lakehouse_auth::service_token::ensure_service_credential(
        pool,
        service_identity_id,
        &lakehouse_auth::Secret::new(token.to_owned()),
    )
    .await
    .unwrap();
}

/// BLD-1 review BLOCKER 1: a failure loading the author's principal is an
/// UPSTREAM error, and its text must never reach a response body
/// (`AGENTS.md` principle 4). The body carries only the fixed
/// `upstream_error` message and a reference id; the driver's own text goes
/// to the log under that reference.
#[tokio::test]
async fn an_upstream_database_failure_loading_the_author_carries_no_upstream_text() {
    const TOKEN: &str = "orch-pipeline-enforce-test-token";
    let (router, pool, _cookie) = app().await;
    let id = seed_pipeline(
        &pool,
        Some(seeded_user_id(&pool, "rina@meridian.example").await),
        None,
    )
    .await;
    // The caller is the orchestrator's service identity, so authenticating
    // reads `service_credential` and `service_identity` and never
    // `app_user_tenant` -- the table about to go away.
    seed_orchestrator_credential(&pool, TOKEN).await;

    // Take away the table `load_principal_for_user` reads the author's
    // tenants and roles through, so the loader fails with the database's
    // own "relation ... does not exist" text. `user_status` above it still
    // reads `app_user`, so the request reaches the loader rather than
    // failing earlier.
    sqlx::query("DROP TABLE app_user_tenant CASCADE")
        .execute(&pool)
        .await
        .unwrap();

    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/pipelines/runnable/{id}/enforce"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::from(json!({ "sql": SQL }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    let message = body["error"].as_str().expect("an error string");
    // Asserted against the helper's own context rather than a copied
    // literal: the body has to be exactly what `upstream_error` renders,
    // so a route that answered something else fails here instead of
    // quietly passing on an unrelated fixed wording.
    let prefix = format!(
        "{} Reference: ",
        lakehouse_api::upstream_error::STORE.failed
    );
    let Some(reference) = message.strip_prefix(&prefix) else {
        panic!("the fixed message and a reference, got: {message}");
    };
    assert_eq!(
        reference.len(),
        10,
        "a ten-character reference an operator can look up: {message}"
    );
    for upstream in ["relation", "app_user_tenant", "does not exist", "ERROR"] {
        assert!(
            !message.contains(upstream),
            "the upstream text {upstream:?} reached the response: {message}"
        );
    }
}
