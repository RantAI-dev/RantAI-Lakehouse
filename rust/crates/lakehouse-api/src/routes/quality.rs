//! Running authored quality rules: `POST /api/governance/quality/{id}/run`.
//!
//! A rule (`quality_rule`) says what should hold for a table in a
//! `threshold` people type. Until this module nothing ever evaluated one,
//! so every rule read "not evaluated" forever. A rule is now run on
//! request when its threshold follows a small grammar ([`parse_threshold`]):
//!
//! - `rows >= 1000` — the table's row count (`>=`, `>`, `=`, `<=`, `<`);
//! - `email not null` or `email not null >= 95%` — the share of rows where
//!   the column is set;
//! - `payment_id unique` — no value of the column repeats (within one
//!   load, on a Bronze table — see below);
//! - `amount between 0 and 1000`, `amount >= 0`, `amount <= 1000` — every
//!   set value of the column lies in the range.
//!
//! A threshold that is not in the grammar is not guessed at: the rule is
//! reported as not evaluable, with the grammar as the hint.
//!
//! Bronze is append-only (ADR 0004): loading a source again appends its
//! rows again, and Silver is where they are deduplicated. A uniqueness
//! check across a whole Bronze table would therefore fail from its second
//! load on, whatever the source holds. On a Bronze table that records
//! which load each row came with ([`LOAD_ID_COLUMN`]) the check counts
//! values repeated *within* a load, and its result says so.
//!
//! The check reads the table the rule's `asset` names (`silver.<t>`,
//! `serving.<t>`, `bronze.<t>`) as the platform, not as the caller: a
//! quality verdict is about the data, not about what one role may see of
//! it. Only aggregates come back — a count, a percentage — never rows.
//!
//! Verdicts are appended to `console.quality_run`, a `ClickHouse` table
//! this module owns and creates on first use (the same arrangement as
//! `gold_export_history`); the latest one per rule is what the Data
//! Quality page and an asset's Quality tab show. `quality_rule`'s own
//! `last_status`/`last_run_at` columns stay unread: they hold seeded
//! placeholders that no run ever produced.

use std::collections::HashMap;

use axum::Extension;
use axum::extract::{Path, State};
use lakehouse_auth::Principal;
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ApiError;
use lakehouse_core::ident::{Ident, SqlLiteral};
use lakehouse_store::governance::QualityRule;
use serde_json::{Map, Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::catalog_source::{self, ReadSource, SourceKind};
use crate::routes::lakehouse::is_unknown_table_or_database_error;
use crate::routes::support::{num_or_zero, str_col};
use crate::state::AppState;

/// What to write instead, shown wherever a threshold cannot be evaluated.
pub(crate) const GRAMMAR_HINT: &str = "Write the threshold as one of: \"rows >= 1000\", \
     \"<column> not null >= 95%\", \"<column> unique\", \"<column> between 0 and 100\", \
     \"<column> >= 0\".";

/// A comparison in a row-count or range threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmp {
    Ge,
    Gt,
    Eq,
    Le,
    Lt,
}

impl Cmp {
    fn parse(token: &str) -> Option<Self> {
        match token {
            ">=" => Some(Self::Ge),
            ">" => Some(Self::Gt),
            "=" | "==" => Some(Self::Eq),
            "<=" => Some(Self::Le),
            "<" => Some(Self::Lt),
            _ => None,
        }
    }

    fn sql(self) -> &'static str {
        match self {
            Self::Ge => ">=",
            Self::Gt => ">",
            Self::Eq => "=",
            Self::Le => "<=",
            Self::Lt => "<",
        }
    }

    fn holds(self, left: i64, right: i64) -> bool {
        match self {
            Self::Ge => left >= right,
            Self::Gt => left > right,
            Self::Eq => left == right,
            Self::Le => left <= right,
            Self::Lt => left < right,
        }
    }
}

/// What a threshold asks of the table.
#[derive(Debug, Clone, PartialEq)]
enum Check {
    /// `rows >= 1000`.
    RowCount { cmp: Cmp, rows: i64 },
    /// `email not null >= 95%` (`100` when no percentage is given).
    NotNull { column: Ident, min_percent: f64 },
    /// `payment_id unique`.
    Unique { column: Ident },
    /// `amount between 0 and 100`, `amount >= 0`: every set value
    /// satisfies each bound.
    Range {
        column: Ident,
        bounds: Vec<(Cmp, f64)>,
    },
}

impl Check {
    fn column(&self) -> Option<&Ident> {
        match self {
            Self::RowCount { .. } => None,
            Self::NotNull { column, .. } | Self::Unique { column } | Self::Range { column, .. } => {
                Some(column)
            }
        }
    }
}

/// Splits a threshold into words, numbers and comparison operators, so
/// `">=95%"` and `">= 95 %"` read alike.
fn tokens(threshold: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut op = String::new();
    for c in threshold.chars() {
        let is_op = matches!(c, '<' | '>' | '=');
        if !is_op && !op.is_empty() {
            out.push(std::mem::take(&mut op));
        }
        if (is_op || c.is_whitespace()) && !word.is_empty() {
            out.push(std::mem::take(&mut word));
        }
        if is_op {
            op.push(c);
        } else if !c.is_whitespace() {
            word.push(c);
        }
    }
    out.extend([op, word].into_iter().filter(|t| !t.is_empty()));
    out
}

