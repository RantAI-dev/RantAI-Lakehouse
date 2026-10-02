//! Pipeline tools: `trigger_lakehouse_build`, `get_build_status`. Moved
//! out of `ai.rs` unchanged (T0.1 registry refactor). Also the Tier 1
//! pipeline-operations tools (T1.3 of the copilot-operations-handover
//! plan): `list_pipelines`, `list_pipeline_runs`, `trigger_pipeline`,
//! `retry_pipeline_run`, `pause_pipeline`, `resume_pipeline`,
//! `cancel_pipeline_run`.
//!
//! Every T1.3 function calls the REAL `routes::pipelines::*` handler (via
//! [`super::response_to_value`]) rather than re-implementing the
//! authored-vs-`Dagster`-job branching those handlers already do — see
//! [`super::response_to_value`]'s doc comment for why this is a stronger
//! form of reuse than calling `DgClient` a second, independent way here.

use serde_json::{Map, Value, json};

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Json, Path, State};
use axum::http::HeaderMap;
use lakehouse_auth::Principal;

use axum::response::IntoResponse;

use super::{api_result_to_value, arg_str, response_to_value};
use crate::routes::pipelines::TriggerBody;
use crate::state::AppState;

/// The Dagster job the demo code location builds the whole lakehouse with.
/// The product code location (`dagster/dispar_orchestrate`) has no such
/// job: it ingests per connector, runs authored pipelines as their own
/// jobs, and exports Gold on its own schedule.
const DEMO_BUILD_JOB: &str = "refresh_lakehouse";

/// Rebuilds the lakehouse from what this deployment actually has.
///
/// This tool used to launch `refresh_lakehouse` unconditionally, a job
/// that exists only in the demo code location; on the product stack every
/// call failed. It now launches `refresh_lakehouse` when that job exists,
/// and otherwise runs the product's own steps in layer order:
///
/// 1. **Bronze**: an ingest run for every connector with a batch or
///    stream ingest spec the caller can see (CDC connectors are streamed
///    continuously by Debezium and have nothing to launch). Needs
///    `connector:manage`, like "Run now" in the console.
/// 2. **Silver/Gold**: every authored pipeline job (`authored__*`).
/// 3. **Gold Iceberg**: `gold_export_job`.
///
/// Every step reports what it launched or why it did not; nothing is
/// claimed that was not launched.
pub(super) async fn trigger_build(state: &AppState, principal: Option<&Principal>) -> Value {
    let Ok(jobs) = state.dagster.list_jobs().await else {
        return json!({ "error": "Dagster could not be reached to list its jobs" });
    };
    if jobs.iter().any(|j| j == DEMO_BUILD_JOB) {
        return match state.dagster.launch_run(DEMO_BUILD_JOB).await {
            Ok(outcome) => match outcome.error {
                Some(error) => json!({ "error": error }),
                None => json!({
                    "launched": true,
                    "runId": outcome.run_id,
                    "note": "The Bronze -> Silver -> Gold build is running. Check it with get_build_status.",
                }),
            },
            Err(_) => json!({ "error": "Dagster did not accept the build run" }),
        };
    }

    let mut launched: Vec<Value> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();

    ingest_step(state, principal, &mut launched, &mut skipped).await;

    // 2 and 3: authored pipelines, then the Gold export.
    let mut ordered: Vec<&String> = jobs
        .iter()
        .filter(|j| j.starts_with("authored__"))
        .collect();
    if ordered.is_empty() {
        skipped
            .push(json!({ "step": "pipelines", "reason": "no authored pipeline is ready to run" }));
    }
    if let Some(export) = jobs.iter().find(|j| *j == "gold_export_job") {
        ordered.push(export);
    } else {
        skipped.push(json!({ "step": "gold_export", "reason": "gold_export_job is not in this code location" }));
    }
    for job in ordered {
        let step = if job == "gold_export_job" {
            "gold_export"
        } else {
            "pipeline"
        };
        match state.dagster.launch_run(job).await {
            Ok(outcome) if outcome.error.is_none() => {
                launched.push(json!({ "step": step, "job": job, "runId": outcome.run_id }));
            }
            Ok(outcome) => {
                skipped.push(json!({ "step": step, "job": job, "error": outcome.error }));
            }
            Err(_) => skipped.push(
                json!({ "step": step, "job": job, "error": "Dagster did not accept the run" }),
            ),
        }
    }

    let first_run = launched
        .iter()
        .find_map(|l| l.get("runId").and_then(Value::as_str).map(str::to_owned));
    json!({
        "launched": launched,
        "skipped": skipped,
        "runId": first_run,
        "note": "Silver and Gold tables loaded by jobs outside this platform are not rebuilt by this step.",
    })
}

