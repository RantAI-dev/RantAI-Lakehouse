//! The one place an upstream error becomes something a response may carry
//! (`SEC-11`, `AGENTS.md` principle 4).
//!
//! # Why this exists
//!
//! `ClickHouse`, Postgres, the Iceberg catalog, an LLM provider and the
//! connector drivers all return error text this code base did not write:
//! table and column names, versions, hosts, sometimes a fragment of the
//! statement. Before `SEC-11` several handlers put that text in a response
//! body, including on the public and embed routes that have no sign-in.
//!
//! The contract here is: the raw error goes to the log, keyed by a short
//! reference id; the caller gets a fixed message and the same id, so an
//! operator can find the log line from what the user quotes. Messages this
//! code base writes itself (validation, not-found, permission and policy
//! refusals, `supported: false` reasons) do NOT go through here; they say
//! what is wrong and stay as they are.
//!
//! # Request ids
//!
//! No request or correlation id exists in this service (no
//! `TraceLayer`/`x-request-id` middleware; verified by search at `9cff2c2`),
//! so a short random reference is generated per reported error with the
//! `uuid` crate this crate already depends on.
//!
//! # What the log line may carry
//!
//! The raw text is passed through [`redact`] first: an upstream error can
//! echo a connection string (`scheme://user:password@host`) or a `key=value`
//! credential, and a log line is not a safe place for either.

use std::fmt::{self, Display};

use lakehouse_clickhouse::ChError;
use lakehouse_core::ApiError;
use serde_json::{Value, json};

/// The longest raw error kept in a log line, in characters. An upstream
/// error that echoes a whole statement should not turn one request into a
/// megabyte of log.
const MAX_LOGGED_CHARS: usize = 2_000;

/// Whether the upstream answered with a failure or could not be reached.
/// The two are kept apart because the status codes differ (a failure is the
/// request's problem, unavailability is retryable) and so does the wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// The upstream was reached and refused or failed the request.
    Failed,
    /// The upstream could not be reached, or timed out.
    Unavailable,
}

/// Which status a [`Class::Failed`] upstream error takes in an
/// [`ApiError`]. [`Class::Unavailable`] is always `503`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailedAs {
    /// `422`, the status `ChError` conversions have always used.
    Unprocessable,
    /// `500`.
    Internal,
    /// `400`, for the handlers that have always answered a failed write with
    /// a bad-request status; kept so SEC-11 changes the text, not the status.
    BadRequest,
    /// `503`, for the stores whose callers want one status whether the
    /// statement failed or the database is down.
    ServiceUnavailable,
}

/// What a call site is doing, in fixed words. Both messages are static: a
/// response can never carry anything that was not written here.
#[derive(Debug, Clone, Copy)]
pub struct Context {
    /// Names the operation in the log line. Not shown to the user.
    pub log: &'static str,
    /// The message when the upstream failed the request.
    pub failed: &'static str,
    /// The message when the upstream could not be reached.
    pub unavailable: &'static str,
}

impl Context {
    /// A context from its three fixed strings.
    #[must_use]
    pub const fn new(log: &'static str, failed: &'static str, unavailable: &'static str) -> Self {
        Self {
            log,
            failed,
            unavailable,
        }
    }
}

/// A dashboard or embed tile that could not be drawn.
pub const TILE: Context = Context::new(
    "dashboard tile",
    "This chart could not be loaded.",
    "The database is unavailable.",
);

/// A `ClickHouse` request that is not a tile.
pub const DATABASE: Context = Context::new(
    "database request",
    "The database request failed.",
    "The database is unavailable.",
);

/// A Postgres (store) request.
pub const STORE: Context = Context::new(
    "store request",
    "The request could not be completed.",
    "The database is unavailable.",
);

/// An Iceberg catalog request.
pub const CATALOG: Context = Context::new(
    "catalog request",
    "The catalog request failed.",
    "The catalog is unavailable.",
);

/// An LLM provider request.
pub const MODEL: Context = Context::new(
    "model request",
    "The request to the model provider failed.",
    "The model provider is unavailable.",
);

/// An object-store, orchestrator or other internal service request.
pub const SERVICE: Context = Context::new(
    "service request",
    "The request to a backing service failed.",
    "A backing service is unavailable.",
);

