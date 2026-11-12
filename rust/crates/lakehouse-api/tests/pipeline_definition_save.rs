//! BLD-1 part 1 / D3, D7, D9, D10, D12: the save path of an authored
//! pipeline.
//!
//! The route is driven through the real router against a real per-test
//! Postgres and a `wiremock` stand-in for `ClickHouse`, the harness
//! `pipeline_enforce.rs` uses — the save-time check is the same gate and the
//! same rewrite the run route applies, so the proof has to come from the same
//! place.
//!
//! What these tests decide: today's body still works and reports the new
//! shape; sending both forms is refused; a SQL step is compiled and stored;
//! a table the author may not read is refused when saved (F1) and a masked
//! column is not; an edit that changes what the pipeline reads moves the
//! author to the editor (D10, `SEC-71`) while an edit of the schedule does
//! not; a restore is held to the same rules; and another tenant's pipeline is
//! the same 404 as an unknown one.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_seeded_user,
    session_cookie_for_user, spin_up_with_env,
};
use lakehouse_store::governance::{self, CreatePolicyInput};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The tenant that owns the shared catalog (Meridian Group, the tenant the
/// seeded identity migration gives `rina`, `fajar` and `yoga`).
const GROUP_TENANT: &str = "11111111-1111-4111-8111-000000000001";
/// Another seeded tenant (Meridian Retail, `andi`'s).
const RETAIL_TENANT: &str = "11111111-1111-4111-8111-000000000002";

async fn mount(ch: &MockServer) {
    // The column metadata the mask needs (`policy_engine`'s obligations
    // source), the same mock `pipeline_enforce.rs` mounts and for the same
    // reason: an unprovable substitution is refused rather than guessed.
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
    Mock::given(method("POST"))
        .and(body_string_contains("system.tables"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "data": [], "rows": 0,
        })))
        .mount(ch)
        .await;
}

async fn app() -> (axum::Router, PgPool, MockServer) {
    let ch = MockServer::builder().start().await;
    mount(&ch).await;
    let mut overrides = std::collections::HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch.uri());
    overrides.insert("CATALOG_TENANT_ID".to_owned(), GROUP_TENANT.to_owned());
    let TestApp { router, pool } = spin_up_with_env(&overrides).await;
    (router, pool, ch)
}

/// A request as an in-tenant caller; `tenant` sets `X-Tenant`, so the
/// pipeline lands where the test says it does.
fn request(
    method: &str,
    uri: &str,
    cookie: &str,
    tenant: Option<&str>,
    body: Option<Value>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie);
    if let Some(tenant) = tenant {
        builder = builder.header("x-tenant", tenant);
    }
    match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

