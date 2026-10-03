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
    /// Unix seconds the run was created (queued). The gap to `start_time`
    /// is time spent queued and launching. Only the per-job query asks for
    /// it; `None` elsewhere.
    #[serde(default)]
    pub creation_time: Option<f64>,
    /// The run this one re-executes, when it is a retry.
    #[serde(default)]
    pub parent_run_id: Option<String>,
    /// The first run of a retry chain, when this is a retry.
    #[serde(default)]
    pub root_run_id: Option<String>,
    /// The run's tags. `Dagster` records what launched a run here
    /// (`dagster/schedule_name`, `dagster/sensor_name`, `dagster/backfill`).
    #[serde(default)]
    pub tags: Vec<DgTag>,
}

/// One `key`/`value` tag on a [`DgRun`].
#[derive(Debug, Clone, Deserialize)]
pub struct DgTag {
    /// The tag's key, e.g. `dagster/schedule_name`.
    pub key: String,
    /// The tag's value.
    pub value: String,
}

/// A [`DgRun`] together with its summed materialization row count, for the
/// volume route (`GET /api/pipelines/{id}/volume`) and the
/// `pipeline_volume_drop` alert (plan 1f).
///
/// `rows` is `None` when no step of the run reported any `IntMetadataEntry`
/// labelled `"rows"` — a run that did real work whose every asset
/// materialization happens not to count rows (or whose steps failed before
/// materialization) is not a row count of `0`; it is a measurement gap.
#[derive(Debug, Clone)]
pub struct DgRunWithRows {
    /// The underlying run record (same shape [`list_runs_for_job`] returns).
    pub run: DgRun,
    /// Sum of every `"rows"`-labelled `IntMetadataEntry` across every
    /// step's materializations; `None` when no step reported rows.
    pub rows: Option<i64>,
}

/// How a re-execution picks the steps it runs again.
///
/// `Selected` is deliberately NOT a variant here: a re-execution of a
/// chosen subset carries step keys and Dagster expresses it through
/// `launchRunReexecution(executionParams { stepKeys })`, not through a
/// `ReexecutionStrategy` value. Parsing that request belongs to the
/// route layer, so this enum stays closed and the strategy mapping can
/// never report a strategy the caller did not ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReexecutionStrategy {
    /// Every step of the parent run.
    AllSteps,
    /// Only the steps that failed or did not run, reusing the outputs of
    /// the steps that succeeded. `Dagster` refuses it for a run that did
    /// not fail.
    FromFailure,
}

impl ReexecutionStrategy {
    const fn graphql(self) -> &'static str {
        match self {
            Self::AllSteps => "ALL_STEPS",
            Self::FromFailure => "FROM_FAILURE",
        }
    }
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
struct IsPipelineConfigValidData {
    #[serde(rename = "isPipelineConfigValid")]
    is_pipeline_config_valid: ConfigValidationResult,
}

/// Wire shape for `isPipelineConfigValid` — a `PipelineConfigValidationResult`
/// union (`dagster_graphql/schema/pipelines/config_result.py`). The
/// `Unknown` variant catches new typenames Dagster might add in a future
/// version; the route layer treats it as `NotFound` so an unknown
/// union member never claims success.
#[derive(Debug, Deserialize)]
#[serde(tag = "__typename")]
enum ConfigValidationResult {
    #[serde(rename = "RunConfigValidationInvalid")]
    Invalid {
        errors: Vec<ConfigValidationErrorWire>,
    },
    #[serde(rename = "PipelineConfigValidationValid")]
    Valid {
        #[serde(rename = "pipelineName")]
        _pipeline_name: String,
    },
    #[serde(rename = "PipelineNotFoundError")]
    NotFound {
        #[serde(default)]
        #[allow(
            dead_code,
            reason = "Dagster message; kept for debug only, never forwarded"
        )]
        message: Option<String>,
    },
    #[serde(rename = "InvalidSubsetError")]
    InvalidSubset {
        #[serde(default)]
        #[allow(
            dead_code,
            reason = "Dagster message; kept for debug only, never forwarded"
        )]
        message: Option<String>,
    },
    #[serde(other)]
    Unknown,
}

/// Wire shape for ONE `ConfigValidationError` — note this struct only
/// reads the `path` and `reason` fields (see [`DgClient::validate_run_config`]
/// for the rationale; `serde(other)` would silently drop free-form text
/// fields, but the route-layer mutation check depends on the fact that
/// those fields are NEVER deserialized here, so even an accidental
/// `serde_json::Value::to_string` downstream cannot echo them back).
#[derive(Debug, Deserialize)]
struct ConfigValidationErrorWire {
    path: Vec<String>,
    reason: String,
}

/// Convert the wire union into the public [`ConfigValidationOutcome`].
/// Dagster's `InvalidSubsetError` is collapsed to `NotFound` with no
/// errors: a subset-mismatch never includes structured `path`/`reason`
/// entries (the union uses a different shape on the `InvalidSubsetError`
/// branch), and the route layer renders an empty `errors` array the same
/// way it renders a non-empty one. `Unknown` is treated as `NotFound` —
/// the safer of the two "refused" outcomes, never a false `Valid`.
fn config_validation_outcome_from(wire: ConfigValidationResult) -> ConfigValidationOutcome {
    match wire {
        ConfigValidationResult::Invalid { errors } => ConfigValidationOutcome::Invalid {
            errors: errors
                .into_iter()
                .map(|e| ConfigValidationError {
                    path: e.path,
                    reason: e.reason,
                })
                .collect(),
        },
        ConfigValidationResult::Valid { .. } => ConfigValidationOutcome::Valid,
        ConfigValidationResult::NotFound { .. }
        | ConfigValidationResult::InvalidSubset { .. }
        | ConfigValidationResult::Unknown => ConfigValidationOutcome::NotFound,
    }
}

#[derive(Debug, Deserialize)]
struct RunConfigSchemaData {
    #[serde(rename = "runConfigSchemaOrError")]
    run_config_schema_or_error: RunConfigSchemaOrError,
}

/// Union wire shape for `runConfigSchemaOrError`. Only `Schema` and
/// `PipelineNotFoundError` map to special-cased variants here;
/// `InvalidSubsetError`, `ModeNotFoundError`, and `PythonError` each
/// capture a `message` but are flattened to `Other(..)` in
/// [`DgClient::run_config_schema`], which surfaces them as a 503 (the
/// route layer does NOT see `message` — the field is `#[allow(dead_code)]`
/// on purpose so it cannot accidentally be read or rendered).
#[derive(Debug, Deserialize)]
#[serde(tag = "__typename")]
enum RunConfigSchemaOrError {
    #[serde(rename = "RunConfigSchema")]
    Schema {
        #[serde(rename = "rootDefaultYaml")]
        root_default_yaml: String,
    },
    #[serde(rename = "PipelineNotFoundError")]
    NotFound {
        #[serde(default)]
        #[allow(
            dead_code,
            reason = "Dagster message; kept for debug only, never forwarded"
        )]
        message: Option<String>,
    },
    #[serde(rename = "InvalidSubsetError")]
    InvalidSubset {
        #[serde(default)]
        #[allow(
            dead_code,
            reason = "Dagster message; kept for debug only, never forwarded"
        )]
        message: Option<String>,
    },
    #[serde(rename = "ModeNotFoundError")]
    ModeNotFound {
        #[serde(default)]
        #[allow(
            dead_code,
            reason = "Dagster message; kept for debug only, never forwarded"
        )]
        message: Option<String>,
    },
    #[serde(rename = "PythonError")]
    PythonError {
        #[serde(default)]
        #[allow(
            dead_code,
            reason = "Dagster message; kept for debug only, never forwarded"
        )]
        message: Option<String>,
    },
}

impl RunConfigSchemaOrError {
    /// The wire-level `message` for any variant that carries one (the
    /// `message` is `None` for `Schema`, which is why this returns
    /// `None` for it). Used by [`DgClient::run_config_schema`] to
    /// surface a useful 503 for `InvalidSubsetError` / `ModeNotFoundError`
    /// / `PythonError`. The function is `pub(crate)` only — never
    /// `pub` — so the public surface of this crate does not expose
    /// upstream text. `message` is deliberately dead-coded at every
    /// call site (see the `#[allow(dead_code)]` attributes on the
    /// variants above) to make it impossible for a future contributor
    /// to render it into a response without first adding the
    /// `pub(crate) -> pub` boundary and getting the diff into review.
    #[allow(dead_code, reason = "see `DgClient::run_config_schema`'s refusal arm")]
    fn dagster_message(&self) -> Option<&str> {
        match self {
            RunConfigSchemaOrError::Schema { .. } | RunConfigSchemaOrError::NotFound { .. } => None,
            RunConfigSchemaOrError::InvalidSubset { message }
            | RunConfigSchemaOrError::ModeNotFound { message }
            | RunConfigSchemaOrError::PythonError { message } => message.as_deref(),
        }
    }

    /// The `__typename` of the deserialized variant, used in lieu of
    /// `message` for the 503 path when Dagster omits a message string.
    fn typename(&self) -> &'static str {
        match self {
            RunConfigSchemaOrError::Schema { .. } => "RunConfigSchema",
            RunConfigSchemaOrError::NotFound { .. } => "PipelineNotFoundError",
            RunConfigSchemaOrError::InvalidSubset { .. } => "InvalidSubsetError",
            RunConfigSchemaOrError::ModeNotFound { .. } => "ModeNotFoundError",
            RunConfigSchemaOrError::PythonError { .. } => "PythonError",
        }
    }
}

/// `runConfigSchemaOrError`'s successful payload — what the route
/// layer returns as `defaultConfigYaml`. Kept separate from
/// [`RunConfigSchemaOrError`] (the wire union) so the public API does
/// not carry an enum with a 503-only arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunConfigSchema {
    /// The default config for the pipeline, as a YAML string from
    /// `Dagster`'s `RunConfigSchema.rootDefaultYaml`. Kept as YAML
    /// (not parsed to JSON) — see [`DgClient::run_config_schema`] for
    /// the rationale.
    pub root_default_yaml: String,
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

/// Outcome of a sensor start/stop mutation (F1.2 #57).
///
/// Same shape as [`ScheduleOutcome`]: a sensor may be `SensorNotFoundError`
/// at start (the sensor does not exist; F1.2 makes the route's `paused`
/// branch not contribute one) and `UnauthorizedError`/`PythonError` at
/// start/stop. `stopSensor` does not surface `SensorNotFoundError`
/// (its result union has none — see Dagster schema `sensors.py`); a
/// `stop` against a missing sensor returns `Ok(SensorOutcome { ok: true,
/// error: None })` after the lookup step fails with `SensorNotFoundError`,
/// which is the tolerance the plan calls out (the chain is already
/// silent in that case, so the route's "already stopped" branch runs).
#[derive(Debug, Clone)]
pub struct SensorOutcome {
    /// Whether the sensor was successfully started/stopped.
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
    /// Unix seconds the run ended, or `None` if `Dagster` hasn't
    /// recorded an end yet (still running). Plan 1f: the run-finished
    /// event route needs this to compute `durationSeconds` and decide
    /// whether the run was over its `pipeline_sla.max_duration_seconds`.
    pub end_time: Option<f64>,
}

