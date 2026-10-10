//! `SEC-11-AC1`: no route builds a response from an upstream error's text.
//!
//! # What this checks
//!
//! A source scan of `src/routes/**` (not of behaviour: the behaviour is in
//! `tests/upstream_errors.rs`). It flags a non-test line that puts an error
//! value's own text into something a response could carry:
//!
//! - `err.to_string()` (and `e`, `error`, `source`, `cause`, `why`, `detail`),
//! - `{err}` / `{e}` / `{error}` / `{source}` / `{cause}` interpolation, also
//!   with `:?`.
//!
//! Lines inside a logging macro (`tracing::warn!(...)`, `error!(...)`, even
//! over several lines) are exempt, as are calls through
//! `upstream_message` / `upstream_error::*`, which are the choke point.
//!
//! # The allowlist
//!
//! [`ALLOWED`] names, per file, a substring of the line and the reason it is
//! our own text rather than an upstream's. There is no blanket entry: a new
//! line needs its own reason. An entry that no longer matches any line is a
//! failure too, so the list cannot go stale and quietly cover a new leak.
//!
//! # Limits
//!
//! This is a lexical scan. It cannot see an error converted to a `String` in
//! one function and returned from another, nor a variable named something
//! else. The compile-time half of the guard is that `lakehouse-clickhouse`
//! has no `From<ChError> for ApiError`, so a `?` on a `ChError` does not
//! compile. Code review covers the rest.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

