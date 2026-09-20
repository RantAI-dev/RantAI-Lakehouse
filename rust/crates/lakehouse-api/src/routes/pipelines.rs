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
use lakehouse_store::pipelines::{self, CreatePipelineInput};
use lakehouse_store::{PgPool, StoreError};
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
    // Dagster is optional. It used to be required, so a stack with no
    // orchestrator — which is every local one — answered 503 and showed an
    // empty page, even with pipelines sitting in Postgres. Losing the
    // orchestrator's jobs is a gap the response states; losing the whole
    // list is a broken page.
    let orchestrator = tokio::try_join!(dagster.list_jobs_with_schedules(), dagster.list_runs(100));
    let (jobs, runs, orchestrator_error) = match orchestrator {
        Ok((jobs, runs)) => (jobs, runs, None),
        Err(err) => {
            tracing::warn!(%err, "pipelines: no orchestrator jobs (Dagster unreachable)");
            (Vec::new(), Vec::new(), Some(js_error(err)))
        }
    };

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
                // Nothing measures a job's output freshness here. This was
                // 0, which the console read as "Fresh · 0s" for every row.
                "freshnessLagSeconds": Value::Null,
                // Which half of the list a row came from. The two have
                // different id spaces and different capabilities — only an
                // orchestrator job can be triggered or retried — and the
                // console could not tell them apart.
                "origin": ORIGIN_ORCHESTRATOR,
            })
        })
        .collect();
    if let Some(pg) = pg {
        let authored = pipelines::list_pipelines(pg).await?;
        pipelines.extend(authored.iter().filter_map(|p| {
            let mut value = serde_json::to_value(p).ok()?;
            value.as_object_mut()?.insert(
                "origin".to_owned(),
                Value::String(ORIGIN_AUTHORED.to_owned()),
            );
            Some(value)
        }));
    }
    Ok(json!({ "pipelines": pipelines, "orchestratorError": orchestrator_error }))
}

/// A pipeline that exists as a job in the orchestrator: it can be
/// triggered, its runs can be cancelled and retried, and its history is
/// real.
const ORIGIN_ORCHESTRATOR: &str = "orchestrator";

/// A pipeline authored in the console and stored in Postgres. There is no
/// engine behind it yet, so it has no runs and cannot be triggered.
const ORIGIN_AUTHORED: &str = "authored";

/// `GET /api/pipelines/{id}` — one pipeline with its runs.
///
/// New. The console used to build this itself: it fetched the whole list,
/// found the row, fetched the runs, and then invented the three fields the
/// detail page shows — a description ("Job Dagster: refresh lakehouse
/// Bronze→Silver→Gold"), a three-node Bronze/Silver/Gold graph marked
/// "completed", and a config summary — identically for every pipeline,
/// whatever it actually did. Everything here comes from what is stored.
pub async fn get_one(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match detail_body(&state, &id).await {
        Ok(Some(body)) => (StatusCode::OK, ApiJson(body)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            ApiJson(json!({ "error": format!("pipeline {id} not found") })),
        )
            .into_response(),
        Err(err) => crate::error::ApiRejection(err).into_response(),
    }
}

