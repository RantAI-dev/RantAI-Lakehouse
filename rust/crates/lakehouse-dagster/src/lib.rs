//! `Dagster` GraphQL client, porting `src/services/clients/dagster.ts`.
//!
//! Talks to `Dagster`'s GraphQL endpoint directly over HTTP, matching the
//! TypeScript client's hand-rolled `fetch`-based `dg()` helper: no GraphQL
//! codegen, no client library, queries built as string literals.

use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;
use time::OffsetDateTime;

/// A single `Dagster` pipeline run, as returned by `runsOrError`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DgRun {
    /// The run's unique id.
    pub run_id: String,
    /// The job (pipeline) name the run belongs to.
    pub job_name: String,
    /// The run's `Dagster` status string (e.g. `"SUCCESS"`, `"FAILURE"`).
    pub status: String,
    /// Unix seconds the run started, or `None` if it hasn't started yet.
    pub start_time: Option<f64>,
    /// Unix seconds the run ended, or `None` if it hasn't finished yet.
    #[serde(default)]
    pub end_time: Option<f64>,
}

/// One `Dagster` schedule attached to a job, as returned by
/// `repositoriesOrError`. Ported from the `DgJob["schedules"]` element
/// shape in `src/services/clients/dagster.ts`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DgSchedule {
    /// The schedule's own name, needed to target it with
    /// `startSchedule`/`stopRunningSchedule` (Phase 2, Task 2.5) — not read
    /// by anything ported in Phase 1, which only ever displayed
    /// `cronSchedule`/`scheduleState`.
    pub name: String,
    /// The schedule's cron expression (e.g. `"0 3 * * *"`).
    pub cron_schedule: String,
    /// The schedule's run state, e.g. `"RUNNING"`/`"STOPPED"`.
    pub schedule_state: DgScheduleState,
}

/// A `Dagster` schedule's run state.
#[derive(Debug, Clone, Deserialize)]
pub struct DgScheduleState {
    /// The schedule state's status string.
    pub status: String,
}

/// A `Dagster` job (pipeline) with its attached schedules, matching
/// `DgJob` in `src/services/clients/dagster.ts`.
#[derive(Debug, Clone)]
pub struct DgJob {
    /// The job's name.
    pub name: String,
    /// Schedules whose `pipelineName` matches this job's name.
    pub schedules: Vec<DgSchedule>,
}

/// Errors produced while talking to `Dagster`.
#[derive(Debug, Error)]
pub enum DgError {
    /// A transport-level failure (connection refused, TLS error, timeout,
    /// ...) surfaced by `reqwest`.
    ///
    /// The `Display` impl deliberately does NOT include `reqwest`'s message:
    /// `reqwest::Error`'s `Display` appends `" for url (http://host:port/)"`,
    /// which would leak the internal `Dagster` host/port to an
    /// unauthenticated caller. `src/services/clients/dagster.ts` never sees
    /// that URL either: Node's `fetch` (undici) rejects a connection failure
    /// with a `TypeError` whose `.message` is the fixed string `"fetch
    /// failed"` (the underlying cause lives on `.cause`, which the TS route
    /// handlers never read). This variant reproduces that fixed string,
    /// exactly the same treatment `ChError::Transport` got in
    /// `lakehouse-clickhouse` (commit `9114abd`). The `reqwest::Error`
    /// itself is kept as `#[source]` so `tracing` (or any structured
    /// logger) can still record the real cause/URL server-side.
    #[error("fetch failed")]
    Transport(#[source] reqwest::Error),
    /// `Dagster` responded with a non-2xx status or a GraphQL error.
    ///
    /// When the failure is a GraphQL `errors` array, the message is the
    /// `JSON.stringify`-equivalent of that array, truncated to 300
    /// characters — reproducing `dagster.ts`'s
    /// `throw new Error(JSON.stringify(json.errors).slice(0, 300))`
    /// verbatim, including the truncation (a TS quirk kept intentionally:
    /// a long GraphQL error list is silently cut off mid-JSON rather than
    /// shown in full).
    #[error("{0}")]
    Server(String),
}

impl From<reqwest::Error> for DgError {
    fn from(err: reqwest::Error) -> Self {
        Self::Transport(err)
    }
}

/// Truncate `s` to at most 300 `char`s, matching JavaScript's
/// `str.slice(0, 300)` closely enough for the ASCII/JSON-syntax-heavy
/// error payloads `Dagster` returns (JS `slice` counts UTF-16 code units,
/// not `char`s; the two only diverge on astral-plane characters, which
/// don't appear in GraphQL error messages).
fn truncate_300(s: &str) -> String {
    s.chars().take(300).collect()
}

#[derive(Debug, Deserialize)]
struct GqlResponse<T> {
    #[serde(default = "Option::default")]
    data: Option<T>,
    #[serde(default)]
    errors: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct RunsOrErrorData {
    #[serde(rename = "runsOrError")]
    runs_or_error: RunsOrError,
}

#[derive(Debug, Deserialize)]
struct RunsOrError {
    #[serde(default)]
    results: Option<Vec<DgRun>>,
}

#[derive(Debug, Deserialize)]
struct ReposOrErrorData {
    #[serde(rename = "repositoriesOrError")]
    repositories_or_error: ReposOrError,
}

#[derive(Debug, Deserialize)]
struct ReposOrError {
    #[serde(default)]
    nodes: Option<Vec<RepoNode>>,
}

#[derive(Debug, Deserialize)]
struct RepoNode {
    jobs: Vec<JobName>,
    #[serde(default)]
    schedules: Vec<ScheduleNode>,
}

#[derive(Debug, Deserialize)]
struct JobName {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ScheduleNode {
    name: String,
    #[serde(rename = "cronSchedule")]
    cron_schedule: String,
    #[serde(rename = "scheduleState")]
    schedule_state: DgScheduleState,
    #[serde(rename = "jobName")]
    job_name: String,
}

#[derive(Debug, Deserialize)]
struct LaunchRunData {
    #[serde(rename = "launchRun")]
    launch_run: LaunchRunResult,
}

#[derive(Debug, Deserialize)]
struct LaunchRunResult {
    #[serde(rename = "__typename")]
    typename: String,
    #[serde(default)]
    run: Option<LaunchedRun>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    errors: Option<Vec<LaunchRunError>>,
}

#[derive(Debug, Deserialize)]
struct LaunchedRun {
    #[serde(rename = "runId")]
    run_id: String,
}

#[derive(Debug, Deserialize)]
struct LaunchRunError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct TerminateRunData {
    #[serde(rename = "terminateRun")]
    terminate_run: TerminateRunResultBody,
}

#[derive(Debug, Deserialize)]
struct TerminateRunResultBody {
    #[serde(rename = "__typename")]
    typename: String,
    #[serde(default)]
    run: Option<LaunchedRun>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LaunchReexecutionData {
    #[serde(rename = "launchRunReexecution")]
    launch_run_reexecution: LaunchRunResult,
}

#[derive(Debug, Deserialize)]
struct ScheduleMutationResultBody {
    #[serde(rename = "__typename")]
    typename: String,
    #[serde(default)]
    message: Option<String>,
}

/// Outcome of a schedule start/stop mutation.
#[derive(Debug, Clone)]
pub struct ScheduleOutcome {
    /// Whether the schedule was successfully started/stopped.
    pub ok: bool,
    /// A human-readable failure reason, present when `ok` is `false`.
    pub error: Option<String>,
}

/// One `Dagster` execution step's status within a run, as reported by
/// `stepStats`, matching `GET /api/ai/build-status`'s `{key, status}`
/// output shape.
#[derive(Debug, Clone)]
pub struct RunStepStatus {
    /// The step's key (e.g. `"bronze_sdi"`).
    pub key: String,
    /// The step's `Dagster` status string.
    pub status: String,
}

/// A run's overall status plus its per-step statuses, returned by
/// [`DgClient::pipeline_run_status`].
#[derive(Debug, Clone)]
pub struct RunStatusInfo {
    /// The run's overall `Dagster` status string (e.g. `"SUCCESS"`) — NOT
    /// passed through [`map_run_status`]; `GET /api/ai/build-status`
    /// returns the raw `Dagster` string verbatim.
    pub status: String,
    /// Each step's key + status, in `Dagster`'s reported order.
    pub steps: Vec<RunStepStatus>,
    /// Unix seconds the run started, or `None` if `Dagster` hasn't
    /// recorded one yet (WS4 item G1 — the value the API layer reports as
    /// `startedAt` when it can, rather than fabricating `now()`).
    pub start_time: Option<f64>,
}

/// One asset materialization reported by a step, matching
/// `stepStats.materializations`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepMaterialization {
    /// The materialized asset's key, joined with `/` (`assetKey.path` —
    /// `Dagster` represents a key as a path segment list, e.g.
    /// `["bronze", "orders"]`), or `None` when the event carries no key.
    pub asset_key: Option<String>,
    /// Row count, when the op emitted an `IntMetadataEntry` labeled
    /// `"rows"` — `None` when it did not (most ops emit no row count at
    /// all; inventing `0` would claim "measured zero rows" for "not
    /// measured", WS4 item A3).
    pub rows: Option<i64>,
}

/// One `Dagster` execution step's full detail within a run (status, timing,
/// materializations), returned by [`DgClient::run_steps`]. Distinct from
/// [`RunStepStatus`] (used by [`DgClient::pipeline_run_status`]), which
/// carries only `key`/`status` — this type is for the pipeline detail
/// view's step list, which additionally needs timing and per-step output
/// row counts.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStep {
    /// The step's key (e.g. `"run_bronze_maintenance"`).
    pub step_key: String,
    /// The step's `Dagster` status string (e.g. `"SUCCESS"`, `"FAILURE"`),
    /// reported verbatim like [`RunStatusInfo::status`].
    pub status: String,
    /// Unix milliseconds the step started, or `None` if it hasn't started
    /// — `Dagster` reports `startTime` in seconds (possibly fractional);
    /// this is that value times 1000, the millisecond convention the API
    /// layer's timestamp rendering expects.
    pub start_ms: Option<i64>,
    /// Unix milliseconds the step ended, or `None` if it hasn't.
    pub end_ms: Option<i64>,
    /// Assets this step materialized, in `Dagster`'s reported order.
    pub materializations: Vec<StepMaterialization>,
}

/// One log line, from a `MessageEvent`-implementing event in a run's log
/// stream, returned by [`DgClient::run_logs`].
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    /// Unix milliseconds the event was recorded — `Dagster` reports
    /// `timestamp` as a STRING of milliseconds (`"1789668190285"`, not
    /// seconds like `startTime`/`endTime`), parsed here.
    pub ts: f64,
    /// The event's log level (e.g. `"INFO"`, `"DEBUG"`, `"ERROR"`).
    pub level: String,
    /// The step this event belongs to, or `None` for a run-level event
    /// (e.g. `RunEnqueuedEvent`, `RunFailureEvent`).
    pub step_key: Option<String>,
    /// The event's message — may be an empty string for structural events
    /// that still implement `MessageEvent` (e.g. `RunEnqueuedEvent`).
    pub message: String,
}

/// One page of a run's log stream, returned by [`DgClient::run_logs`].
#[derive(Debug, Clone)]
pub struct RunLogsPage {
    /// Parsed `MessageEvent` lines, in `Dagster`'s reported order.
    pub lines: Vec<LogLine>,
    /// Opaque pagination cursor — pass as `after_cursor` to fetch the next
    /// page.
    pub cursor: String,
    /// Whether more events exist after this page.
    pub has_more: bool,
}

/// Outcome of [`DgClient::launch_run`], mirroring the TypeScript's
/// `{ runId?: string; error?: string }` return shape (never a thrown
/// error for a well-formed GraphQL response — failures are reported in
/// the `error` field instead).
#[derive(Debug, Clone)]
pub struct LaunchOutcome {
    /// The new run's id, present on success.
    pub run_id: Option<String>,
    /// A human-readable failure reason, present on failure.
    pub error: Option<String>,
}

/// Shared decode/branch tail for [`DgClient::launch_run`],
/// [`DgClient::launch_run_with_config`], and
/// [`DgClient::launch_reexecution`] — all three mutations decode the same
/// `LaunchRunResult` shape and apply the identical
/// `LaunchRunSuccess`/`PythonError`/`RunConfigValidationInvalid` branching
/// (WS3 plan review Z9 added the second call site, which is what pushed
/// this from two duplicated call sites to three — generalized here rather
/// than duplicated a third time).
fn launch_outcome_from(r: LaunchRunResult) -> LaunchOutcome {
    if r.typename == "LaunchRunSuccess"
        && let Some(run) = r.run
    {
        return LaunchOutcome {
            run_id: Some(run.run_id),
            error: None,
        };
    }
    let error = r
        .message
        .or_else(|| {
            r.errors.map(|errs| {
                errs.into_iter()
                    .map(|e| e.message)
                    .collect::<Vec<_>>()
                    .join("; ")
            })
        })
        .unwrap_or(r.typename);
    LaunchOutcome {
        run_id: None,
        error: Some(error),
    }
}

/// HTTP client for `Dagster`'s GraphQL endpoint.
pub struct DgClient {
    client: Client,
    url: String,
    /// Repository name used to target `launchRun`. Default
    /// `"__repository__"` (`dagster.ts:7`).
    repo: String,
    /// Repository location used to target `launchRun`. Default
    /// `"dispar_orchestrate.definitions"` (`dagster.ts:8`).
    location: String,
}

impl DgClient {
    /// Build a client targeting `url` (the `Dagster` GraphQL endpoint),
    /// with the default repository/location the TypeScript client falls
    /// back to when `DAGSTER_REPO`/`DAGSTER_LOCATION` are unset.
    #[must_use]
    pub fn new(url: String) -> Self {
        Self::with_repository(
            url,
            "__repository__".to_owned(),
            "dispar_orchestrate.definitions".to_owned(),
        )
    }

