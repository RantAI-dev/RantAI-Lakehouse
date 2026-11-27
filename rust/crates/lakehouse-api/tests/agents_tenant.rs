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
    CATALOG_TENANT, TestApp, audit_rows, create_principal_with_permissions,
    session_cookie_for_seeded_user, session_cookie_for_user, spin_up, spin_up_with_env,
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

// ── SEC-76 follow-up (adm-147): the agents a person may see, and the replay ──

/// The bearer and `x-run-token` the scheduler presents (`AGENT_RUN_TOKEN`).
const RUN_TOKEN: &str = "secret-run-token";

/// Agents the visibility tests insert, one per organisation shape.
const RETAIL_AGENT: &str = "emp-s76-retail";
const LOGISTICS_AGENT: &str = "emp-s76-logistics";
const UNASSIGNED_AGENT: &str = "emp-s76-unassigned";
const CATALOG_AGENT: &str = "emp-s76-catalog";

/// Seeds the scheduler's service identity the way production's bootstrap does
/// (`agent:manage` only, no organisation), with `RUN_TOKEN` as its credential.
/// The test router does not run the bootstrap, so a test that presents the
/// token as a bearer must seed it.
async fn seed_run_service(pool: &PgPool) {
    let identity = lakehouse_store::identity::create_service_identity(
        pool,
        &lakehouse_store::identity::CreateServiceIdentityInput {
            name: "test-agent-run-service".to_owned(),
            scopes: vec!["agent:manage".to_owned()],
            environment: "test".to_owned(),
        },
    )
    .await
    .expect("create the scheduler's service identity");
    lakehouse_auth::service_token::ensure_service_credential(
        pool,
        identity.id.parse().expect("a UUID service identity id"),
        &lakehouse_auth::Secret::new(RUN_TOKEN.to_owned()),
    )
    .await
    .expect("store the scheduler's credential");
}

/// A `GET` to `uri` with `headers`, through the router.
async fn get_with(
    app: &axum::Router,
    uri: &str,
    headers: &[(&str, &str)],
) -> axum::http::Response<Body> {
    let mut builder = Request::builder().method("GET").uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    app.clone()
        .oneshot(builder.body(Body::empty()).expect("build request"))
        .await
        .expect("router never fails a request outright")
}

/// The `id` of every employee or approval in a JSON array response.
fn listed_ids(body: &Value) -> Vec<String> {
    body.as_array()
        .expect("the list is a JSON array")
        .iter()
        .filter_map(|item| item["id"].as_str().map(str::to_owned))
        .collect()
}

/// A runnable build-mode agent in `tenant` (or none) whose ceiling is
/// `connector:manage`, so a run can ask to delete a connector.
async fn insert_connector_agent(pool: &PgPool, id: &str, tenant: Option<&str>) {
    sqlx::query(
        "INSERT INTO agent_employee (id, name, purpose, autonomy, status, prompt, mode, \
         permissions, tenant_id, read_roles) VALUES ($1, $2, 'test purpose', 'L1', 'ready', \
         'Hapus konektor yang tidak dipakai.', 'build', 'connector:manage', $3::uuid, \
         ARRAY['Analyst']::text[])",
    )
    .bind(id)
    .bind(format!("tenant-test-{id}"))
    .bind(tenant)
    .execute(pool)
    .await
    .expect("insert a connector agent");
}

/// Runs `employee` once on the scheduler's token, with a completion that asks
/// to delete `connector`. The write waits for a decision, so the run stops and
/// the pending approval it created is returned.
async fn run_waiting_on_a_delete(
    app: &TestApp,
    llm: &MockServer,
    employee: &str,
    connector: &str,
) -> String {
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
                        "function": {
                            "name": "delete_connector",
                            "arguments": json!({ "id": connector }).to_string(),
                        },
                    }],
                }
            }]
        })))
        .up_to_n_times(1)
        .mount(llm)
        .await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "sari@meridian.example").await;
    let resp = run_employee(&app.router, employee, &cookie, Some(RUN_TOKEN)).await;
    assert_eq!(resp.status(), StatusCode::OK, "the run starts");
    approval_of(&app.pool, employee).await
}