async fn send(router: &axum::Router, request: Request<Body>) -> (StatusCode, Value) {
    let resp = router.clone().oneshot(request).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// Today's body shape, which every console before this change sends.
fn legacy_body(name: &str) -> Value {
    json!({
        "name": name,
        "kind": "batch",
        "sourceZone": "silver",
        "sourceTable": "orders",
        "transforms": ["dedupe(id)", "select(id,total)"],
        "targetZone": "silver",
        "targetTable": format!("{name}_out"),
        "schedule": "manual",
    })
}

/// The new body shape: one input and one SQL step over it.
fn new_body(name: &str) -> Value {
    json!({
        "name": name,
        "kind": "batch",
        "inputs": [
            {"name": "orders", "zone": "silver", "table": "orders"},
            {"name": "customers", "zone": "silver", "table": "customers"},
        ],
        "steps": [
            {"name": "joined", "kind": "sql",
             "sql": "SELECT o.id, c.name FROM orders AS o LEFT JOIN customers AS c ON o.customer_id = c.id"},
        ],
        "targetZone": "silver",
        "targetTable": format!("{name}_out"),
        "schedule": "manual",
    })
}

/// D3: today's body still creates a pipeline, and both shapes are readable —
/// the flat fields every pre-BLD-1 caller sends, and the inputs/steps the
/// console is moving to.
#[tokio::test]
async fn today_s_body_still_creates_and_reports_both_forms() {
    let (router, pool, _ch) = app().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let (status, created) = send(
        &router,
        request(
            "POST",
            "/api/pipelines",
            &cookie,
            Some(GROUP_TENANT),
            Some(legacy_body("save-legacy")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();

    let (status, detail) = send(
        &router,
        request(
            "GET",
            &format!("/api/pipelines/{id}"),
            &cookie,
            Some(GROUP_TENANT),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    let definition = &detail["definition"];
    assert_eq!(definition["sourceZone"], "silver");
    assert_eq!(definition["sourceTable"], "orders");
    assert_eq!(definition["transforms"][0], "dedupe(id)");
    assert_eq!(definition["inputs"][0]["name"], "orders");
    assert_eq!(definition["inputs"][0]["zone"], "silver");
    assert_eq!(definition["steps"][0]["name"], "transformed");
    assert_eq!(definition["steps"][0]["kind"], "transform");
    assert_eq!(definition["steps"][0]["transforms"][1], "select(id,total)");
}

#[tokio::test]
async fn a_body_with_both_forms_is_400() {
    let (router, pool, _ch) = app().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let mut body = legacy_body("save-both");
    body["inputs"] = json!([{"name": "orders", "zone": "silver", "table": "orders"}]);
    let (status, answer) = send(
        &router,
        request(
            "POST",
            "/api/pipelines",
            &cookie,
            Some(GROUP_TENANT),
            Some(body),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
}

/// D1/D4/D6: a pipeline written as inputs and a SQL step is stored with both,
/// and the compiled text names the inputs as CTEs.
#[tokio::test]
async fn a_new_form_body_creates_a_sql_step_pipeline() {
    let (router, pool, _ch) = app().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let (status, created) = send(
        &router,
        request(
            "POST",
            "/api/pipelines",
            &cookie,
            Some(GROUP_TENANT),
            Some(new_body("save-new")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();

    let (status, detail) = send(
        &router,
        request(
            "GET",
            &format!("/api/pipelines/{id}"),
            &cookie,
            Some(GROUP_TENANT),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    let definition = &detail["definition"];
    assert_eq!(definition["inputs"].as_array().unwrap().len(), 2);
    assert_eq!(definition["steps"][0]["kind"], "sql");
    // D3: a two-input pipeline is not one input and one transform step, so the
    // legacy `transforms` field is empty rather than describing half of it.
    assert_eq!(definition["transforms"], json!([]));
    assert_eq!(definition["sourceZone"], "silver");
    assert_eq!(definition["sourceTable"], "orders");
}

/// D9 (F1) + `GOV-7`: a table the author may not read is refused when the
/// pipeline is SAVED, not only when it runs.
///
/// A table outside the inputs is refused earlier by D6 as a 400, so the
/// GOV-7 refusal can only be about an input — which is exactly the acceptance
/// case ("add T as an input and save").
#[tokio::test]
async fn saving_a_table_the_author_may_not_read_is_422_naming_the_input() {
    let (router, pool, _ch) = app().await;
    // A person who may build pipelines and read only one table.
    let user = create_principal_with_permissions(&pool, "pipeline:write, query:read").await;
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(user)
        .bind(Uuid::parse_str(GROUP_TENANT).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    let role: Uuid = sqlx::query_scalar("SELECT role_id FROM app_user_role WHERE user_id = $1")
        .bind(user)
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE role SET read_scope = 'listed' WHERE id = $1")
        .bind(role)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO role_read_grant (role_id, schema_name, table_name) VALUES ($1, 'silver', 'allowed')",
    )
    .bind(role)
    .execute(&pool)
    .await
    .unwrap();
    let cookie = session_cookie_for_user(&pool, user).await;

    let body = json!({
        "name": "save-governed",
        "kind": "batch",
        "inputs": [{"name": "hidden", "zone": "silver", "table": "orders"}],
        "steps": [{"name": "copied", "kind": "sql", "sql": "SELECT id FROM hidden"}],
        "targetZone": "silver",
        "targetTable": "save_governed_out",
        "schedule": "manual",
    });
    let (status, answer) = send(
        &router,
        request(
            "POST",
            "/api/pipelines",
            &cookie,
            Some(GROUP_TENANT),
            Some(body),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    let message = answer["error"].as_str().unwrap_or_default();
    assert!(
        message.starts_with("inputs[0]: "),
        "the refusal must name the input at fault: {message}"
    );
}

/// D9: a column the author's role has masked is not a refusal — the run
/// masks it, so the pipeline is allowed to be saved.
#[tokio::test]
async fn a_masked_column_passes_the_save() {
    let (router, pool, _ch) = app().await;
    let cookie = session_cookie_for_seeded_user(&pool, "yoga@meridian.example").await;
    governance::create_policy(
        &pool,
        &CreatePolicyInput {
            name: "save-mask".to_owned(),
            kind: "Row filter".to_owned(),
            subjects: "Data Engineer".to_owned(),
            resources: "serving.enforced_x".to_owned(),
            effect: "Permit with obligation".to_owned(),
            conditions: Some(
                json!({"roles": ["Data Engineer"], "table": "serving.enforced_x", "mask": ["email"]})
                    .to_string(),
            ),
            activate: true,
            owner: None,
        },
    )
    .await
    .unwrap();
    let body = json!({
        "name": "save-masked",
        "kind": "batch",
        "inputs": [{"name": "governed", "zone": "serving", "table": "enforced_x"}],
        "steps": [{"name": "copied", "kind": "sql", "sql": "SELECT email FROM governed"}],
        "targetZone": "silver",
        "targetTable": "save_masked_out",
        "schedule": "manual",
    });
    let (status, answer) = send(
        &router,
        request(
            "POST",
            "/api/pipelines",
            &cookie,
            Some(GROUP_TENANT),
            Some(body),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{answer}");
}

/// D10 (`SEC-71`): an edit of the STEPS by someone other than the author
/// makes that person the author, and the edit is checked as them.
#[tokio::test]
async fn an_edit_of_the_steps_by_another_user_moves_the_author() {
    let (router, pool, _ch) = app().await;
    let admin = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let (status, created) = send(
        &router,
        request(
            "POST",
            "/api/pipelines",
            &admin,
            Some(GROUP_TENANT),
            Some(new_body("save-moves-author")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();
    let author_before = created["runAs"].as_str().unwrap().to_owned();

    // A Data Engineer of the same tenant edits the steps.
    let engineer = session_cookie_for_seeded_user(&pool, "yoga@meridian.example").await;
    let mut body = new_body("save-moves-author");
    body["steps"] = json!([
        {"name": "joined", "kind": "sql",
         "sql": "SELECT o.id FROM orders AS o"},
    ]);
    let (status, edited) = send(
        &router,
        request(
            "PUT",
            &format!("/api/pipelines/{id}"),
            &engineer,
            Some(GROUP_TENANT),
            Some(body),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    let author_after = edited["runAs"].as_str().unwrap_or_default().to_owned();
    assert_ne!(author_after, author_before, "the edit must move the author");
    let yoga: Uuid = sqlx::query_scalar("SELECT id FROM app_user WHERE email = $1")
        .bind("yoga@meridian.example")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(author_after, yoga.to_string());
}

/// D10: an edit that changes neither the inputs nor the steps (only the
/// schedule) leaves the author alone.
#[tokio::test]
async fn an_edit_of_only_the_schedule_leaves_the_author() {
    let (router, pool, _ch) = app().await;
    let admin = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let (status, created) = send(
        &router,
        request(
            "POST",
            "/api/pipelines",
            &admin,
            Some(GROUP_TENANT),
            Some(new_body("save-keeps-author")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();
    let author_before = created["runAs"].as_str().unwrap().to_owned();

    let engineer = session_cookie_for_seeded_user(&pool, "yoga@meridian.example").await;
    let mut body = new_body("save-keeps-author");
    body["schedule"] = json!("0 4 * * *");
    let (status, edited) = send(
        &router,
        request(
            "PUT",
            &format!("/api/pipelines/{id}"),
            &engineer,
            Some(GROUP_TENANT),
            Some(body),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(
        edited["runAs"].as_str().unwrap_or_default(),
        author_before,
        "a schedule edit must not change who the pipeline reads as"
    );
}

/// D9 + D13: a restore is held to the same checks — an old version that reads
/// differently is compiled, gated and rewritten, and D10 moves the author to
/// the person restoring it.
#[tokio::test]
async fn restoring_an_earlier_version_runs_the_same_checks_and_moves_the_author() {
    let (router, pool, _ch) = app().await;
    let admin = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let (status, created) = send(
        &router,
        request(
            "POST",
            "/api/pipelines",
            &admin,
            Some(GROUP_TENANT),
            Some(new_body("save-restore")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();
    let author_before = created["runAs"].as_str().unwrap_or_default().to_owned();

    // A second version with a different step, by the admin.
    let mut body = new_body("save-restore");
    body["steps"] = json!([
        {"name": "joined", "kind": "sql",
         "sql": "SELECT o.id FROM orders AS o"},
    ]);
    let (status, edited) = send(
        &router,
        request(
            "PUT",
            &format!("/api/pipelines/{id}"),
            &admin,
            Some(GROUP_TENANT),
            Some(body),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");

    // Someone else restores version 1: the definition reads differently from
    // the current one, so they become the author.
    let engineer = session_cookie_for_seeded_user(&pool, "yoga@meridian.example").await;
    let (status, restored) = send(
        &router,
        request(
            "POST",
            &format!("/api/pipelines/{id}/versions/1/restore"),
            &engineer,
            Some(GROUP_TENANT),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    let yoga: Uuid = sqlx::query_scalar("SELECT id FROM app_user WHERE email = $1")
        .bind("yoga@meridian.example")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        restored["runAs"].as_str().unwrap_or_default(),
        yoga.to_string()
    );
    assert_ne!(
        author_before,
        restored["runAs"].as_str().unwrap_or_default()
    );
}

/// F2.1/SEC: another tenant's pipeline answers the same 404 as an id that
/// does not exist — the save path adds no oracle.
#[tokio::test]
async fn a_pipeline_in_another_tenant_answers_the_same_404_as_an_unknown_one() {
    let (router, pool, _ch) = app().await;
    let admin = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let (status, created) = send(
        &router,
        request(
            "POST",
            "/api/pipelines",
            &admin,
            Some(GROUP_TENANT),
            Some(legacy_body("save-cross-tenant")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();

    let outsider = session_cookie_for_seeded_user(&pool, "andi@meridian.example").await;
    let (out_of_scope, their_body) = send(
        &router,
        request(
            "PUT",
            &format!("/api/pipelines/{id}"),
            &outsider,
            Some(RETAIL_TENANT),
            Some(legacy_body("save-cross-tenant")),
        ),
    )
    .await;
    let (unknown, unknown_body) = send(
        &router,
        request(
            "PUT",
            "/api/pipelines/pl-does-not-exist",
            &outsider,
            Some(RETAIL_TENANT),
            Some(legacy_body("save-cross-tenant")),
        ),
    )
    .await;
    assert_eq!(out_of_scope, StatusCode::NOT_FOUND, "{their_body}");
    assert_eq!(unknown, StatusCode::NOT_FOUND, "{unknown_body}");
    // The sentence names the id the caller sent -- which the caller already
    // knows -- and nothing else: the two failures differ only in that id.
    assert_eq!(
        their_body["error"]
            .as_str()
            .unwrap_or_default()
            .replace(&id, "<id>"),
        unknown_body["error"]
            .as_str()
            .unwrap_or_default()
            .replace("pl-does-not-exist", "<id>"),
        "an out-of-tenant id must be indistinguishable from one that does not exist"
    );
}

/// R1-B3 / D3a: until S3 ships, the console sends today's form on every
/// save. Over a pipeline with its own inputs and steps that form cannot hold
/// them, so it is refused (409) and nothing changes — rather than storing one
/// input and no step, which the next run would use to overwrite the output
/// with a copy of the first input.
#[tokio::test]
async fn a_today_s_form_edit_of_a_two_input_pipeline_is_409_and_changes_nothing() {
    let (router, pool, _ch) = app().await;
    let admin = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let (status, created) = send(
        &router,
        request(
            "POST",
            "/api/pipelines",
            &admin,
            Some(GROUP_TENANT),
            Some(new_body("save-d3a")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();

    let (status, refused) = send(
        &router,
        request(
            "PUT",
            &format!("/api/pipelines/{id}"),
            &admin,
            Some(GROUP_TENANT),
            Some(legacy_body("save-d3a")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(
        refused["error"].as_str(),
        Some(
            "This pipeline has inputs and steps this form cannot hold; edit it in the pipeline \
             editor, which sends inputs and steps."
        ),
        "{refused}"
    );

    let (_, detail) = send(
        &router,
        request(
            "GET",
            &format!("/api/pipelines/{id}"),
            &admin,
            Some(GROUP_TENANT),
            None,
        ),
    )
    .await;
    let definition = &detail["definition"];
    assert_eq!(definition["inputs"].as_array().unwrap().len(), 2);
    assert_eq!(definition["steps"][0]["kind"], "sql");
}

/// R1-B3 / D3a: on a pipeline today's form CAN hold, a save in that form
/// keeps the input's and the step's own names and changes only the zone, the
/// table and the transforms — so it neither renames the pipeline's definition
/// nor moves who it reads as.
#[tokio::test]
async fn a_today_s_form_edit_keeps_the_stored_names_and_the_author() {
    let (router, pool, _ch) = app().await;
    let admin = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let create = json!({
        "name": "save-d3a-names",
        "kind": "batch",
        "inputs": [{"name": "orders_named", "zone": "silver", "table": "orders"}],
        "steps": [{"name": "my_step", "kind": "transform", "reads": "orders_named",
                   "transforms": ["select(id)"]}],
        "targetZone": "silver",
        "targetTable": "save_d3a_names_out",
        "schedule": "manual",
    });
    let (status, created) = send(
        &router,
        request(
            "POST",
            "/api/pipelines",
            &admin,
            Some(GROUP_TENANT),
            Some(create),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();
    let author = created["runAs"].as_str().unwrap_or_default().to_owned();

    // The same transforms, sent in today's form, by someone else.
    let engineer = session_cookie_for_seeded_user(&pool, "yoga@meridian.example").await;
    let body = json!({
        "kind": "batch",
        "sourceZone": "silver",
        "sourceTable": "orders",
        "transforms": ["select(id)"],
        "targetZone": "silver",
        "targetTable": "save_d3a_names_out",
        "schedule": "manual",
    });
    let (status, edited) = send(
        &router,
        request(
            "PUT",
            &format!("/api/pipelines/{id}"),
            &engineer,
            Some(GROUP_TENANT),
            Some(body),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");

    let (_, detail) = send(
        &router,
        request(
            "GET",
            &format!("/api/pipelines/{id}"),
            &engineer,
            Some(GROUP_TENANT),
            None,
        ),
    )
    .await;
    let definition = &detail["definition"];
    assert_eq!(definition["inputs"][0]["name"], "orders_named", "{detail}");
    assert_eq!(definition["steps"][0]["name"], "my_step", "{detail}");
    assert_eq!(
        edited["runAs"].as_str().unwrap_or_default(),
        author,
        "the definition did not change, so the author must not move"
    );
}

/// R1-B9 (D9 + D10): someone other than the author changes what the pipeline
/// reads, and the save is checked as THEM — so a table they may not read is
/// refused, naming the input, instead of the edit being accepted and the
/// pipeline continuing to read as its old author (`SEC-71`).
#[tokio::test]
async fn an_editors_change_is_refused_when_they_may_not_read_an_input() {
    let (router, pool, _ch) = app().await;
    let admin = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let (status, created) = send(
        &router,
        request(
            "POST",
            "/api/pipelines",
            &admin,
            Some(GROUP_TENANT),
            Some(new_body("save-editor-refused")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();

    // A Data Engineer who may build pipelines and read only `finance.orders`
    // (a table this pipeline does not read).
    let editor = create_principal_with_permissions(&pool, "pipeline:write, query:read").await;
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(editor)
        .bind(Uuid::parse_str(GROUP_TENANT).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    let role: Uuid = sqlx::query_scalar("SELECT role_id FROM app_user_role WHERE user_id = $1")
        .bind(editor)
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE role SET read_scope = 'listed' WHERE id = $1")
        .bind(role)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO role_read_grant (role_id, schema_name, table_name) \
         VALUES ($1, 'finance', 'orders')",
    )
    .bind(role)
    .execute(&pool)
    .await
    .unwrap();
    let cookie = session_cookie_for_user(&pool, editor).await;

    // The editor changes the steps; the inputs (`silver.orders`,
    // `silver.customers`) are tables they may not read.
    let mut body = new_body("save-editor-refused");
    body["steps"] = json!([
        {"name": "joined", "kind": "sql", "sql": "SELECT o.id FROM orders AS o"},
    ]);
    let (status, refused) = send(
        &router,
        request(
            "PUT",
            &format!("/api/pipelines/{id}"),
            &cookie,
            Some(GROUP_TENANT),
            Some(body),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    let message = refused["error"].as_str().unwrap_or_default();
    assert!(
        message.starts_with("inputs["),
        "the refusal must name the input they may not read: {message}"
    );
}

/// R1-S4 / D13: a version written before `inputs`/`steps` existed restores
/// through the route into D3's shape — one input named after the table, one
/// `transform` step — and runs the same checks as a save.
#[tokio::test]
async fn an_old_snapshot_restores_through_the_route_into_todays_shape() {
    let (router, pool, _ch) = app().await;
    let admin = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let (status, created) = send(
        &router,
        request(
            "POST",
            "/api/pipelines",
            &admin,
            Some(GROUP_TENANT),
            Some(legacy_body("save-old-snapshot")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();

    // A snapshot in the pre-BLD-1 shape: no `inputs`, no `steps`.
    let old = json!({
        "kind": "batch",
        "sourceZone": "silver",
        "sourceTable": "orders",
        "incrementalColumn": null,
        "transforms": ["dedupe(id)"],
        "fbicEnabled": false,
        "targetZone": "silver",
        "targetTable": "save_old_snapshot_out",
        "schedule": "manual",
        "owner": "Current user",
        "description": null,
        "maxRetries": 2,
        "dependsOn": [],
        "name": "save-old-snapshot",
        "status": "draft"
    });
    sqlx::query(
        "UPDATE pipeline_definition_version SET snapshot = $2 \
         WHERE pipeline_id = $1 AND version = 1",
    )
    .bind(&id)
    .bind(&old)
    .execute(&pool)
    .await
    .unwrap();

    let (status, restored) = send(
        &router,
        request(
            "POST",
            &format!("/api/pipelines/{id}/versions/1/restore"),
            &admin,
            Some(GROUP_TENANT),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{restored}");

    let (_, detail) = send(
        &router,
        request(
            "GET",
            &format!("/api/pipelines/{id}"),
            &admin,
            Some(GROUP_TENANT),
            None,
        ),
    )
    .await;
    let definition = &detail["definition"];
    assert_eq!(definition["inputs"][0]["name"], "orders", "{detail}");
    assert_eq!(definition["inputs"][0]["table"], "orders", "{detail}");
    assert_eq!(definition["steps"][0]["name"], "transformed", "{detail}");
    assert_eq!(definition["steps"][0]["reads"], "orders", "{detail}");
    assert_eq!(definition["steps"][0]["kind"], "transform", "{detail}");
}
