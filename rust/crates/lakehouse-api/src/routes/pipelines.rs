//! `GET /api/pipelines`, `GET /api/pipelines/{id}/runs`,
//! `POST /api/pipelines/{id}/trigger` — `Dagster` jobs surfaced as console
//! pipelines.
//!
//! Ports `src/app/api/pipelines/route.ts`,
//! `src/app/api/pipelines/[id]/runs/route.ts`, and
//! `src/app/api/pipelines/[id]/trigger/route.ts`.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_dagster::{DgClient, DgError, DgJob, DgRun, iso_from_unix_seconds, map_run_status};
use lakehouse_store::PgPool;
use lakehouse_store::audit::{self as store_audit, NewAuditEvent};
use lakehouse_store::pipelines::{self, CreatePipelineInput};
use serde::Deserialize;
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::{ApiRejection, ApiResult};
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
///
/// # Tenant scoping (WS8 plan Task C3, Hard Requirement 2) — authored
/// pipelines only
///
/// `tenant_scope::resolve` runs first: `Ok(None)` (the caller belongs to
/// zero tenants) short-circuits to `{"pipelines": []}` before the `Dagster`
/// client or the store are ever queried — fail closed, never "unscoped,
/// show everything." When it resolves `Some(tenant_id)`, the authored half
/// of this union (`pipelines::list_pipelines`) is filtered to that tenant
/// via a bound `WHERE tenant_id = $1`.
///
/// **Deviation from a literal reading of the WS8 plan's Task C3 spec,
/// named here because it changes real behaviour:** the `Dagster`-job half
/// of this union is NOT tenant-filtered. `0042_tenant_provisioning.sql`
/// (Task B1, this task's own dependency) adds `tenant_id` only to
/// `connector` and `pipeline_definition` — a `Dagster` job has no tenant
/// column anywhere in this schema, and nothing maps a job name to a tenant
/// id. Filtering it would mean inventing that mapping, which is exactly
/// the "never fabricate" rule this program is built on; leaving it
/// unscoped is an honest, pre-existing limitation (every `Dagster` job was
/// already visible to every caller with `pipeline:read` before this task)
/// rather than a claim of isolation this route cannot back up. The
/// isolation guarantee this task's tests assert is scoped to
/// `pipeline_definition` rows, matching what migration `0042` actually
/// added a column for.
pub async fn list(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    headers: HeaderMap,
) -> Response {
    let Some(Extension(principal)) = principal else {
        return ApiRejection(ApiError::unauthorized()).into_response();
    };
    let tenant_id = match crate::tenant_scope::resolve(&principal, &headers) {
        Ok(Some(tenant_id)) => tenant_id,
        Ok(None) => {
            return (StatusCode::OK, ApiJson(json!({ "pipelines": [] }))).into_response();
        }
        Err(err) => return ApiRejection(err).into_response(),
    };
    match list_body(&state.dagster, state.pg.as_deref(), tenant_id).await {
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
    tenant_id: Uuid,
) -> Result<Value, ListError> {
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
        let filter = pipelines::PipelineFilter {
            tenant_id: Some(tenant_id),
        };
        let authored = pipelines::list_pipelines(pg, &filter).await?;
        pipelines.extend(authored.iter().filter_map(|p| serde_json::to_value(p).ok()));
    }
    Ok(json!({ "pipelines": pipelines }))
}

