//! `POST /api/query/run`, `POST /api/query/estimate` — the ad hoc `SQL`
//! Query Studio.
//!
//! Ports `src/app/api/query/run/route.ts` and
//! `src/app/api/query/estimate/route.ts`.

use std::time::Instant;

use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use lakehouse_auth::{Principal, PrincipalId};
use lakehouse_clickhouse::ChClient;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::audit::{self, NewAuditEvent};
use lakehouse_store::queries::{
    self, CollaborationProject, CreateCollaborationProjectInput, QueryHistoryItem,
    RecordHistoryInput, SavedQuery,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;

/// The `{sql}` request body both routes accept.
#[derive(Debug, Deserialize)]
struct SqlBody {
    #[serde(default)]
    sql: Option<String>,
}

/// Parse the raw request body as `{"sql": "..."}`.
///
/// Any body that doesn't parse as JSON at all — not merely a body missing
/// `sql` — is a 400. The message is English like the rest of the console:
/// these strings are shown to the user verbatim.
fn parse_body(body: &Bytes) -> Result<SqlBody, ApiError> {
    serde_json::from_slice(body)
        .map_err(|_err| ApiError::BadRequest("body must be JSON {sql}".to_owned()))
}

/// Whether `sql` is a read-only statement `ClickHouse` may run from Query
/// Studio.
///
/// Both halves of the check run against [`strip_sql_noise`]'s output
/// rather than the raw text, which is the difference from the
/// `query/run/route.ts` guard this was ported from. That guard tested the
/// raw string, and so rejected two things it had no reason to:
///
/// - Anything written under a leading comment — including the editor's own
///   starter line, `-- Write SQL here…`, which made the very first query a
///   new user typed fail with "only read queries are allowed".
/// - Any query merely *mentioning* a DML word, e.g.
///   `SELECT * FROM t WHERE action = 'drop'`.
///
/// What it still rejects is what matters: a statement that does not start
/// with a read keyword, and smuggled DML such as `SELECT 1; DELETE FROM t`
/// — the semicolon trick is why the denied-word test looks at the whole
/// statement rather than just its first word.
#[must_use]
fn is_read_only(sql: &str) -> bool {
    let code = strip_sql_noise(sql);
    starts_with_allowed_keyword(&code) && !contains_denied_keyword(&code)
}

/// `sql` with comments and quoted literals blanked out, so the guard reads
/// only the parts of a statement that can actually do something.
///
/// Removed: `-- line` and `# line` comments, `/* block */` comments, and
/// `'single'`, `"double"` and `` `backtick` `` quoted runs (doubled quotes
/// and backslash escapes inside them included). Each is replaced by a
/// single space rather than deleted, so words either side of it cannot be
/// glued into one.
#[must_use]
fn strip_sql_noise(sql: &str) -> String {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            '-' if next == Some('-') => {
                i = skip_to_line_end(&chars, i);
                out.push(' ');
            }
            '#' => {
                i = skip_to_line_end(&chars, i);
                out.push(' ');
            }
            '/' if next == Some('*') => {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    i += 1;
                }
                i = (i + 2).min(chars.len());
                out.push(' ');
            }
            '\'' | '"' | '`' => {
                i = skip_quoted(&chars, i, c);
                out.push(' ');
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Index just past the end of the line starting at `from`.
fn skip_to_line_end(chars: &[char], from: usize) -> usize {
    let mut i = from;
    while i < chars.len() && chars[i] != '\n' {
        i += 1;
    }
    i
}

/// Index just past the quoted run that opens at `from` with `quote`.
/// An unterminated quote consumes the rest of the statement, which is the
/// safe reading: the guard sees less, not more.
fn skip_quoted(chars: &[char], from: usize, quote: char) -> usize {
    let mut i = from + 1;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == quote {
            // A doubled quote is an escaped quote, not the end of the run.
            if chars.get(i + 1) == Some(&quote) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    chars.len()
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

/// The most rows one run hands back to the browser.
///
/// The console renders every row it receives, so an unbounded `SELECT *`
/// used to be enough to freeze the tab. `ClickHouse` still computes the
/// whole result — this caps what crosses the wire, and the response says
/// when it did (`truncated`), so nobody reads a partial answer as the
/// whole one.
const MAX_RESULT_ROWS: usize = 2_000;

/// `POST /api/query/run` — execute a read-only `SQL` statement against
/// `ClickHouse` and return it in `QueryResult` shape.
///
/// Every run is written to the audit trail and to the caller's query
/// history — failures included, which is the point: a history that only
/// remembers what worked is not a record of what happened.
///
/// # Errors
///
/// - 400 [`ApiError::BadRequest`] on an unparseable body or a missing/empty
///   `sql`.
/// - 422 [`ApiError::Unprocessable`] when `sql` fails the read-only guard,
///   or when `ClickHouse` itself rejects the query.
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
    let actor = Actor::from(principal.as_ref());
    if !is_read_only(&sql) {
        let detail = "only read queries (SELECT/SHOW/DESCRIBE/EXPLAIN) are allowed in Query \
                      Studio";
        record_run(&state, &actor, &sql, RunOutcome::blocked(detail)).await;
        return Err(ApiError::Unprocessable(detail.to_owned()).into());
    }

    let started = Instant::now();
    let result = match state.clickhouse.query(&sql, None).await {
        Ok(result) => result,
        Err(err) => {
            let message = err.to_string();
            record_run(
                &state,
                &actor,
                &sql,
                RunOutcome::failed(&message, elapsed_ms(started)),
            )
            .await;
            return Err(ApiError::from(err).into());
        }
    };

    let columns: Vec<String> = result.meta.iter().map(|m| m.name.clone()).collect();
    let total_rows = result.data.len();
    let truncated = total_rows > MAX_RESULT_ROWS;
    let rows: Vec<Value> = result
        .data
        .iter()
        .take(MAX_RESULT_ROWS)
        .map(|row| {
            let mut out = Map::new();
            for c in &columns {
                // Values keep the type ClickHouse gave them. They used to
                // be stringified here, which made every numeric column
                // sort as text in the console ("10" before "9") and lose
                // its right alignment.
                out.insert(c.clone(), row.get(c).cloned().unwrap_or(Value::Null));
            }
            Value::Object(out)
        })
        .collect();

    let scanned_bytes = result.statistics.as_ref().map_or(0, |s| s.bytes_read);
    let duration_ms = result
        .statistics
        .as_ref()
        .map_or_else(|| elapsed_ms(started), |s| seconds_to_ms(s.elapsed));
    let cost_units = bytes_to_cost_units(scanned_bytes);

    let recorded = record_run(
        &state,
        &actor,
        &sql,
        RunOutcome::completed(duration_ms, scanned_bytes, cost_units),
    )
    .await;

    Ok(ApiJson(json!({
        "id": recorded.id,
        "columns": columns,
        "rows": rows,
        "rowCount": total_rows,
        "truncated": truncated,
        "rowLimit": MAX_RESULT_ROWS,
        "metrics": {
            "durationMs": duration_ms,
            "scannedBytes": scanned_bytes,
            "costUnits": cost_units,
            "engine": "hot-store",
            "workloadClass": "hot-analytics",
            // Nothing here reads ClickHouse's query cache, so claiming a
            // miss would be a guess dressed as a measurement.
            "cacheHit": Value::Null,
            // Same reasoning as the estimate's: no optimizer report is
            // read and no policy engine runs, so an empty list would be a
            // claim ("checked, none apply") rather than a measurement.
            "pushdowns": Value::Null,
            "policyObligations": Value::Null,
        },
        "plan": [
            {
                "id": "s1",
                "label": "ClickHouse (Hot analytical store)",
                "location": "clickhouse@lakehouse",
                "operation": "scan + aggregate",
                "estimatedBytes": scanned_bytes,
                "status": "completed",
            }
        ],
        "auditEventId": recorded.audit_event_id,
    })))
}

/// Who ran a query, in the forms the trail needs: a display name, the id
/// the history row belongs to, and which kind of principal it was.
struct Actor {
    label: String,
    id: Option<Uuid>,
    kind: Option<&'static str>,
}

impl Actor {
    fn from(principal: Option<&Extension<Principal>>) -> Self {
        principal.map_or_else(
            || Self {
                label: "anonymous".to_owned(),
                id: None,
                kind: None,
            },
            |Extension(p)| Self {
                label: p.display_name.clone(),
                id: Some(p.id.uuid()),
                kind: Some(match p.id {
                    PrincipalId::User(_) => "user",
                    PrincipalId::Service(_) => "service",
                }),
            },
        )
    }
}

/// How a run ended, with the numbers that belong in history.
struct RunOutcome<'a> {
    status: &'a str,
    outcome: &'a str,
    detail: Option<String>,
    duration_ms: u64,
    scanned_bytes: u64,
    cost_units: f64,
}

impl<'a> RunOutcome<'a> {
    fn completed(duration_ms: u64, scanned_bytes: u64, cost_units: f64) -> Self {
        Self {
            status: "completed",
            outcome: "executed",
            detail: None,
            duration_ms,
            scanned_bytes,
            cost_units,
        }
    }

    fn failed(message: &str, duration_ms: u64) -> Self {
        Self {
            status: "failed",
            outcome: "failed",
            detail: Some(message.to_owned()),
            duration_ms,
            scanned_bytes: 0,
            cost_units: 0.0,
        }
    }

    fn blocked(reason: &'a str) -> Self {
        Self {
            status: "blocked",
            outcome: "refused",
            detail: Some(reason.to_owned()),
            duration_ms: 0,
            scanned_bytes: 0,
            cost_units: 0.0,
        }
    }
}

/// What [`record_run`] wrote, so the response can point at it.
struct RecordedRun {
    id: String,
    audit_event_id: Option<String>,
}

/// Write one run to the audit trail and to the runner's history.
///
/// Best-effort on purpose: a caller who already has their result must not
/// be handed an error because the bookkeeping failed. Both failures are
/// logged instead. The audit event is written first so history can carry
/// its real id — the id used to be a string built from the timestamp,
/// which pointed the console's "View in Audit" button at an event that had
/// never been written.
async fn record_run(
    state: &AppState,
    actor: &Actor,
    sql: &str,
    outcome: RunOutcome<'_>,
) -> RecordedRun {
    let id = format!("q-{}", epoch_ms());
    let Some(pool) = state.pg.as_deref() else {
        return RecordedRun {
            id,
            audit_event_id: None,
        };
    };

    let event = NewAuditEvent {
        principal_id: actor.id.map(|id| id.to_string()),
        principal_kind: actor.kind.map(ToOwned::to_owned),
        actor_label: Some(actor.label.clone()),
        action: "query.run".to_owned(),
        resource_kind: Some("query".to_owned()),
        resource_id: Some(id.clone()),
        // The SQL, never the rows it returned: `audit::insert` writes args
        // verbatim and result data has no business in the trail.
        args: Some(json!({ "sql": sql })),
        outcome: outcome.outcome.to_owned(),
        detail: outcome.detail.clone(),
        run_id: None,
        approval_id: None,
        session_id: None,
    };
    let audit_event_id = match audit::insert(pool, event).await {
        Ok(written) => Some(written.id),
        Err(err) => {
            tracing::warn!(%err, "failed to write query audit event");
            None
        }
    };

    #[allow(
        clippy::cast_possible_wrap,
        reason = "scanned_bytes/duration_ms are ClickHouse-reported sizes for one query, \
                  well within i64's range"
    )]
    let input = RecordHistoryInput {
        id: &id,
        sql,
        user: &actor.label,
        owner_id: actor.id,
        status: outcome.status,
        duration_ms: outcome.duration_ms as i64,
        scanned_bytes: outcome.scanned_bytes as i64,
        cost_units: outcome.cost_units,
        workload_class: "hot-analytics",
        engine: "hot-store",
        cache_assisted: false,
        audit_event_id: audit_event_id.as_deref(),
    };
    if let Err(err) = queries::record_history(pool, &input).await {
        tracing::warn!(%err, "failed to record query history (query itself succeeded)");
    }

    RecordedRun { id, audit_event_id }
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

/// Cost units for a scan: ~1 unit per MB read, kept to four decimals.
///
/// This used to round to a whole unit, which made every query under half
/// a megabyte cost exactly "0" — the console then showed "0.0000 cu" for
/// a query that had genuinely scanned 33 KB.
#[allow(
    clippy::cast_precision_loss,
    reason = "byte counts here are well within f64's exact-integer range (2^53)"
)]
fn bytes_to_cost_units(scanned_bytes: u64) -> f64 {
    round4(scanned_bytes as f64 / 1_000_000.0)
}

