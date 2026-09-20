//! `GET /api/overview`, `POST /api/overview` — the lakehouse-wide summary
//! and recent-activity feed.
//!
//! Ports `src/app/api/overview/route.ts`. Both handlers combine
//! `ClickHouse` aggregates with `Dagster` run history (`POST` is
//! read-only despite the verb — it only lists recent runs).

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lakehouse_clickhouse::ChError;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::overview::{self, AlertItem};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::support::{js_error, num_or_zero, str_col};
use crate::state::AppState;
use lakehouse_dagster::{iso_from_unix_seconds, map_run_status};

/// The overview is built from `ClickHouse`; only that failing makes the
/// page unavailable. Postgres and `Dagster` contribute parts of it and are
/// allowed to be down — their sections then report what is known instead of
/// taking the whole page with them.
#[derive(Debug, thiserror::Error)]
enum OverviewError {
    /// A `ClickHouse` query failed.
    #[error("{0}")]
    ClickHouse(#[from] ChError),
}

/// `GET /api/overview` — aggregate counts across catalog, storage,
/// queries, pipelines, governance and service health.
pub async fn get(State(state): State<AppState>) -> Response {
    match get_body(&state).await {
        Ok(body) => (StatusCode::OK, ApiJson(body)).into_response(),
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "error": js_error(err) })),
        )
            .into_response(),
    }
}

/// Pipeline run counts: from `Dagster` when it answers (it knows about runs
/// in flight), otherwise from the pipeline definitions the console stores.
async fn pipeline_counts(state: &AppState) -> (Value, bool) {
    if let Ok(runs) = state.dagster.list_runs(100).await {
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
        return (
            json!({ "active": active, "failed": failed, "delayed": 0 }),
            true,
        );
    }
    let counts = match state.pg.as_deref() {
        Some(pool) => overview::counts(pool).await.unwrap_or_default(),
        None => overview::OverviewCounts::default(),
    };
    (
        json!({
            "active": counts.pipelines_running,
            "failed": counts.pipelines_failed,
            "delayed": counts.pipelines_delayed,
        }),
        false,
    )
}

/// Health of the pieces this page depends on, as the console can observe
/// them right now. `ClickHouse` answered (this body exists), so the only
/// open questions are Postgres and the orchestrator.
async fn service_health(state: &AppState, orchestrator_ok: bool) -> Value {
    let postgres_ok = match state.pg.as_deref() {
        Some(pool) => sqlx::query("SELECT 1").fetch_one(pool).await.is_ok(),
        None => false,
    };
    let postgres_status = if postgres_ok { "healthy" } else { "unhealthy" };
    let orchestrator_status = if orchestrator_ok {
        "healthy"
    } else {
        "unavailable"
    };
    let items = json!([
        { "name": "ClickHouse", "status": "healthy" },
        { "name": "Postgres", "status": postgres_status },
        { "name": "Orchestrator", "status": orchestrator_status },
    ]);
    let healthy = 1 + i32::from(postgres_ok) + i32::from(orchestrator_ok);
    json!({
        "healthy": healthy,
        "degraded": 0,
        "unhealthy": i32::from(!postgres_ok),
        "unavailable": i32::from(!orchestrator_ok),
        "items": items,
    })
}

/// The open alerts, newest first, as the page's incident list.
async fn incidents(state: &AppState) -> Vec<Value> {
    let Some(pool) = state.pg.as_deref() else {
        return Vec::new();
    };
    overview::list_alerts(pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|a| a.status != "resolved")
        .take(5)
        .map(|a| {
            json!({
                "id": a.id, "title": a.title, "severity": a.severity,
                "source": a.source, "at": a.at,
            })
        })
        .collect()
}

