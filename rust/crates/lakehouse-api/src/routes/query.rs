//! `POST /api/query/run`, `POST /api/query/estimate` — the ad hoc `SQL`
//! Query Studio.
//!
//! Ports `src/app/api/query/run/route.ts` and
//! `src/app/api/query/estimate/route.ts`.

use std::time::Instant;

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use lakehouse_auth::Principal;
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::queries::{self, QueryHistoryItem, RecordHistoryInput, SavedQuery};
use lakehouse_trino::{TrinoError, TrinoResult};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;

/// The `{sql}` request body both routes accept.
#[derive(Debug, Deserialize)]
struct SqlBody {
    #[serde(default)]
    sql: Option<String>,
    /// Which engine should run this statement: `"clickhouse"` (the
    /// default, when absent) or `"trino"` (WS2 §4). Only [`run`] reads
    /// this — `estimate` always runs `EXPLAIN ESTIMATE` against
    /// `ClickHouse`, unaffected.
    #[serde(default)]
    engine: Option<String>,
}

/// Parse the raw request body as `{"sql": "..."}`.
///
/// Both routes share one `try { ({ sql } = await req.json()) } catch { ...
/// "Body must be JSON {sql}" ... }` shape in the `TypeScript`: any body that
/// doesn't parse as JSON at all — not merely a body missing `sql` — is a
/// 400 with this exact message.
fn parse_body(body: &Bytes) -> Result<SqlBody, ApiError> {
    serde_json::from_slice(body)
        .map_err(|_err| ApiError::BadRequest("Body must be JSON {sql}".to_owned()))
}

/// Whether `sql` is a read-only statement `ClickHouse` may run from Query
/// Studio.
///
/// Ports the guard in `query/run/route.ts` verbatim, including its two
/// quirks:
///
/// - It tests the *whole string* for smuggled DML keywords, so
///   `SELECT 1; DELETE FROM t` is rejected even though the statement
///   *starts* with `SELECT`.
/// - The leading-keyword test anchors to the very start of the (trimmed)
///   string, so a leading comment before a permitted keyword — e.g.
///   `/* c */ SELECT 1` — fails that test and is rejected too, even though
///   the statement contains no DML at all. This is almost certainly an
///   accidental over-restriction upstream (comments are harmless), but the
///   golden corpus captured this exact behavior, so it is reproduced
///   as-is rather than "fixed".
#[must_use]
fn is_read_only(sql: &str) -> bool {
    let starts_with_allowed = starts_with_allowed_keyword(sql);
    let contains_dml = contains_denied_keyword(sql);
    starts_with_allowed && !contains_dml
}

/// `/^\s*(with|select|show|describe|desc|explain)\b/i.test(sql)` — leading
/// whitespace, then one of the allowed keywords, then a word boundary.
fn starts_with_allowed_keyword(sql: &str) -> bool {
    const ALLOWED: [&str; 6] = ["with", "select", "show", "describe", "desc", "explain"];
    let trimmed = sql.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    ALLOWED.iter().any(|kw| {
        lower
            .strip_prefix(kw)
            .is_some_and(|rest| rest.chars().next().is_none_or(|c| !is_word_char(c)))
    })
}

/// `/\b(insert|alter|drop|delete|update|create|truncate|rename|attach|detach|grant|revoke)\b/i.test(sql)`
/// — the whole string, any position, word-boundary delimited.
fn contains_denied_keyword(sql: &str) -> bool {
    const DENIED: [&str; 12] = [
        "insert", "alter", "drop", "delete", "update", "create", "truncate", "rename", "attach",
        "detach", "grant", "revoke",
    ];
    let lower = sql.to_ascii_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    DENIED.iter().any(|kw| word_occurs(&chars, kw))
}

/// Whether `chars` (already lowercased) contains `word` at a position
/// bounded by non-word characters (or the string's edges) on both sides —
/// `\b<word>\b` in a case-insensitive regex.
fn word_occurs(chars: &[char], word: &str) -> bool {
    let word_chars: Vec<char> = word.chars().collect();
    let n = word_chars.len();
    if n == 0 || chars.len() < n {
        return false;
    }
    for start in 0..=(chars.len() - n) {
        if chars[start..start + n] == word_chars[..] {
            let before_ok = start == 0 || !is_word_char(chars[start - 1]);
            let after_ok = start + n == chars.len() || !is_word_char(chars[start + n]);
            if before_ok && after_ok {
                return true;
            }
        }
    }
    false
}

/// `\w` in a JavaScript regex: `[A-Za-z0-9_]`.
fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Whether `sql` is additionally safe to send to `Trino`, on top of
/// [`is_read_only`] (checked first, unconditionally, for every engine —
/// this function is never the only gate). Applied only when the request
/// names `engine: "trino"`.
///
/// [`is_read_only`]'s denylist was written for `ClickHouse` and is not
/// sufficient here, for two reasons specific to `Trino`:
///
/// - `Trino`'s `EXPLAIN ANALYZE` *executes* the statement it explains,
///   unlike `ClickHouse`'s `EXPLAIN`, which never runs anything. A
///   statement starting `EXPLAIN ANALYZE <write>` passes
///   [`is_read_only`]'s "starts with an allowed keyword" test and, unless
///   `<write>` happens to use one of the 12 `ClickHouse`-oriented denied
///   words, sails through it entirely — see [`contains_explain_analyze`].
/// - `Trino` speaks `SQL` statements `ClickHouse` doesn't have, several of
///   which write: `MERGE`, `REFRESH MATERIALIZED VIEW`, `COMMENT ON`,
///   `CALL` (procedures, including maintenance ones), `EXECUTE`/`PREPARE`/
///   `DEALLOCATE` (prepared statements, a smuggling vector for anything
///   above), `SET`/`RESET` (session state), and `DENY` (access control
///   itself). None of these are in [`is_read_only`]'s `ClickHouse`-shaped
///   denylist, so they are refused here instead.
///
/// This guard is the console's OWN boundary, not `Trino`'s. `Trino`'s
/// file-based access control (the compose `trino` service's
/// `rules.json`) restricts the `iceberg` catalog to `SELECT` for every
/// user except `trino-maintenance`, but it does NOT block `ALTER TABLE …
/// EXECUTE optimize` for a plain reader — Trino's
/// `checkCanExecuteTableProcedure` requires only table `SELECT`, with no
/// separate privilege for maintenance procedures (see the compose file's
/// block comment on `rules.json` for the measured proof). So this guard
/// is what actually stops a `query:read` principal from running `ALTER
/// TABLE … EXECUTE optimize` through the console, not `Trino` itself.
#[must_use]
fn is_trino_safe(sql: &str) -> bool {
    !contains_explain_analyze(sql) && !contains_trino_denied_keyword(sql)
}

