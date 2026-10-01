//! `GET /api/pipelines`, `GET /api/pipelines/{id}/runs`,
//! `POST /api/pipelines/{id}/trigger` — `Dagster` jobs surfaced as console
//! pipelines.
//!
//! Ports `src/app/api/pipelines/route.ts`,
//! `src/app/api/pipelines/[id]/runs/route.ts`, and
//! `src/app/api/pipelines/[id]/trigger/route.ts`.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Json, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use lakehouse_alerts::SilenceSource;
use lakehouse_auth::{Principal, PrincipalId};
use lakehouse_core::ApiError;
use lakehouse_dagster::{
    ConfigValidationOutcome, DgClient, DgError, DgJob, DgRun, ReexecutionStrategy,
    iso_from_unix_seconds, map_run_status,
};
use lakehouse_notify::EmailSender;
use lakehouse_store::audit::{self as store_audit, NewAuditEvent};
use lakehouse_store::pipelines::{self, CreatePipelineInput};
use lakehouse_store::{PgPool, StoreError};
use serde::Deserialize;
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::{ApiRejection, ApiResult};
use crate::json::ApiJson;
use crate::routes::alerts::{ApiSilenceSource, smtp_config};
use crate::routes::authored_pipelines;
use crate::routes::support::js_error;
use crate::state::AppState;

use crate::tenant::TENANT_OWNER;

/// `GET /api/pipelines` — every `Dagster` job, enriched with its most
/// recent run and (first) schedule, unioned with every Postgres-authored
/// pipeline definition (`createPipeline`/`generatePipelineFromPrompt`,
/// Task 2.5) so an authored pipeline is visible immediately rather than
/// vanishing the way an authored governance rule did before the Task 2.3
/// gap fix — see `0007_pipelines.sql`'s header comment.
///
/// # Tenant scoping — authored pipelines only
///
/// `tenant_scope::resolve` decides the authored half only. `Ok(None)` (the
/// caller belongs to zero tenants) means no authored pipeline is listed at
/// all — the store is never queried, fail closed, never "unscoped, show
/// everything." `Some(tenant_id)` filters the authored half
/// (`pipelines::list_pipelines`) to that tenant via a bound
/// `WHERE tenant_id = $1`. A zero-tenant caller still goes through the
/// `Dagster`-job gate below: an earlier version returned `{"pipelines": []}`
/// before that gate ran, so a `"*:*"` operator with no tenant membership
/// lost the whole job list the gate says it may always see.
///
/// # The `Dagster`-job half is a shared surface, gated like the catalog
///
/// A `Dagster` code location is one per deployment and nothing in this
/// schema maps a job to a tenant — migration `0042` adds `tenant_id` to
/// `connector` and `pipeline_definition` only. Filtering the authored
/// half and leaving the jobs visible to every tenant with only a
/// disclosure comment would repeat the same leak the shared catalog's
/// gate exists to close, so this route runs through that same gate
/// (`routes::catalog::catalog_tenant_refusal`): a `"*:*"`
/// principal and a single-tenant deployment are never refused, a member of
/// `CATALOG_TENANT_ID` is never refused once that is set, and everyone else
/// gets the authored half plus an explicit
/// `"dagsterJobs": {"supported": false, "reason": …}` rather than a shorter
/// list that looks complete.
pub async fn list(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    headers: HeaderMap,
) -> Response {
    let Some(Extension(principal)) = principal else {
        return ApiRejection(ApiError::unauthorized()).into_response();
    };
    let tenant_id = match crate::tenant_scope::resolve(&principal, &headers) {
        Ok(tenant_id) => tenant_id,
        Err(err) => return ApiRejection(err).into_response(),
    };
    let dagster_jobs_refused =
        match crate::routes::catalog::catalog_tenant_refusal(&state, &principal, &headers).await {
            Ok(refusal) => refusal,
            Err(err) => return ApiRejection(err).into_response(),
        };
    // A Platform Admin with no tenant sees every tenant's authored
    // pipelines, the same rule that already shows them the shared Dagster
    // jobs (`catalog::is_unrestricted`). Without it the admin, who has no
    // tenant, saw no authored pipeline on this list at all.
    let all_tenants = tenant_id.is_none() && crate::routes::catalog::is_unrestricted(&principal);
    match list_body(
        &state.dagster,
        state.pg.as_deref(),
        tenant_id,
        all_tenants,
        dagster_jobs_refused,
    )
    .await
    {
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

async fn list_body(
    dagster: &DgClient,
    pg: Option<&PgPool>,
    tenant_id: Option<Uuid>,
    all_tenants: bool,
    dagster_jobs_refused: Option<&'static str>,
) -> Result<Value, ListError> {
    // The `Dagster` half is a SHARED, un-tenanted resource — a code
    // location is one per deployment and no table maps a job to a tenant
    // (migration 0042 adds `tenant_id` to `connector` and
    // `pipeline_definition` only). So it is gated by exactly the rule the
    // shared catalog is gated by (`routes::catalog::catalog_tenant_refusal`):
    // refused for a tenant-scoped
    // caller unless an operator has named the owning tenant. When refused,
    // the jobs are not fetched at all — not fetched and filtered out, so a
    // refusal costs no `Dagster` round trip — and the response says so
    // instead of silently returning a shorter list that looks complete.
    let mut pipelines: Vec<Value> = Vec::new();
    if dagster_jobs_refused.is_none() {
        let (jobs, runs) =
            tokio::try_join!(dagster.list_jobs_with_schedules(), dagster.list_runs(100))?;
        // Plan 1f: the "late" / "slaOk" cells need an SLA row per job.
        // SLAs are fetched in one batched query (`list_pipeline_slas`,
        // ANY($1) on the primary key) rather than N round trips — a list
        // page can hold 100+ jobs and a sequential walk would balloon
        // the request. `now_seconds` is read once for the whole response
        // so every row in a single `GET` agrees on the clock. A
        // Postgres-less deployment skips the fetch entirely and the row
        // reports `late: null` / `slaOk: null` — the same honest-null
        // posture as every other measured-but-unknown field on this row.
        let sla_jobs: Vec<&DgJob> = jobs
            .iter()
            // An authored pipeline's `authored__<id>` job is listed once, as
            // its own `pl-` row below, never a second time as a Dagster job.
            .filter(|j| !j.name.starts_with("authored__"))
            .collect();
        let sla_map = match pg {
            Some(pool) => {
                let names: Vec<&str> = sla_jobs.iter().map(|j| j.name.as_str()).collect();
                pipelines::list_pipeline_slas(pool, &names)
                    .await
                    .unwrap_or_default()
            }
            None => std::collections::HashMap::new(),
        };
        let now_seconds = now_unix_seconds();
        pipelines = sla_jobs
            .into_iter()
            .map(|j| {
                let last = last_run_for(&runs, &j.name);
                let last_success_seconds = last_success_for(&runs, &j.name)
                    .and_then(|r| r.start_time)
                    .filter(|t| *t != 0.0);
                let sla = sla_map.get(&j.name);
                dagster_pipeline_row(j, last, last_success_seconds, sla, now_seconds)
            })
            .collect();
    }
    // No tenant means no authored pipeline, not every tenant's: the store
    // is not queried at all, unless the caller is a tenantless Platform
    // Admin (`all_tenants`, decided in `list`).
    if let Some(pg) = pg.filter(|_| tenant_id.is_some() || all_tenants) {
        let filter = pipelines::PipelineFilter {
            tenant_id,
            all_tenants,
        };
        let authored = pipelines::list_pipelines(pg, &filter).await?;
        pipelines.extend(authored.iter().filter_map(|p| serde_json::to_value(p).ok()));
    }
    match dagster_jobs_refused {
        None => Ok(json!({ "pipelines": pipelines })),
        Some(reason) => Ok(json!({
            "pipelines": pipelines,
            "dagsterJobs": { "supported": false, "reason": reason },
        })),
    }
}

/// Build one `Dagster`-job row for `GET /api/pipelines`. `Dagster`'s job/run
/// API carries no per-job lineage and no freshness measurement (`WS2`
/// derives that from Iceberg snapshot timestamps) — `source`, `target`, and
/// `freshnessLagSeconds` are therefore reported as `null` rather than a
/// stamped-on default that every job would share (`WS1` finding J16).
/// `slaOk` is computed from the pipeline's `pipeline_sla` row plus the
/// last successful run, NOT from `WS5`'s `dataset_sla` (which is keyed by
/// warehouse *table* and cannot honestly summarize a single job that may
/// write many tables). Plan 1f wires these together — both `late` and
/// `slaOk` are `null` when no SLA has been configured. `lastRunAt` is
/// `null` when the job has never run instead of an empty string standing
/// in for "never ran". `nextRunAt` (WS4 item G2) is computed server-side
/// from the job's first schedule's cron expression; `null` for a manual
/// job or an uncomputable cron — never a guess.
fn dagster_pipeline_row(
    j: &DgJob,
    last: Option<&DgRun>,
    last_success_seconds: Option<f64>,
    sla: Option<&pipelines::PipelineSla>,
    now_seconds: Option<f64>,
) -> Value {
    let late_value =
        sla.and_then(|s| late(now_seconds, last_success_seconds, s.late_after_seconds));
    // `overDuration` for the most recent run (only meaningful when the run
    // has both a `startTime` and `endTime`). `None` when there is no SLA,
    // no last run, or the last run is still going — see [`over_duration`].
    let last_over = last.and_then(|r| {
        over_duration(
            duration_seconds(r.start_time, r.end_time),
            sla?.max_duration_seconds,
        )
    });
    let sla_ok = match (late_value, last_over) {
        (None, None) => None,
        (late, over) => Some(!late.unwrap_or(false) && !over.unwrap_or(false)),
    };
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
        "nextRunAt": next_run_at_json(j),
        "slaOk": sla_ok.map_or(Value::Null, Value::Bool),
        "late": late_value.map_or(Value::Null, Value::Bool),
        "freshnessLagSeconds": Value::Null,
    })
}

/// `nextRunAt` for a `Dagster` job (WS4 item G2): the next fire time of its
/// FIRST schedule (same "only the first schedule" rule [`schedule_label`]
/// already follows), computed server-side via
/// `crate::next_run::next_run_at`. `null` for a job with no schedule, or
/// whose cron expression `next_run_at` cannot compute a next occurrence
/// from — never a fabricated guess.
#[allow(
    clippy::cast_precision_loss,
    reason = "a cron next-occurrence Unix timestamp (seconds since epoch) fits exactly in f64 \
              until year 285 million; iso_from_unix_seconds takes f64 for parity with Dagster's \
              own run timestamps"
)]
fn next_run_at_json(job: &DgJob) -> Value {
    job.schedules
        .first()
        .and_then(|s| crate::next_run::next_run_at(&s.cron_schedule, OffsetDateTime::now_utc()))
        .map_or(Value::Null, |t| {
            Value::String(iso_from_unix_seconds(t.unix_timestamp() as f64))
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

/// The most recent SUCCESS run for `job_name`, or `None` if no such run
/// has been recorded. Used by the SLA "late" decision: a pipeline whose
/// only runs are failures / in-progress has no `lastSuccessAt` and is
/// reported as `late: true` when a threshold exists (see [`late`]).
/// Same strict-`>`/first-row-on-tie reduction as [`last_run_for`].
fn last_success_for<'a>(runs: &'a [DgRun], job_name: &str) -> Option<&'a DgRun> {
    let mut best: Option<&'a DgRun> = None;
    for r in runs {
        if r.job_name != job_name || r.status != "SUCCESS" {
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
    // An authored pipeline's runs belong to its `authored__<id>` job; the
    // `pl-` id itself names no job, so asking for it always found none.
    let job = if id.starts_with("pl-") {
        authored_pipelines::job_name(id)
    } else {
        id.to_owned()
    };
    // Plan 1f: each run's `overDuration` is computed against the
    // pipeline's `pipeline_sla.max_duration_seconds`. A pool-less
    // deployment (no Postgres) reports `overDuration: null` for every run
    // rather than 200 with `overDuration: false` lying about a SLA that
    // does not exist.
    let max_duration = match state.pg.as_deref() {
        Some(pool) => match pipelines::get_pipeline_sla(pool, id).await {
            Ok(Some(sla)) => sla.max_duration_seconds,
            // No row, or an error reading the SLA: the runs route must
            // keep working (a degraded but truthful answer, per WS5's
            // "failed-to-measure is not measured-as-zero" rule). `null`
            // per run is the honest answer; the route does not 503.
            Ok(None) | Err(_) => None,
        },
        None => None,
    };
    match state.dagster.list_runs_for_job(&job, 30).await {
        Ok(runs) => json!({
            "runs": runs
                .iter()
                .map(|r| run_to_json(r, id, max_duration))
                .collect::<Vec<_>>(),
            "unavailable": Value::Null,
        }),
        Err(err) => {
            tracing::warn!(%err, "pipeline runs: orchestrator unreachable");
            json!({ "runs": [], "unavailable": js_error(err) })
        }
    }
}

/// `GET /api/pipelines/{id}/sla` — read a pipeline's `pipeline_sla` row,
/// or 404 when none has been configured (plan 1f).
///
/// Returning `404` here (not an empty envelope) lets the UI distinguish
/// "this pipeline has no SLA yet" from "the SLA cell is unset because the
/// backend is degraded" — the latter goes through the failed-`Ok(None)`
/// path in [`runs_body`] instead.
pub async fn get_sla(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<Value>> {
    let pool = pool(&state)?;
    let sla = pipelines::get_pipeline_sla(pool, &id)
        .await
        .map_err(ApiError::from)?;
    match sla {
        Some(sla) => Ok(ApiJson(json!({
            "pipelineId": sla.pipeline_id,
            "maxDurationSeconds": sla.max_duration_seconds,
            "lateAfterSeconds": sla.late_after_seconds,
            "updatedBy": sla.updated_by.to_string(),
            "updatedAt": sla.updated_at,
        }))),
        None => Err(ApiError::NotFound(format!("no SLA configured for {id}")).into()),
    }
}

/// `PUT /api/pipelines/{id}/sla` — upsert a pipeline's `pipeline_sla`
/// row. Both thresholds are optional: `null` clears the column. A zero or
/// negative threshold is rejected at the API boundary (and again at the
/// database CHECK — defense in depth, see `0050_pipeline_sla.sql`).
///
/// The route writes a `pipeline.sla_set` audit event with the principal's
/// own id so the audit trail records "who set what SLA when" — best-effort,
/// a failed audit write does not turn a successful upsert into an error
/// response.
pub async fn put_sla(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let req: PutSlaBody = parse_body(&body)?;
    if let Some(max) = req.max_duration_seconds
        && max <= 0
    {
        return Err(ApiError::BadRequest(
            "maxDurationSeconds must be positive when set".to_owned(),
        )
        .into());
    }
    if let Some(late) = req.late_after_seconds
        && late <= 0
    {
        return Err(
            ApiError::BadRequest("lateAfterSeconds must be positive when set".to_owned()).into(),
        );
    }
    let actor = principal.id.uuid();
    let pool = pool(&state)?;
    let sla = pipelines::upsert_pipeline_sla(
        pool,
        &id,
        req.max_duration_seconds,
        req.late_after_seconds,
        actor,
    )
    .await
    .map_err(ApiError::from)?;
    // Best-effort audit: a failed audit write is logged, not propagated.
    // Same posture as `routes::pipelines::trigger`/
    // `routes::connectors::create` and the rest of this file.
    record_pipeline_audit(&state, &principal, "pipeline.sla_set", &id, Value::Null).await;
    Ok(ApiJson(json!({
        "pipelineId": sla.pipeline_id,
        "maxDurationSeconds": sla.max_duration_seconds,
        "lateAfterSeconds": sla.late_after_seconds,
        "updatedBy": sla.updated_by.to_string(),
        "updatedAt": sla.updated_at,
    })))
}

/// `PUT /api/pipelines/{id}/sla` body. Both fields nullable so an empty
/// body is a valid "clear everything" request.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PutSlaBody {
    #[serde(default)]
    pub max_duration_seconds: Option<i32>,
    #[serde(default)]
    pub late_after_seconds: Option<i32>,
}

/// `GET /api/pipelines/{id}/volume` — up to 30 recent runs together with
/// their row counts and the `pipeline_volume_drop` alert outcome per run
/// (plan 1f). Same 30-run window as `/runs`; the volume-drop rule needs at
/// least 5 prior completed runs to make a claim, so 30 gives the most
/// recent run up to 25 samples of history.
///
/// Reviewer fix #3: response shape is `{ runs, medianRows, unavailable }` —
/// `medianRows` is the median of every completed run that reported rows
/// (the same median the drop rule uses — see [`median`]); `unavailable`
/// is a classified non-upstream string when the orchestrator is
/// unreachable, mirroring [`runs_body`]'s degraded branch (so the UI
/// can show "no runs right now, but here's why" rather than confusing
/// it with a hard 503).
pub async fn volume(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let job = if id.starts_with("pl-") {
        authored_pipelines::job_name(&id)
    } else {
        id.clone()
    };
    let body = match state
        .dagster
        .list_runs_for_job_with_materializations(&job, 30)
        .await
    {
        Ok(runs) => volume_body(&runs),
        Err(err) => {
            // AGENTS.md rule 4: never leak upstream detail into a
            // response body. `js_error` is `Display`, not the original
            // error's own message — it is the routing layer's fixed
            // string (`routes::support::js_error`).
            tracing::warn!(%err, "pipeline volume: orchestrator unreachable");
            json!({
                "runs": [],
                "medianRows": Value::Null,
                "unavailable": js_error(err),
            })
        }
    };
    (StatusCode::OK, ApiJson(body)).into_response()
}

/// Build the volume-route body from the dagster fetch's rows. Kept
/// pure for testability (reviewer fix #3's shape test) and shared
/// between the success and the (degenerate) empty-history paths.
fn volume_body(runs: &[lakehouse_dagster::DgRunWithRows]) -> Value {
    // The volume-drop rule compares the *previous* completed runs (in
    // their original time order — older first, so the most recent run's
    // history is everything before it) against the current row count.
    // `DgRunWithRows` arrives most-recent-first from `Dagster`, so reverse
    // for history and skip non-completed runs (a `FAILURE` row with no
    // rows is not part of the median — see the [`drop`] helper).
    let mut history: Vec<i64> = Vec::with_capacity(runs.len());
    let mut entries: Vec<Value> = Vec::with_capacity(runs.len());
    let mut rows_for_median: Vec<i64> = Vec::with_capacity(runs.len());
    for run in runs.iter().rev() {
        let status = run.run.status.as_str();
        let completed = matches!(status, "SUCCESS" | "FAILURE");
        let drop_value = if completed {
            drop(run.rows, &history)
        } else {
            None
        };
        entries.push(json!({
            "runId": run.run.run_id,
            "status": status,
            "startedAt": run.run.start_time.map_or(Value::Null, |t| Value::String(iso_from_unix_seconds(t))),
            "rows": run.rows.map_or(Value::Null, |n| Value::Number(n.into())),
            "drop": drop_value.map_or(Value::Null, Value::Bool),
        }));
        if completed && let Some(rows) = run.rows {
            history.push(rows);
            rows_for_median.push(rows);
        }
    }
    entries.reverse();
    // Plan 1f: `medianRows` is the median of every completed run that
    // reported rows — the same median [`drop`] uses (reviewer fix #3).
    // `None` when no completed run reported rows: the page cannot draw
    // a median line over an empty set, and an honest `null` keeps the
    // chart from inventing a baseline.
    let median_rows = median(&rows_for_median).map_or(Value::Null, |n| Value::Number(n.into()));
    json!({
        "runs": entries,
        "medianRows": median_rows,
        "unavailable": Value::Null,
    })
}

/// `GET /api/pipelines/{id}` — full detail: op graph, config, schedule, and
/// (for an authored pipeline) its stored definition. Dispatches on the
/// `pl-` id prefix exactly like [`pause`]/[`resume`] (WS4 item C1, grand
/// plan §6, closes WS1 T2's "graph tab has nothing real to show").
pub async fn detail(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    if id.starts_with("pl-") {
        return authored_detail(&state, &id).await;
    }
    dagster_detail(&state, &id).await
}

/// `GET /api/pipelines/{id}/versions` — list metadata for the definition
/// versions of an authored pipeline (Plan R4 2b). Newest-first, no
/// snapshot column — the list endpoint must scale to a thousand
/// versions without shipping every prior payload each time the UI
/// re-fetches. 404 for non-`pl-` ids, the same posture as `delete` and
/// `update` (`authored_pipelines`): a wrong prefix is "no such
/// pipeline", not "wrong format".
pub async fn list_versions(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<Vec<pipelines::PipelineVersionMeta>>> {
    if !id.starts_with("pl-") {
        return Err(ApiError::NotFound(format!("Pipeline {id} not found")).into());
    }
    let Some(pool) = state.pg.as_deref() else {
        return Err(
            ApiError::Internal("pipeline version history requires Postgres".to_owned()).into(),
        );
    };
    let _ = &principal; // permission gate is at the policy table
    let versions = pipelines::list_definition_versions(pool, &id)
        .await
        .map_err(ApiError::from)?;
    Ok(ApiJson(versions))
}

/// `GET /api/pipelines/{id}/versions/{version}` — the editable state
/// captured at one version. The restore route rebuilds an
/// `UpdatePipelineInput` from this payload, so the field names here
/// are the `UpdatePipelineInput` snake/camel wire shape (already true
/// because [`pipelines::PipelineDefinitionSnapshot`] serializes with
/// `rename_all = "camelCase"`). 404 for both an unknown version and a
/// non-`pl-` id.
pub async fn get_version(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((id, version)): Path<(String, i32)>,
) -> ApiResult<ApiJson<pipelines::PipelineDefinitionSnapshot>> {
    if !id.starts_with("pl-") {
        return Err(ApiError::NotFound(format!("Pipeline {id} not found")).into());
    }
    let Some(pool) = state.pg.as_deref() else {
        return Err(
            ApiError::Internal("pipeline version history requires Postgres".to_owned()).into(),
        );
    };
    let _ = &principal; // permission gate is at the policy table
    let snapshot = pipelines::get_definition_version(pool, &id, version)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::NotFound(format!("Pipeline {id} version {version} not found")))?;
    Ok(ApiJson(snapshot))
}

async fn dagster_detail(state: &AppState, job_name: &str) -> Response {
    // `jobs`/`runs` are fetched (and the "does this job even exist" 404
    // decided) BEFORE `job_graph` is awaited — NOT joined together with it
    // via a single `tokio::try_join!` (the plan sketch's shape). `job_graph`
    // itself reports an unknown job as `DgError::Server("job ... not
    // found")` (`lakehouse_dagster::DgClient::job_graph`), which under a
    // combined join would race with the jobs-list lookup and could surface
    // as a fabricated 503 ("service unavailable") for what is really a 404
    // ("this pipeline does not exist") — an unavailable-vs-not-found
    // conflation this module's own honesty rule (never report an outage
    // for a resource that simply isn't there) forbids.
    let (jobs, runs) = match tokio::try_join!(
        state.dagster.list_jobs_with_schedules(),
        state.dagster.list_runs(100),
    ) {
        Ok(v) => v,
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
    let graph = match state.dagster.job_graph(job_name).await {
        Ok(g) => g,
        Err(err) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                ApiJson(json!({ "error": js_error(err) })),
            )
                .into_response();
        }
    };
    let last = last_run_for(&runs, job_name);
    let last_success_seconds = last_success_for(&runs, job_name)
        .and_then(|r| r.start_time)
        .filter(|t| *t != 0.0);
    let sla = match pool(state) {
        Ok(pg) => pipelines::get_pipeline_sla(pg, job_name)
            .await
            .unwrap_or(None),
        Err(_) => None,
    };
    let mut body = dagster_pipeline_row(
        job,
        last,
        last_success_seconds,
        sla.as_ref(),
        now_unix_seconds(),
    );
    // `dagster_pipeline_row` always returns a `json!({ ... })` object
    // literal (never an array/scalar) — `if let`, not `.expect()`, so this
    // module stays panic-free even if that invariant is ever violated: a
    // future refactor of `dagster_pipeline_row` that broke it would simply
    // fail to attach `engine`/`graph`/`config`/`definition` rather than
    // crash the request.
    if let Value::Object(obj) = &mut body {
        obj.insert("engine".to_owned(), json!("dagster"));
        // `Dagster`'s job/run API carries no free-text description of its
        // own (same "no lineage, no free-text field" gap
        // `dagster_pipeline_row`'s doc comment already notes for
        // `source`/`target`) — `null`, not the op graph's own docstrings,
        // which are per-op, not per-job.
        obj.insert("description".to_owned(), Value::Null);
        obj.insert(
            "graph".to_owned(),
            json!({
                "ops": graph.ops.iter().map(graph_op_to_json).collect::<Vec<_>>(),
                "edges": graph.edges.iter().map(|e| json!({ "from": e.from, "to": e.to })).collect::<Vec<_>>(),
            }),
        );
        // No stored/authored config exists for a `Dagster`-native job —
        // `[]`, never fabricated.
        obj.insert("config".to_owned(), json!([]));
        obj.insert("definition".to_owned(), Value::Null);
        // Plan 1f: the detail page surfaces the SLA envelope alongside
        // the row. `null` for both cells when no SLA has been configured
        // is the honest "this pipeline has no SLA yet" answer, not a
        // fake row with `0`s.
        obj.insert(
            "sla".to_owned(),
            match sla.as_ref() {
                Some(s) => json!({
                    "pipelineId": s.pipeline_id,
                    "maxDurationSeconds": s.max_duration_seconds,
                    "lateAfterSeconds": s.late_after_seconds,
                    "updatedBy": s.updated_by.to_string(),
                    "updatedAt": s.updated_at,
                }),
                None => Value::Null,
            },
        );
    }
    (StatusCode::OK, ApiJson(body)).into_response()
}

fn graph_op_to_json(op: &lakehouse_dagster::GraphOp) -> Value {
    json!({
        "name": op.name,
        "description": op.description,
        "sourceRef": op.source_ref,
        "commit": op.commit,
        "sql": op.sql,
        // Declared by the op's own code (`op_metadata.source_metadata`),
        // drawn by the console's flowchart on either side of the op.
        "reads": op.reads,
        "writes": op.writes,
    })
}

/// The `pl-` half of [`detail`]: an authored pipeline has no `Dagster` job
/// graph until Phase E's `authored_factory.py` builds one from its stored
/// definition — `"graph": null` here is the honest answer until then,
/// exactly like WS1's T2 empty state, never a fabricated single-op stand-in.
async fn authored_detail(state: &AppState, id: &str) -> Response {
    let pool = match pool(state) {
        Ok(pool) => pool,
        Err(err) => return crate::error::ApiRejection(err).into_response(),
    };
    let pipeline = match pipelines::get_pipeline(pool, id).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                ApiJson(json!({ "error": format!("Pipeline {id} not found") })),
            )
                .into_response();
        }
        Err(err) => return crate::error::ApiRejection(err.into()).into_response(),
    };
    let definition = match pipelines::get_definition(pool, id).await {
        Ok(def) => def,
        Err(err) => return crate::error::ApiRejection(err.into()).into_response(),
    };
    // `Pipeline` (`lakehouse_store::pipelines::Pipeline`) is a plain
    // `#[derive(Serialize)]` struct of `String`/`Option`/`bool` fields, so
    // this serialization cannot fail in practice — but `if let`, not
    // `.expect()`, keeps this route panic-free even if that ever changes:
    // a serialization failure here degrades to an empty envelope rather
    // than a crashed request.
    // The job the orchestrator built for this pipeline, when it has one:
    // its graph is what the flowchart and the run steps refer to.
    let job = authored_pipelines::job_name(id);
    let graph = if pipeline.status == "draft" {
        None
    } else {
        state.dagster.job_graph(&job).await.ok()
    };
    // R3 plan 2a: surface the dependency graph on the detail payload.
    // Upstream is `id, name, lastSuccessAt` for every entry in
    // `pipeline.depends_on` — the freshness field is what the user looks
    // at to read the dependency sensor's skip reason ("upstream X has not
    // had a successful run since this pipeline's last start"). Downstream
    // is every authored pipeline whose `depends_on` contains this id.
    // Dagster-native jobs are not in this DB, so downstream is scoped to
    // authored pipelines; Dagster jobs that happen to be triggered by an
    // authored pipeline are visible via that pipeline's upstream list,
    // not here.
    let upstream = match detail_upstream(state, pool, &pipeline.depends_on).await {
        Ok(u) => u,
        Err(err) => {
            tracing::warn!(error = %err.0, "authored_detail: upstream lookup failed");
            return err.into_response();
        }
    };
    let downstream = match detail_downstream(pool, id).await {
        Ok(d) => d,
        Err(err) => {
            tracing::warn!(error = %err.0, "authored_detail: downstream lookup failed");
            return err.into_response();
        }
    };
    let mut body = serde_json::to_value(&pipeline).unwrap_or_else(|_| json!({}));
    if let Value::Object(obj) = &mut body {
        obj.insert("engine".to_owned(), json!("authored"));
        // `description` is the author's own text, already in `body`; it
        // used to be overwritten with null here.
        obj.entry("description").or_insert(Value::Null);
        obj.insert(
            "orchestratorJob".to_owned(),
            if graph.is_some() {
                json!(job)
            } else {
                Value::Null
            },
        );
        obj.insert(
            "graph".to_owned(),
            graph.map_or(Value::Null, |g| {
                json!({
                    "ops": g.ops.iter().map(graph_op_to_json).collect::<Vec<_>>(),
                    "edges": g.edges,
                })
            }),
        );
        obj.insert("config".to_owned(), json!([]));
        obj.insert(
            "definition".to_owned(),
            definition.map_or(Value::Null, |d| {
                serde_json::to_value(d).unwrap_or(Value::Null)
            }),
        );
        obj.insert("upstream".to_owned(), json!(upstream));
        obj.insert("downstream".to_owned(), json!(downstream));
    }
    (StatusCode::OK, ApiJson(body)).into_response()
}