/// A number as people write one: `1000`, `1_000`, `1,000`, `99.5`.
fn number(token: &str) -> Option<f64> {
    let plain: String = token.chars().filter(|c| !matches!(c, '_' | ',')).collect();
    plain.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// A row count as people write one: `1000`, `1_000`, `1,000` — whole and
/// not negative.
fn whole_number(token: &str) -> Option<i64> {
    let plain: String = token.chars().filter(|c| !matches!(c, '_' | ',')).collect();
    plain.parse::<i64>().ok().filter(|n| *n >= 0)
}

/// Reads a rule's `threshold` as a [`Check`].
///
/// # Errors
///
/// [`GRAMMAR_HINT`] when the text is not one of the forms this evaluates.
fn parse_threshold(threshold: &str) -> Result<Check, &'static str> {
    let tokens = tokens(threshold);
    let words: Vec<&str> = tokens.iter().map(String::as_str).collect();
    let lower: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    let lower: Vec<&str> = lower.iter().map(String::as_str).collect();
    let column = |name: &str| Ident::new(name).map_err(|_| GRAMMAR_HINT);

    match lower.as_slice() {
        ["rows" | "row_count", op, n] => Ok(Check::RowCount {
            cmp: Cmp::parse(op).ok_or(GRAMMAR_HINT)?,
            rows: whole_number(n).ok_or(GRAMMAR_HINT)?,
        }),
        [_, "unique"] => Ok(Check::Unique {
            column: column(words[0])?,
        }),
        [_, "not", "null"] => Ok(Check::NotNull {
            column: column(words[0])?,
            min_percent: 100.0,
        }),
        [_, "not", "null", ">=", p] => {
            let percent = number(p.trim_end_matches('%'))
                .filter(|p| (0.0..=100.0).contains(p))
                .ok_or(GRAMMAR_HINT)?;
            Ok(Check::NotNull {
                column: column(words[0])?,
                min_percent: percent,
            })
        }
        [_, "between", a, "and", b] => {
            let (low, high) = (
                number(a).ok_or(GRAMMAR_HINT)?,
                number(b).ok_or(GRAMMAR_HINT)?,
            );
            Ok(Check::Range {
                column: column(words[0])?,
                bounds: vec![(Cmp::Ge, low), (Cmp::Le, high)],
            })
        }
        [_, op, x] => {
            let cmp = Cmp::parse(op)
                .filter(|c| *c != Cmp::Eq)
                .ok_or(GRAMMAR_HINT)?;
            Ok(Check::Range {
                column: column(words[0])?,
                bounds: vec![(cmp, number(x).ok_or(GRAMMAR_HINT)?)],
            })
        }
        _ => Err(GRAMMAR_HINT),
    }
}

/// The hint for a rule whose threshold this cannot evaluate, `None` for
/// one it can — what the list routes attach to each rule.
pub(crate) fn threshold_hint(threshold: &str) -> Option<&'static str> {
    parse_threshold(threshold).err()
}

/// The longest a check may run before `ClickHouse` stops it.
const MAX_EXECUTION_SECONDS: u32 = 30;

/// The column the ingest sink (dlt) stamps on every Bronze row with the
/// id of the load that wrote it.
const LOAD_ID_COLUMN: &str = "_dlt_load_id";

/// Whether `source` is an append-only Bronze table that records each
/// row's load — where uniqueness is a property of a load, not of the
/// table.
fn keeps_loads(source: &ReadSource) -> bool {
    source.kind == SourceKind::Iceberg
        && source
            .columns
            .iter()
            .any(|(name, _)| name == LOAD_ID_COLUMN)
}

/// The one aggregate query a check needs over `from` (an already-resolved,
/// already-quoted `FROM` target). Every column name in it is an [`Ident`].
/// `per_load` makes a uniqueness check count repeats within each load
/// ([`keeps_loads`]) and return how many `loads` there are.
fn check_sql(check: &Check, from: &str, per_load: bool) -> String {
    let body = match check {
        Check::RowCount { .. } => format!("SELECT toString(count()) AS n FROM {from}"),
        Check::NotNull { column, .. } => format!(
            "SELECT toString(count()) AS n, toString(countIf(isNotNull(`{column}`))) AS k FROM {from}"
        ),
        Check::Unique { column } if per_load => format!(
            "SELECT toString(count()) AS n, \
             toString(uniqExact(`{LOAD_ID_COLUMN}`, `{column}`)) AS k, \
             toString(uniqExact(`{LOAD_ID_COLUMN}`)) AS loads \
             FROM {from} WHERE isNotNull(`{column}`)"
        ),
        Check::Unique { column } => format!(
            "SELECT toString(count()) AS n, toString(uniqExact(`{column}`)) AS k \
             FROM {from} WHERE isNotNull(`{column}`)"
        ),
        Check::Range { column, bounds } => {
            let within: Vec<String> = bounds
                .iter()
                .map(|(cmp, value)| format!("`{column}` {} {value}", cmp.sql()))
                .collect();
            format!(
                "SELECT toString(count()) AS n, toString(countIf(NOT ({}))) AS k \
                 FROM {from} WHERE isNotNull(`{column}`)",
                within.join(" AND ")
            )
        }
    };
    format!("{body} SETTINGS max_execution_time = {MAX_EXECUTION_SECONDS}")
}

/// What a run found: the verdict, and the measurement behind it in words.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Verdict {
    /// `passed | warning | failed`.
    status: &'static str,
    /// The measured value, e.g. `"97.2% not null"`.
    value: String,
}

