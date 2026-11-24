//! `DATA-6` T2: the API's launch half, against a `wiremock` Dagster and
//! the REAL shared `ClickHouse` container (`gold_publication`'s — its
//! `OnceCell` cache for `console.gold_latest_copy_run` is process-wide, so
//! a second container would let whichever module ran first pin the
//! other's DDL to the wrong server). The plan's fallback for a
//! `wiremock`-unfriendly `CH_URL` is used for exactly that reason; the
//! rows are counted where they were written.
//!
//! The op itself (`refresh_gold_latest_copy`) is exercised by the
//! Python side (`test_gold_latest_copy.py`) and by the
//! `ops/gold_export` gate on the real stack; what these tests prove is
//! the API's own three outcomes: a launch is queued and records
//! nothing, a refusal is recorded as `failed`/`not_started`, and an
//! unreachable orchestrator is too.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;

use lakehouse_api::gold_latest_copy::{self, LatestCopyRequest};
use lakehouse_api::state::AppState;
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const COPY_MICROS: i64 = 1_795_168_800_123_456;

/// An app whose Dagster points at `dagster_url` and whose `ClickHouse`
/// is the binary's shared container.
async fn state_with(dagster_url: String) -> AppState {
    let (ch_url, _container) = crate::gold_publication::shared_clickhouse().await;
    let mut env = HashMap::new();
    env.insert("DAGSTER_URL".to_owned(), dagster_url);
    env.insert("CH_URL".to_owned(), ch_url.clone());
    let (state, _pool) = common::spin_up_state(&env).await;
    state
}

fn request(mart: &str) -> LatestCopyRequest<'_> {
    LatestCopyRequest {
        zone: lakehouse_iceberg::gold::PublishZone::Gold,
        mart,
        table: "gold_export_smoke",
        copy_exported_at_micros: COPY_MICROS,
        rows: 123,
    }
}

async fn recorded_rows(state: &AppState, mart: &str) -> Vec<serde_json::Map<String, Value>> {
    // The happy-path launch records nothing, so nothing has ensured the
    // table yet — the reader ensures it itself, exactly like
    // `latest_copy_state` does in production.
    gold_latest_copy::ensure_gold_latest_copy_run_table(&state.clickhouse)
        .await
        .expect("ensure the latest-copy run table");
    let lit = format!("'{}'", mart.replace('\\', "\\\\").replace('\'', "''"));
    state
        .clickhouse
        .rows(
            &format!(
                "SELECT status, reason, copy_exported_at, rows \
                 FROM console.gold_latest_copy_run WHERE mart = {lit} \
                 ORDER BY started_at FORMAT JSON"
            ),
            None,
        )
        .await
        .expect("read the latest-copy run rows")
}

#[tokio::test]
async fn a_launched_update_is_queued_and_records_nothing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("launchRun"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "launchRun": { "__typename": "LaunchRunSuccess", "run": { "runId": "run-latest-1" } } }
        })))
        .expect(1)
        .mount(&server)
        .await;
    let state = state_with(format!("{}/graphql", server.uri())).await;

    let status =
        gold_latest_copy::launch(&state.dagster, &state.clickhouse, &request("queued_mart")).await;
    assert_eq!(status, "queued");

    // The publish itself succeeded and the orchestrator took the run, so
    // the record table owes no row: the op writes the run's own row.
    let rows = recorded_rows(&state, "queued_mart").await;
    assert!(rows.is_empty(), "no not-started row was expected: {rows:?}");

    // The launch asked for exactly this job with the copy stamp and the
    // row count in its run config (D9: the op needs both to refuse a
    // backwards move and to prove it read the whole copy).
    let requests = server.received_requests().await.unwrap();
    let body = String::from_utf8(requests[0].body.clone()).unwrap();
    assert!(body.contains("gold_latest_copy_job"), "{body}");
    assert!(body.contains("copy_exported_at_micros"), "{body}");
    assert!(body.contains("latest_table"), "{body}");
}

#[tokio::test]
async fn a_refused_launch_is_recorded_as_not_started() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("launchRun"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "launchRun": { "__typename": "PipelineNotFoundError", "message": "no such job" } }
        })))
        .expect(1)
        .mount(&server)
        .await;
    let state = state_with(format!("{}/graphql", server.uri())).await;

    let status =
        gold_latest_copy::launch(&state.dagster, &state.clickhouse, &request("refused_mart")).await;
    assert_eq!(status, "not_started");

    // Exactly one row, recording the launch that did not happen — with
    // the publish's own stamp and row count (they are facts of the
    // publish that asked, not of a run that never started).
    let rows = recorded_rows(&state, "refused_mart").await;
    assert_eq!(rows.len(), 1, "exactly one not-started row: {rows:?}");
    assert_eq!(rows[0]["status"], "failed");
    assert_eq!(rows[0]["reason"], "not_started");
    assert_eq!(rows[0]["copy_exported_at"], "2026-11-20 10:00:00.123456");
    assert_eq!(rows[0]["rows"], 123);
}

#[tokio::test]
async fn an_unreachable_orchestrator_is_recorded_as_not_started() {
    // The harness's own dead-upstream address, same as
    // `pipeline_lag.rs`'s unreachable-Dagster tests.
    let state = state_with("http://127.0.0.1:1/graphql".to_owned()).await;

    let status = gold_latest_copy::launch(
        &state.dagster,
        &state.clickhouse,
        &request("unreachable_mart"),
    )
    .await;
    assert_eq!(status, "not_started");

    let rows = recorded_rows(&state, "unreachable_mart").await;
    assert_eq!(rows.len(), 1, "exactly one not-started row: {rows:?}");
    assert_eq!(rows[0]["status"], "failed");
    assert_eq!(rows[0]["reason"], "not_started");
}