    /// Build a client targeting `url`, with an explicit repository name
    /// and location — used by [`DgClient::launch_run`]'s selector.
    #[must_use]
    pub fn with_repository(url: String, repo: String, location: String) -> Self {
        Self {
            client: Client::new(),
            url,
            repo,
            location,
        }
    }

    /// List up to `limit` runs, most recent first, matching
    /// `listRuns(undefined, limit)` in the TypeScript client (every caller
    /// among the ported routes omits the `jobName` filter).
    ///
    /// # Errors
    ///
    /// Returns [`DgError::Transport`] on a network-level failure, or
    /// [`DgError::Server`] when `Dagster` responds with a non-2xx status or
    /// a GraphQL `errors` array.
    pub async fn list_runs(&self, limit: u32) -> Result<Vec<DgRun>, DgError> {
        let query = format!(
            "{{ runsOrError(limit: {limit}) {{ __typename ... on Runs {{ results {{ \
             runId jobName status startTime endTime }} }} }} }}"
        );
        let data: RunsOrErrorData = self.execute(&query, None).await?;
        Ok(data.runs_or_error.results.unwrap_or_default())
    }

    /// List job names in the default repository, matching `listJobs()` in
    /// the TypeScript client (which additionally attaches schedules — not
    /// needed by any ported route, since every caller discards the
    /// result).
    ///
    /// # Errors
    ///
    /// See [`DgClient::list_runs`].
    pub async fn list_jobs(&self) -> Result<Vec<String>, DgError> {
        let query = "{ repositoriesOrError { __typename ... on RepositoryConnection { nodes { \
                      jobs { name } } } } }";
        let data: ReposOrErrorData = self.execute(query, None).await?;
        let Some(node) = data
            .repositories_or_error
            .nodes
            .and_then(|n| n.into_iter().next())
        else {
            return Ok(Vec::new());
        };
        Ok(node
            .jobs
            .into_iter()
            .map(|j| j.name)
            .filter(|n| !n.starts_with("__"))
            .collect())
    }

    /// List up to `limit` runs belonging to `job_name`, most recent first
    /// — matching `listRuns(jobName, limit)` in the TypeScript client when
    /// `jobName` is given, used by `GET /api/pipelines/{id}/runs`.
    ///
    /// # Errors
    ///
    /// See [`DgClient::list_runs`].
    pub async fn list_runs_for_job(
        &self,
        job_name: &str,
        limit: u32,
    ) -> Result<Vec<DgRun>, DgError> {
        // `filter: { pipelineName: "<job_name>" }` — the TypeScript
        // interpolates `jobName` into the query string unescaped
        // (`dagster.ts`); reproduced verbatim rather than parameterized,
        // since `job_name` here is always a path segment already resolved
        // against known job names by the caller.
        let query = format!(
            "{{ runsOrError(filter: {{ pipelineName: \"{job_name}\" }}, limit: {limit}) {{ \
             __typename ... on Runs {{ results {{ runId jobName status startTime endTime }} }} \
             }} }}"
        );
        let data: RunsOrErrorData = self.execute(&query, None).await?;
        Ok(data.runs_or_error.results.unwrap_or_default())
    }

