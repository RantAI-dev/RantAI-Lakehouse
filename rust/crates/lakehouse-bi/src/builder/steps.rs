//! Steps after a chart's summary (`BI-6` part 2): filter the grouped result
//! again, or summarise it again, up to two times.
//!
//! # Shape
//!
//! ```text
//! SELECT * FROM (                                  -- the chart's sort and limit
//!   SELECT * FROM (<step 1: WHERE ...>) WHERE ...  -- a filter step
//!   -- or SELECT by, __s0 AS sum_x FROM (SELECT by, agg(x) AS __s0 FROM (<before>) GROUP BY by)
//!   <the grouped result: SELECT groups, aggregates FROM relation WHERE own+dashboard filters GROUP BY groups>
//! ) ORDER BY ... LIMIT n
//! ```
//!
//! The grouped result has no limit of its own: a step reads every group, so a
//! total over 1,000 is not decided on the groups that happened to be kept. The
//! engine refuses (`group_by_overflow_mode = 'throw'`, [`staged::MAX_GROUPS`]) a
//! result with more groups rather than cutting it and letting a step be wrong
//! without saying so. The chart's sort and limit apply to the *last* step's
//! result and may name only its columns.
//!
//! # Safety of the SQL
//!
//! Every column name is a validated [`Ident`] that belongs to the result the
//! step reads (checked in [`plan`]); aggregates are the closed set; text
//! values go through [`SqlLiteral`], numbers are parsed and printed by Rust,
//! dates are parsed and re-printed. Nothing a caller typed is formatted in.

use std::collections::HashMap;

use lakehouse_core::ident::{Ident, SqlLiteral};

use super::staged::MAX_GROUPS;
use super::{QueryBuilder, Ready, add_settings, parse_iso_date, parse_number};
use crate::filters::ColumnKind;
use crate::grain::Grain;
use crate::shape::{MAX_STEPS, Step, StepCondition, Summarise};

/// Most conditions in one filter step, and measures in one summarise step.
const MAX_CONDITIONS: usize = 10;
const MAX_STEP_MEASURES: usize = 6;

/// A step as it is rendered over the result before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannedStep {
    /// `WHERE <predicate>`.
    Filter(String),
    /// `GROUP BY by` with these aggregates (`expr AS __s<i>`) renamed to `names`.
    Summarise {
        /// The groupings kept.
        by: Vec<Ident>,
        /// The aggregate expressions, aliased `__s<i>`.
        aggs: Vec<String>,
        /// The result names, in the same order.
        names: Vec<Ident>,
    },
}

/// The steps of a chart, rendered, and the columns of the last result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepsSql {
    /// The steps in order.
    pub steps: Vec<PlannedStep>,
    /// The groupings of the last result.
    pub groups: Vec<Ident>,
    /// The measures of the last result.
    pub measures: Vec<Ident>,
}

/// What a grouping of the chart's grouped result holds, for the conditions on it.
#[must_use]
pub fn group_kind(column: ColumnKind, grain: Option<Grain>) -> ColumnKind {
    match grain {
        None => column,
        Some(g) if !g.is_truncation() => ColumnKind::Number,
        Some(Grain::Minute | Grain::Hour) => ColumnKind::DateTime,
        Some(_) => ColumnKind::Date,
    }
}

fn compare_op(op: &str) -> Option<&'static str> {
    Some(match op {
        "eq" => "=",
        "ne" => "!=",
        "gt" => ">",
        "gte" => ">=",
        "lt" => "<",
        "lte" => "<=",
        _ => return None,
    })
}

fn bound(kind: ColumnKind, column: &str, raw: Option<&str>, what: &str) -> Result<String, String> {
    let raw = raw.ok_or_else(|| format!("the condition on '{column}' needs a {what}."))?;
    match kind {
        ColumnKind::Number => parse_number(raw)
            .map(|v| v.to_string())
            .ok_or_else(|| format!("the {what} of the condition on '{column}' must be a number.")),
        ColumnKind::Date | ColumnKind::DateTime => parse_iso_date(raw)
            .map(|(y, m, d)| format!("toDate32('{y:04}-{m:02}-{d:02}')"))
            .ok_or_else(|| {
                format!("the {what} of the condition on '{column}' must be a date as YYYY-MM-DD.")
            }),
        ColumnKind::Text => Ok(SqlLiteral::from(raw).to_string()),
    }
}

