//! `POST /api/pipelines/preview` (BLD-1 part 1 / D16, slice S2): the first
//! rows of one input or step of a pipeline definition that may not be saved
//! yet.
//!
//! Driven through the real router against a real per-test Postgres (users,
//! roles, tenants, policies) and a `wiremock` stand-in for `ClickHouse`, the
//! same harness `pipeline_compile_route.rs` uses — the preview runs the same
//! gate and the same rewrite a run does, as the CALLER (decision 13), so the
//! proof has to come from those same pieces.
//!
//! What these tests decide: `upTo` an input answers that input and `upTo` a
//! step answers that step; a column masked for the caller is masked in the
//! statement the engine runs; a table the caller may not read is refused with
//! the fixed sentence and nothing reaches the engine; a step that is not a
//! single `SELECT` is a 400 naming the step; the route needs `pipeline:write`;
//! and more than 100 rows answer 100 with `truncated: true`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::governance::{self, CreatePolicyInput};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_seeded_user,
    session_cookie_for_user, spin_up_with_env,
};

/// The tenant that owns the shared catalog in these tests (Meridian Group).
const GROUP_TENANT: &str = "11111111-1111-4111-8111-000000000001";

/// The catalog database a Bronze input is read through (`D4`).
const CATALOG_DB: &str = "icecat_api";

/// The metadata every rewrite reads, mounted before a test adds its own
/// answers: a governed table's columns (`system.columns`) and the view
/// catalog (`system.tables`, none).
async fn mount_rewrite_metadata(ch: &MockServer) {
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

/// The app and its own `ClickHouse` stand-in. A bare server for the reason
/// `pipeline_compile_route.rs` gives: a pooled one would hand another test an
/// address this one's app still points at.
async fn app() -> (axum::Router, PgPool, MockServer) {
    let ch = MockServer::builder().start().await;
    mount_rewrite_metadata(&ch).await;
    let mut overrides = std::collections::HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch.uri());
    overrides.insert("CATALOG_TENANT_ID".to_owned(), GROUP_TENANT.to_owned());
    overrides.insert("ICEBERG_QUERY_DB".to_owned(), CATALOG_DB.to_owned());
    let TestApp { router, pool } = spin_up_with_env(&overrides).await;
    (router, pool, ch)
}

/// The `DESCRIBE` the column probe reads (`name`, `type` per column).
fn describe_body(columns: &[(&str, &str)]) -> Value {
    json!({
        "meta": [{"name": "name", "type": "String"}, {"name": "type", "type": "String"}],
        "data": columns
            .iter()
            .map(|(name, ty)| json!({"name": name, "type": ty}))
            .collect::<Vec<_>>(),
        "rows": columns.len(),
    })
}

/// The row answer, in `ClickHouse`'s `FORMAT JSON` shape (objects per row).
fn rows_body(columns: &[(&str, &str)], rows: &[Value]) -> Value {
    json!({
        "meta": columns
            .iter()
            .map(|(name, ty)| json!({"name": name, "type": ty}))
            .collect::<Vec<_>>(),
        "data": rows,
        "rows": rows.len(),
    })
}

