//! Small helpers shared by the route modules.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::Arc;

use lakehouse_bi::builder::{FilteredSql, RelationColumns};
use lakehouse_bi::filters::ColumnKind;
use lakehouse_bi::specs::ChartSource;
use lakehouse_bi::store;
use lakehouse_clickhouse::{ChClient, ChError, ChResult};
use serde_json::{Map, Value, json};

use crate::upstream_error;

/// Render any JSON value the way `String(x)` renders it in `TypeScript`,
/// with `null`/missing treated as `""` (matching `String(row[name] ?? "")`
/// in `src/app/api/catalog/[id]/route.ts`).
#[must_use]
pub(crate) fn js_string(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(other @ (Value::Array(_) | Value::Object(_))) => other.to_string(),
    }
}

/// Parse a `ClickHouse` `toString(...)` column as an integer, defaulting to
/// `0` on a missing row, a missing column, or a value that doesn't parse —
/// matching `Number(x) || 0` in `TypeScript`, where `NaN` (a failed parse)
/// is falsy and also becomes `0`.
#[must_use]
pub(crate) fn num_or_zero(row: Option<&Map<String, Value>>, key: &str) -> i64 {
    row.and_then(|r| r.get(key))
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
}

/// A `ClickHouse` `Nullable(UInt64)` column, read as `Option<u64>` and
/// NEVER defaulted to `0` — the counterpart to `support::num_or_zero` for a
/// column where "absent" and "zero" are two different, both-real facts
/// (WS3 plan review Z9; see [`IngestRunRow::rows`]'s doc comment).
///
/// `ClickHouse`'s `FORMAT JSON` renders `UInt64`/`Nullable(UInt64)` as a
/// quoted string when the server's `output_format_json_quote_64bit_integers`
/// is on, and as a bare JSON number when it is off (the default on the
/// `ClickHouse` 26.8 this deployment pins) — this accepts both.
/// Only a JSON `null` or a missing column becomes `None`; a value that
/// fails to parse also becomes `None` rather than crashing the whole
/// response over one bad row, matching this module's other `ClickHouse`
/// readers' "never let one row's odd shape sink the list" posture.
#[must_use]
pub(crate) fn nullable_u64_col(row: &Map<String, Value>, key: &str) -> Option<u64> {
    match row.get(key) {
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::String(s)) => s.parse::<u64>().ok(),
        _ => None,
    }
}
/// The signed counterpart of [`nullable_u64_col`], for an
/// `Int64`/`Nullable(Int64)` column (e.g. an Iceberg snapshot id): same
/// "number or quoted string, never defaulted" contract.
#[must_use]
pub(crate) fn nullable_i64_col(row: &Map<String, Value>, key: &str) -> Option<i64> {
    match row.get(key) {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => s.parse::<i64>().ok(),
        _ => None,
    }
}

/// A `ClickHouse` string column, defaulting to `""` when the row or column
/// is missing.
#[must_use]
pub(crate) fn str_col<'a>(row: &'a Map<String, Value>, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or("")
}

/// `s.replace(/_/g, " ").replace(/\b\w/g, (m) => m.toUpperCase())` —
/// underscores become spaces, then the first letter of every word is
/// capitalized. Ported identically in `catalog/route.ts`,
/// `catalog/[id]/route.ts`.
#[must_use]
pub(crate) fn prettify(s: &str) -> String {
    let spaced = s.replace('_', " ");
    let mut out = String::with_capacity(spaced.len());
    let mut at_word_start = true;
    for c in spaced.chars() {
        if at_word_start && c.is_ascii_alphanumeric() {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
        // `\w` is `[A-Za-z0-9_]`; since underscores were already replaced
        // with spaces, any non-alphanumeric character starts a new word.
        at_word_start = !c.is_ascii_alphanumeric();
    }
    out
}

/// The text a route puts in an `error`/`unavailable` field when an upstream
/// call failed: a fixed sentence and a reference id, never `err`'s own text.
///
/// SEC-11: this used to be `js_error`, which rendered `"Error: <message>"`
/// the way the `TypeScript` ports did and so copied `ClickHouse`'s, Dagster's
/// and the catalog's text into response bodies. The raw error is logged under
/// the id instead (`upstream_error::report`). Use a typed
/// `upstream_error::report_*` call where the caller needs the id or the
/// unavailable/failed distinction as data; this is for the sites that only
/// ever wanted a string.
#[must_use]
pub(crate) fn upstream_message(err: impl Display) -> String {
    crate::upstream_error::report(
        &crate::upstream_error::SERVICE,
        crate::upstream_error::Class::Failed,
        &err,
    )
    .to_string()
}

/// The outermost `{...}` in a model's reply, so a model that wraps its JSON
/// in prose or a code fence is still understood. Shared by every caller that
/// asks the model for one JSON object (`pipelines::llm_pipeline_draft`, the
/// semantic layer's drafting pass).
#[must_use]
pub(crate) fn extract_json_object(reply: &str) -> Option<&str> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    (end > start).then(|| &reply[start..=end])
}

/// `ClickHouse` type-name substrings treated as numeric, matching
/// `/Int|Float|Decimal/` in `dashboard/fields/route.ts` (and reused,
/// identically, by `ai-tools.ts`'s `describe_mart`/`suggest_dashboard`
/// tools).
const NUMERIC_TYPE_MARKERS: [&str; 3] = ["Int", "Float", "Decimal"];

/// `s.replace(/[^a-zA-Z0-9_]/g, "")` — strip (not reject) anything outside
/// `[A-Za-z0-9_]`. Shared by every route that sanitizes a mart/column name
/// this way rather than rejecting it outright.
#[must_use]
pub(crate) fn strip_non_ident(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

/// `NUMERIC.test(type)` — `/Int|Float|Decimal/` substring match.
#[must_use]
pub(crate) fn is_numeric_type(ty: &str) -> bool {
    NUMERIC_TYPE_MARKERS.iter().any(|m| ty.contains(m))
}

/// Render a *stored* chart spec (owned strings, `lakehouse_bi::store::ChartSpec`)
/// the same way `lakehouse_bi::specs::to_render_spec` renders a built-in
/// `&'static` one: everything except `sql`, plus its origin. There is no
/// shared `to_render_spec` overload across the two `ChartSpec` types (one
/// `&'static str`-backed for compile-time specs, one owned for
/// runtime-assembled ones), so this mirrors that function's field set by
/// hand. Shared by `dashboard` and `embed`, which both render stored chart
/// lists.
#[must_use]
pub(crate) fn render_stored_spec(spec: &store::ChartSpec, source: ChartSource) -> Value {
    let mut map = Map::new();
    map.insert("id".to_owned(), json!(spec.id));
    map.insert("title".to_owned(), json!(spec.title));
    map.insert("kind".to_owned(), json!(spec.kind));
    map.insert("mart".to_owned(), json!(spec.mart));
    if let Some(v) = &spec.sql_source {
        map.insert("sqlSource".to_owned(), json!(v));
    }
    map.insert("x".to_owned(), json!(spec.x));
    map.insert("y".to_owned(), json!(spec.y));
    map.insert("source".to_owned(), json!(source));
    if let Some(v) = &spec.subtitle {
        map.insert("subtitle".to_owned(), json!(v));
    }
    if let Some(v) = &spec.series {
        map.insert("series".to_owned(), json!(v));
    }
    // The map kinds' view of the rows: without these the console cannot tell
    // which bundled map to draw or which columns hold the coordinates.
    if let Some(v) = &spec.map {
        map.insert("map".to_owned(), json!(v));
    }
    if let Some(v) = &spec.lat {
        map.insert("lat".to_owned(), json!(v));
    }
    if let Some(v) = &spec.lon {
        map.insert("lon".to_owned(), json!(v));
    }
    if let Some(v) = spec.format {
        map.insert("format".to_owned(), json!(v));
    }
    if let Some(v) = spec.span {
        map.insert("span".to_owned(), json!(v));
    }
    if let Some(v) = &spec.text {
        map.insert("text".to_owned(), json!(v));
    }
    if let Some(v) = &spec.caption {
        map.insert("caption".to_owned(), json!(v));
    }
    if let Some(v) = spec.target {
        map.insert("target".to_owned(), json!(v));
    }
    Value::Object(map)
}

/// `runSpec` in `dashboard/route.ts` (and its `embed`/`public` siblings): an
/// empty `sql` (text tiles) needs no query; any other failure is captured
/// per-tile rather than failing the whole response.
///
/// WS7 item D1: `sql` is rewritten through
/// `policy_engine::rewrite_sql_for_roles` — the SAME shared entry point
/// `routes::query::run` (WS7 item C2) and `gold_export::export_batch_sql`
/// (WS7 item D2) call — for `roles`/`placeholders` BEFORE it ever reaches
/// `ch.query`. A refusal (a table function, a masking/row-filter policy
/// this module cannot prove safe, an authored-but-unenforceable
/// condition, ...) is captured into this tile's own `{"error": ...}`
/// slot, exactly like a `ClickHouse` query failure already is — the
/// dashboard still renders its other tiles — but `ch.query` is never
/// called for a refused tile, so a masked/filtered chart can never
/// silently execute the unrewritten `sql` instead.
pub(crate) async fn run_spec_sql(
    ch: &ChClient,
    gate: &crate::routes::query::StatementGate<'_>,
    id: &str,
    sql: &str,
    roles: &[String],
    placeholders: &crate::sql_rewrite::PlaceholderValues,
    obligations: &crate::policy_engine::PolicyEngineObligations<'_>,
) -> (String, Value) {
    let result = match try_run_spec_sql(ch, gate, sql, roles, placeholders, obligations).await {
        Ok(result) => result,
        Err(SpecRunFailure::Refused(message)) => json!({ "error": message }),
        Err(SpecRunFailure::Tile(answer)) => answer,
        // `SEC-50` slice 2 M3: the gate's own refusal, in the tile's own
        // error slot — the same sentence the route would have answered with,
        // so a tile that may not read its table says why.
        Err(SpecRunFailure::Gated(err)) => json!({ "error": err.to_string() }),
        // SEC-11: a tile on a public or embed link is read by someone with
        // no sign-in; the database's text goes to the log under the id.
        Err(SpecRunFailure::Clickhouse(err)) => {
            upstream_error::report_ch(&upstream_error::TILE, &err).to_json()
        }
    };
    (id.to_owned(), result)
}

/// Why [`try_run_spec_sql`] produced no rows. Kept apart so a caller whose
/// responses must not carry `ClickHouse` text can classify the second kind
/// itself; [`run_spec_sql`] keeps the tile behaviour of showing both.
#[derive(Debug)]
pub(crate) enum SpecRunFailure {
    /// Refused by the policy rewrite before any query ran; the text is fixed.
    /// A `String` because a `GOV-7` refusal names the table (the caller's
    /// own SQL) with `SEC-20`'s sentence.
    Refused(String),
    /// `SEC-50` slice 2 M3: refused by the tenant gate before the rewrite
    /// and before the engine.
    ///
    /// The gate's own error, kept as it is: the sentence is `SEC-20`'s (a
    /// 404 naming the table) or D6's (a 422 naming the setting), never
    /// upstream text, so it may be shown in the tile's error slot.
    Gated(lakehouse_core::ApiError),
    /// The query ended on a limit or a Stop (`BI-5` plan D6): the tile's
    /// whole answer, with a fixed sentence and, for the time limit, a `limit`
    /// object. Built here so every caller says the same thing and none of
    /// them sees the engine's text.
    Tile(Value),
    /// `ClickHouse` failed on the rewritten SQL.
    Clickhouse(ChError),
}

/// What a caller with a data policy is told when a statement with a `RegionOf`
/// lookup is refused (`BI-16` part C3, option B signed by the owner on
/// 2026-10-24). The rewrite refuses every `dictGet*` call of such a caller
/// (`sql_rewrite::DictOrJoinFunctionDenied`: a dictionary's source cannot be
/// seen from the query text); the refusal stays, only the words change, so the
/// caller learns which function and why, in English. The narrow exemption for
/// the server-made `console.geo_dict_*` dictionaries is a backlog follow-up
/// for the security team.
pub(crate) const REGION_OF_REFUSED: &str = "RegionOf is not available to you: your role has a data policy, and the region lookup cannot be checked against it yet. Ask an administrator, or use a column that already holds the region.";

/// The `{{principal_id}}`/`{{principal_tenant_ids}}` values the policy rewrite
/// expands for `principal` — one function, because the dashboard preview and
/// the pipeline preview (BLD-1 part 1 / D16) rewrite a caller's statement with
/// the same placeholder set and a third copy would be the drift `AGENTS.md`
/// rule 4 names.
#[must_use]
pub(crate) fn placeholders_for(
    principal: &lakehouse_auth::Principal,
) -> crate::sql_rewrite::PlaceholderValues {
    crate::sql_rewrite::PlaceholderValues {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_tenant_ids: principal
            .tenant_ids
            .iter()
            .map(ToString::to_string)
            .collect(),
    }
}

/// The columns `rewritten` returns, via `DESCRIBE` — no rows are read.
///
/// The column probe the dashboard preview owns and the pipeline preview
/// reuses (BLD-1 part 1 / D16), rather than a second `DESCRIBE` written for the
/// new route. A `ClickHouse` failure comes back as the engine's own
/// [`ChError`], because the two callers answer it differently: the dashboard
/// preview fixes the sentence, the pipeline preview reports it the way Query
/// Studio does.
///
/// # Errors
///
/// The engine's own [`ChError`] from the `DESCRIBE`.
pub(crate) async fn describe_columns(
    ch: &ChClient,
    rewritten: &str,
) -> Result<Vec<lakehouse_bi::sources::SourceColumn>, ChError> {
    let sql = format!("DESCRIBE (SELECT * FROM (\n{rewritten}\n))");
    let rows = ch.rows(&sql, None).await?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            Some(lakehouse_bi::sources::SourceColumn {
                name: row.get("name")?.as_str()?.to_owned(),
                ty: row.get("type")?.as_str()?.to_owned(),
            })
        })
        .collect())
}