fn condition(c: &StepCondition, kind: ColumnKind) -> Result<String, String> {
    let subject = match kind {
        ColumnKind::Date | ColumnKind::DateTime => format!("toDate32({})", c.column),
        _ => c.column.clone(),
    };
    if c.op == "between" {
        if kind == ColumnKind::Text {
            return Err(format!("'{}' is text: use equal or not equal.", c.column));
        }
        if c.min.is_none() && c.max.is_none() {
            return Err(format!(
                "a range on '{}' needs a minimum, a maximum, or both.",
                c.column
            ));
        }
        let mut parts = Vec::new();
        if let Some(min) = c.min.as_deref() {
            parts.push(format!(
                "{subject} >= {}",
                bound(kind, &c.column, Some(min), "minimum")?
            ));
        }
        if let Some(max) = c.max.as_deref() {
            parts.push(format!(
                "{subject} <= {}",
                bound(kind, &c.column, Some(max), "maximum")?
            ));
        }
        return Ok(format!("({})", parts.join(" AND ")));
    }
    let Some(op) = compare_op(&c.op) else {
        return Err(format!(
            "invalid comparison '{}': use eq, ne, gt, gte, lt, lte or between.",
            c.op
        ));
    };
    if kind == ColumnKind::Text && !matches!(c.op.as_str(), "eq" | "ne") {
        return Err(format!("'{}' is text: use equal or not equal.", c.column));
    }
    Ok(format!(
        "{subject} {op} {}",
        bound(kind, &c.column, c.value.as_deref(), "value")?
    ))
}

fn summarise(
    s: &Summarise,
    groups: &[(String, ColumnKind)],
    measures: &[String],
) -> Result<PlannedStep, String> {
    if s.by.is_empty() {
        return Err("a summary keeps at least one grouping.".to_owned());
    }
    let mut by = Vec::new();
    for name in &s.by {
        if !groups.iter().any(|(g, _)| g == name) {
            return Err(format!(
                "'{name}' is not a grouping of the result before this step."
            ));
        }
        if by.iter().any(|b: &Ident| b.as_str() == name) {
            return Err(format!("'{name}' is kept twice."));
        }
        by.push(Ident::new(name.as_str()).map_err(|_| format!("invalid grouping '{name}'."))?);
    }
    if s.measures.is_empty() || s.measures.len() > MAX_STEP_MEASURES {
        return Err(format!(
            "a summary computes 1 to {MAX_STEP_MEASURES} measures."
        ));
    }
    let (mut aggs, mut names) = (Vec::new(), Vec::new());
    for (i, m) in s.measures.iter().enumerate() {
        if !measures.contains(&m.column) {
            return Err(format!(
                "'{}' is not a measure of the result before this step.",
                m.column
            ));
        }
        let agg = m.aggregate.to_lowercase();
        if !matches!(agg.as_str(), "sum" | "avg" | "max" | "min" | "count") {
            return Err(format!("invalid aggregate: {}", m.aggregate));
        }
        let name = format!("{agg}_{}", m.column);
        let ident = Ident::new(name.as_str()).map_err(|_| format!("invalid measure '{name}'."))?;
        if names.contains(&ident) || by.contains(&ident) {
            return Err(format!(
                "the result name '{name}' is used twice; remove one measure."
            ));
        }
        aggs.push(if agg == "count" {
            format!("count() AS __s{i}")
        } else {
            format!("round({agg}({})) AS __s{i}", m.column)
        });
        names.push(ident);
    }
    Ok(PlannedStep::Summarise { by, aggs, names })
}

