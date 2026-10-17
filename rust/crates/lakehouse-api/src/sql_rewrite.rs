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
///
/// A quoted part containing a dot is split first. `ClickHouse`'s
/// `DataLakeCatalog` engine exposes an Iceberg table as ONE identifier
/// named `namespace.table` inside a catalog database —
/// ``icecat_x.`bronze.orders` `` — which must reach the same key as the
/// same table read through Trino (`iceberg.bronze.orders`), or a policy
/// authored for `bronze.orders` would bind on one engine and silently not
/// on the other. Splitting can only ever make a name MORE likely to match
/// a governed key, never less: a governed table whose real columns then
/// cannot be resolved is refused (`UnprovableSubstitution`), not passed.
fn canonicalize(parts: &[String]) -> String {
    let parts: Vec<&str> = parts.iter().flat_map(|p| p.split('.')).collect();
    match parts.len() {
        0 => String::new(),
        1 => parts[0].to_owned(),
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

    /// A `DataLakeCatalog` read and a Trino read of the same Iceberg
    /// table must bind the same policy key.
    #[test]
    fn a_data_lake_catalog_table_canonicalizes_like_its_trino_name() {
        for sql in [
            "SELECT * FROM iceberg.bronze.orders",      // Trino
            "SELECT * FROM icecat_api.`bronze.orders`", // ClickHouse DataLakeCatalog
            "SELECT * FROM `icecat_api`.`bronze.orders`",
            "SELECT * FROM `bronze.orders`", // unqualified, current database
            "SELECT * FROM icecat_api.`Bronze.Orders`",
        ] {
            let tables = referenced_tables(sql, &ClickHouseDialect {}).unwrap();
            assert_eq!(tables, vec!["bronze.orders".to_owned()], "sql={sql}");
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
    BinaryOperator, Expr, Function, FunctionArg, FunctionArgExpr, FunctionArgumentList,
    FunctionArguments, Ident as SqlIdent, ObjectName, ObjectNamePart, SelectItem, SetExpr,
    Statement, TableAlias, UnaryOperator, Value, ValueWithSpan,
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
    /// principal context (WS7 item A5's policy-preview route, which has no
    /// real query-time principal to expand `__principal_id__`/
    /// `__principal_tenant_ids__` from, and tests).
    #[must_use]
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
    // `SEC` review finding from BI-18·A: the statement is re-serialised below
    // with `Display`, which is not a faithful encoder for a literal in a
    // backslash dialect (see `ReEncodeLiterals`). Re-encoding the literals
    // the caller wrote, before anything else is inserted, makes a literal's
    // value survive the rewrite unchanged.
    if dialect.supports_string_literal_backslash_escape() {
        for stmt in &mut statements {
            let _ = sqlparser::ast::VisitMut::visit(stmt, &mut ReEncodeLiterals);
        }
    }
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

/// Re-encodes every single-quoted literal of the parsed caller statement
/// into the form this dialect decodes back to the same value: `\\` for a
/// backslash and `\'` for a quote.
///
/// `Display` on a string literal is not an encoder for a backslash dialect:
/// it writes the decoded text back, doubling a quote only when it judges the
/// quote "not already escaped" (a quote after a backslash, or next to
/// another quote, is left alone), and never re-escapes a backslash. Handing
/// it text in which every quote is already preceded by its own backslash
/// makes that heuristic a no-op, so the output is exactly this encoding.
struct ReEncodeLiterals;

impl sqlparser::ast::VisitorMut for ReEncodeLiterals {
    type Break = ();

    fn post_visit_value(&mut self, value: &mut ValueWithSpan) -> std::ops::ControlFlow<()> {
        if let Value::SingleQuotedString(text) = &mut value.value
            && text.contains(['\\', '\''])
        {
            *text = text.replace('\\', "\\\\").replace('\'', "\\'");
        }
        std::ops::ControlFlow::Continue(())
    }
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

/// Alias for the derived subquery when the query wrote none: the table's
/// own last segment.
///
/// DATA-16 review BLOCKER 1: the segment of a `DataLakeCatalog` name is
/// one quoted identifier that contains a dot (`` `bronze.orders` ``), and
/// writing its lower-cased string unquoted produced `(SELECT ...)
/// bronze.orders`, which `ClickHouse` rejects. A plain word keeps the old
/// output byte for byte (lower-cased, unquoted); anything else keeps its
/// own value and quoting, and an unquoted non-word is quoted with the
/// dialect's identifier quote, falling back to a backtick (the one the
/// derived `SELECT` already uses) because `ClickHouseDialect` names none.
/// Only the alias changes; obligations are still looked up by the
/// canonical name.
fn derived_alias_ident(name: &ObjectName, dialect: &dyn Dialect) -> SqlIdent {
    let Some(last) = name.0.iter().rev().find_map(ObjectNamePart::as_ident) else {
        return SqlIdent::new(String::new());
    };
    let is_word = {
        let mut chars = last.value.chars();
        chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    if is_word {
        return SqlIdent::new(last.value.to_ascii_lowercase());
    }
    let quote = last.quote_style.unwrap_or_else(|| {
        dialect
            .identifier_quote_style(&last.value)
            .filter(|q| matches!(q, '"' | '`'))
            .unwrap_or('`')
    });
    SqlIdent::with_quote(quote, last.value.clone())
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
                name: derived_alias_ident(name, dialect),
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
/// wrapped in a `replaceRegexpOne(toString(...), '(?s)^.*$', '***') AS ...`
/// call: one anchored whole-value match, so a value becomes exactly
/// `***` (and `NULL` stays `NULL`). An earlier `replaceRegexpAll(..., '.*',
/// ...)` also matched the empty string after the value in `ClickHouse`
/// and produced `******`. This is the ONLY place a masked column's raw name and a row
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
///
/// A `mask` entry never reaches the SQL as text. It is compared with
/// `real_columns` (the table's own column names, each validated through
/// `Ident`), and only the matching real column name is written, so an entry
/// that is not a column adds nothing to the statement.
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
                "replaceRegexpOne(toString(`{safe}`), '(?s)^.*$', '***') AS `{safe}`"
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
/// The shape is deny by default for literals (SEC-32), and for calls,
/// operators and qualifiers (SEC-43). A call is its name and a plain list of
/// unnamed expressions, with no `FILTER`, `OVER`, `WITHIN GROUP`, second
/// parameter list, null treatment, `DISTINCT` or clause inside the
/// parentheses, or the ODBC `{fn ...}` form. A binary operator is a
/// comparison, `AND`, `OR` or one of `+ - * / %`; a unary one is `NOT`, `-`
/// or `+`. A column may have one unquoted table name before it. Before
/// SEC-43 the walk did not look at these parts of a call, at an operator, or
/// at the table name before a column.
///
/// A text value may not contain a backslash (SEC-32). This function parses
/// with `GenericDialect`, which does not read a backslash as an escape, while
/// `ClickHouse` reads `\'` as an escaped quote. The two parsers can then
/// disagree on where a string ends. For `x = '\' OR 1=1 --'` the policy
/// check sees the complete text `\`, then `OR 1=1`, then a `--` comment, so
/// the filter is `x = '\' OR 1=1`. The validated filter is written back as
/// `x = '\' OR 1 = 1`, and `ClickHouse` reads `\'` as an escaped quote and
/// finds an unterminated string. Today that ends in an error, which fails
/// closed. A crafted text could make both parsers accept the filter and read
/// it differently, so the backslash is refused instead of relied on to fail.
/// Only numbers, booleans, `NULL` and a plain single-quoted string are
/// literals here. The `R'..'`, `N'..'`, `X'..'`, `E'..'` and `$$..$$` forms
/// are refused too, because the two engines may read them differently.
///
/// The `ESCAPE` expression of `LIKE` and `ILIKE` is validated like any
/// other child expression, so a backslash text, a subquery or a function off
/// the allowlist is refused there too.
///
/// A unary `-` or `+` may not be followed by another `-` or `+`, and a text
/// value may not contain a control character (SEC-44): `Display` writes a
/// unary sign with no space before its operand, so the validated `x = - -1`
/// would print `x = --1` and `--` starts a SQL line comment in `ClickHouse`;
/// a control character inside a single-quoted string lets the text cross a
/// line, so what the validator checked and what `ClickHouse` reads can
/// differ. The two refusals share one goal: a checked filter must print as
/// a checked filter. `LIKE`/`ILIKE` with an `ANY` pattern is refused for
/// the same reason — the grammar admits many forms, only one pattern is
/// intended here.
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

/// Admits `NOT`, `-` and `+`; any other [`UnaryOperator`] is refused.
/// Then SEC-44: a `-` or `+` over a `-` or `+` is refused, because
/// `Display` writes a sign with no space before its operand, so the
/// validated `x = - -1` would print `x = --1` and `--` starts a SQL
/// line comment in `ClickHouse`. `NOT` writes with a space, so
/// `NOT -x` is safe. The split from [`validate_expr_shape`] keeps that
/// walk under the line-length budget.
fn validate_unary_op(
    op: UnaryOperator,
    inner: &Expr,
    real_columns: &[String],
) -> Result<(), RewriteError> {
    if !matches!(
        op,
        UnaryOperator::Not | UnaryOperator::Minus | UnaryOperator::Plus
    ) {
        return Err(RewriteError::InvalidRowFilter {
            reason: OPERATOR_REFUSAL.to_owned(),
        });
    }
    if matches!(op, UnaryOperator::Minus | UnaryOperator::Plus)
        && matches!(
            inner,
            Expr::UnaryOp {
                op: UnaryOperator::Minus | UnaryOperator::Plus,
                ..
            }
        )
    {
        return Err(RewriteError::InvalidRowFilter {
            reason: SIGN_REFUSAL.to_owned(),
        });
    }
    validate_expr_shape(inner, real_columns)
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
        Expr::CompoundIdentifier(parts) => validate_compound_identifier(parts, real_columns),
        Expr::Value(v) => validate_literal(&v.value),
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
        // SEC-43: the operator was ignored, so `OPERATOR(pg_catalog.+)`, `||`,
        // `XOR` and the bit and regex operators passed.
        Expr::BinaryOp { left, op, right } => {
            if !is_allowed_binary_operator(op) {
                return Err(RewriteError::InvalidRowFilter {
                    reason: OPERATOR_REFUSAL.to_owned(),
                });
            }
            validate_expr_shape(left, real_columns)?;
            validate_expr_shape(right, real_columns)
        }
        Expr::UnaryOp { op, expr: inner } => validate_unary_op(*op, inner.as_ref(), real_columns),
        Expr::Nested(inner) | Expr::IsNull(inner) | Expr::IsNotNull(inner) => {
            validate_expr_shape(inner, real_columns)
        }
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
            negated,
            any,
            escape_char,
        }
        | Expr::ILike {
            expr: inner,
            pattern,
            negated,
            any,
            escape_char,
        } => {
            // SEC-44: `LIKE ... ANY` (the optional `ANY` keyword right
            // after `LIKE`/`ILIKE`) admits a wider grammar than one
            // pattern; the only intended form here is the simple one.
            // Refused before the children are visited so a backslash text
            // or a real column inside the pattern does not leak through
            // when the whole node is the only rejection. Every field is
            // named (no `..`) so a field a later `sqlparser` adds stops
            // the build here until someone decides what it means.
            if *any {
                return Err(RewriteError::InvalidRowFilter {
                    reason: LIKE_ANY_REFUSAL.to_owned(),
                });
            }
            // `negated` is the `NOT` flag — checked explicitly to keep
            // the unused-binding lint quiet while the field is named
            // here for completeness. Today every value is fine; SEC-44
            // changes `any`, not `negated`.
            let _: &bool = negated;
            validate_expr_shape(inner, real_columns)?;
            validate_expr_shape(pattern, real_columns)?;
            // SEC-32 review fix (BLOCKER): the escape is an expression too.
            // Unvisited, it let a backslash text, a subquery or a function
            // off the allowlist pass.
            if let Some(escape) = escape_char {
                validate_expr_shape(escape, real_columns)?;
            }
            Ok(())
        }
        Expr::Function(f) => validate_function_call(f, real_columns),
        _ => Err(RewriteError::InvalidRowFilter {
            reason: format!("unsupported row-filter expression: {expr}"),
        }),
    }
}

/// The binary operators a row filter may use (SEC-43): comparisons, `AND`,
/// `OR` and the arithmetic of `+ - * / %`. Every other variant is refused,
/// including the ones a later `sqlparser` adds.
fn is_allowed_binary_operator(op: &BinaryOperator) -> bool {
    matches!(
        op,
        BinaryOperator::Eq
            | BinaryOperator::NotEq
            | BinaryOperator::Lt
            | BinaryOperator::LtEq
            | BinaryOperator::Gt
            | BinaryOperator::GtEq
            | BinaryOperator::And
            | BinaryOperator::Or
            | BinaryOperator::Plus
            | BinaryOperator::Minus
            | BinaryOperator::Multiply
            | BinaryOperator::Divide
            | BinaryOperator::Modulo
    )
}

/// A column, with at most one plain table name before it (SEC-43). Only the
/// last part was checked, and every part is written back as it came, so a
/// quoted qualifier such as `"a\".x` reached `ClickHouse` unchecked. The
/// qualifier only has to be a plain name; it is not compared with the
/// table's name.
fn validate_compound_identifier(
    parts: &[SqlIdent],
    real_columns: &[String],
) -> Result<(), RewriteError> {
    match parts {
        [] => Err(RewriteError::InvalidRowFilter {
            reason: "empty compound identifier".to_owned(),
        }),
        [column] => validate_identifier(&column.value, real_columns),
        [qualifier, column] if is_plain_qualifier(qualifier) => {
            validate_identifier(&column.value, real_columns)
        }
        _ => Err(RewriteError::InvalidRowFilter {
            reason: QUALIFIER_REFUSAL.to_owned(),
        }),
    }
}

const PLAIN_CALL_REFUSAL: &str = "a function call in a row filter takes plain arguments only";
const OPERATOR_REFUSAL: &str = "operator not allowed in a row filter";
const QUALIFIER_REFUSAL: &str = "a column in a row filter may have one plain table name before it";
// SEC-44 refusals: the validator refuses shapes that print differently
// than checked (a unary sign produces `--`/`-` etc. with no space, a
// control character lets string text cross a line, `LIKE ... ANY` opens
// a wider grammar). Fixed texts so the policy engine can map every
// `InvalidRowFilter` to its own caller's message without echoing the
// filter.
const SIGN_REFUSAL: &str = "a row filter may not put two signs in a row";
const CONTROL_CHAR_REFUSAL: &str = "a text value in a row filter must be on one line";
const LIKE_ANY_REFUSAL: &str = "a row filter accepts LIKE with one pattern only";

/// Admits numbers, booleans, `NULL` and a plain single-quoted string with
/// no backslash in its text. Every other [`Value`] variant is refused, so a
/// variant a later `sqlparser` adds is refused too (SEC-32). The reasons
/// are fixed texts: a refusal never echoes the literal.
fn validate_literal(value: &Value) -> Result<(), RewriteError> {
    match value {
        Value::Number(..) | Value::Boolean(_) | Value::Null => Ok(()),
        Value::SingleQuotedString(text) => {
            if text.contains('\\') {
                Err(RewriteError::InvalidRowFilter {
                    reason: "a text value in a row filter cannot contain a backslash".to_owned(),
                })
            } else if text.chars().any(|c| c < '\u{20}' || c == '\u{7f}') {
                // SEC-44: a control character (U+0000–U+001F here, plus
                // DEL at U+007F) inside a single-quoted string lets the
                // text cross a line — what the validator read and what
                // `ClickHouse` reads can then differ. `char::is_control`
                // covers U+0080–U+009F too; that range is plain ASCII
                // text to `ClickHouse` and the plan's decision is the
                // narrow set, so the comparison is written by hand.
                Err(RewriteError::InvalidRowFilter {
                    reason: CONTROL_CHAR_REFUSAL.to_owned(),
                })
            } else {
                Ok(())
            }
        }
        _ => Err(RewriteError::InvalidRowFilter {
            reason: "a row filter accepts plain 'text' values only".to_owned(),
        }),
    }
}

/// Admits a call to a function on [`ALLOWED_ROW_FILTER_FUNCTIONS`] and
/// nothing else about it (SEC-43). `Function` and `FunctionArgumentList` are
/// destructured with every field named and no `..`, so a field a later
/// `sqlparser` adds stops the build here until someone decides what it
/// means. Before, only `args` was visited: `FILTER (WHERE ...)` with a
/// subquery, `OVER (...)`, `WITHIN GROUP`, a clause inside the parentheses,
/// a second parameter list, `DISTINCT`, a null treatment and the ODBC
/// `{fn ...}` form all passed unseen. The reasons are fixed texts, so a
/// refusal never repeats the filter.
fn validate_function_call(f: &Function, real_columns: &[String]) -> Result<(), RewriteError> {
    let Function {
        name,
        uses_odbc_syntax,
        parameters,
        args,
        within_group,
        filter,
        null_treatment,
        over,
    } = f;
    let name = name.to_string().to_ascii_lowercase();
    if !ALLOWED_ROW_FILTER_FUNCTIONS.contains(&name.as_str()) {
        return Err(RewriteError::InvalidRowFilter {
            reason: format!("function '{name}' is not allowed in a row filter"),
        });
    }
    let refuse = || RewriteError::InvalidRowFilter {
        reason: PLAIN_CALL_REFUSAL.to_owned(),
    };
    if *uses_odbc_syntax
        || !matches!(parameters, FunctionArguments::None)
        || !within_group.is_empty()
        || filter.is_some()
        || null_treatment.is_some()
        || over.is_some()
    {
        return Err(refuse());
    }
    // A subquery passed as the sole argument without parentheses is refused
    // here with the rest of the non-list forms.
    let FunctionArguments::List(list) = args else {
        return Err(refuse());
    };
    let FunctionArgumentList {
        duplicate_treatment,
        args,
        clauses,
    } = list;
    if duplicate_treatment.is_some() || !clauses.is_empty() {
        return Err(refuse());
    }
    for arg in args {
        match arg {
            FunctionArg::Unnamed(FunctionArgExpr::Expr(e)) => {
                validate_expr_shape(e, real_columns)?;
            }
            // None of the four allowed functions takes a named argument or
            // a wildcard.
            FunctionArg::Named { .. }
            | FunctionArg::ExprNamed { .. }
            | FunctionArg::Unnamed(
                FunctionArgExpr::QualifiedWildcard(_)
                | FunctionArgExpr::Wildcard
                | FunctionArgExpr::WildcardWithOptions(_),
            ) => return Err(refuse()),
        }
    }
    Ok(())
}

/// One unquoted table name made of ASCII letters, digits and `_`, not
/// starting with a digit. The qualifier is written back verbatim, so
/// anything else (a quote, a backslash, a space, a non-ASCII letter) may be
/// read differently by `ClickHouse` than by the parser used here.
fn is_plain_qualifier(ident: &SqlIdent) -> bool {
    let mut chars = ident.value.chars();
    ident.quote_style.is_none()
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
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
            escape_char: expand_escape(escape_char.as_deref(), values)?,
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
            escape_char: expand_escape(escape_char.as_deref(), values)?,
        }),
        // Every other shape (a plain identifier, a literal, an already
        // validated function call whose args contain no placeholder —
        // validate_row_filter_expr's own allowlist guarantees nothing
        // else reaches here) is returned unchanged.
        _ => Ok(expr.clone()),
    }
}

fn expand_escape(
    escape: Option<&Expr>,
    values: &PlaceholderValues,
) -> Result<Option<Box<Expr>>, RewriteError> {
    escape
        .map(|e| expand_expr(e, values).map(Box::new))
        .transpose()
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
    use sqlparser::parser::Parser;

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
            out.contains("replaceRegexpOne(toString(`email`), '(?s)^.*$', '***') AS `email`"),
            "{out}"
        );
        assert!(out.contains("WHERE tenant_id = 'tenant-a'"), "{out}");
    }

    /// DATA-16 review BLOCKER 1: a `DataLakeCatalog` name has one quoted,
    /// dotted segment, and the derived alias must keep its quoting or
    /// `ClickHouse` rejects the rewrite. Each output is re-parsed so an
    /// unparseable rewrite cannot pass.
    #[test]
    fn data_lake_catalog_table_gets_a_quoted_alias_that_parses_again() {
        let obl = HashMap::from([(
            "bronze.orders".to_owned(),
            TableObligations {
                mask: vec!["email".to_owned()],
                row_filter: Some("id <= 4".to_owned()),
                real_columns: Some(vec!["id".to_owned(), "email".to_owned()]),
            },
        )]);
        let run = |sql: &str| {
            let out = substitute_governed_tables(
                sql,
                &ClickHouseDialect {},
                &obl,
                &PlaceholderValues::none(),
            )
            .unwrap();
            Parser::parse_sql(&ClickHouseDialect {}, &out)
                .unwrap_or_else(|e| panic!("rewrite does not parse ({e}): {out}"));
            out
        };

        // A derived alias the rewrite invents has `explicit: false`, which
        // sqlparser prints without `AS`; only a user-written `AS o` keeps it.
        let no_alias = run("SELECT count() n FROM icecat_api.`bronze.orders` WHERE 1");
        assert!(no_alias.contains(") `bronze.orders` WHERE 1"), "{no_alias}");

        let aliased = run("SELECT o.id FROM icecat_api.`bronze.orders` AS o WHERE 1");
        assert!(aliased.contains(") AS o WHERE 1"), "{aliased}");

        let pinned = run("SELECT count() n FROM icecat_api.`bronze.orders` WHERE 1 \
             SETTINGS iceberg_snapshot_id = 1");
        assert!(pinned.contains(") `bronze.orders` WHERE 1"), "{pinned}");
        assert!(
            pinned.ends_with("SETTINGS iceberg_snapshot_id = 1"),
            "{pinned}"
        );

        let plain = run_plain("SELECT * FROM silver.customers");
        assert!(plain.contains(") customers"), "{plain}");
    }

    fn run_plain(sql: &str) -> String {
        let out = substituted(sql);
        Parser::parse_sql(&ClickHouseDialect {}, &out).unwrap();
        out
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
            out.contains("replaceRegexpOne(toString(`email`), '(?s)^.*$', '***') AS `email`"),
            "{out}"
        );
    }

    #[test]
    fn a_self_join_substitutes_both_occurrences_independently() {
        let out = substituted(
            "SELECT a.id FROM silver.customers a JOIN silver.customers b ON a.email = b.email",
        );
        assert_eq!(
            out.matches("replaceRegexpOne(toString(`email`)").count(),
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
            out.matches("replaceRegexpOne(toString(`email`)").count(),
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

    use sqlparser::ast::{Expr, Ident as SqlIdent, ObjectName, ObjectNamePart};
    use sqlparser::parser::Parser;

    use super::{
        CONTROL_CHAR_REFUSAL, LIKE_ANY_REFUSAL, PlaceholderValues, RewriteError, SIGN_REFUSAL,
        build_masked_filtered_select, expand_placeholders, validate_expr_shape,
        validate_row_filter_expr,
    };

    const BACKSLASH_REASON: &str = "a text value in a row filter cannot contain a backslash";
    const PLAIN_TEXT_REASON: &str = "a row filter accepts plain 'text' values only";
    const PLAIN_CALL_REASON: &str = "a function call in a row filter takes plain arguments only";
    const OPERATOR_REASON: &str = "operator not allowed in a row filter";
    const QUALIFIER_REASON: &str =
        "a column in a row filter may have one plain table name before it";
    /// A word every refused filter below carries somewhere (in a literal, a
    /// qualifier or a name), so the helper can prove a refusal reason never
    /// echoes the filter text.
    const MARKER: &str = "zebra";

    fn invalid_reason(raw: &str, cols: &[String]) -> String {
        match validate_row_filter_expr(raw, cols).unwrap_err() {
            RewriteError::InvalidRowFilter { reason } => reason,
            other => panic!("expected InvalidRowFilter, got {other:?}"),
        }
    }

    fn table_name() -> ObjectName {
        ObjectName(vec![ObjectNamePart::Identifier(SqlIdent::new("t"))])
    }

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

    // SEC-32: the policy check parses a filter with `GenericDialect`, which
    // does not read a backslash as an escape, while `ClickHouse` does. The
    // two parsers can disagree on where a string ends, so a text value with
    // a backslash is refused rather than re-encoded.
    #[test]
    fn validate_row_filter_expr_refuses_a_text_value_with_a_backslash() {
        let cols = vec!["x".to_owned()];
        assert_eq!(invalid_reason(r"x = 'a\\b'", &cols), BACKSLASH_REASON);
    }

    #[test]
    fn a_filter_that_ends_a_string_with_a_backslash_is_refused_and_produces_no_sql() {
        let cols = vec!["x".to_owned()];
        let raw = r"x = '\' OR 1=1 --'";
        assert_eq!(invalid_reason(raw, &cols), BACKSLASH_REASON);
        let built = build_masked_filtered_select(
            &table_name(),
            &cols,
            &[],
            Some(raw),
            &PlaceholderValues::none(),
        );
        assert!(matches!(built, Err(RewriteError::InvalidRowFilter { .. })));
    }

    #[test]
    fn validate_row_filter_expr_refuses_every_text_form_but_the_plain_single_quoted_one() {
        let cols = vec!["x".to_owned()];
        // All five parse under `GenericDialect` today, so each reaches the
        // literal rule rather than being refused by the parse.
        for raw in ["x = R'a'", "x = N'a'", "x = X'61'", "x = E'a'", "x = $$a$$"] {
            assert_eq!(invalid_reason(raw, &cols), PLAIN_TEXT_REASON, "{raw}");
        }
    }

    // SEC-32 review fix (BLOCKER): `LIKE ... ESCAPE <expr>` carries a child
    // expression in `escape_char`. The shape walk used to skip it, so a
    // backslash text, a subquery or an unknown column passed in that slot.
    #[test]
    fn a_like_escape_with_a_backslash_is_refused() {
        let cols = vec!["x".to_owned()];
        assert_eq!(
            invalid_reason(r"x LIKE 'a' ESCAPE '\'", &cols),
            BACKSLASH_REASON
        );
        assert_eq!(
            invalid_reason(r"x NOT LIKE 'a' ESCAPE '\'", &cols),
            BACKSLASH_REASON
        );
    }

    #[test]
    fn an_ilike_escape_with_a_backslash_is_refused() {
        let cols = vec!["x".to_owned()];
        assert_eq!(
            invalid_reason(r"x ILIKE 'a' ESCAPE '\'", &cols),
            BACKSLASH_REASON
        );
    }

    #[test]
    fn a_second_string_opened_after_a_backslash_escape_is_refused() {
        // The policy check reads a `LIKE` with a one-backslash escape and a
        // comparison against a long text. `ClickHouse` reads `\'` as an
        // escaped quote, so the string boundaries differ. The backslash in
        // the escape is the refusal reason.
        let cols = vec!["x".to_owned()];
        let raw = r"x LIKE 'a' ESCAPE '\' AND x = ' OR 1=1 OR x = '";
        assert_eq!(invalid_reason(raw, &cols), BACKSLASH_REASON);
        let built = build_masked_filtered_select(
            &table_name(),
            &cols,
            &[],
            Some(raw),
            &PlaceholderValues::none(),
        );
        assert!(matches!(built, Err(RewriteError::InvalidRowFilter { .. })));
    }

    #[test]
    fn a_like_escape_with_a_plain_character_still_passes_and_expands_unchanged() {
        let cols = vec!["x".to_owned()];
        for raw in [
            "x LIKE 'a' ESCAPE '!'",
            "x ILIKE 'a' ESCAPE '!'",
            "x NOT LIKE 'a%' ESCAPE '!'",
            "x LIKE 'a'",
        ] {
            let expr = validate_row_filter_expr(raw, &cols)
                .unwrap_or_else(|e| panic!("{raw} was refused: {e:?}"));
            assert_eq!(
                expand_placeholders(&expr, &PlaceholderValues::none()).unwrap(),
                raw
            );
        }
    }

    #[test]
    fn a_like_escape_holding_a_subquery_or_an_unknown_column_is_refused() {
        let cols = vec!["x".to_owned()];
        let reason = invalid_reason("x LIKE 'a' ESCAPE (SELECT 1)", &cols);
        assert!(
            reason.starts_with("unsupported row-filter expression"),
            "{reason}"
        );
        let reason = invalid_reason("x ILIKE 'a' ESCAPE (SELECT 1)", &cols);
        assert!(
            reason.starts_with("unsupported row-filter expression"),
            "{reason}"
        );
        let reason = invalid_reason("x LIKE 'a' ESCAPE secret_col", &cols);
        assert!(reason.contains("is not a real column"), "{reason}");
        let reason = invalid_reason("x LIKE 'a' ESCAPE sleep(1)", &cols);
        assert!(
            reason.contains("is not allowed in a row filter"),
            "{reason}"
        );
    }

    #[test]
    fn a_like_escape_placeholder_is_expanded_not_left_as_an_identifier() {
        let cols = vec!["x".to_owned()];
        let values = PlaceholderValues {
            principal_id: Some("u-1".to_owned()),
            principal_tenant_ids: vec![],
        };
        let expr = validate_row_filter_expr("x LIKE 'a' ESCAPE __principal_id__", &cols).unwrap();
        assert_eq!(
            expand_placeholders(&expr, &values).unwrap(),
            "x LIKE 'a' ESCAPE 'u-1'"
        );
    }

    #[test]
    fn a_refusal_does_not_echo_the_literal() {
        let cols = vec!["x".to_owned()];
        let reason = invalid_reason(r"x = 'secret\\value'", &cols);
        assert!(!reason.contains("secret"));
        let reason = invalid_reason("x = N'secret'", &cols);
        assert!(!reason.contains("secret"));
    }

    #[test]
    fn a_plain_text_number_boolean_and_null_filter_still_passes_and_expands_unchanged() {
        let cols = vec!["x".to_owned(), "flag".to_owned()];
        let values = PlaceholderValues {
            principal_id: Some("u-1".to_owned()),
            principal_tenant_ids: vec![],
        };
        for (raw, expected) in [
            ("x = 'plain'", "x = 'plain'"),
            ("x = 'it''s'", "x = 'it''s'"),
            ("x = 42", "x = 42"),
            ("x > 1.5", "x > 1.5"),
            ("flag = true", "flag = true"),
            ("x IS NULL", "x IS NULL"),
            ("x = NULL", "x = NULL"),
            ("x IN ('a', 'b')", "x IN ('a', 'b')"),
            ("x = __principal_id__", "x = 'u-1'"),
        ] {
            let expr = validate_row_filter_expr(raw, &cols)
                .unwrap_or_else(|e| panic!("{raw} was refused: {e:?}"));
            assert_eq!(
                expand_placeholders(&expr, &values).unwrap(),
                expected,
                "{raw}"
            );
        }
    }

    // Regression: passes on `main` already. A mask entry is compared with the
    // table's real columns only and never reaches the SQL as text.
    #[test]
    fn a_mask_entry_that_is_not_a_column_adds_no_text_to_the_statement() {
        let cols = vec!["x".to_owned(), "y".to_owned()];
        let sql = build_masked_filtered_select(
            &table_name(),
            &cols,
            &["x') OR 1=1 --".to_owned()],
            None,
            &PlaceholderValues::none(),
        )
        .unwrap();
        assert_eq!(sql, "SELECT `x`, `y` FROM t");
    }

    /// Asserts that `raw` is refused with exactly `expected` and that the
    /// reason does not repeat any part of the filter.
    fn assert_refused(raw: &str, expected: &str) {
        assert!(raw.contains(MARKER), "{raw} carries no marker word");
        let cols = vec!["x".to_owned(), "y".to_owned(), "t".to_owned()];
        let reason = invalid_reason(raw, &cols);
        assert_eq!(reason, expected, "{raw}");
        assert!(!reason.contains(MARKER), "{raw}: reason echoes the filter");
    }

    // SEC-43 (follow-up of SEC-32): a function call is checked in every
    // part. Each test below puts one part of `Function` outside the plain
    // shape, with the same allowlisted name and a valid argument, so the
    // only thing wrong with the call is that part.
    #[test]
    fn a_call_with_a_filter_clause_is_refused() {
        assert_refused(
            "lower(x) FILTER (WHERE x = 'zebra') = 'a'",
            PLAIN_CALL_REASON,
        );
        // The subquery inside `FILTER` is never reached: the clause itself
        // is refused first, with the same fixed text.
        assert_refused(
            "lower(x) FILTER (WHERE x IN (SELECT y FROM zebra)) = 'a'",
            PLAIN_CALL_REASON,
        );
    }

    #[test]
    fn a_call_with_an_over_clause_is_refused() {
        assert_refused(
            "lower(x) OVER (PARTITION BY x) = 'zebra'",
            PLAIN_CALL_REASON,
        );
    }

    #[test]
    fn a_call_with_a_within_group_clause_is_refused() {
        assert_refused(
            "lower(x) WITHIN GROUP (ORDER BY x) = 'zebra'",
            PLAIN_CALL_REASON,
        );
    }

    #[test]
    fn a_call_with_a_null_treatment_after_the_arguments_is_refused() {
        assert_refused("lower(x) IGNORE NULLS = 'zebra'", PLAIN_CALL_REASON);
    }

    #[test]
    fn a_call_with_a_second_parameter_list_is_refused() {
        assert_refused("lower(1)(x) = 'zebra'", PLAIN_CALL_REASON);
    }

    #[test]
    fn a_call_in_the_odbc_form_is_refused() {
        assert_refused("{fn lower(x)} = 'zebra'", PLAIN_CALL_REASON);
    }

    #[test]
    fn a_call_with_a_named_argument_is_refused() {
        assert_refused("lower(zebra => x) = 'a'", PLAIN_CALL_REASON);
        assert_refused("lower(zebra := x) = 'a'", PLAIN_CALL_REASON);
    }

    #[test]
    fn a_call_with_a_duplicate_treatment_is_refused() {
        assert_refused("lower(DISTINCT x) = 'zebra'", PLAIN_CALL_REASON);
        assert_refused("lower(ALL x) = 'zebra'", PLAIN_CALL_REASON);
    }

    #[test]
    fn a_call_with_a_clause_inside_the_parentheses_is_refused() {
        for raw in [
            "lower(x ORDER BY x) = 'zebra'",
            "lower(x LIMIT 1) = 'zebra'",
            "lower(x IGNORE NULLS) = 'zebra'",
            "lower(x WHERE x = 'zebra') = 'a'",
        ] {
            assert_refused(raw, PLAIN_CALL_REASON);
        }
    }

    #[test]
    fn a_call_with_a_wildcard_argument_is_refused_without_echoing_the_filter() {
        // `lower(*)` was already refused; the reason now is the same fixed
        // text as for every other argument that is not a plain expression.
        assert_refused("lower(*) = 'zebra'", PLAIN_CALL_REASON);
        assert_refused("lower(zebra.*) = 'a'", PLAIN_CALL_REASON);
    }

    // SEC-43: a binary operator is checked against a fixed list. These
    // parse under `GenericDialect`; `x # 1` and `x DIV 2` do not parse, so
    // the parser already refuses them.
    #[test]
    fn a_custom_operator_is_refused() {
        assert_refused(
            "x = 1 OPERATOR(pg_catalog.+) 2 AND x = 'zebra'",
            OPERATOR_REASON,
        );
    }

    #[test]
    fn an_operator_outside_the_fixed_list_is_refused() {
        for raw in [
            "x || 'zebra' = 'b'",
            "x XOR 'zebra'",
            "x & 1 = 0 OR x = 'zebra'",
            "x ^ 1 = 0 OR x = 'zebra'",
            "x <=> 'zebra'",
            "x ~ 'zebra'",
            "x @> 'zebra'",
            "x -> 'zebra'",
        ] {
            assert_refused(raw, OPERATOR_REASON);
        }
    }

    #[test]
    fn a_unary_operator_outside_not_minus_and_plus_is_refused() {
        assert_refused("~x = 'zebra'", OPERATOR_REASON);
    }

    // SEC-43: a column may carry one plain table name before it. `!x`, `x!`,
    // `|/x`, `[t].x` and `1t.x` do not reach the shape walk: the parser
    // refuses them already.
    #[test]
    fn a_quoted_qualifier_is_refused() {
        assert_refused("\"zebra\\\".x = 1", QUALIFIER_REASON);
        assert_refused("\"zebra\".x = 1", QUALIFIER_REASON);
        assert_refused("`zebra`.x = 1", QUALIFIER_REASON);
    }

    #[test]
    fn a_column_with_two_names_before_it_is_refused() {
        assert_refused("zebra.b.x = 1", QUALIFIER_REASON);
    }

    #[test]
    fn a_qualifier_with_a_character_outside_ascii_letters_digits_and_underscore_is_refused() {
        assert_refused("zebra\u{e9}.x = 1", QUALIFIER_REASON);
    }

    #[test]
    fn a_qualifier_that_starts_with_a_digit_is_refused() {
        // `GenericDialect` cannot parse `1zebra.x`, so the shape walk is
        // given the node directly.
        let expr = Expr::CompoundIdentifier(vec![SqlIdent::new("1zebra"), SqlIdent::new("x")]);
        let err = validate_expr_shape(&expr, &["x".to_owned()]).unwrap_err();
        match err {
            RewriteError::InvalidRowFilter { reason } => assert_eq!(reason, QUALIFIER_REASON),
            other => panic!("expected InvalidRowFilter, got {other:?}"),
        }
    }

    #[test]
    fn a_refused_call_operator_or_qualifier_produces_no_sql() {
        let cols = vec!["x".to_owned()];
        for raw in [
            "lower(x) OVER (PARTITION BY x) = 'zebra'",
            "x = 1 OPERATOR(pg_catalog.+) 2",
            "\"zebra\\\".x = 1",
        ] {
            let built = build_masked_filtered_select(
                &table_name(),
                &cols,
                &[],
                Some(raw),
                &PlaceholderValues::none(),
            );
            assert!(
                matches!(built, Err(RewriteError::InvalidRowFilter { .. })),
                "{raw}"
            );
        }
    }

    #[test]
    fn the_plain_calls_operators_and_qualifiers_still_pass_and_expand_unchanged() {
        let cols = vec!["x".to_owned(), "y".to_owned()];
        for (raw, expected) in [
            ("lower(x) = 'a'", "lower(x) = 'a'"),
            ("upper(x) = 'A'", "upper(x) = 'A'"),
            ("tostring(x) = '1'", "tostring(x) = '1'"),
            ("todate(x) > '2020-01-01'", "todate(x) > '2020-01-01'"),
            ("t.x = 1", "t.x = 1"),
            ("_a.x = 1", "_a.x = 1"),
            ("x = 1 AND y <> 2", "x = 1 AND y <> 2"),
            ("x != 1", "x <> 1"),
            ("x <= 1 OR x < 0", "x <= 1 OR x < 0"),
            ("NOT x = 1", "NOT x = 1"),
            ("-x < 1", "-x < 1"),
            ("+x = 1", "+x = 1"),
            ("x + 1 > 2", "x + 1 > 2"),
            ("x % 2 = 0", "x % 2 = 0"),
            ("x * 2 / 4 >= 1", "x * 2 / 4 >= 1"),
            ("x - 1 >= 0", "x - 1 >= 0"),
        ] {
            let expr = validate_row_filter_expr(raw, &cols)
                .unwrap_or_else(|e| panic!("{raw} was refused: {e:?}"));
            assert_eq!(
                expand_placeholders(&expr, &PlaceholderValues::none()).unwrap(),
                expected,
                "{raw}"
            );
        }
    }

    // SEC-44: `Display` writes a unary `-` or `+` with no space before its
    // operand, so `x = - -1` (parsed) prints `x = --1` and `--` starts a
    // SQL line comment. A control character inside a single-quoted string
    // carries the string across a line, so a literal checked here can read
    // differently in `ClickHouse`. The four accepted shapes below all
    // expand to the SAME text the validator saw, so the checked filter and
    // the running filter stay one expression.
    #[test]
    fn validate_row_filter_expr_accepts_and_expands_a_unary_minus_left_of_an_int_operand() {
        let cols = vec!["x".to_owned()];
        let expr = validate_row_filter_expr("x = -1", &cols)
            .unwrap_or_else(|e| panic!("x = -1 was refused: {e:?}"));
        assert_eq!(
            expand_placeholders(&expr, &PlaceholderValues::none()).unwrap(),
            "x = -1"
        );
    }

    #[test]
    fn validate_row_filter_expr_accepts_and_expands_a_binary_minus_over_a_unary_minus() {
        let cols = vec!["x".to_owned()];
        let expr = validate_row_filter_expr("x - -1 > 0", &cols)
            .unwrap_or_else(|e| panic!("x - -1 > 0 was refused: {e:?}"));
        assert_eq!(
            expand_placeholders(&expr, &PlaceholderValues::none()).unwrap(),
            "x - -1 > 0"
        );
    }

    #[test]
    fn validate_row_filter_expr_accepts_and_expands_not_over_a_signed_operand() {
        // `NOT` writes with a space, so the printed text keeps the space
        // that separates it from the inner sign and never produces `--`.
        let cols = vec!["x".to_owned()];
        let expr = validate_row_filter_expr("NOT -x = 1", &cols)
            .unwrap_or_else(|e| panic!("NOT -x = 1 was refused: {e:?}"));
        assert_eq!(
            expand_placeholders(&expr, &PlaceholderValues::none()).unwrap(),
            "NOT -x = 1"
        );
    }

    #[test]
    fn validate_row_filter_expr_accepts_and_expands_like_with_one_pattern() {
        let cols = vec!["x".to_owned()];
        let expr = validate_row_filter_expr("x LIKE 'a%'", &cols)
            .unwrap_or_else(|e| panic!("x LIKE 'a%' was refused: {e:?}"));
        assert_eq!(
            expand_placeholders(&expr, &PlaceholderValues::none()).unwrap(),
            "x LIKE 'a%'"
        );
        // `negated` is a flag distinct from `any`; today (and after this
        // change) a `NOT LIKE` with one pattern stays accepted. The
        // expansion keeps the `NOT` because `Display` prints it with a
        // space.
        let expr = validate_row_filter_expr("x NOT LIKE 'a%'", &cols)
            .unwrap_or_else(|e| panic!("x NOT LIKE 'a%' was refused: {e:?}"));
        assert_eq!(
            expand_placeholders(&expr, &PlaceholderValues::none()).unwrap(),
            "x NOT LIKE 'a%'"
        );
    }

    // SEC-44: every refusal below pins a fixed reason (no filter text, no
    // marker word) and matches the plan's exact wording so the
    // `policy_engine::refusal_message` mapping can keep using a single
    // fixed text per reason. The constants themselves live next to the
    // other refusal strings (SIGN_REFUSAL, CONTROL_CHAR_REFUSAL,
    // LIKE_ANY_REFUSAL) so the production text and the test pin the same
    // source.

    #[test]
    fn validate_row_filter_expr_refuses_a_unary_minus_whose_operand_is_a_unary_minus() {
        let cols = vec!["x".to_owned()];
        // `x = - -1` parses to `BinaryOp(Eq, x, UnaryOp(Minus, UnaryOp(Minus, 1)))`
        // — the outer Minus has a Minus operand, which `Display` writes
        // as `--`, and `--` starts a SQL line comment.
        assert_eq!(
            invalid_reason("x = - -1", &cols),
            SIGN_REFUSAL,
            "raw must be refused with the fixed sign reason"
        );
    }

    #[test]
    fn validate_row_filter_expr_refuses_every_sign_over_sign_combination() {
        let cols = vec!["x".to_owned()];
        for raw in ["x = - -1", "x = + +1", "x = -+1", "x = +-1"] {
            assert_eq!(invalid_reason(raw, &cols), SIGN_REFUSAL, "{raw}");
        }
    }

    #[test]
    fn validate_row_filter_expr_refuses_a_text_value_with_a_raw_newline() {
        // A control character (U+000A here) lets the text inside a single
        // quoted string cross a line, so what the validator saw and what
        // `ClickHouse` reads can differ — and a text containing `\n` can
        // also carry the rest of the comment a leading `--` opened.
        let cols = vec!["x".to_owned()];
        let raw = "x = 'a\nb'";
        assert_eq!(invalid_reason(raw, &cols), CONTROL_CHAR_REFUSAL);
    }

    #[test]
    fn validate_row_filter_expr_refuses_a_text_value_with_a_tab_character() {
        let cols = vec!["x".to_owned()];
        let raw = "x = 'a\tb'";
        assert_eq!(invalid_reason(raw, &cols), CONTROL_CHAR_REFUSAL);
    }

    /// Rows from the table below are filtered through the validator.
    /// Each `LIKE`/`ILIKE` row sets only the `any` flag to `true` (the
    /// pattern is otherwise valid and the operator/kind are on the
    /// allowlist), so the only thing the new check refuses is `any`.
    /// The plain-pattern row is the row whose `any: true` is the ONLY
    /// refusal trigger — the row used by mutation (c).
    const LIKE_ANY_TEST_CASES: &[(&str, &str)] = &[
        ("x LIKE ANY 'a%'", "plain pattern"),
        ("x ILIKE ANY 'a'", "plain pattern, case-insensitive"),
        ("x LIKE ANY ('a', 'b')", "tuple pattern (may not parse)"),
        ("x ILIKE ANY ('a')", "single-element tuple pattern"),
    ];

    #[test]
    fn validate_row_filter_expr_refuses_like_with_any_and_a_plain_pattern() {
        // Isolates rule (3): with `ANY 'a%'`, the only shape outside the
        // simple-`LIKE` allowlist is `any: true`. If the validator stops
        // refusing `any`, this test alone fails — no other row could mask
        // the regression.
        let cols = vec!["x".to_owned()];
        assert_eq!(invalid_reason("x LIKE ANY 'a%'", &cols), LIKE_ANY_REFUSAL);
    }

    #[test]
    fn validate_row_filter_expr_refuses_ilike_with_any_and_a_plain_pattern() {
        let cols = vec!["x".to_owned()];
        assert_eq!(invalid_reason("x ILIKE ANY 'a'", &cols), LIKE_ANY_REFUSAL);
    }

    #[test]
    fn validate_row_filter_expr_refuses_every_any_form_that_parses_under_generic_dialect() {
        // The plan's verified facts: some `LIKE ANY (...)` shapes do not
        // even parse under `GenericDialect`, so the parser (not the
        // validator) refuses them. We loop over the same set, assert the
        // EITHER path ends in `InvalidRowFilter`, and pin the wording only
        // on the rows that reach the new check. The non-parseable rows
        // are recorded here for the handoff (per the plan).
        let cols = vec!["x".to_owned()];
        for (raw, _) in LIKE_ANY_TEST_CASES {
            let err = validate_row_filter_expr(raw, &cols).unwrap_err();
            match err {
                RewriteError::InvalidRowFilter { reason } => {
                    // The plain-pattern rows must end at the new rule with
                    // the LIKE-ANY fixed reason; a tuple-pattern row that
                    // fails to parse carries the parser's own message.
                    let expected = if raw.contains('\'') && !raw.contains(',') {
                        LIKE_ANY_REFUSAL // plain pattern (with or without tuple wrapping)
                    } else if raw.contains(',') {
                        // `x LIKE ANY ('a', 'b')`: a tuple pattern with
                        // more than one element. Either the validator
                        // refuses `any` (and we get LIKE_ANY_REFUSAL), or
                        // the parser refuses the tuple first. Either path
                        // keeps the test green; we just don't pin the
                        // fixed text here.
                        "tuple"
                    } else {
                        unreachable!("unexpected shape: {raw}")
                    };
                    assert!(
                        reason == LIKE_ANY_REFUSAL || expected == "tuple",
                        "{raw}: unexpected refusal reason `{reason}`"
                    );
                }
                other => panic!("{raw}: expected InvalidRowFilter, got {other:?}"),
            }
        }
    }

    /// Walks every accepted shape the test module exercises for this
    /// validator, expanding each through `expand_placeholders`, and
    /// asserts neither SQL comment opener (`--` or `/*`) appears OUTSIDE a
    /// single-quoted string. The structural invariant the validator must
    /// preserve: the SQL `ClickHouse` parses is the SQL the validator
    /// checked. The accepted literal set already guarantees no `'` inside
    /// a literal's text (a backslash is refused, a control character now
    /// too), so toggling `in_string` on every `'` is a safe boundary
    /// (doubled `''` toggles twice and the literal is still tracked to
    /// the closing quote).
    #[test]
    fn expand_placeholders_never_prints_a_comment_opener_outside_a_string() {
        fn assert_no_comment_opener(text: &str) {
            let chars: Vec<char> = text.chars().collect();
            let mut in_string = false;
            for (i, c) in chars.iter().enumerate() {
                if *c == '\'' {
                    in_string = !in_string;
                    continue;
                }
                if in_string {
                    continue;
                }
                let next = chars.get(i + 1).copied().unwrap_or('\0');
                if *c == '-' && next == '-' {
                    panic!("found `--` outside a string in: {text}");
                } else if *c == '/' && next == '*' {
                    panic!("found `/*` outside a string in: {text}");
                }
            }
        }

        let v_none_a = PlaceholderValues::none();
        let v_principal_only = PlaceholderValues {
            principal_id: Some("u-1".to_owned()),
            principal_tenant_ids: vec!["t-1".to_owned()],
        };
        // The four SEC-44-allowed expansion forms plus the shapes the
        // pre-existing accepted tests exercise — one representative per
        // shape, not the full table. Bound locals (not closures) keep the
        // `&PlaceholderValues` references valid for the slice's lifetime.
        let cols_ab = vec!["x".to_owned(), "y".to_owned()];
        let cols_xf = vec!["x".to_owned(), "flag".to_owned()];
        let cols_x = vec!["x".to_owned()];
        let accepted: &[(&[String], &PlaceholderValues, &[&str])] = &[
            (
                &cols_x,
                &v_none_a,
                &[
                    "x = -1",
                    "x - -1 > 0",
                    "NOT -x = 1",
                    "x LIKE 'a%'",
                    "x NOT LIKE 'a%'",
                    "x = 'tenant-a'",
                    "x IN (__principal_tenant_ids__)",
                    "x = 'plain'",
                    "x = 'it''s'",
                    "x = 42",
                    "x > 1.5",
                    "x IS NULL",
                    "x = NULL",
                    "x IN ('a', 'b')",
                ],
            ),
            (
                &cols_xf,
                &v_principal_only,
                &[
                    "flag = true",
                    "x LIKE 'a' ESCAPE '!'",
                    "x NOT LIKE 'a%' ESCAPE '!'",
                    "x LIKE 'a' ESCAPE __principal_id__",
                    "x = __principal_id__",
                ],
            ),
            (
                &cols_ab,
                &v_none_a,
                &[
                    "lower(x) = 'a'",
                    "upper(x) = 'A'",
                    "tostring(x) = '1'",
                    "todate(x) > '2020-01-01'",
                    "t.x = 1",
                    "_a.x = 1",
                    "x = 1 AND y <> 2",
                    "x != 1",
                    "x <= 1 OR x < 0",
                    "NOT x = 1",
                    "-x < 1",
                    "+x = 1",
                    "x + 1 > 2",
                    "x % 2 = 0",
                    "x * 2 / 4 >= 1",
                    "x - 1 >= 0",
                ],
            ),
        ];

        let mut saw = 0usize;
        for (cols, values, raws) in accepted {
            for raw in *raws {
                let expr = validate_row_filter_expr(raw, cols)
                    .unwrap_or_else(|e| panic!("{raw} was refused: {e:?}"));
                let out = expand_placeholders(&expr, values).unwrap();
                assert_no_comment_opener(&out);
                saw += 1;
            }
        }
        assert!(
            saw > 30,
            "sweep should cover a useful slice of accepted forms, saw {saw}"
        );
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
/// processes). [`classify_statement_for_principal`] admits them only to a
/// caller holding `audit:read`, but [`enforce`] — the path every governed
/// SQL surface takes — passes no permissions, so through those surfaces
/// they are never readable; an ordinary `system.*` catalog table
/// (`system.columns`, `system.tables`) stays open.
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

/// Walks `expr`, collecting every bare `Expr::Identifier`/
/// `Expr::CompoundIdentifier` name (the last segment of a compound one)
/// into `idents` and setting `*has_dict_call` when any `Expr::Function`
/// call anywhere inside it is in the `dictGet*`/`joinGet*` family
/// ([`is_dict_or_join_function`]). Returns `false` the moment it reaches
/// an `Expr` shape it does not specifically model (a subquery, a lambda,
/// anything not one of the handful of composite shapes below) — the
/// caller ([`referenced_identifiers`]/[`contains_dict_or_join_call`])
/// then reports "cannot enumerate" rather than a partial, silently
/// under-counted identifier set: WS7 item C1's N2 fix (Hard Requirement 1's
/// fail-closed rule applies here too — a `default_expression` this walker
/// cannot fully see through must never be treated as "provably references
/// no masked column").
fn collect_from_expr(expr: &Expr, idents: &mut HashSet<String>, has_dict_call: &mut bool) -> bool {
    match expr {
        Expr::Identifier(ident) => {
            idents.insert(ident.value.clone());
            true
        }
        Expr::CompoundIdentifier(parts) => {
            if let Some(last) = parts.last() {
                idents.insert(last.value.clone());
            }
            true
        }
        Expr::Value(_) => true,
        Expr::Function(f) => {
            if is_dict_or_join_function(&f.name.to_string()) {
                *has_dict_call = true;
            }
            match &f.args {
                FunctionArguments::None => true,
                FunctionArguments::List(list) => list.args.iter().all(|arg| match arg {
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(e))
                    | FunctionArg::Named {
                        arg: FunctionArgExpr::Expr(e),
                        ..
                    } => collect_from_expr(e, idents, has_dict_call),
                    _ => false,
                }),
                FunctionArguments::Subquery(_) => false,
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            collect_from_expr(left, idents, has_dict_call)
                && collect_from_expr(right, idents, has_dict_call)
        }
        Expr::UnaryOp { expr: inner, .. }
        | Expr::Nested(inner)
        | Expr::IsNull(inner)
        | Expr::IsNotNull(inner)
        | Expr::Cast { expr: inner, .. } => collect_from_expr(inner, idents, has_dict_call),
        Expr::Between {
            expr: inner,
            low,
            high,
            ..
        } => {
            collect_from_expr(inner, idents, has_dict_call)
                && collect_from_expr(low, idents, has_dict_call)
                && collect_from_expr(high, idents, has_dict_call)
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
            collect_from_expr(inner, idents, has_dict_call)
                && collect_from_expr(pattern, idents, has_dict_call)
        }
        Expr::InList {
            expr: inner, list, ..
        } => {
            collect_from_expr(inner, idents, has_dict_call)
                && list
                    .iter()
                    .all(|item| collect_from_expr(item, idents, has_dict_call))
        }
        _ => false,
    }
}

/// Parses `expr_text` as a standalone `Expr` (same `GenericDialect`/
/// `parse_expr` entry point [`validate_row_filter_expr`] uses, WS7 item
/// C1) and collects every bare column identifier referenced anywhere
/// inside it. `None` when `expr_text` fails to parse at all, OR when it
/// parses but contains a shape [`collect_from_expr`] does not model —
/// both are "cannot prove this expression's column references", so both
/// fail closed identically (see [`collect_from_expr`]'s own doc comment).
/// Used by [`crate::policy_engine`]'s transitive mask closure to decide
/// whether a `ClickHouse` `ALIAS`/`MATERIALIZED`/`DEFAULT` column's
/// `default_expression` derives from an already-masked column.
#[must_use]
pub fn referenced_identifiers(expr_text: &str) -> Option<Vec<String>> {
    let dialect = sqlparser::dialect::GenericDialect {};
    let mut parser = Parser::new(&dialect).try_with_sql(expr_text).ok()?;
    let expr = parser.parse_expr().ok()?;
    if !parser.consume_token(&sqlparser::tokenizer::Token::EOF) {
        return None;
    }
    let mut idents = HashSet::new();
    let mut has_dict_call = false;
    if !collect_from_expr(&expr, &mut idents, &mut has_dict_call) {
        return None;
    }
    Some(idents.into_iter().collect())
}

/// Whether `expr_text` (parsed the same way [`referenced_identifiers`]
/// does) calls a `dictGet*`/`dictHas`/`dictIsIn`/`joinGet*` function
/// anywhere inside it (WS7 item C1, N5). An `expr_text` that fails to
/// parse, or parses but contains a shape [`collect_from_expr`] does not
/// model, already returns `true` — [`crate::policy_engine`]'s own
/// `derives_from_masked` branch already fails closed (masks the column)
/// on `referenced_identifiers` returning `None` — so this function itself
/// only has to decide the PARSES-and-fully-enumerable case.
#[must_use]
pub fn contains_dict_or_join_call(expr_text: &str) -> bool {
    let dialect = sqlparser::dialect::GenericDialect {};
    let Some(mut parser) = Parser::new(&dialect).try_with_sql(expr_text).ok() else {
        return true;
    };
    let Ok(expr) = parser.parse_expr() else {
        return true;
    };
    if !parser.consume_token(&sqlparser::tokenizer::Token::EOF) {
        return true; // trailing input after the expression — fail closed
    }
    let mut idents = HashSet::new();
    let mut has_dict_call = false;
    if !collect_from_expr(&expr, &mut idents, &mut has_dict_call) {
        return true;
    }
    has_dict_call
}

#[cfg(test)]
mod default_expression_analysis {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{contains_dict_or_join_call, referenced_identifiers};

    #[test]
    fn referenced_identifiers_finds_a_simple_function_call_argument() {
        let mut refs = referenced_identifiers("lower(email)").unwrap();
        refs.sort();
        assert_eq!(refs, vec!["email".to_owned()]);
    }

    #[test]
    fn referenced_identifiers_finds_every_identifier_in_a_binary_expression() {
        let mut refs = referenced_identifiers("sipHash64(email_lc)").unwrap();
        refs.sort();
        assert_eq!(refs, vec!["email_lc".to_owned()]);
    }

    #[test]
    fn referenced_identifiers_is_none_for_unparseable_text() {
        assert!(referenced_identifiers("??? not valid sql").is_none());
    }

    #[test]
    fn contains_dict_or_join_call_is_true_for_a_dict_get_call() {
        assert!(contains_dict_or_join_call(
            "dictGet('db.crm_dict', 'email', id)"
        ));
    }

    #[test]
    fn contains_dict_or_join_call_is_true_for_a_join_get_call() {
        assert!(contains_dict_or_join_call(
            "joinGet('db.join_table', 'email', id)"
        ));
    }

    #[test]
    fn contains_dict_or_join_call_is_false_for_an_ordinary_function_call() {
        assert!(!contains_dict_or_join_call("lower(email)"));
    }

    #[test]
    fn contains_dict_or_join_call_fails_closed_on_unparseable_text() {
        assert!(contains_dict_or_join_call("??? not valid sql"));
    }
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
pub fn classify_statement(sql: &str, dialect: &dyn Dialect) -> Result<(), RewriteError> {
    classify_statement_for_principal(sql, dialect, &[], false)
}

/// Resolves a table's engine and (for a view) its defining SQL, so
/// [`classify_views`] can tell a view reading a governed table from an
/// ordinary one. The real implementation (Phase C) queries `SELECT
/// engine, create_table_query FROM system.tables WHERE (database,
/// name) = (...)`; `NoViews` (WS7 item B6) implements this as "nothing is
/// ever a view", for a caller with no catalog access at all.
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
/// does not exercise the view-refusal path). `policy_engine::PrefetchedViews`
/// (WS7 item C1) is the real production `SystemTablesCatalog`
/// `routes::query::run` uses (it genuinely resolves `system.tables`), so
/// this stays test-only in the current call graph.
#[allow(
    dead_code,
    reason = "only a #[cfg(test)] caller exists in this commit (this module's own \
              enforce_tests); policy_engine::PrefetchedViews is the real \
              production SystemTablesCatalog every non-test caller uses"
)]
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
pub fn enforce(
    sql: &str,
    dialect: &dyn Dialect,
    principal_roles: &[String],
    placeholders: &PlaceholderValues,
    obligations_source: &dyn ObligationsSource,
    views_catalog: &dyn SystemTablesCatalog,
) -> Result<String, RewriteError> {
    let has_any_obligation = obligations_source.has_any_obligation(principal_roles);
    // No permissions, explicitly: the governed SQL surfaces (Query Studio,
    // dashboards, embeds, alerts, Gold export) never read the
    // SENSITIVE_SYSTEM_TABLES — the audit pages are the path to that data.
    // This used to pass `principal_roles` here, into a parameter that
    // compares entries with the literal permission "audit:read"; a role
    // name never equals that, so the tables were always refused anyway.
    // Passing an empty list states the rule instead of relying on a type
    // mix-up (decision 2026-09-27, plan §8 item 3).
    classify_statement_for_principal(sql, dialect, &[], has_any_obligation)?;
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

    /// Every single-quoted literal of `sql`, as the dialect decodes it.
    fn literal_values(sql: &str) -> Vec<String> {
        struct Collect(Vec<String>);
        impl sqlparser::ast::Visitor for Collect {
            type Break = ();
            fn post_visit_value(
                &mut self,
                v: &sqlparser::ast::ValueWithSpan,
            ) -> std::ops::ControlFlow<()> {
                if let sqlparser::ast::Value::SingleQuotedString(t) = &v.value {
                    self.0.push(t.clone());
                }
                std::ops::ControlFlow::Continue(())
            }
        }
        let stmts = sqlparser::parser::Parser::parse_sql(&ClickHouseDialect {}, sql).unwrap();
        let mut c = Collect(Vec::new());
        for s in &stmts {
            let _ = sqlparser::ast::Visit::visit(s, &mut c);
        }
        assert_eq!(stmts.len(), 1, "one statement in, one statement out: {sql}");
        c.0
    }

    /// `SEC` review finding from BI-18·A: a literal's value survives the
    /// rewrite unchanged, for every byte pattern in the table.
    #[test]
    fn a_literal_value_survives_the_rewrite_unchanged() {
        use lakehouse_core::ident::SqlLiteral;
        let bs = "\\";
        let values: Vec<String> = vec![
            bs.to_owned(),
            format!("{bs}{bs}"),
            format!("{bs}{bs}{bs}"),
            "'".to_owned(),
            "''".to_owned(),
            format!("{bs}'"),
            format!("'{bs}"),
            format!("a{bs}"),
            format!("{bs}a"),
            format!("a{bs}b"),
            format!("{bs}'{bs}"),
            format!("a{bs}'b"),
            format!("a'{bs}b"),
            format!("x{bs}{bs}"),
            format!("50%{bs}_x"),
            format!("{bs}%"),
            format!("{bs}_"),
            "caf\u{e9} \u{65e5}\u{672c}".to_owned(),
            format!("\u{65e5}{bs}"),
            "a\nb\tc".to_owned(),
            String::new(),
        ];
        let src = FakeObligations { any: false };
        for v in &values {
            let lit = SqlLiteral::from(v.clone()).to_string();
            let sql = format!("SELECT a FROM serving.t WHERE c IN ({lit}, 'z') AND d = {lit}");
            let out = enforce(
                &sql,
                &ClickHouseDialect {},
                &[],
                &PlaceholderValues::none(),
                &src,
                &NoViews,
            )
            .unwrap();
            assert_eq!(
                literal_values(&out),
                vec![v.clone(), "z".to_owned(), v.clone()],
                "value changed by the rewrite: in={sql} out={out}"
            );
            // Rewriting its own output is a fixed point: nothing drifts.
            let again = enforce(
                &out,
                &ClickHouseDialect {},
                &[],
                &PlaceholderValues::none(),
                &src,
                &NoViews,
            )
            .unwrap();
            assert_eq!(literal_values(&again), literal_values(&out));
            assert_eq!(again, out);
        }
    }

    /// The same guarantee when the statement is rewritten for a governed
    /// table (the substitution path, not the pass-through one).
    #[test]
    fn a_literal_value_survives_a_governed_rewrite_unchanged() {
        use lakehouse_core::ident::SqlLiteral;
        let src = FakeObligations { any: true };
        for v in ["\\", "a\\", "\\'", "\\\\'x", "50%\\_"] {
            let lit = SqlLiteral::from(v).to_string();
            let sql = format!("SELECT id FROM silver.customers WHERE id = {lit} AND email != 'q'");
            let out = enforce(
                &sql,
                &ClickHouseDialect {},
                &[],
                &PlaceholderValues::none(),
                &src,
                &NoViews,
            )
            .unwrap();
            assert!(out.contains("replaceRegexpOne"), "{out}");
            let lits = literal_values(&out);
            assert!(lits.contains(&v.to_owned()), "{out}");
            assert!(lits.contains(&"q".to_owned()), "{out}");
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

    /// Decision 2026-09-27: governed surfaces never read the sensitive
    /// system tables. Before, `enforce` passed ROLE names where the
    /// classifier expects PERMISSIONS, so a role literally named
    /// "audit:read" would have been let through; now it is not.
    #[test]
    fn a_sensitive_system_table_is_refused_whatever_the_role_names() {
        let src = FakeObligations { any: false };
        for roles in [
            vec![],
            vec!["audit:read".to_owned()],
            vec!["Platform Admin".to_owned()],
        ] {
            let err = enforce(
                "SELECT * FROM system.query_log",
                &ClickHouseDialect {},
                &roles,
                &PlaceholderValues::none(),
                &src,
                &NoViews,
            )
            .unwrap_err();
            assert!(
                matches!(err, RewriteError::SensitiveSystemTable { .. }),
                "{roles:?}: {err:?}"
            );
        }
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
        assert!(out.contains("replaceRegexpOne(toString(`email`)"));
    }

    /// The bypass this closes: a policy on `silver.customers` must bind a
    /// read of the same table through a `DataLakeCatalog` database.
    #[test]
    fn a_governed_table_read_through_a_data_lake_catalog_database_is_substituted() {
        let src = FakeObligations { any: true };
        let out = enforce(
            "SELECT email FROM icecat_silver.`silver.customers`",
            &ClickHouseDialect {},
            &[],
            &PlaceholderValues::none(),
            &src,
            &NoViews,
        )
        .unwrap();
        assert!(
            out.contains("replaceRegexpOne(toString(`email`)"),
            "out={out}"
        );
        // The derived table still reads the physical DataLakeCatalog name.
        assert!(
            out.contains("icecat_silver.`silver.customers`"),
            "out={out}"
        );
    }

    /// Governed, but the real columns cannot be resolved (the usual case
    /// for a `DataLakeCatalog` table today, which `system.columns` does not
    /// list): refused, never passed through unmasked.
    #[test]
    fn a_governed_data_lake_catalog_table_with_unknown_columns_is_refused() {
        struct NoColumns;
        impl ObligationsSource for NoColumns {
            fn obligations_for(&self, table: &str, _roles: &[String]) -> Option<TableObligations> {
                (table == "bronze.orders").then(|| TableObligations {
                    mask: vec!["email".to_owned()],
                    row_filter: None,
                    real_columns: None,
                })
            }

            fn has_any_obligation(&self, _roles: &[String]) -> bool {
                true
            }
        }
        let err = enforce(
            "SELECT * FROM icecat_maintenance.`bronze.orders`",
            &ClickHouseDialect {},
            &[],
            &PlaceholderValues::none(),
            &NoColumns,
            &NoViews,
        )
        .unwrap_err();
        assert!(
            matches!(err, RewriteError::UnprovableSubstitution { ref table } if table == "bronze.orders"),
            "err={err:?}"
        );
    }

    /// An ungoverned Iceberg table stays queryable.
    #[test]
    fn an_ungoverned_data_lake_catalog_table_passes_through() {
        let src = FakeObligations { any: true };
        let sql = "SELECT * FROM icecat_maintenance.`bronze.orders`";
        let out = enforce(
            sql,
            &ClickHouseDialect {},
            &[],
            &PlaceholderValues::none(),
            &src,
            &NoViews,
        )
        .unwrap();
        assert!(out.contains("`bronze.orders`"), "out={out}");
        assert!(!out.contains("replaceRegexpOne"), "out={out}");
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
        // A column lookup in the shape the removed text-to-SQL schema
        // context used for its DESCRIBE call, mirrored as a query.
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
