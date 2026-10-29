//! HTTP client for `ClickHouse`'s plain HTTP interface.
//!
//! Ports `src/services/clients/clickhouse.ts` — the client every
//! data-reading route in the Next.js backend depends on. `ClickHouse` is
//! talked to over its plain HTTP interface directly (`POST` a SQL body,
//! read back the response), with no vendor SDK, exactly as the TypeScript
//! client does with `fetch`.

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
    /// ...) surfaced by `reqwest`.
    ///
    /// The `Display` impl deliberately does NOT include `reqwest`'s message:
    /// `reqwest::Error`'s `Display` appends `" for url (http://host:port/)"`,
    /// which — had it been rendered into a response body — would leak the internal `ClickHouse` host/port to an unauthenticated
    /// caller. `src/services/clients/clickhouse.ts` never sees that URL
    /// either: Node's `fetch` (undici) rejects a connection failure with a
    /// `TypeError` whose `.message` is the fixed string `"fetch failed"`
    /// (the underlying cause lives on `.cause`, which the TS route handlers
    /// never read), and that fixed string is what callers of `chQuery`
    /// ultimately surface via `e instanceof Error ? e.message : String(e)`
    /// (see `src/app/api/query/run/route.ts:73`). This variant reproduces
    /// that fixed string. The `reqwest::Error` itself is kept as `#[source]`
    /// so `tracing` (or any structured logger) can still record the real
    /// cause/URL server-side.
    #[error("fetch failed")]
    Transport(#[source] reqwest::Error),
    /// `ClickHouse` responded with a non-2xx status. The message is the
    /// `ClickHouse` error body, trimmed, verbatim — SEC-11: it is for the
    /// log, never for a response body — falling back to
    /// `ClickHouse HTTP <status>` when the body is empty.
    #[error("{0}")]
    Server(String),
    /// The request was cancelled via the caller-supplied
    /// [`CancellationToken`] before `ClickHouse` responded.
    ///
    /// Distinct from [`ChError::Server`] so cancellation is never confused
    /// with a genuine `ClickHouse` error message in logs. No corpus entry
    /// and no TS code path produces a cancellation-visible HTTP response
    /// (`AbortSignal`-triggered aborts surface as a generic fetch rejection
    /// in the TS client, and none of the captured routes exercise that
    /// path), so this mapping is unobservable against the parity corpus;
    /// a cancelled request is a server-side condition, not a client input
    /// error.
    #[error("request cancelled")]
    Cancelled,
}

impl ChError {
    /// The `ClickHouse` error code of a [`ChError::Server`] body such as
    /// `Code: 159. DB::Exception: Timeout exceeded ...`, if it has that
    /// shape. The text itself is for the log only (SEC-11); callers branch on
    /// the code and write their own sentence.
    #[must_use]
    pub fn server_code(&self) -> Option<u32> {
        let Self::Server(text) = self else {
            return None;
        };
        let rest = text.trim_start().strip_prefix("Code: ")?;
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    }

    /// Whether the engine stopped the query for running past its
    /// `max_execution_time` (`TIMEOUT_EXCEEDED`, code 159).
    #[must_use]
    pub fn is_time_limit(&self) -> bool {
        self.server_code() == Some(TIMEOUT_EXCEEDED)
    }

    /// Whether the query was killed on the engine (`QUERY_WAS_CANCELLED`,
    /// code 394), for example by `KILL QUERY` after a Stop.
    #[must_use]
    pub fn is_killed_on_engine(&self) -> bool {
        self.server_code() == Some(QUERY_WAS_CANCELLED)
    }
}

/// `ClickHouse` error code `TIMEOUT_EXCEEDED`.
pub const TIMEOUT_EXCEEDED: u32 = 159;
/// `ClickHouse` error code `QUERY_WAS_CANCELLED`.
pub const QUERY_WAS_CANCELLED: u32 = 394;

/// How far past the engine's own limit the HTTP request waits (`BI-5` plan
/// D3): the engine stops the query at `max_execution_time` and answers with
/// its error, and this margin keeps the client from giving up first and
/// turning that answer into a bare transport timeout.
pub const LIMIT_MARGIN: std::time::Duration = std::time::Duration::from_secs(10);

/// What one limited, identified query asks of the engine (`BI-5` plan D3).
///
/// Every field is a URL setting of that one request, not text in the SQL, so
/// the statement the caller wrote is the statement that runs. (A statement-
/// level `SETTINGS` clause overrides these; the SQL source's own cap is kept
/// in the statement for that reason, see `lakehouse_bi::builder`.)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryOptions {
    /// `max_execution_time`: the engine stops the query after this long. Also
    /// sets the request's own timeout to this plus [`LIMIT_MARGIN`].
    pub max_execution_time: Option<std::time::Duration>,
    /// `max_result_rows` with `result_overflow_mode = break`: pass the number
    /// of rows to keep plus one to learn whether the engine had more.
    pub max_result_rows: Option<u64>,
    /// `query_id`: lets `KILL QUERY WHERE query_id ...` find this query.
    pub query_id: Option<String>,
}

/// Most progress headers one limited request may receive; hyper's client
/// refuses a response with more than 100 headers in all.
const PROGRESS_HEADERS_MAX: u128 = 80;

/// The milliseconds between progress headers for a query allowed to run
/// `limit`: one second, or more for a long limit so the count stays at most
/// [`PROGRESS_HEADERS_MAX`].
fn progress_interval_ms(limit: std::time::Duration) -> u128 {
    (limit.as_millis() / PROGRESS_HEADERS_MAX).max(1000)
}

/// The `max_execution_time` URL setting for `limit`: whole seconds, rounded
/// up, so a limit is never shortened.
fn max_execution_time_seconds(limit: std::time::Duration) -> String {
    limit
        .as_secs()
        .saturating_add(u64::from(limit.subsec_nanos() > 0))
        .to_string()
}