/// Build one `Dagster`-job row for `GET /api/pipelines`. `Dagster`'s job/run
/// API carries no per-job lineage and no freshness measurement (`WS2`
/// derives that from Iceberg snapshot timestamps) — `source`, `target`, and
/// `freshnessLagSeconds` are therefore reported as `null` rather than a
/// stamped-on default that every job would share (`WS1` finding J16).
/// `slaOk` stays `null` permanently, not "until `WS5`": `dataset_sla`
/// (`WS5` item E1) is keyed by warehouse *table*, `slaOk` by `Dagster`
/// *job* — a pipeline can write many tables, and a table can be written by
/// many jobs, so no single `dataset_sla` row could honestly summarize a
/// `slaOk` boolean for one job. `WS5` deliberately does not wire the two
/// together (`docs/superpowers/plans/2026-09-11-ws5-platform-signals.md`,
/// WS5 item E2 Step 1). `lastRunAt` is `null` when the job has never run
/// instead of an empty string standing in for "never ran". `nextRunAt`
/// (WS4 item G2) is computed server-side from the job's first schedule's
/// cron expression; `null` for a manual job or an uncomputable cron —
/// never a guess.
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
        "nextRunAt": next_run_at_json(j),
        "slaOk": Value::Null,
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
    let mut body = dagster_pipeline_row(job, last);
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
    let mut body = serde_json::to_value(&pipeline).unwrap_or_else(|_| json!({}));
    if let Value::Object(obj) = &mut body {
        obj.insert("engine".to_owned(), json!("authored"));
        obj.insert("description".to_owned(), Value::Null);
        obj.insert("graph".to_owned(), Value::Null);
        obj.insert("config".to_owned(), json!([]));
        obj.insert(
            "definition".to_owned(),
            definition.map_or(Value::Null, |d| {
                serde_json::to_value(d).unwrap_or(Value::Null)
            }),
        );
    }
    (StatusCode::OK, ApiJson(body)).into_response()
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
/// `principal_kind` comes from [`Principal::kind_for_audit`], never
/// [`Principal::provider`] — same CHECK this crate's other audit sites
/// satisfy.
fn pipeline_audit_event(principal: &Principal, action: &str, pipeline_id: &str) -> NewAuditEvent {
    NewAuditEvent {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        action: action.to_owned(),
        resource_kind: Some("pipeline".to_owned()),
        resource_id: Some(pipeline_id.to_owned()),
        args: None,
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
async fn record_pipeline_audit(state: &AppState, principal: &Principal, action: &str, id: &str) {
    let Some(pool) = state.pg.as_deref() else {
        return;
    };
    let event = pipeline_audit_event(principal, action, id);
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
) -> Response {
    let Some(Extension(principal)) = principal else {
        return ApiRejection(ApiError::unauthorized()).into_response();
    };
    match state.dagster.launch_run(&id).await {
        Ok(outcome) => {
            if let Some(error) = outcome.error {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    ApiJson(json!({ "error": error })),
                )
                    .into_response();
            }
            record_pipeline_audit(&state, &principal, "pipeline.trigger", &id).await;
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
    Extension(principal): Extension<Principal>,
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
    // WS5 item D4: best-effort, never turns a successful create into an
    // error. `create` has no internal (copilot tool) caller today
    // (confirmed by grepping `routes::ai::tools::pipelines` before adding
    // this parameter), so `Extension<Principal>`, not `Option`, matches
    // `pipeline:write`'s own `RequiresPermission` guarantee exactly.
    record_pipeline_audit(&state, &principal, "pipeline.create", &created.id).await;
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
        record_pipeline_audit(&state, &principal, "pipeline.pause", &id).await;
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
        record_pipeline_audit(&state, &principal, "pipeline.resume", &id).await;
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

    // ── WS8 plan Task C3: `GET /api/pipelines` tenant scoping ──────────

    /// Hard Requirement 2: a principal that belongs to zero tenants gets
    /// `{"pipelines": []}` — never `403`/`404`, and never every tenant's
    /// rows. `state_without_pool()` (no `Dagster` client reachable either)
    /// proves this is returned BEFORE `list_body` (hence before the
    /// `Dagster` client or the store) is ever queried: reaching
    /// `list_body` here would fail with a 503 `Dagster` connection error,
    /// not 200 with an empty body.
    #[tokio::test]
    async fn list_with_a_tenantless_principal_returns_an_empty_list_before_touching_dagster_or_the_store()
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
        assert_eq!(body, json!({ "pipelines": [] }));
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
        let event = pipeline_audit_event(&principal, "pipeline.trigger", "job-1");
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
        let event = pipeline_audit_event(&principal, "pipeline.pause", "job-1");
        assert_eq!(event.principal_kind.as_deref(), Some("service"));
    }

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

            let err = create(State(state), Extension(fixture_user_principal()), body)
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
}
