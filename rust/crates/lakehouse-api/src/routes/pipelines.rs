//! `GET /api/pipelines`, `GET /api/pipelines/{id}/runs`,
//! `POST /api/pipelines/{id}/trigger` — `Dagster` jobs surfaced as console
//! pipelines.
//!
//! Ports `src/app/api/pipelines/route.ts`,
//! `src/app/api/pipelines/[id]/runs/route.ts`, and
//! `src/app/api/pipelines/[id]/trigger/route.ts`.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lakehouse_dagster::{DgClient, DgError, DgJob, DgRun, iso_from_unix_seconds, map_run_status};
use serde_json::{Value, json};

use crate::json::ApiJson;
use crate::routes::support::js_error;
use crate::state::AppState;

const OWNER: &str = "Dinas Pariwisata & Ekraf DKI Jakarta";
const SOURCE: &str = "Satu Data Jakarta + berkas";
const TARGET: &str = "serving.mart_* (Gold)";

/// `GET /api/pipelines` — every `Dagster` job, enriched with its most
/// recent run and (first) schedule.
pub async fn list(State(state): State<AppState>) -> Response {
    match list_body(&state.dagster).await {
        Ok(body) => (StatusCode::OK, ApiJson(body)).into_response(),
        // `catch (e) { return NextResponse.json({ pipelines: [], error:
        // String(e) }, { status: 503 }); }` in `pipelines/route.ts`.
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "pipelines": [], "error": js_error(err) })),
        )
            .into_response(),
    }
}

async fn list_body(dagster: &DgClient) -> Result<Value, DgError> {
    let (jobs, runs) =
        tokio::try_join!(dagster.list_jobs_with_schedules(), dagster.list_runs(100))?;

    let pipelines: Vec<Value> = jobs
        .iter()
        .map(|j| {
            let last = last_run_for(&runs, &j.name);
            json!({
                "id": j.name,
                "name": j.name,
                "kind": "batch",
                "status": last.map_or("unknown", |r| map_run_status(&r.status)),
                "owner": OWNER,
                "source": SOURCE,
                "target": TARGET,
                "schedule": schedule_label(j),
                "lastRunAt": last
                    .and_then(|r| r.start_time)
                    .map_or_else(String::new, iso_from_unix_seconds),
                "slaOk": last.is_none_or(|r| r.status == "SUCCESS"),
                "freshnessLagSeconds": 0,
            })
        })
        .collect();
    Ok(json!({ "pipelines": pipelines }))
}

/// The run with the largest `startTime` for `job_name`, matching the
/// TypeScript's `lastByJob` reduction (`r.startTime ?? 0 > prev.startTime ??
/// 0`, keeping the first row on a tie since `>` is strict).
fn last_run_for<'a>(runs: &'a [DgRun], job_name: &str) -> Option<&'a DgRun> {
    let mut best: Option<&DgRun> = None;
    for r in runs {
        if r.job_name != job_name {
            continue;
        }
        let start = r.start_time.unwrap_or(0.0);
        match best {
            None => best = Some(r),
            Some(prev) if start > prev.start_time.unwrap_or(0.0) => best = Some(r),
            Some(_) => {}
        }
    }
    best
}

/// `sched ? cron: ${sched.cronSchedule} (${sched.scheduleState.status}) :
/// "manual"` — only the first schedule is used.
fn schedule_label(job: &DgJob) -> String {
    job.schedules.first().map_or_else(
        || "manual".to_owned(),
        |s| format!("cron: {} ({})", s.cron_schedule, s.schedule_state.status),
    )
}

/// `GET /api/pipelines/{id}/runs` — up to 30 recent runs of one job.
pub async fn runs(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.dagster.list_runs_for_job(&id, 30).await {
        Ok(runs) => {
            let body =
                json!({ "runs": runs.iter().map(|r| run_to_json(r, &id)).collect::<Vec<_>>() });
            (StatusCode::OK, ApiJson(body)).into_response()
        }
        // `catch (e) { return NextResponse.json({ runs: [], error:
        // String(e) }, { status: 503 }); }` in `[id]/runs/route.ts`.
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "runs": [], "error": js_error(err) })),
        )
            .into_response(),
    }
}

fn run_to_json(r: &DgRun, pipeline_id: &str) -> Value {
    json!({
        "id": r.run_id,
        "pipelineId": pipeline_id,
        "status": map_run_status(&r.status),
        "startedAt": r.start_time.map_or_else(String::new, iso_from_unix_seconds),
        "endedAt": r.end_time.map(iso_from_unix_seconds),
        "processed": 0,
        "accepted": 0,
        "rejected": 0,
        "retried": 0,
        "costUnits": cost_units(r.start_time, r.end_time),
    })
}

