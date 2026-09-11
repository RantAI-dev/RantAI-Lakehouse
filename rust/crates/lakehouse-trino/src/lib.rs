//! A thin client for `Trino`'s `/v1/statement` REST protocol.
//!
//! `Trino` has no official Rust SDK, so this crate speaks its documented
//! statement-submission protocol directly over `reqwest`: `POST` a SQL
//! statement, then follow the `nextUri` link the server hands back with
//! `GET` requests until it stops returning one. This mirrors how
//! `lakehouse-clickhouse` talks to `ClickHouse`'s plain HTTP interface with
//! no vendor SDK.
//!
//! The public surface is kept intentionally small —
//! [`TrinoConfig`], [`TrinoClient::new`], [`TrinoClient::run_statement`],
//! [`TrinoResult`], [`TrinoColumn`] and [`TrinoError`] — so a future
//! "consumers" route can depend on this crate without depending on any
//! other change in this workstream.
//!
//! [`TrinoError::Query`] carries `Trino`'s own error message verbatim. That
//! text is safe to forward to an HTTP response ONLY at the one call site
//! where the query's own author is reading back their own SQL error; every
//! other caller of this crate must classify it into a fixed message before
//! it reaches a response.

use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;
use tokio::time::Instant;

/// Configuration for a [`TrinoClient`].
pub struct TrinoConfig {
    /// The base URL of the `Trino` coordinator, e.g. `http://trino:8080`.
    pub base_url: String,
    /// The maximum number of result rows to accumulate across pages before
    /// [`TrinoClient::run_statement`] cancels the query and returns
    /// [`TrinoError::TooManyRows`].
    pub max_rows: usize,
    /// The wall-clock budget for following `nextUri` pages, starting from
    /// the initial `POST`. Every HTTP request this client makes is given a
    /// per-request timeout bounded by the time remaining before this
    /// deadline, so a single hung request cannot outlive the cap either.
    pub paging_cap: Duration,
}

impl TrinoConfig {
    /// Builds a config with the documented defaults: `max_rows` of 10,000
    /// and a `paging_cap` of 60 seconds. Tests that need a short cap or a
    /// small row cap override the fields directly.
    #[must_use]
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            max_rows: 10_000,
            paging_cap: Duration::from_secs(60),
        }
    }
}

/// A client for `Trino`'s `/v1/statement` REST protocol.
pub struct TrinoClient {
    http: reqwest::Client,
    config: TrinoConfig,
}

/// Errors [`TrinoClient::run_statement`] can return.
#[derive(Debug, Error)]
pub enum TrinoError {
    /// The HTTP request itself failed (DNS, connection refused, a timeout
    /// with no application-level response). Carries no upstream text.
    #[error("trino request failed")]
    Transport(#[source] reqwest::Error),
    /// `Trino`'s own error message, forwarded verbatim. Safe to surface to
    /// a caller ONLY at the one call site this crate's consumers document
    /// as authorized (the query's own author reading back their own SQL
    /// error). Every other caller MUST classify this variant into a fixed
    /// message before it reaches a response.
    #[error("{0}")]
    Query(String),
    /// The `paging_cap` deadline passed, either between pages or as a
    /// per-request timeout on an in-flight request.
    #[error("trino query exceeded the paging cap")]
    Timeout,
    /// A non-2xx response that is not retryable (any status other than a
    /// `503`, which this client retries per `Trino`'s client protocol).
    /// Carries only the status code, never the response body.
    #[error("trino returned HTTP status {0}")]
    HttpStatus(u16),
    /// Paging stopped because the accumulated row count would have
    /// exceeded `cap`. The query was cancelled on `Trino` on a best-effort
    /// basis before this error was returned.
    #[error("trino result exceeded the {cap}-row cap")]
    TooManyRows {
        /// The `max_rows` cap that was exceeded.
        cap: usize,
    },
}

impl From<reqwest::Error> for TrinoError {
    fn from(err: reqwest::Error) -> Self {
        map_reqwest_err(err)
    }
}

/// Maps a `reqwest` error to a [`TrinoError`]. A timeout — whether from the
/// per-request `.timeout(remaining)` this client sets on every request, or
/// any other `reqwest`-detected timeout — is a deadline breach, not a
/// generic transport failure, so it becomes [`TrinoError::Timeout`] rather
/// than [`TrinoError::Transport`].
fn map_reqwest_err(err: reqwest::Error) -> TrinoError {
    if err.is_timeout() {
        TrinoError::Timeout
    } else {
        TrinoError::Transport(err)
    }
}

/// A `Trino`-reported error object, present in a statement response page
/// when the query failed.
#[derive(Debug, Deserialize)]
struct TrinoErrorBody {
    message: String,
}

/// A single column descriptor from `Trino`'s `/v1/statement` envelope.
#[derive(Debug, Deserialize)]
struct TrinoColumnRaw {
    name: String,
    #[serde(rename = "type")]
    type_signature: String,
}

/// One page of `Trino`'s `/v1/statement` response.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct TrinoStatementResponse {
    #[serde(default)]
    columns: Vec<TrinoColumnRaw>,
    #[serde(default)]
    data: Vec<Vec<serde_json::Value>>,
    #[serde(default)]
    next_uri: Option<String>,
    #[serde(default)]
    error: Option<TrinoErrorBody>,
}

