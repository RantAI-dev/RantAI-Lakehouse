//! `GET /api/ops/{kind}` — operational views: `observability`, `usage`,
//! `workloads`, `services`.
//!
//! Ports `src/app/api/ops/[kind]/route.ts`. An unrecognized `kind` returns
//! HTTP 400 with `{"error": "kind tak dikenal: <kind>"}`, verified against
//! `ops-unknown-kind.json` in the parity corpus.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ident::SqlLiteral;
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::json::ApiJson;
use crate::routes::support::{js_error, num_or_zero, str_col};
use crate::state::AppState;
use crate::tenant::{TENANT_ID, TENANT_OWNER, TENANT_SITE};
use lakehouse_dagster::{DgClient, DgError};

/// The four recognized `ops/{kind}` values. Ported from the `if (kind ===
/// ...)` chain in `ops/[kind]/route.ts`; anything else is [`Kind::Unknown`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `ops/observability` — query SLOs.
    Observability,
    /// `ops/usage` — 7-day compute/storage usage.
    Usage,
    /// `ops/workloads` — currently running `ClickHouse` processes.
    Workloads,
    /// `ops/services` — component health checks.
    Services,
    /// Anything else, which the TypeScript rejects with HTTP 400.
    Unknown,
}

impl Kind {
    fn parse(kind: &str) -> Self {
        match kind {
            "observability" => Self::Observability,
            "usage" => Self::Usage,
            "workloads" => Self::Workloads,
            "services" => Self::Services,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum OpsError {
    #[error("{0}")]
    ClickHouse(#[from] ChError),
    #[error("{0}")]
    Dagster(#[from] DgError),
}

/// `GET /api/ops/{kind}`.
pub async fn get(State(state): State<AppState>, Path(kind): Path<String>) -> Response {
    match Kind::parse(&kind) {
        Kind::Unknown => (
            StatusCode::BAD_REQUEST,
            ApiJson(json!({ "error": format!("kind tak dikenal: {kind}") })),
        )
            .into_response(),
        parsed => match run(&state, parsed).await {
            Ok(body) => (StatusCode::OK, ApiJson(body)).into_response(),
            // Every branch shares one outer `catch (e)` returning
            // `{ error: String(e) }` at 503.
            Err(err) => (
                StatusCode::SERVICE_UNAVAILABLE,
                ApiJson(json!({ "error": js_error(err) })),
            )
                .into_response(),
        },
    }
}

async fn run(state: &AppState, kind: Kind) -> Result<Value, OpsError> {
    match kind {
        Kind::Observability => observability(&state.clickhouse).await,
        Kind::Usage => usage(&state.clickhouse, &state.dagster).await,
        Kind::Workloads => workloads(&state.clickhouse).await,
        Kind::Services => services(&state.clickhouse, &state.dagster).await,
        Kind::Unknown => unreachable!("Kind::Unknown is handled before `run` is called"),
    }
}

async fn observability(ch: &ChClient) -> Result<Value, OpsError> {
    let rows = ch
        .rows(
            "SELECT toString(round(quantile(0.95)(query_duration_ms))) p95,
                  toString(round(countIf(exception != '') / greatest(count(),1), 4)) err,
                  toString(count()) n
           FROM system.query_log
           WHERE type='QueryFinish' AND event_time > now() - INTERVAL 24 HOUR",
            None,
        )
        .await?;
    let q = rows.first();
    let p95 = num_or_zero(q, "p95");
    let err = q
        .and_then(|r| str_col(r, "err").parse::<f64>().ok())
        .unwrap_or(0.0);
    Ok(json!({
        "queryP95Ms": p95,
        "queryErrorRate": err,
        "ingestLagSeconds": 0,
        "streamingLagSeconds": 0,
        "cacheHitRate": 0,
        "policyDecisionP95Ms": 0,
        "agentSuccessRate": 0,
        "activeIncidents": 0,
        "slos": [
            {
                "name": "Query p95 < 2s",
                "target": "2000ms",
                "current": format!("{p95}ms"),
                "ok": p95 < 2000,
            },
            {
                "name": "Query error rate < 1%",
                "target": "1%",
                "current": format!("{:.2}%", err * 100.0),
                "ok": err < 0.01,
            },
        ],
    }))
}

async fn usage(ch: &ChClient, dagster: &DgClient) -> Result<Value, OpsError> {
    let u_rows = ch
        .rows(
            "SELECT toString(count()) units, toString(sum(read_bytes)) bytes
           FROM system.query_log WHERE type='QueryFinish' AND event_time > now() - INTERVAL 7 DAY",
            None,
        )
        .await?;
    let u = u_rows.first();
    let units = num_or_zero(u, "units");
    let bytes = num_or_zero(u, "bytes");

    let now_ms = now_unix_millis();
    let runs7d = dagster
        .list_runs(200)
        .await?
        .into_iter()
        .filter(|r| r.start_time.unwrap_or(0.0) * 1000.0 > now_ms - 7.0 * 864e5)
        .count();

    let store_rows = ch
        .rows(
            "SELECT toString(sum(bytes_on_disk)) hot,
                  (SELECT toString(sum(total)) FROM (SELECT total FROM lake.`bronze_meta.dataset_sync` UNION ALL SELECT total FROM lake.`bronze_meta_sec.dataset_sync`)) warmRows
           FROM system.parts WHERE database='serving' AND active",
            None,
        )
        .await?;
    let store = store_rows.first();
    let hot = num_or_zero(store, "hot");
    let warm_rows = num_or_zero(store, "warmRows");

    Ok(json!({
        "computeUnits7d": units,
        "scannedBytes7d": bytes,
        "storageByTier": {
            "hot": hot,
            "warm": warm_rows * 220,
            "cold": 0,
            "ai": 0,
        },
        "pipelineRuns7d": runs7d,
        "agentBudgetUsedRate": 0,
        "tenants": [
            {
                "id": TENANT_ID.as_str(),
                "name": TENANT_OWNER.as_str(),
                "computeUnits": units,
                "budgetLimit": 100_000,
                "budgetSpent": units,
            },
        ],
    }))
}

async fn workloads(ch: &ChClient) -> Result<Value, OpsError> {
    let procs = ch
        .rows(
            "SELECT user, toString(elapsed) elapsed, substring(query,1,80) query
         FROM system.processes WHERE query NOT LIKE '%system.processes%' LIMIT 50",
            None,
        )
        .await?;
    let started_at = now_iso();
    let workloads: Vec<Value> = procs
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let elapsed_secs = str_col(p, "elapsed").parse::<f64>().unwrap_or(0.0);
            json!({
                "id": format!("w-{i}"),
                "principal": str_col(p, "user"),
                "tenant": TENANT_ID.as_str(),
                "class": "hot-analytics",
                "engine": "hot-store",
                "status": "running",
                "elapsedMs": elapsed_ms(elapsed_secs),
                "estimatedCost": 1,
                "startedAt": started_at,
            })
        })
        .collect();
    Ok(json!({ "workloads": workloads }))
}

async fn services(ch: &ChClient, dagster: &DgClient) -> Result<Value, OpsError> {
    let ch_ok = ch.rows("SELECT 1", None).await.is_ok();
    let dag_ok = dagster.is_alive().await;
    let entries = [
        (
            "clickhouse",
            "ClickHouse (Hot analytical store)",
            ch_ok,
            vec![],
        ),
        (
            "dagster",
            "Dagster (Orchestration)",
            dag_ok,
            vec!["clickhouse"],
        ),
        (
            "iceberg",
            "Iceberg + Lakekeeper (Open tables)",
            ch_ok,
            vec!["rustfs"],
        ),
        ("rustfs", "RustFS (Object storage)", true, vec![]),
    ];
    let services: Vec<Value> = entries
        .into_iter()
        .map(|(id, name, ok, deps)| {
            json!({
                "id": id,
                "name": name,
                "health": if ok { "healthy" } else { "unhealthy" },
                "version": "-",
                "site": TENANT_SITE.as_str(),
                "replicas": 1,
                "errorRate": 0,
                "latencyMs": 0,
                "dependencies": deps,
            })
        })
        .collect();
    Ok(json!({ "services": services }))
}

/// `Math.round((Number(p.elapsed) || 0) * 1000)`.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "elapsed query time in milliseconds; always non-negative and \
              far below i64::MAX in practice"
)]
fn elapsed_ms(elapsed_secs: f64) -> i64 {
    (elapsed_secs * 1000.0).round() as i64
}

/// `Date.now()` in milliseconds.
#[allow(
    clippy::cast_precision_loss,
    reason = "millisecond-precision comparison against a 7-day window; \
              precision loss at this magnitude is inconsequential"
)]
fn now_unix_millis() -> f64 {
    OffsetDateTime::now_utc().unix_timestamp_nanos() as f64 / 1_000_000.0
}

