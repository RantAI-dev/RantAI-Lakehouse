//! `POST /api/query/run`, `POST /api/query/estimate` — the ad hoc `SQL`
//! Query Studio.
//!
//! Ports `src/app/api/query/run/route.ts` and
//! `src/app/api/query/estimate/route.ts`.

use std::time::Instant;

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use lakehouse_auth::Principal;
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::audit::NewAuditEvent;
use lakehouse_store::queries::{self, QueryHistoryItem, RecordHistoryInput, SavedQuery};
use lakehouse_trino::{TrinoError, TrinoResult};
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
/// 400 with this exact message (byte-for-byte parity corpus match, not
/// merely English-language style).
fn parse_body(body: &Bytes) -> Result<SqlBody, ApiError> {
    serde_json::from_slice(body)
        .map_err(|_err| ApiError::BadRequest("Body must be JSON {sql}".to_owned()))
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

/// WS7 item C2: prefetches obligations for every table `sql` references
/// (WS7 item C1's `PolicyEngineObligations`) and rewrites `sql` through
/// [`crate::sql_rewrite::enforce`] — masking/row-filtering every governed
/// table, refusing a table function, a sensitive `system.*` read, a
/// `dictGet`/`joinGet`-family call, or a view over a governed table
/// outright. `sql` is never forwarded to `ClickHouse`/`Trino` unrewritten:
/// every failure here is an `Err`, matching `sql_rewrite`'s own "never a
/// silent pass-through" rule (Hard Requirement 2), and Hard Requirement 1
/// (a `status = 'ready'` policy row with an authored-but-unparseable
/// condition) surfaces the same way — a 422, never a silently-unenforced
/// query — via [`crate::policy_engine::PolicyEngineError::UnenforceableAuthoredCondition`].
///
/// The elapsed wall time is recorded into `state.policy_decision_latencies`
/// (WS7 item C2 Step 4's `policyDecisionP95Ms`) regardless of outcome — a
/// refused query still cost real time deciding that.
///
/// # Errors
/// 422 [`ApiError::Unprocessable`] with a fixed, non-leaking message
/// (`policy_engine::engine_error_message`/`refusal_message` — the
/// underlying Postgres/`sqlparser` error text never reaches the response,
/// `AGENTS.md` rule 4) when obligations cannot be resolved or `sql` fails
/// classification/substitution.
async fn rewrite_sql_for_principal(
    state: &AppState,
    sql: &str,
    engine: &str,
    principal: &Principal,
) -> Result<String, ApiError> {
    let started_rewrite = Instant::now();
    let result = rewrite_sql_for_principal_inner(state, sql, engine, principal).await;
    state
        .policy_decision_latencies
        .record(started_rewrite.elapsed());
    result
}

/// `Trino`'s catalog.schema.table qualification needs `GenericDialect` (no
/// `PrestoDialect` exists in this `sqlparser` version — verified, see
/// `sql_rewrite`'s own `generic_dialect_parses_a_representative_trino_shape`
/// test); every other engine value (including the `"clickhouse"` default)
/// uses `ClickHouseDialect`, matching `run`'s own engine dispatch.
fn is_trino_engine(engine: &str) -> bool {
    engine == "trino"
}

async fn rewrite_sql_for_principal_inner(
    state: &AppState,
    sql: &str,
    engine: &str,
    principal: &Principal,
) -> Result<String, ApiError> {
    let placeholders = crate::sql_rewrite::PlaceholderValues {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_tenant_ids: principal
            .tenant_ids
            .iter()
            .map(ToString::to_string)
            .collect(),
    };
    let obligations_source =
        crate::policy_engine::PolicyEngineObligations::new(state.pg.as_deref(), &state.clickhouse);

    // WS7 item D1/D2: the actual referenced-tables/prefetch/enforce
    // sequence now lives in ONE place —
    // `policy_engine::rewrite_sql_for_roles` — shared with
    // `routes::support::run_spec_sql` (dashboards/embeds) and
    // `gold_export::export_batch_sql` (Gold export), never a parallel
    // copy of this logic. Only the per-engine dialect *selection*
    // (`ClickHouseDialect` vs. `GenericDialect`) stays here, since the
    // two are different concrete types.
    if is_trino_engine(engine) {
        crate::policy_engine::rewrite_sql_for_roles(
            sql,
            &sqlparser::dialect::GenericDialect {},
            &principal.role_names,
            &placeholders,
            &obligations_source,
        )
        .await
    } else {
        crate::policy_engine::rewrite_sql_for_roles(
            sql,
            &sqlparser::dialect::ClickHouseDialect {},
            &principal.role_names,
            &placeholders,
            &obligations_source,
        )
        .await
    }
    .map_err(|err| {
        ApiError::Unprocessable(crate::policy_engine::enforcement_error_message(&err).to_owned())
    })
}

/// Runs the already-rewritten `sql` against whichever engine `engine`
/// names, returning `(columns, rows, duration_ms, scanned_bytes)`. Split
/// out of [`run`] purely to keep that function under
/// `clippy::too_many_lines` after WS7 item C2 added the rewrite step —
/// no behavior change from the inline block it replaces.
async fn execute_query_against_engine(
    state: &AppState,
    engine: &str,
    sql: &str,
    user_id: &str,
    started: Instant,
) -> Result<(Vec<String>, Vec<Value>, u64, u64), ApiError> {
    if engine == "trino" {
        let trino_user = format!("console-{user_id}");
        let result = state
            .trino
            .run_statement(sql, &trino_user)
            .await
            .map_err(map_trino_error)?;
        let (columns, rows) = trino_result_to_rows(&result);
        Ok((columns, rows, elapsed_ms(started), 0))
    } else {
        let result = state.clickhouse.query(sql, None).await?;

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
        Ok((columns, rows, duration_ms, scanned_bytes))
    }
}

/// The most rows one run hands back to the browser.
///
/// The console renders every row it receives, so an unbounded `SELECT *`
/// used to be enough to freeze the tab. `ClickHouse`/`Trino` still compute
/// the whole result — this caps what crosses the wire, and the response
/// says when it did (`truncated`), so nobody reads a partial answer as the
/// whole one.
const MAX_RESULT_ROWS: usize = 2_000;

/// `POST /api/query/run` — execute a read-only `SQL` statement against
/// `ClickHouse` (`engine` absent or `"clickhouse"`) or `Trino`
/// (`engine: "trino"`, WS2 §4) and return it in `QueryResult` shape.
///
/// Every successful run is written to the audit trail and to the caller's
/// query history (best-effort — see `queries::record_history`'s doc
/// comment). A refused or failed run is answered with an error and is NOT
/// separately recorded here; the audit trail's own refusal path is what
/// WS7 built for enforcement decisions, not this route's history write.
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
///   engine itself rejects the query, or `sql_rewrite::enforce` refuses it
///   (WS7 item C2 — a table function, a sensitive `system.*` read, a
///   `dictGet`/`joinGet`-family call, a view over a governed table, or an
///   authored-but-unenforceable policy condition, WS7 plan Hard
///   Requirement 1).
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

    // WS7 item C2: every ad hoc SQL surface is rewritten through the same
    // `sql_rewrite::enforce` (Phase B) before it reaches EITHER engine —
    // still after `is_read_only`, per this route's existing ordering, and
    // before `started`/`Instant::now()` below so the elapsed time recorded
    // for `policyDecisionP95Ms` covers exactly the rewrite step, nothing
    // else.
    let sql = rewrite_sql_for_principal(&state, &sql, &engine, &p).await?;

    let started = Instant::now();
    let started_epoch_ms = epoch_ms();

    let (columns, mut rows, duration_ms, scanned_bytes) =
        execute_query_against_engine(&state, &engine, &sql, &user_id, started).await?;

    // The console renders every row it receives, so an unbounded `SELECT *`
    // used to be enough to freeze the tab — see `MAX_RESULT_ROWS`'s own doc
    // comment. `ClickHouse`/`Trino` still compute the whole result; only
    // what crosses the wire is capped, and the response says when it did.
    let total_rows = rows.len();
    let truncated = total_rows > MAX_RESULT_ROWS;
    rows.truncate(MAX_RESULT_ROWS);

    let cost_units = std::cmp::max(1, bytes_to_cost_units(scanned_bytes));
    let id = format!("q-{started_epoch_ms}");

    // Record a real audit_event for this run — best-effort, the same
    // non-fatal posture as the history write just below: the query already
    // ran and the caller already has their result, so a failure here is
    // logged and swallowed, never turned into an error response. Written
    // via `lakehouse_store::audit::insert` directly (not
    // `routes::ai::audit::record`, which hardcodes `principal_kind:
    // "copilot"` — wrong here, a query run is human- or service-triggered,
    // never copilot-triggered). `resource_kind: "query_history"` paired
    // with this run's own `id` is the pairing WS1 T16's `LEFT JOIN`
    // resolves on read (WS5 Phase D preamble).
    if let Some(pool) = state.pg.as_deref() {
        let event = query_run_audit_event(&p, &id, &sql);
        if let Err(err) = lakehouse_store::audit::insert(pool, event).await {
            tracing::warn!(%err, "failed to record query.run audit event (query itself succeeded)");
        }
    }

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
            owner_id: Some(p.id.uuid()),
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
        total_rows,
        truncated,
        duration_ms,
        scanned_bytes,
        cost_units,
        &engine,
    )))
}