fn passed_or_failed(ok: bool) -> &'static str {
    if ok { "passed" } else { "failed" }
}

/// Judges the aggregate `row` [`check_sql`] returned. A column check over
/// no rows is a `warning`: nothing was checked, which is neither a pass
/// nor a failure. A uniqueness check counted per load (`loads` is in the
/// row) says so, with how many loads the rows came in.
fn verdict(check: &Check, row: &Map<String, Value>) -> Verdict {
    let n = num_or_zero(Some(row), "n");
    let k = num_or_zero(Some(row), "k");
    if n == 0 && check.column().is_some() {
        return Verdict {
            status: "warning",
            value: "no rows to check".to_owned(),
        };
    }
    match check {
        Check::RowCount { cmp, rows } => Verdict {
            status: passed_or_failed(cmp.holds(n, *rows)),
            value: format!("{n} rows"),
        },
        Check::NotNull { min_percent, .. } => {
            #[allow(
                clippy::cast_precision_loss,
                reason = "a row count as a percentage; exactness past 2^53 rows is irrelevant"
            )]
            let percent = k as f64 / n as f64 * 100.0;
            Verdict {
                status: passed_or_failed(percent >= *min_percent),
                value: format!("{percent:.1}% not null"),
            }
        }
        Check::Unique { .. } => {
            let repeats = n - k;
            let value = if row.contains_key("loads") {
                let loads = num_or_zero(Some(row), "loads");
                let s = if loads == 1 { "" } else { "s" };
                format!("{repeats} repeated values within a load · {n} rows in {loads} load{s}")
            } else {
                format!("{repeats} repeated values in {n} rows")
            };
            Verdict {
                status: passed_or_failed(repeats == 0),
                value,
            }
        }
        Check::Range { .. } => Verdict {
            status: passed_or_failed(k == 0),
            value: format!("{k} of {n} values out of range"),
        },
    }
}

// ── console.quality_run ────────────────────────────────────────────────

static TABLE_ENSURED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn ensure_table(ch: &ChClient) -> Result<(), ChError> {
    TABLE_ENSURED
        .get_or_try_init(|| async {
            ch.exec("CREATE DATABASE IF NOT EXISTS console", None)
                .await?;
            ch.exec(
                "CREATE TABLE IF NOT EXISTS console.quality_run (\n\
                   rule_id String,\n\
                   asset String,\n\
                   status String,\n\
                   value String,\n\
                   run_by String,\n\
                   at DateTime64(3)\n\
                 ) ENGINE = MergeTree ORDER BY (rule_id, at)",
                None,
            )
            .await
        })
        .await
        .map(drop)
}

async fn record_run(
    ch: &ChClient,
    rule: &QualityRule,
    verdict: &Verdict,
    run_by: &str,
) -> Result<(), ChError> {
    ensure_table(ch).await?;
    let sql = format!(
        "INSERT INTO console.quality_run (rule_id, asset, status, value, run_by, at) \
         VALUES ({}, {}, {}, {}, {}, now64(3))",
        SqlLiteral::from(rule.id.as_str()),
        SqlLiteral::from(rule.asset.as_str()),
        SqlLiteral::from(verdict.status),
        SqlLiteral::from(verdict.value.as_str()),
        SqlLiteral::from(run_by),
    );
    ch.exec(&sql, None).await
}

/// The most recent run of one rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LatestRun {
    pub(crate) status: String,
    pub(crate) value: String,
    /// `YYYY-MM-DDTHH:MM:SSZ`.
    pub(crate) at: String,
}

/// The latest run of every rule that has one, by rule id. A table nothing
/// has created yet means no rule has been run, which is the empty map it
/// is; any other failure is logged and reads the same, so a list never
/// fails over its verdict column.
pub(crate) async fn latest_runs(ch: &ChClient) -> HashMap<String, LatestRun> {
    let result = ch
        .rows(
            "SELECT rule_id, argMax(status, at) AS status, argMax(value, at) AS value, \
                    formatDateTime(max(at), '%Y-%m-%dT%H:%i:%SZ', 'UTC') AS ran_at \
             FROM console.quality_run GROUP BY rule_id",
            None,
        )
        .await;
    match result {
        Ok(rows) => rows
            .iter()
            .map(|r| {
                (
                    str_col(r, "rule_id").to_owned(),
                    LatestRun {
                        status: str_col(r, "status").to_owned(),
                        value: str_col(r, "value").to_owned(),
                        at: str_col(r, "ran_at").to_owned(),
                    },
                )
            })
            .collect(),
        Err(ChError::Server(ref body)) if is_unknown_table_or_database_error(body) => {
            HashMap::new()
        }
        Err(err) => {
            tracing::warn!(?err, "quality: latest runs unavailable");
            HashMap::new()
        }
    }
}

/// Puts what running a rule would give, and last gave, onto its JSON
/// (`QualityRule` in `contracts/governance.ts`): `evaluable`, the `hint`
/// when it is not, and — once it has run — `lastStatus`, `lastRunAt` and
/// `lastValue`.
pub(crate) fn annotate_rule(rule: &mut Value, runs: &HashMap<String, LatestRun>) {
    let Some(o) = rule.as_object_mut() else {
        return;
    };
    let hint = o
        .get("threshold")
        .and_then(Value::as_str)
        .and_then(threshold_hint);
    o.insert("evaluable".to_owned(), json!(hint.is_none()));
    o.insert("hint".to_owned(), json!(hint));
    let run = o
        .get("id")
        .and_then(Value::as_str)
        .and_then(|id| runs.get(id));
    if let Some(run) = run {
        o.insert("lastStatus".to_owned(), json!(run.status));
        o.insert("lastRunAt".to_owned(), json!(run.at));
        o.insert("lastValue".to_owned(), json!(run.value));
    }
}

