//! HTTP-level tests for the organisation a digital employee works in
//! (`SEC-76`): a run of an agent with no organisation is refused before any
//! model call, a manual run only reaches an agent inside the caller's own
//! organisation, and `PUT /api/agents/employees/{id}/tenant` sets the
//! organisation with its membership check and its audit row.
//!
//! The LLM is mocked with `wiremock`, as in `agents_run.rs`. A test that must
//! prove no model call happened mounts the completion route with
//! `.expect(0)` and verifies it, so a call that slips through fails the test.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{
    TestApp, audit_rows, create_principal_with_permissions, session_cookie_for_seeded_user,
    session_cookie_for_user, spin_up, spin_up_with_env,
};

/// The seeded organisations (`0002_seed_identity.sql`). `sari` belongs to
/// Retail only, `fajar` to every organisation and is unrestricted (`*:*`).
const RETAIL: &str = "11111111-1111-4111-8111-000000000002";
const LOGISTICS: &str = "11111111-1111-4111-8111-000000000003";

/// The fixed sentence a run of an agent with no organisation gets
/// (`routes::agents::NO_ORGANISATION_SENTENCE`).
const NO_ORGANISATION: &str = "this agent has no organisation; set one before it runs";

async fn spin_up_with_llm(llm_url: &str, agent_run_token: Option<&str>) -> TestApp {
    let mut overrides = HashMap::new();
    overrides.insert("LLM_URL".to_owned(), llm_url.to_owned());
    if let Some(tok) = agent_run_token {
        overrides.insert("AGENT_RUN_TOKEN".to_owned(), tok.to_owned());
    }
    spin_up_with_env(&overrides).await
}

/// Insert a runnable employee with the given organisation (or none). Test-only
/// fixture helper, as in `agents_run.rs`.
async fn insert_employee(pool: &PgPool, id: &str, tenant: Option<&str>) {
    sqlx::query(
        "INSERT INTO agent_employee (id, name, purpose, autonomy, status, prompt, mode, \
         permissions, tenant_id) VALUES ($1, $2, 'test purpose', 'L1', 'ready', \
         'Cek kunjungan harian.', 'ask', '', $3::uuid)",
    )
    .bind(id)
    .bind(format!("tenant-test-{id}"))
    .bind(tenant)
    .execute(pool)
    .await
    .expect("insert a test employee");
}

/// A completion that answers with no tool call. Mounted with `.expect(0)` by
/// the tests that must never reach the model.
async fn mount_completion(mock: &MockServer, times: Option<u64>) {
    let mock_builder = Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "role": "assistant", "content": "Selesai." } }]
        })));
    match times {
        Some(n) => mock_builder.expect(n).mount(mock).await,
        None => mock_builder.mount(mock).await,
    }
}

async fn run_employee(
    app: &axum::Router,
    id: &str,
    cookie: &str,
    token: Option<&str>,
) -> axum::http::Response<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(format!("/api/agents/employees/{id}/run"))
        .header("content-type", "application/json")
        .header("cookie", cookie);
    if let Some(token) = token {
        builder = builder.header("x-run-token", token);
    }
    app.clone()
        .oneshot(builder.body(Body::from("{}")).expect("build request"))
        .await
        .expect("router never fails a request outright")
}

async fn put_tenant(
    app: &axum::Router,
    id: &str,
    cookie: &str,
    tenant_id: &str,
) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/agents/employees/{id}/tenant"))
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(json!({ "tenantId": tenant_id }).to_string()))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright")
}

async fn json_body(resp: axum::http::Response<Body>) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read response body");
    serde_json::from_slice(&bytes).expect("valid JSON body")
}

/// How many `agent_run` rows exist for an employee: a refused run must leave
/// none behind.
async fn run_count(pool: &PgPool, employee: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM agent_run WHERE employee_id = $1")
        .bind(employee)
        .fetch_one(pool)
        .await
        .expect("count runs")
}

async fn tenant_of(pool: &PgPool, employee: &str) -> Option<Uuid> {
    sqlx::query_scalar("SELECT tenant_id FROM agent_employee WHERE id = $1")
        .bind(employee)
        .fetch_one(pool)
        .await
        .expect("read the agent's organisation")
}

/// A session for a new person with `permissions`, who belongs to each of
/// `tenants` and to no other organisation. Restricted unless the permissions
/// say otherwise (`*:*` is never granted here).
async fn principal_in(pool: &PgPool, permissions: &str, tenants: &[&str]) -> String {
    let user = create_principal_with_permissions(pool, permissions).await;
    for tenant in tenants {
        sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2::uuid)")
            .bind(user)
            .bind(tenant)
            .execute(pool)
            .await
            .expect("put the person in an organisation");
    }
    session_cookie_for_user(pool, user).await
}