/// Four decimal places, which is the resolution the console prints.
fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

/// The min (÷2M) and max (÷1M) cost buckets in `query/estimate`, to the
/// same four decimals as [`bytes_to_cost_units`].
#[allow(
    clippy::cast_precision_loss,
    reason = "estimated byte counts here are well within f64's exact-integer range (2^53)"
)]
fn bytes_to_cost_bucket(estimated_bytes: i64, divisor: f64) -> f64 {
    round4(estimated_bytes as f64 / divisor)
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

    // A failed EXPLAIN is reported, not swallowed. It used to come back as
    // a zeroed estimate, so "this query cannot even be planned" and "this
    // query reads nothing" looked identical — and ClickHouse's message is
    // usually the most useful thing on the page (an unknown column, a
    // missing table) before anyone presses Run.
    let estimated = estimate_body(&state.clickhouse, &sql).await;
    let (estimated_bytes, sources, estimate_error) = match estimated {
        Ok((bytes, sources)) => (Some(bytes), sources, None),
        Err(message) => (None, Vec::new(), Some(message)),
    };
    let freshness_lag_seconds = freshness_lag(&state.clickhouse, &sources).await;

    // No floor: an estimate of nothing costs nothing. `max(1, …)` used to
    // make an empty editor read "1.00 cu–1.00 cu".
    let cost_min = estimated_bytes.map(|b| bytes_to_cost_bucket(b, 2_000_000.0));
    let cost_max = estimated_bytes.map(|b| bytes_to_cost_bucket(b, 1_000_000.0));
    let sources_out: Vec<String> = if sources.is_empty() {
        vec!["clickhouse@lakehouse".to_owned()]
    } else {
        sources.clone()
    };
    let plan: Vec<Value> = sources
        .iter()
        .enumerate()
        .map(|(i, s)| {
            json!({
                "id": format!("p{i}"),
                "label": s,
                "location": "clickhouse@lakehouse",
                "operation": "scan",
                // EXPLAIN ESTIMATE reports rows per table but no per-table
                // byte count, so this stage has no size to give. It used
                // to print a literal 0, which the panel rendered as
                // "~0 B" — a measurement that was never taken.
                "estimatedBytes": Value::Null,
                "status": "completed",
            })
        })
        .collect();

    Ok(ApiJson(json!({
        "estimatedBytes": estimated_bytes,
        "estimatedCostMin": cost_min,
        "estimatedCostMax": cost_max,
        "error": estimate_error,
        // True by construction: Query Studio runs everything through
        // ClickHouse, the hot analytical store.
        "workloadClass": "hot-analytics",
        "engine": "hot-store",
        // Nothing in this deployment configures ClickHouse's query cache,
        // and no policy engine is consulted before a run. Both used to be
        // answered anyway — "Eligible", "None" — with the same confidence
        // as the measured numbers above.
        "cacheEligible": Value::Null,
        "freshnessLagSeconds": freshness_lag_seconds,
        "policyObligations": Value::Null,
        "sources": sources_out,
        "plan": plan,
    })))
}

