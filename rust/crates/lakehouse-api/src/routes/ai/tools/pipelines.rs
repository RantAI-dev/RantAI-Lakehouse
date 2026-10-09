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
            // F2.7: `outcome.failure` is a typed `LaunchFailure`. The
            // tool body is the FIXED classified string below — Dagster's
            // own text was logged at the dagster crate boundary.
            Ok(outcome) => match outcome.failure {
                Some(_) => json!({ "error": "Dagster did not accept the build run" }),
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
            Ok(outcome) if outcome.failure.is_none() => {
                launched.push(json!({ "step": step, "job": job, "runId": outcome.run_id }));
            }
            // F2.7: `outcome.failure` is the typed `LaunchFailure`. The
            // tool's "skipped" `error` is the FIXED classified string —
            // Dagster's own text was logged at the dagster crate boundary.
            Ok(_outcome) => {
                skipped.push(
                    json!({ "step": step, "job": job, "error": "Dagster did not accept the run" }),
                );
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
            // F2.7: `outcome.failure` is the typed `LaunchFailure`. The
            // tool's `error` is the FIXED classified string — Dagster's
            // own text was logged at the dagster crate boundary.
            if outcome.failure.is_some() {
                return json!({ "error": "Dagster did not accept the maintenance run" });
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

pub(super) async fn list_pipeline_runs(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    // F2.1 (PR #59 review): forward the principal to the route so the
    // F2.1 scope check (`in_scope`) runs — a tool call WITHOUT a
    // principal (e.g. an internal caller that bypasses the AI auth
    // middleware) would see the route fail closed and return 404. Same
    // `principal: Option<&Principal>` shape as `trigger_pipeline` /
    // `pause_pipeline` already take.
    let extension = principal.cloned().map(Extension);
    response_to_value(
        crate::routes::pipelines::runs(State(state.clone()), extension, HeaderMap::new(), Path(id))
            .await,
    )
    .await
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
        crate::routes::pipelines::trigger(
            State(state.clone()),
            extension,
            HeaderMap::new(),
            Path(id),
            Some(body),
        )
        .await,
    )
    .await
}

pub(super) async fn retry_pipeline_run(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let run_id = arg_str(args, "runId");
    if run_id.is_empty() {
        return json!({ "error": "runId is required" });
    }
    // F2.9 (PR #59 review, `plans/pipelines/day-1-fixes/
    // f2-tenant-scope-and-run-config.md` Part D): `stepKeys` that is
    // NOT an array of strings used to silently fall through to
    // "re-run every step" — the `Value::as_array()` adapter returned
    // `None` for a string/object/null `stepKeys`, and the body
    // builder then took the empty-body branch (or the `fromFailure`
    // branch, whichever matched first). That is a silent
    // reinterpretation of an obvious caller bug: a model that typed
    // `"stepKeys": "foo"` deserved a 400, not a different run.
    //
    // The check is the same shape the rest of this file uses for
    // argument validation (`{"error": …}` keyed by a stable label,
    // like `id is required` above and `runId is required`).
    // Plan 1c (R2, day-1): the copilot tool can ask for a specific
    // subset of steps via the `stepKeys` array — the route layer
    // builds the body, so the tool cannot drift from the console's
    // own contract for `{"strategy":"selected","stepKeys":[...]}`.
    // `fromFailure: true` keeps its existing meaning (re-runs only
    // the failed steps). When both are present, `stepKeys` wins —
    // it is the more specific request, and a model that names keys
    // is presumed to know what it wants.
    let body = match args.get("stepKeys") {
        Some(Value::Array(step_keys)) => {
            // The route layer's body builder serializes the array
            // verbatim; an array containing a non-string would
            // survive the JSON encode and produce an invalid body
            // downstream. Reject now so the failure shape is the
            // same one the type-level mismatch above gives.
            if !step_keys.iter().all(Value::is_string) {
                return json!({
                    "error": "stepKeys must be an array of strings (e.g. [\"step_a\", \"step_b\"])"
                });
            }
            let keys_json = serde_json::to_string(step_keys).unwrap_or_else(|_| "[]".to_owned());
            Bytes::from(format!(
                r#"{{"strategy":"selected","stepKeys":{keys_json}}}"#
            ))
        }
        Some(_) => {
            return json!({
                "error": "stepKeys must be an array of strings (e.g. [\"step_a\", \"step_b\"])"
            });
        }
        None => {
            if args.get("fromFailure").and_then(Value::as_bool) == Some(true) {
                Bytes::from_static(br#"{"strategy":"fromFailure"}"#)
            } else {
                Bytes::new()
            }
        }
    };
    // F2.1 (PR #59 review): forward the principal so the route's runId
    // scope check (`enforce_run_id_pipeline_scope`) runs. A tool call
    // without a principal returns 404 from the same path it does for
    // a restricted caller — `Restricted(None)` sees nothing.
    let extension = principal.cloned().map(Extension);
    response_to_value(
        crate::routes::pipelines::retry_run(
            State(state.clone()),
            extension,
            HeaderMap::new(),
            Path(run_id),
            body,
        )
        .await,
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
        crate::routes::pipelines::pause(
            State(state.clone()),
            extension,
            HeaderMap::new(),
            Path(id),
        )
        .await,
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
        crate::routes::pipelines::resume(
            State(state.clone()),
            extension,
            HeaderMap::new(),
            Path(id),
        )
        .await,
    )
    .await
}

pub(super) async fn cancel_pipeline_run(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let run_id = arg_str(args, "runId");
    if run_id.is_empty() {
        return json!({ "error": "runId is required" });
    }
    // F2.1 (PR #59 review): forward the principal so the route's runId
    // scope check runs. Same posture as `retry_pipeline_run` above.
    let extension = principal.cloned().map(Extension);
    response_to_value(
        crate::routes::pipelines::cancel_run(
            State(state.clone()),
            extension,
            HeaderMap::new(),
            Path(run_id),
        )
        .await,
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
    // F2.1 (PR #59 review): tool calls run with no `x-tenant` header
    // and no principal — the helper fails closed and the route returns the
    // standard 404. A principal-bearing tool entry point already runs
    // the F2.1 scope check on the matching path.
    response_to_value(
        crate::routes::pipelines::detail(State(state.clone()), None, HeaderMap::new(), Path(id))
            .await,
    )
    .await
}

pub(super) async fn mark_pipeline_ready(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id is required" });
    }
    api_result_to_value(
        crate::routes::pipelines::set_status_route(
            State(state.clone()),
            None,
            HeaderMap::new(),
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

    use lakehouse_auth::{PermissionSet, PrincipalId};
    use lakehouse_dagster::LaunchFailure;
    use uuid::Uuid;

    use super::*;
    use crate::config::Config;

    fn state() -> AppState {
        AppState::new(Config::from_map(&HashMap::new()).unwrap())
    }

    fn state_with_dagster(server_uri: &str) -> AppState {
        let mut env = HashMap::new();
        env.insert("DAGSTER_URL".to_owned(), format!("{server_uri}/graphql"));
        AppState::new(Config::from_map(&env).expect("a valid test Config"))
    }

    /// A logged-in principal holding `pipeline:write` — the permission
    /// `POST /api/pipelines/{id}/trigger` is gated on, so
    /// `trigger_pipeline` tests exercise the tool with the identity
    /// the copilot dispatcher would actually pass (same fixture shape
    /// as `routes::ai::tools::connectors::fixture_user_principal`).
    fn fixture_user_principal() -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::from_u128(1)),
            tenant_ids: Vec::new(),
            display_name: "fixture".to_owned(),
            permissions: PermissionSet::parse("pipeline:write"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    #[tokio::test]
    async fn every_id_or_run_id_tool_requires_its_argument() {
        let s = state();
        assert_eq!(
            list_pipeline_runs(&s, None, &Map::new()).await,
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
            retry_pipeline_run(&s, None, &Map::new()).await,
            json!({ "error": "runId is required" })
        );
        assert_eq!(
            cancel_pipeline_run(&s, None, &Map::new()).await,
            json!({ "error": "runId is required" })
        );
    }

    // ── F2.7 sentinels — see `LakehousePlanDay1.md` (Sentinel-Test
    //    Discipline). Each test puts a UNIQUE sentinel in the upstream
    //    GraphQL response and asserts the tool body carries the FIXED
    //    classified string. The upstream detail is logged at the
    //    `lakehouse-dagster` crate boundary (`tracing::warn!`), not in
    //    the tool response.

    /// F2.7 SENTINEL: `trigger_build`'s `DEMO_BUILD_JOB` launch path.
    /// A `PythonError` from Dagster's `launchRun` carries a UNIQUE
    /// sentinel in `message`; the tool body's `error` MUST be the
    /// fixed `"Dagster did not accept the build run"` and the
    /// sentinel MUST NOT appear.
    #[tokio::test]
    async fn trigger_build_demo_path_returns_fixed_body_when_dagster_python_errors() {
        const SENTINEL: &str = "TOOL_TRIGGER_BUILD_PY_SENTINEL_444000";
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::body_string_contains(
                "repositoriesOrError",
            ))
            // Production `list_jobs` makes ONE GraphQL call whose
            // root field is `repositoriesOrError`; the response carries
            // `nodes[].jobs[].name` (see `lakehouse-dagster::list_jobs`).
            // Returning the `DEMO_BUILD_JOB` ("refresh_lakehouse") here
            // makes `trigger_build` take the demo launch path and call
            // `launchRun(DEMO_BUILD_JOB)`, whose mock below answers
            // `PythonError` so the FIXED refusal body is the assertion.
            // Earlier revisions of this test matched the queries
            // `listJobs` / `listJobsForRepository` from the console
            // client, but `list_jobs` does NOT emit either query — the
            // mock never matched, the response shape was missing
            // `nodes[].jobs[].name`, and `list_jobs` failed to
            // deserialize so `trigger_build` returned the early
            // "could not be reached" error instead of the launch
            // failure this test pins.
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "data": { "repositoriesOrError": {
                    "__typename": "RepositoryConnection",
                    "nodes": [{
                        "jobs": [{ "name": "refresh_lakehouse" }],
                    }]
                } }
            })))
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::body_string_contains("launchRun"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "data": { "launchRun": {
                    "__typename": "PythonError",
                    "message": format!("launch attempt refused — {SENTINEL}"),
                    "className": "DagsterInvalidDefinitionError",
                    "stack": [],
                    "causes": [],
                } }
            })))
            .mount(&server)
            .await;

        let state = state_with_dagster(&server.uri());
        let body = trigger_build(&state, None).await;
        let body_str = body.to_string();
        assert!(
            !body_str.contains(SENTINEL),
            "Dagster's PythonError message MUST NOT be forwarded into trigger_build body, got {body_str}",
        );
        assert_eq!(
            body,
            json!({ "error": "Dagster did not accept the build run" }),
            "trigger_build refusal body MUST be the fixed classified string",
        );
        // Sanity: the dagster crate classified the refusal to `Refused`.
        assert_eq!(
            LaunchFailure::Refused.as_str(),
            "refused",
            "F2.7 typed `LaunchFailure::Refused` classifier returns the fixed `refused` body fragment"
        );
    }

    /// F2.7 SENTINEL: `trigger_build`'s "no `demo_build_job`" path —
    /// which calls `launch_run` for each authored pipeline. A
    /// `PythonError` from Dagster's `launchRun` carries a UNIQUE
    /// sentinel in `message`; the tool body's per-step `skipped`
    /// `error` MUST be the fixed `"Dagster did not accept the run"`
    /// and the sentinel MUST NOT appear.
    #[tokio::test]
    async fn trigger_build_authored_pipelines_path_returns_fixed_body_when_dagster_python_errors() {
        const SENTINEL: &str = "TOOL_TRIGGER_AUTHORED_PY_SENTINEL_555111";
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::body_string_contains(
                "repositoriesOrError",
            ))
            // Production `list_jobs` returns the named jobs in this code
            // location; `trigger_build` only takes the demo launch path
            // when `DEMO_BUILD_JOB` (`refresh_lakehouse`) is present,
            // so the mock here returns the `authored__*` jobs the test
            // asserts on (the per-pipeline `launchRun` mock below
            // answers `PythonError`, and each errored pipeline is
            // recorded as a `skipped` entry). See the demo-path test
            // above for the same production-vs-test query mismatch.
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "data": { "repositoriesOrError": {
                    "__typename": "RepositoryConnection",
                    "nodes": [{
                        "jobs": [{ "name": "authored__orders" }],
                    }]
                } }
            })))
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::body_string_contains("launchRun"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "data": { "launchRun": {
                    "__typename": "PythonError",
                    "message": format!("launch attempt refused — {SENTINEL}"),
                    "className": "DagsterInvalidDefinitionError",
                    "stack": [],
                    "causes": [],
                } }
            })))
            .mount(&server)
            .await;

        let state = state_with_dagster(&server.uri());
        let body = trigger_build(&state, None).await;
        let body_str = body.to_string();
        assert!(
            !body_str.contains(SENTINEL),
            "Dagster's PythonError message MUST NOT be forwarded into trigger_build body, got {body_str}",
        );
        let skipped = body["skipped"]
            .as_array()
            .expect("skipped is an array")
            .iter()
            .find(|s| s.get("job") == Some(&json!("authored__orders")))
            .expect("skipped entry for authored__orders");
        assert_eq!(
            skipped["error"], "Dagster did not accept the run",
            "trigger_build authored-pipeline refusal MUST be the fixed classified string",
        );
    }

    /// F2.7 SENTINEL: `run_bronze_maintenance` returning a
    /// `RunNotFoundError` from Dagster's `launchRun` carries a UNIQUE
    /// sentinel in `message`; the tool body's `error` MUST be the
    /// fixed `"Dagster did not accept the maintenance run"` and the
    /// sentinel MUST NOT appear.
    #[tokio::test]
    async fn run_bronze_maintenance_returns_fixed_body_when_dagster_python_errors() {
        const SENTINEL: &str = "TOOL_BRONZE_MAINTENANCE_SENTINEL_666222";
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::body_string_contains("launchRun"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "data": { "launchRun": {
                    "__typename": "PythonError",
                    "message": format!("maintenance refused — {SENTINEL}"),
                    "className": "DagsterInvalidDefinitionError",
                    "stack": [],
                    "causes": [],
                } }
            })))
            .mount(&server)
            .await;

        let state = state_with_dagster(&server.uri());
        let dagster = state.dagster.clone();
        let body = run_bronze_maintenance(&dagster).await;
        let body_str = body.to_string();
        assert!(
            !body_str.contains(SENTINEL),
            "Dagster's PythonError message MUST NOT be forwarded into run_bronze_maintenance body, got {body_str}",
        );
        assert_eq!(
            body,
            json!({ "error": "Dagster did not accept the maintenance run" }),
            "run_bronze_maintenance refusal body MUST be the fixed classified string",
        );
        assert_eq!(
            LaunchFailure::Refused.as_str(),
            "refused",
            "F2.7 typed `LaunchFailure::Refused` classifier returns the fixed `refused` body fragment"
        );
    }

    // ── F2.9 (`plans/pipelines/day-1-fixes/f2-tenant-scope-and-run-config.md`
    //    Part D) — the tool body's `stepKeys` validation and
    //    `runConfig` forwarding contracts. The route-level handler
    //    keeps its existing `{"error": "..."}` shape for argument
    //    validation, so the `stepKeys` tests below mirror the
    //    `runId is required` pattern from `run_id_required` above;
    //    the `runConfig` test pins tool → route with a wiremock
    //    `launchRun` matcher on `variables.cfg` instead, because the
    //    contract there is forwarding, not validation.

    /// F2.9 (PR #59 review): a `stepKeys` that is NOT a JSON array of
    /// strings must surface as an `{"error": "..."}` tool result,
    /// not silently fall through to a "re-run every step" body. The
    /// pre-F2.9 `Value::as_array()` adapter returned `None` for a
    /// string/null/object `stepKeys`, and the body builder then
    /// took the `fromFailure` / empty branch — a silent
    /// reinterpretation of an obvious caller bug.
    ///
    /// **Planned**: dropping the `Some(_)` / `Some(Value::Array(_))`
    /// type guards in [`retry_pipeline_run`] makes the test fail — a
    /// string `stepKeys` would fall through to `fromFailure` (or
    /// the empty-body branch), bypassing the `{"error": ...}`
    /// response.
    #[tokio::test]
    async fn retry_pipeline_run_returns_an_error_for_non_array_step_keys() {
        let state = state();
        for bad in [
            json!("step_a"),            // string
            json!(null),                // null
            json!({ "key": "step_a" }), // object
            json!(["step_a", 42]),      // array of mixed types
        ] {
            let mut args = Map::new();
            args.insert("runId".to_owned(), json!("abc-123"));
            args.insert("stepKeys".to_owned(), bad.clone());
            let body = retry_pipeline_run(&state, None, &args).await;
            assert_eq!(
                body,
                json!({
                    "error": "stepKeys must be an array of strings (e.g. [\"step_a\", \"step_b\"])"
                }),
                "non-array stepKeys {bad} must surface as an error result, not be silently reinterpreted"
            );
        }
    }

    /// F2.9 (PR #59 review): a valid array of strings STILL works —
    /// the validation rejects malformed input without breaking the
    /// happy path. The wiremock here is only there to keep the
    /// `state_with_dagster` route from hitting a real host; the
    /// assertion is about the ARG VALIDATION, not the network
    /// response — a malformed `stepKeys` never reaches the network
    /// call.
    #[tokio::test]
    async fn retry_pipeline_run_accepts_a_string_array_step_keys() {
        let server = wiremock::MockServer::start().await;
        // Three wiremock arms are needed:
        //
        // 1. `pipelineRunOrError` — the route's
        //    `enforce_run_id_pipeline_scope` looks up the run's
        //    pipeline name BEFORE re-executing. Returning a name with
        //    no `pl-` prefix short-circuits the scope check before any
        //    DB read (the function exits on the
        //    `pipeline_id_from_job_name` `else` branch).
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::body_string_contains(
                "pipelineRunOrError",
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "data": { "pipelineRunOrError": {
                    "__typename": "Run",
                    "pipelineName": "silver_rebuild",
                } }
            })))
            .mount(&server)
            .await;
        // 2. `pipelineRunOrError` (status query) + `runOrError`
        //    (steps query) — the Selected strategy requires both:
        //    `pipeline_run_status` confirms the run exists, and
        //    `run_steps` fetches the parent run's known step keys for
        //    the per-key verification. Returning `stepStats` covering
        //    `step_a` and `step_b` makes the route pass the
        //    verification.
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::body_string_contains("runOrError"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "data": { "runOrError": {
                    "__typename": "Run",
                    "stepStats": [
                        { "stepKey": "step_a", "status": "SUCCESS" },
                        { "stepKey": "step_b", "status": "SUCCESS" },
                    ],
                } }
            })))
            .mount(&server)
            .await;
        // 3. `launchRunReexecution` — `retry_pipeline_run` posts
        //    `launchRunReexecution`, NOT `launchRun` (re-execution is a
        //    separate GraphQL mutation on Dagster's API). The wiremock
        //    exists only to keep the test hermetic; the assertion is
        //    about the absence of the validation
        //    `{"error": "stepKeys ..."}`.
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::body_string_contains("launchRunReexecution"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "data": { "launchRunReexecution": { "__typename": "LaunchRunSuccess", "run": { "runId": "abc-123" } } }
            })))
            .mount(&server)
            .await;
        let state = state_with_dagster(&server.uri());
        let mut args = Map::new();
        args.insert("runId".to_owned(), json!("abc-123"));
        args.insert("stepKeys".to_owned(), json!(["step_a", "step_b"]));
        let body = retry_pipeline_run(&state, None, &args).await;
        // The exact `{"runId": ...}` body shape the existing tool
        // contract produces; this test's only claim is "no
        // `{"error": "stepKeys ..."}` for valid input". Asserting
        // on the absence of the error key (rather than the run id)
        // is the literal negative-control for the test above.
        assert!(
            !body.as_object().is_some_and(|o| {
                o.get("error")
                    .and_then(Value::as_str)
                    .is_some_and(|e| e.contains("stepKeys"))
            }),
            "valid string-array stepKeys must NOT surface as the validation error: got {body}"
        );
    }

    /// F2.9 (PR #59 review, `plans/pipelines/day-1-fixes/
    /// f2-tenant-scope-and-run-config.md` Part D): `trigger_pipeline`
    /// MUST forward `args["runConfig"]` into the `TriggerBody` it
    /// hands to `routes::pipelines::trigger` — that tool → route seam
    /// is the only place the copilot's config can be dropped before
    /// it reaches Dagster. The route-level test
    /// `routes::pipelines::tests::trigger_with_config::
    /// trigger_with_valid_run_config_calls_launch_run_with_config`
    /// pins route → Dagster; this one pins tool → route: the
    /// wiremock's `body_partial_json` matcher on `variables.cfg` only
    /// matches when the caller's config arrives inside the
    /// `launchRun` mutation, so dropping the forwarding line in
    /// [`trigger_pipeline`] leaves the mock unmatched (wiremock
    /// answers 404), the launch errors, and the run-id assertion
    /// below fails.
    #[tokio::test]
    async fn trigger_pipeline_forwards_run_config_to_the_trigger_route() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::body_string_contains(
                "isPipelineConfigValid",
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "data": { "isPipelineConfigValid": {
                    "__typename": "PipelineConfigValidationValid",
                    "pipelineName": "silver_rebuild" } }
            })))
            .mount(&server)
            .await;
        // The forwarding pin: `variables.cfg` must carry the exact
        // config the tool was handed, or this mock never matches.
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::body_string_contains("launchRun"))
            .and(wiremock::matchers::body_partial_json(json!({
                "variables": {
                    "cfg": { "ops": { "silver_rebuild_op": {
                        "config": { "target_table": "x" } } } },
                }
            })))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "data": { "launchRun": { "__typename": "LaunchRunSuccess",
                    "run": { "runId": "r-tool-config" } } }
            })))
            .mount(&server)
            .await;

        let state = state_with_dagster(&server.uri());
        let principal = fixture_user_principal();
        let mut args = Map::new();
        args.insert("id".to_owned(), json!("silver_rebuild"));
        args.insert(
            "runConfig".to_owned(),
            json!({ "ops": { "silver_rebuild_op": { "config": { "target_table": "x" } } } }),
        );
        let body = trigger_pipeline(&state, Some(&principal), &args).await;
        assert_eq!(
            body["id"],
            json!("r-tool-config"),
            "args[\"runConfig\"] must reach the trigger route's launch body unchanged: got {body}",
        );
    }
}