/// `(file under src/routes, substring of the flagged line, why it is ours)`.
const ALLOWED: &[(&str, &str, &str)] = &[
    // serde's message about the CALLER's own request body: not an upstream payload.
    (
        "agents.rs",
        "invalid JSON: {err}",
        "serde error over the caller's own request body",
    ),
    (
        "alerts.rs",
        "JSON is invalid: {err}",
        "serde error over the caller's own request body",
    ),
    (
        "auth.rs",
        "invalid JSON body: {err}",
        "serde error over the caller's own request body",
    ),
    (
        "catalog_query.rs",
        "invalid `filters` JSON: {err}",
        "serde error over the caller's own query parameter",
    ),
    (
        "catalog_query.rs",
        "invalid `sort` JSON: {err}",
        "serde error over the caller's own query parameter",
    ),
    (
        "connectors.rs",
        "invalid JSON: {err}",
        "serde error over the caller's own request body",
    ),
    (
        "dashboard.rs",
        "JSON is invalid: {err}",
        "serde error over the caller's own request body",
    ),
    (
        "governance.rs",
        "invalid JSON: {err}",
        "serde error over the caller's own request body",
    ),
    (
        "identity.rs",
        "invalid JSON: {err}",
        "serde error over the caller's own request body",
    ),
    (
        "knowledge.rs",
        "invalid JSON: {err}",
        "serde error over the caller's own request body",
    ),
    (
        "overview.rs",
        "invalid JSON: {err}",
        "serde error over the caller's own request body",
    ),
    (
        "pipelines.rs",
        "invalid JSON: {err}",
        "serde error over the caller's own request body",
    ),
    (
        "pipelines.rs",
        "invalid retry body: {err}",
        "serde error over the caller's own request body",
    ),
    // SEC-11, product owner decision 2026-10-10: Query Studio (`/api/query/run`
    // for both engines, and the cost estimate) shows the signed-in author the
    // engine's diagnosis of their own statement, trimmed. Only this file.
    (
        "query.rs",
        "_for_author(",
        "query-author exception: the author of the statement is shown the trimmed engine diagnosis",
    ),
    (
        "query.rs",
        "invalid JSON: {err}",
        "serde error over the caller's own request body",
    ),
    (
        "quality.rs",
        "Invalid request body: {err}",
        "serde error over the caller's own request body",
    ),
    (
        "storage.rs",
        "invalid JSON: {err}",
        "serde error over the caller's own request body",
    ),
    // Our own validators: the text is written in this repository.
    (
        "authored_pipelines.rs",
        "invalid transform at transforms[{index}]: {err}",
        "transform_grammar's own validation message",
    ),
    (
        "pipelines.rs",
        "invalid transform at transforms[{index}]: {err}",
        "transform_grammar's own validation message",
    ),
    (
        "gold.rs",
        "invalid mart: {e}",
        "lakehouse_core::ident's own validation message about the caller's mart name",
    ),
    (
        "gold.rs",
        "GoldExportError::PolicyRefused(_) => Self::Unprocessable(err.to_string())",
        "policy_engine::enforcement_error_message's fixed text (WS7 item D2)",
    ),
    (
        "connectors.rs",
        "is not an RFC 3339 time: {err}",
        "chrono's parse message about the caller's own time string",
    ),
    (
        "connectors.rs",
        "credential.values.primary: {err}",
        "validate_secret_value's own value-free message",
    ),
    (
        "connectors.rs",
        "credential.values.secondary: {err}",
        "validate_secret_value's own value-free message",
    ),
    (
        "connectors.rs",
        "{}: {err}\", slot.as_str()",
        "validate_secret_value's own value-free message",
    ),
    (
        "connectors.rs",
        "SecretStoreError::InvalidValue(_) => ApiError::BadRequest(err.to_string())",
        "SecretStoreError messages are value-free by definition (its doc comment)",
    ),
    (
        "connectors.rs",
        "SecretStoreError::NotManaged => ApiError::Internal(err.to_string())",
        "SecretStoreError messages are value-free by definition",
    ),
    (
        "connectors.rs",
        "SecretStoreError::Io(_) => ApiError::Unavailable(err.to_string())",
        "SecretStoreError messages are value-free by definition",
    ),
    (
        "connectors.rs",
        "\"dial: {err}\"",
        "IngestSpecError never echoes the dial's values (ingest_spec module doc)",
    ),
    (
        "connectors.rs",
        "dial does not parse as its own adapter {adapter:?}: {err}",
        "IngestSpecError never echoes the dial's values",
    ),
    (
        "connectors.rs",
        "dial does not parse as a mongodb dial: {err}",
        "IngestSpecError never echoes the dial's values",
    ),
    (
        "connectors.rs",
        "fields are invalid: {err}",
        "debezium template validation's own message",
    ),
    (
        "connectors.rs",
        "does not map to a valid CDC slug: {err}",
        "slug validation's own message",
    ),
    (
        "connectors.rs",
        "did not complete ({err})",
        "SEC-15: a fixed sentence from connector_deprovision, never driver text",
    ),
    (
        "connectors.rs",
        ".map_err(|err| ApiError::BadRequest(err.to_string()))?;",
        "validate_load_modes' own message about the caller's source objects",
    ),
    // StoreError's Display is fixed text ("database error", ...), see lakehouse-store's error.rs.
    (
        "alerts.rs",
        ".map_err(|err| err.to_string())",
        "StoreError's Display is fixed text and is the LateSource contract",
    ),
    (
        "pipelines.rs",
        ".map_err(|err| ApiError::Unavailable(err.to_string()))?;",
        "StoreError's Display is fixed text",
    ),
    // pipeline_source::SourceError: written in this repository.
    (
        "pipelines.rs",
        "ApiJson(json!({ \"error\": err.to_string() })),",
        "SourceError::CommitMismatch / UnverifiableProvenance are our own sentences",
    ),
    // Classification only: the text is inspected, never returned.
    (
        "governance.rs",
        "let text = err.to_string();",
        "inspected for UNKNOWN_TABLE to pick a fixed message; never returned",
    ),
    // The assistant's tools.
    (
        "ai/tools/connectors.rs",
        "Err(err) => return json!({ \"error\": err.to_string() })",
        "serialising our own serde_json::Value: infallible, not an upstream",
    ),
    (
        "ai/tools/governance.rs",
        "Err(err) => return json!({ \"error\": err.to_string() })",
        "serialising our own serde_json::Value: infallible, not an upstream",
    ),
    (
        "ai/tools/dashboards.rs",
        ".map_err(|e| e.to_string())",
        "serde error over the model's own tool arguments",
    ),
    (
        "ai/tools/data.rs",
        "err.to_string().contains(\"UNKNOWN_DATABASE\")",
        "classification only: picks the supported:false answer",
    ),
    (
        "ai/tools/gold.rs",
        "mart tidak valid: {e}",
        "lakehouse_core::ident's own validation message",
    ),
    (
        "ai/tools/gold.rs",
        ".map_err(|err| json!({ \"error\": err.to_string() }))?;",
        "read_catalog_token's ApiError text, written in this repository",
    ),
];