/// A column descriptor in a [`TrinoResult`].
#[derive(Debug, Clone)]
pub struct TrinoColumn {
    /// The column name.
    pub name: String,
    /// `Trino`'s type signature for the column (e.g. `"bigint"`).
    pub type_signature: String,
}

/// The accumulated result of a `Trino` statement, once all pages have been
/// followed.
#[derive(Debug, Clone)]
pub struct TrinoResult {
    /// The result columns, taken from the first page that carried them.
    pub columns: Vec<TrinoColumn>,
    /// Every row across every page, in order.
    pub rows: Vec<Vec<serde_json::Value>>,
}

/// A short, best-effort timeout for the cancellation `DELETE` this client
/// sends on timeout, row-cap breach, or a mid-paging error. Its result is
/// ignored either way: cancellation is a courtesy to `Trino`, not something
/// this client's callers can depend on succeeding.
const CANCEL_TIMEOUT: Duration = Duration::from_secs(2);

/// How long to sleep before retrying an HTTP 503, which `Trino`'s client
/// protocol documents as a request the client should retry.
const RETRY_503_DELAY: Duration = Duration::from_millis(75);

impl TrinoClient {
    /// Builds a client for the given configuration.
    #[must_use]
    pub fn new(config: TrinoConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            config,
        }
    }

    /// Runs `sql` against `Trino` as `trino_user`, following `nextUri`
    /// pages until the server stops returning one.
    ///
    /// `trino_user` is sent verbatim as the `X-Trino-User` header; callers
    /// are responsible for deriving it from an authenticated identity, not
    /// from unauthenticated caller input.
    ///
    /// # Errors
    /// [`TrinoError::Transport`] on a network failure; [`TrinoError::Query`]
    /// when `Trino` reports a query error; [`TrinoError::Timeout`] when the
    /// `paging_cap` deadline passes; [`TrinoError::HttpStatus`] on a
    /// non-2xx, non-503 response; [`TrinoError::TooManyRows`] when the
    /// accumulated rows would exceed `max_rows`.
    pub async fn run_statement(
        &self,
        sql: &str,
        trino_user: &str,
    ) -> Result<TrinoResult, TrinoError> {
        let deadline = Instant::now() + self.config.paging_cap;

        let mut resp = self.post_statement(sql, trino_user, deadline).await?;

        let mut columns: Vec<TrinoColumn> = Vec::new();
        let mut rows: Vec<Vec<serde_json::Value>> = Vec::new();
        // The URI of the page most recently fetched via `nextUri`. This is
        // the query's own current-state URI on `Trino`, so it is the right
        // cancellation target when the page that just arrived is the last
        // one (carries no `nextUri` of its own) but still needs cancelling
        // (for example because it pushed the row count over `max_rows`).
        let mut last_fetched_uri: Option<String> = None;

        loop {
            if let Some(err) = resp.error.take() {
                self.cancel_if_present(resp.next_uri.as_deref().or(last_fetched_uri.as_deref()))
                    .await;
                return Err(TrinoError::Query(err.message));
            }

            if columns.is_empty() && !resp.columns.is_empty() {
                columns = resp
                    .columns
                    .drain(..)
                    .map(|c| TrinoColumn {
                        name: c.name,
                        type_signature: c.type_signature,
                    })
                    .collect();
            }

            if rows.len() + resp.data.len() > self.config.max_rows {
                self.cancel_if_present(resp.next_uri.as_deref().or(last_fetched_uri.as_deref()))
                    .await;
                return Err(TrinoError::TooManyRows {
                    cap: self.config.max_rows,
                });
            }
            rows.append(&mut resp.data);

            let Some(next_uri) = resp.next_uri.take() else {
                break;
            };

            match self.get_page(&next_uri, deadline).await {
                Ok(page) => {
                    resp = page;
                    last_fetched_uri = Some(next_uri);
                }
                Err(err) => {
                    self.cancel(&next_uri).await;
                    return Err(err);
                }
            }
        }

        Ok(TrinoResult { columns, rows })
    }

    /// Sends the initial `POST /v1/statement`, retrying a `503` response
    /// per `Trino`'s client protocol (which asks clients to retry that
    /// status) while the deadline allows. Any other non-2xx status becomes
    /// [`TrinoError::HttpStatus`].
    async fn post_statement(
        &self,
        sql: &str,
        trino_user: &str,
        deadline: Instant,
    ) -> Result<TrinoStatementResponse, TrinoError> {
        loop {
            let remaining = remaining_or_timeout(deadline)?;
            let response = self
                .http
                .post(format!("{}/v1/statement", self.config.base_url))
                .header("X-Trino-User", trino_user)
                .header("Content-Type", "text/plain; charset=utf-8")
                .timeout(remaining)
                .body(sql.to_owned())
                .send()
                .await?;

            let status = response.status();
            if status.as_u16() == 503 {
                if Instant::now() >= deadline {
                    return Err(TrinoError::Timeout);
                }
                tokio::time::sleep(RETRY_503_DELAY).await;
                continue;
            }
            if !status.is_success() {
                return Err(TrinoError::HttpStatus(status.as_u16()));
            }
            return Ok(response.json().await?);
        }
    }

    /// Follows one `nextUri` page with a `GET`, retrying a `503` the same
    /// way the initial `POST` does.
    async fn get_page(
        &self,
        uri: &str,
        deadline: Instant,
    ) -> Result<TrinoStatementResponse, TrinoError> {
        loop {
            let remaining = remaining_or_timeout(deadline)?;
            let response = self.http.get(uri).timeout(remaining).send().await?;

            let status = response.status();
            if status.as_u16() == 503 {
                if Instant::now() >= deadline {
                    return Err(TrinoError::Timeout);
                }
                tokio::time::sleep(RETRY_503_DELAY).await;
                continue;
            }
            if !status.is_success() {
                return Err(TrinoError::HttpStatus(status.as_u16()));
            }
            return Ok(response.json().await?);
        }
    }

    /// Sends a best-effort `DELETE {uri}` if `uri` is present, ignoring the
    /// result entirely: cancellation is a courtesy to `Trino`, never
    /// something this client's return value depends on.
    async fn cancel_if_present(&self, uri: Option<&str>) {
        if let Some(uri) = uri {
            self.cancel(uri).await;
        }
    }

    /// Sends a best-effort `DELETE {uri}` with a short timeout, ignoring
    /// both transport errors and non-2xx responses.
    async fn cancel(&self, uri: &str) {
        let _ = self.http.delete(uri).timeout(CANCEL_TIMEOUT).send().await;
    }
}

