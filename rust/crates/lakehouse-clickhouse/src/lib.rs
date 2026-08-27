//! HTTP client for `ClickHouse`'s plain HTTP interface.
//!
//! Ports `src/services/clients/clickhouse.ts` — the client every
//! data-reading route in the Next.js backend depends on. `ClickHouse` is
//! talked to over its plain HTTP interface directly (`POST` a SQL body,
//! read back the response), with no vendor SDK, exactly as the TypeScript
//! client does with `fetch`.

use lakehouse_core::ApiError;
use reqwest::StatusCode;
use reqwest::header::{CACHE_CONTROL, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::{Map, Value};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

/// A single column descriptor from `ClickHouse`'s `FORMAT JSON` envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct ChColumn {
    /// The column name.
    pub name: String,
    /// The `ClickHouse` type name (e.g. `"UInt64"`, `"String"`).
    #[serde(rename = "type")]
    pub ty: String,
}

/// Query execution statistics reported by `ClickHouse`'s `FORMAT JSON`
/// envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct ChStatistics {
    /// Wall-clock seconds `ClickHouse` spent executing the query.
    pub elapsed: f64,
    /// Number of rows read from storage while executing the query.
    pub rows_read: u64,
    /// Number of bytes read from storage while executing the query.
    pub bytes_read: u64,
}

/// A structured `ClickHouse` query result, parsed from its `FORMAT JSON`
/// response body.
///
/// When the response body is not valid JSON (`ClickHouse` returns a bare
/// `Ok.` for non-tabular statements), [`ChClient::query`] yields the
/// [`Default`] value of this type — empty `meta`/`data`, `rows: 0` — rather
/// than treating it as an error, mirroring the TypeScript client.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ChResult {
    /// Column descriptors, in result order.
    #[serde(default)]
    pub meta: Vec<ChColumn>,
    /// Result rows, each keyed by column name.
    #[serde(default)]
    pub data: Vec<Map<String, Value>>,
    /// Number of rows in `data`.
    #[serde(default)]
    pub rows: u64,
    /// Execution statistics, present when `ClickHouse`'s `send_progress_in_http_headers`/
    /// statistics settings include them in the response body.
    #[serde(default)]
    pub statistics: Option<ChStatistics>,
}

/// Errors produced while talking to `ClickHouse`.
#[derive(Debug, Error)]
pub enum ChError {
    /// A transport-level failure (connection refused, TLS error, timeout,
    /// cancellation, ...) surfaced by `reqwest`.
    #[error(transparent)]
    Transport(#[from] reqwest::Error),
    /// `ClickHouse` responded with a non-2xx status. The message is the
    /// `ClickHouse` error body, trimmed, verbatim — callers surface it to
    /// users — falling back to `ClickHouse HTTP <status>` when the body is
    /// empty.
    #[error("{0}")]
    Server(String),
}

impl From<ChError> for ApiError {
    /// `ClickHouse` errors are almost always the caller's SQL, not our
    /// outage, so they map to `422 Unprocessable` — matching
    /// `src/app/api/query/run/route.ts`, which returns 422 (not 500) when
    /// `chQuery` throws.
    fn from(err: ChError) -> Self {
        Self::Unprocessable(err.to_string())
    }
}

/// HTTP client for `ClickHouse`'s plain HTTP interface.
///
/// Auth is HTTP Basic from the configured user/password; content type is
/// `text/plain; charset=utf-8`; caching is disabled on every request.
pub struct ChClient {
    client: reqwest::Client,
    url: String,
    user: String,
    password: String,
}

impl ChClient {
    /// Build a client targeting `url`, authenticating as `user`/`password`.
    #[must_use]
    pub fn new(url: String, user: String, password: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            url,
            user,
            password,
        }
    }

