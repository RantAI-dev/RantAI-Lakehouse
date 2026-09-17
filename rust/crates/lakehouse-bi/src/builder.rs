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

use std::collections::HashSet;
use std::marker::PhantomData;

use lakehouse_core::ident::{Ident, SqlLiteral};

use crate::specs::Aggregate;
use crate::store::{ChartInput, FilterDef, StoredChartSpec};

/// Typestate marker: a [`QueryBuilder`] that still needs its projection
/// (measures) before it can accept filters or be built.
pub struct NeedsProjection;

/// Typestate marker: a [`QueryBuilder`] with a projection set, ready to
/// accept filters and be built.
pub struct Ready;

/// Builder for the general (non-KPI) chart SQL: `SELECT dimension, agg(measure)
/// ... FROM serving.<mart> [WHERE ...] GROUP BY ... ORDER BY ... LIMIT ...`.
///
/// Ports the `buildSql` free function in `bi-store.ts` as a typestate
/// builder: [`QueryBuilder::measures`] must be called before
/// [`QueryBuilder::filter_in`] or [`QueryBuilder::build`] are available,
/// enforced at compile time rather than by convention.
pub struct QueryBuilder<S> {
    mart: Ident,
    dimension: Option<Ident>,
    measures: Vec<Ident>,
    agg: Aggregate,
    order: String,
    limit: u32,
    breakdown: Option<Ident>,
    where_clauses: Vec<String>,
    _state: PhantomData<S>,
}

