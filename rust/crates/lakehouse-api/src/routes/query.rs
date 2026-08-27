//! `POST /api/query/run`, `POST /api/query/estimate` — the ad hoc `SQL`
//! Query Studio.
//!
//! Ports `src/app/api/query/run/route.ts` and
//! `src/app/api/query/estimate/route.ts`.

use std::time::Instant;

use axum::body::Bytes;
use axum::extract::State;
use lakehouse_clickhouse::ChClient;
use lakehouse_core::ApiError;
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
}

/// Parse the raw request body as `{"sql": "..."}`.
///
/// Both routes share one `try { ({ sql } = await req.json()) } catch { ...
/// "Body harus JSON {sql}" ... }` shape in the `TypeScript`: any body that
/// doesn't parse as JSON at all — not merely a body missing `sql` — is a
/// 400 with this exact message.
fn parse_body(body: &Bytes) -> Result<SqlBody, ApiError> {
    serde_json::from_slice(body)
        .map_err(|_err| ApiError::BadRequest("Body harus JSON {sql}".to_owned()))
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

/// `POST /api/query/run` — execute a read-only `SQL` statement against
/// `ClickHouse` and return it in `QueryResult` shape.
///
/// # Errors
///
/// - 400 [`ApiError::BadRequest`] on an unparseable body or a missing/empty
///   `sql`.
/// - 422 [`ApiError::Unprocessable`] when `sql` fails the read-only guard,
///   or when `ClickHouse` itself rejects the query.
pub async fn run(State(state): State<AppState>, body: Bytes) -> ApiResult<ApiJson<Value>> {
    let parsed = parse_body(&body)?;
    let sql = match parsed.sql {
        Some(s) if !s.is_empty() => s,
        _ => return Err(ApiError::BadRequest("sql wajib diisi".to_owned()).into()),
    };
    if !is_read_only(&sql) {
        return Err(ApiError::Unprocessable(
            "Hanya query baca (SELECT/SHOW/DESCRIBE/EXPLAIN) yang diizinkan di Query Studio."
                .to_owned(),
        )
        .into());
    }

    let started = Instant::now();
    let started_epoch_ms = epoch_ms();
    let result = state.clickhouse.query(&sql, None).await?;

    let columns: Vec<String> = result.meta.iter().map(|m| m.name.clone()).collect();
    let rows: Vec<Value> = result
        .data
        .iter()
        .map(|row| {
            let mut out = Map::new();
            for c in &columns {
                let v = row.get(c);
                let s = match v {
                    None | Some(Value::Null) => String::new(),
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Bool(b)) => b.to_string(),
                    Some(Value::Number(n)) => n.to_string(),
                    Some(other) => other.to_string(),
                };
                out.insert(c.clone(), Value::String(s));
            }
            Value::Object(out)
        })
        .collect();

    let scanned_bytes = result.statistics.as_ref().map_or(0, |s| s.bytes_read);
    let duration_ms = result
        .statistics
        .as_ref()
        .map_or_else(|| elapsed_ms(started), |s| seconds_to_ms(s.elapsed));
    let cost_units = std::cmp::max(1, bytes_to_cost_units(scanned_bytes));

    Ok(ApiJson(json!({
        "id": format!("q-{started_epoch_ms}"),
        "columns": columns,
        "rows": rows,
        "metrics": {
            "durationMs": duration_ms,
            "scannedBytes": scanned_bytes,
            "costUnits": cost_units,
            "engine": "hot-store",
            "workloadClass": "hot-analytics",
            "cacheHit": false,
            "pushdowns": [],
            "policyObligations": [],
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
    })))
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
        _ => return Err(ApiError::BadRequest("sql wajib diisi".to_owned()).into()),
    };

    let (estimated_bytes, sources) = estimate_body(&state.clickhouse, &sql).await;

    let cost_min = std::cmp::max(1, bytes_to_cost_bucket(estimated_bytes, 2_000_000.0));
    let cost_max = std::cmp::max(1, bytes_to_cost_bucket(estimated_bytes, 1_000_000.0));
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
                "estimatedBytes": 0,
                "status": "completed",
            })
        })
        .collect();

    Ok(ApiJson(json!({
        "estimatedBytes": estimated_bytes,
        "estimatedCostMin": cost_min,
        "estimatedCostMax": cost_max,
        "workloadClass": "hot-analytics",
        "engine": "hot-store",
        "cacheEligible": true,
        "freshnessLagSeconds": 0,
        "policyObligations": [],
        "sources": sources_out,
        "plan": plan,
    })))
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
}