    /// List jobs in the default repository together with each job's
    /// attached schedules, matching `listJobs()` in the TypeScript client.
    ///
    /// # Errors
    ///
    /// See [`DgClient::list_runs`].
    pub async fn list_jobs_with_schedules(&self) -> Result<Vec<DgJob>, DgError> {
        let query = "{ repositoriesOrError { __typename ... on RepositoryConnection { nodes { \
                      jobs { name } \
                      schedules { name cronSchedule scheduleState { status } jobName: pipelineName } \
                      } } } }";
        let data: ReposOrErrorData = self.execute(query, None).await?;
        let Some(node) = data
            .repositories_or_error
            .nodes
            .and_then(|n| n.into_iter().next())
        else {
            return Ok(Vec::new());
        };
        Ok(node
            .jobs
            .into_iter()
            .filter(|j| !j.name.starts_with("__"))
            .map(|j| {
                let schedules = node
                    .schedules
                    .iter()
                    .filter(|s| s.job_name == j.name)
                    .map(|s| DgSchedule {
                        name: s.name.clone(),
                        cron_schedule: s.cron_schedule.clone(),
                        schedule_state: DgScheduleState {
                            status: s.schedule_state.status.clone(),
                        },
                    })
                    .collect();
                DgJob {
                    name: j.name,
                    schedules,
                }
            })
            .collect())
    }

    /// Fetch `job_name`'s op graph (nodes + dependency edges), matching
    /// `pipelineOrError { __typename ... on Pipeline { solidHandles { solid
    /// { name definition { description } inputs { dependsOn { solid { name
    /// } } } } } } }` (WS4 item A1 — verified live against this
    /// repository's own Dagster `1.13.20` stack; see
    /// `tests/fixtures/job_graph_bronze_maintenance.json`).
    ///
    /// # Errors
    ///
    /// Returns [`DgError::Transport`] on a network-level failure, or
    /// [`DgError::Server`] when `Dagster` responds with a non-2xx status,
    /// a GraphQL `errors` array, or `pipelineOrError.__typename` is not
    /// `Pipeline` (e.g. `PipelineNotFoundError` — the job doesn't exist in
    /// this repository/location).
    pub async fn job_graph(&self, job_name: &str) -> Result<JobGraph, DgError> {
        let query = "query($sel: PipelineSelector!) { pipelineOrError(params: $sel) { \
                      __typename ... on Pipeline { solidHandles { solid { name \
                      definition { description } inputs { dependsOn { solid { name } } } } } } \
                      } }";
        let variables = json!({ "sel": {
            "pipelineName": job_name,
            "repositoryName": self.repo,
            "repositoryLocationName": self.location,
        }});
        let data: PipelineOrErrorData = self.execute(query, Some(variables)).await?;
        let PipelineOrError::Pipeline { solid_handles } = data.pipeline_or_error else {
            return Err(DgError::Server(format!("job {job_name} not found")));
        };
        let ops = solid_handles
            .iter()
            .map(|h| GraphOp {
                name: h.solid.name.clone(),
                description: h.solid.definition.description.clone(),
            })
            .collect();
        let edges = solid_handles
            .iter()
            .flat_map(|h| {
                h.solid.inputs.iter().flat_map(move |input| {
                    input.depends_on.iter().map(move |d| GraphEdge {
                        from: d.solid.name.clone(),
                        to: h.solid.name.clone(),
                    })
                })
            })
            .collect();
        Ok(JobGraph { ops, edges })
    }

    /// Launch a run of `job_name`, matching `launchRun(jobName)` in the
    /// TypeScript client.
    ///
    /// Unlike [`DgClient::list_runs`]/[`DgClient::list_jobs`], a
    /// GraphQL-level failure here does NOT become an `Err` — the mutation
    /// itself can return a typed failure (`PythonError`,
    /// `RunConfigValidationInvalid`) inside a normal `200` GraphQL
    /// response, and `launchRun` in the TypeScript reports that as
    /// `{ error: ... }` rather than throwing, exactly as reproduced here.
    /// `Err` is still returned for transport failures and for the
    /// `json.errors` case (a malformed GraphQL request itself).
    ///
    /// # Errors
    ///
    /// Returns [`DgError::Transport`] on a network-level failure, or
    /// [`DgError::Server`] when `Dagster` responds with a non-2xx status or
    /// a GraphQL `errors` array.
    pub async fn launch_run(&self, job_name: &str) -> Result<LaunchOutcome, DgError> {
        let query = "mutation($sel: JobOrPipelineSelector!) { \
                      launchRun(executionParams: { selector: $sel, mode: \"default\" }) { \
                      __typename \
                      ... on LaunchRunSuccess { run { runId } } \
                      ... on PythonError { message } \
                      ... on RunConfigValidationInvalid { errors { message } } \
                      } }";
        let variables = json!({
            "sel": {
                "repositoryName": self.repo,
                "repositoryLocationName": self.location,
                "pipelineName": job_name,
            }
        });
        let data: LaunchRunData = self.execute(query, Some(variables)).await?;
        Ok(launch_outcome_from(data.launch_run))
    }

    /// Like [`DgClient::launch_run`], but with `runConfigData` set —
    /// `Dagster`'s GraphQL `ExecutionParams.runConfigData` field takes a
    /// JSON STRING (not a nested object), so `run_config` is serialized to
    /// a string once here. Added for WS3's `ingest_job` (one static job,
    /// per-connector `connector_id` config, WS3 plan review Z9) —
    /// [`DgClient::launch_run`] itself is UNCHANGED so every existing
    /// caller (`routes::pipelines::trigger`) keeps its current,
    /// config-free launch; this is a second, additive method, not a
    /// signature change to the first.
    ///
    /// # Errors
    ///
    /// Same as [`DgClient::launch_run`]: a GraphQL-level launch failure
    /// (a bad job name, a `RunConfigValidationInvalid`) comes back as
    /// `Ok(LaunchOutcome { run_id: None, error: Some(msg) })`, NOT `Err`
    /// — only a transport failure or a malformed GraphQL response is
    /// [`DgError::Transport`]/[`DgError::Server`].
    pub async fn launch_run_with_config(
        &self,
        job_name: &str,
        run_config: &Value,
    ) -> Result<LaunchOutcome, DgError> {
        let query = "mutation($sel: JobOrPipelineSelector!, $cfg: String!) { \
                      launchRun(executionParams: { selector: $sel, mode: \"default\", \
                      runConfigData: $cfg }) { \
                      __typename \
                      ... on LaunchRunSuccess { run { runId } } \
                      ... on PythonError { message } \
                      ... on RunConfigValidationInvalid { errors { message } } \
                      } }";
        let variables = json!({
            "sel": {
                "repositoryName": self.repo,
                "repositoryLocationName": self.location,
                "pipelineName": job_name,
            },
            "cfg": run_config.to_string(),
        });
        let data: LaunchRunData = self.execute(query, Some(variables)).await?;
        Ok(launch_outcome_from(data.launch_run))
    }

