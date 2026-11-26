//! Read-only lakehouse-data tools: `run_sql`, `list_datasets`,
//! `describe_dataset`, `get_lineage`, `get_quality`, `describe_mart`.
//! Moved out of `ai.rs` unchanged (T0.1 registry refactor).

use std::fmt::Write as _;

use lakehouse_auth::Principal;
use lakehouse_clickhouse::ChClient;
use lakehouse_core::ApiError;
use lakehouse_core::ident::SqlLiteral;
use serde_json::{Map, Value, json};

use super::{api_result_to_value, arg_str};
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::catalog::{NOT_IN_PRIMARY, apply_serving_certification};
use crate::routes::support::{is_numeric_type, strip_non_ident};
use crate::state::AppState;

// `pub(in crate::routes)` so `routes::lineage`, which sits outside
// `routes::ai`, runs this SAME union query rather than re-deriving an
// equivalent string that could drift from this one.
pub(in crate::routes) const CATALOG_UNION: &str = "(SELECT slug,title,description,tier,table_name FROM lake.`bronze_meta.dataset_catalog` \
     UNION ALL SELECT slug,title,description,tier,table_name FROM lake.`bronze_meta_sec.dataset_catalog`)";

/// WS7 item C3: delegates to the SAME run (`routes::query::run_statement`,
/// the end of `routes::query::run`) Query Studio and saved queries already
/// call — closing the divergent-guard gap this task's own name cites
/// (`WS7 plan §0 item 7`): before this change,
/// `run_sql` called `ch.query` directly, so `sql_rewrite::enforce`'s
/// masking/row-filter/table-function/sensitive-table/view refusal (WS7
/// items B3-B6, wired into `routes::query::run` by WS7 item C2) never ran
/// for a copilot-issued query at all, even though the exact same
/// `Extension(principal)` reached this function. See
/// `tools::queries::run_saved_query`'s doc comment for why "call the real
/// handler, read its JSON back" is the preferred reuse shape over
/// re-implementing the guard a second time — this function now follows
/// that exact same shape.
///
/// `is_read_only_sql`/`dry_run_sql` below stay in place as an ADDITIONAL,
/// cheaper first filter (WS7 item F4) — never a substitute for
/// `routes::query::run`'s own `is_read_only`/`sql_rewrite::enforce`,
/// which still run on every call reaching this function, principal
/// present or not.
pub(super) async fn run_sql(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    // `query-parameters.md` decision 7: the model's `{{name}}` placeholders are filled FIRST, with
    // the function the run route uses (`ClickHouse`, this tool's only engine),
    // so every check below, the policy rewrite and the run read the final
    // statement. A refusal is the tool's error: the model reads the sentence
    // and asks the person. This is the one scan of what the model wrote; the
    // run at the end is of the final statement and never scans again.
    let sql = match crate::routes::query::fill_tool_parameters(
        &arg_str(args, "sql"),
        args.get("parameters").cloned(),
        None,
    ) {
        Ok(filled) => filled,
        Err(refusal) => return json!({ "error": refusal.to_string() }),
    };
    if !is_read_only_sql(&sql) {
        return json!({ "error": "Only a read-only SELECT is allowed." });
    }
    // Same fail-closed shape `tools::queries::run_saved_query` uses for a
    // headless (schedule/service-token) caller with no interactive
    // principal to run as — `routes::query::run` 401s with none, for
    // every engine, so there is no honest value to forward here either.
    // Checked BEFORE `dry_run_sql` below (a real `ClickHouse` round trip)
    // so a headless caller is refused without spending that call at all.
    let Some(principal) = principal else {
        return json!({
            "error": "running SQL needs a signed-in user; this call came from a schedule, \
                       which has no user to run as",
        });
    };
    // `BI-44` slice 3: a `bi-only` install that runs `Trino` must never dial
    // `ClickHouse` — it was not deployed. Skip the `EXPLAIN AST` dry run below
    // (a `ClickHouse` round trip) and use the SAME engine-aware choke point
    // `POST /api/query/run` uses: `run_statement` applies `is_trino_safe`
    // and the `Engine::Trino` governance rewrite before it dials `Trino`.
    // The full stack, and a `bi-only` install on `ClickHouse`, keep the
    // byte-for-byte path below.
    if state.config.is_bi_only() && state.config.query_engine == crate::config::QueryEngine::Trino {
        match crate::sql_rewrite::classify_statement(
            &sql,
            &crate::sql_rewrite::TrinoDialect::default(),
        ) {
            Err(crate::sql_rewrite::RewriteError::Unparseable) | Ok(()) => {}
            Err(refusal) => {
                let message = crate::policy_engine::refusal_message(&refusal).to_owned();
                let refused: ApiResult<ApiJson<Value>> =
                    Err(ApiError::Unprocessable(message).into());
                return api_result_to_value(refused).await;
            }
        }
        if !crate::sql_guard::is_trino_safe(&sql) {
            return json!({
                "error": "This statement is not permitted against Trino (write, session, or \
                           procedure keyword detected).",
            });
        }
        let (result, browser) = match crate::routes::query::run_statement(
            state,
            Some(principal),
            &axum::http::HeaderMap::new(),
            &sql,
            "trino",
        )
        .await
        {
            Ok(ok) => (api_result_to_value(Ok(ok)).await, None),
            Err(failure) => {
                let failed: ApiResult<ApiJson<Value>> = Err(failure.response.into());
                (api_result_to_value(failed).await, Some(failure.browser))
            }
        };
        return compact_query_result(result, browser);
    }
    // SEC-9: the policy refusals that need no database (a table function, a
    // refused system table) come BEFORE the engine dry run below. `EXPLAIN
    // AST` parses and never fetches, but it still sends the caller's
    // statement to the engine, and the engine's query log would then hold
    // `url('http://…')` for a statement the policy refuses. This is
    // `sql_rewrite::classify_statement`, the first step `sql_rewrite::enforce`
    // itself runs with the same (empty) permissions, so the rule is not
    // copied: it is synchronous, so it costs no round trip. The full
    // rewrite (obligations, views) still runs once, in `routes::query::run`.
    // `Unparseable` is left to the dry run, whose classified engine error is
    // what an author sees today for a statement the parser cannot read.
    match crate::sql_rewrite::classify_statement(&sql, &sqlparser::dialect::ClickHouseDialect {}) {
        Err(crate::sql_rewrite::RewriteError::Unparseable) | Ok(()) => {}
        Err(refusal) => {
            // The same 422 body `routes::query::run` would have produced.
            let message = crate::policy_engine::refusal_message(&refusal).to_owned();
            let refused: ApiResult<ApiJson<Value>> = Err(ApiError::Unprocessable(message).into());
            return api_result_to_value(refused).await;
        }
    }
    // WS7 item F4: `is_read_only_sql` above is a regex-shaped first
    // filter (unchanged, still checked first, defense-in-depth) — it can
    // disagree with what `ClickHouse` itself will actually execute for a
    // dialect construct the regex never anticipated. `dry_run_sql` is an
    // ADDITIONAL, engine-verified check: it asks `ClickHouse`'s own
    // `EXPLAIN AST` what KIND of statement this really is (never executed
    // by `EXPLAIN AST` itself) and refuses whenever the engine's own
    // answer is not a `SELECT`, closing the gap where a model-composed
    // SQL string could smuggle a non-`SELECT` construct past the regex.
    // The role rewrite comes next, still before the engine sees the statement:
    // the dry run then asks the engine about the statement the run will really
    // send (rewritten for the caller's current roles, with the run route's own
    // function), and a statement the rewrite refuses is refused here with the
    // run's status and fixed message and never sent. `run_statement`
    // rewrites once more from the final, unrewritten statement, so the two
    // never rewrite an already-rewritten statement.
    let rewritten =
        match crate::routes::query::rewrite_sql_for_principal(state, &sql, "clickhouse", principal)
            .await
        {
            Ok(rewritten) => rewritten,
            Err(refusal) => {
                let refused: ApiResult<ApiJson<Value>> = Err(refusal.into());
                return api_result_to_value(refused).await;
            }
        };
    let mut dry_args = args.clone();
    dry_args.insert("sql".to_owned(), Value::String(rewritten));
    let dry_run = dry_run_sql(&state.clickhouse, &dry_args).await;
    if dry_run.get("error").is_some() {
        return dry_run;
    }
    // The final statement, so `run_statement` and not `run`: `run` would scan
    // it again and refuse a text value that reads `'{{x}}'`.
    //
    // LH-QS-2-08 (owner 2026-11-12): a failure comes in two copies — the
    // model's copy (the engine's trimmed diagnosis, so the model can
    // correct its SQL) and the browser's copy (a fixed sentence and the same
    // reference). The model's copy goes in `error`, as it always did; the
    // browser's copy goes next to it, in the one key only the server reads.
    let (result, browser) = match crate::routes::query::run_statement(
        state,
        Some(principal),
        &axum::http::HeaderMap::new(),
        &sql,
        "clickhouse",
    )
    .await
    {
        Ok(ok) => (api_result_to_value(Ok(ok)).await, None),
        Err(failure) => {
            let failed: ApiResult<ApiJson<Value>> = Err(failure.response.into());
            (api_result_to_value(failed).await, Some(failure.browser))
        }
    };
    compact_query_result(result, browser)
}

/// Rows a `run_sql` result hands the model.
const MAX_RESULT_ROWS: usize = 100;

/// `routes::query::run`'s response, reduced to what the model needs.
///
/// The full response carries an id, engine metrics and a plan the model
/// never uses, and a 700-row result was cut at 8,000 characters in the
/// middle of a JSON object, so the model read a broken fragment as data.
/// Now the rows are capped at [`MAX_RESULT_ROWS`] with an explicit note
/// saying how many more there were and what to do instead, and an error
/// keeps its message but loses the engine boilerplate
/// ([`clean_sql_error`]).
///
/// A result the run route cut carries a second note, under `truncated`,
/// that names the limit which cut it — the route names it in `cutBy` — so
/// the model is never told the wrong limit did it.
///
/// `browser` is the browser's copy of a failure (owner 2026-11-12,
/// LH-QS-2-08): a fixed sentence and the reference, which the chat loop
/// shows to the person instead of the engine's text. It travels next to the
/// error, in the one key only the server reads
/// (`routes::ai::BROWSER_ERROR_KEY`); the model's copy stays in `error`.
///
/// `DATA-29`: the cells are handed over as the route built them, so a
/// `NULL` cell reaches the model as `null` and an empty cell as `""` — the
/// two are told apart, and this tool does not flatten them again on the way
/// to the model.
fn compact_query_result(result: Value, browser: Option<String>) -> Value {
    let Value::Object(mut body) = result else {
        return result;
    };
    if let Some(Value::String(err)) = body.get("error") {
        let mut compact = json!({
            "error": clean_sql_error(err),
            "hint": "Check table and column names against the DATA MAP and fix the query.",
        });
        if let Some(browser) = browser {
            compact = crate::routes::ai::with_browser_error(compact, &browser);
        }
        return compact;
    }
    let rows = match body.remove("rows") {
        Some(Value::Array(rows)) => rows,
        _ => Vec::new(),
    };
    let total = rows.len();
    let shown: Vec<Value> = rows.into_iter().take(MAX_RESULT_ROWS).collect();
    let mut out = Map::new();
    out.insert(
        "columns".to_owned(),
        body.remove("columns").unwrap_or(Value::Null),
    );
    out.insert("rows".to_owned(), Value::Array(shown));
    out.insert("rowCount".to_owned(), json!(total));
    if total > MAX_RESULT_ROWS {
        out.insert(
            "note".to_owned(),
            json!(format!(
                "This display is cut off: only the first {MAX_RESULT_ROWS} of the {total} rows \
                 the query returned are shown. The data itself is complete; do not describe it \
                 as partial or missing. To report totals, rankings or per-period figures, run a \
                 new query that aggregates in SQL (GROUP BY, ORDER BY … LIMIT) instead of \
                 adding up these rows."
            )),
        );
    }
    if body.get("truncated") == Some(&Value::Bool(true)) {
        // The run route names the limit that cut the result (`cutBy`), and
        // the note has to name that same limit: a result cut by the byte
        // cap was not cut by a row limit, and the row limit is the
        // server's own, not the engine's. A route that named no limit
        // leaves the note naming none, rather than guessing one.
        let note = match body.get("cutBy").and_then(Value::as_str) {
            Some("rows") => ROW_LIMIT_NOTE,
            Some("bytes") => BYTE_CAP_NOTE,
            _ => CUT_NOTE,
        };
        out.insert("truncated".to_owned(), json!(note));
    }
    Value::Object(out)
}