/// The table a rule's `asset` names, when it is one this can read.
async fn resolve(state: &AppState, asset: &str) -> Option<ReadSource> {
    let (zone, table) = asset.trim().split_once('.')?;
    match zone.to_lowercase().as_str() {
        "silver" | "serving" => {
            catalog_source::clickhouse_source(&state.clickhouse, &zone.to_lowercase(), table)
                .await
                .ok()
                .flatten()
        }
        "bronze" => catalog_source::iceberg_source(state, table).await,
        _ => None,
    }
}

/// Why a rule could not be run.
#[derive(Debug)]
enum RunError {
    /// The rule cannot be evaluated as written; the message says how to
    /// fix it. A caller's mistake, not an outage.
    NotRunnable(String),
    /// The check query, or saving its verdict, failed. Already logged.
    Failed(String),
}

/// Runs `rule` against its table now and appends the verdict to
/// `console.quality_run`, as run by `run_by`.
async fn evaluate(state: &AppState, rule: &QualityRule, run_by: &str) -> Result<Verdict, RunError> {
    let check =
        parse_threshold(&rule.threshold).map_err(|hint| RunError::NotRunnable(hint.to_owned()))?;
    let source = resolve(state, &rule.asset).await.ok_or_else(|| {
        RunError::NotRunnable(format!(
            "\"{}\" is not a table this can read. Name the asset as silver.<table>, \
             serving.<table> or bronze.<table>.",
            rule.asset
        ))
    })?;
    if let Some(column) = check.column()
        && !source
            .columns
            .iter()
            .any(|(name, _)| name == column.as_str())
    {
        return Err(RunError::NotRunnable(format!(
            "{} has no column \"{column}\".",
            rule.asset
        )));
    }

    let rows = state
        .clickhouse
        .rows(&check_sql(&check, &source.from, keeps_loads(&source)), None)
        .await
        .map_err(|err| {
            tracing::warn!(?err, rule = %rule.name, "quality: check query failed");
            RunError::Failed(format!(
                "The check could not be run against {}.",
                rule.asset
            ))
        })?;
    let verdict = verdict(&check, rows.first().unwrap_or(&Map::new()));

    // A verdict nobody can see again is not worth returning as if it stuck.
    record_run(&state.clickhouse, rule, &verdict, run_by)
        .await
        .map_err(|err| {
            tracing::warn!(?err, rule = %rule.name, "quality: recording the run failed");
            RunError::Failed("The check ran, but its result could not be saved.".to_owned())
        })?;
    Ok(verdict)
}

/// `POST /api/governance/quality/{id}/run` — evaluate one authored rule
/// now, record the verdict, and return it.
///
/// # Errors
///
/// - `404` when no rule has that id.
/// - `400` when the rule cannot be evaluated as written: a threshold
///   outside the grammar (the message is [`GRAMMAR_HINT`]), an `asset`
///   that is not a readable `silver.`/`serving.`/`bronze.` table, or a
///   column the table does not have.
/// - `503` when the store or the check query fails. The query error
///   itself is logged, not returned.
pub async fn run_rule(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<Value>> {
    let rules = lakehouse_store::governance::list_quality_rules(pool(&state)?).await?;
    let rule = rules
        .iter()
        .find(|r| r.id == id)
        .ok_or_else(|| ApiError::NotFound("Quality rule not found".to_owned()))?;
    let verdict = evaluate(&state, rule, &principal.display_name)
        .await
        .map_err(|err| match err {
            RunError::NotRunnable(why) => ApiError::BadRequest(why),
            RunError::Failed(why) => ApiError::Unavailable(why),
        })?;
    Ok(ApiJson(json!({
        "id": rule.id,
        "status": verdict.status,
        "value": verdict.value,
    })))
}

fn pool(state: &AppState) -> Result<&lakehouse_store::PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "quality rules unavailable: no Postgres pool is configured".to_owned(),
        )
    })
}

/// Forgets every run recorded for `rule`: its verdicts describe a rule
/// that no longer exists, or no longer reads the way it did. Waits for the
/// delete (`mutations_sync`), so the next list does not show a verdict
/// that is already gone. Best-effort: a table nothing created yet, or a
/// failed delete, is logged and left.
async fn forget_runs(ch: &ChClient, rule: &QualityRule) {
    let sql = format!(
        "ALTER TABLE console.quality_run DELETE WHERE rule_id = {} SETTINGS mutations_sync = 1",
        SqlLiteral::from(rule.id.as_str())
    );
    if let Err(err) = ch.exec(&sql, None).await {
        tracing::debug!(?err, rule = %rule.name, "quality: runs of the rule were not removed");
    }
}

/// The `PUT /api/governance/quality/{id}` body.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRuleBody {
    asset: String,
    threshold: String,
    severity: String,
    #[serde(default)]
    dimension: Option<String>,
}

