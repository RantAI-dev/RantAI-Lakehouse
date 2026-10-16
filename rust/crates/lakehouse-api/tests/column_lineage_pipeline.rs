//! Column lineage for authored pipelines (`DATA-13`, finding F2), driven
//! through the real router against a per-test Postgres and `wiremock`
//! stand-ins for `ClickHouse` and `Dagster`.
//!
//! The record has two halves with a run in between. The orchestrator posts
//! the SELECT it will run to `.../enforce`, and the API keeps it in Postgres
//! (not in memory: it can restart before the run ends). When Dagster's
//! sensor reports the run `SUCCESS`, the API maps the kept SQL and writes the
//! pipeline's rows. A run that did not succeed writes nothing.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::column_lineage::{self, Direction, Scope};
use lakehouse_store::pipelines::{self, CreatePipelineInput};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

/// What the orchestrator submits for a Bronze source (`authored_factory.py`
/// reads it as ``<catalog db>.`bronze.<table>` ``): one copy, one rename and
/// one cast.
const SQL: &str = "SELECT id, amount AS amt, CAST(price AS Float64) AS price_f \
                   FROM icecat.`bronze.orders`";

/// The router, its database, an admin's session cookie, and the two mock
/// servers (returned so they stay up for the whole test).
async fn app(run_status: &str) -> (axum::Router, PgPool, String, (MockServer, MockServer)) {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("system.columns"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [
                {"name": "name", "type": "String"}, {"name": "default_kind", "type": "String"},
                {"name": "default_expression", "type": "String"}
            ],
            "data": [
                {"name": "id", "default_kind": "", "default_expression": ""},
                {"name": "amount", "default_kind": "", "default_expression": ""},
                {"name": "price", "default_kind": "", "default_expression": ""},
            ],
            "rows": 3,
        })))
        .mount(&ch)
        .await;
    // Anything else (the view catalog, background passes started by the
    // event) answers an empty 200.
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "data": [], "rows": 0,
        })))
        .mount(&ch)
        .await;
    let dagster = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("pipelineRunOrError"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "pipelineRunOrError": {
                "__typename": "Run", "status": run_status,
                "startTime": 1000.0, "endTime": 1010.0, "stepStats": []
            } }
        })))
        .mount(&dagster)
        .await;
    let mut overrides = std::collections::HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch.uri());
    overrides.insert(
        "DAGSTER_URL".to_owned(),
        format!("{}/graphql", dagster.uri()),
    );
    let TestApp { router, pool } = spin_up_with_env(&overrides).await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    (router, pool, cookie, (ch, dagster))
}