/// Seconds since the newest write to any table the query reads, or `None`
/// when that cannot be established — no source tables identified, or
/// `ClickHouse` did not answer.
///
/// The lag reported is the *worst* one across the sources: a join is only
/// as fresh as its stalest side. This used to be hardcoded to 0, so the
/// panel said "Fresh · 0 s" for every query, including one that had not
/// been written yet.
async fn freshness_lag(ch: &ChClient, sources: &[String]) -> Option<i64> {
    let qualified: Vec<&String> = sources.iter().filter(|s| s.contains('.')).collect();
    if qualified.is_empty() {
        return None;
    }
    let predicate = qualified
        .iter()
        .filter_map(|s| s.split_once('.'))
        .map(|(db, table)| {
            format!(
                "(database = '{}' AND table = '{}')",
                escape_literal(db),
                escape_literal(table)
            )
        })
        .collect::<Vec<_>>()
        .join(" OR ");
    if predicate.is_empty() {
        return None;
    }
    let sql = format!(
        "SELECT toString(dateDiff('second', max(modification_time), now())) lag \
         FROM system.parts WHERE active AND ({predicate})"
    );
    let rows = ch.rows(&sql, None).await.ok()?;
    rows.first()?
        .get("lag")
        .and_then(Value::as_str)
        .and_then(|v| v.parse::<i64>().ok())
}

