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

use crate::filters::{
    ColumnKind, FilterDef, FilterOp, RelativeAnchor, RelativeUnit, parse_iso_date, parse_number,
};
use crate::grain::{self, Grain, TimeContext, WeekStart};
use crate::specs::Aggregate;
use crate::store::{ChartInput, StoredChartSpec};

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

/// Where a chart's rows come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Relation {
    /// A Gold mart, unqualified (`mart_x`); rendered as `serving.mart_x`.
    Mart(Ident),
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
}

impl Relation {
    /// The `FROM` target: `serving.<mart>`, or the source text as a derived
    /// table. Public so the filter value list reads a relation exactly the
    /// way a tile does.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Mart(mart) => format!("serving.{mart}"),
            Self::Sql(sql) => format!("(\n{}\n) AS src", sql.trim()),
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
        }
    }

    /// Trailing `SETTINGS` for this relation (empty for a mart, whose
    /// SQL is assembled entirely from validated identifiers).
    fn settings(&self) -> &'static str {
        match self {
            Self::Mart(_) => "",
            Self::Sql(_) => SQL_SOURCE_SETTINGS,
            Self::Filtered { base, .. } | Self::Bucketed { base, .. } => base.settings(),
        }
    }
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
    measures: Vec<Ident>,
    agg: Aggregate,
    order: String,
    limit: u32,
    breakdown: Option<Ident>,
    where_clauses: Vec<String>,
    grain: Option<(Grain, WeekStart)>,
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

    /// Supply the measure columns, completing the projection and unlocking
    /// [`QueryBuilder::filter_in`]/[`QueryBuilder::build`].
    #[must_use]
    pub fn measures(self, measures: Vec<Ident>) -> QueryBuilder<Ready> {
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

    /// Render the final SQL. Ports `buildSql` in `bi-store.ts`.
    #[must_use]
    pub fn build(&self) -> String {
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
            .is_some_and(|(g, _)| grain::cuts_to_latest(g, &self.order));
        let bucket_limit = self.limit + u32::from(latest);
        let key = self.grain.map_or_else(
            || dimension.clone(),
            |(g, week_start)| g.order_key(&dimension, week_start),
        );

        if let Some(breakdown) = &self.breakdown {
            let Some(measure) = self.measures.first() else {
                return String::new();
            };
            let agg_expr = self.agg_of(measure);
            let inner = if self.where_clauses.is_empty() {
                String::new()
            } else {
                format!("WHERE {} ", self.where_clauses.join(" AND "))
            };
            let outer = if self.where_clauses.is_empty() {
                "WHERE".to_owned()
            } else {
                format!("WHERE {} AND", self.where_clauses.join(" AND "))
            };
            // Which buckets are kept: the biggest by value, or (grained and
            // ordered by bucket) the latest.
            let keep_by = if latest {
                format!("{key} DESC")
            } else if self.agg == Aggregate::Count {
                "count() DESC".to_owned()
            } else {
                format!("{}({measure}) DESC", self.agg)
            };
            return format!(
                "SELECT {dimension}, {breakdown}, {agg_expr} FROM {from} \
                 {outer} {dimension} IN (SELECT {dimension} FROM {from} {inner}\
                 GROUP BY {dimension} ORDER BY {keep_by} LIMIT {bucket_limit}) \
                 GROUP BY {dimension}, {breakdown} ORDER BY {key}, {breakdown}{settings}",
            );
        }

        let sel = self
            .measures
            .iter()
            .map(|m| self.agg_of(m))
            .collect::<Vec<_>>()
            .join(", ");
        let order_clause = if self.order == "none" {
            key.clone()
        } else {
            let Some(first) = self.measures.first() else {
                return String::new();
            };
            format!(
                "{first} {}",
                if self.order == "asc" { "ASC" } else { "DESC" }
            )
        };
        if latest {
            return format!(
                "SELECT * FROM (SELECT {dimension}, {sel} FROM {from} {where_sql}GROUP BY {dimension} \
                 ORDER BY {key} DESC LIMIT {bucket_limit}) ORDER BY {key}{settings}",
            );
        }
        format!(
            "SELECT {dimension}, {sel} FROM {from} {where_sql}GROUP BY {dimension} ORDER BY {order_clause} LIMIT {limit}{settings}",
            limit = self.limit,
        )
    }

    fn agg_of(&self, measure: &Ident) -> String {
        if self.agg == Aggregate::Count {
            format!("count() AS {measure}")
        } else {
            format!("round({}({measure})) AS {measure}", self.agg)
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
    let val = if agg == Aggregate::Count {
        "count()".to_owned()
    } else {
        format!("round({agg}({measure}))")
    };
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
        Relation::Mart(_) => POINT_LIMIT,
        Relation::Sql(_) => POINT_LIMIT.min(SQL_SOURCE_MAX_ROWS),
        Relation::Filtered { base, .. } | Relation::Bucketed { base, .. } => point_limit(base),
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
}

impl<'a> ReadContext<'a> {
    /// No dashboard switch: every chart keeps its own grain.
    #[must_use]
    pub fn new(time: &'a TimeContext) -> Self {
        Self { time, grain: None }
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
    let column = cols.get(&def.dimension).copied();
    let r = grain::resolve(def.grain.as_deref(), spec.spec.kind, column, read.grain);
    let g = r.effective.zip(column).map(|(grain, column)| {
        let order = def.order.as_deref().unwrap_or("none");
        let limit =
            grain::effective_limit(grain, order, grain::clamp_limit(spec.spec.kind, def.limit));
        Grained {
            grain,
            column,
            latest_limit: grain::cuts_to_latest(grain, order).then_some(limit),
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
    };
    if spec.spec.kind == crate::specs::ChartKind::Text {
        return FilteredSql {
            sql: String::new(),
            skipped: Vec::new(),
            grain_skipped: None,
            grain: None,
            grain_column: None,
            latest_limit: None,
        };
    }
    let def: &ChartInput = &spec.def;
    if def.mart.is_empty() {
        return unfiltered(Vec::new());
    }
    let empty_cols: RelationColumns<HCols> = RelationColumns::default();
    let cols = mart_cols.get(&def.mart).unwrap_or(&empty_cols);
    let FilterOutcome {
        predicates,
        skipped,
        columns,
    } = filter_predicates(cols, years, filters, read.time);
    if predicates.is_empty() && def.grain.is_none() {
        return unfiltered(skipped);
    }
    // `mart` was already validated as a well-formed identifier when the
    // spec was created via `specFromInput`, so re-validating here would
    // only ever fail on data corruption; fall back to the stored SQL rather
    // than panicking, matching "SQL never comes raw from untrusted input"
    // without introducing a new failure mode.
    let Ok(mart) = Ident::new(def.mart.clone()) else {
        return unfiltered(skipped);
    };
    let (g, grain_skipped) = grained(spec, cols, read);
    let latest_limit = g.as_ref().and_then(|g| g.latest_limit);
    let sql = rebuild(
        spec,
        &Relation::Mart(mart),
        predicates,
        &columns,
        read,
        g.as_ref(),
    )
    .unwrap_or_else(|| spec.spec.sql.clone());
    FilteredSql {
        sql,
        skipped,
        grain_skipped,
        grain: g.as_ref().map(|g| g.grain),
        grain_column: g.as_ref().map(|g| g.column),
        latest_limit,
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
    if spec.spec.kind == crate::specs::ChartKind::Text {
        return Some(FilteredSql {
            sql: String::new(),
            skipped: Vec::new(),
            grain_skipped: None,
            grain: None,
            grain_column: None,
            latest_limit: None,
        });
    }
    let FilterOutcome {
        predicates,
        skipped,
        columns,
    } = filter_predicates(source_cols, years, filters, read.time);
    let (g, grain_skipped) = grained(spec, source_cols, read);
    let latest_limit = g.as_ref().and_then(|g| g.latest_limit);
    let sql = rebuild(
        spec,
        &Relation::Sql(source_sql.to_owned()),
        predicates,
        &columns,
        read,
        g.as_ref(),
    )?;
    Some(FilteredSql {
        sql,
        skipped,
        grain_skipped,
        grain: g.as_ref().map(|g| g.grain),
        grain_column: g.as_ref().map(|g| g.column),
        latest_limit,
    })
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
    match value {
        Some(RecordsValue::Equals(column, value)) => {
            predicates.insert(0, format!("{column} = {}", SqlLiteral::from(value)));
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
    let from_sql = from.render();
    let settings = from.settings();
    RecordsSql {
        rows: format!(
            "SELECT * FROM {from_sql}{where_sql} ORDER BY ALL LIMIT {limit} OFFSET {offset}{settings}"
        ),
        count: format!("SELECT count() AS n FROM {from_sql}{where_sql}{settings}"),
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

/// Rebuild a stored chart's SQL over `from` with `where_clauses`, from its
/// structured definition. `None` if the definition's identifiers no longer
/// validate (only possible through data corruption).
fn rebuild(
    spec: &StoredChartSpec,
    from: &Relation,
    where_clauses: Vec<String>,
    where_columns: &[String],
    read: &ReadContext<'_>,
    grained: Option<&Grained>,
) -> Option<String> {
    let def: &ChartInput = &spec.def;
    if let Some(g) = grained {
        let chart = GrainedChart::from_def(def, spec.spec.kind, g.grain)?;
        return grained_sql(from, where_clauses, &chart, g.column, read.time);
    }
    // BI-18·A review BLOCKER 1: a predicate on a column that the SELECT also
    // names as an alias would resolve to the alias (an aggregate) in WHERE.
    // Only then are the predicates moved into an inner relation; otherwise
    // the SQL is exactly what it was before.
    if !where_clauses.is_empty() {
        let aliases: Vec<&str> = match spec.spec.kind {
            crate::specs::ChartKind::Kpi | crate::specs::ChartKind::Gauge => vec!["v"],
            crate::specs::ChartKind::Boxplot => def
                .measures
                .first()
                .map(String::as_str)
                .into_iter()
                .chain(["__n"])
                .collect(),
            crate::specs::ChartKind::Pointmap | crate::specs::ChartKind::Geoheat => def
                .measures
                .first()
                .map(String::as_str)
                .into_iter()
                .collect(),
            _ => def.measures.iter().map(String::as_str).collect(),
        };
        if where_columns.iter().any(|c| aliases.contains(&c.as_str())) {
            let wrapped = Relation::Filtered {
                base: Box::new(from.clone()),
                predicates: where_clauses,
            };
            return rebuild(spec, &wrapped, Vec::new(), &[], read, None);
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

    if matches!(
        spec.spec.kind,
        crate::specs::ChartKind::Pointmap | crate::specs::ChartKind::Geoheat
    ) {
        // The dimension is only an optional label here, so it must not go
        // through the `Ident::new(&def.dimension)` below (empty is valid).
        let lat = Ident::new(def.lat.clone()?).ok()?;
        let lon = Ident::new(def.lon.clone()?).ok()?;
        let measure = Ident::new(def.measures.first()?.clone()).ok()?;
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

    let dimension = Ident::new(def.dimension.clone()).ok()?;
    if spec.spec.kind == crate::specs::ChartKind::Boxplot {
        let measure = Ident::new(def.measures.first()?.clone()).ok()?;
        return Some(build_boxplot_sql(
            from,
            &dimension,
            &measure,
            &where_clauses,
            def.limit.unwrap_or(20),
        ));
    }
    let mut measures = Vec::with_capacity(def.measures.len());
    for m in &def.measures {
        measures.push(Ident::new(m.clone()).ok()?);
    }
    let breakdown = def
        .breakdown
        .as_ref()
        .and_then(|b| Ident::new(b.clone()).ok());

    let mut builder = QueryBuilder::over(from.clone())
        .dimension(dimension)
        .aggregate(agg)
        .order(def.order.clone().unwrap_or_else(|| "none".to_owned()))
        .limit(def.limit.unwrap_or(20))
        .breakdown(breakdown)
        .measures(measures);
    for clause in where_clauses {
        // The predicates were already assembled as complete SQL fragments
        // (through `SqlLiteral`) by `filter_predicates`; pass them through
        // as-is rather than re-deriving column/values.
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
    /// The measure columns.
    pub measures: Vec<Ident>,
    /// The aggregate.
    pub agg: Aggregate,
    /// `"none"` (by bucket), `"asc"` or `"desc"` (by value).
    pub order: String,
    /// The bucket limit.
    pub limit: u32,
}

impl GrainedChart {
    /// Read the identifiers out of a stored definition. `None` when one no
    /// longer validates (data corruption).
    #[must_use]
    pub fn from_def(def: &ChartInput, kind: crate::specs::ChartKind, grain: Grain) -> Option<Self> {
        let mut measures = Vec::with_capacity(def.measures.len());
        for m in &def.measures {
            measures.push(Ident::new(m.clone()).ok()?);
        }
        let order = def.order.clone().unwrap_or_else(|| "none".to_owned());
        let limit = grain::effective_limit(grain, &order, grain::clamp_limit(kind, def.limit));
        Some(Self {
            grain,
            dimension: Ident::new(def.dimension.clone()).ok()?,
            breakdown: def
                .breakdown
                .as_ref()
                .and_then(|b| Ident::new(b.clone()).ok()),
            measures,
            agg: Aggregate::from_str_lossy(def.aggregate.as_deref().unwrap_or("sum")),
            order,
            limit,
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
    for name in chart
        .breakdown
        .iter()
        .chain(chart.measures.iter())
        .map(ToString::to_string)
    {
        if name != chart.dimension.as_str() && !carried.contains(&name) {
            carried.push(name);
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
            .measures(chart.measures.clone())
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
        assert!(
            filter_predicates(&typed(&cols), &[], &[f])
                .predicates
                .is_empty()
        );
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
        assert!(got.skipped.is_empty());
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
        assert!(got.skipped.is_empty());
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
        ReadContext { time, grain }
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
        spec.def.limit = Some(8);
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
        spec.def.limit = Some(900);
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
                spec.def.limit = Some(limit);
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
        spec.def.limit = Some(5);
        let got = sql_with_filters_report(&spec, &[], &[], &dated(ColumnKind::DateTime));
        assert!(got.sql.contains("ORDER BY v DESC LIMIT 5"), "{}", got.sql);
        let mut month = grained_spec(ChartKind::Bar, "month", "none");
        month.def.limit = Some(5);
        let got = sql_with_filters_report(&month, &[], &[], &dated(ColumnKind::Date));
        assert_eq!(got.latest_limit, Some(5));
        assert!(got.sql.contains("LIMIT 6"), "{}", got.sql);
    }
}
