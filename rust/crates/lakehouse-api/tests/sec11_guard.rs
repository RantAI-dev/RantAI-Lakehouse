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
    // ADM-11 review 1 B4: password rule validation error messages are written in this repo
    (
        "auth.rs",
        ".map_err(|err| ApiError::BadRequest(err.to_string()))?;",
        "password::check_rules validation error text, written in this repository (ADM-11 D4)",
    ),
    (
        "identity.rs",
        ".map_err(|err| ApiError::BadRequest(err.to_string()))?;",
        "password::check_rules validation error text, written in this repository (ADM-11 D4)",
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
        "dashboard.rs",
        "filters is invalid: {err}",
        "serde error over the caller's own query parameter (BI-18, the dashboard `filters` list)",
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
    // SEC-11, product owner decision 2026-10-10, ended for the console by the
    // owner on 2026-11-12 (LH-QS-2-08): the console's `run` and cost estimate
    // answer a fixed sentence and a reference (`_for_console`). The assistant's
    // paths in this file still give the model the trimmed engine diagnosis so
    // it can correct its SQL: `run_statement` (`FailureText::Assistant`) and
    // `check_statement_would_run`. Only this file.
    (
        "query.rs",
        "_for_author(",
        "query-author exception, assistant paths only: the model is shown the trimmed engine diagnosis to correct its SQL",
    ),
    // BLD-1 part 1 / D16 and the spec's rule 7: a preview reports an error the
    // way `POST /api/query/run` reports one for the same SQL. The person
    // previewing is the one who wrote the step's SQL, so the engine's trimmed
    // diagnosis of it is theirs — the same exception Query Studio has, through
    // the same function (`upstream_error::ch_failure_for_author`), never a
    // second classification.
    (
        "pipeline_preview.rs",
        "_for_author(",
        "BLD-1 part 1 D16 / spec rule 7: the caller wrote the previewed step's SQL, so the engine's trimmed diagnosis is theirs — the same query-author exception as Query Studio, through the same function",
    ),
    (
        "query.rs",
        ".map_err(|err| err.to_string())?;",
        "rewrite_sql_for_principal's ApiError: the policy's own non-leaking message, written in this repository",
    ),
    // `run` records every answered run. This line writes the history row for a
    // run the policy rewrite refused or failed: its `err` is the `ApiError`
    // `rewrite_sql_for_principal_with_headers` returns, so `to_string()` is
    // the same fixed text the adjacent `return Err(err.into())` hands the
    // response. The row goes to the caller's own history, not into a
    // response. An engine failure no longer passes here: `EngineFailure`
    // carries the classified sentence the row keeps, never the engine's text
    // (`tests/query_history.rs` pins that).
    (
        "query.rs",
        "&err.to_string(),",
        "the history row keeps the policy rewrite's own fixed ApiError text, the same the response carries",
    ),
    (
        "query.rs",
        "invalid JSON: {err}",
        "serde error over the caller's own request body",
    ),
    // `SEC-50` slice 2 M1: the gate keeps its refusal for the rest of the
    // request so every statement of it is refused the same way. What is kept
    // is the gate's own fixed sentence (`SEC-20`'s 404 naming the table, D6's
    // 422 naming the setting), never upstream text — see `GateRefusal`'s doc
    // comment.
    (
        "query.rs",
        "message: err.to_string(),",
        "GateRefusal keeps the gate's own fixed sentence, written in this repository, to rebuild the same refusal",
    ),
    // `SEC-50` slice 2 M3: `SpecRunFailure::Gated` carries the tenant gate's
    // own `ApiError`, whose sentence is fixed text this repository wrote (the
    // enum's doc comment says so). It is the same sentence the route itself
    // answers with, so it belongs in the tile's, the preview's and the
    // layer's error slot; no engine text is involved.
    (
        "dashboard_sources.rs",
        "Err(SpecRunFailure::Gated(err)) => json!({ \"error\": err.to_string() }),",
        "the tenant gate's own fixed sentence (SEC-20's 404 or D6's 422), written in this repository",
    ),
    (
        "layers.rs",
        "SpecRunFailure::Gated(err) => json!({ \"error\": err.to_string() }),",
        "the tenant gate's own fixed sentence (SEC-20's 404 or D6's 422), written in this repository",
    ),
    (
        "support.rs",
        "Err(SpecRunFailure::Gated(err)) => json!({ \"error\": err.to_string() }),",
        "the tenant gate's own fixed sentence (SEC-20's 404 or D6's 422), written in this repository",
    ),
    (
        // `BI-45` (QA `LH-BI-5-01`): the assistant's reply for a save the gate
        // refused. The same gate error `run_spec_sql` shows in its tile above.
        "support.rs",
        "SpecRunFailure::Gated(err) => json!({ \"error\": err.to_string() }),",
        "the tenant gate's own fixed sentence (SEC-20's 404 or D6's 422), written in this repository",
    ),
    (
        "quality.rs",
        "Invalid request body: {err}",
        "serde error over the caller's own request body",
    ),
    // DATA-14: `detail` is built by our own code from numbers and our own rule/asset names
    // (`open_for_failed_run`: rule name, `Verdict::value`, threshold; `open_anomaly`: the
    // `detail()` sentences of `quality_anomaly`); no ClickHouse or other upstream text.
    (
        "quality_incidents.rs",
        "let text = format!(\"{asset}: {detail}. View: /governance/data-quality\");",
        "asset key and incident detail are written in this repository from counts, percentages and rule names, never upstream error text",
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
        "ApiError::Unavailable(err.to_string()) // SEC-11 allowlisted",
        "SRC-12 A1, `secret_store_error`'s Io and Key arms: SecretStoreError::Io prints only the std::io::ErrorKind variant name (`{0:?}` of an enum, not the OS message) in a sentence written in this repository, and SecretStoreError::Key prints a CryptoError, whose every variant is a fixed sentence naming the setting (secret_crypto.rs), never a key",
    ),
    (
        "connectors.rs",
        "ApiError::BadRequest(err.to_string()) // SEC-11 allowlisted",
        "SRC-11 D9-D11, `InvalidExcludedColumns::BadList`: a fixed sentence (ingest_spec.rs) that names only the source object the request itself sent",
    ),
    (
        "connectors.rs",
        "_ => ApiError::Unprocessable(err.to_string()), // SEC-11 allowlisted",
        "SRC-11 D9-D11, the other InvalidExcludedColumns variants: fixed sentences (ingest_spec.rs) that name only the object and column the request itself sent",
    ),
    (
        "connectors.rs",
        "The connection settings are not valid: {source}",
        "IngestSpecError::InvalidDial's source: a fixed sentence of ingest_spec.rs or serde's field text about the caller's own dial; nothing is dialled before it, so it holds no upstream text and no resolved address (it can repeat a host the caller typed)",
    ),
    (
        "connectors.rs",
        "\"dial: {err}\"",
        "IngestSpecError never echoes the dial's values (ingest_spec module doc)",
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
    // `BI-39` D6: the statement gate's own refusals (SEC-20's 404, D6's 422,
    // the classified 503) are sentences written in this repository, and the
    // alerts crate records one as a rule's skipped reason — never engine text.
    (
        "alerts.rs",
        "GateRefusal::Refused(err.to_string())",
        "StatementGate::check answers with our own fixed sentences, never an upstream payload",
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
    // BI-19 B2 (PR #250 CI): the shared helpers in ai/tools/mod.rs.
    (
        "ai/tools/mod.rs",
        ".map_err(|err| json!({ \"error\": err.to_string() }))",
        "an ApiError from routes::dashboard_access (viewer_for, scope_for_write, require_board_level): fixed texts written in this repository, a store failure is already \"database error\" through From<StoreError>",
    ),
    (
        "ai/tools/alerts.rs",
        "json!({ \"error\": err.to_string() })",
        "an ApiError from routes::alerts' scope checks and visible_rules: fixed texts written in this repository, a store failure is already \"database error\"",
    ),
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
        "cause.to_string()",
        "serde error over the model's own tool arguments, the same class as the look.rs entry below: it names the field the model must fix, never a payload from Postgres, Iceberg or ClickHouse",
    ),
    (
        "ai/tools/dashboards.rs",
        "{field}\\\": {cause}",
        "the same serde error over the model's own tool arguments, in front of the field's name",
    ),
    (
        "ai/tools/pipelines.rs",
        "invalid targetLag: {err}",
        "serde error over the model's own tool arguments, the same class as dashboards.rs above: it tells the model which of its `targetLag` fields is wrong, never a payload from Postgres, Iceberg or ClickHouse",
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
    // The tool's own decode of the model's arguments: serde reporting a
    // shape it cannot parse out of `filters`, which the caller supplied.
    // Same class as the `invalid JSON: {err}` entries above -- our own
    // schema describing the caller's input, never an upstream payload.
    (
        "ai/tools/boards.rs",
        "format!(\"filters: {err}\")",
        "serde error over the caller's own tool arguments",
    ),
    // The tool's own decode of the model's formatting arguments: serde
    // reporting a shape it cannot parse out of the formatting block the
    // model sent. Same class as the `ai/tools/dashboards.rs` entry above --
    // our own schema describing the caller's input, never an upstream
    // payload.
    (
        "ai/tools/look.rs",
        "cause.to_string()",
        "serde error over the model's own formatting arguments, the same class as dashboards.rs above: it names the field the model must fix, never a payload from Postgres, Iceberg or ClickHouse",
    ),
    (
        "ai/tools/look.rs",
        "a formatting field has the wrong shape: ",
        "the same serde error over the model's own formatting arguments, in front of the field's name",
    ),
    // BI-26 slice 2 D4: `embed::check_filter_modes` returns an `ApiError` whose
    // text the settings route writes itself (a column that is no filter, or a
    // mode on a range filter). It is the route's sentence, not an upstream's.
    (
        "ai/tools/embed.rs",
        "return json!({ \"error\": err.to_string() });",
        "check_filter_modes' ApiError: the settings route's own fixed sentences, written in this repository",
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
    let line =
        "        .map_err(|e| upstream_error::ch_failure_for_author(&DATABASE, &e).response)?;";
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