/// A person with `agent:manage` who belongs to one organisation and is not
/// unrestricted: the caller the organisation checks are about.
async fn manager_in(pool: &PgPool, tenant: &str) -> String {
    principal_in(pool, "agent:manage, query:read", &[tenant]).await
}

/// `agent:manage` plus `identity:write`: the administrator who may move an
/// agent between organisations (SEC-76 review, BLOCKER 1).
async fn administrator_in(pool: &PgPool, tenants: &[&str]) -> String {
    principal_in(pool, "agent:manage, identity:write, query:read", tenants).await
}

/// A scheduled run of an agent with no organisation is refused with 409 and
/// the fixed sentence before any model call, and it leaves no run row.
#[tokio::test]
async fn a_scheduled_run_of_an_agent_with_no_organisation_is_refused_before_any_model_call() {
    let llm = MockServer::start().await;
    mount_completion(&llm, Some(0)).await;
    let app = spin_up_with_llm(&llm.uri(), Some("secret-run-token")).await;
    insert_employee(&app.pool, "emp-no-org-sched", None).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "sari@meridian.example").await;

    let resp = run_employee(
        &app.router,
        "emp-no-org-sched",
        &cookie,
        Some("secret-run-token"),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let body = json_body(resp).await;
    assert_eq!(body["error"].as_str(), Some(NO_ORGANISATION));
    assert_eq!(run_count(&app.pool, "emp-no-org-sched").await, 0);
    llm.verify().await;
}

/// A manual run of the same agent is refused the same way: no organisation
/// means no run for anyone, the unrestricted caller included.
#[tokio::test]
async fn a_manual_run_of_an_agent_with_no_organisation_is_refused_for_an_unrestricted_caller() {
    let llm = MockServer::start().await;
    mount_completion(&llm, Some(0)).await;
    let app = spin_up_with_llm(&llm.uri(), None).await;
    insert_employee(&app.pool, "emp-no-org-manual", None).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;

    let resp = run_employee(&app.router, "emp-no-org-manual", &cookie, None).await;

    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let body = json_body(resp).await;
    assert_eq!(body["error"].as_str(), Some(NO_ORGANISATION));
    assert_eq!(run_count(&app.pool, "emp-no-org-manual").await, 0);
    llm.verify().await;
}

/// A person who belongs to another organisation, and is not unrestricted,
/// gets 404 for a manual run of an agent in Retail, and no run starts. The
/// answer is the one an unknown employee gets.
#[tokio::test]
async fn a_manual_run_by_a_person_outside_the_agents_organisation_is_not_found() {
    let llm = MockServer::start().await;
    mount_completion(&llm, Some(0)).await;
    let app = spin_up_with_llm(&llm.uri(), None).await;
    insert_employee(&app.pool, "emp-other-org", Some(RETAIL)).await;
    let cookie = manager_in(&app.pool, LOGISTICS).await;

    let resp = run_employee(&app.router, "emp-other-org", &cookie, None).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(run_count(&app.pool, "emp-other-org").await, 0);
    llm.verify().await;
}

/// An unrestricted person may run an agent in any organisation, and the run
/// completes in the agent's organisation.
#[tokio::test]
async fn an_unrestricted_person_runs_an_agent_in_its_organisation() {
    let llm = MockServer::start().await;
    mount_completion(&llm, None).await;
    let app = spin_up_with_llm(&llm.uri(), None).await;
    insert_employee(&app.pool, "emp-unrestricted", Some(RETAIL)).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;

    let resp = run_employee(&app.router, "emp-unrestricted", &cookie, None).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["run"]["status"].as_str(), Some("succeeded"));
}

/// Setting an organisation writes `agent.tenant.set` with the old and the new
/// organisation ids, and the response shows the new one as `tenantId`.
#[tokio::test]
async fn setting_the_organisation_audits_the_old_and_the_new_ids() {
    let app = spin_up().await;
    insert_employee(&app.pool, "emp-set-org", None).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;

    let resp = put_tenant(&app.router, "emp-set-org", &cookie, RETAIL).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["tenantId"].as_str(), Some(RETAIL));

    let resp = put_tenant(&app.router, "emp-set-org", &cookie, LOGISTICS).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        tenant_of(&app.pool, "emp-set-org")
            .await
            .map(|t| t.to_string()),
        Some(LOGISTICS.to_owned())
    );

    let rows = audit_rows(&app.pool, "agent.tenant.set").await;
    assert_eq!(rows.len(), 2, "each change is one audit row");
    let first = &rows[0];
    assert_eq!(first["resource_kind"].as_str(), Some("digital_employee"));
    assert_eq!(first["resource_id"].as_str(), Some("emp-set-org"));
    assert_eq!(first["principal_kind"].as_str(), Some("user"));
    assert_eq!(first["outcome"].as_str(), Some("executed"));
    assert!(first["args"]["old_tenant_id"].is_null());
    assert_eq!(first["args"]["new_tenant_id"].as_str(), Some(RETAIL));
    let second = &rows[1];
    assert_eq!(second["args"]["old_tenant_id"].as_str(), Some(RETAIL));
    assert_eq!(second["args"]["new_tenant_id"].as_str(), Some(LOGISTICS));
}