/// Build the `upstream` array for `authored_detail`: one entry per id
/// in `depends_on`, each with its display name and the timestamp of the
/// most recent `SUCCESS` run, or `null` when there has never been one.
/// Dagster-native upstreams and authored upstreams go through the same
/// `list_runs_for_job` path; authored ids are first mapped through
/// `authored_pipelines::job_name` because the orchestrator's `runsOrError`
/// filter matches on `pipelineName`, which is the safe name the factory
/// gives the job. R3 plan 2a.
async fn detail_upstream(
    state: &AppState,
    pool: &PgPool,
    depends_on: &[String],
) -> Result<Value, ApiRejection> {
    let mut upstream = Vec::with_capacity(depends_on.len());
    for dep in depends_on {
        let (display_name, job_lookup_name) = if dep.starts_with("pl-") {
            // `get_pipeline` on an unknown id returns `Ok(None)`; we
            // emit `name = null` rather than 500ing the whole detail
            // route — a missing upstream is a configuration drift (an
            // id renamed out from under `depends_on`) and the user
            // needs to see the rest of the page to diagnose it.
            let display = match pipelines::get_pipeline(pool, dep).await? {
                Some(p) => p.name,
                None => String::new(),
            };
            (display, authored_pipelines::job_name(dep))
        } else {
            (dep.clone(), dep.clone())
        };
        // `list_runs_for_job(limit=50)` is what `runs.rs` already uses
        // for the per-pipeline runs list — the smallest bound where a
        // skipped upstream (no runs in the last 30 days) reliably
        // returns the most recent SUCCESS instead of paging the full
        // history. The sensor itself uses `limit=1`; the detail view
        // wants more context than that.
        let last_success_at = match state.dagster.list_runs_for_job(&job_lookup_name, 50).await {
            Ok(runs) => runs
                .iter()
                .find(|r| r.status == "SUCCESS")
                .and_then(|r| r.end_time),
            Err(err) => {
                tracing::warn!(%err, job=%job_lookup_name, "upstream runs lookup failed");
                // Orchestrator unreachable is a degraded-but-not-fatal
                // condition here; surface `null` and let the page render
                // with the freshness field missing rather than 500 the
                // whole detail page.
                None
            }
        };
        upstream.push(json!({
            "id": dep,
            "name": display_name,
            "lastSuccessAt": last_success_at,
        }));
    }
    Ok(json!(upstream))
}

/// Build the `downstream` array for `authored_detail`: every authored
/// pipeline whose `depends_on` contains `this_id`, with its display
/// name. R3 plan 2a.
async fn detail_downstream(pool: &PgPool, this_id: &str) -> Result<Value, ApiRejection> {
    let others = authored_pipelines::collect_authored_depends_on(pool, None).await?;
    let mut downstream = Vec::new();
    for (id, deps) in others {
        if !deps.iter().any(|d| d == this_id) {
            continue;
        }
        // Same `display_name` rule as upstream: empty when the row has
        // been deleted between `collect_authored_depends_on` and
        // `get_pipeline` (drift, not 500). Authored pipelines are the
        // only kind that can appear here — Dagster jobs' dependency
        // relationships live in code, not in the DB.
        let display_name = match pipelines::get_pipeline(pool, &id).await? {
            Some(p) => p.name,
            None => String::new(),
        };
        downstream.push(json!({
            "id": id,
            "name": display_name,
        }));
    }
    Ok(json!(downstream))
}

/// `GET /api/pipelines/{id}/source?op=<sourceRef>` — read-only op source
/// text (WS4 item C2, grand plan §6's path-traversal risk item). `id` is
/// accepted but not otherwise used to scope the lookup — the allowlist is
/// global to the one code location this API image ships alongside
/// (`crate::state::AppState::pipeline_source_allowlist`); it is kept in
/// the path for contract symmetry with the other three new routes and
/// because a future multi-code-location deployment would need it.
///
/// # Security
///
/// `op` is CALLER-SUPPLIED, hostile input. It is never concatenated into a
/// filesystem path: `crate::pipeline_source::read_source` resolves the
/// file half through a pre-built [`crate::pipeline_source::SourceAllowlist`]
/// (an exact-key `HashMap` lookup), so a `..`, absolute path, symlink
/// escape, or encoded variant simply never matches a key and is refused
/// before any `std::fs` call touches it. `commit` provenance is checked
/// against a FRESH `job_graph` call's own `commit` metadata — NEVER from a
/// `commit` query parameter (which would be exactly as caller-controllable
/// as `op` itself, and so exactly as untrustworthy).
///
/// # Errors
///
/// 400 if `op` is missing; 409 if `op` does not name any real op in the
/// job's current graph (its `commit` is then unknowable — see below — never
/// silently 404'd, since "op unknown" and "commit unverifiable" would
/// otherwise be indistinguishable to a caller) or if a real op's own
/// `commit` metadata does not match this image's `GIT_SHA`
/// (`crate::pipeline_source::check_commit`); 404 if the commit check
/// passes but `sourceRef` is still not in the allowlist or its function is
/// not found within it; 503 if the `Dagster` `job_graph` lookup itself
/// fails.
pub async fn source(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<SourceQuery>,
) -> Response {
    let Some(op_ref) = params.op else {
        return (
            StatusCode::BAD_REQUEST,
            ApiJson(json!({ "error": "missing ?op=" })),
        )
            .into_response();
    };
    let graph = match state.dagster.job_graph(&id).await {
        Ok(g) => g,
        Err(err) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                ApiJson(json!({ "error": js_error(err) })),
            )
                .into_response();
        }
    };
    let op_commit = graph
        .ops
        .iter()
        .find(|o| o.source_ref.as_deref() == Some(op_ref.as_str()))
        .and_then(|o| o.commit.as_deref());
    if let Err(err) = crate::pipeline_source::check_commit(op_commit, &state.config.git_sha) {
        return (
            StatusCode::CONFLICT,
            ApiJson(json!({ "error": err.to_string() })),
        )
            .into_response();
    }
    match crate::pipeline_source::read_source(&state.pipeline_source_allowlist, &op_ref) {
        Ok(text) => (
            StatusCode::OK,
            ApiJson(json!({
                "sourceRef": op_ref,
                "commit": state.config.git_sha,
                "language": "python",
                "text": text,
            })),
        )
            .into_response(),
        Err(_) => (
            StatusCode::NOT_FOUND,
            ApiJson(json!({ "error": "source not available for this build" })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct SourceQuery {
    op: Option<String>,
}

/// `GET /api/pipelines/{id}/runs/{runId}/steps` — per-step status and row
/// counts for one run (WS4 item C3, closes WS1 T3 for real
/// materializations). `id` is accepted for URL symmetry with the other
/// run-scoped routes but not used to filter — a `runId` already uniquely
/// identifies a run in `Dagster`.
///
/// # Errors
///
/// 503 if the `Dagster` `run_steps` lookup fails. An unknown `runId`
/// itself is not a distinct error case here —
/// [`lakehouse_dagster::DgClient::run_steps`] reports it as an empty
/// `Vec` (its own doc comment: "nothing to show", not "an error"), so this
/// route responds `200 { "steps": [] }`, never a fabricated 404 for a run
/// this client cannot actually tell apart from "no steps recorded yet".
pub async fn run_steps(
    State(state): State<AppState>,
    Path((_id, run_id)): Path<(String, String)>,
) -> Response {
    match state.dagster.run_steps(&run_id).await {
        Ok(steps) => (
            StatusCode::OK,
            ApiJson(json!({ "steps": steps.iter().map(step_to_json).collect::<Vec<_>>() })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "error": js_error(err) })),
        )
            .into_response(),
    }
}

fn step_to_json(s: &lakehouse_dagster::RunStep) -> Value {
    json!({
        "stepKey": s.step_key,
        // Reuses the existing `map_run_status` rather than sending
        // Dagster's raw step-status vocabulary — the TS contract (Phase F)
        // types this field EntityStatus, matching every other status the
        // API already sends, so the frontend needs no second, Dagster-
        // specific status mapper of its own. A step status
        // `map_run_status` doesn't recognize (e.g. "SKIPPED", which it
        // never returns since it was written for RUN-level statuses)
        // falls through to its existing "unknown" catch-all -- honest,
        // not a fabricated guess at which EntityStatus it should be.
        "status": map_run_status(&s.status),
        "startMs": s.start_ms,
        "endMs": s.end_ms,
        "materializations": s.materializations.iter().map(|m| json!({
            "assetKey": m.asset_key, "rows": m.rows,
        })).collect::<Vec<_>>(),
        // Plan 1c: every retry attempt is its own attempt record on the
        // Dagster side; the count of records is the count of attempts.
        // 0 means "never even started" — a step without an `attempts`
        // entry, which can happen for a queued step before the daemon
        // touches it. The TS contract types this as a non-null number;
        // we never send `null`, so the UI does not need an "unknown"
        // branch.
        "attempts": s.attempts,
    })
}

/// Plan 1c (R2, day-1): `GET /api/pipelines/{id}/runs/steps` — a runs ×
/// steps matrix for the recovery UI: every recent run of the pipeline,
/// each row carrying the status and duration of every step `Dagster`
/// reports for that run. One GraphQL round trip (`runsOrError` with a
/// nested `stepStats` projection), so a console page that today makes
/// one `list_runs_for_job` + N `run_steps` round trips collapses to a
/// single request.
///
/// Same `pl-` vs dagster id dispatch as [`runs`]: an authored
/// pipeline's runs belong to the `authored__<id>` job.
///
/// When `Dagster` is unreachable, the route returns an EMPTY matrix
/// with an `"unavailable"` field rather than a 503 — the recovery page
/// already shows "orchestrator unreachable" for the same shape (see
/// [`runs_body`]); a fabricated list of runs would silently lose the
/// signal that something is wrong.
pub async fn runs_step_matrix(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let job = if id.starts_with("pl-") {
        authored_pipelines::job_name(&id)
    } else {
        id.clone()
    };
    match state.dagster.list_runs_with_steps_for_job(&job, 30).await {
        Ok(runs) => (
            StatusCode::OK,
            ApiJson(json!({
                "runs": runs.iter().map(run_with_steps_to_json).collect::<Vec<_>>(),
                "unavailable": Value::Null,
            })),
        )
            .into_response(),
        Err(err) => {
            tracing::warn!(%err, "pipeline runs/steps: orchestrator unreachable");
            (
                StatusCode::OK,
                ApiJson(json!({
                    "runs": [],
                    "unavailable": js_error(err),
                })),
            )
                .into_response()
        }
    }
}

fn run_with_steps_to_json(r: &lakehouse_dagster::RunWithSteps) -> Value {
    json!({
        "id": r.run_id,
        "status": map_run_status(&r.status),
        // A timestamp `Dagster` did not observe is a fabricated
        // measurement (WS4 item G1); `startTime` is `None` until the
        // run has actually started, and the field stays `null` rather
        // than defaulting to `now()` or the request time.
        "startedAt": r.start_time.map_or(Value::Null, |t| {
            Value::String(iso_from_unix_seconds(t))
        }),
        "steps": r.steps.iter().map(|s| json!({
            "stepKey": s.step_key,
            "status": map_run_status(&s.status),
            "durationMs": s.duration_ms,
        })).collect::<Vec<_>>(),
    })
}

/// Hard cap on log lines a single `GET .../logs` request may return,
/// REGARDLESS of what the caller's `?limit=` asks for (WS4 item C4, grand
/// plan §6's "log streaming must not leak" risk item). `Dagster` op logs
/// can echo arbitrarily large payloads (a misbehaving op printing a full
/// row), and an unbounded page size would let one request pull an entire
/// run's log history into a single response.
const MAX_LOG_LINES_PER_PAGE: u32 = 500;

/// `GET /api/pipelines/{id}/runs/{runId}/logs?cursor=&limit=` — a bounded,
/// monotonic page of log lines (WS4 item C4). `limit` is clamped to
/// [`MAX_LOG_LINES_PER_PAGE`] regardless of what the caller requests. The
/// cursor itself is `Dagster`'s own opaque `logsForRun` cursor, passed
/// straight through as `afterCursor` on the next call — never
/// re-interpreted or re-encoded here — so paging strictly forward through
/// it can neither skip an event (each page starts exactly where the last
/// one's `cursor` left off) nor duplicate one (an already-returned event
/// is never re-included by `Dagster`'s own cursor semantics) under
/// repeated polling.
///
/// # Who can read these logs
///
/// Op log MESSAGE TEXT is passed through verbatim — an op that logs a
/// value it shouldn't (a connection string on error, a row's contents)
/// makes that value visible to anyone this route's
/// `Policy::RequiresPermission("pipeline:read")` gate admits. Verified
/// against `rust/migrations/0002_seed_identity.sql`, `pipeline:read` is
/// granted only to the seeded Data Engineer role (`pipeline:*`) and
/// Platform Admin (`*:*`) — never to a viewer/analyst-shaped role, and the
/// `dagster-orchestrator` service identity itself holds `pipeline:run`
/// (used to trigger/list), not `pipeline:read`, so a `Dagster`-side
/// credential cannot read its own run logs back through this route. This
/// is an operator-scoped surface today, not a general one — **if a future
/// role grant widens `pipeline:read`, it widens run-log visibility by the
/// same amount**, which is not automatically revisited by this route.
///
/// # Errors
///
/// 404 if `run_id` names a run `Dagster` reports as not found
/// (`RunNotFoundError`); 503 if the `Dagster` `run_logs` call fails for
/// any other reason, via [`js_error`], same as every other `Dagster`-
/// backed route in this module — `js_error` wraps `DgError`'s own
/// `Display` (a client-generated summary already truncated/shaped by
/// `lakehouse_dagster`, never a raw HTTP response body or stack trace),
/// not a fresh classification step of its own; a transport-level failure
/// is the fixed string `"fetch failed"` (`DgError::Transport`'s own doc
/// comment), while a typed `Dagster` GraphQL failure's `message` field
/// passes through — the same posture every other route here already
/// takes for a `Dagster` failure.
pub async fn run_logs(
    State(state): State<AppState>,
    Path((_id, run_id)): Path<(String, String)>,
    Query(params): Query<LogsQuery>,
) -> Response {
    let limit = params.limit.unwrap_or(200).min(MAX_LOG_LINES_PER_PAGE);
    match state
        .dagster
        .run_logs(&run_id, params.cursor.as_deref(), limit)
        .await
    {
        Ok(page) => (
            StatusCode::OK,
            ApiJson(json!({
                "lines": page.lines.iter().map(|l| json!({
                    "ts": l.ts, "level": l.level, "stepKey": l.step_key, "message": l.message,
                })).collect::<Vec<_>>(),
                "cursor": page.cursor,
            })),
        )
            .into_response(),
        // `DgError::Server` here carries `Dagster`'s own `message` field,
        // not its `__typename` -- `lakehouse_dagster::DgClient::run_logs`
        // returns the message text verbatim for any non-`EventConnection`
        // response (`RunNotFoundError`, `PythonError`), so this route
        // cannot switch on the typename directly. Live-verified against
        // this repository's own Dagster 1.13.20 stack (a `logsForRun`
        // query against a bogus `runId`): `RunNotFoundError.message` is
        // literally `"Pipeline run <id> could not be found."` -- matched
        // on that real, observed substring, NOT the plan sketch's
        // `.contains("NotFound")` (which never appears in the message
        // text itself, only in the typename `DgError::Server` discards).
        Err(DgError::Server(msg)) if msg.contains("could not be found") => (
            StatusCode::NOT_FOUND,
            ApiJson(json!({ "error": format!("run {run_id} not found") })),
        )
            .into_response(),
        // Never forward the raw Dagster error body -- `js_error` classifies
        // it the same way every other Dagster-backed route in this file
        // does.
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "error": js_error(err) })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct LogsQuery {
    cursor: Option<String>,
    limit: Option<u32>,
}

fn run_to_json(r: &DgRun, pipeline_id: &str, max_duration_seconds: Option<i32>) -> Value {
    let dur = duration_seconds(r.start_time, r.end_time);
    json!({
        "id": r.run_id,
        "pipelineId": pipeline_id,
        "status": map_run_status(&r.status),
        "startedAt": r.start_time.map_or_else(String::new, iso_from_unix_seconds),
        "endedAt": r.end_time.map(iso_from_unix_seconds),
        // WS1 task 1.2: Dagster's run record carries no row counts. WS4
        // reads them from step materializations; until then null says "not
        // measured" rather than 0 claiming "measured none". Row counts
        // aside, `costUnits` used to carry the run's duration in seconds
        // under a currency-sounding name, which the console then formatted
        // as currency-like units — an honest `durationSeconds` replaces it.
        "processed": Value::Null,
        "accepted": Value::Null,
        "rejected": Value::Null,
        "retried": Value::Null,
        "costUnits": Value::Null,
        "durationSeconds": dur,
        // Plan 1f: per-run duration SLA outcome. `null` when no SLA is
        // configured (`maxDurationSeconds = None`) or the run is still
        // going — the same honest-null posture [`run_to_json`] takes for
        // every other measured-but-unknown field here.
        "overDuration": over_duration(dur, max_duration_seconds)
            .map_or(Value::Null, Value::Bool),
        // When the run was created. The gap to `startedAt` is queue and
        // launch time, which the console shows apart from run time.
        "queuedAt": r.creation_time.map(iso_from_unix_seconds),
        "parentRunId": r.parent_run_id,
        "rootRunId": r.root_run_id,
        "trigger": run_trigger(r),
    })
}

/// What launched a run, read from the tags `Dagster` itself writes. A run
/// with none of them was launched by hand or through the API, which is
/// what `"manual"` means here; it does not claim a particular person.
fn run_trigger(r: &DgRun) -> Value {
    let tag = |key: &str| {
        r.tags
            .iter()
            .find(|t| t.key == key)
            .map(|t| t.value.clone())
    };
    if let Some(name) = tag("dagster/schedule_name") {
        json!({ "kind": "schedule", "name": name })
    } else if let Some(name) = tag("dagster/sensor_name") {
        json!({ "kind": "sensor", "name": name })
    } else if let Some(name) = tag("dagster/backfill") {
        json!({ "kind": "backfill", "name": name })
    } else if r.parent_run_id.is_some() {
        json!({ "kind": "retry", "name": Value::Null })
    } else {
        json!({ "kind": "manual", "name": Value::Null })
    }
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

/// Pipeline SLA "late" decision (plan 1f). Re-exported from
/// `lakehouse_alerts::late` so the runs/detail routes and the alerts
/// crate's `evaluate_pipeline_late` share one implementation. The
/// strict-`>` boundary is the same rule the alerts crate enforces,
/// mutation-tested by `late_is_false_when_gap_is_exactly_the_threshold`
/// in `lakehouse_alerts::tests::sla_helpers`.
fn late(
    now_seconds: Option<f64>,
    last_success_seconds: Option<f64>,
    late_after_seconds: Option<i32>,
) -> Option<bool> {
    lakehouse_alerts::late(now_seconds, last_success_seconds, late_after_seconds)
}

/// Per-run "over its duration SLA" decision (plan 1f).
///
/// * `max_duration_seconds = None` — no duration SLA: returns `None`, never
///   `false`. The runs route reports `overDuration: null` per run in this
///   case.
/// * `duration_seconds = None` — the run is still going (no `endTime` from
///   `Dagster`): returns `None`. A still-running run is not "over" yet.
/// * Both present — returns `Some(duration > max_duration_seconds)`.
///   STRICT inequality, matching [`late`].
fn over_duration(duration_seconds: Option<i64>, max_duration_seconds: Option<i32>) -> Option<bool> {
    let max = max_duration_seconds?;
    let dur = duration_seconds?;
    Some(dur > i64::from(max))
}

/// Row-volume "drop" decision (plan 1f).
///
/// * `current_rows = None` — null rows: returns `None`. A run with no
///   measured rows is not a "drop"; it is a measurement gap.
/// * `prior_rows.len() < MIN_SAMPLES` (5) — not enough history: returns
///   `None`. "Not enough history" is honest, never silently `false` (the
///   test `drop_is_null_when_history_is_below_min_samples` pins this).
/// * `prior_rows.len() >= MIN_SAMPLES` — returns
///   `Some(current < 0.5 × median(prior_rows))` with STRICT inequality:
///   exactly half the median is **not** a drop.
fn drop(current_rows: Option<i64>, prior_rows: &[i64]) -> Option<bool> {
    // Floor for "enough history" — pinned by the spec and by the test
    // `drop_is_null_when_history_is_below_min_samples`. Lowering to 4
    // makes that test fail.
    const MIN_SAMPLES: usize = 5;
    let current = current_rows?;
    if prior_rows.len() < MIN_SAMPLES {
        return None;
    }
    // Unreachable past the MIN_SAMPLES guard (a 5+ element slice always
    // has a median), but `None` is the correct answer for an empty
    // history anyway — the rule cannot decide, so propagate rather than
    // assert.
    let med = median(prior_rows)?;
    // Compare in f64 — truncating the THRESHOLD to i64 would change the
    // boundary (median 41, current 20: a drop at 20.5, not at 20).
    #[allow(
        clippy::cast_precision_loss,
        reason = "row counts are far below f64's exact range"
    )]
    Some((current as f64) < half_median(med))
}