/// The progress-header settings of a limited request.
///
/// The headers give the server something to write while the query runs, so a
/// client that goes away is noticed on the engine. The interval keeps the
/// number of headers under [`PROGRESS_HEADERS_MAX`].
fn push_progress_settings(limit: std::time::Duration, settings: &mut Vec<(&str, String)>) {
    settings.push(("send_progress_in_http_headers", "1".to_owned()));
    settings.push((
        "http_headers_progress_interval_ms",
        progress_interval_ms(limit).to_string(),
    ));
}

impl QueryOptions {
    /// The request's own timeout: the time limit plus [`LIMIT_MARGIN`], or
    /// `None` (the client's default) without a limit.
    #[must_use]
    pub fn request_timeout(&self) -> Option<std::time::Duration> {
        self.max_execution_time.map(|limit| limit + LIMIT_MARGIN)
    }
}

impl From<reqwest::Error> for ChError {
    fn from(err: reqwest::Error) -> Self {
        Self::Transport(err)
    }
}

// There is deliberately NO `From<ChError> for ApiError` (SEC-11). It used to
// exist and turned `ChError::Server`'s text -- the database's own message,
// with table names and the server version -- into a response body at every
// `?`. A handler now has to say how it reports the failure
// (`lakehouse_api::upstream_error`), so a new `?` on a `ChError` in a route
// does not compile instead of leaking.

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
    ///
    /// The underlying `reqwest::Client` has a 60s request timeout, matching
    /// `export const maxDuration = 60` on the TypeScript route handlers
    /// (e.g. `src/app/api/query/run/route.ts:5`) — the two run under
    /// different mechanisms (a platform-enforced function deadline in
    /// Next.js vs. a client-side request timeout here) but bound the same
    /// worst case: neither lets a single `ClickHouse` request hang the
    /// service indefinitely.
    ///
    /// # Panics
    ///
    /// Never, in practice: `reqwest::ClientBuilder::build` only fails on a
    /// malformed TLS backend configuration, which cannot occur with the
    /// fixed `rustls-tls` feature set this crate builds with. `unwrap`s are
    /// otherwise denied by this workspace's lints, so this is the one
    /// deliberate exception, documented rather than silently allowed.
    #[must_use]
    #[allow(
        clippy::unwrap_used,
        reason = "ClientBuilder::build cannot fail with this crate's fixed \
                  TLS feature set; see the doc comment above"
    )]
    pub fn new(url: String, user: String, password: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap();
        Self {
            client,
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
        self.query_with_settings(sql, &[], None, cancel).await
    }

    /// Run `sql` like [`ChClient::query`], but ask the engine to stop
    /// producing rows once `max_result_rows` have been produced.
    ///
    /// The two settings travel as URL parameters of this one request
    /// (`max_result_rows` and `result_overflow_mode=break`), not in the SQL
    /// text, so the statement the caller wrote is the statement that runs.
    /// With `break`, `ClickHouse` stops at a block boundary: up to one block
    /// more than `max_result_rows` can still arrive, so a caller that needs
    /// an exact count truncates the result itself. A caller that wants to
    /// know whether the engine had more rows than it keeps passes one more
    /// than the number it keeps.
    ///
    /// # Errors
    ///
    /// See [`ChClient::query`].
    pub async fn query_limited(
        &self,
        sql: &str,
        max_result_rows: usize,
        cancel: Option<&CancellationToken>,
    ) -> Result<ChResult, ChError> {
        self.query_with_options(
            sql,
            &QueryOptions {
                max_result_rows: Some(u64::try_from(max_result_rows).unwrap_or(u64::MAX)),
                ..QueryOptions::default()
            },
            cancel,
        )
        .await
    }

    /// Run `sql` like [`ChClient::query`] under the limits and identity of
    /// `options` (`BI-5` plan D3): `max_execution_time`, `max_result_rows`
    /// (with `result_overflow_mode = break`) and `query_id` travel as URL
    /// settings of this request, together with
    /// `cancel_http_readonly_queries_on_client_close = 1`, so the engine also
    /// ends the query when this request is dropped (the pattern
    /// [`ChClient::query_stream`] already uses).
    ///
    /// `wait_end_of_query = 1` is sent with a time limit: without it the
    /// engine may already have written part of the JSON body when the limit
    /// hits, and the error arrives as text after a `200`, which the parse
    /// below would read as an empty result instead of an error.
    ///
    /// When `max_execution_time` is set the request's own timeout is that
    /// plus [`LIMIT_MARGIN`], replacing the client's fixed 60 s for this call
    /// only.
    ///
    /// # Errors
    ///
    /// See [`ChClient::query`]. A query the engine stopped for its time limit
    /// is a [`ChError::Server`] for which [`ChError::is_time_limit`] holds.
    pub async fn query_with_options(
        &self,
        sql: &str,
        options: &QueryOptions,
        cancel: Option<&CancellationToken>,
    ) -> Result<ChResult, ChError> {
        let mut settings: Vec<(&str, String)> = Vec::new();
        if let Some(limit) = options.max_execution_time {
            settings.push(("max_execution_time", max_execution_time_seconds(limit)));
            settings.push(("wait_end_of_query", "1".to_owned()));
            // `cancel_http_readonly_queries_on_client_close` only takes effect
            // when the server writes to the connection and finds it closed; a
            // buffered `SELECT count()` writes nothing until it ends, so the
            // query ran on after the client left (measured on `ClickHouse`
            // 26.8, `docs/plans/BI-5-RESULT.md` E2). Progress headers give the
            // server something to write: with them the query was gone 0.5 to
            // 1 s after the client closed. hyper reads at most 100 response
            // headers, so the interval is stretched until a limit's headers
            // stay under `PROGRESS_HEADERS_MAX`.
            push_progress_settings(limit, &mut settings);
        }
        if let Some(rows) = options.max_result_rows {
            settings.push(("max_result_rows", rows.to_string()));
            settings.push(("result_overflow_mode", "break".to_owned()));
        }
        if let Some(id) = &options.query_id {
            settings.push(("query_id", id.clone()));
        }
        settings.push((
            "cancel_http_readonly_queries_on_client_close",
            "1".to_owned(),
        ));
        let borrowed: Vec<(&str, &str)> = settings.iter().map(|(k, v)| (*k, v.as_str())).collect();
        self.query_with_settings(sql, &borrowed, options.request_timeout(), cancel)
            .await
    }

    /// The body of [`ChClient::query`] and [`ChClient::query_with_options`]:
    /// `settings` are sent as URL parameters of this request only, and
    /// `timeout`, when given, replaces the client's default for it.
    async fn query_with_settings(
        &self,
        sql: &str,
        settings: &[(&str, &str)],
        timeout: Option<std::time::Duration>,
        cancel: Option<&CancellationToken>,
    ) -> Result<ChResult, ChError> {
        let body = build_query_body(sql);
        let fut = async {
            let mut request = self
                .client
                .post(&self.url)
                .query(settings)
                .basic_auth(&self.user, Some(&self.password))
                .header(CONTENT_TYPE, "text/plain; charset=utf-8")
                .header(CACHE_CONTROL, "no-store")
                .body(body);
            if let Some(timeout) = timeout {
                request = request.timeout(timeout);
            }
            let response = request.send().await?;
            let status = response.status();
            let text = response.text().await?;
            if !status.is_success() {
                return Err(ChError::Server(server_error_message(status, &text)));
            }
            // Non-tabular statements (e.g. issued without a SELECT) return a
            // bare `Ok.` body — not JSON. Treat that as an empty result
            // rather than an error, matching the TS client's try/catch.
            match serde_json::from_str::<ChResult>(&text) {
                Ok(result) => Ok(result),
                // Once progress headers have gone out the status line is
                // already `200`, so an error that comes later (the time
                // limit, a kill) arrives as the body (measured on
                // `ClickHouse` 26.8, `docs/plans/BI-5-RESULT.md` E5). Read as
                // "not JSON, so empty" it would turn a stopped query into an
                // empty result.
                Err(_) if body_is_exception(&text) => {
                    Err(ChError::Server(server_error_message(status, &text)))
                }
                Err(_) => Ok(ChResult::default()),
            }
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

    /// Run `sql` verbatim (no `FORMAT JSON` wrapping — the caller supplies
    /// its own `FORMAT` clause) and return the raw response body bytes,
    /// unparsed.
    ///
    /// Added for `routes::query::download` (WS2 §13, WS2 plan review W9):
    /// a `CSV`/`Parquet` export is not `ClickHouse`'s `FORMAT JSON` shape
    /// [`ChClient::query`] parses, so this is a genuinely new small
    /// addition to this crate's public surface rather than a hidden
    /// dependency, called out in that task's commit body.
    ///
    /// # Errors
    ///
    /// Returns [`ChError::Transport`] on a network-level failure, or
    /// [`ChError::Server`] when `ClickHouse` responds with a non-2xx status.
    pub async fn raw_bytes(
        &self,
        sql: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<Vec<u8>, ChError> {
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
            if !status.is_success() {
                let text = response.text().await?;
                return Err(ChError::Server(server_error_message(status, &text)));
            }
            Ok(response.bytes().await?.to_vec())
        };
        run_cancellable(fut, cancel).await
    }

    /// Run `sql` and hand the rows back one at a time, without holding the
    /// result (`BI-1` D6, a download of up to a million rows).
    ///
    /// The statement gets `FORMAT JSONCompactEachRowWithNamesAndTypes`: a
    /// line of column names, a line of column types, then one JSON array per
    /// row. (Not `JSONEachRow`: an empty result still has its header, so a file
    /// of no rows still has its column names, and a row is an ordered array, so
    /// two columns with one name do not collapse.) The request has its own
    /// `timeout` in place of the client's 60 seconds, which would cut a big
    /// download short; the body is read as it arrives. Dropping the returned
    /// [`RowStream`] closes the connection; closing it does not by itself stop a
    /// running `SELECT`, so every request carries
    /// `cancel_http_readonly_queries_on_client_close=1`, which makes `ClickHouse`
    /// cancel the query when the client goes (`BI-1` review fix (SHOULD-FIX)
    /// R10; the comment used to say the close alone did it).
    ///
    /// This is [`ChClient::query_stream_with_options`] with the default
    /// options: no query id and no time limit.
    ///
    /// # Errors
    ///
    /// [`ChError::Transport`] on a network failure, [`ChError::Server`] on a
    /// non-2xx status or when the header lines are not what the format promises.
    pub async fn query_stream(
        &self,
        sql: &str,
        timeout: std::time::Duration,
    ) -> Result<RowStream, ChError> {
        self.query_stream_with_options(sql, timeout, &QueryOptions::default())
            .await
    }

    /// Run `sql` like [`ChClient::query_stream`], under the identity and the
    /// time limit of `options`.
    ///
    /// `query_id` and `max_execution_time` travel as URL settings of this one
    /// request, together with the progress headers a limited request needs,
    /// so the statement the caller wrote is the statement that runs. The
    /// request's own timeout is the longer of `timeout` and
    /// `options.request_timeout()` (the time limit plus [`LIMIT_MARGIN`]): the
    /// engine's own "time limit" answer then arrives instead of a bare
    /// transport timeout.
    ///
    /// `wait_end_of_query` is **not** sent: the rows are handed over while
    /// the query runs, and a query that hits its time limit after the first
    /// rows are out ends the stream with a line that is not a row.
    ///
    /// `options.max_result_rows` is not sent either: this stream is not
    /// capped by the engine's row limit. The caller that owns the cap counts
    /// the rows as they arrive, and the engine's own cap would cut the
    /// result at a block boundary anyway.
    ///
    /// # Errors
    ///
    /// See [`ChClient::query_stream`].
    pub async fn query_stream_with_options(
        &self,
        sql: &str,
        timeout: std::time::Duration,
        options: &QueryOptions,
    ) -> Result<RowStream, ChError> {
        let mut settings: Vec<(&str, String)> = Vec::new();
        if let Some(limit) = options.max_execution_time {
            settings.push(("max_execution_time", max_execution_time_seconds(limit)));
            push_progress_settings(limit, &mut settings);
        }
        if let Some(id) = &options.query_id {
            settings.push(("query_id", id.clone()));
        }
        settings.push((
            "cancel_http_readonly_queries_on_client_close",
            "1".to_owned(),
        ));
        let borrowed: Vec<(&str, &str)> = settings.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let body = format!(
            "{}\nFORMAT JSONCompactEachRowWithNamesAndTypes",
            strip_trailing_semicolon(sql)
        );
        let request_timeout = match options.request_timeout() {
            Some(limit_timeout) => timeout.max(limit_timeout),
            None => timeout,
        };
        let response = self
            .client
            .post(&self.url)
            .basic_auth(&self.user, Some(&self.password))
            .header(CONTENT_TYPE, "text/plain; charset=utf-8")
            .header(CACHE_CONTROL, "no-store")
            .query(&borrowed)
            .timeout(request_timeout)
            .body(body)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await?;
            return Err(ChError::Server(server_error_message(status, &text)));
        }
        let mut stream = RowStream {
            response,
            buffer: Vec::new(),
            finished: false,
            names: Vec::new(),
            types: Vec::new(),
        };
        let names = stream.next_line().await?;
        let types = stream.next_line().await?;
        stream.names = header_strings(names.as_deref())?;
        stream.types = header_strings(types.as_deref())?;
        Ok(stream)
    }

    /// Run `sql` and hand the response body back as bytes, as they arrive.
    ///
    /// The statement goes to the engine verbatim — no `FORMAT` is appended —
    /// so the caller says which format it wants (`FORMAT Parquet`).
    ///
    /// The body is not held: [`ByteStream::next_chunk`] hands over one chunk
    /// at a time, so a big file costs no more memory than the chunk in the
    /// caller's hands.
    ///
    /// The URL settings are `query_id` and `max_execution_time` when
    /// `options` names them, plus `wait_end_of_query=1` when a time limit is
    /// set: the engine writes a file whole, so a failure after the first
    /// bytes arrives as a status instead of as bytes, and the caller learns
    /// that the file is incomplete rather than writing a truncated one. The
    /// progress headers of a limited request are sent too, and
    /// `cancel_http_readonly_queries_on_client_close=1` always, so a dropped
    /// download stops the query on the engine.
    ///
    /// The request's own timeout is `options.request_timeout()` (the time
    /// limit plus [`LIMIT_MARGIN`]); without a time limit the client's own
    /// default applies.
    ///
    /// # Errors
    ///
    /// [`ChError::Transport`] on a network failure, [`ChError::Server`] when
    /// `ClickHouse` responds with a non-2xx status.
    pub async fn byte_stream(
        &self,
        sql: &str,
        options: &QueryOptions,
    ) -> Result<ByteStream, ChError> {
        let mut settings: Vec<(&str, String)> = Vec::new();
        if let Some(limit) = options.max_execution_time {
            settings.push(("max_execution_time", max_execution_time_seconds(limit)));
            settings.push(("wait_end_of_query", "1".to_owned()));
            push_progress_settings(limit, &mut settings);
        }
        if let Some(id) = &options.query_id {
            settings.push(("query_id", id.clone()));
        }
        settings.push((
            "cancel_http_readonly_queries_on_client_close",
            "1".to_owned(),
        ));
        let borrowed: Vec<(&str, &str)> = settings.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let mut request = self
            .client
            .post(&self.url)
            .basic_auth(&self.user, Some(&self.password))
            .header(CONTENT_TYPE, "text/plain; charset=utf-8")
            .header(CACHE_CONTROL, "no-store")
            .query(&borrowed)
            .body(sql.to_owned());
        if let Some(timeout) = options.request_timeout() {
            request = request.timeout(timeout);
        }
        let response = request.send().await?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await?;
            return Err(ChError::Server(server_error_message(status, &text)));
        }
        Ok(ByteStream {
            response,
            bytes_read: 0,
            checked_first_chunk: false,
        })
    }
}