/// A reported upstream error: our fixed message and the reference id the raw
/// text was logged under.
#[derive(Debug, Clone)]
pub struct UpstreamFailure {
    class: Class,
    message: &'static str,
    reference: String,
}

impl UpstreamFailure {
    /// The fixed message, without the reference.
    #[must_use]
    pub const fn message(&self) -> &'static str {
        self.message
    }

    /// The id the raw error was logged under.
    #[must_use]
    pub fn reference(&self) -> &str {
        &self.reference
    }

    /// The `{ "error": "<fixed>", "errorId": "<id>" }` shape tiles and
    /// assistant tools carry.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({ "error": self.message, "errorId": self.reference })
    }

    /// An [`ApiError`] whose message is the fixed text followed by the
    /// reference. `failed_as` picks the status of a [`Class::Failed`] error;
    /// unavailability is always `503`.
    #[must_use]
    pub fn into_api_error(self, failed_as: FailedAs) -> ApiError {
        let text = self.to_string();
        match (self.class, failed_as) {
            (Class::Unavailable, _) | (Class::Failed, FailedAs::ServiceUnavailable) => {
                ApiError::Unavailable(text)
            }
            (Class::Failed, FailedAs::Unprocessable) => ApiError::Unprocessable(text),
            (Class::Failed, FailedAs::Internal) => ApiError::Internal(text),
            (Class::Failed, FailedAs::BadRequest) => ApiError::BadRequest(text),
        }
    }
}

impl Display for UpstreamFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} Reference: {}", self.message, self.reference)
    }
}

/// Logs `err` under a fresh reference id and returns the fixed message for
/// `class`. [`Class::Failed`] logs at `error`, [`Class::Unavailable`] at
/// `warn` (an outage is expected to happen; it is not a bug).
pub fn report(ctx: &Context, class: Class, err: &dyn Display) -> UpstreamFailure {
    let reference = new_reference();
    let raw = redact(&err.to_string());
    let message = match class {
        Class::Failed => ctx.failed,
        Class::Unavailable => ctx.unavailable,
    };
    match class {
        Class::Failed => {
            tracing::error!(reference = %reference, context = ctx.log, error = %raw, "upstream request failed");
        }
        Class::Unavailable => {
            tracing::warn!(reference = %reference, context = ctx.log, error = %raw, "upstream unavailable");
        }
    }
    UpstreamFailure {
        class,
        message,
        reference,
    }
}

/// [`report`] for a `ClickHouse` error: a server-side error is a
/// [`Class::Failed`], a transport failure or cancellation is
/// [`Class::Unavailable`].
#[must_use]
pub fn report_ch(ctx: &Context, err: &ChError) -> UpstreamFailure {
    let class = match err {
        ChError::Server(_) => Class::Failed,
        ChError::Transport(_) | ChError::Cancelled => Class::Unavailable,
    };
    // `ChError::Transport`'s `Display` is the fixed "fetch failed"; the
    // real cause (and the URL) is its `#[source]`, which only the log gets.
    if let ChError::Transport(source) = err {
        return report(ctx, class, &format!("{err}: {source}"));
    }
    report(ctx, class, err)
}

/// [`report`] for a Postgres error: an unreachable database, a closed or
/// exhausted pool and an I/O failure are [`Class::Unavailable`]; everything
/// else is [`Class::Failed`].
#[must_use]
pub fn report_sqlx(ctx: &Context, err: &sqlx::Error) -> UpstreamFailure {
    let class = match err {
        sqlx::Error::Io(_)
        | sqlx::Error::PoolTimedOut
        | sqlx::Error::PoolClosed
        | sqlx::Error::WorkerCrashed => Class::Unavailable,
        _ => Class::Failed,
    };
    report(ctx, class, err)
}

/// Shorthand: report `err` as a [`Class::Failed`] and convert it to an
/// [`ApiError`] at `500`.
pub fn internal_error(ctx: &Context, err: &dyn Display) -> ApiError {
    report(ctx, Class::Failed, err).into_api_error(FailedAs::Internal)
}

/// Shorthand: report a `ClickHouse` error and convert it to an
/// [`ApiError`] (`422` for a failed statement, `503` when unreachable).
#[must_use]
pub fn ch_error(ctx: &Context, err: &ChError) -> ApiError {
    report_ch(ctx, err).into_api_error(FailedAs::Unprocessable)
}

