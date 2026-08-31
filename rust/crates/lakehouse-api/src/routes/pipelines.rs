//! `GET /api/pipelines`, `GET /api/pipelines/{id}/runs`,
//! `POST /api/pipelines/{id}/trigger` — `Dagster` jobs surfaced as console
//! pipelines.
//!
//! Ports `src/app/api/pipelines/route.ts`,
//! `src/app/api/pipelines/[id]/runs/route.ts`, and
//! `src/app/api/pipelines/[id]/trigger/route.ts`.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lakehouse_core::ApiError;
use lakehouse_dagster::{DgClient, DgError, DgJob, DgRun, iso_from_unix_seconds, map_run_status};
use lakehouse_store::PgPool;
use lakehouse_store::pipelines::{self, CreatePipelineInput};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::support::js_error;
use crate::state::AppState;

use crate::tenant::{TENANT_OWNER, TENANT_SOURCE};
const TARGET: &str = "serving.mart_* (Gold)";

/// `GET /api/pipelines` — every `Dagster` job, enriched with its most
/// recent run and (first) schedule, unioned with every Postgres-authored
/// pipeline definition (`createPipeline`/`generatePipelineFromPrompt`,
/// Task 2.5) so an authored pipeline is visible immediately rather than
/// vanishing the way an authored governance rule did before the Task 2.3
/// gap fix — see `0007_pipelines.sql`'s header comment.
pub async fn list(State(state): State<AppState>) -> Response {
    match list_body(&state.dagster, state.pg.as_deref()).await {
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

#[derive(Debug, thiserror::Error)]
enum ListError {
    #[error("{0}")]
    Dagster(#[from] DgError),
    #[error("{0}")]
    Store(#[from] lakehouse_store::StoreError),
}

async fn list_body(dagster: &DgClient, pg: Option<&PgPool>) -> Result<Value, ListError> {
    let (jobs, runs) =
        tokio::try_join!(dagster.list_jobs_with_schedules(), dagster.list_runs(100))?;

    let mut pipelines: Vec<Value> = jobs
        .iter()
        .map(|j| {
            let last = last_run_for(&runs, &j.name);
            json!({
                "id": j.name,
                "name": j.name,
                "kind": "batch",
                "status": last.map_or("unknown", |r| map_run_status(&r.status)),
                "owner": TENANT_OWNER.as_str(),
                "source": TENANT_SOURCE.as_str(),
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
    if let Some(pg) = pg {
        let authored = pipelines::list_pipelines(pg).await?;
        pipelines.extend(authored.iter().filter_map(|p| serde_json::to_value(p).ok()));
    }
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

// ── Postgres-backed writes + Dagster mutations (Task 2.5) ──────────────
//
// createPipeline/generatePipelineFromPrompt author a `pipeline_definition`
// row (Postgres) -- there is no generic "run an arbitrary pipeline" engine
// behind Dagster to hand these to. cancelRun/retryRun/pausePipeline/
// resumePipeline are real Dagster mutations against jobs/runs that already
// exist there.

/// Borrow the Postgres pool, or fail with a 503. Mirrors
/// `routes::identity::pool`/`routes::governance::pool`.
fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "pipeline store unavailable: no Postgres pool is configured \
             (DATABASE_URL is missing or not a valid Postgres connection string)"
                .to_owned(),
        )
    })
}

fn parse_body<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|err| ApiError::BadRequest(format!("invalid JSON: {err}")))
}

/// The `POST /api/pipelines` body. Mirrors `CreatePipelineInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePipelineBody {
    name: String,
    kind: String,
    source_zone: String,
    source_table: String,
    #[serde(default)]
    #[allow(
        dead_code,
        reason = "accepted for contract compatibility, not yet stored"
    )]
    incremental_column: Option<String>,
    #[serde(default)]
    #[allow(
        dead_code,
        reason = "accepted for contract compatibility, not yet stored"
    )]
    transforms: Vec<String>,
    #[serde(default)]
    #[allow(
        dead_code,
        reason = "accepted for contract compatibility, not yet stored"
    )]
    fbic_enabled: bool,
    target_zone: String,
    target_table: String,
    schedule: String,
    #[serde(default)]
    owner: Option<String>,
}

/// `POST /api/pipelines` — author a new pipeline definition. Returns 201.
///
/// # Errors
///
/// 400 on a malformed body; 409 if the name is taken; 503/500 as above.
pub async fn create(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<pipelines::Pipeline>)> {
    let body: CreatePipelineBody = parse_body(&body)?;
    let input = CreatePipelineInput {
        name: body.name,
        kind: body.kind,
        source_zone: body.source_zone,
        source_table: body.source_table,
        target_zone: body.target_zone,
        target_table: body.target_table,
        schedule: body.schedule,
        owner: body.owner,
    };
    let created = pipelines::create_pipeline(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(created)))
}