/// The longest line a [`RowStream`] buffers. One row of a table of text
/// columns is far below it; a body with no line break past it is not rows.
const MAX_STREAM_LINE_BYTES: usize = 16 * 1024 * 1024;

/// A `ClickHouse` result read line by line (see [`ChClient::query_stream`]).
pub struct RowStream {
    response: reqwest::Response,
    buffer: Vec<u8>,
    finished: bool,
    names: Vec<String>,
    types: Vec<String>,
}

/// A header line: a JSON array of strings.
fn header_strings(line: Option<&[u8]>) -> Result<Vec<String>, ChError> {
    line.and_then(|l| serde_json::from_slice::<Vec<String>>(l).ok())
        .ok_or_else(|| ChError::Server("the result stream has no header".to_owned()))
}

impl RowStream {
    /// The column names, in the order a row lists its values.
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The column types (`Int64`, `Nullable(String)`, ...), same order.
    #[must_use]
    pub fn types(&self) -> &[String] {
        &self.types
    }

    /// The next row, or `None` at the end of the result.
    ///
    /// # Errors
    ///
    /// [`ChError::Transport`] when the connection fails or times out
    /// mid-result; [`ChError::Server`] when a line is not a JSON array (the
    /// engine writes an error into the body when it fails after it began to
    /// answer, so a truncated result is never taken for a complete one).
    pub async fn next_row(&mut self) -> Result<Option<Vec<Value>>, ChError> {
        let Some(line) = self.next_line().await? else {
            return Ok(None);
        };
        match serde_json::from_slice::<Value>(&line) {
            Ok(Value::Array(row)) => Ok(Some(row)),
            _ => Err(ChError::Server(
                "the result stream broke off or held a line that is not a row".to_owned(),
            )),
        }
    }