/// One step in a runs × steps matrix row — the matrix's per-cell shape,
/// `stepKey`/`status`/`durationMs`, sized to what the route layer renders
/// (no `startMs`/`endMs`/materializations, since the matrix view is a
/// grid, not a step detail view).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStepMatrixEntry {
    /// The step's key (e.g. `"run_bronze_maintenance"`).
    pub step_key: String,
    /// The step's raw `Dagster` status string (the route maps it through
    /// [`map_run_status`] for the response, the same way every other
    /// step status the API returns is mapped — see [`RunStep::status`]).
    pub status: String,
    /// `endTime - startTime` in milliseconds — `None` when either side is
    /// missing (a step that never started, or one whose timestamps Dagster
    /// hasn't recorded yet), never a fabricated `0`.
    pub duration_ms: Option<i64>,
}

/// One row in a runs × steps matrix — a run plus its per-step statuses,
/// returned by [`DgClient::list_runs_with_steps_for_job`].
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunWithSteps {
    /// The run's id.
    pub run_id: String,
    /// The run's `Dagster` status string (raw, like [`DgRun::status`]).
    pub status: String,
    /// Unix seconds the run started, or `None` if `Dagster` hasn't
    /// recorded one — the route layer turns this into `startedAt` and
    /// leaves it `null` rather than fabricating `now()`.
    pub start_time: Option<f64>,
    /// Each step's key + status + duration, in `Dagster`'s reported order.
    pub steps: Vec<RunStepMatrixEntry>,
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
    /// How many attempts `Dagster` recorded for this step
    /// (`stepStats.attempts`, a list of `RunMarker { startTime endTime }`).
    /// `1` for a step that ran once, `0` for one that never started
    /// (empty/absent list), `2+` for one that was retried.
    pub attempts: usize,
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

/// One structured validation error from
/// [`DgClient::validate_run_config`]. The fields are the ONLY two this
/// crate reads from Dagster's `ConfigValidationError`: the structured
/// `path` (where in the config) and the enum `reason` (why). Dagster's
/// own `message` text — free-form, sometimes a long English sentence —
/// is intentionally NOT carried here; AGENTS.md principle 4 forbids
/// forwarding upstream error text in a response, and this crate never
/// builds a `String` from `message` so the caller cannot accidentally
/// do so either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigValidationError {
    /// Path within the run config the error refers to (e.g.
    /// `["ops", "run_ingest", "config", "connector_id"]`). `Vec<String>`
    /// because `ConfigValidationError.path` is `[String!]` on the
    /// Dagster side (`dagster_graphql/schema/pipelines/config.py:136`).
    pub path: Vec<String>,
    /// `EvaluationErrorReason` enum value as a string (one of
    /// `RUNTIME_TYPE_MISMATCH`, `MISSING_REQUIRED_FIELD`,
    /// `MISSING_REQUIRED_FIELDS`, `FIELD_NOT_DEFINED`,
    /// `FIELDS_NOT_DEFINED`, `SELECTOR_FIELD_ERROR`).
    pub reason: String,
}

