//! Enforcement-side SQL rewriter: applies masking and row-filter
//! obligations to a query before it ever reaches `ClickHouse`.
//!
//! # Why table substitution, not expression rewriting (WS7 plan, Hard
//! Requirement 1)
//!
//! An earlier draft masked individual `Expr` leaves inside the `SELECT`
//! projection list. That is unsound: a masked column left clear in
//! `WHERE`/`GROUP BY`/`ORDER BY`/`HAVING`/`JOIN ... ON`/a window
//! `PARTITION BY` is a value oracle (`SELECT count() FROM t WHERE email
//! LIKE 'a%'`, then bisect), and projection-list rewriting cannot see
//! through `ClickHouse`'s dynamic selectors (`COLUMNS('e.*')`, `*
//! EXCEPT (...)`, `* REPLACE (...)`, a qualified `t.*`) or a lambda body
//! at all, since none of those name a column as a leaf
//! `Expr::Identifier`/`Expr::CompoundIdentifier`.
//!
//! The fix is table substitution: wherever a governed table is read —
//! any `FROM`/`JOIN`, at any nesting depth, inside a CTE's own
//! definition, inside each `UNION` branch — the `TableFactor::Table`
//! node naming it is replaced with a derived table that projects every
//! real column (masked ones rewritten) and applies the row filter in
//! its own `WHERE`. Everything outside that derived table then reads
//! only the already-masked, already-filtered projection; no other
//! construct needs to be individually recognized or walked.
//!
//! Unparseable input, a construct outside the supported set, or a
//! governed table this module cannot prove it fully understands is
//! **refused** (an `Err`), never silently passed through unmodified.

/// Real, observed `(label, sql)` results — hand-transcribed, never
/// guessed — from
/// `parses_real_repository_query_shapes::record_m1_class_parse_results`
/// (WS7 item B1 Step 3), run with `cargo test -p lakehouse-api --lib
/// sql_rewrite:: -- --nocapture` against `sqlparser` 0.62.0 with
/// `ClickHouseDialect`. Of the twelve M1-listed leak classes, eleven
/// parse under `ClickHouseDialect` today (`ARRAY JOIN` included) and are
/// proven safe by `table_substitution`'s own tests (WS7 item B3) instead.
/// Only `with_scalar_alias` (`WITH 2024 AS target_year SELECT * FROM
/// silver.customers WHERE tahun = target_year` — a bare numeric-literal
/// `WITH` binding, distinct from a `WITH ... AS (<query>)` CTE) is
/// rejected by `sqlparser` 0.62.0's `ClickHouseDialect` grammar; it is
/// refused end-to-end (`RewriteError::Unparseable`), never silently
/// treated as touching no governed table — see
/// `refuses_unparseable_shapes::every_recorded_unparseable_class_is_refused_end_to_end`
/// (WS7 item B4).
#[allow(
    dead_code,
    reason = "only a #[cfg(test)] reader exists (WS7 item B4's \
              refuses_unparseable_shapes module); never reachable from the \
              lakehouse-api binary target until Phase C wires enforce() in"
)]
pub(crate) const REFUSED_UNPARSEABLE: &[(&str, &str)] = &[(
    "with_scalar_alias",
    "WITH 2024 AS target_year SELECT * FROM silver.customers WHERE tahun = target_year",
)];

use std::collections::HashSet;
use std::convert::Infallible;
use std::ops::ControlFlow;

use sqlparser::ast::{Query, TableFactor, Visit, Visitor};
use sqlparser::dialect::Dialect;
use sqlparser::parser::Parser;

/// Collects every [`Query`] node anywhere in a visited AST — the
/// top-level query itself, each CTE's own body, every subquery no
/// matter how deeply nested (a `FROM`, a `WHERE`/`HAVING` predicate, a
/// `SELECT`-list expression, a function argument, a `UNION` branch, a
/// derived table) — via `sqlparser`'s own derived `Visit` traversal
/// (the `visitor` Cargo feature enabled in this crate's `Cargo.toml`),
/// never a hand-written recursive match over every `Expr` variant that
/// could silently miss one. [`referenced_tables`] and
/// [`substitute_governed_tables`] (WS7 item B3) both start from this same
/// flattening, so neither can miss a table hiding inside an expression
/// position the other author did not think to hand-enumerate.
struct QueryCollector {
    found: Vec<Query>,
}

impl Visitor for QueryCollector {
    type Break = Infallible;

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<Infallible> {
        self.found.push(query.clone());
        ControlFlow::Continue(())
    }
}

/// Every [`Query`] node reachable from `stmt`, flattened — see
/// [`QueryCollector`].
fn all_queries(stmt: &sqlparser::ast::Statement) -> Vec<Query> {
    let mut collector = QueryCollector { found: Vec::new() };
    let _: ControlFlow<Infallible> = stmt.visit(&mut collector);
    collector.found
}

/// Every real table (never a CTE alias) referenced anywhere in `sql`,
/// canonicalized to `"schema.table"` — see [`canonicalize`]. `None`
/// means `sql` did not parse under `dialect` at all; the caller treats
/// that as "cannot prove this query touches no governed table" and
/// refuses per the fail-closed rule (WS7 plan, Hard Requirement 2).
/// Read-only: used to decide WHICH tables need substitution before
/// [`substitute_governed_tables`]'s mutating pass runs (WS7 item B3), and
/// (unlike that pass) collapses a self-join's two occurrences of the
/// same table into one name — a call site that needs every AST
/// POSITION, not just every distinct name, uses WS7 item B3's
/// `substitute_governed_tables` directly.
///
/// A bare, unqualified table name is excluded here whenever it matches
/// a CTE alias declared ANYWHERE in `sql` (not lexically scoped to the
/// exact position) — safe because every table this module's caller
/// ever governs is authored fully qualified (`policy_engine::
/// PolicyCondition::table`, e.g. `"serving.mart_x"`), so a bare 1-part
/// name can never collide with a governed table's canonical key
/// regardless of CTE scoping precision; excluding it too eagerly only
/// ever affects this function's own "is this a CTE" cosmetics, never
/// which tables get substituted.
pub fn referenced_tables(sql: &str, dialect: &dyn Dialect) -> Option<Vec<String>> {
    let statements = Parser::parse_sql(dialect, sql).ok()?;
    let mut queries = Vec::new();
    for stmt in &statements {
        queries.extend(all_queries(stmt));
    }
    let ctes: HashSet<String> = queries
        .iter()
        .flat_map(|q| q.with.iter())
        .flat_map(|with| with.cte_tables.iter())
        .map(|cte| cte.alias.name.value.to_ascii_lowercase())
        .collect();
    let mut out = HashSet::new();
    for query in &queries {
        for twj in top_level_table_with_joins(query) {
            collect_table_factor_name(&twj.relation, &ctes, &mut out);
            for join in &twj.joins {
                collect_table_factor_name(&join.relation, &ctes, &mut out);
            }
        }
    }
    Some(out.into_iter().collect())
}

/// `query.body`'s own top-level `FROM` clause, unwrapping a
/// parenthesized `SetExpr::Query` (`(SELECT ... )` around a whole
/// query body, distinct from a derived-table subquery — both cases are
/// already separately present in [`all_queries`]'s flat list) and a
/// `SetExpr::SetOperation` (`UNION`/`EXCEPT`/`INTERSECT`)'s two sides,
/// so a `UNION` branch's own `FROM` is reached even though it has no
/// separate [`Query`] node of its own (a `SetExpr::SetOperation` side
/// is a `SetExpr`, not a boxed `Query`).
fn top_level_table_with_joins(query: &Query) -> Vec<&sqlparser::ast::TableWithJoins> {
    fn from_set_expr(expr: &sqlparser::ast::SetExpr) -> Vec<&sqlparser::ast::TableWithJoins> {
        match expr {
            sqlparser::ast::SetExpr::Select(select) => select.from.iter().collect(),
            sqlparser::ast::SetExpr::SetOperation { left, right, .. } => {
                let mut out = from_set_expr(left);
                out.extend(from_set_expr(right));
                out
            }
            // SetExpr::Query is a parenthesized query body; its own
            // Query node is separately present in `all_queries`'s flat
            // list (pre_visit_query fires for it too), so it is not
            // walked again here.
            _ => Vec::new(),
        }
    }
    from_set_expr(&query.body)
}

/// Records `factor`'s own canonical table name into `out`, when it is
/// an ordinary `TableFactor::Table` naming a real table (not a
/// table-function call — `args.is_some()` — which WS7 item B5 classifies
/// and refuses separately, and not a bare name matching a CTE alias).
/// Does NOT recurse into `TableFactor::Derived`'s subquery or
/// `TableFactor::NestedJoin`'s inner joins beyond one level of
/// unwrapping — both are reachable through [`all_queries`]/this same
/// function being called again for the nested/derived query's own
/// top-level `FROM`, so a second, deeper recursion here would only
/// duplicate (harmlessly, into the same `HashSet`) work already done.
fn collect_table_factor_name(
    factor: &TableFactor,
    ctes: &HashSet<String>,
    out: &mut HashSet<String>,
) {
    match factor {
        TableFactor::Table { name, args, .. } => {
            if args.is_some() {
                return;
            }
            let parts: Vec<String> = name
                .0
                .iter()
                .filter_map(sqlparser::ast::ObjectNamePart::as_ident)
                .map(|ident| ident.value.to_ascii_lowercase())
                .collect();
            if parts.len() == 1 && ctes.contains(&parts[0]) {
                return; // a CTE reference, not a real table
            }
            out.insert(canonicalize(&parts));
        }
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => {
            collect_table_factor_name(&table_with_joins.relation, ctes, out);
            for join in &table_with_joins.joins {
                collect_table_factor_name(&join.relation, ctes, out);
            }
        }
        // TableFactor::Derived's own subquery is a Query node already
        // present in `all_queries`'s flat list — its tables are found
        // when `referenced_tables` processes THAT entry, not here.
        _ => {}
    }
}

/// `"schema.table"`, dropping a leading catalog part when three parts
/// are given (`lake.serving.mart_x` / Trino's `iceberg.serving.mart_x`
/// both canonicalize to `serving.mart_x`) and leaving a bare one-part
/// name as-is.
fn canonicalize(parts: &[String]) -> String {
    match parts.len() {
        0 => String::new(),
        1 => parts[0].clone(),
        2 => parts.join("."),
        _ => parts[parts.len() - 2..].join("."),
    }
}

#[cfg(test)]
mod table_resolution {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use sqlparser::dialect::ClickHouseDialect;

    use super::referenced_tables;

    #[test]
    fn finds_a_plain_table() {
        let tables =
            referenced_tables("SELECT * FROM serving.mart_x", &ClickHouseDialect {}).unwrap();
        assert_eq!(tables, vec!["serving.mart_x".to_owned()]);
    }

    #[test]
    fn normalizes_backtick_and_catalog_qualification_to_the_same_canonical_name() {
        for sql in [
            "SELECT * FROM serving.mart_x",
            "SELECT * FROM `serving`.`mart_x`",
            "SELECT * FROM lake.serving.mart_x", // 3-part ClickHouse-style
            "SELECT * FROM iceberg.serving.mart_x", // Trino catalog-qualified
        ] {
            let tables = referenced_tables(sql, &ClickHouseDialect {}).unwrap();
            assert!(
                tables.contains(&"serving.mart_x".to_owned()),
                "sql={sql} tables={tables:?}"
            );
        }
    }

    #[test]
    fn finds_tables_inside_a_cte_and_does_not_treat_the_cte_name_as_a_real_table() {
        let sql = "WITH recent AS (SELECT id FROM silver.orders_enriched) SELECT * FROM recent";
        let tables = referenced_tables(sql, &ClickHouseDialect {}).unwrap();
        assert_eq!(tables, vec!["silver.orders_enriched".to_owned()]);
    }

