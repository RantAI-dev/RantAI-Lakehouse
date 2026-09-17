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

use crate::tenant::TENANT_OWNER;

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
            dagster_pipeline_row(j, last)
        })
        .collect();
    if let Some(pg) = pg {
        let authored = pipelines::list_pipelines(pg).await?;
        pipelines.extend(authored.iter().filter_map(|p| serde_json::to_value(p).ok()));
    }
    Ok(json!({ "pipelines": pipelines }))
}

/// Build one `Dagster`-job row for `GET /api/pipelines`. `Dagster`'s job/run
/// API carries no per-job lineage, no `SLA` definition (`WS5` adds
/// `dataset_sla`), and no freshness measurement (`WS2` derives that from
/// Iceberg snapshot timestamps) — `source`, `target`, `slaOk`, and
/// `freshnessLagSeconds` are therefore reported as `null` rather than a
/// stamped-on default that every job would share (`WS1` finding J16).
/// `lastRunAt` is `null` when the job has never run instead of an empty
/// string standing in for "never ran".
fn dagster_pipeline_row(j: &DgJob, last: Option<&DgRun>) -> Value {
    json!({
        "id": j.name,
        "name": j.name,
        "kind": "batch",
        "status": last.map_or("unknown", |r| map_run_status(&r.status)),
        "owner": TENANT_OWNER.as_str(),
        "source": Value::Null,
        "target": Value::Null,
        "schedule": schedule_label(j),
        "lastRunAt": last
            .and_then(|r| r.start_time)
            .map_or(Value::Null, |t| Value::String(iso_from_unix_seconds(t))),
        "slaOk": Value::Null,
        "freshnessLagSeconds": Value::Null,
    })
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
        // WS1 task 1.2: Dagster's run record carries no row counts. WS4 reads
        // them from step materializations; until then null says "not
        // measured" rather than 0 claiming "measured none".
        "processed": Value::Null,
        "accepted": Value::Null,
        "rejected": Value::Null,
        "retried": Value::Null,
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
                // Same as run_to_json: see the note there.
                "processed": Value::Null,
                "accepted": Value::Null,
                "rejected": Value::Null,
                "retried": Value::Null,
                // A just-launched run's cost is not yet knowable.
                "costUnits": Value::Null,
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
    incremental_column: Option<String>,
    #[serde(default)]
    transforms: Vec<String>,
    #[serde(default)]
    fbic_enabled: bool,
    target_zone: String,
    target_table: String,
    schedule: String,
    #[serde(default)]
    owner: Option<String>,
}

