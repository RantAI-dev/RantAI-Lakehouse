//! On-load quality checks (`DATA-14`, finding F4), driven through the real
//! `POST /api/pipelines/events/run-finished` route against a per-test
//! Postgres and `wiremock` stand-ins for `ClickHouse` and `Dagster`.
//!
//! Every check below fails (the stand-in says each table holds 5 rows and
//! the rules want 1000), so a rule that ran left an incident behind, and an
//! incident is how these tests see it. The follow-up runs in a background
//! task, so the tests wait for what must happen and, for what must not,
//! give the task time to have done it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::governance::{CreateQualityRuleInput, create_quality_rule};
use lakehouse_store::pipelines::{self, CreatePipelineInput};
use lakehouse_store::quality::{self, ScheduleMode, StatusFilter};
use lakehouse_store::uploads::{self, LoadMode, NewUpload};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

const USER: Uuid = Uuid::from_u128(0x3333_3333_3333_4333_8333_0000_0000_0001);
const SEED_TENANT: &str = "11111111-1111-4111-8111-000000000001";
/// The seeded connector of the alert tests.
const CONNECTOR: &str = "conn-pg-lakehouse";

struct App {
    router: axum::Router,
    pool: PgPool,
    cookie: String,
    ch: MockServer,
    dagster: MockServer,
}

async fn app() -> App {
    let ch = MockServer::start().await;
    let answer = |data: Value| {
        ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "rows": data.as_array().map_or(0, Vec::len), "data": data,
        }))
    };
    Mock::given(method("POST"))
        .and(body_string_contains("system.columns"))
        .respond_with(answer(json!([
            {"name": "id", "type": "String"}, {"name": "amount", "type": "Int64"},
        ])))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("DESCRIBE TABLE"))
        .respond_with(answer(json!([
            {"name": "id", "type": "String"}, {"name": "amount", "type": "Int64"},
        ])))
        .mount(&ch)
        .await;
    // The alert rules: none, so a connector run delivers nothing.
    Mock::given(method("POST"))
        .and(body_string_contains("FROM console.alert_rule"))
        .respond_with(answer(json!([])))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("toString(count()) AS n FROM"))
        .respond_with(answer(json!([{"n": "5"}])))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .respond_with(answer(json!([])))
        .mount(&ch)
        .await;
    let dagster = MockServer::start().await;
    let mut overrides = std::collections::HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch.uri());
    overrides.insert(
        "DAGSTER_URL".to_owned(),
        format!("{}/graphql", dagster.uri()),
    );
    overrides.insert("ICEBERG_QUERY_DB".to_owned(), "icecat_api".to_owned());
    let TestApp { router, pool } = spin_up_with_env(&overrides).await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    App {
        router,
        pool,
        cookie,
        ch,
        dagster,
    }
}