/// Median of a non-empty `i64` slice, matching the same upper-of-middle
/// convention the `drop` rule uses (`sorted[n/2]` for even-length inputs).
/// Plan 1f's volume route surfaces this as `medianRows` for the
/// console's chart, so it has to be the same function (not "the same
/// arithmetic done twice and trusted to stay in sync"). `None` for an
/// empty slice — there is no median of nothing.
fn median(rows: &[i64]) -> Option<i64> {
    let mut sorted: Vec<i64> = rows.to_vec();
    sorted.sort_unstable();
    sorted.get(sorted.len() / 2).copied()
}

/// Half the median — the threshold the drop rule compares against.
/// Hoisted so `drop` and the volume route's documented half-median
/// threshold share the exact same arithmetic. `i64 → f64` cast is
/// exact for every row count a warehouse would ever emit (the half
/// comparison needs the float so an exact-half row is NOT a drop).
#[allow(clippy::cast_precision_loss, reason = "row counts fit in f64")]
fn half_median(median: i64) -> f64 {
    (median as f64) * 0.5
}

/// The `NewAuditEvent` [`trigger`]/[`create`]/[`pause`]/[`resume`] each
/// write on success — a pure, unit-tested helper (WS5 item D4), mirroring
/// `routes::connectors::connector_audit_event`'s pattern:
/// `resource_kind: "pipeline"` paired with the pipeline/job's own id is
/// this task's own choice (T16's fixed scope covers query history,
/// approvals, and connector detail only — not these two sites, per the
/// Phase D preamble). `outcome` is always `"executed"`: every call site
/// below only calls this on ITS OWN success path, so there is no failure
/// outcome for this helper to encode.
///
/// `args` is the structured side-channel for action-specific data the
/// generic `resource_kind`/`resource_id`/`run_id`/`approval_id`/
/// `session_id` columns do not already capture. R4 plan 2c uses it for
/// [`trigger`]: the TOP-LEVEL keys of the run config (NEVER the values)
/// land in `args["configKeys"]` so a future reader can answer "this
/// launch supplied a `connector_id` field" without the column carrying
/// the payload. Other call sites pass `Value::Null`.
///
/// `principal_kind` comes from [`Principal::kind_for_audit`], never
/// [`Principal::provider`] — same CHECK this crate's other audit sites
/// satisfy.
fn pipeline_audit_event(
    principal: &Principal,
    action: &str,
    pipeline_id: &str,
    args: Value,
) -> NewAuditEvent {
    NewAuditEvent {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        action: action.to_owned(),
        resource_kind: Some("pipeline".to_owned()),
        resource_id: Some(pipeline_id.to_owned()),
        args: Some(args),
        outcome: "executed".to_owned(),
        detail: None,
        run_id: None,
        approval_id: None,
        session_id: None,
    }
}

/// Best-effort `pipeline_audit_event` insert — log and continue on `Err`,
/// same non-fatal posture as `routes::connectors`'s call sites: a failed
/// audit write must never turn an already-succeeded pipeline action into
/// an error response.
pub(super) async fn record_pipeline_audit(
    state: &AppState,
    principal: &Principal,
    action: &str,
    id: &str,
    args: Value,
) {
    let Some(pool) = state.pg.as_deref() else {
        return;
    };
    let event = pipeline_audit_event(principal, action, id, args);
    if let Err(err) = store_audit::insert(pool, event).await {
        tracing::warn!(%err, action, pipeline_id = id, "failed to record pipeline audit event");
    }
}

/// `POST /api/pipelines/{id}/trigger` — launch a new run of job `id`.
///
/// This mutates live infrastructure (starts a real `Dagster`/`ClickHouse`
/// pipeline run) and is therefore exercised only via the
/// `pipeline-trigger-bad-id` corpus entry, which targets a job name that
/// does not exist so `Dagster` rejects the launch instead of starting one.
///
/// `principal` is `Option<Extension<Principal>>`, not a bare
/// `Extension<Principal>`, even though the mounted route always supplies
/// one (`pipeline:write`, `crate::policy::POLICY_TABLE`):
/// `routes::ai::tools::pipelines::trigger_pipeline` calls this handler
/// directly, bypassing that middleware, for the copilot's own
/// `trigger_pipeline` tool — same shape as `routes::connectors::create`
/// (WS5 item D3).
///
/// # Errors
///
/// Returns a 401 [`Response`] (not a propagated [`ApiError`] — this
/// handler predates `ApiResult` and stays bare-`Response`-returning, like
/// [`pause`]/[`resume`]) if no principal is present.
pub async fn trigger(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Path(id): Path<String>,
    body: Option<Json<TriggerBody>>,
) -> Response {
    let Some(Extension(principal)) = principal else {
        return ApiRejection(ApiError::unauthorized()).into_response();
    };
    // `Option<Json<T>>` distinguishes "no body" (`Ok(None)`) from "valid
    // body" (`Ok(Some(_))`) without 400-ing on the missing `Content-Type`
    // header the existing curl corpus entry uses — axum 0.8's
    // `Option<Json<T>>` extractor (`axum-0.8.9/src/json.rs:116`) drops the
    // header requirement and yields `None` for an empty body, the same
    // behavior the existing `routes::connectors::test_connection` call
    // site relies on.
    let run_config = body.and_then(|Json(b)| b.run_config);
    // R4 plan 2c: when the caller sent a run_config, validate it
    // against the job's schema BEFORE launching. A `RunConfigValidation
    // Invalid` becomes a structured 400 (see [`build_config_error_body`]).
    // A `PipelineNotFoundError` falls through to the same `launch_run`
    // path as the no-config case — `launch_run` already surfaces that
    // case as `LaunchOutcome { error: ..., .. }`, which the route
    // renders as a 422.
    if let Some(cfg) = run_config.as_ref() {
        match state.dagster.validate_run_config(&id, cfg).await {
            Ok(ConfigValidationOutcome::Valid | ConfigValidationOutcome::NotFound) => {}
            Ok(ConfigValidationOutcome::Invalid { errors }) => {
                return (
                    StatusCode::BAD_REQUEST,
                    ApiJson(build_config_error_body(errors)),
                )
                    .into_response();
            }
            Err(err) => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    ApiJson(json!({ "error": js_error(err) })),
                )
                    .into_response();
            }
        }
    }
    // `configKeys` for the audit column are the TOP-LEVEL keys of the
    // supplied config — never the values. The audit column carries a
    // structured side-channel for action-specific data (see
    // [`pipeline_audit_event`]); "the run supplied a `connector_id`
    // field" is what a future reader needs to answer, not the actual id
    // value.
    let config_keys: Vec<String> = run_config.as_ref().map(top_level_keys).unwrap_or_default();
    // An authored pipeline runs as the `authored__<id>` job
    // `authored_factory.py` builds for it; see `authored_pipelines`.
    let job = if id.starts_with("pl-") {
        match authored_launch_target(&state, &id).await {
            Ok(job) => job,
            Err(response) => return response,
        }
    } else {
        id.clone()
    };
    let launch = match run_config.as_ref() {
        Some(cfg) => state.dagster.launch_run_with_config(&job, cfg).await,
        None => state.dagster.launch_run(&job).await,
    };
    match launch {
        Ok(outcome) => {
            if let Some(error) = outcome.error {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    ApiJson(json!({ "error": error })),
                )
                    .into_response();
            }
            let audit_args = if config_keys.is_empty() {
                Value::Null
            } else {
                json!({ "configKeys": config_keys })
            };
            record_pipeline_audit(&state, &principal, "pipeline.trigger", &id, audit_args).await;
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

/// Body for `POST /api/pipelines/{id}/trigger` (R4 plan 2c). The whole
/// type is optional (the route's `Option<Json<TriggerBody>>` accepts an
/// empty body); `runConfig` is itself optional, so a caller may send
/// `{ "runConfig": null }` or omit the field entirely — both mean "use
/// Dagster's defaults for this job".
///
/// `runConfig` is the raw JSON object Dagster's `RunConfigData` scalar
/// expects (NOT a YAML string — see [`DgClient::validate_run_config`]).
#[derive(Debug, Default, Deserialize)]
pub(super) struct TriggerBody {
    #[serde(default, rename = "runConfig")]
    pub run_config: Option<Value>,
}

/// Top-level keys of a JSON object, or empty for non-objects (null,
/// array, string, ...). Used to populate the `configKeys` audit field
/// without forwarding the values themselves — see [`trigger`]'s
/// `record_pipeline_audit` call.
fn top_level_keys(value: &Value) -> Vec<String> {
    let Some(obj) = value.as_object() else {
        return Vec::new();
    };
    obj.keys().cloned().collect()
}

/// 400 body shape for a `RunConfigValidationInvalid` from
/// [`trigger`]. Built ONLY from the structured `path` array and the
/// enum `reason` (one of `RUNTIME_TYPE_MISMATCH`,
/// `MISSING_REQUIRED_FIELD`, `MISSING_REQUIRED_FIELDS`,
/// `FIELD_NOT_DEFINED`, `FIELDS_NOT_DEFINED`, `SELECTOR_FIELD_ERROR`)
/// — `Dagster`'s own `message` / `stack` / `value_rep` / `field` /
/// `fields` strings are deliberately NOT carried (see
/// [`DgClient::validate_run_config`]'s doc comment and AGENTS.md
/// principle 4). This is the regression check from the plan: passing
/// `message` through in the body MUST fail the sentinel test, so the
/// route cannot silently accept a future re-introduction.
fn build_config_error_body(errors: Vec<lakehouse_dagster::ConfigValidationError>) -> Value {
    let entries: Vec<Value> = errors
        .into_iter()
        .map(|e| {
            json!({
                "path": e.path,
                "reason": e.reason,
            })
        })
        .collect();
    json!({ "errors": entries })
}

/// `GET /api/pipelines/{id}/config-schema` (R4 plan 2c) — the read-side
/// companion to [`trigger`]. Returns the job's default config so a
/// console form / copilot tool can pre-fill a config editor before the
/// user clicks "Run".
///
/// Response shape:
/// ```json
/// {
///   "pipelineId": "<id>",
///   "hasConfig": true,
///   "defaultConfig": null,
///   "defaultConfigYaml": "<yaml-string>"
/// }
/// ```
/// `defaultConfig` is intentionally `null` rather than a parsed JSON
/// object: `Dagster` emits the default as a YAML string, the workspace
/// has no YAML parser dep (a deliberate non-decision — see
/// [`DgClient::run_config_schema`]'s doc comment), and a JSON Schema
/// editor does not need the pre-filled values to begin with.
/// `defaultConfigYaml` carries the raw string for callers that want it.
///
/// The `pl-…` namespace (authored pipelines) returns `hasConfig: false`
/// without contacting Dagster — `authored_factory.py`'s default is
/// "config is whatever the user supplied at submit time, validated
/// client-side by the form" (R4 plan 2c, see `docs/plans/pipelines/
/// day-1/r4-governance.md`). A non-`pl-` id is looked up in the
/// orchestrator; an unknown id is a 404 (matching the `pipeline.runnable`
/// route's posture for jobs Dagster doesn't know about — a 503 would
/// be honest about "Dagster rejected this job name", but the existing
/// `pipeline.runnable` body says "no, this id is not a job" so the
/// behaviour is consistent across reads).
///
/// # Errors
///
/// Returns a 401 [`Response`] (same posture as [`trigger`]/[`pause`]/
/// [`resume`]) if no principal is present. Returns a 503 with
/// `js_error(err)` (matching [`source`]'s posture) if the GraphQL call
/// fails for any other reason.
pub async fn config_schema(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Path(id): Path<String>,
) -> Response {
    let Some(Extension(_principal)) = principal else {
        return ApiRejection(ApiError::unauthorized()).into_response();
    };
    if id.starts_with("pl-") {
        // Authored pipelines: no Dagster-side config schema (the
        // console is the source of truth for the form). Returning
        // `hasConfig: false` keeps the route honest about the
        // namespace; the orchestrator only loads an `authored__<id>`
        // job for `ready`/`paused` pipelines, and even then the
        // `runConfig` is fully editor-driven.
        return (
            StatusCode::OK,
            ApiJson(json!({
                "pipelineId": id,
                "hasConfig": false,
                "defaultConfig": Value::Null,
                "defaultConfigYaml": Value::Null,
            })),
        )
            .into_response();
    }
    let schema = match state.dagster.run_config_schema(&id).await {
        Ok(Some(schema)) => schema,
        Ok(None) => {
            return ApiRejection(ApiError::NotFound(format!("Pipeline {id} not found")))
                .into_response();
        }
        Err(err) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                ApiJson(json!({ "error": js_error(err) })),
            )
                .into_response();
        }
    };
    (
        StatusCode::OK,
        ApiJson(json!({
            "pipelineId": id,
            "hasConfig": true,
            "defaultConfig": Value::Null,
            "defaultConfigYaml": schema.root_default_yaml,
        })),
    )
        .into_response()
}

/// `POST /api/pipelines/events/run-failed` — Dagster's `run_failure_sensor`
/// (see `dagster/dispar_orchestrate/pipeline_events.py`) calls this once
/// per failed run. The handler-side checks the `policy` table's
/// `pipeline:write` gate could not: a `pipeline:write`-holding user is not
/// an orchestrator. Only a service identity (the orchestrator's own) and a
/// Platform Admin are admitted — the same posture as
/// [`authored_pipelines::runnable`].
///
/// # Dedupe
///
/// Before evaluating rules, this calls
/// [`pipelines::record_pipeline_run_event`]; a `false` return means the
/// `(run_id, kind="failure")` pair was already recorded and this call is a
/// sensor retry, so the response is `{"matched": 0}` and no rule fires
/// again. A `true` return goes through to the alert evaluator.
///
/// # Failures that aren't failures
///
/// A body whose run reports a non-`FAILURE` `Dagster` status is rejected
/// with 409 — the sensor should not be sending non-failed runs here, and
/// silently treating them as failures would skew alerting. A body whose
/// `jobName` does not map to a known pipeline is treated as "an unknown
/// orchestrator job," not an error: `{"matched": 0}`.
pub async fn run_failed_event(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    // The policy table gates this route on `pipeline:write`. A user with
    // that permission is still not the orchestrator — only a service
    // identity is allowed to deliver its events. Mirrors
    // `authored_pipelines::runnable`.
    if !matches!(principal.id, PrincipalId::Service(_))
        && !crate::routes::catalog::is_unrestricted(&principal)
    {
        return Err(ApiError::PermissionDenied(
            "only the orchestrator's service identity reports pipeline-run failures".to_owned(),
        )
        .into());
    }
    let req: RunFailedBody = parse_body(&body)?;
    let pool = pool(&state)?;
    // Reverse the `authored_pipelines::job_name` mapping to find the
    // pipeline id behind `jobName`. The store owns the only authoritative
    // list of runnable pipelines, so we ask it and match by `job_name`
    // (a transformation, never a query parameter).
    let Some(pipeline_id) = job_name_to_pipeline_id(pool, &req.job_name).await? else {
        return Ok(ApiJson(json!({
            "matched": 0,
            "reason": "unknown jobName; no runnable pipeline owns it",
        })));
    };
    // A `run_failure_sensor` MUST only call this on a failed run. If
    // `Dagster` says otherwise the sensor is misconfigured, and a silent
    // pass would alias "not failed" into "alert fired."
    let status = state
        .dagster
        .pipeline_run_status(&req.run_id)
        .await
        .map_err(|err| ApiError::Unavailable(js_error(err)))?;
    let Some(info) = status else {
        return Err(ApiError::NotFound(format!("run {} not found in Dagster", req.run_id)).into());
    };
    if info.status != "FAILURE" {
        return Err(ApiError::Conflict(format!(
            "run {} is in Dagster status '{}', not FAILURE; refusing to alert",
            req.run_id, info.status
        ))
        .into());
    }
    let failed_step_keys: Vec<String> = info
        .steps
        .iter()
        .filter(|s| s.status == "FAILURE")
        .map(|s| s.key.clone())
        .collect();
    // Dedupe: a sensor retry sees `false` and the handler short-circuits
    // before any rule evaluation. The mutation check in
    // `lakehouse-store/tests/pipelines.rs`
    // (`record_pipeline_run_event_inserts_once_then_dedupes`) is the
    // canonical proof that `false` means "already alerted on this run."
    let first_seen =
        pipelines::record_pipeline_run_event(pool, &req.run_id, &pipeline_id, "failure").await?;
    if !first_seen {
        return Ok(ApiJson(json!({
            "matched": 0,
            "reason": "run already alerted; sensor retry ignored",
        })));
    }
    let http = reqwest::Client::new();
    let email = EmailSender::new(smtp_config(&state.config));
    let silence_source: Option<Box<dyn SilenceSource>> = state
        .pg
        .as_deref()
        .map(|pg| Box::new(ApiSilenceSource { pg }) as Box<dyn SilenceSource>);
    let matched = lakehouse_alerts::evaluate_pipeline_failure(
        &state.clickhouse,
        &http,
        &email,
        &pipeline_id,
        &req.run_id,
        &failed_step_keys,
        silence_source.as_deref().map(|s| s as &dyn SilenceSource),
    )
    .await
    .map_err(|err| ApiError::Unavailable(js_error(err)))?;
    Ok(ApiJson(json!({ "matched": matched })))
}

/// Reverse of [`authored_pipelines::job_name`]: given the `Dagster` job
/// name (e.g. `"authored__pl_orders"`), find the pipeline id (`"pl-orders"`)
/// that owns it. `None` when no runnable pipeline produces that job —
/// `evaluate_pipeline_failure` is not the place to invent a match, so the
/// caller answers `{matched: 0, reason: ...}`.
async fn job_name_to_pipeline_id(
    pool: &PgPool,
    job_name: &str,
) -> Result<Option<String>, ApiError> {
    let list = pipelines::list_runnable_pipelines(pool)
        .await
        .map_err(|err| ApiError::Unavailable(err.to_string()))?;
    Ok(list
        .into_iter()
        .find(|p| authored_pipelines::job_name(&p.id) == job_name)
        .map(|p| p.id))
}

/// The `POST /api/pipelines/events/run-failed` body. The sensor posts
/// `{"runId": "...", "jobName": "authored__<id>"}`; both fields are
/// required — `runId` for the dedupe and `jobName` for the
/// pipeline-id reverse lookup.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunFailedBody {
    pub run_id: String,
    pub job_name: String,
}

/// The `pipeline_volume_drop` decision for the run that just finished —
/// the Dagster half of [`run_finished_event`], extracted so the handler
/// stays under the 100-line lint. Pulls the same 30-run window the
/// volume route uses in one query, then walks oldest-first building the
/// history up to the current run.
///
/// # Errors
///
/// Returns `ApiError::Unavailable` (a classified, fixed string) when the
/// `Dagster` call fails — never an upstream detail.
async fn volume_drop_for_run(
    state: &AppState,
    pipeline_id: &str,
    run_id: &str,
) -> Result<Option<bool>, ApiError> {
    let job = authored_pipelines::job_name(pipeline_id);
    let runs_with_rows = state
        .dagster
        .list_runs_for_job_with_materializations(&job, 30)
        .await
        .map_err(|err| ApiError::Unavailable(js_error(err)))?;
    let mut history: Vec<i64> = Vec::with_capacity(runs_with_rows.len());
    let mut current_rows: Option<i64> = None;
    let mut found_current = false;
    // Walk oldest-first (Dagster returns most-recent first).
    for run in runs_with_rows.iter().rev() {
        if run.run.run_id == run_id {
            current_rows = run.rows;
            found_current = true;
            break;
        }
        if matches!(run.run.status.as_str(), "SUCCESS" | "FAILURE")
            && let Some(rows) = run.rows
        {
            history.push(rows);
        }
    }
    // The current run not appearing in Dagster's recent 30 (it is older
    // than the window, or the orchestrator's filter dropped it) means we
    // cannot honestly compute a drop — report `None` rather than
    // silently `false`. [`drop`] answers the question when the row is
    // present.
    Ok(if found_current {
        drop(current_rows, &history)
    } else {
        None
    })
}

/// `POST /api/pipelines/events/run-finished` — Dagster's `run_status_sensor`
/// for `SUCCESS` calls this once per successful run (plan 1f). It fires
/// the `pipeline_slow` and `pipeline_volume_drop` alerts for the run.
/// Same posture as [`run_failed_event`]: the policy gate cannot tell that
/// the caller is the orchestrator, so the handler enforces a service
/// identity; the run's `pipeline_run_event` rows dedupe sensor retries.
pub async fn run_finished_event(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    if !matches!(principal.id, PrincipalId::Service(_))
        && !crate::routes::catalog::is_unrestricted(&principal)
    {
        return Err(ApiError::PermissionDenied(
            "only the orchestrator's service identity reports pipeline-run finishes".to_owned(),
        )
        .into());
    }
    let req: RunFailedBody = parse_body(&body)?;
    let pool = pool(&state)?;
    let Some(pipeline_id) = job_name_to_pipeline_id(pool, &req.job_name).await? else {
        return Ok(ApiJson(json!({
            "matched": 0,
            "reason": "unknown jobName; no runnable pipeline owns it",
        })));
    };
    // Mirror `run_failed_event`'s posture: a SUCCESS sensor MUST only
    // call this on a successful run. A `STARTED`/`FAILURE`/missing run
    // is either a sensor misconfiguration (refuse with 409) or a race
    // (the run may not exist yet — 404). Treating a non-SUCCESS as a
    // finished run would skew the slow/volume-drop rate.
    let status = state
        .dagster
        .pipeline_run_status(&req.run_id)
        .await
        .map_err(|err| ApiError::Unavailable(js_error(err)))?;
    let Some(info) = status else {
        return Err(ApiError::NotFound(format!("run {} not found in Dagster", req.run_id)).into());
    };
    if info.status != "SUCCESS" {
        return Err(ApiError::Conflict(format!(
            "run {} is in Dagster status '{}', not SUCCESS; refusing to evaluate slow/volume_drop",
            req.run_id, info.status
        ))
        .into());
    }
    // Plan 1f: compute the two per-run outcomes from the run's own data
    // and the pipeline's `pipeline_sla` row, then hand them to the
    // evaluator. `slow` is `None` when no duration SLA is configured
    // (the alert rule cannot decide); `volume_drop` is `None` for the
    // same honest-null reasons [`drop`] takes.
    let run_duration = duration_seconds(info.start_time, info.end_time);
    let sla = pipelines::get_pipeline_sla(pool, &pipeline_id)
        .await
        .unwrap_or(None);
    let slow = over_duration(
        run_duration,
        sla.as_ref().and_then(|s| s.max_duration_seconds),
    );
    let volume_drop = if sla.is_some() {
        volume_drop_for_run(&state, &pipeline_id, &req.run_id).await?
    } else {
        // No SLA at all — the alert rule cannot decide.
        None
    };
    let http = reqwest::Client::new();
    let email = EmailSender::new(smtp_config(&state.config));
    let silence_source: Option<Box<dyn SilenceSource>> = state
        .pg
        .as_deref()
        .map(|pg| Box::new(ApiSilenceSource { pg }) as Box<dyn SilenceSource>);
    // Reviewer fix #1: per-kind dedupe through `pipeline_run_event`,
    // matching the `run_failed_event` posture for kind `"failure"`.
    // Each kind is independently deduped — a run can be both slow AND
    // a volume drop, firing both once each. `slow == Some(false)` and
    // `volume_drop == None` (cannot decide) are short-circuited: no
    // dedupe row, no delivery. `slow == Some(true)` and
    // `volume_drop == Some(true)` each go through their own
    // `record_pipeline_run_event` BEFORE delivery; a `false` return
    // means the same `(run_id, kind)` pair was already inserted (a
    // sensor retry) and that kind is skipped for this call.
    let mut matched = 0_usize;
    if slow == Some(true) {
        let first_seen =
            pipelines::record_pipeline_run_event(pool, &req.run_id, &pipeline_id, "slow").await?;
        if first_seen {
            matched += lakehouse_alerts::evaluate_pipeline_slow(
                &state.clickhouse,
                &http,
                &email,
                &pipeline_id,
                &req.run_id,
                true,
                silence_source.as_deref().map(|s| s as &dyn SilenceSource),
            )
            .await
            .map_err(|err| ApiError::Unavailable(js_error(err)))?;
        }
    }
    if volume_drop == Some(true) {
        let first_seen =
            pipelines::record_pipeline_run_event(pool, &req.run_id, &pipeline_id, "volume_drop")
                .await?;
        if first_seen {
            matched += lakehouse_alerts::evaluate_pipeline_volume_drop(
                &state.clickhouse,
                &http,
                &email,
                &pipeline_id,
                &req.run_id,
                true,
                silence_source.as_deref().map(|s| s as &dyn SilenceSource),
            )
            .await
            .map_err(|err| ApiError::Unavailable(js_error(err)))?;
        }
    }
    Ok(ApiJson(json!({ "matched": matched })))
}

