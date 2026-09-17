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
use crate::next_run::next_run_at;
use crate::routes::support::{js_error, num_or_zero, str_col};
use crate::state::AppState;
use lakehouse_dagster::{
    DgClient, DgError, DgJob, DgRun, DgSchedule, iso_from_unix_seconds, map_run_status,
};
use time::OffsetDateTime;

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
    let pg = state.pg.as_deref();
    match get_body(&state.clickhouse, &state.dagster, pg, &probes).await {
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
    pg: Option<&PgPool>,
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
    // `jobs` used to be fetched-but-unread, kept only so a `Dagster`
    // outage on this call still 503s like the original TypeScript. Task
    // B1 (WS5) now reads it for real: each `RUNNING` schedule's own
    // latest run, via `list_jobs_with_schedules`/`list_runs_for_job` --
    // never the shared, capped `runs` list above, which a long-stalled
    // schedule can fall out of entirely (WS5 plan review U7).
    let jobs = dagster.list_jobs_with_schedules().await?;
    let delayed = count_delayed_schedules(dagster, &jobs, OffsetDateTime::now_utc()).await;

    // `overview.pendingApprovals`/`agents.activeRuns` (WS5 item B2): `null`
    // both when no Postgres pool is configured and when a configured
    // pool's query fails -- the rest of `/api/overview` is real without
    // Postgres, so a Postgres hiccup degrades these two fields to
    // "not measured," never a 503 for the whole page and never a
    // fabricated `0`.
    let (pending_approvals, active_agent_runs) = match pg {
        Some(pool) => (
            lakehouse_store::agents::count_pending_approvals(pool)
                .await
                .inspect_err(|err| tracing::warn!(%err, "count_pending_approvals failed"))
                .ok(),
            lakehouse_store::agents::count_active_agent_runs(pool)
                .await
                .inspect_err(|err| tracing::warn!(%err, "count_active_agent_runs failed"))
                .ok(),
        ),
        None => (None, None),
    };

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

    let counts = SummaryCounts {
        active,
        failed,
        delayed,
        pending_approvals,
        active_agent_runs,
    };
    Ok(summary_json(
        assets_row, hot_row, warm_row, q_row, &counts, probes,
    ))
}

/// Builds the `GET /api/overview` summary body from already-fetched rows and
/// counts. Pulled out of `get_body` so the JSON shape — in particular which
/// fields are real measurements versus `null` — is assertable without a
/// `ClickHouse`/`Dagster` connection.
///
/// WS1 task 1.8: `queries.cacheAssistRate`, `policyViolations7d`,
/// `agents.budgetUsedRate`, and the `cold`/`ai` tiers are `null` because
/// nothing in this route measures them today — no cache-hit signal, no
/// policy engine, no agent-run budget accounting. `warm.bytes` is `null`
/// because `rows * 220` was an invented per-row byte size, not a
/// measurement. `services.healthy/degraded/unhealthy` (WS5 item A4) are
/// real, measured counts. `pipelines.delayed` (WS5 item B1) is a real count
/// when every `RUNNING` schedule's run lookup succeeds, `null` when any one
/// of them fails — a failed query and "never ran" are different answers,
/// and folding the former into the latter used to make a `Dagster` outage
/// report a confident `delayed: 0`. `pendingApprovals`/`agents.activeRuns`
/// (WS5 item B2) are real
/// Postgres counts when a pool is configured and the query succeeds,
/// `null` otherwise (no pool, or the query itself failed) — never a
/// fabricated `0`, which would read as "definitely zero waiting" rather
/// than "not measured right now." WS7 owns the policy engine and agent
/// budgets. The `streaming` and `incidents` structures are dropped
/// entirely (not nulled): neither was ever measured, and — apart from the
/// incidents card, removed alongside this — nothing in the `TypeScript`
/// client reads either one.
/// The `pipelines`/`agents` counters `summary_json` needs, grouped into
/// one struct purely to stay under `clippy::too_many_arguments` -- these
/// have no shared invariant beyond "computed once in `get_body`, read
/// once here".
struct SummaryCounts {
    active: usize,
    failed: usize,
    /// `None` when the per-schedule `Dagster` run lookup failed for at
    /// least one `RUNNING` schedule (see `count_delayed_schedules`) — never
    /// a fabricated `0`.
    delayed: Option<usize>,
    pending_approvals: Option<i64>,
    active_agent_runs: Option<i64>,
}