impl App {
    /// The orchestrator knows `run_id` (and, for a connector, its config).
    async fn knows_run(&self, run_id: &str, config: Value, status: &str) {
        Mock::given(method("POST"))
            .and(body_string_contains("runOrError"))
            .and(body_string_contains(run_id))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "runOrError": { "__typename": "Run", "runConfig": config } }
            })))
            .mount(&self.dagster)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("pipelineRunOrError"))
            .and(body_string_contains(run_id))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "pipelineRunOrError": {
                    "__typename": "Run", "status": status,
                    "startTime": 1000.0, "endTime": 1010.0, "stepStats": []
                } }
            })))
            .mount(&self.dagster)
            .await;
    }

    async fn run_finished(&self, run_id: &str, job_name: &str) -> (StatusCode, Value) {
        let resp = self
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/pipelines/events/run-finished")
                    .header("content-type", "application/json")
                    .header("cookie", &self.cookie)
                    .body(Body::from(
                        json!({ "runId": run_id, "jobName": job_name }).to_string(),
                    ))
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

    /// A rule that fails on any table the stand-in answers for.
    async fn failing_rule(&self, name: &str, asset: &str) {
        create_quality_rule(
            &self.pool,
            &CreateQualityRuleInput {
                name: name.to_owned(),
                asset: asset.to_owned(),
                dimension: "volume".to_owned(),
                threshold: "rows >= 1000".to_owned(),
                severity: "high".to_owned(),
            },
        )
        .await
        .unwrap();
    }

    async fn schedule(&self, asset: &str, mode: ScheduleMode) {
        quality::put_schedule(&self.pool, asset, mode, 1440, USER)
            .await
            .unwrap();
    }

    /// The assets with an open incident, sorted.
    async fn incident_assets(&self) -> Vec<String> {
        let mut assets: Vec<String> = quality::list(&self.pool, StatusFilter::Open, None)
            .await
            .unwrap()
            .into_iter()
            .map(|i| i.asset)
            .collect();
        assets.sort();
        assets
    }

    /// Waits until an incident for `asset` is open.
    async fn wait_for_incident(&self, asset: &str) {
        for _ in 0..100 {
            if self.incident_assets().await.iter().any(|a| a == asset) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!(
            "no incident opened for {asset}: {:?}",
            self.incident_assets().await
        );
    }

    /// Gives a background task that must NOT act time to have acted.
    async fn settle() {
        tokio::time::sleep(Duration::from_millis(600)).await;
    }

    async fn followups_recorded(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM pipeline_run_event WHERE kind = 'quality_load'")
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    async fn count_checks(&self) -> usize {
        self.ch
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| {
                // A rule's check has no `WHERE`; the load's own row count does.
                let sql = String::from_utf8_lossy(&r.body);
                sql.contains("toString(count()) AS n FROM") && !sql.contains("WHERE 1")
            })
            .count()
    }
}

async fn seed_pipeline(pool: &PgPool, target_zone: &str, target_table: &str) -> String {
    let created = pipelines::create_pipeline(
        pool,
        &CreatePipelineInput {
            name: "quality-on-load".to_owned(),
            kind: "batch".to_owned(),
            source_zone: "bronze".to_owned(),
            source_table: "orders".to_owned(),
            target_zone: target_zone.to_owned(),
            target_table: target_table.to_owned(),
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
            run_as: None,
            target_lag: None,
        },
        None,
    )
    .await
    .unwrap();
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

/// A pipeline's success runs the on-load rules of its target table, and only
/// those: not another table's, not a table on an interval.
#[tokio::test]
async fn a_pipelines_success_runs_the_on_load_rules_of_its_target() {
    let app = app().await;
    let id = seed_pipeline(&app.pool, "silver", "orders_clean").await;
    app.knows_run("run-p1", json!({}), "SUCCESS").await;
    app.failing_rule("target_rows", "silver.orders_clean").await;
    app.failing_rule("other_rows", "silver.other").await;
    app.failing_rule("interval_rows", "silver.interval_table")
        .await;
    app.schedule("silver.orders_clean", ScheduleMode::OnLoad)
        .await;
    app.schedule("silver.other", ScheduleMode::OnLoad).await;
    app.schedule("silver.interval_table", ScheduleMode::Interval)
        .await;

    let (status, body) = app.run_finished("run-p1", &job_name(&id)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    app.wait_for_incident("silver.orders_clean").await;
    App::settle().await;
    assert_eq!(app.incident_assets().await, vec!["silver.orders_clean"]);
}

/// A gold target is checked as the `serving` table it is.
#[tokio::test]
async fn a_gold_target_is_checked_as_serving() {
    let app = app().await;
    let id = seed_pipeline(&app.pool, "gold", "mart_sales").await;
    app.knows_run("run-p2", json!({}), "SUCCESS").await;
    app.failing_rule("mart_rows", "serving.mart_sales").await;
    app.schedule("serving.mart_sales", ScheduleMode::OnLoad)
        .await;

    let (status, _) = app.run_finished("run-p2", &job_name(&id)).await;
    assert_eq!(status, StatusCode::OK);
    app.wait_for_incident("serving.mart_sales").await;
}

/// A run the orchestrator did not finish successfully is refused by the
/// event, and triggers nothing.
#[tokio::test]
async fn a_failed_run_triggers_nothing() {
    let app = app().await;
    let id = seed_pipeline(&app.pool, "silver", "orders_clean").await;
    app.knows_run("run-p3", json!({}), "FAILURE").await;
    app.failing_rule("target_rows", "silver.orders_clean").await;
    app.schedule("silver.orders_clean", ScheduleMode::OnLoad)
        .await;

    let (status, _) = app.run_finished("run-p3", &job_name(&id)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    App::settle().await;
    assert!(app.incident_assets().await.is_empty());
    assert_eq!(app.followups_recorded().await, 0);
    assert_eq!(app.count_checks().await, 0);
}

/// A sensor that reports the same run twice follows the load up once.
#[tokio::test]
async fn a_sensor_retry_follows_the_load_up_once() {
    let app = app().await;
    let id = seed_pipeline(&app.pool, "silver", "orders_clean").await;
    app.knows_run("run-p4", json!({}), "SUCCESS").await;
    app.failing_rule("target_rows", "silver.orders_clean").await;
    app.schedule("silver.orders_clean", ScheduleMode::OnLoad)
        .await;

    app.run_finished("run-p4", &job_name(&id)).await;
    app.wait_for_incident("silver.orders_clean").await;
    app.run_finished("run-p4", &job_name(&id)).await;
    App::settle().await;
    assert_eq!(app.followups_recorded().await, 1);
    assert_eq!(app.count_checks().await, 1);
}

/// A connector run's success covers every table its ingest spec loads, and no
/// other table.
#[tokio::test]
async fn a_connector_runs_success_runs_the_rules_of_every_table_it_loads() {
    let app = app().await;
    sqlx::query(
        "UPDATE connector SET adapter = 'sql', ingest_mode = 'batch', dial = '{}'::jsonb, \
         source_objects = '[{\"name\":\"orders\",\"target\":\"orders_raw\"},\
         {\"name\":\"items\",\"target\":\"items_raw\"}]'::jsonb WHERE id = $1",
    )
    .bind(CONNECTOR)
    .execute(&app.pool)
    .await
    .unwrap();
    app.knows_run(
        "run-c1",
        json!({ "ops": { "run_ingest": { "config": { "connector_id": CONNECTOR } } } }),
        "SUCCESS",
    )
    .await;
    for (name, asset) in [
        ("orders_rows", "bronze.orders_raw"),
        ("items_rows", "bronze.items_raw"),
        ("not_loaded_rows", "bronze.elsewhere_raw"),
    ] {
        app.failing_rule(name, asset).await;
        app.schedule(asset, ScheduleMode::OnLoad).await;
    }

    let (status, body) = app.run_finished("run-c1", "ingest_job").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    app.wait_for_incident("bronze.orders_raw").await;
    app.wait_for_incident("bronze.items_raw").await;
    App::settle().await;
    assert_eq!(
        app.incident_assets().await,
        vec!["bronze.items_raw", "bronze.orders_raw"]
    );
}

// PR #106 review R1 BLOCKER 3
#[tokio::test]
async fn a_paused_connector_run_follows_up_no_tables() {
    let app = app().await;
    sqlx::query(
        "UPDATE connector SET adapter = 'sql', ingest_mode = 'batch', dial = '{}'::jsonb, \
         source_objects = '[{\"name\":\"orders\",\"target\":\"orders_raw\"}]'::jsonb WHERE id = $1",
    )
    .bind(CONNECTOR)
    .execute(&app.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO connector_run (run_id, connector_id, trigger_kind, status, started_at) \
         VALUES ('run-paused', $1, 'schedule', 'paused', now())",
    )
    .bind(CONNECTOR)
    .execute(&app.pool)
    .await
    .unwrap();
    app.knows_run(
        "run-paused",
        json!({ "ops": { "run_ingest": { "config": { "connector_id": CONNECTOR } } } }),
        "SUCCESS",
    )
    .await;
    app.failing_rule("orders_rows", "bronze.orders_raw").await;
    app.schedule("bronze.orders_raw", ScheduleMode::OnLoad)
        .await;

    let (status, body) = app.run_finished("run-paused", "ingest_job").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    App::settle().await;
    assert_eq!(app.followups_recorded().await, 0);
    assert!(app.incident_assets().await.is_empty());
}

// PR #106 review R1 BLOCKER 3
#[tokio::test]
async fn a_one_table_connector_run_follows_up_only_that_object() {
    let app = app().await;
    sqlx::query(
        "UPDATE connector SET adapter = 'sql', ingest_mode = 'batch', dial = '{}'::jsonb, \
         source_objects = '[{\"name\":\"orders\",\"target\":\"orders_raw\"},\
         {\"name\":\"items\",\"target\":\"items_raw\"}]'::jsonb WHERE id = $1",
    )
    .bind(CONNECTOR)
    .execute(&app.pool)
    .await
    .unwrap();
    app.knows_run(
        "run-one-table",
        json!({ "ops": { "run_ingest": { "config": { "connector_id": CONNECTOR, "object": "orders", "action": "refresh" } } } }),
        "SUCCESS",
    )
    .await;
    for (name, asset) in [
        ("orders_rows", "bronze.orders_raw"),
        ("items_rows", "bronze.items_raw"),
    ] {
        app.failing_rule(name, asset).await;
        app.schedule(asset, ScheduleMode::OnLoad).await;
    }

    let (status, body) = app.run_finished("run-one-table", "ingest_job").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    app.wait_for_incident("bronze.orders_raw").await;
    App::settle().await;
    assert_eq!(app.incident_assets().await, vec!["bronze.orders_raw"]);
}

/// A connector run that failed is refused by the event and follows nothing up.
#[tokio::test]
async fn a_failed_connector_run_triggers_nothing() {
    let app = app().await;
    sqlx::query(
        "UPDATE connector SET adapter = 'sql', ingest_mode = 'batch', dial = '{}'::jsonb, \
         source_objects = '[{\"name\":\"orders\",\"target\":\"orders_raw\"}]'::jsonb WHERE id = $1",
    )
    .bind(CONNECTOR)
    .execute(&app.pool)
    .await
    .unwrap();
    app.knows_run(
        "run-c2",
        json!({ "ops": { "run_ingest": { "config": { "connector_id": CONNECTOR } } } }),
        "FAILURE",
    )
    .await;
    app.failing_rule("orders_rows", "bronze.orders_raw").await;
    app.schedule("bronze.orders_raw", ScheduleMode::OnLoad)
        .await;

    let (status, _) = app.run_finished("run-c2", "ingest_job").await;
    assert_eq!(status, StatusCode::CONFLICT);
    App::settle().await;
    assert!(app.incident_assets().await.is_empty());
    assert_eq!(app.followups_recorded().await, 0);
}

async fn loading_upload(pool: &PgPool, id: &str, run_id: &str, table: &str) {
    uploads::insert(
        pool,
        &NewUpload {
            id,
            original_filename: "stock.csv",
            storage_key: &format!("uploads/{id}.csv"),
            content_type: "text/csv",
            size_bytes: 10,
            sha256: "",
            uploaded_by: "Test User",
            tenant_id: SEED_TENANT.parse().unwrap(),
        },
    )
    .await
    .unwrap();
    uploads::mark_ingesting(
        pool,
        id,
        &json!({ "encoding": "utf-8", "delimiter": ",", "headerRow": 0 }),
        table,
        LoadMode::Replace,
        Some(run_id),
    )
    .await
    .unwrap()
    .unwrap();
}

/// An upload's success was "unknown jobName" before and still answers that;
/// it now also runs the rules of the table it loaded.
#[tokio::test]
async fn an_uploads_success_runs_the_rules_of_its_table_and_answers_as_before() {
    let app = app().await;
    loading_upload(&app.pool, "up-1", "run-u1", "stock_raw").await;
    app.knows_run("run-u1", json!({}), "SUCCESS").await;
    app.failing_rule("stock_rows", "bronze.stock_raw").await;
    app.schedule("bronze.stock_raw", ScheduleMode::OnLoad).await;

    let (status, body) = app.run_finished("run-u1", "file_ingest_job").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({ "matched": 0, "reason": "unknown jobName; no runnable pipeline owns it" })
    );
    app.wait_for_incident("bronze.stock_raw").await;
}

/// An upload run the orchestrator did not confirm as successful is not a load.
#[tokio::test]
async fn an_upload_run_that_did_not_succeed_triggers_nothing() {
    let app = app().await;
    loading_upload(&app.pool, "up-2", "run-u2", "stock_raw").await;
    app.knows_run("run-u2", json!({}), "FAILURE").await;
    app.failing_rule("stock_rows", "bronze.stock_raw").await;
    app.schedule("bronze.stock_raw", ScheduleMode::OnLoad).await;

    app.run_finished("run-u2", "file_ingest_job").await;
    App::settle().await;
    assert!(app.incident_assets().await.is_empty());
    assert_eq!(app.followups_recorded().await, 0);
}

/// A rule that cannot be run never fails the load event: the answer is the
/// one it always was, and the table's other rules still run.
#[tokio::test]
async fn an_unrunnable_rule_does_not_fail_the_event_or_stop_the_others() {
    let app = app().await;
    let id = seed_pipeline(&app.pool, "silver", "orders_clean").await;
    app.knows_run("run-p5", json!({}), "SUCCESS").await;
    app.schedule("silver.orders_clean", ScheduleMode::OnLoad)
        .await;
    app.failing_rule("prose", "silver.orders_clean").await;
    sqlx::query("UPDATE quality_rule SET threshold = '>= 95%' WHERE name = 'prose'")
        .execute(&app.pool)
        .await
        .unwrap();
    app.failing_rule("target_rows", "silver.orders_clean").await;

    let (status, body) = app.run_finished("run-p5", &job_name(&id)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    app.wait_for_incident("silver.orders_clean").await;
}

// PR #106 review R2 SHOULD-FIX 3
#[tokio::test]
async fn an_unknown_dagster_connector_run_fails_closed_and_does_not_silently_skip() {
    let app = app().await;
    Mock::given(method("POST"))
        .and(body_string_contains("runOrError"))
        .and(body_string_contains("unknown-run"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "runOrError": { "__typename": "RunNotFoundError" } }
        })))
        .mount(&app.dagster)
        .await;

    let (status, body) = app.run_finished("unknown-run", "ingest_job").await;
    assert_ne!(
        status,
        StatusCode::OK,
        "an unknown dagster run must not return 200: {body}"
    );
}

// PR #106 review R3 SHOULD-FIX 11
#[tokio::test]
async fn a_connector_sensor_retry_follows_the_load_up_once() {
    let app = app().await;
    sqlx::query(
        "UPDATE connector SET adapter = 'sql', ingest_mode = 'batch', dial = '{}'::jsonb, \
         source_objects = '[{\"name\":\"orders\",\"target\":\"orders_raw\"}]'::jsonb WHERE id = $1",
    )
    .bind(CONNECTOR)
    .execute(&app.pool)
    .await
    .unwrap();
    app.knows_run(
        "run-c-retry",
        json!({ "ops": { "run_ingest": { "config": { "connector_id": CONNECTOR } } } }),
        "SUCCESS",
    )
    .await;
    app.failing_rule("orders_rows", "bronze.orders_raw").await;
    app.schedule("bronze.orders_raw", ScheduleMode::OnLoad)
        .await;

    let (status1, _) = app.run_finished("run-c-retry", "ingest_job").await;
    assert_eq!(status1, StatusCode::OK);
    app.wait_for_incident("bronze.orders_raw").await;

    let (status2, body2) = app.run_finished("run-c-retry", "ingest_job").await;
    assert_eq!(status2, StatusCode::OK, "{body2}");
    assert_eq!(
        body2.get("reason").and_then(Value::as_str),
        Some("run already recorded; sensor retry ignored")
    );

    App::settle().await;
    assert_eq!(app.followups_recorded().await, 1);
    assert_eq!(app.count_checks().await, 1);
    assert_eq!(app.incident_assets().await, vec!["bronze.orders_raw"]);
}

// PR #106 review R3 SHOULD-FIX 11
#[tokio::test]
async fn a_connector_sensor_retry_after_partial_commit_runs_follow_up() {
    let app = app().await;
    sqlx::query(
        "UPDATE connector SET adapter = 'sql', ingest_mode = 'batch', dial = '{}'::jsonb, \
         source_objects = '[{\"name\":\"orders\",\"target\":\"orders_raw\"}]'::jsonb WHERE id = $1",
    )
    .bind(CONNECTOR)
    .execute(&app.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO connector_run (run_id, connector_id, trigger_kind, status, started_at) \
         VALUES ('run-c-partial', $1, 'schedule', 'succeeded', now())",
    )
    .bind(CONNECTOR)
    .execute(&app.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO pipeline_run_event (run_id, pipeline_id, kind) \
         VALUES ('run-c-partial', $1, 'success_alerted')",
    )
    .bind(format!("connector:{CONNECTOR}"))
    .execute(&app.pool)
    .await
    .unwrap();

    app.knows_run(
        "run-c-partial",
        json!({ "ops": { "run_ingest": { "config": { "connector_id": CONNECTOR } } } }),
        "SUCCESS",
    )
    .await;
    app.failing_rule("orders_rows", "bronze.orders_raw").await;
    app.schedule("bronze.orders_raw", ScheduleMode::OnLoad)
        .await;

    let (status, body) = app.run_finished("run-c-partial", "ingest_job").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body.get("reason").and_then(Value::as_str),
        Some("run already recorded; sensor retry ignored"),
        "actual body: {body}"
    );
    app.wait_for_incident("bronze.orders_raw").await;
    App::settle().await;
    assert_eq!(app.followups_recorded().await, 1);
    assert_eq!(app.count_checks().await, 1);
    assert_eq!(app.incident_assets().await, vec!["bronze.orders_raw"]);
}