async fn seeded_user_id(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM app_user WHERE email = $1")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// A runnable pipeline `bronze.orders -> silver.orders_clean`.
async fn seed_pipeline(pool: &PgPool, run_as: Option<Uuid>) -> String {
    let created = pipelines::create_pipeline(
        pool,
        &CreatePipelineInput {
            name: "lineage-probe".to_owned(),
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
            tenant_id: None,
            depends_on: Vec::new(),
            connector_id: None,
            id: None,
            run_as,
        },
        run_as,
    )
    .await
    .unwrap();
    // `list_runnable_pipelines` (the job-name lookup) returns `ready` rows.
    sqlx::query("UPDATE pipeline_definition SET status = 'ready' WHERE id = $1")
        .bind(&created.id)
        .execute(pool)
        .await
        .unwrap();
    created.id
}

fn job_name(id: &str) -> String {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("authored__{safe}")
}

async fn post(router: &axum::Router, cookie: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(body.to_string()))
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

async fn enforce(router: &axum::Router, cookie: &str, id: &str) -> (StatusCode, Value) {
    post(
        router,
        cookie,
        &format!("/api/pipelines/runnable/{id}/enforce"),
        json!({ "sql": SQL }),
    )
    .await
}

async fn run_finished(router: &axum::Router, cookie: &str, id: &str) -> (StatusCode, Value) {
    post(
        router,
        cookie,
        "/api/pipelines/events/run-finished",
        json!({ "runId": "run-1", "jobName": job_name(id) }),
    )
    .await
}

fn everything() -> Scope {
    Scope {
        tenant: None,
        all_tenants: true,
        shared: true,
    }
}

async fn pending(pool: &PgPool, id: &str) -> Option<String> {
    column_lineage::get_pending(pool, id).await.unwrap()
}

#[tokio::test]
async fn the_enforce_call_keeps_the_submitted_sql_in_postgres() {
    let (router, pool, cookie, _servers) = app("SUCCESS").await;
    let author = seeded_user_id(&pool, "yoga@meridian.example").await;
    let id = seed_pipeline(&pool, Some(author)).await;

    let (status, body) = enforce(&router, &cookie, &id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(pending(&pool, &id).await.as_deref(), Some(SQL));
}

#[tokio::test]
async fn a_refused_enforce_keeps_nothing() {
    let (router, pool, cookie, _servers) = app("SUCCESS").await;
    let id = seed_pipeline(&pool, None).await;

    let (status, _) = enforce(&router, &cookie, &id).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(pending(&pool, &id).await, None);
}

#[tokio::test]
async fn a_successful_run_writes_the_pipelines_mappings_and_clears_the_pending_sql() {
    let (router, pool, cookie, _servers) = app("SUCCESS").await;
    let author = seeded_user_id(&pool, "yoga@meridian.example").await;
    let id = seed_pipeline(&pool, Some(author)).await;
    // Enforce through the route, then report the run: the pending row is the
    // only thing carried from one request to the other. A restart of the
    // API between the two loses nothing, because nothing is in memory.
    let (status, body) = enforce(&router, &cookie, &id).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = run_finished(&router, &cookie, &id).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let up = column_lineage::walk(
        &pool,
        "silver.orders_clean",
        "price_f",
        Direction::Up,
        everything(),
    )
    .await
    .unwrap();
    assert_eq!(up.len(), 1);
    assert_eq!(up[0].from_table.as_deref(), Some("bronze.orders"));
    assert_eq!(up[0].from_column.as_deref(), Some("price"));
    assert_eq!(up[0].transform, "cast");
    assert_eq!(up[0].producer_kind, "pipeline");
    assert_eq!(up[0].producer_ref, id);
    assert!(!up[0].partial);

    let rows = column_lineage::touching_table(&pool, "silver.orders_clean", everything())
        .await
        .unwrap();
    let mut pairs: Vec<(String, String, String)> = rows
        .iter()
        .map(|r| {
            (
                r.from_column.clone().unwrap(),
                r.to_column.clone(),
                r.transform.clone(),
            )
        })
        .collect();
    pairs.sort();
    assert_eq!(
        pairs,
        vec![
            ("amount".to_owned(), "amt".to_owned(), "copy".to_owned()),
            ("id".to_owned(), "id".to_owned(), "copy".to_owned()),
            ("price".to_owned(), "price_f".to_owned(), "cast".to_owned()),
        ]
    );
    assert_eq!(pending(&pool, &id).await, None, "the pending SQL is spent");
}

#[tokio::test]
async fn pending_sql_left_by_an_earlier_process_is_mapped_by_the_next_success() {
    // The "restart": the row was written by a process that no longer
    // exists; this app instance never saw the enforce call.
    let (router, pool, cookie, _servers) = app("SUCCESS").await;
    let id = seed_pipeline(&pool, None).await;
    column_lineage::put_pending(&pool, &id, SQL).await.unwrap();

    let (status, body) = run_finished(&router, &cookie, &id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = column_lineage::touching_table(&pool, "silver.orders_clean", everything())
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
}

#[tokio::test]
async fn a_run_that_did_not_succeed_writes_nothing() {
    let (router, pool, cookie, _servers) = app("FAILURE").await;
    let author = seeded_user_id(&pool, "yoga@meridian.example").await;
    let id = seed_pipeline(&pool, Some(author)).await;
    let (status, _) = enforce(&router, &cookie, &id).await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = run_finished(&router, &cookie, &id).await;
    assert_eq!(status, StatusCode::CONFLICT, "a non-SUCCESS run is refused");
    let rows = column_lineage::touching_table(&pool, "silver.orders_clean", everything())
        .await
        .unwrap();
    assert!(rows.is_empty(), "a failed run left a mapping: {rows:?}");
}

#[tokio::test]
async fn a_repeated_success_event_leaves_the_rows_as_they_were() {
    let (router, pool, cookie, _servers) = app("SUCCESS").await;
    let author = seeded_user_id(&pool, "yoga@meridian.example").await;
    let id = seed_pipeline(&pool, Some(author)).await;
    enforce(&router, &cookie, &id).await;
    run_finished(&router, &cookie, &id).await;
    let (status, _) = run_finished(&router, &cookie, &id).await;
    assert_eq!(status, StatusCode::OK);
    let rows = column_lineage::touching_table(&pool, "silver.orders_clean", everything())
        .await
        .unwrap();
    assert_eq!(rows.len(), 3, "nothing pending, so nothing was rewritten");
}