fn summary_json(
    assets_row: Option<&serde_json::Map<String, Value>>,
    hot_row: Option<&serde_json::Map<String, Value>>,
    warm_row: Option<&serde_json::Map<String, Value>>,
    q_row: Option<&serde_json::Map<String, Value>>,
    counts: &SummaryCounts,
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
        "pipelines": { "active": counts.active, "failed": counts.failed, "delayed": counts.delayed },
        "queries": {
            "volume24h": num_or_zero(q_row, "vol"),
            "p95Ms": num_or_zero(q_row, "p95"),
            "failureRate": q_row.and_then(|r| str_col(r, "err").parse::<f64>().ok()).unwrap_or(0.0),
            "cacheAssistRate": Value::Null,
            "scannedBytes24h": num_or_zero(q_row, "scan"),
        },
        "policyViolations7d": Value::Null,
        "pendingApprovals": counts.pending_approvals,
        // `budgetUsedRate` stays `Value::Null` -- WS7 owns budget
        // accounting; no code updates `budget_consumed`/`budget_reserved`
        // yet, and WS5 does not add a writer for it, only a reader would
        // be dishonest without one.
        "agents": { "activeRuns": counts.active_agent_runs, "budgetUsedRate": Value::Null },
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

/// A `RUNNING`-state schedule is delayed when its most recent run started
/// before the fire time immediately preceding `now` — i.e. it missed its
/// last expected occurrence. Computed via [`next_run_at`] (WS4 item G2,
/// `croner`-backed) run repeatedly: `croner` only exposes "next after," so
/// "the expected fire time just before now" is derived by walking forward
/// from a 48h-ago probe until the next occurrence is no longer before
/// `now` — matching the grace-window idiom WS5 plan review U7 asks for
/// ("late when the last run started before the previous expected fire
/// time"). A schedule with no run yet (`last_run: None`) is never delayed
/// — no baseline to be late against, and a freshly created schedule must
/// not immediately show red. A cron expression [`next_run_at`] cannot
/// parse (malformed, or a non-5-field shape) also never flags delayed —
/// this build does not guess at an interval it cannot compute.
fn is_schedule_delayed(
    schedule: &DgSchedule,
    last_run: Option<&DgRun>,
    now: OffsetDateTime,
) -> bool {
    if schedule.schedule_state.status != "RUNNING" {
        return false;
    }
    let Some(last_run) = last_run else {
        return false;
    };
    let Some(last_start) = last_run.start_time else {
        return false;
    };
    #[allow(
        clippy::cast_possible_truncation,
        reason = "start_time is Unix seconds as f64; truncation to i64 seconds is inconsequential \
                  at any real-world timestamp magnitude"
    )]
    let last_start_secs = last_start as i64;
    let Ok(last_start) = OffsetDateTime::from_unix_timestamp(last_start_secs) else {
        return false;
    };
    let probe_start = now - time::Duration::hours(48);
    let Some(first_after_probe) = next_run_at(&schedule.cron_schedule, probe_start) else {
        return false; // cron_schedule not parseable -- never guess
    };
    // Walk forward from the 48h-ago probe until the next occurrence is no
    // longer before `now` -- that last "before now" occurrence is the
    // most recent expected fire time.
    let mut candidate = first_after_probe;
    let previous_expected = loop {
        match next_run_at(&schedule.cron_schedule, candidate) {
            Some(next) if next < now => candidate = next,
            _ => break candidate,
        }
    };
    last_start < previous_expected
}

/// Counts every `RUNNING` schedule across `jobs` that [`is_schedule_delayed`]
/// against its own latest run (fetched per-job via `list_runs_for_job`,
/// NEVER the shared, capped `list_runs(100)` window — WS5 plan review U7:
/// a schedule that stopped firing long ago falls out of that window and
/// would otherwise score "no baseline" instead of "delayed").
///
/// `None` when any one `list_runs_for_job` call fails, matching the other
/// WS5 observability metrics (`pendingApprovals`, `ingestLagSeconds`,
/// `cacheHitRate`, `agentSuccessRate`): a failed lookup is "not measured,"
/// never folded into the count as if that schedule had simply never run.
/// Before this fix `.ok()` discarded the error and treated it exactly like
/// "no run yet" — `is_schedule_delayed` correctly calls a schedule with no
/// baseline "not delayed," so a `Dagster` outage silently produced a
/// confident `delayed: 0` instead of "unknown." One failed lookup poisons
/// the whole count: a partial count reported as a total is its own
/// fabrication.
async fn count_delayed_schedules(
    dagster: &DgClient,
    jobs: &[DgJob],
    now: OffsetDateTime,
) -> Option<usize> {
    let mut delayed = 0;
    for job in jobs {
        for schedule in &job.schedules {
            if schedule.schedule_state.status != "RUNNING" {
                continue;
            }
            let runs = dagster
                .list_runs_for_job(&job.name, 1)
                .await
                .inspect_err(|err| {
                    tracing::warn!(%err, job = %job.name, "list_runs_for_job failed");
                })
                .ok()?;
            let last_run = runs.into_iter().next();
            if is_schedule_delayed(schedule, last_run.as_ref(), now) {
                delayed += 1;
            }
        }
    }
    Some(delayed)
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

/// The `POST /api/overview/alerts/{id}/silence` body: how many minutes
/// from now to silence this alert (and its `rule_id`, if any) for.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SilenceAlertBody {
    until_minutes: i64,
}