/// `PUT /api/governance/quality/{id}` — rewrite an authored rule: the
/// table it checks, its threshold, its severity. This is how a rule the
/// evaluator cannot read is made one it can, without losing the rule.
///
/// A rule that now checks something else has no verdict: when its table
/// or its threshold changed, the runs recorded for it are forgotten and it
/// reads "not run yet" until it is run again. The answer is the rule as
/// the lists show it, with `evaluable` and the `hint` when it is not.
///
/// # Errors
///
/// `400` when the body is malformed or leaves the table, the threshold or
/// the severity empty; `404` when no rule has that id; `503`/`500` on a
/// store failure.
pub async fn update_rule(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> ApiResult<ApiJson<Value>> {
    let body: UpdateRuleBody = serde_json::from_slice(&body)
        .map_err(|err| ApiError::BadRequest(format!("Invalid request body: {err}")))?;
    let input = lakehouse_store::governance::UpdateQualityRuleInput {
        asset: body.asset.trim().to_owned(),
        threshold: body.threshold.trim().to_owned(),
        severity: body.severity.trim().to_owned(),
        dimension: body
            .dimension
            .map(|d| d.trim().to_owned())
            .filter(|d| !d.is_empty()),
    };
    if input.asset.is_empty() || input.threshold.is_empty() || input.severity.is_empty() {
        return Err(ApiError::BadRequest(
            "A rule needs the table it checks, a threshold and a severity.".to_owned(),
        )
        .into());
    }
    let pool = pool(&state)?;
    let before = lakehouse_store::governance::list_quality_rules(pool)
        .await?
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| ApiError::NotFound("Quality rule not found".to_owned()))?;
    let rule = lakehouse_store::governance::update_quality_rule(pool, &id, &input)
        .await?
        .ok_or_else(|| ApiError::NotFound("Quality rule not found".to_owned()))?;

    let checks_something_else = before.asset != rule.asset || before.threshold != rule.threshold;
    if checks_something_else {
        forget_runs(&state.clickhouse, &rule).await;
    }
    // Which of its parts the rewrite changed — what the history says.
    let changed: Vec<&str> = [
        ("threshold", before.threshold != rule.threshold),
        ("asset", before.asset != rule.asset),
        ("severity", before.severity != rule.severity),
        ("dimension", before.dimension != rule.dimension),
    ]
    .into_iter()
    .filter_map(|(part, differs)| differs.then_some(part))
    .collect();
    // The change belongs to the table the rule checks now, and to the one
    // it stopped checking. A rewrite that changed nothing is not one.
    let mut tables = vec![rule.asset.as_str()];
    if !before.asset.eq_ignore_ascii_case(&rule.asset) {
        tables.push(before.asset.as_str());
    }
    for table in tables.into_iter().filter(|_| !changed.is_empty()) {
        crate::routes::governance::audit_table_change(
            &state,
            Some(&principal),
            table,
            "quality.rule_update",
            json!({
                "name": rule.name,
                "changed": changed,
                "threshold": rule.threshold,
                "asset": rule.asset,
                "severity": rule.severity,
            }),
        )
        .await;
    }

    let runs = if checks_something_else {
        HashMap::new()
    } else {
        latest_runs(&state.clickhouse).await
    };
    let mut out = serde_json::to_value(&rule).unwrap_or(Value::Null);
    annotate_rule(&mut out, &runs);
    Ok(ApiJson(out))
}