/// Check `steps` against the grouped result they start from (`groups` with the
/// kind each holds, `measures` by result name) and render them. Each step is
/// checked against the result of the one before it.
///
/// # Errors
///
/// The sentence for the person who saved the chart.
pub fn plan(
    steps: &[Step],
    groups: &[(String, ColumnKind)],
    measures: &[String],
) -> Result<StepsSql, String> {
    if steps.is_empty() || steps.len() > MAX_STEPS {
        return Err(format!("a chart takes 1 to {MAX_STEPS} steps."));
    }
    let mut groups: Vec<(String, ColumnKind)> = groups.to_vec();
    let mut measures: Vec<String> = measures.to_vec();
    let mut planned = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        let n = i + 1;
        match (&step.filter, &step.summarise) {
            (Some(conditions), None) => {
                if conditions.is_empty() || conditions.len() > MAX_CONDITIONS {
                    return Err(format!(
                        "step {n}: a filter has 1 to {MAX_CONDITIONS} conditions."
                    ));
                }
                let kinds: HashMap<&str, ColumnKind> = groups
                    .iter()
                    .map(|(g, k)| (g.as_str(), *k))
                    .chain(measures.iter().map(|m| (m.as_str(), ColumnKind::Number)))
                    .collect();
                let mut parts = Vec::new();
                for c in conditions {
                    let kind = *kinds.get(c.column.as_str()).ok_or_else(|| {
                        format!(
                            "step {n}: '{}' is not a column of the result before this step.",
                            c.column
                        )
                    })?;
                    Ident::new(c.column.as_str())
                        .map_err(|_| format!("step {n}: invalid column '{}'.", c.column))?;
                    parts.push(condition(c, kind).map_err(|m| format!("step {n}: {m}"))?);
                }
                planned.push(PlannedStep::Filter(parts.join(" AND ")));
            }
            (None, Some(s)) => {
                let step =
                    summarise(s, &groups, &measures).map_err(|m| format!("step {n}: {m}"))?;
                if let PlannedStep::Summarise { by, names, .. } = &step {
                    groups = by
                        .iter()
                        .map(|b| {
                            let kind = groups
                                .iter()
                                .find(|(g, _)| g == b.as_str())
                                .map_or(ColumnKind::Text, |(_, k)| *k);
                            (b.to_string(), kind)
                        })
                        .collect();
                    measures = names.iter().map(ToString::to_string).collect();
                }
                planned.push(step);
            }
            _ => {
                return Err(format!(
                    "step {n}: give either a filter or a summary, not both or neither."
                ));
            }
        }
    }
    let ident = |n: &String| Ident::new(n.as_str()).map_err(|_| format!("invalid column '{n}'."));
    Ok(StepsSql {
        steps: planned,
        groups: groups
            .iter()
            .map(|(g, _)| ident(g))
            .collect::<Result<_, _>>()?,
        measures: measures.iter().map(ident).collect::<Result<_, _>>()?,
    })
}

impl QueryBuilder<Ready> {
    /// The statement of a chart with steps (module doc).
    pub(super) fn steps_sql(&self) -> String {
        let Some(plan) = &self.steps else {
            return String::new();
        };
        let base: Vec<String> = if self.groupings.is_empty() {
            self.dimension.iter().map(ToString::to_string).collect()
        } else {
            self.groupings.iter().map(ToString::to_string).collect()
        };
        if base.is_empty() {
            return String::new();
        }
        let groups = base.join(", ");
        let (sel, renamed) = self.select_lists();
        let where_sql = if self.where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {} ", self.where_clauses.join(" AND "))
        };
        let from = self.from.render();
        let mut sql = match renamed {
            Some(renamed) => format!(
                "SELECT {groups}, {renamed} FROM (SELECT {groups}, {sel} FROM {from} {where_sql}GROUP BY {groups})"
            ),
            None => format!("SELECT {groups}, {sel} FROM {from} {where_sql}GROUP BY {groups}"),
        };
        for step in &plan.steps {
            sql = match step {
                PlannedStep::Filter(predicate) => {
                    format!("SELECT * FROM ({sql}) WHERE {predicate}")
                }
                PlannedStep::Summarise { by, aggs, names } => {
                    let by = by
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    let as_names = names
                        .iter()
                        .enumerate()
                        .map(|(i, n)| format!("__s{i} AS {n}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(
                        "SELECT {by}, {as_names} FROM (SELECT {by}, {} FROM ({sql}) GROUP BY {by})",
                        aggs.join(", ")
                    )
                }
            };
        }
        let last_groups = plan
            .groups
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let order = match &self.sort {
            Some(s) => format!(
                "{} {}, {last_groups}",
                s.by,
                if s.desc { "DESC" } else { "ASC" }
            ),
            None if self.order == "none" => last_groups,
            None => match plan.measures.first() {
                Some(first) => format!(
                    "{first} {}, {last_groups}",
                    if self.order == "asc" { "ASC" } else { "DESC" }
                ),
                None => return String::new(),
            },
        };
        let settings = add_settings(
            &self.from.settings(),
            &[
                &format!("max_rows_to_group_by = {MAX_GROUPS}"),
                "group_by_overflow_mode = 'throw'",
            ],
        );
        format!(
            "SELECT * FROM ({sql}) ORDER BY {order} LIMIT {}{settings}",
            self.limit
        )
    }
}