/// A caller outside the target organisation gets 404, and the agent keeps its
/// organisation and no audit row is written for the attempt.
#[tokio::test]
async fn a_caller_outside_the_target_organisation_is_not_found_and_changes_nothing() {
    let app = spin_up().await;
    insert_employee(&app.pool, "emp-outside-target", None).await;
    let cookie = administrator_in(&app.pool, &[LOGISTICS]).await;

    let resp = put_tenant(&app.router, "emp-outside-target", &cookie, RETAIL).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(tenant_of(&app.pool, "emp-outside-target").await, None);
    let rows = audit_rows(&app.pool, "agent.tenant.set").await;
    assert!(
        rows.is_empty(),
        "a refused change writes no audit row: {rows:?}"
    );
}

/// An organisation that does not exist is 404 for an unrestricted caller too,
/// with nothing changed.
#[tokio::test]
async fn an_unknown_organisation_is_not_found() {
    let app = spin_up().await;
    insert_employee(&app.pool, "emp-unknown-org", None).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;
    let unknown = Uuid::new_v4().to_string();

    let resp = put_tenant(&app.router, "emp-unknown-org", &cookie, &unknown).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(tenant_of(&app.pool, "emp-unknown-org").await, None);
    let rows = audit_rows(&app.pool, "agent.tenant.set").await;
    assert!(
        rows.is_empty(),
        "a refused change writes no audit row: {rows:?}"
    );
}

/// A `sql`-adapter connector of `tenant`, seeded the way
/// `connector_ingestible_tenant.rs` seeds one: a fixture row, never a real
/// connection.
async fn seed_connector(pool: &PgPool, id: &str, tenant: &str) {
    sqlx::query(
        "INSERT INTO connector (id, tenant_id, name, type, direction, host, secret_ref, \
         environment, tenant, adapter, ingest_mode, dial) VALUES \
         ($1, $2::uuid, $3, 'PostgreSQL', 'source', 'unused', 'env:CONNECTOR_PG_PASSWORD', \
         'production', 'meridian', 'sql', 'batch', \
         '{\"driver\":\"postgres\",\"host\":\"127.0.0.1\",\"port\":5432,\"database\":\"d\",\"user\":\"u\"}'::jsonb)",
    )
    .bind(id)
    .bind(tenant)
    .bind(format!("sec76 test {id}"))
    .execute(pool)
    .await
    .expect("seed a connector");
}

const LOGISTICS_CONNECTOR: &str = "conn-sec76-logistics";
const RETAIL_CONNECTOR: &str = "conn-sec76-retail";

/// B1 (SEC-76, plan step 5): a scheduled run acts in the agent's organisation.
///
/// Two organisations hold one connector each. The agent works in Logistics,
/// with a read grant, and runs on a token. Its one tenant-scoped tool step,
/// `list_connectors`, reads the run principal's organisation through
/// `tenant_scope::resolve`. The step must list Logistics' connector and not
/// Retail's. A run that had no organisation would list nothing, and the old
/// nil principal had none either, so the Logistics connector is the proof.
#[tokio::test]
async fn a_scheduled_run_lists_only_the_agents_organisation() {
    let llm = MockServer::start().await;
    // The first completion asks for `list_connectors`; every later one
    // answers, which ends the run.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": { "name": "list_connectors", "arguments": "{}" },
                    }],
                }
            }]
        })))
        .up_to_n_times(1)
        .mount(&llm)
        .await;
    mount_completion(&llm, None).await;
    let app = spin_up_with_llm(&llm.uri(), Some("secret-run-token")).await;
    sqlx::query(
        "INSERT INTO agent_employee (id, name, purpose, autonomy, status, prompt, mode, \
         permissions, tenant_id, read_roles) VALUES ('emp-b1-logistics', 'b1 logistics agent', \
         'test purpose', 'L1', 'ready', 'Cek konektor.', 'ask', 'connector:manage', $1::uuid, \
         ARRAY['Analyst']::text[])",
    )
    .bind(LOGISTICS)
    .execute(&app.pool)
    .await
    .expect("insert the agent in Logistics");
    seed_connector(&app.pool, LOGISTICS_CONNECTOR, LOGISTICS).await;
    seed_connector(&app.pool, RETAIL_CONNECTOR, RETAIL).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "sari@meridian.example").await;

    let resp = run_employee(
        &app.router,
        "emp-b1-logistics",
        &cookie,
        Some("secret-run-token"),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    let steps = body["run"]["steps"].as_array().expect("the run has steps");
    let step = steps
        .iter()
        .find(|step| step["label"].as_str() == Some("list_connectors"))
        .expect("the run called list_connectors");
    assert_eq!(step["status"].as_str(), Some("succeeded"), "{step}");
    let listed: Value = serde_json::from_str(step["detail"].as_str().expect("a detail string"))
        .expect("the detail is the tool's JSON");
    let ids: Vec<&str> = listed
        .as_array()
        .expect("the tool lists connectors as an array")
        .iter()
        .filter_map(|connector| connector["id"].as_str())
        .collect();
    assert!(
        ids.contains(&LOGISTICS_CONNECTOR),
        "the agent's organisation connector is listed: {ids:?}"
    );
    assert!(
        !ids.contains(&RETAIL_CONNECTOR),
        "another organisation's connector is not listed: {ids:?}"
    );
}