/// `POST /api/overview/alerts/{id}/silence` — silence an alert instance
/// (and, if it carries a `rule_id`, that rule's future firings) for
/// `untilMinutes`. WS5 item C1. A silence marks the row `acknowledged`
/// (like [`acknowledge_alert`]) and sets `silenced_until`, which
/// `lakehouse_alerts::SilenceSource`'s real implementation
/// (`routes::alerts::ApiSilenceSource`) consults to suppress both the next
/// `alert_instance` insert and the next webhook/email delivery while the
/// silence is active.
///
/// # Errors
///
/// 400 on a malformed body; 404 if `id` is unknown; 503/500 as above.
pub async fn silence_alert(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let until: SilenceAlertBody = match serde_json::from_slice(&body) {
        Ok(b) => b,
        Err(err) => {
            return crate::error::ApiRejection(ApiError::BadRequest(format!(
                "invalid JSON: {err}"
            )))
            .into_response();
        }
    };
    let pg = match pool(&state) {
        Ok(p) => p,
        Err(err) => return crate::error::ApiRejection(err).into_response(),
    };
    let until_at = OffsetDateTime::now_utc() + time::Duration::minutes(until.until_minutes);
    match overview::silence_alert(pg, &id, until_at).await {
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
    #![allow(
        clippy::cast_precision_loss,
        reason = "test fixture timestamps are small, real-world Unix seconds; casting to f64 to \
                  match DgRun::start_time's wire shape loses no meaningful precision"
    )]

    use super::*;
    use lakehouse_dagster::{DgRun, DgSchedule, DgScheduleState};
    use time::macros::datetime;

    fn fixture_schedule(cron: &str, status: &str) -> DgSchedule {
        DgSchedule {
            name: "s".to_owned(),
            cron_schedule: cron.to_owned(),
            schedule_state: DgScheduleState {
                status: status.to_owned(),
            },
        }
    }

    fn fixture_run(job_name: &str, start: time::OffsetDateTime) -> DgRun {
        DgRun {
            run_id: "r1".to_owned(),
            job_name: job_name.to_owned(),
            status: "SUCCESS".to_owned(),
            start_time: Some(start.unix_timestamp() as f64),
            end_time: None,
        }
    }

    #[test]
    fn a_running_schedule_is_delayed_when_its_own_latest_run_missed_the_previous_expected_fire_time()
     {
        let now = datetime!(2026 - 09 - 11 03:20:00 UTC); // maintenance.py's "0 3 * * *" fired at 03:00 today
        let schedule = fixture_schedule("0 3 * * *", "RUNNING");
        // last run started yesterday -- the 03:00 fire today never happened.
        let last_run = fixture_run("maintenance_job", datetime!(2026 - 09 - 10 03:00:05 UTC));
        assert!(is_schedule_delayed(&schedule, Some(&last_run), now));
    }

    #[test]
    fn a_running_schedule_that_fired_on_time_is_not_delayed() {
        let now = datetime!(2026 - 09 - 11 03:20:00 UTC);
        let schedule = fixture_schedule("0 3 * * *", "RUNNING");
        let last_run = fixture_run("maintenance_job", datetime!(2026 - 09 - 11 03:00:05 UTC));
        assert!(!is_schedule_delayed(&schedule, Some(&last_run), now));
    }

    #[test]
    fn a_schedule_with_no_run_yet_is_never_delayed() {
        let now = datetime!(2026 - 09 - 11 03:20:00 UTC);
        let schedule = fixture_schedule("*/15 * * * *", "RUNNING");
        assert!(!is_schedule_delayed(&schedule, None, now));
    }

    #[test]
    fn a_stopped_schedule_is_never_counted_regardless_of_its_last_run() {
        let now = datetime!(2026 - 09 - 11 03:20:00 UTC);
        let schedule = fixture_schedule("0 3 * * *", "STOPPED");
        let last_run = fixture_run("maintenance_job", datetime!(2026 - 09 - 01 03:00:00 UTC));
        assert!(!is_schedule_delayed(&schedule, Some(&last_run), now));
    }

    #[test]
    fn now_unix_millis_is_plausible() {
        // Sanity bound: some time after this file was written, well before
        // any realistic clock error.
        assert!(now_unix_millis() > 1_700_000_000_000.0);
    }

    #[test]
    fn overview_reports_every_unmeasured_tile_as_null_and_drops_fabricated_structures() {
        let counts = SummaryCounts {
            active: 0,
            failed: 0,
            delayed: Some(0),
            pending_approvals: None,
            active_agent_runs: None,
        };
        let v = summary_json(None, None, None, None, &counts, &[]);

        assert!(
            v["assetsByTier"]["warm"]["bytes"].is_null(),
            "rows * 220 is not a measurement"
        );
        for tier in ["cold", "ai"] {
            assert!(v["assetsByTier"][tier]["count"].is_null());
            assert!(v["assetsByTier"][tier]["bytes"].is_null());
        }
        assert_eq!(
            v["pipelines"]["delayed"], 0,
            "delayed is a real, measured count (WS5 item B1) when every \
             lookup succeeds — this fixture's Some(0) must serialize as 0, \
             never null"
        );
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
        let counts = SummaryCounts {
            active: 0,
            failed: 0,
            delayed: Some(0),
            pending_approvals: None,
            active_agent_runs: None,
        };
        let v = summary_json(None, None, None, None, &counts, &probes);
        assert_eq!(v["services"]["healthy"], 1);
        assert_eq!(v["services"]["unhealthy"], 1);
        assert_eq!(
            v["services"]["degraded"], 0,
            "no probe in this set ever reports degraded"
        );
    }

    /// WS5 item B2 -- when a Postgres pool is configured and its counts
    /// succeed, `pendingApprovals`/`agents.activeRuns` are the real counts,
    /// never left `null` (the `null` path is only for "no pool" or "the
    /// query failed", asserted above via `None`).
    #[test]
    fn overview_reports_real_pending_approvals_and_active_agent_runs_when_measured() {
        let counts = SummaryCounts {
            active: 0,
            failed: 0,
            delayed: Some(0),
            pending_approvals: Some(3),
            active_agent_runs: Some(2),
        };
        let v = summary_json(None, None, None, None, &counts, &[]);
        assert_eq!(v["pendingApprovals"], 3);
        assert_eq!(v["agents"]["activeRuns"], 2);
    }

    // ── pipelines.delayed: a failed run lookup is "unknown," never "0" ──

    fn fixture_job(name: &str, cron: &str, status: &str) -> DgJob {
        DgJob {
            name: name.to_owned(),
            schedules: vec![fixture_schedule(cron, status)],
        }
    }

    /// The defect this fixes: `.ok()` on a failed `list_runs_for_job`
    /// turned a query failure into `None` ("no run"), which
    /// `is_schedule_delayed` — correctly, for its own job — calls "not
    /// delayed." So an unreachable `Dagster` used to produce a confident
    /// `Some(0)` instead of "unknown." Asserted directly against a
    /// wiremock server that always 500s.
    #[tokio::test]
    async fn count_delayed_schedules_is_none_when_the_run_lookup_fails() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;
        let dagster = DgClient::new(server.uri());
        let jobs = vec![fixture_job("maintenance_job", "0 3 * * *", "RUNNING")];
        let now = datetime!(2026 - 09 - 11 03:20:00 UTC);

        let result = count_delayed_schedules(&dagster, &jobs, now).await;

        assert_eq!(
            result, None,
            "a failed run lookup must poison the whole count, not read as Some(0)"
        );
    }

    /// The companion case this fix must not disturb: when every lookup
    /// succeeds, `count_delayed_schedules` still returns a real `Some`
    /// count.
    #[tokio::test]
    async fn count_delayed_schedules_is_some_when_every_lookup_succeeds() {
        let server = wiremock::MockServer::start().await;
        // last run started yesterday -- the 03:00 fire today never
        // happened, so this schedule counts as delayed.
        let body = json!({
            "data": {
                "runsOrError": {
                    "__typename": "Runs",
                    "results": [{
                        "runId": "r1",
                        "jobName": "maintenance_job",
                        "status": "SUCCESS",
                        "startTime": datetime!(2026 - 09 - 10 03:00:05 UTC).unix_timestamp(),
                        "endTime": Value::Null,
                    }],
                }
            }
        });
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let dagster = DgClient::new(server.uri());
        let jobs = vec![fixture_job("maintenance_job", "0 3 * * *", "RUNNING")];
        let now = datetime!(2026 - 09 - 11 03:20:00 UTC);

        let result = count_delayed_schedules(&dagster, &jobs, now).await;

        assert_eq!(result, Some(1));
    }
}