/// Outcome of [`DgClient::validate_run_config`]. The three variants
/// mirror `PipelineConfigValidationResult`'s four-typename union, minus
/// `PipelineConfigValidationValid` (collapsed into `Valid` since a
/// valid result carries no further data the caller needs).
///
/// This is deliberately NOT a [`crate::DgError`]: a typed refusal is
/// a normal outcome of validation, the same shape `launch_run`'s
/// `LaunchOutcome.error` already uses, never an `Err`. `Err` is
/// reserved for transport failures and malformed GraphQL responses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigValidationOutcome {
    /// `RunConfigValidationInvalid` / `InvalidSubsetError` — the
    /// caller-supplied config (or selector subset) is rejected by
    /// Dagster; the structured errors are returned as-is.
    Invalid {
        /// Every error Dagster reported, in its reported order.
        errors: Vec<ConfigValidationError>,
    },
    /// `PipelineConfigValidationValid` — the config is valid; the
    /// caller may launch.
    Valid,
    /// `PipelineNotFoundError` — `job_name` does not name a known
    /// pipeline in this repository/location. Callers map this to a
    /// 404 — `routes::pipelines::config_schema` already returns 404
    /// for an unknown id, and this variant carries the same answer.
    NotFound,
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
        // F1.5 (PR #55/#56 review BLOCKER): `limit` is caller-supplied, so
        // it travels as a GraphQL variable — every value crossing the
        // client/server seam in this crate goes through the variables
        // channel, never `format!`-interpolated into the query text.
        let query = "query($limit: Int!) { runsOrError(limit: $limit) { __typename \
                      ... on Runs { results { runId jobName status startTime endTime } } } }";
        let data: RunsOrErrorData = self.execute(query, Some(json!({ "limit": limit }))).await?;
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
        // F1.5 (PR #55/#56 review BLOCKER): `job_name` is the raw path id
        // for non-`pl-` ids (`routes::pipelines::runs_body`,
        // `runs_step_matrix`, `volume`), so a `pipeline:read` user who puts
        // `"` in it controls the query text sent to Dagster. The id MUST
        // travel as a GraphQL variable, never be interpolated into the
        // query string. `RunsFilter.pipelineName` is a scalar `String`
        // (`dagster_graphql/schema/inputs.py:51`), matching the inline
        // shape this query emitted before — only the injection channel
        // changes, the server-side semantics are identical.
        let query = "query($job: String!, $limit: Int!) { runsOrError(\
                      filter: { pipelineName: $job }, limit: $limit) { \
                      __typename ... on Runs { results { runId jobName status startTime endTime \
                      creationTime parentRunId rootRunId tags { key value } } } } }";
        let data: RunsOrErrorData = self
            .execute(query, Some(json!({ "job": job_name, "limit": limit })))
            .await?;
        Ok(data.runs_or_error.results.unwrap_or_default())
    }

    /// The runs × steps matrix for `job_name` (Plan 1c, R2): up to `limit`
    /// most-recent runs, each with its `stepStats { stepKey status
    /// startTime endTime }`. ONE GraphQL call — the route layer MUST NOT
    /// fan out per run (`with_materializations` would be a tempting extra
    /// field but is read out separately by [`DgClient::run_steps`] per
    /// single run when needed; the matrix deliberately excludes it to
    /// keep the one-call contract). Modeled on
    /// [`DgClient::list_runs_for_job`]'s `pipelineName` filter.
    ///
    /// # Errors
    ///
    /// See [`DgClient::list_runs`].
    pub async fn list_runs_with_steps_for_job(
        &self,
        job_name: &str,
        limit: u32,
    ) -> Result<Vec<RunWithSteps>, DgError> {
        // F1.5 (PR #55/#56 review BLOCKER): see [`DgClient::list_runs_for_job`]
        // — `job_name` travels as a GraphQL variable. Scalar `RunsFilter
        // .pipelineName` matches the original inline shape, server-side
        // semantics unchanged.
        let query = "query($job: String!, $limit: Int!) { runsOrError(\
                      filter: { pipelineName: $job }, limit: $limit) { \
                      __typename ... on Runs { results { runId status startTime \
                      stepStats { stepKey status startTime endTime } } } } }";
        let body = json!({ "query": query, "variables": { "job": job_name, "limit": limit } });
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
        let results = parsed
            .pointer("/data/runsOrError/results")
            .and_then(Value::as_array);
        let Some(results) = results else {
            return Ok(Vec::new());
        };
        Ok(results.iter().map(run_with_steps_from_value).collect())
    }

    /// Like [`list_runs_for_job`], but each run is paired with its summed
    /// materialization row count (plan 1f). Backed by a single GraphQL
    /// query that pulls `stepStats { materializations { metadataEntries } }`
    /// alongside the run fields, so the volume route stays one round trip
    /// rather than N+1 against `Dagster`. Rows are summed across every
    /// step's materializations; a run with no `"rows"`-labelled metadata
    /// reports `rows = None` (a measurement gap, never `0`).
    ///
    /// # Errors
    ///
    /// See [`DgClient::list_runs`].
    pub async fn list_runs_for_job_with_materializations(
        &self,
        job_name: &str,
        limit: u32,
    ) -> Result<Vec<DgRunWithRows>, DgError> {
        // F1.5 (PR #55/#56 review BLOCKER): see [`DgClient::list_runs_for_job`]
        // — `job_name` travels as a GraphQL variable. Scalar `RunsFilter
        // .pipelineName` matches the original inline shape, server-side
        // semantics unchanged.
        let query = "query($job: String!, $limit: Int!) { runsOrError(\
                      filter: { pipelineName: $job }, limit: $limit) { \
                      __typename ... on Runs { results { runId jobName status startTime endTime \
                      creationTime parentRunId rootRunId tags { key value } \
                      stepStats { stepKey status startTime endTime \
                      materializations { metadataEntries { __typename \
                      ... on IntMetadataEntry { label intValue } } } \
                      } }} }} }";
        // `metadataEntries` is a heterogeneous GraphQL union whose
        // `serde` untag would silently drop real variants, so the run
        // results are pulled as raw JSON and parsed with the same
        // value-navigation approach `pipeline_run_status` already uses
        // for `stepStats`.
        let value: Value = self
            .execute(query, Some(json!({ "job": job_name, "limit": limit })))
            .await?;
        let results = value
            .pointer("/runsOrError/results")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::with_capacity(results.len());
        for run_value in results {
            let run: DgRun = serde_json::from_value(run_value.clone())
                .map_err(|e| DgError::Server(format!("Dagster run payload: {e}")))?;
            let rows = rows_for_run(&run_value);
            out.push(DgRunWithRows { run, rows });
        }
        Ok(out)
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
                      definition { description metadata { key value } } \
                      inputs { dependsOn { solid { name } } } } } } \
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
                source_ref: metadata_value(&h.solid.definition.metadata, "source_ref"),
                commit: metadata_value(&h.solid.definition.metadata, "commit"),
                sql: metadata_value(&h.solid.definition.metadata, "sql"),
                reads: metadata_lines(&h.solid.definition.metadata, "reads"),
                writes: metadata_lines(&h.solid.definition.metadata, "writes"),
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

    /// Like [`DgClient::launch_run`], but with `runConfigData` set.
    /// `Dagster`'s GraphQL `ExecutionParams.runConfigData` is typed
    /// `RunConfigData`, a generic scalar that takes the config OBJECT
    /// itself. An earlier version declared the variable `String!` and sent
    /// the config serialized to a string; `Dagster` rejects that at
    /// validation (`Variable '$cfg' of type 'String!' used in position
    /// expecting type 'RunConfigData'`, HTTP 400), so no connector ingest
    /// run could launch. Used by `ingest_job` (one static job, per-connector
    /// `connector_id` config) —
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
        let query = "mutation($sel: JobOrPipelineSelector!, $cfg: RunConfigData!) { \
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
            "cfg": run_config,
        });
        let data: LaunchRunData = self.execute(query, Some(variables)).await?;
        Ok(launch_outcome_from(data.launch_run))
    }

    /// Validate `run_config` against `job_name`'s schema BEFORE launching
    /// it. Mirrors `query isPipelineConfigValid(pipeline: $sel, mode:
    /// "default", runConfigData: $cfg) { ... }` (R4 plan 2c).
    ///
    /// The query selects ONLY `path` and `reason` from `ConfigValidationError`,
    /// never `message`/`stack`/`valueRep`/`field`/`fields`/... — those are
    /// the strings and structured objects Dagster itself attaches to its
    /// own error type, several of which carry free-form English text (a
    /// "`MissingFieldConfigError.field.name`", a `RuntimeMismatchConfigError
    /// .value_rep` like `"'123' of type 'String'"`). Carrying them into the
    /// `ConfigValidationOutcome::Invalid.errors` would let the route layer
    /// accidentally (or by reviewer pressure) echo upstream text back to
    /// the caller; AGENTS.md principle 4 forbids that. The plan's mutation
    /// check (`passing Dagster's free text through in the 400 must FAIL`)
    /// is the regression test for this discipline.
    ///
    /// `mode` is `"default"` for every job this client targets (the same
    /// literal `launch_run` already passes — `lakehouse-dagster` was never
    /// multi-mode). `runConfigData` is sent as a JSON OBJECT (NOT a string),
    /// matching [`DgClient::launch_run_with_config`]'s shape and Dagster
    /// 1.13's `RunConfigData` scalar definition.
    ///
    /// A typed refusal (`RunConfigValidationInvalid`,
    /// `PipelineNotFoundError`, `InvalidSubsetError`) is reported as
    /// `Ok(ConfigValidationOutcome::{Invalid, NotFound})`, not `Err` —
    /// `Err` is reserved for transport failures and malformed GraphQL
    /// responses, mirroring [`DgClient::launch_run`]'s posture for the
    /// same shape.
    ///
    /// # Errors
    ///
    /// See [`DgClient::launch_run`].
    pub async fn validate_run_config(
        &self,
        job_name: &str,
        run_config: &Value,
    ) -> Result<ConfigValidationOutcome, DgError> {
        let query = "query($sel: PipelineSelector!, $cfg: RunConfigData!) { \
                      isPipelineConfigValid(pipeline: $sel, mode: \"default\", \
                      runConfigData: $cfg) { \
                      __typename \
                      ... on RunConfigValidationInvalid { errors { path reason } } \
                      ... on InvalidSubsetError { message } \
                      ... on PipelineConfigValidationValid { pipelineName } \
                      ... on PipelineNotFoundError { message } \
                      ... on PythonError { message } \
                      } }";
        let variables = json!({
            "sel": {
                "repositoryName": self.repo,
                "repositoryLocationName": self.location,
                "pipelineName": job_name,
            },
            "cfg": run_config,
        });
        // `execute<T>` unwraps `data` into `T`; see `GqlResponse`. A raw
        // `Value` parse is NOT used here (unlike `pipeline_run_status`) —
        // the union is closed (`Invalid | Valid | NotFound | PythonError`),
        // so a typed `Deserialize` keeps the failure-to-`__typename`
        // mapping compile-checked.
        let data: IsPipelineConfigValidData = self.execute(query, Some(variables)).await?;
        Ok(config_validation_outcome_from(
            data.is_pipeline_config_valid,
        ))
    }

    /// Fetch `job_name`'s run-config schema (its default config in YAML),
    /// used by `GET /api/pipelines/{id}/config-schema` (R4 plan 2c).
    /// Mirrors `query runConfigSchemaOrError(selector: $sel, mode: "default")
    /// { ... on RunConfigSchema { rootDefaultYaml } ... on
    /// PipelineNotFoundError { message } }`.
    ///
    /// `rootDefaultYaml` is a YAML string on Dagster's side (`run_config
    /// .py:51`, `resolve_rootDefaultYaml`). Parsing YAML into JSON would
    /// require a `serde_yaml`-family dep this workspace does not yet
    /// depend on (and `serde_yaml` is deprecated upstream); the route
    /// layer surfaces the raw YAML alongside `defaultConfig: null`
    /// rather than carrying a YAML parser for one field. Dagster
    /// emits at most a few dozen lines of plain YAML for a job's
    /// default config, and a console form built from JSON Schema /
    /// per-field editors does not need to pre-fill it anyway.
    ///
    /// # Errors
    ///
    /// See [`DgClient::launch_run`]. A `PipelineNotFoundError` is
    /// collapsed to `Ok(None)` — the same posture
    /// [`DgClient::pipeline_run_status`] already takes for its own
    /// `RunNotFoundError` — and the route maps that to a 404.
    pub async fn run_config_schema(
        &self,
        job_name: &str,
    ) -> Result<Option<RunConfigSchema>, DgError> {
        let query = "query($sel: PipelineSelector!) { \
                      runConfigSchemaOrError(selector: $sel, mode: \"default\") { \
                      __typename \
                      ... on RunConfigSchema { rootDefaultYaml } \
                      ... on InvalidSubsetError { message } \
                      ... on PipelineNotFoundError { message } \
                      ... on ModeNotFoundError { message } \
                      ... on PythonError { message } \
                      } }";
        let variables = json!({
            "sel": {
                "repositoryName": self.repo,
                "repositoryLocationName": self.location,
                "pipelineName": job_name,
            }
        });
        let data: RunConfigSchemaData = self.execute(query, Some(variables)).await?;
        match data.run_config_schema_or_error {
            RunConfigSchemaOrError::Schema { root_default_yaml } => {
                Ok(Some(RunConfigSchema { root_default_yaml }))
            }
            // `PipelineNotFoundError` is the only non-Schema typename
            // this client surfaces as `None`; the route maps that to a
            // 404. Other refusal typenames (`ModeNotFoundError`,
            // `InvalidSubsetError`, `PythonError`) ARE reported as
            // `Err(DgError::Server(...))` so they surface as a 503 like
            // every other `Dagster`-side error in this crate — the
            // "no such job" answer is the only one that collapses to a
            // "not found" answer without surfacing the orchestrator's
            // own message.
            RunConfigSchemaOrError::NotFound { .. } => Ok(None),
            RunConfigSchemaOrError::InvalidSubset { .. }
            | RunConfigSchemaOrError::ModeNotFound { .. }
            | RunConfigSchemaOrError::PythonError { .. } => {
                let wire = &data.run_config_schema_or_error;
                let msg = wire.dagster_message().unwrap_or_else(|| wire.typename());
                Err(DgError::Server(msg.to_owned()))
            }
        }
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

    /// Re-execute a finished (failed/cancelled) run, matching
    /// `mutation { launchRunReexecution(reexecutionParams: { parentRunId,
    /// strategy }) }`. Used by `retryRun` (Phase 2, Task 2.5), which
    /// defaults to [`ReexecutionStrategy::AllSteps`] and offers
    /// [`ReexecutionStrategy::FromFailure`] for a failed run.
    ///
    /// # Errors
    ///
    /// See [`DgClient::launch_run`].
    pub async fn launch_reexecution(
        &self,
        parent_run_id: &str,
        strategy: ReexecutionStrategy,
    ) -> Result<LaunchOutcome, DgError> {
        // `strategy` is an enum literal from a closed Rust enum, never
        // caller text, so it is safe to place in the query string.
        let query = format!(
            "mutation($parentRunId: String!) {{ \
             launchRunReexecution(reexecutionParams: {{ parentRunId: $parentRunId, \
             strategy: {} }}) {{ \
             __typename \
             ... on LaunchRunSuccess {{ run {{ runId }} }} \
             ... on PythonError {{ message }} \
             ... on RunConfigValidationInvalid {{ errors {{ message }} }} \
             }} }}",
            strategy.graphql()
        );
        let data: LaunchReexecutionData = self
            .execute(&query, Some(json!({ "parentRunId": parent_run_id })))
            .await?;
        Ok(launch_outcome_from(data.launch_run_reexecution))
    }

    /// Re-execute a finished run, but only the named `step_keys`. Two
    /// round trips: the parent run's `pipelineName`, `rootRunId`, and
    /// `runConfig` are looked up first; the mutation then calls
    /// `launchRunReexecution(executionParams: { selector, runConfigData,
    /// stepKeys, executionMetadata: { parentRunId, rootRunId } })`.
    ///
    /// Why the lookup-then-mutate shape: Dagster's
    /// `launchRunReexecution(executionParams)` requires the parent run's
    /// config to be passed back as `runConfigData` (`dagster_graphql/
    /// schema/inputs.py:322`, `RunConfigData` accepts the OBJECT form —
    /// see [`DgClient::launch_run_with_config`] for the same declared
    /// type), and the same mutation enforces EXACTLY ONE of
    /// `executionParams` / `reexecutionParams`, so a `strategy`-based
    /// `reexecutionParams` would lose `stepKeys`. The single source of
    /// `runConfig` Dagster exposes is `Run.runConfig`
    /// (`schema/pipelines/pipeline.py:646`, resolver `:811`), a generic
    /// `RunConfigData` scalar; its JSON shape round-trips straight back
    /// to `runConfigData` without any YAML parsing here — no
    /// `serde_yaml` / `dump_run_config_yaml` dependency introduced.
    ///
    /// A parent that does not resolve to a `Run` (`RunNotFoundError`,
    /// `__typename` other than `"Run"`) is returned as
    /// `Ok(LaunchOutcome { error: Some(_), run_id: None })` — matching
    /// [`DgClient::launch_reexecution`]'s posture. The route layer maps
    /// that to a `404`.
    ///
    /// # Errors
    ///
    /// See [`DgClient::launch_run`]. A transport-level failure on
    /// either the lookup or the mutation becomes [`DgError::Transport`]
    /// / [`DgError::Server`]; a Dagster-side typed refusal becomes
    /// `Ok(LaunchOutcome { error: Some(msg), .. })`.
    pub async fn launch_reexecution_of_steps(
        &self,
        parent_run_id: &str,
        step_keys: &[&str],
    ) -> Result<LaunchOutcome, DgError> {
        // ── Lookup: parent run's pipelineName, rootRunId, runConfig.
        // `runConfig` is a `RunConfigData` (`GenericScalar`) — Dagster
        // serialises the underlying Python dict as a JSON object. We
        // forward that object as `runConfigData` on the mutation, the
        // same OBJECT form `launch_run_with_config` already validates
        // (`$cfg: RunConfigData!`, body asserted in its own wiremock
        // test). No YAML parsing is needed: re-serialising this `Value`
        // straight through GraphQL JSON gives Dagster the exact same
        // shape it returned.
        let lookup_query = "query($rid: ID!) { pipelineRunOrError(runId: $rid) { \
                            __typename \
                            ... on Run { pipelineName rootRunId runConfig } } }";
        // `execute<T>` unwraps `data` into `T` (see `GqlResponse`), so
        // `lookup` is already the `{pipelineRunOrError: ...}` object
        // — not the whole response. Same posture as
        // [`DgClient::pipeline_run_status`], which reads
        // `parsed.pointer("/data/...")` only because it uses a raw
        // `Value` parse to tolerate a `Run` vs `RunNotFoundError` split.
        let lookup: Value = self
            .execute(lookup_query, Some(json!({ "rid": parent_run_id })))
            .await?;
        let parent = &lookup["pipelineRunOrError"];
        if parent.get("__typename").and_then(Value::as_str) != Some("Run") {
            return Ok(LaunchOutcome {
                run_id: None,
                error: Some("RunNotFoundError".to_owned()),
            });
        }
        let pipeline_name = parent
            .get("pipelineName")
            .and_then(Value::as_str)
            .ok_or_else(|| DgError::Server("parent run missing pipelineName".to_owned()))?
            .to_owned();
        // F1.6 (PR #56 review BLOCKER): Dagster's `executionParams` re-
        // execution path requires a non-null `rootRunId` whenever
        // `parentRunId` is present — `dagster_graphql/implementation/
        // execution/launch_execution.py:47-49` runs `check.str_param(
        // execution_metadata.root_run_id, "root_run_id")` when
        // `is_reexecuted`, and `check.str_param` raises on a non-`str`
        // (including the JSON `null` Dagster returns for a run that was
        // never re-executed) → PythonError on the mutation. The right
        // normalization is "the first re-execution makes the parent its
        // own root", which is exactly what its own `reexecutionParams`
        // path applies: `dagster/_core/instance/runs/run_domain.py:474`
        // `root_run_id = parent_run.root_run_id or parent_run.run_id`.
        // `parent.get("rootRunId")` is `None` for BOTH a JSON `null` value
        // (not a `str` per `Value::as_str`) AND a missing field; both
        // fall through `unwrap_or` to `parent_run_id`.
        let root_run_id = parent
            .get("rootRunId")
            .and_then(Value::as_str)
            .unwrap_or(parent_run_id)
            .to_owned();
        // `runConfig` is `RunConfigData!` on `Run` (schema/pipelines/
        // pipeline.py:646) — Dagster always returns it for a Run. We
        // still tolerate its absence as an empty object rather than
        // crashing, so a future schema change never makes this method
        // panic on `as_object().unwrap()`.
        let run_config = parent
            .get("runConfig")
            .cloned()
            .unwrap_or_else(|| Value::Object(serde_json::Map::default()));

        // ── Mutation: launchRunReexecution with executionParams.
        // `ExecutionParams.stepKeys` is `[String!]` (inputs.py:331-336).
        // `$rootRunId` is `String!` (F1.6 — null root with parent is
        // rejected at `launch_execution.py:47-49`; see the lookup above).
        let mutation = "mutation($sel: JobOrPipelineSelector!, \
                         $cfg: RunConfigData!, $keys: [String!]!, \
                         $parentRunId: String!, $rootRunId: String!) { \
                         launchRunReexecution(executionParams: { \
                         selector: $sel, runConfigData: $cfg, stepKeys: $keys, \
                         executionMetadata: { parentRunId: $parentRunId, \
                         rootRunId: $rootRunId } }) { \
                         __typename \
                         ... on LaunchRunSuccess { run { runId } } \
                         ... on PythonError { message } \
                         ... on RunConfigValidationInvalid { errors { message } } \
                         } }";
        let variables = json!({
            "sel": {
                "repositoryName": self.repo,
                "repositoryLocationName": self.location,
                "pipelineName": pipeline_name,
            },
            "cfg": run_config,
            "keys": step_keys,
            "parentRunId": parent_run_id,
            "rootRunId": root_run_id,
        });
        let data: LaunchReexecutionData = self.execute(mutation, Some(variables)).await?;
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

    /// Start (resume) or stop (pause) a sensor, matching Dagster 1.13.20
    /// `startSensor`/`stopSensor` mutations. Wired into
    /// `routes::pipelines::authored_status` next to the schedule toggle
    /// (F1.2 #57): when a chained pipeline is paused/resumed, the route
    /// must also stop/start the `authored__<id>_after` sensor so the
    /// chain truly goes quiet across a reload (Dagster keeps sensor
    /// state across a code-location reload; the Python module-level
    /// `default_status=RUNNING` does NOT reach the stored sensor state).
    ///
    /// The two mutations have different argument shapes against the
    /// real schema (`startSensor` takes a `SensorSelector!`; `stopSensor`
    /// takes a `String!` *sensor-state compound id*, NOT a selector).
    /// `stop` therefore does a two-step lookup: query the sensor's
    /// `sensorState { id }` by selector, then call `stopSensor(id: <id>)`.
    /// If the lookup is `SensorNotFoundError`, the sensor never existed
    /// (e.g. the pipeline has no `dependsOn`) and `running=false` is
    /// treated as success -- the chain was already silent.
    ///
    /// `start` returns the standard `SensorOrError` shape:
    /// `Sensor | SensorNotFoundError | UnauthorizedError | PythonError`.
    /// `stop` returns `StopSensorMutationResultOrError` which has NO
    /// `SensorNotFoundError` member (see `dagster_graphql/schema/sensors.py`
    /// line 245); the lookup-step check is the only path that turns
    /// "no such sensor" into `Ok(true)`.
    ///
    /// # Errors
    ///
    /// See [`DgClient::launch_run`].
    pub async fn set_sensor_running(
        &self,
        sensor_name: &str,
        running: bool,
    ) -> Result<SensorOutcome, DgError> {
        let sel = json!({
            "repositoryName": self.repo,
            "repositoryLocationName": self.location,
            "sensorName": sensor_name,
        });
        if running {
            let query = "mutation($sel: SensorSelector!) { startSensor(sensorSelector: $sel) { \
                         __typename \
                         ... on Sensor { sensorState { status } } \
                         ... on SensorNotFoundError { message } \
                         ... on PythonError { message } \
                         ... on UnauthorizedError { message } \
                         } }";
            let data: std::collections::HashMap<String, ScheduleMutationResultBody> =
                self.execute(query, Some(json!({ "sel": sel }))).await?;
            let r = data.into_values().next().ok_or_else(|| {
                DgError::Server("Dagster response missing startSensor result".to_owned())
            })?;
            if r.typename == "Sensor" {
                return Ok(SensorOutcome {
                    ok: true,
                    error: None,
                });
            }
            return Ok(SensorOutcome {
                ok: false,
                error: Some(r.message.unwrap_or(r.typename)),
            });
        }
        // Stop path: two-step. First, fetch the sensor's stored
        // instigation-state id; `stopSensor(id:)` requires it (the
        // schema accepts the string id of an `InstigationState`, not a
        // selector).
        let lookup_query = "query($sel: SensorSelector!) { sensorOrError(sensorSelector: $sel) { \
                            __typename \
                            ... on Sensor { sensorState { id status } } \
                            ... on SensorNotFoundError { message } \
                            } }";
        let lookup: Value = self
            .execute(lookup_query, Some(json!({ "sel": sel })))
            .await?;
        let sensor_block = &lookup["sensorOrError"];
        // `sensorOrError` returns `null` when the field is missing
        // (e.g. a sensor selector that the schema rejects outright);
        // treat that the same as `SensorNotFoundError`.
        let typename = sensor_block
            .get("__typename")
            .and_then(Value::as_str)
            .unwrap_or("");
        if typename != "Sensor" {
            // F1.2 tolerance: a missing sensor at `running=false` is
            // treated as already-stopped. The route (F1.2) calls
            // `set_sensor_running(_, false)` when pausing a pipeline
            // that has no `dependsOn` (no sensor was built for it), and
            // the `paused` branch in Python's `build_authored_dependency_sensor`
            // skips the sensor for paused pipelines whose `dependsOn`
            // has not yet been reloaded. In both cases the chain is
            // already silent; returning `Ok(true)` keeps the route's
            // toggle idempotent. `sensor_or_error` may also return
            // `null` outright when the field is missing entirely; we
            // collapse that to the same `SensorNotFoundError` shape.
            return Ok(SensorOutcome {
                ok: true,
                error: None,
            });
        }
        let Some(state_id) = sensor_block
            .get("sensorState")
            .and_then(|s| s.get("id"))
            .and_then(Value::as_str)
        else {
            return Ok(SensorOutcome {
                ok: false,
                error: Some("sensor missing sensorState.id".to_owned()),
            });
        };
        let stop_query = "mutation($id: String!) { stopSensor(id: $id) { \
                          __typename \
                          ... on StopSensorMutationResult { instigationState { id status } } \
                          ... on PythonError { message } \
                          ... on UnauthorizedError { message } \
                          } }";
        let data: std::collections::HashMap<String, ScheduleMutationResultBody> = self
            .execute(stop_query, Some(json!({ "id": state_id })))
            .await?;
        let r = data.into_values().next().ok_or_else(|| {
            DgError::Server("Dagster response missing stopSensor result".to_owned())
        })?;
        if r.typename == "StopSensorMutationResult" {
            return Ok(SensorOutcome {
                ok: true,
                error: None,
            });
        }
        Ok(SensorOutcome {
            ok: false,
            error: Some(r.message.unwrap_or(r.typename)),
        })
    }

    /// Ask the webserver to reload this client's code location, so jobs
    /// and schedules built at import time (authored pipelines,
    /// `authored_factory.py`) are rebuilt from the API's current data.
    /// Only a code server started with `dagster code-server start`
    /// re-imports its module on reload; a plain `dagster api grpc` server
    /// would reconnect and serve the same definitions.
    ///
    /// A GraphQL-level refusal (`RepositoryLocationNotFound`,
    /// `ReloadNotSupported`, a `PythonError` while loading) is returned as
    /// `Ok` with `error` set, like [`DgClient::launch_run`].
    ///
    /// # Errors
    ///
    /// See [`DgClient::launch_run`].
    pub async fn reload_location(&self) -> Result<ReloadOutcome, DgError> {
        let query = "mutation($name: String!) { reloadRepositoryLocation(repositoryLocationName: $name) { \
                      __typename \
                      ... on WorkspaceLocationEntry { locationOrLoadError { __typename \
                        ... on PythonError { message } } } \
                      ... on RepositoryLocationNotFound { message } \
                      ... on ReloadNotSupported { message } \
                      ... on UnauthorizedError { message } \
                      ... on PythonError { message } \
                      } }";
        let data: Value = self
            .execute(query, Some(json!({ "name": self.location })))
            .await?;
        Ok(reload_outcome_from(&data["reloadRepositoryLocation"]))
    }

    /// The most recent ticks of one schedule, newest first: when it fired,
    /// whether that launched a run, was skipped, or failed. A schedule that
    /// does not exist reads as an empty list, so a job without one simply
    /// has no ticks.
    ///
    /// # Errors
    ///
    /// See [`DgClient::launch_run`].
    pub async fn schedule_ticks(
        &self,
        schedule_name: &str,
        limit: u32,
    ) -> Result<Vec<ScheduleTick>, DgError> {
        let query = "query($sel: ScheduleSelector!, $limit: Int!) { scheduleOrError(scheduleSelector: $sel) { \
                      __typename \
                      ... on Schedule { scheduleState { ticks(limit: $limit) { \
                        tickId status timestamp runIds skipReason error { message } } } } \
                      } }";
        let variables = json!({
            "sel": {
                "repositoryName": self.repo,
                "repositoryLocationName": self.location,
                "scheduleName": schedule_name,
            },
            "limit": limit,
        });
        let data: Value = self.execute(query, Some(variables)).await?;
        Ok(schedule_ticks_from(&data["scheduleOrError"]))
    }

    /// The most recent ticks of one sensor, newest first: when it evaluated,
    /// whether it launched a run, was skipped, or failed. A sensor that does
    /// not exist reads as an empty list (R3 plan 2a — `authored__<id>_after`
    /// sensors only exist once their pipeline has `depends_on`; before that,
    /// the route merges in `[]` rather than failing).
    ///
    /// The query path is `sensorOrError(sensorSelector)` (note: the
    /// singular `SensorSelector`, NOT `SensorOrError.sensors(...)`) —
    /// `Dagster`'s `1.13.20` GraphQL schema names the selector type
    /// `SensorSelector` and the matching field `sensorOrError`. Verified
    /// live against this repository's Dagster stack; see the
    /// `schedule_ticks_from`-style unit test in this file.
    ///
    /// # Errors
    ///
    /// See [`DgClient::launch_run`].
    pub async fn sensor_ticks(
        &self,
        sensor_name: &str,
        limit: u32,
    ) -> Result<Vec<ScheduleTick>, DgError> {
        let query = "query($sel: SensorSelector!, $limit: Int!) { sensorOrError(sensorSelector: $sel) { \
                      __typename \
                      ... on Sensor { sensorState { ticks(limit: $limit) { \
                        tickId status timestamp runIds skipReason error { message } } } } \
                      } }";
        let variables = json!({
            "sel": {
                "repositoryName": self.repo,
                "repositoryLocationName": self.location,
                "sensorName": sensor_name,
            },
            "limit": limit,
        });
        let data: Value = self.execute(query, Some(variables)).await?;
        // The schedule and sensor tick payloads share the same
        // `{ tickId, status, timestamp, runIds, skipReason, error }`
        // shape; a shared `ticks_from_state` helper parses both — only
        // the parent key differs (`scheduleState` vs `sensorState`). A
        // `SensorNotFoundError` typename has no `sensorState`, so the
        // helper returns `[]`.
        Ok(sensor_ticks_from(&data["sensorOrError"]))
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
                      ... on Run { status startTime endTime stepStats { stepKey status } } } }";
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
        let end_time = run.get("endTime").and_then(Value::as_f64);
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
            end_time,
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
                      ... on IntMetadataEntry { label intValue } } } \
                      attempts { startTime endTime } } } } }";
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
    /// The op's own source location, `"dispar_orchestrate/<file>.py::<fn>"`
    /// — read from `definition.metadata`'s `"source_ref"`-keyed entry
    /// (WS4 item C1). `None` when the op carries no such entry (a
    /// foreign/future job this client must not panic on — this repository's
    /// own ops always carry one post-Phase-B, verified by
    /// `dagster/dispar_orchestrate/test_op_source_metadata.py`).
    pub source_ref: Option<String>,
    /// The commit `GIT_SHA` the op's own image was built from — read from
    /// `definition.metadata`'s `"commit"`-keyed entry. `None` when absent;
    /// see [`Self::source_ref`]'s note on untrusted/foreign jobs.
    pub commit: Option<String>,
    /// The literal SQL statement template the op executes, when it has a
    /// single one to show — read from `definition.metadata`'s
    /// `"sql"`-keyed entry. `None` (not empty string) when the op passes no
    /// `sql=` to `source_metadata` (`dagster/dispar_orchestrate/op_metadata.py`),
    /// which is every op in this code location today.
    pub sql: Option<String>,
    /// What the op's own code reads, one short phrase each, from the
    /// newline-joined `"reads"` entry `source_metadata` writes. Declared by
    /// the op, not observed from a run. Empty when the op declares nothing.
    pub reads: Vec<String>,
    /// What the op's own code writes; see [`Self::reads`].
    pub writes: Vec<String>,
}