/// [`ch_error`] with the status of a failed statement chosen by the caller,
/// for the handlers whose status is not `422` today.
#[must_use]
pub fn ch_error_as(ctx: &Context, err: &ChError, failed_as: FailedAs) -> ApiError {
    report_ch(ctx, err).into_api_error(failed_as)
}

// ── The query-author exception (SEC-11, product owner decision 2026-10-10) ──
//
// Query Studio (`POST /api/query/run` and the editor's cost estimate) is the
// one place the engine's own message may reach a response: the person who
// wrote the statement, signed in and holding `query:read`, is entitled to the
// engine's diagnosis of it. Tiles, public and embed links, SQL sources, the
// assistant's tools and saved-query runs stay closed. Every function here
// ends in `_for_author`, and `tests/sec11_guard.rs` flags any route line that
// calls one outside its single allowlist entry for `query.rs`, so the
// exception cannot spread by copy.

/// The longest diagnosis shown to the author, in characters.
const MAX_DIAGNOSIS_CHARS: usize = 800;

/// `ClickHouse` error codes that mean the engine or the network failed, not
/// the statement: `TIMEOUT_EXCEEDED`, `NO_FREE_CONNECTION`, `SOCKET_TIMEOUT`,
/// `NETWORK_ERROR`, `ALL_CONNECTION_TRIES_FAILED`.
const INFRASTRUCTURE_CODES: [u32; 5] = [159, 203, 209, 210, 279];

/// Whether `text` is the engine answering that the STATEMENT is wrong (a
/// `Code: N. DB::Exception: ...` body), as opposed to an unreachable or
/// failing engine (an empty body, a proxy's page, a timeout code).
#[must_use]
pub fn is_statement_error(text: &str) -> bool {
    let line = text.trim_start();
    let Some(rest) = line.strip_prefix("Code: ") else {
        return false;
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let Ok(code) = digits.parse::<u32>() else {
        return false;
    };
    rest.contains("DB::Exception") && !INFRASTRUCTURE_CODES.contains(&code)
}

/// What the author may read of an engine error: the first line, with the
/// trailing `(version ...)` suffix, credentials, URLs, IP addresses and
/// `host:port` pairs removed. The error code name and the message stay.
#[must_use]
pub fn author_diagnosis(raw: &str) -> String {
    let first = raw.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let mut line = redact(first.trim());
    // Cut the trailing `(version A.B.C.D (official build))` group BEFORE
    // masking: its four-part number looks like an IPv4 address, and the
    // group nests parentheses, so it must be matched by balance.
    if line.ends_with(')')
        && let Some(at) = line.rfind("(version ")
        && closes_at_end(&line[at..])
    {
        line.truncate(at);
        line.truncate(line.trim_end().len());
    }
    let masked: Vec<String> = line.split(' ').map(mask_address).collect();
    masked
        .join(" ")
        .trim()
        .chars()
        .take(MAX_DIAGNOSIS_CHARS)
        .collect()
}

/// Whether the parenthesis opened at the start of `group` is closed by the
/// last character and by nothing earlier.
fn closes_at_end(group: &str) -> bool {
    let mut depth = 0_i32;
    for (i, c) in group.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1 == group.len();
                }
            }
            _ => {}
        }
    }
    false
}

/// `token` with its address part replaced by `<host>`; surrounding
/// punctuation is kept.
fn mask_address(token: &str) -> String {
    const EDGE: &[char] = &[
        '(', ')', '[', ']', '{', '}', '<', '>', ',', ';', '\'', '"', '`',
    ];
    let start = token.len() - token.trim_start_matches(EDGE).len();
    let rest = &token[start..];
    // A sentence `.` or `:` after a closing bracket is not part of the address.
    let bare = rest.trim_end_matches(|c: char| EDGE.contains(&c) || c == '.' || c == ':');
    let tail = &rest[bare.len()..];
    if looks_like_address(bare) {
        format!("{}<host>{tail}", &token[..start])
    } else {
        token.to_owned()
    }
}