async fn get_body(state: &AppState) -> Result<Value, OverviewError> {
    let ch = &state.clickhouse;
    // Same set of assets the catalog lists (Bronze datasets, Silver tables
    // that are not just a Bronze table's landing spot, and Gold marts), so
    // this card and Data Explorer answer with the same number.
    let total_row = ch
        .rows(
            "SELECT toString(
         (SELECT count() FROM (SELECT slug FROM lake.`bronze_meta.dataset_catalog`
            UNION ALL SELECT slug FROM lake.`bronze_meta_sec.dataset_catalog`))
       + (SELECT count() FROM system.tables WHERE database='silver' AND name NOT IN (
            SELECT table_name FROM lake.`bronze_meta.dataset_catalog`
            UNION ALL SELECT table_name FROM lake.`bronze_meta_sec.dataset_catalog`))
       + (SELECT count() FROM system.tables WHERE database='serving' AND name NOT LIKE '%\\_baru')) n",
            None,
        )
        .await?;
    let stale_row = ch
        .rows(
            "SELECT toString(countIf(coalesce(s.total,0)=0)) stale FROM (
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

    let (pipelines, orchestrator_ok) = pipeline_counts(state).await;
    let counts = match state.pg.as_deref() {
        Some(pool) => overview::counts(pool).await.unwrap_or_default(),
        None => overview::OverviewCounts::default(),
    };

    let total_row = total_row.first();
    let stale_row = stale_row.first();
    let hot_row = hot_row.first();
    let warm_row = warm_row.first();
    let q_row = q_row.first();

    Ok(json!({
        "assetsTotal": num_or_zero(total_row, "n"),
        // Bronze datasets only: a watermark is the thing that can go stale.
        "staleAssets": num_or_zero(stale_row, "stale"),
        // Only Hot is measurable from here: it is ClickHouse's own parts.
        // Warm lives in Iceberg and Cold/AI are not tracked yet, so they
        // report an unknown size instead of an invented one.
        "assetsByTier": {
            "hot": { "count": num_or_zero(hot_row, "assets"), "bytes": num_or_zero(hot_row, "bytes") },
            "warm": { "count": num_or_zero(warm_row, "assets"), "bytes": Value::Null },
            "cold": { "count": 0, "bytes": Value::Null },
            "ai": { "count": 0, "bytes": Value::Null },
        },
        "pipelines": pipelines,
        "streaming": { "jobs": 0, "maxLagSeconds": 0, "unhealthy": 0 },
        "queries": {
            "volume24h": num_or_zero(q_row, "vol"),
            "p95Ms": num_or_zero(q_row, "p95"),
            "failureRate": q_row.and_then(|r| str_col(r, "err").parse::<f64>().ok()).unwrap_or(0.0),
            "cacheAssistRate": 0,
            "scannedBytes24h": num_or_zero(q_row, "scan"),
        },
        "policyViolations7d": counts.policy_violations_7d,
        "pendingApprovals": counts.pending_approvals,
        "agents": { "activeRuns": counts.active_agent_runs, "budgetUsedRate": 0 },
        "services": service_health(state, orchestrator_ok).await,
        "incidents": incidents(state).await,
        "generatedAt": iso_now(),
    }))
}

/// `POST /api/overview` — the recent-activity feed, from the audit trail
/// (every console and Copilot action lands there). Despite the verb this
/// reads only; there is no request body and nothing is mutated.
///
/// `Dagster` run history is the fallback for deployments whose console
/// Postgres is not configured.
pub async fn refresh(State(state): State<AppState>) -> Response {
    if let Some(pool) = state.pg.as_deref() {
        match overview::recent_activity(pool, 20).await {
            Ok(activity) => {
                return (StatusCode::OK, ApiJson(json!({ "activity": activity }))).into_response();
            }
            // Say so rather than silently falling back to the orchestrator.
            Err(err) => tracing::warn!(error = %err, "audit activity read failed; trying Dagster"),
        }
    }
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
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "activity": [], "error": js_error(err) })),
        )
            .into_response(),
    }
}

/// Now, as an ISO-8601 instant — when the numbers above were read. Written
/// by hand so this does not depend on `time`'s `formatting` feature.
fn iso_now() -> String {
    let at = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second()
    )
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

// ── Alert instances (Task 2.6) ──────────────────────────────────────────
//
// See `lakehouse_store::overview`'s module doc comment for why these live
// in Postgres rather than alongside `lakehouse_alerts`'s rule definitions
// in ClickHouse.

fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "overview alert store unavailable: no Postgres pool is configured \
             (DATABASE_URL is missing or not a valid Postgres connection string)"
                .to_owned(),
        )
    })
}

/// `GET /api/overview/alerts` — every alert instance, most recent first.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_alerts(State(state): State<AppState>) -> ApiResult<ApiJson<Vec<AlertItem>>> {
    Ok(ApiJson(overview::list_alerts(pool(&state)?).await?))
}

/// `POST /api/overview/alerts/{id}/acknowledge`.
///
/// # Errors
///
/// 404 if `id` is unknown; 503/500 as above.
pub async fn acknowledge_alert(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match overview::acknowledge_alert(
        match pool(&state) {
            Ok(p) => p,
            Err(err) => return crate::error::ApiRejection(err).into_response(),
        },
        &id,
    )
    .await
    {
        Ok(Some(alert)) => (StatusCode::OK, ApiJson(alert)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            ApiJson(json!({ "error": format!("Alert {id} not found") })),
        )
            .into_response(),
        Err(err) => crate::error::ApiRejection(err.into()).into_response(),
    }
}

/// The `POST /api/overview/alerts/{id}/resolve` body.
#[derive(Debug, Deserialize)]
pub struct ResolveAlertBody {
    #[serde(default)]
    note: String,
}

/// `POST /api/overview/alerts/{id}/resolve`.
///
/// # Errors
///
/// 400 on a malformed body; 404 if `id` is unknown; 503/500 as above.
pub async fn resolve_alert(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let note: ResolveAlertBody = if body.is_empty() {
        ResolveAlertBody {
            note: String::new(),
        }
    } else {
        match serde_json::from_slice(&body) {
            Ok(b) => b,
            Err(err) => {
                return crate::error::ApiRejection(ApiError::BadRequest(format!(
                    "invalid JSON: {err}"
                )))
                .into_response();
            }
        }
    };
    let pg = match pool(&state) {
        Ok(p) => p,
        Err(err) => return crate::error::ApiRejection(err).into_response(),
    };
    match overview::resolve_alert(pg, &id, &note.note).await {
        Ok(Some(alert)) => (StatusCode::OK, ApiJson(alert)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            ApiJson(json!({ "error": format!("Alert {id} not found") })),
        )
            .into_response(),
        Err(err) => crate::error::ApiRejection(err.into()).into_response(),
    }
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
