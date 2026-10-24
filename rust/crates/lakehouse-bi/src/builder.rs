//! SQL builders for stored chart specs.
//!
//! Ports `buildSql`, `buildKpiSql`, and `sqlWithFilters` from
//! `src/services/clients/bi-store.ts` (around line 396). The general and KPI
//! builders assemble SQL from already-validated identifiers (mart/column
//! names checked against `system.columns` upstream, in [`crate::store`]), so
//! only the WHERE-clause values here need escaping — via
//! [`lakehouse_core::ident::SqlLiteral`], never hand-rolled.
//!
//! `sqlWithFilters`'s 8 loose positional arguments become a small typestate
//! builder ([`QueryBuilder`]) so a caller cannot build a query without first
//! supplying its projection.

use std::collections::HashMap;
use std::marker::PhantomData;

use lakehouse_core::ident::{Ident, SqlLiteral};
use serde::Serialize;

use crate::fields::{self, FieldCatalog};
use crate::filters::{
    ColumnKind, FilterDef, FilterOp, RelativeAnchor, RelativeUnit, parse_iso_date, parse_number,
};
use crate::formula::compile::{FixedJoin, HiddenAgg, Shift};
use crate::grain::{self, Grain, TimeContext, WeekStart};
use crate::joins;
use crate::shape;
use crate::specs::Aggregate;
use crate::store::{ChartInput, StoredChartSpec};
use crate::tables;

mod binned;
mod ranked;
mod staged;
pub mod steps;

pub use binned::BinRead;

/// One more bin over a relation whose rows are already filtered
/// ([`binned::add_layer`] for a caller outside this module).
#[must_use]
pub fn binned_layer(
    relation: Relation,
    column: &Ident,
    mode: crate::bin::BinMode,
    needed: &[String],
) -> Relation {
    binned::add_layer(
        relation,
        &binned::Layer {
            column: column.clone(),
            mode,
        },
        needed,
    )
}

/// Typestate marker: a [`QueryBuilder`] that still needs its projection
/// (measures) before it can accept filters or be built.
pub struct NeedsProjection;

/// Typestate marker: a [`QueryBuilder`] with a projection set, ready to
/// accept filters and be built.
pub struct Ready;

/// `SETTINGS` appended to every statement built over a [`Relation::Sql`]:
/// the same 2 000-row cap as Query Studio (`routes/query.rs`
/// `MAX_RESULT_ROWS`), enforced by `ClickHouse` rather than by truncating
/// afterwards, and a 30 s execution limit. Why 30 s: Superset's SQL Lab
/// default (`SQLLAB_TIMEOUT`), and below the API's 60 s route/HTTP timeout,
/// so `ClickHouse` cancels the query itself instead of it running on after
/// the client gave up (measured: `max_execution_time = 1` on a 1e11-row
/// scan fails with `TIMEOUT_EXCEEDED` after 1.00 s on `ClickHouse` 26.8).
pub const SQL_SOURCE_SETTINGS: &str =
    " SETTINGS max_result_rows = 2000, result_overflow_mode = 'break', max_execution_time = 30";

/// `statement` with the SQL-source cap of a tile ([`SQL_SOURCE_SETTINGS`]:
/// 2 000 rows, 30 seconds) swapped for a download's: `max_rows` rows and 280
/// seconds, below the 300-second timeout of the download's stream (`BI-1`,
/// plan decision D6). A separate path on purpose: a tile keeps its cap, only a
/// whole-result download lifts it. A statement over a mart, which carries no
/// such cap, comes back unchanged; settings added after the cap
/// (`prefer_column_name_to_alias`, ...) are kept.
#[must_use]
pub fn with_export_settings(statement: &str, max_rows: u32) -> String {
    let Some(at) = statement.rfind(SQL_SOURCE_SETTINGS) else {
        return statement.to_owned();
    };
    format!(
        "{} SETTINGS max_result_rows = {max_rows}, result_overflow_mode = 'break', \
         max_execution_time = 280{}",
        &statement[..at],
        &statement[at + SQL_SOURCE_SETTINGS.len()..]
    )
}

/// Where a chart's rows come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Relation {
    /// A Gold mart, unqualified (`mart_x`); rendered as `serving.mart_x`.
    Mart(Ident),
    /// A table and the tables joined to it (`BI-7`), read as one flat table:
    /// the base table's columns under their own names, each joined table's
    /// under `<alias>__<column>`. Every table is a plain `serving.<table>`
    /// reference in a `FROM` or `JOIN` position, so the role rewrite
    /// substitutes its masked, filtered projection at each one. Only
    /// [`crate::joins::plan`] creates it.
    Joined {
        /// The base table, unqualified.
        base: Ident,
        /// Its columns.
        base_columns: Vec<Ident>,
        /// The joined tables, in order.
        joins: Vec<crate::joins::JoinedTable>,
    },
    /// A dashboard SQL source's text. The API validates it (read-only,
    /// `serving.*` tables only, no `;`/`SETTINGS`/`FORMAT`) before storing
    /// it; here it is only ever wrapped as a derived table so this
    /// builder's own `WHERE`/`GROUP BY`/`LIMIT` apply outside it. The
    /// newline before `)` keeps a trailing `-- comment` from swallowing
    /// the closing parenthesis.
    Sql(String),
    /// `base` with `predicates` applied in an inner `SELECT *`, so the row
    /// filter can only see the relation's own columns. Used when a filtered
    /// column has the same name as an output alias of the chart query
    /// (`sum(visitors) AS visitors`): in a plain `WHERE`, `ClickHouse`
    /// resolves that name to the alias and rejects the aggregate (error 184,
    /// `BI-18·A review BLOCKER 1`). Only [`rebuild`] creates it.
    Filtered {
        /// What is filtered.
        base: Box<Relation>,
        /// Complete SQL fragments, joined with `AND`.
        predicates: Vec<String>,
    },
    /// `base` joined to the one-row range of a number column (`BI-10`): the
    /// column's minimum, maximum and bin width over `base` itself, as
    /// `__bmn_<c>`, `__bmx_<c>` and `__bw_<c>` beside every column of `base`.
    /// A [`Relation::Bucketed`] over it turns the column into its bin. The
    /// subquery reads the same `base` text as the outer query, so the edges
    /// come from the rows the bins hold, and the role rewrite substitutes the
    /// governed projection at both reads. Only [`crate::bin::layers`] creates
    /// it.
    Ranged {
        /// What is binned (already filtered when predicates apply).
        base: Box<Relation>,
        /// The column and the rule for its width.
        range: crate::bin::Range,
    },
    /// `base` with the grained dimension replaced by its bucket
    /// (`BI-9`): `SELECT <bucket> AS <dimension>, <carried columns> FROM
    /// base`. Only the columns the chart reads are carried, so the outer
    /// query is the ordinary one over a column that is already the bucket.
    /// Dashboard filters are applied to the raw column *inside* `base`
    /// ([`Relation::Filtered`]), so the alias can never shadow the column in
    /// a `WHERE`. Only [`grained_sql`] creates it; every value in it is a
    /// validated identifier or an expression from [`Grain::bucket_expr`].
    Bucketed {
        /// What is bucketed (already filtered when predicates apply).
        base: Box<Relation>,
        /// The dimension's name, kept as the result column.
        dimension: String,
        /// The bucket expression over the raw column.
        expr: String,
        /// The other columns the chart reads (breakdown, measures).
        carried: Vec<String>,
    },
    /// `base` plus the row-level calculated fields a chart uses (`BI-8`):
    /// `SELECT *, <expression> AS <name>, ... FROM base`. Every column of
    /// `base` is still there, so a filter on a raw column, a grain on a date
    /// column and the role rewrite (which substitutes the *table* inside
    /// `base`) all see what they saw before. The expressions are compiled by
    /// [`crate::formula`] and name only columns of the source; an alias is a
    /// validated field name that is not a column of the source.
    /// `aggregates` are the aggregate fields: they add no column, they stand
    /// in for `sum(measure)` where the chart is built
    /// ([`Relation::aggregate_field`]). Only [`crate::fields::prepare`]
    /// creates it.
    Calculated {
        /// The source relation.
        base: Box<Relation>,
        /// Row-level fields as `(name, expression)`.
        columns: Vec<(String, String)>,
        /// Aggregate fields, usable as measures.
        aggregates: Vec<AggregateField>,
        /// Table calculations and period comparisons (`BI-8` part 2),
        /// computed over the chart's grouped result.
        table_calcs: Vec<TableCalc>,
    },
    /// `base` with `Fixed` aggregates joined back (`BI-8` part 2): each row of
    /// `base` gains the aggregate computed at its fixed columns. The chart's
    /// filters are applied *inside* `base` (see [`Relation::with_filters_inside`]),
    /// so the aggregate is over the same filtered rows the chart reads.
    Fixed {
        /// The filtered source rows.
        base: Box<Relation>,
        /// The joins to add.
        joins: Vec<FixedJoin>,
    },
}

/// A table calculation or period comparison as a measure: the expression
/// over the grouped result, and what the grouped result must carry for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCalc {
    /// The field's name, the result column.
    pub name: String,
    /// The outer expression (window functions over hidden columns).
    pub sql: String,
    /// Hidden aggregates it reads.
    pub aggs: Vec<HiddenAgg>,
    /// Shifted copies it reads.
    pub shifts: Vec<Shift>,
    /// The source columns its aggregates read, which a bucketing inner query
    /// must carry.
    pub reads: Vec<String>,
}

/// An aggregate calculated field as a measure: the whole expression is the
/// measure, so the chart's own aggregate does not apply to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateField {
    /// The field's name, the result column.
    pub name: String,
    /// The compiled aggregate expression.
    pub sql: String,
    /// The source columns it reads, which a bucketing inner query must carry.
    pub reads: Vec<String>,
    /// `BI-2`: the expression reads columns whose names a measure of the same
    /// statement may also use as its result name (`visitors`), which the engine
    /// resolves to the measure and rejects as an aggregate inside an aggregate.
    /// The statement then asks for `prefer_column_name_to_alias`. Only a
    /// metric sets it, so a chart that uses a calculated aggregate keeps the
    /// statement it always had.
    pub prefer_columns: bool,
}

impl Relation {
    /// The `FROM` target: `serving.<mart>`, or the source text as a derived
    /// table. Public so the filter value list reads a relation exactly the
    /// way a tile does.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Mart(mart) => format!("serving.{mart}"),
            Self::Joined {
                base,
                base_columns,
                joins,
            } => Self::render_joined(base, base_columns, joins),
            Self::Sql(sql) => format!("(\n{}\n) AS src", sql.trim()),
            Self::Ranged { base, range } => {
                let inner = base.render();
                format!(
                    "(SELECT * FROM {inner} CROSS JOIN ({}) AS rng) AS ranged",
                    range.scalars_sql(&inner)
                )
            }
            Self::Filtered { base, predicates } => format!(
                "(SELECT * FROM {} WHERE {}) AS flt",
                base.render(),
                predicates.join(" AND ")
            ),
            Self::Bucketed {
                base,
                dimension,
                expr,
                carried,
            } => {
                let rest = carried.iter().fold(String::new(), |mut acc, c| {
                    acc.push_str(", ");
                    acc.push_str(c);
                    acc
                });
                format!(
                    "(SELECT {expr} AS {dimension}{rest} FROM {}) AS bkt",
                    base.render()
                )
            }
            Self::Fixed { base, joins } => {
                // `SELECT *` over a join keeps the base's columns once (the
                // `USING` keys are not repeated) and adds each join's alias.
                let base_sql = base.render();
                let mut from = base_sql.clone();
                for (i, j) in joins.iter().enumerate() {
                    let cols = j.columns.join(", ");
                    if j.columns.is_empty() {
                        from = format!(
                            "{from} CROSS JOIN (SELECT {} AS {} FROM {base_sql}) AS f{i}",
                            j.agg_sql, j.alias
                        );
                    } else {
                        from = format!(
                            "{from} LEFT JOIN (SELECT {cols}, {} AS {} FROM {base_sql} GROUP BY {cols}) AS f{i} USING ({cols})",
                            j.agg_sql, j.alias
                        );
                    }
                }
                format!("(SELECT * FROM {from}) AS fx")
            }
            Self::Calculated { base, columns, .. } if columns.is_empty() => base.render(),
            Self::Calculated { base, columns, .. } => {
                let added = columns.iter().fold(String::new(), |mut acc, (name, expr)| {
                    acc.push_str(", ");
                    acc.push_str(expr);
                    acc.push_str(" AS ");
                    acc.push_str(name);
                    acc
                });
                format!("(SELECT *{added} FROM {}) AS calc", base.render())
            }
        }
    }

    /// Whether the rows come from a dashboard SQL source, looking through the
    /// wrappers a builder puts around it.
    #[must_use]
    pub fn is_sql_source(&self) -> bool {
        match self {
            Self::Mart(_) | Self::Joined { .. } => false,
            Self::Sql(_) => true,
            Self::Filtered { base, .. }
            | Self::Ranged { base, .. }
            | Self::Bucketed { base, .. }
            | Self::Calculated { base, .. }
            | Self::Fixed { base, .. } => base.is_sql_source(),
        }
    }

    /// The aggregate calculated field called `name` that this relation
    /// carries, looking through the wrappers a builder puts around it.
    #[must_use]
    pub fn aggregate_field(&self, name: &str) -> Option<&AggregateField> {
        match self {
            Self::Mart(_) | Self::Sql(_) | Self::Joined { .. } => None,
            Self::Calculated {
                base, aggregates, ..
            } => aggregates
                .iter()
                .find(|a| a.name == name)
                .or_else(|| base.aggregate_field(name)),
            Self::Filtered { base, .. }
            | Self::Ranged { base, .. }
            | Self::Bucketed { base, .. }
            | Self::Fixed { base, .. } => base.aggregate_field(name),
        }
    }

    /// Trailing `SETTINGS` for this relation (empty for a mart, whose
    /// SQL is assembled entirely from validated identifiers). A `Fixed` join
    /// adds `join_use_nulls = 1`, so a row whose key is empty gets an empty
    /// `Fixed` value rather than the type's default.
    pub(crate) fn settings(&self) -> String {
        match self {
            Self::Mart(_) => String::new(),
            // BI-7: an outer join's missing side is empty (NULL), not the
            // type's default; the engine's own join memory limit stays.
            Self::Joined { .. } => " SETTINGS join_use_nulls = 1".to_owned(),
            Self::Sql(_) => SQL_SOURCE_SETTINGS.to_owned(),
            Self::Calculated {
                base, aggregates, ..
            } => {
                if aggregates.iter().any(|a| a.prefer_columns) {
                    add_settings(&base.settings(), &["prefer_column_name_to_alias = 1"])
                } else {
                    base.settings()
                }
            }
            Self::Filtered { base, .. }
            | Self::Ranged { base, .. }
            | Self::Bucketed { base, .. } => base.settings(),
            Self::Fixed { base, .. } => add_settings(&base.settings(), &["join_use_nulls = 1"]),
        }
    }

    /// The table calculation called `name` that this relation carries,
    /// looking through the wrappers a builder puts around it.
    #[must_use]
    pub fn table_calc(&self, name: &str) -> Option<&TableCalc> {
        match self {
            Self::Mart(_) | Self::Sql(_) | Self::Joined { .. } => None,
            Self::Calculated {
                base, table_calcs, ..
            } => table_calcs
                .iter()
                .find(|t| t.name == name)
                .or_else(|| base.table_calc(name)),
            Self::Filtered { base, .. }
            | Self::Ranged { base, .. }
            | Self::Bucketed { base, .. }
            | Self::Fixed { base, .. } => base.table_calc(name),
        }
    }

    /// `base` for the chart's filters: moves `predicates` inside a `Fixed`
    /// join, so the aggregate is computed over the filtered rows, and returns
    /// what is left for the outer query (nothing, for a `Fixed` relation).
    #[must_use]
    pub fn with_filters_inside(self, predicates: Vec<String>) -> (Self, Vec<String>) {
        match self {
            Self::Fixed { base, joins } if !predicates.is_empty() => (
                Self::Fixed {
                    base: Box::new(Self::Filtered { base, predicates }),
                    joins,
                },
                Vec::new(),
            ),
            other => (other, predicates),
        }
    }
}

/// `settings` (empty or ` SETTINGS a, b`) with more settings appended.
pub(crate) fn add_settings(settings: &str, more: &[&str]) -> String {
    // BI-7: a joined relation under a `Fixed` join asks for the same setting
    // twice; a setting already there is not repeated.
    let more: Vec<&str> = more
        .iter()
        .copied()
        .filter(|m| !settings.contains(*m))
        .collect();
    if more.is_empty() {
        settings.to_owned()
    } else if settings.is_empty() {
        format!(" SETTINGS {}", more.join(", "))
    } else {
        format!("{settings}, {}", more.join(", "))
    }
}

/// A measure of a grouped chart as the statement reads it (`BI-6`): the
/// source column, the name of its result column, and the aggregate it carries
/// when it has one of its own (otherwise the chart's).
///
/// A measure without an aggregate of its own is named after its column, so a
/// chart saved before `BI-6` builds exactly the statement it always did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measure {
    /// The column that is aggregated.
    pub column: Ident,
    /// The result column's name.
    pub alias: Ident,
    /// The measure's own aggregate; `None` takes the chart's.
    pub aggregate: Option<Aggregate>,
}

impl Measure {
    /// A measure that takes the chart's aggregate and keeps its column's name.
    #[must_use]
    pub fn plain(column: Ident) -> Self {
        Self {
            alias: column.clone(),
            column,
            aggregate: None,
        }
    }
}

/// Which result column a chart is ordered by (`BI-6`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sort {
    /// The dimension, a grouping, or a measure's result name.
    pub by: Ident,
    /// Descending.
    pub desc: bool,
}

/// Builder for the general (non-KPI) chart SQL: `SELECT dimension, agg(measure)
/// ... FROM <relation> [WHERE ...] GROUP BY ... ORDER BY ... LIMIT ...`.
///
/// Ports the `buildSql` free function in `bi-store.ts` as a typestate
/// builder: [`QueryBuilder::measures`] must be called before
/// [`QueryBuilder::filter_in`] or [`QueryBuilder::build`] are available,
/// enforced at compile time rather than by convention.
pub struct QueryBuilder<S> {
    from: Relation,
    dimension: Option<Ident>,
    measures: Vec<Measure>,
    agg: Aggregate,
    order: String,
    limit: u32,
    breakdown: Option<Ident>,
    where_clauses: Vec<String>,
    grain: Option<(Grain, WeekStart)>,
    /// `BI-6`: an order that replaces `order`.
    sort: Option<Sort>,
    /// `BI-6`: gather the groups past `limit` into an `Other` group.
    other: bool,
    /// `BI-6`: the columns of a table grouped by several (replaces
    /// `dimension`).
    groupings: Vec<Ident>,
    /// `BI-6` part 2: steps applied to the grouped result.
    steps: Option<steps::StepsSql>,
    _state: PhantomData<S>,
}

impl QueryBuilder<NeedsProjection> {
    /// Start building a query against `mart` (unqualified, e.g.
    /// `mart_wisman` — the `serving.` prefix is added by [`Self::build`]).
    #[must_use]
    pub fn new(mart: Ident) -> Self {
        Self::over(Relation::Mart(mart))
    }

    /// Start building a query over any [`Relation`] (a mart or a SQL
    /// source).
    #[must_use]
    pub fn over(from: Relation) -> Self {
        Self {
            from,
            dimension: None,
            measures: Vec::new(),
            agg: Aggregate::Sum,
            order: "none".to_owned(),
            limit: 20,
            breakdown: None,
            where_clauses: Vec::new(),
            grain: None,
            sort: None,
            other: false,
            groupings: Vec::new(),
            steps: None,
            _state: PhantomData,
        }
    }

    /// Set the dimension (GROUP BY / X-axis column).
    #[must_use]
    pub fn dimension(mut self, dimension: Ident) -> Self {
        self.dimension = Some(dimension);
        self
    }

    /// Set the aggregate function.
    #[must_use]
    pub fn aggregate(mut self, agg: Aggregate) -> Self {
        self.agg = agg;
        self
    }

    /// Set the `ORDER BY` mode: `"asc"`, `"desc"`, or `"none"` (order by
    /// dimension instead).
    #[must_use]
    pub fn order(mut self, order: impl Into<String>) -> Self {
        self.order = order.into();
        self
    }

    /// Set the `LIMIT`.
    #[must_use]
    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = limit;
        self
    }

    /// Set the optional breakdown (2nd-dimension) column.
    #[must_use]
    pub fn breakdown(mut self, breakdown: Option<Ident>) -> Self {
        self.breakdown = breakdown;
        self
    }

    /// Mark the dimension as a grain bucket (`BI-9`). The relation must be a
    /// [`Relation::Bucketed`] so the dimension already is the bucket; this
    /// only changes how the result is ordered and cut: by bucket (day of week
    /// from the first day of the week), keeping the latest buckets when the
    /// order is by bucket.
    #[must_use]
    pub fn grain(mut self, grain: Grain, week_start: WeekStart) -> Self {
        self.grain = Some((grain, week_start));
        self
    }

    /// Order the groups by one result column (`BI-6`), in place of `order`.
    #[must_use]
    pub fn sort(mut self, sort: Option<Sort>) -> Self {
        self.sort = sort;
        self
    }

    /// Gather the groups past the limit into one `Other` group (`BI-6`).
    #[must_use]
    pub fn other(mut self, other: bool) -> Self {
        self.other = other;
        self
    }

    /// Group by several columns (`BI-6`, a table), instead of by `dimension`.
    #[must_use]
    pub fn groupings(mut self, groupings: Vec<Ident>) -> Self {
        self.groupings = groupings;
        self
    }

    /// Apply steps to the grouped result (`BI-6` part 2); the sort and limit
    /// then belong to the last step's result.
    #[must_use]
    pub fn steps(mut self, steps: Option<steps::StepsSql>) -> Self {
        self.steps = steps;
        self
    }

    /// Supply the measure columns, completing the projection and unlocking
    /// [`QueryBuilder::filter_in`]/[`QueryBuilder::build`]. Each takes the
    /// chart's aggregate and its column's name.
    #[must_use]
    pub fn measures(self, measures: Vec<Ident>) -> QueryBuilder<Ready> {
        self.measure_list(measures.into_iter().map(Measure::plain).collect())
    }

    /// [`Self::measures`] for measures that may carry an aggregate of their own.
    #[must_use]
    pub fn measure_list(self, measures: Vec<Measure>) -> QueryBuilder<Ready> {
        QueryBuilder {
            from: self.from,
            dimension: self.dimension,
            measures,
            agg: self.agg,
            order: self.order,
            limit: self.limit,
            breakdown: self.breakdown,
            where_clauses: self.where_clauses,
            grain: self.grain,
            sort: self.sort,
            other: self.other,
            groupings: self.groupings,
            steps: self.steps,
            _state: PhantomData,
        }
    }
}

impl QueryBuilder<Ready> {
    /// Add a `column IN (values...)` predicate. `values` are rendered
    /// through [`SqlLiteral`], never hand-escaped.
    #[must_use]
    pub fn filter_in(mut self, column: &Ident, values: &[String]) -> Self {
        if values.is_empty() {
            return self;
        }
        let list = values
            .iter()
            .map(|v| SqlLiteral::from(v.clone()).to_string())
            .collect::<Vec<_>>()
            .join(",");
        self.where_clauses.push(format!("{column} IN ({list})"));
        self
    }

    /// The order in the words `order` always used (`none`, `asc`, `desc`), for
    /// the rules that read it: a sort by the dimension is `none` (by bucket),
    /// a sort by a measure is a value order.
    pub(crate) fn order_words(&self) -> &str {
        match &self.sort {
            None => &self.order,
            Some(s) if self.is_dimension(&s.by) => "none",
            Some(s) if s.desc => "desc",
            Some(_) => "asc",
        }
    }

    fn is_dimension(&self, name: &Ident) -> bool {
        self.dimension.as_ref().is_some_and(|d| d == name)
    }

    /// The measure the chart is sorted by, with its position.
    pub(crate) fn sort_measure(&self) -> Option<(usize, &Measure)> {
        let sort = self.sort.as_ref()?;
        self.measures
            .iter()
            .enumerate()
            .find(|(_, m)| m.alias == sort.by)
    }