/// Single-quote escaping for a `ClickHouse` string literal. The names come
/// from `EXPLAIN ESTIMATE`'s own output rather than from the caller, but
/// they are still interpolated into SQL, so they are escaped.
fn escape_literal(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Runs `EXPLAIN ESTIMATE <sql>` (with any trailing `;` stripped) and
/// tallies `estimatedBytes`/`sources` from the result rows, or returns
/// what `ClickHouse` said about why it could not plan the statement.
async fn estimate_body(ch: &ChClient, sql: &str) -> Result<(i64, Vec<String>), String> {
    let trimmed = strip_trailing_semicolon(sql);
    let query = format!("EXPLAIN ESTIMATE {trimmed}");
    let result = ch.query(&query, None).await.map_err(|err| {
        // Just the first line: ClickHouse follows its message with a stack
        // of internal context nobody reading the console needs.
        let message = err.to_string();
        message
            .lines()
            .next()
            .unwrap_or("could not plan this query")
            .chars()
            .take(240)
            .collect::<String>()
    })?;
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
    Ok((estimated_bytes, sources))
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
// `listSaved`/`listHistory`/`listCollaboration`/`createCollaborationProject`
// back the methods `src/services/clients/queries.ts` used to delegate to
// `mockQueryService`. No TypeScript server-side precedent exists for any of
// them — like `routes::identity`/`routes::governance`'s Postgres-backed
// additions, status codes are chosen to be correct rather than faithful.

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

fn parse_json_body<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|err| ApiError::BadRequest(format!("invalid JSON: {err}")))
}