/// Whether `sql` contains `explain`, followed — modulo any amount of
/// whitespace, and case-insensitively — by `analyze`, as adjacent words.
/// `Trino` accepts arbitrary whitespace between the two keywords
/// (`EXPLAIN   ANALYZE`, `explain\nanalyze`, ...), so the comparison
/// normalizes every whitespace run to a single space before matching.
fn contains_explain_analyze(sql: &str) -> bool {
    let lower = sql.to_ascii_lowercase();
    let collapsed = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    // Sentinel spaces at both ends turn the substring check into a
    // word-bounded match with no separate boundary logic needed.
    format!(" {collapsed} ").contains(" explain analyze ")
}

/// The `Trino`-specific write/session/procedure vocabulary
/// [`is_trino_safe`] refuses, checked as whole words with [`word_occurs`]
/// — the same helper [`contains_denied_keyword`] uses, rather than a
/// second matcher.
fn contains_trino_denied_keyword(sql: &str) -> bool {
    const TRINO_DENIED: [&str; 10] = [
        "merge",
        "refresh",
        "comment",
        "call",
        "execute",
        "deny",
        "set",
        "reset",
        "prepare",
        "deallocate",
    ];
    let lower = sql.to_ascii_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    TRINO_DENIED.iter().any(|kw| word_occurs(&chars, kw))
}

/// `POST /api/query/run` — execute a read-only `SQL` statement against
/// `ClickHouse` (`engine` absent or `"clickhouse"`) or `Trino`
/// (`engine: "trino"`, WS2 §4) and return it in `QueryResult` shape.
///
/// # Errors
///
/// - 400 [`ApiError::BadRequest`] on an unparseable body or a missing/empty
///   `sql`.
/// - 401 [`ApiError::Unauthorized`] with no authenticated principal, for
///   ANY engine. `query_history.user_name` now records the principal's own
///   uuid (previously the fixed placeholder `"anonymous"` — see
///   `QueryHistoryItem::user`'s doc comment), and `engine: "trino"`'s
///   `X-Trino-User` is derived from that same uuid, never
///   `principal.display_name` (a display name is attacker-controlled, and
///   the compose `trino` service's file-based access control grants full
///   `iceberg` catalog write access to exactly the user name
///   `trino-maintenance` — see that service's `rules.json` block comment).
///   Neither has a safe/honest value to fall back to with no principal, so
///   the request is refused before anything runs. The route is
///   policy-gated (`Policy::RequiresPermission("query:read")`), so this is
///   defense in depth, the same shape `routes::gold::export` uses.
/// - 422 [`ApiError::Unprocessable`] when `sql` fails [`is_read_only`] (any
///   engine) or [`is_trino_safe`] (`engine: "trino"` only), or when the
///   engine itself rejects the query.
/// - 503 [`ApiError::Unavailable`] when `Trino` is unreachable or times out.
pub async fn run(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed = parse_body(&body)?;
    let sql = match parsed.sql {
        Some(s) if !s.is_empty() => s,
        _ => return Err(ApiError::BadRequest("sql is required".to_owned()).into()),
    };
    if !is_read_only(&sql) {
        return Err(ApiError::Unprocessable(
            "Only read queries (SELECT/SHOW/DESCRIBE/EXPLAIN) are allowed in Query Studio."
                .to_owned(),
        )
        .into());
    }

    // See this function's doc comment: there is no honest value left to
    // record (or, for Trino, to send as `X-Trino-User`) for an
    // unauthenticated caller, so every engine requires a principal now —
    // `"anonymous"` is never written to `query_history` again.
    let Some(Extension(p)) = principal else {
        return Err(ApiError::unauthorized().into());
    };
    let user_id = p.id.uuid().to_string();

    let engine = parsed.engine.as_deref().unwrap_or("clickhouse").to_owned();
    if engine == "trino" && !is_trino_safe(&sql) {
        return Err(ApiError::Unprocessable(
            "This statement is not permitted against Trino (write, session, or procedure \
             keyword detected)."
                .to_owned(),
        )
        .into());
    }

    let started = Instant::now();
    let started_epoch_ms = epoch_ms();

    let (columns, rows, duration_ms, scanned_bytes) = if engine == "trino" {
        let trino_user = format!("console-{user_id}");
        let result = state
            .trino
            .run_statement(&sql, &trino_user)
            .await
            .map_err(map_trino_error)?;
        let (columns, rows) = trino_result_to_rows(&result);
        (columns, rows, elapsed_ms(started), 0)
    } else {
        let result = state.clickhouse.query(&sql, None).await?;

        let columns: Vec<String> = result.meta.iter().map(|m| m.name.clone()).collect();
        let rows: Vec<Value> = result
            .data
            .iter()
            .map(|row| {
                let mut out = Map::new();
                for c in &columns {
                    let v = row.get(c);
                    out.insert(c.clone(), Value::String(stringify_cell(v)));
                }
                Value::Object(out)
            })
            .collect();

        let scanned_bytes = result.statistics.as_ref().map_or(0, |s| s.bytes_read);
        let duration_ms = result
            .statistics
            .as_ref()
            .map_or_else(|| elapsed_ms(started), |s| seconds_to_ms(s.elapsed));
        (columns, rows, duration_ms, scanned_bytes)
    };

    let cost_units = std::cmp::max(1, bytes_to_cost_units(scanned_bytes));
    let id = format!("q-{started_epoch_ms}");

    // Record this execution in query history — best-effort. A history-write
    // failure (no Postgres pool configured, Postgres down, ...) must never
    // turn an otherwise-successful query into an error response for the
    // caller, who already has their result: log a warning and keep going.
    // See `lakehouse_store::queries::record_history`'s doc comment.
    //
    // `user` is the principal's own uuid (guaranteed present above) and
    // `engine` is the engine that actually ran — both genuine measurements
    // now, not the pre-WS2-§4 `"anonymous"`/`"hot-store"` placeholders.
    if let Some(pool) = state.pg.as_deref() {
        #[allow(
            clippy::cast_possible_wrap,
            clippy::cast_precision_loss,
            reason = "scanned_bytes/duration_ms/cost_units are ClickHouse/Trino-reported sizes \
                      for one query, well within i64/f64's exact-integer range"
        )]
        let input = RecordHistoryInput {
            id: &id,
            sql: &sql,
            user: &user_id,
            status: "completed",
            duration_ms: duration_ms as i64,
            scanned_bytes: scanned_bytes as i64,
            cost_units: cost_units as f64,
            workload_class: "hot-analytics",
            engine: &engine,
            cache_assisted: false,
        };
        if let Err(err) = queries::record_history(pool, &input).await {
            tracing::warn!(%err, "failed to record query history (query itself succeeded)");
        }
    }

    Ok(ApiJson(run_result_json(
        &id,
        &json!(columns),
        &json!(rows),
        duration_ms,
        scanned_bytes,
        cost_units,
        &engine,
    )))
}