async fn detail_body(state: &AppState, id: &str) -> Result<Option<Value>, ApiError> {
    // An authored pipeline is answered from Postgres alone: it has no job
    // in the orchestrator, so asking for its runs would only produce a
    // failure to explain away.
    if let Some(pg) = state.pg.as_deref() {
        if let Some(p) = pipelines::get_pipeline(pg, id)
            .await
            .map_err(|err| ApiError::Internal(err.to_string()))?
        {
            let mut value =
                serde_json::to_value(&p).map_err(|err| ApiError::Internal(err.to_string()))?;
            let object = value
                .as_object_mut()
                .ok_or_else(|| ApiError::Internal("pipeline is not an object".to_owned()))?;
            object.insert("origin".to_owned(), json!(ORIGIN_AUTHORED));
            object.insert("graph".to_owned(), authored_graph(&p));
            object.insert("configSummary".to_owned(), authored_config(&p));
            object.insert("runs".to_owned(), json!([]));
            object.insert(
                "runsUnavailable".to_owned(),
                json!("This pipeline is authored in the console and has no orchestrator job yet, so it has never run."),
            );
            return Ok(Some(value));
        }
    }

    // Otherwise it should be an orchestrator job. Its identity comes from
    // the same list the index builds, so the two never disagree.
    let jobs = match state.dagster.list_jobs_with_schedules().await {
        Ok(jobs) => jobs,
        Err(err) => {
            return Err(ApiError::Unavailable(format!(
                "orchestrator unreachable: {}",
                js_error(err)
            )));
        }
    };
    let Some(job) = jobs.iter().find(|j| j.name == id) else {
        return Ok(None);
    };
    let runs_body = runs_body(state, id).await;
    let runs = runs_body.get("runs").cloned().unwrap_or_else(|| json!([]));
    let last_status = runs
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(|r| r.get("status"))
        .cloned()
        .unwrap_or(Value::String("unknown".to_owned()));
    Ok(Some(json!({
        "id": job.name,
        "name": job.name,
        "kind": "batch",
        "status": last_status,
        "owner": TENANT_OWNER.as_str(),
        "source": TENANT_SOURCE.as_str(),
        "target": TARGET,
        "schedule": schedule_label(job),
        "lastRunAt": runs
            .as_array()
            .and_then(|rows| rows.first())
            .and_then(|r| r.get("startedAt"))
            .cloned()
            .unwrap_or(Value::String(String::new())),
        "slaOk": true,
        "freshnessLagSeconds": Value::Null,
        "origin": ORIGIN_ORCHESTRATOR,
        // The orchestrator's job list carries no op graph, so there is
        // nothing truthful to draw. The console says so rather than
        // showing a diagram of three steps nobody verified.
        "graph": [],
        "configSummary": [
            { "key": "engine", "value": "Dagster" },
            { "key": "schedule", "value": schedule_label(job) },
        ],
        "runs": runs,
        "runsUnavailable": runs_body.get("unavailable").cloned().unwrap_or(Value::Null),
    })))
}

/// The shape an authored pipeline actually describes: what it reads, and
/// what it writes. No per-node status — nothing here executes it, so
/// nothing knows.
fn authored_graph(p: &pipelines::Pipeline) -> Value {
    let mut nodes = Vec::new();
    if let Some(connector) = &p.connector_id {
        nodes.push(json!({ "id": "connector", "label": connector, "kind": "connector" }));
    }
    nodes.push(json!({ "id": "source", "label": p.source, "kind": "source" }));
    nodes.push(json!({ "id": "target", "label": p.target, "kind": "target" }));
    Value::Array(nodes)
}

/// Configuration as stored, limited to what the detail page does not
/// already show in its own fields — kind, schedule, owner, source, target
/// and connector are up there, and repeating them read as a bug.
fn authored_config(p: &pipelines::Pipeline) -> Value {
    let mut items: Vec<Value> = Vec::new();
    if let Some(column) = &p.incremental_column {
        items.push(json!({ "key": "incremental column", "value": column }));
    }
    if !p.transforms.is_empty() {
        items.push(json!({ "key": "transforms", "value": p.transforms.join(", ") }));
    }
    if p.fbic_enabled {
        items.push(json!({ "key": "FBIC", "value": "enabled" }));
    }
    Value::Array(items)
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
///
/// An unreachable orchestrator answers 200 with an empty list and
/// `unavailable` set, not 503: "there are no runs" and "nobody could be
/// asked" are different answers, and the page that shows them should be
/// able to say which one it got.
pub async fn runs(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let body = runs_body(&state, &id).await;
    (StatusCode::OK, ApiJson(body)).into_response()
}

async fn runs_body(state: &AppState, id: &str) -> Value {
    match state.dagster.list_runs_for_job(id, 30).await {
        Ok(runs) => json!({
            "runs": runs.iter().map(|r| run_to_json(r, id)).collect::<Vec<_>>(),
            "unavailable": Value::Null,
        }),
        Err(err) => {
            tracing::warn!(%err, "pipeline runs: orchestrator unreachable");
            json!({ "runs": [], "unavailable": js_error(err) })
        }
    }
}

fn run_to_json(r: &DgRun, pipeline_id: &str) -> Value {
    json!({
        "id": r.run_id,
        "pipelineId": pipeline_id,
        "status": map_run_status(&r.status),
        "startedAt": r.start_time.map_or_else(String::new, iso_from_unix_seconds),
        "endedAt": r.end_time.map(iso_from_unix_seconds),
        // Row counts are not reported by the orchestrator's run list, and
        // these were zeros presented as measurements. The cost field was
        // worse: it carried the run's duration in seconds under the name
        // "cost", which the console then formatted as currency-like units.
        "processed": Value::Null,
        "accepted": Value::Null,
        "rejected": Value::Null,
        "retried": Value::Null,
        "costUnits": Value::Null,
        "durationSeconds": duration_seconds(r.start_time, r.end_time),
    })
}

/// How long a finished run took, in seconds. `None` while it is still
/// running, or when the orchestrator reported no timestamps.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "run durations here are small, non-negative second counts"
)]
fn duration_seconds(start: Option<f64>, end: Option<f64>) -> Option<i64> {
    match (start, end) {
        (Some(s), Some(e)) if s != 0.0 && e != 0.0 => Some((e - s).round() as i64),
        _ => None,
    }
}

