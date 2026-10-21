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
//!   set value of the column lies in the range;
//! - `DATA-14` (F1) adds the rest of the ready library, one aggregate and
//!   one comparison each: `email nulls <= 5%` or `nulls <= 10` (null
//!   percent or count), `id duplicates <= 0`, `id distinct >= 3`,
//!   `loaded_at fresh within 6h` (the newest value is no older; `m`, `h`
//!   or `d`), `status in ('new', 'paid')` (no value outside the list;
//!   nulls are not counted), and `min(amount) >= 0`, `max(amount) <= 100`,
//!   `avg(amount) between 10 and 20`.
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
use axum::http::HeaderMap;
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
     \"<column> not null >= 95%\", \"<column> nulls <= 5%\" (or a count: \"nulls <= 10\"), \
     \"<column> unique\", \"<column> duplicates <= 10\", \"<column> distinct >= 3\", \
     \"<column> fresh within 6h\" (m, h or d), \"<column> in ('a', 'b')\" (nulls are not \
     counted), \"min(<column>) >= 0\", \"max(<column>) <= 100\", \
     \"avg(<column>) between 10 and 20\", \"<column> between 0 and 100\", \"<column> >= 0\".";

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

    /// [`Cmp::holds`] for measured decimals (a percent, an average).
    /// Equality is within a rounding error, as a decimal is never exact.
    fn holds_f64(self, left: f64, right: f64) -> bool {
        match self {
            Self::Ge => left >= right,
            Self::Gt => left > right,
            Self::Eq => (left - right).abs() < 1e-9,
            Self::Le => left <= right,
            Self::Lt => left < right,
        }
    }
}

/// What a null limit is measured in.
#[derive(Debug, Clone, Copy, PartialEq)]
enum NullLimit {
    /// `nulls <= 10`.
    Count(i64),
    /// `nulls <= 5%`.
    Percent(f64),
}

/// The statistic of a `min(col)`, `max(col)` or `avg(col)` threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stat {
    Min,
    Max,
    Avg,
}