    /// The next non-empty line without its line break.
    async fn next_line(&mut self) -> Result<Option<Vec<u8>>, ChError> {
        loop {
            if let Some(at) = self.buffer.iter().position(|b| *b == b'\n') {
                let mut line: Vec<u8> = self.buffer.drain(..=at).collect();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                if line.is_empty() {
                    continue;
                }
                return Ok(Some(line));
            }
            if self.finished {
                if self.buffer.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(std::mem::take(&mut self.buffer)));
            }
            if self.buffer.len() > MAX_STREAM_LINE_BYTES {
                return Err(ChError::Server(
                    "a line of the result is too long".to_owned(),
                ));
            }
            match self.response.chunk().await? {
                Some(chunk) => self.buffer.extend_from_slice(&chunk),
                None => self.finished = true,
            }
        }
    }
}

/// A `ClickHouse` response body read as bytes (see [`ChClient::byte_stream`]).
///
/// The body is handed over chunk by chunk, as it arrives: nothing is held
/// but the chunk the caller has in its hands. Dropping the stream closes the
/// connection, and the engine is told to cancel the query when the client
/// goes.
pub struct ByteStream {
    response: reqwest::Response,
    bytes_read: u64,
    checked_first_chunk: bool,
}

impl ByteStream {
    /// The next chunk of the body as it arrives, or `None` at the end of the
    /// body.
    ///
    /// The first chunk is checked for the engine's exception text: a query
    /// that fails before the engine writes anything answers with a `200` and
    /// that text in the body, and handing it over as bytes would write an
    /// error message into the file. Only the first chunk is checked — the
    /// rest of the body is not held, so a failure after it is not caught
    /// here; the time limit a download runs under is what covers that (the
    /// engine writes the file whole, so its failure arrives as a status).
    ///
    /// # Errors
    ///
    /// [`ChError::Transport`] when the connection fails or times out
    /// mid-body; [`ChError::Server`] when the first chunk holds the engine's
    /// exception text.
    pub async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ChError> {
        let Some(chunk) = self.response.chunk().await? else {
            return Ok(None);
        };
        if !self.checked_first_chunk {
            self.checked_first_chunk = true;
            // Read as text, lossy: a `Parquet` file starts with `PAR1` and
            // does not match the engine's exception text.
            let text = String::from_utf8_lossy(&chunk);
            if body_is_exception(&text) {
                return Err(ChError::Server(text.into_owned()));
            }
        }
        self.bytes_read = self.bytes_read.saturating_add(chunk.len() as u64);
        Ok(Some(chunk.to_vec()))
    }