/// [`crate::policy_engine::enforcement_error_message`], with the sentence
/// above for a refused `dictGet*` call in a statement that does a `RegionOf`
/// lookup (it names a `console.geo_dict_` dictionary); every other refusal
/// keeps its fixed message.
pub(crate) fn enforcement_message_for(
    sql: &str,
    err: &crate::policy_engine::EnforcementError,
) -> String {
    let denied = matches!(
        err,
        crate::policy_engine::EnforcementError::Rewrite(
            crate::sql_rewrite::RewriteError::DictOrJoinFunctionDenied { .. }
        )
    );
    if denied && sql.contains("console.geo_dict_m_") {
        REGION_OF_REFUSED.to_owned()
    } else {
        crate::policy_engine::enforcement_error_message(err)
    }
}

/// The ONE mapping from a policy-rewrite refusal to the answer a caller gets
/// (`GOV-7` D5): every governed read path that builds an `ApiError` from an
/// `EnforcementError` calls this, so a table a role may not read is answered
/// with `SEC-20`'s own 404 and sentence (`D2`) and a narrowed principal's
/// listing with `SEC-20`'s 422 sentence — never "forbidden", and never a
/// second copy of either sentence.
///
/// `sql` is only read for the `RegionOf` sentence above; the table named in a
/// refusal is the caller's own SQL, never upstream text.
pub(crate) fn enforcement_refusal(
    sql: &str,
    err: &crate::policy_engine::EnforcementError,
) -> lakehouse_core::ApiError {
    match err {
        crate::policy_engine::EnforcementError::TableNotReadable(table) => {
            lakehouse_core::ApiError::NotFound(crate::policy_engine::table_not_available_message(
                table,
            ))
        }
        crate::policy_engine::EnforcementError::Unresolvable => {
            lakehouse_core::ApiError::Unprocessable(
                crate::policy_engine::LISTING_NOT_AVAILABLE.to_owned(),
            )
        }
        _ => lakehouse_core::ApiError::Unprocessable(enforcement_message_for(sql, err)),
    }
}

/// The body of [`run_spec_sql`]: the rewrite for `roles`, then the query, as
/// `{"columns", "rows"}`. An empty `sql` (text tiles) needs no query.
///
/// # Errors
///
/// [`SpecRunFailure::Refused`] when the rewrite refuses (nothing was sent to
/// `ClickHouse`), [`SpecRunFailure::Gated`] when the tenant gate refuses
/// (nothing was sent to `ClickHouse` either), [`SpecRunFailure::Clickhouse`]
/// when the query fails.
pub(crate) async fn try_run_spec_sql(
    ch: &ChClient,
    gate: &crate::routes::query::StatementGate<'_>,
    sql: &str,
    roles: &[String],
    placeholders: &crate::sql_rewrite::PlaceholderValues,
    obligations: &crate::policy_engine::PolicyEngineObligations<'_>,
) -> Result<Value, SpecRunFailure> {
    if sql.is_empty() {
        return Ok(json!({ "columns": [], "rows": [] }));
    }
    // `SEC-50` slice 2 M3: the tenant gate runs BEFORE the rewrite, so a
    // tile that may not read its table is refused before the policy's own
    // prefetch asks the engine about that table, before the result cache is
    // looked up, and before the engine is asked at all.
    gate.check(sql).await.map_err(SpecRunFailure::Gated)?;
    let rewritten = crate::policy_engine::rewrite_sql_for_roles(
        sql,
        &sqlparser::dialect::ClickHouseDialect {},
        crate::sql_rewrite::Engine::ClickHouse,
        roles,
        placeholders,
        obligations,
    )
    .await
    // `BI-16` part C3 (option B): a refused `RegionOf` lookup is told in
    // English, naming the function; every other refusal keeps its fixed
    // message (`enforcement_message_for`).
    .map_err(|err| SpecRunFailure::Refused(enforcement_message_for(sql, &err)))?;
    // BI-5 (plan D3): the statement runs under the caller's limits. The
    // SQL-source cap in the text is swapped for the configured numbers (it is
    // also stored with the chart, so the stored text cannot carry them), and
    // the same numbers travel as URL settings, with the query id a Stop finds
    // it by. One row more than the cap is asked for, so a cut is told from an
    // exact fit.
    let run = LimitedRun::current().await?;
    // BI-4 part B: the tables are named on the rewritten statement (after the
    // masks wrapped the governed ones), the one the engine would run. Before
    // the settings swap: the swap only rewrites the trailing `SETTINGS` text.
    let tables = crate::sql_rewrite::referenced_tables(
        &rewritten,
        &sqlparser::dialect::ClickHouseDialect {},
    );
    let rewritten = run.statement(&rewritten);
    let rows_cap = run.rows_cap();
    let execute = || async {
        let mut r = run.send(ch, &rewritten, rows_cap).await?;
        let columns: Vec<String> = r.meta.iter().map(|m| m.name.clone()).collect();
        // The engine was asked for `chart_rows + 1` rows and `break`s at the
        // end of a block, so a result of more than `chart_rows + 1` rows was
        // cut (a block is whole, it does not end one row past the cap).
        // Exactly `chart_rows + 1` is a complete result that is one row over:
        // it is kept as it is, because pivots and previews read one row past
        // their own cap to tell whether they cut (`pivot_trim`,
        // `preview_rows`), and trimming here would blind them.
        let cut = r.data.len() > rows_cap as usize + 1;
        if cut {
            r.data.truncate(rows_cap as usize);
        }
        let mut out = json!({ "columns": columns, "rows": r.data });
        if cut {
            out["limitHit"] = run.limits().rows_limit_json(rows_cap);
        }
        Ok::<_, SpecRunFailure>(crate::result_cache::Computed {
            value: out,
            // A cut result is answered, never remembered (BI-5 plan D6).
            complete: !cut,
        })
    };
    // BI-4 part B (plan D3, D4): the one place a remembered result is looked
    // up. The key is the statement as the engine receives it plus the limit
    // settings it runs under, so a different mask, a different principal
    // filter or a different limit is a different key.
    let Some(scope) = crate::result_cache::current() else {
        return execute().await.map(|done| done.value);
    };
    let settings = [
        (
            "max_execution_time",
            u64::try_from(run.limits().timeout.as_millis()).unwrap_or(u64::MAX),
        ),
        ("max_result_rows", u64::from(rows_cap) + 1),
    ];
    let mut answer = scope
        .run(ch, (&rewritten, &settings), tables, execute)
        .await?;
    // BI-4 part B (plan D8): when the engine computed the answer and whether
    // it came from memory, for the tile to say. Only where a scope is open:
    // a call with none (Query Studio, a preview) keeps the shape it had.
    if let Value::Object(map) = &mut answer.value {
        map.insert(
            "computedAt".to_owned(),
            json!(
                answer
                    .computed_at
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            ),
        );
        map.insert("remembered".to_owned(), json!(answer.remembered));
    }
    Ok(answer.value)
}

/// A `ClickHouse` failure of a tile query as the tile's failure: the engine
/// stopping the query at the time limit and a Stop (`KILL QUERY`) each get a
/// fixed answer (plan D6); anything else stays for the caller to classify.
fn tile_failure(err: ChError, limits: &crate::query_limits::TileLimits) -> SpecRunFailure {
    if err.is_time_limit() {
        SpecRunFailure::Tile(json!({
            "error": crate::query_limits::time_sentence("chart", limits),
            "limit": limits.time_limit_json(),
        }))
    } else if err.is_killed_on_engine() {
        SpecRunFailure::Tile(json!({ "error": crate::query_limits::STOPPED_SENTENCE }))
    } else {
        SpecRunFailure::Clickhouse(err)
    }
}

/// The one sender of a statement that runs under the caller's limits
/// (`BI-45`, plan D4; QA `LH-BI-5-01`).
///
/// A tile, a chart preview, a save's own check and the SQL source preview's
/// row read all reach `ClickHouse` through here. Four steps happen in one
/// place, so no caller can have two of them and miss the others: the limits of
/// the request's run (`QueryRun::current_or_detached().limits()`), the
/// SQL-source `SETTINGS` text swapped for the administrator's numbers
/// (`with_run_settings`), the run's options (time limit, row cap plus one, a
/// query id) sent as URL settings, and the engine's answer mapped by
/// [`tile_failure`] — so a limit the engine enforced is the limit's sentence
/// and never the engine's text (AGENTS.md principle 4).
pub(crate) struct LimitedRun {
    run: Arc<crate::query_limits::QueryRun>,
    limits: crate::query_limits::TileLimits,
}