/// The two engine calls a preview that gets past the probe makes: the
/// `DESCRIBE` and the wrapped row query.
async fn mount_preview(engine: &MockServer, describe: Value, rows: Value) {
    Mock::given(method("POST"))
        .and(body_string_contains("DESCRIBE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(describe))
        .mount(engine)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains(" AS step LIMIT "))
        .respond_with(ResponseTemplate::new(200).set_body_json(rows))
        .mount(engine)
        .await;
}

/// Every body the engine was sent, as text.
async fn sent(engine: &MockServer) -> Vec<String> {
    engine
        .received_requests()
        .await
        .expect("the mock engine records requests")
        .iter()
        .map(|request| String::from_utf8_lossy(&request.body).into_owned())
        .collect()
}

/// One request to the route, as `cookie` acting in `tenant` (when given).
async fn preview(
    router: &axum::Router,
    cookie: &str,
    tenant: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri("/api/pipelines/preview")
        .header("cookie", cookie)
        .header("content-type", "application/json");
    if let Some(tenant) = tenant {
        request = request.header("x-tenant", tenant);
    }
    let resp = router
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// A mask on `table`'s `email` column for `role` (`GOV-4`, the same policy
/// `pipeline_compile_route.rs` seeds).
async fn seed_mask(pool: &PgPool, role: &str, table: &str) {
    governance::create_policy(
        pool,
        &CreatePolicyInput {
            name: format!("preview-mask-{role}"),
            kind: "Row filter".to_owned(),
            subjects: role.to_owned(),
            resources: table.to_owned(),
            effect: "Permit with obligation".to_owned(),
            conditions: Some(
                json!({"roles": [role], "table": table, "mask": ["email"]}).to_string(),
            ),
            activate: true,
            owner: None,
        },
    )
    .await
    .unwrap();
}

/// D16, decision 5: `upTo` an input answers that table's own rows, as the
/// person looking, without compiling any step.
#[tokio::test]
async fn previewing_an_input_returns_that_inputs_rows() {
    let (router, pool, engine) = app().await;
    mount_preview(
        &engine,
        describe_body(&[("id", "UInt64")]),
        rows_body(&[("id", "UInt64")], &[json!({"id": 7}), json!({"id": 8})]),
    )
    .await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let (status, answer) = preview(
        &router,
        &cookie,
        Some(GROUP_TENANT),
        json!({
            "inputs": [{"name": "orders", "zone": "bronze", "table": "group_orders"}],
            "steps": [],
            "upTo": "orders",
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        answer["columns"],
        json!([{"name": "id", "type": "UInt64"}]),
        "{answer}"
    );
    assert_eq!(answer["rows"], json!([[7], [8]]), "{answer}");
    assert_eq!(answer["truncated"], json!(false), "{answer}");
    let bodies = sent(&engine).await;
    assert!(
        bodies
            .iter()
            .any(|body| body.contains("SELECT * FROM icecat_api.`bronze.group_orders`")),
        "the input was not compiled through the catalog database: {bodies:?}"
    );
    assert!(
        bodies
            .iter()
            .any(|body| body.contains(") AS step LIMIT 101")),
        "the row query is not D16's wrapper: {bodies:?}"
    );
}

/// D16: `upTo` a step answers that step — the compiled definition stops
/// there, and the step's own text is what the engine runs.
#[tokio::test]
async fn previewing_a_step_runs_that_step() {
    let (router, pool, engine) = app().await;
    let step_sql = "SELECT c.id, count() AS n FROM orders AS o \
                    JOIN customers AS c ON o.customer_id = c.id GROUP BY c.id";
    mount_preview(
        &engine,
        describe_body(&[("id", "UInt64"), ("n", "UInt64")]),
        rows_body(
            &[("id", "UInt64"), ("n", "UInt64")],
            &[json!({"id": 1, "n": 3})],
        ),
    )
    .await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let (status, answer) = preview(
        &router,
        &cookie,
        Some(GROUP_TENANT),
        json!({
            "inputs": [
                {"name": "orders", "zone": "bronze", "table": "group_orders"},
                {"name": "customers", "zone": "bronze", "table": "group_customers"},
            ],
            "steps": [{"name": "joined", "kind": "sql", "sql": step_sql}],
            "upTo": "joined",
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["rows"], json!([[1, 3]]), "{answer}");
    let bodies = sent(&engine).await;
    let row_query = bodies
        .iter()
        .find(|body| body.contains(") AS step LIMIT 101"))
        .expect("the row query was sent");
    assert!(row_query.contains("JOIN customers"), "{row_query}");
    assert!(
        row_query.contains("SELECT * FROM `joined`"),
        "the definition did not stop at the previewed step: {row_query}"
    );
}

/// Decision 13: the preview runs as the person looking. A column masked for
/// them is masked in the statement the engine runs, and the rows the engine
/// answers are the rows the preview returns.
#[tokio::test]
async fn a_column_masked_for_the_caller_is_masked_in_the_rows() {
    let (router, pool, engine) = app().await;
    seed_mask(&pool, "Data Engineer", "serving.enforced_x").await;
    mount_preview(
        &engine,
        describe_body(&[("email", "String")]),
        rows_body(&[("email", "String")], &[json!({"email": "***"})]),
    )
    .await;
    // Bayu is a Data Engineer, the role the mask names.
    let cookie = session_cookie_for_seeded_user(&pool, "bayu@meridian.example").await;

    let (status, answer) = preview(
        &router,
        &cookie,
        Some(GROUP_TENANT),
        json!({
            "inputs": [{"name": "users", "zone": "serving", "table": "enforced_x"}],
            "steps": [],
            "upTo": "users",
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["rows"], json!([["***"]]), "{answer}");
    let bodies = sent(&engine).await;
    let row_query = bodies
        .iter()
        .find(|body| body.contains(") AS step LIMIT 101"))
        .expect("the row query was sent");
    assert!(
        row_query.contains("replaceRegexpOne(toString(`email`)"),
        "the caller's mask did not reach the engine: {row_query}"
    );
}

/// `GOV-7`: a table the caller may not read is refused with the fixed
/// sentence, as a 422 (D16's contract), and nothing reaches the engine.
#[tokio::test]
async fn an_input_the_caller_may_not_read_is_refused_before_the_engine() {
    let (router, pool, engine) = app().await;
    // A person who may build pipelines and read one table of the catalog.
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
        "INSERT INTO role_read_grant (role_id, schema_name, table_name) \
         VALUES ($1, 'serving', 'allowed')",
    )
    .bind(role)
    .execute(&pool)
    .await
    .unwrap();
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, answer) = preview(
        &router,
        &cookie,
        None,
        json!({
            "inputs": [{"name": "hidden", "zone": "silver", "table": "orders"}],
            "steps": [],
            "upTo": "hidden",
        }),
    )
    .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    assert_eq!(
        answer["error"],
        json!("Table not found or not available to your organisation: silver.orders"),
        "{answer}"
    );
    assert!(
        engine.received_requests().await.unwrap().is_empty(),
        "the refusal must precede the engine, not even the probe may run"
    );
}

/// D5 and D7: a step that is not one `SELECT` is a 400 whose message names
/// the step, so the editor can select it.
#[tokio::test]
async fn a_step_that_is_not_a_select_is_a_400_naming_the_step() {
    let (router, pool, _engine) = app().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let (status, answer) = preview(
        &router,
        &cookie,
        Some(GROUP_TENANT),
        json!({
            "inputs": [{"name": "orders", "zone": "silver", "table": "orders"}],
            "steps": [{"name": "dropped", "kind": "sql", "sql": "DROP TABLE serving.mart_x"}],
            "upTo": "dropped",
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    let message = answer["error"].as_str().unwrap_or_default();
    assert!(message.starts_with("steps[0]: "), "{message}");
}

/// D16: the preview is behind `pipeline:write`; a reader gets 403.
#[tokio::test]
async fn a_caller_without_pipeline_write_is_forbidden() {
    let (router, pool, _engine) = app().await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let (status, answer) = preview(
        &router,
        &cookie,
        None,
        json!({
            "inputs": [{"name": "orders", "zone": "silver", "table": "orders"}],
            "steps": [],
            "upTo": "orders",
        }),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN, "{answer}");
}

/// Decision 5: a preview returns the first 100 rows and says it cut them.
#[tokio::test]
async fn more_than_a_hundred_rows_answer_a_hundred_and_say_they_were_cut() {
    let (router, pool, engine) = app().await;
    let rows: Vec<Value> = (0..101).map(|id| json!({"id": id})).collect();
    mount_preview(
        &engine,
        describe_body(&[("id", "UInt64")]),
        rows_body(&[("id", "UInt64")], &rows),
    )
    .await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let (status, answer) = preview(
        &router,
        &cookie,
        Some(GROUP_TENANT),
        json!({
            "inputs": [{"name": "orders", "zone": "silver", "table": "orders"}],
            "steps": [],
            "upTo": "orders",
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{answer}");
    let answered = answer["rows"].as_array().expect("rows is an array");
    assert_eq!(answered.len(), 100, "{answer}");
    assert_eq!(answered[0], json!([0]));
    assert_eq!(answered[99], json!([99]));
    assert_eq!(answer["truncated"], json!(true), "{answer}");
}
