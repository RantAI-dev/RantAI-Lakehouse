//! The parts of a formula that cannot be one expression over rows: table
//! calculations, period comparisons and `Fixed` (`BI-8` part 2).
//!
//! # What each one becomes
//!
//! A chart groups its rows (`GROUP BY dimension[, breakdown]`). These
//! functions take an *aggregate* of that grouped result and need more than
//! one group at a time, so the compiler does not produce a finished SQL
//! expression for the whole formula. It produces:
//!
//! - **hidden aggregates** ([`HiddenAgg`]): every aggregate they read, named
//!   `__a_<hash>`, which the chart's grouped statement computes as extra
//!   columns;
//! - **shifted copies** ([`Shift`]): `PreviousPeriod` and `SamePeriodLastYear`
//!   read a hidden aggregate of the bucket one period (or one year) earlier;
//!   the builder joins the grouped result to itself on the *calendar date
//!   shift*, so a gap in the data leaves an empty value instead of
//!   misaligning the rows;
//! - **`Fixed` joins** ([`FixedJoin`]): an aggregate computed at fixed columns
//!   over the chart's filtered rows and joined back under `__f_<hash>`;
//! - the **expression** itself, in terms of those columns and of window
//!   functions over the grouped result, ordered by the chart's dimension and
//!   partitioned by its breakdown.
//!
//! # Safety of the SQL
//!
//! Nothing here copies the person's text: aliases are hashes of the SQL the
//! compiler itself generated, window frames and function names are written in
//! this file, the only numbers are checked whole numbers printed by Rust, and
//! a `Fixed` column is a source column that passed [`Ident::new`].

use std::collections::BTreeSet;

use lakehouse_core::ident::Ident;

use super::{
    Cx, FType, FixedJoin, HiddenAgg, Level, Shift, ShiftKind, Typed, err, literal_int, want,
};
use crate::formula::parse::{Kind, Node};
use crate::formula::{FormulaError, clip};
use crate::grain::Grain;