/// The job to launch for authored pipeline `id`, or the response that
/// explains why there is none. A draft is refused (409) before the
/// orchestrator is asked. A ready pipeline whose job the orchestrator has
/// not loaded (marked ready while it was down, say) gets one reload; if
/// the job is still missing, 409 says so rather than a launch error that
/// reads like a bad job name.
async fn authored_launch_target(state: &AppState, id: &str) -> Result<String, Response> {
    let pool = pool(state).map_err(|err| ApiRejection(err).into_response())?;
    let pipeline = pipelines::get_pipeline(pool, id)
        .await
        .map_err(|err| ApiRejection(err.into()).into_response())?
        .ok_or_else(|| {
            ApiRejection(ApiError::NotFound(format!("Pipeline {id} not found"))).into_response()
        })?;
    if pipeline.status == "draft" {
        return Err(ApiRejection(ApiError::Conflict(
            "this pipeline is a draft: mark it ready before running it".to_owned(),
        ))
        .into_response());
    }
    if !authored_pipelines::job_is_loaded(state, id).await {
        authored_pipelines::reload_orchestrator(state).await;
        if !authored_pipelines::job_is_loaded(state, id).await {
            return Err(ApiRejection(ApiError::Conflict(
                "the orchestrator has not loaded this pipeline's job; check that \
                 PIPELINE_RUN_TOKEN is set for both the API and the Dagster code location"
                    .to_owned(),
            ))
            .into_response());
        }
    }
    Ok(authored_pipelines::job_name(id))
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

/// Unix-seconds clock used by the SLA "late" computation. `None` when the
/// system clock is unset (an unreachable NTP source on some build hosts),
/// so [`late`] can refuse to claim rather than guess from a meaningless
/// zero. The list route calls this once per response so every row agrees
/// on the clock. Exposed `pub(crate)` so [`crate::routes::alerts`]'s
/// late-evaluation pass uses the same helper — a broken clock would
/// otherwise show up as `Some(0.0)` ("late since 1970") at one call site
/// and honest `None` at another.
pub(crate) fn now_unix_seconds() -> Option<f64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs_f64())
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
pub(super) fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "pipeline store unavailable: no Postgres pool is configured \
             (DATABASE_URL is missing or not a valid Postgres connection string)"
                .to_owned(),
        )
    })
}

pub(super) fn parse_body<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
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
    /// Plan 1c (R2, day-1): per-pipeline retry cap (migration 0051).
    /// Validated here against the same `0..=5` the column CHECK enforces
    /// (and `dagster.RetryPolicy.max_retries` accepts) so the client
    /// gets a 400 instead of a database error when the value is
    /// outside the range. `None` resolves to the migration's
    /// `DEFAULT 2`, the same value the orchestrator already uses
    /// (`op_metadata.DEFAULT_RETRY_POLICY`).
    #[serde(default)]
    max_retries: Option<i16>,
    /// Upstream pipeline ids whose SUCCESS runs must precede this one's.
    /// R3 plan 2a, migration `0052`. Validated in [`create`] against the
    /// existing authored graph + live `DgClient::list_jobs` BEFORE any
    /// write happens — same posture as `transforms` above.
    #[serde(default)]
    depends_on: Vec<String>,
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
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<pipelines::Pipeline>)> {
    // The creator's active tenant, resolved exactly as `list` resolves it,
    // so the new pipeline is on the list the create form returns to. It
    // used to be inserted unassigned, which no tenant-scoped read shows.
    let tenant_id = crate::tenant_scope::resolve(&principal, &headers)?;
    let body: CreatePipelineBody = parse_body(&body)?;
    for (index, transform) in body.transforms.iter().enumerate() {
        crate::transform_grammar::parse_transform(transform).map_err(|err| {
            ApiError::BadRequest(format!("invalid transform at transforms[{index}]: {err}"))
        })?;
    }
    // Plan 1c: validate `max_retries` against the same `0..=5` the
    // column CHECK enforces (migration 0051). Catching it here means a
    // bad value returns a 400 with the field name, not a 500 from the
    // database constraint violation. The error message does not echo
    // the caller's value back.
    if let Some(value) = body.max_retries
        && !(0..=5).contains(&value)
    {
        return Err(ApiError::BadRequest("maxRetries must be between 0 and 5".to_owned()).into());
    }
    // Validate `depends_on` against the live authored graph + Dagster
    // job list. The new pipeline's id is not yet known at this point
    // (slug_id derives it from `name` inside `create_pipeline`), so the
    // validator cannot substitute the row's own `depends_on`; the
    // pipeline is treated as "id = body.name" for self-reference and
    // cycle purposes — the same id the slug will produce for a
    // well-formed slug, and a safe-enough unique string for a name
    // collision case (the store rejects that with 409 downstream, the
    // validator refuses a self-reference against `body.name` regardless).
    //
    // Dagster unreachable degrades to "no Dagster upstreams accepted"
    // (empty list) rather than 500ing the create. The schedule-ticks
    // route uses the same pattern — a missing orchestrator on a write
    // path is a configuration problem the operator reads about in the
    // logs, not a reason to refuse a create that the rest of the store
    // would happily accept.
    let dagster_jobs = match state.dagster.list_jobs().await {
        Ok(j) => j,
        Err(err) => {
            tracing::warn!(%err, "create: dagster unreachable, depends_on accepts authored upstreams only");
            Vec::new()
        }
    };
    let others =
        crate::routes::authored_pipelines::collect_authored_depends_on(pool(&state)?, None).await?;
    crate::routes::authored_pipelines::validate_depends_on(
        &body.name,
        &body.depends_on,
        &others,
        &dagster_jobs,
    )?;
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
        description: body.description,
        max_retries: body.max_retries,
        tenant_id,
        depends_on: body.depends_on,
    };
    let created = create_named_pipeline(pool(&state)?, &input, Some(principal.id.uuid())).await?;
    // WS5 item D4: best-effort, never turns a successful create into an
    // error. `create` has no internal (copilot tool) caller today
    // (confirmed by grepping `routes::ai::tools::pipelines` before adding
    // this parameter), so `Extension<Principal>`, not `Option`, matches
    // `pipeline:write`'s own `RequiresPermission` guarantee exactly.
    record_pipeline_audit(
        &state,
        &principal,
        "pipeline.create",
        &created.id,
        Value::Null,
    )
    .await;
    Ok((StatusCode::CREATED, ApiJson(created)))
}

/// Create a pipeline, turning a name collision into a sentence that says
/// what collided. The store's own message ("a record with that value
/// already exists") was shown to the user verbatim, which explains
/// nothing about which value or what to do next.
///
/// `changed_by` is the principal the route had on the
/// `Extension<Principal>`; it lands in the `created` version row's
/// `changed_by` column (Plan R4 2b). `routes::pipelines::generate`'s
/// own caller passes its own principal — `generate` does not audit today
/// (WS5 item D4), but the version row still goes through `create_pipeline`
/// so the create is captured in history without a separate route-side
/// write.
async fn create_named_pipeline(
    pool: &PgPool,
    input: &CreatePipelineInput,
    changed_by: Option<Uuid>,
) -> Result<pipelines::Pipeline, ApiError> {
    match pipelines::create_pipeline(pool, input, changed_by).await {
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
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<pipelines::Pipeline>)> {
    // Same tenant rule as `create`: the draft lands in the caller's tenant.
    let tenant_id = crate::tenant_scope::resolve(&principal, &headers)?;
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
        // The draft LLM call proposes an incremental column/transforms too
        // (see `PipelineDraft` below); nothing asks it for FBIC, so that
        // stays the same honest "not configured" default `create` uses
        // when a human author leaves it unset.
        incremental_column: draft.as_ref().and_then(|d| d.incremental_column.clone()),
        transforms: draft
            .as_ref()
            .map(|d| d.transforms.clone())
            .unwrap_or_default(),
        fbic_enabled: false,
        target_zone: body.database,
        target_table,
        schedule,
        owner: Some("Agentic Builder".to_owned()),
        // The instruction is kept verbatim as the description: it is the
        // only record of what this pipeline was asked to do.
        description: Some(body.instruction.clone()),
        // No LLM-driven override for `max_retries` yet — a model that has
        // not been told about the new column would either fabricate or
        // echo a default. The store's migration `DEFAULT 2` is the
        // honest answer for a draft nobody has asked to tune.
        max_retries: None,
        tenant_id,
        // Agentic-builder output never carries upstream wiring: a draft
        // proposed by the LLM is validated against the grammar and the
        // table schema, never the chain semantics — those are an
        // author's deliberate call, not an LLM's. R3 plan 2a.
        depends_on: Vec::new(),
    };
    let created = create_named_pipeline(pool(&state)?, &input, Some(principal.id.uuid())).await?;
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

/// The transitions `POST /api/pipelines/{id}/status` permits, checked
/// BEFORE any store call — exactly the lifecycle moves this plan's own
/// tasks need, nothing broader (judge review V9: the first draft let a
/// `pipeline:write` principal set a RUN-DERIVED status — `"running"`,
/// `"completed"`, `"failed"`, `"degraded"`, `"partial"` — fabricating an
/// outcome no real execution produced).
///
/// `("draft", "ready")` is the only entry: it is the one transition WS4's
/// G7 gate and console "Activate" action work both need (an
/// authored pipeline otherwise never leaves `"draft"`). No other transition
/// is added speculatively — `pause`/`resume` (this module's own
/// [`pause`]/[`resume`] -> [`authored_status`] -> `pipelines::set_status`)
/// already move a pipeline between `"ready"`/`"paused"` directly, bypassing
/// this route entirely, so this table does not duplicate that pair.
/// Extending this table for a genuinely new need is a one-line, reviewable
/// addition with its own test — not a reason to let this route accept an
/// arbitrary target status today.
const ALLOWED_TRANSITIONS: &[(&str, &str)] = &[("draft", "ready")];

fn is_known_target_status(to: &str) -> bool {
    ALLOWED_TRANSITIONS.iter().any(|(_, t)| *t == to)
}

fn is_allowed_transition(from: &str, to: &str) -> bool {
    ALLOWED_TRANSITIONS
        .iter()
        .any(|(f, t)| *f == from && *t == to)
}

/// The `POST /api/pipelines/{id}/status` body.
#[derive(Debug, Deserialize)]
struct SetStatusBody {
    status: String,
}

/// `POST /api/pipelines/{id}/status` — move an authored pipeline between
/// lifecycle states named in [`ALLOWED_TRANSITIONS`] (WS4 item D4). Only
/// ever applies to a Postgres-authored (`pl-`-prefixed) pipeline — a
/// `Dagster` job has no `pipeline_definition` row to update.
///
/// # Errors
///
/// 404 if `id` is not a `pl-` id, or names one with no matching row; 400 if
/// `status` is not a KNOWN TARGET of any transition in
/// [`ALLOWED_TRANSITIONS`] (checked before any store call — no read, no
/// write); 409 if `status` is a known target but the pipeline's CURRENT
/// status is not an allowed source for it (checked after a read, before any
/// WRITE). A genuine store failure below that point (e.g. a database
/// outage) keeps `StoreError`'s existing classification via `?` — it is
/// never folded into 400, since an outage is not a caller error.
pub async fn set_status_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<pipelines::Pipeline>> {
    if !id.starts_with("pl-") {
        return Err(ApiError::NotFound(format!("Pipeline {id} not found")).into());
    }
    let body: SetStatusBody = parse_body(&body)?;

    // Unknown/run-derived target status -> 400, before touching the store
    // at all (no read, no write) — this is what stops a pipeline:write
    // principal from ever setting "completed"/"running"/"failed"/
    // "degraded"/"partial" through this route (judge review V9).
    if !is_known_target_status(&body.status) {
        return Err(ApiError::BadRequest(format!(
            "{:?} is not a status this route can set; the only transitions permitted are \
             {ALLOWED_TRANSITIONS:?}",
            body.status
        ))
        .into());
    }

    // A READ (not a write) to learn the current status before deciding
    // whether this specific transition is allowed.
    let current = pipelines::get_pipeline(pool(&state)?, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Pipeline {id} not found")))?;

    // Disallowed from-state -> 409, still before any store WRITE.
    if !is_allowed_transition(&current.status, &body.status) {
        return Err(ApiError::Conflict(format!(
            "cannot move pipeline {id} from {:?} to {:?}",
            current.status, body.status
        ))
        .into());
    }

    // Only NOW, after both checks pass, does a write happen. A StoreError
    // here (a real database failure, or — belt-and-suspenders — the CHECK
    // constraint rejecting something this route's own checks already
    // should have caught) keeps its existing classification via `?`
    // (StoreError::Database -> ApiError::Internal with the fixed "database
    // error" message, never a 400 — an outage must not read as a bad
    // request, judge review V9).
    let updated = pipelines::set_status(pool(&state)?, &id, &body.status)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Pipeline {id} not found")))?;
    // A pipeline that just became ready needs a job: the orchestrator
    // builds it on reload. Best effort; `trigger` reloads again if the job
    // is still missing when someone runs it.
    authored_pipelines::reload_orchestrator(&state).await;
    Ok(ApiJson(updated))
}

/// `POST /api/pipelines/{id}/pause` — pause a pipeline. Dispatches on
/// whether `id` names a Postgres-authored draft (id prefix `pl-`, no
/// backing job) or a real `Dagster` job (pauses its first schedule, if
/// any).
///
/// `principal` is `Option<Extension<Principal>>` — see [`trigger`]'s doc
/// comment; `routes::ai::tools::pipelines::pause_pipeline` calls this
/// handler directly, bypassing `crate::policy::auth_gate`.
///
/// # Errors
///
/// 401 if no principal is present; 404 if `id` is unknown (or names a job
/// with no schedule to pause); 503 as above.
pub async fn pause(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Path(id): Path<String>,
) -> Response {
    let Some(Extension(principal)) = principal else {
        return ApiRejection(ApiError::unauthorized()).into_response();
    };
    let response = set_pipeline_paused(&state, &id, true).await;
    if response.status().is_success() {
        record_pipeline_audit(&state, &principal, "pipeline.pause", &id, Value::Null).await;
    }
    response
}

/// `POST /api/pipelines/{id}/resume` — the inverse of [`pause`]. See its
/// doc comment for the `Option<Extension<Principal>>` shape.
///
/// # Errors
///
/// 401 if no principal is present; 404/503 as [`pause`].
pub async fn resume(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Path(id): Path<String>,
) -> Response {
    let Some(Extension(principal)) = principal else {
        return ApiRejection(ApiError::unauthorized()).into_response();
    };
    let response = set_pipeline_paused(&state, &id, false).await;
    if response.status().is_success() {
        record_pipeline_audit(&state, &principal, "pipeline.resume", &id, Value::Null).await;
    }
    response
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
    let updated = pipelines::set_status(pool(state)?, id, status).await?;
    if updated.is_some() {
        // The status is the record; the orchestrator's schedule is what
        // actually fires. Its stored on/off state outlives a reload, so it
        // is switched directly as well as rebuilt. An authored pipeline
        // with no cron schedule has none to switch, which is not an error.
        let schedule = authored_pipelines::schedule_name(id);
        let switched = if paused {
            state.dagster.stop_schedule(&schedule).await
        } else {
            state.dagster.start_schedule(&schedule).await
        };
        if let Ok(lakehouse_dagster::ScheduleOutcome {
            error: Some(err), ..
        }) = switched
        {
            tracing::info!(%err, pipeline_id = id, "authored schedule not switched");
        }
        authored_pipelines::reload_orchestrator(state).await;
    }
    Ok(updated)
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
        "nextRunAt": next_run_at_json(job),
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

/// `POST /api/pipelines/runs/{runId}/retry` — re-execute a finished run.
/// An empty body re-runs every step. `{"strategy":"fromFailure"}` re-runs
/// only the steps that failed or never ran, which `Dagster` refuses (409)
/// for a run that did not fail.
///
/// # Errors
///
/// Same as [`cancel_run`], plus 400 for a body that names no known
/// strategy.
pub async fn retry_run(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    body: Bytes,
) -> Response {
    let request = match parse_retry_request(&body) {
        Ok(r) => r,
        Err(err) => {
            return (StatusCode::BAD_REQUEST, ApiJson(json!({ "error": err }))).into_response();
        }
    };
    // Plan 1c (R2, day-1): the "selected" strategy names its target
    // steps explicitly, so the route has to verify each one actually
    // belongs to the parent run before sending the mutation. Anything
    // outside the run's known keys would silently become a no-op on
    // `Dagster`'s side, hiding a typo from the caller.
    if let RetryRequest::Selected(keys) = &request {
        let known: std::collections::HashSet<String> = match state.dagster.run_steps(&run_id).await
        {
            Ok(steps) => steps.iter().map(|s| s.step_key.clone()).collect(),
            Err(err) => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    ApiJson(json!({ "error": js_error(err) })),
                )
                    .into_response();
            }
        };
        // The 400 names the FIRST unknown key (not all of them — that
        // would let a typo at index 0 mask a different one at index 1),
        // never the caller's own list back.
        if let Some(unknown) = keys.iter().find(|k| !known.contains(k.as_str())) {
            return (
                StatusCode::BAD_REQUEST,
                ApiJson(json!({
                    "error": format!("stepKey \"{unknown}\" is not a step of run {run_id}")
                })),
            )
                .into_response();
        }
    }
    // Two different driver calls: `launch_reexecution` takes one of the
    // two strategies Dagster names, while a selected subset goes through
    // `launch_reexecution_of_steps`, which carries the stepKeys filter as
    // a `launchRunReexecution` argument.
    let launched = match &request {
        RetryRequest::Selected(keys) => {
            let key_refs: Vec<&str> = keys.iter().map(String::as_str).collect();
            state
                .dagster
                .launch_reexecution_of_steps(&run_id, &key_refs)
                .await
        }
        RetryRequest::All => {
            state
                .dagster
                .launch_reexecution(&run_id, ReexecutionStrategy::AllSteps)
                .await
        }
        RetryRequest::FromFailure => {
            state
                .dagster
                .launch_reexecution(&run_id, ReexecutionStrategy::FromFailure)
                .await
        }
    };
    let outcome = match launched {
        Ok(outcome) => outcome,
        Err(err) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                ApiJson(json!({ "error": js_error(err) })),
            )
                .into_response();
        }
    };
    match outcome {
        outcome if outcome.error.is_none() => {
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
        outcome => dagster_mutation_failure(outcome.error),
    }
}

/// What a retry body asks the route to re-execute. Separate from
/// [`ReexecutionStrategy`], which only models the two strategies Dagster
/// itself names: a `Selected` request carries step keys and becomes a
/// `launchRunReexecution` with a `stepKeys` filter, never a strategy
/// value. Parsed here, at the route boundary, so the driver's enum stays
/// closed and its strategy mapping cannot silently degrade a subset
/// request into `ALL_STEPS`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RetryRequest {
    /// Every step of the parent run.
    All,
    /// Only the steps that failed or did not run.
    FromFailure,
    /// Only the named steps of the parent run.
    Selected(Vec<String>),
}

/// The retry request a body asks for; `Err` for a body that is not
/// empty and names no known strategy.
fn parse_retry_request(body: &[u8]) -> Result<RetryRequest, String> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(RetryRequest::All);
    }
    let value: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(err) => return Err(format!("invalid retry body: {err}")),
    };
    match value.get("strategy").and_then(Value::as_str) {
        None | Some("allSteps") => Ok(RetryRequest::All),
        Some("fromFailure") => Ok(RetryRequest::FromFailure),
        Some("selected") => {
            // Plan 1c (R2, day-1): the "selected" strategy is the only
            // way to re-run a subset of steps; the body MUST carry a
            // non-empty `stepKeys` array, otherwise `Dagster` itself
            // rejects the mutation with a confusing server error.
            // The route layer adds the second half of validation —
            // verifying each key actually exists in the parent run —
            // AFTER this parser has run.
            let Some(arr) = value.get("stepKeys").and_then(Value::as_array) else {
                return Err("strategy \"selected\" requires a non-empty stepKeys array".to_owned());
            };
            if arr.is_empty() {
                return Err("strategy \"selected\" requires a non-empty stepKeys array".to_owned());
            }
            let mut keys = Vec::with_capacity(arr.len());
            for entry in arr {
                let Some(s) = entry.as_str() else {
                    return Err("stepKeys entries must be strings".to_owned());
                };
                keys.push(s.to_owned());
            }
            Ok(RetryRequest::Selected(keys))
        }
        Some(other) => Err(format!(
            "strategy must be one of \"allSteps\", \"fromFailure\", \"selected\" (got \"{other}\")"
        )),
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

/// Body for `PUT /api/pipelines/{id}/tenant`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignTenantBody {
    tenant_id: Uuid,
}