    /// Terminate a running run, matching `mutation { terminateRun(runId:
    /// ...) }`. Used by `cancelRun` (Phase 2, Task 2.5). Uses
    /// `SAFE_TERMINATE` (the default `terminatePolicy`) rather than
    /// `MARK_AS_CANCELED_IMMEDIATELY`: it lets `Dagster` shut the run down
    /// cleanly instead of abandoning it mid-step, matching what an
    /// operator clicking "cancel" in the `Dagster` UI gets by default.
    ///
    /// Like [`DgClient::launch_run`], a typed `Dagster`-side failure
    /// (`RunNotFoundError`, `TerminateRunFailure`, ...) is reported via
    /// `Ok(LaunchOutcome { error: Some(..), .. })`, not `Err` — `Err` is
    /// reserved for transport failures and malformed GraphQL responses.
    ///
    /// # Errors
    ///
    /// See [`DgClient::launch_run`].
    pub async fn terminate_run(&self, run_id: &str) -> Result<LaunchOutcome, DgError> {
        let query = "mutation($runId: String!) { \
                      terminateRun(runId: $runId, terminatePolicy: SAFE_TERMINATE) { \
                      __typename \
                      ... on TerminateRunSuccess { run { runId } } \
                      ... on TerminateRunFailure { message } \
                      ... on PythonError { message } \
                      } }";
        let data: TerminateRunData = self
            .execute(query, Some(json!({ "runId": run_id })))
            .await?;
        let r = data.terminate_run;
        if r.typename == "TerminateRunSuccess"
            && let Some(run) = r.run
        {
            return Ok(LaunchOutcome {
                run_id: Some(run.run_id),
                error: None,
            });
        }
        Ok(LaunchOutcome {
            run_id: None,
            error: Some(r.message.unwrap_or(r.typename)),
        })
    }

    /// Re-execute a finished (failed/cancelled) run from the start, matching
    /// `mutation { launchRunReexecution(reexecutionParams: { parentRunId,
    /// strategy: ALL_STEPS }) }`. Used by `retryRun` (Phase 2, Task 2.5).
    /// `ALL_STEPS`, not `FROM_FAILURE`: the mock's `retryRun` restarts the
    /// whole run (`processed`/`accepted`/... reset to 0), which
    /// `ALL_STEPS` is the closer match for.
    ///
    /// # Errors
    ///
    /// See [`DgClient::launch_run`].
    pub async fn launch_reexecution(&self, parent_run_id: &str) -> Result<LaunchOutcome, DgError> {
        let query = "mutation($parentRunId: String!) { \
                      launchRunReexecution(reexecutionParams: { parentRunId: $parentRunId, \
                      strategy: ALL_STEPS }) { \
                      __typename \
                      ... on LaunchRunSuccess { run { runId } } \
                      ... on PythonError { message } \
                      ... on RunConfigValidationInvalid { errors { message } } \
                      } }";
        let data: LaunchReexecutionData = self
            .execute(query, Some(json!({ "parentRunId": parent_run_id })))
            .await?;
        Ok(launch_outcome_from(data.launch_run_reexecution))
    }

    /// Start (unpause) a schedule, matching `mutation { startSchedule(...) }`.
    /// Used by `resumePipeline` (Phase 2, Task 2.5).
    ///
    /// # Errors
    ///
    /// See [`DgClient::launch_run`].
    pub async fn start_schedule(&self, schedule_name: &str) -> Result<ScheduleOutcome, DgError> {
        self.schedule_mutation("startSchedule", schedule_name).await
    }

    /// Stop (pause) a running schedule, matching `mutation {
    /// stopRunningSchedule(...) }`. Used by `pausePipeline` (Phase 2, Task
    /// 2.5).
    ///
    /// # Errors
    ///
    /// See [`DgClient::launch_run`].
    pub async fn stop_schedule(&self, schedule_name: &str) -> Result<ScheduleOutcome, DgError> {
        self.schedule_mutation("stopRunningSchedule", schedule_name)
            .await
    }

    async fn schedule_mutation(
        &self,
        mutation: &str,
        schedule_name: &str,
    ) -> Result<ScheduleOutcome, DgError> {
        let query = format!(
            "mutation($sel: ScheduleSelector!) {{ {mutation}(scheduleSelector: $sel) {{ \
             __typename \
             ... on ScheduleStateResult {{ scheduleState {{ status }} }} \
             ... on ScheduleNotFoundError {{ message }} \
             ... on PythonError {{ message }} \
             ... on UnauthorizedError {{ message }} \
             }} }}"
        );
        let variables = json!({
            "sel": {
                "repositoryName": self.repo,
                "repositoryLocationName": self.location,
                "scheduleName": schedule_name,
            }
        });
        let data: std::collections::HashMap<String, ScheduleMutationResultBody> =
            self.execute(&query, Some(variables)).await?;
        let r = data.into_values().next().ok_or_else(|| {
            DgError::Server("Dagster response missing schedule mutation result".to_owned())
        })?;
        if r.typename == "ScheduleStateResult" {
            return Ok(ScheduleOutcome {
                ok: true,
                error: None,
            });
        }
        Ok(ScheduleOutcome {
            ok: false,
            error: Some(r.message.unwrap_or(r.typename)),
        })
    }

    /// A single run's live status + per-step status, matching
    /// `GET /api/ai/build-status`'s inline query (`pipelineRunOrError` on
    /// `Run`).
    ///
    /// Unlike [`DgClient::list_runs`]/[`DgClient::list_jobs`], this does
    /// NOT treat a GraphQL `errors` array or a non-`"Run"` `__typename`
    /// (e.g. `RunNotFoundError`) as an [`Err`] — it returns `Ok(None)` for
    /// both, matching the TypeScript route's own hand-rolled `fetch`,
    /// which never inspects `json.errors` and simply falls through to its
    /// "not found" branch when `json?.data?.pipelineRunOrError` is
    /// `undefined` or not a `Run`. `Err` is reserved for a transport-level
    /// failure or a response body that isn't valid `JSON` at all.
    ///
    /// # Errors
    ///
    /// Returns [`DgError::Transport`] on a network-level failure, or
    /// [`DgError::Server`] when the response body isn't valid `JSON`.
    pub async fn pipeline_run_status(
        &self,
        run_id: &str,
    ) -> Result<Option<RunStatusInfo>, DgError> {
        let query = "query($rid:ID!){ pipelineRunOrError(runId:$rid){ __typename \
                      ... on Run { status startTime stepStats { stepKey status } } } }";
        let body = json!({ "query": query, "variables": { "rid": run_id } });
        let resp = self.client.post(&self.url).json(&body).send().await?;
        let text = resp.text().await?;
        let parsed: Value =
            serde_json::from_str(&text).map_err(|e| DgError::Server(e.to_string()))?;
        let run = parsed.pointer("/data/pipelineRunOrError");
        let Some(run) = run else {
            return Ok(None);
        };
        if run.get("__typename").and_then(Value::as_str) != Some("Run") {
            return Ok(None);
        }
        let status = run
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let start_time = run.get("startTime").and_then(Value::as_f64);
        let steps = run
            .get("stepStats")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .map(|s| RunStepStatus {
                        key: s
                            .get("stepKey")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                        status: s
                            .get("status")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Some(RunStatusInfo {
            status,
            steps,
            start_time,
        }))
    }

    /// Fetch `run_id`'s per-step status, timing, and materializations,
    /// matching `runOrError { __typename ... on Run { stepStats { stepKey
    /// status startTime endTime materializations { assetKey { path }
    /// metadataEntries { __typename ... on IntMetadataEntry { label
    /// intValue } } } } } }` (WS4 item A1 — verified live against this
    /// repository's own Dagster `1.13.20` stack; see
    /// `tests/fixtures/run_steps_captured_fixture.json`). Unlike
    /// [`DgClient::pipeline_run_status`] (Phase 1, returns only
    /// `status`/`key` per step), this also parses each materialization's
    /// `rows` metadata entry when present.
    ///
    /// A run that doesn't exist (`__typename` other than `"Run"`, e.g. the
    /// real `RunNotFoundError` shape this stack returns for a bogus run
    /// id) is a normal "nothing to show" case here — `Ok(Vec::new())`, not
    /// an `Err` — matching [`DgClient::pipeline_run_status`]'s posture for
    /// the same condition.
    ///
    /// # Errors
    ///
    /// Returns [`DgError::Transport`] on a network-level failure, or
    /// [`DgError::Server`] when the response body isn't valid `JSON`.
    pub async fn run_steps(&self, run_id: &str) -> Result<Vec<RunStep>, DgError> {
        let query = "query($rid:ID!){ runOrError(runId:$rid){ __typename \
                      ... on Run { stepStats { stepKey status startTime endTime \
                      materializations { assetKey { path } metadataEntries { __typename \
                      ... on IntMetadataEntry { label intValue } } } } } } }";
        let body = json!({ "query": query, "variables": { "rid": run_id } });
        let resp = self.client.post(&self.url).json(&body).send().await?;
        let text = resp.text().await?;
        let parsed: Value =
            serde_json::from_str(&text).map_err(|e| DgError::Server(e.to_string()))?;
        let run = parsed.pointer("/data/runOrError");
        let Some(run) = run else {
            return Ok(Vec::new());
        };
        if run.get("__typename").and_then(Value::as_str) != Some("Run") {
            return Ok(Vec::new());
        }
        let steps = run
            .get("stepStats")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().map(run_step_from_value).collect())
            .unwrap_or_default();
        Ok(steps)
    }