/// Step 1 of [`trigger_build`]: an ingest run for every visible connector
/// with a batch or stream ingest spec.
async fn ingest_step(
    state: &AppState,
    principal: Option<&Principal>,
    launched: &mut Vec<Value>,
    skipped: &mut Vec<Value>,
) {
    let may_ingest = principal.is_some_and(|p| p.has("connector:manage"));
    if may_ingest {
        let visible = super::connectors::list_connectors(state, principal).await;
        let visible_ids: Vec<String> = visible
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c.get("id").and_then(Value::as_str).map(str::to_owned))
            .collect();
        let ingestible = match state.pg.as_deref() {
            Some(pool) => lakehouse_store::connectors::list_ingestible_connectors(pool)
                .await
                .unwrap_or_default(),
            None => Vec::new(),
        };
        let mut any = false;
        for connector in ingestible.iter().filter(|c| visible_ids.contains(&c.id)) {
            any = true;
            if connector.ingest_mode == "cdc" {
                skipped.push(json!({ "step": "ingest", "connector": connector.id,
                    "reason": "CDC connector: Debezium streams it continuously" }));
                continue;
            }
            let result = response_to_value(
                crate::routes::connectors::ingest_run(
                    State(state.clone()),
                    Path(connector.id.clone()),
                )
                .await
                .into_response(),
            )
            .await;
            if result.get("error").is_some() || result.get("supported") == Some(&json!(false)) {
                skipped
                    .push(json!({ "step": "ingest", "connector": connector.id, "result": result }));
            } else {
                launched
                    .push(json!({ "step": "ingest", "connector": connector.id, "result": result }));
            }
        }
        if !any {
            skipped.push(json!({ "step": "ingest", "reason": "no connector has an ingest spec" }));
        }
    } else {
        skipped.push(json!({ "step": "ingest",
            "reason": "running connector ingest needs connector:manage, which this user does not have" }));
    }
}

pub(super) async fn get_build_status(dagster: &lakehouse_dagster::DgClient) -> Value {
    let jobs = dagster.list_jobs().await;
    let runs = dagster.list_runs(10).await;
    match (jobs, runs) {
        (Ok(jobs), Ok(runs)) => {
            let recent: Vec<Value> = runs
                .iter()
                .map(|r| {
                    json!({
                        "job": r.job_name,
                        "status": lakehouse_dagster::map_run_status(&r.status),
                        "startedAt": r.start_time.map(lakehouse_dagster::iso_from_unix_seconds),
                    })
                })
                .collect();
            json!({ "jobs": jobs, "recentRuns": recent })
        }
        (Err(err), _) | (_, Err(err)) => json!({ "error": err.to_string() }),
    }
}

// ── T2.2 Maintenance (see the copilot-operations-handover plan's C2) ────

/// Launches `bronze_maintenance_job` — the SAME job `DgClient::launch_run`
/// call [`trigger_build`] already makes for `refresh_lakehouse`, just a
/// different job name. There is deliberately no run-config here: per C2,
/// `launch_run` accepts a job name only, and the job itself always runs
/// its dry pass then its applied pass in one go — there is no way to ask
/// for only the dry half.
pub(super) async fn run_bronze_maintenance(dagster: &lakehouse_dagster::DgClient) -> Value {
    match dagster.launch_run("bronze_maintenance_job").await {
        Ok(outcome) => {
            if let Some(error) = outcome.error {
                return json!({ "error": error });
            }
            json!({
                "launched": true,
                "runId": outcome.run_id,
                "note": "Maintenance Bronze dijalankan: file data/manifest Iceberg yatim akan \
                         dihapus. Cek hasilnya dengan get_maintenance_metrics.",
            })
        }
        Err(err) => json!({ "error": err.to_string() }),
    }
}

// ── T1.3 pipeline-operations tools ──────────────────────────────────────

/// `headers: HeaderMap::new()` — no `X-Tenant` selection from the copilot
/// dispatcher today, so `tenant_scope::resolve` falls back to the
/// principal's own first tenant (or `None`/empty list if it belongs to
/// none), the same default an interactive caller gets by omitting the
/// header.
pub(super) async fn list_pipelines(state: &AppState, principal: Option<&Principal>) -> Value {
    let extension = principal.cloned().map(Extension);
    response_to_value(
        crate::routes::pipelines::list(State(state.clone()), extension, HeaderMap::new()).await,
    )
    .await
}

pub(super) async fn list_pipeline_runs(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    response_to_value(crate::routes::pipelines::runs(State(state.clone()), Path(id)).await).await
}

/// `principal` is forwarded as `Option<Extension<Principal>>` — the same
/// re-wrapping `routes::ai::tools::connectors::create_connector` already
/// does for `routes::connectors::create` (WS5 item D3/D4): this internal
/// call bypasses axum's auth middleware, so `routes::pipelines::trigger`
/// (which now writes a real `pipeline.trigger` `audit_event` under the real
/// principal) gets the SAME `Principal` the copilot dispatcher was
/// handed, and 401s honestly when there is none.
///
/// R4 plan 2c: `args["runConfig"]` (optional JSON object) is forwarded
/// to the route unchanged. The route validates the config against the
/// job's schema and either launches (on success) or returns a structured
/// 400 (on `RunConfigValidationInvalid`) — both of which the
/// `response_to_value` wrapper surfaces here as a 200 JSON object with
/// `{ error }` / the route's success body, since the copilot dispatcher
/// turns every `Response` from these tools into the same payload shape.
pub(super) async fn trigger_pipeline(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    let run_config = args.get("runConfig").cloned();
    let body = Json(TriggerBody { run_config });
    let extension = principal.cloned().map(Extension);
    response_to_value(
        crate::routes::pipelines::trigger(State(state.clone()), extension, Path(id), Some(body))
            .await,
    )
    .await
}