/// Coerce one cell to the string-everything shape both `run_result_json`
/// row shapes use, matching the `TypeScript`'s implicit stringification.
fn stringify_cell(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(other) => other.to_string(),
    }
}

/// Convert a [`TrinoResult`] into the same `(columns, rows)` shape the
/// `ClickHouse` branch of [`run`] produces, so both engines answer with an
/// identical `QueryResult` row format.
fn trino_result_to_rows(result: &TrinoResult) -> (Vec<String>, Vec<Value>) {
    let columns: Vec<String> = result.columns.iter().map(|c| c.name.clone()).collect();
    let rows: Vec<Value> = result
        .rows
        .iter()
        .map(|row| {
            let mut out = Map::new();
            for (i, c) in columns.iter().enumerate() {
                out.insert(c.clone(), Value::String(stringify_cell(row.get(i))));
            }
            Value::Object(out)
        })
        .collect();
    (columns, rows)
}

/// Map a [`TrinoError`] to the [`ApiError`] `run` returns.
///
/// [`TrinoError::Query`]'s message is forwarded verbatim — the ONE place
/// this crate lets `Trino`'s own text reach a response, because the caller
/// reading it is the same principal whose own `SQL` produced it (the exact
/// posture `lakehouse_trino`'s crate doc comment documents as safe).
/// Every other variant is classified into a fixed message before it
/// reaches the caller, per this repository's "upstream error text never
/// reaches a response" rule.
fn map_trino_error(err: TrinoError) -> ApiError {
    match err {
        TrinoError::Query(msg) => ApiError::Unprocessable(msg),
        TrinoError::Timeout => ApiError::Unavailable("trino query timed out".to_owned()),
        TrinoError::Transport(_) | TrinoError::HttpStatus(_) => {
            ApiError::Unavailable("trino unavailable".to_owned())
        }
        TrinoError::TooManyRows { cap } => {
            ApiError::Unprocessable(format!("trino result exceeded the {cap}-row cap"))
        }
    }
}

/// Build the `/api/query/run` response body from already-computed values.
///
/// Extracted from [`run`] so the transparency-panel contract — which fields
/// are real measurements versus `null` — is unit-testable without a live
/// `ClickHouse` connection.
///
/// `engine` is the top-level field naming which engine actually ran this
/// statement (`"clickhouse"`/`"trino"`, WS2 §4) — a DIFFERENT thing from
/// `metrics.engine`, which stays the fixed string `"hot-store"` describing
/// `ClickHouse`'s storage tier regardless of which top-level engine ran.
/// Neither renames nor removes the other.
fn run_result_json(
    id: &str,
    columns: &Value,
    rows: &Value,
    duration_ms: u64,
    scanned_bytes: u64,
    cost_units: u64,
    engine: &str,
) -> Value {
    json!({
        "id": id,
        "engine": engine,
        "columns": columns,
        "rows": rows,
        "metrics": {
            "durationMs": duration_ms,
            "scannedBytes": scanned_bytes,
            "costUnits": cost_units,
            "engine": "hot-store",
            "workloadClass": "hot-analytics",
            // WS1 task 1.6: `ClickHouse`'s response carries no cache-hit flag
            // and no pushdown list, and there is no policy engine to produce
            // obligations (`WS7` builds one). Null says "not measured"; the
            // old constants (`false`, `[]`, `[]`) claimed a measurement that
            // was never taken.
            "cacheHit": Value::Null,
            "pushdowns": Value::Null,
            "policyObligations": Value::Null,
        },
        // WS1 task 1.6: the old `plan` was a single hardcoded stage claiming
        // "scan + aggregate" and status "completed" for every query,
        // regardless of what `ClickHouse` actually did. Null until a real
        // plan is computed.
        "plan": Value::Null,
    })
}

/// Current time as Unix milliseconds, matching JavaScript's `Date.now()`
/// used to build the `q-<ms>` id.
#[allow(
    clippy::cast_possible_truncation,
    reason = "milliseconds since epoch fits comfortably in i64 until year 292 million"
)]
fn epoch_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Wall-clock duration since `started`, in milliseconds — the fallback
/// `Date.now() - started` timing path when `ClickHouse` reports no
/// `statistics.elapsed`.
#[allow(
    clippy::cast_possible_truncation,
    reason = "a single request's wall-clock duration in ms cannot exceed u64 range in practice"
)]
fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

/// `Math.round(elapsedSeconds * 1000)`.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "ClickHouse-reported elapsed seconds for one query is always a small, \
              non-negative value in practice"
)]
fn seconds_to_ms(elapsed_seconds: f64) -> u64 {
    (elapsed_seconds * 1000.0).round() as u64
}

/// `Math.round(scannedBytes / 1_000_000)` — "~1 unit / MB read".
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "byte counts here are well within f64's exact-integer range \
              (2^53), and the ratio is always small and non-negative"
)]
fn bytes_to_cost_units(scanned_bytes: u64) -> u64 {
    (scanned_bytes as f64 / 1_000_000.0).round() as u64
}

/// `Math.round(estimatedBytes / divisor)`, used for both the min (÷2M) and
/// max (÷1M) cost buckets in `query/estimate`.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "estimated byte counts here are well within f64's exact-integer \
              range (2^53), and the ratio is always small and non-negative"
)]
fn bytes_to_cost_bucket(estimated_bytes: i64, divisor: f64) -> u64 {
    (estimated_bytes as f64 / divisor).round() as u64
}