/// Calls that are the choke point: a line using one is not a leak.
const CHOKE_POINTS: &[&str] = &["upstream_message", "upstream_error::"];

/// A flagged line: file (relative to `src/routes`), 1-based line number, text.
#[derive(Debug, PartialEq, Eq)]
struct Hit {
    file: String,
    line: usize,
    text: String,
}

fn routes_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/routes")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read src/routes") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Whether `line` interpolates or stringifies an error-named value.
fn mentions_error_text(line: &str) -> bool {
    const NAMES: &[&str] = &["err", "e", "error", "source", "cause", "why", "detail"];
    for name in NAMES {
        if line.contains(&format!("{{{name}}}")) || line.contains(&format!("{{{name}:?}}")) {
            return true;
        }
        let call = format!("{name}.to_string()");
        let mut from = 0;
        while let Some(pos) = line[from..].find(&call) {
            let at = from + pos;
            // Whole identifier only: `layer.to_string()` is not `r.to_string()`.
            let before = line[..at].chars().next_back();
            if !before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
                return true;
            }
            from = at + call.len();
        }
    }
    false
}

/// Scans `source` (one file's text) and returns the flagged lines.
///
/// Stops at the first column-0 `#[cfg(test)]` followed by a `mod`: everything after it must be
/// test modules, and a column-0 `fn`/`impl`/`struct`/... after it would be
/// code the scan silently skips, so that panics instead.
fn scan(file: &str, source: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut in_tests = false;
    // Parenthesis depth of a logging macro call that spans lines.
    let mut log_depth: i32 = 0;
    for (index, line) in lines.iter().copied().enumerate() {
        // `#[cfg(test)] mod ...` starts the test tail; a `#[cfg(test)] use`
        // or `fn` is a single test-only item and the scan carries on.
        if line == "#[cfg(test)]"
            && lines
                .get(index + 1)
                .is_some_and(|next| next.starts_with("mod ") || next.starts_with("pub mod "))
        {
            in_tests = true;
        }
        if in_tests {
            let item = [
                "fn ",
                "pub fn ",
                "pub(crate) fn ",
                "async fn ",
                "pub async fn ",
                "impl",
                "struct ",
                "pub struct ",
                "enum ",
                "pub enum ",
                "const ",
                "pub const ",
                "static ",
                "use ",
            ];
            assert!(
                !item.iter().any(|p| line.starts_with(p)),
                "{file}:{}: non-test item after a #[cfg(test)] module; the SEC-11 scan would skip it",
                index + 1
            );
            continue;
        }
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            continue;
        }
        let logging_start = [
            "tracing::warn!(",
            "tracing::error!(",
            "tracing::info!(",
            "tracing::debug!(",
            "tracing::trace!(",
            " warn!(",
            " error!(",
            " info!(",
            " debug!(",
        ]
        .iter()
        .any(|m| line.contains(m));
        let in_log = logging_start || log_depth > 0;
        if in_log {
            let opens = i32::try_from(line.matches('(').count()).unwrap_or(0);
            let closes = i32::try_from(line.matches(')').count()).unwrap_or(0);
            log_depth = (log_depth + opens - closes).max(0);
            if logging_start && log_depth == 0 {
                continue;
            }
            continue;
        }
        // The query-author exception (product owner, 2026-10-10): these
        // functions return the engine's own text, so they are flagged before
        // the choke-point skip and need an ALLOWED entry per file.
        if line.contains("_for_author(") {
            hits.push(Hit {
                file: file.to_owned(),
                line: index + 1,
                text: trimmed.to_owned(),
            });
            continue;
        }
        if CHOKE_POINTS.iter().any(|c| line.contains(c)) {
            continue;
        }
        if mentions_error_text(line) {
            hits.push(Hit {
                file: file.to_owned(),
                line: index + 1,
                text: trimmed.to_owned(),
            });
        }
    }
    hits
}