/// `POST /api/pipelines/{id}/trigger` — launch a new run of job `id`.
///
/// This mutates live infrastructure (starts a real `Dagster`/`ClickHouse`
/// pipeline run) and is therefore exercised only via the
/// `pipeline-trigger-bad-id` corpus entry, which targets a job name that
/// does not exist so `Dagster` rejects the launch instead of starting one.
pub async fn trigger(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    // An authored pipeline has no job behind it, so launching one is not
    // something that can fail transiently — it is something that cannot
    // happen. It used to be attempted anyway and came back as a 503, which
    // reads as "try again later".
    if id.starts_with("pl-") {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            ApiJson(json!({
                "error": "this pipeline is authored in the console and is not registered \
                          with the orchestrator, so it cannot be run yet",
            })),
        )
            .into_response();
    }
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
    incremental_column: Option<String>,
    #[serde(default)]
    transforms: Vec<String>,
    #[serde(default)]
    fbic_enabled: bool,
    #[serde(default)]
    description: Option<String>,
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
        description: body.description,
        incremental_column: body.incremental_column,
        transforms: body.transforms,
        fbic_enabled: body.fbic_enabled,
    };
    let created = create_named_pipeline(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(created)))
}

/// Create a pipeline, turning a name collision into a sentence that says
/// what collided. The store's own message ("a record with that value
/// already exists") was shown to the user verbatim, which explains
/// nothing about which value or what to do next.
async fn create_named_pipeline(
    pool: &PgPool,
    input: &CreatePipelineInput,
) -> Result<pipelines::Pipeline, ApiError> {
    match pipelines::create_pipeline(pool, input).await {
        Ok(created) => Ok(created),
        Err(StoreError::Conflict) => Err(ApiError::Conflict(format!(
            "a pipeline named \"{}\" already exists — pick a different name",
            input.name
        ))),
        Err(err) => Err(err.into()),
    }
}

/// The `POST /api/pipelines/generate` body. Mirrors `GeneratePipelineInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratePipelineBody {
    instruction: String,
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
    // The draft used to be a name and nothing else: every generated
    // pipeline arrived reading `source_table` → `target_table`, kind
    // "incremental", schedule "On demand", whatever had been asked for.
    // The model now proposes the shape too, and every field it does not
    // give falls back to something derived rather than invented.
    let draft = llm_pipeline_draft(&state, &body.instruction, &body.database).await;
    let fallback_name = derive_pipeline_name(&body.instruction);
    let name = draft
        .as_ref()
        .and_then(|d| d.name.clone())
        .unwrap_or(fallback_name.clone());
    let kind = draft
        .as_ref()
        .and_then(|d| d.kind.clone())
        .filter(|k| PIPELINE_KINDS.contains(&k.as_str()))
        .unwrap_or_else(|| "batch".to_owned());
    let source_table = draft
        .as_ref()
        .and_then(|d| d.source_table.clone())
        .unwrap_or_else(|| fallback_name.clone());
    let target_table = draft
        .as_ref()
        .and_then(|d| d.target_table.clone())
        .unwrap_or_else(|| format!("{fallback_name}_out"));
    let schedule = draft
        .as_ref()
        .and_then(|d| d.schedule.clone())
        .unwrap_or_else(|| "On demand".to_owned());
    let input = CreatePipelineInput {
        name,
        kind,
        source_zone: body.database.clone(),
        source_table,
        target_zone: body.database,
        target_table,
        schedule,
        owner: Some("Agentic Builder".to_owned()),
        // The instruction is kept verbatim as the description: it is the
        // only record of what this pipeline was asked to do.
        description: Some(body.instruction.clone()),
        incremental_column: draft.as_ref().and_then(|d| d.incremental_column.clone()),
        transforms: draft.map(|d| d.transforms).unwrap_or_default(),
        fbic_enabled: false,
    };
    let created = create_named_pipeline(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(created)))
}