impl LimitedRun {
    /// The run of the request being served (or a detached one) and its limits.
    ///
    /// # Errors
    ///
    /// [`SpecRunFailure::Refused`] when the limits cannot be read; nothing is
    /// sent in that case.
    pub(crate) async fn current() -> Result<Self, SpecRunFailure> {
        let run = crate::query_limits::QueryRun::current_or_detached();
        let limits = run.limits().await.map_err(|err| {
            tracing::error!(error = %err, "the query limits could not be read");
            SpecRunFailure::Refused("The query limits could not be read.".to_owned())
        })?;
        Ok(Self { run, limits })
    }

    /// The limits every statement of this run is sent under.
    pub(crate) fn limits(&self) -> &crate::query_limits::TileLimits {
        &self.limits
    }

    /// `sql` with its SQL-source cap swapped for the configured numbers: the
    /// chart row limit plus one (so a cut is told from an exact fit) and the
    /// time limit in whole seconds.
    pub(crate) fn statement(&self, sql: &str) -> String {
        let seconds = u32::try_from(self.limits.timeout_seconds()).unwrap_or(u32::MAX);
        lakehouse_bi::builder::with_run_settings(
            sql,
            self.limits.chart_rows.saturating_add(1),
            seconds,
        )
    }

    /// The rows a statement may read: `BI-5` review B1 — a point map reads up
    /// to the map limit and a pivot up to its own cap, set around the tile by
    /// [`with_tile_rows`]; every other kind stays at the chart row limit. Only
    /// the outer cap moves: the SQL-source `SETTINGS` text keeps the chart row
    /// limit.
    pub(crate) fn rows_cap(&self) -> u32 {
        self.limits
            .chart_rows
            .max(crate::query_limits::row_cap_override().unwrap_or(0))
    }

    /// Send `statement` under this run's limits with `rows_cap` rows.
    ///
    /// # Errors
    ///
    /// [`SpecRunFailure::Tile`] when Stop was pressed on this run before the
    /// query went out, or when the engine stopped the query at the time limit
    /// or with a `KILL QUERY`; [`SpecRunFailure::Clickhouse`] for every other
    /// engine failure, for the caller to classify (its text never reaches a
    /// response).
    pub(crate) async fn send(
        &self,
        ch: &ChClient,
        statement: &str,
        rows_cap: u32,
    ) -> Result<ChResult, SpecRunFailure> {
        // BI-5 review S2: Stop was pressed on this run while the tile waited
        // for its round; it asks the engine nothing.
        if self.run.is_stopped() {
            return Err(SpecRunFailure::Tile(
                json!({ "error": crate::query_limits::STOPPED_SENTENCE }),
            ));
        }
        let options = self.run.options(&self.limits, rows_cap);
        ch.query_with_options(statement, &options, None)
            .await
            .map_err(|err| tile_failure(err, &self.limits))
    }
}

/// Run `sql` once under the caller's limits (`BI-45`, plan D4): the limits,
/// the settings swap, the run's options and the failure mapping all come from
/// [`LimitedRun`]. For a statement that is not a tile — a save's own check and
/// the SQL source preview's row read — whose rows the caller does not keep.
///
/// # Errors
///
/// As [`LimitedRun::current`] and [`LimitedRun::send`].
pub(crate) async fn run_statement_limited(
    ch: &ChClient,
    sql: &str,
) -> Result<ChResult, SpecRunFailure> {
    let run = LimitedRun::current().await?;
    let statement = run.statement(sql);
    run.send(ch, &statement, run.rows_cap()).await
}

/// [`SpecRunFailure`] as the answer of a console route (`BI-45`, plan D6/D5;
/// QA `LH-BI-5-01`): a limit or a Stop is a 422 with the fixed sentence the
/// sender built, a refused rewrite is its own 422, the gate keeps its 404/422,
/// and an engine failure goes through the caller's own mapping — so a route
/// that saves or previews a statement keeps the status it always answered
/// with, never the engine's text (AGENTS.md principle 4).
pub(crate) fn failure_as_api_error(
    failure: SpecRunFailure,
    on_engine: impl FnOnce(&ChError) -> lakehouse_core::ApiError,
) -> lakehouse_core::ApiError {
    let not_finished =
        || lakehouse_core::ApiError::Unprocessable("The query did not finish.".to_owned());
    match failure {
        SpecRunFailure::Tile(answer) => {
            match answer.get("error").and_then(serde_json::Value::as_str) {
                Some(sentence) => lakehouse_core::ApiError::Unprocessable(sentence.to_owned()),
                None => not_finished(),
            }
        }
        SpecRunFailure::Refused(message) => lakehouse_core::ApiError::Unprocessable(message),
        SpecRunFailure::Gated(err) => err,
        SpecRunFailure::Clickhouse(err) => on_engine(&err),
    }
}

/// Save `spec` from the console, its own statement checked under the caller's
/// limit first (`BI-45`, plan D5; QA `LH-BI-5-01`).
///
/// The statement is run AS STORED, not rewritten for roles: a save keeps
/// today's meaning, "does this statement execute", and only gains the time
/// limit, the row cap and the query id — a chart a role's masks would change
/// is saved as its author wrote it, and the mask is applied when someone reads
/// it. The import of a dashboard file, the version revert and Duplicate of a
/// whole dashboard still call [`store::smoke_test_chart`]: they check up to
/// 200 charts in one request and need their own rule for a chart the limit
/// ends (plan §4).
///
/// # Errors
///
/// [`SpecRunFailure::Tile`] when the limit or a Stop ended the statement (the
/// answer carries the limit's sentence), [`SpecRunFailure::Refused`] when the
/// limits cannot be read, or [`SpecRunFailure::Clickhouse`] when the engine
/// failed or the row could not be written; nothing is stored in any of those
/// cases.
pub(crate) async fn insert_chart_checked(
    ch: &ChClient,
    spec: &store::StoredChartSpec,
) -> Result<(), SpecRunFailure> {
    if !spec.spec.sql.is_empty() {
        // The rows are dropped: a save asks whether the statement executes
        // under the limit, not what it returns.
        run_statement_limited(ch, &spec.spec.sql).await?;
    }
    store::insert_chart_unchecked(ch, spec)
        .await
        .map_err(SpecRunFailure::Clickhouse)
}

/// [`SpecRunFailure`] as an assistant tool's reply (`BI-45`, plan D5): the
/// tile's own answer { "error": <sentence>, "limit": {..} } for a limit or a
/// Stop, so the model can say the limit; a refused rewrite or the gate's own
/// answer as `{ "error": <sentence> }`; an engine failure through
/// [`upstream_error::report_ch`] under a reference id, never its text.
pub(crate) fn failure_as_tool_reply(failure: SpecRunFailure) -> Value {
    match failure {
        SpecRunFailure::Tile(answer) => answer,
        SpecRunFailure::Refused(message) => json!({ "error": message }),
        SpecRunFailure::Gated(err) => json!({ "error": err.to_string() }),
        SpecRunFailure::Clickhouse(err) => {
            upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json()
        }
    }
}

/// Run `fut` (one tile's queries) with the rows its kind may read (`BI-5`
/// review B1): a `pointmap` or `geoheat` reads up to the administrator's map
/// limit, a `pivot` up to the cap its own statement carries (its `LIMIT`, one
/// above `pivot_row_cap`, which the pivot trims itself); any other kind runs as
/// it is, at the chart row limit. `sql` is the tile's statement as built.
pub(crate) fn with_tile_rows<'a, T: Send + 'a>(
    kind: lakehouse_bi::specs::ChartKind,
    sql: &'a str,
    fut: impl std::future::Future<Output = T> + Send + 'a,
) -> futures::future::BoxFuture<'a, T> {
    use lakehouse_bi::specs::ChartKind;
    // Boxed: the tile query's future is large (it holds the cache and the
    // engine call), and awaiting it by value in every tile pipeline trips
    // `clippy::large_futures` there.
    Box::pin(async move {
        let cap = match kind {
            ChartKind::Pointmap | ChartKind::Geoheat => {
                crate::query_limits::QueryRun::current_or_detached()
                    .limits()
                    .await
                    .ok()
                    .map(|limits| limits.map_points)
            }
            ChartKind::Pivot => {
                lakehouse_bi::builder::trailing_limit(sql).map(|limit| limit.saturating_sub(1))
            }
            _ => None,
        };
        match cap {
            Some(rows) => crate::query_limits::with_row_cap(rows, fut).await,
            None => fut.await,
        }
    })
}

/// `filtered` with the point-map limit of the caller's query limits
/// (`BI-5`, feature decision 2) in place of the builder's fixed 5,000, for a
/// `pointmap` or `geoheat`; any other kind comes back as it is. When the
/// limits cannot be read the statement is left as built, and the run itself
/// then refuses with the fixed sentence (`try_run_spec_sql`), so a failure
/// here changes nothing the reader sees.
pub(crate) async fn with_map_limit(
    mut filtered: FilteredSql,
    kind: lakehouse_bi::specs::ChartKind,
) -> FilteredSql {
    use lakehouse_bi::specs::ChartKind;
    if !matches!(kind, ChartKind::Pointmap | ChartKind::Geoheat) {
        return filtered;
    }
    if let Ok(limits) = crate::query_limits::QueryRun::current_or_detached()
        .limits()
        .await
    {
        filtered.sql = lakehouse_bi::builder::with_point_limit(
            &filtered.sql,
            limits.map_points,
            limits.chart_rows,
        );
    }
    filtered
}

/// The dashboard SQL sources the given charts read, by id — one
/// `console.bi_source` read, and none at all when no chart uses a source.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` failure.
pub(crate) async fn sources_for<'a>(
    ch: &ChClient,
    charts: impl IntoIterator<Item = &'a store::StoredChartSpec>,
) -> Result<HashMap<String, lakehouse_bi::sources::SqlSource>, ChError> {
    if !charts.into_iter().any(|c| c.def.sql_source.is_some()) {
        return Ok(HashMap::new());
    }
    Ok(lakehouse_bi::sources::list_sources(ch)
        .await?
        .into_iter()
        .map(|s| (s.id.clone(), s))
        .collect())
}

fn own_columns(
    chart: &store::StoredChartSpec,
    mart_cols: &HashMap<String, RelationColumns>,
    sources: &HashMap<String, lakehouse_bi::sources::SqlSource>,
) -> Option<RelationColumns> {
    match chart.def.sql_source.as_deref() {
        Some(id) => sources
            .get(id)
            .map(lakehouse_bi::sources::SqlSource::column_kinds),
        None => mart_cols.get(&chart.def.mart).cloned(),
    }
}

/// The columns of the marts and the SQL sources a read loaded.
pub(crate) type SourceColumns<'a> = (
    &'a HashMap<String, RelationColumns>,
    &'a HashMap<String, lakehouse_bi::sources::SqlSource>,
);

/// BI-2 review fix (SHOULD-FIX) R3: the label of each metric the tile shows,
/// by metric name, as `metricLabels` on the tile's result, so the console
/// labels a legend, an axis, a tooltip and a table header without a second
/// request. Left out when no metric of the chart has a label.
pub(crate) fn annotate_metric_labels(
    tile: &mut Value,
    chart: &store::StoredChartSpec,
    fields: &lakehouse_bi::fields::FieldCatalog,
    columns: Option<SourceColumns<'_>>,
) {
    let Value::Object(tile) = tile else {
        return;
    };
    let own = columns.and_then(|(marts, sources)| own_columns(chart, marts, sources));
    let labels = fields.metric_labels(&chart.def, own.as_ref());
    if !labels.is_empty() {
        tile.insert("metricLabels".to_owned(), json!(labels));
    }
    // BI-17 (D4): the format hint of each metric, the default format of its value.
    let formats = fields.metric_formats(&chart.def, own.as_ref());
    if !formats.is_empty() {
        tile.insert("metricFormats".to_owned(), json!(formats));
    }
}