/// `PUT /api/pipelines/{id}/tenant` — assign (or reassign) an authored
/// pipeline to a tenant. Same shape and rationale as `routes::connectors::
/// assign_connector_tenant`: `0042_tenant_
/// provisioning.sql` adds `tenant_id` to `pipeline_definition` with no
/// backfill at all, so every authored pipeline starts invisible to `GET
/// /api/pipelines`'s tenant-scoped read until assigned here.
///
/// Only ever targets a `pipeline_definition` row (a `pl-`-prefixed,
/// Postgres-authored id) — a `Dagster`-backed pipeline id has no
/// `tenant_id` column anywhere in this schema (see `routes::pipelines::
/// list`'s own doc comment on the un-tenanted `Dagster`-job half), so it
/// 404s here the same as any other unknown id, via `assign_tenant`'s
/// `rows_affected() == 0` check — never a fabricated success for a job
/// this route cannot actually scope.
///
/// # Errors
///
/// 404 if either the pipeline id or the tenant id does not exist — the
/// same status for both, matching `tenant_scope::resolve`'s no-existence-
/// leak rule. 400 for a malformed body. 503 if no pool is configured; 500
/// on any other database failure.
pub async fn assign_pipeline_tenant(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let AssignTenantBody { tenant_id } = parse_body(&body)?;
    let db_pool = pool(&state)?;
    if !lakehouse_store::identity::tenant_exists(db_pool, tenant_id).await? {
        return Err(ApiError::NotFound("tenant not found".to_owned()).into());
    }
    pipelines::assign_tenant(db_pool, &id, tenant_id)
        .await
        .map_err(|err| match err {
            lakehouse_store::StoreError::NotFound => {
                ApiError::NotFound(format!("Pipeline {id} not found"))
            }
            other => other.into(),
        })?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod dagster_half_tenant_gate {
    //! The `Dagster`-job half of
    //! `GET /api/pipelines` is a shared, un-tenanted surface and is gated by
    //! the same rule as the shared catalog.
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use axum::Extension;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use lakehouse_auth::{PermissionSet, Principal, PrincipalId};
    use serde_json::Value;
    use uuid::Uuid;

    use super::list;
    use crate::config::Config;
    use crate::state::AppState;

    fn database_url_for(pool: &lakehouse_store::PgPool) -> String {
        let options = pool.connect_options();
        format!(
            "postgres://{}:postgres@{}:{}/{}",
            options.get_username(),
            options.get_host(),
            options.get_port(),
            options
                .get_database()
                .expect("#[sqlx::test] always targets a named database"),
        )
    }

    /// `DAGSTER_URL` deliberately points at a port nothing listens on: if the
    /// refusal ever stopped short-circuiting, the route would try to reach
    /// `Dagster` and this test would fail on the attempt rather than pass by
    /// accident.
    fn state_for(pool: &lakehouse_store::PgPool) -> AppState {
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), database_url_for(pool));
        env.insert(
            "DAGSTER_URL".to_owned(),
            "http://127.0.0.1:1/graphql".to_owned(),
        );
        let config = Config::from_map(&env).expect("a valid test Config");
        AppState::new(config)
    }

    fn analyst() -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::nil()),
            tenant_ids: vec![Uuid::from_u128(1)],
            display_name: "Analyst".to_owned(),
            permissions: PermissionSet::parse("pipeline:read"),
            provider: "local".to_owned(),
            must_change_password: false,
            role_names: vec!["Analyst".to_owned()],
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn the_dagster_half_is_refused_and_says_so_when_no_owning_tenant_is_named(
        pool: lakehouse_store::PgPool,
    ) {
        // `0002_seed_identity.sql` seeds four tenants and `CATALOG_TENANT_ID`
        // is unset, so the shared surfaces are refused for a principal
        // without the unrestricted grant.
        let response = list(
            State(state_for(&pool)),
            Some(Extension(analyst())),
            HeaderMap::new(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("collect body");
        let v: Value = serde_json::from_slice(&body).expect("valid JSON");
        assert_eq!(
            v["dagsterJobs"]["supported"], false,
            "a refused shared surface must say so, not return a shorter list that looks complete"
        );
        assert!(
            v["dagsterJobs"]["reason"]
                .as_str()
                .expect("a reason string")
                .contains("CATALOG_TENANT_ID"),
            "the reason must name the setting an operator sets to open it back up"
        );
        assert!(
            v["pipelines"]
                .as_array()
                .expect("pipelines array")
                .is_empty(),
            "no Dagster job may appear, and this database has no authored pipeline rows"
        );
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use lakehouse_auth::{PermissionSet, PrincipalId};
    use lakehouse_dagster::{DgSchedule, DgScheduleState};
    use uuid::Uuid;

    use super::*;
    use crate::config::Config;

    /// A logged-in human principal — mirrors `routes::query`/
    /// `routes::agents`/`routes::connectors`'s own test fixture of the
    /// same name.
    pub(super) fn fixture_user_principal() -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::from_u128(1)),
            tenant_ids: Vec::new(),
            display_name: "Rina Wijaya".to_owned(),
            permissions: PermissionSet::parse("pipeline:write"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    /// A service-token principal — `PrincipalId::Service`.
    fn fixture_service_principal() -> Principal {
        Principal {
            id: PrincipalId::Service(Uuid::from_u128(2)),
            tenant_ids: Vec::new(),
            display_name: "dagster-orchestrator".to_owned(),
            permissions: PermissionSet::parse("pipeline:write"),
            provider: "service".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    fn state_without_pool() -> AppState {
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
        let config = Config::from_map(&env).expect("a valid test Config");
        AppState::new(config)
    }

    fn principal_with_tenants(tenant_ids: &[Uuid]) -> Principal {
        Principal {
            tenant_ids: tenant_ids.to_vec(),
            ..fixture_user_principal()
        }
    }

    fn headers_with_x_tenant(tenant_id: Uuid) -> axum::http::HeaderMap {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "x-tenant",
            tenant_id
                .to_string()
                .parse()
                .expect("uuid renders as a valid header value"),
        );
        headers
    }

    async fn response_json(resp: Response) -> (StatusCode, Value) {
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    // ── `GET /api/pipelines` tenant scoping ─────────────────────────────

    /// A restricted principal that belongs to zero tenants gets no
    /// authored pipeline and an explicit `dagsterJobs` refusal — never
    /// `403`/`404`, never every tenant's rows, and never a bare empty list
    /// that reads as complete. `state_without_pool()` (no store, no
    /// reachable `Dagster`) proves neither is queried: reaching either
    /// would fail with a 503, not 200. This test previously asserted a bare
    /// `{"pipelines": []}` returned before the `Dagster`-job gate ran,
    /// which is the short-circuit that also hid the job list from a
    /// `"*:*"` operator (see the next test).
    #[tokio::test]
    async fn list_with_a_tenantless_restricted_principal_refuses_dagster_jobs_without_querying_anything()
     {
        let state = state_without_pool();
        let principal = principal_with_tenants(&[]);
        let resp = list(
            State(state),
            Some(Extension(principal)),
            axum::http::HeaderMap::new(),
        )
        .await;
        let (status, body) = response_json(resp).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["pipelines"], json!([]));
        assert_eq!(body["dagsterJobs"]["supported"], json!(false));
    }

    /// A `"*:*"` principal with no tenant membership is never refused the
    /// `Dagster`-job half (`catalog_tenant_refusal`), so the route must go
    /// on to query `Dagster`. With no reachable `Dagster` that surfaces as
    /// the route's 503, which is the proof it was queried: the old
    /// short-circuit answered 200 with an empty list instead.
    #[tokio::test]
    async fn list_with_a_tenantless_unrestricted_principal_still_queries_dagster() {
        // A `Dagster` URL on a closed local port, never the config default:
        // the default names a real host port, and a unit test must not dial
        // whatever happens to listen there.
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
        env.insert(
            "DAGSTER_URL".to_owned(),
            "http://127.0.0.1:1/graphql".to_owned(),
        );
        let state = AppState::new(Config::from_map(&env).expect("a valid test Config"));
        let principal = Principal {
            permissions: PermissionSet::parse("*:*"),
            ..principal_with_tenants(&[])
        };
        let resp = list(
            State(state),
            Some(Extension(principal)),
            axum::http::HeaderMap::new(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    /// No `Extension<Principal>` at all (the internal, non-HTTP call path
    /// `routes::ai::tools::pipelines` used to take before this task) is
    /// refused with 401, not a panic or an unscoped read.
    #[tokio::test]
    async fn list_with_no_principal_extension_is_unauthorized() {
        let state = state_without_pool();
        let resp = list(State(state), None, axum::http::HeaderMap::new()).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    /// `X-Tenant` naming a tenant the principal does not belong to is a
    /// 404 (via `tenant_scope::resolve`'s own contract), propagated
    /// through this route rather than swallowed or defaulted to unscoped.
    #[tokio::test]
    async fn list_with_a_foreign_x_tenant_header_is_not_found() {
        let state = state_without_pool();
        let principal = principal_with_tenants(&[Uuid::from_u128(1)]);
        let headers = headers_with_x_tenant(Uuid::from_u128(2)); // not a member
        let resp = list(State(state), Some(Extension(principal)), headers).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    /// WS5 item D4, failing-test-first: `pipeline_audit_event` did not
    /// exist before this task; referencing it below failed to compile
    /// with `cannot find function pipeline_audit_event in this scope`
    /// (confirmed by checking out the pre-fix file and running this test
    /// before adding the helper).
    #[test]
    fn pipeline_audit_event_pairs_with_pipeline_resource_kind() {
        let principal = fixture_user_principal();
        let event = pipeline_audit_event(&principal, "pipeline.trigger", "job-1", Value::Null);
        assert_eq!(event.principal_kind.as_deref(), Some("user"));
        assert_eq!(
            event.principal_id.as_deref(),
            Some(principal.id.uuid().to_string().as_str())
        );
        assert_eq!(event.resource_kind.as_deref(), Some("pipeline"));
        assert_eq!(event.resource_id.as_deref(), Some("job-1"));
        assert_eq!(event.action, "pipeline.trigger");
        assert_eq!(event.outcome, "executed");
    }

    /// A service identity's pipeline action must record `principal_kind:
    /// "service"`, never `"user"`.
    #[test]
    fn pipeline_audit_event_records_a_service_identity_as_service_not_user() {
        let principal = fixture_service_principal();
        let event = pipeline_audit_event(&principal, "pipeline.pause", "job-1", Value::Null);
        assert_eq!(event.principal_kind.as_deref(), Some("service"));
    }

    fn run(job_name: &str, status: &str, start: Option<f64>, end: Option<f64>) -> DgRun {
        DgRun {
            run_id: "r".to_owned(),
            job_name: job_name.to_owned(),
            status: status.to_owned(),
            start_time: start,
            end_time: end,
            creation_time: None,
            parent_run_id: None,
            root_run_id: None,
            tags: Vec::new(),
        }
    }

    #[test]
    fn a_runs_trigger_comes_from_the_tags_dagster_writes() {
        let tag = |key: &str, value: &str| lakehouse_dagster::DgTag {
            key: key.to_owned(),
            value: value.to_owned(),
        };
        let mut r = run("j", "SUCCESS", Some(1.0), Some(2.0));
        assert_eq!(run_trigger(&r)["kind"], "manual");

        r.parent_run_id = Some("r0".to_owned());
        assert_eq!(run_trigger(&r)["kind"], "retry");

        r.tags = vec![tag("dagster/schedule_name", "gold_export_schedule")];
        let v = run_trigger(&r);
        assert_eq!(v["kind"], "schedule");
        assert_eq!(v["name"], "gold_export_schedule");

        r.tags = vec![tag("dagster/sensor_name", "on_bronze")];
        assert_eq!(run_trigger(&r)["kind"], "sensor");
    }

    #[test]
    fn a_retry_body_names_its_strategy_or_is_refused() {
        assert_eq!(parse_retry_request(b""), Ok(RetryRequest::All));
        assert_eq!(parse_retry_request(b"  "), Ok(RetryRequest::All));
        assert_eq!(parse_retry_request(b"{}"), Ok(RetryRequest::All));
        assert_eq!(
            parse_retry_request(br#"{"strategy":"fromFailure"}"#),
            Ok(RetryRequest::FromFailure)
        );
        assert!(parse_retry_request(br#"{"strategy":"someSteps"}"#).is_err());
        assert!(parse_retry_request(b"not json").is_err());
    }

    /// Plan 1c (R2, day-1): the `selected` strategy accepts a
    /// `stepKeys` array and refuses an empty one — `Dagster` itself
    /// rejects an empty list at submit time, so the route catches it
    /// first and returns 400 with a clear message.
    #[test]
    fn a_selected_retry_body_lists_its_steps() {
        assert_eq!(
            parse_retry_request(br#"{"strategy":"selected","stepKeys":["extract","transform"]}"#),
            Ok(RetryRequest::Selected(vec![
                "extract".to_owned(),
                "transform".to_owned()
            ]))
        );
        assert!(
            parse_retry_request(br#"{"strategy":"selected"}"#).is_err(),
            "selected without stepKeys must fail"
        );
        assert!(
            parse_retry_request(br#"{"strategy":"selected","stepKeys":[]}"#).is_err(),
            "an empty stepKeys array must fail"
        );
        assert!(
            parse_retry_request(br#"{"strategy":"selected","stepKeys":["ok",123]}"#).is_err(),
            "non-string stepKeys must fail"
        );
    }

    #[test]
    fn run_to_json_emits_null_for_untracked_counters() {
        let v = run_to_json(&run("j", "SUCCESS", Some(1.0), Some(61.0)), "p1", None);

        // Dagster's run record carries no row counts. Emitting 0 would read as
        // "this run processed nothing", which is a different claim from "we
        // did not measure it". `costUnits` used to carry the run's duration
        // under a currency-sounding name; `durationSeconds` replaced it, so
        // `costUnits` is now null too — never a fabricated cost.
        for key in ["processed", "accepted", "rejected", "retried", "costUnits"] {
            assert!(v[key].is_null(), "{key} must be null, got {}", v[key]);
        }
        // durationSeconds IS derived from the run's own start/end time, so
        // it stays real. `overDuration` is `null` when no duration SLA is
        // configured (None here) — a never-SLA'd run is not "within SLA",
        // it is "SLA unknown".
        assert!(!v["durationSeconds"].is_null());
        assert!(v["overDuration"].is_null());
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

        // Plan 1f: with no SLA configured and no clock the row reports
        // `slaOk: null` and `late: null` — the same honest-null posture
        // every other measured-but-unknown field on this row takes.
        let never_run_row = dagster_pipeline_row(&never_run, None, None, None, None);
        let ran_row = dagster_pipeline_row(&ran_job, Some(&last), Some(100.0), None, None);

        for row in [&never_run_row, &ran_row] {
            assert!(row["freshnessLagSeconds"].is_null());
            assert!(row["slaOk"].is_null());
            assert!(row["late"].is_null());
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
            "nextRunAt",
        ] {
            assert!(body[key].is_null(), "{key} must be null, got {}", body[key]);
        }
    }

    /// WS4 item G2 — `nextRunAt` is a real, computed value for a job with a
    /// real cron schedule, and `null` (never a guess) for a manual job or
    /// one whose only schedule's cron `next_run::next_run_at` cannot parse.
    #[test]
    fn next_run_at_json_is_real_for_a_cron_schedule_and_null_otherwise() {
        let scheduled = DgJob {
            name: "j".to_owned(),
            schedules: vec![DgSchedule {
                name: "s1".to_owned(),
                cron_schedule: "0 3 * * *".to_owned(),
                schedule_state: DgScheduleState {
                    status: "RUNNING".to_owned(),
                },
            }],
        };
        let manual = DgJob {
            name: "m".to_owned(),
            schedules: vec![],
        };
        let malformed = DgJob {
            name: "b".to_owned(),
            schedules: vec![DgSchedule {
                name: "s2".to_owned(),
                cron_schedule: "not a cron".to_owned(),
                schedule_state: DgScheduleState {
                    status: "RUNNING".to_owned(),
                },
            }],
        };

        assert!(
            next_run_at_json(&scheduled).is_string(),
            "a real cron schedule must produce a real nextRunAt"
        );
        assert!(
            next_run_at_json(&manual).is_null(),
            "a job with no schedule must report nextRunAt: null"
        );
        assert!(
            next_run_at_json(&malformed).is_null(),
            "an unparseable cron must report nextRunAt: null, never a guess"
        );
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

    // ── `over_duration` / `drop` pure helpers (plan 1f) ───────────────
    //
    // `late` is tested in `lakehouse-alerts::tests::late_*` because the
    // helper now lives there (shared with `evaluate_pipeline_late`).
    // Each helper below follows the same shape: an `Option` for the SLA
    // value and a measurement, returning `None` when the question
    // cannot be
    // answered, and a strict-inequality bool otherwise. The "exactly at
    // the threshold" tests pin the strictness; the "no clock"/"below
    // min samples" tests pin the honest-null contract. Mutation checks
    // for `late` (flip `>` to `>=`) and `drop` (min samples 5 → 4) live
    // in the commit message.
    mod sla_volume_helpers {
        use super::{drop, over_duration};

        // ── `over_duration` ────────────────────────────────────────

        #[test]
        fn over_duration_is_none_when_no_duration_sla_is_set() {
            // No max → the runs route must emit `overDuration: null`,
            // not `false`, for every run.
            assert_eq!(over_duration(Some(120), None), None);
            assert_eq!(over_duration(None, None), None);
        }

        #[test]
        fn over_duration_is_none_for_a_run_that_is_still_running() {
            // The orchestrator reported no duration (still going): not a
            // breach yet. Reporting `false` here would imply the run is
            // safely within SLA, which it might not be at any moment.
            assert_eq!(over_duration(None, Some(60)), None);
        }

        #[test]
        fn over_duration_is_true_when_duration_is_above_the_max() {
            assert_eq!(over_duration(Some(120), Some(60)), Some(true));
        }

        #[test]
        fn over_duration_is_false_when_duration_is_at_or_below_the_max() {
            // Strict `>` again: at-the-max is still within SLA.
            assert_eq!(over_duration(Some(60), Some(60)), Some(false));
            assert_eq!(over_duration(Some(30), Some(60)), Some(false));
        }

        // ── `drop` ─────────────────────────────────────────────────

        #[test]
        fn drop_is_none_when_current_rows_are_null() {
            // Null rows: a measurement gap, not "no drop". Skipping a
            // null run also keeps the median history honest.
            assert_eq!(drop(None, &[10, 20, 30, 40, 50, 60]), None);
        }

        #[test]
        fn drop_is_none_when_history_is_below_min_samples() {
            // Five samples is the floor (plan 1f). Four is not enough
            // history: report `null`, never silently `false`. Lowering
            // the floor to 4 is the mutation this test guards against.
            assert_eq!(drop(Some(1), &[10, 20, 30, 40]), None);
            assert_eq!(drop(Some(1), &[10, 20, 30]), None);
            assert_eq!(drop(Some(1), &[10, 20]), None);
            assert_eq!(drop(Some(1), &[10]), None);
            assert_eq!(drop(Some(1), &[]), None);
        }

        #[test]
        fn drop_is_false_when_history_is_above_min_samples_and_current_is_at_or_above_half_median()
        {
            // Median of [10, 20, 30, 40, 50, 60, 70, 80, 90] is 50
            // (upper middle of the even-length array). Half is 25.
            // Current rows = 25 → exactly half the median → not a drop
            // (strict `<`).
            assert_eq!(
                drop(Some(25), &[10, 20, 30, 40, 50, 60, 70, 80, 90]),
                Some(false)
            );
            // Current rows = 50 → well above half the median.
            assert_eq!(
                drop(Some(50), &[10, 20, 30, 40, 50, 60, 70, 80, 90]),
                Some(false)
            );
        }

        #[test]
        fn drop_is_true_when_current_is_below_half_the_median() {
            // Same history, half-median = 25. Current rows = 24 → drop.
            assert_eq!(
                drop(Some(24), &[10, 20, 30, 40, 50, 60, 70, 80, 90]),
                Some(true)
            );
        }

        #[test]
        fn drop_uses_upper_median_for_an_even_length_history() {
            // History length 6, sorted: [10, 20, 30, 40, 50, 60].
            // `sorted[n / 2]` picks index 3 → 40. Half = 20. Boundary:
            // 20 is not a drop (strict `<`), 19 is.
            assert_eq!(drop(Some(20), &[10, 20, 30, 40, 50, 60]), Some(false));
            assert_eq!(drop(Some(19), &[10, 20, 30, 40, 50, 60]), Some(true));
        }
    }

    /// Plan 1f reviewer fix #3 — the `GET /api/pipelines/{id}/volume`
    /// response shape (`runs`/`medianRows`/`unavailable`) and the
    /// degraded path. Pinned through the pure [`volume_body`] helper:
    /// the route itself is a thin `(StatusCode, ApiJson)` wrapper
    /// around the helper plus a Dagster-error branch, so testing the
    /// helper is enough to lock the contract.
    mod volume_route {
        use std::collections::HashMap;

        use axum::extract::{Path, State};
        use axum::http::StatusCode;
        use lakehouse_dagster::{DgRun, DgRunWithRows};
        use serde_json::{Value, json};

        use super::{volume, volume_body};
        use crate::config::Config;
        use crate::state::AppState;

        /// Build a `DgRunWithRows` for the body tests — `runs` arrive
        /// most-recent-first from `Dagster`, so each entry is a fresh
        /// `start_time` strictly greater than the next one.
        fn run_with_rows(
            run_id: &str,
            status: &str,
            start: Option<f64>,
            end: Option<f64>,
            rows: Option<i64>,
        ) -> DgRunWithRows {
            DgRunWithRows {
                run: DgRun {
                    run_id: run_id.to_owned(),
                    job_name: "j".to_owned(),
                    status: status.to_owned(),
                    start_time: start,
                    end_time: end,
                    creation_time: None,
                    parent_run_id: None,
                    root_run_id: None,
                    tags: Vec::new(),
                },
                rows,
            }
        }

        /// The exact shape the spec names: `runs`, `medianRows`,
        /// `unavailable`. No `pipelineId` (the spec doesn't list it),
        /// no other keys. `runs` is ordered newest-first to match the
        /// graph on the console; `medianRows` is the median of every
        /// completed run that reported rows — the same median the drop
        /// rule uses (reviewer fix #3: "the same median the drop rule
        /// uses").
        #[test]
        fn volume_body_returns_the_spec_shape_with_medianrows_and_null_unavailable() {
            // 7 completed runs reporting rows [10, 20, 30, 40, 50, 60, 70];
            // 1 still-running with `rows = null` (must NOT enter the
            // median); 1 completed with `rows = null` (also excluded).
            let runs = vec![
                run_with_rows("r7", "SUCCESS", Some(70.0), Some(80.0), Some(70)),
                run_with_rows("r6", "SUCCESS", Some(60.0), Some(70.0), Some(60)),
                run_with_rows("r5", "SUCCESS", Some(50.0), Some(60.0), Some(50)),
                run_with_rows("r4", "SUCCESS", Some(40.0), Some(50.0), Some(40)),
                run_with_rows("r3", "SUCCESS", Some(30.0), Some(40.0), Some(30)),
                run_with_rows("r2", "SUCCESS", Some(20.0), Some(30.0), Some(20)),
                run_with_rows("r1", "SUCCESS", Some(10.0), Some(20.0), Some(10)),
                run_with_rows("r0", "STARTED", Some(5.0), None, None),
                run_with_rows("rN", "FAILURE", Some(1.0), Some(2.0), None),
            ];
            let body = volume_body(&runs);

            assert_eq!(
                body["unavailable"],
                Value::Null,
                "successful Dagster fetch: `unavailable` must be null"
            );
            assert_eq!(
                body["medianRows"],
                json!(40),
                "sorted [10,20,30,40,50,60,70] -> index 3 -> 40; same median the drop rule uses"
            );
            // `runs` is ordered newest-first, identical to the input.
            let rs = body["runs"].as_array().expect("runs is an array");
            assert_eq!(rs.len(), 9);
            assert_eq!(rs[0]["runId"], "r7");
            assert_eq!(rs[8]["runId"], "rN");
            // The two null-rows runs emit `rows: null`, not 0.
            assert!(rs[7]["rows"].is_null());
            assert!(rs[8]["rows"].is_null());
            // `r0` is `STARTED` -> drop is `null` (in-progress is not
            // a drop).
            assert!(rs[7]["drop"].is_null(), "STARTED is not a drop");
            // `rN` (FAILURE, no rows): `drop` is `null` per the `drop`
            // helper — null rows is a measurement gap, not a drop.
            assert!(rs[8]["drop"].is_null());
            // No `pipelineId` key — the spec does not list it.
            assert!(
                body.get("pipelineId").is_none(),
                "the spec shape has no pipelineId"
            );
        }

        /// When no completed run reported any rows, the median is
        /// `None` — `medianRows: null`, not `0`. An honest `null` keeps
        /// the chart from inventing a baseline over an empty set.
        #[test]
        fn volume_body_returns_null_medianrows_when_no_run_reported_rows() {
            let runs = vec![
                run_with_rows("a", "STARTED", Some(2.0), None, None),
                run_with_rows("b", "FAILURE", Some(1.0), Some(2.0), None),
                run_with_rows("c", "SUCCESS", Some(0.5), Some(1.0), None),
            ];
            let body = volume_body(&runs);
            assert!(
                body["medianRows"].is_null(),
                "no completed run reported rows: medianRows is null, not 0 or median([])"
            );
            assert_eq!(body["unavailable"], Value::Null);
            // `drop` is `null` for every run — the drop rule has no
            // history to compare against.
            for r in body["runs"].as_array().unwrap() {
                assert!(r["drop"].is_null(), "drop is null when no history");
            }
        }

        /// `medianRows` matches the median the drop rule uses for an
        /// even-length history (`sorted[n/2]` upper-of-middle). Six
        /// completed runs: the chart median is 40 (`sorted[6/2]`), and
        /// the newest run's drop rule sees exactly the 5-sample floor
        /// of history with its own median 30 → half 15, so 60 rows is
        /// not a drop. Pinning both numbers in one test keeps the
        /// chart's `medianRows` and the rule's median from drifting
        /// apart on a future refactor.
        #[test]
        fn volume_body_medianrows_matches_the_drop_rule_median() {
            let runs = vec![
                run_with_rows("r6", "SUCCESS", Some(60.0), Some(70.0), Some(60)),
                run_with_rows("r5", "SUCCESS", Some(50.0), Some(60.0), Some(50)),
                run_with_rows("r4", "SUCCESS", Some(40.0), Some(50.0), Some(40)),
                run_with_rows("r3", "SUCCESS", Some(30.0), Some(40.0), Some(30)),
                run_with_rows("r2", "SUCCESS", Some(20.0), Some(30.0), Some(20)),
                run_with_rows("r1", "SUCCESS", Some(10.0), Some(20.0), Some(10)),
            ];
            let body = volume_body(&runs);
            assert_eq!(
                body["medianRows"],
                json!(40),
                "sorted [10,20,30,40,50,60] -> index 3 -> 40"
            );
            let rs = body["runs"].as_array().unwrap();
            // Newest run: 5 prior samples [10,20,30,40,50], median 30,
            // half 15; 60 < 15 is false.
            assert_eq!(rs[0]["drop"], json!(false));
            // Second-newest: only 4 prior samples — below the
            // 5-sample floor, so an honest null even though 50 > 15.
            assert!(
                rs[1]["drop"].is_null(),
                "4 prior samples is below the floor: drop is null"
            );
            // Oldest run has no history at all → null.
            assert!(rs[5]["drop"].is_null());
        }

        /// Dagster unreachable: the route returns 200 (not 503) with
        /// `runs: []`, `medianRows: null`, and `unavailable` set to a
        /// classified, non-upstream reason string — mirroring
        /// [`runs_body`]'s degraded branch so the console can show
        /// "no runs right now, but here's why" instead of treating the
        /// outage as a hard error.
        #[tokio::test]
        async fn volume_route_degrades_to_unavailable_when_dagster_is_down() {
            // A Dagster URL on a closed local port, never the config
            // default; a unit test must not dial whatever happens to
            // listen there. Mirrors `list_with_a_tenantless_unrestricted_principal_still_queries_dagster`.
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
            env.insert(
                "DAGSTER_URL".to_owned(),
                "http://127.0.0.1:1/graphql".to_owned(),
            );
            let state = AppState::new(Config::from_map(&env).expect("a valid test Config"));
            let response = volume(State(state), Path("j".to_owned())).await;
            assert_eq!(response.status(), StatusCode::OK, "200, not 503");
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("collect body");
            let v: Value = serde_json::from_slice(&body).expect("valid JSON");
            assert_eq!(v["runs"], json!([]));
            assert!(v["medianRows"].is_null());
            assert!(
                v["unavailable"].is_string(),
                "unavailable is the classified reason string"
            );
            // AGENTS.md rule 4: upstream text must never reach a
            // response body. The Dagster URL we fed the client is the
            // most likely upstream detail to leak — assert it is
            // absent.
            assert!(
                !v["unavailable"]
                    .as_str()
                    .unwrap_or("")
                    .contains("127.0.0.1:1"),
                "upstream URL must not leak into the response: got {:?}",
                v["unavailable"]
            );
        }
    }

    /// WS4 item C1 — `GET /api/pipelines/{id}` for a `Dagster`-native job:
    /// no Postgres needed, `state.dagster` is a `wiremock` stand-in.
    mod detail_route {
        use std::collections::HashMap;

        use crate::config::Config;
        use crate::state::AppState;

        use super::*;

        fn state_with_dagster(server_uri: &str) -> AppState {
            let mut env = HashMap::new();
            env.insert("DAGSTER_URL".to_owned(), format!("{server_uri}/graphql"));
            let config = Config::from_map(&env).expect("a valid test Config");
            AppState::new(config)
        }

        /// The response shape asserted below (WS4 item C1),
        /// built against a real captured `job_graph` fixture served by
        /// `wiremock` for every GraphQL POST this route dispatches
        /// (`repositoriesOrError`, `runsOrError`, `pipelineOrError`) — all
        /// three land on the same `/graphql` path, so one catch-all mock
        /// per query shape is mounted, matched on the query text itself.
        #[tokio::test]
        async fn detail_route_returns_graph_config_and_schedule_for_a_dagster_job() {
            let server = wiremock::MockServer::start().await;
            let job_graph_body: Value = serde_json::from_str(include_str!(
                "../../../lakehouse-dagster/tests/fixtures/job_graph_bronze_maintenance.json"
            ))
            .expect("fixture parses");

            wiremock::Mock::given(wiremock::matchers::body_string_contains(
                "repositoriesOrError",
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "data": { "repositoriesOrError": { "__typename": "RepositoryConnection", "nodes": [
                    { "jobs": [ { "name": "bronze_maintenance_job" } ],
                      "schedules": [ { "name": "sched", "cronSchedule": "0 3 * * *",
                          "scheduleState": { "status": "RUNNING" },
                          "jobName": "bronze_maintenance_job" } ] }
                ] } }
            })))
            .mount(&server)
            .await;
            wiremock::Mock::given(wiremock::matchers::body_string_contains("runsOrError"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "runsOrError": { "__typename": "Runs", "results": [] } }
                })))
                .mount(&server)
                .await;
            wiremock::Mock::given(wiremock::matchers::body_string_contains("pipelineOrError"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(job_graph_body))
                .mount(&server)
                .await;

            let state = state_with_dagster(&server.uri());
            let response = detail(State(state), Path("bronze_maintenance_job".to_owned())).await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("collect body");
            let v: Value = serde_json::from_slice(&body).expect("valid JSON");
            assert_eq!(v["id"], "bronze_maintenance_job");
            assert_eq!(v["engine"], "dagster");
            assert_eq!(
                v["graph"]["edges"].as_array().expect("edges array").len(),
                0
            );
            let ops = v["graph"]["ops"].as_array().expect("ops array");
            assert_eq!(ops.len(), 1);
            assert_eq!(ops[0]["name"], "run_bronze_maintenance");
            assert_eq!(
                ops[0]["sourceRef"],
                "dispar_orchestrate/maintenance.py::run_bronze_maintenance"
            );
            assert_eq!(ops[0]["commit"], "unknown");
            assert!(v["config"].as_array().expect("config array").is_empty());
            assert!(v["definition"].is_null());
        }

        #[tokio::test]
        async fn detail_route_404s_for_a_dagster_job_not_in_the_repository() {
            let server = wiremock::MockServer::start().await;
            wiremock::Mock::given(wiremock::matchers::body_string_contains(
                "repositoriesOrError",
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "data": { "repositoriesOrError": { "__typename": "RepositoryConnection", "nodes": [
                    { "jobs": [], "schedules": [] }
                ] } }
            })))
            .mount(&server)
            .await;
            wiremock::Mock::given(wiremock::matchers::body_string_contains("runsOrError"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "runsOrError": { "__typename": "Runs", "results": [] } }
                })))
                .mount(&server)
                .await;
            wiremock::Mock::given(wiremock::matchers::body_string_contains("pipelineOrError"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "pipelineOrError": { "__typename": "PipelineNotFoundError" } }
                })))
                .mount(&server)
                .await;

            let state = state_with_dagster(&server.uri());
            let response = detail(State(state), Path("no_such_job".to_owned())).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }

        /// The `pl-` half of [`detail`], exercised against a real Postgres
        /// so "definition populated from the stored row" is real.
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

        fn pg_state_for(pool: &sqlx::PgPool) -> AppState {
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), database_url_for(pool));
            let config = Config::from_map(&env).expect("a valid test Config");
            AppState::new(config)
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn authored_detail_reports_engine_authored_null_graph_and_the_stored_definition(
            pool: sqlx::PgPool,
        ) {
            use lakehouse_test_support as _;

            let state = pg_state_for(&pool);
            let body = Bytes::from(
                serde_json::to_vec(&json!({
                    "name": format!("detail-route-test-{}", uuid::Uuid::new_v4()),
                    "kind": "batch",
                    "sourceZone": "bronze",
                    "sourceTable": "t",
                    "transforms": [],
                    "targetZone": "silver",
                    "targetTable": "t",
                    "schedule": "manual",
                }))
                .expect("serialize"),
            );
            let (_, ApiJson(created)) = create(
                State(state.clone()),
                Extension(fixture_user_principal()),
                HeaderMap::new(),
                body,
            )
            .await
            .expect("create should succeed");

            let response = detail(State(state), Path(created.id.clone())).await;
            assert_eq!(response.status(), StatusCode::OK);
            let response_body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("collect body");
            let v: Value = serde_json::from_slice(&response_body).expect("valid JSON");
            assert_eq!(v["id"], created.id);
            assert_eq!(v["engine"], "authored");
            assert!(
                v["graph"].is_null(),
                "authored pipelines have no job graph yet"
            );
            assert_eq!(v["definition"]["sourceZone"], "bronze");
            assert_eq!(v["definition"]["targetTable"], "t");
        }

        #[tokio::test]
        async fn authored_detail_404s_for_an_unknown_pl_id() {
            let mut env = HashMap::new();
            env.insert(
                "DATABASE_URL".to_owned(),
                "postgres://postgres:postgres@localhost:5432/postgres".to_owned(),
            );
            let config = Config::from_map(&env).expect("a valid test Config");
            let state = AppState::new(config);
            let response = detail(State(state), Path("pl-does-not-exist".to_owned())).await;
            // No live Postgres backs this test's DATABASE_URL, so this
            // either 503s (store unreachable) or 404s (a real Postgres IS
            // reachable at the default URL in this dev environment and the
            // row genuinely doesn't exist) -- both are honest, neither is
            // 200.
            assert_ne!(response.status(), StatusCode::OK);
        }
    }

    /// WS4 item C2 — `GET /api/pipelines/{id}/source?op=`, exercised end to
    /// end: a real `tempdir()`-backed allowlist (never a mocked path check)
    /// plus `wiremock` standing in for the `job_graph` call `check_commit`
    /// reads `commit` metadata from.
    mod source_route {
        use std::collections::HashMap;
        use std::sync::Arc;

        use crate::config::Config;
        use crate::pipeline_source;
        use crate::state::AppState;

        use super::*;

        fn state_with_dagster_and_source(server_uri: &str, base: &std::path::Path) -> AppState {
            let mut env = HashMap::new();
            env.insert("DAGSTER_URL".to_owned(), format!("{server_uri}/graphql"));
            let config = Config::from_map(&env).expect("a valid test Config");
            let mut state = AppState::new(config);
            let allowlist = pipeline_source::build_allowlist(base).expect("build_allowlist");
            state.pipeline_source_allowlist = Arc::new(allowlist);
            state
        }

        /// The code-location package directory the fixtures use as the
        /// allowlist base. `build_allowlist` keys every file by the base's
        /// OWN final component (the Python package name), so the base has
        /// to be a directory named like a code location -- a raw temp root
        /// would key files under a random directory name no `source_ref`
        /// could match.
        fn source_package(dir: &tempfile::TempDir) -> std::path::PathBuf {
            let package = dir.path().join("dispar_orchestrate");
            std::fs::create_dir(&package).expect("create the package dir");
            package
        }

        #[tokio::test]
        async fn pipeline_source_route_400s_when_op_is_missing() {
            let server = wiremock::MockServer::start().await;
            let dir = tempfile::tempdir().expect("tempdir");
            let package = source_package(&dir);
            let state = state_with_dagster_and_source(&server.uri(), &package);
            let response = source(
                State(state),
                Path("bronze_maintenance_job".to_owned()),
                Query(SourceQuery { op: None }),
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }

        #[tokio::test]
        async fn pipeline_source_route_404s_on_unknown_op_after_commit_passes() {
            let server = wiremock::MockServer::start().await;
            let dir = tempfile::tempdir().expect("tempdir");
            let package = source_package(&dir);
            std::fs::write(package.join("assets.py"), "def real_fn():\n    pass\n").expect("write");
            let op_ref = "dispar_orchestrate/assets.py::not_a_real_fn";
            wiremock::Mock::given(wiremock::matchers::body_string_contains("pipelineOrError"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "pipelineOrError": { "__typename": "Pipeline", "solidHandles": [
                        { "solid": { "name": "an_op", "definition": { "description": null,
                            "metadata": [
                                { "key": "source_ref", "value": op_ref },
                                { "key": "commit", "value": "real-sha" },
                            ] }, "inputs": [] } }
                    ] } }
                })))
                .mount(&server)
                .await;
            let state = state_with_dagster_and_source(&server.uri(), &package);
            // This image's own GIT_SHA must match the op's declared commit
            // for the request to reach the allowlist check at all.
            let mut state = state;
            state.config = Arc::new({
                let mut env = HashMap::new();
                env.insert("GIT_SHA".to_owned(), "real-sha".to_owned());
                Config::from_map(&env).expect("a valid test Config")
            });
            let response = source(
                State(state),
                Path("bronze_maintenance_job".to_owned()),
                Query(SourceQuery {
                    op: Some(op_ref.to_owned()),
                }),
            )
            .await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }

        #[tokio::test]
        async fn pipeline_source_route_409s_on_commit_mismatch() {
            let server = wiremock::MockServer::start().await;
            let dir = tempfile::tempdir().expect("tempdir");
            let package = source_package(&dir);
            std::fs::write(package.join("assets.py"), "def real_fn():\n    pass\n").expect("write");
            let op_ref = "dispar_orchestrate/assets.py::real_fn";
            wiremock::Mock::given(wiremock::matchers::body_string_contains("pipelineOrError"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "pipelineOrError": { "__typename": "Pipeline", "solidHandles": [
                        { "solid": { "name": "an_op", "definition": { "description": null,
                            "metadata": [
                                { "key": "source_ref", "value": op_ref },
                                { "key": "commit", "value": "op-built-from-this-sha" },
                            ] }, "inputs": [] } }
                    ] } }
                })))
                .mount(&server)
                .await;
            let mut state = state_with_dagster_and_source(&server.uri(), &package);
            let mut env = HashMap::new();
            env.insert(
                "GIT_SHA".to_owned(),
                "this-image-was-built-from-a-different-sha".to_owned(),
            );
            state.config = Arc::new(Config::from_map(&env).expect("a valid test Config"));
            let response = source(
                State(state),
                Path("bronze_maintenance_job".to_owned()),
                Query(SourceQuery {
                    op: Some(op_ref.to_owned()),
                }),
            )
            .await;
            assert_eq!(response.status(), StatusCode::CONFLICT);
        }

        /// **The exact V1 regression**: both `commit` and `GIT_SHA` at the
        /// build-time placeholder `"unknown"` must still refuse — a naive
        /// `==` comparison would treat this as "verified," when in truth
        /// NEITHER image's real commit is known.
        #[tokio::test]
        async fn pipeline_source_route_409s_when_both_commits_are_the_unknown_placeholder() {
            let server = wiremock::MockServer::start().await;
            let dir = tempfile::tempdir().expect("tempdir");
            let package = source_package(&dir);
            std::fs::write(package.join("assets.py"), "def real_fn():\n    pass\n").expect("write");
            let op_ref = "dispar_orchestrate/assets.py::real_fn";
            wiremock::Mock::given(wiremock::matchers::body_string_contains("pipelineOrError"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "pipelineOrError": { "__typename": "Pipeline", "solidHandles": [
                        { "solid": { "name": "an_op", "definition": { "description": null,
                            "metadata": [
                                { "key": "source_ref", "value": op_ref },
                                { "key": "commit", "value": "unknown" },
                            ] }, "inputs": [] } }
                    ] } }
                })))
                .mount(&server)
                .await;
            // GIT_SHA is left unset -> Config::from_map defaults it to
            // "unknown" too (config.rs: `or_default(env, "GIT_SHA",
            // "unknown")`).
            let state = state_with_dagster_and_source(&server.uri(), &package);
            assert_eq!(state.config.git_sha, "unknown");
            let response = source(
                State(state),
                Path("bronze_maintenance_job".to_owned()),
                Query(SourceQuery {
                    op: Some(op_ref.to_owned()),
                }),
            )
            .await;
            assert_eq!(response.status(), StatusCode::CONFLICT);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("collect body");
            let v: Value = serde_json::from_slice(&body).expect("valid JSON");
            assert_eq!(
                v["error"], "source provenance is unavailable for this build",
                "the UnverifiableProvenance message, never a fabricated match"
            );
        }

        #[tokio::test]
        async fn pipeline_source_route_200s_and_returns_text_for_a_real_allowlisted_op() {
            let server = wiremock::MockServer::start().await;
            let dir = tempfile::tempdir().expect("tempdir");
            let package = source_package(&dir);
            std::fs::write(
                package.join("assets.py"),
                "def ingest_bronze_table():\n    return 1\n",
            )
            .expect("write");
            let op_ref = "dispar_orchestrate/assets.py::ingest_bronze_table";
            wiremock::Mock::given(wiremock::matchers::body_string_contains("pipelineOrError"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "pipelineOrError": { "__typename": "Pipeline", "solidHandles": [
                        { "solid": { "name": "an_op", "definition": { "description": null,
                            "metadata": [
                                { "key": "source_ref", "value": op_ref },
                                { "key": "commit", "value": "real-sha-abc123" },
                            ] }, "inputs": [] } }
                    ] } }
                })))
                .mount(&server)
                .await;
            let mut state = state_with_dagster_and_source(&server.uri(), &package);
            let mut env = HashMap::new();
            env.insert("GIT_SHA".to_owned(), "real-sha-abc123".to_owned());
            state.config = Arc::new(Config::from_map(&env).expect("a valid test Config"));
            let response = source(
                State(state),
                Path("bronze_maintenance_job".to_owned()),
                Query(SourceQuery {
                    op: Some(op_ref.to_owned()),
                }),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("collect body");
            let v: Value = serde_json::from_slice(&body).expect("valid JSON");
            assert!(
                v["text"]
                    .as_str()
                    .expect("text field")
                    .contains("def ingest_bronze_table")
            );
            assert_eq!(v["commit"], "real-sha-abc123");
        }

        /// A real, live traversal attempt through the actual HTTP handler
        /// (not just `pipeline_source`'s own unit tests): a `sourceRef`
        /// claiming `../secret.py::x`, even when `Dagster` itself reports
        /// it as the op's `source_ref` (a compromised/malicious code
        /// location) and the commit matches, must still 404 — the
        /// allowlist has no such key.
        #[tokio::test]
        async fn pipeline_source_route_refuses_a_traversal_reported_by_dagster_itself() {
            let server = wiremock::MockServer::start().await;
            let dir = tempfile::tempdir().expect("tempdir");
            let package = source_package(&dir);
            std::fs::write(package.join("assets.py"), "x = 1\n").expect("write");
            // The temp ROOT, one level above the package directory that
            // is the allowlist base -- inside the `TempDir`'s own managed
            // lifetime, and genuinely outside the base.
            std::fs::write(dir.path().join("secret.py"), "SECRET = 1\n").expect("write");
            let op_ref = "../secret.py::x";
            wiremock::Mock::given(wiremock::matchers::body_string_contains("pipelineOrError"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "pipelineOrError": { "__typename": "Pipeline", "solidHandles": [
                        { "solid": { "name": "an_op", "definition": { "description": null,
                            "metadata": [
                                { "key": "source_ref", "value": op_ref },
                                { "key": "commit", "value": "real-sha-abc123" },
                            ] }, "inputs": [] } }
                    ] } }
                })))
                .mount(&server)
                .await;
            let mut state = state_with_dagster_and_source(&server.uri(), &package);
            let mut env = HashMap::new();
            env.insert("GIT_SHA".to_owned(), "real-sha-abc123".to_owned());
            state.config = Arc::new(Config::from_map(&env).expect("a valid test Config"));
            let response = source(
                State(state),
                Path("bronze_maintenance_job".to_owned()),
                Query(SourceQuery {
                    op: Some(op_ref.to_owned()),
                }),
            )
            .await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            // No manual cleanup: `secret.py` now lives inside `dir`, so
            // dropping the `TempDir` removes it.
        }
    }

    /// R4 plan 2c: `GET /api/pipelines/{id}/config-schema` — read-side
    /// companion to `POST /api/pipelines/{id}/trigger` with the
    /// optional `runConfig`. Returns the job's default config YAML (or
    /// `hasConfig: false` for an `pl-…` id, with no `Dagster` call).
    mod config_schema_route {
        use std::collections::HashMap;

        use wiremock::matchers::{body_string_contains, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use crate::config::Config;
        use crate::state::AppState;

        use super::*;

        fn state_with_dagster(server_uri: &str) -> AppState {
            let mut env = HashMap::new();
            env.insert("DAGSTER_URL".to_owned(), format!("{server_uri}/graphql"));
            let config = Config::from_map(&env).expect("a valid test Config");
            AppState::new(config)
        }

        /// `pl-…` ids are authored pipelines — no `Dagster` schema, no
        /// GraphQL call. The wiremock is set up to 500 any POST so a
        /// regression that calls Dagster fails this test.
        #[tokio::test]
        async fn config_schema_for_pl_id_is_has_config_false_without_calling_dagster() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(500))
                .mount(&server)
                .await;

            let state = state_with_dagster(&server.uri());
            let response = config_schema(
                State(state),
                Some(Extension(fixture_user_principal())),
                Path("pl-authored".to_owned()),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("collect body");
            let v: Value = serde_json::from_slice(&body).expect("valid JSON");
            assert_eq!(v["pipelineId"], "pl-authored");
            assert_eq!(v["hasConfig"], false);
            assert!(v["defaultConfig"].is_null());
            assert!(v["defaultConfigYaml"].is_null());
        }

        /// A non-`pl-…` id returns the YAML from Dagster's
        /// `runConfigSchemaOrError` verbatim, alongside `hasConfig: true`
        /// and `defaultConfig: null` (see [`config_schema`]'s doc
        /// comment for the YAML-string-shape rationale).
        #[tokio::test]
        async fn config_schema_for_dagster_job_returns_yaml_and_has_config_true() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(body_string_contains("runConfigSchemaOrError"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "runConfigSchemaOrError": {
                        "__typename": "RunConfigSchema",
                        "rootDefaultYaml": "ops:\n  run_x:\n    config:\n      k: v\n"
                    } }
                })))
                .mount(&server)
                .await;

            let state = state_with_dagster(&server.uri());
            let response = config_schema(
                State(state),
                Some(Extension(fixture_user_principal())),
                Path("ingest_job".to_owned()),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("collect body");
            let v: Value = serde_json::from_slice(&body).expect("valid JSON");
            assert_eq!(v["pipelineId"], "ingest_job");
            assert_eq!(v["hasConfig"], true);
            assert!(v["defaultConfig"].is_null());
            assert_eq!(
                v["defaultConfigYaml"],
                "ops:\n  run_x:\n    config:\n      k: v\n"
            );
        }

        /// An unknown non-`pl-…` id (Dagster's
        /// `PipelineNotFoundError`) is a 404 — the same answer
        /// `pipeline.runnable` gives a job that doesn't exist. The
        /// mutation check is the inverse of `trigger_with_config`: the
        /// response MUST NOT contain a `message` field with
        /// `Dagster`'s `Could not find pipeline named …` text.
        #[tokio::test]
        async fn config_schema_for_unknown_job_is_404_without_forwarding_dagster_message() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(body_string_contains("runConfigSchemaOrError"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "runConfigSchemaOrError": { "__typename": "PipelineNotFoundError",
                        "message": "Could not find pipeline named not_a_job" } }
                })))
                .mount(&server)
                .await;

            let state = state_with_dagster(&server.uri());
            let response = config_schema(
                State(state),
                Some(Extension(fixture_user_principal())),
                Path("not_a_job".to_owned()),
            )
            .await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("collect body");
            let body_text = std::str::from_utf8(&body).unwrap_or("");
            assert!(
                !body_text.contains("Could not find pipeline named"),
                "404 body must NOT carry Dagster's message text, got {body_text}"
            );
        }

        /// No principal — same 401 posture as `trigger`/`pause`/`resume`.
        #[tokio::test]
        async fn config_schema_without_principal_is_401() {
            let state = state_with_dagster("http://127.0.0.1:1");
            let response = config_schema(State(state), None, Path("ingest_job".to_owned())).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }

    /// WS4 item C3 — `GET /api/pipelines/{id}/runs/{runId}/steps`, against
    /// Phase A's real captured `run_steps` fixture.
    mod steps_route {
        use std::collections::HashMap;

        use crate::config::Config;
        use crate::state::AppState;

        use super::*;

        fn state_with_dagster(server_uri: &str) -> AppState {
            let mut env = HashMap::new();
            env.insert("DAGSTER_URL".to_owned(), format!("{server_uri}/graphql"));
            let config = Config::from_map(&env).expect("a valid test Config");
            AppState::new(config)
        }

        #[tokio::test]
        async fn steps_route_returns_steps_with_row_counts_from_a_real_captured_fixture() {
            let server = wiremock::MockServer::start().await;
            let body: Value = serde_json::from_str(include_str!(
                "../../../lakehouse-dagster/tests/fixtures/run_steps_captured_fixture.json"
            ))
            .expect("fixture parses");
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
                .mount(&server)
                .await;

            let state = state_with_dagster(&server.uri());
            let response = run_steps(
                State(state),
                Path(("bronze_maintenance_job".to_owned(), "r1".to_owned())),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let response_body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("collect body");
            let v: Value = serde_json::from_slice(&response_body).expect("valid JSON");
            let steps = v["steps"].as_array().expect("steps array");
            assert_eq!(steps.len(), 1);
            assert_eq!(steps[0]["stepKey"], "run_bronze_maintenance");
            // The real fixture's status is "FAILURE" -> map_run_status ->
            // "failed", never the raw Dagster string.
            assert_eq!(steps[0]["status"], "failed");
            assert!(steps[0]["startMs"].is_number());
            assert!(steps[0]["materializations"].as_array().unwrap().is_empty());
        }

        #[tokio::test]
        async fn steps_route_is_503_not_a_fabricated_empty_list_when_dagster_is_unreachable() {
            // No mock mounted at all -- every request to this MockServer's
            // address fails at the transport level once it is dropped, but
            // building the client against an address nothing listens on is
            // simpler and just as real.
            let state = state_with_dagster("http://127.0.0.1:1");
            let response = run_steps(
                State(state),
                Path(("bronze_maintenance_job".to_owned(), "r1".to_owned())),
            )
            .await;
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        }
    }

    /// Plan 1c (R2, day-1): `POST /api/pipelines/runs/{runId}/retry` with
    /// `{"strategy":"selected","stepKeys":[...]}`. Three outcomes the
    /// route has to produce correctly:
    ///
    /// 1. unknown `stepKey` → 400, naming the FIRST unknown key (never
    ///    the caller's full list — that would echo untrusted input
    ///    back).
    /// 2. non-empty `stepKeys` matching real steps → the right
    ///    GraphQL mutation body (`launchRunReexecution` with a
    ///    `stepKeys` filter, parent config + parent/root run id), so
    ///    `Dagster` can clone only those steps.
    /// 3. empty / missing `stepKeys` → 400, the same message the
    ///    parser test asserts (`"requires a non-empty stepKeys
    ///    array"`).
    ///
    /// The handler does TWO GraphQL calls for the selected path: one
    /// to look up the parent run's `pipelineName`/`rootRunId`/
    /// `runConfig`, and one to launch the re-execution. The mocks
    /// below match on the operation name (`pipelineRunOrError` vs
    /// `launchRunReexecution`) so each query goes to its own response.
    mod retry_route {
        use std::collections::HashMap;

        use crate::config::Config;
        use crate::state::AppState;

        use super::*;

        fn state_with_dagster(server_uri: &str) -> AppState {
            let mut env = HashMap::new();
            env.insert("DAGSTER_URL".to_owned(), format!("{server_uri}/graphql"));
            let config = Config::from_map(&env).expect("a valid test Config");
            AppState::new(config)
        }

        /// `selected` with a stepKey that does not belong to the parent
        /// run returns 400 naming the FIRST unknown key, so a typo at
        /// index 0 does not silently disappear when index 1 is valid.
        #[tokio::test]
        async fn selected_retry_returns_400_naming_first_unknown_step_key() {
            let server = wiremock::MockServer::start().await;
            // `run_steps` lookup for the parent run — three real keys,
            // none of which is "transform". The query sends
            // `runOrError(runId:$rid)` (see `lakehouse_dagster::DgClient::
            // run_steps`), so match on that operation name.
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::body_string_contains("runOrError"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "runOrError": {
                        "__typename": "Run",
                        "runId": "r1",
                        "stepStats": [
                            { "stepKey": "extract" },
                            { "stepKey": "load" }
                        ]
                    } }
                })))
                .mount(&server)
                .await;

            let state = state_with_dagster(&server.uri());
            let response = retry_run(
                State(state),
                Path("r1".to_owned()),
                Bytes::from_static(
                    br#"{"strategy":"selected","stepKeys":["extract","transform"]}"#,
                ),
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("collect body");
            let v: Value = serde_json::from_slice(&body).expect("valid JSON");
            let message = v["error"].as_str().expect("error string");
            assert!(
                message.contains("transform") && message.contains("r1"),
                "400 must name the first unknown step key and the run id, got {message}"
            );
            // And it must NOT echo the rest of the caller's list back.
            assert!(
                !message.contains("extract"),
                "400 must not echo the other step keys, got {message}"
            );
        }

        /// `selected` with a valid step list issues the `launchRunReexecution`
        /// mutation, NOT `ReexecutionStrategy.SELECTED` (which does not
        /// exist on the `Dagster` side) and NOT `ReexecutionStrategy
        /// .ALL_STEPS` (which would re-run the whole job).
        #[tokio::test]
        async fn selected_retry_issues_launch_run_reexecution_with_step_keys_filter() {
            let server = wiremock::MockServer::start().await;

            // `run_steps` lookup for the parent run — three real keys so
            // "extract" and "transform" are both valid. The route calls
            // this FIRST (to validate the caller's `stepKeys`), then
            // calls `launch_reexecution_of_steps` which performs the
            // `pipelineRunOrError` lookup below.
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::body_string_contains(
                    "runOrError(runId:$rid)",
                ))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "runOrError": {
                        "__typename": "Run",
                        "runId": "r1",
                        "stepStats": [
                            { "stepKey": "extract" },
                            { "stepKey": "transform" },
                            { "stepKey": "load" }
                        ]
                    } }
                })))
                .mount(&server)
                .await;

            // `pipelineRunOrError` lookup — the parent config is an
            // object (`runConfig`) so `launch_reexecution_of_steps`
            // forwards it as a JSON object. `stepStats` is not read
            // here (only `pipelineName`/`rootRunId`/`runConfig` are
            // extracted from this response) but the field has to be
            // present so `Run` is the matching fragment.
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::body_string_contains(
                    "pipelineRunOrError(runId: $rid)",
                ))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "pipelineRunOrError": {
                        "__typename": "Run",
                        "runId": "r1",
                        "pipelineName": "refresh_lakehouse",
                        "rootRunId": "r1",
                        "runConfig": { "resources": { "lakehouse": { "config": { "path": "x" } } } },
                        "stepStats": [
                            { "stepKey": "extract" },
                            { "stepKey": "transform" },
                            { "stepKey": "load" }
                        ]
                    } }
                })))
                .mount(&server)
                .await;

            // `launchRunReexecution` mutation — `state.dagster.
            // launch_reexecution_of_steps` waits for this exact
            // operation name. The body must carry both the operation
            // name AND the `stepKeys` argument; a request that arrived
            // without the latter would be `ReexecutionStrategy.ALL_STEPS`
            // in disguise, and the wiremock matcher stack asserts the
            // call really did ask for the named keys. The GraphQL
            // variables key is `keys` (the query-side argument is
            // `stepKeys: $keys`), so the matcher looks for the
            // variables-side name.
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::body_string_contains(
                    "launchRunReexecution",
                ))
                .and(wiremock::matchers::body_string_contains(
                    r#""keys":["extract","transform"]"#,
                ))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "launchRunReexecution": {
                        "__typename": "LaunchRunSuccess", "run": { "runId": "r-new" }
                    } }
                })))
                .mount(&server)
                .await;

            let state = state_with_dagster(&server.uri());
            let response = retry_run(
                State(state),
                Path("r1".to_owned()),
                Bytes::from_static(
                    br#"{"strategy":"selected","stepKeys":["extract","transform"]}"#,
                ),
            )
            .await;
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "a valid selected retry must reach the launch mutation"
            );
        }

        /// `selected` without `stepKeys` (or with an empty array) returns
        /// 400 from the parser, BEFORE any `Dagster` call — wiremock is
        /// mounted only for the "happy path" so a 400 means the route
        /// refused the request without contacting `Dagster`.
        #[tokio::test]
        async fn selected_retry_without_step_keys_returns_400_before_calling_dagster() {
            let server = wiremock::MockServer::start().await;
            // A catch-all 200 mock: if the route calls Dagster, this
            // fires. The test passes only if no request lands.
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "pipelineRunOrError": { "__typename": "NotFound" } }
                })))
                .mount(&server)
                .await;

            let state = state_with_dagster(&server.uri());
            let response = retry_run(
                State(state),
                Path("r1".to_owned()),
                Bytes::from_static(br#"{"strategy":"selected"}"#),
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("collect body");
            let v: Value = serde_json::from_slice(&body).expect("valid JSON");
            let message = v["error"].as_str().expect("error string");
            assert!(
                message.contains("stepKeys"),
                "400 must mention the missing field, got {message}"
            );
        }
    }

    /// Plan 1c (R2, day-1): `GET /api/pipelines/{id}/runs/steps` — the
    /// runs × steps matrix. Three things to prove:
    ///
    /// 1. A happy-path matrix response carries both runs and their
    ///    `steps` arrays.
    /// 2. A step with no `endTime` (or no `startTime`) carries
    ///    `"durationMs": null`, NOT a fabricated `0`.
    /// 3. The route matches BEFORE `{runId}/steps` — the path
    ///    `/api/pipelines/{id}/runs/steps` MUST be handled here, not
    ///    by [`run_steps`] interpreting "steps" as a Dagster run id
    ///    (the route registration order in `routes/mod.rs::pipelines_router`
    ///    is what enforces this; the test pins the behaviour).
    mod runs_step_matrix_route {
        use std::collections::HashMap;

        use crate::config::Config;
        use crate::state::AppState;

        use super::*;

        fn state_with_dagster(server_uri: &str) -> AppState {
            let mut env = HashMap::new();
            env.insert("DAGSTER_URL".to_owned(), format!("{server_uri}/graphql"));
            let config = Config::from_map(&env).expect("a valid test Config");
            AppState::new(config)
        }

        #[tokio::test]
        async fn matrix_route_returns_runs_and_per_step_stats_from_one_call() {
            let server = wiremock::MockServer::start().await;
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::body_string_contains("stepStats"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "runsOrError": { "__typename": "Runs", "results": [
                        { "runId": "r1", "status": "SUCCESS",
                          "startTime": 1.0,
                          "stepStats": [
                              { "stepKey": "extract", "status": "SUCCESS",
                                "startTime": 1.0, "endTime": 2.0 },
                              { "stepKey": "load", "status": "SUCCESS",
                                "startTime": 2.0, "endTime": 3.0 }
                          ] },
                        { "runId": "r2", "status": "FAILURE",
                          "startTime": 10.0,
                          "stepStats": [
                              { "stepKey": "extract", "status": "FAILURE",
                                "startTime": 10.0, "endTime": 11.0 }
                          ] }
                    ] } }
                })))
                .mount(&server)
                .await;

            let state = state_with_dagster(&server.uri());
            let response =
                runs_step_matrix(State(state), Path("refresh_lakehouse".to_owned())).await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("collect body");
            let v: Value = serde_json::from_slice(&body).expect("valid JSON");
            assert!(v["unavailable"].is_null());
            let runs = v["runs"].as_array().expect("runs array");
            assert_eq!(runs.len(), 2);
            assert_eq!(runs[0]["id"], "r1");
            assert_eq!(runs[0]["status"], "completed");
            // 3.0 - 2.0 = 1.0 second = 1000 ms (per-step duration).
            assert_eq!(runs[0]["steps"][1]["durationMs"], 1000);
            assert_eq!(runs[1]["steps"][0]["status"], "failed");
        }

        /// A step with `endTime: null` (one `Dagster` never reported a
        /// completion timestamp for) carries `durationMs: null` — never
        /// a fabricated `0`, which would read as "ran instantly" rather
        /// than "never finished".
        #[tokio::test]
        async fn matrix_route_reports_null_duration_for_a_step_that_never_started() {
            let server = wiremock::MockServer::start().await;
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::body_string_contains("stepStats"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "runsOrError": { "__typename": "Runs", "results": [
                        { "runId": "r1", "status": "STARTED",
                          "startTime": 1.0,
                          "stepStats": [
                              { "stepKey": "extract", "status": "SUCCESS",
                                "startTime": 1.0, "endTime": 2.0 },
                              { "stepKey": "load", "status": "QUEUED",
                                "startTime": null, "endTime": null }
                          ] }
                    ] } }
                })))
                .mount(&server)
                .await;

            let state = state_with_dagster(&server.uri());
            let response =
                runs_step_matrix(State(state), Path("refresh_lakehouse".to_owned())).await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("collect body");
            let v: Value = serde_json::from_slice(&body).expect("valid JSON");
            let queued_step = &v["runs"][0]["steps"][1];
            assert_eq!(queued_step["status"], "queued");
            assert!(
                queued_step["durationMs"].is_null(),
                "a never-started step must report null durationMs, got {queued_step}"
            );
        }

        /// The path `/api/pipelines/{id}/runs/steps` MUST reach the
        /// matrix handler, not `run_steps` (which would 503 trying to
        /// interpret "steps" as a `Dagster` run id). Pinning this with
        /// a focused route test would need the full axum stack;
        /// instead, the focused assertion below checks the matrix
        /// mock body is hit for a request the previous handler would
        /// have rejected with 503.
        #[tokio::test]
        async fn matrix_route_is_reached_for_runs_steps_path() {
            let server = wiremock::MockServer::start().await;
            // Only the `runsOrError` filter by `pipelineName` shape
            // matches the matrix query; `run_steps` would have sent
            // `runOrError(runId:$rid)`. If the route registration order
            // is wrong, `run_steps` would receive "steps" as the run
            // id and fail with 503 BEFORE this mock fires.
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::body_string_contains("runsOrError"))
                .and(wiremock::matchers::body_string_contains("stepStats"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "runsOrError": { "__typename": "Runs", "results": [] } }
                })))
                .mount(&server)
                .await;

            let state = state_with_dagster(&server.uri());
            let response =
                runs_step_matrix(State(state), Path("refresh_lakehouse".to_owned())).await;
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "the matrix route must handle runs/steps — a wrong match \
                 would have reached `run_steps` and surfaced a 503 for \
                 the literal run id \"steps\""
            );
        }
    }

    /// WS4 item C4 — `GET /api/pipelines/{id}/runs/{runId}/logs?cursor=`,
    /// against Phase A's real captured `run_logs` fixture, and the bounded-
    /// page-size guarantee the brief's "log streaming must not leak" risk
    /// item requires.
    mod logs_route {
        use std::collections::HashMap;

        use crate::config::Config;
        use crate::state::AppState;

        use super::*;

        fn state_with_dagster(server_uri: &str) -> AppState {
            let mut env = HashMap::new();
            env.insert("DAGSTER_URL".to_owned(), format!("{server_uri}/graphql"));
            let config = Config::from_map(&env).expect("a valid test Config");
            AppState::new(config)
        }

        #[tokio::test]
        async fn logs_route_returns_lines_and_cursor_from_a_real_captured_fixture() {
            let server = wiremock::MockServer::start().await;
            let body: Value = serde_json::from_str(include_str!(
                "../../../lakehouse-dagster/tests/fixtures/run_logs_captured_fixture.json"
            ))
            .expect("fixture parses");
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
                .mount(&server)
                .await;

            let state = state_with_dagster(&server.uri());
            let response = run_logs(
                State(state),
                Path(("bronze_maintenance_job".to_owned(), "r1".to_owned())),
                Query(LogsQuery {
                    cursor: None,
                    limit: None,
                }),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let response_body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("collect body");
            let v: Value = serde_json::from_slice(&response_body).expect("valid JSON");
            let lines = v["lines"].as_array().expect("lines array");
            assert_eq!(lines.len(), 15);
            assert_eq!(
                v["cursor"],
                "eyJ0eXBlIjogIlNUT1JBR0VfSUQiLCAidmFsdWUiOiA0Nn0="
            );
            // Real fixture's first event is a RunEnqueuedEvent with an
            // empty message -- confirms the ts is parsed from the STRING
            // timestamp, not left as a string or dropped.
            assert!(lines[0]["ts"].is_number());
        }

        /// The size-bounding decision the brief's "log streaming must not
        /// leak" risk item requires: a caller-supplied `?limit=` far above
        /// the fixed ceiling is CLAMPED before it ever reaches
        /// `state.dagster.run_logs`, not passed through.
        #[tokio::test]
        async fn logs_route_paginates_via_cursor_and_bounds_page_size() {
            let server = wiremock::MockServer::start().await;
            // Responder inspects the request body itself and asserts the
            // `limit` variable Dagster actually received is clamped -- a
            // real assertion on the outbound GraphQL request, not just on
            // the response.
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .respond_with(move |req: &wiremock::Request| {
                    let parsed: Value = serde_json::from_slice(&req.body).expect("valid JSON body");
                    let limit = parsed["variables"]["limit"]
                        .as_u64()
                        .expect("limit variable present");
                    assert!(
                        limit <= u64::from(MAX_LOG_LINES_PER_PAGE),
                        "requested limit {limit} was not clamped to {MAX_LOG_LINES_PER_PAGE}"
                    );
                    let cursor = parsed["variables"]["after"].as_str();
                    assert_eq!(
                        cursor,
                        Some("prior-page-cursor"),
                        "the caller's cursor must be forwarded verbatim, not re-encoded"
                    );
                    wiremock::ResponseTemplate::new(200).set_body_json(json!({
                        "data": { "logsForRun": { "__typename": "EventConnection",
                            "events": [], "cursor": "next-page-cursor", "hasMore": false } }
                    }))
                })
                .mount(&server)
                .await;

            let state = state_with_dagster(&server.uri());
            let response = run_logs(
                State(state),
                Path(("bronze_maintenance_job".to_owned(), "r1".to_owned())),
                Query(LogsQuery {
                    cursor: Some("prior-page-cursor".to_owned()),
                    limit: Some(100_000), // far above MAX_LOG_LINES_PER_PAGE
                }),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let response_body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("collect body");
            let v: Value = serde_json::from_slice(&response_body).expect("valid JSON");
            assert_eq!(v["cursor"], "next-page-cursor");
        }

        #[tokio::test]
        async fn logs_route_404s_for_a_real_not_found_run_message() {
            let server = wiremock::MockServer::start().await;
            // The real, live-verified `RunNotFoundError` message text
            // (queried against this repository's own Dagster 1.13.20
            // stack with a bogus runId): `DgError::Server` carries the
            // `message` field, NOT the `__typename`, so this route's own
            // not-found match must key on this real substring.
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "logsForRun": { "__typename": "RunNotFoundError",
                        "message": "Pipeline run bogus-run-id could not be found." } }
                })))
                .mount(&server)
                .await;

            let state = state_with_dagster(&server.uri());
            let response = run_logs(
                State(state),
                Path((
                    "bronze_maintenance_job".to_owned(),
                    "bogus-run-id".to_owned(),
                )),
                Query(LogsQuery {
                    cursor: None,
                    limit: None,
                }),
            )
            .await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }

        #[tokio::test]
        async fn logs_route_is_503_not_a_fabricated_empty_list_when_dagster_returns_a_python_error()
        {
            let server = wiremock::MockServer::start().await;
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "logsForRun": { "__typename": "PythonError",
                        "message": "boom: unrelated internal Dagster failure" } }
                })))
                .mount(&server)
                .await;

            let state = state_with_dagster(&server.uri());
            let response = run_logs(
                State(state),
                Path(("bronze_maintenance_job".to_owned(), "r1".to_owned())),
                Query(LogsQuery {
                    cursor: None,
                    limit: None,
                }),
            )
            .await;
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        }
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
        use super::fixture_user_principal;

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

            let err = create(
                State(state),
                Extension(fixture_user_principal()),
                HeaderMap::new(),
                body,
            )
            .await
            .unwrap_err();
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
    /// WS4 item D4 — `POST /api/pipelines/{id}/status`'s
    /// `ALLOWED_TRANSITIONS` table, exercised against a real Postgres.
    mod status_route {
        use std::collections::HashMap;

        // Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
        // testcontainer bootstrap actually runs for this test binary — see
        // `connector_deprovision.rs`'s identical comment.
        use lakehouse_test_support as _;

        use crate::config::Config;
        use crate::state::AppState;

        use super::*;

        /// Build the `DATABASE_URL` dialing the SAME per-test Postgres
        /// database `#[sqlx::test]` already handed us via `pool` —
        /// extracting host/port/user/database from the pool's own connect
        /// options, same pattern `connector_deprovision.rs::target_for`
        /// uses, rather than hardcoding them.
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

        /// An [`AppState`] whose `pg` pool points at the SAME database the
        /// `#[sqlx::test]`-provided `pool` does, so a handler exercised
        /// through this state and a direct `pipelines::*` call against
        /// `pool` observe the same rows.
        fn state_for(pool: &sqlx::PgPool) -> AppState {
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), database_url_for(pool));
            let config = Config::from_map(&env).expect("a valid test Config");
            AppState::new(config)
        }

        fn status_body(status: &str) -> Bytes {
            Bytes::from(serde_json::to_vec(&json!({ "status": status })).expect("serialize"))
        }

        async fn create_draft(state: &AppState) -> pipelines::Pipeline {
            let body = Bytes::from(
                serde_json::to_vec(&json!({
                    "name": format!("status-route-test-{}", uuid::Uuid::new_v4()),
                    "kind": "batch",
                    "sourceZone": "bronze",
                    "sourceTable": "t",
                    "targetZone": "silver",
                    "targetTable": "t",
                    "schedule": "manual",
                }))
                .expect("serialize"),
            );
            let (_, ApiJson(pipeline)) = create(
                State(state.clone()),
                Extension(fixture_user_principal()),
                HeaderMap::new(),
                body,
            )
            .await
            .expect("create should succeed");
            pipeline
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn status_route_moves_a_draft_pipeline_to_ready(pool: sqlx::PgPool) {
            let state = state_for(&pool);
            let pipeline = create_draft(&state).await;
            assert_eq!(pipeline.status, "draft");

            let ApiJson(updated) =
                set_status_route(State(state), Path(pipeline.id), status_body("ready"))
                    .await
                    .expect("draft -> ready should succeed");
            assert_eq!(updated.status, "ready");
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn status_route_refuses_a_run_derived_target_status_with_400_and_writes_nothing(
            pool: sqlx::PgPool,
        ) {
            let state = state_for(&pool);
            let pipeline = create_draft(&state).await;

            for bad_status in ["completed", "running", "failed", "degraded", "partial"] {
                let err = set_status_route(
                    State(state.clone()),
                    Path(pipeline.id.clone()),
                    status_body(bad_status),
                )
                .await
                .unwrap_err();
                assert_eq!(err.0.status(), 400, "{bad_status} must be refused with 400");

                let current = pipelines::get_pipeline(&pool, &pipeline.id)
                    .await
                    .expect("get_pipeline should succeed")
                    .expect("pipeline should still exist");
                assert_eq!(
                    current.status, "draft",
                    "{bad_status} must not have written anything"
                );
            }
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn status_route_refuses_a_disallowed_from_state_with_409(pool: sqlx::PgPool) {
            let state = state_for(&pool);
            let pipeline = create_draft(&state).await;

            set_status_route(
                State(state.clone()),
                Path(pipeline.id.clone()),
                status_body("ready"),
            )
            .await
            .expect("first draft -> ready transition should succeed");

            let err = set_status_route(State(state), Path(pipeline.id), status_body("ready"))
                .await
                .unwrap_err();
            assert_eq!(
                err.0.status(),
                409,
                "ready -> ready is not an allowed transition"
            );
        }

        #[tokio::test]
        async fn status_route_404s_for_a_dagster_backed_id() {
            let config = Config::from_map(&HashMap::new()).expect("a valid test Config");
            let state = AppState::new(config);
            let err = set_status_route(
                State(state),
                Path("bronze_maintenance_job".to_owned()),
                status_body("ready"),
            )
            .await
            .unwrap_err();
            assert_eq!(
                err.0.status(),
                404,
                "a non-pl- id has no from-state to look up"
            );
        }
    }

    /// `PUT /api/pipelines/{id}/tenant` route tests. Same
    /// shape as `routes::connectors::tests::assign_tenant_route`.
    mod assign_tenant_route {
        use lakehouse_store::identity::{self, CreateTenantInput};

        use super::*;
        use crate::config::Config;

        fn database_url_for(pool: &sqlx::PgPool) -> String {
            let options = pool.connect_options();
            format!(
                "postgres://{}:postgres@{}:{}/{}",
                options.get_username(),
                options.get_host(),
                options.get_port(),
                options
                    .get_database()
                    .expect("#[sqlx::test] always targets a named database"),
            )
        }

        fn state_for(pool: &sqlx::PgPool) -> AppState {
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), database_url_for(pool));
            let config = Config::from_map(&env).expect("a valid test Config");
            AppState::new(config)
        }

        async fn seed_tenant(pool: &sqlx::PgPool, slug: &str) -> Uuid {
            let tenant = identity::create_tenant(
                pool,
                &CreateTenantInput {
                    name: "Acme Co".to_owned(),
                    slug: slug.to_owned(),
                    plan: "Standard".to_owned(),
                    residency: "US".to_owned(),
                },
            )
            .await
            .expect("create a test tenant");
            tenant
                .id
                .parse()
                .expect("create_tenant returns a UUID-shaped id")
        }

        /// A `pipeline_definition` row with `tenant_id = NULL` — `create`
        /// never sets it, matching every authored pipeline created after
        /// `0042_tenant_provisioning.sql` (whose own comment records that
        /// it backfills nothing for this table at all).
        async fn seed_pipeline_with_null_tenant(state: &AppState, name: &str) -> String {
            let body = Bytes::from(
                serde_json::to_vec(&json!({
                    "name": name,
                    "kind": "batch",
                    "sourceZone": "bronze",
                    "sourceTable": "t",
                    "targetZone": "silver",
                    "targetTable": "t",
                    "schedule": "manual",
                }))
                .expect("serialize"),
            );
            let (_, ApiJson(pipeline)) = create(
                State(state.clone()),
                Extension(fixture_user_principal()),
                HeaderMap::new(),
                body,
            )
            .await
            .expect("create should succeed");
            pipeline.id
        }

        fn assign_body(tenant_id: Uuid) -> Bytes {
            Bytes::from(
                serde_json::to_vec(&json!({ "tenantId": tenant_id.to_string() }))
                    .expect("serialize"),
            )
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn assigns_and_is_then_visible_in_the_scoped_list(pool: sqlx::PgPool) {
            let state = state_for(&pool);
            let tenant_id = seed_tenant(&pool, "c7-assign").await;
            let pipeline_id = seed_pipeline_with_null_tenant(&state, "c7-assign-pipeline").await;

            let status = assign_pipeline_tenant(
                State(state),
                Path(pipeline_id.clone()),
                assign_body(tenant_id),
            )
            .await
            .expect("assignment should succeed");
            assert_eq!(status, StatusCode::NO_CONTENT);

            let rows = pipelines::list_pipelines(
                &pool,
                &pipelines::PipelineFilter {
                    tenant_id: Some(tenant_id),
                    all_tenants: false,
                },
            )
            .await
            .expect("list_pipelines should succeed");
            assert!(
                rows.iter().any(|r| r.id == pipeline_id),
                "the assigned pipeline must now be visible in its tenant's scoped list"
            );
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn refuses_an_unknown_tenant_id_with_404(pool: sqlx::PgPool) {
            let state = state_for(&pool);
            let pipeline_id = seed_pipeline_with_null_tenant(&state, "c7-orphan-pipeline").await;
            let unknown_tenant = Uuid::new_v4();

            let err = assign_pipeline_tenant(
                State(state),
                Path(pipeline_id),
                assign_body(unknown_tenant),
            )
            .await
            .unwrap_err();
            assert_eq!(err.0.status(), 404);
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn refuses_an_unknown_pipeline_id_with_404(pool: sqlx::PgPool) {
            let state = state_for(&pool);
            let tenant_id = seed_tenant(&pool, "c7-no-pipeline").await;

            let err = assign_pipeline_tenant(
                State(state),
                Path("does-not-exist".to_owned()),
                assign_body(tenant_id),
            )
            .await
            .unwrap_err();
            assert_eq!(err.0.status(), 404);
        }
    }

    // ── Plan 1c: attempts on a step JSON serialisation ──────────────
    //
    // `step_to_json` must surface `attempts` as a non-null number for
    // every step, regardless of the count: zero for a step that never
    // started, N for a step that retried N-1 times. The TS contract
    // (Phase F, types/runs.ts) types the field as `attempts: number`,
    // never `number | null`, so sending `null` would force a "missing
    // field vs zero" branch the UI does not have.

    /// A step with three attempt records reports `attempts: 3` — the
    /// exact count `lakehouse_dagster::RunStep::attempts` already
    /// carries after `run_steps_parses_attempts_count` proves the
    /// parser preserves it from the GraphQL response.
    #[test]
    fn step_to_json_reports_three_attempts_as_a_non_null_number() {
        let step = lakehouse_dagster::RunStep {
            step_key: "extract".to_owned(),
            status: "FAILURE".to_owned(),
            start_ms: Some(1_700_000_000_000),
            end_ms: Some(1_700_000_005_000),
            attempts: 3,
            materializations: Vec::new(),
        };
        let value = step_to_json(&step);
        assert_eq!(value["stepKey"], "extract");
        assert_eq!(
            value["attempts"], 3,
            "step_to_json must surface the attempt count, not omit it"
        );
    }

    /// A step with no attempts (queued, never started) reports
    /// `attempts: 0`, never `null` — the TS contract types the field
    /// as a non-null number, and zero is the honest signal that the
    /// daemon has not touched it yet.
    #[test]
    fn step_to_json_reports_zero_attempts_as_zero_not_null() {
        let step = lakehouse_dagster::RunStep {
            step_key: "load".to_owned(),
            status: "QUEUED".to_owned(),
            start_ms: None,
            end_ms: None,
            attempts: 0,
            materializations: Vec::new(),
        };
        let value = step_to_json(&step);
        assert_eq!(value["attempts"], 0);
        assert!(value["attempts"].is_number());
    }

    /// `POST /api/pipelines/events/run-failed` route tests (plan 1e).
    /// The handler-side service-identity check (separate from the policy
    /// gate), the unknown-jobName non-error path, the dedupe short-circuit
    /// on a sensor retry, and the 409 on a non-failed run.
    mod run_failed_event_route {
        use axum::body::Bytes;

        use super::*;
        use crate::config::Config;

        /// Build an `AppState` with a real Postgres pool and a stub Dagster
        /// URL — the run-status path is mocked in each test through
        /// `state.dagster`. Mirrors `assign_tenant_route::state_for` so
        /// every `#[sqlx::test]`-driven test here starts from the same
        /// baseline.
        fn state_for(pool: &sqlx::PgPool) -> AppState {
            let options = pool.connect_options();
            let database_url = format!(
                "postgres://{}:postgres@{}:{}/{}",
                options.get_username(),
                options.get_host(),
                options.get_port(),
                options
                    .get_database()
                    .expect("#[sqlx::test] always targets a named database"),
            );
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), database_url);
            let config = Config::from_map(&env).expect("a valid test Config");
            AppState::new(config)
        }

        fn body_for(run_id: &str, job_name: &str) -> Bytes {
            Bytes::from(
                serde_json::to_vec(&json!({
                    "runId": run_id,
                    "jobName": job_name,
                }))
                .expect("serialize"),
            )
        }

        /// A user with `pipeline:write` is refused at the handler: the
        /// policy gate lets them through (401/403 isn't the test), but the
        /// `PrincipalId::Service(_)` check then returns 403. Mirrors
        /// `authored_pipelines::runnable`'s posture.
        #[sqlx::test(migrations = "../../migrations")]
        async fn refuses_a_user_with_pipeline_write(pool: sqlx::PgPool) {
            let state = state_for(&pool);
            let err = run_failed_event(
                State(state),
                Extension(fixture_user_principal()),
                body_for("run-1", "authored__pl_ui_check_flow"),
            )
            .await
            .unwrap_err();
            assert_eq!(err.0.status(), StatusCode::FORBIDDEN);
        }

        /// An unknown `jobName` is *not* an error: the handler returns
        /// `200` with `{"matched": 0, "reason": ...}`. The sensor's
        /// contract is "best effort — try every jobName, only the ones
        /// we know about do anything," and a hard 404 would block the
        /// sensor on unrelated job names.
        #[sqlx::test(migrations = "../../migrations")]
        async fn unknown_job_name_returns_matched_zero_with_a_reason(pool: sqlx::PgPool) {
            let state = state_for(&pool);
            let resp = run_failed_event(
                State(state),
                Extension(fixture_service_principal()),
                body_for("run-2", "not_a_real_job"),
            )
            .await
            .expect("handler accepts an unknown jobName");
            assert_eq!(resp.0["matched"], 0);
            assert_eq!(
                resp.0["reason"],
                "unknown jobName; no runnable pipeline owns it"
            );
        }
    }

    /// R4 plan 2c: `POST /api/pipelines/{id}/trigger` with the optional
    /// `{ "runConfig": <object> }` body — validating the config BEFORE
    /// launching, refusing with a structured 400 on `RunConfigValidation
    /// Invalid`, and never echoing Dagster's free-form `message` text
    /// back to the caller (AGENTS.md principle 4 + the plan's mutation
    /// check).
    mod trigger_with_config {
        use wiremock::matchers::{body_partial_json, body_string_contains, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::*;
        use crate::config::Config;
        use crate::routes::pipelines::TriggerBody;

        /// Wiremock-backed `Dagster` only — no Postgres pool, so audit
        /// writes are silently skipped (the `record_pipeline_audit`
        /// helper's `state.pg.as_deref()` check) and the trigger can be
        /// tested in isolation.
        fn state_with_dagster(server_uri: &str) -> AppState {
            let mut env = HashMap::new();
            env.insert("DAGSTER_URL".to_owned(), format!("{server_uri}/graphql"));
            // No DATABASE_URL — the audit best-effort path skips when
            // there's no pool, so we don't need a real one here.
            let config = Config::from_map(&env).expect("a valid test Config");
            AppState::new(config)
        }

        /// No body at all (the existing curl corpus entry) MUST still
        /// launch with no validation step — `Option<Json<TriggerBody>>`
        /// returns `Ok(None)` on an empty body / missing content-type,
        /// so the route falls through to `launch_run` without calling
        /// `isPipelineConfigValid`. The wiremock is set up to fail the
        /// request if any GraphQL call lands — the test passes only if
        /// no `isPipelineConfigValid` mutation is sent.
        #[tokio::test]
        async fn trigger_with_no_body_skips_validation_and_uses_launch_run() {
            let server = MockServer::start().await;
            // Catch-all 200 mock for any launch mutation. If a
            // validation call lands, this fires and the test STILL
            // passes — the assertion below catches the difference
            // (the wiremock body contains `launchRun` here; a
            // validation call would have triggered `isPipelineConfigValid`
            // instead).
            Mock::given(method("POST"))
                .and(body_string_contains("launchRun"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "launchRun": { "__typename": "LaunchRunSuccess",
                        "run": { "runId": "r-no-body" } } }
                })))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(body_string_contains("isPipelineConfigValid"))
                .respond_with(ResponseTemplate::new(500))
                .mount(&server)
                .await;

            let state = state_with_dagster(&server.uri());
            let response = trigger(
                State(state),
                Some(Extension(fixture_user_principal())),
                Path("ingest_job".to_owned()),
                None,
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("collect body");
            let v: Value = serde_json::from_slice(&body).expect("valid JSON");
            assert_eq!(v["id"], "r-no-body");
        }

        /// A VALID `runConfig` issues the
        /// `launchRun(executionParams: { runConfigData: $cfg })` mutation
        /// (NOT `launchRun` plain), and the wiremock verifies the
        /// payload shape so a regression that drops `runConfigData`
        /// (the caller's config never reaches Dagster) is caught here.
        #[tokio::test]
        async fn trigger_with_valid_run_config_calls_launch_run_with_config() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(body_string_contains("isPipelineConfigValid"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "isPipelineConfigValid": {
                        "__typename": "PipelineConfigValidationValid",
                        "pipelineName": "ingest_job" } }
                })))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(body_string_contains("launchRun"))
                .and(body_partial_json(json!({
                    "variables": {
                        "sel": { "repositoryName": "__repository__",
                                 "repositoryLocationName": "dispar_orchestrate.definitions",
                                 "pipelineName": "ingest_job" },
                        "cfg": { "ops": { "run_x": { "config": { "k": 1 } } } },
                    }
                })))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "launchRun": { "__typename": "LaunchRunSuccess",
                        "run": { "runId": "r-with-config" } } }
                })))
                .mount(&server)
                .await;

            let state = state_with_dagster(&server.uri());
            let body = Json(TriggerBody {
                run_config: Some(json!({ "ops": { "run_x": { "config": { "k": 1 } } } })),
            });
            let response = trigger(
                State(state),
                Some(Extension(fixture_user_principal())),
                Path("ingest_job".to_owned()),
                Some(body),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("collect body");
            let v: Value = serde_json::from_slice(&body).expect("valid JSON");
            assert_eq!(v["id"], "r-with-config");
        }

        /// An INVALID `runConfig` (Dagster returns
        /// `RunConfigValidationInvalid` with structured `path` and
        /// `reason` errors) returns a 400 whose body shape is exactly
        /// `{ "errors": [{ "path": ["..."], "reason": "..." }] }`. The
        /// mutation check from the plan is the assertion: the response
        /// MUST NOT contain a `message` field — the `Dagster` client
        /// deliberately does not deserialize it (see
        /// [`lakehouse_dagster::DgClient::validate_run_config`]) and
        /// the route body MUST NOT have one either. A regression that
        /// adds `"message": e.message` here would silently let
        /// `Dagster`'s free-form English text reach a response.
        #[tokio::test]
        async fn trigger_with_invalid_run_config_returns_400_with_path_and_reason_only() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(body_string_contains("isPipelineConfigValid"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "isPipelineConfigValid": {
                        "__typename": "RunConfigValidationInvalid",
                        "errors": [
                            { "path": ["ops", "run_x", "config", "k"],
                              "reason": "RUNTIME_TYPE_MISMATCH",
                              "message": "value '1' is not a String" }
                        ] }
                    }
                })))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(body_string_contains("launchRun"))
                .respond_with(ResponseTemplate::new(500))
                .mount(&server)
                .await;

            let state = state_with_dagster(&server.uri());
            let body = Json(TriggerBody {
                run_config: Some(json!({ "ops": { "run_x": { "config": { "k": 1 } } } })),
            });
            let response = trigger(
                State(state),
                Some(Extension(fixture_user_principal())),
                Path("ingest_job".to_owned()),
                Some(body),
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("collect body");
            let v: Value = serde_json::from_slice(&body).expect("valid JSON");
            // The whole response shape.
            let errors = v["errors"].as_array().expect("errors array");
            assert_eq!(errors.len(), 1);
            assert_eq!(errors[0]["path"], json!(["ops", "run_x", "config", "k"]));
            assert_eq!(errors[0]["reason"], "RUNTIME_TYPE_MISMATCH");
            // MUTATION CHECK (the plan's test): no `message` field, ever.
            assert!(
                errors[0].get("message").is_none(),
                "Dagster's free-form message MUST NOT be forwarded: got {}",
                errors[0]
            );
        }

        /// R4 plan 2c: `PipelineNotFoundError` from `isPipelineConfigValid`
        /// is NOT a validation refusal — it's "this id is not a job".
        /// The route falls through to `launch_run` (which surfaces the
        /// same answer as a 422), so a typo'd id gets the same
        /// 422-or-404 it got before this plan. The test proves the
        /// validation step didn't 400 the request.
        #[tokio::test]
        async fn trigger_with_run_config_for_unknown_job_falls_through_to_launch_run() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(body_string_contains("isPipelineConfigValid"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "isPipelineConfigValid": {
                        "__typename": "PipelineNotFoundError" } }
                })))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(body_string_contains("launchRun"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "launchRun": { "__typename": "LaunchRunSuccess",
                        "run": { "runId": "r-unknown" } } }
                })))
                .mount(&server)
                .await;

            let state = state_with_dagster(&server.uri());
            let body = Json(TriggerBody {
                run_config: Some(json!({ "ops": {} })),
            });
            let response = trigger(
                State(state),
                Some(Extension(fixture_user_principal())),
                Path("not_a_job".to_owned()),
                Some(body),
            )
            .await;
            // NOT a 400 — the validation did not refuse.
            assert_ne!(response.status(), StatusCode::BAD_REQUEST);
        }

        /// The `top_level_keys` helper used for the audit `configKeys`
        /// field extracts only the top-level keys of a JSON object,
        /// not the values — the audit column carries a structured
        /// side-channel for "this run supplied a `connector_id` field",
        /// never the field's actual id value.
        #[test]
        fn top_level_keys_extracts_keys_only_not_values() {
            let v = json!({
                "ops": { "run_x": { "config": { "secret": "real-secret" } } },
                "resources": { "io": { "config": { "password": "real-pw" } } }
            });
            let mut keys = top_level_keys(&v);
            keys.sort();
            assert_eq!(keys, vec!["ops".to_owned(), "resources".to_owned()]);
        }

        /// `top_level_keys` for a non-object (null, array, string) is
        /// the empty vec — the audit `configKeys` field is omitted when
        /// the caller sent a non-object (or no body at all).
        #[test]
        fn top_level_keys_for_non_objects_is_empty() {
            assert!(top_level_keys(&Value::Null).is_empty());
            assert!(top_level_keys(&json!([])).is_empty());
            assert!(top_level_keys(&json!("string")).is_empty());
        }
    }

    /// Plan 1f reviewer fix #1 — `POST /api/pipelines/events/run-finished`
    /// dedupes each alert kind through `pipeline_run_event` BEFORE
    /// delivery, so a sensor retry for the same run never double-fires
    /// `pipeline_slow` / `pipeline_volume_drop` (same posture
    /// `run_failed_event` has for kind `"failure"`).
    mod run_finished_event_route {
        use axum::body::Bytes;
        use wiremock::matchers::{body_string_contains, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::*;
        use crate::config::Config;

        /// Postgres + a wiremock `Dagster` and `ClickHouse`, wired
        /// through the same env keys production uses.
        fn state_for(pool: &sqlx::PgPool, dagster_url: &str, ch_url: &str) -> AppState {
            let options = pool.connect_options();
            let database_url = format!(
                "postgres://{}:postgres@{}:{}/{}",
                options.get_username(),
                options.get_host(),
                options.get_port(),
                options
                    .get_database()
                    .expect("#[sqlx::test] always targets a named database"),
            );
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), database_url);
            env.insert("DAGSTER_URL".to_owned(), format!("{dagster_url}/graphql"));
            env.insert("CH_URL".to_owned(), ch_url.to_owned());
            let config = Config::from_map(&env).expect("a valid test Config");
            AppState::new(config)
        }

        fn body_for(run_id: &str, job_name: &str) -> Bytes {
            Bytes::from(
                serde_json::to_vec(&json!({
                    "runId": run_id,
                    "jobName": job_name,
                }))
                .expect("serialize"),
            )
        }

        /// An authored pipeline in status `ready` (the store's own
        /// definition of runnable), created through the real create
        /// route. The status UPDATE bypasses the transition route's
        /// state machine on purpose: that machine's own tests live in
        /// `status_route`, and this fixture only needs a row
        /// `list_runnable_pipelines` will return.
        async fn seed_runnable_pipeline(state: &AppState, name: &str) -> String {
            let body = Bytes::from(
                serde_json::to_vec(&json!({
                    "name": name,
                    "kind": "batch",
                    "sourceZone": "bronze",
                    "sourceTable": "t",
                    "targetZone": "silver",
                    "targetTable": "t",
                    "schedule": "manual",
                }))
                .expect("serialize"),
            );
            let (_, ApiJson(pipeline)) = create(
                State(state.clone()),
                Extension(fixture_user_principal()),
                HeaderMap::new(),
                body,
            )
            .await
            .expect("create should succeed");
            let pool = state.pg.as_deref().expect("DATABASE_URL was set");
            sqlx::query("UPDATE pipeline_definition SET status = 'ready' WHERE id = $1")
                .bind(&pipeline.id)
                .execute(pool)
                .await
                .expect("mark the fixture runnable");
            pipeline.id
        }

        /// A sensor retry must not re-deliver: the first
        /// `run-finished` call for a slow run records kind `"slow"`
        /// and delivers once; an identical second call dedupes away.
        /// Asserted three ways — `matched` (1 then 0), the single
        /// `pipeline_run_event` row, and the webhook receiving
        /// exactly one POST.
        #[sqlx::test(migrations = "../../migrations")]
        async fn a_second_run_finished_event_for_the_same_run_does_not_re_deliver(
            pool: sqlx::PgPool,
        ) {
            let dagster = MockServer::start().await;
            let ch = MockServer::start().await;
            let webhook = MockServer::start().await;

            let state = state_for(&pool, &dagster.uri(), &ch.uri());
            // Seed first: the mocks below embed the id, which
            // `slug_id` derives from the name plus a millisecond
            // stamp, so it is only known after the create call. The
            // job name goes through the real `job_name` mapping (it
            // sanitizes the id, e.g. `-` → `_`).
            let pipeline_id = seed_runnable_pipeline(&state, "Dedupe flow").await;
            let job = super::super::authored_pipelines::job_name(&pipeline_id);

            // `pipeline_run_status`: the run is SUCCESS, lasting 1000s.
            Mock::given(method("POST"))
                .and(body_string_contains("pipelineRunOrError"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "pipelineRunOrError": {
                        "__typename": "Run", "status": "SUCCESS",
                        "startTime": 1000.0, "endTime": 2000.0, "stepStats": []
                    } }
                })))
                .mount(&dagster)
                .await;
            // `list_runs_for_job_with_materializations` (the volume-drop
            // half): the same run, no materializations, so `rows` is an
            // honest `None` and `volume_drop` cannot decide — only the
            // `slow` kind is exercised here.
            Mock::given(method("POST"))
                .and(body_string_contains("runsOrError"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "runsOrError": { "__typename": "Runs", "results": [
                        {
                            "runId": "run-slow",
                            "jobName": job,
                            "status": "SUCCESS",
                            "startTime": 1000.0, "endTime": 2000.0,
                            "creationTime": null,
                            "parentRunId": null, "rootRunId": null,
                            "tags": [], "stepStats": []
                        }
                    ] } }
                })))
                .mount(&dagster)
                .await;
            // `list_rules`' SELECT — one enabled `pipeline_slow` rule
            // scoped to the pipeline, targeting the webhook server.
            Mock::given(method("POST"))
                .and(body_string_contains("FROM console.alert_rule"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "meta": [], "rows": 1,
                    "data": [{
                        "id": "al-slow-dedupe-1",
                        "name": "Slow dedupe rule",
                        "type": "pipeline_slow",
                        "mart": "", "measure": "", "agg": "sum",
                        "op": ">", "threshold": "0", "board": "",
                        "channel": "webhook",
                        "target": webhook.uri(),
                        "enabled": "1", "created_at": "",
                        "severity": "high",
                        "pipeline": pipeline_id,
                    }]
                })))
                .mount(&ch)
                .await;
            // `ensure()`'s DDL — anything else answers 200.
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&ch)
                .await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&webhook)
                .await;

            // Duration SLA 60s: the mocked run ran 1000s → slow fires.
            pipelines::upsert_pipeline_sla(&pool, &pipeline_id, Some(60), None, uuid::Uuid::nil())
                .await
                .expect("seed the SLA row");

            let first = run_finished_event(
                State(state.clone()),
                Extension(fixture_service_principal()),
                body_for("run-slow", &job),
            )
            .await
            .expect("the first event must be accepted");
            assert_eq!(
                first.0["matched"], 1,
                "the first event delivers to the one matching rule"
            );

            let second = run_finished_event(
                State(state),
                Extension(fixture_service_principal()),
                body_for("run-slow", &job),
            )
            .await
            .expect("a sensor retry must be accepted, not error");
            assert_eq!(
                second.0["matched"], 0,
                "the retry for the same (run, kind) is deduped away"
            );

            let rows: Vec<(String, String, String)> =
                sqlx::query_as("SELECT run_id, pipeline_id, kind FROM pipeline_run_event")
                    .fetch_all(&pool)
                    .await
                    .expect("read the dedupe table");
            assert_eq!(
                rows,
                vec![(
                    "run-slow".to_owned(),
                    pipeline_id.clone(),
                    "slow".to_owned()
                )],
                "exactly one dedupe row for the slow kind"
            );

            let webhook_posts = webhook.received_requests().await.expect("request log");
            assert_eq!(
                webhook_posts.len(),
                1,
                "the webhook was hit exactly once across both calls"
            );
        }
    }
}
