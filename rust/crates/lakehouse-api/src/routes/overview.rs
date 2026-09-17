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
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::overview::{self, AlertItem};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
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
    let probes = crate::health::cached_probe_all(&state).await;
    match get_body(&state.clickhouse, &state.dagster, &probes).await {
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

async fn get_body(
    ch: &ChClient,
    dagster: &DgClient,
    probes: &[crate::health::ServiceHealth],
) -> Result<Value, OverviewError> {
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
    // WS1 task 1.8: only `assets` (the tier's dataset count) is read here.
    // A `rows` column used to feed `warm.bytes` via an invented
    // rows-times-220-bytes constant; that estimate is gone (see below), so
    // the column that only fed it is gone too.
    let warm_row = ch
        .rows(
            "SELECT toString(count()) assets FROM (SELECT total FROM lake.`bronze_meta.dataset_sync` UNION ALL SELECT total FROM lake.`bronze_meta_sec.dataset_sync`)",
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

    Ok(summary_json(
        assets_row, hot_row, warm_row, q_row, active, failed, probes,
    ))
}

/// Builds the `GET /api/overview` summary body from already-fetched rows and
/// counts. Pulled out of `get_body` so the JSON shape — in particular which
/// fields are real measurements versus `null` — is assertable without a
/// `ClickHouse`/`Dagster` connection.
///
/// WS1 task 1.8: `pipelines.delayed`, `queries.cacheAssistRate`,
/// `policyViolations7d`, `pendingApprovals`, `agents.*`,
/// `services.healthy/degraded/unhealthy`, and the `cold`/`ai` tiers are
/// `null` because nothing in this route measures them today — no lateness
/// computation, no cache-hit signal, no policy engine, no Postgres pool for
/// approvals, no agent-run accounting, and no service probes. `warm.bytes`
/// is `null` because `rows * 220` was an invented per-row byte size, not a
/// measurement. WS5 is expected to wire up schedule lateness, approvals, and
/// service probes; WS7 owns the policy engine and agent budgets. The
/// `streaming` and `incidents` structures are dropped entirely (not
/// nulled): neither was ever measured, and — apart from the incidents
/// card, removed alongside this — nothing in the `TypeScript` client reads
/// either one.
fn summary_json(
    assets_row: Option<&serde_json::Map<String, Value>>,
    hot_row: Option<&serde_json::Map<String, Value>>,
    warm_row: Option<&serde_json::Map<String, Value>>,
    q_row: Option<&serde_json::Map<String, Value>>,
    active: usize,
    failed: usize,
    probes: &[crate::health::ServiceHealth],
) -> Value {
    json!({
        "assetsTotal": num_or_zero(assets_row, "n"),
        "staleAssets": num_or_zero(assets_row, "stale"),
        "assetsByTier": {
            "hot": { "count": num_or_zero(hot_row, "assets"), "bytes": num_or_zero(hot_row, "bytes") },
            "warm": { "count": num_or_zero(warm_row, "assets"), "bytes": Value::Null },
            "cold": { "count": Value::Null, "bytes": Value::Null },
            "ai": { "count": Value::Null, "bytes": Value::Null },
        },
        "pipelines": { "active": active, "failed": failed, "delayed": Value::Null },
        "queries": {
            "volume24h": num_or_zero(q_row, "vol"),
            "p95Ms": num_or_zero(q_row, "p95"),
            "failureRate": q_row.and_then(|r| str_col(r, "err").parse::<f64>().ok()).unwrap_or(0.0),
            "cacheAssistRate": Value::Null,
            "scannedBytes24h": num_or_zero(q_row, "scan"),
        },
        "policyViolations7d": Value::Null,
        "pendingApprovals": Value::Null,
        "agents": { "activeRuns": Value::Null, "budgetUsedRate": Value::Null },
        "services": {
            "healthy": probes.iter().filter(|h| h.checked && h.ok).count(),
            // Always 0 -- none of `health::probe_all`'s six probes ever
            // reports a "degraded" state, only ok/not-ok (WS5 item A4,
            // matching `health::ServiceHealth::health_label`'s own
            // three-way map). Documented here rather than fabricating a
            // third state no probe produces.
            "degraded": 0,
            "unhealthy": probes.iter().filter(|h| h.checked && !h.ok).count(),
        },
    })
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

    #[test]
    fn overview_reports_every_unmeasured_tile_as_null_and_drops_fabricated_structures() {
        let v = summary_json(None, None, None, None, 0, 0, &[]);

        assert!(
            v["assetsByTier"]["warm"]["bytes"].is_null(),
            "rows * 220 is not a measurement"
        );
        for tier in ["cold", "ai"] {
            assert!(v["assetsByTier"][tier]["count"].is_null());
            assert!(v["assetsByTier"][tier]["bytes"].is_null());
        }
        assert!(v["pipelines"]["delayed"].is_null());
        assert!(v["queries"]["cacheAssistRate"].is_null());
        assert!(v["policyViolations7d"].is_null());
        assert!(
            v["pendingApprovals"].is_null(),
            "0 is false whenever approvals are waiting"
        );
        assert!(v["agents"]["activeRuns"].is_null());
        assert!(v["agents"]["budgetUsedRate"].is_null());
        // No probes at all -> zero of each, not null: `services.healthy`/
        // `unhealthy` are real counts over an (empty) probe set, not an
        // unmeasured metric — see the dedicated test below for the
        // unconfigured-exclusion behavior with a non-empty probe set.
        assert_eq!(v["services"]["healthy"], 0);
        assert_eq!(v["services"]["degraded"], 0);
        assert_eq!(v["services"]["unhealthy"], 0);
        assert!(
            v.get("streaming").is_none(),
            "streaming is removed, not nulled"
        );
        assert!(
            v.get("incidents").is_none(),
            "incidents is removed, not nulled"
        );
        // Real values survive.
        assert!(!v["assetsTotal"].is_null());
        assert!(!v["assetsByTier"]["hot"]["bytes"].is_null());
        assert!(!v["assetsByTier"]["warm"]["count"].is_null());
        assert!(!v["pipelines"]["failed"].is_null());
    }

    /// WS5 item A4 -- `services.healthy`/`unhealthy` come from the same
    /// probes `/api/ops/services` reads, and an unconfigured (unchecked)
    /// service counts toward NEITHER bucket -- never miscounted as
    /// unhealthy.
    #[test]
    fn overview_services_counts_exclude_unconfigured_probes() {
        fn fixture(id: &'static str, checked: bool, ok: bool) -> crate::health::ServiceHealth {
            crate::health::ServiceHealth {
                id,
                name: id,
                ok,
                checked,
                latency_ms: None,
                version: None,
                checked_at: "2026-08-27T04:00:10.075Z".to_owned(),
                error: None,
            }
        }
        let probes = vec![
            fixture("clickhouse", true, true),
            fixture("dagster", true, false),
            fixture("trino", false, false),
        ];
        let v = summary_json(None, None, None, None, 0, 0, &probes);
        assert_eq!(v["services"]["healthy"], 1);
        assert_eq!(v["services"]["unhealthy"], 1);
        assert_eq!(
            v["services"]["degraded"], 0,
            "no probe in this set ever reports degraded"
        );
    }
}