/// The statement of a chart with steps with its final `LIMIT old` replaced by
/// `LIMIT new`: what the builder's preview reads to show more rows of the last
/// step's result than the chart keeps. `None` unless the statement ends in
/// exactly that limit (followed only by the `SETTINGS` this module writes), so
/// a statement of another shape is never edited.
#[must_use]
pub fn with_limit(sql: &str, old: u32, new: u32) -> Option<String> {
    let needle = format!(" LIMIT {old}");
    let at = sql.rfind(&needle)?;
    let tail = &sql[at + needle.len()..];
    if !(tail.is_empty() || tail.starts_with(" SETTINGS ")) || sql.matches(" LIMIT ").count() != 1 {
        return None;
    }
    Some(format!("{} LIMIT {new}{tail}", &sql[..at]))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::shape::StepMeasure;

    fn groups() -> Vec<(String, ColumnKind)> {
        vec![
            ("kab".to_owned(), ColumnKind::Text),
            ("d".to_owned(), ColumnKind::Date),
            ("n".to_owned(), ColumnKind::Number),
        ]
    }
    fn cond(column: &str, op: &str, value: Option<&str>) -> StepCondition {
        StepCondition {
            column: column.to_owned(),
            op: op.to_owned(),
            value: value.map(str::to_owned),
            min: None,
            max: None,
        }
    }
    fn filter(conditions: Vec<StepCondition>) -> Step {
        Step {
            filter: Some(conditions),
            summarise: None,
        }
    }
    fn summary(by: &[&str], measures: &[(&str, &str)]) -> Step {
        Step {
            filter: None,
            summarise: Some(Summarise {
                by: by.iter().map(|b| (*b).to_owned()).collect(),
                measures: measures
                    .iter()
                    .map(|(c, a)| StepMeasure {
                        column: (*c).to_owned(),
                        aggregate: (*a).to_owned(),
                    })
                    .collect(),
            }),
        }
    }
    fn measures() -> Vec<String> {
        vec!["visitors".to_owned()]
    }

    #[test]
    fn conditions_are_anded_and_bound_by_the_kind_of_their_column() {
        let plan = plan(
            &[filter(vec![
                cond("visitors", "gt", Some("1000")),
                cond("kab", "ne", Some("O'Brien")),
                cond("d", "gte", Some("2026-03-01")),
                StepCondition {
                    min: Some("1".to_owned()),
                    max: Some("9.5".to_owned()),
                    ..cond("n", "between", None)
                },
            ])],
            &groups(),
            &measures(),
        )
        .unwrap();
        assert_eq!(
            plan.steps,
            [PlannedStep::Filter(
                "visitors > 1000 AND kab != 'O''Brien' AND toDate32(d) >= toDate32('2026-03-01') AND (n >= 1 AND n <= 9.5)"
                    .to_owned()
            )]
        );
        assert_eq!(plan.groups.len(), 3);
    }

    #[test]
    fn a_summary_names_its_measures_by_aggregate_and_column_and_narrows_the_result() {
        let plan = plan(
            &[
                summary(&["kab"], &[("visitors", "SUM"), ("visitors", "max")]),
                filter(vec![cond("sum_visitors", "gt", Some("5"))]),
            ],
            &groups(),
            &measures(),
        )
        .unwrap();
        assert_eq!(
            plan.groups
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["kab"]
        );
        assert_eq!(
            plan.measures
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["sum_visitors", "max_visitors"]
        );
    }

    #[test]
    fn a_step_reads_only_the_columns_of_the_result_before_it() {
        let after = [
            summary(&["kab"], &[("visitors", "sum")]),
            filter(vec![cond("d", "eq", Some("2026-01-01"))]),
        ];
        let err = plan(&after, &groups(), &measures()).unwrap_err();
        assert!(err.contains("step 2") && err.contains("'d'"), "{err}");
        let err = plan(
            &[summary(&["kab"], &[("sum_visitors", "sum")])],
            &groups(),
            &measures(),
        )
        .unwrap_err();
        assert!(err.contains("not a measure"), "{err}");
        let err = plan(
            &[summary(&["nope"], &[("visitors", "sum")])],
            &groups(),
            &measures(),
        )
        .unwrap_err();
        assert!(err.contains("not a grouping"), "{err}");
    }

    #[test]
    fn each_refusal_is_a_plain_sentence() {
        let g = groups();
        let m = measures();
        let case = |steps: Vec<Step>, expected: &str| {
            let err = plan(&steps, &g, &m).unwrap_err();
            assert!(err.contains(expected), "{expected}: {err}");
        };
        case(vec![], "1 to 2 steps");
        case(
            vec![filter(vec![cond("visitors", "gt", Some("1"))]); 3],
            "1 to 2 steps",
        );
        case(vec![Step::default()], "either a filter or a summary");
        case(vec![filter(vec![])], "1 to 10 conditions");
        case(
            vec![filter(vec![cond("visitors", "like", Some("1"))])],
            "invalid comparison",
        );
        case(
            vec![filter(vec![cond("visitors", "gt", Some("lots"))])],
            "must be a number",
        );
        case(
            vec![filter(vec![cond("visitors", "gt", None)])],
            "needs a value",
        );
        case(vec![filter(vec![cond("kab", "gt", Some("a"))])], "is text");
        case(
            vec![filter(vec![cond("d", "eq", Some("2026-13-01"))])],
            "date as YYYY-MM-DD",
        );
        case(
            vec![summary(&[], &[("visitors", "sum")])],
            "at least one grouping",
        );
        case(
            vec![summary(&["kab", "kab"], &[("visitors", "sum")])],
            "kept twice",
        );
        case(vec![summary(&["kab"], &[])], "1 to 6 measures");
        case(
            vec![summary(&["kab"], &[("visitors", "median")])],
            "invalid aggregate",
        );
        case(
            vec![summary(
                &["kab"],
                &[("visitors", "sum"), ("visitors", "SUM")],
            )],
            "used twice",
        );
        let both = Step {
            filter: Some(vec![cond("visitors", "gt", Some("1"))]),
            ..summary(&["kab"], &[("visitors", "sum")])
        };
        case(vec![both], "not both or neither");
    }

    #[test]
    fn a_value_never_reaches_the_statement_outside_a_literal_or_a_parsed_number() {
        for nasty in [
            "x' OR 1=1 --",
            "1; DROP TABLE t",
            "back\\slash",
            "0x10",
            "1e999",
            "NaN",
        ] {
            let text = plan(
                &[filter(vec![cond("kab", "eq", Some(nasty))])],
                &groups(),
                &measures(),
            );
            if let Ok(p) = text {
                let PlannedStep::Filter(sql) = &p.steps[0] else {
                    panic!()
                };
                assert_eq!(
                    *sql,
                    format!("kab = {}", SqlLiteral::from(nasty)),
                    "{nasty}"
                );
            }
            let number = plan(
                &[filter(vec![cond("visitors", "eq", Some(nasty))])],
                &groups(),
                &measures(),
            );
            if let Ok(p) = number {
                let PlannedStep::Filter(sql) = &p.steps[0] else {
                    panic!()
                };
                assert!(
                    sql.strip_prefix("visitors = ")
                        .is_some_and(|n| n.parse::<f64>().is_ok()),
                    "{nasty}: {sql}"
                );
            }
        }
    }

    #[test]
    fn the_limit_of_a_steps_statement_is_replaced_and_no_other_statement_is_edited() {
        let sql =
            "SELECT * FROM (SELECT 1) ORDER BY a LIMIT 20 SETTINGS max_rows_to_group_by = 10000";
        assert_eq!(
            with_limit(sql, 20, 2001).unwrap(),
            "SELECT * FROM (SELECT 1) ORDER BY a LIMIT 2001 SETTINGS max_rows_to_group_by = 10000"
        );
        assert_eq!(with_limit(sql, 5, 9), None);
        assert_eq!(
            with_limit("SELECT * FROM (SELECT 1 LIMIT 3) LIMIT 3", 3, 9),
            None
        );
    }
}