impl Stat {
    /// The `ClickHouse` aggregate and the word the verdict uses; both constant.
    fn sql(self) -> &'static str {
        match self {
            Self::Min => "min",
            Self::Max => "max",
            Self::Avg => "avg",
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
    /// `email nulls <= 5%`, `email nulls <= 10` (`DATA-14`, F1).
    Nulls {
        column: Ident,
        cmp: Cmp,
        limit: NullLimit,
    },
    /// `payment_id duplicates <= 10`: how many values repeat. `unique` is
    /// the same count held to zero.
    Duplicates { column: Ident, cmp: Cmp, max: i64 },
    /// `status distinct >= 3`: how many different values the column holds.
    Distinct { column: Ident, cmp: Cmp, n: i64 },
    /// `loaded_at fresh within 6h`: the newest value is at most this old.
    Fresh { column: Ident, within_seconds: i64 },
    /// `status in ('new', 'paid')`: no set value outside the list.
    Accepted { column: Ident, values: Vec<String> },
    /// `min(amount) >= 0`, `avg(amount) between 10 and 20`.
    Stat {
        column: Ident,
        stat: Stat,
        bounds: Vec<(Cmp, f64)>,
    },
}

impl Check {
    fn column(&self) -> Option<&Ident> {
        match self {
            Self::RowCount { .. } => None,
            Self::NotNull { column, .. }
            | Self::Unique { column }
            | Self::Range { column, .. }
            | Self::Nulls { column, .. }
            | Self::Duplicates { column, .. }
            | Self::Distinct { column, .. }
            | Self::Fresh { column, .. }
            | Self::Accepted { column, .. }
            | Self::Stat { column, .. } => Some(column),
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

/// The most values one `in (…)` list holds, and the longest one: the list
/// is typed into a threshold, not a lookup table.
const MAX_ACCEPTED_VALUES: usize = 200;
const MAX_ACCEPTED_VALUE_CHARS: usize = 200;

/// `6h`, `30m`, `2d` as seconds; whole, above zero, and small enough that
/// the product cannot overflow.
fn duration_seconds(token: &str) -> Option<i64> {
    let unit = token.chars().last()?;
    let per = match unit.to_ascii_lowercase() {
        'm' => 60,
        'h' => 3_600,
        'd' => 86_400,
        _ => return None,
    };
    let amount = token[..token.len() - unit.len_utf8()].parse::<i64>().ok()?;
    (1..=1_000_000).contains(&amount).then(|| amount * per)
}

/// `nulls <= 5%` or `nulls <= 10`.
fn null_limit(token: &str) -> Option<NullLimit> {
    match token.strip_suffix('%') {
        Some(p) => number(p)
            .filter(|p| (0.0..=100.0).contains(p))
            .map(NullLimit::Percent),
        None => whole_number(token).map(NullLimit::Count),
    }
}

/// The values of `('a', 'b', 3)`: quoted text (a quote inside is doubled,
/// as in SQL) or a bare number, comma-separated. `None` when the list is
/// empty, malformed, or larger than the limits above.
fn accepted_values(list: &str) -> Option<Vec<String>> {
    let inner = list.trim().strip_prefix('(')?.strip_suffix(')')?;
    let mut values: Vec<String> = Vec::new();
    let mut chars = inner.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let mut value = String::new();
        if chars.next_if_eq(&'\'').is_some() {
            loop {
                match chars.next()? {
                    '\'' if chars.next_if_eq(&'\'').is_some() => value.push('\''),
                    '\'' => break,
                    c => value.push(c),
                }
            }
        } else {
            while let Some(c) = chars.next_if(|c| *c != ',') {
                value.push(c);
            }
            let bare = value.trim().to_owned();
            number(&bare)?;
            value = bare;
        }
        if value.chars().count() > MAX_ACCEPTED_VALUE_CHARS {
            return None;
        }
        values.push(value);
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        match chars.next() {
            None => break,
            Some(',') => {}
            Some(_) => return None,
        }
    }
    (values.len() <= MAX_ACCEPTED_VALUES).then_some(values)
}

/// `status in ('new', 'paid')`. `None` when the text is not shaped like an
/// `in` threshold at all, so the other forms get their turn; an error when
/// it is but the list or the column is not valid. The tokenizer cannot
/// read it (a value may hold spaces and commas), so it is read here first.
fn parse_accepted(threshold: &str) -> Option<Result<Check, &'static str>> {
    let text = threshold.trim();
    let (column, rest) = text.split_once(char::is_whitespace)?;
    let rest = rest.trim_start();
    let list = rest.get(..2).filter(|w| w.eq_ignore_ascii_case("in"))?;
    let list = rest[list.len()..].trim_start();
    if !list.starts_with('(') {
        return None;
    }
    Some(accepted_check(column, list))
}

fn accepted_check(column: &str, list: &str) -> Result<Check, &'static str> {
    Ok(Check::Accepted {
        column: Ident::new(column).map_err(|_| GRAMMAR_HINT)?,
        values: accepted_values(list).ok_or(GRAMMAR_HINT)?,
    })
}

/// `min(col)`, `max(col)`, `avg(col)`: the statistic and the column.
fn stat_call(word: &str) -> Option<(Stat, &str)> {
    let lower = word.to_lowercase();
    let stat = match lower.get(..4)? {
        "min(" => Stat::Min,
        "max(" => Stat::Max,
        "avg(" => Stat::Avg,
        _ => return None,
    };
    Some((stat, word.get(4..)?.strip_suffix(')')?))
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

    if let Some(accepted) = parse_accepted(threshold) {
        return accepted;
    }
    // `min(col) >= 0`: one word for the call, so it would otherwise read as
    // a range over a column named `min(col)` and be refused.
    if let Some(&call) = words.first()
        && let Some((stat, name)) = stat_call(call)
    {
        let bounds = match &lower[1..] {
            ["between", a, "and", b] => vec![
                (Cmp::Ge, number(a).ok_or(GRAMMAR_HINT)?),
                (Cmp::Le, number(b).ok_or(GRAMMAR_HINT)?),
            ],
            [op, x] => vec![(
                Cmp::parse(op)
                    .filter(|c| *c != Cmp::Eq)
                    .ok_or(GRAMMAR_HINT)?,
                number(x).ok_or(GRAMMAR_HINT)?,
            )],
            _ => return Err(GRAMMAR_HINT),
        };
        return Ok(Check::Stat {
            column: column(name)?,
            stat,
            bounds,
        });
    }

    match lower.as_slice() {
        ["rows" | "row_count", op, n] => Ok(Check::RowCount {
            cmp: Cmp::parse(op).ok_or(GRAMMAR_HINT)?,
            rows: whole_number(n).ok_or(GRAMMAR_HINT)?,
        }),
        [_, "unique"] => Ok(Check::Unique {
            column: column(words[0])?,
        }),
        [_, "nulls", op, limit] => Ok(Check::Nulls {
            column: column(words[0])?,
            cmp: Cmp::parse(op).ok_or(GRAMMAR_HINT)?,
            limit: null_limit(limit).ok_or(GRAMMAR_HINT)?,
        }),
        [_, "duplicates", op, n] => Ok(Check::Duplicates {
            column: column(words[0])?,
            cmp: Cmp::parse(op).ok_or(GRAMMAR_HINT)?,
            max: whole_number(n).ok_or(GRAMMAR_HINT)?,
        }),
        [_, "distinct", op, n] => Ok(Check::Distinct {
            column: column(words[0])?,
            cmp: Cmp::parse(op).ok_or(GRAMMAR_HINT)?,
            n: whole_number(n).ok_or(GRAMMAR_HINT)?,
        }),
        [_, "fresh", "within", d] => Ok(Check::Fresh {
            column: column(words[0])?,
            within_seconds: duration_seconds(d).ok_or(GRAMMAR_HINT)?,
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
        Check::Unique { column } | Check::Duplicates { column, .. } if per_load => format!(
            "SELECT toString(count()) AS n, \
             toString(uniqExact(`{LOAD_ID_COLUMN}`, `{column}`)) AS k, \
             toString(uniqExact(`{LOAD_ID_COLUMN}`)) AS loads \
             FROM {from} WHERE isNotNull(`{column}`)"
        ),
        Check::Unique { column }
        | Check::Duplicates { column, .. }
        | Check::Distinct { column, .. } => format!(
            "SELECT toString(count()) AS n, toString(uniqExact(`{column}`)) AS k \
             FROM {from} WHERE isNotNull(`{column}`)"
        ),
        // `WHERE 1`: an unqualified count on an Iceberg table can take the
        // metadata-only path that ignores equality deletes (R11,
        // `docs/plans/P5-RESULT.md`); any `WHERE` forces the row scan.
        Check::Nulls { column, .. } => format!(
            "SELECT toString(count()) AS n, toString(countIf(isNull(`{column}`))) AS k \
             FROM {from} WHERE 1"
        ),
        Check::Fresh { column, .. } => format!(
            "SELECT toString(count()) AS n, \
             toString(dateDiff('second', max(`{column}`), now())) AS k \
             FROM {from} WHERE isNotNull(`{column}`)"
        ),
        // DATA-14 (F1): every literal of the list goes through `SqlLiteral`
        // (backslash and quote escaped), the column through `Ident`; the
        // column is compared as text so a number list works on any type.
        Check::Accepted { column, values } => {
            let list: Vec<String> = values
                .iter()
                .map(|v| SqlLiteral::from(v.as_str()).to_string())
                .collect();
            format!(
                "SELECT toString(count()) AS n, \
                 toString(countIf(toString(`{column}`) NOT IN ({}))) AS k \
                 FROM {from} WHERE isNotNull(`{column}`)",
                list.join(", ")
            )
        }
        Check::Stat { column, stat, .. } => format!(
            "SELECT toString(count()) AS n, toString({}(`{column}`)) AS v \
             FROM {from} WHERE isNotNull(`{column}`)",
            stat.sql()
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

/// A null limit judged against `k` nulls in `n` rows.
fn null_verdict(cmp: Cmp, limit: NullLimit, n: i64, k: i64) -> Verdict {
    match limit {
        NullLimit::Count(max) => Verdict {
            status: passed_or_failed(cmp.holds(k, max)),
            value: format!("{k} nulls in {n} rows"),
        },
        NullLimit::Percent(max) => {
            #[allow(
                clippy::cast_precision_loss,
                reason = "a row count as a percentage; exactness past 2^53 rows is irrelevant"
            )]
            let percent = k as f64 / n as f64 * 100.0;
            Verdict {
                status: passed_or_failed(cmp.holds_f64(percent, max)),
                value: format!("{percent:.1}% null ({k} of {n} rows)"),
            }
        }
    }
}

/// A `min`/`max`/`avg` judged against the aggregate `row`. The aggregate
/// came back as text; a column that is not numeric has a minimum or
/// maximum, but not one a bound can judge.
// PR #106 review R1 SHOULD-FIX DATA-14 1 / D4: never return raw cell values in lastValue, run history, or incident detail
fn stat_verdict(stat: Stat, bounds: &[(Cmp, f64)], row: &Map<String, Value>) -> Verdict {
    let measured = str_col(row, "v")
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite());
    match measured {
        Some(v) => {
            let passed = bounds.iter().all(|(cmp, b)| cmp.holds_f64(v, *b));
            Verdict {
                status: passed_or_failed(passed),
                value: if passed {
                    format!("{} within bounds", stat.sql())
                } else {
                    format!("{} outside bounds", stat.sql())
                },
            }
        }
        None => Verdict {
            status: "warning",
            value: format!("the {} of the column is not a number", stat.sql()),
        },
    }
}

/// How long, for a verdict: `45s`, `12m`, `3h 5m`, `2d 4h`.
fn human_seconds(seconds: i64) -> String {
    let s = seconds.max(0);
    if s < 60 {
        format!("{s}s")
    } else if s < 3_600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h {}m", s / 3_600, s % 3_600 / 60)
    } else {
        format!("{}d {}h", s / 86_400, s % 86_400 / 3_600)
    }
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
        Check::Nulls { cmp, limit, .. } => null_verdict(*cmp, *limit, n, k),
        Check::Duplicates { cmp, max, .. } => {
            let repeats = n - k;
            let value = if row.contains_key("loads") {
                let loads = num_or_zero(Some(row), "loads");
                let s = if loads == 1 { "" } else { "s" };
                format!("{repeats} repeated values within a load · {n} rows in {loads} load{s}")
            } else {
                format!("{repeats} repeated values in {n} rows")
            };
            Verdict {
                status: passed_or_failed(cmp.holds(repeats, *max)),
                value,
            }
        }
        Check::Distinct { cmp, n: wanted, .. } => Verdict {
            status: passed_or_failed(cmp.holds(k, *wanted)),
            value: format!("{k} distinct values in {n} rows"),
        },
        Check::Fresh { within_seconds, .. } => Verdict {
            status: passed_or_failed(k <= *within_seconds),
            value: format!(
                "newest value is {} old (limit {})",
                human_seconds(k),
                human_seconds(*within_seconds)
            ),
        },
        Check::Accepted { .. } => Verdict {
            status: passed_or_failed(k == 0),
            value: format!("{k} of {n} values outside the accepted list"),
        },
        Check::Stat { stat, bounds, .. } => stat_verdict(*stat, bounds, row),
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
            .await?;
            // DATA-14 (F2): a kept run says what it checked, since an
            // edited rule's older runs are no longer erased. Rows from
            // before this column read as empty: their threshold is unknown.
            ch.exec(
                "ALTER TABLE console.quality_run ADD COLUMN IF NOT EXISTS threshold String",
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
        "INSERT INTO console.quality_run (rule_id, asset, status, value, run_by, at, threshold) \
         VALUES ({}, {}, {}, {}, {}, now64(3), {})",
        SqlLiteral::from(rule.id.as_str()),
        SqlLiteral::from(rule.asset.as_str()),
        SqlLiteral::from(verdict.status),
        SqlLiteral::from(verdict.value.as_str()),
        SqlLiteral::from(run_by),
        SqlLiteral::from(rule.threshold.as_str()),
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
    /// The table the run checked (`quality_run.asset`).
    pub(crate) asset: String,
    /// What the rule said when it ran; empty for a run from before the
    /// column existed.
    pub(crate) threshold: String,
}

impl LatestRun {
    /// Whether this run is a verdict on the rule as it reads now.
    /// `DATA-14` (F2): an edited rule keeps its older runs as history, but
    /// they describe what it checked before, so they are not its verdict
    /// (principle 2: no stale result under a new threshold). A run from
    /// before `threshold` was recorded cannot be told apart and counts.
    pub(crate) fn describes(&self, asset: &str, threshold: &str) -> bool {
        self.asset.eq_ignore_ascii_case(asset)
            && (self.threshold.is_empty() || self.threshold == threshold)
    }
}

/// The latest run of every rule that has one, by rule id. A table nothing
/// has created yet means no rule has been run, which is the empty map it
/// is; any other failure is logged and reads the same, so a list never
/// fails over its verdict column.
pub(crate) async fn latest_runs(ch: &ChClient) -> HashMap<String, LatestRun> {
    // The table may predate the `threshold` column; add it before reading.
    let result = match ensure_table(ch).await {
        Ok(()) => {
            ch.rows(
                "SELECT rule_id, argMax(status, at) AS status, argMax(value, at) AS value, \
                        argMax(asset, at) AS asset, argMax(threshold, at) AS threshold, \
                        formatDateTime(max(at), '%Y-%m-%dT%H:%i:%SZ', 'UTC') AS ran_at \
                 FROM console.quality_run GROUP BY rule_id",
                None,
            )
            .await
        }
        Err(err) => Err(err),
    };
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
                        asset: str_col(r, "asset").to_owned(),
                        threshold: str_col(r, "threshold").to_owned(),
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

/// How many runs a rule's history lists (`DATA-14`, F7).
const HISTORY_RUNS: usize = 50;

/// One recorded run of a rule, newest first in [`rule_runs`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RunRecord {
    status: String,
    value: String,
    /// Who ran it: a person's display name, `schedule` or `load`.
    run_by: String,
    /// `YYYY-MM-DDTHH:MM:SSZ`.
    at: String,
    /// What the rule said when it ran; `None` for a run from before the
    /// threshold was recorded.
    threshold: Option<String>,
}

/// `GET /api/governance/quality/{id}/runs` — the last 50 runs of one rule,
/// newest first, each with the threshold it ran with. Kept across edits
/// (`DATA-14`, F2), so a rule that was tightened shows both what it found
/// and against what. Readable by any login, like the rule list.
///
/// # Errors
///
/// `404` when no rule has that id; `503` when the store or the run table
/// cannot be read (the cause is logged, not returned).
pub async fn rule_runs(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<Value>> {
    // PR #106 review R1 SHOULD-FIX DATA-14 1 / D4
    if let Some(reason) =
        crate::routes::catalog::catalog_tenant_refusal(&state, &principal, &headers).await?
    {
        return Err(ApiError::PermissionDenied(reason.to_owned()).into());
    }
    let rules = lakehouse_store::governance::list_quality_rules(pool(&state)?).await?;
    let rule = rules
        .iter()
        .find(|r| r.id == id)
        .ok_or_else(|| ApiError::NotFound("Quality rule not found".to_owned()))?;
    let ch = &state.clickhouse;
    ensure_table(ch).await.map_err(|err| {
        tracing::warn!(?err, rule = %rule.name, "quality: the run table is unavailable");
        ApiError::Unavailable("The run history could not be read.".to_owned())
    })?;
    let sql = format!(
        "SELECT status, value, run_by, threshold, \
                formatDateTime(at, '%Y-%m-%dT%H:%i:%SZ', 'UTC') AS ran_at \
         FROM console.quality_run WHERE rule_id = {} ORDER BY at DESC LIMIT {HISTORY_RUNS}",
        SqlLiteral::from(rule.id.as_str())
    );
    let rows = ch.rows(&sql, None).await.map_err(|err| {
        tracing::warn!(?err, rule = %rule.name, "quality: the run history failed");
        ApiError::Unavailable("The run history could not be read.".to_owned())
    })?;
    let runs: Vec<RunRecord> = rows
        .iter()
        .map(|r| RunRecord {
            status: str_col(r, "status").to_owned(),
            value: str_col(r, "value").to_owned(),
            run_by: str_col(r, "run_by").to_owned(),
            at: str_col(r, "ran_at").to_owned(),
            threshold: Some(str_col(r, "threshold").to_owned()).filter(|t| !t.is_empty()),
        })
        .collect();
    Ok(ApiJson(json!({ "runs": runs })))
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
    let (asset, threshold) = (
        o.get("asset").and_then(Value::as_str).unwrap_or_default(),
        o.get("threshold")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    let run = o
        .get("id")
        .and_then(Value::as_str)
        .and_then(|id| runs.get(id))
        .filter(|run| run.describes(asset, threshold));
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
    if verdict.status == "failed" {
        // DATA-14 (F2): a failure opens an incident, kept open until a
        // person resolves it. Bookkeeping after the verdict is saved: if it
        // fails the run still succeeded, and the next failed run tries again.
        crate::routes::quality_incidents::open_for_failed_run(state, rule, &verdict.value).await;
    } else if verdict.status == "passed" {
        // PR #106 review R1 SHOULD-FIX DATA-14 3 / D2: a passing run marks any open incident for this rule stale
        crate::routes::quality_incidents::mark_stale_for_passed_run(state, rule).await;
    }
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

pub(crate) fn pool(state: &AppState) -> Result<&lakehouse_store::PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "quality rules unavailable: no Postgres pool is configured".to_owned(),
        )
    })
}

/// Forgets every run recorded for `rule`: its verdicts describe a rule
/// that no longer exists. Waits for the delete (`mutations_sync`), so the
/// next list does not show a verdict that is already gone. Best-effort: a
/// table nothing created yet, or a failed delete, is logged and left.
///
/// `DATA-14` (F2): called on delete only. An edit used to call it too,
/// which erased the rule's history; edited rules now keep their runs, each
/// with the threshold it ran with ([`LatestRun::describes`] keeps an old
/// run from passing as the new rule's verdict).
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
/// or its threshold changed it reads "not run yet" until it is run again,
/// while the runs recorded for it stay as its history (`DATA-14`, F2). The
/// answer is the rule as the lists show it, with `evaluable` and the `hint`
/// when it is not.
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

    // An older run of a different asset or threshold is not this rule's
    // verdict; `annotate_rule` leaves it out.
    let runs = latest_runs(&state.clickhouse).await;
    let mut out = serde_json::to_value(&rule).unwrap_or(Value::Null);
    annotate_rule(&mut out, &runs);
    Ok(ApiJson(out))
}

/// `DELETE /api/governance/quality/{id}` — remove an authored rule, the
/// runs recorded for it, and resolve its open incidents ("rule deleted",
/// by the deleting user; `DATA-14`, F2). The seeded demo rules name tables that do
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
    crate::routes::quality_incidents::resolve_for_deleted_rule(&state, &rule, &principal).await;
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

/// How often a table's rules run when it has no schedule: at most once in
/// this many minutes. A check is a full aggregate over its table, so it does
/// not run on every 15-minute tick; an hour keeps a verdict current for
/// tables that refresh daily without rescanning them all day. A table can
/// choose another interval, one hour to one day (`routes::quality_schedules`,
/// `DATA-14`).
const SCHEDULED_EVERY_MINUTES: i64 = 60;

/// The most rules one tick runs; the rest wait for the next tick, oldest
/// verdict first. `DATA-14` (F3): 100, up from 25, which capped hourly rules
/// at about 100 per day-tick-cycle (25 every 15 minutes); the pass says in
/// its log how many it left over.
const SCHEDULED_BATCH: usize = 100;

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

/// The rules a tick will run and how many more were due.
#[derive(Debug)]
struct Due<'a> {
    /// At most [`SCHEDULED_BATCH`], longest-waiting first.
    rules: Vec<&'a QualityRule>,
    /// Due but left for the next tick.
    left: usize,
}

