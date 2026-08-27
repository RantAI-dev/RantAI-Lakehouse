//! Minimal `Dagster` GraphQL client, porting `src/services/clients/dagster.ts`.
//!
//! Only the read-only operations the five ported domains actually call —
//! `listRuns` and `listJobs`, both always invoked without a `jobName`
//! filter — are implemented here. `launchRun` (a mutation) has no caller
//! among the read-only routes ported so far and is left for a later task
//! that ports the write-side pipeline routes.

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
}

/// Errors produced while talking to `Dagster`.
#[derive(Debug, Error)]
pub enum DgError {
    /// A transport-level failure surfaced by `reqwest`.
    #[error(transparent)]
    Transport(#[from] reqwest::Error),
    /// `Dagster` responded with a non-2xx status or a GraphQL error.
    #[error("{0}")]
    Server(String),
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
}

#[derive(Debug, Deserialize)]
struct JobName {
    name: String,
}

/// HTTP client for `Dagster`'s GraphQL endpoint.
pub struct DgClient {
    client: Client,
    url: String,
}

impl DgClient {
    /// Build a client targeting `url` (the `Dagster` GraphQL endpoint).
    #[must_use]
    pub fn new(url: String) -> Self {
        Self {
            client: Client::new(),
            url,
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
        let data: RunsOrErrorData = self.execute(&query).await?;
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
        let data: ReposOrErrorData = self.execute(query).await?;
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

    /// Whether the `Dagster` GraphQL endpoint is reachable, checked via its
    /// `/server_info` REST endpoint with a 3-second timeout — matching the
    /// `check("dagster", dagUrl)` helper in
    /// `src/app/api/ops/[kind]/route.ts`.
    // Used starting with the `ops` commit; harmless to land ahead of its
    // first caller since it's part of this client's public surface.
    #[allow(dead_code)]
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

    async fn execute<T: for<'de> Deserialize<'de>>(&self, query: &str) -> Result<T, DgError> {
        let resp = self
            .client
            .post(&self.url)
            .json(&json!({ "query": query }))
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(DgError::Server(format!("Dagster HTTP {}", status.as_u16())));
        }
        let parsed: GqlResponse<T> =
            serde_json::from_str(&text).map_err(|e| DgError::Server(e.to_string()))?;
        if let Some(errors) = parsed.errors {
            return Err(DgError::Server(errors.to_string()));
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
        // 2026-08-27T04:00:10.075Z, verified against overview-refresh.json.
        let seconds = 1_787_803_210.075_f64;
        assert_eq!(iso_from_unix_seconds(seconds), "2026-08-27T04:00:10.075Z");
    }

    #[test]
    fn iso_from_unix_seconds_zero_is_epoch() {
        assert_eq!(iso_from_unix_seconds(0.0), "1970-01-01T00:00:00.000Z");
    }
}