/// The `POST /api/pipelines/generate` body. Mirrors `GeneratePipelineInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratePipelineBody {
    #[serde(default)]
    #[allow(
        dead_code,
        reason = "the LLM model is fixed by server config, not caller-chosen"
    )]
    model: Option<String>,
    instruction: String,
    #[serde(default)]
    #[allow(
        dead_code,
        reason = "accepted for contract compatibility, not yet used in the prompt"
    )]
    file_name: Option<String>,
    database: String,
}

/// Derive a `snake_case` pipeline name from free text, matching
/// `mock/pipelines.ts`'s `generatePipelineFromPrompt` fallback (first four
/// words, non-alnum stripped, lowercased) — used both as the final
/// fallback when the LLM is unavailable/returns nothing usable, and to
/// sanitize whatever name the LLM does propose.
fn derive_pipeline_name(text: &str) -> String {
    let name: String = text
        .split_whitespace()
        .take(4)
        .collect::<Vec<_>>()
        .join("_")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect::<String>()
        .to_lowercase();
    if name.is_empty() {
        "agentic_pipeline".to_owned()
    } else {
        name
    }
}

/// `POST /api/pipelines/generate` — ask the LLM to name/scaffold a pipeline
/// from a natural-language instruction, then author it the same way
/// [`create`] does. Returns 201.
///
/// The LLM is asked only for a short pipeline name; the rest of the
/// pipeline (kind/source/target/schedule) is filled in deterministically
/// from `instruction`/`database`, matching `mock/pipelines.ts`'s
/// `generatePipelineFromPrompt` shape (`kind: "incremental"`,
/// `schedule: "On demand"`, `owner: "Agentic Builder"`). If the LLM call
/// fails or returns something unusable, [`derive_pipeline_name`]'s
/// deterministic fallback is used instead — an LLM outage must not turn
/// this endpoint into a 503 when the mock-equivalent behavior never needed
/// the LLM to succeed at all.
///
/// # Errors
///
/// 400 on a malformed body; 409 if the derived name collides; 503/500 as
/// above.
pub async fn generate(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<pipelines::Pipeline>)> {
    let body: GeneratePipelineBody = parse_body(&body)?;
    let name = llm_pipeline_name(&state, &body.instruction)
        .await
        .unwrap_or_else(|| derive_pipeline_name(&body.instruction));
    let input = CreatePipelineInput {
        name,
        kind: "incremental".to_owned(),
        source_zone: body.database.clone(),
        source_table: "source_table".to_owned(),
        target_zone: body.database,
        target_table: "target_table".to_owned(),
        schedule: "On demand".to_owned(),
        owner: Some("Agentic Builder".to_owned()),
    };
    let created = pipelines::create_pipeline(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(created)))
}

/// Ask the LLM for a short `snake_case` pipeline name summarizing
/// `instruction`. Returns `None` on any failure (transport, non-2xx, empty
/// reply) or if the sanitized reply is empty — [`generate`] falls back to
/// the deterministic name in every such case rather than surfacing an LLM
/// failure as a hard error.
async fn llm_pipeline_name(state: &AppState, instruction: &str) -> Option<String> {
    use lakehouse_llm::{ChatMessage, ChatOptions, ChatRole};
    let messages = vec![
        ChatMessage {
            role: ChatRole::System,
            content: "You name data pipelines. Reply with ONLY a short snake_case \
                      identifier (2-4 words), no punctuation, no explanation."
                .to_owned(),
        },
        ChatMessage {
            role: ChatRole::User,
            content: instruction.to_owned(),
        },
    ];
    let reply = state
        .llm
        .chat(
            &messages,
            ChatOptions {
                temperature: Some(0.2),
                max_tokens: Some(16),
            },
        )
        .await
        .ok()?;
    let name = derive_pipeline_name(&reply);
    (name != "agentic_pipeline").then_some(name)
}

/// `POST /api/pipelines/{id}/pause` — pause a pipeline. Dispatches on
/// whether `id` names a Postgres-authored draft (id prefix `pl-`, no
/// backing job) or a real `Dagster` job (pauses its first schedule, if
/// any).
///
/// # Errors
///
/// 404 if `id` is unknown (or names a job with no schedule to pause); 503
/// as above.
pub async fn pause(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    set_pipeline_paused(&state, &id, true).await
}

/// `POST /api/pipelines/{id}/resume` — the inverse of [`pause`].
pub async fn resume(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    set_pipeline_paused(&state, &id, false).await
}

async fn set_pipeline_paused(state: &AppState, id: &str, paused: bool) -> Response {
    if id.starts_with("pl-") {
        return match authored_status(state, id, paused).await {
            Ok(Some(p)) => (StatusCode::OK, ApiJson(p)).into_response(),
            Ok(None) => (
                StatusCode::NOT_FOUND,
                ApiJson(json!({ "error": format!("Pipeline {id} not found") })),
            )
                .into_response(),
            Err(err) => crate::error::ApiRejection(err).into_response(),
        };
    }
    dagster_schedule_toggle(state, id, paused).await
}

async fn authored_status(
    state: &AppState,
    id: &str,
    paused: bool,
) -> Result<Option<pipelines::Pipeline>, ApiError> {
    let status = if paused { "paused" } else { "ready" };
    Ok(pipelines::set_status(pool(state)?, id, status).await?)
}

