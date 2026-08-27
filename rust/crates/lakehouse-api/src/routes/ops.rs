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
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::dagster::{DgClient, DgError};
use crate::json::ApiJson;
use crate::routes::support::{js_error, num_or_zero, str_col};
use crate::state::AppState;

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
                "id": "dispar-dki",
                "name": "Dinas Pariwisata & Ekraf DKI Jakarta",
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
                "tenant": "dispar-dki",
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
                "site": "Depok (187)",
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
    crate::dagster::iso_from_unix_seconds(OffsetDateTime::now_utc().unix_timestamp() as f64)
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
}