    /// How many bytes have been handed over so far.
    ///
    /// A chunk that turned out to be the engine's exception is not counted:
    /// it was never handed over. The download route counts the same way to
    /// decide when to refuse a result that is over its cap.
    #[must_use]
    pub fn bytes_read(&self) -> u64 {
        self.bytes_read
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
                () = token.cancelled() => Err(ChError::Cancelled),
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

/// Whether a `200` body is the engine's exception text (`Code: 159. ...`),
/// alone or after the part of the result that was written before it hit.
fn body_is_exception(text: &str) -> bool {
    text.trim_start().starts_with("Code: ")
        || text
            .lines()
            .last()
            .is_some_and(|last| last.starts_with("Code: ") && last.contains("DB::Exception"))
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
    use wiremock::matchers::{body_string_contains, method, query_param};
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
        assert_eq!(
            result.data,
            [] as [serde_json::Map<std::string::String, serde_json::Value>; 0]
        );
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

    /// A real transport failure (connection refused) must never leak the
    /// internal `ClickHouse` host/port through `ChError`'s `Display` — that
    /// would both diverge from the TS contract (`"fetch failed"`, fixed) and
    /// disclose infrastructure details to an unauthenticated caller.
    #[tokio::test]
    async fn transport_error_display_does_not_leak_url() {
        // Bind to an ephemeral port, then drop the listener immediately so
        // nothing is listening there: connecting to it is guaranteed to be
        // refused, producing a genuine `reqwest::Error` without relying on
        // any specific closed port being free on the test host.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let dead_url = format!("http://{addr}/");
        let err = client(&dead_url).query("SELECT 1", None).await.unwrap_err();

        assert!(matches!(err, ChError::Transport(_)));
        let rendered = err.to_string();
        assert_eq!(rendered, "fetch failed");
        assert!(!rendered.contains("http"));
        assert!(!rendered.contains(&addr.ip().to_string()));
        assert!(!rendered.contains(&addr.port().to_string()));
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

    // -- raw_bytes (WS2 §13, WS2 plan review W9) --------------------------

    #[tokio::test]
    async fn raw_bytes_returns_the_response_body_unparsed() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("FORMAT CSV"))
            .respond_with(ResponseTemplate::new(200).set_body_string("n\n1\n2\n"))
            .mount(&server)
            .await;

        let bytes = client(&server.uri())
            .raw_bytes("SELECT n FROM t FORMAT CSV", None)
            .await
            .unwrap();
        assert_eq!(bytes, b"n\n1\n2\n");
    }

    #[tokio::test]
    async fn raw_bytes_maps_a_non_2xx_status_to_ch_error_server() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(500).set_body_string("Code: 47. Unknown identifier"),
            )
            .mount(&server)
            .await;