/// The fixed sentence a caller without `identity:write` (and not unrestricted)
/// gets when it tries to change an agent's organisation
/// (`routes::agents::ORGANISATION_NEEDS_ADMIN_SENTENCE`).
const NEEDS_ADMIN: &str = "changing an agent's organisation needs an administrator";

/// SEC-76 review, BLOCKER 1: `agent:manage` alone is not enough to move an
/// agent between organisations. A person who holds `agent:manage` in both
/// organisations, but not `identity:write`, gets 403 with the fixed sentence,
/// and the agent keeps its organisation with no audit row. The permission check
/// answers first, so the person's memberships do not matter here.
#[tokio::test]
async fn an_agent_manager_without_identity_write_is_refused_and_nothing_changes() {
    let app = spin_up().await;
    insert_employee(&app.pool, "emp-manager-refused", Some(RETAIL)).await;
    let cookie = principal_in(&app.pool, "agent:manage, query:read", &[RETAIL, LOGISTICS]).await;

    let resp = put_tenant(&app.router, "emp-manager-refused", &cookie, LOGISTICS).await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body = json_body(resp).await;
    // `ApiError::PermissionDenied` prefixes its code, as every 403 does.
    let error = body["error"].as_str().unwrap_or_default();
    assert!(error.ends_with(NEEDS_ADMIN), "{body}");
    assert_eq!(
        tenant_of(&app.pool, "emp-manager-refused")
            .await
            .map(|t| t.to_string()),
        Some(RETAIL.to_owned())
    );
    let rows = audit_rows(&app.pool, "agent.tenant.set").await;
    assert!(
        rows.is_empty(),
        "a refused change writes no audit row: {rows:?}"
    );
}

/// SEC-76 review, BLOCKER 1: once an agent has an organisation, an administrator
/// who is not unrestricted must belong to that CURRENT organisation too. This
/// one holds `identity:write` in the target (Logistics) only, so the agent in
/// Retail is not theirs to move: 404, same as `tenant_scope`, nothing changed.
#[tokio::test]
async fn an_administrator_outside_the_agents_current_organisation_is_not_found() {
    let app = spin_up().await;
    insert_employee(&app.pool, "emp-current-outside", Some(RETAIL)).await;
    let cookie = administrator_in(&app.pool, &[LOGISTICS]).await;

    let resp = put_tenant(&app.router, "emp-current-outside", &cookie, LOGISTICS).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        tenant_of(&app.pool, "emp-current-outside")
            .await
            .map(|t| t.to_string()),
        Some(RETAIL.to_owned())
    );
    let rows = audit_rows(&app.pool, "agent.tenant.set").await;
    assert!(
        rows.is_empty(),
        "a refused change writes no audit row: {rows:?}"
    );
}

/// SEC-76 review, BLOCKER 1: an administrator who belongs to both the agent's
/// current organisation and the target moves it, and the change is audited with
/// the old and the new organisation ids.
#[tokio::test]
async fn an_administrator_in_both_organisations_moves_the_agent_and_it_is_audited() {
    let app = spin_up().await;
    insert_employee(&app.pool, "emp-both-orgs", Some(RETAIL)).await;
    let cookie = administrator_in(&app.pool, &[RETAIL, LOGISTICS]).await;

    let resp = put_tenant(&app.router, "emp-both-orgs", &cookie, LOGISTICS).await;

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        tenant_of(&app.pool, "emp-both-orgs")
            .await
            .map(|t| t.to_string()),
        Some(LOGISTICS.to_owned())
    );
    let rows = audit_rows(&app.pool, "agent.tenant.set").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["args"]["old_tenant_id"].as_str(), Some(RETAIL));
    assert_eq!(rows[0]["args"]["new_tenant_id"].as_str(), Some(LOGISTICS));
    assert_eq!(rows[0]["principal_kind"].as_str(), Some("user"));
}