/// `GET /api/query/saved` — every saved query.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_saved(State(state): State<AppState>) -> ApiResult<ApiJson<Vec<SavedQuery>>> {
    Ok(ApiJson(queries::list_saved(pool(&state)?).await?))
}

/// `GET /api/query/history` — the caller's own recent query executions,
/// recorded by [`run`].
///
/// # Errors
///
/// 401 when nobody is signed in — history belongs to a person, so there is
/// no sensible answer for "everyone's". 503 if no pool is configured; 500
/// on a database failure.
pub async fn list_history(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
) -> ApiResult<ApiJson<Vec<QueryHistoryItem>>> {
    let owner = caller_id(principal.as_ref())?;
    Ok(ApiJson(queries::list_history(pool(&state)?, owner).await?))
}

/// The signed-in caller's id, or a 401.
fn caller_id(principal: Option<&Extension<Principal>>) -> Result<Uuid, ApiError> {
    principal
        .map(|Extension(p)| p.id.uuid())
        .ok_or_else(|| ApiError::Unauthorized("sign in required".to_owned()))
}

/// The `POST /api/query/saved` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSavedQueryBody {
    title: String,
    sql: String,
    #[serde(default)]
    tags: Vec<String>,
}

/// `POST /api/query/saved` — save the SQL in the editor under a name.
///
/// Saved queries are shared: the whole team sees them, which is what makes
/// them worth saving. The author is recorded all the same.
///
/// # Errors
///
/// 400 on a malformed body or a blank title/SQL; 401 when nobody is signed
/// in; 422 when the SQL is not read-only — a saved query is run through the
/// same guard as anything else, so refusing it here is better than storing
/// something that can never run; 503/500 as above.
pub async fn create_saved(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<SavedQuery>)> {
    let body: CreateSavedQueryBody = parse_json_body(&body)?;
    let title = body.title.trim().to_owned();
    let sql = body.sql.trim().to_owned();
    if title.is_empty() || sql.is_empty() {
        return Err(ApiError::BadRequest("title and sql are required".to_owned()).into());
    }
    if !is_read_only(&sql) {
        return Err(ApiError::Unprocessable(
            "only read queries (SELECT/SHOW/DESCRIBE/EXPLAIN) can be saved".to_owned(),
        )
        .into());
    }
    let owner_id = caller_id(principal.as_ref())?;
    let owner = principal.map_or_else(
        || "anonymous".to_owned(),
        |Extension(p)| p.display_name.clone(),
    );
    let saved = queries::create_saved_query(
        pool(&state)?,
        &title,
        &sql,
        &owner,
        &body.tags,
        Some(owner_id),
    )
    .await?;
    Ok((StatusCode::CREATED, ApiJson(saved)))
}