/// Returns the time remaining before `deadline`, or [`TrinoError::Timeout`]
/// if none remains.
fn remaining_or_timeout(deadline: Instant) -> Result<Duration, TrinoError> {
    let now = Instant::now();
    if now >= deadline {
        Err(TrinoError::Timeout)
    } else {
        Ok(deadline - now)
    }
}

#[cfg(test)]
mod tests {
    #![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn config(server: &MockServer) -> TrinoConfig {
        TrinoConfig::new(server.uri())
    }

    #[tokio::test]
    async fn run_statement_follows_next_uri_across_two_pages_and_collects_columns() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/statement"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "q1",
                "nextUri": format!("{}/v1/statement/q1/2", server.uri()),
                "columns": [{"name": "n", "type": "bigint"}],
                "data": [[1]],
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/statement/q1/2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "q1",
                "data": [[2]],
            })))
            .mount(&server)
            .await;

        let client = TrinoClient::new(config(&server));
        let result = client
            .run_statement("SELECT n FROM t", "alice")
            .await
            .expect("ok");

        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.columns.len(), 1);
        assert_eq!(result.columns[0].name, "n");
        assert_eq!(result.columns[0].type_signature, "bigint");
    }

    #[tokio::test]
    async fn run_statement_maps_a_trino_error_object_to_query_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/statement"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "q1",
                "error": {"message": "line 1:1: mismatched input", "errorCode": 1, "errorName": "SYNTAX_ERROR"},
            })))
            .mount(&server)
            .await;

        let client = TrinoClient::new(config(&server));
        let err = client
            .run_statement("SELECT", "alice")
            .await
            .expect_err("should fail");

        assert!(matches!(err, TrinoError::Query(msg) if msg.contains("mismatched input")));
    }

    #[tokio::test]
    async fn run_statement_times_out_and_cancels_on_a_slow_page() {
        let server = MockServer::start().await;
        let next_uri = format!("{}/v1/statement/q1/2", server.uri());
        Mock::given(method("POST"))
            .and(path("/v1/statement"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "q1",
                "nextUri": next_uri,
                "data": [[1]],
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/statement/q1/2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(1))
                    .set_body_json(serde_json::json!({ "id": "q1", "data": [[2]] })),
            )
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/v1/statement/q1/2"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let mut cfg = config(&server);
        cfg.paging_cap = Duration::from_millis(200);
        let client = TrinoClient::new(cfg);

        let err = client
            .run_statement("SELECT n FROM t", "alice")
            .await
            .expect_err("should time out");
        assert!(matches!(err, TrinoError::Timeout));

        let received = server
            .received_requests()
            .await
            .expect("mock server records requests");
        assert!(
            received
                .iter()
                .any(|r| r.method.as_str() == "DELETE" && r.url.path() == "/v1/statement/q1/2")
        );
    }

    #[tokio::test]
    async fn run_statement_stops_and_cancels_when_the_row_cap_is_exceeded() {
        let server = MockServer::start().await;
        let next_uri = format!("{}/v1/statement/q1/2", server.uri());
        Mock::given(method("POST"))
            .and(path("/v1/statement"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "q1",
                "nextUri": next_uri,
                "columns": [{"name": "n", "type": "bigint"}],
                "data": [[1]],
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/statement/q1/2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "q1",
                "data": [[2]],
            })))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/v1/statement/q1/2"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let mut cfg = config(&server);
        cfg.max_rows = 1;
        let client = TrinoClient::new(cfg);

        let err = client
            .run_statement("SELECT n FROM t", "alice")
            .await
            .expect_err("should cap rows");
        assert!(matches!(err, TrinoError::TooManyRows { cap: 1 }));

        let received = server
            .received_requests()
            .await
            .expect("mock server records requests");
        assert!(
            received
                .iter()
                .any(|r| r.method.as_str() == "DELETE" && r.url.path() == "/v1/statement/q1/2")
        );
    }

    #[tokio::test]
    async fn run_statement_retries_a_503_and_then_succeeds() {
        let server = MockServer::start().await;
        let next_uri = format!("{}/v1/statement/q1/2", server.uri());
        Mock::given(method("POST"))
            .and(path("/v1/statement"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "q1",
                "nextUri": next_uri,
                "columns": [{"name": "n", "type": "bigint"}],
                "data": [[1]],
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/statement/q1/2"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/statement/q1/2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "q1",
                "data": [[2]],
            })))
            .mount(&server)
            .await;

        let client = TrinoClient::new(config(&server));
        let result = client
            .run_statement("SELECT n FROM t", "alice")
            .await
            .expect("should succeed after retry");

        assert_eq!(result.rows.len(), 2);
    }

    #[tokio::test]
    async fn run_statement_maps_a_non_retryable_status_to_http_status() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/statement"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = TrinoClient::new(config(&server));
        let err = client
            .run_statement("SELECT 1", "alice")
            .await
            .expect_err("should fail");

        assert!(matches!(err, TrinoError::HttpStatus(500)));
    }

    #[tokio::test]
    async fn run_statement_sends_the_x_trino_user_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/statement"))
            .and(header("X-Trino-User", "alice"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "q1",
                "columns": [{"name": "n", "type": "bigint"}],
                "data": [[1]],
            })))
            .mount(&server)
            .await;

        let client = TrinoClient::new(config(&server));
        let result = client
            .run_statement("SELECT n FROM t", "alice")
            .await
            .expect("ok");

        assert_eq!(result.rows.len(), 1);
    }

    #[tokio::test]
    async fn transport_error_display_never_carries_upstream_text() {
        // A closed local port refuses the connection before any
        // application-level response exists, exercising the real
        // `reqwest::Error` -> `TrinoError::Transport` path.
        let client = TrinoClient::new(TrinoConfig::new("http://127.0.0.1:1"));
        let err = client
            .run_statement("SELECT 1", "alice")
            .await
            .expect_err("connection should be refused");

        assert!(matches!(err, TrinoError::Transport(_)));
        assert_eq!(err.to_string(), "trino request failed");
    }
}