#[test]
fn no_route_builds_a_response_from_an_upstream_errors_text() {
    let base = routes_dir();
    let mut files = Vec::new();
    rust_files(&base, &mut files);
    files.sort();

    let mut leaks = Vec::new();
    let mut used = vec![false; ALLOWED.len()];
    for path in &files {
        let rel = path
            .strip_prefix(&base)
            .expect("under src/routes")
            .to_string_lossy()
            .replace('\\', "/");
        let source = fs::read_to_string(path).expect("read a route file");
        for hit in scan(&rel, &source) {
            let allowed = ALLOWED
                .iter()
                .enumerate()
                .find(|(_, (file, pattern, _))| *file == hit.file && hit.text.contains(pattern));
            match allowed {
                Some((i, _)) => used[i] = true,
                None => leaks.push(hit),
            }
        }
    }

    let stale: Vec<_> = ALLOWED
        .iter()
        .zip(&used)
        .filter(|(_, used)| !**used)
        .map(|((file, pattern, _), _)| format!("{file}: {pattern}"))
        .collect();

    assert!(
        leaks.is_empty(),
        "SEC-11-AC1: these route lines put an error's own text into a response. Report the \
         error with `upstream_error` (log the raw text, return a fixed message and a reference \
         id), or, if the text is written in this repository, add an entry with the reason to \
         ALLOWED in tests/sec11_guard.rs:\n{}",
        leaks
            .iter()
            .map(|h| format!("  src/routes/{}:{}: {}", h.file, h.line, h.text))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        stale.is_empty(),
        "ALLOWED entries that match no line any more (remove them so they cannot cover a new \
         leak):\n  {}",
        stale.join("\n  ")
    );
}

#[test]
fn the_scan_flags_the_shapes_the_audit_found() {
    for line in [
        r#"        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, ApiJson(json!({ "error": err.to_string() }))).into_response(),"#,
        "        .map_err(|err| ApiError::Internal(err.to_string()))?;",
        r#"        store_error = Some(format!("Error: {err}"));"#,
        r#"        let detail = format!("AI Copilot unavailable: {err:?}");"#,
        "        .map_err(|e| ApiError::BadRequest(e.to_string()))?;",
    ] {
        assert_eq!(scan("x.rs", line).len(), 1, "not flagged: {line}");
    }
}

#[test]
fn the_scan_flags_the_query_author_exception_wherever_it_is_called() {
    let line = "        .map_err(|e| upstream_error::ch_error_for_author(&DATABASE, &e))?;";
    assert_eq!(scan("dashboard.rs", line).len(), 1);
}

#[test]
fn the_scan_leaves_the_choke_point_logging_and_tests_alone() {
    for source in [
        "        .map_err(|err| upstream_error::ch_error(&DATABASE, &err))?;",
        r#"        ApiJson(json!({ "error": upstream_message(err) }))"#,
        r#"        tracing::warn!(%err, "lookup failed");"#,
        "tracing::warn!(\n    error = %err.to_string(),\n    \"lookup failed\"\n);",
        "        // err.to_string() is described here, not called",
        "#[cfg(test)]\nmod tests {\n    fn t() { let m = err.to_string(); }\n}",
        "        let layer = other_err.to_string();",
    ] {
        assert!(scan("x.rs", source).is_empty(), "wrongly flagged: {source}");
    }
}