fn looks_like_address(text: &str) -> bool {
    use std::net::{IpAddr, SocketAddr};
    if text.contains("://") {
        return true;
    }
    if text.parse::<IpAddr>().is_ok() && text.chars().any(|c| c == '.' || c.is_ascii_hexdigit()) {
        // `DB::` style words are not hex-only with a colon pair and a digit
        // missing; require a digit so a bare `::` or `DB::` is left alone.
        return text.chars().any(|c| c.is_ascii_digit());
    }
    if text.parse::<SocketAddr>().is_ok() {
        return true;
    }
    // `host:port` where the host has a letter or a dot (`1:5` is a position).
    match text.rsplit_once(':') {
        Some((host, port)) => {
            !host.is_empty()
                && !port.is_empty()
                && port.chars().all(|c| c.is_ascii_digit())
                && host
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
                && host.chars().any(|c| c == '.' || c.is_ascii_alphabetic())
        }
        None => false,
    }
}

/// Logs `raw` under a fresh reference and returns the author's message: the
/// trimmed diagnosis followed by the reference.
fn author_message(ctx: &Context, raw: &str) -> String {
    let failure = report(ctx, Class::Failed, &raw);
    format!(
        "{} Reference: {}",
        author_diagnosis(raw),
        failure.reference()
    )
}

/// [`ch_error`] for the person who wrote the statement: a statement error
/// (see [`is_statement_error`]) is shown as the trimmed diagnosis and a
/// reference at `422`; anything else (unreachable, timeout, a non-query
/// failure) is the fixed message exactly as [`ch_error`] gives it.
#[must_use]
pub fn ch_error_for_author(ctx: &Context, err: &ChError) -> ApiError {
    match err {
        ChError::Server(text) if is_statement_error(text) => {
            ApiError::Unprocessable(author_message(ctx, text))
        }
        _ => ch_error(ctx, err),
    }
}

/// [`ch_error_for_author`] returning the message instead of an [`ApiError`],
/// for the cost estimate, which answers `200` with the message in `error`.
#[must_use]
pub fn ch_message_for_author(ctx: &Context, err: &ChError) -> String {
    match err {
        ChError::Server(text) if is_statement_error(text) => author_message(ctx, text),
        _ => report_ch(ctx, err).to_string(),
    }
}

/// The same for an engine that already told statement errors from outages
/// (`Trino`'s `TrinoError::Query`): `raw` is the engine's query-error text.
#[must_use]
pub fn query_error_for_author(ctx: &Context, raw: &str) -> ApiError {
    ApiError::Unprocessable(author_message(ctx, raw))
}

/// What an error enum that mixes our own text with an upstream's becomes.
///
/// `BiError`, `AlertError` and `StoreError` each have a variant whose message
/// this code base wrote (validation, not found) and a variant that wraps an
/// upstream error. Only the second kind is reported and replaced.
#[derive(Debug, Clone)]
pub enum Reported {
    /// Our own message; safe to show as it is.
    Own(String),
    /// An upstream error, now logged under a reference.
    Upstream(UpstreamFailure),
}

impl Reported {
    /// The `{ "error": ... }` object a tile or an assistant tool returns;
    /// carries `errorId` only for an upstream error.
    #[must_use]
    pub fn to_json(&self) -> Value {
        match self {
            Self::Own(message) => json!({ "error": message }),
            Self::Upstream(failure) => failure.to_json(),
        }
    }

    /// An [`ApiError`]: our own text as a `400`, an upstream error as the
    /// fixed message and reference at `failed_as`.
    #[must_use]
    pub fn into_api_error(self, failed_as: FailedAs) -> ApiError {
        match self {
            Self::Own(message) => ApiError::BadRequest(message),
            Self::Upstream(failure) => failure.into_api_error(failed_as),
        }
    }
}

/// [`Reported`] for a `lakehouse_bi` error: validation text is ours, a
/// `ClickHouse` failure is reported.
#[must_use]
pub fn bi(ctx: &Context, err: &lakehouse_bi::store::BiError) -> Reported {
    match err {
        lakehouse_bi::store::BiError::Validation(message) => Reported::Own(message.clone()),
        lakehouse_bi::store::BiError::Clickhouse(ch) => Reported::Upstream(report_ch(ctx, ch)),
    }
}

/// [`Reported`] for a `lakehouse_alerts` error: validation text is ours, a
/// `ClickHouse` failure is reported.
#[must_use]
pub fn alert(ctx: &Context, err: &lakehouse_alerts::AlertError) -> Reported {
    match err {
        lakehouse_alerts::AlertError::Validation(message) => Reported::Own(message.clone()),
        lakehouse_alerts::AlertError::Clickhouse(ch) => Reported::Upstream(report_ch(ctx, ch)),
    }
}