    /// Render the final SQL. Ports `buildSql` in `bi-store.ts`.
    ///
    /// `BI-6`: "Other", a sort by a measure beside a breakdown, and several
    /// groupings have their own statements; everything else is the statement
    /// it always was, with a sort (when set) in place of `order`.
    #[must_use]
    pub fn build(&self) -> String {
        // BI-8 part 2: a chart with a table calculation or a period comparison
        // has its own, staged statement.
        if let Some(sql) = self.staged_sql() {
            return sql;
        }
        if self.steps.is_some() {
            return self.steps_sql();
        }
        if self.other {
            return self.other_sql();
        }
        if !self.groupings.is_empty() {
            return self.groupings_sql();
        }
        if self.breakdown.is_some() && self.sort_measure().is_some() {
            return self.ranked_sql();
        }
        let from = self.from.render();
        let settings = self.from.settings();
        let where_sql = if self.where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {} ", self.where_clauses.join(" AND "))
        };
        // Dimension is always set by the callers in this crate before
        // `build()` is reachable (specFromInput / sqlWithFilters always
        // supply one); absent-dimension is not a state the TS reaches
        // either, since `buildSql` is only ever called with a `dimension`
        // string already validated non-empty.
        let dimension = self
            .dimension
            .as_ref()
            .map_or_else(String::new, ToString::to_string);
        // BI-9: a grained chart ordered by bucket keeps the LATEST buckets
        // and asks for one more than its limit, so the caller can tell a cut
        // from a result that fits (`grain::trim_to_latest`).
        // BI-9 review fix (BLOCKER) R1: a part of the date is never cut to
        // its latest buckets (the limit it carries is its whole range).
        let latest = self
            .grain
            .is_some_and(|(g, _)| grain::cuts_to_latest(g, self.order_words()));
        let bucket_limit = self.limit + u32::from(latest);
        let key = self.grain.map_or_else(
            || dimension.clone(),
            |(g, week_start)| g.order_key(&dimension, week_start),
        );
        // BI-6: a sort by the dimension keeps the groups it keeps and only
        // turns the final order round when it is descending.
        let key_dir = match &self.sort {
            Some(s) if self.is_dimension(&s.by) => {
                format!(" {}", if s.desc { "DESC" } else { "ASC" })
            }
            _ => String::new(),
        };

        if let Some(breakdown) = &self.breakdown {
            let Some(measure) = self.measures.first() else {
                return String::new();
            };
            let agg_expr = self.agg_of(measure);
            let outer = if self.where_clauses.is_empty() {
                "WHERE".to_owned()
            } else {
                format!("WHERE {} AND", self.where_clauses.join(" AND "))
            };
            // Which buckets are kept: the biggest by value, or (grained and
            // ordered by bucket) the latest.
            let keep_by = if latest {
                format!("{key} DESC")
            } else if !key_dir.is_empty() {
                // BI-6: a sort by the dimension keeps its first N in that order.
                format!("{key}{key_dir}")
            } else if let Some(field) = self.from.aggregate_field(measure.column.as_str()) {
                format!("{} DESC", field.sql)
            } else if self.measure_aggregate(measure) == Aggregate::Count {
                "count() DESC".to_owned()
            } else {
                format!(
                    "{}({}) DESC",
                    self.measure_aggregate(measure),
                    measure.column
                )
            };
            return format!(
                "SELECT {dimension}, {breakdown}, {agg_expr} FROM {from} \
                 {outer} {dimension} IN (SELECT {dimension} FROM {from} {where_sql}\
                 GROUP BY {dimension} ORDER BY {keep_by} LIMIT {bucket_limit}) \
                 GROUP BY {dimension}, {breakdown} ORDER BY {key}{key_dir}, {breakdown}{settings}",
            );
        }

        let (sel, renamed) = self.select_lists();
        let Some(order_clause) = self.order_clause(&key, &key_dir) else {
            return String::new();
        };
        if latest {
            let inner = format!(
                "SELECT {dimension}, {sel} FROM {from} {where_sql}GROUP BY {dimension} \
                 ORDER BY {key} DESC LIMIT {bucket_limit}"
            );
            return match renamed {
                Some(renamed) => format!(
                    "SELECT {dimension}, {renamed} FROM ({inner}) ORDER BY {key}{key_dir}{settings}"
                ),
                None => format!("SELECT * FROM ({inner}) ORDER BY {key}{key_dir}{settings}"),
            };
        }
        match renamed {
            Some(renamed) => format!(
                "SELECT {dimension}, {renamed} FROM (SELECT {dimension}, {sel} FROM {from} \
                 {where_sql}GROUP BY {dimension}) ORDER BY {order_clause} LIMIT {limit}{settings}",
                limit = self.limit,
            ),
            None => format!(
                "SELECT {dimension}, {sel} FROM {from} {where_sql}GROUP BY {dimension} ORDER BY {order_clause} LIMIT {limit}{settings}",
                limit = self.limit,
            ),
        }
    }

    /// The measures' select list, and when the engine would read one measure's
    /// result name as a column another measure aggregates, the list that
    /// renames them outside. `round(sum(v)) AS v` beside `round(avg(v)) AS
    /// avg_v` fails with error 184 (the second `v` is read as the first
    /// alias, an aggregate inside an aggregate; measured on `ClickHouse` 26.8),
    /// so such a chart aggregates under `__m<i>` names in an inner select and
    /// renames them in the outer. A chart whose names cannot clash keeps the
    /// statement it always had.
    fn select_lists(&self) -> (String, Option<String>) {
        let clash = self.measures.iter().enumerate().any(|(i, a)| {
            self.measures
                .iter()
                .enumerate()
                .any(|(j, b)| i != j && a.alias == b.column)
        });
        if !clash {
            let sel = self
                .measures
                .iter()
                .map(|m| self.agg_of(m))
                .collect::<Vec<_>>()
                .join(", ");
            return (sel, None);
        }
        let inner = self
            .measures
            .iter()
            .enumerate()
            .map(|(i, m)| format!("{} AS __m{i}", self.agg_expr(m)))
            .collect::<Vec<_>>()
            .join(", ");
        let outer = self
            .measures
            .iter()
            .enumerate()
            .map(|(i, m)| format!("__m{i} AS {}", m.alias))
            .collect::<Vec<_>>()
            .join(", ");
        (inner, Some(outer))
    }

    /// The `ORDER BY` of an ungrouped-by-breakdown chart: the sort when set
    /// (the dimension through `key`, a measure by its result name), else
    /// `order` (`none` is by `key`, a value order is by the first measure).
    /// `None` when the first measure a value order needs is missing.
    fn order_clause(&self, key: &str, key_dir: &str) -> Option<String> {
        Some(match &self.sort {
            Some(s) if !self.is_dimension(&s.by) => {
                format!("{} {}", s.by, if s.desc { "DESC" } else { "ASC" })
            }
            Some(_) => format!("{key}{key_dir}"),
            None if self.order == "none" => key.to_owned(),
            None => format!(
                "{} {}",
                self.measures.first()?.alias,
                if self.order == "asc" { "ASC" } else { "DESC" }
            ),
        })
    }

    fn agg_of(&self, measure: &Measure) -> String {
        format!("{} AS {}", self.agg_expr(measure), measure.alias)
    }

    /// The aggregate a measure is built with: its own, else the chart's.
    pub(crate) fn measure_aggregate(&self, measure: &Measure) -> Aggregate {
        measure.aggregate.unwrap_or(self.agg)
    }

    /// The measure's aggregate expression, without its alias.
    pub(crate) fn agg_expr(&self, measure: &Measure) -> String {
        // BI-8: an aggregate calculated field is the whole measure.
        if let Some(field) = self.from.aggregate_field(measure.column.as_str()) {
            field.sql.clone()
        } else if self.measure_aggregate(measure) == Aggregate::Count {
            "count()".to_owned()
        } else {
            format!(
                "round({}({}))",
                self.measure_aggregate(measure),
                measure.column
            )
        }
    }

    /// `BI-6`: a table grouped by several columns. The groupings that are cut
    /// by a grain are already buckets in the relation (nested
    /// [`Relation::Bucketed`]), so this is the ordinary `GROUP BY` over them;
    /// without a sort or an order it lists the groups in grouping order.
    fn groupings_sql(&self) -> String {
        let groups = self
            .groupings
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let (sel, renamed) = self.select_lists();
        let where_sql = if self.where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {} ", self.where_clauses.join(" AND "))
        };
        let order = match &self.sort {
            Some(s) => format!("{} {}, {groups}", s.by, if s.desc { "DESC" } else { "ASC" }),
            None if self.order == "none" => groups.clone(),
            None => {
                let Some(first) = self.measures.first() else {
                    return String::new();
                };
                format!(
                    "{} {}, {groups}",
                    first.alias,
                    if self.order == "asc" { "ASC" } else { "DESC" }
                )
            }
        };
        let from = self.from.render();
        let (limit, settings) = (self.limit, self.from.settings());
        match renamed {
            Some(renamed) => format!(
                "SELECT {groups}, {renamed} FROM (SELECT {groups}, {sel} FROM {from} \
                 {where_sql}GROUP BY {groups}) ORDER BY {order} LIMIT {limit}{settings}"
            ),
            None => format!(
                "SELECT {groups}, {sel} FROM {from} {where_sql}GROUP BY {groups} ORDER BY {order} LIMIT {limit}{settings}"
            ),
        }
    }
}

/// KPI SQL (single number, column `v`) with an optional WHERE clause. Ports
/// `buildKpiSql` in `bi-store.ts`.
#[must_use]
pub fn build_kpi_sql(
    from: &Relation,
    measure: &Ident,
    agg: Aggregate,
    where_clauses: &[String],
) -> String {
    let val = kpi_value(from, measure, agg);
    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", where_clauses.join(" AND "))
    };
    format!(
        "SELECT {val} AS v FROM {}{where_sql}{}",
        from.render(),
        from.settings()
    )
}

/// The number a single-row statement computes for `measure`.
fn kpi_value(from: &Relation, measure: &Ident, agg: Aggregate) -> String {
    if let Some(field) = from.aggregate_field(measure.as_str()) {
        // BI-8: an aggregate calculated field is the number itself.
        field.sql.clone()
    } else if agg == Aggregate::Count {
        "count()".to_owned()
    } else {
        format!("round({agg}({measure}))")
    }
}

/// The statement of a progress bar, or a bullet graph without a category
/// (`BI-16` part B): the value as `v` and, when the goal is a second value,
/// the goal as `g`, in one row so one filter narrows both.
///
/// With a goal the two aggregates are computed under `__m0` and `__m1` in an
/// inner select and renamed outside: `round(sum(x)) AS v, round(sum(v)) AS g`
/// would read the second `v` as the first alias (engine error 184, the same
/// clash [`QueryBuilder`] avoids for its measures). Without a goal it is
/// exactly [`build_kpi_sql`].
#[must_use]
pub fn build_progress_sql(
    from: &Relation,
    measure: &Ident,
    goal: Option<&Ident>,
    agg: Aggregate,
    where_clauses: &[String],
) -> String {
    let Some(goal) = goal else {
        return build_kpi_sql(from, measure, agg, where_clauses);
    };
    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", where_clauses.join(" AND "))
    };
    format!(
        "SELECT __m0 AS v, __m1 AS g FROM (SELECT {} AS __m0, {} AS __m1 FROM {}{where_sql}{})",
        kpi_value(from, measure, agg),
        kpi_value(from, goal, agg),
        from.render(),
        from.settings()
    )
}

/// A progress bar, or a bullet graph without a category, reads its value and,
/// for a goal that is a value, the goal in one row (`BI-16` part B).
fn single_row_sql(
    def: &ChartInput,
    from: &Relation,
    agg: Aggregate,
    where_clauses: &[String],
) -> Option<String> {
    let measure = Ident::new(def.measures.first()?.clone()).ok()?;
    let goal = def
        .types
        .goal_measure()
        .map(|g| Ident::new(g.to_owned()))
        .transpose()
        .ok()?;
    Some(build_progress_sql(
        from,
        &measure,
        goal.as_ref(),
        agg,
        where_clauses,
    ))
}

/// The most rows (ranges) a histogram's statement returns.
const HISTOGRAM_LIMIT: u32 = 100;

/// The name of a histogram's count column. Not the column's own name (the
/// range's lower edge carries that) and not one a table is likely to have.
pub const HISTOGRAM_COUNT: &str = "__count";

/// The statement of a histogram (`BI-16` part B): how many rows fall in each
/// range of one number column, in range order. It is `BI-10`'s binned path
/// with a count: the predicates sit under the bins on the raw rows, the
/// column's range is read inside the same statement, and the role rewrite
/// substitutes the governed projection at every read. `None` when the
/// definition no longer checks out.
fn histogram_sql(def: &ChartInput, from: &Relation, where_clauses: Vec<String>) -> Option<String> {
    let layers = binned::chart_layers(def)?;
    let column = layers.first()?.column.clone();
    let relation = binned::binned(from, where_clauses, &layers, &[column.to_string()]);
    let count = Ident::new(HISTOGRAM_COUNT).ok()?;
    Some(
        QueryBuilder::over(relation)
            .dimension(column.clone())
            .aggregate(Aggregate::Count)
            .order("none")
            .limit(HISTOGRAM_LIMIT)
            .measure_list(vec![Measure {
                column,
                alias: count,
                aggregate: Some(Aggregate::Count),
            }])
            .build(),
    )
}

/// Most tasks a Gantt chart keeps; the statement asks for one more so the tile
/// can say "more than 1,000" (`BI-16` part C1, plan D2).
pub const GANTT_LIMIT: u32 = 1001;

/// The first result column of a Gantt chart's statement, also the `y` of its
/// spec: the result has `task`, `group`, `start`, `end` and optionally
/// `progress` (the names the console reads).
pub const GANTT_START: &str = "start";

/// The columns a Gantt chart reads, as `Ident`s, from its definition.
struct GanttColumns {
    task: Ident,
    group: Option<Ident>,
    start: Ident,
    end: Ident,
    progress: Option<Ident>,
}

fn gantt_columns(def: &ChartInput) -> Option<GanttColumns> {
    let named = |v: &Option<String>| v.clone().filter(|c| !c.trim().is_empty());
    Some(GanttColumns {
        task: Ident::new(def.dimension.clone()).ok()?,
        group: match def.breakdown.as_deref().filter(|g| !g.is_empty()) {
            Some(g) => Some(Ident::new(g.to_owned()).ok()?),
            None => None,
        },
        start: Ident::new(named(&def.types.start)?).ok()?,
        end: Ident::new(named(&def.types.end)?).ok()?,
        progress: match named(&def.types.progress) {
            Some(p) => Some(Ident::new(p).ok()?),
            None => None,
        },
    })
}

/// A date or timestamp column as the tile reads it: a timestamp in the report
/// zone (a `Date` is never shifted).
fn gantt_instant<H: std::hash::BuildHasher>(
    column: &Ident,
    kinds: &HashMap<String, ColumnKind, H>,
    time: &TimeContext,
) -> String {
    if kinds.get(column.as_str()) == Some(&ColumnKind::DateTime) {
        time.in_zone_expr(&column.to_string())
    } else {
        column.to_string()
    }
}

/// A Gantt chart's statement (`BI-16` part C1, plan D2): one row per task and
/// group with its first start and last end (and mean progress), the first
/// [`GANTT_LIMIT`] by start. Rows that end before they start are left out and
/// counted by [`gantt_reversed_sql`] for the note. The inner select renames
/// the columns to `__`-prefixed names so a source column called `start` or
/// `end` can neither shadow nor be shadowed by the result's names, and the
/// dashboard's predicates sit under it on the raw rows, as in every other
/// builder here. `None` when the definition no longer checks out.
fn gantt_sql<H: std::hash::BuildHasher>(
    def: &ChartInput,
    from: &Relation,
    where_clauses: &[String],
    (kinds, time): (&HashMap<String, ColumnKind, H>, &TimeContext),
) -> Option<String> {
    let c = gantt_columns(def)?;
    let group = c
        .group
        .as_ref()
        .map_or_else(|| "''".to_owned(), ToString::to_string);
    let progress_inner = c
        .progress
        .as_ref()
        .map(|p| format!(", toFloat64({p}) AS __p"))
        .unwrap_or_default();
    let progress_outer = if c.progress.is_some() {
        ", avg(__p) AS `progress`"
    } else {
        ""
    };
    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", where_clauses.join(" AND "))
    };
    Some(format!(
        "SELECT __t AS `task`, __g AS `group`, min(__s) AS `start`, max(__e) AS `end`{progress_outer} \
         FROM (SELECT {task} AS __t, {group} AS __g, {start} AS __s, {end} AS __e{progress_inner} \
         FROM {from}{where_sql}) WHERE __e >= __s GROUP BY __t, __g ORDER BY min(__s), __t LIMIT {GANTT_LIMIT}{}",
        from.settings(),
        task = c.task,
        start = gantt_instant(&c.start, kinds, time),
        end = gantt_instant(&c.end, kinds, time),
        from = from.render(),
    ))
}

/// How many rows of a Gantt chart end before they start: they cannot be drawn,
/// and the tile says how many were left out (`reversedRows`).
fn gantt_reversed_sql<H: std::hash::BuildHasher>(
    def: &ChartInput,
    from: &Relation,
    where_clauses: &[String],
    (kinds, time): (&HashMap<String, ColumnKind, H>, &TimeContext),
) -> Option<String> {
    let c = gantt_columns(def)?;
    let mut clauses = where_clauses.to_vec();
    clauses.push(format!(
        "{} < {}",
        gantt_instant(&c.end, kinds, time),
        gantt_instant(&c.start, kinds, time)
    ));
    Some(format!(
        "SELECT count() AS n FROM {} WHERE {}{}",
        from.render(),
        clauses.join(" AND "),
        from.settings()
    ))
}

/// The statement a Gantt chart is saved with (no dashboard filter).
pub(crate) fn gantt_stored_sql(
    def: &ChartInput,
    from: &Relation,
    kinds: &HashMap<String, ColumnKind>,
    time: &TimeContext,
) -> Option<String> {
    gantt_sql(def, from, &[], (kinds, time))
}

/// The statement a histogram is saved with (no dashboard filter).
pub(crate) fn histogram_stored_sql(def: &ChartInput, from: &Relation) -> Option<String> {
    histogram_sql(def, from, Vec::new())
}

/// Boxplot SQL: per `dimension`, the five-number summary of `measure`
/// (`min`, quartiles, `max` via `quantilesExact`, returned as one array
/// column named after the measure), ordered by dimension. No aggregate
/// applies — the distribution is the point. `__n` is the row count behind
/// each summary: a category with one row collapses to a flat line, and the
/// renderer needs the count to say so instead of looking empty.
#[must_use]
pub fn build_boxplot_sql(
    from: &Relation,
    dimension: &Ident,
    measure: &Ident,
    where_clauses: &[String],
    limit: u32,
) -> String {
    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {} ", where_clauses.join(" AND "))
    };
    format!(
        "SELECT {dimension}, quantilesExact(0, 0.25, 0.5, 0.75, 1)(toFloat64({measure})) AS {measure}, \
         count() AS __n FROM {} {where_sql}GROUP BY {dimension} ORDER BY {dimension} LIMIT {limit}{}",
        from.render(),
        from.settings()
    )
}

/// Most points a `pointmap`/`geoheat` draws. Past a few thousand symbols a
/// map stops being readable and the tile slows down, so the query keeps the
/// top rows by value and the console says when the cap was reached.
pub const POINT_LIMIT: u32 = 5000;

/// Rows `ClickHouse` returns for a statement over a [`Relation::Sql`]; the
/// number in [`SQL_SOURCE_SETTINGS`].
const SQL_SOURCE_MAX_ROWS: u32 = 2000;

/// The row limit of a point map over `from`: [`POINT_LIMIT`] for a mart,
/// but never above what a SQL source may return. A `LIMIT` the cap would
/// undercut would let `ClickHouse` cut the result off by itself at an
/// unpredictable block boundary, and the console could not tell "the map
/// shows the top N" from "the result was cut".
#[must_use]
pub fn point_limit(from: &Relation) -> u32 {
    match from {
        Relation::Mart(_) | Relation::Joined { .. } => POINT_LIMIT,
        Relation::Sql(_) => POINT_LIMIT.min(SQL_SOURCE_MAX_ROWS),
        Relation::Filtered { base, .. }
        | Relation::Ranged { base, .. }
        | Relation::Bucketed { base, .. }
        | Relation::Calculated { base, .. }
        | Relation::Fixed { base, .. } => point_limit(base),
    }
}

/// Point-map SQL: one row per distinct (`lat`, `lon`[, `label`]) with the
/// aggregate of `measure`, the largest first, capped at [`point_limit`].
/// Rows with a NULL coordinate are dropped in SQL (they cannot be placed);
/// out-of-range coordinates are the console's to count and report.
#[must_use]
pub fn build_points_sql(
    from: &Relation,
    coords: (&Ident, &Ident),
    label: Option<&Ident>,
    measure: &Ident,
    agg: Aggregate,
    where_clauses: &[String],
) -> String {
    let (lat, lon) = coords;
    let mut predicates: Vec<String> = where_clauses.to_vec();
    predicates.push(format!("{lat} IS NOT NULL"));
    predicates.push(format!("{lon} IS NOT NULL"));
    let value = if agg == Aggregate::Count {
        format!("count() AS {measure}")
    } else {
        format!("round({agg}({measure})) AS {measure}")
    };
    let (label_select, label_group) = label.map_or_else(
        || (String::new(), String::new()),
        |l| (format!("{l}, "), format!(", {l}")),
    );
    format!(
        "SELECT {lat}, {lon}, {label_select}{value} FROM {} WHERE {} \
         GROUP BY {lat}, {lon}{label_group} ORDER BY {measure} DESC, {lat}, {lon} LIMIT {}{}",
        from.render(),
        predicates.join(" AND "),
        point_limit(from),
        from.settings()
    )
}

/// Why a filter did not apply to one tile's relation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// The relation has no column of that name (or the name is not a valid
    /// identifier).
    NoColumn,
    /// The column exists but its type does not fit the filter (a text match
    /// on a number, a date range on a string, a bound that is not a number).
    WrongType,
}

/// A filter that was left out of one tile's SQL, so the tile can say so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkippedFilter {
    /// The filter's column.
    pub column: String,
    /// Why it did not apply.
    pub reason: SkipReason,
}

/// A relation's columns with their kinds, by name.
pub type RelationColumns<H = std::collections::hash_map::RandomState> =
    HashMap<String, ColumnKind, H>;

/// The WHERE predicates for one relation, and the filters left out of it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FilterOutcome {
    /// Predicates to AND together.
    pub predicates: Vec<String>,
    /// Active filters that did not apply, in filter order.
    pub skipped: Vec<SkippedFilter>,
    /// The columns `predicates` read (with `tahun` for the year predicate),
    /// so a builder can tell whether one is also an output alias.
    pub columns: Vec<String>,
}

/// A chart's SQL with the filters that did not apply to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredSql {
    /// The statement to run.
    pub sql: String,
    /// Active filters left out of `sql`.
    pub skipped: Vec<SkippedFilter>,
    /// A dashboard grain switch this chart could not take (BI-9): it kept its
    /// own grain and the tile says so.
    pub grain_skipped: Option<Grain>,
    /// The grain `sql` was built with (the chart's own, or the dashboard's
    /// switch where it applied), so the tile can label its buckets.
    pub grain: Option<Grain>,
    /// The kind of the grained dimension (`Date` or `DateTime`), so a
    /// dashboard switch knows whether hour and minute are on offer.
    pub grain_column: Option<ColumnKind>,
    /// `Some(limit)` when `sql` keeps the latest `limit` buckets and was
    /// built to return one more, so the caller trims the surplus with
    /// [`grain::trim_to_latest`] and marks the tile `truncated`.
    pub latest_limit: Option<u32>,
    /// What a raw table, a pivot or a comparing KPI needs beyond its rows
    /// (`BI-16` part A).
    pub table: Option<TableRead>,
    /// `BI-10`: one statement per binned column, which reads the bins' width,
    /// minimum and maximum so the tile can label them. Empty without bins.
    pub bins: Vec<BinRead>,
}

/// What the tile of a raw table, a pivot or a KPI with a comparison carries
/// beyond its rows (`BI-16` part A), so the caller can finish the payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableRead {
    /// A raw table's first page: the statement that counts the rows the pages
    /// walk through.
    Rows {
        /// `SELECT count() AS n ...` over the same relation and predicates.
        count_sql: String,
    },
    /// A pivot: the most long-format rows it may keep; the statement asked for
    /// one more, so a surplus row means the cell cap cut the result.
    Pivot {
        /// Rows to keep ([`tables::pivot_row_cap`]).
        row_cap: u32,
    },
    /// A Gantt chart: the statement that counts the rows that end before they
    /// start, left out of the bars (`BI-16` part C1).
    Reversed {
        /// `SELECT count() AS n ...` over the same relation and predicates.
        count_sql: String,
    },
    /// A KPI that compares with its previous period: the period the buckets
    /// are cut by, so the tile can label them.
    Trend {
        /// The period.
        period: Grain,
    },
}