/// Parses the `YYYY-MM-DDTHH:MM:SSZ` form the run times are kept in.
fn parse_time(text: &str) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339).ok()
}

/// Which rules are due: runnable as written, and never tried or last tried
/// at least their table's interval ago (`intervals`, minutes by lower-cased
/// table; [`SCHEDULED_EVERY_MINUTES`] for a table with none) — longest-waiting
/// first, at most [`SCHEDULED_BATCH`]. A rule was last tried when it last ran
/// (`runs`, only a run that still describes the rule) or when the schedule
/// last found it could not be run (`skipped`), whichever is later. Every time
/// is `YYYY-MM-DDTHH:MM:SSZ`, so they order as text.
fn due_rules<'a>(
    rules: &'a [QualityRule],
    runs: &HashMap<String, LatestRun>,
    skipped: &HashMap<String, String>,
    intervals: &HashMap<String, i64>,
    now: &str,
) -> Due<'a> {
    let Some(now) = parse_time(now) else {
        return Due {
            rules: Vec::new(),
            left: 0,
        };
    };
    let mut due: Vec<(&str, &QualityRule)> = rules
        .iter()
        .filter(|r| threshold_hint(&r.threshold).is_none())
        .filter_map(|r| {
            let ran = runs
                .get(&r.id)
                .filter(|run| run.describes(&r.asset, &r.threshold))
                .map_or("", |run| run.at.as_str());
            let skipped = skipped.get(&r.id).map_or("", String::as_str);
            let last = ran.max(skipped);
            let minutes = intervals
                .get(&r.asset.trim().to_lowercase())
                .copied()
                .unwrap_or(SCHEDULED_EVERY_MINUTES);
            // Never tried (or a time that cannot be read) is due at once.
            let waited_enough =
                parse_time(last).is_none_or(|at| at + time::Duration::minutes(minutes) <= now);
            waited_enough.then_some((last, r))
        })
        .collect();
    due.sort_by_key(|(last, _)| *last);
    let left = due.len().saturating_sub(SCHEDULED_BATCH);
    Due {
        rules: due
            .into_iter()
            .take(SCHEDULED_BATCH)
            .map(|(_, r)| r)
            .collect(),
        left,
    }
}