    /// Fetch up to `limit` log lines for `run_id`, matching `logsForRun(runId,
    /// afterCursor, limit) { __typename ... on EventConnection { events {
    /// __typename ... on MessageEvent { message timestamp level stepKey }
    /// } cursor hasMore } }` (WS4 item A1 — verified live against this
    /// repository's own Dagster `1.13.20` stack; see
    /// `tests/fixtures/run_logs_captured_fixture.json`). `logsForRun` is a
    /// TOP-LEVEL query field, not nested under `Run`. Events in the union
    /// that are not `MessageEvent` (rendered as `{"__typename": ...}` with
    /// no `message`/`level`/`timestamp` under the `... on MessageEvent`
    /// fragment) are silently skipped: a log viewer shows messages, not
    /// every internal `Dagster` event type.
    ///
    /// Unlike [`DgClient::run_steps`], a run that doesn't exist here IS an
    /// [`Err`] — the route maps it to a `404` (Phase C), a different
    /// condition than "this run exists but has no log lines yet".
    ///
    /// # Errors
    ///
    /// Returns [`DgError::Transport`] on a network-level failure, or
    /// [`DgError::Server`] when the response body isn't valid `JSON` or
    /// `logsForRun.__typename` isn't `EventConnection` (`RunNotFoundError`,
    /// `PythonError`).
    pub async fn run_logs(
        &self,
        run_id: &str,
        after_cursor: Option<&str>,
        limit: u32,
    ) -> Result<RunLogsPage, DgError> {
        let query = "query($rid:ID!,$after:String,$limit:Int!){ logsForRun(runId:$rid, \
                      afterCursor:$after, limit:$limit){ __typename ... on EventConnection { \
                      events { __typename ... on MessageEvent { message timestamp level \
                      stepKey } } cursor hasMore } ... on RunNotFoundError { message } \
                      ... on PythonError { message } } }";
        let variables = json!({ "rid": run_id, "after": after_cursor, "limit": limit });
        let body = json!({ "query": query, "variables": variables });
        let resp = self.client.post(&self.url).json(&body).send().await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(DgError::Server(format!("Dagster HTTP {}", status.as_u16())));
        }
        let parsed: Value =
            serde_json::from_str(&text).map_err(|e| DgError::Server(e.to_string()))?;
        if let Some(errors) = parsed.get("errors") {
            return Err(DgError::Server(truncate_300(&errors.to_string())));
        }
        let conn = parsed
            .pointer("/data/logsForRun")
            .ok_or_else(|| DgError::Server("Dagster response missing logsForRun".to_owned()))?;
        if conn.get("__typename").and_then(Value::as_str) != Some("EventConnection") {
            let message = conn
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Dagster run not found")
                .to_owned();
            return Err(DgError::Server(message));
        }
        let lines = conn
            .get("events")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(log_line_from_value).collect())
            .unwrap_or_default();
        let cursor = conn
            .get("cursor")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let has_more = conn
            .get("hasMore")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Ok(RunLogsPage {
            lines,
            cursor,
            has_more,
        })
    }

    /// Whether the `Dagster` GraphQL endpoint is reachable, checked via its
    /// `/server_info` REST endpoint with a 3-second timeout — matching the
    /// `check("dagster", dagUrl)` helper in
    /// `src/app/api/ops/[kind]/route.ts`.
    pub async fn is_alive(&self) -> bool {
        let server_info_url = self.url.replace("/graphql", "/server_info");
        let Ok(resp) = self
            .client
            .get(&server_info_url)
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await
        else {
            return false;
        };
        resp.status().is_success()
    }

    async fn execute<T: for<'de> Deserialize<'de>>(
        &self,
        query: &str,
        variables: Option<Value>,
    ) -> Result<T, DgError> {
        let body = match variables {
            Some(v) => json!({ "query": query, "variables": v }),
            None => json!({ "query": query }),
        };
        let resp = self.client.post(&self.url).json(&body).send().await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(DgError::Server(format!("Dagster HTTP {}", status.as_u16())));
        }
        let parsed: GqlResponse<T> =
            serde_json::from_str(&text).map_err(|e| DgError::Server(e.to_string()))?;
        if let Some(errors) = parsed.errors {
            return Err(DgError::Server(truncate_300(&errors.to_string())));
        }
        parsed
            .data
            .ok_or_else(|| DgError::Server("Dagster response missing data".to_owned()))
    }
}

/// One op node in a job's dependency graph, as returned by
/// `pipelineOrError { ... on Pipeline { solidHandles { solid { ... } } } }`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphOp {
    /// The op's name (e.g. `"run_bronze_maintenance"`).
    pub name: String,
    /// The op's docstring, when it has one — `solid.definition.description`
    /// is itself nullable on `Dagster`'s side.
    pub description: Option<String>,
}

/// One dependency edge (`from` runs before `to`), derived from
/// `Input.dependsOn.solid.name` — `Dagster`'s own graph representation is
/// input-to-upstream-output, so this client inverts it once here rather
/// than making every caller do so.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphEdge {
    /// The upstream op's name.
    pub from: String,
    /// The downstream op's name.
    pub to: String,
}

/// A job's op graph: nodes plus dependency edges, returned by
/// [`DgClient::job_graph`].
#[derive(Debug, Clone)]
pub struct JobGraph {
    /// Every op in the job, in the order `Dagster` reported them.
    pub ops: Vec<GraphOp>,
    /// Dependency edges derived from each op's inputs.
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Deserialize)]
struct PipelineOrErrorData {
    #[serde(rename = "pipelineOrError")]
    pipeline_or_error: PipelineOrError,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "__typename")]