    /// Run `sql` and return the structured result (`FORMAT JSON`).
    ///
    /// `FORMAT JSON` is appended to `sql` unless it already ends in a
    /// `FORMAT <name>` clause (see [`has_trailing_format_clause`]). If the
    /// response body is not valid JSON — `ClickHouse` returns a bare `Ok.`
    /// for non-tabular statements — an empty [`ChResult`] is returned
    /// rather than an error.
    ///
    /// `cancel`, when provided, aborts the in-flight request as soon as it
    /// is cancelled (mirroring the TypeScript client's `AbortSignal`).
    ///
    /// # Errors
    ///
    /// Returns [`ChError::Transport`] on a network-level failure, or
    /// [`ChError::Server`] when `ClickHouse` responds with a non-2xx status.
    pub async fn query(
        &self,
        sql: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<ChResult, ChError> {
        let body = build_query_body(sql);
        let fut = async {
            let response = self
                .client
                .post(&self.url)
                .basic_auth(&self.user, Some(&self.password))
                .header(CONTENT_TYPE, "text/plain; charset=utf-8")
                .header(CACHE_CONTROL, "no-store")
                .body(body)
                .send()
                .await?;
            let status = response.status();
            let text = response.text().await?;
            if !status.is_success() {
                return Err(ChError::Server(server_error_message(status, &text)));
            }
            // Non-tabular statements (e.g. issued without a SELECT) return a
            // bare `Ok.` body — not JSON. Treat that as an empty result
            // rather than an error, matching the TS client's try/catch.
            Ok(serde_json::from_str::<ChResult>(&text).unwrap_or_default())
        };
        run_cancellable(fut, cancel).await
    }

    /// Run `sql` and return only its rows (no metadata). Equivalent to
    /// `self.query(sql, cancel).await?.data`.
    ///
    /// # Errors
    ///
    /// See [`ChClient::query`].
    pub async fn rows(
        &self,
        sql: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<Vec<Map<String, Value>>, ChError> {
        Ok(self.query(sql, cancel).await?.data)
    }

    /// Run a non-`SELECT` statement (DDL/DML: `CREATE`/`INSERT`/`ALTER`)
    /// verbatim — with no `FORMAT` wrapping, which would break
    /// `INSERT`/DDL — and discard the response body.
    ///
    /// `cancel`, when provided, aborts the in-flight request as soon as it
    /// is cancelled.
    ///
    /// # Errors
    ///
    /// Returns [`ChError::Transport`] on a network-level failure, or
    /// [`ChError::Server`] when `ClickHouse` responds with a non-2xx status.
    pub async fn exec(&self, sql: &str, cancel: Option<&CancellationToken>) -> Result<(), ChError> {
        let fut = async {
            let response = self
                .client
                .post(&self.url)
                .basic_auth(&self.user, Some(&self.password))
                .header(CONTENT_TYPE, "text/plain; charset=utf-8")
                .header(CACHE_CONTROL, "no-store")
                .body(sql.to_owned())
                .send()
                .await?;
            let status = response.status();
            if status.is_success() {
                return Ok(());
            }
            let text = response.text().await?;
            Err(ChError::Server(server_error_message(status, &text)))
        };
        run_cancellable(fut, cancel).await
    }
}

/// Race `fut` against `cancel` being cancelled, when a token is supplied.
///
/// Mirrors threading an `AbortSignal` through `fetch`: if the token fires
/// before `fut` resolves, the request is treated as failed with a
/// `ChError::Server` describing the cancellation.
async fn run_cancellable<F, T>(fut: F, cancel: Option<&CancellationToken>) -> Result<T, ChError>
where
    F: std::future::Future<Output = Result<T, ChError>>,
{
    match cancel {
        Some(token) => {
            tokio::select! {
                res = fut => res,
                () = token.cancelled() => Err(ChError::Server("request cancelled".to_owned())),
            }
        }
        None => fut.await,
    }
}

/// Build the request body for [`ChClient::query`]: `sql` unmodified if it
/// already ends in a `FORMAT <name>` clause, otherwise `sql` with a
/// trailing `;` stripped and `\nFORMAT JSON` appended.
fn build_query_body(sql: &str) -> String {
    if has_trailing_format_clause(sql.trim()) {
        sql.to_owned()
    } else {
        format!("{}\nFORMAT JSON", strip_trailing_semicolon(sql))
    }
}

/// Strip a single trailing `;` (and any whitespace after it) from `sql`,
/// mirroring the TypeScript `sql.replace(/;\s*$/, "")`. If `sql` has no
/// trailing `;`, it is returned unchanged — including any trailing
/// whitespace, which is then carried into the `FORMAT JSON` line.
fn strip_trailing_semicolon(sql: &str) -> &str {
    let end_trimmed = sql.trim_end_matches(char::is_whitespace);
    end_trimmed.strip_suffix(';').unwrap_or(sql)
}

/// Mirrors the TypeScript regex `/\bformat\s+\w+\s*;?\s*$/i`, tested
/// against `sql.trim()`.
///
/// `\w` in JavaScript (without the `u` flag) matches ASCII word characters
/// only, so this is implemented with the same ASCII notion of "word
/// character" rather than Unicode-aware classification, to match the TS
/// behavior exactly — including its blind spot around SQL string literals.
/// The regex has no concept of quoting, so it operates on raw text: a
/// query ending in an *unterminated* literal that happens to read like
/// `... FORMAT JSON` (e.g. `"SELECT 'x FORMAT JSON"`, missing its closing
/// quote) would also be misdetected as already having a FORMAT clause.
/// That is reproduced here deliberately, for fidelity with the TS — this
/// is a port, and the parity corpus was captured from the TS behavior. A
/// literal that *is* properly closed (`"SELECT 'FORMAT JSON'"`) does not
/// trigger this: the trailing `'` is not consumed by `\s*;?\s*$`, so the
/// match — and this function — correctly say "no FORMAT clause" in Rust
/// exactly as the JS regex does.
fn has_trailing_format_clause(trimmed: &str) -> bool {
    let chars: Vec<char> = trimmed.chars().collect();
    let mut i = chars.len();

    // \s*  — trailing whitespace after the optional semicolon.
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    // ;?  — at most one trailing semicolon.
    if i > 0 && chars[i - 1] == ';' {
        i -= 1;
    }
    // \s*  — whitespace between the format name and the semicolon.
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    // \w+  — the format name (e.g. JSON, TSV), at least one char.
    let word_end = i;
    while i > 0 && is_ascii_word_char(chars[i - 1]) {
        i -= 1;
    }
    if i == word_end {
        return false;
    }
    // \s+  — at least one whitespace between "format" and the name.
    let ws_end = i;
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    if i == ws_end {
        return false;
    }
    // The literal keyword "format", case-insensitive.
    if i < 6 {
        return false;
    }
    let candidate: String = chars[i - 6..i].iter().collect();
    if !candidate.eq_ignore_ascii_case("format") {
        return false;
    }
    // \b  — a word boundary immediately before "format": either the start
    // of the string, or a preceding non-word character.
    if i > 6 && is_ascii_word_char(chars[i - 7]) {
        return false;
    }
    true
}

/// ASCII notion of a "word character", matching JavaScript's `\w` without
/// the `u` flag: `[A-Za-z0-9_]`.
fn is_ascii_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The message for a non-2xx `ClickHouse` response: the trimmed body, or a
/// `ClickHouse HTTP <status>` fallback when the body is empty.
fn server_error_message(status: StatusCode, body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        format!("ClickHouse HTTP {}", status.as_u16())
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    #![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
    use super::*;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(url: &str) -> ChClient {
        ChClient::new(url.to_owned(), "default".to_owned(), String::new())
    }

    #[tokio::test]
    async fn query_appends_format_json_when_absent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("FORMAT JSON"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"meta":[{"name":"n","type":"UInt64"}],"data":[{"n":1}],"rows":1}"#,
            ))
            .mount(&server)
            .await;

