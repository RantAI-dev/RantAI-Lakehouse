//! `GET /api/overview`, `POST /api/overview` — the lakehouse-wide summary
//! and recent-activity feed.
//!
//! Ports `src/app/api/overview/route.ts`. Both handlers combine
//! `ClickHouse` aggregates with `Dagster` run history (`POST` is
//! read-only despite the verb — it only lists recent runs).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lakehouse_clickhouse::{ChClient, ChError};
use serde_json::{Value, json};

use crate::json::ApiJson;
use crate::routes::support::{js_error, num_or_zero, str_col};
use crate::state::AppState;
use lakehouse_dagster::{DgClient, DgError, iso_from_unix_seconds, map_run_status};

/// Errors surfaced while building the overview: either `ClickHouse` or
/// `Dagster` can fail, and — matching the TypeScript's single `try/catch`
/// around both — either failure produces the same 503 body.
#[derive(Debug, thiserror::Error)]
enum OverviewError {
    /// A `ClickHouse` query failed.
    #[error("{0}")]
    ClickHouse(#[from] ChError),
    /// A `Dagster` call failed.
    #[error("{0}")]
    Dagster(#[from] DgError),
}

/// `GET /api/overview` — aggregate counts across catalog, storage,
/// queries, and pipelines.
pub async fn get(State(state): State<AppState>) -> Response {
    match get_body(&state.clickhouse, &state.dagster).await {
        Ok(body) => (StatusCode::OK, ApiJson(body)).into_response(),
        // `catch (e) { return NextResponse.json({ error: String(e) }, {
        // status: 503 }); }` in `overview/route.ts` GET.
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "error": js_error(err) })),
        )
            .into_response(),
    }
}

async fn get_body(ch: &ChClient, dagster: &DgClient) -> Result<Value, OverviewError> {
    let assets_row = ch
        .rows(
            "SELECT toString(count()) n, toString(countIf(coalesce(s.total,0)=0)) stale FROM (
         SELECT slug FROM lake.`bronze_meta.dataset_catalog`
         UNION ALL SELECT slug FROM lake.`bronze_meta_sec.dataset_catalog`) c
       LEFT JOIN (SELECT slug,total FROM lake.`bronze_meta.dataset_sync`
                  UNION ALL SELECT slug,total FROM lake.`bronze_meta_sec.dataset_sync`) s ON c.slug=s.slug",
            None,
        )
        .await?;
    let hot_row = ch
        .rows(
            "SELECT toString(sum(bytes_on_disk)) bytes, toString(uniqExact(table)) assets FROM system.parts WHERE database='serving' AND active",
            None,
        )
        .await?;
    let warm_row = ch
        .rows(
            "SELECT toString(sum(total)) rows, toString(count()) assets FROM (SELECT total FROM lake.`bronze_meta.dataset_sync` UNION ALL SELECT total FROM lake.`bronze_meta_sec.dataset_sync`)",
            None,
        )
        .await?;
    let q_row = ch
        .rows(
            "SELECT toString(count()) vol, toString(round(quantile(0.95)(query_duration_ms))) p95,
              toString(round(countIf(exception!='')/greatest(count(),1),4)) err, toString(sum(read_bytes)) scan
       FROM system.query_log WHERE type='QueryFinish' AND event_time > now() - INTERVAL 24 HOUR",
            None,
        )
        .await?;

    let runs = dagster.list_runs(100).await?;
    // `jobs` is fetched but never read in the TypeScript response — kept
    // here only so a `Dagster` outage on this call still 503s like the
    // original, matching its (accidental) error-propagation behavior.
    let _jobs = dagster.list_jobs().await?;

    let now_ms = now_unix_millis();
    let recent: Vec<&lakehouse_dagster::DgRun> = runs
        .iter()
        .filter(|r| r.start_time.unwrap_or(0.0) * 1000.0 > now_ms - 864e5)
        .collect();
    let failed = recent.iter().filter(|r| r.status == "FAILURE").count();
    let active = recent
        .iter()
        .filter(|r| matches!(r.status.as_str(), "STARTED" | "STARTING" | "QUEUED"))
        .count();

    let assets_row = assets_row.first();
    let hot_row = hot_row.first();
    let warm_row = warm_row.first();
    let q_row = q_row.first();
    let warm_rows = num_or_zero(warm_row, "rows");

    Ok(json!({
        "assetsTotal": num_or_zero(assets_row, "n"),
        "staleAssets": num_or_zero(assets_row, "stale"),
        "assetsByTier": {
            "hot": { "count": num_or_zero(hot_row, "assets"), "bytes": num_or_zero(hot_row, "bytes") },
            "warm": { "count": num_or_zero(warm_row, "assets"), "bytes": warm_rows * 220 },
            "cold": { "count": 0, "bytes": 0 },
            "ai": { "count": 0, "bytes": 0 },
        },
        "pipelines": { "active": active, "failed": failed, "delayed": 0 },
        "streaming": { "jobs": 0, "maxLagSeconds": 0, "unhealthy": 0 },
        "queries": {
            "volume24h": num_or_zero(q_row, "vol"),
            "p95Ms": num_or_zero(q_row, "p95"),
            "failureRate": q_row.and_then(|r| str_col(r, "err").parse::<f64>().ok()).unwrap_or(0.0),
            "cacheAssistRate": 0,
            "scannedBytes24h": num_or_zero(q_row, "scan"),
        },
        "policyViolations7d": 0,
        "pendingApprovals": 0,
        "agents": { "activeRuns": 0, "budgetUsedRate": 0 },
        "services": { "healthy": 4, "degraded": 0, "unhealthy": 0 },
        "incidents": [],
    }))
}

/// `POST /api/overview` — recent activity, sourced entirely from `Dagster`
/// run history. Despite the verb this reads only; there is no request
/// body and nothing is mutated.
pub async fn refresh(State(state): State<AppState>) -> Response {
    match state.dagster.list_runs(20).await {
        Ok(runs) => {
            let activity: Vec<Value> = runs
                .iter()
                .map(|r| {
                    json!({
                        "id": r.run_id,
                        "at": r.start_time.map_or_else(String::new, iso_from_unix_seconds),
                        "actor": "Dagster",
                        "actorKind": "service",
                        "action": format!("pipeline {}", map_run_status(&r.status)),
                        "target": r.job_name,
                        "category": "pipeline",
                    })
                })
                .collect();
            (StatusCode::OK, ApiJson(json!({ "activity": activity }))).into_response()
        }
        // `catch (e) { return NextResponse.json({ activity: [], error:
        // String(e) }, { status: 503 }); }` in `overview/route.ts` POST.
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "activity": [], "error": js_error(err) })),
        )
            .into_response(),
    }
}

/// `Date.now()` — current Unix time in milliseconds, as an `f64` so it can
/// be compared against `startTime * 1000` without overflow concerns.
#[allow(
    clippy::cast_precision_loss,
    reason = "millisecond-precision comparison against a 24h window; \
              precision loss at this magnitude is inconsequential"
)]
fn now_unix_millis() -> f64 {
    time::OffsetDateTime::now_utc().unix_timestamp_nanos() as f64 / 1_000_000.0
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn now_unix_millis_is_plausible() {
        // Sanity bound: some time after this file was written, well before
        // any realistic clock error.
        assert!(now_unix_millis() > 1_700_000_000_000.0);
    }
}