/// The SQL to run for a stored chart right now: a mart chart through
/// `sql_with_filters_report`, a SQL-source chart rebuilt from the source's
/// CURRENT text (`sql_for_sql_source_report`), plus the active filters left
/// out of it. `Err` carries a fixed tile error when the source was deleted
/// or the stored definition no longer validates — reported on that tile,
/// never replaced by other SQL.
///
/// # Errors
///
/// The tile's fixed error message.
pub(crate) fn stored_chart_sql(
    chart: &store::StoredChartSpec,
    years: &[i64],
    filters: &[store::FilterDef],
    mart_cols: &HashMap<String, RelationColumns>,
    sources: &HashMap<String, lakehouse_bi::sources::SqlSource>,
    read: &lakehouse_bi::builder::ReadContext<'_>,
) -> Result<FilteredSql, &'static str> {
    // BI-2: a metric or segment that is gone is the tile's plain error; the
    // statement saved with the chart would show the old number or a wider set.
    let own_cols = own_columns(chart, mart_cols, sources);
    if let Some(cols) = &own_cols
        && read.fields.broken_reference(&chart.def, cols, read.time)
    {
        return Err("this chart uses a metric or segment that is missing or no longer checks out");
    }
    let Some(id) = chart.def.sql_source.as_deref() else {
        // BI-7: a join that no longer checks out (a table or column went away)
        // is the tile's error, never the stored statement run unfiltered.
        if let Ok(mart) = lakehouse_core::ident::Ident::new(chart.def.mart.clone())
            && lakehouse_bi::joins::chart_relation(&mart, chart.def.shape.joins_list(), mart_cols)
                .is_err()
        {
            return Err("this chart's joined tables no longer match its definition");
        }
        return Ok(lakehouse_bi::builder::sql_with_filters_report(
            chart, years, filters, mart_cols, read,
        ));
    };
    let source = sources
        .get(id)
        .ok_or("this chart's SQL source no longer exists")?;
    lakehouse_bi::builder::sql_for_sql_source_report(
        chart,
        &source.sql,
        &source.column_kinds(),
        years,
        filters,
        read,
    )
    .ok_or("this chart's definition is invalid")
}

/// The `resolved` object of a tile whose parameters changed its statement
/// (`BI-11`): the columns, and the row limit, that the parameters gave the slots
/// in this read. The console reads series names, labels and clicks from it
/// (`withResolvedSlots`), because the stored definition still holds the slot as
/// `{param}` text. One writer for the dashboard payload and for a public link or
/// embed (`BI-19` part A QA fix P3: the public payload used to leave it out, and
/// the page then tried to read the unresolved slot as a column name).
pub(crate) fn resolved_json(effective: &store::StoredChartSpec) -> serde_json::Value {
    serde_json::json!({
        "dimension": effective.def.dimension,
        "breakdown": effective.def.breakdown,
        "measures": effective.def.measures,
        "limit": effective.def.limit_value(),
    })
}

/// `BI-11`: the chart as this read sees it, with every parameter in its
/// definition replaced by the read's value ([`lakehouse_bi::params::resolve_chart`]).
/// A chart that names no parameter slot comes back as it is.
///
/// The names a column switch may take are the columns of the chart's relation
/// (the mart, its joins, or the SQL source's probed columns) and the
/// calculated fields of that source; the parameter's own list is checked as
/// well. A SQL source that no longer exists leaves the chart untouched so
/// [`stored_chart_sql`] reports it with its own sentence.
///
/// # Errors
///
/// The tile's plain sentence when a parameter is missing, of the wrong kind,
/// or its value does not fit the slot.
pub(crate) fn effective_chart<'a>(
    chart: &'a store::StoredChartSpec,
    mart_cols: &HashMap<String, RelationColumns>,
    sources: &HashMap<String, lakehouse_bi::sources::SqlSource>,
    read: &lakehouse_bi::builder::ReadContext<'_>,
) -> Result<std::borrow::Cow<'a, store::StoredChartSpec>, String> {
    if !lakehouse_bi::params::changes_statement(&chart.def) {
        return Ok(std::borrow::Cow::Borrowed(chart));
    }
    let mut names: std::collections::HashSet<String> = read
        .fields
        .for_chart(&chart.def)
        .iter()
        .map(|f| f.name.clone())
        .collect();
    if let Some(id) = chart.def.sql_source.as_deref() {
        let Some(source) = sources.get(id) else {
            return Ok(std::borrow::Cow::Borrowed(chart));
        };
        names.extend(source.column_kinds().into_keys());
    } else {
        let mart = lakehouse_core::ident::Ident::new(chart.def.mart.clone());
        match (&mart, chart.def.shape.joins_list().is_empty()) {
            (Ok(_), true) | (Err(_), _) => {
                names.extend(
                    mart_cols
                        .get(&chart.def.mart)
                        .map(|c| c.keys().cloned().collect::<Vec<_>>())
                        .unwrap_or_default(),
                );
            }
            (Ok(mart), false) => {
                if let Ok(joined) = lakehouse_bi::joins::chart_relation(
                    mart,
                    chart.def.shape.joins_list(),
                    mart_cols,
                ) {
                    names.extend(joined.cols.into_keys());
                }
            }
        }
    }
    lakehouse_bi::params::resolve_chart(chart, read.time.params(), &|n| names.contains(n))
        .map(std::borrow::Cow::Owned)
}

/// What a tile carries from the chart's parameters (`BI-11`): its reference
/// lines with the values of this read. A tile that carries an error is left
/// alone.
pub(crate) fn annotate_params(
    tile: &mut Value,
    chart: &store::StoredChartSpec,
    params: &lakehouse_bi::params::ParamValues,
) {
    let Value::Object(tile) = tile else {
        return;
    };
    if !tile.contains_key("rows") {
        return;
    }
    let lines = lakehouse_bi::params::reference_lines(&chart.def, params);
    if !lines.is_empty() {
        tile.insert("referenceLines".to_owned(), json!(lines));
    }
}

/// [`annotate_grain`] for a chart that is not on a dashboard (the builder's
/// preview): its own grain, no dashboard switch, no filters.
pub(crate) fn annotate_saved_grain(tile: &mut Value, chart: &store::StoredChartSpec) {
    let Some(grain) = chart
        .def
        .grain
        .as_deref()
        .and_then(lakehouse_bi::grain::Grain::parse)
    else {
        return;
    };
    let filtered = FilteredSql {
        sql: String::new(),
        skipped: Vec::new(),
        grain_skipped: None,
        grain: Some(grain),
        grain_column: None,
        // `BI-46` (plan D12): a chart with a breakdown keeps every bucket, so
        // the surplus-bucket annotation is the builder's own rule — one place,
        // never a second copy that could disagree with it.
        latest_limit: chart
            .def
            .keeps_latest_buckets(grain)
            .then(|| lakehouse_bi::grain::clamp_limit(chart.def.kind, chart.def.limit_value())),
        table: None,
        bins: Vec::new(),
    };
    annotate_grain(tile, &filtered, chart);
}

/// What a grained chart's tile carries beyond its rows (`BI-9`): the grain it
/// was bucketed with (`grain`), the grain a dashboard switch could not apply (`grainSkipped`, like `filtersSkipped`)
/// and, when the chart keeps its latest buckets, the surplus bucket dropped
/// and `truncated: true` if any bucket was cut off. The query asked for one
/// bucket more than the chart's limit exactly so this can tell "the limit
/// cut the result" from "it fits".
///
/// A tile that carries an error (no `rows`) is left alone.
pub(crate) fn annotate_grain(
    tile: &mut Value,
    filtered: &FilteredSql,
    chart: &store::StoredChartSpec,
) {
    let Value::Object(tile) = tile else {
        return;
    };
    // The grain the rows were bucketed with, so the console labels them.
    if let Some(grain) = filtered.grain {
        tile.insert("grain".to_owned(), json!(grain.as_str()));
    }
    if let Some(kind) = filtered.grain_column {
        tile.insert("grainColumn".to_owned(), json!(kind));
    }
    if let Some(grain) = filtered.grain_skipped {
        tile.insert("grainSkipped".to_owned(), json!(grain.as_str()));
    }
    let Some(limit) = filtered.latest_limit else {
        return;
    };
    let Some(Value::Array(rows)) = tile.get_mut("rows") else {
        return;
    };
    let mut objects: Vec<Map<String, Value>> =
        rows.iter().filter_map(|r| r.as_object().cloned()).collect();
    if objects.len() != rows.len() {
        return;
    }
    let cut =
        lakehouse_bi::grain::trim_to_latest(&mut objects, &chart.def.dimension, limit as usize);
    if cut {
        *rows = objects.into_iter().map(Value::Object).collect();
        tile.insert("truncated".to_owned(), json!(true));
    }
}

/// `BI-10`: what the tile of a binned chart carries so the console can label
/// its bins: `bins: { <column>: { mode, width, min, max } }`, read by the
/// statements the builder made over the same filtered relation as the bins,
/// through the same role rewrite as the tile (the width of a viewer whose role
/// filters rows comes from the rows that role may see). A read that fails or is
/// refused leaves its column out: the tile then shows the bin's lower edges
/// as plain numbers, and the failure is only logged under a reference id.
pub(crate) async fn annotate_bins(
    ch: &ChClient,
    gate: &crate::routes::query::StatementGate<'_>,
    tile: &mut Value,
    reads: &[lakehouse_bi::builder::BinRead],
    context: (
        &[String],
        &crate::sql_rewrite::PlaceholderValues,
        &crate::policy_engine::PolicyEngineObligations<'_>,
    ),
) {
    if let Value::Object(tile) = tile {
        add_bins(ch, gate, tile, reads, context).await;
    }
}

async fn add_bins(
    ch: &ChClient,
    gate: &crate::routes::query::StatementGate<'_>,
    tile: &mut Map<String, Value>,
    reads: &[lakehouse_bi::builder::BinRead],
    (roles, placeholders, obligations): (
        &[String],
        &crate::sql_rewrite::PlaceholderValues,
        &crate::policy_engine::PolicyEngineObligations<'_>,
    ),
) {
    if reads.is_empty() || !tile.contains_key("rows") {
        return;
    }
    let mut out = Map::new();
    for read in reads {
        match try_run_spec_sql(ch, gate, &read.sql, roles, placeholders, obligations).await {
            Ok(result) => {
                let row = &result["rows"][0];
                out.insert(
                    read.column.clone(),
                    json!({
                        "mode": read.mode,
                        "width": row["width"],
                        "min": row["min"],
                        "max": row["max"],
                    }),
                );
            }
            Err(SpecRunFailure::Clickhouse(err)) => {
                let _logged = upstream_error::report_ch(&upstream_error::TILE, &err);
            }
            // `SEC-50` slice 2 M3: a bin read the gate refused leaves the
            // bin's label out, exactly as a refused read does — the tile
            // keeps its rows and says nothing about the bins. The refusal
            // itself is the tile's own error when the tile's own statement
            // was refused (it names the same tables).
            Err(
                SpecRunFailure::Gated(_) | SpecRunFailure::Refused(_) | SpecRunFailure::Tile(_),
            ) => {}
        }
    }
    if !out.is_empty() {
        tile.insert("bins".to_owned(), Value::Object(out));
    }
}