/// `GET /api/query/collaboration` — every collaboration project.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_collaboration(
    State(state): State<AppState>,
) -> ApiResult<ApiJson<Vec<CollaborationProject>>> {
    Ok(ApiJson(queries::list_collaboration(pool(&state)?).await?))
}

/// The `POST /api/query/collaboration` body. Mirrors
/// `CreateCollaborationProjectInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCollaborationProjectBody {
    name: String,
    #[serde(default)]
    collaborators: Vec<String>,
    #[serde(default)]
    description: Option<String>,
}

/// `POST /api/query/collaboration` — create a collaboration project.
/// Returns 201.
///
/// # Errors
///
/// 400 on a malformed body; 503/500 as above.
pub async fn create_collaboration_project(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<CollaborationProject>)> {
    let body: CreateCollaborationProjectBody = parse_json_body(&body)?;
    let input = CreateCollaborationProjectInput {
        name: body.name,
        collaborators: body.collaborators,
        description: body.description,
    };
    let created = queries::create_collaboration_project(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(created)))
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
    fn allows_a_comment_before_the_statement() {
        // The editor opens with exactly this shape — a starter comment,
        // then the user's query underneath — and it used to be refused.
        assert!(is_read_only("-- Write SQL here\nSELECT 1"));
        assert!(is_read_only("/* c */ SELECT 1"));
        assert!(is_read_only("# hash comment\nSELECT 1"));
    }

    #[test]
    fn a_dml_word_inside_a_comment_or_string_is_not_dml() {
        assert!(is_read_only("SELECT 1 -- drop this later"));
        assert!(is_read_only("/* TODO: delete */ SELECT 1"));
        assert!(is_read_only("SELECT * FROM t WHERE action = 'drop'"));
        assert!(is_read_only("SELECT 'it''s an update' AS note"));
        // …but real DML beside a decoy string still is.
        assert!(!is_read_only("SELECT 'drop' AS a; DROP TABLE t"));
    }

    #[test]
    fn strip_sql_noise_blanks_comments_and_literals() {
        // Exact spacing is irrelevant to the guard — what matters is that
        // the comment is gone and the words around it are not glued
        // together.
        let stripped = strip_sql_noise("SELECT 1 -- drop\nFROM t");
        assert!(!stripped.contains("drop"), "{stripped}");
        assert!(stripped.contains("SELECT 1"), "{stripped}");
        assert!(stripped.contains("FROM t"), "{stripped}");
        assert_eq!(strip_sql_noise("SELECT /* x */ 1"), "SELECT   1");
        assert_eq!(
            strip_sql_noise("SELECT 'a' , \"b\" , `c`"),
            "SELECT   ,   ,  "
        );
        // An unterminated quote swallows the rest, which is the safe way
        // round: the guard then sees less, never more.
        assert_eq!(strip_sql_noise("SELECT 'unterminated"), "SELECT  ");
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
}