/// `POST /api/pipelines` — author a new pipeline definition. Returns 201.
///
/// Every entry of `body.transforms` is validated through
/// `transform_grammar::parse_transform` BEFORE `pipelines::create_pipeline`
/// is ever called (WS4 item D3) — an invalid transform is refused with 400
/// and writes nothing to `pipeline_definition`, rather than being stored
/// and only rejected at execution time.
///
/// # Errors
///
/// 400 on a malformed body, or if any `transforms` entry does not parse
/// against the grammar (the 400 names which entry by index, never the
/// entry's own text — the untrusted payload itself is not echoed back);
/// 409 if the name is taken; 503/500 as above.
pub async fn create(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<pipelines::Pipeline>)> {
    let body: CreatePipelineBody = parse_body(&body)?;
    for (index, transform) in body.transforms.iter().enumerate() {
        crate::transform_grammar::parse_transform(transform).map_err(|err| {
            ApiError::BadRequest(format!("invalid transform at transforms[{index}]: {err}"))
        })?;
    }
    let input = CreatePipelineInput {
        name: body.name,
        kind: body.kind,
        source_zone: body.source_zone,
        source_table: body.source_table,
        incremental_column: body.incremental_column,
        transforms: body.transforms,
        fbic_enabled: body.fbic_enabled,
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
/// The console's only caller of this route, the pipelines page's Agentic
/// Builder dialog, was removed in `WS1` task 1.11 (it produced a draft
/// "from mock agent phases", not a real generation). The route stays
/// registered, unchanged, for `WS7`'s copilot to call instead.
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
        incremental_column: None,
        transforms: Vec::new(),
        fbic_enabled: false,
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
            (StatusCode::OK, ApiJson(schedule_mutation_body(job, paused))).into_response()
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

/// Build the pause/resume response body for a `Dagster`-backed pipeline.
/// Same reasoning as [`dagster_pipeline_row`]: no lineage, `SLA`, freshness,
/// or last-run time is known here, so all five are `null` rather than the
/// literal `true`/`""`/`0` this endpoint used to return unconditionally.
fn schedule_mutation_body(job: &DgJob, paused: bool) -> Value {
    json!({
        "id": job.name,
        "name": job.name,
        "kind": "batch",
        "status": if paused { "paused" } else { "ready" },
        "owner": TENANT_OWNER.as_str(),
        "source": Value::Null,
        "target": Value::Null,
        "schedule": schedule_label(job),
        "lastRunAt": Value::Null,
        "slaOk": Value::Null,
        "freshnessLagSeconds": Value::Null,
    })
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
    // The run being cancelled is the SAME run whose real start time is
    // already knowable via `pipeline_run_status` (cheaper than
    // `run_steps`, since only the run's own `startTime` is needed here,
    // not its per-step materializations). A lookup failure or a run
    // Dagster reports as not-yet-started both fall through to `None` —
    // reporting the terminate mutation's own outcome must not be blocked
    // on an unrelated status query.
    let started_at = state
        .dagster
        .pipeline_run_status(&run_id)
        .await
        .ok()
        .flatten()
        .and_then(|info| info.start_time)
        .map(iso_from_unix_seconds);
    match state.dagster.terminate_run(&run_id).await {
        Ok(outcome) if outcome.error.is_none() => (
            StatusCode::OK,
            ApiJson(run_mutation_body(
                &run_id,
                "cancelled",
                started_at.as_deref(),
            )),
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
            // The NEW run's id, just launched: `Dagster` has not populated
            // its `startTime` yet at the instant this handler returns, so
            // `None` is the honest value here — looking up the PARENT
            // run's start time would report the wrong run's timestamp.
            (
                StatusCode::OK,
                ApiJson(run_mutation_body(&new_id, "running", None)),
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
fn run_mutation_body(run_id: &str, status: &str, started_at: Option<&str>) -> Value {
    json!({
        "id": run_id,
        "pipelineId": "",
        "status": status,
        // A timestamp the backend did not observe is a fabricated
        // measurement (WS4 item G1): `started_at` is `None` unless the
        // caller actually looked up the run's real start time, and the
        // field stays `null` rather than being defaulted to `now()`.
        "startedAt": started_at,
        // Same as run_to_json: see the note there.
        "processed": Value::Null,
        "accepted": Value::Null,
        "rejected": Value::Null,
        "retried": Value::Null,
        // A cancelled or retried run has consumed real compute (finding J4);
        // reporting its cost as 0 would understate it, so it stays unmeasured.
        "costUnits": Value::Null,
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
    fn run_to_json_emits_null_for_untracked_counters() {
        let v = run_to_json(&run("j", "SUCCESS", Some(1.0), Some(61.0)), "p1");

        // Dagster's run record carries no row counts. Emitting 0 would read as
        // "this run processed nothing", which is a different claim from "we
        // did not measure it".
        for key in ["processed", "accepted", "rejected", "retried"] {
            assert!(v[key].is_null(), "{key} must be null, got {}", v[key]);
        }
        // costUnits IS derived from the run's own duration, so it stays real.
        assert!(!v["costUnits"].is_null());
    }

    #[test]
    fn run_mutation_body_emits_null_for_unmeasured_fields() {
        let v = run_mutation_body("r1", "cancelled", None);

        for key in ["processed", "accepted", "rejected", "retried", "costUnits"] {
            assert!(v[key].is_null(), "{key} must be null, got {}", v[key]);
        }
    }

    #[test]
    fn run_mutation_body_reports_a_real_started_at_when_known_and_null_otherwise() {
        let v = run_mutation_body("r1", "cancelled", None);
        assert!(
            v["startedAt"].is_null(),
            "no real start time was supplied; must not fabricate now()"
        );

        let v2 = run_mutation_body("r1", "running", Some("2026-01-01T00:00:00.000Z"));
        assert_eq!(v2["startedAt"], "2026-01-01T00:00:00.000Z");
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
    fn dagster_pipeline_rows_never_invent_freshness_sla_or_lineage() {
        let never_run = DgJob {
            name: "bronze_maintenance_job".to_owned(),
            schedules: vec![],
        };
        let ran_job = DgJob {
            name: "silver_orders".to_owned(),
            schedules: vec![],
        };
        let last = run("silver_orders", "SUCCESS", Some(100.0), Some(160.0));

        let never_run_row = dagster_pipeline_row(&never_run, None);
        let ran_row = dagster_pipeline_row(&ran_job, Some(&last));

        for row in [&never_run_row, &ran_row] {
            assert!(row["freshnessLagSeconds"].is_null());
            assert!(row["slaOk"].is_null());
            assert!(row["source"].is_null());
            assert!(row["target"].is_null());
        }
        assert!(
            never_run_row["lastRunAt"].is_null(),
            "a job with no run must not report a lastRunAt"
        );
        assert!(
            ran_row["lastRunAt"].is_string(),
            "a job with a real run keeps its real start time"
        );
    }

    #[test]
    fn schedule_mutation_body_reports_unknowns_as_null() {
        let job = DgJob {
            name: "j".to_owned(),
            schedules: vec![],
        };
        let body = schedule_mutation_body(&job, true);
        for key in [
            "source",
            "target",
            "slaOk",
            "freshnessLagSeconds",
            "lastRunAt",
        ] {
            assert!(body[key].is_null(), "{key} must be null, got {}", body[key]);
        }
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

    /// WS4 item D3 — `POST /api/pipelines` validates every `transforms`
    /// entry through `transform_grammar::parse_transform` before writing
    /// anything, exercised against a real Postgres so the "writes nothing"
    /// half of the assertion is real, not just an in-memory claim.
    mod create_route {
        use lakehouse_test_support as _;

        use crate::config::Config;
        use crate::state::AppState;

        use super::super::*;

        fn database_url_for(pool: &sqlx::PgPool) -> String {
            let options = pool.connect_options();
            format!(
                "postgres://{}:postgres@{}:{}/{}",
                options.get_username(),
                options.get_host(),
                options.get_port(),
                options
                    .get_database()
                    .expect("#[sqlx::test] always targets a named database")
            )
        }

        fn state_for(pool: &sqlx::PgPool) -> AppState {
            let mut env = std::collections::HashMap::new();
            env.insert("DATABASE_URL".to_owned(), database_url_for(pool));
            let config = Config::from_map(&env).expect("a valid test Config");
            AppState::new(config)
        }

        /// A `POST /api/pipelines` body with an unparseable `transforms`
        /// entry (`"filter(1=1; DROP TABLE x)"` — not `<ident> <op>
        /// '<literal>'`) returns 400 naming WHICH entry failed (by index,
        /// never the entry's own text — see [`create`]'s doc comment), and
        /// writes NOTHING to `pipeline_definition`: a direct query for a
        /// row with this name finds none.
        #[sqlx::test(migrations = "../../migrations")]
        async fn create_pipeline_rejects_an_unparseable_transform_with_400(pool: sqlx::PgPool) {
            let state = state_for(&pool);
            let name = format!("create-route-test-{}", uuid::Uuid::new_v4());
            let body = Bytes::from(
                serde_json::to_vec(&json!({
                    "name": name,
                    "kind": "batch",
                    "sourceZone": "bronze",
                    "sourceTable": "t",
                    "transforms": ["filter(1=1; DROP TABLE x)"],
                    "targetZone": "silver",
                    "targetTable": "t",
                    "schedule": "manual",
                }))
                .expect("serialize"),
            );

            let err = create(State(state), body).await.unwrap_err();
            assert_eq!(err.0.status(), 400);
            let message = err.0.to_string();
            assert!(
                message.contains("transforms[0]"),
                "error must name which transform failed by index: {message:?}"
            );
            assert!(
                !message.contains("DROP TABLE"),
                "error must not echo the untrusted payload text: {message:?}"
            );

            let row: Option<(String,)> =
                sqlx::query_as("SELECT id FROM pipeline_definition WHERE name = $1")
                    .bind(&name)
                    .fetch_optional(&pool)
                    .await
                    .expect("query should succeed");
            assert!(
                row.is_none(),
                "an invalid transform must write nothing to pipeline_definition"
            );
        }
    }
}