/// `new Date().toISOString()`.
#[allow(
    clippy::cast_precision_loss,
    reason = "second-precision input to a millisecond-precision formatter"
)]
fn now_iso() -> String {
    lakehouse_dagster::iso_from_unix_seconds(OffsetDateTime::now_utc().unix_timestamp() as f64)
}

// ── cancelWorkload (Task 2.6) ───────────────────────────────────────────
//
// `GET /api/ops/workloads` (above) mints purely positional ids ("w-0",
// "w-1", ...) from a fresh `system.processes` scan every request — the
// parity corpus (`ops-workloads.json`) locks that exact response shape, so
// the real `query_id` `system.processes` actually has cannot be added to
// it without breaking parity. `cancel_workload` below re-derives it
// server-side instead: it re-runs (a superset of) the same query, walks to
// the same positional index, and only then knows the real `query_id` to
// target — which is inherently racy against a process list that can
// change between the two requests (the list a user is looking at may have
// already scrolled by the time they click "cancel"), same tradeoff any
// index-addressed live list has. That `query_id` is used only to build the
// `KILL QUERY` statement server-side; it is never serialized back to the
// client.

/// Build `KILL QUERY WHERE query_id = '<escaped>'` for `query_id`.
/// Extracted from [`cancel_workload_body`] purely so it can be unit tested
/// (string construction only) without ever touching `ClickHouse` — see the
/// module-level safety note in the Task 2.6 brief: no `KILL QUERY` may run
/// against the live cluster during this work.
fn kill_query_sql(query_id: &str) -> String {
    format!("KILL QUERY WHERE query_id = {}", SqlLiteral::from(query_id))
}