/// Runs every due rule once ([`due_rules`]), one after another. A rule
/// that cannot run — its table is gone, a column was renamed — is skipped
/// quietly here: its row already says so wherever it is listed.
async fn scheduled_pass(state: &AppState) {
    let Some(pg) = state.pg.as_deref() else {
        return;
    };
    // DATA-14 (F5): load-history upkeep and the freshness anomalies ride the
    // same tick; they read Postgres only and never stop the rules below.
    crate::routes::quality_anomaly::tick(state).await;
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
    // A schedule that cannot be read must not be guessed at: a daily table
    // would be run hourly. Wait for the next tick.
    let intervals: HashMap<String, i64> = match lakehouse_store::quality::list_schedules(pg).await {
        Ok(schedules) => schedules
            .into_iter()
            .map(|s| (s.asset, i64::from(s.interval_minutes)))
            .collect(),
        Err(err) => {
            tracing::warn!(?err, "quality schedule: schedules unavailable");
            return;
        }
    };
    let ch = &state.clickhouse;
    // The store's clock, in the form its run times are read back in.
    let clock = match ch
        .rows(
            "SELECT formatDateTime(now(), '%Y-%m-%dT%H:%i:%SZ', 'UTC') AS now",
            None,
        )
        .await
    {
        Ok(rows) => rows.first().map(|r| str_col(r, "now").to_owned()),
        Err(err) => {
            tracing::warn!(?err, "quality schedule: clock unavailable");
            None
        }
    };
    let Some(now) = clock else {
        return;
    };
    let runs = latest_runs(ch).await;
    let due = due_rules(&rules, &runs, &skipped_at(), &intervals, &now);
    let mut ran = 0;
    for rule in &due.rules {
        match evaluate(state, rule, SCHEDULE_ACTOR).await {
            Ok(_) => ran += 1,
            Err(RunError::NotRunnable(why)) => {
                tracing::debug!(rule = %rule.name, %why, "quality schedule: rule skipped");
                remember_skipped(&rule.id, &now);
            }
            Err(RunError::Failed(_)) => {}
        }
    }
    tracing::info!(
        due = due.rules.len(),
        left_for_next_tick = due.left,
        ran,
        "quality schedule: pass finished"
    );
}