impl QueryBuilder<NeedsProjection> {
    /// Start building a query against `mart` (unqualified, e.g.
    /// `mart_wisman` — the `serving.` prefix is added by [`Self::build`]).
    #[must_use]
    pub fn new(mart: Ident) -> Self {
        Self {
            mart,
            dimension: None,
            measures: Vec::new(),
            agg: Aggregate::Sum,
            order: "none".to_owned(),
            limit: 20,
            breakdown: None,
            where_clauses: Vec::new(),
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

    /// Supply the measure columns, completing the projection and unlocking
    /// [`QueryBuilder::filter_in`]/[`QueryBuilder::build`].
    #[must_use]
    pub fn measures(self, measures: Vec<Ident>) -> QueryBuilder<Ready> {
        QueryBuilder {
            mart: self.mart,
            dimension: self.dimension,
            measures,
            agg: self.agg,
            order: self.order,
            limit: self.limit,
            breakdown: self.breakdown,
            where_clauses: self.where_clauses,
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
        let mart = &self.mart;
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
            let order_expr = if self.agg == Aggregate::Count {
                "count()".to_owned()
            } else {
                format!("{}({measure})", self.agg)
            };
            return format!(
                "SELECT {dimension}, {breakdown}, {agg_expr} FROM serving.{mart} \
                 {outer} {dimension} IN (SELECT {dimension} FROM serving.{mart} {inner}\
                 GROUP BY {dimension} ORDER BY {order_expr} DESC LIMIT {limit}) \
                 GROUP BY {dimension}, {breakdown} ORDER BY {dimension}, {breakdown}",
                limit = self.limit,
            );
        }

        let sel = self
            .measures
            .iter()
            .map(|m| self.agg_of(m))
            .collect::<Vec<_>>()
            .join(", ");
        let order_clause = if self.order == "none" {
            dimension.clone()
        } else {
            let Some(first) = self.measures.first() else {
                return String::new();
            };
            format!(
                "{first} {}",
                if self.order == "asc" { "ASC" } else { "DESC" }
            )
        };
        format!(
            "SELECT {dimension}, {sel} FROM serving.{mart} {where_sql}GROUP BY {dimension} ORDER BY {order_clause} LIMIT {limit}",
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
    mart: &Ident,
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
    format!("SELECT {val} AS v FROM serving.{mart}{where_sql}")
}

/// SQL for a stored spec with runtime filters applied (year filter + the
/// dashboard's dimension filters). `mart_cols` maps mart name to its column
/// set, so we know which filters actually apply. Ports `sqlWithFilters` in
/// `bi-store.ts`.
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
///   SQL byte-for-byte when no filter applies).
#[must_use]
pub fn sql_with_filters<HMap, HSet>(
    spec: &StoredChartSpec,
    years: &[i64],
    filters: &[FilterDef],
    mart_cols: &std::collections::HashMap<String, HashSet<String, HSet>, HMap>,
) -> String
where
    HMap: std::hash::BuildHasher,
    HSet: std::hash::BuildHasher + Default,
{
    if spec.spec.kind == crate::specs::ChartKind::Text {
        return String::new();
    }
    let def: &ChartInput = &spec.def;
    if def.mart.is_empty() {
        return spec.spec.sql.clone();
    }
    let empty_cols: HashSet<String, HSet> = HashSet::default();
    let cols = mart_cols.get(&def.mart).unwrap_or(&empty_cols);

    let mut where_clauses: Vec<String> = Vec::new();
    if !years.is_empty() && cols.contains("tahun") {
        let years_csv = years
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        where_clauses.push(format!("tahun IN ({years_csv})"));
    }
    for f in filters {
        if f.values.is_empty() {
            continue;
        }
        // Both checks are required — see the "Fidelity notes" doc above.
        let Ok(column) = Ident::new(f.column.clone()) else {
            continue;
        };
        if !cols.contains(&f.column) {
            continue;
        }
        let list = f
            .values
            .iter()
            .map(|v| SqlLiteral::from(v.clone()).to_string())
            .collect::<Vec<_>>()
            .join(",");
        where_clauses.push(format!("{column} IN ({list})"));
    }

    if where_clauses.is_empty() {
        return spec.spec.sql.clone();
    }

    // `Aggregate::from_str_lossy` falls back to `Sum` for a missing OR
    // unrecognized value — this is the untrusted path named in the H4
    // finding (`def.aggregate` comes straight from stored `spec_json`, never
    // re-checked against an allowlist), so it must never hand raw text to
    // the SQL builder below.
    let agg = Aggregate::from_str_lossy(def.aggregate.as_deref().unwrap_or("sum"));
    // `mart` was already validated as a well-formed identifier when the
    // spec was created via `specFromInput`, so re-validating here would
    // only ever fail on data corruption; fall back to the stored SQL rather
    // than panicking, matching "SQL never comes raw from untrusted input"
    // without introducing a new failure mode.
    let Ok(mart) = Ident::new(def.mart.clone()) else {
        return spec.spec.sql.clone();
    };

    if matches!(
        spec.spec.kind,
        crate::specs::ChartKind::Kpi | crate::specs::ChartKind::Gauge
    ) {
        let Some(measure_raw) = def.measures.first() else {
            return spec.spec.sql.clone();
        };
        let Ok(measure) = Ident::new(measure_raw.clone()) else {
            return spec.spec.sql.clone();
        };
        return build_kpi_sql(&mart, &measure, agg, &where_clauses);
    }

    let Ok(dimension) = Ident::new(def.dimension.clone()) else {
        return spec.spec.sql.clone();
    };
    let mut measures = Vec::with_capacity(def.measures.len());
    for m in &def.measures {
        let Ok(m) = Ident::new(m.clone()) else {
            return spec.spec.sql.clone();
        };
        measures.push(m);
    }
    let breakdown = def
        .breakdown
        .as_ref()
        .and_then(|b| Ident::new(b.clone()).ok());

    let mut builder = QueryBuilder::new(mart)
        .dimension(dimension)
        .aggregate(agg)
        .order(def.order.clone().unwrap_or_else(|| "none".to_owned()))
        .limit(def.limit.unwrap_or(20))
        .breakdown(breakdown)
        .measures(measures);
    for clause in where_clauses {
        // The predicates were already assembled as complete SQL fragments
        // above (to preserve `buildSql`'s literal WHERE-joining behavior);
        // route them through as a single pre-built filter rather than
        // re-deriving column/values, by pushing directly onto the builder's
        // internal list via a raw passthrough.
        builder = builder.raw_where(clause);
    }
    builder.build()
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
/// loaded from a deployment-supplied `BUILTIN_DASHBOARD_SPEC` file (Task 6),
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
pub fn apply_builtin_year_filter<HMap, HSet>(
    sql: &str,
    mart: &str,
    years: &[i64],
    mart_cols: &std::collections::HashMap<String, HashSet<String, HSet>, HMap>,
) -> String
where
    HMap: std::hash::BuildHasher,
    HSet: std::hash::BuildHasher,
{
    if years.is_empty() {
        return sql.to_owned();
    }
    let Some(cols) = mart_cols.get(mart) else {
        return sql.to_owned();
    };
    if !cols.contains("tahun") {
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

    fn mart_cols(pairs: &[(&str, &[&str])]) -> HashMap<String, HashSet<String>> {
        pairs
            .iter()
            .map(|(mart, cols)| {
                (
                    (*mart).to_owned(),
                    cols.iter().map(|c| (*c).to_owned()).collect(),
                )
            })
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
        let filters = vec![FilterDef {
            column: "negara".to_owned(),
            values: vec!["ID".to_owned()],
        }];
        let sql = sql_with_filters(&spec, &[], &filters, &cols);
        assert_eq!(sql, spec.spec.sql);
    }

    #[test]
    fn escapes_quote_inside_filter_value() {
        let spec = stored_spec(ChartKind::Bar, "mart_wisman", "kawasan", &["jumlah"]);
        let cols = mart_cols(&[("mart_wisman", &["kawasan", "jumlah"])]);
        let filters = vec![FilterDef {
            column: "kawasan".to_owned(),
            values: vec!["O'Brien".to_owned()],
        }];
        let sql = sql_with_filters(&spec, &[], &filters, &cols);
        assert!(sql.contains("'O''Brien'"), "{sql}");
    }

    #[test]
    fn rejects_filter_column_that_is_not_a_valid_identifier() {
        let spec = stored_spec(ChartKind::Bar, "mart_wisman", "kawasan", &["jumlah"]);
        let mut cols = mart_cols(&[("mart_wisman", &["kawasan", "jumlah"])]);
        cols.get_mut("mart_wisman")
            .unwrap()
            .insert("bad col".to_owned());
        let filters = vec![FilterDef {
            column: "bad col".to_owned(),
            values: vec!["x".to_owned()],
        }];
        let sql = sql_with_filters(&spec, &[], &filters, &cols);
        assert_eq!(sql, spec.spec.sql);
    }

    #[test]
    fn text_chart_produces_no_sql() {
        let spec = stored_spec(ChartKind::Text, "", "", &[]);
        let cols: HashMap<String, HashSet<String>> = HashMap::new();
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
        let filters = vec![FilterDef {
            column: "_part".to_owned(),
            values: vec!["all_0_0_0".to_owned()],
        }];
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
    // deployment-supplied JSON file (Task 6), so "none of the 13 shipped
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
}