/// The note for a result the Query Studio row limit cut. The limit is the
/// server's own, so the note says so instead of blaming the engine.
const ROW_LIMIT_NOTE: &str =
    "the Query Studio row limit (64,000 rows by default) cut this result; aggregate in SQL";

/// The note for a result the server-side byte cap cut: a result of few,
/// wide rows passes the row limit and still reaches the cap, so the note
/// must not point at rows.
const BYTE_CAP_NOTE: &str = "the 10 MB result size cap cut this result; aggregate in SQL";

/// The note for a cut result whose limit the route did not name: the model
/// is told the result was cut and what to do, but not which limit did it.
const CUT_NOTE: &str = "this result was cut; aggregate in SQL";

/// A `ClickHouse` error for the query's own author: the message and its
/// error name, without the `Code: N. DB::Exception:` prefix, the server
/// version suffix, or any stack trace, capped in length. The model needs
/// the message to fix its SQL; it never needs the rest.
pub(super) fn clean_sql_error(err: &str) -> String {
    let first_line = err.lines().next().unwrap_or("");
    let body = first_line
        .find("DB::Exception:")
        .map_or(first_line, |i| &first_line[i + "DB::Exception:".len()..])
        .trim();
    let body = body.find(" (version ").map_or(body, |i| &body[..i]).trim();
    body.chars().take(400).collect()
}

/// `/^\s*(with|select)\b/i.test(sql) && !/\b(insert|alter|drop|delete|update|create|truncate|rename|attach|detach|grant|revoke)\b/i.test(sql)`
/// — the cheap first filter [`run_sql`] applies to the model's SQL. Distinct
/// from `query/run`'s guard (`routes::query::is_read_only`): only
/// `with`/`select` are allowed here (no `show`/`describe`/`explain`).
///
/// This filter reads the raw text: it does not lex SQL and knows no
/// engine's quoting or comment rules, so it cannot be moved onto the
/// shared lexer (`crate::sql_lex`) without changing what it refuses. That
/// is why it is stricter than the shared guard and never weaker — it
/// refuses a statement whose text holds a denied word anywhere, inside a
/// quoted run included (`SELECT 'drop' AS a` is refused here and allowed
/// by the shared guard), so it can refuse more than the shared guard,
/// never less. It stays the cheap first filter (`WS7 item F4`), and the
/// authoritative guard behind it — `routes::query::run`'s `is_read_only`,
/// which reads the statement with the rules of the engine the run names —
/// decides whether the statement runs.
fn is_read_only_sql(sql: &str) -> bool {
    starts_with_with_or_select(sql) && !contains_denied_keyword_full(sql)
}

fn starts_with_with_or_select(sql: &str) -> bool {
    let trimmed = sql.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    ["with", "select"].iter().any(|kw| {
        lower
            .strip_prefix(kw)
            .is_some_and(|rest| rest.chars().next().is_none_or(|c| !is_word_char(c)))
    })
}

fn contains_denied_keyword_full(sql: &str) -> bool {
    const DENIED: [&str; 12] = [
        "insert", "alter", "drop", "delete", "update", "create", "truncate", "rename", "attach",
        "detach", "grant", "revoke",
    ];
    word_occurs_any(sql, &DENIED)
}

fn word_occurs_any(sql: &str, denied: &[&str]) -> bool {
    let lower = sql.to_ascii_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    denied.iter().any(|kw| word_occurs(&chars, kw))
}

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

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Dry-runs `args["sql"]` via `ClickHouse`'s own `EXPLAIN AST`, refusing
/// whenever the engine's OWN reported statement kind is not a `SELECT`
/// (WS7 plan §0 item 6, verified live against `lake-clickhouse` during
/// planning: `EXPLAIN AST` never executes the statement, and its root
/// node's label names the real statement kind — `SelectWithUnionQuery`
/// for every `SELECT` form including `WITH`/`UNION`, `InsertQuery` for an
/// `INSERT`, `AlterQuery` for an `ALTER`, `DropQuery` for a `DROP`, …).
///
/// This is an ADDITIONAL, engine-verified check ahead of
/// [`is_read_only_sql`]'s regex-based guard
/// (unchanged, still checked first by [`run_sql`] itself) — never a
/// replacement for it: a third-party parser's dialect (or a hand-rolled
/// regex) can disagree with what `ClickHouse` will actually execute, so
/// this asks the engine itself rather than guessing. Returns
/// `{"error": "..."}` (naming the REAL reported statement kind, never
/// echoing the user's raw SQL back, per `AGENTS.md` rule 4) when refused,
/// or `{}` when the statement is confirmed a `SELECT`.
pub(super) async fn dry_run_sql(ch: &ChClient, args: &Map<String, Value>) -> Value {
    let sql = arg_str(args, "sql");
    // `raw_bytes`, not `rows`: `ChClient::rows` appends `\nFORMAT JSON`,
    // and `EXPLAIN AST … FORMAT JSON` does NOT answer in JSON —
    // `ClickHouse` parses the FORMAT clause as part of the statement being
    // explained (it appears in the dumped AST as `Identifier JSON`) and
    // still replies in the default text format. The JSON envelope
    // `rows` expects was therefore never there: the row set came back
    // EMPTY, `first_line` was "", and EVERY statement — `SELECT` included
    // — was refused as "melaporkan unknown", so the `run_sql` tool could
    // not succeed at all. Verified live against `ClickHouse` 26.7.
    // `raw_bytes` sends the statement verbatim and hands back the text
    // body `EXPLAIN` actually produces.
    let body = match ch.raw_bytes(&format!("EXPLAIN AST {sql}"), None).await {
        Ok(b) => b,
        Err(err) => {
            // SEC-11: ClickHouse's parse message echoes the statement and the
            // server version; the assistant gets a fixed message and a
            // reference, the log gets the text.
            return crate::upstream_error::report_ch(&crate::upstream_error::DATABASE, &err)
                .to_json();
        }
    };
    let text = String::from_utf8_lossy(&body);
    let first_line = text.lines().next().unwrap_or("").trim().to_owned();
    if first_line.starts_with("SelectWithUnionQuery") {
        return json!({});
    }
    let kind = first_line.split_whitespace().next().unwrap_or("unknown");
    json!({ "error": format!("only SELECT is allowed (EXPLAIN AST reported {kind})") })
}

/// `primer`/`sekunder` names where a dataset comes from, not a layer. The
/// tools used to hand the raw `tier` word to the model, which then
/// described the lakehouse as "primer" and "sekunder" data instead of
/// Bronze/Silver/Gold; they now say what it means.
fn source_kind(tier: &str) -> &'static str {
    match tier {
        "primer" => "primary source",
        "sekunder" => "secondary source",
        _ => "not recorded",
    }
}

/// `"primary"`/`"secondary"` (or the stored words) to the stored tier.
fn tier_filter(raw: &str) -> &str {
    match raw {
        "primary" | "primary source" | "primer" => "primer",
        "secondary" | "secondary source" | "sekunder" => "sekunder",
        _ => "",
    }
}

/// The refusal of a dataset reader called with no principal (a scheduled run).
/// `pub(super)`: the governance and dashboard readers that list tables refuse
/// a headless run with the same sentence (`SEC-76`).
pub(super) const DATASETS_NEED_A_USER: &str = "reading datasets needs a signed-in user";

/// `SEC-20`, plan 97: the tenant reach a reader of the dataset registry
/// narrows its rows by.
///
/// The registry is shared, and an upload another tenant made is a registry
/// row whose `bronze.<table_name>` that tenant claimed. A caller in the
/// catalog-owner tenant passes [`shared_catalog_refusal`] and a role's table
/// access ([`dataset_visible`]) and would still read the row's slug, title,
/// description and columns. The answer to "may this caller read that table" is
/// [`crate::routes::query::reachable_names`], the one rule the catalog routes
/// and the maintenance readers use, so it is asked here and never copied
/// (`AGENTS.md` rule 4; ADR 0018 moves these rules later and they must change
/// in one place).
#[derive(Clone, Copy)]
pub(crate) struct RegistryReach<'a> {
    state: &'a AppState,
    reach: crate::routes::query::TenantReach,
}

impl<'a> RegistryReach<'a> {
    /// The reach of `principal`, resolved as the catalog routes resolve it.
    ///
    /// # Errors
    ///
    /// The error of [`crate::routes::query::resolve_reach`]: `headers` name a
    /// tenant the caller does not belong to, or the deployment's tenant count
    /// cannot be read.
    pub(crate) async fn of(
        state: &'a AppState,
        principal: &Principal,
        headers: &axum::http::HeaderMap,
    ) -> Result<Self, ApiError> {
        let reach = crate::routes::query::resolve_reach(state, principal, headers).await?;
        Ok(Self { state, reach })
    }

    /// A reach that covers no table, for a reader that could not resolve one
    /// (fail closed).
    pub(crate) fn nothing(state: &'a AppState) -> Self {
        Self {
            state,
            reach: crate::routes::query::TenantReach::NoTenant,
        }
    }

    /// Which of the registry's Bronze `tables` this reach covers, in one
    /// claims read. `None` when the reach is everything, so nothing is
    /// narrowed and nothing is read.
    ///
    /// # Errors
    ///
    /// The 503 of [`crate::routes::query::claims_on`] when the claims cannot
    /// be read.
    pub(crate) async fn bronze_in_reach<'t>(
        &self,
        tables: impl IntoIterator<Item = &'t str>,
    ) -> Result<Option<std::collections::HashSet<String>>, ApiError> {
        if self.reach == crate::routes::query::TenantReach::All {
            return Ok(None);
        }
        let tables: std::collections::BTreeSet<&str> = tables.into_iter().collect();
        let keys: Vec<String> = tables
            .iter()
            .map(|table| format!("bronze.{table}"))
            .collect();
        let reachable =
            crate::routes::query::reachable_names(self.state, self.reach, &keys).await?;
        Ok(Some(
            tables
                .into_iter()
                .filter(|table| reachable.contains(&format!("bronze.{table}")))
                .map(str::to_owned)
                .collect(),
        ))
    }
}

/// [`RegistryReach::bronze_in_reach`] for a tool: the tables of `rows` the
/// reach covers, or the tool's own fixed refusal when the claims cannot be
/// read (the cause never reaches the model).
///
/// # Errors
///
/// The tool's answer `{"error": "the dataset claims could not be read right
/// now"}` when the claims cannot be read. `Ok(None)` means nothing narrows.
async fn registry_rows_in_reach(
    reach: Option<RegistryReach<'_>>,
    rows: &[Map<String, Value>],
) -> Result<Option<std::collections::HashSet<String>>, Value> {
    let Some(reach) = reach else {
        return Ok(None);
    };
    reach
        .bronze_in_reach(
            rows.iter()
                .map(|r| r.get("table_name").and_then(Value::as_str).unwrap_or("")),
        )
        .await
        .map_err(|_| json!({ "error": "the dataset claims could not be read right now" }))
}

/// [`RegistryReach::of`] for a tool, with the tool's fixed refusal when the
/// reach cannot be resolved. The tools have no request headers, so the reach
/// is the person's own tenant's, the convention the maintenance tool follows.
///
/// # Errors
///
/// The tool's answer `{"error": "the shared-catalog rule could not be
/// evaluated"}` when the reach cannot be resolved.
async fn callers_registry_reach<'a>(
    state: &'a AppState,
    principal: &Principal,
) -> Result<RegistryReach<'a>, Value> {
    RegistryReach::of(state, principal, &axum::http::HeaderMap::new())
        .await
        .map_err(|_| json!({ "error": "the shared-catalog rule could not be evaluated" }))
}

/// `GOV-7` D6: what the caller's roles may read, or an error sentence for the
/// tool. Every caller refuses a run with no principal before asking
/// (`SEC-20`, plan 97), so a principal is required here.
async fn table_access(
    state: &AppState,
    principal: &Principal,
) -> Result<crate::table_access::TableAccess, &'static str> {
    // SEC-68: `for_principal` distinguishes a genuine `*:*` admin from a
    // role-less service identity; the latter must not silently read every
    // table through `for_roles(&[])`.
    crate::table_access::for_principal(state.pg.as_deref(), principal)
        .await
        .map_err(|_| "the table access rule could not be read right now")
}

