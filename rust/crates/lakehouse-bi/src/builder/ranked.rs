//! The statements of a chart that keeps the top or bottom N groups of its
//! dimension and, when asked, gathers the rest into "Other" (`BI-6`).
//!
//! # Shapes
//!
//! ```text
//! -- "Other" (one dimension, optionally a breakdown)
//! SELECT __d AS dim, [brk,] __m0 AS measure, ..., __o AS __other
//! FROM (SELECT __d, [brk,] __o, <aggregate> AS __m0, ..., <order key> AS __t
//!   FROM (SELECT *, toUInt8(NOT (__k IN (<kept>))) AS __o,
//!                   if(__o = 1, 'Other', __k) AS __d
//!         FROM (SELECT *, ifNull(toString(dim), '') AS __k FROM rel WHERE ..) AS ob)
//!   GROUP BY __o, __d[, brk])
//! ORDER BY __o, <order>
//! ```
//!
//! `<kept>` is the first N groups of the dimension in the chart's order. The
//! "Other" group is aggregated by the same expression over the *rows* of every
//! group that was not kept (one `GROUP BY` over the whole relation), so an
//! average or a distinct count of "Other" is the average of those rows, not an
//! average of hidden averages.
//!
//! - `__k` is the dimension as text, so a number, a date and a text key all
//!   compare the same way and a `NULL` key is its own (empty) group instead of
//!   falling out of `NOT IN`.
//! - `__o` is `1` only on the computed group. The label of a real value
//!   that is literally `Other` is the same text, so the flag, not the label, is
//!   what tells them apart; it is returned as the `__other` column.
//! - The measures are aliased `__m<i>` inside and renamed outside:
//!   `round(sum(v)) AS v` beside a window over `sum(v)` would make the engine
//!   read the second `v` as the first (error 184, the same measurement as in
//!   [`super::staged`]).
//! - With a breakdown the groups are ranked by the *dimension's* total, so the
//!   order of the bars follows the ranking: a window over the grouped rows
//!   (`sum(sum(x)) OVER (PARTITION BY __d)`; an average is the sum over the
//!   count). A calculated aggregate has no such total, so it cannot rank a
//!   chart with a breakdown (refused when the chart is saved).
//!
//! # The breakdown's own limit (`BI-46`)
//!
//! QA `LH-BI-6-01`: on a bar, hbar, line or area chart with a breakdown the
//! limit keeps the N breakdown values with the largest total of the chart's
//! measure over the chart's filtered rows; every other breakdown value is left
//! out (plan D12). [`QueryBuilder::with_breakdown_cap`] adds that as one more
//! `WHERE` clause — `{b} IN (SELECT {b} FROM <relation> [WHERE the same rows]
//! GROUP BY {b} ORDER BY <the measure> DESC, {b} LIMIT N)` — so the cap ranks
//! over the same filtered rows as the statement it sits in, and the
//! dimension's own top N is untouched when the dimension has no grain. With a
//! time grain the limit is the breakdown's: the dimension keeps every bucket
//! and the cap is the only cut. A chart whose measure is a table calculation
//! gets no cap (its windows run over the whole grouped result); heatmap,
//! sankey and sunburst keep their statements, because there the second column
//! is an axis, a target or a ring, not a series.
//!
//! # Safety of the SQL
//!
//! Every name is a validated [`Ident`] or a fixed alias, every aggregate a
//! fixed function name, every predicate a fragment that
//! [`crate::builder::filter_predicates`] made through
//! [`lakehouse_core::ident::SqlLiteral`], and `Other` is a literal in this file.

use super::{Measure, QueryBuilder, Ready};
use crate::specs::Aggregate;

/// What the groups are ranked by.
enum Target<'a> {
    /// The dimension itself.
    Dimension,
    /// A measure, by position.
    Measure(usize, &'a Measure),
}

