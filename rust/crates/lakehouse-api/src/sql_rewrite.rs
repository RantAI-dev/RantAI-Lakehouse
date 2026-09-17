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
/// (Task B1 Step 3), run with `cargo test -p lakehouse-api --lib
/// sql_rewrite:: -- --nocapture` against `sqlparser` 0.62.0 with
/// `ClickHouseDialect`. Of the twelve M1-listed leak classes, eleven
/// parse under `ClickHouseDialect` today (`ARRAY JOIN` included) and are
/// proven safe by `table_substitution`'s own tests (Task B3) instead.
/// Only `with_scalar_alias` (`WITH 2024 AS target_year SELECT * FROM
/// silver.customers WHERE tahun = target_year` — a bare numeric-literal
/// `WITH` binding, distinct from a `WITH ... AS (<query>)` CTE) is
/// rejected by `sqlparser` 0.62.0's `ClickHouseDialect` grammar; it is
/// refused end-to-end (`RewriteError::Unparseable`), never silently
/// treated as touching no governed table — see `refusals::
/// refuses_unparseable_shapes` (Task B5).
#[allow(
    dead_code,
    reason = "no reader exists yet in this commit (Task B1); Task B5's \
              refusals::refuses_unparseable_shapes test is the first caller"
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
/// [`substitute_governed_tables`] (Task B3) both start from this same
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
/// [`substitute_governed_tables`]'s mutating pass runs (Task B3), and
/// (unlike that pass) collapses a self-join's two occurrences of the
/// same table into one name — a call site that needs every AST
/// POSITION, not just every distinct name, uses Task B3's
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
#[allow(
    dead_code,
    reason = "no non-test caller exists yet in this commit (Task B2); \
              Task B6's enforce() is the first production caller"
)]
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
/// table-function call — `args.is_some()` — which Task B5 classifies
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
        // that to one canonical name, but Task B3's MUTATING walk
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

#[cfg(test)]
mod parses_real_repository_query_shapes {
    use sqlparser::dialect::{ClickHouseDialect, GenericDialect};
    use sqlparser::parser::Parser;

    /// Every one of these strings is copied verbatim from a real call
    /// site in this repository. If `sqlparser` cannot parse one, that
    /// shape becomes a permanently refused construct (documented on
    /// [`super::REFUSED_UNPARSEABLE`] and covered by a regression test
    /// in the `refusals` module, Task B5), never a silent
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
        // handled by Task B5's pre-split, so the parser only ever sees this.
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
    /// recorded) — the `table_substitution` module (Task B3) then
    /// proves table substitution masks/filters correctly for every one
    /// that DOES parse; one that does NOT parse is covered instead by
    /// the `refusals` module's `refuses_unparseable_shapes` (Task B5).
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
    /// in `super::REFUSED_UNPARSEABLE`'s entries per Task B1 Step 3. It
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