/// The pending approval a run of `employee` is waiting on.
async fn approval_of(pool: &PgPool, employee: &str) -> String {
    sqlx::query_scalar("SELECT id FROM approval_item WHERE employee_id = $1 AND status = 'pending'")
        .bind(employee)
        .fetch_one(pool)
        .await
        .expect("the run left one pending approval")
}

/// Inserts a pending `delete_connector` approval for `employee` directly in the
/// store, with the run it belongs to. For a refusal that happens before any
/// replay, where a headless run cannot start the approval (a run of an agent
/// with no organisation is refused first).
async fn pending_delete_for(pool: &PgPool, employee: &str, connector: &str) -> (String, String) {
    let run_id = format!("run-s76-{}", Uuid::new_v4());
    lakehouse_store::agents::create_run(pool, &run_id, employee, "schedule", "test agent")
        .await
        .expect("create the run");
    let approval_id = lakehouse_store::agents::create_linked_approval(
        pool,
        lakehouse_store::agents::LinkedApprovalRequest {
            employee_id: employee,
            employee_name: "test agent",
            run_id: &run_id,
            tool: "delete_connector",
            resource: Some(connector),
            reason: "test reason",
            risk: "test risk",
            redacted_args: &json!({ "id": connector }),
            requested_by: None,
        },
    )
    .await
    .expect("create the approval");
    (approval_id, run_id)
}

/// Inserts a pending `delete_connector` approval on the copilot's own run, the
/// way the interactive copilot leaves one: the run is waiting on the approval
/// and its last step holds the stored call, so a decision can replay it.
async fn copilot_delete_waiting(pool: &PgPool, connector: &str) -> String {
    let run_id = format!("run-s76-copilot-{}", Uuid::new_v4());
    let copilot = lakehouse_store::agents::COPILOT_EMPLOYEE_ID;
    lakehouse_store::agents::create_run(pool, &run_id, copilot, "copilot", "test copilot")
        .await
        .expect("create the copilot's run");
    lakehouse_store::agents::append_run_step(
        pool,
        &run_id,
        lakehouse_store::agents::RunStep {
            id: "step-1".to_owned(),
            label: "Waiting for approval".to_owned(),
            status: "waiting".to_owned(),
            detail: json!({ "tool": "delete_connector", "args": { "id": connector } }).to_string(),
        },
    )
    .await
    .expect("store the copilot's tool call");
    lakehouse_store::agents::mark_run_waiting_approval(pool, &run_id)
        .await
        .expect("mark the copilot's run waiting");
    lakehouse_store::agents::create_linked_approval(
        pool,
        lakehouse_store::agents::LinkedApprovalRequest {
            employee_id: copilot,
            employee_name: "Copilot",
            run_id: &run_id,
            tool: "delete_connector",
            resource: Some(connector),
            reason: "test reason",
            risk: "test risk",
            redacted_args: &json!({ "id": connector }),
            requested_by: None,
        },
    )
    .await
    .expect("create the copilot's approval")
}

/// `POST /api/agents/approvals/{id}/decide` with `decision`, as the cookie holder.
async fn decide(
    app: &axum::Router,
    approval: &str,
    cookie: &str,
    decision: &str,
) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agents/approvals/{approval}/decide"))
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(json!({ "decision": decision }).to_string()))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright")
}

async fn approval_status(pool: &PgPool, approval: &str) -> String {
    sqlx::query_scalar("SELECT status FROM approval_item WHERE id = $1")
        .bind(approval)
        .fetch_one(pool)
        .await
        .expect("read the approval's status")
}

/// How many `refused` audit rows name this approval.
async fn refused_decisions(pool: &PgPool, approval: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_event \
         WHERE resource_kind = 'approval' AND resource_id = $1 AND outcome = 'refused'",
    )
    .bind(approval)
    .fetch_one(pool)
    .await
    .expect("count the refused decisions")
}