/// How a chart is read right now (BI-9): the deployment's time context, and
/// the grain the dashboard asked for in place of the charts' own
/// truncations.
#[derive(Debug, Clone, Copy)]
pub struct ReadContext<'a> {
    /// Report time zone and first day of the week.
    pub time: &'a TimeContext,
    /// The dashboard's grain switch, if any.
    pub grain: Option<Grain>,
    /// The calculated fields of every source (`BI-8`); a chart that names
    /// one of its source's fields is built over it.
    pub fields: &'a FieldCatalog,
}

impl<'a> ReadContext<'a> {
    /// No dashboard switch and no calculated fields: every chart keeps its
    /// own grain.
    #[must_use]
    pub fn new(time: &'a TimeContext) -> Self {
        Self {
            time,
            grain: None,
            fields: FieldCatalog::none(),
        }
    }
}

/// A chart's grain right now and what to build with it.
struct Grained {
    grain: Grain,
    column: ColumnKind,
    latest_limit: Option<u32>,
}

/// Resolve `spec`'s grain against the relation's columns.
fn grained<H: std::hash::BuildHasher>(
    spec: &StoredChartSpec,
    cols: &RelationColumns<H>,
    read: &ReadContext<'_>,
) -> (Option<Grained>, Option<Grain>) {
    let def: &ChartInput = &spec.def;
    let pivot = spec.spec.kind == crate::specs::ChartKind::Pivot;
    // A pivot has no `dimension`: its grain applies to its first date field.
    let column = if pivot {
        tables::pivot_grain_field(&def.tables, cols).map(|(_, kind)| kind)
    } else {
        cols.get(&def.dimension).copied()
    };
    let r = grain::resolve(def.grain.as_deref(), spec.spec.kind, column, read.grain);
    let g = r.effective.zip(column).map(|(grain, column)| {
        let order = def.effective_order();
        let order = order.as_str();
        let limit = grain::effective_limit(
            grain,
            order,
            grain::clamp_limit(spec.spec.kind, def.limit_value()),
        );
        Grained {
            grain,
            column,
            // A pivot is cut by its cell cap, not to its latest buckets.
            latest_limit: (!pivot && grain::cuts_to_latest(grain, order)).then_some(limit),
        }
    });
    (g, r.skipped)
}

/// SQL for a stored spec with runtime filters applied (year filter + the
/// dashboard's filters). `mart_cols` maps mart name to its columns with
/// their kinds, so we know which filters apply and which fit. Ports
/// `sqlWithFilters` in `bi-store.ts`; this is [`sql_with_filters_report`]
/// without the skipped list.
#[must_use]
pub fn sql_with_filters<HMap, HCols>(
    spec: &StoredChartSpec,
    years: &[i64],
    filters: &[FilterDef],
    mart_cols: &HashMap<String, RelationColumns<HCols>, HMap>,
    read: &ReadContext<'_>,
) -> String
where
    HMap: std::hash::BuildHasher,
    HCols: std::hash::BuildHasher + Default,
{
    sql_with_filters_report(spec, years, filters, mart_cols, read).sql
}

/// [`sql_with_filters`] plus the filters that were left out.
///
/// # Fidelity notes
///
/// - `kind == "text"` returns `""` — text tiles have no SQL.
/// - When the spec's `def.mart` is empty, the spec's stored `sql` is
///   returned unchanged (mirrors the TS `if (!mart) return spec.sql;`).
/// - The year predicate (`tahun IN (...)`) is added only when the mart
///   actually has a `tahun` column.
/// - A dashboard filter applies only when its column is *both* a valid
///   identifier *and* present in `mart_cols` — this double-check matters
///   because `ClickHouse` virtual columns like `_part`/`_shard_num` pass
///   [`Ident::new`] (a leading underscore is legal) but never appear in
///   `system.columns`, so the `mart_cols` membership check is what actually
///   keeps them out.
/// - If no predicates accumulate at all, the spec's stored `sql` is returned
///   unchanged (not a WHERE-less rebuild — this preserves the exact stored
///   SQL byte-for-byte when no filter applies). BI-9: except a chart that
///   carries a grain, which is always rebuilt, because its SQL depends on the
///   deployment's time settings (and the dashboard's grain switch), which can
///   change after the chart was saved.
#[must_use]
pub fn sql_with_filters_report<HMap, HCols>(
    spec: &StoredChartSpec,
    years: &[i64],
    filters: &[FilterDef],
    mart_cols: &HashMap<String, RelationColumns<HCols>, HMap>,
    read: &ReadContext<'_>,
) -> FilteredSql
where
    HMap: std::hash::BuildHasher,
    HCols: std::hash::BuildHasher + Default,
{
    let unfiltered = |skipped| FilteredSql {
        sql: spec.spec.sql.clone(),
        skipped,
        grain_skipped: None,
        grain: None,
        grain_column: None,
        latest_limit: None,
        table: None,
        bins: Vec::new(),
    };
    if spec.spec.kind.is_static() {
        return FilteredSql {
            sql: String::new(),
            skipped: Vec::new(),
            grain_skipped: None,
            grain: None,
            grain_column: None,
            latest_limit: None,
            table: None,
            bins: Vec::new(),
        };
    }
    let def: &ChartInput = &spec.def;
    if def.mart.is_empty() {
        return unfiltered(Vec::new());
    }
    let empty_cols: RelationColumns<HCols> = RelationColumns::default();
    let cols = mart_cols.get(&def.mart).unwrap_or(&empty_cols);
    // BI-7: the dashboard's filters apply to the columns of the whole flat
    // relation; one on a base column's name still means the base column.
    let Ok((filter_cols, joined)) = joined_relation(def, cols, mart_cols) else {
        return unfiltered(Vec::new());
    };
    let outcome = filter_predicates(&filter_cols, years, filters, read.time);
    // BI-16 part A: a raw table is rebuilt too, because its second statement
    // (the count) is built here and not stored; a comparing KPI because its
    // periods depend on the deployment's time settings. BI-8: so is a chart
    // that names a calculated field, whose formula may have changed.
    let kind = spec.spec.kind;
    let rebuilds = def.grain.is_some()
        || kind == crate::specs::ChartKind::Pivot
        // BI-16 part B: a histogram reads the bins' widths, so it is always
        // rebuilt (the tile carries them), like a binned chart.
        || kind == crate::specs::ChartKind::Histogram
        // BI-16 part C1: a Gantt chart carries the count of its reversed rows,
        // built here and not stored.
        || kind == crate::specs::ChartKind::Gantt
        || def.tables.is_rows_mode(kind)
        || def.tables.compares_to_previous(kind)
        || def.uses_shaping()
        || read.fields.chart_uses(def);
    if outcome.predicates.is_empty() && !rebuilds {
        return unfiltered(outcome.skipped);
    }
    // `mart` was already validated as a well-formed identifier when the
    // spec was created via `specFromInput`, so re-validating here would
    // only ever fail on data corruption; fall back to the stored SQL rather
    // than panicking, matching "SQL never comes raw from untrusted input"
    // without introducing a new failure mode.
    if Ident::new(def.mart.clone()).is_err() {
        return unfiltered(outcome.skipped);
    }
    // BI-7: a calculated field is compiled against the base table's columns
    // alone, and the joined columns are added to what the chart can name
    // afterwards.
    let from = joined.from.clone();
    let prepared = fields::prepare(
        read.fields.for_chart(def),
        def,
        from.clone(),
        cols,
        read.time,
    );
    match prepared {
        Ok(None) => report_over(
            spec,
            &from,
            &filter_cols,
            outcome,
            read,
            Some(&spec.spec.sql),
        ),
        Ok(Some(p)) => match joins::with_joined_columns(p.cols, &filter_cols, &joined.joined) {
            Ok(all) => report_over(spec, &p.from, &all, outcome, read, Some(&spec.spec.sql)),
            Err(_) => Some(unfiltered(outcome.skipped)),
        },
        // A field that no longer checks out: the chart keeps the SQL it was
        // saved with and the dashboard's error is the field's, never other
        // SQL built from a half-understood definition.
        Err(_) => Some(unfiltered(outcome.skipped)),
    }
    .unwrap_or_else(|| unfiltered(Vec::new()))
}

/// The relation a mart chart reads, joins included, and the columns the
/// dashboard's filters can name: the mart's own when it has no joins. `Err`
/// when a join no longer checks out (a table or column went away).
fn joined_relation<H, S>(
    def: &ChartInput,
    cols: &RelationColumns<H>,
    mart_cols: &HashMap<String, RelationColumns<H>, S>,
) -> Result<(RelationColumns, joins::ChartRelation), ()>
where
    H: std::hash::BuildHasher,
    S: std::hash::BuildHasher,
{
    let Ok(mart) = Ident::new(def.mart.clone()) else {
        return Err(());
    };
    if def.shape.joins_list().is_empty() {
        return Ok((
            cols.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            joins::ChartRelation {
                from: Relation::Mart(mart),
                cols: RelationColumns::default(),
                joined: Vec::new(),
                origins: HashMap::new(),
            },
        ));
    }
    let planned =
        joins::chart_relation(&mart, def.shape.joins_list(), mart_cols).map_err(|_| ())?;
    Ok((planned.cols.clone(), planned))
}

/// `outcome` (the year and the dashboard's filters) with the chart's own
/// filters (`BI-6`) in front: they are always applied, and the dashboard's can
/// only narrow them further. `cols` are the columns of the relation *after*
/// its calculated fields, so a chart can filter on a per-row field. `None`
/// when one of the chart's own filters cannot apply (its column is gone or its
/// type no longer fits): dropping it would show more rows than the author
/// chose, so the caller reports the tile instead of building a statement.
fn with_own_filters<H: std::hash::BuildHasher>(
    def: &ChartInput,
    cols: &RelationColumns<H>,
    mut outcome: FilterOutcome,
    read: &ReadContext<'_>,
) -> Option<FilterOutcome> {
    let time = read.time;
    let own = def.shape.own_filters();
    if own.is_empty() {
        return Some(outcome);
    }
    // BI-2: a segment is replaced by its filters; one that is gone fails the
    // tile like a filter that cannot apply, never shows more rows.
    let own = read.fields.expand_chart_filters(def, own).ok()?;
    let mut mine = filter_predicates(cols, &[], &own, time);
    if !mine.skipped.is_empty() {
        return None;
    }
    mine.predicates.append(&mut outcome.predicates);
    mine.columns.append(&mut outcome.columns);
    outcome.predicates = mine.predicates;
    outcome.columns = mine.columns;
    Some(outcome)
}

/// The grain, rebuilt statement and table read of one chart over `from`
/// (already carrying its calculated fields), with `cols` the columns that
/// relation offers. `stored` is the SQL to fall back to when the definition
/// no longer validates (a mart chart); a SQL-source chart passes `None` and
/// gets `None` back instead.
fn report_over<H: std::hash::BuildHasher>(
    spec: &StoredChartSpec,
    from: &Relation,
    cols: &RelationColumns<H>,
    outcome: FilterOutcome,
    read: &ReadContext<'_>,
    stored: Option<&str>,
) -> Option<FilteredSql> {
    let FilterOutcome {
        predicates,
        skipped,
        columns,
    } = with_own_filters(&spec.def, cols, outcome, read)?;
    // BI-8 part 2: a `Fixed` aggregate is computed over the filtered rows, so
    // the filters go inside the join. Nothing is left for the outer query,
    // which also means no filter column can be mistaken for an alias there.
    let (from, predicates) = from.clone().with_filters_inside(predicates);
    let columns = if predicates.is_empty() {
        Vec::new()
    } else {
        columns
    };
    let from = &from;
    let (g, grain_skipped) = grained(spec, cols, read);
    let latest_limit = g.as_ref().and_then(|g| g.latest_limit);
    // BI-10: what the tile needs to label the chart's bins, read over the same
    // filtered rows the bins are cut from.
    let bins = binned::all_layers(&spec.def, spec.spec.kind, cols)
        .map(|layers| binned::reads(from, &predicates, &layers))
        .unwrap_or_default();
    let rebuilt = rebuild(
        spec,
        from,
        predicates,
        &columns,
        &Rebuild {
            read,
            grained: g.as_ref(),
            cols,
        },
    );
    let (sql, table) = match (rebuilt, stored) {
        (Some(r), _) => (r.sql, r.table),
        (None, Some(stored)) => (stored.to_owned(), None),
        (None, None) => return None,
    };
    let grain = g
        .as_ref()
        .map(|g| g.grain)
        .or_else(|| trend_period(table.as_ref()));
    Some(FilteredSql {
        sql,
        skipped,
        grain_skipped,
        grain,
        grain_column: g.as_ref().map(|g| g.column),
        latest_limit,
        table,
        bins,
    })
}

/// The period a comparing KPI's buckets are cut by, when `table` says so.
fn trend_period(table: Option<&TableRead>) -> Option<Grain> {
    match table {
        Some(TableRead::Trend { period }) => Some(*period),
        _ => None,
    }
}

/// SQL for a stored chart built on a dashboard SQL source, with runtime
/// filters applied. Unlike [`sql_with_filters`] this ALWAYS rebuilds from
/// `source_sql` (the source's current text), never returning the SQL stored
/// with the chart, so editing a source updates every chart built on it —
/// the Metabase-model behaviour the plan chose. Filters apply to columns the
/// source actually returns (`source_cols`, probed when it was saved).
///
/// Returns `None` when the stored definition no longer yields valid
/// identifiers (data corruption); the caller reports that tile as an error
/// rather than running something else.
#[must_use]
pub fn sql_for_sql_source<HCols>(
    spec: &StoredChartSpec,
    source_sql: &str,
    source_cols: &RelationColumns<HCols>,
    years: &[i64],
    filters: &[FilterDef],
    read: &ReadContext<'_>,
) -> Option<String>
where
    HCols: std::hash::BuildHasher,
{
    sql_for_sql_source_report(spec, source_sql, source_cols, years, filters, read).map(|f| f.sql)
}

/// [`sql_for_sql_source`] plus the filters that were left out.
#[must_use]
pub fn sql_for_sql_source_report<HCols>(
    spec: &StoredChartSpec,
    source_sql: &str,
    source_cols: &RelationColumns<HCols>,
    years: &[i64],
    filters: &[FilterDef],
    read: &ReadContext<'_>,
) -> Option<FilteredSql>
where
    HCols: std::hash::BuildHasher,
{
    if spec.spec.kind.is_static() {
        return Some(FilteredSql {
            sql: String::new(),
            skipped: Vec::new(),
            grain_skipped: None,
            grain: None,
            grain_column: None,
            latest_limit: None,
            table: None,
            bins: Vec::new(),
        });
    }
    let outcome = filter_predicates(source_cols, years, filters, read.time);
    let from = Relation::Sql(source_sql.to_owned());
    // A field that no longer checks out is the chart's error (`None`, which
    // the caller shows on the tile); it never runs as something else.
    match fields::prepare(
        read.fields.for_chart(&spec.def),
        &spec.def,
        from.clone(),
        source_cols,
        read.time,
    ) {
        Ok(None) => report_over(spec, &from, source_cols, outcome, read, None),
        Ok(Some(p)) => report_over(spec, &p.from, &p.cols, outcome, read, None),
        Err(_) => None,
    }
}

/// The statements behind a records list (drill-down, BI-18·B): one page of
/// rows and the total number of rows they page through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordsSql {
    /// `SELECT * ... ORDER BY ALL LIMIT .. OFFSET ..`.
    pub rows: String,
    /// `SELECT count() AS n ...` over the same relation and predicates.
    pub count: String,
    /// Active filters the relation could not honour, as on the tile.
    pub skipped: Vec<SkippedFilter>,
}

/// What a records list is narrowed to.
#[derive(Debug, Clone, Copy)]
pub enum RecordsValue<'a> {
    /// Rows whose column equals the clicked text.
    Equals(&'a Ident, &'a str),
    /// Rows whose date column falls in the clicked bucket of a grained chart
    /// (BI-9): the bucket is computed by the server in the report zone and
    /// compared as the text the tile carries.
    Bucket {
        /// The raw dimension column.
        column: &'a Ident,
        /// Its kind.
        kind: ColumnKind,
        /// The chart's grain.
        grain: Grain,
        /// The bucket as printed by the tile.
        text: &'a str,
    },
    /// Rows whose number column falls in the clicked bin of a binned chart
    /// (`BI-10`): `[lower, lower + width)`, closed above when that edge is the
    /// maximum of the same filtered rows (the last bin holds the maximum).
    Bin {
        /// The raw number column.
        column: &'a Ident,
        /// The bin's lower edge.
        lower: f64,
        /// The bin's width.
        width: f64,
    },
}

/// SQL for the rows behind a clicked value, or behind a whole tile when
/// `value` is `None` (BI-18·B).
///
/// The relation and the dashboard's active `filters` go through the same
/// [`Relation::render`] and [`filter_predicates`] a tile uses, so a mart and
/// a SQL source share this one path and a filter means here exactly what it
/// means on the chart. The clicked value is a [`SqlLiteral`] and its column an
/// [`Ident`]. A SQL source's `SETTINGS` cap rides on both statements: the
/// count must not run an unbounded source either.
///
/// `ORDER BY ALL` makes `OFFSET` pages deterministic: without an order,
/// `ClickHouse` may return the same rows on two pages and skip others.
#[must_use]
pub fn records_sql<HCols>(
    from: &Relation,
    cols: &RelationColumns<HCols>,
    filters: &[FilterDef],
    value: Option<RecordsValue<'_>>,
    limit: u32,
    offset: u64,
    time: &TimeContext,
) -> RecordsSql
where
    HCols: std::hash::BuildHasher,
{
    let FilterOutcome {
        mut predicates,
        skipped,
        ..
    } = filter_predicates(cols, &[], filters, time);
    let from_sql = from.render();
    match value {
        Some(RecordsValue::Equals(column, value)) => {
            predicates.insert(0, format!("{column} = {}", SqlLiteral::from(value)));
        }
        Some(RecordsValue::Bin {
            column,
            lower,
            width,
        }) => {
            // The maximum is read over the rows the other predicates leave, so
            // the last bin is closed exactly as the tile's bins were cut.
            let others = if predicates.is_empty() {
                String::new()
            } else {
                format!(" WHERE {}", predicates.join(" AND "))
            };
            let max = crate::bin::max_scalar_sql(column, &format!("{from_sql}{others}"));
            predicates.insert(
                0,
                crate::bin::bin_records_predicate(column, lower, width, Some(&max)),
            );
        }
        Some(RecordsValue::Bucket {
            column,
            kind,
            grain,
            text,
        }) => {
            // A grain that does not fit the column matches nothing rather
            // than everything: an honest empty list.
            predicates.insert(
                0,
                grain
                    .records_predicate(column, kind, time, text)
                    .unwrap_or_else(|| "0".to_owned()),
            );
        }
        None => {}
    }
    let where_sql = if predicates.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", predicates.join(" AND "))
    };
    let settings = from.settings();
    RecordsSql {
        rows: format!(
            "SELECT * FROM {from_sql}{where_sql} ORDER BY ALL LIMIT {limit} OFFSET {offset}{settings}"
        ),
        count: format!("SELECT count() AS n FROM {from_sql}{where_sql}{settings}"),
        skipped,
    }
}

/// SQL for one page of a raw table (`BI-16` part A) through the records
/// route: the table's columns and sort, with the dashboard's active `filters`
/// applied exactly as a tile applies them ([`filter_predicates`]), so page two
/// agrees with the first page the dashboard carried. The page and the total
/// share the relation, so a SQL source's row cap rides on both.
#[must_use]
pub fn rows_page_sql<HCols>(
    from: &Relation,
    cols: &RelationColumns<HCols>,
    filters: &[FilterDef],
    plan: &tables::RowsPlan,
    (limit, offset): (u32, u64),
    time: &TimeContext,
) -> RecordsSql
where
    HCols: std::hash::BuildHasher,
{
    let FilterOutcome {
        predicates,
        skipped,
        ..
    } = filter_predicates(cols, &[], filters, time);
    let page = tables::rows_sql(from, plan, &predicates, limit, offset);
    RecordsSql {
        rows: page.rows,
        count: page.count,
        skipped,
    }
}

/// The `tahun IN (...)` and dashboard-filter predicates that apply to a
/// relation with columns `cols`, and the active filters that do not.
///
/// A dashboard filter applies only when its column is *both* a valid
/// identifier *and* present in `cols` — see [`sql_with_filters_report`]'s
/// fidelity notes for why both checks are needed. A filter whose op does not
/// fit the column's kind is skipped, never coerced (an honest "does not
/// apply" beats a silently different query).
///
/// Every value that reaches the SQL is a validated identifier, a
/// [`SqlLiteral`], a finite number printed by Rust, a date this module
/// parsed and re-printed, or one of the closed enums in [`crate::filters`];
/// no raw filter text is ever formatted in.
#[must_use]
pub fn filter_predicates<HCols>(
    cols: &RelationColumns<HCols>,
    years: &[i64],
    filters: &[FilterDef],
    time: &TimeContext,
) -> FilterOutcome
where
    HCols: std::hash::BuildHasher,
{
    let mut out = FilterOutcome::default();
    if !years.is_empty() && cols.contains_key("tahun") {
        let years_csv = years
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        out.predicates.push(format!("tahun IN ({years_csv})"));
        out.columns.push("tahun".to_owned());
    }
    for f in filters {
        if !f.is_active() {
            continue;
        }
        let skip = |reason| SkippedFilter {
            column: f.column.clone(),
            reason,
        };
        let (Ok(column), Some(kind)) = (Ident::new(f.column.clone()), cols.get(&f.column)) else {
            out.skipped.push(skip(SkipReason::NoColumn));
            continue;
        };
        match filter_predicate(f, &column, *kind, time) {
            Some(p) => {
                out.predicates.push(p);
                out.columns.push(f.column.clone());
            }
            None => out.skipped.push(skip(SkipReason::WrongType)),
        }
    }
    out
}

/// One filter's predicate over `column` of `kind`, or `None` when the op
/// does not fit that kind.
fn filter_predicate(
    f: &FilterDef,
    column: &Ident,
    kind: ColumnKind,
    time: &TimeContext,
) -> Option<String> {
    match f.op {
        // Kind-agnostic on purpose: it is what filters did before BI-18, and
        // a value list over a number column compares as the server coerces it.
        FilterOp::In | FilterOp::NotIn => {
            let list = f
                .values
                .iter()
                .map(|v| SqlLiteral::from(v.clone()).to_string())
                .collect::<Vec<_>>()
                .join(",");
            let keyword = if f.op == FilterOp::In { "IN" } else { "NOT IN" };
            Some(format!("{column} {keyword} ({list})"))
        }
        FilterOp::Between => between_predicate(f, column, kind, time),
        FilterOp::Relative => {
            if !kind.is_temporal() {
                return None;
            }
            relative_predicate(f, &time.day_expr(column, kind), time)
        }
        FilterOp::Contains | FilterOp::StartsWith | FilterOp::EndsWith | FilterOp::NotContains => {
            if kind != ColumnKind::Text {
                return None;
            }
            // Position / prefix / suffix functions rather than `ILIKE`, so
            // `%`, `_` and `\` in the needle are plain characters and there
            // is no wildcard escaping to get wrong. The UTF-8 variants keep
            // the match case-insensitive beyond ASCII.
            let needle = SqlLiteral::from(f.text.clone()?);
            Some(match f.op {
                FilterOp::Contains => {
                    format!("positionCaseInsensitiveUTF8(toString({column}), {needle}) > 0")
                }
                FilterOp::StartsWith => {
                    format!("startsWith(lowerUTF8(toString({column})), lowerUTF8({needle}))")
                }
                // BI-18 round two: a NULL value does not contain the text, so
                // it is kept; without `ifNull` the comparison is NULL and the
                // row would silently drop out.
                FilterOp::NotContains => format!(
                    "ifNull(positionCaseInsensitiveUTF8(toString({column}), {needle}) = 0, 1)"
                ),
                _ => format!("endsWith(lowerUTF8(toString({column})), lowerUTF8({needle}))"),
            })
        }
    }
}

/// `col >= min AND col <= max` over a number or date column, either end
/// optional. Ends are inclusive unless `min_exclusive` / `max_exclusive`
/// turn that side into `>` / `<` (BI-18 round two: "after", "before",
/// "greater than", "less than" must not include the named value).
fn between_predicate(
    f: &FilterDef,
    column: &Ident,
    kind: ColumnKind,
    time: &TimeContext,
) -> Option<String> {
    let (subject, render): (String, fn(&str) -> Option<String>) = match kind {
        ColumnKind::Number => (column.to_string(), |raw| {
            parse_number(raw).map(|v| v.to_string())
        }),
        // `toDate32`, not `toDate`: the validated range starts in 1900, which
        // `Date` cannot hold.
        ColumnKind::Date | ColumnKind::DateTime => (time.day_expr(column, kind), |raw| {
            parse_iso_date(raw).map(|(y, m, d)| format!("toDate32('{y:04}-{m:02}-{d:02}')"))
        }),
        ColumnKind::Text => return None,
    };
    let mut parts = Vec::new();
    let min_op = if f.min_exclusive { ">" } else { ">=" };
    let max_op = if f.max_exclusive { "<" } else { "<=" };
    for (op, bound) in [(min_op, &f.min), (max_op, &f.max)] {
        if let Some(raw) = bound.as_deref() {
            parts.push(format!("{subject} {op} {}", render(raw)?));
        }
    }
    match parts.len() {
        0 => None,
        1 => parts.pop(),
        _ => Some(format!("({})", parts.join(" AND "))),
    }
}