/// `r.startTime && r.endTime ? Math.round(r.endTime - r.startTime) : 0` —
/// note the `&&` truthiness check: a `startTime`/`endTime` of exactly `0`
/// (Unix epoch) would also short-circuit to `0` here, same as the
/// TypeScript.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "run durations here are small, non-negative second counts"
)]
fn cost_units(start: Option<f64>, end: Option<f64>) -> i64 {
    match (start, end) {
        (Some(s), Some(e)) if s != 0.0 && e != 0.0 => (e - s).round() as i64,
        _ => 0,
    }
}

/// `POST /api/pipelines/{id}/trigger` — launch a new run of job `id`.
///
/// This mutates live infrastructure (starts a real `Dagster`/`ClickHouse`
/// pipeline run) and is therefore exercised only via the
/// `pipeline-trigger-bad-id` corpus entry, which targets a job name that
/// does not exist so `Dagster` rejects the launch instead of starting one.
pub async fn trigger(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.dagster.launch_run(&id).await {
        Ok(outcome) => {
            if let Some(error) = outcome.error {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    ApiJson(json!({ "error": error })),
                )
                    .into_response();
            }
            let body = json!({
                "id": outcome.run_id,
                "pipelineId": id,
                "status": map_run_status("STARTED"),
                "startedAt": now_iso(),
                "processed": 0,
                "accepted": 0,
                "rejected": 0,
                "retried": 0,
                "costUnits": 0,
            });
            (StatusCode::OK, ApiJson(body)).into_response()
        }
        // `catch (e) { return NextResponse.json({ error: String(e) }, {
        // status: 503 }); }` in `[id]/trigger/route.ts`.
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "error": js_error(err) })),
        )
            .into_response(),
    }
}

/// `new Date().toISOString()` at the moment a run is launched.
fn now_iso() -> String {
    #[allow(
        clippy::cast_precision_loss,
        reason = "current Unix time in seconds fits exactly in f64 until year 285 million"
    )]
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    iso_from_unix_seconds(seconds)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_dagster::{DgSchedule, DgScheduleState};

    use super::*;

    fn run(job_name: &str, status: &str, start: Option<f64>, end: Option<f64>) -> DgRun {
        DgRun {
            run_id: "r".to_owned(),
            job_name: job_name.to_owned(),
            status: status.to_owned(),
            start_time: start,
            end_time: end,
        }
    }

    #[test]
    fn last_run_for_picks_latest_start_time_for_the_job() {
        let runs = vec![
            run("a", "SUCCESS", Some(1.0), None),
            run("a", "FAILURE", Some(5.0), None),
            run("b", "SUCCESS", Some(9.0), None),
        ];
        let last = last_run_for(&runs, "a").unwrap();
        assert_eq!(last.status, "FAILURE");
    }

    #[test]
    fn last_run_for_none_when_job_absent() {
        let runs = vec![run("a", "SUCCESS", Some(1.0), None)];
        assert!(last_run_for(&runs, "missing").is_none());
    }

    #[test]
    fn last_run_for_keeps_first_row_on_tie() {
        // `>` is strict in the TS reduction, so an equal startTime does not
        // replace the first-seen row.
        let runs = vec![
            run("a", "SUCCESS", Some(5.0), None),
            run("a", "FAILURE", Some(5.0), None),
        ];
        let last = last_run_for(&runs, "a").unwrap();
        assert_eq!(last.status, "SUCCESS");
    }

    #[test]
    fn schedule_label_uses_first_schedule_only() {
        let job = DgJob {
            name: "j".to_owned(),
            schedules: vec![
                DgSchedule {
                    cron_schedule: "0 3 * * *".to_owned(),
                    schedule_state: DgScheduleState {
                        status: "RUNNING".to_owned(),
                    },
                },
                DgSchedule {
                    cron_schedule: "0 4 * * *".to_owned(),
                    schedule_state: DgScheduleState {
                        status: "STOPPED".to_owned(),
                    },
                },
            ],
        };
        assert_eq!(schedule_label(&job), "cron: 0 3 * * * (RUNNING)");
    }

    #[test]
    fn schedule_label_manual_when_no_schedules() {
        let job = DgJob {
            name: "j".to_owned(),
            schedules: vec![],
        };
        assert_eq!(schedule_label(&job), "manual");
    }

    #[test]
    fn cost_units_rounds_duration_when_both_present() {
        assert_eq!(cost_units(Some(100.0), Some(103.6)), 4);
    }

    #[test]
    fn cost_units_zero_when_either_missing() {
        assert_eq!(cost_units(None, Some(10.0)), 0);
        assert_eq!(cost_units(Some(10.0), None), 0);
        assert_eq!(cost_units(None, None), 0);
    }

    #[test]
    fn cost_units_zero_when_start_or_end_is_epoch() {
        // `r.startTime && r.endTime` is falsy for exactly 0.
        assert_eq!(cost_units(Some(0.0), Some(10.0)), 0);
        assert_eq!(cost_units(Some(10.0), Some(0.0)), 0);
    }
}