async fn connector_exists(pool: &PgPool, connector: &str) -> bool {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM connector WHERE id = $1")
        .bind(connector)
        .fetch_one(pool)
        .await
        .expect("count the connector");
    count == 1
}

/// Two organisations and an agent in neither: the list holds the caller's own
/// organisation's agents, and no other.
#[tokio::test]
async fn a_person_sees_only_the_agents_of_their_organisation_in_the_list() {
    let app = spin_up().await;
    insert_employee(&app.pool, RETAIL_AGENT, Some(RETAIL)).await;
    insert_employee(&app.pool, LOGISTICS_AGENT, Some(LOGISTICS)).await;
    insert_employee(&app.pool, UNASSIGNED_AGENT, None).await;
    let cookie = principal_in(&app.pool, "query:read", &[RETAIL]).await;

    let resp = get_with(
        &app.router,
        "/api/agents/employees",
        &[("cookie", cookie.as_str())],
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let ids = listed_ids(&json_body(resp).await);
    assert!(ids.contains(&RETAIL_AGENT.to_owned()), "{ids:?}");
    assert!(!ids.contains(&LOGISTICS_AGENT.to_owned()), "{ids:?}");
    assert!(!ids.contains(&UNASSIGNED_AGENT.to_owned()), "{ids:?}");
}

/// The detail of an agent outside the caller's organisation is the answer for
/// an unknown agent (404), never 403, and the agent with no organisation is
/// the same.
#[tokio::test]
async fn a_person_gets_not_found_for_an_agent_outside_their_organisation() {
    let app = spin_up().await;
    insert_employee(&app.pool, RETAIL_AGENT, Some(RETAIL)).await;
    insert_employee(&app.pool, LOGISTICS_AGENT, Some(LOGISTICS)).await;
    insert_employee(&app.pool, UNASSIGNED_AGENT, None).await;
    let cookie = principal_in(&app.pool, "query:read", &[RETAIL]).await;

    for agent in [LOGISTICS_AGENT, UNASSIGNED_AGENT] {
        let resp = get_with(
            &app.router,
            &format!("/api/agents/employees/{agent}"),
            &[("cookie", cookie.as_str())],
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{agent}");
    }
    let own = get_with(
        &app.router,
        &format!("/api/agents/employees/{RETAIL_AGENT}"),
        &[("cookie", cookie.as_str())],
    )
    .await;
    assert_eq!(own.status(), StatusCode::OK);
}

/// An unrestricted caller sees every agent, the one with no organisation too.
#[tokio::test]
async fn an_unrestricted_person_sees_every_agent_including_one_with_no_organisation() {
    let app = spin_up().await;
    insert_employee(&app.pool, RETAIL_AGENT, Some(RETAIL)).await;
    insert_employee(&app.pool, LOGISTICS_AGENT, Some(LOGISTICS)).await;
    insert_employee(&app.pool, UNASSIGNED_AGENT, None).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;

    let resp = get_with(
        &app.router,
        "/api/agents/employees",
        &[("cookie", cookie.as_str())],
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let ids = listed_ids(&json_body(resp).await);
    for agent in [RETAIL_AGENT, LOGISTICS_AGENT, UNASSIGNED_AGENT] {
        assert!(
            ids.contains(&agent.to_owned()),
            "{agent} is listed: {ids:?}"
        );
    }
}

/// Dagster's schedule factory sends the run token as a bearer and as
/// `x-run-token`, and must still see every agent, the one with no organisation
/// too, or it would stop scheduling them.
#[tokio::test]
async fn the_scheduler_token_lists_every_agent() {
    let llm = MockServer::start().await;
    let app = spin_up_with_llm(&llm.uri(), Some(RUN_TOKEN)).await;
    seed_run_service(&app.pool).await;
    insert_employee(&app.pool, RETAIL_AGENT, Some(RETAIL)).await;
    insert_employee(&app.pool, LOGISTICS_AGENT, Some(LOGISTICS)).await;
    insert_employee(&app.pool, UNASSIGNED_AGENT, None).await;

    let resp = get_with(
        &app.router,
        "/api/agents/employees",
        &[
            ("authorization", "Bearer secret-run-token"),
            ("x-run-token", RUN_TOKEN),
        ],
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let ids = listed_ids(&json_body(resp).await);
    for agent in [RETAIL_AGENT, LOGISTICS_AGENT, UNASSIGNED_AGENT] {
        assert!(
            ids.contains(&agent.to_owned()),
            "{agent} is listed: {ids:?}"
        );
    }
}

/// The same bearer without the `x-run-token` header is an ordinary service
/// principal with no organisation, so it lists none of these agents.
#[tokio::test]
async fn a_bearer_without_the_run_token_header_is_filtered_like_any_service_principal() {
    let llm = MockServer::start().await;
    let app = spin_up_with_llm(&llm.uri(), Some(RUN_TOKEN)).await;
    seed_run_service(&app.pool).await;
    insert_employee(&app.pool, RETAIL_AGENT, Some(RETAIL)).await;
    insert_employee(&app.pool, LOGISTICS_AGENT, Some(LOGISTICS)).await;
    insert_employee(&app.pool, UNASSIGNED_AGENT, None).await;

    let resp = get_with(
        &app.router,
        "/api/agents/employees",
        &[("authorization", "Bearer secret-run-token")],
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let ids = listed_ids(&json_body(resp).await);
    for agent in [RETAIL_AGENT, LOGISTICS_AGENT, UNASSIGNED_AGENT] {
        assert!(
            !ids.contains(&agent.to_owned()),
            "{agent} is hidden: {ids:?}"
        );
    }
}

/// An approver in Retail cannot decide a Logistics agent's call: 404, the
/// approval stays pending, the connector is untouched, and the refusal is
/// audited once.
#[tokio::test]
async fn an_approver_outside_the_agents_organisation_gets_not_found_and_decides_nothing() {
    let llm = MockServer::start().await;
    let app = spin_up_with_llm(&llm.uri(), Some(RUN_TOKEN)).await;
    insert_connector_agent(&app.pool, LOGISTICS_AGENT, Some(LOGISTICS)).await;
    seed_connector(&app.pool, LOGISTICS_CONNECTOR, LOGISTICS).await;
    let approval = run_waiting_on_a_delete(&app, &llm, LOGISTICS_AGENT, LOGISTICS_CONNECTOR).await;
    let cookie = principal_in(
        &app.pool,
        "agent:approve, connector:manage, query:read",
        &[RETAIL],
    )
    .await;

    let resp = decide(&app.router, &approval, &cookie, "approved").await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(approval_status(&app.pool, &approval).await, "pending");
    assert!(connector_exists(&app.pool, LOGISTICS_CONNECTOR).await);
    assert_eq!(refused_decisions(&app.pool, &approval).await, 1);
}

/// SEC-76, decision 3, end to end: an approver who belongs to both Logistics
/// and Retail decides two calls that a Logistics agent made. The replay runs
/// with `tenant_ids == [Logistics]` only (`approval_replay_principal`), so the
/// Logistics connector is deleted, and the Retail connector, which the approver
/// may reach, is not. The approver's own permission (`connector:manage`) and
/// roles are the ones the replay keeps, so the deletion runs at all. Roles are
/// asserted directly in the `routes::agents` unit tests.
#[tokio::test]
async fn an_approver_in_the_agents_organisation_replays_the_call_there() {
    let llm = MockServer::start().await;
    let app = spin_up_with_llm(&llm.uri(), Some(RUN_TOKEN)).await;
    insert_connector_agent(&app.pool, LOGISTICS_AGENT, Some(LOGISTICS)).await;
    seed_connector(&app.pool, LOGISTICS_CONNECTOR, LOGISTICS).await;
    seed_connector(&app.pool, RETAIL_CONNECTOR, RETAIL).await;
    let cookie = principal_in(
        &app.pool,
        "agent:approve, connector:manage, query:read",
        &[LOGISTICS, RETAIL],
    )
    .await;

    // The Logistics agent's own connector: the replay runs in Logistics and
    // deletes it.
    let approval = run_waiting_on_a_delete(&app, &llm, LOGISTICS_AGENT, LOGISTICS_CONNECTOR).await;
    let resp = decide(&app.router, &approval, &cookie, "approved").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["executed"], json!(true), "{body}");
    assert!(!connector_exists(&app.pool, LOGISTICS_CONNECTOR).await);

    // The same Logistics agent asks to delete Retail's connector. The approver
    // reaches Retail, but the replay does not: the call fails and nothing is
    // deleted. Before the replay narrowed to the agent's organisation, this
    // call deleted it.
    let approval = run_waiting_on_a_delete(&app, &llm, LOGISTICS_AGENT, RETAIL_CONNECTOR).await;
    let resp = decide(&app.router, &approval, &cookie, "approved").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert!(body["result"]["error"].is_string(), "{body}");
    assert!(connector_exists(&app.pool, RETAIL_CONNECTOR).await);
}

/// The replay runs as the agent in the agent's organisation only. The approver
/// belongs to both organisations, but the call names Retail's connector, which
/// the replay cannot reach, so it is not deleted. Had the replay kept the
/// approver's two organisations, Retail's connector would be deleted.
#[tokio::test]
async fn a_replayed_call_cannot_reach_another_organisation_the_approver_belongs_to() {
    let llm = MockServer::start().await;
    let app = spin_up_with_llm(&llm.uri(), Some(RUN_TOKEN)).await;
    insert_connector_agent(&app.pool, LOGISTICS_AGENT, Some(LOGISTICS)).await;
    seed_connector(&app.pool, RETAIL_CONNECTOR, RETAIL).await;
    let approval = run_waiting_on_a_delete(&app, &llm, LOGISTICS_AGENT, RETAIL_CONNECTOR).await;
    let cookie = principal_in(
        &app.pool,
        "agent:approve, connector:manage, query:read",
        &[RETAIL, LOGISTICS],
    )
    .await;

    let resp = decide(&app.router, &approval, &cookie, "approved").await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert!(body["result"]["error"].is_string(), "{body}");
    assert!(connector_exists(&app.pool, RETAIL_CONNECTOR).await);
}

/// An agent with no organisation cannot have its call approved, even by an
/// unrestricted approver: 409 with the fixed sentence, the approval stays
/// pending, nothing executes, and the refusal is audited once.
#[tokio::test]
async fn an_approval_for_an_agent_with_no_organisation_is_a_conflict_and_stays_pending() {
    let app = spin_up().await;
    insert_connector_agent(&app.pool, UNASSIGNED_AGENT, None).await;
    seed_connector(&app.pool, LOGISTICS_CONNECTOR, LOGISTICS).await;
    let (approval, _run) =
        pending_delete_for(&app.pool, UNASSIGNED_AGENT, LOGISTICS_CONNECTOR).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;

    let resp = decide(&app.router, &approval, &cookie, "approved").await;

    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let body = json_body(resp).await;
    assert_eq!(body["error"].as_str(), Some(NO_ORGANISATION));
    assert_eq!(approval_status(&app.pool, &approval).await, "pending");
    assert!(connector_exists(&app.pool, LOGISTICS_CONNECTOR).await);
    assert_eq!(refused_decisions(&app.pool, &approval).await, 1);
}

/// The approvals list holds the caller's organisation's agents' approvals only,
/// and none for an agent with no organisation.
#[tokio::test]
async fn the_approvals_list_shows_only_the_callers_organisation() {
    let app = spin_up().await;
    insert_connector_agent(&app.pool, RETAIL_AGENT, Some(RETAIL)).await;
    insert_connector_agent(&app.pool, LOGISTICS_AGENT, Some(LOGISTICS)).await;
    insert_connector_agent(&app.pool, UNASSIGNED_AGENT, None).await;
    let (retail, _) = pending_delete_for(&app.pool, RETAIL_AGENT, RETAIL_CONNECTOR).await;
    let (logistics, _) = pending_delete_for(&app.pool, LOGISTICS_AGENT, LOGISTICS_CONNECTOR).await;
    let (unassigned, _) =
        pending_delete_for(&app.pool, UNASSIGNED_AGENT, LOGISTICS_CONNECTOR).await;
    let cookie = principal_in(&app.pool, "agent:approve", &[RETAIL]).await;

    let resp = get_with(
        &app.router,
        "/api/agents/approvals",
        &[("cookie", cookie.as_str())],
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let ids = listed_ids(&json_body(resp).await);
    assert!(ids.contains(&retail), "{ids:?}");
    assert!(!ids.contains(&logistics), "{ids:?}");
    assert!(!ids.contains(&unassigned), "{ids:?}");

    // An unrestricted caller sees every approval, the one of the agent with no
    // organisation included (SEC-76, decision 4).
    let admin = session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;
    let resp = get_with(
        &app.router,
        "/api/agents/approvals",
        &[("cookie", admin.as_str())],
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let ids = listed_ids(&json_body(resp).await);
    for approval in [&retail, &logistics, &unassigned] {
        assert!(ids.contains(approval), "{approval} is listed: {ids:?}");
    }
}

/// Run detail is narrowed through the run's agent. The caller is a member of
/// the catalog's organisation (so the shared catalog rule passes) and holds
/// `agent:read`, but not Logistics, so Logistics' run is not found.
#[tokio::test]
async fn the_run_detail_of_an_agent_outside_the_callers_organisation_is_not_found() {
    let app = spin_up_with_env(&HashMap::from([(
        "CATALOG_TENANT_ID".to_owned(),
        CATALOG_TENANT.to_owned(),
    )]))
    .await;
    insert_connector_agent(&app.pool, LOGISTICS_AGENT, Some(LOGISTICS)).await;
    let (_approval, run) =
        pending_delete_for(&app.pool, LOGISTICS_AGENT, LOGISTICS_CONNECTOR).await;
    let cookie = principal_in(&app.pool, "agent:read", &[CATALOG_TENANT]).await;

    let resp = get_with(
        &app.router,
        &format!("/api/agents/runs/{run}"),
        &[("cookie", cookie.as_str())],
    )
    .await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// The positive control for the run detail: a run of an agent in the caller's
/// own organisation is shown.
#[tokio::test]
async fn the_run_detail_of_an_agent_in_the_callers_organisation_is_shown() {
    let app = spin_up_with_env(&HashMap::from([(
        "CATALOG_TENANT_ID".to_owned(),
        CATALOG_TENANT.to_owned(),
    )]))
    .await;
    insert_connector_agent(&app.pool, CATALOG_AGENT, Some(CATALOG_TENANT)).await;
    let (_approval, run) = pending_delete_for(&app.pool, CATALOG_AGENT, RETAIL_CONNECTOR).await;
    let cookie = principal_in(&app.pool, "agent:read", &[CATALOG_TENANT]).await;

    let resp = get_with(
        &app.router,
        &format!("/api/agents/runs/{run}"),
        &[("cookie", cookie.as_str())],
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
}

/// SEC-76, decision 3 exemption (revision item 2): on a two-organisation
/// install the copilot has no organisation, and its approval is decided and
/// replayed as before. An approver in Retail gets 200 and the call executes;
/// the organisation rule that gives a 409 for an agent with no organisation does
/// not reach the copilot.
#[tokio::test]
async fn a_copilot_approval_is_decided_and_replayed_as_before_on_a_two_organisation_install() {
    let app = spin_up().await;
    seed_connector(&app.pool, RETAIL_CONNECTOR, RETAIL).await;
    let approval = copilot_delete_waiting(&app.pool, RETAIL_CONNECTOR).await;
    let cookie = principal_in(
        &app.pool,
        "agent:approve, connector:manage, query:read",
        &[RETAIL],
    )
    .await;

    let resp = decide(&app.router, &approval, &cookie, "approved").await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    assert_eq!(body["executed"], json!(true), "{body}");
    assert_eq!(approval_status(&app.pool, &approval).await, "approved");
    assert!(!connector_exists(&app.pool, RETAIL_CONNECTOR).await);
}
