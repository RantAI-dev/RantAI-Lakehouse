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
                      schedules { cronSchedule scheduleState { status } jobName: pipelineName } \
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
        let r = data.launch_run;
        if r.typename == "LaunchRunSuccess" {
            if let Some(run) = r.run {
                return Ok(LaunchOutcome {
                    run_id: Some(run.run_id),
                    error: None,
                });
            }
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
        Ok(LaunchOutcome {
            run_id: None,
            error: Some(error),
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
                        { "cronSchedule": "0 3 * * *", "scheduleState": { "status": "RUNNING" }, "jobName": "refresh_lakehouse" }
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
}