    #[test]
    fn finds_tables_inside_from_where_and_select_subqueries() {
        let sql = "SELECT (SELECT max(id) FROM silver.a) AS m FROM silver.b \
                   WHERE id IN (SELECT id FROM silver.c)";
        let mut tables = referenced_tables(sql, &ClickHouseDialect {}).unwrap();
        tables.sort();
        assert_eq!(
            tables,
            vec![
                "silver.a".to_owned(),
                "silver.b".to_owned(),
                "silver.c".to_owned()
            ]
        );
    }

    #[test]
    fn finds_tables_on_both_sides_of_a_union() {
        let sql = "SELECT id FROM silver.a UNION ALL SELECT id FROM silver.b";
        let mut tables = referenced_tables(sql, &ClickHouseDialect {}).unwrap();
        tables.sort();
        assert_eq!(tables, vec!["silver.a".to_owned(), "silver.b".to_owned()]);
    }

    #[test]
    fn finds_every_joined_table_including_a_self_join_twice() {
        let sql =
            "SELECT * FROM silver.a JOIN silver.b ON a.id = b.id LEFT JOIN silver.c ON a.id = c.id";
        let mut tables = referenced_tables(sql, &ClickHouseDialect {}).unwrap();
        tables.sort();
        assert_eq!(
            tables,
            vec![
                "silver.a".to_owned(),
                "silver.b".to_owned(),
                "silver.c".to_owned()
            ]
        );

        // A self-join names the SAME canonical table twice, at two
        // distinct AST positions — referenced_tables (a set) collapses
        // that to one canonical name, but WS7 item B3's MUTATING walk
        // (unlike this read-only one) visits and substitutes both
        // TableFactor nodes independently, proven by its own
        // `self_join` test, not this one.
        let self_join =
            "SELECT a.id FROM silver.customers a JOIN silver.customers b ON a.email = b.email";
        assert_eq!(
            referenced_tables(self_join, &ClickHouseDialect {}).unwrap(),
            vec!["silver.customers".to_owned()]
        );
    }
}

use std::collections::HashMap;

use sqlparser::ast::{
    Expr, Function, FunctionArg, FunctionArgExpr, FunctionArguments, Ident as SqlIdent, ObjectName,
    ObjectNamePart, SelectItem, SetExpr, Statement, TableAlias, Value, ValueWithSpan,
};

/// Reasons this module refuses to rewrite a statement. Every variant is
/// an explicit refusal, never a silent pass-through (WS7 plan, Hard
/// Requirement 2) — a caller that receives an `Err` must not forward
/// the original, unrewritten SQL to `ClickHouse` in its place.
#[derive(Debug, thiserror::Error)]
pub enum RewriteError {
    /// `sql` did not parse under the configured dialect at all — this
    /// module cannot prove the statement touches no governed table, so
    /// it refuses rather than assume it is safe.
    #[error("statement did not parse under the configured SQL dialect")]
    Unparseable,
    /// A governed table's obligations could not be turned into a safe
    /// rewrite — most commonly `real_columns: None` (WS7 item B3's own
    /// `refuses_when_the_real_column_list_is_unknown` test), but also
    /// the fail-closed backstop in [`substitute_governed_tables`]: a
    /// table this function proved (via [`referenced_tables`]) is
    /// touched SOMEWHERE in the statement, yet was never substituted by
    /// the mutating pass — a shape the mutating walk does not (yet)
    /// descend into, refused rather than returned half-protected.
    #[error("cannot prove the rewrite is safe for table `{table}`")]
    UnprovableSubstitution {
        /// The governed table's own canonical `"schema.table"` name.
        table: String,
    },
    /// An authored row filter is not a real, validated expression —
    /// WS7 item B4 formalizes the exact grammar this rejects.
    #[error("invalid row filter: {reason}")]
    InvalidRowFilter {
        /// The specific problem found in the filter text.
        reason: String,
    },
    /// A table-function call (`FROM name(...)`) is not on the (empty)
    /// table-function allowlist — see [`ALLOWED_TABLE_FUNCTIONS`].
    #[error("table function `{name}` is not permitted in a governed query")]
    TableFunctionDenied {
        /// The called function's own name, as written in the query.
        name: String,
    },
    /// A sensitive `system.*` table was read without `audit:read` — see
    /// [`SENSITIVE_SYSTEM_TABLES`].
    #[error("reading `{table}` requires the audit:read permission")]
    SensitiveSystemTable {
        /// The sensitive table's canonical `"schema.table"` name.
        table: String,
    },
    /// A `dictGet*`/`joinGet*` family call was found while the calling
    /// principal has ANY authored obligation anywhere — see
    /// [`is_dict_or_join_function`]'s own doc comment for why the whole
    /// family is refused rather than traced to a specific dictionary.
    #[error("`{name}` is not permitted for a principal with any authored obligation")]
    DictOrJoinFunctionDenied {
        /// The called function's own name, as written in the query.
        name: String,
    },
    /// A view whose own defining query reads a governed table was
    /// found — refused rather than inlined or itself substituted (WS7
    /// plan, Hard Requirement 4).
    #[error("view `{view}` reads a governed table and cannot be rewritten safely")]
    #[allow(
        dead_code,
        reason = "only a #[cfg(test)] constructor exists in this commit \
                  (WS7 item B5's classify_views tests); WS7 item B6's enforce() is \
                  the first production caller"
    )]
    ViewOverGovernedTable {
        /// The view's own canonical `"schema.table"` name.
        view: String,
    },
}

/// Real column list + obligations for one governed table.
/// `real_columns: None` means the caller could not determine the
/// table's real columns — substitution refuses rather than guess which
/// columns to project (`RewriteError::UnprovableSubstitution`).
#[derive(Debug, Clone)]
pub struct TableObligations {
    /// Columns to mask on read.
    pub mask: Vec<String>,
    /// A row-filter expression, already authored and (at authoring
    /// time, `policy_engine::PolicyCondition`) roughly shaped, but
    /// re-validated here through [`validate_row_filter_expr`] before
    /// ever reaching SQL text — never trusted as a raw string.
    pub row_filter: Option<String>,
    /// The table's real, ordered column list, or `None` when the
    /// caller could not resolve it (refuses rather than guesses).
    pub real_columns: Option<Vec<String>>,
}

/// The calling principal's own id/tenant ids, expanded into a row
/// filter's two closed placeholders (WS7 item B4, M3) — threaded from the
/// real `Principal` in Phase C, never read from anywhere else, so a
/// placeholder can never expand to anyone but the actual caller.
#[derive(Debug, Clone, Default)]
pub struct PlaceholderValues {
    /// Expansion for `__principal_id__`.
    pub principal_id: Option<String>,
    /// Expansion for `__principal_tenant_ids__`.
    pub principal_tenant_ids: Vec<String>,
}

impl PlaceholderValues {
    /// No placeholder values available — used by a caller with no
    /// principal context (tests, and any row filter that uses neither
    /// placeholder).
    #[must_use]
    #[allow(
        dead_code,
        reason = "no non-test caller exists yet in this commit (WS7 item B3); \
                  WS7 item B6's enforce() and Phase C's real principal wiring are \
                  the first production callers"
    )]
    pub fn none() -> Self {
        Self::default()
    }
}

/// Bare identifier naming the calling principal's own id — valid
/// anywhere an ordinary column identifier is valid.
pub const PRINCIPAL_ID_PLACEHOLDER: &str = "__principal_id__";
/// Bare identifier naming the calling principal's tenant ids — valid
/// ONLY as the sole element of an `IN (...)` list.
pub const PRINCIPAL_TENANT_IDS_PLACEHOLDER: &str = "__principal_tenant_ids__";
/// Functions a row filter may call. Anything else is refused — a row
/// filter is a closed grammar, not general SQL (WS7 plan, Hard
/// Requirement 2).
const ALLOWED_ROW_FILTER_FUNCTIONS: &[&str] = &["lower", "upper", "tostring", "todate"];

/// The one entry point WS7 item B6 calls. Parses `sql`, and for EVERY
/// `TableFactor::Table` this module can reach — any `FROM`, any
/// `JOIN`'s relation, inside a CTE's own body, inside each `UNION`
/// branch, inside a derived-table subquery, inside a `WHERE`/`HAVING`/
/// `SELECT`-list subquery — whose canonicalized name has an entry in
/// `obligations`, replaces that node with a derived table masking/
/// filtering it. A canonicalized name absent from `obligations`
/// (including a CTE's own alias, never looked up — see
/// [`referenced_tables`]'s own doc comment for why that is safe) is
/// left completely untouched.
///
/// After the mutating pass, this function re-checks: every table
/// [`referenced_tables`] (an exhaustive, `Visit`-based read-only walk —
/// WS7 item B2) proves is touched SOMEWHERE in `sql` AND has an
/// `obligations` entry must also appear in the set of tables the
/// mutating pass actually substituted. The mutating pass's own
/// recursion into expression positions (`WHERE`/`HAVING`/`SELECT`-list
/// subqueries, function arguments) is broad but not exhaustively
/// enumerated over every `Expr` variant; this backstop is what makes
/// the whole function sound regardless — a governed table hiding in a
/// shape the mutating walk does not descend into is refused
/// (`RewriteError::UnprovableSubstitution`), never silently returned
/// unprotected.
///
/// # Errors
/// [`RewriteError::Unparseable`] if `sql` does not parse;
/// [`RewriteError::UnprovableSubstitution`] if a governed table's
/// `real_columns` is `None`, its masked/filtered projection fails to
/// re-parse, or the post-substitution backstop finds a governed table
/// the mutating pass never reached.
#[allow(
    dead_code,
    reason = "no non-test caller exists yet in this commit (WS7 item B3); \
              WS7 item B6's enforce() is the first production caller"
)]
#[allow(
    clippy::implicit_hasher,
    reason = "this crate never builds a HashMap<String, TableObligations> with a \
              non-default hasher; generalizing over BuildHasher adds a type \
              parameter to every caller for a capability nothing here uses"
)]
pub fn substitute_governed_tables(
    sql: &str,
    dialect: &dyn Dialect,
    obligations: &HashMap<String, TableObligations>,
    placeholders: &PlaceholderValues,
) -> Result<String, RewriteError> {
    let mut statements = Parser::parse_sql(dialect, sql).map_err(|_| RewriteError::Unparseable)?;
    let touched = referenced_tables(sql, dialect).ok_or(RewriteError::Unparseable)?;
    let mut substituted = HashSet::new();
    for stmt in &mut statements {
        substitute_in_statement(stmt, dialect, obligations, placeholders, &mut substituted)?;
    }
    for table in &touched {
        if obligations.contains_key(table) && !substituted.contains(table) {
            return Err(RewriteError::UnprovableSubstitution {
                table: table.clone(),
            });
        }
    }
    Ok(statements
        .iter()
        .map(Statement::to_string)
        .collect::<Vec<_>>()
        .join("; "))
}

fn substitute_in_statement(
    stmt: &mut Statement,
    dialect: &dyn Dialect,
    obligations: &HashMap<String, TableObligations>,
    placeholders: &PlaceholderValues,
    substituted: &mut HashSet<String>,
) -> Result<(), RewriteError> {
    match stmt {
        Statement::Query(query) => {
            substitute_in_query(query, dialect, obligations, placeholders, substituted)
        }
        Statement::Explain { statement, .. } => {
            substitute_in_statement(statement, dialect, obligations, placeholders, substituted)
        }
        // Every other statement kind is handled by WS7 item B5's refusal
        // rules, which run before this function is reached — see Task
        // B6's `enforce`.
        _ => Ok(()),
    }
}