/// The kinds a pipeline may be, as the database's `CHECK` constraint
/// allows. A model is free to propose anything; only these are accepted.
const PIPELINE_KINDS: [&str; 4] = ["batch", "incremental", "document", "vector"];

/// What the model proposes for a pipeline. Every field is optional: a
/// missing one falls back to something derived from the instruction, and a
/// failed call falls back entirely, rather than surfacing an LLM outage as
/// a hard error on a form the user has already filled in.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PipelineDraft {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    source_table: Option<String>,
    #[serde(default)]
    target_table: Option<String>,
    #[serde(default)]
    schedule: Option<String>,
    #[serde(default)]
    incremental_column: Option<String>,
    #[serde(default)]
    transforms: Vec<String>,
}

/// Ask the LLM to draft a pipeline for `instruction` against `database`.
async fn llm_pipeline_draft(
    state: &AppState,
    instruction: &str,
    database: &str,
) -> Option<PipelineDraft> {
    use lakehouse_llm::{ChatMessage, ChatOptions, ChatRole};
    let messages = vec![
        ChatMessage {
            role: ChatRole::System,
            content: "You draft data pipelines. Reply with ONLY a JSON object, no prose \
                      and no code fence, with the keys: name (short snake_case), kind \
                      (one of batch, incremental, document, vector), sourceTable, \
                      targetTable, schedule (human readable, e.g. \"Every hour\"), \
                      incrementalColumn (or null), transforms (array of short strings)."
                .to_owned(),
        },
        ChatMessage {
            role: ChatRole::User,
            content: format!("Database: {database}\nRequest: {instruction}"),
        },
    ];
    // Generous enough for a model that thinks before it answers: a reply
    // cut off mid-JSON parses as nothing, and the caller then silently
    // gets the fallback draft.
    let reply = match state
        .llm
        .chat(
            &messages,
            ChatOptions {
                temperature: Some(0.2),
                max_tokens: Some(900),
            },
        )
        .await
    {
        Ok(reply) => reply,
        Err(err) => {
            tracing::warn!(%err, "pipeline draft: LLM unavailable, using the derived draft");
            return None;
        }
    };
    let Some(json) = extract_json_object(&reply) else {
        tracing::warn!(
            reply = %reply.chars().take(200).collect::<String>(),
            "pipeline draft: reply contained no JSON object"
        );
        return None;
    };
    let mut draft: PipelineDraft = match serde_json::from_str(json) {
        Ok(draft) => draft,
        Err(err) => {
            tracing::warn!(
                %err,
                json = %json.chars().take(200).collect::<String>(),
                "pipeline draft: reply was not the expected shape"
            );
            return None;
        }
    };
    // The name is run through the same sanitizer the fallback uses, so
    // whatever the model returns is still a usable identifier.
    draft.name = draft
        .name
        .map(|n| derive_pipeline_name(&n))
        .filter(|n| n != "agentic_pipeline");
    Some(draft)
}

/// The outermost `{...}` in a reply, so a model that wraps its JSON in
/// prose or a code fence is still understood.
fn extract_json_object(reply: &str) -> Option<&str> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    (end > start).then(|| &reply[start..=end])
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
    fn duration_rounds_to_whole_seconds() {
        assert_eq!(duration_seconds(Some(100.0), Some(103.6)), Some(4));
    }

    #[test]
    fn duration_is_unknown_while_a_timestamp_is_missing() {
        // A running job has no end time, and "still running" is not "took
        // zero seconds" — the difference is why this returns an Option.
        assert_eq!(duration_seconds(None, Some(10.0)), None);
        assert_eq!(duration_seconds(Some(10.0), None), None);
        assert_eq!(duration_seconds(None, None), None);
    }

    #[test]
    fn duration_is_unknown_when_a_timestamp_is_the_epoch() {
        assert_eq!(duration_seconds(Some(0.0), Some(10.0)), None);
        assert_eq!(duration_seconds(Some(10.0), Some(0.0)), None);
    }
}