/// `DELETE /api/governance/quality/{id}` — remove an authored rule, and
/// the runs recorded for it. The seeded demo rules name tables that do
/// not exist and can never run; before this, nothing could remove a rule.
///
/// # Errors
///
/// `404` when no rule has that id; `503`/`500` on a store failure.
pub async fn delete_rule(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<Value>> {
    let pool = pool(&state)?;
    let rule = lakehouse_store::governance::delete_quality_rule(pool, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Quality rule not found".to_owned()))?;
    forget_runs(&state.clickhouse, &rule).await;
    crate::routes::governance::audit_table_change(
        &state,
        Some(&principal),
        &rule.asset,
        "quality.rule_delete",
        json!({ "name": rule.name }),
    )
    .await;
    Ok(ApiJson(json!({ "ok": true, "id": rule.id })))
}

// ── The scheduled pass ─────────────────────────────────────────────────

/// How often a rule is run on the schedule: at most once in this many
/// minutes. A check is a full aggregate over its table, so it does not
/// run on every 15-minute tick; an hour keeps a verdict current for
/// tables that refresh daily without rescanning them all day.
const SCHEDULED_EVERY_MINUTES: i64 = 60;

/// The most rules one tick runs. The rest wait for the next tick, oldest
/// verdict first.
const SCHEDULED_BATCH: usize = 25;

/// Who the schedule's runs are recorded as.
const SCHEDULE_ACTOR: &str = "schedule";

/// Set while a scheduled pass is running, so a slow pass is not joined by
/// the next tick's.
static PASS_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Clears [`PASS_RUNNING`] when the pass ends, however it ends — a pass
/// that panicked must not stop every later one from starting.
struct PassGuard;

impl Drop for PassGuard {
    fn drop(&mut self) {
        PASS_RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// When the schedule last tried each rule that could not be run (its
/// table is gone, a column was renamed), by rule id, as `due_rules`
/// compares times. Such a rule records no verdict, so without this it
/// would be due — and first in line — on every tick, crowding rules that
/// can run out of the batch. Kept in memory: after a restart each is
/// simply tried once more.
static SKIPPED_AT: std::sync::Mutex<Option<HashMap<String, String>>> = std::sync::Mutex::new(None);

fn skipped_at() -> HashMap<String, String> {
    SKIPPED_AT
        .lock()
        .map(|skipped| skipped.clone().unwrap_or_default())
        .unwrap_or_default()
}

fn remember_skipped(rule_id: &str, at: &str) {
    if let Ok(mut skipped) = SKIPPED_AT.lock() {
        skipped
            .get_or_insert_with(HashMap::new)
            .insert(rule_id.to_owned(), at.to_owned());
    }
}

/// Which rules are due: runnable as written, and never tried or last
/// tried at least [`SCHEDULED_EVERY_MINUTES`] ago — longest-waiting first,
/// at most [`SCHEDULED_BATCH`]. A rule was last tried when it last ran
/// (`runs`) or when the schedule last found it could not be run
/// (`skipped`), whichever is later. Every time is
/// `YYYY-MM-DDTHH:MM:SSZ`, so they compare as text.
fn due_rules<'a>(
    rules: &'a [QualityRule],
    runs: &HashMap<String, LatestRun>,
    skipped: &HashMap<String, String>,
    due_before: &str,
) -> Vec<&'a QualityRule> {
    let mut due: Vec<(&str, &QualityRule)> = rules
        .iter()
        .filter(|r| threshold_hint(&r.threshold).is_none())
        .filter_map(|r| {
            let ran = runs.get(&r.id).map_or("", |run| run.at.as_str());
            let skipped = skipped.get(&r.id).map_or("", String::as_str);
            let last = ran.max(skipped);
            (last < due_before).then_some((last, r))
        })
        .collect();
    due.sort_by_key(|(last, _)| *last);
    due.into_iter()
        .take(SCHEDULED_BATCH)
        .map(|(_, r)| r)
        .collect()
}

/// Runs every due rule once ([`due_rules`]), one after another. A rule
/// that cannot run — its table is gone, a column was renamed — is skipped
/// quietly here: its row already says so wherever it is listed.
async fn scheduled_pass(state: &AppState) {
    let Some(pg) = state.pg.as_deref() else {
        return;
    };
    let rules = match lakehouse_store::governance::list_quality_rules(pg).await {
        Ok(rules) => rules,
        Err(err) => {
            tracing::warn!(?err, "quality schedule: rules unavailable");
            return;
        }
    };
    if rules.is_empty() {
        return;
    }
    let ch = &state.clickhouse;
    // The store's clock, in the form its run times are read back in.
    let clock = match ch
        .rows(
            &format!(
                "SELECT formatDateTime(now(), '%Y-%m-%dT%H:%i:%SZ', 'UTC') AS now, \
                 formatDateTime(now() - INTERVAL {SCHEDULED_EVERY_MINUTES} MINUTE, \
                 '%Y-%m-%dT%H:%i:%SZ', 'UTC') AS t"
            ),
            None,
        )
        .await
    {
        Ok(rows) => rows
            .first()
            .map(|r| (str_col(r, "now").to_owned(), str_col(r, "t").to_owned())),
        Err(err) => {
            tracing::warn!(?err, "quality schedule: clock unavailable");
            None
        }
    };
    let Some((now, due_before)) = clock else {
        return;
    };
    let runs = latest_runs(ch).await;
    let due = due_rules(&rules, &runs, &skipped_at(), &due_before);
    let mut ran = 0;
    for rule in &due {
        match evaluate(state, rule, SCHEDULE_ACTOR).await {
            Ok(_) => ran += 1,
            Err(RunError::NotRunnable(why)) => {
                tracing::debug!(rule = %rule.name, %why, "quality schedule: rule skipped");
                remember_skipped(&rule.id, &now);
            }
            Err(RunError::Failed(_)) => {}
        }
    }
    tracing::info!(due = due.len(), ran, "quality schedule: pass finished");
}

/// Starts a scheduled pass in the background and returns at once — the
/// caller is the platform's evaluation tick (`POST /api/alerts/run`),
/// which must answer its scheduler promptly whatever a table scan takes.
/// Returns `false`, starting nothing, while the previous pass still runs.
pub(crate) fn spawn_scheduled_pass(state: &AppState) -> bool {
    use std::sync::atomic::Ordering;
    if PASS_RUNNING.swap(true, Ordering::SeqCst) {
        return false;
    }
    let state = state.clone();
    tokio::spawn(async move {
        let _running = PassGuard;
        scheduled_pass(&state).await;
    });
    true
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn ident(name: &str) -> Ident {
        Ident::new(name).unwrap()
    }

    fn row(n: &str, k: &str) -> Map<String, Value> {
        let Value::Object(row) = json!({ "n": n, "k": k }) else {
            unreachable!("a JSON object literal")
        };
        row
    }

    /// Every documented form parses, however the operator is spaced.
    #[test]
    fn parses_each_documented_threshold() {
        assert_eq!(
            parse_threshold("rows >= 1,000"),
            Ok(Check::RowCount {
                cmp: Cmp::Ge,
                rows: 1000
            })
        );
        assert_eq!(
            parse_threshold("row_count>0"),
            Ok(Check::RowCount {
                cmp: Cmp::Gt,
                rows: 0
            })
        );
        assert_eq!(
            parse_threshold("payment_id unique"),
            Ok(Check::Unique {
                column: ident("payment_id")
            })
        );
        assert_eq!(
            parse_threshold("email NOT NULL"),
            Ok(Check::NotNull {
                column: ident("email"),
                min_percent: 100.0
            })
        );
        assert_eq!(
            parse_threshold("email not null >=95%"),
            Ok(Check::NotNull {
                column: ident("email"),
                min_percent: 95.0
            })
        );
        assert_eq!(
            parse_threshold("amount between 0 and 1_000"),
            Ok(Check::Range {
                column: ident("amount"),
                bounds: vec![(Cmp::Ge, 0.0), (Cmp::Le, 1000.0)],
            })
        );
        assert_eq!(
            parse_threshold("amount > 0"),
            Ok(Check::Range {
                column: ident("amount"),
                bounds: vec![(Cmp::Gt, 0.0)]
            })
        );
    }

    /// A threshold outside the grammar is not guessed at. That includes
    /// the seeded ">= 95%", which names no column, and anything that is
    /// not a plain identifier where a column goes.
    #[test]
    fn refuses_what_it_cannot_evaluate() {
        for threshold in [
            ">= 95%",
            "null <5%",
            "email not null >= 120%",
            "rows >= many",
            "rows >= 1.5",
            "a; DROP TABLE x unique",
            "`email` unique",
            "",
        ] {
            assert_eq!(parse_threshold(threshold), Err(GRAMMAR_HINT), "{threshold}");
        }
        assert_eq!(threshold_hint("payment_id unique"), None);
        assert_eq!(threshold_hint(">= 95%"), Some(GRAMMAR_HINT));
    }

    #[test]
    fn builds_one_bounded_aggregate_per_check() {
        let from = "silver.`orders`";
        assert_eq!(
            check_sql(&parse_threshold("rows >= 1").unwrap(), from, false),
            "SELECT toString(count()) AS n FROM silver.`orders` SETTINGS max_execution_time = 30"
        );
        let unique = check_sql(&parse_threshold("payment_id unique").unwrap(), from, false);
        assert!(unique.contains("uniqExact(`payment_id`)"));
        assert!(unique.contains("WHERE isNotNull(`payment_id`)"));
        let range = check_sql(
            &parse_threshold("amount between 0 and 100").unwrap(),
            from,
            false,
        );
        assert!(range.contains("countIf(NOT (`amount` >= 0 AND `amount` <= 100))"));
    }

    #[test]
    fn judges_each_check_from_its_aggregate() {
        let rows = parse_threshold("rows >= 1000").unwrap();
        assert_eq!(verdict(&rows, &row("999", "")).status, "failed");
        assert_eq!(
            verdict(&rows, &row("1000", "")),
            Verdict {
                status: "passed",
                value: "1000 rows".to_owned()
            }
        );

        let not_null = parse_threshold("email not null >= 95%").unwrap();
        assert_eq!(
            verdict(&not_null, &row("200", "190")),
            Verdict {
                status: "passed",
                value: "95.0% not null".to_owned()
            }
        );
        assert_eq!(verdict(&not_null, &row("200", "189")).status, "failed");

        let unique = parse_threshold("payment_id unique").unwrap();
        assert_eq!(verdict(&unique, &row("50", "50")).status, "passed");
        assert_eq!(
            verdict(&unique, &row("50", "47")),
            Verdict {
                status: "failed",
                value: "3 repeated values in 50 rows".to_owned()
            }
        );

        let range = parse_threshold("amount >= 0").unwrap();
        assert_eq!(verdict(&range, &row("50", "0")).status, "passed");
        assert_eq!(
            verdict(&range, &row("50", "2")).value,
            "2 of 50 values out of range"
        );
    }

    fn source(kind: SourceKind, columns: &[&str]) -> ReadSource {
        ReadSource {
            from: "icecat_api.`bronze.customers`".to_owned(),
            policy_key: "bronze.customers".to_owned(),
            kind,
            columns: columns
                .iter()
                .map(|c| ((*c).to_owned(), "String".to_owned()))
                .collect(),
        }
    }

    /// Bronze is append-only: a source loaded three times holds each row
    /// three times, by design. Where the table records the load, a value
    /// repeats only when it repeats within one.
    #[test]
    fn uniqueness_on_a_bronze_table_is_counted_within_a_load() {
        let bronze = source(SourceKind::Iceberg, &["customer_id", "_dlt_load_id"]);
        assert!(keeps_loads(&bronze));
        // No load id to group by, or not an append-only table: the whole table.
        assert!(!keeps_loads(&source(SourceKind::Iceberg, &["customer_id"])));
        assert!(!keeps_loads(&source(
            SourceKind::ClickHouse,
            &["customer_id", "_dlt_load_id"]
        )));

        let unique = parse_threshold("customer_id unique").unwrap();
        let sql = check_sql(&unique, &bronze.from, true);
        assert!(sql.contains("toString(uniqExact(`_dlt_load_id`, `customer_id`)) AS k"));
        assert!(sql.contains("toString(uniqExact(`_dlt_load_id`)) AS loads"));
        // Only uniqueness is a per-load property.
        let rows = parse_threshold("rows >= 1").unwrap();
        assert_eq!(
            check_sql(&rows, &bronze.from, true),
            check_sql(&rows, &bronze.from, false)
        );

        let loaded = |n: &str, k: &str, loads: &str| {
            let mut r = row(n, k);
            r.insert("loads".to_owned(), json!(loads));
            r
        };
        assert_eq!(
            verdict(&unique, &loaded("291", "291", "3")),
            Verdict {
                status: "passed",
                value: "0 repeated values within a load · 291 rows in 3 loads".to_owned()
            }
        );
        assert_eq!(
            verdict(&unique, &loaded("97", "95", "1")),
            Verdict {
                status: "failed",
                value: "2 repeated values within a load · 97 rows in 1 load".to_owned()
            }
        );
    }

    /// Nothing to check is neither a pass nor a failure — but an empty
    /// table does fail a row-count rule.
    #[test]
    fn an_empty_table_warns_on_column_checks_and_fails_a_row_count() {
        let unique = parse_threshold("payment_id unique").unwrap();
        assert_eq!(
            verdict(&unique, &row("0", "0")),
            Verdict {
                status: "warning",
                value: "no rows to check".to_owned()
            }
        );
        let rows = parse_threshold("rows > 0").unwrap();
        assert_eq!(verdict(&rows, &row("0", "")).status, "failed");
    }

    fn rule(id: &str, threshold: &str) -> QualityRule {
        QualityRule {
            id: id.to_owned(),
            name: format!("rule_{id}"),
            asset: "silver.orders".to_owned(),
            dimension: "validity".to_owned(),
            threshold: threshold.to_owned(),
            severity: "medium".to_owned(),
            last_status: None,
            last_run_at: None,
        }
    }

    fn ran_at(at: &str) -> LatestRun {
        LatestRun {
            status: "passed".to_owned(),
            value: "1 rows".to_owned(),
            at: at.to_owned(),
        }
    }

    /// The schedule runs what is runnable and stale — never-run first,
    /// then the oldest verdict — and leaves a fresh verdict and a rule
    /// in prose alone.
    #[test]
    fn due_rules_are_the_runnable_ones_without_a_recent_verdict() {
        let rules = [
            rule("fresh", "id unique"),
            rule("stale", "id unique"),
            rule("never", "rows >= 1"),
            rule("prose", ">= 95%"),
            rule("older", "id not null"),
        ];
        let mut runs = HashMap::new();
        runs.insert("fresh".to_owned(), ran_at("2026-10-01T09:30:00Z"));
        runs.insert("stale".to_owned(), ran_at("2026-10-01T08:00:00Z"));
        runs.insert("older".to_owned(), ran_at("2026-09-30T08:00:00Z"));
        runs.insert("prose".to_owned(), ran_at("2026-09-01T00:00:00Z"));

        let due = due_rules(&rules, &runs, &HashMap::new(), "2026-10-01T09:00:00Z");
        let ids: Vec<&str> = due.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["never", "older", "stale"]);
    }

    /// A rule the schedule could not run records no verdict. It is tried
    /// again an hour later like any other, not first on every tick.
    #[test]
    fn a_rule_that_could_not_be_run_waits_its_hour_too() {
        let rules = [rule("gone", "id unique"), rule("stale", "id unique")];
        let mut runs = HashMap::new();
        runs.insert("stale".to_owned(), ran_at("2026-10-01T08:00:00Z"));
        let ids = |skipped: &HashMap<String, String>| -> Vec<String> {
            due_rules(&rules, &runs, skipped, "2026-10-01T09:00:00Z")
                .iter()
                .map(|r| r.id.clone())
                .collect()
        };

        let mut skipped = HashMap::new();
        assert_eq!(ids(&skipped), vec!["gone", "stale"]);
        skipped.insert("gone".to_owned(), "2026-10-01T09:30:00Z".to_owned());
        assert_eq!(ids(&skipped), vec!["stale"]);
        // An hour on, it is due again — after the rule that waited longer.
        skipped.insert("gone".to_owned(), "2026-10-01T08:30:00Z".to_owned());
        assert_eq!(ids(&skipped), vec!["stale", "gone"]);
    }

    /// One tick runs a bounded batch; the rest wait their turn.
    #[test]
    fn due_rules_are_capped_per_tick() {
        let rules: Vec<QualityRule> = (0..SCHEDULED_BATCH + 5)
            .map(|i| rule(&format!("r{i:03}"), "rows >= 1"))
            .collect();
        assert_eq!(
            due_rules(
                &rules,
                &HashMap::new(),
                &HashMap::new(),
                "2026-10-01T09:00:00Z"
            )
            .len(),
            SCHEDULED_BATCH
        );
    }

    /// A rule's JSON says whether it can be run and what it last found;
    /// one that never ran keeps its `null` verdict.
    #[test]
    fn annotate_rule_adds_evaluability_and_the_latest_run() {
        let mut runs = HashMap::new();
        runs.insert(
            "r1".to_owned(),
            LatestRun {
                status: "failed".to_owned(),
                value: "3 repeated values in 50 rows".to_owned(),
                at: "2026-10-01T09:00:00Z".to_owned(),
            },
        );
        let mut ran = json!({ "id": "r1", "threshold": "payment_id unique", "lastStatus": null });
        annotate_rule(&mut ran, &runs);
        assert_eq!(ran["evaluable"], true);
        assert_eq!(ran["hint"], Value::Null);
        assert_eq!(ran["lastStatus"], "failed");
        assert_eq!(ran["lastRunAt"], "2026-10-01T09:00:00Z");
        assert_eq!(ran["lastValue"], "3 repeated values in 50 rows");

        let mut prose = json!({ "id": "r2", "threshold": ">= 95%", "lastStatus": null });
        annotate_rule(&mut prose, &runs);
        assert_eq!(prose["evaluable"], false);
        assert_eq!(prose["hint"], GRAMMAR_HINT);
        assert_eq!(prose["lastStatus"], Value::Null);
    }
}