fn substitute_in_query(
    query: &mut Query,
    dialect: &dyn Dialect,
    obligations: &HashMap<String, TableObligations>,
    placeholders: &PlaceholderValues,
    substituted: &mut HashSet<String>,
) -> Result<(), RewriteError> {
    if let Some(with) = &mut query.with {
        for cte in &mut with.cte_tables {
            // Substituted inside the CTE's own body — a later
            // reference to the CTE's name never looks up `obligations`
            // at all (bare names never match a dotted governed-table
            // key; see `referenced_tables`'s own doc comment), so no
            // separate CTE-name bookkeeping is needed here either.
            substitute_in_query(
                &mut cte.query,
                dialect,
                obligations,
                placeholders,
                substituted,
            )?;
        }
    }
    substitute_in_set_expr(
        &mut query.body,
        dialect,
        obligations,
        placeholders,
        substituted,
    )
}

fn substitute_in_set_expr(
    expr: &mut SetExpr,
    dialect: &dyn Dialect,
    obligations: &HashMap<String, TableObligations>,
    placeholders: &PlaceholderValues,
    substituted: &mut HashSet<String>,
) -> Result<(), RewriteError> {
    match expr {
        SetExpr::Select(select) => {
            for twj in &mut select.from {
                substitute_table_factor(
                    &mut twj.relation,
                    dialect,
                    obligations,
                    placeholders,
                    substituted,
                )?;
                for join in &mut twj.joins {
                    substitute_table_factor(
                        &mut join.relation,
                        dialect,
                        obligations,
                        placeholders,
                        substituted,
                    )?;
                }
            }
            if let Some(selection) = &mut select.selection {
                substitute_in_expr(selection, dialect, obligations, placeholders, substituted)?;
            }
            if let Some(having) = &mut select.having {
                substitute_in_expr(having, dialect, obligations, placeholders, substituted)?;
            }
            for item in &mut select.projection {
                substitute_in_select_item(item, dialect, obligations, placeholders, substituted)?;
            }
            Ok(())
        }
        SetExpr::Query(inner) => {
            substitute_in_query(inner, dialect, obligations, placeholders, substituted)
        }
        SetExpr::SetOperation { left, right, .. } => {
            // Each UNION branch substituted independently.
            substitute_in_set_expr(left, dialect, obligations, placeholders, substituted)?;
            substitute_in_set_expr(right, dialect, obligations, placeholders, substituted)
        }
        SetExpr::Values(_)
        | SetExpr::Insert(_)
        | SetExpr::Update(_)
        | SetExpr::Delete(_)
        | SetExpr::Merge(_)
        | SetExpr::Table(_) => Ok(()),
    }
}

fn substitute_in_select_item(
    item: &mut SelectItem,
    dialect: &dyn Dialect,
    obligations: &HashMap<String, TableObligations>,
    placeholders: &PlaceholderValues,
    substituted: &mut HashSet<String>,
) -> Result<(), RewriteError> {
    match item {
        SelectItem::UnnamedExpr(e)
        | SelectItem::ExprWithAlias { expr: e, .. }
        | SelectItem::ExprWithAliases { expr: e, .. } => {
            substitute_in_expr(e, dialect, obligations, placeholders, substituted)
        }
        // A wildcard/qualified-wildcard selects from whatever the FROM
        // clause's own substitution already produced — no expression to
        // recurse into.
        _ => Ok(()),
    }
}

/// Recurses into the common `Expr` positions that can hold a nested
/// `Query` (a `WHERE`/`HAVING`/`SELECT`-list subquery, `IN (SELECT
/// ...)`, `EXISTS (...)`, a function argument), substituting any
/// governed table found inside. Not an exhaustive match over every
/// `Expr` variant — [`substitute_governed_tables`]'s own post-pass
/// backstop refuses the statement outright if a governed table turns
/// out to be reachable only through a shape this function does not
/// descend into, so an incomplete match here fails closed rather than
/// silently under-protecting.
fn substitute_in_expr(
    expr: &mut Expr,
    dialect: &dyn Dialect,
    obligations: &HashMap<String, TableObligations>,
    placeholders: &PlaceholderValues,
    substituted: &mut HashSet<String>,
) -> Result<(), RewriteError> {
    match expr {
        Expr::Subquery(q) | Expr::Exists { subquery: q, .. } => {
            substitute_in_query(q, dialect, obligations, placeholders, substituted)
        }
        Expr::InSubquery {
            expr: inner,
            subquery,
            ..
        } => {
            substitute_in_expr(inner, dialect, obligations, placeholders, substituted)?;
            substitute_in_query(subquery, dialect, obligations, placeholders, substituted)
        }
        Expr::BinaryOp { left, right, .. } => {
            substitute_in_expr(left, dialect, obligations, placeholders, substituted)?;
            substitute_in_expr(right, dialect, obligations, placeholders, substituted)
        }
        Expr::UnaryOp { expr: inner, .. }
        | Expr::Nested(inner)
        | Expr::IsNull(inner)
        | Expr::IsNotNull(inner) => {
            substitute_in_expr(inner, dialect, obligations, placeholders, substituted)
        }
        Expr::Between {
            expr: inner,
            low,
            high,
            ..
        } => {
            substitute_in_expr(inner, dialect, obligations, placeholders, substituted)?;
            substitute_in_expr(low, dialect, obligations, placeholders, substituted)?;
            substitute_in_expr(high, dialect, obligations, placeholders, substituted)
        }
        Expr::Like {
            expr: inner,
            pattern,
            ..
        }
        | Expr::ILike {
            expr: inner,
            pattern,
            ..
        } => {
            substitute_in_expr(inner, dialect, obligations, placeholders, substituted)?;
            substitute_in_expr(pattern, dialect, obligations, placeholders, substituted)
        }
        Expr::InList {
            expr: inner, list, ..
        } => {
            substitute_in_expr(inner, dialect, obligations, placeholders, substituted)?;
            for item in list {
                substitute_in_expr(item, dialect, obligations, placeholders, substituted)?;
            }
            Ok(())
        }
        Expr::Function(f) => {
            substitute_in_function(f, dialect, obligations, placeholders, substituted)
        }
        // A leaf (identifier, literal) or a composite this module does
        // not descend into — see this function's own doc comment on
        // the post-pass backstop.
        _ => Ok(()),
    }
}

fn substitute_in_function(
    f: &mut Function,
    dialect: &dyn Dialect,
    obligations: &HashMap<String, TableObligations>,
    placeholders: &PlaceholderValues,
    substituted: &mut HashSet<String>,
) -> Result<(), RewriteError> {
    match &mut f.args {
        FunctionArguments::List(list) => {
            for arg in &mut list.args {
                let inner = match arg {
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(e))
                    | FunctionArg::Named {
                        arg: FunctionArgExpr::Expr(e),
                        ..
                    } => Some(e),
                    _ => None,
                };
                if let Some(e) = inner {
                    substitute_in_expr(e, dialect, obligations, placeholders, substituted)?;
                }
            }
            Ok(())
        }
        FunctionArguments::Subquery(q) => {
            substitute_in_query(q, dialect, obligations, placeholders, substituted)
        }
        FunctionArguments::None => Ok(()),
    }
}

fn substitute_table_factor(
    factor: &mut TableFactor,
    dialect: &dyn Dialect,
    obligations: &HashMap<String, TableObligations>,
    placeholders: &PlaceholderValues,
    substituted: &mut HashSet<String>,
) -> Result<(), RewriteError> {
    match factor {
        TableFactor::Table {
            name, alias, args, ..
        } if args.is_none() => {
            let parts: Vec<String> = name
                .0
                .iter()
                .filter_map(ObjectNamePart::as_ident)
                .map(|ident| ident.value.to_ascii_lowercase())
                .collect();
            let canonical = canonicalize(&parts);
            let Some(obl) = obligations.get(&canonical) else {
                return Ok(());
            };
            let real_columns =
                obl.real_columns
                    .clone()
                    .ok_or_else(|| RewriteError::UnprovableSubstitution {
                        table: canonical.clone(),
                    })?;
            let derived_sql = build_masked_filtered_select(
                name,
                &real_columns,
                &obl.mask,
                obl.row_filter.as_deref(),
                placeholders,
            )?;
            let derived_stmts = Parser::parse_sql(dialect, &derived_sql).map_err(|_| {
                RewriteError::UnprovableSubstitution {
                    table: canonical.clone(),
                }
            })?;
            let Some(Statement::Query(derived_query)) = derived_stmts.into_iter().next() else {
                return Err(RewriteError::UnprovableSubstitution { table: canonical });
            };
            // Preserve the ORIGINAL alias when one was written (`c` in
            // `silver.customers c`) so every outer `c.email`/`c.*`
            // reference keeps resolving; otherwise alias to the
            // table's own last segment, so an unqualified `FROM
            // silver.customers` still resolves unqualified column
            // references the same way.
            let new_alias = alias.clone().unwrap_or_else(|| TableAlias {
                explicit: false,
                name: SqlIdent::new(parts.last().cloned().unwrap_or_default()),
                columns: vec![],
                at: None,
            });
            *factor = TableFactor::Derived {
                lateral: false,
                subquery: derived_query,
                alias: Some(new_alias),
                sample: None,
            };
            // Deliberately NOT recursing into `derived_query` here —
            // it intentionally references the raw, real table (the
            // one place its data is allowed to enter the query); the
            // substituted-tables bookkeeping below records the
            // canonical name as handled so the post-pass backstop in
            // `substitute_governed_tables` does not re-flag it.
            substituted.insert(canonical);
            Ok(())
        }
        TableFactor::Derived { subquery, .. } => {
            substitute_in_query(subquery, dialect, obligations, placeholders, substituted)
        }
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => {
            substitute_table_factor(
                &mut table_with_joins.relation,
                dialect,
                obligations,
                placeholders,
                substituted,
            )?;
            for join in &mut table_with_joins.joins {
                substitute_table_factor(
                    &mut join.relation,
                    dialect,
                    obligations,
                    placeholders,
                    substituted,
                )?;
            }
            Ok(())
        }
        // A table-function call (`args.is_some()`, matched by the
        // `TableFactor::Table { .. } if args.is_none()` guard above
        // failing) is classified and refused by WS7 item B5, not resolved
        // as a normal table reference here; any other `TableFactor`
        // variant has no table reference to substitute.
        _ => Ok(()),
    }
}

/// Builds the derived table's own SQL text — `SELECT` the projections,
/// `FROM` the table, an optional `WHERE` filter — with a masked column
/// wrapped in a `replaceRegexpAll(toString(...), '.*', '***') AS ...`
/// call. This is the ONLY place a masked column's raw name and a row
/// filter are
/// composed into SQL text — both are re-parsed immediately afterward
/// (`substitute_table_factor`'s own parse of `derived_sql`), so a
/// syntactically broken composition fails closed
/// (`UnprovableSubstitution`) rather than reaching `ClickHouse`
/// malformed. Every column name is validated through
/// `lakehouse_core::ident::Ident` before it is interpolated — `format!`
/// is used only for a validated identifier, never a raw, unchecked
/// string (AGENTS.md's SQL-value-binding rule). `filter` is validated
/// through [`validate_row_filter_expr`] and re-serialized from ITS OWN
/// parse (with placeholders expanded), never the raw authored string
/// concatenated.
fn build_masked_filtered_select(
    name: &ObjectName,
    real_columns: &[String],
    mask: &[String],
    filter: Option<&str>,
    placeholders: &PlaceholderValues,
) -> Result<String, RewriteError> {
    let mut projections = Vec::with_capacity(real_columns.len());
    for col in real_columns {
        let safe = lakehouse_core::ident::Ident::new(col).map_err(|_| {
            RewriteError::UnprovableSubstitution {
                table: name.to_string(),
            }
        })?;
        if mask.iter().any(|m| m.eq_ignore_ascii_case(col)) {
            projections.push(format!(
                "replaceRegexpAll(toString(`{safe}`), '.*', '***') AS `{safe}`"
            ));
        } else {
            projections.push(format!("`{safe}`"));
        }
    }
    let where_clause = match filter {
        Some(f) => {
            let expr = validate_row_filter_expr(f, real_columns)?;
            let expanded = expand_placeholders(&expr, placeholders)?;
            format!(" WHERE {expanded}")
        }
        None => String::new(),
    };
    Ok(format!(
        "SELECT {} FROM {name}{where_clause}",
        projections.join(", ")
    ))
}