/// Built once per query run, unit-tested without a `ClickHouse`/Postgres
/// connection — the pure half of the audit write, matching this file's
/// existing pattern for pure helpers (`is_read_only`, `bytes_to_cost_units`,
/// ...). `principal_kind` comes from [`Principal::kind_for_audit`]
/// (`Principal.id`'s variant), never from `principal.provider` — see that
/// method's doc comment for the `0024_audit_event.sql` CHECK it satisfies.
/// `args.sql` reuses `routes::ai::audit`'s own truncation so an
/// unbounded-length `SQL` string can't bloat `audit_event.args`.
fn query_run_audit_event(principal: &Principal, query_id: &str, sql: &str) -> NewAuditEvent {
    NewAuditEvent {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        action: "query.run".to_owned(),
        resource_kind: Some("query_history".to_owned()),
        resource_id: Some(query_id.to_owned()),
        args: Some(json!({ "sql": crate::routes::ai::audit::truncate_string(sql) })),
        outcome: "executed".to_owned(),
        detail: None,
        run_id: None,
        approval_id: None,
        session_id: None,
    }
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
#[allow(
    clippy::too_many_arguments,
    reason = "one flat result row, no natural grouping"
)]
fn run_result_json(
    id: &str,
    columns: &Value,
    rows: &Value,
    total_rows: usize,
    truncated: bool,
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
        "rowCount": total_rows,
        "truncated": truncated,
        "rowLimit": MAX_RESULT_ROWS,
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
/// missing/blank `sql`. An `EXPLAIN ESTIMATE` failure (e.g. non-`SELECT`
/// input) still answers 200 — the console shows the estimate panel either
/// way — but `estimatedBytes`/costs are `null` and the `ClickHouse` message
/// is carried in `error`, rather than a zeroed estimate that looked
/// identical to "this query reads nothing".
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
        sources
    };

    Ok(ApiJson(estimate_result_json(
        estimated_bytes,
        cost_min,
        cost_max,
        &json!(sources_out),
        freshness_lag_seconds,
        estimate_error.as_deref(),
    )))
}