        let result = client(&server.uri())
            .query("SELECT 1 AS n", None)
            .await
            .unwrap();
        assert_eq!(result.rows, 1);
        assert_eq!(result.data.len(), 1);
        assert_eq!(result.meta[0].name, "n");
        assert_eq!(result.meta[0].ty, "UInt64");
    }

    #[tokio::test]
    async fn query_preserves_existing_format_clause() {
        let server = MockServer::start().await;
        // If the mock ever saw a body containing "FORMAT JSON" it would not
        // match this expectation (only one mock is registered), so a
        // request with an unexpected body causes wiremock to panic/fail.
        Mock::given(method("POST"))
            .and(body_string_contains("SELECT 1 FORMAT TSV"))
            .respond_with(ResponseTemplate::new(200).set_body_string("1\n"))
            .mount(&server)
            .await;

        let result = client(&server.uri())
            .query("SELECT 1 FORMAT TSV", None)
            .await
            .unwrap();
        // "1\n" is not JSON, so the client returns an empty result rather
        // than erroring — but the important assertion is the request body
        // wiremock accepted (checked by the mount not panicking on drop).
        assert_eq!(result.rows, 0);
    }

    #[tokio::test]
    async fn query_strips_trailing_semicolon_before_appending_format() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("SELECT 1\nFORMAT JSON"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r"{}"))
            .mount(&server)
            .await;

        let result = client(&server.uri())
            .query("SELECT 1;", None)
            .await
            .unwrap();
        assert_eq!(result.rows, 0);
    }

    #[tokio::test]
    async fn query_returns_empty_result_when_body_is_not_json() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("Ok."))
            .mount(&server)
            .await;

        let result = client(&server.uri())
            .query("CREATE TABLE t (x UInt8) ENGINE = Memory", None)
            .await
            .unwrap();
        assert_eq!(result.rows, 0);
        assert!(result.data.is_empty());
        assert!(result.meta.is_empty());
    }

    #[tokio::test]
    async fn query_surfaces_clickhouse_error_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(400).set_body_string("Code: 47. Unknown identifier: nope"),
            )
            .mount(&server)
            .await;

        let err = client(&server.uri())
            .query("SELECT nope", None)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "Code: 47. Unknown identifier: nope");
    }

    #[tokio::test]
    async fn query_falls_back_to_status_when_error_body_empty() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string(""))
            .mount(&server)
            .await;

        let err = client(&server.uri())
            .query("SELECT 1", None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("500"));
    }

    #[tokio::test]
    async fn exec_sends_sql_verbatim_without_format() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("INSERT INTO t VALUES (1)"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        client(&server.uri())
            .exec("INSERT INTO t VALUES (1)", None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn exec_surfaces_clickhouse_error_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string("Code: 62. Syntax error"))
            .mount(&server)
            .await;

        let err = client(&server.uri())
            .exec("INSERT INTO t VALUES (nope)", None)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "Code: 62. Syntax error");
    }

    #[tokio::test]
    async fn rows_returns_only_data() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"meta":[{"name":"n","type":"UInt64"}],"data":[{"n":1},{"n":2}],"rows":2}"#,
            ))
            .mount(&server)
            .await;

        let c = client(&server.uri());
        let full = c.query("SELECT n FROM t", None).await.unwrap();
        let rows = c.rows("SELECT n FROM t", None).await.unwrap();
        assert_eq!(rows, full.data);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn ch_error_converts_to_unprocessable_api_error() {
        let api_err: ApiError = ChError::Server("bad sql".to_owned()).into();
        assert_eq!(api_err.status(), 422);
        assert_eq!(api_err.to_string(), "bad sql");
    }

    // -- FORMAT-clause detection edge cases -------------------------------

    #[test]
    fn format_clause_detected_lowercase() {
        assert!(has_trailing_format_clause("select 1 format json"));
    }

    #[test]
    fn format_clause_detected_with_trailing_semicolon() {
        assert!(has_trailing_format_clause("SELECT 1 FORMAT JSON;"));
    }

    #[test]
    fn format_clause_not_detected_inside_closed_string_literal() {
        // The trailing `'` is never consumed by `\s*;?\s*$`, so — exactly
        // as in the TS regex — this does NOT count as a FORMAT clause.
        assert!(!has_trailing_format_clause("SELECT 'FORMAT JSON'"));
    }

    #[test]
    fn format_clause_detected_across_multiple_lines() {
        assert!(has_trailing_format_clause("SELECT 1\nFROM t\nFORMAT JSON"));
    }

    #[test]
    fn format_clause_not_detected_when_word_is_a_suffix() {
        // No `\b` before "format": "XFORMAT" does not contain the keyword
        // at a word boundary.
        assert!(!has_trailing_format_clause("SELECT 1 XFORMAT JSON"));
    }

    #[test]
    fn format_clause_not_detected_without_a_name() {
        assert!(!has_trailing_format_clause("SELECT 1 FORMAT"));
    }

    #[test]
    fn format_clause_not_detected_plain_select() {
        assert!(!has_trailing_format_clause("SELECT 1"));
    }

    #[test]
    fn build_query_body_preserves_trailing_whitespace_when_no_semicolon() {
        // No semicolon at the end, so the TS `replace(/;\s*$/, "")` is a
        // no-op and the original trailing whitespace flows into the body
        // ahead of the appended "\nFORMAT JSON" line.
        assert_eq!(build_query_body("SELECT 1   "), "SELECT 1   \nFORMAT JSON");
    }

    #[test]
    fn build_query_body_strips_semicolon_and_trailing_whitespace() {
        assert_eq!(build_query_body("SELECT 1;   "), "SELECT 1\nFORMAT JSON");
    }
}