async fn dagster_schedule_toggle(state: &AppState, job_name: &str, paused: bool) -> Response {
    let jobs = match state.dagster.list_jobs_with_schedules().await {
        Ok(jobs) => jobs,
        Err(err) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                ApiJson(json!({ "error": js_error(err) })),
            )
                .into_response();
        }
    };
    let Some(job) = jobs.iter().find(|j| j.name == job_name) else {
        return (
            StatusCode::NOT_FOUND,
            ApiJson(json!({ "error": format!("Pipeline {job_name} not found") })),
        )
            .into_response();
    };
    let Some(schedule) = job.schedules.first() else {
        return (
            StatusCode::CONFLICT,
            ApiJson(
                json!({ "error": format!("Pipeline {job_name} has no schedule to pause/resume") }),
            ),
        )
            .into_response();
    };
    let outcome = if paused {
        state.dagster.stop_schedule(&schedule.name).await
    } else {
        state.dagster.start_schedule(&schedule.name).await
    };
    match outcome {
        Ok(o) if o.ok => {
            let body = json!({
                "id": job_name,
                "name": job_name,
                "kind": "batch",
                "status": if paused { "paused" } else { "ready" },
                "owner": TENANT_OWNER.as_str(),
                "source": TENANT_SOURCE.as_str(),
                "target": TARGET,
                "schedule": schedule_label(job),
                "lastRunAt": "",
                "slaOk": true,
                "freshnessLagSeconds": 0,
            });
            (StatusCode::OK, ApiJson(body)).into_response()
        }
        Ok(o) => (
            StatusCode::CONFLICT,
            ApiJson(json!({ "error": o.error.unwrap_or_else(|| "schedule mutation failed".to_owned()) })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "error": js_error(err) })),
        )
            .into_response(),
    }
}

/// `POST /api/pipelines/runs/{runId}/cancel` — terminate a running
/// `Dagster` run.
///
/// # Errors
///
/// 404 if `Dagster` reports the run doesn't exist; 409 if it exists but
/// can't be terminated (already finished, ...); 503 on a transport
/// failure.
pub async fn cancel_run(State(state): State<AppState>, Path(run_id): Path<String>) -> Response {
    match state.dagster.terminate_run(&run_id).await {
        Ok(outcome) if outcome.error.is_none() => (
            StatusCode::OK,
            ApiJson(run_mutation_body(&run_id, "cancelled")),
        )
            .into_response(),
        Ok(outcome) => dagster_mutation_failure(outcome.error),
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "error": js_error(err) })),
        )
            .into_response(),
    }
}

/// `POST /api/pipelines/runs/{runId}/retry` — re-execute a finished run
/// from the start.
///
/// # Errors
///
/// Same as [`cancel_run`].
pub async fn retry_run(State(state): State<AppState>, Path(run_id): Path<String>) -> Response {
    match state.dagster.launch_reexecution(&run_id).await {
        Ok(outcome) if outcome.error.is_none() => {
            let new_id = outcome.run_id.unwrap_or(run_id);
            (
                StatusCode::OK,
                ApiJson(run_mutation_body(&new_id, "running")),
            )
                .into_response()
        }
        Ok(outcome) => dagster_mutation_failure(outcome.error),
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "error": js_error(err) })),
        )
            .into_response(),
    }
}

/// A minimal `PipelineRun` body for [`cancel_run`]/[`retry_run`]. `Dagster`
/// terminate/reexecute mutations only ever return a `runId` on success, not
/// the owning job name or run stats — enriching this further would need an
/// extra round trip (`listRuns` + linear scan) for a value the caller
/// already knows (it's the pipeline it just cancelled/retried a run of), so
/// `pipelineId` is left empty here, same tradeoff the contract's `runId`
/// signature already forces.
fn run_mutation_body(run_id: &str, status: &str) -> Value {
    json!({
        "id": run_id,
        "pipelineId": "",
        "status": status,
        "startedAt": now_iso(),
        "processed": 0,
        "accepted": 0,
        "rejected": 0,
        "retried": 0,
        "costUnits": 0,
    })
}

/// A `Dagster`-side typed failure (`RunNotFoundError`, ...) reported via
/// `Ok(LaunchOutcome { error: Some(..), .. })` rather than `Err` — see
/// `DgClient::terminate_run`/`launch_reexecution`'s doc comments. Maps to
/// 404 when the typename/message indicates the run wasn't found, 409
/// (semantically invalid but not "missing") otherwise.
fn dagster_mutation_failure(error: Option<String>) -> Response {
    let message = error.unwrap_or_else(|| "Dagster mutation failed".to_owned());
    let status = if message.contains("NotFound") {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::CONFLICT
    };
    (status, ApiJson(json!({ "error": message }))).into_response()
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
                    name: "s1".to_owned(),
                    cron_schedule: "0 3 * * *".to_owned(),
                    schedule_state: DgScheduleState {
                        status: "RUNNING".to_owned(),
                    },
                },
                DgSchedule {
                    name: "s2".to_owned(),
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