/// Parses `raw` as a standalone `Expr` (via `GenericDialect`, wrapping
/// nothing — a row filter is never a full statement) and validates it:
/// every bare identifier is a real column OR [`PRINCIPAL_ID_PLACEHOLDER`];
/// [`PRINCIPAL_TENANT_IDS_PLACEHOLDER`] is valid ONLY as the sole
/// element of an `IN (...)` list; every function call is on
/// [`ALLOWED_ROW_FILTER_FUNCTIONS`]; no subquery/`EXISTS`/anything else
/// outside this narrow, explicitly allowed shape.
///
/// # Errors
/// [`RewriteError::InvalidRowFilter`] naming the specific problem.
pub fn validate_row_filter_expr(raw: &str, real_columns: &[String]) -> Result<Expr, RewriteError> {
    let dialect = sqlparser::dialect::GenericDialect {};
    let mut parser =
        Parser::new(&dialect)
            .try_with_sql(raw)
            .map_err(|e| RewriteError::InvalidRowFilter {
                reason: e.to_string(),
            })?;
    let expr = parser
        .parse_expr()
        .map_err(|e| RewriteError::InvalidRowFilter {
            reason: e.to_string(),
        })?;
    if !parser.consume_token(&sqlparser::tokenizer::Token::EOF) {
        return Err(RewriteError::InvalidRowFilter {
            reason: "trailing input after the filter expression".to_owned(),
        });
    }
    validate_expr_shape(&expr, real_columns)?;
    Ok(expr)
}

fn is_tenant_placeholder(expr: &Expr) -> bool {
    matches!(expr, Expr::Identifier(ident) if ident.value == PRINCIPAL_TENANT_IDS_PLACEHOLDER)
}

fn is_sole_tenant_placeholder(list: &[Expr]) -> bool {
    matches!(list, [only] if is_tenant_placeholder(only))
}

fn validate_identifier(name: &str, real_columns: &[String]) -> Result<(), RewriteError> {
    if name == PRINCIPAL_ID_PLACEHOLDER {
        return Ok(());
    }
    if name == PRINCIPAL_TENANT_IDS_PLACEHOLDER {
        return Err(RewriteError::InvalidRowFilter {
            reason: format!(
                "{PRINCIPAL_TENANT_IDS_PLACEHOLDER} may only appear as the sole element of an IN list"
            ),
        });
    }
    let lower = name.to_ascii_lowercase();
    if real_columns.iter().any(|c| c.to_ascii_lowercase() == lower) {
        Ok(())
    } else {
        Err(RewriteError::InvalidRowFilter {
            reason: format!("'{name}' is not a real column"),
        })
    }
}

/// Recursively rejects a subquery/`EXISTS`/disallowed function call;
/// every bare `Expr::Identifier`/`Expr::CompoundIdentifier` must be a
/// real column OR [`PRINCIPAL_ID_PLACEHOLDER`]; an `Expr::InList` whose
/// `list` is exactly one [`PRINCIPAL_TENANT_IDS_PLACEHOLDER`] identifier
/// is accepted as a unit — every OTHER appearance of that name is
/// rejected by the ordinary identifier check.
fn validate_expr_shape(expr: &Expr, real_columns: &[String]) -> Result<(), RewriteError> {
    match expr {
        Expr::Identifier(ident) => validate_identifier(&ident.value, real_columns),
        Expr::CompoundIdentifier(parts) => {
            let last = parts.last().ok_or_else(|| RewriteError::InvalidRowFilter {
                reason: "empty compound identifier".to_owned(),
            })?;
            validate_identifier(&last.value, real_columns)
        }
        Expr::Value(_) => Ok(()),
        Expr::InList {
            expr: inner, list, ..
        } => {
            if is_sole_tenant_placeholder(list) {
                validate_expr_shape(inner, real_columns)
            } else {
                validate_expr_shape(inner, real_columns)?;
                for item in list {
                    if is_tenant_placeholder(item) {
                        return Err(RewriteError::InvalidRowFilter {
                            reason: format!(
                                "{PRINCIPAL_TENANT_IDS_PLACEHOLDER} may only appear as the sole element of an IN list"
                            ),
                        });
                    }
                    validate_expr_shape(item, real_columns)?;
                }
                Ok(())
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            validate_expr_shape(left, real_columns)?;
            validate_expr_shape(right, real_columns)
        }
        Expr::UnaryOp { expr: inner, .. }
        | Expr::Nested(inner)
        | Expr::IsNull(inner)
        | Expr::IsNotNull(inner) => validate_expr_shape(inner, real_columns),
        Expr::Between {
            expr: inner,
            low,
            high,
            ..
        } => {
            validate_expr_shape(inner, real_columns)?;
            validate_expr_shape(low, real_columns)?;
            validate_expr_shape(high, real_columns)
        }
        Expr::Like {
            expr: inner,
            pattern,
            ..
        }
        | Expr::ILike {
            expr: inner,
            pattern,
            ..
        } => {
            validate_expr_shape(inner, real_columns)?;
            validate_expr_shape(pattern, real_columns)
        }
        Expr::Function(f) => validate_function_call(f, real_columns),
        _ => Err(RewriteError::InvalidRowFilter {
            reason: format!("unsupported row-filter expression: {expr}"),
        }),
    }
}

fn validate_function_call(f: &Function, real_columns: &[String]) -> Result<(), RewriteError> {
    let name = f.name.to_string().to_ascii_lowercase();
    if !ALLOWED_ROW_FILTER_FUNCTIONS.contains(&name.as_str()) {
        return Err(RewriteError::InvalidRowFilter {
            reason: format!("function '{name}' is not allowed in a row filter"),
        });
    }
    match &f.args {
        FunctionArguments::None => Ok(()),
        FunctionArguments::Subquery(_) => Err(RewriteError::InvalidRowFilter {
            reason: "a row filter may not contain a subquery".to_owned(),
        }),
        FunctionArguments::List(list) => {
            for arg in &list.args {
                match arg {
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(e))
                    | FunctionArg::Named {
                        arg: FunctionArgExpr::Expr(e),
                        ..
                    } => validate_expr_shape(e, real_columns)?,
                    _ => {
                        return Err(RewriteError::InvalidRowFilter {
                            reason: "unsupported function argument in a row filter".to_owned(),
                        });
                    }
                }
            }
            Ok(())
        }
    }
}

fn string_literal_expr(value: &str) -> Expr {
    Expr::Value(ValueWithSpan {
        value: Value::SingleQuotedString(value.to_owned()),
        span: sqlparser::tokenizer::Span::empty(),
    })
}

/// Re-walks an already-[`validate_row_filter_expr`]-validated `Expr`,
/// replacing [`PRINCIPAL_ID_PLACEHOLDER`] with a single string-literal
/// value, and an `IN (__principal_tenant_ids__)` list with one
/// string-literal element per `values.principal_tenant_ids` entry (or a
/// single `NULL` when the principal belongs to no tenant, so the
/// filter becomes "matches nothing" rather than an invalid `IN ()` or
/// an accidentally-permissive special case). Returns the RE-SERIALIZED
/// expression text (`Expr::to_string()` after substitution) — never a
/// find-and-replace on the raw authored string, so a value can never
/// land somewhere the parser did not intend.
///
/// # Errors
/// [`RewriteError::InvalidRowFilter`] if `expr` uses
/// [`PRINCIPAL_ID_PLACEHOLDER`] but `values.principal_id` is `None`.
pub fn expand_placeholders(
    expr: &Expr,
    values: &PlaceholderValues,
) -> Result<String, RewriteError> {
    Ok(expand_expr(expr, values)?.to_string())
}

fn expand_scalar_placeholder(values: &PlaceholderValues) -> Result<Expr, RewriteError> {
    let id = values
        .principal_id
        .as_deref()
        .ok_or_else(|| RewriteError::InvalidRowFilter {
            reason: format!("{PRINCIPAL_ID_PLACEHOLDER} used but no principal id is available"),
        })?;
    Ok(string_literal_expr(id))
}

/// One `Expr::Value(Value::Null)` node, used as the sole element of a
/// `principal_tenant_ids IN (...)` expansion when the calling principal
/// belongs to no tenant — see [`expand_expr`]'s own doc comment for why
/// `NULL`, not an empty list or a special-cased `FALSE`.
fn null_expr() -> Expr {
    Expr::Value(ValueWithSpan {
        value: Value::Null,
        span: sqlparser::tokenizer::Span::empty(),
    })
}

fn expand_tenant_list_placeholder(values: &PlaceholderValues) -> Vec<Expr> {
    if values.principal_tenant_ids.is_empty() {
        vec![null_expr()]
    } else {
        values
            .principal_tenant_ids
            .iter()
            .map(|t| string_literal_expr(t))
            .collect()
    }
}

/// Re-walks an already-validated `Expr` other than the two placeholder
/// shapes [`expand_expr`] handles directly, rebuilding every composite
/// node with its children expanded.
fn expand_expr_rest(expr: &Expr, values: &PlaceholderValues) -> Result<Expr, RewriteError> {
    match expr {
        Expr::InList {
            expr: inner,
            list,
            negated,
        } => {
            let mut new_list = Vec::with_capacity(list.len());
            for item in list {
                new_list.push(expand_expr(item, values)?);
            }
            Ok(Expr::InList {
                expr: Box::new(expand_expr(inner, values)?),
                list: new_list,
                negated: *negated,
            })
        }
        Expr::BinaryOp { left, op, right } => Ok(Expr::BinaryOp {
            left: Box::new(expand_expr(left, values)?),
            op: op.clone(),
            right: Box::new(expand_expr(right, values)?),
        }),
        Expr::UnaryOp { op, expr: inner } => Ok(Expr::UnaryOp {
            op: *op,
            expr: Box::new(expand_expr(inner, values)?),
        }),
        Expr::Nested(inner) => Ok(Expr::Nested(Box::new(expand_expr(inner, values)?))),
        Expr::IsNull(inner) => Ok(Expr::IsNull(Box::new(expand_expr(inner, values)?))),
        Expr::IsNotNull(inner) => Ok(Expr::IsNotNull(Box::new(expand_expr(inner, values)?))),
        Expr::Between {
            expr: inner,
            negated,
            low,
            high,
        } => Ok(Expr::Between {
            expr: Box::new(expand_expr(inner, values)?),
            negated: *negated,
            low: Box::new(expand_expr(low, values)?),
            high: Box::new(expand_expr(high, values)?),
        }),
        Expr::Like {
            negated,
            any,
            expr: inner,
            pattern,
            escape_char,
        } => Ok(Expr::Like {
            negated: *negated,
            any: *any,
            expr: Box::new(expand_expr(inner, values)?),
            pattern: Box::new(expand_expr(pattern, values)?),
            escape_char: escape_char.clone(),
        }),
        Expr::ILike {
            negated,
            any,
            expr: inner,
            pattern,
            escape_char,
        } => Ok(Expr::ILike {
            negated: *negated,
            any: *any,
            expr: Box::new(expand_expr(inner, values)?),
            pattern: Box::new(expand_expr(pattern, values)?),
            escape_char: escape_char.clone(),
        }),
        // Every other shape (a plain identifier, a literal, an already
        // validated function call whose args contain no placeholder —
        // validate_row_filter_expr's own allowlist guarantees nothing
        // else reaches here) is returned unchanged.
        _ => Ok(expr.clone()),
    }
}

fn expand_expr(expr: &Expr, values: &PlaceholderValues) -> Result<Expr, RewriteError> {
    match expr {
        Expr::Identifier(ident) if ident.value == PRINCIPAL_ID_PLACEHOLDER => {
            expand_scalar_placeholder(values)
        }
        Expr::InList {
            expr: inner,
            list,
            negated,
        } if is_sole_tenant_placeholder(list) => Ok(Expr::InList {
            expr: Box::new(expand_expr(inner, values)?),
            list: expand_tenant_list_placeholder(values),
            negated: *negated,
        }),
        other => expand_expr_rest(other, values),
    }
}