/// A date range relative to today over the date expression `d`. Only closed
/// enums and the bounded `n` reach the SQL.
///
/// `last n unit` is the `n` units ending today, both ends included
/// (`d > today - n units`); `next n unit` is the `n` units starting
/// tomorrow (`d > today AND d <= today + n units`); `this` and `previous` are
/// calendar periods. BI-9: "today" is the date in the report time zone
/// ([`TimeContext::today_expr`]) and a week starts on the configured first
/// day, not on the `ClickHouse` server's clock and Monday.
fn relative_predicate(f: &FilterDef, d: &str, time: &TimeContext) -> Option<String> {
    let (unit, anchor) = (f.unit?, f.anchor?);
    let today = time.today_expr();
    let (subtract, add, start) = match unit {
        RelativeUnit::Day => ("subtractDays", "addDays", today.clone()),
        RelativeUnit::Week => ("subtractWeeks", "addWeeks", time.week_start_expr()),
        RelativeUnit::Month => (
            "subtractMonths",
            "addMonths",
            format!("toStartOfMonth({today})"),
        ),
        RelativeUnit::Quarter => (
            "subtractQuarters",
            "addQuarters",
            format!("toStartOfQuarter({today})"),
        ),
        RelativeUnit::Year => (
            "subtractYears",
            "addYears",
            format!("toStartOfYear({today})"),
        ),
    };
    Some(match anchor {
        RelativeAnchor::Last => {
            let n =
                f.n.filter(|n| (1..=crate::filters::MAX_RELATIVE_N).contains(n))?;
            format!("({d} > {subtract}({today}, {n}) AND {d} <= {today})")
        }
        // The mirror of `last`: tomorrow through `n` units ahead, today out.
        RelativeAnchor::Next => {
            let n =
                f.n.filter(|n| (1..=crate::filters::MAX_RELATIVE_N).contains(n))?;
            format!("({d} > {today} AND {d} <= {add}({today}, {n}))")
        }
        RelativeAnchor::This => format!("({d} >= {start} AND {d} < {add}({start}, 1))"),
        RelativeAnchor::Previous => format!("({d} >= {subtract}({start}, 1) AND {d} < {start})"),
    })
}

/// What [`rebuild`] reads besides the definition and the predicates.
struct Rebuild<'a, H> {
    read: &'a ReadContext<'a>,
    grained: Option<&'a Grained>,
    /// The relation's columns, for the kinds that need a column's kind.
    cols: &'a RelationColumns<H>,
}

/// A rebuilt statement and what the tile needs besides its rows.
struct Rebuilt {
    sql: String,
    table: Option<TableRead>,
}

/// Rebuild a stored chart's SQL over `from` with `where_clauses`, from its
/// structured definition. `None` if the definition's identifiers no longer
/// validate (only possible through data corruption).
///
/// `BI-16` part A: a pivot, a raw table and a KPI that compares with its
/// previous period have their own statements ([`crate::tables`]); every other
/// kind goes through [`rebuild_chart`].
fn rebuild<H: std::hash::BuildHasher>(
    spec: &StoredChartSpec,
    from: &Relation,
    where_clauses: Vec<String>,
    where_columns: &[String],
    ctx: &Rebuild<'_, H>,
) -> Option<Rebuilt> {
    let def: &ChartInput = &spec.def;
    let kind = spec.spec.kind;
    let col_set = || -> std::collections::HashSet<String> { ctx.cols.keys().cloned().collect() };
    if kind == crate::specs::ChartKind::Pivot {
        let plan = tables::plan_pivot(&def.tables, &col_set()).ok()?;
        let grain = match ctx.grained {
            Some(g) => {
                let (field, _) = tables::pivot_grain_field(&def.tables, ctx.cols)?;
                Some((g.grain, field, g.column))
            }
            None => None,
        };
        let bin = match &def.shape.bin {
            Some(spec) => {
                let (field, _) = tables::pivot_bin_field(&def.tables, ctx.cols)?;
                Some((field, spec.mode().ok()?))
            }
            None => None,
        };
        let sql = tables::pivot_sql(
            from,
            &plan,
            where_clauses,
            (
                grain.as_ref().map(|(g, f, k)| (*g, f, *k)),
                bin.as_ref().map(|(f, m)| (f, *m)),
            ),
            ctx.read.time,
        )?;
        let row_cap = tables::pivot_row_cap(from, plan.values.len());
        return Some(Rebuilt {
            sql,
            table: Some(TableRead::Pivot { row_cap }),
        });
    }
    if def.tables.is_rows_mode(kind) {
        let plan = tables::plan_rows(&def.tables, &col_set()).ok()?;
        let page = tables::rows_sql(from, &plan, &where_clauses, tables::ROWS_PAGE_SIZE, 0);
        return Some(Rebuilt {
            sql: page.rows,
            table: Some(TableRead::Rows {
                count_sql: page.count,
            }),
        });
    }
    if def.tables.compares_to_previous(kind) {
        let filled = crate::metrics::fill_compare(
            &def.tables,
            &def.measures,
            ctx.read.fields.for_chart(def),
        );
        let plan = tables::plan_compare(&filled, &col_set(), ctx.cols).ok()??;
        let (tables::ComparePlan::Previous { period, .. }
        | tables::ComparePlan::LastYear { period, .. }) = &plan
        else {
            return None;
        };
        let measure = Ident::new(def.measures.first()?.clone()).ok()?;
        let agg = Aggregate::from_str_lossy(def.aggregate.as_deref().unwrap_or("sum"));
        let sql = tables::kpi_trend_sql(from, &measure, agg, where_clauses, &plan, ctx.read.time)?;
        return Some(Rebuilt {
            sql,
            table: Some(TableRead::Trend { period: *period }),
        });
    }
    if kind == crate::specs::ChartKind::Gantt {
        let inputs = (ctx.cols, ctx.read.time);
        let sql = gantt_sql(def, from, &where_clauses, inputs)?;
        let count_sql = gantt_reversed_sql(def, from, &where_clauses, inputs)?;
        return Some(Rebuilt {
            sql,
            table: Some(TableRead::Reversed { count_sql }),
        });
    }
    rebuild_chart(
        spec,
        from,
        where_clauses,
        where_columns,
        (ctx.read, ctx.cols),
        ctx.grained,
    )
    .map(|sql| Rebuilt { sql, table: None })
}

/// The statement a chart that uses `BI-6`'s fields is saved with: the one the
/// dashboard builds for it with no dashboard filter, its own filters applied,
/// in the deployment's time settings. `from` is the relation after the
/// chart's calculated fields, `cols` its columns. `None` when the definition
/// cannot be built (the caller refuses the save).
#[must_use]
pub fn stored_sql<H: std::hash::BuildHasher>(
    spec: &StoredChartSpec,
    from: &Relation,
    cols: &RelationColumns<H>,
    time: &TimeContext,
    fields: &FieldCatalog,
) -> Option<String> {
    // BI-2: the catalog is what expands a chart's segments; the saved
    // statement must be buildable with them or the save is refused.
    let read = ReadContext {
        fields,
        ..ReadContext::new(time)
    };
    let outcome = with_own_filters(&spec.def, cols, FilterOutcome::default(), &read)?;
    let FilterOutcome {
        predicates,
        columns,
        ..
    } = outcome;
    let (from, predicates) = from.clone().with_filters_inside(predicates);
    let (g, _) = grained(spec, cols, &read);
    rebuild(
        spec,
        &from,
        predicates,
        &columns,
        &Rebuild {
            read: &read,
            grained: g.as_ref(),
            cols,
        },
    )
    .map(|r| r.sql)
}

/// The names a chart's statement gives its result columns, which a predicate on
/// a column of the same name would otherwise resolve to (`BI-18·A` review
/// BLOCKER 1).
fn result_aliases(
    kind: crate::specs::ChartKind,
    def: &ChartInput,
    measures: &[Measure],
) -> Vec<String> {
    match kind {
        crate::specs::ChartKind::Kpi | crate::specs::ChartKind::Gauge => vec!["v".to_owned()],
        crate::specs::ChartKind::Progress => vec!["v".to_owned(), "g".to_owned()],
        crate::specs::ChartKind::Bullet if def.dimension.is_empty() => {
            vec!["v".to_owned(), "g".to_owned()]
        }
        crate::specs::ChartKind::Histogram => {
            vec![def.dimension.clone(), HISTOGRAM_COUNT.to_owned()]
        }
        // The rewritten select names its columns `__t` ...; a predicate on a
        // source column of the same name as a result column stays on the raw
        // rows (the predicates sit in the inner select).
        crate::specs::ChartKind::Gantt => Vec::new(),
        crate::specs::ChartKind::Boxplot => def
            .measures
            .first()
            .cloned()
            .into_iter()
            .chain(["__n".to_owned()])
            .collect(),
        crate::specs::ChartKind::Pointmap | crate::specs::ChartKind::Geoheat => {
            def.measures.first().cloned().into_iter().collect()
        }
        _ => measures.iter().map(|m| m.alias.to_string()).collect(),
    }
}