/// FNV-1a over bytes: a stable name for a generated expression, so the same
/// aggregate compiled twice (by two fields, or twice by one) is one column.
fn stable_name(prefix: &str, parts: &[&str]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for byte in part.bytes().chain(std::iter::once(0)) {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{prefix}{hash:016x}")
}

impl Cx<'_> {
    /// Register `sql` as a hidden aggregate and name it.
    fn hidden_agg(&mut self, sql: &str) -> String {
        let alias = stable_name("__a_", &[sql]);
        if !self.staged.aggs.iter().any(|h| h.alias == alias) {
            self.staged.aggs.push(HiddenAgg {
                alias: alias.clone(),
                sql: sql.to_owned(),
            });
        }
        alias
    }

    /// `func OVER (...)` over the grouped result: partitioned by the breakdown,
    /// ordered by the dimension.
    fn over(&self, func: &str, frame: &str) -> String {
        let (key, part) = self.scope.chart.as_ref().map_or_else(
            || ("1".to_owned(), String::new()),
            |c| {
                (
                    c.order_key.clone(),
                    c.breakdown
                        .as_ref()
                        .map_or_else(String::new, |b| format!("PARTITION BY {b} ")),
                )
            },
        );
        format!("{func} OVER ({part}ORDER BY {key}{frame})")
    }

    /// An argument that must be an aggregate: its SQL becomes a hidden
    /// aggregate and the alias is returned.
    fn aggregate_arg(&mut self, node: &Node, fname: &str) -> Result<String, FormulaError> {
        let t = self.node(node)?;
        match t.level {
            Level::Table => {
                return Err(err(
                    node,
                    "a table calculation cannot be inside another one.",
                ));
            }
            Level::Aggregate => {}
            _ => {
                return Err(err(
                    node,
                    format!("{fname} takes an aggregate, for example {fname}(Sum([amount]))."),
                ));
            }
        }
        want(&t, node, FType::Number)?;
        Ok(self.hidden_agg(&t.sql))
    }

    /// The table calculations and the period comparisons.
    pub(super) fn staged_call(
        &mut self,
        name: &str,
        call: &Node,
        nodes: &[Node],
    ) -> Result<Typed, FormulaError> {
        let ctx = self.scope.chart.clone();
        if let Some(c) = &ctx
            && c.dimension.is_none()
        {
            return Err(err(
                call,
                format!("{name} needs a chart with a dimension; a single number has no order."),
            ));
        }
        let rows = " ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW";
        let all = " ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING";
        let sql = match name {
            "RunningTotal" => {
                let a = self.aggregate_arg(&nodes[0], name)?;
                self.over(&format!("sum({a})"), rows)
            }
            "RunningCount" => {
                let agg = match nodes.first() {
                    None => "count()".to_owned(),
                    Some(n) => {
                        let t = self.node(n)?;
                        if !matches!(t.level, Level::Row) {
                            return Err(err(n, "RunningCount takes a column, or nothing."));
                        }
                        format!("count({})", t.sql)
                    }
                };
                let a = self.hidden_agg(&agg);
                self.over(&format!("sum({a})"), rows)
            }
            "Offset" => {
                let a = self.aggregate_arg(&nodes[0], name)?;
                let n = literal_int(&nodes[1], -50, 50)?;
                if n == 0 {
                    return Err(err(&nodes[1], "write a whole number other than 0 here."));
                }
                // The *InFrame forms over the whole partition; `toNullable`
                // makes the first rows empty instead of 0.
                let func = if n > 0 { "lagInFrame" } else { "leadInFrame" };
                self.over(&format!("{func}(toNullable({a}), {})", n.abs()), all)
            }
            "PercentOfTotal" => {
                let a = self.aggregate_arg(&nodes[0], name)?;
                // Of the whole chart, not of a series: no partition.
                format!("(100 * {a} / nullIf(sum({a}) OVER (), 0))")
            }
            "Rank" => {
                let a = self.aggregate_arg(&nodes[0], name)?;
                let part = ctx
                    .as_ref()
                    .and_then(|c| c.breakdown.as_ref())
                    .map_or_else(String::new, |b| format!("PARTITION BY {b} "));
                format!("rank() OVER ({part}ORDER BY {a} DESC)")
            }
            "MovingAverage" => {
                let a = self.aggregate_arg(&nodes[0], name)?;
                let n = literal_int(&nodes[1], 1, 100)?;
                self.over(
                    &format!("avg({a})"),
                    &format!(" ROWS BETWEEN {} PRECEDING AND CURRENT ROW", n - 1),
                )
            }
            "PreviousPeriod" | "SamePeriodLastYear" => {
                return self.period(name, call, nodes);
            }
            _ => return Err(err(call, "this function is not available yet.")),
        };
        Ok(Typed {
            sql,
            ty: FType::Number,
            level: Level::Table,
        })
    }

    fn period(&mut self, name: &str, call: &Node, nodes: &[Node]) -> Result<Typed, FormulaError> {
        let kind = if name == "PreviousPeriod" {
            ShiftKind::Previous
        } else {
            ShiftKind::LastYear
        };
        if let Some(c) = self.scope.chart.clone() {
            let Some(grain) = c.grain else {
                return Err(err(
                    call,
                    format!(
                        "{name} needs a chart that groups a date: choose Group by (day, week, month, quarter or year) on the dimension."
                    ),
                ));
            };
            if !grain.is_truncation() {
                return Err(err(
                    call,
                    format!(
                        "{name} needs Group by to be day, week, month, quarter or year, not a part of the date."
                    ),
                ));
            }
            if kind == ShiftKind::LastYear && matches!(grain, Grain::Minute | Grain::Hour) {
                return Err(err(
                    call,
                    "SamePeriodLastYear needs Group by of day or coarser.",
                ));
            }
        }
        let source = self.aggregate_arg(&nodes[0], name)?;
        let tag = if kind == ShiftKind::Previous {
            "p"
        } else {
            "y"
        };
        let alias = stable_name("__p_", &[tag, &source]);
        if !self.staged.shifts.iter().any(|s| s.alias == alias) {
            self.staged.shifts.push(Shift {
                alias: alias.clone(),
                source,
                kind,
            });
        }
        Ok(Typed {
            sql: alias,
            ty: FType::Number,
            level: Level::Table,
        })
    }

    /// `Fixed([a], [b], ..., aggregate)`.
    pub(super) fn fixed(&mut self, call: &Node, nodes: &[Node]) -> Result<Typed, FormulaError> {
        let Some((last, columns)) = nodes.split_last() else {
            return Err(err(call, "Fixed needs an aggregate as its last argument."));
        };
        let mut names: Vec<String> = Vec::new();
        for n in columns {
            let Kind::Name(name) = &n.kind else {
                return Err(err(
                    n,
                    "write a column of the source here, such as [region].",
                ));
            };
            if !self.scope.columns.contains_key(name) || Ident::new(name.as_str()).is_err() {
                return Err(err(
                    n,
                    format!(
                        "'{}' is not a column of this source; Fixed takes columns, not fields.",
                        clip(name)
                    ),
                ));
            }
            if names.contains(name) {
                return Err(err(n, "this column is listed twice."));
            }
            if let Some(c) = &self.scope.chart {
                let groups: Vec<&String> = c.dimension.iter().chain(c.breakdown.iter()).collect();
                if !groups.contains(&name) {
                    let listed = if groups.is_empty() {
                        "this chart groups by nothing".to_owned()
                    } else {
                        format!(
                            "this chart groups by {}",
                            groups
                                .iter()
                                .map(|g| g.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    };
                    return Err(err(
                        n,
                        format!("Fixed can only use columns the chart groups by; {listed}."),
                    ));
                }
                if c.grain.is_some() && c.dimension.as_deref() == Some(name.as_str()) {
                    return Err(err(
                        n,
                        "Fixed cannot use a date column the chart groups by date.",
                    ));
                }
            }
            names.push(name.clone());
        }
        // The aggregate reads its own columns inside the join, not the
        // chart's grouped statement, so what it reads is not carried.
        let before: BTreeSet<String> = self.reads.clone();
        let t = self.node(last)?;
        self.reads = before;
        match t.level {
            Level::Aggregate => {}
            Level::Table => {
                return Err(err(last, "a table calculation cannot be inside Fixed."));
            }
            _ => {
                return Err(err(
                    last,
                    "the last argument of Fixed is an aggregate, for example Sum([amount]).",
                ));
            }
        }
        let joined = names.join(",");
        let alias = stable_name("__f_", &[&joined, &t.sql]);
        if !self.staged.fixed.iter().any(|f| f.alias == alias) {
            self.staged.fixed.push(FixedJoin {
                alias: alias.clone(),
                columns: names,
                agg_sql: t.sql.clone(),
            });
        }
        self.reads.insert(alias.clone());
        Ok(Typed {
            sql: format!("max({alias})"),
            ty: t.ty,
            level: Level::Aggregate,
        })
    }
}