/// What [`DgClient::reload_location`] reports: `error` is `None` when the
/// location reloaded and loaded cleanly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReloadOutcome {
    /// Why the reload was refused or the location failed to load, as the
    /// orchestrator said it. Server-side only: callers log it and never put
    /// it in a response.
    pub error: Option<String>,
}

fn reload_outcome_from(v: &Value) -> ReloadOutcome {
    let typename = v["__typename"].as_str().unwrap_or("");
    if typename == "WorkspaceLocationEntry" {
        let load = &v["locationOrLoadError"];
        if load["__typename"] == "PythonError" {
            return ReloadOutcome {
                error: Some(
                    load["message"]
                        .as_str()
                        .unwrap_or("location failed to load")
                        .to_owned(),
                ),
            };
        }
        return ReloadOutcome { error: None };
    }
    ReloadOutcome {
        error: Some(
            v["message"]
                .as_str()
                .map_or_else(|| format!("reload refused ({typename})"), str::to_owned),
        ),
    }
}

/// One schedule evaluation, from [`DgClient::schedule_ticks`].
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleTick {
    /// The tick's id.
    pub tick_id: String,
    /// `SUCCESS` (launched runs), `SKIPPED`, `FAILURE` or `STARTED`.
    pub status: String,
    /// Unix seconds the tick was evaluated.
    pub timestamp: f64,
    /// Runs the tick launched.
    pub run_ids: Vec<String>,
    /// Why the schedule decided not to launch, in the schedule's own
    /// words (`SkipReason` in the code location).
    pub skip_reason: Option<String>,
    /// Whether the tick failed with an error. The error text itself is the
    /// orchestrator's and is not carried here (AGENTS.md principle 4).
    pub failed: bool,
}