/// [`Reported`] for a `lakehouse_store` error. `StoreError`'s own `Display`
/// is already free of the `sqlx` text, so every variant but the two that wrap
/// a driver error keeps its message; those two are reported so the user gets
/// a reference id and the log gets the cause.
#[must_use]
pub fn store(ctx: &Context, err: &lakehouse_store::StoreError) -> Reported {
    match err {
        lakehouse_store::StoreError::Database(source) => {
            Reported::Upstream(report_sqlx(ctx, source))
        }
        lakehouse_store::StoreError::Migration(source) => {
            Reported::Upstream(report(ctx, Class::Failed, source))
        }
        other => Reported::Own(other.to_string()),
    }
}

/// A short random reference: ten hex characters, enough to find one log
/// line, short enough to read out over the phone.
fn new_reference() -> String {
    let mut id = uuid::Uuid::new_v4().simple().to_string();
    id.truncate(10);
    id
}

/// Strips what a log line must not hold from an upstream error's text:
/// the `user:password@` part of a URL, the value after a credential-looking
/// key (`password=`, `secret:`, ...), and anything past
/// [`MAX_LOGGED_CHARS`].
///
/// Best-effort by design: the real guarantee is that the text never reaches
/// a response. This keeps the obvious credential shapes out of the log too.
#[must_use]
pub fn redact(raw: &str) -> String {
    let truncated: String = raw.chars().take(MAX_LOGGED_CHARS).collect();
    redact_keys(&redact_userinfo(&truncated))
}