/// `POST /api/query/estimate` — a rough cost/plan estimate via `EXPLAIN
/// ESTIMATE`, never erroring back to the caller.
///
/// # Errors
///
/// Returns 400 [`ApiError::BadRequest`] on an unparseable body or a
/// missing/blank `sql`. `EXPLAIN ESTIMATE` failures (e.g. non-`SELECT`
/// input) are swallowed, matching the `TypeScript`'s inner `catch {}` —
/// the response is always 200 in that case, with a zeroed estimate.
pub async fn estimate(State(state): State<AppState>, body: Bytes) -> ApiResult<ApiJson<Value>> {
    let parsed = parse_body(&body)?;
    let sql = match parsed.sql {
        Some(s) if !s.trim().is_empty() => s,
        _ => return Err(ApiError::BadRequest("sql is required".to_owned()).into()),
    };

    let (estimated_bytes, sources) = estimate_body(&state.clickhouse, &sql).await;

    let cost_min = std::cmp::max(1, bytes_to_cost_bucket(estimated_bytes, 2_000_000.0));
    let cost_max = std::cmp::max(1, bytes_to_cost_bucket(estimated_bytes, 1_000_000.0));
    let sources_out: Vec<String> = if sources.is_empty() {
        vec!["clickhouse@lakehouse".to_owned()]
    } else {
        sources
    };

    Ok(ApiJson(estimate_result_json(
        estimated_bytes,
        cost_min,
        cost_max,
        &json!(sources_out),
    )))
}

/// Build the `/api/query/estimate` response body from already-computed
/// values. See [`run_result_json`] for why this is extracted.
fn estimate_result_json(
    estimated_bytes: i64,
    cost_min: u64,
    cost_max: u64,
    sources_out: &Value,
) -> Value {
    json!({
        "estimatedBytes": estimated_bytes,
        "estimatedCostMin": cost_min,
        "estimatedCostMax": cost_max,
        "workloadClass": "hot-analytics",
        "engine": "hot-store",
        // WS1 task 1.6: no cache-eligibility check runs and no
        // freshness-lag measurement exists, and there is no policy engine
        // yet (`WS7` builds one). The old `plan` marked stages that had
        // *not run* as status "completed" with `estimatedBytes: 0` — a
        // fabricated measurement of work that was never done. Null says
        // "not measured".
        "cacheEligible": Value::Null,
        "freshnessLagSeconds": Value::Null,
        "policyObligations": Value::Null,
        "sources": sources_out,
        "plan": Value::Null,
    })
}

/// Runs `EXPLAIN ESTIMATE <sql>` (with any trailing `;` stripped) and
/// tallies `estimatedBytes`/`sources` from the result rows. Returns
/// `(0, [])` on any `ClickHouse` failure, matching the `TypeScript`'s inner
/// `catch {}`.
async fn estimate_body(ch: &ChClient, sql: &str) -> (i64, Vec<String>) {
    let trimmed = strip_trailing_semicolon(sql);
    let query = format!("EXPLAIN ESTIMATE {trimmed}");
    let Ok(result) = ch.query(&query, None).await else {
        return (0, Vec::new());
    };
    let mut estimated_bytes: i64 = 0;
    let mut sources = Vec::new();
    for row in &result.data {
        let db = row.get("database").and_then(Value::as_str).unwrap_or("");
        let tbl = row.get("table").and_then(Value::as_str).unwrap_or("");
        if !tbl.is_empty() {
            sources.push(if db.is_empty() {
                tbl.to_owned()
            } else {
                format!("{db}.{tbl}")
            });
        }
        let rows_n = row.get("rows").and_then(numeric_value).unwrap_or(0.0);
        estimated_bytes += rows_to_bytes(rows_n);
    }
    (estimated_bytes, sources)
}

/// `rows * 64` — the rough byte-per-row estimate `EXPLAIN ESTIMATE` rows
/// are converted to.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "row counts here are well within f64's exact-integer range and \
              always non-negative"
)]
fn rows_to_bytes(rows: f64) -> i64 {
    (rows * 64.0) as i64
}

/// Coerce a `serde_json::Value` cell to `f64`, matching JavaScript's
/// `Number(row["rows"] ?? 0)`.
fn numeric_value(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

/// `sql.replace(/;\s*$/, "")` — strip one trailing `;` (and any trailing
/// whitespace after it), not every trailing `;`.
fn strip_trailing_semicolon(sql: &str) -> &str {
    let trimmed_end = sql.trim_end();
    trimmed_end.strip_suffix(';').unwrap_or(trimmed_end)
}

// ── Postgres-backed writes (Task 2.4) ───────────────────────────────────
//
// `listSaved`/`listHistory` back the methods `src/services/clients/queries.ts`
// used to delegate to `mockQueryService`. No TypeScript server-side
// precedent exists for either — like `routes::identity`/
// `routes::governance`'s Postgres-backed additions, status codes are chosen
// to be correct rather than faithful.

/// Borrow the Postgres pool, or fail with a 503 explaining why there isn't
/// one. Mirrors `routes::identity::pool`.
fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "query store unavailable: no Postgres pool is configured \
             (DATABASE_URL is missing or not a valid Postgres connection string)"
                .to_owned(),
        )
    })
}

/// `GET /api/query/saved` — every saved query.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_saved(State(state): State<AppState>) -> ApiResult<ApiJson<Vec<SavedQuery>>> {
    Ok(ApiJson(queries::list_saved(pool(&state)?).await?))
}

/// `GET /api/query/history` — recent, real query executions, recorded by
/// [`run`] on success.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_history(
    State(state): State<AppState>,
) -> ApiResult<ApiJson<Vec<QueryHistoryItem>>> {
    Ok(ApiJson(queries::list_history(pool(&state)?).await?))
}

// ── GET /api/query/run/{id}/download (WS2 §13, WS2 plan review W9) ─────

/// `?format=csv|parquet` on [`download`].
#[derive(Debug, Deserialize)]
pub(crate) struct DownloadQuery {
    format: String,
}

/// The most rows a download re-run is allowed to return — this is a
/// bounded RE-EXECUTION against current data, not a replay of the
/// original result, so the cap protects both the response size and the
/// re-run's own cost. Applied twice, belt and braces: as a `LIMIT` in the
/// wrapped statement itself (see [`build_capped_statement`]) and as a
/// `max_result_rows` `SETTINGS` override with `result_overflow_mode =
/// 'break'`, so `ClickHouse` truncates rather than erroring if the outer
/// `LIMIT` is somehow not honored.
const DOWNLOAD_ROW_CAP: u32 = 10_000;