/// The bin reads of a chart that is not on a dashboard (the builder's
/// preview): the statements its no-filter dashboard build would make. Empty
/// for a chart without bins, and when the build cannot be made.
pub(crate) async fn preview_bin_reads(
    ch: &ChClient,
    chart: &store::StoredChartSpec,
    time: &lakehouse_bi::grain::TimeContext,
) -> Vec<lakehouse_bi::builder::BinRead> {
    if !chart.def.shape.has_bins() {
        return Vec::new();
    }
    let Ok(cols) = mart_columns(ch).await else {
        return Vec::new();
    };
    let Ok(sources) = sources_for(ch, [chart]).await else {
        return Vec::new();
    };
    let catalog = match lakehouse_bi::fields::list_catalog_fields(ch).await {
        Ok(found) => lakehouse_bi::fields::FieldCatalog::from_fields(found),
        Err(_) => return Vec::new(),
    };
    let read = lakehouse_bi::builder::ReadContext {
        time,
        grain: None,
        fields: &catalog,
    };
    let Ok(chart) = effective_chart(chart, &cols, &sources, &read) else {
        return Vec::new();
    };
    stored_chart_sql(&chart, &[], &[], &cols, &sources, &read)
        .map(|f| f.bins)
        .unwrap_or_default()
}

/// What the tile of a raw table or a pivot carries beyond its rows
/// (`BI-16` part A).
///
/// A raw table's first page gets `total` (the rows its pages walk through),
/// `limit` and `offset`, from the count statement, which goes through the same
/// role rewrite as the page. A count that fails or is refused leaves `total`
/// out: the tile then shows its rows and offers no paging, and the failure is
/// only logged (the page itself already told the caller if the table could not
/// be read). A pivot keeps the first [`lakehouse_bi::tables::pivot_cap`] rows
/// and is marked `truncated` when the statement returned more.
///
/// A tile that carries an error (no `rows`) is left alone.
pub(crate) async fn annotate_table(
    ch: &ChClient,
    gate: &crate::routes::query::StatementGate<'_>,
    tile: &mut Value,
    filtered: &FilteredSql,
    (roles, placeholders, obligations): (
        &[String],
        &crate::sql_rewrite::PlaceholderValues,
        &crate::policy_engine::PolicyEngineObligations<'_>,
    ),
) {
    use lakehouse_bi::builder::TableRead;
    let Value::Object(tile) = tile else {
        return;
    };
    if !tile.contains_key("rows") {
        return;
    }
    add_bins(
        ch,
        gate,
        tile,
        &filtered.bins,
        (roles, placeholders, obligations),
    )
    .await;
    match &filtered.table {
        Some(TableRead::Rows { count_sql }) => {
            match try_run_spec_sql(ch, gate, count_sql, roles, placeholders, obligations).await {
                Ok(counted) => {
                    let total = counted["rows"][0]["n"].as_u64().or_else(|| {
                        counted["rows"][0]["n"]
                            .as_str()
                            .and_then(|t| t.parse().ok())
                    });
                    if let Some(total) = total {
                        tile.insert("total".to_owned(), json!(total));
                        tile.insert(
                            "limit".to_owned(),
                            json!(lakehouse_bi::tables::ROWS_PAGE_SIZE),
                        );
                        tile.insert("offset".to_owned(), json!(0));
                    }
                }
                Err(SpecRunFailure::Clickhouse(err)) => {
                    // The page was read; only its total was not. Logged under
                    // a reference id, nothing of it in the response.
                    let _logged = upstream_error::report_ch(&upstream_error::TILE, &err);
                }
                // `SEC-50` slice 2 M3: a count the gate refused leaves the
                // total out, exactly as a refused count does — the tile
                // keeps its rows and offers no paging.
                Err(
                    SpecRunFailure::Gated(_) | SpecRunFailure::Refused(_) | SpecRunFailure::Tile(_),
                ) => {}
            }
        }
        Some(TableRead::Pivot { row_cap }) => {
            if let Some(Value::Array(rows)) = tile.get_mut("rows")
                && lakehouse_bi::tables::pivot_trim(rows, *row_cap)
            {
                tile.insert("truncated".to_owned(), json!(true));
            }
        }
        // BI-16 part C1: the Gantt chart's reversed rows, counted through the
        // same role rewrite; a failed count only leaves the note out.
        Some(TableRead::Reversed { count_sql }) => {
            match try_run_spec_sql(ch, gate, count_sql, roles, placeholders, obligations).await {
                Ok(counted) => {
                    let n = counted["rows"][0]["n"].as_u64().or_else(|| {
                        counted["rows"][0]["n"]
                            .as_str()
                            .and_then(|t| t.parse().ok())
                    });
                    if let Some(n) = n.filter(|n| *n > 0) {
                        tile.insert("reversedRows".to_owned(), json!(n));
                    }
                }
                Err(SpecRunFailure::Clickhouse(err)) => {
                    let _logged = upstream_error::report_ch(&upstream_error::TILE, &err);
                }
                // A refused statement, and (BI-5) a count the time limit or a
                // Stop ended: the note is left out, as it is for any other
                // failure.
                Err(
                    SpecRunFailure::Gated(_) | SpecRunFailure::Refused(_) | SpecRunFailure::Tile(_),
                ) => {}
            }
        }
        Some(TableRead::Trend { .. }) | None => {}
    }
}

/// [`annotate_table`] for a chart that is not on a dashboard (the builder's
/// preview): a pivot is cut like a tile; a comparing KPI says which period its
/// buckets are cut by. A raw table's total is not counted, the preview shows
/// its first page.
pub(crate) fn annotate_saved_table(tile: &mut Value, chart: &store::StoredChartSpec) {
    let Value::Object(tile) = tile else {
        return;
    };
    if chart.spec.kind == lakehouse_bi::specs::ChartKind::Pivot {
        let cap = lakehouse_bi::tables::pivot_cap(
            chart.def.sql_source.is_some(),
            chart.def.tables.values.as_ref().map_or(1, Vec::len),
        );
        if let Some(Value::Array(rows)) = tile.get_mut("rows")
            && lakehouse_bi::tables::pivot_trim(rows, cap)
        {
            tile.insert("truncated".to_owned(), json!(true));
        }
    }
    if chart.def.tables.compares_to_previous(chart.spec.kind)
        && let Some(period) = chart
            .def
            .tables
            .compare
            .as_ref()
            .and_then(|c| c.period.as_deref())
    {
        tile.insert("grain".to_owned(), json!(period));
    }
}

/// BI-2: the period of a comparing KPI that measures a metric and leaves its
/// date column and period to the metric's defaults. `annotate_saved_table`
/// reads the period from the comparison as written, which is absent then; the
/// preview reads the metric's from the catalog.
pub(crate) fn annotate_metric_period(
    tile: &mut Value,
    chart: &store::StoredChartSpec,
    entries: &[lakehouse_bi::fields::FieldDef],
) {
    let Value::Object(tile) = tile else {
        return;
    };
    if tile.contains_key("grain") || !chart.def.tables.compares_to_previous(chart.spec.kind) {
        return;
    }
    let filled =
        lakehouse_bi::metrics::fill_compare(&chart.def.tables, &chart.def.measures, entries);
    if let Some(period) = filled.compare.as_ref().and_then(|c| c.period.as_deref()) {
        tile.insert("grain".to_owned(), json!(period));
    }
}

/// Most rows of a chart's last result the builder's preview table shows
/// (`BI-6` part 2). Over a SQL source the engine itself returns at most 2,000
/// rows ([`lakehouse_bi::builder::SQL_SOURCE_SETTINGS`]), so the cap is one
/// below and a cut is seen instead of hidden by the engine.
pub(crate) const PREVIEW_RESULT_ROWS: usize = 2000;

/// The rows beside the chart in the builder's preview: up to
/// [`PREVIEW_RESULT_ROWS`] of the chart's last result, with `truncated` when
/// more exist. A chart with steps keeps only its limit in the tile, so the
/// preview reads the same statement with a larger limit (through the same role
/// rewrite); any other chart's last result is the rows the tile already has.
/// The saved chart's own limit is not touched. `None` when the tile carries an
/// error or the longer read fails (the failure is logged under a reference).
pub(crate) async fn preview_rows(
    ch: &ChClient,
    gate: &crate::routes::query::StatementGate<'_>,
    chart: &store::StoredChartSpec,
    result: &Value,
    (roles, placeholders, obligations): (
        &[String],
        &crate::sql_rewrite::PlaceholderValues,
        &crate::policy_engine::PolicyEngineObligations<'_>,
    ),
) -> Option<Value> {
    let cap = if chart.def.sql_source.is_some() {
        PREVIEW_RESULT_ROWS - 1
    } else {
        PREVIEW_RESULT_ROWS
    };
    let mut shown = result.clone();
    if chart.def.shape.steps.is_some() {
        let limit = chart.def.limit_value().unwrap_or(20);
        let sql = lakehouse_bi::builder::steps::with_limit(
            &chart.spec.sql,
            limit,
            u32::try_from(cap + 1).ok()?,
        )?;
        shown = match try_run_spec_sql(ch, gate, &sql, roles, placeholders, obligations).await {
            Ok(rows) => rows,
            // `SEC-50` slice 2 M3: a statement the gate refused shows no
            // rows, exactly as a refused statement does.
            Err(
                SpecRunFailure::Gated(_) | SpecRunFailure::Refused(_) | SpecRunFailure::Tile(_),
            ) => return None,
            Err(SpecRunFailure::Clickhouse(err)) => {
                let _logged = upstream_error::report_ch(&upstream_error::TILE, &err);
                return None;
            }
        };
    }
    let columns = shown.get("columns")?.clone();
    let mut rows = shown.get("rows")?.as_array()?.clone();
    let truncated = rows.len() > cap;
    rows.truncate(cap);
    Some(json!({ "columns": columns, "rows": rows, "truncated": truncated, "limit": cap }))
}