/// `POST /api/ops/workloads/{id}/cancel` — kill the `ClickHouse` query
/// backing workload `id` (a `"w-<index>"` id from `GET /api/ops/workloads`).
///
/// # Errors
///
/// This is a `Response`-returning handler (not `ApiResult`), matching the
/// rest of this file: 404 if `id` doesn't parse as `"w-<n>"` or `n` is out
/// of range against the current process list (including the ordinary case
/// where the workload already finished); 503 on a `ClickHouse` failure.
pub async fn cancel_workload(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match cancel_workload_body(&state.clickhouse, &id).await {
        Ok(Some(body)) => (StatusCode::OK, ApiJson(body)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            ApiJson(json!({ "error": format!("Workload {id} not found") })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "error": js_error(err) })),
        )
            .into_response(),
    }
}

async fn cancel_workload_body(ch: &ChClient, id: &str) -> Result<Option<Value>, ChError> {
    let Some(index) = id.strip_prefix("w-").and_then(|s| s.parse::<usize>().ok()) else {
        return Ok(None);
    };
    let procs = ch
        .rows(
            "SELECT query_id, user, toString(elapsed) elapsed, substring(query,1,80) query
             FROM system.processes WHERE query NOT LIKE '%system.processes%' LIMIT 50",
            None,
        )
        .await?;
    let Some(row) = procs.get(index) else {
        return Ok(None);
    };
    let query_id = str_col(row, "query_id");
    if query_id.is_empty() {
        return Ok(None);
    }
    ch.exec(&kill_query_sql(query_id), None).await?;
    let elapsed_secs = str_col(row, "elapsed").parse::<f64>().unwrap_or(0.0);
    Ok(Some(json!({
        "id": id,
        "principal": str_col(row, "user"),
        "tenant": TENANT_ID.as_str(),
        "class": "hot-analytics",
        "engine": "hot-store",
        "status": "cancelled",
        "elapsedMs": elapsed_ms(elapsed_secs),
        "estimatedCost": 1,
        "startedAt": now_iso(),
    })))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn kind_parses_known_values() {
        assert_eq!(Kind::parse("observability"), Kind::Observability);
        assert_eq!(Kind::parse("usage"), Kind::Usage);
        assert_eq!(Kind::parse("workloads"), Kind::Workloads);
        assert_eq!(Kind::parse("services"), Kind::Services);
    }

    #[test]
    fn kind_parse_unknown_falls_back() {
        assert_eq!(Kind::parse("bogus-kind"), Kind::Unknown);
        assert_eq!(Kind::parse(""), Kind::Unknown);
        assert_eq!(Kind::parse("lineage"), Kind::Unknown);
    }

    #[test]
    fn now_iso_looks_like_an_iso_timestamp() {
        let s = now_iso();
        assert!(s.ends_with('Z'));
        assert_eq!(s.len(), "2026-08-27T04:00:10.075Z".len());
    }

    // ── cancelWorkload (Task 2.6) ────────────────────────────────────────
    //
    // Pure string-construction/parsing checks only, per the CRITICAL
    // SAFETY constraint: nothing here talks to a real (or mocked)
    // ClickHouse, so a `KILL QUERY` is never issued anywhere in this test
    // suite. Live verification is limited to the id-out-of-range 404
    // path, which returns before `ch.exec` is ever reached — see
    // `cancel_workload_body`.

    #[test]
    fn kill_query_sql_targets_the_exact_query_id() {
        assert_eq!(
            kill_query_sql("abcd-1234"),
            "KILL QUERY WHERE query_id = 'abcd-1234'"
        );
    }

    #[test]
    fn kill_query_sql_escapes_single_quotes() {
        // query_id is server-derived (ClickHouse's own UUID-shaped id), so
        // this is defense in depth rather than a realistic input, but the
        // same escaping discipline as `lineage_body`'s `SqlLiteral` usage
        // applies here.
        assert_eq!(
            kill_query_sql("o'brien"),
            "KILL QUERY WHERE query_id = 'o''brien'"
        );
    }

    #[test]
    fn workload_id_without_w_prefix_does_not_parse() {
        assert!("not-w-0".strip_prefix("w-").is_none());
    }

    #[test]
    fn workload_id_index_parses_from_w_prefix() {
        let id = "w-3";
        let index = id.strip_prefix("w-").and_then(|s| s.parse::<usize>().ok());
        assert_eq!(index, Some(3));
    }

    #[test]
    fn workload_id_non_numeric_suffix_does_not_parse() {
        let id = "w-abc";
        let index = id.strip_prefix("w-").and_then(|s| s.parse::<usize>().ok());
        assert_eq!(index, None);
    }
}