#[cfg(test)]
mod table_substitution {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use sqlparser::dialect::ClickHouseDialect;

    use super::{PlaceholderValues, RewriteError, TableObligations, substitute_governed_tables};

    fn obligations() -> HashMap<String, TableObligations> {
        HashMap::from([(
            "silver.customers".to_owned(),
            TableObligations {
                mask: vec!["email".to_owned()],
                row_filter: Some("tenant_id = 'tenant-a'".to_owned()),
                real_columns: Some(vec![
                    "id".to_owned(),
                    "email".to_owned(),
                    "tenant_id".to_owned(),
                ]),
            },
        )])
    }

    fn substituted(sql: &str) -> String {
        substitute_governed_tables(
            sql,
            &ClickHouseDialect {},
            &obligations(),
            &PlaceholderValues::none(),
        )
        .unwrap()
    }

    /// Every one of these asserts the SAME two structural facts: (1)
    /// the derived-table wrapper appears (proving substitution ran),
    /// and (2) the row filter is present — the governed table is read
    /// exactly once, through the wrapper, no matter how many syntactic
    /// positions referenced it.
    fn assert_only_wrapped_reads(sql: &str) {
        let out = substituted(sql);
        assert!(
            out.contains("replaceRegexpAll(toString(`email`), '.*', '***') AS `email`"),
            "{out}"
        );
        assert!(out.contains("WHERE tenant_id = 'tenant-a'"), "{out}");
    }

    #[test]
    fn where_oracle_reads_the_masked_projection_not_the_raw_column() {
        assert_only_wrapped_reads("SELECT count() FROM silver.customers WHERE email LIKE 'a%'");
    }

    #[test]
    fn group_by_and_order_by_over_the_masked_column_see_only_the_masked_value() {
        assert_only_wrapped_reads(
            "SELECT email, count() FROM silver.customers GROUP BY email ORDER BY email",
        );
    }

    #[test]
    fn columns_dynamic_selector_reads_from_the_substituted_table() {
        assert_only_wrapped_reads("SELECT COLUMNS('e.*') FROM silver.customers");
    }

    #[test]
    fn star_except_reads_from_the_substituted_table() {
        assert_only_wrapped_reads("SELECT * EXCEPT (id) FROM silver.customers");
    }

    #[test]
    fn qualified_star_reads_from_the_substituted_table() {
        assert_only_wrapped_reads("SELECT c.* FROM silver.customers c");
    }

    #[test]
    fn join_on_a_masked_column_compares_only_masked_values() {
        let out = substituted(
            "SELECT o.id FROM silver.orders_enriched o JOIN silver.customers c ON c.email = o.customer_email",
        );
        assert!(
            out.contains("replaceRegexpAll(toString(`email`), '.*', '***') AS `email`"),
            "{out}"
        );
    }

    #[test]
    fn a_self_join_substitutes_both_occurrences_independently() {
        let out = substituted(
            "SELECT a.id FROM silver.customers a JOIN silver.customers b ON a.email = b.email",
        );
        assert_eq!(
            out.matches("replaceRegexpAll(toString(`email`)").count(),
            2,
            "{out}"
        );
    }

    #[test]
    fn a_cte_over_the_table_is_substituted_inside_the_ctes_own_definition() {
        assert_only_wrapped_reads("WITH x AS (SELECT * FROM silver.customers) SELECT * FROM x");
    }

    #[test]
    fn each_union_branch_is_substituted_independently() {
        let mut obl = obligations();
        obl.insert(
            "silver.customers_archive".to_owned(),
            obl["silver.customers"].clone(),
        );
        let out = substitute_governed_tables(
            "SELECT email FROM silver.customers UNION ALL SELECT email FROM silver.customers_archive",
            &ClickHouseDialect {},
            &obl,
            &PlaceholderValues::none(),
        )
        .unwrap();
        assert_eq!(
            out.matches("replaceRegexpAll(toString(`email`)").count(),
            2,
            "{out}"
        );
    }

    #[test]
    fn a_lambda_body_reads_from_the_substituted_table() {
        assert_only_wrapped_reads("SELECT arrayMap(x -> x, [email]) FROM silver.customers");
    }

    #[test]
    fn a_derived_table_wrapping_the_governed_table_is_filtered_where_it_is_read() {
        // M2's own example: the filter column may not even be
        // projected by the OUTER query, so a top-level WHERE append
        // (the old design) could not have reached it at all.
        let out = substituted("SELECT * FROM (SELECT id FROM silver.customers) s");
        assert!(out.contains("WHERE tenant_id = 'tenant-a'"), "{out}");
    }

    #[test]
    fn the_right_side_of_a_left_join_is_filtered_inside_its_own_derived_table() {
        let out = substituted(
            "SELECT o.id, c.email FROM silver.orders_enriched o LEFT JOIN silver.customers c ON c.id = o.customer_id",
        );
        assert!(out.contains("WHERE tenant_id = 'tenant-a'"), "{out}");
    }

    #[test]
    fn a_where_clause_subquery_reading_the_governed_table_is_substituted() {
        // Hard Requirement 3: a subquery is a read site too, not only a
        // FROM/JOIN — this table is never named in the outer FROM at
        // all.
        let out = substituted(
            "SELECT id FROM silver.orders_enriched WHERE customer_id IN (SELECT id FROM silver.customers)",
        );
        assert!(out.contains("WHERE tenant_id = 'tenant-a'"), "{out}");
    }

    #[test]
    fn refuses_when_the_real_column_list_is_unknown() {
        let mut obl = obligations();
        obl.get_mut("silver.customers").unwrap().real_columns = None;
        let err = substitute_governed_tables(
            "SELECT * FROM silver.customers",
            &ClickHouseDialect {},
            &obl,
            &PlaceholderValues::none(),
        )
        .unwrap_err();
        assert!(matches!(err, RewriteError::UnprovableSubstitution { .. }));
    }
}

/// WS7 item B4's own executable specification for the row-filter grammar
/// introduced (out of strict task order — see WS7 item B3's commit
/// message) alongside table substitution: every bare identifier is a
/// real column or [`PRINCIPAL_ID_PLACEHOLDER`];
/// [`PRINCIPAL_TENANT_IDS_PLACEHOLDER`] is valid only as the sole `IN
/// (...)` element; a function call must be on
/// [`ALLOWED_ROW_FILTER_FUNCTIONS`]; a subquery/`EXISTS`/garbage is
/// refused; placeholder expansion re-serializes through `sqlparser`'s
/// own `Value::SingleQuotedString`, never a find-and-replace on the raw
/// authored string.
#[cfg(test)]
mod row_filter {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use sqlparser::ast::Expr;
    use sqlparser::parser::Parser;

    use super::{PlaceholderValues, RewriteError, expand_placeholders, validate_row_filter_expr};

    fn parse_test_expr(raw: &str) -> Expr {
        let dialect = sqlparser::dialect::GenericDialect {};
        Parser::new(&dialect)
            .try_with_sql(raw)
            .unwrap()
            .parse_expr()
            .unwrap()
    }

    #[test]
    fn validate_row_filter_expr_accepts_a_simple_comparison_against_real_columns() {
        let cols = vec!["tenant_id".to_owned(), "tahun".to_owned()];
        assert!(validate_row_filter_expr("tenant_id = 'tenant-a'", &cols).is_ok());
    }

    #[test]
    fn validate_row_filter_expr_rejects_an_unknown_column() {
        let cols = vec!["tenant_id".to_owned()];
        let err = validate_row_filter_expr("secret_col = 1", &cols).unwrap_err();
        assert!(matches!(err, RewriteError::InvalidRowFilter { .. }));
    }

    #[test]
    fn validate_row_filter_expr_rejects_a_function_call_not_on_the_allowlist() {
        let cols = vec!["tenant_id".to_owned()];
        let err = validate_row_filter_expr("tenant_id = (SELECT 1)", &cols).unwrap_err();
        assert!(matches!(err, RewriteError::InvalidRowFilter { .. }));
    }

    #[test]
    fn validate_row_filter_expr_rejects_garbage_that_is_not_an_expression_at_all() {
        let cols = vec!["tenant_id".to_owned()];
        assert!(validate_row_filter_expr("; DROP TABLE t; --", &cols).is_err());
    }

    #[test]
    fn validate_row_filter_expr_accepts_the_scalar_principal_placeholder_as_a_bare_identifier() {
        let cols = vec!["owner_id".to_owned()];
        assert!(validate_row_filter_expr("owner_id = __principal_id__", &cols).is_ok());
    }

    #[test]
    fn validate_row_filter_expr_accepts_the_list_placeholder_only_as_the_sole_in_list_element() {
        let cols = vec!["tenant_id".to_owned()];
        assert!(validate_row_filter_expr("tenant_id IN (__principal_tenant_ids__)", &cols).is_ok());
    }

    #[test]
    fn validate_row_filter_expr_rejects_the_list_placeholder_used_bare() {
        // Not inside an IN-list — nothing else could give a LIST
        // placeholder a well-defined scalar meaning, so this is
        // refused, not silently coerced to one element.
        let cols = vec!["tenant_id".to_owned()];
        let err =
            validate_row_filter_expr("tenant_id = __principal_tenant_ids__", &cols).unwrap_err();
        assert!(matches!(err, RewriteError::InvalidRowFilter { .. }));
    }

    #[test]
    fn validate_row_filter_expr_rejects_the_list_placeholder_alongside_other_in_list_elements() {
        let cols = vec!["tenant_id".to_owned()];
        let err = validate_row_filter_expr("tenant_id IN (__principal_tenant_ids__, 'x')", &cols)
            .unwrap_err();
        assert!(matches!(err, RewriteError::InvalidRowFilter { .. }));
    }

    #[test]
    fn expand_placeholders_renders_the_scalar_placeholder_through_sql_literal() {
        let expr = parse_test_expr("owner_id = __principal_id__");
        let values = PlaceholderValues {
            principal_id: Some("u-1".to_owned()),
            principal_tenant_ids: vec![],
        };
        let out = expand_placeholders(&expr, &values).unwrap();
        assert_eq!(out, "owner_id = 'u-1'");
    }

    #[test]
    fn expand_placeholders_renders_the_list_placeholder_as_one_literal_per_tenant() {
        let expr = parse_test_expr("tenant_id IN (__principal_tenant_ids__)");
        let values = PlaceholderValues {
            principal_id: None,
            principal_tenant_ids: vec!["t-1".to_owned(), "t-2".to_owned()],
        };
        let out = expand_placeholders(&expr, &values).unwrap();
        assert_eq!(out, "tenant_id IN ('t-1', 't-2')");
    }

    #[test]
    fn expand_placeholders_with_an_empty_tenant_list_produces_a_never_matching_in_list() {
        // `IN ()` is invalid SQL in most dialects — a principal with NO
        // tenant memberships renders `IN (NULL)` instead, which is
        // valid and matches nothing, rather than a broken statement or
        // (worse) an empty-list special case some engine optimizes to
        // "always true".
        let expr = parse_test_expr("tenant_id IN (__principal_tenant_ids__)");
        let values = PlaceholderValues {
            principal_id: None,
            principal_tenant_ids: vec![],
        };
        assert_eq!(
            expand_placeholders(&expr, &values).unwrap(),
            "tenant_id IN (NULL)"
        );
    }
}

/// Consumes WS7 item B1's `REFUSED_UNPARSEABLE` (populated with the real,
/// observed `record_m1_class_parse_results` output) to prove every
/// class recorded there is refused end to end through the real entry
/// point, never silently dropped from coverage or treated as touching
/// no governed table.
#[cfg(test)]
mod refuses_unparseable_shapes {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use sqlparser::dialect::ClickHouseDialect;