/// `SEC-20` (plan 95): the shared catalog's own rule, asked for a tool that
/// answers from the shared catalog.
///
/// `None` when the caller may read it. Otherwise the answer the tool returns
/// as it is: `supported: false` with the rule's own reason for a refused
/// caller, or a fixed error when the rule cannot be evaluated (the cause is
/// not forwarded, `AGENTS.md` principle 4). The tool has no request headers,
/// so the rule reads the person's own tenant, as `run_sql` does. It is
/// [`crate::routes::catalog::catalog_tenant_refusal`], never a second copy.
pub(super) async fn shared_catalog_refusal(
    state: &AppState,
    principal: &Principal,
) -> Option<Value> {
    match crate::routes::catalog::catalog_tenant_refusal(
        state,
        principal,
        &axum::http::HeaderMap::new(),
    )
    .await
    {
        Ok(None) => None,
        Ok(Some(reason)) => Some(json!({ "supported": false, "reason": reason })),
        Err(_) => Some(json!({ "error": "the shared-catalog rule could not be evaluated" })),
    }
}

/// Whether the dataset behind Bronze table `table` is visible: any of its
/// layers the caller may read makes the dataset itself visible (`GOV-7` D6).
fn dataset_visible(access: &crate::table_access::TableAccess, table: &str) -> bool {
    access.is_all()
        || access.allows_key(&format!("bronze.{table}"))
        || access.allows_key(&format!("serving.{table}"))
}

/// Row counts of every table in `serving` and `silver`, keyed
/// `"db.table"`. `None` for a view (no stored row count).
async fn table_rows(
    ch: &ChClient,
) -> Result<std::collections::HashMap<String, Option<u64>>, String> {
    let rows = ch
        .rows(
            "SELECT database, name, toString(total_rows) AS n FROM system.tables \
             WHERE database IN ('serving', 'silver')",
            None,
        )
        .await
        .map_err(|_| "the table list could not be read from ClickHouse".to_owned())?;
    Ok(rows
        .iter()
        .map(|r| {
            let db = r.get("database").and_then(Value::as_str).unwrap_or("");
            let name = r.get("name").and_then(Value::as_str).unwrap_or("");
            let n = r
                .get("n")
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok());
            (format!("{db}.{name}"), n)
        })
        .collect())
}

/// Which layers a dataset served from Gold table `table` is present in.
/// Bronze is where every registered dataset starts (the catalog record
/// itself). Gold is `serving.<table>`. Silver has no recorded link to a
/// dataset, so it is reported only when a Silver table of the same name
/// exists; otherwise `present: false` with no row count, never a made-up
/// zero (the old `describe_dataset` counted `silver.<table>`, found
/// nothing, and reported `rows: 0` for data that was really there).
fn layers_for(
    slug: &str,
    table: &str,
    counts: &std::collections::HashMap<String, Option<u64>>,
) -> Value {
    let layer = |db: &str| {
        let key = format!("{db}.{table}");
        match counts.get(&key) {
            Some(rows) => json!({ "table": key, "present": true, "rows": rows }),
            None => json!({ "present": false }),
        }
    };
    json!({
        "bronze": { "registered": true, "dataset": slug },
        "silver": layer("silver"),
        "gold": layer("serving"),
    })
}

/// The dataset list of a `bi-only` install: the active engine's own external
/// tables, never the lakehouse registry, `ClickHouse` (when Trino is active)
/// or `Lakekeeper`. The fields the assistant already reads keep their names;
/// every warehouse statistic is `null` because an in-place source has no
/// parts, row counts or freshness to report (`AGENTS.md` principle 2).
async fn bi_only_list_datasets(state: &AppState, args: &Map<String, Value>) -> Value {
    let engine = state.config.query_engine;
    let cap = 200_usize;
    let outcome = match engine {
        crate::config::QueryEngine::Trino => {
            crate::in_place::introspect_trino_tables(&state.trino, cap).await
        }
        crate::config::QueryEngine::ClickHouse => {
            crate::in_place::introspect_clickhouse_external_tables(&state.clickhouse, cap).await
        }
    };
    let tables = match outcome {
        Ok(tables) => tables,
        Err(err) => {
            // `IntrospectionError` is already classified by `upstream_error`:
            // render its `ApiError` through the same JSON body the routes use,
            // so the SEC-11 guard sees no error text interpolated here.
            let failed: ApiResult<ApiJson<Value>> = Err(err.into_api_error().into());
            return api_result_to_value(failed).await;
        }
    };
    let term = arg_str(args, "search").trim().to_lowercase();
    let hits: Vec<Value> = tables
        .iter()
        .filter(|table| {
            term.is_empty()
                || format!("{} {}", table.full_name, table.name)
                    .to_lowercase()
                    .contains(&term)
        })
        .take(40)
        .map(|table| {
            json!({
                "slug": table.full_name,
                "title": table.name,
                "sourceKind": "external",
                "goldTable": Value::Null,
                "zone": "external",
                "engine": engine.name(),
                "fullName": table.full_name,
                "rows": Value::Null,
                "sizeBytes": Value::Null,
                "columnCount": Value::Null,
            })
        })
        .collect();
    json!({ "total": hits.len(), "datasets": hits })
}

/// `describe_dataset` over an in-place table. The identifier is the dotted
/// name the active engine reads: `catalog.schema.table` on `Trino`,
/// `schema.table` on `ClickHouse`. A short or over-long name is refused
/// before an engine is dialed; an upstream failure is the classified
/// `in_place::IntrospectionError` text (a fixed sentence and a reference
/// id), never the engine's own message.
async fn bi_only_describe_dataset(state: &AppState, args: &Map<String, Value>) -> Value {
    let engine = state.config.query_engine;
    let slug = arg_str(args, "slug");
    let parts: Vec<&str> = slug.split('.').collect();
    let outcome = match engine {
        crate::config::QueryEngine::Trino => {
            if parts.len() != 3 || parts.iter().any(|p| p.is_empty()) {
                return json!({
                    "error": "an in-place Trino table is named catalog.schema.table",
                });
            }
            crate::in_place::introspect_trino_columns(&state.trino, parts[0], parts[1], parts[2])
                .await
        }
        crate::config::QueryEngine::ClickHouse => {
            if parts.len() != 2 || parts.iter().any(|p| p.is_empty()) {
                return json!({
                    "error": "an in-place ClickHouse table is named schema.table",
                });
            }
            crate::in_place::introspect_clickhouse_columns(&state.clickhouse, parts[0], parts[1])
                .await
        }
    };
    let columns = match outcome {
        Ok(columns) => columns,
        Err(err) => {
            // `IntrospectionError` is already classified by `upstream_error`:
            // render its `ApiError` through the same JSON body the routes use,
            // so the SEC-11 guard sees no error text interpolated here.
            let failed: ApiResult<ApiJson<Value>> = Err(err.into_api_error().into());
            return api_result_to_value(failed).await;
        }
    };
    json!({
        "slug": slug,
        "title": slug,
        "description": format!("External table read in place through {}. No lakehouse copy exists.", engine.name()),
        "sourceKind": "external",
        "zone": "external",
        "engine": engine.name(),
        "layers": {
            "bronze": { "registered": false },
            "silver": { "present": false },
            "gold": { "present": false },
            "external": { "present": true, "table": slug, "rows": Value::Null },
        },
        "columns": columns
            .iter()
            .map(|c| json!({ "name": c.name, "type": c.data_type, "description": Value::Null }))
            .collect::<Vec<_>>(),
    })
}

pub(super) async fn list_datasets(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    // SEC-20, plan 97: no principal is a scheduled run, which has no tenant
    // to read the shared registry as.
    let Some(principal) = principal else {
        return json!({ "error": DATASETS_NEED_A_USER });
    };
    // `BI-44` slice 3: a `bi-only` install has no registry to read. Return
    // before any tenant/registry check that would dial the missing
    // lakehouse services; the active engine's own catalog is the answer.
    if state.config.is_bi_only() {
        return bi_only_list_datasets(state, args).await;
    }
    // SEC-20, plan 95: the registry is the shared catalog's, so a tenant that
    // does not own it is refused before the registry is read.
    if let Some(refusal) = shared_catalog_refusal(state, principal).await {
        return refusal;
    }
    let access = match table_access(state, principal).await {
        Ok(access) => access,
        Err(sentence) => return json!({ "error": sentence }),
    };
    let reach = match callers_registry_reach(state, principal).await {
        Ok(reach) => reach,
        Err(refusal) => return refusal,
    };
    list_datasets_with(&state.clickhouse, &access, Some(reach), args).await
}

/// [`list_datasets`] over an already resolved access rule and tenant reach —
/// the body the tests drive without a `Principal`. `reach` is `None` for a
/// caller that narrows nothing.
pub(super) async fn list_datasets_with(
    ch: &ChClient,
    access: &crate::table_access::TableAccess,
    reach: Option<RegistryReach<'_>>,
    args: &Map<String, Value>,
) -> Value {
    let Ok(rows) = ch
        .rows(
            &format!("SELECT slug, title, tier, table_name FROM {CATALOG_UNION} LIMIT 500"),
            None,
        )
        .await
    else {
        return json!({ "error": "the dataset catalog could not be read (the catalog registry is not available)" });
    };
    // SEC-20, plan 97: a dataset whose Bronze table another tenant holds is
    // not listed. One claims read for the whole registry page.
    let in_reach = match registry_rows_in_reach(reach, &rows).await {
        Ok(in_reach) => in_reach,
        Err(refusal) => return refusal,
    };
    let term = arg_str(args, "search").to_lowercase();
    let wanted = tier_filter(&{
        let source = arg_str(args, "source");
        if source.is_empty() {
            arg_str(args, "tier")
        } else {
            source
        }
    })
    .to_owned();
    let hits: Vec<Value> = rows
        .iter()
        .filter(|r| {
            let table = r.get("table_name").and_then(Value::as_str).unwrap_or("");
            if !dataset_visible(access, table) {
                return false;
            }
            if in_reach.as_ref().is_some_and(|set| !set.contains(table)) {
                return false;
            }
            let row_tier = r.get("tier").and_then(Value::as_str).unwrap_or("");
            if !wanted.is_empty() && row_tier != wanted {
                return false;
            }
            if term.is_empty() {
                return true;
            }
            let title = r.get("title").and_then(Value::as_str).unwrap_or("");
            let slug = r.get("slug").and_then(Value::as_str).unwrap_or("");
            format!("{title} {slug}").to_lowercase().contains(&term)
        })
        .take(40)
        .map(|r| {
            let table = r.get("table_name").and_then(Value::as_str).unwrap_or("");
            json!({
                "slug": r.get("slug"),
                "title": r.get("title"),
                "sourceKind": source_kind(r.get("tier").and_then(Value::as_str).unwrap_or("")),
                "goldTable": if table.is_empty() { Value::Null } else { json!(format!("serving.{table}")) },
            })
        })
        .collect();
    json!({ "total": hits.len(), "datasets": hits })
}

pub(super) async fn describe_dataset(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    // SEC-20, plans 95 and 97: as in `list_datasets`.
    let Some(principal) = principal else {
        return json!({ "error": DATASETS_NEED_A_USER });
    };
    // `BI-44` slice 3: as in `list_datasets`, no shared registry is dialed
    // in a `bi-only` install.
    if state.config.is_bi_only() {
        return bi_only_describe_dataset(state, args).await;
    }
    if let Some(refusal) = shared_catalog_refusal(state, principal).await {
        return refusal;
    }
    let access = match table_access(state, principal).await {
        Ok(access) => access,
        Err(sentence) => return json!({ "error": sentence }),
    };
    let reach = match callers_registry_reach(state, principal).await {
        Ok(reach) => reach,
        Err(refusal) => return refusal,
    };
    describe_dataset_with(&state.clickhouse, &access, Some(reach), args).await
}