fn schedule_ticks_from(v: &Value) -> Vec<ScheduleTick> {
    ticks_from_state(&v["scheduleState"])
}

fn sensor_ticks_from(v: &Value) -> Vec<ScheduleTick> {
    ticks_from_state(&v["sensorState"])
}

fn ticks_from_state(state: &Value) -> Vec<ScheduleTick> {
    state["ticks"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| ScheduleTick {
            tick_id: t["tickId"]
                .as_str()
                .map_or_else(|| t["tickId"].to_string(), str::to_owned),
            status: t["status"].as_str().unwrap_or("").to_owned(),
            timestamp: t["timestamp"].as_f64().unwrap_or(0.0),
            run_ids: t["runIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| r.as_str().map(str::to_owned))
                .collect(),
            skip_reason: t["skipReason"].as_str().map(str::to_owned),
            failed: !t["error"].is_null(),
        })
        .collect()
}

/// Look up `label` in `entries` (`definition.metadata`'s `{key, value}`
/// list — `Dagster`'s GraphQL `metadata` field, sourced from the op's own
/// `tags`, not a typed union; see `dagster/dispar_orchestrate/op_metadata.py`'s
/// deviation note on why this client reads a flat key/value list instead of
/// the plan sketch's `TextMetadataEntry` union). `None` when no entry with
/// that key exists.
/// A newline-joined `label` entry split back into its phrases (see
/// `dagster/dispar_orchestrate/op_metadata.py::source_metadata`); blank
/// lines are dropped, and an absent entry is an empty list.
fn metadata_lines(entries: &[MetadataItem], label: &str) -> Vec<String> {
    metadata_value(entries, label)
        .map(|v| {
            v.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn metadata_value(entries: &[MetadataItem], label: &str) -> Option<String> {
    entries
        .iter()
        .find(|e| e.key == label)
        .map(|e| e.value.clone())
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
    /// `Dagster`'s `metadata` field is a `NON_NULL` list — real fixtures
    /// always carry it, but hand-built `json!` test bodies elsewhere in
    /// this module predate this field and omit it entirely, so `#[serde
    /// (default)]` keeps them compiling as empty rather than a parse
    /// failure.
    #[serde(default)]
    metadata: Vec<MetadataItem>,
}

/// One `{key, value}` pair from `SolidDefinition.metadata` — real,
/// live-verified shape (WS4 item C1), NOT the plan sketch's
/// `TextMetadataEntry` union.
#[derive(Debug, Deserialize)]
struct MetadataItem {
    key: String,
    value: String,
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

/// Convert one `runsOrError.results` element into a [`RunWithSteps`] —
/// one row of the runs × steps matrix. The `stepStats` array is mapped
/// through [`run_step_matrix_entry_from_value`] for the per-step parse;
/// the function reads nothing else (no `creationTime`, no `tags`, no
/// `parentRunId`, no `rootRunId`) because the matrix view does not
/// surface them.
fn run_with_steps_from_value(r: &Value) -> RunWithSteps {
    let steps = r
        .get("stepStats")
        .and_then(Value::as_array)
        .map(|stats| stats.iter().map(run_step_matrix_entry_from_value).collect())
        .unwrap_or_default();
    RunWithSteps {
        run_id: r
            .get("runId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        status: r
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        start_time: r.get("startTime").and_then(Value::as_f64),
        steps,
    }
}

/// Convert one `stepStats` element into a [`RunStepMatrixEntry`] — the
/// matrix's per-cell shape (`stepKey`/`status`/`durationMs`), narrower
/// than [`RunStep`] (no materializations, no startMs/endMs, no attempts).
/// `durationMs` is `None` when either `startTime` or `endTime` is
/// missing on the wire — never a fabricated `0`.
fn run_step_matrix_entry_from_value(s: &Value) -> RunStepMatrixEntry {
    let start_ms = seconds_to_ms(s.get("startTime"));
    let end_ms = seconds_to_ms(s.get("endTime"));
    let duration_ms = match (start_ms, end_ms) {
        (Some(a), Some(b)) => Some(b - a),
        _ => None,
    };
    RunStepMatrixEntry {
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
        duration_ms,
    }
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
    let attempts = s
        .get("attempts")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
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
        attempts,
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

/// Sum `rows` across every materialization of every step in `run_value`,
/// or `None` when no step reported any. A run with a mixture of
/// materializations (some with rows, some without) gets the sum of those
/// that did report — partial measurements are not failures, they just
/// aren't `0`.
fn rows_for_run(run_value: &Value) -> Option<i64> {
    let steps = run_value.get("stepStats")?.as_array()?;
    let mut total: Option<i64> = None;
    for step in steps {
        let Some(materializations) = step.get("materializations").and_then(Value::as_array) else {
            continue;
        };
        for mat in materializations {
            if let Some(rows) = rows_from_metadata_entries(mat.get("metadataEntries")?) {
                total = Some(total.map_or(rows, |t| t + rows));
            }
        }
    }
    total
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

    /// `launch_run_with_config` must declare `$cfg` as `RunConfigData!`
    /// and send the config OBJECT, matching `Dagster`'s schema for
    /// `ExecutionParams.runConfigData`. This test previously asserted a
    /// JSON string, which a mock accepts but real `Dagster` rejects with
    /// HTTP 400 at query validation. A mock cannot validate the schema, so
    /// both the declared type and the variable's shape are asserted here;
    /// the end-to-end proof is the G6 gate's real `ingest/run`.
    #[tokio::test]
    async fn launch_run_with_config_sends_run_config_data_as_an_object() {
        use wiremock::matchers::{body_partial_json, body_string_contains};

        let run_config = json!({"ops": {"run_ingest": {"config": {"connector_id": "conn-a"}}}});
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("$cfg: RunConfigData!"))
            .and(body_partial_json(json!({
                "variables": { "cfg": run_config }
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

    /// R4 plan 2c: `isPipelineConfigValid` returning
    /// `RunConfigValidationInvalid` maps to `ConfigValidationOutcome::Invalid`,
    /// and the wire-level `path` + `reason` come through as-is. The test
    /// also asserts the request body shape (`PipelineSelector` with
    /// `mode: "default"` + the user config as `RunConfigData`) and that
    /// the query SELECTS only `path` + `reason` (never `message`) — the
    /// latter is what AGENTS.md principle 4 + the plan's mutation check
    /// depend on, so the assertion is part of the test (without it, a
    /// regression that re-adds `message` to the wire selection would
    /// still pass).
    #[tokio::test]
    async fn validate_run_config_invalid_returns_path_and_reason() {
        use wiremock::matchers::{body_partial_json, body_string_contains};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("isPipelineConfigValid"))
            .and(body_partial_json(json!({
                "variables": {
                    "sel": { "repositoryName": "__repository__",
                             "repositoryLocationName": "dispar_orchestrate.definitions",
                             "pipelineName": "refresh_lakehouse" },
                    "cfg": { "ops": { "run_x": { "config": { "k": 1 } } } },
                }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "isPipelineConfigValid": { "__typename": "RunConfigValidationInvalid",
                    "errors": [
                        { "path": ["ops", "run_x", "config", "k"],
                          "reason": "RUNTIME_TYPE_MISMATCH",
                          "message": "value '1' is not a String" },
                        { "path": ["resources", "io"],
                          "reason": "FIELD_NOT_DEFINED" }
                    ] } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client
            .validate_run_config(
                "refresh_lakehouse",
                &json!({ "ops": { "run_x": { "config": { "k": 1 } } } }),
            )
            .await
            .unwrap();
        let ConfigValidationOutcome::Invalid { errors } = outcome else {
            panic!("expected Invalid, got {outcome:?}");
        };
        assert_eq!(errors.len(), 2);
        assert_eq!(
            errors[0],
            ConfigValidationError {
                path: vec!["ops".into(), "run_x".into(), "config".into(), "k".into()],
                reason: "RUNTIME_TYPE_MISMATCH".into(),
            }
        );
        assert_eq!(errors[1].path, vec!["resources", "io"]);
        assert_eq!(errors[1].reason, "FIELD_NOT_DEFINED");
    }

    /// R4 plan 2c: a `PipelineConfigValidationValid` response maps to
    /// `ConfigValidationOutcome::Valid`. The test also confirms the
    /// selector + variables shape so a future regression that changes
    /// either is caught at the test boundary.
    #[tokio::test]
    async fn validate_run_config_valid_is_ok() {
        use wiremock::matchers::{body_partial_json, body_string_contains};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("isPipelineConfigValid"))
            .and(body_partial_json(json!({
                "variables": {
                    "sel": { "repositoryName": "__repository__",
                             "repositoryLocationName": "dispar_orchestrate.definitions",
                             "pipelineName": "refresh_lakehouse" },
                    "cfg": {},
                }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "isPipelineConfigValid": { "__typename": "PipelineConfigValidationValid",
                    "pipelineName": "refresh_lakehouse" } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client
            .validate_run_config("refresh_lakehouse", &json!({}))
            .await
            .unwrap();
        assert_eq!(outcome, ConfigValidationOutcome::Valid);
    }

    /// R4 plan 2c: `PipelineNotFoundError` from `isPipelineConfigValid`
    /// is reported as `ConfigValidationOutcome::NotFound` (NOT `Err`),
    /// matching `LaunchOutcome.error`'s posture. The test asserts the
    /// message is deserialized-but-not-forwarded (i.e. the field is
    /// dropped at the wire union) — the route layer cannot echo Dagster's
    /// own `message` if it is never read.
    #[tokio::test]
    async fn validate_run_config_not_found_is_ok_not_err() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "isPipelineConfigValid": { "__typename": "PipelineNotFoundError",
                    "message": "Could not find pipeline named refresh_lakehouse" } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client
            .validate_run_config("refresh_lakehouse", &json!({}))
            .await
            .unwrap();
        assert_eq!(outcome, ConfigValidationOutcome::NotFound);
    }

    /// R4 plan 2c: `runConfigSchemaOrError` returning `RunConfigSchema`
    /// carries `rootDefaultYaml` and is mapped to `Some(RunConfigSchema)`.
    /// The query's variables must be `PipelineSelector` (with the same
    /// `repositoryName` / `repositoryLocationName` / `pipelineName`
    /// shape) so a future regression that switches to a different
    /// selector type (e.g. `PipelineSelector.byName` accidentally) is
    /// caught here.
    #[tokio::test]
    async fn run_config_schema_returns_root_default_yaml() {
        use wiremock::matchers::{body_partial_json, body_string_contains};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("runConfigSchemaOrError"))
            .and(body_partial_json(json!({
                "variables": {
                    "sel": {
                        "repositoryName": "__repository__",
                        "repositoryLocationName": "dispar_orchestrate.definitions",
                        "pipelineName": "refresh_lakehouse"
                    }
                }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "runConfigSchemaOrError": {
                        "__typename": "RunConfigSchema",
                        "rootDefaultYaml": "ops:\n  run_x:\n    config:\n      k: v\n"
                    }
                }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let schema = client
            .run_config_schema("refresh_lakehouse")
            .await
            .unwrap()
            .expect("expected Some, got None");
        assert_eq!(
            schema.root_default_yaml,
            "ops:\n  run_x:\n    config:\n      k: v\n"
        );
    }

    /// R4 plan 2c: `PipelineNotFoundError` from `runConfigSchemaOrError`
    /// maps to `Ok(None)` (the route maps that to a 404), NOT to `Err`
    /// — Dagster's `message` ("Could not find pipeline named ...") is
    /// never read by this client, only the `__typename`.
    #[tokio::test]
    async fn run_config_schema_not_found_returns_none() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "runConfigSchemaOrError": { "__typename": "PipelineNotFoundError",
                    "message": "Could not find pipeline named refresh_lakehouse" } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let schema = client.run_config_schema("refresh_lakehouse").await.unwrap();
        assert!(schema.is_none());
    }

    /// R4 plan 2c: a non-`RunConfigSchema` refusal (here `PythonError`)
    /// surfaces as `Err(DgError::Server(_))` — Dagster's own `message`
    /// text IS used here, but only inside the crate's `DgError::Server`,
    /// never forwarded to a response (the route layer translates
    /// `DgError::Server` to a 503 via `js_error` (`"Error: {Display}"`)
    /// — see `routes::pipelines`'s call sites).
    /// The test is the contract for "this is the only path that uses the
    /// message"; a regression that forwards it as a 200 would have to
    /// either change this match arm or add a new variant.
    #[tokio::test]
    async fn run_config_schema_python_error_returns_server_err() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "runConfigSchemaOrError": { "__typename": "PythonError",
                    "message": "schema failure" } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let err = client
            .run_config_schema("refresh_lakehouse")
            .await
            .expect_err("expected Err, got Ok");
        assert!(matches!(err, DgError::Server(_)), "got {err:?}");
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

    /// F1.5 (PR #55/#56 review BLOCKER): `job_name` is the raw path id
    /// for non-`pl-` ids (`routes::pipelines::runs_body`,
    /// `routes_step_matrix`, `volume`), so a `pipeline:read` user could
    /// put `"` and `#` in it. The id MUST travel as a GraphQL variable,
    /// never be interpolated into the query text. The mock pins the
    /// constant query substring (`pipelineName: $job` — a `$variable`
    /// reference that a `format!` interpolation cannot produce) AND the
    /// variables object containing the value. If the implementation
    /// regresses to `format!`, the body won't include the constant
    /// substring, no mock matches, wiremock returns 404, the client
    /// surfaces `DgError::Server("Dagster HTTP 404")`, and the `unwrap`
    /// here fails — that's the mutation-evidence red.
    #[tokio::test]
    async fn list_runs_for_job_job_name_travels_as_a_graphql_variable() {
        use wiremock::matchers::{body_partial_json, body_string_contains};

        // `"` and `#` — exactly the characters the prompt names, in an id
        // that does NOT contain the constant query marker as a substring
        // (so the `body_string_contains` matcher can't false-positive if
        // the id were ever echoed into the query).
        let evil = "r\"#x";
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("pipelineName: $job"))
            .and(body_partial_json(json!({
                "variables": { "job": evil, "limit": 30 }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "runsOrError": { "__typename": "Runs", "results": [] } }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let runs = client.list_runs_for_job(evil, 30).await.unwrap();
        assert!(runs.is_empty());
    }

    /// Same F1.5 contract for `list_runs_with_steps_for_job`. The route
    /// path through `runs_step_matrix` is the same as `runs_body` — it
    /// passes the raw id when it does not start with `pl-`.
    #[tokio::test]
    async fn list_runs_with_steps_for_job_job_name_travels_as_a_graphql_variable() {
        use wiremock::matchers::{body_partial_json, body_string_contains};

        let evil = "r\"#x";
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("pipelineName: $job"))
            .and(body_partial_json(json!({
                "variables": { "job": evil, "limit": 30 }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "runsOrError": { "__typename": "Runs", "results": [] } }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let rows = client.list_runs_with_steps_for_job(evil, 30).await.unwrap();
        assert!(rows.is_empty());
    }

    /// Same F1.5 contract for `list_runs_for_job_with_materializations`,
    /// reached via the `volume` route.
    #[tokio::test]
    async fn list_runs_for_job_with_materializations_job_name_travels_as_a_graphql_variable() {
        use wiremock::matchers::{body_partial_json, body_string_contains};

        let evil = "r\"#x";
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("pipelineName: $job"))
            .and(body_partial_json(json!({
                "variables": { "job": evil, "limit": 30 }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "runsOrError": { "__typename": "Runs", "results": [] } }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let runs = client
            .list_runs_for_job_with_materializations(evil, 30)
            .await
            .unwrap();
        assert!(runs.is_empty());
    }

    /// F1.5 (PR #55/#56 review BLOCKER): `list_runs` is the sibling that
    /// only takes a `limit`. It too used to interpolate the limit into
    /// the query string; `limit` is caller-supplied (a `u32`, so cannot
    /// carry GraphQL syntax), but converting it to `$limit: Int!` is
    /// trivial and keeps every client call in this crate consistent with
    /// the variable-channel contract.
    #[tokio::test]
    async fn list_runs_sends_limit_as_a_graphql_variable() {
        use wiremock::matchers::{body_partial_json, body_string_contains};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("runsOrError(limit: $limit)"))
            .and(body_partial_json(json!({ "variables": { "limit": 25 } })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "runsOrError": { "__typename": "Runs", "results": [] } }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let _ = client.list_runs(25).await.unwrap();
    }

    /// Plan 1c (R2): the runs × steps matrix comes from ONE GraphQL call
    /// per pipeline, not one per run. This test proves the response shape
    /// the route layer turns into `{ runs: [{ runId, status, startedAt,
    /// steps: [{ stepKey, status, durationMs|null }] }], unavailable }`
    /// — and that two runs in one `results[]` arrive in two `RunWithSteps`,
    /// not one (the route never has to fan out per-run).
    #[tokio::test]
    async fn list_runs_with_steps_for_job_returns_step_stats_per_run_in_one_call() {
        use wiremock::matchers::body_string_contains;

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("stepStats"))
            .and(body_string_contains("refresh_lakehouse"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "runsOrError": { "__typename": "Runs", "results": [
                    { "runId": "r1", "jobName": "refresh_lakehouse", "status": "SUCCESS",
                      "startTime": 1.0, "endTime": 3.0,
                      "stepStats": [
                          { "stepKey": "extract", "status": "SUCCESS",
                            "startTime": 1.0, "endTime": 2.0 },
                          { "stepKey": "load", "status": "SUCCESS",
                            "startTime": 2.0, "endTime": 3.0 }
                      ] },
                    { "runId": "r2", "jobName": "refresh_lakehouse", "status": "FAILURE",
                      "startTime": 4.0, "endTime": 7.0,
                      "stepStats": [
                          { "stepKey": "extract", "status": "SUCCESS",
                            "startTime": 4.0, "endTime": 5.0 },
                          { "stepKey": "load", "status": "FAILURE",
                            "startTime": 5.0, "endTime": 7.0 }
                      ] }
                ] } }
            })))
            // PR #57 review F1.10: `.expect(1)` is the "one call per
            // request" guarantee the matrix's per-page shape and the
            // brief's "matrix is single-round-trip" rule need. The route
            // MUST NOT fan out per-run; a regression that adds a second
            // round trip flips this to an expect-1 mismatch rather than
            // a silent "well, the response looked right".
            .expect(1)
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let rows = client
            .list_runs_with_steps_for_job("refresh_lakehouse", 30)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2, "both runs are present in one response");
        assert_eq!(rows[0].run_id, "r1");
        assert_eq!(rows[0].steps.len(), 2);
        assert_eq!(rows[0].steps[0].step_key, "extract");
        assert_eq!(
            rows[0].steps[0].duration_ms,
            Some(1000),
            "durationMs is the per-step end-start diff"
        );
        assert_eq!(rows[1].run_id, "r2");
        assert_eq!(rows[1].steps.len(), 2);
        assert_eq!(rows[1].steps[1].status, "FAILURE");
    }

    /// A step whose `startTime`/`endTime` are both `null` (it never ran)
    /// reports `durationMs: None`, not a fabricated `0` — same honesty
    /// posture as the API layer's own "never fabricate a measurement"
    /// rule (AGENTS.md principle 5).
    #[tokio::test]
    async fn list_runs_with_steps_reports_null_duration_for_a_step_that_never_started() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "runsOrError": { "__typename": "Runs", "results": [
                    { "runId": "r1", "jobName": "j", "status": "FAILURE",
                      "startTime": 1.0, "endTime": 2.0,
                      "stepStats": [
                          { "stepKey": "ran", "status": "SUCCESS",
                            "startTime": 1.0, "endTime": 2.0 },
                          { "stepKey": "never_started", "status": "SKIPPED",
                            "startTime": null, "endTime": null }
                      ] }
                ] } }
            })))
            // PR #57 review F1.10: same `.expect(1)` as the primary
            // "one call" test above — this test also goes through the
            // matrix at limit 30, and a regression that fans out per
            // step to look up the `null` duration would over-fire.
            .expect(1)
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let rows = client.list_runs_with_steps_for_job("j", 30).await.unwrap();
        assert_eq!(rows.len(), 1);
        let step_duration = rows[0]
            .steps
            .iter()
            .find(|s| s.step_key == "never_started")
            .map(|s| s.duration_ms);
        assert_eq!(step_duration, Some(None));
    }

    /// `list_runs_for_job_with_materializations` sums the `"rows"`
    /// metadata entries across every step's materializations, returning
    /// `None` for a run whose steps reported no row count.
    #[tokio::test]
    async fn list_runs_for_job_with_materializations_sums_rows_across_steps() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "runsOrError": { "__typename": "Runs", "results": [
                    { "runId": "r1", "jobName": "refresh_lakehouse", "status": "SUCCESS",
                      "startTime": 1.0, "endTime": 3.0,
                      "stepStats": [
                          { "stepKey": "step_a", "status": "SUCCESS",
                            "materializations": [
                                { "metadataEntries": [
                                    { "__typename": "IntMetadataEntry",
                                      "label": "rows", "intValue": 100 }
                                ] }
                            ] },
                          { "stepKey": "step_b", "status": "SUCCESS",
                            "materializations": [
                                { "metadataEntries": [
                                    { "__typename": "IntMetadataEntry",
                                      "label": "rows", "intValue": 250 }
                                ] },
                                { "metadataEntries": [
                                    { "__typename": "TextMetadataEntry",
                                      "label": "note", "text": "no count here" }
                                ] }
                            ] }
                      ] },
                    { "runId": "r2", "jobName": "refresh_lakehouse", "status": "SUCCESS",
                      "startTime": 4.0, "endTime": 5.0,
                      "stepStats": [
                          { "stepKey": "step_a", "status": "SUCCESS",
                            "materializations": [
                                { "metadataEntries": [
                                    { "__typename": "TextMetadataEntry",
                                      "label": "note", "text": "rows aren't tracked" }
                                ] }
                            ] }
                      ] }
                ] } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let runs = client
            .list_runs_for_job_with_materializations("refresh_lakehouse", 30)
            .await
            .unwrap();
        assert_eq!(runs.len(), 2);
        // r1: 100 + 250 = 350 (the TextMetadataEntry contributes nothing).
        assert_eq!(runs[0].rows, Some(350));
        // r2: nothing reported rows, so the run reports None — not 0.
        assert_eq!(runs[1].rows, None);
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
        let outcome = client
            .launch_reexecution("r1", ReexecutionStrategy::AllSteps)
            .await
            .unwrap();
        assert_eq!(outcome.run_id.as_deref(), Some("r2"));
    }

    #[test]
    fn a_reload_reports_a_clean_load_a_load_error_and_a_refusal_apart() {
        let ok = json!({ "__typename": "WorkspaceLocationEntry",
            "locationOrLoadError": { "__typename": "RepositoryLocation" } });
        assert_eq!(reload_outcome_from(&ok).error, None);
        let broken = json!({ "__typename": "WorkspaceLocationEntry",
            "locationOrLoadError": { "__typename": "PythonError", "message": "boom" } });
        assert_eq!(reload_outcome_from(&broken).error.as_deref(), Some("boom"));
        let refused = json!({ "__typename": "ReloadNotSupported", "message": "no" });
        assert_eq!(reload_outcome_from(&refused).error.as_deref(), Some("no"));
    }

    #[test]
    fn schedule_ticks_keep_skip_reasons_but_not_error_text() {
        let v = json!({ "__typename": "Schedule", "scheduleState": { "ticks": [
            { "tickId": "1", "status": "SUCCESS", "timestamp": 10.0, "runIds": ["r1"],
              "skipReason": null, "error": null },
            { "tickId": "2", "status": "SKIPPED", "timestamp": 5.0, "runIds": [],
              "skipReason": "nothing new", "error": null },
            { "tickId": "3", "status": "FAILURE", "timestamp": 1.0, "runIds": [],
              "skipReason": null, "error": { "message": "Traceback ... secret" } }
        ] } });
        let ticks = schedule_ticks_from(&v);
        assert_eq!(ticks.len(), 3);
        assert_eq!(ticks[0].run_ids, vec!["r1".to_owned()]);
        assert_eq!(ticks[1].skip_reason.as_deref(), Some("nothing new"));
        assert!(ticks[2].failed);
        let serialized = serde_json::to_string(&ticks).unwrap();
        assert!(
            !serialized.contains("Traceback"),
            "error text must not be carried"
        );
        // A schedule that does not exist has no ticks, not an error.
        assert!(schedule_ticks_from(&json!({ "__typename": "ScheduleNotFoundError" })).is_empty());
    }

    /// R3 plan 2a BLOCKER regression: `sensor_ticks` selects
    /// `sensorState { ticks { ... } }`, but the parser used to read
    /// `scheduleState`. A `Sensor` has no `scheduleState`, so the call
    /// returned `[]` for every sensor regardless of data — the
    /// `authored__<id>_after` dependency ticks the UI was supposed to
    /// merge in were silently dropped. The fixture mirrors the shape the
    /// `sensor_ticks` query actually selects, including the
    /// `SensorNotFoundError` typename the route treats as "no ticks".
    #[test]
    fn sensor_ticks_extract_ticks_from_sensor_state_not_schedule_state() {
        let v = json!({ "__typename": "Sensor", "sensorState": { "ticks": [
            { "tickId": "s1", "status": "SUCCESS", "timestamp": 10.0, "runIds": ["r1"],
              "skipReason": null, "error": null },
            { "tickId": "s2", "status": "SKIPPED", "timestamp": 5.0, "runIds": [],
              "skipReason": "upstream not ready", "error": null },
            { "tickId": "s3", "status": "FAILURE", "timestamp": 1.0, "runIds": [],
              "skipReason": null, "error": { "message": "Traceback ... secret" } }
        ] } });
        let ticks = sensor_ticks_from(&v);
        assert_eq!(
            ticks.len(),
            3,
            "sensor ticks must be parsed from sensorState.ticks"
        );
        assert_eq!(ticks[0].run_ids, vec!["r1".to_owned()]);
        assert_eq!(ticks[1].skip_reason.as_deref(), Some("upstream not ready"));
        assert!(ticks[2].failed);
        let serialized = serde_json::to_string(&ticks).unwrap();
        assert!(
            !serialized.contains("Traceback"),
            "Dagster error text must not be carried in the response"
        );
        // A sensor that does not exist has no ticks, not an error.
        assert!(sensor_ticks_from(&json!({ "__typename": "SensorNotFoundError" })).is_empty());
    }

    #[tokio::test]
    async fn launch_reexecution_from_failure_sends_that_strategy() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(wiremock::matchers::body_string_contains(
                "strategy: FROM_FAILURE",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "launchRunReexecution": { "__typename": "LaunchRunSuccess",
                    "run": { "runId": "r3" } } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client
            .launch_reexecution("r1", ReexecutionStrategy::FromFailure)
            .await
            .unwrap();
        assert_eq!(outcome.run_id.as_deref(), Some("r3"));
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
        let outcome = client
            .launch_reexecution("r1", ReexecutionStrategy::AllSteps)
            .await
            .unwrap();
        assert!(outcome.run_id.is_none());
        assert_eq!(outcome.error.as_deref(), Some("boom"));
    }

    /// Plan 1c (R2) + F1.6 (PR #56 review BLOCKER): the selected-steps
    /// re-execution first looks up the parent run's `pipelineName`/
    /// `rootRunId`/`runConfig`, then calls `launchRunReexecution(
    /// executionParams: { selector, runConfigData, stepKeys,
    /// executionMetadata: { parentRunId, rootRunId } })`. The wiremock
    /// must see ONE GraphQL body that carries the requested `stepKeys`
    /// AND the parent config — otherwise Dagster would launch without its
    /// required resources (see `dagster_graphql/schema/inputs.py:314-340`).
    ///
    /// F1.6 reason for the `null` root in the lookup: a run that was
    /// NEVER re-executed has `rootRunId: null` in Dagster's `Run` payload
    /// — that is the shape production produces for an ordinary run, and
    /// is the one that broke "re-run selected steps" in the field (Dagster
    /// refuses a null `rootRunId` with a parent on the `executionParams`
    /// path: `dagster_graphql/implementation/execution/launch_execution.py:
    /// 47-49` runs `check.str_param(execution_metadata.root_run_id,
    /// "root_run_id")`, which raises on None → `PythonError`). The mutation
    /// must therefore carry `rootRunId: parentRunId` — exactly the
    /// normalization Dagster's own `reexecutionParams` path applies
    /// (`dagster/_core/instance/runs/run_domain.py:474`, `root_run_id =
    /// parent_run.root_run_id or parent_run.run_id`).
    #[tokio::test]
    async fn launch_reexecution_of_steps_sends_selector_step_keys_and_parent_config() {
        use wiremock::matchers::{body_partial_json, body_string_contains};

        let server = MockServer::start().await;
        // Register the mutation mock FIRST so it gets the higher priority
        // (wiremock uses "most recently mounted first" — see the docs on
        // `MockServer`). Then mount the lookup mock so it cannot shadow the
        // mutation on a body that contains both `pipelineRunOrError` AND
        // `launchRunReexecution` (none of our requests do, but the order
        // makes the intent explicit).
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("launchRunReexecution"))
            .and(body_string_contains("$cfg: RunConfigData!"))
            .and(body_partial_json(json!({
                "variables": {
                    "sel": { "repositoryName": "__repository__",
                             "repositoryLocationName": "dispar_orchestrate.definitions",
                             "pipelineName": "refresh_lakehouse" },
                    "cfg": { "ops": { "run_x": { "config": { "k": "v" } } } },
                    "keys": ["run_x", "run_y"],
                    "parentRunId": "parent-1",
                    "rootRunId": "parent-1",
                }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "launchRunReexecution": { "__typename": "LaunchRunSuccess",
                    "run": { "runId": "new-run" } } }
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("pipelineRunOrError"))
            .and(body_partial_json(json!({
                "variables": { "rid": "parent-1" }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "pipelineRunOrError": { "__typename": "Run",
                    "pipelineName": "refresh_lakehouse",
                    "rootRunId": null,
                    "runConfig": { "ops": { "run_x": { "config": { "k": "v" } } } }
                } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client
            .launch_reexecution_of_steps("parent-1", &["run_x", "run_y"])
            .await
            .unwrap();
        assert_eq!(outcome.run_id.as_deref(), Some("new-run"));
        assert!(outcome.error.is_none());
    }

    /// F1.6 (PR #56 review BLOCKER): a second-level re-execution — a
    /// parent that is ITSELF a re-execution — already has a non-null
    /// `rootRunId` (the FIRST run's id, not the immediate parent's).
    /// That root is forwarded unchanged: it is the start of the chain,
    /// not the immediate parent. The mock lookup returns a non-null
    /// `rootRunId`; the mutation body must carry the SAME value (NOT the
    /// immediate parent's id), so the chain stays anchored at its origin.
    #[tokio::test]
    async fn launch_reexecution_of_steps_forwards_an_existing_root_unchanged() {
        use wiremock::matchers::{body_partial_json, body_string_contains};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("launchRunReexecution"))
            .and(body_partial_json(json!({
                "variables": {
                    "parentRunId": "parent-1",
                    "rootRunId": "root-9",
                }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "launchRunReexecution": { "__typename": "LaunchRunSuccess",
                    "run": { "runId": "new-run" } } }
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("pipelineRunOrError"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "pipelineRunOrError": { "__typename": "Run",
                    "pipelineName": "refresh_lakehouse",
                    "rootRunId": "root-9",
                    "runConfig": {}
                } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client
            .launch_reexecution_of_steps("parent-1", &["any"])
            .await
            .unwrap();
        assert_eq!(outcome.run_id.as_deref(), Some("new-run"));
        assert!(outcome.error.is_none());
    }

    /// F1.6 (PR #56 review BLOCKER): `parent.rootRunId` absent from the
    /// lookup payload entirely (not present at all — Dagster omits it for
    /// some historical run shapes, and the `parent.get("rootRunId")`
    /// chain must treat it the same as a JSON `null`). The mutation
    /// carries `rootRunId: parentRunId` for the same reason the explicit
    /// `null` does: a parent without a stored root IS its own root.
    #[tokio::test]
    async fn launch_reexecution_of_steps_falls_back_to_parent_when_root_field_is_absent() {
        use wiremock::matchers::{body_partial_json, body_string_contains};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("launchRunReexecution"))
            .and(body_partial_json(json!({
                "variables": {
                    "parentRunId": "parent-1",
                    "rootRunId": "parent-1",
                }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "launchRunReexecution": { "__typename": "LaunchRunSuccess",
                    "run": { "runId": "new-run" } } }
            })))
            .mount(&server)
            .await;
        // `rootRunId` is OMITTED from the payload — the `parent.get(
        // "rootRunId")` chain must yield `None` and `unwrap_or` to the
        // parent run id.
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("pipelineRunOrError"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "pipelineRunOrError": { "__typename": "Run",
                    "pipelineName": "refresh_lakehouse",
                    "runConfig": {}
                } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client
            .launch_reexecution_of_steps("parent-1", &["any"])
            .await
            .unwrap();
        assert_eq!(outcome.run_id.as_deref(), Some("new-run"));
        assert!(outcome.error.is_none());
    }

    /// The lookup of a non-`Run` parent (`RunNotFoundError`) is propagated
    /// as `Ok(LaunchOutcome { error, .. })`, not `Err` — matching
    /// [`DgClient::launch_run`] / [`DgClient::terminate_run`]'s posture
    /// (`DgClient::pipeline_run_status`'s sibling returns `Ok(None)` for
    /// the same condition; the route layer above is what maps that into a
    /// 404).
    #[tokio::test]
    async fn launch_reexecution_of_steps_reports_parent_not_found_as_error_not_err() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(wiremock::matchers::body_string_contains(
                "pipelineRunOrError",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "pipelineRunOrError": { "__typename": "RunNotFoundError" } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client
            .launch_reexecution_of_steps("nope", &["any"])
            .await
            .unwrap();
        assert!(outcome.run_id.is_none());
        assert!(outcome.error.is_some());
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

    /// Plan 1c (R2): each `stepStats` row carries an `attempts` list
    /// (`RunMarker { startTime endTime }` — `dagster_graphql/schema/logs/
    /// events.py:717`). Three attempts → `attempts: 3`. A step that
    /// never ran carries an empty `attempts` list → `attempts: 0` (never
    /// a fabricated `1`).
    #[tokio::test]
    async fn run_steps_parses_attempts_count() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "runOrError": { "__typename": "Run", "stepStats": [
                    { "stepKey": "retried_step", "status": "SUCCESS",
                      "startTime": 1.0, "endTime": 2.0,
                      "attempts": [
                          { "startTime": 1.0, "endTime": 1.5 },
                          { "startTime": 1.6, "endTime": 1.8 },
                          { "startTime": 1.9, "endTime": 2.0 }
                      ] },
                    { "stepKey": "never_started", "status": "SKIPPED",
                      "startTime": null, "endTime": null, "attempts": [] }
                ] } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let steps = client.run_steps("r1").await.unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].step_key, "retried_step");
        assert_eq!(steps[0].attempts, 3);
        assert_eq!(steps[1].step_key, "never_started");
        assert_eq!(
            steps[1].attempts, 0,
            "an empty attempts list means the step never started; never fabricate 1"
        );
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

    /// WS4 item C1: the fixture was recaptured (live, against this
    /// repository's own Dagster 1.13.20 stack) with the query extended to
    /// `definition { description metadata { key value } }` — live
    /// introspection showed `SolidDefinition.metadata` returns a plain
    /// `{key, value}` list sourced from the op's `tags`, NOT a
    /// `TextMetadataEntry` union as the plan sketch assumed (see
    /// `dagster/dispar_orchestrate/op_metadata.py`'s own deviation note).
    /// This test is written against the REAL shape.
    #[tokio::test]
    async fn job_graph_captures_source_ref_commit_and_sql_metadata_entries() {
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
        let op = graph
            .ops
            .iter()
            .find(|o| o.name == "run_bronze_maintenance")
            .unwrap();
        assert_eq!(
            op.source_ref.as_deref(),
            Some("dispar_orchestrate/maintenance.py::run_bronze_maintenance")
        );
        assert_eq!(op.commit.as_deref(), Some("unknown"));
        // This op passes no `sql=` to `source_metadata` (WS4 item B2 —
        // `maintenance.py:841` calls `source_metadata(..., sql=None)`), so
        // the real fixture carries no `sql` metadata key — `None`, not a
        // fabricated empty string.
        assert_eq!(op.sql, None);
        // The captured fixture predates `reads`/`writes`: absent entries
        // read as empty lists, never as a fabricated phrase.
        assert!(op.reads.is_empty() && op.writes.is_empty());
    }

    #[test]
    fn metadata_lines_split_a_newline_joined_entry_and_drop_blank_lines() {
        let entries = vec![MetadataItem {
            key: "writes".to_owned(),
            value: "Iceberg bronze.x\n\n ClickHouse lake.y ".to_owned(),
        }];
        assert_eq!(
            metadata_lines(&entries, "writes"),
            vec![
                "Iceberg bronze.x".to_owned(),
                "ClickHouse lake.y".to_owned()
            ]
        );
        assert!(metadata_lines(&entries, "reads").is_empty());
    }

    /// An op reporting metadata with none of the three recognized labels
    /// (should not happen post-Phase-B, but a foreign/future job's
    /// `tags`-derived metadata list is untrusted input to this client) must
    /// not panic, and reports all three fields `None`.
    #[tokio::test]
    async fn job_graph_op_with_no_recognized_metadata_labels_reports_none() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "pipelineOrError": { "__typename": "Pipeline", "solidHandles": [
                    { "solid": { "name": "foreign_op", "definition": { "description": null,
                        "metadata": [ { "key": "owner", "value": "someone" } ] }, "inputs": [] } }
                ] } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let graph = client.job_graph("some_job").await.unwrap();
        let op = &graph.ops[0];
        assert_eq!(op.source_ref, None);
        assert_eq!(op.commit, None);
        assert_eq!(op.sql, None);
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

    // ── F1.2 #57: `set_sensor_running` (sensor start/stop for the
    //    `authored__<id>_after` sensor that ships with chained authored
    //    pipelines). ────────────────────────────────────────────────

    /// Start path: `startSensor` returns `Sensor` (with
    /// `sensorState { status }`); the method maps that to
    /// `Ok(SensorOutcome { ok: true, .. })`.
    #[tokio::test]
    async fn set_sensor_running_start_returns_ok() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "startSensor": { "__typename": "Sensor",
                    "sensorState": { "status": "RUNNING" } } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client
            .set_sensor_running("authored__pl_down_1_after", true)
            .await
            .unwrap();
        assert!(outcome.ok, "start should succeed: {outcome:?}");
        assert!(outcome.error.is_none());
    }

    /// Stop path is a two-step lookup (the `stopSensor` mutation
    /// needs the `InstigationState.id`, NOT a `SensorSelector`). This
    /// test mocks both steps: the lookup returns a `Sensor` with a
    /// `sensorState.id`, and `stopSensor` returns
    /// `StopSensorMutationResult` with the new `instigationState`.
    #[tokio::test]
    async fn set_sensor_running_stop_returns_ok_after_two_step_lookup() {
        use wiremock::matchers::body_string_contains;

        let server = MockServer::start().await;
        // The stop path is TWO requests with different bodies: a lookup
        // (`sensorOrError`) then the mutation (`stopSensor(id: …)`). A
        // single mock answering both made the stop call's
        // `HashMap` deserialisation pick a value by iteration order, so
        // it could read `sensorOrError` (`__typename: "Sensor"`) and
        // report a spurious failure. Register the mutation mock FIRST so
        // it gets the higher priority (wiremock uses "most recently
        // mounted first" — see the docs on `MockServer`). Then mount the
        // lookup mock; the two body matchers are disjoint, so neither can
        // shadow the other.
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("stopSensor"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "stopSensor": {
                        "__typename": "StopSensorMutationResult",
                        "instigationState": { "id": "remote_origin_id:selector_id", "status": "STOPPED" }
                    }
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("sensorOrError"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {
                    "sensorOrError": {
                        "__typename": "Sensor",
                        "sensorState": { "id": "remote_origin_id:selector_id", "status": "RUNNING" }
                    }
                }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client
            .set_sensor_running("authored__pl_down_1_after", false)
            .await
            .unwrap();
        assert!(outcome.ok, "stop should succeed: {outcome:?}");
        assert!(outcome.error.is_none());
    }

    /// F1.2 tolerance: when the sensor does not exist at all (e.g. a
    /// pipeline whose `dependsOn` is empty, so no sensor was built),
    /// the lookup step returns `SensorNotFoundError` and the route's
    /// `paused` branch treats `running=false` as already-stopped.
    /// `set_sensor_running(_, false)` MUST return `Ok(true)` so the
    /// toggle stays idempotent across reloads.
    #[tokio::test]
    async fn set_sensor_running_stop_is_already_silent_when_sensor_is_missing() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "sensorOrError": { "__typename": "SensorNotFoundError",
                    "message": "no such sensor" } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client
            .set_sensor_running("authored__pl_down_1_after", false)
            .await
            .unwrap();
        assert!(
            outcome.ok,
            "stop on missing sensor -> already silent: {outcome:?}"
        );
        assert!(
            outcome.error.is_none(),
            "missing sensor must NOT surface an error: {outcome:?}"
        );
    }

    /// Start path: `startSensor` returning `SensorNotFoundError`
    /// (e.g. the sensor name is wrong) maps to `Ok(false)` with the
    /// message in `error`. The route's `resume` branch uses the
    /// `error` field to surface a 4xx to the caller.
    #[tokio::test]
    async fn set_sensor_running_start_reports_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "startSensor": { "__typename": "SensorNotFoundError",
                    "message": "no such sensor" } }
            })))
            .mount(&server)
            .await;

        let client = DgClient::new(format!("{}/graphql", server.uri()));
        let outcome = client
            .set_sensor_running("authored__pl_down_1_after", true)
            .await
            .unwrap();
        assert!(!outcome.ok);
        assert_eq!(outcome.error.as_deref(), Some("no such sensor"));
    }
}