/// [`rebuild`] for the kinds that are a plain `GROUP BY` chart, a single
/// number or a point map.
fn rebuild_chart<H: std::hash::BuildHasher>(
    spec: &StoredChartSpec,
    from: &Relation,
    where_clauses: Vec<String>,
    where_columns: &[String],
    (read, cols): (&ReadContext<'_>, &RelationColumns<H>),
    grained: Option<&Grained>,
) -> Option<String> {
    let def: &ChartInput = &spec.def;
    // BI-16 part B: a histogram has its own statement over its bins.
    if spec.spec.kind == crate::specs::ChartKind::Histogram {
        return histogram_sql(def, from, where_clauses);
    }
    // BI-10: a chart whose dimension or breakdown is binned reads a relation
    // in which that column already is its bin, with the predicates under it on
    // the raw rows. Everything after this (sort, "Other", steps) is the
    // statement the chart would have had on that column as it is.
    let bins = binned::chart_layers(def)?;
    if !bins.is_empty() && def.shape.groupings.is_none() {
        let measures = shape::resolve_measures(&def.measures, from)?;
        let groups: Vec<String> = [Some(def.dimension.as_str()), def.breakdown.as_deref()]
            .into_iter()
            .flatten()
            .filter(|g| !g.is_empty())
            .map(str::to_owned)
            .collect();
        let needed = binned::needed_columns(from, &groups, &measures)?;
        let relation = binned::binned(from, where_clauses, &bins, &needed);
        return rebuild_chart_over(spec, &relation, Vec::new(), &[], (read, cols), None);
    }
    rebuild_chart_over(
        spec,
        from,
        where_clauses,
        where_columns,
        (read, cols),
        grained,
    )
}

/// [`rebuild_chart`] once any bin is already in the relation.
fn rebuild_chart_over<H: std::hash::BuildHasher>(
    spec: &StoredChartSpec,
    from: &Relation,
    where_clauses: Vec<String>,
    where_columns: &[String],
    (read, cols): (&ReadContext<'_>, &RelationColumns<H>),
    grained: Option<&Grained>,
) -> Option<String> {
    let def: &ChartInput = &spec.def;
    if let Some(g) = grained {
        let chart = GrainedChart::from_def(def, spec.spec.kind, g.grain, from)?;
        return grained_sql(from, where_clauses, &chart, g.column, read.time);
    }
    // BI-6: the result names of the measures (a measure with its own
    // aggregate is not named after its column).
    let measures = shape::resolve_measures(&def.measures, from)?;
    // BI-18·A review BLOCKER 1: a predicate on a column that the SELECT also
    // names as an alias would resolve to the alias (an aggregate) in WHERE.
    // Only then are the predicates moved into an inner relation; otherwise
    // the SQL is exactly what it was before.
    if !where_clauses.is_empty() {
        let aliases = result_aliases(spec.spec.kind, def, &measures);
        if where_columns.iter().any(|c| aliases.contains(c)) {
            let wrapped = Relation::Filtered {
                base: Box::new(from.clone()),
                predicates: where_clauses,
            };
            return rebuild_chart_over(spec, &wrapped, Vec::new(), &[], (read, cols), None);
        }
    }
    // `Aggregate::from_str_lossy` falls back to `Sum` for a missing OR
    // unrecognized value — this is the untrusted path named in the H4
    // finding (`def.aggregate` comes straight from stored `spec_json`, never
    // re-checked against an allowlist), so it must never hand raw text to
    // the SQL builder below.
    let agg = Aggregate::from_str_lossy(def.aggregate.as_deref().unwrap_or("sum"));

    if matches!(
        spec.spec.kind,
        crate::specs::ChartKind::Kpi | crate::specs::ChartKind::Gauge
    ) {
        let measure = Ident::new(def.measures.first()?.clone()).ok()?;
        return Some(build_kpi_sql(from, &measure, agg, &where_clauses));
    }
    if crate::types_b::is_single_row_kind(spec.spec.kind, def) {
        return single_row_sql(def, from, agg, &where_clauses);
    }

    if matches!(
        spec.spec.kind,
        crate::specs::ChartKind::Pointmap | crate::specs::ChartKind::Geoheat
    ) {
        // The dimension is only an optional label here, so it must not go
        // through the `Ident::new(&def.dimension)` below (empty is valid).
        let lat = Ident::new(def.lat.clone()?).ok()?;
        let lon = Ident::new(def.lon.clone()?).ok()?;
        let measure = Ident::new(def.measures.first()?.clone()).ok()?;
        if let Some(grid) = &def.shape.grid {
            return grid_sql(from, where_clauses, (&lat, &lon), &measure, (agg, grid));
        }
        let label = if def.dimension.is_empty() {
            None
        } else {
            Some(Ident::new(def.dimension.clone()).ok()?)
        };
        return Some(build_points_sql(
            from,
            (&lat, &lon),
            label.as_ref(),
            &measure,
            agg,
            &where_clauses,
        ));
    }

    if def.shape.groupings.is_some() {
        return groupings_sql(def, from, where_clauses, (read, cols), (&measures, agg));
    }
    let dimension = Ident::new(def.dimension.clone()).ok()?;
    if spec.spec.kind == crate::specs::ChartKind::Boxplot {
        let measure = Ident::new(def.measures.first()?.clone()).ok()?;
        return Some(build_boxplot_sql(
            from,
            &dimension,
            &measure,
            &where_clauses,
            def.limit_value().unwrap_or(20),
        ));
    }
    let breakdown = def
        .breakdown
        .as_ref()
        .and_then(|b| Ident::new(b.clone()).ok());
    let steps = plan_steps_of(def, &measures, &[(dimension.to_string(), None)], cols).ok()?;
    let sort = sort_of(
        def,
        &measures,
        std::slice::from_ref(&dimension),
        steps.as_ref(),
    )
    .ok()?;
    // BI-6: a chart that names a table calculation keeps the order and the
    // groups its calculation is defined over; a sort, "Other" or groupings
    // saved beside one (the field changed level since) is not built.
    let has_table_calc = measures
        .iter()
        .any(|m| from.table_calc(m.column.as_str()).is_some());
    if has_table_calc && (sort.is_some() || def.shape.wants_other() || steps.is_some()) {
        return None;
    }

    let mut builder = QueryBuilder::over(from.clone())
        .dimension(dimension)
        .aggregate(agg)
        .order(def.order.clone().unwrap_or_else(|| "none".to_owned()))
        .limit(def.limit_value().unwrap_or(20))
        .breakdown(breakdown)
        .sort(sort)
        .steps(steps)
        .other(def.shape.wants_other())
        .measure_list(measures);
    for clause in where_clauses {
        // The predicates were already assembled as complete SQL fragments
        // (through `SqlLiteral`) by `filter_predicates`; pass them through
        // as-is rather than re-deriving column/values.
        builder = builder.raw_where(clause);
    }
    Some(builder.build())
}

/// `BI-10`: a point map grouped by grid cell. The predicates and the
/// coordinate null checks sit in an inner relation, so they read the raw
/// coordinates and never the cell that is projected under their names.
fn grid_sql(
    from: &Relation,
    mut predicates: Vec<String>,
    (lat, lon): (&Ident, &Ident),
    measure: &Ident,
    (agg, grid): (Aggregate, &crate::bin::GridSpec),
) -> Option<String> {
    predicates.push(format!("{lat} IS NOT NULL"));
    predicates.push(format!("{lon} IS NOT NULL"));
    let src = Relation::Filtered {
        base: Box::new(from.clone()),
        predicates,
    };
    Some(crate::bin::grid_points_sql(
        &src.render(),
        (lat, lon),
        measure,
        agg,
        grid.checked().ok()?,
        (point_limit(from), &from.settings()),
    ))
}

/// The sort of a stored definition as validated identifiers: `by` must be one
/// of `groups` or a measure's result name, else `Err` (a definition that no
/// longer checks out is not built). `Ok(None)` is no sort at all.
fn sort_of(
    def: &ChartInput,
    measures: &[Measure],
    groups: &[Ident],
    steps: Option<&steps::StepsSql>,
) -> Result<Option<Sort>, ()> {
    let Some(sort) = &def.shape.sort else {
        return Ok(None);
    };
    let by = Ident::new(sort.by.clone()).map_err(|_| ())?;
    // With steps the sort names a column of the last step's result.
    let known = match steps {
        Some(plan) => plan.groups.contains(&by) || plan.measures.contains(&by),
        None => groups.contains(&by) || measures.iter().any(|m| m.alias == by),
    };
    if known {
        Ok(Some(Sort {
            by,
            desc: sort.descending(),
        }))
    } else {
        Err(())
    }
}

/// The steps of `def` planned over the groupings `groups` (each with the grain
/// it is cut by) and `measures` of its grouped result (`BI-6` part 2); `Ok(None)`
/// for a chart without steps, `Err` when they no longer check out.
fn plan_steps_of<H: std::hash::BuildHasher>(
    def: &ChartInput,
    measures: &[Measure],
    groups: &[(String, Option<Grain>)],
    cols: &RelationColumns<H>,
) -> Result<Option<steps::StepsSql>, ()> {
    let Some(list) = &def.shape.steps else {
        return Ok(None);
    };
    if def.grain.is_some() || def.breakdown.is_some() {
        return Err(());
    }
    let kinds = groups
        .iter()
        .map(|(name, grain)| {
            let kind = cols.get(name).copied().ok_or(())?;
            Ok((name.clone(), steps::group_kind(kind, *grain)))
        })
        .collect::<Result<Vec<_>, ()>>()?;
    let names: Vec<String> = measures.iter().map(|m| m.alias.to_string()).collect();
    steps::plan(list, &kinds, &names).map(Some).map_err(|_| ())
}

/// The statement of a table grouped by several columns (`BI-6`). A grouping
/// with a grain is replaced by its bucket in a nested [`Relation::Bucketed`]
/// (one per grouped column), the predicates sitting under all of them on the
/// raw rows, as for a grained chart.
fn groupings_sql<H: std::hash::BuildHasher>(
    def: &ChartInput,
    from: &Relation,
    where_clauses: Vec<String>,
    (read, cols): (&ReadContext<'_>, &RelationColumns<H>),
    (measures, agg): (&[Measure], Aggregate),
) -> Option<String> {
    let groupings = def.shape.groupings.as_ref()?;
    let idents: Vec<Ident> = groupings
        .iter()
        .map(|g| Ident::new(g.column.clone()).ok())
        .collect::<Option<_>>()?;
    let grains: Vec<Option<Grain>> = groupings
        .iter()
        .map(|g| g.grain.as_deref().map(Grain::parse))
        .map(|g| match g {
            None => Some(None),
            Some(parsed) => parsed.map(Some),
        })
        .collect::<Option<_>>()?;
    let steps = plan_steps_of(
        def,
        measures,
        &groupings
            .iter()
            .zip(&grains)
            .map(|(g, grain)| (g.column.clone(), *grain))
            .collect::<Vec<_>>(),
        cols,
    )
    .ok()?;
    let sort = sort_of(def, measures, &idents, steps.as_ref()).ok()?;
    let mut needed: Vec<String> = idents.iter().map(ToString::to_string).collect();
    for m in measures {
        match from.aggregate_field(m.column.as_str()) {
            // As in `grained_sql`: an aggregate that reads a grouped column
            // would read the bucket carrying that name.
            Some(field)
                if field
                    .reads
                    .iter()
                    .any(|r| idents.iter().any(|i| i.as_str() == r)) =>
            {
                return None;
            }
            Some(field) => needed.extend(field.reads.iter().cloned()),
            None if from.table_calc(m.column.as_str()).is_some() => return None,
            None => needed.push(m.column.to_string()),
        }
    }
    needed.dedup();
    let mut relation = from.clone();
    let mut predicates = where_clauses;
    // BI-10: a grouping with a bin is replaced by its bin after the grains,
    // over the same filtered rows.
    let bins = binned::grouping_layers(def)?;
    if grains.iter().any(Option::is_some) || !bins.is_empty() {
        if !predicates.is_empty() {
            relation = Relation::Filtered {
                base: Box::new(relation),
                predicates: std::mem::take(&mut predicates),
            };
        }
        for ((ident, grain), group) in idents.iter().zip(&grains).zip(groupings) {
            let Some(grain) = grain else {
                continue;
            };
            // The column's kind decides whether the grain fits it.
            let expr = grain.bucket_expr(ident, *cols.get(&group.column)?, read.time)?;
            relation = Relation::Bucketed {
                base: Box::new(relation),
                dimension: ident.to_string(),
                expr,
                carried: needed
                    .iter()
                    .filter(|n| n.as_str() != ident.as_str())
                    .cloned()
                    .collect(),
            };
        }
    }
    for layer in &bins {
        relation = binned::add_layer(relation, layer, &needed);
    }
    let mut builder = QueryBuilder::over(relation)
        .groupings(idents)
        .aggregate(agg)
        .order(def.order.clone().unwrap_or_else(|| "none".to_owned()))
        .limit(def.limit_value().unwrap_or(20))
        .sort(sort)
        .steps(steps)
        .measure_list(measures.to_vec());
    for clause in predicates {
        builder = builder.raw_where(clause);
    }
    Some(builder.build())
}

/// The parts of a chart definition a grained query reads, as validated
/// identifiers.
#[derive(Debug)]
pub struct GrainedChart {
    /// The grain applied to the dimension.
    pub grain: Grain,
    /// The date or timestamp dimension.
    pub dimension: Ident,
    /// The optional second dimension.
    pub breakdown: Option<Ident>,
    /// The measures.
    pub measures: Vec<Measure>,
    /// The aggregate.
    pub agg: Aggregate,
    /// `"none"` (by bucket), `"asc"` or `"desc"` (by value).
    pub order: String,
    /// The bucket limit.
    pub limit: u32,
    /// `BI-6`: a sort that replaces `order`.
    pub sort: Option<Sort>,
}

impl GrainedChart {
    /// Read the identifiers out of a stored definition over `from` (whose
    /// calculated fields decide the measures' result names). `None` when one
    /// no longer validates (data corruption).
    #[must_use]
    pub fn from_def(
        def: &ChartInput,
        kind: crate::specs::ChartKind,
        grain: Grain,
        from: &Relation,
    ) -> Option<Self> {
        let measures = shape::resolve_measures(&def.measures, from)?;
        let dimension = Ident::new(def.dimension.clone()).ok()?;
        let sort = sort_of(def, &measures, std::slice::from_ref(&dimension), None).ok()?;
        let order = def.effective_order();
        let limit =
            grain::effective_limit(grain, &order, grain::clamp_limit(kind, def.limit_value()));
        Some(Self {
            grain,
            dimension,
            breakdown: def
                .breakdown
                .as_ref()
                .and_then(|b| Ident::new(b.clone()).ok()),
            measures,
            agg: Aggregate::from_str_lossy(def.aggregate.as_deref().unwrap_or("sum")),
            order,
            limit,
            sort,
        })
    }
}

/// The SQL of a grained chart over `from` with `predicates` applied to the
/// raw rows (BI-9). The predicates go into an inner relation
/// ([`Relation::Filtered`]) under the bucketing one ([`Relation::Bucketed`]),
/// so a filter on the dimension's own column reads the raw column, never the
/// bucket that carries its name. `None` when the grain does not apply to
/// `column`.
#[must_use]
pub fn grained_sql(
    from: &Relation,
    predicates: Vec<String>,
    chart: &GrainedChart,
    column: ColumnKind,
    time: &TimeContext,
) -> Option<String> {
    let expr = chart.grain.bucket_expr(&chart.dimension, column, time)?;
    let base = if predicates.is_empty() {
        from.clone()
    } else {
        Relation::Filtered {
            base: Box::new(from.clone()),
            predicates,
        }
    };
    let mut carried: Vec<String> = Vec::new();
    let mut carry = |name: String| {
        if name != chart.dimension.as_str() && !carried.contains(&name) {
            carried.push(name);
        }
    };
    if let Some(b) = &chart.breakdown {
        carry(b.to_string());
    }
    for m in &chart.measures {
        let column = m.column.as_str();
        match from.aggregate_field(column) {
            // BI-8: an aggregate field is computed from the columns it reads,
            // not carried by name. If it reads the grouped column itself it
            // would read the bucket that now carries that name, so no
            // statement is built (the caller reports the chart, never runs
            // a different one).
            Some(field) if field.reads.iter().any(|c| c == chart.dimension.as_str()) => {
                return None;
            }
            Some(field) => field.reads.iter().cloned().for_each(&mut carry),
            // BI-8 part 2: a table calculation is computed from the columns its
            // hidden aggregates read, as an aggregate field is.
            None if from.table_calc(column).is_some() => {
                let calc = from.table_calc(column);
                if calc.is_some_and(|c| c.reads.iter().any(|r| r == chart.dimension.as_str())) {
                    return None;
                }
                calc.into_iter()
                    .flat_map(|c| c.reads.iter().cloned())
                    .for_each(&mut carry);
            }
            None => carry(m.column.to_string()),
        }
    }
    let bucketed = Relation::Bucketed {
        base: Box::new(base),
        dimension: chart.dimension.to_string(),
        expr,
        carried,
    };
    Some(
        QueryBuilder::over(bucketed)
            .dimension(chart.dimension.clone())
            .aggregate(chart.agg)
            .order(chart.order.clone())
            .limit(chart.limit)
            .breakdown(chart.breakdown.clone())
            .grain(chart.grain, time.week_start())
            .sort(chart.sort.clone())
            .measure_list(chart.measures.clone())
            .build(),
    )
}

impl QueryBuilder<Ready> {
    /// Append a pre-built WHERE fragment verbatim (used internally by
    /// [`sql_with_filters`], which already assembled `tahun IN (...)` /
    /// `<col> IN (...)` fragments through [`SqlLiteral`] before this point).
    #[must_use]
    fn raw_where(mut self, clause: String) -> Self {
        self.where_clauses.push(clause);
        self
    }
}

/// Next-token values [`apply_builtin_year_filter`] considers safe to
/// precede with a freshly inserted `WHERE` clause — every `ClickHouse`
/// clause keyword that can legally follow a bare `FROM <table>` with no
/// alias/`JOIN`/existing predicate in between, plus the closing `)` of a
/// subquery. Anything else (an alias, `WHERE`, `PREWHERE`, `JOIN`, `FINAL`,
/// ...) means inserting here would produce invalid or silently wrong SQL,
/// so the caller degrades to unfiltered instead.
const SAFE_NEXT_TOKENS: [&str; 8] = [
    "GROUP", "ORDER", "LIMIT", "HAVING", "SETTINGS", "FORMAT", "UNION", ")",
];

/// Applies a `WHERE tahun IN (...)` predicate to a built-in `KpiSpec`/
/// `ChartSpec`'s raw SQL, for the dashboard's year selector — [`sql_with_filters`]
/// itself cannot be reused for a built-in spec, since it needs a
/// `ChartInput`, which built-ins do not have.
///
/// Inserted as plain text, right after the literal substring `FROM
/// serving.<mart>`, but only when three boundary conditions all hold — see
/// [`SAFE_NEXT_TOKENS`] and the guards below — because built-in specs are
/// loaded from a deployment-supplied `BUILTIN_DASHBOARD_SPEC` file (WS6),
/// not just this repo's own 13 shipped entries, so this function cannot
/// assume the input is as simple as those 13 always were. Returns `sql`
/// completely unchanged (an honest "unfiltered", never a corrupted
/// insertion) when: `years` is empty; `mart` is not in `mart_cols` or has
/// no `tahun` column (checked the same way [`sql_with_filters`] checks it
/// for stored charts); the marker `FROM serving.<mart>` does not appear, or
/// appears more than once (ambiguous — e.g. a `UNION` reading the same mart
/// twice); the character immediately after the marker is neither
/// whitespace nor end-of-string (a prefix collision, e.g. `mart_event`
/// matching inside `mart_event_detail`); or the next real token after the
/// marker is not in [`SAFE_NEXT_TOKENS`] (an alias, an existing
/// `WHERE`/`PREWHERE`, a `JOIN`, `FINAL`, or anything else this function
/// does not recognize as safe to precede).
#[must_use]
pub fn apply_builtin_year_filter<HMap, HCols>(
    sql: &str,
    mart: &str,
    years: &[i64],
    mart_cols: &HashMap<String, RelationColumns<HCols>, HMap>,
) -> String
where
    HMap: std::hash::BuildHasher,
    HCols: std::hash::BuildHasher,
{
    if years.is_empty() {
        return sql.to_owned();
    }
    let Some(cols) = mart_cols.get(mart) else {
        return sql.to_owned();
    };
    if !cols.contains_key("tahun") {
        return sql.to_owned();
    }

    let marker = format!("FROM serving.{mart}");
    if sql.matches(&marker).count() != 1 {
        return sql.to_owned();
    }
    let Some(pos) = sql.find(&marker) else {
        return sql.to_owned();
    };
    let after = pos + marker.len();
    let rest = &sql[after..];

    // Guard 1: the character right after the marker must be end-of-string
    // or whitespace, never an identifier-continuation character.
    match rest.chars().next() {
        None => {}
        Some(c) if c.is_whitespace() => {}
        _ => return sql.to_owned(),
    }

    // Guard 2: the next real token (if any) must be one of SAFE_NEXT_TOKENS.
    let next_token_ok = match rest.split_whitespace().next() {
        None => true,
        Some(token) => {
            let upper = token.to_ascii_uppercase();
            SAFE_NEXT_TOKENS.contains(&upper.as_str())
        }
    };
    if !next_token_ok {
        return sql.to_owned();
    }

    let years_csv = years
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!("{} WHERE tahun IN ({years_csv}){rest}", &sql[..after])
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use super::*;
    use crate::specs::ChartKind;

    // BI-9: the public builders now take the time settings. The tests that
    // predate it call these shims, which pass the default context
    // (`Asia/Jakarta`, Monday) and no dashboard grain switch; the grain tests
    // call the real functions.
    static DEFAULT_TIME: std::sync::LazyLock<TimeContext> =
        std::sync::LazyLock::new(TimeContext::default);

    fn read() -> ReadContext<'static> {
        ReadContext::new(&DEFAULT_TIME)
    }

    fn sql_with_filters<HMap, HCols>(
        spec: &StoredChartSpec,
        years: &[i64],
        filters: &[FilterDef],
        mart_cols: &HashMap<String, RelationColumns<HCols>, HMap>,
    ) -> String
    where
        HMap: std::hash::BuildHasher,
        HCols: std::hash::BuildHasher + Default,
    {
        super::sql_with_filters(spec, years, filters, mart_cols, &read())
    }

    fn sql_with_filters_report<HMap, HCols>(
        spec: &StoredChartSpec,
        years: &[i64],
        filters: &[FilterDef],
        mart_cols: &HashMap<String, RelationColumns<HCols>, HMap>,
    ) -> FilteredSql
    where
        HMap: std::hash::BuildHasher,
        HCols: std::hash::BuildHasher + Default,
    {
        super::sql_with_filters_report(spec, years, filters, mart_cols, &read())
    }

    fn sql_for_sql_source<HCols: std::hash::BuildHasher>(
        spec: &StoredChartSpec,
        source_sql: &str,
        source_cols: &RelationColumns<HCols>,
        years: &[i64],
        filters: &[FilterDef],
    ) -> Option<String> {
        super::sql_for_sql_source(spec, source_sql, source_cols, years, filters, &read())
    }

    fn sql_for_sql_source_report<HCols: std::hash::BuildHasher>(
        spec: &StoredChartSpec,
        source_sql: &str,
        source_cols: &RelationColumns<HCols>,
        years: &[i64],
        filters: &[FilterDef],
    ) -> Option<FilteredSql> {
        super::sql_for_sql_source_report(spec, source_sql, source_cols, years, filters, &read())
    }

    fn filter_predicates<HCols: std::hash::BuildHasher>(
        cols: &RelationColumns<HCols>,
        years: &[i64],
        filters: &[FilterDef],
    ) -> FilterOutcome {
        super::filter_predicates(cols, years, filters, &DEFAULT_TIME)
    }

    fn records_sql<HCols: std::hash::BuildHasher>(
        from: &Relation,
        cols: &RelationColumns<HCols>,
        filters: &[FilterDef],
        value: Option<(&Ident, &str)>,
        limit: u32,
        offset: u64,
    ) -> RecordsSql {
        super::records_sql(
            from,
            cols,
            filters,
            value.map(|(c, v)| RecordsValue::Equals(c, v)),
            limit,
            offset,
            &DEFAULT_TIME,
        )
    }

    /// Every column is `Text`: the legacy `in` filters these tests exercise
    /// do not depend on a column's kind.
    fn text_cols(names: &[&str]) -> RelationColumns {
        names
            .iter()
            .map(|c| ((*c).to_owned(), ColumnKind::Text))
            .collect()
    }

    fn mart_cols(pairs: &[(&str, &[&str])]) -> HashMap<String, RelationColumns> {
        pairs
            .iter()
            .map(|(mart, cols)| ((*mart).to_owned(), text_cols(cols)))
            .collect()
    }

    fn stored_spec(
        kind: ChartKind,
        mart: &str,
        dimension: &str,
        measures: &[&str],
    ) -> StoredChartSpec {
        let sql = "SELECT 1".to_owned();
        StoredChartSpec::for_test(kind, mart, dimension, measures, sql)
    }

    #[test]
    fn returns_base_sql_when_no_filters_apply() {
        let spec = stored_spec(ChartKind::Bar, "mart_wisman", "kawasan", &["jumlah"]);
        let cols = mart_cols(&[("mart_wisman", &["kawasan", "jumlah"])]);
        let sql = sql_with_filters(&spec, &[], &[], &cols);
        assert_eq!(sql, spec.spec.sql);
    }

    #[test]
    fn adds_year_predicate_when_column_exists() {
        let spec = stored_spec(ChartKind::Bar, "mart_wisman", "kawasan", &["jumlah"]);
        let cols = mart_cols(&[("mart_wisman", &["kawasan", "jumlah", "tahun"])]);
        let sql = sql_with_filters(&spec, &[2023, 2024], &[], &cols);
        assert!(sql.contains("WHERE tahun IN (2023,2024)"), "{sql}");
    }

    #[test]
    fn skips_year_predicate_when_column_absent() {
        let spec = stored_spec(ChartKind::Bar, "mart_wisman", "kawasan", &["jumlah"]);
        let cols = mart_cols(&[("mart_wisman", &["kawasan", "jumlah"])]);
        let sql = sql_with_filters(&spec, &[2023], &[], &cols);
        assert_eq!(sql, spec.spec.sql);
    }

    #[test]
    fn skips_filter_whose_column_is_not_in_the_mart() {
        let spec = stored_spec(ChartKind::Bar, "mart_wisman", "kawasan", &["jumlah"]);
        let cols = mart_cols(&[("mart_wisman", &["kawasan", "jumlah"])]);
        let filters = vec![FilterDef::in_values("negara", vec!["ID".to_owned()])];
        let sql = sql_with_filters(&spec, &[], &filters, &cols);
        assert_eq!(sql, spec.spec.sql);
    }

    #[test]
    fn escapes_quote_inside_filter_value() {
        let spec = stored_spec(ChartKind::Bar, "mart_wisman", "kawasan", &["jumlah"]);
        let cols = mart_cols(&[("mart_wisman", &["kawasan", "jumlah"])]);
        let filters = vec![FilterDef::in_values("kawasan", vec!["O'Brien".to_owned()])];
        let sql = sql_with_filters(&spec, &[], &filters, &cols);
        assert!(sql.contains("'O''Brien'"), "{sql}");
    }

    #[test]
    fn rejects_filter_column_that_is_not_a_valid_identifier() {
        let spec = stored_spec(ChartKind::Bar, "mart_wisman", "kawasan", &["jumlah"]);
        let mut cols = mart_cols(&[("mart_wisman", &["kawasan", "jumlah"])]);
        cols.get_mut("mart_wisman")
            .unwrap()
            .insert("bad col".to_owned(), ColumnKind::Text);
        let filters = vec![FilterDef::in_values("bad col", vec!["x".to_owned()])];
        let sql = sql_with_filters(&spec, &[], &filters, &cols);
        assert_eq!(sql, spec.spec.sql);
    }

    #[test]
    fn text_chart_produces_no_sql() {
        let spec = stored_spec(ChartKind::Text, "", "", &[]);
        let cols: HashMap<String, RelationColumns> = HashMap::new();
        let sql = sql_with_filters(&spec, &[2024], &[], &cols);
        assert_eq!(sql, "");
    }

    /// BI-18 part C: a tile that carries no data never has a statement, even
    /// when its stored definition holds a mart, a dimension and SQL (a row
    /// written by hand, or a text tile edited into a heading). The section
    /// kinds are checked by the same `is_static` the text tile is.
    #[test]
    fn a_section_tile_produces_no_sql_whatever_its_stored_definition_holds() {
        let cols = mart_cols(&[("mart_wisman", &["kawasan", "jumlah", "tahun"])]);
        let filters = vec![FilterDef::in_values("kawasan", vec!["Asia".to_owned()])];
        for kind in [ChartKind::Heading, ChartKind::Divider, ChartKind::Section] {
            let mut spec = stored_spec(kind, "mart_wisman", "kawasan", &["jumlah"]);
            spec.spec.sql = "SELECT secret FROM serving.mart_wisman".to_owned();
            assert_eq!(sql_with_filters(&spec, &[2024], &filters, &cols), "");
            let report = sql_with_filters_report(&spec, &[2024], &filters, &cols);
            assert_eq!(report.sql, "", "{kind:?}");
            assert!(report.skipped.is_empty(), "{kind:?}");
        }
    }

    #[test]
    fn kpi_kind_routes_to_kpi_builder() {
        let spec = stored_spec(ChartKind::Kpi, "mart_wisman", "", &["jumlah"]);
        let cols = mart_cols(&[("mart_wisman", &["jumlah", "tahun"])]);
        let sql = sql_with_filters(&spec, &[2024], &[], &cols);
        assert!(
            sql.starts_with(
                "SELECT round(sum(jumlah)) AS v FROM serving.mart_wisman WHERE tahun IN (2024)"
            ),
            "{sql}"
        );
    }

    #[test]
    fn virtual_column_underscore_part_is_rejected_by_mart_cols_check() {
        let spec = stored_spec(ChartKind::Bar, "mart_wisman", "kawasan", &["jumlah"]);
        // `_part` passes `Ident::new` (leading underscore is legal) but is
        // never in `system.columns`, so `mart_cols` correctly omits it.
        assert!(Ident::new("_part").is_ok());
        let cols = mart_cols(&[("mart_wisman", &["kawasan", "jumlah"])]);
        let filters = vec![FilterDef::in_values("_part", vec!["all_0_0_0".to_owned()])];
        let sql = sql_with_filters(&spec, &[], &filters, &cols);
        assert_eq!(sql, spec.spec.sql);
    }

    #[test]
    fn apply_builtin_year_filter_inserts_where_after_the_mart_clause() {
        let cols = mart_cols(&[("mart_wisman", &["tahun", "jumlah"])]);
        let sql = "SELECT sum(jumlah) AS v FROM serving.mart_wisman";
        let filtered = apply_builtin_year_filter(sql, "mart_wisman", &[2023, 2024], &cols);
        assert_eq!(
            filtered,
            "SELECT sum(jumlah) AS v FROM serving.mart_wisman WHERE tahun IN (2023,2024)"
        );
    }

    #[test]
    fn apply_builtin_year_filter_preserves_a_trailing_order_by() {
        let cols = mart_cols(&[("mart_event", &["tahun", "jumlah_event"])]);
        let sql =
            "SELECT jumlah_event AS v, tahun FROM serving.mart_event ORDER BY tahun DESC LIMIT 1";
        let filtered = apply_builtin_year_filter(sql, "mart_event", &[2024], &cols);
        assert_eq!(
            filtered,
            "SELECT jumlah_event AS v, tahun FROM serving.mart_event WHERE tahun IN (2024) \
             ORDER BY tahun DESC LIMIT 1"
        );
    }

    #[test]
    fn apply_builtin_year_filter_is_a_no_op_when_years_is_empty() {
        let cols = mart_cols(&[("mart_wisman", &["tahun"])]);
        let sql = "SELECT 1 FROM serving.mart_wisman";
        assert_eq!(
            apply_builtin_year_filter(sql, "mart_wisman", &[], &cols),
            sql
        );
    }

    #[test]
    fn apply_builtin_year_filter_is_a_no_op_when_the_mart_has_no_tahun_column() {
        let cols = mart_cols(&[("mart_kuliner", &["wilayah", "jumlah_usaha"])]);
        let sql = "SELECT wilayah FROM serving.mart_kuliner";
        assert_eq!(
            apply_builtin_year_filter(sql, "mart_kuliner", &[2024], &cols),
            sql
        );
    }

    #[test]
    fn apply_builtin_year_filter_is_a_no_op_when_the_from_clause_is_not_found() {
        // Defensive: a spec whose SQL does not read `FROM serving.<mart>`
        // directly (none of the shipped built-ins do this) degrades to
        // unfiltered rather than corrupting the query.
        let cols = mart_cols(&[("mart_x", &["tahun"])]);
        let sql = "SELECT 1"; // no FROM clause at all
        assert_eq!(
            apply_builtin_year_filter(sql, "mart_x", &[2024], &cols),
            sql
        );
    }

    // -- boundary guards (judge review W6-2): specs are now loaded from a
    // deployment-supplied JSON file (WS6), so "none of the 13 shipped
    // specs has a WHERE" no longer bounds the input this function sees. Each
    // test below is one of the three failure modes the review named,
    // asserting the safe degradation: SQL returned unchanged, never a
    // corrupted insertion. ---------------------------------------------------

    #[test]
    fn apply_builtin_year_filter_rejects_a_prefix_collision_with_a_longer_mart_name() {
        // `mart = "mart_event"`'s marker, "FROM serving.mart_event", is
        // also a PREFIX of "FROM serving.mart_event_detail" — a naive
        // substring match would insert the predicate mid-identifier.
        let cols = mart_cols(&[("mart_event", &["tahun"])]);
        let sql = "SELECT 1 FROM serving.mart_event_detail";
        assert_eq!(
            apply_builtin_year_filter(sql, "mart_event", &[2024], &cols),
            sql
        );
    }

    #[test]
    fn apply_builtin_year_filter_rejects_an_aliased_from_clause() {
        // `FROM serving.mart_wisman AS w` must not become `... WHERE tahun
        // IN (2024) AS w` — `AS` is not one of the safe next tokens.
        let cols = mart_cols(&[("mart_wisman", &["tahun"])]);
        let sql = "SELECT 1 FROM serving.mart_wisman AS w";
        assert_eq!(
            apply_builtin_year_filter(sql, "mart_wisman", &[2024], &cols),
            sql
        );
    }

    #[test]
    fn apply_builtin_year_filter_rejects_an_existing_where_clause() {
        let cols = mart_cols(&[("mart_wisman", &["tahun"])]);
        let sql = "SELECT 1 FROM serving.mart_wisman WHERE region = 'x'";
        assert_eq!(
            apply_builtin_year_filter(sql, "mart_wisman", &[2024], &cols),
            sql
        );
    }

    #[test]
    fn apply_builtin_year_filter_rejects_a_join_after_the_mart_clause() {
        let cols = mart_cols(&[("mart_wisman", &["tahun"])]);
        let sql = "SELECT 1 FROM serving.mart_wisman JOIN serving.mart_event USING (tahun)";
        assert_eq!(
            apply_builtin_year_filter(sql, "mart_wisman", &[2024], &cols),
            sql
        );
    }

    #[test]
    fn apply_builtin_year_filter_rejects_final_after_the_mart_clause() {
        let cols = mart_cols(&[("mart_wisman", &["tahun"])]);
        let sql = "SELECT 1 FROM serving.mart_wisman FINAL";
        assert_eq!(
            apply_builtin_year_filter(sql, "mart_wisman", &[2024], &cols),
            sql
        );
    }

    #[test]
    fn apply_builtin_year_filter_rejects_when_the_marker_appears_more_than_once() {
        // A UNION reading the same mart twice makes "the" insertion point
        // ambiguous — refuse rather than guess which occurrence to filter.
        let cols = mart_cols(&[("mart_wisman", &["tahun"])]);
        let sql = "SELECT 1 FROM serving.mart_wisman UNION ALL SELECT 1 FROM serving.mart_wisman";
        assert_eq!(
            apply_builtin_year_filter(sql, "mart_wisman", &[2024], &cols),
            sql
        );
    }

    const SOURCE_SQL: &str = "SELECT g.material_group, g.materials, t.material_type\n\
        FROM serving.mart_material_by_group AS g\n\
        JOIN serving.mart_material_by_type AS t USING (material_group) -- joined";

    fn source_spec(kind: ChartKind, dimension: &str, measures: &[&str]) -> StoredChartSpec {
        let mut spec = stored_spec(kind, "", dimension, measures);
        spec.def.sql_source = Some("s_1234abcd".to_owned());
        spec.spec.sql_source = Some("s_1234abcd".to_owned());
        spec
    }

    fn source_cols() -> RelationColumns {
        text_cols(&["material_group", "materials", "material_type"])
    }

    #[test]
    fn a_sql_source_is_wrapped_as_a_derived_table_with_the_caps_appended() {
        let sql = QueryBuilder::over(Relation::Sql(SOURCE_SQL.to_owned()))
            .dimension(Ident::new("material_group").unwrap())
            .aggregate(Aggregate::Sum)
            .order("desc")
            .limit(10)
            .measures(vec![Ident::new("materials").unwrap()])
            .build();
        assert!(
            sql.contains(&format!("FROM (\n{SOURCE_SQL}\n) AS src ")),
            "the source must be a derived table, closed on its own line so a trailing comment cannot swallow the parenthesis: {sql}"
        );
        assert!(sql.ends_with(SQL_SOURCE_SETTINGS), "{sql}");
        assert!(!sql.contains("serving.material_group"), "{sql}");
    }

    #[test]
    fn a_mart_query_gets_no_settings_suffix() {
        let sql = QueryBuilder::new(Ident::new("mart_x").unwrap())
            .dimension(Ident::new("d").unwrap())
            .measures(vec![Ident::new("m").unwrap()])
            .build();
        assert!(sql.contains("FROM serving.mart_x "), "{sql}");
        assert!(!sql.contains("SETTINGS"), "{sql}");
    }

    #[test]
    fn a_kpi_over_a_sql_source_is_capped_too() {
        let sql = build_kpi_sql(
            &Relation::Sql("SELECT 1 AS n".to_owned()),
            &Ident::new("n").unwrap(),
            Aggregate::Sum,
            &[],
        );
        assert_eq!(
            sql,
            format!(
                "SELECT round(sum(n)) AS v FROM (\nSELECT 1 AS n\n) AS src{SQL_SOURCE_SETTINGS}"
            )
        );
    }

    #[test]
    fn a_breakdown_over_a_sql_source_wraps_both_references() {
        let sql = QueryBuilder::over(Relation::Sql("SELECT a, b, v FROM serving.t".to_owned()))
            .dimension(Ident::new("a").unwrap())
            .breakdown(Some(Ident::new("b").unwrap()))
            .measures(vec![Ident::new("v").unwrap()])
            .build();
        assert_eq!(sql.matches("AS src").count(), 2, "{sql}");
        assert_eq!(sql.matches("SETTINGS").count(), 1, "{sql}");
    }

    #[test]
    fn a_source_chart_is_rebuilt_from_the_current_source_even_without_filters() {
        let spec = source_spec(ChartKind::Bar, "material_group", &["materials"]);
        let sql = sql_for_sql_source(&spec, "SELECT 2 AS edited", &source_cols(), &[], &[])
            .expect("a valid definition rebuilds");
        assert!(sql.contains("SELECT 2 AS edited"), "{sql}");
        assert_ne!(
            sql, spec.spec.sql,
            "the SQL stored with the chart is never reused"
        );
    }

    #[test]
    fn dashboard_filters_apply_only_to_columns_the_source_returns() {
        let spec = source_spec(ChartKind::Bar, "material_group", &["materials"]);
        let filters = vec![
            FilterDef::in_values("material_type", vec!["ROH".to_owned()]),
            FilterDef::in_values("not_in_source", vec!["x".to_owned()]),
        ];
        let sql = sql_for_sql_source(&spec, SOURCE_SQL, &source_cols(), &[2024], &filters).unwrap();
        assert!(sql.contains("material_type IN ('ROH')"), "{sql}");
        assert!(!sql.contains("not_in_source"), "{sql}");
        assert!(
            !sql.contains("tahun"),
            "the source has no tahun column: {sql}"
        );
    }

    #[test]
    fn a_corrupt_source_chart_definition_yields_none_not_other_sql() {
        let spec = source_spec(ChartKind::Bar, "bad column", &["materials"]);
        assert_eq!(
            sql_for_sql_source(&spec, SOURCE_SQL, &source_cols(), &[], &[]),
            None
        );
    }

    #[test]
    fn a_boxplot_is_five_quantiles_per_category_with_no_aggregate() {
        let sql = build_boxplot_sql(
            &Relation::Mart(Ident::new("mart_x").unwrap()),
            &Ident::new("region").unwrap(),
            &Ident::new("amount").unwrap(),
            &["tahun IN (2024)".to_owned()],
            10,
        );
        assert_eq!(
            sql,
            "SELECT region, quantilesExact(0, 0.25, 0.5, 0.75, 1)(toFloat64(amount)) AS amount, \
             count() AS __n FROM serving.mart_x WHERE tahun IN (2024) GROUP BY region ORDER BY region LIMIT 10"
        );
    }

    #[test]
    fn a_boxplot_on_a_sql_source_is_rebuilt_as_quantiles_with_the_caps() {
        let spec = source_spec(ChartKind::Boxplot, "material_group", &["materials"]);
        let sql = sql_for_sql_source(&spec, SOURCE_SQL, &source_cols(), &[], &[]).unwrap();
        assert!(
            sql.contains("quantilesExact(0, 0.25, 0.5, 0.75, 1)(toFloat64(materials))"),
            "{sql}"
        );
        assert!(sql.ends_with(SQL_SOURCE_SETTINGS), "{sql}");
    }

    fn point_spec(kind: ChartKind, label: &str) -> StoredChartSpec {
        let mut spec = stored_spec(kind, "mart_x", label, &["visitors"]);
        spec.def.lat = Some("lat".to_owned());
        spec.def.lon = Some("lon".to_owned());
        spec.spec.lat = Some("lat".to_owned());
        spec.spec.lon = Some("lon".to_owned());
        spec
    }

    #[test]
    fn a_point_map_selects_coordinates_label_and_value_largest_first() {
        let sql = build_points_sql(
            &Relation::Mart(Ident::new("mart_x").unwrap()),
            (&Ident::new("lat").unwrap(), &Ident::new("lon").unwrap()),
            Some(&Ident::new("place").unwrap()),
            &Ident::new("visitors").unwrap(),
            Aggregate::Sum,
            &["tahun IN (2024)".to_owned()],
        );
        assert_eq!(
            sql,
            "SELECT lat, lon, place, round(sum(visitors)) AS visitors FROM serving.mart_x \
             WHERE tahun IN (2024) AND lat IS NOT NULL AND lon IS NOT NULL \
             GROUP BY lat, lon, place ORDER BY visitors DESC, lat, lon LIMIT 5000"
        );
    }

    #[test]
    fn a_point_map_without_a_label_groups_by_the_coordinates_alone_and_counts_rows() {
        let sql = build_points_sql(
            &Relation::Mart(Ident::new("mart_x").unwrap()),
            (&Ident::new("lat").unwrap(), &Ident::new("lon").unwrap()),
            None,
            &Ident::new("visitors").unwrap(),
            Aggregate::Count,
            &[],
        );
        assert_eq!(
            sql,
            "SELECT lat, lon, count() AS visitors FROM serving.mart_x \
             WHERE lat IS NOT NULL AND lon IS NOT NULL \
             GROUP BY lat, lon ORDER BY visitors DESC, lat, lon LIMIT 5000"
        );
    }

    #[test]
    fn a_stored_point_map_is_rebuilt_with_the_year_filter_and_its_coordinates() {
        let spec = point_spec(ChartKind::Geoheat, "");
        let cols = mart_cols(&[("mart_x", &["tahun", "lat", "lon", "visitors"])]);
        let sql = sql_with_filters(&spec, &[2024], &[], &cols);
        assert!(
            sql.starts_with("SELECT lat, lon, round(sum(visitors)) AS visitors FROM serving.mart_x WHERE tahun IN (2024) AND lat IS NOT NULL"),
            "{sql}"
        );
    }

    #[test]
    fn a_point_map_on_a_sql_source_is_limited_to_what_the_source_may_return() {
        // `LIMIT 5000` over a 2000-row cap would be cut off by ClickHouse
        // itself; the limit is the cap so "exactly the limit" is meaningful.
        assert_eq!(SQL_SOURCE_MAX_ROWS, 2000);
        assert!(
            SQL_SOURCE_SETTINGS.contains(&format!("max_result_rows = {SQL_SOURCE_MAX_ROWS}")),
            "SQL_SOURCE_MAX_ROWS must stay the number in SQL_SOURCE_SETTINGS"
        );
        let mut spec = point_spec(ChartKind::Pointmap, "materials");
        spec.def.sql_source = Some("s_1234abcd".to_owned());
        let cols = text_cols(&["lat", "lon", "visitors", "materials"]);
        let sql = sql_for_sql_source(&spec, SOURCE_SQL, &cols, &[], &[]).unwrap();
        assert!(sql.contains(" LIMIT 2000 SETTINGS"), "{sql}");
        assert!(sql.ends_with(SQL_SOURCE_SETTINGS), "{sql}");
    }

    #[test]
    fn a_point_map_missing_its_coordinates_yields_none_not_other_sql() {
        let mut spec = point_spec(ChartKind::Pointmap, "");
        spec.def.lon = None;
        let cols = mart_cols(&[("mart_x", &["tahun"])]);
        assert_eq!(
            sql_for_sql_source(&spec, SOURCE_SQL, &source_cols(), &[], &[]),
            None
        );
        // With a mart and a filter the stored SQL stands in, as for any
        // chart whose definition no longer validates.
        assert_eq!(sql_with_filters(&spec, &[2024], &[], &cols), "SELECT 1");
    }

    // ── typed filters (BI-18 part A, T2) ────────────────────────────────

    fn typed(pairs: &[(&str, ColumnKind)]) -> RelationColumns {
        pairs.iter().map(|(n, k)| ((*n).to_owned(), *k)).collect()
    }

    fn filter(json: &str) -> FilterDef {
        serde_json::from_str(json).unwrap()
    }

    /// The one predicate `filters` produce over `cols`, asserting nothing
    /// was skipped.
    fn one_predicate(cols: &[(&str, ColumnKind)], json: &str) -> String {
        let out = filter_predicates(&typed(cols), &[], &[filter(json)]);
        assert!(out.skipped.is_empty(), "{:?}", out.skipped);
        assert_eq!(out.predicates.len(), 1, "{:?}", out.predicates);
        out.predicates[0].clone()
    }

    fn skipped(cols: &[(&str, ColumnKind)], json: &str) -> Vec<SkippedFilter> {
        let out = filter_predicates(&typed(cols), &[], &[filter(json)]);
        assert!(out.predicates.is_empty(), "{:?}", out.predicates);
        out.skipped
    }

    #[test]
    fn not_in_negates_the_value_list() {
        let p = one_predicate(
            &[("region", ColumnKind::Text)],
            r#"{"column":"region","op":"not_in","values":["Bali","O'Brien"]}"#,
        );
        assert_eq!(p, "region NOT IN ('Bali','O''Brien')");
    }

    #[test]
    fn a_number_range_renders_both_ends_inclusive_as_numbers() {
        let p = one_predicate(
            &[("price", ColumnKind::Number)],
            r#"{"column":"price","op":"between","min":"100","max":"2.5e3"}"#,
        );
        assert_eq!(p, "(price >= 100 AND price <= 2500)");
    }

    #[test]
    fn an_open_ended_range_has_only_the_bound_that_is_present() {
        let cols = [("price", ColumnKind::Number)];
        assert_eq!(
            one_predicate(&cols, r#"{"column":"price","op":"between","min":"-5"}"#),
            "price >= -5"
        );
        assert_eq!(
            one_predicate(&cols, r#"{"column":"price","op":"between","max":"7.25"}"#),
            "price <= 7.25"
        );
    }

    #[test]
    fn a_date_range_renders_re_printed_dates_and_a_datetime_compares_by_day() {
        let json = r#"{"column":"d","op":"between","min":"2024-01-05","max":"2024-12-31"}"#;
        assert_eq!(
            one_predicate(&[("d", ColumnKind::Date)], json),
            "(d >= toDate32('2024-01-05') AND d <= toDate32('2024-12-31'))"
        );
        assert_eq!(
            one_predicate(&[("d", ColumnKind::DateTime)], json),
            "(toDate32(toTimeZone(d, 'Asia/Jakarta')) >= toDate32('2024-01-05') AND toDate32(toTimeZone(d, 'Asia/Jakarta')) <= toDate32('2024-12-31'))"
        );
    }

    #[test]
    fn a_bound_that_does_not_fit_the_column_kind_is_skipped_not_coerced() {
        let wrong = vec![SkippedFilter {
            column: "d".to_owned(),
            reason: SkipReason::WrongType,
        }];
        // A number bound on a date column, a date bound on a number column,
        // and a range over text.
        assert_eq!(
            skipped(
                &[("d", ColumnKind::Date)],
                r#"{"column":"d","op":"between","min":"5"}"#
            ),
            wrong
        );
        assert_eq!(
            skipped(
                &[("d", ColumnKind::Number)],
                r#"{"column":"d","op":"between","min":"2024-01-01"}"#
            ),
            wrong
        );
        assert_eq!(
            skipped(
                &[("d", ColumnKind::Text)],
                r#"{"column":"d","op":"between","min":"1"}"#
            ),
            wrong
        );
    }

    #[test]
    fn a_bound_that_is_neither_number_nor_date_never_reaches_sql() {
        for bad in [
            "1; DROP TABLE x",
            "NaN",
            "inf",
            "2024-02-30",
            "0x10",
            "' OR 1=1 --",
        ] {
            let json = serde_json::json!({"column": "c", "op": "between", "min": bad});
            let f: FilterDef = serde_json::from_value(json).unwrap();
            assert!(f.validate().is_err(), "{bad}");
            for kind in [ColumnKind::Number, ColumnKind::Date, ColumnKind::DateTime] {
                let out = filter_predicates(&typed(&[("c", kind)]), &[], std::slice::from_ref(&f));
                assert!(out.predicates.is_empty(), "{bad}: {:?}", out.predicates);
            }
        }
    }

    #[test]
    fn relative_dates_use_only_the_enum_and_the_bounded_number() {
        let cols = [("d", ColumnKind::Date)];
        assert_eq!(
            one_predicate(
                &cols,
                r#"{"column":"d","op":"relative","anchor":"last","n":30,"unit":"day"}"#
            ),
            "(d > subtractDays(toDate(now('Asia/Jakarta')), 30) AND d <= toDate(now('Asia/Jakarta')))"
        );
        assert_eq!(
            one_predicate(
                &cols,
                r#"{"column":"d","op":"relative","anchor":"last","n":3,"unit":"quarter"}"#
            ),
            "(d > subtractQuarters(toDate(now('Asia/Jakarta')), 3) AND d <= toDate(now('Asia/Jakarta')))"
        );
        assert_eq!(
            one_predicate(
                &cols,
                r#"{"column":"d","op":"relative","anchor":"this","unit":"month"}"#
            ),
            "(d >= toStartOfMonth(toDate(now('Asia/Jakarta'))) AND d < addMonths(toStartOfMonth(toDate(now('Asia/Jakarta'))), 1))"
        );
        assert_eq!(
            one_predicate(
                &cols,
                r#"{"column":"d","op":"relative","anchor":"this","unit":"week"}"#
            ),
            "(d >= toStartOfWeek(toDate(now('Asia/Jakarta')), 1) AND d < addWeeks(toStartOfWeek(toDate(now('Asia/Jakarta')), 1), 1))"
        );
        assert_eq!(
            one_predicate(
                &cols,
                r#"{"column":"d","op":"relative","anchor":"previous","unit":"year"}"#
            ),
            "(d >= subtractYears(toStartOfYear(toDate(now('Asia/Jakarta'))), 1) AND d < toStartOfYear(toDate(now('Asia/Jakarta'))))"
        );
        assert_eq!(
            one_predicate(
                &[("t", ColumnKind::DateTime)],
                r#"{"column":"t","op":"relative","anchor":"this","unit":"day"}"#
            ),
            "(toDate32(toTimeZone(t, 'Asia/Jakarta')) >= toDate(now('Asia/Jakarta')) AND toDate32(toTimeZone(t, 'Asia/Jakarta')) < addDays(toDate(now('Asia/Jakarta')), 1))"
        );
    }

    #[test]
    fn next_n_units_start_tomorrow_and_leave_today_out_for_every_unit() {
        let cols = [("d", ColumnKind::Date)];
        for (unit, add) in [
            ("day", "addDays"),
            ("week", "addWeeks"),
            ("month", "addMonths"),
            ("quarter", "addQuarters"),
            ("year", "addYears"),
        ] {
            let json = format!(
                r#"{{"column":"d","op":"relative","anchor":"next","n":7,"unit":"{unit}"}}"#
            );
            assert_eq!(
                one_predicate(&cols, &json),
                format!(
                    "(d > toDate(now('Asia/Jakarta')) AND d <= {add}(toDate(now('Asia/Jakarta')), 7))"
                )
            );
        }
        assert_eq!(
            one_predicate(
                &[("t", ColumnKind::DateTime)],
                r#"{"column":"t","op":"relative","anchor":"next","n":3,"unit":"day"}"#
            ),
            "(toDate32(toTimeZone(t, 'Asia/Jakarta')) > toDate(now('Asia/Jakarta')) AND toDate32(toTimeZone(t, 'Asia/Jakarta')) <= addDays(toDate(now('Asia/Jakarta')), 3))"
        );
        // `next` without `n` is not valid and never reaches SQL.
        let f = filter(r#"{"column":"d","op":"relative","anchor":"next","unit":"day"}"#);
        assert!(f.validate().is_err());
        let report = filter_predicates(&typed(&cols), &[], &[f]);
        assert!(report.predicates.is_empty(), "{:?}", report.predicates);
    }

    #[test]
    fn exclusive_ends_use_strict_comparisons_on_numbers_and_dates() {
        let num = [("price", ColumnKind::Number)];
        assert_eq!(
            one_predicate(
                &num,
                r#"{"column":"price","op":"between","min":"5","minExclusive":true}"#
            ),
            "price > 5"
        );
        assert_eq!(
            one_predicate(
                &num,
                r#"{"column":"price","op":"between","max":"5","maxExclusive":true}"#
            ),
            "price < 5"
        );
        assert_eq!(
            one_predicate(
                &num,
                r#"{"column":"price","op":"between","min":"1","max":"5","maxExclusive":true}"#
            ),
            "(price >= 1 AND price < 5)"
        );
        let json = r#"{"column":"d","op":"between","min":"2026-10-03","minExclusive":true}"#;
        assert_eq!(
            one_predicate(&[("d", ColumnKind::Date)], json),
            "d > toDate32('2026-10-03')"
        );
        assert_eq!(
            one_predicate(&[("d", ColumnKind::DateTime)], json),
            "toDate32(toTimeZone(d, 'Asia/Jakarta')) > toDate32('2026-10-03')"
        );
        assert_eq!(
            one_predicate(
                &[("d", ColumnKind::Date)],
                r#"{"column":"d","op":"between","max":"2026-10-03","maxExclusive":true}"#
            ),
            "d < toDate32('2026-10-03')"
        );
    }

    #[test]
    fn required_is_ignored_when_predicates_are_built() {
        let cols = [("price", ColumnKind::Number)];
        assert_eq!(
            one_predicate(
                &cols,
                r#"{"column":"price","op":"between","min":"5","required":true}"#
            ),
            one_predicate(&cols, r#"{"column":"price","op":"between","min":"5"}"#)
        );
    }

    #[test]
    fn not_contains_keeps_null_values_and_escapes_the_needle_like_contains() {
        let json = serde_json::json!({
            "column": "name", "op": "not_contains", "text": "50%_off\\o'Neil"
        })
        .to_string();
        assert_eq!(
            one_predicate(&[("name", ColumnKind::Text)], &json),
            "ifNull(positionCaseInsensitiveUTF8(toString(name), '50%_off\\\\o''Neil') = 0, 1)"
        );
        assert_eq!(
            skipped(
                &[("c", ColumnKind::Number)],
                r#"{"column":"c","op":"not_contains","text":"x"}"#
            )
            .len(),
            1
        );
    }

    #[test]
    fn a_relative_filter_on_a_number_or_text_column_is_skipped() {
        let json = r#"{"column":"c","op":"relative","anchor":"this","unit":"day"}"#;
        for kind in [ColumnKind::Number, ColumnKind::Text] {
            assert_eq!(
                skipped(&[("c", kind)], json),
                vec![SkippedFilter {
                    column: "c".to_owned(),
                    reason: SkipReason::WrongType
                }]
            );
        }
    }

    #[test]
    fn a_text_needle_with_quote_percent_underscore_and_backslash_is_one_literal() {
        let cols = [("name", ColumnKind::Text)];
        let json = serde_json::json!({
            "column": "name", "op": "contains", "text": "50%_off\\o'Neil"
        })
        .to_string();
        assert_eq!(
            one_predicate(&cols, &json),
            "positionCaseInsensitiveUTF8(toString(name), '50%_off\\\\o''Neil') > 0"
        );
    }

    #[test]
    fn starts_with_and_ends_with_lowercase_both_sides() {
        let cols = [("name", ColumnKind::Text)];
        assert_eq!(
            one_predicate(&cols, r#"{"column":"name","op":"starts_with","text":"Ba"}"#),
            "startsWith(lowerUTF8(toString(name)), lowerUTF8('Ba'))"
        );
        assert_eq!(
            one_predicate(&cols, r#"{"column":"name","op":"ends_with","text":"li"}"#),
            "endsWith(lowerUTF8(toString(name)), lowerUTF8('li'))"
        );
    }

    #[test]
    fn a_text_op_on_a_number_or_date_column_is_skipped() {
        for kind in [ColumnKind::Number, ColumnKind::Date, ColumnKind::DateTime] {
            assert_eq!(
                skipped(
                    &[("c", kind)],
                    r#"{"column":"c","op":"contains","text":"x"}"#
                ),
                vec![SkippedFilter {
                    column: "c".to_owned(),
                    reason: SkipReason::WrongType
                }]
            );
        }
    }

    #[test]
    fn a_filter_on_a_column_the_relation_lacks_is_reported_not_applied() {
        let out = filter_predicates(
            &typed(&[("a", ColumnKind::Text)]),
            &[],
            &[
                filter(r#"{"column":"gone","values":["x"]}"#),
                filter(r#"{"column":"a","values":["y"]}"#),
                filter(r#"{"column":"bad col","values":["x"]}"#),
            ],
        );
        assert_eq!(out.predicates, vec!["a IN ('y')".to_owned()]);
        assert_eq!(
            out.skipped,
            vec![
                SkippedFilter {
                    column: "gone".to_owned(),
                    reason: SkipReason::NoColumn
                },
                SkippedFilter {
                    column: "bad col".to_owned(),
                    reason: SkipReason::NoColumn
                },
            ]
        );
    }

    #[test]
    fn an_inactive_placeholder_filter_is_neither_applied_nor_reported() {
        let out = filter_predicates(
            &typed(&[("a", ColumnKind::Text)]),
            &[],
            &[filter(r#"{"column":"gone","values":[]}"#)],
        );
        assert_eq!(out, FilterOutcome::default());
    }

    #[test]
    fn the_report_carries_skipped_filters_for_a_mart_tile_and_a_source_tile() {
        let spec = stored_spec(ChartKind::Bar, "mart_wisman", "kawasan", &["jumlah"]);
        let cols: HashMap<String, RelationColumns> = HashMap::from([(
            "mart_wisman".to_owned(),
            typed(&[
                ("kawasan", ColumnKind::Text),
                ("jumlah", ColumnKind::Number),
            ]),
        )]);
        let filters = [
            filter(r#"{"column":"jumlah","op":"between","min":"10"}"#),
            filter(r#"{"column":"negara","values":["ID"]}"#),
        ];
        let got = sql_with_filters_report(&spec, &[], &filters, &cols);
        // `jumlah` is also the measure's alias, so the filter sits inside.
        assert!(
            got.sql.contains("WHERE jumlah >= 10) AS flt"),
            "{}",
            got.sql
        );
        assert_eq!(got.skipped.len(), 1);
        assert_eq!(got.skipped[0].column, "negara");

        let source_spec = source_spec(ChartKind::Bar, "material_group", &["materials"]);
        let got = sql_for_sql_source_report(
            &source_spec,
            SOURCE_SQL,
            &typed(&[
                ("material_group", ColumnKind::Text),
                ("materials", ColumnKind::Number),
            ]),
            &[],
            &[filter(r#"{"column":"materials","op":"between","max":"5"}"#)],
        )
        .unwrap();
        assert!(got.sql.contains("materials <= 5) AS flt"), "{}", got.sql);
        assert!(got.skipped.is_empty(), "{:?}", got.skipped);
    }

    // BI-18·A review BLOCKER 1: a predicate on a column the SELECT also
    // aliases must not see the alias.
    #[test]
    fn a_filter_on_an_aliased_measure_is_applied_inside_the_relation() {
        let spec = stored_spec(ChartKind::Bar, "mart_x", "kab", &["visitors"]);
        let cols: HashMap<String, RelationColumns> = HashMap::from([(
            "mart_x".to_owned(),
            typed(&[("kab", ColumnKind::Text), ("visitors", ColumnKind::Number)]),
        )]);
        let got = sql_with_filters(
            &spec,
            &[],
            &[filter(
                r#"{"column":"visitors","op":"between","min":"2000"}"#,
            )],
            &cols,
        );
        assert_eq!(
            got,
            "SELECT kab, round(sum(visitors)) AS visitors FROM \
             (SELECT * FROM serving.mart_x WHERE visitors >= 2000) AS flt \
             GROUP BY kab ORDER BY kab LIMIT 20"
        );
    }

    #[test]
    fn a_filter_on_another_column_keeps_the_plain_where() {
        let spec = stored_spec(ChartKind::Bar, "mart_x", "kab", &["visitors"]);
        let cols: HashMap<String, RelationColumns> = HashMap::from([(
            "mart_x".to_owned(),
            typed(&[("kab", ColumnKind::Text), ("visitors", ColumnKind::Number)]),
        )]);
        let got = sql_with_filters(
            &spec,
            &[],
            &[filter(r#"{"column":"kab","values":["a"]}"#)],
            &cols,
        );
        assert_eq!(
            got,
            "SELECT kab, round(sum(visitors)) AS visitors FROM serving.mart_x \
             WHERE kab IN ('a') GROUP BY kab ORDER BY kab LIMIT 20"
        );
    }

    #[test]
    fn the_collision_wrap_covers_count_points_kpi_and_a_sql_source() {
        let f = filter(r#"{"column":"visitors","op":"between","min":"5"}"#);
        let cols: HashMap<String, RelationColumns> = HashMap::from([(
            "mart_x".to_owned(),
            typed(&[
                ("kab", ColumnKind::Text),
                ("visitors", ColumnKind::Number),
                ("lat", ColumnKind::Number),
                ("lon", ColumnKind::Number),
            ]),
        )]);
        let inner = "(SELECT * FROM serving.mart_x WHERE visitors >= 5) AS flt";

        let mut count = stored_spec(ChartKind::Bar, "mart_x", "kab", &["visitors"]);
        count.def.aggregate = Some("count".to_owned());
        let sql = sql_with_filters(&count, &[], std::slice::from_ref(&f), &cols);
        assert!(
            sql.contains(&format!("count() AS visitors FROM {inner} GROUP BY")),
            "{sql}"
        );

        let mut points = stored_spec(ChartKind::Pointmap, "mart_x", "kab", &["visitors"]);
        points.def.lat = Some("lat".to_owned());
        points.def.lon = Some("lon".to_owned());
        let sql = sql_with_filters(&points, &[], std::slice::from_ref(&f), &cols);
        assert!(
            sql.contains(&format!("FROM {inner} WHERE lat IS NOT NULL")),
            "{sql}"
        );

        let kpi = stored_spec(ChartKind::Kpi, "mart_x", "", &["visitors"]);
        let sql = sql_with_filters(&kpi, &[], std::slice::from_ref(&f), &cols);
        // The KPI's only alias is `v`, so a filter on the measure stays plain.
        assert!(
            sql.contains("AS v FROM serving.mart_x WHERE visitors >= 5"),
            "{sql}"
        );
        // A filter on a column called `v` is the KPI's collision.
        let cols_v: HashMap<String, RelationColumns> = HashMap::from([(
            "mart_x".to_owned(),
            typed(&[("v", ColumnKind::Number), ("visitors", ColumnKind::Number)]),
        )]);
        let sql = sql_with_filters(
            &kpi,
            &[],
            &[filter(r#"{"column":"v","op":"between","min":"1"}"#)],
            &cols_v,
        );
        assert!(
            sql.contains("FROM (SELECT * FROM serving.mart_x WHERE v >= 1) AS flt"),
            "{sql}"
        );

        let source = source_spec(ChartKind::Bar, "material_group", &["materials"]);
        let got = sql_for_sql_source(
            &source,
            SOURCE_SQL,
            &typed(&[
                ("material_group", ColumnKind::Text),
                ("materials", ColumnKind::Number),
            ]),
            &[],
            &[filter(r#"{"column":"materials","op":"between","max":"5"}"#)],
        )
        .unwrap();
        assert!(got.contains("FROM (SELECT * FROM (\n"), "{got}");
        assert!(
            got.contains("WHERE materials <= 5) AS flt GROUP BY"),
            "{got}"
        );
        // The source cap stays on the statement.
        assert!(got.ends_with(SQL_SOURCE_SETTINGS), "{got}");
    }

    #[test]
    fn records_sql_on_a_mart_has_value_filters_order_and_page() {
        let cols = typed(&[("region", ColumnKind::Text), ("n", ColumnKind::Number)]);
        let region = Ident::new("region".to_owned()).unwrap();
        let got = records_sql(
            &Relation::Mart(Ident::new("mart_x".to_owned()).unwrap()),
            &cols,
            &[filter(r#"{"column":"n","op":"between","min":"3"}"#)],
            Some((&region, "o'brien\\")),
            50,
            100,
        );
        assert_eq!(
            got.rows,
            "SELECT * FROM serving.mart_x WHERE region = 'o''brien\\\\' AND n >= 3 \
             ORDER BY ALL LIMIT 50 OFFSET 100"
        );
        assert_eq!(
            got.count,
            "SELECT count() AS n FROM serving.mart_x WHERE region = 'o''brien\\\\' AND n >= 3"
        );
        assert!(got.skipped.is_empty(), "{:?}", got.skipped);
    }

    #[test]
    fn records_sql_for_a_whole_tile_has_no_where_when_nothing_is_active() {
        let cols = typed(&[("region", ColumnKind::Text)]);
        let got = records_sql(
            &Relation::Mart(Ident::new("mart_x".to_owned()).unwrap()),
            &cols,
            &[],
            None,
            50,
            0,
        );
        assert_eq!(
            got.rows,
            "SELECT * FROM serving.mart_x ORDER BY ALL LIMIT 50 OFFSET 0"
        );
        assert_eq!(got.count, "SELECT count() AS n FROM serving.mart_x");
    }

    #[test]
    fn records_sql_over_a_sql_source_caps_the_count_as_well_as_the_rows() {
        let cols = typed(&[("g", ColumnKind::Text)]);
        let got = records_sql(
            &Relation::Sql("SELECT g FROM serving.mart_x".to_owned()),
            &cols,
            &[],
            None,
            10,
            0,
        );
        assert!(got.rows.ends_with(SQL_SOURCE_SETTINGS), "{}", got.rows);
        assert!(got.count.ends_with(SQL_SOURCE_SETTINGS), "{}", got.count);
        assert!(got.count.contains(") AS src"), "{}", got.count);
    }

    #[test]
    fn records_sql_reports_a_filter_its_relation_cannot_honour() {
        let cols = typed(&[("g", ColumnKind::Text)]);
        let got = records_sql(
            &Relation::Mart(Ident::new("mart_x".to_owned()).unwrap()),
            &cols,
            &[filter(r#"{"column":"absent","values":["a"]}"#)],
            None,
            50,
            0,
        );
        assert_eq!(got.skipped.len(), 1);
        assert!(!got.rows.contains("WHERE"), "{}", got.rows);
    }

    // ── BI-9: time grain ────────────────────────────────────────────────

    fn grained_spec(kind: ChartKind, grain: &str, order: &str) -> StoredChartSpec {
        let mut spec = stored_spec(kind, "mart_x", "d", &["v"]);
        spec.def.grain = Some(grain.to_owned());
        spec.def.order = Some(order.to_owned());
        spec
    }

    fn dated(kind: ColumnKind) -> HashMap<String, RelationColumns> {
        let mut cols: RelationColumns = RelationColumns::default();
        cols.insert("d".to_owned(), kind);
        cols.insert("v".to_owned(), ColumnKind::Number);
        cols.insert("g".to_owned(), ColumnKind::Text);
        HashMap::from([("mart_x".to_owned(), cols)])
    }

    fn read_with(time: &TimeContext, grain: Option<Grain>) -> ReadContext<'_> {
        ReadContext {
            time,
            grain,
            fields: FieldCatalog::none(),
        }
    }

    #[test]
    fn a_grained_chart_buckets_the_dimension_in_an_inner_relation_and_keeps_the_latest() {
        let spec = grained_spec(ChartKind::Line, "month", "none");
        let got = sql_with_filters_report(&spec, &[], &[], &dated(ColumnKind::Date));
        assert_eq!(
            got.sql,
            "SELECT * FROM (SELECT d, round(sum(v)) AS v FROM \
             (SELECT date_trunc('month', d) AS d, v FROM serving.mart_x) AS bkt \
             GROUP BY d ORDER BY d DESC LIMIT 21) ORDER BY d"
        );
        assert_eq!(got.latest_limit, Some(20));
        assert_eq!(got.grain_skipped, None);
    }

    #[test]
    fn a_grained_chart_is_never_served_from_the_stored_sql() {
        let spec = grained_spec(ChartKind::Line, "month", "none");
        let got = sql_with_filters(&spec, &[], &[], &dated(ColumnKind::Date));
        assert_ne!(got, "SELECT 1");
        // The same chart with no grain still gets its stored SQL byte for byte.
        let mut plain = spec;
        plain.def.grain = None;
        assert_eq!(
            sql_with_filters(&plain, &[], &[], &dated(ColumnKind::Date)),
            "SELECT 1"
        );
    }

    #[test]
    fn every_grain_builds_on_a_date_and_on_a_timestamp_and_the_refused_ones_fall_back_to_the_raw_column()
     {
        for grain in crate::grain::ALL_GRAINS {
            for (kind, fits) in [
                (ColumnKind::Date, !grain.needs_time()),
                (ColumnKind::DateTime, true),
            ] {
                let spec = grained_spec(ChartKind::Bar, grain.as_str(), "none");
                let sql = sql_with_filters(&spec, &[], &[], &dated(kind));
                if fits {
                    assert!(sql.contains(") AS bkt "), "{grain:?} {kind:?}: {sql}");
                    assert!(sql.contains(" AS d"), "{grain:?} {kind:?}: {sql}");
                } else {
                    // A Date has no hour: the chart is rebuilt without the
                    // grain, never from the stored SQL that carries it.
                    assert!(!sql.contains("bkt"), "{grain:?} {kind:?}: {sql}");
                    assert!(sql.starts_with("SELECT d, "), "{grain:?} {kind:?}: {sql}");
                }
            }
        }
    }

    #[test]
    fn a_dashboard_filter_on_the_grained_column_reads_the_raw_column() {
        let spec = grained_spec(ChartKind::Line, "month", "none");
        let f = filter(r#"{"column":"d","op":"between","min":"2026-01-01","max":"2026-03-31"}"#);
        let sql = sql_with_filters(&spec, &[], &[f], &dated(ColumnKind::Date));
        assert_eq!(
            sql,
            "SELECT * FROM (SELECT d, round(sum(v)) AS v FROM \
             (SELECT date_trunc('month', d) AS d, v FROM \
             (SELECT * FROM serving.mart_x WHERE (d >= toDate32('2026-01-01') AND d <= toDate32('2026-03-31'))) AS flt) AS bkt \
             GROUP BY d ORDER BY d DESC LIMIT 21) ORDER BY d"
        );
    }

    #[test]
    fn a_filter_on_an_aggregated_column_still_filters_rows_before_the_aggregate() {
        let spec = grained_spec(ChartKind::Line, "week", "none");
        let f = filter(r#"{"column":"v","op":"between","min":"5"}"#);
        let sql = sql_with_filters(&spec, &[], &[f], &dated(ColumnKind::Date));
        assert!(
            sql.contains("(SELECT * FROM serving.mart_x WHERE v >= 5) AS flt"),
            "{sql}"
        );
        assert!(!sql.contains("HAVING"), "{sql}");
    }

    #[test]
    fn a_breakdown_keeps_the_latest_buckets_and_orders_by_bucket_then_series() {
        let mut spec = grained_spec(ChartKind::Line, "quarter", "none");
        spec.def.breakdown = Some("g".to_owned());
        spec.def.limit = Some(8.into());
        let got = sql_with_filters_report(&spec, &[], &[], &dated(ColumnKind::DateTime));
        assert_eq!(
            got.sql,
            "SELECT d, g, round(sum(v)) AS v FROM \
             (SELECT date_trunc('quarter', toTimeZone(d, 'Asia/Jakarta')) AS d, g, v FROM serving.mart_x) AS bkt \
             WHERE d IN (SELECT d FROM (SELECT date_trunc('quarter', toTimeZone(d, 'Asia/Jakarta')) AS d, g, v FROM serving.mart_x) AS bkt \
             GROUP BY d ORDER BY d DESC LIMIT 9) GROUP BY d, g ORDER BY d, g"
        );
        assert_eq!(got.latest_limit, Some(8));
    }

    #[test]
    fn a_value_order_is_the_editors_choice_and_keeps_the_top_buckets_not_the_latest() {
        let spec = grained_spec(ChartKind::Bar, "month", "desc");
        let got = sql_with_filters_report(&spec, &[], &[], &dated(ColumnKind::Date));
        assert_eq!(
            got.sql,
            "SELECT d, round(sum(v)) AS v FROM \
             (SELECT date_trunc('month', d) AS d, v FROM serving.mart_x) AS bkt \
             GROUP BY d ORDER BY v DESC LIMIT 20"
        );
        assert_eq!(got.latest_limit, None);
    }

    #[test]
    fn day_of_week_is_ordered_from_the_configured_first_day() {
        let spec = grained_spec(ChartKind::Bar, "day_of_week", "none");
        let cols = dated(ColumnKind::Date);
        let monday = TimeContext::new("Asia/Jakarta", WeekStart::Monday).unwrap();
        let sunday = TimeContext::new("Asia/Jakarta", WeekStart::Sunday).unwrap();
        let m = super::sql_with_filters(&spec, &[], &[], &cols, &read_with(&monday, None));
        let s = super::sql_with_filters(&spec, &[], &[], &cols, &read_with(&sunday, None));
        // BI-9 review fix (BLOCKER) R1: a part is never cut to its latest buckets.
        assert!(m.ends_with("GROUP BY d ORDER BY d LIMIT 64"), "{m}");
        assert!(s.ends_with("GROUP BY d ORDER BY d % 7 LIMIT 64"), "{s}");
        assert!(m.contains("toDayOfWeek(d) AS d") && s.contains("toDayOfWeek(d) AS d"));
    }

    #[test]
    fn the_dashboard_switch_replaces_a_truncation_only_and_reports_a_chart_that_cannot_take_it() {
        let cols = dated(ColumnKind::Date);
        let time = TimeContext::default();
        let line = grained_spec(ChartKind::Line, "day", "none");
        let got = super::sql_with_filters_report(
            &line,
            &[],
            &[],
            &cols,
            &read_with(&time, Some(Grain::Month)),
        );
        assert!(
            got.sql.contains("date_trunc('month', d) AS d"),
            "{}",
            got.sql
        );
        assert_eq!(got.grain_skipped, None);

        let part = grained_spec(ChartKind::Bar, "day_of_week", "none");
        let got = super::sql_with_filters_report(
            &part,
            &[],
            &[],
            &cols,
            &read_with(&time, Some(Grain::Month)),
        );
        assert!(
            got.sql.contains("toDayOfWeek(d) AS d"),
            "a part ignores the switch"
        );
        assert_eq!(got.grain_skipped, None);

        // `hour` on a plain date: the chart keeps its own grain, and says so.
        let got = super::sql_with_filters_report(
            &line,
            &[],
            &[],
            &cols,
            &read_with(&time, Some(Grain::Hour)),
        );
        assert_eq!(got.grain_skipped, Some(Grain::Hour));
        assert!(got.sql.contains("bkt"), "{}", got.sql);

        let plain = stored_spec(ChartKind::Bar, "mart_x", "g", &["v"]);
        let got = super::sql_with_filters_report(
            &plain,
            &[],
            &[],
            &cols,
            &read_with(&time, Some(Grain::Month)),
        );
        assert_eq!(got.sql, "SELECT 1", "a chart without a grain is untouched");
    }

    #[test]
    fn a_calendar_takes_day_only_and_up_to_a_thousand_buckets() {
        let mut spec = grained_spec(ChartKind::Calendar, "day", "none");
        spec.def.limit = Some(900.into());
        let sql = sql_with_filters(&spec, &[], &[], &dated(ColumnKind::DateTime));
        assert!(
            sql.contains("toDate32(toTimeZone(d, 'Asia/Jakarta')) AS d"),
            "{sql}"
        );
        assert!(sql.contains("LIMIT 901"), "{sql}");
    }

    #[test]
    fn a_sql_source_chart_is_grained_inside_the_capped_statement() {
        let mut spec = source_spec(ChartKind::Line, "d", &["v"]);
        spec.def.grain = Some("month".to_owned());
        spec.def.order = Some("none".to_owned());
        let mut cols: RelationColumns = RelationColumns::default();
        cols.insert("d".to_owned(), ColumnKind::Date);
        cols.insert("v".to_owned(), ColumnKind::Number);
        let got = sql_for_sql_source_report(&spec, SOURCE_SQL, &cols, &[], &[]).unwrap();
        assert!(
            got.sql
                .contains(&format!("FROM (\n{SOURCE_SQL}\n) AS src) AS bkt")),
            "{}",
            got.sql
        );
        assert!(got.sql.ends_with(SQL_SOURCE_SETTINGS), "{}", got.sql);
        assert_eq!(got.latest_limit, Some(20));
    }

    #[test]
    fn records_for_a_clicked_bucket_compare_the_bucket_in_the_report_zone() {
        let ts = Ident::new("ts").unwrap();
        let mut cols: RelationColumns = RelationColumns::default();
        cols.insert("ts".to_owned(), ColumnKind::DateTime);
        let from = Relation::Mart(Ident::new("mart_x").unwrap());
        let sunday = TimeContext::new("Europe/Berlin", WeekStart::Sunday).unwrap();
        let got = super::records_sql(
            &from,
            &cols,
            &[],
            Some(RecordsValue::Bucket {
                column: &ts,
                kind: ColumnKind::DateTime,
                grain: Grain::Week,
                text: "2026-03-01",
            }),
            50,
            0,
            &sunday,
        );
        assert_eq!(
            got.rows,
            "SELECT * FROM serving.mart_x WHERE toString(date_trunc('week', \
             toTimeZone(ts, 'Europe/Berlin') + toIntervalDay(1)) - toIntervalDay(1)) = '2026-03-01' \
             ORDER BY ALL LIMIT 50 OFFSET 0"
        );
        // A grain the column cannot take matches nothing, not everything.
        let got = super::records_sql(
            &from,
            &cols,
            &[],
            Some(RecordsValue::Bucket {
                column: &ts,
                kind: ColumnKind::Date,
                grain: Grain::Hour,
                text: "x",
            }),
            50,
            0,
            &sunday,
        );
        assert!(got.rows.contains("WHERE 0 "), "{}", got.rows);
    }

    #[test]
    fn relative_filters_follow_the_zone_and_the_first_day_of_the_week() {
        let sunday = TimeContext::new("Europe/Berlin", WeekStart::Sunday).unwrap();
        let f = filter(r#"{"column":"t","op":"relative","anchor":"this","unit":"week"}"#);
        let out =
            super::filter_predicates(&typed(&[("t", ColumnKind::DateTime)]), &[], &[f], &sunday);
        let day = "toDate32(toTimeZone(t, 'Europe/Berlin'))";
        let start = "toStartOfWeek(toDate(now('Europe/Berlin')), 0)";
        assert_eq!(
            out.predicates,
            [format!(
                "({day} >= {start} AND {day} < addWeeks({start}, 1))"
            )]
        );
    }

    /// BI-9 review fix (BLOCKER) R1: `hour_of_day` ordered by bucket returned
    /// hours 4 to 23 because the latest-N rule dropped 0 to 3.
    #[test]
    fn a_part_ordered_by_bucket_returns_its_whole_range_whatever_the_limit() {
        for limit in [1, 5, 20, 1000] {
            for grain in ["hour_of_day", "day_of_month", "week_of_year", "day_of_week"] {
                let mut spec = grained_spec(ChartKind::Bar, grain, "none");
                spec.def.limit = Some(limit.into());
                let got = sql_with_filters_report(&spec, &[], &[], &dated(ColumnKind::DateTime));
                assert_eq!(got.latest_limit, None, "{grain} {limit}");
                assert!(got.sql.contains("LIMIT 64"), "{grain} {limit}: {}", got.sql);
                assert!(
                    !got.sql.contains("DESC LIMIT"),
                    "{grain} {limit}: {}",
                    got.sql
                );
                assert!(!got.sql.starts_with("SELECT * FROM (SELECT"), "{}", got.sql);
            }
        }
    }

    #[test]
    fn a_part_with_a_value_order_keeps_the_editors_top_n_and_a_truncation_keeps_latest_n() {
        let mut spec = grained_spec(ChartKind::Bar, "hour_of_day", "desc");
        spec.def.limit = Some(5.into());
        let got = sql_with_filters_report(&spec, &[], &[], &dated(ColumnKind::DateTime));
        assert!(got.sql.contains("ORDER BY v DESC LIMIT 5"), "{}", got.sql);
        let mut month = grained_spec(ChartKind::Bar, "month", "none");
        month.def.limit = Some(5.into());
        let got = sql_with_filters_report(&month, &[], &[], &dated(ColumnKind::Date));
        assert_eq!(got.latest_limit, Some(5));
        assert!(got.sql.contains("LIMIT 6"), "{}", got.sql);
    }
    // ── BI-16 part A: raw tables, pivots, KPI comparisons ──────────────

    fn table_cols() -> HashMap<String, RelationColumns> {
        let mut cols: RelationColumns = RelationColumns::default();
        cols.insert("p".to_owned(), ColumnKind::Text);
        cols.insert("d".to_owned(), ColumnKind::Date);
        cols.insert("t".to_owned(), ColumnKind::DateTime);
        cols.insert("v".to_owned(), ColumnKind::Number);
        HashMap::from([("mart_x".to_owned(), cols)])
    }

    fn with_tables(kind: ChartKind, json: &str) -> StoredChartSpec {
        let mut spec = stored_spec(kind, "mart_x", "", &[]);
        spec.def.tables = serde_json::from_str(json).unwrap();
        spec
    }

    #[test]
    fn a_raw_table_is_rebuilt_with_its_filters_and_carries_the_count_of_the_same_rows() {
        let spec = with_tables(
            ChartKind::Table,
            r#"{"tableMode":"rows","columns":["p","v"],"sortColumn":"v","sortDir":"desc"}"#,
        );
        let f = filter(r#"{"column":"p","values":["a"]}"#);
        let got = sql_with_filters_report(&spec, &[], &[f], &table_cols());
        assert_eq!(
            got.sql,
            "SELECT p, v FROM serving.mart_x WHERE p IN ('a') ORDER BY v DESC, p, v LIMIT 50 OFFSET 0"
        );
        assert_eq!(
            got.table,
            Some(TableRead::Rows {
                count_sql: "SELECT count() AS n FROM serving.mart_x WHERE p IN ('a')".to_owned()
            })
        );
        // Without a filter it is still rebuilt, because the count is not stored.
        let got = sql_with_filters_report(&spec, &[], &[], &table_cols());
        assert!(matches!(got.table, Some(TableRead::Rows { .. })));
    }

    #[test]
    fn a_raw_table_over_a_sql_source_keeps_the_source_cap_on_the_page_and_the_count() {
        let spec = with_tables(ChartKind::Table, r#"{"tableMode":"rows","columns":["p"]}"#);
        let cols = text_cols(&["p"]);
        let got = sql_for_sql_source_report(&spec, "SELECT 'a' AS p", &cols, &[], &[]).unwrap();
        assert!(
            got.sql.contains(") AS src ORDER BY p LIMIT 50"),
            "{}",
            got.sql
        );
        assert!(got.sql.ends_with(SQL_SOURCE_SETTINGS), "{}", got.sql);
        let Some(TableRead::Rows { count_sql }) = got.table else {
            panic!("no count");
        };
        assert!(count_sql.ends_with(SQL_SOURCE_SETTINGS), "{count_sql}");
    }

    #[test]
    fn a_page_through_the_records_route_has_the_same_predicates_as_the_dashboards_page() {
        let plan = tables::plan_rows(
            &serde_json::from_str(r#"{"columns":["p","v"]}"#).unwrap(),
            &["p", "v"]
                .iter()
                .map(|c| (*c).to_owned())
                .collect::<std::collections::HashSet<String>>(),
        )
        .unwrap();
        let cols = text_cols(&["p", "v"]);
        let from = Relation::Mart(Ident::new("mart_x").unwrap());
        let f = filter(r#"{"column":"p","values":["a"]}"#);
        let page = rows_page_sql(&from, &cols, &[f], &plan, (50, 100), &DEFAULT_TIME);
        assert_eq!(
            page.rows,
            "SELECT p, v FROM serving.mart_x WHERE p IN ('a') ORDER BY p, v LIMIT 50 OFFSET 100"
        );
        assert_eq!(
            page.count,
            "SELECT count() AS n FROM serving.mart_x WHERE p IN ('a')"
        );
    }

    #[test]
    fn a_pivot_is_rebuilt_with_filters_and_reports_its_row_cap() {
        let spec = with_tables(
            ChartKind::Pivot,
            r#"{"rows":["p"],"values":[{"column":"v","aggregate":"sum"}],"totals":"grand"}"#,
        );
        let f = filter(r#"{"column":"p","values":["a"]}"#);
        let got = sql_with_filters_report(&spec, &[], &[f], &table_cols());
        assert!(
            got.sql
                .contains("FROM serving.mart_x WHERE p IN ('a') GROUP BY GROUPING SETS ((p), ())"),
            "{}",
            got.sql
        );
        assert_eq!(got.table, Some(TableRead::Pivot { row_cap: 10_000 }));
        // No filter: still the same statement, with the cap.
        let got = sql_with_filters_report(&spec, &[], &[], &table_cols());
        assert!(matches!(got.table, Some(TableRead::Pivot { .. })));
    }

    #[test]
    fn a_pivot_takes_the_dashboard_switch_on_its_first_date_field_and_says_when_it_cannot() {
        let mut spec = with_tables(
            ChartKind::Pivot,
            r#"{"rows":["p"],"columns":["d"],"values":[{"column":"v","aggregate":"sum"}]}"#,
        );
        spec.def.grain = Some("month".to_owned());
        let got = sql_with_filters_report(&spec, &[], &[], &table_cols());
        assert!(
            got.sql.contains("date_trunc('month', d) AS d"),
            "{}",
            got.sql
        );
        assert_eq!(got.grain, Some(Grain::Month));
        let time = TimeContext::default();
        let week = ReadContext {
            time: &time,
            grain: Some(Grain::Week),
            fields: FieldCatalog::none(),
        };
        let got = super::sql_with_filters_report(&spec, &[], &[], &table_cols(), &week);
        assert!(
            got.sql.contains("date_trunc('week', d) AS d"),
            "{}",
            got.sql
        );
        let hour = ReadContext {
            time: &time,
            grain: Some(Grain::Hour),
            fields: FieldCatalog::none(),
        };
        let got = super::sql_with_filters_report(&spec, &[], &[], &table_cols(), &hour);
        assert_eq!(got.grain_skipped, Some(Grain::Hour));
        assert!(
            got.sql.contains("date_trunc('month', d) AS d"),
            "{}",
            got.sql
        );
    }

    #[test]
    fn a_kpi_that_compares_is_rebuilt_at_read_time_and_a_filter_on_its_date_reads_the_raw_column() {
        let mut spec = with_tables(
            ChartKind::Kpi,
            r#"{"compare":{"kind":"previous","dateColumn":"d","period":"month"}}"#,
        );
        spec.def.measures = vec!["v".to_owned()].into();
        let f = filter(r#"{"column":"d","op":"between","min":"2026-01-01"}"#);
        let got = sql_with_filters_report(&spec, &[], &[f], &table_cols());
        assert!(
            got.sql.contains(
                "(SELECT date_trunc('month', d) AS d, v FROM (SELECT * FROM serving.mart_x WHERE d >= toDate32('2026-01-01')) AS flt) AS bkt"
            ),
            "{}",
            got.sql
        );
        assert_eq!(got.grain, Some(Grain::Month));
        assert_eq!(
            got.table,
            Some(TableRead::Trend {
                period: Grain::Month
            })
        );
        // A goal is the ordinary KPI statement.
        let mut goal = with_tables(ChartKind::Kpi, r#"{"compare":{"kind":"goal","value":5}}"#);
        goal.def.measures = vec!["v".to_owned()].into();
        let got = sql_with_filters_report(&goal, &[], &[], &table_cols());
        assert_eq!(got.sql, "SELECT 1");
        assert_eq!(got.table, None);
    }

    // ── BI-6: the chart builder's fields ────────────────────────────────

    fn bi6_cols() -> HashMap<String, RelationColumns> {
        HashMap::from([(
            "mart_x".to_owned(),
            typed(&[
                ("kab", ColumnKind::Text),
                ("cat", ColumnKind::Text),
                ("d", ColumnKind::Date),
                ("visitors", ColumnKind::Number),
                ("stay", ColumnKind::Number),
            ]),
        )])
    }

    /// A stored bar chart over `mart_x` by `kab`, with `extra` (a JSON object
    /// of `ChartInput` fields) laid over it.
    fn bi6_spec(extra: &str) -> StoredChartSpec {
        let mut spec = stored_spec(ChartKind::Bar, "mart_x", "kab", &["visitors"]);
        let mut def = serde_json::to_value(&spec.def).unwrap();
        let extra: serde_json::Value = serde_json::from_str(extra).unwrap();
        for (k, v) in extra.as_object().unwrap() {
            def[k] = v.clone();
        }
        spec.def = serde_json::from_value(def).unwrap();
        spec.spec.sql = "SELECT stored".to_owned();
        spec
    }

    fn bi6_sql(extra: &str, filters: &[FilterDef]) -> String {
        sql_with_filters_report(&bi6_spec(extra), &[], filters, &bi6_cols()).sql
    }

    #[test]
    fn charts_saved_before_bi_6_build_the_statement_they_always_did() {
        // Three definitions as they were stored before `BI-6` (no new field):
        // plain, with a breakdown, and with a dashboard filter. The statements
        // are the ones the existing tests pinned; `BI-6` changes none of them.
        let plain = stored_spec(ChartKind::Bar, "mart_x", "kab", &["visitors"]);
        let filtered = sql_with_filters_report(
            &plain,
            &[],
            &[filter(r#"{"column":"cat","values":["a"]}"#)],
            &HashMap::from([(
                "mart_x".to_owned(),
                typed(&[
                    ("kab", ColumnKind::Text),
                    ("cat", ColumnKind::Text),
                    ("visitors", ColumnKind::Number),
                ]),
            )]),
        );
        assert_eq!(
            filtered.sql,
            "SELECT kab, round(sum(visitors)) AS visitors FROM serving.mart_x \
             WHERE cat IN ('a') GROUP BY kab ORDER BY kab LIMIT 20"
        );
        let mut by_cat = stored_spec(ChartKind::Bar, "mart_x", "kab", &["visitors"]);
        by_cat.def.breakdown = Some("cat".to_owned());
        by_cat.def.order = Some("desc".to_owned());
        let got = sql_with_filters_report(
            &by_cat,
            &[],
            &[filter(r#"{"column":"cat","values":["a"]}"#)],
            &bi6_cols(),
        );
        assert_eq!(
            got.sql,
            "SELECT kab, cat, round(sum(visitors)) AS visitors FROM serving.mart_x \
             WHERE cat IN ('a') AND kab IN (SELECT kab FROM serving.mart_x WHERE cat IN ('a') \
             GROUP BY kab ORDER BY sum(visitors) DESC LIMIT 20) GROUP BY kab, cat ORDER BY kab, cat"
        );
        let mut counted = stored_spec(ChartKind::Bar, "mart_x", "kab", &["visitors"]);
        counted.def.aggregate = Some("count".to_owned());
        counted.def.order = Some("asc".to_owned());
        let got = sql_with_filters_report(
            &counted,
            &[],
            &[filter(r#"{"column":"cat","values":["a"]}"#)],
            &bi6_cols(),
        );
        assert_eq!(
            got.sql,
            "SELECT kab, count() AS visitors FROM serving.mart_x WHERE cat IN ('a') \
             GROUP BY kab ORDER BY visitors ASC LIMIT 20"
        );
        // And with no filter the stored statement is returned as it was.
        assert_eq!(
            sql_with_filters_report(&plain, &[], &[], &bi6_cols()).sql,
            plain.spec.sql
        );
    }

    #[test]
    fn two_aggregations_of_one_column_are_two_result_columns() {
        let sql = bi6_sql(
            r#"{"measures":["visitors",{"column":"stay","aggregate":"avg"},{"column":"stay","aggregate":"max"}]}"#,
            &[],
        );
        assert_eq!(
            sql,
            "SELECT kab, round(sum(visitors)) AS visitors, round(avg(stay)) AS avg_stay, \
             round(max(stay)) AS max_stay FROM serving.mart_x GROUP BY kab ORDER BY kab LIMIT 20"
        );
    }

    #[test]
    fn a_measure_beside_another_aggregation_of_its_column_is_renamed_outside_the_aggregate() {
        // `round(sum(visitors)) AS visitors` next to `avg(visitors)` makes the
        // engine read the second `visitors` as the first alias (error 184,
        // measured on ClickHouse 26.8), so the aggregates carry inner names.
        let sql = bi6_sql(
            r#"{"measures":["visitors",{"column":"visitors","aggregate":"avg"}],"sort":{"by":"avg_visitors","dir":"desc"},"limit":5}"#,
            &[],
        );
        assert_eq!(
            sql,
            "SELECT kab, __m0 AS visitors, __m1 AS avg_visitors FROM \
             (SELECT kab, round(sum(visitors)) AS __m0, round(avg(visitors)) AS __m1 \
             FROM serving.mart_x GROUP BY kab) ORDER BY avg_visitors DESC LIMIT 5"
        );
    }

    #[test]
    fn an_own_count_is_a_row_count_named_after_the_column() {
        let sql = bi6_sql(
            r#"{"measures":[{"column":"stay","aggregate":"count"}]}"#,
            &[],
        );
        assert!(sql.contains("count() AS count_stay"), "{sql}");
    }

    #[test]
    fn a_chart_filter_and_a_dashboard_filter_on_one_column_both_apply() {
        let own = r#"{"filters":[{"column":"kab","op":"not_in","values":["x"]}]}"#;
        let dash = [filter(r#"{"column":"kab","values":["a","b"]}"#)];
        // plain
        assert_eq!(
            bi6_sql(own, &dash),
            "SELECT kab, round(sum(visitors)) AS visitors FROM serving.mart_x \
             WHERE kab NOT IN ('x') AND kab IN ('a','b') GROUP BY kab ORDER BY kab LIMIT 20"
        );
        // with a breakdown: the same predicates in the kept-groups subquery and outside it
        let with_breakdown = bi6_sql(
            r#"{"breakdown":"cat","filters":[{"column":"kab","op":"not_in","values":["x"]}]}"#,
            &dash,
        );
        assert_eq!(
            with_breakdown
                .matches("kab NOT IN ('x') AND kab IN ('a','b')")
                .count(),
            2
        );
    }

    #[test]
    fn a_chart_filter_and_a_dashboard_filter_both_read_the_raw_column_under_a_grain() {
        let sql = bi6_sql(
            r#"{"dimension":"d","grain":"month","order":"none","filters":[{"column":"d","op":"between","min":"2026-01-01"}]}"#,
            &[filter(
                r#"{"column":"d","op":"between","max":"2026-06-30"}"#,
            )],
        );
        assert!(
            sql.contains(
                "FROM (SELECT * FROM serving.mart_x WHERE d >= toDate32('2026-01-01') AND d <= toDate32('2026-06-30')) AS flt"
            ),
            "{sql}"
        );
        assert!(sql.contains("date_trunc('month', d) AS d"), "{sql}");
    }

    #[test]
    fn a_chart_filter_that_can_no_longer_apply_keeps_the_stored_statement() {
        let mut spec = bi6_spec(r#"{"filters":[{"column":"gone","values":["a"]}]}"#);
        spec.spec.sql = "SELECT stored".to_owned();
        let got = sql_with_filters_report(
            &spec,
            &[],
            &[filter(r#"{"column":"kab","values":["a"]}"#)],
            &bi6_cols(),
        );
        // Never the statement without the chart's own filter.
        assert_eq!(got.sql, "SELECT stored");
        // Over a SQL source it is the tile's error.
        let source = sql_for_sql_source_report(
            &spec,
            "SELECT kab, visitors FROM serving.mart_x",
            &typed(&[("kab", ColumnKind::Text), ("visitors", ColumnKind::Number)]),
            &[],
            &[],
        );
        assert!(source.is_none());
    }

    #[test]
    fn a_filter_on_an_own_aggregate_name_is_applied_to_the_rows_not_the_aggregate() {
        let mut cols = bi6_cols();
        cols.get_mut("mart_x")
            .unwrap()
            .insert("avg_stay".to_owned(), ColumnKind::Number);
        let sql = sql_with_filters_report(
            &bi6_spec(r#"{"measures":[{"column":"stay","aggregate":"avg"}]}"#),
            &[],
            &[filter(r#"{"column":"avg_stay","op":"between","min":"3"}"#)],
            &cols,
        )
        .sql;
        assert_eq!(
            sql,
            "SELECT kab, round(avg(stay)) AS avg_stay FROM \
             (SELECT * FROM serving.mart_x WHERE avg_stay >= 3) AS flt \
             GROUP BY kab ORDER BY kab LIMIT 20"
        );
    }

    #[test]
    fn a_sort_orders_by_any_measure_or_the_dimension_and_limit_keeps_the_first_n() {
        let by_second = bi6_sql(
            r#"{"measures":["visitors",{"column":"stay","aggregate":"avg"}],"sort":{"by":"avg_stay","dir":"asc"},"limit":5}"#,
            &[],
        );
        assert!(
            by_second.ends_with("GROUP BY kab ORDER BY avg_stay ASC LIMIT 5"),
            "{by_second}"
        );
        let by_dimension = bi6_sql(r#"{"sort":{"by":"kab","dir":"desc"},"limit":5}"#, &[]);
        assert!(
            by_dimension.ends_with("GROUP BY kab ORDER BY kab DESC LIMIT 5"),
            "{by_dimension}"
        );
        // A sort that names nothing the chart returns builds nothing.
        let mut spec = bi6_spec(r#"{"sort":{"by":"nope","dir":"asc"}}"#);
        spec.spec.sql = "SELECT stored".to_owned();
        assert_eq!(
            sql_with_filters_report(&spec, &[], &[], &bi6_cols()).sql,
            "SELECT stored"
        );
    }

    #[test]
    fn a_grained_chart_sorted_by_its_dimension_descending_keeps_the_latest_buckets_newest_first() {
        let sql = bi6_sql(
            r#"{"dimension":"d","grain":"month","sort":{"by":"d","dir":"desc"},"limit":6}"#,
            &[],
        );
        assert_eq!(
            sql,
            "SELECT * FROM (SELECT d, round(sum(visitors)) AS visitors FROM \
             (SELECT date_trunc('month', d) AS d, visitors FROM serving.mart_x) AS bkt \
             GROUP BY d ORDER BY d DESC LIMIT 7) ORDER BY d DESC"
        );
    }

    #[test]
    fn a_breakdown_sorted_by_a_measure_keeps_and_shows_the_dimensions_in_that_order() {
        let sql = bi6_sql(
            r#"{"breakdown":"cat","sort":{"by":"visitors","dir":"desc"},"limit":5}"#,
            &[],
        );
        assert_eq!(
            sql,
            "SELECT kab, cat, __m0 AS visitors FROM (SELECT kab, cat, round(sum(visitors)) AS __m0, \
             sum(sum(visitors)) OVER (PARTITION BY kab) AS __t FROM serving.mart_x \
             WHERE kab IN (SELECT kab FROM serving.mart_x GROUP BY kab \
             ORDER BY sum(visitors) DESC, kab LIMIT 5) GROUP BY kab, cat) \
             ORDER BY __t DESC, kab, cat"
        );
    }

    #[test]
    fn other_is_aggregated_over_the_rows_outside_the_kept_groups_and_flagged() {
        let sql = bi6_sql(
            r#"{"sort":{"by":"visitors","dir":"desc"},"limit":5,"other":true}"#,
            &[filter(r#"{"column":"cat","values":["a"]}"#)],
        );
        let base = "(SELECT *, ifNull(toString(kab), '') AS __k FROM serving.mart_x WHERE cat IN ('a')) AS ob";
        assert_eq!(
            sql,
            format!(
                "SELECT __d AS kab, __m0 AS visitors, __o AS __other FROM \
                 (SELECT __d, __o, round(sum(visitors)) AS __m0, __m0 AS __t FROM \
                 (SELECT *, toUInt8(NOT (__k IN (SELECT __k FROM {base} GROUP BY __k \
                 ORDER BY sum(visitors) DESC, __k LIMIT 5))) AS __o, \
                 if(__o = 1, 'Other', __k) AS __d FROM {base}) GROUP BY __o, __d) \
                 ORDER BY __o, __t DESC, __d"
            )
        );
    }

    #[test]
    fn other_with_a_breakdown_is_computed_per_series_and_ranks_by_the_dimension_total() {
        let sql = bi6_sql(
            r#"{"breakdown":"cat","sort":{"by":"visitors","dir":"desc"},"limit":5,"other":true}"#,
            &[],
        );
        assert!(
            sql.contains("sum(sum(visitors)) OVER (PARTITION BY __d) AS __t"),
            "{sql}"
        );
        assert!(sql.contains("GROUP BY __o, __d, cat"), "{sql}");
        assert!(sql.ends_with("ORDER BY __o, __t DESC, __d, cat"), "{sql}");
    }

    #[test]
    fn other_over_an_average_averages_the_remaining_rows_not_the_hidden_averages() {
        let sql = bi6_sql(
            r#"{"measures":[{"column":"stay","aggregate":"avg"}],"sort":{"by":"avg_stay","dir":"desc"},"limit":3,"other":true}"#,
            &[],
        );
        // The group is chosen by the average and the Other group is one
        // `avg` over its rows, in the same `GROUP BY` as the kept groups.
        assert!(
            sql.contains("ORDER BY avg(stay) DESC, __k LIMIT 3"),
            "{sql}"
        );
        assert!(sql.contains("round(avg(stay)) AS __m0"), "{sql}");
        assert!(sql.contains("__m0 AS avg_stay"), "{sql}");
    }

    #[test]
    fn a_table_grouped_by_several_columns_lists_them_in_order_with_a_grain_on_a_date() {
        let table = |extra: &str| {
            let mut spec = stored_spec(ChartKind::Table, "mart_x", "", &["visitors"]);
            let mut def = serde_json::to_value(&spec.def).unwrap();
            let extra: serde_json::Value = serde_json::from_str(extra).unwrap();
            for (k, v) in extra.as_object().unwrap() {
                def[k] = v.clone();
            }
            spec.def = serde_json::from_value(def).unwrap();
            spec
        };
        let plain = table(r#"{"groupings":[{"column":"kab"},{"column":"cat"}]}"#);
        assert_eq!(
            sql_with_filters_report(&plain, &[], &[], &bi6_cols()).sql,
            "SELECT kab, cat, round(sum(visitors)) AS visitors FROM serving.mart_x \
             GROUP BY kab, cat ORDER BY kab, cat LIMIT 20"
        );
        let grained = table(
            r#"{"groupings":[{"column":"d","grain":"month"},{"column":"cat"}],"sort":{"by":"visitors","dir":"desc"}}"#,
        );
        assert_eq!(
            sql_with_filters_report(
                &grained,
                &[],
                &[filter(r#"{"column":"cat","values":["a"]}"#)],
                &bi6_cols()
            )
            .sql,
            "SELECT d, cat, round(sum(visitors)) AS visitors FROM \
             (SELECT date_trunc('month', d) AS d, cat, visitors FROM \
             (SELECT * FROM serving.mart_x WHERE cat IN ('a')) AS flt) AS bkt \
             GROUP BY d, cat ORDER BY visitors DESC, d, cat LIMIT 20"
        );
    }

    #[test]
    fn steps_are_nested_statements_and_the_sort_and_limit_belong_to_the_last_one() {
        let sql = bi6_sql(
            r#"{"measures":["visitors"],"limit":7,"sort":{"by":"visitors","dir":"desc"},
                "steps":[{"filter":[{"column":"visitors","op":"gt","value":"1000"}]}]}"#,
            &[filter(r#"{"column":"cat","values":["a"]}"#)],
        );
        assert_eq!(
            sql,
            "SELECT * FROM (SELECT * FROM (SELECT kab, round(sum(visitors)) AS visitors FROM \
             serving.mart_x WHERE cat IN ('a') GROUP BY kab) WHERE visitors > 1000) \
             ORDER BY visitors DESC, kab LIMIT 7 \
             SETTINGS max_rows_to_group_by = 10000, group_by_overflow_mode = 'throw'"
        );
        let summed = bi6_sql(
            r#"{"measures":["visitors"],"order":"none","limit":7,
                "steps":[{"summarise":{"by":["kab"],"measures":[{"column":"visitors","aggregate":"max"}]}}]}"#,
            &[],
        );
        assert!(
            summed.contains("SELECT kab, __s0 AS max_visitors FROM (SELECT kab, round(max(visitors)) AS __s0 FROM (SELECT kab, round(sum(visitors)) AS visitors FROM serving.mart_x GROUP BY kab) GROUP BY kab)"),
            "{summed}"
        );
        assert!(summed.contains("ORDER BY kab LIMIT 7"), "{summed}");
    }

    // ── BI-10: bins and grids ───────────────────────────────────────────

    #[test]
    fn a_binned_dimension_reads_the_range_and_the_filters_under_the_bin() {
        let got = sql_with_filters_report(
            &bi6_spec(r#"{"dimension":"stay","bin":{"mode":"width","width":500}}"#),
            &[],
            &[filter(r#"{"column":"stay","op":"between","min":"10"}"#)],
            &bi6_cols(),
        );
        // The bin carries the dimension's name; the filter on the same column
        // is applied on the raw rows beneath it, and the range is read over
        // those same rows.
        assert!(
            got.sql.starts_with(
                "SELECT stay, round(sum(visitors)) AS visitors FROM (SELECT round(least(floor(toFloat64(stay) / __bw_stay)"
            ),
            "{}",
            got.sql
        );
        assert!(
            got.sql.contains("FROM (SELECT * FROM (SELECT * FROM serving.mart_x WHERE stay >= 10) AS flt CROSS JOIN (SELECT w AS __bw_stay, mn AS __bmn_stay, mx AS __bmx_stay FROM (SELECT mn, mx, 500 AS w FROM (SELECT min(toFloat64(stay)) AS mn, max(toFloat64(stay)) AS mx FROM (SELECT * FROM serving.mart_x WHERE stay >= 10) AS flt))) AS rng) AS ranged) AS bkt GROUP BY stay ORDER BY stay"),
            "{}",
            got.sql
        );
        assert_eq!(got.bins.len(), 1);
        assert_eq!(got.bins[0].column, "stay");
        assert_eq!(got.bins[0].mode, "width");
        assert!(
            got.bins[0]
                .sql
                .starts_with("SELECT w AS width, mn AS min, mx AS max FROM")
        );
        // The width statement reads the same filtered rows as the bins.
        assert!(
            got.bins[0]
                .sql
                .contains("(SELECT * FROM serving.mart_x WHERE stay >= 10) AS flt")
        );
    }

    #[test]
    fn a_chart_without_a_bin_carries_none_and_builds_what_it_always_did() {
        let got = sql_with_filters_report(&bi6_spec("{}"), &[], &[], &bi6_cols());
        assert_eq!(got.sql, "SELECT stored");
        assert!(got.bins.is_empty(), "{:?}", got.bins);
        let filtered = sql_with_filters_report(
            &bi6_spec("{}"),
            &[],
            &[filter(r#"{"column":"cat","values":["a"]}"#)],
            &bi6_cols(),
        );
        assert!(!filtered.sql.contains("rng"), "{}", filtered.sql);
        assert!(filtered.bins.is_empty(), "{:?}", filtered.bins);
    }

    #[test]
    fn a_binned_breakdown_and_a_binned_dimension_nest_one_bin_each() {
        let got = sql_with_filters_report(
            &bi6_spec(
                r#"{"dimension":"stay","bin":{"mode":"auto"},"breakdown":"visitors","breakdownBin":{"mode":"count","count":10}}"#,
            ),
            &[],
            &[],
            &bi6_cols(),
        );
        assert!(
            got.sql.contains("__bw_stay") && got.sql.contains("__bw_visitors"),
            "{}",
            got.sql
        );
        assert_eq!(got.bins.len(), 2);
        // Both groupings are read by name from the nested bins.
        assert!(
            got.sql.contains("GROUP BY stay, visitors") || got.sql.contains("GROUP BY stay"),
            "{}",
            got.sql
        );
    }

    #[test]
    fn a_grouped_table_bins_one_of_its_groupings() {
        let mut spec = bi6_spec(
            r#"{"kind":"table","dimension":"","groupings":[{"column":"stay","bin":{"mode":"width","width":5}},{"column":"cat"}]}"#,
        );
        spec.spec.kind = ChartKind::Table;
        let got = sql_with_filters_report(&spec, &[], &[], &bi6_cols());
        assert!(got.sql.contains("__bw_stay"), "{}", got.sql);
        assert!(got.sql.contains("GROUP BY stay, cat"), "{}", got.sql);
        assert_eq!(got.bins.len(), 1);
    }

    #[test]
    fn a_grid_map_groups_the_points_by_cell_under_the_filters() {
        let mut spec = stored_spec(ChartKind::Pointmap, "mart_x", "", &["visitors"]);
        spec.def.lat = Some("stay".to_owned());
        spec.def.lon = Some("visitors".to_owned());
        spec.def.shape.grid = Some(serde_json::from_str(r#"{"size":1}"#).unwrap());
        let got = sql_with_filters_report(
            &spec,
            &[],
            &[filter(r#"{"column":"cat","values":["a"]}"#)],
            &bi6_cols(),
        );
        assert!(
            got.sql.contains("FROM (SELECT * FROM serving.mart_x WHERE cat IN ('a') AND stay IS NOT NULL AND visitors IS NOT NULL) AS flt) GROUP BY __cy, __cx ORDER BY visitors DESC, __cy, __cx LIMIT 5000"),
            "{}",
            got.sql
        );
        assert!(got.sql.contains("count() AS cell_points"));
    }

    #[test]
    fn a_clicked_bin_lists_the_rows_in_its_range_and_closes_the_last_bin_at_the_maximum() {
        let time = TimeContext::default();
        let cols = typed(&[("v", ColumnKind::Number), ("g", ColumnKind::Text)]);
        let v = Ident::new("v").unwrap();
        let got = super::records_sql(
            &Relation::Mart(Ident::new("t").unwrap()),
            &cols,
            &[filter(r#"{"column":"g","values":["a"]}"#)],
            Some(RecordsValue::Bin {
                column: &v,
                lower: 500.0,
                width: 500.0,
            }),
            50,
            0,
            &time,
        );
        assert_eq!(
            got.rows,
            "SELECT * FROM serving.t WHERE ((toFloat64(v) >= 500 AND toFloat64(v) < 1000) OR (toFloat64(v) = 1000 AND toFloat64(v) = (SELECT max(toFloat64(v)) FROM serving.t WHERE g IN ('a')))) AND g IN ('a') ORDER BY ALL LIMIT 50 OFFSET 0"
        );
        assert_eq!(got.count.matches("SELECT max").count(), 1);
    }

    #[test]
    fn export_settings_replace_the_tile_cap_and_keep_what_follows_it() {
        let sql =
            format!("SELECT 1 FROM (SELECT 1\n) AS src{SQL_SOURCE_SETTINGS}, join_use_nulls = 1");
        let got = with_export_settings(&sql, 1_000_001);
        assert_eq!(
            got,
            "SELECT 1 FROM (SELECT 1\n) AS src SETTINGS max_result_rows = 1000001, \
             result_overflow_mode = 'break', max_execution_time = 280, join_use_nulls = 1"
        );
        assert!(!got.contains("= 2000"));
        // A mart statement has no cap to lift.
        assert_eq!(with_export_settings("SELECT 1", 5), "SELECT 1");
    }
}