/// The table `asset` names as it holds now: its row count, for the load
/// history (`DATA-14`, F5). `None` when the table cannot be read or counted;
/// the reason is logged, and a load whose size is not known is not recorded
/// (no invented count).
pub(crate) async fn count_rows(state: &AppState, asset: &str) -> Option<i64> {
    let source = resolve(state, asset).await?;
    // `WHERE 1`: see `check_sql` on counting an Iceberg table (R11).
    let sql = format!(
        "SELECT toString(count()) AS n FROM {} WHERE 1 SETTINGS max_execution_time = {MAX_EXECUTION_SECONDS}",
        source.from
    );
    match state.clickhouse.rows(&sql, None).await {
        Ok(rows) => rows
            .first()
            .and_then(|r| r.get("n"))
            .and_then(Value::as_str)
            .and_then(|n| n.parse::<i64>().ok()),
        Err(err) => {
            tracing::warn!(?err, %asset, "quality: the table's rows could not be counted");
            None
        }
    }
}

/// Who the runs started by a load are recorded as.
const LOAD_ACTOR: &str = "load";

/// Runs the rules of `tables` (lower-cased `zone.table` keys) whose schedule
/// is `on_load`, once each, as the load that just finished (`DATA-14`, F3 and
/// F4). Called from `routes::quality_load` after the load is confirmed; a
/// table with no schedule, or an `interval` one, is left to the tick. Returns
/// how many rules ran. A rule that cannot be run is skipped quietly (its row
/// already says so), and nothing here can fail the event that started it.
pub(crate) async fn run_on_load(state: &AppState, tables: &[String]) -> usize {
    let Some(pg) = state.pg.as_deref() else {
        return 0;
    };
    let on_load: std::collections::HashSet<String> =
        match lakehouse_store::quality::list_schedules(pg).await {
            Ok(schedules) => schedules
                .into_iter()
                .filter(|s| s.mode == lakehouse_store::quality::ScheduleMode::OnLoad.as_str())
                .map(|s| s.asset)
                .filter(|asset| tables.contains(asset))
                .collect(),
            Err(err) => {
                tracing::warn!(?err, "quality on load: schedules unavailable");
                return 0;
            }
        };
    if on_load.is_empty() {
        return 0;
    }
    let rules = match lakehouse_store::governance::list_quality_rules(pg).await {
        Ok(rules) => rules,
        Err(err) => {
            tracing::warn!(?err, "quality on load: rules unavailable");
            return 0;
        }
    };
    let mut ran = 0;
    for rule in rules
        .iter()
        .filter(|r| on_load.contains(&r.asset.trim().to_lowercase()))
    {
        match evaluate(state, rule, LOAD_ACTOR).await {
            Ok(_) => ran += 1,
            Err(RunError::NotRunnable(why)) => {
                tracing::debug!(rule = %rule.name, %why, "quality on load: rule skipped");
            }
            Err(RunError::Failed(_)) => {}
        }
    }
    tracing::info!(
        tables = on_load.len(),
        ran,
        "quality on load: pass finished"
    );
    ran
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
            asset: "silver.orders".to_owned(),
            threshold: String::new(),
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

        // The tick's clock: an hour after the 09:00 the verdicts are held to.
        let due = due_rules(
            &rules,
            &runs,
            &HashMap::new(),
            &HashMap::new(),
            "2026-10-01T10:00:00Z",
        );
        let ids: Vec<&str> = due.rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["never", "older", "stale"]);
        assert_eq!(due.left, 0);
    }

    /// A rule the schedule could not run records no verdict. It is tried
    /// again an hour later like any other, not first on every tick.
    #[test]
    fn a_rule_that_could_not_be_run_waits_its_hour_too() {
        let rules = [rule("gone", "id unique"), rule("stale", "id unique")];
        let mut runs = HashMap::new();
        runs.insert("stale".to_owned(), ran_at("2026-10-01T08:00:00Z"));
        let ids = |skipped: &HashMap<String, String>| -> Vec<String> {
            due_rules(
                &rules,
                &runs,
                skipped,
                &HashMap::new(),
                "2026-10-01T10:00:00Z",
            )
            .rules
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

    /// One tick runs a bounded batch (`DATA-14`, F3: 100, was 25 — the old
    /// test pinned 25 by constant and now pins the new bound the same way);
    /// the rest wait their turn and are counted.
    #[test]
    fn due_rules_are_capped_per_tick_and_the_rest_are_counted() {
        assert_eq!(SCHEDULED_BATCH, 100);
        let rules: Vec<QualityRule> = (0..SCHEDULED_BATCH + 5)
            .map(|i| rule(&format!("r{i:03}"), "rows >= 1"))
            .collect();
        let due = due_rules(
            &rules,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            "2026-10-01T10:00:00Z",
        );
        assert_eq!(due.rules.len(), SCHEDULED_BATCH);
        assert_eq!(due.left, 5);
    }

    /// A table runs at its own interval; one with no schedule, hourly.
    #[test]
    fn a_table_is_due_at_its_own_interval() {
        let mut daily = rule("daily", "rows >= 1");
        daily.asset = "Silver.Daily".to_owned();
        let hourly = rule("hourly", "rows >= 1");
        let rules = [daily, hourly];
        let mut runs = HashMap::new();
        let mut ran = |id: &str, at: &str, asset: &str| {
            let mut run = ran_at(at);
            run.asset = asset.to_owned();
            runs.insert(id.to_owned(), run);
        };
        ran("daily", "2026-10-01T09:00:00Z", "Silver.Daily");
        ran("hourly", "2026-10-01T09:00:00Z", "silver.orders");
        let intervals = HashMap::from([("silver.daily".to_owned(), 1440_i64)]);
        let ids = |now: &str| -> Vec<String> {
            due_rules(&rules, &runs, &HashMap::new(), &intervals, now)
                .rules
                .iter()
                .map(|r| r.id.clone())
                .collect()
        };
        // 59 minutes on: neither. An hour on: the hourly one only. A day on: both.
        assert!(ids("2026-10-01T09:59:59Z").is_empty());
        assert_eq!(ids("2026-10-01T10:00:00Z"), vec!["hourly"]);
        assert_eq!(ids("2026-10-02T08:59:59Z"), vec!["hourly"]);
        assert_eq!(ids("2026-10-02T09:00:00Z"), vec!["daily", "hourly"]);
        // A clock that cannot be read runs nothing rather than everything.
        assert!(ids("not a time").is_empty());
    }

    /// Longest-waiting first, across tables with different intervals.
    #[test]
    fn the_oldest_due_rule_goes_first() {
        let rules = [rule("a", "rows >= 1"), rule("b", "rows >= 1")];
        let mut runs = HashMap::new();
        runs.insert("a".to_owned(), ran_at("2026-10-01T07:00:00Z"));
        runs.insert("b".to_owned(), ran_at("2026-10-01T06:00:00Z"));
        let due = due_rules(
            &rules,
            &runs,
            &HashMap::new(),
            &HashMap::new(),
            "2026-10-01T10:00:00Z",
        );
        let ids: Vec<&str> = due.rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["b", "a"]);
    }

    #[test]
    fn the_default_interval_is_the_stores_default() {
        assert_eq!(
            SCHEDULED_EVERY_MINUTES,
            i64::from(lakehouse_store::quality::DEFAULT_INTERVAL_MINUTES)
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
                asset: "silver.orders".to_owned(),
                threshold: "payment_id unique".to_owned(),
            },
        );
        let mut ran = json!({
            "id": "r1", "asset": "silver.orders", "threshold": "payment_id unique", "lastStatus": null
        });
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

    /// `DATA-14` (F2): an edited rule keeps its older runs as history, but
    /// they are not its verdict, and it is due again at once.
    #[test]
    fn a_run_of_an_earlier_threshold_is_not_the_edited_rules_result() {
        let mut runs = HashMap::new();
        runs.insert(
            "r1".to_owned(),
            LatestRun {
                status: "failed".to_owned(),
                value: "3 repeated values in 50 rows".to_owned(),
                at: "2026-10-01T09:30:00Z".to_owned(),
                asset: "silver.orders".to_owned(),
                threshold: "payment_id unique".to_owned(),
            },
        );
        // Same threshold, and the asset compared without case: its result.
        let mut same =
            json!({ "id": "r1", "asset": "Silver.Orders", "threshold": "payment_id unique" });
        annotate_rule(&mut same, &runs);
        assert_eq!(same["lastStatus"], "failed");
        // Edited threshold, or a different table: no result under the new rule.
        for edited in [
            json!({ "id": "r1", "asset": "silver.orders", "threshold": "payment_id duplicates <= 5" }),
            json!({ "id": "r1", "asset": "silver.payments", "threshold": "payment_id unique" }),
        ] {
            let mut edited = edited;
            annotate_rule(&mut edited, &runs);
            assert!(edited.get("lastStatus").is_none(), "{edited}");
            assert_eq!(edited["evaluable"], true);
        }
        // A run from before the threshold was recorded cannot be told apart.
        runs.get_mut("r1").unwrap().threshold = String::new();
        let mut legacy = json!({ "id": "r1", "asset": "silver.orders", "threshold": "anything" });
        annotate_rule(&mut legacy, &runs);
        assert_eq!(legacy["lastStatus"], "failed");

        // Due: the edited rule is due though its old run is recent.
        runs.get_mut("r1").unwrap().threshold = "payment_id unique".to_owned();
        let edited_rule = QualityRule {
            threshold: "payment_id duplicates <= 5".to_owned(),
            ..rule("r1", "payment_id unique")
        };
        let rules = [edited_rule];
        let due = due_rules(
            &rules,
            &runs,
            &HashMap::new(),
            &HashMap::new(),
            "2026-10-01T10:00:00Z",
        );
        assert_eq!(due.rules.len(), 1);
    }

    // ── DATA-14 (F1): the rest of the ready library ────────────────────

    /// Each added form parses to the check it names; the older forms are
    /// untouched (the tests above still pin them).
    #[test]
    fn parses_each_added_check() {
        assert_eq!(
            parse_threshold("email nulls <= 5%"),
            Ok(Check::Nulls {
                column: ident("email"),
                cmp: Cmp::Le,
                limit: NullLimit::Percent(5.0)
            })
        );
        assert_eq!(
            parse_threshold("email NULLS<10"),
            Ok(Check::Nulls {
                column: ident("email"),
                cmp: Cmp::Lt,
                limit: NullLimit::Count(10)
            })
        );
        assert_eq!(
            parse_threshold("id duplicates <= 3"),
            Ok(Check::Duplicates {
                column: ident("id"),
                cmp: Cmp::Le,
                max: 3
            })
        );
        assert_eq!(
            parse_threshold("status distinct >= 3"),
            Ok(Check::Distinct {
                column: ident("status"),
                cmp: Cmp::Ge,
                n: 3
            })
        );
        assert_eq!(
            parse_threshold("loaded_at fresh within 6h"),
            Ok(Check::Fresh {
                column: ident("loaded_at"),
                within_seconds: 6 * 3_600
            })
        );
        assert_eq!(
            parse_threshold("loaded_at fresh within 30m"),
            Ok(Check::Fresh {
                column: ident("loaded_at"),
                within_seconds: 30 * 60
            })
        );
        assert_eq!(
            parse_threshold("loaded_at FRESH WITHIN 2d"),
            Ok(Check::Fresh {
                column: ident("loaded_at"),
                within_seconds: 2 * 86_400
            })
        );
        assert_eq!(
            parse_threshold("min(amount) >= 0"),
            Ok(Check::Stat {
                column: ident("amount"),
                stat: Stat::Min,
                bounds: vec![(Cmp::Ge, 0.0)]
            })
        );
        assert_eq!(
            parse_threshold("MAX(amount) < 1_000"),
            Ok(Check::Stat {
                column: ident("amount"),
                stat: Stat::Max,
                bounds: vec![(Cmp::Lt, 1000.0)]
            })
        );
        assert_eq!(
            parse_threshold("avg(amount) between 10 and 20.5"),
            Ok(Check::Stat {
                column: ident("amount"),
                stat: Stat::Avg,
                bounds: vec![(Cmp::Ge, 10.0), (Cmp::Le, 20.5)]
            })
        );
    }

    /// A value may hold spaces, commas and a doubled quote; a bare number
    /// is a value too.
    #[test]
    fn parses_an_accepted_values_list() {
        assert_eq!(
            parse_threshold("status in ('new', 'paid', 'on hold, again', 'it''s', 3)"),
            Ok(Check::Accepted {
                column: ident("status"),
                values: vec![
                    "new".to_owned(),
                    "paid".to_owned(),
                    "on hold, again".to_owned(),
                    "it's".to_owned(),
                    "3".to_owned()
                ]
            })
        );
        assert_eq!(
            parse_threshold("status IN('a')"),
            Ok(Check::Accepted {
                column: ident("status"),
                values: vec!["a".to_owned()]
            })
        );
    }

    #[test]
    fn refuses_a_malformed_added_check() {
        for threshold in [
            "email nulls <= 120%",
            "email nulls <= -1",
            "email nulls <= many",
            "email nulls",
            "id duplicates <= 1.5",
            "id distinct >=",
            "loaded_at fresh within 0h",
            "loaded_at fresh within 5s",
            "loaded_at fresh within h",
            "loaded_at fresh within 99999999d",
            "status in ()",
            "status in ('a',)",
            "status in ('a' 'b')",
            "status in (a)",
            "status in ('a'",
            "status in 'a'",
            "a;b in ('x')",
            "min(amount) = 0",
            "min(amount)",
            "min(a b) >= 0",
            "min(a;b) >= 0",
            "median(amount) >= 0",
        ] {
            assert_eq!(parse_threshold(threshold), Err(GRAMMAR_HINT), "{threshold}");
        }
        // A column that merely starts with "in" is not an `in` list.
        assert_eq!(
            parse_threshold("inbox unique"),
            Ok(Check::Unique {
                column: ident("inbox")
            })
        );
    }

    /// The list is bounded: a threshold is not a lookup table.
    #[test]
    fn an_accepted_list_has_a_size_limit() {
        let many: Vec<String> = (0..=MAX_ACCEPTED_VALUES)
            .map(|i| format!("'v{i}'"))
            .collect();
        assert_eq!(
            parse_threshold(&format!("status in ({})", many.join(", "))),
            Err(GRAMMAR_HINT)
        );
        let long = "x".repeat(MAX_ACCEPTED_VALUE_CHARS + 1);
        assert_eq!(
            parse_threshold(&format!("status in ('{long}')")),
            Err(GRAMMAR_HINT)
        );
    }

    #[test]
    fn builds_one_bounded_aggregate_for_each_added_check() {
        let from = "silver.`orders`";
        let sql = |t: &str| check_sql(&parse_threshold(t).unwrap(), from, false);
        assert_eq!(
            sql("email nulls <= 5%"),
            "SELECT toString(count()) AS n, toString(countIf(isNull(`email`))) AS k \
             FROM silver.`orders` WHERE 1 SETTINGS max_execution_time = 30"
        );
        assert!(sql("id duplicates <= 2").contains("uniqExact(`id`)"));
        assert!(sql("id duplicates <= 2").contains("WHERE isNotNull(`id`)"));
        assert!(sql("status distinct >= 2").contains("toString(uniqExact(`status`)) AS k"));
        assert!(
            sql("ts fresh within 1h")
                .contains("toString(dateDiff('second', max(`ts`), now())) AS k")
        );
        assert!(sql("min(amount) >= 0").contains("toString(min(`amount`)) AS v"));
        assert!(sql("max(amount) >= 0").contains("toString(max(`amount`)) AS v"));
        assert!(sql("avg(amount) >= 0").contains("toString(avg(`amount`)) AS v"));
        // Per load, a duplicates check counts like `unique` does.
        let per_load = check_sql(&parse_threshold("id duplicates <= 2").unwrap(), from, true);
        assert!(per_load.contains("uniqExact(`_dlt_load_id`, `id`)"));
    }

    /// Every literal of the list is escaped the way `SqlLiteral` escapes
    /// it, so a value cannot end the string and run SQL.
    #[test]
    fn accepted_values_are_escaped_into_the_list() {
        let check = parse_threshold("status in ('ok', 'x'') OR 1=1 --', 'a\\b')").unwrap();
        let sql = check_sql(&check, "silver.`orders`", false);
        let expected = ["ok", "x') OR 1=1 --", "a\\b"]
            .map(|v| SqlLiteral::from(v).to_string())
            .join(", ");
        assert!(
            sql.contains(&format!("toString(`status`) NOT IN ({expected})")),
            "{sql}"
        );
        // The injected quote is doubled, so the text stays inside the literal.
        assert!(sql.contains("'x'') OR 1=1 --'"), "{sql}");
    }

    #[test]
    fn judges_each_added_check_from_its_aggregate() {
        let verdict_of = |t: &str, r: Map<String, Value>| verdict(&parse_threshold(t).unwrap(), &r);

        // Null count and percent.
        assert_eq!(
            verdict_of("email nulls <= 10", row("200", "10")),
            Verdict {
                status: "passed",
                value: "10 nulls in 200 rows".to_owned()
            }
        );
        assert_eq!(
            verdict_of("email nulls <= 10", row("200", "11")).status,
            "failed"
        );
        assert_eq!(
            verdict_of("email nulls <= 5%", row("200", "10")),
            Verdict {
                status: "passed",
                value: "5.0% null (10 of 200 rows)".to_owned()
            }
        );
        assert_eq!(
            verdict_of("email nulls <= 5%", row("200", "11")).status,
            "failed"
        );
        assert_eq!(
            verdict_of("email nulls <= 5%", row("0", "0")).status,
            "warning"
        );

        // Duplicates: the same count `unique` holds to zero.
        assert_eq!(
            verdict_of("id duplicates <= 3", row("50", "47")),
            Verdict {
                status: "passed",
                value: "3 repeated values in 50 rows".to_owned()
            }
        );
        assert_eq!(
            verdict_of("id duplicates <= 2", row("50", "47")).status,
            "failed"
        );

        // Distinct.
        assert_eq!(
            verdict_of("status distinct >= 3", row("50", "3")),
            Verdict {
                status: "passed",
                value: "3 distinct values in 50 rows".to_owned()
            }
        );
        assert_eq!(
            verdict_of("status distinct >= 3", row("50", "2")).status,
            "failed"
        );
    }

    #[test]
    fn judges_freshness_values_and_statistics_from_their_aggregate() {
        let verdict_of = |t: &str, r: Map<String, Value>| verdict(&parse_threshold(t).unwrap(), &r);

        // Freshness: `k` is the age of the newest value in seconds.
        assert_eq!(
            verdict_of("ts fresh within 6h", row("50", "7200")),
            Verdict {
                status: "passed",
                value: "newest value is 2h 0m old (limit 6h 0m)".to_owned()
            }
        );
        assert_eq!(
            verdict_of("ts fresh within 6h", row("50", "21601")).status,
            "failed"
        );
        assert_eq!(
            verdict_of("ts fresh within 1h", row("0", "0")).status,
            "warning"
        );

        // Accepted values.
        assert_eq!(
            verdict_of("status in ('a', 'b')", row("50", "0")),
            Verdict {
                status: "passed",
                value: "0 of 50 values outside the accepted list".to_owned()
            }
        );
        assert_eq!(
            verdict_of("status in ('a', 'b')", row("50", "4")).status,
            "failed"
        );

        // Min, max, average.
        let stat = |t: &str, v: &str| {
            let mut r = row("50", "");
            r.insert("v".to_owned(), json!(v));
            verdict_of(t, r)
        };
        // PR #106 review R1 SHOULD-FIX DATA-14 1
        assert_eq!(
            stat("min(amount) >= 0", "0"),
            Verdict {
                status: "passed",
                value: "min within bounds".to_owned()
            }
        );
        assert_eq!(stat("min(amount) >= 0", "-2.5").status, "failed");
        assert_eq!(stat("max(amount) <= 100", "100").status, "passed");
        assert_eq!(stat("max(amount) <= 100", "100.5").status, "failed");
        assert_eq!(
            stat("avg(amount) between 10 and 20", "15.25").status,
            "passed"
        );
        assert_eq!(stat("avg(amount) between 10 and 20", "21").status, "failed");
        // A column that is not numeric has no average a bound can judge.
        assert_eq!(
            stat("max(amount) <= 100", "zebra"),
            Verdict {
                status: "warning",
                value: "the max of the column is not a number".to_owned()
            }
        );
    }

    #[test]
    fn says_how_old_in_words() {
        assert_eq!(human_seconds(-5), "0s");
        assert_eq!(human_seconds(59), "59s");
        assert_eq!(human_seconds(60), "1m");
        assert_eq!(human_seconds(3_600 + 5 * 60), "1h 5m");
        assert_eq!(human_seconds(2 * 86_400 + 4 * 3_600), "2d 4h");
    }

    // PR #106 review R1 SHOULD-FIX DATA-14 1 / D4
    #[sqlx::test(migrations = "../../migrations")]
    async fn quality_read_routes_refuse_caller_outside_catalog_tenant(
        pool: lakehouse_store::PgPool,
    ) {
        use lakehouse_auth::{PermissionSet, PrincipalId};
        use uuid::Uuid;

        let t1 = Uuid::new_v4();
        let t2 = Uuid::new_v4();
        sqlx::query("INSERT INTO tenant (id, name, slug, plan, residency) VALUES ($1, 'T1', 't1', 'Standard', 'US'), ($2, 'T2', 't2', 'Standard', 'US')")
            .bind(t1)
            .bind(t2)
            .execute(&pool)
            .await
            .unwrap();

        let options = pool.connect_options();
        let mut env = HashMap::new();
        env.insert(
            "DATABASE_URL".to_owned(),
            format!(
                "postgres://{}:postgres@{}:{}/{}",
                options.get_username(),
                options.get_host(),
                options.get_port(),
                options.get_database().expect("a named database"),
            ),
        );
        env.insert("CATALOG_TENANT_ID".to_owned(), t1.to_string());
        let state =
            AppState::new(crate::config::Config::from_map(&env).expect("a valid test Config"));

        let outside_caller = Principal {
            id: PrincipalId::User(Uuid::new_v4()),
            tenant_ids: vec![t2],
            display_name: "Outsider".to_owned(),
            permissions: PermissionSet::parse("catalog:read"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        };

        // 1. rule_runs
        let err = rule_runs(
            State(state.clone()),
            Extension(outside_caller.clone()),
            HeaderMap::new(),
            Path("r1".to_owned()),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            crate::error::ApiRejection(ApiError::PermissionDenied(_))
        ));

        // 2. list_incidents
        let err = crate::routes::quality_incidents::list_incidents(
            State(state.clone()),
            Extension(outside_caller.clone()),
            HeaderMap::new(),
            axum::extract::Query(crate::routes::quality_incidents::IncidentsQuery {
                status: None,
                asset: None,
            }),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            crate::error::ApiRejection(ApiError::PermissionDenied(_))
        ));

        // 3. list_schedules
        let err = crate::routes::quality_schedules::list_schedules(
            State(state.clone()),
            Extension(outside_caller.clone()),
            HeaderMap::new(),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            crate::error::ApiRejection(ApiError::PermissionDenied(_))
        ));

        // 4. learning_status
        let err = crate::routes::quality_anomaly::learning_status(
            State(state),
            Extension(outside_caller),
            HeaderMap::new(),
            Path("silver.orders".to_owned()),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            crate::error::ApiRejection(ApiError::PermissionDenied(_))
        ));
    }
}
