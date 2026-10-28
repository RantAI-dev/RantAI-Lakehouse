//! The load history and the learning state (`DATA-14`, finding F5), driven
//! through the real router: every load the run-finished event sees is
//! recorded whatever its schedule, a load of 10% of the usual rows opens a
//! volume incident (`DATA-14-AC2`), and a table with too little history says
//! so. The detection rules themselves are tested beside their code.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::pipelines::{self, CreatePipelineInput};
use lakehouse_store::quality::{self, StatusFilter};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

struct App {
    router: axum::Router,
    pool: PgPool,
    cookie: String,
    _ch: MockServer,
    _dagster: MockServer,
}

/// The stand-in `ClickHouse` says every table holds `rows` rows now.
async fn app(rows: &str) -> App {
    let ch = MockServer::start().await;
    let answer = |data: Value| {
        ResponseTemplate::new(200).set_body_json(json!({
            "meta": [], "rows": data.as_array().map_or(0, Vec::len), "data": data,
        }))
    };
    Mock::given(method("POST"))
        .and(body_string_contains("system.columns"))
        .respond_with(answer(json!([{"name": "id", "type": "String"}])))
        .mount(&ch)
        .await;
    // The load's own row count carries `WHERE 1` (R11).
    Mock::given(method("POST"))
        .and(body_string_contains("WHERE 1 SETTINGS"))
        .respond_with(answer(json!([{"n": rows}])))
        .mount(&ch)
        .await;
    Mock::given(method("POST"))
        .respond_with(answer(json!([])))
        .mount(&ch)
        .await;
    let dagster = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("pipelineRunOrError"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "pipelineRunOrError": {
                "__typename": "Run", "status": "SUCCESS",
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
    App {
        router,
        pool,
        cookie,
        _ch: ch,
        _dagster: dagster,
    }
}

impl App {
    async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let resp = self
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .header("cookie", &self.cookie)
                    .body(Body::from(body.map(|b| b.to_string()).unwrap_or_default()))
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

    /// A pipeline loading `silver.orders_clean`, finished successfully.
    async fn load_orders_clean(&self, run_id: &str) {
        let created = pipelines::create_pipeline(
            &self.pool,
            &CreatePipelineInput {
                name: format!("orders-{run_id}"),
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
                run_as: None,
                target_lag: None,
            },
            None,
        )
        .await
        .unwrap();
        sqlx::query("UPDATE pipeline_definition SET status = 'ready' WHERE id = $1")
            .bind(&created.id)
            .execute(&self.pool)
            .await
            .unwrap();
        let safe: String = created
            .id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let (status, body) = self
            .call(
                "POST",
                "/api/pipelines/events/run-finished",
                Some(json!({ "runId": run_id, "jobName": format!("authored__{safe}") })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    async fn wait_for_loads(&self, asset: &str, count: usize) {
        for _ in 0..100 {
            let loads = quality::recent_loads(&self.pool, asset, 100).await.unwrap();
            if loads.len() >= count {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("{asset} never reached {count} recorded loads");
    }
}

async fn learned_growing_table(pool: &PgPool, asset: &str) {
    for i in 1..=10_i32 {
        sqlx::query(
            "INSERT INTO table_load (asset, loaded_at, rows_after) \
             VALUES ($1, now() - make_interval(days => $2), $3)",
        )
        .bind(asset)
        .bind(20 - 2 * i)
        .bind(i64::from(i) * 1_000)
        .execute(pool)
        .await
        .unwrap();
    }
}

/// A load is recorded with the table's row count whatever the table's
/// schedule (here, none), and a table that has just started is learning.
#[tokio::test]
async fn every_load_is_recorded_and_a_young_table_says_it_is_learning() {
    let app = app("777").await;
    app.load_orders_clean("run-h1").await;
    app.wait_for_loads("silver.orders_clean", 1).await;
    let loads = quality::recent_loads(&app.pool, "silver.orders_clean", 10)
        .await
        .unwrap();
    assert_eq!(loads[0].rows_after, 777);

    let (status, learning) = app
        .call(
            "GET",
            "/api/governance/quality/learning/Silver.Orders_Clean",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{learning}");
    assert_eq!(learning["asset"], "silver.orders_clean");
    assert_eq!(learning["learned"], false);
    assert_eq!(learning["loads"], 1);
    assert_eq!(learning["days"], 0);
    assert_eq!(learning["requiredDays"], 14);
    assert_eq!(learning["requiredLoads"], 5);
    assert_eq!(
        learning["message"],
        "learning: 0 days and 1 load so far; anomalies start after 14 days and 5 loads"
    );
    assert_eq!(
        quality::list(&app.pool, StatusFilter::Open, None)
            .await
            .unwrap(),
        [] as [lakehouse_store::quality::QualityIncident; 0]
    );
}

/// A table never loaded reads "learning: 0 days and 0 loads", and a name that
/// is not a table is refused.
#[tokio::test]
async fn the_learning_state_of_an_unknown_table_and_of_a_bad_name() {
    let app = app("1").await;
    let (status, learning) = app
        .call(
            "GET",
            "/api/governance/quality/learning/silver.never_loaded",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(learning["loads"], 0);
    assert_eq!(learning["firstLoadAt"], Value::Null);
    let (status, _) = app
        .call("GET", "/api/governance/quality/learning/orders", None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// `DATA-14-AC2`, end to end: a load that adds 100 rows to a table that
/// usually gets 1000 opens a volume incident, and the learning state reads
/// "learned".
#[tokio::test]
async fn a_load_of_ten_percent_of_the_usual_rows_opens_a_volume_incident() {
    let app = app("10100").await;
    learned_growing_table(&app.pool, "silver.orders_clean").await;
    app.load_orders_clean("run-h2").await;
    app.wait_for_loads("silver.orders_clean", 11).await;

    let mut incident = None;
    for _ in 0..100 {
        let open = quality::list(&app.pool, StatusFilter::Open, None)
            .await
            .unwrap();
        if let Some(found) = open.into_iter().next() {
            incident = Some(found);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let incident = incident.expect("a volume incident was opened");
    assert_eq!(incident.kind, "volume_anomaly");
    assert_eq!(incident.asset, "silver.orders_clean");
    assert!(
        incident.detail.contains("added 100 rows, 10% of the usual"),
        "{}",
        incident.detail
    );

    let (_, learning) = app
        .call(
            "GET",
            "/api/governance/quality/learning/silver.orders_clean",
            None,
        )
        .await;
    assert_eq!(learning["learned"], true);
    assert_eq!(learning["message"], Value::Null);
}

/// A usual-sized load of a learned table opens nothing.
#[tokio::test]
async fn a_load_of_the_usual_size_opens_nothing() {
    let app = app("11000").await;
    learned_growing_table(&app.pool, "silver.orders_clean").await;
    app.load_orders_clean("run-h3").await;
    app.wait_for_loads("silver.orders_clean", 11).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        quality::list(&app.pool, StatusFilter::Open, None)
            .await
            .unwrap(),
        [] as [lakehouse_store::quality::QualityIncident; 0]
    );
}