/// [`describe_dataset`] over an already resolved access rule and tenant
/// reach — the body the tests drive without a `Principal`. `reach` is `None`
/// for a caller that narrows nothing.
pub(super) async fn describe_dataset_with(
    ch: &ChClient,
    access: &crate::table_access::TableAccess,
    reach: Option<RegistryReach<'_>>,
    args: &Map<String, Value>,
) -> Value {
    let slug_raw = arg_str(args, "slug");
    let slug = SqlLiteral::from(slug_raw.as_str());
    let Ok(meta_rows) = ch
        .rows(
            &format!(
                "SELECT title, description, table_name, tier FROM {CATALOG_UNION} WHERE slug={slug} LIMIT 1"
            ),
            None,
        )
        .await
    else {
        return json!({ "error": "the dataset catalog could not be read" });
    };
    let Some(meta) = meta_rows.first() else {
        return json!({ "error": format!("no dataset with slug '{slug_raw}'; call list_datasets for the slugs") });
    };
    let table = meta
        .get("table_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    // SEC-20, plan 97: a dataset whose Bronze table another tenant holds is
    // refused with the sentence a table out of reach gets everywhere else
    // (`describe_iceberg_table`), before its columns are read. The sentence
    // names the slug the caller asked for, never the registry's own table
    // name, which would give away the other tenant's upload id.
    match registry_rows_in_reach(reach, std::slice::from_ref(meta)).await {
        Ok(Some(in_reach)) if !in_reach.contains(&table) => {
            return json!({ "error": crate::policy_engine::table_not_available_message(&slug_raw) });
        }
        Ok(_) => {}
        Err(refusal) => return refusal,
    }
    // DATA-39 (the assistant part of `#315`): the same rule as the DATA
    // MAP's description read above. `FINAL` makes a column a re-run
    // registered one row instead of one per run, and the shared clause
    // takes the primary registry's copy for a slug both registries hold —
    // a slug the primary owns is not answered from the secondary one
    // (`DATA-40` D1). `NOT_IN_PRIMARY` is the constant, spliced in, never
    // retyped.
    let cols = ch
        .rows(
            &format!(
                "SELECT key_asli AS name, tipe AS type, deskripsi AS description FROM lake.`bronze_meta.dataset_column` FINAL WHERE slug={slug} \
                 UNION ALL SELECT key_asli, tipe, deskripsi FROM lake.`bronze_meta_sec.dataset_column` FINAL WHERE slug={slug} AND slug {NOT_IN_PRIMARY}"
            ),
            None,
        )
        .await
        .unwrap_or_default();
    let counts = table_rows(ch).await.unwrap_or_default();
    // `GOV-7` D6: a dataset every one of whose layers this caller may not
    // read is not described at all — the same answer an unknown slug gets.
    let visible = ["bronze", "silver", "serving"]
        .iter()
        .any(|db| access.is_all() || access.allows_key(&format!("{db}.{table}")));
    if !visible {
        return json!({ "error": format!("no dataset with slug '{slug_raw}'; call list_datasets for the slugs") });
    }
    let mut layers = layers_for(&slug_raw, &table, &counts);
    // The layer entries name their tables; drop the ones the caller may not
    // read (Bronze keeps its `registered` flag, which names no table).
    if !access.is_all()
        && let Some(o) = layers.as_object_mut()
    {
        for (db, key) in [("silver", "silver"), ("serving", "gold")] {
            if !access.allows_key(&format!("{db}.{table}"))
                && let Some(layer) = o.get_mut(key)
            {
                *layer = json!({ "present": false });
            }
        }
    }
    // The columns are the Bronze table's own; hidden Bronze hides them.
    let columns = if access.is_all() || access.allows_key(&format!("bronze.{table}")) {
        cols
    } else {
        Vec::new()
    };
    json!({
        "slug": slug_raw,
        "title": meta.get("title"),
        "description": meta.get("description"),
        "sourceKind": source_kind(meta.get("tier").and_then(Value::as_str).unwrap_or("")),
        "layers": layers,
        "columns": columns,
    })
}

/// One view of the whole lakehouse by layer: the registered Bronze
/// datasets, and every Silver and Gold table with its row count. The
/// question "what data do we have?" is answered from this, in layer terms,
/// instead of from a dataset list that only knows source kinds.
pub(super) async fn lakehouse_overview(state: &AppState, principal: Option<&Principal>) -> Value {
    // The same shared-catalog rule the catalog route and the DATA MAP
    // apply: this tool lists the shared catalog and every table in it.
    let Some(principal) = principal else {
        return json!({ "error": "the lakehouse overview needs a signed-in user" });
    };
    if let Some(refusal) = shared_catalog_refusal(state, principal).await {
        return refusal;
    }
    let ch = &state.clickhouse;
    let access = match table_access(state, principal).await {
        Ok(access) => access,
        Err(sentence) => return json!({ "error": sentence }),
    };
    let reach = match callers_registry_reach(state, principal).await {
        Ok(reach) => reach,
        Err(refusal) => return refusal,
    };
    let counts = match table_rows(ch).await {
        Ok(c) => c,
        Err(err) => return json!({ "error": err }),
    };
    let datasets = ch
        .rows(
            &format!(
                "SELECT slug, title, tier, table_name FROM {CATALOG_UNION} ORDER BY slug LIMIT 500"
            ),
            None,
        )
        .await;
    // SEC-20, plan 97: a dataset whose Bronze table another tenant holds is
    // not listed. One claims read for the whole registry page.
    let in_reach = match &datasets {
        Ok(rows) => match registry_rows_in_reach(Some(reach), rows).await {
            Ok(in_reach) => in_reach,
            Err(refusal) => return refusal,
        },
        Err(_) => None,
    };
    let (bronze, catalog_note) = match &datasets {
        Ok(rows) => (
            rows.iter()
                .filter(|r| {
                    let table = r.get("table_name").and_then(Value::as_str).unwrap_or("");
                    dataset_visible(&access, table)
                        && in_reach.as_ref().is_none_or(|set| set.contains(table))
                })
                .map(|r| {
                    let table = r.get("table_name").and_then(Value::as_str).unwrap_or("");
                    json!({
                        "slug": r.get("slug"),
                        "title": r.get("title"),
                        "sourceKind": source_kind(r.get("tier").and_then(Value::as_str).unwrap_or("")),
                        "servedFrom": format!("serving.{table}"),
                    })
                })
                .collect::<Vec<_>>(),
            Value::Null,
        ),
        Err(_) => (Vec::new(), json!("the dataset catalog could not be read, so Bronze datasets are not listed")),
    };
    let mut tables: Vec<(&String, &Option<u64>)> = counts.iter().collect();
    tables.sort();
    let layer = |db: &str| -> Vec<Value> {
        tables
            .iter()
            .filter(|(k, _)| k.starts_with(&format!("{db}.")) && access.allows_key(k))
            .map(|(k, rows)| json!({ "table": k, "rows": rows }))
            .collect()
    };
    json!({
        "bronze": { "description": "raw data landed from source systems (Iceberg)", "datasets": bronze, "note": catalog_note },
        "silver": { "description": "cleaned, typed detail tables", "tables": layer("silver") },
        "gold": { "description": "aggregated serving marts", "tables": layer("serving") },
    })
}

/// The one sentence for a call that names both subjects or neither
/// (`AI-13` decision 1): a slug (or its legacy alias `focus`) and a chart id.
const ONE_SUBJECT: &str = "Call get_lineage with exactly one of slug (a dataset slug or a table name) or chart (a chart id).";

/// The one refusal for every failure of the chart read (`AI-13` decision 3):
/// an unknown id, somebody else's personal chart, a caller without
/// `dashboard:read`, a caller the shared catalog refuses, and a read that
/// failed all answer the same sentence, so no answer says the chart exists,
/// and no upstream error text reaches the answer (principle 4).
const NO_VISIBLE_CHART: &str = "No chart lineage is available for that id.";

/// The one reason for a `slug` focus that is not a table name (`AI-13`
/// review SHOULD-FIX): `used_by::read` refuses such a focus with
/// `BadRequest`, and this sentence names that cause, which is the
/// caller's to fix.
const USED_BY_NOT_A_TABLE: &str =
    "charts are listed for a table name such as serving.<table>, not this focus";

/// The one reason for every other used-by failure (`AI-13` review
/// SHOULD-FIX, principle 2): a table the caller's role hides (`GOV-7` D6
/// answers the route's 404), a tenant or shared-catalog refusal, a
/// database failure. True for its cause, names no table's existence, and
/// no upstream error text reaches the answer.
const USED_BY_UNREADABLE: &str = "the used-by list could not be read for this user";

/// The compact path for the model (`AI-13`): the chart route answer's own
/// steps, nearest the chart first, each with its refresh time or "not
/// recorded". The arrow between two steps is the kind of the edge that
/// connects them, the `{from} -[{kind}]-> {to}` shape the slug path prints
/// per edge; a pair with no edge (branches of the same table) reads
/// `upstream`.
fn chart_chain(answer: &Value) -> String {
    let Some(steps) = answer["steps"].as_array().filter(|s| !s.is_empty()) else {
        return "no recorded lineage".to_owned();
    };
    let kinds: std::collections::HashMap<(&str, &str), &str> = answer["edges"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let from = e.get("from")?.as_str()?;
            let to = e.get("to")?.as_str()?;
            Some(((from, to), e.get("kind")?.as_str()?))
        })
        .collect();
    let mut chain = String::new();
    let mut previous: Option<&str> = None;
    for step in steps {
        let (Some(id), Some(label), Some(kind)) = (
            step.get("id").and_then(Value::as_str),
            step.get("label").and_then(Value::as_str),
            step.get("kind").and_then(Value::as_str),
        ) else {
            continue;
        };
        if let Some(to) = previous {
            let edge = kinds.get(&(id, to)).copied().unwrap_or("upstream");
            let _ = write!(chain, " -[{edge}]-> ");
        }
        chain.push_str(label);
        let _ = write!(chain, " ({kind}");
        match (
            step.get("refreshedAt").and_then(Value::as_str),
            step.get("refreshSource").and_then(Value::as_str),
        ) {
            (Some(at), Some(origin)) => {
                let _ = write!(chain, ", refreshed {at} via {origin}");
            }
            (Some(at), None) => {
                let _ = write!(chain, ", refreshed {at}");
            }
            (None, _) if step.get("notRecorded") == Some(&json!(true)) => {
                let _ = write!(chain, ", refresh not recorded");
            }
            (None, _) => {}
        }
        chain.push(')');
        previous = Some(id);
    }
    chain
}

/// The `chart` path (`AI-13` decision 1): the chart-lineage route's own
/// read, never a graph built here — `charts::read` is where every gate
/// lives (`dashboard:read`, the shared catalog, the chart's visibility,
/// per-role table hiding), so whatever it answers is what the route would.
async fn chart_lineage_answer(state: &AppState, principal: &Principal, chart: &str) -> Value {
    // `headers: HeaderMap::new()` — no `X-Tenant` selection from the copilot:
    // the tenant resolves from the principal, the same default an interactive
    // caller gets by omitting the header.
    let Ok(answer) = crate::routes::lineage::charts::read(
        state,
        principal,
        &axum::http::HeaderMap::new(),
        chart,
    )
    .await
    else {
        return json!({ "error": NO_VISIBLE_CHART });
    };
    let mut out = answer;
    out["chain"] = json!(chart_chain(&out));
    out
}