/// `scheme://user:pass@host` becomes `scheme://***@host`.
fn redact_userinfo(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find("://") {
        let (head, tail) = rest.split_at(pos + 3);
        out.push_str(head);
        // The authority ends at the first `/`, `?`, `#` or whitespace.
        let end = tail
            .find(|c: char| c == '/' || c == '?' || c == '#' || c.is_whitespace())
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        match authority.rfind('@') {
            Some(at) => {
                out.push_str("***");
                out.push_str(&authority[at..]);
            }
            None => out.push_str(authority),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// The words that mark the next value as a credential.
const SECRET_KEYS: [&str; 7] = [
    "password", "passwd", "pwd", "secret", "token", "apikey", "api_key",
];

/// `password=hunter2` and `"secret": "x"` become `password=***`.
fn redact_keys(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let hit = SECRET_KEYS
            .iter()
            .find(|key| lower[i..].starts_with(**key))
            .copied();
        let Some(key) = hit else {
            // Advance one char, not one byte, to stay on a boundary.
            let ch = text[i..].chars().next().map_or(1, char::len_utf8);
            out.push_str(&text[i..i + ch]);
            i += ch;
            continue;
        };
        let key_end = i + key.len();
        out.push_str(&text[i..key_end]);
        let mut j = key_end;
        // Skip a closing quote, spaces and one `=` or `:` separator.
        while j < bytes.len() && matches!(bytes[j], b'"' | b'\'' | b' ') {
            j += 1;
        }
        if j < bytes.len() && matches!(bytes[j], b'=' | b':') {
            j += 1;
            while j < bytes.len() && matches!(bytes[j], b'"' | b'\'' | b' ') {
                j += 1;
            }
            let value_end = text[j..]
                .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '&' | ',' | ';'))
                .map_or(text.len(), |off| j + off);
            out.push_str(&text[key_end..j]);
            out.push_str("***");
            i = value_end;
        } else {
            i = key_end;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::fmt::MakeWriter;

    use super::*;

    const MARKERS: [&str; 3] = [
        "Code: 999. DB::Exception: planted-marker-table-x",
        "connection refused to planted-marker-host:1234",
        "password authentication failed for user planted-marker-user",
    ];

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn with_captured_log<T>(f: impl FnOnce() -> T) -> (T, String) {
        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_ansi(false)
            .finish();
        let out = tracing::subscriber::with_default(subscriber, f);
        let log = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        (out, log)
    }

    #[test]
    fn the_fixed_message_never_contains_the_upstream_text() {
        for marker in MARKERS {
            for class in [Class::Failed, Class::Unavailable] {
                let (failure, _log) = with_captured_log(|| report(&TILE, class, &marker));
                assert!(!failure.to_string().contains(marker), "{marker}");
                assert!(!failure.to_json().to_string().contains(marker), "{marker}");
                let api = failure.into_api_error(FailedAs::Unprocessable);
                assert!(!api.to_string().contains(marker), "{marker}");
            }
        }
    }

    #[test]
    fn the_log_line_carries_the_reference_and_the_raw_text() {
        let (failure, log) = with_captured_log(|| report(&DATABASE, Class::Failed, &MARKERS[0]));
        assert!(log.contains(failure.reference()), "{log}");
        assert!(log.contains("planted-marker-table-x"), "{log}");
        assert!(log.contains("ERROR"), "{log}");
        assert_eq!(failure.reference().len(), 10);
    }

    #[test]
    fn unavailability_logs_at_warn_and_failure_at_error() {
        let (_f, log) = with_captured_log(|| report(&DATABASE, Class::Unavailable, &"down"));
        assert!(log.contains("WARN"), "{log}");
        assert!(!log.contains("ERROR"), "{log}");
    }

    #[test]
    fn the_json_shape_carries_the_message_and_the_reference() {
        let (failure, _log) = with_captured_log(|| report(&TILE, Class::Failed, &"x"));
        let json = failure.to_json();
        assert_eq!(json["error"], "This chart could not be loaded.");
        assert_eq!(json["errorId"], failure.reference());
    }

    #[test]
    fn failure_and_unavailability_map_to_the_statuses_used_today() {
        let (failed, _l) = with_captured_log(|| report(&DATABASE, Class::Failed, &"x"));
        assert_eq!(
            failed
                .clone()
                .into_api_error(FailedAs::Unprocessable)
                .status(),
            422
        );
        assert_eq!(
            failed.clone().into_api_error(FailedAs::BadRequest).status(),
            400
        );
        assert_eq!(failed.into_api_error(FailedAs::Internal).status(), 500);
        let (down, _l) = with_captured_log(|| report(&DATABASE, Class::Unavailable, &"x"));
        assert_eq!(
            down.clone()
                .into_api_error(FailedAs::Unprocessable)
                .status(),
            503
        );
        assert_eq!(down.into_api_error(FailedAs::Internal).status(), 503);
    }

    #[test]
    fn a_clickhouse_server_error_is_a_failure_and_cancellation_is_unavailability() {
        let (server, _l) =
            with_captured_log(|| report_ch(&TILE, &ChError::Server(MARKERS[0].to_owned())));
        assert_eq!(server.message(), TILE.failed);
        let (cancelled, _l) = with_captured_log(|| report_ch(&TILE, &ChError::Cancelled));
        assert_eq!(cancelled.message(), TILE.unavailable);
    }

    #[test]
    fn two_reports_get_different_references() {
        let (a, _l) = with_captured_log(|| report(&TILE, Class::Failed, &"x"));
        let (b, _l) = with_captured_log(|| report(&TILE, Class::Failed, &"x"));
        assert_ne!(a.reference(), b.reference());
    }

    #[test]
    fn our_own_validation_text_is_kept_and_an_upstream_error_is_replaced() {
        let (own, _l) = with_captured_log(|| {
            bi(
                &DATABASE,
                &lakehouse_bi::store::BiError::Validation("name is required".to_owned()),
            )
        });
        assert_eq!(own.to_json(), json!({ "error": "name is required" }));

        let (up, log) = with_captured_log(|| {
            alert(
                &DATABASE,
                &lakehouse_alerts::AlertError::Clickhouse(ChError::Server(MARKERS[0].to_owned())),
            )
        });
        assert!(!up.to_json().to_string().contains("planted-marker"));
        assert!(up.to_json()["errorId"].is_string());
        assert!(log.contains("planted-marker-table-x"), "{log}");

        let (db, log) = with_captured_log(|| {
            store(
                &STORE,
                &lakehouse_store::StoreError::Database(sqlx::Error::PoolClosed),
            )
        });
        assert!(matches!(&db, Reported::Upstream(f) if f.message() == STORE.unavailable));
        assert!(log.contains("WARN"), "{log}");
        let (nf, _l) = with_captured_log(|| store(&STORE, &lakehouse_store::StoreError::NotFound));
        assert_eq!(nf.to_json(), json!({ "error": "record not found" }));
    }

    #[test]
    fn redact_strips_url_credentials_and_secret_values() {
        let out = redact("connect postgres://admin:hunter2@db.internal:5432/x failed");
        assert!(!out.contains("hunter2"), "{out}");
        assert!(!out.contains("admin"), "{out}");
        assert!(out.contains("@db.internal:5432/x"), "{out}");

        let out = redact("bad conn string host=h password=hunter2 user=u");
        assert!(!out.contains("hunter2"), "{out}");
        assert!(out.contains("user=u"), "{out}");

        let out = redact(r#"{"client_secret": "abc123", "ok": 1}"#);
        assert!(!out.contains("abc123"), "{out}");
        assert!(out.contains("\"ok\": 1"), "{out}");
    }

    #[test]
    fn redact_keeps_plain_text_and_multibyte_characters_intact() {
        assert_eq!(
            redact("Table serving.x doesn't exist é ü"),
            "Table serving.x doesn't exist é ü"
        );
    }

    #[test]
    fn redact_truncates_a_very_long_error() {
        let long = "a".repeat(MAX_LOGGED_CHARS + 500);
        assert_eq!(redact(&long).chars().count(), MAX_LOGGED_CHARS);
    }

    #[test]
    fn the_author_diagnosis_drops_the_version_suffix_and_keeps_the_code_name() {
        let raw = "Code: 47. DB::Exception: Unknown identifier 'nope' in scope SELECT nope FROM serving.t. (UNKNOWN_IDENTIFIER) (version 24.8.1.1)";
        let out = author_diagnosis(raw);
        assert!(out.contains("Unknown identifier 'nope'"), "{out}");
        assert!(out.ends_with("(UNKNOWN_IDENTIFIER)"), "{out}");
        assert!(!out.contains("version"), "{out}");
    }

    #[test]
    fn the_author_diagnosis_drops_hosts_and_addresses_but_not_positions() {
        let raw = "Code: 210. DB::Exception: connect to ch-int.example.net:9000 (10.1.2.3:9000) failed at http://10.1.2.3:8123/ line 1:5 [::1] serving.t";
        let out = author_diagnosis(raw);
        for gone in ["ch-int.example", "10.1.2.3", "http://", "::1]"] {
            assert!(!out.contains(gone), "{gone} in {out}");
        }
        assert!(out.contains("line 1:5"), "{out}");
        assert!(out.contains("serving.t"), "{out}");
        assert!(out.contains("DB::Exception"), "{out}");
    }

    #[test]
    fn the_author_diagnosis_keeps_only_the_first_line_and_no_credentials() {
        let out = author_diagnosis(
            "Code: 1. DB::Exception: bad password=hunter2 here\nStack trace:\n0. foo",
        );
        assert!(!out.contains("hunter2") && !out.contains("Stack"), "{out}");
    }

    #[test]
    fn only_a_query_error_body_is_a_statement_error() {
        assert!(is_statement_error(
            "Code: 62. DB::Exception: Syntax error (SYNTAX_ERROR)"
        ));
        assert!(!is_statement_error("ClickHouse HTTP 502"));
        assert!(!is_statement_error("<html>Bad gateway</html>"));
        assert!(!is_statement_error(
            "Code: 159. DB::Exception: Timeout exceeded (TIMEOUT_EXCEEDED)"
        ));
    }

    #[test]
    fn the_author_exception_leaves_an_outage_as_the_fixed_message() {
        let down = ChError::Server("ClickHouse HTTP 503".to_owned());
        let api = ch_error_for_author(&DATABASE, &down);
        assert!(
            api.to_string()
                .starts_with("The database request failed. Reference: ")
        );
        assert!(!api.to_string().contains("503"));
        let msg = ch_message_for_author(&DATABASE, &ChError::Cancelled);
        assert!(
            msg.starts_with("The database is unavailable. Reference: "),
            "{msg}"
        );
    }

    #[test]
    fn the_real_clickhouse_version_suffix_is_cut_whole() {
        for suffix in [
            "(version 25.8.1.3 (official build))",
            "(version 24.3.2 (official build))",
            "(version 24.3.2)",
        ] {
            let raw = format!(
                "Code: 47. DB::Exception: Unknown expression identifier `nope` in scope SELECT nope FROM serving.t. (UNKNOWN_IDENTIFIER) {suffix}"
            );
            let out = author_diagnosis(&raw);
            assert!(out.ends_with("(UNKNOWN_IDENTIFIER)"), "{out}");
            for gone in ["version", "<host>", "official build"] {
                assert!(!out.contains(gone), "{gone} in {out}");
            }
        }
    }
}