        let err = client(&server.uri())
            .raw_bytes("SELECT bogus FORMAT CSV", None)
            .await
            .unwrap_err();
        assert!(matches!(err, ChError::Server(msg) if msg.contains("Unknown identifier")));
    }

    #[tokio::test]
    async fn query_stream_reads_the_header_then_one_row_per_line() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains(
                "FORMAT JSONCompactEachRowWithNamesAndTypes",
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    "[\"a\",\"b\"]\n[\"Int64\",\"String\"]\n[1,\"x\"]\n[2,null]\n",
                ),
            )
            .mount(&server)
            .await;
        let mut rows = client(&server.uri())
            .query_stream("SELECT 1;", std::time::Duration::from_secs(5))
            .await
            .unwrap();
        let sent = server.received_requests().await.unwrap();
        assert_eq!(
            sent[0].url.query(),
            Some("cancel_http_readonly_queries_on_client_close=1"),
            "a dropped download cancels its query (R10)"
        );
        assert_eq!(rows.names(), ["a", "b"]);
        assert_eq!(rows.types(), ["Int64", "String"]);
        assert_eq!(
            rows.next_row().await.unwrap(),
            Some(vec![serde_json::json!(1), serde_json::json!("x")])
        );
        assert_eq!(
            rows.next_row().await.unwrap(),
            Some(vec![serde_json::json!(2), Value::Null])
        );
        assert_eq!(rows.next_row().await.unwrap(), None);
    }

    #[tokio::test]
    async fn query_stream_of_an_empty_result_still_has_its_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("[\"a\"]\n[\"Int64\"]\n"))
            .mount(&server)
            .await;
        let mut rows = client(&server.uri())
            .query_stream("SELECT 1", std::time::Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(rows.names(), ["a"]);
        assert_eq!(rows.next_row().await.unwrap(), None);
    }

    #[tokio::test]
    async fn query_stream_reports_a_server_error_and_a_line_that_is_not_a_row() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("bad"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Code: 62. DB::Exception"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    "[\"a\"]\n[\"Int64\"]\n[1]\nCode: 241. DB::Exception: memory\n",
                ),
            )
            .mount(&server)
            .await;
        let c = client(&server.uri());
        assert!(matches!(
            c.query_stream("SELECT bad", std::time::Duration::from_secs(5))
                .await,
            Err(ChError::Server(_))
        ));
        let mut rows = c
            .query_stream("SELECT 1", std::time::Duration::from_secs(5))
            .await
            .unwrap();
        assert!(rows.next_row().await.unwrap().is_some());
        assert!(matches!(rows.next_row().await, Err(ChError::Server(_))));
    }

    // -- query_limited (SEC-21) -------------------------------------------

    #[tokio::test]
    async fn query_limited_sends_the_cap_as_url_parameters_not_in_the_sql() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(query_param("max_result_rows", "2001"))
            .and(query_param("result_overflow_mode", "break"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"meta":[{"name":"n","type":"UInt64"}],"data":[{"n":1}],"rows":1}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let result = client(&server.uri())
            .query_limited("SELECT n FROM t", 2001, None)
            .await
            .unwrap();
        assert_eq!(result.rows, 1);

        let requests = server.received_requests().await.unwrap();
        let body = String::from_utf8(requests[0].body.clone()).unwrap();
        assert!(
            !body.contains("SETTINGS"),
            "the SQL text stays as written: {body}"
        );
    }

    #[tokio::test]
    async fn query_sends_no_url_parameters() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("Ok."))
            .mount(&server)
            .await;

        client(&server.uri()).query("SELECT 1", None).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests[0].url.query(), None);
    }

    // -- query_with_options (BI-5 plan D3) ---------------------------------

    #[tokio::test]
    async fn query_with_options_sends_the_limits_the_id_and_the_cancel_setting_as_url_parameters() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(query_param("max_execution_time", "45"))
            .and(query_param("max_result_rows", "2001"))
            .and(query_param("result_overflow_mode", "break"))
            .and(query_param("query_id", "u:r:1"))
            .and(query_param(
                "cancel_http_readonly_queries_on_client_close",
                "1",
            ))
            .and(query_param("wait_end_of_query", "1"))
            .and(query_param("send_progress_in_http_headers", "1"))
            .and(query_param("http_headers_progress_interval_ms", "1000"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"meta":[{"name":"n","type":"UInt64"}],"data":[{"n":1}],"rows":1}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let options = QueryOptions {
            max_execution_time: Some(std::time::Duration::from_secs(45)),
            max_result_rows: Some(2001),
            query_id: Some("u:r:1".to_owned()),
        };
        let result = client(&server.uri())
            .query_with_options("SELECT n FROM t", &options, None)
            .await
            .unwrap();
        assert_eq!(result.rows, 1);
        let requests = server.received_requests().await.unwrap();
        let body = String::from_utf8(requests[0].body.clone()).unwrap();
        assert!(
            !body.contains("SETTINGS"),
            "the SQL text stays as written: {body}"
        );
    }

    #[test]
    fn the_request_waits_the_limit_plus_ten_seconds_and_a_call_without_a_limit_keeps_the_client_default()
     {
        let limited = QueryOptions {
            max_execution_time: Some(std::time::Duration::from_secs(300)),
            ..QueryOptions::default()
        };
        assert_eq!(
            limited.request_timeout(),
            Some(std::time::Duration::from_secs(310))
        );
        assert_eq!(QueryOptions::default().request_timeout(), None);
    }

    #[test]
    fn progress_headers_stay_under_the_header_cap_for_every_allowed_limit() {
        for seconds in [10_u64, 60, 600, 1800] {
            let limit = std::time::Duration::from_secs(seconds);
            let interval = progress_interval_ms(limit);
            assert!(interval >= 1000);
            assert!(
                limit.as_millis() / interval <= PROGRESS_HEADERS_MAX,
                "{seconds}"
            );
        }
    }

    #[tokio::test]
    async fn an_exception_that_arrives_in_a_200_body_is_an_error_not_an_empty_result() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "Code: 159. DB::Exception: Timeout exceeded: elapsed 10000.343 ms, maximum: 10000. (TIMEOUT_EXCEEDED)",
            ))
            .mount(&server)
            .await;
        let err = client(&server.uri())
            .query("SELECT 1", None)
            .await
            .unwrap_err();
        assert!(err.is_time_limit(), "{err:?}");
    }

    #[tokio::test]
    async fn a_bare_ok_body_is_still_an_empty_result() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("Ok."))
            .mount(&server)
            .await;
        let result = client(&server.uri()).query("SELECT 1", None).await.unwrap();
        assert_eq!(result.rows, 0);
    }

    #[tokio::test]
    async fn a_limit_given_in_fractions_of_a_second_is_rounded_up() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(query_param("max_execution_time", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_string("Ok."))
            .expect(1)
            .mount(&server)
            .await;
        let options = QueryOptions {
            max_execution_time: Some(std::time::Duration::from_millis(1500)),
            ..QueryOptions::default()
        };
        client(&server.uri())
            .query_with_options("SELECT 1", &options, None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn an_engine_error_is_classified_by_its_code_and_never_by_its_text() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string(
                "Code: 159. DB::Exception: Timeout exceeded: elapsed 1.0 seconds, maximum: 1. (TIMEOUT_EXCEEDED)",
            ))
            .mount(&server)
            .await;
        let err = client(&server.uri())
            .query("SELECT 1", None)
            .await
            .unwrap_err();
        assert_eq!(err.server_code(), Some(159));
        assert!(err.is_time_limit());
        assert!(!err.is_killed_on_engine());

        assert!(
            ChError::Server(
                "Code: 394. DB::Exception: Query was cancelled. (QUERY_WAS_CANCELLED)".to_owned()
            )
            .is_killed_on_engine()
        );
        assert_eq!(
            ChError::Server("Code: 62. DB::Exception: Syntax error".to_owned()).server_code(),
            Some(62)
        );
        assert_eq!(
            ChError::Server("ClickHouse HTTP 500".to_owned()).server_code(),
            None
        );
        assert_eq!(ChError::Cancelled.server_code(), None);
    }

    // -- the row stream under options --------------------------------------

    /// The URL settings of a request a mock server received, as decoded
    /// pairs.
    fn url_settings(request: &wiremock::Request) -> Vec<(String, String)> {
        request
            .url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect()
    }

    #[tokio::test]
    async fn the_row_stream_carries_the_query_id_and_the_time_limit_and_never_waits_for_the_end() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("[\"a\"]\n[\"Int64\"]\n"))
            .expect(1)
            .mount(&server)
            .await;

        let options = QueryOptions {
            max_execution_time: Some(std::time::Duration::from_secs(45)),
            query_id: Some("u:r:1".to_owned()),
            ..QueryOptions::default()
        };
        let rows = client(&server.uri())
            .query_stream_with_options("SELECT 1", std::time::Duration::from_secs(5), &options)
            .await
            .unwrap();
        assert_eq!(rows.names(), ["a"]);
        let requests = server.received_requests().await.unwrap();
        let settings = url_settings(&requests[0]);
        assert!(
            settings.contains(&("query_id".to_owned(), "u:r:1".to_owned())),
            "{settings:?}"
        );
        assert!(
            settings.contains(&("max_execution_time".to_owned(), "45".to_owned())),
            "{settings:?}"
        );
        assert!(
            settings.contains(&("send_progress_in_http_headers".to_owned(), "1".to_owned())),
            "{settings:?}"
        );
        assert!(
            !settings.iter().any(|(key, _)| key == "wait_end_of_query"),
            "the row stream answers while the query runs: {settings:?}"
        );
    }

    #[tokio::test]
    async fn the_row_stream_is_not_capped_by_the_engine_row_limit() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("[\"a\"]\n[\"Int64\"]\n"))
            .expect(1)
            .mount(&server)
            .await;

        let options = QueryOptions {
            max_result_rows: Some(10),
            ..QueryOptions::default()
        };
        client(&server.uri())
            .query_stream_with_options("SELECT 1", std::time::Duration::from_secs(5), &options)
            .await
            .unwrap();

        let requests = server.received_requests().await.unwrap();
        let settings = url_settings(&requests[0]);
        assert!(
            !settings.iter().any(|(key, _)| key == "max_result_rows"),
            "{settings:?}"
        );
        assert!(
            !settings
                .iter()
                .any(|(key, _)| key == "result_overflow_mode"),
            "{settings:?}"
        );
    }

    // -- the byte stream ---------------------------------------------------

    #[tokio::test]
    async fn the_byte_stream_sends_the_statement_verbatim() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("FORMAT Parquet"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"PAR1payloadPAR1"))
            .expect(1)
            .mount(&server)
            .await;

        let mut stream = client(&server.uri())
            .byte_stream("SELECT 1 FORMAT Parquet", &QueryOptions::default())
            .await
            .unwrap();
        let mut body = Vec::new();
        while let Some(chunk) = stream.next_chunk().await.unwrap() {
            body.extend_from_slice(&chunk);
        }
        assert_eq!(body, b"PAR1payloadPAR1");

        let requests = server.received_requests().await.unwrap();
        let sent = String::from_utf8(requests[0].body.clone()).unwrap();
        assert_eq!(
            sent, "SELECT 1 FORMAT Parquet",
            "the statement goes to the engine as it was written"
        );
    }

    #[tokio::test]
    async fn the_byte_stream_carries_the_query_id_the_time_limit_and_the_wait_for_the_end() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"PAR1"))
            .expect(1)
            .mount(&server)
            .await;

        let options = QueryOptions {
            max_execution_time: Some(std::time::Duration::from_secs(45)),
            query_id: Some("u:r:1:download".to_owned()),
            ..QueryOptions::default()
        };
        client(&server.uri())
            .byte_stream("SELECT 1 FORMAT Parquet", &options)
            .await
            .unwrap();

        let requests = server.received_requests().await.unwrap();
        let settings = url_settings(&requests[0]);
        assert!(
            settings.contains(&("query_id".to_owned(), "u:r:1:download".to_owned())),
            "{settings:?}"
        );
        assert!(
            settings.contains(&("max_execution_time".to_owned(), "45".to_owned())),
            "{settings:?}"
        );
        assert!(
            settings.contains(&("wait_end_of_query".to_owned(), "1".to_owned())),
            "the engine writes the file whole, so a failure arrives as a status: {settings:?}"
        );
        assert!(
            settings.contains(&("send_progress_in_http_headers".to_owned(), "1".to_owned())),
            "{settings:?}"
        );
        assert!(
            settings.contains(&(
                "cancel_http_readonly_queries_on_client_close".to_owned(),
                "1".to_owned()
            )),
            "{settings:?}"
        );
    }

    #[tokio::test]
    async fn the_byte_stream_without_a_time_limit_never_waits_for_the_end() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"PAR1"))
            .expect(1)
            .mount(&server)
            .await;

        client(&server.uri())
            .byte_stream("SELECT 1 FORMAT Parquet", &QueryOptions::default())
            .await
            .unwrap();

        let requests = server.received_requests().await.unwrap();
        let settings = url_settings(&requests[0]);
        assert!(
            !settings.iter().any(|(key, _)| key == "wait_end_of_query"),
            "{settings:?}"
        );
        assert!(
            !settings.iter().any(|(key, _)| key == "max_execution_time"),
            "{settings:?}"
        );
        assert!(
            settings.contains(&(
                "cancel_http_readonly_queries_on_client_close".to_owned(),
                "1".to_owned()
            )),
            "a dropped download stops the query on the engine: {settings:?}"
        );
    }

    #[tokio::test]
    async fn the_byte_stream_fails_when_the_engine_writes_its_exception_instead_of_a_file() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "Code: 159. DB::Exception: Timeout exceeded: elapsed 45000.0 ms, maximum: 45000. (TIMEOUT_EXCEEDED)",
            ))
            .expect(1)
            .mount(&server)
            .await;

        let mut stream = client(&server.uri())
            .byte_stream("SELECT 1 FORMAT Parquet", &QueryOptions::default())
            .await
            .unwrap();
        let err = stream.next_chunk().await.unwrap_err();
        assert!(
            matches!(&err, ChError::Server(msg) if msg.contains("TIMEOUT_EXCEEDED")),
            "{err:?}"
        );
        assert!(err.is_time_limit(), "{err:?}");
    }

    #[tokio::test]
    async fn the_byte_stream_maps_a_non_2xx_status_to_ch_error_server() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(500).set_body_string("Code: 47. Unknown identifier"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let Err(err) = client(&server.uri())
            .byte_stream("SELECT bogus FORMAT Parquet", &QueryOptions::default())
            .await
        else {
            panic!("the engine's answer was taken for a file");
        };
        assert!(
            matches!(&err, ChError::Server(msg) if msg.contains("Unknown identifier")),
            "{err:?}"
        );
    }

    /// A server that answers with a chunked body: the first chunk goes out at
    /// once, the second one after `pause`. A client that held the whole body
    /// before handing anything over could not answer before the pause is
    /// over.
    fn chunked_body_server(first: &[u8], second: &[u8], pause: std::time::Duration) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let first = first.to_vec();
        let second = second.to_vec();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            // Read the request head; the body is short and the client writes
            // it before it reads the response.
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                match stream.read(&mut byte) {
                    Ok(0) | Err(_) => return,
                    Ok(_) => head.push(byte[0]),
                }
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                .unwrap();
            for (index, chunk) in [&first, &second].into_iter().enumerate() {
                if index > 0 {
                    std::thread::sleep(pause);
                }
                stream
                    .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                    .unwrap();
                stream.write_all(chunk).unwrap();
                stream.write_all(b"\r\n").unwrap();
                stream.flush().unwrap();
            }
            stream.write_all(b"0\r\n\r\n").unwrap();
            stream.flush().unwrap();
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn the_byte_stream_hands_the_body_over_as_it_arrives() {
        let url = chunked_body_server(
            b"PAR1first",
            b"secondPAR1",
            std::time::Duration::from_secs(2),
        );
        let mut stream = client(&url)
            .byte_stream("SELECT 1 FORMAT Parquet", &QueryOptions::default())
            .await
            .unwrap();

        let first = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next_chunk())
            .await
            .expect("the first chunk arrives before the rest of the body exists")
            .unwrap()
            .unwrap();
        assert_eq!(first, b"PAR1first");
        assert_eq!(stream.bytes_read(), 9);

        let mut body = first;
        while let Some(chunk) = stream.next_chunk().await.unwrap() {
            body.extend_from_slice(&chunk);
        }
        assert_eq!(body, b"PAR1firstsecondPAR1");
        assert_eq!(stream.bytes_read(), 19);
    }
}