/// The recorded lineage around a dataset, a table or a chart: the same
/// graph `GET /api/governance/lineage` serves (`routes::lineage`), and for a
/// chart the same path `GET /api/dashboard/charts/{id}/lineage` serves
/// (`routes::lineage::charts`), so the copilot and the pages can never
/// disagree. `chain` lists the path as one string for the model;
/// `nodes`/`edges` carry the graph. A `slug` answer also carries `usedBy`,
/// the charts that read the table, from the used-by route's own read.
pub(super) async fn get_lineage(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let slug = arg_str(args, "slug");
    let focus = if slug.is_empty() {
        arg_str(args, "focus")
    } else {
        slug
    };
    let chart = arg_str(args, "chart");
    if (chart.is_empty() && focus.is_empty()) || (!chart.is_empty() && !focus.is_empty()) {
        return json!({ "error": ONE_SUBJECT });
    }
    let Some(principal) = principal else {
        return json!({ "error": "lineage needs a signed-in user" });
    };
    if !chart.is_empty() {
        return chart_lineage_answer(state, principal, &chart).await;
    }
    // SEC-20, plan 95: a caller the shared-catalog rule refuses gets no graph
    // at all from the tool. (`GET /api/governance/lineage` keeps drawing the
    // tenant's own records for such a caller, and since plan 96 its builder
    // leaves the dashboard charts out for them, so this refusal is the tool's
    // stricter answer, not the only gate.)
    if let Some(refusal) = shared_catalog_refusal(state, principal).await {
        return refusal;
    }
    let Ok(graph) =
        crate::routes::lineage::build(state, principal, &axum::http::HeaderMap::new(), &focus)
            .await
    else {
        return json!({ "error": "lineage could not be built for this user" });
    };
    let labels: std::collections::HashMap<&str, &str> = graph["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| Some((n.get("id")?.as_str()?, n.get("label")?.as_str()?)))
        .collect();
    let chain: Vec<String> = graph["edges"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let from = e.get("from")?.as_str()?;
            let to = e.get("to")?.as_str()?;
            Some(format!(
                "{} -[{}]-> {}",
                labels.get(from).unwrap_or(&from),
                e.get("kind")?.as_str()?,
                labels.get(to).unwrap_or(&to)
            ))
        })
        .collect();
    let mut out = graph.clone();
    out["chain"] = if chain.is_empty() {
        json!(
            graph
                .get("note")
                .and_then(Value::as_str)
                .unwrap_or("no recorded lineage")
        )
    } else {
        json!(chain.join("\n"))
    };
    // `AI-13` decision 2: the page's other promise — the charts that read
    // this table — from the used-by route's own read under the same caller,
    // never a second list built here. The route caps the list (`MAX_CHARTS`);
    // its answer passes through as it is. `headers: HeaderMap::new()` for the
    // same reason as the chart read above.
    out["usedBy"] = match crate::routes::lineage::used_by::read(
        state,
        principal,
        &axum::http::HeaderMap::new(),
        &focus,
    )
    .await
    {
        Ok(answer) => answer,
        // A focus that is not a table name (a dataset slug, say) has no
        // used-by answer at all. Every other failure — a table the caller's
        // role hides (`GOV-7` D6), a tenant or shared-catalog refusal, a
        // database failure — gets one honest sentence true for its cause,
        // never an empty list that would read as "no chart uses it", and no
        // upstream error text (principle 2, review SHOULD-FIX).
        Err(ApiError::BadRequest(_)) => json!({
            "supported": false,
            "reason": USED_BY_NOT_A_TABLE,
        }),
        Err(_) => json!({
            "supported": false,
            "reason": USED_BY_UNREADABLE,
        }),
    };
    out
}

/// Counts the latest verdict per `(tabel, cek)` a narrowed caller may read,
/// grouped by verdict (`GOV-16`).
///
/// The rows come from the per-table query below rather than a `GROUP BY
/// verdict` in `ClickHouse`, because a table the caller's role hides must be
/// dropped before the counts are made: an aggregate taken over every table
/// cannot be narrowed afterwards. Filtering in [`crate::routes::quality::
/// asset_readable`] — the predicate the quality, incident, schedule and
/// classification lists use — keeps the six readers on one rule. A table with
/// no name is hidden from a narrowed caller (fail closed). The counts are
/// strings, as `toString(count())` made them in the SQL this replaces.
fn quality_summary(
    rows: &[Map<String, Value>],
    access: &crate::table_access::TableAccess,
) -> Value {
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for row in rows {
        let table = row.get("tabel").and_then(Value::as_str);
        if !crate::routes::quality::asset_readable(access, table) {
            continue;
        }
        let verdict = row
            .get("verdict")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        *counts.entry(verdict).or_default() += 1;
    }
    let summary: Vec<Value> = counts
        .into_iter()
        .map(|(verdict, n)| json!({ "verdict": verdict, "n": n.to_string() }))
        .collect();
    json!({ "summary": summary })
}

pub(super) async fn get_quality(state: &AppState, principal: Option<&Principal>) -> Value {
    // GOV-16: narrow the summary by the person's role table access, the GOV-7
    // / GOV-15 mechanism every other quality reader uses. A headless run is a
    // system call and is not narrowed (`GOV-15` D7); a person whose role
    // cannot be read right now gets an error rather than every table's count
    // (fail closed).
    let access = match principal {
        Some(principal) => match table_access(state, principal).await {
            Ok(access) => access,
            Err(sentence) => return json!({ "error": sentence }),
        },
        None => crate::table_access::TableAccess::All,
    };
    match state
        .clickhouse
        .rows(
            "SELECT tabel, verdict FROM ( \
               SELECT tabel, cek, argMax(verdict, dibuat_pada) verdict FROM _silver_meta.quality GROUP BY tabel, cek \
             )",
            None,
        )
        .await
    {
        Ok(rows) => quality_summary(&rows, &access),
        // Measured on the local stack: the results table simply does not
        // exist until a quality run writes it, and the raw
        // `UNKNOWN_DATABASE` text went straight into the answer. Say what
        // it means instead (AGENTS.md principle 4).
        Err(err) if err.to_string().contains("UNKNOWN_DATABASE") || err.to_string().contains("UNKNOWN_TABLE") => json!({
            "supported": false,
            "reason": "no data quality checks have been run yet: there are no quality results to report",
            "hint": "list_quality_rules shows the quality rules that are defined",
        }),
        Err(err) => {
            crate::upstream_error::report_ch(&crate::upstream_error::DATABASE, &err).to_json()
        }
    }
}

/// `DATA-12` task 10: the assistant's mart description carries each
/// mart's certification fields, so it can tell a user that a mart it is
/// about to build a chart on is deprecated, and name the replacement.
pub(super) async fn describe_mart(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    // SEC-76: a headless run has no person whose reach narrows the
    // marts, so it reads none of them, not the whole list.
    let allowed = match principal {
        Some(p) => matches!(
            crate::routes::catalog::catalog_tenant_refusal(state, p, &axum::http::HeaderMap::new())
                .await,
            Ok(None)
        ),
        None => return json!({ "error": DATASETS_NEED_A_USER }),
    };
    let ch = &state.clickhouse;
    let mart = strip_non_ident(&arg_str(args, "mart"));
    if mart.is_empty() {
        let Ok(rows) = ch
            .rows(
                "SELECT name, toString(total_rows) AS total_rows FROM system.tables \
                 WHERE database='serving' AND name NOT LIKE '%\\_baru' ORDER BY name",
                None,
            )
            .await
        else {
            return json!({ "error": "the Gold mart list could not be read from ClickHouse" });
        };
        let mut marts: Vec<Value> = rows
            .iter()
            .map(|r| {
                // A view has no stored row count: report it as unknown,
                // never as an empty table.
                let rows_n: Option<i64> = r
                    .get("total_rows")
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse().ok());
                json!({ "mart": r.get("name"), "rows": rows_n })
            })
            .collect();
        if allowed {
            apply_serving_certification(state, &mut marts, "mart").await;
        }
        return json!({ "marts": marts });
    }
    let Ok(cols) = ch
        .rows(
            &format!("SELECT name, type FROM system.columns WHERE database='serving' AND table='{mart}' ORDER BY position"),
            None,
        )
        .await
    else {
        return json!({ "error": "the mart's columns could not be read from ClickHouse" });
    };
    if cols.is_empty() {
        return json!({ "error": format!("no Gold mart named '{mart}' in serving; call describe_mart with no argument for the list") });
    }
    let dimensions: Vec<&str> = cols
        .iter()
        .filter(|c| !is_numeric_type(c.get("type").and_then(Value::as_str).unwrap_or("")))
        .filter_map(|c| c.get("name").and_then(Value::as_str))
        .collect();
    let measures: Vec<&str> = cols
        .iter()
        .filter(|c| is_numeric_type(c.get("type").and_then(Value::as_str).unwrap_or("")))
        .filter_map(|c| c.get("name").and_then(Value::as_str))
        .collect();
    // BI-9: each column's kind (number, date, datetime, text), so the
    // assistant can tell which dimensions take a `grain`.
    let columns: Vec<Value> = cols
        .iter()
        .filter_map(|c| {
            let name = c.get("name").and_then(Value::as_str)?;
            let ty = c.get("type").and_then(Value::as_str).unwrap_or("");
            Some(json!({
                "name": name,
                "kind": lakehouse_bi::filters::ColumnKind::from_clickhouse_type(ty),
            }))
        })
        .collect();
    // One entry through the list-shaped helper, so the mark merge has a
    // single owner; a single mart is that list of one.
    let mut described = vec![
        json!({ "mart": mart, "dimensions": dimensions, "measures": measures, "columns": columns }),
    ];
    if allowed {
        apply_serving_certification(state, &mut described, "mart").await;
    }
    described.remove(0)
}

#[cfg(test)]
mod dry_run {
    //! WS7 item F4: proves `dry_run_sql` asks `ClickHouse`'s own
    //! `EXPLAIN AST` — not a string guess — and refuses on the engine's
    //! own reported statement kind. Response shapes match `ClickHouse`
    //! `26.7.3.19`'s real `EXPLAIN AST` output, verified live during WS7
    //! planning (this plan's §0 item 6).
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    /// A mocked `ClickHouse` that answers every request (this test only
    /// ever issues the one `EXPLAIN AST` query) with `explain_text` as a
    /// PLAIN TEXT body — the shape `EXPLAIN` really returns. These tests
    /// used to mock a `{"data": [{"explain": …}]}` JSON envelope, which
    /// `EXPLAIN AST` never produces (see [`dry_run_sql`]'s comment): they
    /// passed while the real tool refused every statement. Returns the
    /// [`MockServer`] too — it must outlive the [`ChClient`] built from
    /// its URI, or the mock stops answering before the test runs.
    async fn fake_clickhouse_explain_ast_response(explain_text: &str) -> (MockServer, ChClient) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(explain_text))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        (server, ch)
    }

    /// The regression this pair of tests missed: an EMPTY body (what the
    /// old `FORMAT JSON` path effectively produced) must not be read as a
    /// `SELECT`, and a real `SELECT`'s text dump must be accepted.
    #[tokio::test]
    async fn run_sql_dry_run_refuses_an_empty_explain_body() {
        let (_server, ch) = fake_clickhouse_explain_ast_response("").await;
        let mut args = Map::new();
        args.insert("sql".to_owned(), json!("SELECT 1"));
        assert_eq!(
            dry_run_sql(&ch, &args).await,
            json!({"error": "only SELECT is allowed (EXPLAIN AST reported unknown)"})
        );
    }

    /// SEC-11: the parse step's `ClickHouse` error is not relayed to the
    /// model; it gets a fixed message and a reference id.
    #[tokio::test]
    async fn run_sql_dry_run_hides_a_clickhouse_parse_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string(
                "Code: 62. DB::Exception: Syntax error near planted-marker-token (version 0.0.0)",
            ))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let mut args = Map::new();
        args.insert("sql".to_owned(), json!("SELEC 1"));
        let result = dry_run_sql(&ch, &args).await;
        let text = result.to_string();
        assert!(!text.contains("planted-marker"), "{text}");
        assert!(!text.contains("version 0.0.0"), "{text}");
        assert!(result["errorId"].is_string(), "{text}");
    }

    #[tokio::test]
    async fn run_sql_dry_run_refuses_a_non_select_statement() {
        let (_server, ch) =
            fake_clickhouse_explain_ast_response("InsertQuery   (children 2)\n Identifier t\n")
                .await;
        let mut args = Map::new();
        args.insert("sql".to_owned(), json!("INSERT INTO t VALUES (1)"));
        let result = dry_run_sql(&ch, &args).await;
        assert_eq!(
            result,
            json!({"error": "only SELECT is allowed (EXPLAIN AST reported InsertQuery)"})
        );
    }

    #[tokio::test]
    async fn run_sql_dry_run_allows_a_select_with_a_cte_and_union() {
        let (_server, ch) = fake_clickhouse_explain_ast_response(
            "SelectWithUnionQuery (children 1)\n ExpressionList ...\n",
        )
        .await;
        let mut args = Map::new();
        args.insert(
            "sql".to_owned(),
            json!("WITH x AS (SELECT 1) SELECT * FROM x UNION ALL SELECT 2"),
        );
        assert!(dry_run_sql(&ch, &args).await.get("error").is_none());
    }
}

#[cfg(test)]
mod run_sql_delegation {
    //! WS7 item C3: `run_sql` now enforces policy through the SAME path
    //! Query Studio uses (`routes::query::run`), closing the
    //! divergent-guard gap the WS7 plan's §0 item 7 names — before this
    //! change `run_sql` called `ch.query` directly, so a governed table's
    //! `email` column was never masked for a copilot-issued query even
    //! though the exact same principal reached it. Real Postgres
    //! (`#[sqlx::test]`, a real authored policy row) plus a wiremock
    //! `ClickHouse` (`ChClient` speaks plain HTTP; there is no
    //! `last_query()`-style test double on it — every assertion below
    //! reads the mock server's own recorded requests instead, the same
    //! pattern `routes::query::tests::trino_engine` already uses), the
    //! same two harnesses `tools::queries`'s `principal_forwarding` module
    //! relies on for the equivalent `run_saved_query` proof.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use lakehouse_auth::{PermissionSet, PrincipalId};
    use lakehouse_store::PgPool;
    use lakehouse_store::governance::CreatePolicyInput;
    use uuid::Uuid;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::config::Config;