    use super::{PlaceholderValues, REFUSED_UNPARSEABLE, RewriteError, substitute_governed_tables};

    #[test]
    fn every_recorded_unparseable_class_is_refused_end_to_end() {
        for (label, sql) in REFUSED_UNPARSEABLE {
            let err = substitute_governed_tables(
                sql,
                &ClickHouseDialect {},
                &HashMap::new(),
                &PlaceholderValues::none(),
            )
            .unwrap_err();
            assert!(
                matches!(err, RewriteError::Unparseable),
                "label={label} sql={sql}"
            );
        }
    }
}

/// Real `ClickHouse` system table names that can leak information about
/// OTHER principals' activity (queries they ran, sessions, in-flight
/// processes) — reading these requires `audit:read`, unlike an
/// ordinary `system.*` catalog table (`system.columns`, `system.tables`)
/// which stays open to any authenticated caller.
const SENSITIVE_SYSTEM_TABLES: &[&str] = &[
    "system.query_log",
    "system.query_thread_log",
    "system.processes",
    "system.session_log",
    "system.text_log",
];

/// `ClickHouse` table functions this module allows in a governed
/// query — empty, deliberately. A table-function call reads data by a
/// mechanism table substitution cannot see or govern (`merge()` reads
/// by regex, `view()` embeds an arbitrary query, `url()`/`s3()`/
/// `remote()`/`input()`/... read data this module never resolves to a
/// canonical table name at all), so this is an ALLOWLIST that allows
/// none, not a denylist of the ones a reviewer happened to think of —
/// closing the whole class at once, including every name a future
/// `ClickHouse` release adds (a denylist needs updating for a new name;
/// an empty allowlist does not, by construction). See this module's
/// test `refuses_every_real_table_function_the_allowlist_does_not_name`
/// for the full, live-confirmed 88-name list this covers.
const ALLOWED_TABLE_FUNCTIONS: &[&str] = &[];

/// The `dictGet*`/`dictHas`/`dictIsIn`/`joinGet*` family, lower-cased.
/// A dictionary or `Join`-engine table's SOURCE can be a governed
/// table, but that source is never visible from the query text (it
/// lives in a separate `CREATE DICTIONARY`/`Join`-engine definition
/// this module has no introspection path for) — so this module cannot
/// prove any single call is safe, and refuses the WHOLE FAMILY whenever
/// the calling principal has ANY authored obligation anywhere (never
/// attempting to trace whether THIS PARTICULAR dictionary happens to be
/// backed by a governed table). A principal with no obligation anywhere
/// is unaffected — see [`is_dict_or_join_function`].
const DICT_JOIN_FUNCTIONS: &[&str] = &[
    "dictget",
    "dictgetall",
    "dictgetchildren",
    "dictgetdate",
    "dictgetdateordefault",
    "dictgetdatetime",
    "dictgetdatetimeordefault",
    "dictgetdescendants",
    "dictgetfloat32",
    "dictgetfloat32ordefault",
    "dictgetfloat64",
    "dictgetfloat64ordefault",
    "dictgethierarchy",
    "dictgetipv4",
    "dictgetipv4ordefault",
    "dictgetipv6",
    "dictgetipv6ordefault",
    "dictgetint16",
    "dictgetint16ordefault",
    "dictgetint32",
    "dictgetint32ordefault",
    "dictgetint64",
    "dictgetint64ordefault",
    "dictgetint8",
    "dictgetint8ordefault",
    "dictgetkeys",
    "dictgetordefault",
    "dictgetornull",
    "dictgetroot",
    "dictgetstring",
    "dictgetstringordefault",
    "dictgetuint16",
    "dictgetuint16ordefault",
    "dictgetuint32",
    "dictgetuint32ordefault",
    "dictgetuint64",
    "dictgetuint64ordefault",
    "dictgetuint8",
    "dictgetuint8ordefault",
    "dictgetuuid",
    "dictgetuuidordefault",
    "dicthas",
    "dictisin",
    "joinget",
    "joingetornull",
];

/// Whether `name` (any case) is in the `dictGet*`/`joinGet*` family —
/// the enumerated [`DICT_JOIN_FUNCTIONS`] list PLUS a `dictget`/
/// `joinget` prefix backstop, so a real variant this module's own
/// enumeration missed, or a future `ClickHouse` release's new
/// `dictGetX`, is still caught.
fn is_dict_or_join_function(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    DICT_JOIN_FUNCTIONS.contains(&lower.as_str())
        || lower.starts_with("dictget")
        || lower.starts_with("joinget")
}

fn canonical_object_name(name: &ObjectName) -> String {
    let parts: Vec<String> = name
        .0
        .iter()
        .filter_map(ObjectNamePart::as_ident)
        .map(|ident| ident.value.to_ascii_lowercase())
        .collect();
    canonicalize(&parts)
}

/// Read-only classification pass, run via `sqlparser`'s own derived
/// `Visit` traversal so every `TableFactor` and `Expr::Function`
/// anywhere in the statement is reached — including inside an
/// `EXPLAIN`'s wrapped statement (`Statement::Explain`'s own `Visit`
/// impl recurses into its boxed inner `Statement`, so no separate
/// unwrap step is needed here), a CTE body, or a subquery — without a
/// hand-written match over every nesting shape.
struct Classifier<'a> {
    permissions: &'a [String],
    has_any_obligation: bool,
}

impl Visitor for Classifier<'_> {
    type Break = RewriteError;

    fn pre_visit_table_factor(&mut self, table_factor: &TableFactor) -> ControlFlow<RewriteError> {
        let TableFactor::Table { name, args, .. } = table_factor else {
            return ControlFlow::Continue(());
        };
        if args.is_some() {
            let called = name.to_string();
            let lower = called.to_ascii_lowercase();
            if !ALLOWED_TABLE_FUNCTIONS.contains(&lower.as_str()) {
                return ControlFlow::Break(RewriteError::TableFunctionDenied { name: called });
            }
            return ControlFlow::Continue(());
        }
        let canonical = canonical_object_name(name);
        if SENSITIVE_SYSTEM_TABLES.contains(&canonical.as_str())
            && !self.permissions.iter().any(|p| p == "audit:read")
        {
            return ControlFlow::Break(RewriteError::SensitiveSystemTable { table: canonical });
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<RewriteError> {
        if let Expr::Function(f) = expr {
            let called = f.name.to_string();
            if self.has_any_obligation && is_dict_or_join_function(&called) {
                return ControlFlow::Break(RewriteError::DictOrJoinFunctionDenied { name: called });
            }
        }
        ControlFlow::Continue(())
    }
}

/// Classifies `sql` for a principal carrying `permissions`, refusing:
/// a table-function call not on the (empty) [`ALLOWED_TABLE_FUNCTIONS`]
/// allowlist; a [`SENSITIVE_SYSTEM_TABLES`] read without `audit:read`;
/// and, when `has_any_obligation` is `true`, any `dictGet*`/`joinGet*`
/// family call anywhere in the statement (see [`is_dict_or_join_function`]
/// and [`DICT_JOIN_FUNCTIONS`]'s own doc comment for why the whole
/// family, not just the tables this query's `FROM` names).
///
/// # Errors
/// [`RewriteError::Unparseable`] if `sql` does not parse;
/// [`RewriteError::TableFunctionDenied`], [`RewriteError::SensitiveSystemTable`],
/// or [`RewriteError::DictOrJoinFunctionDenied`] per the rules above.
pub fn classify_statement_for_principal(
    sql: &str,
    dialect: &dyn Dialect,
    permissions: &[String],
    has_any_obligation: bool,
) -> Result<(), RewriteError> {
    let statements = Parser::parse_sql(dialect, sql).map_err(|_| RewriteError::Unparseable)?;
    let mut classifier = Classifier {
        permissions,
        has_any_obligation,
    };
    for stmt in &statements {
        if let ControlFlow::Break(err) = stmt.visit(&mut classifier) {
            return Err(err);
        }
    }
    Ok(())
}

/// [`classify_statement_for_principal`] with no principal context — the
/// correct default for a caller with no `permissions`/obligation
/// information at all (`has_any_obligation: false` matches "a principal
/// with no obligation anywhere is unaffected").
///
/// # Errors
/// Same as [`classify_statement_for_principal`].
#[allow(
    dead_code,
    reason = "no non-test caller exists yet in this commit (WS7 item B5); \
              a future no-principal-context caller (or a test using it \
              directly) is the first production caller"
)]
pub fn classify_statement(sql: &str, dialect: &dyn Dialect) -> Result<(), RewriteError> {
    classify_statement_for_principal(sql, dialect, &[], false)
}

/// Resolves a table's engine and (for a view) its defining SQL, so
/// [`classify_views`] can tell a view reading a governed table from an
/// ordinary one. The real implementation (Phase C) queries `SELECT
/// engine, create_table_query FROM system.tables WHERE (database,
/// name) = (...)`; `NoViews` (WS7 item B6) implements this as "nothing is
/// ever a view", for a caller with no catalog access at all.
#[allow(
    dead_code,
    reason = "only a #[cfg(test)] implementor exists in this commit \
              (WS7 item B5's FakeSystemTablesCatalog); WS7 item B6's NoViews and \
              Phase C's real implementation are the first production callers"
)]
pub trait SystemTablesCatalog {
    /// `(engine, create_table_query)` for `table` (canonical
    /// `"schema.table"`), or `None` if the table is unknown to the
    /// catalog.
    fn engine_and_definition(&self, table: &str) -> Option<(String, Option<String>)>;
}