enum PipelineOrError {
    Pipeline {
        #[serde(rename = "solidHandles")]
        solid_handles: Vec<SolidHandleNode>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct SolidHandleNode {
    solid: SolidNode,
}

#[derive(Debug, Deserialize)]
struct SolidNode {
    name: String,
    definition: SolidDefinitionNode,
    #[serde(default)]
    inputs: Vec<InputNode>,
}

#[derive(Debug, Deserialize)]
struct SolidDefinitionNode {
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InputNode {
    #[serde(rename = "dependsOn")]
    depends_on: Vec<DependsOnNode>,
}

#[derive(Debug, Deserialize)]
struct DependsOnNode {
    solid: DependsOnSolid,
}

#[derive(Debug, Deserialize)]
struct DependsOnSolid {
    name: String,
}

/// Convert one `stepStats` array element into a [`RunStep`] — split out
/// from [`DgClient::run_steps`] so the per-step parse (itself several
/// nested `Option` chains) reads as one function rather than a closure
/// buried in a `.map()`.
fn run_step_from_value(s: &Value) -> RunStep {
    let materializations = s
        .get("materializations")
        .and_then(Value::as_array)
        .map(|mats| {
            mats.iter()
                .map(|m| StepMaterialization {
                    asset_key: asset_key_from(m),
                    rows: m
                        .get("metadataEntries")
                        .and_then(rows_from_metadata_entries),
                })
                .collect()
        })
        .unwrap_or_default();
    RunStep {
        step_key: s
            .get("stepKey")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        status: s
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        start_ms: seconds_to_ms(s.get("startTime")),
        end_ms: seconds_to_ms(s.get("endTime")),
        materializations,
    }
}

/// The `"rows"`-labeled `IntMetadataEntry`'s value, when the
/// materialization's `metadataEntries` union carries one — `Dagster`'s
/// `metadataEntries` is a heterogeneous GraphQL union (`IntMetadataEntry`,
/// `TextMetadataEntry`, ...) that `serde` cannot untag safely without a
/// blanket `#[serde(other)]` swallowing real variants, so this is read via
/// raw `Value` navigation, the same approach
/// [`DgClient::pipeline_run_status`] already uses for `stepStats`.
fn rows_from_metadata_entries(entries: &Value) -> Option<i64> {
    entries.as_array()?.iter().find_map(|e| {
        if e.get("__typename").and_then(Value::as_str) != Some("IntMetadataEntry") {
            return None;
        }
        if e.get("label").and_then(Value::as_str) != Some("rows") {
            return None;
        }
        e.get("intValue").and_then(Value::as_i64)
    })
}

/// Join `assetKey.path` (`Dagster`'s asset key is a path segment list, e.g.
/// `["bronze", "orders"]`) with `/`, or `None` when the materialization
/// carries no asset key at all.
fn asset_key_from(mat: &Value) -> Option<String> {
    let segments = mat
        .get("assetKey")?
        .get("path")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    if segments.is_empty() {
        None
    } else {
        Some(segments.join("/"))
    }
}

/// Convert a `Dagster` `startTime`/`endTime` value (Unix seconds, possibly
/// fractional) into Unix milliseconds — `None` when `Dagster` hasn't
/// recorded the timestamp yet (WS4 item G1's "never fabricate a timestamp"
/// posture, reused here for step-level timing).
#[allow(
    clippy::cast_possible_truncation,
    reason = "millisecond-precision display timestamp; Dagster's own \
              timestamps never approach i64::MAX seconds"
)]
fn seconds_to_ms(v: Option<&Value>) -> Option<i64> {
    v.and_then(Value::as_f64)
        .map(|secs| (secs * 1000.0).round() as i64)
}

/// Convert one `logsForRun.events` array element into a [`LogLine`], or
/// `None` when the event doesn't carry the `MessageEvent` fields at all —
/// `Dagster`'s `metadataEntries`-style unions omit fragment fields
/// entirely for a `__typename` that doesn't implement the fragment's
/// interface, rather than nulling them, so a missing `message` is this
/// function's signal to skip the event (WS4 item A4: "the messages it
/// could read", never a hard parse failure over one unrecognized event).
fn log_line_from_value(e: &Value) -> Option<LogLine> {
    let message = e.get("message").and_then(Value::as_str)?.to_owned();
    let level = e.get("level").and_then(Value::as_str)?.to_owned();
    // `Dagster` reports `timestamp` as a STRING of milliseconds, unlike
    // `startTime`/`endTime`'s numeric seconds elsewhere in this client.
    let ts = e
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<f64>().ok())?;
    let step_key = e.get("stepKey").and_then(Value::as_str).map(str::to_owned);
    Some(LogLine {
        ts,
        level,
        step_key,
        message,
    })
}

/// `Dagster` run status → console `EntityStatus`, porting `mapRunStatus` in
/// `src/services/clients/dagster.ts`.
#[must_use]
pub fn map_run_status(status: &str) -> &'static str {
    match status {
        "SUCCESS" => "completed",
        "FAILURE" => "failed",
        "CANCELED" | "CANCELING" => "cancelled",
        "QUEUED" | "NOT_STARTED" => "queued",
        "STARTED" | "STARTING" | "MANAGED" => "running",
        _ => "unknown",
    }
}