/// `SELECT table, name, type FROM system.columns WHERE database='serving'`,
/// grouped into a mart → column-kinds map — used to decide which dashboard
/// filters apply to which tile, and whether their op fits the column. The
/// kind comes from the type, never from the name (BI-18).
pub(crate) async fn mart_columns(
    ch: &ChClient,
) -> Result<HashMap<String, RelationColumns>, ChError> {
    let rows = ch
        .rows(
            "SELECT table, name, type FROM system.columns WHERE database='serving'",
            None,
        )
        .await?;
    let mut m: HashMap<String, RelationColumns> = HashMap::new();
    for r in &rows {
        let table = r.get("table").and_then(Value::as_str).unwrap_or("");
        let name = r.get("name").and_then(Value::as_str).unwrap_or("");
        let ty = r.get("type").and_then(Value::as_str).unwrap_or("");
        m.entry(table.to_owned())
            .or_default()
            .insert(name.to_owned(), ColumnKind::from_clickhouse_type(ty));
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn extract_json_object_returns_the_outermost_braces() {
        assert_eq!(
            extract_json_object("Sure!\n```json\n{\"a\": {\"b\": 1}}\n```\nDone."),
            Some("{\"a\": {\"b\": 1}}")
        );
    }

    #[test]
    fn extract_json_object_is_none_without_a_pair_of_braces() {
        assert_eq!(extract_json_object("no json here"), None);
        assert_eq!(extract_json_object("} backwards {"), None);
        assert_eq!(extract_json_object(""), None);
    }

    fn point_spec(map: Option<&str>) -> store::ChartSpec {
        store::ChartSpec {
            id: "u_1".to_owned(),
            title: "T".to_owned(),
            subtitle: None,
            kind: lakehouse_bi::specs::ChartKind::Pointmap,
            mart: "mart_x".to_owned(),
            sql_source: None,
            sql: "SELECT 1".to_owned(),
            x: String::new(),
            y: lakehouse_bi::specs::ChartY::Single("visitors".to_owned()),
            series: None,
            map: map.map(str::to_owned),
            lat: Some("lat".to_owned()),
            lon: Some("lon".to_owned()),
            format: None,
            span: None,
            text: None,
            caption: None,
            target: None,
            click: None,
        }
    }

    /// The console draws a map chart from these three fields alone, so a
    /// hand-copied render that forgot one would show "map not available" for
    /// a chart that is stored correctly.
    #[test]
    fn a_rendered_map_chart_carries_its_map_and_coordinate_columns() {
        let rendered = render_stored_spec(&point_spec(Some("id-provinces")), ChartSource::Ui);
        assert_eq!(rendered["map"], "id-provinces");
        assert_eq!(rendered["lat"], "lat");
        assert_eq!(rendered["lon"], "lon");
        let no_map = render_stored_spec(&point_spec(None), ChartSource::Ui);
        assert!(no_map.get("map").is_none());
    }

    #[test]
    fn prettify_replaces_underscores_and_titlecases() {
        assert_eq!(prettify("mart_wisman"), "Mart Wisman");
    }

    #[test]
    fn prettify_handles_single_word() {
        assert_eq!(prettify("atlas"), "Atlas");
    }

    #[test]
    fn prettify_handles_leading_and_trailing_underscores() {
        assert_eq!(prettify("_foo_bar_"), " Foo Bar ");
    }

    #[test]
    fn js_string_converts_null_to_empty_string() {
        assert_eq!(js_string(Some(&Value::Null)), "");
        assert_eq!(js_string(None), "");
    }

    #[test]
    fn js_string_passes_through_plain_string() {
        assert_eq!(js_string(Some(&Value::String("2014".to_owned()))), "2014");
    }

    #[test]
    fn js_string_stringifies_number() {
        assert_eq!(js_string(Some(&serde_json::json!(476))), "476");
    }

    #[test]
    fn num_or_zero_defaults_on_missing_row() {
        assert_eq!(num_or_zero(None, "n"), 0);
    }

    #[test]
    fn num_or_zero_defaults_on_unparseable_value() {
        let mut m = Map::new();
        m.insert("n".to_owned(), Value::String("not-a-number".to_owned()));
        assert_eq!(num_or_zero(Some(&m), "n"), 0);
    }

    #[test]
    fn num_or_zero_parses_valid_string() {
        let mut m = Map::new();
        m.insert("n".to_owned(), Value::String("42".to_owned()));
        assert_eq!(num_or_zero(Some(&m), "n"), 42);
    }

    #[test]
    fn str_col_defaults_to_empty_string() {
        let m = Map::new();
        assert_eq!(str_col(&m, "missing"), "");
    }

    #[test]
    fn upstream_message_never_repeats_the_error_text() {
        let text = upstream_message("planted-marker-bad-sql");
        assert!(!text.contains("planted-marker-bad-sql"), "{text}");
        assert!(text.contains("Reference: "), "{text}");
    }
}

/// `BI-45` (plan T2, T4 and T5): the one sender of a statement that runs
/// under the caller's limits, the checked save, and the assistant's reply for
/// a save the limit ended. A wiremock `ClickHouse` only: no Postgres, so no
/// policy row applies and nothing reaches the chart store.
#[cfg(test)]
mod limited_run {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TIMEOUT_BODY: &str =
        "Code: 159. DB::Exception: planted-marker Timeout exceeded (TIMEOUT_EXCEEDED)";

    fn clickhouse(server: &MockServer) -> ChClient {
        ChClient::new(server.uri(), "default".to_owned(), String::new())
    }

    fn url_settings(request: &wiremock::Request) -> HashMap<String, String> {
        request.url.query_pairs().into_owned().collect()
    }

    /// `BI-45` plan D4 (QA `LH-BI-5-01`): one call sends the caller's time
    /// limit, the row cap plus one and a query id as URL settings, and an
    /// engine that stopped the statement at the limit comes back as the
    /// limit's sentence with its `limit` object, never the engine's text.
    #[tokio::test]
    async fn a_statement_run_under_limits_sends_the_time_limit_and_says_the_limit_sentence_when_the_engine_stops_it()
     {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string(TIMEOUT_BODY))
            .mount(&server)
            .await;
        let ch = clickhouse(&server);

        let Err(err) = run_statement_limited(&ch, "SELECT 1").await else {
            panic!("a statement the engine stopped is a failure");
        };

        let SpecRunFailure::Tile(answer) = err else {
            panic!("a limit the engine enforced is the tile's answer, not {err:?}");
        };
        // A test runs outside a request, so the limits are the defaults.
        assert_eq!(
            answer["error"], "This chart stopped after 60 seconds, the limit for this install.",
            "{answer}"
        );
        assert_eq!(answer["limit"]["kind"], "time", "{answer}");
        assert!(
            !answer.to_string().contains("planted-marker"),
            "the engine's text reached the answer: {answer}"
        );
        let requests = server.received_requests().await.unwrap();
        let settings = url_settings(&requests[0]);
        assert_eq!(settings["max_execution_time"], "60");
        assert_eq!(settings["max_result_rows"], "2001");
        assert!(settings.contains_key("query_id"), "{settings:?}");
    }

    /// A stored chart whose statement is `sql`, on `mart_x` (the shape
    /// `store::spec_from_input` returns, without reading a mart).
    fn stored_chart(sql: &str) -> store::StoredChartSpec {
        let def: store::ChartInput = serde_json::from_value(json!({
            "title": "Visitors",
            "kind": "bar",
            "mart": "mart_x",
            "dimension": "d",
            "measures": ["v"],
        }))
        .expect("a bar chart definition");
        store::StoredChartSpec {
            spec: store::ChartSpec {
                id: "u_1".to_owned(),
                title: "Visitors".to_owned(),
                subtitle: None,
                kind: lakehouse_bi::specs::ChartKind::Bar,
                mart: "mart_x".to_owned(),
                sql_source: None,
                sql: sql.to_owned(),
                x: "d".to_owned(),
                y: lakehouse_bi::specs::ChartY::Single("v".to_owned()),
                series: None,
                map: None,
                lat: None,
                lon: None,
                format: None,
                span: None,
                text: None,
                caption: None,
                target: None,
                click: None,
            },
            source: ChartSource::Ui,
            board: "b1".to_owned(),
            def,
            has_year: false,
            created_by: Some("ui".to_owned()),
            created_at: None,
        }
    }

    /// QA `LH-BI-5-01` (`BI-45`, plan D5): a chart whose statement the limit
    /// ended is not saved, and the failure is the limit's sentence — the same
    /// words Query Studio uses for its own limit.
    #[tokio::test]
    async fn a_chart_whose_statement_hits_the_time_limit_is_not_saved_and_the_failure_is_the_limit_sentence()
     {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string(TIMEOUT_BODY))
            .mount(&server)
            .await;
        let ch = clickhouse(&server);

        let Err(err) = insert_chart_checked(&ch, &stored_chart("SELECT 1 AS n")).await else {
            panic!("a statement the limit ended must not be saved");
        };

        let SpecRunFailure::Tile(answer) = err else {
            panic!("the limit's answer, not {err:?}");
        };
        assert_eq!(
            answer["error"], "This chart stopped after 60 seconds, the limit for this install.",
            "{answer}"
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| !String::from_utf8_lossy(&r.body).contains("INSERT INTO console.bi_chart")),
            "a chart the limit ended was written anyway"
        );
    }

    /// `BI-45` plan D5: a save runs its statement once, under the caller's
    /// limit, and only then writes the row.
    #[tokio::test]
    async fn a_chart_whose_statement_runs_is_saved_after_one_limited_run() {
        let server = MockServer::start().await;
        // The statement, the bootstrap's DDL and the `INSERT` all answer the
        // engine's "Ok."; what the test reads is the order and the settings.
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("Ok."))
            .mount(&server)
            .await;
        let ch = clickhouse(&server);

        insert_chart_checked(&ch, &stored_chart("SELECT 1 AS n"))
            .await
            .expect("a statement that runs is saved");

        let requests = server.received_requests().await.unwrap();
        let bodies: Vec<String> = requests
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect();
        let select = requests
            .iter()
            .position(|r| String::from_utf8_lossy(&r.body).contains("SELECT 1 AS n"))
            .expect("the chart's statement ran");
        assert_eq!(url_settings(&requests[select])["max_execution_time"], "60");
        assert_eq!(
            bodies
                .iter()
                .filter(|b| b.contains("INSERT INTO console.bi_chart"))
                .count(),
            1,
            "one row is written, once"
        );
        assert!(
            select
                < bodies
                    .iter()
                    .position(|b| b.contains("INSERT INTO console.bi_chart"))
                    .expect("the row was written"),
            "the statement runs before the row is written"
        );
    }

    /// `BI-45` plan D5/D6: the one mapping from a failed limited statement to
    /// a console answer — the limit's sentence as a 422, and an engine failure
    /// through the caller's own mapping (never its text).
    #[test]
    fn a_limit_failure_becomes_a_422_with_the_sentence_and_an_engine_failure_keeps_the_callers_mapping()
     {
        let limit = json!({
            "error": "This chart stopped after 5 seconds, the limit for your role.",
            "limit": {"kind": "time", "value": 5, "source": {"kind": "role", "name": "Analyst"}},
        });
        let err = failure_as_api_error(SpecRunFailure::Tile(limit), |_err| {
            lakehouse_core::ApiError::Internal("the engine's own answer".to_owned())
        });
        assert!(
            matches!(
                &err,
                lakehouse_core::ApiError::Unprocessable(message)
                    if message == "This chart stopped after 5 seconds, the limit for your role."
            ),
            "{err:?}"
        );
        // A tile answer without a sentence still says something fixed.
        let err = failure_as_api_error(SpecRunFailure::Tile(json!({ "limit": {} })), |_err| {
            lakehouse_core::ApiError::Internal("unused".to_owned())
        });
        assert!(
            matches!(
                &err,
                lakehouse_core::ApiError::Unprocessable(message) if message == "The query did not finish."
            ),
            "{err:?}"
        );
        let err = failure_as_api_error(
            SpecRunFailure::Refused("The query limits could not be read.".to_owned()),
            |_err| lakehouse_core::ApiError::Internal("unused".to_owned()),
        );
        assert!(
            matches!(
                &err,
                lakehouse_core::ApiError::Unprocessable(message)
                    if message == "The query limits could not be read."
            ),
            "{err:?}"
        );
        let err = failure_as_api_error(
            SpecRunFailure::Gated(lakehouse_core::ApiError::NotFound(
                "Gold mart 'x' not found in serving.".to_owned(),
            )),
            |_err| lakehouse_core::ApiError::Internal("unused".to_owned()),
        );
        assert!(
            matches!(&err, lakehouse_core::ApiError::NotFound(_)),
            "the gate keeps its own answer: {err:?}"
        );
        let err = failure_as_api_error(
            SpecRunFailure::Clickhouse(ChError::Server(
                "Code: 62. DB::Exception: planted-marker".to_owned(),
            )),
            |err| {
                upstream_error::ch_error_as(
                    &upstream_error::DATABASE,
                    err,
                    upstream_error::FailedAs::BadRequest,
                )
            },
        );
        assert!(
            !format!("{err:?}").contains("planted-marker"),
            "the engine's text reached the answer: {err:?}"
        );
        assert!(
            // The console's save keeps its own status for an engine failure:
            // `FailedAs::BadRequest`, with the fixed database sentence.
            matches!(
                &err,
                lakehouse_core::ApiError::BadRequest(message)
                    if message.starts_with("The database request failed.")
            ),
            "{err:?}"
        );
    }

    /// `BI-45` plan D5 (`LH-BI-5-01`): an assistant's save that the limit
    /// ended answers the tile's own sentence with its `limit`, so the model
    /// can say it; a refused rewrite and the gate keep their sentence; an
    /// engine failure is the fixed reply under a reference id and never the
    /// engine's text.
    #[test]
    fn a_tool_reply_for_a_save_the_limit_ended_is_the_sentence_with_its_limit() {
        let tile = json!({
            "error": "This chart stopped after 60 seconds, the limit for this install.",
            "limit": {"kind": "time", "value": 60, "source": {"kind": "install"}},
        });
        assert_eq!(
            failure_as_tool_reply(SpecRunFailure::Tile(tile.clone())),
            tile,
            "the tile's answer comes back as it is"
        );
        assert_eq!(
            failure_as_tool_reply(SpecRunFailure::Refused(
                "The query limits could not be read.".to_owned()
            )),
            json!({ "error": "The query limits could not be read." })
        );
        assert_eq!(
            failure_as_tool_reply(SpecRunFailure::Gated(lakehouse_core::ApiError::NotFound(
                "Gold mart 'x' not found in serving.".to_owned()
            ))),
            json!({ "error": "Gold mart 'x' not found in serving." })
        );
        let reply = failure_as_tool_reply(SpecRunFailure::Clickhouse(ChError::Server(
            "Code: 62. DB::Exception: planted-marker".to_owned(),
        )));
        assert!(
            reply["error"]
                .as_str()
                .is_some_and(|e| !e.contains("planted-marker")),
            "the engine's text reached the tool's reply: {reply}"
        );
        assert!(
            reply["error"]
                .as_str()
                .is_some_and(|e| !e.contains("planted-marker"))
                && reply["errorId"].as_str().is_some_and(|id| id.len() == 10),
            "the engine's text reached the tool's reply: {reply}"
        );
    }
}