pub(super) async fn retry_pipeline_run(state: &AppState, args: &Map<String, Value>) -> Value {
    let run_id = arg_str(args, "runId");
    if run_id.is_empty() {
        return json!({ "error": "runId is required" });
    }
    // Plan 1c (R2, day-1): the copilot tool can ask for a specific
    // subset of steps via the `stepKeys` array — the route layer
    // builds the body, so the tool cannot drift from the console's
    // own contract for `{"strategy":"selected","stepKeys":[...]}`.
    // `fromFailure: true` keeps its existing meaning (re-runs only
    // the failed steps). When both are present, `stepKeys` wins —
    // it is the more specific request, and a model that names keys
    // is presumed to know what it wants.
    let body = if let Some(step_keys) = args.get("stepKeys").and_then(Value::as_array) {
        let keys_json = serde_json::to_string(step_keys).unwrap_or_else(|_| "[]".to_owned());
        Bytes::from(format!(
            r#"{{"strategy":"selected","stepKeys":{keys_json}}}"#
        ))
    } else if args.get("fromFailure").and_then(Value::as_bool) == Some(true) {
        Bytes::from_static(br#"{"strategy":"fromFailure"}"#)
    } else {
        Bytes::new()
    };
    response_to_value(
        crate::routes::pipelines::retry_run(State(state.clone()), Path(run_id), body).await,
    )
    .await
}

pub(super) async fn pause_pipeline(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    let extension = principal.cloned().map(Extension);
    response_to_value(
        crate::routes::pipelines::pause(State(state.clone()), extension, Path(id)).await,
    )
    .await
}

pub(super) async fn resume_pipeline(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    let extension = principal.cloned().map(Extension);
    response_to_value(
        crate::routes::pipelines::resume(State(state.clone()), extension, Path(id)).await,
    )
    .await
}

pub(super) async fn cancel_pipeline_run(state: &AppState, args: &Map<String, Value>) -> Value {
    let run_id = arg_str(args, "runId");
    if run_id.is_empty() {
        return json!({ "error": "runId is required" });
    }
    response_to_value(
        crate::routes::pipelines::cancel_run(State(state.clone()), Path(run_id)).await,
    )
    .await
}

pub(super) async fn create_pipeline(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let Some(principal) = principal else {
        return json!({ "error": "creating a pipeline needs a signed-in user" });
    };
    let mut body = args.clone();
    body.entry("kind").or_insert(json!("batch"));
    body.entry("schedule").or_insert(json!("manual"));
    api_result_to_value(
        crate::routes::pipelines::create(
            State(state.clone()),
            Extension(principal.clone()),
            // No request headers reach a tool call, so the pipeline lands in
            // the principal's first tenant: `tenant_scope::resolve`'s own
            // fallback when no `x-tenant` header is sent.
            HeaderMap::new(),
            axum::body::Bytes::from(Value::Object(body).to_string()),
        )
        .await,
    )
    .await
}

pub(super) async fn get_pipeline(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    response_to_value(crate::routes::pipelines::detail(State(state.clone()), Path(id)).await).await
}

pub(super) async fn mark_pipeline_ready(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    api_result_to_value(
        crate::routes::pipelines::set_status_route(
            State(state.clone()),
            Path(id),
            axum::body::Bytes::from(json!({ "status": "ready" }).to_string()),
        )
        .await,
    )
    .await
}

#[cfg(test)]
mod t1_3_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use super::*;
    use crate::config::Config;

    fn state() -> AppState {
        AppState::new(Config::from_map(&HashMap::new()).unwrap())
    }

    #[tokio::test]
    async fn every_id_or_run_id_tool_requires_its_argument() {
        let s = state();
        assert_eq!(
            list_pipeline_runs(&s, &Map::new()).await,
            json!({ "error": "id is required" })
        );
        assert_eq!(
            trigger_pipeline(&s, None, &Map::new()).await,
            json!({ "error": "id is required" })
        );
        assert_eq!(
            pause_pipeline(&s, None, &Map::new()).await,
            json!({ "error": "id is required" })
        );
        assert_eq!(
            resume_pipeline(&s, None, &Map::new()).await,
            json!({ "error": "id is required" })
        );
        assert_eq!(
            retry_pipeline_run(&s, &Map::new()).await,
            json!({ "error": "runId is required" })
        );
        assert_eq!(
            cancel_pipeline_run(&s, &Map::new()).await,
            json!({ "error": "runId is required" })
        );
    }
}