/// Render a `Dagster` run's `startTime` (Unix seconds, possibly
/// fractional) as an ISO-8601 UTC timestamp with millisecond precision,
/// matching JavaScript's `new Date(startTime * 1000).toISOString()`.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "millisecond-precision display timestamp; sub-millisecond and \
              beyond-i128-range loss is inconsequential here"
)]
pub fn iso_from_unix_seconds(seconds: f64) -> String {
    let nanos = (seconds * 1_000_000_000.0).round() as i128;
    let dt = OffsetDateTime::from_unix_timestamp_nanos(nanos).unwrap_or(OffsetDateTime::UNIX_EPOCH);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        dt.year(),
        u8::from(dt.month()),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second(),
        dt.millisecond()
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[test]
    fn map_run_status_matches_ts_switch() {
        assert_eq!(map_run_status("SUCCESS"), "completed");
        assert_eq!(map_run_status("FAILURE"), "failed");
        assert_eq!(map_run_status("CANCELED"), "cancelled");
        assert_eq!(map_run_status("CANCELING"), "cancelled");
        assert_eq!(map_run_status("QUEUED"), "queued");
        assert_eq!(map_run_status("NOT_STARTED"), "queued");
        assert_eq!(map_run_status("STARTED"), "running");
        assert_eq!(map_run_status("STARTING"), "running");
        assert_eq!(map_run_status("MANAGED"), "running");
        assert_eq!(map_run_status("SOMETHING_ELSE"), "unknown");
    }

    #[test]
    fn iso_from_unix_seconds_matches_js_to_iso_string() {
        let seconds = 1_787_803_210.075_f64;
        assert_eq!(iso_from_unix_seconds(seconds), "2026-08-27T04:00:10.075Z");
    }

    #[test]
    fn iso_from_unix_seconds_zero_is_epoch() {
        assert_eq!(iso_from_unix_seconds(0.0), "1970-01-01T00:00:00.000Z");
    }

    #[tokio::test]
    async fn list_runs_parses_results() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "runsOrError": { "__typename": "Runs", "results": [
                    { "runId": "r1", "jobName": "refresh_lakehouse", "status": "SUCCESS",
                      "startTime": 1.0, "endTime": 2.0 }
                ] } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let runs = client.list_runs(25).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, "r1");
        assert_eq!(runs[0].status, "SUCCESS");
    }

    #[tokio::test]
    async fn list_jobs_filters_dunder_and_maps_names() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "repositoriesOrError": { "__typename": "RepositoryConnection", "nodes": [
                    { "jobs": [ { "name": "refresh_lakehouse" }, { "name": "__ASSET_JOB" } ] }
                ] } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let jobs = client.list_jobs().await.unwrap();
        assert_eq!(jobs, vec!["refresh_lakehouse".to_owned()]);
    }

    #[tokio::test]
    async fn graphql_errors_are_json_stringified_and_truncated_to_300_chars() {
        // Reproduces dagster.ts: `throw new Error(JSON.stringify(json.errors).slice(0, 300))`.
        let long_message = "x".repeat(500);
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "errors": [ { "message": long_message } ]
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let err = client.list_runs(25).await.unwrap_err();
        let DgError::Server(msg) = err else {
            panic!("expected Server error");
        };
        assert_eq!(msg.chars().count(), 300);
        let expected_full = json!([ { "message": "x".repeat(500) } ]).to_string();
        assert_eq!(msg, truncate_300(&expected_full));
    }

    /// Regression test for B2 (transport errors leaking the internal
    /// `Dagster` endpoint): connecting to a closed port must render as the
    /// fixed `"fetch failed"` string, matching Node `fetch`'s
    /// `TypeError.message`, never `reqwest`'s host/port-bearing message.
    #[tokio::test]
    async fn transport_error_display_does_not_leak_host_or_port() {
        // Bind to an ephemeral port, then drop the listener immediately so
        // nothing is listening there: connecting to it is guaranteed to be
        // refused, producing a genuine `reqwest::Error` without relying on
        // any specific closed port being free on the test host.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let dead_url = format!("http://{addr}/graphql");
        let client = DgClient::new(dead_url);
        let err = client.list_runs(25).await.unwrap_err();

        assert!(matches!(err, DgError::Transport(_)));
        let rendered = err.to_string();
        assert_eq!(rendered, "fetch failed");
        assert!(!rendered.contains("http"), "{rendered}");
        assert!(!rendered.contains(&addr.ip().to_string()), "{rendered}");
        assert!(!rendered.contains(&addr.port().to_string()), "{rendered}");
    }

    #[tokio::test]
    async fn launch_run_success_returns_run_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "launchRun": { "__typename": "LaunchRunSuccess", "run": { "runId": "run-123" } } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client.launch_run("refresh_lakehouse").await.unwrap();
        assert_eq!(outcome.run_id.as_deref(), Some("run-123"));
        assert!(outcome.error.is_none());
    }

    /// WS3 item 29 (Z9): `launch_run_with_config` must send
    /// `runConfigData` as a JSON STRING (`ExecutionParams.runConfigData`'s
    /// wire type), not a nested GraphQL object — `body_partial_json`
    /// asserts the exact `variables.cfg` value the mocked server receives.
    #[tokio::test]
    async fn launch_run_with_config_sends_run_config_data_as_a_json_string() {
        use wiremock::matchers::body_partial_json;

        let run_config = json!({"ops": {"run_ingest": {"config": {"connector_id": "conn-a"}}}});
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_partial_json(json!({
                "variables": { "cfg": run_config.to_string() }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "launchRun": { "__typename": "LaunchRunSuccess", "run": { "runId": "run-456" } } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client
            .launch_run_with_config("ingest_job", &run_config)
            .await
            .unwrap();
        assert_eq!(outcome.run_id.as_deref(), Some("run-456"));
        assert!(outcome.error.is_none());
    }

    /// A GraphQL-level failure (`RunConfigValidationInvalid`) must still
    /// come back as `Ok(LaunchOutcome { error: Some(_) })`, the same
    /// non-`Err` shape [`launch_run_python_error_returns_error_not_err`]
    /// proves for the config-free `launch_run`.
    #[tokio::test]
    async fn launch_run_with_config_validation_invalid_is_ok_not_err() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "launchRun": { "__typename": "RunConfigValidationInvalid",
                    "errors": [ { "message": "connector_id is required" } ] } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let run_config = json!({"ops": {"run_ingest": {"config": {}}}});
        let outcome = client
            .launch_run_with_config("ingest_job", &run_config)
            .await
            .unwrap();
        assert!(outcome.run_id.is_none());
        assert_eq!(outcome.error.as_deref(), Some("connector_id is required"));
    }

    #[tokio::test]
    async fn launch_run_python_error_returns_error_not_err() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "launchRun": { "__typename": "PythonError", "message": "boom" } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client.launch_run("refresh_lakehouse").await.unwrap();
        assert!(outcome.run_id.is_none());
        assert_eq!(outcome.error.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn launch_run_validation_invalid_joins_error_messages() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "launchRun": { "__typename": "RunConfigValidationInvalid",
                    "errors": [ { "message": "bad field a" }, { "message": "bad field b" } ] } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client.launch_run("refresh_lakehouse").await.unwrap();
        assert!(outcome.run_id.is_none());
        assert_eq!(outcome.error.as_deref(), Some("bad field a; bad field b"));
    }

    #[tokio::test]
    async fn list_runs_for_job_parses_end_time() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "runsOrError": { "__typename": "Runs", "results": [
                    { "runId": "r1", "jobName": "refresh_lakehouse", "status": "SUCCESS",
                      "startTime": 1.0, "endTime": 3.0 }
                ] } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let runs = client
            .list_runs_for_job("refresh_lakehouse", 30)
            .await
            .unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].end_time, Some(3.0));
    }

    #[tokio::test]
    async fn list_jobs_with_schedules_attaches_matching_schedule_only() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "repositoriesOrError": { "__typename": "RepositoryConnection", "nodes": [
                    { "jobs": [ { "name": "refresh_lakehouse" }, { "name": "other_job" }, { "name": "__ASSET_JOB" } ],
                      "schedules": [
                        { "name": "refresh_lakehouse_schedule", "cronSchedule": "0 3 * * *", "scheduleState": { "status": "RUNNING" }, "jobName": "refresh_lakehouse" }
                      ] }
                ] } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let jobs = client.list_jobs_with_schedules().await.unwrap();
        assert_eq!(jobs.len(), 2);
        let refresh = jobs.iter().find(|j| j.name == "refresh_lakehouse").unwrap();
        assert_eq!(refresh.schedules.len(), 1);
        assert_eq!(refresh.schedules[0].cron_schedule, "0 3 * * *");
        assert_eq!(refresh.schedules[0].schedule_state.status, "RUNNING");
        let other = jobs.iter().find(|j| j.name == "other_job").unwrap();
        assert!(other.schedules.is_empty());
    }

    #[tokio::test]
    async fn pipeline_run_status_parses_steps() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "pipelineRunOrError": { "__typename": "Run", "status": "SUCCESS",
                    "startTime": 1_756_267_200.0,
                    "stepStats": [ { "stepKey": "bronze_sdi", "status": "SUCCESS" } ] } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let info = client
            .pipeline_run_status("ead7470c-a36f-410f-95fe-ddef911805c9")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(info.status, "SUCCESS");
        assert_eq!(info.steps.len(), 1);
        assert_eq!(info.steps[0].key, "bronze_sdi");
        assert_eq!(info.start_time, Some(1_756_267_200.0));
    }

    #[tokio::test]
    // WS4 item G1: a run `Dagster` hasn't started yet reports `startTime:
    // null` over GraphQL — this must parse to `None`, never a fabricated
    // 0.0/now() value the API layer would then render as a fake ISO
    // timestamp.
    async fn pipeline_run_status_start_time_is_none_when_dagster_has_not_started_the_run() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "pipelineRunOrError": { "__typename": "Run", "status": "STARTING",
                    "startTime": null, "stepStats": [] } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let info = client.pipeline_run_status("r1").await.unwrap().unwrap();
        assert!(info.start_time.is_none());
    }

    #[tokio::test]
    async fn pipeline_run_status_none_when_run_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "pipelineRunOrError": { "__typename": "RunNotFoundError" } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let info = client.pipeline_run_status("nope").await.unwrap();
        assert!(info.is_none());
    }

    #[tokio::test]
    async fn pipeline_run_status_none_on_graphql_errors_not_err() {
        // Unlike list_runs/list_jobs, a GraphQL `errors` array here does
        // NOT become an `Err` — it becomes `Ok(None)`, matching the TS
        // route which never inspects `json.errors`.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "errors": [ { "message": "boom" } ]
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let info = client.pipeline_run_status("x").await.unwrap();
        assert!(info.is_none());
    }

    // ── Task 2.5: cancel/retry/pause/resume mutations ──────────────────
    //
    // These verify the mutation wiring (query shape, response parsing)
    // against a local `wiremock` server only. No test in this crate ever
    // talks to a live Dagster instance's mutation surface — the CRITICAL
    // SAFETY constraint in the Task 2.5 brief.

    #[tokio::test]
    async fn terminate_run_success_returns_run_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "terminateRun": { "__typename": "TerminateRunSuccess",
                    "run": { "runId": "r1" } } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client.terminate_run("r1").await.unwrap();
        assert_eq!(outcome.run_id.as_deref(), Some("r1"));
        assert!(outcome.error.is_none());
    }

    #[tokio::test]
    async fn terminate_run_not_found_is_error_not_err() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "terminateRun": { "__typename": "RunNotFoundError" } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client.terminate_run("nope").await.unwrap();
        assert!(outcome.run_id.is_none());
        assert_eq!(outcome.error.as_deref(), Some("RunNotFoundError"));
    }

    #[tokio::test]
    async fn terminate_run_failure_reports_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "terminateRun": { "__typename": "TerminateRunFailure",
                    "message": "already finished" } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client.terminate_run("r1").await.unwrap();
        assert_eq!(outcome.error.as_deref(), Some("already finished"));
    }

    #[tokio::test]
    async fn launch_reexecution_success_returns_new_run_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "launchRunReexecution": { "__typename": "LaunchRunSuccess",
                    "run": { "runId": "r2" } } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client.launch_reexecution("r1").await.unwrap();
        assert_eq!(outcome.run_id.as_deref(), Some("r2"));
    }

    #[tokio::test]
    async fn launch_reexecution_python_error_returns_error_not_err() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "launchRunReexecution": { "__typename": "PythonError",
                    "message": "boom" } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client.launch_reexecution("r1").await.unwrap();
        assert!(outcome.run_id.is_none());
        assert_eq!(outcome.error.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn start_schedule_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "startSchedule": { "__typename": "ScheduleStateResult",
                    "scheduleState": { "status": "RUNNING" } } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client
            .start_schedule("refresh_lakehouse_schedule")
            .await
            .unwrap();
        assert!(outcome.ok);
        assert!(outcome.error.is_none());
    }

    // ── WS4 item A4: run_logs ───────────────────────────────────────────

    #[tokio::test]
    async fn run_logs_parses_real_captured_fixture() {
        let server = MockServer::start().await;
        let body: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/run_logs_captured_fixture.json"
        ))
        .unwrap();
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let page = client
            .run_logs("f13d18ca-0553-410e-8ebf-4a1286bdbbe5", None, 20)
            .await
            .unwrap();
        // The real captured stream has 15 events, every one of them a
        // MessageEvent variant (WS4 item A1 — this run's log stream never
        // exercised a non-MessageEvent skip; see the synthetic test below).
        assert_eq!(page.lines.len(), 15);
        assert_eq!(
            page.cursor,
            "eyJ0eXBlIjogIlNUT1JBR0VfSUQiLCAidmFsdWUiOiA0Nn0="
        );
        assert!(!page.has_more);
        let failure = page
            .lines
            .iter()
            .find(|l| l.step_key.as_deref() == Some("run_bronze_maintenance") && l.level == "ERROR")
            .unwrap();
        assert!(failure.message.contains("failed"));
    }

    /// The real captured fixture's stream never contains a non-`MessageEvent`
    /// union member (every event `Dagster` emitted for that run implements
    /// the interface). This synthetic body (this crate's established
    /// pattern for branches a capture can't exercise) proves a
    /// `StepMaterializationEvent`-shaped entry — present only as
    /// `{"__typename": ...}` with no `message`/`level`/`timestamp`, exactly
    /// how a non-`MessageEvent` renders under the `... on MessageEvent`
    /// fragment — is skipped rather than causing a parse error.
    #[tokio::test]
    async fn run_logs_skips_non_message_events_and_keeps_the_rest() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "logsForRun": { "__typename": "EventConnection", "events": [
                    { "__typename": "MessageEvent", "message": "hello", "timestamp": "1000",
                      "level": "INFO", "stepKey": null },
                    { "__typename": "StepMaterializationEvent" },
                    { "__typename": "MessageEvent", "message": "world", "timestamp": "2000",
                      "level": "DEBUG", "stepKey": "step_a" }
                ], "cursor": "c1", "hasMore": true } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let page = client.run_logs("r1", None, 20).await.unwrap();
        assert_eq!(page.lines.len(), 2);
        assert_eq!(page.lines[0].message, "hello");
        assert_eq!(page.lines[1].message, "world");
        assert_eq!(page.lines[1].step_key.as_deref(), Some("step_a"));
        assert!(page.has_more);
        assert_eq!(page.cursor, "c1");
    }

    /// Unlike [`run_steps_run_not_found_returns_empty_not_err`], a missing
    /// run here is a genuine caller error — the route maps it to 404
    /// (Phase C) rather than an empty page, since a log viewer that can't
    /// find the run at all is a different condition than a run with no log
    /// lines yet. Real shape verified live (WS4 item A1) against a bogus
    /// run id: `logsForRun.__typename` becomes `RunNotFoundError`.
    #[tokio::test]
    async fn run_logs_run_not_found_is_err() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "logsForRun": { "__typename": "RunNotFoundError",
                    "message": "Pipeline run nope could not be found." } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let err = client.run_logs("nope", None, 20).await.unwrap_err();
        assert!(matches!(err, DgError::Server(_)));
    }

    // ── WS4 item A3: run_steps ──────────────────────────────────────────

    #[tokio::test]
    async fn run_steps_parses_real_captured_fixture() {
        let server = MockServer::start().await;
        let body: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/run_steps_captured_fixture.json"
        ))
        .unwrap();
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let steps = client
            .run_steps("f13d18ca-0553-410e-8ebf-4a1286bdbbe5")
            .await
            .unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].step_key, "run_bronze_maintenance");
        assert_eq!(steps[0].status, "FAILURE");
        assert!(steps[0].start_ms.is_some());
        assert!(steps[0].end_ms.is_some());
        // Real fixture: this run's step failed before any asset
        // materialized — must parse to an empty Vec, never fabricated rows.
        assert!(steps[0].materializations.is_empty());
    }

    /// No registered job on the live capture stack (WS4 item A1) emits an
    /// `IntMetadataEntry` labeled `"rows"`, so this synthetic body
    /// (matching this crate's established pattern) proves the field is
    /// parsed at all, and that a materialization with NO `"rows"`-labeled
    /// entry parses to `None`, never a fabricated `0`.
    #[tokio::test]
    async fn run_steps_parses_rows_metadata_and_defaults_missing_to_none() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "runOrError": { "__typename": "Run", "stepStats": [
                    { "stepKey": "with_rows", "status": "SUCCESS", "startTime": 1.0, "endTime": 2.0,
                      "materializations": [ { "assetKey": { "path": ["bronze", "orders"] },
                        "metadataEntries": [ { "__typename": "IntMetadataEntry", "label": "rows", "intValue": 1234 } ] } ] },
                    { "stepKey": "no_rows", "status": "SUCCESS", "startTime": 3.0, "endTime": 4.0,
                      "materializations": [ { "assetKey": { "path": ["bronze", "customers"] },
                        "metadataEntries": [ { "__typename": "TextMetadataEntry", "label": "note", "text": "ok" } ] } ] }
                ] } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let steps = client.run_steps("r1").await.unwrap();
        assert_eq!(steps.len(), 2);
        let with_rows = &steps[0];
        assert_eq!(with_rows.materializations.len(), 1);
        assert_eq!(with_rows.materializations[0].rows, Some(1234));
        assert_eq!(
            with_rows.materializations[0].asset_key.as_deref(),
            Some("bronze/orders")
        );
        let no_rows = &steps[1];
        assert_eq!(no_rows.materializations.len(), 1);
        assert_eq!(no_rows.materializations[0].rows, None);
    }

    /// A missing run is a normal "nothing to show" case here, matching the
    /// posture [`pipeline_run_status_none_when_run_not_found`] proves for
    /// the sibling call — real shape verified live (WS4 item A1) against a
    /// bogus run id: `runOrError.__typename` becomes `RunNotFoundError`.
    #[tokio::test]
    async fn run_steps_run_not_found_returns_empty_not_err() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "runOrError": { "__typename": "RunNotFoundError",
                    "message": "Pipeline run nope could not be found." } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let steps = client.run_steps("nope").await.unwrap();
        assert!(steps.is_empty());
    }

    // ── WS4 item A2: job_graph ──────────────────────────────────────────

    #[tokio::test]
    async fn job_graph_parses_ops_and_edges_from_a_real_captured_fixture() {
        let server = MockServer::start().await;
        let body: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/job_graph_bronze_maintenance.json"
        ))
        .unwrap();
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let graph = client.job_graph("bronze_maintenance_job").await.unwrap();
        assert!(!graph.ops.is_empty());
        assert!(graph.ops.iter().any(|o| o.name == "run_bronze_maintenance"));
    }

    /// The real captured fixture (WS4 item A1) has a single op with no
    /// dependencies, so it can't demonstrate edge extraction. This
    /// synthetic body (consistent with every other mutation/branch test in
    /// this module, which use hand-built `json!` bodies rather than
    /// captures) proves `inputs.dependsOn` is inverted into `from`/`to`
    /// edges correctly.
    #[tokio::test]
    async fn job_graph_derives_edges_from_input_depends_on() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "pipelineOrError": { "__typename": "Pipeline", "solidHandles": [
                    { "solid": { "name": "extract", "definition": { "description": null }, "inputs": [] } },
                    { "solid": { "name": "load", "definition": { "description": "loads rows" },
                      "inputs": [ { "dependsOn": [ { "solid": { "name": "extract" } } ] } ] } }
                ] } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let graph = client.job_graph("some_job").await.unwrap();
        assert_eq!(graph.ops.len(), 2);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].from, "extract");
        assert_eq!(graph.edges[0].to, "load");
        let load = graph.ops.iter().find(|o| o.name == "load").unwrap();
        assert_eq!(load.description.as_deref(), Some("loads rows"));
    }

    /// Real, live-verified shape (WS4 item A1, queried against a bogus job
    /// name on the same stack): a job unknown to this repository/location
    /// resolves `pipelineOrError.__typename` to `PipelineNotFoundError`,
    /// never a `Pipeline`. Must be a hard error, not a panic on `unwrap`.
    #[tokio::test]
    async fn job_graph_errors_when_job_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "pipelineOrError": { "__typename": "PipelineNotFoundError" } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let err = client.job_graph("no_such_job").await.unwrap_err();
        assert!(matches!(err, DgError::Server(_)));
    }

    #[tokio::test]
    async fn stop_schedule_not_found_is_error_not_err() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "stopRunningSchedule": { "__typename": "ScheduleNotFoundError",
                    "message": "no such schedule" } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client.stop_schedule("nope").await.unwrap();
        assert!(!outcome.ok);
        assert_eq!(outcome.error.as_deref(), Some("no such schedule"));
    }
}
