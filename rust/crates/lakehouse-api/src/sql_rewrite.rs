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