    /// `DATA-12` task 10's tests reuse it: the two tool tests that need a
    /// real Postgres behind an `AppState` share one helper rather than two.
    pub(super) fn database_url_for(pool: &PgPool) -> String {
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

    /// The shared catalog belongs to one tenant, and the query path is
    /// scoped to the tenant: this fixture principal is the catalog's own
    /// member, so the shared tables stay reachable. The chat sends no
    /// `X-Tenant` header, so the tenant is the caller's first one.
    const CATALOG_TENANT_ID: &str = "11111111-1111-4111-8111-000000000001";

    fn analyst_principal() -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::nil()),
            tenant_ids: vec![uuid::uuid!("11111111-1111-4111-8111-000000000001")],
            display_name: "alice".to_owned(),
            permissions: PermissionSet::parse("query:read"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: vec!["Analyst".to_owned()],
        }
    }

    /// Mounts every `ClickHouse` response `routes::query::run`'s
    /// enforcement path needs for `SELECT * FROM serving.mart_x`: the
    /// `EXPLAIN AST` dry run (WS7 item F4, still checked first by
    /// `run_sql` itself), `system.columns` (`PolicyEngineObligations`'s
    /// real-column resolution, WS7 item C1), `system.tables` (view
    /// detection), and finally the real, masked `SELECT` the derived-table
    /// substitution produces.
    async fn mount_governed_table_responses(server: &MockServer) {
        // Plain text, not a JSON envelope: `EXPLAIN AST` answers in
        // `ClickHouse`'s default text format even when a `FORMAT JSON`
        // clause is appended (see `dry_run_sql`), and this mock has to
        // answer the way the real engine does or the test proves nothing.
        Mock::given(method("POST"))
            .and(body_string_contains("EXPLAIN AST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("SelectWithUnionQuery (children 1)\n"),
            )
            .mount(server)
            .await;
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

    #[sqlx::test(migrations = "../../migrations")]
    async fn run_sql_is_masked_the_same_way_query_studio_is(pool: PgPool) -> sqlx::Result<()> {
        lakehouse_store::governance::create_policy(
            &pool,
            &CreatePolicyInput {
                name: "run-sql-masking-test".to_owned(),
                kind: "Row filter".to_owned(),
                subjects: "Analyst".to_owned(),
                resources: "serving.mart_x".to_owned(),
                effect: "Permit with obligation".to_owned(),
                conditions: Some(
                    r#"{"roles":["Analyst"],"table":"serving.mart_x","mask":["email"]}"#.to_owned(),
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
        env.insert("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT_ID.to_owned());
        let state = AppState::new(Config::from_map(&env).expect("valid config from a map"));

        let principal = analyst_principal();
        let mut args = Map::new();
        args.insert("sql".to_owned(), json!("SELECT * FROM serving.mart_x"));
        let result = run_sql(&state, Some(&principal), &args).await;

        assert!(
            result.get("error").is_none(),
            "expected a successful, masked run, got {result}"
        );
        assert!(
            result["columns"]
                .as_array()
                .expect("columns array")
                .contains(&json!("email")),
            "expected the email column to still be present (masked, not dropped): {result}"
        );

        let requests = server
            .received_requests()
            .await
            .expect("mock server records requests");
        assert!(
            requests
                .iter()
                .any(|r| String::from_utf8_lossy(&r.body).contains("replaceRegexpOne")),
            "expected the ACTUAL query ClickHouse received to be the rewritten/masked form, \
             never the original literal SQL — this is what closes the divergent-guard gap \
             (WS7 plan §0 item 7)"
        );
        Ok(())
    }

    /// A mocked `ClickHouse` that records every request and, if one ever
    /// arrives, answers `EXPLAIN AST` as a `SELECT` and anything else with
    /// an empty result, so a refusal test fails on the recorded count and
    /// not on a confusing downstream error.
    async fn recording_clickhouse() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("EXPLAIN AST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("SelectWithUnionQuery (children 1)\n"),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [{"name": "one", "type": "UInt8"}],
                "data": [{"one": 1}],
                "rows": 1,
            })))
            .mount(&server)
            .await;
        server
    }

    fn state_for(server: &MockServer, database_url: String) -> AppState {
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), database_url);
        env.insert("CH_URL".to_owned(), server.uri());
        AppState::new(Config::from_map(&env).expect("valid config from a map"))
    }

    /// SEC-9-AC3: a statement the policy refuses is stopped before ANY part
    /// of it reaches the engine, not even as `EXPLAIN AST` (which parses and
    /// never fetches, but still lands the statement text in the engine's
    /// query log). The mock records requests; the count must be zero.
    #[tokio::test]
    async fn run_sql_refuses_a_table_function_before_anything_reaches_the_engine() {
        let server = recording_clickhouse().await;
        let state = state_for(&server, "not a postgres url".to_owned());
        let principal = analyst_principal();
        let mut args = Map::new();
        args.insert(
            "sql".to_owned(),
            json!("SELECT * FROM url('http://192.0.2.1/planted-marker', 'RawBLOB')"),
        );
        let result = run_sql(&state, Some(&principal), &args).await;

        assert_eq!(
            result,
            json!({
                "error": crate::policy_engine::refusal_message(
                    &crate::sql_rewrite::RewriteError::TableFunctionDenied {
                        name: "url".to_owned(),
                    }
                ),
            }),
            "expected the policy refusal, got {result}"
        );
        let requests = server.received_requests().await.expect("recorded");
        assert!(
            requests.is_empty(),
            "the engine received {} request(s) for a refused table function",
            requests.len()
        );
    }

    /// SEC-9-AC3, second refusal class: a refused system table. Reaching it
    /// through the full `rewrite_sql_for_principal` would first ask the engine
    /// for `system.tables` metadata about it, so the early check is the
    /// synchronous classification, which needs no round trip.
    #[tokio::test]
    async fn run_sql_refuses_a_sensitive_system_table_before_anything_reaches_the_engine() {
        let server = recording_clickhouse().await;
        let state = state_for(&server, "not a postgres url".to_owned());
        let principal = analyst_principal();
        let mut args = Map::new();
        args.insert(
            "sql".to_owned(),
            json!("SELECT query FROM system.query_log"),
        );
        let result = run_sql(&state, Some(&principal), &args).await;

        assert_eq!(
            result,
            json!({
                "error": crate::policy_engine::refusal_message(
                    &crate::sql_rewrite::RewriteError::SensitiveSystemTable {
                        table: "system.query_log".to_owned(),
                    }
                ),
            }),
            "expected the policy refusal, got {result}"
        );
        let requests = server.received_requests().await.expect("recorded");
        assert!(
            requests.is_empty(),
            "the engine received {} request(s) for a refused system table",
            requests.len()
        );
    }

    /// The other half of SEC-9's ordering: a statement the policy admits
    /// still gets the engine dry run first and is then executed.
    #[sqlx::test(migrations = "../../migrations")]
    async fn run_sql_dry_runs_then_executes_a_statement_the_policy_admits(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let server = recording_clickhouse().await;
        let state = state_for(&server, database_url_for(&pool));
        let principal = analyst_principal();
        let mut args = Map::new();
        args.insert("sql".to_owned(), json!("SELECT 1 AS one"));
        let result = run_sql(&state, Some(&principal), &args).await;
        assert!(result.get("error").is_none(), "{result}");

        let bodies: Vec<String> = server
            .received_requests()
            .await
            .expect("recorded")
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect();
        assert_eq!(
            bodies.len(),
            2,
            "expected dry run then execution: {bodies:?}"
        );
        assert!(bodies[0].contains("EXPLAIN AST"), "{bodies:?}");
        assert!(!bodies[1].contains("EXPLAIN AST"), "{bodies:?}");
        Ok(())
    }

    /// Every body the mock engine received, in order.
    async fn engine_bodies(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .expect("recorded")
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect()
    }

    fn args_with(sql: &str, parameters: Value) -> Map<String, Value> {
        let mut args = Map::new();
        args.insert("sql".to_owned(), json!(sql));
        args.insert("parameters".to_owned(), parameters);
        args
    }

    /// `query-parameters.md` decision 7: the model's `{{region}}` is filled before the dry run and
    /// before the run, so the engine never sees a brace and both calls carry
    /// the one literal.
    #[tokio::test]
    async fn run_sql_fills_a_placeholder_before_the_engine_sees_the_statement() {
        let server = recording_clickhouse().await;
        let state = state_for(&server, "not a postgres url".to_owned());
        let principal = analyst_principal();
        let args = args_with("SELECT {{region}} AS region", json!({ "region": "EU" }));

        let result = run_sql(&state, Some(&principal), &args).await;

        assert!(result.get("error").is_none(), "{result}");
        let bodies = engine_bodies(&server).await;
        assert_eq!(bodies.len(), 2, "dry run then execution: {bodies:?}");
        for body in &bodies {
            assert!(body.contains("SELECT 'EU' AS region"), "{body}");
            assert!(!body.contains("{{"), "{body}");
        }
    }

    /// A text value is one string literal: a quote inside it is escaped and
    /// cannot close it, on the dry run and on the run.
    #[tokio::test]
    async fn run_sql_prints_a_hostile_text_value_as_one_literal() {
        let server = recording_clickhouse().await;
        let state = state_for(&server, "not a postgres url".to_owned());
        let principal = analyst_principal();
        let args = args_with("SELECT {{v}} AS v", json!({ "v": "x' OR '1'='1" }));

        let result = run_sql(&state, Some(&principal), &args).await;

        assert!(result.get("error").is_none(), "{result}");
        let bodies = engine_bodies(&server).await;
        assert_eq!(bodies.len(), 2, "{bodies:?}");
        for body in &bodies {
            assert!(
                body.contains(r"SELECT 'x\' OR \'1\'=\'1' AS v"),
                "the value must stay in one escaped literal: {body}"
            );
        }
    }

    /// A value that spells a write cannot make the tool write: the tool's own
    /// read-only filter reads the final statement, and it refuses the word
    /// `drop` even inside a literal (as it does for any statement). The
    /// engine receives nothing.
    #[tokio::test]
    async fn run_sql_does_not_let_a_value_make_it_write() {
        let server = recording_clickhouse().await;
        let state = state_for(&server, "not a postgres url".to_owned());
        let principal = analyst_principal();
        let args = args_with("SELECT {{v}} AS v", json!({ "v": "'; DROP TABLE x; --" }));

        let result = run_sql(&state, Some(&principal), &args).await;

        assert_eq!(
            result,
            json!({ "error": "Only a read-only SELECT is allowed." })
        );
        assert!(engine_bodies(&server).await.is_empty(), "nothing is sent");
    }

    /// `query-parameters.md` decision 7: a number the model passes for a placeholder with no
    /// definition is a number, not text. Negative ones are wrapped.
    #[tokio::test]
    async fn run_sql_reads_a_number_for_an_undefined_placeholder_as_a_number() {
        let server = recording_clickhouse().await;
        let state = state_for(&server, "not a postgres url".to_owned());
        let principal = analyst_principal();

        for (value, printed) in [
            (json!(7), "SELECT 7 AS id"),
            (json!(-7), "SELECT (-7) AS id"),
        ] {
            let args = args_with("SELECT {{id}} AS id", json!({ "id": value }));
            let result = run_sql(&state, Some(&principal), &args).await;
            assert!(result.get("error").is_none(), "{result}");
            let bodies = engine_bodies(&server).await;
            let last = bodies.last().expect("the run reached the engine");
            assert!(last.contains(printed), "{last}");
        }
        // A list has no definition to say what it is.
        let args = args_with("SELECT {{id}} AS id", json!({ "id": [1, 2] }));
        let before = engine_bodies(&server).await.len();
        let result = run_sql(&state, Some(&principal), &args).await;
        assert_eq!(
            result,
            json!({ "error": "parameter 'id' is a list: a multi-select needs a saved query's definition." })
        );
        assert_eq!(
            engine_bodies(&server).await.len(),
            before,
            "nothing is sent"
        );
    }

    /// The scanner reads what the model wrote once: a value that itself reads
    /// `{{x}}` is printed in its literal and is not read as a placeholder
    /// when the final statement is run.
    #[tokio::test]
    async fn run_sql_does_not_scan_a_value_that_reads_like_a_placeholder() {
        let server = recording_clickhouse().await;
        let state = state_for(&server, "not a postgres url".to_owned());
        let principal = analyst_principal();
        let args = args_with("SELECT {{v}} AS v", json!({ "v": "{{x}}" }));

        let result = run_sql(&state, Some(&principal), &args).await;

        assert!(result.get("error").is_none(), "{result}");
        let bodies = engine_bodies(&server).await;
        assert_eq!(bodies.len(), 2, "{bodies:?}");
        for body in &bodies {
            assert!(body.contains("SELECT '{{x}}' AS v"), "{body}");
        }
    }

    /// A refusal is the tool's error, in the sentence the person would read,
    /// and the engine receives nothing, not even the dry run.
    #[tokio::test]
    async fn run_sql_returns_a_refusal_as_its_error_and_sends_nothing() {
        let server = recording_clickhouse().await;
        let state = state_for(&server, "not a postgres url".to_owned());
        let principal = analyst_principal();

        for (args, sentence) in [
            (
                // No value and no default.
                args_with("SELECT {{region}} AS region", json!({})),
                "Fill in {{region}} to run this query.",
            ),
            (
                // Inside quotes.
                args_with("SELECT '{{region}}' AS region", json!({ "region": "EU" })),
                "{{region}} is inside quotes: write it without them, for example = {{region}}.",
            ),
            (
                // A shape that is not an object.
                args_with("SELECT {{region}} AS region", json!("EU")),
                "parameters must be an object that maps a parameter name to its value.",
            ),
        ] {
            let result = run_sql(&state, Some(&principal), &args).await;
            assert_eq!(result, json!({ "error": sentence }), "{args:?}");
        }
        assert!(
            engine_bodies(&server).await.is_empty(),
            "the engine received a request for a refused statement"
        );
    }

    /// Without a placeholder and without `parameters` the tool behaves as it
    /// did: the statement reaches the engine byte for byte, and History holds
    /// it as written. With a placeholder, History holds the statement that ran.
    #[sqlx::test(migrations = "../../migrations")]
    async fn run_sql_records_the_statement_that_ran_in_history(pool: PgPool) -> sqlx::Result<()> {
        let server = recording_clickhouse().await;
        let state = state_for(&server, database_url_for(&pool));
        let principal = analyst_principal();

        let mut plain = Map::new();
        plain.insert("sql".to_owned(), json!("SELECT 1 AS one"));
        let result = run_sql(&state, Some(&principal), &plain).await;
        assert!(result.get("error").is_none(), "{result}");
        let result = run_sql(
            &state,
            Some(&principal),
            &args_with("SELECT {{region}} AS region", json!({ "region": "EU" })),
        )
        .await;
        assert!(result.get("error").is_none(), "{result}");

        let history = lakehouse_store::queries::list_history(
            &pool,
            Uuid::nil(),
            &lakehouse_store::queries::HistoryFilter::default(),
        )
        .await
        .expect("query_history is readable");
        let ran: std::collections::BTreeSet<&str> =
            history.iter().map(|h| h.sql.as_str()).collect();
        assert_eq!(
            ran,
            ["SELECT 'EU' AS region", "SELECT 1 AS one"]
                .into_iter()
                .collect()
        );
        Ok(())
    }

    /// The role's row filter and mask govern a parameterised statement: the
    /// rewrite reads the filled statement (a literal in `WHERE email = '…'`), finds
    /// the governed table and masks it. A rewrite over the unfilled text would
    /// not parse, and a rewrite after the fill order would have missed it.
    #[sqlx::test(migrations = "../../migrations")]
    async fn run_sql_masks_a_parameterised_statement_over_a_governed_table(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        lakehouse_store::governance::create_policy(
            &pool,
            &CreatePolicyInput {
                name: "run-sql-parameter-masking-test".to_owned(),
                kind: "Row filter".to_owned(),
                subjects: "Analyst".to_owned(),
                resources: "serving.mart_x".to_owned(),
                effect: "Permit with obligation".to_owned(),
                conditions: Some(
                    r#"{"roles":["Analyst"],"table":"serving.mart_x","mask":["email"]}"#.to_owned(),
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
        env.insert("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT_ID.to_owned());
        let state = AppState::new(Config::from_map(&env).expect("valid config from a map"));
        let principal = analyst_principal();
        let args = args_with(
            "SELECT * FROM serving.mart_x WHERE email = {{who}}",
            json!({ "who": "ana@example.test" }),
        );

        let result = run_sql(&state, Some(&principal), &args).await;

        assert!(result.get("error").is_none(), "{result}");
        let bodies = engine_bodies(&server).await;
        assert!(
            bodies
                .iter()
                .any(|b| b.contains("replaceRegexpOne") && b.contains("'ana@example.test'")),
            "the query the engine ran must be the masked form of the filled statement: {bodies:?}"
        );
        assert!(bodies.iter().all(|b| !b.contains("{{")), "{bodies:?}");
        Ok(())
    }

    /// `DATA-29`: a `NULL` cell reaches the model as `null`, and an empty
    /// cell stays `""` — the tool must not flatten the two again on the way
    /// to the model.
    ///
    /// The tool forwards the run route's rows, so the shape is the route's:
    /// the engine's answer carries one column as `null` and the other as an
    /// empty string, and the model sees both as they came.
    #[tokio::test]
    async fn run_sql_hands_the_model_a_null_cell_as_null() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("EXPLAIN AST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("SelectWithUnionQuery (children 1)\n"),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [
                    {"name": "maybe", "type": "Nullable(String)"},
                    {"name": "text", "type": "String"},
                ],
                "data": [{"maybe": null, "text": ""}],
                "rows": 1,
            })))
            .mount(&server)
            .await;
        let state = state_for(&server, "not a postgres url".to_owned());
        let principal = analyst_principal();
        let mut args = Map::new();
        args.insert("sql".to_owned(), json!("SELECT 1"));
        let result = run_sql(&state, Some(&principal), &args).await;

        assert!(result.get("error").is_none(), "{result}");
        assert_eq!(
            result["rows"],
            json!([{"maybe": null, "text": ""}]),
            "a NULL cell must reach the model as null, and an empty cell as an empty string"
        );
    }

    /// SEC-9-AC4: a statement that fails in the engine comes back to the
    /// model as the trimmed diagnosis with a reference, never the engine
    /// version or an address (SEC-11's query-author exception, reached
    /// through the tool).
    #[sqlx::test(migrations = "../../migrations")]
    async fn run_sql_failure_has_a_reference_and_no_engine_version_or_host(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("EXPLAIN AST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("SelectWithUnionQuery (children 1)\n"),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(404).set_body_string(
                "Code: 47. DB::Exception: Unknown identifier 'nope', reached via \
                 ch-internal.example.net:8123 (192.0.2.7). (UNKNOWN_IDENTIFIER) \
                 (version 24.8.1.1)",
            ))
            .mount(&server)
            .await;
        let state = state_for(&server, database_url_for(&pool));
        let principal = analyst_principal();
        let mut args = Map::new();
        args.insert("sql".to_owned(), json!("SELECT nope"));
        let result = run_sql(&state, Some(&principal), &args).await;

        let text = result.to_string();
        assert!(text.contains("Reference: "), "{text}");
        assert!(text.contains("UNKNOWN_IDENTIFIER"), "{text}");
        for leaked in ["24.8.1.1", "ch-internal.example.net", "192.0.2.7"] {
            assert!(!text.contains(leaked), "{leaked} leaked: {text}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn run_sql_without_a_principal_refuses_with_a_named_reason() {
        // No `ClickHouse` mock is mounted, and `query::run` is never
        // called: same fail-closed shape
        // `tools::queries::run_saved_query_without_a_principal_refuses_with_a_named_reason`
        // proves for the saved-query path — a headless (schedule/service-
        // token) caller has no honest principal to forward.
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
        let state = AppState::new(Config::from_map(&env).expect("valid config from a map"));
        let mut args = Map::new();
        args.insert("sql".to_owned(), json!("SELECT 1"));
        let result = run_sql(&state, None, &args).await;
        assert_eq!(
            result,
            json!({
                "error": "running SQL needs a signed-in user; this call came from a schedule, \
                           which has no user to run as",
            }),
            "expected the honest no-principal refusal, got {result}"
        );
    }
}