// ── WS7 item D1: `run_spec_sql` goes through the same
//    `policy_engine::rewrite_sql_for_roles` Query Studio uses ──────────────
#[cfg(test)]
mod run_spec_sql_enforcement {
    //! Real Postgres (`#[sqlx::test]`, a real authored `policy` row) plus a
    //! wiremock `ClickHouse` — the exact same two-harness shape
    //! `routes::query`'s `enforcement` test module already established for
    //! WS7 item C2 (`run_rewrites_sql_for_a_governed_table_before_executing`).
    //! The WS7 plan names `fake_clickhouse_returning_columns`/
    //! `fake_policy_engine_obligations_masking` helpers that do not exist
    //! anywhere in this codebase — `PolicyEngineObligations` is a concrete
    //! struct over a real `PgPool`/`ChClient`, not a mockable trait, so a
    //! "fake" obligations source can only mean a real (ephemeral)
    //! Postgres seeded with a real policy row, which is what this module
    //! builds instead.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_clickhouse::ChClient;
    use lakehouse_store::PgPool;
    use lakehouse_store::governance::{self, CreatePolicyInput};
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::run_spec_sql;
    use crate::policy_engine::PolicyEngineObligations;
    use crate::sql_rewrite::PlaceholderValues;

    /// `SEC-50` slice 2 T1: the state the unrestricted gate is built from.
    ///
    /// The unrestricted gate reaches everything without a database, so a
    /// test that is about the rewrite and not about the tenant gate needs
    /// no deployment.
    fn gate_state() -> crate::state::AppState {
        crate::state::AppState::without_a_database()
    }