impl QueryBuilder<Ready> {
    fn where_sql(&self) -> String {
        if self.where_clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", self.where_clauses.join(" AND "))
        }
    }

    /// What the chart is ranked by and whether that is descending: its sort,
    /// else its `order` (a value order ranks by the first measure).
    fn target(&self) -> Option<(Target<'_>, bool)> {
        if let Some(s) = &self.sort {
            return Some(match self.sort_measure() {
                Some((i, m)) => (Target::Measure(i, m), s.desc),
                None => (Target::Dimension, s.desc),
            });
        }
        match self.order.as_str() {
            "none" => Some((Target::Dimension, false)),
            order => {
                let first = self.measures.first()?;
                Some((Target::Measure(0, first), order != "asc"))
            }
        }
    }

    /// The aggregate of a measure over a whole group, unrounded, to rank by.
    fn rank_expr(&self, measure: &Measure) -> String {
        if let Some(field) = self.from.aggregate_field(measure.column.as_str()) {
            return field.sql.clone();
        }
        match self.measure_aggregate(measure) {
            Aggregate::Count => "count()".to_owned(),
            agg => format!("{agg}({})", measure.column),
        }
    }

    /// The total of a measure over every row of a dimension value, computed
    /// from the grouped rows with a window: `None` for a calculated
    /// aggregate, which has no such total.
    fn dimension_total(&self, measure: &Measure, partition: &str) -> Option<String> {
        let c = &measure.column;
        if self.from.aggregate_field(c.as_str()).is_some() {
            return None;
        }
        let over = format!("OVER (PARTITION BY {partition})");
        Some(match self.measure_aggregate(measure) {
            Aggregate::Sum => format!("sum(sum({c})) {over}"),
            Aggregate::Count => format!("sum(count()) {over}"),
            Aggregate::Max => format!("max(max({c})) {over}"),
            Aggregate::Min => format!("min(min({c})) {over}"),
            Aggregate::Avg => format!("sum(sum({c})) {over} / sum(count({c})) {over}"),
        })
    }

    /// The columns of the final select: each measure renamed from its inner
    /// `__m<i>`.
    fn renamed(&self) -> String {
        self.measures
            .iter()
            .enumerate()
            .map(|(i, m)| format!("__m{i} AS {}", m.alias))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn inner_measures(&self) -> String {
        self.measures
            .iter()
            .enumerate()
            .map(|(i, m)| format!("{} AS __m{i}", self.agg_expr(m)))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// A chart with a breakdown that is sorted by a measure: the first N
    /// dimension values in that order (the largest or the smallest), shown in
    /// that order, each with its series. The ordinary breakdown statement
    /// orders the dimension alphabetically.
    pub(super) fn ranked_sql(&self) -> String {
        let (Some(dimension), Some(breakdown)) = (&self.dimension, &self.breakdown) else {
            return String::new();
        };
        let Some((Target::Measure(_, sort_measure), desc)) = self.target() else {
            return String::new();
        };
        let Some(total) = self.dimension_total(sort_measure, dimension.as_str()) else {
            return String::new();
        };
        let dir = if desc { "DESC" } else { "ASC" };
        let from = self.from.render();
        let where_sql = self.where_sql();
        let rank = self.rank_expr(sort_measure);
        let kept = format!(
            "SELECT {dimension} FROM {from}{where_sql} GROUP BY {dimension} \
             ORDER BY {rank} {dir}, {dimension} LIMIT {}",
            self.limit
        );
        let clauses = self.where_clauses.join(" AND ");
        // `BI-46` (plan D12): with a breakdown the limit is the breakdown's,
        // so a grained chart keeps every dimension value and the guard is the
        // chart's plain `WHERE` (the cap included, when there is one).
        let guard = if self.cap_breakdown && self.grain.is_some() {
            if clauses.is_empty() {
                String::new()
            } else {
                format!("WHERE {clauses}")
            }
        } else if clauses.is_empty() {
            format!("WHERE {dimension} IN ({kept})")
        } else {
            format!("WHERE {clauses} AND {dimension} IN ({kept})")
        };
        format!(
            "SELECT {dimension}, {breakdown}, {renamed} FROM \
             (SELECT {dimension}, {breakdown}, {inner}, {total} AS __t FROM {from} {guard} \
             GROUP BY {dimension}, {breakdown}) \
             ORDER BY __t {dir}, {dimension}, {breakdown}{settings}",
            renamed = self.renamed(),
            inner = self.inner_measures(),
            settings = self.from.settings(),
        )
    }

    /// The `WHERE` clause that keeps the N breakdown values with the largest
    /// total of the first measure (`BI-46`, plan D12; QA `LH-BI-6-01`), over
    /// the rows the chart's own `WHERE` clauses leave — `None` when the chart
    /// has no breakdown, applies steps or groups by several columns, has no
    /// measure, or that measure is a table calculation (whose windows run over
    /// the whole grouped result).
    ///
    /// "Largest" is always the largest: the subquery orders descending even on
    /// a chart sorted ascending, because the limit counts the biggest values.
    pub(super) fn breakdown_cap(&self) -> Option<String> {
        let breakdown = self.breakdown.as_ref()?;
        if self.steps.is_some() || !self.groupings.is_empty() {
            return None;
        }
        let measure = self.measures.first()?;
        if self.from.table_calc(measure.column.as_str()).is_some() {
            return None;
        }
        let rank = self.rank_expr(measure);
        Some(format!(
            "{breakdown} IN (SELECT {breakdown} FROM {from}{where_sql} \
             GROUP BY {breakdown} ORDER BY {rank} DESC, {breakdown} LIMIT {limit})",
            from = self.from.render(),
            where_sql = self.where_sql(),
            limit = self.limit,
        ))
    }

    /// Keep the N breakdown values with the largest total (`BI-46`, plan D12):
    /// `cap_breakdown` is true when the chart has a breakdown (so a grained
    /// chart keeps every bucket), and the cap itself becomes one more `WHERE`
    /// clause. An empty `breakdown_cap` (a table calculation) still sets the
    /// flag: the limit is the breakdown's, there is just no cap to apply.
    #[must_use]
    pub(crate) fn with_breakdown_cap(mut self) -> Self {
        self.cap_breakdown = self.breakdown.is_some();
        if let Some(cap) = self.breakdown_cap() {
            self.where_clauses.push(cap);
        }
        self
    }

    /// The top or bottom N groups of the dimension and one `Other` group over
    /// every row outside them (module doc).
    pub(super) fn other_sql(&self) -> String {
        let Some(dimension) = &self.dimension else {
            return String::new();
        };
        let Some((target, desc)) = self.target() else {
            return String::new();
        };
        let dir = if desc { "DESC" } else { "ASC" };
        let from = self.from.render();
        let base = format!(
            "(SELECT *, ifNull(toString({dimension}), '') AS __k FROM {from}{}) AS ob",
            self.where_sql()
        );
        let rank = match &target {
            Target::Dimension => format!("min({dimension})"),
            Target::Measure(_, m) => self.rank_expr(m),
        };
        let kept = format!(
            "SELECT __k FROM {base} GROUP BY __k ORDER BY {rank} {dir}, __k LIMIT {}",
            self.limit
        );
        let level = format!(
            "SELECT *, toUInt8(NOT (__k IN ({kept}))) AS __o, \
             if(__o = 1, 'Other', __k) AS __d FROM {base}"
        );
        let settings = self.from.settings();
        let (head, head_inner, group) = match &self.breakdown {
            Some(b) => (
                format!("__d AS {dimension}, {b}"),
                format!("__d, {b}"),
                format!("__o, __d, {b}"),
            ),
            None => (
                format!("__d AS {dimension}"),
                "__d".to_owned(),
                "__o, __d".to_owned(),
            ),
        };
        // The key the kept groups are shown in: the dimension's value, or a
        // measure; with a breakdown a measure's total over its dimension value.
        let order_key = match (&target, &self.breakdown) {
            (Target::Dimension, _) => format!("min({dimension})"),
            (Target::Measure(i, _), None) => format!("__m{i}"),
            (Target::Measure(_, m), Some(_)) => match self.dimension_total(m, "__d") {
                Some(total) => total,
                None => return String::new(),
            },
        };
        let tie = self
            .breakdown
            .as_ref()
            .map_or_else(|| "__d".to_owned(), |b| format!("__d, {b}"));
        format!(
            "SELECT {head}, {renamed}, __o AS __other FROM \
             (SELECT {head_inner}, __o, {inner}, {order_key} AS __t FROM ({level}) GROUP BY {group}) \
             ORDER BY __o, __t {dir}, {tie}{settings}",
            renamed = self.renamed(),
            inner = self.inner_measures(),
        )
    }
}
