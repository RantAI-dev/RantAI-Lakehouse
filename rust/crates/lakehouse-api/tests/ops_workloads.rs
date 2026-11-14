//! `GET /api/ops/workloads` answers on a real `ClickHouse` (OPS-19).
//!
//! The route's statement named its computed column `elapsed`, which
//! shadows `system.processes.elapsed`: the engine then multiplied the
//! alias's `String` by a `UInt16` and refused the statement (error 43),
//! and every `OpsError` became a 503 — so the assistant's
//! `list_workloads` tool could not answer "which queries are running".
//! Nothing ran this statement against a real engine, so the defect
//! shipped.
//!
//! These tests run it against a real `ClickHouse` 26.8 — the compose
//! file's own tag, through the same `testcontainers` arrangement
//! `tests/gold_publication.rs` uses — and assert two halves: the route
//! answers 200 with the documented fields, and the engine's own record
//! of the statement it ran (`system.query_log`) carries no exception.
//! The second half is what names the cause when the first one fails: the
//! route's 503 body is a fixed sentence with a reference id (SEC-11
//! keeps the engine's text out of a response), so the engine's log is
//! where "Code: 43" is visible.
//!
//! The container is stopped when the test ends. The crate's other
//! harnesses keep theirs running for the life of the test binary; this
//! one does not leak.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_clickhouse::{ChClient, QueryOptions};
use serde_json::{Map, Value, json};
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::clickhouse::ClickHouse;
use tower::ServiceExt;

use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};

/// A disposable `ClickHouse` 26.8 — the compose file's own tag.
struct Engine {
    /// The address the API is pointed at (`CH_URL`).
    url: String,
    /// Kept alive for the test: dropping it removes the container.
    container: testcontainers::ContainerAsync<ClickHouse>,
}

impl Engine {
    async fn start() -> Self {
        // The compose service's env trio (the compose file's own
        // local-dev default); all three are required, see
        // `tests/gold_publication.rs`.
        let container = ClickHouse::default()
            .with_env_var("CLICKHOUSE_USER", "default")
            .with_env_var("CLICKHOUSE_PASSWORD", "")
            .with_env_var("CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT", "1")
            .with_tag("26.8")
            .start()
            .await
            .expect("start the ClickHouse testcontainer");
        let host = container.get_host().await.expect("container host");
        let port = container
            .get_host_port_ipv4(8123)
            .await
            .expect("container HTTP port");
        Self {
            url: format!("http://{host}:{port}"),
            container,
        }
    }

    /// A client for this engine, for the test's own reads.
    fn client(&self) -> ChClient {
        ChClient::new(self.url.clone(), "default".to_owned(), String::new())
    }

    /// The API under test, pointed at this engine.
    async fn app(&self) -> TestApp {
        let mut overrides = HashMap::new();
        // `CH_URL`, not `CLICKHOUSE_URL`: the config reads this key, so a
        // test that sets the other one silently dials the default
        // `http://localhost:18123` and proves nothing.
        overrides.insert("CH_URL".to_owned(), self.url.clone());
        spin_up_with_env(&overrides).await
    }

    /// Stops the container; dropping it removes it.
    async fn stop(self) {
        self.container
            .stop()
            .await
            .expect("stop the ClickHouse testcontainer");
    }
}

/// A query that stays open, so `system.processes` holds a row that is
/// not the route's own statement: the route's answer must not depend on
/// the self-row its own `query` alias leaves behind.
///
/// `sleepEachRow` sleeps per row and the engine caps a block's total
/// sleep at three seconds, so the block size is pinned to one row: the
/// query then stays open for as long as the caller keeps it open.
const HELD_QUERY: &str = "SELECT sleepEachRow(2) FROM numbers(1000) SETTINGS max_block_size = 1";

/// The id the held query runs under, so the wait below can find it
/// without matching on the statement's text (a read that named the text
/// would match itself).
const HELD_QUERY_ID: &str = "ops19-held-query";