/// `query_history.id` shape: `q-<epoch_ms>`, digits only after the
/// prefix. Validated before `id` is ever interpolated into the
/// `Content-Disposition` header (WS2 plan review W9) — this is a
/// header-injection guard, not a lookup optimization; the database lookup
/// itself already binds `id` as a parameter and needs no such check.
fn is_valid_history_id(id: &str) -> bool {
    id.strip_prefix("q-")
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

/// Only these `query_history.engine` values may be re-run through this
/// route (WS2 plan review W9). `"hot-store"` is the legacy placeholder
/// every row recorded before Task C2 threaded a real `engine` field
/// carries — it named `ClickHouse`'s storage tier, not a different
/// engine, so it is downloadable exactly like `"clickhouse"`. `"trino"`
/// is deliberately excluded: re-running it on the wrong engine is not an
/// option this route offers.
fn is_downloadable_engine(engine: &str) -> bool {
    engine == "clickhouse" || engine == "hot-store"
}

/// Wrap `sql` (a `query_history` row's stored statement) in an outer
/// `SELECT * FROM (...)  LIMIT <cap> ...` rather than appending `LIMIT
/// <cap>` to it directly.
///
/// Appending would be invalid `SQL` whenever the stored statement already
/// ends in its own `LIMIT` clause, and worse: a trailing `-- comment` in
/// the stored `SQL` would comment out an appended `LIMIT`, uncapping the
/// download entirely. Wrapping keeps the stored statement — comment and
/// all — fully enclosed in its own subquery, so nothing in it can reach
/// outside the parentheses to interfere with the outer `LIMIT`/`SETTINGS`
/// clause (WS2 §13, WS2 plan review W9).
///
/// Only a single trailing `;` (and any whitespace around it) is stripped
/// first — the same `strip_trailing_semicolon` shape
/// `lakehouse-clickhouse` already uses for its own `FORMAT` appending, so
/// a genuinely empty statement or one with internal semicolons is left
/// otherwise untouched.
fn build_capped_statement(sql: &str, format_clause: &str) -> String {
    let trimmed = sql.trim_end();
    let trimmed = trimmed.strip_suffix(';').unwrap_or(trimmed).trim_end();
    format!(
        "SELECT * FROM (\n{trimmed}\n) LIMIT {DOWNLOAD_ROW_CAP} SETTINGS \
         max_result_rows = {DOWNLOAD_ROW_CAP}, result_overflow_mode = 'break' \
         FORMAT {format_clause}"
    )
}

/// Map a [`ChError`] from the re-run `ClickHouse` call performed by
/// [`download`] to a fixed [`ApiError`] — never `?`, and never the
/// upstream message.
///
/// `POST /api/query/run`'s `ChError -> ApiError` conversion
/// (`lakehouse_clickhouse::ChError`'s own `From` impl, tested by
/// `error.rs`'s `ch_error_converts_through_question_mark_to_422_rejection`)
/// forwards `ClickHouse`'s message verbatim — a narrow, deliberate
/// exception because the caller reading it is the same principal whose
/// own `SQL` produced it, at the moment they submitted it. This route
/// re-runs a PAST query, possibly minutes or days later, so the caller is
/// no longer necessarily in a position to make sense of a raw `ClickHouse`
/// diagnostic; more importantly, per this repository's "upstream error
/// text never reaches a response" rule, this route does not reuse that
/// narrow precedent (WS2 plan review W9). `ChError::Server` becomes a
/// fixed 422; every other variant is treated as the service being
/// unavailable and becomes 503. The real detail is logged server-side
/// either way.
fn map_download_ch_error(err: &ChError) -> ApiError {
    match err {
        ChError::Server(_) => {
            tracing::warn!(%err, "stored query failed to re-run for download");
            ApiError::Unprocessable("stored query failed to re-run".to_owned())
        }
        ChError::Transport(_) | ChError::Cancelled => {
            tracing::warn!(%err, "clickhouse unreachable while re-running a query for download");
            ApiError::Unavailable("clickhouse unavailable while re-running the query".to_owned())
        }
    }
}

/// `GET /api/query/run/{id}/download?format=csv|parquet` — re-run a past
/// `query_history` row's `SQL` against current data and return the result
/// as a file download (WS2 §13, WS2 plan review W9).
///
/// This is NOT the original result the caller once saw: it re-executes
/// the stored statement against whatever `ClickHouse` holds right now,
/// capped at [`DOWNLOAD_ROW_CAP`] rows. Data ingested or deleted since the
/// original run changes what comes back.
///
/// Only rows whose `engine` is `"clickhouse"` or the legacy `"hot-store"`
/// placeholder are eligible (WS2 plan review W9) — a `"trino"` row cannot
/// be re-run through this route at all.
///
/// Ownership is exact-match on `query_history.user_name` against the
/// caller's own principal id string. Every row recorded before Task C2
/// threaded a real principal into `routes::query::run` carries the fixed
/// placeholder `"anonymous"`, which — by construction — never equals any
/// authenticated caller's real id string, so those legacy rows are
/// permanently undownloadable through this route. That is a deliberate,
/// fail-closed consequence of scoping by identity, not an oversight to be
/// worked around.
///
/// # Errors
///
/// - 400 [`ApiError::BadRequest`] when `format` is neither `csv` nor
///   `parquet`, or when `id` does not match the `query_history.id` shape
///   (`q-<digits>`).
/// - 404 [`ApiError::NotFound`] when no history row matches `id`, or when
///   one does but its recorded owner is not the calling principal
///   (including every legacy `"anonymous"` row — see this doc comment).
/// - 422 [`ApiError::Unprocessable`] when the row's `engine` is not
///   downloadable, when its stored `SQL` fails [`is_read_only`] (defense
///   in depth: `routes::query::run` already gated this at insert time),
///   or when `ClickHouse` rejects the re-run.
/// - 503 [`ApiError::Unavailable`] when the query store or `ClickHouse`
///   itself is unreachable.
pub async fn download(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<DownloadQuery>,
    principal: Option<Extension<Principal>>,
) -> ApiResult<Response> {
    let (format_clause, content_type, extension) = match query.format.as_str() {
        "csv" => ("CSV", "text/csv", "csv"),
        "parquet" => ("Parquet", "application/octet-stream", "parquet"),
        _ => {
            return Err(ApiError::BadRequest("format must be csv or parquet".to_owned()).into());
        }
    };
    if !is_valid_history_id(&id) {
        return Err(ApiError::BadRequest("id must look like a query run id".to_owned()).into());
    }

    let Some(item) = queries::get_history_item(pool(&state)?, &id).await? else {
        return Err(ApiError::NotFound("query run not found".to_owned()).into());
    };

    // Fails closed identically for a stranger's row and a legacy
    // "anonymous" row (see this function's doc comment) — a caller cannot
    // distinguish "not yours" from "does not exist" either way.
    let caller_id = principal
        .as_ref()
        .map(|Extension(p)| p.id.uuid().to_string());
    if caller_id.as_deref() != Some(item.user.as_str()) {
        return Err(ApiError::NotFound("query run not found".to_owned()).into());
    }

    if !is_downloadable_engine(&item.engine) {
        return Err(
            ApiError::Unprocessable("download supports ClickHouse runs only".to_owned()).into(),
        );
    }
    // Defense in depth: `routes::query::run` already refused a non-read-only
    // statement at insert time, so this should be unreachable in practice —
    // re-checked here because this route is about to re-execute the stored
    // SQL, not merely read it back.
    if !is_read_only(&item.sql) {
        return Err(ApiError::Unprocessable("stored query is not read-only".to_owned()).into());
    }

    let capped_sql = build_capped_statement(&item.sql, format_clause);
    let bytes = state
        .clickhouse
        .raw_bytes(&capped_sql, None)
        .await
        .map_err(|err| map_download_ch_error(&err))?;

    let content_disposition = format!("attachment; filename=\"{id}.{extension}\"");
    // Bypasses the `ApiJson` choke point deliberately: the response body
    // here is a raw file (CSV/Parquet bytes), not a JSON value — there is
    // nothing for `ApiJson` to serialize (AGENTS.md requires a comment at
    // any choke-point bypass explaining why).
    Ok((
        [
            (header::CONTENT_TYPE, content_type.to_owned()),
            (header::CONTENT_DISPOSITION, content_disposition),
        ],
        bytes,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn allows_each_permitted_keyword() {
        for sql in [
            "SELECT 1",
            "with x as (select 1) select * from x",
            "SHOW TABLES",
            "DESCRIBE t",
            "DESC t",
            "EXPLAIN SELECT 1",
        ] {
            assert!(is_read_only(sql), "expected allowed: {sql}");
        }
    }

    #[test]
    fn denies_each_denied_keyword_even_after_select() {
        for kw in [
            "insert", "alter", "drop", "delete", "update", "create", "truncate", "rename",
            "attach", "detach", "grant", "revoke",
        ] {
            let sql = format!("SELECT 1; {kw} something");
            assert!(!is_read_only(&sql), "expected denied: {sql}");
        }
    }

    #[test]
    fn denies_bare_dml() {
        assert!(!is_read_only("INSERT INTO x VALUES (1)"));
        assert!(!is_read_only("DROP TABLE x"));
    }

    #[test]
    fn allows_leading_whitespace() {
        assert!(is_read_only("   \n\t SELECT 1"));
    }

    #[test]
    fn rejects_leading_comment_before_select() {
        // TS quirk: the leading-keyword regex anchors to the very start of
        // the string, so a comment before SELECT fails it even though the
        // statement is otherwise pure read.
        assert!(!is_read_only("/* c */ SELECT 1"));
        assert!(!is_read_only("-- c\nSELECT 1"));
    }

    #[test]
    fn is_case_insensitive() {
        assert!(is_read_only("SeLeCt 1"));
        assert!(!is_read_only("select 1; DeLeTe from t"));
    }

    #[test]
    fn smuggled_dml_after_select_via_semicolon_is_rejected() {
        assert!(!is_read_only("SELECT 1 AS a; DELETE FROM t"));
    }

    #[test]
    fn word_boundary_avoids_false_positive_substring_match() {
        // "createdAt" contains "create" but not as a whole word.
        assert!(is_read_only("SELECT createdAt FROM t"));
        // "updated_at" contains "update" but not as a whole word (the
        // trailing `_` is a \w character, so no boundary there).
        assert!(is_read_only("SELECT updated_at FROM t"));
    }

    #[test]
    fn rejects_statement_starting_with_disallowed_keyword() {
        assert!(!is_read_only("TRUNCATE t"));
        assert!(!is_read_only("GRANT SELECT ON t TO u"));
    }

    #[test]
    fn strip_trailing_semicolon_removes_one_trailing_semicolon_and_whitespace() {
        assert_eq!(strip_trailing_semicolon("SELECT 1;  "), "SELECT 1");
        assert_eq!(strip_trailing_semicolon("SELECT 1"), "SELECT 1");
    }

    #[test]
    fn numeric_value_coerces_string_and_number() {
        assert_eq!(numeric_value(&json!(5)), Some(5.0));
        assert_eq!(numeric_value(&json!("7")), Some(7.0));
        assert_eq!(numeric_value(&json!(null)), None);
    }

    #[test]
    fn run_result_emits_null_for_unmeasured_transparency_fields() {
        let v = run_result_json("q-1", &json!([]), &json!([]), 12, 34, 1, "clickhouse");

        // The transparency panel exists to tell the reader what actually
        // happened. ClickHouse's response carries no cache-hit flag and no
        // pushdown list, and no policy engine exists yet (WS7), so these are
        // not measurements.
        for key in ["cacheHit", "pushdowns", "policyObligations"] {
            assert!(
                v["metrics"][key].is_null(),
                "{key} must be null, got {}",
                v["metrics"][key]
            );
        }
        assert!(
            v["plan"].is_null(),
            "the plan was one invented stage, not a real plan"
        );

        // Genuinely measured values survive.
        assert_eq!(v["metrics"]["durationMs"], json!(12));
        assert_eq!(v["metrics"]["scannedBytes"], json!(34));
        assert_eq!(v["metrics"]["costUnits"], json!(1));

        // The new top-level `engine` (which engine ran) and the untouched
        // `metrics.engine` (ClickHouse's storage tier) are two different
        // concepts and must not collapse into one value.
        assert_eq!(v["engine"], json!("clickhouse"));
        assert_eq!(v["metrics"]["engine"], json!("hot-store"));
    }

    #[test]
    fn estimate_result_emits_null_for_unmeasured_fields() {
        let v = estimate_result_json(100, 1, 2, &json!([]));

        for key in [
            "cacheEligible",
            "freshnessLagSeconds",
            "policyObligations",
            "plan",
        ] {
            assert!(v[key].is_null(), "{key} must be null, got {}", v[key]);
        }
        // The EXPLAIN ESTIMATE numbers are real.
        assert_eq!(v["estimatedBytes"], json!(100));
    }

    // ── `engine: "trino"` (WS2 §4) ──────────────────────────────────────

    mod trino_engine {
        use std::collections::HashMap;

        use lakehouse_auth::{PermissionSet, PrincipalId};
        use uuid::Uuid;
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::*;
        use crate::config::Config;

        /// Builds `AppState` the way `routes::connectors::state_without_pool`
        /// does (`AppState::new(Config::from_map(..))`), overriding
        /// `TRINO_URL` so `AppState::trino` points at the wiremock server
        /// instead of the real compose service. No live Postgres either
        /// (`DATABASE_URL` is deliberately unparseable): history recording
        /// degrades to a logged warning, per `record_history`'s doc
        /// comment, which is fine for these handler-level tests.
        fn test_state_with_trino(trino_uri: &str) -> AppState {
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
            env.insert("TRINO_URL".to_owned(), trino_uri.to_owned());
            AppState::new(Config::from_map(&env).expect("valid config from a plain map"))
        }

        /// An authenticated `query:read` principal for `run`'s tests.
        /// `display_name` is deliberately a parameter — one test sets it to
        /// `"trino-maintenance"` to prove [`run`] never derives
        /// `X-Trino-User` from it.
        fn principal_with_display_name(display_name: &str) -> Extension<Principal> {
            Extension(Principal {
                id: PrincipalId::User(Uuid::nil()),
                tenant_ids: Vec::new(),
                display_name: display_name.to_owned(),
                permissions: PermissionSet::parse("query:read"),
                provider: "session".to_owned(),
                must_change_password: false,
            })
        }

        fn trino_body(sql: &str) -> Bytes {
            Bytes::from(serde_json::to_vec(&json!({"sql": sql, "engine": "trino"})).unwrap())
        }

        async fn assert_refused_before_reaching_trino(sql: &str) {
            let server = MockServer::start().await;
            let state = test_state_with_trino(&server.uri());
            let result = run(
                State(state),
                Some(principal_with_display_name("alice")),
                trino_body(sql),
            )
            .await;
            assert!(result.is_err(), "expected {sql:?} to be refused");
            assert_eq!(
                server
                    .received_requests()
                    .await
                    .expect("mock server records requests")
                    .len(),
                0,
                "{sql:?} must never reach Trino"
            );
        }

        // Refusals `is_read_only` already catches, unchanged — the Trino
        // guard is additive, never a relaxation of the existing gate.
        #[tokio::test]
        async fn refuses_call_without_reaching_trino() {
            assert_refused_before_reaching_trino("CALL system.runtime.kill_query(query_id => 'x')")
                .await;
        }

        #[tokio::test]
        async fn refuses_alter_table_execute_optimize() {
            assert_refused_before_reaching_trino("ALTER TABLE t EXECUTE optimize").await;
        }

        #[tokio::test]
        async fn refuses_merge_into() {
            assert_refused_before_reaching_trino(
                "MERGE INTO t USING s ON t.id = s.id WHEN MATCHED THEN DELETE",
            )
            .await;
        }

        #[tokio::test]
        async fn refuses_set_session() {
            assert_refused_before_reaching_trino("SET SESSION query_max_run_time = '1h'").await;
        }

        #[tokio::test]
        async fn refuses_explain_analyze_insert() {
            assert_refused_before_reaching_trino("EXPLAIN ANALYZE INSERT INTO t SELECT 1").await;
        }

        // Refusals the new Trino-specific guard adds — each of these passes
        // `is_read_only` today (verified against this branch's unmodified
        // `is_read_only`/`contains_denied_keyword` before `is_trino_safe`
        // existed): none of the 12 ClickHouse-shaped denied words appear,
        // and each starts with the allowed `explain` keyword.
        #[tokio::test]
        async fn refuses_explain_analyze_merge_bypass() {
            assert_refused_before_reaching_trino(
                "EXPLAIN ANALYZE MERGE INTO t USING s ON t.id = s.id WHEN MATCHED THEN DELETE",
            )
            .await;
        }

        #[tokio::test]
        async fn refuses_explain_analyze_refresh_materialized_view_bypass() {
            assert_refused_before_reaching_trino("EXPLAIN ANALYZE REFRESH MATERIALIZED VIEW v")
                .await;
        }

        #[tokio::test]
        async fn refuses_explain_analyze_comment_bypass() {
            assert_refused_before_reaching_trino("EXPLAIN ANALYZE COMMENT ON TABLE t IS 'x'").await;
        }

        #[tokio::test]
        async fn refuses_explain_analyze_with_extra_whitespace_and_mixed_case() {
            assert_refused_before_reaching_trino("ExPlAiN     aNaLyZe SELECT 1").await;
        }

        #[tokio::test]
        async fn runs_a_select_and_reports_engine_in_the_response() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/statement"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "id": "q1",
                    "columns": [{"name": "n", "type": "bigint"}],
                    "data": [[1]],
                })))
                .mount(&server)
                .await;

            let state = test_state_with_trino(&server.uri());
            let ApiJson(value) = run(
                State(state),
                Some(principal_with_display_name("alice")),
                trino_body("SELECT 1"),
            )
            .await
            .expect("ok");

            assert_eq!(value["engine"], json!("trino"));
            assert_eq!(value["rows"], json!([{"n": "1"}]));
        }

        #[tokio::test]
        async fn x_trino_user_header_is_derived_from_the_uuid_never_the_display_name() {
            let server = MockServer::start().await;
            let expected_header = format!("console-{}", Uuid::nil());
            Mock::given(method("POST"))
                .and(path("/v1/statement"))
                .and(header("X-Trino-User", expected_header.as_str()))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "id": "q1",
                    "columns": [{"name": "n", "type": "bigint"}],
                    "data": [[1]],
                })))
                .mount(&server)
                .await;

            let state = test_state_with_trino(&server.uri());
            // The display name is deliberately the exact string the
            // maintenance user is named — proving `X-Trino-User` is never
            // derived from it is the point of this test.
            let result = run(
                State(state),
                Some(principal_with_display_name("trino-maintenance")),
                trino_body("SELECT 1"),
            )
            .await;

            assert!(
                result.is_ok(),
                "the wiremock header matcher would reject a wrong header"
            );
        }

        #[tokio::test]
        async fn missing_principal_is_refused_before_reaching_trino() {
            let server = MockServer::start().await;
            let state = test_state_with_trino(&server.uri());
            let result = run(State(state), None, trino_body("SELECT 1")).await;
            let err = result.expect_err("no principal must be refused");
            assert_eq!(err.0.status(), 401);
            assert_eq!(
                server
                    .received_requests()
                    .await
                    .expect("mock server records requests")
                    .len(),
                0
            );
        }

        #[test]
        fn maps_query_error_to_422_forwarding_the_message_verbatim() {
            let err = map_trino_error(TrinoError::Query("line 1:1: mismatched input".to_owned()));
            assert_eq!(err.status(), 422);
            assert!(err.to_string().contains("mismatched input"));
        }

        #[test]
        fn maps_timeout_to_503() {
            let err = map_trino_error(TrinoError::Timeout);
            assert_eq!(err.status(), 503);
        }

        #[test]
        fn maps_http_status_to_503() {
            let err = map_trino_error(TrinoError::HttpStatus(500));
            assert_eq!(err.status(), 503);
        }

        #[test]
        fn maps_too_many_rows_to_422_naming_the_cap() {
            let err = map_trino_error(TrinoError::TooManyRows { cap: 10_000 });
            assert_eq!(err.status(), 422);
            assert!(err.to_string().contains("10000") || err.to_string().contains("10,000"));
        }

        #[test]
        fn allows_a_plain_select_against_the_trino_guard() {
            assert!(is_trino_safe("SELECT 1"));
        }

        // ── WS2 §4: the real principal now gates every engine ──────────

        /// The default (`ClickHouse`) engine is refused with 401 too, not
        /// just `engine: "trino"` — `run`'s doc comment documents there is
        /// no honest `"anonymous"` fallback left for `query_history.user`.
        /// This must fail before either engine's client is ever called, so
        /// no `ClickHouse`/`Trino` reachability is needed for this test.
        #[tokio::test]
        async fn missing_principal_is_401_for_the_default_engine_too() {
            let server = MockServer::start().await;
            let state = test_state_with_trino(&server.uri());
            let body = Bytes::from(r#"{"sql": "SELECT 1"}"#);
            let err = run(State(state), None, body)
                .await
                .expect_err("no principal must be refused");
            assert_eq!(err.0.status(), 401);
        }

        /// A run with an authenticated principal records that principal's
        /// own uuid string as `query_history.user` — proven at the
        /// `lakehouse_store` layer
        /// (`record_history_carries_the_real_principal_uuid_and_engine` in
        /// `lakehouse-store/tests/queries.rs`), since this crate's handler
        /// tests run with no live Postgres pool (`record_history` degrades
        /// to a logged warning per its own doc comment, so there is
        /// nothing to assert against here). This test instead pins the
        /// pure derivation `run` performs — `p.id.uuid().to_string()` — so
        /// a future refactor of that one line is caught here even without
        /// a database.
        #[test]
        fn the_recorded_user_is_the_principals_uuid_string_not_the_display_name() {
            let Extension(p) = principal_with_display_name("trino-maintenance");
            let recorded_user = p.id.uuid().to_string();
            assert_eq!(recorded_user, Uuid::nil().to_string());
            assert_ne!(recorded_user, p.display_name);
        }
    }

    // ── GET /api/query/run/{id}/download (WS2 §13, WS2 plan review W9) ──

    mod download_pure {
        use super::*;

        #[test]
        fn wraps_sql_that_already_has_its_own_limit_clause() {
            let capped = build_capped_statement("SELECT * FROM t LIMIT 5", "CSV");
            assert_eq!(
                capped,
                "SELECT * FROM (\nSELECT * FROM t LIMIT 5\n) LIMIT 10000 SETTINGS \
                 max_result_rows = 10000, result_overflow_mode = 'break' FORMAT CSV"
            );
        }

        #[test]
        fn wraps_sql_ending_in_a_trailing_comment_without_letting_it_uncap_the_limit() {
            // A naive `format!("{sql} LIMIT {cap}")` would have this trailing
            // `--` comment out the appended LIMIT, uncapping the download —
            // wrapping the whole statement in its own subquery keeps the
            // comment fully enclosed, unable to reach the outer LIMIT.
            let capped = build_capped_statement("SELECT * FROM t -- trailing comment", "CSV");
            assert_eq!(
                capped,
                "SELECT * FROM (\nSELECT * FROM t -- trailing comment\n) LIMIT 10000 \
                 SETTINGS max_result_rows = 10000, result_overflow_mode = 'break' FORMAT CSV"
            );
        }

        #[test]
        fn wraps_a_with_query() {
            let capped = build_capped_statement("WITH x AS (SELECT 1) SELECT * FROM x", "Parquet");
            assert_eq!(
                capped,
                "SELECT * FROM (\nWITH x AS (SELECT 1) SELECT * FROM x\n) LIMIT 10000 \
                 SETTINGS max_result_rows = 10000, result_overflow_mode = 'break' \
                 FORMAT Parquet"
            );
        }

        #[test]
        fn strips_exactly_one_trailing_semicolon_and_its_surrounding_whitespace() {
            let capped = build_capped_statement("SELECT 1;   ", "CSV");
            assert_eq!(
                capped,
                "SELECT * FROM (\nSELECT 1\n) LIMIT 10000 SETTINGS max_result_rows = 10000, \
                 result_overflow_mode = 'break' FORMAT CSV"
            );
        }

        #[test]
        fn clickhouse_and_legacy_hot_store_engines_are_downloadable() {
            assert!(is_downloadable_engine("clickhouse"));
            assert!(is_downloadable_engine("hot-store"));
        }

        #[test]
        fn trino_and_unknown_engines_are_not_downloadable() {
            assert!(!is_downloadable_engine("trino"));
            assert!(!is_downloadable_engine("something-else"));
        }

        #[test]
        fn valid_history_ids_are_the_q_dash_digits_shape() {
            assert!(is_valid_history_id("q-1"));
            assert!(is_valid_history_id("q-1736200000000"));
        }

        #[test]
        fn ids_without_the_q_dash_digits_shape_are_rejected() {
            for id in [
                "",
                "q-",
                "q-abc",
                "Q-1",
                "q-1;DROP TABLE t",
                "../q-1",
                "q-1\r\nSet-Cookie: x=y",
            ] {
                assert!(!is_valid_history_id(id), "expected rejected: {id:?}");
            }
        }

        #[test]
        fn ch_server_error_maps_to_422_with_a_fixed_message_never_the_upstream_text() {
            let err = map_download_ch_error(&ChError::Server(
                "Code: 47. Unknown identifier: nope".to_owned(),
            ));
            assert_eq!(err.status(), 422);
            assert_eq!(err.to_string(), "stored query failed to re-run");
            assert!(!err.to_string().contains("Unknown identifier"));
        }

        #[test]
        fn ch_transport_error_maps_to_503() {
            let err = map_download_ch_error(&ChError::Cancelled);
            assert_eq!(err.status(), 503);
        }
    }
}