/// Build the `/api/query/estimate` response body from already-computed
/// values. See [`run_result_json`] for why this is extracted.
#[allow(
    clippy::too_many_arguments,
    reason = "one flat result row, no natural grouping"
)]
fn estimate_result_json(
    estimated_bytes: Option<i64>,
    cost_min: Option<f64>,
    cost_max: Option<f64>,
    sources_out: &Value,
    freshness_lag_seconds: Option<i64>,
    error: Option<&str>,
) -> Value {
    json!({
        "estimatedBytes": estimated_bytes,
        "estimatedCostMin": cost_min,
        "estimatedCostMax": cost_max,
        "error": error,
        // True by construction: Query Studio runs everything through
        // ClickHouse, the hot analytical store.
        "workloadClass": "hot-analytics",
        "engine": "hot-store",
        // Nothing in this deployment configures ClickHouse's query cache,
        // and no policy engine is consulted before a run (`WS7` builds
        // one) — `cacheEligible`/`policyObligations` stay `null`, not a
        // fabricated "Eligible"/`[]`. `freshnessLagSeconds` IS a real
        // measurement (`freshness_lag`, ClickHouse `system.parts`), unlike
        // the `0` it used to hardcode.
        "cacheEligible": Value::Null,
        "freshnessLagSeconds": freshness_lag_seconds,
        "policyObligations": Value::Null,
        "sources": sources_out,
        // WS1 task 1.6: the old `plan` marked stages that had *not run* as
        // status "completed" with `estimatedBytes: 0` — a fabricated
        // measurement of work that was never done. Null until a real plan
        // is computed.
        "plan": Value::Null,
    })
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
    let body: CreateSavedQueryBody = serde_json::from_slice(&body)
        .map_err(|err| ApiError::BadRequest(format!("invalid JSON: {err}")))?;
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
    let owner = principal.map_or_else(|| "anonymous".to_owned(), |Extension(p)| p.display_name);
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
/// every row recorded before this workstream threaded a real `engine`
/// field carries — it named `ClickHouse`'s storage tier, not a different
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
/// Ownership is exact-match on `query_history.owner_id`
/// (`0046_query_ownership.sql`) against the caller's own principal id.
/// Every row recorded before that migration — and every row recorded with
/// no authenticated caller — has `owner_id = NULL`, which never equals any
/// principal's id, so those legacy rows are permanently undownloadable
/// through this route. That is a deliberate, fail-closed consequence of
/// scoping by identity, not an oversight to be worked around.
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
    // pre-`0046` row with no recorded `owner_id` (see this function's doc
    // comment) — a caller cannot distinguish "not yours" from "does not
    // exist" either way. Checked against `owner_id`
    // (`0046_query_ownership.sql`), not the `user` display name: `user` no
    // longer carries a principal id (`QueryHistoryItem::user`'s own doc
    // comment).
    let caller_id = principal.as_ref().map(|Extension(p)| p.id.uuid());
    if caller_id.is_none() || caller_id != item.owner_id {
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

// ── GET /api/query/scheduling (WS2 §13, WS2 plan review W10, round 2, item 1) ──

/// `{ "supported": false, "reason": ... }` — scheduled execution of a saved
/// query needs a safe per-principal authority model that does not exist
/// yet: a scheduled job has no live session to run "as," and running the
/// query under a shared service identity would execute with different
/// authority than the query's author holds at trigger time. That model is
/// planned for WS7's policy-obligations engine, not this workstream.
///
/// An earlier revision of this capability added a `saved_query.schedule_cron`
/// column that the API accepted and stored while nothing ever ran it — dead
/// schema advertising a capability that does not exist. This probe adds no
/// column, no migration, and no field on [`SavedQuery`]: nothing is stored
/// that nothing then reads (AGENTS.md principle 2).
#[must_use]
fn scheduling_capability_body() -> Value {
    json!({
        "supported": false,
        "reason": "scheduled execution needs a safe per-principal authority model, planned for WS7",
    })
}

/// `GET /api/query/scheduling` — a static capability probe the
/// saved-queries UI calls once, so it can render an honest "not supported
/// yet" instead of offering a schedule control that does nothing (WS2 §13,
/// WS2 plan review W10, round 2, item 1).
///
/// # Errors
///
/// This handler never fails — it reports a fixed, static capability.
pub async fn scheduling(State(_state): State<AppState>) -> ApiResult<ApiJson<Value>> {
    Ok(ApiJson(scheduling_capability_body()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_auth::{PermissionSet, PrincipalId};
    use uuid::Uuid;

    use super::*;

    /// A logged-in human principal — `PrincipalId::User`.
    fn fixture_user_principal() -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::from_u128(1)),
            tenant_ids: Vec::new(),
            display_name: "Rina Wijaya".to_owned(),
            permissions: PermissionSet::parse("query:read"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    /// A service-token principal — `PrincipalId::Service`, e.g. the Dagster
    /// orchestrator running a scheduled query.
    fn fixture_service_principal() -> Principal {
        Principal {
            id: PrincipalId::Service(Uuid::from_u128(2)),
            tenant_ids: Vec::new(),
            display_name: "dagster-orchestrator".to_owned(),
            permissions: PermissionSet::parse("query:read"),
            provider: "service".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    #[test]
    fn query_run_audit_event_uses_the_real_principal_and_a_truncated_sql() {
        let principal = fixture_user_principal();
        let event = query_run_audit_event(&principal, "q-123", "SELECT * FROM orders");
        assert_eq!(event.principal_kind.as_deref(), Some("user"));
        assert_eq!(
            event.principal_id.as_deref(),
            Some(principal.id.uuid().to_string().as_str())
        );
        assert_eq!(event.resource_kind.as_deref(), Some("query_history"));
        assert_eq!(event.resource_id.as_deref(), Some("q-123"));
        assert_eq!(event.action, "query.run");
        assert_eq!(event.outcome, "executed");
    }

    #[test]
    fn query_run_audit_event_records_a_service_identity_as_service_not_user() {
        let principal = fixture_service_principal();
        let event = query_run_audit_event(&principal, "q-124", "SELECT 1");
        assert_eq!(event.principal_kind.as_deref(), Some("service"));
    }

    #[test]
    fn query_run_audit_event_truncates_long_sql() {
        // Mirrors `routes::ai::audit`'s own `redact_truncates_long_strings`
        // assertion shape (`…[truncated]` suffix, strictly shorter than the
        // input) rather than a fixed length bound — `truncate_string`
        // keeps the first `MAX_STRING_LEN` characters AND appends the
        // marker, so the output is `MAX_STRING_LEN` plus the marker's own
        // length, not `MAX_STRING_LEN` or fewer.
        let principal = fixture_user_principal();
        let long_sql = "x".repeat(600);
        let event = query_run_audit_event(&principal, "q-125", &long_sql);
        let recorded_sql = event.args.unwrap()["sql"].as_str().unwrap().to_owned();
        assert!(
            recorded_sql.ends_with("…[truncated]"),
            "must reuse ai::audit's MAX_STRING_LEN truncation, not the raw SQL: {recorded_sql:?}"
        );
        assert!(recorded_sql.chars().count() < long_sql.chars().count());
    }

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

    #[test]
    fn run_result_emits_null_for_unmeasured_transparency_fields() {
        let v = run_result_json(
            "q-1",
            &json!([]),
            &json!([]),
            0,
            false,
            12,
            34,
            1,
            "clickhouse",
        );

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
        let v = estimate_result_json(Some(100), Some(1.0), Some(2.0), &json!([]), None, None);

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
                role_names: Vec::new(),
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

    // ── GET /api/query/scheduling (WS2 §13, WS2 plan review W10, round 2, item 1) ──

    mod scheduling_pure {
        use super::*;

        #[test]
        fn scheduling_capability_reports_unsupported_with_a_reason() {
            let body = scheduling_capability_body();
            assert_eq!(body["supported"], serde_json::json!(false));
            assert!(body["reason"].as_str().unwrap().contains("WS7"));
        }
    }

    // ── WS7 item C2: `run` rewrites/refuses through `sql_rewrite::enforce`
    //    before EITHER engine ────────────────────────────────────────────

    mod enforcement {
        //! Real Postgres (`#[sqlx::test]`, a real authored `policy` row)
        //! plus a wiremock `ClickHouse` — the same two harnesses
        //! `tools::data::run_sql_delegation` uses for the equivalent
        //! proof one layer up (the copilot's `run_sql` tool). `ChClient`
        //! has no `last_query()`-style test double, so every assertion
        //! here reads the mock server's own recorded requests, matching
        //! this file's existing `trino_engine::assert_refused_before_reaching_trino`.

        #![allow(clippy::unwrap_used, clippy::expect_used)]

        use std::collections::HashMap;

        use lakehouse_auth::{PermissionSet, PrincipalId};
        use lakehouse_store::governance::CreatePolicyInput;
        use uuid::Uuid;
        use wiremock::matchers::{body_string_contains, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::*;
        use crate::config::Config;

        fn database_url_for(pool: &PgPool) -> String {
            let options = pool.connect_options();
            format!(
                "postgres://{}:postgres@{}:{}/{}",
                options.get_username(),
                options.get_host(),
                options.get_port(),
                options
                    .get_database()
                    .expect("#[sqlx::test] always targets a named database"),
            )
        }

        fn analyst_principal() -> Extension<Principal> {
            Extension(Principal {
                id: PrincipalId::User(Uuid::nil()),
                tenant_ids: Vec::new(),
                display_name: "alice".to_owned(),
                permissions: PermissionSet::parse("query:read"),
                provider: "session".to_owned(),
                must_change_password: false,
                role_names: vec!["Analyst".to_owned()],
            })
        }

        async fn mount_governed_table_responses(server: &MockServer) {
            Mock::given(method("POST"))
                .and(body_string_contains("system.columns"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "meta": [
                        {"name": "name", "type": "String"},
                        {"name": "default_kind", "type": "String"},
                        {"name": "default_expression", "type": "String"},
                    ],
                    "data": [
                        {"name": "id", "default_kind": "", "default_expression": ""},
                        {"name": "email", "default_kind": "", "default_expression": ""},
                    ],
                    "rows": 2,
                })))
                .mount(server)
                .await;
            Mock::given(method("POST"))
                .and(body_string_contains("system.tables"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "meta": [
                        {"name": "database", "type": "String"},
                        {"name": "name", "type": "String"},
                        {"name": "engine", "type": "String"},
                        {"name": "create_table_query", "type": "String"},
                    ],
                    "data": [
                        {"database": "serving", "name": "mart_x", "engine": "MergeTree", "create_table_query": ""},
                    ],
                    "rows": 1,
                })))
                .mount(server)
                .await;
            Mock::given(method("POST"))
                .and(body_string_contains("replaceRegexpOne"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "meta": [
                        {"name": "id", "type": "UInt64"},
                        {"name": "email", "type": "String"},
                    ],
                    "data": [{"id": "1", "email": "***"}],
                    "rows": 1,
                })))
                .mount(server)
                .await;
        }

        /// Failing-test-first for WS7 item C2: before this task wired
        /// `sql_rewrite::enforce` into `run`, this assertion failed
        /// because `run` sent the LITERAL, unmasked `SELECT * FROM
        /// serving.mart_x` straight to `ClickHouse` — no request in
        /// `server.received_requests()` ever contained
        /// `"replaceRegexpOne"` at all. Quoted failure text (captured
        /// before Step 2's implementation): `assertion failed:
        /// requests.iter().any(|r| ...contains("replaceRegexpOne"))`.
        #[sqlx::test(migrations = "../../migrations")]
        async fn run_rewrites_sql_for_a_governed_table_before_executing(
            pool: PgPool,
        ) -> sqlx::Result<()> {
            lakehouse_store::governance::create_policy(
                &pool,
                &CreatePolicyInput {
                    name: "query-run-masking-test".to_owned(),
                    kind: "Row filter".to_owned(),
                    subjects: "Analyst".to_owned(),
                    resources: "serving.mart_x".to_owned(),
                    effect: "Permit with obligation".to_owned(),
                    conditions: Some(
                        r#"{"roles":["Analyst"],"table":"serving.mart_x","mask":["email"]}"#
                            .to_owned(),
                    ),
                    activate: true,
                    owner: None,
                },
            )
            .await
            .expect("seeding the governing policy must succeed");

            let server = MockServer::start().await;
            mount_governed_table_responses(&server).await;

            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), database_url_for(&pool));
            env.insert("CH_URL".to_owned(), server.uri());
            let state = AppState::new(Config::from_map(&env).expect("valid config from a map"));

            let body = Bytes::from(json!({ "sql": "SELECT * FROM serving.mart_x" }).to_string());
            let result = run(State(state), Some(analyst_principal()), body).await;
            assert!(
                result.is_ok(),
                "expected a successful, masked run: {result:?}"
            );

            let requests = server
                .received_requests()
                .await
                .expect("mock server records requests");
            assert!(
                requests
                    .iter()
                    .any(|r| String::from_utf8_lossy(&r.body).contains("replaceRegexpOne")),
                "expected the query ClickHouse actually received to be the rewritten/masked \
                 form, never the original literal SQL"
            );
            Ok(())
        }

        /// Failing-test-first for WS7 item C2's Hard Requirement 2 (never
        /// silently pass an unrewritten/unrefused query through): before
        /// this task, `run` had no classification step at all, so a
        /// table-function read reached `ClickHouse` unrefused —
        /// `server.received_requests()` would have been non-empty.
        /// Quoted failure text (captured before Step 2's implementation):
        /// `assertion failed: requests.is_empty()`, `left: false, right:
        /// true` (a request WAS recorded, the table function reached
        /// `ClickHouse`).
        #[sqlx::test(migrations = "../../migrations")]
        async fn run_refuses_a_table_function_before_touching_clickhouse_at_all(
            pool: PgPool,
        ) -> sqlx::Result<()> {
            let server = MockServer::start().await;
            // No mocks mounted at all — any request reaching `ClickHouse`
            // fails with a connection/404 error, which would itself prove
            // the refusal came too late.
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), database_url_for(&pool));
            env.insert("CH_URL".to_owned(), server.uri());
            let state = AppState::new(Config::from_map(&env).expect("valid config from a map"));

            let body = Bytes::from(json!({ "sql": "SELECT * FROM url('h','CSV')" }).to_string());
            let result = run(State(state), Some(analyst_principal()), body).await;
            let err = result.expect_err("a table function must be refused");
            assert_eq!(err.into_response().status(), 422);

            let requests = server
                .received_requests()
                .await
                .expect("mock server records requests");
            assert!(
                requests.is_empty(),
                "a table function must never reach ClickHouse at all: {requests:?}"
            );
            Ok(())
        }
    }
}