/// Refuses `sql` if it reads a view (`engine` `"View"` or
/// `"MaterializedView"`) whose OWN defining query touches any table in
/// `obligated_tables` — the view is never inlined or itself
/// substituted (WS7 plan, Hard Requirement 4): a view definition is
/// authored independently of this module's own obligations map, so
/// this module cannot prove the view's own `SELECT` doesn't re-expose
/// a masked column or bypass a row filter by construction; refusing the
/// whole query is the only fail-closed option.
///
/// # Errors
/// [`RewriteError::Unparseable`] if `sql` does not parse;
/// [`RewriteError::ViewOverGovernedTable`] if such a view is read.
#[allow(
    clippy::implicit_hasher,
    reason = "this crate never builds a HashSet<String> with a non-default \
              hasher; see the matching allow on substitute_governed_tables"
)]
#[allow(
    dead_code,
    reason = "no non-test caller exists yet in this commit (WS7 item B5); \
              WS7 item B6's enforce() is the first production caller"
)]
pub fn classify_views(
    sql: &str,
    dialect: &dyn Dialect,
    catalog: &dyn SystemTablesCatalog,
    obligated_tables: &HashSet<String>,
) -> Result<(), RewriteError> {
    let tables = referenced_tables(sql, dialect).ok_or(RewriteError::Unparseable)?;
    for table in &tables {
        let Some((engine, definition)) = catalog.engine_and_definition(table) else {
            continue;
        };
        if engine != "View" && engine != "MaterializedView" {
            continue;
        }
        let Some(def_sql) = definition else { continue };
        let Some(def_tables) = referenced_tables(&def_sql, dialect) else {
            continue;
        };
        if def_tables.iter().any(|t| obligated_tables.contains(t)) {
            return Err(RewriteError::ViewOverGovernedTable {
                view: table.clone(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod refusals {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use sqlparser::dialect::ClickHouseDialect;

    use super::{
        RewriteError, SystemTablesCatalog, classify_statement, classify_statement_for_principal,
        classify_views,
    };

    /// The full, real `ClickHouse` table-function name list — confirmed
    /// live (`SELECT name FROM system.table_functions ORDER BY name`)
    /// against a `ClickHouse` 26.7.3.19 instance, per the WS7 plan's own
    /// WS7 item B5 header. Every one of these must be refused since
    /// [`super::ALLOWED_TABLE_FUNCTIONS`] is empty.
    const ALL_TABLE_FUNCTIONS_CONFIRMED_LIVE: &[&str] = &[
        "SQLStandardValues",
        "arrowFlight",
        "arrowflight",
        "azureBlobStorage",
        "azureBlobStorageCluster",
        "cluster",
        "clusterAllReplicas",
        "cosn",
        "deltaLake",
        "deltaLakeAzure",
        "deltaLakeAzureCluster",
        "deltaLakeCluster",
        "deltaLakeLocal",
        "deltaLakeS3",
        "deltaLakeS3Cluster",
        "dictionary",
        "eval",
        "executable",
        "file",
        "fileCluster",
        "filesystem",
        "format",
        "fuzzJSON",
        "fuzzQuery",
        "gcs",
        "generateRandom",
        "generateSeries",
        "generate_series",
        "hdfs",
        "hdfsCluster",
        "hive",
        "hudi",
        "hudiCluster",
        "iceberg",
        "icebergAzure",
        "icebergAzureCluster",
        "icebergCluster",
        "icebergHDFS",
        "icebergHDFSCluster",
        "icebergLocal",
        "icebergLocalCluster",
        "icebergS3",
        "icebergS3Cluster",
        "input",
        "jdbc",
        "loop",
        "merge",
        "mergeTreeAnalyzeIndexes",
        "mergeTreeAnalyzeIndexesUUID",
        "mergeTreeIndex",
        "mergeTreeProjection",
        "mergeTreeTextIndex",
        "mongodb",
        "mysql",
        "null",
        "numbers",
        "numbers_mt",
        "odbc",
        "oss",
        "paimon",
        "paimonAzure",
        "paimonAzureCluster",
        "paimonCluster",
        "paimonHDFS",
        "paimonHDFSCluster",
        "paimonLocal",
        "paimonS3",
        "paimonS3Cluster",
        "postgresql",
        "primes",
        "prometheusQuery",
        "prometheusQueryRange",
        "redis",
        "remote",
        "remoteSecure",
        "s3",
        "s3Cluster",
        "sqlite",
        "timeSeriesData",
        "timeSeriesMetrics",
        "timeSeriesSamples",
        "timeSeriesSelector",
        "timeSeriesTags",
        "url",
        "urlCluster",
        "values",
        "view",
        "viewExplain",
        "viewIfPermitted",
        "ytsaurus",
        "zeros",
        "zeros_mt",
    ];

    /// The full, real `dictGet*`/`dictHas`/`dictIsIn`/`joinGet*` family
    /// — confirmed live against the same instance, per the WS7 plan's
    /// WS7 item B5 header.
    const DICT_JOIN_FUNCTIONS_CONFIRMED_LIVE: &[&str] = &[
        "dictGet",
        "dictGetAll",
        "dictGetChildren",
        "dictGetDate",
        "dictGetDateOrDefault",
        "dictGetDateTime",
        "dictGetDateTimeOrDefault",
        "dictGetDescendants",
        "dictGetFloat32",
        "dictGetFloat32OrDefault",
        "dictGetFloat64",
        "dictGetFloat64OrDefault",
        "dictGetHierarchy",
        "dictGetIPv4",
        "dictGetIPv4OrDefault",
        "dictGetIPv6",
        "dictGetIPv6OrDefault",
        "dictGetInt16",
        "dictGetInt16OrDefault",
        "dictGetInt32",
        "dictGetInt32OrDefault",
        "dictGetInt64",
        "dictGetInt64OrDefault",
        "dictGetInt8",
        "dictGetInt8OrDefault",
        "dictGetKeys",
        "dictGetOrDefault",
        "dictGetOrNull",
        "dictGetRoot",
        "dictGetString",
        "dictGetStringOrDefault",
        "dictGetUInt16",
        "dictGetUInt16OrDefault",
        "dictGetUInt32",
        "dictGetUInt32OrDefault",
        "dictGetUInt64",
        "dictGetUInt64OrDefault",
        "dictGetUInt8",
        "dictGetUInt8OrDefault",
        "dictGetUUID",
        "dictGetUUIDOrDefault",
        "dictHas",
        "dictIsIn",
        "joinGet",
        "joinGetOrNull",
    ];

    #[test]
    fn refuses_every_real_table_function_the_allowlist_does_not_name() {
        for name in ALL_TABLE_FUNCTIONS_CONFIRMED_LIVE {
            let sql = format!("SELECT * FROM {name}('arg1', 'arg2')");
            let err = classify_statement(&sql, &ClickHouseDialect {}).unwrap_err();
            assert!(
                matches!(err, RewriteError::TableFunctionDenied { .. }),
                "name={name}"
            );
        }
    }

    #[test]
    fn refuses_merge_which_reads_tables_by_regex() {
        let err = classify_statement(
            "SELECT * FROM merge('silver', '^customers')",
            &ClickHouseDialect {},
        )
        .unwrap_err();
        assert!(matches!(err, RewriteError::TableFunctionDenied { .. }));
    }

    #[test]
    fn refuses_view_which_embeds_an_arbitrary_query_as_a_table() {
        // sqlparser 0.62.0 does not parse a bare (unparenthesized)
        // SELECT as a table-function argument at all (verified: this
        // exact string fails with "Expected: ), found: email") -- a
        // plan-stale detail (the plan's own illustrative test expected
        // TableFunctionDenied), but the outcome Hard Requirement 2
        // actually cares about -- refused, never silently passed
        // through -- holds regardless of which error variant it is.
        let err = classify_statement(
            "SELECT * FROM view(SELECT email FROM silver.customers)",
            &ClickHouseDialect {},
        )
        .unwrap_err();
        assert!(matches!(err, RewriteError::Unparseable));
    }

    #[test]
    fn refuses_executable_which_runs_a_script() {
        let err = classify_statement(
            "SELECT * FROM executable('script.py', 'CSV', 'x Int32')",
            &ClickHouseDialect {},
        )
        .unwrap_err();
        assert!(matches!(err, RewriteError::TableFunctionDenied { .. }));
    }

    #[test]
    fn refuses_input_which_was_on_neither_the_old_denylist_nor_commonly_discussed() {
        let err = classify_statement("SELECT * FROM input('x Int32')", &ClickHouseDialect {})
            .unwrap_err();
        assert!(matches!(err, RewriteError::TableFunctionDenied { .. }));
    }

    #[test]
    fn refuses_query_log_without_audit_read() {
        let err = classify_statement_for_principal(
            "SELECT query FROM system.query_log",
            &ClickHouseDialect {},
            &["catalog:read".to_owned()],
            false,
        )
        .unwrap_err();
        assert!(matches!(err, RewriteError::SensitiveSystemTable { .. }));
    }

    #[test]
    fn allows_query_log_with_audit_read() {
        assert!(
            classify_statement_for_principal(
                "SELECT query FROM system.query_log",
                &ClickHouseDialect {},
                &["audit:read".to_owned()],
                false,
            )
            .is_ok()
        );
    }

    #[test]
    fn allows_ordinary_system_columns_without_audit_read() {
        assert!(
            classify_statement_for_principal(
                "SELECT name FROM system.columns",
                &ClickHouseDialect {},
                &[],
                false
            )
            .is_ok()
        );
    }

    #[test]
    fn explain_unwraps_to_the_inner_statement_for_classification() {
        let err = classify_statement(
            "EXPLAIN SELECT * FROM remote('h', 'db', 't')",
            &ClickHouseDialect {},
        )
        .unwrap_err();
        assert!(matches!(err, RewriteError::TableFunctionDenied { .. }));
    }

    #[test]
    fn refuses_dict_get_in_a_projection_when_the_principal_has_any_obligation() {
        let sql = "SELECT dictGet('db.users_dict', 'email', toUInt64(1)) AS e";
        let err =
            classify_statement_for_principal(sql, &ClickHouseDialect {}, &[], true).unwrap_err();
        assert!(matches!(err, RewriteError::DictOrJoinFunctionDenied { .. }));
    }

    #[test]
    fn allows_dict_get_when_the_principal_has_no_obligation_anywhere() {
        let sql = "SELECT dictGet('db.public_dict', 'label', toUInt64(1)) AS e";
        assert!(classify_statement_for_principal(sql, &ClickHouseDialect {}, &[], false).is_ok());
    }

    #[test]
    fn refuses_every_dict_get_variant_and_dict_has_dict_is_in_when_the_principal_has_any_obligation()
     {
        for fname in DICT_JOIN_FUNCTIONS_CONFIRMED_LIVE {
            let sql = format!("SELECT {fname}('db.d', 'k', toUInt64(1))");
            let err = classify_statement_for_principal(&sql, &ClickHouseDialect {}, &[], true)
                .unwrap_err();
            assert!(
                matches!(err, RewriteError::DictOrJoinFunctionDenied { .. }),
                "fname={fname}"
            );
        }
    }

    #[test]
    fn refuses_a_dict_get_prefixed_name_not_individually_enumerated() {
        let err = classify_statement_for_principal(
            "SELECT dictGetSomeFutureVariant('db.d', 'k', 1)",
            &ClickHouseDialect {},
            &[],
            true,
        )
        .unwrap_err();
        assert!(matches!(err, RewriteError::DictOrJoinFunctionDenied { .. }));
    }

    #[test]
    fn refuses_join_get_when_the_principal_has_any_obligation() {
        let err = classify_statement_for_principal(
            "SELECT joinGet('db.join_table', 'email', 1)",
            &ClickHouseDialect {},
            &[],
            true,
        )
        .unwrap_err();
        assert!(matches!(err, RewriteError::DictOrJoinFunctionDenied { .. }));
    }

    struct FakeSystemTablesCatalog {
        views: HashMap<String, String>,
    }

    impl FakeSystemTablesCatalog {
        fn new() -> Self {
            Self {
                views: HashMap::new(),
            }
        }

        fn with_view(mut self, name: &str, definition: &str) -> Self {
            self.views.insert(name.to_owned(), definition.to_owned());
            self
        }
    }

    impl SystemTablesCatalog for FakeSystemTablesCatalog {
        fn engine_and_definition(&self, table: &str) -> Option<(String, Option<String>)> {
            self.views
                .get(table)
                .map(|def| ("View".to_owned(), Some(def.clone())))
        }
    }

    #[test]
    fn refuses_a_view_whose_definition_reads_a_governed_table() {
        let catalog = FakeSystemTablesCatalog::new().with_view(
            "serving.v_customers",
            "SELECT id, email FROM silver.customers",
        );
        let obligated = std::collections::HashSet::from(["silver.customers".to_owned()]);
        let err = classify_views(
            "SELECT * FROM serving.v_customers",
            &ClickHouseDialect {},
            &catalog,
            &obligated,
        )
        .unwrap_err();
        assert!(matches!(err, RewriteError::ViewOverGovernedTable { .. }));
    }

    #[test]
    fn allows_a_view_whose_definition_reads_no_governed_table() {
        let catalog = FakeSystemTablesCatalog::new()
            .with_view("serving.v_public", "SELECT id FROM silver.orders_enriched");
        let obligated = std::collections::HashSet::from(["silver.customers".to_owned()]);
        assert!(
            classify_views(
                "SELECT * FROM serving.v_public",
                &ClickHouseDialect {},
                &catalog,
                &obligated,
            )
            .is_ok()
        );
    }
}

/// Supplies obligations for `(table, principal_roles)`. Implemented by
/// `policy_engine` in Phase C; kept as a trait so this module's own
/// tests run with zero database/`ClickHouse` dependency.
#[allow(
    dead_code,
    reason = "only a #[cfg(test)] implementor exists in this commit (WS7 \
              item B6's FakeObligations); Phase C's policy_engine-backed \
              implementation is the first production caller"
)]
pub trait ObligationsSource {
    /// The mask/row-filter obligations `table` carries for a principal
    /// holding `principal_roles`, or `None` if none apply.
    fn obligations_for(&self, table: &str, principal_roles: &[String]) -> Option<TableObligations>;
    /// N5: whether ANY authored policy names ANY role in
    /// `principal_roles`, for ANY table — not only the tables the
    /// CURRENT query references. Needed because a `dictGet`/`joinGet`
    /// call names no table syntactically at all; [`enforce`] cannot
    /// know whether the dictionary/join-table it reads is backed by a
    /// governed table, so it refuses the whole family whenever this is
    /// `true`, rather than attempting to trace an unreachable source.
    fn has_any_obligation(&self, principal_roles: &[String]) -> bool;
}

/// A [`SystemTablesCatalog`] that reports every table as "not a view" —
/// for a caller with no `system.tables` access at all (or a test that
/// does not exercise the view-refusal path).
pub struct NoViews;

impl SystemTablesCatalog for NoViews {
    fn engine_and_definition(&self, _table: &str) -> Option<(String, Option<String>)> {
        None
    }
}

/// The one entry point Phase C calls. Order: (1) classify (table
/// functions — an ALLOWLIST, N5 — sensitive `system.*`, and
/// `dictGet*`/`joinGet*` whenever `obligations_source.has_any_obligation`
/// is true, N5 — refuses before touching per-table obligations at all);
/// (2) resolve every referenced table via [`referenced_tables`], look
/// each up in `obligations_source`, and build the obligations map
/// [`substitute_governed_tables`] needs; (3) [`classify_views`] over the
/// SAME table list, refusing a view reading a governed table; (4)
/// [`substitute_governed_tables`], with `placeholders` expanded from the
/// real calling principal. `sql` is never returned unmodified after a
/// step that could not fully verify it — a query touching no governed
/// table and no refused construct passes through byte-for-byte
/// (`Statement::to_string()`'s own re-serialization), everything else
/// is rewritten or refused.
///
/// # Errors
/// See [`classify_statement_for_principal`], [`classify_views`], and
/// [`substitute_governed_tables`] — every error any of those three can
/// return, `enforce` can return.
#[allow(
    dead_code,
    reason = "no non-test caller exists yet in this commit (WS7 item B6); \
              Phase C wires this into Query Studio and the copilot's run_sql"
)]
pub fn enforce(
    sql: &str,
    dialect: &dyn Dialect,
    principal_roles: &[String],
    placeholders: &PlaceholderValues,
    obligations_source: &dyn ObligationsSource,
    views_catalog: &dyn SystemTablesCatalog,
) -> Result<String, RewriteError> {
    let has_any_obligation = obligations_source.has_any_obligation(principal_roles);
    classify_statement_for_principal(sql, dialect, principal_roles, has_any_obligation)?;
    let tables = referenced_tables(sql, dialect).ok_or(RewriteError::Unparseable)?;
    let mut obligations = HashMap::new();
    for table in &tables {
        if let Some(obl) = obligations_source.obligations_for(table, principal_roles) {
            obligations.insert(table.clone(), obl);
        }
    }
    let obligated: HashSet<String> = obligations.keys().cloned().collect();
    classify_views(sql, dialect, views_catalog, &obligated)?;
    substitute_governed_tables(sql, dialect, &obligations, placeholders)
}

#[cfg(test)]
mod enforce_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use sqlparser::dialect::ClickHouseDialect;

    use super::{
        NoViews, ObligationsSource, PlaceholderValues, RewriteError, TableObligations, enforce,
    };

    struct FakeObligations {
        any: bool,
    }

    impl ObligationsSource for FakeObligations {
        fn obligations_for(&self, table: &str, _roles: &[String]) -> Option<TableObligations> {
            (table == "silver.customers").then(|| TableObligations {
                mask: vec!["email".to_owned()],
                row_filter: None,
                real_columns: Some(vec!["id".to_owned(), "email".to_owned()]),
            })
        }

        fn has_any_obligation(&self, _roles: &[String]) -> bool {
            self.any
        }
    }

    #[test]
    fn a_query_touching_no_governed_table_passes_through_unchanged() {
        let src = FakeObligations { any: true };
        let out = enforce(
            "SELECT 1",
            &ClickHouseDialect {},
            &[],
            &PlaceholderValues::none(),
            &src,
            &NoViews,
        )
        .unwrap();
        assert_eq!(out, "SELECT 1");
    }

    #[test]
    fn a_query_touching_a_governed_table_is_substituted() {
        let src = FakeObligations { any: true };
        let out = enforce(
            "SELECT * FROM silver.customers",
            &ClickHouseDialect {},
            &[],
            &PlaceholderValues::none(),
            &src,
            &NoViews,
        )
        .unwrap();
        assert!(out.contains("replaceRegexpAll(toString(`email`)"));
    }

    #[test]
    fn a_table_function_is_refused_even_with_no_governed_table_involved() {
        let src = FakeObligations { any: false };
        assert!(
            enforce(
                "SELECT * FROM url('h', 'CSV')",
                &ClickHouseDialect {},
                &[],
                &PlaceholderValues::none(),
                &src,
                &NoViews,
            )
            .is_err()
        );
    }

    #[test]
    fn an_unparseable_statement_is_refused() {
        let src = FakeObligations { any: false };
        assert!(
            enforce(
                "SELECT ??? garbage",
                &ClickHouseDialect {},
                &[],
                &PlaceholderValues::none(),
                &src,
                &NoViews,
            )
            .is_err()
        );
    }

    #[test]
    fn n5_a_dict_get_call_is_refused_when_has_any_obligation_is_true_even_with_no_table_in_the_from_clause()
     {
        let src = FakeObligations { any: true };
        let err = enforce(
            "SELECT dictGet('db.d', 'email', toUInt64(1))",
            &ClickHouseDialect {},
            &[],
            &PlaceholderValues::none(),
            &src,
            &NoViews,
        )
        .unwrap_err();
        assert!(matches!(err, RewriteError::DictOrJoinFunctionDenied { .. }));
    }

    #[test]
    fn n5_a_dict_get_call_is_allowed_when_the_principal_has_no_obligation_anywhere() {
        let src = FakeObligations { any: false };
        assert!(
            enforce(
                "SELECT dictGet('db.d', 'label', toUInt64(1))",
                &ClickHouseDialect {},
                &[],
                &PlaceholderValues::none(),
                &src,
                &NoViews,
            )
            .is_ok()
        );
    }
}

#[cfg(test)]
mod parses_real_repository_query_shapes {
    use sqlparser::dialect::{ClickHouseDialect, GenericDialect};
    use sqlparser::parser::Parser;

    /// Every one of these strings is copied verbatim from a real call
    /// site in this repository. If `sqlparser` cannot parse one, that
    /// shape becomes a permanently refused construct (documented on
    /// [`super::REFUSED_UNPARSEABLE`] and covered by a regression test
    /// in the `refusals` module, WS7 item B5), never a silent
    /// pass-through — table substitution never runs on a statement
    /// that failed to parse at all.
    const CLICKHOUSE_SHAPES: &[&str] = &[
        // routes/ai/tools/data.rs, the dataset catalog UNION view.
        "SELECT slug,title,description,tier,table_name FROM lake.`bronze_meta.dataset_catalog` \
         UNION ALL SELECT slug,title,description,tier,table_name FROM lake.`bronze_meta_sec.dataset_catalog`",
        // routes/agent.rs schema_context's DESCRIBE call shape mirrored as a query.
        "SELECT name, type FROM system.columns WHERE database='serving'",
        // lakehouse-bi specs.rs's kpi_gci (backtick-qualified, two aggregates).
        "SELECT sum(data_tersedia) AS v, count() AS total FROM serving.mart_gci_readiness",
        // lakehouse-bi specs.rs's kpi_event (ORDER BY + LIMIT on an aggregate query).
        "SELECT tahun, count() AS n FROM serving.mart_event ORDER BY tahun DESC LIMIT 1",
        // gold_export.rs's select_projection DateTime cast; FORMAT JSON is
        // handled by WS7 item B5's pre-split, so the parser only ever sees this.
        "SELECT toString(toTimeZone(`created_at`, 'UTC')) AS `created_at`, `id` FROM silver.orders_enriched \
         ORDER BY `id` LIMIT 500 OFFSET 0",
        // query.rs test fixture `with x as (select 1) select * from x`.
        "with x as (select 1) select * from x",
        // A representative CTE + subquery + JOIN shape a real analyst would type.
        "WITH recent AS (SELECT id, tahun FROM silver.orders_enriched WHERE tahun >= 2024) \
         SELECT r.id, c.email FROM recent r JOIN silver.customers c ON c.id = r.id \
         WHERE r.id IN (SELECT id FROM silver.flagged)",
    ];

    /// The exact classes the WS7 plan's Hard Requirement 1 (M1) requires
    /// a named test for. Each is checked for parse success here (once,
    /// recorded) — the `table_substitution` module (WS7 item B3) then
    /// proves table substitution masks/filters correctly for every one
    /// that DOES parse; one that does NOT parse is covered instead by
    /// the `refusals` module's `refuses_unparseable_shapes` (WS7 item B5).
    const M1_CLASS_SHAPES: &[(&str, &str)] = &[
        (
            "where_oracle",
            "SELECT count() FROM silver.customers WHERE email LIKE 'a%'",
        ),
        (
            "group_by_order_by",
            "SELECT email, count() FROM silver.customers GROUP BY email ORDER BY email",
        ),
        (
            "columns_dynamic_selector",
            "SELECT COLUMNS('e.*') FROM silver.customers",
        ),
        ("star_except", "SELECT * EXCEPT (id) FROM silver.customers"),
        ("qualified_star", "SELECT c.* FROM silver.customers c"),
        (
            "join_on_masked_column",
            "SELECT o.id FROM silver.orders_enriched o JOIN silver.customers c ON c.email = o.email",
        ),
        (
            "self_join",
            "SELECT a.id FROM silver.customers a JOIN silver.customers b ON a.email = b.email",
        ),
        (
            "cte_over_table",
            "WITH x AS (SELECT * FROM silver.customers) SELECT * FROM x",
        ),
        (
            "union_branch",
            "SELECT email FROM silver.customers UNION ALL SELECT email FROM silver.customers_archive",
        ),
        (
            "lambda_over_column",
            "SELECT arrayMap(x -> x, [email]) FROM silver.customers",
        ),
        (
            "array_join",
            "SELECT arr FROM silver.tags_table ARRAY JOIN tags AS arr",
        ),
        (
            "with_scalar_alias",
            "WITH 2024 AS target_year SELECT * FROM silver.customers WHERE tahun = target_year",
        ),
    ];

    #[test]
    fn clickhouse_dialect_parses_every_real_shape() {
        for sql in CLICKHOUSE_SHAPES {
            let result = Parser::parse_sql(&ClickHouseDialect {}, sql);
            assert!(result.is_ok(), "failed to parse: {sql}\n{result:?}");
        }
    }

    /// Not an assertion of pass/fail either way for the M1 classes —
    /// this test's OUTPUT is read by the implementer and used to fill
    /// in `super::REFUSED_UNPARSEABLE`'s entries per WS7 item B1 Step 3. It
    /// always passes; it exists to make the finding reproducible in CI
    /// forever, not just once during planning.
    #[test]
    fn record_m1_class_parse_results() {
        for (label, sql) in M1_CLASS_SHAPES {
            let result = Parser::parse_sql(&ClickHouseDialect {}, sql);
            eprintln!(
                "{label}: {sql} => {}",
                if result.is_ok() { "PARSES" } else { "REJECTED" }
            );
        }
    }

    #[test]
    fn generic_dialect_parses_a_representative_trino_shape() {
        // Trino catalog.schema.table qualification (WS2's engine) — no
        // PrestoDialect exists in sqlparser 0.62 (verified: the
        // `sqlparser::dialect` module's exported dialect list has no
        // Presto/Trino entry), so this is GenericDialect's job.
        let sql = "SELECT o.id, o.email FROM iceberg.serving.mart_orders o WHERE o.tahun = 2024";
        assert!(Parser::parse_sql(&GenericDialect {}, sql).is_ok());
    }
}