#[cfg(test)]
mod describe_mart_marks {
    //! `DATA-12` task 10: the assistant's mart description carries the same
    //! certification fields the picker shows, so it can warn about a
    //! deprecated mart and name the replacement. Real Postgres for the
    //! annotations, a wiremock `ClickHouse` for `system.tables` and
    //! `system.columns` — the same two harnesses `run_sql_delegation`
    //! uses; `ChClient` speaks plain HTTP and has no query-recording
    //! double of its own.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use lakehouse_store::PgPool;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::run_sql_delegation::database_url_for;
    use super::*;
    use crate::config::Config;

    async fn mount_mart_responses(server: &MockServer, marts: Value) {
        Mock::given(method("POST"))
            .and(body_string_contains("system.tables"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [
                    {"name": "name", "type": "String"},
                    {"name": "total_rows", "type": "String"},
                ],
                "data": marts,
                "rows": marts.as_array().map_or(0, Vec::len),
            })))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("system.columns"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [
                    {"name": "name", "type": "String"},
                    {"name": "type", "type": "String"},
                ],
                "data": [
                    {"name": "customer_id", "type": "String"},
                    {"name": "amount", "type": "UInt64"},
                ],
                "rows": 2,
            })))
            .mount(server)
            .await;
    }

    fn state_for(pool: &PgPool, server: &MockServer) -> AppState {
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), database_url_for(pool));
        env.insert("CH_URL".to_owned(), server.uri());
        AppState::new(Config::from_map(&env).expect("valid config from a map"))
    }

    /// A person with every permission. The shared-catalog gate lets an
    /// unrestricted principal through, so these tests read the mark and not
    /// the gate. A headless run is refused before any mark is read (`SEC-76`).
    fn an_unrestricted_person() -> lakehouse_auth::Principal {
        lakehouse_auth::Principal {
            id: lakehouse_auth::PrincipalId::User(uuid::Uuid::from_u128(9)),
            tenant_ids: Vec::new(),
            display_name: "Rina".to_owned(),
            permissions: lakehouse_auth::PermissionSet::parse("*:*"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    async fn mark_old_deprecated(pool: &PgPool) {
        lakehouse_store::annotation::set_certification(
            pool,
            &lakehouse_store::annotation::CertificationInput {
                asset_id: "serving.mart_old".to_owned(),
                status: "deprecated".to_owned(),
                note: Some("Superseded by mart_new".to_owned()),
                replacement_asset_id: Some("serving.mart_new".to_owned()),
                certified_by: "Rina".to_owned(),
            },
        )
        .await
        .expect("seeding the mark must succeed");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn describe_mart_lists_a_marked_mart_with_its_replacement(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        mark_old_deprecated(&pool).await;
        let server = MockServer::start().await;
        mount_mart_responses(
            &server,
            json!([
                {"name": "mart_old", "total_rows": "1000"},
                {"name": "mart_plain", "total_rows": "7"},
            ]),
        )
        .await;
        let state = state_for(&pool, &server);
        let person = an_unrestricted_person();
        let result = describe_mart(&state, Some(&person), &Map::new()).await;
        let marts = result["marts"].as_array().expect("marts array");
        let old = marts
            .iter()
            .find(|m| m["mart"] == json!("mart_old"))
            .expect("mart_old is listed");
        assert_eq!(
            old["certification"],
            json!("deprecated"),
            "expected the mart's mark on the list entry, got {old}"
        );
        assert_eq!(old["certificationNote"], json!("Superseded by mart_new"));
        assert_eq!(old["replacementAssetId"], json!("serving.mart_new"));
        assert_eq!(old["certifiedBy"], json!("Rina"));
        // mart_new is not in this list, so it stays unnamed — never "".
        assert!(
            old.get("replacementName").is_none(),
            "expected no replacementName for a replacement outside the list, got {old}"
        );
        let plain = marts
            .iter()
            .find(|m| m["mart"] == json!("mart_plain"))
            .expect("mart_plain is listed");
        assert!(
            plain.get("certification").is_none(),
            "an unmarked mart gains no key, got {plain}"
        );
        Ok(())
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn describe_mart_of_one_mart_carries_the_mark_and_its_columns(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        mark_old_deprecated(&pool).await;
        let server = MockServer::start().await;
        mount_mart_responses(&server, json!([])).await;
        let state = state_for(&pool, &server);
        let mut args = Map::new();
        args.insert("mart".to_owned(), json!("mart_old"));
        let person = an_unrestricted_person();
        let result = describe_mart(&state, Some(&person), &args).await;
        assert_eq!(result["mart"], json!("mart_old"));
        assert_eq!(result["dimensions"], json!(["customer_id"]));
        assert_eq!(result["measures"], json!(["amount"]));
        assert_eq!(
            result["certification"],
            json!("deprecated"),
            "expected the mark beside the columns, got {result}"
        );
        assert_eq!(result["replacementAssetId"], json!("serving.mart_new"));
        Ok(())
    }
}

#[cfg(test)]
mod read_only_guard {
    use super::*;

    #[test]
    fn is_read_only_sql_allows_with_and_select_only() {
        assert!(is_read_only_sql("SELECT 1"));
        assert!(is_read_only_sql("WITH x AS (SELECT 1) SELECT * FROM x"));
        assert!(!is_read_only_sql("SHOW TABLES"));
        assert!(!is_read_only_sql("EXPLAIN SELECT 1"));
    }

    #[test]
    fn is_read_only_sql_rejects_full_dml_list() {
        for kw in [
            "insert", "alter", "drop", "delete", "update", "create", "truncate", "rename",
            "attach", "detach", "grant", "revoke",
        ] {
            assert!(!is_read_only_sql(&format!("SELECT 1; {kw} x")));
        }
    }
}

#[cfg(test)]
mod truncated_note {
    //! The note `compact_query_result` hands the model when the run route
    //! reports a cut result: it must name the limit that actually cut it.
    //! The run route names that limit (`cutBy`), so a note that blames the
    //! wrong one sends the model after the wrong fix — a byte-cut result
    //! was not cut by a row limit, and the row limit is the server's own,
    //! not the engine's.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// A run response as `routes::query::run` builds it: `truncated` says
    /// whether anything was cut, `cutBy` names the limit that did it and is
    /// absent when the route named none.
    fn run_body(truncated: bool, cut_by: Option<&str>) -> Value {
        let mut body = Map::new();
        body.insert("columns".to_owned(), json!([{"name": "n"}]));
        body.insert("rows".to_owned(), json!([{"n": 1}]));
        body.insert("rowCount".to_owned(), json!(1));
        if truncated {
            body.insert("truncated".to_owned(), json!(true));
        }
        if let Some(cut_by) = cut_by {
            body.insert("cutBy".to_owned(), json!(cut_by));
        }
        Value::Object(body)
    }

    /// The note of a cut result.
    fn note_for(body: Value) -> String {
        let out = compact_query_result(body, None);
        out["truncated"]
            .as_str()
            .expect("a cut result carries the note")
            .to_owned()
    }

    #[test]
    fn a_result_cut_by_the_row_limit_names_the_row_limit() {
        let note = note_for(run_body(true, Some("rows")));
        assert!(note.contains("row limit"), "{note}");
        assert!(note.contains("aggregate in SQL"), "{note}");
        // The row limit is the server's own, so the note must not blame the
        // engine for a cut the server made.
        assert!(!note.contains("engine"), "{note}");
    }

    #[test]
    fn a_result_cut_by_the_byte_cap_names_the_byte_cap() {
        let note = note_for(run_body(true, Some("bytes")));
        assert!(note.contains("10 MB"), "{note}");
        assert!(note.contains("aggregate in SQL"), "{note}");
        // A byte-cut result was not cut by a row limit.
        assert!(!note.contains("row"), "{note}");
    }

    #[test]
    fn a_cut_result_without_a_named_limit_names_no_limit() {
        let note = note_for(run_body(true, None));
        assert!(note.contains("aggregate in SQL"), "{note}");
        // The route named no limit, so the note may not guess one.
        assert!(!note.contains("row"), "{note}");
        assert!(!note.contains("limit"), "{note}");
        assert!(!note.contains("10 MB"), "{note}");
    }

    #[test]
    fn a_result_that_was_not_cut_carries_no_note() {
        let out = compact_query_result(run_body(false, None), None);
        assert!(out.get("truncated").is_none(), "{out}");

        // A response without the field at all is not cut either.
        let mut body = run_body(false, None);
        body.as_object_mut().expect("an object").remove("truncated");
        let out = compact_query_result(body, None);
        assert!(out.get("truncated").is_none(), "{out}");
    }
}

#[cfg(test)]
mod registry_clash {
    //! `DATA-39` D1: `describe_dataset` reads the Bronze column registries
    //! ONCE per column.
    //!
    //! The registry is a `ReplacingMergeTree ORDER BY (slug, key_asli)`
    //! that a re-run writes again, and `ClickHouse` merges the copies only
    //! in the background. Without `FINAL` a re-registered column is one row
    //! per run — and the tool hands the model a LIST, so the duplicate
    //! reaches the answer. A slug in BOTH registries was read twice, with
    //! the secondary's copy answering for a dataset the primary owns.
    //!
    //! These tests run against a REAL `ClickHouse` (the compose file's own
    //! `26.8` tag, through `testcontainers`), because the fix is a
    //! statement: `FINAL` and the `NOT IN (SELECT slug FROM ...)` clause are
    //! the engine's, and a fake that answered them would only prove that
    //! this module asks for what it means to ask for.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_clickhouse::ChClient;

    use super::*;
    use crate::routes::bronze_registry_fixture::{
        clickhouse, pinned, seed_catalog, seed_column_with,
    };

    /// The tool's arguments for one dataset.
    fn args_for(slug: &str) -> Map<String, Value> {
        let mut args = Map::new();
        args.insert("slug".to_owned(), json!(slug));
        args
    }

    /// The columns the tool answered whose name is `name`.
    fn columns_named<'a>(out: &'a Value, name: &str) -> Vec<&'a Value> {
        out["columns"]
            .as_array()
            .unwrap_or_else(|| panic!("the tool answers with columns: {out}"))
            .iter()
            .filter(|column| column["name"] == json!(name))
            .collect()
    }

    /// A column a second run registered is ONE column, from the newer row.
    ///
    /// This is the test the mutation check removes `FINAL` for: without it
    /// the two parts the run wrote answer as two columns of the dataset.
    #[tokio::test]
    async fn a_column_written_twice_to_the_primary_is_listed_once() {
        let (url, _container) = clickhouse().await;
        let ch = ChClient::new(pinned(&url), "default".to_owned(), String::new());

        // The two copies of the row stay apart: no merge may collapse them
        // before the tool reads them.
        ch.exec("SYSTEM STOP MERGES lake.`bronze_meta.dataset_column`", None)
            .await
            .expect("stop the merges of the column table");

        seed_catalog(
            &ch,
            "bronze_meta",
            "twice_columns",
            "Twice columns",
            "twice_columns_raw",
        )
        .await;
        seed_column_with(
            &ch,
            "bronze_meta",
            "twice_columns",
            "amount",
            "UInt64",
            "the first description",
        )
        .await;
        // The same run again, before `ClickHouse` merged the parts.
        seed_column_with(
            &ch,
            "bronze_meta",
            "twice_columns",
            "amount",
            "UInt64",
            "the second description",
        )
        .await;

        let out = describe_dataset_with(
            &ch,
            &crate::table_access::TableAccess::All,
            None,
            &args_for("twice_columns"),
        )
        .await;
        let amount = columns_named(&out, "amount");
        assert_eq!(
            amount.len(),
            1,
            "the column a second run registered is one column: {out}"
        );
        assert_eq!(
            amount[0]["description"],
            json!("the second description"),
            "the newer row's description: {out}"
        );
    }

    /// A slug in BOTH registries answers with the PRIMARY's columns.
    ///
    /// `max_threads=1` makes a `UNION ALL` answer in the order of its
    /// branches (a URL setting, so the statement the module sends is the
    /// statement that runs); the secondary branch is last, so the old
    /// statement reads the secondary's columns for a dataset the primary
    /// owns.
    #[tokio::test]
    async fn a_slug_in_both_registries_answers_with_the_primary_columns() {
        let (url, _container) = clickhouse().await;
        let ch = ChClient::new(pinned(&url), "default".to_owned(), String::new());

        seed_catalog(
            &ch,
            "bronze_meta",
            "clash_columns",
            "Primary clash",
            "clash_columns_raw",
        )
        .await;
        seed_column_with(
            &ch,
            "bronze_meta",
            "clash_columns",
            "id",
            "UInt64",
            "the primary id",
        )
        .await;

        seed_catalog(
            &ch,
            "bronze_meta_sec",
            "clash_columns",
            "Secondary clash",
            "clash_columns_sec_raw",
        )
        .await;
        seed_column_with(
            &ch,
            "bronze_meta_sec",
            "clash_columns",
            "id",
            "String",
            "the secondary id",
        )
        .await;
        seed_column_with(
            &ch,
            "bronze_meta_sec",
            "clash_columns",
            "note",
            "String",
            "the secondary note",
        )
        .await;

        let out = describe_dataset_with(
            &ch,
            &crate::table_access::TableAccess::All,
            None,
            &args_for("clash_columns"),
        )
        .await;
        let id = columns_named(&out, "id");
        assert_eq!(
            id.len(),
            1,
            "the column the two registries both hold is one column: {out}"
        );
        assert_eq!(
            id[0]["description"],
            json!("the primary id"),
            "the primary registry's copy: {out}"
        );
        assert!(
            columns_named(&out, "note").is_empty(),
            "the secondary's own column is not read for a slug the primary \
             holds: {out}"
        );
    }

    /// A dataset only the secondary registry holds still answers: the
    /// clause hides a clash, not the secondary registry's own datasets.
    #[tokio::test]
    async fn a_slug_only_in_the_secondary_registry_still_answers() {
        let (url, _container) = clickhouse().await;
        let ch = ChClient::new(pinned(&url), "default".to_owned(), String::new());

        seed_catalog(
            &ch,
            "bronze_meta_sec",
            "sec_only_columns",
            "Secondary only",
            "sec_only_columns_raw",
        )
        .await;
        seed_column_with(
            &ch,
            "bronze_meta_sec",
            "sec_only_columns",
            "id",
            "UInt64",
            "the secondary id",
        )
        .await;

        let out = describe_dataset_with(
            &ch,
            &crate::table_access::TableAccess::All,
            None,
            &args_for("sec_only_columns"),
        )
        .await;
        let id = columns_named(&out, "id");
        assert_eq!(id.len(), 1, "the secondary registry's own dataset: {out}");
        assert_eq!(
            id[0]["description"],
            json!("the secondary id"),
            "the secondary registry's own dataset: {out}"
        );
    }
}

#[cfg(test)]
mod gov16_quality_summary {
    //! GOV-16: the assistant's quality summary is counted over the tables the
    //! person's role may read, using the same predicate the quality, incident,
    //! schedule and classification lists use. These are pure row-shaping
    //! tests: they need no `ClickHouse`, only the access decision.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use std::collections::HashSet;

    fn narrowed() -> crate::table_access::TableAccess {
        crate::table_access::TableAccess::Listed(
            [("silver".to_owned(), "orders".to_owned())]
                .into_iter()
                .collect(),
            HashSet::new(),
        )
    }

    fn verdict_row(tabel: &str, verdict: &str) -> Map<String, Value> {
        let Value::Object(row) = json!({"tabel": tabel, "verdict": verdict}) else {
            unreachable!("a JSON object literal")
        };
        row
    }

    #[test]
    fn the_summary_counts_only_the_granted_tables() {
        let rows = vec![
            verdict_row("silver.orders", "pass"),
            verdict_row("silver.customers", "fail"),
            verdict_row("orders", "warn"),
            verdict_row("", "fail"),
        ];
        assert_eq!(
            quality_summary(&rows, &narrowed()),
            json!({"summary": [{"verdict": "pass", "n": "1"}]})
        );
        assert_eq!(
            quality_summary(&rows, &crate::table_access::TableAccess::All),
            json!({"summary": [
                {"verdict": "fail", "n": "2"},
                {"verdict": "pass", "n": "1"},
                {"verdict": "warn", "n": "1"},
            ]})
        );
    }
}