/// Starts the held query and waits until the engine lists it.
async fn hold_a_query_open(engine: &Engine) -> tokio::task::JoinHandle<()> {
    let client = engine.client();
    let handle = tokio::spawn(async move {
        let options = QueryOptions {
            query_id: Some(HELD_QUERY_ID.to_owned()),
            ..QueryOptions::default()
        };
        // The result is never read: the query is held open so the engine
        // lists a running process for the route to report.
        let _ = client.query_with_options(HELD_QUERY, &options, None).await;
    });
    for _ in 0..100 {
        let rows = engine
            .client()
            .rows(
                // The id is spelled out here, not interpolated: the wait
                // below only succeeds when this literal and
                // `HELD_QUERY_ID` are the same, so the test proves the
                // two agree instead of assuming it.
                "SELECT count() AS n FROM system.processes WHERE query_id = 'ops19-held-query'",
                None,
            )
            .await
            .expect("count the running queries");
        let running = rows
            .first()
            .and_then(|row| row.get("n"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if running > 0 {
            return handle;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the held query never showed up in system.processes");
}

/// The engine's own record of the statements that read
/// `system.processes`: the newest rows of `system.query_log` whose
/// statement is one of them.
///
/// The route's 503 body is a fixed sentence with a reference id (SEC-11
/// keeps the engine's text out of a response), so the engine's log is
/// the only place where the cause of a refusal is visible.
async fn engine_records_of_the_route_statements(engine: &Engine) -> Vec<Map<String, Value>> {
    let client = engine.client();
    // The query log is flushed every few seconds; the flush below makes
    // the statement the route just ran visible at once.
    client
        .exec("SYSTEM FLUSH LOGS", None)
        .await
        .expect("flush the engine's logs");
    client
        .rows(
            "SELECT type, exception_code, exception FROM system.query_log \
             WHERE query LIKE '%system.processes%' \
               AND query NOT LIKE '%query_log%' \
               AND query NOT LIKE '%ops19-held-query%' \
             ORDER BY event_time_microseconds DESC LIMIT 5",
            None,
        )
        .await
        .expect("read the engine's query log")
}

/// The rows of the engine's record that are a refusal, with the engine's
/// own text — the only place the cause of a 503 is visible.
fn refusals(records: &[Map<String, Value>]) -> Vec<&Map<String, Value>> {
    records
        .iter()
        .filter(|record| record.get("exception_code") != Some(&json!(0)))
        .collect()
}

/// Sends one request to the API under test and returns its status and
/// body.
async fn send(router: &axum::Router, method: &str, uri: &str, cookie: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("cookie", cookie)
        .body(Body::empty())
        .expect("build request");
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("in-memory request");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, body)
}

/// The fields `GET /api/ops/workloads` documents for one row, and no
/// others: the JSON the route answers does not change.
const DOCUMENTED_FIELDS: [&str; 9] = [
    "id",
    "principal",
    "tenant",
    "class",
    "engine",
    "status",
    "elapsedMs",
    "estimatedCost",
    "startedAt",
];

/// Asserts that one row of the answer carries the documented fields with
/// the documented shape.
fn assert_a_documented_row(row: &Value) {
    let object = row.as_object().expect("a workload row is an object");
    let mut names: Vec<&str> = object.keys().map(String::as_str).collect();
    names.sort_unstable();
    let mut documented = DOCUMENTED_FIELDS.to_vec();
    documented.sort_unstable();
    assert_eq!(
        names, documented,
        "the row must carry exactly the documented fields"
    );

    let id = object["id"].as_str().expect("id is a string");
    assert!(
        id.starts_with("w-") && id.len() > 2,
        "the id is positional: {id}"
    );
    assert!(
        !object["principal"].as_str().unwrap_or_default().is_empty(),
        "the principal is the engine's own user"
    );
    assert_eq!(
        object["tenant"].as_str(),
        Some(lakehouse_api::tenant::TENANT_ID.as_str()),
        "the tenant is this deployment's own"
    );
    // No workload classifier exists: null, never an invented label.
    assert!(object["class"].is_null(), "class is null");
    // Every row comes from `system.processes`, which IS `ClickHouse`.
    assert_eq!(object["engine"].as_str(), Some("hot-store"));
    assert_eq!(object["status"].as_str(), Some("running"));
    assert!(
        object["elapsedMs"].is_i64() || object["elapsedMs"].is_u64(),
        "elapsedMs is a whole number of milliseconds"
    );
    // No cost model exists: null, never a fabricated amount.
    assert!(object["estimatedCost"].is_null(), "estimatedCost is null");
    let started_at = object["startedAt"].as_str().expect("startedAt is a string");
    chrono::DateTime::parse_from_rfc3339(started_at)
        .expect("startedAt is an RFC 3339 timestamp the engine's own clock wrote");
}

/// The route answers 200 with the documented fields, and the engine
/// accepted the statement it ran.
#[tokio::test]
async fn the_workloads_route_answers_200_with_the_documented_fields() {
    let engine = Engine::start().await;
    let app = engine.app().await;
    // `ADM-13` D5: reading the route takes `ops:read`, so the caller holds
    // exactly that and nothing else — the fields, not any other permission,
    // are what this test is about.
    let user = create_principal_with_permissions(&app.pool, "ops:read").await;
    let cookie = session_cookie_for_user(&app.pool, user).await;
    let _held = hold_a_query_open(&engine).await;

    let (status, body) = send(&app.router, "GET", "/api/ops/workloads", &cookie).await;

    // The engine's own record of the statement the route ran.
    let records = engine_records_of_the_route_statements(&engine).await;
    assert!(
        !records.is_empty(),
        "the engine recorded no statement that reads system.processes"
    );
    let refused = refusals(&records);
    assert!(
        refused.is_empty(),
        "the engine refused a statement that reads system.processes: {refused:?}"
    );

    assert_eq!(status, StatusCode::OK, "the route must answer 200: {body}");
    let workloads = body["workloads"]
        .as_array()
        .expect("the answer holds a workloads array");
    assert!(
        !workloads.is_empty(),
        "the engine was running a query, so the answer holds a row"
    );
    for row in workloads {
        assert_a_documented_row(row);
    }
    // The held query has been running for seconds, so at least one row
    // carries a real elapsed time: a reader that lost the column would
    // report 0 for every row.
    assert!(
        workloads
            .iter()
            .any(|row| row["elapsedMs"].as_i64().is_some_and(|ms| ms > 0)),
        "at least one row carries the engine's own elapsed time: {workloads:?}"
    );

    engine.stop().await;
}

/// The cancel route reads the same `system.processes` statement, so it
/// must not answer 503 either: an id that is out of range answers 404,
/// which is only reachable when the engine accepted the statement.
#[tokio::test]
async fn the_cancel_route_reads_the_same_processes_statement() {
    let engine = Engine::start().await;
    let app = engine.app().await;
    // `workload:cancel` is the permission the cancel route asks for.
    let user = create_principal_with_permissions(&app.pool, "workload:cancel").await;
    let cookie = session_cookie_for_user(&app.pool, user).await;

    // `w-999` is out of range: the statement reads at most 50 rows, so
    // the route answers "not found" and never issues a `KILL QUERY`.
    let (status, body) = send(
        &app.router,
        "POST",
        "/api/ops/workloads/w-999/cancel",
        &cookie,
    )
    .await;

    // The engine's own record of the statement the route ran.
    let records = engine_records_of_the_route_statements(&engine).await;
    assert!(
        !records.is_empty(),
        "the engine recorded no statement that reads system.processes"
    );
    let refused = refusals(&records);
    assert!(
        refused.is_empty(),
        "the engine refused a statement that reads system.processes: {refused:?}"
    );

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "an out-of-range workload is not found: {body}"
    );

    engine.stop().await;
}