    /// Mounts the `system.columns` response `PolicyEngineObligations`
    /// needs to resolve `silver.customers`'s real column list, and the
    /// masked-projection query's own response — the ONLY two `ClickHouse`
    /// requests a masked `run_spec_sql` call makes.
    async fn mount_masked_customers_responses(server: &MockServer) {
        Mock::given(method("POST"))
            .and(body_string_contains("system.columns"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
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
            .and(body_string_contains("replaceRegexpOne"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
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

    /// Failing-test-first for WS7 item D1: before `run_spec_sql` grew its
    /// `roles`/`placeholders`/`obligations` parameters and its internal
    /// `policy_engine::rewrite_sql_for_roles` call, it took only
    /// `(ch, id, sql)` and sent `sql` to `ClickHouse` completely
    /// unrewritten. Quoted failure text (captured against the pre-D1
    /// signature, `cargo test -p lakehouse-api support::run_spec_sql
    /// 2>&1 | tail -40`): `error[E0061]: this function takes 3 arguments
    /// but 6 arguments were supplied` — the call this test makes does not
    /// even compile against the old signature, let alone mask `email`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn run_spec_sql_is_rewritten_through_sql_rewrite_enforce_for_the_given_roles(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        governance::create_policy(
            &pool,
            &CreatePolicyInput {
                name: "dashboard-tile-masking-test".to_owned(),
                kind: "Row filter".to_owned(),
                subjects: "Analyst".to_owned(),
                resources: "silver.customers".to_owned(),
                effect: "Permit with obligation".to_owned(),
                conditions: Some(
                    r#"{"roles":["Analyst"],"table":"silver.customers","mask":["email"]}"#
                        .to_owned(),
                ),
                activate: true,
                owner: None,
            },
        )
        .await
        .expect("seeding the governing policy must succeed");

        let server = MockServer::start().await;
        mount_masked_customers_responses(&server).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let obligations = PolicyEngineObligations::new(Some(&pool), &ch);

        let state = gate_state();
        let (_id, _val) = run_spec_sql(
            &ch,
            &crate::routes::query::StatementGate::unrestricted(&state),
            "chart1",
            "SELECT id, email FROM silver.customers",
            &["Analyst".to_owned()],
            &PlaceholderValues::none(),
            &obligations,
        )
        .await;

        let requests = server
            .received_requests()
            .await
            .expect("mock server records requests");
        assert!(
            requests
                .iter()
                .any(|r| String::from_utf8_lossy(&r.body)
                    .contains("replaceRegexpOne(toString(`email`)")),
            "expected ClickHouse to receive the masked/rewritten query, never the raw one"
        );
        Ok(())
    }

    /// `BI-4` part B T4 (feature decision 4, `BI-4-AC2`): a masked viewer and
    /// an unmasked one open the same tile one after the other, in both orders,
    /// through one shared memory. Each sees only their own values, because the
    /// masks are in the statement the key is made of.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_masked_and_an_unmasked_viewer_never_share_a_remembered_result(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        governance::create_policy(
            &pool,
            &CreatePolicyInput {
                name: "dashboard-tile-cache-masking-test".to_owned(),
                kind: "Row filter".to_owned(),
                subjects: "Analyst".to_owned(),
                resources: "silver.customers".to_owned(),
                effect: "Permit with obligation".to_owned(),
                conditions: Some(
                    r#"{"roles":["Analyst"],"table":"silver.customers","mask":["email"]}"#
                        .to_owned(),
                ),
                activate: true,
                owner: None,
            },
        )
        .await
        .expect("seeding the governing policy must succeed");
        let analyst = vec!["Analyst".to_owned()];
        let admin = vec!["Admin".to_owned()];
        // GOV-7 D4: a role name is resolved against `role` BY NAME, and a name
        // with no row contributes nothing (it is not "all"). This fixture
        // names a role the seed migrations do not create, so the fixture
        // creates it with the migration's default `read_scope = 'all'` — the
        // assertion below is unchanged and D4 is not loosened. Production
        // never sees this: every name in `principal.role_names` came from a
        // `role` row.
        sqlx::query("INSERT INTO role (name, permissions) VALUES ('Admin', '') ON CONFLICT (name) DO NOTHING")
            .execute(&pool)
            .await?;
        for order in [[&analyst, &admin], [&admin, &analyst]] {
            let server = MockServer::start().await;
            // Nothing in `system.parts`: the table is not watched, which is
            // enough for the entry to be stored and found again.
            Mock::given(method("POST"))
                .and(body_string_contains("system.parts"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "meta": [], "data": [], "rows": 0,
                })))
                .mount(&server)
                .await;
            mount_masked_customers_responses(&server).await;
            Mock::given(method("POST"))
                .and(body_string_contains("silver.customers"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "meta": [
                        {"name": "id", "type": "UInt64"},
                        {"name": "email", "type": "String"},
                    ],
                    "data": [{"id": "1", "email": "ann@example.test"}],
                    "rows": 1,
                })))
                .mount(&server)
                .await;
            let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
            let obligations = PolicyEngineObligations::new(Some(&pool), &ch);
            let cache = std::sync::Arc::new(crate::result_cache::ResultCache::new(8));
            // Each viewer twice: the second read of a viewer is the remembered
            // one and must still be that viewer's own.
            for (i, roles) in [order[0], order[1], order[0], order[1]]
                .into_iter()
                .enumerate()
            {
                let scope = crate::result_cache::CacheScope::new(
                    std::sync::Arc::clone(&cache),
                    lakehouse_bi::cache_rule::CacheRule::Follow,
                    crate::result_cache::Fresh::No,
                );
                let (_, val) = crate::result_cache::scoped(
                    scope,
                    run_spec_sql(
                        &ch,
                        &crate::routes::query::StatementGate::unrestricted(&gate_state()),
                        "chart1",
                        "SELECT id, email FROM silver.customers",
                        roles,
                        &PlaceholderValues::none(),
                        &obligations,
                    ),
                )
                .await;
                let expected = if roles == &analyst {
                    "***"
                } else {
                    "ann@example.test"
                };
                assert_eq!(val["rows"][0]["email"], expected, "{roles:?}");
                // BI-4 B review P-S3: the first read of each viewer computes,
                // the second is the remembered one. Without this the test
                // would pass with the cache switched off.
                assert_eq!(
                    val["remembered"],
                    serde_json::json!(i >= 2),
                    "read {i}: {roles:?}"
                );
            }
            // Two statements (one per viewer), each asked once.
            let requests = server.received_requests().await.expect("recorded");
            let asked = requests
                .iter()
                .filter(|r| !String::from_utf8_lossy(&r.body).contains("system."))
                .count();
            assert_eq!(asked, 2, "one engine request per viewer, not per read");
        }
        Ok(())
    }

    /// SEC-11: a tile whose `ClickHouse` query fails shows a fixed message
    /// and a reference id, never the database's own text (which names
    /// tables and carries the server version, and reaches public and embed
    /// links that have no sign-in).
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_failing_tile_query_reports_a_fixed_message_and_a_reference(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string(
                "Code: 999. DB::Exception: planted-marker-table-x (version 0.0.0)",
            ))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let obligations = PolicyEngineObligations::new(Some(&pool), &ch);

        let state = gate_state();
        let (id, val) = run_spec_sql(
            &ch,
            &crate::routes::query::StatementGate::unrestricted(&state),
            "chart1",
            "SELECT 1 AS n",
            &[],
            &PlaceholderValues::none(),
            &obligations,
        )
        .await;

        assert_eq!(id, "chart1");
        assert_eq!(val["error"], "This chart could not be loaded.");
        assert!(
            val["errorId"].as_str().is_some_and(|r| r.len() == 10),
            "{val:?}"
        );
        assert!(
            !val.to_string().contains("planted-marker"),
            "the database's text reached the tile: {val:?}"
        );
        Ok(())
    }

    /// BI-5 (plan D6): the engine stopping a tile at the time limit (code 159)
    /// or a Stop (code 394) is a fixed sentence, with the limit for the first,
    /// and never the engine's text.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_tile_the_time_limit_or_a_stop_ended_says_so_without_the_engines_text(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        for (body, expected_error, has_limit) in [
            (
                "Code: 159. DB::Exception: planted-marker Timeout exceeded (TIMEOUT_EXCEEDED)",
                "This chart stopped after 60 seconds, the limit for this install.",
                true,
            ),
            (
                "Code: 394. DB::Exception: planted-marker Query was cancelled (QUERY_WAS_CANCELLED)",
                "Stopped",
                false,
            ),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(500).set_body_string(body))
                .mount(&server)
                .await;
            let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
            let obligations = PolicyEngineObligations::new(Some(&pool), &ch);
            let state = gate_state();
            let (_, val) = run_spec_sql(
                &ch,
                &crate::routes::query::StatementGate::unrestricted(&state),
                "c",
                "SELECT 1 AS n",
                &[],
                &PlaceholderValues::none(),
                &obligations,
            )
            .await;
            assert_eq!(val["error"], expected_error);
            assert_eq!(
                val["limit"],
                if has_limit {
                    serde_json::json!({"kind": "time", "value": 60, "source": {"kind": "install"}})
                } else {
                    serde_json::Value::Null
                }
            );
            assert!(!val.to_string().contains("planted-marker"), "{val}");
        }
        Ok(())
    }

    /// BI-5 (plan D3): a tile query carries the time limit, the row cap plus
    /// one, a query id and the cancel-on-close setting as URL settings, and
    /// the SQL source's cap in the text follows the same numbers.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_tile_query_sends_its_limits_and_its_id_as_url_settings(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"meta":[{"name":"n","type":"UInt8"}],"data":[{"n":1}],"rows":1}"#,
            ))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let obligations = PolicyEngineObligations::new(Some(&pool), &ch);
        let sql = format!(
            "SELECT 1 AS n{}",
            lakehouse_bi::builder::SQL_SOURCE_SETTINGS
        );
        let state = gate_state();
        let (_, val) = run_spec_sql(
            &ch,
            &crate::routes::query::StatementGate::unrestricted(&state),
            "c",
            &sql,
            &[],
            &PlaceholderValues::none(),
            &obligations,
        )
        .await;
        assert_eq!(val["rows"].as_array().map(Vec::len), Some(1));
        let requests = server.received_requests().await.unwrap();
        let url = requests[0].url.clone();
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q["max_execution_time"], "60");
        assert_eq!(q["max_result_rows"], "2001");
        assert_eq!(q["result_overflow_mode"], "break");
        assert_eq!(q["cancel_http_readonly_queries_on_client_close"], "1");
        assert!(
            q["query_id"].starts_with(crate::query_limits::ANONYMOUS_PRINCIPAL),
            "a detached run names the nil principal: {}",
            q["query_id"]
        );
        let body = String::from_utf8(requests[0].body.clone()).unwrap();
        assert!(
            body.contains("max_result_rows = 2001") && body.contains("max_execution_time = 60"),
            "{body}"
        );
        Ok(())
    }

    /// `BI-5` review B1: a point map reads up to the map limit and a pivot up to
    /// its own cap, with no row-cut marker, while a chart of any other kind is
    /// still cut at the chart row limit.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_point_map_and_a_pivot_read_past_the_chart_row_limit_and_a_bar_does_not(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        use lakehouse_bi::specs::ChartKind;
        let many = |n: usize| {
            let data: Vec<_> = (0..n).map(|i| serde_json::json!({ "n": i })).collect();
            serde_json::json!({ "meta": [{"name": "n", "type": "UInt32"}], "data": data, "rows": n })
        };
        for (kind, sql, rows, expect_cap, cut) in [
            (
                ChartKind::Pointmap,
                "SELECT n FROM serving.m LIMIT 5000",
                3000,
                "5001",
                false,
            ),
            (
                ChartKind::Pivot,
                "SELECT n FROM serving.m LIMIT 10001",
                10001,
                "10001",
                false,
            ),
            (
                ChartKind::Bar,
                "SELECT n FROM serving.m LIMIT 5000",
                3000,
                "2001",
                true,
            ),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_json(many(rows)))
                .mount(&server)
                .await;
            let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
            let obligations = PolicyEngineObligations::new(Some(&pool), &ch);
            let state = gate_state();
            let (_, val) = super::with_tile_rows(
                kind,
                sql,
                run_spec_sql(
                    &ch,
                    &crate::routes::query::StatementGate::unrestricted(&state),
                    "c",
                    sql,
                    &[],
                    &PlaceholderValues::none(),
                    &obligations,
                ),
            )
            .await;
            let requests = server.received_requests().await.unwrap();
            let run = requests
                .iter()
                .find(|r| r.url.query_pairs().any(|(k, _)| k == "query_id"))
                .expect("the tile query was sent");
            let q: std::collections::HashMap<_, _> = run.url.query_pairs().into_owned().collect();
            assert_eq!(q["max_result_rows"], expect_cap, "{kind:?}");
            assert_eq!(val.get("limitHit").is_some(), cut, "{kind:?}: {val}");
            let kept = val["rows"].as_array().map(Vec::len);
            assert_eq!(kept, Some(if cut { 2000 } else { rows }), "{kind:?}");
        }
        Ok(())
    }

    /// `BI-5` review S2: a tile that has not sent its query when Stop is pressed
    /// on its run answers "Stopped" and asks the engine nothing.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_tile_of_a_stopped_run_sends_no_query(pool: PgPool) -> sqlx::Result<()> {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"meta":[{"name":"n","type":"UInt8"}],"data":[{"n":1}],"rows":1}"#,
            ))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let obligations = PolicyEngineObligations::new(Some(&pool), &ch);
        let run = crate::query_limits::QueryRun::detached();
        let id = uuid::Uuid::parse_str(run.run_id()).unwrap();
        let none = PlaceholderValues::none();
        let state = gate_state();
        let gate = crate::routes::query::StatementGate::unrestricted(&state);
        crate::query_limits::mark_stopped(uuid::Uuid::nil(), id);
        let (_, val) = crate::query_limits::scope(
            run,
            run_spec_sql(&ch, &gate, "c", "SELECT 1 AS n", &[], &none, &obligations),
        )
        .await;
        assert_eq!(val["error"], "Stopped");
        assert!(server.received_requests().await.unwrap().is_empty());
        // Another run of the same person is not affected.
        let (_, other) = crate::query_limits::scope(
            crate::query_limits::QueryRun::detached(),
            run_spec_sql(&ch, &gate, "c", "SELECT 1 AS n", &[], &none, &obligations),
        )
        .await;
        assert_eq!(other["rows"].as_array().map(Vec::len), Some(1));
        Ok(())
    }

    /// SEC-11: an unreachable `ClickHouse` is "unavailable", not "failed"
    /// (the wording and the status differ).
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_unreachable_database_reports_the_unavailable_message(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let ch = ChClient::new(
            "http://127.0.0.1:1".to_owned(),
            "default".to_owned(),
            String::new(),
        );
        let obligations = PolicyEngineObligations::new(Some(&pool), &ch);
        let state = gate_state();
        let (_id, val) = run_spec_sql(
            &ch,
            &crate::routes::query::StatementGate::unrestricted(&state),
            "chart1",
            "SELECT 1 AS n",
            &[],
            &PlaceholderValues::none(),
            &obligations,
        )
        .await;
        assert_eq!(val["error"], "The database is unavailable.");
        assert!(val["errorId"].is_string(), "{val:?}");
        Ok(())
    }

    /// Failing-test-first for WS7 item D1's other half (Hard Requirement
    /// 2): before this task, `run_spec_sql` had no rewrite/refusal step at
    /// all, so a table-function chart SQL would have reached `ClickHouse`
    /// unrefused. Quoted failure text (same pre-D1 signature): `error[E0061]:
    /// this function takes 3 arguments but 6 arguments were supplied`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn run_spec_sql_refuses_and_never_executes_when_enforce_refuses(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        // No mocks mounted at all — any request that reaches ClickHouse
        // fails loudly (connection/404), which would itself prove the
        // refusal happened too late.
        let server = MockServer::start().await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let obligations = PolicyEngineObligations::new(Some(&pool), &ch);

        let state = gate_state();
        let (_id, val) = run_spec_sql(
            &ch,
            &crate::routes::query::StatementGate::unrestricted(&state),
            "chart1",
            "SELECT * FROM url('h', 'CSV')",
            &[],
            &PlaceholderValues::none(),
            &obligations,
        )
        .await;

        assert!(val.get("error").is_some(), "expected a refusal: {val:?}");
        let requests = server
            .received_requests()
            .await
            .expect("mock server records requests");
        assert!(
            requests.is_empty(),
            "a refused chart SQL must never reach ClickHouse: {requests:?}"
        );
        Ok(())
    }
}

/// `BI-16` part C3 (option B): a refused `RegionOf` lookup is told in English,
/// naming the function; any other refusal keeps its fixed message.
#[cfg(test)]
mod region_of_refusal {
    use super::*;
    use crate::policy_engine::EnforcementError;
    use crate::sql_rewrite::RewriteError;

    fn denied() -> EnforcementError {
        EnforcementError::Rewrite(RewriteError::DictOrJoinFunctionDenied {
            name: "dictGetOrDefault".to_owned(),
        })
    }

    #[test]
    fn a_denied_region_lookup_names_region_of_and_other_dictionary_calls_keep_the_fixed_message() {
        let lookup = "SELECT dictGetOrDefault('console.geo_dict_m_0123456789ab', 'name', (lon, lat), '') FROM t";
        assert_eq!(
            enforcement_message_for(lookup, &denied()),
            REGION_OF_REFUSED
        );
        assert!(REGION_OF_REFUSED.starts_with("RegionOf is not available to you"));
        let other = "SELECT dictGet('crm.people', 'email', id) FROM t";
        assert_eq!(
            enforcement_message_for(other, &denied()),
            crate::policy_engine::enforcement_error_message(&denied())
        );
        let unparseable = EnforcementError::Rewrite(RewriteError::Unparseable);
        assert_eq!(
            enforcement_message_for(lookup, &unparseable),
            crate::policy_engine::enforcement_error_message(&unparseable)
        );
    }
}
